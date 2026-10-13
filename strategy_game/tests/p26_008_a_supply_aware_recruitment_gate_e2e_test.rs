//! P26-008-A: MilitarySuppliesを考慮した募兵停止／再開ゲートの恒久テスト。
//!
//! P26-008-PRE-001が設計し、P26-008-PRE-002が正規cadenceを復元した基盤の上に、
//! `military::recruitment_supply_gate`(純粋な指標・ヒステリシス計算)と、それを
//! `military::ai::decide_and_recruit_for_country`へ接続した結果を検証する。
//! 境界値・ヒステリシス自体の単体テストは`src/military/recruitment_supply_gate.rs`
//! (20件)に既にあるため、本ファイルは**既存の月次募兵AI経路(`evaluate_ai_monthly_
//! recruitment`/`decide_and_recruit_for_country`)を通した統合**と、**実App
//! (`handle_monthly_ai_recruitment`)経由の検証**に集中する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionId, StateId, WarId};
use strategy_game::country::country_ai::CountryAiRegistry;
use strategy_game::country::{CountryData, CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::ai::evaluate_ai_monthly_recruitment;
use strategy_game::military::data::{
    Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
    STANDARD_INFANTRY_DEFINITION_ID,
};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};

const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const DWARF: CountryId = CountryId(2);

// ─────────────────────────────────────────────────────────────────────────
// 純粋harness(非Bevy、`military::ai`自身のテストと同じ構成方針)
// ─────────────────────────────────────────────────────────────────────────

fn infantry_def(required_manpower: u64) -> DivisionDefinition {
    DivisionDefinition {
        id: STANDARD_INFANTRY_DEFINITION_ID,
        name: "Standard Infantry".to_string(),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        required_manpower,
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

fn test_state(id: usize, owner: usize, population: u64) -> StateData {
    StateData {
        id: StateId(id),
        owner_country_id: CountryId(owner),
        population,
        ..StateData::default()
    }
}

/// `mobilized_manpower`(累積動員負担)は、渡す生存Division群の`manpower`合計と
/// 呼び出し側が一致させる。`military::manpower::compute_manpower_ledger`は
/// `living_military_manpower(= 生存Division現在manpower合計 + queue予約分)`が
/// `workforce_burden(= mobilized_manpower)`を上回ると`unaccounted_living_excess`
/// を検出し`SkippedLedgerInvalid`で募兵AI全体を早期returnさせるため、0のまま
/// 放置すると(既存Divisionを持つ全テストで)補給ゲート自体に到達しなくなる。
fn test_country(id: CountryId, available_manpower: u64, supplies: f64, mobilized_manpower: u64) -> CountryData {
    let mut country = CountryData {
        id,
        capital_state_id: StateId(id.0),
        available_manpower,
        mobilized_manpower,
        treasury: 1_000_000.0,
        ..CountryData::default()
    };
    country.stockpile.set(ResourceType::MilitarySupplies, supplies);
    country
}

fn test_division(id: usize, owner: CountryId, manpower: u64) -> Division {
    Division {
        id: DivisionId(id),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(owner.0),
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
    military: MilitaryRegistry,
    states: StateRegistry,
    war: WarRegistry,
    country_ai: CountryAiRegistry,
    game_date: GameDate,
}

/// `required_manpower=10_000`・`population=200_000`(cap=20,000、target=1)を既定とする
/// 単一国家harness。呼び出し側が`countries[0].stockpile`/`divisions`/`game_date`/
/// `war`を目的に応じて上書きする。
///
/// 注意: `target=1`のため、既存Divisionを1個でも足すと`current(1) >= target(1)`で
/// `SkippedTargetMet`に達し、補給ゲート自体が評価されない。既存Divisionを持たせた
/// 状態でゲートの挙動(reserve/bridge/配送健全性)を検証したいテストは、
/// [`single_country_harness_with_population`]で`target>=2`になる人口を指定すること。
fn single_country_harness(country_id: CountryId, supplies: f64, divisions: Vec<Division>) -> Harness {
    single_country_harness_with_population(country_id, supplies, divisions, 200_000)
}

/// [`single_country_harness`]の人口指定版。`population=500_000`なら
/// `cap=50,000`・`target=(50,000/2)/10,000=2`になり、既存Divisionを1個持たせても
/// `current(1) < target(2)`のため補給ゲートまで到達する。
fn single_country_harness_with_population(
    country_id: CountryId,
    supplies: f64,
    divisions: Vec<Division>,
    population: u64,
) -> Harness {
    let mobilized_manpower: u64 = divisions.iter().map(|d| d.manpower).sum();
    let mut military = MilitaryRegistry::default();
    military.definitions.insert(STANDARD_INFANTRY_DEFINITION_ID, infantry_def(10_000));
    for d in divisions {
        military.divisions.insert(d.id, d);
    }
    Harness {
        countries: CountryRegistry {
            countries: vec![test_country(country_id, 1_000_000, supplies, mobilized_manpower)],
        },
        military,
        states: StateRegistry::build(vec![test_state(country_id.0, country_id.0, population)]),
        war: WarRegistry::default(),
        country_ai: CountryAiRegistry::default(),
        game_date: GameDate::new(1800, 1, 1),
    }
}

fn evaluate(h: &mut Harness, country_id: CountryId) {
    evaluate_ai_monthly_recruitment(
        &mut h.countries,
        &h.military,
        &h.states,
        &strategy_game::building::data::BuildingRegistry::default(),
        &h.war,
        &h.game_date,
        &mut h.country_ai,
        None,
    );
    let _ = country_id;
}

fn is_blocked(h: &Harness, country_id: CountryId) -> bool {
    h.country_ai
        .ai_states
        .get(&country_id)
        .map(|s| s.recruitment_supply_blocked)
        .unwrap_or(false)
}

fn queue_len(h: &Harness, country_id: CountryId) -> usize {
    h.countries.get(country_id).unwrap().recruitment_queue.len()
}

fn set_stockpile(h: &mut Harness, country_id: CountryId, value: f64) {
    h.countries.get_mut(country_id).unwrap().stockpile.set(ResourceType::MilitarySupplies, value);
}

fn set_blocked(h: &mut Harness, country_id: CountryId, blocked: bool) {
    h.country_ai.get_or_create_mut(country_id).recruitment_supply_blocked = blocked;
}

fn add_war(h: &mut Harness, attacker: CountryId, defender: CountryId) {
    h.war.wars.insert(
        WarId(0),
        War {
            id: WarId(0),
            name: "Test War".to_string(),
            attackers: [attacker].into_iter().collect(),
            defenders: [defender].into_iter().collect(),
            primary_attacker: None,
            primary_defender: None,
            source_crisis_id: None,
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
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
        },
    );
}

/// 1個師団の日次需要ちょうど1.0(units=1_000_000)を基準に、`days`日分の備蓄値を返す。
fn stockpile_for_days(days: f64) -> f64 {
    days
}

// ─────────────────────────────────────────────────────────────────────────
// Test A/B: 停止境界(平時/戦時)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_a_peacetime_stop_boundary_via_real_recruitment_path() {
    // 20日ちょうど: 既存Division(需要1.0/日)を1個持ち、新規1個の将来需要も
    // 合わせて2.0/日のplanning需要になる点に注意し、"現在需要"(1.0/日)基準の
    // reserve判定だけを単体で確認する(bridge判定はTest H系で別途検証)。
    // 月初(day=1)でbridge必要量も併せて満たすよう、目標日数+暦分を積む。
    let mut h = single_country_harness(ARCADIA, stockpile_for_days(20.0) + 31.0, vec![]);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 1, "20日ちょうどは停止しないはず");
    assert!(!is_blocked(&h, ARCADIA));
}

/// reserve境界系テスト(A2/B/B2/C/D/E)共通のharnessを作る。既存Division 1個
/// (需要1.0/日)を持たせ、`population=500_000`(target=2)で`SkippedTargetMet`に
/// よるゲート未評価("マスキング")を避ける。月末寄りの日付(1月30日、31日中の30日目)
/// を使うことで、bridge必要量を`planning_demand(2.0/日) × (1+1)=4.0`という小さな
/// 固定値に抑え、テストが狙うreserve日数のしきい値(20〜70)と競合させない
/// (bridge自体の検証はTest F/H/I/Sが別途担当する)。この条件下では
/// `stockpile == reserve_days`(現在需要1.0/日を分母にした日数)がそのまま成立する。
fn reserve_boundary_harness(stockpile_units: f64, at_war: bool) -> Harness {
    let mut h = single_country_harness_with_population(
        ARCADIA,
        stockpile_units,
        vec![test_division(1, ARCADIA, 10_000)],
        500_000,
    );
    h.game_date = GameDate::new(1800, 1, 30);
    if at_war {
        add_war(&mut h, ARCADIA, ELFIN);
    }
    h
}

#[test]
fn test_a2_peacetime_stop_boundary_just_below() {
    let mut h = reserve_boundary_harness(19.999, false);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 0, "20日未満は停止するはず");
    assert!(is_blocked(&h, ARCADIA));
}

