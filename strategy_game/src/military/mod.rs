pub mod ai;
pub mod combat;
pub mod data;
pub mod movement;
pub mod pathfinding;
pub mod recruitment;
pub mod supply;
pub mod update;

#[cfg(test)]
mod tests;

use crate::app::game_state::GameState;
use crate::military::data::MilitaryRegistry;
use crate::military::update::handle_daily_military;
use bevy::prelude::*;

pub struct MilitaryPlugin;

impl Plugin for MilitaryPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MilitaryRegistry::default())
            .add_systems(
                Update,
                handle_daily_military.run_if(in_state(GameState::Playing)),
            );
    }
}
