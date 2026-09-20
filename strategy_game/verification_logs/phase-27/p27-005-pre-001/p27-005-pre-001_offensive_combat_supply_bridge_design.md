# P27-005-PRE-001: 攻撃側補給と占領成立の循環依存の調査・設計

本報告書は、P27-004によって移動停滞が解消された結果として
初めて実際の敵州戦闘で観測可能になった、
攻撃側補給と占領成立の循環依存を調査・設計するものである。
P27-004の修正を回帰として扱うものではない。

## 1. 総合判定

```text
P27-005-PRE-001 ANALYSIS COMPLETE
REPRODUCTION CONFIRMED
ROOT CAUSE CONFIRMED (2要因、うち補給側が本タスクの対象)
DESIGN DECIDED (候補1)
P27-005 IMPLEMENTATION READY
```

重要な追加発見: 循環依存(補給側)を候補1で解消しても、複数Division戦闘には
補給と独立した**第2の膠着要因**(実効戦力0の陣営が被害を一切受けない
`distribute_damage`の仕様)が存在する。P27-005の範囲は補給側に限定し、第2要因は
独立タスク(P27-006候補)として分離することを推奨する(§17・§42)。

## 2. 再現判定

REPRODUCTION CONFIRMED。実App・実System・実カレンダーで一時診断テスト
(削除済み)により日次観測した(§14〜§16)。

## 3. 根本原因判定

ROOT CAUSE CONFIRMED。「補給不足そのもの」ではなく、
**「勝利前の攻撃側が敵controller州にいる間の補給経路を表現する仕組みがない」**
ことが原因である。国家備蓄は10^6と潤沢なまま、配分の資格判定
(`connected_states.contains(division.current_state)`)だけで0になる(§8.3)。

## 4. 設計確定判定

DESIGN DECIDED。候補1(隣接する首都連結済み自国支配州からの前線補給投射)。

## 5. P27-005準備判定

READY。新規永続状態なし、Save V1維持、実装範囲は`daily_supply.rs`中心の小規模。

## 6. 開始時HEAD

`f20aec9fa17bc4d950e60265e9dd96b537e2221a`、`git status --short`は空。
開始時基準は2623 passed / 0 failed / 1 ignored、clippy 0 warnings(実測)。

## 7. 監査対象HEAD

同上(監査中コミットなし、報告書1件のみコミット)。

## 8. P27-004保護

P27-004の`start_or_maintain_movement`・冪等契約・System順序は一切変更せず、
`tests/p27_004_idempotent_movement_orders_e2e_test.rs`(6件)を含め全て緑。
P26-008-Bの更新済みテストは「P27-004後の現行挙動の正しい記録」として扱い、
恒久膠着を望ましい仕様として固定する根拠にはしない(本報告書がその反証を与える)。

## 9. 現行攻勢経路

`process_daily_frontline_plans`(`FrontlineOrders`)→`process_offensive_plan`が
`start_or_maintain_movement`で移動命令を発行 → `process_movement`(`MilitaryAction`)が
到着処理として`invasion::process_division_arrival`を呼ぶ。

## 10. 現行補給経路

`daily_supply::run_daily_military_supply_consumption`(`Economy`Set、月次財政直後)が
国家ごとに`connected_states_from_capital`(首都から自国`controller()`陸上州のみを
`neighbors`でBFS、`capital_connectivity.rs`)を計算し、`plan_country_daily_supply`が
`connected_states.contains(&division.current_state)`のDivisionだけを国家備蓄消費・
Hamilton配分の対象にする。含まれないDivisionは配分0、需要が正なら`fulfillment=0.0`
(`daily_supply.rs` 178〜224行)。

## 11. 現行戦闘経路

`process_division_arrival`: 敵controller州へ到着し、①進行中戦闘があれば攻撃側として
合流、②敵Divisionがいれば`start_battle_between`(Battleに`attacker_country`・
`attacker_division_ids`・`attacker_origins`を保存)、③敵Divisionがいなければ
**即座に`occupy_state`**。日次解決は`process_daily_battles`
(`calculate_combat_readiness_with_supply`で検証後、1対1は
`resolve_combat_day_with_supply`、複数は`resolve_combat_day_multi_with_supply`)。

