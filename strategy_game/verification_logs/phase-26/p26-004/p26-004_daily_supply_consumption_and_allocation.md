# P26-004: Division日次補給需要・国家備蓄消費・決定的配分 実装報告書

## 1. 総合判定

**COMPLETE**

P26-004-PRE-005が確立した正規の日次需要関数(`division_daily_demand_units`/
`country_daily_demand_units`)をそのまま再利用し、国家備蓄からの実消費・不足時の
Hamilton方式(最大剰余法)による決定的な比例配分・Divisionごとの日次補給充足率確定を
実装した。新規System(`military::daily_supply::process_daily_military_supply_consumption`)を
`DailySimulationSet::Economy`(月次生産`economy::handle_monthly_economy`の直後、
`.after()`で明示固定)へ登録し、`DailySimulationSet`の宣言順により`MilitaryAction`
(募兵完了・移動・戦闘、`military::update::handle_daily_military`)より必ず先に実行される
ことを保証した。これにより「前日終値状態から需要を確定し、その後に当日の移動・戦闘・
募兵完了が起こる」という正規順序と、「月境界日は月次生産→同日の日次消費」という順序の
両方が、Bevy Systemの明示的な順序制約として成立する。

`Division`構造体自体には新規フィールドを追加せず(`tests/land_war_combat_peace_test.rs`
[変更禁止]を含む数十箇所の全フィールド列挙リテラルを壊さないため)、`MilitaryRegistry`に
`division_supply_fulfillment: HashMap<DivisionId, f64>`という補助mapを追加する設計を
採用した(既存の`ArmyRegistry::division_army_map`と同じ設計パターン)。欠落キーは
`MilitaryRegistry::supply_fulfillment_of`経由でdefault `1.0`として扱われ、Save V1
構文互換性(`SavedMilitaryRegistry::division_supply_fulfillment`に`#[serde(default)]`)
と組み合わさって、spec §8が要求するdefaultの意味論を自然に実現する。

原子性・保存則・決定性は新規単体テスト24件(`src/military/daily_supply.rs`)+新規E2E
テスト19件(`tests/p26_004_daily_supply_consumption_allocation_e2e_test.rs`)+
`src/save/dto.rs`の新規往復テスト2件で検証した。P26-004が実際に日次消費を接続した
ことで、既存のP26-002/003/004-PRE-002/004-PRE-005/DET-001/DET-001-REV-001の
E2Eテスト群のうち、「消費経路が存在しない」ことを前提としていた実App経由テスト
(合計17件)が意味的に不正確になったため、各テストの本来の主旨(生産順序決定性・
生産の存在確認・二重計上なし等)を保った上で、日次消費を実測して分離する形へ
更新した(弱体化ではなく、実際の仕様変更を反映する更新)。全テスト0 failed、
clippy 0 warnings。

## 2. 開始時HEAD

```text
8d0fd7ce01836bec227f7d66a870a2a1a36372c3
```

`git rev-parse HEAD`・`git status --short`・`git show --stat --oneline HEAD`で確認し、
想定(`feat: initialize military supply bridge stockpiles`、P26-004-PRE-005のコミット)と
一致していた。既存差分は` M saves/savegame_v1.ron`のみで、内容は一切参照していない。

## 3. 監査対象HEAD

`8d0fd7ce01836bec227f7d66a870a2a1a36372c3`(開始時HEADと同一。本タスクはこのコミットの
上に作業ツリー上で実装し、末尾に1コミットとして記録する)。

## 4. 日次処理の正規時系列

```text
DayChanged
→ (月境界日のみ) 月次生産が Economy Set 前段で確定
→ Division日次補給消費・配分確定(本タスクの新規System、Economy Set後段)
→ CountryAi Set(月次AI募兵判断のキュー投入)
→ MilitaryAction Set: 募兵完了(process_recruitment)→ manpower/equipment補充 →
  移動処理(process_movement)→ 戦闘処理(process_daily_battles/resolve_finished_battles)
```

`DailySimulationSet`(`src/app/time.rs`)の宣言順序:
`TimeUpdate → Economy → Research → Diplomacy → CountryAi → WarPreparation →
MilitaryAi → FrontlineOrders → MilitaryAction → WarResolution → UiUpdate`
(`GameTimePlugin::build`の`configure_sets(... .chain() ...)`が保証する)。

