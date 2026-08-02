use crate::app::time::{DayChangedMessage, GameDate};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::data::DiplomacyRegistry;
use crate::ui::notification::GameNotification;
use bevy::prelude::*;

pub const ACTIVITY_DURATION_DAYS: u32 = 30;
pub const COOLDOWN_DURATION_DAYS: u32 = 30;

#[allow(clippy::too_many_arguments)]
pub fn handle_daily_diplomacy(
    mut day_events: MessageReader<DayChangedMessage>,
    mut diplomacy_registry: ResMut<DiplomacyRegistry>,
    player_country: Res<PlayerCountry>,
    mut notif_writer: MessageWriter<GameNotification>,
    country_registry: Res<CountryRegistry>,
    date: Res<GameDate>,
    mut justification_registry: ResMut<crate::war::justification::WarJustificationRegistry>,
    state_registry: Res<crate::state::data::StateRegistry>,
) {
    for _event in day_events.read() {
        // 0. 正当化の日次進行 (CountryAiより前に完了判定を行うため)
        justification_registry.process_daily_justifications(&state_registry);

        for (&key, relation) in diplomacy_registry.relations.iter_mut() {
            // 1. クールダウン減算
            let mut expired_cooldowns = Vec::new();
            for (&cid, cd) in relation.cooldowns.iter_mut() {
                if *cd > 0 {
                    *cd -= 1;
                }
                if *cd == 0 {
                    expired_cooldowns.push(cid);
                }
            }
            for cid in expired_cooldowns {
                relation.cooldowns.remove(&cid);
            }

            // 2. 実行中外交活動の進行
            let mut activity_finished = false;
            let mut finished_type = None;
            let mut initiator_cid = None;

            if let Some(ref mut act) = relation.active_activity {
                relation.opinion =
                    (relation.opinion + act.daily_opinion_change).clamp(-100.0, 100.0);
                if act.days_remaining > 0 {
                    act.days_remaining -= 1;
                }
                if act.days_remaining == 0 {
                    activity_finished = true;
                    finished_type = Some(act.activity_type);
                    initiator_cid = Some(act.initiator);
                }
            }

            if activity_finished {
                relation.active_activity = None;
                if let Some(init_id) = initiator_cid {
                    relation.cooldowns.insert(init_id, COOLDOWN_DURATION_DAYS);
                }
                relation.last_updated_date = date.display();

                if player_country.0 == Some(key.0) || player_country.0 == Some(key.1) {
                    let other_id = if player_country.0 == Some(key.0) {
                        key.1
                    } else {
                        key.0
                    };
                    let other_name = country_registry
                        .get(other_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown");
                    let act_name = finished_type
                        .map(|t| t.display_name())
                        .unwrap_or("Diplomatic Activity");

                    notif_writer.write(GameNotification {
                        message: format!(
                            "Diplomacy Finished: {} with {} (Opinion: {:.0})",
                            act_name, other_name, relation.opinion
                        ),
                    });
                }
            }
        }
    }
}
