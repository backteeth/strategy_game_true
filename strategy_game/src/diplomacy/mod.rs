pub mod ai;
pub mod ai_claim_generation;
pub mod ai_crisis_start;
pub mod claims;
pub mod crisis;
pub mod crisis_response;
pub mod data;
pub mod player_demand_alert;
pub mod proposal;
pub mod update;

#[cfg(test)]
mod tests;

use crate::app::game_state::GameState;
use crate::app::time::DailySimulationSet;
use crate::diplomacy::ai_claim_generation::handle_monthly_ai_claim_generation;
use crate::diplomacy::ai_crisis_start::handle_monthly_ai_crisis_start;
use crate::diplomacy::claims::ClaimRegistry;
use crate::diplomacy::crisis::CrisisRegistry;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::diplomacy::player_demand_alert::{
    SeenPlayerTargetedCrisisIds, pause_on_new_crisis_targeting_player,
};
use crate::diplomacy::update::handle_daily_diplomacy;
use bevy::prelude::*;

pub struct DiplomacyPlugin;

impl Plugin for DiplomacyPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::localization::TranslationCorePlugin)
            .insert_resource(DiplomacyRegistry::default())
            .insert_resource(ClaimRegistry::default())
            .insert_resource(CrisisRegistry::default())
            .insert_resource(SeenPlayerTargetedCrisisIds::default())
            .add_systems(
                Update,
                handle_daily_diplomacy
                    .in_set(DailySimulationSet::Diplomacy)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                // P21-017: AI国家による既存Claimからの月次Crisis開始評価。
                // `DailySimulationSet::CountryAi`はDiplomacyより後に実行されるため、
                // 開始と同じフレームではP21-012/015の日次評価が既に完了済みであり、
                // 新規Crisisは次の`DayChangedMessage`から通常どおり処理される。
                //
                // P21-017(二次レビュー対応): 同じSet内の`handle_daily_country_ai`
                // (`process_war_preparation_ai`を含む、Claim/Crisisと独立に生の正当化を
                // 開始し得る既存の戦争準備AI)より必ず先に実行されるよう明示的に
                // `.before()`で固定する。これにより、同一フレーム・同一AI国家に対して
                // 両者が競合する場合、常にCrisis開始が先に成立し、`process_war_
                // preparation_ai`側の`has_unresolved_crisis_initiated_by`チェックが
                // その日のうちに反映される(順序への依存を無くし、決定論を保つ)。
                handle_monthly_ai_crisis_start
                    .in_set(DailySimulationSet::CountryAi)
                    .before(crate::country::country_ai::handle_daily_country_ai)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                // P21-018: AI国家による月次Claim生成。§9の要求どおり、通常戦争準備AI・
                // AI宣戦布告を含む`handle_daily_country_ai`より必ず後に実行されるよう
                // 明示的に`.after()`で固定する(同じフレームで、その日のうちに正当化を
                // 開始したりWarへ入ったりしたAIは、既にResourceへ反映済みの結果を見て
                // Claim生成をスキップできる)。`handle_monthly_ai_crisis_start`は
                // `handle_daily_country_ai`より前に実行されるため、この`.after()`だけで
                // 「P21-017 → 通常戦争準備AI/AI宣戦 → P21-018」という全順序が
                // (Bevyのスケジューラが解決する推移的な順序制約として)保証され、
                // System登録順そのものへは依存しない。
                handle_monthly_ai_claim_generation
                    .in_set(DailySimulationSet::CountryAi)
                    .after(handle_monthly_ai_crisis_start)
                    .after(crate::country::country_ai::handle_daily_country_ai)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                // P21-018フォローアップ: プレイヤーが新しくCrisis要求(DemandSent)の
                // 対象になった瞬間、ゲーム速度に関わらず必ず気づけるよう自動一時停止する
                // (`DailySimulationSet`のシミュレーション連鎖とは独立した、UI寄りの
                // 監視Systemのため`in_set`は指定しない。新規Crisisが今フレーム/次フレーム
                // どちらで検出されても実害はない)。
                pause_on_new_crisis_targeting_player.run_if(in_state(GameState::Playing)),
            );
    }
}
