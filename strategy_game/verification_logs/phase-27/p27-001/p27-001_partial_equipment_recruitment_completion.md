# P27-001: Equipment不足時の募兵完了と既存補充Systemへの引き渡し

本報告書は、P27-001-PRE-001が確定した設計(候補C: 利用可能な範囲でだけ支給して
Divisionを完成させる)を実装し、募兵完了(人的資源の訓練完了)とEquipment充足
(装備品の物理的支給)を分離した上で、不足装備Divisionとして正しく完成させ、
既存の日次装備補充・戦闘力計算Systemへ過不足なく引き渡すことを実証するものである。

## 1. 総合判定

```
P27-001 COMPLETE
PARTIAL EQUIPMENT COMPLETION VALID
ZERO EQUIPMENT COMPLETION VALID
MANPOWER ACCOUNTING PRESERVED
EQUIPMENT ACCOUNTING PRESERVED
REINFORCEMENT HANDOFF VALID
SAVE CONTINUITY PRESERVED
SAVE V1 PRESERVED
PHASE 26 REGRESSION FREE
NEXT ATOMIC TASK READY
```

## 2. 開始HEAD

`49167956c7155cf7534f4d54fe4fdc102563c280`(P27-001-PRE-001の設計監査コミット)。
開始時`git status --short`はクリーン、`cargo test --all-targets`は2506 passed/0
failed/1 ignored、clippyは0警告——いずれも実測して確認した。

## 3. 修正前の再現結果

`src/military/recruitment.rs::try_spawn_division`(修正前)を確認し、
`try_issue_equipment`が常に`def.required_equipment`(必要量の全額)を`requested`
として要求すること、この関数がAll-or-Nothingであること(全額支給できなければ
`Err(InsufficientStockpile)`を返し、備蓄・Division equipmentのいずれも変更しない
こと)を確認した。これにより、備蓄が必要量に1単位でも届かない限り、
`process_recruitment`は該当queue項目を完了させず、`days_remaining=0`のまま
無期限キューに残す。既存テスト`test_recruitment_equipment_shortage_delays_
division_spawn_until_stockpile_is_sufficient`(`src/military/tests.rs`)・
`test_c_insufficient_equipment_delays_completion_without_altering_manpower_
treasury_or_stockpile`(`tests/p25_003_...`)・`blocked_completed_queue_protects_
stockpile_and_resolves_the_next_day`(`tests/p25_005_...`)がこの旧挙動を明示的に
固定していたことを確認した(§36で更新内容を報告)。

## 4. 根本原因

P27-001-PRE-001が確定した通り: (1)`request_recruitment`はキュー投入時に
Equipmentを一切確認・予約しない、(2)`try_spawn_division`が完成時に必要量の
全額を一括要求するAll-or-Nothing実装である、(3)`src/military/ai.rs`の
`decide_and_recruit_for_country`が`recruitment_queue.is_empty()`チェックを他の
全条件より最優先で行うため、1件の未完了項目が国家単位でAI募兵を無期限停止させる。
この3点の組み合わせが原因であり、いずれか1つを解消するだけでは不十分だが、
(2)の支給ロジックを「利用可能な範囲での部分支給」へ変更するだけで(1)(3)の
影響が実質的に解消されることを確認した。

## 5. 採用方式

P27-001-PRE-001の候補C。`try_spawn_division`が`try_issue_equipment`へ渡す
`requested`引数を、`def.required_equipment`固定から`def.required_equipment.
min(現在のMilitaryEquipment備蓄)`へ変更した。新しい予約・timeout・最低装備率・
AI/プレイヤー別条件のいずれも導入していない。

## 6. 変更ファイル

* `src/military/recruitment.rs`(本体修正、+ドキュメントコメント更新)
* `src/military/tests.rs`(既存の1テストを新挙動へ更新)
* `tests/p25_003_recruitment_equipment_issue_e2e_test.rs`(既存6テストを新挙動へ更新)
* `tests/p25_005_daily_equipment_reinforcement_e2e_test.rs`(既存1テストを新挙動へ更新)
* `tests/p26_004_daily_supply_consumption_allocation_e2e_test.rs`(既存1テストの
  制御方法を明示化、§35参照)
* `tests/p27_001_partial_equipment_recruitment_completion_e2e_test.rs`(新規、9テスト)

他のファイル(`equipment_reinforcement.rs`・`combat_calc.rs`・`daily_supply.rs`・
`ai.rs`・`recruitment_supply_gate.rs`・Save関連ファイル)はいずれも無変更。

