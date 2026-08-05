use crate::app::game_state::GameState;
use crate::common::CountryId;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::localization::{
    CurrentLocale, LanguageToggleButton, LanguageToggleButtonText, LocalizedText,
    TranslationCatalog, localized_text, t, tf,
};
use crate::state::data::StateRegistry;
use bevy::prelude::*;

#[derive(Component)]
pub struct CountrySelectionRoot;

#[derive(Component)]
pub struct CountrySelectButton(pub CountryId);

#[derive(Resource, Default)]
pub struct PreviewCountry(pub Option<CountryId>);

#[derive(Component)]
pub struct PreviewDetailText;

pub struct CountrySelectionPlugin;

impl Plugin for CountrySelectionPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PreviewCountry::default())
            .add_systems(OnEnter(GameState::CountrySelection), setup_ui)
            .add_systems(
                Update,
                (
                    handle_country_button_click,
                    update_preview_details,
                    handle_start_button,
                )
                    .run_if(in_state(GameState::CountrySelection)),
            )
            .add_systems(OnExit(GameState::CountrySelection), cleanup_ui);
    }
}

fn setup_ui(
    mut commands: Commands,
    country_registry: Res<CountryRegistry>,
    mut preview: ResMut<PreviewCountry>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    preview.0 = country_registry.countries.first().map(|c| c.id);

    commands
        .spawn((
            CountrySelectionRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                padding: UiRect::all(Val::Px(32.0)),
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.08, 1.0)),
        ))
        .with_children(|root| {
            root.spawn((Node {
                width: Val::Percent(30.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            },))
                .with_children(|left_panel| {
                    {
                        let (text, marker) =
                            localized_text(&catalog, locale.0, "country_selection.title", vec![]);
                        left_panel.spawn((
                            text,
                            marker,
                            TextFont {
                                font_size: FontSize::Px(24.0),
                                ..default()
                            },
                            TextColor(Color::srgb(0.9, 0.9, 0.9)),
                        ));
                    }

                    left_panel
                        .spawn((
                            LanguageToggleButton,
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                                margin: UiRect::bottom(Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.25, 0.35, 0.3, 1.0)),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                LanguageToggleButtonText,
                                Text::new(locale.0.next().own_name()),
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(13.0),
                                    ..default()
                                },
                            ));
                        });

                    for country in &country_registry.countries {
                        left_panel
                            .spawn((
                                CountrySelectButton(country.id),
                                Button,
                                Node {
                                    width: Val::Percent(100.0),
                                    padding: UiRect::all(Val::Px(12.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.2, 0.2, 0.2, 1.0)),
                            ))
                            .with_children(|btn| {
                                btn.spawn((
                                    Text::new(&country.name),
                                    TextColor(Color::srgb(0.9, 0.9, 0.9)),
                                    TextFont {
                                        font_size: FontSize::Px(18.0),
                                        ..default()
                                    },
                                ));
                            });
                    }
                });

            root.spawn((Node {
                width: Val::Percent(65.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(24.0),
                ..default()
            },))
                .with_children(|right_panel| {
                    {
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "country_selection.select_prompt",
                            vec![],
                        );
                        right_panel.spawn((
                            PreviewDetailText,
                            text,
                            marker,
                            TextColor(Color::srgb(0.8, 0.8, 0.8)),
                            TextFont {
                                font_size: FontSize::Px(20.0),
                                ..default()
                            },
                        ));
                    }

                    right_panel
                        .spawn((
                            StartGameButton,
                            Button,
                            Node {
                                padding: UiRect::new(
                                    Val::Px(32.0),
                                    Val::Px(32.0),
                                    Val::Px(16.0),
                                    Val::Px(16.0),
                                ),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.2, 0.6, 0.2)),
                        ))
                        .with_children(|btn| {
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "country_selection.start_button",
                                vec![],
                            );
                            btn.spawn((
                                text,
                                marker,
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(24.0),
                                    ..default()
                                },
                            ));
                        });
                });
        });
}

#[derive(Component)]
pub struct StartGameButton;

fn handle_country_button_click(
    mut interaction_query: Query<
        (&Interaction, &CountrySelectButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
    mut preview: ResMut<PreviewCountry>,
) {
    for (interaction, btn, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                preview.0 = Some(btn.0);
                *bg = BackgroundColor(Color::srgb(0.4, 0.4, 0.4));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgb(0.3, 0.3, 0.3));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.2, 0.2, 0.2, 1.0));
            }
        }
    }
}

fn update_preview_details(
    preview: Res<PreviewCountry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut text_query: Query<(&mut Text, &mut LocalizedText), With<PreviewDetailText>>,
) {
    if !preview.is_changed() && !locale.is_changed() {
        return;
    }

    let Ok((mut text, mut marker)) = text_query.single_mut() else {
        return;
    };

    if let Some(country) = preview.0.and_then(|id| country_registry.get(id)) {
        let country_id = country.id;
        let capital_name = state_registry
            .get(country.capital_state_id)
            .map(|s| s.name.to_string())
            .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));

        let total_pop: u64 = state_registry
            .states
            .iter()
            .filter(|s| s.owner_country_id == country_id)
            .map(|s| s.population)
            .sum();

        let args = vec![
            ("name", country.name.clone()),
            (
                "government",
                t(&catalog, locale.0, country.government_type.display_name()),
            ),
            (
                "economy",
                t(&catalog, locale.0, country.economic_system.display_name()),
            ),
            (
                "population",
                crate::ui::state_panel::format_population(total_pop),
            ),
            ("treasury", format!("{:.0}", country.treasury)),
            ("capital", capital_name),
        ];
        let info = tf(
            &catalog,
            locale.0,
            "country_selection.preview",
            args.clone(),
        );
        if text.0 != info {
            *text = Text::new(info);
        }
        marker.key = "country_selection.preview";
        marker.args = args;
    }
}

fn handle_start_button(
    interaction_query: Query<&Interaction, (With<StartGameButton>, Changed<Interaction>)>,
    preview: Res<PreviewCountry>,
    mut player_country: ResMut<PlayerCountry>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    for interaction in interaction_query.iter() {
        if *interaction == Interaction::Pressed
            && let Some(id) = preview.0
        {
            player_country.0 = Some(id);
            next_state.set(GameState::Playing);
        }
    }
}

fn cleanup_ui(mut commands: Commands, query: Query<Entity, With<CountrySelectionRoot>>) {
    if let Ok(entity) = query.single() {
        commands.entity(entity).despawn();
    }
}
