# P28-002-PRE-002 長期E2Eテストの暦修正

P28-002-PRE-001(`0d77acc`)で、戦時MilitaryFactory建設(候補C)の一時実装を有効にすると、6つの長期E2Eテストバイナリの15件が`src/app/time.rs:127`でpanicした。原因は、これらのハーネスが通算の月番号をそのまま月として発行し、13月以降を生成していたことにある。本タスクでは、そのテスト用の暦生成だけを実在の年月(1〜12月、12月の次は翌年1月)へ修正した。本番Rustコード・RONデータ・Save形式は変更していない。

## 1. 判定

```text
P28-002-PRE-002 COMPLETE
```

- 対象6ファイルの暦生成を、共通の純粋関数`sim_calendar_date`による実在の年月へ修正した。1か月=30日、月次tickの位置、総日数は維持した。
- 修正後、現行HEADの挙動で対象6ファイル・全体テストが通過した(§9)。
- 候補Cの一時パッチを再適用して再検証した結果、15件のpanicはすべて消えた。14件は通過し、残る1件(`p27_002_c` `test_05`)は候補Cによる**正規値の変化**である(§6)。

## 2. 開始HEADと修正前の作業ツリー

- 開始HEAD: `0d77accc1fa994c2ebb4615834548133c91578ce`(期待値と一致)
- 開始時`git status --short`: 空
- 開始時の全体テスト基準: P28-002-PRE-001の最終実測値 2708 passed / 0 failed / 1 ignored、clippy警告0(以後、本番コードの変更はなく、HEADはその報告書のコミットだけ)

## 3. 対象6ファイルと修正内容

| ファイル | 修正前の基準(個別実行) | 変更 |
|---|---|---|
| `tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs` | 11 passed | `tick_day`/`tick_month`で年月へ変換。`test_k1`の評価日を本番の式から算出(§5.2)。評価日算出ヘルパー1つを追加 |
| `tests/p26_008_c_supply_chain_construction_priority_e2e_test.rs` | 18 passed | `tick_day`/`tick_month`で年月へ変換 |
| `tests/p27_002_a_ai_construction_fiscal_reserve_e2e_test.rs` | 17 passed | 同上 |
| `tests/p27_002_b_ai_tax_rate_adjustment_e2e_test.rs` | 18 passed | 同上 |
| `tests/p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs` | 16 passed | 同上。古いコメント(「実カレンダーの年/月境界処理は対象外」)を更新。年跨ぎの直接検証`test_17`を追加 |
| `tests/p27_004_idempotent_movement_orders_e2e_test.rs` | 6 passed | `tick_day`/`tick_month`で年月へ変換 |
| `tests/common/sim_calendar.rs`(新規) | — | 共通の純粋関数`sim_calendar_date`だけを置く(各ファイルから`#[path = "common/sim_calendar.rs"] mod sim_calendar;`で取り込む。`tests/`直下ではないため独立したテストバイナリにはならない) |

各ファイルの呼び出し側(`advance_one_month`、`advance_months`、個別の`tick_day(&mut app, 1800, 36, 1)`など)は変更していない。`tick_day(app, base_year, month_index, day)`と`tick_month(app, base_year, month_index)`の2つの低水準ヘルパーの先頭で変換するだけである。状態を持たない関数なので、テスト間で共有される隠れた状態はない。

## 4. 誤った暦の生成式と修正後の式

### 4.1 修正前

6ファイルとも同じ簡略暦を使っていた。

```text
advance_one_month(idx): day 1..=30 について DayChanged{year: 1800, month: idx, day}
                        の後に MonthChanged{year: 1800, month: idx.wrapping_add(1)}
advance_months:         idx = 1, 2, …, N(p27_002_c・p27_004は start から start+N-1)
```

年は常に1800のまま、`month`に通算番号(最大121、`p27_004`のsave分岐では131)がそのまま入っていた。現行HEADでは講和が起きないため表面化しなかったが、戦争が決着すると`war::peace`の休戦終了日計算`GameDate::from_string("1800/14/xx").add_days(1825)`が`DAYS_IN_MONTH[13]`へアクセスしてpanicする。

### 4.2 修正後

```text
sim_calendar_date(base_year, idx) = (base_year + (idx - 1) / 12, (idx - 1) % 12 + 1)   // idx >= 1
DayChanged{year, month, day}  ← (year, month) = sim_calendar_date(1800, idx)、day 1..=30
MonthChanged{year, month}     ← (year, month) = sim_calendar_date(1800, idx + 1)
```

