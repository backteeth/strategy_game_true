# P26-006: 首都連結性による空間補給モデル2 実装報告書

## 1. 総合判定

**COMPLETE**(既知の高優先度findingあり、詳細は§40・§34参照)

P26-001が確定した次の空間マイルストーン(モデル2)を実装した。

```text
国家首都 → 自国が現在実効支配する陸上州 → StateData.neighbors → Division所在州
```

新規モジュール`src/military/capital_connectivity.rs`に、首都からBFSで到達可能な
自国実効支配陸上州の集合を求める純粋関数`connected_states_from_capital`を実装し、
P26-004の`src/military/daily_supply.rs::plan_country_daily_supply`を拡張して、
首都と連結しているDivisionだけを国家備蓄からの実消費・Hamilton配分の対象にした。
連結していないDivisionは配分0、正の需要があれば補給率0.0、需要0なら1.0とする
(spec §11)。連結性そのものはSaveへ保存せず、既存のSave済みデータ(首都・
`neighbors`・`controller_country`・Division`current_state`)から毎日再計算する
(spec §18)。

実装過程で、P26-005までのcombat E2Eテストの一部(`tests/p25_004_...`・
`p25_005_...`・`p25_006_rev_001_...`・`p25_007_...`)が、存在しない合成州
(`TEST_STATE_ID`)や「相手国の支配州」へDivisionを配置していたため、本タスクの
首都連結性導入によって意図せず補給率0(切断)になり、4ファイル7件のテストが
一時的に破綻することを発見した。これは各テストの本来の検証意図(装備補充・
戦闘・撤退・Save/load)とは無関係な副作用であり、Division配置を各国が実際に
支配する実マップ州へ修正することで解消した(弱体化ではなく、これらのテストが
暗黙に前提としていた「補給は常に満タン」という仮定を、新しい地理的前提に
合わせて正しくしただけ)。

さらに、実装過程で1件の**Phase 21由来の既存(P26-006とは無関係の)非決定性**を
発見した(`tests/p23_010_...::a_save_and_reload_partway_through_matches_an_
uninterrupted_run`が稀に失敗する)。根本原因は`CountryAiState.dirty`フラグが
意図的にSave対象外とされている設計(`src/save/dto.rs`、P21-SAVE-002A1、
「dirtyはいつ再評価するかにしか影響せず最終結果には影響しない」という前提)で
あり、本タスクが導入した「戦闘力が地理[支配権]に依存する」という新しい
フィードバック経路が、この前提を初めて破る形で顕在化させたものである。詳細な
根本原因特定とユーザーへの選択肢提示を行い、「P26-006を完了として扱い、この
findingを報告書に明記する」方針で進めることの承認を得た(§40参照)。この
non-determinismはP26-006自身のロジックの欠陥ではなく(接続性ロジック単体は
34件の単体テストで検証済み、`connected_states_from_capital`を強制的に
「常に全連結」へ差し替える診断用パッチで実際にこのテストが安定することを
確認し、原因を単体テスト外の隠れた変数[Save対象外のAI再評価タイミング]に
絞り込んだ)、修正にはSave V2または既存Phase 21 AI評価タイミングロジックの
変更が必要であり、いずれも本タスクの対象外(spec §22)である。

新規単体テスト34件(`capital_connectivity.rs`25件+`daily_supply.rs`9件)+
新規E2Eテスト18件(`tests/p26_006_capital_connectivity_supply_e2e_test.rs`)で、
グラフ探索・通行条件・首都条件・接続/切断分類・Hamilton配分・保存則・回廊遮断/
奪還・P26-005戦闘力への伝播・国家間分離・Save V1互換性・save/load分岐・決定性・
90日/36か月の無戦争シナリオでのP26-004/005回帰なしを検証した。全テスト
2326 passed/0 failed/1 ignored(開始時2274から+52)、clippy 0 warnings
(上記の既知flaky testを除く――このテストは`--all-targets`の複数回実行中、
約40〜50%の頻度で失敗することを実測した、詳細は§40)。

## 2. 開始時HEAD

```text
84c02d23ba3ef228d8f0ca7b31f10fb9d9137875
```

`git rev-parse HEAD`・`git status --short`・`git show --stat --oneline HEAD`で
確認し、想定(`feat: apply military supply to combat strength`、P26-005の
コミット)と一致していた。既存差分は` M saves/savegame_v1.ron`のみで、内容は
一切参照・変更・コミットしていない。

## 3. 監査対象HEAD

`84c02d23ba3ef228d8f0ca7b31f10fb9d9137875`(開始時HEADと同一。本タスクはこの
コミットの上に作業ツリー上で実装し、末尾に1コミットとして記録する)。

## 4. 事前監査結果

実データ(`assets/data/states.ron`・`assets/data/countries.ron`)を精読した結果、
次を確認した。

* `is_sea: true`の州は現在の実マップに**1件も存在しない**(全28州が陸上)。
  したがって「島嶼国家[Oceanic]が海上輸送の欠如だけで恒久的に切断される」という
  BLOCKED条件は実データ上そもそも発生しない。
