use crate::app::game_state::GameState;
use crate::map::camera::GameCamera;
use crate::map::rendering::StateVisual;
use crate::state::{SelectedState, StateSelectionChanged};
/// 州選択モジュール
/// マウスクリックによる州選択・選択解除と視覚フィードバックを実装する
/// Bevy 0.19: EventWriter → MessageWriter, EventReader → MessageReader
use bevy::prelude::*;

/// 選択プラグイン（MapPlugin から登録）
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
/// 画面座標をワールド座標に変換し、AABB判定でヒットを検出する
/// Bevy 0.19: EventWriter → MessageWriter
fn handle_state_click(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_q: Query<&Transform, With<GameCamera>>,
    state_visuals_q: Query<(&Transform, &StateVisual)>,
    mut selected: ResMut<SelectedState>,
    mut selection_changed: MessageWriter<StateSelectionChanged>,
) {
    // 左クリックが押された瞬間のみ処理
    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(cam_transform) = camera_q.single() else {
        return;
    };

    // カーソル位置（ウィンドウ外なら処理しない）
    let Some(cursor_screen) = window.cursor_position() else {
        return;
    };

    // 画面座標 → ワールド座標変換
    // スクリーン座標: 左上原点、Y軸下向き
    // ワールド座標: 中央原点、Y軸上向き
    let window_size = Vec2::new(window.width(), window.height());
    let cursor_ndc = (cursor_screen / window_size) * 2.0 - Vec2::ONE;
    // Y軸を反転（スクリーンは下が正、ワールドは上が正）
    let cursor_ndc = Vec2::new(cursor_ndc.x, -cursor_ndc.y);

    // カメラのスケール（ズーム）を考慮してワールド座標へ変換
    let scale = cam_transform.scale.x;
    let half_size = window_size * 0.5 * scale;
    let world_pos = cam_transform.translation.xy() + cursor_ndc * half_size;

    // AABB ヒット判定（最もZ値が大きい＝手前の州を選択）
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

    // 選択状態が変わったときだけメッセージ発行
    if selected.0 != hit_state {
        selected.0 = hit_state;
        selection_changed.write(StateSelectionChanged(hit_state));
    }
}

/// StateSelectionChanged メッセージを受け取ったときだけ州の色を更新する
/// 毎フレーム全スプライトを更新する無駄を避けるためメッセージ駆動にしている
/// Bevy 0.19: EventReader → MessageReader
fn update_state_visuals(
    mut selection_changed: MessageReader<StateSelectionChanged>,
    selected: Res<SelectedState>,
    mut state_visuals_q: Query<(&mut Sprite, &StateVisual)>,
) {
    // メッセージがなければ何もしない
    if selection_changed.is_empty() {
        return;
    }
    selection_changed.clear();

    for (mut sprite, visual) in state_visuals_q.iter_mut() {
        if Some(visual.state_id) == selected.0 {
            // 選択中: 明度を上げて強調表示
            sprite.color = brighten_color(visual.base_color, 1.5);
        } else {
            // 非選択: 基本色に戻す
            sprite.color = visual.base_color;
        }
    }
}

/// 色の明度を倍率で調整するヘルパー
fn brighten_color(color: Color, factor: f32) -> Color {
    let linear = color.to_linear();
    Color::linear_rgb(
        (linear.red * factor).min(1.0),
        (linear.green * factor).min(1.0),
        (linear.blue * factor).min(1.0),
    )
}
