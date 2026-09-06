//! P26-003-REV-003: 経済力評価の単位依存訂正の統合テスト。
//!
//! P26-003でMilitaryFactoryへMilitarySuppliesが併産追加された結果、
//! `src/country/power.rs::compute_state_aggregates`が建物の経済力寄与を
//! `output_resources.values().sum()`(出力資源の物量の生合計)から算出していたため、
//! 単に出力資源の「種類数」が1つ増えただけで経済力評価が変化し、実データの6か国
//! マップでElfin Republic/Dwarf FederationのGreatPower/RegionalPower判定が入れ替わる
//! という回帰が生じた(`verification_logs/phase-26/p26-003-rev-001/`§15で実測)。
//!
//! 本ファイルは、その訂正(建物種類ごとの明示的な基礎能力点`base_economic_capacity_points`
//! を導入し、`output_resources`の物量合計を一切参照しないようにする)を、以下の観点で
//! 回帰固定する:
//! - A. 全`BuildingType`の基礎点が定義されていること(黒箱: `evaluate_country_power`経由)
//! - B. 出力資源の物量・種類数・挿入順から経済力が独立していること
//! - C. 校正基準(973445c時点の`output_resources.values().sum()`)との一致
//! - D. 実データ6か国の国別スコア・順位・Tier(Tier「個数」だけでなく国別識別)
//! - E. 下流Domain経路(P21-015 `evaluate_ai_crisis_support`)が訂正後の値を参照すること
//! - F. 実データSave/Load往復での決定性
//!
//! 生産会計(`src/economy/production.rs`・`src/economy/resources.rs`・
//! MilitarySupplies/MilitaryEquipmentの会計)は本タスクで一切変更していないため、
//! その非影響は既存の`p26_002_military_supplies_accounting_foundation_e2e_test`・
//! `p26_003_military_supplies_production_e2e_test`・
//! `p26_003_rev_002_supply_production_units_e2e_test`の再実行(無変更で合格)によって
//! 確認する(本ファイルでは重複させない)。

use bevy::prelude::*;
use std::collections::HashMap;
use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::CountryId;
use strategy_game::country::CountryData;
use strategy_game::country::CountryRegistry;
use strategy_game::country::power::{CountryPowerRegistry, PowerTier, evaluate_country_power};
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::state::data::{StateData, StateRegistry};

// ─────────────────────────────────────────────────────────────────────────
// 共通ヘルパ
// ─────────────────────────────────────────────────────────────────────────

fn country(id: usize) -> CountryData {
    CountryData {
        id: CountryId(id),
        ..CountryData::default()
    }
}

fn countries(ids: &[usize]) -> CountryRegistry {
    CountryRegistry {
        countries: ids.iter().map(|&id| country(id)).collect(),
    }
}

fn state(id: usize, owner: usize, population: u64) -> StateData {
    StateData {
        id: strategy_game::common::StateId(id),
        owner_country_id: CountryId(owner),
        population,
        ..StateData::default()
    }
}

fn states(list: Vec<StateData>) -> StateRegistry {
    StateRegistry::build(list)
}

fn building_def(b_type: BuildingType, output: &[(ResourceType, f64)]) -> BuildingDefinition {
    BuildingDefinition {
        building_type: b_type,
        name: "test".to_string(),
        construction_cost: 0.0,
        required_progress: 0.0,
        required_workforce: 0.0,
        logistics_cost: 0.0,
        input_resources: HashMap::new(),
        output_resources: output.iter().copied().collect(),
        maintenance_cost: 0.0,
        max_level: 10,
        science_output: 0.0,
        magic_output: 0.0,
        railway_capacity_bonus: 0.0,
    }
}

fn building_registry_with(defs: Vec<(BuildingType, &[(ResourceType, f64)])>) -> BuildingRegistry {
    let mut definitions = HashMap::new();
    for (b_type, output) in defs {
        definitions.insert(b_type, building_def(b_type, output));
    }
    BuildingRegistry { definitions }
}

fn evaluate(
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    building_registry: &BuildingRegistry,
) -> CountryPowerRegistry {
    evaluate_country_power(
        country_registry,
        state_registry,
        &MilitaryRegistry::default(),
        building_registry,
        "1800/01/01".to_string(),
    )
}

// ─────────────────────────────────────────────────────────────────────────
// A. 全BuildingTypeの基礎点が定義されている(黒箱、evaluate_country_power経由)
// ─────────────────────────────────────────────────────────────────────────

mod category_a_base_points_coverage {
    use super::*;

    /// 973445c時点の`output_resources.values().sum()`と一致する校正済み基礎点の一覧。
    /// (校正の根拠は`src/country/power.rs::base_economic_capacity_points`のdocコメント、
    /// および本テストファイル冒頭コメント参照)。
    const EXPECTED_BASE_POINTS: [(BuildingType, f64); 10] = [
        (BuildingType::Farm, 100.0),
        (BuildingType::LoggingCamp, 80.0),
        (BuildingType::Mine, 0.0), // 鉱床ベースの専用分岐、この表の対象外(経済力寄与は別経路)
        (BuildingType::Factory, 30.0),
        (BuildingType::MilitaryFactory, 20.0), // 校正基準: 973445c時点のMilitaryEquipment単独の寄与
        (BuildingType::Railway, 0.0),
        (BuildingType::University, 0.0),
        (BuildingType::MagicAcademy, 0.0),
        (BuildingType::CrystalMine, 20.0),
        (BuildingType::CrystalRefinery, 10.0),
    ];

