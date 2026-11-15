//! P32-037 (M6-d): 日次補給(首都からの到達判定 + 日次の配分)の測定コマンド(読み取り専用。保存・アセットは書かない。性能値はテストの合否条件ではない)。
//!
//! 入力は P32-031 と同じ測定専用のグラフ(現在の地図の陸セル 1 つ=1 プロヴィンス。35,221 ノード。**正式な全世界の区画ではない**)。国家数 3 / 30 / 100。
//! P32-031 では各主体が重なる通行集合を持てたが、実効支配は1プロヴィンス1国なので、支配を次の2通りに分けて作る(どちらも正式な国境ではない):
//! - `split`: P32-031 の「通行範囲が分かれる」と同じ作り。陸を `q` 昇順に国家数で等分した帯を、国家ごとに支配する。
//! - `skewed`: 国家1が陸の 90% を支配し、残りを他の国が等分して支配する(大国が広い範囲を探索する、`heavy` に近い負荷。ただし支配は排他なので、全国家の到達範囲の合計は陸の総数以下で、P32-031 の `heavy`
//!   (各主体が約 17,200 件を探索する)より小さい)。
//!
//! 各国家に首都(その国家の支配地の最大の連結成分の中ほど)と、その成分の中に 10 個の師団(標準の戦闘能力。補給消費量 1.0。備蓄は十分)を置く(師団数 = 国家数 × 10)。
//!
//! 1日 = `advance_day`(補給の計画・移動・戦闘・組織率の回復を含む。師団に命令がないので、補給以外の処理は小さい)。補給なしの同じ師団の `advance_day` も測り、その差を補給の費用とみなす。
//! 条件: `reuse`(到達結果をすべて再利用)/ `partial`(国家数の約 1 割だけ無効化して計算し直す)/ `all`(全国家を無効化して計算し直す)。計測区間は重ねず、静的グラフ構築・探索だけ(`reach` の直接呼び出し。
//! 通行集合の準備を含む)・1日の合計を区別して出す。各条件を `--trials` 回(既定 3)測り、全試行・中央値・最大値を出す。
//!
//! 実行(`strategy_game/` で、release): cargo run --release --bin world_supply_bench

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use strategy_game::common::CountryId;
use strategy_game::hexmap::data::load_from_path;
use strategy_game::world::measure::{MeasureRule, land_definitions};
use strategy_game::world::{
    Coverage, ExplicitPassable, MapDivision, MapDivisionId, MapMilitary, MoveEvent, MovementParams, ProvinceControl, ProvinceGraph, ProvinceId, SupplyCountryConfig, reach,
};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 }
}

fn stats(v: &[f64]) -> String {
    format!(
        "median={:.3} max={:.3} min={:.3} (trials: {})",
        median(v),
        v.iter().cloned().fold(f64::MIN, f64::max),
        v.iter().cloned().fold(f64::MAX, f64::min),
        v.iter().map(|x| format!("{x:.3}")).collect::<Vec<_>>().join(" / ")
    )
}

/// 陸を `q` 昇順に並べ、レイアウトに従って国家ごとのグループへ分ける。
fn groups(graph: &ProvinceGraph, layout: &str, n: usize) -> Vec<Vec<ProvinceId>> {
    let mut by_q: Vec<(i32, i32, ProvinceId)> = graph
        .node_ids()
        .map(|id| {
            let c = graph.cells_of(id).expect("node")[0];
            (c.q, c.r, id)
        })
        .collect();
    by_q.sort_unstable();
    let total = by_q.len();
    let ids: Vec<ProvinceId> = by_q.into_iter().map(|t| t.2).collect();
    let mut out = Vec::with_capacity(n);
    match layout {
        "split" => {
            for k in 0..n {
                out.push(ids[total * k / n..total * (k + 1) / n].to_vec());
            }
        }
        _ => {
            let big = total * 9 / 10;
            out.push(ids[..big].to_vec());
            let rest = &ids[big..];
            let m = n - 1;
            for k in 0..m {
                out.push(rest[rest.len() * k / m..rest.len() * (k + 1) / m].to_vec());
            }
        }
    }
    out
}

struct Setup {
    military: MapMilitary,
    no_supply: MapMilitary,
    countries: Vec<CountryId>,
    passables: Vec<(ProvinceId, Vec<ProvinceId>)>,
    divisions: usize,
}

fn setup(graph: &ProvinceGraph, layout: &str, n: usize) -> Setup {
    let gs = groups(graph, layout, n);
    let mut control: BTreeMap<ProvinceId, CountryId> = BTreeMap::new();
    let mut divisions: Vec<MapDivision> = Vec::new();
    let mut configs: Vec<SupplyCountryConfig> = Vec::new();
    let mut countries = Vec::new();
    let mut passables = Vec::new();
    for (k, g) in gs.iter().enumerate() {
        let c = CountryId(k + 1);
        countries.push(c);
        for p in g {
            control.insert(*p, c);
        }
        let pass = ExplicitPassable::new(graph, g.iter().copied()).expect("ids");
        let comps = graph.components_among(&pass);
        let largest = comps.iter().fold(None::<&Vec<ProvinceId>>, |acc, c| match acc {
            Some(a) if a.len() >= c.len() => Some(a),
            _ => Some(c),
        });
        let comp = largest.expect("a component");
        let capital = comp[comp.len() / 2];
        configs.push(SupplyCountryConfig { country: c, capital, stockpile: 1000.0 });
        for j in 0..10usize {
            let pos = comp[(comp.len() * (2 * j + 1)) / 20];
            divisions.push(MapDivision::new(MapDivisionId((k * 10 + j + 1) as u32), c, pos, 1.0));
        }
        passables.push((capital, g.clone()));
    }
    let count = divisions.len();
    let ctl = ProvinceControl::new(control);
    let params = MovementParams { base_days: 5.0 };
    let no_supply = MapMilitary::new(divisions.clone(), ctl.clone(), params).expect("military");
    let military = MapMilitary::new(divisions, ctl, params).expect("military").with_supply(configs).expect("supply");
    Setup { military, no_supply, countries, passables, divisions: count }
}

