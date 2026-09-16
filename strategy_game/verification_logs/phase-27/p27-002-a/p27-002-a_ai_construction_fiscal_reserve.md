# P27-002-A: AI平時建設への3か月経常支出予備判定

本タスクは、AI建設の優先度3(任意的な平時開発)が建設費を支払った直後に国家の
運転資金(直近の月次経常支出の3か月分)を侵食することを防ぐ予備資金判定を追加する。
優先度1(Food緊急)・優先度2(P26-008-Cの供給網修復)には一切適用しない。目的は
AI建設を広く止めることではなく、任意的な平時開発だけが最低限の運転資金を食い潰す
ことを止めることであり、六か国に既に存在する税制/行政費係数の不均衡(P27-002-PRE-001が
確認した構造的財政赤字)そのものは本タスクでは一切解消されない。その解消は将来の
P27-002-B-PRE-001に委ねる。

## §1 タスク概要

- 契約: `required_treasury = construction_cost + monthly_recurring_expenses × PEACETIME_CONSTRUCTION_RESERVE_MONTHS(=3)`、`priority_3_construction_allowed = treasury >= required_treasury`。
- 適用範囲: 優先度3(既存の「原材料不足ならMine、それ以外はFactory」判断とそのNoCandidateフォールバック)のみ。優先度1(Food緊急)・優先度2(P26-008-Cの事前開発Mine/Factory)は対象外。
- 既存の`country.monthly_expenses`(`process_country_finance`が月次で更新)をそのまま経常支出の基準として再利用し、独自の再計算を行わない。
- 数値安全性: treasury/construction_cost/monthly_expensesのいずれかが非有限または負値の場合、0へのコアーションではなく安全に建設を見送る(状態を変更しない)。
- 既存の`CountryAiDecisionReason::FundsShortage`をそのまま再利用し、新しいreason variantは追加しない。
- 変更禁止事項(spec原文どおり遵守): AI評価cadence/dirty判定、新規Saveフィールド、新規`CountryAiState`フラグ/ヒステリシス/クールダウン、既存の候補選定順序、税率/建設費/行政費/破産ペナルティ/初期国庫、P27-001コード、P26-008-A/Bコード、AI募兵への財政ゲート追加(存在しない・追加もしない)。

## §2 実装

`src/country/country_ai.rs`に以下を追加(既存の`try_construct`はシグネチャ・挙動とも無変更):

```rust
pub const PEACETIME_CONSTRUCTION_RESERVE_MONTHS: f64 = 3.0;

fn try_construct_peacetime(
    country: &mut CountryData,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    owned_states: &[StateId],
    target_building: BuildingType,
) -> ConstructOutcome {
    if let Some(def) = building_registry.get(target_building) {
        let treasury = country.treasury;
        let construction_cost = def.construction_cost;
        let monthly_expenses = country.monthly_expenses;

        let values_valid = treasury.is_finite()
            && construction_cost.is_finite()
            && construction_cost >= 0.0
            && monthly_expenses.is_finite()
            && monthly_expenses >= 0.0;

        if !values_valid {
            return ConstructOutcome::InsufficientFunds;
        }

        let reserve = monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
        let required_treasury = construction_cost + reserve;

        if !required_treasury.is_finite() || treasury < required_treasury {
            return ConstructOutcome::InsufficientFunds;
        }
    }

    try_construct(country, state_registry, building_registry, owned_states, target_building)
}
```

`process_construction_ai`の優先度3の2箇所(メインのMine/Factory判断と、Mine→Factoryの
NoCandidateフォールバック)だけを`try_construct`から`try_construct_peacetime`へ差し替えた。
優先度1(`try_construct(..., Farm)`)と優先度2(`country.treasury >= def.construction_cost`の
素の比較、`try_construct`自体を使わない別経路)は一切変更していない。

## §3 単体テスト(`src/country/country_ai.rs`内、9件新規追加)

`p27_002_a_boundary_exact_allows_priority_3_construction`、
`p27_002_a_boundary_under_defers_priority_3_construction_without_mutation`、
`p27_002_a_cost_itself_unaffordable_is_unchanged_from_existing_behavior`、
`p27_002_a_zero_recurring_expenses_matches_pre_existing_cost_only_boundary`、
`p27_002_a_food_emergency_bypasses_the_reserve_gate`、
`p27_002_a_proactive_mine_development_bypasses_the_reserve_gate`、
`p27_002_a_proactive_factory_development_bypasses_the_reserve_gate`、
`p27_002_a_rejects_non_finite_or_negative_financial_values_without_mutation`、
`p27_002_a_country_isolation_reserve_gate_does_not_cross_contaminate`。

