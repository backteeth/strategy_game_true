//! P32-034 (M6-a): 新経路の師団の初期配置と、自国支配のプロヴィンス間の日単位の移動(`world::movement`・`world::division_scenario`)の検証。
//!
//! 期待値は、手で書いた小さな地図(軸座標。隣接の規則: (q,r) の6近傍は (q+1,r) (q+1,r-1) (q,r-1) (q-1,r) (q-1,r+1) (q,r+1))と、旧経路
//! (`military::movement`・`military::pathfinding`)を**旧型の fixture で実際に実行した結果**から作る(新旧IDの対応表を明示)。
//! 時間は合否条件にしない(性能は `world_move_bench`)。保存・アセットは書かない(読むだけ)。

use std::collections::BTreeMap;

use serde_json::json;
use strategy_game::common::{CountryId, DivisionDefinitionId, DivisionId, StateId};
use strategy_game::hexmap::data::{HexMap, load_from_path, parse_hex_map};
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::data::{Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry};
use strategy_game::military::movement::{process_movement, start_or_maintain_movement, validate_and_stop_invalid_movements};
use strategy_game::military::pathfinding::find_path;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::war::data::WarRegistry;
use strategy_game::world::division_scenario::{CountryEntry, ControllerOverrideEntry, DivisionEntry, StateOwnerEntry};
use strategy_game::world::fingerprint::FINGERPRINT_ALGORITHM;
use strategy_game::world::*;

const C1: CountryId = CountryId(1);
const C2: CountryId = CountryId(2);

// ---------------------------------------------------------------- 人工地図の部品

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

fn graph_of(map: &HexMap, provinces: &[(u32, i32, i32)]) -> ProvinceGraph {
    let mut geography = compute_fingerprint(map);
    geography.algorithm = FINGERPRINT_ALGORITHM;
    let t = ProvinceTableFile {
        format_version: WORLD_FORMAT_VERSION,
        coverage: Coverage::World,
        geography,
        next_province_id: provinces.iter().map(|p| p.0).max().unwrap_or(0) + 1,
        retired: Vec::new(),
        provinces: provinces.iter().map(|&(id, q, r)| ProvinceEntry { id, q, r }).collect(),
    };
    let layout = StateLayoutFile { format_version: WORLD_FORMAT_VERSION, states: Vec::new() };
    let w = WorldModel::build(t, layout).expect("test world must build");
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

fn military(divs: Vec<MapDivision>, control: ProvinceControl, speed_base: f32) -> MapMilitary {
    MapMilitary::new(divs, control, MovementParams { base_days: speed_base }).unwrap()
}

/// 1行 n 陸: ID = 10 + q。すべて国家1の支配。
fn line(n: usize) -> (ProvinceGraph, ProvinceControl) {
    let map = map_from(&[&"#".repeat(n)]);
    let provs: Vec<(u32, i32, i32)> = (0..n).map(|q| (10 + q as u32, q as i32, 0)).collect();
    let c: Vec<(u32, CountryId)> = provs.iter().map(|(id, _, _)| (*id, C1)).collect();
    (graph_of(&map, &provs), control_of(&c))
}

fn pos(m: &MapMilitary, id: u32) -> (u32, MapDivisionStatus, f32) {
    let d = m.division(md(id)).unwrap();
    (d.province.0, d.status, d.progress)
}

// ---------------------------------------------------------------- 命令: 受理・no-op・拒否

#[test]
fn an_order_is_applied_at_once_without_moving_the_division_and_arrival_follows_the_old_formula() {
    let (g, c) = line(5);
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], c, 5.0);
    assert_eq!(m.issue_move_order(&g, md(1), p(11)), Ok(OrderOutcome::Started { hops: 1 }));
    let d = m.division(md(1)).unwrap();
    assert_eq!((d.province, d.destination, d.path.clone(), d.target, d.progress, d.status), (p(10), Some(p(11)), vec![p(11)], None, 0.0, MapDivisionStatus::Moving));
    // 1ホップ = max(1, 5×1/(1.0×1.0)) = 5 日。日1〜4は留まり、日5に着く。
    let mut arrival_day = None;
    for day in 1..=7 {
        let ev = m.advance_day();
        let (prov, st, _) = pos(&m, 1);
        if day < 5 {
            assert_eq!((prov, st), (10, MapDivisionStatus::Moving), "day {day}");
        } else if day == 5 {
            assert_eq!((prov, st), (11, MapDivisionStatus::Idle));
            assert!(ev.contains(&MoveEvent::Arrived { division: md(1), province: p(11) }));
            arrival_day = Some(day);
        }
    }
    assert_eq!(arrival_day, Some(5));
    // 複数ホップ: 4ホップ = 20日。途中の到着は HopCompleted だけで、最終日にだけ Arrived
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], line(5).1, 5.0);
    assert_eq!(m.issue_move_order(&g, md(1), p(14)), Ok(OrderOutcome::Started { hops: 4 }));
    let mut hops = Vec::new();
    let mut arrived = None;
    for day in 1..=25 {
        for e in m.advance_day() {
            match e {
                MoveEvent::HopCompleted { province, .. } => hops.push((day, province.0)),
                MoveEvent::Arrived { province, .. } => arrived = Some((day, province.0)),
                _ => panic!("{e:?}"),
            }
        }
    }
    assert_eq!(hops, vec![(5, 11), (10, 12), (15, 13), (20, 14)]);
    assert_eq!(arrived, Some((20, 14)));
    assert_eq!(pos(&m, 1), (14, MapDivisionStatus::Idle, 0.0));
}

