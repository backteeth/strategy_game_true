//! P26-002: `ResourceType::MilitarySupplies`とその固定小数点会計基盤(`economy::resources`)
//! のE2Eテスト。
//!
//! `verification_logs/phase-26/p26-001/`が確定した設計(単一抽象資源、
//! `MilitaryEquipment`との型・意味の分離、国家備蓄方式、Division携行在庫なし、
//! Save V1維持)に基づき、P25-002/P25-002-REV-001(`tests/p25_002_equipment_accounting_
//! foundation_e2e_test.rs`/`tests/p25_002_rev_001_fixed_point_equipment_accounting_
//! e2e_test.rs`)と同じ粒度・同じ検証観点でTest A〜Iを実装する。
//!
//! P26-002時点では、`MilitarySupplies`はどの本番生産・消費Systemからも呼び出されない
//! (会計基盤のみ)。Category Iはこの非接続性を実App経由で確認する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::CountryId;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::{
    CountryStockpile, MILITARY_SUPPLIES_MAX_UNITS, MILITARY_SUPPLIES_SCALE,
    MilitarySuppliesUnitsError, ResourceType, military_supplies_units_to_value,
    military_supplies_value_to_units,
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
use strategy_game::war::WarPlugin;

fn stockpile_with(resource: ResourceType, amount: f64) -> CountryStockpile {
    let mut s = CountryStockpile::new();
    s.set(resource, amount);
    s
}

// ─────────────────────────────────────────────────────────────────────────
// Test A: ResourceType
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_a_all_contains_military_supplies_exactly_once() {
    let count = ResourceType::ALL
        .iter()
        .filter(|&&r| r == ResourceType::MilitarySupplies)
        .count();
    assert_eq!(count, 1, "MilitarySupplies must appear exactly once in ALL");
    assert_eq!(ResourceType::ALL.len(), 9);
}

#[test]
fn test_a_all_existing_variants_are_preserved() {
    let expected = [
        ResourceType::Food,
        ResourceType::Wood,
        ResourceType::Iron,
        ResourceType::Coal,
        ResourceType::MagicCrystal,
        ResourceType::IndustrialGoods,
        ResourceType::MilitaryEquipment,
        ResourceType::RawMagicCrystal,
    ];
    for res in expected {
        assert!(
            ResourceType::ALL.contains(&res),
            "existing variant {res:?} must remain in ALL"
        );
    }
}

