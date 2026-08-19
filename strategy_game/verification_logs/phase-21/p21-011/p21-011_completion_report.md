# P21-011 完了報告: Crisis最後通牒・平和受諾・戦争正当化への接続

日付: 2026-08-17

## 0. 前提: ベースライン数値の乖離について

本タスクの仕様書に記載されたベースライン数値(`cargo test --lib`=590, `--tests`=699,
integration=109, 2000州 normal≈0.2371ms, high_load≈1.0759ms, fmt=68 hunks/20 files)は、
本セッション内で先に完了した別タスク(タブバー/スクロール/スクロールバーのUIバグ修正、
lib純増35件)より前の状態を指しており、本タスク開始時点の実際のベースラインとは一致し
ない。実測して確認した本タスク開始時点の実際のベースラインは以下の通り:

- `cargo test --lib`: 625件
- `cargo test --tests`(lib込み合計): 734件 → integration側は734-625=109件(この数字だけは
  仕様書記載と一致)
- fmt: 直前のセッションで整形済みのため、本タスク開始時点でのhunks/files数は未計測
  (このタスクで新たに触れたファイルのみを対象にrustfmtを適用したため、以降の「fmt比較」
  節は「このタスクで触れたファイル群」を対象とした差分として報告する)

以降のテスト純増数・品質ゲートは、この実測ベースライン(625/734)を基準に報告する。

## 1. 事前調査結果(要約)

実装前に以下を実コード読解で確認した:
- `CrisisPhase`は既に`Preparing/DemandSent/Negotiating/Escalating/ResolvedPeacefully/
  WarStarted/Cancelled`の7バリアントを持ち、`DemandSent`(要求送付済み)と`Escalating`
  (激化中)が本タスクの「要求中」「拒否済み・宣戦待ち」の意味論とそのまま一致することを
  確認。新規バリアントの追加は不要と判断した。
- `WarJustification`は`{initiator, target, target_state, required_days, days_passed,
  is_ready}`のみを持つ完全に汎用的な構造体で、由来(Crisis経由か手動正当化か)を区別する
  フィールドが存在しないことを確認。`grant_completed_justification`(即座にis_ready=trueで
  付与)・`cancel_justification`(取消)という2つの新メソッドで対応可能と判断し、
  `WarJustification`自体の構造変更は不要とした。
- `WarRegistry::declare_war`の実呼び出し元は`country_ai.rs::process_war_declaration_ai`
  と`ui/diplomacy_panel.rs`の`DeclareWarButton`ハンドラの2箇所のみであることをgrepで確認
  (過去のP21-008の教訓通り、共有関数`declare_war`自体は変更せず、両呼び出し元に個別に
  同期フックを追加する設計とした)。
- `war/peace.rs::execute_peace_settlement`は既存Warの存在を前提とする講和専用処理であり、
  Crisis受諾(War不在)へ転用できるのは`StateRegistry::transfer_region_ownership`という
  低レベルのプリミティブのみであることを確認。`execute_peace_settlement`自体は呼ばず、
  同じプリミティブだけを新しい`crisis_response::accept_demand`から直接呼ぶ設計とした。
- `diplomacy::ai::calculate_demand_acceptance`は実装済みだが未接続(呼び出し元ゼロ)である
  ことをgrepで確認。仕様通り、本タスクでは一切接続していない(AI側は常に無応答→期限切れ
  自動拒否のみで解決する)。
- `diplomacy_panel.rs`の既存`CrisisCommand::{RequestStart,Confirm,Cancel}`
  (`pending_crisis_claim`による2段階確認)パターンをそのまま踏襲し、
  `RequestAccept/ConfirmAccept/CancelAccept`・`RequestReject/ConfirmReject/CancelReject`
  を追加。撤回(`Withdraw`)のみ単発操作とした(既存パターンに前例がある確認フローと、
  破棄的操作ではあるが取り消し可能な性質を踏まえた設計判断)。

## 2. 採用したphase対応

| フェーズ | 意味 | 遷移元 |
|---|---|---|
| `DemandSent` | 最後通牒送付中(要求期間30日、`deadline_date`あり) | `start_crisis`が直接ここへ遷移(旧`Preparing`初期化を変更) |
| `Escalating` | 拒否済み・宣戦の正当性を保持(`related_justification_id`あり) | 拒否確定 or 期限切れ自動拒否 |
| `ResolvedPeacefully` | 受諾により平和的解決 | 受諾確定 |
| `WarStarted` | 実際に宣戦された | 既存宣戦処理からの同期 |
| `Cancelled` | initiatorによる撤回 | いつでも(DemandSent/Escalatingから) |

