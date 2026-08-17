# P21-SAVE-002E 完了報告: ゲーム内セーブ／ロード導線と本番App接続

日付: 2026-08-14

## 0. 最終評決

**COMPLETE**

すべての必須実装・自動テスト・検証コマンド・「実際のcargo run手動検証」の自動化代替
(Headless実描画・実クリックテスト)が完了し、全て green。実装中に本ラウンド自身の
新規コードとは無関係な、既存コードの実バグを2件発見し、いずれも修正・再検証済み
(詳細は§16「発見された問題」)。

---

## 1. 本番App(`main.rs`)への登録内容

```rust
.add_plugins(SaveGamePlugin)
.add_plugins(LoadGamePlugin)
```

を、既存の全ゲームプラグイン(`AppPlugin`〜`DebugPlugin`)の**後**に追加した
(`SaveGameResourceParams`/`apply::prepare_load`が読む全Resourceが出揃っている必要が
あるため)。`UiPlugin`は既に`LoadConfirmPlugin`・`TopBarPlugin`(セーブ/ロードボタン)を
含んでいる。

確認済み事項:
- `Message`/`Resource`の二重登録なし(`LoadRequestMessage`は`SaveGamePlugin`のみが登録し、
  `LoadGamePlugin`は登録しない設計を維持)。
- `PostUpdate`処理の二重登録なし。
- 起動しただけ(`app.update()`を複数回)では`saves/`ディレクトリを一切作らないことを
  `starting_the_game_does_not_create_a_save_file`(E2Eテスト)および
  `save_load_ui_headless_render_produces_real_pixels_and_real_state_changes`
  (Headlessテスト、Playing突入直後の`assert!(!save_dir.join(...).exists())`)で確認。
- `GameState::Playing`遷移だけでもファイルI/Oは発生しない(同上)。
- Startupの静的マスターローダー(`DataLoaderPlugin::load_game_data`)とプレイヤーセーブの
  読込は完全に別経路(前者はRON静的アセット、後者は`handle_load_requests`のみ)。

---

## 2. LoadRequest + 処理経路

`src/save/runtime.rs`に追加:

- `LoadRequestMessage`(`#[derive(Message)]`、一回性Message)。
- `LoadOperationError { ReadOrValidate(LoadSaveError), Apply(ApplyLoadError) }`
  (意図的に2バリアント。`ApplyLoadError`が既に`RuntimeCompatibility`等の内部分類を
  持つため)。
- `LoadOutcome { Success { path }, Failure { path, error } }`
- `LastLoadOutcome(pub Option<LoadOutcome>)` / `LoadExecutionCount(pub u32)` Resource。
- `handle_load_requests(world: &mut World, reader_state: &mut SystemState<MessageReader<LoadRequestMessage>>)`
  — 排他的System。手順:
  1. `LoadRequestMessage`を集約(0件なら即return、複数件も1回へ集約)
  2. `SaveFileConfig`から単一スロットパスを取得
  3. 現在の静的マスター(`BuildingRegistry`/`TechnologyRegistry`/`MilitaryRegistry.definitions`/
     `WorldCivilizationState.stage_definitions`)から`SaveValidationContext`を構築し、
     その共有借用を確実に終了してから
  4. `read_and_validate_save_file`(002Cの公開API)
  5. 成功した場合だけ`apply_validated_save`(002Dの公開API)を呼ぶ
  6. 成功時のみ`CameraResetRequestMessage`を発行
  7. `LastLoadOutcome`/`LoadExecutionCount`を記録
  8. 成功・失敗いずれも通知を1回だけ発行
- `LoadGamePlugin`: `SaveFileConfig`/`LastLoadOutcome`/`LoadExecutionCount`を初期化し、
  `PostUpdate`へ`handle_load_requests.run_if(in_state(GameState::Playing))`を登録。

