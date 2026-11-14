//! P32-031 (M5-b): 到達判定とステート集約の**規模測定コマンド**(読み取り専用。保存・アセットは書かない)。
//!
//! 現在の地図の陸セルを、測定専用のグラフ(1陸セル=1プロヴィンス)としてメモリ上だけで作り、探索主体 3 / 30 / 100 の
//! 「1ゲーム日相当」(対象全主体の 通行条件の準備 → 到達判定 → ステート集約 → 結果の解放 の1巡)を測る。
//! **測定入力は正式な全世界の区画定義・所有・国境・実効支配ではない**(`world::measure` に規則を記載)。
//! 到達可能は「十分な補給が届く」ではない。D10 は確定しない(判断材料の取得だけ)。
//!
//! 実行(`strategy_game/` で、release): cargo run --release --bin world_reach_bench
//!   --map <path>        地図JSON(既定 assets/data/hex_atlas_map.json)
//!   --trials <n>        各条件の試行回数(既定 3)
//!   --limit-secs <n>    1条件全体の上限(既定 120。超えたら未完了として記録し、再試行しない)
//!   --only <load:n>     1条件だけ実行(例 split:30 / heavy:100)
//!   --small <b>         測定入力の格子の1辺(既定 10)
//!
//! 計時区間(重ならない): file_load / definitions / static_build(毎日の費用に含めない)と、
//! 1日の中の prep(通行条件の準備)/ search(主体ごとの到達判定。合計と主体別)/ aggregate(ステート集約)/ release(結果の解放)。
//! day_total は1日の外側の1つの計時で、区間の和との差は other として示す。コンパイル時間は含まない(別途記録)。

use std::time::{Duration, Instant};

use strategy_game::hexmap::data::load_from_path;
use strategy_game::world::measure::{AgentSpec, LoadKind, MeasureRule, land_definitions, make_agents};
use strategy_game::world::{ExplicitPassable, ProvinceGraph, Reach, StateReachKind, StateReachSummary, reach, summarize_by_state};

const USAGE: &str = "usage: world_reach_bench [--map P] [--trials N] [--limit-secs N] [--only split|heavy:AGENTS] [--small BLOCK]";

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 }
}

fn max_of(v: &[f64]) -> f64 {
    v.iter().copied().fold(f64::NAN, f64::max)
}

fn min_of(v: &[f64]) -> f64 {
    v.iter().copied().fold(f64::NAN, f64::min)
}

struct Output {
    buf: Vec<String>,
    spent: Duration,
}

impl Output {
    fn line(&mut self, s: String) {
        self.buf.push(s);
    }
    fn flush(&mut self) {
        let t = Instant::now();
        for l in self.buf.drain(..) {
            println!("{l}");
        }
        self.spent += t.elapsed();
    }
}

