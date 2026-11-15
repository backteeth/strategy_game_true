//! P32-037 (M6-d): 新経路の師団への日次補給(`world::supply`・`world::legacy_supply`、旧の共通の核 `military::daily_supply::plan_supply_core`)の検証。
//!
//! 旧の日次補給との比較は、旧の公開API(`plan_country_daily_supply_with_frontline`・`run_daily_military_supply_consumption_with_battles`・`connected_states_from_capital`・
//! `is_frontline_supply_eligible`)を**旧型の fixture で実際に実行**して行う(新旧IDの対応表: 旧 `StateId i` ↔ 新 `ProvinceId 10+i`(鎖)または `100+10i`(星形)、旧 `DivisionId n` ↔ 新 `MapDivisionId n`)。
//! 時間は合否条件にしない(測定は `world_supply_bench`)。保存・アセットは書かない。

use std::collections::HashSet;

use serde_json::json;
use strategy_game::common::{CountryId, DivisionDefinitionId, DivisionId, StateId};
use strategy_game::country::{CountryData, CountryRegistry};
use strategy_game::economy::resources::{ResourceType, military_supplies_value_to_units};
use strategy_game::hexmap::data::{HexMap, load_from_path, parse_hex_map};
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::capital_connectivity::connected_states_from_capital;
use strategy_game::military::daily_supply::{is_frontline_supply_eligible, plan_country_daily_supply_with_frontline, run_daily_military_supply_consumption_with_battles};
use strategy_game::military::data::{Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry};
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::world::division_scenario::{CombatEntry, CountryEntry, DivisionEntry, StateOwnerEntry, SupplyCountryEntry, SupplySetup, SupplyUsageEntry, WarEntry};
use strategy_game::world::fingerprint::FINGERPRINT_ALGORITHM;
use strategy_game::world::legacy_combat;
use strategy_game::world::*;

const C1: CountryId = CountryId(1);
const C2: CountryId = CountryId(2);
const C3: CountryId = CountryId(3);
const SUPPLY: &str = "assets/data/world/preview/division_supply_scenario.ron";

/// 標準の師団(人員 10,000・補給消費量 1.0)の日次需要(旧の固定小数点の単位)。
const STD: u128 = 1_000_000;

// ---------------------------------------------------------------- 部品

fn map_from(rows: &[&str]) -> HexMap {
    let (h, w) = (rows.len(), rows.iter().map(|r| r.len()).max().unwrap());
    let mut cells = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (q, ch) in row.chars().enumerate() {
            let (terrain, climate, veg) = match ch {
                '.' => ("sea", "unclassified", "unclassified"),
                _ => ("plains", "C", "grassland"),
            };
            cells.push(json!({ "q": q, "r": r, "terrain": terrain, "climate": climate, "vegetation": veg }));
        }
    }
    let v = json!({
        "schemaVersion": 9,
        "kind": "hex-atlas-game-map",
        "grid": {
            "orientation": "pointy", "size": 2.5, "origin": { "x": 2.1650635094610964, "y": 2.5 },
            "imageWidth": 100, "imageHeight": 60,
            "extent": { "minQ": 0, "maxQ": w - 1, "minR": 0, "maxR": h - 1 }
        },
        "cells": cells,
        "rivers": [],
    });
    parse_hex_map(v.to_string().as_bytes()).expect("synthetic map must parse")
}

fn graph_with(map: &HexMap, provinces: &[(u32, i32, i32)], coverage: Coverage) -> ProvinceGraph {
    let mut geography = compute_fingerprint(map);
    geography.algorithm = FINGERPRINT_ALGORITHM;
    let t = ProvinceTableFile {
        format_version: WORLD_FORMAT_VERSION,
        coverage,
        geography,
        next_province_id: provinces.iter().map(|p| p.0).max().unwrap_or(0) + 1,
        retired: Vec::new(),
        provinces: provinces.iter().map(|&(id, q, r)| ProvinceEntry { id, q, r }).collect(),
    };
    let w = WorldModel::build(t, StateLayoutFile { format_version: WORLD_FORMAT_VERSION, states: Vec::new() }).expect("test world must build");
    ProvinceGraph::build(&w, map).expect("graph must build")
}

fn p(id: u32) -> ProvinceId {
    ProvinceId(id)
}

fn md(id: u32) -> MapDivisionId {
    MapDivisionId(id)
}

fn control_of(entries: &[(u32, CountryId)]) -> ProvinceControl {
    ProvinceControl::new(entries.iter().map(|(id, c)| (p(*id), *c)).collect())
}

fn cfg(country: CountryId, capital: u32, stockpile: f64) -> SupplyCountryConfig {
    SupplyCountryConfig { country, capital: p(capital), stockpile }
}

fn military(divs: Vec<MapDivision>, control: ProvinceControl, wars: Vec<(CountryId, CountryId)>, supply: Vec<SupplyCountryConfig>) -> MapMilitary {
    MapMilitary::new(divs, control, MovementParams { base_days: 5.0 }).unwrap().with_wars(wars).unwrap().with_supply(supply).unwrap()
}

fn weak() -> CombatStats {
    CombatStats { manpower: 2500, organization: 20.0, ..CombatStats::standard() }
}

/// 1行 n 陸(ID = 10 + q)。
fn line_graph(n: usize) -> ProvinceGraph {
    let map = map_from(&[&"#".repeat(n)]);
    let provs: Vec<(u32, i32, i32)> = (0..n).map(|q| (10 + q as u32, q as i32, 0)).collect();
    graph_with(&map, &provs, Coverage::World)
}

const STAR: [(i32, i32); 7] = [(1, 1), (2, 1), (2, 0), (1, 0), (0, 1), (0, 2), (1, 2)];

fn pid(idx: usize) -> u32 {
    100 + 10 * idx as u32
}

fn star_graph() -> ProvinceGraph {
    let map = map_from(&[".##", "###", "##."]);
    let provs: Vec<(u32, i32, i32)> = STAR.iter().enumerate().map(|(i, (q, r))| (pid(i), *q, *r)).collect();
    graph_with(&map, &provs, Coverage::World)
}

fn star_control(c: &[CountryId; 7]) -> ProvinceControl {
    control_of(&c.iter().enumerate().map(|(i, c)| (pid(i), *c)).collect::<Vec<_>>())
}

fn report(ev: &[MoveEvent], country: CountryId) -> SupplyReport {
    ev.iter()
        .find_map(|e| if let MoveEvent::DailySupply(r) = e { (r.country == country).then(|| r.clone()) } else { None })
        .unwrap_or_else(|| panic!("no daily supply report for country {}", country.0))
}

