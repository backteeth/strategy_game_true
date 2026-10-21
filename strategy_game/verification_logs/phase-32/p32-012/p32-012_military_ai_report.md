# P32-012 (M4-D) 軍事状況を参照する経済AIと履歴管理の新ステート経路への接続 報告

- 開始時HEAD: `7e90dff`(P32-011、origin/mainと一致、未追跡は `hex_atlas/` のみ)
- 終了時HEAD: コミット後に最終報告へ記載
- **判定: PARTIAL**。今回の対象(優先度2.5・2.8・2.9と履歴に基づく判断)は新ステート経路へ接続し、旧経路と毎日一致を確認したが、下の「未接続」と「M4残件」が残る。新マップ上の軍事シミュレーションの完成ではない。Phase 32 は継続。

## 1. 旧AIの実際の条件(コードから特定。番号から推測していない)
すべて `process_construction_ai_with_contexts` 内、月次評価(`day_number % 30 == country_id % 30` または dirty)で、税率AIの後に呼ばれる。建設キューが2件以上なら何もしない。乱数は使わない(候補は昇順の地域リスト・`ResourceType::ALL`・`BuildingType`固定順だけで決まる)。クールダウンは建設AIには無い(`cooldown_until_day`は戦争準備AIのみ)。重複判断の抑止は「同種の注文が建設キューにある間は着工しない」。

| 優先度 | 行動 | 条件(要約) | 参照する国家・軍事情報 | 参照する地域情報 |
|---|---|---|---|---|
| 1 | Farm | Food<50 | 備蓄 | (経済) |
| 2 | Mine/Factory(事前開発) | 保有施設の入力資源に生産経路が無い | 備蓄・キュー | 施設・鉱床 |
| **2.5** | MilitaryFactory(戦時) | 戦時で日次需要を算出できる / キューにMFなし / 実月産(units) < 日次需要×30 / 入力備蓄が1か月分以上 / 増産見込みが正で最大の州 / 国庫 ≥ 費用+月次経常支出×1 | 戦争状態、`country_daily_demand_units`(生存師団から) | 所有州のMF稼働・施設・労働力・物流 |
| 2.7 | Railway | 物流比<0.9の最低の地域(経済。P32-010で接続済み) | | |
| **2.8** | Mine(Iron供給) | Ironの継続的フロー不足(履歴: 同じMineレベルの連続4か月、全月で未充足>0) / 残り不足 ≥ 1レベル見込みの半分 / 候補はIron鉱床がある州で見込み最大 / 国庫 ≥ 費用+月次経常支出×3 | `IronShortageHistory` | 鉱床・Mine稼働・Mineレベル |
| **2.9** | MilitaryFactory(再建) | 履歴(軍需生産、連続4か月) / 所有MFレベル合計0 / 戦力(生存師団+募兵キュー) < 目標(`calculate_ai_target_divisions`) / MFがキューに無い / 国内供給(Mine・Factoryの稼働実績)がIron・IGの必要量を賄える / 国庫 ≥ 費用+月次経常支出×3 | `SupplyProductionHistory`、生存師団数、募兵キュー、標準歩兵の必要人的資源 | 人口・施設・稼働率・鉱床 |
| 3 | Mine/Factory | 原材料<30→Mine、他はFactory(ただし**Factory増設抑止**が真ならFactoryは建てない) | `FactoryStarvationHistory`(連続4か月、全Factoryが飢餓、同じ設備署名) | |

履歴の更新(旧`handle_monthly_economy`、月次生産の直後=その月のAI判断より前): 軍需生産(`military_supplies_produced`→units)、Iron未充足(`iron_unmet_military`・Mineレベル合計)、Factory飢餓(全entryが飢餓か・設備署名)。プレイヤー国は対象外。所有州が無い国は履歴を消す。履歴の更新は**建設注文の成否には依存しない**(注文を受理した時・支払った時・失敗した時に履歴は更新されない。評価した時・決定した時にも更新しない)。同月・過去月の記録は無視される。

## 2. 今回接続した項目と、後続に残る項目
- **接続**: 2.5・2.8・2.9、3の増設抑止(履歴)、3つの履歴の記録(国家ごと・月次生産の直後)。判断の結果は、すべてステート単位の既存経済施設(Mine / MilitaryFactory)の建設注文で、P32-010の建設経路(キュー・支払い・進捗・完成)をそのまま使う。
- **後続(新しい軍事モデルが必要)**: 軍隊への命令・募兵・補給ゲート(`recruitment_supply_blocked`)、戦争状態の導出、実際の師団の位置・損耗、軍需の実消費。今回の軍事状況は**明示した入力**であり、これらを新マップ上で導出する処理は実装していない。軍隊へ命令する処理は含まれず、命令を実行せずに履歴だけ更新することもない。

