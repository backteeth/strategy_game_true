//! P32-035 (M6-b): 交戦国への進入・プロヴィンス単位の戦闘・退却(`world::movement`・`world::battle`・`world::division_scenario`)の検証。
//!
//! 旧経路との比較は、**旧の日次処理(`military::update::handle_daily_military`)を最小の Bevy App で実際に動かし**、同じ条件の新経路と各日の状態を比べる
//! (新旧IDの対応表: 旧 `StateId i` ↔ 新 `ProvinceId 100+10i`、旧 `DivisionId n` ↔ 新 `MapDivisionId n`。隣接はテスト内で独立に六角形の規則から作る)。
//! 実効支配の更新(占領)は M6-c で新経路にも入った(旧と同じく、到着処理・終了処理の中で、当日の組織率回復の前に更新する)。M6-b で入れていた比較ハーネスの補正は不要になり、
//! 占領後も各日の全項目を比べる。時間は合否条件にしない。保存・アセットは書かない。

use std::collections::{BTreeMap, BTreeSet};

use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use serde_json::json;
use strategy_game::app::time::DayChangedMessage;
use strategy_game::common::{CountryId, DivisionDefinitionId, DivisionId, StateId, WarId};
use strategy_game::country::CountryRegistry;
use strategy_game::hexmap::data::{HexMap, load_from_path, parse_hex_map};
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::data::{Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry};
use strategy_game::military::movement::start_or_maintain_movement;
use strategy_game::military::update::handle_daily_military;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::world::division_scenario::{CombatEntry, CountryEntry, DivisionEntry, StateOwnerEntry, WarEntry};
use strategy_game::world::fingerprint::FINGERPRINT_ALGORITHM;
use strategy_game::world::*;

const C1: CountryId = CountryId(1);
const C2: CountryId = CountryId(2);
const C3: CountryId = CountryId(3);

const BATTLE: &str = "assets/data/world/preview/division_battle_scenario.ron";
const MOVE: &str = "assets/data/world/preview/division_movement_scenario.ron";

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

fn graph_with_states(map: &HexMap, provinces: &[(u32, i32, i32)], states: &[(u32, Vec<u32>)]) -> (WorldModel, ProvinceGraph) {
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
    let layout = StateLayoutFile {
        format_version: WORLD_FORMAT_VERSION,
        states: states
            .iter()
            .map(|(id, ps)| StateEntry { id: *id, name_key: format!("t.{id}"), provinces: ps.clone(), representative: None, allow_disconnected: true })
            .collect(),
    };
    let w = WorldModel::build(t, layout).expect("test world must build");
    let g = ProvinceGraph::build(&w, map).expect("graph must build");
    (w, g)
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

fn weak() -> CombatStats {
    CombatStats { manpower: 2500, organization: 20.0, ..CombatStats::standard() }
}

fn inert() -> CombatStats {
    CombatStats { attack_power: 0, defense_power: 0, ..CombatStats::standard() }
}

/// 星形(中心 idx0 と、その6隣接 idx1..6)。ID = 100 + 10×idx。座標: idx0=(1,1) idx1=(2,1) idx2=(2,0) idx3=(1,0) idx4=(0,1) idx5=(0,2) idx6=(1,2)。
const STAR: [(i32, i32); 7] = [(1, 1), (2, 1), (2, 0), (1, 0), (0, 1), (0, 2), (1, 2)];

fn pid(idx: usize) -> u32 {
    100 + 10 * idx as u32
}

fn star_graph() -> ProvinceGraph {
    let map = map_from(&[".##", "###", "##."]);
    let provs: Vec<(u32, i32, i32)> = STAR.iter().enumerate().map(|(i, (q, r))| (pid(i), *q, *r)).collect();
    graph_with_states(&map, &provs, &[]).1
}

fn star_control(c: &[CountryId; 7]) -> ProvinceControl {
    control_of(&c.iter().enumerate().map(|(i, c)| (pid(i), *c)).collect::<Vec<_>>())
}

fn military(divs: Vec<MapDivision>, control: ProvinceControl, wars: Vec<(CountryId, CountryId)>) -> MapMilitary {
    MapMilitary::new(divs, control, MovementParams { base_days: 5.0 }).unwrap().with_wars(wars).unwrap()
}

fn div(id: u32, owner: CountryId, idx: usize, combat: CombatStats) -> MapDivision {
    MapDivision::new(md(id), owner, p(pid(idx)), 1.0).with_combat(combat)
}

fn claims(ev: &[MoveEvent]) -> Vec<&MoveEvent> {
    ev.iter().filter(|e| matches!(e, MoveEvent::ControlClaim { .. })).collect()
}

/// 不変条件: 師団は同時に複数の戦闘に属さない・同じ戦闘へ重複登録されない・参加者は Fighting で戦闘IDが一致・1地点に進行中の戦闘は1つ。
fn check_invariants(m: &MapMilitary) {
    let mut seen: BTreeMap<MapDivisionId, MapBattleId> = BTreeMap::new();
    let mut provinces: BTreeSet<ProvinceId> = BTreeSet::new();
    for b in m.battles() {
        if b.status == MapBattleStatus::Ongoing {
            assert!(provinces.insert(b.province), "two ongoing battles at {}", b.province);
        }
        for list in [&b.attackers, &b.defenders] {
            let mut sorted = list.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(&sorted, list, "{} participants are sorted and unique", b.id);
            for id in list {
                assert!(seen.insert(*id, b.id).is_none(), "{id} is registered in more than one battle (or twice)");
                let d = m.division(*id).unwrap_or_else(|| panic!("{id} in {} does not exist", b.id));
                assert_eq!((d.status, d.battle, d.province), (MapDivisionStatus::Fighting, Some(b.id), b.province), "{id}");
            }
        }
    }
    for d in m.divisions() {
        if d.status == MapDivisionStatus::Fighting {
            assert!(d.battle.is_some() && seen.get(&d.id) == d.battle.as_ref(), "{} is Fighting without being in its battle", d.id);
            assert!(d.target.is_none() && d.destination.is_none() && d.path.is_empty(), "{} fights without orders", d.id);
        } else {
            assert!(d.battle.is_none(), "{} is not fighting but holds a battle id", d.id);
        }
    }
}

// ---------------------------------------------------------------- シナリオの検証(交戦関係・戦闘能力)

fn real() -> (HexMap, WorldPreviewData, ProvinceGraph) {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let w = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).unwrap();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    (map, w, g)
}

fn template() -> DivisionScenarioFile {
    DivisionScenarioFile {
        format_version: 1,
        description: "t".into(),
        base_days: 5.0,
        countries: vec![CountryEntry { label: 1, country_id: 1 }, CountryEntry { label: 2, country_id: 2 }],
        state_owners: vec![StateOwnerEntry { state: 20, country: 1 }, StateOwnerEntry { state: 21, country: 2 }],
        controller_overrides: vec![],
        wars: vec![WarEntry { a: 1, b: 2 }],
        divisions: vec![DivisionEntry { id: 1, country: 1, province: 18309, speed: 1.0, combat: CombatEntry::default() }],
    }
}

