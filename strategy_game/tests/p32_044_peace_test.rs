//! P32-044 (M7-d): 講和の提案・受諾(`world::peace`)と、日次の自動講和の検証。
//!
//! 検証用シナリオ A(`division_war_scenario.ron`)・B(`division_war_elimination_scenario.ron`)の所有・支配・戦争は検証入力で、正式な世界データではない。
//! 点数の境界は支配を `set_controller` で作り、戦闘の勝敗は `BattleLedger` に入れて組み立てる。拒否の検証は、拒否の前後で状態(Territory・Conflicts・MapMilitary)が一致することを確かめる。

use strategy_game::common::CountryId;
use strategy_game::hexmap::data::load_from_path;
use strategy_game::world::*;

const SC_A: &str = "assets/data/world/preview/division_war_scenario.ron";
const SC_B: &str = "assets/data/world/preview/division_war_elimination_scenario.ron";

const S20: [u32; 7] = [18109, 18308, 18309, 18310, 18311, 18499, 18500];
const S21: [u32; 13] = [17269, 17486, 17487, 17694, 17695, 17696, 17904, 17905, 17906, 18111, 18112, 18312, 18502];
const S22: [u32; 6] = [17266, 17267, 17268, 17484, 17485, 17693];

fn c(n: usize) -> CountryId {
    CountryId(n)
}
fn wid(n: u32) -> MapWarId {
    MapWarId(n)
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

type State = (Territory, Conflicts, MapMilitary);

impl World {
    fn ctx(&self) -> TransferContext<'_> {
        TransferContext { model: &self.data.model, graph: &self.graph }
    }
    fn scenario(&self, path: &str) -> DivisionScenario {
        load_division_scenario(path, &self.graph, &self.data.model).unwrap()
    }
    fn build(&self, sc: DivisionScenario) -> State {
        let t = Territory::from_scenario(&sc, &self.data.model, &self.graph).unwrap();
        let cf = sc.conflicts();
        let mut m = sc.into_military();
        cf.sync(&mut m);
        (t, cf, m)
    }
    fn load(&self, path: &str) -> State {
        let sc = self.scenario(path);
        self.build(sc)
    }
    fn accept(&self, st: &mut State, l: &BattleLedger, p: &PeaceProposal, who: usize, today: u32) -> Result<EndWarOutcome, PeaceError> {
        accept_peace(self.ctx(), &mut st.0, &mut st.1, &mut st.2, l, p, c(who), today)
    }
    fn check(&self, st: &State, l: &BattleLedger, p: &PeaceProposal, who: usize, today: u32) -> Result<Acceptance, PeaceError> {
        check_acceptance(self.ctx(), &st.0, &st.1, &st.2, l, p, c(who), today)
    }
    fn assess(&self, st: &State, l: &BattleLedger, war: u32) -> WarAssessment {
        assess_war(st.1.war(wid(war)).unwrap(), &self.data.model, &st.0, &st.2, l).unwrap()
    }
    fn auto(&self, st: &mut State, l: &mut BattleLedger, events: &[MoveEvent], day: u32) -> AutoPeaceReport {
        run_auto_peace(self.ctx(), &mut st.0, &mut st.1, &mut st.2, l, events, day)
    }
}

fn hold(m: &mut MapMilitary, provinces: &[u32], country: usize) {
    for p in provinces {
        m.set_controller(ProvinceId(*p), Some(c(country)));
    }
}

fn snapshot(st: &State) -> State {
    (st.0.clone(), st.1.clone(), st.2.clone())
}

fn assert_same(a: &State, b: &State, what: &str) {
    assert_eq!(a.0, b.0, "territory changed: {what}");
    assert_eq!(a.1, b.1, "conflicts changed: {what}");
    assert_eq!(a.2, b.2, "military changed: {what}");
}

/// 戦争1(攻撃側 2・防御側 1)の戦闘の勝敗を台帳へ入れる(攻撃側の勝利数, 防御側の勝利数)。
fn ledger_for_war1(cf: &Conflicts, wins: (u32, u32)) -> BattleLedger {
    let mut l = BattleLedger::new();
    let mut n = 1000;
    for _ in 0..wins.0 {
        l.record(cf, MapBattleId(n), c(2), c(1), BattleOutcome::AttackerWon);
        n += 1;
    }
    for _ in 0..wins.1 {
        l.record(cf, MapBattleId(n), c(2), c(1), BattleOutcome::DefenderWon);
        n += 1;
    }
    l
}

fn zero_manpower(sc: &mut DivisionScenario, owner: usize) {
    for d in sc.divisions.iter_mut().filter(|d| d.owner == c(owner)) {
        d.combat.manpower = 0;
    }
}

fn propose(st: &State, war: u32, proposer: usize, recipient: usize, terms: ProposalTerms, day: u32) -> PeaceProposal {
    propose_peace(&st.1, &st.0, wid(war), c(proposer), c(recipient), terms, day).unwrap()
}

// ---------------------------------------------------------------- 白紙講和

