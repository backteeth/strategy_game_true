//! P32-043 (M7-c): 戦争点数・降伏判定(`world::war_score`)の検証。判定結果を返すところまで(講和の適用は M7-d)。
//!
//! 合成した所属表(`summarize_control_membership`)で占領率の境界を、検証用シナリオ A(`division_war_scenario.ron`)で戦闘の集計と終結済みの除外を、
//! 旧型(`war::war_score` / `war::capitulation`)で全域支配だけのケースの一致と意図した差を確かめる。保存・アセットは書かない。

use std::collections::{BTreeMap, HashSet};

use strategy_game::common::{CountryId, DivisionDefinitionId, DivisionId, StateId, WarId};
use strategy_game::hexmap::data::load_from_path;
use strategy_game::military::data::{Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry};
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::world::*;

const SC_A: &str = "assets/data/world/preview/division_war_scenario.ron";

fn c(n: usize) -> CountryId {
    CountryId(n)
}
fn s(n: u32) -> MapStateId {
    MapStateId(n)
}

// ---- 合成した所属表 ----

/// `(ステート, 法的所有国, 所属総数, [(支配国, 件数)...])`。残りは支配国なし。プロヴィンス ID はステートごとに連番。
type Spec<'a> = (u32, usize, usize, &'a [(usize, usize)]);

fn summary(spec: &[Spec]) -> StateControlSummary {
    let mut next = 1u32;
    let mut members: Vec<(MapStateId, Vec<ProvinceId>)> = Vec::new();
    let mut owners = BTreeMap::new();
    let mut control = BTreeMap::new();
    for (state, owner, total, ctrl) in spec {
        let ids: Vec<ProvinceId> = (0..*total).map(|_| { let p = ProvinceId(next); next += 1; p }).collect();
        owners.insert(MapStateId(*state), CountryId(*owner));
        let mut i = 0;
        for (cc, n) in *ctrl {
            for _ in 0..*n {
                control.insert(ids[i], CountryId(*cc));
                i += 1;
            }
        }
        assert!(i <= *total, "more controlled provinces than members");
        members.push((MapStateId(*state), ids));
    }
    let slices: Vec<(MapStateId, &[ProvinceId])> = members.iter().map(|(st, v)| (*st, v.as_slice())).collect();
    summarize_control_membership(&slices, &[], &owners, &ProvinceControl::new(control)).unwrap()
}

fn military(divs: &[(u32, usize, u64)]) -> MapMilitary {
    let ds: Vec<MapDivision> = divs
        .iter()
        .map(|(id, owner, manpower)| {
            let mut cs = CombatStats::standard();
            cs.manpower = *manpower;
            MapDivision::new(MapDivisionId(*id), CountryId(*owner), ProvinceId(1), 1.0).with_combat(cs)
        })
        .collect();
    MapMilitary::new(ds, ProvinceControl::new(BTreeMap::new()), MovementParams { base_days: 5.0 }).unwrap()
}

fn war(id: u32, attacker: usize, defender: usize, target: u32) -> WarRecord {
    WarRecord { id: MapWarId(id), attacker: WarSide::new(c(attacker), []), defender: WarSide::new(c(defender), []), target_state: s(target), start_day: 1, end: None }
}

fn assess(spec: &[Spec], divs: &[(u32, usize, u64)], wins: (u32, u32)) -> WarAssessment {
    let sm = summary(spec);
    let w = war(1, 1, 2, 10);
    let mut ledger = BattleLedger::new();
    let conflicts = Conflicts::new([w.clone()], []).unwrap();
    let mut n = 1;
    for _ in 0..wins.0 {
        ledger.record(&conflicts, MapBattleId(n), c(1), c(2), BattleOutcome::AttackerWon);
        n += 1;
    }
    for _ in 0..wins.1 {
        ledger.record(&conflicts, MapBattleId(n), c(1), c(2), BattleOutcome::DefenderWon);
        n += 1;
    }
    assess_with_summary(&w, &sm, &military(divs), &ledger).unwrap()
}

const BOTH_ALIVE: &[(u32, usize, u64)] = &[(1, 1, 100), (2, 2, 100)];

// ---- 占領率(S 案) ----

#[test]
fn occupation_rate_is_the_mean_of_per_state_shares_not_the_pooled_province_share() {
    // 国家2 の所有: ステート10(13 プロヴィンス)とステート11(7 プロヴィンス)。
    // 13 側を全部取る場合と、7 側を全部取る場合は、どちらも平均 1/2(プロヴィンスの合計比なら 65% と 35% で結果が分かれる)。
    let big = summary(&[(10, 2, 13, &[(1, 13)]), (11, 2, 7, &[])]);
    let small = summary(&[(10, 2, 13, &[]), (11, 2, 7, &[(1, 7)])]);
    let rb = occupation_rate(&big, c(1), c(2)).unwrap();
    let rs = occupation_rate(&small, c(1), c(2)).unwrap();
    assert_eq!(rb.rate, Some(Ratio { num: 1, den: 2 }));
    assert_eq!(rs.rate, Some(Ratio { num: 1, den: 2 }));
    assert_eq!((rb.points().unwrap(), rs.points().unwrap()), (20, 20));
    assert!(!rb.reaches_surrender_rate().unwrap() && !rs.reaches_surrender_rate().unwrap(), "50% < 60% even though 13/20 = 65% of the provinces are held");
    // 入力(所属表)の並びを変えても同じ
    let swapped = summary(&[(11, 2, 7, &[]), (10, 2, 13, &[(1, 13)])]);
    assert_eq!(occupation_rate(&swapped, c(1), c(2)).unwrap().rate, rb.rate);
}

