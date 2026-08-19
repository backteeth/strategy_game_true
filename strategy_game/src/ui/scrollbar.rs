//! パネル本文の可視縦スクロールバー(トラック+ドラッグ可能なつまみ)。
//!
//! `ui::scroll::handle_panel_scroll`によるマウスホイールスクロールだけでは、
//! (1) パネルがスクロール可能であること自体が視覚的にわからない、
//! (2) 内容がどれだけ・どの位置まであるかわからない、(3) ホイールを持たない/使いにくい
//! 環境でスクロールする手段がない、という問題が残る。本モジュールは各スクロール可能
//! パネルの右端に、内容量に応じて高さが変わるつまみを持つ縦スクロールバーを追加し、
//! ドラッグでもスクロールできるようにする。
//!
//! 実装は`bevy_ui_widgets`crateの`Scrollbar`/`ScrollbarThumb`(`bevy_picking`の
//! Pointerイベント/Observerベース)を使わず、このコードベース全体で既に一貫して
//! 使われている`Interaction`+`Button`+`ButtonInput<MouseButton>`ベースの操作パターン
//! (`map/division_selection.rs`のドラッグ選択等)に合わせて自前実装する
//! (`bevy_picking`のObserverパターンはこのコードベースに前例が無く、
//! 導入するとUI操作の実装方式が二重化するため)。
use bevy::input::mouse::MouseButton;
use bevy::prelude::*;

/// スクロールバートラックの幅(px)。
pub const TRACK_WIDTH_PX: f32 = 8.0;
/// つまみの最小高さ(px)。内容が非常に長い場合でもつまみが消えないようにする下限。
const MIN_THUMB_HEIGHT_PX: f32 = 24.0;

/// つまみEntity。`target`はスクロールされる対象(`Node`+`ComputedNode`+`ScrollPosition`を
/// 持つスクロール可能パネル自身)のEntity。
#[derive(Component)]
pub struct ScrollbarThumb {
    pub target: Entity,
}

#[derive(Component, Default)]
struct ScrollbarDragState {
    dragging: bool,
    drag_start_cursor_y: f32,
    drag_start_scroll_y: f32,
}

/// `target`のスクロール本体の兄弟として、トラック+つまみのペアを追加する
/// (`ui::scroll::spawn_scrollable_body`から呼び出される。トラック自身は通常のFlex子
/// として配置する — スクロール対象の**内側**に置くと、つまみ自身もスクロールされてしまい
/// 見かけ上「動かない」ように見える不具合が実機で確認されているため、必ず外側に置くこと)。
/// `target`はスクロールされる対象のEntity(`commands.spawn((...)).id()`で取得したもの)。
pub fn spawn_vertical_scrollbar(parent: &mut ChildSpawnerCommands, target: Entity) {
    parent
        .spawn(Node {
            width: Val::Px(TRACK_WIDTH_PX),
            ..default()
        })
        .with_children(|track| {
            track.spawn((
                ScrollbarThumb { target },
                ScrollbarDragState::default(),
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    width: Val::Px(TRACK_WIDTH_PX),
                    height: Val::Px(MIN_THUMB_HEIGHT_PX),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.35)),
                Visibility::Hidden,
            ));
        });
}

/// 対象パネルの`ComputedNode`(表示サイズ/内容サイズ)と`ScrollPosition`から、
/// つまみの高さ・位置を毎フレーム更新する。内容が表示領域に収まる場合はつまみを隠す
/// (=スクロール自体が不要なパネルではバーを出さない)。
fn update_scrollbar_thumbs(
    target_q: Query<(&ComputedNode, &ScrollPosition)>,
    mut thumb_q: Query<(&ScrollbarThumb, &mut Node, &mut Visibility)>,
) {
    for (thumb, mut node, mut visibility) in &mut thumb_q {
        let Ok((computed, scroll)) = target_q.get(thumb.target) else {
            continue;
        };
        let visible_h = computed.size().y * computed.inverse_scale_factor;
        let content_h = computed.content_size().y * computed.inverse_scale_factor;

        if visible_h <= 0.0 || content_h <= visible_h + 1.0 {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        }
        if *visibility != Visibility::Visible {
            *visibility = Visibility::Visible;
        }

        let (thumb_h, thumb_top) =
            thumb_geometry(visible_h, content_h, scroll.y, MIN_THUMB_HEIGHT_PX);
        node.height = Val::Px(thumb_h);
        node.top = Val::Px(thumb_top);
    }
}

/// つまみの高さを計算する(トラック長`visible_h`に対し、可視/内容比率で縮小する)。
fn thumb_height(visible_h: f32, content_h: f32, min_thumb_h: f32) -> f32 {
    (visible_h * visible_h / content_h.max(1.0))
        .max(min_thumb_h)
        .min(visible_h)
}

/// つまみの高さ・トラック内オフセットを計算する(視覚更新・ドラッグ計算で共有)。
fn thumb_geometry(visible_h: f32, content_h: f32, scroll_y: f32, min_thumb_h: f32) -> (f32, f32) {
    let thumb_h = thumb_height(visible_h, content_h, min_thumb_h);
    let max_scroll = (content_h - visible_h).max(0.0);
    let max_thumb_travel = (visible_h - thumb_h).max(0.0);
    let ratio = if max_scroll > 0.0 {
        (scroll_y / max_scroll).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (thumb_h, ratio * max_thumb_travel)
}

/// つまみが押された(`Interaction::Pressed`になった)瞬間、ドラッグ開始位置を記録する。
fn start_scrollbar_drag(
    windows: Query<&Window>,
    mut thumb_q: Query<
        (&Interaction, &ScrollbarThumb, &mut ScrollbarDragState),
        Changed<Interaction>,
    >,
    target_q: Query<&ScrollPosition>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };

    for (interaction, thumb, mut drag) in &mut thumb_q {
        if *interaction == Interaction::Pressed
            && let Ok(scroll) = target_q.get(thumb.target)
        {
            drag.dragging = true;
            drag.drag_start_cursor_y = cursor.y;
            drag.drag_start_scroll_y = scroll.y;
        }
    }
}

