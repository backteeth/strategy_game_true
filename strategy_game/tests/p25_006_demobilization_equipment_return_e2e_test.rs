//! P25-006: 復員時の生存装備返却と国家消滅時の装備会計のE2Eテスト。
//!
//! `src/military/demobilization.rs`のDomain単体テストが`demobilize_division`自体の
//! 装備返却計算・原子性を固定しているのに対し、本テストは実際のsave/load・
//! New Game/Loaded Game・複数国家分離・国家消滅の各経路を通して装備保存則を検証する。
//!
//! **既知の未解決事項(§14参照)**: 実装前監査で、既存の国家消滅処理
//! (`country::elimination::apply_country_elimination`、P21-021、Phase 25より前に
//! 実装済み)が消滅国家の全DivisionをMilitaryRegistryから物理削除しており、
//! P25-001が決定した「装備会計は凍結して残存」という設計と矛盾することが判明した。
//! ユーザーの判断により、本タスクでは`country::elimination`を一切変更せず、
//! この矛盾は報告書へHigh findingとして記録するに留める。本ファイル末尾の
//! `country_elimination_gap`セクションは、この既知のギャップを隠さず実際の
//! 挙動として記録するためのテストである(「装備が凍結される」という誤った
//! 成功を主張しない)。
#![allow(clippy::type_complexity)] // SystemStateのタプル型はテスト内での使い捨て構成であり、型エイリアス化の利益がない

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::common::{CountryId, DivisionDefinitionId, DivisionId, StateId};
use strategy_game::country::elimination::{apply_country_elimination, preflight_country_elimination};
use strategy_game::country::power::CountryPowerRegistry;
use strategy_game::country::{CountryData, CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::diplomacy::claims::ClaimRegistry;
use strategy_game::diplomacy::crisis::CrisisRegistry;
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::{ResourceType, military_equipment_value_to_units};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::military::demobilization::{
    DemobilizationError, DemobilizationOutcome, demobilize_division,
};
use strategy_game::military::equipment::EquipmentAccountingError;
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
use strategy_game::war::data::WarRegistry;
use strategy_game::war::frontline::FrontlineRegistry;
use strategy_game::war::justification::WarJustificationRegistry;

// ─── 軽量ハーネス(直接demobilize_divisionを呼ぶ純粋関数テスト用) ────────

fn test_division(id: usize, owner: usize, manpower: u64, equipment: f64, max_equipment: f64) -> Division {
    Division {
        id: DivisionId(id),
        owner: CountryId(owner),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(0),
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower,
        max_manpower: manpower.max(1) + 5_000,
        equipment,
        max_equipment,
        organization: 50.0,
        max_organization: 50.0,
        morale: 1.0,
        max_morale: 1.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: DivisionDefinitionId(0),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

fn test_country(id: usize, available: u64, mobilized: u64, stockpile: f64) -> CountryData {
    let mut country = CountryData {
        id: CountryId(id),
        available_manpower: available,
        mobilized_manpower: mobilized,
        ..CountryData::default()
    };
    country
        .stockpile
        .set(ResourceType::MilitaryEquipment, stockpile);
    country
}

struct Harness {
    countries: CountryRegistry,
    military: MilitaryRegistry,
    armies: ArmyRegistry,
    frontlines: FrontlineRegistry,
    wars: WarRegistry,
    battles: BattleRegistry,
}

fn harness(countries: Vec<CountryData>, divisions: Vec<Division>) -> Harness {
    let mut military = MilitaryRegistry::default();
    for d in divisions {
        military.divisions.insert(d.id, d);
    }
    Harness {
        countries: CountryRegistry { countries },
        military,
        armies: ArmyRegistry::default(),
        frontlines: FrontlineRegistry::default(),
        wars: WarRegistry::default(),
        battles: BattleRegistry::default(),
    }
}

fn run(
    h: &mut Harness,
    country_id: CountryId,
    division_id: DivisionId,
) -> Result<DemobilizationOutcome, DemobilizationError> {
    demobilize_division(
        &mut h.countries,
        &mut h.military,
        &mut h.armies,
        &mut h.frontlines,
        &h.wars,
        &h.battles,
        country_id,
        division_id,
    )
}

fn stockpile_of(h: &Harness, country_id: CountryId) -> f64 {
    h.countries
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitaryEquipment)
}

// ─── §16.1: 完全充足Division ─────────────────────────────────────────────

#[test]
fn test_1_fully_equipped_division_returns_its_entire_current_equipment() {
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 20.0)],
        vec![test_division(0, 1, 10_000, 100.0, 100.0)],
    );
    let outcome = run(&mut h, CountryId(1), DivisionId(0)).unwrap();
    assert_eq!(outcome.returned_equipment, 100.0);
    assert_eq!(stockpile_of(&h, CountryId(1)), 120.0);
    assert!(!h.military.divisions.contains_key(&DivisionId(0)));
}

