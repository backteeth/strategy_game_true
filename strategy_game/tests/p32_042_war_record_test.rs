//! P32-042 (M7-b): 戦争記録・終結・休戦(`world::war_record`)の検証。
//!
//! 検証用シナリオ(`division_war_scenario.ron` = A、`division_war_elimination_scenario.ron` = B)の所有・支配・戦争・首都は検証入力で、正式な世界データではない。
//! 旧経路(`war::peace::execute_peace_settlement` の休戦 1825 日、`DiplomacyRegistry` の休戦の境界)との比較は、旧型で実際に実行する。保存・アセットは書かない。

use strategy_game::app::time::GameDate;
use strategy_game::common::CountryId;
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::hexmap::data::{HexMap, load_from_path};
use strategy_game::world::*;

const C1: CountryId = CountryId(1);
const C2: CountryId = CountryId(2);
const C3: CountryId = CountryId(3);
const C4: CountryId = CountryId(4);
const SC_A: &str = "assets/data/world/preview/division_war_scenario.ron";
const SC_B: &str = "assets/data/world/preview/division_war_elimination_scenario.ron";
const LEGACY_TERR: &str = "assets/data/world/preview/division_territory_scenario.ron";
const LEGACY_BATTLE: &str = "assets/data/world/preview/division_battle_scenario.ron";

fn p(id: u32) -> ProvinceId {
    ProvinceId(id)
}
fn s(id: u32) -> MapStateId {
    MapStateId(id)
}
fn md(id: u32) -> MapDivisionId {
    MapDivisionId(id)
}
fn wid(id: u32) -> MapWarId {
    MapWarId(id)
}

struct World {
    #[allow(dead_code)]
    map: HexMap,
    data: WorldPreviewData,
    graph: ProvinceGraph,
}

fn world() -> World {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let data = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).unwrap();
    let graph = ProvinceGraph::build(&data.model, &map).unwrap();
    World { map, data, graph }
}

type State = (Territory, Conflicts, MapMilitary);

impl World {
    fn ctx(&self) -> TransferContext<'_> {
        TransferContext { model: &self.data.model, graph: &self.graph }
    }

    fn load(&self, path: &str) -> State {
        let sc = load_division_scenario(path, &self.graph, &self.data.model).unwrap();
        self.build(sc)
    }

    fn build(&self, sc: DivisionScenario) -> State {
        let t = Territory::from_scenario(&sc, &self.data.model, &self.graph).unwrap();
        let c = sc.conflicts();
        let mut m = sc.into_military();
        c.sync(&mut m);
        (t, c, m)
    }

    fn end(&self, st: &mut State, req: &EndWarRequest) -> EndWarOutcome {
        end_war(self.ctx(), &mut st.0, &mut st.1, &mut st.2, req).unwrap_or_else(|e| panic!("{e}"))
    }
}

fn pairs(v: &[(usize, usize)]) -> Vec<(CountryId, CountryId)> {
    v.iter().map(|(a, b)| (CountryId(*a), CountryId(*b))).collect()
}

fn until_battle(m: &mut MapMilitary, g: &ProvinceGraph, province: ProvinceId) {
    for _ in 0..40 {
        if m.ongoing_battle_at(province).is_some() {
            return;
        }
        m.advance_day(g);
    }
    panic!("no battle at {province}");
}

fn assert_unchanged(a: &State, b: &State) {
    assert_eq!(a.0, b.0, "territory");
    assert_eq!(a.1, b.1, "conflicts");
    assert_eq!(a.2, b.2, "military");
}

// ---------------------------------------------------------------- 記録と導出