#[test]
fn test_b_wartime_stop_boundary() {
    let mut h = reserve_boundary_harness(40.0, true);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 1, "戦時40日ちょうどは停止しないはず");
}

#[test]
fn test_b2_wartime_stop_boundary_just_below() {
    let mut h = reserve_boundary_harness(39.5, true);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 0, "戦時40日未満は停止するはず");
    assert!(is_blocked(&h, ARCADIA));
}

// ─────────────────────────────────────────────────────────────────────────
// Test C/D: 再開境界(平時/戦時)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_c_peacetime_resume_boundary() {
    let mut h = reserve_boundary_harness(34.9, false);
    set_blocked(&mut h, ARCADIA, true);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 0, "平時35日未満はblocked中なら再開しないはず");
    assert!(is_blocked(&h, ARCADIA));

    set_stockpile(&mut h, ARCADIA, 35.0);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 1, "平時35日ちょうどで再開するはず");
    assert!(!is_blocked(&h, ARCADIA));
}

#[test]
fn test_d_wartime_resume_boundary() {
    let mut h = reserve_boundary_harness(69.9, true);
    set_blocked(&mut h, ARCADIA, true);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 0, "戦時70日未満はblocked中なら再開しないはず");

    set_stockpile(&mut h, ARCADIA, 70.0);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 1, "戦時70日ちょうどで再開するはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test E: 中間帯の状態保持(真のヒステリシス)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_e_middle_band_state_depends_on_prior_blocked_status() {
    // 平時25日(20日以上35日未満の中間帯)。
    let reserve = 25.0;

    let mut was_unblocked = reserve_boundary_harness(reserve, false);
    set_blocked(&mut was_unblocked, ARCADIA, false);
    evaluate(&mut was_unblocked, ARCADIA);

    let mut was_blocked = reserve_boundary_harness(reserve, false);
    set_blocked(&mut was_blocked, ARCADIA, true);
    evaluate(&mut was_blocked, ARCADIA);

    assert_eq!(queue_len(&was_unblocked, ARCADIA), 1, "以前から許可中なら中間帯では継続許可");
    assert_eq!(queue_len(&was_blocked, ARCADIA), 0, "以前に停止していたなら中間帯では再開待ち");
}