#[test]
fn thirteen_to_seven_example_scores_floor_of_each_side_and_subtracts() {
    // 20 プロヴィンスのステート: 攻撃側が防御側の領土の 13/20 = 65%(⌊26⌋ = 26)、防御側が攻撃側の領土の 7/20 = 35%(⌊14⌋ = 14)。目標(ステート10)は全域支配ではない。
    let a = assess(&[(10, 2, 20, &[(1, 13), (2, 7)]), (20, 1, 20, &[(2, 7)])], BOTH_ALIVE, (0, 0));
    let b = &a.breakdown;
    assert_eq!(b.attacker_occupation.rate, Some(Ratio { num: 13, den: 20 }));
    assert_eq!(b.defender_occupation.rate, Some(Ratio { num: 7, den: 20 }));
    assert_eq!((b.war_goal_score, b.attacker_occupation_score, b.defender_occupation_score), (0, 26, 14));
    assert_eq!((b.raw_total, b.total_score), (12, 12));
    assert!(!b.target_held);
    assert_eq!(a.verdict, Verdict::None, "the target is not fully held: no defender surrender even at 65%");
}

#[test]
fn sixty_percent_uses_the_unrounded_value_just_below_and_exactly_at_the_threshold() {
    // 1000 プロヴィンス: 599(59.9%)は不成立、600(60%)は成立。占領点は ⌊23.96⌋ = 23 と 24。
    for (n, reaches, pts) in [(599usize, false, 23), (600, true, 24)] {
        let sm = summary(&[(10, 2, 1000, &[(1, n)])]);
        let r = occupation_rate(&sm, c(1), c(2)).unwrap();
        assert_eq!(r.reaches_surrender_rate().unwrap(), reaches, "{n}/1000");
        assert_eq!(r.points().unwrap(), pts, "{n}/1000");
    }
    // 3/5 はちょうど 60%(浮動小数の比較に頼らない)。2/5 は不成立
    assert!(occupation_rate(&summary(&[(10, 2, 5, &[(1, 3)])]), c(1), c(2)).unwrap().reaches_surrender_rate().unwrap());
    assert!(!occupation_rate(&summary(&[(10, 2, 5, &[(1, 2)])]), c(1), c(2)).unwrap().reaches_surrender_rate().unwrap());
    // 複数ステートの平均でも境界は厳密: (1/1 + 1/5 + 0/1... ) ではなく (3/5 + 3/5)/2 = 3/5 ちょうど、(3/5 + 59/100)/2 = 119/200 = 59.5% は不成立
    let exact = summary(&[(10, 2, 5, &[(1, 3)]), (11, 2, 5, &[(1, 3)])]);
    assert!(occupation_rate(&exact, c(1), c(2)).unwrap().reaches_surrender_rate().unwrap());
    let below = summary(&[(10, 2, 5, &[(1, 3)]), (11, 2, 100, &[(1, 59)])]);
    let r = occupation_rate(&below, c(1), c(2)).unwrap();
    assert_eq!(r.rate, Some(Ratio { num: 119, den: 200 }));
    assert!(!r.reaches_surrender_rate().unwrap());
    // 途中で丸めると変わる例: 1/3 + 1/3 + 1/3 の平均は 1/3(0.33 で丸めた和 0.99 にはならない)。3 ステート全部 1/3 なら ちょうど 1/3
    let thirds = summary(&[(10, 2, 3, &[(1, 1)]), (11, 2, 3, &[(1, 1)]), (12, 2, 3, &[(1, 1)])]);
    assert_eq!(occupation_rate(&thirds, c(1), c(2)).unwrap().rate, Some(Ratio { num: 1, den: 3 }));
    // 全ステートを全域支配すれば 100%(⌊40⌋ = 40)
    let all = summary(&[(10, 2, 3, &[(1, 3)]), (11, 2, 7, &[(1, 7)])]);
    assert_eq!(occupation_rate(&all, c(1), c(2)).unwrap().points().unwrap(), 40);
}

#[test]
fn third_countries_supporters_and_uncontrolled_provinces_stay_in_the_denominator_only() {
    // 10 プロヴィンス: 攻撃側代表国 4、第三国(3)が 2、支持国(5)が 1、支配国なし 3。分子は代表国の 4 だけ。
    let sm = summary(&[(10, 2, 10, &[(1, 4), (3, 2), (5, 1)])]);
    let r = occupation_rate(&sm, c(1), c(2)).unwrap();
    assert_eq!(r.rate, Some(Ratio { num: 2, den: 5 }));
    assert_eq!(r.states, vec![StateOccupation { state: s(10), controlled: 4, total: 10 }]);
    assert_eq!(r.points().unwrap(), 16);
}

