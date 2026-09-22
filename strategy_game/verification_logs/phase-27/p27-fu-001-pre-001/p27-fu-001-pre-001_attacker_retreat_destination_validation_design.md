# P27-FU-001-PRE-001 攻撃側撤退先の有効性・代替退却先・退路なし契約の設計監査

これはPhase 27を再オープンする実装ではなく、P27-008で詳細化されたLow finding(攻撃側撤退先の安全性チェック不足)を独立して評価する設計監査である。

## 1. 判定

```text
ANALYSIS COMPLETE
REPRODUCTION CONFIRMED
ROOT CAUSE CONFIRMED
RETREAT DESTINATION CONTRACT DECIDED
P27-FU-001 IMPLEMENTATION READY
PHASE 27 STATUS PRESERVED
```

Phase 27を再オープンする必要はない。今回のfindingはクラッシュ・Division複製・controller破壊・会計破壊を伴わない、既存契約の非対称性であり、独立タスクとして実装できる。

## 2. 基準状態

開始時HEAD: `ae37ec6d95673e670b7b22e21de7df8f6c574196`。`git status --short`は空。開始時基準: 2685 passed / 0 failed / 1 ignored、clippy 0。

## 3. 現行撤退経路の完全監査

| | 攻撃側(現行) | 防御側(現行、`find_retreat_destination`) |
|---|---|---|
| 開始条件 | organization<=0、またはP27-007の両陣営ゼロ損害 | organization<=0 |
| 呼び出し元 | `resolve_finished_battles`(2箇所: 通常経路・P27-007経路) | `resolve_finished_battles` |
| 候補州の生成 | `Battle.attacker_origins`に記録済みの1点のみ | `current_state`(=battle州)の隣接州全て |
| controller条件 | **なし(検証しない)** | 隣接州の`controller()==owner`必須 |
| owner条件 | ownerで記録された1点をそのまま使う | 同上(controllerで代用) |
| 隣接条件 | 記録時点で隣接だったことのみ保証、使用時は未検証 | 使用時に`current_state.neighbors`から生成するため常に隣接 |
| 通行可能性 | 検証なし | 検証なし(`StateData`にそもそも地形通行可否フィールドがない) |
| 首都連結 | 検証なし | 検証なし |
| 進行中の他戦闘 | 検証なし | 候補州に進行中戦闘があれば除外 |
| tie-break | 不要(1点のみ) | StateId昇順、先頭を採用 |
| 撤退成功時 | `force_retreat_or_destroy`: `current_state=dest`、`status=Idle`、`combat_id=None`、`destination/target_state=None`、`current_path`クリア、`movement_progress=0.0` | 同一関数を共有 |
| 撤退失敗時 | 装備全損→`remove_division`(既存消滅処理) | 同左 |
| manpowerペナルティ | `RETREAT_MANPOWER_LOSS_RATIO`(最大manpowerの10%) | 同左 |
| Equipment | `calculate_proportional_equipment_loss`で実減少量に比例して破壊、国家備蓄へは戻らない | 同左 |
| MilitarySupplies | 触れない | 同左 |
| Battleからの除外時点 | `remaining_attackers`/`remaining_defenders`構築と同時 | 同左 |
| Save対象 | `Battle.attacker_origins`(既存フィールド)、Division本体 | Division本体のみ(探索結果は非永続) |

## 4. `attacker_origins`の意味監査

- **保存時点**: `start_battle_between`(新規戦闘開始時)または`join_battle_as_attacker`(合流時)。いずれも呼び出し元の`process_division_arrival`が渡す`origin_state`は、そのDivisionの**直前の`current_state`**(移動が単発ホップであるため、到着州の隣接州であることが構造的に保証される)。
- **保存単位**: Battle単位ではなくDivision単位(`HashMap<DivisionId, StateId>`)。
- **複数攻撃Division**: 各自が個別の`origin_state`を持つ(合流タイミングが異なれば出撃元も異なり得る)。
- **戦闘中の再保存**: 一度記録された値は、そのDivisionが同じBattleに参加し続ける限り上書きされない。
- **controller変更後の保持**: 記録した州のcontrollerがその後変化しても、`attacker_origins`の値自体は追従・更新されない(**不変条件は「記録時点で隣接だった」ことだけで、「現在も自国支配」を保証しない**)。
- **講和・国家消滅**: `attacker_origins`を個別にクリーンアップする経路はない。Battle自体が終了・削除されれば`attacker_origins`ごと消える。
- **不正・欠落**: `HashMap::get`が`None`を返すケースを`force_retreat_or_destroy`が既に安全に処理する(消滅)。