#[test]
fn test_a_unregistered_military_supplies_key_reads_as_zero() {
    let stockpile = CountryStockpile::default();
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_a_military_supplies_operations_do_not_affect_food_or_equipment() {
    let mut stockpile = CountryStockpile::new();
    stockpile.set(ResourceType::Food, 250.0);
    stockpile.set(ResourceType::MilitaryEquipment, 88.0);

    stockpile.add_military_supplies_produced(40.0).unwrap();
    stockpile.try_consume_military_supplies(15.0).unwrap();

    assert_eq!(stockpile.get(ResourceType::Food), 250.0);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 88.0);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 25.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Test B: 基本変換
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_b_basic_conversions() {
    assert_eq!(military_supplies_value_to_units(0.0).unwrap(), 0);
    assert_eq!(military_supplies_value_to_units(1.0).unwrap(), 1_000_000);
    assert_eq!(military_supplies_value_to_units(3.7).unwrap(), 3_700_000);
    assert_eq!(military_supplies_value_to_units(0.000001).unwrap(), 1);
}

#[test]
fn test_b_units_to_value_to_units_round_trips_at_boundaries() {
    for units in [
        0u64,
        1,
        MILITARY_SUPPLIES_SCALE - 1,
        MILITARY_SUPPLIES_SCALE,
        MILITARY_SUPPLIES_SCALE + 1,
        MILITARY_SUPPLIES_MAX_UNITS - 1,
        MILITARY_SUPPLIES_MAX_UNITS,
    ] {
        let value = military_supplies_units_to_value(units);
        assert!(value.is_finite());
        let round_trip = military_supplies_value_to_units(value)
            .unwrap_or_else(|e| panic!("round trip failed for units={units}: {e:?}"));
        assert_eq!(round_trip, units, "units->value->units mismatch for {units}");
    }
}

#[test]
fn test_b_value_to_units_to_value_is_canonical_for_grid_aligned_values() {
    for value in [0.0, 1.0, 3.7, 0.000001, 100.0, 999_999.999999] {
        let units = military_supplies_value_to_units(value).unwrap();
        let restored = military_supplies_units_to_value(units);
        let restored_units = military_supplies_value_to_units(restored).unwrap();
        assert_eq!(
            units, restored_units,
            "value->units->value->units must be canonical for {value}"
        );
    }
}

#[test]
fn test_b_just_before_and_after_a_quantization_boundary() {
    // 0.0000005が四捨五入でどちらへ丸まっても(0 or 1)、量子化後は往復可能。
    let boundary = 0.0000005;
    let units = military_supplies_value_to_units(boundary).unwrap();
    assert!(units == 0 || units == 1);
    let restored = military_supplies_units_to_value(units);
    assert_eq!(military_supplies_value_to_units(restored).unwrap(), units);
}

// ─────────────────────────────────────────────────────────────────────────
// Test C: 不正値
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_c_rejects_nan_and_infinities_without_panicking() {
    assert_eq!(
        military_supplies_value_to_units(f64::NAN),
        Err(MilitarySuppliesUnitsError::NonFinite)
    );
    assert_eq!(
        military_supplies_value_to_units(f64::INFINITY),
        Err(MilitarySuppliesUnitsError::NonFinite)
    );
    assert_eq!(
        military_supplies_value_to_units(f64::NEG_INFINITY),
        Err(MilitarySuppliesUnitsError::NonFinite)
    );
}

#[test]
fn test_c_rejects_negative_values() {
    assert_eq!(
        military_supplies_value_to_units(-1.0),
        Err(MilitarySuppliesUnitsError::Negative)
    );
    assert_eq!(
        military_supplies_value_to_units(-0.0000001),
        Err(MilitarySuppliesUnitsError::Negative)
    );
}

#[test]
fn test_c_negative_zero_is_accepted_as_zero() {
    // IEEE754上 -0.0 < 0.0 は偽なので、Negativeとしては拒否されずunits=0となる。
    assert_eq!(military_supplies_value_to_units(-0.0), Ok(0));
}

#[test]
fn test_c_rejects_value_beyond_max_and_f64_max() {
    let just_over_max = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS)
        + (2.0 / MILITARY_SUPPLIES_SCALE as f64);
    assert_eq!(
        military_supplies_value_to_units(just_over_max),
        Err(MilitarySuppliesUnitsError::OutOfRange)
    );
    assert_eq!(
        military_supplies_value_to_units(f64::MAX),
        Err(MilitarySuppliesUnitsError::OutOfRange)
    );
}

#[test]
fn test_c_rejects_ambiguous_two_pow_53_neighborhood_values() {
    // P25-002-REV-001が発見した2^53近傍の曖昧な値問題を再導入しないことを確認する。
    // MilitarySuppliesの正規値域(MAX_UNITS/SCALE=10億)を大幅に超えるため、値域外
    // として明確に拒否される(黙って近似成功しない)。
    for exp in [52, 53, 54, 60] {
        let value = 2f64.powi(exp);
        assert_eq!(
            military_supplies_value_to_units(value),
            Err(MilitarySuppliesUnitsError::OutOfRange),
            "2^{exp} must be rejected as out of range, not silently accepted"
        );
    }
}

#[test]
fn test_c_multiplication_overflow_to_infinity_is_rejected() {
    // value自体はfiniteだが、value * SCALEがf64::INFINITYへoverflowするケース。
    let huge = f64::MAX / 2.0;
    assert_eq!(
        military_supplies_value_to_units(huge),
        Err(MilitarySuppliesUnitsError::OutOfRange)
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test D: 生産加算
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_d_zero_produced_is_a_deterministic_no_op() {
    let mut stockpile = stockpile_with(ResourceType::MilitarySupplies, 12.5);
    stockpile.add_military_supplies_produced(0.0).unwrap();
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 12.5);
}

#[test]
fn test_d_integer_and_fractional_accumulation() {
    let mut stockpile = CountryStockpile::new();
    stockpile.add_military_supplies_produced(10.0).unwrap();
    stockpile.add_military_supplies_produced(0.25).unwrap();
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 10.25);
}

