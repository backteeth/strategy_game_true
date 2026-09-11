pub mod ai_change_tracking;
pub mod capitulation;
pub mod combat;
pub mod data;
pub mod frontline;
pub mod justification;
pub mod military_ai;
pub mod occupation;
pub mod offensive_supply_gate;
pub mod peace;
pub mod update;
pub mod war_score;

#[cfg(test)]
pub mod tests;

use crate::app::game_state::GameState;
use crate::app::time::DailySimulationSet;
use crate::war::ai_change_tracking::AiChangeSnapshot;
use crate::war::data::WarRegistry;
use crate::war::frontline::{
    FrontlineRegistry, handle_daily_frontline_plans, sync_army_frontline_references,
};
use crate::war::justification::WarJustificationRegistry;
use crate::war::military_ai::{MilitaryAiRegistry, handle_daily_military_ai};
use crate::war::update::{
    ReportedPeaceSettlementErrors, handle_daily_war_prep, handle_daily_war_resolution,
    mark_ai_registries_dirty_on_change,
};
use bevy::prelude::*;

pub struct WarPlugin;

impl Plugin for WarPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WarRegistry::default())
            .insert_resource(WarJustificationRegistry::default())
            .insert_resource(FrontlineRegistry::default())
            .insert_resource(MilitaryAiRegistry::default())
            .insert_resource(AiChangeSnapshot::default())
            .insert_resource(ReportedPeaceSettlementErrors::default())
            .add_systems(
                Update,
                (
                    handle_daily_war_prep.in_set(DailySimulationSet::WarPreparation),
                    handle_daily_war_resolution.in_set(DailySimulationSet::WarResolution),
                    // P21-020: `handle_daily_war_resolution`から分離(Bevyのfunction-system
                    // 引数上限のため)。同一日内の実行順(旧実装ではこのSystemの内容が
                    // 講和処理の後に実行されていた)を保つため明示的に`.after()`で固定する。
                    // P26-008-PRE-002: 無条件の`mark_ai_registries_dirty_daily`から、
                    // 州支配・戦争参加の変化だけを検出する`mark_ai_registries_dirty_
                    // on_change`へ置き換えた(`war::ai_change_tracking`参照)。
                    // `handle_daily_war_resolution`の後に置く理由は変わらない
                    // (講和・国家消滅・州所有権移転が確定した後の状態を比較するため)。
                    mark_ai_registries_dirty_on_change
                        .in_set(DailySimulationSet::WarResolution)
                        .after(handle_daily_war_resolution),
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