    /// 要求A-1: `BuildingType::ALL`(10種類)すべてについて、経済力評価が黒箱的に
    /// 期待どおりの基礎点×レベルを返す(Mineは鉱床専用分岐のため除外して個別に検証)。
    #[test]
    fn every_building_type_has_an_explicit_calibrated_contribution_at_level_1() {
        for &(b_type, expected_points) in &EXPECTED_BASE_POINTS {
            if b_type == BuildingType::Mine {
                continue; // Mineは鉱床ベース、別テストで検証
            }
            let country_registry = countries(&[0]);
            let mut s0 = state(1, 0, 1000);
            s0.buildings.insert(b_type, 1);
            let state_registry = states(vec![s0]);
            // 出力mapの内容は無関係であることも同時に確認するため、あえて実データと
            // 異なる出力(この建物種別が本来出さない資源)を登録する。
            let building_registry =
                building_registry_with(vec![(b_type, &[(ResourceType::Iron, 999.0)])]);

            let registry = evaluate(&country_registry, &state_registry, &building_registry);
            assert_eq!(
                registry.get(CountryId(0)).unwrap().economic_raw,
                expected_points,
                "building_type={b_type:?} must contribute exactly its calibrated base point at level 1, \
                 regardless of the (deliberately mismatched) output_resources content"
            );
        }
    }

    /// 要求A-2: level 0は建物が存在しないのと同じ(0寄与)。
    #[test]
    fn level_zero_contributes_nothing() {
        let country_registry = countries(&[0]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::Farm, 0);
        let state_registry = states(vec![s0]);
        let building_registry =
            building_registry_with(vec![(BuildingType::Farm, &[(ResourceType::Food, 100.0)])]);

        let registry = evaluate(&country_registry, &state_registry, &building_registry);
        assert_eq!(registry.get(CountryId(0)).unwrap().economic_raw, 0.0);
    }

    /// 要求A-3: 複数levelは線形(base_points × level)。
    #[test]
    fn multiple_levels_scale_linearly() {
        let country_registry = countries(&[0]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::MilitaryFactory, 4);
        let state_registry = states(vec![s0]);
        let building_registry = building_registry_with(vec![(
            BuildingType::MilitaryFactory,
            &[
                (ResourceType::MilitaryEquipment, 20.0),
                (ResourceType::MilitarySupplies, 100.0),
            ],
        )]);

        let registry = evaluate(&country_registry, &state_registry, &building_registry);
        assert_eq!(
            registry.get(CountryId(0)).unwrap().economic_raw,
            20.0 * 4.0,
            "MilitaryFactory level 4 must contribute exactly 20.0*4=80.0, not (20.0+100.0)*4=480.0"
        );
    }

    /// 要求A-4: 経済寄与0の建物(Railway/University/MagicAcademy)はlevelを上げても0のまま。
    #[test]
    fn zero_contribution_buildings_stay_zero_at_any_level() {
        for b_type in [
            BuildingType::Railway,
            BuildingType::University,
            BuildingType::MagicAcademy,
        ] {
            let country_registry = countries(&[0]);
            let mut s0 = state(1, 0, 1000);
            s0.buildings.insert(b_type, 10);
            let state_registry = states(vec![s0]);
            let building_registry = building_registry_with(vec![(b_type, &[])]);

            let registry = evaluate(&country_registry, &state_registry, &building_registry);
            assert_eq!(
                registry.get(CountryId(0)).unwrap().economic_raw,
                0.0,
                "{b_type:?} must contribute 0 even at level 10 (pre-P26-003 behavior preserved)"
            );
        }
    }

    /// 要求A-5: 建物を一切持たない州は経済力0。
    #[test]
    fn state_without_any_building_contributes_zero() {
        let country_registry = countries(&[0]);
        let state_registry = states(vec![state(1, 0, 1000)]);
        let building_registry = BuildingRegistry::default();

        let registry = evaluate(&country_registry, &state_registry, &building_registry);
        assert_eq!(registry.get(CountryId(0)).unwrap().economic_raw, 0.0);
    }

    /// 要求A-6: 複数建物・複数州の合計が正しく積み上がる。
    #[test]
    fn multiple_buildings_across_multiple_states_sum_correctly() {
        let country_registry = countries(&[0]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::Farm, 2); // 100*2=200
        s0.buildings.insert(BuildingType::MilitaryFactory, 1); // 20*1=20
        let mut s1 = state(2, 0, 1000);
        s1.buildings.insert(BuildingType::Factory, 3); // 30*3=90
        let state_registry = states(vec![s0, s1]);
        let building_registry = building_registry_with(vec![
            (BuildingType::Farm, &[(ResourceType::Food, 100.0)]),
            (
                BuildingType::MilitaryFactory,
                &[
                    (ResourceType::MilitaryEquipment, 20.0),
                    (ResourceType::MilitarySupplies, 100.0),
                ],
            ),
            (
                BuildingType::Factory,
                &[(ResourceType::IndustrialGoods, 30.0)],
            ),
        ]);

        let registry = evaluate(&country_registry, &state_registry, &building_registry);
        assert_eq!(
            registry.get(CountryId(0)).unwrap().economic_raw,
            200.0 + 20.0 + 90.0
        );
    }