新規System登録(`src/military/mod.rs`):

```rust
daily_supply::process_daily_military_supply_consumption
    .in_set(DailySimulationSet::Economy)
    .after(crate::economy::handle_monthly_economy)
    .run_if(in_state(GameState::Playing)),
```

`economy::mod.rs`の既存chain`(handle_daily_construction, handle_monthly_economy)`
(`DailySimulationSet::Economy`)の直後に位置するため、月境界日には月次生産が確実に
先に完了してから日次消費が走る。`Economy`Set全体は`MilitaryAction`Set(募兵完了・
移動・戦闘が全て含まれる単一System`handle_daily_military`)より必ず先に実行されるため、
「前日終値状態から需要を確定してから当日の移動・戦闘・募兵完了が起こる」という順序が
成立する。この`.after()`による明示的な順序固定は、既存の`population::mod.rs`
(`handle_monthly_population_growth.before(economy::handle_monthly_economy)`)と
同じ確立済みの慣用パターンを踏襲したものであり、イベントreaderの登録順には一切依存
しない。

## 5. P26-004-PRE-005需要関数の再利用

`src/military/daily_supply.rs::plan_country_daily_supply`は以下を直接呼ぶ(複製しない):

* `crate::military::supply_initialization::division_daily_demand_units`(1個Division分の
  日次需要units、Divisionごとに呼ぶ)。
* `crate::military::supply_initialization::country_daily_demand_units`(国家合計、`D_c`の
  authoritativeな値として使い、`debug_assert_eq!`で自前の集計値と実測突き合わせる)。
* `crate::military::supply_initialization::is_living`(生存Division判定、`pub(crate)`へ
  可視性変更のみ行い、ロジックは一切変更していない)。

`rg -n "division_daily_demand_units|country_daily_demand_units" src/military/daily_supply.rs`
で、需要式そのもの(`usage_units × manpower / max_manpower`のfloor除算)を独自に
再実装していないことを確認できる。

## 6. Division需要式

```text
D_i = division_daily_demand_units(division, definitions)
    = floor(Q(definition.supply_usage × 1.0) × manpower / max_manpower)
```

(`Q`はP26-002の`military_supplies_value_to_units`。P26-004-PRE-005で確立済みの式を
そのまま使用、活動倍率は本タスクでも常に`1.0`。)

## 7. 国家総需要式

```text
D_c = Σ D_i (生存Division、CountryId一致、DivisionId昇順で合算)
```

`country_daily_demand_units`をそのまま呼んで得る。

## 8. 実消費式

```text
A_c = military_supplies_value_to_units(country.stockpile.get(MilitarySupplies))
C_c = min(D_c, A_c)
```

`C_c`は`u64`域に収まることが構造的に保証される(`A_c`由来の`u64`とのmin)。

## 9. 比例配分式

備蓄十分(`C_c == D_c`、需要0の場合を含む)なら`allocation_i = D_i`。不足時は
Hamilton方式(§10)。

## 10. 最大剰余法

```text
base_i = floor(C_c × D_i / D_c)
remainder_i = (C_c × D_i) mod D_c
R = C_c - Σ base_i
```

`R` unitsを、remainder降順→同値ならDivisionId昇順の順で1 unitずつ`base_i`へ加算する。
数学的に`R`は常に「remainderが厳密に正のDivision数」未満であることを示せる(§14参照)
ため、需要0または`remainder=0`のDivisionへ誤って余剰が配分されることはない
(`src/military/daily_supply.rs::allocate_by_largest_remainder`のdocコメント参照)。

## 11. tie-break

```rust
order.sort_by(|&a, &b| {
    items[b].remainder.cmp(&items[a].remainder)
        .then(items[a].id.0.cmp(&items[b].id.0))
});
```

remainder降順、同値なら`DivisionId`昇順。Bevy Entity番号・HashMap挿入順・query反復順は
一切参照しない。単体テスト`one_unit_short_of_two_equal_divisions_gives_the_extra_unit_to_the_lower_division_id`・
`only_one_unit_of_stockpile_goes_to_the_lowest_division_id_among_many`、E2Eテスト
`test_c1_one_unit_short_real_app_tie_break_favors_lower_division_id`で実測確認済み。

