//! P32-036 (M6-c): プロヴィンス単位の即時占領と実効支配の更新(`world::occupation`)の検証。
//!
//! D8: 旧ゲーム本体の挙動 = 即時占領を採用(+5/日の占領進捗は将来の仕様判断)。守備隊のいない到着は到着処理の中で、戦闘の勝利は終了処理の中で、
//! その場で対象プロヴィンスの実効支配を変える(当日の組織率回復より前)。法的所有(ステート)には触れない。旧の日次処理との各日の比較(支配変更日・組織率を含む)は
//! `tests/p32_035_map_battle_test.rs` の `old_comparison_*`(M6-b の補正を外して、占領後も全項目を比べる)で行う。時間は合否条件にしない。保存・アセットは書かない。

use std::collections::BTreeMap;

use serde_json::json;
use strategy_game::common::CountryId;
use strategy_game::hexmap::data::{HexMap, load_from_path, parse_hex_map};
use strategy_game::world::fingerprint::FINGERPRINT_ALGORITHM;
use strategy_game::world::legacy_combat;
use strategy_game::world::*;

const C1: CountryId = CountryId(1);
const C2: CountryId = CountryId(2);
const C3: CountryId = CountryId(3);

const BATTLE: &str = "assets/data/world/preview/division_battle_scenario.ron";

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

fn military(divs: Vec<MapDivision>, control: ProvinceControl, wars: Vec<(CountryId, CountryId)>) -> MapMilitary {
    MapMilitary::new(divs, control, MovementParams { base_days: 5.0 }).unwrap().with_wars(wars).unwrap()
}

fn weak() -> CombatStats {
    CombatStats { manpower: 2500, organization: 20.0, ..CombatStats::standard() }
}

/// 1行 n 陸: ID = 10 + q。
fn line_graph(n: usize) -> ProvinceGraph {
    let map = map_from(&[&"#".repeat(n)]);
    let provs: Vec<(u32, i32, i32)> = (0..n).map(|q| (10 + q as u32, q as i32, 0)).collect();
    graph_of(&map, &provs)
}

const STAR: [(i32, i32); 7] = [(1, 1), (2, 1), (2, 0), (1, 0), (0, 1), (0, 2), (1, 2)];

fn pid(idx: usize) -> u32 {
    100 + 10 * idx as u32
}

fn star_graph() -> ProvinceGraph {
    let map = map_from(&[".##", "###", "##."]);
    let provs: Vec<(u32, i32, i32)> = STAR.iter().enumerate().map(|(i, (q, r))| (pid(i), *q, *r)).collect();
    graph_of(&map, &provs)
}

fn star_control(c: &[CountryId; 7]) -> ProvinceControl {
    control_of(&c.iter().enumerate().map(|(i, c)| (pid(i), *c)).collect::<Vec<_>>())
}

fn run_days(m: &mut MapMilitary, g: &ProvinceGraph, days: usize) -> Vec<(usize, MoveEvent)> {
    let mut all = Vec::new();
    for day in 1..=days {
        for e in m.advance_day(g) {
            all.push((day, e));
        }
    }
    all
}

fn real() -> (WorldPreviewData, ProvinceGraph) {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let w = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).unwrap();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    (w, g)
}

fn real_military(g: &ProvinceGraph, w: &WorldPreviewData) -> MapMilitary {
    load_division_scenario(BATTLE, g, &w.model).unwrap().into_military()
}

fn count(ev: &[(usize, MoveEvent)], f: &dyn Fn(&MoveEvent) -> bool) -> usize {
    ev.iter().filter(|(_, e)| f(e)).count()
}

fn is_claim(e: &MoveEvent) -> bool {
    matches!(e, MoveEvent::ControlClaim { .. })
}

fn is_changed(e: &MoveEvent) -> bool {
    matches!(e, MoveEvent::ControlChanged { .. })
}

fn is_not_applied(e: &MoveEvent) -> bool {
    matches!(e, MoveEvent::ClaimNotApplied { .. })
}

// ---------------------------------------------------------------- 守備隊なし: 到着日に支配が変わり、同日に組織率が回復し、翌日以降に重複しない

