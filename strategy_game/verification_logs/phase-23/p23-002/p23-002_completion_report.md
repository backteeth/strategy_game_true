# P23-002 完了報告: 動員人的資源(mobilized_manpower)による民間労働力の圧迫

## 1. 目的とスコープ

P23-001に続き、既に募兵で追跡されている`CountryData.mobilized_manpower`を正規の情報源
として使い、国家が動員した人員を現在の所有州へ人口比で按分し、既存の月次雇用計算
(`economy::handle_monthly_economy` → `population::update::update_state_population_and_
employment`)が使う民間労働力から控除する。`war_exhaustion`・徴兵制度・政治体制連動・
新規UI等はいずれもユーザー指示の対象外リスト通り実装していない。

## 2. 事前調査結果

- `CountryData.mobilized_manpower: u64`(募兵`request_recruitment`で加算、キャンセル
  `cancel_recruitment`で全額返還、`src/military/recruitment.rs`)。他に更新経路は無い
  (grep調査で確認済み)。
- 州人口→民間労働力は`population::update::calculate_workforce(population, workforce_ratio)`
  (`.round()`丸め)が正規の計算経路。`StateData::total_workforce()`(`state/data.rs`、
  `as u64`切り捨て)は別の再導出で、`handle_monthly_economy`内の国家別`employment_rate`
  集計(表示指標)専用に使われており、`employed_workforce`/`unemployed_workforce`の
  実際の算出には使われていない。今回変更したのは前者(`calculate_workforce`経由の
  `update_state_population_and_employment`)のみ。
- `update_state_population_and_employment`(`population/update.rs`)の唯一の呼び出し元は
  `economy::handle_monthly_economy`(`DailySimulationSet::Economy`)。呼び出し側は
  国家ごとに`owned_states`(`owner_country_id`基準、既存の法的所有の正規経路)を
  `filter`で抽出するループを既に持っており、按分計算をこのループ内に自然に挿入できた。
  P23-001の月次人口成長は同じEconomy Setで`.before(handle_monthly_economy)`により
  先に実行されるため、按分に使う`state.population`は既に成長後の値。
- 人口・人的資源・雇用はいずれも`u64`。丸め規則は`calculate_workforce`が`.round()`
  (四捨五入)、`update_state_population_and_employment`内の新規控除は`saturating_sub`。
- セーブ対象は既存の`CountryData.mobilized_manpower`・`StateData.population`のみで、
  按分結果自体は最初から永続化しない設計とした(ユーザー指示通り、月次処理時に毎回
  再導出)。

## 3. 実装内容

### 州別按分方法(最大剰余法)

`src/population/mobilization.rs::apportion_mobilized_manpower(mobilized_manpower, states)`
(純粋関数、ECS非依存):

```
population_total = Σ population_i
base_i = floor(mobilized_manpower * population_i / population_total)   // u128中間計算
remainder_i = (mobilized_manpower * population_i) mod population_total
deficit = mobilized_manpower - Σ base_i
```

`deficit`件を、`remainder_i`降順・同率は`StateId`昇順(既存の安定した順序基準)でソートした
上位`deficit`件へ1人ずつ追加する。これにより通常ケースで合計が`mobilized_manpower`と
正確に一致する。

### 端数処理

切り捨て(`base_i`)+最大剰余法による1人単位の再配分。分数の繰越フィールドは追加していない
(P23-001と同じ方針)。

### 民間労働力への接続箇所

`population::update::update_state_population_and_employment`に`mobilized_burden: u64`
引数を追加し、

```rust
let total_wf = calculate_workforce(state.population, state.workforce_ratio)
    .saturating_sub(mobilized_burden);
```

とした(以降の`employed`/`unemployed`計算・建物側労働需要量・雇用優先順位は無変更)。
呼び出し元`economy::handle_monthly_economy`で、国家の`owned_states`を集めた直後に
`apportion_mobilized_manpower(country.mobilized_manpower, &population_pairs)`を1回だけ
呼び、各州へ按分結果を渡す。二重適用は起きない(呼び出しは1箇所のみ)。