/// ドラッグ中は毎フレーム、カーソルの縦移動量をトラック内比率(`可視高さ/内容高さ`)で
/// スケールして`ScrollPosition.y`へ反映する。左ボタンが離されたらドラッグを終了する
/// (`Interaction`の変化ではなく実際のボタン押下状態で判定する: ドラッグ中にカーソルが
/// つまみの外へ出ると`Interaction`が`None`に変わってしまうため、それでは終了判定に使えない)。
fn apply_scrollbar_drag(
    windows: Query<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut thumb_q: Query<(&ScrollbarThumb, &mut ScrollbarDragState)>,
    mut target_q: Query<(&mut ScrollPosition, &ComputedNode)>,
) {
    let Ok(window) = windows.single() else {
        return;
    };

    for (thumb, mut drag) in &mut thumb_q {
        if !drag.dragging {
            continue;
        }
        if !mouse.pressed(MouseButton::Left) {
            drag.dragging = false;
            continue;
        }
        let Some(cursor) = window.cursor_position() else {
            continue;
        };
        let Ok((mut scroll, computed)) = target_q.get_mut(thumb.target) else {
            continue;
        };

        let visible_h = computed.size().y * computed.inverse_scale_factor;
        let content_h = computed.content_size().y * computed.inverse_scale_factor;
        let max_scroll = (content_h - visible_h).max(0.0);
        let thumb_h = thumb_height(visible_h, content_h, MIN_THUMB_HEIGHT_PX);
        let max_thumb_travel = (visible_h - thumb_h).max(0.0);

        if max_thumb_travel <= 0.0 || max_scroll <= 0.0 {
            continue;
        }

        let delta_cursor_y = cursor.y - drag.drag_start_cursor_y;
        let delta_scroll_y = delta_cursor_y * (max_scroll / max_thumb_travel);
        scroll.y = (drag.drag_start_scroll_y + delta_scroll_y).clamp(0.0, max_scroll);
    }
}

pub struct ScrollbarPlugin;

impl Plugin for ScrollbarPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                update_scrollbar_thumbs,
                start_scrollbar_drag,
                apply_scrollbar_drag,
            )
                .chain(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_geometry_fills_track_when_content_fits() {
        let (h, top) = thumb_geometry(500.0, 400.0, 0.0, 24.0);
        assert_eq!(h, 500.0);
        assert_eq!(top, 0.0);
    }

    #[test]
    fn thumb_geometry_shrinks_proportionally_to_visible_over_content_ratio() {
        // visible=500, content=1000 -> thumb should be half the track (250px)
        let (h, _top) = thumb_geometry(500.0, 1000.0, 0.0, 24.0);
        assert!((h - 250.0).abs() < 0.01, "expected ~250, got {h}");
    }

    #[test]
    fn thumb_geometry_respects_minimum_height_for_very_long_content() {
        let (h, _top) = thumb_geometry(500.0, 100_000.0, 0.0, 24.0);
        assert_eq!(h, 24.0);
    }

    #[test]
    fn thumb_geometry_moves_to_bottom_of_track_at_max_scroll() {
        let visible_h = 500.0;
        let content_h = 1000.0;
        let max_scroll = content_h - visible_h;
        let (thumb_h, top) = thumb_geometry(visible_h, content_h, max_scroll, 24.0);
        let expected_top = visible_h - thumb_h;
        assert!(
            (top - expected_top).abs() < 0.01,
            "expected top ~{expected_top}, got {top}"
        );
    }

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, update_scrollbar_thumbs);
        app
    }

    fn computed_node(visible: Vec2, content: Vec2) -> ComputedNode {
        ComputedNode {
            size: visible,
            content_size: content,
            inverse_scale_factor: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn thumb_is_hidden_when_content_fits_in_visible_area() {
        let mut app = build_test_app();
        let target = app
            .world_mut()
            .spawn((
                computed_node(Vec2::new(600.0, 500.0), Vec2::new(600.0, 400.0)),
                ScrollPosition::default(),
            ))
            .id();
        let thumb = app
            .world_mut()
            .spawn((
                ScrollbarThumb { target },
                Node::default(),
                Visibility::Visible,
            ))
            .id();

        app.update();

        assert_eq!(
            *app.world().get::<Visibility>(thumb).unwrap(),
            Visibility::Hidden
        );
    }

    #[test]
    fn thumb_is_visible_when_content_overflows() {
        let mut app = build_test_app();
        let target = app
            .world_mut()
            .spawn((
                computed_node(Vec2::new(600.0, 500.0), Vec2::new(600.0, 1200.0)),
                ScrollPosition::default(),
            ))
            .id();
        let thumb = app
            .world_mut()
            .spawn((
                ScrollbarThumb { target },
                Node::default(),
                Visibility::Hidden,
            ))
            .id();

        app.update();

        assert_eq!(
            *app.world().get::<Visibility>(thumb).unwrap(),
            Visibility::Visible
        );
        let node = app.world().get::<Node>(thumb).unwrap();
        assert_eq!(node.height, Val::Px(500.0 * 500.0 / 1200.0));
    }
}
