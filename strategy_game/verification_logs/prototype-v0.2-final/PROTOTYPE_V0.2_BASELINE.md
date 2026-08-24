# Prototype v0.2 最終固定基準文書

監査日: 2026-08-24
監査方式: Phase 21(P21-001〜P21-021、P21-SAVE系、P21-MAP-001)+ Phase 22(P22-001〜004)の
実装・テスト・コミット状態を実コードから再確認し、既存報告と食い違う場合は実コード・
テストを優先。新規ゲーム機能の追加なし。本監査自体は新規タスクとして実装は行わない。

## 1. 結論

**Prototype v0.2 を READY として固定する。**

判定根拠は本文書§6・§9を参照。

## 2. 基準コミット

```
77e4d1f0b0573890e37fdbc0c0c2b486612d7d1f  fix: reset terminal UI state after loading
```

このコミットに至るPhase 21〜22関連コミット履歴(最新→旧):

```
77e4d1f fix: reset terminal UI state after loading                              (P22-003)
a23f42d feat: notify observers when the world is unified                        (P22-002)
5d8e56a feat: establish Phase 21 integration and P22 victory baseline           (P21一式+P22-001)
8f172c5 Add performance results and summaries for crisis scaling and AI response verification
d83a05c Add completion report for P21-SAVE-003 ...
```

`5d8e56a`は、それまでワーキングツリー上に未コミットのまま蓄積していたP21-005〜P21-021・
P21-SAVE系・P21-MAP-001の全実装(country/diplomacy/map/military/save/state/ui/warの
各モジュール)を、P22-MAINT-001監査(4分類による棚卸し・非決定的PNGの安全な除外)を経て
一括で基準化したものである。本リポジトリはローカルのみで運用されており、いずれのコミットも
リモートへはpushされていない。

## 3. 対象Phase・タスク

| Phase/タスク | 内容 | 状態 |
|---|---|---|
| P21-001〜P21-021 | 募兵・Army・前線・防御/攻勢、資源チェーン、Claim/Crisis/Ultimatum、AI受諾・支持・Claim/Crisis開始、国力スコア、Crisis/War/講和接続、州所有権移転、国家消滅 | 実コード・テストで実在確認、コミット済み |
| P21-SAVE-001〜003 | セーブ/ロード基盤、起動直後「続きから」導線 | 実コード・テストで実在確認、コミット済み |
| P21-MAP-001 | 6か国28州への地図拡張 | 実コード・テストで実在確認、コミット済み |
| P22-MAINT-001 | Phase21+P22-001検証済み基準点の固定(4分類棚卸し、PNG安全復元、単一コミット化) | COMPLETE、コミット`5d8e56a` |
| P22-001 | プレイヤー世界征服検知と勝利オーバーレイ | COMPLETE、コミット`5d8e56a`内 |
| P22-002 | 観戦中の世界統一検知と終戦通知 | COMPLETE、コミット`a23f42d` |
| P22-003 | 終局状態のロード再導出とオーバーレイ順序E2E(ロード時の状態残留バグを発見・修正) | COMPLETE、コミット`77e4d1f` |
| P22-004 | 実クライアントでのP22-001〜003目視確認 | PASS、`verification_logs/phase-21/p22-004/p22-004_manual_verification_report.md` |

## 4. 実装済み機能(Prototype v0.1からの追加分、v0.1既存分は§13参照)

- Claim作成・Crisis開始/日次進行・Ultimatum(受諾/拒否/撤回)・Crisisから宣戦への接続
- AIのUltimatum受諾判断、Great Powerによる第三国支持とAIの支持判断
- 軍事・経済・人口から算出する国力スコアとGreat Power/Regional Power/Minor区分
- AI国家による既存Claimからの月次Crisis開始、AI国家による月次Claim生成
- Crisis/Claimと War・講和解決の接続、自動降伏および州所有権移転(首都再割当含む)
- 消滅国家の外交UI表示と行動禁止(Domain APIレベルでの二重拒否)
- **プレイヤー国家消滅時の敗北UX**(全画面オーバーレイ、観戦モードへの導線)
- **プレイヤー世界征服(最後の生存国)検知と勝利オーバーレイ**(P22-001)
- **観戦中(プレイヤー消滅後)の他国世界統一検知と終戦オーバーレイ**(P22-002、敗北→
  世界統一の表示順序を保証)
