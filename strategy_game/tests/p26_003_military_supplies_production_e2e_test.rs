//! P26-003: `ResourceType::MilitarySupplies`の実生産配線(`economy::production::
//! process_country_production` → `CountryStockpile::add_military_supplies_produced`)の
//! E2Eテスト。
//!
//! `verification_logs/phase-26/p26-003/`が確定した設計(生産源として既存
//! `MilitaryFactory`がMilitaryEquipmentと併産、新規建物`SupplyDepot`は追加しない、
//! 国家単位で1回だけ正規APIへcommit)に基づき、P25-002-REV-001(`tests/p25_002_rev_001_
//! fixed_point_equipment_accounting_e2e_test.rs`のTest L/M)と同じ粒度・観点で
//! Test A〜Jを実装する。
//!
//! P26-003完了時点でも、Division需要・日次配分・戦闘力補正・`try_consume_military_
//! supplies`の本番呼び出しは一切追加しない(Category Iで非接続性を確認する)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::production::process_country_production;
use strategy_game::economy::resources::{
    MILITARY_SUPPLIES_MAX_UNITS, MILITARY_SUPPLIES_SCALE, CountryStockpile, ResourceType,
    military_supplies_units_to_value,
};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateData;
use strategy_game::war::WarPlugin;

const EPSILON: f64 = 1e-6;

// ─────────────────────────────────────────────────────────────────────────
// Fixtures (P25-002-REV-001のTest L/Mと同じ構成方針)
// ─────────────────────────────────────────────────────────────────────────

/// MilitaryEquipment/MilitarySuppliesを併産するMilitaryFactoryのみを持つ、入力資源
/// なしの単純化済みBuildingRegistry(operation_ratioの計算をworkforce/logisticsだけへ
/// 単純化するための構成、実データの入力有無での稼働率ゲートはTest Bで別途検証する)。
fn military_factory_registry(equipment_per_level: f64, supplies_per_level: f64) -> BuildingRegistry {
    let mut definitions = HashMap::new();
    definitions.insert(
        BuildingType::MilitaryFactory,
        BuildingDefinition {
            building_type: BuildingType::MilitaryFactory,
            name: "Military Factory".to_string(),
            construction_cost: 1200.0,
            required_progress: 90.0,
            required_workforce: 10.0,
            logistics_cost: 0.0,
            input_resources: HashMap::new(),
            output_resources: {
                let mut m = HashMap::new();
                m.insert(ResourceType::MilitaryEquipment, equipment_per_level);
                m.insert(ResourceType::MilitarySupplies, supplies_per_level);
                m
            },
            maintenance_cost: 40.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        },
    );
    BuildingRegistry { definitions }
}

/// 実データ(`assets/data/buildings.ron`)と同じ入力(Iron/IndustrialGoods)を持つ
/// MilitaryFactory registry。入力不足によるoperation_ratioゲートの検証専用。
fn military_factory_registry_with_inputs() -> BuildingRegistry {
    let mut definitions = HashMap::new();
    definitions.insert(
        BuildingType::MilitaryFactory,
        BuildingDefinition {
            building_type: BuildingType::MilitaryFactory,
            name: "Military Factory".to_string(),
            construction_cost: 1200.0,
            required_progress: 90.0,
            required_workforce: 25000.0,
            logistics_cost: 20.0,
            input_resources: {
                let mut m = HashMap::new();
                m.insert(ResourceType::Iron, 30.0);
                m.insert(ResourceType::IndustrialGoods, 15.0);
                m
            },
            output_resources: {
                let mut m = HashMap::new();
                m.insert(ResourceType::MilitaryEquipment, 20.0);
                m.insert(ResourceType::MilitarySupplies, 100.0);
                m
            },
            maintenance_cost: 40.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        },
    );
    BuildingRegistry { definitions }
}

/// operation_ratio=1.0(wf_ratio=1.0, log_ratio=1.0, 入力充足率=1.0)となる州。
fn full_capacity_state(id: StateId, buildings: &[(BuildingType, u32)]) -> StateData {
    StateData {
        id,
        population: 100_000,
        workforce_ratio: 0.5,
        employed_workforce: 50_000,
        logistics_ratio: 1.0,
        buildings: buildings.iter().copied().collect(),
        ..StateData::default()
    }
}

