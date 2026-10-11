# P28-010-IMPL-001：生産入力の原子的会計

P28-010-PRE-002 で確定した FIX1(エントリ単位の原子的クランプ)を `src/economy/production.rs` の月次生産 Step 3 に実装した。候補D(MilitaryFactory への Iron 優先配分)・Factory 増設抑制・P28-004 の募兵制限・AI の建設判断は本タスクの対象外で、変更していない。

## 0. 基準状態と判定

| 項目 | 値 |
|---|---|
| 期待HEAD | `1eea8a547f346cada4e5b40adb3e1af1a34256ee` |
| 開始時の実HEAD | `1eea8a547f346cada4e5b40adb3e1af1a34256ee`(一致) |
| 開始時 `git status --short` | 空 |
| 開始時のテスト基準 | 2794 passed / 0 failed / 1 ignored、clippy 警告0(**前回報告からの引用**。今回の実測は §9) |

```text
P28-010-IMPL-001 IMPLEMENTATION COMPLETE
UNPAID PRODUCTION RESOLVED（全6か国・240か月、資源保存則の残差 < 1e-3）
NON-REGRESSION CONFIRMED（入力引落しの失敗がなかった Dwarf・Passguard は不変）
EXISTING CANONICAL VALUES UPDATED（因果確認済み、§7）
CANDIDATE D NOT IMPLEMENTED
```

## 1. 実装

### 1.1 旧来の欠陥(再掲)

Step 2 の `sat` は月初備蓄を、稼働率係数 `t = min(労働力比, 物流比)` を掛けた需要で割った値である。Step 3 の実消費は `base × level × min(t, sat)` で `t` を掛けないため、`sat < t` の月は Σ消費が備蓄を超え得る。`CountryStockpile::consume` は全額判定で、戻り値を捨てていたため、失敗しても産出と研究力だけが加算された(P28-010-PRE-002 §2〜§3)。

### 1.2 変更内容(`src/economy/production.rs` のみ)

| 追加・変更 | 内容 |
|---|---|
| `limit_operation_ratio_to_stock(planned_op, level, inputs, stockpile)` | 計画稼働率のまま全入力が現在備蓄で賄えるなら、**そのまま返す**(従来と同一の消費・産出)。賄えなければ、最も厳しい入力に合わせた稼働率(`備蓄 / (base × level)` の最小値)へ引き下げる。`f32` への丸めで備蓄を超える場合は 1 ulp ずつ下げる(有限回で収束) |
| `consume_inputs_atomically(stockpile, inputs, level, op)` | 全入力の備蓄を事前確認し、すべて賄える場合だけ引き落とす。`consume` の戻り値を必ず扱い、途中で失敗した場合は引き落とし済みの分を戻して `false` を返す。入力は `ResourceType::ALL` 順で処理する |
| Step 3 の呼び出し | `final_op` が `0` になった(入力を1つも引き落とせない)entry は、産出・研究力を加算せず `continue`。`building_operation_ratios` には実際の稼働率を記録する |

- 入力が足りる entry は、`limit_operation_ratio_to_stock` が `planned_op` をそのまま返し、`calculate_actual_input` の結果も従来と同じため、**消費量・産出量・量子化結果はビット単位で同一**。
- 稼働率を引き下げた entry は、その稼働率で消費・産出・研究力・MS の entry ごとの量子化(`SuppliesUnitsAccumulation`)を行う。MS の quantize-then-sum(方式A)や `add_military_equipment_produced` の経路は変更していない。
- 処理順(`BuildingType::ALL` 外側 × `StateId` 昇順内側)、Mine の当月産出が後続の消費に使えること、国家間の資源分離は変更していない。
- FIX2(`op = t × sat`)は採用していない(P28-010-PRE-002 §5.3)。
- Save V1 のフォーマット・フィールドは変更していない(永続フィールドの追加なし、migration なし)。

### 1.3 量子化の境界

| 項目 | 扱い |
|---|---|
| 入力の消費量 | `f64`(`base × level × op`)。`consume` の比較は `current >= amount`。事前確認も同じ式 |
| 稼働率 | `f32`。引き下げ後の稼働率は、消費量が備蓄を超えない `f32` へ切り捨てる(1 ulp 単位の調整) |
| MilitarySupplies | entry ごとに fixed-point units へ量子化し、国家合計を1回だけ commit(従来どおり) |
| MilitaryEquipment | `add_military_equipment_produced`(units 経由、従来どおり) |
| 境界テスト | `slightly_short_inputs_never_go_negative_and_stay_consistent`(備蓄 = 需要の 0.9999999〜1e-9 倍)、`sweep_of_stock_levels_keeps_accounting_identities`(13³ = 2,197 通りの備蓄水準) |

