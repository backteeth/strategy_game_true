# P28-002 戦時のMilitaryFactory建設AIによる攻勢補給の持続化

P28-002-PRE-001で採用した候補C(戦時のAI国家が、実際のMilitarySupplies生産不足を埋めるMilitaryFactoryを建設する)を恒久実装した。攻勢補給ゲート(停止40日・再開70日、前線配送0.34)、前線戦力比(開始110%・継続90%)、宣戦判定、日次補給・前線補給、戦闘・講和の式、平時建設の予備資金契約、Save形式は変更していない。

## 1. 判定

```text
P28-002 COMPLETE
```

全AI・無操作の自然運用で、宣戦したDwarfが戦時MilitaryFactoryを建設した。完成後の自国生産によって月産が需要を上回り(40→78.3/月、需要62/月)、国家備蓄が**既存の再開値70日**に達した。その後、攻勢→敵州到着→戦闘→Arcadia降伏まで進んだ。120か月で戦争10件、終戦6件、戦闘10件、敵州到着23件、支配州変更14件(現状は戦争3件・その他すべて0件)。補給の注入やゲート条件の変更はしていない。

## 2. 開始HEAD・基準

- 開始HEAD: `7b3f406ce6d9d00de41a7756b1d7438c7ef87f9a`(期待値と一致)
- 開始時`git status --short`: 空
- 開始時の全体テスト基準: 2709 passed / 0 failed / 1 ignored、clippy警告0。この値は、同一コミット・同一作業ツリー(空)で実行したP28-002-PRE-002の最終検証の実測値である。本タスクの開始時には全体テストを再実行していない(本番コードの変更はその後なく、HEADは同じ)。

## 3. 実コードとの照合

| 項目 | 正規の定義場所 | 照合結果 |
|---|---|---|
| MilitaryFactoryの定義 | `assets/data/buildings.ron` | 建設費1200、必要進捗90、必要労働力25000、物流コスト20、入力Iron 30・IndustrialGoods 15、出力MilitarySupplies 100(level・月あたり)。報告書と一致 |
| 生産式 | `economy::production::process_country_production` | `稼働率 = min(労働力比, 物流比, 入力充足率)`(国庫が負なら×0.8)、`出力 = 基準出力 × level × 稼働率`。所有州(`owner_country_id`)で数える |
| 労働力比 | `population::update`(`update_state_population_and_employment`、`calculate_economic_workforce_ratio`) | `雇用 = min(州の総必要労働力, 民間労働力)`、`労働力比 = 雇用 ÷ 労働力(total_workforce)` |
| 物流比 | `logistics::update::calculate_logistics_ratio` | `min(容量 ÷ 使用量, 1)`、使用量は`Σ logistics_cost × level` |
| 月次経常支出 | `CountryData.monthly_expenses`(`economy::finance`が月次で確定) | 平時建設の予備判定(`try_construct_peacetime`)と同じ値を使う |
| 建設キュー・支払い | `ConstructionQueueItem`、`country.treasury -= construction_cost` | 既存の優先度2と同じ経路 |
| AIの優先順位 | `country_ai::process_construction_ai` | 優先度1 Food<50、優先度2 供給網の構造的欠落の修復(Mine/Factory、予備判定なし)、優先度3 平時開発(予備3か月) |
| 攻勢補給ゲート | `military::recruitment_supply_gate`(`WARTIME_STOP_DAYS=40`、`WARTIME_RESUME_DAYS=70`)、`war::offensive_supply_gate`(`MIN_OFFENSIVE_SUPPLY_FULFILLMENT=0.34`) | 変更なし |

P28-002-PRE-001との違い: 同報告書§13.1は対象を「`is_at_war`または`CountryAiMode::PreparingWar`/`AtWar`」としていた。本タスクの仕様は「対象は戦時のAI国家」で、平時AIには新経路を適用しない。このため、戦争記録上の交戦状態(`WarRegistry`のActive戦争への参加)だけを対象にし、開戦準備中(平時)は含めなかった。`CountryAiMode::AtWar`は終戦後も`Recovering`へ移るまで残るため、判定にはモードを使っていない。結果の差は§8で比較する。

## 4. 実装した判定式と優先順位

変更ファイル: `src/country/country_ai.rs`(本番コードはこの1ファイルだけ)。

