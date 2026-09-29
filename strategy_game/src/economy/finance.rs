use crate::building::data::BuildingRegistry;
use crate::country::CountryData;
use crate::population::update::calculate_economic_workforce_ratio;
use crate::state::data::StateData;

pub const TAX_COEFFICIENT: f64 = 0.0001;
/// P27-002-C: P27-002-C-PRE-001の120か月監査が実測した長期崩壊(Ferrowynの
/// 生活水準が月54で0.00に到達し税収が恒久的に消失、Arcadia/Dwarfが黒字化後に
/// 再崩壊、Oceanicも月120で収支が負転)を踏まえた較正。この変更はAI専用補正では
/// なく正規財政式そのものの較正であり、プレイヤー国家にも同じ係数が適用される
/// (`process_country_finance`から国家種別を問わず一律に呼ばれるため、コード上
/// 自然にそうなる——国家ID特例は追加していない)。
pub const ADMIN_COEFFICIENT: f64 = 0.0008;
pub const LOGISTICS_MAINTENANCE_COEFFICIENT: f64 = 0.05;

/// P28-005: `update_state_welfare_and_unrest`の物流生活水準ボーナスの境界
/// (物流比率がこの値以上なら生活水準+0.3/月、これ未満で0.5未満なら-0.5/月)。
/// 建設AIのRailway優先建設(`country::country_ai`優先度2.7)は、この定数を
/// そのまま対象州の選定閾値として共有する——生活水準が改善に転じる物流比率と、
/// AIがRailwayを検討する物流比率を、同じ定数で常に一致させるため
/// (`verification_logs/phase-28/p28-005-pre-001/`の監査で確認した境界)。
pub const LOGISTICS_WELFARE_BONUS_RATIO: f32 = 0.9;

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

