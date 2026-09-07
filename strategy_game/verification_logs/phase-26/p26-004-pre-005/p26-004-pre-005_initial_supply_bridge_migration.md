# P26-004-PRE-005: 初期MilitarySuppliesブリッジ付与と旧Save意味的移行 実装報告書

## 1. 総合判定

**COMPLETE**

New Gameの現行6か国全てが、実際のDivision構成・実カレンダーから導出した正規計算により
一律31.0のMilitarySuppliesブリッジ備蓄を得る。31.0はCountryId別の固定値ではなく、
共通関数(`src/military/supply_initialization.rs`)がDivision需要とカレンダーから毎回
導出する。旧Saveへは同じ共通関数を使った一度限りの意味的migrationを適用し、Save V1の
構文互換性を維持したまま、月途中の残日数相当だけをtop-upする。原子性・冪等性・決定性は
すべて恒久テスト(単体55件+E2E22件)で検証済み。既存の関連E2Eテスト(P26-002/003/
REV-002・P26-DET-001/REV-001)のうち、New GameのMilitarySupplies初期値が0.0であることを
前提としていた7件は、新しい正規挙動(31.0のブリッジ付与)を反映するよう意図的に更新した
(弱体化ではなく、実際の仕様変更を反映する更新)。全テスト0 failed、clippy 0 warnings。

## 2. 開始時HEAD

```text
b09c3b1a89a55501b1a3684fce050b3a50ecb713
```

`git rev-parse HEAD`・`git status --short`・`git show --stat --oneline HEAD`で確認し、
想定(`fix: make national production order deterministic`、P26-DET-001-REV-001のコミット)
と一致していた。既存差分は` M saves/savegame_v1.ron`のみで、内容は一切参照していない。

## 3. 監査対象HEAD

`b09c3b1a89a55501b1a3684fce050b3a50ecb713`(開始時HEADと同一。本タスクはこのコミットの
上に作業ツリー上で実装し、末尾に1コミットとして記録する)。

## 4. New Gameの正規初期化方式

`src/app/loader.rs`の`OnEnter(GameState::Playing)`チェーンへ、新規System
`grant_initial_military_supplies_bridge`(`src/military/supply_initialization.rs`)を
`spawn_debug_divisions`の直後・`crate::country::power::rebuild_country_power_on_enter_playing`
の前に挿入した:

```rust
(
    spawn_debug_divisions,
    crate::military::supply_initialization::grant_initial_military_supplies_bridge,
    crate::country::power::rebuild_country_power_on_enter_playing,
    reset_playing_entry_mode,
)
    .chain(),
```

`spawn_debug_divisions`と同じ`PlayingEntryMode::NewGame`ガードを内部で持つため、
Loaded Game進入では実行しない。国家を`CountryId`昇順に処理し、各国家ごとに共通関数
`apply_military_supplies_bridge_grant_for_country`を1回だけ呼ぶ。`countries.ron`へ31.0を
直接書き込んでいない――全ての値は`MilitaryRegistry`の実際のDivision構成と
`Res<GameDate>`の実際の日付から都度計算される。

## 5. 旧Saveの意味的移行方式

`src/save/apply.rs::prepare_load`の`mine_migration`・`supply_chain_migration`呼び出しの
直後に、`apply_military_supplies_bridge_migration`(`src/military/supply_initialization.rs`)
の呼び出しを追加した。引数は`&mut save.countries`・`&save.military.divisions`・
`&preserved_division_definitions`(§4で既に複製済みの静的マスターデータ)・
`save.date.day`/`save.date.month`。Worldへは一切触れない、失敗し得ない純粋なデータ変換
として、mine_migration/supply_chain_migrationと同じ隣接位置に置いた。国家を`CountryId`
昇順で1か国ずつ処理し、New Gameと全く同じ`apply_military_supplies_bridge_grant_for_country`
を呼ぶため、両経路の論理的構造は完全に一致する。

## 6. Save V1／V2判断