## 12. 現行占領・controller変更経路

戦闘勝利時のみ、`resolve_finished_battles`が防御側全滅
(`(false, true)`)で`occupy_state(battle.state_id, attacker_country)`を呼ぶ。
無防備州は到着日に`occupy_state`される(補給不要)。

## 13. System順序

`Economy`(月次生産→日次補給配分、前日終値のcontrollerで連結判定)→`CountryAi`→…→
`FrontlineOrders`→`MilitaryAction`(募兵完了→補充→移動→到着処理→戦闘計算→
戦闘終了判定→占領)。到着日の`supply_fulfillment`は「前日の位置」で確定した値
(連結中の出発州ならば1.0)、翌日の`Economy`で初めて新位置(敵州)が評価され0になる。
戦闘開始と補給配分の間に**1日のラグ**がある。controller変更は勝利判定と同じ
`MilitaryAction`内で当日に起きる。

## 14. 1日単位の時系列(実測、Arcadia基準攻勢)

```text
d5  StateId(1)(自国国境)到着、Idle
d6〜d9 StateId(1)→StateId(3)(Elfin首都、owner=ctrl=Elfin)へ移動
d10 到着=戦闘開始(BattleId(0))、この日のfulfillment=1.000(前日値)
d11 fulfillment=0.000(StateId(3)はArcadia連結集合外)、以降0固定
```

## 15. Arcadia再現結果

P26-008-Bフィクスチャ(Arcadia manpower=100000、備蓄1,000,000)。d11以降
`supply_fulfillment=0.000`、Arcadia備蓄は999,990のまま不変(=備蓄枯渇ではない)。
攻撃側`combat_readiness=min(mp,eq,0)=0`。

## 16. 125日以上の膠着結果

130日間観測: 戦闘は終結せず、`controller(StateId(3))`は一度も変わらない。
攻撃側manpowerは99,829で固定(d40以降1日も変化なし)。**防御側も**d40頃から
ElfinのMilitarySupplies備蓄が尽き`fulfillment=0.00`となり、両陣営とも実効戦力0の
完全膠着となる(§17 命題Dの補足)。別の自然な構成(4個Division対1個、Elfin備蓄潤沢)でも
`ful0_fight_days=110/111`、`controller`変化なし(§24)。

## 17. 循環依存A〜Gの判定

- A(敵controller州の攻撃Divisionは首都非連結): **CONFIRMED**(`capital_connectivity.rs`
  `is_traversable`が`controller()==country`のみ通行可、`daily_supply.rs:180`)。
- B(非連結には備蓄十分でも支給0): **CONFIRMED**(備蓄999,990のまま`fulfillment=0.000`)。
- C(`fulfillment=0`なら`combat_readiness=0`): **CONFIRMED**
  (`combat_calc.rs` `min(manpower,equipment,supply)`、実測`readiness=Ok(0)`)。
- D(`combat_readiness=0`の攻撃側は戦闘に勝利できない): **CONFIRMED、ただし補足あり**。
  1対1では攻撃側は`atk_eff=0`で防御側に0ダメージ。複数Divisionでは
  `distribute_damage`が`total_effective<=0`の陣営に**一切被害を配分しない**
  (`combat_calc.rs:597`)ため、実効戦力0の側は攻撃されても被害を受けない
  (この規則は補給0の防御側にも対称に働く、§42 Medium-2)。
- E(controller変更には攻撃側勝利が必要): **CONFIRMED**(無防備州を除く。無防備州は即占領)。
- F(controllerが変わらなければ連結は回復しない): **CONFIRMED**。
- G(A〜Fの組合せで無期限膠着): **CONFIRMED**(条件付き: 防御側が存在し、
  攻撃側の当該州が連結集合外の場合。無防備州では成立しない)。

## 18. Equipmentとの因果分離

攻撃Divisionは`equipment=998/1000`(ほぼ満額)で、`min()`の律速は常にsupply項。
全額装備でも同一(readiness=min(1.0,1.0,0)=0)。装備補充対象外(Fighting)は二次影響。

## 19. 国家備蓄との因果分離

備蓄は1,000,000(消費も0)。枯渇が原因ではなく、連結・配分**資格**が原因。

## 20. Hamilton配分との因果分離