#[test]
fn white_peace_needs_thirty_days_from_the_start_29_rejected_30_accepted() {
    let w = world();
    let mut st = w.load(SC_A);
    let l = BattleLedger::new();
    let pr = propose(&st, 1, 1, 2, ProposalTerms::WhitePeace, 1);
    let before = snapshot(&st);
    assert_eq!(w.accept(&mut st, &l, &pr, 2, 29).unwrap_err(), PeaceError::WhitePeaceTooEarly { war_days: 29, required: 30 });
    assert_same(&st, &before, "29 days");
    assert!(w.check(&st, &l, &pr, 2, 29).is_err() && w.check(&st, &l, &pr, 2, 30).is_ok(), "the read-only query agrees");
    assert_same(&st, &before, "queries are read-only");
    let o = w.accept(&mut st, &l, &pr, 2, 30).unwrap();
    assert_eq!(o.ended[0], EndedWar { war: wid(1), result: WarResult::WhitePeace, reason: EndReason::WhitePeace });
    assert!(o.transfer.is_none(), "white peace moves no territory");
    assert_eq!(st.0.owner(MapStateId(22)), Some(c(1)));
    assert_eq!(st.1.war(wid(1)).unwrap().end, Some(WarEnd { day: 30, result: WarResult::WhitePeace, reason: EndReason::WhitePeace }));
    // 休戦: 代表国 1-2 間に終結日から 1825 日(1854 日目まで有効、1855 日目から切れる)
    assert_eq!(st.1.truce_until(c(1), c(2)), Some(30 + 1825));
    assert!(st.1.in_truce(c(1), c(2), 30 + 1824) && !st.1.in_truce(c(1), c(2), 30 + 1825));
}

#[test]
fn white_peace_score_minus_11_rejected_minus_10_accepted_and_the_viewpoint_is_shared_by_both_proposers() {
    let w = world();
    for proposer in [1usize, 2] {
        let recipient = 3 - proposer;
        // -11: 国家2 が 17266 を支配(+3)、国家1 は 21 の 3/13(−9)、防御側が 1 勝(−5)
        let mut st = w.load(SC_A);
        hold(&mut st.2, &[17266], 2);
        let l11 = ledger_for_war1(&st.1, (0, 1));
        assert_eq!(w.assess(&st, &l11, 1).breakdown.total_score, -11);
        let pr = propose(&st, 1, proposer, recipient, ProposalTerms::WhitePeace, 1);
        let before = snapshot(&st);
        assert_eq!(w.accept(&mut st, &l11, &pr, recipient, 30).unwrap_err(), PeaceError::WhitePeaceScoreTooLow { score: -11, required: -10 }, "proposer {proposer}");
        assert_same(&st, &before, "-11");
        // -10: 国家2 が 20 の 1・22 の 2 を支配(+9)、国家1 は 21 の 3/13(−9)、防御側が 2 勝(−10)
        let mut st = w.load(SC_A);
        hold(&mut st.2, &[18109, 17266, 17267], 2);
        let l10 = ledger_for_war1(&st.1, (0, 2));
        assert_eq!(w.assess(&st, &l10, 1).breakdown.total_score, -10);
        let pr = propose(&st, 1, proposer, recipient, ProposalTerms::WhitePeace, 1);
        assert!(w.accept(&mut st, &l10, &pr, recipient, 30).is_ok(), "proposer {proposer}");
    }
}

// ---------------------------------------------------------------- 割譲

#[test]
fn cession_total_59_rejected_60_accepted_and_the_territory_moves_with_the_capital() {
    let w = world();
    let l = BattleLedger::new();
    // 59: 国家2 が 22 を全域(+40)と 20 の 1(率 (1/7+1)/2 → +22)、国家1 は 21 の 1/13(−3)
    let mut st = w.load(SC_A);
    hold(&mut st.2, &S22, 2);
    hold(&mut st.2, &[18109], 2);
    hold(&mut st.2, &[17905, 18112], 2);
    let a = w.assess(&st, &l, 1);
    assert_eq!(a.breakdown.total_score, 59);
    assert!(a.defender_reasons.is_empty(), "the defender-alone condition is not met");
    let pr = propose(&st, 1, 2, 1, ProposalTerms::CedeTarget, 5);
    let before = snapshot(&st);
    assert_eq!(w.accept(&mut st, &l, &pr, 1, 5).unwrap_err(), PeaceError::CedeConditionsNotMet { score: 59, required: 60 });
    assert_same(&st, &before, "59");
    // 60: 国家2 が 22 を全域(+40)と 20 は 0(率 1/2 → +20)、国家1 は 21 を 0(0)
    let mut st = w.load(SC_A);
    hold(&mut st.2, &S22, 2);
    hold(&mut st.2, &[17696, 17905, 18112], 2);
    let a = w.assess(&st, &l, 1);
    assert_eq!(a.breakdown.total_score, 60);
    assert!(a.defender_reasons.is_empty());
    let pr = propose(&st, 1, 2, 1, ProposalTerms::CedeTarget, 5);
    let o = w.accept(&mut st, &l, &pr, 1, 5).unwrap();
    assert_eq!(o.ended[0], EndedWar { war: wid(1), result: WarResult::AttackerVictory, reason: EndReason::Cession }, "a proposal's cession is recorded as Cession, not as a surrender");
    assert_eq!(st.0.owner(MapStateId(22)), Some(c(2)));
    assert_eq!(st.0.capital(c(1)), Some(ProvinceId(18109)));
    assert_eq!(st.1.truce_until(c(1), c(2)), Some(5 + 1825));
}