#[test]
fn the_records_derive_the_hostility_and_legacy_scenarios_still_work() {
    let w = world();
    let (t, c, m) = w.load(SC_A);
    assert_eq!(c.wars().count(), 4);
    assert!(c.active_wars().all(|x| x.end.is_none()));
    assert_eq!(c.hostile_pairs(), pairs(&[(1, 2), (1, 3), (3, 4)]));
    assert!(m.at_war(C1, C2) && m.at_war(C2, C1) && m.at_war(C1, C3) && m.at_war(C3, C4));
    assert!(!m.at_war(C2, C3) && !m.at_war(C1, C4) && !m.at_war(C2, C4));
    assert_eq!(c.check_consistency(&m), Ok(()));
    assert_eq!(c.check_world(&w.data.model, &t), Ok(()));
    let w1 = c.war(wid(1)).unwrap();
    assert_eq!((w1.attacker.primary, w1.defender.primary, w1.target_state, w1.start_day), (C2, C1, s(22), 0));
    // 支持国は代表国と別に持つ(B の戦争6: 攻撃側 1 + 支持国 2)
    let (_, cb, mb) = w.load(SC_B);
    let w6 = cb.war(wid(6)).unwrap();
    assert_eq!((w6.attacker.primary, w6.attacker.supporters.clone(), w6.defender.primary), (C1, vec![C2], C4));
    assert_eq!(cb.hostile_pairs(), pairs(&[(1, 2), (1, 4), (2, 3), (2, 4), (3, 4)]));
    assert_eq!(cb.check_consistency(&mb), Ok(()));
    // 記録を持たない従来のシナリオ(wars だけ)は従来どおり: 導出された組と従来の組が同じで、sync は何も変えない
    for path in [LEGACY_TERR, LEGACY_BATTLE] {
        let sc = load_division_scenario(path, &w.graph, &w.data.model).unwrap();
        assert!(sc.war_records.is_empty());
        let legacy_pairs = sc.wars.clone();
        let conflicts = sc.conflicts();
        assert_eq!(conflicts.wars().count(), 0);
        assert_eq!(conflicts.legacy_pairs().len(), legacy_pairs.len());
        let mut m = sc.into_military();
        let before = m.clone();
        conflicts.sync(&mut m);
        assert_eq!(m, before, "{path}: the legacy hostility is unchanged by sync");
        assert_eq!(conflicts.check_consistency(&m), Ok(()));
        for (a, b) in &legacy_pairs {
            assert!(m.at_war(*a, *b));
        }
    }
}

#[test]
fn invalid_records_are_refused() {
    let side = |p: usize, sup: &[usize]| WarSide::new(CountryId(p), sup.iter().map(|x| CountryId(*x)));
    let rec = |id: u32, a: WarSide, d: WarSide| WarRecord { id: wid(id), attacker: a, defender: d, target_state: s(20), start_day: 0, end: None };
    assert_eq!(Conflicts::new([rec(1, side(1, &[]), side(2, &[])), rec(1, side(3, &[]), side(4, &[]))], []).unwrap_err(), ConflictError::DuplicateWar(wid(1)));
    assert_eq!(Conflicts::new([rec(1, side(1, &[]), side(1, &[]))], []).unwrap_err(), ConflictError::SameCountryBothSides { war: wid(1), country: C1 });
    assert_eq!(Conflicts::new([rec(1, side(1, &[3]), side(2, &[3]))], []).unwrap_err(), ConflictError::CountryOnBothSides { war: wid(1), country: C3 });
    assert_eq!(Conflicts::new([rec(1, side(1, &[]), side(2, &[]))], [(C2, C1)]).unwrap_err(), ConflictError::DuplicatesLegacyPair { war: wid(1), a: C1, b: C2 });
    assert_eq!(Conflicts::new([], [(C1, C1)]).unwrap_err(), ConflictError::SelfWar(C1));
    // 支持国の重複・代表国の混入は正規化される
    let sd = WarSide::new(C1, [C3, C2, C3]);
    assert_eq!(sd.supporters, vec![C2, C3]);
    // シナリオ: 未知のステート・未解決のラベル・構造の不正は入力不正として全件報告
    let w = world();
    let text = std::fs::read_to_string(SC_A).unwrap().replace("(id: 4, attacker: 4, defender: 3, target_state: 20)", "(id: 4, attacker: 4, defender: 9, target_state: 99)");
    let file = parse_division_scenario(&text).unwrap();
    let issues = resolve_division_scenario(&file, &w.graph, &w.data.model).unwrap_err();
    assert!(issues.iter().any(|i| i.code() == "war-record-unknown-state"), "{issues:?}");
    assert!(issues.iter().any(|i| i.code() == "unresolved-country"), "{issues:?}");
    let text = std::fs::read_to_string(SC_A).unwrap().replace("(id: 4, attacker: 4, defender: 3, target_state: 20)", "(id: 1, attacker: 4, defender: 3, target_state: 20)");
    let file = parse_division_scenario(&text).unwrap();
    let issues = resolve_division_scenario(&file, &w.graph, &w.data.model).unwrap_err();
    assert!(issues.iter().any(|i| i.code() == "war-record-invalid"), "{issues:?}");
}

// ---------------------------------------------------------------- 白紙講和と休戦