fn codes(f: &DivisionScenarioFile, g: &ProvinceGraph, w: &WorldModel) -> Vec<&'static str> {
    match resolve_division_scenario(f, g, w) {
        Ok(_) => vec![],
        Err(v) => v.iter().map(|i| i.code()).collect(),
    }
}

#[test]
fn war_pairs_are_validated_bidirectionally_and_scenarios_without_wars_still_run() {
    let (_m, w, g) = real();
    let m = &w.model;
    assert!(codes(&template(), &g, m).is_empty());
    let with = |f: &dyn Fn(&mut DivisionScenarioFile)| {
        let mut t = template();
        f(&mut t);
        codes(&t, &g, m)
    };
    // 未知の国家 / 自国との組 / 逆順を含む重複
    assert_eq!(with(&|t| t.wars.push(WarEntry { a: 1, b: 9 })), vec!["unresolved-country"]);
    assert_eq!(with(&|t| t.wars = vec![WarEntry { a: 2, b: 2 }]), vec!["self-war"]);
    assert_eq!(with(&|t| t.wars.push(WarEntry { a: 2, b: 1 })), vec!["duplicate-war"], "the reversed pair is a duplicate");
    assert_eq!(with(&|t| t.wars.push(WarEntry { a: 1, b: 2 })), vec!["duplicate-war"]);
    // 戦闘能力の不正
    assert_eq!(with(&|t| t.divisions[0].combat.manpower = 20_000), vec!["invalid-combat"]);
    assert_eq!(with(&|t| t.divisions[0].combat.equipment = f64::NAN), vec!["invalid-combat"]);
    assert_eq!(with(&|t| t.divisions[0].combat.max_organization = 0.0), vec!["invalid-combat"]);
    assert_eq!(with(&|t| t.divisions[0].combat.attack_power = -1), vec!["invalid-combat"]);
    // 交戦は双方向
    let sc = resolve_division_scenario(&template(), &g, m).unwrap();
    let mil = sc.into_military();
    assert!(mil.at_war(C1, C2) && mil.at_war(C2, C1));
    assert!(!mil.at_war(C1, C1) && !mil.at_war(C1, C3));
    // 交戦指定のない M6-a のシナリオ(wars を書かない)は従来どおり読め、移動できる
    let sc = load_division_scenario(MOVE, &g, m).unwrap();
    assert!(sc.wars.is_empty());
    let mut mil = sc.into_military();
    assert_eq!(mil.issue_move_order(&g, md(1), p(18308)), Ok(OrderOutcome::Started { hops: 1 }));
    for _ in 0..5 {
        mil.advance_day(&g);
    }
    assert_eq!(mil.division(md(1)).unwrap().province, p(18308));
    // 直接構築でも、自国との組・重複は拒否される
    let base = || MapMilitary::new(vec![], ProvinceControl::default(), MovementParams { base_days: 5.0 }).unwrap();
    assert_eq!(base().with_wars(vec![(C1, C1)]).unwrap_err(), MapMilitaryError::SelfWar(C1));
    assert_eq!(base().with_wars(vec![(C1, C2), (C2, C1)]).unwrap_err(), MapMilitaryError::DuplicateWar(C2, C1));
}

#[test]
fn the_shipped_battle_scenario_resolves_with_explicit_wars_and_combat() {
    let (_m, w, g) = real();
    let s = load_division_scenario(BATTLE, &g, &w.model).unwrap();
    assert_eq!(s.wars, vec![(C1, C2), (C3, C2)]);
    assert_eq!(s.divisions.len(), 14);
    assert_eq!(s.control.controller(p(18312)), Some(C3), "an explicit override beats the state owner (state 21 = country 2)");
    for q in [17696u32, 17905, 18112] {
        assert_eq!(s.control.controller(p(q)), Some(C1));
    }
    let weak_ids: Vec<u32> = s.divisions.iter().filter(|d| d.combat.manpower == 2500).map(|d| d.id.0).collect();
    assert_eq!(weak_ids, vec![3, 5, 12]);
}

// ---------------------------------------------------------------- 進入の可否(命令と日次)

#[test]
fn belligerents_may_try_to_enter_enemy_provinces_and_other_countries_are_distinguished() {
    let (_m, w, g) = real();
    let mut m = load_division_scenario(BATTLE, &g, &w.model).unwrap().into_military();
    // 交戦国(国家2)の支配地(18502)へは進入を試みられる
    assert_eq!(m.issue_move_order(&g, md(1), p(18502)), Ok(OrderOutcome::Started { hops: 1 }));
    // 非交戦国(国家3)の支配地 / 支配国なし(未所属の 18110)/ 未知のID
    let before = m.clone();
    assert_eq!(m.issue_move_order(&g, md(4), p(18312)), Err(OrderRejection::DestinationNotPassable { province: p(18312), controller: Some(C3) }));
    assert_eq!(m.issue_move_order(&g, md(4), p(18110)), Err(OrderRejection::DestinationNotPassable { province: p(18110), controller: None }));
    assert_eq!(m.issue_move_order(&g, md(4), p(99999)), Err(OrderRejection::UnknownProvince(p(99999))));
    assert_eq!(m, before, "rejections change nothing");
    // 交戦は双方向: 国家2の師団は国家1の支配地へ進入できる
    assert!(matches!(m.issue_move_order(&g, md(8), p(18311)), Ok(OrderOutcome::Started { .. })));
    // 国家3は国家2とは交戦しているが国家1とは交戦していない
    assert!(matches!(m.issue_move_order(&g, md(10), p(18111)), Ok(OrderOutcome::Started { .. })));
    assert!(matches!(m.issue_move_order(&g, md(10), p(18311)), Err(OrderRejection::DestinationNotPassable { controller: Some(c), .. }) if c == C1));
}

#[test]
fn a_province_that_stops_being_enterable_stops_the_division_before_arriving() {
    // 10(国家1) 11(国家2) 12(国家2)。命令後に 11 が非交戦国(国家3)の支配になる → 進入前に停止
    let map = map_from(&["###"]);
    let provs = [(10, 0, 0), (11, 1, 0), (12, 2, 0)];
    let (_w, g) = graph_with_states(&map, &provs, &[]);
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], control_of(&[(10, C1), (11, C2), (12, C2)]), vec![(C1, C2)]);
    assert_eq!(m.issue_move_order(&g, md(1), p(12)), Ok(OrderOutcome::Started { hops: 2 }), "a path through enemy provinces is accepted");
    m.advance_day(&g);
    m.advance_day(&g);
    m.set_controller(p(11), Some(C3));
    let ev = m.advance_day(&g);
    assert_eq!(ev, vec![MoveEvent::Stopped { division: md(1), at: p(10), blocked: p(11) }]);
    let d = m.division(md(1)).unwrap();
    assert_eq!((d.province, d.status, d.destination, d.path.is_empty()), (p(10), MapDivisionStatus::Idle, None, true));
    // 支配国なしになった場合も同じ
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], control_of(&[(10, C1), (11, C2), (12, C2)]), vec![(C1, C2)]);
    m.issue_move_order(&g, md(1), p(11)).unwrap();
    m.advance_day(&g);
    m.set_controller(p(11), None);
    assert_eq!(m.advance_day(&g), vec![MoveEvent::Stopped { division: md(1), at: p(10), blocked: p(11) }]);
    // 戦争がなければ(交戦指定なし)敵支配地へは命令が通らない
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], control_of(&[(10, C1), (11, C2), (12, C2)]), vec![]);
    assert_eq!(m.issue_move_order(&g, md(1), p(11)), Err(OrderRejection::DestinationNotPassable { province: p(11), controller: Some(C2) }));
}

