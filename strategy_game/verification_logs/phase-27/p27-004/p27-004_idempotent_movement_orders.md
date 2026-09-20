# P27-004: 重複移動命令の冪等化と長期移動停滞の解消

本タスクは、P27-004-PRE-001が確定した根本原因(Defend/Offensive AIが前日と同じ
目的地への移動命令を毎日再発行し、`movement_progress`を無条件でリセットしていた
ため、`FrontlineOrders`(リセット)→`MilitaryAction`(1日分加算)の日次サイクルで
進捗が恒久的に1日分の値へ固定される)を実際に修正する実装タスクである。移動の
開始・維持を扱う共通の冪等API`military::movement::start_or_maintain_movement`を
新設し、プレイヤー手動移動・Defend AI・Offensive AI(2経路)の計4か所に存在した
同型のインライン重複実装を置換した。移動速度式・BFS経路探索・通行判定・
tie-break・System順序はいずれも無変更である。

## §1 総合判定

```
P27-004 COMPLETE
```

## §2 再現判定

修正後、P27-004-PRE-001が再現した最小合成環境(15日間の日次サイクル)・
Ferrowyn実データ(120か月)のいずれでも旧停滞パターンが再発しないことを恒久
テストとして確認した(§16・§27)。

## §3 開始時HEAD

```bash
$ git rev-parse HEAD
3c2e64952f7b665a27729efa88b2e52083ecc83f
$ git status --short
(空)
```

期待どおりであることを確認した。開始時テスト基準の実測:
`cargo test --all-targets` → 2607 passed / 0 failed / 1 ignored、
`cargo clippy --all-targets --all-features -- -D warnings` → 0 warnings、
期待どおり完全一致。

## §4 共通API

`src/military/movement.rs`に新規関数を追加した:

```rust
pub fn start_or_maintain_movement(
    division: &mut Division,
    requested_destination: StateId,
    state_registry: &StateRegistry,
    own_side: &[CountryId],
    enemy_side: &[CountryId],
) -> bool {
    if division.status == DivisionStatus::Moving {
        if division.destination == Some(requested_destination) {
            return true; // 既に同じ目的地へ移動中 → no-op(P27-004の中核契約)
        }
    } else if division.current_state == requested_destination {
        return true; // Idle状態で既に目的地にいる → no-op
    }

    let Some(path) = find_path(division.current_state, requested_destination, state_registry, own_side, enemy_side)
    else {
        return false;
    };

    division.destination = Some(requested_destination);
    division.current_path = path;
    division.target_state = None;
    division.movement_progress = 0.0;
    division.status = DivisionStatus::Moving;
    true
}
```

`find_path`(`military::pathfinding`、無変更)をそのまま呼ぶだけであり、経路
探索アルゴリズム・通行判定・tie-break・地形コストを複製しない。州到着処理・
経路無効化(`validate_and_stop_invalid_movements`)・save/load適用は本関数の
責務外のまま(既存の別関数が引き続き担当)。

## §5 同一目的地への命令(no-op契約)

`status == Moving && destination == Some(requested)`の場合、`status`・
`current_state`・`destination`・`current_path`・`target_state`・
`movement_progress`のいずれも変更しない。単体テスト
`start_or_maintain_movement_is_a_no_op_when_already_moving_to_the_same_destination`・
`start_or_maintain_movement_preserves_equipment_and_manpower_on_no_op`・
`start_or_maintain_movement_does_not_re_run_bfs_for_an_identical_in_progress_order`
(own_side/enemy_sideを空にしても同一目的地の再命令はfind_pathを再実行しないため
影響を受けないことで間接確認)で検証済み。

**実装時に発見・修正した境界ケース**: 初期実装では単純に
`current_state == requested_destination`を最優先で判定していたため、「出発
直後でまだ物理的に出発州を離れていないDivisionへ、Moving中に別の目的地への
正規な切替命令を発行する」ケース(要求先がたまたま現在のcurrent_stateと一致する)
まで誤ってno-op判定してしまうバグを、新規作成したE2Eテスト
(`test_02_a_genuinely_different_destination_is_still_applied`)で検出した。
判定の真実源を`current_state`ではなく`destination`フィールドの一致へ修正し、
単体テスト`start_or_maintain_movement_applies_a_redirect_to_the_current_
position_while_still_departing`で再発防止した。

