# P27-007-PRE-001 両陣営が真の実効戦力0となった戦闘の終了・撤退契約設計

本報告書はP27-006のM・T・Cを保護したまま、
両陣営が真にdamageを生成できない戦闘について、
無期限膠着を終了させる契約、攻撃側の退却先、
退路なし時の扱い、およびP27-005回廊テストの
M-2依存解消を設計するものである。

## 1. 総合判定

```text
P27-007-PRE-001 ANALYSIS COMPLETE
REPRODUCTION CONFIRMED
RECOVERY PATHS CONFIRMED
RETREAT DESTINATION DESIGN DECIDED
NO-RETREAT-PATH DESIGN DECIDED
ZERO-POWER BATTLE TERMINATION DESIGN DECIDED
P27-005 TEST REDESIGN DECIDED
P27-007 IMPLEMENTATION READY
```

## 2. 再現判定

CONFIRMED。P27-006で追加した`test_08_both_sides_truly_zero_keeps_the_battle_without_fictional_damage`が、両陣営の補給0(readiness 0、organization・manpowerは健全)で40日間戦闘が変化しないことを既に固定している。本監査ではこれに加えて、一時パッチにより実際にこの状態を検出・終了させられることを動的に確認した(§19以降)。

## 3. 回復経路判定

CONFIRMED。§7で動的監査した。

## 4. 退却先設計判定

DECIDED。候補R1(既存`attacker_origins`をそのまま使う)を採用(§10、§27、§29)。

## 5. 退路なし設計判定

DECIDED。既存の`resolve_participant_outcome`の消滅分岐をそのまま再利用する(§15、§31)。

## 6. 戦闘終了設計判定

DECIDED。候補1(両陣営0を検出した当日に防御側勝利・攻撃側撤退)を採用(§11、§35、§36)。

## 7. P27-005テスト再設計判定

DECIDED。テスト案C(責務分割: 補給専用テスト+実戦闘テスト)を採用し、実際に代替実装を試作して成立を確認した(§18、§42、§43)。

## 8. P27-007準備判定

READY。範囲は`src/military/update.rs`(`process_daily_battles`の戻り値追加、`resolve_finished_battles`への引数追加、新規判定・強制撤退関数)のみ。新規Saveフィールドなし。

## 9. 開始時HEAD

`cc32014fbacb87432fe1b1151ecf2d50e2a0102b`。`git status --short`は空(9枚の既知PNGのみ、監査後に個別復元)。

## 10. 監査対象HEAD

同上。

## 11. P27-005手動確認未実施

自動検証のみ。実プレイでの手動確認は本監査でも実施していない。

## 12. 両陣営0の正規判定

**採用: 条件B相当を、既に計算済みの当日damage結果から導出する。**

新規の実効戦力再計算関数を作らず、`resolve_combat_day_multi_with_supply`が既に返す`atk_results`/`def_results`(`HashMap<DivisionId,(manpower_loss,org_loss)>`)を使い、

```text
atk_took_damage = atk_results内のいずれかの manpower_loss > 0
def_took_damage = def_results内のいずれかの manpower_loss > 0
stalled_today   = !atk_took_damage && !def_took_damage
```

と定義する。`damage_from(effective, 1)`の分子が0のとき分母に関わらず結果は常に0であるP27-006の性質により、`def_took_damage==false`は「攻撃側合計実効戦力が0」、`atk_took_damage==false`は「防御側合計実効戦力が0」と数学的に同値であり、条件A(陣営合計effective power)と完全に一致する。条件Cとは、基礎power=0のケース(実運用データには存在しない)でのみ乖離しうるが、その場合もeffective power=0に帰着するため実質的に同じ結果になる。

**この定義は`process_daily_battles`の中で、状態を一切変更する前に確定する値だけを使う**。`resolve_finished_battles`側でDivisionの現在状態(特にorganization)から事後的に再判定すると、「今日の戦闘で両者の組織率が同時に0まで削られた」正常な相討ちを誤って両陣営0と混同する(§19.1で実際に再現・特定した実装上の罠)。これは正しい実装のための必須知見であり、報告書としてもっとも重要な発見の一つである。

## 13. 条件A〜D比較

