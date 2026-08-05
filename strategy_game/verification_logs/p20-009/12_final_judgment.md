# P20-009 最終判定根拠

## 判定: RESOLVED

## 判定基準ごとの根拠

| # | 基準 | 判定 | 根拠 |
|---|---|---|---|
| 1 | 日本語・英語の翻訳リソースがある | 満たす | `assets/localization/ja-JP.ron` / `en-US.ron`, 336キー |
| 2 | 表示中に言語を切り替えられる | 満たす | `LanguageToggleButton`(top_bar・country_selection双方に配置), `CurrentLocale`リソース変更のみで即時反映 |
| 3 | 既存の表示テキストも切り替え時に更新される | 満たす | `LocalizedText`コンポーネント + `retranslate_on_locale_change`System。画面の再構築不要をHeadless実描画テストで実証(往復PNGがSHA-256完全一致) |
| 4 | 現在実装されている全UI表示文字列を監査した | 満たす | `verification_logs/p20-009/string_audit/01_display_string_inventory_and_migration.md` (src/ui/*.rs 10ファイル + 通知5ファイル + enum表示12ファイル + エラー文字列3ファイル) |
| 5 | 対象文字列が翻訳キーへ移行されている | 満たす | 336キー、除外は理由付きで9箇所(記号/空文字列/placeholder) + データ由来固有名詞(保護対象含め変更なし) |
| 6 | キー集合とプレースホルダーが両言語で一致する | 満たす | `p20_009_localization_resource_test.rs`: `ja_jp_and_en_us_key_sets_match_exactly`, `placeholder_sets_match_between_locales_for_every_key` |
| 7 | 未翻訳・欠落・重複・空翻訳を自動検出できる | 満たす | 同テストファイル: `no_duplicate_keys_in_either_locale_file`, `no_empty_translations_in_either_locale`, `missing_key_is_detected_not_silently_hidden` |
| 8 | 日本語フォントが実描画できる | 満たす | Noto Sans JP (SIL OFL 1.1) を`AssetId::<Font>::default()`へ差し替え。Headless実GPU描画で確認 |
| 9 | 日本語・英語の本番UIをHeadless実描画で確認した | 満たす | `p20_009_localization_headless_render_test.rs`: ja-JP/en-US/切替後en-US/再度ja-JPの4状態でPNG保存・ピクセル差分・非背景ピクセル数を検証 |
| 10 | ゲームロジックと決定論が不変 | 満たす | 同テストで`SimSnapshot`(国庫・人口・戦争数・AI状態数)が言語切替前後で完全一致することを確認。既存152件のシミュレーションテストも全てPASS |
| 11 | P20-007を維持 | 満たす | `tests/ui_headless_render_test.rs`を無変更のまま再実行、PASS(閾値も無変更) |
| 12 | P20-008を維持 | 満たす | `tests/profile_workload_correctness_test.rs`再実行PASS、`profile_1000_states`バイナリ再実行成功(全スケール・シナリオ完了) |
| 13 | 全テスト、check、clippy、run、diff checkが成功 | 満たす | 下記「回帰結果」参照 |
| 14 | `cargo fmt --check`の実結果を正確に報告した | 満たす | FAIL(保護対象の既知15差分のみ)と明記。新規・変更ファイルは`rustfmt`個別適用済み |
| 15 | 保護対象ハッシュが開始時と終了時で一致する | 満たす | 下記「保護対象ハッシュ」参照 |
| 16 | 実ログとPNGを保存した | 満たす | `verification_logs/p20-009/` 以下に格納 |

## 回帰結果

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | PASS |
| `cargo test` (全体) | PASS (152件, 0 failed) |
| `cargo test -- --list` | PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS (0 warnings) |
| `cargo run` (GUI起動+日本語表示確認+安全終了) | PASS (プロセス残存なし) |
| `cargo fmt --check` | **FAIL** — 原因: 保護対象`tests/land_war_combat_peace_test.rs`に作業開始前から存在する既知のrustfmt差分15箇所。新規・変更したRustファイル17個は`rustfmt`を個別パス指定で適用済みでFAILの原因ではない。保護指示によりこの既知差分は未修正。 |
| `git diff --check` | PASS (exit 0, CRLF変換警告のみ) |
| `cargo test --test ui_headless_render_test` (P20-007) | PASS |
| `cargo test --test profile_workload_correctness_test` (P20-008) | PASS |
| `cargo run --release --bin profile_1000_states -- <new_dir>` (P20-008) | PASS (全スケール/シナリオ完了) |

「fmtを含めてすべてPASS」とは記載しない。上記のとおり`cargo fmt --check`は保護対象の
既知差分によりFAILである。

## 保護対象ハッシュ

| ファイル | 開始時 | 終了時 |
|---|---|---|
| `strategy_game/assets/data/states.ron` | `c5fab07532a6c651a1f54962f78653d3a8518f9041475f52a29507e3ad39dc24` | `c5fab07532a6c651a1f54962f78653d3a8518f9041475f52a29507e3ad39dc24` (一致) |
| `strategy_game/tests/land_war_combat_peace_test.rs` | `06f7cfee9e2413dec6f18b4f5af86b82ff5556152c4f2026ede8c4d3142796a9` | `06f7cfee9e2413dec6f18b4f5af86b82ff5556152c4f2026ede8c4d3142796a9` (一致) |

## Prototype v0.1 判定

P20-009がRESOLVEDとなり、既存資料上でP20-001〜P20-008がすべてRESOLVED相当(Phase 20B-1i PASS、
P20-007 RESOLVED、P20-008 RESOLVEDは今回の再検証でも維持を確認)であることをコードとログから
確認できたため、**Prototype v0.1を READY とする**。