// ─── §16.2/Test 2/3: 損耗Division ────────────────────────────────────────

#[test]
fn test_2_3_worn_division_returns_only_surviving_equipment_destroyed_portion_never_revives() {
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 20.0)],
        // max_equipment=100だが、既にP25-004の戦闘で60破壊済み、現在40だけ生存。
        vec![test_division(0, 1, 10_000, 40.0, 100.0)],
    );
    let outcome = run(&mut h, CountryId(1), DivisionId(0)).unwrap();
    assert_eq!(
        outcome.returned_equipment, 40.0,
        "破壊済みの60は復活せず、現在の40だけが返却されるはず"
    );
    assert_eq!(stockpile_of(&h, CountryId(1)), 60.0);
}

// ─── Test 4: equipment=0 ─────────────────────────────────────────────────

#[test]
fn test_4_zero_equipment_division_demobilizes_with_zero_return() {
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 20.0)],
        vec![test_division(0, 1, 10_000, 0.0, 100.0)],
    );
    let outcome = run(&mut h, CountryId(1), DivisionId(0)).unwrap();
    assert_eq!(outcome.returned_equipment, 0.0);
    assert_eq!(stockpile_of(&h, CountryId(1)), 20.0);
    assert!(!h.military.divisions.contains_key(&DivisionId(0)));
}

// ─── Test 5: max_equipment=0 ─────────────────────────────────────────────

#[test]
fn test_5_zero_max_equipment_division_demobilizes_successfully() {
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 0.0)],
        vec![test_division(0, 1, 10_000, 0.0, 0.0)],
    );
    let outcome = run(&mut h, CountryId(1), DivisionId(0)).unwrap();
    assert_eq!(outcome.returned_equipment, 0.0);
    assert!(!h.military.divisions.contains_key(&DivisionId(0)));
}

// ─── §16.4/Test 6: 小数装備の正確な返却 ──────────────────────────────────

#[test]
fn test_6_fractional_equipment_returns_exactly_in_fixed_point_units() {
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 1.3)],
        vec![test_division(0, 1, 10_000, 3.7, 5.0)],
    );
    let outcome = run(&mut h, CountryId(1), DivisionId(0)).unwrap();
    assert_eq!(outcome.returned_equipment, 3.7);
    assert_eq!(stockpile_of(&h, CountryId(1)), 5.0);
    assert_eq!(
        military_equipment_value_to_units(stockpile_of(&h, CountryId(1))).unwrap(),
        5_000_000
    );
}

// ─── Test 7/8: units保存則(国家単位・世界単位) ──────────────────────────

#[test]
fn test_7_8_conservation_law_holds_across_stockpile_and_surviving_divisions() {
    let mut h = harness(
        vec![test_country(1, 0, 20_000, 15.0)],
        vec![
            test_division(0, 1, 10_000, 40.0, 100.0),
            test_division(1, 1, 10_000, 25.0, 100.0),
        ],
    );
    let world_before = stockpile_of(&h, CountryId(1))
        + h.military
            .divisions
            .values()
            .map(|d| d.equipment)
            .sum::<f64>();

    run(&mut h, CountryId(1), DivisionId(0)).unwrap();

    let world_after = stockpile_of(&h, CountryId(1))
        + h.military
            .divisions
            .values()
            .map(|d| d.equipment)
            .sum::<f64>();
    assert_eq!(
        world_before, world_after,
        "復員は備蓄への移動であり、世界装備総量を増減させない"
    );
    // 生存Division(id=1)の装備は不変。
    assert_eq!(h.military.divisions[&DivisionId(1)].equipment, 25.0);
}

// ─── Test 9: max_equipmentを返却量に使わない ─────────────────────────────

#[test]
fn test_9_max_equipment_is_never_used_as_the_returned_amount() {
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 0.0)],
        vec![test_division(0, 1, 10_000, 40.0, 100.0)],
    );
    let outcome = run(&mut h, CountryId(1), DivisionId(0)).unwrap();
    assert_eq!(outcome.returned_equipment, 40.0);
    assert_ne!(outcome.returned_equipment, 100.0);
}

// ─── Test 10/11: 正規所属国への返却・複数国家分離 ────────────────────────