fn f(m: &MapMilitary, id: u32) -> f64 {
    m.division(md(id)).unwrap().fulfillment
}

fn real() -> (WorldPreviewData, ProvinceGraph) {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let w = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).unwrap();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    (w, g)
}

// ---------------------------------------------------------------- 十分な備蓄 / 備蓄不足

#[test]
fn a_sufficient_stockpile_supplies_every_connected_division_and_is_consumed_exactly_once_per_day() {
    let g = line_graph(4);
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(11), 1.0), MapDivision::new(md(2), C1, p(13), 1.0)],
        control_of(&[(10, C1), (11, C1), (12, C1), (13, C1)]),
        vec![],
        vec![cfg(C1, 10, 10.0)],
    );
    assert_eq!(m.supply_stockpile_units(C1), Some(10_000_000));
    let ev = m.advance_day(&g);
    let r = report(&ev, C1);
    assert_eq!((r.capital_controlled, r.connected_provinces, r.unknown_provinces, r.reach_recomputed), (true, 4, 0, true));
    assert_eq!((r.total_demand_units, r.eligible_demand_units, r.consumed_units, r.stockpile_units), (2 * STD, 2 * STD, 2 * STD, 8_000_000));
    assert_eq!((f(&m, 1), f(&m, 2)), (1.0, 1.0));
    // 毎日1回だけ消費する(同じ日に二重に消費しない)。再利用した到達結果でも、需要と消費はその日の師団で計算し直す。
    let ev2 = m.advance_day(&g);
    let r2 = report(&ev2, C1);
    assert_eq!((r2.reach_recomputed, r2.consumed_units, r2.stockpile_units), (false, 2 * STD, 6_000_000));
    m.advance_day(&g);
    assert_eq!(m.supply_stockpile_units(C1), Some(4_000_000));
    assert_eq!(m.supply_stats(), Some(SupplyStats { reach_computations: 1, reach_reuses: 2 }));
}

#[test]
fn a_shortage_is_allocated_by_largest_remainder_with_the_division_id_tie_break_and_never_exceeds_the_stock() {
    let g = line_graph(4);
    let ctl = control_of(&[(10, C1), (11, C1), (12, C1), (13, C1)]);
    // 2 単位の備蓄を、需要が等しい3師団へ: 基本配分は全員 0、余り2単位は剰余が同じなので師団IDの小さい順(1, 2)へ
    let mut m = military(
        vec![MapDivision::new(md(3), C1, p(13), 1.0), MapDivision::new(md(1), C1, p(11), 1.0), MapDivision::new(md(2), C1, p(12), 1.0)],
        ctl.clone(),
        vec![],
        vec![cfg(C1, 10, 0.000002)],
    );
    let ev = m.advance_day(&g);
    let units: Vec<u64> = [1, 2, 3].iter().map(|i| (f(&m, *i) * 1e6).round() as u64).collect();
    assert_eq!(units, vec![1, 1, 0], "units go to the lower division ids first");
    let r = report(&ev, C1);
    assert_eq!((r.consumed_units, r.stockpile_units), (2, 0));
    // 備蓄が尽きた翌日は、配分 0(需要が正なので充足率 0.0)で、備蓄は動かない
    let r2 = report(&m.advance_day(&g), C1);
    assert_eq!((r2.consumed_units, r2.stockpile_units, f(&m, 1), f(&m, 3)), (0, 0, 0.0, 0.0));
    // 比例配分: 備蓄 1.5 を需要 1.0 の2師団へ → 各 0.75
    let mut m = military(vec![MapDivision::new(md(1), C1, p(11), 1.0), MapDivision::new(md(2), C1, p(12), 1.0)], ctl, vec![], vec![cfg(C1, 10, 1.5)]);
    m.advance_day(&g);
    assert_eq!((f(&m, 1), f(&m, 2)), (0.75, 0.75));
    // 需要は人員比に比例する: 人員 2,500 の弱い師団の需要は 0.25
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(11), 1.0).with_combat(weak()), MapDivision::new(md(2), C1, p(12), 1.0)],
        control_of(&[(10, C1), (11, C1), (12, C1)]),
        vec![],
        vec![cfg(C1, 10, 10.0)],
    );
    let r = report(&m.advance_day(&g), C1);
    assert_eq!((r.total_demand_units, r.consumed_units), (STD + STD / 4, STD + STD / 4));
}

// ---------------------------------------------------------------- 分断・首都喪失・「不明」

#[test]
fn a_disconnected_division_gets_zero_and_does_not_consume_and_a_lost_capital_empties_the_reach() {
    let g = line_graph(5);
    // 10(首都) 11(国家2の支配: 分断) 12 13。国家1の師団1 は 12(切断)、師団2 は 10(連結)。
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(12), 1.0), MapDivision::new(md(2), C1, p(10), 1.0)],
        control_of(&[(10, C1), (11, C2), (12, C1), (13, C1)]),
        vec![],
        vec![cfg(C1, 10, 10.0)],
    );
    let r = report(&m.advance_day(&g), C1);
    assert_eq!((r.connected_provinces, r.total_demand_units, r.eligible_demand_units, r.consumed_units, r.stockpile_units), (1, 2 * STD, STD, STD, 9_000_000));
    assert_eq!((f(&m, 1), f(&m, 2)), (0.0, 1.0), "the cut division is 0.0 and does not consume the stockpile");
    // 首都が自国の支配を失う(検証用の支配の変更): 到達集合は空、全師団が切断、備蓄は動かない
    m.set_controller(p(10), Some(C2));
    let r = report(&m.advance_day(&g), C1);
    assert_eq!((r.capital_controlled, r.connected_provinces, r.eligible_demand_units, r.consumed_units, r.stockpile_units), (false, 0, 0, 0, 9_000_000));
    assert_eq!((f(&m, 1), f(&m, 2)), (0.0, 0.0));
    // 取り戻すと、翌日の補給で再接続される(支配の変更で到達結果が無効化され、次の補給で計算し直す)
    m.set_controller(p(10), Some(C1));
    let r = report(&m.advance_day(&g), C1);
    assert!(r.capital_controlled && r.reach_recomputed);
    assert_eq!((f(&m, 1), f(&m, 2)), (0.0, 1.0));
}