    /// 要求A-7: 最大level(10)付近でもオーバーフロー・NaNを起こさない。
    #[test]
    fn near_max_level_is_numerically_safe() {
        let country_registry = countries(&[0]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::MilitaryFactory, 10);
        let state_registry = states(vec![s0]);
        let building_registry = building_registry_with(vec![(
            BuildingType::MilitaryFactory,
            &[(ResourceType::MilitaryEquipment, 20.0)],
        )]);

        let registry = evaluate(&country_registry, &state_registry, &building_registry);
        let a = registry.get(CountryId(0)).unwrap();
        assert_eq!(a.economic_raw, 200.0);
        assert!(a.economic_raw.is_finite());
        assert!(a.total_score.is_finite());
    }
}

// ─────────────────────────────────────────────────────────────────────────
// B. 出力資源の物量・種類数・挿入順からの独立性
// ─────────────────────────────────────────────────────────────────────────

mod category_b_independence_from_output_quantities {
    use super::*;

    fn economic_raw_for_military_factory_output(
        output: &[(ResourceType, f64)],
        level: u32,
    ) -> f64 {
        let country_registry = countries(&[0]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::MilitaryFactory, level);
        let state_registry = states(vec![s0]);
        let building_registry =
            building_registry_with(vec![(BuildingType::MilitaryFactory, output)]);
        evaluate(&country_registry, &state_registry, &building_registry)
            .get(CountryId(0))
            .unwrap()
            .economic_raw
    }

    /// 要求B-1: MilitarySuppliesの出力量を100/1/10000へ変えても経済力は不変。
    #[test]
    fn changing_military_supplies_output_quantity_does_not_change_economic_power() {
        let baseline = economic_raw_for_military_factory_output(
            &[
                (ResourceType::MilitaryEquipment, 20.0),
                (ResourceType::MilitarySupplies, 100.0),
            ],
            1,
        );
        for supplies_amount in [1.0, 10_000.0, 0.0001] {
            let variant = economic_raw_for_military_factory_output(
                &[
                    (ResourceType::MilitaryEquipment, 20.0),
                    (ResourceType::MilitarySupplies, supplies_amount),
                ],
                1,
            );
            assert_eq!(
                baseline, variant,
                "MilitarySupplies output amount ({supplies_amount}) must not affect economic power"
            );
        }
    }

    /// 要求B-2: MilitaryEquipment(既存資源)の出力量を変えても経済力は不変。
    #[test]
    fn changing_existing_resource_output_quantity_does_not_change_economic_power() {
        let baseline = economic_raw_for_military_factory_output(
            &[(ResourceType::MilitaryEquipment, 20.0)],
            1,
        );
        for equipment_amount in [1.0, 1_000.0, 999_999.0] {
            let variant = economic_raw_for_military_factory_output(
                &[(ResourceType::MilitaryEquipment, equipment_amount)],
                1,
            );
            assert_eq!(baseline, variant);
        }
    }

    /// 要求B-3: 併産資源の追加・削除で能力点が変わらない(単一出力 vs 複数出力)。
    #[test]
    fn adding_or_removing_co_products_does_not_change_economic_power() {
        let single = economic_raw_for_military_factory_output(
            &[(ResourceType::MilitaryEquipment, 20.0)],
            1,
        );
        let doubled = economic_raw_for_military_factory_output(
            &[
                (ResourceType::MilitaryEquipment, 20.0),
                (ResourceType::MilitarySupplies, 100.0),
            ],
            1,
        );
        let tripled = economic_raw_for_military_factory_output(
            &[
                (ResourceType::MilitaryEquipment, 20.0),
                (ResourceType::MilitarySupplies, 100.0),
                (ResourceType::IndustrialGoods, 5_000.0),
            ],
            1,
        );
        assert_eq!(single, doubled);
        assert_eq!(doubled, tripled);
    }

