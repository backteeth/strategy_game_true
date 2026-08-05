---

# P20-009 実装・検証結果(追記: 2026-08-04)

## 結論

P20-009「UI表示テキストの完全ローカライズ」は **RESOLVED** と判定する。

`audit_report.md`/`walkthrough.md`にはP20-009の判定表エントリ(OPEN)のみが存在し、詳細な受入条件は
記載されていなかったため、本タスクの依頼文(日本語/英語2言語対応・i18n基盤・実行中切替・
Headless実描画検証・回帰維持)を正式な受入条件として採用し、以下の対応を行った。

## 1. 目的と受入条件

- 目的: ハードコードされたUI表示文字列(日本語・英語混在、大半が英語)を、安定した翻訳キー経由の
  最小限のi18n基盤へ移行し、実行中の言語切り替え(既定: 日本語, フォールバック: 英語)を可能にする。
- 受入条件(採用): 2言語(ja-JP/en-US)対応、実行中切替、既存Textの再翻訳、全UI文字列の監査、
  キー集合・プレースホルダーの両言語一致、重複/空/欠落キーの自動検出、日本語フォントの実描画確認、
  シミュレーション・決定論の不変、P20-007/P20-008の維持。

## 2. 対応言語

- 既定言語: 日本語 (`ja-JP`)
- フォールバック言語: 英語 (`en-US`)
- 言語切り替え: 画面右上(TopBar)および国選択画面に配置した`LanguageToggleButton`をクリックする
  ことで、`CurrentLocale`リソースを即時変更し、既に生成済みの`Text`コンポーネントを画面の
  作り直し無しに再翻訳する(`LocalizedText`コンポーネント + `retranslate_on_locale_change`System)。

## 3. i18n基盤の構成

新規モジュール `strategy_game/src/localization.rs` に集約。

- `Locale`(ja-JP/en-US)、`TranslationCatalog`(RONから読み込む言語別キー→テンプレート文字列)、
  `CurrentLocale`(Resource)、`LocalizedText`(どのキー・引数で表示中かを保持するComponent)
- `translate()` / `t()` / `tf()`: キーを現在言語で解決し、`{name}`形式のテンプレート引数を置換する。
  ja-JPに無ければen-USへフォールバックし、両方に無ければ開発時に識別可能な`⟦MISSING:key⟧`
  マーカーを返す(空文字列・原文フォールバックで隠さない)。
- `TranslationCorePlugin`(Resourceのみ提供、`Assets<Font>`/`Window`に依存しないため
  `MinimalPlugins`ベースの既存統合テストにも安全に追加可能)と、
  `LocalizationPlugin`(フォント差し替え・言語切替ボタン・Text/Window再翻訳を追加する
  UI表示層プラグイン、`UiPlugin`から自動的に読み込まれる)の2段構成。
- 表示文をRustコード内のmatch文へ埋め込む代わりに、enumの`display_name()`は翻訳キー
  (例: `"building.farm"`)を返すよう変更し、UI側で`t()`により言語ごとの表示名へ解決する。
  新しいenum variant追加時はRustの`match`網羅性チェックにより取りこぼしがコンパイルエラーで
  検出される。

## 4. 翻訳リソースの場所・キー数

- `strategy_game/assets/localization/ja-JP.ron`
- `strategy_game/assets/localization/en-US.ron`
- 形式: `Vec<(String, String)>`(順序付きリスト、重複キーを検出しやすくするためHashMapへの
  直接デシリアライズは使わない)
- キー数: **336**(ja-JP = en-US、全キーがコード側から最低1箇所参照されていることを確認済み)

## 5. UI表示文字列の監査件数・移行件数・除外件数

- 監査対象: `src/ui/*.rs`全10ファイル、通知メッセージ構築5ファイル(economy/research/
  diplomacy::update/politics/debug)、enum表示名12ファイル、UI配線済みエラー文字列3ファイル
  (war/data.rs, war/justification.rs, war/peace.rs)
- 監査件数: 約230箇所
- 翻訳キーへ移行: 約210箇所(キー数336、ja/en)
- 正当な除外: 9箇所(記号ラベル4種、空文字列placeholder5箇所)+ データ由来固有名詞
  (国名・州名・技術名・建物名など、`assets/data/*.ron`由来、保護対象`states.ron`は無変更)
