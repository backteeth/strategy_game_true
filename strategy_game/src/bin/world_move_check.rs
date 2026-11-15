//! P32-034 (M6-a): 新経路の師団の初期配置と、自国支配のプロヴィンス間の日単位の移動の**確認用コマンド**(読み取り専用。保存・アセットは書かない)。
//!
//! 検証用シナリオ(`division_movement_scenario.ron`)を読み、移動命令と日次処理を実行して、各日の位置・状態・進捗を標準出力へ出す。
//! **シナリオの所有・支配は検証入力であり、正式な世界データではない。** 戦争関係・戦闘・占領・日次補給・前線/AI・UI・保存には接続しない。
//!
//! 実行(`strategy_game/` で):
//!   cargo run --release --bin world_move_check -- --days 5 --order 1:1:18308
//!   cargo run --release --bin world_move_check -- --days 25 --order 1:1:17266
//!   cargo run --release --bin world_move_check -- --scenario assets/data/world/preview/division_movement_scenario_no_bridge.ron --days 3 --order 1:1:17266
//!
//! 引数:
//!   --scenario <path>          検証用シナリオ(既定 assets/data/world/preview/division_movement_scenario.ron)
//!   --map <path> / --world-provinces <path> / --world-states <path>   既存の地図・区画(既定は検証地域の既存アセット)
//!   --days <n>                 処理する日数(既定 0 = 初期配置だけ表示)
//!   --order <日>:<師団ID>:<ProvinceId>      移動命令。その日の**初めに**(日次処理の前に)出す。繰り返し可
//!   --control <日>:<ProvinceId>:<国家ラベル|none>   その日の初めに支配国を変える(命令後の通行条件の変更の検証用。占領ではない)。繰り返し可
//! 日の流れ(1日目から): 支配の変更 → 命令(同じ日の中は指定順)→ 日次処理(経路の検証 → 移動。師団ID昇順)→ 各師団の状態を表示。
//! 命令の拒否は**正常な結果**(終了コード 0。`rejected:` と理由を出す)。終了コード: 0 = 実行できた、64 = 引数の誤り、
//! 65 = 入力不正(検証に失敗したシナリオ、`--control` の未知の ID・国家ラベル)、66 = 読み込みの失敗(地図・区画・シナリオが読めない、RON として不正、非対応の版)。

use std::collections::HashMap;

use strategy_game::common::CountryId;
use strategy_game::hexmap::data::load_from_path;
use strategy_game::world::{
    BattleOutcome, ClaimReason, DivisionScenarioError, FinishReason, MapDivision, MapDivisionId, MapDivisionStatus, MapMilitary, MoveEvent, OrderOutcome, ProvinceGraph,
    ProvinceId, StopReason, load_division_scenario, load_world_preview,
};

const USAGE: &str = "usage: world_move_check [--scenario PATH] [--map P] [--world-provinces P] [--world-states P] [--days N] \
[--order DAY:DIVISION:PROVINCE]... [--control DAY:PROVINCE:LABEL|none]... [--combat]";

#[derive(Default)]
struct Args {
    scenario: Option<String>,
    map: Option<String>,
    provinces: Option<String>,
    states: Option<String>,
    days: u32,
    orders: Vec<(u32, u32, u32)>,
    controls: Vec<(u32, u32, Option<u32>)>,
    combat: bool,
}