### 4.1 評価位置と優先順位

- `process_daily_country_ai`の月次評価(`current_day % 30 == country_id % 30`、または`dirty`)で、`process_tax_rate_ai`の後に`process_construction_ai_with_wartime_supply`を呼ぶ。この日次ループは元からプレイヤー国家と消滅国家(`!is_active()`)を除外している。
- 戦時(`is_at_war`)のAI国家についてだけ、`WartimeSupplyContext { daily_demand_units }`(`country_daily_demand_units`)を渡す。平時や、需要の算出が失敗した場合は`None`になり、従来とまったく同じ建設判断をする。
- 優先順位: **優先度1 Food緊急 → 優先度2 供給網の修復 → 優先度2.5 戦時MilitaryFactory(新規) → 優先度3 平時開発**。優先度2.5で着工した月は優先度3を評価しない(1回の判断で1件)。着工しない場合は優先度3へそのまま進み、優先度3の3か月予備の契約は変えない。
- 既存の`process_construction_ai`(公開API)は`None`を渡す薄い入口として残した。挙動はP28-002以前と同一で、既存の呼び出し元・テストは無変更。
- 判断理由: Saveに保存される`CountryAiDecisionReason`へ新しい値を足さず、既存の`WarInProgress`を再利用した(Save V1の値域を広げないため)。

### 4.2 着工条件(`try_wartime_supply_construction`)

すべて満たす場合だけ、MilitaryFactoryを1件着工する。

1. 建設キューにMilitaryFactoryがない(未完了の同種建設を重複発注しない)。キュー上限2件は既存の冒頭判定のまま。
2. **実月産 < 日次需要 × 30**。実月産は所有州の`100 × level × 稼働率`の合計で、稼働率は前回の月次生産が書き戻した値。比較は正規units(`military_supplies_value_to_units`)同士で行う。需要0なら着工しない。
3. **入力資源**: 定義上の各入力(Iron、IndustrialGoods)について、`国家備蓄 ≥ 所有州の全建物が稼働率1.0で消費する月量 + 新規1レベル分`。既存施設の消費は、実際の稼働率を掛けない最大量で数える(実消費より厳しい、保守的な判定)。
4. **州の選択**: 自国が所有かつ支配する陸上州のうち、増産見込み(§4.3)が正で最大の州を選ぶ。同値はStateIdの小さい方(建設可能州は`StateId`昇順で走査し、厳密に大きい場合だけ更新)。上限level・労働力0の州は候補外。
5. **資金**: `国庫 ≥ 建設費(RONの1200) + 月次経常支出 × WARTIME_SUPPLY_CONSTRUCTION_RESERVE_MONTHS(=1.0)`。国庫・建設費・経常支出のいずれかが非有限または負なら着工しない。
6. 着工時は既存経路で`国庫 −= 1200`とし、`ConstructionQueueItem { target_level: 現在level+1, progress: 0, required_progress: 90, paid_cost: 1200, InQueue }`を追加する。

### 4.3 増産見込み(`projected_military_factory_supply_gain`)

```text
雇用(J)       = min(J, 民間労働力)      J = Σ(required_workforce × level)、整数、BuildingType::ALL順
労働力比(J)   = calculate_economic_workforce_ratio(雇用(J), total_workforce)  … f32、[0,1]
物流比(U)     = calculate_logistics_ratio(物流容量, U)
稼働率(J, U)  = min(労働力比(J), 物流比(U))
見込み = 100 × (level+1) × 稼働率(J + 25000, U + 20) − 100 × level × 稼働率(J, U)
```

評価時点は月次評価の日で、州の建物構成・雇用(前回の月次雇用計算の値)・物流容量はその時点の値を使う。入力充足率は国家単位で決まるため見込みには入れず、条件3で別途要求する。建設期間(90日)の間に人口・雇用・他の建物・入力備蓄が変わるため、**見込みは完成後の実生産と一致するとは限らない**。完成後は、次回以降の判定で条件2の実月産として再評価される。例として、Dwarfの州20 lv1(着工時の見込みは労働力比・物流比から計算)は、完成後の実月産でDwarf全体を40→78.3/月へ押し上げた(§6)。

### 4.4 扱った境界