## 12. fixed-point scale

P26-002/PRE-005と同じ`MILITARY_SUPPLIES_SCALE = 1,000,000`をそのまま使用(新規定数は
追加していない)。

## 13. overflow処理

`consumed_units.checked_mul(demand)`・`base_sum.checked_add(base)`・
`consumed_units.checked_sub(base_sum)`・`item.base.checked_add(1)`のすべてでchecked
演算を使用する(spec §7.3、既存の値域で到達不能に見える箇所でも省略しない、
P26-002/PRE-005と同じ規律)。`saturating_*`はこのモジュール内で一切使用していない。

## 14. Division補給フィールド

```rust
// src/military/data.rs
pub struct MilitaryRegistry {
    ...
    pub division_supply_fulfillment: HashMap<DivisionId, f64>,
}
impl MilitaryRegistry {
    pub fn supply_fulfillment_of(&self, id: DivisionId) -> f64 {
        self.division_supply_fulfillment.get(&id).copied().unwrap_or(1.0)
    }
}
```

**設計変更の理由(重要)**: spec §8は`Division`構造体へ直接フィールドを追加することを
「推奨」していたが、実コード調査の結果、`Division`はテスト・本番コード合わせて60箇所
以上で全フィールドを列挙するリテラルとして直接構築されており、その中に
`tests/land_war_combat_peace_test.rs`(本タスクで変更が禁止されているファイル)が
含まれることを確認した。新規必須フィールドを追加するとこのファイルがコンパイル
できなくなり、保護事項(§21)と直接衝突する。既存の`ArmyRegistry::division_army_map`
(`DivisionId`をキーにした`MilitaryRegistry`の補助map)と全く同じ設計パターンを踏襲した
`MilitaryRegistry::division_supply_fulfillment`という代替設計を採用することで、
`Division`構造体を一切変更せずに済ませた。これは「既存設計に沿った同等の明確な名称」
というspec §8自身の許容範囲内の選択であると判断した。`remove_division`時にはこの
mapからも対応エントリを削除する(DivisionIdは再利用されないため実害はないが、
迷子データを残さないための衛生措置)。

## 15. 値域とdefault

* 値域: `0.0..=1.0`(Hamilton配分の数学的性質により`allocation_i <= D_i`が保証されるため、
  `fulfillment = allocated/demand`は自然に`<=1.0`。実装は`.clamp(0.0, 1.0)`で明示的に
  クランプする)。
* 需要0: `1.0`。
* 欠落キー(未処理・旧Save・存在しないDivisionId): `supply_fulfillment_of`が`1.0`を返す。
* NaN/Infinity: `allocated`/`demand`はいずれも有限なunitsのf64キャストであり、生成経路上
  NaN/Infinityにはなり得ない(除算は`demand>0`の場合のみ行われ、分母0を回避)。

## 16. Save V1互換性

`SavedMilitaryRegistry`(`src/save/dto.rs`)に`#[serde(default)] pub
division_supply_fulfillment: HashMap<DivisionId, f64>`を追加した。旧Save(フィールド
欠落)は空`HashMap`として復元され、`supply_fulfillment_of`が欠落キーを`1.0`として扱う
ため、"旧Saveのdefaultは1.0"という要件を、専用のserde defaultフィールド関数を書かずに
自然に満たす。Save形式番号(`SAVE_FORMAT_VERSION_V1 = 1`)は変更していない。
`src/save/validate.rs`に、このmapのキー(既存Divisionを参照しているか)と値(有限・
`0.0..=1.0`)を検証する新規ブロックを追加した(`DanglingReference`/`NonFiniteValue`/
`InvalidRange`、既存の`division_army_map`検証と同じパターン)。

## 17. 前日終値スナップショット

`process_daily_military_supply_consumption`は`Economy`Set(募兵完了・移動・戦闘を含む
`MilitaryAction`Setより前)に位置するため、その日の`DayChangedMessage`が発行された
時点でまだ当日の移動・戦闘・募兵完了は一切実行されておらず、Division状態は「前日終値」
のままである。実App E2Eテスト`test_f2_a_division_completed_by_recruitment_today_has_no_demand_until_tomorrow`
で、募兵完了当日はその新規Divisionの需要が計上されず(消費は既存Division分のみ)、
翌日から正規に需要へ含まれることを実測確認した。