fn num(name: &str, s: &str) -> Result<u32, String> {
    s.trim().parse::<u32>().map_err(|_| format!("{name}: \"{s}\" is not a non-negative integer"))
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut i = 0;
    while i < args.len() {
        let name = args[i].as_str();
        let mut value = || {
            i += 1;
            args.get(i).cloned().ok_or_else(|| format!("{name} needs a value"))
        };
        match name {
            "--scenario" => a.scenario = Some(value()?),
            "--map" => a.map = Some(value()?),
            "--world-provinces" => a.provinces = Some(value()?),
            "--world-states" => a.states = Some(value()?),
            "--days" => a.days = num(name, &value()?)?,
            "--order" => {
                let v = value()?;
                let parts: Vec<&str> = v.split(':').collect();
                if parts.len() != 3 {
                    return Err(format!("--order: \"{v}\" is not DAY:DIVISION:PROVINCE"));
                }
                let day = num("--order day", parts[0])?;
                if day == 0 {
                    return Err("--order: the day starts at 1".into());
                }
                a.orders.push((day, num("--order division", parts[1])?, num("--order province", parts[2])?));
            }
            "--control" => {
                let v = value()?;
                let parts: Vec<&str> = v.split(':').collect();
                if parts.len() != 3 {
                    return Err(format!("--control: \"{v}\" is not DAY:PROVINCE:LABEL|none"));
                }
                let day = num("--control day", parts[0])?;
                if day == 0 {
                    return Err("--control: the day starts at 1".into());
                }
                let label = if parts[2] == "none" { None } else { Some(num("--control label", parts[2])?) };
                a.controls.push((day, num("--control province", parts[1])?, label));
            }
            "--combat" => a.combat = true,
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
        i += 1;
    }
    Ok(a)
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

fn path_text(p: &[ProvinceId]) -> String {
    if p.is_empty() { "-".into() } else { p.iter().map(|x| x.0.to_string()).collect::<Vec<_>>().join(">") }
}

fn opt(p: Option<ProvinceId>) -> String {
    p.map(|x| x.0.to_string()).unwrap_or_else(|| "-".into())
}

fn print_state(day: u32, m: &MapMilitary, labels: &HashMap<CountryId, u32>, combat: bool) {
    for d in m.divisions() {
        print_division(day, d, labels, combat);
    }
}

fn print_division(day: u32, d: &MapDivision, labels: &HashMap<CountryId, u32>, combat: bool) {
    println!(
        "day {day} {} (country {}) at {} {} progress {:.2} destination {} next {} remaining {}{}{}",
        d.id,
        labels.get(&d.owner).map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
        d.province.0,
        match d.status {
            MapDivisionStatus::Idle => "Idle",
            MapDivisionStatus::Moving => "Moving",
            MapDivisionStatus::Fighting => "Fighting",
        },
        d.progress,
        opt(d.destination),
        opt(d.target),
        path_text(&d.path),
        d.battle.map(|b| format!(" battle {}", b.0)).unwrap_or_default(),
        if combat { format!(" manpower {} organization {:.1} equipment {:.2}", d.combat.manpower, d.combat.organization, d.combat.equipment) } else { String::new() }
    );
}

fn run(a: &Args) -> i32 {
    let map_path = a.map.clone().unwrap_or_else(|| "assets/data/hex_atlas_map.json".into());
    let prov_path = a.provinces.clone().unwrap_or_else(|| "assets/data/world/preview/provinces.ron".into());
    let state_path = a.states.clone().unwrap_or_else(|| "assets/data/world/preview/states.ron".into());
    let scenario_path = a.scenario.clone().unwrap_or_else(|| "assets/data/world/preview/division_movement_scenario.ron".into());
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
    let scenario = match load_division_scenario(&scenario_path, &graph, &world.model) {
        Ok(s) => s,
        Err(DivisionScenarioError::Invalid(issues)) => {
            println!("SCENARIO INPUT ERROR: {scenario_path}: {} issue(s)", issues.len());
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
    let labels: HashMap<CountryId, u32> = scenario.countries.iter().map(|(l, c)| (*c, *l)).collect();
    // --control の参照を事前に確認(未知のプロヴィンス・国家ラベルは入力不正)
    let mut controls: Vec<(u32, ProvinceId, Option<CountryId>)> = Vec::new();
    for (day, p, label) in &a.controls {
        if !graph.contains(ProvinceId(*p)) {
            println!("INPUT ERROR: --control: Province#{p} is not a province of the graph");
            return 65;
        }
        let c = match label {
            Some(l) => match scenario.countries.get(l) {
                Some(c) => Some(*c),
                None => {
                    println!("INPUT ERROR: --control: country label {l} is not defined in the scenario");
                    return 65;
                }
            },
            None => None,
        };
        controls.push((*day, ProvinceId(*p), c));
    }
    println!("=== 師団の移動 (M6-a: verification input, not official ownership or control) ===");
    println!("world: {prov_path} / {state_path}");
    println!("scenario: {scenario_path}");
    println!("description: {}", scenario.description);
    println!("graph: nodes={} edges={}", graph.node_count(), graph.edge_count());
    println!(
        "countries (label -> CountryId): {}",
        scenario.countries.iter().map(|(l, c)| format!("{l}->{}", c.0)).collect::<Vec<_>>().join(" ")
    );
    println!("initial controlled provinces: {}", scenario.control.len());
    println!("base_days={} supply_modifier=1.0 (fixed) step_cost=1 (rivers do not matter)", scenario.params.base_days);
    println!("divisions: {}", scenario.divisions.len());
    if !scenario.wars.is_empty() {
        println!(
            "wars (both directions): {}",
            scenario
                .wars
                .iter()
                .map(|(x, y)| format!("{}-{}", labels.get(x).map(|l| l.to_string()).unwrap_or_else(|| "?".into()), labels.get(y).map(|l| l.to_string()).unwrap_or_else(|| "?".into())))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    let mut military = scenario.into_military();
    println!("\n--- day 0 (initial placement) ---");
    print_state(0, &military, &labels, a.combat);
    let mut rejected = 0usize;
    for day in 1..=a.days {
        println!("\n--- day {day} ---");
        for (d, p, c) in controls.iter().filter(|(d, _, _)| *d == day) {
            let _ = d;
            military.set_controller(*p, *c);
            println!(
                "control change: {p} -> {}",
                match c {
                    Some(c) => format!("country {}", labels.get(c).map(|l| l.to_string()).unwrap_or_else(|| "?".into())),
                    None => "no controller".into(),
                }
            );
        }
        for (_, div, p) in a.orders.iter().filter(|(d, _, _)| *d == day) {
            let id = MapDivisionId(*div);
            match military.issue_move_order(&graph, id, ProvinceId(*p)) {
                Ok(OrderOutcome::Started { hops }) => println!("order {id} -> Province#{p}: accepted ({hops} hop(s))"),
                Ok(OrderOutcome::AlreadyThere) => println!("order {id} -> Province#{p}: no-op (already there)"),
                Ok(OrderOutcome::AlreadyMoving) => println!("order {id} -> Province#{p}: no-op (already moving to it)"),
                Err(r) => {
                    rejected += 1;
                    println!("order {id} -> Province#{p}: rejected: {r}");
                }
            }
        }
        for e in military.advance_day(&graph) {
            match e {
                MoveEvent::HopCompleted { division, province } => println!("event: {division} reached {province}"),
                MoveEvent::Arrived { division, province } => println!("event: {division} arrived at its destination {province}"),
                MoveEvent::Stopped { division, at, blocked } => {
                    println!("event: {division} stopped at {at} before entering {blocked} (no longer passable)")
                }
                MoveEvent::PathTruncated { division } => println!("event: {division} path truncated (a later province is no longer passable)"),
                MoveEvent::StoppedBefore { division, at, blocked, reason } => println!(
                    "event: {division} stopped at {at} before entering {blocked} ({})",
                    match reason {
                        StopReason::NotPassable => "no longer passable",
                        StopReason::ContestedByOthers => "a battle of another country is going on there; it cannot join",
                    }
                ),
                MoveEvent::BattleStarted { battle, province, attacker, defender, attackers, defenders } => println!(
                    "event: {battle} started at {province}: country {} attacks country {} (attackers {}; defenders {})",
                    labels.get(&attacker).map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
                    labels.get(&defender).map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
                    attackers.iter().map(|d| d.0.to_string()).collect::<Vec<_>>().join(","),
                    defenders.iter().map(|d| d.0.to_string()).collect::<Vec<_>>().join(",")
                ),
                MoveEvent::BattleJoined { battle, division, province } => println!("event: {division} joined {battle} at {province} as an attacker"),
                MoveEvent::Retreated { division, battle, from, to } => println!("event: {division} retreated from {from} to {to} ({battle})"),
                MoveEvent::Destroyed { division, battle, at } => println!("event: {division} destroyed at {at} ({battle}; no retreat or manpower exhausted)"),
                MoveEvent::BattleFinished { battle, province, attacker, defender, outcome, reason } => println!(
                    "event: {battle} finished at {province}: {} (attacker country {}, defender country {}; reason {})",
                    match outcome {
                        BattleOutcome::AttackerWon => "attacker won",
                        BattleOutcome::DefenderWon => "defender won",
                    },
                    labels.get(&attacker).map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
                    labels.get(&defender).map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
                    match reason {
                        FinishReason::DefenderGone => "defender gone",
                        FinishReason::AttackerGone => "attacker gone",
                        FinishReason::BothGone => "both gone",
                        FinishReason::ZeroDamage => "zero damage on both sides",
                    }
                ),
                MoveEvent::ControlClaim { province, claimant, previous_controller, reason, divisions } => println!(
                    "event: CONTROL CLAIM for M6-c (control NOT changed here): {province} by country {} (previous controller {}; reason {}; divisions {})",
                    labels.get(&claimant).map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
                    previous_controller.map(|c| labels.get(&c).map(|l| l.to_string()).unwrap_or_else(|| "?".into())).unwrap_or_else(|| "none".into()),
                    match reason {
                        ClaimReason::Undefended => "undefended".to_string(),
                        ClaimReason::AttackerWonBattle(b) => format!("attacker won {b}"),
                    },
                    divisions.iter().map(|d| d.0.to_string()).collect::<Vec<_>>().join(",")
                ),
            }
        }
        print_state(day, &military, &labels, a.combat);
    }
    let unused: Vec<u32> = a.orders.iter().filter(|(d, _, _)| *d > a.days).map(|(d, _, _)| *d).collect();
    if !unused.is_empty() {
        println!("\nnote: {} order(s) are after day {} and were not issued", unused.len(), a.days);
    }
    println!("\norders rejected: {rejected} (a rejection is a normal result)");
    0
}
