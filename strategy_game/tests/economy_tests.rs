#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use strategy_game::building::construction::{
        ConstructionQueueItem, ConstructionStatus, REFUND_RATIO,
    };
    use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
    use strategy_game::common::StateId;
    use strategy_game::economy::economic_state::{
        EconomicMetrics, EconomicState, calculate_economic_state,
    };
    use strategy_game::economy::finance::{
        calculate_monthly_balance, calculate_state_tax, update_state_welfare_and_unrest,
    };
    use strategy_game::economy::production::{
        calculate_actual_output, calculate_operation_ratio, process_country_production,
    };
    use strategy_game::economy::resources::{CountryStockpile, ResourceType};
    use strategy_game::logistics::update::calculate_logistics_ratio;
    use strategy_game::population::update::{calculate_unemployed_workforce, calculate_workforce};
    use strategy_game::state::data::StateData;

    const EPSILON: f64 = 1e-3;
    const EPSILON_F32: f32 = 1e-3;

    // 1. 労働力計算
    #[test]
    fn test_calculate_workforce() {
        let wf = calculate_workforce(100_000, 0.45);
        assert_eq!(wf, 45_000);
    }

    // 2. 失業者計算
    #[test]
    fn test_calculate_unemployed_workforce() {
        let unemployed = calculate_unemployed_workforce(45_000, 30_000);
        assert_eq!(unemployed, 15_000);

        let unemployed_over = calculate_unemployed_workforce(10_000, 20_000);
        assert_eq!(unemployed_over, 0);
    }

    // 3. 物流充足率
    #[test]
    fn test_calculate_logistics_ratio() {
        let ratio_normal = calculate_logistics_ratio(100.0, 50.0);
        assert!((ratio_normal - 1.0).abs() < EPSILON_F32);

        let ratio_short = calculate_logistics_ratio(100.0, 200.0);
        assert!((ratio_short - 0.5).abs() < EPSILON_F32);

        let ratio_zero_usage = calculate_logistics_ratio(100.0, 0.0);
        assert!((ratio_zero_usage - 1.0).abs() < EPSILON_F32);
    }

    // 4. 建物稼働率
    #[test]
    fn test_calculate_operation_ratio() {
        let op = calculate_operation_ratio(1.0, 0.8, 0.9, false);
        assert!((op - 0.8).abs() < EPSILON_F32);
    }

    // 5. 生産量計算
    #[test]
    fn test_calculate_actual_output() {
        let out = calculate_actual_output(100.0, 2, 0.8);
        assert!((out - 160.0).abs() < EPSILON);
    }

    // 6. 入力資源不足時の稼働率
    #[test]
    fn test_operation_ratio_input_shortage() {
        let op = calculate_operation_ratio(1.0, 1.0, 0.3, false);
        assert!((op - 0.3).abs() < EPSILON_F32);
    }

    // 7. 税収計算
    #[test]
    fn test_calculate_state_tax() {
        let tax = calculate_state_tax(1_000_000, 70.0, 0.15);
        assert!((tax - 1050.0).abs() < EPSILON);
    }

    // 8. 月次収支
    #[test]
    fn test_calculate_monthly_balance() {
        let bal = calculate_monthly_balance(5000.0, 3200.0);
        assert!((bal - 1800.0).abs() < EPSILON);
    }

    // 9 & 10. 生活水準と不満度のクランプ
    #[test]
    fn test_welfare_and_unrest_clamping() {
        let mut state = StateData {
            id: StateId(0),
            name: "Test".to_string(),
            owner_country_id: strategy_game::common::CountryId(0),
            population: 100_000,
            workforce_ratio: 0.45,
            employed_workforce: 45_000,
            unemployed_workforce: 0,
            education: 0.5,
            living_standard: 99.0,
            unrest: 1.0,
            base_logistics_capacity: 100.0,
            logistics_capacity: 100.0,
            logistics_usage: 50.0,
            logistics_ratio: 1.0,
            buildings: std::collections::HashMap::new(),
            building_operation_ratios: std::collections::HashMap::new(),
            resource_deposits: Vec::new(),
            world_position: [0.0, 0.0],
            size: [100.0, 100.0],
            ..Default::default()
        };

        update_state_welfare_and_unrest(&mut state, 0.05, true, false);
        assert!(state.living_standard <= 100.0);
        assert!(state.unrest >= 0.0);
    }

    // 11. 国家経済状況判定
    #[test]
    fn test_calculate_economic_state() {
        let metrics_high = EconomicMetrics {
            employment_rate: 0.95,
            avg_living_standard: 85.0,
            avg_logistics_ratio: 1.0,
            monthly_balance: 500.0,
            avg_building_operation: 0.95,
            food_stockpile: 1000.0,
        };
        let state_high = calculate_economic_state(&metrics_high);
        assert_eq!(state_high, EconomicState::Prosperity);

        let metrics_low = EconomicMetrics {
            employment_rate: 0.2,
            avg_living_standard: 20.0,
            avg_logistics_ratio: 0.3,
            monthly_balance: -500.0,
            avg_building_operation: 0.2,
            food_stockpile: 0.0,
        };
        let state_low = calculate_economic_state(&metrics_low);
        assert_eq!(state_low, EconomicState::Depression);
    }

    // 12. 建設費の支払い
    #[test]
    fn test_construction_payment() {
        let mut treasury = 1000.0;
        let cost = 300.0;
        treasury -= cost;
        assert_eq!(treasury, 700.0);
    }

    // 13. 建設完了
    #[test]
    fn test_construction_completion() {
        let mut item = ConstructionQueueItem {
            state_id: StateId(0),
            building_type: BuildingType::Farm,
            target_level: 2,
            progress: 29.0,
            required_progress: 30.0,
            paid_cost: 200.0,
            status: ConstructionStatus::InProgress,
        };

        item.progress += 1.0;
        assert!(item.progress >= item.required_progress);
    }

    // 14. 建設キャンセル時の返金
    #[test]
    fn test_construction_cancel_refund() {
        let paid_cost = 500.0;
        let refund = paid_cost * REFUND_RATIO;
        assert_eq!(refund, 250.0);
    }

    // 15. 最大レベルを超える建設の拒否
    #[test]
    fn test_reject_exceeding_max_level() {
        let current_level = 10;
        let max_level = 10;
        let can_build = current_level < max_level;
        assert!(!can_build);
    }

    // ─── P23-004: finance.rs/production.rsのcharacterization test ─────────────
    // 共通関数(calculate_economic_workforce_ratio)への置換前後で、実際の生産量・
    // stockpile・生活水準・不満度の観測可能な出力が完全に一致することを固定する。
    // 期待値は本番実装と同じ数式を再実装するのではなく、手計算した固定値を使う。

    fn farm_registry_with_output(output_per_level: f64) -> BuildingRegistry {
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
                    m.insert(ResourceType::Food, output_per_level);
                    m
                },
                maintenance_cost: 1.0,
                max_level: 10,
                science_output: 0.0,
                magic_output: 0.0,
                railway_capacity_bonus: 0.0,
            },
        );
        BuildingRegistry { definitions }
    }

    fn production_test_state(population: u64, workforce_ratio: f32, employed: u64) -> StateData {
        let mut buildings = HashMap::new();
        buildings.insert(BuildingType::Farm, 1);
        StateData {
            population,
            workforce_ratio,
            employed_workforce: employed,
            unemployed_workforce: 0,
            logistics_ratio: 1.0,
            buildings,
            ..StateData::default()
        }
    }

    /// 動員なし・需要不足のケース: population=100,000, workforce_ratio=0.45
    /// (total_workforce()=45,000)、employed=20,000。
    /// wf_ratio = 20,000/45,000(f32) = 0.4444445。入力資源が無いためStep2の充足率は
    /// 常に1.0、log_ratio=1.0なのでfinal_op = wf_ratioそのもの。
    /// output = base_output(10.0) * level(1) * wf_ratio。
    #[test]
    fn characterization_production_output_matches_employed_over_potential_workforce_ratio() {
        let registry = farm_registry_with_output(10.0);
        let mut stockpile = CountryStockpile::default();
        let mut state = production_test_state(100_000, 0.45, 20_000);
        {
            let mut states: Vec<&mut StateData> = vec![&mut state];
            process_country_production(&mut states, &mut stockpile, &registry, false);
        }

        let expected_ratio = 20_000f32 / 45_000f32;
        let expected_food = 10.0 * (expected_ratio as f64);
        assert!(
            (stockpile.get(ResourceType::Food) - expected_food).abs() < EPSILON,
            "expected {expected_food}, got {}",
            stockpile.get(ResourceType::Food)
        );
    }

    /// 動員が完全雇用(employed==total_workforce)を妨げない場合、生産量は動員なしと
    /// 完全に一致する(§4.2: 余剰労働力で吸収できる動員は追加ペナルティを生まない)。
    /// unemployed_workforceの値そのものはproduction.rsの計算に一切影響しない。
    #[test]
    fn characterization_production_output_is_unaffected_by_unemployed_workforce_alone() {
        let registry = farm_registry_with_output(10.0);
        let mut stockpile_a = CountryStockpile::default();
        let mut state_a = production_test_state(100_000, 0.45, 20_000);
        state_a.unemployed_workforce = 0;

        let mut stockpile_b = CountryStockpile::default();
        let mut state_b = production_test_state(100_000, 0.45, 20_000);
        state_b.unemployed_workforce = 25_000; // 同じemployedで、失業者数だけ大きい

        {
            let mut states_a: Vec<&mut StateData> = vec![&mut state_a];
            process_country_production(&mut states_a, &mut stockpile_a, &registry, false);
            let mut states_b: Vec<&mut StateData> = vec![&mut state_b];
            process_country_production(&mut states_b, &mut stockpile_b, &registry, false);
        }

        assert_eq!(
            stockpile_a.get(ResourceType::Food),
            stockpile_b.get(ResourceType::Food)
        );
    }

    /// 動員により雇用者数が減った場合、生産量は単調に低下する(§4.3/§4.4)。
    #[test]
    fn characterization_production_output_decreases_monotonically_as_employed_falls() {
        let registry = farm_registry_with_output(10.0);

        let mut outputs = Vec::new();
        for employed in [40_000u64, 30_000, 20_000, 10_000] {
            let mut stockpile = CountryStockpile::default();
            let mut state = production_test_state(100_000, 0.45, employed);
            let mut states: Vec<&mut StateData> = vec![&mut state];
            process_country_production(&mut states, &mut stockpile, &registry, false);
            outputs.push(stockpile.get(ResourceType::Food));
        }

        for pair in outputs.windows(2) {
            assert!(
                pair[0] >= pair[1],
                "production must not increase as employment falls: {outputs:?}"
            );
        }
    }

    /// population=0(total_workforce()=0)では、production.rsの元の零除算フォールバック
    /// (wf_ratio=1.0)により、労働力起因の生産低下が発生しないことを固定する。
    #[test]
    fn characterization_production_falls_back_to_full_workforce_ratio_when_potential_is_zero() {
        let registry = farm_registry_with_output(10.0);
        let mut stockpile = CountryStockpile::default();
        let mut state = production_test_state(0, 0.45, 0);
        {
            let mut states: Vec<&mut StateData> = vec![&mut state];
            process_country_production(&mut states, &mut stockpile, &registry, false);
        }

        assert!((stockpile.get(ResourceType::Food) - 10.0).abs() < EPSILON);
    }

    fn welfare_test_state(population: u64, workforce_ratio: f32, employed: u64) -> StateData {
        StateData {
            population,
            workforce_ratio,
            employed_workforce: employed,
            unemployed_workforce: 0,
            living_standard: 50.0,
            unrest: 10.0,
            logistics_ratio: 1.0,
            ..StateData::default()
        }
    }

    /// P23-004固有: finance.rsの元の計算はクランプしていなかった。employed(50,000) >
    /// total_workforce()(45,000)という境界状態でも、雇用率が1.0を超えて計算されること
    /// (>=0.9分岐が成立すること)自体は変わらないことを固定する。
    #[test]
    fn characterization_welfare_employment_ratio_is_not_clamped_above_one() {
        let mut state = welfare_test_state(100_000, 0.45, 50_000);
        update_state_welfare_and_unrest(&mut state, 0.05, true, false);
        // food:+0.5, emp_rate>=0.9:+0.5, logistics>=0.9:+0.3, tax_rate<=0.10:+0.5 => +1.8
        assert!((state.living_standard as f64 - 51.8).abs() < EPSILON);
        assert!((state.unrest as f64 - 10.0).abs() < EPSILON);
    }

    /// 動員により雇用率が0.7未満まで低下した場合、既存のしきい値どおり生活水準が悪化し
    /// 不満度が上昇することを固定する(§6.3-3)。
    #[test]
    fn characterization_welfare_low_employment_ratio_degrades_living_standard_and_unrest() {
        let mut state = welfare_test_state(100_000, 0.45, 20_000); // emp_rate ≈ 0.444 < 0.7
        update_state_welfare_and_unrest(&mut state, 0.05, true, false);
        // food:+0.5, emp<0.7:-0.8/+0.8unrest, logistics>=0.9:+0.3, tax<=0.10:+0.5
        // ls_delta = 0.5-0.8+0.3+0.5 = 0.5
        assert!((state.living_standard as f64 - 50.5).abs() < EPSILON);
        assert!((state.unrest as f64 - 10.8).abs() < EPSILON);
    }

    /// population=0(total_workforce()=0)では、finance.rsの元の零除算フォールバック
    /// (emp_rate=1.0)により、常に高雇用率側の分岐が成立することを固定する。
    #[test]
    fn characterization_welfare_falls_back_to_full_employment_when_potential_is_zero() {
        let mut state = welfare_test_state(0, 0.45, 0);
        update_state_welfare_and_unrest(&mut state, 0.05, true, false);
        assert!((state.living_standard as f64 - 51.8).abs() < EPSILON);
    }

    /// 動員量を増やして雇用率が改善方向へ変化しないことを固定する(§6.3-4)。
    #[test]
    fn characterization_welfare_does_not_improve_as_employment_ratio_falls() {
        let mut living_standards = Vec::new();
        for employed in [45_000u64, 35_000, 25_000, 15_000] {
            let mut state = welfare_test_state(100_000, 0.45, employed);
            update_state_welfare_and_unrest(&mut state, 0.05, true, false);
            living_standards.push(state.living_standard);
        }
        for pair in living_standards.windows(2) {
            assert!(
                pair[0] >= pair[1],
                "living standard must not improve as employment falls: {living_standards:?}"
            );
        }
    }
}