`connected_states`に含まれないDivisionは需要集合`eligible`に入らず、配分は直接0
(`allocated_units:0`、`fulfillment`は0.0固定)。「重み0」「配分後の拒否」「量子化」
ではない。

## 21. P26-008-Bゲートとの因果分離

攻勢ゲート(開始判定=国家備蓄・前線配送率)は通過済み(`stance=Offensive`で出撃)。
敵州到達後の前線補給は別契約。閾値を上げても`connected_states`の資格が変わらないため
解決しない。

## 22. controller/ownerの意味

連結判定は`controller()`基準(`capital_connectivity.rs`)。owner基準へ単純変更すると
「敵に占領された自国領も補給可能」「自国が占領した敵領も補給不能」となり、
P26-006の包囲・回廊遮断の意味論(占領による補給遮断)が失われるため不採用。

## 23. 無防備州結果(9.2)

敵Divisionなしなら到着日(d10)に`occupy_state`され、以後は通常連結。
現行・候補1ともに同一結果(`ctrl3_to_arcadia_day=Some(10)`)。補給0は占領を妨げない。

## 24. 複数Division結果(9.3・9.4)

4個攻撃vs1防御(現行): d10戦闘開始、`ful0_fight_days=110/111`、`controller`変化なし、
攻撃側manpower 9990付近で固定。4個攻撃vs3防御(現行): 同じく110/111日膠着。
DivisionIdは決定的(`DivisionId`昇順のHamilton)で、同一構成の独立2実行で結果一致。

## 25. 回廊遮断結果(9.5)

現行: 遮断以前から0。候補1: d20〜d45に`StateId(1)`のcontrollerを敵へ変更すると
`ful0_fight_days=25`(遮断期間そのもの)、d45再接続で`fulfillment=1.00`へ自然回復。

## 26. 包囲結果(9.6)

候補1: d20に首都`StateId(0)`を敵controllerへ変更すると、隣接する自国州が
首都非連結になるため`fulfillment=0`が維持される(`ful0_fight_days=41`、以降0のまま)。
投射資格は「隣接州が**首都連結済み**」に依存するため、包囲は無効化されない。

## 27. save/load結果(9.7)

4個vs1、d15(戦闘中)にsave→load: 現行・候補1とも、save/load無しの対照と
要約値(`first_fight_day`/`fight_days`/`ful0_fight_days`/`ctrl3_to_arcadia_day`/
最終controller)が完全一致。連結性・投射資格はSaveせず毎日再計算できる。

## 28. 候補0〜7比較

- 候補0(現状維持): 不採用。攻撃側は防御Divisionのいる州を原理的に落とせない。
- **候補1(採用)**: 隣接する自国controllerかつ首都連結済み州が1つ以上ある戦闘州の攻撃側だけに
  連結扱い。新規永続状態なし、既存Hamilton・保存則を再利用、回廊遮断・包囲を保持。
- 候補2(出発州を補給元として保持): Divisionごとの永続フィールド追加が必要
  (`Division`リテラルが変更禁止ファイルを含め多数、追加はSave/コンパイル影響大)。
  不採用。
- 候補3(携行備蓄): Division別会計・migration・UIが必要で範囲超過。将来案。
- 候補4(勝利まで攻撃側を出発州に残す): 移動・戦闘・占領モデルの大規模再設計。不採用。
- 候補5(無条件補給): 包囲・回廊遮断が無意味化。却下。
- 候補6(補給下限保証): `min()`契約の弱体化で補給モデルの欠陥を隠すだけ。却下。
- 候補7(先行controller変更): 占領と勝利の意味が逆転。却下。

## 29. 採用候補

候補1。ただし実装の粒度は「州単位のbool資格」で足りる(供給元の選択・永続化は不要)。

## 30. 候補1の正規契約

`plan_country_daily_supply`へ渡す連結集合を次で拡張する:

```text
extended_connected(country) =
    connected_states_from_capital(capital, country)
  ∪ { b.state_id | b ∈ BattleRegistry, b.status == Ongoing,
                   b.attacker_country == country,
                   state(b.state_id).controller() != country,
                   ∃ n ∈ state(b.state_id).neighbors: n ∈ connected_states_from_capital }
```

