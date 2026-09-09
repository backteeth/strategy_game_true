# P26-006-REV-001: Save後のCountry AI再評価タイミング決定性修正 検証報告書

## 1. 総合判定

```text
P26-006-REV-001 COMPLETE
ROOT CAUSE CONFIRMED (ただしタスク仕様が想定した`CountryAiState.dirty`は原因ではなく、
                       別の2箇所[process_movement/process_daily_frontline_plans]の
                       HashMap反復順非決定性が真の原因であることを実測で確定した)
SAVE CONTINUITY RESTORED
P23-010 STABLE
P26-006 REGRESSION FREE
P26-007 READY
```

**総合判定: COMPLETE**

## 2. 開始時HEAD

```text
06101b3c1330ec5b647477a910bf9bd4664a1efd
```

`git rev-parse HEAD`・`git status --short`で確認し、想定(`feat: gate military
supply by capital connectivity`、P26-006のコミット)と一致していた。既存差分は
` M saves/savegame_v1.ron`のみ。

## 3. 問題の背景

タスク仕様は、`tests/p23_010_phase23_manpower_lifecycle_audit_test.rs::a_save_
and_reload_partway_through_matches_an_uninterrupted_run`がP26-006導入後に
間欠的に失敗する原因を、Phase 21(`P21-SAVE-002A1`)が`CountryAiState.dirty`を
意図的にSave対象外とし、ロード後は常に`dirty=true`から再開する設計に求めていた
(P26-006自身の検証報告書§40.2で立てた仮説)。本タスクはこの仮説を鵜呑みにせず、
実コードから独立に再監査するよう明示的に指示されていた。

## 4. 修正前の再現回数

独立した`cargo test`プロセスを6回連続実行し、3 passed / 3 failed(50%)を確認した
(要求された「最低1回の失敗確認、最大20回」を満たした時点で次工程[原因特定]へ
進んだ)。

```text
run 1: FAILED
run 2: ok
run 3: ok
run 4: ok
run 5: FAILED
run 6: FAILED
```

## 5. 最初の意味的分岐

一時的な診断テスト(`CountryAiRegistry`・`StateRegistry`・`MilitaryRegistry`・
`WarRegistry`・`ClaimRegistry`・`CrisisRegistry`・`BattleRegistry`・
`MilitaryAiRegistry`・`FrontlineRegistry`を月4の日ごとに全ダンプして比較、
コミット対象外)により、次を確定した。

* 3か月目終了時点(save直前)のダンプは、継続分岐とsave直前分岐とで**完全一致**。
* save直後 vs load直後のダンプも**完全一致**(Save/Load自体のシリアライズ・
  デシリアライズは正確)。
* 月4の**日1〜6は完全一致**。**日7で初めて分岐**。
* 日7時点で`CountryAiState`(`mode`/`decision_reason`/`last_daily_evaluation_
  day`/`last_weekly_evaluation_day`/`last_monthly_evaluation_day`/
  `cooldown_until_day`/`dirty`)は**全国家について両分岐で完全一致**
  (`dirty=true`で両分岐とも同一)。`MilitaryAiState`(`last_evaluated_day`/
  `last_decision_reason`/`estimated_own_power`/`estimated_enemy_power`/
  `dirty`)も**全国家について両分岐で完全一致**。
* 一方、`StateData.controller_country`(StateId(1))と`Division.current_state`/
  `status`(Elfin・Oceanicの全師団)が両分岐で**入れ替わって**いた
  (継続分岐ではElfinが州1を占領しOceanicは自国に留まる、split分岐ではOceanicが
  占領しElfinが自国に留まる)。
* `Frontline.attacker_front_regions`(war0=Elfin、war2=Oceanic)も対応して
  入れ替わっていた。

この時点で、「AI再評価タイミング(`dirty`)」は分岐の原因ではなく、**軍事的な
占領競合(どちらの師団が先に無防備な州へ到達するか)**が原因であることが判明した。

