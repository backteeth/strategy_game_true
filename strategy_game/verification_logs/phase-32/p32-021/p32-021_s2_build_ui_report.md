# P32-021 S2: 経済施設の建設注文・進捗・キャンセルUI — 報告書

判定: **実装・自動検証は完了。新しいUI(建設パネル・スクロール・取消しボタン)の実マウス操作の手動確認が未実施のため、S2 全体は「COMPLETE」とは扱わない(手動確認待ち)。**
S3(保存・復元UI)・M4全体・Phase 32 は未完成/継続。

- 開始 HEAD: `0f240fc`(main、origin/main と一致、未追跡は `hex_atlas/` のみ)
- 終了 HEAD: コミット後に最終報告へ記載

## 0. S1 の完了記録(ユーザー確認による)

P32-020 では実マウス操作が未確認のため「実装・自動検証は完了、手動確認待ち」としていた(その記載は残してある)。今回の指示で、ユーザーから §11 の手動確認について**「問題なし」との報告**を受けた。この報告を根拠に **S1 の手動確認完了・S1 完了**と記録した(P32-020 報告書 §14 に追記)。実装者自身が実機のマウス操作を確認したものではない。項目別の詳細な報告は受け取っていない。

## 1. 変更ファイルと目的

| ファイル | 目的 |
|---|---|
| `src/hexmap/build_view.rs`(新規) | 建設パネル: 表示内容を導出する純粋関数 `build_panel_model`、UI部品(`BuildButton`/`CancelButton`/…)、行の作り直し(`sync_build_panel`)、表示切替 |
| `src/hexmap/economy_view.rs` | `SessionCommand` に `Build`/`Cancel`/`ToggleBuildPanel`、`apply_command` の適用、結果文言(`outcome_text`)と拒否理由の文言、`session_buttons` の拡張、パネルの生成・登録 |
| `src/hexmap/mod.rs` | `pub mod build_view;` |
| `src/bin/map_economy_session.rs` | `--shots` に S2 の自動スクリプト(候補→発注→進捗→取消し→他国家→水域→パネル非表示)を追加 |
| `tests/p32_021_build_ui_test.rs`(新規 17件) | Bevy `App` を通すUI・コマンドのテスト |
| `tests/p32_020_economy_session_view_test.rs` | 画面の注記文言の期待値を更新(「保存・復元の操作は後続の段階(S3)」)。検証は弱めていない |
| 共有側(`economy::map_session`/`map_state`) | **変更なし**(P32-019 の `check_build`/`request_build`/`cancel_order` で足りた) |

## 2. 建設候補と共通APIの接続

- 実コードの確認: 施設の定義元は `BuildingRegistry`(`session.buildings`)と `economy::map_state::is_state_level_economic_facility`。`check_build(selected, state, building) -> Result<OrderCheck{state, building, current_level, target_level, cost, required_progress}, BuildRequestError{Actor, Order(OrderError)}>`(状態を変更しない)、`request_build(selected, state, building) -> Result<ConstructionOrderId, BuildRequestError>`、`cancel_order(selected, ConstructionOrderId) -> Result<CancelReceipt, CancelRequestError{Actor, Cancel(CancelError)}>`。`ConstructionOrderId{country: u32, seq: u64}` は国家ごとの連番で再利用しない。キューの正本は `ScenarioCountry.construction_queue`(`MapConstructionOrder{id, state, building_type, target_level, progress, required_progress, paid_cost, status}`)。進捗は日次処理が1日につき +1.0、完成・返金は日次処理/`cancel_order`(支払済み×0.5)。
- 候補は `BuildingType::ALL` を `is_state_level_economic_facility` で絞ったもの(UI独自の一覧ではない)。各行の受付可否は `check_build`。表示は施設名(建物定義の `name`)・現在→目標レベル・費用・期間(`required_progress` 日)。拒否理由は `OrderError`/`ActorError` を**そのまま対応する文言へ変換**(別の理由に置き換えない)。費用・鉱床・上限・重複・国家対応・クリスタル専用・Mineの資源設定の判定はUIに無い。
- 受理可能な行は「発注」ボタン、拒否された行は理由つきの無効表示(ボタンなし)。候補の表示・再描画だけで経済状態は変わらない(テストで全体比較)。実際の注文は押した時点で `request_build` が現在の状態に対して再検証する。
- 未選択・水域・未割当・検証範囲外・地図外・経済に含まれないステート・他国家のステートでは、発注ボタンを出さず理由を表示。選択が変わるたびに行を全て作り直す(前のステートの候補は残らない)。

## 3. コマンドの対象識別と適用順

