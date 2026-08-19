use crate::app::game_state::GameState;
use crate::common::StateId;
use crate::country::PlayerCountry;
use crate::map::camera::GameCamera;
use crate::map::division_selection::screen_to_world;
use crate::map::frontline_selection::FrontlineSelectMode;
use crate::map::offensive_line_selection::OffensiveLineEditMode;
use crate::military::army::{ArmyRegistry, SelectedArmy};
use crate::state::data::StateRegistry;
use crate::war::data::{WarRegistry, WarStatus};
use crate::war::frontline::{FrontlineRegistry, FrontlineStance};
use bevy::prelude::*;
use std::collections::HashSet;

/// 前線オーバーレイ表示のトグルリソース
#[derive(Resource, Debug, Clone)]
pub struct FrontlineRenderSettings {
    pub visible: bool,
}

impl Default for FrontlineRenderSettings {
    fn default() -> Self {
        Self { visible: true }
    }
}

/// 前線ハイライトスプライトのマーカー
#[derive(Component)]
pub struct FrontlineOverlayVisual;

pub struct FrontlineRenderPlugin;

impl Plugin for FrontlineRenderPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FrontlineRenderSettings::default())
            .add_systems(
                Update,
                (toggle_frontline_overlay_key, update_frontline_overlay)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn toggle_frontline_overlay_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<FrontlineRenderSettings>,
) {
    if keys.just_pressed(KeyCode::KeyF) {
        settings.visible = !settings.visible;
    }
}

/// 現在のマウスカーソルが指している州を返す(見つからなければNone)。
/// `map::division_selection::handle_division_selection`等と同じクリック判定の考え方
/// (Window/Cameraからワールド座標へ変換し、州の矩形と当たり判定する)を、前線選択モードの
/// hover強調表示のためだけに読み取り専用で流用する。
fn hovered_state_id(
    windows: &Query<&Window>,
    camera_q: &Query<&Transform, With<GameCamera>>,
    state_registry: &StateRegistry,
) -> Option<StateId> {
    let window = windows.single().ok()?;
    let cam_transform = camera_q.single().ok()?;
    let cursor_screen = window.cursor_position()?;
    let window_size = Vec2::new(window.width(), window.height());
    let world_pos = screen_to_world(cursor_screen, window_size, cam_transform);

    for state in &state_registry.states {
        let pos = state.position();
        let size = state.rect_size();
        let min = pos - size * 0.5;
        let max = pos + size * 0.5;
        if world_pos.x >= min.x
            && world_pos.x <= max.x
            && world_pos.y >= min.y
            && world_pos.y <= max.y
        {
            return Some(state.id);
        }
    }
    None
}