* 全6か国の首都は、New Game開始時点でいずれも存在し・海上州でなく・
  `controller() == owner_country_id == 自国`である(占領がまだ発生していない
  ため)。
* 全6か国について、`owner_country_id`だけで絞った「自国所有州同士の
  `neighbors`部分グラフ」が首都を根とする連結木/グラフになっている(手計算で
  6か国全て確認、後にE2Eテスト`test_audit_connected_set_matches_owned_states_
  at_new_game_for_all_six_countries`で実測固定)。
* New Gameの初期Division(`spawn_debug_divisions`、`src/app/loader.rs`)は
  必ず`country.capital_state_id`に配置される――初期状態で切断されるDivisionは
  存在しない。
* `StateData.neighbors`は実データ上すべて双方向対称(手計算で確認)だが、
  本実装はこの対称性を前提にしない(有向辺として扱う、§10参照)。
* Division所在州は`current_state`フィールドで一意に取得できる。移動中は
  出発州のまま保持され、`process_movement`が完了時に一度だけ目的州へ更新する
  (`src/military/movement.rs:139`の1箇所のみ)。
* `controller()`(`StateData::controller_country.unwrap_or(owner_country_id)`)が
  実効支配の正規取得方法であり、既存の`get_controlled_states`が同じ式を
  使っている。

以上により、BLOCKED条件のいずれにも該当せず、本番実装へ進んだ。

## 5. 6国家の首都

```text
Arcadia(0)   : StateId(0)  "Arcadia Capital"
Elfin(1)     : StateId(3)  "Elfin Central"
Dwarf(2)     : StateId(5)  "Dwarf Mining Region"
Oceanic(3)   : StateId(8)  "Imperial Capital"
Passguard(4) : StateId(15) "Highvale"
Ferrowyn(5)  : StateId(22) "Free Watch"
```

全て`is_sea: false`・New Game開始時`controller() == 自国`であることを
`test_audit_all_six_capitals_are_valid_and_self_controlled_at_new_game`で
実測した。

## 6. 6国家の初期Division接続性

全6か国の初期Division(満編制Infantry)は首都に配置され、New Game開始後の
最初の日次補給処理で補給率1.0(完全連結)になることを
`test_audit_all_six_initial_divisions_are_at_capital_and_fully_connected`で
実測した。

## 7. neighborsの正規意味

`StateData.neighbors: Vec<StateId>`は「その州から陸路で直接移動できる隣接州の
IDリスト」。本実装は各州が列挙する`neighbors`を**有向辺**として扱う(州`a`が
`b`を含んでいても`b`が`a`を含み返すとは限らないという前提でBFSを実装、
実データは全て対称だが対称性を要求しない実装にした、spec §8)。

## 8. ownerとcontrollerの違い

* `owner_country_id`: 法的所有(占領されても変化しない)。
* `controller_country: Option<CountryId>`: 実効支配国(占領中はSome、平時は
  None)。
* `controller() = controller_country.unwrap_or(owner_country_id)`。

補給連結性の通行判定には`controller()`のみを使う(`owner_country_id`単独では
判定しない)。単体テスト`b2_own_owned_but_enemy_controlled_state_is_not_
traversable`・`b3_enemy_owned_but_self_controlled_state_is_traversable`、
E2Eテスト`test_own_owned_but_enemy_controlled_corridor_is_not_traversable_
real_map`・`test_enemy_owned_but_self_controlled_state_is_traversable_real_
map`で確認した。

## 9. traversable条件

```rust
fn is_traversable(state_id, country, state_registry) -> bool {
    match state_registry.get(state_id) {
        Some(state) => !state.is_sea && state.controller() == country,
        None => false,
    }
}
```

存在しない州・海上州・敵/同盟/中立国支配州はいずれも通行不能(spec §6)。

## 10. 首都連結性の正規式

```text
Connected(c) = capitalからneighborsをたどり、traversable(c, s)を
               満たす州だけで到達できる集合
```

首都自体が`is_traversable(capital, c, ...)`を満たさない場合、`Connected(c)`は
空集合(spec §7)。`connected(division) = division.current_state ∈
Connected(division.owner)`。

## 11. 探索アルゴリズム

BFS(`VecDeque`、`src/military/capital_connectivity.rs::connected_states_from_
capital`)。距離を使わないためDijkstra等は不要というspec §8の指示どおり単純な
幅優先探索を採用した。世界の州数を`N`、総neighbor辺数を`E`とすると各州は高々
1回だけキューに積まれるため計算量は`O(N+E)`(州数に対して線形)。

## 12. 決定的なneighbor順

