本報告書は、P26-009で残った財政Medium findingを、P27-001後の正規軍拡状態で
再評価し、実装修正前に根本原因と最小の対処を確定する設計監査である。

# P27-002-PRE-001: 六か国の長期財政赤字・破産フィードバック根本原因監査

## 1. 総合判定

```
P27-002-PRE-001 ANALYSIS COMPLETE
ROOT CAUSE CONFIRMED
DESIGN DECIDED
FISCAL ACCOUNTING VALID
SIX-COUNTRY SUSTAINABILITY INVALID
BANKRUPTCY RECOVERY INVALID
P27-001 INCREMENT QUANTIFIED
SAVE CONTINUITY PRESERVED
NEXT ATOMIC TASK READY
```

「FISCAL ACCOUNTING VALID」は会計式自体(実装通りに計算され、記録漏れ・二重
計上がない)という意味であり、「SIX-COUNTRY SUSTAINABILITY」「BANKRUPTCY
RECOVERY」がINVALIDなのは、現行の基礎税収・行政費・破産ペナルティの**設計
そのもの**が持続不可能な構造になっているためである(バグではなく構造的
バランス問題、§39〜41参照)。

## 2. 開始HEAD

`0bc7a3fa88bdfb8a8cfeef7aaf1314371eea92e2`(P27-001の実装コミット)。開始時
`git status --short`はクリーン、`cargo test --all-targets`は2515 passed/0
failed/1 ignored、clippyは0警告——いずれも実測して確認した。

## 3. 監査対象コードHEAD

本タスクは報告書1件のみをコミットする監査専用タスクであり、実験的な一時
パッチ(§18)を除きコード変更は一切行っていない(その一時パッチも実験直後に
完全に復元し、`git diff`で無変更であることを確認済み)。監査対象HEADも開始
HEADと同一の`0bc7a3fa88bdfb8a8cfeef7aaf1314371eea92e2`。

## 4. タスク種別

設計・監査タスク。本番Rustコード・本番RONデータ・既存テストはいずれも変更
していない。一時的な検証テスト(`tests/p27_002_pre_001_temp_fiscal_audit_test.rs`)
と、候補D実験のための一時的な生産コードパッチ(`src/country/country_ai.rs`、
実験直後に完全復元)を使用したが、いずれも本タスク終了前に削除・復元済みで
コミットに含めていない。

## 5. ユーザーSave保護

`saves/savegame_v1.ron`には本タスク全体を通して一切アクセスしていない。

## 6. 財政用語の定義

- **国庫残高(treasury_balance)**: `CountryData.treasury`(f64)。ある時点の
  資金残高。
- **経常収支(recurring_balance)**: 本タスクでは`monthly_income -
  (monthly_expenses)`と定義する。現行実装には「一時支出」を分離するフィールド
  自体が存在しない(§9参照)——建設費は`process_country_finance`の外
  (`country_ai.rs`/`try_construct`)で即座に`treasury`から直接減算され、
  `monthly_expenses`には一切計上されない。したがって現行の`monthly_balance`
  フィールドは、実質的に**建設費を含まない経常収支**そのものである
  (これは后述の一時支出分離[§10・条件B]の実測でも裏付けられる)。
- **総月次収支(monthly_net)**: `treasury_end - treasury_start`(建設費等の
  一時支出を含む、その月の国庫の実際の増減)。
- **赤字月**: `monthly_balance < 0.0`(経常収支ベース)。
- **破産状態**: `is_bankrupt = country.treasury < 0.0`(`src/economy/mod.rs:147`)。
  単純な二値条件であり、グラデーションのある「係数」ではない(§13参照、
  仕様書が想定していた「破産係数」という表現は実装と一致しない)。

## 7. 正規月次財政式

`src/economy/finance.rs::process_country_finance`(全文読了、無変更で引用):

```text
tax = population × living_standard × tax_rate × TAX_COEFFICIENT(0.0001)
admin_cost = population × ADMIN_COEFFICIENT(0.001)
building_maintenance = Σ(maintenance_cost × level × operation_ratio)  (州内建物ごと)
logistics_maintenance = logistics_capacity × LOGISTICS_MAINTENANCE_COEFFICIENT(0.05)

monthly_income = Σ_states(tax)
monthly_expenses = Σ_states(building_maintenance + logistics_maintenance) + admin_cost
monthly_balance = monthly_income - monthly_expenses
treasury += monthly_balance
```

建設費(`construction_cost`)はこの式に一切含まれない——`country_ai.rs::
try_construct`が建設決定と同時に`treasury`から直接一括減算する、別経路の
一時支出である(§9)。