#[test]
fn reissuing_the_same_destination_and_ordering_the_current_province_are_no_ops_that_change_nothing() {
    let (g, c) = line(5);
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], c, 5.0);
    // Idle で現在地: no-op
    let before = m.clone();
    assert_eq!(m.issue_move_order(&g, md(1), p(10)), Ok(OrderOutcome::AlreadyThere));
    assert_eq!(m, before);
    m.issue_move_order(&g, md(1), p(12)).unwrap();
    m.advance_day();
    m.advance_day();
    let mid = m.clone();
    assert!(mid.division(md(1)).unwrap().progress > 0.0);
    // 移動中の同じ目的地: no-op(進捗・経路・目的地・到着日が変わらない)
    assert_eq!(m.issue_move_order(&g, md(1), p(12)), Ok(OrderOutcome::AlreadyMoving));
    assert_eq!(m, mid);
    // 到着日の比較: 再命令した場合と、しなかった場合で同じ日に着く
    let mut a = mid.clone();
    let mut b = mid;
    b.issue_move_order(&g, md(1), p(12)).unwrap();
    for _ in 0..8 {
        a.advance_day();
        b.advance_day();
        assert_eq!(a, b);
    }
}

#[test]
fn rejections_are_distinguished_and_leave_every_division_untouched() {
    // 10 11 12 13 14 のうち 12 は国家2の支配(分断)。13・14 は自国支配だが 12 に遮られる。
    let map = map_from(&["#####"]);
    let provs: Vec<(u32, i32, i32)> = (0..5).map(|q| (10 + q as u32, q, 0)).collect();
    let g = graph_of(&map, &provs);
    let c = control_of(&[(10, C1), (11, C1), (12, C2), (13, C1)]); // 14 は支配国なし
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0), MapDivision::new(md(2), C1, p(11), 1.0)], c, 5.0);
    m.issue_move_order(&g, md(2), p(10)).unwrap(); // 師団2は移動中(拒否が他の師団・自分の命令に影響しないことを見る)
    m.advance_day();
    let before = m.clone();
    assert_eq!(m.issue_move_order(&g, md(9), p(11)), Err(OrderRejection::UnknownDivision(md(9))));
    assert_eq!(m.issue_move_order(&g, md(1), p(999)), Err(OrderRejection::UnknownProvince(p(999))));
    assert_eq!(m.issue_move_order(&g, md(1), p(12)), Err(OrderRejection::DestinationNotPassable { province: p(12), controller: Some(C2) }));
    assert_eq!(m.issue_move_order(&g, md(1), p(14)), Err(OrderRejection::DestinationNotPassable { province: p(14), controller: None }));
    assert_eq!(m.issue_move_order(&g, md(1), p(13)), Err(OrderRejection::NoPath));
    // 移動中の師団への拒否される命令も、既存の命令・経路・進捗を保つ
    assert_eq!(m.issue_move_order(&g, md(2), p(13)), Err(OrderRejection::NoPath));
    assert_eq!(m.issue_move_order(&g, md(2), p(12)), Err(OrderRejection::DestinationNotPassable { province: p(12), controller: Some(C2) }));
    assert_eq!(m, before);
}

#[test]
fn rivers_and_the_state_layout_do_not_change_movement_cost_or_permission() {
    // 河川の辺があっても、1ホップ=5日のまま(費用は常に 1)。
    let mut cells = Vec::new();
    for q in 0..3 {
        cells.push(json!({ "q": q, "r": 0, "terrain": "plains", "climate": "C", "vegetation": "grassland" }));
    }
    let v = json!({
        "schemaVersion": 9, "kind": "hex-atlas-game-map",
        "grid": { "orientation": "pointy", "size": 2.5, "origin": { "x": 2.1650635094610964, "y": 2.5 }, "imageWidth": 100, "imageHeight": 60,
                  "extent": { "minQ": 0, "maxQ": 2, "minR": 0, "maxR": 0 } },
        "cells": cells,
        "rivers": [ { "q": 0, "r": 0, "direction": 0 }, { "q": 1, "r": 0, "direction": 0 } ],
    });
    let map = parse_hex_map(v.to_string().as_bytes()).unwrap();
    let provs = [(10, 0, 0), (11, 1, 0), (12, 2, 0)];
    let g = graph_of(&map, &provs);
    assert!(g.river_edge_count() > 0, "the test map has rivers on shared sides");
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], control_of(&[(10, C1), (11, C1), (12, C1)]), 5.0);
    assert_eq!(m.issue_move_order(&g, md(1), p(12)), Ok(OrderOutcome::Started { hops: 2 }));
    for _ in 0..10 {
        m.advance_day();
    }
    assert_eq!(pos(&m, 1), (12, MapDivisionStatus::Idle, 0.0), "2 hops = 10 days regardless of rivers");
}