## 3. 軍事状況の入力と出典
`MilitaryAiFacts { at_war, daily_demand_units, living_divisions, recruitment_queue_len, infantry_required_manpower }`(`country/ai_military.rs`)。
- 旧経路: `military_ai_facts(country, at_war, military_registry)` が戦争・軍隊・募兵キューの登録簿から作る(同じ型)。
- 新経路: 検証用シナリオ(`ScenarioCountryInput.military`)が**明示**する。出典はfixture(標準歩兵N個)に旧経路の同関数を通した値で、テストが一致を検証する(10個→日次需要 10,000,000 units/日)。新しい脅威評価・戦略は無い。
- `None` = 入力なし。「平和」「脅威ゼロ」とは扱わず、2.5・2.9は評価されない(評価の記録 `AiEvaluation.military_input` が `None` を明示)。需要を算出できない場合(`daily_demand_units: None`)は0ではなく文脈なし。
- 旧`StateId`と`MapStateId`は数値一致で対応付けない(履歴の設備署名の数値は識別空間が違うため新旧で異なり、比較では数値を伏せた。同じ国家の履歴内の一致判定という意味は同じ)。
- RONが128bit整数を扱えないため需要は64bit(1.8e19 units/日超は「算出失敗」と同じ扱い)。実用範囲では旧と同一。

## 4. 共通化した判断処理(式・閾値・順序は変更していない)
- 新規 `src/country/ai_military.rs`: `try_wartime_supply_construction`(2.5)・`try_iron_supply_construction`(2.8)・`try_military_rebuild_construction`(2.9)・`projected_military_factory_supply_gain_for`・`military_rebuild_context_for`・`iron_supply_context_from`・`factory_expansion_blocked_from`・`MilitaryHooks`(差し込み)。旧`country_ai.rs`の同名の関数本体を移したもので、旧・新が同じ関数を通る(旧は薄いラッパーのみ)。
- トレイト拡張: `AiRegions::state`(地域の経済データ)、`AiCountry::queued_regions`、`StateEconomyAccess::logistics_usage`。
- 新経路の追加前提: Mine(と新経路のMilitaryFactory)の候補は `order_precondition`(P32-009・010の資源設定確認)を満たすものだけ。満たさない候補は飛ばして他の候補の評価を続ける。
- 新経路: `MapStateEconomyRegistry::run_day` が履歴を記録(`record_ai_history`)し、文脈を作って `run_construction_priorities` に `MilitaryHooks` を渡す。国家ごとの履歴 `ScenarioAi.history`(実行時のみ。**永続化しない**。寿命はその実行の間=ロード・New Gameで空。新しい永続化形式・セーブ移行は無い)。国家間で混ざらない。

## 5. 新旧比較(条件と結果)
- 旧経路 = 実際の旧System(`handle_daily_construction`・`handle_monthly_population_growth`・`handle_monthly_economy`。履歴は実際の`CountryAiRegistry`へ記録)+旧AIの公開関数(`process_tax_rate_ai`・`process_construction_ai_with_contexts`・`military_rebuild_context`・`iron_supply_context`・`factory_expansion_blocked`)を、旧と同じ日程(経済→AI、`label % 30`)で駆動。
- 条件: 同じ初期状態、同じ軍事fixture、乱数なし。比較は毎日、全ステート・全国家の国庫・税率・備蓄・キュー(施設・目標レベル・進捗・支払済み費用)・雇用・物流・稼働率・経済状態・**3つの履歴の内容**。
- 結果: 3か国・300日(10か月)、および境界の変種で一致。実際に発火:
  - 2.5(国家1): 1800/02/01 MilitaryFactoryを着工(費用1200を1回だけ支払い、必要90日→5/02に完成。手計算: 2/02〜2/28=27、3月31=58、4月30=88、5/01=89、5/02=90)。以後も実月産が需要に届かない間は再着工(5/31、8/31)。
  - 2.8(国家2): 7/02 Iron供給のMine(平均未充足36.19)。Mineレベルが変わる3/03以降の4か月(4〜7月)が揃った日が最初で、それ以前は文脈なし。次の評価(8/02)は建設中の増分を差し引くため重複着工しない。
  - 2.9(国家3): 履歴4か月(2〜5月)が揃う5/03から再建の文脈が成立し、先行する建設が空いた8/03に着工。
  - Factory増設抑止(国家2): 履歴4か月・同じ設備の5/02だけ抑止(4/02はまだ、5/03にFactory完成後の6/02は解除)。
  - 競合: 戦時(師団2<目標3)ではMF着工(4/01、2.5)が先で、2.5と2.9の文脈が併存する間もキューにMFがある限り二重に着工しない。
