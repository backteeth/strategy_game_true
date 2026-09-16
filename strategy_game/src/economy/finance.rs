use crate::building::data::BuildingRegistry;
use crate::country::CountryData;
use crate::population::update::calculate_economic_workforce_ratio;
use crate::state::data::StateData;

pub const TAX_COEFFICIENT: f64 = 0.0001;
pub const ADMIN_COEFFICIENT: f64 = 0.001;
pub const LOGISTICS_MAINTENANCE_COEFFICIENT: f64 = 0.05;

/// P27-002-B: tax_rateを1回の操作で変更する幅。プレイヤーUI(`ui::economy_panel`の
/// 税率±ボタン)とAI税率自己調整(`country::country_ai::process_tax_rate_ai`)の
/// 両方が、この定数と下の`apply_tax_rate_delta`を共有する唯一の変更経路とする
/// (P27-002-B-PRE-001 spec §4: 既存の刻みを再利用し、AI専用の新しい刻みを
/// 作らない)。
pub const TAX_RATE_STEP: f32 = 0.01;

/// tax_rateへ`delta`を適用し、既存の正規範囲`[0.0, 1.0]`でclampする(プレイヤーUIの
/// 税率±ボタンが元々インラインで行っていたのと同じ検証・適用ロジックを、AIからも
/// 再利用できる共通関数として抽出したもの)。呼び出し側でのみ`delta`の符号を選ぶため、
/// AI専用の新しい範囲を作らない。
pub fn apply_tax_rate_delta(current_tax_rate: f32, delta: f32) -> f32 {
    (current_tax_rate + delta).clamp(0.0, 1.0)
}

/// P27-002-B候補A: AI国家の月次tax_rate自己調整を、既存の`process_country_finance`が
/// 既に算出した当月の確定済み`monthly_balance`だけから決定する純粋関数(税収/行政費の
/// 式自体はここへ複製しない)。
///
/// - 月次収支が負(赤字)→ 引き上げ(`+TAX_RATE_STEP`)。
/// - 月次収支が正(黒字)かつ現在のtax_rateが既定値(`country::DEFAULT_TAX_RATE`、
///   P27-002-B-PRE-001 candidate Aの「基準値まで戻す」という確定仕様)を上回る場合
///   → 引き下げ(`-TAX_RATE_STEP`)。
/// - 月次収支がちょうど0、または黒字だが既に基準値以下 → 据え置き(`None`)。
/// - `monthly_balance`・`current_tax_rate`のいずれかが非有限値(NaN/Infinity)の場合は、
///   0へのコアーションではなく安全に据え置く(`None`)。
///
/// ヒステリシス幅・独自の目標黒字額・国庫残高に応じた比例制御は意図的に持たない
/// (P27-002-B-PRE-001 spec §3: 候補Aへ勝手に新しい調整ロジックを追加しない)。
pub fn decide_ai_tax_rate_delta(monthly_balance: f64, current_tax_rate: f32) -> Option<f32> {
    if !monthly_balance.is_finite() || !current_tax_rate.is_finite() {
        return None;
    }
    if monthly_balance < 0.0 {
        Some(TAX_RATE_STEP)
    } else if monthly_balance > 0.0 && current_tax_rate > crate::country::DEFAULT_TAX_RATE {
        Some(-TAX_RATE_STEP)
    } else {
        None
    }
}

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

    // 経済用労働力比率(P23-004: population::update::calculate_economic_workforce_ratio
    // へ集約。UI用のcivilian employment rate(P23-003)とは分母が異なる別指標であり、
    // 混同しない——詳細は同関数のドキュメントコメントを参照)。
    let emp_rate =
        calculate_economic_workforce_ratio(state.employed_workforce, state.total_workforce());
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