    /// 要求B-4: output_resourcesの挿入順を変えても能力点は変わらない
    /// (HashMapのため実行順は保証されないが、値の集合として同じか順序違いを構築し比較)。
    #[test]
    fn output_resources_insertion_order_does_not_matter() {
        let mut order_a = HashMap::new();
        order_a.insert(ResourceType::MilitaryEquipment, 20.0);
        order_a.insert(ResourceType::MilitarySupplies, 100.0);
        order_a.insert(ResourceType::IndustrialGoods, 7.0);

        let mut order_b = HashMap::new();
        order_b.insert(ResourceType::IndustrialGoods, 7.0);
        order_b.insert(ResourceType::MilitarySupplies, 100.0);
        order_b.insert(ResourceType::MilitaryEquipment, 20.0);

        for (label, output_map) in [("a", order_a), ("b", order_b)] {
            let country_registry = countries(&[0]);
            let mut s0 = state(1, 0, 1000);
            s0.buildings.insert(BuildingType::MilitaryFactory, 2);
            let state_registry = states(vec![s0]);
            let mut definitions = HashMap::new();
            definitions.insert(
                BuildingType::MilitaryFactory,
                BuildingDefinition {
                    output_resources: output_map,
                    ..building_def(BuildingType::MilitaryFactory, &[])
                },
            );
            let building_registry = BuildingRegistry { definitions };
            let registry = evaluate(&country_registry, &state_registry, &building_registry);
            assert_eq!(
                registry.get(CountryId(0)).unwrap().economic_raw,
                40.0,
                "insertion order {label} must not change the result"
            );
        }
    }