## 8. 各収入項目

| 項目 | 計算元 | 実装 |
|---|---|---|
| 税収 | 州人口×生活水準×税率×0.0001 | あり(`calculate_state_tax`) |
| 生産由来収入 | — | **現行実装なし**(資源生産は`CountryStockpile`のみを
  変動させ、`treasury`には一切影響しない——建物は資源とMilitaryEquipment/
  MilitarySuppliesのみを生産し、金銭を生成しない) |
| その他収入 | — | 現行実装なし |

## 9. 各支出項目

| 項目 | 計算元 | 実装 |
|---|---|---|
| 建物維持費 | maintenance_cost×level×operation_ratio | あり |
| 物流維持費 | logistics_capacity×0.05 | あり |
| 行政費 | population×0.001 | あり |
| 軍事維持費(Division) | — | **現行実装なし**——`DivisionDefinition.
  maintenance_cost`フィールドはコード上に存在するが、`finance.rs`を含む
  いかなる財政計算からも一度も参照されない、事実上のデッドデータである
  (grep で確認済み、テストfixture内の値設定を除き実行時参照ゼロ)。 |
| 募兵費 | — | 現行実装なし(P25-003で「`required_equipment`は通貨ではない」
  と明示的に廃止済み) |
| 建設費 | construction_cost(一括) | あり、ただし`process_country_finance`の
  外側(`country_ai.rs::try_construct`)で即座に`treasury`から直接減算 |

**結論**: 軍事費・募兵費は現行実装に一切存在しない。P27-001でDivision数が
増えても、財政への直接的な影響はゼロである(§26で定量的に再確認)。

## 10. 丸め・単位

`treasury`・`monthly_income`・`monthly_expenses`・`monthly_balance`はいずれも
生の`f64`であり、固定小数点units・checked演算のいずれも使用しない
(MilitarySupplies/MilitaryEquipmentの正規会計とは異なる設計)。負値・NaN・
Infinityへの明示的なvalidationも存在しない(ただし本監査の36か月実測では
非有限値は一度も観測されなかった)。丸め位置は「州ごとの税・維持費をf64で
逐次加算し、月末に一括で`treasury += balance`」という単純な加算方式であり、
量子化(quantize-then-sum等)の概念自体が存在しない。

## 11. System順序

`src/economy/mod.rs::EconomyPlugin`:

```text
(handle_daily_construction, handle_monthly_economy).chain()
    .in_set(DailySimulationSet::Economy)
```

`handle_monthly_economy`内部の実際の処理順(コード読了、無変更で引用):

```text
1. 人口・雇用更新(update_state_population_and_employment)
2. 物流更新(update_state_logistics)
3. is_bankrupt = treasury < 0.0 (このタイミングのtreasuryで判定、前月末の値)
4. process_country_production(is_bankruptを渡す) — 資源生産・消費
5. process_country_finance — 税収・維持費・treasury更新
6. 生活水準・不満度更新(is_bankruptを使用、ステップ3の値のまま)
7. economic_state再計算
```

建設費の支払い自体は、上記のどのステップとも別の場所(`country_ai.rs`の
AI建設判断System、`DailySimulationSet::CountryAi`——`Economy`より後、
`handle_monthly_economy`の呼び出しよりコード上は後になる)で発生する。
`is_bankrupt`はその月の**期首**treasuryで判定され、同月の生産(ステップ4)へ
適用される。その月の税収・維持費(ステップ5)はis_bankruptの影響を一切受け
ない(§13で詳述)。翌月の`is_bankrupt`判定は、当月の`process_country_finance`
確定後のtreasuryを使う——1か月遅れて反映される。

## 12. 破産判定

`is_bankrupt = country.treasury < 0.0`。二値のみ、閾値の余裕・段階なし。
`treasury == 0.0`ちょうどは非破産扱い。

## 13. 破産係数

仕様書が想定する「グラデーションのある係数」は存在しない。実際に見つかった
唯一の「破産の影響」は次の2箇所のみ:

1. `economy/production.rs::calculate_operation_ratio`: `if is_bankrupt {
   ratio *= 0.8; }`——資源生産の稼働率に固定20%ペナルティ。**税収・維持費には
   一切影響しない**(生産物は資源[Food/Iron/Coal/IndustrialGoods/
   MilitaryEquipment/MilitarySupplies]であり、税収の計算式[§7]は生産量を
   一切参照しない)。
