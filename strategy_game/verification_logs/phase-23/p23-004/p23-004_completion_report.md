# P23-004 完了報告: 経済用労働力比率の共通化と動員影響の回帰固定

## 1. 総合判定

**COMPLETE**

新しい動員ペナルティやゲームバランス変更は導入していない。既存の`E / P`(雇用者数 ÷
潜在労働力)という経済用労働力比率をそのまま維持し、3箇所の重複計算を1つの共通純粋
関数へ集約した上で、余剰労働力での吸収・需要割れでの悪化・動員解除での回復・他国
非波及・save/load後の再現性を自動テストで固定した。UI変更は無いため手動GUI確認待ちは
付けない。

## 2. 基準状態

* 開始時HEAD: `1556ef4f3f056402d40283c3cd6943b2d0a8c364`(P23-003完了、報告書ハッシュ修正コミット)
* 実装コミットの完全なハッシュ: `697b0ece6ab46a459db66e39261785eb69f6d136`
  (`refactor: centralize economic workforce ratio`)
* 報告書コミットの完全なハッシュ: 本報告コミット後に確定するため、このコミット自身の
  ハッシュはここへ自己参照的に記載しない(§9参照、[[feedback_report_hash_before_commit]])
* 最終HEAD: 報告書コミット後に確定(§9参照)
* ユーザーの既存未コミット変更(`saves/savegame_v1.ron`)は本タスク開始前から存在する
  実プレイデータであり、一切参照・変更・ステージ・コミットしていない。

## 3. 実装内容

### 3.1 共通関数の配置・名称・数式

`population::update::calculate_economic_workforce_ratio(employed_workforce: u64,
potential_workforce: u64) -> f32`(新規)。

```rust
pub fn calculate_economic_workforce_ratio(
    employed_workforce: u64,
    potential_workforce: u64,
) -> f32 {
    if potential_workforce == 0 {
        return 1.0;
    }
    employed_workforce as f32 / potential_workforce as f32
}
```

`finance.rs`・`production.rs`が元々使っていた`employed as f32 / total_workforce as f32`
(`f64`を経由しない直接除算)とビット単位で一致するよう、`f32`のみで計算する。分母0では
1.0を返す(両ファイルの元の零除算フォールバックをそのまま踏襲)。**クランプはこの関数
自体では行わない**——`finance.rs`の元の計算はクランプしておらず、`production.rs`は
呼び出し側で`.clamp(0.0, 1.0)`していたため、その違いを関数へ一方的に押し付けず、
呼び出し側の既存差異をそのまま維持した。

### 3.2 UI用雇用率との違い

P23-003の`calculate_employment_rate(employed, civilian_workforce)`(分母は動員控除後の
民間労働力`C`)とは別の関数として維持した。今回の`calculate_economic_workforce_ratio`
は分母が`total_workforce()`(`P`、動員を考慮しない潜在労働力)であり、`C > 0`のとき
`E / P = (E / C) × (C / P)`と分解できる——つまり既存の`E / P`には「動員後も民間に
残っている労働力の割合(`C / P`)」と「残った民間労働力のうち雇用されている割合
(`E / C`)」の両方が暗黙に含まれている。ユーザー指示どおり、この式を
`(C / P) × calculate_workforce_satisfaction(C, required)`のような分解式へ書き換える
ことはしていない(動員0でも既存の`E / P`と一致しなくなるため——ユーザー提示の反例
`P=C=100, E=40`で確認済み)。

### 3.3 production Step1／Step3への接続

`process_country_production`のStep1(入力資源需要の事前集計)・Step3(最終稼働率決定)の
両方で、同一の`calculate_economic_workforce_ratio(state.employed_workforce,
state.total_workforce()).clamp(0.0, 1.0)`を呼ぶよう統一した。`calculate_operation_ratio`
自体・`logistics_ratio`・`input_resource_ratio`・建物需要/入力資源需要/在庫更新/最終
生産量の式はいずれも変更していない。

### 3.4 financeへの接続