/// P21-007: 中抜き矩形の縁取り(4本の細い帯スプライト)を1州ぶん生成する。既存の
/// 塗りつぶしオーバーレイ群(前線・攻勢目標・選択モード強調)とは視覚的に異なる
/// 「線」として攻勢線を表現するために使う。Gizmosは使わない(`GizmoPlugin`が
/// `MinimalPlugins`に無く、このファイルの`MinimalPlugins`ベースのテストが壊れるため。
/// `map::army_render`のGizmosベース所有者マーカーが専用の未テストSystemへ分離している
/// のと同じ理由でSprite方式を選ぶ)。生成した4枚は他のオーバーレイと同じく
/// `FrontlineOverlayVisual`を付与し、毎フレームの全despawn→再構築サイクルに含める。
fn spawn_region_outline(
    commands: &mut Commands,
    center: Vec2,
    size: Vec2,
    color: Color,
    thickness: f32,
    z: f32,
) {
    let half = size * 0.5;
    let bars = [
        (
            Vec2::new(center.x, center.y + half.y - thickness * 0.5),
            Vec2::new(size.x, thickness),
        ),
        (
            Vec2::new(center.x, center.y - half.y + thickness * 0.5),
            Vec2::new(size.x, thickness),
        ),
        (
            Vec2::new(center.x - half.x + thickness * 0.5, center.y),
            Vec2::new(thickness, size.y),
        ),
        (
            Vec2::new(center.x + half.x - thickness * 0.5, center.y),
            Vec2::new(thickness, size.y),
        ),
    ];
    for (pos, bar_size) in bars {
        commands.spawn((
            Sprite {
                color,
                custom_size: Some(bar_size),
                ..default()
            },
            Transform::from_xyz(pos.x, pos.y, z),
            FrontlineOverlayVisual,
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn update_frontline_overlay(
    mut commands: Commands,
    settings: Res<FrontlineRenderSettings>,
    player_country: Res<PlayerCountry>,
    war_registry: Res<WarRegistry>,
    state_registry: Res<StateRegistry>,
    frontline_registry: Res<FrontlineRegistry>,
    overlay_q: Query<Entity, With<FrontlineOverlayVisual>>,
    frontline_select_mode: Res<FrontlineSelectMode>,
    offensive_line_edit_mode: Res<OffensiveLineEditMode>,
    army_registry: Res<ArmyRegistry>,
    selected_army: Res<SelectedArmy>,
    windows: Query<&Window>,
    camera_q: Query<&Transform, With<GameCamera>>,
) {
    // 既存のオーバーレイをクリア
    for entity in overlay_q.iter() {
        commands.entity(entity).despawn();
    }

    if !settings.visible {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };

    // プレイヤーが参加中のアクティブ戦争前線を対象にする
    let active_wars: Vec<_> = war_registry
        .wars
        .values()
        .filter(|w| {
            w.status == WarStatus::Active
                && (w.attackers.contains(&player_cid) || w.defenders.contains(&player_cid))
        })
        .collect();

    for war in active_wars {
        let Some(frontline) = frontline_registry.get_frontline_for_war(war.id) else {
            continue;
        };

        let is_attacker = war.attackers.contains(&player_cid);
        let (my_front, enemy_front) = if is_attacker {
            (
                &frontline.attacker_front_regions,
                &frontline.defender_front_regions,
            )
        } else {
            (
                &frontline.defender_front_regions,
                &frontline.attacker_front_regions,
            )
        };

        let plan = frontline_registry.get_plan(frontline.frontline_id, player_cid);
        let objective = plan.and_then(|p| p.objective_region_id);
        let stance = plan.map(|p| p.stance).unwrap_or(FrontlineStance::Stopped);

        // 自国側前線地域のハイライト (水色/青系)
        let my_color = match stance {
            FrontlineStance::Stopped => Color::srgba(0.2, 0.6, 0.9, 0.45),
            FrontlineStance::Defend => Color::srgba(0.1, 0.8, 0.4, 0.5),
            FrontlineStance::Offensive => Color::srgba(0.9, 0.3, 0.2, 0.5),
        };

        for &state_id in my_front {
            if let Some(state) = state_registry.get(state_id) {
                let pos = state.position();
                let size = state.rect_size() + Vec2::new(6.0, 6.0);

                commands.spawn((
                    Sprite {
                        color: my_color,
                        custom_size: Some(size),
                        ..default()
                    },
                    Transform::from_xyz(pos.x, pos.y, 1.2),
                    FrontlineOverlayVisual,
                ));
            }
        }

        // 敵側前線地域のハイライト (赤/橙系)
        let enemy_set: HashSet<StateId> = enemy_front.iter().copied().collect();
        for &state_id in &enemy_set {
            if let Some(state) = state_registry.get(state_id) {
                let pos = state.position();
                let size = state.rect_size() + Vec2::new(4.0, 4.0);

                commands.spawn((
                    Sprite {
                        color: Color::srgba(0.9, 0.1, 0.1, 0.35),
                        custom_size: Some(size),
                        ..default()
                    },
                    Transform::from_xyz(pos.x, pos.y, 1.15),
                    FrontlineOverlayVisual,
                ));
            }
        }

        // 攻勢目標地域のハイライト (金/黄色枠)
        if let Some(obj_id) = objective
            && let Some(state) = state_registry.get(obj_id)
        {
            let pos = state.position();
            let size = state.rect_size() + Vec2::new(10.0, 10.0);

            commands.spawn((
                Sprite {
                    color: Color::srgba(1.0, 0.85, 0.1, 0.6),
                    custom_size: Some(size),
                    ..default()
                },
                Transform::from_xyz(pos.x, pos.y, 1.25),
                FrontlineOverlayVisual,
            ));
        }

        // P21-007: 確定済み攻勢線のハイライト(計画データのみ)。既存の塗りつぶし
        // オーバーレイ群とは異なる色(オレンジ)・線種(中抜き矩形の縁取り)で表示する。
        if let Some(line_ids) = plan.and_then(|p| p.offensive_line_region_ids.as_ref()) {
            let mut sorted_ids: Vec<StateId> = line_ids.clone();
            sorted_ids.sort_by_key(|s| s.0);
            for state_id in sorted_ids {
                if let Some(state) = state_registry.get(state_id) {
                    let pos = state.position();
                    let size = state.rect_size() + Vec2::new(10.0, 10.0);
                    spawn_region_outline(
                        &mut commands,
                        pos,
                        size,
                        Color::srgba(1.0, 0.55, 0.0, 0.95),
                        3.0,
                        1.3,
                    );
                }
            }
        }
    }

    // P21-005: 前線選択モード中、割当可能な前線の自国側地域を強調する。hover中の地域は
    // さらに強調する。色・線幅は既存の前線オーバーレイと同系統(半透明の重ね塗り)に揃え、
    // 過度に派手な新規デザインは追加しない。
    if let Some(army_id) = frontline_select_mode.army_id
        && let Some(select_army) = army_registry.armies.get(&army_id)
    {
        let assignable =
            frontline_registry.assignable_frontlines_for_army(select_army.owner, &war_registry);
        let hovered = hovered_state_id(&windows, &camera_q, &state_registry);

        for fl_id in assignable {
            let Some(fl) = frontline_registry.frontlines.get(&fl_id) else {
                continue;
            };
            let own_front = if fl.attacker_country_id == select_army.owner {
                &fl.attacker_front_regions
            } else {
                &fl.defender_front_regions
            };
            for &state_id in own_front {
                let Some(state) = state_registry.get(state_id) else {
                    continue;
                };
                let pos = state.position();
                let (color, pad, z) = if hovered == Some(state_id) {
                    (
                        Color::srgba(1.0, 1.0, 1.0, 0.75),
                        Vec2::new(14.0, 14.0),
                        1.4,
                    )
                } else {
                    (
                        Color::srgba(1.0, 1.0, 1.0, 0.4),
                        Vec2::new(10.0, 10.0),
                        1.35,
                    )
                };
                commands.spawn((
                    Sprite {
                        color,
                        custom_size: Some(state.rect_size() + pad),
                        ..default()
                    },
                    Transform::from_xyz(pos.x, pos.y, z),
                    FrontlineOverlayVisual,
                ));
            }
        }
    }

    // P21-005: 選択中Armyが前線へ割り当て済みなら、モードの有無に関わらず常に
    // その前線の自国側地域を識別可能にする(Army UIの配色系統(紫)に揃える)。
    if let Some(army_id) = selected_army.0
        && let Some(fl_id) = frontline_registry.frontline_for_army(army_id)
        && let Some(fl) = frontline_registry.frontlines.get(&fl_id)
        && let Some(army) = army_registry.armies.get(&army_id)
    {
        let own_front = if fl.attacker_country_id == army.owner {
            &fl.attacker_front_regions
        } else {
            &fl.defender_front_regions
        };
        for &state_id in own_front {
            if let Some(state) = state_registry.get(state_id) {
                let pos = state.position();
                commands.spawn((
                    Sprite {
                        color: Color::srgba(0.5, 0.3, 0.55, 0.55),
                        custom_size: Some(state.rect_size() + Vec2::new(8.0, 8.0)),
                        ..default()
                    },
                    Transform::from_xyz(pos.x, pos.y, 1.28),
                    FrontlineOverlayVisual,
                ));
            }
        }
    }

    // P21-007: 攻勢線編集モード中は、選択可能な地点(このFrontlineの戦争における
    // 敵国支配の陸地すべて。既存の前線オーバーレイのような国境地域限定ではない)を
    // 塗りつぶしで、hover中の地点はさらに強調して表示する。draft(未確定)は
    // 確定済み攻勢線よりも明確に強調するため、より太い縁取り・より強い色で表示する。
    if let (Some(fl_id), Some(country_id)) = (
        offensive_line_edit_mode.frontline_id,
        offensive_line_edit_mode.country_id,
    ) && let Some(fl) = frontline_registry.frontlines.get(&fl_id)
    {
        let enemy_id = if country_id == fl.attacker_country_id {
            fl.defender_country_id
        } else {
            fl.attacker_country_id
        };
        let hovered = hovered_state_id(&windows, &camera_q, &state_registry);

        let mut candidates: Vec<StateId> = state_registry
            .states
            .iter()
            .filter(|s| !s.is_sea && s.controller() == enemy_id)
            .map(|s| s.id)
            .collect();
        candidates.sort_by_key(|s| s.0);

        for state_id in candidates {
            let Some(state) = state_registry.get(state_id) else {
                continue;
            };
            let pos = state.position();
            let (color, pad, z) = if hovered == Some(state_id) {
                (
                    Color::srgba(1.0, 0.95, 0.3, 0.75),
                    Vec2::new(14.0, 14.0),
                    1.42,
                )
            } else {
                (Color::srgba(1.0, 0.9, 0.3, 0.3), Vec2::new(8.0, 8.0), 1.32)
            };
            commands.spawn((
                Sprite {
                    color,
                    custom_size: Some(state.rect_size() + pad),
                    ..default()
                },
                Transform::from_xyz(pos.x, pos.y, z),
                FrontlineOverlayVisual,
            ));
        }

        let mut draft_ids = offensive_line_edit_mode.draft.clone();
        draft_ids.sort_by_key(|s| s.0);
        for state_id in draft_ids {
            if let Some(state) = state_registry.get(state_id) {
                let pos = state.position();
                let size = state.rect_size() + Vec2::new(14.0, 14.0);
                spawn_region_outline(
                    &mut commands,
                    pos,
                    size,
                    Color::srgba(1.0, 0.1, 0.85, 0.95),
                    5.0,
                    1.45,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{ArmyId, CountryId, DivisionId, FrontlineId, WarId};
    use crate::military::data::MilitaryRegistry;
    use crate::state::data::StateData;
    use crate::war::data::War;
    use crate::war::frontline::update_all_frontlines;
    use std::collections::HashSet as StdHashSet;

    fn setup_world() -> (App, FrontlineId) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        let s1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(2)],
            world_position: [0.0, 0.0],
            size: [100.0, 100.0],
            ..Default::default()
        };
        let s2 = StateData {
            id: StateId(2),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(1)],
            world_position: [300.0, 0.0],
            size: [100.0, 100.0],
            ..Default::default()
        };
        let state_registry = StateRegistry::build(vec![s1, s2]);

        let mut war_registry = WarRegistry::default();
        let war = War {
            id: WarId(0),
            name: "Test War".to_string(),
            attackers: [CountryId(1)].into_iter().collect(),
            defenders: [CountryId(2)].into_iter().collect(),
            primary_attacker: None,
            primary_defender: None,
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            end_date: None,
            duration_days: 0,
            war_score: 0.0,
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: StdHashSet::new(),
            status: WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: StdHashSet::new(),
        };
        war_registry.wars.insert(war.id, war);

        let military_registry = MilitaryRegistry::default();
        let mut frontline_registry = FrontlineRegistry::default();
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(0))
            .unwrap()
            .frontline_id;

        app.insert_resource(FrontlineRenderSettings::default());
        app.insert_resource(PlayerCountry(Some(CountryId(1))));
        app.insert_resource(war_registry);
        app.insert_resource(state_registry);
        app.insert_resource(frontline_registry);
        app.insert_resource(military_registry);
        app.insert_resource(FrontlineSelectMode::default());
        app.insert_resource(OffensiveLineEditMode::default());
        app.insert_resource(ArmyRegistry::default());
        app.insert_resource(SelectedArmy::default());
        app.add_systems(Update, update_frontline_overlay);

        (app, fl_id)
    }

    fn overlay_count(app: &mut App) -> usize {
        let world = app.world_mut();
        world.query::<&FrontlineOverlayVisual>().iter(world).count()
    }

    /// 通常表示(選択モード非アクティブ)では、自国側/敵側/攻勢目標のオーバーレイのみが
    /// 描画される。
    #[test]
    fn normal_overlay_renders_without_select_mode_highlights() {
        let (mut app, _fl_id) = setup_world();
        app.update();
        assert!(
            overlay_count(&mut app) > 0,
            "通常表示でも前線オーバーレイ自体は描画されるはず"
        );
    }

    /// 要求テスト項目39: 前線選択モード中は割当可能な前線が追加のオーバーレイで
    /// 強調表示される(モード非アクティブ時より描画数が増える)。
    #[test]
    fn select_mode_adds_highlight_overlays_for_assignable_frontlines() {
        let (mut app, _fl_id) = setup_world();
        app.update();
        let baseline = overlay_count(&mut app);

        let division = crate::military::data::Division {
            id: DivisionId(0),
            owner: CountryId(1),
            division_type: crate::military::data::DivisionType::Infantry,
            size: crate::military::data::DivisionSize::Standard,
            current_state: StateId(1),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 1000,
            max_manpower: 1000,
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 1.0,
            max_morale: 1.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: crate::military::data::DivisionStatus::Idle,
            def_id: crate::common::DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let army_id = app
            .world_mut()
            .resource_scope(|world, mut mil: Mut<MilitaryRegistry>| {
                mil.divisions.insert(division.id, division);
                world
                    .resource_mut::<ArmyRegistry>()
                    .create_army(CountryId(1), &[DivisionId(0)], &mil)
                    .unwrap()
            });
        app.world_mut()
            .resource_mut::<FrontlineSelectMode>()
            .activate(army_id);

        app.update();
        let with_mode = overlay_count(&mut app);
        assert!(
            with_mode > baseline,
            "前線選択モード中は強調表示ぶんオーバーレイ数が増えるはず(baseline={baseline}, with_mode={with_mode})"
        );
    }

    /// 要求テスト項目41: 選択中Armyが前線へ割り当て済みなら、モードの有無に関わらず
    /// 識別用のオーバーレイが常に描画される。
    #[test]
    fn assigned_army_frontline_is_highlighted_even_without_select_mode() {
        let (mut app, fl_id) = setup_world();

        let division = crate::military::data::Division {
            id: DivisionId(0),
            owner: CountryId(1),
            division_type: crate::military::data::DivisionType::Infantry,
            size: crate::military::data::DivisionSize::Standard,
            current_state: StateId(1),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 1000,
            max_manpower: 1000,
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 1.0,
            max_morale: 1.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: crate::military::data::DivisionStatus::Idle,
            def_id: crate::common::DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let army_id = app
            .world_mut()
            .resource_scope(|world, mut mil: Mut<MilitaryRegistry>| {
                mil.divisions.insert(division.id, division);
                world
                    .resource_mut::<ArmyRegistry>()
                    .create_army(CountryId(1), &[DivisionId(0)], &mil)
                    .unwrap()
            });

        app.update();
        let baseline = overlay_count(&mut app);

        app.world_mut()
            .resource_scope(|world, army_registry: Mut<ArmyRegistry>| {
                world.resource_scope(|world, war_registry: Mut<WarRegistry>| {
                    world
                        .resource_mut::<FrontlineRegistry>()
                        .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
                        .unwrap();
                });
            });
        app.world_mut().resource_mut::<SelectedArmy>().0 = Some(army_id);

        app.update();
        let with_assignment = overlay_count(&mut app);
        assert!(
            with_assignment > baseline,
            "割当済みArmy選択中はモード無しでも識別オーバーレイが増えるはず"
        );
    }

    /// 要求テスト項目43/44: フレームをまたいで状態が変化しなければオーバーレイ総数は
    /// 増え続けない(既存の毎フレーム全despawn→再構築パターンにより保証される)。
    #[test]
    fn overlay_count_does_not_grow_across_unchanged_frames() {
        let (mut app, _fl_id) = setup_world();
        app.update();
        let first = overlay_count(&mut app);
        app.update();
        let second = overlay_count(&mut app);
        app.update();
        let third = overlay_count(&mut app);
        assert_eq!(first, second);
        assert_eq!(second, third);
    }

    /// 要求テスト項目43: Frontline削除後は古いオーバーレイが残らない。
    #[test]
    fn overlay_disappears_after_frontline_removed() {
        let (mut app, fl_id) = setup_world();
        app.update();
        assert!(overlay_count(&mut app) > 0);

        app.world_mut()
            .resource_scope(|world, military_registry: Mut<MilitaryRegistry>| {
                world
                    .resource_mut::<FrontlineRegistry>()
                    .remove_frontline(fl_id, &military_registry);
            });

        app.update();
        assert_eq!(
            overlay_count(&mut app),
            0,
            "前線削除後は古いオーバーレイが一切残ってはならない"
        );
    }

    /// P21-007: 確定済み攻勢線が設定されていると、専用のオーバーレイ(縁取り4枚)が
    /// 追加で描画される。
    #[test]
    fn committed_offensive_line_adds_outline_overlays() {
        let (mut app, fl_id) = setup_world();
        app.update();
        let baseline = overlay_count(&mut app);

        app.world_mut()
            .resource_mut::<FrontlineRegistry>()
            .get_plan_mut(fl_id, CountryId(1))
            .unwrap()
            .offensive_line_region_ids = Some(vec![StateId(2)]);

        app.update();
        let with_line = overlay_count(&mut app);
        assert!(
            with_line >= baseline + 4,
            "攻勢線1州につき4枚の縁取りスプライトが追加されるはず(baseline={baseline}, with_line={with_line})"
        );
    }

    /// P21-007要求: 攻勢線編集モード中は、選択可能な敵支配地域のハイライトが追加される。
    #[test]
    fn edit_mode_adds_candidate_highlight_overlays() {
        let (mut app, fl_id) = setup_world();
        app.update();
        let baseline = overlay_count(&mut app);

        let mut mode = OffensiveLineEditMode::default();
        mode.activate(ArmyId(0), fl_id, CountryId(1), Vec::new());
        app.insert_resource(mode);

        app.update();
        let with_mode = overlay_count(&mut app);
        assert!(
            with_mode > baseline,
            "編集モード中は候補地域の強調表示ぶんオーバーレイ数が増えるはず(baseline={baseline}, with_mode={with_mode})"
        );
    }

    /// P21-007要求: 編集中のdraftは、候補ハイライトだけの状態よりもさらにオーバーレイが
    /// 増える(専用の縁取りが追加されるため、確定済み表示より明確に強調される)。
    #[test]
    fn edit_mode_draft_adds_more_overlays_than_candidate_highlight_alone() {
        let (mut app, fl_id) = setup_world();

        let mut mode = OffensiveLineEditMode::default();
        mode.activate(ArmyId(0), fl_id, CountryId(1), Vec::new());
        app.insert_resource(mode);
        app.update();
        let candidate_only = overlay_count(&mut app);

        app.world_mut()
            .resource_mut::<OffensiveLineEditMode>()
            .toggle_region(StateId(2));

        app.update();
        let with_draft = overlay_count(&mut app);
        assert!(
            with_draft >= candidate_only + 4,
            "draftへの追加で縁取り4枚ぶんオーバーレイが増えるはず(candidate_only={candidate_only}, with_draft={with_draft})"
        );
    }

    /// P21-007要求: 編集モードをキャンセルすると、編集中特有のオーバーレイは残らない
    /// (毎フレーム全despawn→再構築のため、モードが非アクティブに戻れば自動的に消える)。
    #[test]
    fn edit_mode_overlays_disappear_after_cancel() {
        let (mut app, fl_id) = setup_world();
        app.update();
        let baseline = overlay_count(&mut app);

        let mut mode = OffensiveLineEditMode::default();
        mode.activate(ArmyId(0), fl_id, CountryId(1), vec![StateId(2)]);
        app.insert_resource(mode);
        app.update();
        assert!(overlay_count(&mut app) > baseline);

        app.world_mut()
            .resource_mut::<OffensiveLineEditMode>()
            .cancel();

        app.update();
        assert_eq!(
            overlay_count(&mut app),
            baseline,
            "編集モードをキャンセルすると編集中特有のオーバーレイは一切残らないはず"
        );
    }
}