#[test]
fn white_peace_ends_the_war_dissolves_only_that_hostility_and_sets_the_1825_day_truce_between_the_primaries() {
    let w = world();
    let mut st = w.load(SC_A);
    st.2.advance_day(&w.graph); // 到達結果をキャッシュ
    let o = w.end(&mut st, &EndWarRequest::white_peace(wid(1), 1));
    assert_eq!(o.ended, vec![EndedWar { war: wid(1), result: WarResult::WhitePeace, reason: EndReason::WhitePeace }]);
    assert!(o.transfer.is_none() && o.supporters_removed.is_empty());
    assert_eq!(o.hostility_ended, vec![(C1, C2)]);
    assert!(o.hostility_kept.is_empty());
    // 領土は動かない
    assert_eq!(st.0.owners().collect::<Vec<_>>(), vec![(s(20), C1), (s(21), C2), (s(22), C1)]);
    // 占領の正常化: 国家1 が占領していた国家2 所有のステート21 の 3 プロヴィンスを法的所有国へ
    assert_eq!(
        o.occupation_normalized,
        vec![
            ControlChange { province: p(17696), previous: Some(C1), new: Some(C2) },
            ControlChange { province: p(17905), previous: Some(C1), new: Some(C2) },
            ControlChange { province: p(18112), previous: Some(C1), new: Some(C2) },
        ]
    );
    for pr in [17696, 17905, 18112] {
        assert_eq!(st.2.control().controller(p(pr)), Some(C2));
    }
    // 師団11(国家1、17905)は帰還。移転地ではないステート22 内の師団 2・3・14 は自国の領土なので残る
    assert_eq!(o.returned.len(), 1);
    assert_eq!((o.returned[0].division, o.returned[0].owner, o.returned[0].from), (md(11), C1, p(17905)));
    assert_eq!(st.2.control().controller(o.returned[0].to), Some(C1));
    for d in [2, 3, 14] {
        assert!(w.data.model.provinces_of(s(22)).unwrap().contains(&st.2.division(md(d)).unwrap().province), "division {d} stays");
    }
    assert!(o.disbanded.is_empty());
    // 交戦関係は導出で更新され、別の戦争の組は残る。同じ目標の戦争2 は割譲がないので維持される
    assert_eq!(st.1.hostile_pairs(), pairs(&[(1, 3), (3, 4)]));
    assert!(!st.2.at_war(C1, C2) && st.2.at_war(C1, C3));
    assert_eq!(st.1.check_consistency(&st.2), Ok(()));
    assert!(st.1.war(wid(2)).unwrap().is_active() && st.1.war(wid(3)).unwrap().is_active() && st.1.war(wid(4)).unwrap().is_active());
    // 終結の記録
    assert_eq!(st.1.war(wid(1)).unwrap().end, Some(WarEnd { day: 1, result: WarResult::WhitePeace, reason: EndReason::WhitePeace }));
    // 休戦: 代表国間で 1825 日。終了日 1826(その日から切れる)
    assert_eq!(o.truce, Some(TruceSet { a: C1, b: C2, until: 1826 }));
    assert_eq!(st.1.truce_until(C2, C1), Some(1826));
    assert!(st.1.in_truce(C1, C2, 1) && st.1.in_truce(C2, C1, 1825) && !st.1.in_truce(C1, C2, 1826) && !st.1.in_truce(C1, C2, 5000));
    assert!(!st.1.in_truce(C1, C3, 1), "no truce with a country that was not a primary of the ended war");
    // 補給: 変更前後の国と敵対が解けた組の国が無効化され、第三国(国家3)はそのまま
    assert_eq!(o.invalidated_countries, vec![C1, C2]);
    assert!(!st.2.reach_is_cached(C1) && !st.2.reach_is_cached(C2) && st.2.reach_is_cached(C3));
    let ev = st.2.advance_day(&w.graph);
    let r1 = ev.iter().find_map(|e| if let MoveEvent::DailySupply(r) = e { (r.country == C1).then(|| r.clone()) } else { None }).expect("supply of country 1");
    assert!(r1.reach_recomputed);
}

#[test]
fn a_truce_is_only_between_the_primaries_and_the_boundary_matches_the_old_path() {
    let w = world();
    let (t, _, m) = w.load(SC_A);
    let side = |p: usize, sup: &[usize]| WarSide::new(CountryId(p), sup.iter().map(|x| CountryId(*x)));
    let rec = WarRecord { id: wid(1), attacker: side(2, &[4]), defender: side(1, &[3]), target_state: s(22), start_day: 0, end: None };
    let c = Conflicts::new([rec], []).unwrap();
    let mut mm = m.clone();
    c.sync(&mut mm);
    let mut st: State = (t, c, mm);
    let o = w.end(&mut st, &EndWarRequest::white_peace(wid(1), 40));
    assert_eq!(o.truce, Some(TruceSet { a: C1, b: C2, until: 40 + TRUCE_DAYS }));
    assert_eq!(st.1.truces().collect::<Vec<_>>(), vec![(C1, C2, 1865)]);
    assert!(st.1.hostile_pairs().is_empty());
    // 旧経路との比較: 旧は終結日から 1825 日後(`add_days(1825)`)を休戦の終了日とし、その日から切れる(`is_at_least`)。新も終了日 = 終結日 + 1825 で、前日まで休戦・終了日から切れる
    assert_eq!(TRUCE_DAYS, 1825);
    let start = GameDate::from_string("1936/01/01").unwrap();
    let until = start.add_days(1825);
    let mut d = DiplomacyRegistry::default();
    d.set_truce(C1, C2, until.display());
    let day_before = start.add_days(1824).display();
    assert!(d.is_in_truce(C1, C2, &day_before) && !d.is_in_truce(C1, C2, &until.display()) && !d.is_in_truce(C1, C2, &start.add_days(1826).display()));
    assert!(st.1.in_truce(C1, C2, 40 + 1824) && !st.1.in_truce(C1, C2, 40 + 1825) && !st.1.in_truce(C1, C2, 40 + 1826));
}