// ---------------------------------------------------------------- 経路の tie-break と決定性

/// A(0,0)=10, B(1,0), C(0,1), D(1,1)。B と C はどちらも A・D に隣接(同費用の経路が2本)。
fn diamond(id_b: u32, id_c: u32) -> (ProvinceGraph, ProvinceControl) {
    let map = map_from(&["##", "##"]);
    let provs = [(10, 0, 0), (id_b, 1, 0), (id_c, 0, 1), (40, 1, 1)];
    let g = graph_of(&map, &provs);
    let c = control_of(&[(10, C1), (id_b, C1), (id_c, C1), (40, C1)]);
    (g, c)
}

#[test]
fn equal_cost_candidates_are_taken_in_province_id_order_regardless_of_input_order() {
    for (b, c) in [(30u32, 20u32), (20, 30)] {
        let (g, ctl) = diamond(b, c);
        let path = find_land_path(&g, &ctl, C1, p(10), p(40)).unwrap();
        assert_eq!(path, vec![p(20), p(40)], "the lower ProvinceId (20) is taken, whichever cell holds it (b={b}, c={c})");
    }
    // 入力(プロヴィンス表・支配の列挙)の順序を入れ替えても同じ
    let map = map_from(&["##", "##"]);
    let g1 = graph_of(&map, &[(10, 0, 0), (30, 1, 0), (20, 0, 1), (40, 1, 1)]);
    let g2 = graph_of(&map, &[(40, 1, 1), (20, 0, 1), (30, 1, 0), (10, 0, 0)]);
    let c1 = control_of(&[(10, C1), (30, C1), (20, C1), (40, C1)]);
    let c2 = control_of(&[(40, C1), (20, C1), (30, C1), (10, C1)]);
    assert_eq!(find_land_path(&g1, &c1, C1, p(10), p(40)), find_land_path(&g2, &c2, C1, p(10), p(40)));
}

#[test]
fn division_registration_order_and_order_issue_order_do_not_change_the_result() {
    let (g, c) = line(6);
    let make = |ids: &[u32]| {
        let divs: Vec<MapDivision> = ids.iter().map(|i| MapDivision::new(md(*i), C1, p(10 + *i - 1), 1.0 + (*i as f32) * 0.5)).collect();
        military(divs, c.clone(), 5.0)
    };
    let mut a = make(&[1, 2, 3]);
    let mut b = make(&[3, 1, 2]);
    for (id, dest) in [(1, 15), (2, 10), (3, 11)] {
        a.issue_move_order(&g, md(id), p(dest)).unwrap();
    }
    for (id, dest) in [(3, 11), (1, 15), (2, 10)] {
        b.issue_move_order(&g, md(id), p(dest)).unwrap();
    }
    for _ in 0..30 {
        assert_eq!(a.advance_day(), b.advance_day());
        assert_eq!(a, b);
    }
    assert_eq!(a.divisions().map(|d| d.id.0).collect::<Vec<_>>(), vec![1, 2, 3], "ascending MapDivisionId");
}

// ---------------------------------------------------------------- 命令後に通行条件が変わった場合

#[test]
fn the_division_stops_before_entering_a_province_that_is_no_longer_passable() {
    let (g, c) = line(5);
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], c, 5.0);
    m.issue_move_order(&g, md(1), p(14)).unwrap();
    m.advance_day();
    m.advance_day(); // 10 から 11 へ向かう途中(進捗 0.4)
    assert_eq!(m.division(md(1)).unwrap().target, Some(p(11)));
    m.set_controller(p(11), Some(C2)); // 次の移動先が他国の支配になった
    let ev = m.advance_day();
    assert_eq!(ev, vec![MoveEvent::Stopped { division: md(1), at: p(10), blocked: p(11) }]);
    let d = m.division(md(1)).unwrap();
    assert_eq!((d.province, d.status, d.destination, d.target, d.path.clone(), d.progress), (p(10), MapDivisionStatus::Idle, None, None, vec![], 0.0));
    // 支配国なしになった場合も同じ
    let (g, c) = line(5);
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], c, 5.0);
    m.issue_move_order(&g, md(1), p(12)).unwrap();
    for _ in 0..5 {
        m.advance_day(); // 11 に到着。次は 12 へ
    }
    assert_eq!(pos(&m, 1).0, 11);
    m.set_controller(p(12), None);
    let ev = m.advance_day();
    assert_eq!(ev, vec![MoveEvent::Stopped { division: md(1), at: p(11), blocked: p(12) }]);
    assert_eq!(pos(&m, 1), (11, MapDivisionStatus::Idle, 0.0), "stopped before entering 12");
}