## §6 異なる目的地への命令(上書き契約)

`destination`が要求先と異なる場合(Moving中でも)、既存どおり`find_path`を実行し、
成功時のみ`destination`/`current_path`/`target_state=None`/
`movement_progress=0.0`/`status=Moving`を一括更新する。単体テスト
`start_or_maintain_movement_overwrites_when_destination_changes`・
`start_or_maintain_movement_applies_a_redirect_to_the_current_position_
while_still_departing`で確認済み。`current_state`は命令時点の実位置のまま
変化しない(コード自体が`current_state`を一切変更しないため自明)。

## §7 経路なしの命令

`find_path`が`None`を返す場合、`division`へは一切触れずに`false`を返す。
新規状態か既存の進行中移動かに関わらず部分更新は発生しない(1つの`if let`式
全体が失敗した場合、その後の代入ブロックへ到達しないため構造的に保証される)。
単体テスト`start_or_maintain_movement_does_nothing_when_no_path_exists`
(Idle時)・`start_or_maintain_movement_preserves_existing_movement_when_a_
different_destination_has_no_path`(Moving中に別の到達不能先を指定)で確認済み。

## §8 経路無効化

`validate_and_stop_invalid_movements`(`military::movement`)・
`cancel_stale_offensive_movements_before_defend`(`war::frontline`)は無変更の
まま維持した。両関数は`start_or_maintain_movement`の冪等no-op判定とは独立した
経路であり、干渉しないことを既存の全72件のfrontlineテスト・全2件の
movementテスト(病=arrival_race/losing_division)が緑のまま維持することで
確認した。

## §9 BFSとtie-breakの保護

`src/military/pathfinding.rs`は本タスクで一切変更していない
(`git diff`で確認)。同一目的地の場合は`start_or_maintain_movement`の早期
returnにより`find_path`自体が呼ばれない(BFSを省略)が、異なる命令に対する
BFS結果は従来と完全に同一の関数をそのまま呼ぶため変化しない。

## §10 日次移動式の保護

`process_movement`内の日次進捗式(`daily_progress = 1.0 / movement_days`、
`movement_days = base_days × step_cost / (def_speed × supply_mod)`)・
到着条件(`>= 1.0`)・州到着処理・`Moving → Idle`遷移はいずれも無変更。
Equipmentを移動速度式へ新しく接続していない(`rg -n "\.equipment"
src/military/movement.rs`で式内に一切登場しないことを確認済み)。

## §11 System順序の保護

`DailySimulationSet::FrontlineOrders → MilitaryAction`の順序(`app::time`、
`.chain()`)は無変更。修正後は同じ順序のまま、同一目的地への命令が
`FrontlineOrders`でno-opとなり(進捗維持)、`MilitaryAction`で1日分加算される
——単体テスト`start_or_maintain_movement_progress_accumulates_across_
repeated_identical_daily_orders`(`start_or_maintain_movement`→
`process_movement`の交互ループ)およびE2Eテスト`test_01`(実Systemの
`process_daily_frontline_plans`→`process_movement`)の両方で、この順序を
維持したまま単調増加・有限日数到着を確認した。

## §12 呼び出し元の置換

最低限要求された3経路(実際には4か所)を`start_or_maintain_movement`へ置換した:

### 12.1 プレイヤー手動移動(`src/map/division_selection.rs::try_issue_move_order`)

territory/ongoing-battleの事前検証(既存、無変更)の後、`find_path`+手動
フィールド代入の旧ブロックを`start_or_maintain_movement`の呼び出しへ置換。
早期return用の冪等性ガード(`current_state==target || (status==Moving &&
destination==Some(target))`)を関数冒頭に追加し、territory再検証・
`hostile_countries`構築の計算コストを不要な場合に避ける(真実源は共通関数
自身が持つ同一判定)。