#[test]
fn cession_is_accepted_by_the_defender_alone_condition_even_if_the_final_verdict_would_go_the_other_way() {
    let w = world();
    // 国家1 の師団をすべて戦闘不能に(防御側単独の降伏条件: 目標全域 + 戦闘可能な師団なし)
    let mut sc = w.scenario(SC_A);
    zero_manpower(&mut sc, 1);
    let mut st = w.build(sc);
    hold(&mut st.2, &S22, 2);
    let l = BattleLedger::new();
    let a = w.assess(&st, &l, 1);
    assert_eq!(a.breakdown.total_score, 51, "below 60");
    assert_eq!(a.defender_reasons, vec![DefenderReason::TargetHeldAndNoCombatReadyDivisions]);
    let pr = propose(&st, 1, 2, 1, ProposalTerms::CedeTarget, 3);
    assert!(w.check(&st, &l, &pr, 1, 3).is_ok());
    // 双方成立・総点 0: 国家1 が 21 を全域支配(攻撃側の条件が成立。−40)、防御側が 4 勝(−20)。最終裁定は攻撃側の降伏だが、割譲は防御側単独の条件で受諾できる
    let mut sc = w.scenario(SC_A);
    zero_manpower(&mut sc, 1);
    let mut st = w.build(sc);
    hold(&mut st.2, &S22, 2);
    hold(&mut st.2, &S21, 1);
    let l = ledger_for_war1(&st.1, (0, 4));
    let a = w.assess(&st, &l, 1);
    assert_eq!(a.breakdown.total_score, 0);
    assert!(a.both_met_tie_break);
    assert_eq!(a.verdict, Verdict::AttackerSurrenders, "the final verdict of a both-met war");
    assert_eq!(a.defender_reasons, vec![DefenderReason::TargetHeldAndNoCombatReadyDivisions], "the defender-alone condition is still reported on its own");
    let pr = propose(&st, 1, 2, 1, ProposalTerms::CedeTarget, 3);
    assert!(w.check(&st, &l, &pr, 1, 3).is_ok(), "the proposal's acceptance uses the defender-alone condition, not the final verdict");
}

#[test]
fn cession_is_rejected_when_the_defender_does_not_own_the_target() {
    let w = world();
    // 戦争3(攻撃側 3・防御側 1、目標 20)の目標の所有者を国家2 に変えた検証入力(所有者は防御側ではない)
    let mut sc = w.scenario(SC_A);
    sc.state_owners.insert(MapStateId(20), c(2));
    let st = w.build(sc);
    let pr = propose(&st, 3, 3, 1, ProposalTerms::CedeTarget, 1);
    let l = BattleLedger::new();
    assert_eq!(w.check(&st, &l, &pr, 1, 1).unwrap_err(), PeaceError::CedeNotTargetOwner { owner: Some(c(2)) });
}

// ---------------------------------------------------------------- 敗北受諾・権限・拒否

#[test]
fn defeat_acceptance_gives_the_defender_a_victory_without_moving_territory() {
    let w = world();
    let mut st = w.load(SC_A);
    let l = BattleLedger::new();
    let owners: Vec<_> = st.0.owners().collect();
    let pr = propose(&st, 1, 2, 1, ProposalTerms::AcceptDefeat, 2);
    let o = w.accept(&mut st, &l, &pr, 1, 2).unwrap();
    assert_eq!(o.ended[0], EndedWar { war: wid(1), result: WarResult::DefenderVictory, reason: EndReason::DefeatAccepted });
    assert!(o.transfer.is_none());
    assert_eq!(st.0.owners().collect::<Vec<_>>(), owners, "no territory moves");
    assert_eq!(st.1.truce_until(c(1), c(2)), Some(2 + 1825));
    assert!(!st.1.at_war(c(1), c(2)));
    // 占領は法的所有国へ戻る(国家1 が占領していたステート21 の 3 プロヴィンス)
    assert!(o.occupation_normalized.iter().any(|x| x.province == ProvinceId(17696) && x.new == Some(c(2))));
}

