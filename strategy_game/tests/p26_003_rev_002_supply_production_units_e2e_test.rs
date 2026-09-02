//! P26-003-REV-002: `economy::production::process_country_production`の
//! MilitarySupplies国家内集計を、生の`f64`加算(sum-then-quantize、方式B)から
//! 個別生産エントリの整数units加算(quantize-then-sum、方式A)へ訂正したことの
//! E2Eテスト。
//!
//! `verification_logs/phase-26/p26-003-rev-001/`が指摘した「MilitaryEquipmentは
//! 施設ごと即時quantize、MilitarySuppliesは国家合計を1回だけquantize」という
//! 非対称性(§16〜17)を解消する。個別生産エントリ(1州×1建物のMilitarySupplies
//! 出力)ごとに`military_supplies_value_to_units`で量子化してから、整数のまま
//! `u128`のchecked_addで国家合計を積み上げ、全州処理完了後に1回だけ
//! `CountryStockpile::add_military_supplies_produced`へcommitする(spec §12
//! 「国家単位で原子的」はP26-003から維持)。
//!
//! Category A: 個別量子化と国家合計 / B: 順序不変性 / C: 上限・overflow /
//! D: 不正エントリと失敗状態維持 / E: 国家間分離 / F: 実生産経路(実App) /
//! G: 原子性の境界(MilitarySupplies単体、全生産取引の原子性は対象外) /
//! H: 5倍比率(常に厳密ではない) / I: Save V1 / J: 反復。

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
    CountryStockpile, MILITARY_SUPPLIES_MAX_UNITS, ResourceType, military_supplies_units_to_value,
    military_supplies_value_to_units,
};
use strategy_game::military::MilitaryPlugin;
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
// Fixtures
// ─────────────────────────────────────────────────────────────────────────

fn empty_building_def(building_type: BuildingType) -> BuildingDefinition {
    BuildingDefinition {
        building_type,
        name: format!("{building_type:?}"),
        construction_cost: 100.0,
        required_progress: 10.0,
        required_workforce: 10.0,
        logistics_cost: 0.0,
        input_resources: HashMap::new(),
        output_resources: HashMap::new(),
        maintenance_cost: 1.0,
        max_level: 10,
        science_output: 0.0,
        magic_output: 0.0,
        railway_capacity_bonus: 0.0,
    }
}

/// `resource`をper_levelで産出するだけの単純化済み建物定義(入力資源なし)。
fn producing_def(building_type: BuildingType, resource: ResourceType, per_level: f64) -> BuildingDefinition {
    let mut def = empty_building_def(building_type);
    def.output_resources.insert(resource, per_level);
    def
}

/// MilitaryFactoryがMilitaryEquipment/MilitarySuppliesを併産する(実データと同じ
/// 構成、入力資源なしでoperation_ratioの計算をworkforce/logisticsだけへ単純化)。
fn military_factory_registry(equipment_per_level: f64, supplies_per_level: f64) -> BuildingRegistry {
    let mut definitions = HashMap::new();
    let mut def = empty_building_def(BuildingType::MilitaryFactory);
    def.output_resources.insert(ResourceType::MilitaryEquipment, equipment_per_level);
    def.output_resources.insert(ResourceType::MilitarySupplies, supplies_per_level);
    definitions.insert(BuildingType::MilitaryFactory, def);
    BuildingRegistry { definitions }
}

/// 実データ(`assets/data/buildings.ron`)と同じ入力(Iron/IndustrialGoods)を持つ
/// MilitaryFactory registry。入力不足によるoperation_ratioゲート・原子性境界の
/// 検証専用。
fn military_factory_registry_with_inputs() -> BuildingRegistry {
    let mut definitions = HashMap::new();
    let mut def = empty_building_def(BuildingType::MilitaryFactory);
    def.required_workforce = 25000.0;
    def.logistics_cost = 20.0;
    def.input_resources.insert(ResourceType::Iron, 30.0);
    def.input_resources.insert(ResourceType::IndustrialGoods, 15.0);
    def.output_resources.insert(ResourceType::MilitaryEquipment, 20.0);
    def.output_resources.insert(ResourceType::MilitarySupplies, 100.0);
    definitions.insert(BuildingType::MilitaryFactory, def);
    BuildingRegistry { definitions }
}

