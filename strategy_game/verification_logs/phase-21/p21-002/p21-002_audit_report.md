# P21-002 完了確認監査レポート(「師団の直接移動・接敵戦闘操作のUI化」)

作成日: 2026-08-10。本レポートは読み取り専用の監査であり、監査対象のソースコードは一切変更していない。
発見した不具合・仕様不整合は修正せず報告のみに留めた。作業開始時点で既にdirtyだった変更
(前セッションで実装したP21-001・「前線命令のボタンUI化」・外交パネルのバグ修正)はユーザーの
変更として一切上書き・復元していない。

## 1. 最終判定

**INCOMPLETE(仕様不整合2件、ユーザー判断が必要)**

自動検証(コンパイル・テスト・clippy・fmt・保護対象ファイル)はすべてPASSしており、
「自動検証上の問題」は存在しない。しかし本タスクの依頼書に列挙された10仕様のうち、
以下2件は**現在のコードで満たされていない**:

- 仕様1「攻撃・防衛モードを選択するUIは存在しない」→ **満たしていない**。前セッションで
  実装した前線命令ボタン(`military_panel.frontline_defend_button` / `frontline_offensive_button`)
  が、まさに「防衛」「攻勢」という攻撃/防衛モードを選択するUIそのものである。
- 仕様8「停止命令が機能する」→ **部分的にのみ満たす**。前線に割り当てられた師団の
  自動生成移動(`frontline_generated_movements`)は停止ボタンで止まるが、右クリックで
  手動発行した移動命令を止める汎用の「停止」命令はコード上どこにも存在しない
  (`test_manual_vs_frontline_priority`が明示的に「手動移動中はStopped命令でも解除されない」
  ことを検証している)。

これは実装の「バグ」ではなく、**2つの異なる機能が同じ「P21-002」というタスクIDの下で
混在した結果**である。詳細は7節参照。

## 2. 実装内容

本タスクで新規に実装したものはない(監査のみ)。監査対象となった既存実装は2つの異なる
時期に追加されたもの:

1. **移動・接敵戦闘そのもの(本タスクの本来の対象、変更なし)**: `map::army_selection`の
   `handle_movement_order`(右クリック移動命令)と`military::invasion::process_army_arrival`
   (到着時の占領/戦闘開始判定)。これらはP21-001以前から存在し、本セッションはもちろん
   前セッションでも一切変更していない。
2. **前線命令ボタン(前セッションで「P21-002: 前線命令のボタンUI化」として実装済み)**:
   `war::frontline::FrontlineStance`(Stopped/Defend/Offensive)を選択するボタン3個と、
   前線への師団割当/解除/全解除ボタン3個。これは1.とは独立した「複数師団の自動移動生成」
   機能であり、依頼書が対象とする「師団の直接移動・接敵戦闘操作のUI化」とは異なる機能である。

## 3. 変更ファイル一覧

本監査での変更: なし。

参考(前セッションまでに変更済みで、本監査が読み取り対象としたファイル):
`src/map/army_selection.rs`, `src/military/invasion.rs`, `src/military/movement.rs`,
`src/war/frontline.rs`, `src/ui/military_panel.rs`, `src/war/military_ai.rs`。

## 4. 新規・変更テスト一覧

本監査での追加・変更テストはなし。テスト総数の比較は6節参照。

## 5. 全検証コマンドと終了コード

| コマンド | 終了コード | ログ |
|---|---|---|
| `cargo fmt --check` | 1(**既知のベースラインFAIL**。差分15箇所、すべて保護対象`tests/land_war_combat_peace_test.rs`のみ。今回監査対象に変更は加えていないため新規差分は0件) | `02_cargo_fmt_check.log` |
| `cargo check --all-targets` | 0 | `03_cargo_check.log` |
| `cargo test -- --list` | 0(177件) | `04_cargo_test_list.log` / `04b_test_names_sorted.txt` |
| `cargo test` | 0(177 passed, 0 failed、全10バイナリ合計) | `05_cargo_test.log` |
| `cargo clippy --all-targets --all-features -- -D warnings` | 0 | `06_cargo_clippy.log` |
| `cargo build --release --all-targets` | 0 | `07_cargo_build_release.log` |
| `git diff --check` | 0(CRLF変換警告のみ、実質差分なし) | `08_git_diff_check_after.log` |

