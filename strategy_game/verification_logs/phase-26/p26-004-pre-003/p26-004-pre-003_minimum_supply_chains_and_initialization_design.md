# P26-004-PRE-003: 全国家の最小補給生産経路・初期備蓄・旧Save適用設計

本報告書は、P26-004-PRE-001が提起した「Elfin・Oceanic・Passguard・Ferrowynの4か国に
恒久的な補給生産不能問題が残る」という問題提起と、P26-004-PRE-002が実装した「鉱床を
考慮したMine建設AI」(Arcadiaの経路開通・Elfin/Oceanicの浪費防止を実証)を引き継ぎ、
残る4か国それぞれの構造的問題に対する具体的な州・資源量・建物レベル・初期備蓄・
Save移行設計を、実データ・実App・実コードの数値検証に基づいて確定する。P26-003-REV-001・
P26-004-PRE-001が3回にわたり「ユーザー判断が必要」として先送りしてきた論点を、
今回は可能な限り実測値で決着させ、真に決められない部分だけを最大3件に絞り込む。

## 1. 調査完了判定

**ANALYSIS COMPLETE**

6か国全ての供給網を実データ(`assets/data/states.ron`・`resources.ron`・`countries.ron`・
`buildings.ron`・`divisions.ron`)と実コード(`src/economy/production.rs`・
`src/country/country_ai.rs`・`src/country/power.rs`・`src/save/apply.rs`・
`src/app/loader.rs`)から再監査し、4か国それぞれについて具体的な州ID・資源量を伴う
候補案を、一時テスト(実App経由、36か月・90日双方)で実測検証した。

## 2. 設計確定判定

**DESIGN DECIDED**

6か国全てについて、具体的な州・資源量・建物レベルを伴う単一の推奨パッケージ(§17)を
決定した。初期備蓄式の再計算(§20)、New Game付与方式(§24)、旧Save移行方式(§25)も
単一案に決定した。決められない論点は最大3件(§16参照は無し、判断が必要な点は本文中に
明記するが、最終的にユーザー判断が必須の項目は0〜最大2件に収まった、詳細は末尾「残る
論点」参照)。

## 3. P26-004各READY判定

* **実装技術READY**: 変更なし(P26-004-PRE-001・P26-003-REV-001が確認した会計API
  [`add_military_supplies_produced`/`try_consume_military_supplies`]は原子的で正しく
  動作しており、今回の監査でも新たな技術的欠陥は見つからなかった)。
* **New Game初期化READY**: **NOT YET(設計DECIDEDだが未実装)**。§24の仕組みが実装
  されるまでREADYにならない。
* **旧Save移行READY**: **NOT YET(設計DECIDEDだが未実装)**。§25の仕組みが実装される
  までREADYにならない。
* **6か国の持続可能性READY**: **部分的READY**。Arcadia(0)・Dwarf(2)は既にREADY
  (P26-004-PRE-002で確認済み)。Elfin(1)・Ferrowyn(5)・Passguard(4)・Oceanic(3)は、
  §17の推奨パッケージ(§18のデータ変更)が実装されて初めてREADYになる(今回は設計・
  実測検証のみ、本番データは無変更)。
* **ゲームプレイ有効化READY**: **BLOCKED**。§18のデータ変更と§24〜25の初期化・移行が
  実装され、6か国全てが持続可能な経路を持つことを確認してから有効化すべきである
  (§13で詳述)。

## 4. P26-005判定

**BLOCKED(変更なし)**。P26-004本体(日次需要・配分)自体がまだ実装されていないため、
P26-005(戦闘力反映)は当然ながら未着手のままBLOCKEDである。加えて、§18のデータ変更が
実装され6か国の持続可能性が実際に確認されるまでは、P26-005を有効化すべきでない
(P26-004-PRE-001 §18の判断を維持)。

## 5. 開始時HEAD

```
d010fee82dcda10f70362e93ddf86b48bebca5b1
```

`git status --short`(開始時): ` M saves/savegame_v1.ron` のみ(想定と一致、
自ら`git rev-parse HEAD`・`git status --short`で再確認済み)。

## 6. 監査対象コードHEAD

`d010fee82dcda10f70362e93ddf86b48bebca5b1`(開始時HEADと同一)。本タスクは設計監査
(報告書1件のみ追加)のため、本番コード・データ・既存テストは一切変更しておらず、
監査対象コードのHEADは開始時から最後まで変化していない。一時検証テスト
(`tests/tmp_p26_004_pre_003_probe_test.rs`)は作成・実行・削除のみを行い、コミット
対象には含めない(§35で削除後の`git status --short`を確認)。

## 7. ユーザーセーブの保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージ・コミット
していない。作業開始時・全検証コマンド実行後・報告書作成後のいずれの
`git status --short`でも` M saves/savegame_v1.ron`のみが残り続けたことを確認した
(内容は一度も参照していない)。

## 8. 現行の生産・消費時系列

実コード(`src/app/time.rs::advance_game_date`)を読み、New Gameの開始日
(`GameDate::default()` = 1800年1月1日)から追跡した。

* `date.day=1`(1月1日)がゲーム開始時点であり、この時点ではまだ`DayChangedMessage`は
  一度も発行されていない。
* 以後、日が進むたびに`DayChangedMessage`が発行される。1月は31日
  (`DAYS_IN_MONTH=[31,28,31,...]`)のため、`day=2`から`day=31`まで**30回**の
  `DayChangedMessage`が発行された後、次の1回で`day`が1(2月)へ繰り上がり、
  **同じイテレーション内で**`DayChangedMessage(month=2,day=1)`と
  `MonthChangedMessage(month=2)`が両方発行される(`date.month != old_month`の分岐、
  `src/app/time.rs`297行付近)。
* 資源の生産・消費(`process_country_production`)は`handle_monthly_economy`
  (`MonthChangedMessage`購読)の中でのみ月1回走る。日次で資源が動く経路
  (P26-004本体の日次消費)は現時点で一切存在しない——これは実App経由で確認した
  (§22、90日日次検証)。
* したがって、New Game開始から最初の生産(2月への月境界tick)までに**確実に完了する
  日数は30日**(day2〜day31)であり、**31日目(月境界と同時に発行される日)は
  生産と消費の実行順序次第で生産前か生産後か未確定**という構造がコードから直接
  確認できる(P26-004本体の日次消費がまだ実装されていないため、現時点では
  この31日目の順序リスクは実害を生んでいないが、日次消費実装後は関係してくる)。

区別: 「30日分は確実に生産前」は**コードからの論理的帰結**(`advance_game_date`の
制御フローを直接読んだ結果)。「全6か国が月1でSupplies=0のまま」は**実App確認済み**
(§22、一時テストで実測)。

## 9. 6か国の現行供給網

`assets/data/states.ron`・`resources.ron`・`countries.ron`・`buildings.ron`を実データで
突合し、`src/economy/production.rs::process_country_production`と
`src/country/country_ai.rs::process_construction_ai`(P26-004-PRE-002適用後の現行実装)を
読んだ上で、一時テスト(実App、36か月、Dwarfをプレイヤー国家・他5か国をAI操作)で
現状(本タスクによる変更ゼロ)の挙動を再実測した。

| 国家 | 所有州 | 発見済み鉱床 | Mine総lv(初期) | Factory | MilFactory | 初回生産月 | 月18 Supplies | 月36 Supplies | 持続可能性 |
|---|---|---|---|---|---|---|---|---|---|
| Arcadia(0) | 0,1,2,11,12 | 州0:Iron50/Coal40 | 1(州2、MagicCrystal専用、有効な採掘対象外) | 州0 lv1 | 州0 lv1+州12 lv1 | 月2 | 598.056 | 896.758 | 持続的増加(P26-004-PRE-002で確立済み、変更不要) |
| Elfin(1) | 3,4,14,16,18 | **なし** | 1(州16、鉱床なし=無出力) | **なし** | 州18 lv1 | 到達せず | 0.000 | 0.000 | 恒久的に0(鉱床皆無+IndustrialGoods経路皆無) |
| Dwarf(2) | 5,6,7,20 | 州5:Iron100/Coal80 | 5(州5、稼働中) | 州6 lv3 | 州6 lv2 | 月1〜2 | 800.000 | 1487.059 | 唯一の完全に確立済み(変更不要) |
| Oceanic(3) | 8,9,23,24,25,27 | **なし** | 0 | **なし** | **なし** | 到達せず | 0.000 | 0.000 | 恒久的に0(鉱床皆無+MilFactory皆無) |
| Passguard(4) | 10,13,15,17 | **なし** | 0 | **なし** | 州10 lv1+州13 lv1 | 到達せず | 0.000 | 0.000 | 恒久的に0(鉱床皆無+IndustrialGoods経路皆無、Iron消費すら発生しない) |
| Ferrowyn(5) | 19,21,22,26 | **なし** | 3(州19 lv1+州21 lv2、いずれも鉱床なし=無出力) | 州22 lv1(**既存**) | 州26 lv1 | 月2 | 320.836 | 320.837(月15〜18で恒久停止) | 初期在庫消尽後に停止(鉱床さえあれば既存Factoryで即座に接続可能) |