#[test]
fn states_without_provinces_and_other_owners_are_excluded_and_no_target_states_gives_an_undefined_rate() {
    // 所属 0 件のステートと、第三国所有のステートは平均に入らない
    let sm = summary(&[(10, 2, 4, &[(1, 4)]), (11, 2, 0, &[]), (12, 3, 9, &[(1, 9)])]);
    let r = occupation_rate(&sm, c(1), c(2)).unwrap();
    assert_eq!(r.states.len(), 1);
    assert_eq!(r.rate, Some(Ratio { num: 1, den: 1 }));
    // 相手の代表国の所有ステートが 0 件(または所属 0 件だけ): 定義しない。点は 0、60% は不成立
    let none = summary(&[(11, 2, 0, &[]), (12, 3, 9, &[(1, 9)])]);
    let r = occupation_rate(&none, c(1), c(2)).unwrap();
    assert_eq!((r.rate, r.states.len()), (None, 0));
    assert_eq!(r.points().unwrap(), 0);
    assert!(!r.reaches_surrender_rate().unwrap());
}

// ---- 点数 ----

#[test]
fn score_adds_target_and_occupation_and_battle_points_with_caps() {
    // 目標(ステート10、6 プロヴィンス)を全域支配 +40、防御側の所有は 10 と 11(7)で 11 は 0 → 平均 1/2 → +20、攻撃側の自領 20(5)を防御側が 2/5 → ⌊16⌋ = −16
    let spec: &[Spec] = &[(10, 2, 6, &[(1, 6)]), (11, 2, 7, &[]), (20, 1, 5, &[(2, 2)])];
    let a = assess(spec, BOTH_ALIVE, (3, 1));
    let b = &a.breakdown;
    assert!(b.target_held);
    assert_eq!((b.war_goal_score, b.attacker_occupation_score, b.defender_occupation_score), (40, 20, 16));
    assert_eq!((b.attacker_battle_wins, b.defender_battle_wins, b.battle_score), (3, 1, 10));
    assert_eq!((b.raw_total, b.total_score), (54, 54));
    // 戦闘点は ±20 に収める(勝利数の差 7 → 35 → 20、逆は −20)
    assert_eq!(assess(spec, BOTH_ALIVE, (7, 0)).breakdown.battle_score, 20);
    assert_eq!(assess(spec, BOTH_ALIVE, (0, 7)).breakdown.battle_score, -20);
    // 総点は ±100 に収める: 目標 40 + 占領 40 + 戦闘 20 = 100 ちょうど。収める前の合計も出る
    let full = assess(&[(10, 2, 6, &[(1, 6)]), (20, 1, 5, &[])], BOTH_ALIVE, (9, 0));
    assert_eq!((full.breakdown.raw_total, full.breakdown.total_score), (100, 100));
    // 負の側: 目標なし・占領 −40・戦闘 −20 = −60(収まる範囲)
    let neg = assess(&[(10, 2, 6, &[]), (20, 1, 5, &[(2, 5)])], BOTH_ALIVE, (0, 9));
    assert_eq!((neg.breakdown.raw_total, neg.breakdown.total_score), (-60, -60));
}

#[test]
fn score_is_a_pure_function_of_the_inputs_not_of_their_order() {
    let a = assess(&[(10, 2, 6, &[(1, 6)]), (11, 2, 7, &[(1, 1)]), (20, 1, 5, &[(2, 2)])], BOTH_ALIVE, (2, 1));
    let b = assess(&[(20, 1, 5, &[(2, 2)]), (11, 2, 7, &[(1, 1)]), (10, 2, 6, &[(1, 6)])], &[(2, 2, 100), (1, 1, 100)], (2, 1));
    assert_eq!(a, b);
}

// ---- 戦闘の集計 ----

fn conflicts_of(records: Vec<WarRecord>, legacy: Vec<(CountryId, CountryId)>) -> Conflicts {
    Conflicts::new(records, legacy).unwrap()
}

#[test]
fn a_battle_is_counted_once_even_if_it_is_reported_again() {
    let cf = conflicts_of(vec![war(1, 1, 2, 10)], vec![]);
    let mut l = BattleLedger::new();
    let first = l.record(&cf, MapBattleId(7), c(1), c(2), BattleOutcome::AttackerWon);
    assert_eq!(first, LedgerEntry::Counted { battle: MapBattleId(7), war: MapWarId(1), attacker_side_won: true });
    assert_eq!(l.record(&cf, MapBattleId(7), c(1), c(2), BattleOutcome::AttackerWon), LedgerEntry::Duplicate { battle: MapBattleId(7) });
    // 結果が違う再報告でも二重に数えない
    assert_eq!(l.record(&cf, MapBattleId(7), c(1), c(2), BattleOutcome::DefenderWon), LedgerEntry::Duplicate { battle: MapBattleId(7) });
    assert_eq!(l.wins(MapWarId(1)), (1, 0));
}