- `SessionCommand::Build{actor, state, building}` と `Cancel{actor, order}`。ボタンが**押した時点の操作国家・対象ステート・施設/注文識別子**を保持し、適用時に選択セルや行の位置から読み直さない。
- 適用は S1 と同じ1つの経路(`apply_session_commands` → `apply_command`)で到着順に逐次。日付進行・発注・取消しは同じ順で適用され、複数日進行は1コマンドとして同期的に完了するので途中へ割り込まない(受付順テスト: [発注, 5日, 取消し, 30日, 発注, 1日] が直接実行と一致。フレームを分けても同じ)。
- 適用時の操作主体が、コマンドを出したときの `actor` と違えば**実行しない**(`ActorMismatch`)。国家切替で古くなった操作を、新しい国家の操作として実行しない(テスト)。操作主体の検証・支払い・返金・二重取消しの拒否は共通APIが行う。
- ボタンは押した瞬間(`Interaction` の `Changed`)に1回だけコマンドを発行。行は作り直しのたびに新しいEntity(`Interaction::None`)なので、押し続け・再描画・パネル再生成で再発注・再取消しされない(テスト)。UI独自の重複禁止は無い(同じ施設の重複は共通APIが `AlreadyQueued`。別の施設の発注は通る)。

## 4. 注文・課金・進捗・完成・取消し・返金

- 発注: `request_build` が費用を1回だけ支払いキューへ追加(テスト: 国庫 5000→4800、連番 1→2、押し続けても1件)。完成: 日次処理だけが施設レベルを上げる(UIは触らない。Farm Lv3→Lv4 を確認)。取消し: `cancel_order` が返金(支払済み×0.5)とキュー削除を行い、`order_log` に記録。UIの返金・キュー操作は無い。
- 進捗表示は実キューの値から: 「進捗 p/r日(残り ⌈r−p⌉日)」「待機中(次の日から進行)」「建設中」。`p ≥ r` のまま残る注文は「完成条件を満たしましたが、前提不足で完成を保留中」。既存仕様(1日+1.0)から正確に求まる値で、推測値は出さない。取消しボタンに「返金 N」を表示(共通の `cancel_refund`)。内部の注文識別子は画面に出さない(行は施設・レベル・ステート・状態で区別。同じ(ステート,施設)の注文は重複できないので一意)。
- 他国家の注文は閲覧のみ(取消しボタンなし)。強制したコマンドも共通APIが `NotIssuersOrder` で拒否(テスト)。

## 5. 拒否理由と古い操作の扱い

拒否はAPIの理由を文言化して結果欄に出し、状態は変えない(国庫・キュー・連番・order_log を含む全体比較で確認)。
- 古い取消し: 完成後 → 「完成済みの注文のため取り消せません(返金しません)」、取消し済みの再取消し → 「すでに取り消された注文です(二重に返金しません)」。別の注文は取り消されない(完成後に新しい注文を入れても残る)。
- 古い受付確認: 表示後に国庫が減れば `InsufficientFunds`、同じ施設が先に注文されていれば `AlreadyQueued` として実注文で再判定(古い確認結果で通さない)。
- 表示順が変わっても同じ識別子を操作する(先頭/中間を取り消しても残りの行は自分の識別子を保持)。

## 6. 入力透過防止とスクロール

建設パネル・各ボタンは `Button`(`Interaction` を発行)で、既存の `camera_input`(`ui_is_hovered`・`DragState.started_on_ui`)の抑止がそのまま効く。一覧は既存の `ui::scroll::spawn_scrollable_body`(縦スクロール+スクロールバー)と `handle_panel_scroll`、`ScrollbarPlugin` を再利用(旧ゲームで実機に出た不具合への対策が入った実装)。S1 の入力抑止と `Tab` 遮断は維持。表示/非表示ボタンで地図を覆わないようにできる。
**これらの実マウス確認は未実施**(§9)。

## 7. 直接実行との状態遷移比較(自動テスト)

UIの発注・取消し・進行(70日。月次・AI建設を含む)の結果が、共有セッションの直接の `request_build`/`cancel_order`/`advance_day` と `MapEconomySimulation` 全体で一致。取消し後の状態、受付順(複数コマンド)、フレーム分割も一致。AI国家3の発注・月次処理・既存施設の表示は維持(S1 のテスト19件も緑)。

## 8. 必須チェック

- `cargo test -j 1 -- --test-threads=1`: **3302 passed / 0 failed / 6 ignored、exit 0**(前回 3285 + 新規 17)。途中、テスト内の日付の期待値の計算誤り(36日後=2/6)を修正(実装の不具合ではない)。
- `cargo clippy -j 1 --all-targets --all-features -- -D warnings`: 警告 0、exit 0。
- 旧V1 SHA-256 先頭 `098b2175…` 不変、`src/save/`・`saves/`・`assets/` に差分なし。保存処理は変更していない。スクリーンショットは `git checkout` で復元。

## 9. GUI確認

**自動スクリプト(`--shots`。実ウィンドウの描画。画面と同じコマンド経路を実行するもので、実マウスの代替ではない)で確認したこと**(6枚を `screenshots/` に保存): 日本語表示・文字欠け・パネルの重なりなし/建設候補(費用・期間・拒否理由)/6件発注後のキューと進捗(7日後 `進捗 7/30日(残り 23日)`)/識別子指定の取消しと返金(国庫が+250、候補が再び発注可能に)/他国家ステートの閲覧のみ表示/水域での候補なし/パネルの表示切替/長い一覧にスクロールバーが出ること。
**実マウスで未確認(PASSとしない)**: ボタンの実クリックによる発注・取消し(重複しないこと)、パネル上のクリック・ドラッグ・ホイールが地図へ透過しないこと、キュー内スクロール、長い一覧の実操作。

