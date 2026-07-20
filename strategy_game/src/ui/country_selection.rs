use crate::app::game_state::GameState;
use crate::common::CountryId;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::state::data::StateRegistry;
/// 国家選択画面 UI
/// CountrySelection 状態時に表示され、プレイヤー国家を確定する
use bevy::prelude::*;

/// 画面全体のルートノード
#[derive(Component)]
pub struct CountrySelectionRoot;

/// 国家リストのボタン
#[derive(Component)]
pub struct CountrySelectButton(pub CountryId);

/// 選択中プレビュー国家
#[derive(Resource, Default)]
pub struct PreviewCountry(pub Option<CountryId>);

/// プレビュー詳細テキスト
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

/// UI構築
fn setup_ui(
    mut commands: Commands,
    country_registry: Res<CountryRegistry>,
    mut preview: ResMut<PreviewCountry>,
) {
    // 初期選択は1番目の国家
    preview.0 = country_registry.countries.first().map(|c| c.id);

    // フルスクリーンコンテナ
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
            // 左側: 国家リスト
            root.spawn((Node {
                width: Val::Percent(30.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            },))
                .with_children(|left_panel| {
                    left_panel.spawn((
                        Text::new("Select Your Nation"),
                        TextFont {
                            font_size: FontSize::Px(24.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.9, 0.9, 0.9)),
                    ));

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

            // 右側: プレビュー詳細と開始ボタン
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
                    // 詳細テキスト
                    right_panel.spawn((
                        PreviewDetailText,
                        Text::new("Select a nation..."),
                        TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        TextFont {
                            font_size: FontSize::Px(20.0),
                            ..default()
                        },
                    ));

                    // 開始ボタン
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
                            btn.spawn((
                                Text::new("Start Game"),
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

/// 国家リストボタンのクリック
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

/// プレビュー詳細テキストの更新
fn update_preview_details(
    preview: Res<PreviewCountry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    mut text_query: Query<&mut Text, With<PreviewDetailText>>,
) {
    if !preview.is_changed() {
        return;
    }

    let Ok(mut text) = text_query.single_mut() else {
        return;
    };

    if let Some(country_id) = preview.0 {
        if let Some(country) = country_registry.get(country_id) {
            let capital_name = state_registry
                .get(country.capital_state_id)
                .map(|s| s.name.as_str())
                .unwrap_or("Unknown");

            // 人口合算
            let total_pop: u64 = state_registry
                .states
                .iter()
                .filter(|s| s.owner_country_id == country_id)
                .map(|s| s.population)
                .sum();

            let info = format!(
                "Name: {}\nGovernment: {}\nEconomy: {}\nTotal Pop: {}\nTreasury: {}\nCapital: {}",
                country.name,
                country.government_type.display_name(),
                country.economic_system.display_name(),
                crate::ui::state_panel::format_population(total_pop),
                country.treasury,
                capital_name
            );
            *text = Text::new(info);
        }
    }
}

/// 開始ボタンのクリック処理
fn handle_start_button(
    interaction_query: Query<&Interaction, (With<StartGameButton>, Changed<Interaction>)>,
    preview: Res<PreviewCountry>,
    mut player_country: ResMut<PlayerCountry>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    for interaction in interaction_query.iter() {
        if *interaction == Interaction::Pressed {
            if let Some(id) = preview.0 {
                // プレイヤー国家を確定して遷移
                player_country.0 = Some(id);
                next_state.set(GameState::Playing);
            }
        }
    }
}

/// ステート退出時にUI破棄
fn cleanup_ui(mut commands: Commands, query: Query<Entity, With<CountrySelectionRoot>>) {
    if let Ok(entity) = query.single() {
        commands.entity(entity).despawn();
    }
}
