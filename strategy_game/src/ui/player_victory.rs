//! P22-001: プレイヤーが世界征服(残る唯一のActive国家)を達成した際の勝利オーバーレイ。
//!
//! `country::PlayerVictoryMode`が`VictoryOverlayPending`になった瞬間、全画面
//! オーバーレイ(達成日を表示)を出す。「プレイを続ける」ボタンを押すと
//! `Acknowledged`へ遷移してオーバーレイを閉じ、以後は通常どおりプレイを継続できる
//! (`ui::player_elimination::PlayerEliminationPlugin`と同じ、常設パネルではない
//! 一時的モーダルのパターンを踏襲する)。

use crate::country::{CountryRegistry, PlayerCountry, PlayerVictoryMode};
use crate::localization::{CurrentLocale, TranslationCatalog, localized_text};
use bevy::prelude::*;

#[derive(Component)]
pub struct PlayerVictoryOverlayRoot;

#[derive(Component)]
struct PlayerVictoryBodyText;

#[derive(Component)]
struct ContinuePlayingButton;

pub struct PlayerVictoryPlugin;

impl Plugin for PlayerVictoryPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(crate::app::game_state::GameState::Playing),
            setup_player_victory_overlay,
        )
        .add_systems(
            Update,
            (sync_player_victory_overlay, handle_continue_playing_button)
                .run_if(in_state(crate::app::game_state::GameState::Playing)),
        )
        .add_systems(
            OnExit(crate::app::game_state::GameState::Playing),
            cleanup_player_victory_overlay,
        );
    }
}

fn setup_player_victory_overlay(
    mut commands: Commands,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    commands
        .spawn((
            PlayerVictoryOverlayRoot,
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
                    BackgroundColor(Color::srgba(0.08, 0.1, 0.06, 1.0)),
                ))
                .with_children(|dialog| {
                    let (title_text, title_marker) =
                        localized_text(&catalog, locale.0, "player_victory.title", Vec::new());
                    dialog.spawn((
                        title_text,
                        title_marker,
                        TextColor(Color::srgb(0.9, 0.85, 0.3)),
                        TextFont {
                            font_size: FontSize::Px(24.0),
                            ..default()
                        },
                    ));

                    let (body_text, body_marker) =
                        localized_text(&catalog, locale.0, "player_victory.body", Vec::new());
                    dialog.spawn((
                        PlayerVictoryBodyText,
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
                            ContinuePlayingButton,
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.25, 0.3, 0.2, 1.0)),
                        ))
                        .with_children(|btn| {
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "player_victory.continue_button",
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

/// `PlayerVictoryMode`が変化したときだけ表示/非表示・本文を更新する。
/// `VictoryOverlayPending`のときだけ表示し、`NotConcluded`/`Acknowledged`では隠す。
#[allow(clippy::too_many_arguments)]
fn sync_player_victory_overlay(
    mode: Res<PlayerVictoryMode>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut root_q: Query<&mut Node, With<PlayerVictoryOverlayRoot>>,
    mut body_q: Query<
        (&mut Text, &mut crate::localization::LocalizedText),
        With<PlayerVictoryBodyText>,
    >,
) {
    if !mode.is_changed() {
        return;
    }
    let Ok(mut node) = root_q.single_mut() else {
        return;
    };
    let PlayerVictoryMode::VictoryOverlayPending { victory_date } = &*mode else {
        node.display = Display::None;
        return;
    };
    node.display = Display::Flex;

    let country_name = player_country
        .0
        .and_then(|id| country_registry.get(id))
        .map(|c| c.name.clone())
        .unwrap_or_default();
    let args = vec![("country", country_name), ("date", victory_date.clone())];
    if let Ok((mut text, mut marker)) = body_q.single_mut() {
        let rendered =
            crate::localization::translate(&catalog, locale.0, "player_victory.body", &args);
        text.0 = rendered;
        marker.args = args;
    }
}

#[allow(clippy::type_complexity)]
fn handle_continue_playing_button(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (With<ContinuePlayingButton>, Changed<Interaction>),
    >,
    mut mode: ResMut<PlayerVictoryMode>,
) {
    for (interaction, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *mode = PlayerVictoryMode::Acknowledged;
                *bg = BackgroundColor(Color::srgb(0.35, 0.4, 0.3));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgb(0.3, 0.35, 0.25));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.25, 0.3, 0.2, 1.0));
            }
        }
    }
}

