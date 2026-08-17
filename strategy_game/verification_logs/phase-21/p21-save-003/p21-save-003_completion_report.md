# P21-SAVE-003 完了報告: 起動直後の「続きから」ロード導線

日付: 2026-08-14

## 1. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

自動ロードは行わず、`CountrySelection`画面の「続きから」ボタンを明示的に押した場合だけ
既存の単一スロット(`saves/savegame_v1.ron`)をロードし、成功時だけ`Playing`へ遷移する。
失敗時は`CountrySelection`に留まり、失敗分類をインライン表示する。New Game経路
(`StartGameButton`)は既存の`spawn_debug_divisions`挙動を含めて回帰なし。実7か国28州の
静的マスターRONを使った実プラグイン構成のEnd-to-Endテストで一連のフローを確認した。
本セッションにはGUI操作手段が無いため、実ウィンドウでの目視確認は未実施(判定基準どおり、
実施済みと偽らない)。

---

## 2. 事前監査結果(セクション2の必須監査)

実装着手前に、過去の報告書ではなく現在のソースコードを直接読んで確認した
([[re-verify own audit findings]]の教訓どおり)。

### 2.1 `GameState`の全バリアントと遷移箇所

`src/app/game_state.rs`は`MainMenu`(既定)/`CountrySelection`/`Playing`の3状態のみ。
遷移箇所は3つだけ:
- `app::loader::transition_to_country_selection`(`Startup`システム): `MainMenu → CountrySelection`
- `ui::country_selection::handle_start_button`(New Game): `CountrySelection → Playing`
- (今回追加) `save::runtime::handle_load_requests`のCountrySelection起点ロード成功時:
  `CountrySelection → Playing`

`Playing`から他状態へ戻る遷移は存在しない(タイトルへ戻る機能は無い、P21-SAVE-005の範囲外)。

### 2.2 `OnEnter(GameState::Playing)`へ登録されている全システム

`grep`で11ファイル・11システムを確認:

| ファイル | システム | 分類 |
|---|---|---|
| `app/loader.rs` | `spawn_debug_divisions` | **New Game専用の正規データ生成**(唯一) |
| `map/rendering.rs` | `setup_map` | 表示初期化 |
| `ui/top_bar.rs` | `setup_top_bar` | 表示初期化 |
| `ui/load_confirm.rs` | `setup_load_confirm_dialog` | 表示初期化 |
| `ui/military_panel.rs` | `setup_military_panel` | 表示初期化 |
| `ui/diplomacy_panel.rs` | `setup_diplomacy_panel` | 表示初期化 |
| `ui/peace_panel.rs` | `setup_peace_panel` | 表示初期化 |
| `ui/research_panel.rs` | `setup_research_panel` | 表示初期化 |
| `ui/politics_panel.rs` | `setup_politics_panel` | 表示初期化 |
| `ui/state_panel.rs` | `setup_state_panel` | 表示初期化 |
| `ui/economy_panel.rs` | `setup_economy_panel` | 表示初期化 |

**`spawn_debug_divisions`以外にロード済み正規データを変更するOnEnter処理は無い**(全ファイル
個別確認済み。他は全て`commands.spawn(...)`によるUIツリー/Entity生成のみで、Registry等の
ゲームデータResourceを変更しない)。`Playing`は一度しか進入されないため(前述のとおり
`Playing`から他状態への遷移が無い)、これらのUI初期化システムを二重実行する経路も無い。

### 2.3 CountrySelection UIのspawn/despawn経路

`ui/country_selection.rs`の`CountrySelectionPlugin`: `OnEnter(CountrySelection)`で`setup_ui`
(`CountrySelectionRoot`配下へ全UIをspawn)、`OnExit(CountrySelection)`で`cleanup_ui`
(`CountrySelectionRoot`をdespawn)。`Playing`への遷移時は`OnExit(CountrySelection)`が
先に走るため、Playing進入前にCountrySelection UIは確実に消える。

### 2.4 通知UIがCountrySelection中にも表示可能か

