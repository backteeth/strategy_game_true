# P23-001 完了報告: 州人口の月次成長とavailable_manpowerの月次回復

## 1. 目的とスコープ

Prototype v0.2基準文書(`verification_logs/prototype-v0.2-final/PROTOTYPE_V0.2_BASELINE.md`)
のPhase 23候補として選定した通り、`StateData.population`が時間経過で一切増加せず、
`CountryData.available_manpower`も募兵消費・キャンセル返還のみで時間経過による回復が
無いという中核ループの欠落を、既存の月次処理へ最小限接続することで解消する。年齢構成・
出生死亡・移民・戦争被害・疫病等の複雑な人口動態、州ごとの個別成長率、UI新規パネル等は
対象外とした(いずれもユーザー指示の対象外リストの通り)。

## 2. 事前調査結果

- `StateData.population: u64`(`#[serde(default)]`無し=必須フィールド、常にRONに存在)。
  実データ(`assets/data/states.ron`)の州人口は25,000〜2,100,000の範囲。
- `CountryData.available_manpower: u64`(`#[serde(default = "default_available_manpower")]`、
  既定値100,000)。既存テスト値はこの既定値およびP21-021等のテストで使う任意値のみで、
  月次回復に依存する既存アサーションは無かった(回帰確認済み、§8参照)。
- Country側の人口キャッシュ・集計フィールドは存在しない。ただし`country::power::
  compute_state_aggregates`(private)が、州を1回走査して`HashMap<CountryId, f64>`を
  構築する「所有国(owner、controllerではない)ごとの人口合計」パターンを既に持っていた
  (P21-014、表示用ランキング目的のため`f64`)。今回はこのパターンを踏襲しつつ、
  ゲームプレイへ直接影響する値のため`f64`を使わない独自の`u64`版
  (`population::growth::compute_population_by_owner`)を新設した(既存のprivate関数を
  流用せず、決定論要件が異なるため意図的に分離)。
- 州所有者の正規データは`StateData.owner_country_id`(法的所有、既存の`StateRegistry::
  get_owned_states`と同じ基準)。`controller_country`(占領による実効支配)は使わない
  (人的資源は国家主権に属するリソースであり、占領国ではなく法的所有国へ帰属させるのが
  既存の経済処理[`handle_monthly_economy`]と同じ規約)。
- `MonthChangedMessage`は`app::time::advance_game_date`(`DailySimulationSet::TimeUpdate`)
  が発行する。ポーズ中は発行されない。高速なゲーム速度で1フレームに複数日が経過した場合、
  月境界をまたぐたびに複数件発行され得る(既存コードのコメントで確認済み)。
- `update_state_population_and_employment`(`population/update.rs`)は`state.population`を
  読むだけで書き込まない。`economy::handle_monthly_economy`(`DailySimulationSet::Economy`)
  から、国家ごとに所有州を`filter`で再抽出するループの中で呼ばれている
  (`for country { for state in states.filter(owner==country) { ... } }`という
  O(国家数×州数)の既存パターン。今回のタスクでは変更しない、既存コードのため)。
- 募兵時の人的資源消費(`request_recruitment`、`available_manpower -= required`)・
  キャンセル時返還(`cancel_recruitment`、`available_manpower += required`、全額)は
  そのまま。今回のSystemはこれらと同じフィールドを月次で加算するだけで、消費/返還の
  契約自体には触れていない。
- 国家消滅(`country::elimination`)は消滅国家の全所有州を併合国へ移転してから
  `CountryStatus::Eliminated`へ遷移させるため、消滅国家は消滅後常に所有州0件になる
  (P21-021の既存保証)。州所有権移転(`state::ownership`)は`owner_country_id`を
  書き換えるだけで、`StateData`自体は削除されない。
- セーブ対象は既存の`CountryData`/`StateData`をそのまま使う(`SaveGameV1.countries`/
  `.states`に埋め込み済み)。新規フィールドは追加していない。
- 既存の定数配置規則: `military/combat_calc.rs`の`RETREAT_MANPOWER_LOSS_RATIO`
  (`pub const`、モジュールトップレベル)、および同ファイルの
  `(x as i64 * ratio_1000) / 1_000_000`という整数固定小数点の書き方を踏襲した。

## 3. 採用した数値と根拠

| 定数 | 値 | 根拠 |
|---|---|---|
| `MONTHLY_POPULATION_GROWTH_RATE_BP` | 5(basis points、=0.05%/月) | ユーザー指定の標準値。年率換算約0.6%、150年(1800か月)複利で`e^(0.0005*1800)≒2.4596`倍——「開始時の2〜3倍に収まる水準」を満たす(§7の1800か月テストで実測確認)。実データ最小州人口25,000でも月12(floor)の非ゼロ成長。 |
| `MANPOWER_CAP_RATIO_BP` | 1,000(=10%) | ユーザー指定の標準値。既存の類似比率(募兵可能人口率)がリポジトリ内に存在しないことを確認した上で採用。 |
| `MANPOWER_RECOVERY_PERIOD_MONTHS` | 120(=10年) | ユーザー指定の標準値。「空の人的資源が概ね10年で基準上限まで回復する」を満たす。 |

