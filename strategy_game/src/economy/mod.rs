pub mod economic_state;
pub mod finance;
pub mod production;
pub mod resources;

use crate::app::game_state::GameState;
use crate::app::time::{DailySimulationSet, DayChangedMessage, MonthChangedMessage};
use crate::building::construction::ConstructionStatus;
use crate::building::data::BuildingRegistry;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::economy::economic_state::{EconomicMetrics, calculate_economic_state};
use crate::economy::finance::{process_country_finance, update_state_welfare_and_unrest};
use crate::economy::production::process_country_production;
use crate::economy::resources::ResourceType;
use crate::localization::{CurrentLocale, TranslationCatalog, t, tf};
use crate::logistics::update::update_state_logistics;
use crate::population::update::update_state_population_and_employment;
use crate::state::data::StateRegistry;
use crate::ui::notification::{GameNotification, NotificationHistory};
use bevy::prelude::*;

pub struct EconomyPlugin;

impl Plugin for EconomyPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::localization::TranslationCorePlugin)
            .insert_resource(NotificationHistory::default())
            .add_message::<GameNotification>()
            .add_systems(
                Update,
                (handle_daily_construction, handle_monthly_economy)
                    .chain()
                    .in_set(DailySimulationSet::Economy)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                handle_notifications
                    .in_set(DailySimulationSet::UiUpdate)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

/// 日次建設キュー進捗システム
#[allow(clippy::too_many_arguments)]
pub fn handle_daily_construction(
    mut day_events: MessageReader<DayChangedMessage>,
    mut country_registry: ResMut<CountryRegistry>,
    mut state_registry: ResMut<StateRegistry>,
    _building_registry: Res<BuildingRegistry>,
    player_country: Res<PlayerCountry>,
    mut notif_writer: MessageWriter<GameNotification>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    for _event in day_events.read() {
        for country in country_registry.countries.iter_mut() {
            let mut completed_indices = Vec::new();

            for (idx, item) in country.construction_queue.iter_mut().enumerate() {
                item.status = ConstructionStatus::InProgress;
                item.progress += 1.0;

                if item.progress >= item.required_progress {
                    completed_indices.push(idx);

                    if let Some(state) = state_registry.get_mut(item.state_id) {
                        let current = state.building_level(item.building_type);
                        state.buildings.insert(item.building_type, current + 1);

                        if Some(country.id) == player_country.0 {
                            notif_writer.write(GameNotification {
                                message: tf(
                                    &catalog,
                                    locale.0,
                                    "notif.build_complete",
                                    vec![
                                        (
                                            "building",
                                            t(
                                                &catalog,
                                                locale.0,
                                                item.building_type.display_name(),
                                            ),
                                        ),
                                        ("state", state.name.clone()),
                                        ("level", (current + 1).to_string()),
                                    ],
                                ),
                            });
                        }
                    }
                }
            }

            for idx in completed_indices.into_iter().rev() {
                country.construction_queue.remove(idx);
            }
        }
    }
}

/// 月次全経済計算システム
#[allow(clippy::too_many_arguments)]
pub fn handle_monthly_economy(
    mut month_events: MessageReader<MonthChangedMessage>,
    mut country_registry: ResMut<CountryRegistry>,
    mut state_registry: ResMut<StateRegistry>,
    building_registry: Res<BuildingRegistry>,
    player_country: Res<PlayerCountry>,
    mut notif_writer: MessageWriter<GameNotification>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    for _event in month_events.read() {
        for country in country_registry.countries.iter_mut() {
            let country_id = country.id;

            let mut owned_states: Vec<&mut crate::state::data::StateData> = state_registry
                .states
                .iter_mut()
                .filter(|s| s.owner_country_id == country_id)
                .collect();

            if owned_states.is_empty() {
                continue;
            }

            for state in owned_states.iter_mut() {
                update_state_population_and_employment(state, &building_registry);
                update_state_logistics(state, &building_registry);
            }

            let is_bankrupt = country.treasury < 0.0;
            let (sci, mag) = process_country_production(
                &mut owned_states,
                &mut country.stockpile,
                &building_registry,
                is_bankrupt,
            );
            country.science_research_capacity = sci;
            country.magic_research_capacity = mag;

            process_country_finance(country, &mut owned_states, &building_registry);

            let has_food = country.stockpile.get(ResourceType::Food) > 0.0;
            for state in owned_states.iter_mut() {
                update_state_welfare_and_unrest(state, country.tax_rate, has_food, is_bankrupt);
            }

            let total_wf: u64 = owned_states.iter().map(|s| s.total_workforce()).sum();
            let total_emp: u64 = owned_states.iter().map(|s| s.employed_workforce).sum();

            let emp_rate = if total_wf > 0 {
                total_emp as f32 / total_wf as f32
            } else {
                1.0
            };

            let avg_ls: f32 = if !owned_states.is_empty() {
                owned_states.iter().map(|s| s.living_standard).sum::<f32>()
                    / owned_states.len() as f32
            } else {
                50.0
            };

            let avg_log: f32 = if !owned_states.is_empty() {
                owned_states.iter().map(|s| s.logistics_ratio).sum::<f32>()
                    / owned_states.len() as f32
            } else {
                1.0
            };

            let avg_op: f32 = if !owned_states.is_empty() {
                let mut sum_op = 0.0f32;
                let mut count = 0;
                for s in owned_states.iter() {
                    for &op in s.building_operation_ratios.values() {
                        sum_op += op;
                        count += 1;
                    }
                }
                if count > 0 {
                    sum_op / count as f32
                } else {
                    1.0
                }
            } else {
                1.0
            };

            let metrics = EconomicMetrics {
                employment_rate: emp_rate,
                avg_living_standard: avg_ls,
                avg_logistics_ratio: avg_log,
                monthly_balance: country.monthly_balance,
                avg_building_operation: avg_op,
                food_stockpile: country.stockpile.get(ResourceType::Food),
            };

            country.economic_state = calculate_economic_state(&metrics);

            if Some(country.id) == player_country.0 {
                if country.monthly_balance < 0.0 {
                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
                            "notif.balance_negative",
                            vec![("balance", format!("{:.1}", country.monthly_balance))],
                        ),
                    });
                }
                if is_bankrupt {
                    notif_writer.write(GameNotification {
                        message: t(&catalog, locale.0, "notif.bankrupt"),
                    });
                }
                if country.stockpile.get(ResourceType::Food) <= 0.0 {
                    notif_writer.write(GameNotification {
                        message: t(&catalog, locale.0, "notif.food_shortage"),
                    });
                }
            }
        }
    }
}

fn handle_notifications(
    mut notif_reader: MessageReader<GameNotification>,
    mut history: ResMut<NotificationHistory>,
) {
    for notif in notif_reader.read() {
        history.add(notif.message.clone());
        info!("[Notification] {}", notif.message);
    }
}