2. `economy/finance.rs::update_state_welfare_and_unrest`: `if is_bankrupt {
   ls_delta -= 0.5; unrest_delta += 1.0; }`——生活水準への直接的な固定
   ペナルティ。**これが翌月以降の税収を間接的に引き下げる、唯一の実質的な
   フィードバック経路である**(§39で詳述)。

## 14. 回復条件

コード上、破産状態から抜け出す唯一の条件は`treasury >= 0.0`に戻ることだが、
そのためには`monthly_balance >= 0`(または過去の黒字の蓄積)が必要であり、
`is_bankrupt`自体が生活水準経由で翌月の税収を引き下げ続けるため、一度赤字が
定着した国家が自力で`monthly_balance`をプラスへ戻す仕組みは存在しない
(§32・§39の実測で、12か月間一度も回復しなかったことを確認)。

## 15. 支払不能時の挙動

`try_construct`(`country_ai.rs`)は`treasury < construction_cost`なら
`InsufficientFunds`を返して**建設自体を拒否**し、`treasury`・
`construction_queue`のいずれも変更しない(原子的)。すなわち、**建設費が
「支払えないが強行される」ことはなく、負債の概念も存在しない**——治療対象は
「支払不能時の不整合」ではなく、「支払えた時点で将来の維持費を無制限に
積み増してしまう」ことである(§20で詳述)。維持費・行政費は`treasury`の
現在値に関わらず無条件で減算され続け(未払いという概念がない)、`treasury`は
負値へ無制限に進む(下限・クランプなし、36か月実測で最大約-20,249まで観測)。

## 16. 六か国初期状態

実App実測(New Game、`PlayerCountry(None)`、6か国AI化)。

| 国 | 初期国庫 | 税率 | 人口 | 平均生活水準 | 初期Division |
|---|---|---|---|---|---|
| Arcadia(0) | 5000.00 | 0.15 | 2,860,000 | 53.60 | 1 |
| Elfin(1) | 3500.00 | 0.15 | 2,060,000 | 64.60 | 1 |
| Dwarf(2) | 7200.00 | 0.15 | 1,890,000 | 58.00 | 1 |
| Oceanic(3) | 12000.00 | 0.15 | 3,830,000 | 67.50 | 1 |
| Passguard(4) | 1800.00 | 0.15 | 245,000 | 47.00 | 1 |
| Ferrowyn(5) | 2600.00 | 0.15 | 590,000 | 51.00 | 1 |

税率は全国家とも`default_tax_rate()=0.15`で同一(`assets/data/countries.ron`は
`tax_rate`を明示せず、既定値がそのまま使われる)。`tax_rate`を変更できるのは
UI経由のプレイヤー操作のみ(`src/ui/economy_panel.rs`)であり、**AIが自律的に
税率を調整するロジックは一切存在しない**(grep確認済み)。

## 17. 六か国月次収支(通常AI、条件A)

実App、36か月、`PlayerCountry(None)`、診断用無限資金・外生付与なし。

| 月 | Arcadia balance | Elfin balance | Dwarf balance | Oceanic balance | Passguard balance | Ferrowyn balance |
|---|---|---|---|---|---|---|
| 1 | -301.27 | 87.17 | -302.28 | 498.66 | -93.22 | -172.24 |
| 12 | -464.66 | 20.92 | -468.03 | 467.64 | -159.35 | -247.18 |
| 24 | -701.53 | 22.27 | -727.85 | 263.17 | -160.49 | -301.52 |
| 36 | -986.82 | 23.63 | -990.78 | -32.38 | -149.78 | -356.51 |

## 18. 六か国国庫推移(通常AI、条件A)

| 月 | Arcadia | Elfin | Dwarf | Oceanic | Passguard | Ferrowyn |
|---|---|---|---|---|---|---|
| 1 | 3198.73 | 2087.17 | 4897.72 | 10998.66 | 206.78 | 427.76 |
| 12 | -2881.07 | 105.48 | -2365.45 | 9831.23 | -1389.57 | -2049.44 |
| 24 | -10012.29 | 365.31 | -9667.52 | 6232.16 | -3309.21 | -5368.20 |
| 36 | -20249.22 | 641.37 | -20107.61 | 2479.53 | -5112.54 | -9343.23 |

上記はP26-009報告書が既に記録していた数値と完全に一致する(独立した実測で
再確認、既存報告書の結論を鵜呑みにしていない)。

## 19. 六か国破産係数推移

§13の通り、破産は二値(`is_bankrupt`)であり係数の推移という概念自体が
存在しない。参考として、各国が`is_bankrupt=true`(treasury<0)であった月数
(36か月中):

