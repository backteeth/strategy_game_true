# P28-008：複数戦争下の講和処理を当該戦争へ限定する

## 0. 結論

`P28-008 COMPLETE`。P28-008-PRE-001で確定した候補Bを、`src/war/peace.rs`の`execute_peace_settlement`（降伏・割譲・白紙講和・E1・E2・プレイヤー講和の共通経路）へ実装した。

- 終結するWarに帰属しない占領と、別のActive Warで敵領内にいるDivisionは、今回の講和で変更されなくなった。
- 実装は`state.war_id`に一切依存しない（本番の占領経路は`war_id`を設定しないため）。判定は州の`owner`・`controller`、各Warの参加国、Warの`status`だけで行う。
- 自然運用（標準シナリオ1本・全AI・600か月）の別Warへのcollateralは、実装前の1件（1801/03/22、war3のE2がwar2のs9占領を消去）から**0件**になった。
- 同日に2件の降伏条件が成立しても、実`handle_daily_war_resolution`で両方の勝利が保たれる（候補Bで解消された。別契約は不要）。
- `p27_002_c`の月120正規値はP28-005時点の値へ戻り、P28-007が付けた誤った因果コメントを訂正した。
- Save V1維持、新規永続フィールドなし、migrationなし。

## 1. 基準状態

- 開始時 `git rev-parse HEAD` / `git log -1`: `316b7a7341015bc0ef8aa648f58f8c319955823e docs: audit concurrent war peace occupation normalization`（P28-008-PRE-001の報告書コミット、期待どおり）
- 開始時 `git status --short`: 出力なし（クリーン）
- 開始時テスト基準: 前タスク（P28-008-PRE-001/P28-007）の最終実測 2755 passed / 0 failed / 1 ignored、clippy 0（今回の開始時には再実行していない）

## 2. 実コードで再確認した事項（編集前）

- `execute_peace_settlement`の順序: 領土移転(1) → E1連鎖 → **占領状態の正常化(2)** → 戦闘中止(3) → 前線削除(4) → **敵領残存陸軍の帰還(5)** → 休戦(6) → 終結情報(7)。ステップ5の時点で終結Warの`status`はまだ`Active`（ステップ7で更新される）ため、「他のActive War」の判定では終結Warを明示的に除外する必要がある。
- 旧ステップ2は`controller_country`が設定済みで所有国が主要当事国（`primary_attacker/defender`）の州を、占領者・占領元Warを問わず全て戻していた（支持国所有の占領は逆に残留していた）。旧ステップ5は全Divisionを走査し、所有国でもcontrollerでもない州にいるDivisionをWarを問わず自国領へ移動していた。
- `state.war_id`の書込は`war/occupation.rs::process_occupation`のみで、本番Systemからは呼ばれない（呼び出しは`war/tests.rs`の1テストだけ）。本番の占領は`military::invasion::occupy_state`（`controller_country`のみ）。
- Active Warの列挙: `WarRegistry.wars`（std `HashMap`、反復順はインスタンスごとにランダム）。`handle_daily_war_resolution`のステップ3は未ソートで走査し、ループ内で各Warの降伏判定を直前の講和の後に評価する。ステップ3.5（E2）も未ソート。
- Save V1: `SaveGameV1.states: Vec<StateData>`。`controller_country`・`war_id`・`occupation_progress`・`original_owner`は既にV1に含まれる。

## 3. 実装（候補B）

### 3.1 占領状態の正常化（ステップ2）

`P(W)` = 終結Wの`attackers ∪ defenders`（主要当事国だけでなく全参加国）。各陸州で`controller != owner`（占領中）のとき、次をすべて満たす場合に限り`controller=owner`、`occupation_progress=0`、`original_owner=None`、`war_id=None`に戻す。

1. `owner ∈ P(W)` かつ `controller ∈ P(W)`
2. W以外のActive Warで、`owner`と`controller`が敵対陣営に分かれていない

それ以外の占領は変更しない。終結Wを指す`state.war_id`が残っていれば（本番では設定されない旧フィールドの衛生処理として）`None`にするが、判定には使わない。

### 3.2 敵領残存陸軍の帰還（ステップ5）

W以外のActive Warで、そのDivisionの所有国が、いるのが州の`controller`または`owner`と敵対している場合、そのDivisionを帰還対象から外す。除外以外の挙動（Idle化、経路クリア、帰還先の優先順位、Destroyed化）は変更しない。`relocate_hostile_territory_divisions`（2引数、公開API）は残し、新しい`relocate_hostile_territory_divisions_except`（skip集合付き）へ委譲する。