// ─────────────────────────────────────────────────────────────────────────
// Test F: 需要0国家
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_f_zero_demand_country_does_not_panic_and_still_checks_the_bridge() {
    // Divisionなし(現在需要0)。備蓄0のままだと、新規候補自身の将来需要による
    // bridge判定で見送られるはず(需要0を理由にした誤判定ではない)。
    let mut h = single_country_harness(ARCADIA, 0.0, vec![]);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 0, "bridge不足のため見送るはず");

    // 十分な備蓄を与えれば、需要0自体は障害にならない。
    set_stockpile(&mut h, ARCADIA, 1000.0);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 1);
}

// ─────────────────────────────────────────────────────────────────────────
// Test G: 将来需要(実Definitionから導出)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_g_future_demand_scales_with_the_real_division_definition() {
    // required_manpowerを変えず、supply_usageを変えたharnessを2つ用意し、
    // bridge必要量がsupply_usageに比例して変わることを実際の募兵経路で確認する。
    fn harness_with_usage(supply_usage: f32) -> Harness {
        let mut h = single_country_harness(ARCADIA, 0.0, vec![]);
        h.military.definitions.get_mut(&STANDARD_INFANTRY_DEFINITION_ID).unwrap().supply_usage =
            supply_usage;
        h
    }

    let mut low_usage = harness_with_usage(0.5);
    set_stockpile(&mut low_usage, ARCADIA, 16.0); // 0.5/日 * 31日 = 15.5、余裕を持って16
    evaluate(&mut low_usage, ARCADIA);
    assert_eq!(queue_len(&low_usage, ARCADIA), 1, "低いsupply_usageならこの備蓄で足りるはず");

    let mut high_usage = harness_with_usage(2.0);
    set_stockpile(&mut high_usage, ARCADIA, 16.0); // 2.0/日 * 31日 = 62、不足するはず
    evaluate(&mut high_usage, ARCADIA);
    assert_eq!(queue_len(&high_usage, ARCADIA), 0, "高いsupply_usageなら同じ備蓄で不足するはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test H: bridge不足(reserve自体は十分でも見送る)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_h_ample_reserve_days_does_not_bypass_the_bridge_check() {
    // 現在需要0(Divisionなし)なのでreserve判定は常に通るが、月初(day=1)で
    // 新規候補自身の日次需要を次回生産まで支えられるだけの備蓄が無ければ見送る。
    let mut h = single_country_harness(ARCADIA, 5.0, vec![]); // 31日分に遠く足りない
    h.game_date = GameDate::new(1800, 1, 1);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 0, "reserveが十分に見えてもbridge不足で見送るはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test I: 月境界(28/30/31日の月、実カレンダー関数)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_i_calendar_boundary_matches_real_month_lengths() {
    // 1月(31日)・2月(28日)・4月(30日)それぞれの月初(day=1)で、必要bridge量が
    // 実際の月の長さちょうどになることを、募兵可否の閾値を跨がせて確認する。
    // day=1のとき必要bridge = demand * days_in_month(month)。
    for (month, length) in [(1u8, 31.0), (2, 28.0), (4, 30.0)] {
        let just_below = length - 0.5;
        let mut h = single_country_harness(ARCADIA, just_below, vec![]);
        h.game_date = GameDate::new(1800, month, 1);
        evaluate(&mut h, ARCADIA);
        assert_eq!(
            queue_len(&h, ARCADIA),
            0,
            "month={month}: 月の日数よりわずかに少ない備蓄では見送るはず(length={length})"
        );

        let mut h2 = single_country_harness(ARCADIA, length, vec![]);
        h2.game_date = GameDate::new(1800, month, 1);
        evaluate(&mut h2, ARCADIA);
        assert_eq!(
            queue_len(&h2, ARCADIA),
            1,
            "month={month}: 月の日数ちょうどの備蓄なら募兵できるはず(length={length})"
        );
    }
}

#[test]
fn test_i2_bridge_shrinks_near_month_end() {
    // 月末に近いほど、次回生産までの残り日数が減りbridge必要量も減る。
    let mut early = single_country_harness(ARCADIA, 10.0, vec![]);
    early.game_date = GameDate::new(1800, 1, 1);
    evaluate(&mut early, ARCADIA);
    assert_eq!(queue_len(&early, ARCADIA), 0);

    let mut late = single_country_harness(ARCADIA, 10.0, vec![]);
    late.game_date = GameDate::new(1800, 1, 25);
    evaluate(&mut late, ARCADIA);
    assert_eq!(queue_len(&late, ARCADIA), 1, "月末に近ければ同じ備蓄でも足りるはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test J/K/L: 配送健全性(既存Divisionの部分/完全不足、切断、首都占領)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_j_partial_delivery_shortage_blocks_new_recruitment() {
    // population=500_000(target=2)にして、既存Division1個だけでは`SkippedTargetMet`
    // に達せず、実際に補給ゲート(配送健全性)まで評価が進むようにする。
    let mut h = single_country_harness_with_population(
        ARCADIA,
        1000.0,
        vec![test_division(1, ARCADIA, 10_000)],
        500_000,
    );
    h.military.division_supply_fulfillment.insert(DivisionId(1), 0.5);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 0, "既存Divisionが部分充足なら見送るはず");
    assert!(is_blocked(&h, ARCADIA));
}