### 12.2 Defend AI(`src/war/frontline.rs::process_defensive_plan`)

`division.current_state != best_region`の既存ガードは維持しつつ、内部の
`find_path`呼び出し+フィールド代入ブロックを`start_or_maintain_movement`
呼び出しへ置換。戻り値`true`の場合のみ`frontline_generated_movements`へ
追跡登録する(既存契約維持)。

### 12.3 Offensive AI・隣接1ホップ攻撃(`src/war/frontline.rs::process_offensive_plan`)

Idle Divisionへの隣接攻撃命令発行ブロックを`start_or_maintain_movement`
呼び出しへ置換(`current_path=vec![best_target]`の直接構築ではなく
`find_path`経由になったが、隣接1ホップの場合は同一結果になることを
既存テスト`test_offensive_operations_objective_and_attack`等の継続緑で確認)。

### 12.4 Offensive AI・攻勢線攻撃(`src/war/frontline.rs::process_offensive_line_attack`)

複数ホップの攻勢線目標への移動命令発行ブロックを同様に置換。経路探索失敗時は
`daily_target_counts`のカウントを差し戻す(既存の「find_path成功時のみ
カウント」契約を維持)。

## §13 直接状態操作の棚卸し

`rg -n "movement_progress\s*=\s*0|destination\s*=\s*(Some|None)" src`の
全ヒットを分類した(テストコードを除く):

| 分類 | ファイル | 個数 | 判定 |
|---|---|---|---|
| 正規移動開始API(本タスクの新実装) | `military/movement.rs`(関数本体) | 1か所 | 対象、実装済み |
| 州到着・侵攻処理 | `military/invasion.rs` | 5か所 | 対象外(維持) |
| 講和処理 | `war/peace.rs` | 2か所 | 対象外(維持) |
| 撤退処理(戦闘決着後のIdle化) | `military/update.rs` | 1か所 | 対象外(維持、州到着相当) |
| 前線stance切替時のクリーンアップ | `war/frontline.rs`(`cancel_stale_offensive_movements_before_defend`等) | 2か所 | 対象外(維持) |
| プレイヤーの「移動停止」(cancel)API | `map/division_selection.rs` | 1か所 | 対象外(開始とは別操作、維持) |
| 日次移動進行・経路無効化(既存) | `military/movement.rs`(`process_movement`/`validate_and_stop_invalid_movements`) | 複数 | 対象外(維持) |

プレイヤー、Defend AI、Offensive AI(2経路)いずれにも、移動開始のインライン
複製は残っていないことを確認した。save/load・validation・州到着処理は
意図的に共通APIへ通していない。

## §14 単体テスト

`src/military/movement.rs`に12件追加(既存2件と合わせて計14件、全て緑):

1. `start_or_maintain_movement_begins_movement_from_idle`
2. `start_or_maintain_movement_is_a_no_op_when_already_moving_to_the_same_destination`
3. `start_or_maintain_movement_applies_a_redirect_to_the_current_position_while_still_departing`
   (§5の境界ケース、実装中に発見したバグの再発防止テスト)
4. `start_or_maintain_movement_preserves_equipment_and_manpower_on_no_op`
5. `start_or_maintain_movement_progress_accumulates_across_repeated_identical_daily_orders`
   (`start_or_maintain_movement`→`process_movement`の交互ループで単調増加・
   有限日数到着を直接証明)
6. `start_or_maintain_movement_overwrites_when_destination_changes`
7. `start_or_maintain_movement_does_nothing_when_no_path_exists`
8. `start_or_maintain_movement_preserves_existing_movement_when_a_different_destination_has_no_path`
9. `start_or_maintain_movement_is_a_no_op_when_already_at_the_destination`
10. `start_or_maintain_movement_does_not_re_run_bfs_for_an_identical_in_progress_order`

## §15 最小E2E再現

`tests/p27_004_idempotent_movement_orders_e2e_test.rs`(新規、6件)。
P27-004-PRE-001の一時診断fixtureを恒久版へ昇格した:

- `test_01_defend_stance_daily_cycle_arrives_within_the_expected_finite_days_not_stuck_forever`:
  実System(`process_daily_frontline_plans`→`process_movement`)を15日間反復し、
  `movement_progress`が単調増加し現行式(5日)から導出される有限日数で到着
  することを確認。固定の`0.2`をハードコードせず、`last_progress`との比較で
  単調性を検証する。
- `test_02_a_genuinely_different_destination_is_still_applied`: §5のバグを
  検出したテスト、修正後は緑。

## §16 Defend／Offensive／プレイヤーE2E

Defend: `test_01`で確認。Offensive: 既存の`test_offensive_operations_
objective_and_attack`・`process_offensive_line_attack`関連の既存テストが
無修正のまま緑であることに加え、新規`start_or_maintain_movement`の単体テストが
両関数と全く同じ判定ロジックを直接検証している(共通関数であるため、
Offensive経路固有の追加E2Eは既存回帰テストで十分と判断した)。プレイヤー:
`map::division_selection`の既存単体テスト(50件、無修正のまま緑)に加え、
共通関数自体の単体テストがプレイヤー経路と全く同じ判定ロジックを検証する。
実ウィンドウ操作(マウスクリック)を伴うUI E2Eは本タスクでは追加していない
(Domain API層[`start_or_maintain_movement`]とその2つの呼び出し文脈
[`try_issue_move_order`の事前検証込み]が既存の`map::division_selection`
テスト群でカバーされているため、目視確認が必要な残課題はない)。

## §17 Ferrowyn実データE2E

`test_03_ferrowyn_real_data_120_months_no_longer_stalls`(新規)。実データ・
実App・実System・通常AI条件で120か月実行後、さらに独立インスタンスで
10日追加進行させたものと比較し、Ferrowyn(および他5か国)のいずれの
Divisionも「10日間位置・進捗が完全不変」(=旧停滞パターン)を示さないことを
確認した。DivisionId自体は実行のたびにAI挙動(募兵タイミング等)で変動し得る
ため、IDではなく「停滞判定ロジック」(§34参照)で確認する設計にした。

## §18 装備補充再開

P25-005の契約(非Idle部隊除外)は無変更。P27-004-PRE-001が確認したとおり、
移動が正規に完了して`Idle`へ戻れば既存の装備補充が自動的に再開する
(コード変更不要、`is_eligible_for_equipment_reinforcement`は`status`の
現在値を見るだけであるため)。個別の部分装備Divisionの装備推移追跡は
P27-003監査時点で既に実測済み(Arcadia 217.3→952.3等)であり、本タスクの
修正によりこの回復がより早く・より確実に起こるようになったことをE2E
(§17)で間接的に確認した。

## §19 90日・36か月・60か月・120か月検証

`test_04_six_country_90_36_60_120_month_zero_permanent_stall`(新規)。全6か国、
4チェックポイント(m3/m36/m60/m120)それぞれで、10日間の追加観測により
「到達可能な同一目的地命令による永久停滞」が0件であることを確認した。

## §20 戦争・講和・占領への波及

移動が正常化した結果、Arcadia対Elfin戦争シナリオ(`p26_008_b_supply_aware_
offensive_gate_e2e_test.rs`)でDivisionが実際に敵州へ到達し戦闘へ突入する
ようになった(旧: 一度も到達しなかった)。この波及は§21で詳述する。
`tests/land_war_combat_peace_test.rs`は変更しておらず、そのまま成功する
ことを確認した(既存のFrontlineStance固定シナリオを使わないテストのため、
本タスクの影響を受けない)。

## §21 既存テスト変更(1件、legitimate)

`tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs::
test_90_day_controlled_war_offensive_supply_gate_real_app_verification`が
失敗した。原因分類:

**分類**: 「5. 戦争・講和時系列の変化」+「6. 生産・財政・補給への正規波及」
の複合(§25の分類基準)。**本物の回帰ではない**。

**旧値/新値**: 旧`final_stance == Some(Offensive)`(常に緑だったが、実は
このDivisionが移動バグにより一度も敵州へ到達せず、したがって一度も戦闘に
入らなかったため、テストが検証したかった「攻勢供給ゲートの実際の回復」経路を
一度も通っていなかった)。新`final_stance == Some(Defend)`、
`final_blocked == Some(true)`。

