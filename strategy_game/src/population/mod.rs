pub mod growth;
pub mod mobilization;
pub mod update;

use crate::app::game_state::GameState;
use crate::app::time::DailySimulationSet;
use bevy::prelude::*;
use growth::handle_monthly_population_growth;

pub struct PopulationPlugin;

impl Plugin for PopulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            // P23-001: `economy::handle_monthly_economy`(同じEconomy Set)が
            // `update_state_population_and_employment`経由でこの後すぐ`state.population`を
            // 読むため、成長後の値を確実に使わせるよう明示的に`.before()`で固定する
            // (Setが同じというだけでは同一Set内のSystem順序は保証されない)。
            handle_monthly_population_growth
                .in_set(DailySimulationSet::Economy)
                .before(crate::economy::handle_monthly_economy)
                .run_if(in_state(GameState::Playing)),
        );
    }
}