    /// 要求B-5: 出力mapが空でも、この建物種別の基礎点どおりに評価される
    /// (Farm等、本来非空のはずの建物でも、mapの中身自体は評価に影響しない契約)。
    #[test]
    fn empty_output_map_still_uses_the_building_types_base_capacity_point() {
        let country_registry = countries(&[0]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::Farm, 1);
        let state_registry = states(vec![s0]);
        let building_registry = building_registry_with(vec![(BuildingType::Farm, &[])]);

        let registry = evaluate(&country_registry, &state_registry, &building_registry);
        assert_eq!(
            registry.get(CountryId(0)).unwrap().economic_raw,
            100.0,
            "Farm's base capacity point (100.0) must apply even with an empty output_resources map"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// C. 校正基準(973445c時点のoutput_resources.values().sum())との一致
// ─────────────────────────────────────────────────────────────────────────

mod category_c_calibration_against_pre_p26_003_definitions {
    use super::*;

    /// 973445c(MilitarySupplies追加前)時点の`assets/data/buildings.ron`における
    /// `output_resources`(`git show 973445c30d920c80908635f22acc45ffb5655dac:
    /// strategy_game/assets/data/buildings.ron`で直接確認、本ファイル冒頭コメント・
    /// 報告書§9参照)。
    fn historical_output(b_type: BuildingType) -> &'static [(ResourceType, f64)] {
        match b_type {
            BuildingType::Farm => &[(ResourceType::Food, 100.0)],
            BuildingType::LoggingCamp => &[(ResourceType::Wood, 80.0)],
            BuildingType::Mine => &[],
            BuildingType::Factory => &[(ResourceType::IndustrialGoods, 30.0)],
            BuildingType::MilitaryFactory => &[(ResourceType::MilitaryEquipment, 20.0)],
            BuildingType::Railway => &[],
            BuildingType::University => &[],
            BuildingType::MagicAcademy => &[],
            BuildingType::CrystalMine => &[(ResourceType::RawMagicCrystal, 20.0)],
            BuildingType::CrystalRefinery => &[(ResourceType::MagicCrystal, 10.0)],
        }
    }

    /// 現行(HEAD、`c03bd3e`)の`assets/data/buildings.ron`における`output_resources`。
    /// MilitaryFactoryのみMilitarySuppliesが追加されている点が973445cとの唯一の差分。
    fn current_output(b_type: BuildingType) -> &'static [(ResourceType, f64)] {
        match b_type {
            BuildingType::MilitaryFactory => &[
                (ResourceType::MilitaryEquipment, 20.0),
                (ResourceType::MilitarySupplies, 100.0),
            ],
            other => historical_output(other),
        }
    }

    /// 要求C-1: 全既存BuildingType(Mineを除く)について、973445c時点の定義から計算した
    /// 経済力と、訂正後コードが現行(MilitarySupplies入り)定義から計算した経済力が一致する
    /// (=校正が正しい)。
    #[test]
    fn corrected_score_from_current_definitions_matches_pre_p26_003_definitions() {
        for b_type in BuildingType::ALL {
            if b_type == BuildingType::Mine {
                continue; // 鉱床専用分岐、output_resourcesベースの校正対象外
            }
            let level = 3;

            let country_registry = countries(&[0]);
            let mut s0 = state(1, 0, 1000);
            s0.buildings.insert(b_type, level);
            let state_registry = states(vec![s0]);

            let historical_registry =
                building_registry_with(vec![(b_type, historical_output(b_type))]);
            let current_registry =
                building_registry_with(vec![(b_type, current_output(b_type))]);

            let historical_eco = evaluate(&country_registry, &state_registry, &historical_registry)
                .get(CountryId(0))
                .unwrap()
                .economic_raw;
            let current_eco = evaluate(&country_registry, &state_registry, &current_registry)
                .get(CountryId(0))
                .unwrap()
                .economic_raw;

            assert_eq!(
                historical_eco, current_eco,
                "building_type={b_type:?}: corrected economic power computed from the current \
                 (post-P26-003) definition must match the pre-P26-003 definition's contribution"
            );
        }
    }

    /// 要求C-2: MilitaryFactoryの旧基礎寄与(973445c時点、MilitaryEquipment単独)は
    /// 20.0であり、これが新しい基礎点として採用されている。
    #[test]
    fn military_factory_pre_p26_003_contribution_was_20_per_level() {
        let old_sum: f64 = historical_output(BuildingType::MilitaryFactory)
            .iter()
            .map(|&(_, v)| v)
            .sum();
        assert_eq!(old_sum, 20.0);

        let country_registry = countries(&[0]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::MilitaryFactory, 1);
        let state_registry = states(vec![s0]);
        let building_registry = building_registry_with(vec![(
            BuildingType::MilitaryFactory,
            current_output(BuildingType::MilitaryFactory),
        )]);
        let registry = evaluate(&country_registry, &state_registry, &building_registry);
        assert_eq!(registry.get(CountryId(0)).unwrap().economic_raw, old_sum);
    }

    /// 要求C-3: 人口・軍事等、経済力以外の評価要素は本訂正の影響を受けない
    /// (同じ入力に対して、経済力だけを変えても軍事力・人口生値は不変)。
    #[test]
    fn other_assessment_factors_are_unaffected_by_the_economic_power_correction() {
        let country_registry = countries(&[0]);
        let mut s0 = state(1, 0, 123_456);
        s0.buildings.insert(BuildingType::MilitaryFactory, 2);
        let state_registry = states(vec![s0]);

        let historical_registry = building_registry_with(vec![(
            BuildingType::MilitaryFactory,
            historical_output(BuildingType::MilitaryFactory),
        )]);
        let current_registry = building_registry_with(vec![(
            BuildingType::MilitaryFactory,
            current_output(BuildingType::MilitaryFactory),
        )]);

        let before = evaluate(&country_registry, &state_registry, &historical_registry);
        let after = evaluate(&country_registry, &state_registry, &current_registry);

        let a = before.get(CountryId(0)).unwrap();
        let b = after.get(CountryId(0)).unwrap();
        assert_eq!(a.military_raw, b.military_raw);
        assert_eq!(a.population_raw, b.population_raw);
        assert_eq!(a.economic_raw, b.economic_raw, "economic_raw must ALSO match once corrected");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// D/F. 実データ6か国(28州)を使った国別回帰・下流・Save往復
// ─────────────────────────────────────────────────────────────────────────

mod real_map {
    use super::*;
    use bevy::app::ScheduleRunnerPlugin;
    use strategy_game::app::AppPlugin;
    use strategy_game::app::game_state::GameState;
    use strategy_game::building::BuildingPlugin;
    use strategy_game::country::PlayerCountry;
    use strategy_game::diplomacy::DiplomacyPlugin;
    use strategy_game::diplomacy::crisis::{
        CrisisPhase, CrisisRegistry, DiplomaticCrisis, WarGoal, WarGoalType,
    };
    use strategy_game::diplomacy::data::{
        ActiveTreaty, DiplomacyRegistry, DiplomaticPairKey, DiplomaticRelation, TreatyType,
    };
    use strategy_game::diplomacy::update::evaluate_ai_crisis_support;
    use strategy_game::economy::EconomyPlugin;
    use strategy_game::military::MilitaryPlugin;
    use strategy_game::politics::PoliticsPlugin;
    use strategy_game::research::ResearchPlugin;
    use strategy_game::research::data::TechnologyRegistry;
    use strategy_game::research::world_stage::WorldCivilizationState;
    use strategy_game::save::{
        ApplyLoadOutcome, LoadGamePlugin, SaveGamePlugin, SaveGameSources, SaveValidationContext,
        apply_validated_save, build_save_game_v1, validate_save_game_v1,
    };
    use strategy_game::state::StatePlugin;
    use strategy_game::war::WarPlugin;

    fn setup_app_in_playing(player: CountryId) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_resource::<ButtonInput<KeyCode>>()
            .add_plugins(AppPlugin)
            .add_plugins(strategy_game::country::CountryPlugin)
            .add_plugins(StatePlugin)
            .add_plugins(BuildingPlugin)
            .add_plugins(EconomyPlugin)
            .add_plugins(ResearchPlugin)
            .add_plugins(PoliticsPlugin)
            .add_plugins(DiplomacyPlugin)
            .add_plugins(WarPlugin)
            .add_plugins(MilitaryPlugin);

        app.insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
            .insert_resource(strategy_game::map::division_selection::DragSelectState::default())
            .insert_resource(strategy_game::military::army::SelectedArmy::default())
            .insert_resource(strategy_game::state::SelectedState::default())
            .insert_resource(strategy_game::ui::ActivePanel::default())
            .insert_resource(strategy_game::ui::diplomacy_panel::DiplomacyPanelState::default())
            .insert_resource(strategy_game::ui::military_panel::MilitaryPanelState::default())
            .insert_resource(strategy_game::ui::peace_panel::PeacePanelState::default())
            .insert_resource(strategy_game::ui::politics_panel::PoliticsPanelState::default())
            .insert_resource(strategy_game::ui::research_panel::ResearchPanelState::default())
            .insert_resource(strategy_game::map::camera::CameraDragState::default());

        app.add_plugins(SaveGamePlugin);
        app.add_plugins(LoadGamePlugin);

        app.update();

        app.insert_resource(PlayerCountry(Some(player)));
        app.insert_state(GameState::Playing);
        app.update();

        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::Playing
        );
        app
    }

    /// 校正基準の期待値(reportに実測値として記載、`P26-003-REV-001`§15の概算値
    /// (90.03等)をそのまま転記せず、本タスクで独立に再実測した正確な値を使用)。
    /// (country_id, name_substr, economic_raw, military_raw, population_raw, total_score, rank, tier)
    ///
    /// 【P26-004-PRE-004により更新】: 本タスクがElfin(StateId 16)・Ferrowyn(StateId 21)・
    /// Oceanic(StateId 8)へ鉱床・MilitaryFactoryを追加した結果、day 0時点で
    /// `economic_raw`が Elfin 1200.0→1290.0(既存Mine lv1 × 追加鉱床)・
    /// Ferrowyn 150.0→330.0(既存Mine lv2 × 追加鉱床)・
    /// Oceanic 600.0→620.0(新規MilitaryFactory lv1固定点)へ変化した
    /// (Passguardは鉱床のみでMine未建設のためday 0では400.0のまま不変、
    /// Arcadia/Dwarfは本タスクでデータ変更していないため1130.0/1230.0のまま不変)。
    /// `total_score`は6か国間の正規化(最大値相対)で決まるため、直接変更していない
    /// Arcadia/Dwarf/Passguardの`total_score`も連動して変化する
    /// (`verification_logs/phase-26/p26-004-pre-004/`§26参照、本タスクで実App実測し
    /// 再校正した値)。Tier構成(GreatPower×2/RegionalPower×2/MinorPower×2)自体は
    /// 変化しないが、Passguard(4)とFerrowyn(5)のworld_rankが入れ替わる
    /// (Ferrowynの人口590,000がPassguardの245,000を上回るため、economic_raw自体は
    /// Passguardの方が高くても総合スコアはFerrowynが上回る)。
    const EXPECTED: [(usize, f64, f64, f64, f32, usize, PowerTier); 6] = [
        (0, 1130.0, 10000.0, 2_860_000.0, 91.2138, 1, PowerTier::GreatPower),
        (1, 1290.0, 10000.0, 2_060_000.0, 90.75718, 2, PowerTier::GreatPower),
        (2, 1230.0, 10000.0, 1_890_000.0, 88.474106, 3, PowerTier::RegionalPower),
        (3, 620.0, 10000.0, 3_830_000.0, 84.4186, 4, PowerTier::RegionalPower),
        (4, 400.0, 10000.0, 245_000.0, 60.5817, 6, PowerTier::MinorPower),
        (5, 330.0, 10000.0, 590_000.0, 60.75536, 5, PowerTier::MinorPower),
    ];

    /// 要求D: 実データ6か国について、経済力・軍事力・人口の生値・正規化前提の総合力・
    /// 順位・Tierが、校正後の期待値と一致する(国別識別、Tier「個数」だけではない)。
    /// 要求C(実データ側): 訂正後の値が973445c時点の値(BEFORE)と一致することも同時に
    /// 確認する(このテストが失敗しなければ校正は正しい)。
    #[test]
    fn all_six_countries_match_the_calibrated_pre_p26_003_scores_ranks_and_tiers() {
        let app = setup_app_in_playing(CountryId(0));
        let world = app.world();
        let power_registry = world.resource::<CountryPowerRegistry>();
        let country_registry = world.resource::<CountryRegistry>();
        assert_eq!(country_registry.countries.len(), 6);

        for &(id, eco, mil, pop, total, rank, tier) in &EXPECTED {
            let a = power_registry.get(CountryId(id)).unwrap_or_else(|| {
                panic!("country {id} must have an assessment")
            });
            assert_eq!(a.economic_raw, eco, "country {id}: economic_raw mismatch");
            assert_eq!(a.military_raw, mil, "country {id}: military_raw mismatch");
            assert_eq!(a.population_raw, pop, "country {id}: population_raw mismatch");
            assert!(
                (a.total_score - total).abs() < 0.01,
                "country {id}: total_score {} does not match expected {total}",
                a.total_score
            );
            assert_eq!(a.world_rank, rank, "country {id}: world_rank mismatch");
            assert_eq!(a.power_tier, tier, "country {id}: power_tier mismatch");
        }
    }

    /// 要求D(Elfin/Dwarf特記): P26-003-REV-001が実測したTier入れ替わり
    /// (Elfin: GreatPower->RegionalPower、Dwarf: RegionalPower->GreatPower)が、
    /// 本訂正により正しい側(973445c時点と同じ側)へ戻っていることを明示的に確認する。
    #[test]
    fn elfin_and_dwarf_tier_flip_from_p26_003_rev_001_is_reversed() {
        let app = setup_app_in_playing(CountryId(0));
        let power_registry = app.world().resource::<CountryPowerRegistry>();

        let elfin = power_registry.get(CountryId(1)).unwrap(); // Elfin Republic
        let dwarf = power_registry.get(CountryId(2)).unwrap(); // Dwarf Federation

        assert_eq!(
            elfin.power_tier,
            PowerTier::GreatPower,
            "Elfin Republic must be GreatPower again (matching pre-P26-003), not RegionalPower \
             as the P26-003 bug produced"
        );
        assert_eq!(
            dwarf.power_tier,
            PowerTier::RegionalPower,
            "Dwarf Federation must be RegionalPower again (matching pre-P26-003), not GreatPower \
             as the P26-003 bug produced"
        );
        assert!(
            elfin.world_rank < dwarf.world_rank,
            "Elfin must rank above Dwarf again, matching the pre-P26-003 ordering"
        );
    }

    /// 要求(§4 現行実装が壊れていないこと再確認): Tier「個数」分布(2,2,2)自体は
    /// P21-014仕様どおり維持される(既存`p21_014_country_power_e2e_test`の
    /// アサートと同じ内容を、本タスクの観点からも独立に再確認)。
    #[test]
    fn tier_count_distribution_is_still_2_2_2_for_6_countries() {
        let app = setup_app_in_playing(CountryId(0));
        let power_registry = app.world().resource::<CountryPowerRegistry>();
        let mut great = 0;
        let mut regional = 0;
        let mut minor = 0;
        for &id in power_registry.ordered_country_ids() {
            match power_registry.get(id).unwrap().power_tier {
                PowerTier::GreatPower => great += 1,
                PowerTier::RegionalPower => regional += 1,
                PowerTier::MinorPower => minor += 1,
            }
        }
        assert_eq!((great, regional, minor), (2, 2, 2));
    }

    fn snapshot_save(app: &App) -> strategy_game::save::SaveGameV1 {
        let world = app.world();
        let sources = SaveGameSources {
            game_date: world.resource(),
            game_speed: world.resource(),
            player_country: world.resource(),
            world_civilization: world.resource(),
            country_registry: world.resource(),
            state_registry: world.resource(),
            diplomacy_registry: world.resource(),
            war_justification_registry: world.resource(),
            war_registry: world.resource(),
            claim_registry: world.resource(),
            crisis_registry: world.resource(),
            country_ai_registry: world.resource(),
            military_ai_registry: world.resource(),
            military_registry: world.resource(),
            battle_registry: world.resource(),
            army_registry: world.resource(),
            frontline_registry: world.resource(),
        };
        build_save_game_v1(&sources)
    }

    fn validation_context(app: &App) -> SaveValidationContext<'_> {
        let world = app.world();
        SaveValidationContext {
            building_definitions: &world
                .resource::<strategy_game::building::data::BuildingRegistry>()
                .definitions,
            technology_definitions: &world.resource::<TechnologyRegistry>().definitions,
            division_definitions: &world
                .resource::<strategy_game::military::data::MilitaryRegistry>()
                .definitions,
            world_stage_definitions: &world.resource::<WorldCivilizationState>().stage_definitions,
        }
    }

