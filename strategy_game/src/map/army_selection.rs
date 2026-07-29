use crate::app::game_state::GameState;
use crate::common::ArmyId;
use crate::country::PlayerCountry;
use crate::map::camera::GameCamera;
use crate::military::data::{ArmyStatus, MilitaryRegistry};
use crate::state::SelectedState;
use crate::state::data::StateRegistry;
use bevy::prelude::*;

#[derive(Resource, Default, Debug)]
pub struct SelectedArmy {
    pub army_id: Option<ArmyId>,
}

pub struct ArmySelectionPlugin;

impl Plugin for ArmySelectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SelectedArmy>().add_systems(
            Update,
            (handle_army_selection, handle_movement_order).run_if(in_state(GameState::Playing)),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_army_selection(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_q: Query<&Transform, With<GameCamera>>,
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    ui_interactions_q: Query<&Interaction>,
    mut selected_army: ResMut<SelectedArmy>,
    mut selected_state: ResMut<SelectedState>,
) {
    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }

    for interaction in ui_interactions_q.iter() {
        if *interaction == Interaction::Hovered || *interaction == Interaction::Pressed {
            return;
        }
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(cam_transform) = camera_q.single() else {
        return;
    };

    let Some(cursor_screen) = window.cursor_position() else {
        return;
    };

    let window_size = Vec2::new(window.width(), window.height());
    let cursor_ndc = (cursor_screen / window_size) * 2.0 - Vec2::ONE;
    let cursor_ndc = Vec2::new(cursor_ndc.x, -cursor_ndc.y);
    let scale = cam_transform.scale.x;
    let half_size = window_size * 0.5 * scale;
    let world_pos = cam_transform.translation.xy() + cursor_ndc * half_size;

    let mut hit_army = None;
    let army_radius = 16.0;

    for (army_id, army) in military_registry.armies.iter() {
        let mut pos = state_registry
            .get(army.current_state)
            .map(|s| s.position())
            .unwrap_or(Vec2::ZERO);

        if let Some(target_state) = army.target_state {
            let target_pos = state_registry
                .get(target_state)
                .map(|s| s.position())
                .unwrap_or(Vec2::ZERO);
            pos = pos.lerp(target_pos, army.movement_progress);
        }

        if world_pos.distance(pos) < army_radius {
            hit_army = Some(*army_id);
            break;
        }
    }

    if let Some(army_id) = hit_army {
        selected_army.army_id = Some(army_id);
        // ユニット選択時は州選択をクリアしてUIの競合を抑止
        selected_state.0 = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_movement_order(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_q: Query<&Transform, With<GameCamera>>,
    player_country: Res<PlayerCountry>,
    state_registry: Res<StateRegistry>,
    selected_army: Res<SelectedArmy>,
    mut military_registry: ResMut<MilitaryRegistry>,
    ui_interactions_q: Query<&Interaction>,
) {
    if !mouse_buttons.just_pressed(MouseButton::Right) {
        return;
    }

    for interaction in ui_interactions_q.iter() {
        if *interaction == Interaction::Hovered || *interaction == Interaction::Pressed {
            return;
        }
    }

    let Some(army_id) = selected_army.army_id else {
        return;
    };

    let Some(player_cid) = player_country.0 else {
        return;
    };

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(cam_transform) = camera_q.single() else {
        return;
    };

    let Some(cursor_screen) = window.cursor_position() else {
        return;
    };

    let window_size = Vec2::new(window.width(), window.height());
    let cursor_ndc = (cursor_screen / window_size) * 2.0 - Vec2::ONE;
    let cursor_ndc = Vec2::new(cursor_ndc.x, -cursor_ndc.y);
    let scale = cam_transform.scale.x;
    let half_size = window_size * 0.5 * scale;
    let world_pos = cam_transform.translation.xy() + cursor_ndc * half_size;

    let mut target_state = None;
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
            target_state = Some(state.id);
            break;
        }
    }

    let Some(target) = target_state else {
        return;
    };

    let Some(army) = military_registry.armies.get_mut(&army_id) else {
        return;
    };

    // 他国ユニットへの命令防止
    if army.owner != player_cid {
        warn!(
            "[ArmyMovement] Cannot command Army {}: Unit belongs to country {:?}",
            army.id.0, army.owner
        );
        return;
    }

    if army.current_state == target && army.destination == Some(target) {
        return;
    }

    // 経路探索
    let path = crate::military::pathfinding::find_path(
        army.current_state,
        target,
        &state_registry,
        &[army.owner],
        &[],
    );

    if let Some(path) = path {
        army.destination = Some(target);
        army.current_path = path;
        army.target_state = None;
        army.movement_progress = 0.0;
        army.status = ArmyStatus::Moving;
        info!(
            "[ArmyMovement] Army {} path set to {:?} ({} steps)",
            army.id.0,
            target,
            army.current_path.len()
        );
    } else {
        warn!(
            "[ArmyMovement] Army {} cannot reach state {:?}: No valid path in own territory",
            army.id.0, target
        );
    }
}
