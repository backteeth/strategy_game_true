//! P32-038 (M6-e): プロヴィンスの実効支配のステート単位の集計(`world::state_control`)の検証。
//!
//! 集計はプロヴィンスの支配から導出する純粋な関数。法的所有はシナリオ(検証入力)の `state_owners`。時間は合否条件にしない。保存・アセットは書かない。

use std::collections::BTreeMap;

use strategy_game::common::CountryId;
use strategy_game::hexmap::data::load_from_path;
use strategy_game::world::*;

const C1: CountryId = CountryId(1);
const C2: CountryId = CountryId(2);
const C3: CountryId = CountryId(3);
const BATTLE: &str = "assets/data/world/preview/division_battle_scenario.ron";

fn p(id: u32) -> ProvinceId {
    ProvinceId(id)
}

fn s(id: u32) -> MapStateId {
    MapStateId(id)
}

fn ctl(entries: &[(u32, CountryId)]) -> ProvinceControl {
    ProvinceControl::new(entries.iter().map(|(id, c)| (p(*id), *c)).collect())
}

fn owners(entries: &[(u32, CountryId)]) -> BTreeMap<MapStateId, CountryId> {
    entries.iter().map(|(id, c)| (s(*id), *c)).collect()
}

fn real() -> (WorldPreviewData, ProvinceGraph) {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let w = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).unwrap();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    (w, g)
}

fn real_scenario(g: &ProvinceGraph, w: &WorldPreviewData) -> DivisionScenario {
    load_division_scenario(BATTLE, g, &w.model).unwrap()
}

fn summary_of(w: &WorldPreviewData, sc: &DivisionScenario, control: &ProvinceControl) -> StateControlSummary {
    summarize_control(&w.model, &sc.state_owners, control).unwrap()
}

// ---------------------------------------------------------------- 純粋な集計(合成の所属)

/// 状態 1: 4 件(全域 C1)/ 状態 2: 4 件(C1 と C2 と支配国なし)/ 状態 3: 3 件(全件支配国なし)/ 状態 4: 所属なし / 未所属 2 件。
fn members() -> Vec<(MapStateId, Vec<ProvinceId>)> {
    vec![
        (s(1), vec![p(1), p(2), p(3), p(4)]),
        (s(2), vec![p(5), p(6), p(7), p(8)]),
        (s(3), vec![p(9), p(10), p(11)]),
        (s(4), vec![]),
    ]
}

fn run(members: &[(MapStateId, Vec<ProvinceId>)], unassigned: &[ProvinceId], o: &BTreeMap<MapStateId, CountryId>, c: &ProvinceControl) -> Result<StateControlSummary, ControlSummaryError> {
    let m: Vec<(MapStateId, &[ProvinceId])> = members.iter().map(|(id, v)| (*id, v.as_slice())).collect();
    summarize_control_membership(&m, unassigned, o, c)
}

fn base_control() -> ProvinceControl {
    // 未所属の 100・101 は C3 が支配(どのステートの分母にも入らない)
    ctl(&[(1, C1), (2, C1), (3, C1), (4, C1), (5, C1), (6, C2), (7, C2), (100, C3), (101, C3)])
}

#[test]
fn kinds_are_distinguished_multi_partial_none_and_empty() {
    let sum = run(&members(), &[p(100), p(101)], &owners(&[(1, C1), (2, C1), (3, C2)]), &base_control()).unwrap();
    let st = |id| sum.state(s(id)).unwrap();
    assert_eq!(st(1).kind(), StateControlKind::Full(C1));
    // 複数国 + 支配国なし
    assert_eq!(st(2).kind(), StateControlKind::Mixed);
    assert_eq!(st(2).by_country, vec![(C1, 1), (C2, 2)]);
    assert_eq!(st(2).uncontrolled, 1);
    // 全件が支配国なし(所属 0 件とは別)
    assert_eq!(st(3).kind(), StateControlKind::NoController);
    assert_eq!((st(3).total, st(3).uncontrolled), (3, 3));
    // 所属なし
    assert_eq!(st(4).kind(), StateControlKind::NoProvinces);
    assert_eq!(st(4).total, 0);
    assert_eq!(st(4).owner_share(), None);
    assert_eq!(st(4).share_of(C1), None);
    // 一部だけ 1 国が支配し、残りが支配国なし → Mixed(全域支配ではない)
    let part = run(&[(s(1), vec![p(1), p(2)])], &[], &BTreeMap::new(), &ctl(&[(1, C1)])).unwrap();
    assert_eq!(part.state(s(1)).unwrap().kind(), StateControlKind::Mixed);
}