新規バリアントは追加していない(`Preparing`/`Negotiating`は本タスクの範囲では使用しない
まま残置)。

## 3. Claim消費済みの表現

`diplomacy::claims::ClaimStatus`(`Active`/`Consumed`、`#[serde(default)]`で`Active`)を
`TerritorialClaim`に追加し、`ClaimRegistry::mark_consumed`(冪等)で受諾時に`Consumed`へ
遷移させる。`CrisisRegistry::can_start_crisis`は`status != Active`のClaimからの新規Crisis
開始を`diplomacy_error.crisis.claim_consumed`で拒否する。

## 4. 受諾時に再利用した州移転処理

`state::data::StateRegistry::transfer_region_ownership`(P21-010以前から既存、
`war/peace.rs`の`CedeWarGoalRegion`講和条件が使うのと同じプリミティブ)をそのまま
`crisis_response::accept_demand`から呼び出している。新しい州所有権変更ロジックは
実装していない。

## 5. 拒否時のJustification生成方法

`WarJustificationRegistry::grant_completed_justification`(新規メソッド)が、
既存正当化があれば即座に完了扱いへ引き上げ(冪等)、なければ`is_ready=true`
`days_passed=required_days`の状態で新規作成する。通常の30日進行(`start_justification`
→`process_daily_justifications`)は一切経由しない。

## 6. 既存宣戦処理との接続

`WarRegistry::declare_war`自体は変更していない。実呼び出し元2箇所
(`country_ai.rs::process_war_declaration_ai`、`ui/diplomacy_panel.rs`の
`DeclareWarButton`ハンドラ)の両方で、`declare_war`呼び出し直前に
`justification_registry.get_ready_justification(...)`で正当化idを捕捉し
(`declare_war`が内部で消費・削除するため、消費後では手遅れ)、成功後に
`crisis_response::sync_crisis_on_war_declared`を呼んでいる。一致するEscalating中の
Crisisがあれば`WarStarted`へ遷移し、`related_war_id`を設定、`related_justification_id`
は`None`へ戻す(理由: `declare_war`自体が直前にjustificationをRegistryから削除する
ため、idを保持し続けるとsave検証がdangling referenceとして拒否してしまうことを
E2Eテストで実際に検出し、修正した)。

## 7. 旧セーブ移行

以下すべて`#[serde(default)]`付きで追加し、旧セーブ(フィールド自体が存在しないRON)は
自動的に安全な既定値へ復元される:
- `TerritorialClaim.status` → `ClaimStatus::Active`
- `DiplomaticCrisis.related_claim_id` / `related_justification_id` / `related_war_id`
  → すべて`None`

`src/diplomacy/claims.rs`・`src/diplomacy/crisis.rs`にそれぞれ、実際に
`ron::to_string`→フィールド除去→`ron::from_str`で往復させる後方互換テストを追加し、
確認済み。

## 8. 変更ファイル一覧

新規:
- `src/diplomacy/crisis_response.rs`(accept/reject/withdraw/sync-on-war-declaredのドメイン
  関数、テスト11件を含む)
- `tests/p21_011_crisis_ultimatum_e2e_test.rs`(軽量App + 実マップE2E、5件)

変更:
- `src/diplomacy/claims.rs`(`ClaimStatus`追加、`mark_consumed`追加)
- `src/diplomacy/crisis.rs`(`related_claim_id`/`related_justification_id`/`related_war_id`
  追加、`start_crisis`のphase/deadline初期化変更、`can_start_crisis`にconsumed拒否追加)
- `src/diplomacy/mod.rs`(新モジュール登録)
- `src/diplomacy/tests.rs`(`status`フィールド追加によるフィクスチャ修正)
- `src/diplomacy/update.rs`(`handle_daily_diplomacy`に期限切れ自動拒否ブロックを追加)
- `src/war/justification.rs`(`grant_completed_justification`/`cancel_justification`追加)
- `src/war/tests.rs`(上記2メソッドのテスト3件追加)
- `src/country/country_ai.rs`(`process_war_declaration_ai`/`process_daily_country_ai`/
  `handle_daily_country_ai`へ`CrisisRegistry`パラメータ追加、宣戦成功時の同期呼び出し追加)
