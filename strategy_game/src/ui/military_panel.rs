use crate::app::game_state::GameState;
use crate::common::DivisionId;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::map::army_selection::SelectedArmy;
use crate::military::data::{ArmyStatus, MilitaryRegistry};
use crate::military::recruitment::RecruitmentQueueItem;
use crate::state::data::StateRegistry;
use bevy::prelude::*;

#[derive(Component)]
pub struct MilitaryPanelRoot;

#[derive(Component)]
pub struct MilitaryPanelText;

#[derive(Component)]
pub struct RecruitButton(pub DivisionId);

#[derive(Resource, Default)]
pub struct MilitaryPanelState {
    pub open: bool,
}

#[derive(Component)]
pub struct ToggleMilitaryPanelButton;

pub struct MilitaryPanelPlugin;

impl Plugin for MilitaryPanelPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MilitaryPanelState::default())
            .add_systems(OnEnter(GameState::Playing), setup_military_panel)
            .add_systems(
                Update,
                (
                    toggle_military_panel_key,
                    update_military_panel_ui,
                    handle_recruit_buttons,
                )
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_military_panel(mut commands: Commands) {
    // トグルボタン
    commands
        .spawn((
            ToggleMilitaryPanelButton,
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(570.0),
                top: Val::Px(45.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.3, 0.2, 0.5, 0.9)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("[M] Military Panel"),
                TextColor(Color::WHITE),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));
        });

    // メインパネル（初期は非表示）
    commands
        .spawn((
            MilitaryPanelRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(310.0),
                top: Val::Px(75.0),
                width: Val::Px(580.0),
                height: Val::Px(600.0),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                display: Display::None,
                overflow: Overflow::clip_y(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.06, 0.14, 0.95)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("-- Military --"),
                TextColor(Color::srgb(0.9, 0.7, 0.5)),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
            ));

            // パネル内テキスト
            parent.spawn((
                MilitaryPanelText,
                Text::new("Loading..."),
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                TextLayout {
                    linebreak: LineBreak::AnyCharacter,
                    ..default()
                },
            ));
        });
}