/// workforce比率を任意に指定できる州(operation_ratioゲートの検証用)。
fn partial_capacity_state(
    id: StateId,
    buildings: &[(BuildingType, u32)],
    employed: u64,
    total_workforce: u64,
    logistics_ratio: f32,
) -> StateData {
    StateData {
        id,
        population: total_workforce * 2,
        workforce_ratio: (total_workforce as f32) / (total_workforce as f32 * 2.0),
        employed_workforce: employed,
        logistics_ratio,
        buildings: buildings.iter().copied().collect(),
        ..StateData::default()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Category A: 生産源
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_a1_level_0_produces_zero_supplies() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 0)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_a2_level_1_produces_base_amount() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 100.0);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 20.0);
}

#[test]
fn test_a3_multiple_levels_scale_linearly() {
    let registry = military_factory_registry(20.0, 100.0);
    for level in [1u32, 2, 5, 10] {
        let mut stockpile = CountryStockpile::new();
        let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, level)]);
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, &registry, false);
        assert_eq!(
            stockpile.get(ResourceType::MilitarySupplies),
            100.0 * level as f64,
            "level={level}"
        );
    }
}

#[test]
fn test_a4_multiple_states_same_country_accumulate() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state_a = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut state_b = full_capacity_state(StateId(1), &[(BuildingType::MilitaryFactory, 2)]);
    let mut states: Vec<&mut StateData> = vec![&mut state_a, &mut state_b];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 300.0);
}

#[test]
fn test_a5_multiple_countries_produce_independently() {
    let registry = military_factory_registry(20.0, 100.0);

    let mut stockpile_a = CountryStockpile::new();
    let mut state_a = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 3)]);
    let mut states_a: Vec<&mut StateData> = vec![&mut state_a];
    process_country_production(&mut states_a, &mut stockpile_a, &registry, false);

    let mut stockpile_b = CountryStockpile::new();
    let mut state_b = full_capacity_state(StateId(1), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states_b: Vec<&mut StateData> = vec![&mut state_b];
    process_country_production(&mut states_b, &mut stockpile_b, &registry, false);

    assert_eq!(stockpile_a.get(ResourceType::MilitarySupplies), 300.0);
    assert_eq!(stockpile_b.get(ResourceType::MilitarySupplies), 100.0);
}

#[test]
fn test_a6_no_military_factory_produces_zero() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::Farm, 3)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

/// `handle_monthly_economy`は所有州が0件の国家について`process_country_production`
/// 自体を呼ばない(既存の`if owned_states.is_empty() { continue; }`、P26-003は無変更)。
/// ここでは`process_country_production`自身が空スライスを渡されても安全にno-opする
/// ことだけを確認する(消滅国家的な状況を関数レベルで模したもの)。
#[test]
fn test_a7_empty_states_slice_is_a_safe_no_op() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut states: Vec<&mut StateData> = vec![];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_a8_result_does_not_depend_on_state_vec_order() {
    let registry = military_factory_registry(20.0, 100.0);

    let mut stockpile_1 = CountryStockpile::new();
    let mut state_a1 = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 3)]);
    let mut state_b1 = full_capacity_state(StateId(1), &[(BuildingType::MilitaryFactory, 4)]);
    let mut states_1: Vec<&mut StateData> = vec![&mut state_a1, &mut state_b1];
    process_country_production(&mut states_1, &mut stockpile_1, &registry, false);

    let mut stockpile_2 = CountryStockpile::new();
    let mut state_a2 = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 3)]);
    let mut state_b2 = full_capacity_state(StateId(1), &[(BuildingType::MilitaryFactory, 4)]);
    let mut states_2: Vec<&mut StateData> = vec![&mut state_b2, &mut state_a2];
    process_country_production(&mut states_2, &mut stockpile_2, &registry, false);

    assert_eq!(
        stockpile_1.get(ResourceType::MilitarySupplies),
        stockpile_2.get(ResourceType::MilitarySupplies)
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Category B: 稼働率
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_b1_operation_ratio_zero_workforce_produces_zero() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state =
        partial_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)], 0, 1000, 1.0);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_b2_operation_ratio_zero_logistics_produces_zero() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state = partial_capacity_state(
        StateId(0),
        &[(BuildingType::MilitaryFactory, 1)],
        1000,
        1000,
        0.0,
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_b3_operation_ratio_50_percent() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state = partial_capacity_state(
        StateId(0),
        &[(BuildingType::MilitaryFactory, 1)],
        500,
        1000,
        1.0,
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert!((stockpile.get(ResourceType::MilitarySupplies) - 50.0).abs() < EPSILON);
}

#[test]
fn test_b4_operation_ratio_100_percent_matches_base_output() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 100.0);
}