### 3.3 実装前に固定した真理値表（コード内docと恒久テストに反映）

| 状況 | owner∈P(W) | controller∈P(W) | 他Active Warで敵対 | 結果 | 対応テスト |
|---|---|---|---|---|---|
| 単独War | ○ | ○ | × | 戻す | `single_war_peace_normalizes_all_of_its_own_occupations` |
| 同じ防御国への別War（占領者はP(W)外） | ○ | × | - | 保持 | `peace_of_one_war_keeps_other_wars_occupation_and_division` |
| 当事国を共有（占領者はP(W)外） | ○ | × | - | 保持 | `peace_does_not_touch_occupation_of_a_state_owned_by_a_party_but_held_by_a_third_war` |
| 支持国所有州をP(W)の敵が占領 | ○ | ○ | × | 戻す | `state_owned_by_a_supporter_of_the_ended_war_is_normalized` |
| 講和後も同じ2国が別Active Warで敵対 | ○ | ○ | ○ | 保持 | `occupation_is_kept_while_the_same_pair_is_still_at_war_in_another_active_war` |
| 所有国がP(W)外 | × | - | - | 保持 | `state_whose_owner_is_not_a_party_of_the_ended_war_is_left_alone` |

## 4. 再現結果（実装前→実装後）

### 4.1 占領（関数レベル、実`execute_peace_settlement`）

恒久テスト15件を、旧`peace.rs`（HEAD）に対して実行する負の対照を行った: **11件が失敗、4件が通過**（通過は単独War、E1連鎖したWar自身の占領の復元、連鎖占領S10の現状維持、2引数の旧公開APIで、旧実装でも成立すべき挙動）。実装後は15件すべて通過。

| 状況 | 旧実装 | 新実装 |
|---|---|---|
| 同じ防御国への2つのWar、片方のみ白紙講和（E2相当） | 継続中Warの占領が消え、その師団が自国領へ移動 | 占領・師団とも保持、継続中WarはActive |
| 同上＋継続中Warの戦闘が進行中 | 戦闘中Divisionが戦場から移動してIdle化、Battleは`Ongoing`のまま不整合 | Fighting・`combat_id`・Battleとも保持 |
| 別Warの進軍中（Moving）Division | 自国領へ移動 | 保持 |
| 当事国共有（Xが一方で攻撃国、他方で防御国） | 別Warの占領が消える | 保持 |
| 降伏→割譲 | 割譲は実施されるが別Warの占領が消える | 割譲は実施、別Warの占領は保持 |
| 割譲で別WarがE1失効 | そのWar自身の占領は戻る | 同じ（終結Warの占領は残らない） |
| 国家消滅＋無関係なWar | 無関係なWarの占領が消える | 保持（消滅処理本体は不変） |
| 支持国所有州の占領 | **終結後も残留（逆方向の既存不具合）** | 解除 |
| 同じ2国が別Active Warでも敵対 | 消える | 保持（最後のWarの終結で解除） |

### 4.2 占領・Division（実App制御シナリオ）

P26-008-Bの90日シナリオからElfin隔離を外した再現（Arcadia(c0)対Elfin(c1)の制御War0、戦争目標なし）。

| 日付 | 旧実装 | 新実装 |
|---|---|---|
| 1800/03/13 | Arcadia占領中のElfin所有州s3,s4,s14,s16,s18、d0はs18にIdle | 同左 |
| 1800/03/14（同日宣戦のWar1(c3対c1)がE2で白紙講和） | 5州すべてElfin支配へ、d0はs18→s0へ移動、War0はActive | **5州保持、d0はs18のまま**、War0はActive |

### 4.3 自然運用（標準シナリオ1本、600か月）

占領正常化の分類（その日に終結したWarの中に占領者と所有国が敵対するものがあるか）の600か月累計:

| 指標 | 実装前（P28-007 HEAD） | 実装後 |
|---|---|---|
| 正当な正常化（州数） | 5 | 5 |
| 別Warの占領を巻き込んだ正常化 | **1**（1801/03/22 war3のE2がwar2のs9を消去） | **0** |
| 終結Warに無関係なDivision移動 | 0 | 0 |
| 終結Warの敵領からの正当な帰還 | 2 | 2 |
| 同時Activeの最大数 / 当事国を共有した日数 / 講和件数 / うち当事国共有中の講和 | 3 / 2303日 / 8 / 2 | 同左 |

## 5. 同日複数終結の判定