**Save V1を維持した(形式番号は変更していない)**。新規テスト
`test_v1_syntax_compatibility_no_new_save_format_version`で
`SaveGameV1.version == SAVE_FORMAT_VERSION_V1 == 1`であることを確認済み。ただし
「migrationなし」ではない――旧Saveの論理状態(マーカー未設定・備蓄0または不完全)を
New Gameと同じ論理状態(ブリッジ備蓄充足済み)へ一度だけ書き換える、明示的な**意味的
migration**である。

## 7. マーカーフィールドとserde規則

`CountryData`(`src/country/mod.rs`)へ以下を追加した:

```rust
#[serde(default)]
pub military_supplies_initial_grant_applied: bool,
```

旧Save(フィールド欠落)は`#[serde(default)]`によりfalseとして復元される
(`old_format_ron_without_military_supplies_grant_marker_defaults_to_false`で確認)。
`military_supplies_grant_marker_round_trips_through_ron`でtrue/false双方のRON往復も
確認した。

## 8. マーカーの保存単位

国家単位(`CountryData`に1フィールド)。国家ごとに独立して判定・更新される
(`one_countrys_failure_does_not_disturb_another_countrys_migration`で確認)。

## 9. 日次需要の正規式

```text
daily_demand = definition.supply_usage × current_manpower / max_manpower × activity_multiplier
```

本タスクの`activity_multiplier`は常に`1.0`(`BASELINE_ACTIVITY_MULTIPLIER`定数)。
待機・移動・戦闘等による倍率差はP26-004本体で導入する。

## 10. fixed-point計算方式

```text
usage_units = Q(definition.supply_usage × activity_multiplier)   // P26-002のmilitary_supplies_value_to_units
demand_units = floor(usage_units × current_manpower / max_manpower)   // u128 checked_mul → 整数除算(floor)
```

`usage_units`はP26-002の量子化・値域検証(NaN/Infinity/負数/範囲外を明示的に拒否)を経る。
乗算は`u128::checked_mul`、国家合計は`u128::checked_add`。最後まで生のf64加算は行わない。
`saturating_*`はDivision需要計算内では一切使用していない(overflow境界の単体テスト
`checked_scaled_demand_units_never_overflows_at_the_u64_boundary`・
`checked_add_demand_reports_overflow_at_the_u128_boundary`で、checked演算が実際に機能する
ことを確認)。完全編制Infantry(supply_usage=1.0)は`daily_demand_units=1_000_000`
(=1.0)になることを`full_strength_infantry_division_demands_exactly_one_million_units`
で確認した。

## 11. カレンダーとSystem順序

`src/app/time.rs::advance_game_date`を実コードで監査した。New Game開始日は
`GameDate::default()`(1800/01/01)。日が進むたびに`DayChangedMessage`が発行され、月が
繰り上がる当日は同一イテレーション内で`DayChangedMessage`と`MonthChangedMessage`が
両方発行される。資源生産・消費は`handle_monthly_economy`(`MonthChangedMessage`購読)
内でのみ動く。

## 12. 次回生産までの消費イベント数

```rust
events_before_next_monthly_production(day, days_in_month) = days_in_month - day
```

現在日(`day`)自体は消費回数へ含めない(この時点で確定済みの日として扱う)。
`day+1`から`days_in_month`まで(`days_in_month - day`回)は確実に月境界より前の
`DayChangedMessage`である。New Game開始日(day=1, 1月=31日)では`31-1=30`。

## 13. 1日分予備の根拠

`days_in_month`の次の1回(月が繰り上がる当日)は、その同一イテレーション内で
`MonthChangedMessage`も発行されるため、その日の消費が生産の前か後かは実行順序に依存し
確定しない「境界日」である。この1日分を「予備」として`events + 1`という形で常に加算し、
不確定な順序リスクを吸収する。

## 14. New Gameで31.0になる導出

```text
daily_demand_units = Q(1.0 × 1.0) × 10000/10000 = 1_000_000   (満編制Infantry1個師団)
events = days_in_month(1) - day(1) = 31 - 1 = 30
bridge_units = daily_demand_units × (events + 1) = 1_000_000 × 31 = 31_000_000
             = 31.0(value換算)
```