// ---------------------------------------------------------------- 割譲・同一目標の別戦争・別の戦争で続く敵対

#[test]
fn cession_expires_the_other_war_with_the_same_target_keeps_the_hostility_other_wars_hold_and_leaves_unrelated_wars() {
    let w = world();
    let mut st = w.load(SC_A);
    let o = w.end(&mut st, &EndWarRequest::cession(wid(1), 7));
    assert_eq!(
        o.ended,
        vec![
            EndedWar { war: wid(1), result: WarResult::AttackerVictory, reason: EndReason::Cession },
            EndedWar { war: wid(2), result: WarResult::WhitePeace, reason: EndReason::TargetOwnerChanged },
        ]
    );
    assert_eq!(st.1.war(wid(2)).unwrap().end, Some(WarEnd { day: 7, result: WarResult::WhitePeace, reason: EndReason::TargetOwnerChanged }));
    assert!(st.1.war(wid(3)).unwrap().is_active() && st.1.war(wid(4)).unwrap().is_active(), "different target / unrelated war");
    // 割譲は M7-a の移転(所有・支配・首都・第三国の占領・帰還)
    let t = o.transfer.as_ref().unwrap();
    assert_eq!((t.state, t.previous_owner, t.new_owner), (s(22), C1, C2));
    assert_eq!(st.0.owner(s(22)), Some(C2));
    assert_eq!(t.capital_move, Some(CapitalMove { country: C1, from: p(17266), to: p(18109) }));
    assert_eq!(t.relocated, vec![Relocation { division: md(17), owner: C3, from: p(17484), to: p(17903) }]);
    assert_eq!(st.0.check_consistency(&st.2), Ok(()));
    // 1-2 の敵対は解け、1-3 は戦争3(と戦争2)で続くので維持
    assert_eq!(o.hostility_ended, vec![(C1, C2)]);
    assert_eq!(o.hostility_kept, vec![(C1, C3)]);
    assert!(st.2.at_war(C1, C3) && !st.2.at_war(C1, C2) && st.2.at_war(C3, C4));
    assert_eq!(st.1.hostile_pairs(), pairs(&[(1, 3), (3, 4)]));
    // 敵対が解けたので、移転地(ステート22)に残っていた国家1 の師団と、ステート21 にいた師団11 は帰還
    let returned: Vec<MapDivisionId> = o.returned.iter().map(|r| r.division).collect();
    assert_eq!(returned, vec![md(2), md(3), md(11), md(14)]);
    for r in &o.returned {
        assert_eq!(st.2.control().controller(r.to), Some(C1));
        let d = st.2.division(r.division).unwrap();
        assert_eq!((d.province, d.status, d.destination, d.path.len()), (r.to, MapDivisionStatus::Idle, None, 0));
    }
    // 占領の正常化(ステート21 の国家1 の占領)
    assert_eq!(o.occupation_normalized.len(), 3);
    // 休戦は代表国 2-1 の間だけ(失効した戦争2 の代表国 3-1 には休戦を作らない)
    assert_eq!(st.1.truces().collect::<Vec<_>>(), vec![(C1, C2, 7 + TRUCE_DAYS)]);
    assert_eq!(st.1.check_consistency(&st.2), Ok(()));
}