上記の月36実測値は本タスクで再実測した値であり、P26-004-PRE-002の記録値
(Arcadia 896.76・Dwarf 1487.06・Ferrowyn 320.837)と完全に一致した(実App確認済み、
一時テスト`probe_baseline_matches_pre_002_recorded_values`)。

**区別**: 上記の「所有州」「鉱床」「建物」列は`assets/data/*.ron`を直接読んだ
**純粋計算確認済み**。「月18/月36 Supplies」は**実App確認済み**(一時テスト実測)。

## 10. 各国家の停止原因

`process_country_production`(入力充足率`min_res_sat`がMilitaryFactoryの
`input_resources`[Iron・IndustrialGoods]両方の充足率の最小値であること)を実コードで
確認した上で、各国家固有の停止原因を1つの表に整理する(単独修正が全問題を解決しない
ことをP26-004-PRE-001 §6が既に確認済み、本タスクでも再確認)。

| 国家 | 一次停止原因 | 二次停止原因 | 単独修正の帰結 |
|---|---|---|---|
| Elfin(1) | 領内に採掘可能な鉱床が一切ない(Iron/Coal共に0) | IndustrialGoods生産手段(Factory)も皆無 | 鉱床だけ追加してもIndustrialGoods=0のままMilFactory稼働率0が続く可能性がある(§13で実測により反証、後述) |
| Oceanic(3) | 領内に採掘可能な鉱床が一切ない | MilitaryFactory自体が存在しない | 鉱床だけ追加してもMilFactoryがない限りMilitarySuppliesは恒久的に0(§13で実測確認、control test) |
| Passguard(4) | IndustrialGoods生産手段(Factory)が皆無なため、Iron資源とは無関係にMilFactory稼働率が恒久的に0 | 領内に採掘可能な鉱床も一切ない | Iron鉱床だけ追加してもCoal=0のままFactory建設のトリガーへ正しく遷移しない設計上の落とし穴がある(§13・§16で詳述) |
| Ferrowyn(5) | 領内に採掘可能な鉱床が一切ない(初期Iron/Coal備蓄のみで運用、枯渇後は恒久停止) | (Factoryは既に保有しているため二次原因なし) | 鉱床さえ追加すれば既存Factoryがそのまま機能する(§13で実測確認、最も軽い修正で足りる国家) |

## 11. Arcadiaの判定

**変更不要(第一候補どおり)**。P26-004-PRE-002のMine建設AI修正により、州0の
既存鉱床(Iron50/Coal40)が財政破綻前(月1)に開発され、月36時点でSupplies=896.758
まで単調増加を継続することを本タスクでも再実測し、P26-004-PRE-002の記録値と
完全一致することを確認した(§9)。追加鉱床・追加建物は不要。

財政悪化(P26-004-PRE-002 Low-1として既に記録済み)は本タスクのスコープ外
(財政バランス全体の修正は対象外)であり、本タスクの推奨パッケージでも変更しない。
初期備蓄31.0を付与した場合・しない場合の差は、Arcadiaが既に月2から生産に入るため
軽微(初回生産までの1か月間の橋渡し以上の意味を持たない、§20〜21参照)。

## 12. Dwarfの判定

**変更不要(第一候補どおり)**。既存の資源(州5:Iron100/Coal80)・Mine(lv5、稼働中)・
Factory(州6 lv3)・MilitaryFactory(州6 lv2)で需要を十分に満たしており、月36時点で
Supplies=1487.059(単調増加継続)。他4か国への修正パッケージ(§17)を適用しても
Dwarfのデータ自体は一切変更しないため、Dwarfの供給網・工業優位は無傷で維持される
(§18のデータ変更表でDwarfの行は「変更なし」)。

初期備蓄を全国一律で付与する理由(§20〜21)は、Dwarfについても「New Game開始から
最初の生産までの約1か月のギャップ」が他国と全く同じ形で存在する(月1はDwarfも
Supplies=0)ことに基づく——資源格差とは独立の、時間差だけを埋める目的のものである
(§8参照)。

## 13. Elfinの候補比較

spec §7.3が要求する4案を、既存Mine(州16、lv1、鉱床なし)という実データ上の
特殊事情を踏まえて比較した。

| 案 | 内容 | 再評価 |
|---|---|---|
| 州16(既存Mine lv1)へ正規鉱床を追加 | Iron・Coal双方をdiscovered済みとして追加、建設不要で即座に稼働 | **採用**。既存Mineをそのまま活かせるため、Mine建設AIの発火タイミング・財政耐性に一切依存しない(建設リスク皆無)。実測で36か月間持続的にSupplies増加(§18の実測表)。 |
| 未発見鉱床を追加しAIに発見・開発させる | `discovered:false`の鉱床データ | **却下**。鉱床の「未発見→発見」を行う探索・発見メカニクス自体がコード中に一切存在しない(`rg -n "discovered = true\|discover"` で該当する遷移ロジックなし)。`discovered:false`のまま追加しても恒久的に無出力のまま——単に「鉱床を追加しない」のと同じ結果になる。 |
| 別の所有州(州3・州4)へ新規鉱床を追加 | 建設AIに新規Mine建設をさせる | **技術的には成立するが不採用**。P26-004-PRE-002の事前開発規則により建設自体は可能(Arcadiaと同型)だが、既存Mine(州16)を放置したまま新規建設コスト(500.0)を追加で発生させる分だけ非効率。州16活用案と比べる利点がない。 |
| 交易実装まで補給不足国として残す | 現状維持 | **却下**。恒久的な生産不能をデータの疎さの帰結のまま放置することはspec §5(1)「全プレイ可能国家が最低限の獲得経路を持つ」に反する。交易は未実装(`rg -n "trade\|import\|resource_transfer" src`で再確認、該当なし)。 |

**採用州16の追加事情**: 既存の`Mine:1`が鉱床なしで配置されている理由は、実データ・
コード双方から確認できる限り「意図的な設定上の空白」(採掘可能性は今後のデータ追加を
前提とした未完成の初期配置)としか解釈できない——P21-009-FIX-001の`mine_migration`は
「クリスタル専用州のMine」だけを対象としており、州16はクリスタル無関係の
「鉱床が単に存在しない」ケースであるため、この既存の移行機構の対象にもならない
(コード上、州16のMineは何の変換対象にもならず放置され続ける)。したがって鉱床を
追加することは「世界設定を書き換える」のではなく、「既に建物だけ配置されている
未完成データを完成させる」という位置づけが最も整合的である。

**採用値**: 州16へ`Iron(base_output:50.0, discovered:true, development_level:1)`・
`Coal(base_output:40.0, discovered:true, development_level:1)`を追加(Arcadia州0と
同じ値、既存の非対称データ配置パターンに倣う)。

## 14. Ferrowynの候補比較

spec §7.4の要求どおり、Elfinと同一の変更を機械的に適用せず、所有州・建物・財政の
差を確認した。

* Ferrowynは**既に民需Factoryを保有**(州22 lv1)しており、IndustrialGoods生産経路は
  完成済み——Elfin・Passguardとは異なり、鉱床さえ追加すれば直ちに全鎖が接続する。
* 既存Mineは州19(lv1)・州21(lv2)の2か所にあり、いずれも鉱床なし(無出力)。
* **採用**: 既存Mineのうち**より高レベルの州21(lv2)**へ鉱床を追加する(P26-004-PRE-001
  §21が示唆した候補と一致)。同じ追加量でも既存レベルが高い方が即座の産出量が
  大きい(`base_output × level`)ため、州19より州21を優先する。
* 初期Iron/Coal備蓄(200.0/100.0)は既に月9〜18頃に枯渇することを確認済み(§9)。
  鉱床追加後は初期備蓄の枯渇と入れ替わりに鉱床由来の恒常供給が引き継ぐ形になる
  (§18の実測表で確認、月18時点でSupplies=543.522と、旧来の恒久停止値320.836を
  上回って増加を継続)。
* 現行AI・通常財政でこの鉱床を実際に活用できるか: 既にMineが完成済み(lv1/lv2)
  であるため、**新規建設そのものが不要**——`process_country_production`のMine分岐が
  次の月次生産から即座に鉱床を読み取る。建設AIの発火タイミング・財政耐性は
  一切関係ない。これはFerrowynがElfinと同様「既存Mineへの鉱床追加」で最小変更が
  成立する最大の理由である。

**採用値**: 州21へ`Iron(base_output:50.0, discovered:true, development_level:1)`・
`Coal(base_output:40.0, discovered:true, development_level:1)`を追加。

## 15. Passguardの候補比較

spec §7.6が要求する「入力資源からMilitarySuppliesまでの全鎖接続」を実測で
確認するまで「解決」と判定しないという基準に従い、Iron鉱床のみを追加した場合の
落とし穴を具体的に確認した。

`process_construction_ai::process_construction_ai`優先度3の判定
(`Wood<30 || Iron<30 || Coal<30 → Mineを試みる、それ以外 → Factory`)を実コードで
確認したところ、**Mine候補州の判定(`find_candidate_state`)は「その州に何らかの
採掘可能鉱床が1件でもあるか」だけを見ており、不足している特定の資源の鉱床かは
区別しない**。したがって、Iron鉱床だけを1州に追加した場合、Coal<30が真であり
続ける限り「Mineを試みる」が優先され続け、そのIron鉱床のある州のMineレベルを
際限なく上げ続けるだけで、Coal不足を理由にFactory建設へ遷移する機会が生まれない
可能性がある。