#[test]
fn unauthorized_unknown_ended_and_pre_start_proposals_are_rejected_without_changing_anything() {
    let w = world();
    let st = w.load(SC_A);
    let l = BattleLedger::new();
    let b = snapshot(&st);
    let p = |war, a, r, t, d| propose_peace(&st.1, &st.0, wid(war), c(a), c(r), t, d);
    // 防御側からの割譲・敗北の提案は権限がない
    assert_eq!(p(1, 1, 2, ProposalTerms::CedeTarget, 1).unwrap_err(), PeaceError::NotAuthorized { terms: ProposalTerms::CedeTarget, proposer: c(1) });
    assert_eq!(p(1, 1, 2, ProposalTerms::AcceptDefeat, 1).unwrap_err(), PeaceError::NotAuthorized { terms: ProposalTerms::AcceptDefeat, proposer: c(1) });
    // 第三国・戦争の当事者でない国
    assert!(matches!(p(1, 3, 1, ProposalTerms::WhitePeace, 1).unwrap_err(), PeaceError::NotOpposingPrimaries { .. }));
    assert!(matches!(p(1, 4, 2, ProposalTerms::WhitePeace, 1).unwrap_err(), PeaceError::NotOpposingPrimaries { .. }));
    // 同じ陣営どうしや同じ国
    assert!(matches!(p(2, 3, 3, ProposalTerms::WhitePeace, 1).unwrap_err(), PeaceError::NotOpposingPrimaries { .. }));
    // 未知の戦争
    assert_eq!(p(99, 2, 1, ProposalTerms::WhitePeace, 1).unwrap_err(), PeaceError::UnknownWar(wid(99)));
    // 受領国でない国は受諾できない
    let pr = propose(&st, 1, 2, 1, ProposalTerms::AcceptDefeat, 1);
    assert_eq!(w.check(&st, &l, &pr, 2, 1).unwrap_err(), PeaceError::NotRecipient { recipient: c(1), acceptor: c(2) });
    assert_eq!(w.check(&st, &l, &pr, 3, 1).unwrap_err(), PeaceError::NotRecipient { recipient: c(1), acceptor: c(3) });
    // 受諾の日が提案日より前
    let pr5 = propose(&st, 1, 2, 1, ProposalTerms::AcceptDefeat, 5);
    assert_eq!(w.check(&st, &l, &pr5, 1, 4).unwrap_err(), PeaceError::DayBeforeProposal { proposal_day: 5, today: 4 });
    assert_same(&st, &b, "read-only");
    // 開戦前: 開始日 10 の戦争
    let mut sc = w.scenario(SC_A);
    sc.war_records[0].start_day = 10;
    let late = w.build(sc);
    assert_eq!(propose_peace(&late.1, &late.0, wid(1), c(2), c(1), ProposalTerms::AcceptDefeat, 9).unwrap_err(), PeaceError::BeforeWarStart { war: wid(1), start_day: 10, day: 9 });
    let pr = propose(&late, 1, 2, 1, ProposalTerms::AcceptDefeat, 10);
    let mut late2 = snapshot(&late);
    assert!(matches!(w.accept(&mut late2, &l, &pr, 1, 9).unwrap_err(), PeaceError::BeforeWarStart { .. } | PeaceError::DayBeforeProposal { .. }));
    assert_same(&late2, &late, "before start");
}

#[test]
fn a_proposal_whose_premise_lapsed_is_rejected_and_a_double_acceptance_does_not_apply_twice() {
    let w = world();
    // 割譲: 60 で提案し、国家2 が 1 プロヴィンスを失って 59 以下になったあとの受諾は拒否
    let mut st = w.load(SC_A);
    hold(&mut st.2, &S22, 2);
    hold(&mut st.2, &[17696, 17905, 18112], 2);
    let l = BattleLedger::new();
    let pr = propose(&st, 1, 2, 1, ProposalTerms::CedeTarget, 5);
    assert!(w.check(&st, &l, &pr, 1, 5).is_ok());
    hold(&mut st.2, &[17696], 1); // 国家1 が 21 の 1 つを取り戻す(−3 → 57)
    let before = snapshot(&st);
    assert_eq!(w.accept(&mut st, &l, &pr, 1, 6).unwrap_err(), PeaceError::CedeConditionsNotMet { score: 57, required: 60 });
    assert_same(&st, &before, "lapsed cession");
    // 白紙講和: −10 で提案し、防御側がもう 1 勝して −15 になったあとの受諾は拒否
    let mut st = w.load(SC_A);
    hold(&mut st.2, &[18109, 17266, 17267], 2);
    let mut l = ledger_for_war1(&st.1, (0, 2));
    let pr = propose(&st, 1, 1, 2, ProposalTerms::WhitePeace, 1);
    assert!(w.check(&st, &l, &pr, 2, 30).is_ok());
    l.record(&st.1, MapBattleId(5000), c(2), c(1), BattleOutcome::DefenderWon);
    assert_eq!(w.check(&st, &l, &pr, 2, 31).unwrap_err(), PeaceError::WhitePeaceScoreTooLow { score: -15, required: -10 });
    // 二重受諾: 1 回目は成立、2 回目は終結済みで拒否され、状態は 1 回目のまま
    let mut st = w.load(SC_A);
    let l = BattleLedger::new();
    let pr = propose(&st, 1, 2, 1, ProposalTerms::AcceptDefeat, 1);
    w.accept(&mut st, &l, &pr, 1, 1).unwrap();
    let after_first = snapshot(&st);
    let e = w.accept(&mut st, &l, &pr, 1, 2).unwrap_err();
    assert!(matches!(e, PeaceError::WarEnded { war, end } if war == wid(1) && end.day == 1));
    assert_same(&st, &after_first, "double acceptance");
    assert_eq!(st.1.truce_until(c(1), c(2)), Some(1 + 1825), "the truce is not set a second time");
    // 目標の所有国が変わって戦争が失効した場合も(戦争2 は目標 22 の割譲で失効)終結済みとして拒否
    let mut st = w.load(SC_A);
    hold(&mut st.2, &S22, 2);
    hold(&mut st.2, &[17696, 17905, 18112], 2);
    let l = BattleLedger::new();
    let p2 = propose(&st, 2, 3, 1, ProposalTerms::AcceptDefeat, 1);
    let p1 = propose(&st, 1, 2, 1, ProposalTerms::CedeTarget, 1);
    w.accept(&mut st, &l, &p1, 1, 1).unwrap();
    let b = snapshot(&st);
    assert!(matches!(w.accept(&mut st, &l, &p2, 1, 2).unwrap_err(), PeaceError::WarEnded { war, end } if war == wid(2) && end.reason == EndReason::TargetOwnerChanged));
    assert_same(&st, &b, "expired war");
}

