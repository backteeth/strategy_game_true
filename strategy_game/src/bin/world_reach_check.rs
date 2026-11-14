//! P32-030 (M5-a): プロヴィンスの陸上隣接グラフと、明示的な通行条件による到達判定の**確認用コマンド**(読み取り専用)。
//!
//! 既存の地図・区画の読み込みと検証(`world::load_world_preview`)を使い、`world::connectivity` の結果を標準出力へ表示する。
//! ファイル・アセット・保存は書き換えない。経済・保存・軍には接続しない。**到達可能は「十分な補給が届く」ではない**
//! (輸送能力・補給量は計算しない)。通行条件は、**利用者が引数で明示したものだけ**を使う(国家や範囲を列挙順から選ばない)。
//!
//! 実行(`strategy_game/` で):
//!   cargo run --release --bin world_reach_check                                  グラフの事実だけ
//!   cargo run --release --bin world_reach_check -- --source 18309 --passable all --target 17266
//!   cargo run --release --bin world_reach_check -- --source 18309 --passable-states 20,22
//!
//! 引数:
//!   --map <path>                 地図JSON(既定 assets/data/hex_atlas_map.json)
//!   --world-provinces <path>     プロヴィンス表(既定 assets/data/world/preview/provinces.ron)
//!   --world-states <path>        ステート配置(既定 assets/data/world/preview/states.ron)
//!   --source <ProvinceId>        起点(繰り返し可)
//!   --passable all               グラフの全ノードを通行可能にする(**利用者の明示的な指定**)
//!   --passable-provinces a,b,..  通行可能な ProvinceId
//!   --passable-states a,b,..     通行可能な MapStateId の所属プロヴィンス(**名前つきの変換**。所有・支配の導出ではない)
//!   --blocked-provinces a,b,..   通行可能な集合から取り除く ProvinceId
//!   --blocked-states a,b,..      通行可能な集合から取り除く MapStateId の所属プロヴィンス
//!   --target <ProvinceId>        判定する対象(繰り返し可)
//!   --target-cell <q,r>          判定する対象のセル(繰り返し可。水域・範囲外の陸は「対象外」)
//!   --no-list                    到達したプロヴィンスの一覧を出さない
//!   --state-summary              MapStateId 単位の到達要約を出す(P32-031。Reach から導出。再探索しない)
//!   --scenario <path>            補給連結性の検証用シナリオ(RON。P32-032)から、主体ごとの補給源と通行可能範囲を読む。
//!                                主体はラベル昇順に、補給源・到達集合・ステート要約を出す(要約は常に出る)。
//!                                **個別指定(--source/--passable*/--blocked*/--target*)とは併用できない**(終了コード 64)。
//!   --scenario-agent <label>     シナリオのうち、指定したラベルの主体だけを出す(繰り返し可。--scenario が必要)
//! 通行可能の指定が1つもなく起点を指定した場合は入力不正(既定では何も通れない)。
//! 終了コード: 0 = 判定できた(到達不能・不明も正常な判定)、64 = 引数の誤り、65 = 入力不正(未知のID・通行不可の起点・
//! 検証に失敗したシナリオの内容など)、66 = 読み込みまたは検証の失敗(シナリオが読めない・RONとして不正・非対応の版を含む)。

use std::collections::BTreeSet;

use strategy_game::hexmap::data::load_from_path;
use strategy_game::world::{
    ExplicitPassable, HexCoord, MapStateId, ProvinceGraph, ProvinceId, Reachability, StateReachSummary, SupplyScenarioError, WorldModel,
    evaluate_supply_scenario, kind_label, load_supply_scenario, load_world_preview, reach, summarize_by_state, summarize_model,
};

const USAGE: &str = "usage: world_reach_check [--map P] [--world-provinces P] [--world-states P] [--source ID]... \
[--passable all] [--passable-provinces A,B] [--passable-states A,B] [--blocked-provinces A,B] [--blocked-states A,B] \
[--target ID]... [--target-cell Q,R]... [--no-list] [--state-summary] [--scenario PATH [--scenario-agent LABEL]...]";