#[test]
fn a_hostility_that_another_war_keeps_keeps_its_occupation_and_its_battle_unless_named_in_the_transferred_state() {
    let w = world();
    // 国家1 の師団3 が 17484(ステート22。国家3 が占領。国家3 の師団17 がいる)へ攻撃して戦闘にする(1-3 の敵対は戦争2・3 で続く)
    let mut st = w.load(SC_A);
    st.2.issue_move_order(&w.graph, md(3), p(17484)).unwrap();
    until_battle(&mut st.2, &w.graph, p(17484));
    let b = st.2.ongoing_battle_at(p(17484)).unwrap().id;
    // 白紙講和(1-2 だけ解ける): 戦闘・占領は維持される
    let mut white = (st.0.clone(), st.1.clone(), st.2.clone());
    let o = w.end(&mut white, &EndWarRequest::white_peace(wid(1), 9));
    assert!(o.cancelled_battles.is_empty() && o.all_cancelled_battles().is_empty());
    assert!(white.2.ongoing_battle_at(p(17484)).is_some());
    assert_eq!(white.2.control().controller(p(17484)), Some(C3), "the occupation kept by the other war stays");
    assert_eq!(white.2.division(md(17)).unwrap().province, p(17484));
    // 割譲はステート22 内の戦闘を明示しないと拒否(何も変えない)
    let before = (st.0.clone(), st.1.clone(), st.2.clone());
    let e = plan_end_war(w.ctx(), &st.0, &st.1, &st.2, &EndWarRequest::cession(wid(1), 9)).unwrap_err();
    assert_eq!(e, EndWarError::Transfer(TransferError::BattleInTransferredState { battle: b, province: p(17484) }));
    assert!(end_war(w.ctx(), &mut st.0, &mut st.1, &mut st.2, &EndWarRequest::cession(wid(1), 9)).is_err());
    assert_unchanged(&before, &st);
    // 明示すれば中止して割譲(1-3 の敵対は続く。勝敗・占領は起きない)
    let o = w.end(&mut st, &EndWarRequest::cession(wid(1), 9).with_cancel_battles([b]));
    assert_eq!(o.all_cancelled_battles().len(), 1);
    assert!(st.2.battles().next().is_none());
    assert!(st.2.at_war(C1, C3));
}

#[test]
fn battles_of_a_dissolved_hostility_are_cancelled_without_a_result_and_other_battles_are_kept() {
    let w = world();
    // 国家1 の師団14 が 17486(ステート21、国家2 の領土)を攻撃して戦闘(1-2)
    let mut st = w.load(SC_A);
    st.2.issue_move_order(&w.graph, md(14), p(17486)).unwrap();
    until_battle(&mut st.2, &w.graph, p(17486));
    let b = st.2.ongoing_battle_at(p(17486)).unwrap().id;
    let combat14 = st.2.division(md(14)).unwrap().combat;
    let controller = st.2.control().controller(p(17486));
    // 白紙講和: 戦闘は中止(移転の外)。勝敗・占領なし、参加師団は待機、戦力は不変
    let mut white = (st.0.clone(), st.1.clone(), st.2.clone());
    let o = w.end(&mut white, &EndWarRequest::white_peace(wid(1), 9));
    assert_eq!(o.cancelled_battles.len(), 1);
    assert_eq!((o.cancelled_battles[0].battle, o.cancelled_battles[0].province), (b, p(17486)));
    assert!(white.2.battles().next().is_none());
    assert_eq!(white.2.control().controller(p(17486)), controller);
    for d in [14, 15] {
        let div = white.2.division(md(d)).unwrap();
        assert_eq!((div.status, div.battle), (MapDivisionStatus::Idle, None));
    }
    assert_eq!(white.2.division(md(14)).unwrap().combat, combat14);
    // 割譲(ステート22。戦闘はステート21): 戦闘は移転の中で中止される(同じ結果)
    let o = w.end(&mut st, &EndWarRequest::cession(wid(1), 9));
    assert_eq!(o.all_cancelled_battles().len(), 1);
    assert!(o.cancelled_battles.is_empty() && o.transfer.as_ref().unwrap().cancelled_battles.len() == 1);
    assert!(st.2.battles().next().is_none());
    // 別の組の戦闘は維持: 国家3 の師団17 を攻撃する国家1 の戦闘が、戦争1 の白紙講和では残る
    let mut other = w.load(SC_A);
    other.2.issue_move_order(&w.graph, md(3), p(17484)).unwrap();
    until_battle(&mut other.2, &w.graph, p(17484));
    w.end(&mut other, &EndWarRequest::white_peace(wid(1), 9));
    assert!(other.2.ongoing_battle_at(p(17484)).is_some(), "an unrelated battle is kept");
}

// ---------------------------------------------------------------- 国家消滅による決着