## 7. `try_spawn_division`の変更内容

```rust
// 変更前
let result = try_issue_equipment(
    stockpile, &mut equipment, def.required_equipment, def.required_equipment,
)?;
// ...
equipment: result.equipment_after,
max_equipment: result.equipment_after,   // 全額支給が前提のため常にrequired_equipmentと一致していた

// 変更後
let available = stockpile.get(ResourceType::MilitaryEquipment);
let issued_request = def.required_equipment.min(available);
let result = try_issue_equipment(
    stockpile, &mut equipment, def.required_equipment, issued_request,
)?;
// ...
equipment: result.equipment_after,        // 実際に支給できた量(0の場合を含む)
max_equipment: def.required_equipment,    // 常に満編制上限のまま(支給量に連動させない)
```

`max_equipment`を`result.equipment_after`ではなく`def.required_equipment`へ
明示的に固定した点が、実装中に発見した必須の区別である(§16参照)——`result.
equipment_after`をそのまま使うと、部分支給されたDivisionの`max_equipment`が
支給量に永久に固定されてしまい、後日の日次装備補充が「不足0」と誤判定して
二度と満編制まで補充されなくなる。

## 8. Equipment支給式

```text
issued_units = min(required_equipment_units, current_stockpile_units)
```

`try_issue_equipment`自体(`src/military/equipment.rs`)は無変更——全ての
units変換・validation・checked演算・往復不変条件確認は既存の正規APIをそのまま
再利用している。

## 9. 100%/99%/50%/1 unit/0%の結果

`tests/p27_001_partial_equipment_recruitment_completion_e2e_test.rs::
test_b_equipment_boundary_percentages_complete_via_real_app`で実App経由(実際の
`handle_daily_military`System実行)で実測した。

| 備蓄割合 | 結果 |
|---|---|
| 100%(満額) | 訓練完了と同時に満編制で完成、`max_equipment`と一致 |
| 99% | 訓練完了と同時に99%分で完成、`max_equipment`は満編制のまま |
| 50% | 訓練完了と同時に50%分で完成、`max_equipment`は満編制のまま |
| 1 canonical unit(0.000001) | 訓練完了と同時にその1単位だけで完成(0でも満額でもない) |
| 0%(備蓄0) | 訓練完了と同時にequipment=0で完成 |

全ケースで完成が遅延しないこと、`max_equipment`が常に満編制上限のままである
ことを確認した。

## 10. Equipment保存則

`src/military/equipment.rs`の`try_issue_equipment`が持つ`debug_assert_eq!`
(支給前後の国家備蓄+Division装備量の合計が不変)がそのまま適用される。加えて、
`tests/p25_003_...`の既存テスト(`test_g_fractional_required_equipment_is_
issued_exactly_without_drift`・`test_h_off_grid_required_equipment_uses_one_
consistent_canonical_value`等)が固定小数点units上での無ドリフトを既に検証
しており、いずれも無変更で成功する。新しいEquipment保存則の破壊は確認されな
かった。

## 11. manpower保存則

`compute_manpower_ledger`(`src/military/manpower.rs`)は無変更。queue内予約分
(`queue_living`)を`living_military_manpower`へ含める既存ロジックが、完成の
遅延・即時完成いずれの場合も同じ式で正しく計上されることを確認した(P27-001
実装によりmanpowerの予約タイミング・返却タイミングはどちらも一切変更していない)。

## 12. 装備不足Divisionの対応能力(再確認)

`equipment_reinforcement.rs`(P25-005、無変更)が`equipment < max_equipment`の
Divisionを日次で正式に扱う既存メカニズムであることを、`test_c_completed_
division_is_topped_up_by_the_existing_daily_reinforcement_system`で実App経由
で再確認した——0装備で完成した翌日に備蓄を供給すると、既存Systemが自動的に
満編制まで補充する(新しいコード追加は一切不要)。

## 13. 装備補充Systemとの関係

`src/military/update.rs::handle_daily_military`のSystem順序(1. 募兵完成
→2. manpower補充→2.5 equipment補充→3〜. 移動・戦闘)は無変更のまま。実装後も
募兵完成がその日の備蓄を最優先で消費し、残余があれば同日中に既存Divisionへ
装備補充が回るという既存の優先順位(`tests/p25_005_...`の`completed_
recruitment_leftover_stockpile_reinforces_existing_divisions_the_same_day`・
`a_newly_completed_recruit_is_not_double_supplied_on_its_completion_day`)が
そのまま成立することを再確認した。

