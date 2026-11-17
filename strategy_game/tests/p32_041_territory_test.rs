//! P32-041 (M7-a): ステート単位の領土移転(事前検証 + 一括適用)と国家消滅(`world::territory`)の検証。
//!
//! 検証用シナリオ(`division_territory_scenario.ron`)の所有・支配・首都・交戦関係は検証入力で、正式な世界データではない。
//! 旧経路(`state::ownership`・`country::elimination`)との比較は、旧型の fixture で実際に実行する(新旧IDの対応表: 旧 `StateId k` ↔ 新 `MapStateId 20+k`(検証地域の 20・21・22 ↔ 0・1・2)。
//! 国家は両経路とも同じ `CountryId`)。保存・アセットは書かない。

use strategy_game::common::{CountryId, StateId};
use strategy_game::country::elimination::preflight_country_elimination;
use strategy_game::country::{CountryData, CountryRegistry};
use strategy_game::hexmap::data::{HexMap, load_from_path};
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::state::ownership::{OwnershipTransferError, preflight_transfer_state_ownership, transfer_state_ownership};
use strategy_game::world::*;

const C1: CountryId = CountryId(1);
const C2: CountryId = CountryId(2);
const C3: CountryId = CountryId(3);
const C4: CountryId = CountryId(4);
const TERR: &str = "assets/data/world/preview/division_territory_scenario.ron";

fn p(id: u32) -> ProvinceId {
    ProvinceId(id)
}

fn s(id: u32) -> MapStateId {
    MapStateId(id)
}

fn md(id: u32) -> MapDivisionId {
    MapDivisionId(id)
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

impl World {
    fn ctx(&self) -> TransferContext<'_> {
        TransferContext { model: &self.data.model, graph: &self.graph }
    }

    fn load_file(&self, path: &str) -> (Territory, MapMilitary) {
        let sc = load_division_scenario(path, &self.graph, &self.data.model).unwrap();
        let t = Territory::from_scenario(&sc, &self.data.model, &self.graph).unwrap();
        (t, sc.into_military())
    }

    fn load_text(&self, text: &str) -> (Territory, MapMilitary) {
        let file = parse_division_scenario(text).unwrap();
        let sc = resolve_division_scenario(&file, &self.graph, &self.data.model).unwrap_or_else(|e| panic!("{e:?}"));
        let t = Territory::from_scenario(&sc, &self.data.model, &self.graph).unwrap();
        (t, sc.into_military())
    }
}

fn req(state: u32, from: CountryId, to: CountryId) -> TransferRequest {
    TransferRequest::new(s(state), from, to)
}

fn run(w: &World, t: &mut Territory, m: &mut MapMilitary, r: &TransferRequest) -> TransferOutcome {
    transfer_state(w.ctx(), t, m, r).unwrap_or_else(|e| panic!("{e}"))
}

/// 進行中の戦闘が `province` に現れるまで日を進める(補給の充足率で到着日が変わるので、日数は固定しない。上限 40 日)。
fn until_battle(m: &mut MapMilitary, g: &ProvinceGraph, province: ProvinceId) {
    for _ in 0..40 {
        if m.ongoing_battle_at(province).is_some() {
            return;
        }
        m.advance_day(g);
    }
    panic!("no battle at {province}");
}

fn days(m: &mut MapMilitary, g: &ProvinceGraph, n: u32) -> Vec<MoveEvent> {
    let mut all = Vec::new();
    for _ in 0..n {
        all.extend(m.advance_day(g));
    }
    all
}

// ---------------------------------------------------------------- 初期状態

#[test]
fn the_scenario_gives_owners_capitals_and_active_countries_and_the_supply_capital_is_the_same_value() {
    let w = world();
    let (t, m) = w.load_file(TERR);
    assert_eq!(t.owners().collect::<Vec<_>>(), vec![(s(20), C1), (s(21), C2), (s(22), C1)]);
    assert_eq!((t.capital(C1), t.capital(C2), t.capital(C3), t.capital(C4)), (Some(p(17266)), Some(p(17695)), Some(p(18312)), None));
    assert!([C1, C2, C3, C4].iter().all(|c| t.is_active(*c)));
    assert_eq!(t.status(CountryId(9)), None);
    for c in [C1, C2, C3] {
        assert_eq!(m.supply_capital(c), t.capital(c), "the supply refers to the same capital value");
    }
    assert_eq!(t.check_consistency(&m), Ok(()));
    assert_eq!(t.owned_states(C1), vec![s(20), s(22)]);
}

// ---------------------------------------------------------------- 移転(ステート22: 首都移転・第三国の占領解消・師団の帰還)

