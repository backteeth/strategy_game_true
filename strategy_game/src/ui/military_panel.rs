use crate::app::game_state::GameState;
use crate::common::DivisionId;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::map::army_selection::SelectedArmy;
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::data::{ArmyStatus, MilitaryRegistry};
use crate::military::recruitment::RecruitmentQueueItem;
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;
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
                width: Val::Px(600.0),
                height: Val::Px(650.0),
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
    war_registry: Res<WarRegistry>,
    battle_registry: Res<BattleRegistry>,
    mut frontline_registry: ResMut<crate::war::frontline::FrontlineRegistry>,
    ai_registry: Res<crate::war::military_ai::MilitaryAiRegistry>,
    country_ai_registry: Res<crate::country::country_ai::CountryAiRegistry>,
    frontline_settings: Res<crate::map::frontline_render::FrontlineRenderSettings>,
    selected_army: Res<SelectedArmy>,
    keys: Res<ButtonInput<KeyCode>>,
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

    // プレイヤー参加中のアクティブ戦争と前線を取得
    let active_war = war_registry.get_active_war_for_country(player_cid);
    let frontline =
        active_war.and_then(|w| frontline_registry.get_frontline_for_war(w.id).cloned());

    // --- キー操作による前線コントロール ---
    if let Some(ref fl) = frontline {
        // [Key1] 選択中陸軍を前線へ割り当て
        if keys.just_pressed(KeyCode::Digit1)
            && let Some(army_id) = selected_army.army_id
        {
            let _ = frontline_registry.assign_army(
                army_id,
                fl.frontline_id,
                player_cid,
                &military_registry,
                &war_registry,
            );
        }
        // [Key2] 選択中陸軍を前線から解除
        if keys.just_pressed(KeyCode::Digit2)
            && let Some(army_id) = selected_army.army_id
        {
            frontline_registry.unassign_army(army_id);
        }
        // [Key3] 全部隊の割り当て解除
        if keys.just_pressed(KeyCode::Digit3) {
            frontline_registry.unassign_all_armies_for_plan(fl.frontline_id, player_cid);
        }
        // [Key7] 停止 (Stopped)
        if keys.just_pressed(KeyCode::Digit7)
            && let Some(plan) = frontline_registry.get_plan_mut(fl.frontline_id, player_cid)
        {
            plan.stance = crate::war::frontline::FrontlineStance::Stopped;
        }
        // [Key8] 防御 (Defend)
        if keys.just_pressed(KeyCode::Digit8)
            && let Some(plan) = frontline_registry.get_plan_mut(fl.frontline_id, player_cid)
        {
            plan.stance = crate::war::frontline::FrontlineStance::Defend;
        }
        // [Key9] 攻勢 (Offensive)
        if keys.just_pressed(KeyCode::Digit9)
            && let Some(plan) = frontline_registry.get_plan_mut(fl.frontline_id, player_cid)
        {
            plan.stance = crate::war::frontline::FrontlineStance::Offensive;
        }
    }

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
        "前線表示 [F]: {}",
        if frontline_settings.visible {
            "ON"
        } else {
            "OFF"
        }
    ));
    lines.push("".to_string());

    // 前線・作戦命令表示
    lines.push("── 前線・作戦命令 ──".to_string());
    if let (Some(war), Some(fl)) = (active_war, frontline.as_ref()) {
        let is_attacker = war.attackers.contains(&player_cid);
        let atk_name = country_registry
            .get(fl.attacker_country_id)
            .map(|c| c.name.as_str())
            .unwrap_or("Attacker");
        let def_name = country_registry
            .get(fl.defender_country_id)
            .map(|c| c.name.as_str())
            .unwrap_or("Defender");

        lines.push(format!(
            "前線ID: Frontline #{} (戦争: {})",
            fl.frontline_id.0, war.name
        ));
        lines.push(format!(
            "交戦国: {} vs {} | 国境ペア数: {}",
            atk_name,
            def_name,
            fl.border_region_pairs.len()
        ));
        lines.push(format!(
            "自国前線地域: {} 州 | 敵側前線地域: {} 州",
            if is_attacker {
                fl.attacker_front_regions.len()
            } else {
                fl.defender_front_regions.len()
            },
            if is_attacker {
                fl.defender_front_regions.len()
            } else {
                fl.attacker_front_regions.len()
            }
        ));

        if fl.border_region_pairs.is_empty() {
            lines.push("【注意】直接接触する国境地域が存在しません (空の前線)".to_string());
        }

        let plan = frontline_registry.get_plan(fl.frontline_id, player_cid);
        let stance = plan.map(|p| p.stance).unwrap_or_default();
        let assigned_ids = plan.map(|p| p.assigned_army_ids.as_slice()).unwrap_or(&[]);

        lines.push(format!("命令状態: [{}]", stance.display_name()));

        if let Some(obj_id) = plan.and_then(|p| p.objective_region_id) {
            let obj_name = state_registry
                .get(obj_id)
                .map(|s| s.name.as_str())
                .unwrap_or("Unknown");
            lines.push(format!("攻勢目標地域: {} (#{})", obj_name, obj_id.0));
        } else {
            lines.push("攻勢目標地域: 未設定".to_string());
        }

        // 割当部隊の内訳集計
        let mut count_idle = 0;
        let mut count_moving = 0;
        let mut count_fighting = 0;
        for &id in assigned_ids {
            if let Some(a) = military_registry.armies.get(&id) {
                match a.status {
                    ArmyStatus::Idle => count_idle += 1,
                    ArmyStatus::Moving => count_moving += 1,
                    ArmyStatus::Fighting => count_fighting += 1,
                    _ => {}
                }
            }
        }
        lines.push(format!(
            "割り当て部隊: {} 部隊 (待機/戦闘可能: {} / 移動中: {} / 戦闘中: {})",
            assigned_ids.len(),
            count_idle,
            count_moving,
            count_fighting
        ));

        lines.push(
            "操作キー: [1]選択部隊を割当 [2]割当解除 [3]全解除 [7]停止 [8]防御 [9]攻勢".to_string(),
        );
    } else {
        lines.push("進行中の戦争がないため、前線は形成されていません。".to_string());
    }
    lines.push("".to_string());

    // AI軍事作戦状況
    if !ai_registry.ai_states.is_empty() {
        lines.push("── 軍事AI作戦状況 (AI国家) ──".to_string());
        let mut ai_countries: Vec<_> = ai_registry.ai_states.values().collect();
        ai_countries.sort_by_key(|a| a.country_id.0);

        for ai in ai_countries {
            let country_name = country_registry
                .get(ai.country_id)
                .map(|c| c.name.as_str())
                .unwrap_or("AI Country");
            lines.push(format!(
                "[{}] 自軍戦力:{} / 敵戦力:{} | 判断: {}",
                country_name,
                ai.estimated_own_power,
                ai.estimated_enemy_power,
                ai.last_decision_reason.display_name()
            ));
        }
        lines.push("".to_string());
    }

    // 国家AI運営状況
    if !country_ai_registry.ai_states.is_empty() {
        lines.push("── 国家AI運営状況 (AI国家) ──".to_string());
        let mut country_ais: Vec<_> = country_ai_registry.ai_states.values().collect();
        country_ais.sort_by_key(|c| c.country_id.0);

        for cai in country_ais {
            let country_name = country_registry
                .get(cai.country_id)
                .map(|c| c.name.as_str())
                .unwrap_or("AI Country");
            lines.push(format!(
                "[{}] モード:{} | 判断: {}",
                country_name,
                cai.mode.display_name(),
                cai.decision_reason.display_name()
            ));
        }
        lines.push("".to_string());
    }

    // 選択中ユニット詳細
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

        // 前線割り当て状態
        if let Some(fl_id) = frontline_registry.army_frontline_map.get(&army.id) {
            lines.push(format!("前線割り当て: Frontline #{}", fl_id.0));
        } else {
            lines.push("前線割り当て: 未割り当て".to_string());
        }

        // 割り当て不可理由の判定と表示
        if army.owner != player_cid {
            lines.push("【操作不可】自国の陸軍ではありません".to_string());
        } else if active_war.is_none() {
            lines.push("【割当不可】進行中の戦争が存在しません".to_string());
        } else if army.manpower == 0 || army.status == ArmyStatus::Destroyed {
            lines.push("【割当不可】部隊が撃破済みまたは戦力0です".to_string());
        }

        // 戦力・組織率
        lines.push(format!(
            "戦力: {} / {} ({:.0}%)",
            army.manpower,
            army.max_manpower,
            army.manpower as f32 / army.max_manpower as f32 * 100.0
        ));
        lines.push(format!(
            "組織率: {:.0} / {:.0} ({:.0}%)",
            army.organization,
            army.max_organization,
            army.organization / army.max_organization * 100.0
        ));
        lines.push(format!(
            "攻撃力: {} | 防御力: {}",
            army.attack_power, army.defense_power
        ));

        let status_str = match army.status {
            ArmyStatus::Idle => "待機中",
            ArmyStatus::Moving => "移動中",
            ArmyStatus::Fighting => "戦闘中",
            ArmyStatus::Occupying => "占領中",
            ArmyStatus::Retreating => "退却中",
            ArmyStatus::Disbanding => "解散中",
            ArmyStatus::Destroyed => "撃破",
        };
        lines.push(format!("状態: {}", status_str));

        if let Some(dest_id) = army.destination {
            let dest_name = state_registry
                .get(dest_id)
                .map(|s| s.name.as_str())
                .unwrap_or("Unknown");
            lines.push(format!("目的地: {}", dest_name));
        } else {
            lines.push("目的地: なし".to_string());
        }
        lines.push("".to_string());
    }

    // 進行中の戦闘一覧
    let ongoing_battles: Vec<_> = battle_registry
        .battles
        .values()
        .filter(|b| b.status == BattleStatus::Ongoing)
        .collect();

    if !ongoing_battles.is_empty() {
        lines.push(format!("── 進行中の戦闘 ({} 件) ──", ongoing_battles.len()));
        for battle in &ongoing_battles {
            let battle_state_name = state_registry
                .get(battle.state_id)
                .map(|s| s.name.as_str())
                .unwrap_or("?");
            let atk_name = country_registry
                .get(battle.attacker_country)
                .map(|c| c.name.as_str())
                .unwrap_or("?");
            let def_name = country_registry
                .get(battle.defender_country)
                .map(|c| c.name.as_str())
                .unwrap_or("?");

            lines.push(format!(
                "[戦闘] {} | {} vs {}",
                battle_state_name, atk_name, def_name
            ));
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
            ArmyStatus::Destroyed => "撃破",
        };
        let fl_tag = if frontline_registry.army_frontline_map.contains_key(&army.id) {
            " [前線]"
        } else {
            ""
        };
        let selected = selected_army.army_id == Some(army.id);
        let sel_mark = if selected { "► " } else { "  " };
        lines.push(format!(
            "{}#{} @ {} | {}{} | 兵力:{}",
            sel_mark, army.id.0, state_name, status_str, fl_tag, army.manpower,
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
