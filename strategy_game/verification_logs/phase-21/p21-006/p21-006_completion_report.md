# P21-006 完了報告書: Army単位の前線割当を既存の防御配置処理へ接続

## 1. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

Army(編成)の前線割当が、既存の`process_defensive_plan`(防御配置)経路へ接続され、Army経由の
Divisionが直接割当と全く同じロジックで前線地域へ配置されるようになった。新しい移動アルゴリズム・
新しい永続状態・新しいセーブフィールドは一切追加していない。自動検証(lib 478件 + 安全な統合テスト
71件 = 549件、clippy 0警告、release build成功)は全てグリーン。ただし実`cargo run`によるGUI手動確認は
本セッションでは実施できていない(セクション7参照、確認済みとは偽らない)。

## 2. 環境情報(作業開始時点)

- `rustc -V`: `rustc 1.97.1 (8bab26f4f 2026-07-14)`
- `cargo -V`: `cargo 1.97.1 (c980f4866 2026-06-30)`
- `rustfmt -V`: `rustfmt 1.9.0-stable (8bab26f4f6 2026-07-14)`
- `rust-toolchain` / `rust-toolchain.toml`: 存在しない
- `cargo fmt --all -- --check`(開始時点): **68 diffハンク、20ファイル**(P21-005終了時点と同一。
  内訳は本報告書セクション6参照)

## 3. 事前調査結果

実装前に以下を実コードで確認した(コード変更なし)。

### 1. Division単位の前線割当から移動要求が生成される全経路
`process_defensive_plan`(Defend)と`process_offensive_plan`(Offensive、内部で
`process_defensive_plan`を呼んでから独自の攻撃対象選定を行う)の2箇所のみが
`division.destination`/`current_path`/`status = Moving`を書き換え、
`frontline_registry.frontline_generated_movements`へ追加する。`process_stopped_plan`は
新規移動を生成せず、既存のfrontline生成移動だけを取り消す。

### 2. process_daily_frontline_plansとprocess_defensive_planの実行順序・実行頻度
`handle_daily_frontline_plans`(`DailySimulationSet::FrontlineOrders`、`DayChangedMessage`ごとに
1回)→`process_daily_frontline_plans`→(Frontlineごと×attacker/defender国ごとに)
`plan.stance`に応じて`process_stopped_plan`/`process_defensive_plan`/`process_offensive_plan`の
いずれか1つを呼ぶ。`FrontlineOrders`は`MilitaryAction`(Army/Division消滅処理)より前。

### 3. assigned_division_idsとdivision_frontline_mapの正規不変条件
`assign_division`が両方を単一操作で同時更新(1 Division→最大1 Frontline)。
P21-005で追加された`assigned_army_ids`/`army_frontline_map`も同型の不変条件
(1 Army→最大1 Frontline)を持つが、これらは相互に独立しており、P21-005時点では
Army側の情報がDivision側の移動生成処理から一切参照されていなかった(今回接続する対象そのもの)。

### 4. ArmyRegistryのmember_division_idsとdivision_army_mapの整合性
`ArmyRegistry::sanitize_references`(日次、`MilitaryAction`セット、`FrontlineOrders`より後)が
消滅済みDivisionを`member_division_ids`/`division_army_map`から整理する。よって
`FrontlineOrders`時点の`member_division_ids`には最大1日分の遅延が生じうるが、
`process_defensive_plan`側の既存ロジックが未存在Division(`military_registry.divisions.get`が
`None`)・戦力0・撃破済みDivisionを安全に`continue`でスキップする既存ガードがそのまま機能するため、
新たなpanicや不整合は発生しない(既存Division直接割当でも従来から同じ許容範囲)。

### 5. 前線解除、Army解散、DivisionのArmy加入・離脱、和平時cleanupの経路
P21-005で実装済みの`unassign_army`/`sanitize_army_references`(`ArmyRegistry`変化時に毎フレーム
反応)/`remove_frontline`(War終了時に`execute_peace_settlement`から必ず呼ばれる唯一の経路)が
`army_frontline_map`/`assigned_army_ids`を清掃する。今回はこれらの経路を変更していない
(実行時解決のみを追加したため、追加のcleanup経路は不要)。