#[test]
fn the_winner_decides_the_side_whoever_attacked_in_the_battle() {
    // 戦争1: 攻撃側 2(支持国 5)対 防御側 1。国家1 が国家5 を攻撃して防御側(国家5)が勝つ = 戦争の攻撃側の勝利。
    let w = WarRecord { id: MapWarId(1), attacker: WarSide::new(c(2), [c(5)]), defender: WarSide::new(c(1), []), target_state: s(10), start_day: 1, end: None };
    let cf = conflicts_of(vec![w], vec![]);
    let mut l = BattleLedger::new();
    assert_eq!(l.record(&cf, MapBattleId(1), c(1), c(5), BattleOutcome::DefenderWon), LedgerEntry::Counted { battle: MapBattleId(1), war: MapWarId(1), attacker_side_won: true });
    // 国家5(支持国)が国家1 を攻撃して勝つ = 攻撃側の勝利、負ける = 防御側の勝利
    assert_eq!(l.record(&cf, MapBattleId(2), c(5), c(1), BattleOutcome::AttackerWon), LedgerEntry::Counted { battle: MapBattleId(2), war: MapWarId(1), attacker_side_won: true });
    assert_eq!(l.record(&cf, MapBattleId(3), c(5), c(1), BattleOutcome::DefenderWon), LedgerEntry::Counted { battle: MapBattleId(3), war: MapWarId(1), attacker_side_won: false });
    assert_eq!(l.wins(MapWarId(1)), (2, 1));
}

#[test]
fn a_battle_with_several_candidate_wars_is_not_counted_and_the_reason_is_kept() {
    // 戦争1・2 とも 2(攻撃側)対 1(防御側)。国家2 と国家1 の戦闘はどちらの戦争か決められない。
    let cf = conflicts_of(vec![war(1, 2, 1, 10), war(2, 2, 1, 11), war(3, 3, 1, 10)], vec![]);
    let mut l = BattleLedger::new();
    let e = l.record(&cf, MapBattleId(1), c(2), c(1), BattleOutcome::AttackerWon);
    assert_eq!(e, LedgerEntry::Skipped { battle: MapBattleId(1), reason: SkipReason::Ambiguous(vec![MapWarId(1), MapWarId(2)]) });
    assert_eq!((l.wins(MapWarId(1)), l.wins(MapWarId(2))), ((0, 0), (0, 0)));
    // 3 対 1 は戦争3 だけ → 一意
    assert!(matches!(l.record(&cf, MapBattleId(2), c(3), c(1), BattleOutcome::AttackerWon), LedgerEntry::Counted { war: MapWarId(3), .. }));
    // スキップした戦闘は、あとで候補が 1 件になっても再計上しない
    assert_eq!(l.record(&cf, MapBattleId(1), c(2), c(1), BattleOutcome::AttackerWon), LedgerEntry::Duplicate { battle: MapBattleId(1) });
    assert_eq!(l.skipped().len(), 1);
}

#[test]
fn legacy_pair_and_ended_war_battles_are_not_counted() {
    // 記録のない従来の組(3-4)の戦闘は計上しない。終結した戦争も候補にならない。
    let mut ended = war(2, 5, 6, 10);
    ended.end = Some(WarEnd { day: 3, result: WarResult::WhitePeace, reason: EndReason::WhitePeace });
    let cf = conflicts_of(vec![war(1, 1, 2, 10), ended], vec![(c(3), c(4))]);
    let mut l = BattleLedger::new();
    assert_eq!(l.record(&cf, MapBattleId(1), c(3), c(4), BattleOutcome::AttackerWon), LedgerEntry::Skipped { battle: MapBattleId(1), reason: SkipReason::NoWar });
    assert_eq!(l.record(&cf, MapBattleId(2), c(5), c(6), BattleOutcome::AttackerWon), LedgerEntry::Skipped { battle: MapBattleId(2), reason: SkipReason::NoWar });
}

struct World {
    data: WorldPreviewData,
    graph: ProvinceGraph,
}

fn world() -> World {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let data = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).unwrap();
    let graph = ProvinceGraph::build(&data.model, &map).unwrap();
    World { data, graph }
}

impl World {
    fn load(&self, path: &str) -> (Territory, Conflicts, MapMilitary) {
        let sc = load_division_scenario(path, &self.graph, &self.data.model).unwrap();
        let t = Territory::from_scenario(&sc, &self.data.model, &self.graph).unwrap();
        let cf = sc.conflicts();
        let mut m = sc.into_military();
        cf.sync(&mut m);
        (t, cf, m)
    }
}