- **解消済み**: 実`handle_daily_war_resolution`で、実マップのElfinに対しArcadiaとDwarfが別の州を目標に宣戦し、両方の目標州を占領済み、Elfinは戦闘可能な陸軍なし・新規募兵不可、という状態を作って1日進めるテスト（`real_handler_same_day_capitulations_both_win_for_every_registry_hash_order`）。`WarRegistry`の`HashMap`反復順はインスタンスごとにランダムなため、新しいAppを12回作って処理順を確率的に網羅した。旧実装では必ず片方が負けた（反復0回目で失敗を確認）。新実装では12回とも両Warが`AttackerVictory`で、目標州はそれぞれArcadia・Dwarfへ割譲される。
- レジストリレベルでも、2降伏の両順序（`two_wars_capitulating_the_same_day_both_win_in_either_order`）と、降伏＋白紙2件の全6順序（`three_wars_ending_the_same_day_reach_the_same_state_in_all_orders`、旧実装は2通り→新実装は1通り）で最終状態（owner・controller・War・Division）が一致する。
- 反復順そのもの（ステップ3・3.5の未ソート）は変更していない。テストした範囲では結果が順序に依存しないが、ソートによる決定論化は別の衛生対策として残している（§9）。HashMap反復順の網羅は確率的（12回）であり、証明ではない。

## 6. P28-007のE2とwar6・war9の保護

標準シナリオ600か月（実装後、WAR行はP28-008-PRE-001の候補B一時パッチ結果と、HashSetの表示順（war6の`att`の並び）を除き完全一致）:

| war | 当事国 | 結果 | 備考 |
|---|---|---|---|
| war0 | 2→0 | AttackerVictory 1801/01/30 | 変化なし |
| war1 | 5→0 | WhitePeace 1801/01/30（E1） | 変化なし |
| war2 | 2→3 | AttackerVictory **1801/04/06** | 実装前 04/12（講和前の値に復帰） |
| war3 | 5→3 | WhitePeace **1801/03/22** `Frontline Unreachable` | E2は意図どおり発火 |
| war4 | 1→0 | AttackerVictory 1811/12/05 | 変化なし |
| war5 | 2→5 | AttackerVictory 1802/01/16、開戦**1801/05/07** | 実装前の開戦 05/13（講和前の値に復帰） |
| war6 | [1,2]→0 | **Active** | 国境が残る限り誤終了しない |
| war7 | 3→5 | DefenderVictory 1808/12/05、開戦1806/05/02 | E2自体の前倒し（06/02→05/02）が残る |
| war8 | 5→3 | AttackerVictory 1813/12/10 | 変化なし |
| war9 | 5→3 | **Active** | 国境が残る限り誤終了しない |

通常勝利の種類・war6/war9の継続はいずれも不変。

## 7. 36・120・240・600か月の比較（標準シナリオ、固定シード・全AI・プレイヤー操作なし）

| 時点 | 累計War | 継続中 | 終結 | battles_ever | battle_days | 別Warへのcollateral |
|---|---|---|---|---|---|---|
| m36 (1803/01/01) | 6 | 1 | 5 | 23 | 34 | 0 |
| m120 (1810/01/01) | 8 | 2 | 6 | 23 | 34 | 0 |
| m240 (1820/01/01) | 10 | 2 | 8 | 24 | 34 | 0 |
| m600 (1850/01/01) | 10 | 2 | 8 | 24 | 34 | 0 |

実装前（P28-007 HEAD）は battles_ever が1つ多かった（m36=24, m600=25）。これはwar2が失った占領を取り戻すために生じた余分な戦闘で、collateralの消滅とともに無くなった。占領州数・戦闘数・Division移動は上記のとおり。国別の月次財政（FIS行、6か国×600か月=3600行）は、P28-008-PRE-001の候補B一時パッチ結果と完全一致し、P28-007以前の基準（B0）とも、`atwar`フラグの3行を除いて完全一致した。その3行はいずれもE2自体の意図した効果である（1801/04/01 c5: war3がE2で03/22に終結しているため非交戦、1806/06/01 c3・c5: war7の開戦が05/02へ前倒しされたため交戦中）。軍需・募兵・Divisionに関する月次行（MON行）は全行がB0と一致した。補給の日数指標（`unsupplied_attacker_battle_days`）は全時点で0。

自然運用は固定された標準シナリオ1本であり、複数シードは検証していない。因果の主たる証拠は複数Warの制御シナリオ（§4.1・§4.2・§5）であり、自然運用はその実在を確認する位置づけである。

