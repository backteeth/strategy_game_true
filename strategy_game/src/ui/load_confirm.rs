/// P21-SAVE-002E: ロード確認ダイアログ。
///
/// 誤操作による未保存進行の消失を防ぐため、トップバーの「ロード」ボタンを押しても
/// 即座に`LoadRequestMessage`は発行しない。まずこのダイアログを開き、ダイアログ内の
/// 「ロード」ボタンを押した場合だけ実際にロードを要求する。
///
/// `open: bool`という独立した状態は、既存の`PeacePanelState`等と同じパターン
/// (`ui::ActivePanel`が担う4大パネル[Research/Politics/Diplomacy/Military]の排他制御とは
/// 別系統。このダイアログは常設パネルではなく一時的な確認モーダルであり、既存の
/// パネル排他制御の対象に混ぜ込まない方が責務が明確なため、既存の`PeacePanelState`の
/// 前例に倣う)。
use crate::app::game_state::GameState;
use crate::localization::{CurrentLocale, TranslationCatalog, localized_text};
use crate::save::runtime::LoadRequestMessage;
use bevy::prelude::*;

/// ロード確認ダイアログの開閉状態。ロード成功・失敗いずれの場合も、確認ボタンを
/// 押した時点で同期的に`open = false`にするため(`handle_load_confirm_button`)、
/// ロード処理(`PostUpdate`)の結果を待たずに必ず閉じる。002Dの適用中核へこの状態を
/// 必須Resourceとして追加してはいない(UI層だけで完結する)。
#[derive(Resource, Default)]
pub struct LoadConfirmState {
    pub open: bool,
}

#[derive(Component)]
pub struct LoadConfirmRoot;

#[derive(Component)]
pub struct LoadConfirmButton;

#[derive(Component)]
pub struct LoadCancelButton;

pub struct LoadConfirmPlugin;

impl Plugin for LoadConfirmPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadConfirmState>()
            .add_systems(OnEnter(GameState::Playing), setup_load_confirm_dialog)
            .add_systems(
                Update,
                (
                    sync_load_confirm_visibility,
                    handle_load_confirm_button,
                    handle_load_cancel_button,
                )
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(OnExit(GameState::Playing), cleanup_load_confirm_dialog);
    }
}

/// ダイアログ全体(背景+本体)を1つのEntityツリーとしてStartup相当([`OnEnter(Playing)`])で
/// 一度だけ生成する。既定では非表示(`Display::None`)。背景自体も`Button`にして、
/// ダイアログが開いている間はどこをクリック/ホバーしても既存の
/// 「UIのHovered/Pressed中はマップ操作をスキップする」ガード
/// (`map::selection::handle_state_click`等が`Query<&Interaction>`を走査するだけの
/// 汎用チェックのため、新たにこのファイルを変更する必要はない)に確実に引っかかるようにする。
fn setup_load_confirm_dialog(
    mut commands: Commands,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    commands
        .spawn((
            LoadConfirmRoot,
            Button,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                display: Display::None,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        ))
        .with_children(|backdrop| {
            backdrop
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(20.0)),
                        row_gap: Val::Px(16.0),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.15, 0.15, 0.2, 1.0)),
                ))
                .with_children(|dialog| {
                    let (text, marker) =
                        localized_text(&catalog, locale.0, "load_confirm.body", Vec::new());
                    dialog.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.95, 0.95, 0.95)),
                        TextFont {
                            font_size: FontSize::Px(16.0),
                            ..default()
                        },
                    ));

                    dialog
                        .spawn((Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(12.0),
                            ..default()
                        },))
                        .with_children(|row| {
                            row.spawn((
                                LoadConfirmButton,
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0)),
                            ))
                            .with_children(|btn| {
                                let (text, marker) = localized_text(
                                    &catalog,
                                    locale.0,
                                    "load_confirm.confirm_button",
                                    Vec::new(),
                                );
                                btn.spawn((
                                    text,
                                    marker,
                                    TextColor(Color::WHITE),
                                    TextFont {
                                        font_size: FontSize::Px(14.0),
                                        ..default()
                                    },
                                ));
                            });

                            row.spawn((
                                LoadCancelButton,
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.3, 0.3, 0.3, 1.0)),
                            ))
                            .with_children(|btn| {
                                let (text, marker) = localized_text(
                                    &catalog,
                                    locale.0,
                                    "load_confirm.cancel_button",
                                    Vec::new(),
                                );
                                btn.spawn((
                                    text,
                                    marker,
                                    TextColor(Color::WHITE),
                                    TextFont {
                                        font_size: FontSize::Px(14.0),
                                        ..default()
                                    },
                                ));
                            });
                        });
                });
        });
}

