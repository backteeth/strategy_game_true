pub mod country_selection;
pub mod diplomacy_panel;
pub mod economy_panel;
pub mod load_confirm;
pub mod military_panel;
pub mod notification;
pub mod peace_panel;
pub mod politics_panel;
pub mod research_panel;
pub mod state_panel;
pub mod top_bar;

use crate::localization::LocalizationPlugin;
use bevy::prelude::*;
use country_selection::CountrySelectionPlugin;
use diplomacy_panel::DiplomacyPluginUI;
use economy_panel::EconomyPanelPlugin;
use load_confirm::LoadConfirmPlugin;
use military_panel::MilitaryPanelPlugin;
use politics_panel::PoliticsPluginUI;
use research_panel::ResearchPluginUI;
use state_panel::StatePanelPlugin;
use top_bar::TopBarPlugin;

/// 現在開いているパネルの識別子
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelKind {
    #[default]
    None,
    Research,
    Politics,
    Diplomacy,
    Military,
}

/// アクティブなパネルを一元管理するリソース
/// これを変更することで、他のパネルが自動的に閉じる
#[derive(Resource, Default)]
pub struct ActivePanel {
    pub current: PanelKind,
}

impl ActivePanel {
    /// パネルをトグルする。既に開いていれば閉じ、閉じていれば開く
    pub fn toggle(&mut self, kind: PanelKind) {
        if self.current == kind {
            self.current = PanelKind::None;
        } else {
            self.current = kind;
        }
    }
}

use crate::app::game_state::GameState;
use crate::ui::diplomacy_panel::DiplomacyPanelRoot;
use crate::ui::military_panel::MilitaryPanelRoot;
use crate::ui::politics_panel::PoliticsPanelRoot;
use crate::ui::research_panel::ResearchPanelRoot;

/// ActivePanel が変化したとき、各パネルの display を同期する
#[allow(clippy::type_complexity)]
fn sync_panels_to_active(
    active_panel: Res<ActivePanel>,
    mut research_q: Query<
        &mut Node,
        (
            With<ResearchPanelRoot>,
            Without<PoliticsPanelRoot>,
            Without<DiplomacyPanelRoot>,
            Without<MilitaryPanelRoot>,
        ),
    >,
    mut politics_q: Query<
        &mut Node,
        (
            With<PoliticsPanelRoot>,
            Without<ResearchPanelRoot>,
            Without<DiplomacyPanelRoot>,
            Without<MilitaryPanelRoot>,
        ),
    >,
    mut diplomacy_q: Query<
        &mut Node,
        (
            With<DiplomacyPanelRoot>,
            Without<ResearchPanelRoot>,
            Without<PoliticsPanelRoot>,
            Without<MilitaryPanelRoot>,
        ),
    >,
    mut military_q: Query<
        &mut Node,
        (
            With<MilitaryPanelRoot>,
            Without<ResearchPanelRoot>,
            Without<PoliticsPanelRoot>,
            Without<DiplomacyPanelRoot>,
        ),
    >,
) {
    if !active_panel.is_changed() {
        return;
    }
    if let Ok(mut node) = research_q.single_mut() {
        node.display = if active_panel.current == PanelKind::Research {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(mut node) = politics_q.single_mut() {
        node.display = if active_panel.current == PanelKind::Politics {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(mut node) = diplomacy_q.single_mut() {
        node.display = if active_panel.current == PanelKind::Diplomacy {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(mut node) = military_q.single_mut() {
        node.display = if active_panel.current == PanelKind::Military {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// UI統合プラグイン
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(LocalizationPlugin)
            .insert_resource(ActivePanel::default())
            .add_plugins(CountrySelectionPlugin)
            .add_plugins(StatePanelPlugin)
            .add_plugins(EconomyPanelPlugin)
            .add_plugins(ResearchPluginUI)
            .add_plugins(PoliticsPluginUI)
            .add_plugins(DiplomacyPluginUI)
            .add_plugins(MilitaryPanelPlugin)
            .add_plugins(peace_panel::PeacePanelPlugin)
            .add_plugins(LoadConfirmPlugin)
            .add_plugins(TopBarPlugin)
            .add_systems(
                Update,
                sync_panels_to_active.run_if(in_state(GameState::Playing)),
            );
    }
}