| 条件 | 内容 | 判定 |
|---|---|---|
| A: 陣営合計effective power | `atk_total<=0 && def_total<=0` | 採用と同値 |
| B: outgoing damage | `atk_total_loss==0 && def_total_loss==0` | **採用**(§12) |
| C: 全参加者readiness==0 | | Aとほぼ同値。base power=0の人工データでのみ乖離するが実データには存在しない |
| D: 継戦能力(将来予測込み) | | 不採用。将来の国家備蓄生産や首都奪還を予測する必要があり、判定が過度に複雑になる(§7の回復可能性監査で代替) |

## 14. supply 0

一方または双方の`supply_fulfillment==0`は、他の要因(manpower・equipment・organization)が健全でも`combat_readiness_units==0`となり、readiness経由でeffective power 0になる。P27-005前線補給圏外・回廊遮断・国家備蓄枯渇のいずれでも到達する。**Fighting中でも毎日再計算されるため、一時的な原因なら翌日以降に自然回復し得る**(§7)。

## 15. Equipment 0

`max_equipment>0`の師団でEquipment 0はreadiness 0を意味する。**Fighting中は装備補充の対象外**(`is_eligible_for_daily_reinforcement_by_position`がstatus==Idle/Occupyingのみを対象とし、`combat_id.is_some()`または戦闘参加中のDivisionを除外することを確認した、§7.2)。戦闘を続ける限り絶対に回復しない。

## 16. manpower 0

M-2の対象(まだ参加者リストに残っているDivision)はmanpower>0で構成される。manpower==0のDivisionは既に`resolve_participant_outcome`の消滅分岐で毎日除去されるため、M-2の「両陣営0」状態には現れない。

## 17. organization 0

**現行契約により、organization<=0のDivisionはM-2として残留しない。** `resolve_finished_battles`は`battle.attacker_division_ids`/`defender_division_ids`の全員に対して**毎日必ず**`resolve_participant_outcome`を呼ぶ(その日に損害があったかどうかに関わらず)。この関数は`organization>0.0`なら継続、そうでなければ即座に撤退または消滅させる。したがって、organization 0はP27-006以前から既に自己解消しており、**M-2の実際の原因にはなり得ない**(§17の動的監査で確認)。M-2で観測される師団は必ずorganization>0を維持したまま、supply/equipment/base powerの理由でoutgoing damageが0になっている。

## 18. base power 0

実データの標準歩兵は`attack_power=10`,`defense_power=15`で常に正。base power 0はテストfixtureのみで到達する人工的な状態であり、回復経路は存在しない(定義に固定された値のため)。

## 19. 一時的0

supply 0は「一時的0」の典型。P26-004が日次で再計算するため、国家備蓄回復・回廊再接続・首都奪還の翌日には即座に反映される(§7)。

## 20. 永久的0

Equipment 0、organization<=0(ただし§17のとおりこれは自己解消する)、base power 0(実データ非該当)は「戦闘を続ける限り絶対に回復しない」永久的0である。

## 21. Fighting中の回復

| 要素 | Fighting中に回復するか |
|---|---|
| supply_fulfillment | **する**(P26-004は全生存Divisionを毎日再計算、statusで除外しない) |
| Equipment | **しない**(`is_eligible_for_daily_reinforcement_by_position`がFighting除外) |
| manpower | **しない**(同上の関数を再利用) |
| organization | 0になった時点で即座に`resolve_participant_outcome`が処理する(§17)ため「回復を待つ」状態自体が存在しない |
| base power | 定義固定、変化しない |

## 22. 現行戦闘終了経路

`resolve_finished_battles`(`src/military/update.rs`)が、`battle.attacker_division_ids`/`defender_division_ids`の全員へ`resolve_participant_outcome`(manpower==0→消滅、organization<=0→撤退または消滅、それ以外→継続)を毎日適用し、結果として空になった陣営で勝敗を決める: 防御側全滅→攻撃側勝利+`occupy_state`、攻撃側全滅→防御側勝利(支配権不変)、両者全滅→防御側勝利(支配権不変)。**時間切れ規則・実効戦力0による終了規則は元々存在しない**。

## 23. 防御側勝利経路

再利用可能。既存の「攻撃側全滅→防御側勝利」分岐と全く同じ`BattleStatus::DefenderWon`・支配権不変・防御側`status=Idle,combat_id=None`を、M-2判定時にも適用する。攻撃側は消滅ではなく撤退させる点だけが異なる。

## 24. controller

M-2終了では変更しない(防御側勝利と同じ扱い)。

## 25. status