#[test]
fn an_undefended_arrival_changes_control_on_the_arrival_day_and_the_organization_recovers_that_same_day() {
    // 10(国家1) 11(国家2、守備隊なし)。組織率 50 の師団が 10 → 11。
    let g = line_graph(3);
    let hurt = CombatStats { organization: 50.0, ..CombatStats::standard() };
    let mut m = military(vec![MapDivision::new(md(1), C1, p(10), 1.0).with_combat(hurt)], control_of(&[(10, C1), (11, C2), (12, C2)]), vec![(C1, C2)]);
    m.issue_move_order(&g, md(1), p(11)).unwrap();
    let mut all = Vec::new();
    for day in 1..=8usize {
        let ev = m.advance_day(&g);
        let org = m.division(md(1)).unwrap().combat.organization;
        if day < 5 {
            assert_eq!((m.control().controller(p(11)), org), (Some(C2), 50.0), "day {day}: moving divisions do not recover, and nothing is occupied yet");
        } else if day == 5 {
            // 到着日: 支配が変わり(要求 → 適用の順)、当日の組織率回復(+5)が占領後の自国支配地として効く
            assert_eq!(m.control().controller(p(11)), Some(C1));
            assert_eq!(org, 55.0, "recovery happens after the occupation on the arrival day");
            let claim_at = ev.iter().position(is_claim).unwrap();
            let changed_at = ev.iter().position(is_changed).unwrap();
            assert!(claim_at < changed_at);
            assert_eq!(
                ev[changed_at],
                MoveEvent::ControlChanged { province: p(11), previous: Some(C2), new: C1, reason: ClaimReason::Undefended, divisions: vec![md(1)] }
            );
        } else {
            assert_eq!(org, 55.0 + 5.0 * (day as f32 - 5.0), "day {day}: recovery continues in the newly owned province");
        }
        all.extend(ev.into_iter().map(|e| (day, e)));
    }
    // 翌日以降に占領が重複しない: 要求・変更は1件ずつ、見送り・古い要求の結果も出ない
    assert_eq!((count(&all, &is_claim), count(&all, &is_changed), count(&all, &is_not_applied)), (1, 1, 0));
    // 12 は国家2のまま(同じ命令で奥へ進んでいない)
    assert_eq!(m.control().controller(p(12)), Some(C2));
}

// ---------------------------------------------------------------- 攻撃側の勝利: 終了日に支配が変わり、同日に組織率が回復する

#[test]
fn an_attacker_victory_changes_control_on_the_finishing_day_and_the_organization_recovers_that_same_day() {
    // 星形: 攻撃側(国家1、標準)が idx1 から中心 idx0(国家2、弱い守備)を攻める。1日で決着する。
    let g = star_graph();
    let ctl = star_control(&[C2, C1, C1, C2, C2, C1, C2]);
    let mut m = military(vec![MapDivision::new(md(1), C1, p(pid(1)), 1.0), MapDivision::new(md(2), C2, p(pid(0)), 1.0).with_combat(weak())], ctl, vec![(C1, C2)]);
    m.issue_move_order(&g, md(1), p(pid(0))).unwrap();
    let ev = run_days(&mut m, &g, 4);
    assert!(ev.is_empty() || ev.iter().all(|(_, e)| !is_changed(e)), "nothing happens before the arrival day");
    let day5 = m.advance_day(&g);
    // 戦闘の終了 → 要求 → 適用 の順で、同じ日に起きる
    let pos = |f: &dyn Fn(&MoveEvent) -> bool| day5.iter().position(f).unwrap();
    let finished = pos(&|e| matches!(e, MoveEvent::BattleFinished { .. }));
    let claim = pos(&is_claim);
    let changed = pos(&is_changed);
    assert!(finished < claim && claim < changed);
    assert_eq!(m.control().controller(p(pid(0))), Some(C1));
    assert_eq!(
        day5[changed],
        MoveEvent::ControlChanged { province: p(pid(0)), previous: Some(C2), new: C1, reason: ClaimReason::AttackerWonBattle(MapBattleId(0)), divisions: vec![md(1)] }
    );
    // 当日の組織率回復は、占領後(自国支配地)として効く: 旧の計算で求めた1日目の損耗 + 5
    let std = CombatStats::standard();
    let w = weak();
    let dmg = legacy_combat::resolve_day(&[(md(1), C1, &std)], &[(md(2), C2, &w)]).unwrap();
    let expected = ((100.0f32 - dmg.attackers[&md(1)].1).max(0.0) + 5.0).min(100.0);
    assert_eq!(m.division(md(1)).unwrap().combat.organization, expected);
    // 翌日以降に重複しない
    let later = run_days(&mut m, &g, 20);
    assert_eq!(count(&later, &is_claim) + count(&later, &is_changed) + count(&later, &is_not_applied), 0);
}

