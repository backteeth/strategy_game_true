use crate::common::{CountryId, DivisionId, StateId};
use crate::country::CountryRegistry;
use crate::military::data::{ArmyStatus, ArmyUnit, MilitaryRegistry};
use crate::state::data::StateRegistry;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecruitmentQueueItem {
    pub division_id: DivisionId,
    pub target_state: StateId,
    pub days_remaining: u32,
    pub total_days: u32,
}

pub fn process_recruitment(
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
) {
    let mut new_armies = Vec::new();

    for country in country_registry.countries.iter_mut() {
        let mut completed_indices = Vec::new();

        for (i, item) in country.recruitment_queue.iter_mut().enumerate() {
            if item.days_remaining > 0 {
                item.days_remaining -= 1;
            }

            if item.days_remaining == 0 {
                completed_indices.push(i);

                if let Some(def) = military_registry.definitions.get(&item.division_id) {
                    new_armies.push(ArmyUnit {
                        id: crate::common::ArmyId(0), // Assigned in add_army
                        owner: country.id,
                        division_type: def.division_type,
                        size: def.size,
                        current_state: item.target_state,
                        destination: None,
                        current_path: Vec::new(),
                        target_state: None,
                        manpower: def.required_manpower,
                        max_manpower: def.required_manpower,
                        equipment: def.required_equipment,
                        max_equipment: def.required_equipment,
                        organization: def.organization * 0.1, // Start with low organization
                        max_organization: def.organization,
                        morale: def.morale * 0.5,
                        max_morale: def.morale,
                        experience: 0.0,
                        supply_ratio: 1.0,
                        movement_progress: 0.0,
                        status: ArmyStatus::Idle,
                        def_id: item.division_id,
                        attack_power: def.attack as i32,
                        defense_power: def.defense as i32,
                        combat_id: None,
                    });
                }
            }
        }

        for i in completed_indices.iter().rev() {
            country.recruitment_queue.remove(*i);
        }
    }

    for army in new_armies {
        military_registry.add_army(army);
    }
}

pub fn request_recruitment(
    country: &mut crate::country::CountryData,
    military_registry: &MilitaryRegistry,
    division_id: DivisionId,
    target_state: StateId,
) -> Result<(), &'static str> {
    let def = military_registry
        .definitions
        .get(&division_id)
        .ok_or("Unknown division type")?;

    if country.available_manpower < def.required_manpower {
        return Err("Insufficient manpower");
    }

    if country.treasury < def.required_equipment {
        return Err("Insufficient equipment/treasury"); // Assuming equipment costs money for now
    }

    country.available_manpower -= def.required_manpower;
    country.mobilized_manpower += def.required_manpower;
    country.treasury -= def.required_equipment; // Simplified

    country.recruitment_queue.push(RecruitmentQueueItem {
        division_id,
        target_state,
        days_remaining: def.recruitment_days,
        total_days: def.recruitment_days,
    });

    Ok(())
}

pub fn cancel_recruitment(
    country: &mut crate::country::CountryData,
    military_registry: &MilitaryRegistry,
    queue_index: usize,
) -> Result<(), &'static str> {
    if queue_index >= country.recruitment_queue.len() {
        return Err("Invalid queue index");
    }

    let item = country.recruitment_queue.remove(queue_index);
    if let Some(def) = military_registry.definitions.get(&item.division_id) {
        country.available_manpower += def.required_manpower;
        country.mobilized_manpower = country
            .mobilized_manpower
            .saturating_sub(def.required_manpower);
        country.treasury += def.required_equipment; // Refund
    }

    Ok(())
}

/// P21-001: UIが募兵ボタンの表示・押下可否を判定するための副作用のない事前評価結果。
///
/// `request_recruitment`自体は対象州の所有権を検証しない(呼び出し元の責務)ため、
/// この関数がUI層向けに所有権チェックを含めた一段階手前の判定として提供する。
/// 実際の資金・人的資源消費は`request_recruitment`が改めて原子的に検証・実行する
/// (この関数の結果が`Ready`でも、呼び出し間に状態が変化した場合は
/// `request_recruitment`側の再検証で安全に拒否される)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecruitFeasibility {
    Ready,
    NoStateSelected,
    NotOwnState,
    DefinitionUnavailable,
    InsufficientManpower,
    InsufficientFunds,
}

impl RecruitFeasibility {
    pub fn is_ready(self) -> bool {
        matches!(self, RecruitFeasibility::Ready)
    }
}

/// 指定した州・部隊定義に対して募兵操作が実行可能かを判定する(副作用なし)。
#[allow(clippy::too_many_arguments)]
pub fn evaluate_recruit_feasibility(
    selected_state: Option<StateId>,
    player_country_id: CountryId,
    state_registry: &StateRegistry,
    country: &crate::country::CountryData,
    military_registry: &MilitaryRegistry,
    division_id: DivisionId,
) -> RecruitFeasibility {
    let Some(state_id) = selected_state else {
        return RecruitFeasibility::NoStateSelected;
    };
    let Some(state) = state_registry.get(state_id) else {
        return RecruitFeasibility::NoStateSelected;
    };
    if state.owner_country_id != player_country_id {
        return RecruitFeasibility::NotOwnState;
    }

    let Some(def) = military_registry.definitions.get(&division_id) else {
        return RecruitFeasibility::DefinitionUnavailable;
    };

    if country.available_manpower < def.required_manpower {
        return RecruitFeasibility::InsufficientManpower;
    }
    if country.treasury < def.required_equipment {
        return RecruitFeasibility::InsufficientFunds;
    }

    RecruitFeasibility::Ready
}