- `src/ui/diplomacy_panel.rs`(`CrisisCommand`拡張、`DiplomacyPanelState`拡張、UI描画拡張
  [期限表示・受諾/拒否/撤回ボタン・正当性獲得表示]、`DeclareWarButton`ハンドラの同期フック、
  `handle_diplomacy_action_buttons`のSystemParam数がBevyの16個上限へ到達したため
  `locale`+`catalog`を既存の`Loc`ヘルパーへ統合)
- `src/save/{apply,dto,export,validate}.rs`(`status`/`related_*`フィールド追加による
  フィクスチャ修正、`validate.rs`に新規検証ルール7件のテスト追加)
- `assets/localization/{ja-JP,en-US}.ron`(新規UI文言・エラーキー・通知キーを両言語に
  追加、`real_catalog_loads_and_ja_en_key_sets_match`/
  `real_catalog_placeholders_match_between_locales`で整合性確認済み)
- `tests/p21_010_claim_crisis_e2e_test.rs`(フィールド追加によるフィクスチャ修正のみ、
  既存アサーション内容は無変更)

**変更していない**: `assets/data/states.ron`、`assets/data/resources.ron`、
`land_war_combat_peace_test.rs`、`DailySimulationSet`の列挙順、既存の宣戦布告日付ガード、
既存のSnapshot、ヘッドレス閾値定数(`BG_TOLERANCE`/`MIN_NON_BACKGROUND_PIXELS`/
`MIN_DIFF_PIXELS`)。

## 9. テスト純増数と全件数

実測ベースライン(625/734、上記0節参照)を基準とする。

| | ベースライン | 完了後 | 純増 |
|---|---|---|---|
| `cargo test --lib` | 625 | 656 | **+31** |
| `cargo test --tests`(lib込み合計) | 734 | 770 | **+36** |
| うちintegration(tests合計 − lib) | 109 | 114 | +5 |

新規テストの内訳: `diplomacy::claims`+2(消費済み往復・mark_consumed)、
`diplomacy::crisis`+2(deadline初期化・consumed拒否)、`diplomacy::crisis_response`+11
(accept/reject/withdraw/sync全経路)、`war::tests`+3(grant/cancel justification)、
`ui::diplomacy_panel`+6(UI経路のaccept/reject/withdraw、非当事者拒否含む)、
`save::validate`+7(新規参照整合性・phase一貫性ルール)、
`tests/p21_011_crisis_ultimatum_e2e_test.rs`+5(期限切れ自動拒否×2、実マップ
受諾/拒否宣戦同期/撤回のsave往復×3)。既存テストは一切削除・弱体化していない
(P21-008/P21-009/P21-010のE2E・2000州性能ベンチマークも含め全て現存)。

## 10. 性能測定

### 状態数スケーリング(`profile_1000_states`、100/500/1000/2000州 × normal/high_load)

2000州: normal mean=0.2631ms, high_load mean=1.3780ms(単発計測)。

P21-011の新規コード(`handle_daily_diplomacy`内の期限判定ブロック)は
`CrisisRegistry`が空であれば実質O(0)であり、この標準ベンチマークは`CrisisRegistry`へ
一切Crisisを注入しない(`profiling.rs`にCrisisRegistry関連コードが存在しないことを
grepで確認済み)。したがって状態数スケーリングの数値は構造的にP21-011の影響を受けない。
先行タスク(P21-010-PERF-VERIFY)が同一手法(単発計測はノイズが大きく0.2〜0.7ms程度で
変動しうる)で既に確認済みであるため、本タスクではA/B再検証は行わず、単発計測による
「壊れていないことの確認」に留めた。

### Crisis件数スケーリング(専用一時プロファイラ、本流へは残していない)

2000州固定、DemandSentフェーズ(期限は遠い未来固定、期限判定filterが最悪ケースで毎日
全件スキャンする状況)でCrisis件数を0/1/100/1000件と変化させて計測:

| crisis_count | mix | overall mean(ms) | Diplomacy set mean(ms) |
|---|---|---|---|
| 0 | DemandSent(遠未来) | 0.27430 | 0.03818 |
| 1 | DemandSent(遠未来) | 0.26216 | 0.03627 |
| 100 | DemandSent(遠未来) | 0.26747 | 0.04196 |
| 1000 | DemandSent(遠未来) | 0.34213 | 0.10922 |
| 1000 | ResolvedPeacefully(terminal) | 0.26441 | 0.04188 |

