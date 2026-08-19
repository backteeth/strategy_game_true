use crate::app::game_state::{GameState, PlayingEntryMode};
use crate::common::CountryId;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::localization::{
    CurrentLocale, LanguageToggleButton, LanguageToggleButtonText, LocalizedText,
    TranslationCatalog, localized_text, t, tf, translate,
};
use crate::save::{LastLoadOutcome, LoadOutcome, LoadRequestMessage};
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

/// P21-SAVE-003: 「続きから」ボタン。既存の単一スロット(`saves/savegame_v1.ron`)を
/// 直接ロードする。国家未選択でも押下可能(New Gameの`StartGameButton`と異なり
/// `PreviewCountry`に依存しない)。押すと確認ダイアログ無しで即座に`LoadRequestMessage`を
/// 発行する(現在のWorldを失う状況ではないため、`ui::load_confirm`のゲーム内確認ダイアログとは
/// 別系統)。
#[derive(Component)]
pub struct ContinueButton;

/// P21-SAVE-003: 「続きから」の直近ロード失敗を示すインライン状態テキスト。
/// `GameNotification`/`NotificationHistory`はPlaying専用UI(`economy_panel`)でしか描画
/// されないため、CountrySelection画面では代わりにこのテキストで`LastLoadOutcome`の
/// 失敗分類をそのまま表示する(成功時はPlayingへ即座に遷移するため空のまま)。
#[derive(Component)]
pub struct ContinueStatusText;

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
                    // P21-SAVE-003: handle_continue_buttonを必ずhandle_start_buttonより先に
                    // 実行する(`.chain()`)。handle_start_buttonは同一フレームの
                    // LoadRequestMessage発行有無をpeekしてNew Gameを裁定するため、
                    // 書き込み(続きから)が先に完了している必要がある
                    // (save::runtime::handle_save_requestsのSave/Load調停と同じパターン)。
                    (handle_continue_button, handle_start_button).chain(),
                    update_continue_status_text,
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

                    // P21-SAVE-003: 「続きから」ボタン。国家未選択でも押下可能。
                    right_panel
                        .spawn((
                            ContinueButton,
                            Button,
                            Node {
                                padding: UiRect::new(
                                    Val::Px(32.0),
                                    Val::Px(32.0),
                                    Val::Px(12.0),
                                    Val::Px(12.0),
                                ),
                                margin: UiRect::top(Val::Px(8.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.3, 0.3, 0.45)),
                        ))
                        .with_children(|btn| {
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "country_selection.continue_button",
                                vec![],
                            );
                            btn.spawn((
                                text,
                                marker,
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(20.0),
                                    ..default()
                                },
                            ));
                        });

                    {
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "country_selection.continue_hint",
                            vec![],
                        );
                        right_panel.spawn((
                            text,
                            marker,
                            TextColor(Color::srgb(0.6, 0.6, 0.65)),
                            TextFont {
                                font_size: FontSize::Px(13.0),
                                ..default()
                            },
                        ));
                    }

                    // P21-SAVE-003: 「続きから」の失敗をインライン表示する(成功時は
                    // Playingへ即座に遷移するため、この画面に留まって見えることはない)。
                    // 既定は空文字列(表示する失敗が無い間は何も見えない)。
                    // `LocalizedText::default()`(key=="")は`update_continue_status_text`が
                    // 実際の失敗を検出するまで未初期化のまま保持される
                    // (`LocalizedText::render`は空keyに対してNoneを返す設計)。
                    right_panel.spawn((
                        ContinueStatusText,
                        Text::new(""),
                        LocalizedText::default(),
                        TextColor(Color::srgb(0.9, 0.4, 0.4)),
                        TextFont {
                            font_size: FontSize::Px(14.0),
                            ..default()
                        },
                    ));
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

/// P21-SAVE-003: 同一フレームに`ContinueButton`(続きから)も押されて
/// `LoadRequestMessage`が発行されている場合、New Gameは実行しない(Loadを優先する)。
/// `pending_loads`は消費専用の"peek"であり(`save::runtime::handle_save_requests`の
/// Save/Load調停と同じパターン)、`handle_load_requests`自身のカーソルには影響しない。
/// このSystemは`CountrySelectionPlugin`側で`handle_continue_button`の後に`.chain()`実行
/// されるよう登録されているため、同一フレーム内であれば書き込みが必ず先に完了している。
fn handle_start_button(
    interaction_query: Query<&Interaction, (With<StartGameButton>, Changed<Interaction>)>,
    preview: Res<PreviewCountry>,
    mut player_country: ResMut<PlayerCountry>,
    mut next_state: ResMut<NextState<GameState>>,
    mut pending_loads: MessageReader<LoadRequestMessage>,
    mut entry_mode: ResMut<PlayingEntryMode>,
) {
    let load_pending_this_frame = pending_loads.read().count() > 0;

    for interaction in interaction_query.iter() {
        if *interaction == Interaction::Pressed
            && let Some(id) = preview.0
            && !load_pending_this_frame
        {
            player_country.0 = Some(id);
            *entry_mode = PlayingEntryMode::NewGame;
            next_state.set(GameState::Playing);
        }
    }
}