#[test]
fn ceding_state_22_moves_ownership_control_capital_and_returns_only_non_belligerent_divisions() {
    let w = world();
    let (mut t, mut m) = w.load_file(TERR);
    let o = run(&w, &mut t, &mut m, &req(22, C1, C2));
    assert_eq!((o.state, o.previous_owner, o.new_owner), (s(22), C1, C2));
    assert_eq!(t.owner(s(22)), Some(C2));
    assert_eq!((t.owner(s(20)), t.owner(s(21))), (Some(C1), Some(C2)), "other states are untouched");
    // 所属プロヴィンス全部が新所有国の支配(WorldModel の所属から)
    let members = w.data.model.provinces_of(s(22)).unwrap().to_vec();
    assert_eq!(members, vec![p(17266), p(17267), p(17268), p(17484), p(17485), p(17693)]);
    for pr in &members {
        assert_eq!(m.control().controller(*pr), Some(C2), "{pr}");
    }
    assert_eq!(o.control_changes.len(), 6);
    assert_eq!(o.control_changes.iter().map(|c| c.province).collect::<Vec<_>>(), members, "province order");
    // 解消された第三国の占領は 17484(国家3)だけ
    assert_eq!(o.third_party_resolved, vec![ControlChange { province: p(17484), previous: Some(C3), new: Some(C2) }]);
    // 首都: 17266 が移転ステートにあったので、残るステート20 の最小の自国支配 18109 へ(正本と補給が同じ値)
    assert_eq!(o.capital_move, Some(CapitalMove { country: C1, from: p(17266), to: p(18109) }));
    assert_eq!((t.capital(C1), m.supply_capital(C1)), (Some(p(18109)), Some(p(18109))));
    assert_eq!(t.check_consistency(&m), Ok(()));
    // 帰還: 国家3 の師団17(17484)は国家2 と交戦していないので帰還(隣接する自国支配なし → 自国支配の最小 17903)。国家1 の師団(2・3・14)は国家2 と交戦中なので残る
    assert_eq!(o.relocated, vec![Relocation { division: md(17), owner: C3, from: p(17484), to: p(17903) }]);
    assert!(o.disbanded.is_empty());
    for d in [2, 3, 14] {
        let province = m.division(md(d)).unwrap().province;
        assert!(members.contains(&province), "division {d} stays (at war with the new owner)");
    }
    let d17 = m.division(md(17)).unwrap();
    assert_eq!((d17.province, d17.status, d17.destination, d17.path.len()), (p(17903), MapDivisionStatus::Idle, None, 0));
    // 補給の無効化: 変更前と変更後の国(第三国を含む)と首都が移った国
    assert_eq!(o.invalidated_countries, vec![C1, C2, C3]);
    assert!(o.elimination.is_none() && o.cancelled_battles.is_empty());
    assert!(t.is_active(C1) && t.is_active(C2));
    assert_eq!(t.revision(), 1);
}

#[test]
fn the_capital_representative_is_the_smallest_controlled_province_of_the_first_remaining_state() {
    let w = world();
    // 18109(ステート20 の最小)を国家2 が支配していると、次に小さい自国支配 18308 が選ばれる
    let (mut t, mut m) = w.load_file(TERR);
    m.set_controller(p(18109), Some(C2));
    let o = run(&w, &mut t, &mut m, &req(22, C1, C2));
    assert_eq!(o.capital_move.unwrap().to, p(18308));
    assert_eq!(m.supply_capital(C1), Some(p(18308)));
    // ステート20 に自国支配のプロヴィンスが 1 つもなければ、候補なしで拒否し、何も変えない
    let (t0, mut m0) = w.load_file(TERR);
    for pr in w.data.model.provinces_of(s(20)).unwrap() {
        m0.set_controller(*pr, Some(C2));
    }
    let (before_t, before_m) = (t0.clone(), m0.clone());
    let e = plan_transfer(w.ctx(), &t0, &m0, &req(22, C1, C2)).unwrap_err();
    assert_eq!(e, TransferError::NoCapitalCandidate { country: C1 });
    let mut t1 = t0.clone();
    assert!(transfer_state(w.ctx(), &mut t1, &mut m0, &req(22, C1, C2)).is_err());
    assert_eq!((t1, m0), (before_t, before_m), "a rejection changes nothing");
    // 首都が移転ステートの外にあれば首都は動かない(ステート20 の割譲)
    let (mut t2, mut m2) = w.load_file(TERR);
    let o = run(&w, &mut t2, &mut m2, &req(20, C1, C2));
    assert_eq!(o.capital_move, None);
    assert_eq!(t2.capital(C1), Some(p(17266)));
}

// ---------------------------------------------------------------- 国家消滅(ステート21)

