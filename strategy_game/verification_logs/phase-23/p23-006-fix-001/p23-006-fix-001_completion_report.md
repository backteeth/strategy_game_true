# P23-006-FIX-001 完了報告: New Game初期軍の人的資源帳簿初期化と未解決募兵定義の顕在化

## 1. 総合判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

自動テスト・検証コマンドはすべて成功したが、実ウィンドウでの目視確認(`cargo run`)は
本セッションでは実施していない(§8参照)。

## 2. 基準状態

* 開始時HEAD: `99c1b80d9d2caeff4a201d1dca6204a5af90d310`(P23-006完了報告書コミット)
* 実装コミットの完全なハッシュ: `4927c5050201769af8ef261c7aa2c7727f566f85`
  (`fix: initialize new-game manpower ledger`)
* 報告書コミットの完全なハッシュ: 本報告コミット後に確定するため、このコミット自身の
  ハッシュはここへ自己参照的に記載しない(コミット後の最終回答で報告する)
* 最終HEAD: 報告書コミット後に確定
* ユーザーの既存未コミット変更(`saves/savegame_v1.ron`)は本タスク開始前から存在する
  実プレイデータであり、一切参照・変更・ステージ・コミットしていない。

事前確認: `src/app/loader.rs::spawn_debug_divisions`・`PlayingEntryMode`・New Game/
Loaded Gameの実行条件・`CountryData.mobilized_manpower`・
`military::manpower::compute_manpower_ledger`・`ManpowerLedger`・募兵キューと
`DivisionDefinition`の参照方法・軍事パネルのエラー処理・save validationの未知
`DivisionDefinition`拒否経路・P23-006の単体/統合テストを、現在のHEADで再確認した。
プロンプトの前提との重大な食い違いはなかった。

## 3. New Game修正

### 3.1 初期Divisionと`mobilized_manpower`の原子的対応

`spawn_debug_divisions`(`src/app/loader.rs`)で、初期Division生成に使った
`def.required_manpower`をローカル変数`initial_manpower`へ束縛し、`Division.manpower`/
`max_manpower`とその所有国の`CountryData.mobilized_manpower`(`saturating_add`)の
両方へ同じ値を使う。固定値の別途ハードコードや、UI側・別処理での再計算は行っていない。

### 3.2 二重spawn・二重加算の防止

新しい独立フラグは追加していない。既存の仕組み(`src/app/game_state.rs`の
`PlayingEntryMode` + `OnEnter(GameState::Playing)`)をそのまま再利用した。

* `spawn_debug_divisions`は`OnEnter(GameState::Playing)`へ登録されており、Bevyの
  `OnEnter`スケジュール自体の性質上、実際の状態遷移ごとに正確に1回だけ実行される。
* `entry_mode != PlayingEntryMode::NewGame`の早期returnは変更していない。
* 同じ`OnEnter`チェーン内で`spawn_debug_divisions`の直後に実行される
  `reset_playing_entry_mode`が、処理完了後に`PlayingEntryMode`を必ず`NewGame`へ
  決定的に初期化し直す(既存動作)。
* この仕組みが実際に機能することを、`tests/p23_006_fix_001_initial_manpower_ledger_
  e2e_test.rs::re_running_after_entry_mode_resets_away_from_new_game_does_not_
  duplicate`で固定した。

### 3.3 `available_manpower`とpopulationを変更していないこと

`spawn_debug_divisions`は`available_manpower`・`StateData.population`のいずれにも
一切触れていない(変更前のコードから触れていなかった箇所は変更していない)。

### 3.4 New Game直後の`B`・`L`・`K`・不整合量

実データ(6か国)を使った統合テスト
(`a_real_new_game_start_gives_every_active_country_a_balanced_ledger`)で、
すべての活動国家について:

```text
workforce_burden == living_military_manpower
permanent_military_losses == 0
unaccounted_living_excess == 0
```

が成立することを固定した。

### 3.5 初期軍が民間労働力へ反映されるタイミング

初期Division配置自体は月次雇用計算を先行実行しない
(`the_initial_spawn_itself_does_not_pre_run_monthly_employment`で、Startup +
`OnEnter(Playing)`直後は`employed_workforce`/`unemployed_workforce`が初期値0のままで
あることを確認)。次の正規月次処理(`MonthChangedMessage`駆動の
`economy::handle_monthly_economy`)で、`population::mobilization::
apportion_mobilized_manpower`が新しい`mobilized_manpower`を州へ按分し、民間労働力へ
反映される(`initial_army_burden_reduces_civilian_workforce_after_the_next_monthly_
update`で、反実仮想[初期負担0]との比較により確認)。