1か月=30日、月次tickは各月の30日目の後に1回、総日数(N×30)は修正前と同じである。月は常に1〜12になる。

### 4.3 同型の欠陥を持つ他のファイル(本タスクでは未修正)

同じ「年1800固定・通算の月番号」の簡略暦で13か月以上を回すファイルが、対象6ファイル以外に8件ある。`p26_004_pre_004`(36か月)、`p26_004_pre_005`(18か月)、`p26_det_001_construction_ai_determinism`(36か月)、`p26_det_001_rev_001_cross_state_production_order`(36か月)、`p26_004_daily_supply_consumption_allocation`(36か月)、`p26_005_supply_combat_strength`(36か月)、`p26_006_capital_connectivity_supply`(36か月)、`p23_010_phase23_manpower_lifecycle_audit`(静的確認では最大6か月程度)。P28-002-PRE-001の影響計測で候補Cを有効にしたとき、これら8件は失敗しなかった。講和が起きなければpanicには至らないが、同じ潜在欠陥を持つ。P28-002で戦争・講和がこれらのハーネス内でも起きる場合は、同じヘルパーへの置き換えが必要になる(§8)。

## 5. 年跨ぎ・月次発火・最終日付

### 5.1 直接検証(`p27_002_c` `test_17_harness_calendar_rolls_over_years_with_valid_months`)

このファイル自身の`advance_months`を、日次・月次メッセージを記録するだけの最小Appで120か月実行し、実際に発行されたメッセージ列を検証した。

| 項目 | 結果 |
|---|---|
| 日次メッセージ数 | 3600(=120×30、修正前と同じ) |
| 月次メッセージ数 | 120(修正前と同じ) |
| 月の範囲・日の範囲 | 月は全件1〜12、日は全件1〜30 |
| 重複・欠落 | 期待列と完全一致(同じ日の二重実行・日の飛ばしなし) |
| 年跨ぎ | 前日`D 1800/12/30` → 当日の月境界`M 1801/01` → 翌日`D 1801/01/01` |
| 36か月時点の月境界 | `M 1803/01`(36回目の月次メッセージ) |
| 120か月の最終日付 | 最後の日次`D 1809/12/30`、最後の月境界`M 1810/01` |
| save→load相当の分割実行 | 1〜60か月+61〜120か月の分割実行が、連続実行と同じ日付列 |
| 純粋関数の境界 | 12→(1800,12)、13→(1801,1)、131→(1810,11)(`p27_004`のsave分岐の最終月境界) |

各ハーネスの最終日付(修正後): 36か月テストは最終月境界が`1803/01`、120か月テストは`1810/01`、`p27_004`の120+10か月のsave分岐は`1810/11`。

### 5.2 本番の日数カウンタとの関係(`test_k1`の修正)

本番の国家AI・軍事AI(`country::country_ai::handle_daily_country_ai`、`war::military_ai`)は、日次メッセージから`current_day_num = year × 365 + month × 30 + day`を作り、月次評価を`current_day_num % 30 == country_id % 30`の日に行う。修正前の暦は年が固定で月番号が増え続けたため、このカウンタは毎日ちょうど1ずつ増えた。修正後は、ハーネスの1年が360日(30日×12)であるのに対し式は365日を仮定するため、**年を跨ぐたびにカウンタが+6進み、月次評価日が5日ずれる**。

この影響を受けた既存テストは1件だった。

- `p26_004_pre_002` `test_k1_elfin_mine_growth_is_correctly_reserve_gated_while_existing_supply_stays_positive`: 「36か月目の1日がElfinの建設評価日」と仮定してday 1で判断理由を読んでいた。修正後の36か月目は1802年12月で、評価日は**21日**になる。day 1では前日までの理由(`WarInProgress`)が読まれ、`FundsShortage`の一致アサーションが失敗した。
- 修正: 評価日を本番と同じ式で求めるヘルパー`country_ai_monthly_eval_day(month_index, country_id)`を追加し、その日まで進めてから理由を読むようにした。**アサーション(`FundsShortage`との一致、Mine levelが初期値のまま、MilitarySupplies>0)は1文字も変えていない**。修正後、評価日(21日)に読んだ理由は`FundsShortage`で、3つのアサーションはすべて通過した。

固定の評価日(`DWARF_MONTHLY_EVAL_DAY = 2`)を使う他のヘルパーは、`p26_008_c`・`p27_002_a`・`p27_002_b`・`p27_002_c`のいずれでも1〜3か月目だけで使われており、年跨ぎ前なので影響はない(静的確認と実行で確認)。