#[test]
fn unknown_reach_is_reported_separately_and_never_counted_as_confirmed_supply() {
    // q0 q1 [q2 = 区画表に無い陸(範囲外)] q3 q4: 国家1がすべて支配。首都 q0。q3・q4 は範囲外を通ればつながり得る(不明)。
    let map = map_from(&["#####"]);
    let g = graph_with(&map, &[(10, 0, 0), (11, 1, 0), (13, 3, 0), (14, 4, 0)], Coverage::Partial);
    assert!(!g.scope().is_complete());
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(11), 1.0), MapDivision::new(md(2), C1, p(13), 1.0)],
        control_of(&[(10, C1), (11, C1), (13, C1), (14, C1)]),
        vec![],
        vec![cfg(C1, 10, 10.0)],
    );
    let r = report(&m.advance_day(&g), C1);
    assert_eq!((r.connected_provinces, r.unknown_provinces), (2, 2), "confirmed 10 and 11; 13 and 14 are unknown");
    assert_eq!((f(&m, 1), f(&m, 2)), (1.0, 0.0), "unknown is not confirmed supply");
    assert_eq!((r.eligible_demand_units, r.consumed_units), (STD, STD));
}

// ---------------------------------------------------------------- 占領との前後関係: 当日の配分は変わらず、翌日に反映される

#[test]
fn an_occupation_does_not_change_todays_allocation_and_the_next_day_reflects_reconnection_or_a_lost_capital() {
    // 再接続: 10(首都,国家1) 11(国家2、守備隊なし) 12(国家1)。師団A(10)が 11 を5日目に占領する。師団D(12)は、占領の前は切断されている。
    let g = line_graph(3);
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(10), 1.0), MapDivision::new(md(2), C1, p(12), 1.0)],
        control_of(&[(10, C1), (11, C2), (12, C1)]),
        vec![(C1, C2)],
        vec![cfg(C1, 10, 100.0)],
    );
    m.issue_move_order(&g, md(1), p(11)).unwrap();
    let mut day5 = Vec::new();
    for day in 1..=5 {
        let ev = m.advance_day(&g);
        assert_eq!(f(&m, 2), 0.0, "day {day}: division 2 is cut off until the occupation is reflected");
        if day == 5 {
            day5 = ev;
        }
    }
    // 5日目の占領で支配が変わったが、その日の補給は計画の後なので配分は変わらない(充足率 0.0 のまま)
    assert!(day5.iter().any(|e| matches!(e, MoveEvent::ControlChanged { province, .. } if *province == p(11))));
    assert_eq!(report(&day5, C1).connected_provinces, 1);
    assert_eq!(f(&m, 2), 0.0, "today's allocation is unchanged by today's occupation");
    assert!(!m.reach_is_cached(C1), "the occupation invalidated the previous and new controllers' reach results");
    // 翌日: 到達結果を計算し直し、再接続が反映される
    let r6 = report(&m.advance_day(&g), C1);
    assert!(r6.reach_recomputed);
    assert_eq!(r6.connected_provinces, 3);
    assert_eq!(f(&m, 2), 1.0);

    // 首都喪失: 星形。首都 idx1(国家1)。国家2の師団Y(idx0)が、守備隊のいない首都 idx1 へ入って占領する。国家1の師団A は idx2 にいて、占領までは連結。
    let g = star_graph();
    let ctl = star_control(&[C2, C1, C1, C1, C1, C1, C1]);
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(pid(2)), 1.0), MapDivision::new(md(9), C2, p(pid(0)), 1.0)],
        ctl,
        vec![(C1, C2)],
        vec![cfg(C1, pid(1), 100.0), cfg(C2, pid(0), 100.0)],
    );
    m.issue_move_order(&g, md(9), p(pid(1))).unwrap();
    let mut last = Vec::new();
    for _ in 1..=5 {
        last = m.advance_day(&g);
    }
    assert!(last.iter().any(|e| matches!(e, MoveEvent::ControlChanged { province, new, .. } if *province == p(pid(1)) && *new == C2)));
    assert_eq!(f(&m, 1), 1.0, "day 5: the allocation was decided before the capital fell");
    let r6 = report(&m.advance_day(&g), C1);
    assert!(!r6.capital_controlled);
    assert_eq!((r6.connected_provinces, f(&m, 1)), (0, 0.0), "day 6: the lost capital cuts every division of country 1");
}

// ---------------------------------------------------------------- 前線補給の例外

#[test]
fn the_frontline_exception_applies_to_an_attacker_in_battle_next_to_connected_own_territory_and_not_otherwise() {
    // 星形。国家1の攻撃側A(idx1)が 中心 idx0(国家2、標準の守備 D)を攻める(攻撃力 12 で 5日目から数日続く)。首都は国家1が idx2、国家2が idx3。
    let build = || {
        let g = star_graph();
        let ctl = star_control(&[C2, C1, C1, C2, C2, C2, C2]);
        let a = MapDivision::new(md(1), C1, p(pid(1)), 1.0).with_combat(CombatStats { attack_power: 12, ..CombatStats::standard() });
        let d = MapDivision::new(md(2), C2, p(pid(0)), 1.0);
        let m = military(vec![a, d], ctl, vec![(C1, C2)], vec![cfg(C1, pid(2), 100.0), cfg(C2, pid(3), 100.0)]);
        (g, m)
    };
    let (g, mut m) = build();
    m.issue_move_order(&g, md(1), p(pid(0))).unwrap();
    for _ in 1..=5 {
        m.advance_day(&g);
    }
    assert_eq!(m.division(md(1)).unwrap().status, MapDivisionStatus::Fighting);
    // 6日目の補給: 攻撃側A は中心(国家2の支配)にいて連結集合の外だが、進行中の戦闘の攻撃側で、隣接する自国支配の idx1・idx2 が首都と連結している → 例外の対象。需要は一度だけ計上される
    // 戦闘で人員が減っているので、需要は人員比に比例する値(旧と同じ式)。6日目の補給の直前(5日目の終わり)の人員から求める。需要は一度だけ計上され、連結扱いの需要と一致する
    let expected_demand = m.division(md(1)).unwrap().combat.manpower as u128 * STD / 10_000;
    let r = report(&m.advance_day(&g), C1);
    assert_eq!(r.frontline_divisions, vec![md(1)]);
    assert_eq!((r.total_demand_units, r.eligible_demand_units), (expected_demand, expected_demand), "counted once");
    // 防御側D は(国家2の支配地にいて)連結しており、前線補給の例外ではない(攻撃側だけが対象)
    assert!(report(&m.advance_day(&g), C2).frontline_divisions.is_empty());
    // 対象外: 中心に隣接する自国支配の連結した地点がない(首都を含む idx1・idx2 が国家1の支配でなくなった)
    let (g, mut m2) = build();
    m2.issue_move_order(&g, md(1), p(pid(0))).unwrap();
    for _ in 1..=5 {
        m2.advance_day(&g);
    }
    m2.set_controller(p(pid(1)), Some(C3));
    m2.set_controller(p(pid(2)), Some(C3));
    let r = report(&m2.advance_day(&g), C1);
    assert!(r.frontline_divisions.is_empty() && !r.capital_controlled);
    assert_eq!((r.eligible_demand_units, f(&m2, 1)), (0, 0.0));
    // 対象外: 戦闘に参加していない(守備側ではなく、移動中でもない)師団が敵支配地にいる場合は例外にならない
    let g = star_graph();
    let ctl = star_control(&[C2, C1, C1, C2, C2, C2, C2]);
    let mut m3 = military(vec![MapDivision::new(md(1), C1, p(pid(0)), 1.0)], ctl, vec![(C1, C2)], vec![cfg(C1, pid(2), 100.0)]);
    let r = report(&m3.advance_day(&g), C1);
    assert!(r.frontline_divisions.is_empty() && f(&m3, 1) == 0.0, "an idle division on enemy ground without a battle gets no frontline supply");
}