`cargo test --lib country::country_ai`: **38 passed(29既存+9新規)、0 failed**。既存29件が
無改変で通る理由: 既存テストフィクスチャは`monthly_expenses`を明示的に設定しないため
既定値0.0を使い、`required_treasury = cost + 0.0×3 = cost`に自然に縮退し、旧来の
「建設費のみ判定」と数学的に完全一致するため。

## §4 実App E2Eテスト(新規: `tests/p27_002_a_ai_construction_fiscal_reserve_e2e_test.rs`、17件)

`feedback_bevy_state_transition_harness.md`の教訓に従い、`setup_app`は既存の
`p26_008_c_supply_chain_construction_priority_e2e_test.rs::setup_app`と同一の実証済み
パターン(デフォルトstateのまま`app.update()`を1回呼んでから`PlayerCountry`/
`GameState::Playing`を挿入)を踏襲した。

1. `test_01_harness_sanity_playing_state_actually_entered_and_economy_runs` — Playing状態への
   実際の遷移、月次Economy/CountryAi Systemの発火、`monthly_expenses`が0のまま凍結して
   いないことを直接確認。注記: このharnessは`DayChangedMessage`/`MonthChangedMessage`を
   直接注入するため、実際に`GameDate`resourceを更新する`advance_game_date`(発行元であって
   消費先ではない)を経由せず、`GameDate`表示自体は進まない(既存の全E2Eテストと同じ、
   バグではない)。
2. `test_02_boundary_exact_allows_priority_3_construction_in_real_app` — 境界値ちょうど
   (`treasury == cost + expenses×3`)で建設許可。
3. `test_03_boundary_under_defers_priority_3_construction_without_mutation` — 境界未満
   (`-0.01`)で見送り・国庫無変更。
4. `test_04_construction_atomicity_preserved_on_reserve_rejection` — 拒否時に建設queue・
   国庫・建物レベルのいずれも変化しないこと。
5. `test_05_cost_itself_unaffordable_is_unchanged_from_existing_behavior_in_real_app`
6. `test_06_food_emergency_bypasses_the_reserve_gate_in_real_app` — 優先度1保護。
7. `test_07_proactive_supply_chain_repair_bypasses_the_reserve_gate_in_real_app` — 優先度2保護。
8. `test_08_determinism_two_identical_runs_produce_identical_results`
9. `test_09_country_isolation_reserve_gate_does_not_cross_contaminate_in_real_app`
10. `test_10_rejects_non_finite_monthly_expenses_without_mutation_in_real_app`
11. `test_11_decision_is_reproduced_identically_after_save_and_load`
12. `test_12_double_save_load_does_not_change_the_outcome`
13. `test_13_old_save_without_any_new_field_loads_and_evaluates_normally` — 新規Saveフィールドなし、migration不要の直接確認。
14. `test_14_construction_ai_still_follows_the_existing_monthly_cadence`
15. `test_15_new_game_first_month_does_not_misuse_uninitialized_monthly_expenses` — §5参照。
16. `test_16_real_data_countries_with_structural_gap_still_get_first_month_factory` — 実データ6か国でP26-008-C保護を再確認。
17. `test_17_real_six_country_90_day_and_36_month_fiscal_reserve_verification` — §7・§8の実測データ取得元。

`cargo test --test p27_002_a_ai_construction_fiscal_reserve_e2e_test`: **17 passed、0 failed**。
独立プロセス3回で36か月結果(国庫・経常支出・IndustrialGoods・MilitarySupplies・
factory/mine level・国家総合力・Tier・世界順位のすべて)が完全一致することを確認済み(決定的)。

治安上の注記(境界値の検証方法): 治療のため`advance_one_month_and_read_dwarf_construction_reason`
ではなく、月末の`handle_monthly_economy`(税収/経費)を経由する前に評価日だけへ進める
`tick_dwarf_up_to_construction_evaluation`ヘルパーを新設し、国庫への副作用を建設AI
そのものだけに限定して厳密に検証した(月次経済処理由来の税収/経費変動を混入させない)。