**結論**: `attacker_origins`を「常に安全な撤退先」とみなせる不変条件は存在しない。保証されるのは「記録時点で隣接する自国支配州だった」ことのみ。

## 5. 敵支配origin再現(動的、P27-008と同一手法で再確認)

一時パッチで、攻撃側の出撃元(州1)を戦闘継続中に敵(Elfin)支配へ変更したうえで両陣営ゼロ損害を発生させたところ、攻撃側は敵支配下の州1で`Idle`になった(`current_state==StateId(1)`、`controller==ELFIN`)。P27-008の再現と一致。

## 6. 第三国origin結果

現行の外交モデルでは、`state.controller()`は単一の`CountryId`であり、「同盟国」「共同交戦国」「中立国」を区別する追加ロジックは撤退先判定に組み込まれていない。防御側の`find_retreat_destination`も同様に`controller()==owner`(自国のみ)しか許可しない。したがって、第三国(同盟国等)の州を撤退先として明示的に許可する現行の仕組みはなく、本監査でもそれを新設しない(§8参照、必須ではないと判定)。

## 7. 首都非連結origin結果

現行の`find_retreat_destination`・`attacker_origins`使用のいずれも、首都連結性を検証しない。動的確認により、首都非連結だが自国controllerの州は、現行の防御側探索でも有効な撤退先として選ばれることを確認した(既存契約、変更しない)。

## 8. 通行不能origin結果

`StateData`に地形通行可否フィールド自体が存在しないため、「通行不能」は現行データモデルでは表現不能(§8項目8「現行システムでは表現不能」)。

## 9. origin欠落結果

`force_retreat_or_destroy`の`match retreat_destination { Some(dest) if ... => ..., _ => 消滅 }`により、`None`は既に安全に消滅処理される。panicなし。

## 10. 複数Division結果(動的)

一時パッチで、2攻撃Division(origin=州1[有効]、origin=州4)を用意したところ、**州4は実データ上デフォルトでElfin(敵)所有**であることが判明した(P27-005の回廊遮断機構は「占領後にArcadia支配になった州4」を前提としており、素の状態では敵領である)。この結果、州4を出撃元とする既存テスト(P27-006 test_11)自体が「現在も自国支配のorigin」という前提を満たしていなかったことが判明した。これは今回の監査の副産物であり、既存テストの設計上の見落としである(修正は本タスクの対象外、次のP27-FU-001実装タスクでの既存テスト更新時に併せて扱う)。

## 11. 国家消滅結果

`force_retreat_or_destroy`はDivisionとBattleの整合性のみで動作し、国家の生死を直接参照しない。攻撃国が消滅した場合の後始末は既存の国家消滅処理(P21-021)の管轄であり、本監査では新たな不整合を導入しないことのみ確認した(既存契約、変更なし)。

## 12. 講和結果

`process_daily_battles`は戦争が`WarStatus::Active`でなくなると戦闘を`Cancelled`にして`cleanup_battle_units`を呼ぶ既存経路があり、攻撃側撤退処理とは別の経路で後始末される(既存契約、無変更)。

## 13. 仮説A〜M