- 詳細: `strategy_game/verification_logs/p20-009/string_audit/01_display_string_inventory_and_migration.md`
  (対応表: `02_translation_key_usage_map.md`)

## 6. 日本語フォントとライセンス

- 既存のデフォルトフォント(Bevy `default_font`機能が同梱する`FiraMono-subset.ttf`)は
  ASCII/Latin-1専用で日本語グリフを含まないことを確認した。
- 既存の`assets/fonts/JapaneseFont.ttc`(未使用ファイル)はMicrosoft "MS Gothic"系
  プロプライエタリフォントであり再配布不可のため使用せず、新たに以下を追加した。
  - フォント名: **Noto Sans JP** (Variable Font)
  - 入手元: `github.com/google/fonts` (`ofl/notosansjp/NotoSansJP[wght].ttf`)
  - ライセンス: **SIL Open Font License 1.1**(再配布・改変可)
  - 配置場所: `strategy_game/assets/fonts/NotoSansJP-Variable.ttf` + ライセンス文書
    `NotoSansJP-OFL.txt`
  - 適用方法: `AssetId::<Font>::default()`へ上書き挿入。既存の全`TextFont{..default()}`
    呼び出し(9ファイル、100箇所超)は無変更のままフォントのみ差し替わる。
  - 詳細: `verification_logs/p20-009/string_audit/03_font_investigation_and_license.md`

## 7. 自動テスト結果

新規3ファイル、計13テスト、すべてPASS:

- `tests/p20_009_localization_resource_test.rs`(8件): キー集合一致、重複無し、空翻訳無し、
  プレースホルダー一致、必須キーカテゴリ存在、フォールバック動作、欠落キー検出、
  テンプレート置換のエンドツーエンド確認
- `tests/p20_009_hardcoded_string_scan_test.rs`(4件): `Text::new("literal")` /
  `message: "literal"`パターンの残存走査(理由付き固定除外リストのみ許可)、除外リストの
  陳腐化検知、対象ファイル存在確認
- `tests/p20_009_localization_headless_render_test.rs`(1件、下記参照)
- `src/localization.rs`内`#[cfg(test)]`単体テスト(9件、`cargo test --lib`に含まれる)

## 8. Headless実描画結果

P20-007の`tests/ui_headless_render_test.rs`のHeadless実GPU・offscreen描画・PNG readback方式を
再利用(本番`UiPlugin`・本番`GameCamera`・実GPU実行、偽UIへの置き換え無し)。

- ja-JP(国選択画面・Playing画面)→ 言語切替ボタンクリックでen-US → 再度ja-JPへ切替、という
  往復を国選択画面・Playing画面の両方で実施し、各状態でPNG保存・非背景ピクセル数・ピクセル差分を検証。
- 折りたたみパネル(研究/政治/外交/軍事)も全て開いた状態で検証し、`LocalizedText`を持つ
  全Textに欠落キーマーカーが一切出現しないことを確認。
- ja-JP→en-US→ja-JPの往復で、Playing画面のTopBar表示テキストは元のja-JP表示と完全一致
  (文字列比較)、往復PNGはSHA-256で完全一致(決定的な再描画を確認)。
- 言語切替前後でシミュレーション状態(国庫・利用可能人的資源・総人口・戦争数・AI状態数)が
  完全に不変であることをアサート。
- 証拠PNG: `verification_logs/p20-009/screenshots/01`〜`06`(SHA-256は同ディレクトリの
  `png_sha256.txt`)。目視でも文字化け・豆腐表示・ロード失敗が無いことを確認済み。

## 9. P20-007・P20-008の維持確認

- P20-007: `tests/ui_headless_render_test.rs`は無変更のまま再実行し、PASS(閾値も無変更)。
- P20-008: `tests/profile_workload_correctness_test.rs`再実行PASS。
  `cargo run --release --bin profile_1000_states -- verification_logs/p20-009/profile_1000_states_output`
  も全スケール(100/500/1000/2000)・全シナリオ(normal/high_load)で正常完了。

## 10. 全テスト件数

**152件、すべてPASS**(P20-009追加分13件を含む。既存139件は無変更で継続PASS)。

