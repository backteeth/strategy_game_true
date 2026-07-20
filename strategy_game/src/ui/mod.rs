/// ゲームUIモジュール
/// 各種パネルやメニューを統合する
pub mod country_selection;
pub mod state_panel;
pub mod top_bar;

use bevy::prelude::*;
use country_selection::CountrySelectionPlugin;
use state_panel::StatePanelPlugin;
use top_bar::TopBarPlugin;

/// UI統合プラグイン
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CountrySelectionPlugin)
            .add_plugins(StatePanelPlugin)
            .add_plugins(TopBarPlugin);
    }
}
