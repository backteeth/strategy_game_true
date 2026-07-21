use crate::building::data::{BuildingRegistry, BuildingType};
use crate::state::data::StateData;

/// 物流充足率を計算する純粋関数 (0.0〜1.0)
pub fn calculate_logistics_ratio(capacity: f32, usage: f32) -> f32 {
    if usage <= 0.0 {
        return 1.0;
    }
    (capacity / usage).clamp(0.0, 1.0)
}

/// 州の物流容量・使用量・充足率を計算する
pub fn update_state_logistics(state: &mut StateData, building_registry: &BuildingRegistry) {
    let railway_level = state.building_level(BuildingType::Railway);
    let railway_bonus = if let Some(def) = building_registry.get(BuildingType::Railway) {
        def.railway_capacity_bonus
    } else {
        50.0
    };

    state.logistics_capacity =
        (state.base_logistics_capacity + (railway_level as f32) * railway_bonus).max(0.0);

    let mut usage = 0.0;
    for (&b_type, &level) in &state.buildings {
        if level == 0 {
            continue;
        }
        if let Some(def) = building_registry.get(b_type) {
            usage += def.logistics_cost * (level as f32);
        }
    }

    state.logistics_usage = usage.max(0.0);
    state.logistics_ratio =
        calculate_logistics_ratio(state.logistics_capacity, state.logistics_usage);
}