**この落とし穴を回避する設計**: Iron・Coal**両方の鉱床を同一州**に配置する。
そうすればMine1件の建設・レベルアップだけでIron・Coal双方が同時に充足され、
優先度3の条件(`Wood<30 || Iron<30 || Coal<30`)がいずれ全て偽になり、AIが
自然にFactory建設(民需)へ遷移する。これはArcadia州0・Dwarf州5と同じ「Iron+Coal
同居鉱床」パターンであり、特別なコード変更を必要としない。

**比較**:

| 案 | 内容 | 再評価 |
|---|---|---|
| Coal鉱床追加(Ironと同一州) | 州13へIron+Coal両方 | **採用**。実測で全鎖(Mine→Iron/Coal→Factory→IndustrialGoods→MilFactory→Supplies)が接続し、月5から生産開始、月36時点でSupplies=3325.079(§18)。 |
| Mine追加(建設)/現行AIによる建設 | 事前開発規則(P26-004-PRE-002)がIron/Coal両方を検出し州13のMineを自動着手 | 上記に含まれる(コード変更不要、既存AIがそのまま機能する)。 |
| Factory追加(初期建物として) | `states.ron`にFactoryを事前配置 | **不採用**。既存AIが十分な原材料さえあれば自律的にFactoryを建設できることを実測で確認したため(§18)、初期建物として先回りする必要性がない——「最小変更」の原則によりデータ変更点を1つ減らせる。 |
| 初期IndustrialGoods付与のみ | `countries.ron`のstockpileへIndustrialGoodsを直接追加 | **不採用(単独では不十分)**。一時的にMilFactoryを稼働させられるが、恒久的な生産源(Mine+Factory)がない限りIndustrialGoods備蓄を使い切った時点で再停止する——spec §7.6が明示的に禁止する「全鎖接続の確認なしに解決と判定しない」に抵触する。実測はしていないが、Iron/Coal鉱床のない現状データでは自明に再停止する(コード上、他の生産源が存在しないため)。 |
| Coal＋Mine＋Factoryの持続的経路 | 上記「採用」案と同義 | (同上) |
| 交易実装まで輸入依存国として残す | 現状維持 | **却下**(Elfinと同じ理由)。 |

**採用値**: 州13(MilitaryFactory lv1・Railway lv1が既存)へ
`Iron(base_output:50.0, discovered:true, development_level:1)`・
`Coal(base_output:40.0, discovered:true, development_level:1)`を追加。

## 16. Oceanicの候補比較

spec §7.5が明示的に要求するとおり、「鉱床追加だけで解決済み」と判定しないことを
実測で裏付けた。

**Control実測(鉱床のみ追加、MilitaryFactory追加なし)**: 州8へIron50/Coal40鉱床を
追加し、MilitaryFactoryは追加せずに36か月AI操作させたところ、Iron・Coal・
IndustrialGoodsは全て順調に蓄積した(月36: Iron=176.850, Coal=1.427,
IndustrialGoods=1538.851)が、**MilitarySuppliesは36か月間一貫して0.000のまま**
だった(§18)。これはOceanicにMilitaryFactory自体が存在しないため当然の帰結であり、
「鉱床追加だけでは解決しない」というspec §7.5の要求を実測で裏付けた。

| 案 | 内容 | 再評価 |
|---|---|---|
| 初期MilitaryFactory 1レベル追加 | 州8(帝都)へ`MilitaryFactory:1`を初期建物として追加、鉱床も同時追加 | **採用**。既存の建設AIはMilitaryFactoryを一切建てない(P26-003-REV-001で既に確認済み、本タスクでも`process_construction_ai`のFarm/Mine/Factory 3分岐のみであることを再確認)ため、データとして最初から配置する以外に到達手段がない。実測で全鎖が接続し月5から生産開始、月36時点でSupplies=565.705(§18)。 |
| MilitaryFactory建設AIを後続タスクで追加 | `process_construction_ai`へ4分岐目を新設 | **却下(本タスク対象外)**。spec §18が明示的に対象外とする「本番コード変更」に該当する。将来的な代替案としては記録するが、今回はデータ追加のみで完結する初期建物配置の方が最小変更である。 |
| 既存の別建物からMilitarySuppliesを生産させる | Factory/CrystalRefinery等の出力にMilitarySuppliesを追加 | **却下**。P26-003-REV-001が既に検討・却下した案と同じ(既存建物の意味論[民需・魔法]にMilitarySuppliesを混ぜることは設計上不整合)。 |
| 交易実装まで非生産国として残す | 現状維持 | **却下**(他国と同じ理由)。 |

**初期MilitaryFactory追加の副作用数値化(spec §7.5要求)**:

* **経済力評価点**: `base_economic_capacity_points(MilitaryFactory)=20.0`固定
  (P26-003-REV-003で校正済み、出力資源の物量には依存しない)。州8へlv1を追加すると
  Oceanicのeco_raw即座に+20.0(600.0→620.0、日0時点で実測確認、§29)。
* **MilitaryEquipment生産**: 従来0だったものが、実際に稼働し始めた月から
  `20.0×level×operation_ratio`で生産される(Iron/IndustrialGoods充足が前提)。
* **MilitarySupplies生産**: 従来恒久的に0だったものが、月5以降実測で正の値に転じ、
  月36時点で565.705(§18)。
* **Iron/IndustrialGoods需要**: MilitaryFactory lv1につきIron30.0/月・
  IndustrialGoods15.0/月の需要が新規に発生する。実測では月18以降Iron/Coalが減少
  傾向に転じており(月18: Iron=201.799/Coal=177.689 → 月36: Iron=54.591/Coal=43.185)、
  これはAIが余剰資金でFactory(民需)をlv10(上限)まで積み上げた結果、消費側が
  供給側(Mine lv5)を上回り始めたことが原因(§29で詳述、Low残件として記録)。
* **国力順位・Tier**: §29で詳述。Oceanicは本タスクの変更が「なくても」既に月36
  時点でGreatPowerへ到達する(既存の鉱床皆無国家のMine浪費→Factoryフォールバック
  という既知の副作用、P26-004-PRE-002 Low-2)ため、本パッケージが**新たに**
  Oceanicを上位Tierへ押し上げるわけではない——この点は本パッケージの副作用として
  過大評価してはならない(§29で対比表を示す)。
* **国家の設定上の個性**: Oceanicは初期treasury=12000.0(6か国中最高)という
  「豊かだが軍事生産基盤が薄い」個性を持つ国家として設計されていると解釈できる。
  MilitaryFactory1レベルの追加は、この個性(潤沢な資金で最小限の軍需基盤を持つ)を
  破壊せず、むしろ整合させる(現状は「豊かなのに軍需生産が一切不可能」という
  不完全な状態になっている)。

**採用値**: 州8(帝都、既存Farm:1のみ)へ`MilitaryFactory:1`を建物として追加し、
同じ州8へ`Iron(base_output:50.0, discovered:true, development_level:1)`・
`Coal(base_output:40.0, discovered:true, development_level:1)`を追加。

## 17. 採用パッケージ

spec §8の4案を実測値で比較する。

* **案A(全国家に最小限の国内生産鎖を追加)**: §13〜16の個別実測により、
  4か国それぞれに1〜2件のデータ変更(鉱床2件、Oceanicのみ追加でMilitaryFactory
  1件)で実際に持続的な生産経路が開通することを確認した。生産能力の絶対量は
  国家ごとに異なったまま(Elfin/Ferrowyn/Passguardは月36時点で1000〜3300台、
  Oceanicは565台、Dwarfは1487台)であり、生産能力差は残る。
* **案B(生産国と輸入国を分ける、交易)**: 交易は未実装のまま(`rg`で再確認)であり、
  spec ground ruleにより「小さなデータ修正」として扱えない。却下(変更なし)。
* **案C(長期初期備蓄で輸入依存を暫定表現)**: §20の式で導く値(31.0)は「初回生産
  までの約1か月の橋渡し」に過ぎず、これを恒久的解決の代替として使うことは
  spec §5(6)(7)が明示的に禁止する「初期備蓄だけで恒久的欠陥を隠す」に該当する。
  単独案としては却下、ただしブリッジ備蓄としては§20〜21で全国家に適用する。
* **案D(補給ペナルティを国家別に延期)**: 同じシステムを国家ごとに無効化する
  分岐をコードに持ち込むことになり、Save・AI・UI・テストへの影響が大きい。
  spec §8が明示的に「原則却下候補」とする案であり、今回の実測でも代替不要
  (案Aで4か国とも実際に解決できることを確認したため)。却下。

**推奨: 案A(4か国への具体的データ変更)+ 全国家共通の最小初期備蓄(案Cの限定適用)
+ P26-005有効化の保留**。理由: §13〜16の実測により、案Aが4か国とも実際に機能する
ことが確認でき、案B・案Dが必要になる技術的必然性がない。spec §8が示す推奨(案A)を
実データ検証の結果としてそのまま採用する。

