pub mod ai;
pub mod claims;
pub mod crisis;
pub mod data;
pub mod proposal;
pub mod update;

#[cfg(test)]
mod tests;

use crate::app::game_state::GameState;
use crate::app::time::DailySimulationSet;
use crate::diplomacy::claims::ClaimRegistry;
use crate::diplomacy::crisis::CrisisRegistry;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::diplomacy::update::handle_daily_diplomacy;
use bevy::prelude::*;

pub struct DiplomacyPlugin;

impl Plugin for DiplomacyPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::localization::TranslationCorePlugin)
            .insert_resource(DiplomacyRegistry::default())
            .insert_resource(ClaimRegistry::default())
            .insert_resource(CrisisRegistry::default())
            .add_systems(
                Update,
                handle_daily_diplomacy
                    .in_set(DailySimulationSet::Diplomacy)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}