**不可能だった**。`GameNotification`を`NotificationHistory`へ反映する
`economy::handle_notifications`は`.run_if(in_state(GameState::Playing))`でのみ登録されており、
`NotificationHistory`の描画先(`ui/economy_panel.rs`)自体もPlaying専用UI。CountrySelection中は
通知が画面上のどこにも現れない。これが今回`ContinueStatusText`という専用インライン表示を
追加した直接の理由(仕様セクション7の要求どおり)。

### 2.5 `LoadRequestMessage`の登録元・送信元・Reader

登録元: `SaveGamePlugin::build`(`save/runtime.rs`)。送信元(改修前): `ui/load_confirm.rs`の
確認ダイアログ内ボタンのみ。Reader: `save::runtime::handle_load_requests`
(`&mut SystemState<MessageReader<LoadRequestMessage>>`を排他Systemで保持、フレームを跨いだ
カーソル永続化が必須)。

### 2.6 `handle_load_requests`の現在のrun conditionと実行スケジュール

改修前は`PostUpdate`に`.run_if(in_state(GameState::Playing))`のみで登録。CountrySelection中は
`LoadRequestMessage`を送っても一切実行されなかった。

### 2.7 `CameraResetRequestMessage`の処理タイミング

`map/camera.rs`の`reset_camera_on_request`(`Update`、無条件登録)。`GameCamera`は
`CameraPlugin::build`の`setup_camera`が`Startup`で無条件に1体spawnする
(`GameState`に一切依存しない)。したがって**CountrySelection中も`GameCamera`は既に存在する**
ことを確認した(仕様の「まだ存在しない可能性がある」という前提は、監査の結果、この
コードベースでは実際には成立しないことが分かったが、安全側の設計方針
[CountrySelection起点ではCameraResetを送らない]自体は変更する理由がないため維持した)。

### 2.8 New Game開始ボタンの現在の処理経路

`ui/country_selection.rs::handle_start_button`。`StartGameButton`が押され、`PreviewCountry`が
`Some`のとき、`PlayerCountry`を設定し同期的に`next_state.set(GameState::Playing)`を呼ぶ
(Message経由ではなく直接呼び出し)。既存の確認ダイアログは無い(New Gameは元から即座に開始)。

---

## 3. 採用した状態遷移

新しい`GameState`は追加していない(`Loading`状態は不要と判断)。理由: `handle_load_requests`
は`PostUpdate`の排他Systemとして同一フレーム内で読込→検証→適用を同期的に完結させる設計
(P21-SAVE-002D/002Eから継続)であり、非同期I/Oも導入しないため、「ロード中」を表す
中間状態を跨ぐ必要が無い。`CountrySelection`のまま`PostUpdate`でロードが完了し、成功時だけ
その場で`NextState::Playing`をセットする(1フレーム後に適用)。

```
MainMenu --(Startup)--> CountrySelection --[New Game]--> Playing
                              |
                              +--[続きから 成功]--> Playing
                              |
                              +--[続きから 失敗]--> CountrySelection (留まる)
```

---

## 4. New GameとLoaded GameのPlaying進入差

遷移制御専用の一時Resource `app::game_state::PlayingEntryMode`(`NewGame`/`LoadedGame`、
既定`NewGame`、セーブ対象ではない)を新規追加した。

- New Game (`handle_start_button`): `PlayerCountry`設定 → `PlayingEntryMode::NewGame`設定 →
  `NextState::Playing`
- Loaded Game (`handle_load_requests`のCountrySelection起点成功時): `apply_validated_save`が
  17 Resourceを含む正規データを`commit_load`で反映 → `PlayingEntryMode::LoadedGame`設定 →
  `NextState::Playing`

`app::loader::spawn_debug_divisions`(`OnEnter(Playing)`、`spawn_debug_divisions`の後に
`.chain()`で`reset_playing_entry_mode`を実行)は`PlayingEntryMode::NewGame`のときだけ
デバッグDivisionを生成し、`LoadedGame`のときは即座にreturnして何もしない。処理完了後、
`reset_playing_entry_mode`が`PlayingEntryMode`を`NewGame`へ決定的に戻す(次回`Playing`進入に
備えた既定化。現状`Playing`から他状態へ戻る経路が無いため実質的な再利用は起きないが、
仕様の「決定的な初期化」要求どおり実装した)。