- 境界: 予算不足(国庫1200)・入力備蓄不足(Iron 10)・平時・需要None・需要0・入力なし・標準歩兵定義なし・戦力=目標(再建の需要なし)・募兵キューが戦力に算入・資源未設定の候補を飛ばして他を評価。

## 6. 未接続部分と理由
- 位置施設(港・要塞・駅)の建設判断・効果: 今回の対象外(位置施設の建設キューは未実装)。
- 軍隊への命令・募兵AI・補給ゲート、戦争状態・師団の導出: 新しい軍事モデルが必要。入力は明示fixtureのみ。
- 履歴の`CountryAiRegistry`への永続化(セーブ): 実装しない(旧セーブ互換は旧経路のまま)。
- 旧経路との差(比較対象外): 旧AIは資源の設定状況を持たないため、未設定資源のステートへのMineを新経路は採用しない(P32-010と同じ意図した差)。

## 7. 地理に関する記録の訂正
P32-011報告の「実地図には湖がない」は不正確だった。確認した範囲: **全体の地理JSONには湖セルが202件ある**。**検証地域(previewの29プロヴィンスと近傍)には湖が無い**(水域はすべて海)。P32-011報告に訂正を追記した(地理JSONは変更なし)。テスト: `lakes_exist_in_the_full_geography_but_not_around_the_preview_region`。

## 8. 検証結果
- 新規テスト12件(`tests/p32_012_military_ai_test.rs`)。`cargo test -j 1 -- --test-threads=1`: **3181 passed / 0 failed / 6 ignored、exit 0**(`full_test_j1_threads1.log`。再生成された既存スクリーンショット9枚は`git checkout`で復元しコミット対象外)。`cargo clippy -j 1 --all-targets --all-features -- -D warnings`: 警告0、exit 0(`clippy.log`)。
- `world_economy_check`: 入力・判断理由・行動・履歴更新を表示(`world_economy_check_output.txt`)。
- 既存の旧AI関連テスト(p27_002_c・p28_002・p29_009・p29_010・p30_002・p30_003、`--lib country` 301件)は、リファクタ後も合格。
- GUI変更なし(ユーザー手動確認待ち項目なし)。

## 9. 影響
既存ゲーム・アセット(新規ファイルの追加のみ)・セーブへの意図した影響なし。旧AIの判断は共通関数への移動で、条件・順序・閾値・予算の式は変更していない。

## 10. M4の残件一覧
- プレイヤーの手動建設注文(新ステート経路)
- 新注文のキャンセルと返金
- 軍事: 軍隊への命令・募兵・補給ゲート・戦争状態の新マップ上の導出(新しい軍事モデルが必要)
- 位置施設のゲーム効果と建設(キュー・AI)
- AIの履歴の永続化・セーブ形式、新マップへのゲーム本体の切り替え
- 設計書の未決事項(部分割譲・法的所有・占領/支配の導出・前線計画単位・移動/占領速度・補給の計算単位・旧セーブ/V2・海軍/湖・ステート連結規則)

## 11. 次の実装単位(提案・要確認)
新ステート経路のプレイヤー手動建設注文とキャンセル/返金(M4残件のうち、新しい軍事モデルを要しない部分)。

## 追記(P32-013で判定範囲を再整理)
本報告のPARTIALは、「P32-012で対象にしたAI判断・履歴接続」と「M4・新マップ移行全体の残件」を区別せずに付けていた。P32-013で照合した結果、**P32-012の対象内に未実装の処理は無く、対象の範囲は COMPLETE と整理する**(上のPARTIALの記述は当時の報告として残す)。第6節「未接続部分」と第10節「M4の残件」は、P32-012の対象外だったM4全体の残件であり、M4全体とPhase 32は未完了のまま。このうち「プレイヤーの手動建設注文・キャンセル」はP32-013で対応した。詳細は `verification_logs/phase-32/p32-013/p32-013_manual_order_report.md` の第1節。