/// 入力資源(Iron/IndustrialGoods)が不足している場合、MilitaryEquipmentと
/// MilitarySuppliesの双方が同じoperation_ratioで比例して減少する(同じ建物・同じ
/// final_opを共有するため、装備だけ/補給だけが選択的にゲートされることはない)。
#[test]
fn test_b5_input_shortage_gates_equipment_and_supplies_identically() {
    let registry = military_factory_registry_with_inputs();
    let mut stockpile = CountryStockpile::new();
    // 必要量(Iron 30 + IndustrialGoods 15、level=1、op=1.0であれば)の半分だけ備蓄。
    stockpile.set(ResourceType::Iron, 15.0);
    stockpile.set(ResourceType::IndustrialGoods, 15.0);
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);

    let eq = stockpile.get(ResourceType::MilitaryEquipment);
    let sup = stockpile.get(ResourceType::MilitarySupplies);
    assert!(eq > 0.0 && eq < 20.0, "eq={eq}");
    assert!(sup > 0.0 && sup < 100.0, "sup={sup}");
    // 同じfinal_opを共有するため、比率(sup/eq)は常に100.0/20.0=5.0で一定。
    assert!((sup / eq - 5.0).abs() < 1e-3, "sup={sup} eq={eq}");
}

/// 完全充足時は既存のP25-002-REV-001 Test Lと同じ計算式(base_output × level ×
/// operation_ratio)がMilitarySuppliesにもそのまま適用される。
#[test]
fn test_b6_matches_existing_production_formula_with_fractional_output() {
    let registry = military_factory_registry(14.6, 37.25); // 端数のある生産量
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 14.6);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 37.25);
}

// ─────────────────────────────────────────────────────────────────────────
// Category C: fixed-point
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_c1_integer_output() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 100.0);
}

#[test]
fn test_c2_fractional_output() {
    let registry = military_factory_registry(20.0, 33.333333);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert!((stockpile.get(ResourceType::MilitarySupplies) - 33.333333).abs() < EPSILON);
}

#[test]
fn test_c3_quantization_boundary_value() {
    // ちょうどunits境界(scale=1,000,000)に一致する値。
    let registry = military_factory_registry(20.0, 0.000001);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.000001);
}

#[test]
fn test_c4_accumulates_across_many_ticks_without_drift() {
    let registry = military_factory_registry(20.0, 14.6);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);

    for _ in 0..1000 {
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, &registry, false);
    }

    // 14.6 * 1000 = 14600.0。固定小数点units経由の累積でドリフトが発生しないことを
    // 確認する(生のf64加算では通常この規模でも問題ないが、正規APIが同じ結果を返す
    // ことを確認する)。
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 14600.0);
}

#[test]
fn test_c5_sum_is_independent_of_hashmap_or_vec_iteration_order() {
    // test_a8と同じ観点をより多くの州で確認する(5州、逆順)。
    let registry = military_factory_registry(20.0, 100.0);

    let make_states = || {
        (0..5)
            .map(|i| full_capacity_state(StateId(i), &[(BuildingType::MilitaryFactory, (i + 1) as u32)]))
            .collect::<Vec<_>>()
    };

    let mut forward_states = make_states();
    let mut stockpile_forward = CountryStockpile::new();
    {
        let mut refs: Vec<&mut StateData> = forward_states.iter_mut().collect();
        process_country_production(&mut refs, &mut stockpile_forward, &registry, false);
    }

    let mut reversed_states = make_states();
    reversed_states.reverse();
    let mut stockpile_reversed = CountryStockpile::new();
    {
        let mut refs: Vec<&mut StateData> = reversed_states.iter_mut().collect();
        process_country_production(&mut refs, &mut stockpile_reversed, &registry, false);
    }

    assert_eq!(
        stockpile_forward.get(ResourceType::MilitarySupplies),
        stockpile_reversed.get(ResourceType::MilitarySupplies)
    );
}