#[test]
fn test_k_fully_disconnected_division_blocks_new_recruitment_despite_ample_stockpile() {
    let mut h = single_country_harness_with_population(
        ARCADIA,
        100_000.0,
        vec![test_division(1, ARCADIA, 10_000)],
        500_000,
    );
    h.military.division_supply_fulfillment.insert(DivisionId(1), 0.0);
    evaluate(&mut h, ARCADIA);
    assert_eq!(
        queue_len(&h, ARCADIA),
        0,
        "国家備蓄が多くても既存Divisionの配送率0なら見送るはず"
    );
    assert!(is_blocked(&h, ARCADIA), "配送率0が原因でblockedになっているはず(target-met等ではない)");
}

#[test]
fn test_l_capital_occupation_style_zero_fulfillment_blocks_recruitment() {
    // 首都占領で生産は継続し備蓄が増え続けていても、配送が0なら見送る
    // (P26-008-PRE-001 §21の罠、備蓄健全性と配送健全性の分離)。
    let mut h = single_country_harness_with_population(
        ARCADIA,
        100_000.0,
        vec![test_division(1, ARCADIA, 10_000)],
        500_000,
    );
    h.military.division_supply_fulfillment.insert(DivisionId(1), 0.0);
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 0);
    assert!(is_blocked(&h, ARCADIA));
}

