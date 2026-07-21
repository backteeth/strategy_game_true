use crate::app::game_state::GameState;
use crate::building::construction::{ConstructionQueueItem, ConstructionStatus, REFUND_RATIO};
use crate::building::data::{BuildingRegistry, BuildingType};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::state::SelectedState;
use crate::state::data::StateRegistry;
use crate::ui::JapaneseFont;
use crate::ui::notification::GameNotification;
use bevy::prelude::*;

#[derive(Component)]
pub struct StatePanelRoot;

#[derive(Component)]
pub struct StatePanelText;

#[derive(Component)]
pub struct BuildButton(pub BuildingType);

pub struct StatePanelPlugin;

impl Plugin for StatePanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), setup_state_panel)
            .add_systems(
                Update,
                (
                    update_state_panel,
                    handle_build_buttons,
                    handle_cancel_queue_buttons,
                )
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_state_panel(mut commands: Commands, font: Res<JapaneseFont>) {
    commands
        .spawn((
            StatePanelRoot,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(0.0),
                top: Val::Px(40.0),
                bottom: Val::Px(0.0),
                width: Val::Px(340.0),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                overflow: Overflow::clip_y(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.1, 0.9)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("-- 州詳細 & 建設 --"),
                TextColor(Color::srgb(0.9, 0.85, 0.6)),
                TextFont {
                    font: font.0.clone().into(),
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
            ));

            parent.spawn((
                StatePanelText,
                Text::new("マップ上の州をクリックして選択"),
                TextColor(Color::srgb(0.85, 0.85, 0.9)),
                TextFont {
                    font: font.0.clone().into(),
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));

            parent.spawn((
                Text::new("[ 建設可能な建物 ]"),
                TextColor(Color::srgb(0.7, 0.9, 0.7)),
                TextFont {
                    font: font.0.clone().into(),
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));

            for b_type in BuildingType::ALL {
                parent
                    .spawn((
                        BuildButton(b_type),
                        Button,
                        Node {
                            width: Val::Percent(100.0),
                            padding: UiRect::all(Val::Px(4.0)),
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            margin: UiRect::bottom(Val::Px(2.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.2, 0.25, 0.3, 1.0)),
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            Text::new(format!("建設: {}", b_type.display_name())),
                            TextColor(Color::WHITE),
                            TextFont {
                                font: font.0.clone().into(),
                                font_size: FontSize::Px(12.0),
                                ..default()
                            },
                        ));
                    });
            }
        });
}

fn update_state_panel(
    selected: Res<SelectedState>,
    state_registry: Res<StateRegistry>,
    country_registry: Res<CountryRegistry>,
    mut text_q: Query<&mut Text, With<StatePanelText>>,
) {
    let Ok(mut text) = text_q.single_mut() else {
        return;
    };

    let Some(state_id) = selected.0 else {
        *text = Text::new("マップ上の州をクリックして選択");
        return;
    };

    let Some(state) = state_registry.get(state_id) else {
        return;
    };

    let country_name = country_registry
        .get(state.owner_country_id)
        .map(|c| c.name.as_str())
        .unwrap_or("不明");

    let mut b_str = String::new();
    for b_type in BuildingType::ALL {
        let lvl = state.building_level(b_type);
        if lvl > 0 {
            let op = state.building_operation(b_type);
            b_str.push_str(&format!(
                "  {}: Lv.{} (稼働率: {:.0}%)\n",
                b_type.display_name(),
                lvl,
                op * 100.0
            ));
        }
    }
    if b_str.is_empty() {
        b_str.push_str("  (なし)\n");
    }

    let mut dep_str = String::new();
    for dep in &state.resource_deposits {
        if dep.discovered {
            dep_str.push_str(&format!(
                "  {}: 基礎産出量 {:.0}/月\n",
                dep.resource_type.display_name(),
                dep.base_output
            ));
        }
    }
    if dep_str.is_empty() {
        dep_str.push_str("  (なし)\n");
    }

    let total_wf = state.total_workforce();

    let info = format!(
        "州名: {}\n領有国: {}\n人口: {} | 労働力: {}\n就業者: {} | 失業者: {}\n生活水準: {:.1} / 100\n不満度: {:.1} / 100\n物流容量: {:.0} / 使用量: {:.0} (充足率: {:.0}%)\n\n[資源鉱床]\n{}\n[州内建物]\n{}",
        state.name,
        country_name,
        format_population(state.population),
        format_population(total_wf),
        format_population(state.employed_workforce),
        format_population(state.unemployed_workforce),
        state.living_standard,
        state.unrest,
        state.logistics_capacity,
        state.logistics_usage,
        state.logistics_ratio * 100.0,
        dep_str,
        b_str,
    );

    if text.0 != info {
        *text = Text::new(info);
    }
}

fn handle_build_buttons(
    mut interaction_q: Query<
        (&Interaction, &BuildButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
    selected: Res<SelectedState>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    building_registry: Res<BuildingRegistry>,
    mut notif_writer: MessageWriter<GameNotification>,
) {
    let Some(state_id) = selected.0 else {
        return;
    };

    let Some(player_cid) = player_country.0 else {
        return;
    };

    let Some(state) = state_registry.get(state_id) else {
        return;
    };

    for (interaction, btn, mut bg) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.4, 0.5, 0.6, 1.0));

                if state.owner_country_id != player_cid {
                    notif_writer.write(GameNotification {
                        message: "建設失敗: 自国の州ではありません".to_string(),
                    });
                    continue;
                }

                let def = match building_registry.get(btn.0) {
                    Some(d) => d,
                    None => continue,
                };

                let current_level = state.building_level(btn.0);
                let country = match country_registry.get_mut(player_cid) {
                    Some(c) => c,
                    None => continue,
                };

                if current_level >= def.max_level {
                    notif_writer.write(GameNotification {
                        message: format!(
                            "建設失敗: {} は既に最大レベル (Lv.{}) です",
                            btn.0.display_name(),
                            def.max_level
                        ),
                    });
                    continue;
                }

                let in_queue = country
                    .construction_queue
                    .iter()
                    .any(|item| item.state_id == state_id && item.building_type == btn.0);
                if in_queue {
                    notif_writer.write(GameNotification {
                        message: format!(
                            "建設失敗: {} にて {} は既に建設キューに存在します",
                            state.name,
                            btn.0.display_name()
                        ),
                    });
                    continue;
                }

                if country.treasury < def.construction_cost {
                    notif_writer.write(GameNotification {
                        message: format!(
                            "建設失敗: 資金不足です (必要: {:.0} G, 所持: {:.0} G)",
                            def.construction_cost, country.treasury
                        ),
                    });
                    continue;
                }

                country.treasury -= def.construction_cost;
                country.construction_queue.push(ConstructionQueueItem {
                    state_id,
                    building_type: btn.0,
                    target_level: current_level + 1,
                    progress: 0.0,
                    required_progress: def.required_progress,
                    paid_cost: def.construction_cost,
                    status: ConstructionStatus::InQueue,
                });

                notif_writer.write(GameNotification {
                    message: format!(
                        "建設開始: {} ({}, Lv.{})",
                        btn.0.display_name(),
                        state.name,
                        current_level + 1
                    ),
                });
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.3, 0.35, 0.45, 1.0));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.2, 0.25, 0.3, 1.0));
            }
        }
    }
}

fn handle_cancel_queue_buttons(
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
    mut notif_writer: MessageWriter<GameNotification>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if keys.just_pressed(KeyCode::KeyC) {
        let Some(player_cid) = player_country.0 else {
            return;
        };
        let Some(country) = country_registry.get_mut(player_cid) else {
            return;
        };

        if let Some(item) = country.construction_queue.pop() {
            let refund = item.paid_cost * REFUND_RATIO;
            country.treasury += refund;

            notif_writer.write(GameNotification {
                message: format!(
                    "建設キャンセル: {} (返金: {:.0} G)",
                    item.building_type.display_name(),
                    refund
                ),
            });
        }
    }
}

pub fn format_population(pop: u64) -> String {
    if pop >= 1_000_000 {
        format!("{:.2}M", pop as f64 / 1_000_000.0)
    } else if pop >= 1_000 {
        format!("{:.1}K", pop as f64 / 1_000.0)
    } else {
        format!("{}", pop)
    }
}