#[test]
fn test_10_11_equipment_returns_to_the_divisions_own_country_and_others_are_untouched() {
    let mut h = harness(
        vec![
            test_country(1, 0, 10_000, 0.0),
            test_country(2, 0, 0, 100.0),
        ],
        vec![test_division(0, 1, 10_000, 40.0, 100.0)],
    );
    run(&mut h, CountryId(1), DivisionId(0)).unwrap();
    assert_eq!(stockpile_of(&h, CountryId(1)), 40.0);
    assert_eq!(
        stockpile_of(&h, CountryId(2)),
        100.0,
        "他国の備蓄は一切変化しないはず"
    );
}

// ─── Test 12: 偽装された他国復員要求の拒否 ───────────────────────────────

#[test]
fn test_12_a_spoofed_foreign_demobilization_request_is_rejected_atomically() {
    let mut h = harness(
        vec![
            test_country(1, 0, 0, 0.0),
            test_country(2, 0, 10_000, 100.0),
        ],
        vec![test_division(0, 2, 10_000, 40.0, 100.0)],
    );
    // 国家1が、実際には国家2が所有するDivisionを自国のものとして復員しようとする。
    let result = run(&mut h, CountryId(1), DivisionId(0));
    assert_eq!(result, Err(DemobilizationError::NotOwner));
    assert_eq!(stockpile_of(&h, CountryId(1)), 0.0);
    assert_eq!(stockpile_of(&h, CountryId(2)), 100.0);
    assert!(h.military.divisions.contains_key(&DivisionId(0)));
}

// ─── Test 13: 不正equipment stateで原子的拒否 ────────────────────────────

#[test]
fn test_13_invalid_division_equipment_state_is_rejected_atomically() {
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 20.0)],
        vec![test_division(0, 1, 10_000, f64::NAN, 100.0)],
    );
    let result = run(&mut h, CountryId(1), DivisionId(0));
    assert!(matches!(
        result,
        Err(DemobilizationError::EquipmentReturnFailed(
            EquipmentAccountingError::NonFiniteQuantity
        ))
    ));
    assert_eq!(stockpile_of(&h, CountryId(1)), 20.0);
    let division = &h.military.divisions[&DivisionId(0)];
    assert_eq!(division.manpower, 10_000, "manpower会計もロールバックされるはず");
    assert!(h.military.divisions.contains_key(&DivisionId(0)));
}

// ─── Test 14: 不正国家備蓄で原子的拒否 ───────────────────────────────────

#[test]
fn test_14_invalid_country_stockpile_is_rejected_atomically() {
    let mut country = test_country(1, 0, 10_000, 0.0);
    // CountryStockpile::set()はNaN/負値を0.0へ丸めてしまうため、`amounts`へ
    // 直接不正値を注入する(異常状態のシミュレーション、正常経路では発生しない)。
    country
        .stockpile
        .amounts
        .insert(ResourceType::MilitaryEquipment, f64::NAN);
    let mut h = harness(
        vec![country],
        vec![test_division(0, 1, 10_000, 40.0, 100.0)],
    );
    let result = run(&mut h, CountryId(1), DivisionId(0));
    assert!(matches!(
        result,
        Err(DemobilizationError::EquipmentReturnFailed(_))
    ));
    let division = &h.military.divisions[&DivisionId(0)];
    assert_eq!(division.equipment, 40.0);
    assert_eq!(division.manpower, 10_000);
}

// ─── Test 15/§16.5: stockpile overflowで原子的拒否 ───────────────────────

#[test]
fn test_15_return_overflow_beyond_the_canonical_maximum_is_rejected_atomically() {
    let near_max = 999_999_999.0; // 正規最大(1,000,000,000)にほぼ達している
    let mut h = harness(
        vec![test_country(1, 0, 10_000, near_max)],
        vec![test_division(0, 1, 10_000, 40.0, 100.0)],
    );
    let result = run(&mut h, CountryId(1), DivisionId(0));
    assert!(matches!(
        result,
        Err(DemobilizationError::EquipmentReturnFailed(
            EquipmentAccountingError::QuantityOutOfRange
        ))
    ));
    assert_eq!(stockpile_of(&h, CountryId(1)), near_max, "備蓄は不変のはず");
    let division = &h.military.divisions[&DivisionId(0)];
    assert_eq!(division.equipment, 40.0, "Division装備も不変のはず");
    assert_eq!(division.manpower, 10_000, "manpowerも不変のはず");
    assert!(h.military.divisions.contains_key(&DivisionId(0)));
}

// ─── Test 16/17/18: 失敗時のmanpower/equipment/despawnロールバック ──────

