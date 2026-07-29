use crate::common::StateId;
use crate::military::combat::calculate_combat_strength;
use crate::military::data::{ArmyStatus, MilitaryRegistry};
use crate::war::data::WarRegistry;
use std::collections::HashMap;

pub fn process_combat(military_registry: &mut MilitaryRegistry, war_registry: &WarRegistry) {
    // Group armies by state
    let mut state_armies: HashMap<StateId, Vec<crate::common::ArmyId>> = HashMap::new();

    for army in military_registry.armies.values() {
        state_armies
            .entry(army.current_state)
            .or_default()
            .push(army.id);
    }

    for (_state_id, armies) in state_armies.into_iter() {
        if armies.len() < 2 {
            continue;
        } // Need at least 2 armies for combat

        // Very basic combat: check for hostilities
        // Find if any two armies belong to countries at war
        let mut hostile_pairs = Vec::new();
        for i in 0..armies.len() {
            for j in (i + 1)..armies.len() {
                let a1 = military_registry.armies.get(&armies[i]).unwrap();
                let a2 = military_registry.armies.get(&armies[j]).unwrap();

                if war_registry.are_countries_at_war(a1.owner, a2.owner) {
                    hostile_pairs.push((armies[i], armies[j]));
                }
            }
        }

        // Resolve combat for hostile pairs (1-on-1 for simplicity, no complex frontlines yet)
        for (id1, id2) in hostile_pairs {
            // Re-borrow armies
            let a1 = military_registry.armies.get(&id1).unwrap().clone();
            let a2 = military_registry.armies.get(&id2).unwrap().clone();

            let s1 = calculate_combat_strength(&a1, military_registry);
            let s2 = calculate_combat_strength(&a2, military_registry);

            // Apply damage
            let damage_to_1 = s2 * 0.1; // Base damage factor
            let damage_to_2 = s1 * 0.1;

            let a1_mut = military_registry.armies.get_mut(&id1).unwrap();
            a1_mut.status = ArmyStatus::Fighting;
            a1_mut.manpower = a1_mut.manpower.saturating_sub(damage_to_1 as u64);
            a1_mut.equipment = (a1_mut.equipment - damage_to_1 as f64 * 0.1).max(0.0);
            a1_mut.organization = (a1_mut.organization - damage_to_1 * 0.5).max(0.0);

            let a2_mut = military_registry.armies.get_mut(&id2).unwrap();
            a2_mut.status = ArmyStatus::Fighting;
            a2_mut.manpower = a2_mut.manpower.saturating_sub(damage_to_2 as u64);
            a2_mut.equipment = (a2_mut.equipment - damage_to_2 as f64 * 0.1).max(0.0);
            a2_mut.organization = (a2_mut.organization - damage_to_2 * 0.5).max(0.0);
        }
    }
}