## 4. 未知定義の修正

### 4.1 `compute_manpower_ledger`の新しい戻り値

```rust
pub fn compute_manpower_ledger(
    country: &CountryData,
    military_registry: &MilitaryRegistry,
) -> Result<ManpowerLedger, ManpowerLedgerError>
```

### 4.2 エラー型

```rust
pub struct ManpowerLedgerError {
    pub unknown_definitions: Vec<DivisionDefinitionId>,
}
```

募兵キューが参照する`DivisionDefinitionId`のうち`military_registry.definitions`に
存在しないものを全て収集し、1件でもあれば`Err`を返す(0人として黙って`Ok`を返さない)。

### 4.3 決定性

未解決IDの一覧は昇順ソート・重複除去済みで返す。`recruitment_queue`(`Vec`)内の
出現順にも`military_registry.definitions`(`HashMap`)の走査順にも依存しないことを、
`multiple_unknown_definitions_are_reported_sorted_and_deduplicated`(挿入順の異なる
複数の未知ID・重複IDを含むキューで、常に同じソート済み一覧が返ることを固定)で確認した。

### 4.4 恒久損失への誤分類を防いだ方法

未知定義を検出した時点で即座に`Err`を返し、`ManpowerLedger`(`K`を含む)を一切構築
しない。したがって未解決の予約人数が`K`(恒久軍事損失)へ混入する経路は存在しない。
`a_queue_mixing_known_and_unknown_definitions_still_errors`で、既知の項目が1件でも
含まれていても、未知の項目が1件あれば帳簿全体が失敗することを確認した。

### 4.5 UIでの安全な表示

`update_military_panel_ui`(`src/ui/military_panel.rs`)を`match`で分岐し、
`Ok`なら既存どおり3行(利用可能/基準上限、現役/累積動員負担、恒久軍事損失)を表示、
`Err`なら「現役/累積動員負担」「恒久軍事損失」の2行だけを、新規ローカライズキー
`military_panel.manpower_unavailable`(JA「計算不能」/EN「Unable to Compute」)へ
差し替えて表示する。`DivisionDefinitionId`そのものは表示に使わない(IDを露出しない)。
「利用可能人的資源/基準上限」の行は`compute_manpower_ledger`に依存しないため、
エラー時も変わらず表示される。panicしないことを
`military_panel_shows_unable_to_compute_for_an_unresolved_queue_without_panicking`
で確認した。

### 4.6 save validationとの関係

`save::validate`側のロジックは変更していない。既存の
`unknown_recruitment_division_definition_is_rejected`テストは無変更のまま
今回のテスト実行でも成功しており(§7)、New Gameおよび検証済みV1セーブでは
`compute_manpower_ledger`が`Err`を返す状況自体が発生しない(save validationが
ロード時点で既に拒否するため)ことを裏付けている。`compute_manpower_ledger`の
`Err`経路は、その外部契約を信頼しない防御的な安全策という位置づけのままである。

## 5. 変更ファイル一覧と役割

| ファイル | 役割 |
|---|---|
| `src/app/loader.rs`(変更) | `spawn_debug_divisions`が初期Divisionのmanpowerを所有国の`mobilized_manpower`へ加算 |
| `src/military/manpower.rs`(変更) | `compute_manpower_ledger`を`Result`化、`ManpowerLedgerError`追加、既存テストを`.unwrap()`化、新規テスト3件 |
| `src/ui/military_panel.rs`(変更) | `compute_manpower_ledger`のErr分岐で「計算不能」表示 |
| `assets/localization/en-US.ron`・`ja-JP.ron`(変更) | `military_panel.manpower_unavailable`を追加 |
| `tests/p23_006_fix_001_initial_manpower_ledger_e2e_test.rs`(新規) | 本タスクの実経路統合テスト19件 |
| `tests/p23_006_manpower_ledger_e2e_test.rs`(変更) | `compute_manpower_ledger`のResult化に伴う`.unwrap()`追加のみ(値の意味は無変更) |
| `tests/p23_001_population_manpower_e2e_test.rs`(変更) | 初期軍の付随的負担を考慮するよう1件のテストの期待値式を修正(回帰、§9参照) |
| `tests/p23_002_mobilized_manpower_workforce_e2e_test.rs`(変更) | 「動員なし」の前提を明示的にゼロへ揃えるよう2件のテストを修正(回帰、§9参照) |
| `tests/p23_004_economic_workforce_ratio_e2e_test.rs`(変更) | 「動員なし」の前提を明示的にゼロへ揃えるよう2件のテスト・共通ヘルパー1件を修正(回帰、§9参照) |
| 本完了報告書(新規、別コミット) | — |