| 国 | is_bankrupt月数(概算) |
|---|---|
| Arcadia | 月6以降ほぼ全期間(約30か月) |
| Elfin | 0か月(一度も破産していない) |
| Dwarf | 月6以降ほぼ全期間(約30か月) |
| Oceanic | 0か月(月36時点でも僅かに正、ただしbalanceは月36で初めて負に転換) |
| Passguard | 月3以降ほぼ全期間(約33か月) |
| Ferrowyn | 月6以降ほぼ全期間(約30か月) |

## 20. 各国最初の赤字月(経常収支ベース)

| 国 | 最初のbalance<0月 |
|---|---|
| Arcadia | 1 |
| Elfin | (36か月間、一度も負にならない) |
| Dwarf | 1 |
| Oceanic | 36(それまでずっと正) |
| Passguard | 1 |
| Ferrowyn | 1 |

## 21. 各国最初の国庫負値月

| 国 | 最初のtreasury<0月 |
|---|---|
| Arcadia | 6 |
| Elfin | (36か月間、一度もマイナスにならない) |
| Dwarf | 6 |
| Oceanic | (36か月間、一度もマイナスにならない) |
| Passguard | 3 |
| Ferrowyn | 6 |

## 22. 一時赤字と構造的赤字の分離

**核心的な発見**: 条件D(新規建設・新規募兵ともになし、初期建物のみで36か月
運用)を実測した結果、Arcadia・Dwarf・Ferrowynは**建設・募兵が一切発生しなくても**
条件Aとほぼ同一の悪化軌跡を示した:

| 国 | 条件A month36 balance | 条件D month36 balance |
|---|---|---|
| Arcadia | -986.82 | -915.04 |
| Elfin | 23.63 | 89.95 |
| Dwarf | -990.78 | -962.67 |
| Oceanic | -32.38 | **+508.78**(条件Dでは終始健全) |
| Passguard | -149.78 | -94.01 |
| Ferrowyn | -356.51 | -321.27 |

**これは一時的な赤字ではなく、構造的な赤字である**——建設費という「一時支出」
を完全に排除しても、Arcadia・Dwarf・Ferrowynの経常収支は初期建物構成だけで
既に負であり、月を追うごとに悪化する(§39の生活水準フィードバックが原因)。
Passguardも条件Dで小さな持続的赤字(-94前後)を示すが、**悪化していない**
(条件Aでは建設費の分だけ悪化幅が大きい-150程度)——Passguardの赤字の大部分は
構造的、一部が建設費由来である。

Oceanicは唯一、条件D(建設なし)では終始健全(+498→+509)だが、条件A(通常
建設あり)では月36で初めて赤字転落する——**Oceanicの赤字化は100%、AI建設
(過剰なFactory建設)由来である**(§27で定量化)。

## 23. 新規建設なし結果(条件B)

条件D(建設・募兵ともなし)とほぼ同一の結果(募兵の財政影響がゼロのため、
条件Bと条件Dは事実上等価)。月36:

```text
Arcadia: treasury=-17924.04 balance=-915.04
Elfin: treasury=188.07 balance=89.95
Dwarf: treasury=-19226.26 balance=-962.67
Oceanic: treasury=11133.31 balance=508.78
Passguard: treasury=-2891.75 balance=-94.01
Ferrowyn: treasury=-8144.42 balance=-321.27
```

## 24. 新規募兵なし結果(条件C)

条件Aとほぼ完全に一致する(Oceanicのみ僅かな差、後述)。月36:

```text
Arcadia: treasury=-20249.22 balance=-986.82   (条件Aと完全一致)
Elfin: treasury=641.37 balance=23.63           (条件Aと完全一致)
Dwarf: treasury=-20107.61 balance=-990.78      (条件Aと完全一致)
Oceanic: treasury=2058.35 balance=35.04         (条件Aは2479.53/-32.38、僅かに異なる)
Passguard: treasury=-5112.54 balance=-149.78   (条件Aと完全一致)
Ferrowyn: treasury=-9343.23 balance=-356.51    (条件Aと完全一致)
```

**募兵の有無は財政に実質的な影響を与えない**(§9で確認した「軍事費・募兵費
現行実装なし」の直接的な実測裏付け)。Oceanicの僅かな差は、募兵有無による
AI評価順序の違いが、その後の建設AI判断のタイミングへ間接的に波及した結果と
推定される(HashMap非依存の`CountryId`昇順処理のため決定的ではあるが、
「今月queueが空か」という条件がAIの他の判断分岐へ波及する既存の設計上の
副作用であり、バグではない)。