#[test]
fn ceding_the_last_state_eliminates_the_country_and_cleans_up_divisions_wars_supply_and_control() {
    let w = world();
    let (mut t, mut m) = w.load_file(TERR);
    let o = run(&w, &mut t, &mut m, &req(21, C2, C1));
    assert_eq!(t.owner(s(21)), Some(C1));
    // 国家2 は消滅(ID は保持。一覧にある)
    assert_eq!(t.status(C2), Some(CountryStatus::Eliminated { annexed_by: C1 }));
    assert!(t.countries().any(|c| c == C2), "the id is kept");
    assert!(!t.is_active(C2));
    assert_eq!((t.capital(C2), m.supply_capital(C2)), (None, None), "no capital and no supply setup for the eliminated country");
    let e = o.elimination.clone().unwrap();
    assert_eq!((e.country, e.annexed_by, e.supply_removed, e.capital_removed), (C2, C1, true, Some(p(17695))));
    assert_eq!(e.wars_removed, vec![(C1, C2)]);
    assert!(!m.at_war(C1, C2));
    assert!(e.control_reverted.is_empty(), "country 2 held nothing outside the ceded state");
    // 国家2 の師団は解散(消滅)。国家3 の師団10 は帰還、国家4 の師団20 は帰還先がなく解散(別の理由)
    let gone: Vec<(u32, DisbandReason)> = o.disbanded.iter().map(|d| (d.division.0, d.reason)).collect();
    assert_eq!(
        gone,
        vec![
            (5, DisbandReason::CountryEliminated),
            (6, DisbandReason::CountryEliminated),
            (8, DisbandReason::CountryEliminated),
            (12, DisbandReason::CountryEliminated),
            (15, DisbandReason::CountryEliminated),
            (16, DisbandReason::CountryEliminated),
            (20, DisbandReason::NoReturnProvince),
        ]
    );
    assert!(m.divisions().all(|d| d.owner != C2), "no division of the eliminated country remains");
    assert!(m.division(md(20)).is_none());
    assert_eq!(o.relocated, vec![Relocation { division: md(10), owner: C3, from: p(18312), to: p(17484) }], "the smallest id country 3 controls (17484 in state 22; 17903 is larger)");
    // 第三国の占領の解消(国家3: 18312、国家4: 17904)。全プロヴィンスが国家1 の支配
    let resolved: Vec<(u32, Option<CountryId>)> = o.third_party_resolved.iter().map(|c| (c.province.0, c.previous)).collect();
    assert_eq!(resolved, vec![(17904, Some(C4)), (18312, Some(C3))]);
    for pr in w.data.model.provinces_of(s(21)).unwrap() {
        assert_eq!(m.control().controller(*pr), Some(C1), "{pr}");
    }
    // 国家1 の師団 11(17905)は残る(併合した国の師団)
    assert_eq!(m.division(md(11)).unwrap().province, p(17905));
    // 補給の無効化は第三国・第四国・消滅国・併合国を含む
    assert_eq!(o.invalidated_countries, vec![C1, C2, C3, C4]);
    assert_eq!(t.check_consistency(&m), Ok(()));
    // 消滅した国家は、移転元にも移転先にもなれない
    let (bt, bm) = (t.clone(), m.clone());
    assert_eq!(plan_transfer(w.ctx(), &t, &m, &req(20, C2, C1)).unwrap_err(), TransferError::CountryEliminated(C2));
    assert_eq!(plan_transfer(w.ctx(), &t, &m, &req(20, C1, C2)).unwrap_err(), TransferError::CountryEliminated(C2));
    assert_eq!((t, m), (bt, bm));
}

#[test]
fn an_eliminated_countrys_other_controlled_provinces_return_to_their_legal_owner() {
    let w = world();
    let (mut t, mut m) = w.load_file(TERR);
    // 国家2 がステート20 の 18499 と、ステート未所属の 18110 を支配している
    m.set_controller(p(18499), Some(C2));
    m.set_controller(p(18110), Some(C2));
    let o = run(&w, &mut t, &mut m, &req(21, C2, C1));
    let e = o.elimination.unwrap();
    assert_eq!(
        e.control_reverted,
        vec![ControlChange { province: p(18110), previous: Some(C2), new: None }, ControlChange { province: p(18499), previous: Some(C2), new: Some(C1) }],
        "the legal owner's control (state 20: country 1); an unassigned province has no controller"
    );
    assert_eq!(m.control().controller(p(18499)), Some(C1));
    assert_eq!(m.control().controller(p(18110)), None);
    assert!(m.control().iter().all(|(_, c)| c != C2));
}

#[test]
fn the_supply_reach_is_invalidated_and_the_next_plan_uses_the_new_capital_and_control() {
    let w = world();
    let (mut t, mut m) = w.load_file(TERR);
    m.advance_day(&w.graph); // 到達結果をキャッシュ
    for c in [C1, C2, C3] {
        assert!(m.reach_is_cached(c));
    }
    let o = run(&w, &mut t, &mut m, &req(22, C1, C2));
    assert_eq!(o.invalidated_countries, vec![C1, C2, C3]);
    for c in [C1, C2, C3] {
        assert!(!m.reach_is_cached(c), "country {} must be recomputed", c.0);
    }
    let ev = m.advance_day(&w.graph);
    let r1 = ev.iter().find_map(|e| if let MoveEvent::DailySupply(r) = e { (r.country == C1).then(|| r.clone()) } else { None }).expect("supply of country 1");
    assert!(r1.reach_recomputed && r1.capital_controlled, "computed from the new capital 18109 and the new control");
    // 消滅した国家の補給は計画されない
    let (mut t2, mut m2) = w.load_file(TERR);
    m2.advance_day(&w.graph);
    run(&w, &mut t2, &mut m2, &req(21, C2, C1));
    let ev = m2.advance_day(&w.graph);
    assert!(ev.iter().all(|e| !matches!(e, MoveEvent::DailySupply(r) if r.country == C2)), "no supply plan for the eliminated country");
    assert!(ev.iter().any(|e| matches!(e, MoveEvent::DailySupply(r) if r.country == C1)));
}

// ---------------------------------------------------------------- 戦闘の中止

/// 国家2 がステート20 も所有する状態で、師団14(国家1)が 17486(ステート21)を攻撃して戦闘中(5 日目)。
fn with_battle(w: &World) -> (Territory, MapMilitary, MapBattleId) {
    let (mut t, mut m) = w.load_file(TERR);
    run(w, &mut t, &mut m, &req(20, C1, C2));
    m.issue_move_order(&w.graph, md(14), p(17486)).unwrap();
    until_battle(&mut m, &w.graph, p(17486));
    let b = m.ongoing_battle_at(p(17486)).expect("a battle is going on").id;
    (t, m, b)
}