#[test]
fn a_later_province_becoming_impassable_cuts_the_path_and_the_current_hop_still_finishes() {
    let (g, c) = line(5);
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], c, 5.0);
    m.issue_move_order(&g, md(1), p(14)).unwrap();
    m.advance_day();
    m.set_controller(p(13), Some(C2)); // 目的地までの途中(次の次)が通れなくなった
    let ev = m.advance_day();
    assert_eq!(ev, vec![MoveEvent::PathTruncated { division: md(1) }]);
    let d = m.division(md(1)).unwrap();
    assert_eq!((d.destination, d.path.is_empty(), d.target), (None, true, Some(p(11))), "the path and destination are dropped; the hop to 11 continues");
    for _ in 0..3 {
        m.advance_day();
    }
    // 現在のホップ(10→11)だけは完了して、そこで止まる
    assert_eq!(pos(&m, 1), (11, MapDivisionStatus::Idle, 0.0));
}

// ---------------------------------------------------------------- 旧経路との比較(新旧IDの対応表を明示)

/// 旧: 1本の鎖(旧 StateId 0..=n)。すべて国家1が所有。旧 `process_movement` を毎日実行し、各日の (位置, 状態, 進捗) を返す。
/// 旧の1日分の結果: (現在の StateId の数値, 状態, 進捗)。
type OldDay = (usize, DivisionStatus, f32);

fn old_timeline(n: usize, speed: f32, days: usize) -> (Vec<OldDay>, Vec<usize>) {
    let owner = C1;
    let states: Vec<StateData> = (0..=n)
        .map(|i| {
            let mut nb = Vec::new();
            if i > 0 {
                nb.push(StateId(i - 1));
            }
            if i < n {
                nb.push(StateId(i + 1));
            }
            StateData { id: StateId(i), owner_country_id: owner, neighbors: nb, ..Default::default() }
        })
        .collect();
    let mut state_reg = StateRegistry::build(states);
    let def = DivisionDefinition {
        id: DivisionDefinitionId(0),
        name: "t".into(),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        required_manpower: 1,
        required_equipment: 1.0,
        recruitment_days: 1,
        movement_speed: speed,
        attack: 1.0,
        defense: 1.0,
        breakthrough: 1.0,
        organization: 1.0,
        morale: 1.0,
        supply_usage: 1.0,
        maintenance_cost: 0.0,
    };
    let mut mil = MilitaryRegistry::default();
    mil.definitions.insert(def.id, def);
    let div = Division {
        id: DivisionId(1),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(0),
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower: 10_000,
        max_manpower: 10_000,
        equipment: 100.0,
        max_equipment: 100.0,
        organization: 100.0,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: DivisionDefinitionId(0),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    };
    mil.divisions.insert(div.id, div);
    let started = {
        let d = mil.divisions.get_mut(&DivisionId(1)).unwrap();
        start_or_maintain_movement(d, StateId(n), &state_reg, &[owner], &[])
    };
    assert!(started);
    let path: Vec<usize> = mil.divisions[&DivisionId(1)].current_path.iter().map(|s| s.0).collect();
    let war = WarRegistry::default();
    let mut battles = BattleRegistry::default();
    let mut out = Vec::new();
    for _ in 0..days {
        validate_and_stop_invalid_movements(&mut mil, &state_reg, &war);
        process_movement(&mut mil, &mut state_reg, &war, &mut battles, "1800/01/01");
        let d = &mil.divisions[&DivisionId(1)];
        out.push((d.current_state.0, d.status, d.movement_progress));
    }
    (out, path)
}

/// 新: 同じ鎖(ProvinceId = 100 + 3×旧StateId。**旧→新の対応表**。順序を保つ)。
fn new_timeline(n: usize, speed: f32, days: usize) -> (Vec<(u32, MapDivisionStatus, f32)>, Vec<u32>) {
    let (g, c) = {
        let map = map_from(&[&"#".repeat(n + 1)]);
        let provs: Vec<(u32, i32, i32)> = (0..=n).map(|i| (100 + 3 * i as u32, i as i32, 0)).collect();
        let c: Vec<(u32, CountryId)> = provs.iter().map(|(id, _, _)| (*id, C1)).collect();
        (graph_of(&map, &provs), control_of(&c))
    };
    let mut m = military(vec![MapDivision::new(md(1), C1, p(100), speed)], c, 5.0);
    m.issue_move_order(&g, md(1), p(100 + 3 * n as u32)).unwrap();
    let path: Vec<u32> = m.division(md(1)).unwrap().path.iter().map(|x| x.0).collect();
    let mut out = Vec::new();
    for _ in 0..days {
        m.advance_day();
        let (a, b, c) = pos(&m, 1);
        out.push((a, b, c));
    }
    (out, path)
}