### 6. frontlinesのowner/country検証
`assign_army`が割当時点で`army.owner == country_id`かつ`country_id`がFrontlineの参加国
(attacker/defender)であることを検証済み。今回追加する実行時解決処理でも、念のため
`army.owner == plan.commanding_country_id`を再検証する防御的チェックを追加した
(通常API経由では発生し得ないが、他手段での不正状態注入に対する多重防御)。

### 7. Army割当とDivision直接割当が競合した場合の既存仕様
**既存仕様は存在しなかった**(P21-005時点でArmy割当はどの移動生成処理からも一切参照されていない
ため、「競合」という概念自体が存在しなかった)。よってタスク指示に明記された後方互換規則
(直接割当優先・複製禁止・実行時解決)をそのまま採用した。実コードの不変条件との衝突は
見つからなかった。

## 4. 採用した有効前線の解決規則

`src/war/frontline.rs`へ2つの純粋関数を追加した。

```rust
/// 単一Divisionの「有効な前線」を解決する(直接割当が最優先、無ければ所属Armyの
/// 前線割当を継承)。O(1)のHashMap参照のみ、全件走査なし。
pub fn resolve_effective_frontline_for_division(
    division_id: DivisionId,
    frontline_registry: &FrontlineRegistry,
    army_registry: &ArmyRegistry,
) -> Option<FrontlineId>

/// 特定の(Frontline, Country)Planについて、防御配置処理の対象となるDivisionId集合を
/// 実行時に解決する。直接割当(assigned_division_ids)に加え、plan.assigned_army_ids
/// (このplan専用に絞り込み済み)経由でArmy所属Divisionのうち直接割当のないものを合流させる。
/// DivisionId昇順・重複無しで返す。
pub fn resolve_effective_division_ids_for_plan(
    plan: &FrontlinePlan,
    army_registry: &ArmyRegistry,
    division_frontline_map: &HashMap<DivisionId, FrontlineId>,
) -> Vec<DivisionId>
```

- 優先順位: Divisionの直接前線割当が常に最優先。直接割当のないDivisionだけが所属Armyの
  前線割当を継承する。
- 二重管理なし: `division_frontline_map`/`assigned_division_ids`/`army_frontline_map`/
  `assigned_army_ids`のいずれも複製・書換えしない。全て既存のP21-005データをそのまま参照する
  純粋な読み取り専用計算。
- 不必要な全件走査を避ける: `plan.assigned_army_ids`は`assign_army`によって既にこのplan専用に
  絞り込まれているため、全Army・全Divisionを走査せず、対象Armyの`member_division_ids`
  (`ArmyRegistry.member_division_ids`)だけを見る。単一Division向けの
  `resolve_effective_frontline_for_division`は`division_army_map`/`army_frontline_map`を
  直接O(1)参照する。
- 決定的な順序: 結果は常に`DivisionId`昇順・`dedup()`済み。Army側も`plan.assigned_army_ids`を
  `ArmyId`昇順で処理してから合流するため、HashMapの反復順には一切依存しない。

## 5. 接続範囲(スコープ境界)

- `process_defensive_plan`: 内部の対象Division列挙を`plan.assigned_division_ids`から
  `resolve_effective_division_ids_for_plan(...)`の結果へ差し替えた(早期returnの空判定も
  解決後リストで行うよう修正)。
- `process_offensive_plan`: 内部で呼ぶ`process_defensive_plan`(前線への分散配置)には
  Army経由分も自然に合流する。**攻撃対象選定ループ自体は意図的に変更していない**
  (`plan.assigned_division_ids`のみを見る既存のまま)。Army経由Divisionを新しい攻撃・
  戦闘開始の対象に含めないための明示的なスコープ境界。
- `process_stopped_plan`: 対象Division列挙も同じ解決関数を使うよう拡張した(Defend/Offensive時に
  Army経由で生成された移動を、Stopped切替時に正しく停止できるようにするための対称性維持。
  新しい移動やAttack命令を生成する変更ではない)。
- `process_daily_frontline_plans`/`handle_daily_frontline_plans`: `army_registry`引数を追加し、
  上記3関数へ受け渡すだけ。

## 6. 変更ファイル一覧