| 仮説 | 判定 |
|---|---|
| A: 直接原因はoriginの現在controllerを検証しないこと | **CONFIRMED** |
| B: 敵支配originへ撤退すると敵州でIdleになる | **CONFIRMED**(§5) |
| C: 敵州Idleは翌日の補給・AI・再戦闘へ影響する | PARTIAL。補給は翌日から通常のIdle Division扱いになる(P27-005前線補給対象外、既存Idle Division同様の需要になるだけで異常値は生じない)。AI再攻撃・再戦闘は自然な120か月運用で対象経路自体が発火しないため実測できず、NOT TESTABLEに近い |
| D: 防御側の既存探索を攻撃側へそのまま再利用できる | PARTIAL。**探索起点が異なる**(防御側は`current_state`固定、攻撃側は`origin`優先+`battle_state`隣接がフォールバック起点)ため「そのまま」ではないが、候補生成ロジック(隣接州・controller・進行中戦闘の3条件)は共通関数として抽出・再利用できる(§14) |
| E: originが無効でも隣接する自国支配州へ安全に撤退できる | PARTIAL。**battle州(現在の交戦州)に隣接する自国支配州が存在する場合のみ**成立する。動的確認(§10)では該当州が存在せず消滅した |
| F: 首都非連結州も撤退先として許可できる | CONFIRMED(既存の防御側探索が既にそうしているため、対称性のために許可する) |
| G: 同盟国controller州を撤退先にできる | REJECTED(現行モデルでは自国controllerのみ、§6) |
| H: 代替先なしの消滅は既存契約と整合する | CONFIRMED(既存の防御側包囲撃破と同じ契約) |
| I: 新規Saveフィールドは不要 | CONFIRMED(§18) |
| J: 修正はP27-004〜008の契約を変えない | CONFIRMED(§19〜23で再回帰確認) |
| K: 自然AI運用でも現行欠陥が発火する | REJECTED。120か月運用で戦闘自体が0件のため発火しない(P27-005〜008で既に確立した事実と整合) |
| L: 現行重要度Lowは妥当 | CONFIRMED(§20で再評価、クラッシュ・複製・破壊なし) |
| M: Phase 27を再オープンする必要がある | REJECTED |

## 14. 有効な撤退先の条件

| 条件 | 判定 |
|---|---|
| Stateが存在する | 必須 |
| battle州と隣接する | 必須 |
| controllerがownerである | 必須 |
| ownerがownerである(=controller判定で代用) | 必須(controllerと同一視) |
| 同盟国・共同交戦国controllerも許可 | 不要(現行モデルで区別不能、§6) |
| 中立国controllerを許可 | 不要 |
| 敵国controllerを拒否 | 必須(controller==owner必須の裏返し) |
| 通行可能な地形 | 現行システムでは表現不能 |
| 海峡・海上隣接 | 現行データに区別なし、既存探索と同じ扱いでよい(推奨: 変更しない) |
| 首都連結 | 不要(既存の防御側探索と対称にするため要求しない) |
| 補給可能性 | 不要。**撤退可能性と補給可能性は分離する**(補給0でも撤退は認める、既存の防御側契約と同じ) |
| 敵Divisionが存在しない | 推奨(既存探索は明示チェックしないが、controller==ownerであれば通常は敵Divisionもいない) |
| 別Battleが存在しない | 必須(既存の防御側探索が既にチェックしている条件を踏襲) |
| 国家が消滅していない | 必須(existing、Division自体が既に消滅処理される) |
| 講和後の通行権 | 現行モデルで表現不能、既存契約のまま |
| Divisionが対象Battleへ参加中 | 前提条件(呼び出し元で既に保証) |

## 15. 候補0〜6比較(動的検証込み)

| 候補 | 判定 | 根拠 |
|---|---|---|
| 0 現状維持 | 不採用 | §5の敵州Idleが解消しない |
| **2 origin優先＋既存探索相当の代替先** | **採用** | 動的検証で正しく機能(§16)。防御側と対称的な契約。新規状態不要 |
| 1 origin無効なら即消滅(代替なし) | 不採用 | 候補2で代替先探索を安価に追加できるため、無条件消滅は過度に保守的(せっかく隣接する自国州があるのに見捨てる) |
| 3 origin優先＋隣接自国支配州のみ探索 | 候補2に内包 | 候補2の代替探索ロジックは実質これと同一(§14の首都連結を要求しないため) |
| 4 origin優先＋首都連結済み州のみ探索 | 不採用 | 撤退可能性と補給可能性を結合してしまう(§14で分離すると判定) |
| 5 最寄り安全州までBFS | 不採用(過剰設計) | 隣接1ホップを超える探索は経路上の敵州通過・実装規模の観点で本タスクの原子性を超える |
| 6 撤退を通常Movementへ変換 | 不採用(対象外) | 撤退中の再戦闘・移動速度・新規状態が必要になり、本タスクの範囲を大きく超える |

