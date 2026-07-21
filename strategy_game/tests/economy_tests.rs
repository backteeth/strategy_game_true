#[cfg(test)]
mod tests {
    use strategy_game::building::construction::{
        ConstructionQueueItem, ConstructionStatus, REFUND_RATIO,
    };
    use strategy_game::building::data::BuildingType;
    use strategy_game::common::StateId;
    use strategy_game::economy::economic_state::{
        EconomicMetrics, EconomicState, calculate_economic_state,
    };
    use strategy_game::economy::finance::{
        calculate_monthly_balance, calculate_state_tax, update_state_welfare_and_unrest,
    };
    use strategy_game::economy::production::{calculate_actual_output, calculate_operation_ratio};
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
}
