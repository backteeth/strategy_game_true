pub mod economic_state;
pub mod factory_starvation;
pub mod finance;
pub mod iron_shortage;
pub mod production;
pub mod resources;
pub mod supply_balance;

use crate::app::game_state::GameState;
use crate::app::time::{DailySimulationSet, DayChangedMessage, MonthChangedMessage};
use crate::building::construction::ConstructionStatus;
use crate::building::data::BuildingRegistry;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::economy::economic_state::{EconomicMetrics, calculate_economic_state};
use crate::economy::finance::{process_country_finance, update_state_welfare_and_unrest};
use crate::economy::production::process_country_production_with_outcome;
use crate::economy::supply_balance::MilitarySupplyProductionLedger;
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
            .insert_resource(MilitarySupplyProductionLedger::default())
            .add_systems(
                OnEnter(GameState::CountrySelection),
                clear_military_supply_production_ledger,
            )
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
    mut supply_ledger: Option<ResMut<MilitarySupplyProductionLedger>>,
    mut ai_registry: Option<ResMut<crate::country::country_ai::CountryAiRegistry>>,
) {
    for event in month_events.read() {
        for country in country_registry.countries.iter_mut() {
            let country_id = country.id;

            let mut owned_states: Vec<&mut crate::state::data::StateData> = state_registry
                .states
                .iter_mut()
                .filter(|s| s.owner_country_id == country_id)
                .collect();

            if owned_states.is_empty() {
                // 生産を行わない国家に古い実績を残さない(表示専用)。
                if let Some(ledger) = supply_ledger.as_mut() {
                    ledger.remove(country_id);
                }
                // P29-005: 生産を行わない国の履歴は、次に生産を再開した月から計測し直す。
                if let Some(reg) = ai_registry.as_mut() {
                    reg.supply_production_history.remove(&country_id);
                    reg.iron_shortage_history.remove(&country_id);
                    reg.factory_starvation_history.remove(&country_id);
                }
                continue;
            }

            // P23-002: 現在の所有州人口(既にP23-001の月次成長を反映済み)を基に、
            // 動員済み人的資源(mobilized_manpower)を州の人口比で按分する。永続化せず、
            // 毎月このSystem内で再導出する派生値であり、他国の州へは一切漏れない
            // (owned_statesは現在この国家が所有する州だけ)。
            let population_pairs: Vec<(crate::common::StateId, u64)> =
                owned_states.iter().map(|s| (s.id, s.population)).collect();
            let mobilized_burdens = crate::population::mobilization::apportion_mobilized_manpower(
                country.mobilized_manpower,
                &population_pairs,
            );

            for state in owned_states.iter_mut() {
                let burden = mobilized_burdens.get(&state.id).copied().unwrap_or(0);
                update_state_population_and_employment(state, &building_registry, burden);
                update_state_logistics(state, &building_registry);
            }

            // P29-009: この月次生産の時点の Mine レベル合計(Iron 不足の履歴の「設備」の識別に使う)。
            let mine_levels_total: u32 = owned_states
                .iter()
                .map(|s| s.building_level(crate::building::data::BuildingType::Mine))
                .sum();
            let is_bankrupt = country.treasury < 0.0;
            let outcome = process_country_production_with_outcome(
                &mut owned_states,
                &mut country.stockpile,
                &building_registry,
                is_bankrupt,
            );
            country.science_research_capacity = outcome.science;
            country.magic_research_capacity = outcome.magic;
            // P29-002: 表示専用。実際に備蓄へ反映したMilitarySuppliesの量を、この月次生産の
            // 年月と共に記録する(ゲーム状態・Save V1には影響しない)。
            if let Some(ledger) = supply_ledger.as_mut() {
                ledger.record(
                    country_id,
                    event.year,
                    event.month,
                    outcome.military_supplies_produced,
                );
            }

            // P29-005: AI 募兵の能力推定用の履歴(永続)。UI 表示用 ledger とは別に、実際の産出
            // (units)を対象年月とともに記録する。集計失敗は None(0 と区別)。プレイヤー国は対象外。
            if let Some(reg) = ai_registry.as_mut()
                && player_country.0 != Some(country_id)
            {
                let units = outcome
                    .military_supplies_produced
                    .and_then(|v| crate::economy::resources::military_supplies_value_to_units(v).ok());
                reg.supply_production_history
                    .entry(country_id)
                    .or_default()
                    .record(event.year, event.month, units);
            }

            // P29-009: 建設 AI 用の Iron 不足の履歴(永続)。MilitaryFactory の Iron 未充足の要求量を、
            // Mine レベル合計とともに記録する。プレイヤー国は対象外。
            if let Some(reg) = ai_registry.as_mut()
                && player_country.0 != Some(country_id)
            {
                reg.iron_shortage_history
                    .entry(country_id)
                    .or_default()
                    .record(
                        event.year,
                        event.month,
                        Some(outcome.iron_unmet_military),
                        mine_levels_total,
                    );
            }

            // P29-010: Factory 増設抑止(B2)用の履歴(永続)。全 Factory entry が Iron 不足で稼働率 0.1
            // 未満の月を、Factory の設備署名とともに記録する。プレイヤー国は対象外。
            if let Some(reg) = ai_registry.as_mut()
                && player_country.0 != Some(country_id)
            {
                let entries: Vec<(usize, u32)> = owned_states
                    .iter()
                    .map(|s| (s.id.0, s.building_level(crate::building::data::BuildingType::Factory)))
                    .filter(|(_, l)| *l > 0)
                    .collect();
                let starved = outcome.factory_entries > 0
                    && outcome.factory_iron_starved_entries == outcome.factory_entries;
                reg.factory_starvation_history
                    .entry(country_id)
                    .or_default()
                    .record(
                        event.year,
                        event.month,
                        crate::economy::factory_starvation::factory_signature(&entries),
                        starved,
                    );
            }

            process_country_finance(country, &mut owned_states, &building_registry);

            let has_food = country.stockpile.get(ResourceType::Food) > 0.0;
            for state in owned_states.iter_mut() {
                update_state_welfare_and_unrest(state, country.tax_rate, has_food, is_bankrupt);
            }

            // P23-003: 動員負担控除後の民間労働力(employed+unemployed、月次雇用計算が
            // 確定させた実値)を分母にする。total_workforce()(人口とworkforce_ratioから
            // 動員を考慮せず再導出する潜在労働力)を使うと、動員中でも見かけ上の雇用率が
            // 下がらず、この値から導出されるeconomic_state表示指標が実態とずれていた。
            let total_wf: u64 = owned_states.iter().map(|s| s.civilian_workforce()).sum();
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

/// P29-002: New Game(国家選択画面への進入)で、前のゲームの月次生産実績(表示専用)を
/// 残さない。ロード成功時の消去は`save::runtime::handle_load_requests`が行う。
fn clear_military_supply_production_ledger(
    mut ledger: ResMut<MilitarySupplyProductionLedger>,
    ai_registry: Option<ResMut<crate::country::country_ai::CountryAiRegistry>>,
) {
    ledger.clear();
    // P29-005: AI 募兵の能力推定用の履歴(永続)も、New Game では前セッションの値を残さない。
    // ロード成功時は、Save に保存された履歴で置き換わる(旧 Save では空)。
    if let Some(mut reg) = ai_registry {
        reg.supply_production_history.clear();
        reg.iron_shortage_history.clear();
        reg.factory_starvation_history.clear();
    }
}
