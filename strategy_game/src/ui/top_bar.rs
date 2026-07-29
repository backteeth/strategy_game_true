use crate::app::game_state::GameState;
use crate::app::time::{GameDate, GamePaused, GameSpeed};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::research::world_stage::WorldCivilizationState;
use crate::state::data::StateRegistry;
use crate::ui::state_panel::format_population;
use bevy::prelude::*;

#[derive(Component)]
pub struct TopBarRoot;

#[derive(Component)]
pub struct TopBarPlayerInfoText;

#[derive(Component)]
pub struct TopBarDateText;

#[derive(Component)]
pub struct SpeedButton(pub u8);

#[derive(Component)]
pub struct PauseButton;

pub struct TopBarPlugin;

impl Plugin for TopBarPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), setup_top_bar)
            .add_systems(
                Update,
                (
                    update_top_bar_player_info,
                    update_top_bar_date,
                    handle_speed_buttons,
                    handle_pause_button,
                )
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(OnExit(GameState::Playing), cleanup_top_bar);
    }
}

fn setup_top_bar(mut commands: Commands) {
    commands
        .spawn((
            TopBarRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Px(40.0),
                padding: UiRect::horizontal(Val::Px(16.0)),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.1, 0.1, 0.15, 0.95)),
        ))
        .with_children(|root| {
            root.spawn((
                TopBarPlayerInfoText,
                Text::new(""),
                TextColor(Color::srgb(0.9, 0.9, 0.9)),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
            ));

            root.spawn((Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            },))
                .with_children(|right_panel| {
                    right_panel.spawn((
                        TopBarDateText,
                        Text::new("1800/01/01"),
                        TextColor(Color::srgb(0.9, 0.9, 0.9)),
                        TextFont {
                            font_size: FontSize::Px(14.0),
                            ..default()
                        },
                    ));

                    right_panel
                        .spawn((
                            PauseButton,
                            Button,
                            Node {
                                padding: UiRect::all(Val::Px(6.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0)),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new("||"),
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(12.0),
                                    ..default()
                                },
                            ));
                        });

                    for i in 1..=4 {
                        right_panel
                            .spawn((
                                SpeedButton(i),
                                Button,
                                Node {
                                    padding: UiRect::all(Val::Px(6.0)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0)),
                            ))
                            .with_children(|btn| {
                                btn.spawn((
                                    Text::new(format!(">{}", i)),
                                    TextColor(Color::WHITE),
                                    TextFont {
                                        font_size: FontSize::Px(12.0),
                                        ..default()
                                    },
                                ));
                            });
                    }
                });
        });
}

fn update_top_bar_player_info(
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    world_state: Res<WorldCivilizationState>,
    mut text_query: Query<&mut Text, With<TopBarPlayerInfoText>>,
) {
    let Ok(mut text) = text_query.single_mut() else {
        return;
    };

    if let Some(country) = player_country.0.and_then(|id| country_registry.get(id)) {
        let id = country.id;
        let total_pop: u64 = state_registry
            .states
            .iter()
            .filter(|s| s.owner_country_id == id)
            .map(|s| s.population)
            .sum();

        let active_techs = country.research_state.in_progress.len();
        let reform_str = if let Some(ref r) = country.current_reform {
            format!("Reform: {:.0}%", (r.progress / r.required_progress) * 100.0)
        } else {
            "Reform: None".to_string()
        };

        let info = format!(
            "{} | Pop: {} | Treasury: {:.0} G | Era: {} | Active Research: {}/4 | {}",
            country.name,
            format_population(total_pop),
            country.treasury,
            world_state.current_stage.display_name(),
            active_techs,
            reform_str
        );

        if text.0 != info {
            *text = Text::new(info);
        }
    }
}

fn update_top_bar_date(
    date: Res<GameDate>,
    paused: Res<GamePaused>,
    speed: Res<GameSpeed>,
    mut text_query: Query<&mut Text, With<TopBarDateText>>,
) {
    if !date.is_changed() && !paused.is_changed() && !speed.is_changed() {
        return;
    }

    let Ok(mut text) = text_query.single_mut() else {
        return;
    };

    let status = if paused.0 {
        "PAUSED".to_string()
    } else {
        format!("Spd: {}", speed.0)
    };

    let info = format!("{} [{}]", date.display(), status);
    if text.0 != info {
        *text = Text::new(info);
    }
}

fn handle_speed_buttons(
    mut interaction_query: Query<
        (&Interaction, &SpeedButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
    mut speed: ResMut<GameSpeed>,
    mut paused: ResMut<GamePaused>,
) {
    for (interaction, btn, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                speed.0 = btn.0;
                paused.0 = false;
                *bg = BackgroundColor(Color::srgb(0.5, 0.5, 0.5));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgb(0.4, 0.4, 0.4));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0));
            }
        }
    }
}

#[allow(clippy::type_complexity)]
fn handle_pause_button(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (With<PauseButton>, Changed<Interaction>),
    >,
    mut paused: ResMut<GamePaused>,
) {
    for (interaction, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                paused.0 = !paused.0;
                *bg = BackgroundColor(Color::srgb(0.6, 0.2, 0.2));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgb(0.5, 0.3, 0.3));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0));
            }
        }
    }
}

fn cleanup_top_bar(mut commands: Commands, query: Query<Entity, With<TopBarRoot>>) {
    if let Ok(entity) = query.single() {
        commands.entity(entity).despawn();
    }
}
