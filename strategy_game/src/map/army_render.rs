use crate::app::game_state::GameState;
use crate::common::ArmyId;
use crate::country::CountryRegistry;
use crate::map::army_selection::SelectedArmy;
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::data::{ArmyStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use bevy::prelude::*;

#[derive(Component)]
pub struct ArmyVisual {
    pub army_id: ArmyId,
}

pub struct ArmyRenderPlugin;

impl Plugin for ArmyRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                sync_army_visuals,
                update_army_visuals,
                draw_army_paths,
                draw_battle_overlays,
                draw_occupation_overlays,
            )
                .run_if(in_state(GameState::Playing)),
        );
    }
}

fn sync_army_visuals(
    mut commands: Commands,
    military_registry: Res<MilitaryRegistry>,
    country_registry: Res<CountryRegistry>,
    selected_army: Res<SelectedArmy>,
    query: Query<(Entity, &ArmyVisual)>,
) {
    let mut rendered_armies = std::collections::HashSet::new();
    for (entity, visual) in query.iter() {
        if !military_registry.armies.contains_key(&visual.army_id) {
            commands.entity(entity).despawn();
        } else {
            rendered_armies.insert(visual.army_id);
        }
    }

    for (army_id, army) in military_registry.armies.iter() {
        if !rendered_armies.contains(army_id) {
            let base_color = country_registry
                .get(army.owner)
                .map(|c| c.bevy_color())
                .unwrap_or(Color::WHITE);

            let is_selected = selected_army.army_id == Some(*army_id);
            let size = if is_selected {
                Vec2::new(20.0, 20.0)
            } else {
                Vec2::new(14.0, 14.0)
            };

            commands.spawn((
                Sprite {
                    color: base_color,
                    custom_size: Some(size),
                    ..default()
                },
                Transform::from_xyz(0.0, 0.0, 5.0),
                ArmyVisual { army_id: *army_id },
            ));
        }
    }
}

fn update_army_visuals(
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    country_registry: Res<CountryRegistry>,
    selected_army: Res<SelectedArmy>,
    mut query: Query<(&ArmyVisual, &mut Transform, &mut Sprite)>,
) {
    // 州ごとにグループ化して複数ユニットの重ね表示をオフセット
    let mut state_army_indices: std::collections::HashMap<crate::common::StateId, Vec<ArmyId>> =
        std::collections::HashMap::new();

    for (army_id, army) in military_registry.armies.iter() {
        state_army_indices
            .entry(army.current_state)
            .or_default()
            .push(*army_id);
    }
    for list in state_army_indices.values_mut() {
        list.sort_by_key(|id| id.0);
    }

    for (visual, mut transform, mut sprite) in query.iter_mut() {
        if let Some(army) = military_registry.armies.get(&visual.army_id) {
            let start_pos = state_registry
                .get(army.current_state)
                .map(|s| s.position())
                .unwrap_or(Vec2::ZERO);

            let is_selected = selected_army.army_id == Some(visual.army_id);

            // 重なり回避用オフセット (同じ州にいる場合のズレ)
            let mut offset = Vec2::ZERO;
            if let Some(armies_in_state) = state_army_indices.get(&army.current_state)
                && armies_in_state.len() > 1
                && let Some(idx) = armies_in_state.iter().position(|&id| id == visual.army_id)
            {
                let col = (idx % 3) as f32 - 1.0;
                let row = (idx / 3) as f32;
                offset = Vec2::new(col * 7.0, row * 7.0);
            }

            let pos = if let Some(target_state) = army.target_state {
                let target_pos = state_registry
                    .get(target_state)
                    .map(|s| s.position())
                    .unwrap_or(Vec2::ZERO);
                start_pos.lerp(target_pos, army.movement_progress)
            } else {
                start_pos + offset
            };

            transform.translation.x = pos.x;
            transform.translation.y = pos.y;
            transform.translation.z = if is_selected { 6.0 } else { 5.0 };

            let base_color = country_registry
                .get(army.owner)
                .map(|c| c.bevy_color())
                .unwrap_or(Color::WHITE);

            if army.status == ArmyStatus::Fighting {
                // 戦闘中: 赤みがかった色
                sprite.custom_size = Some(Vec2::new(16.0, 16.0));
                sprite.color = Color::srgb(1.0, 0.2, 0.2);
            } else if is_selected {
                // 強調表示: サイズ拡大 & 黄金色
                sprite.custom_size = Some(Vec2::new(20.0, 20.0));
                sprite.color = Color::srgb(1.0, 0.9, 0.3);
            } else {
                sprite.custom_size = Some(Vec2::new(14.0, 14.0));
                sprite.color = base_color;
            }
        }
    }
}