読込・検証(手順4)が失敗した場合は`apply_validated_save`を呼ばない。適用(手順5)が
失敗した場合も、002Dの設計(Prepare失敗時はWorld無変更)により現在の正規状態が
そのまま維持される。002C/002Dの公開APIだけを経由し、それらを迂回する独自の
読込・適用経路は持たない。

---

## 3. Save/Load同時要求の調停

`handle_save_requests`に`MessageReader<LoadRequestMessage>`を追加し、`SaveRequestMessage`を
読み切った直後、保存処理の前に「同一フレームに`LoadRequestMessage`も来ているか」を
`.read().count() > 0`で確認する。来ていれば、その回の`SaveRequestMessage`は消費済みの
まま(次フレームへ持ち越さない)、実際の保存処理はスキップする。`DailySimulationSet`の
順序は一切変更していない。`SaveGamePlugin`の既存公開API・既存テストは壊していない。

6件の自動テストでロック済み(`save::runtime::tests`):
`save_and_load_same_frame_executes_load_only`,
`failed_simultaneous_load_still_skips_save`,
`multiple_saves_and_loads_still_collapse_to_one_load`, 他3件。

---

## 4. セーブ/ロードUI(トップバー)

`src/ui/top_bar.rs`に`SaveButton`/`LoadButton`マーカーComponentと2つのボタンを追加。
既存のスピード/一時停止/言語切替ボタンと同じ配色・サイズ・`Interaction`処理パターンを
再利用(新規スタイル定義なし)。

- **セーブ**: 1クリック→`SaveRequestMessage`を1件発行→単一スロットへ直接上書き
  (今回は確認ダイアログなし)。同一フレーム内の複数押下は`handle_save_requests`側で
  1回へ集約。
- **ロード**: 1クリック目では**ロードしない**。`LoadConfirmState.open = true`にして
  確認ダイアログを開くだけ。

UIクリックが地図/師団選択/移動命令/ドラッグ選択へ漏れないことは、既存の汎用ガード
(`map::selection`/`map::division_selection`が`Query<&Interaction>`でHovered/Pressed中を
グローバルにスキップする仕組み)にそのまま乗る形で保証される(新規ボタン・ダイアログ側の
コード変更は不要)。

---

## 5. ロード確認フロー

新規ファイル`src/ui/load_confirm.rs`(`LoadConfirmPlugin`):

- `LoadConfirmState { open: bool }`(独立Resource、`PeacePanelState`と同じ前例に倣い
  `ActivePanel`の4大パネル排他制御には混ぜない)。
- `LoadConfirmRoot`(全画面`Button`背景+中央ダイアログ)。背景自体もBoardButtonにする
  ことで、ダイアログが開いている間はどこをクリックしても既存の
  「UIのHovered/Pressed中はマップ操作をスキップする」ガードに確実に引っかかる。
- ダイアログ本文「保存済みゲームをロードしますか？」+「ロード」/「キャンセル」ボタン。
- 「ロード」ボタン: `LoadRequestMessage`を1件発行し、**同じフレーム内で同期的に**
  `open = false`(ロード成功/失敗の結果を待たない)。
- 「キャンセル」ボタン: `open = false`のみ。ゲーム状態は一切変更しない。

確認済み:
- ダイアログを開くだけでは`GamePaused`は変化しない
  (`load_button_press_does_not_change_game_paused`)。
- キャンセルはゲーム状態を一切変更しない(Headlessテストの Checkpoint 4)。
- ダイアログはロード成功・失敗いずれの場合も閉じたままになる
  (`confirm_button_press_emits_load_request_and_closes_dialog`、Headlessテスト Checkpoint 5)。
- ロード成功時に確認ダイアログが再び開いたままになることはない(Headlessテストで実描画
  確認済み)。

---

## 6. ローカライゼーション追加

`assets/localization/en-US.ron`/`ja-JP.ron`へ同一の14キーを追加:

`top_bar.save_button`, `top_bar.load_button`, `load_confirm.body`,
`load_confirm.confirm_button`, `load_confirm.cancel_button`, `notif.save_success`,
`notif.save_failed`, `notif.load_success`, `notif.load_failed_file_not_found`,
`notif.load_failed_read`, `notif.load_failed_deserialize`,
`notif.load_failed_unsupported_version`, `notif.load_failed_validation`
(`{count}`プレースホルダ付き), `notif.load_failed_apply`。

`cargo test --lib localization::`(9件)・`p20_009_localization_resource_test`(8件、
キー集合完全一致・重複なし・空文字なし・プレースホルダ一致を検証)全てpass。
Rustコード側にハードコードされた文字列は無く(`p20_009_hardcoded_string_scan_test`、
`load_confirm.rs`を新規に`TARGET_FILES`へ追加済み)、4件全てpass。

---

## 7. 通知経路

既存の`GameNotification`/`NotificationHistory`をそのまま再利用。

- 成功: `notif.save_success`, `notif.load_success`(「ゲームは一時停止中です」を含む
  文言で、ロード後にポーズされたことを通知内で明示)。
- 失敗: セーブ側は`notif.save_failed`(主要カテゴリのみ)。ロード側は
  `FileNotFound`/`Read`/`Deserialize`/`UnsupportedVersion`/`Validation`/`ApplyLoadError`
  それぞれ別キー。`Validation`失敗のみ`{count}`(問題件数)を通知本文に含み、詳細な
  リストは通知に出さない(診断ログのみ)。
- 1操作につき通知は必ず1回だけ発行される(`save_success_emits_exactly_one_notification`,
  `load_success_emits_exactly_one_notification`, `same_outcome_is_not_renotified_on_the_next_frame`
  等で保証。詳細は§16のバグ修正も参照)。
- ロード成功通知は002Dの`NotificationHistory`リセット(空へ)が既に発生した**後**にのみ
  現れる(`load_success_notification_appears_after_notification_history_is_cleared`)。

---

## 8. カメラ初期化

`src/map/camera.rs`:

- `pub(crate) fn default_camera_transform() -> Transform { Transform::IDENTITY }`
  — `setup_camera`(起動時)と`reset_camera_on_request`(ロード成功後)の両方が参照する
  唯一の共有定義(数値の二重管理なし)。
- `CameraResetRequestMessage`(一回性Message、ロード成功時に`handle_load_requests`が
  発行)。
- `reset_camera_on_request`: 複数要求を1回へ集約、既存`GameCamera` Entityの`Transform`
  だけを書き換え(新規spawn・despawnなし)、`CameraDragState`も初期化。`GameCamera`が
  0体・2体以上でもpanicせず`warn!`を出して視覚リセットだけを諦める(ロード自体の
  成否とは切り離す)。このコードベースのズームは`Transform.scale`のみで表現され
  (`OrthographicProjection`は未使用、grep確認済み)、`Transform::IDENTITY`で位置・
  ズーム両方をカバーする。

6件の単体テスト(camera.rs)+ Headlessテストでの実ピクセル確認(カメラを意図的に
ズーム・パンした状態からロード成功後に元の位置へ視覚的に戻ることをフレーム差分で
確認)で保証。

---

## 9. §3 再監査: 002Dの`commit_load`実際のResource数

`src/save/apply.rs::commit_load`の`world.insert_resource`呼び出しを本ラウンドで
再カウント: **31個**(17個の正規Resource + 1個`GamePaused` + 13個の一時的/UI Resource)。
002Dの完了報告書にあった「29個」は、報告書自身の内部矛盾(見出しでは「11個」、本文では
「13個」と自己言及していた一時Resourceの数え間違い)によるドキュメント上の誤りであり、
実装・機能上のギャップではない。全13個の一時Resourceは実際に`prepare_load`/`commit_load`
両方に実装済みで、コード上の欠落は無かった。**コード修正は不要、本報告書での数値訂正
のみ**(002Dの報告書自体は上書きしていない)。

