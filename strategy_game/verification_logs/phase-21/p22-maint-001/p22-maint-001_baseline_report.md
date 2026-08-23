# P22-MAINT-001 完了報告: Phase 21＋P22-001検証済み基準点の固定

## 0. 目的とスコープ

機能追加は行わず、監査済みのPhase 21実装(P21-001〜P21-021、P21-SAVE系、P21-MAP-001)と
P22-001(プレイヤー世界征服検知と勝利オーバーレイ)を、再現可能な「検証済み基準コミット」
として1件のローカルコミットに固定する。push・force push・rebase・amend・tag作成・reset・
stash・ブランチ切り替え・`git add -A`/`git add .`・ディレクトリ単位の無差別restore・
P22-002以降の実装は、ユーザー指示により今回一切行っていない。

## 1. 基準化前のHEAD

```
8f172c51aba37bd21ddd4b3e7b38227e1999ecf5
8f172c5 Add performance results and summaries for crisis scaling and AI response verification
```

ブランチ: `main`。作業開始時点で`git diff --cached`は空(steged変更なし)。

## 2. ワーキングツリー棚卸しと変更分類

`git status --short`・`git diff --stat`・`git diff --name-status`を実行し、全変更ファイルを
内容・Git差分・既存完了報告・テスト生成処理の突き合わせで4分類した(推測による分類は行っていない)。

### 分類A(コミット対象、48 tracked modified + 22 untracked)

以下の根拠により、全てP21-001〜P21-021・P21-SAVE系・P21-MAP-001・P22-001の意図された成果物と判定した。

- tracked modifiedの48ファイルは、本セッション開始前(このMaintenanceタスクの前に実行された
  NEXT-PHASE-001セッション開始時点)の`git status --short`と完全一致するファイル集合(country/
  diplomacy/map/military/save/state/ui/warの各モジュール、localization、tests/、
  verification_logs/phase-21/p21-016/)であり、いずれも既存の`p21-XXX_completion_report.md`群
  (`verification_logs/phase-21/p21-*/`)が記述する実装範囲と1:1で対応する。
- `verification_logs/phase-21/p21-016/`配下3ファイルの変更内容は、`p21-016_completion_report.md`
  の見出しが「二次レビュー対応: 軸2を1000 War x 100参加国まで拡張」と更新されており、既存の
  P21-016二次レビュー(War数拡張)による性能再計測の反映と確認できた(diffの実内容が既存報告書の
  改訂履歴と整合)。
- untracked 22件(ファイル+ディレクトリ)は、`src/country/elimination.rs`・
  `src/diplomacy/ai_claim_generation{,/tests.rs}`・`src/diplomacy/ai_crisis_start{,/tests.rs}`・
  `src/diplomacy/player_demand_alert.rs`・`src/state/ownership.rs`・`src/ui/player_elimination.rs`・
  `tests/p21_017〜021_*.rs`・`tests/profile_p21_021_elimination_scaling_test.rs`・
  `verification_logs/phase-21/p21-017〜021/`であり、いずれも対応する`p21-01[7-9]_completion_report.md`/
  `p21-02[01]_completion_report.md`が存在し、実装内容の記述とソースの`pub mod`宣言・関数シグネチャが
  一致することを確認した。加えて`src/ui/player_victory.rs`は本セッション(P22-001)で新規作成した
  ファイルである。
- 全Rust差分をコンテンツレベルで走査し(§4参照)、認証情報・APIキー・個人情報・巨大一時ファイル・
  ビルド成果物・環境依存絶対パスが含まれていないことを確認した。

### 分類B(生成画像、条件判定の上で復元)

`verification_logs/{p20-007,p20-009,phase-21/p21-save-002e}/screenshots/`配下の9ファイルが
`cargo test --all-targets`実行により非決定的に再生成されていた。判定・処理は§3を参照。

### 分類C(無関係なユーザー変更として保護)

該当なし。ワーキングツリー全体を走査した結果、P21/P22-001の実装範囲外の変更は検出されなかった。

### 分類D(分類不能)

該当なし。全ファイルが根拠を伴って分類Aまたは分類Bに分類できた。

## 3. PNG汚染の安全な処理

### 3.1 対象ファイルの正確な一覧(`git diff --name-status -- '*.png'`)

```
M	verification_logs/p20-007/screenshots/03_playing_topbar.png
M	verification_logs/p20-009/screenshots/04_playing_ja_jp.png
M	verification_logs/p20-009/screenshots/05_playing_en_us.png
M	verification_logs/p20-009/screenshots/06_playing_ja_jp_again.png
M	verification_logs/phase-21/p21-save-002e/screenshots/01_playing_topbar_with_save_load_buttons.png
M	verification_logs/phase-21/p21-save-002e/screenshots/02_after_save_click.png
M	verification_logs/phase-21/p21-save-002e/screenshots/03_load_confirm_dialog_open.png
M	verification_logs/phase-21/p21-save-002e/screenshots/04_after_cancel_dialog_closed.png
M	verification_logs/phase-21/p21-save-002e/screenshots/05_after_load_success.png
```