#[derive(Default)]
struct Args {
    map: Option<String>,
    provinces: Option<String>,
    states: Option<String>,
    sources: Vec<u32>,
    pass_all: bool,
    pass_provinces: Vec<u32>,
    pass_states: Vec<u32>,
    blocked_provinces: Vec<u32>,
    blocked_states: Vec<u32>,
    targets: Vec<u32>,
    target_cells: Vec<(i32, i32)>,
    no_list: bool,
    state_summary: bool,
    scenario: Option<String>,
    scenario_agents: Vec<u32>,
    any_pass_spec: bool,
}

fn parse_list(name: &str, v: &str) -> Result<Vec<u32>, String> {
    v.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().parse::<u32>().map_err(|_| format!("{name}: \"{s}\" is not a non-negative integer")))
        .collect()
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut i = 0;
    while i < args.len() {
        let name = args[i].as_str();
        let value = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            args.get(*i).cloned().ok_or_else(|| format!("{name} needs a value"))
        };
        match name {
            "--map" => a.map = Some(value(&mut i)?),
            "--world-provinces" => a.provinces = Some(value(&mut i)?),
            "--world-states" => a.states = Some(value(&mut i)?),
            "--source" => a.sources.push(parse_list(name, &value(&mut i)?)?.first().copied().ok_or("--source needs an id")?),
            "--target" => a.targets.push(parse_list(name, &value(&mut i)?)?.first().copied().ok_or("--target needs an id")?),
            "--passable" => {
                let v = value(&mut i)?;
                if v != "all" {
                    return Err(format!("--passable: \"{v}\" is not supported (only \"all\"; use --passable-provinces / --passable-states)"));
                }
                a.pass_all = true;
                a.any_pass_spec = true;
            }
            "--passable-provinces" => {
                a.pass_provinces.extend(parse_list(name, &value(&mut i)?)?);
                a.any_pass_spec = true;
            }
            "--passable-states" => {
                a.pass_states.extend(parse_list(name, &value(&mut i)?)?);
                a.any_pass_spec = true;
            }
            "--blocked-provinces" => a.blocked_provinces.extend(parse_list(name, &value(&mut i)?)?),
            "--blocked-states" => a.blocked_states.extend(parse_list(name, &value(&mut i)?)?),
            "--target-cell" => {
                let v = value(&mut i)?;
                let (q, r) = v.split_once(',').ok_or_else(|| format!("--target-cell: \"{v}\" is not Q,R"))?;
                let q = q.trim().parse::<i32>().map_err(|_| format!("--target-cell: bad q in \"{v}\""))?;
                let r = r.trim().parse::<i32>().map_err(|_| format!("--target-cell: bad r in \"{v}\""))?;
                a.target_cells.push((q, r));
            }
            "--no-list" => a.no_list = true,
            "--state-summary" => a.state_summary = true,
            "--scenario" => a.scenario = Some(value(&mut i)?),
            "--scenario-agent" => a.scenario_agents.push(parse_list(name, &value(&mut i)?)?.first().copied().ok_or("--scenario-agent needs a label")?),
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
        i += 1;
    }
    let individual = !a.sources.is_empty()
        || a.any_pass_spec
        || !a.blocked_provinces.is_empty()
        || !a.blocked_states.is_empty()
        || !a.targets.is_empty()
        || !a.target_cells.is_empty();
    if a.scenario.is_some() && individual {
        return Err("--scenario cannot be combined with the individual options (--source/--passable*/--blocked*/--target*); the scenario file supplies sources and passable ranges".into());
    }
    if a.scenario.is_none() && !a.scenario_agents.is_empty() {
        return Err("--scenario-agent needs --scenario".into());
    }
    Ok(a)
}

fn ids(v: &[ProvinceId]) -> String {
    v.iter().map(|p| p.0.to_string()).collect::<Vec<_>>().join(" ")
}