// ---------------------------------------------------------------- 守備隊なし

#[test]
fn an_undefended_enemy_province_stops_the_division_issues_one_claim_and_discards_the_rest_of_the_path() {
    // 10(国家1) 11(国家2、守備隊なし) 12(国家2、守備隊なし)。10 → 12(2ホップ)を命じても、11 で止まる。
    let map = map_from(&["###"]);
    let (_w, g) = graph_with_states(&map, &[(10, 0, 0), (11, 1, 0), (12, 2, 0)], &[]);
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], control_of(&[(10, C1), (11, C2), (12, C2)]), vec![(C1, C2)]);
    assert_eq!(m.issue_move_order(&g, md(1), p(12)), Ok(OrderOutcome::Started { hops: 2 }));
    let mut all = Vec::new();
    for day in 1..=40 {
        let ev = m.advance_day(&g);
        if day == 5 {
            assert_eq!(
                ev.iter().filter(|e| matches!(e, MoveEvent::ControlClaim { .. })).cloned().collect::<Vec<_>>(),
                vec![MoveEvent::ControlClaim { province: p(11), claimant: C1, previous_controller: Some(C2), reason: ClaimReason::Undefended, divisions: vec![md(1)] }]
            );
        }
        all.extend(ev);
    }
    assert_eq!(claims(&all).len(), 1, "the claim is issued once; the same result is never repeated on later days");
    // M6-c: 実効支配は到着日に更新される(対象プロヴィンスだけ。12 は国家2のまま)。師団は対象プロヴィンスにいて Idle、残りの経路・目的地は破棄されている
    assert_eq!(m.control().controller(p(11)), Some(C1));
    assert_eq!(m.control().controller(p(12)), Some(C2));
    let d = m.division(md(1)).unwrap();
    assert_eq!((d.province, d.status, d.destination, d.target, d.path.is_empty(), d.progress, d.battle), (p(11), MapDivisionStatus::Idle, None, None, true, 0.0, None));
    assert!(m.battles().next().is_none(), "no battle is created without defenders");
    // 12 へは進まない(占領が成立したものとして奥へ自動進行しない)
    assert_ne!(d.province, p(12));
}

// ---------------------------------------------------------------- 戦闘の開始・参加者・合流

#[test]
fn a_battle_includes_only_the_divisions_in_the_same_province_even_inside_one_state() {
    // 星形。中心 T(idx0) と隣接 idx2 はどちらも国家2の支配で、同じステートに属する。国家1が idx1 から T へ攻める。
    let map = map_from(&[".##", "###", "##."]);
    let provs: Vec<(u32, i32, i32)> = STAR.iter().enumerate().map(|(i, (q, r))| (pid(i), *q, *r)).collect();
    let (_w, g) = graph_with_states(&map, &provs, &[(7, vec![pid(0), pid(2)])]);
    let ctl = star_control(&[C2, C1, C2, C1, C1, C1, C1]);
    let mut m = military(vec![div(1, C1, 1, CombatStats::standard()), div(2, C2, 0, CombatStats::standard()), div(3, C2, 2, CombatStats::standard())], ctl, vec![(C1, C2)]);
    m.issue_move_order(&g, md(1), p(pid(0))).unwrap();
    let mut started = None;
    for _ in 1..=5 {
        for e in m.advance_day(&g) {
            if let MoveEvent::BattleStarted { battle, province, attacker, defender, attackers, defenders } = e {
                started = Some((battle, province, attacker, defender, attackers, defenders));
            }
        }
    }
    let (b, prov, a, d, atk, def) = started.expect("a battle started on day 5");
    assert_eq!((b, prov, a, d, atk, def), (MapBattleId(0), p(pid(0)), C1, C2, vec![md(1)], vec![md(2)]), "division 3 is in another province of the same state and is not a participant");
    let d3 = m.division(md(3)).unwrap();
    assert_eq!((d3.province, d3.status, d3.battle), (p(pid(2)), MapDivisionStatus::Idle, None));
    check_invariants(&m);
}

#[test]
fn reinforcements_join_the_ongoing_battle_in_id_order_other_countries_stop_before_entering_and_nobody_is_registered_twice() {
    let (_m, w, g) = real();
    let mut m = load_division_scenario(BATTLE, &g, &w.model).unwrap().into_military();
    for (d, dest) in [(4u32, 18111u32), (7, 18111), (10, 18111)] {
        assert!(matches!(m.issue_move_order(&g, md(d), p(dest)), Ok(OrderOutcome::Started { .. })));
    }
    let mut log: Vec<(usize, MoveEvent)> = Vec::new();
    for day in 1..=12 {
        for e in m.advance_day(&g) {
            log.push((day, e));
        }
        check_invariants(&m);
    }
    let find = |f: &dyn Fn(&MoveEvent) -> bool| log.iter().find(|(_, e)| f(e)).map(|(d, e)| (*d, e.clone()));
    // 5日目: 師団4が攻撃を開始(防御側は 18111 にいる師団8だけ)
    assert_eq!(
        find(&|e| matches!(e, MoveEvent::BattleStarted { .. })),
        Some((5, MoveEvent::BattleStarted { battle: MapBattleId(0), province: p(18111), attacker: C1, defender: C2, attackers: vec![md(4)], defenders: vec![md(8)] }))
    );
    // 5日目: 国家3の師団10は、国家1が攻撃側の戦闘が進行中の 18111 へは進入前に止まる(合流できない)
    assert_eq!(
        find(&|e| matches!(e, MoveEvent::StoppedBefore { .. })),
        Some((5, MoveEvent::StoppedBefore { division: md(10), at: p(18312), blocked: p(18111), reason: StopReason::ContestedByOthers }))
    );
    let d10 = m.division(md(10)).unwrap();
    assert_eq!((d10.province, d10.status, d10.battle, d10.destination), (p(18312), MapDivisionStatus::Idle, None, None));
    // 10日目: 国家1の師団7(2ホップ)が攻撃側として合流(戦闘は11日目に終わる)
    assert_eq!(find(&|e| matches!(e, MoveEvent::BattleJoined { .. })), Some((10, MoveEvent::BattleJoined { battle: MapBattleId(0), division: md(7), province: p(18111) })));
    // 合流後の参加者は昇順で、重複しない(戦闘の終了後は除去済み)
    let finish = find(&|e| matches!(e, MoveEvent::BattleFinished { .. })).expect("the battle ended");
    assert_eq!(finish.0, 11, "the battle ends one day after the reinforcement joined");
    assert!(matches!(finish.1, MoveEvent::BattleFinished { outcome: BattleOutcome::AttackerWon, reason: FinishReason::DefenderGone, .. }));
    assert!(m.battles().next().is_none());
}