### 3.2 6条件の判定(全ファイル共通)

1. **HEADで追跡済みか**: `git ls-files --error-unmatch`で9件全てTRACKEDを確認。
2. **検証用画像であってソース/テストでないか**: 全て`verification_logs/*/screenshots/`配下。
3. **テスト実行による非決定的再生成か**: `git hash-object`の値がHEADのblobハッシュと不一致
   (例: `03_playing_topbar.png`は現在`79aa777...`、HEADは`3e33228...`)。さらに
   `verification_logs/prototype-v0.1-final/known_issues.md`項目3が、Playing画面(04〜06系列)PNGは
   プロセス実行ごとにバイト列が変わる既知の非決定性("国選択画面は完全同一、Playing画面は毎回異なる")
   と明記しており、本タスクでも`cargo test --all-targets`を2回実行し、両回とも同じ9ファイルだけが
   変化し、`p20-009`の国選択画面PNG(01〜03)は変化しないことを確認した(既知の機構と実測が一致)。
4. **今回の実装に必要な新規証拠画像でないか**: P22-001はスクリーンショットを一切生成していない
   (GUI自動操作手段なしのため証拠画像自体が存在しない)。該当せず。
5. **ユーザーの意図的編集の形跡がないか**: 9ファイルの更新タイムスタンプは、変化しなかった
   p20-009の01〜03(国選択画面)PNGとミリ秒単位で連続しており、単一のテストバイナリ実行による
   一括書き込みであることを裏付ける。個別の手動編集を示す痕跡はない。
6. **復元対象をファイル単位で明示できるか**: 上記9件を個別パスで列挙済み。

全9ファイルが6条件全てを満たしたため、`git checkout -- <個別パス>`をファイルごとに9回実行して
HEAD版へ復元した(ディレクトリ一括restoreは使用していない)。

### 3.3 検証実行後の再確認

§4の`cargo test --all-targets`実行後、同じ9ファイルが再度非決定的に変化したことを確認した
(条件3の再現性を追加で裏付け)。同じ6条件・同じ個別`git checkout --`手順で再度復元し、
`git status --short -- '*.png'`が空になったことを確認した。復元しなかった生成物はない
(9件全てが条件を満たし、9件全てを復元した)。

## 4. 実装差分の監査

- コンパイル不能な一時コード: なし(`cargo check --all-targets`常時PASS)。
- デバッグ用仮実装・無条件ログ出力: `git diff -- '*.rs'`を`todo!|unimplemented!|panic!\(\"DEBUG|
  println!\(\"DEBUG|dbg!|#\[ignore\]`で走査し0件。
- `todo!()`/`unimplemented!()`/意図しない`panic!()`: 追加なし(同上走査)。
- テストのskip/ignore化: `#[ignore]`の新規追加なし。既存の性能テスト1件(P21-021由来、
  `profile_p21_021_elimination_scaling_test.rs`系)が`#[ignore]`のままなのは変更前から一貫。
- テスト期待値の不当な弱体化: 目視差分で確認、該当なし。
- セーブ互換の破壊: `SAVE_FORMAT_VERSION_V1`・`SaveGameV1`の`pub`フィールド削除は差分中に0件
  (`git diff -- src/save/`を`^-.*pub \|SAVE_FORMAT_VERSION`で走査し該当なし)。`CountryStatus`
  フィールドは`#[serde(default)]`付きで後方互換。
- localizationキーのJA/EN一致: `en-US.ron`/`ja-JP.ron`とも573キーで一致数一致、
  `ja_jp_and_en_us_key_sets_match_exactly`・`placeholder_sets_match_between_locales_for_every_key`
  ともにPASS。
- 絶対パス・環境依存パス: `C:\\Users\\hirom`等のパターンで走査し0件。
- 認証情報・APIキー・個人情報・巨大一時ファイル: 走査0件、新規ファイルは全て数KB〜78KB台の
  Rustソース/Markdownのみ(§2参照)。
- ビルド成果物の誤追跡: `.gitignore`は`/target`を除外済み、`git status`に`target/`由来のエントリなし。
- P22-001の`PlayerVictoryMode`と既存の敗北処理(`PlayerEliminationMode`)の競合: 読み書きする
  Resourceが完全に分離しており、`ui::player_elimination::tests`(4件)と
  `ui::player_victory::tests`(4件)を同一セッションで両方再実行しPASSを確認。
  `detect_player_victory`は消滅済みプレイヤーでは発火しないことを専用テストで固定済み
  (`detect_player_victory_does_nothing_when_the_player_itself_is_eliminated`)。