攻撃側: 撤退成功なら`Idle`(既存の撤退分岐と同じフィールドリセット一式)。撤退不能なら消滅。防御側: `Idle`、`combat_id=None`(既存の防御側勝利分岐と同じ)。

## 26. 出撃元保持

CONFIRMED。`Battle.attacker_origins: HashMap<DivisionId, StateId>`は既存フィールドで、戦闘開始時(および合流時)に個別記録され、戦闘中ずっと保持される。`resolve_finished_battles`の既存の攻撃側撤退分岐が既にこれを退却先として使っている。**新規フィールド不要**。

## 27. 退却先候補

**採用: 候補R1(既存の`attacker_origins`をそのまま使う)。**

理由: 既存の組織率0撤退が既にこの経路を使っており、動作実績がある。候補R2〜R4(隣接自国州探索、P27-005補給元条件、BFS最寄り州)は、出撃元が既に敵に占領された場合により安全な代替先になり得るが、**この問題は候補1適用の有無に関わらず既存のorganization撤退経路に既にある(§28)ため、本タスクの新規スコープではない**。R1を再利用するだけで、既存契約からの逸脱・新規リスクを増やさない。

## 28. 複数退却先

R1は師団ごとに個別の出撃元を持つため、tie-break不要(各師団が自分の記録した1点へ戻る)。

## 29. 退路なし

`attacker_origins.get(&division_id)`が`None`を返す(データ上ありえないはずだが防御的に扱う)場合、または出撃元の残存manpowerが0になる場合は、**既存の消滅分岐と全く同じ処理**(現在装備を全損させてから`remove_division`)を再利用する。新しい降伏・捕虜システムは導入しない。

## 30. 完全包囲

攻撃側が出撃元へ戻れない状況(出撃元が戦闘中である、等の既存の`find_retreat_destination`的な追加検証)は、本タスクでは導入しない。既存の攻撃側撤退が出撃元の現状を検証せずに使っている(§9の「既存の一次ギャップ」)のと同じ挙動を維持し、新しい検証を追加しない(スコープ外。追加するなら別タスク)。攻撃側自身が完全に包囲される(出撃元自体が奪われる)ケースは、既存のorganization撤退でも同じ扱いであり、候補1導入によって新たに悪化するものではない。

## 31. AI再攻撃

一時パッチ(§34)を実際のAI駆動App(`setup_app(None, ...)`、`process_daily_military_ai`が動く構成)へ適用し、90日間観測した。d17付近で両国のMilitarySupplies備蓄を同時に0へ落として両陣営0を発生させ、d47(30日後)に備蓄を回復させた。結果:

```text
battle_terminations = 1 (90日間で1回のみ)
90日間、以後は再戦闘が一切発生しない
最終stance = Defend、offensive_supply_blocked = Some(true)
```

**新しいcooldownは不要**。P26-008-Bで既に実装済みの攻勢供給ゲート(国家備蓄・前線配送率のヒステリシス)が、備蓄回復後も新規攻勢の再開条件(0.34以上の配送率、resume閾値以上の備蓄が一定期間続く)を満たすまでstance=Defendを維持し続けるため、結果として再攻撃ループが自然に防止される。

## 32. プレイヤー攻撃

候補1の判定・処理はDivisionの`owner`やプレイヤー/AIの区別を一切参照しない(純粋に当日のdamage結果だけを見る)ため、プレイヤーの攻撃にも自動的に同じ契約が適用される。UI通知は本タスクでは実装しないが、プレイヤーが「なぜ急に撤退したか分からない」となる可能性はLow findingとして記録する(§52)。

## 33. 複数Division

一時パッチによる単体テストで検証した(§20)。

- 攻撃側複数、一部だけ正の実効戦力を持つ: 防御側は正しく損害を受け、候補1は誤発火しない。
- 防御側複数、一部だけ正の実効戦力を持つ: 攻撃側は正しく損害を受け、候補1は誤発火しない。
- 双方とも「1体でも正の実効戦力を持てば」候補1は発火しない(§12の定義どおり)。

## 34. System順序

`handle_daily_military`内で`process_daily_battles`と`resolve_finished_battles`は同じ関数(同じシステム呼び出し)内で順に呼ばれる**通常のRust関数呼び出し**であり、別のBevy Systemではない。したがって当日の判定結果を`HashSet<BattleId>`として直接引数渡しでき、新しいResourceや永続フィールドを介さずに同日中に消費・破棄できる。

