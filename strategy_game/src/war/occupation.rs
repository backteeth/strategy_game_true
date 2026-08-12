use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;

pub fn process_occupation(
    state_registry: &mut StateRegistry,
    military_registry: &mut MilitaryRegistry,
    war_registry: &WarRegistry,
) {
    for division in military_registry.divisions.values_mut() {
        if division.status != DivisionStatus::Idle && division.status != DivisionStatus::Occupying {
            continue;
        }

        let state_owner = if let Some(s) = state_registry.get(division.current_state) {
            s.controller_country
        } else {
            continue;
        };

        if let (Some(owner), Some(war)) = (
            state_owner,
            war_registry.get_active_war_for_country(division.owner),
        ) {
            let is_enemy_state = (war.attackers.contains(&division.owner)
                && war.defenders.contains(&owner))
                || (war.defenders.contains(&division.owner) && war.attackers.contains(&owner));

            if is_enemy_state {
                division.status = DivisionStatus::Occupying;

                if let Some(state) = state_registry.get_mut(division.current_state) {
                    state.occupation_progress += 5.0; // 20日で占領完了

                    if state.occupation_progress >= 100.0 {
                        state.occupation_progress = 100.0;
                        state.controller_country = Some(division.owner);
                        state.war_id = Some(war.id);
                        division.status = DivisionStatus::Idle;
                    }
                }
            } else if division.status == DivisionStatus::Occupying {
                division.status = DivisionStatus::Idle;
            }
        }
    }
}