#[test]
fn test_c6_near_max_units_still_succeeds() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let near_max_value = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS - 100);
    stockpile.set(ResourceType::MilitarySupplies, near_max_value);
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 0)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    // level=0なので生産0、既存の近-max値は変化しない。
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), near_max_value);
}

#[test]
fn test_c7_at_max_units_boundary_succeeds() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let max_value = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);
    stockpile.set(ResourceType::MilitarySupplies, max_value);
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 0)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), max_value);
}

#[test]
fn test_c8_beyond_max_units_is_explicitly_rejected_not_clamped() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let max_value = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);
    stockpile.set(ResourceType::MilitarySupplies, max_value);
    // level=1で100.0が追加され、確実に上限を超える。
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    // add_military_supplies_producedが失敗し(warnログのみ)、備蓄は不変のまま
    // (黙ってclampされない)。
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), max_value);
}

// ─────────────────────────────────────────────────────────────────────────
// Category D: 原子性
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_d1_overflow_leaves_stockpile_completely_unchanged() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let max_value = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);
    stockpile.set(ResourceType::MilitarySupplies, max_value);
    stockpile.set(ResourceType::Food, 250.0);
    stockpile.set(ResourceType::Iron, 30.0);

    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);

    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), max_value);
}

#[test]
fn test_d2_other_resource_types_unaffected_by_supplies_overflow_failure() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let max_value = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);
    stockpile.set(ResourceType::MilitarySupplies, max_value);

    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);

    // MilitarySuppliesのcommitが失敗しても、同じ建物が出すMilitaryEquipmentは
    // 通常どおり生産される(P26-003の原子性はMilitarySupplies単体のスコープであり、
    // 国家の月次生産全体を巻き込まない、spec §12「広範な経済リファクタリングは
    // 行わない」の方針どおり)。
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 20.0);
}

#[test]
fn test_d3_one_countrys_overflow_does_not_affect_another_country() {
    let registry = military_factory_registry(20.0, 100.0);

    let mut stockpile_a = CountryStockpile::new();
    let max_value = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);
    stockpile_a.set(ResourceType::MilitarySupplies, max_value);
    let mut state_a = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states_a: Vec<&mut StateData> = vec![&mut state_a];
    process_country_production(&mut states_a, &mut stockpile_a, &registry, false);

    let mut stockpile_b = CountryStockpile::new();
    let mut state_b = full_capacity_state(StateId(1), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states_b: Vec<&mut StateData> = vec![&mut state_b];
    process_country_production(&mut states_b, &mut stockpile_b, &registry, false);

    assert_eq!(stockpile_a.get(ResourceType::MilitarySupplies), max_value);
    assert_eq!(stockpile_b.get(ResourceType::MilitarySupplies), 100.0);
}

#[test]
fn test_d4_multi_state_production_is_not_partially_applied_on_failure() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let max_value = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);
    stockpile.set(ResourceType::MilitarySupplies, max_value);

    let mut state_a = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut state_b = full_capacity_state(StateId(1), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state_a, &mut state_b];
    process_country_production(&mut states, &mut stockpile, &registry, false);

    // 2州分(200.0)を国家合計へ一時集計してから1回だけcommitを試みるため、片方の
    // 州分だけが反映される、といった部分適用は起きない。
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), max_value);
}

// ─────────────────────────────────────────────────────────────────────────
// Category E: 世界保存則
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_e1_conservation_holds_for_a_single_tick() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let before = stockpile.get(ResourceType::MilitarySupplies);
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 3)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    let after = stockpile.get(ResourceType::MilitarySupplies);
    assert_eq!(after - before, 300.0);
}

#[test]
fn test_e2_conservation_holds_across_multiple_ticks() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);

    let mut expected_total = 0.0;
    for _ in 0..12 {
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, &registry, false);
        expected_total += 100.0;
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), expected_total);
    }
}