// ---------------------------------------------------------------- 補給率の移動・戦闘への反映

#[test]
fn the_fulfillment_changes_the_movement_days_with_the_old_floor_of_one_half_and_supply_off_keeps_the_old_speed() {
    let g = line_graph(2);
    let ctl = control_of(&[(10, C1), (11, C1)]);
    let arrival = |fulfillment: f64| -> usize {
        let mut d = MapDivision::new(md(1), C1, p(10), 1.0);
        d.fulfillment = fulfillment;
        let mut m = military(vec![d], ctl.clone(), vec![], vec![]);
        m.issue_move_order(&g, md(1), p(11)).unwrap();
        for day in 1..=40 {
            m.advance_day(&g);
            if m.division(md(1)).unwrap().province == p(11) {
                return day;
            }
        }
        panic!("never arrived");
    };
    // 旧の式 max(1, 5 / (速度 × max(充足率, 0.5)))
    assert_eq!(arrival(1.0), 5);
    assert_eq!(arrival(0.8), 7, "5/0.8 = 6.25 days per hop: progress 0.16 per day reaches 1.0 on day 7");
    assert_eq!(arrival(0.5), 10);
    assert_eq!(arrival(0.3), 10, "the lower bound of the modifier is 0.5");
    assert_eq!(arrival(0.0), 10);
    // 日次補給を通した値でも同じ: 備蓄 1.0 を2師団で分けると各 0.5 → 10日(補給なしの設定なら 5日のまま)
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(10), 1.0), MapDivision::new(md(2), C1, p(10), 1.0)],
        ctl.clone(),
        vec![],
        vec![cfg(C1, 10, 1.0)],
    );
    m.issue_move_order(&g, md(1), p(11)).unwrap();
    let mut arrived = None;
    for day in 1..=20 {
        m.advance_day(&g);
        if arrived.is_none() && m.division(md(1)).unwrap().province == p(11) {
            arrived = Some(day);
        }
    }
    assert_eq!(arrived, Some(10), "a half-supplied division moves at half speed");
    let mut free = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], ctl, vec![], vec![]);
    free.issue_move_order(&g, md(1), p(11)).unwrap();
    for _ in 1..=5 {
        free.advance_day(&g);
    }
    assert_eq!(free.division(md(1)).unwrap().province, p(11), "no supply setup: the M6-a speed is unchanged");
}

#[test]
fn the_days_allocation_is_used_by_the_days_battle_calculation() {
    // 星形。攻撃側(国家1、補給 0.5)が、1日目に守備(国家2、十分な補給)と接触して戦闘する(補給 0.5 では移動係数が 0.5 になるので、速度 10.0 で 1ホップ 1日)。
    // その日の充足率が、その日の戦闘計算に使われる。
    let g = star_graph();
    let ctl = star_control(&[C2, C1, C1, C2, C2, C2, C2]);
    let a = MapDivision::new(md(1), C1, p(pid(1)), 10.0);
    let d = MapDivision::new(md(2), C2, p(pid(0)), 1.0);
    let mut m = military(vec![a, d], ctl, vec![(C1, C2)], vec![cfg(C1, pid(2), 0.5), cfg(C2, pid(0), 100.0)]);
    m.issue_move_order(&g, md(1), p(pid(0))).unwrap();
    let ev = m.advance_day(&g);
    assert!(ev.iter().any(|e| matches!(e, MoveEvent::BattleStarted { .. })), "the battle takes place on day 1");
    let (fa, fd) = (f(&m, 1), f(&m, 2));
    assert_eq!((fa, fd), (0.5, 1.0));
    let std = CombatStats::standard();
    let expected = legacy_combat::resolve_day_with_supply(&[(md(1), C1, &std, fa)], &[(md(2), C2, &std, fd)]).unwrap();
    let full = legacy_combat::resolve_day_with_supply(&[(md(1), C1, &std, 1.0)], &[(md(2), C2, &std, 1.0)]).unwrap();
    assert_ne!(expected.defenders, full.defenders, "the half-supplied attacker deals less damage than a fully supplied one");
    assert_eq!(m.division(md(1)).unwrap().combat.manpower, 10_000 - expected.attackers[&md(1)].0);
    assert_eq!(m.division(md(2)).unwrap().combat.manpower, 10_000 - expected.defenders[&md(2)].0);
}

// ---------------------------------------------------------------- 再計算(国家ごとの Reach のキャッシュ)