## 18. 月境界日の生産・消費順

`tick_month_boundary_day`(実際の`advance_game_date`と同様、`DayChangedMessage`と
`MonthChangedMessage`を同一フレームで発行する専用harness)を用いたE2Eテスト
`test_f1_month_boundary_day_production_happens_before_same_day_consumption`で、
「単独では当日の需要を賄えない水準まで意図的に減らした備蓄」でも、実際の月境界日の
処理後にDivisionが完全充足(1.0)になることを実測した——これは月次生産が同日の消費より
先に確定した場合にのみ成立する結果である。

## 19. 募兵完了日の扱い

前日終値方式(System順序、§17)により、募兵完了で生成された新規Divisionは生成された
当日は需要判定の対象に含まれない(§8のExclusion経由ではなく、そもそもSystem実行時点で
まだ存在しないため)。翌日の`DayChangedMessage`から正規に需要へ含まれる。manpower0の
Division・Destroyed状態のDivisionは`is_living`フィルタにより需要から除外されるが、
`apply_country_daily_supply_plan`の第2段階で、除外されたが生存中(Destroyedでない)の
全Divisionへ明示的に`1.0`を書き込むため、「前日の不足率を理由なく持ち越さない」
(spec §8)も満たす。

## 20. 移動・戦闘との順序

移動(`process_movement`)・戦闘(`process_daily_battles`/`resolve_finished_battles`)は
いずれも`handle_daily_military`(`MilitaryAction`Set)内で、日次補給処理(`Economy`Set)
より後に実行される。移動中・戦闘中のDivisionもis_living判定(manpower>0かつ
Destroyedでない)を満たす限り通常どおり需要・配分の対象になる(status別の倍率は
本タスクで導入していない)。E2Eテスト`test_f3_moving_and_fighting_divisions_are_still_processed_for_supply`
で実測確認した。

## 21. 国家単位のvalidate→apply

`plan_country_daily_supply`(Phase 1、純粋関数、備蓄・Divisionを一切変更しない)→
`apply_country_daily_supply_plan`(Phase 2、`try_consume_military_supplies`を1回だけ
呼んだ後に`division_supply_fulfillment`へ書き込む)の二段階構成。国家処理順は
`CountryId`昇順に固定(`run_daily_military_supply_consumption`)。

## 22. 失敗時原子性

Phase 1が失敗(`DailySupplyPlanError`)した場合、その国家の備蓄・Division充足率は一切
変更されない(Phase 2自体が呼ばれない)。Phase 2が失敗(`DailySupplyApplyError`)した
場合も、`try_consume_military_supplies`の呼び出しが最初のステップであり、それが失敗
すれば`?`で即座に関数を抜けるため、`division_supply_fulfillment`への書き込みは一切
発生しない。単体テスト`apply_failure_leaves_stockpile_and_all_fulfillment_values_unchanged`・
`consume_api_failure_leaves_stockpile_and_fulfillment_unchanged`で確認した。

## 23. 国家間分離

`run_daily_military_supply_consumption`は国家ごとに独立したborrow・独立した
plan/applyを行うため、1か国の失敗は他国家の処理へ一切波及しない。単体テスト
`one_countrys_planning_failure_does_not_disturb_another_countrys_processing`、
E2Eテスト`test_d1_one_countrys_corrupt_division_does_not_disturb_others_real_app`
で確認した。

## 24. 国家保存則

```text
stockpile_after_c = stockpile_before_c - consumed_units_c
```

`try_consume_military_supplies`が正規fixed-point APIとして保証する。

## 25. 世界保存則

90日実App検証(`test_90_day_all_six_countries_real_app_verification`)で、6か国全てに
ついて

```text
final = initial + total_production - total_consumption
```

が誤差`1e-9`未満で厳密に一致することを実測確認した(§31参照)。

## 26. 配分量保存則

`Σ allocation_i = C_c`は`allocate_by_largest_remainder`の構成上自明に成立する
(base_sum + R = C_c)。単体テスト`no_allocation_ever_exceeds_its_own_demand_under_a_sweep_of_stockpile_levels`
で、需要合計の全範囲(0..=total)にわたって`Σ allocated == consumed_units`かつ
`allocated <= demand`を悉皆的に確認した。

