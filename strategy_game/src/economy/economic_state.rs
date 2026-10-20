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
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            EconomicState::Depression => "economic_state.depression",
            EconomicState::Recession => "economic_state.recession",
            EconomicState::Stable => "economic_state.stable",
            EconomicState::Boom => "economic_state.boom",
            EconomicState::Prosperity => "economic_state.prosperity",
        }
    }

    /// 短縮表示も現状は`display_name()`と同じ翻訳キーを指す。
    pub fn display_name_short(self) -> &'static str {
        self.display_name()
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

/// 国家に対応づけられた地域の集合から、経済状態の判定に使う指標を求める共通実装
/// (P32-010で`handle_monthly_economy`から切り出し。旧`StateData`も新ステートも同じ関数を通る)。
///
/// - 雇用率: 動員負担控除後の民間労働力(`employed + unemployed`)に対する雇用者の割合
///   (分母0なら1.0。UI用の指標とは分母が異なる、P23-003の仕様のまま)。
/// - 平均生活水準・平均物流比率は地域数での単純平均(地域なしは 50.0 / 1.0)。
/// - 平均稼働率は、全地域の全建物の稼働率の単純平均(記録なしは 1.0)。建物ごとの稼働率の加算は、
///   従来は`HashMap`の反復順(不定)だったため、`BuildingType::ALL`の宣言順で決定的に加算する
///   (式は同一で、f32の加算順だけを固定)。
pub fn compute_economic_metrics<S: crate::economy::state_access::StateEconomyAccess>(
    states: &[&mut S],
    monthly_balance: f64,
    food_stockpile: f64,
) -> EconomicMetrics {
    let total_wf: u64 = states.iter().map(|s| s.civilian_workforce()).sum();
    let total_emp: u64 = states.iter().map(|s| s.employed_workforce()).sum();
    let employment_rate = if total_wf > 0 {
        total_emp as f32 / total_wf as f32
    } else {
        1.0
    };
    let avg_living_standard: f32 = if !states.is_empty() {
        states.iter().map(|s| s.living_standard()).sum::<f32>() / states.len() as f32
    } else {
        50.0
    };
    let avg_logistics_ratio: f32 = if !states.is_empty() {
        states.iter().map(|s| s.logistics_ratio()).sum::<f32>() / states.len() as f32
    } else {
        1.0
    };
    let avg_building_operation: f32 = if !states.is_empty() {
        let mut sum_op = 0.0f32;
        let mut count = 0;
        for s in states.iter() {
            let ops = s.operation_ratios();
            for b in crate::building::data::BuildingType::ALL {
                if let Some(&op) = ops.get(&b) {
                    sum_op += op;
                    count += 1;
                }
            }
        }
        if count > 0 { sum_op / count as f32 } else { 1.0 }
    } else {
        1.0
    };
    EconomicMetrics {
        employment_rate,
        avg_living_standard,
        avg_logistics_ratio,
        monthly_balance,
        avg_building_operation,
        food_stockpile,
    }
}
