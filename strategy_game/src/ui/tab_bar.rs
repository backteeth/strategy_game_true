//! パネル切替タブの共通コンテナ。
//!
//! 従来は研究/政治/外交/講和/軍事の5トグルボタンがそれぞれ独立したハードコード絶対座標
//! (`left: 310/440/570/710/840px`)で配置されており、ボタン自体の幅(テキスト依存)が
//! 間隔(130px刻み)を上回るため隣接ボタン同士が重なり合い、さらに右端(840px)は
//! 右側`StatePanelRoot`(940pxから開始)の直前まで達していた。これが「タブが見切れる」
//! 不具合の実体であり、いずれのコンテナにも`Overflow`によるクリップは設定されていない
//! (重なり・衝突であって、クリップされていたわけではない)。
//!
//! 本モジュールは実タブボタンを子として受け取る共通コンテナ`TabBarRoot`を提供する。
//! `flex_wrap: FlexWrap::Wrap`で1行に収まらない分を自動的に2行目へ折り返し、
//! コンテナ自体は1行分の高さに固定した上で`overflow: Overflow::scroll_y()`を設定し、
//! 折り返された行はマウスホイールで下にスクロールして閲覧できるようにする
//! (`military_panel.rs`の`ScrollPosition`スクロール実装と同一パターン)。
//!
//! `military_panel.rs`の既存スクロール実装は`MouseWheel`を無条件に読むため、
//! パネルが開いている間はカーソル位置に関わらず`map::camera::camera_zoom`と
//! 同時にホイールイベントを消費してしまう(パネルスクロールとカメラズームが同時に
//! 発生する)既知の未解決事象がある。本タブバーはその反省を踏まえ、コンテナ自身に
//! `RelativeCursorPosition`を持たせてホバー中のみホイールを消費するようゲーティングする。
//! `Interaction`ではなく`RelativeCursorPosition`を使う理由は`ui::scroll`モジュールの
//! ドキュメントコメント参照(タブボタン自身がBevyのデフォルト`FocusPolicy::Block`で
//! `Interaction`を「捕捉」してしまい、親のTabBarRootには伝播しないため)。
use crate::app::game_state::GameState;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

/// タブボタンをまとめる共通コンテナ。各パネルのトグルボタンはこのEntityの子として
/// spawnされる(`Query<Entity, With<TabBarRoot>>`で取得し、`.with_children(...)`で追加)。
#[derive(Component)]
pub struct TabBarRoot;

/// タブバーの縦スクロール上限(px)。2行目までしか使わない想定のゆとりある固定値
/// (`military_panel.rs`の`handle_military_panel_scroll`と同じ「正確な最大値計算は
/// レイアウト依存で難しいため緩めの固定上限に留める」という考え方を踏襲)。
const TAB_BAR_SCROLL_MAX_PX: f32 = 200.0;

/// タブバー本体の`left`起点(px)。左側`EconomyPanelRoot`(幅300px)の直後。
pub const TAB_BAR_LEFT_PX: f32 = 310.0;
/// タブバー本体の`top`(px)。既存の各トグルボタンと同じ位置。
pub const TAB_BAR_TOP_PX: f32 = 45.0;
/// タブバー本体の幅(px)。右側`StatePanelRoot`(1280px幅ウィンドウで`left`960px)の
/// 手前で止め、衝突を避ける。
pub const TAB_BAR_WIDTH_PX: f32 = 630.0;
/// タブバー本体の表示高さ(px)。1行分。折り返した2行目以降はスクロールで見る。
pub const TAB_BAR_HEIGHT_PX: f32 = 40.0;

pub struct TabBarPlugin;

impl Plugin for TabBarPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), spawn_tab_bar)
            .add_systems(
                Update,
                handle_tab_bar_scroll.run_if(in_state(GameState::Playing)),
            )
            .add_systems(OnExit(GameState::Playing), cleanup_tab_bar);
    }
}

pub fn spawn_tab_bar(mut commands: Commands) {
    commands.spawn((
        TabBarRoot,
        // P21-013: 背景自体もButton化し、タブボタンの隙間をクリックしてもマップ操作へ
        // 貫通しないようにする(`ui::load_confirm`の既存パターン)。
        Button,
        RelativeCursorPosition::default(),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(TAB_BAR_LEFT_PX),
            top: Val::Px(TAB_BAR_TOP_PX),
            width: Val::Px(TAB_BAR_WIDTH_PX),
            height: Val::Px(TAB_BAR_HEIGHT_PX),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_content: AlignContent::FlexStart,
            column_gap: Val::Px(6.0),
            row_gap: Val::Px(4.0),
            overflow: Overflow::scroll_y(),
            ..default()
        },
        ScrollPosition::default(),
    ));
}

/// タブバーがホバーされている間のみ、マウスホイールで折り返し2行目以降をスクロールする。
fn handle_tab_bar_scroll(
    mut scroll_events: MessageReader<MouseWheel>,
    mut bar_q: Query<(&RelativeCursorPosition, &mut ScrollPosition), With<TabBarRoot>>,
) {
    let Ok((relative_cursor, mut scroll)) = bar_q.single_mut() else {
        scroll_events.clear();
        return;
    };

    if !relative_cursor.cursor_over() {
        scroll_events.clear();
        return;
    }

    let mut delta_y = 0.0_f32;
    for event in scroll_events.read() {
        delta_y += match event.unit {
            MouseScrollUnit::Line => event.y * 24.0,
            MouseScrollUnit::Pixel => event.y,
        };
    }
    if delta_y == 0.0 {
        return;
    }

    scroll.y = (scroll.y - delta_y).clamp(0.0, TAB_BAR_SCROLL_MAX_PX);
}