#[test]
fn one_hop_and_multi_hop_arrival_days_match_the_old_path_for_the_same_adjacency_cost_and_speed() {
    let map_province = |old: usize| 100 + 3 * old as u32; // 新旧IDの対応表(旧 StateId → 新 ProvinceId)
    for speed in [1.0f32, 2.0, 0.5, 10.0, 1.7] {
        for hops in [1usize, 2, 4] {
            let days = 60;
            let (old, old_path) = old_timeline(hops, speed, days);
            let (new, new_path) = new_timeline(hops, speed, days);
            assert_eq!(old_path.iter().map(|s| map_province(*s)).collect::<Vec<_>>(), new_path, "speed {speed} hops {hops}: the same path");
            for (day, (o, n)) in old.iter().zip(&new).enumerate() {
                let old_status = match o.1 {
                    DivisionStatus::Idle => MapDivisionStatus::Idle,
                    DivisionStatus::Moving => MapDivisionStatus::Moving,
                    other => panic!("{other:?}"),
                };
                assert_eq!((map_province(o.0), old_status), (n.0, n.1), "speed {speed} hops {hops} day {}: position/status", day + 1);
                assert_eq!(o.2, n.2, "speed {speed} hops {hops} day {}: progress (same f32 arithmetic)", day + 1);
            }
            // 到着日(最初に Idle で最終地点にいる日)
            let arr = |v: Vec<bool>| v.iter().position(|b| *b);
            let old_arrival = arr(old.iter().map(|o| o.0 == hops && o.1 == DivisionStatus::Idle).collect());
            let new_arrival = arr(new.iter().map(|n| n.0 == map_province(hops) && n.1 == MapDivisionStatus::Idle).collect());
            assert_eq!(old_arrival, new_arrival);
            assert!(old_arrival.is_some(), "speed {speed} hops {hops} arrives within {days} days");
        }
    }
    // 具体値(仕様): 速度1.0・1ホップは5日目に到着、4ホップは20日目
    assert_eq!(new_timeline(1, 1.0, 10).0.iter().position(|n| n.1 == MapDivisionStatus::Idle), Some(4));
    assert_eq!(new_timeline(4, 1.0, 30).0.iter().position(|n| n.0 == 112 && n.1 == MapDivisionStatus::Idle), Some(19));
}

#[test]
fn equal_cost_path_choice_matches_the_old_find_path_when_the_id_correspondence_preserves_order() {
    // 旧 StateId: A=0, C=1, B=2, D=3(新 ProvinceId A=10, C=20, B=30, D=40 と順序が対応する)。隣接は手書き(六角形の隣接と同じ関係)。
    let state = |id: usize, nb: &[usize]| StateData { id: StateId(id), owner_country_id: C1, neighbors: nb.iter().map(|x| StateId(*x)).collect(), ..Default::default() };
    let reg = StateRegistry::build(vec![state(0, &[2, 1]), state(1, &[0, 2, 3]), state(2, &[0, 1, 3]), state(3, &[2, 1])]);
    let old = find_path(StateId(0), StateId(3), &reg, &[C1], &[]).unwrap();
    assert_eq!(old, vec![StateId(1), StateId(3)], "old: via the lower StateId (1 = C)");
    let (g, ctl) = diamond(30, 20);
    let new = find_land_path(&g, &ctl, C1, p(10), p(40)).unwrap();
    let table: BTreeMap<usize, u32> = [(0, 10), (1, 20), (2, 30), (3, 40)].into_iter().collect();
    assert_eq!(old.iter().map(|s| table[&s.0]).collect::<Vec<_>>(), new.iter().map(|x| x.0).collect::<Vec<_>>());
    // 意図した差(仕様上の扱い): 同費用の選択は「各モデルのID昇順」なので、**対応表が順序を保たない**と選ばれる経路が変わる。
    // 旧の番号を A=0, B=1, C=2, D=3(B < C)にすると旧は B を選ぶが、新は ProvinceId の小さい C(20)を選ぶ。
    let reg2 = StateRegistry::build(vec![state(0, &[1, 2]), state(1, &[0, 2, 3]), state(2, &[0, 1, 3]), state(3, &[1, 2])]);
    let old2 = find_path(StateId(0), StateId(3), &reg2, &[C1], &[]).unwrap();
    assert_eq!(old2, vec![StateId(1), StateId(3)], "old with a different numbering: via B");
    assert_eq!(new.iter().map(|x| x.0).collect::<Vec<_>>(), vec![20, 40], "new: via the lower ProvinceId, independent of the old numbering");
}

// ---------------------------------------------------------------- シナリオ(実データ: 検証地域の29プロヴィンス・3ステート)

const SCENARIO: &str = "assets/data/world/preview/division_movement_scenario.ron";
const NO_BRIDGE: &str = "assets/data/world/preview/division_movement_scenario_no_bridge.ron";

fn real() -> (HexMap, WorldPreviewData, ProvinceGraph) {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let w = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).unwrap();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    (map, w, g)
}