#[test]
fn fighting_divisions_take_no_orders_and_ordinary_idempotency_and_rejection_safety_hold_with_wars() {
    let (_m, w, g) = real();
    let mut m = load_division_scenario(BATTLE, &g, &w.model).unwrap().into_military();
    m.issue_move_order(&g, md(4), p(18111)).unwrap();
    // 通常移動の冪等性: 移動中の同じ目的地は no-op(状態が変わらない)、拒否は状態を保つ
    m.advance_day(&g);
    m.advance_day(&g);
    let mid = m.clone();
    assert_eq!(m.issue_move_order(&g, md(4), p(18111)), Ok(OrderOutcome::AlreadyMoving));
    assert_eq!(m.issue_move_order(&g, md(4), p(18312)), Err(OrderRejection::DestinationNotPassable { province: p(18312), controller: Some(C3) }));
    assert_eq!(m.issue_move_order(&g, md(4), p(99999)), Err(OrderRejection::UnknownProvince(p(99999))));
    assert_eq!(m, mid);
    for _ in 0..3 {
        m.advance_day(&g);
    }
    // 5日目: 戦闘中。攻撃側も防御側も、現在地への命令を含めてすべて拒否され、状態は変わらない
    assert_eq!(m.division(md(4)).unwrap().status, MapDivisionStatus::Fighting);
    let fighting = m.clone();
    for (d, dest) in [(4u32, 18111u32), (4, 18311), (4, 18310), (8, 18111), (8, 17904)] {
        assert_eq!(m.issue_move_order(&g, md(d), p(dest)), Err(OrderRejection::DivisionInCombat(md(d))), "division {d} -> {dest}");
    }
    assert_eq!(m, fighting);
    // 戦闘が終わったあとは、また命令を受け付ける(増援のない単独の攻撃は長引くので、終わるまで進める)
    let mut days = 0;
    while m.battles().next().is_some() {
        m.advance_day(&g);
        check_invariants(&m);
        days += 1;
        assert!(days < 100, "the battle must end");
    }
    assert_eq!(m.division(md(4)).unwrap().status, MapDivisionStatus::Idle);
    assert!(matches!(m.issue_move_order(&g, md(4), p(18310)), Ok(OrderOutcome::Started { .. })));
}

// ---------------------------------------------------------------- 勝敗・退却・結果(実データのシナリオ)

#[test]
fn attacker_victory_issues_one_claim_with_the_full_result_and_leaves_control_unchanged() {
    let (_m, w, g) = real();
    let mut m = load_division_scenario(BATTLE, &g, &w.model).unwrap().into_military();
    m.issue_move_order(&g, md(2), p(17694)).unwrap();
    let mut all = Vec::new();
    for _ in 1..=20 {
        all.extend(m.advance_day(&g));
        check_invariants(&m);
    }
    // 結果は1回だけ。対象 ProvinceId・攻撃国・直前の支配国・理由(戦闘ID)・関わった師団を含む
    assert_eq!(
        claims(&all).into_iter().cloned().collect::<Vec<_>>(),
        vec![MoveEvent::ControlClaim { province: p(17694), claimant: C1, previous_controller: Some(C2), reason: ClaimReason::AttackerWonBattle(MapBattleId(0)), divisions: vec![md(2)] }]
    );
    // 防御側は退路(自国支配の隣接で最小の ProvinceId = 17486)へ退却した
    assert!(all.contains(&MoveEvent::Retreated { division: md(5), battle: MapBattleId(0), from: p(17694), to: p(17486) }));
    // 支配が更新される前の状態: 師団は対象プロヴィンスにいて Idle、命令なし。支配は変わっていない。戦闘は除去済み
    let d = m.division(md(2)).unwrap();
    assert_eq!((d.province, d.status, d.destination, d.target, d.path.is_empty(), d.battle), (p(17694), MapDivisionStatus::Idle, None, None, true, None));
    assert_eq!(m.control().controller(p(17694)), Some(C1), "M6-c: the victory changes the control of that province on the finishing day");
    assert!(m.battles().next().is_none());
    let d5 = m.division(md(5)).unwrap();
    assert_eq!((d5.province, d5.status, d5.battle), (p(17486), MapDivisionStatus::Idle, None));
}

#[test]
fn defender_victory_sends_the_attacker_back_to_its_origin_and_issues_no_claim() {
    let (_m, w, g) = real();
    let mut m = load_division_scenario(BATTLE, &g, &w.model).unwrap().into_military();
    m.issue_move_order(&g, md(3), p(17269)).unwrap();
    let mut all = Vec::new();
    for _ in 1..=12 {
        all.extend(m.advance_day(&g));
    }
    assert!(all.contains(&MoveEvent::Retreated { division: md(3), battle: MapBattleId(0), from: p(17269), to: p(17268) }), "origin 17268 is the attacker's retreat");
    assert!(all.contains(&MoveEvent::BattleFinished {
        battle: MapBattleId(0),
        province: p(17269),
        attacker: C1,
        defender: C2,
        outcome: BattleOutcome::DefenderWon,
        reason: FinishReason::AttackerGone
    }));
    assert!(claims(&all).is_empty());
    assert_eq!(m.division(md(3)).unwrap().province, p(17268));
    assert_eq!(m.division(md(6)).unwrap().province, p(17269));
    assert_eq!(m.control().controller(p(17269)), Some(C2));
}

#[test]
fn a_defender_with_no_retreat_is_destroyed_and_the_attacker_wins() {
    let (_m, w, g) = real();
    let mut m = load_division_scenario(BATTLE, &g, &w.model).unwrap().into_military();
    m.issue_move_order(&g, md(11), p(17906)).unwrap();
    let mut all = Vec::new();
    for _ in 1..=6 {
        all.extend(m.advance_day(&g));
    }
    assert!(all.contains(&MoveEvent::Destroyed { division: md(12), battle: MapBattleId(0), at: p(17906) }));
    assert!(m.division(md(12)).is_none(), "the destroyed division is removed");
    assert_eq!(claims(&all).len(), 1);
}

// ---------------------------------------------------------------- 退路の規則(星形の小さな地図)

/// 星形で、攻撃側(国家1)が idx1 から中心(idx0, 国家2)へ攻める。`attacker`・`defender` は戦闘能力。
fn star_battle(controls: [CountryId; 7], attacker: CombatStats, defender: CombatStats) -> (MapMilitary, ProvinceGraph) {
    let g = star_graph();
    let mut m = military(vec![div(1, C1, 1, attacker), div(2, C2, 0, defender)], star_control(&controls), vec![(C1, C2)]);
    m.issue_move_order(&g, md(1), p(pid(0))).unwrap();
    (m, g)
}

