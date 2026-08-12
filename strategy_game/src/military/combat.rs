use crate::military::data::{Division, MilitaryRegistry};

pub fn calculate_combat_strength(division: &Division, registry: &MilitaryRegistry) -> f32 {
    let def = registry.definitions.get(&division.def_id).unwrap();
    let manpower_ratio = division.manpower as f32 / division.max_manpower as f32;
    let equip_ratio = if division.max_equipment > 0.0 {
        division.equipment as f32 / division.max_equipment as f32
    } else {
        1.0
    };
    let org_ratio = division.organization / division.max_organization;

    let base_attack = def.attack;
    base_attack * manpower_ratio.min(equip_ratio) * org_ratio * division.supply_ratio
}

pub fn process_combat() {
    // TODO: Full combat resolution between divisions in same state
}