**`src/war/frontline.rs`のみ**(既存ファイル編集。新規ファイルは作成していない)。

## 7. 追加テスト一覧と正確な増加件数

`src/war/frontline.rs`へ13件の新規テストを追加した(net +13)。うち1件は既存テストの
仕様変更に伴う置き換え。

| # | テスト名 | 要求テスト項目 |
|---|---|---|
| 1 | `test_army_assignment_with_defend_stance_generates_defensive_placement_p21_006` | 1(Army割当済み・直接割当なしのDivisionが防御配置対象になる) |
| 2 | `test_army_assignment_alone_without_stance_change_does_not_move_division` | (P21-005由来の不変条件をP21-006後も真である狭い形で維持。既存テストの改名・置換) |
| 3 | `test_resolve_includes_army_member_without_direct_assignment` | 1 |
| 4 | `test_resolve_without_army_assignment_matches_existing_behavior` | 2(Army未割当なら既存挙動のまま) |
| 5 | `test_resolve_direct_assignment_takes_priority_over_army_inheritance` | 3(直接割当が優先) |
| 6 | `test_direct_and_army_assignment_to_same_frontline_is_not_double_processed` | 4(同一前線でも二重処理されない) |
| 7 | `test_adding_division_to_assigned_army_includes_it_from_next_processing` | 5(Army加入は次回処理から反映) |
| 8 | `test_removing_division_from_assigned_army_excludes_it_from_next_processing` | 6(Army離脱は次回処理から反映) |
| 9 | `test_unassigning_army_frontline_stops_generating_new_placement` | 7(前線解除後は新規生成なし) |
| 10 | `test_multiple_armies_on_same_frontline_process_deterministically` | 8(複数Armyでも決定的) |
| 11 | `test_resolve_excludes_army_with_owner_country_mismatch` | 9(owner/country不一致を通さない) |
| 12 | `test_army_driven_placement_survives_ron_round_trip` | 10(セーブ/ロード後も再現) |
| 13 | `test_old_format_restored_plan_generates_no_army_driven_placement` | 11(旧セーブでは配置なし) |
| 14 | `test_process_daily_frontline_plans_is_safe_after_frontline_removed` | 12(前線削除後に参照が残らない・安全) |

項目13(既存Division単位前線テストがすべて維持される)は新規テストの追加ではなく、既存テスト群
(`test_defensive_positioning_and_determinism`/`test_offensive_operations_objective_and_attack`/
`test_manual_vs_frontline_priority`/`test_division_assignment_and_validation`等)が全てそのまま
greenであることで満たしている。

### 既存テストの仕様変更に伴う改名(削除ではない)

`test_army_assignment_alone_does_not_trigger_frontline_orders_automation`(P21-005で追加)は
「Army割当だけでは(Defendスタンスを設定しても)自動移動は一切発生しない」ことを検証していたが、
これはP21-006が意図的に変更する対象そのものだった。**削除はせず**、以下の2テストへ分割・更新した。

- `test_army_assignment_with_defend_stance_generates_defensive_placement_p21_006`:
  同じセットアップで、P21-006後の正しい挙動(Division が自国側前線へ移動する)を検証する
  よう更新。
- `test_army_assignment_alone_without_stance_change_does_not_move_division`: P21-005当時の
  不変条件のうち、P21-006後も真であり続ける部分(スタンスを明示的に変更しない限り、
  `assign_army`という操作自体は何も動かさない)を、新規テストとして分離・維持した。

正味のテスト数はこの1件の改名で net +1(旧1件→新2件)、他13件は完全新規。

## 8. 全検証コマンドの結果

- `cargo check --all-targets`: ✅ 成功
- `cargo test --lib`: ✅ **478 passed**(P21-006開始前466件 → +12。改名分+1と合わせて
  合計+13、セクション7参照)
- `cargo test --tests`相当(安全な統合テスト11バイナリ実行): ✅ **71 passed, 0 failed**
  (daily_system_integration/diplomacy/economy/land_war_combat_peace/
  p20_009_hardcoded_string_scan/p20_009_localization_resource/p21_005_army_frontline_e2e/
  p21_save_002e_end_to_end/p21_save_003_end_to_end/profile_workload_correctness/
  research_and_politics)
