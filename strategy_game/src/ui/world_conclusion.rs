//! P22-002: 観戦中(プレイヤー消滅後)に他国同士の決着で世界が統一された際の
//! 終戦オーバーレイ。
//!
//! `country::WorldConclusionMode`が`OverlayPending`になった瞬間、全画面
//! オーバーレイ(勝者国名・成立日を表示)を出す。ただし、`PlayerEliminationMode`が
//! `DefeatOverlayPending`(プレイヤー自身の敗北オーバーレイをまだ閉じていない)間は
//! このオーバーレイのspawnを抑制し、敗北オーバーレイを先に閉じてから表示する
//! (`ui::player_elimination::PlayerEliminationPlugin`/
//! `ui::player_victory::PlayerVictoryPlugin`と同じ、常設パネルではない一時的
//! モーダルのパターンを踏襲する)。

use crate::country::{CountryRegistry, PlayerEliminationMode, WorldConclusionMode};
use crate::localization::{CurrentLocale, TranslationCatalog, localized_text};
use bevy::prelude::*;

#[derive(Component)]
pub struct WorldConclusionOverlayRoot;

#[derive(Component)]
struct WorldConclusionBodyText;

/// `pub`: P22-002の統合テストが実ボタン押下経由でオーバーレイ解除フローを
/// 外部crateのtests/から駆動するために公開する(`ui::player_elimination::
/// ContinueSpectatingButton`/`ui::peace_panel::PeaceActionButton`と同じ理由)。
#[derive(Component)]
pub struct ContinueSpectatingAfterConclusionButton;

pub struct WorldConclusionPlugin;

impl Plugin for WorldConclusionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(crate::app::game_state::GameState::Playing),
            setup_world_conclusion_overlay,
        )
        .add_systems(
            Update,
            (
                // P22-003: `country::detect_world_conclusion`/`detect_player_elimination`
                // (`DailySimulationSet::UiUpdate`、同じUpdateスケジュール内)との明示的な
                // 順序が無いと、同一フレーム内での実行順序が未規定なため、世界統一が
                // 確定した、または敗北オーバーレイが閉じられたまさにそのフレームで
                // このSystemが先に走ってしまい、オーバーレイの表示切替が1フレーム遅延
                // する場合があった(ロードSystem経由のテストで発見)。このSystemは
                // `WorldConclusionMode`だけでなく`PlayerEliminationMode`(表示抑制判定、
                // §敗北オーバーレイとの競合防止)も読むため、両方のdetect Systemの後に
                // 固定する。
                sync_world_conclusion_overlay
                    .after(crate::country::detect_world_conclusion)
                    .after(crate::country::detect_player_elimination),
                handle_continue_spectating_after_conclusion_button,
            )
                .run_if(in_state(crate::app::game_state::GameState::Playing)),
        )
        .add_systems(
            OnExit(crate::app::game_state::GameState::Playing),
            cleanup_world_conclusion_overlay,
        );
    }
}

fn setup_world_conclusion_overlay(
    mut commands: Commands,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    commands
        .spawn((
            WorldConclusionOverlayRoot,
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
                    BackgroundColor(Color::srgba(0.08, 0.08, 0.1, 1.0)),
                ))
                .with_children(|dialog| {
                    let (title_text, title_marker) =
                        localized_text(&catalog, locale.0, "world_conclusion.title", Vec::new());
                    dialog.spawn((
                        title_text,
                        title_marker,
                        TextColor(Color::srgb(0.8, 0.8, 0.95)),
                        TextFont {
                            font_size: FontSize::Px(24.0),
                            ..default()
                        },
                    ));

                    let (body_text, body_marker) =
                        localized_text(&catalog, locale.0, "world_conclusion.body", Vec::new());
                    dialog.spawn((
                        WorldConclusionBodyText,
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
                            ContinueSpectatingAfterConclusionButton,
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.25, 0.25, 0.3, 1.0)),
                        ))
                        .with_children(|btn| {
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "world_conclusion.continue_button",
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

/// `WorldConclusionMode`または`PlayerEliminationMode`が変化したときだけ
/// 表示/非表示・本文を更新する。`OverlayPending`かつ`PlayerEliminationMode`が
/// `DefeatOverlayPending`でない場合だけ表示する――プレイヤー自身の敗北オーバーレイを
/// 先に閉じるまでは、Domain状態が既にPendingへ進んでいても画面には出さない
/// (§敗北オーバーレイとの競合防止: 表示順序を1.敗北→2.世界統一の順に固定する)。
#[allow(clippy::too_many_arguments)]
fn sync_world_conclusion_overlay(
    mode: Res<WorldConclusionMode>,
    elimination_mode: Res<PlayerEliminationMode>,
    country_registry: Res<CountryRegistry>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut root_q: Query<&mut Node, With<WorldConclusionOverlayRoot>>,
    mut body_q: Query<
        (&mut Text, &mut crate::localization::LocalizedText),
        With<WorldConclusionBodyText>,
    >,
) {
    if !mode.is_changed() && !elimination_mode.is_changed() {
        return;
    }
    let Ok(mut node) = root_q.single_mut() else {
        return;
    };
    let WorldConclusionMode::OverlayPending {
        winner_country_id,
        conclusion_date,
    } = &*mode
    else {
        node.display = Display::None;
        return;
    };
    if *elimination_mode == PlayerEliminationMode::DefeatOverlayPending {
        // プレイヤー自身の敗北オーバーレイがまだ表示中。Domain状態は既にPendingだが、
        // 表示順序(敗北→世界統一)を守るためここではまだ出さない。
        node.display = Display::None;
        return;
    }
    node.display = Display::Flex;

    let winner_name = country_registry
        .get(*winner_country_id)
        .map(|c| c.name.clone())
        .unwrap_or_default();
    let args = vec![("country", winner_name), ("date", conclusion_date.clone())];
    if let Ok((mut text, mut marker)) = body_q.single_mut() {
        let rendered =
            crate::localization::translate(&catalog, locale.0, "world_conclusion.body", &args);
        text.0 = rendered;
        marker.args = args;
    }
}

#[allow(clippy::type_complexity)]
fn handle_continue_spectating_after_conclusion_button(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (
            With<ContinueSpectatingAfterConclusionButton>,
            Changed<Interaction>,
        ),
    >,
    mut mode: ResMut<WorldConclusionMode>,
) {
    for (interaction, mut bg) in interaction_query.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *mode = WorldConclusionMode::Acknowledged;
                *bg = BackgroundColor(Color::srgb(0.4, 0.4, 0.45));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgb(0.32, 0.32, 0.38));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.25, 0.25, 0.3, 1.0));
            }
        }
    }
}