// ---------------------------------------------------------------- 首都候補(返還後)

#[test]
fn capital_candidates_include_provinces_the_same_peace_returns_and_a_standalone_transfer_keeps_the_old_rule() {
    let w = world();
    // 国家2 が 22 と 20 を全域支配(国家1 は 20 に支配するプロヴィンスがない)
    let mut st = w.load(SC_A);
    hold(&mut st.2, &S22, 2);
    hold(&mut st.2, &S20, 2);
    // 単独の移転(現行の規則): 首都の候補がなく拒否
    let treq = TransferRequest::new(MapStateId(22), c(1), c(2));
    assert_eq!(plan_transfer(w.ctx(), &st.0, &st.2, &treq).unwrap_err(), TransferError::NoCapitalCandidate { country: c(1) });
    // 同じ割譲を講和の中で: 敵対が解けて 20 が返還されるので、20 の最小プロヴィンス 18109 が首都になる
    let b = snapshot(&st);
    let plan = plan_end_war(w.ctx(), &st.0, &st.1, &st.2, &EndWarRequest::cession(wid(1), 1)).unwrap();
    assert_same(&st, &b, "plan is read-only");
    assert_eq!(plan.outcome().transfer.as_ref().unwrap().capital_move, Some(CapitalMove { country: c(1), from: ProvinceId(17266), to: ProvinceId(18109) }));
    let o = end_war(w.ctx(), &mut st.0, &mut st.1, &mut st.2, &EndWarRequest::cession(wid(1), 1)).unwrap();
    assert_eq!(&o, plan.outcome());
    assert_eq!(st.0.capital(c(1)), Some(ProvinceId(18109)));
    assert_eq!(st.2.control().controller(ProvinceId(18109)), Some(c(1)), "the province is back under its legal owner");
    assert!(st.0.check_consistency(&st.2).is_ok() && st.1.check_consistency(&st.2).is_ok());
}

#[test]
fn when_hostility_continues_through_another_war_no_candidate_remains_and_the_peace_is_rejected_atomically() {
    let w = world();
    // 戦争3(3 対 1)が続くので、国家3 が 20 を全域支配していれば返還されず、首都の候補がない。
    let mut st = w.load(SC_A);
    hold(&mut st.2, &S22, 2);
    hold(&mut st.2, &S20, 3);
    let b = snapshot(&st);
    let e = plan_end_war(w.ctx(), &st.0, &st.1, &st.2, &EndWarRequest::cession(wid(1), 1)).unwrap_err();
    assert_eq!(e, EndWarError::Transfer(TransferError::NoCapitalCandidate { country: c(1) }));
    let e = end_war(w.ctx(), &mut st.0, &mut st.1, &mut st.2, &EndWarRequest::cession(wid(1), 1)).unwrap_err();
    assert!(matches!(e, EndWarApplyError::Stale(EndWarStale::Invalid(EndWarError::Transfer(TransferError::NoCapitalCandidate { .. })))));
    assert_same(&st, &b, "rejected atomically");
}

// ---------------------------------------------------------------- 自動講和

