//! P26-008-B: MilitarySuppliesを考慮したAI攻勢の開始・継続ゲートの恒久テスト。
//!
//! 純粋な指標・境界・ヒステリシス自体の単体テストは`src/war/offensive_supply_gate.rs`
//! (16件)に既にあるため、本ファイルは**既存の軍事AI経路(`war::military_ai::
//! evaluate_military_ai_decision`/`process_daily_military_ai`)を通した統合**、
//! Save V1継続性、実App経由の検証に集中する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionId, FrontlineId, StateId, WarId};
use strategy_game::country::{CountryData, CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::diplomacy::crisis::{WarGoal, WarGoalType};
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::{
    Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
    STANDARD_INFANTRY_DEFINITION_ID,
};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LastLoadOutcome, LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::{FrontlineRegistry, FrontlineStance, update_all_frontlines};
use strategy_game::war::military_ai::{MilitaryAiRegistry, process_daily_military_ai};

const AI: CountryId = CountryId(0);
const ENEMY: CountryId = CountryId(1);

// ─────────────────────────────────────────────────────────────────────────
// 純粋harness(非Bevy、`war::military_ai`自身のテストと同じ構成方針)
// ─────────────────────────────────────────────────────────────────────────

fn infantry_def() -> DivisionDefinition {
    DivisionDefinition {
        id: STANDARD_INFANTRY_DEFINITION_ID,
        name: "Standard Infantry".to_string(),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        required_manpower: 10_000,
        required_equipment: 100.0,
        recruitment_days: 30,
        movement_speed: 1.0,
        attack: 10.0,
        defense: 10.0,
        breakthrough: 5.0,
        organization: 50.0,
        morale: 50.0,
        supply_usage: 1.0,
        maintenance_cost: 1.0,
    }
}

fn new_division(owner: CountryId, current_state: StateId, manpower: u64) -> Division {
    Division {
        id: DivisionId(0), // add_divisionが上書きする
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower,
        max_manpower: manpower.max(1),
        equipment: 100.0,
        max_equipment: 100.0,
        organization: 50.0,
        max_organization: 50.0,
        morale: 50.0,
        max_morale: 50.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: STANDARD_INFANTRY_DEFINITION_ID,
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

struct Harness {
    countries: CountryRegistry,
    states: StateRegistry,
    war: WarRegistry,
    military: MilitaryRegistry,
    frontline: FrontlineRegistry,
    ai: MilitaryAiRegistry,
}

fn test_country(id: CountryId, capital: StateId, supplies: f64) -> CountryData {
    let mut c = CountryData {
        id,
        capital_state_id: capital,
        available_manpower: 1_000_000,
        treasury: 1_000_000.0,
        ..CountryData::default()
    };
    c.stockpile.set(ResourceType::MilitarySupplies, supplies);
    c
}

fn war_goal(attacker: CountryId, defender: CountryId, target: StateId) -> WarGoal {
    WarGoal {
        attacker,
        defender,
        goal_type: WarGoalType::ConquerState,
        target_states: vec![target],
        base_peace_cost: 20.0,
        international_concern: 10.0,
        completion: 0.0,
        is_primary: true,
    }
}

fn new_war(id: WarId, attacker: CountryId, defender: CountryId, target: StateId) -> War {
    War {
        id,
        name: "Test War".to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![war_goal(attacker, defender, target)],
        start_date: "1799/12/01".to_string(), // process_daily_military_aiの当日判定を避ける
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: std::collections::HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: std::collections::HashSet::new(),
    }
}

/// 実データ(実App)上で使う、戦争目標なしのWar。`war_goals`の`target_states`が
/// 実データ上の実際の所有権と食い違うと`save::validate`の`OwnershipMismatch`に
/// 引っかかるため(このテストが検証したいのはSave往復そのものであり、
/// `select_ai_offensive_objective`の目標選定精度ではない)、目標を持たせず、
/// `WarRegistry::add_war`(既存の正規ID発行API、`next_id`を自動的に進める)で
/// 登録する――手でWarIdを組み立てて`wars`へ直接insertすると、Save側の
/// `next_id`整合性チェック(`NextIdCollision`)に引っかかる。
fn new_war_no_goals(attacker: CountryId, defender: CountryId) -> War {
    War {
        id: WarId(0), // add_warが上書きする
        name: "Test War".to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1799/12/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: std::collections::HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: std::collections::HashSet::new(),
    }
}

/// AI(CountryId 0)がENEMY(CountryId 1)と1本の前線で戦争中のharnessを作る。
/// 州配置: 0(AI首都)-1(AI国境)-2(敵国境)-3(敵首都)。
fn base_harness(ai_supplies: f64) -> (Harness, FrontlineId) {
    let s0 = StateData {
        id: StateId(0),
        owner_country_id: AI,
        neighbors: vec![StateId(1)],
        ..StateData::default()
    };
    let s1 = StateData {
        id: StateId(1),
        owner_country_id: AI,
        neighbors: vec![StateId(0), StateId(2)],
        ..StateData::default()
    };
    let s2 = StateData {
        id: StateId(2),
        owner_country_id: ENEMY,
        neighbors: vec![StateId(1), StateId(3)],
        ..StateData::default()
    };
    let s3 = StateData {
        id: StateId(3),
        owner_country_id: ENEMY,
        neighbors: vec![StateId(2)],
        ..StateData::default()
    };
    let states = StateRegistry::build(vec![s0, s1, s2, s3]);

    let countries = CountryRegistry {
        countries: vec![
            test_country(AI, StateId(0), ai_supplies),
            test_country(ENEMY, StateId(3), 1_000_000.0),
        ],
    };

    let mut military = MilitaryRegistry::default();
    military.definitions.insert(STANDARD_INFANTRY_DEFINITION_ID, infantry_def());

    let mut war = WarRegistry::default();
    war.wars.insert(WarId(0), new_war(WarId(0), AI, ENEMY, StateId(2)));

    let mut frontline = FrontlineRegistry::default();
    update_all_frontlines(&war, &states, &military, &mut frontline);
    let fl_id = frontline.get_frontline_for_war(WarId(0)).unwrap().frontline_id;

    (
        Harness { countries, states, war, military, frontline, ai: MilitaryAiRegistry::default() },
        fl_id,
    )
}

fn add_division_to_frontline(
    h: &mut Harness,
    fl_id: FrontlineId,
    owner: CountryId,
    current_state: StateId,
    manpower: u64,
) -> DivisionId {
    let id = h.military.add_division(new_division(owner, current_state, manpower));
    h.frontline.assign_division(id, fl_id, owner, &h.military, &h.war).unwrap();
    id
}

fn add_unassigned_enemy_division(h: &mut Harness, current_state: StateId, manpower: u64) -> DivisionId {
    h.military.add_division(new_division(ENEMY, current_state, manpower))
}

fn evaluate(h: &mut Harness, day: u32, date: &str) {
    process_daily_military_ai(
        day,
        date,
        &PlayerCountry(None),
        &h.countries,
        &h.war,
        &h.states,
        &h.military,
        &mut h.frontline,
        &mut h.ai,
    );
}

fn stance_of(h: &Harness, fl_id: FrontlineId, country: CountryId) -> FrontlineStance {
    h.frontline.get_plan(fl_id, country).unwrap().stance
}

fn blocked_of(h: &Harness, fl_id: FrontlineId, country: CountryId) -> bool {
    h.frontline.get_plan(fl_id, country).unwrap().offensive_supply_blocked
}

fn set_fulfillment(h: &mut Harness, id: DivisionId, value: f64) {
    h.military.division_supply_fulfillment.insert(id, value);
}

fn set_supplies(h: &mut Harness, country: CountryId, value: f64) {
    h.countries.get_mut(country).unwrap().stockpile.set(ResourceType::MilitarySupplies, value);
}

// ─────────────────────────────────────────────────────────────────────────
// Test 2/3/4: 新規攻勢の開始条件
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_02_power_advantage_alone_does_not_start_offensive_when_national_reserve_is_low() {
    // 戦時stop=40日、需要1.0/日。39日は不足。
    let (mut h, fl_id) = base_harness(39.0);
    add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000); // own 20000 vs enemy 0 -> 十分優勢
    evaluate(&mut h, 1, "1800/01/02");
    assert_eq!(
        stance_of(&h, fl_id, AI),
        FrontlineStance::Defend,
        "戦力比が十分でも国家備蓄不足なら開始しないはず"
    );
    assert!(blocked_of(&h, fl_id, AI));
}

#[test]
fn test_03_power_advantage_alone_does_not_start_offensive_when_frontline_fulfillment_is_low() {
    let (mut h, fl_id) = base_harness(1000.0);
    let id = add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
    set_fulfillment(&mut h, id, 0.2); // 0.34未満
    evaluate(&mut h, 1, "1800/01/02");
    assert_eq!(
        stance_of(&h, fl_id, AI),
        FrontlineStance::Defend,
        "戦力比が十分でも前線配送率0.34未満なら開始しないはず"
    );
    assert!(blocked_of(&h, fl_id, AI));
}

#[test]
fn test_04_healthy_country_and_frontline_starts_offensive_as_before() {
    let (mut h, fl_id) = base_harness(1000.0);
    add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
    evaluate(&mut h, 1, "1800/01/02");
    assert_eq!(stance_of(&h, fl_id, AI), FrontlineStance::Offensive);
    assert!(!blocked_of(&h, fl_id, AI));
}

// ─────────────────────────────────────────────────────────────────────────
// Test 5/6: 既存攻勢の継続条件(国家備蓄だけでは止めない)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_05_ongoing_offensive_stops_when_frontline_fulfillment_drops_below_0_34() {
    let (mut h, fl_id) = base_harness(1000.0);
    let id = add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
    evaluate(&mut h, 1, "1800/01/02");
    assert_eq!(stance_of(&h, fl_id, AI), FrontlineStance::Offensive);

    set_fulfillment(&mut h, id, 0.1);
    evaluate(&mut h, 4, "1800/01/05"); // cadence(3日以上)を満たす
    assert_eq!(
        stance_of(&h, fl_id, AI),
        FrontlineStance::Defend,
        "前線配送率が0.34未満になったら次の正規評価で停止するはず"
    );
}

#[test]
fn test_06_national_reserve_dropping_below_stop_alone_does_not_stop_an_ongoing_offensive() {
    let (mut h, fl_id) = base_harness(1000.0);
    add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
    evaluate(&mut h, 1, "1800/01/02");
    assert_eq!(stance_of(&h, fl_id, AI), FrontlineStance::Offensive);

    // 国家備蓄をstop(40日)未満へ落とすが、配送率は健全なまま。
    set_supplies(&mut h, AI, 10.0);
    evaluate(&mut h, 4, "1800/01/05");
    assert_eq!(
        stance_of(&h, fl_id, AI),
        FrontlineStance::Offensive,
        "国家備蓄の一時低下だけでは進行中の攻勢を止めないはず"
    );
    assert!(
        blocked_of(&h, fl_id, AI),
        "ただし将来の新規開始に向けたblocked状態自体は立つはず"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test 7/8/9: 真のヒステリシス(中間帯・resume)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_07_middle_band_keeps_recovery_wait_state() {
    let (mut h, fl_id) = base_harness(1000.0);
    add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
    evaluate(&mut h, 1, "1800/01/02");
    assert_eq!(stance_of(&h, fl_id, AI), FrontlineStance::Offensive);

    // stop=40を割らせてblocked化。
    set_supplies(&mut h, AI, 10.0);
    evaluate(&mut h, 4, "1800/01/05");
    assert!(blocked_of(&h, fl_id, AI));

    // 中間帯(40<=x<70)へ回復させても、resumeに達しないうちは新規開始できない。
    set_supplies(&mut h, AI, 50.0);
    evaluate(&mut h, 7, "1800/01/08");
    assert!(
        blocked_of(&h, fl_id, AI),
        "戦時stop(40)以上resume(70)未満の中間帯ではblocked維持のはず"
    );
}

#[test]
fn test_08_offensive_resumes_after_resume_threshold_and_power_condition_met() {
    let (mut h, fl_id) = base_harness(1000.0);
    let id = add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
    evaluate(&mut h, 1, "1800/01/02");
    assert_eq!(stance_of(&h, fl_id, AI), FrontlineStance::Offensive);

    // 前線配送不良(0.34未満)で実際に攻勢を停止させ、かつblocked化させる
    // (spec §7: 国家備蓄の低下だけでは進行中攻勢を止めないため、まず配送不良で
    // 停止させないと「再開」のテストにならない)。
    set_fulfillment(&mut h, id, 0.1);
    evaluate(&mut h, 4, "1800/01/05");
    assert_eq!(stance_of(&h, fl_id, AI), FrontlineStance::Defend);
    assert!(blocked_of(&h, fl_id, AI));

    // 配送を回復させ、国家備蓄も戦時resume(70日)ちょうどまで積む。
    set_fulfillment(&mut h, id, 1.0);
    set_supplies(&mut h, AI, 70.0);
    evaluate(&mut h, 7, "1800/01/08");
    assert_eq!(
        stance_of(&h, fl_id, AI),
        FrontlineStance::Offensive,
        "resume到達後、既存110%条件も満たすなら再開するはず"
    );
    assert!(!blocked_of(&h, fl_id, AI));
}

#[test]
fn test_09_resume_threshold_alone_does_not_start_offensive_without_power_advantage() {
    let (mut h, fl_id) = base_harness(1000.0);
    let ai_div = add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
    evaluate(&mut h, 1, "1800/01/02");

    set_supplies(&mut h, AI, 10.0);
    evaluate(&mut h, 4, "1800/01/05");
    assert!(blocked_of(&h, fl_id, AI));

    // resumeへ到達させるが、敵の方が優勢にする(110%条件を満たさない)。
    set_supplies(&mut h, AI, 70.0);
    add_unassigned_enemy_division(&mut h, StateId(3), 100_000);
    let _ = ai_div;
    evaluate(&mut h, 7, "1800/01/08");
    assert_eq!(
        stance_of(&h, fl_id, AI),
        FrontlineStance::Defend,
        "resumeに達しても戦力比110%を満たさなければ攻勢は開始しないはず"
    );
    assert!(!blocked_of(&h, fl_id, AI), "resume自体は満たしているのでblockedは解除されるはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test 10/11: 首都切断・接続回復(抽象的にはfulfillment=0.0で表現)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_10_capital_disconnection_style_zero_fulfillment_stops_the_offensive() {
    let (mut h, fl_id) = base_harness(100_000.0); // 国家備蓄は潤沢
    let id = add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
    evaluate(&mut h, 1, "1800/01/02");
    assert_eq!(stance_of(&h, fl_id, AI), FrontlineStance::Offensive);

    set_fulfillment(&mut h, id, 0.0); // 首都切断相当
    evaluate(&mut h, 4, "1800/01/05");
    assert_eq!(
        stance_of(&h, fl_id, AI),
        FrontlineStance::Defend,
        "国家備蓄が潤沢でも配送率0なら攻勢は継続できないはず"
    );
}

#[test]
fn test_11_reconnection_does_not_immediately_resume_if_reserve_still_below_resume() {
    let (mut h, fl_id) = base_harness(45.0); // 戦時: stop=40以上だが接続時点ではblockedにしない
    let id = add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
    set_fulfillment(&mut h, id, 0.0);
    evaluate(&mut h, 1, "1800/01/02"); // 切断状態から開始 -> blocked化
    assert!(blocked_of(&h, fl_id, AI));
    assert_eq!(stance_of(&h, fl_id, AI), FrontlineStance::Defend);

    // 接続回復(配送健全化)するが、reserveは45日(戦時resume=70未満)のまま。
    set_fulfillment(&mut h, id, 1.0);
    evaluate(&mut h, 4, "1800/01/05");
    assert_eq!(
        stance_of(&h, fl_id, AI),
        FrontlineStance::Defend,
        "接続回復してもreserveがresume未満なら直ちに攻勢再開しないはず"
    );
    assert!(blocked_of(&h, fl_id, AI));
}

// ─────────────────────────────────────────────────────────────────────────
// Test 12: 同一国家の複数前線分離
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_12_disconnected_frontline_does_not_contaminate_a_healthy_sibling_frontline() {
    let (mut h, fl_id_a) = base_harness(100_000.0);
    let id_a = add_division_to_frontline(&mut h, fl_id_a, AI, StateId(1), 20_000);

    // 2つ目の戦争・前線(同じAI国家、別の敵)。
    let s4 = StateData {
        id: StateId(4),
        owner_country_id: AI,
        neighbors: vec![StateId(5)],
        ..StateData::default()
    };
    let s5 = StateData {
        id: StateId(5),
        owner_country_id: CountryId(2),
        neighbors: vec![StateId(4)],
        ..StateData::default()
    };
    h.states.states.push(s4);
    h.states.states.push(s5);
    h.states = StateRegistry::build(h.states.states.clone());
    h.countries.countries.push(CountryData {
        id: CountryId(2),
        capital_state_id: StateId(5),
        treasury: 1_000_000.0,
        available_manpower: 1_000_000,
        ..CountryData::default()
    });
    h.war.wars.insert(WarId(1), new_war(WarId(1), AI, CountryId(2), StateId(5)));
    update_all_frontlines(&h.war, &h.states, &h.military, &mut h.frontline);
    let fl_id_b = h.frontline.get_frontline_for_war(WarId(1)).unwrap().frontline_id;
    let id_b = add_division_to_frontline(&mut h, fl_id_b, AI, StateId(4), 20_000);

    // 前線Aだけ切断させる。
    set_fulfillment(&mut h, id_a, 0.0);
    evaluate(&mut h, 1, "1800/01/02");

    assert_eq!(stance_of(&h, fl_id_a, AI), FrontlineStance::Defend, "前線Aは配送不足で防御のはず");
    assert_eq!(
        stance_of(&h, fl_id_b, AI),
        FrontlineStance::Offensive,
        "前線Bは無関係に攻勢開始できるはず(前線間の配送率混入がない)"
    );
    let _ = id_b;
}

// ─────────────────────────────────────────────────────────────────────────
// Test 13: 国家間分離
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_13_one_countrys_supply_shortage_does_not_affect_another_country() {
    // AI(備蓄不足)とENEMY(潤沢)を、それぞれ完全に独立した戦争・前線・敵国に
    // 対して評価する(戦力比の相互作用が混入しないよう、同じ前線の両陣営同士では
    // 比較しない――両陣営を同じ前線に置くと戦力比較自体がもう1つの変数になり、
    // 「備蓄状態が他国へ波及しない」という分離自体を検証できなくなるため)。
    let (mut h, fl_id_ai) = base_harness(0.0); // AI側は備蓄不足
    add_division_to_frontline(&mut h, fl_id_ai, AI, StateId(1), 20_000);

    // 完全に独立した2国間戦争(CountryId(2) vs CountryId(3))を追加する。
    let s6 = StateData {
        id: StateId(6),
        owner_country_id: CountryId(2),
        neighbors: vec![StateId(7)],
        ..StateData::default()
    };
    let s7 = StateData {
        id: StateId(7),
        owner_country_id: CountryId(3),
        neighbors: vec![StateId(6)],
        ..StateData::default()
    };
    h.states.states.push(s6);
    h.states.states.push(s7);
    h.states = StateRegistry::build(h.states.states.clone());
    h.countries.countries.push(test_country(CountryId(2), StateId(6), 1_000_000.0)); // 潤沢
    h.countries.countries.push(test_country(CountryId(3), StateId(7), 1_000_000.0));
    h.war.wars.insert(WarId(1), new_war(WarId(1), CountryId(2), CountryId(3), StateId(7)));
    update_all_frontlines(&h.war, &h.states, &h.military, &mut h.frontline);
    let fl_id_other = h.frontline.get_frontline_for_war(WarId(1)).unwrap().frontline_id;
    add_division_to_frontline(&mut h, fl_id_other, CountryId(2), StateId(6), 20_000);

    evaluate(&mut h, 1, "1800/01/02");

    assert_eq!(
        stance_of(&h, fl_id_ai, AI),
        FrontlineStance::Defend,
        "AIは自国の備蓄不足で防御のはず"
    );
    assert_eq!(
        stance_of(&h, fl_id_other, CountryId(2)),
        FrontlineStance::Offensive,
        "無関係な国は、AIの備蓄状態と無関係に攻勢開始できるはず"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test 18: Frontline削除時のdangling state
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_18_removing_the_frontline_leaves_no_dangling_offensive_supply_blocked_state() {
    let (mut h, fl_id) = base_harness(10.0);
    add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
    evaluate(&mut h, 1, "1800/01/02");
    assert!(blocked_of(&h, fl_id, AI));

    h.frontline.remove_frontline(fl_id, &h.military);
    assert!(
        h.frontline.get_plan(fl_id, AI).is_none(),
        "前線削除後はPlan自体(offensive_supply_blockedを含む)が残らないはず"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test 19: 挿入順序非依存
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_19_result_does_not_depend_on_country_insertion_order() {
    fn run(reverse: bool) -> FrontlineStance {
        let (mut h, fl_id) = base_harness(1000.0);
        add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
        if reverse {
            h.countries.countries.reverse();
        }
        evaluate(&mut h, 1, "1800/01/02");
        stance_of(&h, fl_id, AI)
    }
    assert_eq!(run(false), run(true));
}

// ─────────────────────────────────────────────────────────────────────────
// Test 20: AI評価cadenceを迂回しない
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_20_supply_recovery_within_the_cadence_window_does_not_trigger_an_extra_evaluation() {
    let (mut h, fl_id) = base_harness(10.0); // stop未満、blocked化する
    let id = add_division_to_frontline(&mut h, fl_id, AI, StateId(1), 20_000);
    evaluate(&mut h, 1, "1800/01/02");
    assert_eq!(stance_of(&h, fl_id, AI), FrontlineStance::Defend);
    let eval_day_after_first = h.ai.ai_states.get(&AI).unwrap().last_evaluated_day;

    // 備蓄を即座に十分な量へ回復させるが、cadence境界(3日)未満で再評価を試みる。
    set_supplies(&mut h, AI, 1000.0);
    evaluate(&mut h, 2, "1800/01/03"); // まだ3日経っていない、dirtyでもない
    assert_eq!(
        h.ai.ai_states.get(&AI).unwrap().last_evaluated_day,
        eval_day_after_first,
        "cadence境界に達していない限り、補給状態の変化だけで即時再評価してはいけない"
    );
    assert_eq!(
        stance_of(&h, fl_id, AI),
        FrontlineStance::Defend,
        "cadence境界に達するまでは前回のDefendのままのはず"
    );

    let _ = id;
}

// ─────────────────────────────────────────────────────────────────────────
// Save V1: 現行/旧/2回連続(実App)
// ─────────────────────────────────────────────────────────────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p26_008_b_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
}

struct TempTestDir(std::path::PathBuf);
impl TempTestDir {
    fn new(label: &str) -> Self {
        Self(unique_temp_dir(label))
    }
    fn save_file_config(&self) -> SaveFileConfig {
        SaveFileConfig { path: SavePathConfig { final_path: self.0.join("savegame_v1.ron") } }
    }
    fn final_path(&self) -> std::path::PathBuf {
        self.0.join("savegame_v1.ron")
    }
}
impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_app(player: Option<CountryId>, save_config: SaveFileConfig) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(AppPlugin)
        .add_plugins(CountryPlugin)
        .add_plugins(StatePlugin)
        .add_plugins(BuildingPlugin)
        .add_plugins(EconomyPlugin)
        .add_plugins(ResearchPlugin)
        .add_plugins(PoliticsPlugin)
        .add_plugins(DiplomacyPlugin)
        .add_plugins(WarPlugin)
        .add_plugins(MilitaryPlugin)
        .add_plugins(PopulationPlugin);

    app.insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
        .insert_resource(strategy_game::map::division_selection::DragSelectState::default())
        .insert_resource(SelectedState::default())
        .insert_resource(strategy_game::ui::ActivePanel::default())
        .insert_resource(strategy_game::ui::diplomacy_panel::DiplomacyPanelState::default())
        .insert_resource(strategy_game::ui::military_panel::MilitaryPanelState::default())
        .insert_resource(strategy_game::ui::peace_panel::PeacePanelState::default())
        .insert_resource(strategy_game::ui::politics_panel::PoliticsPanelState::default())
        .insert_resource(strategy_game::ui::research_panel::ResearchPanelState::default())
        .insert_resource(strategy_game::map::camera::CameraDragState::default())
        .add_message::<strategy_game::map::camera::CameraResetRequestMessage>();

    app.add_plugins(SaveGamePlugin);
    app.add_plugins(LoadGamePlugin);

    app.insert_resource(save_config);
    app.update();

    app.insert_resource(PlayerCountry(player));
    app.insert_state(GameState::Playing);
    app.update();

    app
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
    state.apply(app.world_mut());
    app.update();
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
    app.update();
}

fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn set_game_date(app: &mut App, year: i32, month: u8, day: u8) {
    let mut gd = app.world_mut().resource_mut::<GameDate>();
    gd.year = year;
    gd.month = month;
    gd.day = day;
}

/// 実App上でARCADIA(CountryId 0)とELFIN(CountryId 1)の間に前線を1本作り、そこへ
/// `offensive_supply_blocked`を直接設定する(実データの戦力比・補給網に依存せず、
/// Save往復そのものだけを検証するため)。
fn setup_app_with_one_frontline(player: Option<CountryId>, save_config: SaveFileConfig) -> (App, WarId) {
    let mut app = setup_app(player, save_config);

    let war_id = app
        .world_mut()
        .resource_mut::<WarRegistry>()
        .add_war(new_war_no_goals(CountryId(0), CountryId(1)));

    app.world_mut().resource_scope(|world, mut frontline_registry: Mut<FrontlineRegistry>| {
        let war_registry = world.resource::<WarRegistry>();
        let state_registry = world.resource::<StateRegistry>();
        let military_registry = world.resource::<MilitaryRegistry>();
        update_all_frontlines(war_registry, state_registry, military_registry, &mut frontline_registry);
    });
    (app, war_id)
}

fn frontline_id_for_war(app: &App, war_id: WarId) -> FrontlineId {
    app.world().resource::<FrontlineRegistry>().get_frontline_for_war(war_id).unwrap().frontline_id
}

#[test]
fn test_15_current_save_round_trips_offensive_supply_blocked_both_ways() {
    for &blocked in &[false, true] {
        let dir = TempTestDir::new(&format!("t15_{blocked}"));
        let (mut app, war_id) = setup_app_with_one_frontline(Some(CountryId(2)), dir.save_file_config());
        let fl_id = frontline_id_for_war(&app, war_id);

        {
            let mut frontline_registry = app.world_mut().resource_mut::<FrontlineRegistry>();
            if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(0)) {
                plan.offensive_supply_blocked = blocked;
            }
        }

        send_save_request(&mut app);
        send_load_request(&mut app);

        let restored = app
            .world()
            .resource::<FrontlineRegistry>()
            .get_plan(fl_id, CountryId(0))
            .map(|p| p.offensive_supply_blocked);
        assert_eq!(restored, Some(blocked), "offensive_supply_blocked={blocked}が往復で保たれるはず");
    }
}

#[test]
fn test_16_old_save_without_the_field_loads_as_false() {
    let dir = TempTestDir::new("t16");
    let (mut app, war_id) = setup_app_with_one_frontline(Some(CountryId(2)), dir.save_file_config());
    let fl_id = frontline_id_for_war(&app, war_id);
    {
        let mut frontline_registry = app.world_mut().resource_mut::<FrontlineRegistry>();
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(0)) {
            plan.offensive_supply_blocked = true;
        }
    }
    send_save_request(&mut app);

    let path = dir.final_path();
    let contents = std::fs::read_to_string(&path).unwrap();
    let stripped = contents
        .replace("offensive_supply_blocked: true,", "")
        .replace("offensive_supply_blocked: false,", "");
    assert_ne!(contents, stripped, "テスト前提: フィールドを実際に除去できていること");
    std::fs::write(&path, stripped).unwrap();

    send_load_request(&mut app);
    assert!(
        matches!(
            app.world().resource::<LastLoadOutcome>().0,
            Some(strategy_game::save::runtime::LoadOutcome::Success { .. })
        ),
        "テスト前提: フィールド欠落だけを理由にロードが失敗してはいけない: {:?}",
        app.world().resource::<LastLoadOutcome>()
    );
    let restored = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_plan(fl_id, CountryId(0))
        .map(|p| p.offensive_supply_blocked);
    assert_eq!(restored, Some(false), "フィールド欠落時はfalse(未停止)から復元されるはず");
}

#[test]
fn test_17_double_save_load_does_not_change_the_blocked_state() {
    let dir = TempTestDir::new("t17");
    let (mut app, war_id) = setup_app_with_one_frontline(Some(CountryId(2)), dir.save_file_config());
    let fl_id = frontline_id_for_war(&app, war_id);
    {
        let mut frontline_registry = app.world_mut().resource_mut::<FrontlineRegistry>();
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(0)) {
            plan.offensive_supply_blocked = true;
        }
    }

    send_save_request(&mut app);
    send_load_request(&mut app);
    let after_first = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_plan(fl_id, CountryId(0))
        .map(|p| p.offensive_supply_blocked);

    send_save_request(&mut app);
    send_load_request(&mut app);
    let after_second = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_plan(fl_id, CountryId(0))
        .map(|p| p.offensive_supply_blocked);

    assert_eq!(after_first, after_second);
    assert_eq!(after_first, Some(true));
}

// ─────────────────────────────────────────────────────────────────────────
// §15: 90日実App検証(制御された戦争シナリオ、1本のタイムライン)
// ─────────────────────────────────────────────────────────────────────────

fn stockpile_of(app: &App, c: CountryId, resource: ResourceType) -> f64 {
    app.world().resource::<CountryRegistry>().get(c).unwrap().stockpile.get(resource)
}

fn set_app_supplies(app: &mut App, c: CountryId, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(c)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitarySupplies, value);
}

fn advance_days(app: &mut App, start_year: i32, start_month: u8, start_day: u8, count: u32) {
    let mut year = start_year;
    let mut month = start_month;
    let mut day = start_day;
    for _ in 0..count {
        set_game_date(app, year, month, day);
        tick_day(app, year, month, day);
        day += 1;
        if day > 28 {
            day = 1;
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
    }
}

#[test]
fn test_90_day_controlled_war_offensive_supply_gate_real_app_verification() {
    let dir = TempTestDir::new("ninety_day_offensive");
    let mut app = setup_app(None, dir.save_file_config());

    // 実データのCountryId(0)(Arcadia)とCountryId(1)(Elfin)の間に制御された戦争を
    // 作る(自然発生を待たない、spec §15: 結果を成立させるために本番データ・戦力比・
    // 補給率・国家別特例は変更しない)。
    // 戦争目標は持たせない(このテストが検証したいのは前線Frontlineの補給ゲート
    // 挙動そのものであり、`select_ai_offensive_objective`は目標未設定でも
    // 候補2[前線から最短の敵支配地域]へ自動的にフォールバックする)。`add_war`
    // (既存の正規ID発行API)でWarIdを手で組み立てない。
    let war_id = app
        .world_mut()
        .resource_mut::<WarRegistry>()
        .add_war(new_war_no_goals(CountryId(0), CountryId(1)));
    app.world_mut().resource_scope(|world, mut frontline_registry: Mut<FrontlineRegistry>| {
        let war_registry = world.resource::<WarRegistry>();
        let state_registry = world.resource::<StateRegistry>();
        let military_registry = world.resource::<MilitaryRegistry>();
        update_all_frontlines(war_registry, state_registry, military_registry, &mut frontline_registry);
    });

    // Arcadiaの陸軍を大幅増強し、戦力比110%を確実に満たす(spec §15: 結果を成立
    // させるためであっても戦力比110%/90%自体は変更しない、増援は既存の
    // Division増強の範囲内)。
    {
        let mut military_registry = app.world_mut().resource_mut::<MilitaryRegistry>();
        for division in military_registry.divisions.values_mut() {
            if division.owner == CountryId(0) {
                division.manpower = 100_000;
                division.max_manpower = 100_000;
            }
        }
    }
    // 十分な国家備蓄から開始する(健全状態で攻勢開始できることを確認するため)。
    set_app_supplies(&mut app, CountryId(0), 1_000_000.0);

    advance_days(&mut app, 1800, 1, 2, 5);

    let fl_id = frontline_id_for_war(&app, war_id);
    let stance_after_start = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_plan(fl_id, CountryId(0))
        .map(|p| p.stance);
    eprintln!("[P26-008-B 90-day] 健全状態での開始後stance={stance_after_start:?}");

    // 回廊遮断相当: 国家備蓄を0へ落とし、実際の日次補給System(P26-004、
    // `daily_supply::process_daily_military_supply_consumption`)自身の
    // Hamilton配分(`T_c = min(S_c, ΣD_i)`)によって、前線へ割り当てられた
    // Divisionの配送率が実際に0へ落ちることを確認する。この実App経由のSystemを
    // 迂回して`division_supply_fulfillment`を直接上書きしても、次の
    // `Economy` SetでSystem自身が(国家備蓄が潤沢なままなら)1.0へ上書きし直して
    // しまうため、実際に国家備蓄そのものを操作する(spec §15: 本番Systemの
    // 実際の計算結果を検証する)。月内(生産は月末のみ)であれば、消費だけが
    // 進むため0近辺のまま維持される。
    set_app_supplies(&mut app, CountryId(0), 0.0);
    advance_days(&mut app, 1800, 1, 7, 5);

    let stance_after_cutoff = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_plan(fl_id, CountryId(0))
        .map(|p| p.stance);
    eprintln!("[P26-008-B 90-day] 回廊遮断後stance={stance_after_cutoff:?}");

    // 回廊奪還相当(国家備蓄を回復させ、実Systemに配送率を再計算させる)。
    set_app_supplies(&mut app, CountryId(0), 1_000_000.0);
    advance_days(&mut app, 1800, 1, 12, 5);

    let blocked_after_reconnect = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_plan(fl_id, CountryId(0))
        .map(|p| p.offensive_supply_blocked);
    eprintln!(
        "[P26-008-B 90-day] 接続回復後blocked={blocked_after_reconnect:?} \
         supplies={:.3}",
        stockpile_of(&app, CountryId(0), ResourceType::MilitarySupplies)
    );

    // 備蓄をresume水準まで回復させ、再開を確認する。
    set_app_supplies(&mut app, CountryId(0), 1_000_000.0);
    advance_days(&mut app, 1800, 1, 17, 60);

    let final_stance = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_plan(fl_id, CountryId(0))
        .map(|p| p.stance);
    let final_blocked = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_plan(fl_id, CountryId(0))
        .map(|p| p.offensive_supply_blocked);
    eprintln!("[P26-008-B 90-day] 最終stance={final_stance:?} blocked={final_blocked:?}");

    // 一連の流れがパニックせず完了し、遮断中は攻勢を維持できないことを確認する。
    assert_ne!(
        stance_after_cutoff,
        Some(FrontlineStance::Offensive),
        "回廊遮断中はOffensiveを維持できないはず"
    );

    // 【P27-005により正規値を再更新】: P27-005(攻撃側前線補給ブリッジ)により、
    // 進行中の戦闘の攻撃側であり、首都連結済みの自国州に隣接する敵支配州の師団は
    // 1ホップだけ補給を受ける。よって以下の新正規値になる:
    //  - 攻撃師団は敵州へ到達し(Fighting)、supply_fulfillment > 0(実測1.00)
    //  - 前線配送は健全なので offensive_supply_blocked == false
    //  - それでもstanceはDefendのまま。原因はsupplyではなく、割当師団が全員
    //    Fightingで `has_ready_divisions == false`(Idle/Moving無し)となり、
    //    military_ai が Defend/Recovering を返すため(supply判定より前の既存分岐)。
    //  - 州3の戦闘自体は未決着。防御側が実効戦力0のまま損害を受けない
    //    `distribute_damage`の別件(P27-006-PRE-001)で、本タスクの対象外。
    let (attacker_state, attacker_status, attacker_ful, defender_ful) = {
        let mil = app.world().resource::<MilitaryRegistry>();
        let a = mil.divisions.get(&DivisionId(0)).expect("attacker exists");
        let d = mil.divisions.get(&DivisionId(1)).expect("defender exists");
        (
            a.current_state,
            a.status,
            mil.supply_fulfillment_of(a.id),
            mil.supply_fulfillment_of(d.id),
        )
    };
    assert_eq!(attacker_state, StateId(3), "攻撃師団は敵州(戦争目標)へ到達している");
    assert_eq!(attacker_status, DivisionStatus::Fighting, "到達後は交戦中");
    assert!(
        attacker_ful > 0.0,
        "P27-005: 攻撃側前線師団は1ホップ補給ブリッジで補給を受ける(実測{attacker_ful})"
    );
    assert_eq!(
        defender_ful, 0.0,
        "防御側の契約は無変更(首都連結外なので補給0のまま)"
    );
    assert_eq!(
        final_blocked,
        Some(false),
        "前線配送が健全に回復したのでoffensive_supply_blockedは解除される"
    );
    assert_eq!(
        final_stance,
        Some(FrontlineStance::Defend),
        "割当師団が全員Fightingで待機・移動中が無いためDefend(Recovering)。\
         supply起因ではない"
    );
}