表示初期化系のOnEnter(Playing)システム(`setup_map`/`setup_top_bar`/各パネルsetup等)は
`PlayingEntryMode`を一切参照せず、New Game/Loaded Gameどちらの進入でも無条件に実行される
(仕様どおり: マップ・パネル・カメラ等の表示初期化はロード開始時にも必要)。

---

## 5. 再利用した既存ロード経路

`read_and_validate_save_file`・`apply_validated_save`・`LoadRequestMessage`・`LoadOutcome`・
`LastLoadOutcome`・`LoadExecutionCount`は全てそのまま再利用し、CountrySelection専用の
read/deserialize/validate/apply実装は一切追加していない。`handle_load_requests`自体を
共有窓口とし、以下の2点だけを`GameState`で分岐させた:

1. `PostUpdate`での実行条件: `.run_if(in_state(GameState::Playing))` →
   `.run_if(in_playing_or_country_selection)`(`MainMenu`では引き続き実行しない)
2. 成功時の後処理: `GameState::Playing`中の実行なら従来どおり`CameraResetRequestMessage`を
   発行。`GameState::CountrySelection`中の実行(起動直後の続きから)なら、代わりに
   `PlayingEntryMode::LoadedGame`を設定して`NextState::Playing`をセットする。

`MessageReader<LoadRequestMessage>`用`SystemState`のフレーム間永続化(P21-SAVE-002Eで
確立した設計、`&mut SystemState<P>`を排他Systemの引数として受け取る)はそのまま維持した。

`src/save/apply.rs`(Prepare→Commitの二段階、17 Resourceのcommit仕様、Country/State集合
互換性検査)は**一切変更していない**。

---

## 6. 「続きから」UI

`ui/country_selection.rs`の`CountrySelectionRoot`右パネルへ、既存の`StartGameButton`の直下に
追加:

- `ContinueButton`(Component): 押下で確認ダイアログ無しに`LoadRequestMessage`を1件発行するだけ。
  `PreviewCountry`を一切参照しないため、国家未選択でも押下可能。
- 補助文(`country_selection.continue_hint`、静的テキスト)
- `ContinueStatusText`(Component): `LastLoadOutcome`が失敗を示す間だけ、
  `save::runtime::load_failure_notification`(このラウンドで`pub(crate)`化、既存の失敗分類
  ロジックを複製せず再利用)の結果をそのまま翻訳して表示するインライン状態テキスト。
  成功時・未実行時は空文字列。

表示文字列は全てローカライズキー経由(`localized_text`/`translate`)で、Rustへのハードコード
無し。新規キー2件を`en-US.ron`/`ja-JP.ron`双方へ追加(セクション9参照)。単一スロット前提の
ため、スロット一覧画面は作っていない。

---

## 7. 成功・失敗時の挙動

### 成功時
- 保存済み全17正規Resourceを`commit_load`が復元(変更していない既存仕様)
- `PlayerCountry`はセーブ値になる
- `PlayingEntryMode::LoadedGame`により`spawn_debug_divisions`はスキップ(0件セーブでも
  Divisionを追加しない)
- `GamePaused(true)`(`commit_load`の既存仕様、変更なし)
- `GameSpeed`はセーブ値を維持(既存仕様)
- 一時/UI Resource 13件は既存002D仕様どおり`PreparedLoadGameV1`が初期化
- AI dirty 4箇所はtrue(既存仕様)
- `LastLoadOutcome`はSuccess、`LoadExecutionCount`は1回だけ増加、成功通知(`GameNotification`)
  は1件発行(Playing遷移後の`handle_notifications`が消費)
- `Playing`へ正確に1回だけ遷移(次フレームで適用)、遷移フレーム自体は
  `LoadRequestMessage`を再消費しないため二重ロードなし