// ---------------------------------------------------------------- 守備側勝利・全滅・ゼロ損害では占領しない

#[test]
fn defender_victory_total_annihilation_and_zero_damage_never_occupy() {
    // 守備側勝利(実データ C: 弱い攻撃側が 17269 を攻める)
    let (w, g) = real();
    let mut m = real_military(&g, &w);
    let initial: Vec<(ProvinceId, CountryId)> = m.control().iter().collect();
    m.issue_move_order(&g, md(3), p(17269)).unwrap();
    let ev = run_days(&mut m, &g, 15);
    assert!(ev.iter().any(|(_, e)| matches!(e, MoveEvent::BattleFinished { outcome: BattleOutcome::DefenderWon, reason: FinishReason::AttackerGone, .. })));
    assert_eq!(count(&ev, &is_claim) + count(&ev, &is_changed) + count(&ev, &is_not_applied), 0);
    assert_eq!(m.control().iter().collect::<Vec<_>>(), initial);
    // ゼロ損害(攻撃力・防御力 0): 防御側勝利
    let g = star_graph();
    let inert = CombatStats { attack_power: 0, defense_power: 0, ..CombatStats::standard() };
    let ctl = star_control(&[C2, C1, C1, C2, C2, C2, C2]);
    let mut m = military(vec![MapDivision::new(md(1), C1, p(pid(1)), 1.0).with_combat(inert), MapDivision::new(md(2), C2, p(pid(0)), 1.0).with_combat(inert)], ctl.clone(), vec![(C1, C2)]);
    m.issue_move_order(&g, md(1), p(pid(0))).unwrap();
    let ev = run_days(&mut m, &g, 8);
    assert!(ev.iter().any(|(_, e)| matches!(e, MoveEvent::BattleFinished { reason: FinishReason::ZeroDamage, .. })));
    assert_eq!(count(&ev, &is_claim), 0);
    assert_eq!(m.control(), &ctl);
    // 両陣営の全滅(人員 500・組織率 1 の同じ戦闘能力)。防御側が到着までの数日に組織率を回復しないよう、速度 5.0 で1日目に接触させる(両者とも 1日で組織率 0 になり、
    // 退却の追加損失(最大人員の1割)で人員が尽きて撃破される)。防御側勝利扱いで、支配は変わらない。
    let frail = CombatStats { manpower: 500, organization: 1.0, ..CombatStats::standard() };
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(pid(1)), 5.0).with_combat(frail), MapDivision::new(md(2), C2, p(pid(0)), 1.0).with_combat(frail)],
        ctl.clone(),
        vec![(C1, C2)],
    );
    m.issue_move_order(&g, md(1), p(pid(0))).unwrap();
    let ev = run_days(&mut m, &g, 3);
    assert!(ev.iter().any(|(d, e)| *d == 1 && matches!(e, MoveEvent::BattleFinished { reason: FinishReason::BothGone, outcome: BattleOutcome::DefenderWon, .. })), "{:?}", ev.iter().map(|(_, e)| e).collect::<Vec<_>>());
    assert_eq!(count(&ev, &is_claim), 0);
    assert_eq!(m.control(), &ctl);
    assert!(m.division(md(1)).is_none() && m.division(md(2)).is_none(), "both divisions were destroyed");
}

// ---------------------------------------------------------------- 到達結果の無効化対象

