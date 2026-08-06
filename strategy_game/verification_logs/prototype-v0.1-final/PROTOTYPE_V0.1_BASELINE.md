# Prototype v0.1 最終固定基準文書

監査日: 2026-08-06
監査方式: P20-001〜P20-009完了確認・資料間の矛盾解消・再現可能な基準情報の保存
(新規ゲーム機能の追加なし。本監査自体は新規フェーズ「P20-010」として扱わない)

## 1. 結論

**Prototype v0.1 を READY として固定する。**

判定根拠の詳細は`phase_judgment_table.md`を参照。本文書はその要約と、
今後Phase 21で参照すべき基準情報をまとめたものである。

## 2. Prototype v0.1で実装済みの機能

- 国家選択・ゲーム開始フロー(4か国から選択、実データプレビュー)
- 経済(建設キュー、資源、税制、月次収支)
- 研究(技術ツリー、配分、月次進捗)
- 政治(利益団体、改革、価値観3軸)
- 外交(関係値、条約、活動、危機、AI提案)
- 戦争(正当化、宣戦布告、前線生成・割当、日次戦闘解決、講和交渉・領土割譲)
- 軍事AI(戦争準備評価、前線割当、進軍)
- 日次シミュレーション(`DailySimulationSet`: TimeUpdate→Economy→Research→
  Diplomacy→CountryAi→WarPreparation→MilitaryAi→FrontlineOrders→
  MilitaryAction→WarResolution→UiUpdateの厳格な順序保証)
- UI(トップバー、国家/州/経済/研究/政治/外交/講和/軍事の各パネル、
  折りたたみ可能なパネルUI)
- 日本語・英語の完全ローカライズ、実行中言語切替
- 1000〜2000州規模でのCountry AIパフォーマンス最適化

## 3. P20-001〜P20-009 最終判定

詳細表は`phase_judgment_table.md`参照。要約:

| 項目 | 判定 |
|---|---|
| P20-001〜P20-006 | 監査対象外(個別記録がリポジトリに存在しない。機能自体はコード・テストで実在確認) |
| Phase 20B-1i | PASS |
| P20-007 | RESOLVED |
| P20-008 | RESOLVED |
| P20-009 | RESOLVED |
| **Prototype v0.1** | **READY** |

## 4. テスト件数

**152件、全てPASS**(`cargo test`、本監査で複数回独立再実行し毎回152/152で再現)。

内訳(`cargo test -- --list`より):
- ライブラリ単体テスト(`#[cfg(test)]`各所、localization/country_ai等): 93件
- `daily_system_integration_test.rs`(Phase 20B-1i Test A〜F含む): 6件
- `diplomacy_tests.rs`: 5件
- `economy_tests.rs`: 14件
- `land_war_combat_peace_test.rs`(保護対象): 2件
- `p20_009_hardcoded_string_scan_test.rs`: 4件
- `p20_009_localization_headless_render_test.rs`: 1件
- `p20_009_localization_resource_test.rs`: 8件
- `profile_workload_correctness_test.rs`: 9件
- `research_and_politics_tests.rs`: 9件
- `ui_headless_render_test.rs`(P20-007): 1件

## 5. 対応言語

- 既定: 日本語(ja-JP)
- フォールバック: 英語(en-US)
- 翻訳キー数: **336**(両言語で完全一致、重複0、空値0、プレースホルダー完全一致
  — 本監査でRustテストとは独立に生RONファイルを直接照合し再確認済み)
- 実行中の言語切替(TopBar/国選択画面のトグルボタン)で即時反映、動的数値
  (人口・国庫等)は切替の前後で不変

## 6. 代表性能値(1000〜2000州、releaseビルド、本監査での再実測)

計測環境: 本文書末尾「7. 動作確認環境」と同一。Seed固定
(`0x00C0FFEE12345678`)、60日次tick、10日warmup。

| 規模 | シナリオ | mean(ms) | median(ms) | p95(ms) | ticks/秒 |
|---|---|---|---|---|---|
| 1000 | normal | 0.3397 | 0.3617 | 0.6135 | 2943.9 |
| 1000 | high_load | 0.7802 | 0.7073 | 1.0865 | 1281.8 |
| 2000 | normal | 0.3710 | 0.3294 | 0.7075 | 2695.3 |
| 2000 | high_load | 1.1742 | 1.1334 | 1.4549 | 851.7 |

生データ: `strategy_game/verification_logs/p20-008/prototype_v0_1_final_verified/`
(最終Cargo.lock状態での計測)。参考: 最適化前ベースライン
(`p20-008/baseline/`)では2000州normal medianが5.057msであり、
90%以上の改善が現在のコードでも維持されている。

## 7. 動作確認環境

- OS: Windows 11 Pro 10.0.26200
- CPU: 12th Gen Intel(R) Core(TM) i7-12700F(12コア/20論理プロセッサ)
- メモリ: 31.79 GB
- GPU: NVIDIA GeForce RTX 5070 Ti(Vulkanバックエンド)
- Rust: rustc 1.97.1 / cargo 1.97.1
- Bevy: 0.19.0

詳細は`environment.txt`参照。

## 8. 起動方法

```
cd strategy_game
cargo run                 # デバッグ実行(通常の開発・確認用)
cargo run --release       # リリース実行
cargo build --release     # リリースビルドのみ
```

起動すると日本語UIの国選択画面が表示される。画面左上のトグルボタン
(既定表示「English」)をクリックすると英語表示へ即時切替できる
(本監査で実クリックにより実機確認済み。証拠:
`screenshots/manual_gui_02_ja_clean.png` →
`screenshots/manual_gui_03_en_after_real_click.png`)。

