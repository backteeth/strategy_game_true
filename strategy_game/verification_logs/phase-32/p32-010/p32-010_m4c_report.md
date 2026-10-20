# P32-010 (M4-C) 経済状態・建設キュー・経済AIの新ステート接続 報告

- 開始時HEAD: `0388467`(P32-009、origin/mainと一致)
- 終了時HEAD: コミット後に最終報告へ記載(本報告は同コミットに含まれる)
- Phase 32 は継続。M4-C の完了は M4 全体・新マップでのゲーム進行の完了ではない。

## 変更ファイル
- 新規: `src/country/ai_core.rs`(経済AIの共通中核)、`tests/p32_010_map_state_ai_test.rs`(15件)、`assets/data/world/preview/economy_scenario_m4c.ron`
- 変更: `src/country/country_ai.rs`(共通中核を呼ぶ形に整理。振る舞いは不変)、`src/country/mod.rs`、`src/building/construction.rs`(`advance_construction_queue`)、`src/economy/mod.rs`、`economic_state.rs`(`compute_economic_metrics`)、`state_access.rs`、`map_state.rs`、`src/bin/world_economy_check.rs`、`economy_scenario_m4b.ron` と `tests/p32_008_*`(必須項目 `ai_controlled` の追加のみ)
- 保存形式・地理JSON・Bevy/Rustは変更なし。

## 共通化した範囲(新旧で式を複製しない)
- 経済状態の指標算出 `compute_economic_metrics` + 既存 `calculate_economic_state`
- 建設進捗・完成判定 `advance_construction_queue`(旧`handle_daily_construction`も同じ関数)
- 税率AI `next_ai_tax_rate`(既存 `decide_ai_tax_rate_delta` / `apply_tax_rate_delta`)
- 建設AIの優先度 1・2・2.7・3 `run_construction_priorities`(トレイト `AiCountry` / `AiRegions` を旧`CountryData`/`StateRegistry`と新`ScenarioCountry`/`MapAiRegions`が実装)

## 対応する経済施設 / 非対応の位置施設
- `BuildingType`の現行10種はすべてステート単位の経済施設(Railwayはステート単位の物流施設)。判定は `is_state_level_economic_facility`。
- 位置施設(港・要塞・駅)は型として存在せず未実装。新経路の注文型は `MapStateId` のみを対象にする。
- 新経路は未対応(比較対象外): 戦時MilitaryFactory(2.5)、Iron供給(2.8)、軍需再建(2.9)、履歴に基づくFactory増設抑止、プレイヤーの手動注文とキャンセル/返金(既存規則はプレイヤー専用で、新規則は作っていない)。

## 実行順序(1日)
1. 建設進捗(完成した注文を1回だけ反映) → 2. 月が変わる日は月次経済(人口成長・雇用・物流・生産・財政・福祉・経済状態) → 3. AIの評価周期の国家で税率AI → 建設AI。旧スケジュール(Economy Set → CountryAi Set)と同じ。

## 未設定資源の扱い
- Mine の注文受付時と候補評価時に、そのステートの資源が「設定済み(資源なしの明示を含む)」であることを要求。未設定は `OrderBlock::ResourcesNotConfigured` で拒否/候補除外(他の候補の評価は継続)。未発見の鉱床は発見済み扱いにしない。
- 注文後に資源が未設定へ変わった場合、完成時に `Blocked`(注文はキューに残り、費用・施設レベル・他の値は一切変更しない)。再設定後に1回だけ完成。
- Mineを持つステートへ未設定資源を接続する `attach_province_resources` もエラー(部分更新なし)。

## 更新単位
注文=国家単位の国庫支払い(注文時に1回、プロヴィンス数に依存しない) / 完成=ステートの施設レベル+1 / 税率=国家 / 経済状態=国家。

## 旧新比較と独立期待値
- 比較: 実際の旧System(`handle_daily_construction`・`handle_monthly_population_growth`・`handle_monthly_economy`)と旧AI公開関数を同じ日程・同じ初期条件で駆動し、**120日間の毎日**、全ステート・全国家(国庫・税率・収支・備蓄・経済状態・キュー・施設・雇用・物流・稼働率)の一致を確認。境界入力(人口0・施設なし・動員0/過大・国庫不足)も90日一致。
- 独立期待値(手計算、旧経路を使わない): Mine費用500・必要60日→1/02注文は3/03にLv2→3へ1回だけ、Farm費用200・30日→2/01にLv6→7、完成後の必要労働力68,000(Farm3×5000+Mine3×10000+Factory1×20000+Railway1×3000)、経済状態の閾値(80/60/40/20)の境界値、建設費と所要日数はプロヴィンス数(7と13)に依存しない。
- 意図した差(比較対象外): 旧経路は資源の設定状況を持たないため、未設定資源のステートへの Mine を新経路は採用しない。

## 複数か月結果(`world_economy_check_output.txt`)
4か月/120日(通常の建設期間)。国家1: Mine(MapState#20)・Factory、国家2: Farm・税率0.30→0.27(月次調整)・Mine が実際に完成。完成後に雇用が増加(例 58000→68000→88000)。経済状態は Stable/Boom で推移。

## 検証結果
- `cargo test -j 1 -- --test-threads=1`: 3155 passed / 0 failed / 6 ignored、exit 0(`full_test_j1_threads1.log`)。再生成されたスクリーンショット9枚は `git checkout` で復元済み(コミット対象外)。
- `cargo clippy -j 1 --all-targets --all-features -- -D warnings`: 警告0、exit 0(`clippy.log`)。
- `world_economy_check`: exit 0。
- GUI変更なし(ユーザー手動確認待ち項目なし)。

## 影響
既存ゲーム・アセット・セーブへの意図した影響なし(旧AI・旧建設の全既存回帰テスト合格、36か月の旧正準値も一致)。ULP級の差が出得るのは、P32-008の維持費合計と今回の平均稼働率の合計順序を `BuildingType::ALL` 順で決定的にした点のみ。

## M4 の実装済み / 残り
- 実装済み: M4-A(人口・雇用・物流・生産)、M4-B(財政・人口成長・福祉)、M4-C(経済状態・建設・経済AI)。
- 残り: 位置施設の入力層、AIの軍事優先度と履歴、新注文のキャンセル/返金、占領・補給などの未決ルール、セーブ形式。いずれも未決のまま。

## 次の実装単位(提案)
位置施設(港・要塞・駅)のプロヴィンス入力層、またはAI軍事フックの新経路対応。着手前にユーザー確認を取る。