- **ロード成功時の終局UI状態(敗北/勝利/世界統一)の正しい再導出**(P22-003、ロード失敗時は
  現状態を維持)
- 6か国28州への地図拡張(P21-MAP-001)
- 関連するセーブ検証、localization、回帰テスト全般

## 5. 明示的な非対応機能(今後のPhaseで誤って追加しないこと)

**Phase 21〜22で意図的にスコープ外とした機能**:
- 経済勝利・技術勝利・宗教勝利などの追加勝利条件(勝利条件は「最後の生存国」のみ)
- 同盟陣営単位の勝利
- 複数国家が残っている状態でのゲーム終了
- 世界統一後のコマンド制限(統一後もサンドボックスとして観戦・プレイを継続できる)
- 自動的なタイトル画面への帰還、エンドクレジット、統計画面
- 複数セーブスロット、オートセーブ、非同期I/O
- HoI4型の師団テンプレートシステム
- Focusシステム(国家固有の分岐イベントツリー)
- 遺跡研究システム(本作固有の「魔法/古代遺跡」要素の研究への組み込み)

**リポジトリ全体としての方針(README/ジャンル定義に基づく恒久方針)**:
- 内政システムは複雑化させすぎない(Victoria 3ほど深い経済・利益団体シミュレーションは
  目指さない)
- 戦争と内政の両方を扱うが、目標水準はHoI4とVictoria 3の中間程度に留める
- 上記はPrototype v0.1〜v0.2を通じた恒久的な設計方針であり、将来のエージェントが
  「機能の充実」を理由にこれらを追加してはならない

## 6. 主要Domain API

| API | ファイル | 責務 |
|---|---|---|
| `country::elimination::{preflight,apply}_country_elimination` | `src/country/elimination.rs` | 国家消滅(完全併合)、Registry横断の清掃(Army/Frontline/Battle/War/Diplomacy/Claim/Crisis/Justification/PowerRegistry) |
| `state::ownership::{preflight,apply}_transfer_state_ownership` | `src/state/ownership.rs` | 州所有権移転の共通TOCTOU安全API |
| `diplomacy::ai_crisis_start::handle_monthly_ai_crisis_start` | `src/diplomacy/ai_crisis_start.rs` | AI国家の月次Crisis開始評価 |
| `diplomacy::ai_claim_generation::handle_monthly_ai_claim_generation` | `src/diplomacy/ai_claim_generation.rs` | AI国家の月次Claim生成 |
| `country::power::{evaluate_country_power,rebuild_country_power_monthly}` | `src/country/power.rs` | 国力スコア算出とPowerTier区分 |
| `country::detect_player_elimination` / `PlayerEliminationMode` | `src/country/mod.rs` | プレイヤー敗北検知(`CountryRegistry`から再導出、Save非対象) |
| `country::detect_player_victory` / `PlayerVictoryMode` | `src/country/mod.rs` | プレイヤー世界征服検知(同上) |
| `country::detect_world_conclusion` / `WorldConclusionMode` | `src/country/mod.rs` | 観戦中の世界統一検知(同上) |
| `save::runtime::reset_terminal_ui_state` | `src/save/runtime.rs` | ロード成功時のみ上記3終局Resourceを初期化(P22-003) |

## 7. SystemSetと重要な実行順序

`DailySimulationSet`(`src/app/time.rs`、`.chain()`で厳格に順序保証):

```
TimeUpdate → Economy → Research → Diplomacy → CountryAi → WarPreparation
    → MilitaryAi → FrontlineOrders → MilitaryAction → WarResolution → UiUpdate
```

`CountryAi`内の追加順序制約(P21-017/018で導入):

```
handle_monthly_ai_crisis_start
    .before(handle_daily_country_ai)
handle_monthly_ai_claim_generation
    .after(handle_monthly_ai_crisis_start)
    .after(handle_daily_country_ai)
```

`UiUpdate`内の終局検知Systemの順序(P22-001〜003で導入):

```
detect_player_elimination
detect_player_victory
detect_world_conclusion
    .after(detect_player_elimination)
    .after(detect_player_victory)
```

UI側の表示同期Systemの順序(P22-003で追加、`Update`スケジュール、いずれも上記detect系との
明示的順序):