fn run_days(m: &mut MapMilitary, g: &ProvinceGraph, days: usize) -> Vec<MoveEvent> {
    let mut all = Vec::new();
    for _ in 0..days {
        all.extend(m.advance_day(g));
        check_invariants(m);
    }
    all
}

#[test]
fn the_defender_retreats_to_the_lowest_province_id_it_controls() {
    // 中心の隣接: idx1(国家1) idx2(国家1) idx3(国家2) idx4(国家2) idx5(国家1) idx6(国家2)。国家2の最小は idx3。
    let (mut m, g) = star_battle([C2, C1, C1, C2, C2, C1, C2], CombatStats::standard(), weak());
    let ev = run_days(&mut m, &g, 6);
    assert!(ev.contains(&MoveEvent::Retreated { division: md(2), battle: MapBattleId(0), from: p(pid(0)), to: p(pid(3)) }));
    assert_eq!(m.division(md(2)).unwrap().province, p(pid(3)));
}

#[test]
fn a_retreat_candidate_with_an_ongoing_battle_is_skipped_and_a_conquest_earlier_in_id_order_is_seen_by_later_retreat_decisions() {
    // 師団1(idx1)が中心 idx0 を、師団3(idx2)が idx3 を、同じ日に攻める(どちらも国家2の弱い守備)。先に始まった戦闘0(中心)の防御側は、
    // 最小の候補 idx3 が戦闘中(戦闘1)なので飛ばし、次の idx4 へ退却する。
    let g = star_graph();
    let ctl = star_control(&[C2, C1, C1, C2, C2, C1, C2]);
    let mut m = military(vec![div(1, C1, 1, CombatStats::standard()), div(2, C2, 0, weak()), div(3, C1, 2, CombatStats::standard()), div(4, C2, 3, weak())], ctl, vec![(C1, C2)]);
    m.issue_move_order(&g, md(1), p(pid(0))).unwrap();
    m.issue_move_order(&g, md(3), p(pid(3))).unwrap();
    let ev = run_days(&mut m, &g, 5);
    let started: Vec<(MapBattleId, ProvinceId)> = ev.iter().filter_map(|e| if let MoveEvent::BattleStarted { battle, province, .. } = e { Some((*battle, *province)) } else { None }).collect();
    assert_eq!(started, vec![(MapBattleId(0), p(pid(0))), (MapBattleId(1), p(pid(3)))], "battle ids follow the division id order of arrival");
    let retreats: Vec<(MapDivisionId, ProvinceId)> = ev.iter().filter_map(|e| if let MoveEvent::Retreated { division, to, .. } = e { Some((*division, *to)) } else { None }).collect();
    // 戦闘0(先): 候補 idx3 は戦闘1が進行中なので飛ばして idx4。戦闘1(後): 戦闘0は既に終了し、**M6-c で中心 idx0 の支配は国家1に変わっている**ので候補にならず、
    // 最小の候補は idx4(M6-b では支配が未更新で、中心 idx0 へ退却した。占領後の支配国が同日の後続の退路判定から見える)。
    assert_eq!(retreats, vec![(md(2), p(pid(4))), (md(4), p(pid(4)))]);
}

#[test]
fn the_attacker_retreats_to_its_origin_then_to_the_lowest_controlled_neighbour_then_is_destroyed() {
    let controls = [C2, C1, C1, C2, C2, C2, C2];
    // 出撃元(idx1)が有効 → 出撃元へ
    let (mut m, g) = star_battle(controls, weak(), CombatStats::standard());
    let ev = run_days(&mut m, &g, 6);
    assert!(ev.contains(&MoveEvent::Retreated { division: md(1), battle: MapBattleId(0), from: p(pid(0)), to: p(pid(1)) }));
    // 出撃元が敵に取られた → 戦闘地点の隣接で自国支配かつ戦闘中でない最小(idx2)
    let (mut m, g) = star_battle(controls, weak(), CombatStats::standard());
    m.set_controller(p(pid(1)), Some(C2));
    let ev = run_days(&mut m, &g, 6);
    assert!(ev.contains(&MoveEvent::Retreated { division: md(1), battle: MapBattleId(0), from: p(pid(0)), to: p(pid(2)) }));
    // 代替先もない → 撃破される
    let (mut m, g) = star_battle(controls, weak(), CombatStats::standard());
    m.set_controller(p(pid(1)), Some(C2));
    m.set_controller(p(pid(2)), Some(C2));
    let ev = run_days(&mut m, &g, 6);
    assert!(ev.contains(&MoveEvent::Destroyed { division: md(1), battle: MapBattleId(0), at: p(pid(0)) }));
    assert!(m.division(md(1)).is_none());
    assert!(claims(&ev).is_empty());
}

#[test]
fn a_day_with_no_damage_on_either_side_is_a_defender_win_and_the_attacker_goes_back() {
    let (mut m, g) = star_battle([C2, C1, C1, C2, C2, C2, C2], inert(), inert());
    let ev = run_days(&mut m, &g, 6);
    assert!(ev.contains(&MoveEvent::BattleFinished { battle: MapBattleId(0), province: p(pid(0)), attacker: C1, defender: C2, outcome: BattleOutcome::DefenderWon, reason: FinishReason::ZeroDamage }));
    assert!(ev.contains(&MoveEvent::Retreated { division: md(1), battle: MapBattleId(0), from: p(pid(0)), to: p(pid(1)) }));
    assert!(claims(&ev).is_empty());
    let d2 = m.division(md(2)).unwrap();
    assert_eq!((d2.province, d2.status, d2.battle), (p(pid(0)), MapDivisionStatus::Idle, None));
}

// ---------------------------------------------------------------- 決定性

#[test]
fn registration_order_and_order_issue_order_do_not_change_events_or_final_state() {
    let (_m, w, g) = real();
    let run = |reverse: bool| {
        let mut sc = load_division_scenario(BATTLE, &g, &w.model).unwrap();
        if reverse {
            sc.divisions.reverse();
        }
        let mut m = sc.into_military();
        let mut orders: Vec<(u32, u32)> = vec![(1, 18502), (2, 17694), (3, 17269), (4, 18111), (7, 18111), (10, 18111), (11, 17906)];
        if reverse {
            orders.reverse();
        }
        for (d, dest) in orders {
            let _ = m.issue_move_order(&g, md(d), p(dest));
        }
        let mut ev = Vec::new();
        for _ in 0..40 {
            ev.push(m.advance_day(&g));
            check_invariants(&m);
        }
        (ev, m)
    };
    let (ea, ma) = run(false);
    let (eb, mb) = run(true);
    assert_eq!(ea, eb);
    assert_eq!(ma, mb);
    // 戦闘は複数あり、ID昇順で進む(同じ日に始まる戦闘の ID は師団ID昇順の到着順)
    let started: Vec<MapBattleId> = ea.iter().flatten().filter_map(|e| if let MoveEvent::BattleStarted { battle, .. } = e { Some(*battle) } else { None }).collect();
    assert!(started.len() >= 4, "several battles took place: {started:?}");
    let mut sorted = started.clone();
    sorted.sort();
    assert_eq!(started, sorted);
}

