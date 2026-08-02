# Phase 20B-1i 監査・受入結果

監査日: 2026-08-02  
Gitルート: `C:\Users\hirom\Desktop\strategy_game`  
Rustクレート: `C:\Users\hirom\Desktop\strategy_game\strategy_game`  
Manifest: `C:\Users\hirom\Desktop\strategy_game\strategy_game\Cargo.toml`

## 結論

Phase 20B-1iはPASSと判定する。判定は既存報告書ではなく、実ソース、Test A〜F、全テスト、Clippy、Git差分、保護対象SHA-256から独立に行った。

`cargo fmt --check`だけはFAILである。差分対象は保護対象の`strategy_game/tests/land_war_combat_peace_test.rs`だけで、保護指示に従い変更していない。非保護ファイルのrustfmt差分は解消済みである。

## 開始時監査

- 開始時の`git status --short`: 出力なし
- `tests/daily_system_integration_test.rs`: 全文を確認
- `SetBoundarySnapshot`、全Observer、Test A〜F: 全定義と全関数を確認
- `audit_report.md`、`walkthrough.md`: 開始時には存在しなかった
- 保護対象2ファイル: 開始時と終了時でSHA-256一致、Git差分なし

## 保護対象SHA-256

| 対象 | Phase 20B-1i開始時 | Phase 20B-1i終了時 | 判定 |
|---|---|---|---|
| `strategy_game/assets/data/states.ron` | `c5fab07532a6c651a1f54962f78653d3a8518f9041475f52a29507e3ad39dc24` | `c5fab07532a6c651a1f54962f78653d3a8518f9041475f52a29507e3ad39dc24` | PASS |
| `strategy_game/tests/land_war_combat_peace_test.rs` | `06f7cfee9e2413dec6f18b4f5af86b82ff5556152c4f2026ede8c4d3142796a9` | `06f7cfee9e2413dec6f18b4f5af86b82ff5556152c4f2026ede8c4d3142796a9` | PASS |

## Test B 元構造体とSnapshotの対応

| 原本 | 比較対象フィールド | Snapshot |
|---|---|---|
| `GameDate` | `year`、`month`、`day`、private `accumulator` | `GameStateSnapshot.date`のcloneと`PartialEq` |
| `GamePaused`、`PlayerCountry` | 内包値 | `paused`、`player_country` |
| `CountryData` | `id`、`name`、`map_color`、`capital_state_id`、`treasury`、`government_type`、`economic_system`、`stockpile`、`tax_rate`、月次収支3項目、研究力2項目、`economic_state`、`construction_queue`、`research_state`、`politics`、`current_reform`、人的資源2項目、軍備3項目、`recruitment_queue` | `CountryDetailSnapshot`と各nested Snapshot |
| `ConstructionQueueItem` | `state_id`、`building_type`、`target_level`、`progress`、`required_progress`、`paid_cost`、`status` | `ConstructionQueueItemSnapshot`。Vec順を保持 |
| `CountryResearchState`、`InProgressTech` | 完了技術、全研究対象の`field`、`tech_id`、`progress`、`cost`、4配分、優先分野 | `ResearchStateSnapshot`、`InProgressTechSnapshot` |
| 政治・改革 | 価値観3軸、全利益団体の3値、改革の9フィールド | `country_values`、`InterestGroupSnapshot`、`PoliticalReformSnapshot` |
| `DiplomaticRelation` | 関係4値、戦争・通行・休戦、全条約、全cooldown、活動、更新日 | `DiplomaticRelationSnapshot`とnested Snapshot |
| `WarJustification` | `id`、参加国、対象州、開始日、必要日、経過日、完了値 | `JustificationDetailSnapshot` |
| `CountryAiState`、Registry | mode、reason、日次・週次・月次評価日、cooldown、state dirty、registry dirty | `CountryAiDetailSnapshot`、`country_ai_registry_dirty` |
| `MilitaryAiState`、Registry | country、評価日、reason、双方推定戦力、state dirty、registry dirty | `MilitaryAiDetailSnapshot`、`military_ai_registry_dirty` |
| `DivisionDefinition` | 全15フィールド | `DivisionDefinitionSnapshot` |
| `ArmyUnit` | ID、所有国、師団種別・規模、位置・destination・path・target、兵員、装備、組織、士気、経験、補給、移動進捗、status、`def_id`、攻防値、`combat_id` | `ArmyDetailSnapshot` |
| `Battle` | 全11フィールド | `BattleDetailSnapshot` |
| `War`、`WarGoal` | ID、名称、全参加国、全戦争目的、開始・終了日、期間、score、双方疲弊、占領州、status、winner、end_reason、terms、双方勝利数、処理済Battle ID | `WarDetailSnapshot`、`WarGoalSnapshot` |
| `Frontline`、`FrontlinePlan` | 前線全7フィールド、plan全5フィールド、全割当Army ID | `FrontlineDetailSnapshot`、`frontline_plans` |
| `FrontlineRegistry` | `frontlines`、`plans`、`next_frontline_id`、`army_frontline_map`、`frontline_generated_movements` | 対応する5つのSnapshotフィールド |
| `StateData` | 全27フィールド | `StateDetailSnapshot`と`StateResourceDepositSnapshot` |
| private Registry状態 | 正当化・軍・Battle・Warの次ID、State索引キャッシュ | 読取専用accessorと`justification_next_id`、`military_next_army_id`、`battle_next_id`、`war_next_id`、`state_index_entries` |