#[test]
fn real_battle_events_are_counted_once_per_finished_battle_and_never_double_counted() {
    let w = world();
    let (_t, cf, mut m) = w.load(SC_A);
    // 国家1 の師団14 が 17486(国家2 の領土)を攻撃: 戦闘 1-2(戦争1 が一意)
    m.issue_move_order(&w.graph, MapDivisionId(14), ProvinceId(17486)).unwrap();
    let mut ledger = BattleLedger::new();
    let mut finished = 0usize;
    let mut all_entries = Vec::new();
    for _ in 0..60 {
        let events = m.advance_day(&w.graph);
        finished += events.iter().filter(|e| matches!(e, MoveEvent::BattleFinished { .. })).count();
        let entries = ledger.record_events(&cf, &events);
        // 同じ日のイベントを再度渡しても、すべて Duplicate(二重に数えない)
        let again = ledger.record_events(&cf, &events);
        assert!(again.iter().all(|e| matches!(e, LedgerEntry::Duplicate { .. })));
        assert_eq!(entries.len(), again.len());
        all_entries.extend(entries);
    }
    assert!(finished >= 1, "the scenario must produce a finished battle");
    assert_eq!(all_entries.len(), finished, "one ledger entry per finished battle");
    let counted = all_entries.iter().filter(|e| matches!(e, LedgerEntry::Counted { .. })).count();
    let (aw, dw) = ledger.wins(MapWarId(1));
    assert!(counted >= 1);
    let others: u32 = [2, 3, 4].iter().map(|i| { let (a, d) = ledger.wins(MapWarId(*i)); a + d }).sum();
    assert_eq!(aw + dw + others, counted as u32);
}

#[test]
fn a_cancelled_battle_and_an_undefended_occupation_are_not_battle_wins() {
    let w = world();
    let (mut t, mut cf, mut m) = w.load(SC_A);
    m.issue_move_order(&w.graph, MapDivisionId(14), ProvinceId(17486)).unwrap();
    for _ in 0..40 {
        if m.ongoing_battle_at(ProvinceId(17486)).is_some() {
            break;
        }
        m.advance_day(&w.graph);
    }
    let b = m.ongoing_battle_at(ProvinceId(17486)).expect("a battle").id;
    let ctx = TransferContext { model: &w.data.model, graph: &w.graph };
    let o = end_war(ctx, &mut t, &mut cf, &mut m, &EndWarRequest::white_peace(MapWarId(1), 9)).unwrap();
    assert!(o.cancelled_battles.iter().any(|c| c.battle == b));
    // 中止した戦闘は BattleFinished を出さない → 台帳に入らない
    let mut ledger = BattleLedger::new();
    for _ in 0..30 {
        let ev = m.advance_day(&w.graph);
        assert!(!ev.iter().any(|e| matches!(e, MoveEvent::BattleFinished { battle, .. } if *battle == b)));
        for e in ledger.record_events(&cf, &ev) {
            assert!(!matches!(e, LedgerEntry::Counted { battle, .. } if battle == b));
        }
    }
    // 守備隊なしの占領(ControlClaim)だけのイベントは数えない
    let claim_only = vec![MoveEvent::ControlClaim { province: ProvinceId(1), claimant: c(1), previous_controller: None, reason: ClaimReason::Undefended, divisions: vec![] }];
    assert!(BattleLedger::new().record_events(&cf, &claim_only).is_empty());
}

// ---- 降伏 ----

#[test]
fn defender_surrenders_when_the_target_is_fully_held_and_the_rate_or_the_army_condition_holds() {
    // 目標(10)を全域支配。防御側は 10 と 11(13)で 11 は取られていない → 1/2 = 50%、防御側に師団あり → 降伏しない
    let held: &[Spec] = &[(10, 2, 6, &[(1, 6)]), (11, 2, 13, &[]), (20, 1, 5, &[])];
    let a = assess(held, BOTH_ALIVE, (0, 0));
    assert_eq!(a.verdict, Verdict::None);
    assert!(a.defender_reasons.is_empty() && a.attacker_reasons.is_empty());
    // 占領率 60% 以上(11 も全域: 100%)→ 降伏
    let a = assess(&[(10, 2, 6, &[(1, 6)]), (11, 2, 13, &[(1, 13)]), (20, 1, 5, &[])], BOTH_ALIVE, (0, 0));
    assert_eq!((a.verdict.clone(), a.defender_reasons.clone()), (Verdict::DefenderSurrenders, vec![DefenderReason::TargetHeldAndOccupation]));
    // 防御側代表国に戦闘可能な師団がない(人員 0 の師団だけ、または師団なし)→ 降伏。理由は師団なし
    for divs in [&[(1u32, 1usize, 100u64), (2, 2, 0)][..], &[(1, 1, 100)][..]] {
        let a = assess(held, divs, (0, 0));
        assert_eq!((a.verdict.clone(), a.defender_reasons.clone()), (Verdict::DefenderSurrenders, vec![DefenderReason::TargetHeldAndNoCombatReadyDivisions]));
        assert!(!a.defender_combat_ready);
    }
    // 目標を全域支配していなければ、占領率 100% でも師団がなくても降伏しない(目標の 1 プロヴィンスだけ欠ける)
    let a = assess(&[(10, 2, 6, &[(1, 5)]), (11, 2, 13, &[(1, 13)]), (20, 1, 5, &[])], &[(1, 1, 100)], (0, 0));
    assert!(!a.breakdown.target_held);
    assert_eq!(a.verdict, Verdict::None);
}