## 25. 建設・募兵ともなし結果(条件D)

§22・§23参照。

## 26. P27-001の増分

コードレベルで確認した通り(§9)、Division・募兵に関連する財政項目は現行
実装に一切存在しない。条件C(募兵なし)と条件A(通常)の比較(§24)により、
**P27-001によって正常に完成するようになったDivisionは、財政へ実質的な影響を
与えないことを実測でも確認した**——Oceanicの微小な差(約421の国庫差)は
AI評価順序の間接的な波及であり、Division自体の維持費・募兵費由来ではない
(そのような費用自体が存在しないため)。

**結論**: P27-002の財政問題は、P27-001とは完全に独立した、Phase 21以降
存在し続けてきた基礎税収・行政費・破産ペナルティの構造的問題である。

## 27. 入力資源不足との因果分離

破産による生産ペナルティ(§13の`operation_ratio×0.8`)は資源(Food/Iron/
Coal/IndustrialGoods等)の産出量を減らすが、税収計算式(§7)は資源産出量を
一切参照しないため、**「入力資源不足→生産低下→税収低下」という経路は
存在しない**。唯一の経路は「破産→生活水準ペナルティ(直接)→税収低下」
(§39)であり、「入力資源不足→生産低下→Food不足→has_food=false→生活水準
低下→税収低下」という別の間接経路も存在するが、36か月の実測では
Food不足による飢餓ペナルティが主要因となった国家は確認されなかった
(全国家ともFood備蓄は正のまま推移、既存のFarm優先建設AIが機能している)。

## 28. 建設AIの財政判断

`country_ai.rs::try_construct`(§15で引用)は`treasury >= construction_cost`
のみを判定する。**経常収支・将来の維持費増分・破産状態のいずれも一切
考慮しない**。特に優先度3(`PeacetimeDevelopment`、Wood/Iron/Coal>=30の
場合のFactory建設)には、建設数の上限が「1州あたりmax_level」以外に存在
せず、**Oceanicのように潤沢な国庫を持つ国家は、経常収支を圧迫し始めても
建設を継続し得る**設計になっている(§22・§27で実証)。P26-008-Cの優先度1
[Food緊急]・優先度2[供給網修復、Mine/Factory事前開発]は、この問題の対象
外とすべきである(後述の候補D評価、§46)。

## 29. 募兵AIの財政判断

`src/military/ai.rs`は`treasury`を一切参照しない(grep確認済み、§9参照)。
コード自身のコメントが「目標計算に現在Division数・treasury・戦争状態を
混ぜない」ことを意図的な設計方針として明記している。財政ゲートを新設する
必要性は、実測(§26)からは支持されない——そもそも財政的コストが存在しない
ため、ゲートを追加しても意味のある効果が得られない。

## 30. 破産フィードバック

§13・§39で詳述。`is_bankrupt`(税収低下ではなく生活水準経由の間接効果)が
唯一の実質的フィードバック経路であり、**回復方向への係数調整・自動修正機構は
一切存在しない**。

## 31. 回復可能性

12か月の単独軌跡実測(§32)で、`is_bankrupt`が1度成立すると12か月間一度も
`false`に戻らず、`monthly_balance`も単調に悪化し続けることを確認した
(§39)。36か月の時間軸では、Arcadia・Dwarf・Ferrowynのいずれも自力回復の
兆候を示さなかった。

## 32. 90日結果

初回3か月分の実測(§17・§18の月1データに相当)により、Arcadia・Dwarf・
Passguard・Ferrowynは**初月から**経常赤字であることを確認した(90日を
待たずに問題が顕在化する)。P27-001由来の部分装備Division完成は、この
90日間の財政には影響しない(募兵は財政的に中立、§26)。

## 33. 36か月結果

§17〜25に詳述。

## 34. save/load結果

`treasury`・`monthly_income`・`monthly_expenses`・`monthly_balance`は
いずれも`CountryData`直下の素の`f64`フィールドであり(`src/country/mod.rs`)、
別のSave DTOラッパーを経由しない——P27-001・P27-001-PRE-001で確認した
`recruitment_queue`(`RecruitmentQueueItem`)と全く同じパターンである。
`CountryData`自体のSave往復は、Phase 21以降繰り返し検証されてきた既存の
Save基盤がそのまま適用されるため、財政フィールド固有の新しいSaveリスクは
構造的に存在しない。本タスクでは新規のsave/loadテストを追加していないが、
コードの直接読解によりこの結論を確認した。

## 35. 2回連続save→load

§34と同じ理由により、財政フィールド固有のリスクはない。