#[test]
fn reach_invalidation_targets_are_only_the_previous_and_new_controllers_without_duplicates() {
    // 10(国家1) 11(国家2) 12(国家2) 13(国家1) 14(国家3、傍観)。国家1の師団が 10→11 と 13→12 を同じ日に占領する(どちらも 国家2 → 国家1)。
    let g = line_graph(5);
    let ctl = control_of(&[(10, C1), (11, C2), (12, C2), (13, C1), (14, C3)]);
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(10), 1.0), MapDivision::new(md(2), C1, p(13), 1.0), MapDivision::new(md(3), C3, p(14), 1.0)],
        ctl,
        vec![(C1, C2), (C3, C2)],
    );
    m.issue_move_order(&g, md(1), p(11)).unwrap();
    m.issue_move_order(&g, md(2), p(12)).unwrap();
    let mut day5 = Vec::new();
    for _ in 1..=5 {
        day5 = m.advance_day(&g);
    }
    assert_eq!(day5.iter().filter(|e| is_changed(e)).count(), 2, "two provinces changed on the same day");
    assert_eq!(countries_to_invalidate(&day5), vec![C1, C2], "country 3 (a bystander) is not a target and no country appears twice");
    // 別の日に国家3が国家2から占領すると、その日の対象は 国家2 と 国家3(国家1 は含まれない)
    let mut m2 = military(
        vec![MapDivision::new(md(3), C3, p(14), 1.0), MapDivision::new(md(1), C1, p(10), 1.0)],
        control_of(&[(10, C1), (11, C1), (12, C2), (13, C2), (14, C3)]),
        vec![(C1, C2), (C3, C2)],
    );
    m2.issue_move_order(&g, md(3), p(13)).unwrap();
    let mut ev = Vec::new();
    for _ in 1..=5 {
        ev = m2.advance_day(&g);
    }
    assert_eq!(countries_to_invalidate(&ev), vec![C2, C3]);
    // 変更のない日は空
    assert!(countries_to_invalidate(&m2.advance_day(&g)).is_empty());
}

// ---------------------------------------------------------------- 適用の確認: 適用済み・古い要求・交戦関係・有効な攻撃側師団・交戦国の残存・同日の競合

fn claim_world() -> (MapMilitary, ClaimRequest) {
    // 10(国家1) 11(国家2)。国家1の師団1 が 11 にいる(守備隊なしで入ったあと)。
    let m = military(vec![MapDivision::new(md(1), C1, p(11), 1.0)], control_of(&[(10, C1), (11, C2)]), vec![(C1, C2)]);
    let req = ClaimRequest { province: p(11), claimant: C1, previous_controller: Some(C2), reason: ClaimReason::Undefended, divisions: vec![md(1)] };
    (m, req)
}