#[test]
fn attacker_surrenders_on_its_own_occupation_rate_or_with_no_army_and_a_state_fully_held() {
    // 自領(20: 5、21: 5)の被占領率 60%: 3/5 と 3/5 → 平均 60% ちょうど → 降伏
    let a = assess(&[(10, 2, 6, &[]), (20, 1, 5, &[(2, 3)]), (21, 1, 5, &[(2, 3)])], BOTH_ALIVE, (0, 0));
    assert_eq!((a.verdict.clone(), a.attacker_reasons.clone()), (Verdict::AttackerSurrenders, vec![AttackerReason::OccupationRate]));
    // 59%(3/5 と 59/100)は不成立
    let a = assess(&[(10, 2, 6, &[]), (20, 1, 5, &[(2, 3)]), (21, 1, 100, &[(2, 59)])], BOTH_ALIVE, (0, 0));
    assert_eq!(a.verdict, Verdict::None);
    // 攻撃側に戦闘可能な師団がなく、自領のステートを 1 件全域支配された(占領率は 50% で足りない)
    let spec: &[Spec] = &[(10, 2, 6, &[]), (20, 1, 5, &[(2, 5)]), (21, 1, 5, &[])];
    let a = assess(spec, &[(2, 2, 100)], (0, 0));
    assert_eq!(a.verdict, Verdict::AttackerSurrenders);
    assert_eq!(a.attacker_reasons, vec![AttackerReason::NoCombatReadyDivisionsAndStateHeld(vec![s(20)])]);
    // 師団があれば(占領率 50%)降伏しない。師団がなくても全域支配でなく占領率も低ければ(2/5)降伏しない
    assert_eq!(assess(spec, BOTH_ALIVE, (0, 0)).verdict, Verdict::None);
    assert_eq!(assess(&[(10, 2, 6, &[]), (20, 1, 5, &[(2, 2)])], &[(2, 2, 100)], (0, 0)).verdict, Verdict::None);
}

#[test]
fn when_both_sides_meet_a_condition_the_score_decides_and_zero_goes_to_the_attacker() {
    // 防御側: 目標を全域支配 + 防御側に師団なし。攻撃側: 自領 A(20)を防御側が全域支配 → 被占領率 100%。
    // 防御側の所有は 10(目標)と 11 で 11 は 0 → 攻撃側の占領率 1/2 → +20。総点 = 40 + 20 − 40 + 戦闘点。
    let spec: &[Spec] = &[(10, 2, 6, &[(1, 6)]), (11, 2, 7, &[]), (20, 1, 5, &[(2, 5)])];
    let divs: &[(u32, usize, u64)] = &[(1, 1, 100)];
    // 戦闘なし: 総点 20 > 0 → 防御側が降伏
    let a = assess(spec, divs, (0, 0));
    assert!(a.both_met_tie_break);
    assert_eq!(a.breakdown.total_score, 20);
    assert_eq!(a.verdict, Verdict::DefenderSurrenders);
    // 防御側が 4 勝(−20)→ 総点 0: 0 は攻撃側が降伏
    let a = assess(spec, divs, (0, 4));
    assert_eq!(a.breakdown.total_score, 0);
    assert_eq!(a.verdict, Verdict::AttackerSurrenders);
    // 防御側がさらに勝っても(上限 −20)総点は 0 のまま → 攻撃側。攻撃側が 1 勝(+5)なら 25 > 0 → 防御側
    assert_eq!(assess(spec, divs, (1, 0)).breakdown.total_score, 25);
    assert_eq!(assess(spec, divs, (1, 0)).verdict, Verdict::DefenderSurrenders);
    // 総点が負になる例: 目標なし条件では防御側の条件が成立しないので別の組み立て(防御側の条件を師団なしで成立させ、攻撃側の被占領を重くする)
    let spec2: &[Spec] = &[(10, 2, 6, &[(1, 6)]), (11, 2, 7, &[]), (11 + 10, 1, 5, &[(2, 5)]), (22, 1, 5, &[(2, 5)])];
    let a = assess(spec2, divs, (0, 4));
    assert_eq!(a.breakdown.defender_occupation_score, 40);
    assert_eq!(a.breakdown.total_score, 0);
    assert_eq!(a.verdict, Verdict::AttackerSurrenders);
}

#[test]
fn an_undefined_rate_is_not_a_surrender_unlike_the_old_zero_state_rule() {
    // 攻撃側が陸ステートを 1 件も持たない(新: 占領率は定義せず、降伏条件は成立しない。旧: 0 件なら攻撃側が降伏)。
    let a = assess(&[(10, 2, 6, &[(1, 2)])], BOTH_ALIVE, (0, 0));
    assert_eq!(a.breakdown.defender_occupation.rate, None);
    assert_eq!(a.verdict, Verdict::None);
}