全6か国が同一の`STANDARD_INFANTRY_DEFINITION_ID`・同一の初期manpower(満編制)・同一の
`DivisionStatus::Idle`で始まるため(`spawn_debug_divisions`の共通コードパス)、6か国とも
同じ31.0になる。これは共通関数がDivision構成から都度計算した結果が偶然一致したもので
あり、固定値分岐ではない――将来初期編制が国家ごとに異なるようになれば、共通関数は
自動的に異なる値を返す。

## 15. 旧Saveの月途中計算

```text
required_bridge_units = daily_demand_units × (events_before_next_production + 1)
```

`events_before_next_production`は`save.date.day`/`save.date.month`から都度計算する
(§12と同じ関数)。例: 1月20日(31日の月)なら`events=31-20=11`、
`required=(11+1)=12.0`(`mid_month_old_save_grants_the_remaining_days_worth`・
`test_c5_old_save_mid_month_tops_up_the_remaining_days_worth`で確認)。

## 16. top-up式

```text
grant_units = max(0, required_bridge_units - current_stockpile_units)
```

実装は`required_bridge_units.saturating_sub(current_units as u128)`(§36参照:
`checked_sub().unwrap_or(0)`と数学的に同一だが、clippy `manual_saturating_arithmetic`
の指摘に従いidiomaticな形を採用した。これはspec §5.1が禁止する「本来checked演算で
エラー検出すべき箇所をsaturatingで握り潰す」パターンとは異なる――ここでの
`current > required`は不正状態でもoverflowでもなく、「既存備蓄を減らさない」という
仕様そのものの下限クランプである)。

## 17. 正の既存備蓄の扱い

```text
必要量31.0、現在10.0 → 21.0付与、最終31.0(test_c2)
必要量31.0、現在31.0 → 付与0、最終31.0、マーカーtrue(test_c3)
必要量31.0、現在50.0 → 付与0、最終50.0(減らさない)、マーカーtrue(test_c4)
```

無条件に31.0を加算する経路は一切実装していない――常に`max(0, required - current)`の
差分だけを加算する。

## 18. マーカーtrue・備蓄0の扱い

`apply_military_supplies_bridge_grant_for_country`の最初の分岐でマーカーを確認し、trueなら
即座に`AlreadyGranted`を返して以降の計算を一切行わない。備蓄が0であっても再計算・
再付与しない(`marker_true_with_zero_stockpile_never_regrants`・E2E
`test_a3_reinvoking_the_grant_system_does_not_double_grant`で確認)。

## 19. 消滅国家の扱い

`!country.is_active()`(P21-021の既存判定)ならDivision需要計算・カレンダー計算を
一切行わず、`grant=0`・マーカーtrueとして即座に返す
(`eliminated_country_gets_zero_grant_and_marker_true`、E2E
`test_c8_eliminated_country_gets_no_grant_but_marker_becomes_true`で確認)。既存備蓄が
あってもtop-upしない(消滅国家は正規化された需要を持たないため)。

## 20. Divisionなし国家の扱い

特別分岐は設けていない――`country_daily_demand_units`が保有Divisionを`CountryId`で
フィルタするため、0件なら合計は自然に0となり、`required_bridge_units=0`、
`grant_units=max(0, 0-current)=0`、マーカーtrueという結果が式の自然な帰結として得られる
(`active_country_without_any_division_gets_zero_required_and_marker_true`、E2E
`test_e2_active_country_without_any_division_gets_zero_and_marker_true_via_migration`)。

## 21. 不正Division・不正備蓄の扱い

`SupplyDemandError`(`ManpowerExceedsMax`/`MaxManpowerZeroWithPositiveManpower`/
`UnknownDefinition`/`InvalidSupplyUsage`/`Overflow`)・`CalendarError`
(`InvalidMonth`/`DayOutOfRange`)・`BridgeGrantError`(上記2つ+
`InvalidCurrentStockpile`/`InvalidRequiredOrGrantValue`/`Apply`)を通じて、不正状態は
黙って需要0扱いにせず明示的に`Err`を返す。呼び出し元(New Game System・旧Save
migration関数)は`warn!`でログを出しつつ、その国家の備蓄・マーカーを一切変更せず処理を
続ける(他国のmigrationを妨げない)。panicは一切発生しない。

## 22. 初期付与API

