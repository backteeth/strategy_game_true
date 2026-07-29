use crate::app::time::DayChangedMessage;
use crate::country::CountryRegistry;
use crate::military::data::MilitaryRegistry;
use crate::state::data::StateRegistry;
use bevy::prelude::*;

pub fn handle_daily_military(
    mut day_events: MessageReader<DayChangedMessage>,
    mut country_registry: ResMut<CountryRegistry>,
    mut military_registry: ResMut<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
) {
    for _event in day_events.read() {
        crate::military::recruitment::process_recruitment(
            &mut country_registry,
            &mut military_registry,
        );
        crate::military::movement::process_movement(&mut military_registry, &state_registry);
        // crate::military::combat::process_combat(...);
        // crate::military::supply::process_supply(...);
    }
}