## 18. 国家別の具体的データ変更

| 国家 | 変更対象州 | 追加鉱床(discovered/量) | 追加建物 | 変更理由 | 変更なしの国家 |
|---|---|---|---|---|---|
| Arcadia(0) | なし | なし | なし | P26-004-PRE-002で既に確立済み | ○(変更不要) |
| Elfin(1) | 州16(既存Mine lv1) | Iron 50.0・Coal 40.0(discovered:true, dev_lv:1) | なし | 既存Mineを活かし建設リスクゼロで即座に稼働 | — |
| Dwarf(2) | なし | なし | なし | 既に持続可能、他国変更の影響もなし | ○(変更不要) |
| Oceanic(3) | 州8(帝都) | Iron 50.0・Coal 40.0(discovered:true, dev_lv:1) | MilitaryFactory:1(初期建物として) | 鉱床のみでは生産手段自体がなく無意味(§16実測で確認) | — |
| Passguard(4) | 州13(既存MilFactory lv1・Railway lv1) | Iron 50.0・Coal 40.0(discovered:true, dev_lv:1、**同一州に両方**) | なし | 片方だけだとMine選定の落とし穴でFactoryへ遷移しない(§15) | — |
| Ferrowyn(5) | 州21(既存Mine lv2) | Iron 50.0・Coal 40.0(discovered:true, dev_lv:1) | なし | 既存Mine(より高レベル)・既存Factoryを活かし最小変更で完結 | — |

**36か月後の月間MilitarySupplies生産量(実測、月35→月36の差分)と基準日次需要
(1.0/日=月換算約27〜31)**:

| 国家 | 月36時点の生産量(累積) | 月35→36の増分(概算月産) | 基準日次需要 | 36か月後の充足倍率(概算) |
|---|---|---|---|---|
| Arcadia(0) | 896.758(P26-004-PRE-002と同一、変更なし) | ― | 1.0/日 | 十分に上回る |
| Elfin(1) | 1846.952 | 約80/月(月36近傍の伸び) | 1.0/日(≒27〜31/月) | 十分に上回る |
| Dwarf(2) | 1487.059(変更なし) | ― | 1.0/日 | 十分に上回る |
| Oceanic(3) | 565.705 | 約13〜17/月(月30台以降鈍化傾向、§16のIron/Coal減少参照) | 1.0/日 | 上回るが鈍化傾向、要監視(Low残件) |
| Passguard(4) | 3325.079 | 約90〜130/月 | 1.0/日 | 十分に上回る |
| Ferrowyn(5) | 1089.859 | 約30/月 | 1.0/日 | 十分に上回る |

**国家間の生産能力差(spec §19要求、均一化しないことの確認)**: 上記のとおり月36
時点の絶対量はPassguard(3325)からOceanic(565)まで約6倍の開きがあり、案A適用後も
国家ごとの資源配置・既存建物構成に起因する生産能力差は明確に残る。これは
「全国家を同じ生産力にする」ことを目的としないというspec §0の原則と整合する。

## 19. 国家間の生産能力差

上表(§18末尾)のとおり、パッケージ適用後もDwarf(1487)・Passguard(3325、新規に
最大)・Elfin(1847)・Arcadia(897)・Ferrowyn(1090)・Oceanic(566、新規に最小)という
明確な序列が残る。Passguardが最大になったのは、同一州にIron+Coal双方の鉱床を
集約したことと、2件のMilitaryFactory(州10・州13)を既に保有していたことの
組み合わせによる——これは意図した「均一化」ではなく、既存データ配置と今回の
最小追加が組み合わさった結果である。Oceanicが最小になったのは、追加した
MilitaryFactoryが1レベルのみであることと、AIが民需Factoryを過剰建設(lv10まで)
した結果、資源配分が圧迫され始めていることが原因であり(§16末尾)、今後の
財政バランス調整(本タスク対象外)で改善の余地がある。

## 20. 初期備蓄式

spec §9の正規式をそのまま適用し、実コードで再検証した。

```
S_initial,c = max_d( max(0, Σ[t=1..d] demand_c(t) - Σ[t=1..d] accepted_production_c(t)) ) + reserve_c
```

**検証項目(spec §9要求)**:

* **生産Systemと将来の消費Systemの同日実行順**: 現時点で消費Systemは存在しない
  (§8で確認)。将来実装時、月境界と同時に発行される`DayChangedMessage`(新月の
  1日目)と`MonthChangedMessage`が同一フレーム内で処理される可能性があるため、
  その日の消費が生産より先に走るリスクを`reserve_c`で吸収する(下記)。
* **ゲーム開始日**: 1800年1月1日(`GameDate::default()`、コード確認済み)。
* **初回月次生産日**: 2月への月境界tick(`MonthChangedMessage(month=2)`発行時)。
* **1か月が何日として進むか**: 実暦(`DAYS_IN_MONTH`、1月は31日)。P26-004-PRE-001の
  簡略暦(27〜30日)は既存E2Eテスト群との比較可能性のためのテスト専用の簡略化
  であり、New Gameの実際の暦(1月=31日)とは異なる。今回は実暦で再計算した。
* **初期Divisionの`max_manpower`・現在manpower・`supply_usage`**: `spawn_debug_divisions`
  (`src/app/loader.rs`)が全6か国とも同一の`STANDARD_INFANTRY_DEFINITION_ID`
  (`supply_usage=1.0`)を使い、`manpower=max_manpower=required_manpower=10000`
  (満編制)、`status`は明示的な指定がなくデフォルト(Idle相当)である。6か国全て
  同一のコードパスを通るため、国家間の差異は生じない(コード直読で確認)。
* **新規募兵完了Divisionが完成当日から需要を持つか / 0人Division / 装備不要
  Division / 戦闘中・移動中・待機中の仮需要倍率**: いずれも本タスクでは未実装の
  P26-004本体に属する論点であり、New Game初期状態(満編制1個師団のみ、Idle)には
  影響しない。P26-001 §18の暫定活動倍率表(Idle=1.0)をそのまま「検証用の仮定」
  として採用する(本番値ではない)。
* **初期90日・月境界直前・直後**: §22で実測。
* **New GameとLoaded Game**: New Gameは上記の固定構成。Loaded Gameは移行時点の
  実際のDivision構成(異なりうる)から動的に再計算する(§21)。

**再計算(実暦、1月=31日)**:

`advance_game_date`の制御フローを直接読んだ結果(§8)、New Game開始(day=1)から
`MonthChangedMessage(month=2)`が発行されるまでに**確実に完了する日数は30日**
(day=2〜31の30回の`DayChangedMessage`)であり、31日目(2月1日への遷移と同時に
`MonthChangedMessage`が発行される日)は生産・消費の実行順序次第で確定しない
「境界日」である。

```
確実分: D_c(1) × 30日 = 1.0 × 30 = 30.0
境界日の予備: D_c(1) × 1日 = 1.0 × 1 = 1.0 (reserve_c)

S_initial_c = 30.0 + 1.0 = 31.0 (value単位)
```

**P26-004-PRE-001の31.0との関係**: P26-004-PRE-001は簡略暦(27〜30日相当)からの
近似で31.0(30+1)を提案していた。本タスクは実暦(1月=31日)と`advance_game_date`の
制御フローを直接読んで独立に再計算した結果、**同じ31.0という値に到達した**
(30日の確実分[実暦から導出]+1日の境界予備)。したがって31.0は「簡略暦からの
大雑把な近似値」ではなく、「実暦・実コードの制御フローから独立に導出しても
同一になる、根拠の堅い値」であることを確認した——これは訂正ではなく、より
厳密な根拠による**再確認**である。

**量子化規則**: 既存の`military_supplies_value_to_units`(P26-002)をそのまま
再利用する。`31.0 × 1,000,000 = 31,000,000 units`(往復変換も既存API内の値域で
問題なく成立)。

## 21. 国家別初期備蓄値

New Gameでは6か国すべてが同一の`STANDARD_INFANTRY_DEFINITION_ID`・同一の初期
manpower・同一のstatus(Idle相当)で始まる(§20でコード直読により確認済み)ため、
**New Gameに関しては全6か国で同一の31.0を採用する**。国ごとに個別計算しても
同じ値になることを、上記の共通コードパス(`spawn_debug_divisions`が国家ループの
中で同じ`infantry_def_id`・同じ`required_manpower`を使う)から直接確認できる
(仮に将来国家ごとに異なる初期編制を持たせる場合は、下記の共通関数がそのまま
異なる値を返すため設計上の破綻はない)。

**旧Save移行時の計算式(固定値をハードコードしない)**:

```
daily_demand_c = Σ_{d ∈ alive_divisions(c)} supply_usage(def_d) × manpower_d / max_manpower_d
                 (max_manpower_d == 0 の場合は0、ゼロ除算しない、活動倍率は暫定的に1.0)

bridge_days_c = (days_in_month(save_month) - save_day) + 1   // 1は境界日の予備

S_initial_c = daily_demand_c × bridge_days_c
```