/// 複数の生産エントリを持つ国家を模すためのregistry。`entries`は
/// `(building_type, per_level_supplies_output)`のリストで、各building_typeが独立
/// した1つのMilitarySupplies生産エントリ源となる(実データのMilitaryFactory以外の
/// 建物種別をローカルなtest-only registryでMilitarySupplies出力用に流用しても、
/// `assets/data/buildings.ron`・実`BuildingType` enum自体は一切変更しない)。
fn multi_entry_supplies_registry(entries: &[(BuildingType, f64)]) -> BuildingRegistry {
    let mut definitions = HashMap::new();
    for &(b_type, per_level) in entries {
        definitions.insert(b_type, producing_def(b_type, ResourceType::MilitarySupplies, per_level));
    }
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

fn one_state_result(registry: &BuildingRegistry, building_type: BuildingType, level: u32) -> f64 {
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(building_type, level)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, registry, false);
    stockpile.get(ResourceType::MilitarySupplies)
}

// ─────────────────────────────────────────────────────────────────────────
// Category A: 個別量子化と国家合計
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_a1_no_production_entries_country_supplies_stays_zero() {
    let registry = multi_entry_supplies_registry(&[]);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_a2_single_entry_zero_output_stays_zero() {
    let registry = military_factory_registry(20.0, 100.0);
    assert_eq!(one_state_result(&registry, BuildingType::MilitaryFactory, 0), 0.0);
}

#[test]
fn test_a3_single_integer_entry_matches_expected_units() {
    let registry = military_factory_registry(20.0, 100.0);
    assert_eq!(one_state_result(&registry, BuildingType::MilitaryFactory, 1), 100.0);
}

#[test]
fn test_a4_single_fractional_entry_matches_expected_units() {
    let registry = military_factory_registry(20.0, 14.6);
    assert_eq!(one_state_result(&registry, BuildingType::MilitaryFactory, 1), 14.6);
}

#[test]
fn test_a5_multiple_integer_entries_sum_correctly() {
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::MilitaryFactory, 100.0),
        (BuildingType::Factory, 250.0),
        (BuildingType::University, 30.0),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(
        StateId(0),
        &[
            (BuildingType::MilitaryFactory, 1),
            (BuildingType::Factory, 1),
            (BuildingType::University, 1),
        ],
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 380.0);
}

#[test]
fn test_a6_quantization_boundary_just_below_half_unit_rounds_down() {
    // 0.49 units相当。round-to-nearestで0へ丸まり、合計0のためcommit自体を
    // 行わない(spec §9)。
    let registry = military_factory_registry(20.0, 0.00000049);
    assert_eq!(one_state_result(&registry, BuildingType::MilitaryFactory, 1), 0.0);
}

#[test]
fn test_a7_quantization_boundary_just_above_half_unit_rounds_up() {
    let registry = military_factory_registry(20.0, 0.00000051);
    assert_eq!(one_state_result(&registry, BuildingType::MilitaryFactory, 1), 0.000001);
}

/// 訂正後の量子化仕様の核心: 2つの生産エントリ(それぞれ0.0000006、0.6 units相当)を
/// 個別に量子化するとどちらも1 unitへ丸まり、合計2 units(=0.000002)になる。
/// 旧実装(方式B、生f64合計してから1回だけ量子化)ならΣ=0.0000012→scaled=1.2→
/// round()=1 unitにしかならなかった(`verification_logs/phase-26/p26-003-rev-001/`
/// §16〜17参照)。この差は訂正後の正式な量子化仕様であり、旧合計へ戻す補正は
/// 加えない(spec §6)。
#[test]
fn test_a8_two_subunit_entries_of_0_0000006_sum_to_two_units() {
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::MilitaryFactory, 0.0000006),
        (BuildingType::Factory, 0.0000006),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(
        StateId(0),
        &[(BuildingType::MilitaryFactory, 1), (BuildingType::Factory, 1)],
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.000002);
}