#[test]
fn the_shipped_scenarios_resolve_the_initial_control_from_membership_and_explicit_overrides_only() {
    let (_m, w, g) = real();
    let s = load_division_scenario(SCENARIO, &g, &w.model).unwrap();
    assert_eq!(s.params.base_days, 5.0);
    assert_eq!(s.countries.len(), 2);
    // ステート20・22 は国家1、21 は国家2(所属情報から)。未所属のうち明示した 17903 だけが国家1。18110・18501 は支配国なし。
    for sid in [20u32, 22] {
        for pr in w.model.provinces_of(MapStateId(sid)).unwrap() {
            assert_eq!(s.control.controller(*pr), Some(C1), "{pr}");
        }
    }
    for pr in w.model.provinces_of(MapStateId(21)).unwrap() {
        assert_eq!(s.control.controller(*pr), Some(C2));
    }
    assert_eq!(s.control.controller(p(17903)), Some(C1));
    assert_eq!(s.control.controller(p(18110)), None);
    assert_eq!(s.control.controller(p(18501)), None);
    assert_eq!(s.control.len(), 27, "26 assigned + the one explicit override");
    assert_eq!(s.divisions.iter().map(|d| (d.id.0, d.owner.0, d.province.0)).collect::<Vec<_>>(), vec![(1, 1, 18309), (2, 1, 17266), (3, 2, 17269)]);
    // 明示を外した版: 差は 17903 だけ
    let nb = load_division_scenario(NO_BRIDGE, &g, &w.model).unwrap();
    assert_eq!(nb.control.controller(p(17903)), None);
    assert_eq!(nb.control.len(), 26);
}

#[test]
fn real_18309_to_18308_stays_on_day_one_and_arrives_on_day_five_even_with_a_reissued_order() {
    let (_m, w, g) = real();
    let mut m = load_division_scenario(SCENARIO, &g, &w.model).unwrap().into_military();
    assert_eq!(m.issue_move_order(&g, md(1), p(18308)), Ok(OrderOutcome::Started { hops: 1 }));
    assert_eq!(pos(&m, 1).0, 18309, "the order does not move the division");
    let mut timeline = Vec::new();
    for day in 1..=6 {
        if day == 3 {
            assert_eq!(m.issue_move_order(&g, md(1), p(18308)), Ok(OrderOutcome::AlreadyMoving));
        }
        m.advance_day();
        timeline.push(pos(&m, 1));
    }
    assert_eq!(timeline[0].0, 18309, "stays on day 1");
    assert_eq!(timeline[3].0, 18309);
    assert_eq!((timeline[4].0, timeline[4].1), (18308, MapDivisionStatus::Idle), "arrives on day 5");
    // 再命令なしと同じ進捗
    let mut m2 = load_division_scenario(SCENARIO, &g, &w.model).unwrap().into_military();
    m2.issue_move_order(&g, md(1), p(18308)).unwrap();
    for t in timeline.iter().take(6) {
        m2.advance_day();
        assert_eq!(pos(&m2, 1), *t);
    }
}

#[test]
fn real_18309_to_17266_crosses_the_state_boundary_in_five_hops_only_when_17903_is_explicitly_controlled() {
    let (_m, w, g) = real();
    let mut m = load_division_scenario(SCENARIO, &g, &w.model).unwrap().into_military();
    assert_eq!(m.issue_move_order(&g, md(1), p(17266)), Ok(OrderOutcome::Started { hops: 5 }));
    let d = m.division(md(1)).unwrap();
    assert_eq!(d.path.iter().map(|x| x.0).collect::<Vec<_>>(), vec![18109, 17903, 17693, 17484, 17266]);
    // 状態の境界: 18309(20)→18109(20)→17903(未所属)→17693(22)→...
    assert_eq!(g.state_of(p(18109)), Some(MapStateId(20)));
    assert_eq!(g.state_of(p(17903)), None);
    assert_eq!(g.state_of(p(17693)), Some(MapStateId(22)));
    let mut arrival = None;
    for day in 1..=30 {
        for e in m.advance_day() {
            if let MoveEvent::Arrived { province, .. } = e {
                arrival = Some((day, province.0));
            }
        }
    }
    assert_eq!(arrival, Some((25, 17266)), "5 hops x 5 days");
    // 明示を外した条件: 設計書どおり NoPath(目的地 17266 は国家1の支配なので DestinationNotPassable ではない)
    let mut nb = load_division_scenario(NO_BRIDGE, &g, &w.model).unwrap().into_military();
    let before = nb.clone();
    assert_eq!(nb.issue_move_order(&g, md(1), p(17266)), Err(OrderRejection::NoPath));
    assert_eq!(nb, before);
}

#[test]
fn real_rejections_other_controller_unknown_id_and_no_route() {
    let (_m, w, g) = real();
    let mut m = load_division_scenario(SCENARIO, &g, &w.model).unwrap().into_military();
    let before = m.clone();
    // 他国支配(ステート21は国家2)
    assert_eq!(m.issue_move_order(&g, md(1), p(17269)), Err(OrderRejection::DestinationNotPassable { province: p(17269), controller: Some(C2) }));
    // 支配国なし(未所属で明示していない 18110)
    assert_eq!(m.issue_move_order(&g, md(1), p(18110)), Err(OrderRejection::DestinationNotPassable { province: p(18110), controller: None }));
    // 未知のID・未知の師団
    assert_eq!(m.issue_move_order(&g, md(1), p(99999)), Err(OrderRejection::UnknownProvince(p(99999))));
    assert_eq!(m.issue_move_order(&g, md(7), p(18308)), Err(OrderRejection::UnknownDivision(md(7))));
    // 国家2の師団は国家1の領域へは動けない
    assert_eq!(m.issue_move_order(&g, md(3), p(18309)), Err(OrderRejection::DestinationNotPassable { province: p(18309), controller: Some(C1) }));
    assert_eq!(m, before);
}