## 27. 需要0

`total_demand_units == 0`の国家は、備蓄を変更せず(consumed=0)、全Division(存在すれば)
の充足率を`1.0`とする。単体テスト`zero_demand_country_leaves_stockpile_untouched_and_fulfillment_at_one`
で確認した。

## 28. 備蓄0

備蓄0かつ需要が正なら、配分0・充足率0.0。単体テスト
`zero_stockpile_allocates_zero_to_every_division_with_zero_fulfillment`、E2Eテスト
`test_c2_zero_stockpile_gives_zero_allocation_and_zero_fulfillment_real_app`で確認した。

## 29. 部分充足

`fifty_percent_stockpile_allocates_half_to_a_single_division`(単体)、
`test_c1_one_unit_short_real_app_tie_break_favors_lower_division_id`(E2E)で確認した。

## 30. New Game初期31.0との整合

New Gameの満編制Infantry1個師団は、P26-004-PRE-005の導出どおり日次需要1.0/日を持つ。
`test_a2_stockpile_decreases_by_exactly_the_known_daily_demand_before_first_monthly_production`
(P26-004-PRE-005ファイル側で更新)で、day2〜28(27日)にわたり31.0から1.0/日ずつ正確に
減少し、day28終了時点でちょうど4.0(=1日分の予備)が残ることを実測確認した——これは
PRE-005が31.0を「30イベント+1日予備」として導出した設計とP26-004の日次消費が
数値的に完全に整合することを示す。

## 31. 90日結果

`test_90_day_all_six_countries_real_app_verification`(月1〜3、day2〜28×3か月)の実測
(diagnostic出力):

```text
Arcadia:   initial=31.0 min=0.0 shortage_days=23 first_shortage_day=32 production=85.650362 consumption=58.0 final=58.650362
Elfin:     initial=31.0 min=0.0 shortage_days=50 first_shortage_day=32 production=0.0       consumption=31.0 final=0.0
Dwarf:     initial=31.0 min=0.0 shortage_days=23 first_shortage_day=32 production=94.117648 consumption=58.0 final=67.117648
Oceanic:   initial=31.0 min=0.0 shortage_days=50 first_shortage_day=32 production=0.0       consumption=31.0 final=0.0
Passguard: initial=31.0 min=0.0 shortage_days=50 first_shortage_day=32 production=0.0       consumption=31.0 final=0.0
Ferrowyn:  initial=31.0 min=0.0 shortage_days=23 first_shortage_day=32 production=76.827341 consumption=58.0 final=49.827341
```

全6か国について`final == initial + production - consumption`(誤差`1e-9`未満)、
`final >= 0.0`、`min_stockpile >= 0.0`を確認した。Elfin/Oceanic/Passguardは
P26-004-PRE-004/PRE-002が既に記録済みの「立ち上がりが遅い(1〜3か月では生産0)」
供給網であるため、初期31.0のブリッジが尽きた後(31日目)から備蓄0が続く——これは
本タスクの意図どおりの正常な"不足"であり、供給網自体の弱さ(P26-004-PRE-002/004の
既知の特性)に起因するもので、P26-004が原因のバグではない。New Game/Loaded Game
分岐比較(90日時点でsave→load)も一致した。

## 32. 36か月結果

### 32.1 初期Division固定(募兵なし)

`test_36_month_fixed_initial_divisions_real_app_verification`で、全6か国が
`division_count`1のまま36か月(972日)を通じてstockpileが負にならないことを確認した。

### 32.2 通常AI・募兵を含む実プレイ相当

```text
Arcadia:   min=0.0 shortage_days=133 final=11.394198   equipment=164.492 iron=1.326   coal=148.026  industrial_goods=221.226 treasury=-20233.028
Elfin:     min=0.0 shortage_days=109 final=1014.948535 equipment=369.390 iron=444.683 coal=735.919  industrial_goods=196.179 treasury=684.312
Dwarf:     min=0.0 shortage_days=23  final=569.058852  equipment=297.412 iron=1563.427 coal=1615.589 industrial_goods=467.294 treasury=-10564.558
Oceanic:   min=0.0 shortage_days=418 final=26.191306   equipment=113.141 iron=54.591  coal=43.185   industrial_goods=1623.957 treasury=2521.775
Passguard: min=0.0 shortage_days=104 final=2488.078516 equipment=665.016 iron=36.810  coal=678.621  industrial_goods=17.580  treasury=-5009.819
Ferrowyn:  min=0.0 shortage_days=23  final=171.858917  equipment=217.972 iron=887.661 coal=777.197  industrial_goods=845.257 treasury=-9325.676
```

