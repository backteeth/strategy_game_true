use crate::building::data::BuildingRegistry;
use crate::state::data::{StateData, canonical_total_workforce};

/// 労働力を計算する純粋関数。`StateData::total_workforce()`と同じ正規計算
/// (`canonical_total_workforce`、P24-002)へ委譲する——両者は同じ「州の潜在総労働力」を
/// 表す概念であり、丸め規則を統一するため計算式の真実源を1か所に集約する。
pub fn calculate_workforce(population: u64, workforce_ratio: f32) -> u64 {
    canonical_total_workforce(population, workforce_ratio)
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

/// 経済用労働力比率を計算する純粋関数(P23-004)。`economy::finance`(生活水準・不満度)・
/// `economy::production`(建物稼働率)が共通で使う、雇用者数(`E`)を人口由来の潜在労働力
/// (`StateData::total_workforce()`、`P`)で割った比率`E / P`。
///
/// 上の`calculate_employment_rate`(UI用、分母は動員控除後の民間労働力`C`)とは
/// 分母が異なる別の指標であり、混同しない。`C > 0`のとき`E / P = (E / C) × (C / P)`と
/// 分解できるため、この比率には既に「動員後も民間に残っている労働力の割合(`C / P`)」と
/// 「残った民間労働力のうち雇用されている割合(`E / C`)」の両方が暗黙に含まれている——
/// 動員を明示的に二重適用する計算は行わない(`P23-004`の設計判断)。
///
/// 分母0では1.0を返す(`finance.rs`/`production.rs`が元々使っていた零除算フォールバック
/// をそのまま踏襲)。クランプはここでは行わない——`finance.rs`は元々クランプしておらず、
/// `production.rs`は呼び出し側で`.clamp(0.0, 1.0)`していたため、その違いをこの関数へ
/// 一方的に押し付けない。`f32`のみで除算する(`f64`を経由しない)のは、双方の呼び出し元が
/// 元々`as f32`の直接除算だった経路をビット単位で保つため。
pub fn calculate_economic_workforce_ratio(
    employed_workforce: u64,
    potential_workforce: u64,
) -> f32 {
    if potential_workforce == 0 {
        return 1.0;
    }
    employed_workforce as f32 / potential_workforce as f32
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
    // P32-007: 計算の中核は`compute_employment`へ集約し、旧`StateData`も新ステートも同じ関数を通る。
    crate::economy::state_access::update_employment(state, building_registry, mobilized_burden);
}

/// 雇用計算の結果(`compute_employment`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmploymentResult {
    /// 動員負担を控除した後の民間労働力。
    pub civilian_workforce: u64,
    pub employed: u64,
    pub unemployed: u64,
}

/// 月次雇用計算の中核(純粋関数、P32-007でStateData依存から切り出し)。式・丸め・処理は
/// 従来の`update_state_population_and_employment`と同一: 潜在労働力(`calculate_workforce`)から
/// 動員負担を`saturating_sub`で控除し、施設の必要労働力(`required_workforce * level`の合計)
/// までを雇用、残りを失業とする。
pub fn compute_employment(
    population: u64,
    workforce_ratio: f32,
    buildings: &std::collections::HashMap<crate::building::data::BuildingType, u32>,
    building_registry: &BuildingRegistry,
    mobilized_burden: u64,
) -> EmploymentResult {
    let total_wf =
        calculate_workforce(population, workforce_ratio).saturating_sub(mobilized_burden);

    let mut total_required_wf: u64 = 0;
    for (&b_type, &level) in buildings {
        if level == 0 {
            continue;
        }
        if let Some(def) = building_registry.get(b_type) {
            total_required_wf += (def.required_workforce as u64) * (level as u64);
        }
    }

    let employed = if total_wf >= total_required_wf {
        total_required_wf
    } else {
        total_wf
    };

    EmploymentResult {
        civilian_workforce: total_wf,
        employed,
        unemployed: calculate_unemployed_workforce(total_wf, employed),
    }
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

    // ─── P23-004: calculate_economic_workforce_ratio ──────────────────────────

    #[test]
    fn calculate_economic_workforce_ratio_matches_the_original_finance_and_production_formula() {
        // 元のfinance.rs/production.rsの`employed as f32 / total_workforce as f32`と
        // ビット単位で一致することを固定する(f64を経由しない)。
        let expected = 20_000f32 / 45_000f32;
        assert_eq!(calculate_economic_workforce_ratio(20_000, 45_000), expected);
    }

    #[test]
    fn calculate_economic_workforce_ratio_returns_one_for_zero_potential_workforce() {
        // finance.rs/production.rsの元の零除算フォールバックを踏襲する。
        assert_eq!(calculate_economic_workforce_ratio(0, 0), 1.0);
        assert_eq!(calculate_economic_workforce_ratio(12_345, 0), 1.0);
    }

    #[test]
    fn calculate_economic_workforce_ratio_is_zero_when_employed_is_zero_and_potential_is_positive()
    {
        assert_eq!(calculate_economic_workforce_ratio(0, 100), 0.0);
    }

    #[test]
    fn calculate_economic_workforce_ratio_is_one_when_employed_equals_potential() {
        assert_eq!(calculate_economic_workforce_ratio(100, 100), 1.0);
    }

    #[test]
    fn calculate_economic_workforce_ratio_is_not_clamped_when_employed_exceeds_potential() {
        // finance.rsの元の計算はクランプしていなかった。この関数自体もクランプしない
        // (クランプが必要な呼び出し元は自分で`.clamp(0.0, 1.0)`を適用する)。
        let ratio = calculate_economic_workforce_ratio(150, 100);
        assert!(
            ratio > 1.0,
            "must stay unclamped so finance.rs's original (uncapped) behavior is preserved, got {ratio}"
        );
        assert_eq!(ratio, 1.5);
    }

    #[test]
    fn calculate_economic_workforce_ratio_does_not_panic_or_overflow_at_the_u64_boundary() {
        let ratio = calculate_economic_workforce_ratio(u64::MAX, u64::MAX);
        assert!(ratio.is_finite());
        let ratio_zero = calculate_economic_workforce_ratio(0, u64::MAX);
        assert_eq!(ratio_zero, 0.0);
    }

    #[test]
    fn calculate_economic_workforce_ratio_is_monotonically_non_increasing_as_employed_falls() {
        let potential = 100_000;
        let high = calculate_economic_workforce_ratio(90_000, potential);
        let mid = calculate_economic_workforce_ratio(60_000, potential);
        let low = calculate_economic_workforce_ratio(30_000, potential);
        assert!(high >= mid);
        assert!(mid >= low);
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