---

## 10. 変更ファイル一覧(本ラウンドのみ)

**既存ファイルの変更:**
- `assets/localization/en-US.ron` / `ja-JP.ron`(§6の14キー追加)
- `src/main.rs`(`SaveGamePlugin`/`LoadGamePlugin`登録)
- `src/map/camera.rs`(`CameraResetRequestMessage`/`reset_camera_on_request`/
  `default_camera_transform`追加、テストモジュールをファイル末尾へ移動)
- `src/ui/mod.rs`(`LoadConfirmPlugin`登録)
- `src/ui/top_bar.rs`(`SaveButton`/`LoadButton`追加)
- `src/save/mod.rs`(pub use拡張、ドキュメントコメント更新)
- `src/save/runtime.rs`(`LoadRequestMessage`等の追加、`handle_save_requests`の調停ロジック、
  `SaveOutcomeReporter` SystemParam抽出、`handle_load_requests`の永続`SystemState`化)
- `src/save/validate.rs`(§16のPreIndustrialバグ修正)
- `tests/p20_009_hardcoded_string_scan_test.rs`(`load_confirm.rs`を`TARGET_FILES`へ追加)

**新規ファイル:**
- `src/ui/load_confirm.rs`
- `tests/p21_save_002e_end_to_end_test.rs`
- `tests/p21_save_002e_headless_render_test.rs`
- `verification_logs/phase-21/p21-save-002e/`(本報告書 + スクリーンショット5枚)

`src/save/`配下の`dto.rs`/`export.rs`/`read.rs`/`write.rs`/`apply.rs`は002A〜002Dで
既に実装済みであり、本ラウンドでは変更していない(§9のRe-audit時に`apply.rs`を
読み直しただけ)。`src/app/time.rs`・`src/diplomacy/claims.rs`・`src/diplomacy/crisis.rs`・
`src/lib.rs`・`src/map/division_selection.rs`・`src/map/rendering.rs`・`src/map/selection.rs`・
`src/military/*`・`src/ui/military_panel.rs`・`src/war/*`は、本ラウンド開始前から既に
未コミットで変更されていた別ラウンドの作業であり、本ラウンドでは一切触れていない
(`git status`の初期スナップショットと突き合わせ済み)。

---

## 11. 追加テスト一覧

- `src/map/camera.rs`(6件、新規`#[cfg(test)] mod tests`):
  `reset_request_restores_position_and_zoom_to_default`,
  `reset_request_restores_camera_drag_state`,
  `no_reset_request_leaves_camera_unchanged`,
  `multiple_reset_requests_in_one_frame_still_reset_once_and_do_not_duplicate_camera`,
  `missing_camera_entity_does_not_panic`, `multiple_camera_entities_does_not_panic`。
- `src/save/runtime.rs`(23件新規、うち代表例):
  Load runtime系11件(`load_request_while_playing_reads_validates_and_applies`,
  `load_success_sets_game_paused_true`, `apply_failure_preserves_current_state`,
  `malformed_ron_preserves_current_state`, `unsupported_version_preserves_current_state`,
  `validation_failure_preserves_current_state`, `missing_save_file_preserves_current_state`
  他)、調停系6件(§3参照)、通知系6件(`save_success_emits_exactly_one_notification`,
  `load_success_emits_exactly_one_notification`,
  `load_failure_categories_produce_distinct_notification_text`,
  `same_outcome_is_not_renotified_on_the_next_frame`,
  `load_success_notification_appears_after_notification_history_is_cleared`,
  `save_failure_emits_exactly_one_notification`)。
- `src/ui/top_bar.rs`(4件): `save_and_load_buttons_are_spawned_in_top_bar_ui_tree`,
  `save_button_press_emits_save_request_message`,
  `load_button_first_press_opens_confirm_dialog_without_emitting_load_request`,
  `load_button_press_does_not_change_game_paused`。
