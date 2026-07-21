pub mod approval;
pub mod interest_groups;
pub mod modifiers;
pub mod reform;
pub mod values;

use crate::app::game_state::GameState;
use crate::app::time::MonthChangedMessage;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::politics::approval::calculate_interest_group_approval;
use crate::politics::interest_groups::calculate_interest_group_influence;
use crate::politics::reform::{apply_reform_completion, update_reform_progress_step};
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use bevy::prelude::*;

pub struct PoliticsPlugin;

impl Plugin for PoliticsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            handle_monthly_politics.run_if(in_state(GameState::Playing)),
        );
    }
}

fn handle_monthly_politics(
    mut month_events: MessageReader<MonthChangedMessage>,
    mut country_registry: ResMut<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    player_country: Res<PlayerCountry>,
    mut notif_writer: MessageWriter<GameNotification>,
) {
    for _event in month_events.read() {
        for country in country_registry.countries.iter_mut() {
            let country_id = country.id;

            let owned_states: Vec<&crate::state::data::StateData> = state_registry
                .states
                .iter()
                .filter(|s| s.owner_country_id == country_id)
                .collect();

            if owned_states.is_empty() {
                continue;
            }

            let raw_inf = calculate_interest_group_influence(
                country.government_type,
                country.economic_system,
                &country.politics.values,
                &owned_states,
            );

            for (ig_type, inf_val) in raw_inf {
                if let Some(ig_state) = country.politics.interest_groups.get_mut(&ig_type) {
                    ig_state.influence = inf_val;
                }
            }
            country.politics.normalize_influence();

            let avg_ls = owned_states.iter().map(|s| s.living_standard).sum::<f32>()
                / owned_states.len() as f32;
            let avg_unrest =
                owned_states.iter().map(|s| s.unrest).sum::<f32>() / owned_states.len() as f32;

            let sci_alloc = country.research_state.allocation.science;
            let mag_alloc = country.research_state.allocation.magic;

            for (&ig_type, ig_state) in country.politics.interest_groups.iter_mut() {
                ig_state.approval = calculate_interest_group_approval(
                    ig_type,
                    country.government_type,
                    country.economic_system,
                    &country.politics.values,
                    country.economic_state,
                    country.tax_rate,
                    avg_ls,
                    avg_unrest,
                    sci_alloc,
                    mag_alloc,
                );
            }

            if let Some(ref mut reform) = country.current_reform {
                update_reform_progress_step(
                    reform,
                    country.government_type,
                    &mut country.politics.interest_groups,
                );

                if reform.progress >= reform.required_progress {
                    apply_reform_completion(reform, &mut country.politics.values);

                    if player_country.0 == Some(country.id) {
                        notif_writer.write(GameNotification {
                            message: format!(
                                "Political Reform Completed: {} target reached!",
                                reform.axis.display_name()
                            ),
                        });
                    }
                    country.current_reform = None;
                }
            }
        }
    }
}
