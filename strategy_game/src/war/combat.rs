use crate::common::StateId;
use crate::military::combat::calculate_combat_strength;
use crate::military::data::{ArmyStatus, MilitaryRegistry};
use crate::war::data::{WarRegistry, WarStatus};
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

/// 終了した戦闘の勝敗結果を戦争データに一度だけ記録する（BattleId昇順で決定的に処理）
pub fn sync_battle_results_to_wars(
    battle_registry: &crate::military::battle::BattleRegistry,
    war_registry: &mut WarRegistry,
) {
    let mut battle_ids: Vec<crate::common::BattleId> =
        battle_registry.battles.keys().copied().collect();
    battle_ids.sort_by_key(|id| id.0);

    for battle_id in battle_ids {
        let battle = match battle_registry.battles.get(&battle_id) {
            Some(b) => b,
            None => continue,
        };

        if battle.status == crate::military::battle::BattleStatus::Ongoing
            || battle.status == crate::military::battle::BattleStatus::Cancelled
        {
            continue;
        }

        if let Some(war) = war_registry.wars.get_mut(&battle.war_id) {
            if war.status != WarStatus::Active {
                continue;
            }
            if war.processed_battle_ids.contains(&battle.id) {
                continue;
            }

            match battle.status {
                crate::military::battle::BattleStatus::AttackerWon => {
                    if war.attackers.contains(&battle.attacker_country) {
                        war.won_attacker_battles += 1;
                    } else {
                        war.won_defender_battles += 1;
                    }
                    war.processed_battle_ids.insert(battle.id);
                }
                crate::military::battle::BattleStatus::DefenderWon => {
                    if war.defenders.contains(&battle.defender_country) {
                        war.won_defender_battles += 1;
                    } else {
                        war.won_attacker_battles += 1;
                    }
                    war.processed_battle_ids.insert(battle.id);
                }
                _ => {}
            }
        }
    }
}
