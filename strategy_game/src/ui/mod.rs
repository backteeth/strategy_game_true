pub mod country_selection;
pub mod diplomacy_panel;
pub mod economy_panel;
pub mod load_confirm;
pub mod military_panel;
pub mod notification;
pub mod peace_panel;
pub mod politics_panel;
pub mod research_panel;
pub mod scroll;
pub mod scrollbar;
pub mod state_panel;
pub mod tab_bar;
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
use tab_bar::TabBarPlugin;
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
    /// 講和パネル。他4パネルとは異なり、開閉の実体(`Node.display`)は
    /// 独自の`peace_panel::update_peace_panel_ui`が`PeacePanelState.open`から毎フレーム
    /// 導出する(講和パネルは戦争中でなければ元々表示されないため、通知メッセージや
    /// 戦況更新に応じた独自の描画タイミング調整が必要だった名残)。そのため
    /// `sync_panels_to_active`はここでは`Node`ではなく`PeacePanelState.open`を同期する。
    Peace,
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
    mut peace_state: ResMut<crate::ui::peace_panel::PeacePanelState>,
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
    // 講和パネル自身のtoggle System(`peace_panel::toggle_peace_panel`)は自分が
    // アクティブになる場合を直接`state.open = true`へ設定するが、他パネルのタブへ
    // 切り替えた場合にここで確実に閉じる(他4パネルのNode.display初期化と対になる処理)。
    peace_state.open = active_panel.current == PanelKind::Peace;
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
            .add_plugins(TabBarPlugin)
            .add_plugins(ResearchPluginUI)
            .add_plugins(PoliticsPluginUI)
            .add_plugins(DiplomacyPluginUI)
            .add_plugins(MilitaryPanelPlugin)
            .add_plugins(peace_panel::PeacePanelPlugin)
            .add_plugins(LoadConfirmPlugin)
            .add_plugins(TopBarPlugin)
            .add_plugins(crate::ui::scrollbar::ScrollbarPlugin)
            .init_resource::<crate::ui::scroll::CursorOverScrollableUi>()
            .add_systems(
                Update,
                (
                    sync_panels_to_active,
                    crate::ui::scroll::handle_panel_scroll,
                    crate::ui::scroll::update_cursor_over_scrollable_ui,
                )
                    .run_if(in_state(GameState::Playing)),
            );
    }
}