実データの人口単位(万〜百万規模)とこれらの比率は著しく不整合ではないと判断し、
ユーザー指定値をそのまま採用した(変更の必要性は生じなかった)。

## 4. 実装内容

### 変更・追加ファイル

- `src/population/growth.rs`(新規): 成長・集計・回復の純粋関数群とSystem本体。
- `src/population/mod.rs`(変更): `PopulationPlugin`(既存、main.rsには登録済みだが
  `build()`が空だった)へ`handle_monthly_population_growth`を実際に登録。
- `tests/p23_001_population_manpower_e2e_test.rs`(新規): 経済処理との統合・セーブ/ロード
  往復の統合テスト。

### 人口成長式

```
grow_state_population(population) =
    population + (population * 5) / 10_000   // floor、saturating演算
```

### 人的資源上限・回復式

```
compute_manpower_cap(population) = (population * 1_000) / 10_000   // = population / 10
recover_manpower(current, cap):
    if cap == 0 || current >= cap: return current   // 削除しない、回復もしない
    deficit = cap - current
    raw = cap / 120
    recovery = max(raw, 1).min(deficit)              // 0にはしない、上限は超えない
    return current + recovery
```

### 丸め・overflow方針

- 浮動小数点は一切使わない(全て`u64`固定小数点、basis points分母10,000)。
- 端数は切り捨て。分数繰越フィールドは追加していない(ユーザー指示通り最小スコープ)。
- 人口0の州は`0*5/10000==0`で自然に0のまま、特別扱いのコードは書いていない。
- 小人口州への最低成長数の強制は行っていない(1,999人の州は月0人成長のまま、テストで固定)。
- `saturating_mul`/`saturating_add`によりオーバーフロー時もwrapしない(`u64::MAX`での
  テストで確認)。実データの規模(最大2,100,000)では発生し得ないが防御的に扱った。

### System実行順序

```
handle_monthly_population_growth
    .in_set(DailySimulationSet::Economy)
    .before(economy::handle_monthly_economy)
    .run_if(in_state(GameState::Playing))
```

`DailySimulationSet::Economy`自体は`TimeUpdate`(`MonthChangedMessage`発行元)の直後、
`WarResolution`(州所有権移転・国家消滅が実際に起こるWarPlugin側の処理)より前に位置する
(`GameTimePlugin::configure_sets`、既存の`.chain()`)。同一Set内の`handle_monthly_economy`
との相対順序は上記`.before()`で明示した(Setが同じというだけでは順序は保証されないため、
コメントのみに頼らずコードで固定)。

**州所有権移転・国家消滅が同じ日に起きる場合の反映タイミング**: 実ゲームでは州所有権移転・
国家消滅は`WarResolution` Set(講和settlement等)で発生する。`Economy`は`WarResolution`より
前に実行されるため、移転・消滅が起きたまさにその日が同時に月替わりの日であっても、その月の
人口集計・人的資源回復はまだ旧所有国側で行われ、新所有国側への反映は翌月の月替わりから
となる。この挙動を`population::growth::tests::ownership_transfer_in_war_resolution_is_
reflected_starting_the_following_months_tick`で、実際に設定された`DailySimulationSet`
(`GameTimePlugin`が`configure_sets`する本番の順序)を通して固定した。

`MessageReader<MonthChangedMessage>`は`economy::handle_monthly_economy`・
`research::handle_monthly_research`・`country::power::rebuild_country_power_monthly`
・本Systemがそれぞれ独立したカーソルで読むため、Bevyの仕様上、同じフレームで複数System
が同じメッセージ集合を安全に読める(1つのSystemが`.read()`で消費しても他のSystemには
影響しない)。

### 計算量

州走査2回(成長・集計、いずれもO(州数))+ 国家走査1回(回復、O(国家数))
= O(州数 + 国家数)。国家ごとに全州を再走査するO(国家数×州数)は行っていない
(既存の`handle_monthly_economy`自体はこのアンチパターンを持つが、今回のタスクでは
変更しない既存コードのため対象外)。

### セーブ形式への影響

**なし。** `SAVE_FORMAT_VERSION_V1`据え置き。人口・`available_manpower`は既存の
`CountryData`/`StateData`(`SaveGameV1.countries`/`.states`)フィールドをそのまま
更新するだけで、新規フィールド・新規DTOは追加していない。人的資源基準上限自体も
恒久フィールドとして保持せず、月次処理のたびに所有州人口から再導出する
(ユーザー指示通り)。

## 5. テスト

### 人口成長(8件、`src/population/growth.rs`)

1. `one_month_changed_message_grows_population_by_exactly_one_month`
2. `no_month_changed_message_leaves_population_and_manpower_unchanged`
3. `two_month_changed_messages_in_one_frame_apply_growth_exactly_twice`
4. `grow_state_population_zero_stays_zero`
5. `grow_state_population_rounds_down_for_a_small_population`(25,000→12人増、1,999→0人増、
   2,000→ちょうど1人増、の3境界を1テストで固定)