| 状況 | 挙動 |
|---|---|
| 需要0 | 着工しない(条件2) |
| 建設可能州なし(全州が上限level、労働力0、支配していない) | 着工しない |
| Iron/IndustrialGoodsが1か月分未満 | 着工しない。優先度3へ進む |
| 国庫がしきい値の直下 | 着工せず、国庫を変更しない |
| 国庫がしきい値ちょうど | 着工する(`>=`) |
| 戦争終結直後 | `is_at_war`が偽になった月から文脈を渡さない。キューにある建設は通常どおり完成する |
| 複数戦争 | 需要は国家総需要1つで、戦争数に依存しない。着工は国家ごとに1件 |
| プレイヤー国家・消滅国家 | 日次ループの対象外(既存)。単体テストとE2Eで確認 |

## 5. 必須テスト

### 5.1 単体テスト(`src/country/country_ai.rs`、12件追加)

| テスト | 契約 |
|---|---|
| `p28_projected_gain_uses_workforce_and_logistics_of_the_candidate_state` | 見込み=110(労働力比0.9→1.0)、物流飽和州は`200×35/55−90`、上限level・労働力0は`None` |
| `p28_wartime_ai_starts_exactly_one_military_factory_when_realised_supply_falls_short` | 着工1件、国庫−1200、理由`WarInProgress`(契約1) |
| `p28_peacetime_path_never_starts_a_military_factory` | 文脈なし(平時)では着工しない(契約2) |
| `p28_state_is_chosen_by_projected_supply_gain_not_by_level_or_state_id` | StateIdが小さくlevelも高い州ではなく、見込みの大きい州を選ぶ(契約3) |
| `p28_equal_gain_breaks_ties_by_state_id_regardless_of_registration_order` | 同値はStateId順、登録順を反転しても同じ(契約4) |
| `p28_missing_inputs_block_the_wartime_factory` | Iron 79.99/80、IndustrialGoods 29.99/30の直下・ちょうど(契約5) |
| `p28_treasury_threshold_is_cost_plus_one_month_of_expenses` | 1299.99は着工せず国庫不変、1300ちょうどで着工、NaNは着工しない(契約5・6) |
| `p28_no_buildable_state_no_demand_or_sufficient_supply_means_no_factory` | 建設可能州なし、需要0、実月産=需要(契約5) |
| `p28_unfinished_military_factory_is_never_ordered_twice` | 2回評価してもキューのMilitaryFactoryは1件(契約7) |
| `p28_food_emergency_and_supply_chain_repair_keep_priority_over_the_wartime_factory` | Food<50はFarm、IndustrialGoods経路がなければFactory(優先度2)を先に建てる(契約8) |
| `p28_wartime_one_month_reserve_does_not_leak_into_peacetime_development` | 優先度3は従来どおり3か月予備(国庫1500 < 1000+200×3 → `FundsShortage`) |
| `p28_daily_ai_applies_the_wartime_factory_only_to_active_ai_countries_at_war` | `process_daily_country_ai`経由で、プレイヤー(戦時)・消滅国家(戦争記録上は交戦中)・平時AIは着工せず、戦時AIだけが着工する(契約2) |

### 5.2 E2E(`tests/p28_002_wartime_military_factory_construction_e2e_test.rs`、3件追加)

`GameDate`を1日ずつ進めて日次・月次メッセージを同じフレームで発行する(実時間システムと同じ進め方)。

| テスト | 内容 |
|---|---|
| `test_01_dwarf_builds_completes_and_passes_the_unchanged_offensive_gate_by_its_own_production` | 毎日、新規着工が「その日の開始時点で交戦中のAI国家」によることを確認。開戦前は着工0件。Dwarfは1800/06/05に州20 lv1を着工し、1800/09/03に完成。1801/03/24にlv2を着工。完成後の実月産は開戦時を上回る。war0のDwarf側の`blocked`がtrue→falseになった日(1801/03/01)に、備蓄が`需要×70`以上であることを確認(ゲート迂回なし)。攻勢は再開より後(1801/03/04)、最初の戦闘1801/03/08、war0は1801/03/29に攻撃側勝利(契約9)。ゲート定数40/70の不変も確認 |
| `test_02_player_country_at_war_never_receives_the_ai_wartime_factory` | プレイヤーのArcadiaはAIに宣戦されて交戦中でも着工しない。AIのDwarfは着工する(契約2) |
| `test_03_save_load_before_during_and_after_wartime_construction_matches_continuous_run` | 宣戦当日(着工前)、キュー中(1800/07/01)、完成後(1800/10/01)の3分岐でsave→loadし、GameDateとキューがload直後に保存時点と一致し、以後毎日キューのMilitaryFactoryが1件以下で、1801/04/01の全国の国庫・備蓄・建設キュー・MilitaryFactory配置・戦争状態・戦闘ID発行数が連続実行と一致(契約10) |