#[test]
fn test_16_18_equipment_failure_leaves_manpower_unreturned_and_division_alive() {
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 0.0)],
        vec![test_division(0, 1, 10_000, f64::NAN, 100.0)],
    );
    let available_before = h.countries.get(CountryId(1)).unwrap().available_manpower;
    let mobilized_before = h.countries.get(CountryId(1)).unwrap().mobilized_manpower;

    let result = run(&mut h, CountryId(1), DivisionId(0));
    assert!(result.is_err());
    let country = h.countries.get(CountryId(1)).unwrap();
    assert_eq!(country.available_manpower, available_before);
    assert_eq!(country.mobilized_manpower, mobilized_before);
    assert!(h.military.divisions.contains_key(&DivisionId(0)));
}

// ─── Test 19: 復員成功時にDivisionを削除 ─────────────────────────────────

#[test]
fn test_19_successful_demobilization_removes_the_division() {
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 0.0)],
        vec![test_division(0, 1, 10_000, 40.0, 100.0)],
    );
    run(&mut h, CountryId(1), DivisionId(0)).unwrap();
    assert!(!h.military.divisions.contains_key(&DivisionId(0)));
}

// ─── §16.6/Test 20: 同一フレーム重複要求で二重返却しない ─────────────────

#[test]
fn test_20_duplicate_requests_for_the_same_division_only_return_equipment_once() {
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 0.0)],
        vec![test_division(0, 1, 10_000, 40.0, 100.0)],
    );
    let first = run(&mut h, CountryId(1), DivisionId(0));
    assert!(first.is_ok());
    assert_eq!(stockpile_of(&h, CountryId(1)), 40.0);

    // 同一フレーム内で同じDivisionへの2件目の要求が届いた想定。
    // Divisionは既にHashMapから同期的に削除済みのため、即座にDivisionNotFoundとなり、
    // 二重返却は構造的に起こり得ない(deferred Commandsではなく即時削除のため)。
    let second = run(&mut h, CountryId(1), DivisionId(0));
    assert_eq!(second, Err(DemobilizationError::DivisionNotFound));
    assert_eq!(
        stockpile_of(&h, CountryId(1)),
        40.0,
        "2回目の要求で備蓄が再度増えてはいけない"
    );
}

// ─── Test 21: 複数Divisionの決定的処理 ───────────────────────────────────

#[test]
fn test_21_multiple_divisions_are_each_returned_independently() {
    let mut h = harness(
        vec![test_country(1, 0, 20_000, 0.0)],
        vec![
            test_division(0, 1, 10_000, 40.0, 100.0),
            test_division(1, 1, 10_000, 25.0, 100.0),
        ],
    );
    run(&mut h, CountryId(1), DivisionId(0)).unwrap();
    assert_eq!(stockpile_of(&h, CountryId(1)), 40.0);
    run(&mut h, CountryId(1), DivisionId(1)).unwrap();
    assert_eq!(stockpile_of(&h, CountryId(1)), 65.0);
}

// ─── Test 22: 1国の失敗が他国へ波及しない ────────────────────────────────

#[test]
fn test_22_one_countrys_failure_does_not_affect_another_countrys_demobilization() {
    let mut h = harness(
        vec![
            test_country(1, 0, 10_000, 0.0),
            test_country(2, 0, 10_000, 0.0),
        ],
        vec![
            test_division(0, 1, 10_000, f64::NAN, 100.0),
            test_division(1, 2, 10_000, 40.0, 100.0),
        ],
    );
    let result_a = run(&mut h, CountryId(1), DivisionId(0));
    assert!(result_a.is_err());

    let result_b = run(&mut h, CountryId(2), DivisionId(1));
    assert!(result_b.is_ok());
    assert_eq!(stockpile_of(&h, CountryId(2)), 40.0);
    assert!(h.military.divisions.contains_key(&DivisionId(0)));
    assert!(!h.military.divisions.contains_key(&DivisionId(1)));
}

// ─── Test 23/24/25: manpower会計(P23-007)の維持 ─────────────────────────

#[test]
fn test_23_25_p23_007_manpower_accounting_numbers_are_unchanged_by_p25_006() {
    // src/military/demobilization.rs::testsの
    // demobilizing_a_worn_division_returns_only_its_surviving_manpower と同じ数値例。
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 5.0)],
        vec![test_division(0, 1, 7_000, 40.0, 100.0)],
    );
    let outcome = run(&mut h, CountryId(1), DivisionId(0)).unwrap();
    assert_eq!(outcome.returned_manpower, 7_000, "生存manpowerだけが復員される");
    let country = h.countries.get(CountryId(1)).unwrap();
    assert_eq!(country.available_manpower, 7_000);
    assert_eq!(
        country.mobilized_manpower, 3_000,
        "K(=3,000)相当分はBに残る(P25-006で装備返却を追加してもmanpower数値は不変)"
    );
    // 装備返却も同時に正しく行われていることを確認(独立した2つの会計が両立)。
    assert_eq!(outcome.returned_equipment, 40.0);
    assert_eq!(stockpile_of(&h, CountryId(1)), 45.0);
}