### overflow/underflow対策

- `mobilized_manpower * population_i`は`u128`で計算(`base_i`/`remainder_i`は
  `mobilized_manpower`・`population_total`以下に収まるため`u64`へ安全に戻せる)。
- 労働力からの控除は`saturating_sub`(負値・アンダーフローなし、0で飽和)。
- `apportion_mobilized_manpower`自体は`u64::MAX`規模の`mobilized_manpower`・人口でも
  panicしないことをテストで確認(§5)。

### セーブV1を維持した方法

按分結果(州別動員負担)は一切永続化せず、`handle_monthly_economy`が呼ばれるたびに
`country.mobilized_manpower`(既存セーブ対象)と現在の`owned_states`人口(既存セーブ対象)
から毎回再導出する。新規セーブフィールド・新規DTO・バージョン変更は無い
(`SAVE_FORMAT_VERSION_V1`据え置き)。ロード後は`mobilized_manpower`がそのまま復元され、
次の月次雇用計算で同じ按分ロジックが同じ入力から同じ結果を再導出する
(§6のセーブ/ロード統合テストで確認)。

## 4. 変更ファイル一覧

| ファイル | 役割 |
|---|---|
| `src/population/mobilization.rs`(新規) | 按分の純粋関数とDomain単体テスト |
| `src/population/mod.rs`(変更) | `pub mod mobilization;`を追加 |
| `src/population/update.rs`(変更) | `update_state_population_and_employment`へ`mobilized_burden`引数を追加 |
| `src/economy/mod.rs`(変更) | `handle_monthly_economy`内で按分を計算し、各州の呼び出しへ渡す |
| `tests/p23_002_mobilized_manpower_workforce_e2e_test.rs`(新規) | 実経路(月次雇用計算・セーブ/ロード)の統合テスト |
| 本完了報告書(新規) | — |

## 5. Domain単体テスト(11件、`src/population/mobilization.rs`)

1. `zero_mobilized_manpower_yields_zero_burden_for_every_state`
2. `apportions_proportionally_to_population_across_multiple_states`
3. `state_totals_sum_exactly_to_the_original_mobilized_manpower`(複数の`mobilized_manpower`
   値で合計一致を確認)
4. `remainder_deficit_is_distributed_without_losing_a_single_person`
5. `equal_remainders_are_broken_by_ascending_state_id`
6. `a_zero_population_state_never_receives_burden`
7. `all_owned_states_with_zero_population_does_not_panic_or_divide_by_zero`
8. `no_owned_states_returns_an_empty_map_without_panicking`
9. `does_not_leak_burden_to_states_outside_the_provided_slice`
10. `extreme_values_do_not_overflow_or_panic`(`u64::MAX`規模)
11. `insertion_order_does_not_affect_the_result`

## 6. 月次雇用統合テスト(8件、`tests/p23_002_mobilized_manpower_workforce_e2e_test.rs`)

実`AppPlugin`+`CountryPlugin`+`StatePlugin`+`BuildingPlugin`+`EconomyPlugin`+
`PopulationPlugin`+`SaveGamePlugin`/`LoadGamePlugin`(実データ6か国)を組み合わせ、
`MonthChangedMessage`経由で実際の`handle_monthly_economy`を通した。

1. `without_mobilization_employment_reaches_the_full_civilian_workforce`
2. `mobilizing_manpower_reduces_available_workforce_and_employable_count_next_month`
3. `the_workforce_reduction_matches_the_population_weighted_apportionment`(実州データで
   `apportion_mobilized_manpower`を直接呼び、実System結果と一致することを確認)
