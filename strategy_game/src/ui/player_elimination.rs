//! P21-021: プレイヤー国家消滅時の敗北オーバーレイと観戦モードへの導線。
//!
//! `country::PlayerEliminationMode`が`DefeatOverlayPending`になった瞬間、全画面
//! オーバーレイ(消滅日・併合国・旧首都名を表示)を出す。「観戦を続ける」ボタンを
//! 押すと`Spectating`へ遷移してオーバーレイを閉じる——`Spectating`でも
//! `country::player_can_issue_commands`は引き続き`false`を返すため、観戦モードは
//! 「情報閲覧・ゲーム進行は可能、自国コマンドは一切発行不可」を意味する
//! (`ui::load_confirm::LoadConfirmPlugin`と同じ、常設パネルではない一時的
//! モーダルのパターンを踏襲する)。

use crate::country::{CountryRegistry, PlayerCountry, PlayerEliminationMode};
use crate::localization::{CurrentLocale, TranslationCatalog, localized_text};
use crate::state::data::StateRegistry;
use bevy::prelude::*;

#[derive(Component)]
pub struct PlayerEliminationOverlayRoot;

#[derive(Component)]
struct PlayerEliminationBodyText;

#[derive(Component)]
struct ContinueSpectatingButton;

pub struct PlayerEliminationPlugin;

impl Plugin for PlayerEliminationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(crate::app::game_state::GameState::Playing),
            setup_player_elimination_overlay,
        )
        .add_systems(
            Update,
            (
                sync_player_elimination_overlay,
                handle_continue_spectating_button,
            )
                .run_if(in_state(crate::app::game_state::GameState::Playing)),
        )
        .add_systems(
            OnExit(crate::app::game_state::GameState::Playing),
            cleanup_player_elimination_overlay,
        );
    }
}

fn setup_player_elimination_overlay(
    mut commands: Commands,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    commands
        .spawn((
            PlayerEliminationOverlayRoot,
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
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
        ))
        .with_children(|backdrop| {
            backdrop
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(28.0)),
                        row_gap: Val::Px(18.0),
                        align_items: AlignItems::Center,
                        max_width: Val::Px(480.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.12, 0.06, 0.06, 1.0)),
                ))
                .with_children(|dialog| {
                    let (title_text, title_marker) =
                        localized_text(&catalog, locale.0, "player_elimination.title", Vec::new());
                    dialog.spawn((
                        title_text,
                        title_marker,
                        TextColor(Color::srgb(0.95, 0.4, 0.4)),
                        TextFont {
                            font_size: FontSize::Px(24.0),
                            ..default()
                        },
                    ));

                    let (body_text, body_marker) =
                        localized_text(&catalog, locale.0, "player_elimination.body", Vec::new());
                    dialog.spawn((
                        PlayerEliminationBodyText,
                        body_text,
                        body_marker,
                        TextColor(Color::srgb(0.9, 0.9, 0.9)),
                        TextFont {
                            font_size: FontSize::Px(15.0),
                            ..default()
                        },
                    ));

                    dialog
                        .spawn((
                            ContinueSpectatingButton,
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.35, 0.3, 0.3, 1.0)),
                        ))
                        .with_children(|btn| {
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "player_elimination.continue_button",
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
}

/// `PlayerEliminationMode`が変化したときだけ表示/非表示・本文を更新する。
/// `DefeatOverlayPending`のときだけ表示し、`NotEliminated`/`Spectating`では隠す
/// (`Spectating`は「一度閉じた後の継続」状態であり、オーバーレイ自体は不要)。
#[allow(clippy::too_many_arguments)]
fn sync_player_elimination_overlay(
    mode: Res<PlayerEliminationMode>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut root_q: Query<&mut Node, With<PlayerEliminationOverlayRoot>>,
    mut body_q: Query<
        (&mut Text, &mut crate::localization::LocalizedText),
        With<PlayerEliminationBodyText>,
    >,
) {
    if !mode.is_changed() {
        return;
    }
    let Ok(mut node) = root_q.single_mut() else {
        return;
    };
    node.display = if *mode == PlayerEliminationMode::DefeatOverlayPending {
        Display::Flex
    } else {
        Display::None
    };

    if *mode != PlayerEliminationMode::DefeatOverlayPending {
        return;
    }

    let Some(player_id) = player_country.0 else {
        return;
    };
    let Some(eliminated) = country_registry.get(player_id) else {
        return;
    };
    let crate::country::CountryStatus::Eliminated {
        eliminated_date,
        annexed_by,
    } = &eliminated.status
    else {
        return;
    };
    let annexer_name = country_registry
        .get(*annexed_by)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| format!("{annexed_by:?}"));
    let capital_name = state_registry
        .get(eliminated.capital_state_id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| format!("{:?}", eliminated.capital_state_id));

    let args = vec![
        ("country", eliminated.name.clone()),
        ("annexer", annexer_name),
        ("date", eliminated_date.clone()),
        ("capital", capital_name),
    ];
    if let Ok((mut text, mut marker)) = body_q.single_mut() {
        let rendered =
            crate::localization::translate(&catalog, locale.0, "player_elimination.body", &args);
        text.0 = rendered;
        marker.args = args;
    }
}

