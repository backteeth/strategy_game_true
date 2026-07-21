pub mod country_selection;
pub mod economy_panel;
pub mod notification;
pub mod state_panel;
pub mod top_bar;

use bevy::prelude::*;
use country_selection::CountrySelectionPlugin;
use economy_panel::EconomyPanelPlugin;
use state_panel::StatePanelPlugin;
use top_bar::TopBarPlugin;

/// 日本語フォント保持リソース
#[derive(Resource, Clone)]
pub struct JapaneseFont(pub Handle<Font>);

/// UI統合プラグイン
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_japanese_font)
            .add_plugins(CountrySelectionPlugin)
            .add_plugins(StatePanelPlugin)
            .add_plugins(EconomyPanelPlugin)
            .add_plugins(TopBarPlugin);
    }
}

fn setup_japanese_font(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font_handle: Handle<Font> = asset_server.load("fonts/JapaneseFont.ttc");
    commands.insert_resource(JapaneseFont(font_handle));
}