## 16. 採用候補の動的検証

一時パッチ(`src/military/invasion.rs`に`tmp_attacker_retreat_destination`、`src/military/update.rs`の2呼び出し箇所を差し替え)を実装し、以下を確認した(監査後に完全復元、§27)。

```text
tmp_attacker_retreat_destination(battle_state, owner, preferred_origin, state_registry, battle_registry):
  1. preferred_originが Some(x) かつ x.controller()==owner かつ xに進行中の他戦闘がない
     → x を返す
  2. それ以外
     → battle_stateの隣接州から、controller()==owner かつ進行中の戦闘がない州を
        StateId昇順で探索し、先頭を返す(既存find_retreat_destinationの候補生成
        ロジックをbattle_state基準で共有関数化したもの)
  3. 代替先もなければ None(既存の消滅処理)
```

**動的結果**:
- 敵支配origin＋battle州に他の自国隣接州が存在しない構成(§5・§10と同一マップ): 正しく**消滅**した(候補なしの契約どおり)。
- save/load: この消滅結果はsave→load後も一致した。
- **全体テストへの影響**: `cargo test --all-targets --no-fail-fast -j 2`で**13件が失敗**した(候補2適用時のみ、本番コードには反映していない)。

内訳:

| ファイル | 失敗件数 | 原因 |
|---|---|---|
| tests/p25_004_combat_equipment_loss_e2e_test.rs | 1 | 同上 |
| tests/p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs | 5 | 同上 |
| tests/p25_007_equipment_combat_effectiveness_e2e_test.rs | 3 | 同上 |
| tests/p26_005_supply_combat_strength_e2e_test.rs | 1 | 同上 |
| tests/p26_006_capital_connectivity_supply_e2e_test.rs | 1 | 同上 |
| tests/p27_006_low_effective_power_damage_e2e_test.rs | 2 | 同上(§10で判明した州4誤前提を含む) |

**全13件が同一パターン**: 各テストのfixtureが、実際には自国支配ではない(またはbattle州に有効な代替隣接州が存在する)`origin`を使っており、**旧実装(無条件でoriginを使う)では確実にDivisionが消滅していた**ものが、**候補2では代替先が見つかり生存する**(または逆に、旧実装で生存していたケースが実は無効なoriginにたまたま救われていただけだったことが判明する)ケースだった。いずれも**クラッシュ・panic・Division複製・controller破壊・会計破壊ではなく、「生存するか消滅するか」という結果の相違**であり、各テストの`.unwrap()`が前提としていた「Division不在」または「Division存在」が変わったことによるものである(`tests/land_war_combat_peace_test.rs`自体は含まれない)。

**判定**: 候補2は正しく機能しており(クラッシュ・保存則破壊はゼロ)、変化は全て「無効なoriginを無条件に信頼していた既存の暗黙の前提」を修正した結果である。ただし**当初想定した「敵支配化した出撃元のみを救済する」より意味的に広い**(「州4のように最初から自国支配ではなかったorigin」も同様に扱う、動作としては正しいが影響範囲は広い)。次の実装タスクでは、この13件の期待値をレビューし、意図的な前提だったのか単なる見落としだったのかを個別に判定して更新する必要がある。

## 17. 防御側関数の再利用可否

**判定: 内部の純粋な候補生成ロジックだけ抽出して再利用可能。関数全体はそのまま再利用不可(探索起点が異なるため)。**

`find_retreat_destination`から「隣接州を`controller==owner`かつ進行中戦闘なしでフィルタしStateId昇順で選ぶ」部分を`tmp_neighbor_retreat_destination(current_state, owner, ...)`として抽出し、防御側は`current_state=battle.state_id`で呼び、攻撃側は同じ関数を代替探索のフォールバックとして`battle_state`基準で呼ぶ設計とした(§16で実装・検証済み)。防御側の公開シグネチャ(`division_id`を取り`military_registry`からownerを引く形)は既存呼び出し元との互換性のため変更しない。

## 18. tie-break

StateId昇順、既存の防御側探索と同一(§14)。origin自体は1点のみのため、origin優先分岐にtie-breakは不要。

## 19. 状態変更の原子性