HashMapとHashSet由来の値は、CountryId、StateId、ArmyId、BattleId、WarId、FrontlineId、DivisionId、またはenum順でソートした。建設・募集・戦争目的・条約・経路などVec自体に意味がある値は元の順序を保持した。

開始時に未比較だったCountryDataの多数フィールド、正当化の`id/start_date`、Country AIの週次・月次・cooldown、Armyの`def_id/combat_id`、Warの`war_goals/processed_battle_ids`、FrontlineRegistryの次IDと自動移動Set、StateDataの残存フィールド、各private次ID、State索引はすべて比較対象へ追加した。

## Test D

- MilitaryAi直前値は同日WarPreparation直後Observerから取得
- MilitaryAi直後値は`handle_daily_military_ai`の後に順序制約したObserverから取得
- `last_evaluated_day`、`dirty`、`decision_reason`、`stance`、全`assigned_army_ids`を直前・直後で保存して更新を検証
- 割当IDごとに同じMilitaryAi境界で所有国を保存し、全IDが実在して`CountryId(1)`所有であることを検証

判定: PASS

## Test E

- Economy、Research、MilitaryAi、FrontlineOrders、MilitaryActionの5境界を独立Observerで取得
- 建設キューはEconomy直後に`progress == 1.0`を確認し、MilitaryAi直後と全7フィールド完全一致
- 研究はResearch直後とMilitaryAi直後で、対象、ID、進捗、コスト、完了技術、配分、優先分野を完全一致
- 手動移動は`StateId(0)`から`StateId(1)`、`current_path == [StateId(1)]`としてセットアップ
- MilitaryAi直後とFrontlineOrders直後でプレイヤーArmy全フィールド完全一致
- MilitaryAction直後は`current_state == StateId(0)`、`destination == Some(StateId(1))`、空の`current_path`、`target_state == Some(StateId(1))`、`movement_progress == 0.2`、`status == Moving`を検証
- AI国家は評価日更新、`dirty == false`、`reason == NoActiveWar`、stanceなし、割当IDなしを検証

判定: PASS

## Test F

- `frontline_plan_count`は`frontline_reg.plans.len()`から算出
- `capitulation_result`はWarResolution後に永続化済み`status`と`end_reason`から事後導出
- MilitaryAction直後の占領と、WarResolution直後の戦勝点更新を分離
- 攻撃側`CountryId(1)`、防御側`CountryId(0)`、`AttackerVictory`、勝者`CountryId(1)`、終戦理由`Defender Capitulation`、終戦日`1800/01/03`を直接検証
- `StateId(0)`の所有・支配が`CountryId(1)`へ移転し、占領進捗、元所有国、war IDが正規化されたことを検証
- active war 0件、war record 1件を検証
- frontlines、plans、`army_frontline_map`、`frontline_generated_movements`がすべて空であることを検証

判定: PASS

## Test A〜F

| Test | 判定 |
|---|---|
| Test A | PASS |
| Test B | PASS |
| Test C | PASS |
| Test D | PASS |
| Test E | PASS |
| Test F | PASS |

## 最終検証

生ログ保存先: `strategy_game/verification_logs/phase20b-1i/`

| コマンド | 結果 | 生ログ |
|---|---|---|
| `cargo fmt --check` | FAIL。保護対象`tests/land_war_combat_peace_test.rs`の既存rustfmt差分だけ | `01_cargo_fmt_check.log` |
| `cargo check` | PASS、exit 0 | `02_cargo_check.log` |
| `cargo test -- --list` | PASS、117 tests、0 benchmarks | `03_cargo_test_list.log` |
| `cargo test` | PASS、117 passed、0 failed | `04_cargo_test.log` |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、exit 0 | `05_cargo_clippy.log` |
| `cargo run` | 起動PASS。`target\debug\strategy_game.exe`起動後、GUI常駐のため20秒で監視終了、runner exit 124、panicなし | `06_cargo_run.log` |
| `git diff --check` | PASS、exit 0 | `07_git_diff_check.log` |
| `git status --short` | PASS、未コミット状態を記録 | `08_git_status_short.log` |
| `git diff --stat` | PASS、追跡ファイル差分を記録 | `09_git_diff_stat.log` |

