# P28-003 飛び地Divisionによる攻勢配送判定の停滞を解消

P28-003-PRE-001で採用した候補C5を実装した。攻勢可否の前線配送判定に限り、**自国支配州にいるが首都と陸路でつながらないDivision**(飛び地に取り残されたDivision)を判定の母集合から外す。敵支配州で攻撃中のDivisionは外さない。全員が除外対象になる前線は、従来の母集合で判定する。戦力比・前線割当・国家備蓄日数・日次補給・移動・戦闘・講和の計算、攻勢ゲートの40/70日と前線配送しきい値0.34は変更していない。

## 1. 判定

```text
P28-003 COMPLETE
```

全AI・無操作の自然運用で、war8(Dwarf→Arcadia、1806/05/02宣戦)の停止原因が解消した。州8の割譲(1802/02/09)で飛び地に残ったDwarfの2個Divisionは、従来は補給充足率0で前線配送判定を塞いでいた。実装後は連結済みDivisionで配送を判定し、宣戦翌日の1806/05/03に攻勢へ移り、1806/05/18にArcadiaが降伏した。監査の候補C5の一時実測と、全期間で同じ結果になった。補給0のDivisionを含む攻撃は0日。

## 2. 開始HEAD・基準

- 開始HEAD: `17d87e8ad1e5304c46aed7543836db41a564b244`(期待値と一致)
- 開始時`git status --short`: 空
- 開始時の全体テスト基準: 2725 passed / 0 failed / 1 ignored、clippy警告0。P28-003-PRE-001の最終検証(同一コミット・空の作業ツリー)の実測値で、本タスクの開始時には再実行していない。

## 3. 正規の関数・データ構造(実コードで確認)

| 対象 | 正規の定義 |
|---|---|
| 首都連結集合 | `military::capital_connectivity::connected_states_from_capital(首都, 国, state_registry)`。首都から`neighbors`をたどり、controllerが自国の陸上州だけを通る。首都自体が自国支配でなければ空集合 |
| 日次補給 | `military::daily_supply::run_daily_military_supply_consumption_with_battles`。上記の連結集合に含まれないDivisionは、配分対象外で充足率0。敵支配州で戦闘中の攻撃側は、P27-005の一歩先の前線補給(`is_frontline_supply_eligible`)で例外的に補給される |
| 前線配送判定 | `war::offensive_supply_gate::evaluate_offensive_supply_status` → `frontline_min_fulfillment(division_ids, …)`。需要正の生存Divisionの充足率の最小値を取り、0.34未満なら新規攻勢を拒否し、進行中の攻勢も継続しない |
| 攻勢判定 | `war::military_ai::evaluate_military_ai_decision`(純粋関数)。呼び出しは`process_daily_military_ai`の1か所だけ |

## 4. 変更内容

変更ファイル: `src/war/military_ai.rs`(本番コードはこの1ファイルだけ)。

### 4.1 新規の純粋関数 `offensive_delivery_division_ids`

```text
入力: 前線割当 assigned、指揮国 C、首都連結集合 connected、military_registry、state_registry
取り残された(stranded) ⇔ Divisionが登録に存在し、現在州が存在し、
                          その州の controller == C かつ 州 ∉ connected
scoped = assigned から stranded を除いたもの(assignedの順序を保つ)
返り値 = scoped が空なら assigned(そのまま)、そうでなければ scoped
```

- 除かないもの:
  - 敵支配州(controller ≠ C)にいるDivision(攻撃中。P27-005の対象になり得る)
  - 連結済みの自国支配州にいるDivision
  - 登録に存在しないDivision、州が存在しないDivision(従来どおり`frontline_min_fulfillment`側の判定に委ねる)
- ownerとcontrollerが異なる州は、controllerで判定する。他国所有で自国が占領している飛び地は除く。自国所有で敵が占領している州は除かない。
- `assigned`が空なら空を返す(割当0件の既存の扱いのまま)。

### 4.2 `evaluate_military_ai_decision`の引数追加

`delivery_division_ids: &[DivisionId]`を追加した。この集合は`evaluate_offensive_supply_status`(前線配送判定の最小値の母集合)**だけ**に渡し、次の判定には使わない。