1. 両陣営0の判定日 = その日の`process_daily_battles`実行時
2. 同日に戦闘終了: する(`resolve_finished_battles`は同じ呼び出しの直後)
3. 同日に撤退移動を開始: する(`current_state`を出撃元へ即座に書き換える、既存撤退と同じ)
4. 翌日開始: しない(即日)
5. 退却中に参加者として残るか: 残らない(即座に`Idle`化し`combat_id=None`)
6. 補給需要: 翌日から通常のIdle Divisionとして評価される(P27-005前線補給の対象から外れる)
7. controller確定: 即日(変更なし、防御側勝利と同時)
8. AI再命令可能: 翌日のFrontline評価から(既存のcadenceに従う、§31)

## 35. 候補0〜7

| 候補 | 判定 | 理由 |
|---|---|---|
| 0 無期限回復待ち | 不採用 | Equipment 0・base power 0は自己回復しない(§21)ため恒久膠着になり得る |
| **1 即日防御側勝利・攻撃側撤退** | **採用** | 新規状態不要、既存経路を100%再利用、AI再攻撃ループは既存ゲートで自然抑制(§31) |
| 2 猶予期間後に終了 | 不採用 | 新規カウンタ・Save影響が必要。§31の実測で猶予がなくても再攻撃ループが起きないことを確認したため、複雑化を正当化する根拠がない |
| 3 回復可能性判定 | 不採用 | 将来の国家備蓄生産・首都奪還を予測する必要があり過度に複雑 |
| 4 organization 0を撤退条件に | 不採用(既に部分的に成立) | §17のとおり、organization 0は既に別経路で処理済みで、M-2の直接原因にならない。M-2の原因はsupply/equipment/base powerであり、organizationを拡張しても解決しない |
| 5 最低戦闘力を与える | 不採用 | P27-006の「真の0保護」に反する |
| 6 その場で戦闘解除 | 不採用 | 敵州に攻撃側が残留し翌日再戦する矛盾を生む |
| 7 攻撃側だけ戦闘解除+移動命令再発行 | 不採用(候補1で代替) | 既存の`resolve_participant_outcome`の撤退分岐で同等以上に単純に実現できる。新たに`start_or_maintain_movement`を呼ぶ必要がない |

## 36. 採用候補

**候補1**。即時判定・即時終了。猶予期間は設けない。

## 37. 90日結果

§31のとおり。両陣営0発生から即日で終了、攻撃側は出撃元(隣接する自国州)へ撤退、以後90日間再戦闘なし。

## 38. 36か月結果

P27-002-A/B/Cの36か月実App検証は、候補1適用後も全件通過。一時的な発火計測で、自然な通常AI運用ではM-2判定(候補1のトリガ)は**0回発火**(既存の「自然プレイでは戦闘が0件」という監査結果と整合)。

## 39. 120か月結果

同上(P27-002-C、90/36/60/120か月、16件)、候補1適用後も全件通過、発火0件。

## 40. P27-005 test_03依存

現行(P27-006時点)の`test_03_corridor_cut_stops_bridge_supply_for_exactly_the_cut_days_then_recovers`は、`Cfg::neutral_power`(攻撃側攻撃力0・防御側防御力0)で両陣営を恒久的にoutgoing damage 0にし、25日間の戦闘継続を成立させている。候補1を実装すると、このfixtureは**開始直後(d11)に即座に終了**し、25日間の交戦を維持できない(§41で実測)。

## 41. P27-005 test_04依存

同様に`test_04_capital_fall_keeps_the_siege_supply_at_zero`も`neutral_power`に依存しており、候補1適用後は即日終了する(実測: 一時パッチ適用状態で`cargo test --all-targets`を実行し、test_03・test_04のみが新たに失敗することを確認、§45)。

## 42. 代替テスト案A〜D