#[test]
fn auto_peace_with_only_the_target_held_does_not_end_war_1_but_with_state_20_it_ends_by_defender_surrender() {
    let w = world();
    // (a) 22 だけ全域: 占領率 50%、師団あり → 継続
    let mut st = w.load(SC_A);
    hold(&mut st.2, &S22, 2);
    let mut l = BattleLedger::new();
    let b = snapshot(&st);
    let a = w.assess(&st, &l, 1);
    assert_eq!(a.breakdown.attacker_occupation.rate, Some(Ratio { num: 1, den: 2 }));
    let r = w.auto(&mut st, &mut l, &[], 1);
    assert!(r.entries.is_empty());
    assert_same(&st, &b, "no surrender");
    assert!(st.1.war(wid(1)).unwrap().is_active());
    // (b) 20 も全域 → 防御側降伏
    let mut st = w.load(SC_A);
    hold(&mut st.2, &S22, 2);
    hold(&mut st.2, &S20, 2);
    let r = w.auto(&mut st, &mut l, &[], 1);
    assert_eq!(r.entries.len(), 2, "war 1 ended; war 2 skipped as already ended by the chain");
    let AutoPeaceEntry::Ended { war, verdict, outcome } = &r.entries[0] else { panic!("{:?}", r.entries[0]) };
    assert_eq!((*war, verdict.clone()), (wid(1), Verdict::DefenderSurrenders));
    assert_eq!(r.entries[1], AutoPeaceEntry::SkippedEnded { war: wid(2) });
    assert_eq!(outcome.ended[0], EndedWar { war: wid(1), result: WarResult::AttackerVictory, reason: EndReason::DefenderSurrender }, "recorded as an automatic surrender, not a proposal's Cession");
    assert_eq!(outcome.ended[1], EndedWar { war: wid(2), result: WarResult::WhitePeace, reason: EndReason::TargetOwnerChanged });
    assert_eq!(st.0.owner(MapStateId(22)), Some(c(2)));
    assert_eq!(st.0.owner(MapStateId(20)), Some(c(1)));
    assert_eq!(st.0.capital(c(1)), Some(ProvinceId(18109)));
    assert_eq!(st.2.control().controller(ProvinceId(18109)), Some(c(1)), "20 is returned");
    assert!(st.1.war(wid(3)).unwrap().is_active() && st.1.war(wid(4)).unwrap().is_active());
    assert!(st.1.at_war(c(1), c(3)) && !st.1.at_war(c(1), c(2)), "1-3 hostility is kept");
    // 休戦は直接終結した戦争 1 の代表国 1-2 間だけ(連鎖終結した戦争 2 の 3-1 には作らない)
    let truces: Vec<_> = st.1.truces().collect();
    assert_eq!(truces, vec![(c(1), c(2), 1 + 1825)]);
    assert!(st.0.check_consistency(&st.2).is_ok() && st.1.check_consistency(&st.2).is_ok());
}

#[test]
fn auto_attacker_surrender_is_a_defender_victory_without_territory_transfer() {
    let w = world();
    let mut st = w.load(SC_A);
    hold(&mut st.2, &S21, 1); // 国家1 が国家2 の唯一のステートを全域支配 → 被占領率 100%
    let owners: Vec<_> = st.0.owners().collect();
    let mut l = BattleLedger::new();
    let r = w.auto(&mut st, &mut l, &[], 7);
    let AutoPeaceEntry::Ended { war, verdict, outcome } = &r.entries[0] else { panic!("{:?}", r.entries) };
    assert_eq!((*war, verdict.clone()), (wid(1), Verdict::AttackerSurrenders));
    assert_eq!(outcome.ended[0], EndedWar { war: wid(1), result: WarResult::DefenderVictory, reason: EndReason::AttackerSurrender });
    assert!(outcome.transfer.is_none());
    assert_eq!(st.0.owners().collect::<Vec<_>>(), owners);
    assert_eq!(st.1.truce_until(c(1), c(2)), Some(7 + 1825));
    assert_eq!(st.2.control().controller(ProvinceId(17269)), Some(c(2)), "the occupation returns to the legal owner");
}

#[test]
fn both_sides_met_and_total_zero_means_the_attacker_surrenders_and_todays_battle_counts_before_the_peace() {
    let w = world();
    let make = || {
        let mut sc = w.scenario(SC_A);
        zero_manpower(&mut sc, 1);
        let mut st = w.build(sc);
        hold(&mut st.2, &S22, 2);
        hold(&mut st.2, &S21, 1);
        st
    };
    // 総点 0(防御側 4 勝): 双方成立 → 攻撃側(0 以下)が降伏。領土移転なしの防御側勝利
    let mut st = make();
    let owners: Vec<_> = st.0.owners().collect();
    let mut l = ledger_for_war1(&st.1, (0, 4));
    assert_eq!(w.assess(&st, &l, 1).breakdown.total_score, 0);
    let r = w.auto(&mut st, &mut l, &[], 3);
    let AutoPeaceEntry::Ended { verdict, outcome, .. } = &r.entries[0] else { panic!("{:?}", r.entries) };
    assert_eq!(*verdict, Verdict::AttackerSurrenders);
    assert_eq!(outcome.ended[0], EndedWar { war: wid(1), result: WarResult::DefenderVictory, reason: EndReason::AttackerSurrender });
    assert!(outcome.transfer.is_none());
    assert_eq!(st.0.owners().collect::<Vec<_>>(), owners);
    // 総点 +5(防御側 3 勝): 台帳だけなら防御側の降伏(割譲)。その日の確定戦闘(防御側の勝利)が先に計上されると 0 になり、攻撃側の降伏になる
    let mut st1 = make();
    let mut l1 = ledger_for_war1(&st1.1, (0, 3));
    assert_eq!(w.assess(&st1, &l1, 1).breakdown.total_score, 5);
    let r1 = w.auto(&mut st1, &mut l1, &[], 3);
    assert!(matches!(&r1.entries[0], AutoPeaceEntry::Ended { verdict: Verdict::DefenderSurrenders, .. }));
    let mut st2 = make();
    let mut l2 = ledger_for_war1(&st2.1, (0, 3));
    let events = vec![MoveEvent::BattleFinished {
        battle: MapBattleId(7777),
        province: ProvinceId(17484),
        attacker: c(2),
        defender: c(1),
        outcome: BattleOutcome::DefenderWon,
        reason: FinishReason::AttackerGone,
    }];
    let r2 = w.auto(&mut st2, &mut l2, &events, 3);
    assert_eq!(r2.ledger_entries, vec![LedgerEntry::Counted { battle: MapBattleId(7777), war: wid(1), attacker_side_won: false }]);
    assert!(matches!(&r2.entries[0], AutoPeaceEntry::Ended { verdict: Verdict::AttackerSurrenders, outcome, .. } if outcome.ended[0].reason == EndReason::AttackerSurrender));
    // 同じ戦闘を翌日に再度渡しても二重に計上しない
    let again = w.auto(&mut st2, &mut l2, &events, 4);
    assert_eq!(again.ledger_entries, vec![LedgerEntry::Duplicate { battle: MapBattleId(7777) }]);
}