#[test]
fn ended_wars_are_not_judged_and_active_wars_are_listed_in_id_order() {
    let w = world();
    let (mut t, mut cf, mut m) = w.load(SC_A);
    let ledger = BattleLedger::new();
    let before = assess_active_wars(&cf, &w.data.model, &t, &m, &ledger);
    assert_eq!(before.iter().map(|r| r.as_ref().unwrap().breakdown.war).collect::<Vec<_>>(), (1..=4).map(MapWarId).collect::<Vec<_>>());
    let ctx = TransferContext { model: &w.data.model, graph: &w.graph };
    end_war(ctx, &mut t, &mut cf, &mut m, &EndWarRequest::white_peace(MapWarId(1), 5)).unwrap();
    let after = assess_active_wars(&cf, &w.data.model, &t, &m, &ledger);
    assert_eq!(after.iter().map(|r| r.as_ref().unwrap().breakdown.war).collect::<Vec<_>>(), (2..=4).map(MapWarId).collect::<Vec<_>>());
    let ended = cf.war(MapWarId(1)).unwrap();
    assert_eq!(assess_war(ended, &w.data.model, &t, &m, &ledger), Err(ScoreError::AlreadyEnded(MapWarId(1))));
    assert_eq!(war_score(ended, &w.data.model, &t, &m, &ledger), Err(ScoreError::AlreadyEnded(MapWarId(1))));
}

#[test]
fn assessment_is_read_only_and_repeatable() {
    let w = world();
    let (t, cf, m) = w.load(SC_A);
    let (t0, cf0, m0) = (t.clone(), cf.clone(), m.clone());
    let ledger = BattleLedger::new();
    let a = assess_active_wars(&cf, &w.data.model, &t, &m, &ledger);
    let b = assess_active_wars(&cf, &w.data.model, &t, &m, &ledger);
    assert_eq!(a, b);
    assert_eq!((t, cf, m), (t0, cf0, m0), "judging changes nothing (no peace is applied in M7-c)");
}

// ---- 旧経路との比較(全域支配だけのケース) ----

fn old_war(attacker: usize, defender: usize, target: u32) -> strategy_game::war::data::War {
    use strategy_game::diplomacy::crisis::{WarGoal, WarGoalType};
    strategy_game::war::data::War {
        id: WarId(1),
        name: "old".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attackers: [CountryId(attacker)].into_iter().collect(),
        defenders: [CountryId(defender)].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![WarGoal {
            attacker: CountryId(attacker),
            defender: CountryId(defender),
            goal_type: WarGoalType::ConquerState,
            target_states: vec![StateId(target as usize)],
            base_peace_cost: 20.0,
            international_concern: 10.0,
            completion: 0.0,
            is_primary: true,
        }],
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: HashSet::new(),
        status: strategy_game::war::data::WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: HashSet::new(),
    }
}