```
sync_player_elimination_overlay.after(detect_player_elimination)
sync_player_victory_overlay.after(detect_player_victory)
sync_world_conclusion_overlay
    .after(detect_world_conclusion)
    .after(detect_player_elimination)
```

`PlayerVictoryMode`と`WorldConclusionMode`は判定条件(プレイヤーがActiveか消滅済みか)が
構造的に排他であり、同時にPendingへ遷移することはない(country側テストで固定)。
`WorldConclusionMode`のUI表示は`PlayerEliminationMode::DefeatOverlayPending`の間
`Display::None`のまま抑制され、敗北オーバーレイの解除後にのみ表示される(表示順序:
敗北→世界統一)。

## 8. セーブ形式

- `SAVE_FORMAT_VERSION_V1 = 1`(Phase 21〜22を通じて変更なし)
- `CountryStatus`フィールド(Active/Eliminated)は`CountryData`に`#[serde(default)]`付きで
  埋め込まれており、旧セーブ(このフィールドが存在しない)は自動的に`Active`として復元される
  (`old_format_ron_without_status_field_loads_as_active`テストで固定)
- `PlayerEliminationMode`/`PlayerVictoryMode`/`WorldConclusionMode`は3つとも
  `CountryRegistry`(セーブ済み)から毎フレーム再導出されるランタイムUI状態であり、
  `SaveGameV1`へは含まれない。ロード成功時のみ`save::runtime::reset_terminal_ui_state`が
  この3つを初期化し、失敗時は現在の状態を一切変更しない(P22-003)
- 旧セーブ互換移行: 追加フィールドは全て`#[serde(default)]`方式、専用マイグレーション
  コードは不要な設計を維持している

## 9. localization状態

- 対応言語: 日本語(ja-JP、既定)・英語(en-US、フォールバック)
- 翻訳キー数: **576**(両言語で完全一致、重複0、空値0、プレースホルダー完全一致を
  `p20_009_localization_resource_test.rs`8件で継続的に検証)
- P22-001〜002で追加した新規キー: `player_victory.{title,body,continue_button}`、
  `world_conclusion.{title,body,continue_button}`(いずれもJA/EN追加済み、ハードコード無し)
- P22-004の実クライアント確認で、JA/EN両言語の表示・placeholder展開に問題がないことを
  ユーザー本人が確認済み(`verification_logs/phase-21/p22-004/`)

## 10. 自動テスト件数

**1275件、全てPASS**(`cargo test --all-targets`、本監査で複数回独立再実行し再現)。
`cargo test -- --list`(lib単体)で1073件、integration testを含めた全体で1275件
(`cargo test --all-targets -- --list`)。

主なintegrationテストファイル(P21〜P22関連分のみ抜粋):
- `p21_017〜021_*_e2e_test.rs`: AI Claim/Crisis開始、Claim生成、Crisis-War-講和解決、
  州所有権移転、国家消滅
- `p22_002_world_conclusion_e2e_test.rs`: 敗北→世界統一の表示順序(1件)
- `p22_003_terminal_state_load_e2e_test.rs`: ロード時の終局状態再導出(7シナリオ)
- `profile_p21_021_elimination_scaling_test.rs`: 国家消滅の性能プロファイル(`#[ignore]`)
- 既存の`p21_005/008/010/011/012/013/015/016_*_e2e_test.rs`群、`land_war_combat_peace_
  test.rs`(保護対象)、`p21_save_002e/003_end_to_end_test.rs`、`p20_009_localization_
  resource_test.rs`、`ui_headless_render_test.rs`はいずれも継続してPASS

## 11. P22-004手動確認結果

`verification_logs/phase-21/p22-004/p22-004_manual_verification_report.md`を参照。
確認者はユーザー本人、確認日2026-08-24、全項目PASS(勝利オーバーレイの内容・レイアウト、
敗北→世界統一の表示順序、ボタン操作、再表示防止、JA/EN表示のいずれも問題なし)。
自動GUI操作やスクリーンショット証拠は本セッションの制約により作成していない
(ユーザー自身の実クライアント確認結果として記録)。

## 12. 性能・描画に関する既存基準

