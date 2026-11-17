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

use std::collections::{BTreeMap, HashMap};

use strategy_game::world::{DisbandReason, MapBattleId, Territory, TransferContext, TransferError, TransferOutcome, TransferRequest, apply_transfer, plan_transfer};

use strategy_game::common::CountryId;
use strategy_game::hexmap::data::load_from_path;
use strategy_game::world::{
    BattleOutcome, ClaimReason, ClaimRejection, DivisionScenarioError, FinishReason, MapDivision, MapDivisionId, MapDivisionStatus, MapMilitary, MoveEvent, OrderOutcome, ProvinceGraph,
    ProvinceId, StopReason, countries_to_invalidate, load_division_scenario, load_world_preview, MapStateId, StateControlKind, WorldModel, control_kind_label, summarize_control,
};

const USAGE: &str = "usage: world_move_check [--scenario PATH] [--map P] [--world-provinces P] [--world-states P] [--days N] \
[--order DAY:DIVISION:PROVINCE]... [--control DAY:PROVINCE:LABEL|none]... [--capital DAY:LABEL:PROVINCE]... [--combat] [--state-control] [--transfer DAY:STATE:FROM_LABEL:TO_LABEL[:BATTLE,BATTLE...]]...";

#[derive(Default)]
struct Args {
    scenario: Option<String>,
    map: Option<String>,
    provinces: Option<String>,
    states: Option<String>,
    days: u32,
    orders: Vec<(u32, u32, u32)>,
    controls: Vec<(u32, u32, Option<u32>)>,
    capitals: Vec<(u32, u32, u32)>,
    combat: bool,
    state_control: bool,
    transfers: Vec<(u32, u32, u32, u32, Vec<u32>)>,
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
            "--capital" => {
                let v = value()?;
                let parts: Vec<&str> = v.split(':').collect();
                if parts.len() != 3 {
                    return Err(format!("--capital: \"{v}\" is not DAY:LABEL:PROVINCE"));
                }
                let day = num("--capital day", parts[0])?;
                if day == 0 {
                    return Err("--capital: the day starts at 1".into());
                }
                a.capitals.push((day, num("--capital label", parts[1])?, num("--capital province", parts[2])?));
            }
            "--combat" => a.combat = true,
            "--state-control" => a.state_control = true,
            "--transfer" => {
                let v = value()?;
                let parts: Vec<&str> = v.split(':').collect();
                if parts.len() != 4 && parts.len() != 5 {
                    return Err(format!("--transfer: \"{v}\" is not DAY:STATE:FROM_LABEL:TO_LABEL[:BATTLE,BATTLE...]"));
                }
                let day = num("--transfer day", parts[0])?;
                if day == 0 {
                    return Err("--transfer: the day starts at 1".into());
                }
                let mut battles = Vec::new();
                if parts.len() == 5 {
                    for b in parts[4].split(',').filter(|b| !b.is_empty()) {
                        battles.push(num("--transfer battle", b)?);
                    }
                }
                a.transfers.push((day, num("--transfer state", parts[1])?, num("--transfer from", parts[2])?, num("--transfer to", parts[3])?, battles));
            }
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

/// M6-e: ステート単位の実効支配の集計(法的所有と実効支配は別々に出す。占領は交戦関係を明示して数え、第三国の支配は加算しない)。
fn rejection_hint(e: &TransferError) -> &'static str {
    match e {
        TransferError::NoCapitalCandidate { .. } => " [capital: no valid province in the remaining states]",
        TransferError::BattleInTransferredState { .. } => " [name the battle: --transfer DAY:STATE:FROM:TO:BATTLE_NUMBER]",
        _ => "",
    }
}