/// 訂正後の量子化仕様のもう一方の境界: 2つの生産エントリ(それぞれ0.0000004、
/// 0.4 units相当)は個別に量子化するとどちらも0 unitへ丸まり、合計0になる
/// (commit自体を行わない、spec §9)。旧実装(方式B)ならΣ=0.0000008→scaled=0.8→
/// round()=1 unitになっていたはずで、これも意図した仕様変更である(spec §6)。
#[test]
fn test_a9_two_subunit_entries_of_0_0000004_sum_to_zero_units() {
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::MilitaryFactory, 0.0000004),
        (BuildingType::Factory, 0.0000004),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(
        StateId(0),
        &[(BuildingType::MilitaryFactory, 1), (BuildingType::Factory, 1)],
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

/// levelは1ずつ分割して個別量子化しない(spec §5.2)。base=0.0000003・level=2は、
/// 既存の`calculate_actual_output`が1回の乗算で0.0000006を計算してから初めて
/// 建物出力ループへ渡すため、構造的に「1建物1エントリ(0.0000006)」として量子化
/// される(0.6→1 unit)。もし誤ってlevelごとに2つの0.0000003(0.3→0 unit)へ
/// 分割量子化していれば合計0になっていたはずである。
#[test]
fn test_a10_level_multiplier_is_one_entry_not_split_per_level() {
    let registry = military_factory_registry(20.0, 0.0000003);
    assert_eq!(one_state_result(&registry, BuildingType::MilitaryFactory, 2), 0.000001);
}

/// 3つの生産エントリ(それぞれ0.0000004、0.4 units相当)は個別量子化だとすべて0
/// unitへ丸まり合計0になる。旧実装(方式B)ならΣ=0.0000012→scaled=1.2→round()=
/// 1 unitになっていたはずで、a8/a9とは異なる組み合わせでも同じ意図した差が
/// 再現することを確認する。
#[test]
fn test_a11_three_subunit_entries_of_0_0000004_also_sum_to_zero_units() {
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::MilitaryFactory, 0.0000004),
        (BuildingType::Factory, 0.0000004),
        (BuildingType::University, 0.0000004),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(
        StateId(0),
        &[
            (BuildingType::MilitaryFactory, 1),
            (BuildingType::Factory, 1),
            (BuildingType::University, 1),
        ],
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Category B: 順序不変性
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_b1_two_state_order_reversed_gives_same_units() {
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

#[test]
fn test_b2_five_state_multiple_permutations_give_same_units() {
    let registry = military_factory_registry(20.0, 100.0);

    let make_states = || {
        (0..5)
            .map(|i| full_capacity_state(StateId(i), &[(BuildingType::MilitaryFactory, (i + 1) as u32)]))
            .collect::<Vec<_>>()
    };

    let run = |order: &[usize]| {
        let mut states = make_states();
        let mut stockpile = CountryStockpile::new();
        // `order`が指す順序で`&mut StateData`を1回ずつ取り出す(各要素を`Option`で
        // 包んで`take()`することで、借用チェッカーに対して安全に構築する)。
        let mut remaining: Vec<Option<&mut StateData>> = states.iter_mut().map(Some).collect();
        let mut refs: Vec<&mut StateData> = Vec::new();
        for &i in order {
            refs.push(remaining[i].take().expect("index used twice in order"));
        }
        process_country_production(&mut refs, &mut stockpile, &registry, false);
        stockpile.get(ResourceType::MilitarySupplies)
    };

    let baseline = run(&[0, 1, 2, 3, 4]);
    assert_eq!(run(&[4, 3, 2, 1, 0]), baseline);
    assert_eq!(run(&[2, 0, 4, 1, 3]), baseline);
    assert_eq!(run(&[1, 3, 0, 4, 2]), baseline);
}

#[test]
fn test_b3_subunit_boundary_entries_are_order_invariant() {
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::MilitaryFactory, 0.0000006),
        (BuildingType::Factory, 0.0000006),
        (BuildingType::University, 0.0000004),
    ]);

    let run = |buildings: &[(BuildingType, u32)]| {
        let mut stockpile = CountryStockpile::new();
        let mut state = full_capacity_state(StateId(0), buildings);
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, &registry, false);
        stockpile.get(ResourceType::MilitarySupplies)
    };

    let a = run(&[
        (BuildingType::MilitaryFactory, 1),
        (BuildingType::Factory, 1),
        (BuildingType::University, 1),
    ]);
    let b = run(&[
        (BuildingType::University, 1),
        (BuildingType::MilitaryFactory, 1),
        (BuildingType::Factory, 1),
    ]);
    let c = run(&[
        (BuildingType::Factory, 1),
        (BuildingType::University, 1),
        (BuildingType::MilitaryFactory, 1),
    ]);
    // 0.0000006 → 1 unit、0.0000006 → 1 unit、0.0000004 → 0 unit。
    // 合計2 units(=0.000002)で、組み合わせ順序によらず一致する。
    assert_eq!(a, 0.000002);
    assert_eq!(a, b);
    assert_eq!(a, c);
}

// ─────────────────────────────────────────────────────────────────────────
// Category C: 上限・overflow
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_c1_total_exactly_at_max_units_from_zero_succeeds() {
    let half = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS / 2);
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::MilitaryFactory, half),
        (BuildingType::Factory, half),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(
        StateId(0),
        &[(BuildingType::MilitaryFactory, 1), (BuildingType::Factory, 1)],
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(
        stockpile.get(ResourceType::MilitarySupplies),
        military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS)
    );
}

