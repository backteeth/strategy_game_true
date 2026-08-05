use crate::app::game_state::GameState;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::economy::resources::ResourceType;
use crate::localization::{
    CurrentLocale, LocalizedText, TranslationCatalog, localized_text, t, tf,
};
use crate::state::data::StateRegistry;
use crate::ui::notification::NotificationHistory;
use bevy::prelude::*;

#[derive(Component)]
pub struct EconomyPanelRoot;

#[derive(Component)]
pub struct EconomyPanelText;

#[derive(Component)]
pub struct TaxUpButton;

#[derive(Component)]
pub struct TaxDownButton;

pub struct EconomyPanelPlugin;

impl Plugin for EconomyPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), setup_economy_panel)
            .add_systems(
                Update,
                (update_economy_panel, handle_tax_buttons).run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_economy_panel(
    mut commands: Commands,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    commands
        .spawn((
            EconomyPanelRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(40.0),
                bottom: Val::Px(0.0),
                width: Val::Px(300.0),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.1, 0.85)),
        ))
        .with_children(|parent| {
            let (text, marker) = localized_text(&catalog, locale.0, "economy_panel.title", vec![]);
            parent.spawn((
                text,
                marker,
                TextLayout {
                    linebreak: LineBreak::AnyCharacter,
                    ..default()
                },
                TextColor(Color::srgb(0.9, 0.85, 0.6)),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
            ));

            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    let (text, marker) =
                        localized_text(&catalog, locale.0, "economy_panel.tax_rate_label", vec![]);
                    row.spawn((
                        text,
                        marker,
                        TextLayout {
                            linebreak: LineBreak::AnyCharacter,
                            ..default()
                        },
                        TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        TextFont {
                            font_size: FontSize::Px(14.0),
                            ..default()
                        },
                    ));

                    row.spawn((
                        TaxDownButton,
                        Button,
                        Node {
                            padding: UiRect::horizontal(Val::Px(8.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0)),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new("-1%"),
                            TextLayout {
                                linebreak: LineBreak::AnyCharacter,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(12.0),
                                ..default()
                            },
                        ));
                    });

                    row.spawn((
                        TaxUpButton,
                        Button,
                        Node {
                            padding: UiRect::horizontal(Val::Px(8.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0)),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new("+1%"),
                            TextLayout {
                                linebreak: LineBreak::AnyCharacter,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(12.0),
                                ..default()
                            },
                        ));
                    });
                });

            let (text, marker) =
                localized_text(&catalog, locale.0, "economy_panel.loading", vec![]);
            parent.spawn((
                EconomyPanelText,
                text,
                marker,
                TextLayout {
                    linebreak: LineBreak::AnyCharacter,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.9)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));
        });
}

fn update_economy_panel(
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    notif_history: Res<NotificationHistory>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut text_q: Query<(&mut Text, &mut LocalizedText), With<EconomyPanelText>>,
) {
    let Ok((mut text, mut marker)) = text_q.single_mut() else {
        return;
    };

    let Some(country_id) = player_country.0 else {
        let key = "economy_panel.no_country";
        let rendered = t(&catalog, locale.0, key);
        if text.0 != rendered {
            *text = Text::new(rendered);
        }
        marker.key = key;
        marker.args = vec![];
        return;
    };

    let Some(country) = country_registry.get(country_id) else {
        return;
    };

    let mut stockpile_str = String::new();
    for res in ResourceType::ALL {
        let amount = country.stockpile.get(res);
        stockpile_str.push_str(&tf(
            &catalog,
            locale.0,
            "economy_panel.stockpile_line",
            vec![
                ("resource", t(&catalog, locale.0, res.display_name())),
                ("amount", format!("{:.0}", amount)),
            ],
        ));
    }

    let mut queue_str = String::new();
    if country.construction_queue.is_empty() {
        queue_str.push_str(&t(&catalog, locale.0, "economy_panel.queue_empty"));
    } else {
        for (i, item) in country.construction_queue.iter().enumerate() {
            let state_name = state_registry
                .get(item.state_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
            queue_str.push_str(&tf(
                &catalog,
                locale.0,
                "economy_panel.queue_line",
                vec![
                    ("index", (i + 1).to_string()),
                    (
                        "building",
                        t(&catalog, locale.0, item.building_type.display_name()),
                    ),
                    ("state", state_name),
                    ("progress", format!("{:.0}", item.progress)),
                    ("required", format!("{:.0}", item.required_progress)),
                ],
            ));
        }
    }

    let last_notif = notif_history
        .recent
        .last()
        .cloned()
        .unwrap_or_else(|| t(&catalog, locale.0, "common.none"));

    let args = vec![
        (
            "state",
            t(
                &catalog,
                locale.0,
                country.economic_state.display_name_short(),
            ),
        ),
        ("tax_rate", format!("{:.1}", country.tax_rate * 100.0)),
        ("income", format!("{:.1}", country.monthly_income)),
        ("expenses", format!("{:.1}", country.monthly_expenses)),
        ("balance", format!("{:.1}", country.monthly_balance)),
        (
            "sci_cap",
            format!("{:.1}", country.science_research_capacity),
        ),
        ("mag_cap", format!("{:.1}", country.magic_research_capacity)),
        ("stockpile", stockpile_str),
        ("queue", queue_str),
        ("last_notification", last_notif),
    ];
    let info = tf(&catalog, locale.0, "economy_panel.info", args.clone());

    if text.0 != info {
        *text = Text::new(info);
    }
    marker.key = "economy_panel.info";
    marker.args = args;
}

fn handle_tax_buttons(
    up_q: Query<&Interaction, (With<TaxUpButton>, Changed<Interaction>)>,
    down_q: Query<&Interaction, (With<TaxDownButton>, Changed<Interaction>)>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
) {
    let Some(country_id) = player_country.0 else {
        return;
    };
    let Some(country) = country_registry.get_mut(country_id) else {
        return;
    };

    for interaction in up_q.iter() {
        if *interaction == Interaction::Pressed {
            country.tax_rate = (country.tax_rate + 0.01).clamp(0.0, 1.0);
        }
    }
    for interaction in down_q.iter() {
        if *interaction == Interaction::Pressed {
            country.tax_rate = (country.tax_rate - 0.01).clamp(0.0, 1.0);
        }
    }
}