| 案 | 内容 | 判定 |
|---|---|---|
| A: 正の戦力で25日以上継続 | 実際の1v1・1v4戦闘で25日以上生き残るfixtureを模索 | 不採用。P27-006後の損害式では、健全な戦力同士の戦闘は数日で決着する(P27-006報告書のシミュレーション結果)。よほど戦力を拮抗させても、乱数がない決定的計算のため「ちょうど25日ぎりぎり続く」ような精密な調整は本番バランス変更に等しくなり、堅牢性がない |
| **B: 補給Systemだけを日次実行** | 実際のActive Battle fixtureを保持しつつ、`run_daily_military_supply_consumption_with_battles`だけを直接80日ループで呼び、戦闘解決Systemを一切実行しない | **採用(一部)**。一時実装で実測し、d20〜d44の**ちょうど25日間**が補給0になることを確認した(§43)。P27-005の補給契約(1ホップ・回廊遮断・再接続)だけを、戦闘終了ポリシーと完全に独立して検証できる |
| **C: 責務を2テストへ分割** | (1)補給専用テスト(案B)で25日遮断を検証、(2)実戦闘テストで終了・撤退を検証 | **採用**。第一候補として実装可能性を確認済み |
| D: 25日を短縮 | 不採用(原則どおり) | P27-005の契約を弱めるため採用しない |

## 43. 採用するテスト再設計

**テスト案C**(補給専用テスト+実戦闘テストへの分割)を採用する。

- **新設: 補給専用テスト**(`daily_supply.rs`の単体テスト、または新規の軽量統合テスト)。実App(Bevy)を起動せず、`StateRegistry`・`MilitaryRegistry`・`BattleRegistry`・`CountryRegistry`を直接構築し、`run_daily_military_supply_consumption_with_battles`だけを80日ループで呼ぶ。d20・d45で`occupy_state`により回廊(隣接する2つの自国連結州)を敵支配へ切り替え/戻す。本監査の一時実装で**実測: 0.0になる日はd20〜d44の正確に25日間、それ以外は1.0**であることを確認した(§12参照コードは daily_supply.rs のテストモジュールへ追加する形を想定)。これが`test_03`の「ちょうど25日」契約を、戦闘終了ポリシーと無関係に引き継ぐ。
- **`test_04`(首都陥落)も同型の補給専用テストへ置き換える**: d20で首都(StateId(0)相当)を占領し、以後ずっと0が続くことを検証する(戦闘の生死とは独立)。
- **既存`test_01`(基本ブリッジ、d10到着・d11勝利)は改変不要**: 攻撃側4対防御側1で、防御側は常に正の実効戦力を持つため、候補1のトリガ条件を満たさず、そのまま通過する(実測済み、§39参照)。
- **既存`test_02`,`05`,`06`,`07`〜`10`も改変不要**(いずれも「両陣営とも0」を作らないシナリオ)。
- **新設: 実戦闘統合テスト**(P27-007の一部として新規追加、`tests/p27_007_...`が想定名)。両陣営0を実際に発生させ、即日終了・攻撃側撤退・controller不変・90日間再攻撃なしを検証する(§31の一時パッチで実現可能性を確認済み)。

## 44. Save V1/V2判断

**Save V1を維持する。新規永続フィールドは不要**。候補1は既存フィールド(`Battle.status`、`Battle.attacker_origins`、`Division`の各既存フィールド)だけを使う。

## 45. save/load

P27-006の`test_10_save_load_matches_the_continuous_run_at_every_critical_point`の`both_zero_spec`分岐(d1、d20)を、候補1の一時パッチ適用状態で再実行し、連続実行と完全一致することを確認した(候補1適用前は「維持」、適用後は「即日終了」に結果自体は変わるが、**save→loadの一致性そのものは影響を受けない**ことを確認)。

## 46. manpower保存則

撤退時の10%ペナルティは既存の`RETREAT_MANPOWER_LOSS_RATIO`・既存の比例装備損失計算をそのまま再利用するため、既存の保存則(実際の減少量に基づく装備破壊)がそのまま成立する。

## 47. Equipment保存則

同上、既存の`calculate_proportional_equipment_loss`・`try_destroy_equipment`をそのまま再利用。国家備蓄には触れない(既存契約どおり)。

## 48. MilitarySupplies保存則

無変更。撤退後のDivisionは翌日から通常のIdle Divisionとして日次補給の対象になるだけで、二重消費・二重需要は発生しない。

## 49. 決定性

`process_daily_battles`は既にBattleId昇順で処理しており、`tmp_stalled`のような戦闘ごとの判定はHashMap反復順に依存しない(戦闘ごとに独立計算)。複数Divisionの登録順反転は§33で検証済み、結果に影響しない。

## 50. 国家間分離

候補1の判定・処理は当該戦闘の参加者IDだけを対象とし、他の戦闘・他国のDivision・備蓄・controllerには一切触れない。

