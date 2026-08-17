use crate::app::game_state::GameState;
use crate::map::camera::GameCamera;
use crate::map::division_selection::DragSelectState;
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_state_click(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_q: Query<&Transform, With<GameCamera>>,
    state_visuals_q: Query<(&Transform, &StateVisual)>,
    ui_interactions_q: Query<&Interaction>,
    military_registry: Res<crate::military::data::MilitaryRegistry>,
    state_registry: Res<crate::state::data::StateRegistry>,
    drag_state: Res<DragSelectState>,
    mut selected: ResMut<SelectedState>,
    mut selection_changed: MessageWriter<StateSelectionChanged>,
    frontline_select_mode: Res<crate::map::frontline_selection::FrontlineSelectMode>,
) {
    // P21-005: 前線選択モード中は州クリックを一切発生させない
    // (`map::frontline_selection::handle_frontline_select_click`が同じクリックを処理する)。
    if frontline_select_mode.is_active() {
        return;
    }

    // 左クリックが離された瞬間のみ処理(押下瞬間ではない)。
    // P21-004: division_selectionの矩形選択は「押下→ドラッグ→解放」で確定するため、
    // 押下瞬間に反応すると、ドラッグ選択の開始点がたまたま州の上だった場合に
    // 意図せずその州を選択してしまう。解放時点でドラッグ中だったかを判定できる
    // よう、州クリックも解放イベントで統一する。
    if !mouse_buttons.just_released(MouseButton::Left) {
        return;
    }

    // 直前の操作がドラッグ選択だった場合は州選択を行わない
    if drag_state.is_dragging {
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

    // 陸軍ユニットをクリックした場合は州選択をスキップ
    let division_radius = 16.0;
    for division in military_registry.divisions.values() {
        let mut pos = state_registry
            .get(division.current_state)
            .map(|s| s.position())
            .unwrap_or(Vec2::ZERO);

        if let Some(target_state) = division.target_state {
            let target_pos = state_registry
                .get(target_state)
                .map(|s| s.position())
                .unwrap_or(Vec2::ZERO);
            pos = pos.lerp(target_pos, division.movement_progress);
        }

        if world_pos.distance(pos) < division_radius {
            return;
        }
    }

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

pub(crate) fn brighten_color(color: Color, factor: f32) -> Color {
    let linear = color.to_linear();
    Color::linear_rgb(
        (linear.red * factor).min(1.0),
        (linear.green * factor).min(1.0),
        (linear.blue * factor).min(1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::military::data::MilitaryRegistry;
    use crate::state::data::StateRegistry;

    fn press_left(app: &mut App) {
        let mut mouse = ButtonInput::<MouseButton>::default();
        mouse.press(MouseButton::Left);
        app.insert_resource(mouse);
        app.update();
    }

    fn release_left(app: &mut App) {
        {
            let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            mouse.clear();
            mouse.release(MouseButton::Left);
        }
        app.update();
    }

    /// P21-004 spec項目17: UIボタン(編成一覧の行など)をクリックしている間、
    /// 同時に走る州クリック処理(`handle_state_click`)が誤って発火しないことを検証する。
    /// `ArmyListRowButton`固有のロジックではなく、`Interaction::Pressed`を持つ
    /// あらゆるUIノードを対象にした既存の汎用ガードが、新しく追加した動的な
    /// 編成一覧の行にもそのまま適用されることの回帰テスト。
    #[test]
    fn ui_button_press_blocks_map_state_click_from_firing() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<StateSelectionChanged>();
        app.add_systems(Update, handle_state_click);

        app.insert_resource(MilitaryRegistry::default());
        app.insert_resource(StateRegistry::build(vec![]));
        app.insert_resource(DragSelectState::default());
        app.insert_resource(SelectedState(None));
        app.insert_resource(crate::map::frontline_selection::FrontlineSelectMode::default());

        // 何らかのUIボタン(編成一覧の行など)がクリックされている状態を再現する。
        // マーカーコンポーネントの種類は問わない(`handle_state_click`は
        // `Interaction`を持つノードを型を問わず走査するため)。
        app.world_mut().spawn(Interaction::Pressed);

        press_left(&mut app);
        release_left(&mut app);

        assert_eq!(
            app.world().resource::<SelectedState>().0,
            None,
            "UIボタン押下中はマップの州クリックが発火してはならない"
        );
    }

    /// P21-005要求テスト項目34: 前線選択モード中は州クリックが一切発火しない
    /// (`FrontlineSelectMode`がアクティブなら`handle_state_click`は即座にreturnする)。
    /// 実際にクリック座標がStateVisualの矩形へ命中する状況を再現した上で、モードが
    /// 非アクティブなら選択される・アクティブなら選択されない、の両方を確認する。
    #[test]
    fn frontline_select_mode_blocks_map_state_click_from_firing() {
        fn build_app() -> App {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins);
            app.add_message::<StateSelectionChanged>();
            app.add_systems(Update, handle_state_click);

            app.insert_resource(MilitaryRegistry::default());
            app.insert_resource(StateRegistry::build(vec![]));
            app.insert_resource(DragSelectState::default());
            app.insert_resource(SelectedState(None));

            let mut window = Window {
                resolution: bevy::window::WindowResolution::new(800, 600),
                ..Default::default()
            };
            window.set_cursor_position(Some(Vec2::new(400.0, 300.0)));
            app.world_mut().spawn(window);
            app.world_mut()
                .spawn((GameCamera, Transform::from_xyz(0.0, 0.0, 0.0)));
            app.world_mut().spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                StateVisual {
                    state_id: crate::common::StateId(1),
                    size: Vec2::new(100.0, 100.0),
                    base_color: Color::WHITE,
                },
            ));
            app
        }

        // モード非アクティブなら通常通り選択される(前提の妥当性確認)。
        let mut app_without_mode = build_app();
        app_without_mode
            .insert_resource(crate::map::frontline_selection::FrontlineSelectMode::default());
        press_left(&mut app_without_mode);
        release_left(&mut app_without_mode);
        assert_eq!(
            app_without_mode.world().resource::<SelectedState>().0,
            Some(crate::common::StateId(1)),
            "前提確認: 前線選択モード非アクティブなら通常通り州クリックが発火するはず"
        );

        // モードアクティブなら同じクリックでも選択されない。
        let mut app_with_mode = build_app();
        let mut mode = crate::map::frontline_selection::FrontlineSelectMode::default();
        mode.activate(crate::common::ArmyId(0));
        app_with_mode.insert_resource(mode);
        press_left(&mut app_with_mode);
        release_left(&mut app_with_mode);
        assert_eq!(
            app_with_mode.world().resource::<SelectedState>().0,
            None,
            "前線選択モード中は同じクリックでも通常の州クリックが発火してはならない"
        );
    }
}
