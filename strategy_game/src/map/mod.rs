/// map モジュール
/// カメラ、描画、州選択を統合するプラグインを提供する
pub mod army_render;
pub mod army_selection;
pub mod camera;
pub mod frontline_render;
pub mod rendering;
pub mod selection;

use army_render::ArmyRenderPlugin;
use army_selection::ArmySelectionPlugin;
use bevy::prelude::*;
use camera::CameraPlugin;
use frontline_render::FrontlineRenderPlugin;
use rendering::RenderingPlugin;
use selection::SelectionPlugin;

/// マップ関連の全プラグインをまとめるプラグイン
pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CameraPlugin)
            .add_plugins(RenderingPlugin)
            .add_plugins(SelectionPlugin)
            .add_plugins(ArmyRenderPlugin)
            .add_plugins(ArmySelectionPlugin)
            .add_plugins(FrontlineRenderPlugin);
    }
}
