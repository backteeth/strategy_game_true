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

/// 雇用率を計算する純粋関数 (0.0〜1.0、P23-003)。動員負担控除後の民間労働力
/// (`StateData::civilian_workforce()` = `employed_workforce + unemployed_workforce`)を
/// 分母とする。分母0では0を返す(`calculate_workforce_satisfaction`の「需要0なら
/// 充足率100%」とは意味が異なる別概念であり、混同しない)。
pub fn calculate_employment_rate(employed_workforce: u64, civilian_workforce: u64) -> f32 {
    if civilian_workforce == 0 {
        return 0.0;
    }
    ((employed_workforce as f64) / (civilian_workforce as f64)).clamp(0.0, 1.0) as f32
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculate_employment_rate_returns_zero_when_civilian_workforce_is_zero() {
        assert_eq!(calculate_employment_rate(0, 0), 0.0);
        assert_eq!(
            calculate_employment_rate(0, 0),
            0.0,
            "must not divide by zero or return NaN"
        );
    }

    #[test]
    fn calculate_employment_rate_computes_the_expected_ratio() {
        assert_eq!(calculate_employment_rate(50, 100), 0.5);
        assert_eq!(calculate_employment_rate(100, 100), 1.0);
        assert_eq!(calculate_employment_rate(0, 100), 0.0);
    }

    #[test]
    fn calculate_employment_rate_clamps_when_employed_exceeds_civilian_workforce() {
        // 通常は起こらない(employed <= civilian_workforceがupdate_state_population_and_
        // employmentの不変条件)が、防御的に1.0を超えない。
        assert_eq!(calculate_employment_rate(150, 100), 1.0);
    }

    #[test]
    fn civilian_workforce_sums_employed_and_unemployed() {
        let state = StateData {
            employed_workforce: 30_000,
            unemployed_workforce: 5_000,
            ..StateData::default()
        };
        assert_eq!(state.civilian_workforce(), 35_000);
    }

    #[test]
    fn civilian_workforce_does_not_overflow_at_the_u64_boundary() {
        let state = StateData {
            employed_workforce: u64::MAX,
            unemployed_workforce: u64::MAX,
            ..StateData::default()
        };
        assert_eq!(state.civilian_workforce(), u64::MAX);
    }
}