`CountryStockpile::try_grant_military_supplies(&mut self, grant: f64) -> Result<(), MilitarySuppliesUnitsError>`
(`src/economy/resources.rs`)を新設した。既存の`add_military_supplies_produced`を
「初期化時のgrant」目的でそのまま呼ぶことは避け、意味の異なる専用APIとした。両者は
内部の同一検証ロジックを`checked_military_supplies_addition`という非公開ヘルパーへ
括り出して共有する(read→validate→units変換→checked_add→上限検証→f64正規変換→
往復検証、の1つの実装)が、公開シグネチャ・関数名・doc commentは完全に分離している。
生の`stockpile.add`は使用していない。

## 23. 原子性境界

国家単位。`apply_military_supplies_bridge_grant_for_country`が1国家分の
「読取→Division需要検証→bridge units計算→現在備蓄検証→grant units計算→
付与後備蓄検証(`try_grant_military_supplies`内)→備蓄commit→marker=true」を1つの
関数として実行し、途中で失敗した場合は`country`(備蓄・マーカーとも)を一切変更しない。
旧Save側は、国家ごとに独立してこの関数を呼ぶ設計(`supply_chain_migration`が採用した
「対象州が1件でも欠落したら全体を拒否」という世界全体の原子性境界とは意図的に異なる)を
採用した――理由は、各国家のgrantが完全に独立しており、1か国の異常値(不正な
Division・不正な備蓄など)が他国の意味的migrationを妨げるべきではないため。マーカーが
立たなかった国家は、次回のロード/New Game再試行で自動的に再計算される。

## 24. 世界保存式

```text
Σstockpile_after = Σstockpile_before + Σaccepted_initial_grants
```

初期付与は生産量に含めず、独立した外生的grantとして扱う。
`world_conservation_matches_sum_of_accepted_grants`(単体)・
`test_f1_world_total_after_new_game_grant_equals_sum_of_six_grants`(E2E、186.0=31.0×6)
で確認した。

## 25. 国家間分離

`country_daily_demand_units`はDivisionを`owner==country_id`でフィルタするため、他国の
Divisionへ一切依存しない(`countries_are_isolated_from_each_other`)。ある国家の
grant失敗が他国のgrant成功を妨げないことを
`one_countrys_failure_does_not_disturb_another_countrys_migration`で確認した。

## 26. 決定性

* Division合計はDivisionId昇順にソートしてから合算する(`HashMap`走査順に非依存、
  `division_insertion_order_does_not_change_the_total`)。
* Definition参照はHashMap経由だが挿入順に依存しない
  (`definition_insertion_order_does_not_change_the_total`)。
* 国家処理順はCountryId昇順の明示的な全順序
  (`migration_over_multiple_countries_is_deterministic_regardless_of_slice_order`)。
* P26-DET-001/REV-001が確定させたArcadia等の正規値(§34参照)は、本タスクの初期
  31.0付与を実測値から差し引いた「生産分のみ」として比較することで無傷のまま保護した。

## 27. New Game／Loaded Game比較

`test_e1_new_game_and_migrated_old_save_agree_on_the_same_logical_state`で、New Gameの
正規付与値(31.0)と、同じDivision構成・同じ日付(1800/01/01)から旧Save migration経由で
導出した値が完全に一致することを確認した。

## 28. save/load分岐

`test_d3_month_crossing_save_load_is_stable`(月境界を跨いだ後のsave/loadで値が完全一致)、
P26-DET-001-REV-001側の`test_k_save_load_branch_matches_continuous_run_at_month_36`
(継続実行と分岐実行の月36一致、生産分のみで比較)で確認した。

## 29. 二重付与防止

* New Game: マーカーが国家単位で一度立てば、同じSystemを実App上で再実行しても
  再付与しない(`test_a3_reinvoking_the_grant_system_does_not_double_grant`、
  `bevy::ecs::system::RunSystemOnce`で実際にSystemを直接再実行して確認)。
* 旧Save: `migration_applying_twice_stays_idempotent`(単体)、
  `test_d1_save_load_twice_in_a_row_is_stable`・
  `test_d2_old_save_migration_then_resave_then_reload_never_double_grants`(E2E、
  save→load→save→loadの2周を検証)で確認した。

## 30. Save V1構文互換性