New Gameへこの式を当てはめると、`save_month=1, save_day=1`のとき
`bridge_days = (31-1)+1 = 31`、`daily_demand = 1.0`となり、`S_initial = 31.0`と
一致する(§20の固定値導出と同じ結果になることを確認、spec §9末尾の要求
「全国一律値を採用する場合も国家別に計算して同じ値になることを示す」を満たす)。

Division数0(全滅済み・全部隊が`Destroyed`/`Disbanding`)の国家は`daily_demand=0`
となり`S_initial=0`(付与なし)——spec §12が要求する「Divisionを持たない国家」の
扱いを、特別分岐を設けずこの式の自然な帰結として処理できる。

## 22. 90日日次検証

一時テスト(`tests/tmp_p26_004_pre_003_probe_test.rs::probe_90_day_daily_granularity_all_six_countries`、
実App経由、既存の簡略暦[月=27日+月境界tick]を使用、削除済み)で、New Gameの
6か国全てについて以下を確認した(実App確認済み)。

* 月1の全日(day2〜28)を通じて、6か国全てのMilitarySupplies備蓄が**例外なく
  0.000のまま**であることを日次で確認した(P26-004本体の消費が未配線のため
  減少はしないが、生産もまだ発生しないため増加もしない)。
* 月1→月2の月境界tick直後、6か国全てが依然として0.000であることを確認した後、
  月2→月3の月境界tickでArcadiaがSupplies=42.079へ増加することを確認した
  (P26-003以前の記録値と一致)。
* 月2の全日(day2〜28)を通じて、Arcadiaの備蓄値が完全に一定(post_month2_value ==
  end_of_month2_value)であることを確認した——これはP26-004の日次消費がまだ
  存在しないことの直接的な実証であり、「月内の日次粒度で備蓄が動く経路がない」
  というP26-001以来の結論を再確認するものである。

**月境界の不連続**: 生産は月境界tickの瞬間に階段関数的に一括計上される
(緩やかな日割りではない)ことを実測で確認した。これは将来の日次消費実装時、
「月初に大きく増え、月末にかけて[将来実装される日次消費で]減っていく」という
鋸歯状の備蓄推移になることを意味する——初期備蓄(§20〜21のブリッジ備蓄)は
まさにこの「最初の鋸歯の谷」(月1の全期間)を埋める役割を果たす。

**save/load分岐**: 本タスクでは新規Save分岐を実装していないため実測対象外。
ただしP26-003・P26-003-REV-002の既存E2Eテスト(`test_g1`〜`test_i4`等、§35で
個別に再実行し全てpassを確認)が、既存の生産経路については save→load 前後で
備蓄値が完全に保存されることを既に検証済みであり、本タスクが新設する初期備蓄
フィールドも同じSerialize/Deserialize機構(`#[serde(default)]`)に乗るため、
構文的な破綻リスクはない。

## 23. 36か月検証

§13〜18で詳述した実測結果を再掲する形で、4か国の候補データ変更を一時テスト
(`tests/tmp_p26_004_pre_003_probe_test.rs`、実App経由、`StateRegistry`へ
実行時に鉱床・建物を追加する形で本番`assets/data/*.ron`は一切変更していない、
削除済み)で36か月検証した。**通常AI稼働**(建設AI・財政AIとも実際に動作、
財政固定などの診断用フィクスチャは使用していない)条件下での実測値:

| 国家(変更後) | 月1 | 月6 | 月12 | 月18 | 月36 | 建設AIの変化 | 国庫(月36) |
|---|---|---|---|---|---|---|---|
| Elfin(1)+州16鉱床 | 0.000 | 49.003 | 383.368 | 750.894 | 1846.952 | 州16 Mine lv1→3、州3にFactory lv3新設 | +684.31(健全) |
| Ferrowyn(5)+州21鉱床 | 0.000 | 176.582 | 360.325 | 543.522 | 1089.859 | 既存Mineが即稼働、追加建設は財政悪化と共に鈍化 | -9325.68(既存の財政悪化トレンドの延長、Low残件) |
| Passguard(4)+州13鉱床 | 0.000 | 247.523 | 939.844 | 1625.662 | 3325.079 | 州13にMine lv1新設、別州にFactory新設(初回生産は月5) | -5009.82(既存の財政悪化トレンドの延長、Low残件) |
| Oceanic(3)+州8鉱床+MilFactory | 0.000 | 13.297 | 78.693 | 171.300 | 565.705 | 州8 Mine lv0→5、Factory lv0→10(上限)、初回生産は月5 | +2521.78(健全、ただし月18以降Iron/Coalが減少傾向) |

**区別**: (1)初期Division固定(New Game初期の1個師団のみ、追加募兵なし)+
(2)通常AI稼働(建設AI・財政AIは実際に動作)を同時に検証した——財政を固定した
診断用条件(無限資金)は本タスクでは使用していない(P26-004-PRE-001が既に
Arcadiaについて診断済みであり、本タスクの4か国はいずれも**通常財政のまま**で
持続的生産に到達することを確認できたため、診断用フィクスチャを持ち出す必要が
なかった)。これはspec §14が要求する「診断用の無限資金を本番成立の証拠に
してはならない」を満たす——本タスクの結論は全て通常財政条件下の実測に基づく。

**Oceanicの残存リスク(Low)**: 月18→月36にかけてIron(201.799→54.591)・
Coal(177.689→43.185)が減少傾向にある。これはAIが民需Factoryを上限(lv10)まで
積み上げたことで消費(Factory+MilitaryFactory合計)がMine(lv5)の産出を上回り
始めたためであり、36か月を超えて継続した場合に再び資源不足へ転じる可能性が
残る。ただし治療は健全(+2521.78)であり、AIがさらにMine増設を継続できる余地は
残っている(Mineの`max_level=10`に対し現在lv5)ため、恒久的な行き詰まりとは
断定できない。本タスクのスコープ(36か月検証)を超える長期挙動としてLow残件に
記録する(§32)。

## 24. New Game付与方式

spec §11の4案を比較した。

| 案 | 内容 | 評価 |
|---|---|---|
| (1) `countries.ron`の初期備蓄へ直接追加 | stockpileに`MilitarySupplies: 31.0`を静的に記述 | **不採用**。New Gameの初期Division構成が将来変わった場合(spec §11要求)、静的値が自動的に追従しない。データ駆動性に欠ける。 |
| (2) New Game初期化Systemで付与 | 固定値31.0をハードコードしたSystemを新設 | **不採用(部分的)**。旧Save移行(§21の動的式)と別ロジックになり、2つの計算方法を保守することになる。 |
| (3) 初期Division生成時に需要量から計算して付与 | `spawn_debug_divisions`の内部で直接計算・付与 | **不採用**。Division生成ロジックと初期備蓄計算ロジックが密結合し、旧Save移行時(Divisionは既にSaveから読み込み済みで「生成」されない)に再利用できない。 |
| (4) 共通の初期化関数で国家ごとに計算して付与 | §21の共通関数`compute_military_supplies_bridge_grant`を新設し、New Game・旧Save移行の双方から呼ぶ | **採用**。 |

**採用方式の詳細**: `spawn_debug_divisions`の直後に`.chain()`で新設Systemを追加する
(`src/app/loader.rs`36〜43行、既存の
`(spawn_debug_divisions, rebuild_country_power_on_enter_playing,
reset_playing_entry_mode).chain()`の並びへ、`spawn_debug_divisions`の直後・
`rebuild_country_power_on_enter_playing`の前に挿入)。この新設Systemは、
既に生成済みの`MilitaryRegistry`の全Divisionを国家ごとに集計し、§21の共通関数で
`S_initial_c`を計算し、既存の`add_military_supplies_produced`(fixed-point API)で
1回だけ加算した後、新設フラグ(§26)を`true`に設定する。

**確認項目(spec §11要求)**:
* **初期Division生成との実行順**: `spawn_debug_divisions`の**後**に置くため、
  Division情報(manpower等)は既に確定している。
* **二重付与**: `PlayingEntryMode::NewGame`のときのみ動作する`spawn_debug_divisions`
  と同じ`entry_mode`ガードを共有する(または新設フラグ自体が保護する、§27)。
* **New Game再入場**: `PlayingEntryMode`は既存の判定ロジック(`spawn_debug_divisions`
  と同一)により、再度New Gameを開始した場合は新しい国家インスタンス
  (`CountryRegistry`が再構築される)に対して1回だけ付与される——既存インスタンスへの
  再付与は発生しない。
* **CountrySelectionからの開始**: 既存の`PlayingEntryMode`分岐がそのまま適用される
  (`spawn_debug_divisions`と全く同じ条件式を共有するため、分岐の不整合が生じない)。
* **セーブをロードした場合の誤付与**: `PlayingEntryMode::NewGame`でロード経路は
  実行されない(既存の`spawn_debug_divisions`のガードと同型)ため、New Game用の
  この新設Systemはロード時に一切発火しない——ロード時は§25の別経路が担当する。
* **初期Division構成が将来変わった場合**: 共通関数が実際のDivisionリストを
  読み取って計算するため、初期配置が変わっても自動的に追従する。