#[test]
fn a_failed_auto_peace_leaves_the_war_active_unchanged_and_is_judged_again_on_the_next_day() {
    let w = world();
    // 戦争2 だけ(3 対 1、目標 22)。国家3 が 22 と 20 を全域支配 → 防御側降伏だが、国家1 の首都(22)の候補が 20 にない(戦争3 がなければ返還だが、この世界は戦争2 のみ)
    // → 戦争2 だけだと 3-1 の敵対が解けて 20 が返還され成立してしまうので、戦争3 も置いて敵対を維持する
    let make = |keep_war3: bool| {
        let mut sc = w.scenario(SC_A);
        sc.war_records.retain(|r| r.id == wid(2) || (keep_war3 && r.id == wid(3)));
        let mut st = w.build(sc);
        hold(&mut st.2, &S22, 3);
        hold(&mut st.2, &S20, 3);
        st
    };
    // 戦争2 と 3 の両方: 戦争2 が先(ID 昇順)。首都の候補がなく失敗 → 戦争3(20 の割譲)は成立
    let mut st = make(true);
    let mut l = BattleLedger::new();
    let b = snapshot(&st);
    let r = w.auto(&mut st, &mut l, &[], 1);
    assert!(matches!(&r.entries[0], AutoPeaceEntry::Failed { war, verdict: Verdict::DefenderSurrenders, error: EndWarApplyError::Stale(EndWarStale::Invalid(EndWarError::Transfer(TransferError::NoCapitalCandidate { .. }))) } if *war == wid(2)));
    assert!(matches!(&r.entries[1], AutoPeaceEntry::Ended { war, .. } if *war == wid(3)));
    assert!(st.1.war(wid(2)).unwrap().is_active(), "the failed war stays active");
    assert_eq!(st.0.owner(MapStateId(20)), Some(c(3)));
    assert_eq!(st.0.owner(MapStateId(22)), Some(c(1)), "the failed cession changed nothing");
    // 失敗だけの世界(戦争2 のみで敵対が解けない…は成立するので、失敗の原子性は上の戦争2 の失敗の前後で確認する): 戦争2 の失敗時点の状態 = 戦争3 の成立だけの差
    let mut only3 = b.clone();
    let mut l3 = BattleLedger::new();
    let end3 = end_war(w.ctx(), &mut only3.0, &mut only3.1, &mut only3.2, &EndWarRequest::defender_surrender(wid(3), 1)).unwrap();
    assert_eq!(end3.ended[0].reason, EndReason::DefenderSurrender);
    assert_same(&st, &only3, "the failed attempt left no trace");
    let _ = &mut l3;
    // 翌日: 戦争2 を最新の状態(国家1 の所有は 22 だけ)で再判定 → 割譲で国家1 が消滅。戦争1(2 対 1)は消滅による決着
    let r2 = w.auto(&mut st, &mut l, &[], 2);
    let AutoPeaceEntry::Ended { war, outcome, .. } = &r2.entries[0] else { panic!("{:?}", r2.entries) };
    assert_eq!(*war, wid(2));
    assert!(outcome.transfer.as_ref().unwrap().elimination.is_some());
    assert!(!st.0.is_active(c(1)));
    let _ = make(false);
}