## 8. `p27_002_c`の正規値と因果コメント

`tests/p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs`の`M120_EXPECTED`は、実装後の全体実行で失敗した唯一の既存テスト（2773件中1件）。変更は次の2か国のみで、原因を特定した（機械的な更新ではない）。

| 国 | P28-007の値（変更前） | P28-005の値（今回戻した値） |
|---|---|---|
| Dwarf | (0.140, 15156.42, 232.72) | (0.140, 15453.41, 206.56) |
| Oceanic | (0.120, 4759.42, 35.54) | (0.130, 4695.83, 113.36) |

Arcadia・Elfin・Passguard・Ferrowynは変更なし。月120の6か国すべての(tax, treasury, balance)がP28-005時点の値と完全に一致する。

**消えた正常化**: 1801/03/22のwar3（Ferrowyn対Oceanic）のE2白紙講和が、Active中のwar2（Dwarf対Oceanic）がOceanic所有のs9に持っていた占領を消していた。war2はs9を再占領する必要があり、終結が1801/04/06→04/12に6日遅れ、以降のDwarf・Oceanicの財政軌道が変わっていた。今回の限定により、この正常化は発生しない。

**コメントの訂正**: P28-007が書いた「15日の前倒しがAI意思決定タイミングへ連鎖した」という因果説明は誤りだったため、上記の実測に基づく因果に書き換えた（P28-007の旧報告書は変更していない）。E2自体の効果として月120の財政値に影響するものは無く、残るのはwar3の終結日とwar7の開戦日（06/02→05/02）のみ。

## 9. Save・決定性・国家間分離

- **Save V1**: 維持。判定は`owner`・`controller`・Warの参加国・Warの`status`だけで、すべてSave V1に含まれる。新規永続フィールド・migrationなし。`state.war_id`は参照しない（旧Saveでも意味が変わらない）。
- **save→load**: 標準シナリオで、講和（1801/03/22）の**直前**（03/21にsave→load→連続load）と**直後**（03/22にsave→load）のそれぞれで、1801/04/13時点のWar・州・Divisionの全状態が連続実行と一致（`save_load_around_the_peace_matches_continuous_run`）。
- **決定性**: 標準シナリオ600か月を独立した2プロセスで実行し、SUMMARY・WAR・FIS行が一致した。差分は診断ハーネスの表示順だけ（`HashSet`の`att=[1, 2]`/`[2, 1]`、同日の2行の並び、`ended_today=[0, 1]`/`[1, 0]`）で、ゲーム状態は同一。新規テスト2ファイルも独立した2プロセスで同じ結果。
- **国家間分離**: 恒久テストで、別Warの占領・Division・戦闘が、当事国の共有の有無、割譲、国家消滅、降伏のいずれの経路でも変化しないことを確認した。国家消滅時の`apply_country_elimination`の占領整理（`controller==消滅国`のみ）は変更していない。

## 10. 既存テストへの影響

- 実装前後の全体比較で変化した既存テストは`p27_002_c`の1件のみ（原因・値は§8）。`p26_008_b`・`p27_005`・`p21_*`・`p27_fu_*`・`p28_002/003/005`・`p23_006`（2引数の`relocate_hostile_territory_divisions`を直接呼ぶ）・`daily_system_integration_test`は無変更で通過。
- **`p26_008_b`のElfin隔離**: 一時診断（隔離呼び出しを外したコピー）で、新実装では19件すべて通過することを確認した。ただし本タスクの修正に不可欠ではないため、`p26_008_b`は変更していない。隔離ヘルパーとそのコメント（「`execute_peace_settlement`の既存の仕様に起因する副作用」）は新実装では実態に合わなくなったため、別件として整理を推奨する（§12）。

## 11. 保護範囲の遵守

P28-007のE2判定、降伏条件、戦闘・補給・AI募兵の式、Railway・戦時MilitaryFactory建設AI、国境・地形データは変更していない。`peace.rs`以外の本番コードは変更していない。P28-004の平時過剰募兵とwar9型の停止は解決済みと判定していない（war6・war9は従来どおり継続）。

## 12. 未解決事項