## 6. 根本原因

`src/military/movement.rs::process_movement`が、移動処理対象のDivisionId
リストを`military_registry.divisions.keys().copied().collect()`で収集した
まま、**`DivisionId`昇順へソートせずに**処理していた。

互いには交戦していない2か国(本シナリオではElfin・Oceanic)が、それぞれ独立に
第3国(Arcadia)と交戦しており、Arcadiaの同じ無防備な州(StateId(1)、実マップで
Elfin[3]ともOceanic[8]とも隣接)へ両国の師団が同日中に到達する場合、
`military::invasion::process_division_arrival`の「敵軍なし→即占領」判定は、
`process_movement`のループ内で**先に処理された師団の所属国**を勝者にする。
`military_registry.divisions`はRustの標準`HashMap`であり、その反復順序は
プロセスごとにランダムなハッシュシード(`RandomState`)に依存するため、
**同じ論理状態から出発しても、Appインスタンス(=プロセス)が異なれば占領国が
異なり得た**。

`continuous`分岐(3か月間動き続けてきた同一Appインスタンス)と`reloaded`分岐
(save→loadで新規に構築されたAppインスタンス)は、`MilitaryRegistry.divisions`
というHashMapをそれぞれ独立に構築するため、内部のハッシュシードが異なり、
反復順序も異なる――これが「save/loadを挟んだかどうか」で結果が変わる直接の
メカニズムだった(「同じシナリオを2回独立実行」する既存の決定性テストが
安定していたのは、そのテストのシナリオ設定[装備備蓄ブーストなし、4か月]では、
たまたま複数国家が同じ無防備な州を同日に奪い合う場面が発生しなかったためで
あり、根本原因の存在自体を否定するものではないことも確認した)。

副次的に、`src/war/frontline.rs::process_daily_frontline_plans`も同様の
パターン(`frontline_registry.frontlines.keys().copied().collect()`を
未ソートのまま処理)を持っていたため、こちらも合わせて修正した(単独では
本テストの間欠失敗を再現・解消しなかったが、同じ監査で発見した同種の
潜在的非決定性であり、放置しない)。

## 7. `CountryAiState`監査結果

`src/country/country_ai.rs`の`CountryAiState`(`mode`/`decision_reason`/
`last_daily_evaluation_day`/`last_weekly_evaluation_day`/
`last_monthly_evaluation_day`/`cooldown_until_day`/`dirty`)を全項目監査した。

* `mode`/`decision_reason`/`cooldown_until_day`: `save/dto.rs`の
  `SavedCountryAiState`にすべて保存済み、既存のまま。
* `last_daily_evaluation_day`/`last_weekly_evaluation_day`/
  `last_monthly_evaluation_day`: すべて保存済みだが、**書き込み専用**
  (`process_daily_country_ai`内で更新されるだけで、`is_weekly_due`/
  `is_monthly_due`の判定式自体はこれらを一切参照しない――判定は
  `current_day % 7 == country_id.0 % 7`という絶対日ベースの式のみで、
  `last_*_day`は記録用の副産物に過ぎない)。
* `dirty`: Save対象外(既存方針)。ロード後は常に`true`から再開する。

## 8. dirtyの全変更経路

```text
true化: CountryAiRegistry::mark_all_dirty()(全国家) / mark_country_dirty(1国)
  呼び出し元: war::update::mark_ai_registries_dirty_daily
    (DailySimulationSet::WarResolution、DayChangedMessageごとに無条件で
    全AI国家をdirty化――戦争の有無に関わらず**毎日**実行される)
  呼び出し元: country::country_ai::process_war_declaration_ai内(新規宣戦布告時、
    ai_registry[MilitaryAiRegistry]のmark_all_dirtyのみ、CountryAiRegistry側は
    上記の日次無条件呼び出しでカバーされる)

false化: process_daily_country_ai内、各国家の日次処理の最後(§9参照)で
  無条件にfalseへ戻す。
```