## 6. 追加・更新したテスト

### 6.1 loader/System(最小構成、7件)

`spawning_a_division_increases_the_owners_burden_by_its_actual_manpower`、
`multiple_countries_each_gain_independent_burden_for_their_own_initial_division`、
`a_country_gains_no_division_or_burden_when_no_matching_definition_exists`、
`re_running_after_entry_mode_resets_away_from_new_game_does_not_duplicate`、
`extreme_manpower_values_do_not_overflow_or_panic`、
`available_manpower_is_unchanged_by_initial_division_spawn`
(populationは実データを使う統合テスト側§6.2で確認)。

### 6.2 実New Game(完全なApp構成、実データ6か国、6件)

`a_real_new_game_start_gives_every_active_country_a_balanced_ledger`、
`initial_divisions_are_owned_by_the_correct_country`、
`advancing_several_updates_does_not_duplicate_the_initial_army_or_burden`、
`initial_army_placement_alone_does_not_reduce_available_manpower_or_change_population`、
`military_panel_shows_matching_living_and_burden_with_zero_losses_at_new_game_start`。

### 6.3 月次民間労働力(3件)

`initial_army_burden_reduces_civilian_workforce_after_the_next_monthly_update`、
`another_countrys_initial_army_does_not_affect_this_countrys_civilian_workforce`、
`the_initial_spawn_itself_does_not_pre_run_monthly_employment`。

### 6.4 Loaded Game・Save/Load(3件)

`loaded_game_does_not_run_spawn_debug_divisions_and_does_not_duplicate_saved_divisions`、
`a_balanced_new_game_save_reproduces_the_same_ledger_after_reload`、
`a_save_with_existing_permanent_losses_preserves_them_after_reload`。

### 6.5 未知DivisionDefinition(UI・実経路、2件、単体レベルは§ 5.1参照)

`military_panel_shows_unable_to_compute_for_an_unresolved_queue_without_panicking`、
`ja_and_en_resolve_the_unavailable_label_without_missing_key_markers`。

### 6.6 Domain単体テスト追加(`src/military/manpower.rs`、3件)

`multiple_unknown_definitions_are_reported_sorted_and_deduplicated`、
`a_queue_mixing_known_and_unknown_definitions_still_errors`、
`a_fully_resolved_queue_is_unaffected_by_the_error_path`(既存の
`an_unknown_division_definition_in_the_queue_contributes_zero_without_panicking`は
`an_unknown_division_definition_in_the_queue_returns_an_explicit_error_without_
panicking`へ改名し、`Err`をassertするよう更新)。

## 7. 実行した全コマンドと正確な結果

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | PASS |
| `cargo test --lib military::manpower::` | PASS、16 passed |
| `cargo test --test p23_006_fix_001_initial_manpower_ledger_e2e_test` | PASS、19 passed |
| `cargo test --test p23_006_manpower_ledger_e2e_test` | PASS、22 passed |
| `cargo test --test p23_001_population_manpower_e2e_test` | 初回1件FAILED(§9)→修正後PASS、3 passed |
| `cargo test --test p23_002_mobilized_manpower_workforce_e2e_test` | 初回2件FAILED(§9)→修正後PASS、8 passed |
| `cargo test --test p23_003_population_manpower_ui_e2e_test` | PASS、8 passed |
| `cargo test --test p23_004_economic_workforce_ratio_e2e_test` | 初回4件FAILED(§9)→修正後PASS、7 passed |
| `cargo test --test p20_009_localization_resource_test` | PASS、8 passed |
| `cargo test --test p20_009_hardcoded_string_scan_test` | PASS、4 passed |
| `cargo test --lib save::` | PASS |
| `rustfmt --edition 2024 --check`(変更・新規ファイル個別指定) | 初回差分あり→適用後PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、**警告0** |
| `cargo test --lib` | PASS、1142 passed |
| `cargo test --all-targets`(全体、最終実行) | PASS、**全件PASS**(1418 passed、**0 failed**、1 ignored[既存の`profile_p21_021_elimination_scaling_test`、本タスク無関係]) |