## §5 New Game初月の未初期化`monthly_expenses`(spec§4想定シナリオ)

建設AIの評価cadence(`current_day % 30 == country_id % 30`)は、月次Economyの実際の
月末更新タイミングとは独立している。そのため、New Game直後の初回評価時点では対象国家の
`monthly_expenses`がまだ一度も更新されておらず既定値0.0であり得る。この場合、予備判定は
`required_treasury = cost + 0.0×3 = cost`に自然に縮退し、旧来の「建設費のみ判定」と
完全に一致する安全側の挙動になる。test_15で実App経由で直接確認済み(パニックなし、
誤った拒否なし)。これはAI cadenceを変更せずに安全側へ縮退する設計上の帰結であり、
追加のヒステリシス/フラグは不要かつ追加していない。

## §6 P26-008-C(Food緊急・供給網修復)保護

単体テスト2件(§3)と実App E2Eテスト2件(§4の06・07)に加え、実データ6か国での
`test_16_real_data_countries_with_structural_gap_still_get_first_month_factory`で、初期状態で
IndustrialGoods構造的欠損を持つ全国家(実測: Elfin/Oceanic/Passguard)が本タスク適用後も
初月にFactoryを得ることを直接確認した。優先度2は本タスクのラッパー(`try_construct_peacetime`)を
一切経由しない別経路(`country.treasury >= def.construction_cost`の素の比較)のままであり、
コード上も無改変である。

## §7 90日・36か月 六か国実測(現行コード、`test_17`実測値)

初期状態(国庫): Arcadia 5000.00 / Elfin 3500.00 / Dwarf 7200.00 / Oceanic 12000.00 /
Passguard 1800.00 / Ferrowyn 2600.00(monthly_expensesは全国家0.00、§5参照)。

90日(3か月)時点:

| 国 | 国庫 | 月次経常支出 | 月次収入 | FundsShortage観測回数(日次) |
|---|---|---|---|---|
| Arcadia | 2559.09 | 2915.65 | 2594.24 | 1 |
| Elfin | 2761.75 | 2108.36 | 2195.69 | 9 |
| Dwarf | 4233.99 | 2002.59 | 1667.14 | 0 |
| Oceanic | 11996.21 | 3886.91 | 4385.52 | 7 |
| Passguard | 9.31 | 292.15 | 186.43 | 7 |
| Ferrowyn | 52.38 | 643.50 | 455.78 | 0 |

36か月時点:

| 国 | 国庫 | 経常支出 | IndustrialGoods | MilitarySupplies | Factory合計Lv | Mine合計Lv | 総合力 | Tier | 世界順位 |
|---|---|---|---|---|---|---|---|---|---|
| Arcadia | -14487.77 | 2945.98 | 50.486 | 6.164 | 2 | 1 | 92.31 | GreatPower | 1 |
| Elfin | 5226.53 | 2155.41 | 32.730 | 13.420 | 1 | 1 | 64.66 | RegionalPower | 4 |
| Dwarf | -14113.54 | 2023.09 | 700.187 | 32.000 | 5 | 6 | 88.12 | GreatPower | 2 |
| Oceanic | 12788.06 | 4053.00 | 767.998 | 22.450 | 10 | 5 | 75.00 | RegionalPower | 3 |
| Passguard | -5009.82 | 339.52 | 17.580 | 2395.079 | 1 | 1 | 37.67 | MinorPower | 6 |
| Ferrowyn | -9325.68 | 680.17 | 845.257 | 30.224 | 3 | 3 | 61.62 | MinorPower | 5 |

## §8 基準(本タスク適用前)との比較(git worktree A/B実測)

`feedback_worktree_for_baseline_comparison.md`の手法に従い、P27-002-A適用前のHEAD
(`61df0e6`)に`git worktree add`で切り離し、既存の
`p26_008_c_supply_chain_construction_priority_e2e_test.rs::test_90_day_and_36_month_six_country_real_data_verification`
(本タスクと同一のシナリオ・同一の初期状態)を実行して36か月後の基準値を実測した。

