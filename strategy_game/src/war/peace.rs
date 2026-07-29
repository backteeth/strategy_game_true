use crate::common::{CountryId, StateId, WarId};
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;

pub enum PeaceDemand {
    AnnexState(StateId),
    WhitePeace,
}

pub fn execute_peace_treaty(
    war_id: WarId,
    winner: CountryId,
    demands: Vec<PeaceDemand>,
    state_registry: &mut StateRegistry,
    war_registry: &mut WarRegistry,
) -> Result<(), &'static str> {
    let _war = war_registry.wars.get(&war_id).ok_or("War not found")?;

    // Execute demands
    for demand in demands {
        match demand {
            PeaceDemand::AnnexState(state_id) => {
                if let Some(state) = state_registry.get_mut(state_id) {
                    state.owner_country_id = winner;
                    state.controller_country = None;
                    state.occupation_progress = 0.0;
                    state.original_owner = None;
                    state.integration = 10.0; // Starts with low integration
                }
            }
            PeaceDemand::WhitePeace => {
                // Do nothing
            }
        }
    }

    // Reset all states occupied in this war
    for state in state_registry.states.iter_mut() {
        if state.war_id == Some(war_id) {
            state.controller_country = None;
            state.occupation_progress = 0.0;
            state.original_owner = None;
            state.war_id = None;
        }
    }

    // Remove the war
    war_registry.wars.remove(&war_id);

    Ok(())
}