#[test]
fn the_unassigned_provinces_are_in_no_denominator() {
    let sum = run(&members(), &[p(100), p(101)], &BTreeMap::new(), &base_control()).unwrap();
    assert_eq!(sum.unassigned(), 2);
    let total: usize = sum.states().iter().map(|x| x.total).sum();
    assert_eq!(total, 11, "only the member provinces are counted in the denominators");
    for st in sum.states() {
        assert_eq!(st.count_of(C3), 0, "C3 controls only unassigned provinces; it must not appear in any state");
    }
}

#[test]
fn shares_keep_numerator_and_denominator_and_count_no_controller_in_the_denominator() {
    let sum = run(&members(), &[], &owners(&[(2, C1)]), &base_control()).unwrap();
    let st = sum.state(s(2)).unwrap();
    assert_eq!(st.share_of(C2), Some(ShareOf { numerator: 2, denominator: 4 }));
    assert_eq!(st.owner_share(), Some(ShareOf { numerator: 1, denominator: 4 }));
    assert_eq!(st.uncontrolled_share(), Some(ShareOf { numerator: 1, denominator: 4 }));
    // 分母は支配国なしを含む所属総数: 国別の分子の合計は分母より小さい
    assert!(st.by_country.iter().map(|(_, n)| n).sum::<usize>() < st.total);
}

#[test]
fn counts_always_add_up_to_the_membership_total() {
    let sum = run(&members(), &[p(100), p(101)], &owners(&[(1, C1)]), &base_control()).unwrap();
    assert!(sum.is_consistent());
    for st in sum.states() {
        assert!(st.is_consistent(), "{:?}", st);
        assert_eq!(st.by_country.iter().map(|(_, n)| n).sum::<usize>() + st.uncontrolled, st.total);
    }
}

#[test]
fn output_order_and_the_result_do_not_depend_on_the_input_order() {
    let o = owners(&[(1, C1), (2, C1), (3, C2)]);
    let a = run(&members(), &[p(100), p(101)], &o, &base_control()).unwrap();
    let mut rev = members();
    rev.reverse();
    for (_, v) in rev.iter_mut() {
        v.reverse();
    }
    let o2: BTreeMap<MapStateId, CountryId> = [(s(3), C2), (s(2), C1), (s(1), C1)].into_iter().collect();
    let c2 = ctl(&[(101, C3), (100, C3), (7, C2), (6, C2), (5, C1), (4, C1), (3, C1), (2, C1), (1, C1)]);
    let b = run(&rev, &[p(101), p(100)], &o2, &c2).unwrap();
    assert_eq!(a, b);
    let ids: Vec<u32> = a.states().iter().map(|x| x.state.0).collect();
    assert_eq!(ids, vec![1, 2, 3, 4], "ascending by state id");
    for st in a.states() {
        let cs: Vec<usize> = st.by_country.iter().map(|(c, _)| c.0).collect();
        assert!(cs.windows(2).all(|w| w[0] < w[1]), "ascending by country id");
    }
}

#[test]
fn invalid_membership_is_rejected() {
    assert_eq!(run(&[(s(1), vec![p(1)]), (s(1), vec![p(2)])], &[], &BTreeMap::new(), &ctl(&[])), Err(ControlSummaryError::DuplicateState(s(1))));
    assert_eq!(run(&[(s(1), vec![p(1)]), (s(2), vec![p(1)])], &[], &BTreeMap::new(), &ctl(&[])), Err(ControlSummaryError::ProvinceInMultipleStates(p(1))));
    assert_eq!(run(&[(s(1), vec![p(1)])], &[p(1)], &BTreeMap::new(), &ctl(&[])), Err(ControlSummaryError::ProvinceInMultipleStates(p(1))));
    assert_eq!(run(&[(s(1), vec![p(1)])], &[], &owners(&[(9, C1)]), &ctl(&[])), Err(ControlSummaryError::OwnerOfUnknownState(s(9))));
}