候補2の実装は既存の`force_retreat_or_destroy`をそのまま再利用し、退却先の決定(`tmp_attacker_retreat_destination`)は状態を変更しない純粋関数として、状態変更(`force_retreat_or_destroy`)の**前**に完了する。動的検証で、Battleに残ったままDivisionだけIdleになる、current_stateだけ変更されdestinationが残る等の不整合は確認されなかった(既存の一括更新ブロックをそのまま使うため)。

## 20. manpowerペナルティ・Equipment・MilitarySupplies

候補2でも`force_retreat_or_destroy`をそのまま呼ぶため、既存の10%ペナルティ・比例装備損失・MilitarySupplies非関与は無変更。

## 21. controller

撤退処理自体はcontrollerを変更しない(既存契約、無変更)。

## 22. AI副作用

新しいcooldownや撤退AIは設計・実装していない。§16の代替先探索により、敵州`Idle`ケースの一部が「代替先へ撤退」または「消滅」に変わるため、**敵州`Idle`件数はゼロへ近づく方向の変化**になる。P27-007のAI再攻撃ループ防止(P26-008-Bの既存ヒステリシス)には影響しない(撤退先の選び方が変わるだけで、`stance`/`blocked`ロジックには触れていない)。

## 23. プレイヤー副作用

`tmp_attacker_retreat_destination`はDivisionの`owner`とBattleの`attacker_country`だけを参照し、プレイヤー/AIの区別をしない。プレイヤー操作への影響はAIと同一。

## 24. save/load

§16の動的検証で、敵支配origin→代替先なし→消滅のケースがsave→load後も連続実行と完全一致することを確認した。

## 25. Save V1／V2

**Save V1を維持できる。新規永続フィールドは不要。** `tmp_attacker_retreat_destination`はDivision・StateRegistry・BattleRegistryの既存フィールドだけを読み、何も書き込まない。

## 26. 決定性・国家間分離

代替探索はStateId昇順の決定的選択(既存の防御側探索と同一アルゴリズム)。国家間分離は、判定が当該Battleのowner・battle_state・隣接州だけを参照するため既存どおり成立する。

## 27. P27-004〜008保護

一時パッチ適用状態で`cargo test --all-targets --no-fail-fast -j 2`を実行し、P27-004〜008自身の恒久テスト(`p27_004_...`、`p27_005_...`、`p27_006_...`のtest_11/12以外、`p27_007_...`、`p27_008`関連)はいずれも**候補2の影響を受けず通過**することを確認した(§16の13件はいずれもPhase 25〜26寄りの既存テストで、P27-004〜008自身の契約とは別物)。一時パッチは完全に復元し、本番コードは無変更のまま最終回帰(§28)で2685 passed / 0 failed / 1 ignoredへ戻ることを確認した。

## 28. 36か月・120か月結果

自然な通常AI運用では、120か月を通じて戦闘自体が0件であるため、攻撃側撤退処理(候補2を含む)は**発火しない**。この事実はP27-005〜008の既存報告書と一貫している。制御シナリオ(§5・§10・§16)を主証拠とし、自然発火0件は追加情報として報告する。

## 29. Critical/High/Medium/Low

- Critical: 0(Save破壊、Division複製、controller破壊はいずれも未検出)
- High: 0(通常戦闘の恒久停止や戦争・占領の不成立は未検出)
- **Medium: 0**。敵支配originによる敵州`Idle`は、クラッシュ・複製・会計破壊を伴わず、自然運用で到達しない特定条件(origin recapture)下でのみ発生するため、Mediumへの引き上げは不要と判定する。
- **Low: 1(既存findingの継続、詳細化済み)**。ただし候補2の動的検証により、**影響範囲は当初のP27-008記述より広い**(「敵支配化したoriginだけでなく、そもそも自国支配でなかったorigin全般」に及ぶ)ことが判明した。次のP27-FU-001実装タスクの見積もりに反映する。

## 30. 次の原子的タスク

**P27-FU-001: 攻撃側撤退先の有効性検証と代替退却先の追加**