`mark_ai_registries_dirty_daily`(`DailySimulationSet::WarResolution`)は
`DailySimulationSet::CountryAi`(`process_daily_country_ai`が属する)より
**後**に実行される(`DailySimulationSet`の宣言順`.chain()`)。したがって、
ある日の`CountryAi` Setが`dirty`を読む時点の値は、**前日の`WarResolution`
Setが無条件に立てた`true`**である。この結果、`dirty`は実質的に「常にtrue」
となり、`is_weekly_due`/`is_monthly_due`の週次・月次判定における
`current_day % 7 == country_id.0 % 7`という絶対日ベースの staggering は
**事実上常にオーバーライドされている**(この点自体は本タスクの対象[AI戦略・
評価頻度の変更]ではないため一切変更していないが、findingとして§34へ記録する)。

## 9. dirty=falseが依存する状態

`is_weekly_due`/`is_monthly_due`の判定式自体は`dirty`と絶対日modulo条件の
論理和のみであり、`dirty=false`が「有効であるために追加で必要とするキャッシュ・
候補リスト・タイマー・キュー」は**存在しない**ことを確認した。`dirty`は
純粋な1ビットのブーリアンフラグであり、他のどのフィールドとも相互依存しない。
`mark_all_dirty`/`mark_country_dirty`のいずれの呼び出し元も、`dirty`を
立てると同時に**別の(既にSave対象の)状態**(`WarRegistry`への新規War追加等)を
変更しているだけであり、`dirty`自体が指し示す「派生キャッシュ」は存在しない。

## 10. 比較した修正案

タスク仕様が提示した案A(dirty+依存状態を保存)・案B(load時に決定的再構築)・
案C(絶対タイムスタンプ方式)・案D(両経路で強制再評価)は、**いずれも
`CountryAiState.dirty`が原因であるという前提**に基づいていた。§5〜§9の監査で
この前提自体が誤りであることを実測で確定したため、これらの案はいずれも
「採用しても間欠失敗を解消しない」ことが先に判明した(実験的に確認: 診断用
パッチで`connected_states_from_capital`を強制的に「常に全連結」へ差し替えても
安定したが、`CountryAiState`は元々両分岐で完全一致していたため、dirty関連の
修正は不要かつ無意味であることが明らかだった)。

真の原因(§6)に対しては、次の2案を比較した。

* **採用案**: `process_movement`・`process_daily_frontline_plans`双方の
  ID収集箇所へ、既存の`DivisionId`/`FrontlineId`昇順ソートを追加する
  (`P26-DET-001`/`P26-DET-001-REV-001`が同じ問題クラス[HashMap反復順
  非決定性]に対して確立した既存の修正パターンをそのまま踏襲)。
* **不採用案**: 占領判定自体を「同日に到着した複数の交戦国のうち、最も強い
  戦力を持つ国が勝つ」のような別の決定的基準へ再設計する。既存のゲーム
  バランス・仕様(先着順)を変更することになり、対象外(「AI戦略や戦闘・
  補給バランスを変更するタスクではない」)に抵触するため不採用。

## 11. 採用方式

`src/military/movement.rs::process_movement`と`src/war/frontline.rs::
process_daily_frontline_plans`の両方で、HashMapから収集した処理対象IDの
`Vec`を、既存の安定ID(`DivisionId`/`FrontlineId`)昇順へ明示的にソートしてから
反復するよう変更した。既存のBattle処理(`process_daily_battles`、BattleId
昇順)・国家AI処理(`process_daily_country_ai`、CountryId昇順)・前線割当
(`assign_unassigned_divisions_to_frontlines`、DivisionId/WarId/FrontlineId
昇順)がすでに採用している確立済みのパターンと完全に一致する。