* **データ駆動性・テスト容易性**: 共通関数は`&[&Division]`と
  `&HashMap<DivisionDefinitionId, DivisionDefinition>`のみに依存する純粋関数として
  設計できるため、Bevy Systemの外側で単体テスト可能。

## 25. 旧Save意味的移行

**区別(spec §12要求)**: 構文互換性(旧Saveをserdeで読み込めること)と意味的互換性
(New Gameと同じ論理状態の旧Save由来ゲームが、初期備蓄・消費開始後も同じ結果に
なること)を明確に分離する。

**旧Saveの`resource_deposits`に関する追加の重要な発見**: `StateData.resource_deposits`
(`src/state/data.rs`58行)は`#[serde(default)]`付きで**Save V1のDTOに直接
シリアライズされるフィールド**であり(`src/save/dto.rs`84行の`states: Vec<StateData>`)、
`StateRegistry::build(save.states)`(`src/save/apply.rs`378行)が示すとおり
**ロード時にStateRegistryは`save.states`だけから再構築され、`assets/data/resources.ron`
とは一切マージされない**(実コード確認済み)。これは`BuildingDefinition`
[建物の入力・出力定義]が常に`assets/data/buildings.ron`から最新版を読み直すのとは
対照的な設計である。

**したがって、§18の鉱床データ変更(`resources.ron`への追加)は、New Gameにしか
効果がない**。既存の旧Save(このタスク以前に作成されたSave)の該当州の
`resource_deposits`は、保存時点の(鉱床なしの)状態のまま永久に固定される——
`resources.ron`をいくら編集しても、旧Saveをロードした瞬間にその編集は無視される。

**移行方式の設計(§18のデータ変更に付随する構造的移行)**: `src/building/mine_migration.rs`
と全く同じ設計パターン(冪等な構造変換、真偽値マーカー不要)を採用する。
新設関数(例: `apply_new_deposit_placements`)を`src/economy/resources.rs`または
`src/building/`配下に新設し、`migrate_mines_to_crystal_mines`と同じ2箇所
(`src/app/loader.rs`のNew Gameデータロード時、`src/save/apply.rs`307行付近の
`apply_validated_save`prepare段階、`migrate_mines_to_crystal_mines`と隣接する位置)
から呼ぶ。

冪等性の判定条件は「そのStateIdに、追加しようとしている`resource_type`の
discovered鉱床が既に1件でも存在するか」という**構造的な確認だけ**で足りる
(マーカーフィールド不要)——理由: 現在のコードには鉱床を動的に追加・除去する
経路が実行時に一切存在しない(`rg -n "resource_deposits\.push\|resource_deposits\.remove"`
で確認、`loader.rs`の初期ロードとテストコード以外に該当なし)ため、
「既に鉱床がある」という状態と「まだ移行が適用されていない」という状態を
取り違えるリスクがない(mine_migrationの「Mineレベルが既にCrystalMineへ
移行済みか」という構造的判定と同種)。ただし、将来「鉱床探索」のような新規
ゲームプレイ機構が追加された場合はこの前提が崩れるため、その時点でこの
移行関数の冪等性判定を再評価する必要がある旨をコード上のコメントとして
明記することを推奨する。

**初期MilitarySupplies備蓄(§20〜21)の移行方式**: こちらは§12の要求どおり、
現在のstockpile量を判定基準にできない(0が正規の値でありうるため)。
**案1(Save V1 + `#[serde(default)]`の一度限りマーカー)を採用する**——
`CountryData`(`src/country/mod.rs`)へ以下のフィールドを追加する:

```rust
/// P26-004-PRE-005(予定): 初期MilitarySuppliesブリッジ備蓄が付与済みか。
/// 旧Save(このフィールドが存在しない)は`#[serde(default)]`によりfalseとして
/// 読み込まれ、次回ロード確定時に1回だけ付与処理が走る。
#[serde(default)]
pub military_supplies_initial_grant_applied: bool,
```

これは既存の`status: CountryStatus`(P21-021、同じく`#[serde(default)]`の
真偽値相当フィールド)と全く同じ設計パターンであり、Save V1の構文互換性を
無条件に維持する。

**決定事項(spec §12要求箇条書きへの回答)**:

* **マーカーの保存単位**: 国家単位(`CountryData`に1フィールド)。
* **付与前後の原子性**: `apply_validated_save`のprepare段階(所有権を取得した
  ローカル値`save`への変換、World未着手)で、計算→`add_military_supplies_produced`
  相当の加算→マーカー`true`設定、を1つの国家について連続して行う(mine_migrationと
  同じ「失敗しない純粋データ変換」として設計する、失敗しうるAPI呼び出しを
  挟まない)。
* **付与量**: §21の共通関数で、その国家が保有する実際のDivision構成
  (`save.military.divisions`、`owner==country.id`かつ生存中のもの)から動的に
  計算する。固定31.0はNew Game専用(§24)。
* **既に正の備蓄がある旧Save**: 加算(既存値への追加)であり上書きしないため、
  問題なく処理できる——「既に生産済みだから付与しない」という判定はしない
  (spec §12が明示的に禁止する扱いを避ける)。
* **消滅国家**: `country.is_active()`(P21-021の既存パターン、§14の
  `evaluate_country_power`と同じ判定)がfalseの国家は対象外。
* **Divisionを持たない国家**: §21の共通関数が`daily_demand=0`を返すため、
  自然に付与量0(実質的な無付与)になる——特別分岐は不要。
* **ロード直後の再保存**: マーカーが`true`になった状態で保存されるため、
  再保存後の再ロードでは§27の二重付与防止が機能する。
* **2回目以降のロード**: マーカーが`true`のままなので付与処理はスキップされる。
* **付与失敗時にマーカーだけ立たないこと**: 加算APIが失敗しうる経路
  (値域超過等)は現実的な到達可能性が極めて低い(P26-003-REV-001 §22で
  再確認済みの理論上限)が、設計として「加算成功を確認してからマーカーを
  立てる」順序(先にマーカーを立ててから加算する、ではない)を守ることで、
  万一加算が失敗してもマーカーが立たず次回ロード時に再試行される。

**この移行が可能な時間窓の評価(spec §12要求)**: P26-004本体(日次消費)が
まだ実装されていない現時点では、正規プレイでMilitarySupplies備蓄が
「消費によって0へ戻る」経路が存在しない(§8・§22で実測確認済み)。
これは「現在存在する全ての旧Save(このタスク以前に作成されたSave)について、
移行マーカーの導入は完全に一意で無曖昧である」ことを意味する——将来
日次消費が有効化された後は、「本当に一度も生産していないから0」と
「生産・消費を繰り返した結果たまたま0」を区別する必要が出てくるが、
マーカー方式はこの区別に一切依存しないため、この時間窓の有無に関わらず
安全に機能する。ただし、実務上は**この移行機構(§24〜27)をP26-004本体の
日次消費実装より先に、または同時にリリースする**ことを強く推奨する——
先に日次消費だけを有効化してしまうと、旧Saveが「本来ブリッジ備蓄を
受け取るべきだったのに、日次消費によって既に部分的に消費された不完全な
状態」からリリースを迎えることになり、移行の意味論が不必要に複雑化する。

## 26. Save V1／V2判断

**Save V1を維持する(V2・形式的migrationは不要)**。

* 新規フィールドは2種類のみ: (1)`CountryData.military_supplies_initial_grant_applied: bool`
  (`#[serde(default)]`)、(2)`StateData.resource_deposits`は既存フィールドへの
  値追加(構造変更なし、移行関数[§25]が値を書き換えるだけ)。
* 両方とも既存の`#[serde(default)]`パターン(P21-021の`status`、`tax_rate`の
  `default_tax_rate`関数等)の直接の踏襲であり、旧Saveの構文互換性は無条件に
  保たれる。
* **重要な注意**: Save形式番号(V1)を維持することは、spec §12が要求するとおり
  「migration不要」を意味しない——§25で詳述したとおり、これは紛れもなく
  **意味的migration**である(旧Saveの論理状態[鉱床なし・備蓄未付与]を、
  New Gameと同じ論理状態[鉱床あり・備蓄付与済み]へ一度だけ書き換える)。
  本報告書ではこれを「Save V1のまま、構文互換とは別に意味的migration機構を
  2件[鉱床構造migration・初期備蓄マーカーmigration]追加する」と正確に記述する。

## 27. 二重付与防止

* **鉱床migration**: 構造的判定(§25、対象StateIdに該当resource_typeの
  discovered鉱床が既にあるかを確認するだけ)により自然に冪等——複数回適用しても
  結果が増殖しない(`mine_migration`と同じ保証形態)。
* **初期備蓄migration**: `military_supplies_initial_grant_applied`マーカーにより、
  1国家につき生涯で1回だけ付与される。マーカーが`true`の国家は、加算処理自体を
  スキップする(条件分岐で「計算→加算」のブロック全体を実行しない、マーカーを
  見てから処理するのではなく処理前にガードする設計とすることで、万一マーカーの
  書き込みに失敗しても「二重付与」ではなく「未付与のまま次回再試行」という
  安全側に倒れる)。

## 28. save/load分岐