## 51. P27-004〜006保護

- P27-004(移動冪等化): 候補1の撤退はDivisionの`destination`/`current_path`/`movement_progress`を明示的にリセットしてから`current_state`を直接書き換える(既存の組織率撤退と同一の手順)ため、翌日の移動命令発行と衝突しない。
- P27-005(前線補給): 資格判定関数(`is_frontline_supply_eligible`)・1ホップ制限は無変更。全10件のE2E中、`test_03`・`test_04`のみが§43の再設計対象で、他8件は候補1適用後もそのまま通過することを実測した。
- P27-006(M・T・C): `src/military/combat_calc.rs`は一切変更していない。256件グリッドテストを含む全26件の単体テストは無変更のまま通過する(候補1は`update.rs`だけの変更のため、`combat_calc.rs`のテストに影響しない)。

## 52. Critical/High/Medium/Low

- Critical: 0
- High: 0
- Medium: 0(M-1・M-2とも設計・実測により解消経路が確定した。以後はP27-007の実装待ち)
- Low: 2
  - 既存の「攻撃側撤退先の安全性を検証しない」ギャップ(§27、§30)。候補1導入前から存在し、候補1で悪化しない。将来の独立検証候補。
  - プレイヤーが両陣営0による強制撤退の理由を画面上で把握できない(§32、UI通知は本タスク・P27-007のスコープ外)。

## 53. 次の原子的タスク

**P27-007: 両陣営が真の実効戦力0となった戦闘を、正規の防御側勝利・攻撃側撤退経路で終了する**

範囲: `src/military/update.rs`の`process_daily_battles`(戻り値`HashSet<BattleId>`追加)・`resolve_finished_battles`(引数追加、候補1の判定・処理追加)・撤退共通ヘルパー(既存の組織率撤退分岐との重複を関数抽出で解消することが望ましい)。P27-005 test_03・test_04の§43再設計。単体テスト、E2E、save/load、決定性、90日・36か月・120か月検証。

## 54. 対象外

organization全面再設計、捕虜、戦略的撤退UI、士気、戦線全体のAI再設計、補給式変更、損害式変更、P27-004〜006の再実装、講和全面変更、攻撃側撤退先の安全性検証強化(§52のLow finding、独立検討)。

## 55. 実行コマンドと正確な結果

一時パッチ適用状態での測定(§12・§31・§33・§38・§39・§40〜41・§45):

- `cargo test --test p27_006_low_effective_power_damage_e2e_test`: 候補1適用前10 passed、適用後9 passed/1 failed(`test_08`のみ、意図どおり)
- `cargo test --test p26_008_b_supply_aware_offensive_gate_e2e_test`: 候補1適用後も19 passed(不変)
- `cargo test --test p27_005_offensive_frontline_supply_bridge_e2e_test`: 候補1適用後8 passed/2 failed(`test_03`・`test_04`のみ、意図どおり)
- `cargo test --lib`(補給専用25日テスト・複数師団混成テストを含む一時テスト): 全通過
- `cargo test --test p27_002_a/b/c`: 候補1適用後も全通過、発火0件
- `cargo test --all-targets --no-fail-fast -j 2`(候補1適用状態): 失敗は上記3件のみ

一時パッチ復元後:

- `cargo test --all-targets -j 2`: **2680 passed / 0 failed / 1 ignored**(監査開始時と同一、一時変更を全て復元済みのため)
- `cargo clippy --all-targets --all-features -- -D warnings`: 警告0

Windowsのページファイル不足を避けるため、フル実行は`-j 2`で行った。

## 56. 一時検証コード削除

`src/military/update.rs`・`src/military/daily_supply.rs`・`tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs`への一時変更は、`git checkout -- <path>`で個別に復元した(3ファイルとも本監査では恒久変更なし)。復元後、`TMPPATCH`・`tmp_`等の一時識別子が0件であることを確認した。

## 57. ユーザーSave保護

`saves/savegame_v1.ron`は読み書き・復元・ステージのいずれも行っていない。

## 58. PNG差分処理

テスト実行で再生成された既知の9件のPNGを、個別指定の`git checkout -- <path>`で複数回復元した。

## 59. コミット対象

本報告書1件のみ。

## 60. 最終`git status --short`

コミット後は空(最終報告に実測値を記載する)。
