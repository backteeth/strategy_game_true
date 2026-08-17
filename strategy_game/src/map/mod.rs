/// map モジュール
/// カメラ、描画、州選択を統合するプラグインを提供する
pub mod division_render;
pub mod division_selection;
pub mod camera;
pub mod frontline_render;
pub mod frontline_selection;
pub mod offensive_line_selection;
pub mod rendering;
pub mod selection;

use division_render::DivisionRenderPlugin;
use division_selection::DivisionSelectionPlugin;
use bevy::prelude::*;
use camera::CameraPlugin;
use frontline_render::FrontlineRenderPlugin;
use frontline_selection::FrontlineSelectionPlugin;
use offensive_line_selection::OffensiveLineSelectionPlugin;
use rendering::RenderingPlugin;
use selection::SelectionPlugin;

/// マップ関連の全プラグインをまとめるプラグイン
pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CameraPlugin)
            .add_plugins(RenderingPlugin)
            .add_plugins(SelectionPlugin)
            .add_plugins(DivisionRenderPlugin)
            .add_plugins(DivisionSelectionPlugin)
            .add_plugins(FrontlineRenderPlugin)
            .add_plugins(FrontlineSelectionPlugin)
            .add_plugins(OffensiveLineSelectionPlugin);
    }
}
