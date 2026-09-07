pub mod ai;
pub mod army;
pub mod battle;
pub mod combat;
pub mod combat_calc;
pub mod daily_supply;
pub mod data;
pub mod demobilization;
pub mod equipment;
pub mod equipment_reinforcement;
pub mod invasion;
pub mod manpower;
pub mod movement;
pub mod pathfinding;
pub mod recruitment;
pub mod reinforcement;
pub mod supply;
pub mod supply_initialization;
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
            )
            .add_systems(
                Update,
                // P26-004: 日次補給消費・配分。`economy::handle_monthly_economy`
                // (MilitarySupplies月次生産を含む、P26-003)と同じ`Economy`Setに置き、
                // `.after()`で明示的にその直後へ固定する――月境界日には月次生産が
                // 必ず先に完了してからこの日次消費が走る(spec §6.2)。`Economy`Set全体は
                // `DailySimulationSet`の宣言順により`MilitaryAction`Set
                // (`handle_daily_military`――募兵完了・移動・戦闘)より必ず先に実行される
                // ため、「前日終値状態から需要を確定してから当日の移動・戦闘・募兵完了が
                // 起こる」という正規順序(spec §6)が、イベントreaderの偶然の登録順ではなく
                // 明示的なSystem順序制約として保証される。
                daily_supply::process_daily_military_supply_consumption
                    .in_set(DailySimulationSet::Economy)
                    .after(crate::economy::handle_monthly_economy)
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
