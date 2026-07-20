/// app モジュール
/// ゲーム状態・設定・データローダー・時間管理を統合する
pub mod game_state;
pub mod loader;
pub mod settings;
pub mod time;

use bevy::prelude::*;
use game_state::GameState;
use loader::DataLoaderPlugin;
use settings::CameraSettings;
use time::GameTimePlugin;

/// アプリケーション基盤プラグイン
pub struct AppPlugin;

impl Plugin for AppPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .insert_resource(CameraSettings::default())
            .add_plugins(DataLoaderPlugin)
            .add_plugins(GameTimePlugin);
    }
}