`CountryAiState.dirty`のSave除外方針(P21-SAVE-002A1)自体は変更していない
――§7〜9の監査により、この方針は現状のコードベース(`mark_ai_registries_
dirty_daily`が毎日全国家をdirty化する)の下では引き続き無害であることを
確認した。

## 12. Saveへ追加した状態

**なし。** Save DTO(`src/save/dto.rs`)・export(`src/save/export.rs`)・
apply(`src/save/apply.rs`)・validate(`src/save/validate.rs`)はいずれも
本タスクで一切変更していない(`git diff --stat`で確認)。

## 13. 再構築する状態

**なし。** ロード時の`CountryAiState.dirty=true`復元(既存のP21-SAVE-002A1
方針、`src/save/apply.rs:369-370`)を含め、ロード処理自体を一切変更して
いない。

## 14. Save V1判断

**Save V1を維持し、変更なし。** 新規フィールドは一切追加していないため、
「USER DECISION REQUIRED」に該当する状況(Save V2が不可避)は発生しなかった。

## 15. 旧Saveのdefault

変更なし(既存どおり、`dirty`はロード時常に`true`)。この既存defaultが
間欠失敗の原因ではないことを§5〜§9で確定した。

## 16. validation

`src/save/validate.rs`は本タスクで一切変更していない(新規フィールドが
存在しないため、検証すべき新規状態も存在しない)。

## 17. applyの原子性

`src/save/apply.rs`のapply処理自体は本タスクで一切変更していない。既存の
原子性契約(全体検証→適用、失敗時は既存Worldを一切変更しない)がそのまま
維持される。

## 18. New Gameへの影響

New Game開始時の挙動(`spawn_debug_divisions`等)は一切変更していない。
`process_movement`/`process_daily_frontline_plans`は移動中・前線保有中の
Divisionが存在する場合にのみ意味を持つ処理であり、New Game直後
(Division移動なし、前線なし)には実質的な処理対象が空のため影響しない。

## 19. 現行Saveの往復

新規E2Eテスト`test_d_double_save_load_round_trip_does_not_change_country_
ai_or_state_snapshots`で、係争状態を含むシナリオのsave→loadが状態を変更
しないことを確認した。

## 20. 旧Saveの初回ロード

新規E2Eテスト`test_b_old_save_without_any_new_fields_loads_successfully`で、
(本タスクが新規フィールドを追加していないため)現行Save自体が旧Save相当で
あり、正常にロードできることを確認した。

## 21. 2回目以降のロード

`test_d_double_save_load_round_trip_does_not_change_country_ai_or_state_
snapshots`で2回連続save→loadの安定性を確認した(P23-010本体の
`test_g3_double_save_load_round_trip_is_stable`系のテストも既存のまま
全て合格、§32参照)。

## 22. AI再評価日比較

新規E2Eテスト`test_a_country_ai_state_is_bit_identical_between_branches_
at_the_divergence_point`で、`CountryAiState`の全フィールド
(`mode`/`decision_reason`/`last_daily_evaluation_day`/
`last_weekly_evaluation_day`/`last_monthly_evaluation_day`/
`cooldown_until_day`)が継続分岐とsave/load分岐とで完全一致することを
実測固定した。

## 23. AI評価回数比較

診断用ダンプで、`MilitaryAiState`(`last_evaluated_day`含む)が両分岐で
完全一致することを確認済み(§5参照、恒久テストとしては§22のCountryAiState
テストで代表させている――両者とも同じ「dirtyは分岐の原因でない」という
結論を支持する)。

## 24. 建設・募兵比較

`CountryAiState.mode`/`decision_reason`が完全一致することから
(`process_construction_ai`はこれらの状態のみを更新し、その入力は既に
Save済みの`CountryRegistry`/`StateRegistry`/`BuildingRegistry`のみに
依存する)、建設判断も間接的に一致することを確認した。既存の
`p23_010`本体テスト(`full_lifecycle_cycle_preserves_the_manpower_ledger_
at_every_stage`等)が募兵判断の一致を直接検証しており、そのまま全合格した。