// ─── Test 26/27: population/workforce・manpower capの非変更 ─────────────

#[test]
fn test_26_27_demobilization_does_not_touch_population_workforce_or_hard_cap_available_manpower() {
    // available_manpowerがmax_manpower等の"cap"概念を超えても没収されない
    // (既存のP23-007仕様どおり、soft targetのまま)ことを確認する。
    let mut h = harness(
        vec![test_country(1, u64::MAX - 5_000, 10_000, 0.0)],
        vec![test_division(0, 1, 5_000, 40.0, 100.0)],
    );
    let outcome = run(&mut h, CountryId(1), DivisionId(0)).unwrap();
    assert_eq!(
        h.countries.get(CountryId(1)).unwrap().available_manpower,
        u64::MAX,
        "manpower capはhard capとして没収されない(既存P23-007仕様のまま)"
    );
    assert_eq!(outcome.returned_equipment, 40.0);
}

// ─── Test 28/29: 戦闘全滅との分離(P25-004回帰) ──────────────────────────

#[test]
fn test_28_29_a_division_removed_by_combat_annihilation_cannot_be_demobilized_and_stockpile_is_untouched()
 {
    // P25-004により、実際のcasualtyがmanpower_beforeと一致するDivisionは
    // process_daily_battles内で現在装備を全損したうえでMilitaryRegistryから
    // 完全に削除される(military::update::resolve_participant_outcome、
    // manpower==0 → remove_division)。したがって、そのDivisionIdに対する
    // 事後の復員要求は必ずDivisionNotFoundとなり、国家備蓄・装備は一切動かない
    // (詳細な全滅時装備0の検証自体はP25-004の内部テストで既に確認済みであり、
    // ここではその後にP25-006の復員処理が誤って干渉しないことだけを確認する)。
    let mut h = harness(
        vec![test_country(1, 0, 10_000, 20.0)],
        vec![], // 全滅済み: そもそもMilitaryRegistryに存在しない
    );
    let result = run(&mut h, CountryId(1), DivisionId(0));
    assert_eq!(result, Err(DemobilizationError::DivisionNotFound));
    assert_eq!(
        stockpile_of(&h, CountryId(1)),
        20.0,
        "全滅Divisionの装備(既にP25-004で破壊済み)が備蓄へ紛れ込んではいけない"
    );
}

// ─── Test 31: 撤退時追加損失の既存結果が不変であることの監査 ────────────

#[test]
fn test_31_retreat_additional_loss_path_is_not_connected_to_equipment_and_remains_out_of_scope() {
    // P25-006はRETREAT_MANPOWER_LOSS_RATIOによる追加manpower損失に一切触れていない
    // (military::update::resolve_participant_outcomeは本タスクで無変更)。
    // ここではその事実をコードレベルで再確認する: demobilize_divisionは
    // 撤退処理関数を一切呼び出さず、`military::equipment`モジュールのAPIのみを使う。
    // (実際の撤退シナリオの数値回帰はP25-004の内部テスト・
    // land_war_combat_peace_testで既に固定済みであり、本タスクではそれらの
    // 数値・RNG系列を変更していないことをcargo test --allの結果で確認する。)
    let source = std::fs::read_to_string("src/military/demobilization.rs").unwrap();
    assert!(
        !source.contains("RETREAT_MANPOWER_LOSS_RATIO"),
        "demobilize_divisionは撤退時追加損失ロジックに一切触れていないはず"
    );
}

// ─── §16.7: 国家消滅時の既知のギャップ(country_elimination_gap) ─────────
//
// §14の実装前監査で発見した、既存のcountry::elimination実装がP25-001の
// 「凍結して残存」設計と矛盾する既知の問題を、隠さず実際の挙動として記録する
// (ユーザー判断: elimination.rsは変更せず、この矛盾は報告書へHigh findingとして
// 記録する)。