- `src/ui/load_confirm.rs`(4件、新規ファイル):
  `confirm_button_press_emits_load_request_and_closes_dialog`,
  `cancel_button_press_closes_dialog_without_emitting_load_request`,
  `dialog_is_spawned_hidden_by_default`, `sync_visibility_shows_and_hides_dialog_node`。
- `tests/p21_save_002e_end_to_end_test.rs`(新規、3件): 実本番プラグイン構成
  (`AppPlugin`+`CountryPlugin`+`StatePlugin`+`BuildingPlugin`+`EconomyPlugin`+
  `ResearchPlugin`+`PoliticsPlugin`+`DiplomacyPlugin`+`WarPlugin`+`MilitaryPlugin`、
  実7か国28州マップ)+`SaveGamePlugin`+`LoadGamePlugin`での
  `save_change_load_restores_state_a_new_ids_do_not_collide_and_resave_works`,
  `starting_the_game_does_not_create_a_save_file`,
  `failed_load_preserves_the_real_running_game_state`。
- `tests/p21_save_002e_headless_render_test.rs`(新規、1件):
  `save_load_ui_headless_render_produces_real_pixels_and_real_state_changes`
  (実GPU描画・実クリック注入によるSave→state変更→Load確認→キャンセル→Load確定の
  フルフロー、実PNG証跡5枚を出力)。

**新規テスト合計: 41件**

---

## 12. テスト件数(本ラウンド前後)

- 開始時(002D完了時点、永続メモリ記録): 411件
- 現在(本ラウンド完了時点、`cargo test --lib`389件 +
  安全な統合テスト63件[headless-render PNG2本[`ui_headless_render_test`/
  `p20_009_localization_headless_render_test`]を除く、既存方針どおり]): **452件**
- 差分: **+41件**(§11と一致)

内訳(現在の統合テスト63件): `daily_system_integration_test`(6) +
`diplomacy_tests`(5) + `economy_tests`(14) + `land_war_combat_peace_test`(4) +
`p20_009_hardcoded_string_scan_test`(4) + `p20_009_localization_resource_test`(8) +
`p21_save_002e_end_to_end_test`(3) + `p21_save_002e_headless_render_test`(1) +
`profile_workload_correctness_test`(9) + `research_and_politics_tests`(9)。

---

## 13. End-to-End結果

`tests/p21_save_002e_end_to_end_test.rs`(実プラグイン構成、実7か国28州マップ):

1. `starting_the_game_does_not_create_a_save_file`: 起動して複数フレーム経過しても
   `saves/`相当のファイルが作られないことを確認 — **pass**。
2. `save_change_load_restores_state_a_new_ids_do_not_collide_and_resave_works`:
   状態A(実データ)をセーブ → 日付・国庫・師団の州所属を大きく変更(状態B) → Load →
   日付・国庫・師団の州所属が状態Aへ復元、`SelectedArmy`/`SelectedState`が
   初期化されていること、ロード後に発行した新規`DivisionId`/`ArmyId`が既存IDと
   衝突しないこと、ロード後に再度セーブが成功することを確認 — **pass**。
3. `failed_load_preserves_the_real_running_game_state`: セーブファイル無しでLoadを
   要求しても、実行中の国庫・州数が一切変化しないことを確認 — **pass**。

---

## 14. 手動GUI検証(Headless実描画・実クリック代替)結果

エージェント環境には実際のウィンドウを操作する手段(スクリーン/マウス自動化)が
存在しないため、ユーザー承認の上で`tests/ui_headless_render_test.rs`(P20-007)と
同一手法を踏襲した`tests/p21_save_002e_headless_render_test.rs`を新規作成した。
本番`main.rs`と同一のプラグイン構成をWindowなしのoffscreen`RenderTarget::Image`へ
接続し、実フレームをGPU上で実行、`Interaction`コンポーネントへ実クリック相当の値を
注入して本番のボタンハンドラを実際に実行させ、GPUから実際にピクセルをreadbackして
PNGとして保存する。