fn print_transfer_outcome(o: &TransferOutcome, labels: &HashMap<CountryId, u32>) {
    let name = |c: CountryId| labels.get(&c).map(|l| l.to_string()).unwrap_or_else(|| "?".into());
    let opt = |c: Option<CountryId>| c.map(name).unwrap_or_else(|| "none".into());
    println!("transfer APPLIED: {} legal owner country {} -> country {}", o.state, name(o.previous_owner), name(o.new_owner));
    println!("  control changes (all provinces of the state are set to country {}): {}", name(o.new_owner), o.control_changes.len());
    for c in &o.control_changes {
        println!("    {}: country {} -> country {}", c.province, opt(c.previous), opt(c.new));
    }
    for c in &o.third_party_resolved {
        println!("  third-party occupation resolved: {} held by country {}", c.province, opt(c.previous));
    }
    match &o.capital_move {
        Some(m) => println!("  capital moved: country {} {} -> {} (smallest province the country controls in the first remaining state with one; the representative if it is controlled)", name(m.country), m.from, m.to),
        None => println!("  capital: unchanged"),
    }
    for b in &o.cancelled_battles {
        println!(
            "  battle cancelled: {} at {} (attacker country {}, defender country {}); released divisions [{}]; no winner, no occupation",
            b.battle,
            b.province,
            name(b.attacker_country),
            name(b.defender_country),
            b.released.iter().map(|d| d.0.to_string()).collect::<Vec<_>>().join(",")
        );
    }
    if let Some(e) = &o.elimination {
        println!(
            "  COUNTRY ELIMINATED: country {} (annexed by country {}; the id is kept). wars removed: [{}]; supply setup removed: {}; capital removed: {}",
            name(e.country),
            name(e.annexed_by),
            e.wars_removed.iter().map(|(x, y)| format!("{}-{}", name(*x), name(*y))).collect::<Vec<_>>().join(" "),
            e.supply_removed,
            e.capital_removed.map(|p| p.to_string()).unwrap_or_else(|| "none".into())
        );
        for c in &e.control_reverted {
            println!("    control outside the state returned: {}: country {} -> country {}", c.province, opt(c.previous), opt(c.new));
        }
    }
    for r in &o.relocated {
        println!("  division {} (country {}) returned: {} -> {}", r.division.0, name(r.owner), r.from, r.to);
    }
    for d in &o.disbanded {
        println!(
            "  division {} (country {}) DISBANDED at {}: {} (not a battle loss)",
            d.division.0,
            name(d.owner),
            d.at,
            match d.reason {
                DisbandReason::CountryEliminated => "its country was eliminated",
                DisbandReason::NoReturnProvince => "no province to return to",
            }
        );
    }
    println!("  supply reach invalidated for countries: [{}]", o.invalidated_countries.iter().map(|c| name(*c)).collect::<Vec<_>>().join(", "));
}

fn print_state_control(day: u32, model: &WorldModel, owners: &BTreeMap<MapStateId, CountryId>, m: &MapMilitary, labels: &HashMap<CountryId, u32>) {
    let name = |c: CountryId| labels.get(&c).map(|l| l.to_string()).unwrap_or_else(|| "?".into());
    let summary = match summarize_control(model, owners, m.control()) {
        Ok(s) => s,
        Err(e) => {
            println!("state control (day {day}): summary FAILED: {e}");
            return;
        }
    };
    println!("state control (day {day}; derived from province control, nothing stored; consistent={}; unassigned provinces (in no state's denominator): {}):", summary.is_consistent(), summary.unassigned());
    let at_war = |a: CountryId, b: CountryId| m.at_war(a, b);
    for s in summary.states() {
        let owner = s.legal_owner.map(name).unwrap_or_else(|| "none".into());
        let kind = match s.kind() {
            StateControlKind::Full(c) => format!("{} (country {})", control_kind_label(s.kind()), name(c)),
            k => control_kind_label(k).to_string(),
        };
        let mut parts: Vec<String> = s.by_country.iter().map(|(c, n)| format!("country {} {n}/{}", name(*c), s.total)).collect();
        parts.push(format!("no controller {}/{}", s.uncontrolled, s.total));
        println!("  {} legal owner: country {owner} | provinces {} | {kind} | controllers: {}", s.state, s.total, parts.join(", "));
        for (c, n) in &s.by_country {
            if s.legal_owner == Some(*c) {
                continue;
            }
            match s.occupation_by(*c, &at_war) {
                Some(share) => println!("      occupation by country {} (at war with the owner, country {owner}): {}/{} = {:.3}", name(*c), share.numerator, share.denominator, share.as_f64()),
                None => println!("      control by country {} {n}/{} is NOT counted as occupation (not at war with the owner)", name(*c), s.total),
            }
        }
    }
}

fn print_state(day: u32, m: &MapMilitary, labels: &HashMap<CountryId, u32>, combat: bool) {
    let supply = m.supply_enabled();
    for d in m.divisions() {
        print_division(day, d, labels, combat, supply);
    }
}

