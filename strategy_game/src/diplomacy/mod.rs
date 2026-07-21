pub mod data;
pub mod proposal;
pub mod update;

use crate::app::game_state::GameState;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::diplomacy::update::handle_daily_diplomacy;
use bevy::prelude::*;

pub struct DiplomacyPlugin;

impl Plugin for DiplomacyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DiplomacyRegistry::default())
            .add_systems(
                Update,
                handle_daily_diplomacy.run_if(in_state(GameState::Playing)),
            );
    }
}