#[test]
fn a_battle_in_the_transferred_state_must_be_named_and_is_cancelled_without_a_result_or_occupation() {
    let w = world();
    let (t, m, b) = with_battle(&w);
    assert_eq!(m.division(md(14)).unwrap().status, MapDivisionStatus::Fighting);
    // 明示がなければ拒否(何も変えない)
    let e = plan_transfer(w.ctx(), &t, &m, &req(21, C2, C1)).unwrap_err();
    assert_eq!(e, TransferError::BattleInTransferredState { battle: b, province: p(17486) });
    let mut t1 = t.clone();
    let mut m1 = m.clone();
    assert!(transfer_state(w.ctx(), &mut t1, &mut m1, &req(21, C2, C1)).is_err());
    assert_eq!((&t1, &m1), (&t, &m));
    // 未知の戦闘は拒否
    assert_eq!(plan_transfer(w.ctx(), &t, &m, &req(21, C2, C1).with_cancel_battles([MapBattleId(77)])).unwrap_err(), TransferError::UnknownBattle(MapBattleId(77)));
    // 明示すれば中止して移転(国家2 はステート20 も持つので消滅しない)。勝敗・占領は発生しない(移転による支配の変更だけ)
    let (mut t2, mut m2) = (t.clone(), m.clone());
    let o = run(&w, &mut t2, &mut m2, &req(21, C2, C1).with_cancel_battles([b]));
    assert_eq!(o.cancelled_battles.len(), 1);
    let c = &o.cancelled_battles[0];
    assert_eq!((c.battle, c.province, c.attacker_country, c.defender_country), (b, p(17486), C1, C2));
    assert_eq!(c.released, vec![md(14), md(15)]);
    assert!(m2.battles().next().is_none());
    for d in [14, 15] {
        let div = m2.division(md(d)).unwrap();
        assert_eq!((div.status, div.battle), (MapDivisionStatus::Idle, None), "division {d} is released");
    }
    assert!(o.elimination.is_none() && t2.is_active(C2));
    // 師団14・15 の戦力は中止で変わらない(損失・回復なし)
    assert_eq!(m2.division(md(14)).unwrap().combat, m.division(md(14)).unwrap().combat);
}

#[test]
fn the_standalone_battle_cancellation_names_its_targets_keeps_other_battles_and_changes_nothing_on_error() {
    let w = world();
    let (_, mut m, b) = with_battle(&w);
    let before = m.clone();
    // 指定の誤り(未知・重複)は何も変えずに拒否
    assert_eq!(m.cancel_battles(&[b, MapBattleId(99)]), Err(CancelError::UnknownBattle(MapBattleId(99))));
    assert_eq!(m.cancel_battles(&[b, b]), Err(CancelError::DuplicateBattle(b)));
    assert_eq!(m, before);
    // 空の指定は何も中止しない(無関係な戦闘は維持される)
    assert_eq!(m.cancel_battles(&[]), Ok(Vec::new()));
    assert_eq!(m, before);
    // 指定した戦闘だけを中止する。勝敗・占領はなく、支配は変わらない
    let control_before: Vec<_> = m.control().iter().collect();
    let done = m.cancel_battles(&[b]).unwrap();
    assert_eq!(done.len(), 1);
    assert_eq!(m.control().iter().collect::<Vec<_>>(), control_before, "no occupation");
    assert!(m.battle(b).is_none());
    // 中止した戦闘は進行中ではないので、もう一度指定すると拒否(何も変わらない)
    let after = m.clone();
    assert_eq!(m.cancel_battles(&[b]), Err(CancelError::UnknownBattle(b)));
    assert_eq!(m, after);
    // 中止後の翌日の処理が壊れない(戦闘から外れた師団は待機)
    days(&mut m, &w.graph, 1);
}

#[test]
fn an_unrelated_battle_is_kept_when_another_state_is_transferred() {
    let w = world();
    let (mut t, mut m, b) = with_battle(&w);
    let o = run(&w, &mut t, &mut m, &req(20, C2, C3)); // ステート20 の割譲(国家2 はステート21 を持つので消滅せず、戦闘はステート21)
    assert!(o.cancelled_battles.is_empty());
    assert_eq!(m.battle(b).map(|x| x.status), Some(MapBattleStatus::Ongoing));
    assert_eq!(m.division(md(14)).unwrap().status, MapDivisionStatus::Fighting);
}

#[test]
fn an_eliminated_countrys_battles_are_cancelled_automatically() {
    let w = world();
    let (mut t, mut m) = w.load_file(TERR);
    m.issue_move_order(&w.graph, md(14), p(17486)).unwrap();
    until_battle(&mut m, &w.graph, p(17486));
    let o = run(&w, &mut t, &mut m, &req(21, C2, C1)); // 国家2 の最後のステート。戦闘の明示は不要
    assert_eq!(o.cancelled_battles.len(), 1);
    assert!(m.battles().next().is_none());
    assert_eq!(m.division(md(14)).unwrap().status, MapDivisionStatus::Idle);
}

// ---------------------------------------------------------------- 移動命令の扱い