#[test]
fn eliminating_a_primary_decides_its_other_wars_for_the_opposing_side_and_a_supporter_is_only_removed() {
    let w = world();
    let mut st = w.load(SC_B);
    let o = w.end(&mut st, &EndWarRequest::cession(wid(8), 3));
    assert_eq!(
        o.ended,
        vec![
            EndedWar { war: wid(8), result: WarResult::AttackerVictory, reason: EndReason::Cession },
            EndedWar { war: wid(5), result: WarResult::DefenderVictory, reason: EndReason::AttackerEliminated },
            EndedWar { war: wid(7), result: WarResult::AttackerVictory, reason: EndReason::DefenderEliminated },
        ],
        "the war of the cession, then the eliminated primary's wars by id (a victory for the opposing side, not a white peace)"
    );
    // 戦争7 は目標(ステート21)が割譲されるステートでもあるが、消滅による決着が優先(白紙講和の失効ではない)
    assert_eq!(st.1.war(wid(7)).unwrap().end.unwrap().reason, EndReason::DefenderEliminated);
    assert_eq!(st.1.war(wid(5)).unwrap().end, Some(WarEnd { day: 3, result: WarResult::DefenderVictory, reason: EndReason::AttackerEliminated }));
    // 支持国としてのみ参加する戦争6 は継続し、国家2 は陣営から除かれる
    assert_eq!(o.supporters_removed, vec![SupporterRemoved { war: wid(6), country: C2 }]);
    let w6 = st.1.war(wid(6)).unwrap();
    assert!(w6.is_active() && w6.attacker.supporters.is_empty() && w6.attacker.primary == C1);
    // 無関係な戦争4 は維持
    assert!(st.1.war(wid(4)).unwrap().is_active());
    // 国家2 は消滅(ID は保持)。交戦関係は導出: 1-4(戦争6)と 3-4(戦争4)だけが残る
    assert_eq!(st.0.status(C2), Some(CountryStatus::Eliminated { annexed_by: C1 }));
    assert_eq!(st.1.hostile_pairs(), pairs(&[(1, 4), (3, 4)]));
    assert_eq!(o.hostility_ended, vec![(C1, C2), (C2, C3), (C2, C4)]);
    assert!(o.hostility_kept.is_empty());
    assert_eq!(st.1.check_consistency(&st.2), Ok(()));
    assert_eq!(st.0.check_consistency(&st.2), Ok(()));
    // 休戦は消滅した国との間には残らない
    assert_eq!(o.truce, None);
    assert_eq!(st.1.truce_until(C1, C2), None);
    // 消滅国の師団は解散(帰還や撃破ではない)
    let t = o.transfer.as_ref().unwrap();
    assert!(t.disbanded.iter().filter(|d| d.reason == DisbandReason::CountryEliminated).count() >= 5);
    assert!(st.2.divisions().all(|d| d.owner != C2));
}

#[test]
fn an_eliminated_supporter_is_removed_even_from_a_war_that_the_cession_expires_and_a_decision_by_elimination_wins() {
    let w = world();
    let (t, _, m) = w.load(SC_B);
    let side = |p: usize, sup: &[usize]| WarSide::new(CountryId(p), sup.iter().map(|x| CountryId(*x)));
    let rec = |id: u32, a: WarSide, d: WarSide, target: u32| WarRecord { id: wid(id), attacker: a, defender: d, target_state: s(target), start_day: 0, end: None };
    // 戦争9: 攻撃側 3(支持国 2)対 防御側 4、目標ステート21(割譲されるステート)→ 支持国 2 は除かれ、目標の所有国が変わるので白紙講和で失効
    // 戦争8: 攻撃側 1 対 防御側 2、目標ステート21(割譲)
    let c = Conflicts::new([rec(8, side(1, &[]), side(2, &[]), 21), rec(9, side(3, &[2]), side(4, &[]), 21)], []).unwrap();
    let mut mm = m.clone();
    c.sync(&mut mm);
    let mut st: State = (t, c, mm);
    let o = w.end(&mut st, &EndWarRequest::cession(wid(8), 2));
    assert_eq!(o.supporters_removed, vec![SupporterRemoved { war: wid(9), country: C2 }]);
    assert_eq!(o.ended[1], EndedWar { war: wid(9), result: WarResult::WhitePeace, reason: EndReason::TargetOwnerChanged });
    assert_eq!(st.1.war(wid(9)).unwrap().attacker.supporters, Vec::<CountryId>::new());
    assert!(st.1.hostile_pairs().is_empty());
}

// ---------------------------------------------------------------- 拒否時の状態保持・二重適用・古い計画