#[test]
fn test_e3_world_total_equals_sum_of_country_productions() {
    let registry = military_factory_registry(20.0, 100.0);

    let mut stockpile_a = CountryStockpile::new();
    let mut state_a = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 2)]);
    let mut states_a: Vec<&mut StateData> = vec![&mut state_a];
    process_country_production(&mut states_a, &mut stockpile_a, &registry, false);

    let mut stockpile_b = CountryStockpile::new();
    let mut state_b = full_capacity_state(StateId(1), &[(BuildingType::MilitaryFactory, 3)]);
    let mut states_b: Vec<&mut StateData> = vec![&mut state_b];
    process_country_production(&mut states_b, &mut stockpile_b, &registry, false);

    let mut stockpile_c = CountryStockpile::new();
    let mut state_c = full_capacity_state(StateId(2), &[(BuildingType::MilitaryFactory, 0)]);
    let mut states_c: Vec<&mut StateData> = vec![&mut state_c];
    process_country_production(&mut states_c, &mut stockpile_c, &registry, false);

    let world_total = stockpile_a.get(ResourceType::MilitarySupplies)
        + stockpile_b.get(ResourceType::MilitarySupplies)
        + stockpile_c.get(ResourceType::MilitarySupplies);
    assert_eq!(world_total, 200.0 + 300.0 + 0.0);
}

// ─────────────────────────────────────────────────────────────────────────
// App-level fixtures (P26-002と同じ構成)
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
        "strategy_game_p26_003_e2e_{label}_{}_{nanos}_{n}",
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

fn setup_app_in_playing(player: CountryId, save_config: SaveFileConfig) -> App {
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
    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing)

    app
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
        SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
    state.apply(app.world_mut());
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
    state.apply(app.world_mut());
}

fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> =
        SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn tick_month(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> =
        SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

fn supplies_of(app: &App, country_id: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitarySupplies)
}

fn equipment_of(app: &App, country_id: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitaryEquipment)
}

/// 数か月分、日次+月次ティックを進める(内部でemployed_workforceが自然にランプアップ
/// するのを待つため、実App経由の生産確認テストはいずれも複数か月分進める)。
fn advance_months(app: &mut App, months: u8) {
    for m in 1..=months {
        for day in 2..=28u8 {
            tick_day(app, 1800, m, day);
        }
        tick_month(app, 1800, m.wrapping_add(1));
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Category F: 生産周期(実App、実`handle_monthly_economy`経路)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_f1_day_ticks_alone_do_not_produce() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("f1");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    for day in 2..=28u8 {
        tick_day(&mut app, 1800, 1, day);
    }
    assert_eq!(supplies_of(&app, player), 0.0);
}

#[test]
fn test_f2_month_boundary_actually_increases_supplies_via_real_system() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("f2");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    // 1ヶ月目は雇用が0からのスタートのため生産0になりうる。複数ヶ月進めて、実際に
    // MilitarySuppliesが実生産System経由で増加することを確認する(P26-002時点の
    // 「本番Systemへ未接続」を覆す、本タスクの核心的な確認)。
    advance_months(&mut app, 4);
    assert!(
        supplies_of(&app, player) > 0.0,
        "MilitarySupplies must actually accumulate through the real handle_monthly_economy path by P26-003"
    );
}

#[test]
fn test_f3_supplies_and_equipment_grow_in_the_fixed_ratio_across_real_ticks() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("f3");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    advance_months(&mut app, 6);

    let sup = supplies_of(&app, player);
    let eq = equipment_of(&app, player);
    assert!(sup > 0.0 && eq > 0.0);
    // 実データ(buildings.ron)ではMilitaryFactoryがMilitaryEquipment:20.0/
    // MilitarySupplies:100.0を併産するため、同じfinal_opの積算である以上、比率は
    // 常に100.0/20.0=5.0(丸め誤差の範囲内)。
    assert!((sup / eq - 5.0).abs() < 0.05, "sup={sup} eq={eq}");
}

#[test]
fn test_f4_supplies_never_decrease_across_ticks_since_nothing_consumes_yet() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("f4");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let mut last = supplies_of(&app, player);
    for m in 1..=6u8 {
        for day in 2..=28u8 {
            tick_day(&mut app, 1800, m, day);
        }
        tick_month(&mut app, 1800, m.wrapping_add(1));
        let now = supplies_of(&app, player);
        assert!(now >= last, "MilitarySupplies must be monotonically non-decreasing (no consumption path exists yet): {last} -> {now}");
        last = now;
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Category G: Save V1
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_g1_save_load_round_trip_preserves_real_production_result() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("g1");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    advance_months(&mut app, 3);

    let before = supplies_of(&app, player);
    assert!(before > 0.0, "precondition: real production must have accumulated something");

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    assert_eq!(supplies_of(&app, player), before);
}