fn fnv(h: &mut u64, v: u64) {
    for b in v.to_le_bytes() {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

struct Trial {
    prep: f64,
    search: f64,
    search_each: Vec<f64>,
    aggregate: f64,
    release: f64,
    day: f64,
    reached: Vec<usize>,
    unknown: Vec<usize>,
    kinds: [usize; 6],
    checksum: u64,
}

fn one_day(graph: &ProvinceGraph, agents: &[AgentSpec], limit: Instant) -> Option<Trial> {
    let day = Instant::now();
    // --- 通行条件の準備(毎日、全主体について明示集合を作り直す = 最悪側の更新) ---
    let t = Instant::now();
    let passes: Vec<ExplicitPassable> =
        agents.iter().map(|a| ExplicitPassable::new(graph, a.passable.iter().copied()).expect("ids come from the graph")).collect();
    let prep = ms(t.elapsed());
    // --- 主体ごとの到達判定 ---
    let t = Instant::now();
    let mut results: Vec<Reach> = Vec::with_capacity(agents.len());
    let mut search_each = Vec::with_capacity(agents.len());
    for (a, pass) in agents.iter().zip(&passes) {
        let s = Instant::now();
        let r = reach(graph, &[a.source], pass).expect("source is passable by construction");
        search_each.push(ms(s.elapsed()));
        results.push(r);
        if Instant::now() > limit {
            return None;
        }
    }
    let search = ms(t.elapsed());
    // --- ステート集約(到達結果と所属だけから。再探索しない) ---
    let t = Instant::now();
    let summaries: Vec<StateReachSummary> = results.iter().map(|r| summarize_by_state(graph, r).expect("same graph")).collect();
    let aggregate = ms(t.elapsed());
    // --- 結果の解放(検証用の集計は計時から除く) ---
    let reached: Vec<usize> = results.iter().map(|r| r.reached_count()).collect();
    let unknown: Vec<usize> = results.iter().map(|r| r.unknown_count()).collect();
    let mut kinds = [0usize; 6];
    let mut checksum: u64 = 0xcbf2_9ce4_8422_2325;
    let release_start = Instant::now();
    // 検証用の集計(計時外): reached ids と区分の合計
    let verify_start = Instant::now();
    for (i, (r, s)) in results.iter().zip(&summaries).enumerate() {
        fnv(&mut checksum, i as u64);
        for id in r.reached_ids() {
            fnv(&mut checksum, id.0 as u64);
        }
        for st in s.states() {
            fnv(&mut checksum, st.state.0 as u64);
            fnv(&mut checksum, st.counts.reachable as u64);
            fnv(&mut checksum, st.counts.unknown as u64);
            let k = match st.kind {
                StateReachKind::Full => 0,
                StateReachKind::Partial => 1,
                StateReachKind::Unreachable => 2,
                StateReachKind::Unknown => 3,
                StateReachKind::NotApplicable => 4,
                StateReachKind::Empty => 5,
            };
            kinds[k] += 1;
        }
    }
    let verify = verify_start.elapsed();
    drop(results);
    drop(summaries);
    drop(passes);
    let release = ms(release_start.elapsed() - verify);
    let day_ms = ms(day.elapsed() - verify);
    Some(Trial { prep, search, search_each, aggregate, release, day: day_ms, reached, unknown, kinds, checksum })
}

fn run_condition(out: &mut Output, graph: &ProvinceGraph, kind: LoadKind, n: usize, trials: usize, limit_secs: u64) {
    let name = match kind {
        LoadKind::Split => "split",
        LoadKind::Heavy => "heavy",
    };
    out.line(format!("\n=== condition {name}:{n} (agents={n}) ==="));
    let t = Instant::now();
    let agents = make_agents(graph, kind, n);
    out.line(format!("agent_setup_ms (not part of the daily cost): {:.3}", ms(t.elapsed())));
    let passable_sizes: Vec<usize> = agents.iter().map(|a| a.passable.len()).collect();
    out.line(format!(
        "passable per agent: min={} median={} max={} total={}; sources: {}",
        passable_sizes.iter().min().unwrap(),
        { let mut s = passable_sizes.clone(); s.sort(); s[s.len() / 2] },
        passable_sizes.iter().max().unwrap(),
        passable_sizes.iter().sum::<usize>(),
        agents.iter().take(5).map(|a| a.source.0.to_string()).collect::<Vec<_>>().join(",") + if n > 5 { ",..." } else { "" }
    ));
    out.flush();
    let started = Instant::now();
    let limit = started + Duration::from_secs(limit_secs);
    let mut done: Vec<Trial> = Vec::new();
    for i in 1..=trials {
        if Instant::now() > limit {
            break;
        }
        match one_day(graph, &agents, limit) {
            Some(t) => {
                out.line(format!(
                    "trial {i}: day_total={:.3}ms = prep {:.3} + search {:.3} + aggregate {:.3} + release {:.3} (+ other {:.3}) | search per agent min/med/max = {:.3}/{:.3}/{:.3} ms | checksum {:016x}",
                    t.day, t.prep, t.search, t.aggregate, t.release,
                    t.day - (t.prep + t.search + t.aggregate + t.release),
                    min_of(&t.search_each), median(&t.search_each), max_of(&t.search_each), t.checksum
                ));
                done.push(t);
            }
            None => {
                out.line(format!("trial {i}: INCOMPLETE (the {limit_secs}s limit for this condition was exceeded); not retried"));
                break;
            }
        }
        out.flush();
    }
    if done.len() < trials {
        out.line(format!("condition status: INCOMPLETE ({} of {trials} trials finished within {limit_secs}s)", done.len()));
    } else {
        out.line(format!("condition status: complete ({trials} trials, {:.1}s total)", started.elapsed().as_secs_f64()));
    }
    if let Some(first) = done.first() {
        let col = |f: fn(&Trial) -> f64| -> Vec<f64> { done.iter().map(f).collect() };
        let line = |label: &str, v: Vec<f64>| format!("  {label:<14} min={:.3} median={:.3} max={:.3} (ms; trials: {})", min_of(&v), median(&v), max_of(&v), v.iter().map(|x| format!("{x:.3}")).collect::<Vec<_>>().join(" / "));
        out.line("summary over trials:".into());
        out.line(line("prep", col(|t| t.prep)));
        out.line(line("search_total", col(|t| t.search)));
        out.line(line("aggregate", col(|t| t.aggregate)));
        out.line(line("release", col(|t| t.release)));
        out.line(line("DAY_TOTAL", col(|t| t.day)));
        let same = done.iter().all(|t| t.checksum == first.checksum && t.reached == first.reached);
        let mut r = first.reached.clone();
        r.sort();
        out.line(format!(
            "searched: reached per agent min={} median={} max={} sum={} | unknown sum={} | state summaries per all agents: full={} partial={} unreachable={} unknown={} not_applicable={} empty={}",
            r[0], r[r.len() / 2], r[r.len() - 1], r.iter().sum::<usize>(), first.unknown.iter().sum::<usize>(),
            first.kinds[0], first.kinds[1], first.kinds[2], first.kinds[3], first.kinds[4], first.kinds[5]
        ));
        out.line(format!("results identical across trials: {same}"));
    }
    out.flush();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut map_path = "assets/data/hex_atlas_map.json".to_string();
    let mut trials = 3usize;
    let mut limit_secs = 120u64;
    let mut only: Option<(LoadKind, usize)> = None;
    let mut block = MeasureRule::DEFAULT.block;
    let mut i = 0;
    while i < args.len() {
        let name = args[i].as_str();
        let mut value = || {
            i += 1;
            args.get(i).cloned().unwrap_or_else(|| {
                eprintln!("USAGE ERROR: {name} needs a value\n{USAGE}");
                std::process::exit(64)
            })
        };
        match name {
            "--map" => map_path = value(),
            "--trials" => trials = value().parse().unwrap_or_else(|_| usage("--trials")),
            "--limit-secs" => limit_secs = value().parse().unwrap_or_else(|_| usage("--limit-secs")),
            "--small" => block = value().parse().unwrap_or_else(|_| usage("--small")),
            "--only" => {
                let v = value();
                let (k, n) = v.split_once(':').unwrap_or_else(|| usage("--only"));
                let kind = match k {
                    "split" => LoadKind::Split,
                    "heavy" => LoadKind::Heavy,
                    _ => usage("--only"),
                };
                only = Some((kind, n.parse().unwrap_or_else(|_| usage("--only"))));
            }
            _ => usage(name),
        }
        i += 1;
    }
    let mut out = Output { buf: Vec::new(), spent: Duration::ZERO };
    out.line("=== environment ===".into());
    out.line(format!(
        "os={} arch={} profile={} logical_cpus_visible={} (the measurement is single-threaded; no affinity/priority was set)",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) { "DEBUG (not a valid measurement)" } else { "release" },
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0)
    ));
    out.line(format!("trials={trials} limit_secs_per_condition={limit_secs}"));

    let t = Instant::now();
    let map = match load_from_path(&map_path) {
        Ok(m) => m,
        Err(e) => {
            println!("LOAD ERROR: {e}");
            std::process::exit(66);
        }
    };
    out.line(format!("file_load_ms (map json, not part of any search cost): {:.3}", ms(t.elapsed())));

    let rule = MeasureRule { block, ..MeasureRule::DEFAULT };
    let t = Instant::now();
    let (defs, facts) = land_definitions(&map, rule);
    out.line(format!("definitions_ms (measurement-only input, in memory): {:.3}", ms(t.elapsed())));
    out.line(format!(
        "MEASUREMENT-ONLY INPUT (not the official world definition): 1 land cell = 1 province; ProvinceId = rank of (r,q); states = {0}x{0}-cell blocks (MapStateId from {1}); ProvinceId % {2} == 0 is unassigned",
        rule.block, rule.state_base, rule.unassigned_modulus
    ));
    out.line(format!("input: land_cells={} provinces={} states={} unassigned={}", facts.land_cells, facts.provinces, facts.states, facts.unassigned));
    out.flush();

    // 静的グラフ構築(毎日の費用ではない)。試行ごとに作り直して計り、最後のものを使う。
    let mut builds = Vec::new();
    let mut graph: Option<ProvinceGraph> = None;
    let started = Instant::now();
    for _ in 0..trials {
        if started.elapsed() > Duration::from_secs(limit_secs) {
            break;
        }
        drop(graph.take()); // 前のグラフの解放を、次の構築の計時に混ぜない
        let t = Instant::now();
        let g = ProvinceGraph::from_definitions(&defs, &map, strategy_game::world::Coverage::World).expect("graph builds");
        builds.push(ms(t.elapsed()));
        graph = Some(g);
    }
    let Some(graph) = graph else {
        println!("static build did not finish");
        std::process::exit(1);
    };
    out.line(format!(
        "static_build_ms (once per world change, NOT a daily cost): min={:.3} median={:.3} max={:.3} (trials: {})",
        min_of(&builds), median(&builds), max_of(&builds), builds.iter().map(|x| format!("{x:.3}")).collect::<Vec<_>>().join(" / ")
    ));
    let t = Instant::now();
    let comps = graph.components();
    let comp_ms = ms(t.elapsed());
    let scope = graph.scope();
    out.line(format!(
        "graph: nodes={} undirected_edges={} connected_components={} (largest {}) unassigned_nodes={} out_of_scope_land_cells={} cross_state_edges={} (components computed in {comp_ms:.3} ms, not part of any condition)",
        graph.node_count(), graph.edge_count(), comps.len(), comps.iter().map(|c| c.len()).max().unwrap_or(0),
        scope.unassigned_nodes, scope.out_of_scope_land_cells, graph.cross_state_edge_count()
    ));
    out.flush();

    let conditions: Vec<(LoadKind, usize)> = match only {
        Some(c) => vec![c],
        None => [LoadKind::Split, LoadKind::Heavy].into_iter().flat_map(|k| [3, 30, 100].map(|n| (k, n))).collect(),
    };
    for (kind, n) in conditions {
        run_condition(&mut out, &graph, kind, n, trials, limit_secs);
    }
    out.line(format!("\noutput_ms (printing this report, accumulated up to the previous flush): {:.3}", ms(out.spent)));
    out.flush();
}

fn usage(what: &str) -> ! {
    eprintln!("USAGE ERROR: bad value for {what}\n{USAGE}");
    std::process::exit(64)
}