Phase 21〜22では、Prototype v0.1で確立した1000〜2000州規模でのCountry AI性能最適化
(`compute_total_power_by_country`/`compute_land_states_by_controller`のVecインデックス化)
に対する再測定・再最適化は実施していない。P21-016の性能検証(`verification_logs/phase-21/
p21-016/perf/`)はCrisis支持スナップショット生成と多国間War処理のスケーリング検証であり、
一般的な州数スケーリングの再測定ではない。したがって、Prototype v0.1の性能基準
(`verification_logs/prototype-v0.1-final/PROTOTYPE_V0.1_BASELINE.md`§6)が
Prototype v0.2でも引き続き有効な基準値として扱われる。P20-007のHeadless実描画テスト
閾値(`BG_TOLERANCE=16`、`MIN_NON_BACKGROUND_PIXELS=300`、`MIN_DIFF_PIXELS=50`)も
Phase 21〜22で変更されておらず、`ui_headless_render_test.rs`は継続してPASSしている。

## 13. 既知問題(存在を再確認、今回は修正しない)

| 項目 | 内容 | READYへの影響 |
|---|---|---|
| Playing画面PNG9枚の非決定的再生成 | `cargo test --all-targets`実行のたびに`verification_logs/{p20-007,p20-009,phase-21/p21-save-002e}/screenshots/`配下の9枚(Playing画面のみ、国選択画面は完全再現)がバイト単位で変化する。原因はUIパネルのHashMapベース描画順序の非決定性(既にPrototype v0.1で判明・文書化済み、`known_issues.md`項目3)。本監査でも再現を確認し、個別`git checkout --`で復元済み | **非該当**。シミュレーション状態・決定論には無関係、コミットには含まれない |
| `tests/land_war_combat_peace_test.rs`の意図的なrustfmt drift | 保護対象ファイル。Phase 20B-1i以降一貫して未整形のまま保護されている。今回`rustfmt --check`(読み取り専用)で再確認したが書き込みは行っていない。SHA-256は`bd7876d6052bb798d498769cc4bf5b6f1bcf0ce042b7d0c3ae550e9875b89a7d`(P21-MAP-001の6か国28州対応で内容自体は正当に更新済み、Prototype v0.1時点のハッシュ`06f7cfee...`とは異なる——「保護」の対象は常にフォーマット非変更であり内容凍結ではない) | **非該当**。フォーマットのみの既知差分、機能・テスト結果に影響なし |
| `military::ai::process_military_ai`の未接続scaffold | 初回コミット由来の空実装、どのSystemからも呼ばれていない(実処理は`war::combat::process_combat`と`country::country_ai`が担う)。本監査で再確認、変更なし | **非該当**。デッドコード、実行経路に含まれない |
| `military::combat::process_combat`引数なし版の未接続scaffold | 同上(`war::combat::process_combat`という同名の別関数が実際に使われる) | **非該当**。同上 |
| `p21-018_completion_report.md`の既知trailing whitespace | 92行目に1箇所。ドキュメントの体裁上の瑕疵のみ、コード・テストへの影響なし。本監査で再確認、変更なし | **非該当**。ドキュメントのみ |
| `cargo fmt --check`が広範囲でFAILする | 上記の保護対象ファイルに加え、`map/division_render.rs`・`map/mod.rs`・`military/movement.rs`・`military/supply.rs`・`tests/daily_system_integration_test.rs`・`tests/p21_save_003_end_to_end_test.rs`・`tests/profile_workload_correctness_test.rs`・`verification_logs/phase-21/p21-010-perf-verify/03_crisis_scaling/profile_crisis_scaling_source.rs`(アーカイブ済みソーススナップショット)に、Phase 20〜21のどこかの時点で生じた未整形の既存差分がある。本監査で個別`rustfmt --check`により実際に確認した(45ハンク)。今回のPhase 22作業では一切変更していないファイル群であり、`cargo clippy`・全テストはこれらのファイルを含めて継続してPASSしている | **非該当**。フォーマットのみ、コンパイル・clippy・テストいずれにも影響なし。ただし将来のPhaseで`cargo fmt`の範囲を明確化するタスクとして拾う価値はある(§Phase 23候補では選定していない、優先度は低い) |

いずれもPrototype v0.2のREADY判定を妨げる重大な問題とは判定しない(根拠は各行の
「READYへの影響」列を参照)。

## 14. 回帰判定条件(Phase 23以降で維持すべき基準)