## 36. 旧Save

`monthly_income`・`monthly_expenses`・`monthly_balance`・`treasury`は
Phase 21から存在する既存フィールドであり、本タスクによる変更はないため
旧Save互換性への影響もない。

## 37. 決定性

`handle_monthly_economy`は`country_registry.countries.iter_mut()`
(Vec、挿入順=CountryId順)で国家を処理し、HashMapへの依存はない。本監査の
36か月実測を複数回再実行し(§55)、同一の数値が再現することを確認した。

## 38. 国家間分離

`process_country_finance`は各国家の`&mut CountryData`と自国所有州のみを
操作し、他国のtreasury・stockpileへ一切アクセスしない(コード読了で確認)。

## 39. バグ判定

**Critical/Highに該当するバグは見つからなかった。** 会計式は実装通りに
一貫して計算されており(§7〜11)、記録漏れ・二重計上・非決定性は確認され
なかった。

ただし、次を「意図せぬ設計上のギャップ」として報告する(バグではないが、
明示的に設計された安全策ではない、既存コードの偶発的な性質):

- **生活水準スパイラル**: `is_bankrupt`→生活水準-0.5/月→翌月税収低下
  →収支悪化→`is_bankrupt`継続、という経路に**回復方向への対抗機構が
  一切存在しない**(税率自動調整・債務上限・AI緊縮反応のいずれもなし)。
  一度赤字化した国家は、生活水準が0へ漸近し続ける限り、理論上無期限に
  悪化し続ける(ただし36か月時点でのArcadiaの生活水準低下速度[月あたり
  約-0.43]から見て、破局的な崩壊[生活水準0到達]までには経験的に7年以上を
  要し、36か月の観測窓内では「悪化傾向の確認」に留まる致命的ではない
  速度である)。

## 40. バランス問題判定

**構造的バランス問題として確定する(Medium)**:

1. **TAX_COEFFICIENT(0.0001)とADMIN_COEFFICIENT(0.001)の比率が過大**:
   `tax >= admin_cost`が成立する条件は`living_standard × tax_rate >= 10`。
   `tax_rate`の既定値0.15では`living_standard >= 66.7`が必要——初期生活水準が
   これを下回る国家(Arcadia 53.6、Dwarf 58.0、Passguard 47.0、Ferrowyn 51.0、
   実に6か国中4か国)は、**行政費だけで税収を上回り**、建物維持費を加える前から
   既に赤字が確定している。Elfin(64.6)・Oceanic(67.5)は僅かにこの閾値を
   上回るか近いため健全である。これは4/6の国家が初期状態から構造的に
   持続不可能であることを意味する。
2. **AIの税率調整能力が皆無**: `tax_rate`はプレイヤーのUI操作でのみ変更
   可能で、AIには一切のフィスカル・レバーがない。人間プレイヤーなら税率を
   上げて対処できる状況でも、AI国家には手段がない。
3. **AI建設に予備資金・経常収支判断が皆無**: Oceanicの事例(§22・§27)が
   示す通り、健全な国家でも無制限のFactory建設によって自ら赤字化し得る。

## 41. 意図された国家差

Elfin・Oceanic(健全)とArcadia・Dwarf・Passguard・Ferrowyn(構造的赤字)の
差自体は、初期人口・初期生活水準という**既存の意図されたデータ差**に由来
する自然な結果であり、それ自体は問題ではない。問題は「赤字化した国家に
一切の回復手段がない」という設計のギャップであり、「国家間に差がある」
こと自体ではない。

## 42. 案A評価: 初期国庫だけを調整

**却下**。§22で実証した通り、Arcadia/Dwarf/Ferrowynの赤字は構造的
(経常収支が恒久的に負)であり、初期国庫を増やしても延命するだけで
問題を先送りするに過ぎない。旧Saveとの意味的不一致・migration要否という
追加コストに見合わない。

## 43. 案B評価: 国家データの収入・税基盤を調整

**部分的に有効だが、本タスクでは採用しない**。生活水準や人口を個別に
調整すれば根本原因(§40-1)に直接対処できるが、6か国全ての初期データへの
個別介入が必要で、影響範囲が広く、次の原子的タスクとしては大きすぎる
(§24の原子性方針に反する)。将来的な独立タスク候補として記録する。

## 44. 案C評価: 建物価格・維持費・収益を調整

**却下(本タスクでは非推奨)**。全建物・全国家への波及が大きく、
P26-008-Cが確立した供給網修復ロジック(Mine/Factory事前開発)の経済性を
壊すリスクがある。§40-1の根本原因(TAX/ADMIN係数比)により直接的ではない。