#[test]
fn a_claim_is_applied_once_and_applied_stale_or_unjustified_claims_never_overwrite_control() {
    let (mut m, req) = claim_world();
    // 適用
    assert_eq!(m.apply_claim(&req), ClaimOutcome::Applied { previous: Some(C2), new: C1 });
    assert_eq!(m.control().controller(p(11)), Some(C1));
    // 適用済みの要求を再度: 上書きせず、適用済みとして区別
    assert_eq!(m.apply_claim(&req), ClaimOutcome::Rejected(ClaimRejection::AlreadyApplied));
    assert_eq!(m.control().controller(p(11)), Some(C1));

    // 古い要求: 要求のあと(別の経路で)支配国が変わった → 上書きしない
    let (mut m, req) = claim_world();
    m.set_controller(p(11), Some(C3));
    assert_eq!(m.apply_claim(&req), ClaimOutcome::Rejected(ClaimRejection::Stale { current: Some(C3), requested_from: Some(C2) }));
    assert_eq!(m.control().controller(p(11)), Some(C3), "a stale claim does not overwrite");
    // 古い要求(支配国なしになっていた)
    m.set_controller(p(11), None);
    assert_eq!(m.apply_claim(&req), ClaimOutcome::Rejected(ClaimRejection::Stale { current: None, requested_from: Some(C2) }));
    assert_eq!(m.control().controller(p(11)), None);

    // 交戦関係: 現在の支配国が要求した国と交戦していない(要求時の支配国は一致している)
    let (mut m, _) = claim_world();
    m.set_controller(p(11), Some(C3));
    let req3 = ClaimRequest { previous_controller: Some(C3), ..ClaimRequest { province: p(11), claimant: C1, previous_controller: None, reason: ClaimReason::Undefended, divisions: vec![md(1)] } };
    assert_eq!(m.apply_claim(&req3), ClaimOutcome::Rejected(ClaimRejection::NotAtWar { current: Some(C3) }));
    assert_eq!(m.control().controller(p(11)), Some(C3));
    // 支配国なしの地点への要求(要求時も支配国なし): 交戦相手がいないので適用しない
    m.set_controller(p(11), None);
    let req_none = ClaimRequest { previous_controller: None, ..req3.clone() };
    assert_eq!(m.apply_claim(&req_none), ClaimOutcome::Rejected(ClaimRejection::NotAtWar { current: None }));

    // 有効な攻撃側師団がいない: 存在しない・別の国の師団・別の地点にいる師団・戦闘中の師団
    let (mut m, mut req) = claim_world();
    req.divisions = vec![md(9)];
    assert_eq!(m.apply_claim(&req), ClaimOutcome::Rejected(ClaimRejection::NoValidAttacker));
    req.divisions = vec![];
    assert_eq!(m.apply_claim(&req), ClaimOutcome::Rejected(ClaimRejection::NoValidAttacker));
    assert_eq!(m.control().controller(p(11)), Some(C2));
    let mut other_country = military(vec![MapDivision::new(md(1), C3, p(11), 1.0)], control_of(&[(10, C1), (11, C2)]), vec![(C1, C2)]);
    let (_, req) = claim_world();
    assert_eq!(other_country.apply_claim(&req), ClaimOutcome::Rejected(ClaimRejection::NoValidAttacker), "the listed division belongs to another country");
    let mut elsewhere = military(vec![MapDivision::new(md(1), C1, p(10), 1.0)], control_of(&[(10, C1), (11, C2)]), vec![(C1, C2)]);
    assert_eq!(elsewhere.apply_claim(&req), ClaimOutcome::Rejected(ClaimRejection::NoValidAttacker), "the division is not at the province");
    assert_eq!(elsewhere.control().controller(p(11)), Some(C2));
}

#[test]
fn enemy_divisions_that_remain_defer_the_claim_while_neutral_and_own_divisions_do_not_block_it_and_keep_their_state() {
    // 交戦国(国家2)の師団が地点に残っている → 見送り(理由に師団を明示)。支配は変えない
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(11), 1.0), MapDivision::new(md(2), C2, p(11), 1.0)],
        control_of(&[(10, C1), (11, C2)]),
        vec![(C1, C2)],
    );
    let req = ClaimRequest { province: p(11), claimant: C1, previous_controller: Some(C2), reason: ClaimReason::AttackerWonBattle(MapBattleId(0)), divisions: vec![md(1)] };
    assert_eq!(m.apply_claim(&req), ClaimOutcome::Rejected(ClaimRejection::EnemyDivisionsRemain { divisions: vec![md(2)] }));
    assert_eq!(m.control().controller(p(11)), Some(C2));
    // 要求は自動では再適用されない: 何もしなくても、日次処理を進めても支配は変わらない(要求を持ち越さない)
    let g2 = line_graph(2);
    for _ in 0..10 {
        let ev = m.advance_day(&g2);
        assert!(ev.iter().all(|e| !is_changed(e) && !is_claim(e) && !is_not_applied(e)));
    }
    assert_eq!(m.control().controller(p(11)), Some(C2));

    // 中立国(交戦していない)の師団・自国の別の師団が地点にいても、更新は妨げられず、その師団の状態は保たれる
    let neutral = MapDivision::new(md(3), C3, p(11), 1.0).with_combat(weak());
    let own_other = MapDivision::new(md(4), C1, p(11), 1.0);
    let mut m = military(vec![MapDivision::new(md(1), C1, p(11), 1.0), neutral.clone(), own_other.clone()], control_of(&[(10, C1), (11, C2)]), vec![(C1, C2)]);
    assert_eq!(m.apply_claim(&req), ClaimOutcome::Applied { previous: Some(C2), new: C1 });
    assert_eq!(m.division(md(3)), Some(&neutral), "the neutral division keeps its whole state");
    assert_eq!(m.division(md(4)), Some(&own_other), "an own division that did not take part keeps its state");
}