`update_state_welfare_and_unrest`の`emp_rate`計算を
`calculate_economic_workforce_ratio(state.employed_workforce, state.total_workforce())`
(クランプなし)へ置き換えた。`calculate_state_tax()`の数式・税率・行政費・生活水準/
不満度のしきい値(`0.7`/`0.9`)・月次変化量(`+0.5`/`-0.8`/`+0.8`)・クランプ(`±2.0`/月)
はいずれも変更していない。動員量を直接税収から差し引く新処理は追加していない——
税収への影響は既存どおり「経済用労働力比率→生活水準/不満度の月次変化→(生活水準を
参照する)後続月の税収」という間接経路のみである。

### 3.5 動員ゼロで数値を維持した方法

`tests/economy_tests.rs`に、リファクタリング前の`finance.rs`/`production.rs`を対象に
実際の生産量・stockpile・生活水準・不満度の固定値を手計算でassertする
characterization testを7件追加し、**リファクタリング前にPASSすることを確認した上で**、
`calculate_economic_workforce_ratio`への置換を行い、**同じテストが1文字も変更せずに
そのまま再度PASSすること**を確認した(§6参照)。テスト自体は本番実装と同じ数式を
複製せず、観測可能な出力(`stockpile.get(Food)`・`living_standard`・`unrest`)への
固定値assertとした。

### 3.6 余剰労働力と労働力不足時の挙動

* 余剰労働力で動員を吸収できる間(`civilian_workforce() >= 建物需要`)は、
  `employed_workforce`が需要で頭打ちのまま変化しないため、`E`が変わらず、`P`
  (`total_workforce()`)も動員の影響を受けないため、`E / P`自体が数学的に一切変化
  しない。これは新しい保護ロジックではなく、既存の`employed = min(civilian_workforce,
  required_workforce)`(P23-002確定済み)という不変条件から自動的に導かれる性質であり、
  実e2eテストで確認した。
* 動員が建物需要を割り込ませた場合(`civilian_workforce() < 建物需要`)は、
  `employed_workforce = civilian_workforce()`(需要ではなくプール自体で頭打ち)となり、
  `E`が動員量に応じて減るため、`E / P`が単調に低下する。

### 3.7 `calculate_workforce_satisfaction()`と`_satisfaction`の扱い

全呼び出しを確認した結果(`grep -rn calculate_workforce_satisfaction src`)、実際に
使われているのは`population::update::update_state_population_and_employment`内の
`let _satisfaction = calculate_workforce_satisfaction(total_wf, total_required_wf);`
の1箇所のみで、その戻り値は`_`接頭辞のとおり一切使用されず、副作用も無かった
(この州で建物需要が民間労働力を上回っているかどうかを表す、今回の経済用労働力比率
とは異なる指標)。他に呼び出し元が無いことを確認したため、**この1行のみを削除した**。
公開関数`calculate_workforce_satisfaction()`自体、雇用更新ロジック
(`employed`/`unemployed_workforce`の算出式)は変更していない。将来、この値を
production.rsの需要充足度チェックへ接続すべきかという論点は§8の後続候補として残す。

### 3.8 月次処理順序の確認結果

`economy::handle_monthly_economy`(単一のSystem関数)内で、州別動員負担の算出→
`update_state_population_and_employment`(雇用更新)→`process_country_production`
(生産、Step1/Step3とも更新後の`employed_workforce`を読む)→`process_country_finance`
(労働力を使わない)→`update_state_welfare_and_unrest`(福祉、同じく更新後の
`employed_workforce`を読む)の順に、**同一関数内で逐次実行**されている。全ステップが
同じ月次tick内・同じ`employed_workforce`値を参照するため、1か月遅延のような意図的
または非決定的なタイミング差は存在しないことを確認した。`DailySimulationSet`レベルの
順序修正は不要だった(P23-003で発見・修正したUI表示System(`update_state_panel`等)の
Set未割り当て問題とは異なり、production/financeはいずれも`handle_monthly_economy`
という単一Systemの内部処理であり、Bevyのスケジューラによる並行実行順序の不確定性が
そもそも介在しない)。

### 3.9 セーブV1を維持した方法

新規セーブフィールド・DTO変更・バージョン変更は無い。`calculate_economic_workforce_ratio`
は`employed_workforce`・`total_workforce()`(=`population`・`workforce_ratio`から
その場で再導出)という既存のセーブ対象/導出値のみから毎回再計算する派生値であり、
一切永続化していない。