- 通常連結(controller基準首都連結)は無変更で優先され、拡張は攻撃側フォールバックのみ。
- 需要式・Hamilton・国家備蓄消費は既存関数をそのまま使用(`plan_country_daily_supply`は
  `connected_states`引数だけが拡張される)。二重支給・供給元備蓄なし。
- 補給資格は毎日、前日終値状態から再計算されるため、回廊遮断で翌日停止、再接続で自然回復。
- 勝利後にcontrollerが変わると`connected_states_from_capital`側に含まれ、通常補給へ移行。
- 防御側(自国/友軍controller州にいる側)の契約は不変。
- 国家ID特例なし、`neighbors`は`StateId`昇順に走査(bool判定のため順序は結果に影響しない)。

## 31. 攻撃側識別方法

`Battle.attacker_country`と`attacker_division_ids`が既に保存されており、追加状態は不要。
州単位判定(`b.attacker_country == country`かつ当該州にownerのDivisionがいる)で足りる。
同一州に複数国家が存在しても`attacker_country`で区別できる。無防備州進入は戦闘が
存在せず即占領されるため対象外(不要)。Retreating/Moving状態のDivisionは
`current_state`が戦闘州でなければ対象にならない。講和当日・国家消滅当日は
`war_active`/`is_active`判定で戦闘がCancelledとなり資格も消える。

## 32. 90日候補検証(一時パッチ、削除済み)

4個vs1(Elfin備蓄潤沢)、同一初期状態で比較:

```text
現行:    first_fight_day=10 fight_days=111 ful0_fight_days=110 ctrl3変化なし final_ctrl3=Elfin
候補1:   first_fight_day=10 fight_days=106 ful0_fight_days=0   ctrl3_to_arcadia_day=11 final_ctrl3=Arcadia
```

到着翌日(d11)に勝利しcontrollerが変更され、以後は通常連結。支給消費はArcadia備蓄から
(999,990→999,989...)既存会計で減少。回廊遮断・再接続・包囲は§25・§26。
save→load一致は§27。決定性: 要約行のmd5が独立3プロセスで一致。

## 33. 36か月候補検証

通常AI・全6か国・120か月の実プレイ相当条件では、36か月時点で活発な戦争は2件だが
**ongoing_battles=0、Fighting=0、occupied_states=0**であり、現行・候補1で全指標
(国庫・備蓄・Division数)が完全一致した。自然発生の攻勢が実際に敵州戦闘へ至る
ケースは今回の6か国・120か月では発生していない(戦力比110%要件等が未達のため)。

## 34. 120か月候補検証

同上(m60・m120でも`max_fight_streak=0`、`divisions_over90=0`、現行=候補1で全項目一致)。
候補1は自然プレイの既存挙動を変えない(回帰なし)ことを確認した。
「全ての攻撃が必ず成功する」状態にはならない: 候補1でも4vs3
(§42 Medium-2の第2要因が発現し)攻撃側が組織率低下で膠着する場合がある。

## 35. MilitarySupplies保存則

候補1は`plan_country_daily_supply`へ渡す集合を広げるだけで、`consumed_units=
min(eligible_demand, 備蓄)`とHamilton配分・`try_consume_military_supplies`を
そのまま使うため、`Σ配分=consumed`と備蓄減少量の一致は既存単体テストがそのまま担保する。
実測でも備蓄は日次需要分のみ減少した。

## 36. 防御側補給保護

拡張は`b.attacker_country == country`の戦闘州のみ。防御側は無変更(候補1の
影響は攻撃側のfulfillmentだけ)。

## 37. P27-001〜004保護

P27-001(部分装備)・P27-002-A/B/C(財政)・P27-004(移動)の関連E2E(§45)は全て緑。

## 38. Phase 23〜26保護

`cargo test --lib military::`(458)・`country::`(125)・`economy::`(58)・`save::`(305)、
P26-004/005/006/008-A/008-B、`land_war_combat_peace_test`を含め回帰なし。

## 39. Save V1/V2判断

**Save V1維持**。候補1は`BattleRegistry`(既存・Save済み)、`StateRegistry`、
`capital_state_id`から毎日再計算でき、新規永続フィールド不要。旧Saveは意味的にも
そのまま互換(候補1適用後の挙動は現在状態から即座に導出される)。

## 40. 決定性