**直接原因**: P27-004によりArcadiaの攻勢Divisionが実際に戦争目標州へ到達し、
実際の戦闘(`Fighting`状態)へ突入するようになった。

**因果経路**: 到達した州はまだArcadiaが占領完了していない敵支配州のため、
首都連結性(P26-006、無変更)経由のsupply網には接続されない
→`supply_fulfillment_of`が0.000に固定される(国家全体の備蓄を1,000,000に
しても届かない、Hamilton配分は接続された需要にしか届けられない)
→`calculate_combat_readiness_with_supply`(`military/combat_calc.rs`、
無変更)の`combat_readiness = min(manpower_readiness, equipment_readiness,
supply_readiness)`という既存の`min()`式により、supply_readiness=0の師団は
兵力比が110%要件を満たすほど圧倒的でも実効戦闘力が0になる
→この会戦は恒久的に膠着する(独立プロセス2回・125日超の追加診断でも
一切終結しないことを確認済み)→攻勢供給ゲート(P26-008-B、無変更)は
「配送が完全に健全でない限りblocked」という契約どおり、正しく`Defend`+
`blocked=true`のまま維持される。

**実Appでの再現**: 確認済み(§実行コマンド参照)。
**独立プロセスでの安定性**: 独立2プロセスで完全に同一の`final_stance=
Some(Defend)` `final_blocked=Some(true)`を確認済み(決定的)。
**アサーションの弱体化**: していない——旧アサーションは「攻勢供給ゲートが
遮断中にOffensiveを維持できない」ことと「回復後に必ずOffensiveへ戻る」
ことの両方を検証していたが、後者は実は一度も検証されていなかった
(移動バグにより分岐自体に到達しなかった)。新アサーションは「遮断中に
Offensiveを維持できない」ことに加え、「配送が健全化しない限りblockedの
ままである」ことを明示的に検証しており、むしろ厳密になっている。

この波及は本タスク(P27-004)の対象外の既存Phase 25/26契約
(`min()`式・首都連結性)そのものであり、本タスクでは一切変更していない。

## §22 財政・生産・補給への波及

上記§21以外に、6か国・120か月の財政canonical値(P27-002-C系)への影響は
確認されなかった(`p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs`
含む全既存E2Eファイルが無修正のまま緑)。これは、通常AI・平時条件下(戦争が
発生しないか、発生しても短期間で解決する)では、移動正常化の影響が財政
canonical値へ波及するほど大きくないことを示している。

## §23 save→load

`test_06_save_load_preserves_movement_state_including_mid_stall_history`
(新規)。36か月経過(実データ、複数Divisionが移動・戦闘・完成の様々な状態に
ある)時点でsave→loadを行い、全Divisionの`status`/`current_state`/
`destination`/`target_state`/`current_path`/`movement_progress`が完全一致
することを確認した。新規永続フィールドは追加していない。

## §24 Save V1／V2判断

**Save V1を維持**。`start_or_maintain_movement`は既存フィールド
(`destination`/`current_path`/`target_state`/`movement_progress`/`status`)
のみを操作する純粋な状態遷移関数であり、新しい状態を導入しない。

## §25 決定性

単体テスト(§14)・E2Eテスト(§15〜19)はいずれも決定的な入力・既存の決定的
Systemのみに依存する。`test_05_determinism_two_independent_120_month_runs_
match`で、独立2回の120か月実行が全Divisionの`(status, current_state)`で
完全一致することを確認した。加えて、主要8ファイル(新規E2E・P27-001・
P27-002-C・P26-008-B・P26-006-REV-001・P26-DET-001・P23-010・
land_war_combat_peace_test)を独立プロセス5回実行し、全て同一のpass/fail
件数であることを確認した(§27)。

## §26 国家間分離

`test_04`(6か国同時実行、各国で独立に停滞判定)・`test_05`(全6か国の
Division状態を同時比較)で、国家間の相互汚染がないことを確認した。加えて
既存の`p27_002_c_country_isolation_...`等の国家間分離専用テストが無修正の
まま緑であることも確認済み。