- `DailySimulationSet`の実行順序(`.chain()`)、および§7に記載した`CountryAi`/`UiUpdate`内の
  追加順序制約
- `cargo test --all-targets`は1275件以上を維持し、既存テストを緩めない(新規テスト追加は歓迎)
- `cargo clippy --all-targets --all-features -- -D warnings`が0 warningsを維持
- 翻訳キー集合・プレースホルダーのja-JP/en-US完全一致(576キー以上)
- `SAVE_FORMAT_VERSION_V1`の後方互換(旧セーブが引き続きロード可能であること)
- `PlayerEliminationMode`/`PlayerVictoryMode`/`WorldConclusionMode`の3つがSave非対象で
  あり続けること、およびロード成功時のみ初期化されロード失敗時は現状態を維持するという
  契約(`save::runtime::reset_terminal_ui_state`)
- P20-007のHeadless実描画テスト閾値を弱めないこと
- P20-008のCountry AI最適化を個別計算との数値的完全一致を壊さずに保つこと

## 15. 次フェーズで保護すべきファイルまたは契約

- `tests/land_war_combat_peace_test.rs`: 引き続きフォーマット変更禁止(内容変更自体は
  従来通り可能、フォーマットのみ凍結)
- `assets/data/states.ron` / `assets/data/countries.ron`: 6か国28州の本番マップデータ。
  改行コード変換・無意味なフォーマット変更を避けること
- `country::CountryStatus` / `country::CountryRegistry::active_country_ids()`:
  国家消滅・生存国列挙の唯一の正規経路。これを複製する新しい判定ロジックを追加しないこと
- `PlayerEliminationMode` / `PlayerVictoryMode` / `WorldConclusionMode`の3 Resourceを
  Save対象へ安易に追加しないこと(ランタイム再導出という設計そのものが契約)
- `save::runtime::reset_terminal_ui_state`: 終局UI状態リセットの唯一の実装箇所。
  同じロジックを他のロード経路(将来複数セーブスロット等を追加する場合)へ複製せず、
  この関数を再利用または責務を明確に引き継ぐこと

## 16. 動作確認環境

- OS: Windows 11 Pro 10.0.26200
- Rust: rustc 1.97.1 / cargo 1.97.1
- Bevy: 0.19.0

## 17. 起動方法

```
cd strategy_game
cargo run                 # デバッグ実行
cargo run --release       # リリース実行
```

## 18. 検証方法(本監査で実行したコマンド)

```
git diff --check
rustfmt --edition 2024 --check <保護対象を除く追跡Rustファイル全件>
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

いずれもPASS(`cargo fmt --check`自体は§13に記載の既知drift群によりFAILするため、
`rustfmt --check`をファイル個別指定で実行する既存の安全な確認方法を用いた)。

## 19. 証拠への相対パス

すべて`strategy_game/verification_logs/`からの相対パス。

- Phase 21各タスク: `phase-21/p21-001/`〜`phase-21/p21-021/`
- Phase 21 SAVE系: `phase-21/p21-save-001/`〜`phase-21/p21-save-003/`
- P21-MAP-001: `phase-21/p21-map-001/`
- P22-MAINT-001: `phase-21/p22-maint-001/`
- P22-001〜003: `phase-21/p22-002/`, `phase-21/p22-003/`
  (P22-001単独の完了報告書は作成されず、会話履歴内の最終報告のみ)
- P22-004: `phase-21/p22-004/`(本監査で新規作成)
- 本監査: `prototype-v0.2-final/`
  - `PROTOTYPE_V0.2_BASELINE.md`(本文書)

## 20. Prototype v0.1からの差分要約

Prototype v0.1(`verification_logs/prototype-v0.1-final/PROTOTYPE_V0.1_BASELINE.md`)は
経済・研究・政治・外交・戦争・軍事AI・UI・localization・性能最適化という縦切りをREADYと
判定した。Prototype v0.2はその上に、Claim/Crisis/Ultimatum/AI外交判断/国力スコア/
戦後処理(州移転・国家消滅)/プレイヤー敗北・勝利・観戦者向け終局通知という、
「戦争が実際に国家の運命を左右する」ところまでの縦切りを追加した。テスト件数は
152件(v0.1)から1275件(v0.2)へ増加した。
