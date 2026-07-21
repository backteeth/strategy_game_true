use serde::{Deserialize, Serialize};

/// 国家経済状況の5段階
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EconomicState {
    /// 大恐慌 (Depression)
    Depression,
    /// 不況 (Recession)
    Recession,
    /// 安定 (Stable)
    #[default]
    Stable,
    /// 好況 (Boom)
    Boom,
    /// 繁栄 (Prosperity)
    Prosperity,
}

impl EconomicState {
    pub fn display_name(self) -> &'static str {
        match self {
            EconomicState::Depression => "大恐慌 (Depression)",
            EconomicState::Recession => "不況 (Recession)",
            EconomicState::Stable => "安定 (Stable)",
            EconomicState::Boom => "好況 (Boom)",
            EconomicState::Prosperity => "繁栄 (Prosperity)",
        }
    }

    pub fn display_name_short(self) -> &'static str {
        match self {
            EconomicState::Depression => "大恐慌",
            EconomicState::Recession => "不況",
            EconomicState::Stable => "安定",
            EconomicState::Boom => "好況",
            EconomicState::Prosperity => "繁栄",
        }
    }
}

/// 経済指標パラメータ群
pub struct EconomicMetrics {
    pub employment_rate: f32,        // 0.0〜1.0
    pub avg_living_standard: f32,    // 0.0〜100.0
    pub avg_logistics_ratio: f32,    // 0.0〜1.0
    pub monthly_balance: f64,        // 月次収支
    pub avg_building_operation: f32, // 0.0〜1.0
    pub food_stockpile: f64,         // 食料備蓄
}

/// 国家経済状況を判定する専用関数
pub fn calculate_economic_state(metrics: &EconomicMetrics) -> EconomicState {
    let mut score = 0.0;

    score += (metrics.employment_rate.clamp(0.0, 1.0) * 25.0) as f64;
    score += (metrics.avg_living_standard.clamp(0.0, 100.0) * 0.25) as f64;
    score += (metrics.avg_logistics_ratio.clamp(0.0, 1.0) * 15.0) as f64;
    score += (metrics.avg_building_operation.clamp(0.0, 1.0) * 15.0) as f64;

    if metrics.monthly_balance > 0.0 {
        score += 10.0;
    } else if metrics.monthly_balance < 0.0 {
        score -= 10.0;
    }

    if metrics.food_stockpile >= 500.0 {
        score += 10.0;
    } else if metrics.food_stockpile <= 0.0 {
        score -= 10.0;
    }

    if score >= 80.0 {
        EconomicState::Prosperity
    } else if score >= 60.0 {
        EconomicState::Boom
    } else if score >= 40.0 {
        EconomicState::Stable
    } else if score >= 20.0 {
        EconomicState::Recession
    } else {
        EconomicState::Depression
    }
}