// ─────────────────────────────────────────────────────────────────────────
// Test M: 需要0 Divisionはminimum判定から除外
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_m_zero_demand_division_is_excluded_from_the_minimum_fulfillment() {
    let mut h = single_country_harness(ARCADIA, 1000.0, vec![test_division(1, ARCADIA, 0)]); // manpower=0
    h.military.division_supply_fulfillment.insert(DivisionId(1), 0.0); // 需要0なので無視されるはず
    evaluate(&mut h, ARCADIA);
    assert_eq!(
        queue_len(&h, ARCADIA),
        1,
        "需要0(manpower=0)のDivisionの補給率はminimum判定に影響しないはず"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test N: 進行中募兵はキャンセルされない
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_n_existing_queue_entry_is_never_cancelled_by_the_supply_gate() {
    let mut h = single_country_harness(ARCADIA, 0.0, vec![]);
    h.countries.get_mut(ARCADIA).unwrap().recruitment_queue.push(
        strategy_game::military::recruitment::RecruitmentQueueItem {
            division_id: STANDARD_INFANTRY_DEFINITION_ID,
            target_state: StateId(0),
            days_remaining: 10,
            total_days: 30,
        },
    );
    evaluate(&mut h, ARCADIA);
    assert_eq!(
        queue_len(&h, ARCADIA),
        1,
        "既存のキュー項目は補給不足でもキャンセルされないはず(既存のqueue-not-emptyガードで新規だけ止まる)"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test O: Equipment不足との分離
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_o_equipment_shortage_is_a_separate_concern_from_the_supply_gate() {
    // MilitarySuppliesは十分・MilitaryEquipmentは不足 -> queueには積まれる
    // (P25-003: request_recruitment自体はEquipmentを消費しない、消費は完成時)。
    let mut h = single_country_harness(ARCADIA, 1000.0, vec![]);
    // MilitaryEquipmentは既定0のまま(不足)。
    evaluate(&mut h, ARCADIA);
    assert_eq!(
        queue_len(&h, ARCADIA),
        1,
        "MilitarySuppliesが十分ならEquipment不足はqueue投入自体をブロックしないはず"
    );
    assert!(!is_blocked(&h, ARCADIA), "この場合、補給ゲート自体はblockedにならないはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test P: 既存条件の維持
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_p_existing_conditions_are_preserved() {
    // queue non-empty
    let mut h1 = single_country_harness(ARCADIA, 100_000.0, vec![]);
    h1.countries.get_mut(ARCADIA).unwrap().recruitment_queue.push(
        strategy_game::military::recruitment::RecruitmentQueueItem {
            division_id: STANDARD_INFANTRY_DEFINITION_ID,
            target_state: StateId(0),
            days_remaining: 5,
            total_days: 30,
        },
    );
    evaluate(&mut h1, ARCADIA);
    assert_eq!(queue_len(&h1, ARCADIA), 1, "queue非空は既存どおり新規投入しないはず");

    // target met
    let mut h2 = single_country_harness(ARCADIA, 100_000.0, vec![test_division(1, ARCADIA, 10_000)]);
    evaluate(&mut h2, ARCADIA); // population=200,000 -> cap=20,000 -> target=1、既に1個保有
    assert_eq!(queue_len(&h2, ARCADIA), 0, "target達成済みは既存どおり新規投入しないはず");

    // manpower不足
    let mut h3 = single_country_harness(ARCADIA, 100_000.0, vec![]);
    h3.countries.get_mut(ARCADIA).unwrap().available_manpower = 100;
    evaluate(&mut h3, ARCADIA);
    assert_eq!(queue_len(&h3, ARCADIA), 0, "manpower不足は既存どおり新規投入しないはず");

    // 募兵州なし
    let mut h4 = single_country_harness(ARCADIA, 100_000.0, vec![]);
    h4.states = StateRegistry::build(vec![test_state(0, 1, 200_000)]); // 他国所有
    evaluate(&mut h4, ARCADIA);
    assert_eq!(queue_len(&h4, ARCADIA), 0, "募兵州なしは既存どおり新規投入しないはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test Q/R: 平時⇔戦時の遷移
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_q_peacetime_to_wartime_transition_applies_the_new_threshold_next_evaluation() {
    // 30日分は平時では許可(停止しきい値20以上)だが、戦時では停止(停止しきい値40未満)。
    // `reserve_boundary_harness`と同じく月末寄り(1月30日)にしてbridgeを小さく保ち、
    // reserve日数のしきい値だけが遷移の可否を決めるようにする。
    let mut h = reserve_boundary_harness(30.0, false);
    evaluate(&mut h, ARCADIA);
    assert!(!is_blocked(&h, ARCADIA), "平時ではblockedにならないはず");

    add_war(&mut h, ARCADIA, ELFIN);
    h.countries.get_mut(ARCADIA).unwrap().recruitment_queue.clear();
    evaluate(&mut h, ARCADIA);
    assert!(is_blocked(&h, ARCADIA), "戦時開始後の次回評価ではblockedになるはず");
}

#[test]
fn test_r_wartime_to_peacetime_transition_only_resumes_once_the_peacetime_threshold_is_met() {
    let mut h = reserve_boundary_harness(45.0, true);
    set_blocked(&mut h, ARCADIA, true);
    evaluate(&mut h, ARCADIA);
    assert!(is_blocked(&h, ARCADIA), "45日は戦時のままなら再開しない(<70)");

    // 終戦、備蓄はそのまま(45日相当)。平時35日基準なら再開できるはず。
    h.war.wars.clear();
    h.countries.get_mut(ARCADIA).unwrap().recruitment_queue.clear();
    evaluate(&mut h, ARCADIA);
    assert!(!is_blocked(&h, ARCADIA), "終戦後は平時resume(35日)基準で再開するはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test S: New Game(初期31.0を安全判定に使わない)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_s_new_game_bridge_initial_31_is_not_treated_as_a_strategic_safety_margin() {
    // 初期ブリッジ31.0そのままでは、月半ば(day=15)ならbridge(1.0*16=16)を満たすが、
    // 月初(day=1)ならbridge(1.0*31=31)ぎりぎりで、既存Divisionが1個あると
    // 現在需要込みのbridgeを満たせず見送られる、という実際の需要・カレンダー計算の
    // 結果であることを確認する(31.0という値自体を特別扱いしていない)。
    let mut h = single_country_harness_with_population(
        ARCADIA,
        31.0,
        vec![test_division(1, ARCADIA, 10_000)],
        500_000,
    );
    h.game_date = GameDate::new(1800, 1, 1);
    evaluate(&mut h, ARCADIA);
    assert_eq!(
        queue_len(&h, ARCADIA),
        0,
        "既存Division需要込みでは初期31.0だけでは月初のbridgeを満たせないはず"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test W: 国家間分離
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_w_one_countrys_shortage_does_not_affect_another_countrys_gate() {
    let mut military = MilitaryRegistry::default();
    military.definitions.insert(STANDARD_INFANTRY_DEFINITION_ID, infantry_def(10_000));
    let mut h = Harness {
        countries: CountryRegistry {
            countries: vec![
                test_country(ARCADIA, 1_000_000, 0.0, 0),     // 不足
                test_country(ELFIN, 1_000_000, 100_000.0, 0), // 潤沢
            ],
        },
        military,
        states: StateRegistry::build(vec![
            test_state(0, 0, 200_000),
            test_state(1, 1, 200_000),
        ]),
        war: WarRegistry::default(),
        country_ai: CountryAiRegistry::default(),
        game_date: GameDate::new(1800, 1, 1),
    };
    evaluate(&mut h, ARCADIA);
    assert_eq!(queue_len(&h, ARCADIA), 0, "Arcadiaは不足で見送られるはず");
    assert_eq!(queue_len(&h, ELFIN), 1, "Elfinは無関係に募兵できるはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test X: 挿入順序非依存
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_x_result_does_not_depend_on_country_or_division_insertion_order() {
    fn run(reverse: bool) -> (usize, usize) {
        let mut military = MilitaryRegistry::default();
        military.definitions.insert(STANDARD_INFANTRY_DEFINITION_ID, infantry_def(10_000));
        let mut countries = vec![
            test_country(ARCADIA, 1_000_000, 1000.0, 0),
            test_country(ELFIN, 1_000_000, 0.0, 0),
        ];
        let mut states = vec![test_state(0, 0, 200_000), test_state(1, 1, 200_000)];
        if reverse {
            countries.reverse();
            states.reverse();
        }
        let mut h = Harness {
            countries: CountryRegistry { countries },
            military,
            states: StateRegistry::build(states),
            war: WarRegistry::default(),
            country_ai: CountryAiRegistry::default(),
            game_date: GameDate::new(1800, 1, 1),
        };
        evaluate(&mut h, ARCADIA);
        (queue_len(&h, ARCADIA), queue_len(&h, ELFIN))
    }

    assert_eq!(run(false), run(true));
}

// ─────────────────────────────────────────────────────────────────────────
// Test T/U/V: Save V1(現行/旧/2回連続)
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
        "strategy_game_p26_008_a_e2e_{label}_{}_{nanos}_{n}",
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

fn tick_month(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

fn advance_one_month(app: &mut App, month_index: u8) {
    for day in 1..=28u8 {
        tick_day(app, 1800, month_index, day);
    }
    tick_month(app, 1800, month_index.wrapping_add(1));
}

fn blocked_of(app: &App, country_id: CountryId) -> bool {
    app.world()
        .resource::<CountryAiRegistry>()
        .ai_states
        .get(&country_id)
        .map(|s| s.recruitment_supply_blocked)
        .unwrap_or(false)
}

fn set_app_stockpile(app: &mut App, country_id: CountryId, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(country_id)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitarySupplies, value);
}

#[test]
fn test_t_current_save_round_trips_the_blocked_flag_both_ways() {
    for &blocked in &[false, true] {
        let dir = TempTestDir::new(&format!("t_{blocked}"));
        let mut app = setup_app(Some(DWARF), dir.save_file_config());
        app.world_mut()
            .resource_mut::<CountryAiRegistry>()
            .get_or_create_mut(ARCADIA)
            .recruitment_supply_blocked = blocked;

        send_save_request(&mut app);
        send_load_request(&mut app);

        assert_eq!(
            blocked_of(&app, ARCADIA),
            blocked,
            "recruitment_supply_blocked={blocked}が往復で保たれるはず"
        );
    }
}

#[test]
fn test_u_old_save_without_the_new_field_loads_and_settles_from_actual_state() {
    let dir = TempTestDir::new("u");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());
    send_save_request(&mut app);

    // 旧Save形式を模擬: "recruitment_supply_blocked: true/false,"の行を機械的に除去する。
    let path = dir.final_path();
    let contents = std::fs::read_to_string(&path).unwrap();
    let stripped = contents
        .replace("recruitment_supply_blocked: true,", "")
        .replace("recruitment_supply_blocked: false,", "");
    std::fs::write(&path, stripped).unwrap();

    send_load_request(&mut app);
    assert!(
        !blocked_of(&app, ARCADIA),
        "フィールド欠落時はfalse(未停止)から復元されるはず"
    );

    // その後は次回の正規月次募兵評価で実際の状態から判定される(臨時判定はしない)。
    advance_one_month(&mut app, 1);
    // パニックしないこと・queueや評価が正常に続くことの確認で十分
    // (具体的なblocked値は実データの経済状況に依存するため固定しない)。
    let _ = blocked_of(&app, ARCADIA);
}

#[test]
fn test_v_double_save_load_does_not_change_the_blocked_state() {
    let dir = TempTestDir::new("v");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());
    set_app_stockpile(&mut app, ARCADIA, 0.0);
    app.world_mut()
        .resource_mut::<CountryAiRegistry>()
        .get_or_create_mut(ARCADIA)
        .recruitment_supply_blocked = true;

    send_save_request(&mut app);
    send_load_request(&mut app);
    let after_first = blocked_of(&app, ARCADIA);

    send_save_request(&mut app);
    send_load_request(&mut app);
    let after_second = blocked_of(&app, ARCADIA);

    assert_eq!(after_first, after_second, "2回連続save/loadで状態が変化しないはず");
    assert!(after_first, "true=trueのまま往復するはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test Y: 実App月次System経由
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_y_real_monthly_system_suppresses_and_resumes_new_recruitment_requests() {
    let dir = TempTestDir::new("y");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());

    // Arcadiaを需要0(Division除去)+備蓄0にして、月1の実評価で見送られることを
    // 実際の`handle_monthly_ai_recruitment` System経由で確認する。
    app.world_mut().resource_mut::<MilitaryRegistry>().divisions.retain(|_, d| d.owner != ARCADIA);
    set_app_stockpile(&mut app, ARCADIA, 0.0);

    advance_one_month(&mut app, 1);
    assert_eq!(
        app.world().resource::<CountryRegistry>().get(ARCADIA).unwrap().recruitment_queue.len(),
        0,
        "実App経由でも備蓄不足時は新規募兵要求が抑制されるはず"
    );
    assert!(blocked_of(&app, ARCADIA));

    // 備蓄を回復させ、平時resume(35日)を上回らせる。
    set_app_stockpile(&mut app, ARCADIA, 1000.0);
    advance_one_month(&mut app, 2);
    assert!(!blocked_of(&app, ARCADIA), "備蓄回復後は実App経由で再開するはず");
}

// ─────────────────────────────────────────────────────────────────────────
// §21-22: 90日・36か月の6か国実App検証(PlayerCountry(None)、診断用の無限資金
// フィクスチャなし)
// ─────────────────────────────────────────────────────────────────────────

const DAYS_IN_MONTH_REAL: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
const ALL_SIX: [CountryId; 6] =
    [ARCADIA, ELFIN, DWARF, CountryId(3), CountryId(4), CountryId(5)];

fn set_game_date(app: &mut App, year: i32, month: u8, day: u8) {
    let mut gd = app.world_mut().resource_mut::<GameDate>();
    gd.year = year;
    gd.month = month;
    gd.day = day;
}

/// 1か月分、実際の月の日数(28/30/31、うるう年なし)だけ`DayChangedMessage`を送り、
/// `GameDate`resourceもその都度実際の日付へ同期させたうえで、月末に`GameDate`を
/// 翌月1日へ進めてから`MonthChangedMessage`を送る。
///
/// この実App E2Eハーネスは`advance_game_date`(`src/app/time.rs`、`Res<Time>`の
/// 経過に応じて`GameDate`と両メッセージを自動生成する本番System)を経由せず、
/// メッセージを直接注入する既存の確立されたテスト方式(`tick_day`/`tick_month`)を
/// 踏襲する。そのため`GameDate`resource自体は放置すると動かない――本関数のように
/// 明示的に同期させないと、募兵の補給ゲート(`Res<GameDate>`のmonth/dayを直接読む、
/// spec §10)が常に初期日付のまま評価されてしまう(単体テストでは`GameDate::new`で
/// 個別に与えているため問題にならないが、複数か月にわたる実App検証では月境界を
/// またぐ実際のカレンダー計算を検証する必要がある、spec §10・§21-22)。
/// 簡略暦(`day 2..=28`固定、P26-008-PRE-002が検出した既知の罠)は使わず、月の
/// 実際の日数を1日目から使い切る。
fn advance_one_calendar_month(app: &mut App, year: i32, month: u8) -> (i32, u8) {
    let month_length = DAYS_IN_MONTH_REAL[(month - 1) as usize];
    for day in 1..=month_length {
        set_game_date(app, year, month, day);
        tick_day(app, year, month, day);
    }
    let (next_year, next_month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    set_game_date(app, next_year, next_month, 1);
    tick_month(app, next_year, next_month);
    (next_year, next_month)
}

fn stockpile_of(app: &App, c: CountryId, resource: ResourceType) -> f64 {
    app.world().resource::<CountryRegistry>().get(c).unwrap().stockpile.get(resource)
}

fn living_division_count_of(app: &App, c: CountryId) -> usize {
    app.world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == c && d.manpower > 0 && d.status != DivisionStatus::Destroyed)
        .count()
}

fn recruitment_queue_len_of(app: &App, c: CountryId) -> usize {
    app.world().resource::<CountryRegistry>().get(c).unwrap().recruitment_queue.len()
}

/// 月次スナップショット1件(spec §21-22の報告項目)。
#[derive(Debug, Clone)]
struct MonthlySnapshot {
    month_index: u32,
    division_count: usize,
    queue_len: usize,
    military_supplies: f64,
    military_equipment: f64,
    blocked: bool,
}

/// 6か国全てについて、指定した月数だけ実際のカレンダーで実Appを進め、各国・各月末の
/// スナップショットを記録する(spec §21-22: 診断用の無限資金フィクスチャなし、
/// `PlayerCountry(None)`で全6か国をAI候補にする)。
fn run_multi_month_real_app_verification(
    label: &str,
    months: u8,
) -> (App, std::collections::HashMap<CountryId, Vec<MonthlySnapshot>>) {
    let dir = TempTestDir::new(label);
    let mut app = setup_app(None, dir.save_file_config());
    set_game_date(&mut app, 1800, 1, 1);

    let mut histories: std::collections::HashMap<CountryId, Vec<MonthlySnapshot>> =
        ALL_SIX.iter().map(|&c| (c, Vec::new())).collect();

    let mut year = 1800i32;
    let mut month = 1u8;
    for month_index in 1..=(months as u32) {
        let (next_year, next_month) = advance_one_calendar_month(&mut app, year, month);
        year = next_year;
        month = next_month;

        for &c in &ALL_SIX {
            histories.get_mut(&c).unwrap().push(MonthlySnapshot {
                month_index,
                division_count: living_division_count_of(&app, c),
                queue_len: recruitment_queue_len_of(&app, c),
                military_supplies: stockpile_of(&app, c, ResourceType::MilitarySupplies),
                military_equipment: stockpile_of(&app, c, ResourceType::MilitaryEquipment),
                blocked: blocked_of(&app, c),
            });
        }
    }

    (app, histories)
}

/// あるDrop dir(TempTestDir)を保持しない`run_multi_month_real_app_verification`の
/// 戻り値`App`は、内部でSaveFileConfigが指す一時ディレクトリを直接指すのみで、
/// この関数自体は一時ファイルを作らない(save/loadを行わないテストのため)。
///
/// 各国の月次履歴から、「MilitarySupplies補給ゲート(P26-008-A)による見送り」と
/// 「既存のEquipment不足によるqueue滞留(spec §17、既知のMedium finding、本タスクの
/// 対象外)」を区別してeprintln!で報告する(spec §21-22)。両者は独立した信号から
/// 判定する: 前者は`recruitment_supply_blocked`(本タスクの新規state)、後者は
/// 「queueが複数か月連続で同じ長さのまま停滞し、かつMilitaryEquipment備蓄が乏しい」
/// という既存挙動(P23-009等が既に検証済みの現象)から推測する。
fn report_monthly_histories(
    title: &str,
    histories: &std::collections::HashMap<CountryId, Vec<MonthlySnapshot>>,
) {
    eprintln!("[{title}]");
    for &c in &ALL_SIX {
        let history = &histories[&c];
        let Some(last) = history.last() else { continue };
        let ever_gate_blocked = history.iter().any(|s| s.blocked);
        let first_gate_blocked_month =
            history.iter().find(|s| s.blocked).map(|s| s.month_index);
        let ever_queue_stalled_with_low_equipment = history
            .windows(2)
            .any(|w| w[0].queue_len > 0 && w[1].queue_len == w[0].queue_len && w[1].military_equipment < 10.0);
        eprintln!(
            "  {c:?}: final_divisions={} final_queue_len={} final_supplies={:.3} \
             final_equipment={:.3} ever_supply_gate_blocked={} first_gate_blocked_month={:?} \
             ever_queue_stalled_with_low_equipment={} division_history={:?}",
            last.division_count,
            last.queue_len,
            last.military_supplies,
            last.military_equipment,
            ever_gate_blocked,
            first_gate_blocked_month,
            ever_queue_stalled_with_low_equipment,
            history.iter().map(|s| s.division_count).collect::<Vec<_>>()
        );
    }
}

#[test]
fn test_90_day_six_country_recruitment_gate_real_app_verification() {
    // 90日 ≈ 3か月(spec §21: 実カレンダーで3か月分進める)。
    let (app, histories) = run_multi_month_real_app_verification("ninety_day_gate", 3);
    report_monthly_histories("P26-008-A 90-day report", &histories);

    // 個別の期待値は強制しない(spec §21: 全国家を同じ結果に均さない)。ここでは
    // 「パニックしない」「備蓄が負値にならない」「Division数が(このシナリオでは
    // 戦闘が発生しないため)減少しない」という、どの国のシナリオにも共通する
    // 一般的な健全性だけを検証する。
    for &c in &ALL_SIX {
        let history = &histories[&c];
        assert_eq!(history.len(), 3);
        for s in history {
            assert!(s.military_supplies >= 0.0, "{c:?}: MilitarySuppliesが負値になってはいけない");
        }
        for w in history.windows(2) {
            assert!(
                w[1].division_count >= w[0].division_count,
                "{c:?}: 開戦していないシナリオでDivision数が減少してはいけない"
            );
        }
    }
    let _ = app;
}

#[test]
fn test_36_month_six_country_recruitment_gate_real_app_verification() {
    let (app, histories) = run_multi_month_real_app_verification("thirtysix_month_gate", 36);
    report_monthly_histories("P26-008-A 36-month report", &histories);

    for &c in &ALL_SIX {
        let history = &histories[&c];
        assert_eq!(history.len(), 36);
        for s in history {
            assert!(s.military_supplies >= 0.0, "{c:?}: MilitarySuppliesが負値になってはいけない");
        }
        // 開戦していない国では、Division数は減少しない(自動解隊・消失の検出)。P29-004 で
        // 募兵の許可条件が変わり、36 か月の間に戦争が発生する世界の経過も変わるため、
        // 当該国が(いずれかの時点で)戦争に参加した場合は、戦闘による損耗を許容する。
        let participated = app
            .world()
            .resource::<strategy_game::war::data::WarRegistry>()
            .wars
            .values()
            .any(|w| w.is_participant(c));
        if !participated {
            for w in history.windows(2) {
                assert!(
                    w[1].division_count >= w[0].division_count,
                    "{c:?}: 開戦していないシナリオでDivision数が減少してはいけない"
                );
            }
        }
    }

    // spec §21で明示された既存の国別差異(人口成長率・補給網の厚さ等、いずれも
    // P26-008-Aの対象外の既存要因)により、36か月間Division数が全く増えない国が
    // 複数存在してもよい――目標Division数(`calculate_ai_target_divisions`、
    // 人口連動)自体が36か月では1のまま動かない国もあれば、目標が増えても
    // 本ゲートが一度も「補給不足」と判定しない国もある。それ自体は本タスクの
    // 失敗ではない(既存のEquipment不足・目標数到達等、無関係な既存要因が原因の
    // 場合を含む、spec §21-22)。
    //
    // 「6か国を均質化していない」ことの実証には、最終Division数そのものではなく
    // (今回の36か月・実データでは6か国とも目標が1のまま、または1で頭打ちになり
    // 得るため、最終値だけを見ると偶然一致し得る)、各国が本ゲートから受けた
    // 判定そのものの違い――`ever_supply_gate_blocked`(一度でも新規募兵をこの
    // ゲートで見送ったか)――を使う。全6か国が寸分違わず同じ判定を受けているなら、
    // 国ごとの実際の備蓄・需要状況を無視して均一なゲートを掛けている疑いがある。
    let gate_blocked_flags: Vec<bool> =
        ALL_SIX.iter().map(|&c| histories[&c].iter().any(|s| s.blocked)).collect();
    eprintln!("  ever_supply_gate_blocked per country: {gate_blocked_flags:?}");
    assert!(
        gate_blocked_flags.iter().collect::<std::collections::HashSet<_>>().len() > 1,
        "6か国全てが本ゲートについて全く同じ判定(全員blocked、または全員not-blocked)になっている\
         (国ごとの実際の備蓄・需要の違いが反映されていない可能性): {gate_blocked_flags:?}"
    );
    let _ = app;
}