#[test]
fn test_d_multiple_additions_accumulate_exactly() {
    let mut stockpile = CountryStockpile::new();
    for _ in 0..10 {
        stockpile.add_military_supplies_produced(1.1).unwrap();
    }
    // 1.1は2進浮動小数点で正確に表現できないが、固定小数点units経由の累積は
    // 10 * 1_100_000 = 11_000_000 units = 11.0 と正確に一致する。
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 11.0);
}

#[test]
fn test_d_reaching_exactly_max_units_succeeds() {
    let mut stockpile = CountryStockpile::new();
    let max_value = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);
    stockpile.add_military_supplies_produced(max_value).unwrap();
    assert_eq!(
        military_supplies_value_to_units(stockpile.get(ResourceType::MilitarySupplies)).unwrap(),
        MILITARY_SUPPLIES_MAX_UNITS
    );
}

#[test]
fn test_d_exceeding_max_units_is_rejected_atomically() {
    let mut stockpile = CountryStockpile::new();
    let max_value = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);
    stockpile.add_military_supplies_produced(max_value).unwrap();

    let err = stockpile
        .add_military_supplies_produced(military_supplies_units_to_value(1))
        .unwrap_err();
    assert_eq!(err, MilitarySuppliesUnitsError::OutOfRange);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), max_value);
}

#[test]
fn test_d_invalid_current_stockpile_is_rejected_without_mutation() {
    let mut stockpile = CountryStockpile::new();
    stockpile
        .amounts
        .insert(ResourceType::MilitarySupplies, f64::INFINITY);
    let err = stockpile.add_military_supplies_produced(1.0).unwrap_err();
    assert_eq!(err, MilitarySuppliesUnitsError::NonFinite);
    assert_eq!(
        stockpile.get(ResourceType::MilitarySupplies),
        f64::INFINITY
    );
}

#[test]
fn test_d_invalid_produced_amount_is_rejected_without_mutation() {
    let mut stockpile = stockpile_with(ResourceType::MilitarySupplies, 5.0);
    let err = stockpile
        .add_military_supplies_produced(f64::NAN)
        .unwrap_err();
    assert_eq!(err, MilitarySuppliesUnitsError::NonFinite);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 5.0);

    let err2 = stockpile
        .add_military_supplies_produced(-1.0)
        .unwrap_err();
    assert_eq!(err2, MilitarySuppliesUnitsError::Negative);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 5.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Test E: 消費
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_e_zero_consumption_is_a_deterministic_no_op() {
    let mut stockpile = stockpile_with(ResourceType::MilitarySupplies, 8.0);
    stockpile.try_consume_military_supplies(0.0).unwrap();
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 8.0);
}

#[test]
fn test_e_partial_and_full_consumption() {
    let mut stockpile = stockpile_with(ResourceType::MilitarySupplies, 50.0);
    stockpile.try_consume_military_supplies(20.0).unwrap();
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 30.0);
    stockpile.try_consume_military_supplies(30.0).unwrap();
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_e_fractional_consumption() {
    let mut stockpile = stockpile_with(ResourceType::MilitarySupplies, 10.0);
    stockpile.try_consume_military_supplies(0.333333).unwrap();
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 9.666667);
}

#[test]
fn test_e_insufficient_stockpile_rejects_the_entire_request_atomically() {
    let mut stockpile = stockpile_with(ResourceType::MilitarySupplies, 9.99);
    let err = stockpile
        .try_consume_military_supplies(10.0)
        .unwrap_err();
    assert_eq!(err, MilitarySuppliesUnitsError::InsufficientStockpile);
    // 「可能な分だけ部分消費」してはいけない: 備蓄は完全に不変。
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 9.99);
}