fn old_division(id: usize, owner: usize) -> Division {
    Division {
        id: DivisionId(id),
        owner: CountryId(owner),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(1),
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 5000,
        max_manpower: 10000,
        equipment: 50.0,
        max_equipment: 100.0,
        organization: 50.0,
        max_organization: 100.0,
        morale: 50.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: DivisionDefinitionId(1),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

/// 旧と新に同じ「全域支配だけ」の状況を入れ、点数の内訳と降伏判定を比べる。
/// `states` = (ステート, 所有国, 全域の支配国。新は所属数 3 のステートで全域/全くなし)。`divisions` = 師団を持つ国。
fn compare(states: &[(u32, usize, usize)], divisions: &[usize], target: u32, battles: (u32, u32)) {
    use strategy_game::war::capitulation::{CapitulationResult, evaluate_war_capitulation};
    use strategy_game::war::war_score::calculate_war_score;
    let new_spec: Vec<Spec> = states.iter().map(|(st, owner, ctrl)| (*st, *owner, 3usize, if *ctrl == 0 { &[][..] } else { Box::leak(Box::new([(*ctrl, 3usize)])) as &[(usize, usize)] })).collect();
    let sm = summary(&new_spec);
    let divs_new: Vec<(u32, usize, u64)> = divisions.iter().enumerate().map(|(i, o)| (i as u32 + 1, *o, 100)).collect();
    let nw = war(1, 1, 2, target);
    let cf = Conflicts::new([nw.clone()], []).unwrap();
    let mut ledger = BattleLedger::new();
    let mut n = 1;
    for _ in 0..battles.0 {
        ledger.record(&cf, MapBattleId(n), c(1), c(2), BattleOutcome::AttackerWon);
        n += 1;
    }
    for _ in 0..battles.1 {
        ledger.record(&cf, MapBattleId(n), c(1), c(2), BattleOutcome::DefenderWon);
        n += 1;
    }
    let new = assess_with_summary(&nw, &sm, &military(&divs_new), &ledger).unwrap();

    let old_states: Vec<StateData> = states
        .iter()
        .map(|(st, owner, ctrl)| StateData { id: StateId(*st as usize), owner_country_id: CountryId(*owner), controller_country: Some(CountryId(if *ctrl == 0 { *owner } else { *ctrl })), ..Default::default() })
        .collect();
    let reg = StateRegistry::build(old_states);
    let mut ow = old_war(1, 2, target);
    ow.won_attacker_battles = battles.0 as u64 as _;
    ow.won_defender_battles = battles.1 as u64 as _;
    let old = calculate_war_score(&ow, &reg);
    let mut mil = MilitaryRegistry::default();
    for (i, o) in divisions.iter().enumerate() {
        let d = old_division(i + 1, *o);
        mil.divisions.insert(d.id, d);
    }
    ow.war_score = old.total_score as f32;
    let old_res = evaluate_war_capitulation(&ow, &reg, &mil);

    let b = &new.breakdown;
    assert_eq!(
        (b.war_goal_score, b.attacker_occupation_score, -b.defender_occupation_score, b.battle_score, b.total_score),
        (old.war_goal_score, old.attacker_occupation_score, old.defender_occupation_score, old.battle_score, old.total_score),
        "score breakdown {states:?} divisions {divisions:?}"
    );
    let new_res = match new.verdict {
        Verdict::None => CapitulationResult::None,
        Verdict::DefenderSurrenders => CapitulationResult::DefenderCapitulated,
        Verdict::AttackerSurrenders => CapitulationResult::AttackerCapitulated,
    };
    assert_eq!(new_res, old_res, "verdict {states:?} divisions {divisions:?}");
}

#[test]
fn full_control_only_cases_match_the_old_path_for_score_and_surrender() {
    // 国家1(攻撃側)が 22 を所有、国家2(防御側)が 20・21 を所有。支配は全域か全くなし(旧のステート単位の支配と同じ粒度)。
    // (ステート, 所有国, 全域支配国。0 = 所有国のまま)
    let cases: Vec<Vec<(u32, usize, usize)>> = vec![
        vec![(20, 2, 0), (21, 2, 0), (22, 1, 0)],
        vec![(20, 2, 0), (21, 2, 1), (22, 1, 0)],
        vec![(20, 2, 1), (21, 2, 1), (22, 1, 0)],
        vec![(20, 2, 1), (21, 2, 0), (22, 1, 0)],
        vec![(20, 2, 0), (21, 2, 0), (22, 1, 2)],
        vec![(20, 2, 1), (21, 2, 0), (22, 1, 2)],
        vec![(20, 2, 1), (21, 2, 1), (22, 1, 2)],
    ];
    for states in &cases {
        for divisions in [&[1usize, 2][..], &[1][..], &[2][..]] {
            for battles in [(0u32, 0u32), (3, 1), (0, 7), (9, 0)] {
                compare(states, divisions, 21, battles);
            }
        }
    }
}

#[test]
fn intended_differences_from_the_old_path() {
    // 1) 部分占領: 旧はステートの支配国が 1 つなので 7 プロヴィンス中 1 つの占領は表現できない。新は 1/7 を占領率に入れる。
    let a = assess(&[(10, 2, 7, &[(1, 1)])], BOTH_ALIVE, (0, 0));
    assert_eq!(a.breakdown.attacker_occupation.rate, Some(Ratio { num: 1, den: 7 }));
    assert_eq!(a.breakdown.attacker_occupation_score, 5, "floor(40/7) = 5; the old path would give 0 (the state is not controlled)");
    // 2) 陸ステートが 0 件: 旧は降伏(攻撃側: 自領 0 件、防御側: 目標を押さえていれば 0 件で降伏)。新は占領率を定義せず、降伏条件にしない。
    use strategy_game::war::capitulation::{CapitulationResult, evaluate_war_capitulation};
    let reg = StateRegistry::build(vec![StateData { id: StateId(21), owner_country_id: CountryId(2), controller_country: Some(CountryId(2)), ..Default::default() }]);
    let mut mil = MilitaryRegistry::default();
    for (i, o) in [1usize, 2].iter().enumerate() {
        let d = old_division(i + 1, *o);
        mil.divisions.insert(d.id, d);
    }
    let ow = old_war(1, 2, 21);
    assert_eq!(evaluate_war_capitulation(&ow, &reg, &mil), CapitulationResult::AttackerCapitulated, "old: the attacker owns no land state, so it capitulates");
    let new = assess(&[(10, 2, 6, &[])], BOTH_ALIVE, (0, 0));
    assert_eq!(new.verdict, Verdict::None, "new: the attacker's rate is undefined and not a surrender condition");
    // 3) 支持国の支配は占領に入らない(旧の代表国基準と同じだが、新は分母に含めて明示)。第三国の支配は分母のみ。
    let a = assess(&[(10, 2, 10, &[(1, 4), (3, 3), (5, 3)])], BOTH_ALIVE, (0, 0));
    assert_eq!(a.breakdown.attacker_occupation.rate, Some(Ratio { num: 2, den: 5 }));
}
