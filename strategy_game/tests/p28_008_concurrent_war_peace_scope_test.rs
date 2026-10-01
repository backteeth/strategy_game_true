//! P28-008: 講和処理(`execute_peace_settlement`)の占領状態正常化とDivision帰還を、
//! 終結するWarに帰属するものだけに限定する契約(候補B)のレジストリレベル恒久テスト。
//!
//! 実`execute_peace_settlement`・実`evaluate_war_capitulation`を、手組みのレジストリ
//! (Bevy Appなし)で直接呼ぶ。本番の占領経路(`military::invasion::occupy_state`)は
//! `controller_country`しか書かず`state.war_id`を設定しないため、本テストの占領も
//! `controller_country`だけを設定する(`war_id`に依存する実装を検出するため)。
//!
//! 真理値表(W=終結するWar、P(W)=Wの全参加国):
//!
//! | 状況 | owner∈P(W) | controller∈P(W) | 他Active Warで敵対 | 結果 |
//! |---|---|---|---|---|
//! | 単独War | ○ | ○ | × | 戻す |
//! | 同じ防御国への別War(占領者はP(W)外) | ○ | × | - | 保持 |
//! | 当事国共有(占領者はP(W)外) | ○ | × | - | 保持 |
//! | 支持国所有州をP(W)の敵が占領 | ○ | ○ | × | 戻す |
//! | 同じ2国が別Active Warでも敵対 | ○ | ○ | ○ | 保持 |
//! | 所有国がP(W)外 | × | - | - | 保持 |

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use strategy_game::common::{BattleId, CountryId, DivisionId, StateId, WarId};
use strategy_game::country::{CountryData, CountryRegistry};
use strategy_game::diplomacy::claims::ClaimRegistry;
use strategy_game::diplomacy::crisis::{CrisisRegistry, WarGoal, WarGoalType};
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
    STANDARD_INFANTRY_DEFINITION_ID,
};
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::war::capitulation::{CapitulationResult, evaluate_war_capitulation};
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::FrontlineRegistry;
use strategy_game::war::justification::WarJustificationRegistry;
use strategy_game::war::peace::{PeaceTerm, execute_peace_settlement};

const X: CountryId = CountryId(0);
const Y: CountryId = CountryId(1);
const Z: CountryId = CountryId(2);
const W: CountryId = CountryId(3);
const V: CountryId = CountryId(4);

struct World {
    states: StateRegistry,
    wars: WarRegistry,
    mil: MilitaryRegistry,
    battles: BattleRegistry,
    dip: DiplomacyRegistry,
    fl: FrontlineRegistry,
    crisis: CrisisRegistry,
    claims: ClaimRegistry,
    countries: CountryRegistry,
    armies: ArmyRegistry,
    just: WarJustificationRegistry,
    power: strategy_game::country::power::CountryPowerRegistry,
}

fn st(id: usize, owner: CountryId, neighbors: &[usize]) -> StateData {
    StateData {
        id: StateId(id),
        owner_country_id: owner,
        neighbors: neighbors.iter().map(|n| StateId(*n)).collect(),
        ..Default::default()
    }
}

fn war(att: CountryId, def: CountryId, target: usize) -> War {
    War {
        id: WarId(0),
        name: "T".to_string(),
        attackers: [att].into_iter().collect(),
        defenders: [def].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![WarGoal {
            attacker: att,
            defender: def,
            goal_type: WarGoalType::ConquerState,
            target_states: vec![StateId(target)],
            base_peace_cost: 20.0,
            international_concern: 10.0,
            completion: 0.0,
            is_primary: true,
        }],
        start_date: "1800/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: HashSet::new(),
    }
}

