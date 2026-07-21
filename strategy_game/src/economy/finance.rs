use crate::building::data::BuildingRegistry;
use crate::country::CountryData;
use crate::state::data::StateData;

pub const TAX_COEFFICIENT: f64 = 0.0001;
pub const ADMIN_COEFFICIENT: f64 = 0.001;
pub const LOGISTICS_MAINTENANCE_COEFFICIENT: f64 = 0.05;

/// 州の税収を計算する純粋関数
pub fn calculate_state_tax(population: u64, living_standard: f32, tax_rate: f32) -> f64 {
    (population as f64) * (living_standard as f64) * (tax_rate as f64) * TAX_COEFFICIENT
}

/// 州の行政費を計算する純粋関数
pub fn calculate_admin_cost(population: u64) -> f64 {
    (population as f64) * ADMIN_COEFFICIENT
}

/// 月次収支を計算する純粋関数
pub fn calculate_monthly_balance(income: f64, expenses: f64) -> f64 {
    income - expenses
}

/// 1つの国家の税収・支出・財政を計算して適用する
pub fn process_country_finance(
    country: &mut CountryData,
    states: &mut [&mut StateData],
    building_registry: &BuildingRegistry,
) {
    let mut total_income = 0.0;
    let mut total_expenses = 0.0;
    let mut total_pop = 0u64;

    for state in states.iter() {
        total_pop += state.population;

        // 税収
        let tax = calculate_state_tax(state.population, state.living_standard, country.tax_rate);
        total_income += tax;

        // 建物維持費
        for (&b_type, &level) in &state.buildings {
            if level == 0 {
                continue;
            }
            if let Some(def) = building_registry.get(b_type) {
                let op = state.building_operation(b_type);
                total_expenses += def.maintenance_cost * (level as f64) * (op as f64);
            }
        }

        // 物流維持費
        total_expenses += (state.logistics_capacity as f64) * LOGISTICS_MAINTENANCE_COEFFICIENT;
    }

    // 行政費
    let admin_cost = calculate_admin_cost(total_pop);
    total_expenses += admin_cost;

    let balance = calculate_monthly_balance(total_income, total_expenses);
    country.monthly_income = total_income;
    country.monthly_expenses = total_expenses;
    country.monthly_balance = balance;
    country.treasury += balance;
}

/// 州の生活水準と不満度を更新する
pub fn update_state_welfare_and_unrest(
    state: &mut StateData,
    tax_rate: f32,
    has_food: bool,
    is_bankrupt: bool,
) {
    let mut ls_delta = 0.0f32;
    let mut unrest_delta = 0.0f32;

    // 食料
    if has_food {
        ls_delta += 0.5;
    } else {
        ls_delta -= 1.0;
        unrest_delta += 1.0;
    }

    // 雇用率
    let emp_rate = if state.total_workforce() > 0 {
        state.employed_workforce as f32 / state.total_workforce() as f32
    } else {
        1.0
    };
    if emp_rate >= 0.9 {
        ls_delta += 0.5;
    } else if emp_rate < 0.7 {
        ls_delta -= 0.8;
        unrest_delta += 0.8;
    }

    // 物流
    if state.logistics_ratio >= 0.9 {
        ls_delta += 0.3;
    } else if state.logistics_ratio < 0.5 {
        ls_delta -= 0.5;
        unrest_delta += 0.5;
    }

    // 税率
    if tax_rate <= 0.10 {
        ls_delta += 0.5;
    } else if tax_rate > 0.25 {
        ls_delta -= 0.8;
        unrest_delta += 0.8;
    }

    // 財政赤字
    if is_bankrupt {
        ls_delta -= 0.5;
        unrest_delta += 1.0;
    }

    // 月あたり最大±2.0に制限
    let ls_change = ls_delta.clamp(-2.0, 2.0);
    let unrest_change = unrest_delta.clamp(-2.0, 2.0);

    state.living_standard = (state.living_standard + ls_change).clamp(0.0, 100.0);
    state.unrest = (state.unrest + unrest_change).clamp(0.0, 100.0);
}
