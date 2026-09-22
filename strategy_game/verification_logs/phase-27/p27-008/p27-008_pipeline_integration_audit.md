# P27-008 Phase 27戦闘・補給パイプライン最終統合監査

本報告書は、P27-004(移動冪等化)・P27-005(攻撃側前線補給ブリッジ)・P27-006(低実効戦力の損害連続性)・P27-007(両陣営ゼロ損害戦闘の即日終了)を、同一の戦争・移動・補給・戦闘パイプライン上で統合監査するものである。本番コード・データ・既存テストは変更していない。

## 1. 判定

```text
P27-008 AUDIT COMPLETE
P27-004→007 INTEGRATION CONFIRMED
PHASE 27 RE-CLOSED
NEXT PHASE READY
```

Critical・High findingはない。Medium findingはない。Low findingは4件(§13、うち1件を本監査で動的に再現・詳細化した)で、いずれもPhase 27をブロックしない。

## 2. 基準状態

開始時HEAD: `4dcb3d0d5a7b7fc413ba1a880c6a61b318192af9`。`git status --short`は空。開始時基準: 2685 passed / 0 failed / 1 ignored、clippy 0。監査終了時も同一HEAD・同一件数であることを確認した(§17)。

## 3. 正規パイプライン(実コード確認)

```text
移動命令(start_or_maintain_movement, src/military/movement.rs)
→ 冪等な命令維持(同一destinationならno-op、異なるならmovement_progress以外を原子的に上書き、P27-004)
→ 日次移動進捗(process_movement、MilitaryAction Set)
→ 敵州到着(process_division_arrival、src/military/invasion.rs)
→ 戦闘開始(start_battle、attacker_origins記録)
→ 攻撃側前線補給(is_frontline_supply_eligible、Economy Set、process_daily_battlesより前、P27-005)
→ readiness算出(calculate_combat_readiness_with_supply、src/military/combat_calc.rs)
→ 実効戦力算出(effective_power_with_floor、P27-006 T)
→ 日次損害(damage_from・distribute_damage、P27-006 M・C)
→ 通常決着(resolve_participant_outcome、既存)またはゼロ損害終了(P27-007、zero_damage_battles_today)
→ controller更新(occupy_state)または攻撃側撤退(force_retreat_or_destroy)
→ 戦闘後のAI stance(evaluate_military_ai_decision、P26-008-B攻勢供給ゲート)
```

`handle_daily_military`(src/military/update.rs)内の実行順序: 募兵→日次manpower補充→日次装備補充→移動の事前検証→`process_movement`→`process_daily_battles`→`resolve_finished_battles`→終了戦闘のクリーンアップ→組織率回復。日次補給(P26-004/P27-005)はこれより前のEconomy Setで確定済みの値を読む。P27-007の`zero_damage_battles_today`は`process_daily_battles`の戻り値として`resolve_finished_battles`へ直接渡される非永続のローカル値(Bevy Resourceでも永続フィールドでもない)。国家間分離は、全処理が戦闘・国家単位でループするため自明に成立する。決定性はBattleId昇順処理・DivisionId昇順のtie-break・HashMap反復順への非依存による。

## 4. 静的監査

### 4.1 移動命令

`rg`で`start_or_maintain_movement`の呼び出し元を再棚卸しした。実際の呼び出し元は3箇所: `src/map/division_selection.rs`(プレイヤー手動)、`src/war/frontline.rs`の3箇所(Defend AI、Offensive隣接攻撃、Offensive line attack)。いずれも共通APIを経由する。`movement_progress`への直接代入(`rg`で15件検出)は、いずれも既存の停止・到着・撤退・講和処理での完全リセット(`destination = None`等を同時に行う)であり、同一目的地への再発行によるサイレントリセットではないことを1件ずつ確認した。

### 4.2 攻撃側前線補給