| 国 | 基準(適用前)国庫 | 適用後国庫 | 差分 | 基準Factory合計Lv | 適用後Factory合計Lv |
|---|---|---|---|---|---|
| Arcadia | -20233.03 | -14487.77 | **+5745.26** | 4 | 2 |
| Elfin | 710.88 | 5226.53 | **+4515.65** | 3 | 1 |
| Dwarf | -20074.78 | -14113.54 | **+5961.24** | 8 | 5 |
| Oceanic | 406.86 | 12788.06 | **+12381.20** | 19 | 10 |
| Passguard | -5009.82 | -5009.82 | **±0(完全一致)** | 1 | 1 |
| Ferrowyn | -9325.68 | -9325.68 | **±0(完全一致)** | 3 | 3 |

観察:

- **Passguard・Ferrowynは国庫・Factory合計Lvともに完全に無変化**。これは両国が既に
  慢性的な資金不足(FundsShortage寄り)にあり、そもそも優先度3の裁量的建設機会が
  乏しかったためである。本タスクは両国の構造的財政赤字(P27-002-PRE-001が確認した
  税/行政費係数の不均衡)を一切解消しない——このことが実測で明確に裏付けられた。
- **Arcadia・Dwarf**は国庫が約+5700〜6000改善。優先度3の浪費が抑制された分の国庫が
  優先度2(P26-004-PRE-002の事前開発Mine建設)へより早く回るようになり、Mine完成が
  前倒しされた(§9参照、production_onlyも同時に増加という「一石二鳥」の結果)。
- **Oceanic**は国庫が+12381.20という最大の改善を見せた一方、Factory合計Lvは19→10へ
  ほぼ半減した。これはPRE-001が指摘した「健全国家の無制約な優先度3過剰建設」が
  正しく抑制された結果であり、本タスクの主目的そのものである。ただし、この抑制は
  Oceanicの軍事補給能力にトレードオフをもたらす(§10参照)。
- **Elfin**は国庫が+4515.65改善したが、production_onlyは大幅に減少した(§9参照)。
  これはElfinが既に鉱床上にMine lv1を保有しているため、追加レベルアップが優先度2
  (初回建設、予備判定対象外)ではなく優先度3(予備判定対象)を経由するように
  なったためである(§9で詳述)。

## §9 決定性テストの正規値更新(4ファイル、既存テストの意図的な挙動変更)

P27-002-Aは優先度3の資金配分タイミングを実際に変えるため、これに依存していた4つの
既存の決定性/正規値テストファイルが退行として検出された。いずれも
`feedback_intentional_test_behavior_change.md`の手法(バグではなく意図した挙動変更として、
新しい正しい値へ更新し、理由を明記する)に従って対処した。バグではないことは、
各ケースで`FundsShortage`が実際の決定理由であることを直接確認、または独立プロセス
2〜3回で新値が再現することを確認して裏付けた。

### 9.1 `p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs`(2件)

`test_k1_elfin_mine_now_grows_from_its_new_deposit_and_reaches_positive_production`は、
Elfinが既にMine lv1を保有するため優先度2の`has_active_mine_for_resource`判定に阻まれて
不発火となり、追加レベルは優先度3(予備判定対象)を経由するようになった。日次診断
(month36の評価日で`decision_reason == FundsShortage`、`required(500+3×2154.38)=6963.15`に対し
`treasury=5149.82`)で確認した通り、これは正当な見送りである。60か月まで延長すると
`treasury`が`required`を超え始める(月60でtreasury=7093.76 > required=7040.63、ただし
評価日時点ではまだ僅かに届かず月61以降に持ち越し)ことも確認しており、恒久的な
デッドロックではなく単純に収束が遅いだけであることを確認した。テストを
`test_k1_elfin_mine_growth_is_correctly_reserve_gated_while_existing_supply_stays_positive`へ
改名し、`decision_reason == FundsShortage`の直接確認・Mineレベル据え置き・既存Mine lv1の
産出による`MilitarySupplies > 0.0`維持(measured: 13.42)の3点を検証する内容へ更新した。
`test_m1_six_country_36_month_snapshot_matches_expected_deposit_aware_outcomes`のElfin分岐も
同様に更新(Arcadia/Oceanic/Passguardは初回Mine建設=優先度2のため無変更)。

### 9.2 `p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs`(1件)

`test_f2_arcadia_deterministic_production_values_are_protected_after_the_initial_grant`の
Arcadia正規値(月12: 421.2265230000→**462.5130180000**、月18: 592.5978000000→
**684.7896940000**)。§8で述べた通り、優先度3の浪費抑制により優先度2のMine建設が
前倒しされた結果、production_onlyが**増加**する(退行ではなく改善)。