fn division(owner: CountryId, at: usize, status: DivisionStatus) -> Division {
    Division {
        id: DivisionId(0),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(at),
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 10_000,
        max_manpower: 10_000,
        equipment: 100.0,
        max_equipment: 100.0,
        organization: 50.0,
        max_organization: 50.0,
        morale: 50.0,
        max_morale: 50.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status,
        def_id: STANDARD_INFANTRY_DEFINITION_ID,
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

/// X=c0, Y=c1, Z=c2, W=c3, V=c4。s0,s1: X / s10,s11,s12: Y / s2: Z / s3: W / s4: V。
/// `y_single_state`ならYの陸州はs10のみ(s11,s12はV所有)。Yの師団は置かない
/// (=降伏条件2「防御側に戦闘可能な陸軍なし」により、攻撃側が目標州を支配すれば降伏する)。
fn world(y_single_state: bool) -> World {
    let mut states = vec![
        st(0, X, &[10, 1]),
        st(1, X, &[0]),
        st(10, Y, &[0, 11]),
        st(2, Z, &[11]),
        st(3, W, &[12]),
        st(4, V, &[]),
    ];
    let y_owner_of_s11_s12 = if y_single_state { V } else { Y };
    states.push(st(11, y_owner_of_s11_s12, &[10, 12, 2]));
    states.push(st(12, y_owner_of_s11_s12, &[11, 3]));
    let caps = [
        (X, 1usize),
        (Y, if y_single_state { 10 } else { 12 }),
        (Z, 2),
        (W, 3),
        (V, 4),
    ];
    World {
        states: StateRegistry::build(states),
        wars: WarRegistry::default(),
        mil: MilitaryRegistry::default(),
        battles: BattleRegistry::default(),
        dip: DiplomacyRegistry::default(),
        fl: FrontlineRegistry::default(),
        crisis: CrisisRegistry::default(),
        claims: ClaimRegistry::default(),
        countries: CountryRegistry {
            countries: caps
                .iter()
                .map(|(c, cap)| CountryData {
                    id: *c,
                    capital_state_id: StateId(*cap),
                    ..CountryData::default()
                })
                .collect(),
        },
        armies: ArmyRegistry::default(),
        just: WarJustificationRegistry::default(),
        power: Default::default(),
    }
}

/// 本番の占領経路と同じく`controller_country`だけを設定する(`state.war_id`は設定しない)。
fn occupy(w: &mut World, sid: usize, by: CountryId) {
    w.states.get_mut(StateId(sid)).unwrap().controller_country = Some(by);
}

fn ctrl(w: &World, sid: usize) -> CountryId {
    w.states.get(StateId(sid)).unwrap().controller()
}

fn owner(w: &World, sid: usize) -> CountryId {
    w.states.get(StateId(sid)).unwrap().owner_country_id
}

fn settle(w: &mut World, id: WarId, term: PeaceTerm, reason: &str) {
    execute_peace_settlement(
        id,
        term,
        reason,
        "1800/06/01",
        &mut w.states,
        &mut w.wars,
        &mut w.mil,
        &mut w.battles,
        &mut w.dip,
        &mut w.fl,
        &mut w.crisis,
        &mut w.claims,
        &mut w.countries,
        &mut w.armies,
        &mut w.just,
        &mut w.power,
    )
    .expect("settlement must succeed");
}

/// `handle_daily_war_resolution`ステップ3(降伏判定→即時講和)と同じ手順。
/// `order`はHashMap反復順(プロセス・インスタンスごとに非決定的)を表す。
fn capitulation_step(w: &mut World, order: &[WarId]) {
    for &id in order {
        let cap = evaluate_war_capitulation(w.wars.wars.get(&id).unwrap(), &w.states, &w.mil);
        match cap {
            CapitulationResult::DefenderCapitulated => {
                settle(w, id, PeaceTerm::CedeWarGoalRegion, "Defender Capitulation")
            }
            CapitulationResult::AttackerCapitulated => {
                settle(w, id, PeaceTerm::AttackerConcedes, "Attacker Capitulation")
            }
            CapitulationResult::None => {}
        }
    }
}

/// 順序独立性の比較に使う、ワールド状態の決定論的な文字列化。
fn snapshot(w: &World) -> String {
    let mut s = String::new();
    for st in &w.states.states {
        let _ = writeln!(
            s,
            "s{} o{} c{}",
            st.id.0,
            st.owner_country_id.0,
            st.controller().0
        );
    }
    let mut ids: Vec<_> = w.wars.wars.keys().copied().collect();
    ids.sort_by_key(|x| x.0);
    for id in ids {
        let wr = &w.wars.wars[&id];
        let _ = writeln!(
            s,
            "war{} {:?} {:?} {:?}",
            id.0, wr.status, wr.winner, wr.end_reason
        );
    }
    let mut dv: Vec<_> = w.mil.divisions.values().collect();
    dv.sort_by_key(|d| d.id.0);
    for d in dv {
        let _ = writeln!(
            s,
            "div{} c{} s{} {:?}",
            d.id.0, d.owner.0, d.current_state.0, d.status
        );
    }
    s
}

// ── 単独War ─────────────────────────────────────────────────────────────

#[test]
fn single_war_peace_normalizes_all_of_its_own_occupations() {
    let mut w = world(false);
    let a = w.wars.add_war(war(X, Y, 10));
    occupy(&mut w, 10, X);
    occupy(&mut w, 11, X);
    w.mil.add_division(division(X, 11, DivisionStatus::Idle));

    settle(&mut w, a, PeaceTerm::WhitePeace, "T");

    assert_eq!(ctrl(&w, 10), Y);
    assert_eq!(ctrl(&w, 11), Y);
    // 終結Warの敵領に残っていたX師団は、従来どおり自国領へ帰還する。
    let d = w.mil.divisions.values().next().unwrap();
    assert_eq!(d.current_state, StateId(0));
    assert_eq!(d.status, DivisionStatus::Idle);
    assert_eq!(w.wars.wars[&a].status, WarStatus::WhitePeace);
}

// ── 同じ防御国に対する二つのWar ─────────────────────────────────────────────

#[test]
fn peace_of_one_war_keeps_other_wars_occupation_and_division() {
    let mut w = world(false);
    let a = w.wars.add_war(war(X, Y, 10));
    let b = w.wars.add_war(war(Z, Y, 11));
    occupy(&mut w, 10, X);
    occupy(&mut w, 11, Z);
    let xd = w.mil.add_division(division(X, 10, DivisionStatus::Idle));
    let zd = w.mil.add_division(division(Z, 11, DivisionStatus::Idle));

    // E2(国境消失)・降伏以外の白紙講和も同じ経路を通る。理由文字列は挙動に影響しない。
    settle(&mut w, a, PeaceTerm::WhitePeace, "Frontline Unreachable");

    assert_eq!(ctrl(&w, 10), Y, "War Aの占領は戻る");
    assert_eq!(
        w.mil.divisions[&xd].current_state,
        StateId(0),
        "War AのX師団は帰還"
    );
    assert_eq!(ctrl(&w, 11), Z, "War Bの占領は保持される");
    assert_eq!(
        w.mil.divisions[&zd].current_state,
        StateId(11),
        "War BのZ師団は動かない"
    );
    assert_eq!(w.mil.divisions[&zd].status, DivisionStatus::Idle);
    assert_eq!(w.wars.wars[&b].status, WarStatus::Active);
}

#[test]
fn peace_of_one_war_does_not_pull_other_wars_fighting_division_out_of_its_battle() {
    let mut w = world(false);
    let a = w.wars.add_war(war(X, Y, 10));
    let b = w.wars.add_war(war(Z, Y, 12));
    occupy(&mut w, 10, X);
    let zd = w
        .mil
        .add_division(division(Z, 12, DivisionStatus::Fighting));
    let yd = w
        .mil
        .add_division(division(Y, 12, DivisionStatus::Fighting));
    let mut origins = HashMap::new();
    origins.insert(zd, StateId(11));
    let bid = w
        .battles
        .start_battle(Battle {
            id: BattleId(0),
            war_id: b,
            state_id: StateId(12),
            attacker_country: Z,
            defender_country: Y,
            attacker_division_ids: vec![zd],
            defender_division_ids: vec![yd],
            attacker_origins: origins,
            start_date: "1800/05/30".to_string(),
            elapsed_days: 0,
            status: BattleStatus::Ongoing,
        })
        .unwrap();
    w.mil.divisions.get_mut(&zd).unwrap().combat_id = Some(bid);
    w.mil.divisions.get_mut(&yd).unwrap().combat_id = Some(bid);

    settle(&mut w, a, PeaceTerm::WhitePeace, "T");

    let z = &w.mil.divisions[&zd];
    assert_eq!(z.current_state, StateId(12), "戦闘中のZ師団は戦場に残る");
    assert_eq!(z.status, DivisionStatus::Fighting);
    assert_eq!(z.combat_id, Some(bid));
    assert_eq!(
        w.battles.battles[&bid].status,
        BattleStatus::Ongoing,
        "War Bの戦闘は継続"
    );
    assert_eq!(w.wars.wars[&b].status, WarStatus::Active);
}

#[test]
fn moving_division_of_other_war_in_enemy_territory_is_not_relocated() {
    let mut w = world(false);
    let a = w.wars.add_war(war(X, Y, 10));
    let _b = w.wars.add_war(war(Z, Y, 12));
    occupy(&mut w, 10, X);
    // ZはWar Bでs12(Y領・Y支配)へ進軍中。s12は占領ではなく、Z師団は敵領内にいる。
    let zd = w.mil.add_division(division(Z, 12, DivisionStatus::Moving));

    settle(&mut w, a, PeaceTerm::WhitePeace, "T");

    assert_eq!(w.mil.divisions[&zd].current_state, StateId(12));
    assert_eq!(w.mil.divisions[&zd].status, DivisionStatus::Moving);
}

#[test]
fn division_of_ended_war_party_stays_if_it_is_at_war_there_in_another_active_war() {
    let mut w = world(false);
    let a = w.wars.add_war(war(X, Y, 10));
    let _e = w.wars.add_war(war(X, Z, 2)); // XはZとも別Warで交戦中
    occupy(&mut w, 10, X);
    // XのDivisionはZ領s2に居る(War Eでの正当な敵領内)。War Aの終結で帰還させない。
    let xd = w.mil.add_division(division(X, 2, DivisionStatus::Idle));

    settle(&mut w, a, PeaceTerm::WhitePeace, "T");

    assert_eq!(w.mil.divisions[&xd].current_state, StateId(2));
}

// ── 当事国を共有するWar ───────────────────────────────────────────────────

#[test]
fn peace_does_not_touch_occupation_of_a_state_owned_by_a_party_but_held_by_a_third_war() {
    let mut w = world(false);
    let a = w.wars.add_war(war(X, Y, 10));
    let d = w.wars.add_war(war(W, X, 1)); // Xは War Aでは攻撃国、War Dでは防御国
    occupy(&mut w, 1, W); // X所有のs1をWがWar Dで占領
    occupy(&mut w, 10, X);
    let wd = w.mil.add_division(division(W, 1, DivisionStatus::Idle));

    settle(&mut w, a, PeaceTerm::WhitePeace, "T");

    assert_eq!(ctrl(&w, 1), W, "War Dの占領は保持");
    assert_eq!(w.mil.divisions[&wd].current_state, StateId(1));
    assert_eq!(ctrl(&w, 10), Y, "War Aの占領は戻る");
    assert_eq!(w.wars.wars[&d].status, WarStatus::Active);
}

#[test]
fn capitulation_cession_keeps_other_wars_occupation_but_transfers_its_own_target() {
    let mut w = world(false);
    let a = w.wars.add_war(war(X, Y, 10));
    let b = w.wars.add_war(war(Z, Y, 11));
    occupy(&mut w, 10, X);
    occupy(&mut w, 11, Z);
    let zd = w.mil.add_division(division(Z, 11, DivisionStatus::Idle));

    settle(
        &mut w,
        a,
        PeaceTerm::CedeWarGoalRegion,
        "Defender Capitulation",
    );

    assert_eq!(owner(&w, 10), X, "割譲は現行どおり実施される");
    assert_eq!(ctrl(&w, 10), X);
    assert_eq!(ctrl(&w, 11), Z, "別Warの占領は割譲でも消えない");
    assert_eq!(w.mil.divisions[&zd].current_state, StateId(11));
    assert_eq!(w.wars.wars[&b].status, WarStatus::Active);
}

#[test]
fn cession_that_invalidates_the_other_war_still_returns_that_wars_own_occupation() {
    // War Bの戦争目標もs10: 既存E1で白紙講和になる。そのWar B自身の占領(s11のZ占領)は
    // War B自身の終結として従来どおり所有国へ戻る(終結済みWarの占領は残らない)。
    let mut w = world(false);
    let a = w.wars.add_war(war(X, Y, 10));
    let b = w.wars.add_war(war(Z, Y, 10));
    occupy(&mut w, 10, X);
    occupy(&mut w, 11, Z);

    settle(
        &mut w,
        a,
        PeaceTerm::CedeWarGoalRegion,
        "Defender Capitulation",
    );

    assert_eq!(w.wars.wars[&b].status, WarStatus::WhitePeace);
    assert_eq!(
        w.wars.wars[&b].end_reason.as_deref(),
        Some("War Goal No Longer Achievable")
    );
    assert_eq!(ctrl(&w, 11), Y);
}

#[test]
fn country_elimination_cession_keeps_unrelated_wars_occupation() {
    let mut w = world(true);
    let a = w.wars.add_war(war(X, Y, 10));
    let b = w.wars.add_war(war(Z, Y, 10));
    let d = w.wars.add_war(war(V, X, 1)); // 無関係なWar D(VがX所有のs1を占領)
    occupy(&mut w, 10, X);
    occupy(&mut w, 1, V);

    settle(
        &mut w,
        a,
        PeaceTerm::CedeWarGoalRegion,
        "Defender Capitulation",
    );

    // Yの最後の陸州の割譲=国家消滅。消滅処理そのもの(War Bの終結)は従来どおり。
    assert_eq!(w.wars.wars[&a].status, WarStatus::AttackerVictory);
    assert_eq!(w.wars.wars[&b].status, WarStatus::AttackerVictory);
    assert_eq!(
        w.wars.wars[&b].end_reason.as_deref(),
        Some("Participant Country Eliminated")
    );
    assert_eq!(ctrl(&w, 1), V, "War Dの占領は保持される");
    assert_eq!(w.wars.wars[&d].status, WarStatus::Active);
}

// ── 講和後も敵対関係が残る州 / 支持国 / 対象外 ───────────────────────────────

#[test]
fn occupation_is_kept_while_the_same_pair_is_still_at_war_in_another_active_war() {
    let mut w = world(false);
    let a = w.wars.add_war(war(X, Y, 10));
    // 宣言時の`already_at_war`検査は通常は同じ2国の並行Warを許さないが、多国間参加などで
    // 同じペアが別Warでも敵対する状態になった場合の契約を固定する。
    let e = w.wars.add_war(war(X, Y, 11));
    occupy(&mut w, 10, X);

    settle(&mut w, a, PeaceTerm::WhitePeace, "T");
    assert_eq!(ctrl(&w, 10), X, "別のActive Warでも敵対が続くため保持");

    // 最後のWarが終われば、残りの占領も戻る(終結済みWarの占領は残らない)。
    settle(&mut w, e, PeaceTerm::WhitePeace, "T");
    assert_eq!(ctrl(&w, 10), Y);
}

#[test]
fn state_owned_by_a_supporter_of_the_ended_war_is_normalized() {
    let mut w = world(false);
    let a = w.wars.add_war(war(X, Y, 10));
    w.wars.wars.get_mut(&a).unwrap().defenders.insert(W); // WはYの支持国
    occupy(&mut w, 10, X);
    occupy(&mut w, 3, X); // W所有のs3をXが占領

    settle(&mut w, a, PeaceTerm::WhitePeace, "T");

    // 旧実装は主要当事国所有の州しか戻さず、支持国所有の占領が終結後も残留していた。
    assert_eq!(ctrl(&w, 3), W);
    assert_eq!(ctrl(&w, 10), Y);
}

#[test]
fn state_whose_owner_is_not_a_party_of_the_ended_war_is_left_alone() {
    // 連鎖占領: W所有のs3をYがWar E(Y対W)で占領→XがWar A(X対Y)でYから奪取。
    // 所有国WはWar Aの参加国ではないため、占領の帰属は現行データから一意に決まらない。
    // 任意のWarへ帰属させず、従来どおり変更しない(逆方向の既存の残課題として別件記録)。
    let mut w = world(false);
    let a = w.wars.add_war(war(X, Y, 10));
    let _e = w.wars.add_war(war(Y, W, 3));
    occupy(&mut w, 3, Y);
    occupy(&mut w, 3, X);
    occupy(&mut w, 10, X);

    settle(&mut w, a, PeaceTerm::WhitePeace, "T");

    assert_eq!(ctrl(&w, 3), X);
    assert_eq!(ctrl(&w, 10), Y);
}

// ── 同日の複数終結 ───────────────────────────────────────────────────────

#[test]
fn two_wars_capitulating_the_same_day_both_win_in_either_order() {
    for swap in [false, true] {
        let mut w = world(false);
        let a = w.wars.add_war(war(X, Y, 10));
        let b = w.wars.add_war(war(Z, Y, 11));
        occupy(&mut w, 10, X);
        occupy(&mut w, 11, Z);
        for id in [a, b] {
            let c = evaluate_war_capitulation(w.wars.wars.get(&id).unwrap(), &w.states, &w.mil);
            assert_eq!(
                c,
                CapitulationResult::DefenderCapitulated,
                "前提: 両Warとも降伏条件成立"
            );
        }
        let order = if swap { [b, a] } else { [a, b] };

        capitulation_step(&mut w, &order);

        assert_eq!(
            w.wars.wars[&a].status,
            WarStatus::AttackerVictory,
            "swap={swap}"
        );
        assert_eq!(
            w.wars.wars[&b].status,
            WarStatus::AttackerVictory,
            "swap={swap}"
        );
        assert_eq!(owner(&w, 10), X, "swap={swap}");
        assert_eq!(owner(&w, 11), Z, "swap={swap}");
    }
}

#[test]
fn three_wars_ending_the_same_day_reach_the_same_state_in_all_orders() {
    let orders: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut snapshots: Vec<String> = Vec::new();
    for ord in orders {
        let mut w = world(false);
        let ids = [
            w.wars.add_war(war(X, Y, 10)), // 降伏(割譲)
            w.wars.add_war(war(Z, Y, 11)), // 白紙(E2相当)
            w.wars.add_war(war(W, X, 1)),  // 白紙
        ];
        occupy(&mut w, 10, X);
        occupy(&mut w, 11, Z);
        occupy(&mut w, 1, W);
        w.mil.add_division(division(X, 10, DivisionStatus::Idle));
        w.mil.add_division(division(Z, 11, DivisionStatus::Idle));
        w.mil.add_division(division(W, 1, DivisionStatus::Idle));
        for k in ord {
            if k == 0 {
                settle(
                    &mut w,
                    ids[k],
                    PeaceTerm::CedeWarGoalRegion,
                    "Defender Capitulation",
                );
            } else {
                settle(
                    &mut w,
                    ids[k],
                    PeaceTerm::WhitePeace,
                    "Frontline Unreachable",
                );
            }
        }
        snapshots.push(snapshot(&w));
    }
    for (i, s) in snapshots.iter().enumerate() {
        assert_eq!(
            s, &snapshots[0],
            "order {:?} differs from {:?}",
            orders[i], orders[0]
        );
    }
}

// ── 既存の公開API互換 ────────────────────────────────────────────────────

#[test]
fn legacy_two_argument_relocation_still_relocates_every_division_in_hostile_territory() {
    use strategy_game::war::peace::relocate_hostile_territory_divisions;
    let mut w = world(false);
    // 他のActive Warの有無に関わらず、2引数版は従来どおり全Divisionを対象にする。
    let zd = w.mil.add_division(division(Z, 12, DivisionStatus::Moving));
    relocate_hostile_territory_divisions(&w.states, &mut w.mil);
    assert_eq!(w.mil.divisions[&zd].current_state, StateId(2));
    assert_eq!(w.mil.divisions[&zd].status, DivisionStatus::Idle);
}
