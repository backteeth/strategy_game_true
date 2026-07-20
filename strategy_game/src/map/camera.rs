use crate::app::settings::CameraSettings;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
/// カメラ操作モジュール
/// WASD/矢印キー移動、マウスホイールズーム、ドラッグ移動を実装する
/// Bevy 0.19 API: EventReader → MessageReader, get_single → single
use bevy::prelude::*;

/// カメラドラッグ状態を追跡するリソース
#[derive(Resource, Default)]
pub struct CameraDragState {
    /// ドラッグ開始時のカーソル位置
    pub drag_start: Option<Vec2>,
    /// ドラッグ開始時のカメラ位置
    pub camera_start: Option<Vec2>,
}

/// カメラ操作プラグイン（MapPlugin から登録）
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CameraDragState::default())
            .add_systems(Startup, setup_camera)
            .add_systems(Update, (camera_keyboard_move, camera_zoom, camera_drag));
    }
}

/// ゲームカメラを識別するマーカーコンポーネント
#[derive(Component)]
pub struct GameCamera;

/// カメラを初期化する
fn setup_camera(mut commands: Commands) {
    commands.spawn((Camera2d, GameCamera));
}

/// キーボード（WASD / 矢印キー）でカメラを移動する
fn camera_keyboard_move(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<CameraSettings>,
    mut camera_q: Query<&mut Transform, With<GameCamera>>,
) {
    let Ok(mut transform) = camera_q.single_mut() else {
        return;
    };

    let mut direction = Vec2::ZERO;

    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        direction.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        direction.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        direction.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        direction.x += 1.0;
    }

    if direction != Vec2::ZERO {
        let delta =
            direction.normalize() * settings.move_speed * time.delta_secs() * transform.scale.x; // ズームに応じて移動速度を調整

        let new_pos = transform.translation.xy() + delta;
        transform.translation.x = new_pos.x.clamp(-settings.map_bound_x, settings.map_bound_x);
        transform.translation.y = new_pos.y.clamp(-settings.map_bound_y, settings.map_bound_y);
    }
}

/// マウスホイールでカメラをズームする
/// Bevy 0.19: EventReader → MessageReader
fn camera_zoom(
    mut scroll_events: MessageReader<MouseWheel>,
    settings: Res<CameraSettings>,
    mut camera_q: Query<&mut Transform, With<GameCamera>>,
) {
    let Ok(mut transform) = camera_q.single_mut() else {
        return;
    };

    let mut scroll_total = 0.0_f32;
    for event in scroll_events.read() {
        let amount = match event.unit {
            MouseScrollUnit::Line => event.y,
            MouseScrollUnit::Pixel => event.y / 50.0,
        };
        scroll_total += amount;
    }

    if scroll_total != 0.0 {
        let zoom_factor = 1.0 - scroll_total * settings.zoom_speed;
        let new_scale =
            (transform.scale.x * zoom_factor).clamp(settings.min_scale, settings.max_scale);
        transform.scale = Vec3::splat(new_scale);
    }
}

/// 右ドラッグまたは中ドラッグでカメラを移動する
/// Bevy 0.19: EventReader → MessageReader
fn camera_drag(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut cursor_moved: MessageReader<CursorMoved>,
    mut drag_state: ResMut<CameraDragState>,
    settings: Res<CameraSettings>,
    mut camera_q: Query<&mut Transform, With<GameCamera>>,
) {
    let Ok(mut transform) = camera_q.single_mut() else {
        return;
    };

    let is_dragging =
        mouse_buttons.pressed(MouseButton::Right) || mouse_buttons.pressed(MouseButton::Middle);

    // ドラッグ開始
    if mouse_buttons.just_pressed(MouseButton::Right)
        || mouse_buttons.just_pressed(MouseButton::Middle)
    {
        drag_state.camera_start = Some(transform.translation.xy());
        drag_state.drag_start = None;
    }

    // ドラッグ中: カーソル移動イベントを処理
    if is_dragging {
        for event in cursor_moved.read() {
            let current_pos = event.position;

            if let Some(start) = drag_state.drag_start {
                // カーソル移動量に応じてカメラを動かす（逆方向）
                let delta = (current_pos - start) * settings.drag_sensitivity * transform.scale.x;
                if let Some(cam_start) = drag_state.camera_start {
                    let new_pos = cam_start + Vec2::new(-delta.x, delta.y);
                    transform.translation.x =
                        new_pos.x.clamp(-settings.map_bound_x, settings.map_bound_x);
                    transform.translation.y =
                        new_pos.y.clamp(-settings.map_bound_y, settings.map_bound_y);
                }
            } else {
                // 最初のカーソル位置を記録
                drag_state.drag_start = Some(current_pos);
                drag_state.camera_start = Some(transform.translation.xy());
            }
        }
    } else {
        // ドラッグ終了
        drag_state.drag_start = None;
        drag_state.camera_start = None;
    }
}
