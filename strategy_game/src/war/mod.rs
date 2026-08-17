pub mod capitulation;
pub mod combat;
pub mod data;
pub mod frontline;
pub mod justification;
pub mod military_ai;
pub mod occupation;
pub mod peace;
pub mod update;
pub mod war_score;

#[cfg(test)]
pub mod tests;

use crate::app::game_state::GameState;
use crate::app::time::DailySimulationSet;
use crate::war::data::WarRegistry;
use crate::war::frontline::{
    FrontlineRegistry, handle_daily_frontline_plans, sync_army_frontline_references,
};
use crate::war::justification::WarJustificationRegistry;
use crate::war::military_ai::{MilitaryAiRegistry, handle_daily_military_ai};
use crate::war::update::{handle_daily_war_prep, handle_daily_war_resolution};
use bevy::prelude::*;

pub struct WarPlugin;

impl Plugin for WarPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WarRegistry::default())
            .insert_resource(WarJustificationRegistry::default())
            .insert_resource(FrontlineRegistry::default())
            .insert_resource(MilitaryAiRegistry::default())
            .add_systems(
                Update,
                (
                    handle_daily_war_prep.in_set(DailySimulationSet::WarPreparation),
                    handle_daily_war_resolution.in_set(DailySimulationSet::WarResolution),
                    handle_daily_military_ai.in_set(DailySimulationSet::MilitaryAi),
                    handle_daily_frontline_plans.in_set(DailySimulationSet::FrontlineOrders),
                )
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                sync_army_frontline_references.run_if(in_state(GameState::Playing)),
            );
    }
}
