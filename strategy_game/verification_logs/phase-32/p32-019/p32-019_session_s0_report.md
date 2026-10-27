# P32-019 S0: 経済セッション組み立ての共通化と画面用API — 報告書

判定: **COMPLETE**(S0 の前提整理の範囲)。画面・独立App・M4全体・新マップでのゲーム進行の完成ではない。Phase 32 は継続。次は S1(`map_economy_session` の起動、経済表示、日付進行)。

- 開始 HEAD: `aed2e17`(main、origin/main と一致、未追跡は `hex_atlas/` のみ)
- 終了 HEAD: コミット後に最終報告へ記載

## 1. 変更ファイルと目的

| ファイル | 目的 |
|---|---|
| `src/economy/map_session.rs`(新規) | セッション組み立て(`SessionScenario`/`SCENARIOS`、`SessionPaths`、`GeographyData`、`build_simulation[_from_text]`、`EconomySession`)、操作主体の検証、確認API、保存/復元/確認/一覧のラッパー |
| `src/economy/map_state.rs` | `OrderCheck`、`check_order`/`check_build` を追加。`place_order` を同じ中核検証(`check_order`)を呼ぶ形に整理(判定の複製なし) |
| `src/economy/map_economy_file.rs` | `inspect`/`InspectReport`/`SaveSummary`、`list_saves`/`SaveList`/`SaveEntry`/`EntryStatus`/`DirectoryStatus` を追加。`load` と `inspect` が解析・判定(`examine`)を共有 |
| `src/building/data.rs`、`src/app/loader.rs` | `BuildingRegistry::from_ron`(旧ローダーの検証を純粋関数へ切り出し)。旧ローダーはこれを呼ぶ(検証内容・パニック経路は同じ) |
| `src/bin/world_economy_check.rs` | `m4d_simulation` と `run_snapshot_resume` の組み立てをライブラリ経由に。`--scenario`、`--economy-list`、`--economy-inspect` を追加 |
| `src/economy/mod.rs` | `pub mod map_session;` |
| `.gitignore` | `/saves/map_economy/`(U6) |
| `tests/p32_019_session_api_test.rs`(新規 23件) | 本報告の自動検証 |
| `verification_logs/phase-32/p32-018/…design.md` | §15 追記(U1/U2/U3/U6 の反映、S0 の実装対応) |

## 2. U1・U2・U3・U6 の反映

- **U1**: 設計書 §15 に採用を追記。独立バイナリ・独立Appは今回実装しない(旧ゲームの更新プラグインを登録しない方針は S1 で実装)。
- **U2**: `EconomySession::resolve_actor`/`manual_actors`。画面/CLIの選択状態は `EconomySession` に持たない。`--actor` フラグは S1 の独立Appで実装する(今回の CLI には付けていない)。
- **U3**: `SCREEN_DEFAULT_SCENARIO = "m4e"`(資源は `province_resources_m4d.ron`)。`CLI_DEFAULT_SCENARIO = "m4d"` は従来のまま。CLI は `--scenario m4e` で明示的に選ぶ。**対応関係の確認**: m4e のシナリオ先頭のコメントが `province_resources_m4d.ron` を前提と明記しており、組み立てが検証を通ること、国家1・2が手動で国家3がAI、ステート20/21/22が揃うことをテストが確認(不整合は見つからなかった)。補足: m4e の `months: 4` は画面セッションでは使わない(日数は操作で決まる)。P32-013 の検証用変種(資源設定の一部を外す)は既定ではなく、テスト内の入力差し替えでだけ使う。
- **U6**: `/saves/map_economy/` を `strategy_game/.gitignore` に追加。`git check-ignore` で `saves/map_economy/x.mapeco.ron` は無視され、`saves/savegame_v1.ron` は無視されないこと(追跡済みのまま)を確認。`git rm` はしていない。

## 3. 共通化した組み立て