- カメラは`CameraResetRequestMessage`を送らず、`setup_camera`の既定Transformのまま
  (カメラEntityは1体のまま)
- ロード後に再セーブ可能・ゲーム中Loadも引き続き可能・新規ID非衝突を確認(E2E)

### 失敗時(FileNotFound/Read/Deserialize/UnsupportedVersion/Validation/Apply Prepare/
Country・State集合不一致/その他RuntimeCompatibility)
- `Playing`へ遷移しない、`PlayerCountry`/`PreviewCountry`を変更しない、デバッグDivisionを
  生成しない、`GamePaused`を変更しない、カメラを生成・初期化しない、CameraReset要求を
  送らない、成功通知を送らない
- 失敗通知は`ContinueStatusText`へ正確に1件分だけインライン表示(`GameNotification`が
  CountrySelectionで描画されない問題への対応、セクション2.4参照)
- `LastLoadOutcome`へ構造化された失敗を保持、`LoadExecutionCount`は1回だけ増加
- 同じ要求を翌フレームに再実行しない、セーブファイルを変更しない
- 失敗後にファイルを修復して再度「続きから」を押せば成功できる(E2Eで確認)

---

## 8. 競合処理

- 複数押下の1回への集約: 既存の`handle_load_requests`の集約ロジック(変更なし)がそのまま
  適用される
- New GameとLoadの同時要求裁定: `CountrySelectionPlugin`のUpdateシステム登録を
  `(handle_continue_button, handle_start_button).chain()`とし、`handle_start_button`が
  同一フレームの`LoadRequestMessage`発行有無を`MessageReader`でpeekし、発行済みなら
  New Gameを実行しない(`save::runtime::handle_save_requests`の既存Save/Load調停と同一パターン)。
  Loadが後で(`PostUpdate`で)失敗しても、New Gameは同一フレームには実行されていないため
  後から実行されることもない。
- 消費済み要求(`MessageReader`のpeek)は当該Systemのカーソルだけを進め、
  `handle_load_requests`自身のカーソルには影響しない(既存Save/Load調停と同じ設計)
- 成功後の最初のPlayingフレームでの二重ロード無し(新しい`LoadRequestMessage`が無い限り
  `LoadExecutionCount`は増えないことをテストで確認)
- `CountrySelection`では`SaveRequestMessage`を処理しない(`handle_save_requests`は
  `.run_if(in_state(GameState::Playing))`のまま、CountrySelection UIにSaveボタン自体も無い)

---

## 9. ローカライズ

新規キー(`en-US.ron`/`ja-JP.ron`両方へ追加、キー集合は一致):

| キー | EN | JA |
|---|---|---|
| `country_selection.continue_button` | Continue | 続きから |
| `country_selection.continue_hint` | Load the most recently saved game | 最後に保存したゲームをロードします |

失敗時のインライン表示は新規キーを追加せず、既存の`notif.load_failed_*`キー群を
`load_failure_notification`経由でそのまま再利用した。`localization::tests::
real_catalog_loads_and_ja_en_key_sets_match`・`p20_009_localization_resource_test`の
`ja_jp_and_en_us_key_sets_match_exactly`ともにPASS(キー集合一致を確認)。

---

## 10. 変更ファイル一覧

### 変更(既存ファイル)
- `src/app/game_state.rs`: `PlayingEntryMode`・`reset_playing_entry_mode`を新規追加
- `src/app/mod.rs`: `AppPlugin`へ`init_resource::<PlayingEntryMode>()`を追加
- `src/app/loader.rs`: `spawn_debug_divisions`へ`PlayingEntryMode`判定を追加、
  `OnEnter(Playing)`登録を`(spawn_debug_divisions, reset_playing_entry_mode).chain()`へ変更
- `src/save/runtime.rs`(未追跡ディレクトリ内、既存): `in_playing_or_country_selection`
  run condition新設、`LoadGamePlugin`の実行条件変更、`handle_load_requests`へ
  CountrySelection起点分岐を追加、`load_failure_notification`を`pub(crate)`化、
  新規テスト8件、既存テスト1件を状態対象の見直しに合わせて更新