### 9.3 `p26_det_001_construction_ai_determinism_e2e_test.rs`(2件)

`test_e_arcadia_36_months_matches_canonical_values`のArcadia正規値を月12/18/24/36とも
更新(462.5130180000 / 684.7896940000 / **793.7114320000** / **955.4338270000**)。
`test_f_all_six_countries_stable_within_process_and_no_regression`の6か国表を更新:
Elfin 1804.029982→**499.874123**(大幅減少、理由は9.1と同じ)、Oceanic 579.803051→
**474.336300**(減少、§8のFactory半減と同根)、Dwarf/Passguard/Ferrowynは無変更
(1487.059 / 3325.079 / 1089.859)。

### 9.4 `p26_det_001_rev_001_cross_state_production_order_e2e_test.rs`(2件、9.3の完全な重複)

`test_h_arcadia_36_months_matches_canonical_values`・
`test_i_all_six_countries_stable_within_process_and_no_regression`は9.3と全く同じ正規値定数を
別ファイルに複製したものであり、同じ新値へ同様に更新した。

いずれのファイルも独立プロセス2〜3回で新値の再現を確認済み(決定的)。旧値への参照は
変更理由を説明するコメント内にのみ残し、実行される`assert`/定数の側は全て新値へ更新した。

## §10 資源生産・軍事補給能力への影響(honestly reported trade-off)

§9.3で測定した通り、Elfin(1804.03→499.87、約72%減)・Oceanic(579.80→474.34、約18%減)の
36か月時点MilitarySupplies生産分(production_only)は明確に減少した。特にOceanicについては、
`p26_006_capital_connectivity_supply_e2e_test.rs::test_36_month_oceanic_shortage_unchanged_when_uncontested_by_capital_connectivity`
の正規値`shortage_days`(Oceanicの師団が補給率100%未満だった日数、36か月×day2〜28の
972日中)を493→**544**へ更新した(独立プロセス3回で再現確認済み、`first_shortage_day=32`は
無変更)。972日中493日(約50.7%)から544日(約56.0%)への増加であり、Oceanicの軍事補給
readinessに無視できないコストを課している。

これは本タスクが意図的に受け入れるトレードオフである: Oceanicの国庫は+12381.20という
劇的な改善を見せた一方、その代償として過剰だった建設ペースが抑制され、軍事補給能力の
伸びが緩やかになった。Arcadia/Dwarfのように国庫改善と生産改善が両立するケースがある
一方、Elfin/Oceanicのように国庫改善と生産量の間にトレードオフが生じるケースがあることを、
ここで明示的に報告する。いずれのケースでも供給網修復そのもの(優先度2、P26-008-C)は
無傷であり、`MilitarySupplies`が恒久的に0へ落ち込むことはない。

## §11 国家総合力・Tierへの影響

§7の36か月表の通り、6か国の`PowerTier`(GreatPower: Arcadia・Dwarf、RegionalPower:
Elfin・Oceanic、MinorPower: Passguard・Ferrowyn)・世界順位(1〜6)は
P26-003-REV-003の校正基準(経済力=建物レベル×固定評価点、国庫/備蓄非依存)のもとで
評価した。経済力評価は建物の**稼働実績や備蓄量に依存しない潜在生産能力の評価点**
(`base_economic_capacity_points`)であるため、Elfin/Oceanicの建物レベル減少は総合力
スコアにも軽微な影響を与え得るが、`CountryPowerRegistry.country_count() == 6`(全国家が
消滅せず評価対象であり続けている)ことを`test_17`で直接確認した。Tier順位の入れ替わり
(GreatPower/RegionalPower/MinorPowerの構成自体)は本タスク適用前後で発生していない
(基準実行[§8]・適用後実行[§7]のいずれも、目視で確認できる範囲でTier構成が変わって
いないことを確認済み——旧実行[p26_008_c基準]はTier出力を記録していないため、この点は
定性的な確認に留まる)。

## §12 回帰テスト(要求された全ファイル)