#[test]
fn only_the_previous_and_new_controllers_are_recomputed_and_unrelated_countries_reuse_their_reach() {
    // 10(国家1 首都) 11(国家1) 12(国家2) 13(国家2 首都) 14(国家3 首都) 15(国家3)。国家1の師団X(11)が 12 を占領する(5日目)。
    let g = line_graph(6);
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(11), 1.0), MapDivision::new(md(2), C2, p(13), 1.0), MapDivision::new(md(3), C3, p(15), 1.0)],
        control_of(&[(10, C1), (11, C1), (12, C2), (13, C2), (14, C3), (15, C3)]),
        vec![(C1, C2)],
        vec![cfg(C1, 10, 100.0), cfg(C2, 13, 100.0), cfg(C3, 14, 100.0)],
    );
    m.issue_move_order(&g, md(1), p(12)).unwrap();
    let mut day5 = Vec::new();
    for day in 1..=5 {
        let ev = m.advance_day(&g);
        if day == 1 {
            assert!([C1, C2, C3].iter().all(|c| report(&ev, *c).reach_recomputed), "day 1: every country computes once");
        } else {
            assert!([C1, C2, C3].iter().all(|c| !report(&ev, *c).reach_recomputed), "day {day}: reach results are reused, but demand and allocation are recomputed from the day's divisions");
        }
        if day == 5 {
            day5 = ev;
        }
    }
    assert_eq!(countries_to_invalidate(&day5), vec![C1, C2]);
    assert!(!m.reach_is_cached(C1) && !m.reach_is_cached(C2) && m.reach_is_cached(C3), "only the previous and new controllers are invalidated");
    let ev6 = m.advance_day(&g);
    assert_eq!((report(&ev6, C1).reach_recomputed, report(&ev6, C2).reach_recomputed, report(&ev6, C3).reach_recomputed), (true, true, false));
    assert_eq!(report(&ev6, C1).connected_provinces, 3, "10, 11 and the newly occupied 12");
    assert_eq!(report(&ev6, C2).connected_provinces, 1);
    // 検証用の支配の変更: 変更前・変更後の支配国だけ無効化
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(11), 1.0)],
        control_of(&[(10, C1), (11, C1), (12, C2), (13, C2), (14, C3), (15, C3)]),
        vec![],
        vec![cfg(C1, 10, 100.0), cfg(C2, 13, 100.0), cfg(C3, 14, 100.0)],
    );
    m.advance_day(&g);
    assert!([C1, C2, C3].iter().all(|c| m.reach_is_cached(*c)));
    m.set_controller(p(12), Some(C3));
    assert!(m.reach_is_cached(C1) && !m.reach_is_cached(C2) && !m.reach_is_cached(C3));
    // 首都の変更は、その国家だけを無効化する
    m.advance_day(&g);
    assert!(m.set_capital(C1, p(11)));
    assert!(!m.reach_is_cached(C1) && m.reach_is_cached(C2));
    assert!(!m.set_capital(CountryId(9), p(11)), "a country without a supply setup has no capital to change");
    let ev = m.advance_day(&g);
    assert!(report(&ev, C1).reach_recomputed);
    assert_eq!(report(&ev, C1).connected_provinces, 2, "capital 11 reaches 10 and 11");
}

#[test]
fn replacing_the_graph_discards_every_cached_reach() {
    let g1 = line_graph(4);
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(11), 1.0)],
        control_of(&[(10, C1), (11, C1), (12, C1), (13, C1)]),
        vec![],
        vec![cfg(C1, 10, 100.0)],
    );
    assert!(report(&m.advance_day(&g1), C1).reach_recomputed);
    assert!(!report(&m.advance_day(&g1), C1).reach_recomputed);
    // 同じ ID の別のグラフ(陸が1つ多い)へ差し替える: 署名が変わるので古い結果は残らない
    let g2 = line_graph(5);
    assert_ne!(g1.signature(), g2.signature());
    let r = report(&m.advance_day(&g2), C1);
    assert!(r.reach_recomputed, "the old result belongs to the old graph");
    assert_eq!(r.connected_provinces, 4, "the new graph has the same controlled provinces");
}

// ---------------------------------------------------------------- 旧経路との比較

fn old_division(id: u32, owner: usize, idx: usize, c: &CombatStats) -> Division {
    Division {
        id: DivisionId(id as usize),
        owner: CountryId(owner),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(idx),
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower: c.manpower,
        max_manpower: c.max_manpower,
        equipment: c.equipment,
        max_equipment: c.max_equipment,
        organization: c.organization,
        max_organization: c.max_organization,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: DivisionDefinitionId(0),
        attack_power: c.attack_power,
        defense_power: c.defense_power,
        combat_id: None,
    }
}

const OFFSETS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];

fn adjacency(cells: &[(i32, i32)]) -> Vec<Vec<usize>> {
    cells
        .iter()
        .map(|(q, r)| {
            let mut v: Vec<usize> = OFFSETS.iter().filter_map(|(dq, dr)| cells.iter().position(|c| *c == (q + dq, r + dr))).collect();
            v.sort_unstable();
            v
        })
        .collect()
}

struct Old {
    states: StateRegistry,
    mil: MilitaryRegistry,
    countries: CountryRegistry,
    battles: BattleRegistry,
}