- 変更ファイル: `src/military/invasion.rs`(`find_retreat_destination`から候補生成ロジックを共通ヘルパーへ抽出、新規`attacker_retreat_destination`相当の公開関数を追加)、`src/military/update.rs`(`resolve_finished_battles`の2箇所の攻撃側撤退呼び出しを新関数経由へ変更)
- 新規/変更関数: §16の`tmp_attacker_retreat_destination`相当を正式実装(仮称`attacker_retreat_destination`)。引数は`battle_state: StateId, owner: CountryId, preferred_origin: Option<StateId>, state_registry: &StateRegistry, battle_registry: &BattleRegistry`、戻り値`Option<StateId>`
- 正規の有効条件: §14の表のとおり(controller==owner、隣接、進行中の他戦闘なし。首都連結・補給可能性・同盟国は要求しない)
- origin優先規則: §16のとおり(origin有効ならoriginを最優先)
- 代替先探索: battle_stateの隣接州から、既存`find_retreat_destination`と同一条件・同一tie-break(StateId昇順)
- 退路なし: 既存の`force_retreat_or_destroy`の消滅分岐をそのまま使用(変更なし)
- 状態変更順序: 変更なし(既存の`force_retreat_or_destroy`を再利用)
- Save判断: Save V1維持、新規フィールドなし
- 単体テスト: 最低限、origin有効・origin敵支配化＋代替先あり・origin敵支配化＋代替先なし・origin欠落・複数Division(有効/無効混在)・登録順反転
- E2Eテスト: §16の敵支配origin＋代替先ありのケースを追加(現行の一時パッチでは代替先なしのケースしか実マップで再現できなかったため、複数方向からの占領等で代替先が実在する構成を新たに設計する必要がある)
- 既存テスト更新: §16の13件(tests/p25_004_combat_equipment_loss_e2e_test.rs、tests/p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs、tests/p25_007_equipment_combat_effectiveness_e2e_test.rs、tests/p26_005_supply_combat_strength_e2e_test.rs、tests/p26_006_capital_connectivity_supply_e2e_test.rs、tests/p27_006_low_effective_power_damage_e2e_test.rsのtest_11・test_12)を個別に精査し、意図(「退路なし」を検証したいテストか、単に無効なoriginを流用していただけか)に応じてfixtureまたは期待値を更新する
- 対象外: UI通知、外交上の通行権モデル拡張、撤退の移動状態化、戦略的撤退AI、cooldown追加
- 完了条件: §14の条件が実装に反映され、§16の13件が意図に沿って更新され、全テスト成功、clippy 0 warnings

## 31. 対象外

撤退先の安全性以外(UI通知、P27-005手動プレイ確認、`daily_supply.rs`のrustfmt差分)は、P27-008の既存記載のまま変更していない。

## 32. 実行コマンドと正確な結果

- 開始時: `cargo test --all-targets -j 2`: 2685 passed / 0 failed / 1 ignored
- 一時パッチ適用時: `cargo test --all-targets --no-fail-fast -j 2`: 2672 passed / 13 failed / 1 ignored(§16、本番コードには反映せず)
- 一時パッチ復元後: `cargo test --all-targets --no-fail-fast -j 2`: 2685 passed / 0 failed / 1 ignored(§33)
- `cargo clippy --all-targets --all-features -- -D warnings`: 警告0(復元後)

## 33. 全テスト件数

**2685 passed / 0 failed / 1 ignored**(開始時と同一、一時パッチは完全復元)。

## 34. clippy警告数

0。

## 35. 本番コード・データ・既存テスト無変更

`git status --short`で確認済み(§37)。

## 36. 一時検証コード削除

`src/military/invasion.rs`・`src/military/update.rs`・`tests/p27_006_low_effective_power_damage_e2e_test.rs`への一時変更は`git checkout -- <path>`で個別に復元した。復元後、`TMPDBG`・`TMPFU1`・`tmp_attacker_retreat_destination`等の一時識別子が0件であることを確認した。

## 37. ユーザーSave保護

`saves/savegame_v1.ron`は読み書き・復元・ステージのいずれも行っていない。

## 38. PNG差分処理

`cargo test --all-targets`実行のたびに再生成される既知の9件のPNGが今回も差分として現れた。`git status --short`で対象を確認し、該当9ファイルだけを個別指定で`git checkout --`により復元した。

## 39. コミット対象

本報告書1件のみ。

## 40. 最終`git status --short`

コミット後は空(最終報告に記載する)。
