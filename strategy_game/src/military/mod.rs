pub mod ai;
pub mod army;
pub mod battle;
pub mod combat;
pub mod combat_calc;
pub mod data;
pub mod demobilization;
pub mod equipment;
pub mod invasion;
pub mod manpower;
pub mod movement;
pub mod pathfinding;
pub mod recruitment;
pub mod reinforcement;
pub mod supply;
pub mod update;

#[cfg(test)]
mod tests;

use crate::app::game_state::GameState;
use crate::app::time::{DailySimulationSet, DayChangedMessage};
use crate::military::ai::handle_monthly_ai_recruitment;
use crate::military::army::{ArmyRegistry, SelectedArmy};
use crate::military::battle::BattleRegistry;
use crate::military::data::MilitaryRegistry;
use crate::military::update::handle_daily_military;
use bevy::prelude::*;

pub struct MilitaryPlugin;

impl Plugin for MilitaryPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MilitaryRegistry::default())
            .insert_resource(BattleRegistry::default())
            .insert_resource(ArmyRegistry::default())
            .insert_resource(SelectedArmy::default())
            .add_systems(
                Update,
                (
                    handle_daily_military,
                    handle_daily_army_maintenance.after(handle_daily_military),
                )
                    .in_set(DailySimulationSet::MilitaryAction)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                // P23-009: 月次AI募兵判断。`DailySimulationSet::CountryAi`は
                // `Economy`より後・`UiUpdate`より前に位置するため、月次人的資源回復・
                // 州人口集計が確定した後、UI表示が読む前に実行される
                // (Set順序自体は`app::time::DailySimulationSet`の宣言順が保証する)。
                handle_monthly_ai_recruitment
                    .in_set(DailySimulationSet::CountryAi)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

/// 消滅・撃破済み師団の参照を全編成(Army)から日次で整理する。
/// `war::frontline::handle_daily_frontline_plans`が`FrontlineRegistry::sanitize_references`を
/// 呼ぶのと同じ役割。
fn handle_daily_army_maintenance(
    mut day_events: MessageReader<DayChangedMessage>,
    military_registry: Res<MilitaryRegistry>,
    mut army_registry: ResMut<ArmyRegistry>,
) {
    for _ in day_events.read() {
        army_registry.sanitize_references(&military_registry);
    }
}