fn verdict(v: Reachability) -> String {
    use strategy_game::world::{NotApplicableReason as N, UnreachableReason as U};
    match v {
        Reachability::Reachable => "到達可能 (reachable)".into(),
        Reachability::Unreachable(U::NotPassable) => "到達不能・確定 (unreachable: the target is not passable under the given condition)".into(),
        Reachability::Unreachable(U::NoRoute) => "到達不能・確定 (unreachable: no route even if every out-of-scope land cell were passable)".into(),
        Reachability::Unknown => "不明 (unknown: a route through out-of-scope land is possible, the data is incomplete)".into(),
        Reachability::NotApplicable(N::UnknownProvince) => "対象外 (not applicable: not a province of the graph)".into(),
        Reachability::NotApplicable(N::Water) => "対象外 (not applicable: water cell)".into(),
        Reachability::NotApplicable(N::OutOfScopeLand) => "対象外 (not applicable: land outside the province table)".into(),
        Reachability::NotApplicable(N::OutsideMap) => "対象外 (not applicable: outside the map)".into(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a = match parse_args(&args) {
        Ok(a) => a,
        Err(m) => {
            if !m.is_empty() {
                eprintln!("USAGE ERROR: {m}");
            }
            eprintln!("{USAGE}");
            std::process::exit(if m.is_empty() { 0 } else { 64 });
        }
    };
    std::process::exit(run(&a));
}

fn run(a: &Args) -> i32 {
    let map_path = a.map.clone().unwrap_or_else(|| "assets/data/hex_atlas_map.json".into());
    let prov_path = a.provinces.clone().unwrap_or_else(|| "assets/data/world/preview/provinces.ron".into());
    let state_path = a.states.clone().unwrap_or_else(|| "assets/data/world/preview/states.ron".into());
    let map = match load_from_path(&map_path) {
        Ok(m) => m,
        Err(e) => {
            println!("LOAD ERROR: the map could not be loaded: {e}");
            return 66;
        }
    };
    let world = match load_world_preview(&prov_path, &state_path, &map) {
        Ok(w) => w,
        Err(e) => {
            println!("LOAD ERROR: the world definition could not be loaded or validated:\n{e}");
            return 66;
        }
    };
    let graph = match ProvinceGraph::build(&world.model, &map) {
        Ok(g) => g,
        Err(e) => {
            println!("LOAD ERROR: the province graph could not be built: {e}");
            return 66;
        }
    };

    // ---- グラフの事実
    let scope = graph.scope();
    let comps = graph.components();
    println!("=== グラフ (province land-adjacency graph) ===");
    println!("map: {map_path}");
    println!("world: {prov_path} / {state_path}");
    println!("nodes: {}", graph.node_count());
    println!("undirected edges (deduplicated): {}", graph.edge_count());
    println!("  of which cross-state (both ends assigned to different states): {}", graph.cross_state_edge_count());
    println!("  of which with a river on a shared side (recorded only; rivers do not block): {}", graph.river_edge_count());
    println!("connected components: {} (sizes: {})", comps.len(), comps.iter().map(|c| c.len().to_string()).collect::<Vec<_>>().join(" "));
    println!(
        "scope: coverage={:?} unassigned_nodes={} out_of_scope_land_cells={} complete={}",
        scope.coverage,
        scope.unassigned_nodes,
        scope.out_of_scope_land_cells,
        scope.is_complete()
    );
    println!("(out-of-scope land has no ProvinceId and is never used as a bridge; it only makes some results `unknown`)");

    if let Some(path) = &a.scenario {
        return run_scenario(a, path, &graph, &world.model);
    }

    // ---- 通行条件(利用者が明示したものだけ)
    let mut passable: BTreeSet<ProvinceId> = BTreeSet::new();
    let mut input_errors: Vec<String> = Vec::new();
    if a.pass_all {
        passable.extend(ExplicitPassable::all_nodes(&graph).ids());
    }
    if !a.pass_provinces.is_empty() {
        match ExplicitPassable::new(&graph, a.pass_provinces.iter().map(|p| ProvinceId(*p))) {
            Ok(p) => passable.extend(p.ids()),
            Err(e) => input_errors.push(format!("--passable-provinces: {e}")),
        }
    }
    if !a.pass_states.is_empty() {
        let st: Vec<MapStateId> = a.pass_states.iter().map(|s| MapStateId(*s)).collect();
        match ExplicitPassable::from_states(&graph, &st) {
            Ok(p) => passable.extend(p.ids()),
            Err(missing) => input_errors.push(format!(
                "--passable-states: no province belongs to {}",
                missing.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(", ")
            )),
        }
    }
    let mut blocked: BTreeSet<ProvinceId> = BTreeSet::new();
    if !a.blocked_provinces.is_empty() {
        match ExplicitPassable::new(&graph, a.blocked_provinces.iter().map(|p| ProvinceId(*p))) {
            Ok(p) => blocked.extend(p.ids()),
            Err(e) => input_errors.push(format!("--blocked-provinces: {e}")),
        }
    }
    if !a.blocked_states.is_empty() {
        let st: Vec<MapStateId> = a.blocked_states.iter().map(|s| MapStateId(*s)).collect();
        match ExplicitPassable::from_states(&graph, &st) {
            Ok(p) => blocked.extend(p.ids()),
            Err(missing) => input_errors.push(format!(
                "--blocked-states: no province belongs to {}",
                missing.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(", ")
            )),
        }
    }
    if !input_errors.is_empty() {
        for e in &input_errors {
            println!("INPUT ERROR: {e}");
        }
        return 65;
    }
    let pass = match ExplicitPassable::new(&graph, passable.iter().copied()) {
        Ok(p) => p,
        Err(e) => {
            println!("INPUT ERROR: {e}");
            return 65;
        }
    };
    let pass = pass.without(&ExplicitPassable::new(&graph, blocked.iter().copied()).unwrap_or_default());

    if a.any_pass_spec {
        println!("\n=== 通行条件 (explicit; nothing else is passable) ===");
        println!("passable provinces: {}", pass.len());
        let pc = graph.components_among(&pass);
        println!("components of the passable part: {} (sizes: {})", pc.len(), pc.iter().map(|c| c.len().to_string()).collect::<Vec<_>>().join(" "));
    }

    // ---- 到達判定
    if a.sources.is_empty() {
        if !a.targets.is_empty() || !a.target_cells.is_empty() {
            println!("INPUT ERROR: targets were given but no --source");
            return 65;
        }
        return 0;
    }
    if !a.any_pass_spec {
        println!("INPUT ERROR: no passable condition was given (--passable all / --passable-provinces / --passable-states); nothing is passable by default");
        return 65;
    }
    let sources: Vec<ProvinceId> = a.sources.iter().map(|s| ProvinceId(*s)).collect();
    let result = match reach(&graph, &sources, &pass) {
        Ok(r) => r,
        Err(e) => {
            println!("INPUT ERROR: {e}");
            return 65;
        }
    };
    println!("\n=== 到達判定 (pure reachability; not a supply amount, not a capacity) ===");
    println!("sources: {}", ids(result.sources()));
    println!("reached: {}", result.reached_count());
    println!("unknown (reachable only through out-of-scope land): {}", result.unknown_count());
    println!("reached set touches out-of-scope land: {}", result.touches_out_of_scope());
    if !a.no_list {
        println!("reached provinces: {}", ids(&result.reached_ids()));
        if result.unknown_count() > 0 {
            println!("unknown provinces: {}", ids(&result.unknown_ids()));
        }
    }
    for t in &a.targets {
        println!("target Province#{t}: {}", verdict(result.reachability(ProvinceId(*t))));
    }
    for (q, r) in &a.target_cells {
        println!("target cell (q={q}, r={r}): {}", verdict(result.reachability_of_cell(&graph, &map, HexCoord::new(*q, *r))));
    }
    if a.state_summary {
        // 所属の正本(WorldModel)から導出した要約と、グラフのラベルから導出した要約は一致する(不一致なら異常として終了)。
        let by_model = summarize_model(&world.model, &result);
        let by_graph = summarize_by_state(&graph, &result);
        match (by_model, by_graph) {
            (Ok(m), Ok(g)) => {
                println!("
=== ステート単位の要約 (derived from the province result; no search on a state graph) ===");
                print_state_table(&m);
                let g_tot = m.grand_totals();
                println!("grand total: {} provinces (graph nodes: {}); summary from graph labels identical to summary from the model: {}", g_tot.total, graph.node_count(), g.states() == m.states() && g.unassigned() == m.unassigned());
            }
            (m, g) => {
                println!("INTERNAL ERROR: state summary failed (model: {:?}, graph: {:?})", m.err(), g.err());
                return 66;
            }
        }
    }
    0
}

/// ステート要約の表(`--state-summary` とシナリオ出力で共通)。
fn print_state_table(m: &StateReachSummary) {
    println!("state    total reachable unreachable (blocked)  unknown   n/a  kind");
    for s in m.states() {
        let c = &s.counts;
        println!(
            "{:<8} {:>5} {:>9} {:>11} {:>9} {:>8} {:>5}  {}",
            s.state.0, c.total, c.reachable, c.unreachable(), c.unreachable_not_passable, c.unknown, c.not_applicable, kind_label(s.kind)
        );
    }
    let u = m.unassigned();
    println!(
        "{:<8} {:>5} {:>9} {:>11} {:>9} {:>8} {:>5}  (not counted in any state)",
        "unassigned", u.total, u.reachable, u.unreachable(), u.unreachable_not_passable, u.unknown, u.not_applicable
    );
}

/// シナリオ入力(P32-032)。個別指定とは併用できない(引数の段階で拒否済み)。
/// 失敗の区別: 読めない・RONとして不正・非対応の版 = 66、検証に失敗した内容 = 65、到達不能・不明は正常な判定(0)。
fn run_scenario(a: &Args, path: &str, graph: &ProvinceGraph, model: &WorldModel) -> i32 {
    let scenario = match load_supply_scenario(path, graph, model) {
        Ok(s) => s,
        Err(SupplyScenarioError::Invalid(issues)) => {
            println!("SCENARIO INPUT ERROR: {path}: {} issue(s)", issues.len());
            for i in &issues {
                println!("  {i}");
            }
            return 65;
        }
        Err(e) => {
            println!("SCENARIO LOAD ERROR: {e}");
            return 66;
        }
    };
    let known: Vec<u32> = scenario.agents.iter().map(|x| x.label).collect();
    let unknown: Vec<String> = a.scenario_agents.iter().filter(|l| !known.contains(l)).map(|l| l.to_string()).collect();
    if !unknown.is_empty() {
        println!("INPUT ERROR: --scenario-agent: no agent has the label {} (labels in the scenario: {})", unknown.join(", "), known.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" "));
        return 65;
    }
    let outcomes = match evaluate_supply_scenario(graph, model, &scenario) {
        Ok(o) => o,
        Err(e) => {
            println!("INTERNAL ERROR: {e}");
            return 66;
        }
    };
    println!("
=== 補給連結性シナリオ (verification input: the passable ranges are NOT official borders or control) ===");
    println!("scenario: {path}");
    println!("description: {}", scenario.description);
    println!("agents: {} (printed in label order)", scenario.agents.len());
    for (agent, out) in scenario.agents.iter().zip(&outcomes) {
        if !a.scenario_agents.is_empty() && !a.scenario_agents.contains(&agent.label) {
            continue;
        }
        println!("
--- agent {} ---", agent.label);
        println!("sources: {}", ids(&agent.sources));
        println!(
            "passable: {} provinces (explicit provinces: {}; states: {})",
            agent.passable.len(),
            agent.explicit_provinces.len(),
            if agent.states.is_empty() { "-".to_string() } else { agent.states.iter().map(|s| s.0.to_string()).collect::<Vec<_>>().join(" ") }
        );
        if !agent.explicit_provinces.is_empty() {
            println!("explicit provinces: {}", ids(&agent.explicit_provinces));
        }
        let r = &out.reach;
        println!("reached: {}", r.reached_count());
        println!("unknown (reachable only through out-of-scope land): {}", r.unknown_count());
        println!("reached set touches out-of-scope land: {}", r.touches_out_of_scope());
        if !a.no_list {
            println!("reached provinces: {}", ids(&r.reached_ids()));
            if r.unknown_count() > 0 {
                println!("unknown provinces: {}", ids(&r.unknown_ids()));
            }
        }
        println!("state summary (derived from this agent's province result):");
        print_state_table(&out.summary);
        println!("grand total: {} provinces (graph nodes: {})", out.summary.grand_totals().total, graph.node_count());
    }
    0
}