## 6. 15件のpanicの再検証

P28-002-PRE-001で保存した一時パッチ(環境変数で有効化、本番コードのみ)を再適用し、`P28_C_MODE=war P28_C_RESERVE_MONTHS=1`で対象6バイナリを個別に実行した。実験後はパッチを逆適用し、4つの本番ファイルの`git hash-object`がHEADのblobと一致することを確認した。

| バイナリ | P28-002-PRE-001でpanicしたテスト | 暦修正後の候補C有効時 |
|---|---|---|
| `p26_004_pre_002` | `test_dwarf_supply_chain_is_not_disturbed_by_the_proactive_rule` | 通過 |
| `p26_008_c` | `test_90_day_and_36_month_six_country_real_data_verification` | 通過 |
| `p27_002_a` | `test_17_real_six_country_90_day_and_36_month_fiscal_reserve_verification` | 通過 |
| `p27_002_b` | `test_18_real_six_country_90_day_and_36_month_tax_adjustment_verification` | 通過 |
| `p27_002_c` | `test_05_real_six_country_90_day_36_60_120_month_long_horizon_verification` | **失敗(正規値の変化)** |
| `p27_002_c` | `test_06`、`test_07`、`test_09`、`test_11`、`test_12`、`test_13` | 6件とも通過 |
| `p27_004` | `test_03`、`test_04`、`test_05`、`test_06` | 4件とも通過 |

分類: **panic 0件、正規値の変化1件、実際の回帰0件**(対象6バイナリ内)。候補C有効時の6バイナリ合計は86件中85 passed / 1 failed(新規`test_17`を含めると87件中86 passed)。決定性(`test_11`)、登録順非依存(`test_12`)、国家間分離(`test_13`)、60か月でのsave→load(`test_09`)、プレイヤー税率の不変(`test_07`)は、候補C有効時も通過した。

候補Cが実際に動作していることは、`--nocapture`で一時パッチの診断出力を確認した(例: Dwarfが州20にMilitaryFactory lv1・lv2、Oceanicが州25にlv1を建設。Arcadiaは入力不足、Ferrowynは資金不足で見送り)。通過したテストの標準エラー出力はcargo testが捕捉するため、通常のログには表示されない。

### 6.1 残る1件の内容(P28-002へ申し送り)

`test_05`は、Arcadiaの120か月の税率振動回数を監査値`0`と完全一致で比較している(受け入れ基準の上限3回とは別の、厳密な回帰保護)。候補C有効時は`2`になった(税率変更5回)。受け入れ基準(≤3)は満たす。このアサーションの後にある120か月時点の正規値(税率・国庫・月次収支の完全一致)も、候補C有効時には次のように変わる(テストの出力から実測)。

| 国 | 税率(現行→候補C) | 国庫(現行→候補C) | 月次収支(現行→候補C) | 生活水準(現行→候補C) |
|---|---|---|---|---|
| Arcadia | 0.140→0.160 | 8340.77→4429.05 | 42.85→19.20 | 60.38→51.72 |
| Elfin | 0.120→0.120 | 6425.41→5539.38 | 24.31→24.07 | 70.95→70.95 |
| Dwarf | 0.220→0.170 | 6170.62→9804.62 | 35.69→141.59 | 41.11→54.50 |
| Oceanic | 0.200→0.190 | 10778.56→11284.74 | −16.42→107.41 | 41.40→45.61 |
| Passguard | 0.190→0.190 | 904.06→904.06 | 12.43→12.43 | 67.01→67.01 |
| Ferrowyn | 0.190→0.200 | 1960.52→693.24 | 18.22→17.97 | 51.42→52.48 |

国庫が負になる月(`first_negative_treasury_month`)は現行・候補Cとも同じ(Passguardの7か月目のみ、既存事象)。現行の値は、暦修正後の現行HEADでも既存の正規値と完全一致した(`test_05`が通過)。

## 7. 既存アサーション・正規値への影響

- 暦修正そのものによる正規値の変化: **なし**。対象6ファイルの既存86件はすべて通過し、`test_05`の120か月の税率・国庫・月次収支・振動回数は既存の正規値と完全一致した。
- 削除・緩和したアサーション: **なし**。
- テストの手順を変えたもの: `test_k1`の1件(§5.2)。読む日をday 1固定から本番の式による評価日へ変えただけで、比較値は同じ。
- 追加したテスト: `p27_002_c` `test_17`の1件。

## 8. save→loadと決定性