- 割当0件の判定(`NoAvailableDivision`)
- 即応可能Divisionの判定(`Recovering`)
- 戦力比(呼び出し側が`plan.assigned_division_ids`で計算した`own_power`/`enemy_power`)

これらは従来どおり`plan.assigned_division_ids`で判定する。国家備蓄日数の分子・分母(国家の総備蓄・総需要)も変わらない。

### 4.3 `process_daily_military_ai`での接続

- 国ごとの評価(3日ごとまたはdirty)の冒頭で、その日の州controllerから首都連結集合を1回だけ計算する(永続化しない)。首都がない場合は空集合。
- 前線ごとに`offensive_delivery_division_ids`で母集合を求め、`evaluate_military_ai_decision`へ渡す。
- 前線割当(`assign_unassigned_divisions_to_frontlines`)・戦力比(`evaluate_powers`)・移動命令・日次補給は無変更。

### 4.4 分子・分母の整理

| 値 | 変更前 | 変更後 |
|---|---|---|
| 前線配送の最小値の母集合 | 前線の全割当Division | 前線の全割当Divisionから取り残されたDivisionを除く(除くと空なら全割当) |
| 国家備蓄日数(総備蓄 ÷ 総需要) | 全Division | 変更なし |
| 戦力比(前線の割当戦力 ÷ 敵国の全陸上Division戦力) | 全割当 | 変更なし |
| 割当0件・即応判定 | 全割当 | 変更なし |

## 5. 境界ケース

### 5.1 単体テスト(`src/war/military_ai.rs`、7件追加)

| テスト | 結果 |
|---|---|
| `p28_connected_friendly_divisions_stay_in_the_delivery_scope` | 連結済みの自国支配州だけ → 全員が母集合に残る |
| `p28_stranded_friendly_exclave_divisions_are_removed_when_connected_ones_remain` | 連結済み1+飛び地2(自国所有・自国支配の州6と、他国所有・自国支配の州7) → 連結済みだけ |
| `p28_all_stranded_front_keeps_the_original_set_instead_of_an_empty_scope` | 飛び地だけ → 元の集合(空集合にしない)。割当0件 → 空のまま |
| `p28_divisions_in_enemy_controlled_states_are_never_removed` | 敵支配州(州2)と、自国所有だが敵が占領中の州8のDivision → 除かない |
| `p28_scope_follows_the_current_controller_when_the_route_is_cut_and_restored` | 経路上の州9を敵が占領すると州10のDivisionが外れ、奪回すると戻る(その日のcontrollerに追従) |
| `p28_scope_does_not_depend_on_registration_or_neighbor_order` | 州の登録順・隣接州の格納順・割当の順を逆にしても同じ集合 |
| `p28_stranded_zero_fulfillment_no_longer_blocks_an_otherwise_ready_offensive` | `evaluate_military_ai_decision`経由: 割当全体(飛び地の充足率0を含む)なら`SupplyShortage`、本タスクの母集合なら`SufficientAdvantage`で攻勢。戦力比1:1なら`ComparableStrength`(110%条件は不変)。全員が飛び地なら`SupplyShortage`(空集合で攻勢許可しない) |

### 5.2 既存の境界

- 首都陥落: 首都が自国支配でなくなると連結集合が空になり、自国支配州の全Divisionが取り残された扱いになる。敵支配州にいるDivisionがいれば、その充足率で判定する。全員が自国支配州なら、全員除外となるため元の集合(充足率0)で判定し、攻勢は開かない。日次補給も同じ条件で補給0なので、判定と実態は一致する。
- 消滅国家: 戦争・前線を持たず、軍事AIの評価対象外(既存)。
- 前線割当0件: 空集合のまま渡し、従来どおり`NoAvailableDivision`/`Stopped`になる。
- 既存の`p26_008_b` `test_12`: 前線Bの全Divisionが首都と非連結の自国州にいる。この場合は元の集合で判定するため、期待値は変わらない(Offensive)。

## 6. 実App検証(E2E、`tests/p28_003_disconnected_friendly_frontline_delivery_e2e_test.rs`、2件追加)

実時間システムと同じく、`GameDate`を1日ずつ進めて日次・月次メッセージを同じフレームで発行する。