- `src/save/mod.rs`(未追跡ディレクトリ内、既存): モジュール先頭doc更新のみ
- `src/ui/country_selection.rs`: `ContinueButton`/`ContinueStatusText`とハンドラ3個
  (`handle_continue_button`/`update_continue_status_text`/`handle_start_button`改修)を追加、
  `setup_ui`へUI要素追加、新規テスト7件
- `assets/localization/en-US.ron` / `ja-JP.ron`: 新規キー2件ずつ
- `tests/p20_009_hardcoded_string_scan_test.rs`: `country_selection.rs`向け新規exemption1件

### 新規ファイル
- `tests/p21_save_003_end_to_end_test.rs`: E2Eテスト4件

### 変更していない(保護対象、確認済み)
`src/save/apply.rs`・`src/save/dto.rs`・`src/save/export.rs`・`src/save/write.rs`・
`src/save/read.rs`・`src/save/validate.rs`・`src/ui/load_confirm.rs`・`src/ui/top_bar.rs`・
`src/app/loader.rs`の`load_game_data`/`validate_data`本体・`states.ron`/`countries.ron`・
`tests/land_war_combat_peace_test.rs`・`tests/p21_save_002e_*`・`main.rs`・`lib.rs`・
`ui/mod.rs`(いずれも本セッションでは未編集。以前のセッションからのdirty差分はそのまま
保護し、`git diff --stat`で本セッション中に増分が無いことを確認済み)。

---

## 11. 追加テスト一覧

### `src/save/runtime.rs`(単体、8件)
1. `load_from_country_selection_does_not_read_when_no_request`
2. `load_from_country_selection_success_sets_loaded_game_entry_mode_and_transitions_to_playing`
3. `load_from_country_selection_success_does_not_request_camera_reset`
4. `load_from_country_selection_failure_stays_in_country_selection`
5. `load_from_country_selection_failure_does_not_change_player_country_or_preview`
6. `load_while_already_playing_still_requests_camera_reset_and_does_not_touch_entry_mode`(回帰)
7. (既存テスト`load_request_outside_playing_state_does_not_read`を
   `load_request_outside_playing_or_country_selection_state_does_not_read`へ改名・対象状態を
   `CountrySelection`→`MainMenu`へ更新。CountrySelectionが今回から正式にロード実行可能な
   状態になったため、既存テストの前提そのものが仕様変更の対象だった。)

### `src/ui/country_selection.rs`(単体、7件)
8. `entering_country_selection_alone_emits_no_load_request`
9. `exactly_one_continue_button_is_spawned`
10. `continue_button_label_is_routed_through_localization`
11. `pressing_continue_emits_one_load_request_even_without_country_selected`
12. `pressing_continue_alone_does_not_trigger_new_game`
13. `simultaneous_new_game_and_continue_requests_execute_load_only`
14. `continue_status_text_shows_failure_and_clears_on_new_attempt`

### `tests/p21_save_003_end_to_end_test.rs`(E2E、4件)
15. `reaching_country_selection_does_not_read_or_create_a_save_file`
16. `continue_button_loads_real_save_and_enters_playing_without_debug_divisions`
   (実7か国28州データ、0件Divisionセーブ、再セーブ、新規ID非衝突、ロード後の
   ゲーム中Loadまで一連で検証)
17. `start_game_button_from_country_selection_still_spawns_debug_divisions`
18. `continue_button_failure_stays_in_country_selection_and_recovers_on_retry`

合計19件新規追加。

### 仕様セクション9の47項目との対応(重複追加せず、既存/新規テストへの対応を明記)

