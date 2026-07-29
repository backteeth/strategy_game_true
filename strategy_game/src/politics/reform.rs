use crate::country::GovernmentType;
use crate::politics::interest_groups::{InterestGroupState, InterestGroupType};
use crate::politics::values::{CountryValues, ValueAxis};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 実行中の価値観改革データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoliticalReform {
    pub axis: ValueAxis,
    pub start_value: f32,
    pub target_value: f32,
    pub progress: f32,
    pub required_progress: f32,
    pub monthly_progress: f32,
    pub support_score: f32,
    pub opposition_score: f32,
    pub clergy_resistance: f32,
}

impl PoliticalReform {
    pub fn new(axis: ValueAxis, current: f32, delta: f32) -> Self {
        let target = (current + delta).clamp(-100.0, 100.0);
        Self {
            axis,
            start_value: current,
            target_value: target,
            progress: 0.0,
            required_progress: 100.0,
            monthly_progress: 10.0,
            support_score: 0.0,
            opposition_score: 0.0,
            clergy_resistance: 0.0,
        }
    }
}

/// 利益団体ごとの改革への賛否 (-1.0 〜 1.0) を計算
pub fn calculate_reform_support_per_group(
    ig: InterestGroupType,
    axis: ValueAxis,
    delta: f32,
) -> f32 {
    let direction = if delta > 0.0 { 1.0 } else { -1.0 };

    match axis {
        ValueAxis::ScienceMagic => match ig {
            InterestGroupType::Mages => 1.0 * direction,
            InterestGroupType::Scholars => -direction,
            InterestGroupType::Clergy => 0.5 * direction,
            InterestGroupType::Capitalists => -0.5 * direction,
            _ => 0.0,
        },
        ValueAxis::IndividualState => match ig {
            InterestGroupType::Military | InterestGroupType::Nobility => 1.0 * direction,
            InterestGroupType::Capitalists | InterestGroupType::Merchants => -direction,
            _ => 0.0,
        },
        ValueAxis::SecularReligious => match ig {
            InterestGroupType::Clergy => 1.0 * direction,
            InterestGroupType::Scholars => -direction,
            InterestGroupType::Mages => 0.5 * direction,
            _ => 0.0,
        },
    }
}

/// 月次改革進捗と聖職者抵抗を計算
pub fn update_reform_progress_step(
    reform: &mut PoliticalReform,
    gov: GovernmentType,
    igs: &mut HashMap<InterestGroupType, InterestGroupState>,
) {
    let delta = reform.target_value - reform.start_value;
    let mut total_support = 0.0f32;
    let mut total_opp = 0.0f32;

    for (&ig_type, state) in igs.iter_mut() {
        let supp = calculate_reform_support_per_group(ig_type, reform.axis, delta);
        state.support_for_current_reform = supp;

        if supp > 0.0 {
            total_support += state.influence * supp;
        } else if supp < 0.0 {
            total_opp += state.influence * (-supp);
        }
    }

    reform.support_score = total_support;
    reform.opposition_score = total_opp;

    // 聖職者の抵抗
    let clergy_inf = igs
        .get(&InterestGroupType::Clergy)
        .map(|s| s.influence / 100.0)
        .unwrap_or(0.0);
    let clergy_supp = igs
        .get(&InterestGroupType::Clergy)
        .map(|s| s.support_for_current_reform)
        .unwrap_or(0.0);

    let clergy_resistance = if clergy_supp < 0.0 {
        clergy_inf * (-clergy_supp) * 0.8
    } else {
        0.0
    };
    reform.clergy_resistance = clergy_resistance;

    // 統治体制別補正
    let gov_mult = match gov {
        GovernmentType::Republic => {
            if total_support > total_opp {
                1.3
            } else {
                0.6
            }
        }
        GovernmentType::Dictatorship => 1.5,
        GovernmentType::Monarchy => 1.0,
        GovernmentType::Theocracy => {
            if reform.axis == ValueAxis::SecularReligious && delta < 0.0 {
                0.2 // 神権制の世俗化は極めて遅い
            } else {
                1.1
            }
        }
    };

    let base_rate = 10.0;
    let supp_mult = (total_support / (total_opp + 1.0)).clamp(0.2, 2.0);
    let final_rate = (base_rate * supp_mult * (1.0 - clergy_resistance) * gov_mult).max(1.0);

    reform.monthly_progress = final_rate;
    reform.progress += final_rate;
}

/// 完了時に価値観を適用
pub fn apply_reform_completion(reform: &PoliticalReform, values: &mut CountryValues) {
    match reform.axis {
        ValueAxis::ScienceMagic => values.science_magic = reform.target_value,
        ValueAxis::IndividualState => values.individual_state = reform.target_value,
        ValueAxis::SecularReligious => values.secular_religious = reform.target_value,
    }
    values.clamp_all();
}