## 25. 移動・戦闘比較

新規E2Eテスト`test_m_contested_border_state_outcome_matches_between_
continuous_and_save_load_branches`で、州の占領国・全Divisionの
`current_state`/`status`/`manpower`が継続分岐とsave/load分岐とで完全一致
することを実測固定した。

## 26. controller比較

上記`test_m_...`のStateSnapshot比較に含まれる(`owner`/`controller`両方を
比較)。

## 27. 補給接続性・補給率の一致

新規E2Eテスト`test_i_capital_connectivity_recomputes_correctly_regardless_
of_which_country_wins_the_race`で、占領競合の結果(どちらの国が勝つか)に
関わらず、P26-006の首都連結性が勝者側から見て正しく再計算されることを
確認した。

## 28. 国家間分離

新規E2Eテスト`test_j_uninvolved_countrys_state_is_unaffected_by_the_
contested_race_outcome`で、係争と無関係な第三国(Dwarf)の財政状態が
save/load分岐に関わらず一致することを確認した。

## 29. `p23_010`独立プロセス20回結果

```text
実行1(単一テスト、修正前診断用): 3 passed / 3 failed(50%、§4参照)
実行2(単一テスト、process_daily_frontline_plansのみソート): 6 passed / 14 failed
  (frontline側の修正だけでは不十分であることを実測確認)
実行3(単一テスト、process_movement追加ソート後): 20 passed / 0 failed
実行4(単一テスト、再確認): 20 passed / 0 failed
実行5(p23_010ファイル全体[9テスト]、最終確認): 20 passed / 0 failed
```

いずれも独立した`cargo test`プロセスとして逐次実行した(並列実行なし)。

## 30. 新規E2E独立プロセス結果

`tests/p26_006_rev_001_ai_save_resume_determinism_e2e_test.rs`(9テスト)を
独立プロセスとして5回実行し、5/5すべて合格した。

## 31. P26-006回帰

`tests/p26_006_capital_connectivity_supply_e2e_test.rs`(18件)は無変更で
全て合格した。`src/military/capital_connectivity.rs`は一切変更していない。

## 32. P26-005回帰

`tests/p26_005_supply_combat_strength_e2e_test.rs`(10件)は無変更で全て
合格した。`src/military/combat_calc.rs`は一切変更していない。

## 33. P26-004回帰

`tests/p26_004_daily_supply_consumption_allocation_e2e_test.rs`(19件)・
`p26_004_pre_005_...`(22件)・`p26_004_pre_004_...`(15件)はいずれも無変更で
全て合格した。`src/military/daily_supply.rs`は一切変更していない。

## 34. Phase 23〜25回帰

`cargo test --lib country::`(91)・`--lib save::`(302)・`--lib war::`
(153)・`--lib state::`(19)・`--lib economy::`(39)はいずれもP26-006時点から
変化なし(military::のみ+2、新規determinismテスト分)。P26-006で配置修正した
`tests/p25_004*`(5件)・`p25_005*`(10件)・`p25_006_rev_001*`(8件)・
`p25_007*`(21件)はいずれも無変更で全て合格した。

**Finding(Low、対象外・修正せず)**: `mark_ai_registries_dirty_daily`が
毎日全AI国家を無条件でdirty化するため、`is_weekly_due`/`is_monthly_due`の
`current_day % 7 == country_id.0 % 7`という絶対日staggeringは事実上常に
オーバーライドされ、全AI国家が毎日週次・月次評価を実行している(意図した
負荷分散が機能していない可能性がある)。本タスクは「AI戦略・評価頻度の
変更」を明示的に対象外としているため一切変更していないが、将来のAI
パフォーマンス最適化タスクの候補として記録する。

## 35. 決定性