#[test]
fn test_e_invalid_current_stockpile_is_rejected_without_mutation() {
    let mut stockpile = CountryStockpile::new();
    stockpile
        .amounts
        .insert(ResourceType::MilitarySupplies, -5.0);
    let err = stockpile.try_consume_military_supplies(1.0).unwrap_err();
    assert_eq!(err, MilitarySuppliesUnitsError::Negative);
}

#[test]
fn test_e_invalid_requested_amount_is_rejected_without_mutation() {
    let mut stockpile = stockpile_with(ResourceType::MilitarySupplies, 20.0);
    let err = stockpile
        .try_consume_military_supplies(f64::INFINITY)
        .unwrap_err();
    assert_eq!(err, MilitarySuppliesUnitsError::NonFinite);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 20.0);

    let err2 = stockpile.try_consume_military_supplies(-1.0).unwrap_err();
    assert_eq!(err2, MilitarySuppliesUnitsError::Negative);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 20.0);
}

#[test]
fn test_e_underflow_beyond_stockpile_is_rejected_not_saturated() {
    let mut stockpile = stockpile_with(ResourceType::MilitarySupplies, 0.0);
    let err = stockpile
        .try_consume_military_supplies(0.000001)
        .unwrap_err();
    assert_eq!(err, MilitarySuppliesUnitsError::InsufficientStockpile);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Test F: 保存則
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_f_produced_units_increase_matches_world_stockpile_increase() {
    let mut stockpile = CountryStockpile::new();
    let before = military_supplies_value_to_units(stockpile.get(ResourceType::MilitarySupplies))
        .unwrap();
    stockpile.add_military_supplies_produced(42.5).unwrap();
    let after = military_supplies_value_to_units(stockpile.get(ResourceType::MilitarySupplies))
        .unwrap();
    let produced_units = military_supplies_value_to_units(42.5).unwrap();
    assert_eq!(after - before, produced_units);
}

#[test]
fn test_f_consumed_units_decrease_matches_world_stockpile_decrease() {
    let mut stockpile = stockpile_with(ResourceType::MilitarySupplies, 100.0);
    let before = military_supplies_value_to_units(stockpile.get(ResourceType::MilitarySupplies))
        .unwrap();
    stockpile.try_consume_military_supplies(37.5).unwrap();
    let after = military_supplies_value_to_units(stockpile.get(ResourceType::MilitarySupplies))
        .unwrap();
    let consumed_units = military_supplies_value_to_units(37.5).unwrap();
    assert_eq!(before - after, consumed_units);
}

#[test]
fn test_f_produce_then_consume_matches_expected_value() {
    let mut stockpile = CountryStockpile::new();
    stockpile.add_military_supplies_produced(100.0).unwrap();
    stockpile.add_military_supplies_produced(50.0).unwrap();
    stockpile.try_consume_military_supplies(30.0).unwrap();
    stockpile.try_consume_military_supplies(70.0).unwrap();
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 50.0);
}

#[test]
fn test_f_repeated_produce_consume_cycles_do_not_drift() {
    let mut stockpile = CountryStockpile::new();
    // 100,000回のproduce/consumeサイクル(小規模・安全な値)を繰り返しても、
    // 固定小数点units経由なのでf64丸め誤差が蓄積しないことを確認する。
    for _ in 0..100_000 {
        stockpile.add_military_supplies_produced(1.5).unwrap();
        stockpile.try_consume_military_supplies(0.5).unwrap();
    }
    // 差し引き1サイクルあたり+1.0、100,000サイクルで+100,000.0。
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 100_000.0);
}

#[test]
fn test_f_multiple_countries_world_total_is_conserved() {
    let mut countries: Vec<CountryStockpile> = (0..5).map(|_| CountryStockpile::new()).collect();

    let world_total = |countries: &[CountryStockpile]| -> f64 {
        countries
            .iter()
            .map(|s| s.get(ResourceType::MilitarySupplies))
            .sum()
    };

    assert_eq!(world_total(&countries), 0.0);

    countries[0].add_military_supplies_produced(30.0).unwrap();
    countries[1].add_military_supplies_produced(50.0).unwrap();
    countries[2].add_military_supplies_produced(20.0).unwrap();
    assert_eq!(world_total(&countries), 100.0);

    countries[1].try_consume_military_supplies(15.0).unwrap();
    assert_eq!(world_total(&countries), 85.0);

    // 1国の消費失敗が世界合計を変えないこと。
    let err = countries[3].try_consume_military_supplies(1.0).unwrap_err();
    assert_eq!(err, MilitarySuppliesUnitsError::InsufficientStockpile);
    assert_eq!(world_total(&countries), 85.0);
}

