pub mod allocation;
pub mod data;
pub mod progress;
pub mod world_stage;

use crate::app::game_state::GameState;
use crate::app::time::{GameDate, MonthChangedMessage};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::research::data::{TechnologyField, TechnologyRegistry};
use crate::research::progress::calculate_field_monthly_points;
use crate::research::world_stage::WorldCivilizationState;
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use bevy::prelude::*;

pub struct ResearchPlugin;

impl Plugin for ResearchPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TechnologyRegistry::default())
            .insert_resource(WorldCivilizationState::default())
            .add_systems(
                Update,
                (handle_monthly_research, handle_npc_auto_research)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn handle_monthly_research(
    mut month_events: MessageReader<MonthChangedMessage>,
    mut country_registry: ResMut<CountryRegistry>,
    mut state_registry: ResMut<StateRegistry>,
    tech_registry: Res<TechnologyRegistry>,
    mut world_state: ResMut<WorldCivilizationState>,
    player_country: Res<PlayerCountry>,
    mut notif_writer: MessageWriter<GameNotification>,
    date: Res<GameDate>,
) {
    for _event in month_events.read() {
        let mut newly_completed_milestones: Vec<(usize, String)> = Vec::new();

        for country in country_registry.countries.iter_mut() {
            let country_id_raw = country.id.0;

            let mil_bonus = 0.0;
            let fusion_mult = 1.0;

            let (_, points_arr) = calculate_field_monthly_points(
                country.science_research_capacity,
                country.magic_research_capacity,
                5.0,
                &country.research_state.allocation,
                mil_bonus,
                fusion_mult,
            );

            let mut finished_fields = Vec::new();

            for (&field, in_prog) in country.research_state.in_progress.iter_mut() {
                let monthly_pts = match field {
                    TechnologyField::Science => points_arr[0],
                    TechnologyField::Magic => points_arr[1],
                    TechnologyField::Military => points_arr[2],
                    TechnologyField::Fusion => points_arr[3],
                };

                in_prog.progress += monthly_pts;

                if in_prog.progress >= in_prog.cost {
                    finished_fields.push((field, in_prog.tech_id.clone()));
                }
            }

            for (field, tech_id) in finished_fields {
                country.research_state.in_progress.remove(&field);
                country
                    .research_state
                    .completed_technologies
                    .insert(tech_id.clone());

                if let Some(def) = tech_registry.get(&tech_id) {
                    if def.is_world_milestone {
                        newly_completed_milestones.push((country_id_raw, tech_id.clone()));
                    }

                    apply_technology_effects_to_country(
                        country,
                        def,
                        &mut state_registry,
                        player_country.0 == Some(country.id),
                        &mut notif_writer,
                    );

                    if player_country.0 == Some(country.id) {
                        notif_writer.write(GameNotification {
                            message: format!(
                                "Research Complete: {} ({})",
                                def.name,
                                field.display_name()
                            ),
                        });
                    }
                }
            }
        }

        if let Some(next_stage) = world_state.get_next_stage() {
            if let Some(stage_def) = world_state.stage_definitions.get(&next_stage).cloned() {
                for (cid, tech_id) in newly_completed_milestones {
                    if stage_def.milestone_technologies.contains(&tech_id) {
                        let set = world_state
                            .milestone_countries
                            .entry(next_stage)
                            .or_default();
                        set.insert(cid);
                    }
                }

                let count = world_state
                    .milestone_countries
                    .get(&next_stage)
                    .map(|s| s.len())
                    .unwrap_or(0);

                if count >= stage_def.required_country_count {
                    world_state.current_stage = next_stage;
                    world_state.last_advanced_date = date.display();
                    notif_writer.write(GameNotification {
                        message: format!(
                            "WORLD EVENT: Entered new World Era [{}]!",
                            next_stage.display_name()
                        ),
                    });
                }
            }
        }
    }
}

fn handle_npc_auto_research(
    mut month_events: MessageReader<MonthChangedMessage>,
    mut country_registry: ResMut<CountryRegistry>,
    tech_registry: Res<TechnologyRegistry>,
    world_state: Res<WorldCivilizationState>,
    player_country: Res<PlayerCountry>,
) {
    for _event in month_events.read() {
        for country in country_registry.countries.iter_mut() {
            if player_country.0 == Some(country.id) {
                continue;
            }

            for field in TechnologyField::ALL {
                if country.research_state.in_progress.contains_key(&field) {
                    continue;
                }

                let mut candidates: Vec<_> = tech_registry
                    .definitions
                    .values()
                    .filter(|def| def.field == field)
                    .filter(|def| {
                        !country
                            .research_state
                            .completed_technologies
                            .contains(&def.id)
                    })
                    .filter(|def| def.minimum_world_stage <= world_state.current_stage)
                    .filter(|def| {
                        def.prerequisites
                            .iter()
                            .all(|pre| country.research_state.completed_technologies.contains(pre))
                    })
                    .collect();

                candidates.sort_by_key(|def| def.ui_order);

                if let Some(chosen) = candidates.first() {
                    country.research_state.in_progress.insert(
                        field,
                        crate::research::allocation::InProgressTech {
                            tech_id: chosen.id.clone(),
                            progress: 0.0,
                            cost: chosen.cost,
                        },
                    );
                }
            }
        }
    }
}

fn apply_technology_effects_to_country(
    country: &mut crate::country::CountryData,
    def: &crate::research::data::TechnologyDefinition,
    state_registry: &mut StateRegistry,
    is_player: bool,
    notif_writer: &mut MessageWriter<GameNotification>,
) {
    for effect in &def.effects {
        if let crate::research::data::TechnologyEffect::RevealResourceType(res_type) = effect {
            for state in state_registry.states.iter_mut() {
                if state.owner_country_id == country.id {
                    for dep in state.resource_deposits.iter_mut() {
                        if dep.resource_type == *res_type && !dep.discovered {
                            dep.discovered = true;
                            if is_player {
                                notif_writer.write(GameNotification {
                                    message: format!(
                                        "Resource Discovered: {} in {}",
                                        res_type.display_name(),
                                        state.name
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}
