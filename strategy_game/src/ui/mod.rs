pub mod country_selection;
pub mod diplomacy_panel;
pub mod economy_panel;
pub mod notification;
pub mod politics_panel;
pub mod research_panel;
pub mod state_panel;
pub mod top_bar;

use bevy::prelude::*;
use country_selection::CountrySelectionPlugin;
use diplomacy_panel::DiplomacyPluginUI;
use economy_panel::EconomyPanelPlugin;
use politics_panel::PoliticsPluginUI;
use research_panel::ResearchPluginUI;
use state_panel::StatePanelPlugin;
use top_bar::TopBarPlugin;

/// UI統合プラグイン
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CountrySelectionPlugin)
            .add_plugins(StatePanelPlugin)
            .add_plugins(EconomyPanelPlugin)
            .add_plugins(ResearchPluginUI)
            .add_plugins(PoliticsPluginUI)
            .add_plugins(DiplomacyPluginUI)
            .add_plugins(TopBarPlugin);
    }
}
