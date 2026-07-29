use crate::app::time::DayChangedMessage;
use crate::military::data::MilitaryRegistry;
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;
use crate::war::justification::WarJustificationRegistry;
use bevy::prelude::*;

pub fn handle_daily_war(
    mut day_events: MessageReader<DayChangedMessage>,
    mut state_registry: ResMut<StateRegistry>,
    mut military_registry: ResMut<MilitaryRegistry>,
    mut war_registry: ResMut<WarRegistry>,
    mut justification_registry: ResMut<WarJustificationRegistry>,
) {
    for _event in day_events.read() {
        justification_registry.process_daily_justifications(&state_registry);

        crate::war::combat::process_combat(&mut military_registry, &war_registry);

        crate::war::occupation::process_occupation(
            &mut state_registry,
            &mut military_registry,
            &war_registry,
        );

        crate::war::war_score::process_war_score(&state_registry, &mut war_registry);
    }
}