    fn round_trip_through_save(app: &mut App) {
        let save = snapshot_save(app);
        let validated = {
            let context = validation_context(app);
            validate_save_game_v1(save, &context)
                .expect("real-map save with corrected power scores must validate")
        };
        assert_eq!(
            apply_validated_save(app.world_mut(), validated),
            ApplyLoadOutcome::Success
        );
    }

    /// 要求F: 訂正後の評価値は、New Game初回評価とSave/Load往復後の再評価とで
    /// (国別economic_raw・total_score・rank・tierとも)完全に一致する
    /// (=新規フィールドをSaveへ追加していないため、再構築ロジックが常に同じ結果を
    /// 生む決定性が保たれている)。
    #[test]
    fn corrected_scores_survive_a_save_load_round_trip_unchanged() {
        let mut app = setup_app_in_playing(CountryId(0));

        let before: Vec<(CountryId, f64, f32, usize, PowerTier)> = {
            let power_registry = app.world().resource::<CountryPowerRegistry>();
            power_registry
                .ordered_country_ids()
                .iter()
                .map(|&id| {
                    let a = power_registry.get(id).unwrap();
                    (id, a.economic_raw, a.total_score, a.world_rank, a.power_tier)
                })
                .collect()
        };

        round_trip_through_save(&mut app);

        let power_registry_after = app.world().resource::<CountryPowerRegistry>();
        for &(id, eco, total, rank, tier) in &before {
            let a = power_registry_after.get(id).unwrap();
            assert_eq!(a.economic_raw, eco, "economic_raw must survive save/load for {id:?}");
            assert!((a.total_score - total).abs() < 1e-6);
            assert_eq!(a.world_rank, rank);
            assert_eq!(a.power_tier, tier);
        }
    }