#[test]
fn a_non_participating_enemy_division_defers_the_conquest_after_a_won_battle_and_it_is_not_retried_the_next_day() {
    // 実データ(場面 G): 師団14(国家1)が 17486 を攻め、戦闘の最中に国家2の師団16が入る(自国支配地なので入れるが、戦闘には参加しない)。
    let (w, g) = real();
    let mut m = real_military(&g, &w);
    m.issue_move_order(&g, md(14), p(17486)).unwrap();
    for _ in 1..=2 {
        let _ = m.advance_day(&g);
    }
    m.issue_move_order(&g, md(16), p(17486)).unwrap();
    let mut ev: Vec<(usize, MoveEvent)> = Vec::new();
    for day in 3..=40usize {
        for e in m.advance_day(&g) {
            ev.push((day, e));
        }
    }
    // 戦闘は 5 日目に始まり、師団16 は 7 日目に入る(合流しない)
    assert!(ev.iter().any(|(d, e)| *d == 5 && matches!(e, MoveEvent::BattleStarted { province, attackers, defenders, .. } if *province == p(17486) && *attackers == vec![md(14)] && *defenders == vec![md(15)])));
    assert!(ev.iter().any(|(d, e)| *d == 7 && matches!(e, MoveEvent::Arrived { division, .. } if *division == md(16))));
    assert_eq!(count(&ev, &|e| matches!(e, MoveEvent::BattleJoined { division, .. } if *division == md(16))), 0, "a defender-side reinforcement does not join (inherited gap)");
    // 攻撃側の勝利の日: 要求は出るが、交戦国の師団(16)が残っているので適用が見送られる(理由つき)
    let finish_day = ev.iter().find(|(_, e)| matches!(e, MoveEvent::BattleFinished { outcome: BattleOutcome::AttackerWon, .. })).map(|(d, _)| *d).expect("the attacker won");
    assert_eq!(finish_day, 10);
    assert!(ev.iter().any(|(d, e)| *d == finish_day
        && matches!(e, MoveEvent::ClaimNotApplied { province, claimant, rejection: ClaimRejection::EnemyDivisionsRemain { divisions }, .. } if *province == p(17486) && *claimant == C1 && *divisions == vec![md(16)])));
    assert_eq!(count(&ev, &is_changed), 0, "the control never changes: the deferred claim is not applied later automatically");
    assert_eq!(m.control().controller(p(17486)), Some(C2));
    // 見送りのあと(11日目以降)は、要求も結果も二度と出ない
    assert_eq!(ev.iter().filter(|(d, e)| *d > finish_day && (is_claim(e) || is_changed(e) || is_not_applied(e))).count(), 0);
    // 非参加の師団16 は既存の状態を保つ(戦闘に入らず、Idle、元の戦闘能力)。攻撃側の師団14 は 17486 に Idle で残る
    let d16 = m.division(md(16)).unwrap();
    assert_eq!((d16.province, d16.status, d16.battle, d16.combat), (p(17486), MapDivisionStatus::Idle, None, CombatStats::standard()));
    let d14 = m.division(md(14)).unwrap();
    assert_eq!((d14.province, d14.status, d14.battle), (p(17486), MapDivisionStatus::Idle, None));
}

