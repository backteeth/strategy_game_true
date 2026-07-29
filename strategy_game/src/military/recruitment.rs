use crate::common::{DivisionId, StateId};
use crate::country::CountryRegistry;
use crate::military::data::{ArmyStatus, ArmyUnit, MilitaryRegistry};
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