#[test]
fn real_control_change_after_the_order_stops_the_division_before_entering() {
    let (_m, w, g) = real();
    let mut m = load_division_scenario(SCENARIO, &g, &w.model).unwrap().into_military();
    m.issue_move_order(&g, md(1), p(17266)).unwrap();
    for _ in 0..7 {
        m.advance_day(); // 18309 → 18109 (day 5)、18109 → 17903 の途中
    }
    assert_eq!(pos(&m, 1).0, 18109);
    m.set_controller(p(17903), None); // 橋渡しの明示が失われた(通行条件の変更)
    let ev = m.advance_day();
    assert_eq!(ev, vec![MoveEvent::Stopped { division: md(1), at: p(18109), blocked: p(17903) }]);
    assert_eq!(pos(&m, 1), (18109, MapDivisionStatus::Idle, 0.0));
}

// ---------------------------------------------------------------- シナリオの検証(種類ごと)

fn template() -> DivisionScenarioFile {
    DivisionScenarioFile {
        format_version: 1,
        description: "t".into(),
        base_days: 5.0,
        countries: vec![CountryEntry { label: 1, country_id: 1 }, CountryEntry { label: 2, country_id: 2 }],
        state_owners: vec![StateOwnerEntry { state: 20, country: 1 }, StateOwnerEntry { state: 21, country: 2 }],
        controller_overrides: vec![],
        divisions: vec![DivisionEntry { id: 1, country: 1, province: 18309, speed: 1.0 }],
    }
}

fn codes(f: &DivisionScenarioFile, g: &ProvinceGraph, w: &WorldModel) -> Vec<&'static str> {
    match resolve_division_scenario(f, g, w) {
        Ok(_) => vec![],
        Err(v) => v.iter().map(|i| i.code()).collect(),
    }
}

#[test]
fn every_kind_of_invalid_scenario_input_is_reported_with_its_own_code() {
    let (_m, w, g) = real();
    let m = &w.model;
    assert!(codes(&template(), &g, m).is_empty());
    let with = |f: &dyn Fn(&mut DivisionScenarioFile)| {
        let mut t = template();
        f(&mut t);
        codes(&t, &g, m)
    };
    assert_eq!(with(&|t| t.countries.push(CountryEntry { label: 1, country_id: 9 })), vec!["duplicate-country-label"]);
    assert_eq!(with(&|t| t.countries.push(CountryEntry { label: 3, country_id: 2 })), vec!["duplicate-country-id"]);
    assert_eq!(with(&|t| t.state_owners.push(StateOwnerEntry { state: 22, country: 7 })), vec!["unresolved-country"]);
    assert_eq!(with(&|t| t.controller_overrides.push(ControllerOverrideEntry { province: 17903, country: 7 })), vec!["unresolved-country"]);
    assert_eq!(with(&|t| t.divisions[0].country = 7), vec!["unresolved-country"]);
    assert_eq!(with(&|t| t.state_owners.push(StateOwnerEntry { state: 20, country: 2 })), vec!["duplicate-state-owner"]);
    assert_eq!(with(&|t| t.state_owners.push(StateOwnerEntry { state: 99, country: 1 })), vec!["unknown-state"]);
    assert_eq!(
        with(&|t| t.controller_overrides.extend([ControllerOverrideEntry { province: 17903, country: 1 }, ControllerOverrideEntry { province: 17903, country: 2 }])),
        vec!["duplicate-override"]
    );
    assert_eq!(with(&|t| t.controller_overrides.push(ControllerOverrideEntry { province: 99999, country: 1 })), vec!["unknown-province"]);
    assert_eq!(with(&|t| t.divisions[0].province = 99999), vec!["unknown-province"]);
    assert_eq!(with(&|t| t.divisions.push(DivisionEntry { id: 1, country: 1, province: 18308, speed: 1.0 })), vec!["duplicate-division"]);
    // 初期位置の支配不一致: 他国の支配地(ステート21)・支配国なし(未所属 18110)
    assert_eq!(with(&|t| t.divisions[0].province = 17269), vec!["initial-position-not-controlled"]);
    assert_eq!(with(&|t| t.divisions[0].province = 18110), vec!["initial-position-not-controlled"]);
    // 非有限・非正の速度、基準日数
    for bad in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(with(&|t| t.divisions[0].speed = bad), vec!["invalid-speed"], "speed {bad}");
        assert_eq!(with(&|t| t.base_days = bad), vec!["invalid-base-days"], "base_days {bad}");
    }
    // 複数の問題はすべて集める
    assert_eq!(
        with(&|t| {
            t.base_days = 0.0;
            t.divisions[0].speed = -1.0;
            t.divisions.push(DivisionEntry { id: 2, country: 9, province: 99999, speed: 1.0 });
        }),
        vec!["invalid-base-days", "invalid-speed", "unresolved-country", "unknown-province"]
    );
}