#[test]
fn test_f_operating_on_one_country_never_affects_another() {
    let mut country_a = CountryStockpile::new();
    let mut country_b = stockpile_with(ResourceType::MilitarySupplies, 100.0);

    country_a.add_military_supplies_produced(60.0).unwrap();
    assert_eq!(country_a.get(ResourceType::MilitarySupplies), 60.0);
    assert_eq!(
        country_b.get(ResourceType::MilitarySupplies),
        100.0,
        "country A's production must never affect country B's stockpile"
    );

    country_b.try_consume_military_supplies(40.0).unwrap();
    assert_eq!(country_b.get(ResourceType::MilitarySupplies), 60.0);
    assert_eq!(
        country_a.get(ResourceType::MilitarySupplies),
        60.0,
        "country B's consumption must never affect country A's stockpile"
    );
}

#[test]
fn test_f_other_resource_types_are_never_touched_by_military_supplies_operations() {
    let mut stockpile = CountryStockpile::new();
    for res in ResourceType::ALL {
        if res != ResourceType::MilitarySupplies {
            stockpile.set(res, 111.0);
        }
    }
    stockpile.add_military_supplies_produced(5.0).unwrap();
    stockpile.try_consume_military_supplies(2.0).unwrap();
    let _ = stockpile.try_consume_military_supplies(1_000_000.0); // 失敗するはず

    for res in ResourceType::ALL {
        if res != ResourceType::MilitarySupplies {
            assert_eq!(
                stockpile.get(res),
                111.0,
                "{res:?} must be unaffected by MilitarySupplies operations"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Test G: Save V1互換性
// ─────────────────────────────────────────────────────────────────────────

/// P26-002より前(このタスク以前)に保存された旧V1相当のCountryStockpile RON
/// (手書きで実際の旧セーブ出力形式を再現、`tests/p21_009_magic_crystal_chain_test.rs`
/// の`old_save_ron_without_new_keys_loads_with_safe_defaults`と同じ手法)。
#[test]
fn test_g_old_v1_stockpile_ron_without_military_supplies_key_loads_successfully() {
    let old_stockpile_ron = "(amounts:{Iron:10.0,Coal:5.0,MilitaryEquipment:42.0})";
    let restored: CountryStockpile = ron::from_str(old_stockpile_ron)
        .expect("old-format (pre-P26-002) stockpile RON must still deserialize");

    assert_eq!(restored.get(ResourceType::MilitarySupplies), 0.0);
    assert_eq!(restored.get(ResourceType::Iron), 10.0);
    assert_eq!(restored.get(ResourceType::Coal), 5.0);
    assert_eq!(restored.get(ResourceType::MilitaryEquipment), 42.0);
}

#[test]
fn test_g_stockpile_with_military_supplies_round_trips_through_ron() {
    let mut stockpile = CountryStockpile::new();
    stockpile.set(ResourceType::MilitarySupplies, 123.456789);
    stockpile.set(ResourceType::Iron, 77.0);

    let ron_str = ron::to_string(&stockpile).expect("CountryStockpile must serialize to RON");
    let restored: CountryStockpile =
        ron::from_str(&ron_str).expect("CountryStockpile RON must deserialize back");

    // canonical unitsの一致(生のf64ビット一致ではなく、量子化後の意味的な一致)を検証する。
    assert_eq!(
        military_supplies_value_to_units(restored.get(ResourceType::MilitarySupplies)).unwrap(),
        military_supplies_value_to_units(123.456789).unwrap()
    );
    assert_eq!(restored.get(ResourceType::Iron), 77.0);
}

#[test]
fn test_g_double_round_trip_through_ron_does_not_change_units() {
    let mut stockpile = CountryStockpile::new();
    stockpile.set(ResourceType::MilitarySupplies, 999.999999);

    let ron1 = ron::to_string(&stockpile).unwrap();
    let restored1: CountryStockpile = ron::from_str(&ron1).unwrap();
    let ron2 = ron::to_string(&restored1).unwrap();
    let restored2: CountryStockpile = ron::from_str(&ron2).unwrap();

    let units1 =
        military_supplies_value_to_units(restored1.get(ResourceType::MilitarySupplies)).unwrap();
    let units2 =
        military_supplies_value_to_units(restored2.get(ResourceType::MilitarySupplies)).unwrap();
    assert_eq!(units1, units2);
}

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p26_002_e2e_{label}_{}_{nanos}_{n}",
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

fn set_military_supplies_stockpile(app: &mut App, country_id: CountryId, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(country_id)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitarySupplies, value);
}

fn military_supplies_stockpile(app: &App, country_id: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitarySupplies)
}

/// Test G(実App): save→load往復で、量子化済みのMilitarySupplies備蓄値が変化しないこと。
/// 他ResourceTypeも維持されること。
#[test]
fn test_g_app_save_load_round_trip_preserves_quantized_stockpile_and_other_resources() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_g_app");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    set_military_supplies_stockpile(&mut app, player, 456.789123);
    let other_before = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .stockpile
        .get(ResourceType::Food);
    let before = military_supplies_stockpile(&app, player);
    let world_total_before: f64 = app
        .world()
        .resource::<CountryRegistry>()
        .countries
        .iter()
        .map(|c| c.stockpile.get(ResourceType::MilitarySupplies))
        .sum();

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    let after = military_supplies_stockpile(&app, player);
    assert_eq!(
        before, after,
        "MilitarySupplies stockpile must round-trip unchanged through save/load"
    );
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .stockpile
            .get(ResourceType::Food),
        other_before,
        "other ResourceTypes must be preserved across save/load"
    );
    let world_total_after: f64 = app
        .world()
        .resource::<CountryRegistry>()
        .countries
        .iter()
        .map(|c| c.stockpile.get(ResourceType::MilitarySupplies))
        .sum();
    assert_eq!(
        world_total_before, world_total_after,
        "save/load alone must not change the world total"
    );

    // 2回連続save/loadでも変化しない。
    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();
    assert_eq!(military_supplies_stockpile(&app, player), after);
}