fn cleanup_world_conclusion_overlay(
    mut commands: Commands,
    query: Query<Entity, With<WorldConclusionOverlayRoot>>,
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
        app.insert_resource(WorldConclusionMode::default());
        app.insert_resource(PlayerEliminationMode::Spectating);
        app.insert_resource(CountryRegistry {
            countries: vec![CountryData {
                id: CountryId(1),
                name: "Passguard".to_string(),
                ..CountryData::default()
            }],
        });
        app.add_systems(Startup, setup_world_conclusion_overlay);
        app.add_systems(
            Update,
            (
                sync_world_conclusion_overlay,
                handle_continue_spectating_after_conclusion_button,
            ),
        );
        app.update();
        app
    }

    fn pending_mode() -> WorldConclusionMode {
        WorldConclusionMode::OverlayPending {
            winner_country_id: CountryId(1),
            conclusion_date: "1850/06/15".to_string(),
        }
    }

    #[test]
    fn overlay_is_spawned_hidden_by_default() {
        let mut app = build_test_app();
        let mut query = app
            .world_mut()
            .query::<(&WorldConclusionOverlayRoot, &Node)>();
        let (_, node) = query
            .single(app.world())
            .expect("overlay root must be spawned");
        assert_eq!(node.display, Display::None, "the overlay must start hidden");
    }

    #[test]
    fn overlay_stays_hidden_while_the_defeat_overlay_has_not_been_dismissed() {
        let mut app = build_test_app();
        *app.world_mut().resource_mut::<PlayerEliminationMode>() =
            PlayerEliminationMode::DefeatOverlayPending;
        app.update();
        *app.world_mut().resource_mut::<WorldConclusionMode>() = pending_mode();
        app.update();

        let mut query = app
            .world_mut()
            .query::<(&WorldConclusionOverlayRoot, &Node)>();
        let (_, node) = query.single(app.world()).unwrap();
        assert_eq!(
            node.display,
            Display::None,
            "the world-conclusion overlay must not show while the defeat overlay is still \
             pending, even if the domain state has already reached OverlayPending"
        );
    }

    #[test]
    fn overlay_shows_once_the_defeat_overlay_has_been_dismissed() {
        let mut app = build_test_app();
        // 敗北オーバーレイが先に発生し、既にSpectatingへ遷移済み(=閉じた後)という前提。
        *app.world_mut().resource_mut::<WorldConclusionMode>() = pending_mode();
        app.update();

        let mut root_q = app
            .world_mut()
            .query::<(&WorldConclusionOverlayRoot, &Node)>();
        let (_, node) = root_q.single(app.world()).unwrap();
        assert_eq!(
            node.display,
            Display::Flex,
            "must show once the defeat overlay is no longer pending"
        );

        let mut body_q = app.world_mut().query::<(&WorldConclusionBodyText, &Text)>();
        let (_, text) = body_q.single(app.world()).unwrap();
        assert!(
            text.0.contains("Passguard"),
            "body must name the winning country, got: {}",
            text.0
        );
        assert!(
            text.0.contains("1850/06/15"),
            "body must show the conclusion date, got: {}",
            text.0
        );
    }

    #[test]
    fn overlay_transitions_from_hidden_to_shown_when_the_defeat_overlay_is_dismissed_afterward() {
        let mut app = build_test_app();
        *app.world_mut().resource_mut::<PlayerEliminationMode>() =
            PlayerEliminationMode::DefeatOverlayPending;
        app.update();
        *app.world_mut().resource_mut::<WorldConclusionMode>() = pending_mode();
        app.update();
        {
            let mut query = app
                .world_mut()
                .query::<(&WorldConclusionOverlayRoot, &Node)>();
            let (_, node) = query.single(app.world()).unwrap();
            assert_eq!(node.display, Display::None);
        }

        // プレイヤーが敗北オーバーレイを閉じる(Spectatingへ遷移)。
        *app.world_mut().resource_mut::<PlayerEliminationMode>() =
            PlayerEliminationMode::Spectating;
        app.update();

        let mut query = app
            .world_mut()
            .query::<(&WorldConclusionOverlayRoot, &Node)>();
        let (_, node) = query.single(app.world()).unwrap();
        assert_eq!(
            node.display,
            Display::Flex,
            "dismissing the defeat overlay must reveal the already-pending world-conclusion \
             overlay in the same or a later frame"
        );
    }

    #[test]
    fn overlay_hides_again_once_acknowledged() {
        let mut app = build_test_app();
        *app.world_mut().resource_mut::<WorldConclusionMode>() = pending_mode();
        app.update();
        *app.world_mut().resource_mut::<WorldConclusionMode>() = WorldConclusionMode::Acknowledged;
        app.update();

        let mut query = app
            .world_mut()
            .query::<(&WorldConclusionOverlayRoot, &Node)>();
        let (_, node) = query.single(app.world()).unwrap();
        assert_eq!(
            node.display,
            Display::None,
            "once dismissed into Acknowledged, the overlay must hide again"
        );
    }

    #[test]
    fn continue_button_transitions_mode_and_closes_overlay() {
        let mut app = build_test_app();
        *app.world_mut().resource_mut::<WorldConclusionMode>() = pending_mode();
        app.update();

        app.world_mut().spawn((
            ContinueSpectatingAfterConclusionButton,
            Interaction::Pressed,
            BackgroundColor(Color::WHITE),
        ));
        app.update();

        assert_eq!(
            *app.world().resource::<WorldConclusionMode>(),
            WorldConclusionMode::Acknowledged
        );
    }

    /// P22-001のプレイヤー勝利オーバーレイと、P22-002の観戦者向け世界統一オーバーレイが
    /// 同時に表示されないことを確認する。両者のDomain状態(`PlayerVictoryMode`/
    /// `WorldConclusionMode`)は「プレイヤーが唯一の生存国」と「プレイヤー以外が
    /// 唯一の生存国」という排他的な条件でしか`Pending`にならないため、本来
    /// 同時に両方が`Pending`になることはあり得ない――そのことを、両方のUI Systemを
    /// 同一Appへ登録した上で、`WorldConclusionMode`だけを`OverlayPending`にした状態で
    /// 検証する(プレイヤー勝利オーバーレイのRootは最初から非表示のまま)。
    #[test]
    fn player_victory_overlay_does_not_show_alongside_the_world_conclusion_overlay() {
        use crate::app::game_state::GameState;
        use crate::country::PlayerVictoryMode;
        use crate::ui::player_victory::{PlayerVictoryOverlayRoot, PlayerVictoryPlugin};

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.insert_resource(WorldConclusionMode::default());
        app.insert_resource(PlayerVictoryMode::default());
        app.insert_resource(PlayerEliminationMode::Spectating);
        app.insert_resource(crate::country::PlayerCountry(Some(CountryId(0))));
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
                    ..CountryData::default()
                },
            ],
        });
        app.add_plugins((WorldConclusionPlugin, PlayerVictoryPlugin));
        app.insert_state(GameState::Playing);
        app.update();

        // 観戦者向け世界統一だけがPendingになる(プレイヤー勝利は起きていない)。
        *app.world_mut().resource_mut::<WorldConclusionMode>() = pending_mode();
        app.update();

        let world_flex = {
            let mut q = app
                .world_mut()
                .query::<(&WorldConclusionOverlayRoot, &Node)>();
            q.single(app.world()).unwrap().1.display
        };
        let victory_flex = {
            let mut q = app
                .world_mut()
                .query::<(&PlayerVictoryOverlayRoot, &Node)>();
            q.single(app.world()).unwrap().1.display
        };
        assert_eq!(
            world_flex,
            Display::Flex,
            "world-conclusion overlay must show"
        );
        assert_eq!(
            victory_flex,
            Display::None,
            "player-victory overlay must stay hidden while only the world-conclusion mode is \
             pending"
        );
    }
}