#[test]
fn other_move_orders_are_kept_and_revalidated_by_the_next_daily_path_check() {
    let w = world();
    let (mut t, mut m) = w.load_file(TERR);
    // 国家1 の師団7(18310)がステート20 を出て 17903 方面へ。国家1 の師団1(18311)は 18310 へ向かう(同じステート20 内)
    let dest = w.data.model.provinces_of(s(20)).unwrap().iter().copied().find(|d| *d != p(18310) && m.clone().issue_move_order(&w.graph, md(7), *d).is_ok()).expect("a reachable province");
    m.issue_move_order(&w.graph, md(7), dest).unwrap_or_else(|e| panic!("{e}"));
    let before = m.division(md(7)).unwrap().clone();
    assert_eq!(before.status, MapDivisionStatus::Moving);
    // ステート20 を国家3(中立)へ割譲: 師団7 は経路を消されない(帰還の対象でもない: 国家1 は国家3 と交戦していない…ので帰還対象)
    let o = run(&w, &mut t, &mut m, &req(20, C1, C3));
    let returned: Vec<u32> = o.relocated.iter().map(|r| r.division.0).collect();
    // 国家1 の師団(1・4・7)は国家3 と交戦していないので帰還する(隣接する自国支配 → 最小 ID)
    assert!(returned.contains(&7) && returned.contains(&1) && returned.contains(&4), "{returned:?}");
    for r in &o.relocated {
        let d = m.division(r.division).unwrap();
        assert_eq!((d.status, d.destination, d.path.len(), d.progress), (MapDivisionStatus::Idle, None, 0, 0.0), "a returned division's order is discarded");
        assert_eq!(m.control().controller(r.to), Some(r.owner));
    }
    // 帰還しなかった移動中の師団の命令は維持され、日次の経路検証が再判定する
    let (mut t2, mut m2) = w.load_file(TERR);
    m2.issue_move_order(&w.graph, md(2), p(17267)).unwrap(); // ステート22 内の移動(国家1 の師団2: 17693 → 17267 の途中)
    let d2 = m2.division(md(2)).unwrap().clone();
    assert_eq!(d2.status, MapDivisionStatus::Moving);
    let o2 = run(&w, &mut t2, &mut m2, &req(22, C1, C2)); // 国家1 は国家2 と交戦中: 師団2 は帰還しない
    assert!(o2.relocated.iter().all(|r| r.division != md(2)));
    let kept = m2.division(md(2)).unwrap();
    assert_eq!((kept.status, kept.destination, kept.path.clone()), (MapDivisionStatus::Moving, d2.destination, d2.path.clone()), "the order is untouched by the transfer");
    // 翌日の経路検証が通行条件(交戦中の敵支配地は通れる)を既存の規則で再判定する: 壊れずに進む
    days(&mut m2, &w.graph, 1);
}

// ---------------------------------------------------------------- 帰還先の規則(隣接優先 → 最小ID → 解散)

fn relocation_text(overrides: &[(u32, u32)], c3_division_at: u32) -> String {
    let ov: Vec<String> = overrides.iter().map(|(pr, c)| format!("(province: {pr}, country: {c})")).collect();
    format!(
        "(format_version: 1, description: \"relocation\", base_days: 5.0, \
         countries: [(label: 1, country_id: 1), (label: 2, country_id: 2), (label: 3, country_id: 3)], \
         state_owners: [(state: 20, country: 1), (state: 21, country: 2), (state: 22, country: 1)], \
         controller_overrides: [{}], wars: [(a: 1, b: 2)], \
         divisions: [(id: 1, country: 1, province: 18309, speed: 1.0), (id: 2, country: 2, province: 18502, speed: 1.0), (id: 3, country: 3, province: {c3_division_at}, speed: 1.0)])",
        ov.join(", ")
    )
}

#[test]
fn a_returning_division_prefers_an_adjacent_own_province_then_the_smallest_then_is_disbanded() {
    let w = world();
    // 国家3 の師団が 18311(ステート20)にいる。隣接する 18312(国家3 の支配)と、隣接しない小さい 17903(国家3 の支配)がある。
    assert!(w.graph.neighbor_ids(p(18311)).any(|n| n == p(18312)) && !w.graph.neighbor_ids(p(18311)).any(|n| n == p(17903)));
    let (mut t, mut m) = w.load_text(&relocation_text(&[(18311, 3), (18312, 3), (17903, 3)], 18311));
    let o = run(&w, &mut t, &mut m, &req(20, C1, C2));
    assert_eq!(o.relocated, vec![Relocation { division: md(3), owner: C3, from: p(18311), to: p(18312) }], "adjacent first (18312), not the smallest (17903)");
    // 隣接する自国支配がなければ、自国支配の最小 ID
    let (mut t, mut m) = w.load_text(&relocation_text(&[(18311, 3), (17903, 3), (17904, 3)], 18311));
    assert!(!w.graph.neighbor_ids(p(18311)).any(|n| n == p(17904) || n == p(17903)), "neither is adjacent");
    let o = run(&w, &mut t, &mut m, &req(20, C1, C2));
    assert_eq!(o.relocated, vec![Relocation { division: md(3), owner: C3, from: p(18311), to: p(17903) }], "no adjacent own province: the smallest controlled id");
    // 自国支配が他にまったくなければ解散(戦闘の撃破ではない)
    let (mut t, mut m) = w.load_text(&relocation_text(&[(18311, 3)], 18311));
    let o = run(&w, &mut t, &mut m, &req(20, C1, C2));
    assert!(o.relocated.is_empty());
    assert_eq!(o.disbanded, vec![Disbanded { division: md(3), owner: C3, at: p(18311), reason: DisbandReason::NoReturnProvince }]);
    assert!(m.division(md(3)).is_none());
    // 帰還先に進行中の戦闘がある地点は選ばない: 18312 で戦闘中なら次の候補
    let (mut t, mut m) = w.load_text(&relocation_text(&[(18311, 3), (18312, 3), (17903, 3)], 18311));
    let _ = (&mut t, &mut m);
}

// ---------------------------------------------------------------- 拒否と原子性