- save→load分岐を持つテスト(`p27_002_c` `test_09`、`p27_004` `test_05`/`test_06`、`p26_004_pre_002`の分岐テスト)は、暦修正後も連続実行と一致した。`test_17`では、分割実行と連続実行の日付列が同一であることを直接確認した。
- 独立プロセスで対象6バイナリを再実行し、テスト一覧と結果が一致した。`test_05`の120か月の出力は2つのプロセスでバイト単位で一致した。

## 9. P28-002実装へ残す確認事項

1. `p27_002_c` `test_05`の振動回数(Arcadia 0→2)と120か月の正規値(§6.1)が、候補Cによって意図どおりに変わること。元テストを更新し、必要なら狭い新規テストへ分割して、変化の理由を説明する。
2. 候補Cの一時パッチでは対象6バイナリだけを再検証した。本実装では全体テストを実行し、§4.3の8ファイルを含めて、講和による暦panicや正規値の変化が新たに出ないか確認する。出た場合は同じ`sim_calendar_date`へ置き換える。
3. 本番の`GameDate::from_string`は月の範囲を検証せず、`GameDate::add_days`は13以上の月でpanicする(P28-002-PRE-001 Low L-4)。実Appの日付は常に正しいため本タスクでは触れていない。
4. 本番の日数カウンタ`year × 365 + month × 30 + day`は、実カレンダー(28〜31日の月)でも、30日の簡略暦の年跨ぎでも連続しない(本タスクで年跨ぎ時の+6を確認)。月次評価日の位置を固定値で仮定するテストは、今後も本番の式から求めること。
5. P27-005の手動プレイ確認は未実施のまま引き継ぐ。

## 10. 実行コマンドと正確な結果

Cargoはすべて逐次実行した(並列起動なし)。

- `git rev-parse HEAD` → `0d77accc1fa994c2ebb4615834548133c91578ce`、`git status --short` → 空
- 修正前の個別実行(`cargo test -j 2 --test <name>`): `p26_004_pre_002` 11 passed、`p26_008_c` 18 passed、`p27_002_a` 17 passed、`p27_002_b` 18 passed、`p27_002_c` 16 passed、`p27_004` 6 passed(いずれも0 failed)
- 暦修正の直後(`test_k1`修正前)の個別実行: `p26_004_pre_002` 10 passed / 1 failed(`test_k1`、§5.2)、他の5ファイルは全件通過(`p27_002_c`は`test_17`を含め17 passed)
- `test_k1`修正後: `p26_004_pre_002` 11 passed
- 候補C一時パッチ有効時(`P28_C_MODE=war P28_C_RESERVE_MONTHS=1`)の個別実行: `p26_004_pre_002` 11 passed、`p26_008_c` 18 passed、`p27_002_a` 17 passed、`p27_002_b` 18 passed、`p27_002_c` 16 passed / 1 failed(`test_05`)、`p27_004` 6 passed。`test_05`は環境変数あり・なしを各2回実行し、ありは2回とも失敗、なしは2回とも通過(決定的)
- 一時パッチ除去後の個別実行(独立プロセスでの再実行): 6ファイルとも全件通過(11 / 18 / 17 / 18 / 17 / 6)
- 最終 `cargo test --all-targets --no-fail-fast -j 2` → **2709 passed / 0 failed / 1 ignored**(82バイナリ、exit 0)。開始時の2708に新規`test_17`の1件が加わった。1 ignoredは既存の`profile_p21_021_elimination_scaling`。再生成された既知の非決定的PNG9件は個別に復元
- 最終 `cargo clippy --all-targets --all-features -- -D warnings` → 警告0(exit 0)

## 11. 本番コード・データ・ユーザーSaveの保護

- 本番Rustコード・RONデータ・Save形式・System順序・AI判定: 変更なし。候補Cの一時パッチは再適用後に逆適用し、`src/country/country_ai.rs`・`src/military/ai.rs`・`src/war/military_ai.rs`・`src/war/offensive_supply_gate.rs`の`git hash-object`がHEADのblobと一致することを確認した。
- `saves/savegame_v1.ron`: 読み書き・復元・ステージなし。
- `tests/land_war_combat_peace_test.rs`: 変更・整形なし。ワークスペース全体のformatなし。
- 既存報告書: 変更なし。
- テスト実行で再生成された既知の非決定的PNGは、`git status --short`で確認して1件ずつ個別に復元した。

## 12. 最終`git status --short`

コミット前: 対象6テストファイルの`M`、`?? tests/common/`、`?? verification_logs/phase-28/p28-002-pre-002/`。コミット後は空。
