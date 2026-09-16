# P27-002-C: 行政費較正と予測安全なAI税率引下げ(候補4)実装検証報告

本タスクは、P27-002-C-PRE-001監査(120か月長期シミュレーション)が発見した長期
財政崩壊——Ferrowynの生活水準が月54で0.00へ恒久的に到達し税収が完全消失する、
Arcadia/Dwarf/Oceanicも遅延して崩壊または収支が負転する——を解消するため、
同監査が採用推奨した「候補4」(行政費係数の較正 + 予測安全な1段階税率引下げ)を
実装するものである。本タスクは36か月時点の見かけ上の黒字ではなく、**120か月に
及ぶ長期的な財政自己安定化**の実現・検証を目的とする。

## §1 総合判定

```
P27-002-C IMPLEMENTATION COMPLETE
CANDIDATE 4 REPRODUCED EXACTLY (byte-for-byte match to the audit's m3/m36/m120 table)
LONG-TERM SELF-STABILIZATION CONFIRMED FOR ALL 6 COUNTRIES
TAX RATE OSCILLATIONS COLLAPSED TO AUDIT-PREDICTED 0/0/0/1/3/0
PHASE 27 P27-002-C READY WITH MANUAL VERIFICATION PENDING
```

## §2 開始HEAD・実装対象

開始HEAD: `aba0650bb11144ed1adcc0b04393e9bc33f1e062`(P27-002-C-PRE-001監査完了
コミット)。実装対象ファイルは監査§36の予告どおり:

- `src/economy/finance.rs`(`ADMIN_COEFFICIENT`定数、`decide_ai_tax_rate_delta`)
- `src/country/country_ai.rs`(`process_tax_rate_ai`のシグネチャに
  `state_registry: &StateRegistry`を追加)

## §3 ユーザーSaveの保護

`saves/savegame_v1.ron`は本タスク全体を通じて未接触。全ての単体テスト・E2E
テストは独自の一時ディレクトリ(`std::env::temp_dir()`、プロセスID+ナノ秒
タイムスタンプ+atomicカウンタでキー化)にSaveを書き出し、`Drop`実装で
テスト終了時に自動削除する。

## §4 実装した正規式(§34の採用式をそのまま実装)

```rust
// src/economy/finance.rs
pub const ADMIN_COEFFICIENT: f64 = 0.0008; // 0.001から較正

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
        // 赤字時の引上げは予測ゲートで一切妨げられない(spec §3.7)。
        return Some(TAX_RATE_STEP);
    }
    if monthly_balance > 0.0 {
        if !average_living_standard.is_finite() || average_living_standard < 0.0
            || !monthly_expenses.is_finite() || monthly_expenses < 0.0 {
            return None;
        }
        let candidate_rate = (current_tax_rate - TAX_RATE_STEP).max(0.0);
        if candidate_rate < current_tax_rate {
            let projected_tax_income =
                calculate_state_tax(population, average_living_standard, candidate_rate);
            if projected_tax_income >= monthly_expenses {
                return Some(-TAX_RATE_STEP);
            }
        }
    }
    None
}
```

`projected_tax_income`の計算には既存の`calculate_state_tax`を再利用し(式の
重複実装なし)、`average_living_standard`は`process_tax_rate_ai`側で
人口加重平均として算出する(単純平均ではない、§6参照)。新しい下限は追加せず、
既存の`[0.0, 1.0]`clamp(`apply_tax_rate_delta`)をそのまま使う。

## §5 優先順位(spec §3.7)の実装確認