// ─────────────────────────────────────────────────────────────────────────
// Test H: New Game／Loaded Game
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_h_new_game_and_loaded_game_agree_on_identical_stockpile_value() {
    let player = CountryId(0);

    let temp_dir_continuous = TempTestDir::new("test_h_continuous");
    let mut continuous = setup_app_in_playing(player, temp_dir_continuous.save_file_config());
    set_military_supplies_stockpile(&mut continuous, player, 77.0);

    let temp_dir_branch = TempTestDir::new("test_h_branch");
    let mut branch = setup_app_in_playing(player, temp_dir_branch.save_file_config());
    set_military_supplies_stockpile(&mut branch, player, 77.0);
    send_save_request(&mut branch);
    branch.update();
    send_load_request(&mut branch);
    branch.update();
    branch.update();

    assert_eq!(
        military_supplies_stockpile(&continuous, player),
        military_supplies_stockpile(&branch, player),
        "New Game and Loaded Game paths must agree on the same MilitarySupplies stockpile"
    );
}

/// P26-002時点(本テストファイル作成当時)ではNew GameのMilitarySuppliesは0から
/// 始まっていたが、P26-004-PRE-005が「初回月次生産までのブリッジ備蓄」を各国家へ
/// 一度限り付与するようになったため、New Gameの現行データ(満編制Infantry1個師団の
/// 日次需要1.0、初回生産までの実暦30日+予備1日)では全6か国とも厳密に31.0から
/// 始まる(`verification_logs/phase-26/p26-004-pre-005/`)。本テストは意図的に
/// この新しい正規挙動へ更新した(P26-002自身の会計基盤・往復変換保証には一切
/// 変更がないため、他のTest A〜Gは無変更のまま)。
#[test]
fn test_h_new_game_starts_at_the_initial_bridge_grant_for_all_countries() {
    let temp_dir = TempTestDir::new("test_h_new_game_zero");
    let app = setup_app_in_playing(CountryId(0), temp_dir.save_file_config());

    for country in app.world().resource::<CountryRegistry>().countries.iter() {
        assert_eq!(
            country.stockpile.get(ResourceType::MilitarySupplies),
            31.0,
            "New Game must grant exactly the P26-004-PRE-005 initial bridge stockpile \
             (31.0 for the current 1-division/full-manpower New Game data) to country {:?}",
            country.id
        );
        assert!(
            country.military_supplies_initial_grant_applied,
            "the one-time grant marker must be true after New Game for country {:?}",
            country.id
        );
    }
}