- P22-001の縮退世界除外・冪等性・勝利後継続の仕様: `detect_player_victory_does_nothing_for_a_
  degenerate_single_country_world`・`detect_player_victory_does_not_revert_acknowledged_back_
  to_pending`・`continue_playing_button_transitions_mode_and_closes_overlay`の3テストで固定。
- 新規ファイルのmod登録漏れ: `src/country/elimination.rs`→`country/mod.rs`、
  `src/diplomacy/{ai_claim_generation,ai_crisis_start,player_demand_alert}`→`diplomacy/mod.rs`、
  `src/state/ownership.rs`→`state/mod.rs`、`src/ui/{player_elimination,player_victory}.rs`→
  `ui/mod.rs`の全`pub mod`宣言を`grep`で確認、漏れなし。
- 完了報告と実コードの一致: `verification_logs/phase-21/p21-017〜021/`の各完了報告が主張する
  Domain API名・System名を実ソースでgrepし、記述と一致することを確認した。

### 既知の未接続スタブ(今回削除・接続せず、存在のみ記録)

- `src/military/ai.rs::process_military_ai()` — 空実装、どのSystemからも呼ばれていない。
- `src/military/combat.rs::process_combat()`(引数なし版) — 空実装、`war::combat::process_combat`
  (実際に使用される同名別関数)とは別物で、こちらも未接続。

いずれも初回コミット(`9bf0eb2`)由来のscaffold残骸であり、今回の差分には含まれていない
(diff上で変更されていないことを確認済み)。

### 保護対象ファイルの既知のrustfmt drift(今回も維持)

`tests/land_war_combat_peace_test.rs`は、Phase 20B-1i監査以降一貫して「保護対象」として
rustfmt差分が意図的に残されているファイルである(`audit_report.md`のPhase 20B-1iセクション、
およびP20-007セクションの事故報告を参照)。今回`rustfmt --edition 2024 --check`をこのファイルに
対しても実行したが(読み取り専用のcheckのみ)、書き込みは一切行っていない。

## 5. 検証結果

| コマンド | 結果 |
|---|---|
| `git diff --check` | PASS、exit 0(autocrlf由来の警告のみ、コンフリクトマーカー等の実エラーなし) |
| `cargo check --all-targets` | PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | PASS、警告0 |
| `cargo test --lib country::tests::` | PASS、15 passed(P22-001の`detect_player_victory`系5件含む) |
| `cargo test --lib ui::player_victory` | PASS、4 passed |
| `cargo test --lib ui::player_elimination` | PASS、4 passed(競合しないことの確認) |
| `cargo test --test p20_009_localization_resource_test` | PASS、8 passed(JA/EN整合性・placeholder一致含む) |
| `cargo test --test p21_save_002e_end_to_end_test --test p21_save_003_end_to_end_test` | PASS、7 passed(セーブ/ロード主要回帰) |
| `cargo test --lib save::` | PASS、300 passed(save全モジュール) |
| `cargo test --all-targets`(2回実行) | PASS、**1250 passed; 0 failed**(lib 1057 + integration 193、両回とも同一件数) |
| `rustfmt --edition 2024 --check`(保護対象1件を除く差分対象59ファイル個別指定) | PASS、drift 0件 |
| `rustfmt --edition 2024 --check`(保護対象`land_war_combat_peace_test.rs`含む60ファイル) | 保護対象1件のみ既知drift、書き込みなし |

ワークスペース全体への`cargo fmt`は今回も実行していない。

### 未実施の手動確認

- P22-001勝利オーバーレイの実クライアントGUI上での見た目・レイアウト・ボタン操作感
  (GUI自動操作手段なし、前回セッションから継続する既知の制約)。
- 6か国マップでの実プレイによる世界征服到達自体の実地確認。

## 6. ステージングとコミット

`git add -A`・`git add .`は使用せず、分類Aに確定した70ファイル(tracked modified 48 +
untracked 22)を個別パス指定で`git add`した。ステージ後、`git status --short`・
`git diff --cached --stat`・`git diff --cached --name-status`・`git diff --cached --check`で
以下を確認した。

- ステージされたファイルは分類Aの70件+ 本レポート自身のみ。
- 分類B(PNG 9件)はステージ前に既にHEAD版へ復元済みのため、そもそも差分が存在せず
  ステージ対象にもならない。
- 分類C・Dは0件のため混入なし。
- `git diff --cached --check`はexit 0(コンフリクトマーカー・行末空白の実エラーなし)。

全条件を満たしたため、コミットメッセージ`feat: establish Phase 21 integration and P22 victory baseline`
でローカルコミットを1件作成した(push・tag・rebase・amendは実施していない)。

コミットハッシュ・`git show --stat --oneline HEAD`の内容は本レポートの外側(会話内最終報告)に記載する
(コミット作成はレポート確定後に実行するため)。