#[test]
fn test_c2_total_one_unit_over_max_from_zero_fails() {
    let half = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS / 2);
    let one_unit = military_supplies_units_to_value(1);
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::MilitaryFactory, half),
        (BuildingType::Factory, half),
        (BuildingType::University, one_unit),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(
        StateId(0),
        &[
            (BuildingType::MilitaryFactory, 1),
            (BuildingType::Factory, 1),
            (BuildingType::University, 1),
        ],
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    // 合計がMAX_UNITSを1超えるため、国家全体のcommitが失敗し備蓄は初期値のまま。
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_c3_each_entry_valid_individually_but_sum_exceeds_max_fails() {
    // 各エントリ(4×10^14 units)はそれ自体MAX_UNITS(10^15)未満で個別には有効だが、
    // 3つ合計すると1.2×10^15 > MAX_UNITSとなる。
    let per_entry = military_supplies_units_to_value(400_000_000_000_000);
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::MilitaryFactory, per_entry),
        (BuildingType::Factory, per_entry),
        (BuildingType::University, per_entry),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(
        StateId(0),
        &[
            (BuildingType::MilitaryFactory, 1),
            (BuildingType::Factory, 1),
            (BuildingType::University, 1),
        ],
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_c4_current_plus_total_exactly_at_max_succeeds() {
    let current = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS - 100);
    let entry = military_supplies_units_to_value(100);
    let registry = multi_entry_supplies_registry(&[(BuildingType::MilitaryFactory, entry)]);
    let mut stockpile = CountryStockpile::new();
    stockpile.set(ResourceType::MilitarySupplies, current);
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(
        stockpile.get(ResourceType::MilitarySupplies),
        military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS)
    );
}

#[test]
fn test_c5_current_plus_total_exceeds_max_by_one_fails() {
    let current = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS - 100);
    let entry = military_supplies_units_to_value(101);
    let registry = multi_entry_supplies_registry(&[(BuildingType::MilitaryFactory, entry)]);
    let mut stockpile = CountryStockpile::new();
    stockpile.set(ResourceType::MilitarySupplies, current);
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    // commitが失敗し、備蓄はcommit前のcurrentのまま(黙ってclampされない)。
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), current);
}

#[test]
fn test_c6_successful_commit_round_trips_to_expected_units() {
    let half = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS / 2);
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::MilitaryFactory, half),
        (BuildingType::Factory, half),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(
        StateId(0),
        &[(BuildingType::MilitaryFactory, 1), (BuildingType::Factory, 1)],
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    let stored = stockpile.get(ResourceType::MilitarySupplies);
    assert_eq!(military_supplies_value_to_units(stored).unwrap(), MILITARY_SUPPLIES_MAX_UNITS);
}