/// 旧の fixture。`cells[i]` の州 `StateId(i)` ↔ 新の `ProvinceId(base + 10 i)`。
fn old_world(cells: &[(i32, i32)], controllers: &[usize], divisions: &[(u32, usize, usize, CombatStats)], supplies: &[(usize, usize, f64)]) -> Old {
    let adj = adjacency(cells);
    let states: Vec<StateData> =
        (0..cells.len()).map(|i| StateData { id: StateId(i), owner_country_id: CountryId(controllers[i]), neighbors: adj[i].iter().map(|j| StateId(*j)).collect(), ..Default::default() }).collect();
    let mut mil = MilitaryRegistry::default();
    mil.definitions.insert(
        DivisionDefinitionId(0),
        DivisionDefinition {
            id: DivisionDefinitionId(0),
            name: "t".into(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 1,
            required_equipment: 1.0,
            recruitment_days: 1,
            movement_speed: 1.0,
            attack: 1.0,
            defense: 1.0,
            breakthrough: 1.0,
            organization: 1.0,
            morale: 1.0,
            supply_usage: 1.0,
            maintenance_cost: 0.0,
        },
    );
    for (id, owner, idx, c) in divisions {
        mil.divisions.insert(DivisionId(*id as usize), old_division(*id, *owner, *idx, c));
    }
    let mut countries = CountryRegistry::default();
    for (country, capital, stock) in supplies {
        let mut c = CountryData { id: CountryId(*country), capital_state_id: StateId(*capital), ..CountryData::default() };
        c.stockpile.set(ResourceType::MilitarySupplies, *stock);
        countries.countries.push(c);
    }
    Old { states: StateRegistry::build(states), mil, countries, battles: BattleRegistry::default() }
}

fn old_stock_units(o: &Old, country: usize) -> u64 {
    military_supplies_value_to_units(o.countries.get(CountryId(country)).unwrap().stockpile.get(ResourceType::MilitarySupplies)).unwrap()
}

/// 旧の公開APIで、国家の1日分の計画(需要・消費・配分)を求める。
fn old_plan(o: &Old, country: usize) -> strategy_game::military::daily_supply::CountrySupplyPlan {
    let c = o.countries.get(CountryId(country)).unwrap();
    let connected: HashSet<StateId> = connected_states_from_capital(c.capital_state_id, CountryId(country), &o.states);
    let frontline: HashSet<DivisionId> = o
        .mil
        .divisions
        .values()
        .filter(|d| d.owner == CountryId(country) && d.manpower > 0)
        .filter(|d| is_frontline_supply_eligible(d, &connected, &o.states, &o.battles))
        .map(|d| d.id)
        .collect();
    plan_country_daily_supply_with_frontline(CountryId(country), c.stockpile.get(ResourceType::MilitarySupplies), &o.mil.divisions, &o.mil.definitions, &connected, &frontline).unwrap()
}

/// 同じ条件(鎖 5 州、位置は動かさない)で、旧と新の各日の需要・消費・備蓄・充足率を比べる。
#[allow(clippy::too_many_arguments)]
fn compare_days(
    controllers: &[usize],
    divisions: &[(u32, usize, usize, CombatStats)],
    supplies: &[(usize, usize, f64)],
    control_changes: &[(usize, usize, usize)],
    days: usize,
) -> Vec<String> {
    let n = controllers.len();
    let cells: Vec<(i32, i32)> = (0..n as i32).map(|i| (i, 0)).collect();
    let mut old = old_world(&cells, controllers, divisions, supplies);
    let g = line_graph(n);
    let ctl = control_of(&controllers.iter().enumerate().map(|(i, c)| (10 + i as u32, CountryId(*c))).collect::<Vec<_>>());
    let divs: Vec<MapDivision> = divisions.iter().map(|(id, o, i, c)| MapDivision::new(md(*id), CountryId(*o), p(10 + *i as u32), 1.0).with_combat(*c)).collect();
    let mut new = military(divs, ctl, vec![], supplies.iter().map(|(c, cap, s)| cfg(CountryId(*c), 10 + *cap as u32, *s)).collect());
    let mut diffs = Vec::new();
    for day in 1..=days {
        for (_, idx, c) in control_changes.iter().filter(|(d, _, _)| *d == day) {
            old.states.get_mut(StateId(*idx)).unwrap().controller_country = Some(CountryId(*c));
            new.set_controller(p(10 + *idx as u32), Some(CountryId(*c)));
        }
        let plans: Vec<(usize, strategy_game::military::daily_supply::CountrySupplyPlan)> = supplies.iter().map(|(c, _, _)| (*c, old_plan(&old, *c))).collect();
        run_daily_military_supply_consumption_with_battles(&mut old.countries, &mut old.mil, &old.states, &old.battles);
        let ev = new.advance_day(&g);
        for (c, plan) in &plans {
            let r = report(&ev, CountryId(*c));
            if (r.total_demand_units, r.eligible_demand_units, r.consumed_units) != (plan.total_demand_units, plan.eligible_demand_units, plan.consumed_units) {
                diffs.push(format!("day {day} country {c}: new demand/eligible/consumed {:?} != old {:?}", (r.total_demand_units, r.eligible_demand_units, r.consumed_units), (plan.total_demand_units, plan.eligible_demand_units, plan.consumed_units)));
            }
            if new.supply_stockpile_units(CountryId(*c)) != Some(old_stock_units(&old, *c)) {
                diffs.push(format!("day {day} country {c}: stockpile new {:?} != old {}", new.supply_stockpile_units(CountryId(*c)), old_stock_units(&old, *c)));
            }
        }
        for (id, _, _, _) in divisions {
            let (o, nw) = (old.mil.supply_fulfillment_of(DivisionId(*id as usize)), f(&new, *id));
            if o != nw {
                diffs.push(format!("day {day} division {id}: fulfillment new {nw} != old {o}"));
            }
        }
    }
    diffs
}

fn std() -> CombatStats {
    CombatStats::standard()
}

#[test]
fn old_comparison_sufficient_stock_a_cut_division_and_a_zero_manpower_division_match() {
    // 鎖 0..4。国家1の首都 idx0。idx2 は国家2の支配(切断の原因)。師団: 1(idx1 連結)、2(idx3 切断)、3(人員 0。需要 0)
    let dead = CombatStats { manpower: 0, ..std() };
    let divisions = [(1, 1, 1, std()), (2, 1, 3, std()), (3, 1, 0, dead)];
    let diffs = compare_days(&[1, 1, 2, 1, 1], &divisions, &[(1, 0, 10.0)], &[], 4);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
}

#[test]
fn old_comparison_shortage_and_the_largest_remainder_tie_break_match() {
    // 需要 1.0・0.5・0.25 の3師団に備蓄 1.5(不足)→ 最大剰余法。次に 2 単位だけの備蓄(同順位は師団ID昇順)
    let half = CombatStats { manpower: 5000, ..std() };
    let quarter = CombatStats { manpower: 2500, ..std() };
    let divisions = [(3, 1, 3, quarter), (1, 1, 1, std()), (2, 1, 2, half)];
    let diffs = compare_days(&[1, 1, 1, 1], &divisions, &[(1, 0, 1.5)], &[], 3);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    let eq = [(1, 1, 1, std()), (2, 1, 2, std()), (3, 1, 3, std())];
    let diffs = compare_days(&[1, 1, 1, 1], &eq, &[(1, 0, 0.000002)], &[], 3);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
}

#[test]
fn old_comparison_capital_loss_reconnection_and_the_update_day_match() {
    // 3日目に首都 idx0 が国家2の支配になり(旧の controller_country = 国家2)、5日目に国家1へ戻る。2か国(国家2の首都は idx4)。備蓄は 3.5(数日で尽きる)。
    let divisions = [(1, 1, 1, std()), (2, 1, 3, std()), (4, 2, 4, std())];
    let diffs = compare_days(&[1, 1, 1, 1, 2], &divisions, &[(1, 0, 3.5), (2, 4, 50.0)], &[(3, 0, 2), (5, 0, 1)], 7);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
}

#[test]
fn old_comparison_the_frontline_exception_matches_the_old_eligibility() {
    // 星形(旧 StateId i ↔ 新 ProvinceId 100+10 i)。国家1の攻撃側A(idx1)が中心 idx0(国家2)にいて、進行中の戦闘の攻撃側。旧は戦闘を記録として直接入れ、新は実際の戦闘(6日目の補給)で比べる。
    let cells = STAR.to_vec();
    let controllers = [2usize, 1, 1, 2, 2, 2, 2];
    let a_stats = CombatStats { attack_power: 12, ..std() };
    // 新: 実際に 5日目に戦闘が始まる。6日目の補給の直前の師団の状態(戦闘で減った人員)を、旧の fixture にそのまま使って、旧と新の同じ入力で比べる。
    let g = star_graph();
    let ctl = star_control(&[C2, C1, C1, C2, C2, C2, C2]);
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(pid(1)), 1.0).with_combat(a_stats), MapDivision::new(md(2), C2, p(pid(0)), 1.0)],
        ctl,
        vec![(C1, C2)],
        vec![cfg(C1, pid(2), 100.0), cfg(C2, pid(3), 100.0)],
    );
    m.issue_move_order(&g, md(1), p(pid(0))).unwrap();
    for _ in 1..=5 {
        m.advance_day(&g);
    }
    let (sa, sd) = (m.division(md(1)).unwrap().combat, m.division(md(2)).unwrap().combat);
    let mut old = old_world(&cells, &controllers, &[(1, 1, 0, sa), (2, 2, 0, sd)], &[(1, 2, 100.0), (2, 3, 100.0)]);
    // 旧: 攻撃側A は中心にいて Fighting。戦闘は進行中(出撃元 idx1)
    old.mil.divisions.get_mut(&DivisionId(1)).unwrap().status = DivisionStatus::Fighting;
    old.mil.divisions.get_mut(&DivisionId(2)).unwrap().status = DivisionStatus::Fighting;
    old.battles
        .start_battle(Battle {
            id: strategy_game::common::BattleId(0),
            war_id: strategy_game::common::WarId(0),
            state_id: StateId(0),
            attacker_country: CountryId(1),
            defender_country: CountryId(2),
            attacker_division_ids: vec![DivisionId(1)],
            defender_division_ids: vec![DivisionId(2)],
            attacker_origins: [(DivisionId(1), StateId(1))].into_iter().collect(),
            start_date: "1800/01/01".into(),
            elapsed_days: 0,
            status: BattleStatus::Ongoing,
        })
        .unwrap();
    let plan1 = old_plan(&old, 1);
    let plan2 = old_plan(&old, 2);
    let ev6 = m.advance_day(&g);
    let (r1, r2) = (report(&ev6, C1), report(&ev6, C2));
    assert_eq!((r1.total_demand_units, r1.eligible_demand_units, r1.consumed_units), (plan1.total_demand_units, plan1.eligible_demand_units, plan1.consumed_units));
    assert_eq!((r2.total_demand_units, r2.eligible_demand_units, r2.consumed_units), (plan2.total_demand_units, plan2.eligible_demand_units, plan2.consumed_units));
    assert_eq!(r1.frontline_divisions, vec![md(1)], "new: the attacker is eligible, like the old function");
    // 旧の判定そのもの: 攻撃側は例外の対象、防御側は(連結の外ではないので)対象外
    let connected1 = connected_states_from_capital(StateId(2), CountryId(1), &old.states);
    assert!(is_frontline_supply_eligible(&old.mil.divisions[&DivisionId(1)], &connected1, &old.states, &old.battles));
    assert!(r2.frontline_divisions.is_empty());
}