`#[serde(default)]`の新規フィールド1件のみを追加(既存のP21-021`status`パターンの
直接踏襲)。Save形式番号は無変更。§7・§30・`test_v1_syntax_compatibility_no_new_save_format_version`
で確認済み。

## 31. 意味的migrationの結果

旧Saveの論理状態(鉱床・grantマーカー未設定)を、New Gameと同じ論理状態
(ブリッジ備蓄充足済み)へ一度だけ書き換える意味的migrationとして正しく機能することを
§27・E2E Category C/D/Eで実証した。

## 32. 日次消費を先行実装していないこと

`try_consume_military_supplies`への新規呼び出しは追加していない
(`rg -n "try_consume_military_supplies" src`が`src/economy/resources.rs`の定義本体
以外に0件であることを確認済み)。`test_a2_stockpile_unchanged_by_daily_ticks_before_first_monthly_production`
・既存の`test_f4_supplies_never_decrease_across_ticks_since_nothing_consumes_yet`
(P26-003、無変更のまま合格)で、日次Tickだけでは備蓄が減少しないことを確認した。

## 33. P26-004-PRE-004保護

`assets/data/resources.ron`・`assets/data/states.ron`・
`src/building/supply_chain_migration.rs`はいずれも本タスクで変更していない。
`cargo test --test p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test`
(15 passed、無変更)・`cargo test --test p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test`
(11 passed、無変更)で確認した。

## 34. P26-DET-001／REV-001保護

`src/economy/production.rs`・`src/country/country_ai.rs`はいずれも変更していない。
既存の2ファイル(`p26_det_001_construction_ai_determinism_e2e_test.rs`・
`p26_det_001_rev_001_cross_state_production_order_e2e_test.rs`)にある4件のArcadia/
6か国比較テスト(test_e/test_f/test_h/test_i)は、New Gameが導入する初期31.0を実測して
差し引いた「生産分のみ」を、既存の正規値定数(421.2265230000/592.5978000000/
667.8538710000/822.4577790000など)とそのまま突き合わせるよう更新した――正規値定数
自体は1つも変更していない。全12+10テストがpassすることを確認した(§37参照)。

## 35. Phase 23～25保護

`cargo test --lib country::`(89 passed、baseline通り)・`cargo test --lib economy::`
(39 passed = baseline12+新規P26-002系20+新規grant系7)・`cargo test --lib save::`
(300 passed、baseline通り)を実行し、Phase 23(manpower)・Phase 24(workforce)・
Phase 25(装備会計)関連コードを一切変更していないことを確認した。

## 36. Critical／High／Medium／Low

* **Critical: 0件**
* **High: 0件**
* **Medium: 0件**
* **Low: 2件**
  1. `checked_scaled_demand_units`のchecked_mul(usage_units×manpower)は、
     `usage_units`が`MILITARY_SUPPLIES_MAX_UNITS`(10^15)以下、`manpower`が`u64`という
     現在の型境界の下では、数学的に`u128::MAX`を超え得ない
     (`u64::MAX^2 < u128::MAX`)。したがってこの箇所のOverflow分岐は現実的な
     経路からは到達不能な防御的分岐である(P26-002の`ArithmeticOverflow`と同種の
     既知パターン)。単体テストでは低レベルヘルパーへ直接大きな値を渡すことで
     checked演算自体が機能することを実証した。将来の型変更(例:manpowerが
     u128になる等)に備え、checked演算のまま維持することを推奨する。
  2. `grant_units = max(0, required - current)`の実装に`saturating_sub`を採用した
     (§16・§36の理由)。これはclippy `manual_saturating_arithmetic`の指摘に従った
     結果であり、spec §5.1が禁止する「本来checked演算でエラー検出すべき箇所を
     saturatingで握り潰す」パターンとは意味的に異なる(current > requiredは
     不正状態ではなく仕様上の正規の下限クランプ)ことを本文中に明記したが、
     将来spec文言を厳密に再解釈する場合は要再確認としてLowで記録する。

## 37. 全テスト結果

```text
開始時baseline: 2103 passed / 0 failed / 1 ignored(自ら再測定)
実装後最終:     2189 passed / 0 failed / 1 ignored(cargo test --all-targetsで実測)
```