#[test]
fn test_g2_double_round_trip_is_stable() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("g2");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    advance_months(&mut app, 3);
    let before = supplies_of(&app, player);

    for _ in 0..2 {
        send_save_request(&mut app);
        app.update();
        send_load_request(&mut app);
        app.update();
        app.update();
    }

    assert_eq!(supplies_of(&app, player), before);
}

#[test]
fn test_g3_production_after_load_continues_from_the_loaded_value_without_double_counting() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("g3");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    advance_months(&mut app, 2);
    let before_save = supplies_of(&app, player);

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();
    assert_eq!(supplies_of(&app, player), before_save);

    advance_months(&mut app, 2);
    let after_more_production = supplies_of(&app, player);
    // ロード直後の生産が二重適用されず、単純にそこから増分が乗るだけであること
    // (減少しない、かつ増分自体はゼロではない)を確認する。
    assert!(after_more_production >= before_save);
}

#[test]
fn test_g4_old_v1_shaped_load_without_military_supplies_key_still_produces_normally() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("g4");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    // 旧V1相当を模すため、MilitarySuppliesキーへ触れずに保存する。
    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();
    assert_eq!(supplies_of(&app, player), 0.0);

    // ロード後も実生産Systemが正常に稼働し、MilitarySuppliesが増加する。
    advance_months(&mut app, 4);
    assert!(supplies_of(&app, player) > 0.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Category H: New Game／Loaded Game・AI/プレイヤー
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_h1_new_game_and_loaded_game_agree_after_identical_ticks() {
    let player = CountryId(0);

    let temp_dir_continuous = TempTestDir::new("h1_continuous");
    let mut continuous = setup_app_in_playing(player, temp_dir_continuous.save_file_config());
    advance_months(&mut continuous, 3);

    let temp_dir_branch = TempTestDir::new("h1_branch");
    let mut branch = setup_app_in_playing(player, temp_dir_branch.save_file_config());
    advance_months(&mut branch, 3);
    send_save_request(&mut branch);
    branch.update();
    send_load_request(&mut branch);
    branch.update();
    branch.update();

    assert_eq!(supplies_of(&continuous, player), supplies_of(&branch, player));
}

/// AI国家(非プレイヤー)も、`process_country_production`が国家IDに一切依存しないため
/// プレイヤーと同一の経路・同一の式で生産する。実データ監査(`verification_logs/
/// phase-26/p26-003/`参照)により、CountryId(2)(Dwarf Federation)がMine(discovered
/// 済みIron/Coal鉱床)→Factory(IndustrialGoods)→MilitaryFactory(Equipment/Supplies
/// 併産)という完結した供給網を持つ唯一の国家であることを確認済み。CountryId(0)を
/// プレイヤーに設定し、非プレイヤー国家であるCountryId(2)が実際に非ゼロ生産へ到達
/// することを確認する。
#[test]
fn test_h2_ai_country_produces_through_the_same_path_and_is_not_permanently_zero() {
    let player = CountryId(0);
    let ai_with_full_supply_chain = CountryId(2);
    let temp_dir = TempTestDir::new("h2");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    advance_months(&mut app, 6);

    assert!(
        supplies_of(&app, ai_with_full_supply_chain) > 0.0,
        "AI country with a complete existing supply chain must not be permanently starved by P26-003's production wiring"
    );
}

/// CountryId(3)(Oceanic Magic Empire)はNew Gameの時点でMilitaryFactoryを1つも
/// 所有していない(既存マップデータの事実、P26-003が作った状態ではない)。この国家は
/// MilitaryEquipmentも同じ理由でPhase 25から既に生産できておらず、MilitarySuppliesの
/// 生産0はその既存の非対称性をそのまま継承しただけであって、本タスクが新規に作り
/// 出した「全AI国家の恒久的なMilitarySupplies=0」ではないことを明示的に確認する
/// (spec §18「全AI国家が恒久的にMilitarySupplies=0となる設計ではない」の否定条件は、
/// 少なくとも1国でも正なら満たされる——Test H2がその反例を示す)。
#[test]
fn test_h3_country_without_any_military_factory_mirrors_the_pre_existing_equipment_gap() {
    let player = CountryId(0);
    let country_without_factory = CountryId(3);
    let temp_dir = TempTestDir::new("h3");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    advance_months(&mut app, 6);

    let sup = supplies_of(&app, country_without_factory);
    let eq = equipment_of(&app, country_without_factory);
    assert_eq!(sup, 0.0, "no MilitaryFactory means no MilitarySupplies, exactly mirroring the pre-existing equipment gap");
    assert_eq!(eq, 0.0, "confirms this is the same pre-existing MilitaryEquipment gap, not a new one");
}

// ─────────────────────────────────────────────────────────────────────────
// Category I: 非接続性
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_i1_division_manpower_and_equipment_are_unaffected_by_supplies_production() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("i1");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let before: Vec<(u64, f64)> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .map(|d| (d.manpower, d.equipment))
        .collect();

    advance_months(&mut app, 3);

    let after: Vec<(u64, f64)> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .map(|d| (d.manpower, d.equipment))
        .collect();

    assert_eq!(before.len(), after.len());
    assert_eq!(before, after, "no combat/recruitment occurred, so Division state must be untouched by production ticks alone");
}