## 14. AIへの影響

`test_d_ai_resumes_recruitment_after_its_stuck_queue_clears`(実App、AI国家の
備蓄を意図的に0にした状態で月次AI募兵評価を発火)で、AIのqueueが即座に完成
(空に)なること、以後の月次サイクルでAIが正常に再評価できることを確認した。
`src/military/ai.rs`の`decide_and_recruit_for_country`・`recruitment_supply_
gate.rs`はいずれも無変更。

## 15. プレイヤーへの影響

`request_recruitment`・`process_recruitment`・`try_spawn_division`はAI・
プレイヤー共通の唯一の経路であり(`test_i_result_is_deterministic_regardless_
of_country_and_queue_registration_order`・`test_t_ai_recruitment_receives_
the_same_equipment_completion_rule_as_the_player`で確認)、プレイヤー操作専用の
分岐は追加していない。

## 16. 100%/99%/50%/1 unit/0%結果(まとめ)

§9のとおり。加えて、`max_equipment`を`result.equipment_after`ではなく
`def.required_equipment`へ固定する変更が無いと、部分支給されたDivisionが
以後永久に補充対象から外れてしまうことを実装中に発見し、修正した(§7参照、
テスト`test_b_equipment_boundary_percentages_complete_via_real_app`・
`test_c_completed_division_is_topped_up_by_the_existing_daily_reinforcement_
system`の両方でこの区別を明示的に検証している)。

## 17. Equipment回復後の結果

`test_c_completed_division_is_topped_up_by_the_existing_daily_reinforcement_
system`・`tests/p25_005_...`の更新済みテストで、翌日以降に備蓄が供給されると
既存の日次装備補充Systemが自動的に(新規完成Divisionと既存Divisionを区別せず)
不足分を補うことを確認した。

## 18. 同日月次生産との順序

`DailySimulationSet::Economy`(月次MilitaryEquipment生産を含む)が
`DailySimulationSet::MilitaryAction`(募兵完成)より先に実行される順序
(P26-009で確認済み、`.chain()`による明示的なSet順序制約)は無変更のまま。
既存テスト`test_l_same_day_produced_equipment_can_be_used_by_the_same_days_
recruitment`(`tests/p25_003_...`、無変更で成功)が、月次生産分が同日の募兵
完成に使用可能であることを引き続き検証している。

## 19. 同日複数queue

`test_j_multiple_queue_items_both_complete_same_day_first_full_then_
remainder`(軽量ハーネス)で、`process_recruitment`が`recruitment_queue`の
Vec順序で逐次処理し、各項目の完成時点で都度その時点の備蓄を読み直すため、
先着の項目が優先的に備蓄を消費し、後続の項目は残余だけを部分支給されて
「同日中に両方とも完成する」ことを確認した。これは新しいHamilton配分等を
導入したものではなく、既存のVec順逐次処理がそのまま自然に決定的なFIFO配分を
実現している——実装の複雑さを増やさない、意図しない良い副作用である。

## 20. queue取消結果

`cancel_recruitment`は無変更。`test_h_cancel_recruitment_still_returns_only_
manpower_not_equipment`で、Equipmentの「予約」概念自体が存在しないため
(§11)、取消時の返却対象が人的資源のみのままであることを確認した。

## 21. 国家消滅結果

`src/country/elimination.rs`は無変更。消滅処理が`recruitment_queue`を
明示的にクリアする既存契約(P27-001-PRE-001§27で確認済み)により、消滅国家の
queue項目が本実装の新しい完成規則の対象になることはない。

## 22. 現行Save往復

`test_g_double_save_load_stays_stable_for_a_zero_equipped_division`で、
0装備Divisionが2回連続のsave→loadを跨いでも重複・消失しないことを実App経由で
確認した。

## 23. 旧Save結果

`RecruitmentQueueItem`・`Division`のいずれのフィールドも変更していないため、
旧Saveの構文的・意味的互換性双方に影響なし。追加のmigration処理は不要かつ
実装していない。

## 24. 2回連続save→load

§22の`test_g_...`で確認済み。

## 25. Save V1/V2判断

**Save V1を完全に維持した。** 新規フィールド・新規DTO・新規migrationロジックの
いずれも追加していない。

## 26. 90日結果