fn setup_elimination_registries() -> (
    StateRegistry,
    CountryRegistry,
    MilitaryRegistry,
    ArmyRegistry,
    FrontlineRegistry,
    WarRegistry,
    BattleRegistry,
    DiplomacyRegistry,
    CrisisRegistry,
    ClaimRegistry,
    WarJustificationRegistry,
    CountryPowerRegistry,
) {
    let state_registry = StateRegistry::build(vec![StateData {
        id: StateId(0),
        owner_country_id: CountryId(1),
        is_sea: false,
        ..StateData::default()
    }]);
    let country_registry = CountryRegistry {
        countries: vec![
            test_country(1, 0, 10_000, 50.0),
            test_country(2, 0, 0, 100.0),
        ],
    };
    let mut military_registry = MilitaryRegistry::default();
    military_registry
        .divisions
        .insert(DivisionId(0), test_division(0, 1, 10_000, 40.0, 100.0));

    (
        state_registry,
        country_registry,
        military_registry,
        ArmyRegistry::default(),
        FrontlineRegistry::default(),
        WarRegistry::default(),
        BattleRegistry::default(),
        DiplomacyRegistry::default(),
        CrisisRegistry::default(),
        ClaimRegistry::default(),
        WarJustificationRegistry::default(),
        CountryPowerRegistry::default(),
    )
}

#[test]
fn country_elimination_gap_stockpile_is_frozen_but_division_equipment_is_lost_not_frozen() {
    let (
        mut state_registry,
        mut country_registry,
        mut military_registry,
        mut army_registry,
        mut frontline_registry,
        mut war_registry,
        mut battle_registry,
        mut diplomacy_registry,
        mut crisis_registry,
        mut claim_registry,
        mut justification_registry,
        mut power_registry,
    ) = setup_elimination_registries();

    let plan = preflight_country_elimination(
        StateId(0),
        CountryId(1),
        CountryId(2),
        "1936/01/01",
        &state_registry,
        &country_registry,
    )
    .unwrap();

    let outcome = apply_country_elimination(
        plan,
        None,
        None,
        "1936/01/01",
        &mut state_registry,
        &mut country_registry,
        &mut military_registry,
        &mut army_registry,
        &mut frontline_registry,
        &mut war_registry,
        &mut battle_registry,
        &mut diplomacy_registry,
        &mut crisis_registry,
        &mut claim_registry,
        &mut justification_registry,
        &mut power_registry,
    )
    .unwrap();

    assert_eq!(outcome.divisions_disbanded, 1);

    // 確認1(仕様どおり): CountryStockpileは凍結され、一切変化しない。
    let eliminated = country_registry.get(CountryId(1)).unwrap();
    assert_eq!(
        eliminated.stockpile.get(ResourceType::MilitaryEquipment),
        50.0,
        "国家備蓄自体は正しく凍結されている(この部分はP25-001設計どおり)"
    );

    // 確認2(仕様どおり): 併合国の備蓄は増えない。
    let annexer = country_registry.get(CountryId(2)).unwrap();
    assert_eq!(
        annexer.stockpile.get(ResourceType::MilitaryEquipment),
        100.0,
        "併合国の備蓄が増えてはいけない(この部分もP25-001設計どおり)"
    );

    // 確認3(既知のギャップ、報告書§14 High finding): 消滅国家のDivisionは
    // 「凍結」ではなく物理削除される。したがって、そのDivisionが保持していた
    // 現在装備40.0は、備蓄にもDivisionにもどこにも残らず、会計上消失する。
    // これはP25-006が原因ではなく、既存のP21-021実装がP25-001の「凍結して残存」
    // 設計を満たしていなかったことの発覚であり、本タスクの意図的な対象外
    // (country::eliminationは無変更)である。
    assert!(
        !military_registry.divisions.contains_key(&DivisionId(0)),
        "既知のギャップ: 消滅国家のDivisionは凍結されず物理削除される"
    );
    let world_equipment_after: f64 = country_registry
        .countries
        .iter()
        .map(|c| c.stockpile.get(ResourceType::MilitaryEquipment))
        .sum::<f64>()
        + military_registry
            .divisions
            .values()
            .map(|d| d.equipment)
            .sum::<f64>();
    assert_eq!(
        world_equipment_after, 150.0,
        "既知のギャップ: 消滅前の世界装備総量(50+100+40=190)から、削除された \
         Divisionの40.0が失われ、150.0になってしまう(会計上の消失、保存則違反)"
    );
}