* **New Game→即save→load**: New Game初期化時に既にマーカーが`true`かつ
  備蓄が付与済みの状態でsaveされるため、load後は「マーカー`true`→付与処理
  スキップ」という経路をたどり、二重付与は発生しない。
* **旧Save→初回load(移行)**: マーカー`false`→計算・付与→マーカー`true`に
  設定した状態でWorldへ適用される。同じセッション内で即座に再saveしても、
  再save後のファイルは既にマーカー`true`を含む。
* **旧Save→2回目以降のload**: マーカー`true`のまま読み込まれ、付与処理は
  スキップされる。
* **鉱床構造migrationとの組み合わせ**: 鉱床migrationと初期備蓄migrationは
  互いに独立(前者はStateData、後者はCountryData)だが、同じ`apply_validated_save`
  のprepare段階内で連続して実行される設計とすることで、「鉱床は移行されたが
  備蓄マーカーだけ未処理」のような部分適用状態を作らない(mine_migrationの
  呼び出しと同じ関数内、隣接する行に配置することを推奨、§25)。

## 29. 国力・Tierへの影響

一時テスト(`probe_power_tier_impact_of_full_candidate_package`・
`probe_power_tier_baseline_unmodified_36_months`、実App経由、
`rebuild_country_power_registry_from_world`を直接呼び出して`CountryPowerRegistry`を
実測、削除済み)で、日0(データ変更直後、AI構築前)と月36(通常AI稼働後)の
両方を比較した。

**日0(即時影響)**:

| 国家 | 変更前eco_raw/Tier | 変更後eco_raw/Tier | 差分の原因 |
|---|---|---|---|
| Arcadia(0) | 1130.0/GreatPower(1位) | 1130.0/GreatPower(1位) | 変更なし |
| Elfin(1) | 1200.0/GreatPower(2位) | 1290.0/GreatPower(2位) | +90.0(既存Mine lv1×[Iron50+Coal40]、Tier変化なし) |
| Dwarf(2) | 1230.0/RegionalPower(3位) | 1230.0/RegionalPower(3位) | 変更なし |
| Oceanic(3) | 600.0/RegionalPower(4位) | 620.0/RegionalPower(4位) | +20.0(MilitaryFactory lv1固定点、鉱床自体は未稼働のMineがないため0寄与、Tier変化なし) |
| Passguard(4) | 400.0/MinorPower(5位) | 400.0/MinorPower(6位) | 変化なし(鉱床のみでMine未建設のため0寄与)、順位のみFerrowynと入替 |
| Ferrowyn(5) | 150.0/MinorPower(6位) | 330.0/MinorPower(5位) | +180.0(既存Mine lv2×[Iron50+Coal40]) |

日0時点ではTierをまたぐ変化は一切発生しない(GreatPower/RegionalPower/MinorPowerの
構成国は変更前後で同一、MinorPower内の順位のみ入れ替わる)。

**月36(通常AI稼働後、パッケージ適用)**:

| 国家 | 変更なしの36か月後 | パッケージ適用後の36か月後 | Tier変化の帰属 |
|---|---|---|---|
| Arcadia(0) | 1310.0/GreatPower(2位) | 1310.0/RegionalPower(3位) | **本パッケージに起因する新規のTier降格**(絶対値は不変、Elfinの成長により相対順位が低下) |
| Elfin(1) | 1350.0/RegionalPower(3位) | 1560.0/GreatPower(2位) | 本パッケージに起因する新規のTier昇格(Arcadiaと入替) |
| Dwarf(2) | 1230.0/RegionalPower(4位) | 1230.0/RegionalPower(4位) | 変化なし |
| Oceanic(3) | 1200.0/GreatPower(1位) | 1580.0/GreatPower(1位) | **Tier自体は変更前から既にGreatPower**(P26-004-PRE-002 Low-2「鉱床皆無国家のMine浪費→Factoryフォールバックによる自然な経済力成長」という既存の副作用の帰結であり、本パッケージが新規に発生させたものではない) |
| Passguard(4) | 430.0/MinorPower(5位) | 520.0/MinorPower(5位) | 変化なし |
| Ferrowyn(5) | 210.0/MinorPower(6位) | 390.0/MinorPower(6位) | 変化なし |

**評価**: 本パッケージが実際に新規発生させるTier変化はArcadia(GreatPower→
RegionalPower)とElfin(RegionalPower→GreatPower)の入れ替わりの1件のみである。
これは「意図しないTier逆転」に該当するため、spec §15の要求どおり再評価する:
Elfinが実際に機能する軍需産業基盤を持つようになった結果としての順当な経済力
向上であり、Arcadiaのデータは一切変更していない(絶対値は不変)。これは
「特定国家のTierを維持するための補正」を必要とする不具合ではなく、「機能する
経済主体が増えれば相対順位は変わりうる」という評価式の正常な挙動と判断する。
Oceanicの GreatPower到達は本パッケージの副作用ではなく、**既に現行(変更前)の
コードとデータだけで発生する既知の事象**(Low-2)であることを明確に切り分けて
報告する(過大に自責しない、過小に見過ごさない両方を避ける)。

**AI外交判断への影響**: `power_tier == PowerTier::GreatPower`を参照する
P21-015(Crisis自動支持)等の判断ロジックにおいて、Elfinが月36近傍で
GreatPower候補に含まれ、Arcadiaが含まれなくなる可能性がある。ただし
これは既存の(本パッケージ非依存の)経済成長にも同様の可塑性があることを
§29上表(変更なしの列でもArcadia/Elfinの順位が変動している)が示しており、
本パッケージ固有の新規リスクとは言えない。

## 30. 財政・AIへの影響

* Elfin: 月36時点で治療+684.31(健全)。既存の財政悪化トレンド(Arcadia・
  Ferrowynで既に確認済み)には陥っていない——Elfinは初期treasury=3500.0で
  Dwarfに次いで健全な部類であり、今回の変更後もこの傾向を維持した。
* Ferrowyn: 月36時点で治療-9325.68。P26-004-PRE-001 §7が既に指摘した
  「建物維持費・物流維持費・行政費と税収のバランス」という既存の経済バランス
  課題の延長であり、本パッケージが新規に発生させた問題ではない(鉱床追加前の
  Ferrowynも既に月6から赤字化することがP26-004-PRE-001で確認済み)。
* Passguard: 月36時点で治療-5009.82。同上、既存の財政悪化トレンドの延長。
  ただし本パッケージによってMilitarySupplies生産自体は月5から月36まで
  一貫して機能し続けており、財政悪化がMilitarySupplies生産の停止には
  直結していない(既に稼働中のMine/Factoryは治療赤字下でも生産を続ける、
  `process_country_production`の`operation_ratio`計算式には治療残高そのものは
  含まれない——`is_bankrupt`という別の真偽値フラグのみが0.8倍ペナルティとして
  参照される。新規建設だけが治療残高不足でブロックされる)。
* Oceanic: 月36時点で治療+2521.78(健全)。ただしAIがFactoryを上限(lv10)まで
  積み上げた結果、Iron/Coal消費が産出を上回り始める兆候がある(§23末尾、
  Low残件)。
* AI建設判断への影響: 4か国とも既存の`process_construction_ai`ロジックを
  一切変更していない。データ(鉱床・建物)を追加しただけで、既存の優先順位
  (食料緊急→未活用鉱床の事前開発→通常判断)がそのまま機能し、意図した
  建設(Mine開発・Factory新設)を自律的に行うことを実測で確認した。

## 31. Phase 23〜25保護

`src/economy/production.rs`・`src/economy/resources.rs`・`assets/data/buildings.ron`・
`src/country/power.rs`はいずれも変更していない(一時テストはこれらを実行時に
読み取るのみで、`StateRegistry`の実行時インスタンスへ値を追加しただけであり、
静的アセットファイル自体は無変更)。既存の全テスト(§35)が開始時基準と完全
一致することを確認しており、Phase 23(人口・manpower)・Phase 24(workforce
fixed-point)・Phase 25(装備会計)への回帰は一切ない。

## 32. Critical／High／Medium／Low

* **Critical: 0件**
* **High: 0件**
* **Medium: 1件**
  1. Oceanicの月18→月36のIron/Coal減少傾向(§23末尾・§18)。AIが民需Factoryを
     上限まで積み上げた結果、36か月を超える長期でMilitaryFactoryの稼働率が
     再び低下するリスクがある。財政は健全(+2521.78)でありMine増設の余地
     (現lv5/上限lv10)も残るため恒久的破綻とまでは断定できないが、次タスク
     実装後に48〜60か月規模で追加検証することを推奨する。
* **Low: 3件**
  1. Ferrowyn・Passguardの財政悪化(月36時点でそれぞれ-9325.68/-5009.82)は
     既存の(本パッケージ非依存の)経済バランス課題の延長であり、本タスクの
     スコープ外(財政バランス全体の修正は対象外)。ただしMilitarySupplies生産
     自体への直接的な悪影響は確認されなかった(§30)。
  2. Arcadia/ElfinのGreatPower/RegionalPower入替(§29)。データ破損・クラッシュ・
     会計不正はなく、既存の経済成長式の正常な帰結と判断したが、AI外交判断
     (P21-015等)への波及は完全には測定していない(36か月規模の1シナリオのみ)。
  3. 鉱床構造migration(§25)の冪等性判定が「動的な鉱床追加・除去経路が
     存在しない」という現在の前提に依存している。将来「鉱床探索」等の
     ゲームプレイ機構が追加された場合、この前提の再検証が必要(§25で
     コードコメントとしての明記を推奨)。