| # | 内容 | 対応するテスト |
|---|---|---|
| 1-2 | 起動だけではロード/saves作成しない | E2E#15 |
| 3-4 | CountrySelection入っただけではロードしない、LoadExecutionCount=0 | 単体#8、E2E#15 |
| 5 | 続きからボタン1個だけ | 単体#9 |
| 6 | ボタン文字列がローカライズ経由 | 単体#10 |
| 7-8 | 押下で1件発行、国家未選択でも押下可能 | 単体#11 |
| 9 | 複数押下が1回へ集約 | 既存`multiple_load_requests_in_one_frame_collapse_to_a_single_execution`(save/runtime.rs、変更なしで再検証済み) |
| 10 | New Gameへ漏れない | 単体#12 |
| 11 | New GameとLoad同時要求はLoadのみ | 単体#13 |
| 12-21,25,26 | 実データ成功・復元・0件Division・再セーブ・ID非衝突 | E2E#16 |
| 22 | New Gameは従来どおり1回生成 | E2E#17(+既存`p21_save_002e_end_to_end_test.rs`の`save_change_load_restores_state_...`が回帰確認) |
| 23-24 | カメラEntity1体・既定Transform / マップ等同期可能 | 単体#3(カメラReset未送信の構造的証明)。実ウィンドウでの目視は未実施(セクション18) |
| 27 | ロード後のゲーム中Loadが従来どおり | E2E#16内、既存`load_only_executes_load_once`等 |
| 28-40 | 各失敗分類・World不変・通知1件・Camera無し・GamePaused不変・翌フレーム非再実行・再試行成功 | 単体#4,5、E2E#18 |
| 41-43 | ゲーム中Load確認ダイアログ・カメラリセット・Save/Load裁定の回帰 | 既存`ui/load_confirm.rs`・`save/runtime.rs`テスト群(変更なしで再検証済み、単体#6で明示的にも再確認) |
| 44 | 起動ロード後の再保存・再ロード | E2E#16 |
| 45 | 既存470件へ回帰なし | セクション12,14参照 |
| 46 | JA/EN キー集合一致 | 既存`real_catalog_loads_and_ja_en_key_sets_match`・`ja_jp_and_en_us_key_sets_match_exactly`(変更なしで再検証済み) |
| 47 | ハードコード検査 | 既存`p20_009_hardcoded_string_scan_test.rs`(exemption1件追加のうえ再検証済み) |

---

## 12. テスト数の変更前後

| | Before | After | 差分 |
|---|---|---|---|
| `cargo test --lib`(`src/`内`#[test]`) | 405 | 420 | +15 |
| `tests/`配下の統合テスト(`#[test]`総数、headless-render含む) | 65 | 69 | +4 |
| **合計** | **470** | **489** | **+19** |

(`#[test]`属性の静的カウントで算出。470という数字は本タスクの前提として与えられた
「現行管理上のテスト総数」と完全に一致することを確認した。)

---

## 13. E2E結果

`tests/p21_save_003_end_to_end_test.rs`(4件、全PASS、実7か国28州の静的マスターRON・
実`AppPlugin`/`CountryPlugin`/`StatePlugin`/`BuildingPlugin`/`EconomyPlugin`/`ResearchPlugin`/
`PoliticsPlugin`/`DiplomacyPlugin`/`WarPlugin`/`MilitaryPlugin`/`SaveGamePlugin`/
`LoadGamePlugin`/実`CountrySelectionPlugin`を使用、`MapPlugin`/`UiPlugin`全体は
Window/フォント資産依存のため002E以来の既定方針どおり除外):

1. `reaching_country_selection_does_not_read_or_create_a_save_file` — 起動〜CountrySelection
   到達後、複数フレーム経過してもロード不実行・`saves/`相当の一時ディレクトリにファイル
   非生成
2. `continue_button_loads_real_save_and_enters_playing_without_debug_divisions` —
   実プロデューサーAppで7か国28州データを使い一度Playingへ進み、意図的にDivision集合を
   空にしてセーブ → 別の消費者AppでCountrySelectionから「続きから」ボタンを実際に押下 →
   1フレーム目でLoadSuccess・`CountrySelection`のまま、2フレーム目で`Playing`へ遷移 →
   日付・国庫・`PlayerCountry`が復元 → `GamePaused(true)` → Division集合は0件のまま
   (デバッグDivision非混入) → 再セーブ成功 → 新規Division/Army IDが既存と非衝突 →
   ロード後のゲーム中Loadも成功
3. `start_game_button_from_country_selection_still_spawns_debug_divisions` — 実際の
   `StartGameButton`押下経由でNew Gameを開始し、デバッグDivisionが従来どおり生成される
   ことと、`PlayingEntryMode`がOnEnter(Playing)処理後に`NewGame`へ戻ることを確認
4. `continue_button_failure_stays_in_country_selection_and_recovers_on_retry` — セーブ
   ファイル未作成の状態で「続きから」を押下 → FileNotFound失敗 → `CountrySelection`に
   留まる・`PlayerCountry`不変 → `ContinueStatusText`が空でない失敗メッセージを表示 →
   ファイルを正しく用意して再度「続きから」を押下 → 成功

固定PNGを書き込む既存headless-renderテスト(`ui_headless_render_test.rs`/
`p20_009_localization_headless_render_test.rs`/`p21_save_002e_headless_render_test.rs`)は
実行していない(`--no-run`でコンパイル確認のみ、セクション17参照)。新しいスクリーンショット
証拠は作成していない(実ウィンドウ操作手段が無いため)。

---

## 14. 全検証結果

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | 成功 |
| `cargo test --lib` | 420 passed; 0 failed |
| `cargo test --test economy_tests --test diplomacy_tests --test research_and_politics_tests --test p20_009_localization_resource_test --test profile_workload_correctness_test --test land_war_combat_peace_test --test daily_system_integration_test --test p20_009_hardcoded_string_scan_test --test p21_save_002e_end_to_end_test --test p21_save_003_end_to_end_test` | 全10バイナリ、計66テスト、全PASS |
| `cargo test --no-run --test ui_headless_render_test --test p20_009_localization_headless_render_test --test p21_save_002e_headless_render_test` | コンパイル成功、未実行(証拠保護) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| `cargo build --release --all-targets` | 成功 |
| `cargo fmt --check` | セクション15参照 |
| `git diff --check` | エラー無し(exit 0) |

---

## 15. rustfmtベースライン比較

`cargo fmt --check`の結果、差分は**`src/app/loader.rs`の1箇所のみ**
(`use crate::military::data::{DivisionStatus, Division, DivisionDefinition, MilitaryRegistry};`
の並び順)。この行は本セッションで一切編集していない行であることを`git diff`で確認済み。
さらに`git show HEAD:strategy_game/src/app/loader.rs`(セッション開始前のコミット済み内容)
を取り出して単体で`rustfmt --edition 2024 --check`にかけたところ、**同一の差分が
セッション開始前から既に存在していた**ことを確認した。したがってこの1件は本タスクが
一括修正すべきでない既存ベースラインの一部であり、変更していない。

本セッションで新規に編集・追加した全ファイル
(`game_state.rs`/`app/mod.rs`/`loader.rs`本文/`runtime.rs`/`save/mod.rs`/
`country_selection.rs`/`p21_save_003_end_to_end_test.rs`/`p20_009_hardcoded_string_scan_test.rs`
への追記)は、この1件を除き個別にrustfmt準拠であることを確認した。

なお、タスクの前提説明にある「rustfmt既知ベースラインは74 diff hunks」という数字と、
今回`cargo fmt --check`が実際に報告した1件のみという結果は一致しない。これは74件という
数値が測定された時点(記憶上はP21-SAVE-002F1前後、主に`tests/land_war_combat_peace_test.rs`
に集中していたはず)と、その後の`land_war_combat_peace_test.rs`自体を含む複数ファイルへの
未コミット編集(本セッション開始時点の`git status`に見えるとおり、P21-MAP-001等の別作業に
由来)との間で、既存の未整形状態が変化した可能性が高い。本タスクの範囲外であり深追いは
していないが、数値の食い違いとして正直に報告する。`cargo fmt`(書き込みモード)は
一度も実行していない([[cargo fmt scope gotcha]]のとおり、このリポジトリでは
`cargo fmt -- <path>`がワークスペース全体を整形してしまうため)。

---

## 16. git status/diff

作業開始時点のdirty差分(21ファイルのM、複数の未追跡ディレクトリ)はそのまま保護した。
本セッションで新たに変更したのは、セクション10に列挙したファイルのみ。`git diff --stat`で
`src/main.rs`/`src/lib.rs`/`src/ui/mod.rs`(いずれも開始時点で既にdirty)を個別に確認し、
本セッション中に追加の変更が無いことを確認した(diffの行数が開始時点の記録と一致)。
`git checkout`/`restore`/`reset`は一度も使用していない。

---

## 17. 固定証拠を上書きしていないこと

`verification_logs/p20-007/`・`verification_logs/p20-009/`・
`verification_logs/phase-21/p21-save-002e/`配下の既存PNG証拠は、本セッションが
対応する3本のheadless-renderテストバイナリを一度も実行していないため(`--no-run`のみ)、
書き換わっていない。新しいスクリーンショット証拠も作成していない(実ウィンドウ操作
手段が無いため)。`saves/savegame_v1.ron`(未追跡、以前のセッションの人間による
手動確認由来と推定)もタイムスタンプ未変化で、本セッションの全テストは専用の一時
ディレクトリ(`std::env::temp_dir()`配下、`Drop`で自動削除)だけを使用しファイルI/Oを
行った。テスト終了後、一時ディレクトリの残留が無いことも確認した。

---

## 18. 人間によるGUI確認待ち項目

本セッションには実ウィンドウ操作手段が無いため、以下は未確認(実施済みと偽らない):

1. `cargo run`でCountrySelection画面に「続きから」ボタンが意図した位置・見た目
   (New Gameボタンの直下、ヒント文言付き)で表示されること
2. ボタン押下で実際にロードが実行され、ちらつき・UI二重表示無くPlaying画面へ遷移すること
3. セーブファイルが無い状態でボタンを押した際、`ContinueStatusText`のインライン失敗
   メッセージが画面上に正しく・読める形で表示されること
4. JA/EN切り替えで「続きから」ボタン・ヒント・失敗メッセージが正しく再翻訳されること
5. 「続きから」ロード成功後のPlaying画面で、TopBar・各パネル・マップ・Division・
   カメラが実際に正常表示されること(自動テストはResourceレベルの検証に留まる)
6. New Gameボタンと「続きから」ボタンの並び・クリック感が既存デザインと違和感ないこと
7. ロード後の再セーブ・ゲーム中Loadを実際にプレイして確認すること
8. スパムクリック(連打)でロードが多重実行されないことの体感確認(自動テストは
   同一フレーム内の複数Message集約のみ検証、複数フレームにまたがる連打は未検証)

---

## 19. 発見した問題

- セクション2.7のとおり、仕様書が想定した「CountrySelection中はGameCameraがまだ存在しない
  可能性がある」という前提は、現在のコードでは成立しない(`setup_camera`が`Startup`で
  無条件にspawnするため)。ただし、あえてCameraResetを送らない安全側の設計自体は
  変更する理由が無いため、仕様の推奨どおり実装した。
- セクション15のとおり、`cargo fmt --check`の実測結果(1 hunk)がタスク前提の
  「74 diff hunks」と一致しない。原因はセッション開始前の未コミット編集の蓄積と
  推定され、本タスクの範囲外として深追いしていない。
- それ以外に、既存設計・既存テストとの矛盾や、想定外の副作用は見つからなかった。

---

## 20. P21-005への移行可否

**移行可能**と判断する。二重ロード・デバッグDivision混入・失敗時Playing遷移・部分適用は
いずれも自動テストで否定された。複数スロット・セーブ一覧・オートセーブ・クイックセーブ・
非同期I/O・Main Menu再設計等は本タスクの指示どおり一切追加していない。セクション18の
人間によるGUI確認が完了すれば、判定は`COMPLETE`へ更新できる状態にある。