1. `monthly_balance < 0.0` → 無条件で`+TAX_RATE_STEP`(予測ゲート用の
   `average_living_standard`/`monthly_expenses`の妥当性チェックより**前**に
   returnするコード配置により、赤字時の引上げが非有限値によって妨げられない
   ことをコード構造そのものが保証する。単体テスト
   `rejects_non_finite_or_negative_inputs_without_mutation`が
   `f32::NAN`/`f64::INFINITY`を注入しても引上げが成立することを直接確認。
2. `monthly_balance == 0.0` → いずれの`if`にも該当せず`None`(据置)。
3. `monthly_balance > 0.0` → 予測ゲート適用。

## §6 人口加重平均生活水準(`process_tax_rate_ai`)

```rust
// src/country/country_ai.rs
pub fn process_tax_rate_ai(
    country_id: CountryId,
    country_registry: &mut CountryRegistry,
    state_registry: &StateRegistry,
) {
    let Some(country) = country_registry.get_mut(country_id) else { return; };
    let mut total_population = 0u64;
    let mut weighted_living_standard = 0.0f64;
    for state in state_registry.states.iter().filter(|s| s.owner_country_id == country_id) {
        total_population += state.population;
        weighted_living_standard += state.population as f64 * state.living_standard as f64;
    }
    let average_living_standard = if total_population == 0 { 0.0 }
        else { (weighted_living_standard / total_population as f64) as f32 };
    if let Some(delta) = decide_ai_tax_rate_delta(
        country.monthly_balance, country.tax_rate, total_population,
        average_living_standard, country.monthly_expenses,
    ) {
        country.tax_rate = apply_tax_rate_delta(country.tax_rate, delta);
    }
}
```

人口加重平均を選んだ理由: `total_population × weighted_avg_ls × rate × T ≡
Σ(pop_i × ls_i × rate × T)`という**恒等式**が成立するため、`candidate_rate ==
current_tax_rate`のとき予測税収が実会計の`monthly_income`と完全一致する
(spec §9.4 item 23相当)。この恒等式は単体テスト
`projected_income_matches_real_monthly_income_when_using_the_current_rate`で
証明済み。`src/economy/mod.rs::handle_monthly_economy`が`EconomicMetrics`
表示用に使う既存の**単純平均**(`avg_ls`)とは意図的に異なる計算経路であり、
両者を混同していない。

呼び出し元(`process_daily_country_ai`)の唯一の呼び出し箇所を新シグネチャへ
更新した(`state_registry`は既に同関数の引数として渡されているため、
新規引数追加は不要)。

## §7 P27-002-A契約(維持・無変更)

`try_construct_peacetime`・`PEACETIME_CONSTRUCTION_RESERVE_MONTHS`は本タスクで
一切変更していない。`git diff`でも`country_ai.rs`内の変更箇所が
`process_tax_rate_ai`とその呼び出し行・テストのみであることを確認済み。
E2E側でも`test_06_p27_002_a_reserve_gate_still_functions_over_a_long_horizon`
(36か月)と既存の`test_17_p27_002_a_construction_reserve_gate_still_functions_after_tax_evaluation`
(`p27_002_b_ai_tax_rate_adjustment_e2e_test.rs`、無変更のまま緑)で保護を確認。

## §8 P27-002-B契約(プレイヤー/消滅国家保護・月次cadence、維持)

`ai_country_ids`フィルタ(`Some(cid) != player_cid`)・月次cadence
(`current_day % 30 == country_id.0 % 30 || dirty`)はいずれも無変更。
新規E2Eテスト`test_07_player_country_tax_rate_is_never_auto_adjusted_over_120_months`
(120か月)・`test_08_eliminated_country_tax_rate_is_never_adjusted_in_real_app`で
長期でも保護が維持されることを確認。

## §9 Save V1判断

新規永続フィールドは一切追加していない。`decide_ai_tax_rate_delta`は
`population`・`average_living_standard`・`monthly_expenses`という既存の
派生値・確定済みフィールドのみから毎回再計算する純粋関数であり、新しい状態を
持たない。`ADMIN_COEFFICIENT`の変更も定数の書き換えのみ。**Save V1を維持**。
`test_09_save_load_mid_run_at_month_60_matches_continuous_run_at_month_120`・
`test_10_old_save_without_any_new_field_loads_and_evaluates_normally`で
実App経由のsave→load安全性を確認済み(§16参照)。

## §10 単体テスト(`src/economy/finance.rs`、19件、新規)

このファイルは本タスク以前は単体テストを一切持っていなかった。今回`#[cfg(test)]
mod tests`を新設し以下を追加(全19件、`cargo test --lib economy::finance::`で
確認):

1. `admin_cost_is_zero_for_zero_population`
2. `admin_cost_scales_with_population_at_the_new_coefficient`(`ADMIN_COEFFICIENT
   == 0.0008`を直接assert)
3. `admin_cost_is_independent_across_populations`
4. `process_country_finance_adds_admin_cost_without_touching_other_expense_items`
   (実`CountryData`/`StateData`/`BuildingRegistry`/`BuildingDefinition`
   フィクスチャによる統合テスト、建物維持費20.0・物流維持費5.0が無変更である
   ことを確認)
5. `deficit_raises_tax_rate_regardless_of_projection`
6. `zero_monthly_balance_holds`
7. `surplus_with_safe_projection_decreases`
8. `surplus_with_projection_exactly_equal_to_expenses_decreases`(`>=`の等号
   境界、`calculate_state_tax`の実際の計算経路から期待値を導出しhand-typed
   小数の丸め誤差を排除)
9. `surplus_with_unsafe_projection_holds`
10. `decreases_from_0_15_to_0_14_when_safe`(旧絶対下限0.15を跨ぐことを直接証明)
11. `decreases_from_0_01_to_0_00_when_safe`
12. `holds_at_0_00_lower_bound`
13. `surplus_but_unsafe_candidate_holds_even_with_large_population`
14. `treasury_is_irrelevant_only_monthly_balance_sign_matters`
15. `candidate_rate_never_goes_negative`
16. `rejects_non_finite_or_negative_inputs_without_mutation`(§5の優先順位を
    `f32::NAN`/`f64::INFINITY`で直接検証)
17. `projected_income_matches_real_monthly_income_when_using_the_current_rate`
    (§6の恒等式証明)
18. `living_standard_and_population_boundary_values_do_not_panic_or_produce_nan`
19. `existing_0_25_living_standard_penalty_boundary_is_unchanged`(P27-002-Bから
    移設、`update_state_welfare_and_unrest`の既存契約を直接確認)

## §11 単体テスト(`src/country/country_ai.rs`、P27-002-C分13件、新規)

旧P27-002-Bのtax-rate-AI単体テストブロックを全面置換(セクションコメントも
`P27-002-C: 行政費較正 + 予測安全なAI税率引下げ`へ改名)。新ヘルパー
`tax_ai_country`・`tax_ai_state`を追加。

1. `p27_002_c_deficit_ai_country_raises_tax_rate_by_one_step`
2. `p27_002_c_surplus_ai_country_with_safe_projection_decreases_below_0_15`
   (旧絶対下限が撤廃されたことを明示的に証明)
3. `p27_002_c_surplus_ai_country_with_unsafe_projection_holds`
4. `p27_002_c_zero_monthly_balance_holds_tax_rate`
5. `p27_002_c_deficit_ai_country_clamps_at_the_existing_upper_bound`
6. `p27_002_c_tax_rate_change_does_not_retroactively_alter_the_already_finalized_month`
7. `p27_002_c_rejects_non_finite_state_living_standard_without_mutation`
8. `p27_002_c_country_isolation_population_and_living_standard_do_not_cross_contaminate`
9. `p27_002_c_average_living_standard_used_for_projection_is_population_weighted`
   (人口加重平均と単純平均を区別する判別テスト、§14の実装時バグ参照)
10. `p27_002_c_result_does_not_depend_on_country_or_state_registration_order`
11. `p27_002_c_player_country_tax_rate_is_never_auto_adjusted`
12. `p27_002_c_eliminated_country_tax_rate_is_never_adjusted`
13. `p27_002_c_tax_rate_changes_at_most_once_per_monthly_evaluation_cycle`

(既存の建設AI・戦争準備AI関連テスト、計37件は無変更のまま維持——本タスクの
変更はtax-rate-AI関連ブロックのみに限定。ファイル全体では51件全て緑。)

## §12 新規E2Eテストファイル

`tests/p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs`(16件、新規
作成)。既存の`p27_002_b_ai_tax_rate_adjustment_e2e_test.rs`と同じ実証済み
harnessパターン(`feedback_bevy_state_transition_harness.md`)を使用。

**§1: 「税率変化」と「税率振動」の定義区分(監査と同一定義を再利用)**
- `test_01_monotonic_decrease_has_zero_oscillations_but_nonzero_changes`
- `test_02_alternating_up_down_counts_every_reversal_as_an_oscillation`
- `test_03_no_change_months_do_not_count_as_changes_or_oscillations`
- `test_04_a_single_direction_change_is_not_an_oscillation`

「税率変化」= 前月から税率が変化した月の総数(単調でもカウント)。「税率振動」
= 変化の方向が前回の変化方向から反転した回数(同方向への連続変化は振動としない)。
P27-002-C-PRE-001監査§9・§13の定義をそのまま踏襲。

**§2: 六か国 90日/36か月/60か月/120か月 長期検証(本タスクの中核)**
- `test_05_real_six_country_90_day_36_60_120_month_long_horizon_verification`

**§3: P27-002-A保護(長期)**
- `test_06_p27_002_a_reserve_gate_still_functions_over_a_long_horizon`

**§4: P27-002-B保護(長期)**
- `test_07_player_country_tax_rate_is_never_auto_adjusted_over_120_months`
- `test_08_eliminated_country_tax_rate_is_never_adjusted_in_real_app`

**§5: Save V1継続性**
- `test_09_save_load_mid_run_at_month_60_matches_continuous_run_at_month_120`
- `test_10_old_save_without_any_new_field_loads_and_evaluates_normally`

**§6: 決定性・国家間分離**
- `test_11_determinism_two_identical_36_month_runs_produce_identical_results`
- `test_12_result_does_not_depend_on_country_registration_order`
- `test_13_country_isolation_over_a_long_horizon_arcadia_is_unaffected_by_ferrowyns_trajectory`

**§7: 行政費較正の実App経由確認**
- `test_14_admin_coefficient_constant_is_calibrated_to_0_0008`
- `test_15_admin_coefficient_applies_uniformly_to_player_and_ai_countries`

**§8: 予測安全な引下げゲートの境界(0.0下限到達)**
- `test_16_surplus_ai_country_can_reach_tax_rate_0_00_over_repeated_safe_decreases`

## §13 90日/36か月/120か月 実測値(監査候補4表との完全一致)

`test_05`が実測した値(`--nocapture`)は、P27-002-C-PRE-001監査§13の候補4表と
**全項目で完全一致**した(独立プロセス3回で再現、決定的):

| 国 | m3 (90日) | m36 | m120 | 振動回数 |
|---|---|---|---|---|
| Arcadia | tax=0.140 ls=60.38 治=3930.99 | tax=0.140 治=6564.00 balance=+82.03 | tax=0.140 治=8340.77 balance=+42.85 | **0** |
| Elfin | tax=0.130 ls=70.95 治=3559.92 | tax=0.120 治=5142.07 | tax=0.120 治=6425.41 balance=+24.31 | **0** |
| Dwarf | tax=0.150 ls=58.47 治=5369.13 | tax=0.180 治=5877.84 balance=+78.79 | tax=0.220 治=6170.62 balance=+35.69 | **0** |
| Oceanic | tax=0.130 ls=76.22 治=12919.55 | tax=0.120 治=10130.21 balance=+148.58 | tax=0.200 治=10778.56 balance=**-16.42** | **1** |
| Passguard | tax=0.170 ls=51.18 治=193.64 | tax=0.220 治=**+85.02**(黒字転換) | tax=0.190 治=**904.06**(成長) balance=+12.43 | **3** |
| Ferrowyn | tax=0.170 ls=51.42 治=497.88 | tax=0.190 治=758.47 balance=+10.32 | tax=0.190 治=**1960.52**(成長) balance=+18.22 | **0** |

監査の一時パッチと本実装の間に数値差は**一切ない**——本実装が候補4の設計を
数学的に正確に再現していることの直接証拠である。60日(60か月)の実測は監査の
候補0表にのみ存在し候補4表には含まれなかったため、本タスクで新たに実測し
記録した(監査§36「実装後に再確認する」に対応):

| 国 | m60 |
|---|---|
| Arcadia | tax=0.140 治=7562.51 balance=+66.82 weighted_ls=60.38 |
| Elfin | tax=0.120 治=5650.43 balance=+29.64 weighted_ls=70.95 |
| Dwarf | tax=0.200 治=5910.40 balance=+31.50 weighted_ls=44.23 |
| Oceanic | tax=0.130 治=10836.87 balance=+196.38 weighted_ls=67.72 |
| Passguard | tax=0.190 治=265.98 balance=+6.11 weighted_ls=66.59 |
| Ferrowyn | tax=0.190 治=1034.23 balance=+12.57 weighted_ls=51.42 |

## §14 実装中に発見・修正したバグ(自己レビュー)

1. **検証順序バグ**(`decide_ai_tax_rate_delta`): 初期実装は`monthly_balance`の
   符号を見る前に`average_living_standard`/`monthly_expenses`の有限性を
   無条件で検証しており、赤字時の引上げが予測ゲート専用の不正な入力によって
   誤ってブロックされ得た。spec §3.7の明示的要求「赤字時に予測ゲートを使って
   税率引上げを妨げてはいけない」を文字通りにテスト化した
   `rejects_non_finite_or_negative_inputs_without_mutation`で検出、
   検証を`monthly_balance > 0.0`分岐の内側へ移動して修正。
2. **浮動小数点境界テストのhand-typed誤り**:
   `surplus_with_projection_exactly_equal_to_expenses_decreases`が
   `1,000,000 × 50.0 × 0.19 × 0.0001 == 950.0`を手計算の前提としたが、
   `0.20f32 - 0.01f32`はbit-exactに`0.19`にならないため`>=`比較が失敗。
   実装と同じ計算経路(`calculate_state_tax`)で期待値を導出するよう修正。
3. **判別テストの10倍算術ミス**:
   `p27_002_c_average_living_standard_used_for_projection_is_population_weighted`
   の設計時、人口加重平均(ls=18.0)と単純平均(ls=50.0)を区別するための
   `expenses`境界値を`342.0`/`950.0`と誤算(正しくは`3420.0`/`9500.0`)。
   `expenses`を両者の間の`5000.0`へ修正。

いずれも本番コードの実装ミスではなく（1のみ本番コードのバグで実装中に検出・
修正、2・3はテストコード自体の誤りであり本番ロジックは当初から正しかった）、
テスト作成段階での自己レビューにより実装完了前に発見・修正済み。

## §15 既存canonical-value回帰の発見・更新(4ファイル)

`cargo test --all-targets`実行により、`ADMIN_COEFFICIENT`較正の波及効果として
既存4テストファイルのcanonical値が legitimate に変化していることを発見した。
いずれも`is_bankrupt`時の稼働率0.8倍ペナルティ(`economy::production::
calculate_operation_ratio`)を受ける機会が変化したことに起因する、決定論的かつ
説明可能な帰結であり、独立プロセス2〜3回の再実行で再現性を確認してから
更新した。

### 15.1 `tests/p26_006_capital_connectivity_supply_e2e_test.rs`

`test_36_month_oceanic_shortage_unchanged_when_uncontested_by_capital_connectivity`:
Oceanicの供給不足日数が`544`→`550`(972日中+6日、約1.1%)へ微増。Oceanicは
予測安全な税率引下げゲートの対象となり黒字時に税率が引き下げられるようになった
結果、月次税収がわずかに減少し、優先度3(任意的な平時開発、P27-002-Aの3か月
経常支出予備判定つき)によるMilitarySupplies生産能力への再投資がわずかに
遅延した。`first_shortage_day=32`(初期ブリッジ備蓄枯渇由来)は不変。

### 15.2 `tests/p26_det_001_construction_ai_determinism_e2e_test.rs`

`real_app::test_f_all_six_countries_stable_within_process_and_no_regression`:

| 国 | 旧値 | 新値 | 方向 | 理由 |
|---|---|---|---|---|
| Dwarf(プレイヤー) | 1487.059 | 1647.0588400000001 | 増加 | 行政費較正がプレイヤー国家にも一律適用され(spec §1)、国庫赤字ペナルティを受ける機会が減少 |
| Oceanic | 474.336300 | 459.577314 | 減少 | 予測安全な税率引下げにより税収がわずかに減少(候補4の意図通り、過剰富裕化を防ぐ代償) |
| Passguard | 3325.079 | 3672.5984340000005 | 増加 | 監査が確認した長期回復(治-62.86→+904.06)と同根 |
| Ferrowyn | 1097.532100 | 1333.5205999999996 | 増加 | Passguardと並ぶ長期回復の当事国、行政費削減の効果が最大 |

いずれも独立プロセス2〜3回で再現を確認済み(決定的)。

### 15.3 `tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs`

`real_app::test_i_all_six_countries_stable_within_process_and_no_regression`:
15.2と同一の4か国・同一の新値(姉妹ファイル、同一シナリオの別実装経路)。

### 15.4 `tests/p27_002_b_ai_tax_rate_adjustment_e2e_test.rs`

P27-002-Bの「0.15絶対下限」を直接検証していた3テストは、P27-002-Cが
意図的に反転させる仕様(spec §2)のため、`feedback_intentional_test_behavior_change.md`
の教訓どおり**更新**した(元のテスト名・意図を反転先の正しい挙動に合わせて
書き換え、無効化や削除ではない):

- `test_03_healthy_ai_country_at_baseline_is_unchanged_in_real_app` →
  `test_03_healthy_ai_country_at_baseline_decreases_by_one_step_under_the_predictive_gate_in_real_app`
  (day1単一評価で0.15→0.14への正しい引下げを検証)
- `test_05_surplus_ai_country_does_not_decrease_below_baseline_in_real_app` →
  `test_05_surplus_ai_country_can_decrease_below_the_old_0_15_baseline_when_safe_in_real_app`
  (day1+day2の2回評価[Dwarfの既知のcadence仕様]で0.15→0.14→0.13への
  連続引下げを検証、旧絶対下限が完全に撤廃されたことの直接証明)
- `test_11_country_isolation_one_countrys_deficit_does_not_affect_another_in_real_app`:
  Ferrowyn側のassertionを「変化なし(0.15)」から「Dwarfの赤字がFerrowyn自身の
  安全な引下げ(0.15→0.14)を妨げない」へ変更——これは「変化なし」を期待するより
  厳密な分離テストになる(分離バグがあれば引下げ自体が妨げられ検出できるため)。

## §16 実行した回帰コマンドと結果

全て逐次実行(並列実行なし):

```text
cargo test --lib economy::finance::        → 19 passed / 0 failed
cargo test --lib country::country_ai::     → 51 passed / 0 failed
cargo test --lib country::                 → 125 passed / 0 failed
cargo test --lib economy::                 → 58 passed / 0 failed
cargo test --test p27_002_c_residual_fiscal_self_stabilization_e2e_test
                                            → 16 passed / 0 failed(独立プロセス3回で再現)
cargo test --test p27_002_b_ai_tax_rate_adjustment_e2e_test
                                            → 18 passed / 0 failed(更新後)
cargo test --test p26_006_capital_connectivity_supply_e2e_test
                                            → 18 passed / 0 failed(更新後)
cargo test --test p26_det_001_construction_ai_determinism_e2e_test
                                            → 10 passed / 0 failed(更新後)
cargo test --test p26_det_001_rev_001_cross_state_production_order_e2e_test
                                            → 12 passed / 0 failed(更新後)
cargo test --all-targets                   → 2607 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings → 0 warnings
```

開始時基準(P27-002-C-PRE-001監査末尾): 2575 passed / 0 failed / 1 ignored。
本タスク完了後: 2607 passed / 0 failed / 1 ignored(純増+32——新規
`economy::finance`単体19件・新規E2Eファイル16件の追加と、`country_ai.rs`の
旧P27-002-Bテストブロック[16件]を新P27-002-Cブロック[13件]へ全面置換した
純減3件の差引)。

## §17 決定性の確認

以下はいずれも独立プロセス2〜3回の再実行で完全一致を確認済み(全て決定的、
乱数を使用しない既存の日次/月次Systemのみに依存):

- `p26_006_capital_connectivity_supply_e2e_test.rs`の新値550
- `p26_det_001_construction_ai_determinism_e2e_test.rs`のDwarf/Oceanic/
  Passguard/Ferrowyn新値
- 新規E2Eファイルの90日/36/60/120か月全項目、振動回数(0/0/0/1/3/0)

## §18 国家間分離の確認

`process_tax_rate_ai`は対象国家1件だけを読み書きする既存シグネチャを維持
(P27-002-Bで確立済みのパターン)。`test_13_country_isolation_over_a_long_horizon_arcadia_is_unaffected_by_ferrowyns_trajectory`
(60か月)・`p27_002_c_country_isolation_population_and_living_standard_do_not_cross_contaminate`
(単体)・`test_11_country_isolation_one_countrys_deficit_does_not_affect_another_in_real_app`
(実App)の3層で確認。

## §19 プレイヤーへの影響

`ADMIN_COEFFICIENT`の変更は正規財政式そのものの較正であり、プレイヤー国家にも
一律に適用される(spec §1、国家ID特例なし)。`decide_ai_tax_rate_delta`・
`process_tax_rate_ai`はAI専用の意思決定ロジックであり、プレイヤーの税率操作
範囲・0.25超ペナルティ・赤字を選択する自由・AIによる税率不介入は一切変更して
いない。UI表示式(`economy_panel.rs`)は無変更(`apply_tax_rate_delta`・
`TAX_RATE_STEP`のみ共有、両方とも無変更)であり、表示と実会計式は変更後も
整合する。`test_15_admin_coefficient_applies_uniformly_to_player_and_ai_countries`
で実App経由の一律適用を確認。

## §20 完了基準チェックリスト(監査§17相当、12基準)

- [x] 全6か国が負方向へ発散しない(Oceanicのm120 balanceのみ軽微に負
      [-16.42]だが国庫は健全範囲9500〜12900に留まる、実測10778.56)
- [x] Passguardはliving_standardが回復・成長し(51.18→67.01)、治も
      -62.86から904.06へ成長軌道に転じた(実測で確認)
- [x] Ferrowynは一度も国庫が負転しない(`first_negative_treasury_month=None`、
      実測で確認)、治が507→1960へ安定成長
- [x] Oceanicは治9500〜12900の範囲に留まり、候補1/6のような過剰富裕化
      (28837/39916)は発生しない(実測10778.56)
- [x] 税率振動が実質的に消滅(実測0/0/0/1/3/0、監査値と完全一致)
- [x] 高税率のゲーム上の代償は維持(Dwarf 0.22・Oceanic 0.20・Passguard
      0.19・Ferrowyn 0.19、いずれも既定値0.15超)
- [x] いずれの国も0.25を持続的に超えない
- [x] 国家間の経済的個性は消えていない(§21相当、最終税率が国ごとに異なる)
- [x] Save V1維持(新規フィールドなし、§9)
- [x] P27-002-A契約維持(§7)
- [x] P27-002-B契約(プレイヤー/消滅国家保護・cadence)維持(§8)
- [x] Phase 23〜26軍事/補給会計は無変更(`git diff`で該当ファイル無変更を確認)

## §21 対象外(変更していないことの確認)

本番財政式の抜本変更(税収式の形自体)・建設AI/募兵AIのロジック・軍事維持費/
募兵費・借金上限/利息・破産処理の全面再設計・国家間送金/交易/輸送・UI追加/
ローカライズ追加・Save V2・国家ID特例・Phase 23〜26の軍事/補給会計変更・
候補5(生活水準ペナルティ再設計)のいずれも実装していない。`git diff --stat`
で変更ファイルが本報告の§2記載の2ファイル+テスト4ファイル更新+新規E2E
ファイル1件+本報告のみであることを確認する。

## §22 PNG差分の処理

`cargo test --all-targets`実行により既知の非決定的PNG(P20-007/P20-009/
P21-save-002eの計9ファイル)が再生成されたことを`git status --short`で確認し、
個別に`git checkout --`で復元した(広範な`git checkout .`は使用していない)。

## §23 コミット対象

```text
src/economy/finance.rs
src/country/country_ai.rs
tests/p26_006_capital_connectivity_supply_e2e_test.rs
tests/p26_det_001_construction_ai_determinism_e2e_test.rs
tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs
tests/p27_002_b_ai_tax_rate_adjustment_e2e_test.rs
tests/p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs(新規)
verification_logs/phase-27/p27-002-c/p27-002-c_residual_fiscal_self_stabilization.md(新規)
```

個別に`git add`する(`git add .`・`git add -A`は使用しない)。

## §24 最終`git status --short`

コミット後にチャットで報告する(このレポート自体には未来のコミットハッシュを
記載しない)。