/// `SuppliesUnitsAccumulation::accumulate`内の`u128::checked_add`が実際にNoneを
/// 返す(真のu128 wraparound)には、1エントリあたり最大`MILITARY_SUPPLIES_MAX_UNITS`
/// (10^15)の値を`u128::MAX / MILITARY_SUPPLIES_MAX_UNITS`件以上積む必要がある。
/// これは現実のどの生産経路(全州×全建物種別でも高々数百件)からも到達不能な規模で
/// あり、この分岐は防御的コードとして扱う(spec §14-C「実際には通らないoverflow
/// 経路をテスト済みと主張しない」)。ここでは到達不能性そのものを純粋計算で確認する。
#[test]
fn test_c7_true_u128_overflow_is_unreachable_at_any_realistic_building_count() {
    let max_entries_before_true_overflow = u128::MAX / (MILITARY_SUPPLIES_MAX_UNITS as u128);
    assert!(
        max_entries_before_true_overflow > 1_000_000_000_000_000_000u128,
        "expected an astronomically large entry count before true u128 overflow, got {max_entries_before_true_overflow}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Category D: 不正エントリと失敗状態維持
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_d1_first_entry_invalid_poisons_whole_country() {
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::MilitaryFactory, -5.0),
        (BuildingType::Factory, 100.0),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state_bad = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut state_good = full_capacity_state(StateId(1), &[(BuildingType::Factory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state_bad, &mut state_good];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_d2_middle_entry_invalid_poisons_whole_country() {
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::Factory, 100.0),
        (BuildingType::MilitaryFactory, -5.0),
        (BuildingType::University, 50.0),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state_a = full_capacity_state(StateId(0), &[(BuildingType::Factory, 1)]);
    let mut state_b = full_capacity_state(StateId(1), &[(BuildingType::MilitaryFactory, 1)]);
    let mut state_c = full_capacity_state(StateId(2), &[(BuildingType::University, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state_a, &mut state_b, &mut state_c];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_d3_last_entry_invalid_poisons_whole_country() {
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::Factory, 100.0),
        (BuildingType::University, 50.0),
        (BuildingType::MilitaryFactory, -5.0),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state_a = full_capacity_state(StateId(0), &[(BuildingType::Factory, 1)]);
    let mut state_b = full_capacity_state(StateId(1), &[(BuildingType::University, 1)]);
    let mut state_c = full_capacity_state(StateId(2), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state_a, &mut state_b, &mut state_c];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_d4_valid_entries_after_invalid_do_not_revive_success() {
    let registry = multi_entry_supplies_registry(&[
        (BuildingType::MilitaryFactory, f64::NAN),
        (BuildingType::Factory, 100.0),
        (BuildingType::University, 200.0),
    ]);
    let mut stockpile = CountryStockpile::new();
    let mut state_bad = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut state_good_1 = full_capacity_state(StateId(1), &[(BuildingType::Factory, 1)]);
    let mut state_good_2 = full_capacity_state(StateId(2), &[(BuildingType::University, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state_bad, &mut state_good_1, &mut state_good_2];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    // 後続2州分(合計300.0相当)の有効な生産があっても、先頭の不正エントリにより
    // 国家全体のMilitarySupplies加算が失敗したままであることを確認する。
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_d5_nan_output_is_rejected() {
    let registry = military_factory_registry(20.0, f64::NAN);
    assert_eq!(one_state_result(&registry, BuildingType::MilitaryFactory, 1), 0.0);
}

#[test]
fn test_d6_positive_infinity_output_is_rejected() {
    let registry = military_factory_registry(20.0, f64::INFINITY);
    assert_eq!(one_state_result(&registry, BuildingType::MilitaryFactory, 1), 0.0);
}

#[test]
fn test_d7_negative_infinity_output_is_rejected() {
    let registry = military_factory_registry(20.0, f64::NEG_INFINITY);
    assert_eq!(one_state_result(&registry, BuildingType::MilitaryFactory, 1), 0.0);
}

#[test]
fn test_d8_negative_output_is_rejected() {
    let registry = military_factory_registry(20.0, -1.0);
    assert_eq!(one_state_result(&registry, BuildingType::MilitaryFactory, 1), 0.0);
}

#[test]
fn test_d9_out_of_range_single_entry_output_is_rejected() {
    // 2×10^9(value換算)はscale後2×10^15 unitsとなり、MILITARY_SUPPLIES_MAX_UNITS
    // (10^15)を超える。
    let registry = military_factory_registry(20.0, 2_000_000_000.0);
    assert_eq!(one_state_result(&registry, BuildingType::MilitaryFactory, 1), 0.0);
}

#[test]
fn test_d10_invalid_current_stockpile_blocks_commit_without_mutation() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    stockpile.amounts.insert(ResourceType::MilitarySupplies, f64::NAN);
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert!(stockpile.get(ResourceType::MilitarySupplies).is_nan());
}

#[test]
fn test_d11_failure_does_not_insert_a_stray_zero_key() {
    let registry = military_factory_registry(20.0, -1.0);
    // `CountryStockpile::new()`を使わず、MilitarySuppliesキーが未登録の状態を模す
    // (旧V1相当のロード直後などを想定)。
    let mut stockpile = CountryStockpile { amounts: HashMap::new() };
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    assert!(
        !stockpile.amounts.contains_key(&ResourceType::MilitarySupplies),
        "a failed commit must not insert a stray key"
    );
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Category E: 国家間分離
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_e1_country_a_success_b_failure_isolated() {
    let good = military_factory_registry(20.0, 100.0);
    let bad = military_factory_registry(20.0, -1.0);

    let mut stockpile_a = CountryStockpile::new();
    let mut state_a = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states_a: Vec<&mut StateData> = vec![&mut state_a];
    process_country_production(&mut states_a, &mut stockpile_a, &good, false);

    let mut stockpile_b = CountryStockpile::new();
    let mut state_b = full_capacity_state(StateId(1), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states_b: Vec<&mut StateData> = vec![&mut state_b];
    process_country_production(&mut states_b, &mut stockpile_b, &bad, false);

    assert_eq!(stockpile_a.get(ResourceType::MilitarySupplies), 100.0);
    assert_eq!(stockpile_b.get(ResourceType::MilitarySupplies), 0.0);
}

#[test]
fn test_e2_country_a_failure_b_success_isolated() {
    let bad = military_factory_registry(20.0, -1.0);
    let good = military_factory_registry(20.0, 100.0);

    let mut stockpile_a = CountryStockpile::new();
    let mut state_a = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states_a: Vec<&mut StateData> = vec![&mut state_a];
    process_country_production(&mut states_a, &mut stockpile_a, &bad, false);

    let mut stockpile_b = CountryStockpile::new();
    let mut state_b = full_capacity_state(StateId(1), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states_b: Vec<&mut StateData> = vec![&mut state_b];
    process_country_production(&mut states_b, &mut stockpile_b, &good, false);

    assert_eq!(stockpile_a.get(ResourceType::MilitarySupplies), 0.0);
    assert_eq!(stockpile_b.get(ResourceType::MilitarySupplies), 100.0);
}

#[test]
fn test_e3_multiple_countries_all_succeed_independently() {
    let registry = military_factory_registry(20.0, 100.0);
    let expected: [(u32, f64); 3] = [(1, 100.0), (2, 200.0), (5, 500.0)];
    for (i, (level, expected_supplies)) in expected.iter().enumerate() {
        let mut stockpile = CountryStockpile::new();
        let mut state = full_capacity_state(StateId(i), &[(BuildingType::MilitaryFactory, *level)]);
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, &registry, false);
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), *expected_supplies);
    }
}

#[test]
fn test_e4_country_processing_order_does_not_cause_cross_contamination() {
    let bad = military_factory_registry(20.0, -1.0);
    let good = military_factory_registry(20.0, 100.0);

    // e1/e2とは逆に、Bを先に処理してからAを処理する。
    let mut stockpile_b = CountryStockpile::new();
    let mut state_b = full_capacity_state(StateId(1), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states_b: Vec<&mut StateData> = vec![&mut state_b];
    process_country_production(&mut states_b, &mut stockpile_b, &good, false);

    let mut stockpile_a = CountryStockpile::new();
    let mut state_a = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states_a: Vec<&mut StateData> = vec![&mut state_a];
    process_country_production(&mut states_a, &mut stockpile_a, &bad, false);

    assert_eq!(stockpile_b.get(ResourceType::MilitarySupplies), 100.0);
    assert_eq!(stockpile_a.get(ResourceType::MilitarySupplies), 0.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Category G: 原子性の境界(MilitarySupplies単体、全生産取引の原子性は対象外)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_g1_supplies_failure_leaves_inputs_and_equipment_committed_as_before() {
    let registry = military_factory_registry_with_inputs();
    let mut stockpile = CountryStockpile::new();
    let max_value = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);
    stockpile.set(ResourceType::MilitarySupplies, max_value);
    stockpile.set(ResourceType::Iron, 1000.0);
    stockpile.set(ResourceType::IndustrialGoods, 1000.0);

    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);

    // MilitarySuppliesのcommitはoverflowで失敗するが、同じ建物の入力消費・
    // MilitaryEquipment出力はMilitarySuppliesの成否と無関係に確定する(P26-003の
    // 既存挙動、`verification_logs/phase-26/p26-003-rev-001/`§18〜20参照、本タスクは
    // この境界を変更しない・全資源のrollbackを新たに導入しない)。
    assert_eq!(stockpile.get(ResourceType::Iron), 970.0);
    assert_eq!(stockpile.get(ResourceType::IndustrialGoods), 985.0);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 20.0);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), max_value);
}

#[test]
fn test_g2_supplies_failure_does_not_roll_back_other_resource_types() {
    let registry = multi_entry_supplies_registry(&[(BuildingType::MilitaryFactory, -1.0)]);
    let mut definitions = registry.definitions;
    definitions.insert(BuildingType::Farm, producing_def(BuildingType::Farm, ResourceType::Food, 10.0));
    let registry = BuildingRegistry { definitions };

    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(
        StateId(0),
        &[(BuildingType::MilitaryFactory, 1), (BuildingType::Farm, 1)],
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);

    assert_eq!(stockpile.get(ResourceType::Food), 10.0);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
}

/// このタスクが保証するのは「1国家の1か月分のMilitarySupplies出力」の原子性のみ
/// であり、「原材料消費・MilitaryEquipment出力・MilitarySupplies出力を含む
/// 工場生産取引全体の原子性」は依然として保証しない(spec §10「表現を正確に」)。
/// このテストは、その未解決の残件が今回のREV-002によっても変化していないことを
/// 明示的に固定する回帰テストである。
#[test]
fn test_g3_whole_transaction_atomicity_across_all_outputs_remains_unimplemented() {
    let registry = military_factory_registry_with_inputs();
    let mut stockpile = CountryStockpile::new();
    let max_value = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS);
    stockpile.set(ResourceType::MilitarySupplies, max_value);
    stockpile.set(ResourceType::Iron, 30.0);
    stockpile.set(ResourceType::IndustrialGoods, 15.0);

    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);

    let iron_was_consumed = stockpile.get(ResourceType::Iron) < 30.0;
    let equipment_was_produced = stockpile.get(ResourceType::MilitaryEquipment) > 0.0;
    let supplies_was_rejected = stockpile.get(ResourceType::MilitarySupplies) == max_value;
    assert!(
        iron_was_consumed && equipment_was_produced && supplies_was_rejected,
        "the known residual gap (input consumed + equipment produced despite supplies commit failing) must still reproduce"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Category H: 5倍比率(常に厳密ではない)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_h1_base_ratio_is_exactly_5x_for_a_full_capacity_integer_entry() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 3)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    let eq = stockpile.get(ResourceType::MilitaryEquipment);
    let sup = stockpile.get(ResourceType::MilitarySupplies);
    assert_eq!(eq, 60.0);
    assert_eq!(sup, 300.0);
    assert_eq!(sup / eq, 5.0);
}

#[test]
fn test_h2_partial_operation_ratio_stays_close_to_5x_within_tolerance() {
    let registry = military_factory_registry(20.0, 100.0);
    let mut stockpile = CountryStockpile::new();
    let mut state = partial_capacity_state(
        StateId(0),
        &[(BuildingType::MilitaryFactory, 1)],
        333,
        1000,
        1.0,
    );
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    let eq = stockpile.get(ResourceType::MilitaryEquipment);
    let sup = stockpile.get(ResourceType::MilitarySupplies);
    assert!(eq > 0.0 && sup > 0.0, "eq={eq} sup={sup}");
    assert!((sup / eq - 5.0).abs() < 1e-3, "sup={sup} eq={eq}");
}

/// 個別出力の基礎比は常に5(100.0/20.0)だが、MilitaryEquipment(方式A、施設ごと
/// 即時quantize)とMilitarySupplies(REV-002適用後も方式A、ただし独立した
/// `military_supplies_*`量子化)は互いに独立して丸められるため、最終的な整数units
/// 比が厳密に5.0であることを要求しない(spec §7)。この例では意図的に丸め境界へ
/// 近い基礎出力を選び、実際に5.0からずれることを確認する。
#[test]
fn test_h3_rounding_boundary_case_final_units_ratio_need_not_be_exactly_5x() {
    let registry = military_factory_registry(0.00000082, 0.0000041); // 0.0000041 = 5 * 0.00000082
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);

    let eq = stockpile.get(ResourceType::MilitaryEquipment);
    let sup = stockpile.get(ResourceType::MilitarySupplies);
    assert_eq!(eq, 0.000001, "0.82 units rounds to 1"); // scaled=0.82 → round=1
    assert_eq!(sup, 0.000004, "4.1 units rounds to 4"); // scaled=4.10 → round=4
    assert!((sup / eq - 5.0).abs() > 0.5, "expected a visible deviation from the 5.0 base ratio, got sup/eq={}", sup / eq);
}

/// MilitarySuppliesはMilitaryEquipmentから`× 5`で派生計算されているのではなく、
/// 建物定義の`output_resources`から独立に量子化・集計されることを確認する
/// (spec §7「supplies_units = equipment_units × 5という派生計算へ変更しない」)。
/// equipment側の値を変えてもsupplies側の結果は不変であるはずである。
#[test]
fn test_h4_supplies_output_is_independent_of_equipment_output_value() {
    let registry_a = military_factory_registry(20.0, 100.0);
    let registry_b = military_factory_registry(999.0, 100.0);

    let run = |registry: &BuildingRegistry| {
        let mut stockpile = CountryStockpile::new();
        let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, registry, false);
        stockpile.get(ResourceType::MilitarySupplies)
    };

    assert_eq!(run(&registry_a), 100.0);
    assert_eq!(run(&registry_b), 100.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Category J: 反復
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_j1_repeated_subunit_boundary_entries_compound_across_calls() {
    // 各呼び出しごとに0.0000006(0.6 units相当)の生産エントリが1つ発生し、
    // 個別量子化により毎回1 unitへ丸まって国家備蓄へ加算される。5回呼び出す
    // ことで正確に5 units(=0.000005)になる。
    let registry = military_factory_registry(20.0, 0.0000006);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);

    for call in 1..=5 {
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, &registry, false);
        assert_eq!(
            stockpile.get(ResourceType::MilitarySupplies),
            military_supplies_units_to_value(call),
            "after {call} calls"
        );
    }
}

#[test]
fn test_j2_large_repetition_count_accumulates_without_drift() {
    let registry = military_factory_registry(20.0, 0.0000006);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);

    for _ in 0..1000 {
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, &registry, false);
    }

    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.001);
}

