use crate::app::time::DayChangedMessage;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::military::battle::BattleRegistry;
use crate::military::data::MilitaryRegistry;
use crate::state::data::StateRegistry;
use crate::war::data::{WarRegistry, WarStatus};
use crate::war::justification::WarJustificationRegistry;
use bevy::prelude::*;

pub fn handle_daily_war(
    mut day_events: MessageReader<DayChangedMessage>,
    mut state_registry: ResMut<StateRegistry>,
    mut war_registry: ResMut<WarRegistry>,
    mut justification_registry: ResMut<WarJustificationRegistry>,
    mut military_registry: ResMut<MilitaryRegistry>,
    mut battle_registry: ResMut<BattleRegistry>,
    mut diplomacy_registry: ResMut<DiplomacyRegistry>,
) {
    for event in day_events.read() {
        let current_date_str = format!("{:04}/{:02}/{:02}", event.year, event.month, event.day);

        // 1. 正当化の日次進行
        justification_registry.process_daily_justifications(&state_registry);

        // 2. 終了した戦闘勝敗の戦争データへの集計
        crate::war::combat::sync_battle_results_to_wars(&battle_registry, &mut war_registry);

        // 3. 戦勝点更新
        crate::war::war_score::process_war_score(&state_registry, &mut war_registry);

        // 4. 降伏判定と自動講和終結
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
                        );
                    }
                    crate::war::capitulation::CapitulationResult::None => {}
                }
            }
        }
    }
}