#[test]
fn every_rejected_request_leaves_everything_exactly_as_before() {
    let w = world();
    let st = w.load(SC_A);
    let attempt = |st: &State, req: &EndWarRequest| -> EndWarError {
        let e = plan_end_war(w.ctx(), &st.0, &st.1, &st.2, req).unwrap_err();
        let mut copy = (st.0.clone(), st.1.clone(), st.2.clone());
        assert!(end_war(w.ctx(), &mut copy.0, &mut copy.1, &mut copy.2, req).is_err());
        assert_unchanged(st, &copy);
        e
    };
    assert_eq!(attempt(&st, &EndWarRequest::white_peace(wid(9), 1)), EndWarError::UnknownWar(wid(9)));
    // 目標の所有国が防御側でない戦争4(目標20 は国家1 の所有で、防御側は 3)の割譲
    assert_eq!(attempt(&st, &EndWarRequest::cession(wid(4), 1)), EndWarError::Transfer(TransferError::OwnerMismatch { state: s(20), expected: C3, actual: Some(C1) }));
    // 開始日より前
    let (t, _, m) = w.load(SC_A);
    let late = WarRecord { id: wid(1), attacker: WarSide::new(C2, []), defender: WarSide::new(C1, []), target_state: s(22), start_day: 10, end: None };
    let c = Conflicts::new([late], []).unwrap();
    let mut mm = m.clone();
    c.sync(&mut mm);
    let st2: State = (t, c, mm);
    assert_eq!(attempt(&st2, &EndWarRequest::white_peace(wid(1), 5)), EndWarError::DayBeforeStart { war: wid(1), start_day: 10, day: 5 });
    // 記録を経由しない移転(消滅)で交戦関係がずれた状態
    let mut st3 = w.load(SC_B);
    transfer_state(w.ctx(), &mut st3.0, &mut st3.2, &TransferRequest::new(s(21), C2, C1)).unwrap();
    assert!(st3.1.check_consistency(&st3.2).is_err());
    assert_eq!(attempt(&st3, &EndWarRequest::white_peace(wid(4), 1)), EndWarError::HostilityOutOfSync);
}

#[test]
fn an_ended_war_is_not_applied_twice_and_a_stale_plan_is_refused() {
    let w = world();
    let mut st = w.load(SC_A);
    let plan = plan_end_war(w.ctx(), &st.0, &st.1, &st.2, &EndWarRequest::cession(wid(1), 7)).unwrap();
    // 計画は状態を変えない
    assert_eq!(st.0.owner(s(22)), Some(C1));
    let o = apply_end_war(w.ctx(), &mut st.0, &mut st.1, &mut st.2, &plan).unwrap();
    assert_eq!(&o, plan.outcome());
    let after_first = (st.0.clone(), st.1.clone(), st.2.clone());
    // 同じ計画の再適用・終結済みの戦争への再要求(別の日・別の条件でも)は、割譲も休戦も二重に適用しない
    let end = st.1.war(wid(1)).unwrap().end.unwrap();
    assert_eq!(apply_end_war(w.ctx(), &mut st.0, &mut st.1, &mut st.2, &plan), Err(EndWarApplyError::AlreadyEnded { war: wid(1), end }));
    for req in [EndWarRequest::cession(wid(1), 100), EndWarRequest::white_peace(wid(1), 100)] {
        assert_eq!(plan_end_war(w.ctx(), &st.0, &st.1, &st.2, &req).unwrap_err(), EndWarError::AlreadyEnded { war: wid(1), end });
        assert_eq!(end_war(w.ctx(), &mut st.0, &mut st.1, &mut st.2, &req), Err(EndWarApplyError::AlreadyEnded { war: wid(1), end }));
    }
    assert_unchanged(&after_first, &st);
    assert_eq!(st.1.truce_until(C1, C2), Some(7 + TRUCE_DAYS), "the truce is not extended by the re-request");
    assert_eq!(st.0.revision(), 1, "the transfer was applied once");
    // 失効した戦争2 への要求も同じ(終結済み)
    assert!(matches!(plan_end_war(w.ctx(), &st.0, &st.1, &st.2, &EndWarRequest::white_peace(wid(2), 8)), Err(EndWarError::AlreadyEnded { .. })));
    // 古い計画: 戦争1 の割譲の計画(戦争2 の失効を含む)のあとに、戦争2 が別に白紙講和で終わると、結果が計画と違う
    let mut st2 = w.load(SC_A);
    let plan = plan_end_war(w.ctx(), &st2.0, &st2.1, &st2.2, &EndWarRequest::cession(wid(1), 7)).unwrap();
    w.end(&mut st2, &EndWarRequest::white_peace(wid(2), 5));
    let before = (st2.0.clone(), st2.1.clone(), st2.2.clone());
    assert_eq!(apply_end_war(w.ctx(), &mut st2.0, &mut st2.1, &mut st2.2, &plan), Err(EndWarApplyError::Stale(EndWarStale::OutcomeChanged)));
    assert_unchanged(&before, &st2);
    // 古い計画: 所有国が変わって要求が成り立たない(戦争1 の割譲の計画のあとに、戦争1 とは別にステート22 が国家3 へ移った)
    let mut st3 = w.load(SC_A);
    let plan = plan_end_war(w.ctx(), &st3.0, &st3.1, &st3.2, &EndWarRequest::cession(wid(1), 7)).unwrap();
    transfer_state(w.ctx(), &mut st3.0, &mut st3.2, &TransferRequest::new(s(22), C1, C3)).unwrap();
    let before = (st3.0.clone(), st3.1.clone(), st3.2.clone());
    match apply_end_war(w.ctx(), &mut st3.0, &mut st3.1, &mut st3.2, &plan) {
        Err(EndWarApplyError::Stale(EndWarStale::Invalid(EndWarError::Transfer(TransferError::OwnerMismatch { .. })))) => {}
        other => panic!("{other:?}"),
    }
    assert_unchanged(&before, &st3);
}