`is_frontline_supply_eligible`(src/military/daily_supply.rs)を再読した。条件は変更されていない: 現在州が首都連結集合に含まれない、州のcontrollerが自国と異なる、進行中の戦闘で自国が攻撃側として登録されている、隣接州に自国controller・首都連結済みの州が1つ以上ある。防御側には適用されない(攻撃側専用の分岐)。

### 4.3 損害計算

`effective_power_with_floor`・`damage_from`・`distribute_damage`(src/military/combat_calc.rs)を再読した。M(分母下限1)・T(正の入力のみ実効戦力1)・C(全weight 0時だけmanpower比フォールバック)はいずれも変更されていない。

### 4.4 ゼロ損害戦闘

`process_daily_battles`・`resolve_finished_battles`(src/military/update.rs)を再読した。判定は状態変更前の`atk_results`/`def_results`から行われ(§3参照)、`resolve_finished_battles`側でDivisionの事後状態(organization等)を再判定していないことを確認した。

## 5. 制御シナリオの動的検証

自然な120か月運用では戦闘自体が発生しない(§12)ため、以下を統合成立の主証拠とする。

### シナリオA: 通常の国境越え攻撃(実測)

P26-008-Bの90日ハーネスを拡張し、実データ・実App・実AIで150日間の日次ログを取得した(一時診断、監査後に削除)。観測された実際の流れ:

```text
d1〜d4: Arcadia divisionが自国州0→州1を移動(progress 0.2刻みで単調増加、P27-004の冪等再発行が実際に効いている)
d5: 州1でIdle(1日休止、AIが翌日の命令を再評価)
d6〜d9: 州1→州3(Elfin首都)を移動(0.2刻み)
d10: 州3到着、Fighting開始(P27-005: 攻撃側ful=1.000)
d10〜d11: 日次損害が実際に発生(P27-006、manpower・organizationが実際に減少)
d12: 攻撃側が組織率0で出撃元(州1)へ通常撤退(既存のorganization経路、P27-007ではない)
d13〜d18: 州1でorganization自然回復、再度移動命令発行(P27-004の冪等再発行)
d19〜: 再交戦、最終的にArcadiaが州3を占領(controller変更)、Elfin側は退却を繰り返し消滅
d40以降: Arcadia divisionが占領地をさらに前進(P27-005のconnected集合が占領で拡張されている)
```

移動・到着・補給・損害・通常決着・controller変更・追撃のすべてが実際に連結して動作することを確認した。

### シナリオB: 回廊遮断と再接続(実測、P27-005 test_03/test_04として恒久テスト化済み)

戦闘解決から独立した`run_supply_only`(P27-007後に必要になった設計、`tests/p27_005_offensive_frontline_supply_bridge_e2e_test.rs`)で再実行し、d1〜d19満額、**d20〜d44のちょうど25日間だけ補給0**、d45以降自然回復を再確認した。P27-007後もこの契約は独立して成立する(戦闘解決を呼ばないため、P27-007の影響を受けない設計)。

### シナリオC: 両陣営ゼロ損害(実測、P27-007の恒久テストとして確認済み)

`tests/p27_007_zero_damage_battle_termination_e2e_test.rs`のtest_01を再実行し、実AIを含む90日間で終了が1回だけ、以後再攻撃が発生しないことを確認した。防御側は不正に撤退せず、10%ペナルティが既存どおり適用される。

### シナリオD: 退路なし(実測、P27-006 test_12として確認済み)

出撃元が`attacker_origins`に記録されていない場合、対象攻撃Divisionだけが既存の消滅処理で除去され、防御側・第三国へ波及しないことを確認した(再実行、§17)。

### シナリオE: 通常相討ち(実測、P27-006 test_01として確認済み)

両陣営が同日に正の損害を受け、双方のorganizationが0へ到達するケースを再実行した。ゼロ損害戦闘と誤判定されず、既存のorganization撤退・消滅契約が適用されることを確認した(実装中にこの誤判定を実際に1件踏んで特定・修正した経緯はP27-007報告書に記載済み)。

### シナリオF: 一方だけ真の0(実測、P27-006 test_02・05・06として確認済み)