fn cleanup_tab_bar(mut commands: Commands, query: Query<Entity, With<TabBarRoot>>) {
    if let Ok(entity) = query.single() {
        commands.entity(entity).despawn();
    }
}

/// タブ切替のショートカットが`Ctrl`と併用されているかを判定する共通ヘルパー。
/// 軍事パネル内のフロントライン操作(素の`Digit1/2/3/7/8/9`, `military_panel.rs`)との
/// キー衝突を避けるため、タブ切替は全て`Ctrl+数字`に統一する。
pub fn ctrl_held(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<MouseWheel>();
        app
    }

    #[test]
    fn spawn_tab_bar_creates_exactly_one_wrapping_scrollable_root() {
        let mut app = build_test_app();
        app.add_systems(Startup, spawn_tab_bar);
        app.update();

        let mut query = app
            .world_mut()
            .query::<(&TabBarRoot, &Node, &ScrollPosition)>();
        let results: Vec<_> = query.iter(app.world()).collect();
        assert_eq!(results.len(), 1, "exactly one TabBarRoot must be spawned");

        let (_, node, _) = results[0];
        assert_eq!(node.flex_wrap, FlexWrap::Wrap);
        assert_eq!(node.overflow, Overflow::scroll_y());
    }

    /// P21-013: `TabBarRoot`自身が`Button`であることを確認する回帰テスト。これにより
    /// タブボタンの隙間をクリック/ホバーしても`Interaction`が確実に発行され、
    /// `map::selection::handle_state_click`等の既存「UIのHovered/Pressed中はマップ操作を
    /// スキップする」ガードがこの領域にも効くようになる(`ui::load_confirm`の既存パターンと
    /// 同じ)。
    #[test]
    fn tab_bar_root_background_is_itself_a_button() {
        let mut app = build_test_app();
        app.add_systems(Startup, spawn_tab_bar);
        app.update();

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<TabBarRoot>>()
            .single(app.world())
            .expect("TabBarRoot must be spawned");
        assert!(
            app.world().entity(root).contains::<Button>(),
            "TabBarRoot's own background must be a Button so hovering it registers Interaction"
        );
    }

    #[test]
    fn scroll_is_ignored_while_tab_bar_is_not_hovered() {
        let mut app = build_test_app();
        app.add_systems(Update, handle_tab_bar_scroll);
        let entity = app
            .world_mut()
            .spawn((
                TabBarRoot,
                RelativeCursorPosition::default(),
                ScrollPosition::default(),
            ))
            .id();

        app.world_mut().write_message(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: 5.0,
            window: Entity::PLACEHOLDER,
            phase: bevy::input::touch::TouchPhase::Moved,
        });
        app.update();

        let scroll = app.world().get::<ScrollPosition>(entity).unwrap();
        assert_eq!(
            scroll.y, 0.0,
            "wheel input must be ignored while the tab bar is not hovered \
             (this is the fix for the ungated-scroll/camera-zoom collision \
             known from military_panel.rs)"
        );
    }

    #[test]
    fn scroll_moves_when_tab_bar_is_hovered() {
        let mut app = build_test_app();
        app.add_systems(Update, handle_tab_bar_scroll);
        let entity = app
            .world_mut()
            .spawn((
                TabBarRoot,
                RelativeCursorPosition {
                    cursor_over: true,
                    normalized: Some(Vec2::ZERO),
                },
                ScrollPosition::default(),
            ))
            .id();

        // 負のyは「下スクロール」ジェスチャ(`military_panel.rs`の`handle_military_panel_scroll`
        // と同じ`scroll.y -= delta_y`の符号規約): 折り返された2行目を下方向に見せるには
        // scroll.yを増加させる必要があるため、ここではy<0を送る。
        app.world_mut().write_message(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: -5.0,
            window: Entity::PLACEHOLDER,
            phase: bevy::input::touch::TouchPhase::Moved,
        });
        app.update();

        let scroll = app.world().get::<ScrollPosition>(entity).unwrap();
        assert!(
            scroll.y > 0.0,
            "wheel input must scroll the tab bar while it is hovered"
        );
    }

    #[test]
    fn scroll_is_clamped_to_the_fixed_upper_bound() {
        let mut app = build_test_app();
        app.add_systems(Update, handle_tab_bar_scroll);
        let entity = app
            .world_mut()
            .spawn((
                TabBarRoot,
                RelativeCursorPosition {
                    cursor_over: true,
                    normalized: Some(Vec2::ZERO),
                },
                ScrollPosition::default(),
            ))
            .id();

        for _ in 0..50 {
            app.world_mut().write_message(MouseWheel {
                unit: MouseScrollUnit::Pixel,
                x: 0.0,
                y: -1000.0,
                window: Entity::PLACEHOLDER,
                phase: bevy::input::touch::TouchPhase::Moved,
            });
            app.update();
        }

        let scroll = app.world().get::<ScrollPosition>(entity).unwrap();
        assert_eq!(scroll.y, TAB_BAR_SCROLL_MAX_PX);
    }

    #[test]
    fn ctrl_held_detects_either_left_or_right_control() {
        let mut left = ButtonInput::<KeyCode>::default();
        left.press(KeyCode::ControlLeft);
        assert!(ctrl_held(&left));

        let mut right = ButtonInput::<KeyCode>::default();
        right.press(KeyCode::ControlRight);
        assert!(ctrl_held(&right));

        let none = ButtonInput::<KeyCode>::default();
        assert!(!ctrl_held(&none));
    }
}