## 変更の要点

- Test Bの完全Snapshot化
- MilitaryAi直前・直後Observer比較
- Economyを含むTest Eの5境界分離
- Test Fの直接状態検証と戦後整理検証
- Economy/Researchハンドラの公開によるObserver順序制約
- private Registry状態の読取専用accessor追加
- 非保護4ソースの既存import順をrustfmt準拠へ修正
- 保護対象は未変更

## フェーズ判定

| 項目 | 判定 |
|---|---|
| Phase 20B-1i | PASS |
| P20-007 | OPEN |
| P20-008 | OPEN |
| P20-009 | OPEN |
| Prototype v0.1 | NOT READY |

---

# P20-007 実装・検証結果(追記: 2026-08-02)

## 結論

P20-007「Headless環境でのUiPlugin描画完全観測」は **RESOLVED** と判定する。

本番 `main.rs` と同一のプラグイン構成(`UiPlugin` を含む全15ゲームプラグイン)をWindow非生成のHeadless構成で起動し、固定解像度(640x480, `Rgba8UnormSrgb`)のoffscreen `RenderTarget::Image` へ本番の `GameCamera`(`map::camera::setup_camera`が生成する実カメラ)を接続し、実GPU(NVIDIA GeForce RTX 5070 Ti, Vulkanバックエンド)でRenderGraphを実フレーム実行し、GPUからCPUへピクセルreadbackを行い、PNG保存とピクセル内容の自動assertまで実施した。ECS構造・Node数・Assetロード状態の確認だけでなく、実際に生成されたRGBAピクセルバッファに対して背景色との差分・ユニーク色数・バウンディングボックス・状態変更前後の差分ピクセル数を検証している。

## 実装方式

- 専用テスト: `strategy_game/tests/ui_headless_render_test.rs`(新規、775行)
- 実行コマンド: `cargo test --test ui_headless_render_test -- --nocapture`
- 本番UiPluginからピクセルreadbackまでの実行経路:
  1. `App::new()` に `main.rs` と全く同一の15プラグイン(`AppPlugin`〜`DebugPlugin`、`UiPlugin`含む)を同一順序で登録
  2. `DefaultPlugins` の `WindowPlugin` を `primary_window: None` / `exit_condition: ExitCondition::DontExit` に差し替え、`WinitPlugin` と `PipelinedRenderingPlugin` を `.disable::<T>()` で無効化(Window非生成・同期的Extract/Render)
  3. `PostStartup` で `Image::new_target_texture(640, 480, Rgba8UnormSrgb, None)` を生成し `Assets<Image>` へ登録、`TextureUsages::COPY_SRC` を付与
  4. 本番の `GameCamera`(`map::camera::setup_camera` がStartupで生成する実Entity)へ `RenderTarget::Image(...)` と `IsDefaultUiCamera` を後付けで挿入し、offscreen画像をUIの描画先に接続(本番のカメラ生成システム自体は無変更)
  5. Bevy公式 `examples/app/headless_renderer.rs` と同型の `ImageCopyPlugin`(`RenderApp`側: `ExtractSchedule`→`image_copy_extract`、`Render`(`RenderSystems::Render`後)→`receive_image_from_buffer`、`RenderGraph`→`image_copy_driver`)でGPUテクスチャをCPU可読バッファへコピーし、`crossbeam-channel` でMain Worldへ転送
  6. `App::run()` はselfをrunnerへmoveし実行後に world を検査できないため、`run_once` ランナーと同じ初期化列(`plugins_state()==Adding` の間 `tick_global_task_pools_on_main_thread`→`app.finish()`→`app.cleanup()`)を手動再現し、以後 `app.update()` を明示ループして毎フレームのExtract/Renderを同期実行
  7. `Startup`→`GameState::CountrySelection` 自動遷移により本番UIルート `CountrySelectionRoot` が実際に生成されるのを30フレームのウォームアップで安定化させ、1回目のPNG/ピクセルassertを実施
  8. `PreUpdate` に `UiSystems::Focus`(`ui_focus_system`)の**後**で動く注入専用システムを追加し、`Interaction::Pressed` を対象Buttonへ書き込むことで、Pointer入力が存在しないHeadless環境でも本番の `handle_country_button_click` / `handle_start_button` を実際に発火させ、2ヶ国目選択・Start Gameクリックを実行(疑似入力ではなく本番ハンドラを実行させる設計)
  9. Start Gameクリックにより `GameState::Playing` へ遷移し、本番の `TopBarRoot`(`ui::top_bar::TopBarPlugin`)が実際に生成・レイアウト・描画されるのを確認し、3回目のPNG/ピクセルassertを実施

