use crate::politics::interest_groups::{InterestGroupState, InterestGroupType};
use std::collections::HashMap;

/// 利益団体の満足度と影響力から再計算される派生国家補正
#[derive(Debug, Clone, Default)]
pub struct PoliticalModifierSummary {
    pub science_research_bonus: f64,
    pub magic_research_bonus: f64,
    pub military_research_bonus: f64,
    pub industrial_output_bonus: f64,
    pub tax_income_bonus: f32,
    pub monthly_unrest_modifier: f32,
    pub reform_speed_multiplier: f32,
}

pub fn calculate_political_modifiers(
    igs: &HashMap<InterestGroupType, InterestGroupState>,
) -> PoliticalModifierSummary {
    let mut summary = PoliticalModifierSummary {
        science_research_bonus: 0.0,
        magic_research_bonus: 0.0,
        military_research_bonus: 0.0,
        industrial_output_bonus: 0.0,
        tax_income_bonus: 0.0,
        monthly_unrest_modifier: 0.0,
        reform_speed_multiplier: 1.0,
    };

    for (&ig_type, state) in igs {
        let weight = (state.influence / 100.0) as f64;
        let approval = state.approval as f64;

        match ig_type {
            InterestGroupType::Scholars => {
                if approval > 25.0 {
                    summary.science_research_bonus += weight * (approval / 100.0) * 0.3;
                }
            }
            InterestGroupType::Mages => {
                if approval > 25.0 {
                    summary.magic_research_bonus += weight * (approval / 100.0) * 0.3;
                }
            }
            InterestGroupType::Military => {
                if approval > 25.0 {
                    summary.military_research_bonus += weight * (approval / 100.0) * 0.4;
                }
            }
            InterestGroupType::Workers => {
                if approval < -25.0 {
                    summary.monthly_unrest_modifier += ((-approval / 100.0) * 5.0) as f32;
                }
            }
            InterestGroupType::Capitalists => {
                if approval > 25.0 {
                    summary.industrial_output_bonus += weight * (approval / 100.0) * 0.25;
                    summary.tax_income_bonus += (weight * (approval / 100.0) * 0.15) as f32;
                }
            }
            InterestGroupType::Clergy => {
                if approval > 25.0 {
                    summary.monthly_unrest_modifier -= ((approval / 100.0) * 3.0) as f32;
                } else if approval < -25.0 {
                    summary.monthly_unrest_modifier += ((-approval / 100.0) * 4.0) as f32;
                    summary.reform_speed_multiplier *= 0.7;
                }
            }
            InterestGroupType::Nobility => {
                if approval > 25.0 {
                    summary.tax_income_bonus += (weight * 0.1) as f32;
                }
            }
            InterestGroupType::Merchants => {
                if approval > 25.0 {
                    summary.industrial_output_bonus += weight * 0.15;
                }
            }
        }
    }

    summary
}