実行結果(`cargo test --test p21_save_002e_headless_render_test -- --nocapture`): **pass**
(NVIDIA GeForce RTX 5070 Ti / Vulkan backend、実アダプタ使用)。

確認された項目:
- New Game相当(国選択→Start Game)でPlaying突入、TopBarに「セーブ」「ロード」ボタンが
  実際に1個ずつ存在(§7)。
- Saveボタンを実クリック → 一時ディレクトリへ`savegame_v1.ron`が実際に生成される
  (実リポジトリの`saves/`には触れない) → セーブ完了通知が実際に1件だけ発行される
  (ログ`[Notification] セーブが完了しました。`が正確に1回だけ出力)。
- 日付を進め、師団の州所属・カメラ位置/ズームを変更(状態B)。
- Loadボタンを実クリック → 即ロードされず確認ダイアログが実際に画面へ表示される
  (ピクセル差分あり、PNG保存)。
- キャンセルを実クリック → ダイアログが閉じ、状態・カメラは一切変化しない
  (通知も0件)。
- 再度Load→確認ダイアログの「ロード」を実クリック → 実際にロードが実行され、
  日付が状態Aへ復元、`GamePaused == true`、カメラが**視覚的に**既定位置・ズームへ
  戻る(`Transform::IDENTITY`)、確認ダイアログは開いたままにならない、ロード完了
  通知が実際に1件だけ発行される。
- ロード後に再度Saveボタンを実クリック → 再セーブが成功、通知1件。

新規スクリーンショット5枚を`verification_logs/phase-21/p21-save-002e/screenshots/`へ
保存(既存の`p20-007`等の固定PNGは一切上書きしていない):
`01_playing_topbar_with_save_load_buttons.png`, `02_after_save_click.png`,
`03_load_confirm_dialog_open.png`, `04_after_cancel_dialog_closed.png`,
`05_after_load_success.png`。

未実施(環境制約): 実ウィンドウでの目視操作、ウィンドウを閉じた後のプロセス残留確認
(`cargo run`自体は§16の理由により今回はビルド確認のみ)。上記のHeadless実描画・実クリック
テストが機能的に最も近い代替であると判断した。

---

## 15. 全検証結果

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | pass |
| `cargo test --lib` | 389 passed / 0 failed |
| 安全な統合テスト9本(headless-render PNG2本を除く) | 63 passed / 0 failed |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass(warning 0) |
| `cargo build --release --all-targets` | pass |
| `cargo fmt --check` | 既知のベースライン(§16参照)のみ残存、本ラウンドで
  触れた10ファイルは0 diff |
| `git diff --check` | pass(LF/CRLF警告のみ、実際の空白エラーなし) |
| 一時ファイル/プロセス残留確認 | `saves/`ディレクトリなし、テスト用一時ディレクトリ
  なし(いずれも`Drop`で自動削除・確認済み) |

---

## 16. rustfmtベースライン比較

- rustfmtバージョン: `rustfmt 1.9.0-stable (8bab26f4f6 2026-07-14)`
- 開始時点(本ラウンドの新規/変更コードを書き終えた直後、フォーマット前):
  `cargo fmt --check`で**109 diff hunks**(既存ベースライン81個 + 本ラウンドの
  未フォーマットコード28個)。
- 本ラウンドで触れた10ファイルのみを個別に`rustfmt --edition 2024 <file>`で
  フォーマット(`cargo fmt -- <path>`は本リポジトリでは単一ファイルへスコープされず
  全体を対象にしてしまう既知の問題があるため、意図的に回避):
  `src/map/camera.rs`, `src/save/runtime.rs`, `src/save/mod.rs`, `src/save/validate.rs`,
  `src/ui/top_bar.rs`, `src/ui/load_confirm.rs`, `src/ui/mod.rs`, `src/main.rs`,
  `tests/p20_009_hardcoded_string_scan_test.rs`,
  `tests/p21_save_002e_end_to_end_test.rs`。
