//! パネル本文の共通縦スクロール。
//!
//! `research_panel.rs`/`politics_panel.rs`/`diplomacy_panel.rs`/`peace_panel.rs`/
//! `military_panel.rs`はいずれも固定高さ+`Overflow::clip_y()`のパネルを持ち、
//! 内容(国家間関係一覧・技術一覧等)が高さを超えると見切れた上にスクロールする手段が
//! 一切なく閲覧不能になる不具合があった(`military_panel.rs`のみP21-004で
//! `scroll_y()`化済みだったが、ホバー判定なしで`MouseWheel`を無条件消費していたため、
//! パネルが開いている間は画面のどこでホイールを回してもマップカメラのズームと同時に
//! 発生してしまう別の不具合を抱えていた)。
//!
//! **実機で再発した不具合1(2026-08-17)**: 初回修正では`Interaction`をホバー判定に
//! 使ったが、これはBevyの`bevy_ui::focus::ui_focus_system`のデフォルト
//! `FocusPolicy::Block`により、カーソルがパネル内の子Button(条約提案・請求選択等、
//! パネル内容の大部分を占める)の上にある間、子Buttonが`Interaction::Hovered`を
//! 「捕捉」してしまい、親であるパネル自身の`Interaction`は更新されない
//! (`hovered_nodes`を手前から辿り、`FocusPolicy::Block`のNodeに達した時点で
//! それより奥のNode — 親パネルを含む — への伝播を打ち切るため)。
//! そのためカーソルが子Buttonの隙間の余白部分にある時しかスクロールが反応せず、
//! 実質使い物にならなかった。対策として`Interaction`ではなく`RelativeCursorPosition`を
//! 使う。これは`ui_focus_system`内で`FocusPolicy`のBlock/Pass判定より前段の、
//! 全Node共通のループで(クリップ考慮込みで)無条件に更新されるため、子Buttonに
//! 「奪われる」ことがない。
//!
//! **実機で再発した不具合2(2026-08-17)**: 上記1の修正後も、パネル上でホイールすると
//! マップカメラも同時にズームしてしまう不具合が残っていた。原因は`MessageReader`が
//! パネル側とカメラ側でそれぞれ独立したカーソルを持つため、パネル側が読んでも
//! `map::camera::camera_zoom`側の`MessageReader`は同じイベントを独立に読み続けてしまう
//! ことにある(「読んだ側が消費してもう片方には残らない」という単純な早い者勝ちの
//! 仕組みではない)。対策として、カーソルがスクロール可能なUI本体の上にあるかどうかを
//! 共有Resource([`CursorOverScrollableUi`])として毎フレーム計算し、
//! `camera_zoom`側がこれを見て自分から処理をスキップするようにする
//! (UIがポインタ入力を消費している間はゲーム世界側に渡さない、という一般的な
//! パターンをこのプロジェクトで初めて導入する)。
//!
//! **実機で再発した不具合3(2026-08-17)**: スクロールバーのつまみを、スクロール対象の
//! パネル自身の直接の子として(`Overflow::scroll_y()`を持つNodeの内側に)配置していたため、
//! つまみ自身もスクロール対象コンテンツの一部として一緒にスクロールしてしまい、
//! 見かけ上「つまみが動かない」(トラックとつまみが同じ量だけ一緒にずれるため、
//! 相対位置が変わらないように見える)結果になっていた。対策として、
//! [`spawn_scrollable_body`]でスクロール本体を独立したラッパー行の中に置き、
//! スクロールバーはそのラッパー行の兄弟として(スクロール対象の外側に)配置する。
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use crate::ui::tab_bar::TabBarRoot;

/// パネル内容スクロールの共通上限(px)。パネルごとの正確な内容高さはレイアウト依存で
/// 計算が難しいため、緩めの固定上限に留める(`military_panel.rs`の既存方針を踏襲)。
pub const PANEL_SCROLL_MAX_PX: f32 = 4000.0;

/// カーソルが現在、スクロール可能なUI本体(パネルのスクロール領域・タブバー)の
/// 可視部分の上にあるかどうか。`map::camera::camera_zoom`がこれを見て、
/// UI上ではズームしないようにする。
#[derive(Resource, Default)]
pub struct CursorOverScrollableUi(pub bool);