### ユーザー向け手動確認の手順(実際の検証シナリオ m4e に基づく)

起動: `strategy_game/` で `cargo run --bin map_economy_session`(既定 m4e)。
1. 上部の「国家1(手動)」をクリック → 操作国家が国家1に。右パネル「国庫: 5000.0」。
2. 緑のセル(検証ステートA = ステート#20)をクリック → 左の建設パネルに候補: Farm Lv3→Lv4 費用200 期間30日 / Logging Camp Lv0→Lv1 250/30日 / Mine Lv2→Lv3 500/60日 / Civilian Factory Lv1→Lv2 1000/90日 / Military Factory 1200/90日 / Railway Lv1→Lv2 800/60日 / University 1500/120日 / Magic Academy 1800/120日 / Crystal Refinery 1000/90日(「発注:」ボタン)、Crystal Mine は「× 発見済みのクリスタル鉱床がないため建設できません」(ボタンなし)。
3. 「発注: Farm …」を**1回だけ**クリック(押し続けても1回) → 国庫 5000.0 → 4800.0、キューに「Farm Lv4 … 待機中(次の日から進行) 進捗 0/30日(残り 30日)」と「取消 返金 100」、Farm の候補が「× 同じ施設の注文が建設キューにあります」に、結果欄に「発注しました…費用 200」。
4. 「+7日」 → 日付 1800/01/08、Farm は「建設中 進捗 7/30日(残り 23日)」。
5. Farm 行の「取消 返金 100」をクリック → 国庫 +100(4900.0 前後)、キューから消え、Farm が再び発注可能、結果欄に「取り消しました…返金」。もう一度同じ位置をクリックしても何も起きない(行が無い)。
6. Farm を再発注して「+30日」 → Farm が完成して Lv3→Lv4(右パネルの施設 Farm Lv4)、キューが空、結果欄に完成と(2/1をまたぐので)月次経済1回。国庫は月次収支で変動する。
7. 赤いセル(ステートB = #21、国家2)をクリック → 「他国家のステートのため発注できません(閲覧のみ)」、各候補に理由、発注ボタンなし。国家2の注文があれば「閲覧のみ」のキューに出て取消しボタンはない。
8. 水域(濃い青)→「水域(海・湖)には建設できません」、白いセル(未割当)→「…ステート未割当のため建設できません」、暗い緑(検証範囲外)→「検証範囲外の陸セルには建設できません」。いずれも候補なし。前のステートの候補が残らない。
9. 国家2(手動)を選択してステートB(#21)を選び、Farm を発注 → 国家2の国庫 800 → 600。国家1に戻すとキューは国家1のものになる(国家2の注文は他国家として閲覧のみ)。
10. 長い一覧: 国家1・ステート#20 で Farm・Logging Camp・Mine・Civilian Factory・Military Factory・Railway を続けて発注(合計3950、国庫5000以内) → キューが6行になり、パネル内をホイールでスクロールして全行と取消しボタンに届く。**このとき背後の地図がズームしないこと**。
11. 入力透過: パネル・ボタンの上でクリック → 背後のセル選択が変わらない。パネル上でドラッグ → 地図がパンしない。地図上ではパン・ズーム・選択が従来どおり。「建設パネル 表示/非表示」で隠せる。`Tab` で画面が閉じない(S1 から維持)。
GUIの手動確認が済むまで、S2 は「実装・自動検証完了、手動確認待ち」として扱う。

## 10. 旧ゲーム・V1・既存preview・アセットへの影響

旧ゲームのコード・登録・起動経路は無変更。独立App(登録プラグインは `DefaultPlugins`・`HexPreviewPlugin`・`EconomySessionPlugin` の3つだけ)のまま。既存 `hex_map_preview`・アセット・セーブ形式・`saves/` は無変更。共有API(`economy`)も無変更。

## 11. 未確認事項と観察

- 実マウス操作(§9)。ファイルのシンボリックリンク拒否は権限不足で引き続き未確認。
- 観察(既存規則どおりで、今回は変更していない): Crystal Refinery は(Crystal Mine と違い)鉱床の条件がなく発注できる。他国家のステートでは各候補に同じ理由が並び冗長(表示の改善余地)。建設パネルは地図の左側に重なる(表示切替で隠せる)。

## 12. S3へ渡す接続点

`SessionCommand`(保存・復元の `Save{name}`/`Load{name}` を追加し、同じ `apply_command` の1経路で適用)、`EconomySession::{save, inspect, load, list_saves}`(P32-019。`inspect` で警告/拒否を事前表示)、`SessionUi`(保存結果の表示)、`SessionSet`(Input→Apply→Target→Sync)。復元後は選択・操作国家を保持し、パネルは `is_changed` で自動的に作り直される。`.gitignore` は `/saves/map_economy/` を無視済み。