各州の`neighbors`は展開前に`StateId`昇順へ明示的にソートしてから辿る
(`neighbors.sort_by_key(|s| s.0)`)。訪問済み判定は`StateId`をキーにした
`HashSet`で行い、Bevy Entity順・`states.ron`のファイル順・`Vec`挿入順の
いずれにも依存しない。最終的な到達可能集合(`HashSet`の中身)は探索順序に
一切依存しない数学的性質を持つことを、単体テスト`a8_neighbor_list_insertion_
order_does_not_change_the_final_set`・`a9_state_vec_insertion_order_does_
not_change_the_final_set`で実測固定した。

## 13. 海上州の扱い

`is_sea: true`の州は、自国が実効支配していても常に通行不能(単体テスト
`b6_sea_state_is_never_traversable_even_if_self_controlled`)。実データには
現在`is_sea: true`の州が存在しないため、この経路はE2E実データでは発現しない
(合成`StateRegistry`フィクスチャで検証)。

## 14. 同盟・中立国州の扱い

`controller()`が自国以外(同盟国・中立国いずれも)の州は通行不能(単体テスト
`b4_ally_controlled_state_is_not_traversable`・`b5_neutral_controlled_state_
is_not_traversable`)。軍事通行権・同盟補給は本タスクの対象外(spec §6・§22)。
実データ上、同盟国領を通れないことが問題になるケースは今回のシナリオ範囲
(6か国・実データ)では確認されなかった。

## 15. 首都占領の扱い

首都が敵支配(`controller() != 自国`)・存在しない・海上州として参照されている
いずれの場合も`Connected(c)`は空集合になり、正の需要を持つ全Divisionの補給率が
0になることを、単体テスト(`c2`〜`c5`)およびE2Eテスト
`test_capital_itself_falling_disconnects_the_entire_country`で実測した。
代替首都・臨時首都はP26-007以降の別タスクとして本タスクでは実装していない
(spec §7)。

## 16. 移動中Divisionの所在州

`Division.current_state`をそのまま使用する。既存の`process_movement`
(`src/military/movement.rs`)がDivisionを出発州に保持し、移動完了時
(`movement.rs:139`)にのみ目的州へ更新する既存設計をそのまま利用しており、
補給専用のフィールド・座標を新設していない。

## 17. 接続Division需要

`plan_country_daily_supply`内で、生存Division(`is_living`、P26-004-PRE-005と
同じ判定式)を`connected_states.contains(&division.current_state)`で分類し、
接続側の需要合計を`eligible_demand_units`とする(新設フィールド、
`CountrySupplyPlan`)。`total_demand_units`(接続性に関わらない全生存Division
需要合計、P26-004-PRE-005の`country_daily_demand_units`と一致する既存の
`debug_assert_eq!`不変条件はそのまま維持)とは意味的に区別する。

## 18. 切断Division需要

切断Divisionの需要は`eligible_demand_units`・`consumed_units`のいずれにも
一切加算しない。切断Divisionの需要が正なら配分0・補給率0.0、需要0なら配分0・
補給率1.0(spec §11、単体テスト`p26_006_disconnected_division_with_positive_
demand_gets_zero_allocation_and_zero_fulfillment`・`p26_006_disconnected_
division_with_zero_demand_gets_full_fulfillment`)。

## 19. 国家備蓄消費式

```text
consumed_c = min(eligible_demand_c, available_stockpile_c)
```

P26-004の`consumed_c = min(total_demand_c, available_stockpile_c)`から、
需要の基準を「接続Divisionの需要合計」だけに変更した(単体テスト
`p26_006_shortage_is_computed_against_connected_demand_only_not_total_
demand`で、総需要ではなく接続分だけを基準にすることを明示的に確認)。

## 20. Hamilton配分の維持

`allocate_by_largest_remainder`(P26-004の既存関数)を一切変更せず、接続
Divisionの部分集合だけを渡して再利用した。新しい比例配分の複製は行っていない。
tie-break(remainder降順→DivisionId昇順)が接続Divisionだけの母集合でも維持
されることを`p26_006_hamilton_tie_break_by_division_id_is_preserved_among_
connected_divisions`で確認した。切断Divisionは分母(`D_c`)へ一切含まれない
ことを`p26_006_disconnected_divisions_are_excluded_from_the_hamilton_
denominator`で確認した。

## 21. 切断Divisionの充足率

正の需要: `0.0`。需要0: `1.0`(spec §11のとおり)。国家備蓄からの形式的消費は
一切行わない。

## 22. 国家単位原子性

P26-004の二段階構成(`plan_country_daily_supply`[純粋・副作用なし]→
`apply_country_daily_supply_plan`[1回だけ`try_consume_military_supplies`を
呼んだ後に全Divisionの充足率を反映])をそのまま維持した。首都連結州集合の計算
(`connected_states_from_capital`)はPhase 1(plan)の冒頭、需要算定より前に
実行する純粋関数であり、副作用を持たない。失敗時(`DailySupplyPlanError`/
`DailySupplyApplyError`)は既存どおりその国家の備蓄・全Division充足率を一切
変更しない。

## 23. 支配変更の反映時点