/// 選択中ユニットの移動経路を Gizmos でライン描画
fn draw_army_paths(
    selected_army: Res<SelectedArmy>,
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    mut gizmos: Gizmos,
) {
    let Some(army_id) = selected_army.army_id else {
        return;
    };
    let Some(army) = military_registry.armies.get(&army_id) else {
        return;
    };

    let start_pos = state_registry
        .get(army.current_state)
        .map(|s| s.position())
        .unwrap_or(Vec2::ZERO);

    let mut current_pos = if let Some(target_state) = army.target_state {
        let target_pos = state_registry
            .get(target_state)
            .map(|s| s.position())
            .unwrap_or(Vec2::ZERO);
        start_pos.lerp(target_pos, army.movement_progress)
    } else {
        start_pos
    };

    // target_state への線
    if let Some(target_state) = army.target_state
        && let Some(target_st) = state_registry.get(target_state)
    {
        let next_pos = target_st.position();
        gizmos.line_2d(current_pos, next_pos, Color::srgb(0.2, 0.9, 0.3));
        current_pos = next_pos;
    }

    // current_path の残りのノードへの線
    for &path_state_id in &army.current_path {
        if let Some(st) = state_registry.get(path_state_id) {
            let next_pos = st.position();
            gizmos.line_2d(current_pos, next_pos, Color::srgb(0.9, 0.8, 0.2));
            current_pos = next_pos;
        }
    }

    // 目的地のハイライトマーク
    if army.destination.is_some() || army.target_state.is_some() {
        gizmos.circle_2d(current_pos, 8.0, Color::srgb(0.9, 0.2, 0.2));
    }
}

/// 戦闘中の地域を Gizmos でハイライト表示（赤い枠）
fn draw_battle_overlays(
    battle_registry: Res<BattleRegistry>,
    state_registry: Res<StateRegistry>,
    mut gizmos: Gizmos,
) {
    for battle in battle_registry.battles.values() {
        if battle.status != BattleStatus::Ongoing {
            continue;
        }
        if let Some(state) = state_registry.get(battle.state_id) {
            let pos = state.position();
            let size = state.rect_size();
            let half = size * 0.5;
            // 赤い枠で戦闘中を表示
            gizmos.line_2d(
                pos + Vec2::new(-half.x, -half.y),
                pos + Vec2::new(half.x, -half.y),
                Color::srgb(1.0, 0.1, 0.1),
            );
            gizmos.line_2d(
                pos + Vec2::new(half.x, -half.y),
                pos + Vec2::new(half.x, half.y),
                Color::srgb(1.0, 0.1, 0.1),
            );
            gizmos.line_2d(
                pos + Vec2::new(half.x, half.y),
                pos + Vec2::new(-half.x, half.y),
                Color::srgb(1.0, 0.1, 0.1),
            );
            gizmos.line_2d(
                pos + Vec2::new(-half.x, half.y),
                pos + Vec2::new(-half.x, -half.y),
                Color::srgb(1.0, 0.1, 0.1),
            );
        }
    }
}

/// 所有国と支配国が異なる地域（占領地）を Gizmos でハイライト表示（黄色の枠）
fn draw_occupation_overlays(state_registry: Res<StateRegistry>, mut gizmos: Gizmos) {
    for state in &state_registry.states {
        let controller = state.controller();
        if controller == state.owner_country_id {
            continue; // 所有国=支配国なら表示不要
        }

        let pos = state.position();
        let size = state.rect_size();
        let half = size * 0.5 + Vec2::new(2.0, 2.0); // 少し外側

        let color = Color::srgb(1.0, 0.85, 0.1);
        gizmos.line_2d(
            pos + Vec2::new(-half.x, -half.y),
            pos + Vec2::new(half.x, -half.y),
            color,
        );
        gizmos.line_2d(
            pos + Vec2::new(half.x, -half.y),
            pos + Vec2::new(half.x, half.y),
            color,
        );
        gizmos.line_2d(
            pos + Vec2::new(half.x, half.y),
            pos + Vec2::new(-half.x, half.y),
            color,
        );
        gizmos.line_2d(
            pos + Vec2::new(-half.x, half.y),
            pos + Vec2::new(-half.x, -half.y),
            color,
        );
    }
}