## 2. 会計保存則

月次・国家別・資源別: `期末備蓄 = 期首備蓄 + 実際の産出 − 実際の引落し`。実際の引落しは、記録された稼働率(`building_operation_ratios`)から再構成した `base × level × op` と一致する(consume は必ず成功する)。

実データの全6か国・240か月で、Iron・Coal・IndustrialGoods・MagicCrystal の保存則の残差が全月 `< 1e-3`、備蓄は全月 `≥ 0`、入力を使う建物が稼働しているのに Iron が据え置かれる月は 0(`real_app_240_months_keeps_the_resource_ledger_closed_for_all_six_countries`)。

## 3. 旧来の無償産出の再現と解消

### 3.1 制御シナリオ(合成 fixture、`p28_010_impl_001_atomic_production_input_accounting_test.rs`)

| テスト | 内容 | 旧実装(HEAD)での結果 |
|---|---|---|
| `arcadia_like_shortage_no_free_output_and_exact_conservation` | Arcadia m019 型: Factory 1 + MilitaryFactory 1、Iron 0.975 | **失敗**(備蓄が減らず満額産出) |
| `one_of_two_inputs_short_limits_the_entry_and_scales_both_inputs` | 複数入力の一方(Coal)だけ不足 | 失敗 |
| `magic_academy_research_follows_actual_crystal_consumption` | MagicCrystal 不足で研究力(Oceanic 型) | 失敗(無償の研究力) |
| `many_entries_sharing_one_national_stockpile_conserve_resources` | 4 Factory 州 + 3 MilitaryFactory 州が同じ備蓄を使う | 失敗 |
| `bankrupt_penalty_does_not_reintroduce_free_output` | 破産ペナルティ(×0.8)との併用 | 失敗 |
| `slightly_short_inputs_never_go_negative_and_stay_consistent` | わずかに不足 | 失敗 |
| `sweep_of_stock_levels_keeps_accounting_identities` | 2,197 通りの掃引 | 失敗 |
| `sufficient_inputs_keep_legacy_consumption_and_output_exactly` | 入力が十分 | 成功(旧と完全一致) |
| `zero_inputs_produce_nothing_and_consume_nothing` | 入力 0 | 成功 |
| `exactly_sufficient_inputs_run_at_planned_operation` | ちょうど充足 | 成功 |
| `other_countries_stockpile_is_untouched` | 国家間分離 | 成功 |

「旧実装での結果」は、`production.rs` を HEAD の内容へ一時的に戻して実測した(7 件が失敗、4 件が成功)。実データのテスト `real_app_240_months_keeps_the_resource_ledger_closed_for_all_six_countries` も旧実装では失敗する(残差が出る)。

さらに `src/economy/production.rs` の単体テスト 3 件で、`consume_inputs_atomically` が「一方の入力だけ不足なら何も引かない」「全入力が賄えるなら全入力を引く」ことと、`limit_operation_ratio_to_stock` の境界を確認した。

### 3.2 実データでの解消(旧→新、120か月までの代表値)

| 国 | 旧(HEAD)の無償産出 | 新 |
|---|---|---|
| Arcadia | MS の 37.7%(367 / 974)、IG の 61.6%。Iron 備蓄 0.975 が 53 か月固定 | 0。MS 産出 974 → 652 |
| Oceanic | IG の 13.2%(40 / 304)、Coal・Iron・MagicCrystal の引落し失敗 | 0。IG 287 → 248 |
| Dwarf | 失敗 0 | 不変 |
| Passguard | 失敗 0 | 不変 |

## 4. 6か国の長期比較(通常AI、全6か国同時、実App)

「旧」= HEAD(`1eea8a5`)、「新」= 本実装。同一ハーネス(1日ずつ進める、開始 1800/01/01)。`shortD` は補給充足率 < 0.34 の Division 日数(Division ごとに日数を加算)。LS は州単純平均の生活水準。

### 4.1 90日

