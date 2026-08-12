pub mod ai;
pub mod army_group;
pub mod battle;
pub mod combat;
pub mod combat_calc;
pub mod data;
pub mod invasion;
pub mod movement;
pub mod pathfinding;
pub mod recruitment;
pub mod supply;
pub mod update;

#[cfg(test)]
mod tests;

use crate::app::game_state::GameState;
use crate::app::time::{DailySimulationSet, DayChangedMessage};
use crate::military::army_group::ArmyGroupRegistry;
use crate::military::battle::BattleRegistry;
use crate::military::data::MilitaryRegistry;
use crate::military::update::handle_daily_military;
use bevy::prelude::*;

pub struct MilitaryPlugin;

impl Plugin for MilitaryPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MilitaryRegistry::default())
            .insert_resource(BattleRegistry::default())
            .insert_resource(ArmyGroupRegistry::default())
            .add_systems(
                Update,
                (
                    handle_daily_military,
                    handle_daily_army_group_maintenance.after(handle_daily_military),
                )
                    .in_set(DailySimulationSet::MilitaryAction)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

/// 消滅・撃破済み師団の参照を全編成(ArmyGroup)から日次で整理する。
/// `war::frontline::handle_daily_frontline_plans`が`FrontlineRegistry::sanitize_references`を
/// 呼ぶのと同じ役割。
fn handle_daily_army_group_maintenance(
    mut day_events: MessageReader<DayChangedMessage>,
    military_registry: Res<MilitaryRegistry>,
    mut army_group_registry: ResMut<ArmyGroupRegistry>,
) {
    for _ in day_events.read() {
        army_group_registry.sanitize_references(&military_registry);
    }
}