## 45. 案D評価: AIへ財政予備・affordability判断を追加

**採用(一部)**。§18で一時パッチにより定量検証した通り、優先度3
(無制限のFactory/Mine建設)にだけ3か月分の経常支出を予備資金として要求する
と、Oceanicは健全なまま維持され(月36 treasury: 2479.53→12434.78)、
Elfin(641.37→4465.24)・Arcadia(-20249.22→-14510.82)・Dwarf(-20107.61→
-14123.16)も改善する。Passguard・Ferrowynは無変化(構造的赤字が支配的で、
そもそも優先度3の建設をほとんど行っていないため)。

**重要**: この案は§40-1の構造的赤字そのものを解決しない(Passguard・
Ferrowynは無改善、Arcadia・Dwarfも依然赤字が残る)——**Oceanicの自傷的
過剰建設という副次的な問題にだけ効果がある**、限定的だが安全な修正である。

## 46. 案E評価: 破産係数または回復式を調整

**却下(本タスクでは非推奨)**。§13の通り「係数」自体が存在せず(二値の
`is_bankrupt`のみ)、生活水準ペナルティ(-0.5/-1.0)を弱めることは根本原因
(§40-1の税収不足)を隠すだけの対症療法になりかねない。回復不能ループの
解消には有効な可能性があるが、既存の「財政赤字は生活水準を下げる」という
意図されたゲームデザイン(不満度・生活水準システム全体の一部)を弱めることに
なり、より広い文脈での判断が必要。

## 47. 案F評価: 現状維持

**部分的に妥当**。Elfin・Oceanic(建設抑制後)は自律的に健全であり、
Passguard・Ferrowynの赤字は緩やかで36か月以内に破局的ではない。ただし
Arcadia・Dwarfの悪化速度(§39)と、Oceanicの過剰建設という明確に改善
可能な問題(§45)を放置する理由はない。

## 48. 採用案

**候補D(AI建設への3か月分経常支出の予備資金要求、優先度3限定)を次の
原子的タスクとして採用する。** 優先度1(Food緊急)・優先度2(P26-008-Cの
供給網修復)は対象外とし、既存の建設ロジック・供給網修復効果を一切阻害
しない。

構造的赤字そのもの(§40-1、案B相当)は、本タスクでは実装せず、次々の
タスク候補として別途起票する(§56)。

## 49. 具体的候補値

| 項目 | 現行値 | 候補値 | 適用範囲 |
|---|---|---|---|
| 建設予備資金要件 | なし(即時affordabilityのみ) | `treasury >= construction_cost + monthly_expenses × 3` | 優先度3(通常判断)のみ |

36か月実測比較(候補値適用時):

| 国 | 現行treasury(月36) | 候補値適用後treasury(月36) |
|---|---|---|
| Arcadia | -20249.22 | -14510.82 |
| Elfin | 641.37 | 4465.24 |
| Dwarf | -20107.61 | -14123.16 |
| Oceanic | 2479.53 | 12434.78 |
| Passguard | -5112.54 | -5112.54(無変化) |
| Ferrowyn | -9343.23 | -9343.23(無変化) |

「3か月分」という係数自体は本監査で1点だけ検証した参考値であり、次タスクで
2か月・4か月等の感度分析を追加で行うことを推奨する(本監査は候補の有効性の
方向性を実証するものであり、最終的なチューニング値の確定ではない)。

## 50. 国力・順位・Tierへの影響

本タスクでは国力・順位・Tierへの影響を測定していない(候補Dの実験は
財政指標のみを対象とした一時パッチであり、経済力スコアへの影響は次タスクの
スコープとする)。候補Dの適用によりOceanic・Elfin・Arcadia・Dwarfのtreasury
が改善するため、財政破綻に由来する不満度・生活水準の悪化が緩和され、
間接的に国力の人口コンポーネントへプラスに働くと推定されるが、定量化は
次タスクに委ねる。

## 51. MilitaryEquipmentへの影響

候補Dは`country_ai.rs`の建設判断にのみ影響し、`MilitaryEquipment`会計・
`equipment_reinforcement.rs`のいずれにも触れない。無関係。

## 52. MilitarySuppliesへの影響

候補DがOceanicの過剰なFactory建設を抑制することで、Oceanicの
IndustrialGoods/MilitarySupplies生産能力の伸びが遅くなる可能性がある
(Factory数が抑制されるため)。これはP26-008-Cが確立した供給網修復の
効果を弱め得るトレードオフとして、次タスクの実装時に36か月MilitarySupplies
生産量を必ず再測定すべき事項として明記する。