#[test]
fn occupation_names_the_war_and_a_third_country_is_not_added() {
    // 状態 2(法的所有 C1): C1 が 1 件、C2 が 2 件、C3 が 1 件を支配。C2 だけが C1 と交戦中。
    let c = ctl(&[(5, C1), (6, C2), (7, C2), (8, C3)]);
    let sum = run(&[(s(2), vec![p(5), p(6), p(7), p(8)])], &[], &owners(&[(2, C1)]), &c).unwrap();
    let st = sum.state(s(2)).unwrap();
    let wars = |a: CountryId, b: CountryId| (a == C1 && b == C2) || (a == C2 && b == C1);
    assert_eq!(st.occupation_by(C2, &wars), Some(ShareOf { numerator: 2, denominator: 4 }));
    // 第三国 C3 の支配は、C1 との交戦関係がないので占領に数えない(内訳にはある)
    assert_eq!(st.occupation_by(C3, &wars), None);
    assert_eq!(st.count_of(C3), 1);
    // C3 が C1 と交戦中なら占領として数える(関係を明示すれば判定が変わる)
    let wars2 = |a: CountryId, b: CountryId| wars(a, b) || (a == C1 && b == C3) || (a == C3 && b == C1);
    assert_eq!(st.occupation_by(C3, &wars2), Some(ShareOf { numerator: 1, denominator: 4 }));
    // C2 と C3 の「合計」を占領としない: 交戦国ごとに別々の値
    assert_eq!(st.occupation_by(C2, &wars2), Some(ShareOf { numerator: 2, denominator: 4 }));
    // 法的所有国自身・所有国なしは None
    assert_eq!(st.occupation_by(C1, &wars), None);
    let no_owner = run(&[(s(2), vec![p(5), p(6), p(7), p(8)])], &[], &BTreeMap::new(), &c).unwrap();
    assert_eq!(no_owner.state(s(2)).unwrap().occupation_by(C2, &wars), None);
    // 交戦中だが 1 件も支配していなければ 0 / 総数
    let none = run(&[(s(2), vec![p(5)])], &[], &owners(&[(2, C1)]), &ctl(&[(5, C1)])).unwrap();
    assert_eq!(none.state(s(2)).unwrap().occupation_by(C2, &wars), Some(ShareOf { numerator: 0, denominator: 1 }));
}

// ---------------------------------------------------------------- 実際の検証地域・シナリオ

#[test]
fn the_scenario_state_owners_are_kept_and_the_initial_summary_matches_the_inputs() {
    let (w, g) = real();
    let sc = real_scenario(&g, &w);
    assert_eq!(sc.state_owners, owners(&[(20, C1), (22, C1), (21, C2)]));
    let sum = summary_of(&w, &sc, &sc.control);
    assert!(sum.is_consistent());
    assert_eq!(sum.states().len(), 3);
    assert_eq!(sum.unassigned(), 3, "17903, 18110 and 18501 belong to no state");
    // ステート 21(7+... = 13 件。所有 C2)は全域 C2 の支配(上書きの 17696 だけ C1)
    let st21 = sum.state(s(21)).unwrap();
    assert_eq!(st21.legal_owner, Some(C2));
    assert_eq!(st21.total, 13);
    assert_eq!(st21.count_of(C1), 3, "17696, 17905 and 18112 are overridden to country 1");
    assert_eq!(st21.count_of(C3), 1, "18312 is overridden to country 3");
    assert_eq!(st21.count_of(C2), 9);
    // 所属総数はモデルの所属と一致
    for st in sum.states() {
        assert_eq!(st.total, w.model.provinces_of(st.state).unwrap().len());
    }
}