/// P21-SAVE-003: 「続きから」ボタン。押下時、確認ダイアログ無しで即座に
/// `LoadRequestMessage`を1件発行するだけ(実際の読込→検証→適用は
/// `save::runtime::handle_load_requests`が`PostUpdate`で行う)。
#[allow(clippy::type_complexity)]
fn handle_continue_button(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (With<ContinueButton>, Changed<Interaction>),
    >,
    mut load_writer: MessageWriter<LoadRequestMessage>,
) {
    for (interaction, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                load_writer.write(LoadRequestMessage);
                *bg = BackgroundColor(Color::srgb(0.45, 0.45, 0.6));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgb(0.38, 0.38, 0.52));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgb(0.3, 0.3, 0.45));
            }
        }
    }
}

/// P21-SAVE-003: `LastLoadOutcome`が失敗を示している間だけ、その失敗分類をそのまま
/// インライン表示する(`GameNotification`はPlaying専用UIでしか描画されないため)。
/// `save::runtime::load_failure_notification`をそのまま再利用し、失敗分類ロジックを
/// 複製しない。成功時・未実行時は空文字列に戻す(成功時はPlayingへ遷移してこの画面自体が
/// 消えるため、実際に見えることはない)。
fn update_continue_status_text(
    outcome: Res<LastLoadOutcome>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut text_query: Query<(&mut Text, &mut LocalizedText), With<ContinueStatusText>>,
) {
    if !outcome.is_changed() {
        return;
    }
    let Ok((mut text, mut marker)) = text_query.single_mut() else {
        return;
    };

    match &outcome.0 {
        Some(LoadOutcome::Failure { error, .. }) => {
            let (key, args) = crate::save::runtime::load_failure_notification(error);
            let rendered = translate(&catalog, locale.0, key, &args);
            if text.0 != rendered {
                *text = Text::new(rendered);
            }
            marker.key = key;
            marker.args = args;
        }
        _ => {
            if !text.0.is_empty() {
                *text = Text::new(String::new());
            }
            marker.key = "";
            marker.args = Vec::new();
        }
    }
}