§9の境界値検証(`test_b_...`)および§14のAI再開検証(`test_d_...`)を実App
(実際の`handle_daily_military`System)経由で実施した。合成fixtureでの
最小再現(装備0→30日後に完成)も確認した(§3参照、旧テストの更新版として
現在も`src/military/tests.rs`・`tests/p25_003_...`に存在)。

## 27. 36か月結果

`tests/p26_004_daily_supply_consumption_allocation_e2e_test.rs`の既存36か月
実App検証(6か国、現行データ)を再実行し、Arcadia(CountryId 0)のAIが実際に
募兵を判断・完成させ、36か月間でDivision数が1→2へ増加することを確認した
(§35で詳述)。これはP27-001が意図した正しい効果(AIが装備不足で永久に
足止めされなくなる)の直接的な実証である。

## 28. 6か国別結果

§27の36か月実測(修正版テスト、`test_36_month_fixed_initial_divisions_real_
app_verification`)で得られた最終状態:

| 国 | shortage_days(36か月中) | 最終MilitarySupplies | division_count |
|---|---|---|---|
| Arcadia(0) | 30 | 16.377 | 1(募兵queueを明示的に取消制御) |
| Elfin(1) | 469 | 13.420 | 1 |
| Dwarf(2) | 23 | 569.059 | 1 |
| Oceanic(3) | 493 | 23.385 | 1 |
| Passguard(4) | 104 | 2488.079 | 1 |
| Ferrowyn(5) | 23 | 171.859 | 1 |

このテストは「募兵なし」をこのテスト自身の明示的な制御(`cancel_recruitment`に
よる即時取消)として維持するよう更新したため(§35)、division_countは意図的に
1のまま固定されている——これはP27-001実装の欠陥ではなく、このテスト自身が
意図的に選んだ制御条件である。P27-001の効果そのもの(AIの自然な募兵成功)は
§27で別途確認済み。

## 29. P26-008-A保護

`src/military/recruitment_supply_gate.rs`は無変更。平時30/20/35日・戦時
60/40/70日の閾値、bridge forecast計算式、`recruitment_supply_blocked`の
Save契約のいずれも変更していない。既存の`p26_008_a_supply_aware_
recruitment_gate_e2e_test.rs`(30テスト)が無変更のまま全て成功することを
確認した。

## 30. Phase 23〜26回帰

Phase 23(`p23_001`・`p23_002`・`p23_006`・`p23_006_fix_001`・`p23_007`・
`p23_008`・`p23_009`・`p23_010`)・Phase 25(`p25_002`・`p25_002_rev_001`・
`p25_003`・`p25_004`・`p25_005`・`p25_006`・`p25_006_rev_001`・`p25_007`)・
Phase 26(`p26_004`・`p26_007`・`p26_008_a`・`p26_008_b`・`p26_008_c`)・
`land_war_combat_peace_test`を全て再実行し、§36で述べる3件の意図的な更新を
除き、無変更のまま全て成功することを確認した。

## 31. 既存テスト変更と理由

以下、「既存動作を意図的に反転する場合は、元のテストを新動作へ更新し、理由を
記録する」という方針に従って更新した。

1. `src/military/tests.rs::test_recruitment_equipment_shortage_delays_
   division_spawn_until_stockpile_is_sufficient` →
   `test_recruitment_equipment_shortage_completes_immediately_with_zero_
   equipped_division`へ改名・更新。旧テストは備蓄0の場合に完成が遅延する
   ことを固定していたが、これはP27-001が意図的に反転する契約そのものである。
2. `tests/p25_003_...::test_c_insufficient_equipment_delays_completion_
   without_altering_manpower_treasury_or_stockpile` →
   `test_c_zero_equipment_stockpile_completes_immediately_with_a_zero_
   equipped_division`へ改名・更新。同上の理由。
3. `tests/p25_003_...::test_j_multiple_queue_items_only_the_first_in_order_
   consumes_available_equipment` →
   `test_j_multiple_queue_items_both_complete_same_day_first_full_then_
   remainder`へ改名・更新。旧テストは2件目が待機し続けることを固定していたが、
   P27-001では2件目も残余で部分完成する(§19)。
4. `tests/p25_003_...::test_k_multiple_countries_do_not_share_equipment_or_
   manpower`——テスト名は維持(国家間分離という核心の主張は変わらない)、
   本文中のcountry Aの結果(旧: 待機のまま/新: 0装備で完成)を更新。