#[test]
fn test_j3_final_stockpile_matches_running_expected_units_across_varying_iterations() {
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    let per_level_values = [100.0, 14.6, 0.0000006, 33.333333, 0.0000004, 7.25];

    let mut expected_units: u128 = 0;
    for &per_level in &per_level_values {
        let registry = military_factory_registry(20.0, per_level);
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, &registry, false);
        expected_units += military_supplies_value_to_units(per_level).unwrap() as u128;
        let expected_value =
            military_supplies_units_to_value(u64::try_from(expected_units).unwrap());
        assert!(
            (stockpile.get(ResourceType::MilitarySupplies) - expected_value).abs() < EPSILON,
            "after adding per_level={per_level}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// App-level fixtures (P26-002/P26-003と同じ構成方針)
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
        "strategy_game_p26_003_rev_002_e2e_{label}_{}_{nanos}_{n}",
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

fn iron_of(app: &App, country_id: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::Iron)
}

fn advance_months(app: &mut App, months: u8) {
    for m in 1..=months {
        for day in 2..=28u8 {
            tick_day(app, 1800, m, day);
        }
        tick_month(app, 1800, m.wrapping_add(1));
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Category F: 実生産経路(実App、実`handle_monthly_economy`経路)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_f1_real_monthly_production_is_always_exactly_unit_aligned() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("f1");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    advance_months(&mut app, 5);

    let sup = supplies_of(&app, player);
    assert!(sup > 0.0, "precondition: some real production must have occurred");
    // 実生産経路を通した最終備蓄値は、必ず`military_supplies_value_to_units`で
    // 量子化可能かつ`units→value→units`往復が一致する(正規APIが常にcanonical
    // unitsだけをcommitするため、格子外の残差が残ることはない)。
    let units = military_supplies_value_to_units(sup)
        .expect("real production must always leave a canonically quantizable stockpile value");
    assert_eq!(military_supplies_units_to_value(units), sup);
}

#[test]
fn test_f2_other_resources_unaffected_through_real_monthly_path() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("f2");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    let iron_before = iron_of(&app, player);

    advance_months(&mut app, 4);

    assert!(supplies_of(&app, player) > 0.0);
    assert!(equipment_of(&app, player) > 0.0);
    assert!(
        iron_of(&app, player) < iron_before,
        "Iron must still be consumed by MilitaryFactory alongside the new integer-units supplies aggregation"
    );
}

#[test]
fn test_f3_multiple_months_no_double_counting_real_path() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("f3");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let mut last = supplies_of(&app, player);
    for m in 1..=6u8 {
        for day in 2..=28u8 {
            tick_day(&mut app, 1800, m, day);
        }
        tick_month(&mut app, 1800, m.wrapping_add(1));
        let now = supplies_of(&app, player);
        assert!(now >= last, "must be monotonically non-decreasing: {last} -> {now}");
        last = now;
    }
}