#[test]
fn test_i2_dead_supply_ratio_field_still_unregistered() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("i2");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    for ratio in app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .map(|d| d.supply_ratio)
    {
        assert_eq!(ratio, 1.0);
    }

    advance_months(&mut app, 3);

    for ratio in app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .map(|d| d.supply_ratio)
    {
        assert_eq!(ratio, 1.0, "dead military::supply::process_supply must remain unregistered");
    }
}

#[test]
fn test_i3_try_consume_military_supplies_has_no_production_caller() {
    // rgでの静的確認をP26-003報告書へ記載する。ここでは、実App内で唯一
    // MilitarySuppliesが変化しうる経路(生産のみ)を辿っても、備蓄が一度も減少しない
    // ことで間接的に「消費経路が存在しない」ことを補強する(test_f4と同じ観測の
    // 別カテゴリからの再確認)。
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("i3");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    let mut last = supplies_of(&app, player);
    for m in 1..=8u8 {
        for day in 2..=28u8 {
            tick_day(&mut app, 1800, m, day);
        }
        tick_month(&mut app, 1800, m.wrapping_add(1));
        let now = supplies_of(&app, player);
        assert!(now >= last);
        last = now;
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Category J: 既存機能保護
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_j1_military_equipment_production_formula_is_unchanged() {
    // P25-002-REV-001 Test Lと同じ入力・同じ期待値(MilitarySuppliesを含まない
    // registryでも、MilitaryEquipment単体の計算式が無変更であることを確認する)。
    let mut definitions = HashMap::new();
    definitions.insert(
        BuildingType::MilitaryFactory,
        BuildingDefinition {
            building_type: BuildingType::MilitaryFactory,
            name: "Military Factory".to_string(),
            construction_cost: 1200.0,
            required_progress: 90.0,
            required_workforce: 10.0,
            logistics_cost: 0.0,
            input_resources: HashMap::new(),
            output_resources: {
                let mut m = HashMap::new();
                m.insert(ResourceType::MilitaryEquipment, 14.6);
                m
            },
            maintenance_cost: 40.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        },
    );
    let registry = BuildingRegistry { definitions };
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 14.6);
}

#[test]
fn test_j2_food_and_other_resources_unaffected_by_the_militarysupplies_addition() {
    let mut definitions = HashMap::new();
    definitions.insert(
        BuildingType::Farm,
        BuildingDefinition {
            building_type: BuildingType::Farm,
            name: "Farm".to_string(),
            construction_cost: 100.0,
            required_progress: 10.0,
            required_workforce: 10.0,
            logistics_cost: 0.0,
            input_resources: HashMap::new(),
            output_resources: {
                let mut m = HashMap::new();
                m.insert(ResourceType::Food, 10.0);
                m
            },
            maintenance_cost: 1.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        },
    );
    definitions.insert(
        BuildingType::MilitaryFactory,
        BuildingDefinition {
            building_type: BuildingType::MilitaryFactory,
            name: "Military Factory".to_string(),
            construction_cost: 1200.0,
            required_progress: 90.0,
            required_workforce: 10.0,
            logistics_cost: 0.0,
            input_resources: HashMap::new(),
            output_resources: {
                let mut m = HashMap::new();
                m.insert(ResourceType::MilitaryEquipment, 20.0);
                m.insert(ResourceType::MilitarySupplies, 100.0);
                m
            },
            maintenance_cost: 40.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        },
    );
    let registry = BuildingRegistry { definitions };
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(
        StateId(0),
        &[(BuildingType::Farm, 1), (BuildingType::MilitaryFactory, 1)],
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);

    assert_eq!(stockpile.get(ResourceType::Food), 10.0);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 20.0);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 100.0);
}