4. `a_burden_larger_than_the_workforce_clamps_employment_to_zero_without_underflow`
5. `demobilizing_restores_workforce_on_the_next_monthly_tick`
6. `one_countrys_mobilization_does_not_affect_another_countrys_employment`
7. `ownership_transfer_moves_the_burden_to_the_new_owner_on_the_next_tick`
8. `a_real_save_and_load_round_trip_reproduces_the_same_workforce_result`
   (実`SaveRequestMessage`/`LoadRequestMessage`経路)

### 境界条件のカバレッジ

上記のうち、mobilized=0(#1)・州境負担が労働力超過(#4)・動員解除で回復(#5)・他国非波及
(#6)・所有権移転(#7)・save/load往復(#8)は、いずれもDomain単体テスト側の対応する
境界条件(§5の1・6/7・—・9・—・—)と対になっており、単体レベルと実System経路レベルの
両方で二重に固定されている。

## 7. 実行したコマンドと正確な結果

| コマンド | 結果 |
|---|---|
| `git diff --check` | PASS、exit 0(autocrlf警告のみ) |
| `rustfmt --edition 2024 --check`(変更・新規5ファイル個別指定) | PASS(適用後) |
| `cargo check --all-targets` | PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、**警告0** |
| `cargo test --lib population::mobilization` | PASS、11 passed |
| `cargo test --lib population::` | PASS、34 passed(growth23+mobilization11) |
| `cargo test --test p23_002_mobilized_manpower_workforce_e2e_test` | PASS、8 passed |
| `cargo test --test p23_001_population_manpower_e2e_test`(P23-001回帰) | PASS、3 passed |
| `cargo test --lib military::tests::`(recruitment回帰) | PASS、36 passed |
| `cargo test --lib state::ownership::` | PASS、19 passed |
| `cargo test --lib country::elimination::` | PASS、17 passed |
| `cargo test --test p20_009_localization_resource_test` | PASS、8 passed |
| `cargo test --lib save::` | PASS、300 passed |
| `cargo test --all-targets` | PASS、**全件PASS**(lib 1107、統合含め合計1320件、`cargo test --all-targets -- --list`で確認。旧1301から+19: 新規lib11件+新規integration8件) |

`tests/land_war_combat_peace_test.rs`は今回一切変更・整形していない(既知の保護対象)。

## 8. 対象外を先行実装していないこと

`war_exhaustion`・徴兵制度/徴兵法/動員法・政治体制や法律による動員可能率・幸福度/不満度/
支持率との連動・戦争疲弊による人口/生産補正・職業/階級/性別/年齢別労働人口・軍需産業への
特別な労働優先・新規UI/ツールチップ/統計画面・AIの動員判断変更・新しい軍事ユニット補充
システム・セーブV2は、いずれも変更ファイル一覧(§4)に含まれておらず、実装していない
ことを確認した。

## 9. 残課題または後続候補

- `economy::handle_monthly_economy`自体が既に持つ、国家ごとに全州へ`filter`をかける
  O(国家数×州数)の既存パターン(P23-001の完了報告でも既出)。今回の按分計算は
  この既存ループの中に追加しただけで、既存パターン自体の最適化は範囲外。
- `StateData::total_workforce()`(建物側`employment_rate`集計専用)は動員負担を
  考慮しない別経路のまま残っている。実際の`employed_workforce`/`unemployed_workforce`
  には影響しないため、今回のP23-002の成立には支障がないと判断し変更していない
  (国全体の雇用率表示指標が、動員による民間労働力圧迫を厳密には反映しないという
  軽微な不整合が残る——「後続候補」として記録するのみで、P23-003以降での対応が
  必要かはユーザー判断とする)。

## 10. コミット

- 対象ファイル: `src/population/mobilization.rs`(新規)、`src/population/mod.rs`
  (変更)、`src/population/update.rs`(変更)、`src/economy/mod.rs`(変更)、
  `tests/p23_002_mobilized_manpower_workforce_e2e_test.rs`(新規)、本完了報告書
- 除外したユーザーデータ: `saves/savegame_v1.ron`(ユーザーの実プレイによる変更、
  本タスクで一切参照・変更していない)