fn toggle_military_panel_key(
    mut state: ResMut<MilitaryPanelState>,
    mut active_panel: ResMut<crate::ui::ActivePanel>,
    keys: Res<ButtonInput<KeyCode>>,
    btn_q: Query<&Interaction, (With<ToggleMilitaryPanelButton>, Changed<Interaction>)>,
    mut panel_q: Query<&mut Node, With<MilitaryPanelRoot>>,
) {
    let mut toggle = false;
    if keys.just_pressed(KeyCode::KeyM) {
        toggle = true;
    }
    for interaction in btn_q.iter() {
        if *interaction == Interaction::Pressed {
            toggle = true;
        }
    }

    if toggle {
        active_panel.toggle(crate::ui::PanelKind::Military);
        state.open = active_panel.current == crate::ui::PanelKind::Military;
        if let Ok(mut node) = panel_q.single_mut() {
            node.display = if state.open {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_military_panel_ui(
    state: Res<MilitaryPanelState>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    selected_army: Res<SelectedArmy>,
    mut text_q: Query<&mut Text, With<MilitaryPanelText>>,
    _commands: Commands,
    _panel_q: Query<Entity, With<MilitaryPanelRoot>>,
) {
    if !state.open {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };
    let Some(country) = country_registry.get(player_cid) else {
        return;
    };

    // 自国の軍隊を集計
    let my_armies: Vec<_> = military_registry
        .armies
        .values()
        .filter(|a| a.owner == player_cid)
        .collect();

    let mut lines = Vec::new();

    lines.push(format!(
        "人的資源: {} / 動員済み: {}",
        country.available_manpower, country.mobilized_manpower
    ));
    lines.push(format!(
        "軍維持費: {:.1} G/月",
        country.monthly_military_expenses
    ));
    lines.push(format!(
        "募集キュー: {} 件",
        country.recruitment_queue.len()
    ));
    lines.push("".to_string());

    if let Some(army) = selected_army
        .army_id
        .and_then(|id| military_registry.armies.get(&id))
    {
        lines.push("── 選択中ユニット詳細 ──".to_string());
        let owner_name = country_registry
            .get(army.owner)
            .map(|c| c.name.as_str())
            .unwrap_or("Unknown");
        let current_state_name = state_registry
            .get(army.current_state)
            .map(|s| s.name.as_str())
            .unwrap_or("Unknown");
        lines.push(format!("ID: Army #{} | 所有国: {}", army.id.0, owner_name));
        lines.push(format!("現在位置: {}", current_state_name));

        let status_str = match army.status {
            ArmyStatus::Idle => "待機中",
            ArmyStatus::Moving => "移動中",
            ArmyStatus::Fighting => "戦闘中",
            ArmyStatus::Occupying => "占領中",
            ArmyStatus::Retreating => "退却中",
            ArmyStatus::Disbanding => "解散中",
        };
        lines.push(format!("状態: {}", status_str));

        if let Some(dest_id) = army.destination {
            let dest_name = state_registry
                .get(dest_id)
                .map(|s| s.name.as_str())
                .unwrap_or("Unknown");
            let remaining_steps =
                army.current_path.len() + if army.target_state.is_some() { 1 } else { 0 };
            let est_days =
                (remaining_steps as f32 * 5.0 * (1.0 - army.movement_progress)).ceil() as u32;
            lines.push(format!(
                "目的地: {} (残り {} 州 / 推定 {} 日)",
                dest_name, remaining_steps, est_days
            ));
        } else {
            lines.push("目的地: なし".to_string());
        }
        lines.push("".to_string());
    }

    lines.push(format!("── 部隊一覧 ({} 部隊) ──", my_armies.len()));

    for army in &my_armies {
        let state_name = state_registry
            .get(army.current_state)
            .map(|s| s.name.as_str())
            .unwrap_or("Unknown");
        let status_str = match army.status {
            ArmyStatus::Idle => "待機",
            ArmyStatus::Moving => "移動中",
            ArmyStatus::Fighting => "戦闘中",
            ArmyStatus::Occupying => "占領中",
            ArmyStatus::Retreating => "退却中",
            ArmyStatus::Disbanding => "解散中",
        };
        let selected = selected_army.army_id == Some(army.id);
        let sel_mark = if selected { "► " } else { "  " };
        lines.push(format!(
            "{}{} [{:?}/{:?}] @ {} | {} | 士気:{:.0}%",
            sel_mark,
            army.id.0,
            army.division_type,
            army.size,
            state_name,
            status_str,
            army.morale / army.max_morale * 100.0,
        ));
    }

    lines.push("".to_string());
    lines.push("── 師団招募 ──".to_string());
    lines.push("選択中の州に招募されます".to_string());
    for def in military_registry.definitions.values() {
        lines.push(format!(
            "[Recruit] {} ({:?}/{:?}) 人員:{} 日数:{}日",
            def.name, def.division_type, def.size, def.required_manpower, def.recruitment_days,
        ));
    }

    if let Ok(mut text) = text_q.single_mut() {
        text.0 = lines.join("\n");
    }
}

fn handle_recruit_buttons(
    btn_q: Query<(&Interaction, &RecruitButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
    military_registry: Res<MilitaryRegistry>,
    selected_state: Res<crate::state::SelectedState>,
) {
    let Some(player_cid) = player_country.0 else {
        return;
    };

    let target_state = match selected_state.0 {
        Some(s) => s,
        None => return,
    };

    for (interaction, btn) in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let def_id = btn.0;
        let Some(def) = military_registry.definitions.get(&def_id) else {
            continue;
        };

        let Some(country) = country_registry.get_mut(player_cid) else {
            continue;
        };

        if country.available_manpower < def.required_manpower {
            continue; // 人員不足
        }

        country.recruitment_queue.push(RecruitmentQueueItem {
            division_id: def_id,
            target_state,
            days_remaining: def.recruitment_days,
            total_days: def.recruitment_days,
        });

        country.mobilized_manpower += def.required_manpower;
        country.available_manpower = country
            .available_manpower
            .saturating_sub(def.required_manpower);
    }
}
