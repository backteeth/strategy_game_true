use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;

pub fn process_war_score(state_registry: &StateRegistry, war_registry: &mut WarRegistry) {
    for war in war_registry.wars.values_mut() {
        let mut score: f32 = 0.0;

        for state in state_registry.states.iter() {
            if war.defenders.contains(&state.owner_country_id) {
                if matches!(state.controller_country, Some(c) if war.attackers.contains(&c)) {
                    score += 10.0; // 10 points per occupied state
                }
            } else if war.attackers.contains(&state.owner_country_id)
                && matches!(state.controller_country, Some(c) if war.defenders.contains(&c))
            {
                score -= 10.0; // -10 points if defender occupies attacker's state
            }
        }

        war.war_score = score.clamp(-100.0, 100.0);
    }
}
