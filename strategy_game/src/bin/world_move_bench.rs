//! P32-034 (M6-a): 移動命令 1 回の処理時間の測定コマンド(読み取り専用。保存・アセットは書かない。性能値はテストの合否条件ではない)。
//!
//! 2 つの入力で測る: (1) 検証地域(29 プロヴィンス。既存の区画・検証用シナリオ)、(2) 測定専用の 35,221 ノード(現在の地図の陸セル 1 つ=1 プロヴィンス。
//! **正式な全世界の区画ではない**。メモリ上だけで作り、どこにも保存しない。支配は「全プロヴィンスを国家1の支配、`ProvinceId % 97 == 0` を国家2の支配」とする測定専用の規則)。
//! グラフ構築と命令処理を分けて計る(命令処理の計時にはグラフ構築・師団状態の複製を含めない)。各条件を `--trials` 回(既定 5)繰り返し、全試行・中央値・最大値を出す。
//!
//! 実行(`strategy_game/` で、release): cargo run --release --bin world_move_bench
//! 注: 命令の経路探索は `find_land_path`(同費用は ProvinceId 昇順の階層 BFS)。結果は旧経路と同じ規則で、時間は他のプロセスとの共存・実行間で揺れる。

use std::time::{Duration, Instant};

use strategy_game::common::CountryId;
use strategy_game::hexmap::data::load_from_path;
use strategy_game::world::measure::{MeasureRule, land_definitions};
use strategy_game::world::{
    Coverage, ExplicitPassable, MapDivision, MapDivisionId, MapMilitary, MovementParams, OrderOutcome, OrderRejection, ProvinceControl, ProvinceGraph, ProvinceId,
    load_division_scenario, load_world_preview,
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

fn describe(r: &Result<OrderOutcome, OrderRejection>) -> String {
    match r {
        Ok(OrderOutcome::Started { hops }) => format!("accepted, path length {hops} hop(s)"),
        Ok(OrderOutcome::AlreadyThere) => "no-op (already there)".into(),
        Ok(OrderOutcome::AlreadyMoving) => "no-op (already moving)".into(),
        Err(e) => format!("rejected: {e}"),
    }
}

/// 1 条件を `trials` 回測る。命令ごとに、未命令の状態の複製を作ってから(計時外)、`issue_move_order` だけを計る。
fn measure(label: &str, base: &MapMilitary, graph: &ProvinceGraph, div: MapDivisionId, dest: ProvinceId, trials: usize, limit: Duration) {
    let started = Instant::now();
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..trials {
        if started.elapsed() > limit {
            println!("  {label}: INCOMPLETE ({} of {trials} trials within the limit)", times.len());
            break;
        }
        let mut m = base.clone();
        let t = Instant::now();
        let r = m.issue_move_order(graph, div, dest);
        times.push(ms(t.elapsed()));
        last = Some(r);
    }
    if let Some(r) = last {
        println!(
            "  {label}: {} | order_ms median={:.4} max={:.4} min={:.4} (trials: {})",
            describe(&r),
            median(&times),
            times.iter().cloned().fold(f64::MIN, f64::max),
            times.iter().cloned().fold(f64::MAX, f64::min),
            times.iter().map(|x| format!("{x:.4}")).collect::<Vec<_>>().join(" / ")
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut trials = 5usize;
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
                eprintln!("USAGE ERROR: unknown argument {other}\nusage: world_move_bench [--trials N]");
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
    println!("file_load_ms (map json; not part of any order cost): {:.3}", ms(t.elapsed()));

    // ---- (1) 検証地域(既存の区画と検証用シナリオ)
    println!("\n=== (1) verification region (29 provinces; division_movement_scenario.ron) ===");
    let world = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).expect("world");
    let t = Instant::now();
    let graph = ProvinceGraph::build(&world.model, &map).expect("graph");
    println!("graph_build_ms (not part of an order): {:.4} | nodes={} edges={}", ms(t.elapsed()), graph.node_count(), graph.edge_count());
    let t = Instant::now();
    let scenario = load_division_scenario("assets/data/world/preview/division_movement_scenario.ron", &graph, &world.model).expect("scenario");
    println!("scenario_load_ms (not part of an order): {:.4}", ms(t.elapsed()));
    let base = scenario.into_military();
    measure("18309 -> 18308 (adjacent, same state)", &base, &graph, MapDivisionId(1), ProvinceId(18308), trials, limit);
    measure("18309 -> 17266 (5 hops across the state boundary via 17903)", &base, &graph, MapDivisionId(1), ProvinceId(17266), trials, limit);
    measure("18309 -> 17269 (state 21, controlled by country 2: rejected)", &base, &graph, MapDivisionId(1), ProvinceId(17269), trials, limit);
    measure("18309 -> Province#99999 (unknown)", &base, &graph, MapDivisionId(1), ProvinceId(99999), trials, limit);
    {
        // 17903 の明示を外した条件(同じ区画・同じ師団で、支配だけを変える): NoPath
        let mut b2 = base.clone();
        b2.set_controller(ProvinceId(17903), None);
        measure("18309 -> 17266 without the 17903 override (NoPath; the whole reachable part is explored)", &b2, &graph, MapDivisionId(1), ProvinceId(17266), trials, limit);
    }

    // ---- (2) 測定専用の 35,221 ノード
    println!("\n=== (2) measurement-only graph (1 land cell = 1 province; NOT the official world definition) ===");
    let t = Instant::now();
    let (defs, facts) = land_definitions(&map, MeasureRule::DEFAULT);
    println!("definitions_ms (in memory only): {:.3} | land_cells={} provinces={}", ms(t.elapsed()), facts.land_cells, facts.provinces);
    let t = Instant::now();
    let big = ProvinceGraph::from_definitions(&defs, &map, Coverage::World).expect("graph");
    println!("graph_build_ms (not part of an order): {:.3} | nodes={} edges={}", ms(t.elapsed()), big.node_count(), big.edge_count());
    // 測定専用の支配: ProvinceId % 97 == 0 は国家2、他は国家1。
    let c1 = CountryId(1);
    let c2 = CountryId(2);
    let mut control = std::collections::BTreeMap::new();
    for p in big.node_ids() {
        control.insert(p, if p.0 % 97 == 0 { c2 } else { c1 });
    }
    let pass1 = ExplicitPassable::new(&big, big.node_ids().filter(|p| p.0 % 97 != 0)).expect("ids");
    let comps = big.components_among(&pass1);
    let largest = comps.iter().fold(None::<&Vec<ProvinceId>>, |acc, c| match acc {
        Some(a) if a.len() >= c.len() => Some(a),
        _ => Some(c),
    });
    let largest = largest.expect("a component");
    let smallest_other = comps.iter().filter(|c| !std::ptr::eq(*c, largest)).min_by_key(|c| c.len()).expect("another component");
    println!(
        "country 1 controlled part: {} provinces in {} connected component(s); largest has {} provinces (measurement-only rule: ProvinceId % 97 == 0 belongs to country 2)",
        pass1.len(),
        comps.len(),
        largest.len()
    );
    let src = largest[largest.len() / 2];
    let division = MapDivision::new(MapDivisionId(1), c1, src, 1.0);
    let base = MapMilitary::new(vec![division], ProvinceControl::new(control), MovementParams { base_days: 5.0 }).expect("military");
    let adjacent = big.neighbor_ids(src).find(|n| n.0 % 97 != 0).expect("a controlled neighbour");
    let far_a = *largest.first().expect("nonempty");
    let far_b = *largest.last().expect("nonempty");
    let other = smallest_other[0];
    let blocked = big.node_ids().find(|p| p.0 % 97 == 0).expect("a country-2 province");
    println!("source Province#{} (middle of the largest component)", src.0);
    measure(&format!("adjacent: -> {}", adjacent.0), &base, &big, MapDivisionId(1), adjacent, trials, limit);
    measure(&format!("far (lowest id in the component): -> {}", far_a.0), &base, &big, MapDivisionId(1), far_a, trials, limit);
    measure(&format!("far (highest id in the component): -> {}", far_b.0), &base, &big, MapDivisionId(1), far_b, trials, limit);
    measure(
        &format!("NoPath: -> {} (country 1's, but in another component of {} provinces; the whole source component is explored)", other.0, smallest_other.len()),
        &base,
        &big,
        MapDivisionId(1),
        other,
        trials,
        limit,
    );
    measure(&format!("rejected (destination controlled by country 2): -> {}", blocked.0), &base, &big, MapDivisionId(1), blocked, trials, limit);
    measure("unknown province: -> 4294967295", &base, &big, MapDivisionId(1), ProvinceId(u32::MAX), trials, limit);
    println!("\nnote: order_ms covers `issue_move_order` only (validation, pathfinding, applying the order); the clone of the unordered state, graph build and file load are excluded.");
}