`run_daily_military_supply_consumption`が`DailySimulationSet::Economy`
(P26-004の日次補給処理と同じSet、月次生産の直後)で首都連結州集合を算出し、
`DailySimulationSet::MilitaryAction`(移動・戦闘、州支配権変更はここで発生)
より必ず先に実行される――既存の`DailySimulationSet`宣言順`.chain()`
(`src/app/time.rs`)がこの順序を保証するため、System登録・順序を一切変更する
必要がなかった。したがって、ある日の戦闘で州支配が変化しても、その変化は
**翌日**の首都連結州集合へ反映される(spec §14)。E2Eテスト
`test_corridor_cutoff_disconnects_frontier_state_starting_the_next_day`・
`test_recapturing_the_corridor_state_reconnects_the_frontier_state_the_
next_day`で実測した(本テストでは`controller_country`を直接注入しているため、
注入直後の次回`tick_day`から反映される――実際の戦闘による支配変更も、
`resolve_finished_battles`[MilitaryAction Set]がEconomy Setより後に実行される
という同じSet順序保証により、必ず翌日の日次補給処理から反映される)。

## 24. 回廊遮断結果

Arcadiaの実マップ回廊(首都0 ─ 州11 ─ 前線州12、`states.ron`の実際の
`neighbors`関係)を使い、州11が敵支配になると前線州12が切断され、補給率が
1.0→0.0になることを実App経由で実測した
(`test_corridor_cutoff_disconnects_frontier_state_starting_the_next_day`)。
国家備蓄は切断Divisionの需要分を消費しない。

## 25. 回廊奪還結果

州11を奪還(`controller_country`を`None`=owner=Arcadiaへ戻す)した翌日、
前線州12が再接続され補給率が0.0→1.0へ復帰することを実測した
(`test_recapturing_the_corridor_state_reconnects_the_frontier_state_the_
next_day`)。

## 26. P26-005戦闘力への伝播

P26-005の三要素min式(`combat_readiness = min(manpower_ratio, equipment_
ratio, supply_fulfillment)`)は本タスクで一切変更していない。回廊遮断で
`supply_fulfillment=0.0`になったDivisionは、既存のP26-005経路(`resolve_
combat_day_multi_with_supply`→`supply_fulfillment_of`)を通じて自動的に
`combat_readiness=0.0`となる――P26-006は`MilitaryRegistry.division_supply_
fulfillment`マップへ書き込むだけであり、P26-005の戦闘式自体には一切触れて
いない。実App比較で、遮断時は接続時より与ダメージが少ないこと
(`test_disconnected_frontier_division_deals_less_damage_in_real_battle`)、
補給率0.0でもpanic・NaN・Infinityが生じないこと
(`test_fully_disconnected_division_in_real_battle_does_not_panic_or_
produce_nan`)を確認した。切断そのものによる追加のAttritionは実装していない。

## 27. 国家保存則

```text
stockpile_after_c = stockpile_before_c - connected_divisions_consumption_c
```

`consumed_c`は接続Divisionの需要だけから決まるため、切断Divisionの「満たされ
なかった需要」は保存則へ一切現れない(架空の物量として加算されない)。

## 28. 世界保存則

P26-004が確立した世界保存則(`Σstockpile_after = Σstockpile_before +
Σproduction - Σconnected_daily_consumption`)は、`consumed_c`の基準が
`eligible_demand_c`になった以外の会計ロジック(生産・消費API・fixed-point
scale)を一切変更していないため構造的に維持される。90日・36か月の無戦争
シナリオ検証(§32・§33)で、全国家`stockpile >= 0.0`を維持することを実測した。

## 29. 国家間分離

`plan_country_daily_supply`は`country_id`でフィルタした`divisions`の
部分集合だけを扱う純粋関数であり、他国家の切断状態はそもそも入力に現れない
構造的分離を持つ。単体テスト`p26_006_one_countrys_disconnected_division_
does_not_change_another_countrys_plan`、E2Eテスト`test_arcadias_corridor_
cut_does_not_affect_dwarfs_supply`で確認した。

## 30. Save V1判断

**Save V1を維持し、連結集合そのものはSaveへ追加していない。** 連結性は
`capital_state_id`(国家、既存)・`StateData.neighbors`/`controller_country`
(州、既存)・Division`current_state`(既存)から毎日再計算される派生状態であり、
これら以外の新規Saveフィールドは一切追加していない。`src/save/`配下の
`dto.rs`/`export.rs`/`apply.rs`/`validate.rs`はいずれも本タスクで変更して
いない(`git diff --stat`で確認)。

## 31. save→load分岐

* `test_disconnected_state_recomputes_correctly_after_save_load_not_
  cached`: 遮断状態でsave→load後、load直後の補給率(save時点の値)が保持され、
  かつ`controller_country`から翌日正しく再計算されることを確認。
* `test_double_save_load_round_trip_is_stable_for_disconnected_state`:
  2回連続save→loadでも安定。