#[test]
fn a_partial_occupation_changes_counts_and_shares_but_not_the_legal_owner() {
    let (w, g) = real();
    let sc = real_scenario(&g, &w);
    let before = summary_of(&w, &sc, &sc.control);
    let mut m = sc.clone().into_military();
    m.issue_move_order(&g, MapDivisionId(2), p(17694)).unwrap();
    let mut changed_day = None;
    let mut day_before: Option<StateControlSummary> = None;
    for day in 1..=15 {
        let pre = summary_of(&w, &sc, m.control());
        let ev = m.advance_day(&g);
        let post = summary_of(&w, &sc, m.control());
        if ev.iter().any(|e| matches!(e, MoveEvent::ControlChanged { province, .. } if *province == p(17694))) {
            changed_day = Some(day);
            day_before = Some(pre);
            // 占領当日の要約に古い支配が残らない
            assert_eq!(post.state(s(21)).unwrap().count_of(C1), 4);
            assert_eq!(post.state(s(21)).unwrap().count_of(C2), 8);
            break;
        }
        assert_eq!(pre, post, "no control change, no summary change (day {day})");
    }
    assert!(changed_day.is_some(), "the province was occupied");
    let day_before = day_before.unwrap();
    assert_eq!(day_before, before, "before the occupation the summary is the initial one");
    let after = summary_of(&w, &sc, m.control());
    let (b, a) = (before.state(s(21)).unwrap(), after.state(s(21)).unwrap());
    assert_eq!(a.legal_owner, b.legal_owner, "the legal owner does not move on a partial occupation");
    assert_eq!(a.total, b.total);
    assert_eq!(a.by_country, vec![(C1, 4), (C2, 8), (C3, 1)]);
    assert_eq!(b.by_country, vec![(C1, 3), (C2, 9), (C3, 1)]);
    assert_eq!(a.kind(), StateControlKind::Mixed);
    // 交戦(1-2)を明示した占領の割合: 国家1 が国家2 の法的所有のステートで 4/13(元から支配していた上書きの 3 件を含む。検証入力)
    let wars = |x: CountryId, y: CountryId| m.at_war(x, y);
    assert_eq!(a.occupation_by(C1, &wars), Some(ShareOf { numerator: 4, denominator: 13 }));
    // 他のステートは変わらない
    assert_eq!(after.state(s(20)), before.state(s(20)));
    assert_eq!(after.state(s(22)), before.state(s(22)));
}

#[test]
fn full_occupation_and_recapture_update_the_summary() {
    let (w, g) = real();
    let sc = real_scenario(&g, &w);
    let mut control = sc.control.clone();
    let members: Vec<ProvinceId> = w.model.provinces_of(s(21)).unwrap().to_vec();
    // 全域を国家1 が支配
    for pr in &members {
        control.set(*pr, Some(C1));
    }
    let full = summary_of(&w, &sc, &control);
    let st = full.state(s(21)).unwrap();
    assert_eq!(st.kind(), StateControlKind::Full(C1));
    assert_eq!(st.legal_owner, Some(C2), "still the legal owner: the transfer of territory is for M7");
    assert_eq!(st.owner_share(), Some(ShareOf { numerator: 0, denominator: 13 }));
    assert_eq!(st.share_of(C1), Some(ShareOf { numerator: 13, denominator: 13 }));
    // 1 件を国家2 が奪還
    control.set(members[0], Some(C2));
    let re = summary_of(&w, &sc, &control);
    let st = re.state(s(21)).unwrap();
    assert_eq!(st.kind(), StateControlKind::Mixed);
    assert_eq!(st.by_country, vec![(C1, 12), (C2, 1)]);
    // 全件を奪還
    for pr in &members {
        control.set(*pr, Some(C2));
    }
    assert_eq!(summary_of(&w, &sc, &control).state(s(21)).unwrap().kind(), StateControlKind::Full(C2));
    // 全件を支配国なしにする
    for pr in &members {
        control.set(*pr, None);
    }
    assert_eq!(summary_of(&w, &sc, &control).state(s(21)).unwrap().kind(), StateControlKind::NoController);
}