1000件のDemandSent中Crisisがあっても全体の日次tickは0.34ms程度(0件時0.27msから
+0.07ms)に収まっており、線形増加はしているが破綻的な増加ではない。1000件のterminal
Crisisはbaseline(0.264ms)と同水準で、期限判定filterが正しくterminal状態を除外
できていることも確認できた。

## 11. 品質ゲート結果

- `cargo check --all-targets`: 成功
- `cargo test --lib`: 656 passed, 0 failed
- `cargo test --tests`(デフォルト並列): 770 passed(lib 656 + integration 114), 0 failed
- `cargo clippy --all-targets -- -D warnings`: 警告0
- `cargo build --release`: 成功
- `git diff --check`: 実質的な空白エラーなし(CRLF/LF正規化警告のみ、pre-existing)
- ヘッドレス描画テスト実行のたびにチェックイン済みスクリーンショット8件が副作用で
  dirtyになった(既知の挙動)。毎回`git checkout --`で元に戻した(セッション内で
  既に確立済みの対応方針を踏襲)。

## 12. fmt比較

このタスクで実際に触れたファイルのみへ`rustfmt --edition 2024 <file>`を個別適用した
(ワークスペース全体への`cargo fmt`は実行していない)。適用後、リポジトリ全体を対象に
`cargo fmt --check`を実行したところ、**hunks=0, files=0**(仕様書記載の68 hunks/20 files
というベースラインから改善)。これは触れたファイル群の中に、以前から存在していた
未整形箇所(例: `country_ai.rs`の`NoAvailableDivisions`周辺)が含まれており、
ファイル単位でのrustfmt適用がその副作用としてまとめて解消したためである
(このプロジェクトで既知の「rustfmtはファイル単位で丸ごと整形する」制約の範囲内の挙動)。

## 13. verification_logs差分

新規追加(削除・上書きなし):
- `verification_logs/phase-21/p21-011/p21-011_completion_report.md`(本報告書)
- `verification_logs/p20-008/p21011_final_state_scaling/`(状態数スケーリング計測結果)
- `verification_logs/phase-21/p21011_crisis_scaling_verify/`(Crisis件数スケーリング
  計測結果)

セッション開始時点で存在していた未追跡ディレクトリ(`verification_logs/p20-008/
baseline_verify_run*`, `p21010-verify-run*`, `verification_logs/phase-21/
p21-010-perf-verify/` — いずれも前タスクP21-010-PERF-VERIFYの成果物)は一切削除・
変更していない。

## 14. 実GUI確認項目(未実施であることの明示)

**実際にゲームを起動してのGUI操作確認は行っていない。** 本タスクの検証はすべて自動化
テスト(Bevy `App`を用いたヘッドレスなユニット/統合テスト、実7か国28州マップデータ経由の
E2Eテスト)によるものであり、実際のウィンドウでマウス操作を行った確認ではない。

手動確認が必要な項目(次回セッションまたはユーザー自身による確認を推奨):
1. 外交パネルでClaim作成→Crisis開始した際、フェーズ表示が「要求送付済み」になり、
   期限日が表示されること。
2. 対象国側(自国がtargetの場合)で「受諾」「拒否」ボタンが表示され、押すと確認
   プロンプト→確認ボタンの2段階になっていること。「受諾」で対象州の色がマップ上で
   変わること。
3. 「拒否」後、initiator側で「戦争の正当性を獲得済みです」の表示が出て、既存の
   宣戦布告ボタンから実際に宣戦できること。
4. initiator側の「撤回」ボタンが単発で機能し、Escalating中に撤回すると正当性表示が
   消えること。
5. JA/EN切り替えボタンで上記の新規UI文言がすべて即座に切り替わること。
6. (Task Bで修正済みの)外交パネルのスクロール・スクロールバーが、Crisis一覧セクションの
   行数増加(受諾/拒否/撤回ボタン追加により縦に伸びる)後も引き続き正常に機能すること。

## 15. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

自動テスト・品質ゲートはすべて成功しているが、上記14節の実GUI操作確認が未実施のため、
「完全に検証済み」とは言えない。ロジック面・save互換性・性能面については十分な自動
検証を行った。