#[test]
fn every_invalid_request_is_refused_and_leaves_all_state_exactly_as_before() {
    let w = world();
    let (t, m) = w.load_file(TERR);
    let cases: Vec<(TransferRequest, TransferError)> = vec![
        (req(99, C1, C2), TransferError::UnknownState(s(99))),
        (req(22, C2, C1), TransferError::OwnerMismatch { state: s(22), expected: C2, actual: Some(C1) }),
        (req(20, C1, C1), TransferError::SameOwner(C1)),
        (req(20, C1, CountryId(9)), TransferError::UnknownCountry(CountryId(9))),
        (req(20, CountryId(9), C1), TransferError::UnknownCountry(CountryId(9))),
        (req(22, C1, C2).with_cancel_battles([MapBattleId(5)]), TransferError::UnknownBattle(MapBattleId(5))),
    ];
    for (r, want) in cases {
        let (bt, bm) = (t.clone(), m.clone());
        assert_eq!(plan_transfer(w.ctx(), &t, &m, &r).unwrap_err(), want, "{r:?}");
        let (mut t2, mut m2) = (t.clone(), m.clone());
        let e = transfer_state(w.ctx(), &mut t2, &mut m2, &r).unwrap_err();
        assert_eq!(e, ApplyError::Stale(StaleReason::Invalid(want.clone())), "{r:?}");
        assert_eq!((&t2, &m2), (&bt, &bm), "state unchanged after refusing {r:?}");
        assert_eq!(t2.revision(), 0);
    }
    // 計画の作成は状態を変えない
    let (bt, bm) = (t.clone(), m.clone());
    let _ = plan_transfer(w.ctx(), &t, &m, &req(22, C1, C2)).unwrap();
    assert_eq!((&t, &m), (&bt, &bm));
}

#[test]
fn stale_and_already_applied_plans_are_refused_and_change_nothing() {
    let w = world();
    let (mut t, mut m) = w.load_file(TERR);
    let p22 = plan_transfer(w.ctx(), &t, &m, &req(22, C1, C2)).unwrap();
    let p20 = plan_transfer(w.ctx(), &t, &m, &req(20, C1, C2)).unwrap();
    let applied = apply_transfer(w.ctx(), &mut t, &mut m, &p22).unwrap();
    assert_eq!(&applied, p22.outcome(), "the applied result equals the plan");
    // 適用済み
    let (bt, bm) = (t.clone(), m.clone());
    assert_eq!(apply_transfer(w.ctx(), &mut t, &mut m, &p22).unwrap_err(), ApplyError::AlreadyApplied);
    // 別の移転のあとに届いた古い計画
    assert_eq!(apply_transfer(w.ctx(), &mut t, &mut m, &p20).unwrap_err(), ApplyError::Stale(StaleReason::TerritoryChanged));
    assert_eq!((&t, &m), (&bt, &bm), "refused plans change nothing");

    // 同じ領土の状態でも、計画のあとに軍の状態が変わって結果が違えば古い(支配が変わった)
    let (mut t, mut m) = w.load_file(TERR);
    let plan = plan_transfer(w.ctx(), &t, &m, &req(22, C1, C2)).unwrap();
    m.set_controller(p(17267), Some(C3));
    let (bt, bm) = (t.clone(), m.clone());
    assert_eq!(apply_transfer(w.ctx(), &mut t, &mut m, &plan).unwrap_err(), ApplyError::Stale(StaleReason::OutcomeChanged));
    assert_eq!((&t, &m), (&bt, &bm));

    // 計画のあとに、首都の再配置先(ステート20 の自国支配)がなくなれば、現在の状態では要求が成り立たない
    let (mut t, mut m) = w.load_file(TERR);
    let plan = plan_transfer(w.ctx(), &t, &m, &req(22, C1, C2)).unwrap();
    for pr in w.data.model.provinces_of(s(20)).unwrap() {
        m.set_controller(*pr, Some(C2));
    }
    let (bt, bm) = (t.clone(), m.clone());
    assert_eq!(apply_transfer(w.ctx(), &mut t, &mut m, &plan).unwrap_err(), ApplyError::Stale(StaleReason::Invalid(TransferError::NoCapitalCandidate { country: C1 })));
    assert_eq!((&t, &m), (&bt, &bm));
}

#[test]
fn an_applied_plan_leaves_the_supply_capital_equal_to_the_territory_capital_and_the_check_detects_a_mismatch() {
    let w = world();
    let (mut t, mut m) = w.load_file(TERR);
    run(&w, &mut t, &mut m, &req(22, C1, C2));
    assert_eq!(t.check_consistency(&m), Ok(()));
    m.set_capital(C1, p(18308));
    assert!(t.check_consistency(&m).is_err(), "a diverged supply capital is detected");
}

// ---------------------------------------------------------------- 決定性