/// 旧V1相当ロード(MilitarySuppliesキー・grantマーカーへの言及がないセーブ)は、
/// P26-004-PRE-005の意味的migrationによりブリッジ備蓄(31.0)まで一度限りtop-upされる
/// (無条件加算ではない、`verification_logs/phase-26/p26-004-pre-005/`参照)。
#[test]
fn test_h_loaded_old_v1_save_is_topped_up_to_the_initial_bridge_grant() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_h_old_v1_load");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    // 旧V1相当を模すため、MilitarySuppliesキー・grantマーカーへ一切触れずに他資源だけを
    // 変更して保存する(marker自体はNew Game時点で既にtrueのため、このテストは
    // 「マーカー付き・備蓄31.0」のセーブをそのままsave/loadする、意味的には
    // 「マーカーtrueなら再付与しない」不変条件の確認を兼ねる)。
    {
        let mut registry = app.world_mut().resource_mut::<CountryRegistry>();
        registry.get_mut(player).unwrap().stockpile.set(ResourceType::Iron, 999.0);
    }
    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    assert_eq!(military_supplies_stockpile(&app, player), 31.0);
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .stockpile
            .get(ResourceType::Iron),
        999.0
    );
}

/// P26-002時点では、本番App更新(日次・月次ティック含む)だけではMilitarySuppliesが
/// 増減しないことを確認していた(H・I両方の観点を兼ねる)。P26-004-PRE-005導入後は
/// New Game時点で全6か国が31.0(合計186.0)のブリッジ備蓄を持つため基準値を更新した。
///
/// P26-004導入後は、日次ティック自体が(月次生産とは独立に)Division日次補給を実際に
/// 消費するようになったため、「短いティックだけでは増減しない」という主旨はもはや
/// 成り立たない――day 2〜10の9回の日次ティックは、New Gameが各国に1個ずつ配置する
/// 満編制Infantry1個師団の日次需要(1.0/日)を、6か国全てが備蓄31.0から確実に賄える
/// (常に充足)ため、世界合計は`186.0 - 6か国 × 9日 × 1.0/日 = 132.0`だけ減少する。
/// 実際の月次生産(P26-003で接続済み)は、この短い区間(建設が完了するには短すぎる)
/// ではまだ発火しないため、"生産"側が変化しないという本テストの主旨自体は
/// P26-004後も変わらない――日次"消費"の分だけ、期待値を明示的に更新した。
#[test]
fn test_h_production_app_ticks_alone_do_not_change_military_supplies() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_h_ticks");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let world_total_before: f64 = app
        .world()
        .resource::<CountryRegistry>()
        .countries
        .iter()
        .map(|c| c.stockpile.get(ResourceType::MilitarySupplies))
        .sum();
    assert_eq!(world_total_before, 31.0 * 6.0, "6 countries x the P26-004-PRE-005 bridge grant");

    // 数日分の日次ティック + 1回の月次ティック(既存の経済生産Systemが実際に走る経路)。
    for day in 2..=10u8 {
        let mut state: SystemState<MessageWriter<DayChangedMessage>> =
            SystemState::new(app.world_mut());
        state
            .get_mut(app.world_mut())
            .unwrap()
            .write(DayChangedMessage {
                year: 1800,
                month: 1,
                day,
            });
        state.apply(app.world_mut());
        app.update();
    }
    {
        let mut state: SystemState<MessageWriter<MonthChangedMessage>> =
            SystemState::new(app.world_mut());
        state
            .get_mut(app.world_mut())
            .unwrap()
            .write(MonthChangedMessage {
                year: 1800,
                month: 2,
            });
        state.apply(app.world_mut());
        app.update();
    }

    let world_total_after: f64 = app
        .world()
        .resource::<CountryRegistry>()
        .countries
        .iter()
        .map(|c| c.stockpile.get(ResourceType::MilitarySupplies))
        .sum();
    // P26-004: day 2..=10 (9 ticks) x 6 countries x 1.0/day (each country's single
    // full-strength New Game Infantry division) daily consumption; month-2 production
    // is still 0 in this short window (construction has not had time to complete).
    let expected_after = world_total_before - (9.0 * 6.0 * 1.0);
    assert_eq!(
        world_total_after, expected_after,
        "MilitarySupplies must decrease by exactly the known daily consumption (P26-004) and \
         by nothing else (no building has had time to complete construction yet, so production \
         stays 0)"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test I: 非接続性
// ─────────────────────────────────────────────────────────────────────────

/// 数日分の日次ティックを経ても、既存のDivision.supply_ratio(P26-001設計監査§10・23
/// で確認済みの死んだ書き込み先)が初期値1.0のまま変化しないことを確認する。
/// もし死んだ`military::supply::process_supply`が誤って本番Systemへ接続されていれば、
/// `supply_ratio`は州の`logistics_ratio`へ書き換えられ、このテストは失敗する。
#[test]
fn test_i_division_supply_ratio_remains_untouched_by_daily_ticks() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_i_supply_ratio");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let supply_ratios_before: Vec<f32> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .map(|d| d.supply_ratio)
        .collect();
    assert!(
        !supply_ratios_before.is_empty(),
        "New Game must have at least one Division to observe"
    );
    for ratio in &supply_ratios_before {
        assert_eq!(*ratio, 1.0, "initial supply_ratio must be the existing default 1.0");
    }

    for day in 2..=6u8 {
        let mut state: SystemState<MessageWriter<DayChangedMessage>> =
            SystemState::new(app.world_mut());
        state
            .get_mut(app.world_mut())
            .unwrap()
            .write(DayChangedMessage {
                year: 1800,
                month: 1,
                day,
            });
        state.apply(app.world_mut());
        app.update();
    }

    let supply_ratios_after: Vec<f32> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .map(|d| d.supply_ratio)
        .collect();
    for ratio in &supply_ratios_after {
        assert_eq!(
            *ratio, 1.0,
            "supply_ratio must remain the dead default; process_supply must stay unregistered"
        );
    }
}

