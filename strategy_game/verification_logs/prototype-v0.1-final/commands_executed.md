# 本監査で実行した主なコマンド

## 開始時確認
- `git rev-parse --show-toplevel` / `git status --short` / `git diff --stat` / `git log -3 --oneline`
- `sha256sum strategy_game/assets/data/states.ron strategy_game/tests/land_war_combat_peace_test.rs`
- `rustc --version` / `cargo --version`
- PowerShell `Get-CimInstance Win32_OperatingSystem/Win32_Processor/Win32_VideoController`
- `grep` によるCargo.lockの主要依存バージョン抽出

## P20-009 独立検証(Rustテストとは別の一次データ照合)
- `grep -oE` による ja-JP.ron / en-US.ron のキー抽出・`sort`/`comm`/`uniq -d` による
  重複・欠落・キー集合一致の照合
- 自作Perlスクリプトによる `{placeholder}` トークン集合抽出・両言語diff
- `grep -nE '""\)'` による完全空値の検査
- `sha256sum` によるP20-009スクリーンショットPNGの実ハッシュとpng_sha256.txtの照合

## ビルド・テスト・静的解析(修正前・修正後の2ラウンド)
- `cargo check` / `cargo check --all-targets`
- `cargo test -- --list`
- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo build --release --all-targets`
- `cargo fmt --check`
- `git diff --check`
- `git status --short`
- `git diff --stat`

## P20-008 再計測
- `cargo run --release --bin profile_1000_states -- prototype_v0_1_final`(icu_provider修正前)
- `cargo run --release --bin profile_1000_states -- prototype_v0_1_final_verified`(icu_provider修正後、最終版)

## ICU4Xログ問題の切り分け
- `cargo run`(修正前、ICU4Xメッセージ再現)
- `RUST_LOG=icu_provider=off cargo run`(EnvFilter経路でないことの反証)
- icu_provider/icu_segmenter/parleyのベンダーソースをGrep/Readで直接調査し、
  `icu_provider-2.2.0/src/lib.rs`の`logging`機能フォールバック実装を特定
- `strategy_game/Cargo.toml`に`icu_provider = { version = "2.2.0", features = ["logging"] }`を追加
- `cargo run`(修正後、ICU4Xメッセージが出力されないことを確認)

## GUI手動確認
- `cargo run`(バックグラウンド起動)
- PowerShell: `AttachThreadInput`/`SetForegroundWindow`によるウィンドウ前面化
- PowerShell: `PrintWindow` APIによるウィンドウ内容のスクリーンショット取得
  (`CopyFromScreen`は他アプリのウィンドウに隠れて誤ったキャプチャになったため、
  遮蔽の影響を受けない`PrintWindow`へ切替。詳細は本文参照)
- PowerShell: `SetCursorPos` + `mouse_event` による実際の言語切替ボタンへの
  マウスクリック(日本語→英語の切替を実クリックで確認)
- PowerShell: `PostMessage(WM_CLOSE)` による安全な終了、`Get-Process`による
  残存プロセスなしの確認

## 終了時確認
- `sha256sum` による保護対象2ファイルの最終ハッシュ再検証
- `git status --short` / `git diff --stat` / `git diff --check`(最終)