注入シナリオはこの実装の成功証拠に使っていない。

## 6. 自然AI運用の結果(120か月、全AI・無操作)

【実App実測】P28-002-PRE-001と同じ診断ハーネス(実カレンダー、`GameDate`を1日ずつ進める)を一時テストとして使い、計測後に削除した。現状はP28-002-PRE-001の現状基準、「監査C」は同報告書の候補C一時実測(戦時・予備1か月、開戦準備中も対象)である。

### 6.1 MilitaryFactoryの建設

| 国 | 州 | level | 着工日 | 着工後の国庫 | 完成日 |
|---|---|---|---|---|---|
| Dwarf | 20 | 1 | 1800/06/05(宣戦の翌日) | 4215.1 | 1800/09/03 |
| Dwarf | 20 | 2 | 1801/03/24 | 3403.9 | 1801/06/22 |
| Oceanic | 25 | 1 | 1801/06/28 | 9060.6 | 1801/09/26 |
| Oceanic | 25 | 2 | 1801/09/28 | 8053.3 | 1801/12/27 |
| Dwarf | 20 | 3 | 1806/05/03 | 11481.4 | 1806/08/01 |
| Ferrowyn | 26 | 2 | 1808/07/25 | 450.7 | 1808/10/23 |

計6件(Dwarf 3、Oceanic 2、Ferrowyn 1)。ArcadiaとElfinは建設しなかった。Arcadiaは主力工業州を失った後にIronが0になり(入力条件)、ElfinはIndustrialGoodsが4.9しかない(入力条件)。Passguardは交戦中でも月産が需要を上回っていた。

### 6.2 生産・需要(月次、実測)

| 国 | 建設前 | 建設後 | 備考 |
|---|---|---|---|
| Dwarf | 月産40.0 / 需要62(1800年6〜8月、毎月10〜11日が補給<0.34) | 1800年10月から月産78.3 / 需要62(P/D 1.26、補給<1.0の日0)。24か月目183.5/154.8 | 120か月平均: 月産205.36、需要187.95(P/D 1.093)。現状は39.84/60.08(0.663) |
| Oceanic | 月産12.3 / 需要31(1800年末) | 24か月目178.9 / 115.6 | 1802/02/09に降伏。その後はIronが0で稼働率0(120か月平均P/D 0.288) |
| Ferrowyn | 月産36.7 / 需要62(1808年まで毎月12〜13日が補給<0.34) | 1808年11月から月産145.8 / 需要64 | その後6 Divisionまで募兵し、119か月目は月産145.0 / 需要186(P/D 0.78) |

工場の稼働率(完成直後): Dwarf州20 lv1は0.383(労働力比0.383、物流1.0)、lv2で0.654、lv3の時点で0.714(物流比0.714)【実測、P28-002-PRE-001のスナップショット形式】。

### 6.3 備蓄日数が40日・70日を通過した日(交戦中)

- Dwarf: 70日を最初に通過したのは1801/02/01(71.63日)。軍事AIの評価は3日ごとで、その間に69.63日へ下がったため、ゲートの再開は1801/03/01の評価だった。
- Oceanic: 70日は1801/10/01(88.57日)。
- Elfin: 70日は1807/08/01(86.05日)。war6でゲートは1807/08/02に再開したが、`EnemyStronger`で攻勢に出なかった。
- Ferrowyn: 70日は1808/11/01(71.98日)。war9でゲートが1808/12/01に再開し、攻勢の結果、1808/12/16にOceanicが降伏した。
- Arcadia: 開戦直後の1800/07/01に初期備蓄で70日を一度通過した後は、生産州を失い回復しなかった。

**「70日に一度達した」ことを持続可能性とは判定しない。** Dwarfは攻勢の後も月産が需要を上回り続けた(120か月平均P/D 1.093、備蓄1172日分)。Ferrowynは増産後に需要が3倍に増え、P/Dは0.78に戻った(募兵による吸収、§10 M-2)。

