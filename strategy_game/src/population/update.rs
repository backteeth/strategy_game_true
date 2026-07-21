use crate::building::data::BuildingRegistry;
use crate::state::data::StateData;

/// 労働力を計算する純粋関数
pub fn calculate_workforce(population: u64, workforce_ratio: f32) -> u64 {
    ((population as f64) * (workforce_ratio as f64))
        .round()
        .max(0.0) as u64
}

/// 失業者数を計算する純粋関数
pub fn calculate_unemployed_workforce(total_workforce: u64, employed_workforce: u64) -> u64 {
    total_workforce.saturating_sub(employed_workforce)
}

/// 労働力充足率を計算する純粋関数 (0.0〜1.0)
pub fn calculate_workforce_satisfaction(available_workforce: u64, required_workforce: u64) -> f32 {
    if required_workforce == 0 {
        return 1.0;
    }
    ((available_workforce as f64) / (required_workforce as f64)).clamp(0.0, 1.0) as f32
}

/// 州の人口と労働力、雇用状況を更新する
pub fn update_state_population_and_employment(
    state: &mut StateData,
    building_registry: &BuildingRegistry,
) {
    let total_wf = calculate_workforce(state.population, state.workforce_ratio);

    let mut total_required_wf: u64 = 0;
    for (&b_type, &level) in &state.buildings {
        if level == 0 {
            continue;
        }
        if let Some(def) = building_registry.get(b_type) {
            total_required_wf += (def.required_workforce as u64) * (level as u64);
        }
    }

    let _satisfaction = calculate_workforce_satisfaction(total_wf, total_required_wf);
    let employed = if total_wf >= total_required_wf {
        total_required_wf
    } else {
        total_wf
    };

    state.employed_workforce = employed;
    state.unemployed_workforce = calculate_unemployed_workforce(total_wf, employed);
}