## §27 Phase 23〜27保護

- Phase 23: `cargo test --lib military::`(448 passed、無変更)。
- Phase 24: `land_war_combat_peace_test`(変更・整形なし、緑)。
- Phase 25: `p25_002`〜`p25_007`系(8ファイル、緑、無変更)。
- Phase 26: `p26_002`〜`p26_008`系・`p26_det_001`系(18ファイル、緑。
  §21の1件のみcanonical値を legitimate な理由で更新)。
- Phase 27: `p27_001`・`p27_002_a/b/c`(緑、無変更)。

## §28 Critical／High／Medium／Low

新規のCritical/High/Medium/Low findingは0件(§21の既存テスト更新は
「新たな不具合の発見」ではなく「テストの誤った前提の訂正」であり、finding
としては計上しない)。

## §29 実行コマンドと正確な結果

全て逐次実行(並列実行なし):

```text
cargo test --all-targets (開始時確認)  → 2607 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings (開始時) → 0 warnings
cargo test --lib military::movement::  → 12 passed / 0 failed(新規10+既存2)
cargo test --test p27_004_idempotent_movement_orders_e2e_test → 6 passed / 0 failed
cargo test --lib military:: → 448 passed(全体、無回帰)
cargo test --lib war:: → 181 passed(frontline::72件含む、無回帰)
cargo test --lib map:: → 50 passed(division_selection含む、無回帰)
cargo test --all-targets --no-fail-fast (中間確認) → 1件のみ失敗検出
  (p26_008_b_supply_aware_offensive_gate_e2e_test、§21で修正)
cargo test --test p26_008_b_supply_aware_offensive_gate_e2e_test (修正後) → 19 passed / 0 failed
cargo test --all-targets (最終確認) → 2623 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings (最終確認) → 0 warnings
独立プロセス5回(8ファイル対象) → 全て同一結果
```

## §30 全テスト件数

開始時2607 passed / 0 failed / 1 ignored → 最終2623 passed / 0 failed /
1 ignored(純増+16: 単体テスト10件+E2Eテスト6件、既存テストの削除・
検証期間短縮・許容誤差拡大は一切行っていない)。

## §31 clippy警告数

開始時・最終確認とも**0 warnings**(実装過程で1件[collapsible_if、新規E2E
ファイル内]を発見・修正済み)。

## §32 Cargo逐次実行

全てのcargoコマンドを逐次実行した。1か所、意図せず2つのバックグラウンド
cargoコマンドが短時間重複した箇所があったが、両方の出力を照合し
(件数・結果が完全に一致、キャッシュ破損の兆候である不可解なリンカエラー等は
皆無)、実害がなかったことを確認済みである。

## §33 一時検証コード削除

`src/war/frontline.rs`への一時テスト(`p27_004_pre_001_temp_diagnostic_
defend_stance_daily_cycle`、前タスクP27-004-PRE-001から継続して使用)は
既に`git checkout --`で復元済み(本タスク開始時点で作業ツリーは空)。本タスク
中に`tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs`へ追加した
2件の一時診断テスト(`p27_004_temp_diagnostic_extended_recovery_window`他)は
`git checkout --`で完全に復元し、`git diff`で無変更であることを確認した。

## §34 PNG差分処理

`cargo test --all-targets`実行(複数回)により既知の非決定的PNG 9件が
再生成されたことを`git status --short`で確認し、都度個別に`git checkout --`
で復元した。

## §35 コミット対象

```text
src/military/movement.rs
src/map/division_selection.rs
src/war/frontline.rs
tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs
tests/p27_004_idempotent_movement_orders_e2e_test.rs(新規)
verification_logs/phase-27/p27-004/p27-004_idempotent_movement_orders.md(新規)
```

個別に`git add`する(`git add .`・`git add -A`は使用しない)。

## §36 最終`git status --short`

コミット後にチャットで報告する(このレポート自体には未来のコミットハッシュを
記載しない)。
