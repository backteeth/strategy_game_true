use crate::country::{EconomicSystem, GovernmentType};
use crate::economy::economic_state::EconomicState;
use crate::politics::interest_groups::InterestGroupType;
use crate::politics::values::CountryValues;

/// 利益団体ごとの満足度 (approval: -100.0 〜 100.0) を計算する関数
#[allow(clippy::too_many_arguments)]
pub fn calculate_interest_group_approval(
    ig: InterestGroupType,
    gov: GovernmentType,
    econ: EconomicSystem,
    values: &CountryValues,
    econ_state: EconomicState,
    tax_rate: f32,
    avg_living_standard: f32,
    avg_unrest: f32,
    research_allocation_sci: f32,
    research_allocation_mag: f32,
) -> f32 {
    let mut score = 0.0f32;

    // 基本経済環境・満足度影響
    score += (avg_living_standard - 50.0) * 0.5;
    score -= avg_unrest * 0.5;

    match econ_state {
        EconomicState::Prosperity => score += 20.0,
        EconomicState::Boom => score += 10.0,
        EconomicState::Stable => score += 0.0,
        EconomicState::Recession => score -= 15.0,
        EconomicState::Depression => score -= 30.0,
    }

    match ig {
        InterestGroupType::Nobility => {
            if gov == GovernmentType::Monarchy {
                score += 25.0;
            }
            if values.individual_state > 0.0 {
                score += values.individual_state * 0.3;
            }
        }
        InterestGroupType::Clergy => {
            if gov == GovernmentType::Theocracy {
                score += 30.0;
            }
            if econ == EconomicSystem::TempleEconomy {
                score += 20.0;
            }
            if values.secular_religious > 0.0 {
                score += values.secular_religious * 0.5;
            }
            if values.secular_religious < 0.0 {
                score += values.secular_religious * 0.5;
            }
        }
        InterestGroupType::Military => {
            if gov == GovernmentType::Dictatorship {
                score += 25.0;
            }
            if values.individual_state > 0.0 {
                score += values.individual_state * 0.3;
            }
        }
        InterestGroupType::Workers => {
            if econ == EconomicSystem::Socialism || econ == EconomicSystem::PlannedEconomy {
                score += 25.0;
            }
            if tax_rate > 0.3 {
                score -= (tax_rate - 0.3) * 50.0;
            }
        }
        InterestGroupType::Capitalists => {
            if econ == EconomicSystem::FreeMarket || econ == EconomicSystem::StateCapitalism {
                score += 25.0;
            }
            if tax_rate > 0.2 {
                score -= (tax_rate - 0.2) * 60.0;
            }
        }
        InterestGroupType::Scholars => {
            if values.science_magic < 0.0 {
                score += (-values.science_magic) * 0.4;
            }
            if values.secular_religious < 0.0 {
                score += (-values.secular_religious) * 0.3;
            }
            if research_allocation_sci >= 0.3 {
                score += 15.0;
            }
        }
        InterestGroupType::Mages => {
            if values.science_magic > 0.0 {
                score += values.science_magic * 0.4;
            }
            if research_allocation_mag >= 0.3 {
                score += 15.0;
            }
        }
        InterestGroupType::Merchants => {
            if econ == EconomicSystem::FreeMarket || econ == EconomicSystem::Mercantilism {
                score += 20.0;
            }
            if tax_rate > 0.25 {
                score -= (tax_rate - 0.25) * 40.0;
            }
        }
    }

    score.clamp(-100.0, 100.0)
}