// ---------------------------------------------------------------- シナリオ(検証用 RON)

fn template() -> DivisionScenarioFile {
    DivisionScenarioFile {
        format_version: 1,
        description: "t".into(),
        base_days: 5.0,
        countries: vec![CountryEntry { label: 1, country_id: 1 }, CountryEntry { label: 2, country_id: 2 }],
        state_owners: vec![StateOwnerEntry { state: 20, country: 1 }, StateOwnerEntry { state: 21, country: 2 }],
        controller_overrides: vec![],
        wars: vec![WarEntry { a: 1, b: 2 }],
        divisions: vec![
            DivisionEntry { id: 1, country: 1, province: 18309, speed: 1.0, combat: CombatEntry::default() },
            DivisionEntry { id: 2, country: 2, province: 17269, speed: 1.0, combat: CombatEntry::default() },
        ],
        supply: SupplySetup {
            countries: vec![SupplyCountryEntry { country: 1, capital: 18309, military_supplies: 10.0 }, SupplyCountryEntry { country: 2, capital: 17269, military_supplies: 5.0 }],
            divisions: vec![SupplyUsageEntry { division: 1, supply_usage: 1.0 }, SupplyUsageEntry { division: 2, supply_usage: 2.0 }],
        },
    }
}

fn codes(f: &DivisionScenarioFile, g: &ProvinceGraph, w: &WorldModel) -> Vec<&'static str> {
    match resolve_division_scenario(f, g, w) {
        Ok(_) => vec![],
        Err(v) => v.iter().map(|i| i.code()).collect(),
    }
}