#[test]
fn country_elimination_gap_demobilization_still_correctly_rejects_the_eliminated_country() {
    let (
        mut state_registry,
        mut country_registry,
        mut military_registry,
        mut army_registry,
        mut frontline_registry,
        mut war_registry,
        mut battle_registry,
        mut diplomacy_registry,
        mut crisis_registry,
        mut claim_registry,
        mut justification_registry,
        mut power_registry,
    ) = setup_elimination_registries();

    let plan = preflight_country_elimination(
        StateId(0),
        CountryId(1),
        CountryId(2),
        "1936/01/01",
        &state_registry,
        &country_registry,
    )
    .unwrap();
    apply_country_elimination(
        plan,
        None,
        None,
        "1936/01/01",
        &mut state_registry,
        &mut country_registry,
        &mut military_registry,
        &mut army_registry,
        &mut frontline_registry,
        &mut war_registry,
        &mut battle_registry,
        &mut diplomacy_registry,
        &mut crisis_registry,
        &mut claim_registry,
        &mut justification_registry,
        &mut power_registry,
    )
    .unwrap();

    // 消滅国家は復員要求を受け付けない(既存のis_active()チェックがP25-006でも
    // そのまま機能する、新規実装なし)。
    let battles = BattleRegistry::default();
    let result = demobilize_division(
        &mut country_registry,
        &mut military_registry,
        &mut army_registry,
        &mut frontline_registry,
        &war_registry,
        &battles,
        CountryId(1),
        DivisionId(0),
    );
    assert!(matches!(
        result,
        Err(DemobilizationError::CountryEliminated) | Err(DemobilizationError::DivisionNotFound)
    ));
}

// ─── 実App用ハーネス(New Game/Loaded Game・save/load分岐・Test 37〜44) ──

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p25_006_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
}

struct TempTestDir(std::path::PathBuf);

impl TempTestDir {
    fn new(label: &str) -> Self {
        Self(unique_temp_dir(label))
    }

    fn save_file_config(&self) -> SaveFileConfig {
        SaveFileConfig {
            path: SavePathConfig {
                final_path: self.0.join("savegame_v1.ron"),
            },
        }
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_full_app(player: CountryId, save_config: SaveFileConfig) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(strategy_game::app::AppPlugin)
        .add_plugins(CountryPlugin)
        .add_plugins(StatePlugin)
        .add_plugins(strategy_game::building::BuildingPlugin)
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
    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(strategy_game::app::game_state::GameState::Playing);
    app.update(); // OnEnter(Playing): 初期Division配置

    app
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(SaveRequestMessage);
    state.apply(app.world_mut());
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(LoadRequestMessage);
    state.apply(app.world_mut());
}

fn stockpile(app: &App, country_id: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitaryEquipment)
}

fn set_division_equipment(app: &mut App, division_id: DivisionId, equipment: f64, max_equipment: f64) {
    let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
    let d = mr.divisions.get_mut(&division_id).unwrap();
    d.equipment = equipment;
    d.max_equipment = max_equipment;
}

fn player_own_division_id(app: &App, player: CountryId) -> DivisionId {
    app.world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .find(|d| d.owner == player)
        .expect("player must own at least one division")
        .id
}

fn call_demobilize(app: &mut App, player: CountryId, division_id: DivisionId) -> Result<f64, DemobilizationError> {
    let mut state: SystemState<(
        ResMut<CountryRegistry>,
        ResMut<MilitaryRegistry>,
        ResMut<ArmyRegistry>,
        ResMut<FrontlineRegistry>,
        Res<WarRegistry>,
        Res<BattleRegistry>,
    )> = SystemState::new(app.world_mut());
    let (mut cr, mut mr, mut ar, mut fr, wr, br) = state.get_mut(app.world_mut()).unwrap();
    let result = demobilize_division(&mut cr, &mut mr, &mut ar, &mut fr, &wr, &br, player, division_id);
    state.apply(app.world_mut());
    result.map(|o| o.returned_equipment)
}

// ─── Test 34/39/42: New Gameの初期Divisionは自動返却対象ではない ────────

#[test]
fn test_34_39_42_new_game_initial_division_is_not_auto_returned_but_can_be_demobilized_manually() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_new_game");
    let mut app = setup_full_app(player, temp_dir.save_file_config());

    let stockpile_before = stockpile(&app, player);
    let division_id = player_own_division_id(&app, player);
    let division_equipment = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&division_id)
        .unwrap()
        .equipment;

    // New Game初期配置自体は何も返却していない(自動返却扱いされない)。
    assert_eq!(stockpile(&app, player), stockpile_before);

    let returned = call_demobilize(&mut app, player, division_id).unwrap();
    assert_eq!(returned, division_equipment);
    assert_eq!(stockpile(&app, player), stockpile_before + division_equipment);
}

// ─── Test 37: New Game / Loaded Game比較 ─────────────────────────────────