攻撃側supply 0、防御側supply 0、防御側Equipment 0の各ケースを再実行し、ゼロ損害終了が誤発火しないこと、0側が無敵にならないこと、有限日数で状態が変化することを確認した。

## 6. 日次時系列

シナリオAの一時診断ログ(§5)で、command発行後・移動処理後・補給配分後・戦闘計算後・戦闘終了処理後の状態を日次で記録した。`status`・`current_state`・`destination`・`movement_progress`・`supply_fulfillment`・`manpower`・`organization`・`equipment`・`controller`が期待どおり遷移することを確認した(監査後に一時診断は削除、恒久化していない)。

## 7. save/load統合監査

一時診断(監査後削除)により、実際のシナリオAのタイムライン上で次の分岐点を検証した。

| 分岐点 | 結果 |
|---|---|
| d3(移動途中) | 連続実行と完全一致 |
| d9(敵州到着直前) | 一致 |
| d10(戦闘開始直後) | 一致 |
| d11(通常戦闘継続中) | 一致 |
| d20(通常決着後の追撃中) | 一致 |
| d10で2回連続save→load | 一致 |

比較項目はDivision存在・status・current_state・destination・movement_progress・manpower・organization・equipment・supply_fulfillment・controllerの全てで、いずれも完全一致した。ゼロ損害終了・回廊遮断中・再接続後の分岐は、P27-005・P27-006・P27-007それぞれの既存恒久テスト(test_10、test_07/08)で個別に確認済みであり、本監査でも再実行して通過を確認した(§17)。Save V1を維持し、新規永続フィールドがないことを`rg`で再確認した(§4.4、`attacker_origins`・`Battle`・`Division`のフィールド一覧に変更なし)。

## 8. AI・プレイヤー・消滅国家

- **AI**: `src/war/frontline.rs`の3箇所がいずれも共通の`start_or_maintain_movement`を使用する(§4.1)。P26-008-Bの攻勢供給ゲート(ヒステリシス)は無変更で、ゼロ損害終了後の再攻撃を新規cooldownなしに抑制することをシナリオCで再確認した。
- **プレイヤー**: `src/map/division_selection.rs`も同じ共通APIを使用する。前線補給資格判定(`is_frontline_supply_eligible`)はDivisionの`owner`だけを見ており、プレイヤー専用の例外分岐は存在しない(コード上確認)。
- **消滅国家**: `daily_supply.rs`の日次補給ループは`country.is_active()`を確認してから対象に含める(既存契約、無変更)。`apply_daily_reinforcement`・`apply_daily_equipment_reinforcement`も同様。撤退・消滅処理(`force_retreat_or_destroy`)はDivisionを`remove_division`するだけで、国家の生死とは独立している。

## 9. 決定性・国家間分離

独立プロセス5回(P27-005・P27-006・P27-007・P26-008-B・P27-004・land war)、全回で件数・結果が一致した(§17)。P27-006 test_13(登録順反転)、シナリオAの一時診断での同一プロセス内5回実行でも完全一致を確認した。第三国(Passguard)への非影響は`tests/p27_007_...`のtest_02で確認済み。

## 10. 長期回帰

P27-002-A/B/Cの36か月・120か月実App検証を、本監査でも独立に再実行し、全件通過(§17)を確認した。自然な通常AI運用では、P27-004〜007のいずれの修正経路も**発火しない**(先行タスクで一時計測により0回を確認済みであり、本番コードに変更がないため本監査でも同一結果と判断する)。この事実は制御シナリオ(§5)と明確に区別して報告する。制御シナリオが統合成立の主証拠であり、自然発火0件はそれを裏付ける追加情報に過ぎない。

## 11. Low findingの扱い

### 1. 撤退先の安全性チェック不足(詳細化)

**静的**: `force_retreat_or_destroy`(旧`resolve_participant_outcome`)は、`attacker_origins`に記録された州の現在のcontrollerを検証せずに`current_state`を書き換える。この経路はP21-siege期から存在し、P27-004〜007はいずれもこれを変更していない。

