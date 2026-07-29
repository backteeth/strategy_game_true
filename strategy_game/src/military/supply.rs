use crate::military::data::MilitaryRegistry;
use crate::state::data::StateRegistry;
use std::collections::HashMap;

pub fn process_supply(
    military_registry: &mut MilitaryRegistry,
    state_registry: &mut StateRegistry,
) {
    let mut state_supply_usage: HashMap<crate::common::StateId, f32> = HashMap::new();

    // Calculate total usage per state
    for army in military_registry.armies.values() {
        let usage = military_registry
            .definitions
            .get(&army.def_id)
            .map(|d| d.supply_usage)
            .unwrap_or(1.0);
        *state_supply_usage.entry(army.current_state).or_insert(0.0) += usage;
    }

    // Update state logistics ratio and apply to armies
    for (state_id, usage) in state_supply_usage.into_iter() {
        if let Some(state) = state_registry.get_mut(state_id) {
            state.logistics_usage = usage;
            let ratio = if usage > 0.0 {
                (state.logistics_capacity / usage).clamp(0.0, 1.0)
            } else {
                1.0
            };
            state.logistics_ratio = ratio;
        }
    }

    // Apply ratio to armies
    for army in military_registry.armies.values_mut() {
        if let Some(state) = state_registry.get(army.current_state) {
            army.supply_ratio = state.logistics_ratio;
        }
    }
}