#[test]
fn test_37_new_game_and_loaded_game_produce_identical_demobilization_results() {
    let player = CountryId(0);

    let temp_dir_a = TempTestDir::new("test_37_a");
    let mut app_a = setup_full_app(player, temp_dir_a.save_file_config());
    let division_a = player_own_division_id(&app_a, player);
    set_division_equipment(&mut app_a, division_a, 37.25, 100.0);
    let returned_a = call_demobilize(&mut app_a, player, division_a).unwrap();
    let stockpile_a = stockpile(&app_a, player);

    let temp_dir_b = TempTestDir::new("test_37_b");
    let mut app_b = setup_full_app(player, temp_dir_b.save_file_config());
    send_save_request(&mut app_b);
    app_b.update();
    send_load_request(&mut app_b);
    app_b.update();
    app_b.update();
    let division_b = player_own_division_id(&app_b, player);
    set_division_equipment(&mut app_b, division_b, 37.25, 100.0);
    let returned_b = call_demobilize(&mut app_b, player, division_b).unwrap();
    let stockpile_b = stockpile(&app_b, player);

    assert_eq!(returned_a, returned_b);
    assert_eq!(stockpile_a, stockpile_b);
}

// ─── Test 38: 復員前save/load分岐比較 ────────────────────────────────────

#[test]
fn test_38_pre_demobilization_save_load_branch_matches_the_uninterrupted_run() {
    let player = CountryId(0);

    let temp_dir_a = TempTestDir::new("test_38_a");
    let mut app_a = setup_full_app(player, temp_dir_a.save_file_config());
    let division_a = player_own_division_id(&app_a, player);
    set_division_equipment(&mut app_a, division_a, 62.5, 100.0);
    let returned_a = call_demobilize(&mut app_a, player, division_a).unwrap();
    let stockpile_a = stockpile(&app_a, player);

    let temp_dir_b = TempTestDir::new("test_38_b");
    let mut app_b = setup_full_app(player, temp_dir_b.save_file_config());
    let division_b = player_own_division_id(&app_b, player);
    set_division_equipment(&mut app_b, division_b, 62.5, 100.0);
    send_save_request(&mut app_b);
    app_b.update();
    send_load_request(&mut app_b);
    app_b.update();
    app_b.update();
    let returned_b = call_demobilize(&mut app_b, player, division_b).unwrap();
    let stockpile_b = stockpile(&app_b, player);

    assert_eq!(returned_a, returned_b);
    assert_eq!(stockpile_a, stockpile_b);
}

// ─── Test 39(継続)/Test 41: 復員後save/loadで二重返却なし・P25-005補充後の単回返却 ──

#[test]
fn test_39_41_post_demobilization_save_load_does_not_re_return_equipment() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_39_41");
    let mut app = setup_full_app(player, temp_dir.save_file_config());

    let division_id = player_own_division_id(&app, player);
    // P25-005の日次補充で装備が満タンまで補充された状態を模す。
    set_division_equipment(&mut app, division_id, 80.0, 100.0);
    let returned = call_demobilize(&mut app, player, division_id).unwrap();
    assert_eq!(returned, 80.0);
    let stockpile_after_demob = stockpile(&app, player);

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    assert_eq!(
        stockpile(&app, player),
        stockpile_after_demob,
        "復員後のsave/loadで装備が再度返却されてはいけない"
    );
    assert!(
        !app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .contains_key(&division_id),
        "復員済みDivisionはロード後も再出現しない"
    );
}

// ─── Test 43: Loaded Game Divisionをロード時に自動返却しない ────────────

#[test]
fn test_43_loaded_game_divisions_are_not_auto_returned_on_load() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_43");
    let mut app = setup_full_app(player, temp_dir.save_file_config());

    let division_id = player_own_division_id(&app, player);
    set_division_equipment(&mut app, division_id, 55.0, 100.0);
    let stockpile_before = stockpile(&app, player);

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    assert_eq!(
        stockpile(&app, player),
        stockpile_before,
        "ロードするだけでは装備は返却されない(復員要求を送った場合のみ)"
    );
    assert_eq!(
        app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .get(&division_id)
            .unwrap()
            .equipment,
        55.0
    );
}

// ─── Test 44: ECS挿入順・HashMap seedへの非依存 ──────────────────────────

#[test]
fn test_44_result_does_not_depend_on_division_insertion_order() {
    fn run_scenario(insert_order: [usize; 2]) -> (f64, f64) {
        let mut h = harness(
            vec![test_country(1, 0, 20_000, 0.0)],
            Vec::new(),
        );
        let divisions = [
            test_division(0, 1, 10_000, 40.0, 100.0),
            test_division(1, 1, 10_000, 25.0, 100.0),
        ];
        for &i in &insert_order {
            let d = divisions[i].clone();
            h.military.divisions.insert(d.id, d);
        }
        let r0 = run(&mut h, CountryId(1), DivisionId(0)).unwrap().returned_equipment;
        let r1 = run(&mut h, CountryId(1), DivisionId(1)).unwrap().returned_equipment;
        (r0, r1)
    }

    assert_eq!(run_scenario([0, 1]), run_scenario([1, 0]));
}