`tests/land_war_combat_peace_test.rs`は今回一切変更・整形していない(既知の保護対象)。
既知の非決定的PNG9枚は`cargo test --all-targets`実行後に想定どおり再生成され、
個別ファイル単位で`git checkout --`により復元した(ディレクトリ単位の復元はしていない)。

## 8. 手動GUI確認

**実施していない。** 本セッションには実ウィンドウを操作する手段が無く、
`cargo run`によるスクリーンショット撮影・実際の目視確認は一切行っていない。

未確認項目:

* New Game開始直後の軍事パネルで、実際の画面上で現役軍事人的資源と累積動員負担が
  一致して見えるか、恒久軍事損失0の表示レイアウトが崩れていないか
* 日本語・英語で、正常データでは「計算不能」が露出しないこと、長い英語ラベルでの
  重なりの有無
* 実際のSave/Load操作(UIボタン経由)後の表示
* パネルを閉じて再表示した際の重複表示の有無(実UI上での見た目)

これらは自動テスト(§6)でSystem経由の実値・ローカライズキー解決・spawn構造は
検証済みだが、実際のレイアウト崩れの有無はテストでは検出できない。

## 9. 対象外を先行実装していないこと

戦闘損耗時・部隊撃破時の`mobilized_manpower`減算、初期Division人数の
`available_manpower`からの減算、損耗兵員の`available_manpower`への返還、
population減少、初期population変更、人的資源基準上限の変更、
`available + mobilized <= cap`の強制、初期`available_manpower = 100,000`の
再調整、個別Division復員、国家全体の動員解除、補充、Attrition、AI募兵、
戦死者/負傷者/捕虜の内訳、戦闘式・撤退式の変更、未使用の
`war::combat::process_combat`への接続、新規セーブフィールド、セーブV2は、
いずれも変更ファイル一覧(§5)に含まれておらず、実装していないことを確認した。

**回帰修正について(対象外の範囲を超えていないことの確認)**: 実装の結果、
`tests/p23_001_population_manpower_e2e_test.rs`・
`tests/p23_002_mobilized_manpower_workforce_e2e_test.rs`・
`tests/p23_004_economic_workforce_ratio_e2e_test.rs`の計5件の既存テストが、
「New Gameは`mobilized_manpower == 0`から始まる」という(本タスクが修正した
バグに暗黙に依存していた)前提でFAILEDした。これらはassertを弱めたり
削除・ignore化したりせず、各テストの本来の意図(「動員なしなら」「動員解除後は」
「他国の動員は影響しない」等)を保ったまま、期待値・前提条件を「明示的にゼロへ
揃える」または「実際の按分結果を正規Domain関数で計算し直す」方向へ修正した
(実装コードの数式やテストのassert自体を弱める修正は行っていない)。

## 10. 残課題または後続候補

P23-006の完了報告(§9)に記載した以下は本タスクでも未着手のまま:

* 個別Divisionの復員(人的資源返還)
* 補充(reinforcement)
* Attrition(非戦闘損耗)
* AI募兵
* 人口と恒久軍事損失の将来統合(P23-005が示した丸め逆説を踏まえた別設計が必要)
* 人的資源基準上限の不変条件(`available + mobilized <= cap`)は依然として
  強制されない(DESIGN UNDECIDED)

新規に発見した事項はない(本タスクはP23-006完了報告の残課題を解消する修正であり、
その過程で新たな未解決の設計課題は見つからなかった)。

## 11. 実装コミット直後と報告書コミット後の`git status --short`

### 実装コミット(`4927c5050201769af8ef261c7aa2c7727f566f85`)直後

```
 M saves/savegame_v1.ron
```

`saves/savegame_v1.ron`はユーザーの既存未コミット変更であり、実装コミットには
含めていない。それ以外の全変更(§5の実装・テスト・ローカライズファイル)はコミット済み。

### 報告書コミット後

報告書コミット作成後の最終`git status --short`は、コミット完了後の最終回答で報告する。