#[test]
fn same_day_same_province_competition_is_decided_by_the_stable_processing_order() {
    // 10(国家1) 11(国家2、守備隊なし) 12(国家3)。国家1(師団A)と国家3(師団B)が、どちらも国家2と交戦中で、同じ日に 11 へ着く。
    let g = line_graph(3);
    for (id_c1, id_c3, winner, loser, loser_from) in [(2u32, 1u32, C3, C1, 10u32), (1, 2, C1, C3, 12)] {
        let mut m = military(
            vec![MapDivision::new(md(id_c1), C1, p(10), 1.0), MapDivision::new(md(id_c3), C3, p(12), 1.0)],
            control_of(&[(10, C1), (11, C2), (12, C3)]),
            vec![(C1, C2), (C3, C2)],
        );
        m.issue_move_order(&g, md(id_c1), p(11)).unwrap();
        m.issue_move_order(&g, md(id_c3), p(11)).unwrap();
        let ev = run_days(&mut m, &g, 6);
        // 師団IDの小さい側が先に処理され、占領する。後から着く側は、処理時点の(更新済みの)支配国を見て、進入前に止まる
        assert_eq!(m.control().controller(p(11)), Some(winner), "ids c1={id_c1} c3={id_c3}");
        assert_eq!(count(&ev, &is_changed), 1);
        let loser_id = if loser == C1 { id_c1 } else { id_c3 };
        assert!(ev.iter().any(|(_, e)| *e == MoveEvent::Stopped { division: md(loser_id), at: p(loser_from), blocked: p(11) }), "the other country stops before entering");
        assert_eq!(m.division(md(loser_id)).unwrap().province, p(loser_from));
        assert_eq!(count(&ev, &is_not_applied), 0, "the later division never reaches a claim");
    }
    // 同じ国の2師団が同日に着く場合: 先に処理された師団が占領し、後の師団は自国支配地への通常の到着(2回目の要求は出ない)
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(10), 1.0), MapDivision::new(md(2), C1, p(12), 1.0)],
        control_of(&[(10, C1), (11, C2), (12, C1)]),
        vec![(C1, C2)],
    );
    m.issue_move_order(&g, md(1), p(11)).unwrap();
    m.issue_move_order(&g, md(2), p(11)).unwrap();
    let ev = run_days(&mut m, &g, 6);
    assert_eq!((count(&ev, &is_claim), count(&ev, &is_changed)), (1, 1));
    assert!(ev.iter().any(|(d, e)| *d == 5 && matches!(e, MoveEvent::Arrived { division, province } if *division == md(2) && *province == p(11))));
}

// ---------------------------------------------------------------- 後続の移動・退路から新しい支配国が見える

#[test]
fn later_orders_and_the_same_days_later_retreat_decisions_see_the_new_controller() {
    // 10(国家1) 11(国家2) 12(国家2)。師団1 が 5 日目に 11 を占領。国家1の別の師団2(10)は、その後 12 まで命令できる(11 は自国支配地として通る)。
    let g = line_graph(3);
    let mut m = military(
        vec![MapDivision::new(md(1), C1, p(10), 1.0), MapDivision::new(md(2), C1, p(10), 1.0)],
        control_of(&[(10, C1), (11, C2), (12, C2)]),
        vec![(C1, C2)],
    );
    m.issue_move_order(&g, md(1), p(11)).unwrap();
    run_days(&mut m, &g, 5);
    assert_eq!(m.control().controller(p(11)), Some(C1));
    // 12 までの経路: 11(自国)→12(敵)。11 は敵支配地としての接触を起こさず、通常の移動で通る
    assert_eq!(m.issue_move_order(&g, md(2), p(12)), Ok(OrderOutcome::Started { hops: 2 }));
    let ev = run_days(&mut m, &g, 10);
    assert!(ev.iter().any(|(d, e)| *d == 5 && matches!(e, MoveEvent::HopCompleted { division, province } if *division == md(2) && *province == p(11))));
    assert_eq!(count(&ev, &is_changed), 1, "the second province (12) is occupied on the day division 2 arrives there");
    assert_eq!(m.control().controller(p(12)), Some(C1));
    // 実データ: 占領のあと、師団2 は新しい命令で隣接プロヴィンスへ進める
    let (w, gr) = real();
    let mut m = real_military(&gr, &w);
    m.issue_move_order(&gr, md(2), p(17694)).unwrap();
    let ev1 = run_days(&mut m, &gr, 5);
    assert_eq!(count(&ev1, &is_changed), 1);
    assert_eq!(m.issue_move_order(&gr, md(2), p(17695)), Ok(OrderOutcome::Started { hops: 1 }), "17694 is now controlled by country 1; the next province can be ordered");
    let ev2 = run_days(&mut m, &gr, 6);
    assert!(ev2.iter().any(|(d, e)| *d == 5 && matches!(e, MoveEvent::ControlChanged { province, previous: Some(prev), new, .. } if *province == p(17695) && *prev == C2 && *new == C1)));
}