全6か国とも`division_count`は36か月を通じて1のまま変化しなかった(初期Division固定
シナリオと数値が完全一致)。**Finding(Low、本タスクのスコープ外)**: `military::ai`の
月次AI募兵判断は、`calculate_ai_target_divisions`が現行データのpopulation/manpower
capから導く目標師団数がこの期間中ずっと1(=既存Division数)以下のままだったため、
`SkippedTargetMet`として一度もキューへ投入しなかった(実コード
`src/military/ai.rs::decide_and_recruit_for_country`で確認、`military::ai`は本タスクの
対象外のため変更していない)。したがって「通常AIで募兵による需要増加が発生する」という
spec §15の想定シナリオは、本マップ・本データでは36か月以内に発現しなかった——これは
P26-004が引き起こした挙動ではなく、既存`military::ai`の目標師団数計算式と現行マップの
人口規模の組み合わせによるものであり、診断用の無限資金は使わず実データのまま観測した
結果である。不足自体(shortage_days)は発生しており(特にOceanicが418/972日と顕著)、
供給網の弱さに起因する不足がP26-004の日次消費によって正しく可視化されることは実証
できている。全6か国とも`final >= 0.0`(負の備蓄なし)を確認した。

## 33. New Game/Loaded Game比較

`test_g4_new_game_continuation_matches_save_load_branch`で、New Gameのまま5日進めた
場合と、1日進めてからsave→loadを挟んで残り4日を進めた場合とで、備蓄・Division充足率が
完全一致することを確認した。

## 34. save→load分岐

上記§33に加え、`test_g5_month_boundary_save_load_does_not_double_fire_production_or_consumption`
で月境界を跨いだ後のsave/loadが値を変えないこと、`test_g3_double_save_load_round_trip_is_stable`
で2回連続のsave→loadが安定することを確認した。

## 35. 決定性

* 単体: `division_insertion_order_does_not_change_the_plan`・
  `country_insertion_order_does_not_change_the_outcome`・
  `repeated_independent_runs_from_identical_inputs_are_bit_for_bit_deterministic`。
* E2E: `test_e1_two_independent_app_instances_produce_identical_results`
  (独立した2つのAppインスタンスで、全6か国の最終備蓄・全Divisionの充足率が
  ビット単位で一致)。

## 36. P26-DET-001/REV-001保護

`src/economy/production.rs`・`src/country/country_ai.rs`はいずれも本タスクで
変更していない。既存の正規値定数(421.2265230000/592.5978000000/667.8538710000/
822.4577790000等)は1つも変更していない。P26-004導入前は「初期grantを差し引けば
生産分のみになる」だけで足りたが、P26-004が日次消費を接続したことで、raw stockpileは
もはや「初期grant + 生産」だけでは説明できなくなった(消費分も差し引かれるため)。
このため、両ファイルのtest_e/test_f(`p26_det_001_construction_ai_determinism_e2e_test.rs`)・
test_h/test_i(`p26_det_001_rev_001_cross_state_production_order_e2e_test.rs`)を、
「初期grant控除」に加えて「日次消費(day-tick前後の実測差分)を加え戻す」処理を追加する
形へ更新した(`advance_one_month_tracking_supply_consumption`という新規ヘルパーを追加、
既存の`advance_one_month`自体は無変更のまま他の全テストが引き続き使用する)。この
更新後、両ファイルとも独立した複数回の`cargo test`実行(本タスク中に3回以上)で
正規値と完全一致することを確認した。

## 37. Phase 23〜25保護

`cargo test --lib military::`(362 passed)・`cargo test --lib economy::`(39 passed)・
`cargo test --lib country::`(91 passed)・`cargo test --lib save::`(302 passed)を実行し、
Phase 23(manpower)・Phase 24(workforce)・Phase 25(装備会計)関連コードを一切変更して
いないことを確認した(いずれもbaseline通り、既存回帰なし)。

