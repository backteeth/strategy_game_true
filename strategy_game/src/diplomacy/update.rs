use crate::app::time::{DayChangedMessage, GameDate};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::crisis::{CrisisPhase, CrisisRegistry};
use crate::diplomacy::data::DiplomacyRegistry;
use crate::localization::{CurrentLocale, TranslationCatalog, t, tf};
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
    mut crisis_registry: ResMut<CrisisRegistry>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    for _event in day_events.read() {
        // 0. 正当化の日次進行 (CountryAiより前に完了判定を行うため)
        justification_registry.process_daily_justifications(&state_registry);

        // 3. 外交危機の日次進行 (P21-010: 既存の状態遷移規則が存在しないため、
        // 接続範囲は`days_in_phase`のインクリメントのみに限定する。フェーズ自動遷移・
        // 自動終結・自動宣戦は実装しない[対象外: 新しい外交ルール]。terminal state
        // [ResolvedPeacefully/WarStarted/Cancelled]は既存実装どおり日数を進めない)。
        for crisis in crisis_registry.crises.values_mut() {
            if !matches!(
                crisis.current_phase,
                CrisisPhase::ResolvedPeacefully | CrisisPhase::WarStarted | CrisisPhase::Cancelled
            ) {
                crisis.days_in_phase += 1;
            }
        }

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
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
                    let act_name = finished_type
                        .map(|ty| t(&catalog, locale.0, ty.display_name()))
                        .unwrap_or_else(|| {
                            t(&catalog, locale.0, "notif.diplomatic_activity_fallback")
                        });

                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
                            "notif.diplomacy_finished",
                            vec![
                                ("activity", act_name),
                                ("country", other_name),
                                ("opinion", format!("{:.0}", relation.opinion)),
                            ],
                        ),
                    });
                }
            }
        }
    }
}