### 6.4 90日・36か月・120か月

| 条件 | 90日 戦争/戦闘 | 36か月 戦争(累計/継続/終結)・戦闘・支配変更・敵州到着 | 120か月 戦争(累計/継続/終結)・戦闘・支配変更・敵州到着 |
|---|---|---|---|
| 現状(P28-002-PRE-001) | 0/0 | 2/2/0・0・0・0 | 3/3/0・0・0・0 |
| 監査C(一時実測) | 0/0 | 7/5/2・2・5・2 | 9/4/5・9・13・28 |
| **本実装** | **0/0** | **8/3/5・10・11・23** | **10/4/6・10・14・23** |

120か月の内訳(本実装): 戦闘10件(戦闘日38日、全日で攻撃側に補給>0)、実損害(manpower)合計28,088(Arcadia 3,266、Dwarf 8,790、Oceanic 12,698、Ferrowyn 3,334)、消滅Division 6、終戦6件(降伏4件: war0 Dwarf→Arcadia 1801/03/29、war2 Dwarf→Oceanic 1802/02/09、war7 Dwarf→Ferrowyn 1802/04/05、war9 Ferrowyn→Oceanic 1808/12/16。白紙講和2件: war1 Ferrowyn→Arcadia、war4 Ferrowyn→Oceanic)。消耗ループ(同一前線での反復攻勢)は発生していない(最多はwar2のDwarfで攻勢開始3回)。

監査Cとの差: 最初の戦闘が1801/09/08から1801/03/08へ早まり、戦闘・終戦は増えたが、敵州到着は28→23件に減った。原因は次の2点で、いずれも決定的に再現する。

1. 本実装は開戦準備中(平時)を対象外にした(§3)。
2. 増産見込みを本番の雇用計算と同じ整数の総必要労働力で求め、入力判定を所有州(月次生産と同じ範囲)で数えるなど、式の細部が一時パッチと違う。これによりDwarfの2件目の着工時期と募兵の時期がずれ、前線比が1.1を満たす時期が変わった。

同じ時間軸の比較ではないため、件数の大小だけで優劣は判断しない。

### 6.5 前線戦力比・stance・blocked と、未終結戦争の停止判定(120か月)

| 戦争 | 攻撃側の状態 | 防御側の状態 | 止まっている判定 |
|---|---|---|---|
| war3 Oceanic→Arcadia(1801/04/29) | 割当0、`NoValidFrontline` 2288日 | Arcadia 2 Div割当、`Stopped` | 割譲で両国の境界が消え、有効な前線がない(`border_pairs=0`) |
| war5 Passguard→Arcadia(1801/04/30) | 1 Div、`blocked=false`、前線比0.5、`EnemyStronger` 3166日 | Arcadia割当0 | 戦力不足(補給は十分) |
| war6 Arcadia→Elfin(1801/05/11) | Arcadia割当0(`NoAvailableDivision`、2 Divはwar3の前線に割当済み) | Elfin 3 Div、前線比1.5、`blocked=true`、`EnemyStronger` 2309日・`SupplyShortage` 822日 | Arcadiaは配分の問題、Elfinは補給不足(IndustrialGoods不足で工場を建てられない) |
| war8 Dwarf→Arcadia(1806/05/02) | 7 Div、前線比3.49、`blocked=true`、`SupplyShortage` 1339日。国家備蓄は1172日分 | Arcadia割当0 | **配送不健全**: Dwarfの2 Division(d9・d10)が、降伏で得た飛び地の州8(旧Oceanic首都)にいて首都と陸路で連結せず、補給充足率0。前線配送の最小値が0.34未満のためゲートが開かない |

## 7. 既存40/70日ゲートと前線110%条件の保護

- `military::recruitment_supply_gate`、`war::offensive_supply_gate`、`war::military_ai`、`military::daily_supply`、`military::ai`は変更していない(差分は`src/country/country_ai.rs`だけ)。
- E2Eの`test_01`は、ゲートの定数が40/70であることと、ゲート再開の日に備蓄が需要×70以上であることを確認する。
- P26-008-A(30件)、P26-008-B(19件)、P27-005〜007・FU-001の各E2Eは無変更で通過した。

## 8. 財政・募兵・資源入力への影響