新規テスト86件(`src/military/supply_initialization.rs`単体55件+
`src/economy/resources.rs`単体7件+`src/country/mod.rs`単体2件+
`tests/p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs`22件)がすべて
新規に合格し、2103+86=2189と算術的に完全一致する。既存テストの純減・純増はない
(7件のテスト名を更新したが、いずれも1対1のrename+assertion更新であり、テスト数は
変化していない)。

## 38. clippy結果

`cargo clippy --all-targets --all-features -- -D warnings`は実装後も**0 warnings**。

## 39. Cargo逐次実行

全てのCargoコマンドを逐次実行した(並列実行なし)。`cargo test --all-targets`の初回
実行時に一過性のOSリンクエラー(`error creating import library for rpcrt4.dll`、
Windowsのクォータ不足)が発生したため、単純に再実行して正常完走を確認した
(既存タスクが記録した既知の一過性OS resource枯渇と同種の事象、並行cargo実行は
一切していない)。

## 40. ユーザーセーブの保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージ・コミットして
いない。開始時・全検証コマンド実行後・本報告書作成時点のいずれの`git status --short`
でも` M saves/savegame_v1.ron`のみが変化せず残り続けたことを確認した(内容は一度も
参照していない)。テスト用の合成「旧Save」は全て一時ディレクトリ
(`std::env::temp_dir()`配下)に作成し、実ユーザーセーブへは一切アクセスしていない。

## 41. 変更ファイル

```text
src/app/loader.rs                                                            (New Game System挿入)
src/country/mod.rs                                                           (marker追加+単体テスト2件)
src/economy/resources.rs                                                     (try_grant_military_supplies追加+単体テスト7件)
src/military/mod.rs                                                          (新規モジュール宣言)
src/military/supply_initialization.rs                                        (新規、需要式・カレンダー・grant関数+単体テスト55件)
src/save/apply.rs                                                            (旧Save migration呼び出し追加)
tests/p26_002_military_supplies_accounting_foundation_e2e_test.rs            (New Game基準値更新、3テスト)
tests/p26_003_military_supplies_production_e2e_test.rs                       (New Game基準値更新、3テスト)
tests/p26_003_rev_002_supply_production_units_e2e_test.rs                    (New Game基準値更新、2テスト)
tests/p26_det_001_construction_ai_determinism_e2e_test.rs                    (初期grant分の差引比較、2テスト)
tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs           (初期grant分の差引比較、2テスト)
tests/p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs            (新規、E2Eテスト22件)
verification_logs/phase-26/p26-004-pre-005/p26-004-pre-005_initial_supply_bridge_migration.md  (本報告書)
```

`saves/savegame_v1.ron`はコミット対象に含めない。

## 42. 最終`git status --short`

コミット前(本報告書作成完了直後):

```text
 M saves/savegame_v1.ron
 M src/app/loader.rs
 M src/country/mod.rs
 M src/economy/resources.rs
 M src/military/mod.rs
 M src/save/apply.rs
 M tests/p26_002_military_supplies_accounting_foundation_e2e_test.rs
 M tests/p26_003_military_supplies_production_e2e_test.rs
 M tests/p26_003_rev_002_supply_production_units_e2e_test.rs
 M tests/p26_det_001_construction_ai_determinism_e2e_test.rs
 M tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs
?? src/military/supply_initialization.rs
?? tests/p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs
?? verification_logs/phase-26/p26-004-pre-005/
```

## 43. P26-004本体へ進める状態か

進める状態である。MilitarySuppliesは実際にNew Game・旧Save双方で初回月次生産までの
ブリッジ備蓄を一度限り持つようになったが、日次消費・Division配分・補給充足率・
戦闘力補正はまだ一切接続されていない(意図どおりのブリッジ付与のみの状態)。P26-004
(Division需要+日次配分)着手時は、本タスクが確立した正規日次需要式
(`division_daily_demand_units`/`country_daily_demand_units`)をそのまま再利用できる。
P26-004実装時は、月次生産と日次消費という時間粒度の非対称性(P26-003 §15で既に指摘済み)
の扱いを最初に決定する必要がある。