// ---------------------------------------------------------------- 法的所有と他プロヴィンスの維持

#[test]
fn only_the_conquered_province_changes_and_the_state_ownership_and_other_provinces_are_untouched() {
    let (w, g) = real();
    let mut m = real_military(&g, &w);
    let before: BTreeMap<ProvinceId, CountryId> = m.control().iter().collect();
    let state21_before: Vec<ProvinceId> = w.model.provinces_of(MapStateId(21)).unwrap().to_vec();
    m.issue_move_order(&g, md(2), p(17694)).unwrap();
    run_days(&mut m, &g, 12);
    let after: BTreeMap<ProvinceId, CountryId> = m.control().iter().collect();
    // 変わったのは 17694 だけ(国家2 → 国家1)
    let changed: Vec<(ProvinceId, Option<CountryId>, Option<CountryId>)> = before
        .keys()
        .chain(after.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|k| before.get(*k) != after.get(*k))
        .map(|k| (*k, before.get(k).copied(), after.get(k).copied()))
        .collect();
    assert_eq!(changed, vec![(p(17694), Some(C2), Some(C1))]);
    // 同じステート(21)の別プロヴィンスは国家2の支配のまま。ステートの所属(法的所有の単位)は不変
    for pr in state21_before.iter().filter(|x| **x != p(17694)) {
        assert_eq!(after.get(pr).copied(), before.get(pr).copied(), "{pr}");
    }
    assert_eq!(w.model.provinces_of(MapStateId(21)).unwrap(), state21_before.as_slice());
    assert_eq!(w.model.state_of(p(17694)), Some(MapStateId(21)), "the province still belongs to state 21");
}

// ---------------------------------------------------------------- 決定性

#[test]
fn occupation_results_do_not_depend_on_registration_or_order_issue_order() {
    let (w, g) = real();
    let run = |reverse: bool| {
        let mut sc = load_division_scenario(BATTLE, &g, &w.model).unwrap();
        if reverse {
            sc.divisions.reverse();
        }
        let mut m = sc.into_military();
        let mut orders: Vec<(u32, u32)> = vec![(1, 18502), (2, 17694), (3, 17269), (4, 18111), (7, 18111), (10, 18111), (11, 17906), (14, 17486)];
        if reverse {
            orders.reverse();
        }
        for (d, dest) in orders {
            let _ = m.issue_move_order(&g, md(d), p(dest));
        }
        let mut ev = Vec::new();
        for _ in 0..40 {
            ev.push(m.advance_day(&g));
        }
        (ev, m)
    };
    let (ea, ma) = run(false);
    let (eb, mb) = run(true);
    assert_eq!(ea, eb);
    assert_eq!(ma, mb);
    // 占領は複数あり(守備隊なし 1、攻撃側の勝利 B・D・E・G の 4 戦のうち支配が変わるもの)、競合なく重複もない
    let changed: Vec<ProvinceId> = ea.iter().flatten().filter_map(|e| if let MoveEvent::ControlChanged { province, .. } = e { Some(*province) } else { None }).collect();
    let mut sorted = changed.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), changed.len(), "no province is occupied twice: {changed:?}");
    assert!(changed.len() >= 3, "several provinces were occupied: {changed:?}");
}

// ---------------------------------------------------------------- 分離の確認

#[test]
fn the_occupation_code_stays_separate_from_economy_saving_the_old_military_and_state_ownership() {
    let src = std::fs::read_to_string("src/world/occupation.rs").unwrap();
    let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    for banned in [
        "military::", "war::", "MapEconomy", "economy::", "snapshot", "SaveLocation", "StateRegistry", "WarRegistry", "std::fs", "std::time", "Instant", "rand::", "WorldModel", "MapStateId",
        "owner_country", "occupation_progress",
    ] {
        assert!(!code.contains(banned), "occupation.rs: found {banned}");
    }
    assert!(!code.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == "StateId"), "must not use the old StateId");
}