### 8.1 120か月時点(現状 → 本実装)

| 国 | Div | MF level | 国庫 | 税率 | 生活水準 | 建物合計 | 最低国庫 |
|---|---|---|---|---|---|---|---|
| Arcadia | 2→2 | 2→1 | 8408.81→4422.94 | 0.14→0.16 | 60.38→51.72 | 25→20 | 3500→3500 |
| Elfin | 3→3 | 1→1 | 6269.94→6269.94 | 0.12→0.12 | 70.95→70.95 | 21→21 | 2000→2000 |
| Dwarf | 2→7 | 2→7 | 6170.62→15194.29 | 0.22→0.14 | 41.11→64.48 | 26→50 | 4975.5→3403.9 |
| Oceanic | 1→1 | 1→2 | 11121.98→2552.66 | 0.17→0.13 | 49.08→67.44 | 32→14 | 9582.3→2295.6 |
| Passguard | 1→1 | 2→2 | 904.06→904.06 | 0.19→0.19 | 67.01→67.01 | 9→9 | −62.9→−62.9 |
| Ferrowyn | 2→6 | 1→2 | 1960.52→3984.24 | 0.19→0.16 | 51.42→60.07 | 9→20 | 474.8→450.7 |

- 国庫が負になった国はない(Passguardの負の期間は現状と同じ既存事象)。戦時建設による最低国庫の低下はDwarf 3403.9、Ferrowyn 450.7で、いずれも正。
- 他の建設への影響: 建設キューは国家ごとに2件までで、戦時建設は1件だけ。優先度1・2は先に評価される。MilitaryFactoryを着工した月は優先度3を評価しない(単体テストで固定)。
- 募兵: 増産で募兵ゲートの備蓄条件が満たされ、DwarfとFerrowynは募兵を増やした(Dwarf 2→7 Div、Ferrowyn 2→6 Div)。Ferrowynでは増産分の多くが需要増に吸収された。
- 資源入力: MilitaryFactoryはIron 30・IndustrialGoods 15/levelを消費する。建設は1か月分の備蓄がある国に限られ、Dwarfの120か月の備蓄はIron 612.8、IndustrialGoods 3495.8。

### 8.2 既存正規値の更新(`p27_002_c` `test_05`)

`test_05`の元の目的は、P27-002-Cの財政自己安定化が120か月の実データで成立することの確認である。受け入れ基準(全国で税率振動0〜3回、Ferrowynの国庫が一度も負にならない、生活水準>40、国庫>300、Passguardが黒字転換、Oceanicが過剰富裕化しない、6か国とも評価対象)に加え、監査時の実測値との完全一致(振動回数、120か月の税率・国庫・月次収支)を回帰保護として置いている。

P28-002では、受け入れ基準のアサーションは**すべて無変更で通過した**。完全一致の正規値は、戦争の決着(降伏・割譲)と戦時建設の建設費による意図した因果で変わった。このharness(30日×12か月の簡略暦)では、1801/11/14にArcadiaが、1802/01/27にOceanicが降伏し、州0・州8をDwarfへ割譲した。Ferrowynも戦時建設を行った。

| 項目 | 旧値 | 新値 | 原因 |
|---|---|---|---|
| 税率振動回数 [Arcadia, Elfin, Dwarf, Oceanic, Passguard, Ferrowyn] | [0, 0, 0, 1, 3, 0] | [2, 0, 2, 3, 3, 0] | 領土・建物の移動で収支が変わった3国が増加。全国で上限3以内 |
| Arcadia (税率, 国庫, 月次収支) | (0.140, 8340.77, 42.85) | (0.160, 4365.70, 25.67) | 降伏・州0割譲 |
| Dwarf | (0.220, 6170.62, 35.69) | (0.170, 14536.65, 215.39) | 戦時建設・州0/州8の獲得 |
| Oceanic | (0.200, 10778.56, −16.42) | (0.130, 4761.01, 53.85) | 降伏・州8割譲 |
| Ferrowyn | (0.190, 1960.52, 18.22) | (0.200, 693.24, 17.97) | 戦時建設の建設費1200 |
| Elfin・Passguard | (0.120, 6425.41, 24.31)・(0.190, 904.06, 12.43) | 同じ | 戦時建設も領土変化もない |

