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

/// P21-SAVE-002E: ロード成功後にカメラを初期状態へ戻す要求。`GameState::Playing`中に
/// 発行される一回性Message(`save::runtime::handle_load_requests`がロード成功時に発行する)。
#[derive(Message, Debug, Clone, Copy)]
pub struct CameraResetRequestMessage;

/// カメラ操作プラグイン（MapPlugin から登録）
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CameraDragState::default())
            .add_message::<CameraResetRequestMessage>()
            .add_systems(Startup, setup_camera)
            .add_systems(
                Update,
                (
                    camera_keyboard_move,
                    camera_zoom,
                    camera_drag,
                    reset_camera_on_request,
                ),
            );
    }
}

/// ゲームカメラを識別するマーカーコンポーネント
#[derive(Component)]
pub struct GameCamera;

/// カメラの初期Transform(位置・ズーム)。`setup_camera`(起動時の初期spawn)と
/// `reset_camera_on_request`(P21-SAVE-002E: ロード成功後のリセット)の両方が
/// この1つの定義だけを参照する(値を別ファイルへコピーして二重管理しない)。
/// このコードベースのズームは`OrthographicProjection`ではなく`Transform.scale`で
/// 表現されている(`camera_zoom`参照)ため、`Transform::IDENTITY`で位置・ズームの
/// 両方が表現できる。
pub(crate) fn default_camera_transform() -> Transform {
    Transform::IDENTITY
}

/// カメラを初期化する
fn setup_camera(mut commands: Commands) {
    commands.spawn((Camera2d, GameCamera, default_camera_transform()));
}

/// P21-SAVE-002E: ロード成功後、既存の`GameCamera` Entityを初期位置・ズームへ戻す。
/// 新規spawn・despawnは行わない(既存Entityの`Transform`を書き換えるだけ)。
/// `CameraDragState`も初期状態に戻す(通常は`save::apply`が既にロード適用の一部として
/// リセット済みだが、このシステム単体でも自己完結して安全に動作するよう念のため揃える)。
/// 同一フレームに複数の要求が来ても1回のリセットへ集約する。`GameCamera`が存在しない・
/// または複数存在する場合はpanicせず、診断可能な警告を出して視覚的なリセットだけを
/// 諦める(ロードされたゲーム正規データ自体には一切影響しない)。
fn reset_camera_on_request(
    mut requests: MessageReader<CameraResetRequestMessage>,
    mut camera_q: Query<&mut Transform, With<GameCamera>>,
    mut drag_state: ResMut<CameraDragState>,
) {
    let request_count = requests.read().count();
    if request_count == 0 {
        return;
    }

    match camera_q.single_mut() {
        Ok(mut transform) => {
            *transform = default_camera_transform();
        }
        Err(e) => {
            warn!(
                "[Camera] reset requested but GameCamera entity query failed ({e:?}); skipping visual camera reset (loaded game data is unaffected)"
            );
        }
    }

    drag_state.drag_start = None;
    drag_state.camera_start = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::ScheduleRunnerPlugin;
    use bevy::ecs::system::SystemState;

    /// `reset_camera_on_request`だけを検証する最小App。`CameraPlugin`全体
    /// (`Startup`のカメラspawn・キーボード/ホイール/ドラッグ操作)は意図的に含めず、
    /// テストごとに`GameCamera` Entityの有無・個数を明示的に制御する。
    fn setup_minimal_camera_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()));
        app.insert_resource(CameraDragState::default())
            .add_message::<CameraResetRequestMessage>()
            .add_systems(Update, reset_camera_on_request);
        app
    }

    fn send_camera_reset_request(app: &mut App) {
        let mut state: SystemState<MessageWriter<CameraResetRequestMessage>> =
            SystemState::new(app.world_mut());
        state
            .get_mut(app.world_mut())
            .expect("MessageWriter<CameraResetRequestMessage> must be valid")
            .write(CameraResetRequestMessage);
        state.apply(app.world_mut());
    }

    #[test]
    fn reset_request_restores_position_and_zoom_to_default() {
        let mut app = setup_minimal_camera_app();
        let entity = app
            .world_mut()
            .spawn((
                GameCamera,
                Transform::from_xyz(500.0, -300.0, 0.0).with_scale(Vec3::splat(2.5)),
            ))
            .id();

        send_camera_reset_request(&mut app);
        app.update();

        let transform = app.world().get::<Transform>(entity).unwrap();
        assert_eq!(transform.translation, Vec3::ZERO);
        assert_eq!(transform.scale, Vec3::ONE);
    }

    #[test]
    fn reset_request_restores_camera_drag_state() {
        let mut app = setup_minimal_camera_app();
        app.world_mut().spawn((GameCamera, Transform::IDENTITY));
        app.world_mut().resource_mut::<CameraDragState>().drag_start = Some(Vec2::new(1.0, 2.0));
        app.world_mut()
            .resource_mut::<CameraDragState>()
            .camera_start = Some(Vec2::new(3.0, 4.0));

        send_camera_reset_request(&mut app);
        app.update();

        let drag_state = app.world().resource::<CameraDragState>();
        assert_eq!(drag_state.drag_start, None);
        assert_eq!(drag_state.camera_start, None);
    }

    #[test]
    fn no_reset_request_leaves_camera_unchanged() {
        let mut app = setup_minimal_camera_app();
        let entity = app
            .world_mut()
            .spawn((
                GameCamera,
                Transform::from_xyz(42.0, 7.0, 0.0).with_scale(Vec3::splat(1.8)),
            ))
            .id();

        app.update();

        let transform = app.world().get::<Transform>(entity).unwrap();
        assert_eq!(transform.translation, Vec3::new(42.0, 7.0, 0.0));
        assert_eq!(transform.scale, Vec3::splat(1.8));
    }

    #[test]
    fn multiple_reset_requests_in_one_frame_still_reset_once_and_do_not_duplicate_camera() {
        let mut app = setup_minimal_camera_app();
        app.world_mut().spawn((
            GameCamera,
            Transform::from_xyz(10.0, 10.0, 0.0).with_scale(Vec3::splat(3.0)),
        ));

        send_camera_reset_request(&mut app);
        send_camera_reset_request(&mut app);
        send_camera_reset_request(&mut app);
        app.update();

        let mut query = app.world_mut().query_filtered::<Entity, With<GameCamera>>();
        let count = query.iter(app.world()).count();
        assert_eq!(
            count, 1,
            "reset must never spawn or despawn GameCamera entities"
        );
    }

    #[test]
    fn missing_camera_entity_does_not_panic() {
        let mut app = setup_minimal_camera_app();
        // GameCameraを1体もspawnしない。

        send_camera_reset_request(&mut app);
        app.update();

        // ここまで到達したこと自体が、パニックしなかった証拠。
    }

    #[test]
    fn multiple_camera_entities_does_not_panic() {
        let mut app = setup_minimal_camera_app();
        app.world_mut().spawn((GameCamera, Transform::IDENTITY));
        app.world_mut().spawn((GameCamera, Transform::IDENTITY));

        send_camera_reset_request(&mut app);
        app.update();

        // ここまで到達したこと自体が、パニックしなかった証拠
        // (`Query::single_mut`が複数一致でErrを返し、警告ログのみで処理を継続する)。
        let mut query = app.world_mut().query_filtered::<Entity, With<GameCamera>>();
        assert_eq!(
            query.iter(app.world()).count(),
            2,
            "no entity must be spawned or despawned"
        );
    }
}