## 11. 全検証コマンドの実結果

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | PASS |
| `cargo test -- --list` | PASS |
| `cargo test` | PASS(152件、0 failed) |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS(0 warnings) |
| `cargo run`(GUI起動・日本語表示確認・安全終了) | PASS(残存プロセス無し) |
| `cargo fmt --check` | **FAIL** — 保護対象`land_war_combat_peace_test.rs`の既知差分15箇所のみ。新規・変更Rustファイル17個は個別パス指定の`rustfmt`で整形済みでFAILの原因ではない。 |
| `git diff --check` | PASS |
| P20-007専用テスト再実行 | PASS |
| P20-008専用テスト・プロファイリングバイナリ再実行 | PASS |

「fmtを含めてすべてPASS」とは記載しない。

## 12. 変更ファイル一覧

新規:
- `strategy_game/src/localization.rs`
- `strategy_game/assets/localization/ja-JP.ron`, `en-US.ron`
- `strategy_game/assets/fonts/NotoSansJP-Variable.ttf`, `NotoSansJP-OFL.txt`
- `strategy_game/tests/p20_009_localization_resource_test.rs`
- `strategy_game/tests/p20_009_hardcoded_string_scan_test.rs`
- `strategy_game/tests/p20_009_localization_headless_render_test.rs`
- `strategy_game/verification_logs/p20-009/`(証拠一式)

変更:
- `strategy_game/src/lib.rs`(`pub mod localization;`追加)
- `strategy_game/src/ui/mod.rs`(`UiPlugin`が`LocalizationPlugin`を追加)
- `strategy_game/src/ui/{country_selection,state_panel,economy_panel,research_panel,
  politics_panel,military_panel,diplomacy_panel,peace_panel,top_bar}.rs`(翻訳キー移行・
  言語切替ボタン追加)
- `strategy_game/src/{economy,research,politics,debug}/mod.rs`, `src/diplomacy/update.rs`
  (通知メッセージの翻訳キー化、`TranslationCorePlugin`追加)
- `strategy_game/src/{building/data,country/mod,country/country_ai,diplomacy/data,
  economy/resources,economy/economic_state,military/data,politics/values,
  politics/interest_groups,research/data,war/peace,war/frontline,war/military_ai}.rs`
  (`display_name()`が翻訳キーを返すよう変更)
- `strategy_game/src/war/{data,justification,peace}.rs`(UI配線済みエラー文字列を翻訳キー化)
- `strategy_game/src/war/tests.rs`(エラー文字列アサーション7箇所を新キーへ更新、
  テストの厳密さは維持・弱化なし)
- `strategy_game/src/diplomacy/mod.rs`(`TranslationCorePlugin`追加)
- `audit_report.md`, `walkthrough.md`(本セクション追記)

保護対象2ファイルは内容変更なし。

## 13. 証拠保存場所

`strategy_game/verification_logs/p20-009/`
- `pre_audit/`: 作業開始前状態の記録
- `string_audit/`: 表示文字列監査・翻訳キー対応表・フォント調査
- `regression/`: 全回帰コマンドの生ログ
- `screenshots/`: ja-JP/en-US/切替後のPNG + SHA-256
- `12_final_judgment.md`: 判定根拠の一覧表

## 14. 保護対象SHA-256(本作業開始時・終了時)

| 対象 | 開始時 | 終了時 | 判定 |
|---|---|---|---|
| `strategy_game/assets/data/states.ron` | `c5fab07532a6c651a1f54962f78653d3a8518f9041475f52a29507e3ad39dc24` | `c5fab07532a6c651a1f54962f78653d3a8518f9041475f52a29507e3ad39dc24` | PASS |
| `strategy_game/tests/land_war_combat_peace_test.rs` | `06f7cfee9e2413dec6f18b4f5af86b82ff5556152c4f2026ede8c4d3142796a9` | `06f7cfee9e2413dec6f18b4f5af86b82ff5556152c4f2026ede8c4d3142796a9` | PASS |

## 15. フェーズ判定(最新・最終)

| 項目 | 判定 |
|---|---|
| Phase 20B-1i | PASS |
| P20-007 | RESOLVED(維持) |
| P20-008 | RESOLVED(維持) |
| P20-009 | **RESOLVED** |
| Prototype v0.1 | **READY** |