財政ロジック自体の回帰ではないことを分けて固定するため、狭いテスト`test_18_countries_untouched_by_wartime_construction_keep_the_p27_002_c_fiscal_canon`を追加した。戦時建設を一度も行わず、領土・MilitaryFactoryも変わらないElfin・Passguardについて、120か月間キューにMilitaryFactoryが入らないこと、領土が不変であること、P27-002-Cの旧正規値(税率・国庫・月次収支・振動回数)と完全一致することを確認する。

これ以外の既存テストの期待値は変更していない。P28-002-PRE-002で懸念した「他の長期ファイルでの範囲外の月によるpanic」は、全体テストで1件も発生しなかった。

## 9. Save V1・決定性・プレイヤーと消滅国家の保護

- Save: 新しい永続フィールド・migration・enum値はない。建設は既存の`ConstructionQueueItem`だけを使い、判断理由は既存の`WarInProgress`を再利用した。`save::`の単体テスト305件は無変更で通過した。
- save→load: E2E `test_03`で、着工前・キュー中・完成後の3分岐とも連続実行と一致し、キューの重複もなかった。
- 決定性: 州は`StateId`昇順、建物は`BuildingType::ALL`順、総必要労働力は整数で合算し、HashMapの反復順に依存しない。登録順を反転しても選択が同じであることを単体テストで確認した。120か月の診断を独立プロセスで2回実行し、ラベル以外の出力が一致した。新E2Eも独立プロセスで2回実行し、いずれも3件通過した。
- プレイヤー・消滅国家: 日次ループの除外(既存)により新経路を通らない。単体テスト(`process_daily_country_ai`経由)と、プレイヤーが交戦中になるE2E(`test_02`)で確認した。
- P27-005の手動プレイ確認: **未実施のまま引き継ぐ**。

## 10. Critical／High／Medium／Low

```text
Critical: 0
High: 0
Medium: 3
Low: 4
```

- **High H-1(P28-001から継続)は解消**: 自然運用で、自国生産による備蓄回復→既存ゲートの正規の再開→攻勢→戦闘→降伏の経路が成立した。
- **Medium M-1(継続)**: 戦闘を伴わない終戦経路がない。120か月時点で4戦争が継続している(§6.5)。
- **Medium M-2(継続)**: 募兵ゲートは備蓄量だけを見る。増産が募兵に吸収され、FerrowynのP/Dは0.78に戻った。
- **Medium M-3(新規)**: 降伏・割譲で得た飛び地(例: Dwarfの州8)にいるDivisionは首都と連結せず補給0になる。これが前線配送の最小値を0.34未満に落とし、国家備蓄が1172日分あってもゲートが開かない(war8)。P26-006の首都連結補給とP26-008-Bの前線配送判定の組み合わせによる現象で、本タスクの範囲外。
- **Low L-1**: 単一回廊(継続)。
- **Low L-2(継続)**: 攻勢判定の敵戦力が敵国の全Divisionを数える。
- **Low L-3(新規)**: 入力資源が構造的に不足する国(ElfinのIndustrialGoods、失地後のArcadiaとOceanicのIron)は戦時建設できない。入力条件は意図した保護なので不具合ではないが、戦時の供給網修復は既存の優先度2・3に依存する。
- **Low L-4(P28-002-PRE-001・PRE-002から継続)**: 本番の`GameDate::add_days`は範囲外の月でpanicする。同じ暦を持つ8つの長期テストは未修正だが、本タスクの全体テストでは発火しなかった。

### 次の原子的タスク

**P28-003-PRE-001: 自然AI運用で恒久化する残存戦争の停止原因の設計監査**。本実装後の120か月で残った4戦争(§6.5)について、次の3つの原因ごとに、影響する戦争の数・期間・財政への影響を定量化し、次に実装する1件を決める。

- M-1: 非戦闘の終戦経路がない
- M-3: 飛び地のDivisionが前線配送を塞ぐ
- M-2: 募兵がフローを見ない

## 11. 実行コマンドと正確な結果

Cargoはすべて逐次実行した(並列起動なし)。