## 38. P26-005へ渡す契約

`MilitaryRegistry::supply_fulfillment_of(division_id: DivisionId) -> f64`を読み取り専用APIとして提供する。

```text
* 当日確定済みの補給充足率(日次補給処理後・戦闘処理前に確定、§4のSystem順序で保証)
* 値域 0.0..=1.0
* 需要0なら1.0
* Save V1で復元可能(欠落キーはdefault 1.0として自然に復元)
* P26-005は`manpower_ratio`・`equipment_ratio`・`supply_fulfillment`の3要素をmin合成する
  予定だが、本タスクではこの式を戦闘へ一切実装していない
  (`rg -n "supply_fulfillment_of" src`が本ファイル・単体/E2Eテスト以外に0件であることを
  確認済み)。
```

## 39. 対象外

戦闘力式・`war::combat::process_combat`の再活性化・`military::supply::process_supply`・
移動速度・organization・Attrition・携行在庫・首都連結性・包囲・港・海上輸送・鉄道輸送・
交易・補給建物の追加・生産量調整・鉱床/州データ・建設AI・財政バランス・国力評価・UI・
ローカライズ・Save V2・Phase 25装備会計、いずれにも触れていない。`rg -n
"process_combat|process_supply"`で、これらが引き続き未登録・未接続のままであることを
確認済み(既存の`test_i2_dead_supply_ratio_field_still_unregistered`等が引き続き合格)。

## 40. Critical/High/Medium/Low

* **Critical: 0件**
* **High: 0件**
* **Medium: 0件**
* **Low: 2件**
  1. §32.2で記載したとおり、36か月・実データのシナリオでは`military::ai`の月次募兵
     判断が一度も新規Divisionをキューへ投入しなかった(既存の目標師団数計算式と
     現行マップの人口規模の組み合わせによるもの、`military::ai`は本タスクの対象外)。
     このため「通常AIで募兵による需要増加が発生する」というspec §15の想定シナリオ
     そのものは本データで再現できなかった。不足自体(shortage_days)は発生しており、
     日次消費機能自体は正しく動作している。
  2. `MilitaryRegistry`構造体自体には`Division`のような全フィールド列挙リテラルの
     制約がないため、`division_supply_fulfillment`フィールドを構造体へ直接追加できたが、
     spec §8の文言どおり厳密に解釈すれば「Division自体にフィールドを追加する」ことを
     推奨していた。本タスクでは`tests/land_war_combat_peace_test.rs`保護を優先し、
     機能的に同等な代替設計(§14)を採用した。将来Save V2等でDivision構造体自体の
     刷新機会があれば、フィールドへの統合を検討する余地がある。

## 41. 全テスト結果

```text
開始時baseline: 2189 passed / 0 failed / 1 ignored(自ら再測定)
実装後最終:     2234 passed / 0 failed / 1 ignored(cargo test --all-targetsで実測、
                同一結果を独立して2回確認)
```

新規テスト45件(`src/military/daily_supply.rs`単体24件+`src/save/dto.rs`単体2件+
`tests/p26_004_daily_supply_consumption_allocation_e2e_test.rs`19件)が新規に合格し、
2189+45=2234と算術的に完全一致する。既存テストの純減はない(17件の既存E2Eテスト
[p26_002×1・p26_003×4・p26_003_rev_002×3・p26_004_pre_002×3・p26_004_pre_004×3・
p26_004_pre_005×3・det_001×2・det_001_rev_001×2、うち一部重複を含む]を、P26-004の
日次消費接続に伴う意味的な正確化のため更新したが、いずれも1対1のrename+assertion
更新であり、テスト数は変化していない)。

## 42. clippy結果

`cargo clippy --all-targets --all-features -- -D warnings`は実装後**0 warnings**
(型複雑度の警告1件を型エイリアス`MonthlySupplySeriesByCountry`の導入で解消済み)。

## 43. Cargo逐次実行