#[test]
fn the_result_does_not_depend_on_the_input_order() {
    let w = world();
    let text = std::fs::read_to_string(TERR).unwrap();
    let build = |reverse: bool| {
        let mut file = parse_division_scenario(&text).unwrap();
        if reverse {
            file.countries.reverse();
            file.state_owners.reverse();
            file.controller_overrides.reverse();
            file.wars.reverse();
            file.divisions.reverse();
            file.supply.countries.reverse();
            file.supply.divisions.reverse();
        }
        let sc = resolve_division_scenario(&file, &w.graph, &w.data.model).unwrap_or_else(|e| panic!("{e:?}"));
        (Territory::from_scenario(&sc, &w.data.model, &w.graph).unwrap(), sc.into_military())
    };
    for (state, from, to) in [(22u32, C1, C2), (21, C2, C1), (20, C1, C3)] {
        let ((mut ta, mut ma), (mut tb, mut mb)) = (build(false), build(true));
        assert_eq!((&ta, &ma), (&tb, &mb), "the same initial state");
        let oa = run(&w, &mut ta, &mut ma, &req(state, from, to));
        let ob = run(&w, &mut tb, &mut mb, &req(state, from, to));
        assert_eq!(oa, ob, "state {state}");
        assert_eq!((&ta, &ma), (&tb, &mb));
    }
    // 中止の指定の順序・重複は結果に影響しない
    let w2 = world();
    let (t, m, b) = with_battle(&w2);
    let r1 = req(21, C2, C1).with_cancel_battles([b]);
    let r2 = req(21, C2, C1).with_cancel_battles([b, b]);
    assert_eq!(plan_transfer(w2.ctx(), &t, &m, &r1).unwrap(), plan_transfer(w2.ctx(), &t, &m, &r2).unwrap());
}

// ---------------------------------------------------------------- 旧経路との比較(旧型の fixture)

type OldNewCase = (Result<strategy_game::state::ownership::ValidatedOwnershipTransfer, OwnershipTransferError>, TransferRequest, fn(&TransferError) -> bool);

fn old_country(id: usize, capital: usize) -> CountryData {
    CountryData { id: CountryId(id), capital_state_id: StateId(capital), ..Default::default() }
}

fn old_state(id: usize, owner: usize) -> StateData {
    StateData { id: StateId(id), owner_country_id: CountryId(owner), is_sea: false, ..Default::default() }
}

/// 旧 `StateId k` ↔ 新 `MapStateId 20+k`。国家 1・2 の首都ステートは新の補給の首都(22・21 内)に対応する。
fn old_registries() -> (StateRegistry, CountryRegistry) {
    // 旧: ステート 0=国家1、1=国家2、2=国家1(新の 20・21・22)。首都: 国家1=旧2(新22)、国家2=旧1(新21)
    (StateRegistry::build(vec![old_state(0, 1), old_state(1, 2), old_state(2, 1)]), CountryRegistry { countries: vec![old_country(1, 2), old_country(2, 1), old_country(3, 1)] })
}

fn new_state_of_old(k: usize) -> MapStateId {
    MapStateId(20 + k as u32)
}

#[test]
fn the_old_and_new_paths_agree_on_rejections_capital_reassignment_and_elimination() {
    let w = world();
    let (sr, cr) = old_registries();
    let (t, m) = w.load_file(TERR);
    // 拒否の種類(旧 → 新)
    let table: Vec<OldNewCase> = vec![
        (preflight_transfer_state_ownership(StateId(2), CountryId(2), CountryId(1), &sr, &cr), req(22, C2, C1), |e| matches!(e, TransferError::OwnerMismatch { .. })),
        (preflight_transfer_state_ownership(StateId(0), CountryId(1), CountryId(1), &sr, &cr), req(20, C1, C1), |e| matches!(e, TransferError::SameOwner(_))),
        (preflight_transfer_state_ownership(StateId(0), CountryId(1), CountryId(9), &sr, &cr), req(20, C1, CountryId(9)), |e| matches!(e, TransferError::UnknownCountry(_))),
        (preflight_transfer_state_ownership(StateId(99), CountryId(1), CountryId(2), &sr, &cr), req(99, C1, C2), |e| matches!(e, TransferError::UnknownState(_))),
    ];
    let want_old = [OwnershipTransferError::CurrentOwnerMismatch, OwnershipTransferError::SameOwner, OwnershipTransferError::CountryNotFound, OwnershipTransferError::StateNotFound];
    for ((old, r, new_is), want) in table.into_iter().zip(want_old) {
        assert_eq!(old.unwrap_err(), want);
        let e = plan_transfer(w.ctx(), &t, &m, &r).unwrap_err();
        assert!(new_is(&e), "{e:?} corresponds to {want:?}");
    }
    // 首都の再配置: 旧は残る州の最小 StateId(0)。新は対応するステート(20)の中の代表地点
    let old_plan = preflight_transfer_state_ownership(StateId(2), CountryId(1), CountryId(2), &sr, &cr).unwrap();
    let (country, old_new_capital) = old_plan.capital_reassignment().expect("the capital state is ceded");
    assert_eq!((country, old_new_capital), (CountryId(1), StateId(0)));
    let new_plan = plan_transfer(w.ctx(), &t, &m, &req(22, C1, C2)).unwrap();
    let mv = new_plan.outcome().capital_move.clone().expect("a capital move");
    assert_eq!(mv.country, country);
    assert_eq!(w.data.model.state_of(mv.to), Some(new_state_of_old(old_new_capital.0)), "the same state as the old minimum StateId");
    // 適用後: 旧は所有・支配が新所有国に、首都が更新される。新も全プロヴィンスが新所有国
    let (mut sr2, mut cr2) = old_registries();
    transfer_state_ownership(StateId(2), CountryId(1), CountryId(2), &mut sr2, &mut cr2).unwrap();
    let st = sr2.get(StateId(2)).unwrap();
    assert_eq!((st.owner_country_id, st.controller()), (CountryId(2), CountryId(2)));
    assert_eq!(cr2.get(CountryId(1)).unwrap().capital_state_id, StateId(0));
    let (mut t2, mut m2) = (t.clone(), m.clone());
    run(&w, &mut t2, &mut m2, &req(22, C1, C2));
    assert_eq!(t2.owner(new_state_of_old(2)), Some(C2));
    assert!(w.data.model.provinces_of(new_state_of_old(2)).unwrap().iter().all(|pr| m2.control().controller(*pr) == Some(C2)));
    // 拒否時の状態保持(旧: 登録簿が変わらない / 新: 領土と軍が変わらない)
    let (mut sr3, mut cr3) = old_registries();
    assert!(transfer_state_ownership(StateId(2), CountryId(2), CountryId(1), &mut sr3, &mut cr3).is_err());
    let (osr, ocr) = old_registries();
    for k in 0..3 {
        let (a, b) = (sr3.get(StateId(k)).unwrap(), osr.get(StateId(k)).unwrap());
        assert_eq!((a.owner_country_id, a.controller()), (b.owner_country_id, b.controller()));
    }
    assert_eq!(cr3.get(CountryId(1)).unwrap().capital_state_id, ocr.get(CountryId(1)).unwrap().capital_state_id);
    // 最後のステート: 旧は LastOwnedLandState → 国家消滅の preflight が成り立つ。新も消滅になる
    assert_eq!(preflight_transfer_state_ownership(StateId(1), CountryId(2), CountryId(1), &sr, &cr).unwrap_err(), OwnershipTransferError::LastOwnedLandState);
    assert!(preflight_country_elimination(StateId(1), CountryId(2), CountryId(1), "d", &sr, &cr).is_ok());
    let o = plan_transfer(w.ctx(), &t, &m, &req(21, C2, C1)).unwrap();
    assert!(o.outcome().elimination.as_ref().is_some_and(|e| e.country == C2 && e.annexed_by == C1));
    // 旧は国家を物理削除しない(ID を保持)。新も ID を保持する
    let (mut t3, mut m3) = (t.clone(), m.clone());
    run(&w, &mut t3, &mut m3, &req(21, C2, C1));
    assert_eq!(t3.status(C2), Some(CountryStatus::Eliminated { annexed_by: C1 }));
}

