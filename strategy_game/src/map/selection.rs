use crate::app::game_state::GameState;
use crate::map::camera::GameCamera;
use crate::map::rendering::StateVisual;
use crate::state::{SelectedState, StateSelectionChanged};
use bevy::prelude::*;

pub struct SelectionPlugin;

impl Plugin for SelectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (handle_state_click, update_state_visuals).run_if(in_state(GameState::Playing)),
        );
    }
}

/// マウス左クリックで州を選択・解除する
/// UI上の要素をホバー/クリックしている場合は無効化し、競合を避ける
fn handle_state_click(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_q: Query<&Transform, With<GameCamera>>,
    state_visuals_q: Query<(&Transform, &StateVisual)>,
    ui_interactions_q: Query<&Interaction>,
    mut selected: ResMut<SelectedState>,
    mut selection_changed: MessageWriter<StateSelectionChanged>,
) {
    // 左クリックが押された瞬間のみ処理
    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }

    // UIノードの上にマウスがある（Hovered or Pressed）場合はマップ選択をスキップ
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

    let mut hit_state = None;
    let mut best_z = f32::NEG_INFINITY;

    for (transform, visual) in state_visuals_q.iter() {
        let pos = transform.translation.xy();
        let half = visual.size * 0.5;

        let hit = world_pos.x >= pos.x - half.x
            && world_pos.x <= pos.x + half.x
            && world_pos.y >= pos.y - half.y
            && world_pos.y <= pos.y + half.y;

        if hit && transform.translation.z > best_z {
            best_z = transform.translation.z;
            hit_state = Some(visual.state_id);
        }
    }

    if selected.0 != hit_state {
        selected.0 = hit_state;
        selection_changed.write(StateSelectionChanged(hit_state));
    }
}

fn update_state_visuals(
    mut selection_changed: MessageReader<StateSelectionChanged>,
    selected: Res<SelectedState>,
    mut state_visuals_q: Query<(&mut Sprite, &StateVisual)>,
) {
    if selection_changed.is_empty() {
        return;
    }
    selection_changed.clear();

    for (mut sprite, visual) in state_visuals_q.iter_mut() {
        if Some(visual.state_id) == selected.0 {
            sprite.color = brighten_color(visual.base_color, 1.5);
        } else {
            sprite.color = visual.base_color;
        }
    }
}

fn brighten_color(color: Color, factor: f32) -> Color {
    let linear = color.to_linear();
    Color::linear_rgb(
        (linear.red * factor).min(1.0),
        (linear.green * factor).min(1.0),
        (linear.blue * factor).min(1.0),
    )
}