**動的(本監査で新たに再現)**: 一時診断で、攻撃側の出撃元(州1)を戦闘継続中に敵(Elfin)支配へ変更したうえで両陣営ゼロ損害を発生させたところ、攻撃側は実際に**敵支配下の州1でIdle状態**になった(`current_state==StateId(1)`かつ`controller==ELFIN`)。クラッシュ・Division複製・controller不整合は発生しないが、この師団は敵領土上で戦闘もされず放置される。

**到達可能性**: 現行の実データ・AI経路では、120か月の自然運用で戦闘自体が0件のため到達しない。ただし、P27-005の回廊遮断(出撃元の途中奪還)とP27-007のゼロ損害終了が同時に起きる状況では、原理的に到達し得ることを本監査で直接構成して確認した。

**重要度判定**: Divisionの重複・controller破壊・クラッシュは発生しないため、**Lowのまま維持する**(既存分類を変更しない)。ただし「原理的に到達可能」であることが未確認から確認済みへ変わったため、次の独立監査候補として明記する: `find_retreat_destination`(防御側が既に使っている隣接自国州探索)を攻撃側の退却先にも適用するかどうかの設計判断。

### 2〜4. その他のLow finding

ゼロ損害終了のプレイヤー向けUI通知不足、P27-005手動プレイ確認未実施、`daily_supply.rs`の既存rustfmt由来差分は、いずれも先行報告書の記載から変化なし。本タスクでは修正していない。

## 12. Critical/High/Medium/Low

- Critical: 0
- High: 0
- Medium: 0
- Low: 4(§11、うち1件を動的に再現・詳細化)

## 13. Phase 27の状態

P27-004(移動冪等化)・P27-005(前線補給ブリッジ)・P27-006(損害連続性)・P27-007(ゼロ損害戦闘終了)が、単一の戦争・移動・補給・戦闘パイプライン上で相互に矛盾なく連結して動作することを、静的監査(§4)と6種類の制御シナリオ(§5)・save/load統合(§7)・決定性(§9)・長期回帰(§10)で確認した。新規のCritical・High・Medium findingはない。**Phase 27をこの状態(HEAD `4dcb3d0`)で再び基準状態として固定できると判定する。**

## 14. 次のフェーズ

Phase 27の核となる戦闘・補給パイプライン(P27-004〜008)はこれで一巡した。§11のLow finding 1件(攻撃側撤退先の安全性)は、次の独立監査候補として記録するが、Phase 27をブロックしない。

## 15. 実行コマンドと正確な結果

- 開始時: `cargo test --all-targets -j 2`: 2685 passed / 0 failed / 1 ignored
- 静的監査: `rg -n "start_or_maintain_movement"`、`rg -n "movement_progress\s*="`、`rg -n "attacker_origins"`(§4)
- 制御シナリオ(一時診断、監査後削除): シナリオA(150日日次ログ)、save/load分岐(5点+2回連続)、決定性(同一プロセス5回)、撤退先安全性の動的再現
- `cargo clippy --all-targets --all-features -- -D warnings`: 警告0(監査終了時)

## 16. 独立プロセス5回再実行(§9・§17)

`p27_005`・`p27_006`・`p27_007`・`p26_008_b`・`p27_004`・`land_war_combat_peace_test`を各5回、独立プロセスで実行し、全回で件数一致を確認した。

## 17. 最終確認

一時診断コードはすべて`git checkout --`で個別に復元し、恒久化していない(`TMP008`・`TMPPATCH`等の識別子0件を確認)。最終`cargo test --all-targets -j 2`: **2685 passed / 0 failed / 1 ignored**(開始時と同一)。`cargo clippy --all-targets --all-features -- -D warnings`: 警告0。`saves/savegame_v1.ron`は読み書き・復元・ステージのいずれも行っていない。既知の9件のPNGは個別に復元した。コミット対象は本報告書1件のみ。