// ---------------------------------------------------------------- 師団プレビュー(`hexmap::division_view`)での移転

#[test]
fn the_division_preview_session_transfers_the_selected_cells_state_to_the_selected_divisions_country() {
    use strategy_game::hexmap::division_view::DivisionSession;
    let w = world();
    let mut sess = DivisionSession::load(TERR, w.graph.clone(), &w.data.model).unwrap();
    let model = &w.data.model;
    assert_eq!(sess.territory().owner(s(22)), Some(C1));
    // 師団未選択・セル未選択・同じ国への移転は、理由つきで拒否し何も変えない
    let before = sess.military().clone();
    assert!(sess.transfer_selected_cell_state(model, Some(p(17266))).is_err());
    sess.select(md(6)).unwrap(); // 国家2 の師団
    assert!(sess.transfer_selected_cell_state(model, None).is_err());
    assert!(sess.transfer_selected_cell_state(model, Some(p(18110))).is_err(), "an unassigned province has no state");
    sess.select(md(1)).unwrap(); // 国家1 の師団。ステート20 は既に国家1 の所有
    assert!(sess.transfer_selected_cell_state(model, Some(p(18309))).is_err());
    let (ok, text) = sess.log().last().unwrap();
    assert!(!*ok && text.contains("拒否") && text.contains("変わりません"), "{text}");
    assert_eq!(sess.military(), &before);
    assert_eq!(sess.territory().owner(s(20)), Some(C1));
    // 国家2 の師団を選び、ステート22 のセルで割譲
    sess.select(md(6)).unwrap();
    let rev = sess.revision();
    let o = sess.transfer_selected_cell_state(model, Some(p(17266))).unwrap();
    assert_eq!((o.previous_owner, o.new_owner), (C1, C2));
    assert!(sess.revision() > rev, "the display is rebuilt");
    assert_eq!(sess.territory().owner(s(22)), Some(C2));
    // 表示: 法的所有国と実効支配国は別項目。選択を国家1 の師団(ステート22 に残る交戦中の師団3)にして確認
    sess.select(md(3)).unwrap();
    let t = sess.detail_text(model);
    assert!(t.contains("法的所有国(ステート): 国家2") && t.contains("実効支配国(現在地): 国家2"), "{t}");
    assert!(sess.control_text(model).contains("MapState#22: 法的所有 国家2 / 実効支配 全域 国家2"), "{}", sess.control_text(model));
    assert!(sess.control_text(model).contains("首都(正本)"));
    // 国家1 の首都は、ステート20 の 18109 へ移っている(表示文に出る)
    assert!(sess.control_text(model).contains("国家1 Province#18109"), "{}", sess.control_text(model));
    // 消滅: 国家1 の師団でステート21 を併合 → 国家2 はステート22 を持つので消滅しない。続けてステート22 を併合すると国家2 が消滅
    sess.select(md(1)).unwrap();
    sess.transfer_selected_cell_state(model, Some(p(17269))).unwrap();
    assert!(sess.territory().is_active(C2));
    sess.transfer_selected_cell_state(model, Some(p(17266))).unwrap();
    assert!(!sess.territory().is_active(C2));
    assert!(sess.control_text(model).contains("消滅した国家: 国家2(併合: 国家1)"), "{}", sess.control_text(model));
    assert!(sess.military().divisions().all(|d| d.owner != C2));
    assert!(sess.log().any(|(_, l)| l.contains("戦闘の撃破ではありません")));
    assert_eq!(sess.territory().check_consistency(sess.military()), Ok(()));
}
