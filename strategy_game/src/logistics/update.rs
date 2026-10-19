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
    // P32-007: 計算の中核は`compute_logistics`へ集約し、旧`StateData`も新ステートも同じ関数を通る。
    crate::economy::state_access::update_logistics(state, building_registry);
}

/// 物流計算の結果(`compute_logistics`)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogisticsResult {
    pub capacity: f32,
    pub usage: f32,
    pub ratio: f32,
}

/// 物流容量・使用量・充足率の中核計算(P32-007でStateData依存から切り出し。式は従来どおり)。
pub fn compute_logistics(
    base_logistics_capacity: f32,
    buildings: &std::collections::HashMap<BuildingType, u32>,
    building_registry: &BuildingRegistry,
) -> LogisticsResult {
    let railway_level = buildings.get(&BuildingType::Railway).copied().unwrap_or(0);
    let railway_bonus = if let Some(def) = building_registry.get(BuildingType::Railway) {
        def.railway_capacity_bonus
    } else {
        50.0
    };

    let capacity = (base_logistics_capacity + (railway_level as f32) * railway_bonus).max(0.0);

    let mut usage = 0.0;
    for (&b_type, &level) in buildings {
        if level == 0 {
            continue;
        }
        if let Some(def) = building_registry.get(b_type) {
            usage += def.logistics_cost * (level as f32);
        }
    }

    let usage = usage.max(0.0);
    LogisticsResult {
        capacity,
        usage,
        ratio: calculate_logistics_ratio(capacity, usage),
    }
}