| 国 | 旧 Div / shortD / Iron / IG / 国庫 / LS | 新 同 |
|---|---|---|
| Arcadia | 1 / 27 / 174 / 1 / 4104 / 53.6 | 1 / 27 / 189 / 3 / 4124 / 53.6 |
| Elfin | 1 / 59 / 32 / 0 / 2909 / 64.6 | 同一 |
| Dwarf | 1 / 27 / 612 / 49 / 5569 / 57.4 | 同一 |
| Oceanic | 1 / 59 / 2 / 0 / 12920 / 67.5 | 同一 |
| Passguard | 1 / 59 / 146 / 0 / 194 / 48.7 | 同一 |
| Ferrowyn | 1 / 27 / 255 / 8 / 498 / 51.0 | 同一 |

### 4.2 36か月

| 国 | 旧 | 新 |
|---|---|---|
| Arcadia | 2 / 1135 / 1 / 24 / 5116 / 49.5(無償MS 201) | 2 / 1321 / 0 / 6 / 4208 / 49.5 |
| Elfin | 1 / 372 / 389 / 2 / 4251 / 64.6 | 同一 |
| Dwarf | 5 / 27 / 1218 / 1139 / 8947 / 60.2 | 同一 |
| Oceanic | 0 / 322 / 0 / 286 / 4774 / 64.6 | 0 / 322 / 9 / 248 / 4811 / 64.6 |
| Passguard | 1 / 119 / 37 / 18 / 85 / 59.2 | 同一 |
| Ferrowyn | 2 / 462 / 1140 / 896 / 768 / 51.3 | 同一 |

### 4.3 120か月

| 国 | 旧 | 新 |
|---|---|---|
| Arcadia | 2 / 6049 / 0 / 109 / 4517 / 48.5(無償MS 367) | 2 / **6431** / 0 / 6 / 4580 / 48.5 |
| Elfin | 3 / 1715 / 808 / 5 / 6270 / 64.6 | 同一 |
| Dwarf | 11 / 5857 / 49 / 5074 / 15176 / 65.7 | 同一 |
| Oceanic | 0 / 322 / 0 / 287 / 5022 / 63.6 | 0 / 322 / 9 / 248 / 4950 / 63.6 |
| Passguard | 1 / 119 / 37 / 18 / 904 / 70.0 | 同一 |
| Ferrowyn | 2 / 2080 / 3277 / 2190 / 686 / 55.3 | 同一 |

### 4.4 240か月

| 国 | 旧 | 新 |
|---|---|---|
| Arcadia | 0 / 7453 / 0 / 109 / 3722 / 50.3 | 0 / **7835** / 0 / 6 / 3731 / 50.3 |
| Elfin | 9 / 23771 / 34 / 3886 / 8216 / 69.0(無償MS 56) | 9 / 23879 / 34 / 3840 / 8128 / 69.0 |
| Dwarf | 11 / 19266 / 108 / 23481 / 16523 / 66.1 | 同一 |
| Oceanic | 0 / 322 / 0 / 287 / 3053 / 72.0 | 0 / 322 / 9 / 248 / 3351 / 70.3 |
| Passguard | 1 / 119 / 44 / 18 / 1842 / 70.0 | 同一 |
| Ferrowyn | 7 / 4614 / 47 / 9711 / 4259 / 68.0(無償MS 182) | 7 / 5069 / 30 / 9400 / 4359 / 68.4 |

### 4.5 戦争・国力

| 時点 | 旧 戦争総数 / 活動中 / 戦闘 | 新 |
|---|---|---|
| 36か月 | 6 / 1 / 23 | 5 / 1 / 23 |
| 120か月 | 8 / 2 / 23 | 7 / 2 / 23 |
| 240か月 | 10 / 2 / 24 | 9 / 2 / 24 |

- 戦争総数が 1 件少ない(Arcadia の無償産出が消えた結果、戦争の経過が変わった)。
- 国力・順位・Tier の個別の変化は測定していない(未確認)。

### 4.6 正当な変化として明示する点

- **Arcadia の補給不足は増える**(120か月 6,049→6,431、+6.3%)。旧実装の補給は無償産出で水増しされていた。本タスクは「ゲームバランス改善」を目的としない。
- **Elfin(m149〜)・Ferrowyn(m204〜)** の無償産出も消え、補給不足がわずかに増える(240か月 +0.5%、+9.9%)。
- **Oceanic** の IG・生活水準(240か月 72.0→70.3)も、軌跡の変化に応じて変わる。
- 無償産出の解消のための備蓄追加や AI 調整は行っていない。

### 4.7 工場稼働率・制約要因

