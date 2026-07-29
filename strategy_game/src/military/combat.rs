use crate::military::data::{ArmyUnit, MilitaryRegistry};

pub fn calculate_combat_strength(army: &ArmyUnit, registry: &MilitaryRegistry) -> f32 {
    let def = registry.definitions.get(&army.def_id).unwrap();
    let manpower_ratio = army.manpower as f32 / army.max_manpower as f32;
    let equip_ratio = if army.max_equipment > 0.0 {
        army.equipment as f32 / army.max_equipment as f32
    } else {
        1.0
    };
    let org_ratio = army.organization / army.max_organization;

    let base_attack = def.attack;
    base_attack * manpower_ratio.min(equip_ratio) * org_ratio * army.supply_ratio
}

pub fn process_combat() {
    // TODO: Full combat resolution between armies in same state
}