fn cleanup_player_victory_overlay(
    mut commands: Commands,
    query: Query<Entity, With<PlayerVictoryOverlayRoot>>,
) {
    if let Ok(entity) = query.single() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::CountryId;
    use crate::country::CountryData;

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.insert_resource(PlayerVictoryMode::default());
        app.insert_resource(PlayerCountry(Some(CountryId(0))));
        app.insert_resource(CountryRegistry {
            countries: vec![CountryData {
                id: CountryId(0),
                name: "Arcadia".to_string(),
                ..CountryData::default()
            }],
        });
        app.add_systems(Startup, setup_player_victory_overlay);
        app.add_systems(
            Update,
            (sync_player_victory_overlay, handle_continue_playing_button),
        );
        app.update();
        app
    }

    #[test]
    fn overlay_is_spawned_hidden_by_default() {
        let mut app = build_test_app();
        let mut query = app
            .world_mut()
            .query::<(&PlayerVictoryOverlayRoot, &Node)>();
        let (_, node) = query
            .single(app.world())
            .expect("overlay root must be spawned");
        assert_eq!(node.display, Display::None, "the overlay must start hidden");
    }

    #[test]
    fn overlay_shows_and_renders_the_victory_date_once_pending() {
        let mut app = build_test_app();
        *app.world_mut().resource_mut::<PlayerVictoryMode>() =
            PlayerVictoryMode::VictoryOverlayPending {
                victory_date: "1850/06/15".to_string(),
            };
        app.update();

        let mut root_q = app
            .world_mut()
            .query::<(&PlayerVictoryOverlayRoot, &Node)>();
        let (_, node) = root_q.single(app.world()).unwrap();
        assert_eq!(node.display, Display::Flex, "victory must show the overlay");

        let mut body_q = app.world_mut().query::<(&PlayerVictoryBodyText, &Text)>();
        let (_, text) = body_q.single(app.world()).unwrap();
        assert!(
            text.0.contains("1850/06/15"),
            "body must show the victory date, got: {}",
            text.0
        );
        assert!(
            text.0.contains("Arcadia"),
            "body must name the victorious country, got: {}",
            text.0
        );
    }

    #[test]
    fn overlay_hides_again_once_acknowledged() {
        let mut app = build_test_app();
        *app.world_mut().resource_mut::<PlayerVictoryMode>() =
            PlayerVictoryMode::VictoryOverlayPending {
                victory_date: "1850/06/15".to_string(),
            };
        app.update();
        *app.world_mut().resource_mut::<PlayerVictoryMode>() = PlayerVictoryMode::Acknowledged;
        app.update();

        let mut query = app
            .world_mut()
            .query::<(&PlayerVictoryOverlayRoot, &Node)>();
        let (_, node) = query.single(app.world()).unwrap();
        assert_eq!(
            node.display,
            Display::None,
            "once dismissed into Acknowledged, the overlay must hide again"
        );
    }

    #[test]
    fn continue_playing_button_transitions_mode_and_closes_overlay() {
        let mut app = build_test_app();
        *app.world_mut().resource_mut::<PlayerVictoryMode>() =
            PlayerVictoryMode::VictoryOverlayPending {
                victory_date: "1850/06/15".to_string(),
            };
        app.update();

        app.world_mut().spawn((
            ContinuePlayingButton,
            Interaction::Pressed,
            BackgroundColor(Color::WHITE),
        ));
        app.update();

        assert_eq!(
            *app.world().resource::<PlayerVictoryMode>(),
            PlayerVictoryMode::Acknowledged
        );
    }
}