- 稼働率は `building_operation_ratios` に実際の値で記録される。入力が足りない entry だけが `min(計画値, 備蓄 / (base × level))` へ下がる。
- 入力に制約されない国(Dwarf・Passguard、および Elfin・Ferrowyn の大半の月)では、稼働率は従来と同一。
- 資源別の詳細な時系列ダンプ(州別)は今回は行っていない。保存則の残差で代表した(§2)。

## 5. P28-010-PRE-001 の「2か月」と PRE-002 の「12か月」の定義

| 報告 | 指標 | 期間 | 値 |
|---|---|---|---|
| P28-010-PRE-001(Oceanic「2か月」) | Iron 帳簿の残差(`post − (pre + mine − F − MF)`)が PRE-001 の許容を超えた月。PRE-001 は 120 か月を観測 | 120 か月 | 2 |
| P28-010-PRE-002(Oceanic「12か月」) | 要求した Iron と実際に引き落とした Iron の差が 0.01 を超えた月(Factory + MilitaryFactory の合計) | 120 か月 | 12 |
| 本タスク | 4 資源(Iron・Coal・IG・MagicCrystal)の保存則の残差が 1e-3 以上の月 | 240 か月 | 0(アサーション) |

PRE-001 と PRE-002 の数字は定義(許容・対象資源)が異なる別の指標であり、同一視していない。PRE-001 の許容は今回は再確認していない。

## 6. Save と保護範囲

| 項目 | 結果 |
|---|---|
| Save V1 | 変更なし。永続フィールド・migration の追加なし |
| プレイヤー・AI | 同じ `process_country_production` を共有。国家IDの特例なし |
| `src/country/country_ai.rs` | 変更なし |
| Iron の Factory/MilitaryFactory 間の優先配分 | 変更なし(候補Dは未実装) |
| 募兵AI・攻勢補給ゲート・戦闘式・RON データ | 変更なし |
| `saves/savegame_v1.ron` | 読み書き・復元・ステージなし |
| `tests/land_war_combat_peace_test.rs` | 変更なし |
| save→load・決定性 | 既存の save→load 系テスト(p26_det_001 の test_h、p26_det_001_rev_001 の test_k、p28_002 の test_03、p28_008 の save_load_around_the_peace_matches_continuous_run)が全て合格。独立した2つのプロセスで実App 240か月のテストを再実行し、出力が一致 |

## 7. 変更した既存テスト(旧挙動に合わせて弱めていない)

全て、無償産出の消滅 → Arcadia 等の産出・戦争経過の変化 → 結果、という因果の確認後に、必要な期待値だけを更新した。差分は `src/economy/production.rs` の入力引落しだけである。

| テスト | 変更 | 因果 |
|---|---|---|
| `p26_004_pre_005_..._test::test_f2_arcadia_deterministic_production_values_...` | Arcadia 月12/月18 の正規値 471.790651→453.452948、749.636486→731.298783 | 1800/03 の MilitaryFactory が IG 備蓄不足で IG を引けないまま満額 MS を産出していた無償産出が消え、IG 備蓄が減って後続月の IG 律速の稼働率も下がった |
| `p26_det_001_..._test::test_e / test_f`、`p26_det_001_rev_001_..._test::test_h / test_i` | Arcadia の月12/18/24/36 の正規値(453.452948 / 731.298783 / 810.186777 / 967.794283) | 同上。他5か国の値は実測で不変(同テスト内で確認) |
| `p26_008_c_..._test::test_90_day_and_36_month_six_country_real_data_verification` | Passguard について「Factory 州の喪失」を明示的に確認する例外を追加 | 修正後は Passguard が州10(Factory 1 + MilitaryFactory 1)を戦争で失う(戦争総数 7→10)。旧テストの「36か月後に構造的欠損が無い」は、AI 建設判断の不発ではなく州の喪失で成立しなくなる。州10の所有者で確認するようにした(他5か国は従来どおり欠損なしを確認) |
| `p27_002_c_..._test::test_05_real_six_country_...` | 120か月の財政正規値(6か国)・振動回数(Elfin 0→1)・Oceanic の振幅上限(0.02→0.05、実測 0.04) | 世界の軌跡の変化(Elfin は州3、Passguard は州10 を失う)。国庫は全国で非負、最大税率は 0.15、Oceanic の振動は有界な範囲(崩壊型の 44〜118 回・0.97 とは別) |
| `p27_002_c_..._test::test_18_countries_untouched_...` | 領土不変の前提を「州3/州10 の喪失」へ更新、新正規値(Elfin 0.14/3410.35/10.33/1、Passguard 0.18/921.35/8.29/3)。開始時の領土も固定 | 同上 |
| `p28_002_..._test::test_02_player_country_at_war_...` | 観測日を 1801/01/01 → 1800/12/01 | プレイヤー Arcadia への AI の宣戦3件が 1800/12/10 に終結するようになり、1801/01/01 時点では交戦中でなくなった |
| `p28_008_..._test::standard_scenario_e2_...` | 検証対象の WarId を 3 → 1 | 戦争の発生順が変わった。同じ組み合わせ(att c5 / def c3)が WarId(1)(1800/08/03 開戦、終結は同じく 1801/03/22 の E2)になった。WarId(2)(att c2 / def c3、1801/04/06 降伏終結)は同じ。E2 の講和が war2 の s9 占領を消さない因果は同じ実データ上で再現する |