### 技術的注意点(実装中に発見した実挙動)

- `bevy_ui::focus::ui_focus_system` は、カメラの `RenderTarget` が `Window` でない場合(本テストの `RenderTarget::Image`)、カーソル座標を解決できずカメラのUI Interactionを「Windowを持たない = カーソル不明」として扱い、毎フレーム全UI Nodeの `Interaction` を強制的に `None` へリセットする。そのため `world_mut()` からの直接注入は同一フレームのPreUpdateで即座に上書きされ、初回実装では2回目のクリックが全く反映されない(diff=0 pixel)不具合が発生した。`PreUpdate.after(UiSystems::Focus)` に注入システムを置くことで解消した。この挙動はHeadless offscreen UIテストの一般的な落とし穴であり、報告書に明記する。

## 使用Renderer/Backend/Adapter

| 項目 | 値 |
|---|---|
| Adapter名 | NVIDIA GeForce RTX 5070 Ti |
| Backend | Vulkan |
| Device Type | DiscreteGpu |
| Driver | NVIDIA 610.74 |
| RenderTarget解像度 | 640 x 480 |
| ピクセル形式 | `Rgba8UnormSrgb`(readback後 RGBA8 unpadded) |

Adapter/Backendが利用不可な場合、テストは `RenderDevice`/`RenderAdapterInfo` リソース欠如を検知して即座にpanicし、非ゼロ終了する(スキップしない設計)。今回の環境では実GPU Adapterが取得でき、実描画・実readbackが成功した。

## 専用テストと主要assert

テスト名: `ui_headless_render_produces_real_pixels`(`ui_headless_render_test.rs`内、単一関数)

| assert内容 | 実測値 |
|---|---|
| `RenderDevice`/`RenderAdapterInfo` 存在(Adapter利用可能性) | 存在確認PASS |
| `Assets<Font>` 非空、`AssetId::<Font>::default()` ロード済み | count=1, PASS |
| 本番 `CountrySelectionRoot` Entity生成・`ComputedNode`レイアウト完了 | size=(640.0, 480.0) |
| `GameCamera` の `RenderTarget::Image` と `IsDefaultUiCamera` 有効性 | PASS |
| Checkpoint1(初期国選択画面): 出力幅高・非透明・非単色・非背景ピクセル数・bbox画面内 | non_bg=70135, unique_colors=528, bbox=(32,35)-(563,393) |
| Checkpoint2(2ヶ国目選択後): 同上 + Checkpoint1との差分ピクセル数 | non_bg=69473, unique_colors=527, diff(1→2)=11818px |
| 本番 `GameState` が `Playing` へ遷移、本番 `TopBarRoot` Entity生成・レイアウト完了 | size=(640.0, 40.0) |
| Checkpoint3(Playing/TopBar): 同上 + Checkpoint2との差分ピクセル数 | non_bg=283237, unique_colors=3531, diff(2→3)=307198px |

いずれの差分も許容誤差付きの閾値判定(完全一致ハッシュ非依存): `MIN_NON_BACKGROUND_PIXELS=300`, `MIN_DIFF_PIXELS=50`, `BG_TOLERANCE=16`(RGBA各chの最大差)に対し、実測値は数十〜数百倍のマージンで上回った。

## 証拠保存先

- 生ログ: `strategy_game/verification_logs/p20-007/01_ui_headless_render_test.log`(`--nocapture`の未編集出力)
- 生成PNG: `strategy_game/verification_logs/p20-007/screenshots/`
  - `01_country_selection_default.png`(640x480、本番国選択画面、Kingdom of Arcadiaプレビュー)
  - `02_country_selection_after_click.png`(640x480、Elfin Republicへプレビュー変更後)
  - `03_playing_topbar.png`(640x480、Playing状態、本番TopBarRoot・Economy/Statesパネル等)
- その他検証ログ: 同ディレクトリ配下 `02`〜`10` 番の各コマンドログ、保護対象SHA-256記録

## 全検証コマンドの結果

生ログ保存先: `strategy_game/verification_logs/p20-007/`