5. `tests/p25_003_...::test_p_pending_equipment_shortage_queue_survives_
   save_load_and_completes_once_equipped` →
   `test_p_zero_equipment_division_completes_immediately_and_round_trips_
   through_save_load`へ全面刷新。旧テストの前提(備蓄0では完成しない)自体が
   成立しなくなったため。
6. `tests/p25_003_...::test_t_ai_recruitment_receives_the_same_equipment_
   gating_as_the_player` →
   `test_t_ai_recruitment_receives_the_same_equipment_completion_rule_as_
   the_player`へ改名・更新。「同じゲートを共有する(=どちらも完成しない)」
   から「同じ完成規則を共有する(=どちらも部分/0装備で完成する)」へ主張を
   反転。
7. `tests/p25_003_...::test_u_stockpile_bits_are_unchanged_across_every_
   equipment_failure_path` →
   `test_u_stockpile_changes_exactly_once_on_the_completion_day_then_stays_
   bit_stable`へ全面刷新。旧テストは40日間ずっと不変であることを検証して
   いたが、P27-001では完成日にちょうど1回だけ変化する。
8. `tests/p25_005_...::blocked_completed_queue_protects_stockpile_and_
   resolves_the_next_day` →
   `recruitment_completion_takes_priority_over_existing_division_
   reinforcement_the_same_day`へ全面刷新。旧テストは「新規募兵が完了しない
   ため既存Divisionへの補充予算を0にする保護」を検証していたが、P27-001に
   より新規募兵は常に(部分的にでも)完了するため、この具体的な「保護」機構
   は発動しなくなった——既存のSystem順序(募兵完成が装備補充より先)がその
   まま同じ優先順位を実現することを検証する内容へ更新した。
9. `tests/p26_004_...::test_36_month_fixed_initial_divisions_real_app_
   verification`——本文中の「募兵なし」制御を、旧・equipment不足バグへの
   偶然の依存から、`cancel_recruitment`による明示的な取消制御へ更新した
   (§35で詳述、本質的なテストの目的[MilitarySupplies消費だけを分離観測する]
   は維持)。

いずれも、アサーションを弱める・許容誤差を広げる・テストをignoreする形の
変更ではなく、新しい正規挙動を明示的に固定する形の更新である。改名した関数は
全てgrepで再確認済み(旧関数名の残存参照なし)。

## 32. Critical/High/Medium/Low

**Critical: 0件。High: 0件。Medium: 0件。**

**Low: 1件**(P27-001-PRE-001から継続する既知の制約、本タスクで再確認)——
本タスクの実装過程で構築した一時的な最小`setup_app`ハーネス
(`PlayerCountry(None)`、`AppPlugin`を含まない最小プラグイン構成)において、
`handle_daily_military`(および`process_recruitment`)が実行されない現象を
再現した。同じ一時ハーネスで`daily_supply::process_daily_military_supply_
consumption`(Economy Set)は正常に動作することも確認しており、`MilitaryAction`
Set固有の未解明な条件が原因と推測されるが、根本原因は特定できていない。
本タスクの本実装・恒久テストはいずれも`AppPlugin`を含む完全なプラグイン構成
かつ`PlayerCountry(Some(..))`の実証済みハーネス
(`tests/p25_003_...::setup_app_in_playing`と同一パターン)を使用しており、
この制約の影響を受けていない。将来この種の最小ハーネスを新設する場合は、
本タスクのTest A相当の事前検証(実際にSystemが起動することの直接確認)を
行うことを推奨する。

## 33. 全テストpassed/failed/ignored件数

最終確認: 2515 passed / 0 failed / 1 ignored(開始時2506から、新規E2Eファイル
`p27_001_partial_equipment_recruitment_completion_e2e_test.rs`の9テスト分が
純増、既存ファイルの改名・更新はいずれも件数を変えていない)。

## 34. clippy警告数

0件(開始時・最終確認時ともに)。

## 35. 一時的な副作用として発見した既存テストへの影響(§27・§28で先述)

`tests/p26_004_daily_supply_consumption_allocation_e2e_test.rs::test_36_
month_fixed_initial_divisions_real_app_verification`が、初回の全体テスト実行
(既定の並列度)では偶然成功したが、`--test-threads=1`での再実行時に決定的に
失敗することを発見した(スレッド数に依存する非決定性ではなく、Arcadia
[CountryId(0)]のAIが36か月のどこかで実際に募兵を判断・完成させる、
message-count駆動の純粋に決定的な結果——単に、この具体的な失敗が「たまたま
このタスクで最初に実行した既定並列実行の結果セットの中では他の失敗[p25_005側]
に隠れて見えていなかった」だけであり、個別に切り出すと毎回100%再現することを
確認済み)。このテストの「募兵なし」前提は、実際には旧・equipment不足バグに
偶然支えられていたことが判明したため、§31-9の方法で明示的な制御へ更新した。