- `test_01_exclave_divisions_no_longer_block_war8_and_no_unsupplied_attack_occurs`:
  1. 州8(旧Oceanic首都)のownerがOceanic→Dwarfへ変わった日が**1802/02/09**(Oceanic降伏による割譲)。
  2. war8の宣戦当日(1806/05/02)に、Dwarfの取り残されたDivisionが**2個**で、いずれも州8にいて充足率0.0。国家備蓄は再開値70日を超える。
  3. war8のDwarf側の最初の攻勢は**1806/05/03**(宣戦翌日、軍事AIが動ける最初の日)。その日に`blocked=false`で、割り当てられた取り残しDivisionの充足率はすべて0.34未満だった。つまり従来の母集合なら停止していた。
  4. war8は**1806/05/18**に攻撃側勝利(防御側降伏)で終わる。
  5. 1800/01/01〜1806/06/01の毎日、進行中の戦闘の攻撃側に充足率0のDivisionが含まれた日は**0日**。
  - ゲート定数40/70と配送しきい値0.34が不変であることも確認する。
- `test_02_save_load_around_cession_exclave_and_offensive_matches_continuous_run`: 次の4分岐でsave→loadを挟み、GameDateが保存時点に戻ること、1806/06/01の全国の国庫・備蓄・建設キュー件数・全州のowner/controller・全Divisionの位置・人員・状態・全戦争の状態と終了日・戦闘ID発行数が連続実行と一致することを確認する。
  - 州8割譲前(1802/02/01)
  - 飛び地成立後(1803/01/01)
  - 攻勢開始前(1806/05/03)
  - 攻勢開始後(1806/05/10)

## 7. war8の時系列

| 日付 | 出来事 |
|---|---|
| 1801/04/29 | war2 Dwarf→Oceanic 宣戦 |
| 1802/02/09 | Oceanic降伏。州8(旧Oceanic首都)をDwarfへ割譲。Dwarf本土と陸路でつながらない |
| 1806/05/02 | war8 Dwarf→Arcadia 宣戦。Dwarfは7個Divisionを割当(うち州8の2個が補給0)。国家備蓄は再開値を大きく超える(月初時点で358日前後) |
| 1806/05/03 | 攻勢開始(従来は同日に`SupplyShortage`で停止し、240か月まで再開しなかった) |
| 1806/05/12 | 支配州変更(Arcadia州の占領) |
| 1806/05/18 | Arcadia降伏(攻撃側勝利)。この戦争で戦闘は発生していない(Arcadiaはこの前線に割当Divisionを持たない) |

監査の一時実測(候補C5)と同じ日付である。

## 8. 36・120・240か月の戦争別結果(全AI・無操作、実App)

【実App実測】P28-003-PRE-001と同じ診断ハーネス(実カレンダー、240か月)を実装後のコードで一時的に実行し、計測後に削除した。独立プロセスで2回実行し、ハーネスの表示順を除いて一致した。候補C5の一時パッチを当てた実行と比べても、戦争・戦闘・終戦のすべての結果が一致した(違いはハーネスが独自に集計した停止理由の数だけで、ハーネス側が割当全体の母集合で理由を再計算していたことによる)。

| 観測点 | 戦争(累計/継続/終結) | 戦闘(ID発行数) | 戦闘日数 | 攻撃側が補給0の戦闘日 | 現状(P28-003以前) |
|---|---|---|---|---|---|
| 90日 | 0/0/0 | 0 | 0 | 0 | 同じ |
| 36か月 | 8/3/5 | 10 | 38 | 0 | 同じ(8/3/5) |
| 120か月 | 12/3/9 | 16 | 40 | 0 | 10/4/6 |
| 180か月 | 15/1/14 | 21 | 42 | 0 | 12/2/10 |
| 240か月 | 15/1/14 | 21 | 42 | 0 | 13/3/10 |

1806年までの経過は現状と完全に一致する(36か月の値、war0〜war7の宣戦・終結日が同じ)。

継続戦争の分類(P28-003で解消したものと、残るもの):