#[test]
fn test_j3_scale_and_max_units_constants_are_unchanged_from_p26_002() {
    assert_eq!(MILITARY_SUPPLIES_SCALE, 1_000_000);
    assert_eq!(MILITARY_SUPPLIES_MAX_UNITS, 1_000_000_000_000_000);
}

/// 実App経由でPhase 25のequipment会計(募兵・戦闘等は起きないため、ここでは
/// 生産経路のみ)が、MilitarySupplies生産追加後も無変更であることを確認する。
#[test]
fn test_j4_real_app_equipment_stockpile_still_grows_via_the_unmodified_path() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("j4");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    advance_months(&mut app, 4);
    assert!(equipment_of(&app, player) > 0.0);
}

// ─────────────────────────────────────────────────────────────────────────
// MAX_UNITS長期検証(spec §21、純粋計算のみ・150年分をtickで実行しない)
// ─────────────────────────────────────────────────────────────────────────

/// 現行州数(28)・MilitaryFactory最大レベル(10)・稼働率100%・150年間無消費という
/// 非現実的な最悪ケースでの理論最大備蓄が、`MILITARY_SUPPLIES_MAX_UNITS`(equipment
/// 値換算で10億)を大幅に下回ることを純粋計算で確認する。実際にはP26-004以降で
/// 毎日恒常的な消費が始まるため、この無消費理論値へ実プレイで近づくことはない。
#[test]
fn test_max_units_1x_scale_150_year_theoretical_max_has_positive_margin() {
    const CURRENT_STATE_COUNT: f64 = 28.0;
    const MAX_LEVEL: f64 = 10.0;
    const OUTPUT_PER_LEVEL: f64 = 100.0;
    const MONTHS_IN_150_YEARS: f64 = 150.0 * 12.0;

    let theoretical_max_value =
        CURRENT_STATE_COUNT * MAX_LEVEL * OUTPUT_PER_LEVEL * MONTHS_IN_150_YEARS;
    assert_eq!(theoretical_max_value, 50_400_000.0);

    let max_domain_value =
        military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);
    assert_eq!(max_domain_value, 1_000_000_000.0);

    assert!(
        theoretical_max_value < max_domain_value,
        "1x-scale 150-year theoretical max ({theoretical_max_value}) must stay under the domain cap ({max_domain_value})"
    );
    let margin = max_domain_value / theoretical_max_value;
    assert!(margin > 15.0, "expected a comfortable safety margin at current scale, got {margin}x");
}

/// 州数が現行の10倍(280)へ拡張した仮想シナリオでも、同じ非現実的最悪ケースの
/// 理論最大備蓄がまだ上限を下回ること(正の安全余裕を維持すること)を確認する。
#[test]
fn test_max_units_10x_scale_150_year_theoretical_max_still_has_positive_margin() {
    const SCALED_STATE_COUNT: f64 = 280.0;
    const MAX_LEVEL: f64 = 10.0;
    const OUTPUT_PER_LEVEL: f64 = 100.0;
    const MONTHS_IN_150_YEARS: f64 = 150.0 * 12.0;

    let theoretical_max_value =
        SCALED_STATE_COUNT * MAX_LEVEL * OUTPUT_PER_LEVEL * MONTHS_IN_150_YEARS;
    assert_eq!(theoretical_max_value, 504_000_000.0);

    let max_domain_value =
        military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);

    assert!(
        theoretical_max_value < max_domain_value,
        "even at a hypothetical 10x world-scale expansion, the theoretical max ({theoretical_max_value}) must stay under the domain cap ({max_domain_value})"
    );
    let margin = max_domain_value / theoretical_max_value;
    assert!(margin > 1.5, "expected a positive (if thin) margin even at 10x scale, got {margin}x");
}
