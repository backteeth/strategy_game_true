use crate::building::data::BuildingType;
use crate::country::{EconomicSystem, GovernmentType};
use crate::politics::values::CountryValues;
use crate::state::data::StateData;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 8利益団体
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InterestGroupType {
    Nobility,
    Clergy,
    Military,
    Workers,
    Capitalists,
    Scholars,
    Mages,
    Merchants,
}

impl InterestGroupType {
    pub const ALL: [InterestGroupType; 8] = [
        InterestGroupType::Nobility,
        InterestGroupType::Clergy,
        InterestGroupType::Military,
        InterestGroupType::Workers,
        InterestGroupType::Capitalists,
        InterestGroupType::Scholars,
        InterestGroupType::Mages,
        InterestGroupType::Merchants,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            InterestGroupType::Nobility => "Nobility",
            InterestGroupType::Clergy => "Clergy",
            InterestGroupType::Military => "Military",
            InterestGroupType::Workers => "Workers",
            InterestGroupType::Capitalists => "Capitalists",
            InterestGroupType::Scholars => "Scholars",
            InterestGroupType::Mages => "Mages",
            InterestGroupType::Merchants => "Merchants",
        }
    }
}

/// 1利益団体の状態
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterestGroupState {
    pub influence: f32,                  // 0.0 〜 100.0 (合計100に正規化)
    pub approval: f32,                   // -100.0 〜 100.0
    pub support_for_current_reform: f32, // -1.0 〜 1.0
}

impl Default for InterestGroupState {
    fn default() -> Self {
        Self {
            influence: 12.5,
            approval: 0.0,
            support_for_current_reform: 0.0,
        }
    }
}

/// 国家全体の全利益団体状態
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CountryPoliticsData {
    pub values: CountryValues,
    pub interest_groups: HashMap<InterestGroupType, InterestGroupState>,
}

impl CountryPoliticsData {
    pub fn new_default() -> Self {
        let mut igs = HashMap::new();
        for ig in InterestGroupType::ALL {
            igs.insert(ig, InterestGroupState::default());
        }
        Self {
            values: CountryValues::default(),
            interest_groups: igs,
        }
    }

    /// 影響力合計を100.0に正規化する
    pub fn normalize_influence(&mut self) {
        let sum: f32 = self
            .interest_groups
            .values()
            .map(|ig| ig.influence.max(0.1))
            .sum();
        if sum > 0.001 {
            for ig in self.interest_groups.values_mut() {
                ig.influence = (ig.influence.max(0.1) / sum) * 100.0;
            }
        }
    }
}

/// 利益団体の基礎影響力を計算する関数
pub fn calculate_interest_group_influence(
    gov: GovernmentType,
    econ: EconomicSystem,
    values: &CountryValues,
    owned_states: &[&StateData],
) -> HashMap<InterestGroupType, f32> {
    let mut raw_inf = HashMap::new();
    for ig in InterestGroupType::ALL {
        raw_inf.insert(ig, 10.0f32);
    }

    let total_farms: u32 = owned_states
        .iter()
        .map(|s| s.building_level(BuildingType::Farm))
        .sum();
    let total_factories: u32 = owned_states
        .iter()
        .map(|s| s.building_level(BuildingType::Factory))
        .sum();
    let total_mil_factories: u32 = owned_states
        .iter()
        .map(|s| s.building_level(BuildingType::MilitaryFactory))
        .sum();
    let total_universities: u32 = owned_states
        .iter()
        .map(|s| s.building_level(BuildingType::University))
        .sum();
    let total_academies: u32 = owned_states
        .iter()
        .map(|s| s.building_level(BuildingType::MagicAcademy))
        .sum();

    // Nobility
    let mut nob = 10.0;
    if gov == GovernmentType::Monarchy {
        nob += 30.0;
    }
    nob += total_farms as f32 * 3.0;
    if values.individual_state > 0.0 {
        nob += values.individual_state * 0.1;
    }
    raw_inf.insert(InterestGroupType::Nobility, nob.max(1.0));

    // Clergy
    let mut cle = 10.0;
    if gov == GovernmentType::Theocracy {
        cle += 50.0;
    }
    if econ == EconomicSystem::TempleEconomy {
        cle += 30.0;
    }
    if values.secular_religious > 0.0 {
        cle += values.secular_religious * 0.4;
    }
    raw_inf.insert(InterestGroupType::Clergy, cle.max(1.0));

    // Military
    let mut mil = 10.0;
    if gov == GovernmentType::Dictatorship {
        mil += 30.0;
    }
    mil += total_mil_factories as f32 * 5.0;
    if values.individual_state > 0.0 {
        mil += values.individual_state * 0.2;
    }
    raw_inf.insert(InterestGroupType::Military, mil.max(1.0));

    // Workers
    let mut wrk = 10.0;
    wrk += (total_factories + total_mil_factories) as f32 * 4.0;
    if econ == EconomicSystem::Socialism || econ == EconomicSystem::PlannedEconomy {
        wrk += 30.0;
    }
    raw_inf.insert(InterestGroupType::Workers, wrk.max(1.0));

    // Capitalists
    let mut cap = 10.0;
    if econ == EconomicSystem::FreeMarket || econ == EconomicSystem::StateCapitalism {
        cap += 25.0;
    }
    cap += total_factories as f32 * 4.0;
    if values.individual_state < 0.0 {
        cap += (-values.individual_state) * 0.2;
    }
    raw_inf.insert(InterestGroupType::Capitalists, cap.max(1.0));

    // Scholars
    let mut sch = 10.0;
    sch += total_universities as f32 * 6.0;
    if values.science_magic < 0.0 {
        sch += (-values.science_magic) * 0.3;
    }
    if values.secular_religious < 0.0 {
        sch += (-values.secular_religious) * 0.2;
    }
    raw_inf.insert(InterestGroupType::Scholars, sch.max(1.0));

    // Mages
    let mut mag = 10.0;
    mag += total_academies as f32 * 6.0;
    if values.science_magic > 0.0 {
        mag += values.science_magic * 0.3;
    }
    if gov == GovernmentType::Theocracy {
        mag += 15.0;
    }
    raw_inf.insert(InterestGroupType::Mages, mag.max(1.0));

    // Merchants
    let mut mer = 10.0;
    if econ == EconomicSystem::FreeMarket
        || econ == EconomicSystem::Mercantilism
        || econ == EconomicSystem::GuildEconomy
    {
        mer += 20.0;
    }
    mer += total_factories as f32 * 2.0;
    raw_inf.insert(InterestGroupType::Merchants, mer.max(1.0));

    raw_inf
}