/// `LoadConfirmState.open`が変化したときだけ表示/非表示を切り替える
/// (`ui::sync_panels_to_active`と同じ`is_changed()`ガード付きパターン)。
fn sync_load_confirm_visibility(
    state: Res<LoadConfirmState>,
    mut query: Query<&mut Node, With<LoadConfirmRoot>>,
) {
    if !state.is_changed() {
        return;
    }
    if let Ok(mut node) = query.single_mut() {
        node.display = if state.open {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// ダイアログ内の「ロード」ボタン。押した時点で`LoadRequestMessage`を1件発行し、
/// 同じフレーム内で同期的にダイアログを閉じる(ロード成功/失敗の結果を待たない。
/// 結果は`PostUpdate`の`handle_load_requests`が別途`LastLoadOutcome`へ記録し、
/// 通知として表示する)。
#[allow(clippy::type_complexity)]
fn handle_load_confirm_button(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (With<LoadConfirmButton>, Changed<Interaction>),
    >,
    mut confirm_state: ResMut<LoadConfirmState>,
    mut load_writer: MessageWriter<LoadRequestMessage>,
) {
    for (interaction, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                load_writer.write(LoadRequestMessage);
                confirm_state.open = false;
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

/// ダイアログ内の「キャンセル」ボタン。ゲーム状態を一切変更せず、ダイアログを閉じるだけ。
#[allow(clippy::type_complexity)]
fn handle_load_cancel_button(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (With<LoadCancelButton>, Changed<Interaction>),
    >,
    mut confirm_state: ResMut<LoadConfirmState>,
) {
    for (interaction, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                confirm_state.open = false;
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

fn cleanup_load_confirm_dialog(
    mut commands: Commands,
    query: Query<Entity, With<LoadConfirmRoot>>,
) {
    if let Ok(entity) = query.single() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::localization::{CurrentLocale, TranslationCatalog};
    use bevy::ecs::system::SystemState;

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<LoadRequestMessage>();
        app.insert_resource(LoadConfirmState::default());
        app
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

    #[test]
    fn confirm_button_press_emits_load_request_and_closes_dialog() {
        let mut app = build_test_app();
        app.world_mut().resource_mut::<LoadConfirmState>().open = true;
        app.add_systems(Update, handle_load_confirm_button);
        app.world_mut().spawn((
            LoadConfirmButton,
            Interaction::Pressed,
            BackgroundColor(Color::WHITE),
        ));

        app.update();

        assert_eq!(
            read_load_requests(&mut app),
            1,
            "confirming must emit exactly one LoadRequestMessage"
        );
        assert!(
            !app.world().resource::<LoadConfirmState>().open,
            "the dialog must close synchronously when confirmed"
        );
    }

    #[test]
    fn cancel_button_press_closes_dialog_without_emitting_load_request() {
        let mut app = build_test_app();
        app.world_mut().resource_mut::<LoadConfirmState>().open = true;
        app.add_systems(Update, handle_load_cancel_button);
        app.world_mut().spawn((
            LoadCancelButton,
            Interaction::Pressed,
            BackgroundColor(Color::WHITE),
        ));

        app.update();

        assert_eq!(
            read_load_requests(&mut app),
            0,
            "cancelling must never emit a LoadRequestMessage"
        );
        assert!(
            !app.world().resource::<LoadConfirmState>().open,
            "the dialog must close on cancel"
        );
    }

    #[test]
    fn dialog_is_spawned_hidden_by_default() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Startup, setup_load_confirm_dialog);
        app.update();

        let mut query = app.world_mut().query::<(&LoadConfirmRoot, &Node)>();
        let (_, node) = query
            .single(app.world())
            .expect("dialog root must be spawned");
        assert_eq!(node.display, Display::None, "the dialog must start hidden");
    }

    #[test]
    fn sync_visibility_shows_and_hides_dialog_node() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.insert_resource(LoadConfirmState::default());
        app.add_systems(Startup, setup_load_confirm_dialog);
        app.add_systems(Update, sync_load_confirm_visibility);
        app.update();

        app.world_mut().resource_mut::<LoadConfirmState>().open = true;
        app.update();
        {
            let mut query = app.world_mut().query::<(&LoadConfirmRoot, &Node)>();
            let (_, node) = query.single(app.world()).unwrap();
            assert_eq!(node.display, Display::Flex, "opening must show the dialog");
        }

        app.world_mut().resource_mut::<LoadConfirmState>().open = false;
        app.update();
        {
            let mut query = app.world_mut().query::<(&LoadConfirmRoot, &Node)>();
            let (_, node) = query.single(app.world()).unwrap();
            assert_eq!(
                node.display,
                Display::None,
                "closing must hide the dialog again"
            );
        }
    }
}