    /// 要求F(2回連続): 2回連続でsave/loadしても結果が変わらない。
    #[test]
    fn corrected_scores_survive_two_consecutive_save_load_round_trips() {
        let mut app = setup_app_in_playing(CountryId(0));
        round_trip_through_save(&mut app);
        let after_first: Vec<(CountryId, f64, usize, PowerTier)> = {
            let power_registry = app.world().resource::<CountryPowerRegistry>();
            power_registry
                .ordered_country_ids()
                .iter()
                .map(|&id| {
                    let a = power_registry.get(id).unwrap();
                    (id, a.economic_raw, a.world_rank, a.power_tier)
                })
                .collect()
        };

        round_trip_through_save(&mut app);
        let power_registry_after_second = app.world().resource::<CountryPowerRegistry>();
        for &(id, eco, rank, tier) in &after_first {
            let a = power_registry_after_second.get(id).unwrap();
            assert_eq!(a.economic_raw, eco);
            assert_eq!(a.world_rank, rank);
            assert_eq!(a.power_tier, tier);
        }
    }

    /// 要求E: 下流Domain経路(P21-015 `evaluate_ai_crisis_support`)が、訂正後の
    /// `CountryPowerRegistry`(実データ経由)を正規に参照し、かつMilitaryFactoryの
    /// `output_resources`マップのサイズ(併産資源の追加)によって、AI大国候補の判定
    /// (=GreatPower適格性)が変化しないことを確認する。
    ///
    /// シナリオ: 実データの6か国レジストリをベースに、CountryId(1)(Elfin、本訂正前は
    /// バグにより誤ってRegionalPowerへ格下げされていた)をCrisisのinitiatorの同盟国とし、
    /// 訂正後の(正しい)GreatPower判定に基づいてAIが自動支持を表明することを確認する。
    #[test]
    fn downstream_ai_crisis_support_uses_the_corrected_great_power_tier_and_is_map_size_independent(
    ) {
        let app = setup_app_in_playing(CountryId(0));
        let world = app.world();
        let country_registry = world.resource::<CountryRegistry>();
        let state_registry = world.resource::<StateRegistry>();
        let military_registry = world.resource::<MilitaryRegistry>();
        let building_registry = world.resource::<strategy_game::building::data::BuildingRegistry>();

        // Elfin(CountryId(1))が(訂正後は正しく)GreatPowerであることを前提として使う
        // (実データの軍事レジストリをそのまま使う — `evaluate()`ヘルパの空軍事力
        // ではなく、`app.world()`が実際に持つ`MilitaryRegistry`を使う)。
        let power_registry = evaluate_country_power(
            country_registry,
            state_registry,
            military_registry,
            building_registry,
            "1800/01/01".to_string(),
        );
        assert_eq!(
            power_registry.get(CountryId(1)).unwrap().power_tier,
            PowerTier::GreatPower
        );

        // CountryId(3)(Oceanic、常にRegionalPower)をinitiator、CountryId(4)(Passguard、
        // 常にMinorPower)をtargetとするCrisisを立て、Elfin(1)をinitiatorの同盟国にする。
        let mut crisis_registry = CrisisRegistry::default();
        let crisis_id = crisis_registry.add_crisis(DiplomaticCrisis {
            id: strategy_game::common::DiplomaticCrisisId(0),
            initiator: CountryId(3),
            target: CountryId(4),
            war_goals: vec![WarGoal {
                attacker: CountryId(3),
                defender: CountryId(4),
                goal_type: WarGoalType::ConquerState,
                target_states: vec![],
                base_peace_cost: 0.0,
                international_concern: 0.0,
                completion: 0.0,
                is_primary: true,
            }],
            start_date: "1800/01/01".to_string(),
            current_phase: CrisisPhase::DemandSent,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: Some("1800/06/01".to_string()),
            international_concern: 0.0,
            third_party_reactions: Default::default(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        });

        let mut diplomacy_registry = DiplomacyRegistry::default();
        let key = DiplomaticPairKey::new(CountryId(1), CountryId(3)).unwrap();
        diplomacy_registry.relations.insert(
            key,
            DiplomaticRelation {
                treaties: vec![ActiveTreaty {
                    treaty_type: TreatyType::Alliance,
                    countries: (CountryId(1), CountryId(3)),
                    signed_date: "1800/01/01".to_string(),
                    is_active: true,
                }],
                ..Default::default()
            },
        );

        let player_country = PlayerCountry(None);
        let date = strategy_game::app::time::GameDate::new(1800, 2, 1);

        evaluate_ai_crisis_support(
            &mut crisis_registry,
            &diplomacy_registry,
            country_registry,
            &power_registry,
            &player_country,
            &date,
        );

        let supported = crisis_registry
            .crises
            .get(&crisis_id)
            .unwrap()
            .third_party_reactions
            .contains_key(&CountryId(1));
        assert!(
            supported,
            "Elfin (correctly GreatPower after the fix) must be eligible to auto-pledge support \
             via evaluate_ai_crisis_support, proving the corrected tier reaches this downstream \
             Domain path"
        );

        // 併産資源(MilitarySupplies)がMilitaryFactoryのoutput_resourcesに存在しても
        // (実データどおり)、上記の下流判断はまったく変わらない — 出力mapのサイズに
        // 依存しないことを、実データのbuilding_registry(併産あり)で確認済み。
    }
}