#[allow(clippy::type_complexity)]
fn handle_continue_spectating_button(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (With<ContinueSpectatingButton>, Changed<Interaction>),
    >,
    mut mode: ResMut<PlayerEliminationMode>,
) {
    for (interaction, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *mode = PlayerEliminationMode::Spectating;
                *bg = BackgroundColor(Color::srgb(0.5, 0.45, 0.45));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgb(0.45, 0.4, 0.4));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.35, 0.3, 0.3, 1.0));
            }
        }
    }
}

fn cleanup_player_elimination_overlay(
    mut commands: Commands,
    query: Query<Entity, With<PlayerEliminationOverlayRoot>>,
) {
    if let Ok(entity) = query.single() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, StateId};
    use crate::country::{CountryData, CountryStatus};
    use crate::state::data::StateData;

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.insert_resource(PlayerEliminationMode::default());
        app.insert_resource(PlayerCountry(Some(CountryId(1))));
        app.insert_resource(CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    name: "Arcadia".to_string(),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    name: "Passguard".to_string(),
                    capital_state_id: StateId(5),
                    status: CountryStatus::Eliminated {
                        eliminated_date: "1936/06/01".to_string(),
                        annexed_by: CountryId(0),
                    },
                    ..CountryData::default()
                },
            ],
        });
        app.insert_resource(StateRegistry::build(vec![StateData {
            id: StateId(5),
            name: "Old Fortress".to_string(),
            ..Default::default()
        }]));
        app.add_systems(Startup, setup_player_elimination_overlay);
        app.add_systems(
            Update,
            (
                sync_player_elimination_overlay,
                handle_continue_spectating_button,
            ),
        );
        app.update();
        app
    }

    #[test]
    fn overlay_is_spawned_hidden_by_default() {
        let mut app = build_test_app();
        let mut query = app
            .world_mut()
            .query::<(&PlayerEliminationOverlayRoot, &Node)>();
        let (_, node) = query
            .single(app.world())
            .expect("overlay root must be spawned");
        assert_eq!(node.display, Display::None, "the overlay must start hidden");
    }

    #[test]
    fn overlay_shows_and_renders_country_annexer_and_date_once_defeat_pending() {
        let mut app = build_test_app();
        *app.world_mut().resource_mut::<PlayerEliminationMode>() =
            PlayerEliminationMode::DefeatOverlayPending;
        app.update();

        let mut root_q = app
            .world_mut()
            .query::<(&PlayerEliminationOverlayRoot, &Node)>();
        let (_, node) = root_q.single(app.world()).unwrap();
        assert_eq!(node.display, Display::Flex, "defeat must show the overlay");

        let mut body_q = app
            .world_mut()
            .query::<(&PlayerEliminationBodyText, &Text)>();
        let (_, text) = body_q.single(app.world()).unwrap();
        assert!(
            text.0.contains("Passguard"),
            "body must name the eliminated country, got: {}",
            text.0
        );
        assert!(
            text.0.contains("Arcadia"),
            "body must name the annexer, got: {}",
            text.0
        );
        assert!(
            text.0.contains("1936/06/01"),
            "body must show the elimination date, got: {}",
            text.0
        );
        assert!(
            text.0.contains("Old Fortress"),
            "body must show the former capital, got: {}",
            text.0
        );
    }

    #[test]
    fn overlay_hides_again_once_not_eliminated_or_spectating() {
        let mut app = build_test_app();
        *app.world_mut().resource_mut::<PlayerEliminationMode>() =
            PlayerEliminationMode::DefeatOverlayPending;
        app.update();
        *app.world_mut().resource_mut::<PlayerEliminationMode>() =
            PlayerEliminationMode::Spectating;
        app.update();

        let mut query = app
            .world_mut()
            .query::<(&PlayerEliminationOverlayRoot, &Node)>();
        let (_, node) = query.single(app.world()).unwrap();
        assert_eq!(
            node.display,
            Display::None,
            "once dismissed into Spectating, the overlay must hide again"
        );
    }

    #[test]
    fn continue_spectating_button_transitions_mode_and_closes_overlay() {
        let mut app = build_test_app();
        *app.world_mut().resource_mut::<PlayerEliminationMode>() =
            PlayerEliminationMode::DefeatOverlayPending;
        app.update();

        app.world_mut().spawn((
            ContinueSpectatingButton,
            Interaction::Pressed,
            BackgroundColor(Color::WHITE),
        ));
        app.update();

        assert_eq!(
            *app.world().resource::<PlayerEliminationMode>(),
            PlayerEliminationMode::Spectating
        );
    }
}