/// 毎フレーム、`RelativeCursorPosition`を持つ全UI Node(スクロール可能パネル本体+
/// タブバー)を確認し、[`CursorOverScrollableUi`]を更新する。
/// `map::camera::camera_zoom`より前に実行される必要がある
/// (`CameraPlugin::build`側で`.before(camera_zoom)`制約を付与している)。
pub fn update_cursor_over_scrollable_ui(
    mut cursor_over: ResMut<CursorOverScrollableUi>,
    q: Query<&RelativeCursorPosition>,
) {
    cursor_over.0 = q.iter().any(RelativeCursorPosition::cursor_over);
}

/// カーソルが実際にNode上(クリップされていない可視部分)にある間のみ、
/// マウスホイールで`ScrollPosition.y`を更新する。子Buttonが`Interaction`を
/// 捕捉していても影響されない(`RelativeCursorPosition`はFocusPolicyのBlock/Passに
/// 関係なく全Node共通で更新されるため)。ホバーされていないNode・`TabBarRoot`は対象外。
pub fn handle_panel_scroll(
    mut scroll_events: MessageReader<MouseWheel>,
    mut q: Query<(&RelativeCursorPosition, &mut ScrollPosition), Without<TabBarRoot>>,
) {
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

    for (relative_cursor, mut scroll) in &mut q {
        if relative_cursor.cursor_over() {
            scroll.y = (scroll.y - delta_y).clamp(0.0, PANEL_SCROLL_MAX_PX);
        }
    }
}