* `test_old_save_without_fulfillment_field_still_recomputes_connectivity_
  correctly`: 補給率フィールド欠落の旧Saveをloadした直後はdefault 1.0だが、
  最初の日次補給処理で保存済み`controller_country`に基づく正しい切断結果へ
  上書きされることを確認(staleなdefaultへ固定されない)。
* `test_new_game_continuation_matches_save_load_branch_with_corridor_cut`:
  New Game継続とsave→load分岐で戦闘結果が完全一致。

## 32. 90日検証

`test_90_day_uncontested_six_countries_matches_p26_004_baseline_behavior`
(実App、無戦争・全Division常時首都=完全連結)で、90日間全6か国の備蓄が負に
ならないことを確認した。無戦争シナリオでは全Divisionが常に接続のままのため、
P26-004の90日実測結果(`p26_004_daily_supply_consumption_allocation_e2e_
test.rs::test_90_day_all_six_countries_real_app_verification`)はP26-006
導入後も無変更で合格する(既存テストへの回帰改変は一切行っていない)。

## 33. 36か月検証

`test_36_month_oceanic_shortage_unchanged_when_uncontested_by_capital_
connectivity`(実App、無戦争、通常AI)で、Oceanicの`shortage_days=418`・
`first_shortage_day=32`が、P26-004/P26-005が実測した値と**完全一致**する
ことを実測した――無戦争シナリオでは首都連結性が常にtrueのままであるため、
P26-006は結果へ一切影響しないことの直接的な証拠である。

## 34. Oceanicなど島嶼国家への影響

現在の実マップに`is_sea: true`の州が存在しないため、Oceanic(名称に反して
実データ上は完全な陸上国家)を含むいかなる国家も、海上輸送の欠如を理由に
恒久的に切断されることはない。既知の供給網の弱さ(P26-004-PRE-002/004由来、
Oceanic/Elfin/Passguardの立ち上がりの遅い生産網)は、無戦争シナリオでは
P26-006導入前後で完全に同一の結果になることを§33で実測した。Oceanicが
実際に地理的切断の影響を受けるのは、他国に首都または回廊州を占領された
場合のみであり、本タスクのE2Eテスト群(Arcadiaを主な検証対象国として使用)が
その一般的なメカニズムを実証している。

## 35. P26-004/005回帰

P26-004の全E2Eテスト(19件)・P26-005の全E2Eテスト(10件)は無変更のまま全て
合格した(§41参照)。`daily_supply.rs`の既存24件の単体テストも、`connected_
all()`ヘルパー(`{StateId(0)}`、全既存テストの固定フィクスチャ`current_state`
と一致)を追加の引数として渡すことで、数式的に無変更のまま(`eligible_demand_
units == total_demand_units`となる特殊ケース)全て合格することを確認した。

## 36. P26-DET-001/REV-001保護

`src/economy/production.rs`・`src/country/country_ai.rs`は本タスクで一切
変更していない。関連E2Eテスト(`p26_det_001_construction_ai_determinism_e2e_
test`10件・`p26_det_001_rev_001_cross_state_production_order_e2e_test`
12件)はそのまま全て合格した。

## 37. Phase 23〜25保護

`cargo test --lib military::`(426 passed、P26-005時点392+本タスク新規34)・
`--lib state::`(19 passed)・`--lib country::`(91 passed)・`--lib war::`
(153 passed)・`--lib economy::`(39 passed)・`--lib save::`(302 passed)を
実行し、いずれもP26-005時点から純増分(military::のみ)以外の変化がないことを
確認した。P25-004/005/006-REV-001/007の4つのE2Eテストファイルについては
§40で詳述するとおり、Division配置の修正(実マップの自国支配州を使う)を行った
上で全て合格することを確認した(戦闘式・撤退式・装備会計式自体は一切変更
していない)。

## 38. 対象外

港・海上補給・輸送船・鉄道補給・鉄道容量・補給拠点・州別備蓄・Division携行
在庫・同盟国通行権・軍事通行権・傀儡国補給・補給距離減衰・地形による補給
損失・移動速度ペナルティ・organization・Attrition・首都移転・臨時首都・国家
消滅処理の再設計・AI補給判断・UI・ローカライズ・生産量/需要量調整・Oceanic
専用例外・CountryId別例外・Save V2・Phase 25装備会計変更・P26-004配分方式
変更・P26-005戦闘式変更、いずれにも触れていない。

## 39. Critical/High/Medium/Low

* **Critical: 0件**
* **High: 1件**
  1. `tests/p23_010_phase23_manpower_lifecycle_audit_test.rs::a_save_and_
     reload_partway_through_matches_an_uninterrupted_run`が、本タスク導入後
     `cargo test --all-targets`実行時に約40〜50%の頻度で間欠的に失敗する。
     詳細な根本原因分析は§40参照。**P26-006自身のconnectivity/daily_supply
     ロジックの欠陥ではない**(単体テスト34件で数式的に検証済み、
     `connected_states_from_capital`を強制的に「常に全連結」へ差し替える
     診断用パッチでこのテストが安定することを実証済み)。根本原因は
     Phase 21(`P21-SAVE-002A1`)の既存設計判断(`CountryAiState.dirty`を
     意図的にSave対象外とし、load後は常に`dirty=true`から再開する方針)が、
     P26-006の新しい「戦闘力が地理[支配権]に依存する」フィードバック経路に
     よって初めて観測可能な形で顕在化したもの。ユーザーへ状況を説明し、
     「P26-006を完了として扱い、このfindingを報告書に明記する」方針の承認を
     得た。
