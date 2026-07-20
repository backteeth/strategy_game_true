/// 2Dグランドストラテジーゲーム "Strategy" - エントリポイント
/// Rust + Bevy 0.19 で実装された魔法と科学が共存するファンタジー世界を舞台とする戦略ゲーム
mod app;
mod common;
mod country;
mod debug;
mod map;
mod state;
mod ui;

use app::AppPlugin;
use bevy::prelude::*;
use country::CountryPlugin;
use debug::DebugPlugin;
use map::MapPlugin;
use state::StatePlugin;
use ui::UiPlugin;

fn main() {
    App::new()
        // Bevy のデフォルトプラグイン（ウィンドウ、レンダリング、入力など）
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Grand Strategy - Prototype".to_string(),
                resolution: (1280u32, 720u32).into(),
                ..default()
            }),
            ..default()
        }))
        // ゲーム独自プラグイン（依存関係の順に登録）
        .add_plugins(AppPlugin) // ゲーム状態・設定
        .add_plugins(CountryPlugin) // 国家データ
        .add_plugins(StatePlugin) // 州データ・選択状態
        .add_plugins(MapPlugin) // カメラ・描画・州選択
        .add_plugins(UiPlugin) // ゲームUI
        .add_plugins(DebugPlugin) // デバッグ情報
        .run();
}
