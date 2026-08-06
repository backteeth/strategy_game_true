# P20-001〜P20-009 判定表(本監査による独立再判定)

| 項目 | 判定 | 根拠 |
|---|---|---|
| P20-001〜P20-006 | 監査対象外(記録なし) | リポジトリ内に個別の受入条件・監査証跡が存在しない。対応する機能(経済/研究/政治/外交/戦争/軍事/UI基盤)はコードとして実在し、152テスト中の対応するテストで継続的に検証されている。詳細は`known_issues.md`項目1。 |
| Phase 20B-1i | **PASS**(維持) | `src/app/time.rs`の`DailySimulationSet`定義・`.chain()`によるSystemSet順序、`tests/daily_system_integration_test.rs`のTest A〜F(全関数の存在をコードで確認)が現在のコードでも成立。152テスト全PASSに含まれ再現。 |
| P20-007 | **RESOLVED**(維持) | `tests/ui_headless_render_test.rs`の閾値(`BG_TOLERANCE=16`, `MIN_NON_BACKGROUND_PIXELS=300`, `MIN_DIFF_PIXELS=50`)は無変更。本番`UiPlugin`・実GPU(RTX 5070 Ti, Vulkan)によるHeadless実描画テストが現在のコードでもPASS。 |
| P20-008 | **RESOLVED**(維持) | `country_ai.rs`の`compute_total_power_by_country`/`compute_land_states_by_controller`(Vecインデックス化・遅延構築)が現在のコードに存在。1000/2000州releaseプロファイリングを本監査で再実測し、2000州normalシナリオでmedian 0.23〜0.36ms(初回未最適化ベースラインの5.057msから90%以上改善)を複数回にわたり再現。決定論・正しさの回帰テストも全PASS。 |
| P20-009 | **RESOLVED**(維持、軽微な証拠の鮮度問題を追加記録) | 翻訳キー336件・ja-JP/en-US完全一致・重複0・空値0・プレースホルダー完全一致を本監査で独立に再検証(Rustテストとは別に、シェルスクリプトで生RONファイルから直接抽出して照合)。ハードコード文字列除外9件も現行コードと一致。実行時言語切替・動的数値維持は、自動テストに加えて実際に起動したGUIへの本物のマウスクリックでも確認した(スクリーンショット2枚: 日本語→クリック→英語、数値2.33M/5000Gが不変)。P20-009自身のスクリーンショットSHA-256記録に鮮度の古さがあったため本監査で新規に整合したハッシュを記録した(`known_issues.md`項目3)。 |
| Prototype v0.1 | **READY** | 下記「最終判定根拠」参照。 |

## 最終判定根拠(READY)

- P20-007・P20-008・P20-009が全てRESOLVED(維持または本監査で再確認)。
- Phase 20B-1iがPASS(維持)。
- 全152テストがPASS(`cargo test`, exit 0)。
- `cargo check --all-targets`がPASS。
- `cargo clippy --all-targets --all-features -- -D warnings`が0 warningsでPASS。
- `cargo build --release --all-targets`がPASS。
- `cargo run`によるGUI起動・日本語UI表示・実クリックによる英語への切替・
  `WM_CLOSE`による安全終了・残存プロセスなしを実機で確認。
- `git diff --check`がPASS。
- `cargo fmt --check`はFAILだが、原因は保護対象
  `tests/land_war_combat_peace_test.rs`に作業開始前から存在する既知の
  rustfmt差分15箇所のみであり、新規・変更した全Rustファイルは個別に
  rustfmt準拠を確認済み。「fmtを含めて全てPASS」とは記載しない。
- 保護対象2ファイルのSHA-256が監査開始時・終了時で完全一致。
- 発見した軽微な問題(ICU4Xログの真の原因特定・修正、P20-009 PNGハッシュの
  鮮度)はいずれもP20-009またはツール/証拠の範囲内で解決済み。ゲーム仕様・
  AI判断・数値・SystemSet順序・P20-008最適化ロジックへの変更は一切行っていない。
- 未解決の重大な問題はない。
