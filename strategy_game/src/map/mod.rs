/// map モジュール
/// カメラ、描画、州選択を統合するプラグインを提供する
pub mod camera;
pub mod rendering;
pub mod selection;

use bevy::prelude::*;
use camera::CameraPlugin;
use rendering::RenderingPlugin;
use selection::SelectionPlugin;

/// マップ関連の全プラグインをまとめるプラグイン
pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CameraPlugin)
            .add_plugins(RenderingPlugin)
            .add_plugins(SelectionPlugin);
    }
}
