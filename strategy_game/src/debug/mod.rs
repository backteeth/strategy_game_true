use crate::app::game_state::GameState;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::localization::{CurrentLocale, TranslationCatalog, t, tf};
use crate::politics::reform::apply_reform_completion;
use crate::research::world_stage::WorldCivilizationState;
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use bevy::prelude::*;

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::localization::TranslationCorePlugin)
            .add_systems(
                Update,
                handle_debug_shortcuts.run_if(in_state(GameState::Playing)),
            );
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_debug_shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
    mut state_registry: ResMut<StateRegistry>,
    mut world_state: ResMut<WorldCivilizationState>,
    mut notif_writer: MessageWriter<GameNotification>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    // F1: Advance World Stage
    if keys.just_pressed(KeyCode::F1)
        && let Some(next_stage) = world_state.get_next_stage()
    {
        world_state.current_stage = next_stage;
        notif_writer.write(GameNotification {
            message: tf(
                &catalog,
                locale.0,
                "notif.debug_advanced_era",
                vec![("era", t(&catalog, locale.0, next_stage.display_name()))],
            ),
        });
    }

    // F2: Reveal All Deposits for Player
    if keys.just_pressed(KeyCode::F2)
        && let Some(cid) = player_country.0
    {
        for state in state_registry.states.iter_mut() {
            if state.owner_country_id == cid {
                for dep in state.resource_deposits.iter_mut() {
                    dep.discovered = true;
                }
            }
        }
        notif_writer.write(GameNotification {
            message: t(&catalog, locale.0, "notif.debug_revealed_deposits"),
        });
    }

    // F3: Instant Complete Current Reform
    if keys.just_pressed(KeyCode::F3)
        && let Some(cid) = player_country.0
        && let Some(country) = country_registry.get_mut(cid)
        && let Some(ref reform) = country.current_reform.clone()
    {
        apply_reform_completion(reform, &mut country.politics.values);
        country.current_reform = None;
        notif_writer.write(GameNotification {
            message: t(&catalog, locale.0, "notif.debug_reform_completed"),
        });
    }
}
