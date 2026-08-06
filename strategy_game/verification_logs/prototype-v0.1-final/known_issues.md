# Prototype v0.1 最終固定監査 — 既知の問題一覧

いずれも独立再検証の過程で発見したもの。ゲーム仕様・シミュレーション決定論・
SystemSet順序・P20-008最適化には影響しない。重大な未解決問題はない。

## 1. P20-001〜P20-006のトレーサビリティ欠落(文書上の既知の限界)

`audit_report.md` / `walkthrough.md` は「Phase 20B-1i」セクションから始まり、
P20-001〜P20-006を個別に監査・判定した記録がリポジトリ内のどこにも存在しない
(grep調査: `P20-00[1-6]` はP20-009の`12_final_judgment.md`内の一般的な言及
1箇所を除き0件)。Gitコミット履歴(全11件)も、Phase 20B-1i以前のコミットは
`feat: implement core mechanics for research, politics, diplomacy, and economy
systems ...`のような機能単位のコミットメッセージのみで、P20-00Xという
フェーズ番号のラベルは一切付与されていない。

**影響**: P20-001〜P20-006が「実施された」こと自体は、対応する機能
(経済・研究・政治・外交・戦争・軍事・UI)が実際にコードとして存在し、
152件のテストで検証されていることから間接的に推測できるが、P20-007/008/009
のような個別の受入条件・独立監査・証拠ログを伴う形での「RESOLVED」判定は
存在しない。本監査ではこれを「Phase 20B-1iより前は監査証跡の対象外
(この監査フレームワーク自体がPhase 20B-1iから開始された)」と明記し、
存在しない記録を捏造しなかった。

**対応**: 修正不要(該当する不具合ではなく、監査対象時代以前の記録欠落)。
PROTOTYPE_V0.1_BASELINE.mdに明記。

## 2. P20-008の成果がP20-009と同一コミットに混在

`git log --oneline --all -- strategy_game/src/profiling.rs
strategy_game/src/bin/profile_1000_states.rs` の結果、P20-008の成果物
(profiling.rs, profile_1000_states.rs, country_ai.rs最適化)は単独のコミットを
持たず、`8a8e8f4 Add Japanese localization and font support for P20-009`という
P20-009向けの単一コミットにまとめて含まれていた。

**影響**: 機能的な問題はない(コードは実在し152テストに含まれ全PASS)。
Git粒度でのフェーズ追跡がしづらいという記録上の粗さのみ。

**対応**: 修正不要(過去コミットの分割はやり直しが利かず、保護対象外の
既存コミット履歴を書き換える行為(rebase等)は本監査の権限外)。

## 3. P20-009スクリーンショットのSHA-256記録が実ファイルと不一致(修正・再収録)

`strategy_game/verification_logs/p20-009/screenshots/png_sha256.txt` に記録された
`04_playing_ja_jp.png` / `05_playing_en_us.png` / `06_playing_ja_jp_again.png` の
SHA-256は、現在Gitにコミットされている実ファイルのSHA-256と一致しなかった
(`01`〜`03`の国選択画面PNGは一致していた)。

タイムスタンプ調査の結果、`png_sha256.txt`(22:44:01作成)より後に
`04`〜`06`のPNGが再生成(22:48:15)されており、記録更新が漏れたまま
コミットされたことが原因と判明した。

**根本原因の追加調査**: 本監査で`cargo test`を複数回独立実行した結果、
国選択画面(01〜03)のPNGバイト列は実行のたびに完全に同一(ビット単位で
再現可能)だったのに対し、Playing画面(04〜06)のPNGバイト列は実行ごとに
異なった。ただし**同一プロセス実行内でのja→en→ja往復では、04と06は
常にSHA-256完全一致**(本監査で3回の独立実行すべてで確認)であり、
言語切替の往復描画が決定的であるという製品の主張自体は損なわれていない。
Playing画面はUIパネル(研究・政治・外交・軍事)を開いた状態を含み、
描画順序に依存する要素(HashMapベースのUI要素等)がプロセス起動ごとに
サブピクセル単位でわずかに異なる可能性が高いと推測されるが、これは
シミュレーション状態(国庫・人口・戦争数等)や決定論には一切影響しない
(該当テストは`SimSnapshot`による厳密な状態一致を別途アサートしており、
そちらは常にPASSしている)。

**対応**: `strategy_game/verification_logs/p20-009/`配下の既存ログは
指示に従い上書きしていない。本監査で新たに取得した、現在のコードに対する
自己整合的なPNGとSHA-256を`prototype-v0.1-final/screenshots/`に保存した。

## 4. 統合テスト`fallback_to_en_us_works_for_synthetic_missing_ja_key`の名称と実装の乖離(軽微・非ブロッキング)

`tests/p20_009_localization_resource_test.rs`内のこのテストは、関数名が
示唆する「ja-JPに存在しないキーでen-USへのフォールバックを検証する」動作を
実際には行っていない(テスト自身のコメントに「実カタログにはそのような
キーが存在しないため検証できない」旨が明記されている)。これは構造的に
妥当である: 同ファイル内の`ja_jp_and_en_us_key_sets_match_exactly`が
両言語のキー集合完全一致を強制しているため、実カタログにフォールバックが
発生する状況を作ること自体が不可能。

フォールバック機構自体は`src/localization.rs`の
`#[cfg(test)] translate_falls_back_to_en_us_when_ja_jp_key_missing`
(合成カタログを用いた単体テスト)で正しく検証されている。

**対応**: 修正不要(機能欠陥ではなくテスト命名の紛らわしさのみ)。

## 5. ICU4Xログノイズ ("ICU4X data error: No segmentation model for language: ja") — 根本修正済み

前セッションでのLogPluginフィルタによる抑制の試みが実際には無効であることを
本監査中に実機検証で発見した(`RUST_LOG`環境変数を明示指定しても抑制されず)。

**根本原因**: `icu_provider`クレートは、"logging" Cargo機能が無効かつ
`debug_assertions`が有効(=通常の`cargo run`/`cargo test`/`cargo check`の
dev profile)の場合、`log`/`tracing`のファサードを一切経由せず
`std::eprintln!`へ直接フォールバックする実装になっている
(`icu_provider-2.2.0/src/lib.rs:192-204`)。releaseビルドでは
`debug_assertions`が偽になるため、この経路は最初から無効(=releaseビルドでは
元々発生しない)。

**修正**: `strategy_game/Cargo.toml`に`icu_provider = { version = "2.2.0",
features = ["logging"] }`を追加し、ログが`log`クレート経由の正規パイプラインを
通るようにした。`src/main.rs`のLogPluginフィルタ(`icu_provider=error`)と
合わせて、実機の`cargo run`でメッセージが出力されないことを確認した
(修正前後の生ログ: `regression_logs/11b_cargo_run_first_launch_before_icu4x_fix.log`
と`regression_logs/11_cargo_run_icu4x_verification.log`)。

**対応**: 修正済み・実機検証済み。ゲームロジック・アセット・保護対象ファイルは
無関係のため影響なし。

## 6. 外交/講和/政治パネルのショートカットキー重複・WASD衝突 — 本監査より前のセッションで修正済み

本監査開始時点で既に修正が適用済みだった(同一セッションの直前ターン)。
外交パネル: `KeyD`→`KeyG`(WASDカメラ移動の`KeyD`との衝突回避)。
講和パネル: `KeyP`→`KeyN`(政治パネルの`KeyP`との重複回避)。
本監査ではこれを既存コードとして扱い、152件のテスト全PASS(該当する
ヘッドレス描画テストの新キー割当を含む)で継続的に検証した。

**対応**: 追加修正不要。