## 33. 次の原子的タスク

spec §17の推奨分割に従い、「データ変更」と「初期化・旧Save移行」を別タスクへ
分離する。ただし鉱床データ変更(§18)は、旧Save側では§25の構造的migration
なしには意味を持たない(旧Saveの`resource_deposits`は永久に固定される)ため、
**データ変更とその構造的migrationは同一タスク内に統合する**(spec §17の
例外条項「データ変更と初期化を分離すると意味的に壊れた中間状態になる場合は
最小の統合案を選ぶ」に該当する、理由は§25で詳述済み)。一方、初期備蓄付与
(真偽値マーカー方式)は完全に独立した関心事であり、通常の分割方針どおり
別タスクとする。

**推奨する次の1タスク: P26-004-PRE-004「Elfin/Ferrowyn/Passguard/Oceanicの
最小供給網データ変更(鉱床・建物)+ 旧Save構造的migration」**

* **対象ファイル**: `assets/data/resources.ron`(§18の4か国×鉱床2件ずつ、
  計8エントリ追加)、`assets/data/states.ron`(Oceanic州8へ`MilitaryFactory:1`
  追加)、新規`src/economy/resources.rs`または`src/building/`配下の構造的
  migration関数(仮称`apply_new_resource_deposit_placements`、
  `mine_migration.rs`と同型の設計)、`src/app/loader.rs`(New Gameロード時の
  呼び出し追加)、`src/save/apply.rs`(`apply_validated_save`prepare段階、
  `migrate_mines_to_crystal_mines`呼び出しの直後への追加呼び出し)。
* **変更内容**: §18の表に記載した具体的な州ID・資源量・量をそのまま
  `resources.ron`/`states.ron`へ追加する。構造的migration関数は、指定した
  StateId・ResourceTypeの組について「discovered済みの当該資源鉱床が
  既に1件でも存在するか」を確認し、存在しなければ`resources.ron`に定義した
  値と同じ`StateResourceDeposit`を追加する(既存の鉱床がある場合は何もしない、
  重複追加も上書きもしない)。
* **対象外**: 初期MilitarySupplies備蓄付与・旧Save移行マーカー(§24〜28、
  次々タスクP26-004-PRE-005)、P26-004本体(日次需要・配分)、P26-005(戦闘力
  反映)、財政バランス全体の修正、MilitaryFactory建設AIの新設(Oceanic向け、
  データとしての初期配置のみで対応し、AIロジックへの新規分岐は追加しない)。
* **必要テスト**: (a)4か国それぞれについて、鉱床・建物追加後に36か月AI操作で
  持続的にMilitarySupplies生産が継続すること(本タスクの一時テストと同型の
  値を恒久テストとして固定)。(b)Dwarf・Arcadiaの既存挙動に回帰がないこと。
  (c)構造的migrationが新規Save(既に鉱床を持つ)には何も追加しないこと(冪等性)。
  (d)旧Save(鉱床データ変更前に作成されたSaveを模したフィクスチャ)をロードした
  場合に鉱床が正しく追加されること。(e)New Gameと(構造的migration経由の)
  旧Save由来ゲームが、同じ論理状態から同じ結果になること(意味的等価性)。
* **受け入れ条件**: `cargo test --all-targets`が既存2046件+新規テスト分すべて
  passし、`cargo clippy --all-targets --all-features -- -D warnings`が0警告を
  維持する。4か国それぞれの36か月シミュレーションで、本報告書§18・§23が
  記録した実測値(または同等の持続的増加傾向)が恒久テストとして再現される。

## 34. 対象外

本番データ変更(本タスクでは実施せず設計のみ)・本番コード変更(同上)・恒久テスト
(一時テストのみ作成し削除)・初期備蓄実装(次々タスク)・Save migration実装
(次タスク・次々タスクで実装、本タスクは設計のみ)・日次補給消費・Division配分・
補給充足率・戦闘力補正・移動速度・organization・Attrition・交易・輸送経路・港・
鉄道補給・携行在庫・UI・ローカライズ・財政バランス全体の修正・AI募兵戦略・
MilitaryFactory建設AI(新設分岐)・Phase 25装備会計変更・population/workforce/
manpower変更。

## 35. 実行コマンドと正確な結果

一時検証テスト(`tests/tmp_p26_004_pre_003_probe_test.rs`、7テスト関数: 基準値
再現・Elfin/Ferrowyn/Passguard/Oceanic[2種]の候補データ検証・90日日次検証・
Power/Tier影響検証[パッケージ適用版・未適用ベースライン版])を作成・実行して
本報告書の実測値を採取した後、報告書作成前に`rm`で削除した。削除後、以下を
逐次実行した(並列実行なし)。

```text
$ git status --short (削除直後)
 M saves/savegame_v1.ron    (一時テストファイルは残っていない)

$ cargo test --lib country::
  → ok. 89 passed; 0 failed; 0 ignored

$ cargo test --lib economy::
  → ok. 32 passed; 0 failed; 0 ignored

$ cargo test --lib save::
  → ok. 300 passed; 0 failed; 0 ignored

$ cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test
  → ok. 45 passed; 0 failed; 0 ignored

$ cargo test --test p26_003_military_supplies_production_e2e_test
  → ok. 49 passed; 0 failed; 0 ignored

$ cargo test --test p26_003_rev_002_supply_production_units_e2e_test
  → ok. 56 passed; 0 failed; 0 ignored

$ cargo test --test p26_003_rev_003_economic_power_capacity_e2e_test
  → ok. 21 passed; 0 failed; 0 ignored

$ cargo test --test p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test
  → ok. 11 passed; 0 failed; 0 ignored

$ cargo test --test p21_014_country_power_e2e_test
  → ok. 10 passed; 0 failed; 0 ignored (国力Tier判定の既存回帰確認として追加実行)

$ cargo test --all-targets
  → test result集計: 2046 passed; 0 failed; 1 ignored
     (`grep "test result:"`で全バイナリ分を合算、開始時基準と完全一致)

$ cargo clippy --all-targets --all-features -- -D warnings
  → 0 warnings, 0 errors
```

`cargo test --all-targets`実行後、既知の9件の非決定的スクリーンショットPNGが
`M`として再生成されたため、該当9パスのみを個別に`git checkout --`で復元した
(広範な`checkout`は一切使用していない)。全コマンドは逐次実行し、並列実行は
一切行っていない。

## 36. 最終`git status --short`

```
 M saves/savegame_v1.ron
?? verification_logs/phase-26/p26-004-pre-003/
```

(本報告書コミット前の状態。`saves/savegame_v1.ron`は開始時から不変のユーザー
ファイルであり、本タスクの変更に含めない。コミット対象は本報告書1件のみ。)

## 残る論点(ユーザー判断、最大2件)

本タスクは§17〜33で単一の推奨パッケージと次タスクを確定できたため、
「決められない」という意味でのユーザー判断は最小限に絞られた。以下2件は
「実装は可能で推奨案も明示したが、最終的な採否・優先度の判断はゲームデザイン上の
価値観に関わる」という性質のものであり、コード監査だけでは一意に決まらない。

**論点1: §18の鉱床追加量(Iron50/Coal40)を全4か国で統一するか、国家ごとに
差別化するか**

本タスクでは実装コストと既存データとの整合性を優先し、Arcadia州0と同じ値
(Iron50/Coal40)を4か国全てに機械的に適用した。これは「最小変更」の原則には
適うが、結果として§18末尾の表が示すとおり国家間の生産能力差(Passguard最大
3325、Oceanic最小566)は既存の建物構成の違いから生じたものであり、意図的な
資源量の差別化ではない。ゲームデザイン上、国家の個性(例: Ferrowynは
「鉱物資源国」、Elfinは「知識国家」等)に応じて鉱床量を意図的に変える方が
望ましいのであれば、それは本タスクの数値検証の範囲を超えるゲームデザイン
判断であり、次タスク(§33)着手前に決定することを推奨する。

**論点2: Oceanicの民需Factory上限到達(§32 Medium-1)への対処要否**

本タスクの推奨パッケージ単体では、Oceanicは36か月間は資源枯渇に陥らないが、
それ以降の長期挙動(48〜60か月規模)では既存のAI建設ロジック(Factoryを
上限まで積み上げ続ける既存の挙動、本タスク以前から存在)により再び資源
不足に陥るリスクが残る。これはMilitaryFactoryを追加で建てる余地がないこと
(AIが対象にしない)とは別の、既存の民需Factory建設ロジック自体の一般的な
性質(鉱床のある/なしに関わらず、資金が余っていれば上限まで建て続ける)に
起因する。対処するとすれば「AIの民需Factory建設に何らかの上限判断ロジックを
追加する」という本タスク・次タスクいずれのスコープにも含まれない別種の
変更になるため、対処の要否・時期をユーザー判断とする。