- **事故**: `src/main.rs`はクレートルートであり、`rustfmt`へ直接渡すと`mod`宣言を
  辿って到達可能な全モジュールを再帰的にフォーマットする(単一ファイル指定にならない)
  ことが判明。この副作用で、本ラウンドと無関係だった`src/ui/peace_panel.rs`が意図せず
  再フォーマットされた(純粋な空白差分、ロジック変更なし)。既存ベースラインの各ファイルの
  diff位置(ファイル名:行番号)をフォーマット前後で完全一致することを確認し
  (`peace_panel.rs`以外は1件も変化なし)、被害範囲を`peace_panel.rs`のみに特定。
  ユーザーへ報告の上、`git checkout`で`peace_panel.rs`をHEADへ復元する許可を得て実施
  (自己判断での`git checkout`は行わず、必ずユーザー確認を経た)。
- 事後(全ラウンド作業完了後、`p21_save_002e_headless_render_test.rs`新規作成・
  `handle_load_requests`修正を含む): **74 diff hunks**、うち本ラウンドで触れた
  全ファイルは**0**。残存74件は全て本ラウンド開始前から存在した既存の未フォーマット
  ファイル(`app/loader.rs`, `country_ai.rs`, `division_render.rs`,
  `division_selection.rs`, `map/mod.rs`, `map/selection.rs`, `military/movement.rs`,
  `military/recruitment.rs`, `military/supply.rs`, `military/tests.rs`, `profiling.rs`,
  `ui/peace_panel.rs`[事故から復元後の元の2箇所], `war/capitulation.rs`,
  `war/frontline.rs`, `war/military_ai.rs`, `war/peace.rs`, `war/tests.rs`,
  `tests/daily_system_integration_test.rs`, `tests/land_war_combat_peace_test.rs`,
  `tests/profile_workload_correctness_test.rs`)であり、本ラウンドは一切これらを
  一括修正していない。

---

## 17. 発見された問題(本ラウンドで修正した既存コードの実バグ2件)

このラウンドの主目的はUI導線の追加だったが、実際の本番プラグイン構成・実データでの
統合テストを新規に書いたことで、既存の002C/002D実装に存在した2件の実バグを発見し、
両方とも修正・再検証した。いずれも「今回追加したコードの新しいバグ」ではなく、
実データ・複数フレームでの実行という、これまでのテストがカバーしていなかった条件で
初めて顕在化したものである。

### 17.1 `WorldStage::PreIndustrial`が参照整合性検証を必ず失敗させるバグ(重大)

`assets/data/world_stages.ron`(本番静的データ)は、ゲーム開始時点の既定段階
`WorldStage::PreIndustrial`の定義エントリを**意図的に持たない**(それ自身へ「到達する」
という遷移が存在しないため、`IndustrialRevolution`以降の4段階のみを定義)。しかし
`src/save/validate.rs`の`validate_save_game_v1`(002C)と`check_static_master_compatibility`
(002D)は、どちらも`current_stage`が`world_stage_definitions`のキーとして存在することを
無条件に要求していた。この結果、**現実のプレイでは開始直後からずっと(=最初の時代を
超えるまでの間ずっと)、セーブ・ロードが常に`DanglingReference`検証エラーで失敗する**
という、機能そのものを無効化する重大なバグだった。これまでの002C/002Dのテストは
全て人工的なフィクスチャで`PreIndustrial`を`world_stage_definitions`へ明示的に登録して
いたため発覚しておらず、本ラウンドで実データ(`DataLoaderPlugin`経由の本物の
`world_stages.ron`)を使うEnd-to-Endテストを初めて書いたことで発見した。