#[test]
fn a_third_country_control_in_the_real_scenario_is_not_an_occupation_of_the_owner() {
    let (w, g) = real();
    let sc = real_scenario(&g, &w);
    let m = sc.clone().into_military();
    // ステート 20(所有 C1)の 1 件を C3 が支配する。C3 は C2 とだけ交戦中で、C1 とは交戦していない。
    let mut control = sc.control.clone();
    let target = w.model.provinces_of(s(20)).unwrap()[0];
    control.set(target, Some(C3));
    let sum = summary_of(&w, &sc, &control);
    let st = sum.state(s(20)).unwrap();
    assert_eq!(st.count_of(C3), 1, "the third country appears in the breakdown");
    let wars = |x: CountryId, y: CountryId| m.at_war(x, y);
    assert_eq!(st.occupation_by(C3, &wars), None, "but it is not an occupation by a belligerent of the owner");
    assert_eq!(st.occupation_by(C2, &wars), Some(ShareOf { numerator: 0, denominator: 7 }));
    // ステート 21(所有 C2)の 1 件を C3 が支配する場合は、C3 は C2 と交戦中なので占領として数える
    let base_c3 = sum.state(s(21)).unwrap().count_of(C3);
    let t21 = w.model.provinces_of(s(21)).unwrap().iter().copied().find(|x| control.controller(*x) == Some(C2)).unwrap();
    control.set(t21, Some(C3));
    let sum = summary_of(&w, &sc, &control);
    assert_eq!(sum.state(s(21)).unwrap().occupation_by(C3, &wars), Some(ShareOf { numerator: base_c3 + 1, denominator: 13 }));
}

#[test]
fn a_deferred_claim_does_not_change_the_summary() {
    let (w, g) = real();
    let sc = real_scenario(&g, &w);
    let mut m = sc.clone().into_military();
    // 場面 G: 見送りになる占領(他の師団には命令を出さない)
    m.issue_move_order(&g, MapDivisionId(14), p(17486)).unwrap();
    let mut deferred_day = None;
    let mut before_day: Option<StateControlSummary> = None;
    for day in 1..=40 {
        let pre = summary_of(&w, &sc, m.control());
        if day == 3 {
            m.issue_move_order(&g, MapDivisionId(16), p(17486)).unwrap();
        }
        let ev = m.advance_day(&g);
        let post = summary_of(&w, &sc, m.control());
        if ev.iter().any(|e| matches!(e, MoveEvent::ClaimNotApplied { rejection: ClaimRejection::EnemyDivisionsRemain { .. }, .. })) {
            deferred_day = Some(day);
            before_day = Some(pre);
            assert_eq!(pre_eq(&before_day), post, "the deferred claim leaves the summary unchanged");
            break;
        }
    }
    assert_eq!(deferred_day, Some(10));
    // 見送り後も(自動で再適用されないので)変わらない
    for _ in 11..=15 {
        m.advance_day(&g);
    }
    assert_eq!(summary_of(&w, &sc, m.control()), pre_eq(&before_day));
    assert_eq!(m.control().controller(p(17486)), Some(C2));
}

fn pre_eq(x: &Option<StateControlSummary>) -> StateControlSummary {
    x.clone().expect("recorded")
}

#[test]
fn the_summary_is_derived_and_does_not_touch_the_control_or_the_military() {
    let (w, g) = real();
    let sc = real_scenario(&g, &w);
    let m = sc.clone().into_military();
    let control_before: Vec<(ProvinceId, CountryId)> = m.control().iter().collect();
    let a = summarize_control(&w.model, &sc.state_owners, m.control()).unwrap();
    let b = summarize_control(&w.model, &sc.state_owners, m.control()).unwrap();
    assert_eq!(a, b, "deterministic");
    assert_eq!(m.control().iter().collect::<Vec<_>>(), control_before, "summarizing does not change the control");
}

#[test]
fn existing_scenarios_still_run_with_the_new_scenario_field() {
    let (w, g) = real();
    for path in [
        "assets/data/world/preview/division_movement_scenario.ron",
        "assets/data/world/preview/division_movement_scenario_no_bridge.ron",
        "assets/data/world/preview/division_battle_scenario.ron",
        "assets/data/world/preview/division_supply_scenario.ron",
    ] {
        let sc = load_division_scenario(path, &g, &w.model).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut m = sc.clone().into_military();
        for _ in 0..3 {
            m.advance_day(&g);
        }
        assert!(summarize_control(&w.model, &sc.state_owners, m.control()).unwrap().is_consistent(), "{path}");
    }
}