同様の「旧equipment不足バグに暗黙に依存していた既存テスト」が他にも存在しない
か、Phase 23-26の全関連テストファイル(§30)を実行して確認したが、他に該当する
失敗は見つからなかった。

## 36. Cargo逐次実行

逐次実行(並列cargo実行なし)。

```text
開始時:
cargo test --all-targets                                      → 2506 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings       → 0 warnings

回帰テスト(military::/economy::/country::/save::の--lib、および
p23_001/002/006/006-fix-001/007/008/009/010、p25_002/002-rev-001/003/004/005/
006/006-rev-001/007、p26_004/007/008_a/008_b/008_c、land_war_combat_peace_test、
新規p27_001):
                                                                 → 複数回、最終的に全て成功

最終確認:
cargo test --all-targets                                      → 2515 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings       → 0 warnings
```

## 37. 独立プロセス結果

以下を含む組み合わせを独立した`cargo test`プロセスとして5回実行し、全て
安定して成功した(不安定・失敗は0件)。

* `p27_001_partial_equipment_recruitment_completion_e2e_test`(9テスト)
* `p25_003_recruitment_equipment_issue_e2e_test`(22テスト)
* `p25_005_daily_equipment_reinforcement_e2e_test`(10テスト)
* `p26_004_daily_supply_consumption_allocation_e2e_test`(19テスト)
* `p26_008_a_supply_aware_recruitment_gate_e2e_test`(30テスト)
* `--lib`(1617テスト)

なお、実装過程で`p25_005`側に1件、`--test-threads=1`実行時にのみ表面化した
真の非決定性(HashMap反復順序に依存する、私自身のテストコードの不備——「新規
完成Division」を`id != existing`という緩い条件で特定していたため、New Game
自体が事前にspawnしていた満編制の初期Divisionを誤って拾う場合があった)を
発見・修正した(§31-8参照、事前ID集合との差分方式へ訂正)。この修正後、
5回の独立実行を含むあらゆる組み合わせで再現しないことを確認した。

## 38. 対象外の先行実装なし

AIの目標Division数計算式・Equipment生産量・破産係数・MilitaryFactory建設AI・
財政バランス・戦闘式・UI・localization・Save V2・交易・輸送のいずれも変更して
いない。

## 39. ユーザーSave保護

`saves/savegame_v1.ron`には本タスク全体を通して一切アクセスしていない。

## 40. PNG差分処理

`cargo test --all-targets`実行のたびに、既知の9件の非決定的スクリーンショット
PNGが差分として現れたが(本タスク中に複数回発生)、いずれも個別に`git checkout
--`で復元し、広範な`checkout .`は使用していない。

## 41. コミット対象

* `src/military/recruitment.rs`
* `src/military/tests.rs`
* `tests/p25_003_recruitment_equipment_issue_e2e_test.rs`
* `tests/p25_005_daily_equipment_reinforcement_e2e_test.rs`
* `tests/p26_004_daily_supply_consumption_allocation_e2e_test.rs`
* `tests/p27_001_partial_equipment_recruitment_completion_e2e_test.rs`(新規)
* 本報告書

一時検証コード(`tests/p27_001_temp_90day_36month_verification_test.rs`)は
コミット前に削除済み。

## 42. 最終`git status --short`

```
(クリーン、上記コミット対象ファイルのみ追加予定)
```

## 43. 次の原子的タスク

Critical/High/Mediumのいずれも0件のため、P27-001-PRE-001報告書§32が示した
優先順位に従い、次の候補を提示する。

1. **財政バランス調整**(Medium、P26-009で既に記録済み): 36か月実測
   (§27・§28)で、複数国が早期に国庫赤字へ転落する傾向が引き続き観測される。
   country-specific赤字額・発生月・供給網停止との因果関係の定量化が、着手前に
   さらに必要。
2. Phase 27の新機能(交易・輸送等、ユーザー次第)。

装備キュー完成規則そのもの(本タスクの対象)は完全に解消したため、
P27-001-PRE-001が候補2として挙げていた項目は本タスクをもって完了とする。