#[test]
fn a_later_war_of_the_same_day_is_judged_on_the_state_the_earlier_peace_left() {
    let w = world();
    // 検証入力: 国家2 が 21 と 22 を所有、国家1 が 20。戦争1(1 対 2、目標 22)、戦争3(2 対 3、目標 20)。
    // 国家1 が 22 全域と 21 の 5/13、国家3 が 21 の 8/13 を支配。
    //  戦争1 前: 国家1 の占領率 = (1 + 5/13)/2 ≈ 69% → 防御側(国家2)降伏 → 22 が国家1 へ。
    //  戦争3 前: 国家3 の占領率(国家2 の所有 21・22 に対し)= (8/13 + 0)/2 ≈ 31% → 降伏なし。戦争1 の割譲後は国家2 の所有が 21 だけ → 8/13 ≈ 61.5% ≥ 60% → 攻撃側(国家2)降伏。
    let mut sc = w.scenario(SC_A);
    sc.state_owners.insert(MapStateId(22), c(2));
    let rec = |id: u32, a: usize, d: usize, t: u32| WarRecord { id: wid(id), attacker: WarSide::new(c(a), []), defender: WarSide::new(c(d), []), target_state: MapStateId(t), start_day: 0, end: None };
    sc.war_records = vec![rec(1, 1, 2, 22), rec(3, 2, 3, 20)];
    let mut st = w.build(sc);
    hold(&mut st.2, &S22, 1);
    hold(&mut st.2, &S21[..5], 1);
    hold(&mut st.2, &S21[5..], 3);
    let mut l = BattleLedger::new();
    let before3 = w.assess(&st, &l, 3);
    assert_eq!(before3.verdict, Verdict::None, "before the earlier peace, war 3 has no surrender");
    assert_eq!(before3.breakdown.defender_occupation.rate, Some(Ratio { num: 4, den: 13 }));
    let r = w.auto(&mut st, &mut l, &[], 9);
    assert_eq!(r.entries.len(), 2, "{:?}", r.entries);
    assert!(matches!(&r.entries[0], AutoPeaceEntry::Ended { war, verdict: Verdict::DefenderSurrenders, .. } if *war == wid(1)));
    let AutoPeaceEntry::Ended { war, verdict, outcome } = &r.entries[1] else { panic!("{:?}", r.entries[1]) };
    assert_eq!((*war, verdict.clone()), (wid(3), Verdict::AttackerSurrenders), "war 3 is judged on the updated ownership");
    assert_eq!(outcome.ended[0].reason, EndReason::AttackerSurrender);
}

#[test]
fn the_last_state_cession_eliminates_the_country_ends_related_wars_and_clears_its_truces() {
    let w = world();
    let mut st = w.load(SC_B);
    // 先に戦争5(2 対 4)を白紙講和 → 2-4 の休戦ができる
    end_war(w.ctx(), &mut st.0, &mut st.1, &mut st.2, &EndWarRequest::white_peace(wid(5), 1)).unwrap();
    assert!(st.1.truce_until(c(2), c(4)).is_some());
    // 国家1 が国家2 の唯一のステート 21 を全域支配 → 戦争8 の防御側(国家2)降伏 = 割譲で国家2 が消滅
    hold(&mut st.2, &S21, 1);
    let mut l = BattleLedger::new();
    let r = w.auto(&mut st, &mut l, &[], 2);
    let AutoPeaceEntry::Ended { war, outcome, .. } = &r.entries[0] else { panic!("{:?}", r.entries) };
    assert_eq!(*war, wid(8));
    assert_eq!(outcome.ended[0], EndedWar { war: wid(8), result: WarResult::AttackerVictory, reason: EndReason::DefenderSurrender });
    assert!(outcome.ended.contains(&EndedWar { war: wid(7), result: WarResult::AttackerVictory, reason: EndReason::DefenderEliminated }), "war 7: its defender primary was eliminated");
    assert!(outcome.supporters_removed.iter().any(|s| s.war == wid(6) && s.country == c(2)), "war 6: the eliminated supporter is removed from its side");
    assert!(st.1.war(wid(6)).unwrap().is_active());
    let el = outcome.transfer.as_ref().unwrap().elimination.as_ref().unwrap();
    assert_eq!((el.country, el.annexed_by), (c(2), c(1)));
    assert!(!st.0.is_active(c(2)));
    // 消滅国との休戦はすべて消える: 以前の 2-4 と、この講和で一度設定した 1-2(消滅国との休戦は残さない)
    assert_eq!(outcome.truces_cleared, vec![(c(1), c(2)), (c(2), c(4))]);
    assert_eq!(outcome.truce, None, "no truce with an eliminated country");
    assert!(st.1.truces().all(|(a, b, _)| a != c(2) && b != c(2)));
    assert!(st.0.check_consistency(&st.2).is_ok() && st.1.check_consistency(&st.2).is_ok());
}

#[test]
fn assessment_in_war_score_alone_never_applies_anything() {
    // 自動適用は run_auto_peace を呼んだときだけ。判定(assess_active_wars)は状態を変えない。
    let w = world();
    let mut st = w.load(SC_A);
    hold(&mut st.2, &S22, 2);
    hold(&mut st.2, &S20, 2);
    let b = snapshot(&st);
    let l = BattleLedger::new();
    let rs = assess_active_wars(&st.1, &w.data.model, &st.0, &st.2, &l);
    assert_eq!(rs[0].as_ref().unwrap().verdict, Verdict::DefenderSurrenders);
    assert_same(&st, &b, "read-only");
}