## 4. 変更ファイル一覧と役割

| ファイル | 役割 |
|---|---|
| `src/population/update.rs`(変更) | `calculate_economic_workforce_ratio`を追加、`_satisfaction`の dead code を削除、単体テスト7件追加 |
| `src/economy/finance.rs`(変更) | `update_state_welfare_and_unrest`の`emp_rate`計算を共通関数へ置換 |
| `src/economy/production.rs`(変更) | `process_country_production`のStep1/Step3の`wf_ratio`計算を共通関数へ置換 |
| `tests/economy_tests.rs`(変更) | finance/productionのcharacterization test 7件追加 |
| `tests/p23_004_economic_workforce_ratio_e2e_test.rs`(新規) | 実月次処理経路の統合テスト7件 |
| 本完了報告書(新規、別コミット) | — |

## 5. 追加・更新したテスト

### 5.1 純粋関数テスト(`src/population/update.rs`、7件)

1. `calculate_economic_workforce_ratio_matches_the_original_finance_and_production_formula`
2. `calculate_economic_workforce_ratio_returns_one_for_zero_potential_workforce`
3. `calculate_economic_workforce_ratio_is_zero_when_employed_is_zero_and_potential_is_positive`
4. `calculate_economic_workforce_ratio_is_one_when_employed_equals_potential`
5. `calculate_economic_workforce_ratio_is_not_clamped_when_employed_exceeds_potential`
6. `calculate_economic_workforce_ratio_does_not_panic_or_overflow_at_the_u64_boundary`
7. `calculate_economic_workforce_ratio_is_monotonically_non_increasing_as_employed_falls`

### 5.2 production回帰テスト(`tests/economy_tests.rs`、4件)

1. `characterization_production_output_matches_employed_over_potential_workforce_ratio`
2. `characterization_production_output_is_unaffected_by_unemployed_workforce_alone`
3. `characterization_production_output_decreases_monotonically_as_employed_falls`
4. `characterization_production_falls_back_to_full_workforce_ratio_when_potential_is_zero`

### 5.3 finance回帰テスト(`tests/economy_tests.rs`、3件)

1. `characterization_welfare_employment_ratio_is_not_clamped_above_one`
2. `characterization_welfare_low_employment_ratio_degrades_living_standard_and_unrest`
3. `characterization_welfare_falls_back_to_full_employment_when_potential_is_zero`
4. `characterization_welfare_does_not_improve_as_employment_ratio_falls`

(※4件、上と合わせて計7件が§3.5のcharacterization test)

### 5.4 実月次経路の統合テスト(`tests/p23_004_economic_workforce_ratio_e2e_test.rs`、7件)

実`AppPlugin`+`EconomyPlugin`+`PopulationPlugin`等(UIパネルは対象外のため未登録)を
組み合わせ、実際の`handle_monthly_economy`を通した。人口成長(P23-001)は動員の有無に
関わらず毎月必ず実行されるため、同一Appを複数月進めて比較すると人口成長由来の差分と
動員由来の差分が混ざる問題を発見し、比較が必要なケースは経過月数を揃えた2つの独立
したAppで比較する設計にした(詳細はファイル冒頭のコメント参照)。

1. `zero_mobilization_yields_a_healthy_workforce_ratio_and_positive_production`
2. `mobilization_absorbed_by_surplus_labor_does_not_change_production_or_welfare`
3. `mobilization_below_required_workforce_reduces_production`
4. `heavier_mobilization_never_improves_welfare_and_can_push_it_into_the_degraded_band`
5. `demobilizing_restores_the_workforce_ratio_and_that_months_production_on_the_next_tick`
6. `one_countrys_mobilization_does_not_affect_another_countrys_production_or_welfare`
7. `a_real_save_and_load_round_trip_reproduces_the_same_economic_workforce_ratio`

これらのテストを組む過程で、実データの州が既に建物を保有していること、および
`apportion_mobilized_manpower`(P23-002)が国家の動員量を「所有する全州」の人口比で
按分するため、対象州以外にも実データの州を所有したままだと指定した動員量のごく
一部しか対象州へ届かないことを発見した。対象州以外の同一国家所有州の人口を0にし
(所有権自体は変更しない)、既存建物をクリアしてから制御用のFarmだけを設定すること
で、動員量と経済結果の関係を確定的にテストできるようにした。

