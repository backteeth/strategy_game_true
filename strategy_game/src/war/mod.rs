pub mod combat;
pub mod data;
pub mod justification;
pub mod occupation;
pub mod peace;
pub mod update;
pub mod war_score;

#[cfg(test)]
pub mod tests;

use crate::app::game_state::GameState;
use crate::war::data::WarRegistry;
use crate::war::justification::WarJustificationRegistry;
use crate::war::update::handle_daily_war;
use bevy::prelude::*;

pub struct WarPlugin;

impl Plugin for WarPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WarRegistry::default())
            .insert_resource(WarJustificationRegistry::default())
            .add_systems(
                Update,
                handle_daily_war.run_if(in_state(GameState::Playing)),
            );
    }
}