fn recomputed(ev: &[MoveEvent]) -> usize {
    ev.iter().filter(|e| matches!(e, MoveEvent::DailySupply(r) if r.reach_recomputed)).count()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut trials = 3usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--trials" => {
                i += 1;
                trials = args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("USAGE ERROR: --trials needs a number");
                    std::process::exit(64)
                });
            }
            other => {
                eprintln!("USAGE ERROR: unknown argument {other}\nusage: world_supply_bench [--trials N]");
                std::process::exit(64);
            }
        }
        i += 1;
    }
    let limit = Duration::from_secs(120);
    println!("=== environment ===");
    println!(
        "os={} arch={} profile={} logical_cpus_visible={} (single-threaded; no affinity/priority set) trials={trials} limit_per_condition=120s",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) { "DEBUG (not a valid measurement)" } else { "release" },
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0)
    );
    let t = Instant::now();
    let map = load_from_path("assets/data/hex_atlas_map.json").expect("map");
    println!("file_load_ms (not part of any day): {:.3}", ms(t.elapsed()));
    let (defs, facts) = land_definitions(&map, MeasureRule::DEFAULT);
    let mut builds = Vec::new();
    let mut graph = None;
    for _ in 0..trials {
        let t = Instant::now();
        let g = ProvinceGraph::from_definitions(&defs, &map, Coverage::World).expect("graph");
        builds.push(ms(t.elapsed()));
        graph = Some(g);
    }
    let graph = graph.expect("graph");
    println!("MEASUREMENT-ONLY INPUT (not the official world definition): 1 land cell = 1 province; land_cells={} nodes={} edges={}", facts.land_cells, graph.node_count(), graph.edge_count());
    println!("static_build_ms (once per world change, NOT a daily cost): {}", stats(&builds));

    for layout in ["split", "skewed"] {
        for n in [3usize, 30, 100] {
            println!("\n=== layout {layout}, countries {n} ===");
            let t = Instant::now();
            let s = setup(&graph, layout, n);
            println!("setup_ms (groups, capitals, divisions; not part of any day): {:.3} | countries={} divisions={}", ms(t.elapsed()), s.countries.len(), s.divisions);
            // 到達探索だけ(通行集合の準備 + reach。P32-031 の「準備」+「探索」に当たる)
            let started = Instant::now();
            let mut explore = Vec::new();
            let mut reached_total = 0usize;
            for _ in 0..trials {
                if started.elapsed() > limit {
                    println!("  explore: INCOMPLETE");
                    break;
                }
                let t = Instant::now();
                let mut total = 0usize;
                for (capital, g) in &s.passables {
                    let pass = ExplicitPassable::new(&graph, g.iter().copied()).expect("ids");
                    let r = reach(&graph, &[*capital], &pass).expect("reach");
                    total += r.reached_count();
                }
                explore.push(ms(t.elapsed()));
                reached_total = total;
            }
            println!("  reach exploration only, all {n} countries (passable preparation + reach): {} | provinces reached in total: {reached_total}", stats(&explore));
            // 1日(ウォームアップの1日で全国家の到達結果をキャッシュしてから、各条件を測る)
            let mut warm = s.military.clone();
            let ev = warm.advance_day(&graph);
            println!("  warm-up day: reach recomputed for {} countries", recomputed(&ev));
            let mut warm_ns = s.no_supply.clone();
            warm_ns.advance_day(&graph);
            let partial_k = (n / 10).max(1);
            let all_countries = s.countries.clone();
            let subset: Vec<CountryId> = s.countries.iter().take(partial_k).copied().collect();
            for (name, subset_ref) in [("reuse", Vec::new()), ("partial", subset.clone()), ("all", all_countries.clone())] {
                let started = Instant::now();
                let (mut with, mut without, mut diff) = (Vec::new(), Vec::new(), Vec::new());
                let mut last_recomputed = 0usize;
                for _ in 0..trials {
                    if started.elapsed() > limit {
                        println!("  {name}: INCOMPLETE");
                        break;
                    }
                    let mut m = warm.clone();
                    m.invalidate_reach(&subset_ref);
                    let t = Instant::now();
                    let ev = m.advance_day(&graph);
                    let w = ms(t.elapsed());
                    let mut base = warm_ns.clone();
                    let t = Instant::now();
                    base.advance_day(&graph);
                    let wo = ms(t.elapsed());
                    last_recomputed = recomputed(&ev);
                    with.push(w);
                    without.push(wo);
                    diff.push(w - wo);
                }
                println!(
                    "  day [{name}]: recomputed countries={last_recomputed}/{n} | day_with_supply_ms {} | day_without_supply_ms {} | supply_cost_ms (difference) {}",
                    stats(&with),
                    stats(&without),
                    stats(&diff)
                );
            }
        }
    }
    println!("\nnote: day_with_supply covers `advance_day` (supply planning + the other daily steps); the clone of the warmed-up state, graph build and file load are excluded. Divisions have no orders, so the non-supply steps are small.");
}