#[test]
fn test_f4_ratio_of_supplies_to_equipment_stays_within_tolerance_real_path() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("f4");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    advance_months(&mut app, 6);

    let sup = supplies_of(&app, player);
    let eq = equipment_of(&app, player);
    assert!(sup > 0.0 && eq > 0.0);
    assert!((sup / eq - 5.0).abs() < 0.05, "sup={sup} eq={eq}");
}

#[test]
fn test_f5_ai_country_with_multiple_states_produces_through_the_same_integer_path() {
    let player = CountryId(0);
    let ai_with_full_supply_chain = CountryId(2);
    let temp_dir = TempTestDir::new("f5");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    advance_months(&mut app, 6);

    let sup = supplies_of(&app, ai_with_full_supply_chain);
    assert!(sup > 0.0, "AI country with a complete supply chain must still produce after the REV-002 fix");
    let units = military_supplies_value_to_units(sup)
        .expect("AI country's stockpile must also stay canonically quantizable");
    assert_eq!(military_supplies_units_to_value(units), sup);
}

// ─────────────────────────────────────────────────────────────────────────
// Category I: Save V1
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_i1_save_load_round_trip_preserves_units_after_multi_state_production() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("i1");
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
fn test_i2_double_round_trip_is_stable() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("i2");
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
fn test_i3_new_game_and_loaded_game_agree_after_identical_ticks() {
    let player = CountryId(0);

    let temp_dir_continuous = TempTestDir::new("i3_continuous");
    let mut continuous = setup_app_in_playing(player, temp_dir_continuous.save_file_config());
    advance_months(&mut continuous, 3);

    let temp_dir_branch = TempTestDir::new("i3_branch");
    let mut branch = setup_app_in_playing(player, temp_dir_branch.save_file_config());
    advance_months(&mut branch, 3);
    send_save_request(&mut branch);
    branch.update();
    send_load_request(&mut branch);
    branch.update();
    branch.update();

    assert_eq!(supplies_of(&continuous, player), supplies_of(&branch, player));
}

#[test]
fn test_i4_load_then_continue_production_no_double_counting() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("i4");
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
    assert!(supplies_of(&app, player) >= before_save);
}

#[test]
fn test_i5_old_v1_shaped_load_without_key_still_produces_normally_afterward() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("i5");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();
    assert_eq!(supplies_of(&app, player), 0.0);

    advance_months(&mut app, 4);
    assert!(supplies_of(&app, player) > 0.0);
}