/// パネルの「タイトル以外」をスクロール可能な内側ボディへラップし、
/// スクロールバー(`ui::scrollbar`)を添える共通ヘルパー。
///
/// タイトルをスクロール対象から外に置く(呼び出し元が別途spawnする)ことで、
/// タイトルは常に表示されたままになる。`spawn_children`でスクロール本体の
/// 実際の中身(header/list等)を追加する。戻り値はスクロール本体自身のEntity
/// (テストや追加のクエリで必要な場合に使う)。
///
/// スクロールバーはスクロール本体の兄弟として、スクロール対象の外側(Row内)に
/// 配置される。これがスクロール対象の**内側**にあると、つまみ自身もスクロールされて
/// しまい見かけ上「動かない」ように見える不具合が実機で確認されている(本ファイル冒頭の
/// 不具合3参照)。
pub fn spawn_scrollable_body(
    parent: &mut ChildSpawnerCommands,
    row_gap: Val,
    spawn_children: impl FnOnce(&mut ChildSpawnerCommands),
) -> Entity {
    let mut scroll_body_entity = Entity::PLACEHOLDER;
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            flex_grow: 1.0,
            // Flexboxの既定`min-height: auto`は「内容の最小サイズより縮められない」
            // という意味を持ち、これがネストしたflex_grow経由の高さ制約を上書きして
            // しまい、`Overflow::scroll_y()`によるクリップ/スクロールが実質的に
            // 無効化される(内容がどれだけ長くても箱自体が伸びてしまい、
            // `ComputedNode::content_size() == size()`になってしまう)という
            // 古典的なFlexboxの落とし穴が実機で確認されている。`min_height: 0`で解除する。
            min_height: Val::Px(0.0),
            column_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|body_row| {
            scroll_body_entity = body_row
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        flex_grow: 1.0,
                        min_height: Val::Px(0.0),
                        row_gap,
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    ScrollPosition::default(),
                    RelativeCursorPosition::default(),
                ))
                .with_children(spawn_children)
                .id();

            crate::ui::scrollbar::spawn_vertical_scrollbar(body_row, scroll_body_entity);
        });
    scroll_body_entity
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<MouseWheel>();
        app.add_systems(Update, handle_panel_scroll);
        app
    }

    fn send_wheel(app: &mut App, y: f32) {
        app.world_mut().write_message(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y,
            window: Entity::PLACEHOLDER,
            phase: bevy::input::touch::TouchPhase::Moved,
        });
        app.update();
    }

    fn hovered() -> RelativeCursorPosition {
        RelativeCursorPosition {
            cursor_over: true,
            normalized: Some(Vec2::ZERO),
        }
    }

    fn not_hovered() -> RelativeCursorPosition {
        RelativeCursorPosition {
            cursor_over: false,
            normalized: None,
        }
    }

    #[test]
    fn hovered_panel_scrolls() {
        let mut app = build_test_app();
        let entity = app
            .world_mut()
            .spawn((hovered(), ScrollPosition::default()))
            .id();

        send_wheel(&mut app, -5.0);

        let scroll = app.world().get::<ScrollPosition>(entity).unwrap();
        assert!(scroll.y > 0.0);
    }

    #[test]
    fn unhovered_panel_does_not_scroll() {
        let mut app = build_test_app();
        let entity = app
            .world_mut()
            .spawn((not_hovered(), ScrollPosition::default()))
            .id();

        send_wheel(&mut app, -5.0);

        let scroll = app.world().get::<ScrollPosition>(entity).unwrap();
        assert_eq!(scroll.y, 0.0);
    }

    /// バグ回帰テスト(2026-08-17): カーソルが子Button上にあり`Interaction`が
    /// そちらに奪われていても、`RelativeCursorPosition.cursor_over`は
    /// パネル自身について正しく`true`のままであることを確認する
    /// (`Interaction`ベースの実装ではここが`false`相当になり、スクロールが反応しなかった)。
    #[test]
    fn scroll_works_even_when_cursor_is_over_a_child_button() {
        let mut app = build_test_app();
        // 子Buttonが`Interaction::Hovered`を「捕捉」している状況を模擬する。
        // パネル自身は`Interaction`を持たず(=このSystemはInteractionを見ない)、
        // `RelativeCursorPosition.cursor_over`だけがカーソルの実位置を表す。
        let panel = app
            .world_mut()
            .spawn((hovered(), ScrollPosition::default()))
            .id();
        app.world_mut().spawn((
            Interaction::Hovered,
            RelativeCursorPosition {
                cursor_over: true,
                normalized: Some(Vec2::ZERO),
            },
        ));

        send_wheel(&mut app, -5.0);

        let scroll = app.world().get::<ScrollPosition>(panel).unwrap();
        assert!(
            scroll.y > 0.0,
            "panel must still scroll via RelativeCursorPosition even when a child button holds Interaction::Hovered"
        );
    }

    #[test]
    fn only_the_hovered_panel_scrolls_when_multiple_panels_exist() {
        let mut app = build_test_app();
        let hovered_entity = app
            .world_mut()
            .spawn((hovered(), ScrollPosition::default()))
            .id();
        let not_hovered_entity = app
            .world_mut()
            .spawn((not_hovered(), ScrollPosition::default()))
            .id();

        send_wheel(&mut app, -5.0);

        assert!(app.world().get::<ScrollPosition>(hovered_entity).unwrap().y > 0.0);
        assert_eq!(
            app.world()
                .get::<ScrollPosition>(not_hovered_entity)
                .unwrap()
                .y,
            0.0
        );
    }

    #[test]
    fn tab_bar_root_is_excluded_to_avoid_double_application() {
        let mut app = build_test_app();
        let entity = app
            .world_mut()
            .spawn((TabBarRoot, hovered(), ScrollPosition::default()))
            .id();

        send_wheel(&mut app, -5.0);

        let scroll = app.world().get::<ScrollPosition>(entity).unwrap();
        assert_eq!(
            scroll.y, 0.0,
            "TabBarRoot must be handled by its own dedicated scroll system, not this shared one"
        );
    }

    #[test]
    fn scroll_is_clamped_to_the_shared_upper_bound() {
        let mut app = build_test_app();
        let entity = app
            .world_mut()
            .spawn((hovered(), ScrollPosition::default()))
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
        assert_eq!(scroll.y, PANEL_SCROLL_MAX_PX);
    }

    #[test]
    fn cursor_over_scrollable_ui_is_true_when_any_tracked_node_is_hovered() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<CursorOverScrollableUi>();
        app.add_systems(Update, update_cursor_over_scrollable_ui);
        app.world_mut().spawn(not_hovered());
        app.world_mut().spawn(hovered());

        app.update();

        assert!(app.world().resource::<CursorOverScrollableUi>().0);
    }

    #[test]
    fn cursor_over_scrollable_ui_is_false_when_nothing_is_hovered() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<CursorOverScrollableUi>();
        app.add_systems(Update, update_cursor_over_scrollable_ui);
        app.world_mut().spawn(not_hovered());
        app.world_mut().spawn(not_hovered());

        app.update();

        assert!(!app.world().resource::<CursorOverScrollableUi>().0);
    }
}