// ---------------------------------------------------------------- 命令・補給・決定性

#[test]
fn returned_divisions_lose_their_orders_while_other_orders_are_left_to_the_daily_path_check() {
    let w = world();
    let mut st = w.load(SC_A);
    // 師団11(国家1、17905)に自国の別の地点への命令、師団1(国家1)にも命令を出しておく
    st.2.issue_move_order(&w.graph, md(11), p(18112)).unwrap();
    st.2.issue_move_order(&w.graph, md(1), p(18310)).ok();
    let d1_before = st.2.division(md(1)).cloned().unwrap();
    assert!(st.2.division(md(11)).unwrap().destination.is_some());
    let o = w.end(&mut st, &EndWarRequest::white_peace(wid(1), 1));
    assert!(o.returned.iter().any(|r| r.division == md(11)));
    let d11 = st.2.division(md(11)).unwrap();
    assert_eq!((d11.destination, d11.target, d11.path.len(), d11.status), (None, None, 0, MapDivisionStatus::Idle), "the returned division's order is discarded");
    // 帰還しなかった師団の命令は触らない(次の advance_day の既存の経路検証が再判定する)
    assert_eq!(st.2.division(md(1)).unwrap(), &d1_before);
    st.2.advance_day(&w.graph);
}

#[test]
fn the_result_does_not_depend_on_the_input_order() {
    let w = world();
    for (path, war, cede) in [(SC_A, 1u32, true), (SC_A, 1, false), (SC_B, 8, true)] {
        let sc = load_division_scenario(path, &w.graph, &w.data.model).unwrap();
        let mut reversed = sc.clone();
        reversed.war_records.reverse();
        reversed.wars.reverse();
        reversed.divisions.reverse();
        let req = if cede { EndWarRequest::cession(wid(war), 4) } else { EndWarRequest::white_peace(wid(war), 4) };
        let mut a = w.build(sc);
        let mut b = w.build(reversed);
        let oa = w.end(&mut a, &req);
        let ob = w.end(&mut b, &req);
        assert_eq!(oa, ob, "{path} war {war}");
        assert_eq!(a.0, b.0);
        assert_eq!(a.1, b.1);
        assert_eq!(a.2.divisions().cloned().collect::<Vec<_>>(), b.2.divisions().cloned().collect::<Vec<_>>());
        assert_eq!(a.2.control().iter().collect::<Vec<_>>(), b.2.control().iter().collect::<Vec<_>>());
    }
    // 中止指定の順序と重複は計画に影響しない
    let mut st = w.load(SC_A);
    st.2.issue_move_order(&w.graph, md(3), p(17484)).unwrap();
    until_battle(&mut st.2, &w.graph, p(17484));
    let b = st.2.ongoing_battle_at(p(17484)).unwrap().id;
    let p1 = plan_end_war(w.ctx(), &st.0, &st.1, &st.2, &EndWarRequest::cession(wid(1), 9).with_cancel_battles([b, b])).unwrap();
    let p2 = plan_end_war(w.ctx(), &st.0, &st.1, &st.2, &EndWarRequest::cession(wid(1), 9).with_cancel_battles([b])).unwrap();
    assert_eq!(p1, p2);
}

#[test]
fn the_ended_state_is_deterministic_across_repeated_runs() {
    let w = world();
    let run = || {
        let mut st = w.load(SC_B);
        let o = w.end(&mut st, &EndWarRequest::cession(wid(8), 1));
        for _ in 0..3 {
            st.2.advance_day(&w.graph);
        }
        (o, st.0, st.1, st.2)
    };
    let a = run();
    let b = run();
    assert_eq!(a.0, b.0);
    assert_eq!((a.1, a.2, a.3), (b.1, b.2, b.3));
}