`process_movement`/`process_daily_frontline_plans`の新規単体テスト
(`arrival_race_between_two_unrelated_attackers_is_decided_by_division_id_
not_insertion_order`・`arrival_race_result_is_stable_across_many_
insertion_orders`)で、HashMap挿入順序を変えても占領結果が
`DivisionId`昇順で決定的に決まることを直接確認した。E2Eテスト
`test_k_division_map_insertion_order_does_not_affect_the_contested_race_
outcome`でも、実App経由で同じ性質を確認した。

## 36. 国家間分離

§28参照。

## 37. Critical/High/Medium/Low

* **Critical: 0件**
* **High: 0件**(発見はしたが、原因特定・修正・再発防止テストまで完了済み)
* **Medium: 0件**
* **Low: 1件**(§34参照、AI週次/月次評価staggeringの実質的無効化、対象外・
  修正せず)

## 38. 全テスト件数

```text
開始時baseline: 2326 passed / 0 failed / 1 ignored(P26-006報告書の記録と一致)
実装後最終:     2337 passed / 0 failed / 1 ignored(+11、cargo test
                --all-targetsで2回連続実測して安定)
```

新規テスト11件(`src/military/movement.rs`単体2件+`tests/p26_006_rev_001_
ai_save_resume_determinism_e2e_test.rs`9件)が新規に合格し、
2326+11=2337と算術的に完全一致する。

## 39. clippy警告数

`cargo clippy --all-targets --all-features -- -D warnings`は実装後
**0 warnings**。

## 40. Cargo逐次実行

全てのCargoコマンドを逐次実行した(並列実行なし)。

## 41. 対象外の先行実装なし

AI戦略・優先順位・募兵目標の再設計、戦争バランス調整、補給生産・消費量の
調整、首都接続性ルールの変更、港・海上補給・鉄道補給・補給距離・同盟国
通行権・首都移転・organization・Attrition・移動速度補正・交易・UI・
ローカライズ、いずれにも触れていない。

## 42. PNG差分処理

`cargo test --all-targets`実行後、既知の非決定的PNG8件に差分が生じたため、
`git status --short`で対象を確認した上で該当ファイルだけを`git checkout --`
で個別に復元した。

## 43. ユーザーセーブ保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージ・
コミットしていない。診断用の一時テスト(コミット対象外、`git checkout --`で
完全に削除済み)も含め、実ユーザーセーブへは一切アクセスしていない。

## 44. コミット対象

```text
src/military/movement.rs   (process_movementのDivisionId昇順ソート追加、
                             単体テスト2件追加)
src/war/frontline.rs       (process_daily_frontline_plansのFrontlineId
                             昇順ソート追加)
tests/p26_006_rev_001_ai_save_resume_determinism_e2e_test.rs
                            (新規、E2Eテスト9件)
verification_logs/phase-26/p26-006-rev-001/
p26-006-rev-001_ai_save_resume_determinism.md
                            (本報告書)
```

`saves/savegame_v1.ron`はコミット対象に含めない。

## 45. 最終`git status --short`

```text
 M saves/savegame_v1.ron
 M src/military/movement.rs
 M src/war/frontline.rs
?? tests/p26_006_rev_001_ai_save_resume_determinism_e2e_test.rs
?? verification_logs/phase-26/p26-006-rev-001/
```

## 46. P26-007へ進める状態か

進める状態である。`p23_010`の間欠失敗は、独立プロセス20回(2バッチ)で
安定して解消したことを確認した。真の根本原因(`process_movement`/
`process_daily_frontline_plans`のHashMap反復順非決定性)を特定・修正し、
再発防止テストを固定した。当初仕様が想定した`CountryAiState.dirty`の
Save永続化は不要であることを実測で確定し、Save V1・P26-004〜006のいずれも
変更せずに済んだ(スコープを不必要に拡大しなかった)。§34で記録した
AI評価staggeringの実質無効化は、本タスクの対象外として修正せず、将来の
最適化候補として記録するに留めた。