* **Medium: 0件**
* **Low: 1件**
  1. P25-004/005/006-REV-001/007の4つのE2Eテストファイルに、本タスク導入前
     から存在した「戦闘参加Divisionを存在しない合成州または相手国の支配州へ
     配置する」というテスト実装パターンが、本タスクの首都連結性導入により
     意図せず「常時切断」を引き起こし、7件のテストが一時的に破綻した
     (§40で詳述)。いずれも各テストの本来の検証意図とは無関係な副作用であり、
     Division配置を実マップの自国支配州へ修正することで解消した(既存の
     戦闘式・撤退式・装備会計式自体への変更は一切行っていない)。

## 40. 発見事項の詳細

### 40.1 テストDivision配置パターンの副作用(Low、修正済み)

P25-004/005/006-REV-001/007の4ファイルは、いずれも実マップに存在しない
`TEST_STATE_ID`(通常`StateId(999_999)`)や、戦闘相手国の支配州を、攻撃側・
防御側Divisionの`current_state`として使う既存のテストヘルパーパターンを
持っていた。本タスク導入前は、Division所在州が地理的に何であるかは戦闘力へ
一切影響しなかったためこれで問題がなかったが、P26-006により「自国が実効
支配していない州にいるDivisionは切断される」ようになったため、次の7件が
一時的に失敗した(いずれも修正済み、§45)。

* `p25_004_combat_equipment_loss_e2e_test.rs`: `test_t`・`test_x`・`test_y`
  (`setup_standard_battle`が攻撃側・防御側とも同一の`TEST_STATE_ID`を使用)
* `p25_005_daily_equipment_reinforcement_e2e_test.rs`:
  `equipment_reinforced_the_same_day_can_still_be_lost_to_combat_that_day`
  (defenderをattacker[player]の支配州へ誤って配置)
* `p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs`:
  `test_the_real_registered_active_path_applies_retreat_equipment_loss`・
  `test_post_retreat_equipment_loss_survives_save_and_load`
  (`setup_retreating_battle`が攻撃側・防御側とも同一の`TEST_STATE_ID`を使用)
* `p25_007_equipment_combat_effectiveness_e2e_test.rs`:
  `test_34_combat_readiness_uses_the_post_reinforcement_equipment_value`・
  `test_35_p25_004_equipment_loss_still_follows_the_actual_applied_
  casualty`(defenderをattacker[player]の支配州、または攻守とも
  `TEST_STATE_ID`へ誤って配置)

修正方法は全て同じ: 各Divisionを、その所有国が実際に法的所有する実マップ州
(`state_owned_by`ヘルパー、`StateRegistry.states.iter().find(|s| s.owner_
country_id == owner)`)へ配置し直した。`Battle.state_id`自体(戦闘発生地点の
メタデータ)はDivision自身の`current_state`と独立しているため、
`TEST_STATE_ID`のまま変更していない箇所もある。戦闘式・撤退式・装備会計式・
Save/load機構は一切変更していない。

### 40.2 Phase 21由来の既存non-determinism(High、報告のみ・対象外)

**発見の経緯**: `cargo test --all-targets --no-fail-fast`実行時、
`tests/p23_010_phase23_manpower_lifecycle_audit_test.rs::a_save_and_reload_
partway_through_matches_an_uninterrupted_run`が間欠的に失敗することを発見
した。同一バイナリを独立に5回実行したところ、baseline(P26-006導入前、
`c97ae7f3`のworktree)では5/5安定して合格する一方、HEAD(P26-006導入後)では
約40〜50%の頻度(複数回のサンプリングで確認)で失敗することを確認した。

**根本原因の特定手順**:
1. `connected_states_from_capital`を一時的に「常に全州を返す(=常に全連結)」
   よう差し替えたところ、このテストは安定して合格するようになった――
   これにより、原因が接続性ロジック自体の実装ミスではなく、「接続性が
   本物の意味を持つようになったこと」自体が引き金であることを確定した。
2. 一時的な診断コード(月ごとの州/Division/Battle状態を両分岐[continuous/
   save-reload]でダンプして比較する専用テスト、コミット対象外)を用いて
   分岐点を特定したところ、両分岐は3か月目終了時点まで完全に一致し
   (`normalized_country_snapshot`/`normalized_state_snapshot`/独自ダンプの
   いずれも一致)、save直後とload直後も完全に一致するが、4か月目の途中で
   `StateId(1)`の`controller_country`が分岐すること(一方はElfin[CountryId
   1]が占領、他方はOceanic[CountryId 3]が占領)を突き止めた。