要約行md5が独立3プロセスで一致。BFS・`neighbors`は既存の`StateId`昇順、資格は
bool判定でHashMap反復順・供給元選択に依存しない。複数戦闘・複数国家でも
州単位の独立判定。

## 41. 国家間分離

判定は`b.attacker_country == country`と自国controller・自国首都連結のみを参照し、
他国の備蓄・Divisionには触れない。

## 42. Critical/High/Medium/Low

- **High-1(本タスクの対象)**: 防御Divisionのいる敵州への国境越え攻撃が恒久膠着し、
  戦争・占領の中核ループが成立しない。国家非依存(プレイヤー攻撃にも該当)。
  自然な6か国120か月AI運用ではまだ戦闘へ至らないため潜在的だが、制御シナリオでは
  100%再現(4vs1: 110/111日膠着)。P27-004のfindingではなく独立したP27-005 finding。
- **Medium-2(独立、補給と無関係)**: `combat_calc::distribute_damage`が
  `total_effective<=0`の陣営に被害を配分しないため、複数Division戦闘では実効戦力0の側
  (補給0の防御側、組織率が極小に落ちた攻撃側)は被害を受けず戦闘が終結しない。
  候補1適用後も、Elfin首都陥落後の追撃(防御側補給0)や4vs3で観測された。
  最小修正案は別タスクで、0陣営にも総被害を実効戦力に依らず(例えば均等または
  DivisionId順)配分するか、1対1経路と整合させること。P27-005には含めない。
- Low: 携行備蓄・輸送容量・補給距離減衰(将来の補給モデル)。
- Critical: 0。

## 43. 次の原子的タスク

**P27-005: 攻撃側前線補給の実装(候補1)**。
- 変更: `src/military/daily_supply.rs`(戦闘州の攻撃側資格を判定する純粋関数
  `attacker_front_supply_states(country, connected, state_registry, battle_registry)`を追加し、
  `run_daily_military_supply_consumption`とBevy System(`Res<BattleRegistry>`追加)から呼ぶ)。
- 単体テスト: 資格判定(隣接連結州あり/なし・自国controller州・防御側・非Ongoing戦闘)、
  拡張集合での配分・保存則、`plan_country_daily_supply`の無変更確認。
- E2E: 新規`tests/p27_005_*`。4個攻撃vs1で有限日数勝利、回廊遮断で停止・再接続で回復、
  包囲(首都陥落)で停止維持、save/load一致、独立プロセス5回一致、6か国120か月で
  現行と自然プレイ指標が一致。
- 既存テスト更新: P26-008-Bの90日テスト(P27-004で更新済み)は候補1で最終stanceが
  変わり得るため、実測に基づき再更新する。
- 完了条件: 上記全て、`Save V1`維持、clippy 0、全テスト緑。

## 44. 対象外

携行備蓄・鉄道・港・輸送容量・補給距離減衰・UI・AI戦略変更・戦闘式変更
(Medium-2は別タスク)・owner/controllerモデル変更・移動API変更・交易・財政再調整。

## 45. 実行コマンドと正確な結果

全て逐次実行。

```text
cargo test --lib military::  → 458 passed
cargo test --lib country::   → 125 passed
cargo test --lib economy::   → 58 passed
cargo test --lib save::      → 305 passed
P26-008-B(19)/P27-004(6)/P26-006(18)/P26-005(10)/P26-004(19)/P26-008-A(9)/
P27-001(9)/P27-002-C(16)/land_war_combat_peace(4) の各E2E → 全て0 failed
cargo test --all-targets → 2623 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings → 0 warnings
```

## 46. 一時検証コード削除

`src/military/daily_supply.rs`への一時パッチ(戦闘州の攻撃側資格判定+ON/OFF用static)は
`git checkout --`で個別復元、`tests/p27_005_temp_diag.rs`は削除。復元後に
`git status --short`が空であることと、上記の全テスト結果を確認した。

## 47. ユーザーSave保護

`saves/savegame_v1.ron`は未接触。一時テストは`std::env::temp_dir()`のみ使用。

## 48. コミット対象

本報告書1件のみ。

## 49. 最終`git status --short`

コミット後にチャットで報告する(本報告書に未来のハッシュは記載しない)。