`GeographyData::load(paths)`(地図+ワールド設定、整合検証つき)→ `EconomySession::load_files(scenario, world, paths)`(建物定義 `BuildingRegistry::from_ron`、位置施設、シナリオ+資源から `MapEconomySimulation`)。
`EconomySession` は `MapEconomySimulation` を**唯一の正本**として所有し、経済状態のコピーは作らない。保存照合用の識別情報は `session.identity(map, world)`(保存時と同じ生成)。
CLI の `--economy-*` と P32-015 の `run_snapshot_resume` は同じ `build_simulation` を使う。CLI用と画面用の初期化は分けていない。他の `run_*`(m4b/m4c/手動注文の変種など)は、それぞれ別シナリオ・変種の入力を扱う検証用でそのままにした(今回の共通化の範囲外)。
通常起動でファイルを作らない: 一覧・確認・組み立ては作成/書き込みをしない(テストで確認)。

## 4. 操作主体の検証

`resolve_actor(Option<u32>)`: `NotSelected` / `UnknownCountry` / `NotManual`(AI国家)を区別して返し、別の国家へ置き換えない。`manual_actors()` は候補(ラベル昇順)で、既定の選択ではない。
`resolve_actor_for_state` は `ScenarioCountry.states`(経済計算上の対応関係)に照らして `StateNotMapped` を返す(表示用の参照。建設の受付判定は `check_build`/`request_build` の既存規則 `NotMappedToCountry`)。
`request_build`/`cancel_order`/`check_build` は選択を検証してから既存関数を呼ぶ。旧 `CountryId` との対応は作らない(ラベルは検証シナリオ内の値)。法的所有・占領モデルは確定していない。

## 5. 受付可否の確認 API

`MapStateEconomyRegistry::check_order(&self, &ScenarioCountry, state, building, &BuildingRegistry) -> Result<OrderCheck, OrderError>` と `check_build(…, issuer, …)`。
`place_order` はこの `check_order` を現在の状態で呼んでから支払い・キュー追加を行うため、判定条件と順序は常に一致する(定義 → クリスタル条件 → 上限 → 重複 → 費用 → Mine の資源設定)。`OrderCheck` は状態・施設・現在/目標レベル・費用・必要進捗。
**確認だけでは何も変更しない**(国庫・備蓄・キュー・連番・order_log・AI履歴・レベル)。確認結果は予約ではなく、実注文は常に再検証する。
検証: 全(操作主体×ステート×建物)で `check_build` と複製した状態への `request_build` が結果・拒否理由・注文内容で一致/確認前後で `sim` 全体が一致/確認で連番を消費しない/確認後に国庫を減らす・同じ注文を入れる・建物定義の費用を変えると実注文が再判定される。

## 6. `inspect` と `list_saves`

- **`inspect(sim, inputs, location, name) -> Result<InspectReport, FileError>`**: `load` と同じ `examine`(`decode` の厳密な解析 → 復元先から識別情報生成 → `judge`)を共有。緩い別パーサーは無い。`kind`・形式版・バイト数・保存日付・概要(ステート/国家/建設キュー数)・項目別判定(一致/警告/拒否/識別情報の問題)を返す。判定が通る場合は `restore_candidate` まで確認して `restorable`(`Some(Ok)`/`Some(Err)`)を返す(**復元はしない**)。互換性拒否は「確認の成功+拒否理由」、非対応/破損/別kind/V1/形式1/不正名/不在は `Err`。ディレクトリを作らない。
- **`list_saves(location) -> SaveList`**: `<base>/map_economy/` 直下の `*.mapeco.ron` のみ。`DirectoryStatus`(`Missing`=保存なし/`Readable`/`Refused(理由)`)と各 `SaveEntry.status`(`Valid{date,bytes}`/`Unusable(理由)`)を分離。名前昇順で安定。壊れたファイル・非対応形式・別kind・旧V1・リンク・通常でないもの・不正な名前は `Unusable` で、`Valid` にならない。ディレクトリ作成・修復・変換・一時ファイルの削除・保存先外の読み取りをしない。保存先がリンク/ジャンクションで別の場所を指す場合は `Refused` で、外側のファイルは列挙しない(Windows のジャンクションで実際に確認。SKIPPED ではない)。
- **保証範囲**: `inspect` の結果はその時点のもので、復元の許可ではない。`load` は毎回ファイルを読み直して全検証をやり直す(確認後にファイル/静的入力が変われば拒否: テストで確認)。確認と復元の間に別プロセスがファイルを書き換える競合(TOCTOU)は、この追加では解決していない(P32-016 と同じ保証範囲)。