| コマンド | 結果 | 生ログ |
|---|---|---|
| `cargo test --test ui_headless_render_test -- --nocapture` | PASS、1 passed; 0 failed | `01_ui_headless_render_test.log` |
| `cargo check --all-targets` | PASS、exit 0 | `02_cargo_check.log` |
| `cargo test -- --list` | PASS、118 tests(既存117+新規1) | `03_cargo_test_list.log` |
| `cargo test` | PASS、118 passed; 0 failed | `04_cargo_test.log` |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、exit 0(新規ファイルも警告0件) | `05_cargo_clippy.log` |
| `cargo run` | 起動PASS。GUIウィンドウ生成を確認後、プロセスを正常終了(残存プロセスなし) | `06_cargo_run.log` |
| `cargo fmt --check` | FAIL。保護対象`tests/land_war_combat_peace_test.rs`の既存rustfmt差分のみ(`git diff --stat`で0行差分、内容同一を確認済み)。新規`ui_headless_render_test.rs`はフォーマット準拠(差分0件) | `06_cargo_fmt_check.log` |
| `git diff --check` | PASS、exit 0(warningはautocrlf由来の表示のみ) | `07_git_diff_check.log` |
| `git status --short` | 未コミット状態を記録 | `08_git_status_short_final.log` |
| `git diff --stat` | PASS、Cargo.lock/Cargo.tomlの差分のみ、保護対象は0行差分 | `09_git_diff_stat.log` |

### `cargo fmt --check` と保護対象ファイルに関する重要な注記

作業中に一度、`cargo fmt -- tests/ui_headless_render_test.rs` を実行した際、意図に反して保護対象 `tests/land_war_combat_peace_test.rs` のフォーマットも書き換わる事故が発生した(`cargo fmt` はファイル引数を渡してもパッケージ全体を対象にする挙動であったため)。これを検知した時点で直ちに `git show HEAD:strategy_game/tests/land_war_combat_peace_test.rs > tests/land_war_combat_peace_test.rs` によりGitオブジェクトから元のバイト列を復元し、SHA-256が基準値と完全一致することを確認した。

さらに、このリポジトリは `core.autocrlf=true` の環境設定であり、`git checkout --` 等の通常のGit操作は当該ファイルをLF→CRLFへ変換してしまい、SHA-256が変化することが判明した(`git show HEAD:...`による直接復元はこの変換を経由しないため基準値と一致する)。復元後、`git status --short` は環境のautocrlf設定に起因して当該ファイルを `M` と表示するが、`git diff`・`git diff --stat` は共に **0行差分** であり、内容は完全に元のままであることを確認済みである。以後、当該ファイルに対して `cargo fmt`(ファイル指定含む)や `git checkout` を再実行しないよう徹底した。

## 変更ファイル一覧

- `strategy_game/Cargo.toml`(`[dev-dependencies]` に `crossbeam-channel = "0.5"`, `image = "0.25"` を追加。いずれも既存の依存グラフに既に含まれるバージョンで新規ダウンロード不要)
- `strategy_game/Cargo.lock`(上記dev-dependencies追加に伴う再解決。既存の本番依存バージョンに変更なし)
- `strategy_game/tests/ui_headless_render_test.rs`(新規、P20-007専用Headless UI描画テスト)
- `strategy_game/verification_logs/p20-007/`(新規、証拠ディレクトリ: ログ・PNG)
- `audit_report.md`、`walkthrough.md`(本追記)
- 保護対象2ファイルは内容変更なし(詳細は上記注記を参照)

## 保護対象SHA-256(P20-007作業開始時・終了時)

| 対象 | 開始時 | 終了時 | 判定 |
|---|---|---|---|
| `strategy_game/assets/data/states.ron` | `c5fab07532a6c651a1f54962f78653d3a8518f9041475f52a29507e3ad39dc24` | `c5fab07532a6c651a1f54962f78653d3a8518f9041475f52a29507e3ad39dc24` | PASS |
| `strategy_game/tests/land_war_combat_peace_test.rs` | `06f7cfee9e2413dec6f18b4f5af86b82ff5556152c4f2026ede8c4d3142796a9` | `06f7cfee9e2413dec6f18b4f5af86b82ff5556152c4f2026ede8c4d3142796a9` | PASS |

## フェーズ判定(更新)

| 項目 | 判定 |
|---|---|
| Phase 20B-1i | PASS |
| P20-007 | **RESOLVED** |
| P20-008 | OPEN |
| P20-009 | OPEN |
| Prototype v0.1 | NOT READY(P20-008、P20-009が未解決のため) |
