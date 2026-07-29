use crate::military::data::{ArmyStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;

pub fn process_occupation(
    state_registry: &mut StateRegistry,
    military_registry: &mut MilitaryRegistry,
    war_registry: &WarRegistry,
) {
    for army in military_registry.armies.values_mut() {
        if army.status != ArmyStatus::Idle && army.status != ArmyStatus::Occupying {
            continue;
        }

        let state_owner = if let Some(s) = state_registry.get(army.current_state) {
            s.controller_country
        } else {
            continue;
        };

        if let (Some(owner), Some(war)) = (
            state_owner,
            war_registry.get_active_war_for_country(army.owner),
        ) {
            let is_enemy_state = (war.attackers.contains(&army.owner)
                && war.defenders.contains(&owner))
                || (war.defenders.contains(&army.owner) && war.attackers.contains(&owner));

            if is_enemy_state {
                army.status = ArmyStatus::Occupying;

                if let Some(state) = state_registry.get_mut(army.current_state) {
                    state.occupation_progress += 5.0; // 20日で占領完了

                    if state.occupation_progress >= 100.0 {
                        state.occupation_progress = 100.0;
                        state.controller_country = Some(army.owner);
                        state.war_id = Some(war.id);
                        army.status = ArmyStatus::Idle;
                    }
                }
            } else if army.status == ArmyStatus::Occupying {
                army.status = ArmyStatus::Idle;
            }
        }
    }
}
