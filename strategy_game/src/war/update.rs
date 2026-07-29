use crate::app::time::DayChangedMessage;
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;
use crate::war::justification::WarJustificationRegistry;
use bevy::prelude::*;

pub fn handle_daily_war(
    mut day_events: MessageReader<DayChangedMessage>,
    state_registry: Res<StateRegistry>,
    mut war_registry: ResMut<WarRegistry>,
    mut justification_registry: ResMut<WarJustificationRegistry>,
) {
    for _event in day_events.read() {
        justification_registry.process_daily_justifications(&state_registry);

        // 注: 戦闘処理は military::update::handle_daily_military に統合済み
        // war/combat.rs の旧 process_combat は使用しない

        crate::war::war_score::process_war_score(&state_registry, &mut war_registry);
    }
}
