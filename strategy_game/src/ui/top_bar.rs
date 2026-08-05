use crate::app::game_state::GameState;
use crate::app::time::{GameDate, GamePaused, GameSpeed};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::localization::{
    CurrentLocale, LanguageToggleButton, LanguageToggleButtonText, Locale, LocalizedText,
    TranslationCatalog, t, tf,
};
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

fn setup_top_bar(mut commands: Commands, locale: Res<CurrentLocale>) {
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
                LocalizedText::default(),
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
                        LocalizedText::default(),
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

                    // 言語切り替えボタン(P20-009): ラベルは切り替え先言語の自称(翻訳キーを介さない)
                    right_panel
                        .spawn((
                            LanguageToggleButton,
                            Button,
                            Node {
                                padding: UiRect::horizontal(Val::Px(8.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                margin: UiRect::left(Val::Px(8.0)),
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
                                    font_size: FontSize::Px(12.0),
                                    ..default()
                                },
                            ));
                        });
                });
        });
}

#[allow(clippy::too_many_arguments)]
fn update_top_bar_player_info(
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    world_state: Res<WorldCivilizationState>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut text_query: Query<(&mut Text, &mut LocalizedText), With<TopBarPlayerInfoText>>,
) {
    let Ok((mut text, mut marker)) = text_query.single_mut() else {
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
            tf(
                &catalog,
                locale.0,
                "top_bar.reform_active",
                vec![(
                    "percent",
                    format!("{:.0}", (r.progress / r.required_progress) * 100.0),
                )],
            )
        } else {
            tf(&catalog, locale.0, "top_bar.reform_none", vec![])
        };

        let args = vec![
            ("country", country.name.clone()),
            ("pop", format_population(total_pop)),
            ("treasury", format!("{:.0}", country.treasury)),
            (
                "era",
                t(&catalog, locale.0, world_state.current_stage.display_name()),
            ),
            ("research", active_techs.to_string()),
            ("reform", reform_str),
        ];
        let info = tf(&catalog, locale.0, "top_bar.info", args.clone());

        if text.0 != info {
            *text = Text::new(info);
        }
        marker.key = "top_bar.info";
        marker.args = args;
    }
}

/// UI表示専用の日付表記。`GameDate::display()`(内部シリアライズ形式 "YYYY/MM/DD",
/// 治療条約・戦争開始日などのゲーム状態文字列として使われる)とは完全に独立しており、
/// 値(年月日)そのものは変えず、言語ごとの慣用表記のみを切り替える。
fn format_date_for_locale(date: &GameDate, locale: Locale) -> String {
    const EN_MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    match locale {
        Locale::JaJp => format!("{}年{}月{}日", date.year, date.month, date.day),
        Locale::EnUs => {
            let month_idx = (date.month.max(1) as usize - 1).min(EN_MONTHS.len() - 1);
            format!("{} {}, {}", EN_MONTHS[month_idx], date.day, date.year)
        }
    }
}

fn update_top_bar_date(
    date: Res<GameDate>,
    paused: Res<GamePaused>,
    speed: Res<GameSpeed>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut text_query: Query<(&mut Text, &mut LocalizedText), With<TopBarDateText>>,
) {
    if !date.is_changed() && !paused.is_changed() && !speed.is_changed() && !locale.is_changed() {
        return;
    }

    let Ok((mut text, mut marker)) = text_query.single_mut() else {
        return;
    };

    let localized_date = format_date_for_locale(&date, locale.0);
    let (key, args) = if paused.0 {
        ("top_bar.date_paused", vec![("date", localized_date)])
    } else {
        (
            "top_bar.date_speed",
            vec![("date", localized_date), ("speed", speed.0.to_string())],
        )
    };

    let info = tf(&catalog, locale.0, key, args.clone());
    if text.0 != info {
        *text = Text::new(info);
    }
    marker.key = key;
    marker.args = args;
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