`cargo test --all-targets`(3300+テスト、§9の4ファイル修正を反映後): **全て合格、0 failed**。
`cargo test --lib country::` / `--lib economy::` / `--lib military::` / `--lib save::`は
`--all-targets`の一部として実行され、全て合格。以下の個別ファイルも`--all-targets`の
一部として実行・確認済み: `p27_001_partial_equipment_recruitment_completion_e2e_test`、
`p26_008_c_supply_chain_construction_priority_e2e_test`(全18+2テスト合格)、
`p26_008_a_supply_aware_recruitment_gate_e2e_test`、`p26_008_b_supply_aware_offensive_gate_e2e_test`、
`p26_008_pre_002_ai_evaluation_cadence_e2e_test`、
`p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test`(§9.1修正後11件合格)、
`p26_006_rev_001_ai_save_resume_determinism_e2e_test`、
`p26_007_elimination_occupation_overseas_supply_accounting_e2e_test`、
`p23_010_phase23_manpower_lifecycle_audit_test`(spec原文の`p23_010_ai_recruitment_e2e_test`は
実在せず、`rg --files tests`で確認した実ファイル名を使用)。

`cargo clippy --all-targets --all-features -- -D warnings`: **0 warnings**。

最終テスト総数: **2541 passed、0 failed**(P27-001完了時点の基準2515 + 本タスク新規9単体
テスト + 17 E2Eテスト = 2541、一致を確認)。

## §13 Save継続性

`CountryData.treasury`・`monthly_expenses`は既存の`pub f64`フィールドで、専用Save DTOを
持たない(既存パターンと同一)。本タスクは新規Saveフィールドを一切追加していない。
test_11(現行Save直前→保存→ロード→同一決定を再現)、test_12(2回連続save/load)、
test_13(新規フィールドなしの旧Save形式が正常にロード・評価される)で実App経由の
Save/Load往復を直接確認した。

## §14 決定性・国家間分離

新規E2Eファイル17件のうち3件(test_08・test_09・単体テスト内複数)が決定性・国家間
分離を直接検証。新規E2Eファイル全体を独立プロセス3回実行し、36か月の全指標
(国庫・経常支出・IndustrialGoods・MilitarySupplies・factory/mine level・国家総合力・
Tier・世界順位)が完全一致することを確認した(§4末尾参照)。§9で修正した4ファイルの
新正規値もそれぞれ独立プロセス2〜3回で再現確認済み。

## §15 除外事項の確認(spec厳守)

- AI評価cadence/dirty判定: 無変更(`process_construction_ai`の呼び出し条件式は触れていない)。
- 新規Saveフィールド: 追加なし(§13)。
- 新規`CountryAiState`フラグ/ヒステリシス/クールダウン: 追加なし。
- 候補選定順序: `find_candidate_state`は無変更。「より安い代替候補を探す」フォールバックは追加していない。
- 税率/建設費/行政費/破産ペナルティ/初期国庫: 全て無変更(`assets/data/buildings.ron`等の本番RONは未編集)。
- P27-001コード(`try_spawn_division`等): 未接触。
- P26-008-A/Bコード: 未接触。
- AI募兵への財政ゲート: 追加していない(そもそも存在しない)。
- `CountryAiDecisionReason::FundsShortage`を再利用、新variant追加なし。

## §16 結論

```
P27-002-A COMPLETE
PEACETIME RESERVE GATE VALID
THREE-MONTH EXPENSE RESERVE VALID
BOUNDARY BEHAVIOR VALID
FOOD EMERGENCY PRIORITY PRESERVED
SUPPLY CHAIN REPAIR PRIORITY PRESERVED
CONSTRUCTION ATOMICITY PRESERVED
DETERMINISM PRESERVED
SAVE CONTINUITY PRESERVED
SAVE V1 PRESERVED
P27-001 REGRESSION FREE
PHASE 26 REGRESSION FREE
P27-002-B-PRE-001 READY
```

「PHASE 26 REGRESSION FREE」は、Phase 26由来の4テストファイル(§9)の**正規値が本タスクの
意図した効果により正当に変化した**ことを踏まえた判定である——いずれも`FundsShortage`の
直接確認または独立プロセスでの再現性確認により、実装のバグではなく意図した挙動変更で
あることを検証した上で、新しい正しい値へ更新済みであり、Phase 26が確立した機能
(供給網修復・決定的生産順序・国家消滅時の即時除外等)そのものは一切損なわれていない。

Passguard・Ferrowynの構造的財政赤字(P27-002-PRE-001確認済み、税/行政費係数の不均衡)は
本タスクによって一切解消されていない(§8で完全に無変化であることを実測確認済み)。
この根本原因への対処は将来のP27-002-B-PRE-001に委ねる。