## 6. テスト数の実装前後比較

基準: P21-001完了時点(`verification_logs/phase-21/p21-001/addendum_2026-08-10/02_after_test_names_sorted.txt`、168件)。

現在: 177件(`04d_test_names_sorted_clean.txt`)。

差分(`04e_test_list_diff_clean.txt`): **追加9件、削除0件**。既存テストの消失なし
(前セッションの前線命令ボタン実装で追加された`war::frontline::tests::test_unassign_army_rejects_non_owner`、
`war::frontline::tests::test_evaluate_frontline_army_command_feasibility`、
`ui::military_panel::tests::handle_frontline_command_buttons_*` 7件)。

本監査自体はテストを追加していないため、177件は監査開始時点でも監査終了時点でも変化していない。

## 7. P21-002の各仕様を保証するコード経路

| # | 仕様 | 判定 | 保証するコード経路 |
|---|---|---|---|
| 1 | 攻撃・防衛モードを選択するUIは存在しない | **NG** | 反例: [military_panel.rs:314-326](../../../src/ui/military_panel.rs#L314-L326)。`FrontlineCommand::SetStance(FrontlineStance::Defend)`/`Offensive`ボタンが「防御」「攻勢」という攻撃/防衛モード選択UIとして実在する。 |
| 2 | 選択師団に対して移動先の州を指定できる | OK | [army_selection.rs:97-289](../../../src/map/army_selection.rs#L97-L289) `handle_movement_order`。右クリックで`selected_army.army_id`に対し目的地州を指定。 |
| 3 | 空の通行可能州なら通常移動になる | OK | [invasion.rs:19-93](../../../src/military/invasion.rs#L19-L93) `process_army_arrival`。到着地に敵軍がいなければ`occupy_state`のみで戦闘は発生しない。 |
| 4 | 戦争中の敵軍がいる州へ進入すると自動的に戦闘になる | OK | 同上、[invasion.rs:78-89](../../../src/military/invasion.rs#L78-L89)。敵軍発見時は`start_battle_between`を無条件に呼ぶ(プレイヤーの追加操作は不要)。 |
| 5 | 攻撃された師団は自動的に防衛側になる | OK | [invasion.rs:121-201](../../../src/military/invasion.rs#L121-L201) `start_battle_between`。到着した側が`attacker_army_id`、州に元々いた側が`defender_army_id`に機械的に決まる。プレイヤーが「防衛」を選ぶ操作は存在しない。 |
| 6 | 戦争していない国家や中立国の州には進入できない | OK | [army_selection.rs:217-229](../../../src/map/army_selection.rs#L217-L229)(命令発行時)、[movement.rs:82-97](../../../src/military/movement.rs#L82-L97)(移動中の再検証)、[invasion.rs:45-56](../../../src/military/invasion.rs#L45-L56)(到着時の最終防衛線)の3段階で拒否。 |
| 7 | 到達不能な州への命令は拒否される | OK | [army_selection.rs:260-288](../../../src/map/army_selection.rs#L260-L288)。`find_path`が`None`の場合、`ArmyUnit`は一切変更せず`warn!`のみ。 |
| 8 | 停止命令が機能する | **一部NG** | [frontline.rs:759-782](../../../src/war/frontline.rs#L759-L782) `process_stopped_plan`は前線自動生成移動のみ解除。手動移動(右クリック)を止める汎用コマンドは存在しない。反証: [frontline.rs](../../../src/war/frontline.rs) `test_manual_vs_frontline_priority`が「Stopped命令を出しても手動移動中の部隊は解除されない」ことを明示的に検証・確認済み。 |
| 9 | 同じ州に複数師団がいても、選択した師団だけが命令対象になる | OK | [army_render.rs](../../../src/map/army_render.rs) `army_display_positions` + [army_selection.rs:29-94](../../../src/map/army_selection.rs#L29-L94) `handle_army_selection`(P21-001で個別選択化済み)。`handle_movement_order`は`selected_army.army_id`単体のみを対象にする。 |
| 10 | UIクリックがマップへの移動命令として誤処理されない | OK | [army_selection.rs:113-117](../../../src/map/army_selection.rs#L113-L117)。`ui_interactions_q`でUI要素のHovered/Pressedを検知した場合は移動命令処理自体をスキップする。新規追加された`FrontlineCommandButton`等もBevyの`Interaction`により自動的にこのガード対象になる。 |

**総括**: 8/10仕様はPASS。すべて本タスクが対象とする「直接移動・接敵戦闘」機能
(P21-001以前から存在し、今回・前回のセッションでも変更していない部分)によって保証されている。
NGの2件はいずれも、別セッションで実装された「前線命令のボタンUI化」(`FrontlineStance`関連)
に起因しており、直接移動・接敵戦闘の実装自体に問題があるわけではない。

## 8. 手動確認が必要な項目

本セッションにGUI自動操作ツールはなく、以下は実機での確認が必要(P21-001と同様の制約):

- 右クリックによる移動命令の見た目上の反応(カーソル・経路表示など、視覚的なもの)
- 同一州の複数師団を左クリックで選び分ける操作感(P21-001でロジックは検証済みだが、
  クリック判定半径の使用感は未確認)
- 前線命令ボタン(割当/解除/全解除/停止/防御/攻勢)の見た目・配色が意図通りか
  (前回セッションでユーザーが`cargo run`で軍事パネルを開いたログは残っていない)

## 9. 保護対象の不変確認

| 対象 | 結果 |
|---|---|
| `assets/data/states.ron` | SHA-256完全一致(P21-001終了時点の記録`c5fab075...`と一致)。`git diff --stat`も出力なし。 |
| `assets/data/divisions.ron` | SHA-256完全一致(`c39404fe...`と一致)。`git diff --stat`も出力なし。 |
| `tests/land_war_combat_peace_test.rs` | SHA-256完全一致(`06f7cfee...`と一致)。`cargo fmt --check`の15箇所差分はP21-001以前からの既知のフォーマット未整合であり、内容差分ではない(SHA-256一致がその証拠)。 |
| `src/app/time.rs`(`DailySimulationSet`順序) | `git diff`出力なし(無変更)。 |
| `src/war/military_ai.rs`(プレイヤー国除外ロジック) | `git diff`出力なし(無変更)。 |
| 宣戦当日ガード(`war/frontline.rs`の`war.start_date == curr`判定) | 該当行を含むハンクは`git diff`に一切出現せず、無変更を確認。 |

詳細: `10_protected_files_sha256_now.txt`。

## 10. 未解決事項

1. **【要ユーザー判断】仕様1・8の不整合をどう解消するか**: (a) 前線命令ボタン
   (Defend/Offensive/Stoppedスタンス)を「P21-002」の範囲外の別機能として名称・ドキュメントを
   整理し直す、(b) スタンスボタンのうち攻撃/防衛モード的な性格が強い部分を撤去する、
   (c) 依頼書側の仕様を「前線自動化機能とは独立して直接移動は元々満たしている」ことを
   確認した上でP21-002自体は元々COMPLETEだったとみなし、前線命令は別タスク番号を
   正式に割り振る、のいずれかの方針決定が必要。本監査では一切変更していない。
2. **手動移動の汎用停止命令が存在しない**(仕様8関連の根本原因)。前線割当外の師団や、
   前線に割り当てられているが手動で移動命令を出した師団を止める手段が、キーボードにも
   UIにもない。HoI4的な「移動して衝突したら戦う」という設計方針と、今後この機能が
   必要になるかどうかはユーザー判断。
3. 8節の手動確認項目は実機確認が必要。
