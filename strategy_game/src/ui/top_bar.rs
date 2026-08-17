use crate::app::game_state::GameState;
use crate::app::time::{GameDate, GamePaused, GameSpeed};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::localization::{
    CurrentLocale, LanguageToggleButton, LanguageToggleButtonText, Locale, LocalizedText,
    TranslationCatalog, localized_text, t, tf,
};
use crate::research::world_stage::WorldCivilizationState;
use crate::save::runtime::SaveRequestMessage;
use crate::state::data::StateRegistry;
use crate::ui::load_confirm::LoadConfirmState;
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

/// P21-SAVE-002E: セーブボタン。押すと単一スロットへ直接上書き保存する
/// (今回は上書き確認を追加しない)。
#[derive(Component)]
pub struct SaveButton;

/// P21-SAVE-002E: ロードボタン。押しても即ロードせず、確認UI
/// (`ui::load_confirm`)を開くだけ。
#[derive(Component)]
pub struct LoadButton;

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
                    handle_save_button,
                    handle_load_button,
                )
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(OnExit(GameState::Playing), cleanup_top_bar);
    }
}

fn setup_top_bar(
    mut commands: Commands,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
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

                    // P21-SAVE-002E: セーブ/ロードボタン。既存ボタン群と同じ配色・
                    // サイズ・Interaction処理パターンを再利用する。
                    right_panel
                        .spawn((
                            SaveButton,
                            Button,
                            Node {
                                padding: UiRect::horizontal(Val::Px(8.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                margin: UiRect::left(Val::Px(8.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0)),
                        ))
                        .with_children(|btn| {
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "top_bar.save_button",
                                Vec::new(),
                            );
                            btn.spawn((
                                text,
                                marker,
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(12.0),
                                    ..default()
                                },
                            ));
                        });

                    right_panel
                        .spawn((
                            LoadButton,
                            Button,
                            Node {
                                padding: UiRect::horizontal(Val::Px(8.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                margin: UiRect::left(Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0)),
                        ))
                        .with_children(|btn| {
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "top_bar.load_button",
                                Vec::new(),
                            );
                            btn.spawn((
                                text,
                                marker,
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

/// P21-SAVE-002E: セーブボタン。押すと`SaveRequestMessage`を1件発行するだけ
/// (単一スロットへ直接上書き保存、確認は挟まない)。連打で同一フレームに複数回
/// 押されても、`handle_save_requests`側の集約([`.read().count()`])により1回へ集約される。
#[allow(clippy::type_complexity)]
fn handle_save_button(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (With<SaveButton>, Changed<Interaction>),
    >,
    mut save_writer: MessageWriter<SaveRequestMessage>,
) {
    for (interaction, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                save_writer.write(SaveRequestMessage);
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

/// P21-SAVE-002E: ロードボタン。押しても`LoadRequestMessage`は発行せず、確認UI
/// (`ui::load_confirm::LoadConfirmState`)を開くだけ。誤操作による未保存進行の消失を
/// 防ぐため、実際のロードは確認UI側の「ロード」ボタンでのみ発行される。
#[allow(clippy::type_complexity)]
fn handle_load_button(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (With<LoadButton>, Changed<Interaction>),
    >,
    mut confirm_state: ResMut<LoadConfirmState>,
) {
    for (interaction, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                confirm_state.open = true;
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

fn cleanup_top_bar(mut commands: Commands, query: Query<Entity, With<TopBarRoot>>) {
    if let Ok(entity) = query.single() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::SystemState;

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<SaveRequestMessage>();
        app.insert_resource(LoadConfirmState::default());
        app
    }

    /// `military_panel.rs`の`recruit_button_is_spawned_in_military_panel_ui_tree`と
    /// 同じパターン: `GameState::Playing`への遷移は経由せず、`setup_top_bar`を直接
    /// `Startup`へ登録してUIツリーの構造だけを検証する
    /// (`OnEnter(GameState::Playing)`にしか登録していないこと自体は`TopBarPlugin::build`の
    /// コード自体が示す構造的事実であり、ここでは実際に構築されるツリーの中身を検証する)。
    fn build_ui_spawn_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Startup, setup_top_bar);
        app
    }

    #[test]
    fn save_and_load_buttons_are_spawned_in_top_bar_ui_tree() {
        let mut app = build_ui_spawn_test_app();
        app.update();

        let save_count = app
            .world_mut()
            .query::<&SaveButton>()
            .iter(app.world())
            .count();
        let load_count = app
            .world_mut()
            .query::<&LoadButton>()
            .iter(app.world())
            .count();
        assert_eq!(save_count, 1, "exactly one SaveButton must be spawned");
        assert_eq!(load_count, 1, "exactly one LoadButton must be spawned");
    }

    #[test]
    fn save_button_press_emits_save_request_message() {
        let mut app = build_test_app();
        app.add_systems(Update, handle_save_button);
        app.world_mut().spawn((
            SaveButton,
            Interaction::Pressed,
            BackgroundColor(Color::WHITE),
        ));

        app.update();

        let mut state: SystemState<MessageReader<SaveRequestMessage>> =
            SystemState::new(app.world_mut());
        let count = state
            .get_mut(app.world_mut())
            .expect("reader")
            .read()
            .count();
        assert_eq!(
            count, 1,
            "pressing Save must emit exactly one SaveRequestMessage"
        );
    }

    #[test]
    fn load_button_first_press_opens_confirm_dialog_without_emitting_load_request() {
        let mut app = build_test_app();
        app.add_systems(Update, handle_load_button);
        app.world_mut().spawn((
            LoadButton,
            Interaction::Pressed,
            BackgroundColor(Color::WHITE),
        ));

        app.update();

        assert!(
            app.world().resource::<LoadConfirmState>().open,
            "the first Load button press must open the confirmation dialog"
        );
    }

    #[test]
    fn load_button_press_does_not_change_game_paused() {
        let mut app = build_test_app();
        app.insert_resource(crate::app::time::GamePaused(true));
        app.add_systems(Update, handle_load_button);
        app.world_mut().spawn((
            LoadButton,
            Interaction::Pressed,
            BackgroundColor(Color::WHITE),
        ));

        app.update();

        assert!(
            app.world().resource::<crate::app::time::GamePaused>().0,
            "opening the confirmation dialog alone must not change GamePaused"
        );
    }
}