fn cleanup_ui(mut commands: Commands, query: Query<Entity, With<CountrySelectionRoot>>) {
    if let Ok(entity) = query.single() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::StateId;
    use crate::country::{CountryData, EconomicSystem, GovernmentType};
    use crate::localization::TranslationCorePlugin;
    use bevy::ecs::system::SystemState;

    /// `CountrySelectionPlugin`単体+ロード要求の受け皿だけを持つ最小App。
    /// `save::SaveGamePlugin`/`LoadGamePlugin`は追加しない(このファイルの関心は
    /// UI層が正しい要求を発行・裁定することであり、実際の読込→検証→適用は
    /// `save::runtime`側のテストで別途検証済みのため)。
    fn build_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.add_plugins(TranslationCorePlugin);
        app.add_message::<LoadRequestMessage>();
        app.insert_resource(CountryRegistry::default());
        app.insert_resource(StateRegistry::default());
        app.insert_resource(PlayerCountry(None));
        app.insert_resource(PlayingEntryMode::NewGame);
        app.insert_resource(LastLoadOutcome::default());
        app.insert_state(GameState::CountrySelection);
        app.add_plugins(CountrySelectionPlugin);
        app
    }

    fn one_country() -> CountryData {
        CountryData {
            id: CountryId(0),
            capital_state_id: StateId(0),
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::FreeMarket,
            ..CountryData::default()
        }
    }

    fn read_load_requests(app: &mut App) -> usize {
        let mut state: SystemState<MessageReader<LoadRequestMessage>> =
            SystemState::new(app.world_mut());
        state
            .get_mut(app.world_mut())
            .expect("reader")
            .read()
            .count()
    }

    /// 実際にOnEnterでspawnされたボタンEntityの`Interaction`を`Pressed`へ書き換える
    /// (テスト用に別のダミーEntityを追加spawnしない。実際のUIツリーへ作用させる)。
    fn press_button<T: Component>(app: &mut App) {
        let entity = app
            .world_mut()
            .query_filtered::<Entity, With<T>>()
            .iter(app.world())
            .next()
            .expect("button entity must exist");
        app.world_mut()
            .entity_mut(entity)
            .insert(Interaction::Pressed);
    }

    // ─── Startup/非自動実行 ──────────────────────────────────────────────────

    // 3. CountrySelectionへ入っただけではロードしない。 4. LoadExecutionCountは0のまま
    // (LoadExecutionCountはLoadGamePluginが管理するため、このApp単体では計測できない。
    // 「入っただけではLoadRequestMessageが一切発行されない」ことで同じ主張を検証する)。
    #[test]
    fn entering_country_selection_alone_emits_no_load_request() {
        let mut app = build_app();
        app.update(); // OnEnter(CountrySelection): setup_ui

        assert_eq!(
            read_load_requests(&mut app),
            0,
            "merely entering CountrySelection must not emit any LoadRequestMessage"
        );
    }

    // 5. 「続きから」ボタンが1個だけ表示される。
    #[test]
    fn exactly_one_continue_button_is_spawned() {
        let mut app = build_app();
        app.update();

        let count = app
            .world_mut()
            .query::<&ContinueButton>()
            .iter(app.world())
            .count();
        assert_eq!(count, 1, "exactly one ContinueButton must be spawned");
    }

    // 6. ボタン文字列がローカライズ経由。
    #[test]
    fn continue_button_label_is_routed_through_localization() {
        let mut app = build_app();
        app.update();

        let count = app
            .world_mut()
            .query::<&LocalizedText>()
            .iter(app.world())
            .filter(|lt| lt.key == "country_selection.continue_button")
            .count();
        assert_eq!(
            count, 1,
            "the continue button's label must be routed through country_selection.continue_button, not a hardcoded literal"
        );
    }

    // 7. 「続きから」押下でLoadRequestを1件発行。 8. 国家未選択でも押下可能。
    #[test]
    fn pressing_continue_emits_one_load_request_even_without_country_selected() {
        let mut app = build_app();
        app.update(); // CountryRegistry is empty -> PreviewCountry stays None
        assert_eq!(
            app.world().resource::<PreviewCountry>().0,
            None,
            "test setup must have no country selected"
        );

        press_button::<ContinueButton>(&mut app);
        app.update();

        assert_eq!(read_load_requests(&mut app), 1);
    }

    // 10. ボタン操作がNew Game開始要求へ漏れない。
    #[test]
    fn pressing_continue_alone_does_not_trigger_new_game() {
        let mut app = build_app();
        app.world_mut()
            .resource_mut::<CountryRegistry>()
            .countries
            .push(one_country());
        app.update(); // setup_ui: preview.0 = Some(CountryId(0)) (first registered country)
        assert_eq!(
            app.world().resource::<PreviewCountry>().0,
            Some(CountryId(0))
        );

        press_button::<ContinueButton>(&mut app);
        app.update();

        assert_eq!(
            app.world().resource::<PlayerCountry>().0,
            None,
            "pressing Continue must never set PlayerCountry; that is New Game's job"
        );
        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::CountrySelection,
            "pressing Continue alone must not transition state directly (only a later successful load does)"
        );
    }

    // 11. New GameとLoad同時要求ではLoadだけを実行。
    #[test]
    fn simultaneous_new_game_and_continue_requests_execute_load_only() {
        let mut app = build_app();
        app.world_mut()
            .resource_mut::<CountryRegistry>()
            .countries
            .push(one_country());
        app.update(); // setup_ui: preview.0 = Some(CountryId(0))

        press_button::<ContinueButton>(&mut app);
        press_button::<StartGameButton>(&mut app);
        app.update();

        assert_eq!(
            read_load_requests(&mut app),
            1,
            "the continue press must still emit exactly one LoadRequestMessage"
        );
        assert_eq!(
            app.world().resource::<PlayerCountry>().0,
            None,
            "New Game must not execute (PlayerCountry must stay unset) when a Load was also requested this frame"
        );
        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::CountrySelection,
            "New Game must not transition state when a Load is pending the same frame"
        );
    }

    // ─── 失敗時のインライン状態表示 ─────────────────────────────────────────────

    #[test]
    fn continue_status_text_shows_failure_and_clears_on_new_attempt() {
        let mut app = build_app();
        app.update();

        // 失敗が記録されると、ContinueStatusTextへ空でない文言が表示される。
        app.world_mut().resource_mut::<LastLoadOutcome>().0 = Some(LoadOutcome::Failure {
            path: std::path::PathBuf::from("saves/savegame_v1.ron"),
            error: crate::save::runtime::LoadOperationError::ReadOrValidate(
                crate::save::read::LoadSaveError::FileNotFound("saves/savegame_v1.ron".to_string()),
            ),
        });
        app.update();

        let text = app
            .world_mut()
            .query_filtered::<&Text, With<ContinueStatusText>>()
            .single(app.world())
            .expect("status text must exist")
            .0
            .clone();
        assert!(
            !text.is_empty(),
            "a load failure must produce a non-empty inline status message"
        );

        // 新しい試行が始まれば(outcomeがNoneへ戻れば)クリアされる。
        app.world_mut().resource_mut::<LastLoadOutcome>().0 = None;
        app.update();
        let text_after = app
            .world_mut()
            .query_filtered::<&Text, With<ContinueStatusText>>()
            .single(app.world())
            .expect("status text must exist")
            .0
            .clone();
        assert!(text_after.is_empty());
    }
}