6. `grow_state_population_does_not_overflow_for_a_very_large_population`
7. `state_population_keeps_growing_after_its_owner_country_is_eliminated`
8. `compute_population_by_owner_is_independent_of_insertion_order`

### 人的資源(10件)

1. `active_country_manpower_recovers_monthly_toward_its_owned_population_cap`
2. `recover_manpower_never_exceeds_the_cap`
3. `recover_manpower_does_nothing_once_the_cap_is_already_reached`
4. `recover_manpower_does_not_delete_a_surplus_when_the_cap_drops_below_current`
5. `eliminated_country_does_not_recover_manpower_even_with_leftover_population_credit`
6. `a_country_with_no_owned_states_does_not_recover_manpower`
7. `compute_population_by_owner_sums_multiple_states_and_does_not_cross_owners`
8. (同上テストが他国非混入も同時に検証)
9. `ownership_transfer_in_war_resolution_is_reflected_starting_the_following_months_tick`、
   `manpower_recovery_follows_ownership_transfer_reflected_before_this_months_tick`
10. 既存`military::tests::test_recruitment_cancellation`・`test_recruitment_manpower_
    shortage`等40件を無変更のまま回帰確認(§6参照)

### 長期挙動(4件)

1. `eighteen_hundred_months_run_without_panicking_or_overflowing`(150年、panicなし、
   人口が開始時の2〜3倍に収まることを同時に確認)
2. `identical_starting_state_and_month_count_produce_identical_results`(決定論)
3. `empty_manpower_reaches_the_cap_within_the_configured_recovery_period`(120か月で
   基準上限の95%以上に到達——人口・上限自体も同時に成長し続ける移動目標のため、
   150年テストで確認した「十分な月数が経てば完全一致」と役割分担した現実的な閾値)
4. `no_month_changed_message_leaves_population_and_manpower_unchanged`(誤発火なし、
   人口成長の§2と共用)

### 統合・セーブ(3件、`tests/p23_001_population_manpower_e2e_test.rs`)

1. `monthly_economy_employment_uses_the_already_grown_population_in_the_same_month`
   — 実`EconomyPlugin`+`PopulationPlugin`を組み合わせ、`handle_monthly_economy`が
   同じ月内で成長後の人口を実際に参照することを確認
2. `growth_and_recovery_survive_a_real_save_and_load_round_trip` — 実`SaveGamePlugin`/
   `LoadGamePlugin`経路(read/validate/apply)で、成長・回復後の値が保存・復元され、
   ロード直後は追加成長せず、ロード後の最初の月替わりで正確に1回だけ成長することを確認
3. `manpower_recovery_respects_ownership_after_a_real_state_ownership_transfer`
   — 実州所有権移転後、新所有者側に人口・人的資源が反映されることを確認

### 実行コマンドと結果

| コマンド | 結果 |
|---|---|
| `git diff --check` | PASS |
| `rustfmt --edition 2024 --check`(変更・新規3ファイル個別指定) | PASS(適用後) |
| `cargo check --all-targets` | PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、警告0 |
| `cargo test --lib population::` | PASS、23 passed |
| `cargo test --test p23_001_population_manpower_e2e_test` | PASS、3 passed |
| `cargo test --lib military::tests::`(recruitment回帰) | PASS、40 passed |
| `cargo test --lib state::ownership` | PASS、19 passed |
| `cargo test --lib country::elimination` | PASS、17 passed |
| `cargo test --test p20_009_localization_resource_test` | PASS、8 passed |
| `cargo test --lib save::` | PASS、300 passed |
| `cargo test --all-targets` | PASS、全件PASS(合計1301件、`cargo test --all-targets -- --list`で確認。旧1275から+26: 新規lib23件+新規integration3件) |

`tests/land_war_combat_peace_test.rs`は今回一切変更・整形していない(既知の保護対象)。

## 6. 発見した問題(今回修正しなかったもの)

- `economy::handle_monthly_economy`自体が、国家ごとに全州へ`filter`をかけるO(国家数×州数)
  の既存パターンを持つ(`for country { states.iter_mut().filter(owner==country_id) }`)。
  今回のP23-001Systemはこのアンチパターンを踏襲せず単一走査で実装したが、
  `handle_monthly_economy`側の最適化自体は今回のタスク範囲外のため変更していない。

## 7. コミット

- 対象ファイル: `src/population/growth.rs`(新規)、`src/population/mod.rs`(変更)、
  `tests/p23_001_population_manpower_e2e_test.rs`(新規)、本完了報告書
- 除外したユーザーデータ: `saves/savegame_v1.ron`(ユーザーの実プレイによる変更、
  本タスクで一切参照・変更していない)

## 8. 次の推奨タスク

会話内の最終報告(§次の推奨タスク)を参照。