- headless-render系3バイナリ: 既存の証拠保護方針に従い`--no-run`でコンパイルのみ確認、実行せず
- `cargo clippy --all-targets --all-features -- -D warnings`: ✅ 0警告
- `cargo build --release --all-targets`: ✅ 成功(6分39秒)
- `git diff --check`: ✅ 空白関連の実質的な問題なし(LF/CRLF変換警告のみ)
- 一時ディレクトリ・プロセス残留: なし確認済み(`tasklist`/temp dir grep共に空)
- 実7か国28州データを使う既存E2E経路(`tests/p21_005_army_frontline_e2e_test.rs`)を
  **維持したまま**再実行し、2件とも変更なくgreenを確認(P21-006はこのE2Eのシナリオでは
  Plan.stanceがStopped[既定値]のままのため、挙動に変化がないことも確認できた)。

## 9. fmtベースライン不一致の扱い

- **開始時点**: `cargo fmt --all -- --check` = 68 diffハンク、20ファイル
  (`loader.rs`/`country_ai.rs`/`division_render.rs`/`map/mod.rs`/`movement.rs`/
  `recruitment.rs`/`supply.rs`/`military/tests.rs`/`profiling.rs`/`save/runtime.rs`/
  `country_selection.rs`/`peace_panel.rs`/`capitulation.rs`/`war/military_ai.rs`/
  `war/peace.rs`/`war/tests.rs`/`daily_system_integration_test.rs`/
  `land_war_combat_peace_test.rs`/`p21_save_003_end_to_end_test.rs`/
  `profile_workload_correctness_test.rs`)。P21-005終了時点と完全一致。
- **今回変更したファイル**: `src/war/frontline.rs`の1ファイルのみ。`rustfmt --edition 2024
  src/war/frontline.rs`(このファイルは`mod`宣言を含まないリーフファイルのため、モジュール
  ツリー全体を巻き込む懸念なし。`git diff --stat`で他ファイルへの影響が皆無なことを直接確認済み)
  で個別整形し、整形後は同ファイル単独のdiffが0になったことを確認。
- **終了時点**: `cargo fmt --all -- --check` = **68 diffハンク、20ファイル(開始時と完全一致)**。
  今回変更した`frontline.rs`はこのリストに含まれない(整形済みのため)。開始前からの
  fmt不一致(20ファイル)と今回の変更による回帰(0件)は完全に分離されている。
- `cargo fmt`をリポジトリ全体・`main.rs`/`lib.rs`/`mod.rs`へは一切実行していない。

## 10. 実GUIで確認すべき項目

本セッションでは実ウィンドウ操作ができないため、以下は**未実施**として正直に報告する。

1. `cargo run`でActiveな戦争を発生させ、Armyを編成する
2. Armyを前線へ設定し(P21-005 UI)、Defendスタンスへ切り替える(前線命令ボタン、既存P21-002 UI)
3. Army所属のDivisionが、直接前線割当のDivisionと同様に自国側前線地域へ自動的に移動を開始する
   ことを目視確認する
4. ArmyへDivisionを追加/除外した際、次の日次更新で移動対象が動的に増減することを確認する
5. 「前線を解除」ボタンでArmy割当を解除した際、それ以降そのArmyのDivisionが新たに自動移動しない
   ことを確認する
6. 直接割当のあるDivisionと、Army経由のDivisionが同じ前線に混在していても、UI上の表示・
   移動指示が破綻しないことを確認する
7. 攻勢(Offensive)スタンスでも、Army経由のDivisionが自国側前線への分散配置までは行われるが、
   自動攻撃の対象にはならないことを確認する(スコープ境界の目視確認)
8. セーブ→ロード後もArmy経由の配置動作が継続することを確認する
9. ウィンドウ終了後にプロセスが残らないことを確認する

自動テスト(549件)によって、ロジック・データ・実行順序・優先順位・セーブ互換性・スコープ境界の
いずれも検証済みだが、実際のマウス操作・実際の日次シミュレーション進行を伴う画面上の確認では
ない。ユーザーによる実`cargo run`確認後、判定を`COMPLETE`へ更新することを推奨する。