## 7. 状態とファイルの非変更の検証(自動テスト)

確認・一覧の前後で、`sim` 全体、保存先の全ファイル(名前と内容)、一時ファイル、実際の `saves/savegame_v1.ron` が不変。負例は一時ディレクトリとその内のコピーのみ(実ユーザーセーブは破壊的試験に使っていない)。

## 8. CLI と保存再開の回帰

- 通常実行(引数なし)の出力 267 行が P32-015 の記録と**完全に一致**(`world_economy_check_output.txt`、cargo のビルド行を除く)。
- 100日保存(18401バイト)→ 別プロセスで60日再開 = 連続実行の101〜160日と **60/60行一致**(終了コード0、行数60、日付 1800/04/12〜06/10)。先頭/末尾の行は共通化前の記録(P32-017 `cli_identity_output.txt`)と同じダイジェスト(`0efefcfcfc398595` / `7f6f2d0ddbd2cc19`)で、テストにも埋め込んだ。
- 追加CLI: `--economy-list`(保存なし/一覧)、`--economy-inspect`(判定表示、ファイル不変)、`--scenario`(未知のIDは候補を示して失敗)。m4d の保存は `--scenario m4e` では拒否される(`scenario_structure`・`military_inputs`)。
- 実出力: `cli_regression_output.txt`。

## 9. 必須チェック

- `cargo test -j 1 -- --test-threads=1`: **3266 passed / 0 failed / 6 ignored、exit 0**(前回 3243 + 新規 23)。
- `cargo clippy -j 1 --all-targets --all-features -- -D warnings`: 警告 0、exit 0(途中でテストの初期化の書き方 1 件を修正)。
- 途中の失敗: テストのコンパイルエラー(`BuildingRegistry` が `Debug` でない)を `.err().unwrap()` で回避(旧型には手を入れていない)。
- スクリーンショットは `git checkout` で復元し、コミットしていない。

## 10. 旧ゲーム・V1・アセットへの影響

- `src/save/`、`saves/savegame_v1.ron`(SHA-256 先頭 `098b2175…` 不変)、`assets/` に差分なし。旧ゲームの起動経路は変更なし。
- 旧ゲームに触れた箇所: `app/loader.rs` の建物定義の検証を `BuildingRegistry::from_ron` 呼び出しに置き換えた(検証内容・パニック経路は同じ。メッセージの接頭が `[DataLoader] assets/data/buildings.ron: …` で同形式)。既存テストは全て緑。
- GUI は変更していない。GUI確認待ちはない。

## 11. S1 へ渡す接続点と残件

接続点: `EconomySession`(Bevy `Resource` にそのまま載せられる)、`EconomySession::{advance_day, request_build, cancel_order, check_build, resolve_actor, manual_actors, save, inspect, load, list_saves}`、`GeographyData`(CLI)/既存 `LoadedHexMap`・`LoadedWorld`(画面)、`SessionPaths`、`SCREEN_DEFAULT_SCENARIO`。
S1 で実装: 独立バイナリ `map_economy_session`(`--actor`、`--scenario`、`--save-base`)、`SessionCommand`/`apply`、`EconomySessionPlugin`、`HexPreviewSet`、画面表示(日付・ステート経済・国庫)、日付進行。
残件・未確認: ファイルのシンボリックリンク拒否は権限不足で引き続き未確認(ジャンクションは確認済み)/TOCTOU・電源断の永続性は未保証/画面の見え方・UI入力の透過は未実装のため未確認/SaveGameV2・D11・軍/戦争/所有の保存は別単位。