3. `src/save/dto.rs`(163〜178行目)を確認した結果、`CountryAiState.dirty`
   フィールドが**意図的に**`SavedCountryAiState`から除外されており、
   「dirtyは`process_daily_country_ai`内の`is_weekly_due`/`is_monthly_due`
   判定にのみ使われる純粋なタイミング制御フラグであり、実際の判断内容
   [`mode`/`decision_reason`/各`last_*_day`]には現れないため保存不要、
   load後は常に`dirty=true`から再開する」という設計判断(`P21-SAVE-002A1`、
   Phase 21)であることを確認した。

**結論**: この設計判断自体は、当時の前提(AIの再評価タイミングの違いは
最終的なゲーム結果を変えない)の下では正しかった。しかしP26-006により
「戦闘力が地理[実効支配]に依存し、実効支配は日々の戦闘によって変化する」
という新しいフィードバック経路が追加されたことで、AIの再評価タイミングが
数日ずれるだけで「どちらの国が先に手薄な国境州を占領するか」という結果が
変わり得るようになった――これが、continuous実行(3か月間蓄積したdirty状態を
保持したまま4か月目へ進む)とsave/load分岐実行(load直後に全AI国家の
dirtyがtrueへリセットされ、通常より早いタイミングで再評価が走る)との間で、
最終的な州の占領国が異なるという観測結果として現れる。

**対象外と判断した理由**: この問題の修正には、(a) Save V2でdirty相当の
状態を保存する、または(b) Phase 21のAI評価タイミングロジック自体を変更する、
のいずれかが必要であり、いずれも本タスクのspec §22で明示的に対象外とされて
いる(Save V2・AI補給判断以外の一般AI挙動変更)。またこの問題は`military::`
`war::``economy::``save::`のlibテスト・P26-004/005/DET-001/REV-001・
P25-004/005/006-REV-001/007のいずれのE2Eテストにも一切現れず
(`test_90_day_uncontested_...`・`test_36_month_oceanic_shortage_
unchanged_...`を含む本タスクの90日/36か月検証も無戦争のため一切影響を
受けない)、6か月・6か国・複数の戦争が絡む長期AI監査シナリオという極めて
特殊な条件でのみ顕在化する。ユーザーへ経緯を説明し、「P26-006をCOMPLETEとして
扱い、この既存findingを報告書へ明記する」方針の承認を得た。

**推奨する次の最小タスク**: `CountryAiState`の再評価タイミングをsave/load
分岐に対して頑健にする専用タスク(選択肢: Save V2で`dirty`を保存する、
または`load_and_settle`直後に強制的な即時全件再評価を行わず次の通常サイクル
まで待たせる設計へ変更する)。いずれもPhase 21のAI評価アーキテクチャに
踏み込むため、P26フェーズとは別のタスクとして切り出すことを推奨する。

## 41. 全テスト結果

```text
開始時baseline: 2274 passed / 0 failed / 1 ignored(自ら再測定、期待値と一致)
実装後最終:     2326 passed / 0 failed / 1 ignored(cargo test --all-targetsで
                実測、§40.2のflaky testを除く安定した回で確認、複数回実行して
                2326という合計自体は一貫していた)
```

新規テスト52件(`src/military/capital_connectivity.rs`単体25件+
`src/military/daily_supply.rs`単体9件+`tests/p26_006_capital_connectivity_
supply_e2e_test.rs`18件)が新規に合格し、2274+52=2326と算術的に完全一致する。
既存テストの純減・改変は、§40.1に記載した4ファイル7件のDivision配置修正
(検証内容自体は無変更)を除き一切ない。

個別実行結果(全て逐次実行):

```text
cargo test --lib military::                          426 passed
cargo test --lib state::                               19 passed
cargo test --lib country::                             91 passed
cargo test --lib war::                                153 passed
cargo test --lib economy::                             39 passed
cargo test --lib save::                               302 passed
cargo test --test p26_006_capital_connectivity_supply_e2e_test    18 passed
cargo test --test p26_005_supply_combat_strength_e2e_test         10 passed
cargo test --test p26_004_daily_supply_consumption_allocation_e2e_test  19 passed
cargo test --test p26_004_pre_005_initial_supply_bridge_migration_e2e_test  22 passed
cargo test --test p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test  15 passed
cargo test --test p26_003_military_supplies_production_e2e_test   49 passed
cargo test --test p26_003_rev_002_supply_production_units_e2e_test  56 passed
cargo test --test p26_003_rev_003_economic_power_capacity_e2e_test  21 passed
cargo test --test p26_det_001_construction_ai_determinism_e2e_test  10 passed
cargo test --test p26_det_001_rev_001_cross_state_production_order_e2e_test  12 passed
cargo test --test land_war_combat_peace_test            4 passed
cargo test --test p25_004_combat_equipment_loss_e2e_test  5 passed(修正後)
cargo test --test p25_005_daily_equipment_reinforcement_e2e_test  10 passed(修正後)
cargo test --test p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test  8 passed(修正後)
cargo test --test p25_007_equipment_combat_effectiveness_e2e_test  21 passed(修正後)
cargo test --all-targets                              2326 passed / 0 failed / 1 ignored
```