#[test]
fn format_failures_are_distinguished_from_content_issues() {
    let (_m, w, g) = real();
    assert_eq!(parse_division_scenario("(format_version: 2)").unwrap_err(), DivisionScenarioError::UnsupportedVersion { found: 2, supported: 1 });
    assert!(matches!(parse_division_scenario("(format_version: 1, base_days: 5.0,"), Err(DivisionScenarioError::Parse(_))));
    // 必須項目の欠落(base_days なし)、未知のフィールド
    assert!(matches!(parse_division_scenario("(format_version: 1, countries: [], state_owners: [], divisions: [])"), Err(DivisionScenarioError::Parse(_))));
    assert!(matches!(
        parse_division_scenario("(format_version: 1, base_days: 5.0, countries: [], state_owners: [], divisions: [], extra: 1)"),
        Err(DivisionScenarioError::Parse(_))
    ));
    assert!(matches!(load_division_scenario("assets/data/world/preview/nope.ron", &g, &w.model), Err(DivisionScenarioError::Io { .. })));
    // 初期位置が他国の支配 → Invalid(形式の失敗とは別)
    let text = "(format_version: 1, base_days: 5.0, countries: [(label: 1, country_id: 1)], state_owners: [], divisions: [(id: 1, country: 1, province: 18309, speed: 1.0)])";
    let f = parse_division_scenario(text).unwrap();
    assert!(resolve_division_scenario(&f, &g, &w.model).is_err());
}

#[test]
fn ids_are_resolved_through_membership_never_by_numeric_equality() {
    // MapStateId 5 と ProvinceId 5 が同じ数値。所有は「ステート」の表で、プロヴィンスの番号では引かない。
    let map = map_from(&["###"]);
    let mut geography = compute_fingerprint(&map);
    geography.algorithm = FINGERPRINT_ALGORITHM;
    let t = ProvinceTableFile {
        format_version: WORLD_FORMAT_VERSION,
        coverage: Coverage::World,
        geography,
        next_province_id: 8,
        retired: vec![],
        provinces: vec![ProvinceEntry { id: 5, q: 0, r: 0 }, ProvinceEntry { id: 6, q: 1, r: 0 }, ProvinceEntry { id: 7, q: 2, r: 0 }],
    };
    let layout = StateLayoutFile {
        format_version: WORLD_FORMAT_VERSION,
        states: vec![
            StateEntry { id: 5, name_key: "a".into(), provinces: vec![6, 7], representative: None, allow_disconnected: true },
            StateEntry { id: 6, name_key: "b".into(), provinces: vec![5], representative: None, allow_disconnected: true },
        ],
    };
    let w = WorldModel::build(t, layout).unwrap();
    let g = ProvinceGraph::build(&w, &map).unwrap();
    let f = DivisionScenarioFile {
        format_version: 1,
        description: String::new(),
        base_days: 5.0,
        countries: vec![CountryEntry { label: 1, country_id: 100 }, CountryEntry { label: 2, country_id: 200 }],
        state_owners: vec![StateOwnerEntry { state: 5, country: 1 }, StateOwnerEntry { state: 6, country: 2 }],
        controller_overrides: vec![],
        divisions: vec![DivisionEntry { id: 5, country: 1, province: 6, speed: 1.0 }],
    };
    let s = resolve_division_scenario(&f, &g, &w).unwrap();
    // ProvinceId 5 は MapStateId 6(国家ラベル2 = CountryId 200)に属する。MapStateId 5 の所有国(CountryId 100)ではない。
    assert_eq!(s.control.controller(p(5)), Some(CountryId(200)));
    assert_eq!(s.control.controller(p(6)), Some(CountryId(100)));
    assert_eq!(s.control.controller(p(7)), Some(CountryId(100)));
    // 師団ID 5 は ProvinceId 5 とも MapStateId 5 とも無関係
    assert_eq!(s.divisions[0].id, md(5));
    assert_eq!(s.divisions[0].province, p(6));
}

// ---------------------------------------------------------------- 分離の確認

#[test]
fn the_movement_code_stays_separate_from_economy_saving_war_and_the_old_military() {
    for (path, banned) in [
        (
            "src/world/movement.rs",
            vec!["MapEconomy", "economy::", "snapshot", "SaveLocation", "military::", "war::", "StateRegistry", "std::fs", "std::time", "Instant", "rand::", "StateId"],
        ),
        (
            "src/world/division_scenario.rs",
            vec!["MapEconomy", "economy::", "EconomyScenario", "ScenarioCountry", "snapshot", "SaveLocation", "military::", "war::", "StateRegistry", "fs::write", "File::create", "std::time", "rand::"],
        ),
    ] {
        let src = std::fs::read_to_string(path).unwrap();
        let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for b in banned {
            // 旧 `StateId` は `MapStateId` の部分文字列なので、型の境界の確認は単語としてだけ見る
            if b == "StateId" {
                assert!(!code.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == "StateId"), "{path}: must not use the old StateId");
            } else {
                assert!(!code.contains(b), "{path}: found {b}");
            }
        }
    }
}
