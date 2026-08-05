use crate::app::game_state::GameState;
use crate::common::DivisionId;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::localization::{CurrentLocale, TranslationCatalog, localized_text, t, tf};
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

fn setup_military_panel(
    mut commands: Commands,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
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
            let (text, marker) =
                localized_text(&catalog, locale.0, "military_panel.toggle_button", vec![]);
            parent.spawn((
                text,
                marker,
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
            let (text, marker) = localized_text(&catalog, locale.0, "military_panel.title", vec![]);
            parent.spawn((
                text,
                marker,
                TextColor(Color::srgb(0.9, 0.7, 0.5)),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
            ));

            // パネル内テキスト
            // NOTE: このパネルは複数の翻訳キーを1行ずつ結合した合成テキストであり、
            // 単一の翻訳キーで表現できないため、意図的に`LocalizedText`マーカーを付与しない
            // (汎用の`retranslate_on_locale_change`による上書きを避ける)。
            // 言語切り替え時の再翻訳は`update_military_panel_ui`自身が
            // `!state.open && !locale.is_changed()`ガードにより担う。
            parent.spawn((
                MilitaryPanelText,
                Text::new(t(&catalog, locale.0, "military_panel.loading")),
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

fn army_status_key(status: ArmyStatus) -> &'static str {
    match status {
        ArmyStatus::Idle => "army_status.idle",
        ArmyStatus::Moving => "army_status.moving",
        ArmyStatus::Fighting => "army_status.fighting",
        ArmyStatus::Occupying => "army_status.occupying",
        ArmyStatus::Retreating => "army_status.retreating",
        ArmyStatus::Disbanding => "army_status.disbanding",
        ArmyStatus::Destroyed => "army_status.destroyed",
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
    loc: crate::localization::Loc,
    mut text_q: Query<&mut Text, With<MilitaryPanelText>>,
) {
    let locale = &loc.locale;
    let catalog = &loc.catalog;
    if !state.open && !locale.is_changed() {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };
    let Some(country) = country_registry.get(player_cid) else {
        return;
    };

    let tr = |key: &'static str| t(catalog, locale.0, key);
    let trf =
        |key: &'static str, args: Vec<(&'static str, String)>| tf(catalog, locale.0, key, args);

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

    lines.push(trf(
        "military_panel.manpower",
        vec![
            ("available", country.available_manpower.to_string()),
            ("mobilized", country.mobilized_manpower.to_string()),
        ],
    ));
    lines.push(trf(
        "military_panel.upkeep",
        vec![("cost", format!("{:.1}", country.monthly_military_expenses))],
    ));
    lines.push(trf(
        "military_panel.frontline_visibility",
        vec![(
            "state",
            tr(if frontline_settings.visible {
                "common.on"
            } else {
                "common.off"
            }),
        )],
    ));
    lines.push(String::new());

    // 前線・作戦命令表示
    lines.push(tr("military_panel.frontline_orders_header"));
    if let (Some(war), Some(fl)) = (active_war, frontline.as_ref()) {
        let is_attacker = war.attackers.contains(&player_cid);
        let atk_name = country_registry
            .get(fl.attacker_country_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| tr("common.unknown"));
        let def_name = country_registry
            .get(fl.defender_country_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| tr("common.unknown"));

        lines.push(trf(
            "military_panel.frontline_id",
            vec![
                ("id", fl.frontline_id.0.to_string()),
                ("war", war.name.clone()),
            ],
        ));
        lines.push(trf(
            "military_panel.frontline_belligerents",
            vec![
                ("attacker", atk_name),
                ("defender", def_name),
                ("pairs", fl.border_region_pairs.len().to_string()),
            ],
        ));
        lines.push(trf(
            "military_panel.frontline_regions",
            vec![
                (
                    "own",
                    if is_attacker {
                        fl.attacker_front_regions.len()
                    } else {
                        fl.defender_front_regions.len()
                    }
                    .to_string(),
                ),
                (
                    "enemy",
                    if is_attacker {
                        fl.defender_front_regions.len()
                    } else {
                        fl.attacker_front_regions.len()
                    }
                    .to_string(),
                ),
            ],
        ));

        if fl.border_region_pairs.is_empty() {
            lines.push(tr("military_panel.frontline_no_border"));
        }

        let plan = frontline_registry.get_plan(fl.frontline_id, player_cid);
        let stance = plan.map(|p| p.stance).unwrap_or_default();
        let assigned_ids = plan.map(|p| p.assigned_army_ids.as_slice()).unwrap_or(&[]);

        lines.push(trf(
            "military_panel.order_state",
            vec![("stance", tr(stance.display_name()))],
        ));

        if let Some(obj_id) = plan.and_then(|p| p.objective_region_id) {
            let obj_name = state_registry
                .get(obj_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| tr("common.unknown"));
            lines.push(trf(
                "military_panel.objective_set",
                vec![("state", obj_name), ("id", obj_id.0.to_string())],
            ));
        } else {
            lines.push(tr("military_panel.objective_unset"));
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
        lines.push(trf(
            "military_panel.assigned_units",
            vec![
                ("total", assigned_ids.len().to_string()),
                ("idle", count_idle.to_string()),
                ("moving", count_moving.to_string()),
                ("fighting", count_fighting.to_string()),
            ],
        ));

        lines.push(tr("military_panel.controls_hint"));
    } else {
        lines.push(tr("military_panel.no_active_war"));
    }
    lines.push(String::new());

    // AI軍事作戦状況
    if !ai_registry.ai_states.is_empty() {
        lines.push(tr("military_panel.ai_ops_header"));
        let mut ai_countries: Vec<_> = ai_registry.ai_states.values().collect();
        ai_countries.sort_by_key(|a| a.country_id.0);

        for ai in ai_countries {
            let country_name = country_registry
                .get(ai.country_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| tr("common.unknown"));
            lines.push(trf(
                "military_panel.ai_ops_line",
                vec![
                    ("country", country_name),
                    ("own_power", ai.estimated_own_power.to_string()),
                    ("enemy_power", ai.estimated_enemy_power.to_string()),
                    ("reason", tr(ai.last_decision_reason.display_name())),
                ],
            ));
        }
        lines.push(String::new());
    }

    // 国家AI運営状況
    if !country_ai_registry.ai_states.is_empty() {
        lines.push(tr("military_panel.country_ai_header"));
        let mut country_ais: Vec<_> = country_ai_registry.ai_states.values().collect();
        country_ais.sort_by_key(|c| c.country_id.0);

        for cai in country_ais {
            let country_name = country_registry
                .get(cai.country_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| tr("common.unknown"));
            lines.push(trf(
                "military_panel.country_ai_line",
                vec![
                    ("country", country_name),
                    ("mode", tr(cai.mode.display_name())),
                    ("reason", tr(cai.decision_reason.display_name())),
                ],
            ));
        }
        lines.push(String::new());
    }

    // 選択中ユニット詳細
    if let Some(army) = selected_army
        .army_id
        .and_then(|id| military_registry.armies.get(&id))
    {
        lines.push(tr("military_panel.selected_unit_header"));
        let owner_name = country_registry
            .get(army.owner)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| tr("common.unknown"));
        let current_state_name = state_registry
            .get(army.current_state)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| tr("common.unknown"));
        lines.push(trf(
            "military_panel.selected_unit_id",
            vec![("id", army.id.0.to_string()), ("owner", owner_name)],
        ));
        lines.push(trf(
            "military_panel.selected_unit_location",
            vec![("state", current_state_name)],
        ));

        // 前線割り当て状態
        if let Some(fl_id) = frontline_registry.army_frontline_map.get(&army.id) {
            lines.push(trf(
                "military_panel.frontline_assigned",
                vec![("id", fl_id.0.to_string())],
            ));
        } else {
            lines.push(tr("military_panel.frontline_unassigned"));
        }

        // 割り当て不可理由の判定と表示
        if army.owner != player_cid {
            lines.push(tr("military_panel.not_own_army"));
        } else if active_war.is_none() {
            lines.push(tr("military_panel.no_active_war_assign"));
        } else if army.manpower == 0 || army.status == ArmyStatus::Destroyed {
            lines.push(tr("military_panel.destroyed_or_no_power"));
        }

        // 戦力・組織率
        lines.push(trf(
            "military_panel.strength",
            vec![
                ("current", army.manpower.to_string()),
                ("max", army.max_manpower.to_string()),
                (
                    "percent",
                    format!(
                        "{:.0}",
                        army.manpower as f32 / army.max_manpower as f32 * 100.0
                    ),
                ),
            ],
        ));
        lines.push(trf(
            "military_panel.organization",
            vec![
                ("current", format!("{:.0}", army.organization)),
                ("max", format!("{:.0}", army.max_organization)),
                (
                    "percent",
                    format!("{:.0}", army.organization / army.max_organization * 100.0),
                ),
            ],
        ));
        lines.push(trf(
            "military_panel.power",
            vec![
                ("attack", army.attack_power.to_string()),
                ("defense", army.defense_power.to_string()),
            ],
        ));

        lines.push(trf(
            "military_panel.status_line",
            vec![("status", tr(army_status_key(army.status)))],
        ));

        if let Some(dest_id) = army.destination {
            let dest_name = state_registry
                .get(dest_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| tr("common.unknown"));
            lines.push(trf(
                "military_panel.destination_set",
                vec![("state", dest_name)],
            ));
        } else {
            lines.push(tr("military_panel.destination_none"));
        }
        lines.push(String::new());
    }

    // 進行中の戦闘一覧
    let ongoing_battles: Vec<_> = battle_registry
        .battles
        .values()
        .filter(|b| b.status == BattleStatus::Ongoing)
        .collect();

    if !ongoing_battles.is_empty() {
        lines.push(trf(
            "military_panel.ongoing_battles_header",
            vec![("count", ongoing_battles.len().to_string())],
        ));
        for battle in &ongoing_battles {
            let battle_state_name = state_registry
                .get(battle.state_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "?".to_string());
            let atk_name = country_registry
                .get(battle.attacker_country)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "?".to_string());
            let def_name = country_registry
                .get(battle.defender_country)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "?".to_string());

            lines.push(trf(
                "military_panel.battle_line",
                vec![
                    ("state", battle_state_name),
                    ("attacker", atk_name),
                    ("defender", def_name),
                ],
            ));
        }
        lines.push(String::new());
    }

    lines.push(trf(
        "military_panel.army_list_header",
        vec![("count", my_armies.len().to_string())],
    ));

    for army in &my_armies {
        let state_name = state_registry
            .get(army.current_state)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| tr("common.unknown"));
        let status_str = tr(army_status_key(army.status));
        let fl_tag = if frontline_registry.army_frontline_map.contains_key(&army.id) {
            tr("military_panel.frontline_tag")
        } else {
            String::new()
        };
        let selected = selected_army.army_id == Some(army.id);
        let sel_mark = if selected { "► " } else { "  " };
        lines.push(trf(
            "military_panel.army_line",
            vec![
                ("mark", sel_mark.to_string()),
                ("id", army.id.0.to_string()),
                ("state", state_name),
                ("status", status_str),
                ("frontline_tag", fl_tag),
                ("manpower", army.manpower.to_string()),
            ],
        ));
    }

    if let Ok(mut text) = text_q.single_mut() {
        let joined = lines.join("\n");
        if text.0 != joined {
            text.0 = joined;
        }
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