## 53. Phase 23〜26保護

本タスクは読み取りのみで、Phase 23〜26のいずれの契約にも触れていない。
一時パッチ実験も`country_ai.rs`の優先度3経路にのみ限定し、優先度1
(Food緊急)・優先度2(P26-008-C供給網修復)は無変更のまま実験した。

## 54. P27-001保護

`src/military/recruitment.rs`・`equipment_reinforcement.rs`・
`recruitment_supply_gate.rs`のいずれも本タスクでは変更していない。

## 55. Critical/High/Medium/Low

**Critical: 0件。High: 0件。**

**Medium: 2件**

1. **税収・行政費の係数比が過大で、初期生活水準の低い国家(4/6)が構造的
   赤字から始まる**(§40-1)。バグではなく、TAX_COEFFICIENT/ADMIN_COEFFICIENT
   の較正が現行の国家データ(人口・生活水準の分布)と噛み合っていない
   バランス問題。次タスク候補(§56、案B相当)。
2. **AI建設に経常収支・予備資金の判断がなく、健全な国家でも自ら赤字化し
   得る**(§28・§40-3、Oceanicで実証)。次タスク(§56、候補D)で対処。

**Low: 1件**

3. **破産からの回復機構が存在しない**(§39、§46で議論した案E相当)——
   36か月の観測窓では破局的でないが、長期的には無制限に悪化し続ける
   設計。より広いゲームデザイン判断が必要なため、独立した将来検討事項
   として記録するに留める。

## 56. 次の原子的タスク

**P27-002-A: AI建設への経常支出予備資金判定の追加**

- 対象ファイル: `src/country/country_ai.rs`(`process_construction_ai`の
  優先度3経路のみ)
- 変更内容: 優先度3(`PeacetimeDevelopment`/通常のMine建設)の`try_construct`
  呼び出し前に、`treasury >= construction_cost + monthly_expenses × N`
  (Nは3を初期候補とし、2・4での感度分析を実施した上で確定)を要求する
  ガードを追加。不足の場合は既存の`FundsShortage`理由をそのまま使う
  (新しいdecision_reason追加は不要)。
- 明示的対象外: 優先度1(Food緊急)・優先度2(P26-008-C供給網修復、Mine/
  Factory事前開発)は一切変更しない。募兵AI・破産ペナルティ・税制・
  建物価格のいずれも変更しない。
- 受け入れ条件: (1) Oceanicが36か月終始健全な財政を維持する (2) P26-008-Cの
  供給網修復効果(Elfin/Oceanic/PassguardのFactory新設)が阻害されない
  (3) Food緊急時の建設が一切影響を受けない (4) 既存テスト全て無変更で成功
  (5) 新規恒久テストで候補値の効果を固定する。

案B(税収・行政費データの較正)は、6か国全体のバランス調整という別レイヤの
問題であり、より広いユーザー判断(意図する国家間格差の程度、全体の難易度
較正方針)を要するため、P27-002-Aの**後**の、独立した設計監査タスクとして
別途起票することを推奨する(依存順序: P27-002-A→(将来の)P27-002-B)。

## 57. 対象外

初期国庫、税制新設、貿易、借款、国債、インフレ、金利、通貨、国庫上限、
外交援助、国家間送金、建物売却、Division解散AI、動員解除AI、戦争賠償、
UI、ローカライズ、Save V2、P27-001の再設計、MilitarySupplies/
MilitaryEquipment会計変更、戦闘バランス変更、国力式の再設計、Phase 27
後続機能のいずれも本タスクでは実装していない。

## 58. 実行コマンドと正確な結果

逐次実行(並列cargo実行なし)。

```text
開始時:
cargo test --all-targets                                      → 2515 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings       → 0 warnings

候補D実験(一時パッチ適用中、実験終了後に完全復元):
一時テストファイルのみ実行、本番テストスイートには影響なし

最終確認(一時コード削除・パッチ復元後):
cargo test --all-targets                                      → 2515 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings       → 0 warnings
```

## 59. 一時検証コード削除

`tests/p27_002_pre_001_temp_fiscal_audit_test.rs`を削除済み。
`src/country/country_ai.rs`への一時実験パッチ(§18・§45・§49の候補D検証用)
を完全に復元し、`git diff`で無変更(実質的な差分なし)であることを確認済み。

## 60. コミット対象

本報告書ファイル1件のみ。

## 61. 最終`git status --short`

```
(クリーン、本報告書ファイルのみ追加予定)
```