- `git rev-parse HEAD` → `7b3f406ce6d9d00de41a7756b1d7438c7ef87f9a`、`git status --short` → 空
- `cargo build -q` → 成功
- `cargo test -q -j 2 --lib country::country_ai::tests::p28` → 1回目 11 passed / 1 failed。新規テスト自身の許容誤差(f32の0.9で110.0000024)が原因で、1e-4へ修正した。修正後の`--lib country::country_ai` → 63 passed / 0 failed
- `cargo test -q -j 2 --test p28_002_wartime_military_factory_construction_e2e_test` → 1回目はテストのコンパイルエラー(借用)。2回目は`test_01`が失敗した。原因は観測ヘルパーが1日ごとに既出一覧を作り直していたテスト側の不具合で、実装の問題ではない(キュー中の建設を終戦後に再検出していた)。観測を永続する構造体に直し、着工日の判定を「その日の開始時点で交戦中」にした後は3 passed
- 一時診断(全AI・120か月、2回、独立プロセス) → 戦争10、終戦6、戦闘10、敵州到着23、支配変更14。ラベル以外の出力が一致
- 実装後の全体テスト(一時診断を含む) → 2724 passed / 1 failed(`p27_002_c` `test_05`)/ 1 ignored(84バイナリ)
- `test_05`の新しい値は`--nocapture`の出力で取得した。振動回数の全6か国分は、完全一致の比較を一時的に無効にした使い捨ての実行で取得し、ファイルはすぐ元に戻した(内容の差分0を確認)。harness内の戦争・割譲の経過も、同じ方法で一時的に出力して確認した
- `test_05`更新と`test_18`追加後 `cargo test -q -j 2 --test p27_002_c_residual_fiscal_self_stabilization_e2e_test` → 18 passed
- 個別実行(一時診断の削除後): `p28_002_wartime_military_factory_construction_e2e_test` 3(2回とも)、`p26_004_pre_002` 11、`p26_008_a` 30、`p26_008_b` 19、`p26_008_c` 18、`p26_003` 49、`p26_004_daily_supply` 19、`p26_006` 18、`p26_det_001_construction_ai_determinism` 10、`p27_002_a` 17、`p27_002_b` 18、`p27_002_c` 18、`p27_004` 6、`p27_005` 10、`p27_006` 17、`p27_007` 2、`p27_fu_001_ai_retreat_alternative` 3、`p27_fu_001_elimination_peace` 3、`land_war_combat_peace_test` 4(無変更)。`--lib`: `country::country_ai` 63、`economy::` 58、`military::` 508、`war::` 181、`save::` 305。いずれも0 failed
- 1回目の最終検証: `cargo test --all-targets --no-fail-fast -j 2` → 2725 passed / 0 failed / 1 ignored(83バイナリ)。続く`cargo clippy`で、入力判定の`!(stock >= need)`(NaNを不足扱いにする意図の否定比較)が`clippy::neg_cmp_op_on_partial_ord`で1件エラー。`stock.is_nan() || stock < need`へ同じ意味のまま書き換え、clippy・全体テストを再実行した
- 最終 `cargo test --all-targets --no-fail-fast -j 2` → **2725 passed / 0 failed / 1 ignored**(83バイナリ、exit 0)。開始時2709に、単体テスト12件・E2E 3件・`p27_002_c` `test_18` 1件を加えた数。1 ignoredは既存の`profile_p21_021_elimination_scaling`。再生成された既知の非決定的PNG9件は個別に復元
- 最終 `cargo clippy --all-targets --all-features -- -D warnings` → 警告0(exit 0、修正後に再実行)

## 12. 保護事項

- 本番コードの変更は`src/country/country_ai.rs`だけ。RONデータ・Save形式・System順序・攻勢ゲート・募兵ゲート・前線比・宣戦判定・補給・戦闘・講和の式は無変更。
- `saves/savegame_v1.ron`: 読み書き・復元・ステージなし(テストはすべて一時ディレクトリのSaveを使う)。
- `tests/land_war_combat_peace_test.rs`: 変更・整形なし。ワークスペース全体のformatなし。既存報告書は変更していない。
- 一時診断テスト(`tests/p28_002_temp_diagnostic.rs`)は計測後に削除した。
- テストで再生成された既知の非決定的PNG9件は、`git status --short`で確認してから1件ずつ個別に復元した。

## 13. 最終`git status --short`

コミット前: `M src/country/country_ai.rs`、`M tests/p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs`、`?? tests/p28_002_wartime_military_factory_construction_e2e_test.rs`、`?? verification_logs/phase-28/p28-002/`。コミット後は空。