/// P27-002-C: AI国家の月次tax_rate自己調整を、既存の`process_country_finance`が
/// 既に算出した当月の確定済み`monthly_balance`・`monthly_expenses`と、当月の
/// 人口・平均生活水準だけから決定する純粋関数。税収の式自体は複製せず、
/// `calculate_state_tax`(本会計が使うのと全く同じ関数)をそのまま呼ぶ。
///
/// 判定優先順位(P27-002-C-PRE-001 候補4、spec §3.7):
/// 1. 月次収支が負(赤字)→ 引き上げ(`+TAX_RATE_STEP`)。予測ゲートは適用しない
///    (赤字時の引上げを妨げてはいけない、spec §3.7)。
/// 2. 月次収支がちょうど0 → 据え置き。
/// 3. 月次収支が正(黒字)→ 1段階下げた`candidate_rate`での予測税収
///    (`population × average_living_standard × candidate_rate × TAX_COEFFICIENT`、
///    `calculate_state_tax`を再利用)が当月の`monthly_expenses`以上を賄える場合だけ
///    引き下げる(`-TAX_RATE_STEP`)。それ以外は据え置き。
///
/// `average_living_standard`は人口加重平均でなければならない
/// (`Σ population_i × living_standard_i / Σ population_i`)——これにより
/// `candidate_rate`に現在の`tax_rate`をそのまま渡した場合、この関数の予測値が
/// 実際の`monthly_income`と数学的に完全一致する(州ごとの`calculate_state_tax`の
/// 合計と、加重平均を使った国家単位の`calculate_state_tax`が恒等になるため)。
/// 呼び出し側(`country_ai::process_tax_rate_ai`)はこの人口加重平均を計算してから
/// 渡す。
///
/// 新しい税率下限は導入しない(既存範囲`[0.0, 1.0]`を`apply_tax_rate_delta`が
/// そのままclampする)。`monthly_balance`・`current_tax_rate`が非有限値の場合は
/// 安全に据え置く(`None`)。`average_living_standard`・`monthly_expenses`の
/// 妥当性検証(非有限値・負値の拒否)は黒字分岐でのみ行う——赤字時の引上げは
/// これらを一切参照しないため、無効な値によって引上げが妨げられてはいけない
/// (spec §3.7)。いずれの場合も0へのコアーションは行わない。
///
/// ヒステリシス幅・独自の目標黒字額・国庫残高に応じた比例制御・将来の建設や
/// 人口変動を先読みするシミュレーターは意図的に持たない(spec §3.5・§4)。
pub fn decide_ai_tax_rate_delta(
    monthly_balance: f64,
    current_tax_rate: f32,
    population: u64,
    average_living_standard: f32,
    monthly_expenses: f64,
) -> Option<f32> {
    if !monthly_balance.is_finite() || !current_tax_rate.is_finite() {
        return None;
    }

    if monthly_balance < 0.0 {
        // spec §3.7: 赤字時の引上げを予測ゲートで妨げてはいけない。この分岐は
        // population/average_living_standard/monthly_expensesを一切参照しないため、
        // それらの妥当性チェックより前に確定させる。
        return Some(TAX_RATE_STEP);
    }

    if monthly_balance > 0.0 {
        if !average_living_standard.is_finite()
            || average_living_standard < 0.0
            || !monthly_expenses.is_finite()
            || monthly_expenses < 0.0
        {
            return None;
        }
        let candidate_rate = (current_tax_rate - TAX_RATE_STEP).max(0.0);
        if candidate_rate < current_tax_rate {
            let projected_tax_income = calculate_state_tax(population, average_living_standard, candidate_rate);
            if projected_tax_income >= monthly_expenses {
                return Some(-TAX_RATE_STEP);
            }
        }
    }

    None
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
    if state.logistics_ratio >= LOGISTICS_WELFARE_BONUS_RATIO {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
    use crate::common::{CountryId, StateId};
    use std::collections::HashMap;

    // ─── §9.1: 行政費較正 ───────────────────────────────────────────────────

    #[test]
    fn admin_cost_is_zero_for_zero_population() {
        assert_eq!(calculate_admin_cost(0), 0.0);
    }

    #[test]
    fn admin_cost_scales_with_population_at_the_new_coefficient() {
        assert_eq!(calculate_admin_cost(1_000_000), 1_000_000.0 * 0.0008);
        assert_eq!(ADMIN_COEFFICIENT, 0.0008, "P27-002-C較正値であることを固定する");
    }

    #[test]
    fn admin_cost_is_independent_across_populations() {
        let a = calculate_admin_cost(500_000);
        let b = calculate_admin_cost(1_500_000);
        assert_eq!(b, a * 3.0, "人口比例のはず(国家間の按分・特例なし)");
    }

    fn building_def(building_type: BuildingType, maintenance_cost: f64) -> BuildingDefinition {
        BuildingDefinition {
            building_type,
            name: "test".to_string(),
            construction_cost: 0.0,
            required_progress: 0.0,
            required_workforce: 0.0,
            logistics_cost: 0.0,
            input_resources: HashMap::new(),
            output_resources: HashMap::new(),
            maintenance_cost,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        }
    }

    #[test]
    fn process_country_finance_adds_admin_cost_without_touching_other_expense_items() {
        let mut country = CountryData { id: CountryId(1), tax_rate: 0.15, ..CountryData::default() };
        let mut state = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            population: 1_000_000,
            living_standard: 50.0,
            logistics_capacity: 100.0,
            ..StateData::default()
        };
        state.buildings.insert(BuildingType::Farm, 1);
        state.building_operation_ratios.insert(BuildingType::Farm, 1.0);
        let mut registry = BuildingRegistry::default();
        registry.definitions.insert(BuildingType::Farm, building_def(BuildingType::Farm, 20.0));

        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_finance(&mut country, &mut states, &registry);

        let expected_admin = 1_000_000.0 * ADMIN_COEFFICIENT;
        let expected_building_maint = 20.0; // maintenance_cost(20) * level(1) * operation(1.0)
        let expected_logistics_maint = 100.0 * LOGISTICS_MAINTENANCE_COEFFICIENT;
        let expected_expenses = expected_admin + expected_building_maint + expected_logistics_maint;
        assert_eq!(country.monthly_expenses, expected_expenses);
        // 建物維持費・物流維持費は行政費係数の変更と無関係に既存式のままであることを固定する。
        assert_eq!(expected_building_maint, 20.0);
        assert_eq!(expected_logistics_maint, 5.0);
    }

    // ─── §9.2〜§9.4: 予測安全な税率引下げ判定 ──────────────────────────────

    #[test]
    fn deficit_raises_tax_rate_regardless_of_projection() {
        // 予測が不可能なほど支出が巨額でも、赤字ならまず引き上げる(spec §3.7)。
        assert_eq!(decide_ai_tax_rate_delta(-1.0, 0.15, 1, 0.0, 1_000_000.0), Some(TAX_RATE_STEP));
    }

    #[test]
    fn zero_monthly_balance_holds() {
        assert_eq!(decide_ai_tax_rate_delta(0.0, 0.20, 1_000_000, 50.0, 100.0), None);
    }

    #[test]
    fn surplus_with_safe_projection_decreases() {
        // pop=1,000,000 ls=50.0 candidate=0.19 → projected=950.0 >= expenses=900.0
        let result = decide_ai_tax_rate_delta(1.0, 0.20, 1_000_000, 50.0, 900.0);
        assert_eq!(result, Some(-TAX_RATE_STEP));
    }

    #[test]
    fn surplus_with_projection_exactly_equal_to_expenses_decreases() {
        // 予測値と全く同じ浮動小数点演算経路(f32減算→f64キャスト→calculate_state_tax)
        // で期待値を算出し、真の境界(`>=`の等号側)を検証する——手打ちの十進値
        // (例:0.20-0.01=0.19)はf32丸め誤差により厳密な等価にならないため使わない。
        let current_tax_rate = 0.20f32;
        let candidate_rate = (current_tax_rate - TAX_RATE_STEP).max(0.0);
        let population = 1_000_000u64;
        let average_living_standard = 50.0f32;
        let exact_projected_income = calculate_state_tax(population, average_living_standard, candidate_rate);

        let result = decide_ai_tax_rate_delta(
            1.0,
            current_tax_rate,
            population,
            average_living_standard,
            exact_projected_income,
        );
        assert_eq!(result, Some(-TAX_RATE_STEP), "予測税収と支出が正確に等しい場合も引下げてよい");
    }

    #[test]
    fn surplus_with_unsafe_projection_holds() {
        // projected=950.0 < expenses=951.0
        let result = decide_ai_tax_rate_delta(1.0, 0.20, 1_000_000, 50.0, 951.0);
        assert_eq!(result, None, "予測税収が支出を下回るなら据置くはず");
    }

    #[test]
    fn decreases_from_0_15_to_0_14_when_safe() {
        // candidate=0.14, pop=1,000,000, ls=50.0 → projected=700.0
        let result = decide_ai_tax_rate_delta(1.0, 0.15, 1_000_000, 50.0, 700.0);
        assert_eq!(result, Some(-TAX_RATE_STEP), "0.15は絶対下限ではないはず(P27-002-Cで撤廃)");
    }

    #[test]
    fn decreases_from_0_01_to_0_00_when_safe() {
        // candidate=max(0.0, 0.01-0.01)=0.0 → projected=0.0、expenses=0.0で境界的に安全
        let result = decide_ai_tax_rate_delta(1.0, 0.01, 1_000_000, 50.0, 0.0);
        assert_eq!(result, Some(-TAX_RATE_STEP));
    }

    #[test]
    fn holds_at_0_00_lower_bound() {
        // candidate=max(0.0, 0.0-0.01)=0.0、candidate < current(0.0)が偽なので据置
        let result = decide_ai_tax_rate_delta(1.0, 0.0, 1_000_000, 50.0, 0.0);
        assert_eq!(result, None, "新しい下限は追加しないが、0.0からはこれ以上下げようがないはず");
    }

    #[test]
    fn surplus_but_unsafe_candidate_holds_even_with_large_population() {
        // 人口が大きくても生活水準が低ければ予測税収は小さいままのはず。
        let result = decide_ai_tax_rate_delta(1.0, 0.20, 10_000_000, 1.0, 10_000.0);
        assert_eq!(result, None);
    }

    #[test]
    fn treasury_is_irrelevant_only_monthly_balance_sign_matters() {
        // decide_ai_tax_rate_delta自体はtreasuryを一切参照しない(引数に存在しない)
        // ——国庫が巨額・国庫が負のいずれでも、monthly_balanceの符号だけで判定される
        // ことを、同じ入力で2回呼んで同一結果になることにより間接確認する。
        let a = decide_ai_tax_rate_delta(1.0, 0.20, 1_000_000, 50.0, 900.0);
        let b = decide_ai_tax_rate_delta(1.0, 0.20, 1_000_000, 50.0, 900.0);
        assert_eq!(a, b);
        assert_eq!(a, Some(-TAX_RATE_STEP));
    }

    #[test]
    fn candidate_rate_never_goes_negative() {
        // current=0.0のとき、内部のcandidate_rateはmax(0.0, -0.01)=0.0にclampされ、
        // 負の税率が計算に使われないことを、0.0での据置結果を通じて確認する。
        assert_eq!(decide_ai_tax_rate_delta(1.0, 0.0, 1_000_000, 50.0, 0.0), None);
    }

    #[test]
    fn rejects_non_finite_or_negative_inputs_without_mutation() {
        assert_eq!(decide_ai_tax_rate_delta(f64::NAN, 0.20, 1_000_000, 50.0, 900.0), None);
        assert_eq!(decide_ai_tax_rate_delta(1.0, f32::NAN, 1_000_000, 50.0, 900.0), None);
        assert_eq!(decide_ai_tax_rate_delta(1.0, 0.20, 1_000_000, f32::NAN, 900.0), None);
        assert_eq!(decide_ai_tax_rate_delta(1.0, 0.20, 1_000_000, -1.0, 900.0), None);
        assert_eq!(decide_ai_tax_rate_delta(1.0, 0.20, 1_000_000, 50.0, f64::NAN), None);
        assert_eq!(decide_ai_tax_rate_delta(1.0, 0.20, 1_000_000, 50.0, -1.0), None);
        // spec §3.7: 赤字判定は予測ゲート用の入力(living_standard/monthly_expenses)を
        // 一切参照しないため、それらが非有限でも引上げは妨げられないはず。
        assert_eq!(
            decide_ai_tax_rate_delta(-1.0, 0.20, 1_000_000, f32::NAN, f64::INFINITY),
            Some(TAX_RATE_STEP),
            "赤字時の引上げは予測ゲート用入力の妥当性チェックより前に確定するため妨げられない"
        );
    }

    /// §9.4 Test 23: 候補レートに現在のtax_rateをそのまま渡した場合、予測税収が
    /// 実際の`monthly_income`(複数州の`calculate_state_tax`合計)と数学的に完全に
    /// 一致することを確認する——人口加重平均を使う設計の正当性を直接証明する。
    #[test]
    fn projected_income_matches_real_monthly_income_when_using_the_current_rate() {
        let tax_rate = 0.20f32;
        let states = [(300_000u64, 40.0f32), (700_000u64, 80.0f32)];
        let real_monthly_income: f64 =
            states.iter().map(|&(pop, ls)| calculate_state_tax(pop, ls, tax_rate)).sum();

        let total_population: u64 = states.iter().map(|&(pop, _)| pop).sum();
        let weighted_ls: f64 = states.iter().map(|&(pop, ls)| pop as f64 * ls as f64).sum();
        let average_living_standard = (weighted_ls / total_population as f64) as f32;

        let projected_at_current_rate = calculate_state_tax(total_population, average_living_standard, tax_rate);

        assert!(
            (projected_at_current_rate - real_monthly_income).abs() < 1e-6,
            "人口加重平均を使えば、州別合計と数学的に一致するはず(real={real_monthly_income}, \
             projected={projected_at_current_rate})"
        );
    }

    #[test]
    fn living_standard_and_population_boundary_values_do_not_panic_or_produce_nan() {
        for (pop, ls, rate) in [(0u64, 0.0f32, 0.0f32), (0, 100.0, 1.0), (u64::MAX, 100.0, 1.0)] {
            let v = calculate_state_tax(pop, ls, rate);
            assert!(v.is_finite(), "pop={pop} ls={ls} rate={rate} => {v}");
        }
    }

    /// 既存の`tax_rate > 0.25`生活水準ペナルティ契約(無変更)が、P27-002-C後も
    /// そのまま機能することを確認する(0.25ちょうどは対象外、0.26は対象)。
    #[test]
    fn existing_0_25_living_standard_penalty_boundary_is_unchanged() {
        let mut at_boundary = StateData { living_standard: 50.0, ..StateData::default() };
        let mut just_above = StateData { living_standard: 50.0, ..StateData::default() };
        update_state_welfare_and_unrest(&mut at_boundary, 0.25, true, false);
        update_state_welfare_and_unrest(&mut just_above, 0.26, true, false);
        assert!(
            just_above.living_standard < at_boundary.living_standard,
            "0.25を1刻み超えると既存の税率ペナルティにより0.25ちょうどより低くなるはず \
             (at_boundary={}, just_above={})",
            at_boundary.living_standard,
            just_above.living_standard
        );
    }
}