#[test]
fn supply_setup_is_validated_with_distinct_codes_and_scenarios_without_it_keep_the_old_behaviour() {
    let (w, g) = real();
    let m = &w.model;
    assert!(codes(&template(), &g, m).is_empty());
    let with = |f: &dyn Fn(&mut DivisionScenarioFile)| {
        let mut t = template();
        f(&mut t);
        codes(&t, &g, m)
    };
    assert_eq!(with(&|t| t.supply.countries.push(SupplyCountryEntry { country: 1, capital: 18309, military_supplies: 1.0 })), vec!["supply-duplicate-country"]);
    assert_eq!(with(&|t| t.supply.countries.push(SupplyCountryEntry { country: 9, capital: 18309, military_supplies: 1.0 })), vec!["unresolved-country"]);
    assert_eq!(with(&|t| t.supply.countries[0].capital = 99999), vec!["supply-unknown-capital"]);
    assert_eq!(with(&|t| t.supply.countries[0].capital = 17269), vec!["supply-capital-not-controlled"], "17269 belongs to country 2's state");
    assert_eq!(with(&|t| t.supply.countries[0].capital = 18110), vec!["supply-capital-not-controlled"], "18110 has no controller");
    for bad in [f64::NAN, -1.0, f64::INFINITY, 1e30] {
        assert_eq!(with(&|t| t.supply.countries[0].military_supplies = bad), vec!["supply-invalid-stockpile"], "stockpile {bad}");
    }
    assert_eq!(with(&|t| t.supply.divisions.push(SupplyUsageEntry { division: 9, supply_usage: 1.0 })), vec!["supply-unknown-division"]);
    assert_eq!(with(&|t| t.supply.divisions.push(SupplyUsageEntry { division: 1, supply_usage: 1.0 })), vec!["supply-duplicate-usage"]);
    for bad in [f32::NAN, -1.0, f32::INFINITY] {
        assert_eq!(with(&|t| t.supply.divisions[0].supply_usage = bad), vec!["supply-invalid-usage"], "usage {bad}");
    }
    assert_eq!(with(&|t| { t.supply.divisions.remove(1); }), vec!["supply-missing-usage"], "a division of a configured country needs a usage entry");
    assert_eq!(with(&|t| { t.supply.countries.remove(1); }), vec!["supply-usage-without-country"]);
    // 補給の設定がないシナリオは従来どおり(補給なし。充足率はすべて 1.0)
    let mut none = template();
    none.supply = SupplySetup::default();
    let sc = resolve_division_scenario(&none, &g, m).unwrap();
    assert!(sc.supply.is_empty());
    assert!(!sc.into_military().supply_enabled());
    for path in ["assets/data/world/preview/division_movement_scenario.ron", "assets/data/world/preview/division_battle_scenario.ron"] {
        let sc = load_division_scenario(path, &g, m).unwrap();
        assert!(sc.supply.is_empty(), "{path}");
        let mut mil = sc.into_military();
        assert!(!mil.supply_enabled());
        let ev = mil.advance_day(&g);
        assert!(ev.iter().all(|e| !matches!(e, MoveEvent::DailySupply(_) | MoveEvent::SupplyFailed { .. })));
        assert!(mil.divisions().all(|d| d.fulfillment == 1.0));
    }
    // 直接構築でも、重複・不正な備蓄・不正な補給消費量は拒否される
    let base = || MapMilitary::new(vec![MapDivision::new(md(1), C1, p(10), 1.0).with_supply_usage(f32::NAN)], control_of(&[(10, C1)]), MovementParams { base_days: 5.0 }).unwrap();
    assert_eq!(base().with_supply(vec![cfg(C1, 10, 1.0)]).unwrap_err(), MapMilitaryError::InvalidSupplyUsage(md(1)));
    let ok = || MapMilitary::new(vec![MapDivision::new(md(1), C1, p(10), 1.0)], control_of(&[(10, C1)]), MovementParams { base_days: 5.0 }).unwrap();
    assert_eq!(ok().with_supply(vec![cfg(C1, 10, 1.0), cfg(C1, 10, 2.0)]).unwrap_err(), MapMilitaryError::DuplicateSupplyCountry(C1));
    assert_eq!(ok().with_supply(vec![cfg(C1, 10, f64::NAN)]).unwrap_err(), MapMilitaryError::InvalidSupplyStockpile(C1));
}

#[test]
fn the_shipped_supply_scenario_resolves_and_runs_with_the_expected_allocations() {
    let (w, g) = real();
    let sc = load_division_scenario(SUPPLY, &g, &w.model).unwrap();
    assert_eq!(sc.supply.len(), 3);
    assert_eq!(sc.divisions.len(), 14);
    let mut m = sc.into_military();
    let ev = m.advance_day(&g);
    // 国家1: 備蓄 100(十分)。連結(18309 から 14 件)の師団は全量、孤立した 17905 の師団11 は切断
    let r1 = report(&ev, C1);
    assert_eq!((r1.connected_provinces, r1.unknown_provinces), (14, 3));
    assert_eq!(f(&m, 11), 0.0);
    assert!([1u32, 2, 3, 4, 7, 14].iter().all(|i| f(&m, *i) == 1.0));
    // 国家2: 備蓄 0.5 は連結した需要(4.25)より小さい → 比例配分(0.5/4.25)
    let r2 = report(&ev, C2);
    assert_eq!((r2.eligible_demand_units, r2.consumed_units, r2.stockpile_units), (4_250_000, 500_000, 0));
    assert!((f(&m, 6) - 500_000.0 / 4_250_000.0).abs() < 1e-5, "the largest-remainder allocation is in whole units");
    assert_eq!(f(&m, 12), 0.0, "division 12 is isolated in a pocket surrounded by country 1");
}

// ---------------------------------------------------------------- 決定性と分離

#[test]
fn supply_results_do_not_depend_on_registration_order() {
    let (w, g) = real();
    let run = |reverse: bool| {
        let mut sc = load_division_scenario(SUPPLY, &g, &w.model).unwrap();
        if reverse {
            sc.divisions.reverse();
            sc.supply.reverse();
        }
        let mut m = sc.into_military();
        let mut orders: Vec<(u32, u32)> = vec![(1, 18502), (2, 17694), (4, 18111), (7, 18111), (14, 17486)];
        if reverse {
            orders.reverse();
        }
        for (d, dest) in orders {
            let _ = m.issue_move_order(&g, md(d), p(dest));
        }
        let mut ev = Vec::new();
        for _ in 0..30 {
            ev.push(m.advance_day(&g));
        }
        (ev, m)
    };
    let (ea, ma) = run(false);
    let (eb, mb) = run(true);
    assert_eq!(ea, eb);
    assert_eq!(ma, mb);
}

#[test]
fn the_supply_code_depends_on_the_old_model_only_through_the_isolated_adapter() {
    let strip = |path: &str| -> String {
        let src = std::fs::read_to_string(path).unwrap();
        src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
    };
    let supply = strip("src/world/supply.rs");
    for banned in ["military::", "war::", "economy::", "CountryStockpile", "MapEconomy", "snapshot", "SaveLocation", "StateRegistry", "std::fs", "std::time", "Instant", "rand::"] {
        assert!(!supply.contains(banned), "supply.rs: found {banned}");
    }
    assert!(!supply.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == "StateId"));
    let adapter = strip("src/world/legacy_supply.rs");
    assert!(adapter.contains("military::daily_supply") && adapter.contains("military::supply_initialization") && adapter.contains("economy::resources"));
    for banned in ["BattleRegistry", "WarRegistry", "StateRegistry", "CountryRegistry", "military::battle", "military::update", "military::movement", "military::invasion", "std::fs", "std::time"] {
        assert!(!adapter.contains(banned), "legacy_supply.rs: found {banned}");
    }
    // 他の新経路のファイルは、旧の補給・戦闘へ直接は依存しない
    for path in ["src/world/movement.rs", "src/world/battle.rs", "src/world/occupation.rs", "src/world/division_scenario.rs"] {
        let code = strip(path);
        for banned in ["military::", "economy::", "StateRegistry", "WarRegistry"] {
            assert!(!code.contains(banned), "{path}: found {banned}");
        }
    }
}