## 9. 検証方法

```
cargo check --all-targets
cargo test -- --list
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release --all-targets
cargo fmt --check          # 既知の理由でFAILする(下記10参照)
git diff --check
cargo run --release --bin profile_1000_states -- <output_subdir>
```

全コマンドの生ログ: `regression_logs/`配下。

## 10. 既知の制約

1. `cargo fmt --check`はFAILする。原因は保護対象
   `tests/land_war_combat_peace_test.rs`に作業開始前から存在する既知の
   rustfmt差分15箇所のみ(保護指示により未修正、`git diff --stat`では
   この保護対象ファイルの差分は0行)。新規・変更した全Rustファイルは
   個別rustfmt準拠を確認済み。
2. `assets/fonts/JapaneseFont.ttc`(初回コミットから存在する未使用ファイル)は
   Microsoft "MS Gothic"系のプロプライエタリフォントで再配布ライセンスが
   ないため、実際の日本語表示にはNoto Sans JP(SIL OFL 1.1)を新規追加して
   使用している。`.ttc`ファイル自体はP20-009の対応範囲外として削除せず
   保持されている(ユーザー判断待ち、P20-009報告書に既記載)。
3. P20-009のHeadless描画テストで生成されるPlaying画面のスクリーンショット
   (`p20-009/screenshots/04`〜`06`)は、シミュレーション決定論やUI言語切替の
   往復一致性には影響しないが、プロセス実行ごとにバイト列が完全には
   再現しない(国選択画面のスクリーンショットは常にバイト完全再現する)。
   詳細は`known_issues.md`項目3。
4. P20-001〜P20-006には個別の監査証跡がリポジトリに存在しない
   (`known_issues.md`項目1)。
5. デバッグビルド(`cargo run`, `cargo test`等)実行時、依存クレート
   `icu_provider`が有効化する`logging`機能により、日本語の行分割
   (word-wrap)が辞書ベースのセグメンテーションモデルではなくフォールバック
   アルゴリズムで行われる場合に警告ログが出る可能性がある(文字のグリフ
   表示自体には影響しない)。本監査でメッセージ自体の出力は抑制済み。

## 11. 保護対象ハッシュ(基準値・本監査開始時・終了時)

| ファイル | 基準値/開始時/終了時 | SHA-256 |
|---|---|---|
| `strategy_game/assets/data/states.ron` | 全て一致 | `c5fab07532a6c651a1f54962f78653d3a8518f9041475f52a29507e3ad39dc24` |
| `strategy_game/tests/land_war_combat_peace_test.rs` | 全て一致 | `06f7cfee9e2413dec6f18b4f5af86b82ff5556152c4f2026ede8c4d3142796a9` |

## 12. 証拠への相対パス

すべて`strategy_game/verification_logs/`からの相対パス。

- Phase 20B-1i: `phase20b-1i/`(既存)
- P20-007: `p20-007/`(既存、Headless実描画ログ・PNG)
- P20-008: `p20-008/`(既存の`baseline/`, `after_optimization/`,
  `addendum_smallscale_fix/`, `smallscale_fix_final/`に加え、本監査で
  追加した`prototype_v0_1_final/`, `prototype_v0_1_final_verified/`)
- P20-009: `p20-009/`(既存、翻訳監査・PNG・回帰ログ)
- 本監査: `prototype-v0.1-final/`
  - `PROTOTYPE_V0.1_BASELINE.md`(本文書)
  - `phase_judgment_table.md`
  - `known_issues.md`
  - `commands_executed.md`
  - `environment.txt`
  - `changed_files_final.txt`
  - `00_start_protected_sha256.log`, `00_end_protected_sha256.log`
  - `00_start_git_status_note.log`
  - `regression_logs/`(全検証コマンドの生ログ、修正適用後の最終版)
  - `screenshots/`(実GUIクリックによる手動確認の生スクリーンショット、
    P20-009 Headless描画PNGの再取得ハッシュ)

## 13. 今後のPhase 21で変更してよい範囲・維持すべき回帰基準

**変更してよい範囲**:
- 新規ゲーム機能・UI追加
- 未実装システムの拡張(例: 人口成長システム — `profiling.rs`の正しさ
  テストのコメントに「現行実装には人口"成長"システムが存在しない」と
  明記されている既知の未実装領域)
- パフォーマンスのさらなる最適化(ただし決定論・SystemSet順序を壊さないこと)
- P20-001〜P20-006相当の機能への追加テスト・監査証跡の後付け整備

**維持すべき回帰基準**:
- `DailySimulationSet`の実行順序(TimeUpdate→...→UiUpdate、`.chain()`)
- 152件の既存テストは全てPASSを維持すること(新規テスト追加は歓迎)
- `tests/land_war_combat_peace_test.rs`・`assets/data/states.ron`は
  引き続き変更・移動・削除・改行変換禁止(SHA-256は本文書11節の値で固定)
- P20-007のHeadless実描画テスト閾値(`BG_TOLERANCE=16`,
  `MIN_NON_BACKGROUND_PIXELS=300`, `MIN_DIFF_PIXELS=50`)を弱めないこと
- P20-008のCountry AI最適化(`compute_total_power_by_country`/
  `compute_land_states_by_controller`のVecインデックス化・遅延構築)を
  個別計算との数値的完全一致を壊さずに保つこと
- 翻訳キー集合・プレースホルダーのja-JP/en-US完全一致
- `cargo clippy --all-targets --all-features -- -D warnings`が0 warningsを維持