/// MilitaryEquipment・manpower会計が、MilitarySupplies APIの追加によって一切
/// 影響を受けないことを確認する(既存Phase 25/23会計の保護、実App経由)。
#[test]
fn test_i_equipment_and_manpower_accounting_is_unaffected_by_new_resource_type() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_i_equipment_manpower");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let equipment_before: f64 = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitaryEquipment);
    let manpower_before: Vec<(u64, u64)> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == player)
        .map(|d| (d.manpower, d.max_manpower))
        .collect();

    // 本タスクで追加したMilitarySupplies APIをここで直接呼び出しても、既存のequipment
    // stockpileには一切影響しないことを確認する。
    {
        let mut registry = app.world_mut().resource_mut::<CountryRegistry>();
        let country = registry.get_mut(player).unwrap();
        country
            .stockpile
            .add_military_supplies_produced(100.0)
            .unwrap();
        country
            .stockpile
            .try_consume_military_supplies(40.0)
            .unwrap();
    }

    let equipment_after: f64 = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitaryEquipment);
    let manpower_after: Vec<(u64, u64)> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == player)
        .map(|d| (d.manpower, d.max_manpower))
        .collect();

    assert_eq!(equipment_before, equipment_after);
    assert_eq!(manpower_before, manpower_after);
}
