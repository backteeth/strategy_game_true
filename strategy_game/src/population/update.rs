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

/// 州の人口と労働力、雇用状況を更新する。
///
/// `mobilized_burden`(P23-002: 国家の`mobilized_manpower`をこの州の人口比で按分した値、
/// `population::mobilization::apportion_mobilized_manpower`が算出する派生値)を、
/// 民間労働力から`saturating_sub`で控除してから雇用計算へ渡す。人口(`state.population`)
/// 自体は変更しない——動員は民間労働力の一時的な圧迫であり、住民が消えるわけではない。
pub fn update_state_population_and_employment(
    state: &mut StateData,
    building_registry: &BuildingRegistry,
    mobilized_burden: u64,
) {
    let total_wf = calculate_workforce(state.population, state.workforce_ratio)
        .saturating_sub(mobilized_burden);

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