fn print_division(day: u32, d: &MapDivision, labels: &HashMap<CountryId, u32>, combat: bool, supply: bool) {
    println!(
        "day {day} {} (country {}) at {} {} progress {:.2} destination {} next {} remaining {}{}{}{}",
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
        if combat { format!(" manpower {} organization {:.1} equipment {:.2}", d.combat.manpower, d.combat.organization, d.combat.equipment) } else { String::new() },
        if supply { format!(" supply {:.4}", d.fulfillment) } else { String::new() }
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
    let scenario_countries = scenario.countries.clone();
    let mut territory = match Territory::from_scenario(&scenario, &world.model, &graph) {
        Ok(t) => t,
        Err(e) => {
            println!("SCENARIO INPUT ERROR: {e}");
            return 65;
        }
    };
    for (_, _, from, to, _) in &a.transfers {
        for l in [from, to] {
            if !scenario.countries.contains_key(l) {
                println!("INPUT ERROR: --transfer: country label {l} is not defined in the scenario");
                return 65;
            }
        }
    }
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
    for (_, label, p) in &a.capitals {
        if !scenario.countries.contains_key(label) {
            println!("INPUT ERROR: --capital: country label {label} is not defined in the scenario");
            return 65;
        }
        if !scenario.supply.iter().any(|s| scenario.countries.get(label) == Some(&s.country)) {
            println!("INPUT ERROR: --capital: country label {label} has no supply setup in the scenario");
            return 65;
        }
        if !graph.contains(ProvinceId(*p)) {
            println!("INPUT ERROR: --capital: Province#{p} is not a province of the graph");
            return 65;
        }
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
    if !scenario.supply.is_empty() {
        println!(
            "daily supply (M6-d; computed once at the start of each day, before movement, battle and occupation): {}",
            scenario
                .supply
                .iter()
                .map(|s| format!("country {} capital {} stockpile {}", labels.get(&s.country).map(|l| l.to_string()).unwrap_or_else(|| "?".into()), s.capital.0, s.stockpile))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    let mut military = scenario.into_military();
    println!("\n--- day 0 (initial placement) ---");
    print_state(0, &military, &labels, a.combat);
    if a.state_control {
        print_state_control(0, &world.model, &territory.owners().collect(), &military, &labels);
    }
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
        for (_, label, p) in a.capitals.iter().filter(|(d, _, _)| *d == day) {
            if let Some(c) = scenario_countries.get(label) {
                military.set_capital(*c, ProvinceId(*p));
                println!("capital change: country {label} -> {}", ProvinceId(*p));
            }
        }
        for (_, state, from, to, battles) in a.transfers.iter().filter(|t| t.0 == day) {
            let req = TransferRequest::new(MapStateId(*state), scenario_countries[from], scenario_countries[to]).with_cancel_battles(battles.iter().map(|b| MapBattleId(*b)));
            let ctx = TransferContext { model: &world.model, graph: &graph };
            println!(
                "transfer request (between days; before day {day} is processed): {} owner country {from} -> country {to}; cancel battles [{}]",
                MapStateId(*state),
                battles.iter().map(|b| format!("MapBattle#{b}")).collect::<Vec<_>>().join(", ")
            );
            match plan_transfer(ctx, &territory, &military, &req) {
                Err(e) => println!("transfer REJECTED at planning: {e} (nothing changed){}", rejection_hint(&e)),
                Ok(plan) => match apply_transfer(ctx, &mut territory, &mut military, &plan) {
                    Err(e) => println!("transfer REJECTED at apply: {e} (nothing changed)"),
                    Ok(o) => {
                        print_transfer_outcome(&o, &labels);
                        match territory.check_consistency(&military) {
                            Ok(()) => println!("consistency: supply capitals equal the territory capitals; no supply setup remains for an eliminated country"),
                            Err(e) => println!("consistency: BROKEN: {e}"),
                        }
                        println!("--- state right after the transfer (before day {day} is processed) ---");
                        print_state(day, &military, &labels, a.combat);
                        if a.state_control {
                            print_state_control(day, &world.model, &territory.owners().collect(), &military, &labels);
                        }
                    }
                },
            }
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
        let day_events = military.advance_day(&graph);
        for e in day_events.iter().cloned() {
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
                    "event: control claim: {province} by country {} (controller at the time {}; reason {}; divisions {})",
                    labels.get(&claimant).map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
                    previous_controller.map(|c| labels.get(&c).map(|l| l.to_string()).unwrap_or_else(|| "?".into())).unwrap_or_else(|| "none".into()),
                    match reason {
                        ClaimReason::Undefended => "undefended".to_string(),
                        ClaimReason::AttackerWonBattle(b) => format!("attacker won {b}"),
                    },
                    divisions.iter().map(|d| d.0.to_string()).collect::<Vec<_>>().join(",")
                ),
                MoveEvent::ControlChanged { province, previous, new, reason, divisions } => println!(
                    "event: CONTROL CHANGED {province}: country {} -> country {} ({}; divisions {}). Only this province changed; the state's legal owner is untouched",
                    previous.map(|c| labels.get(&c).map(|l| l.to_string()).unwrap_or_else(|| "?".into())).unwrap_or_else(|| "none".into()),
                    labels.get(&new).map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
                    match reason {
                        ClaimReason::Undefended => "undefended".to_string(),
                        ClaimReason::AttackerWonBattle(b) => format!("attacker won {b}"),
                    },
                    divisions.iter().map(|d| d.0.to_string()).collect::<Vec<_>>().join(",")
                ),
                MoveEvent::DailySupply(r) => println!(
                    "supply: country {}: capital {}; connected {} province(s), unknown {} (not counted as connected); reach {}; demand {} (eligible {}) consumed {} units; stockpile {} units{}",
                    labels.get(&r.country).map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
                    if r.capital_controlled { "controlled" } else { "LOST (reach is empty)" },
                    r.connected_provinces,
                    r.unknown_provinces,
                    if !r.capital_controlled { "not computed (the capital is not controlled: empty)" } else if r.reach_recomputed { "recomputed" } else { "reused from cache" },
                    r.total_demand_units,
                    r.eligible_demand_units,
                    r.consumed_units,
                    r.stockpile_units,
                    if r.frontline_divisions.is_empty() { String::new() } else { format!("; frontline supply exception for divisions {}", r.frontline_divisions.iter().map(|d| d.0.to_string()).collect::<Vec<_>>().join(",")) }
                ),
                MoveEvent::SupplyFailed { country, reason } => println!(
                    "supply: country {}: planning/applying FAILED ({reason:?}); stockpile and fulfillment left unchanged",
                    labels.get(&country).map(|l| l.to_string()).unwrap_or_else(|| "?".into())
                ),
                MoveEvent::ClaimNotApplied { province, claimant, reason, rejection } => println!(
                    "event: claim NOT applied at {province} by country {} ({}): {} (control unchanged; not retried automatically)",
                    labels.get(&claimant).map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
                    match reason {
                        ClaimReason::Undefended => "undefended".to_string(),
                        ClaimReason::AttackerWonBattle(b) => format!("attacker won {b}"),
                    },
                    match rejection {
                        ClaimRejection::AlreadyApplied => "already applied".to_string(),
                        ClaimRejection::Stale { .. } => "stale: the controller changed after the request".to_string(),
                        ClaimRejection::NotAtWar { .. } => "the current controller is not at war with the claimant".to_string(),
                        ClaimRejection::NoValidAttacker => "no valid attacking division is left at the province".to_string(),
                        ClaimRejection::EnemyDivisionsRemain { divisions } => format!(
                            "deferred: divisions of a country at war with the claimant remain there ({})",
                            divisions.iter().map(|d| d.0.to_string()).collect::<Vec<_>>().join(",")
                        ),
                    }
                ),
            }
        }
        let invalidated = countries_to_invalidate(&day_events);
        if !invalidated.is_empty() {
            println!(
                "reach invalidation targets for M6-d (previous and new controllers; no duplicates): countries {}",
                invalidated.iter().map(|c| labels.get(c).map(|l| l.to_string()).unwrap_or_else(|| "?".into())).collect::<Vec<_>>().join(", ")
            );
        }
        print_state(day, &military, &labels, a.combat);
        if a.state_control {
            print_state_control(day, &world.model, &territory.owners().collect(), &military, &labels);
        }
    }
    let unused: Vec<u32> = a.orders.iter().filter(|(d, _, _)| *d > a.days).map(|(d, _, _)| *d).collect();
    if !unused.is_empty() {
        println!("\nnote: {} order(s) are after day {} and were not issued", unused.len(), a.days);
    }
    println!("\norders rejected: {rejected} (a rejection is a normal result)");
    0
}