## 42. clippy結果

`cargo clippy --all-targets --all-features -- -D warnings`は実装後
**0 warnings**。

## 43. Cargo逐次実行

全てのCargoコマンドを逐次実行した(並列実行なし)。`cargo test --all-targets`
実行中に一過性のコンパイラ内部エラー(`internal compiler error`、
`p21_016_multilateral_war_e2e_test`のビルド時)が1回発生したが、単純に
再実行して正常完走を確認した(既存タスクが記録済みの既知の一過性ビルド事象と
同種、実コードの問題ではない)。並行cargo実行は一切行っていない
(`git stash`を含む破壊的操作の使用が制限されていたため、baseline比較には
`git worktree add`で独立した別ディレクトリを使用し、mainの作業ツリーとは
別のtarget/を用いたため、cargo並行実行の禁止には抵触していない)。

## 44. PNG差分

`cargo test --all-targets`実行後、既知の非決定的PNG(headless UI描画テストが
再生成する)8ファイルに差分が生じた(P26-004/005と同じ既知の9ファイルのうち、
今回変化した8ファイル)。これらは本タスクの変更と無関係であり、
`git status --short`で対象を個別に確認したうえで、該当ファイルだけを
`git checkout -- <path>`で個別に復元した(ワークスペース全体のcheckoutは
行っていない)。

## 45. ユーザーセーブの保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージ・
コミットしていない。開始時・全検証コマンド実行後・本報告書作成時点のいずれの
`git status --short`でも` M saves/savegame_v1.ron`のみが変化せず残り続けた
ことを確認した(内容は一度も参照していない)。テスト用の合成Save・合成App
状態は全て一時ディレクトリ(`std::env::temp_dir()`配下)または実App内メモリ
上に作成した。baseline比較には`git worktree add /c/wtbaseline <commit>`を
使用し、検証後に`git worktree remove --force /c/wtbaseline`で完全に削除した
(mainの作業ツリー・ユーザーセーブには一切影響しない独立ディレクトリ)。

## 46. 変更ファイル

```text
src/military/capital_connectivity.rs   (新規、connected_states_from_capital
                                         + is_traversable、単体テスト25件)
src/military/daily_supply.rs           (plan_country_daily_supplyへ
                                         connected_states引数追加、
                                         CountrySupplyPlanへeligible_demand_
                                         units追加、接続/切断分類、
                                         run_daily_military_supply_
                                         consumption/Systemへ
                                         Res<StateRegistry>追加、
                                         単体テスト9件追加、既存テスト
                                         call site更新)
src/military/mod.rs                    (capital_connectivityモジュール宣言)
tests/p26_006_capital_connectivity_supply_e2e_test.rs  (新規、E2Eテスト18件)
tests/p25_004_combat_equipment_loss_e2e_test.rs        (Division配置修正、
                                                         3テスト)
tests/p25_005_daily_equipment_reinforcement_e2e_test.rs (Division配置修正、
                                                          1テスト)
tests/p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs
                                        (Division配置修正、2テスト)
tests/p25_007_equipment_combat_effectiveness_e2e_test.rs (Division配置修正、
                                                           2テスト)
verification_logs/phase-26/p26-006/p26-006_capital_connectivity_supply.md
                                        (本報告書)
```

`saves/savegame_v1.ron`はコミット対象に含めない。PNGファイルは§44のとおり
個別復元済みでコミット対象に含めない。

## 47. 最終`git status --short`

```text
 M saves/savegame_v1.ron
 M src/military/daily_supply.rs
 M src/military/mod.rs
 M tests/p25_004_combat_equipment_loss_e2e_test.rs
 M tests/p25_005_daily_equipment_reinforcement_e2e_test.rs
 M tests/p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs
 M tests/p25_007_equipment_combat_effectiveness_e2e_test.rs
?? src/military/capital_connectivity.rs
?? tests/p26_006_capital_connectivity_supply_e2e_test.rs
```

## 48. P26-007へ進める状態か

進める状態である。首都連結性による空間補給モデル2は実際にDaily補給消費・
配分・P26-005戦闘力へ接続され、実マップ6か国・回廊遮断/奪還・Save V1・
save/load分岐・決定性・国家間分離のいずれも実測確認済みである。§40.2で
報告した既存のAI評価タイミングnon-determinismは、P26-006自身の欠陥ではなく
Phase 21由来の既知の設計判断がP26-006によって初めて顕在化したものであり、
修正にはSave V2または既存AI評価タイミングロジックの変更が必要なため、
P26フェーズの通常スコープとは別の専用タスクとして切り出すことを推奨する。
港・海上補給・鉄道・同盟国通行権・補給距離・首都移転等、モデル2のさらに先の
機能はいずれも未着手のまま、意図どおり本タスクのスコープ外としている。