1. **連鎖占領（Low）**: 所有国が終結Warの参加国でない占領（例: W所有の州をYがWar Eで占領し、XがWar AでYから奪取）は、帰属が現行データから一意に決まらないため、任意のWarへ帰属させず従来どおり残留する。恒久テスト`state_whose_owner_is_not_a_party_of_the_ended_war_is_left_alone`が現状を固定する。自然運用で未観測。必要になれば、占領経路で占領元Warを記録する別契約（候補C）が必要。
2. **Division帰還の全体走査（Low、既存）**: どのWarにも関与しない国のDivisionでも、外国領にいれば、いずれかのWarの終結で自国領へ移動される（旧実装から不変）。
3. **反復順の未ソート（Low）**: ステップ3・3.5は`HashMap`反復順のまま。テストした範囲では結果が順序に依存しないが、WarId昇順による決定論化は別の衛生対策。
4. **`p26_008_b`の隔離ヘルパーとコメントの整理（Low）**: §10。
5. **E2が宣戦と同日に成立し得る点（既知、P28-007報告書で訂正済み）**: 変更なし。
6. **war9型停止・P28-004（未解決のまま）**: 本タスクの対象外。

## 13. Critical / High / Medium / Low

- **Critical**: なし。
- **High**: なし（P28-008-PRE-001のHigh「同日2降伏で勝利が1件失われる」は解消）。
- **Medium**: なし（別Warの占領・戦闘中Divisionの消去と、支持国所有州の残留は解消）。
- **Low**: §12の1〜4。

**次の原子的タスク候補**: Phase 28の残課題はP28-004（平時過剰募兵・war9型停止、ユーザー判断待ち）。本タスク由来の小さな整理候補は§12の4（`p26_008_b`の隔離ヘルパー削除とコメント更新）と3（反復順のソート）で、いずれも優先度は低い。

## 14. 実行コマンドと結果

**恒久テスト（新規、18件）**:
- `tests/p28_008_concurrent_war_peace_scope_test.rs`（15件、レジストリレベル、実`execute_peace_settlement`・実`evaluate_war_capitulation`）
- `tests/p28_008_concurrent_war_peace_scope_e2e_test.rs`（3件、実App・実Systemと実マップ）

**一時診断（コミットしていない）**: 候補Bの一時パッチ版診断ハーネス（600か月、占領分類・Division移動分類）、実App制御シナリオ、`p26_008_b`隔離なしコピー。いずれも削除済み。実装後の診断では`peace.rs`のみを変更した状態でハーネスを実行している（環境変数による切替なし）。

**個別実行（`rg --files tests`で実名確認、最終コードで実行）**: lib `war::`（191 passed）、`p28_008_concurrent_war_peace_scope_test`（15）、`p28_008_concurrent_war_peace_scope_e2e_test`（3、2プロセスで各1回）、`p21_019`（4）、`p21_021`（1）、`p21_016`（1）、`p28_005`（3）、`p28_003`（2）、`p28_002`（3）、`p27_fu_001_elimination_peace`（3）、`p27_fu_001_ai_retreat`（3）、`p27_007`（2）、`p27_005`（10）、`p26_008_b`（19）、`p26_008_a`（30）、`p27_002_c`（20）、`p23_006`（22）、`land_war_combat_peace_test`（4）、`p21_005`（2）、`p21_save_003`（4）、`p26_006_rev_001`（9）、`daily_system_integration_test`（6）、`diplomacy_tests`（5）。すべて0 failed。

**Cargo逐次実行（最終コードの実測）**:
```
cargo test --all-targets -j 2
→ 2773 passed; 0 failed; 1 ignored   （前タスク基準 2755 + 新規18）

cargo clippy --all-targets --all-features -- -D warnings
→ warning 0件
```

**フォーマット**: 変更した3ファイル（`src/war/peace.rs`の自分の追加箇所、新規テスト2ファイル）だけに`rustfmt`を適用した。ワークスペース全体のformatは行っていない。`tests/land_war_combat_peace_test.rs`と`saves/savegame_v1.ron`には触れていない。非決定的PNG 9件は、全体実行のたびに再生成を確認し、対象を確認してファイルごとに`git checkout --`で個別復元した。`git add .`・`git add -A`・push・tag・amend・rebase・reset・stashは使用していない（負の対照では`git diff`で退避した差分を`git checkout -- src/war/peace.rs`で一時的にHEADへ戻し、検証後に`git apply`で復元した）。

## 15. 最終 `git status --short`（コミット直前）

```
 M src/war/peace.rs
 M tests/p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs
?? tests/p28_008_concurrent_war_peace_scope_e2e_test.rs
?? tests/p28_008_concurrent_war_peace_scope_test.rs
?? verification_logs/phase-28/p28-008/
```

（コミット後の`git status --short`が出力なしであることは、コミット後に実測して最終報告で示す。）

P27-005の手動プレイ確認は今回も未実施であり、引き継ぎ事項として残す。
