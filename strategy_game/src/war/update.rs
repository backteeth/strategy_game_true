use crate::app::time::DayChangedMessage;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::military::battle::BattleRegistry;
use crate::military::data::MilitaryRegistry;
use crate::state::data::StateRegistry;
use crate::war::data::{WarRegistry, WarStatus};
use crate::war::frontline::FrontlineRegistry;
use bevy::prelude::*;

/// 戦争前処理システム (WarPreparation Set)
/// 宣戦布告等で変更された全前線の初期化および最新状態同期を行う
pub fn handle_daily_war_prep(
    mut day_events: MessageReader<DayChangedMessage>,
    state_registry: Res<StateRegistry>,
    war_registry: Res<WarRegistry>,
    military_registry: Res<MilitaryRegistry>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
) {
    for _event in day_events.read() {
        // 全前線の最新状態再計算・初期化同期
        crate::war::frontline::update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
    }
}

/// 戦争解決・後処理システム (WarResolution Set)
/// 戦闘結果の集計、戦勝点更新、降伏判定・自動講和、戦後整理、AI dirty化を行う
#[allow(clippy::too_many_arguments)]
pub fn handle_daily_war_resolution(
    mut day_events: MessageReader<DayChangedMessage>,
    mut state_registry: ResMut<StateRegistry>,
    mut war_registry: ResMut<WarRegistry>,
    mut military_registry: ResMut<MilitaryRegistry>,
    mut battle_registry: ResMut<BattleRegistry>,
    mut diplomacy_registry: ResMut<DiplomacyRegistry>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
    mut ai_registry: ResMut<crate::war::military_ai::MilitaryAiRegistry>,
    mut country_ai_registry: ResMut<crate::country::country_ai::CountryAiRegistry>,
) {
    for event in day_events.read() {
        let current_date_str = format!("{:04}/{:02}/{:02}", event.year, event.month, event.day);

        // 1. 終了した戦闘勝敗の戦争データへの集計
        crate::war::combat::sync_battle_results_to_wars(&battle_registry, &mut war_registry);

        // 2. 戦勝点更新
        crate::war::war_score::process_war_score(&state_registry, &mut war_registry);

        // 3. 降伏判定と自動講和終結
        let active_war_ids: Vec<crate::common::WarId> = war_registry
            .wars
            .values()
            .filter(|w| w.status == WarStatus::Active)
            .map(|w| w.id)
            .collect();

        for war_id in active_war_ids {
            if let Some(war) = war_registry.wars.get(&war_id) {
                let cap = crate::war::capitulation::evaluate_war_capitulation(
                    war,
                    &state_registry,
                    &military_registry,
                );

                match cap {
                    crate::war::capitulation::CapitulationResult::DefenderCapitulated => {
                        let _ = crate::war::peace::execute_peace_settlement(
                            war_id,
                            crate::war::peace::PeaceTerm::CedeWarGoalRegion,
                            "Defender Capitulation",
                            &current_date_str,
                            &mut state_registry,
                            &mut war_registry,
                            &mut military_registry,
                            &mut battle_registry,
                            &mut diplomacy_registry,
                            &mut frontline_registry,
                        );
                    }
                    crate::war::capitulation::CapitulationResult::AttackerCapitulated => {
                        let _ = crate::war::peace::execute_peace_settlement(
                            war_id,
                            crate::war::peace::PeaceTerm::AttackerConcedes,
                            "Attacker Capitulation",
                            &current_date_str,
                            &mut state_registry,
                            &mut war_registry,
                            &mut military_registry,
                            &mut battle_registry,
                            &mut diplomacy_registry,
                            &mut frontline_registry,
                        );
                    }
                    crate::war::capitulation::CapitulationResult::None => {}
                }
            }
        }

        // 4. 無効前線・陸軍参照の整理と前線の最新状態再計算
        crate::war::frontline::update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        // 5. 軍事AI・国家AI作戦状態の dirty 化
        ai_registry.mark_all_dirty();
        country_ai_registry.mark_all_dirty();
    }
}