| 観測点 | 戦争 | 状態 | 分類 |
|---|---|---|---|
| 36・120か月 | war3 Oceanic→Arcadia | 国境消失(境界ペア0)、両国`NoValidFrontline` | **未解消** — 非戦闘終結経路なし(M-1)。1811/11/12に別戦争の割譲で白紙講和 |
| 36・120か月 | war5 Passguard→Arcadia | Passguard 1 Div、前線比0.500、補給ゲートは開 | **未解消** — 戦力不足・敵戦力の数え方(L-2)。1811/11/12に連鎖で終結 |
| 36・120か月 | war6 Arcadia→Elfin | Elfin 前線比1.5、備蓄不足で`SupplyShortage` | **未解消** — 募兵と生産(M-2)。1811/11/07に終結 |
| — | war8 Dwarf→Arcadia | 1806/05/18に降伏 | **P28-003で解消** |
| 240か月 | war14 Dwarf・Elfin→Oceanic(1812/08/02) | Dwarf 25 Div、月需要750 / 月産207、備蓄7.3日、`SupplyShortage` | **未解消** — 募兵と生産(M-2) |

war3・war5・war6の終結は、現状(1811/12)より約1か月早い(1811/11)。war8の決着で戦争の流れが変わったことによる連鎖であり、P28-003が直接解消したものではない。

## 9. Save・決定性・プレイヤー保護・既存契約

- Save V1: 新しい永続フィールド・migrationはない。首都連結集合は評価のたびに計算し直し、保存しない。E2E `test_02`の4分岐で、save→loadが連続実行と一致した。
- 決定性:
  - 単体テストで、州の登録順・隣接州の格納順・割当の順に依存しないことを確認した。
  - 新しいE2Eを独立プロセスで2回実行し、いずれも通過した。
  - 240か月の診断を独立プロセスで2回実行し、結果が一致した。
- 国家間分離: 連結集合は指揮国ごとに、その国の首都から計算し、除外の条件もその国のcontrollerだけで決まる。他国の前線判定には影響しない。自然運用でも、1806年まで他国の経過が現状と同一だった。
- プレイヤー: 軍事AIはプレイヤー国家を評価しない(既存)。プレイヤーの前線・補給は変わらない。
- P27-005(敵州の攻撃側前線補給): 敵支配州のDivisionを除外しない(単体テスト)。`p27_005_offensive_frontline_supply_bridge_e2e_test` 10件は無変更で通過した。
- P26-006(首都連結補給): 日次補給・連結計算は無変更。`p26_006_capital_connectivity_supply_e2e_test` 18件、`p26_006_rev_001_ai_save_resume_determinism_e2e_test` 9件が通過した。
- P26-008-B(攻勢ゲート): 関数・定数は無変更。`p26_008_b_supply_aware_offensive_gate_e2e_test` 19件は無変更で通過した(`test_12`を含む)。
- P28-002(戦時建設): `p28_002_wartime_military_factory_construction_e2e_test` 3件が通過した。1801年までの正規値に影響しない。
- 既存テストの期待値の変更: なし。
- P27-005の手動プレイ確認: **未実施のまま引き継ぐ**。

## 10. Critical／High／Medium／Low

```text
Critical: 0
High: 0
Medium: 2
Low: 4
```

- **Medium M-3(P28-002・P28-003-PRE-001)は解消**: 飛び地の補給0が前線配送判定を塞ぐ問題。war8は1806/05/18に降伏まで進んだ。
- **Medium M-2(継続)**: 募兵ゲートが生産量を見ず、戦時工場の増産分が募兵に吸収される。120か月のwar6の遅延と、240か月で唯一残るwar14(Dwarf 25 Div、需要750・生産207)の原因。
- **Medium M-1(継続)**: 国境が消えた戦争に非戦闘の終結経路がない(war3)。
- **Low L-2(継続)**: 攻勢判定の敵戦力が敵国の全陸上Divisionを数える(war5)。
- **Low L-5(継続)**: 無効になった前線(境界ペア0)に割り当てたDivisionを外さない。
- **Low L-6(新規)**: 取り残されたDivisionは、配送判定からは外れるが、前線割当・移動命令の対象には残る。自然運用では、これらが攻撃に加わって補給0で戦った日は0日だった。首都陥落時に敵支配州のDivisionだけで判定される場合も、日次補給と同じ条件なので判定と実態は一致する。自動再配置は本タスクの対象外。
- **Low L-1(継続)**: 単一回廊。