## 6. 実行した全コマンドと正確な結果

| コマンド | 結果 |
|---|---|
| `cargo test --test economy_tests`(リファクタ前、characterization test追加直後) | PASS、22 passed(既存15+新規7、**旧`total_workforce()`直書きのfinance.rs/production.rsに対して**手計算した期待値が正しいことを確認) |
| `finance.rs`/`production.rs`をリファクタリング | — |
| `cargo check --all-targets` | PASS |
| `cargo test --test economy_tests`(リファクタ後、テストは1文字も変更せず再実行) | PASS、**22 passed(全件、リファクタ前と完全に同じ結果)** |
| `cargo test --lib population::update::` | PASS、19 passed(既存12+新規7) |
| `cargo test --lib` | PASS、1126 passed |
| `cargo test --test p23_004_economic_workforce_ratio_e2e_test` | PASS、7 passed |
| `rustfmt --edition 2024 --check`(変更・新規5ファイル個別指定) | 初回差分あり→`rustfmt --edition 2024`適用後PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、**警告0** |
| `cargo test --all-targets`(全体、最終実行) | PASS、**全件PASS**(1361 passed、**0 failed**、1 ignored[既存の`profile_p21_021_elimination_scaling_test`、本タスク無関係]) |

`tests/land_war_combat_peace_test.rs`は今回一切変更・整形していない(既知の保護対象)。

### 非決定的スクリーンショットPNGの扱い

`cargo test --all-targets`実行後、既知の9枚が想定どおり再生成されたことを`git status`
で確認し、個別ファイル単位で`git checkout -- <path>`により復元した(ディレクトリ単位の
復元は行っていない)。それ以外のファイルへの意図しない変更は無い。

## 7. 対象外を先行実装していないこと

動員しただけで発生する一律の生産ペナルティ・新しい`mobilization_disruption_ratio`・
徴兵に対する政治的不満・徴兵法/動員法/政治体制補正・`war_exhaustion`・戦争疲弊による
生産/人口補正・税収への直接的な動員乗数・`calculate_state_tax()`の数式変更・生活水準/
不満度のしきい値や変化量の再調整・生産量/建物需要の基礎バランス調整・
`calculate_workforce_satisfaction()`を使用した生産方式への移行・雇用者/失業者の算出
ロジック変更・UI/ローカライズ変更・AI動員判断・新しい軍事補充システム・新しいセーブ
フィールド・セーブV2は、いずれも変更ファイル一覧(§4)に含まれておらず、実装して
いないことを確認した。

## 8. 残課題または後続候補

* `calculate_workforce_satisfaction()`(建物需要に対する民間労働力の充足率)は、
  今回削除した`_satisfaction`ローカル計算以外に呼び出し元が無い、事実上未使用の
  公開関数のまま残っている。将来、production.rsの稼働率計算をこの充足率ベースの
  設計へ移行すべきかという論点があるが、これは今回の「既存挙動を維持したまま集約
  する」というスコープを超えたゲームバランス変更であり、本タスクでは意図的に
  手を付けていない。
* `economy::finance`の`emp_rate`しきい値(`0.7`/`0.9`)や月次変化量
  (`+0.5`/`-0.8`/`+0.8`)自体が、動員シナリオにおいて妥当な水準かどうかは未検証
  (P23-004は既存しきい値を維持することが要件だったため、妥当性の検証自体は対象外)。

## 9. 実装コミット直後と報告書コミット後、それぞれの`git status --short`

### 実装コミット(`697b0ece6ab46a459db66e39261785eb69f6d136`)直後

```
 M saves/savegame_v1.ron
```

`saves/savegame_v1.ron`はユーザーの既存未コミット変更であり、実装コミットには
含めていない。それ以外の全変更(§4の実装・テストファイル)はコミット済み。

### 報告書コミット後

報告書コミット作成後の最終`git status --short`は、コミット完了後の最終回答で報告する
(本報告書自身のハッシュを本報告書内へ自己参照的に記載しないため、
[[feedback_report_hash_before_commit]]参照)。