**修正**: `src/save/validate.rs`の両箇所で、`current_stage == WorldStage::PreIndustrial`の
場合は`world_stage_definitions`に存在しなくても参照切れとして扱わない例外を追加
(§15「静的RONアセットは変更しない」の制約があるため、`world_stages.ron`側へ
`PreIndustrial`エントリを追加する対応は取らなかった)。既存の`unknown_world_stage_is_rejected`
テスト(`PreIndustrial`以外の未定義Stageで正しく拒否されることを検証)は無傷で
pass。

### 17.2 `handle_load_requests`が1回のLoadクリックで複数回実行されるバグ(重大)

`handle_load_requests`(排他的System)は、`LoadRequestMessage`を読むために
`SystemState::new(world)`を**毎フレーム新規作成**していた。Bevyの`Message`は
Double-buffer方式で約2フレームは内部に残り続けるため、新規作成された
`MessageReader`は「まだ自分は読んでいない」とみなしてしまい、同一メッセージを
複数フレームにわたって重複して読んでしまう。結果として、Loadボタンを1回押しただけで
`apply_validated_save`が2〜3回実行され、`LoadExecutionCount`が1ではなく複数増加し、
通知も複数回発行される(§10「1操作につき通知は必ず1回」に違反)。

このバグは、1フレームだけ実行して即座に結果を確認する既存のロジックレベルテストでは
検出できず(その場合はメッセージバッファがまだ有効な間にしか読まないため症状が
出ない)、本ラウンドで新規に書いたHeadlessレンダーテスト(クリック後に見た目安定化の
ため多数のフレームを追加実行する)で初めて再現・発見した。

**修正**: Bevyが公式サポートする`ExclusiveSystemParam`実装
(`impl<P: SystemParam> ExclusiveSystemParam for &mut SystemState<P>`、`Local<T>`と同様
システム専用の永続ストレージへ自動的に保持される)を利用し、
`handle_load_requests(world: &mut World, reader_state: &mut SystemState<MessageReader<LoadRequestMessage>>)`
という形へシグネチャを変更。関数内で新しい`SystemState`を作らず、Bevyが自動的に
永続化する`reader_state`をそのまま使うことで、カーソル位置がフレームをまたいで
正しく保持されるようにした(`MessageWriter`側は書き込みだけでカーソルを持たない
ため、`CameraResetRequestMessage`/`GameNotification`の毎回`SystemState::new`は
そのまま安全)。修正後、Headlessテストで「Saveログ1回・Loadログ1回」を実際の
コンソール出力で確認し、`LoadExecutionCount == 1`をassertで確認した。

### 17.3 (バグではないが記録)rustfmtクレートルート再帰の事故

§16参照。`rustfmt <file>`へクレートルート(`main.rs`)を直接渡すと、`mod`宣言を
辿って到達可能な全モジュールを再帰的にフォーマットしてしまう(単一ファイルへは
スコープされない)。今後、特定ファイルだけをrustfmtしたい場合は`main.rs`/`lib.rs`/
`mod.rs`をその対象リストへ含めないこと。永続メモリへ記録済み。

---

## 18. P21-SAVE-002Fへの移行判定

**READY**

002Eにより、実際のゲーム画面(トップバーのセーブ/ロードボタン)から単一スロットの
保存・読込が実際に使用可能になった。§17.1/§17.2の2件の実バグは本ラウンド内で修正・
再検証済みであり、追加の未解決課題としては残っていない。ユーザーの方針
(「002Fで最終受入監査を行い、別タスクで起動直後ロードを追加してからP21-005へ戻る」)
どおり、次は002Fの最終受入監査へ進んで問題ない状態にある。

なお、本ラウンドでは真の意味での「人間によるウィンドウ操作」検証は実施できていない
(§14参照)。002Fの最終受入監査、またはその後の実プレイテストのタイミングで、
可能であれば人間による実際の`cargo run`操作(特にウィンドウを閉じた後のプロセス
残留確認)を一度行うことを推奨する。