### 次の原子的タスク

**P28-004-PRE-001: 募兵ゲートとMilitarySupplies生産の均衡の設計監査(M-2)**。飛び地の影響を取り除いた状態で、残る唯一の恒久停止原因(war14)と、war6の遅延を対象にする。

- 比較する案:
  - 交戦中の募兵に「月産 ≥ 募兵後の月需要」を要求する案
  - 平時・戦時で条件を分ける案
  - 募兵数の上限を生産能力から導く案
- 評価する影響:
  - 開戦初期の戦争の帰結(P28-003-PRE-001のB-flowでは、Dwarfがwar0で敗れた)
  - 宣戦判定130%・攻勢判定110%との相互作用
  - 財政

## 11. 実行コマンドと正確な結果

Cargoはすべて逐次実行した(並列起動なし)。

- `git rev-parse HEAD` → `17d87e8ad1e5304c46aed7543836db41a564b244`、`git status --short` → 空
- `cargo build -q` → 成功
- `cargo test -q -j 2 --lib war::military_ai` → 10 passed(既存3+新規7)
- `cargo test -q -j 2 --test p28_003_disconnected_friendly_frontline_delivery_e2e_test`:
  - 1回目: `test_01`が失敗した。最初の攻勢日の期待値を仮に1806/05/04と置いていたが、実測は1806/05/03(宣戦翌日)。実測値へ修正し、未使用のimportを削除した。
  - 修正後: 2 passed。その後、独立プロセスでもう2回実行し、2回とも2 passed。
- 一時診断(全AI・240か月、2回、独立プロセス) → 90日0/0/0、36か月8/3/5、120か月12/3/9、240か月15/1/14、戦闘21、攻撃側が補給0の戦闘日0。ハーネスの表示順を除き一致
- 個別実行(一時診断の削除後): `p28_002` 3、`p27_005` 10、`p26_006` 18、`p26_006_rev_001` 9、`p26_008_b` 19、`p26_008_a` 30、`p27_004` 6、`p27_006` 17、`p27_007` 2、`p27_fu_001_ai_retreat_alternative` 3、`p27_fu_001_elimination_peace` 3、`p27_002_c` 18、`p21_016` 1、`land_war_combat_peace_test` 4(無変更)。`--lib`: `war::` 188、`military::` 508、`country::country_ai` 63、`save::` 305。いずれも0 failed
- 1回目の最終検証: `cargo test --all-targets --no-fail-fast -j 2` → 2734 passed / 0 failed / 1 ignored(84バイナリ)。続く`cargo clippy`で、新規E2Eの`w.status.clone()`(`WarStatus`は`Copy`)が`clippy::clone_on_copy`で1件エラー。`w.status`へ直し、clippy・全体テストを再実行した
- 最終 `cargo test --all-targets --no-fail-fast -j 2` → **2734 passed / 0 failed / 1 ignored**(84バイナリ、exit 0)。開始時2725に、単体テスト7件・E2E 2件を加えた数。1 ignoredは既存の`profile_p21_021_elimination_scaling`。再生成された既知の非決定的PNG9件は個別に復元
- 最終 `cargo clippy --all-targets --all-features -- -D warnings` → 警告0(exit 0、修正後に再実行)

## 12. 保護事項

- 本番コードの変更は`src/war/military_ai.rs`だけ。RONデータ・Save形式・System順序・日次補給・首都連結・攻勢ゲート・募兵・戦闘・講和・財政・建設の式は無変更。
- `saves/savegame_v1.ron`: 読み書き・復元・ステージなし(テストはすべて一時ディレクトリのSaveを使う)。
- `tests/land_war_combat_peace_test.rs`: 変更・整形なし。ワークスペース全体のformatなし。既存報告書は変更していない。
- 一時診断テスト(`tests/p28_003_temp_diagnostic.rs`)は計測後に削除した。
- テストで再生成された既知の非決定的PNGは、`git status --short`で確認してから1件ずつ個別に復元した。

## 13. 最終`git status --short`

コミット前: `M src/war/military_ai.rs`、`?? tests/p28_003_disconnected_friendly_frontline_delivery_e2e_test.rs`、`?? verification_logs/phase-28/p28-003/`。コミット後は空。