// ---------------------------------------------------------------- 旧経路との比較(M6-a で専用テストがなかった「移動中に現在地へ命令」を含む)

/// 比較用ハーネスの1ケース。旧 `StateId i` ↔ 新 `ProvinceId 100+10i`。国家は 1 と 2(交戦)。
struct Case {
    cells: Vec<(i32, i32)>,
    controllers: Vec<usize>,
    /// (師団ID, 国家, 位置 idx, 戦闘能力)
    divisions: Vec<(u32, usize, usize, CombatStats)>,
    /// (日, 師団ID, 目的地 idx)
    orders: Vec<(usize, u32, usize)>,
    /// (日, idx, 国家)
    control_changes: Vec<(usize, usize, usize)>,
    days: usize,
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

#[derive(Debug, Clone, PartialEq)]
struct DivSnap {
    id: u32,
    idx: usize,
    status: u8,
    manpower: u64,
    organization: u32,
    equipment: u64,
    destination: Option<usize>,
    target: Option<usize>,
    path: Vec<usize>,
    progress: u32,
}

#[derive(Debug, Clone, PartialEq)]
struct Snap {
    divisions: Vec<DivSnap>,
    /// 進行中の戦闘: (位置 idx, 攻撃側, 防御側)
    battles: Vec<(usize, Vec<u32>, Vec<u32>)>,
    controllers: Vec<usize>,
}

fn war(id: usize) -> War {
    War {
        id: WarId(id),
        name: "t".into(),
        attackers: [C1].into_iter().collect(),
        defenders: [C2].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1800/01/01".into(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: Default::default(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: Default::default(),
    }
}

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

fn old_app(case: &Case, adj: &[Vec<usize>]) -> App {
    let mut app = App::new();
    app.add_message::<DayChangedMessage>();
    app.insert_resource(CountryRegistry::default());
    let states: Vec<StateData> = (0..case.cells.len())
        .map(|i| StateData { id: StateId(i), owner_country_id: CountryId(case.controllers[i]), neighbors: adj[i].iter().map(|j| StateId(*j)).collect(), ..Default::default() })
        .collect();
    app.insert_resource(StateRegistry::build(states));
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
    for (id, owner, idx, c) in &case.divisions {
        mil.divisions.insert(DivisionId(*id as usize), old_division(*id, *owner, *idx, c));
    }
    app.insert_resource(mil);
    app.insert_resource(BattleRegistry::default());
    let mut wars = WarRegistry::default();
    wars.wars.insert(WarId(0), war(0));
    app.insert_resource(wars);
    app.add_systems(Update, handle_daily_military);
    app
}

fn old_day(app: &mut App, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(DayChangedMessage { year: 1800, month: 1, day });
    state.apply(app.world_mut());
    app.update();
}

fn old_snapshot(app: &App, n: usize) -> Snap {
    let mil = app.world().resource::<MilitaryRegistry>();
    let states = app.world().resource::<StateRegistry>();
    let battles = app.world().resource::<BattleRegistry>();
    let mut divisions: Vec<DivSnap> = mil
        .divisions
        .values()
        .map(|d| DivSnap {
            id: d.id.0 as u32,
            idx: d.current_state.0,
            status: match d.status {
                DivisionStatus::Idle => 0,
                DivisionStatus::Moving => 1,
                DivisionStatus::Fighting => 2,
                other => panic!("{other:?}"),
            },
            manpower: d.manpower,
            organization: d.organization.to_bits(),
            equipment: d.equipment.to_bits(),
            destination: d.destination.map(|s| s.0),
            target: d.target_state.map(|s| s.0),
            path: d.current_path.iter().map(|s| s.0).collect(),
            progress: d.movement_progress.to_bits(),
        })
        .collect();
    divisions.sort_by_key(|d| d.id);
    let mut bs: Vec<(usize, Vec<u32>, Vec<u32>)> = battles
        .battles
        .values()
        .filter(|b| b.status == strategy_game::military::battle::BattleStatus::Ongoing)
        .map(|b| (b.state_id.0, b.attacker_division_ids.iter().map(|d| d.0 as u32).collect(), b.defender_division_ids.iter().map(|d| d.0 as u32).collect()))
        .collect();
    bs.sort();
    Snap { divisions, battles: bs, controllers: (0..n).map(|i| states.get(StateId(i)).unwrap().controller().0).collect() }
}

fn new_snapshot(m: &MapMilitary, n: usize) -> Snap {
    let idx_of = |pr: ProvinceId| ((pr.0 - 100) / 10) as usize;
    let divisions: Vec<DivSnap> = m
        .divisions()
        .map(|d| DivSnap {
            id: d.id.0,
            idx: idx_of(d.province),
            status: match d.status {
                MapDivisionStatus::Idle => 0,
                MapDivisionStatus::Moving => 1,
                MapDivisionStatus::Fighting => 2,
            },
            manpower: d.combat.manpower,
            organization: d.combat.organization.to_bits(),
            equipment: d.combat.equipment.to_bits(),
            destination: d.destination.map(idx_of),
            target: d.target.map(idx_of),
            path: d.path.iter().map(|x| idx_of(*x)).collect(),
            progress: d.progress.to_bits(),
        })
        .collect();
    let mut bs: Vec<(usize, Vec<u32>, Vec<u32>)> = m
        .battles()
        .filter(|b| b.status == MapBattleStatus::Ongoing)
        .map(|b| (idx_of(b.province), b.attackers.iter().map(|d| d.0).collect(), b.defenders.iter().map(|d| d.0).collect()))
        .collect();
    bs.sort();
    Snap { divisions, battles: bs, controllers: (0..n).map(|i| m.control().controller(p(pid(i))).unwrap().0).collect() }
}

/// 旧と新を同じ条件で動かし、各日の状態を比べる。違いの一覧と、新側の出来事の記録を返す。
fn compare(case: &Case) -> (Vec<String>, Vec<MoveEvent>) {
    let n = case.cells.len();
    let adj = adjacency(&case.cells);
    let mut old = old_app(case, &adj);
    // 新: 同じ地図(座標から作る)
    let (max_q, max_r) = (case.cells.iter().map(|c| c.0).max().unwrap(), case.cells.iter().map(|c| c.1).max().unwrap());
    let rows: Vec<String> = (0..=max_r).map(|r| (0..=max_q).map(|q| if case.cells.contains(&(q, r)) { '#' } else { '.' }).collect()).collect();
    let row_refs: Vec<&str> = rows.iter().map(|s| s.as_str()).collect();
    let map = map_from(&row_refs);
    let provs: Vec<(u32, i32, i32)> = case.cells.iter().enumerate().map(|(i, (q, r))| (pid(i), *q, *r)).collect();
    let (_w, g) = graph_with_states(&map, &provs, &[]);
    let ctl = control_of(&case.controllers.iter().enumerate().map(|(i, c)| (pid(i), CountryId(*c))).collect::<Vec<_>>());
    let divs: Vec<MapDivision> = case.divisions.iter().map(|(id, o, i, c)| div(*id, CountryId(*o), *i, *c)).collect();
    let mut new = military(divs, ctl, vec![(C1, C2)]);

    let mut diffs = Vec::new();
    let mut events = Vec::new();
    let s0 = (old_snapshot(&old, n), new_snapshot(&new, n));
    if s0.0 != s0.1 {
        diffs.push(format!("day 0: {:?} != {:?}", s0.0, s0.1));
    }
    for day in 1..=case.days {
        for (_, idx, c) in case.control_changes.iter().filter(|(d, _, _)| *d == day) {
            old.world_mut().resource_mut::<StateRegistry>().get_mut(StateId(*idx)).unwrap().controller_country = Some(CountryId(*c));
            new.set_controller(p(pid(*idx)), Some(CountryId(*c)));
        }
        for (_, id, dest) in case.orders.iter().filter(|(d, _, _)| *d == day) {
            let owner = case.divisions.iter().find(|d| d.0 == *id).unwrap().1;
            let other = if owner == 1 { 2 } else { 1 };
            let old_ok = old.world_mut().resource_scope::<MilitaryRegistry, bool>(|world, mut mil| {
                let states = world.resource::<StateRegistry>();
                let d = mil.divisions.get_mut(&DivisionId(*id as usize)).unwrap();
                start_or_maintain_movement(d, StateId(*dest), states, &[CountryId(owner)], &[CountryId(other)])
            });
            let new_ok = new.issue_move_order(&g, md(*id), p(pid(*dest))).is_ok();
            if old_ok != new_ok {
                diffs.push(format!("day {day}: order {id}->{dest}: old accepted={old_ok}, new accepted={new_ok}"));
            }
        }
        old_day(&mut old, day as u8);
        let ev = new.advance_day(&g);
        check_invariants(&new);
        // M6-c: 占領(実効支配の更新)は、新経路も旧と同じく、到着処理・終了処理の中でその場で行う(当日の組織率回復より前)。
        // M6-b のハーネスにあった「要求を新側へ適用する」補正と「占領の日で比較を止める」処理は不要になった。
        events.extend(ev);
        let (so, sn) = (old_snapshot(&old, n), new_snapshot(&new, n));
        if so != sn {
            diffs.push(format!("day {day}:\n  old {so:?}\n  new {sn:?}"));
        }
    }
    (diffs, events)
}

fn line(n: usize) -> Vec<(i32, i32)> {
    (0..n as i32).map(|i| (i, 0)).collect()
}

fn case(cells: Vec<(i32, i32)>, controllers: Vec<usize>, divisions: Vec<(u32, usize, usize, CombatStats)>, orders: Vec<(usize, u32, usize)>, days: usize) -> Case {
    Case { cells, controllers, divisions, orders, control_changes: vec![], days }
}

#[test]
fn old_comparison_attacker_wins_the_defender_retreats_and_the_claim_matches_the_old_occupation() {
    let c = case(line(5), vec![1, 1, 2, 2, 2], vec![(1, 1, 1, CombatStats::standard()), (2, 2, 2, weak())], vec![(1, 1, 2)], 18);
    let (diffs, ev) = compare(&c);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert!(ev.contains(&MoveEvent::Retreated { division: md(2), battle: MapBattleId(0), from: p(pid(2)), to: p(pid(3)) }));
    assert_eq!(claims(&ev).len(), 1, "the old model also changed the controller exactly once (the harness applied the claim; controllers matched)");
}

#[test]
fn old_comparison_defender_wins_and_the_attacker_retreats_to_its_origin_and_recovers() {
    let c = case(line(5), vec![1, 1, 2, 2, 2], vec![(1, 1, 1, weak()), (2, 2, 2, CombatStats::standard())], vec![(1, 1, 2)], 16);
    let (diffs, ev) = compare(&c);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert!(ev.contains(&MoveEvent::Retreated { division: md(1), battle: MapBattleId(0), from: p(pid(2)), to: p(pid(1)) }));
    assert!(claims(&ev).is_empty());
}

#[test]
fn old_comparison_a_defender_without_a_retreat_is_destroyed() {
    // 国家2の守備(idx2)は、隣接 idx1・idx3 がどちらも国家1の支配で、退路がない
    let c = case(line(5), vec![1, 1, 2, 1, 1], vec![(1, 1, 1, CombatStats::standard()), (2, 2, 2, weak())], vec![(1, 1, 2)], 16);
    let (diffs, ev) = compare(&c);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert!(ev.contains(&MoveEvent::Destroyed { division: md(2), battle: MapBattleId(0), at: p(pid(2)) }));
}

#[test]
fn old_comparison_the_attacker_origin_is_lost_and_there_is_no_other_retreat_so_it_is_destroyed() {
    // 出撃元 idx1 が戦闘の最中に国家2へ移る。戦闘地点 idx2 の隣接はどちらも国家2 → 退路なし
    let mut c = case(line(5), vec![1, 1, 2, 2, 2], vec![(1, 1, 1, weak()), (2, 2, 2, CombatStats::standard())], vec![(1, 1, 2)], 10);
    c.control_changes = vec![(2, 1, 2)];
    let (diffs, ev) = compare(&c);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert!(ev.contains(&MoveEvent::Destroyed { division: md(1), battle: MapBattleId(0), at: p(pid(2)) }));
}

#[test]
fn old_comparison_the_attacker_origin_is_lost_and_the_lowest_controlled_neighbour_is_used() {
    // 星形: 中心 idx0 は国家2、攻撃側は idx1 から。idx1 を戦闘中に国家2へ → 代替は国家1の支配で最小の idx2
    let mut c = case(STAR.to_vec(), vec![2, 1, 1, 2, 2, 2, 2], vec![(1, 1, 1, weak()), (2, 2, 0, CombatStats::standard())], vec![(1, 1, 0)], 10);
    c.control_changes = vec![(2, 1, 2)];
    let (diffs, ev) = compare(&c);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert!(ev.contains(&MoveEvent::Retreated { division: md(1), battle: MapBattleId(0), from: p(pid(0)), to: p(pid(2)) }));
}

#[test]
fn old_comparison_a_reinforcement_joins_the_ongoing_battle_as_an_attacker() {
    // 国家1: 師団1(idx2)が idx3 を攻め、師団3(idx1)が2ホップで後から合流。守備: 師団2(国家2、idx3)
    let c = case(line(5), vec![1, 1, 1, 2, 2], vec![(1, 1, 2, CombatStats::standard()), (2, 2, 3, CombatStats::standard()), (3, 1, 1, CombatStats::standard())], vec![(1, 1, 3), (1, 3, 3)], 20);
    let (diffs, ev) = compare(&c);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert!(ev.iter().any(|e| matches!(e, MoveEvent::BattleJoined { division, .. } if *division == md(3))));
}

#[test]
fn old_comparison_zero_damage_on_both_sides_sends_the_attacker_back() {
    let c = case(line(5), vec![1, 1, 2, 2, 2], vec![(1, 1, 1, inert()), (2, 2, 2, inert())], vec![(1, 1, 2)], 10);
    let (diffs, ev) = compare(&c);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert!(ev.iter().any(|e| matches!(e, MoveEvent::BattleFinished { reason: FinishReason::ZeroDamage, .. })));
}

#[test]
fn old_comparison_ordinary_movement_and_a_moving_division_ordered_to_its_current_province() {
    // 直線を自国領内で移動。3日目に、移動中の師団へ現在地(idx0)への命令: 旧も新も、空の経路で Moving にして現在のホップを取り消し、翌日 Idle になる。
    let c = case(line(5), vec![1, 1, 1, 1, 1], vec![(1, 1, 0, CombatStats::standard())], vec![(1, 1, 3), (3, 1, 0)], 12);
    let (diffs, _) = compare(&c);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    // 新の状態を直接確認: 命令の直後は destination=現在地・経路なし・進捗 0 のまま Moving、翌日の日次処理で Idle・目的地なし
    let map = map_from(&["#####"]);
    let provs: Vec<(u32, i32, i32)> = (0..5).map(|i| (pid(i), i as i32, 0)).collect();
    let (_w, g) = graph_with_states(&map, &provs, &[]);
    let mut m = military(vec![div(1, C1, 0, CombatStats::standard())], control_of(&(0..5).map(|i| (pid(i), C1)).collect::<Vec<_>>()), vec![(C1, C2)]);
    m.issue_move_order(&g, md(1), p(pid(3))).unwrap();
    m.advance_day(&g);
    m.advance_day(&g);
    assert!(m.division(md(1)).unwrap().progress > 0.0);
    assert_eq!(m.issue_move_order(&g, md(1), p(pid(0))), Ok(OrderOutcome::Started { hops: 0 }));
    let d = m.division(md(1)).unwrap();
    assert_eq!((d.destination, d.path.is_empty(), d.target, d.progress, d.status, d.province), (Some(p(pid(0))), true, None, 0.0, MapDivisionStatus::Moving, p(pid(0))));
    m.advance_day(&g);
    let d = m.division(md(1)).unwrap();
    assert_eq!((d.destination, d.status, d.province), (None, MapDivisionStatus::Idle, p(pid(0))));
}

#[test]
fn old_comparison_known_differences_undefended_entry_and_a_third_country() {
    // 守備隊なし: 旧も新も到着日(5日目)に支配国が変わる(M6-c)。残る差: 旧は師団を翌日まで Moving のままにし、新は到着日に Idle にする。
    let c = case(line(3), vec![1, 2, 2], vec![(1, 1, 0, CombatStats::standard())], vec![(1, 1, 1)], 5);
    let adj = adjacency(&c.cells);
    let mut old = old_app(&c, &adj);
    let mut old_status_by_day = Vec::new();
    for day in 1..=6u8 {
        if day == 1 {
            old.world_mut().resource_scope::<MilitaryRegistry, ()>(|world, mut mil| {
                let states = world.resource::<StateRegistry>();
                start_or_maintain_movement(mil.divisions.get_mut(&DivisionId(1)).unwrap(), StateId(1), states, &[C1], &[C2]);
            });
        }
        old_day(&mut old, day);
        let s = old_snapshot(&old, 3);
        old_status_by_day.push((s.controllers.clone(), s.divisions[0].idx, s.divisions[0].status));
    }
    // 旧: 5日目の到着で即座に占領(idx1 の支配国が国家1へ)。師団は5日目は Moving のままで、6日目に Idle になる。
    assert_eq!(old_status_by_day[3], (vec![1, 2, 2], 0, 1), "old day 4: still moving, nothing changed");
    assert_eq!(old_status_by_day[4], (vec![1, 1, 2], 1, 1), "old day 5: the controller of idx1 changed on arrival; the division is still Moving");
    assert_eq!(old_status_by_day[5], (vec![1, 1, 2], 1, 0), "old day 6: the division becomes Idle");
    // 新: 到着日(5日目)に Idle・ControlClaim。支配は未更新
    let map = map_from(&["###"]);
    let (_w, g) = graph_with_states(&map, &[(pid(0), 0, 0), (pid(1), 1, 0), (pid(2), 2, 0)], &[]);
    let mut m = military(vec![div(1, C1, 0, CombatStats::standard())], control_of(&[(pid(0), C1), (pid(1), C2), (pid(2), C2)]), vec![(C1, C2)]);
    m.issue_move_order(&g, md(1), p(pid(1))).unwrap();
    let mut ev = Vec::new();
    for _ in 0..5 {
        ev.extend(m.advance_day(&g));
    }
    assert_eq!(claims(&ev).len(), 1);
    assert_eq!(m.control().controller(p(pid(1))), Some(C1), "new (M6-c): the control also changes on the arrival day (day 5), the same day as the old model");
    assert_eq!(m.division(md(1)).unwrap().status, MapDivisionStatus::Idle, "new: Idle on the arrival day (the old model keeps it Moving for one more day)");
}

// ---------------------------------------------------------------- 分離の確認

#[test]
fn the_battle_code_depends_on_the_old_model_only_through_the_isolated_adapter() {
    let strip = |path: &str| -> String {
        let src = std::fs::read_to_string(path).unwrap();
        src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
    };
    for path in ["src/world/movement.rs", "src/world/battle.rs", "src/world/division_scenario.rs"] {
        let code = strip(path);
        for banned in ["military::", "war::", "MapEconomy", "economy::", "snapshot", "SaveLocation", "StateRegistry", "WarRegistry", "std::time", "Instant", "rand::", "fs::write"] {
            assert!(!code.contains(banned), "{path}: found {banned}");
        }
        assert!(!code.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == "StateId"), "{path}: must not use the old StateId");
    }
    let adapter = strip("src/world/legacy_combat.rs");
    assert!(adapter.contains("military::combat_calc") && adapter.contains("military::equipment"));
    for banned in ["MilitaryRegistry::", "BattleRegistry", "WarRegistry", "StateRegistry", "military::battle", "military::update", "military::movement", "military::invasion", "std::fs", "std::time"] {
        // `MilitaryRegistry::default()` は補給充足率を 1.0 にするための空のレジストリで、旧の状態は一切渡さない
        if banned == "MilitaryRegistry::" {
            assert_eq!(adapter.matches("MilitaryRegistry::").count(), adapter.matches("MilitaryRegistry::default()").count(), "only an empty registry may be created");
            continue;
        }
        assert!(!adapter.contains(banned), "legacy_combat.rs: found {banned}");
    }
}