全てのCargoコマンドを逐次実行した(並列実行なし)。`cargo test --all-targets`の初回
実行時に一過性のコンパイラ内部エラー(`internal compiler error: no resolution for an
import`、`can't find crate for bevy`)が発生したため、単純に再実行して正常完走を確認
した(直前の`cargo check --all-targets`は同じソースで即座に成功しており、実コードの
問題ではなく既存タスクが記録した既知の一過性ビルド事象[PRE-005報告書§39と同種]と
判断した)。並行cargo実行は一切していない。

## 44. ユーザーセーブの保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージ・コミットして
いない。開始時・全検証コマンド実行後・本報告書作成時点のいずれの`git status --short`
でも` M saves/savegame_v1.ron`のみが変化せず残り続けたことを確認した(内容は一度も
参照していない)。テスト用の合成Save・合成App状態は全て一時ディレクトリ
(`std::env::temp_dir()`配下)または実App内メモリ上に作成し、実ユーザーセーブへは
一切アクセスしていない。

## 45. 変更ファイル

```text
src/military/data.rs                                                          (MilitaryRegistry.division_supply_fulfillment追加+単体テスト調整)
src/military/mod.rs                                                           (新規モジュール宣言+System登録)
src/military/daily_supply.rs                                                  (新規、計画・配分・適用関数+Bevy System+単体テスト24件)
src/military/supply_initialization.rs                                        (is_livingをpub(crate)化)
src/save/apply.rs                                                             (from_saved_parts呼び出しへ新フィールド追加)
src/save/dto.rs                                                               (SavedMilitaryRegistry.division_supply_fulfillment追加+往復テスト2件)
src/save/export.rs                                                            (SavedMilitaryRegistry構築へ新フィールド追加)
src/save/validate.rs                                                          (division_supply_fulfillment検証ブロック追加)
src/war/frontline.rs                                                          (テスト内from_saved_parts呼び出し更新)
tests/p26_004_daily_supply_consumption_allocation_e2e_test.rs                 (新規、E2Eテスト19件+90日/36か月実App検証)
tests/p26_002_military_supplies_accounting_foundation_e2e_test.rs            (日次消費接続に伴う期待値更新、1テスト)
tests/p26_003_military_supplies_production_e2e_test.rs                       (同上、4テスト)
tests/p26_003_rev_002_supply_production_units_e2e_test.rs                    (同上、3テスト)
tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs         (同上、3テスト)
tests/p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test.rs        (同上、3テスト)
tests/p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs            (同上、3テスト)
tests/p26_det_001_construction_ai_determinism_e2e_test.rs                    (同上、2テスト)
tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs           (同上、2テスト)
verification_logs/phase-26/p26-004/p26-004_daily_supply_consumption_and_allocation.md  (本報告書)
```

`saves/savegame_v1.ron`はコミット対象に含めない。

## 46. 最終`git status --short`

コミット前(本報告書作成完了直後):

```text
 M saves/savegame_v1.ron
 M src/military/data.rs
 M src/military/mod.rs
 M src/military/supply_initialization.rs
 M src/save/apply.rs
 M src/save/dto.rs
 M src/save/export.rs
 M src/save/validate.rs
 M src/war/frontline.rs
 M tests/p26_002_military_supplies_accounting_foundation_e2e_test.rs
 M tests/p26_003_military_supplies_production_e2e_test.rs
 M tests/p26_003_rev_002_supply_production_units_e2e_test.rs
 M tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs
 M tests/p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test.rs
 M tests/p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs
 M tests/p26_det_001_construction_ai_determinism_e2e_test.rs
 M tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs
?? src/military/daily_supply.rs
?? tests/p26_004_daily_supply_consumption_allocation_e2e_test.rs
?? verification_logs/phase-26/p26-004/
```

## 47. P26-005へ進める状態か

進める状態である。Division日次補給充足率は実際に日次処理から確定・保存され、
`MilitaryRegistry::supply_fulfillment_of`経由でP26-005が読み取り専用で安全に参照
できる。戦闘力・移動速度・organization・Attritionへの接続は一切行っていない
(意図どおり本タスクのスコープ外のまま)。P26-005着手時は、本タスクが確立した
`manpower_ratio`・`equipment_ratio`との`min`合成方式(spec §16)を実装する前に、
まず`supply_fulfillment_of`を戦闘計算のどの箇所へ差し込むか(P26-005自身の設計判断)
を決定する必要がある。