### 7.1 変更しなかった(失敗しなかった)ことの確認

- 入力引落しの失敗がなかった Dwarf・Passguard は、**P28-010-PRE-002 の FIX1 全国同時の基準と同一の値**で一致する(`dwarf_and_passguard_match_pre_change_normative_values_at_month_120`)。
- 注: p27_002_c のテストハーネス(月の進め方が別)では、Passguard の値も新しい軌跡に移る(州10の喪失による)。これは Passguard の生産会計の変化ではなく、戦争経過の変化による。

## 8. 残課題

| 項目 | 状況 |
|---|---|
| 候補D(MilitaryFactory への Iron 優先配分) | 本タスクでは未実装。会計補正後の基準で再測定してから採否を決める(別タスク) |
| Factory 増設抑制・P28-004 募兵制限 | 対象外 |
| 戦争経過の変化の個別の因果 | 差分は production.rs のみであることまでは確定。個別の戦闘経過は追っていない |
| 国力・順位・Tier の変化 | 未測定 |
| 手動プレイの「約20%」 | 未確認 |
| UI 実ウィンドウ確認 | MANUAL VERIFICATION PENDING |
| P27-005 手動確認 | PENDING |
| MagicAcademy の無償研究力の量 | 定量していない(保存則・単体テストで解消を確認) |

## 9. 実行コマンドと結果(今回の実測)

| コマンド | 結果 |
|---|---|
| `cargo test --test p28_010_impl_001_atomic_production_input_accounting_test` | 11 passed / 0 failed |
| `cargo test --test p28_010_impl_001_real_data_atomic_accounting_e2e_test` | 3 passed / 0 failed |
| `cargo test --lib economy::`(単体テスト3件を含む) | 61 passed / 0 failed |
| 関連テスト(p23_004、p26_003 ×3、p26_004 ×4、p26_008_c、p26_det_001 ×2、p27_002 a/b/c、p28_002、p28_005)を個別に実行 | 全て 0 failed(更新後) |
| 独立した2プロセスで実App 240か月のテストを再実行 | 出力(ROW 行)が一致 |
| `cargo test --all-targets -j 2 --no-fail-fast` | **2811 passed / 0 failed / 1 ignored**(90 binaries。開始時の 2794 + 新規 17) |
| `cargo clippy --all-targets --all-features -- -D warnings` | **警告 0** |

- 修正前のコード(HEAD の `production.rs`)に対して新規テストを実行し、実データ・fixture の無償産出系 8 件が失敗、入力が十分な系 6 件が成功することを確認した。
- 全体テストの途中、既知の非決定的PNG 9件が再生成された。`git status --short` で確認した9件を個別に `git checkout --` で復元した。

## 10. 変更ファイル

| ファイル | 内容 |
|---|---|
| `src/economy/production.rs` | 原子的会計(2つの関数と Step 3 の呼び出し)+単体テスト3件 |
| `tests/p28_010_impl_001_atomic_production_input_accounting_test.rs` | 新規(11件) |
| `tests/p28_010_impl_001_real_data_atomic_accounting_e2e_test.rs` | 新規(3件) |
| `tests/p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs` | 正規値の更新(§7) |
| `tests/p26_008_c_supply_chain_construction_priority_e2e_test.rs` | 例外の追加(§7) |
| `tests/p26_det_001_construction_ai_determinism_e2e_test.rs` | 正規値の更新 |
| `tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs` | 正規値の更新 |
| `tests/p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs` | 正規値・前提の更新 |
| `tests/p28_002_wartime_military_factory_construction_e2e_test.rs` | 観測日の更新 |
| `tests/p28_008_concurrent_war_peace_scope_e2e_test.rs` | WarId の更新 |
| 本報告書 | — |
