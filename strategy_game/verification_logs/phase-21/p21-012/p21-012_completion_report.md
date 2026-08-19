# P21-012 完了報告: AI国家によるCrisis受諾・拒否判断の接続

日付: 2026-08-17

## 0. 開始時の実測ベースライン

仕様書記載のベースラインを実測で確認した。**乖離なし**、記載値と完全に一致した。

- `git status --short`: P21-011セッションからの未コミット状態がそのまま残存(モジュール
  ファイル群+`verification_logs`配下の未追跡ディレクトリ)。既存の未追跡ファイル・
  ユーザー変更は一切変更・削除していない。
- `cargo test --lib`: 656 passed(一致)
- `cargo test --tests`: 656(lib) + 114(integration) = 770 passed(一致)
- `cargo clippy --all-targets -- -D warnings`: 0 warnings(一致)
- `cargo fmt --check`: 0 hunks / 0 files(一致)

## 1. `calculate_demand_acceptance`の既存仕様と接続方法

`src/diplomacy/ai.rs`にP21-010の時点から実装済みだったが、実呼び出し元はテスト
(`diplomacy::tests::test_demand_acceptance`)のみで、本番コードからは一切呼ばれていな
かった(grepで確認)。

シグネチャ: `(crisis: &DiplomaticCrisis, target_country: &CountryData, initiator_country:
&CountryData, target_states: &[&StateData], relation: &DiplomaticRelation,
target_allies_power: f32, initiator_allies_power: f32) -> f32`、戻り値は`-100.0..=100.0`
にclampされたスコア。関数自身のdocコメントに「正の値なら受諾、負の値なら拒否に傾く」と
明記されている。

**閾値解釈(`score > 0.0`→受諾、それ以外→拒否)の根拠**: (1) 関数自身のdocコメント、
(2) このスコアを最初に使った既存テスト`test_demand_acceptance`自身が`score > 0.0`を
「受諾すべき」の判定条件として直接アサートしている、(3) 同じ`-100.0..=100.0`スコア
パターンを使う既存コード`war/capitulation.rs`の`if war.war_score > 0.0`分岐と同型 ——
複数の独立した根拠が一致したため、独自解釈を新設せず、この解釈で実装した(`NEEDS USER
DECISION`が必要なほど曖昧ではないと判断)。

**同盟戦力(`target_allies_power`/`initiator_allies_power`)の扱い**: 関数はこの2引数を
既に受け取り、実際に計算式内で使用しているが、これを計算する既存ヘルパーはコードベース
のどこにも存在しなかった(唯一の既存呼び出し元であるテストは両方とも`0.0`を渡している)。
「関数の既存計算式を変更しない」という制約は守りつつ、関数が要求する入力自体は実際の
データから正しく埋めるべきと判断し、`diplomacy::update::compute_ally_power_by_country`
(新規、日次1回だけ`DiplomacyRegistry.relations`を1パスして`TreatyType::Alliance`の
ある相手国の`available_manpower`を合算する)を追加した。新しい外交補正値ではなく、
関数が既に宣言している既存パラメータへ実データを正しく渡しているだけである。この判断は
妥当と考えるが、仕様上明示されていなかった実装詳細のため完了報告で明記する。

## 2. AI国家とプレイヤー国家の判別方法

既存`PlayerCountry(pub Option<CountryId>)`リソースをそのまま使用。AI応答対象の絞り込みは
`player_country.0 != Some(crisis.target)`(targetがプレイヤー国家自身でなければAI応答
対象)。新しい判別APIは追加していない。

## 3. AIが応答する正確なタイミング

`diplomacy::update::handle_daily_diplomacy`(既存System、`DailySimulationSet::Diplomacy`
に登録済み、`DayChangedMessage`購読)の`for _event in day_events.read()`ループ内に新しい
ブロック(3a)として追加した。新しいSystemは作らず、既存のPause/一日一回保証をそのまま
継承する(`DayChangedMessage`はPause中・日付未変化のUpdateでは発行されないため)。

## 4. 期限切れ処理との優先順位

AI応答ブロック(3a)自身の対象条件に`!date.is_at_least(&deadline)`(期限未到達)を含めた。
これにより、期限に達したCrisisはAI応答ブロックの対象条件そのものから除外される ——
ブロックの実行順(3a→3bのタイムアウト処理)に依存せず、条件判定自体で優先順位を保証して
いる。回帰テスト`timeout_rejection_takes_priority_over_ai_acceptance_on_the_deadline_day`
で、AIが受諾するはずの国家データを与えつつ期限を初日に設定し、実際には
`Escalating`(タイムアウト拒否)になることを確認した。

## 5. 受諾・拒否で再利用したP21-011 API

`crisis_response::accept_demand`/`crisis_response::reject_demand`をそのまま呼ぶだけで、
`CrisisRegistry`/`ClaimRegistry`/`StateRegistry`/`WarJustificationRegistry`の各フィールド
をAI応答コードから直接書き換える箇所は一切ない。AI専用の状態変更ロジックは追加していない。

## 6. 拒否後の既存宣戦AIへの接続確認

`country_ai::process_war_declaration_ai`はP21-011の時点で既に
`justification_registry.justifications.values().filter(|j| j.initiator==country_id &&
j.is_ready)`という条件でjustificationの由来を問わずに拾う実装になっており、
`crisis_response::sync_crisis_on_war_declared`もP21-011の時点で既に接続済みだった。
本タスクではこの経路に**一切コード変更を加えていない** —— 実データE2Eテスト
`ai_initiator_ai_target_rejection_flows_through_existing_war_ai_to_war_started`で、
AI initiator(CountryId(1))・AI target(CountryId(3)、資産データ上で1-3間に既存条約なし)
の組み合わせで拒否→既存宣戦AI経由での`WarStarted`到達→`related_war_id`設定・
`related_justification_id`が`None`に解消されていること→save往復後も保持されることを
確認した。

**発見した実データ上の注意点(既存仕様、バグではない)**: 最初はCountryId(1)/(2)の組み
合わせでテストしたが、`assets/data/diplomacy.ron`に1-2間の`NonAggressionPact`が事前
設定されており、`declare_war`自身の既存条約チェックにより宣戦がブロックされ続けた
(正しい既存挙動)。CountryId(1)/(3)(無条約)に変更して解決した。AI応答処理自体は一切
関係ない、テストデータ選定の問題だった。

## 7. 複数Crisis処理の決定論保証

対象Crisis idは`evaluate_ai_crisis_responses`の冒頭で一度だけ`.filter().map().collect()`
して確定させてから処理する(既存のP21-011期限切れタイムアウト処理と同じパターン、
明示的なsortはしていない)。各Crisisの受諾/拒否結果は、そのCrisis自身の
initiator/target/target_state/relation/同盟戦力のみに依存し、同一tick内の他Crisisの
処理結果によって変化しない(受諾/拒否のいずれも他国のCountryData/StateData/
DiplomaticRelationを書き換えない)ため、`CrisisRegistry`のHashMap反復順に関わらず個々の
結果は不変。挿入順を変えた2パターンで同一結果になることをテストで確認済み。

## 8. Save形式変更の有無

**なし。** AIが応答済みかどうかは既存の`CrisisPhase`(`DemandSent`→`ResolvedPeacefully`/
`Escalating`)で表現でき、新しい永続フィールドは追加していない。

## 9. 変更ファイル一覧

変更:
- `src/diplomacy/update.rs`(`evaluate_ai_crisis_responses`・
  `compute_ally_power_by_country`を新規追加、`handle_daily_diplomacy`に呼び出しブロック
  3aを追加、`ClaimRegistry`/`StateRegistry`(ResMut化)をパラメータへ追加)
- `assets/localization/{ja-JP,en-US}.ron`(`notif.crisis_ai_accepted`/
  `notif.crisis_ai_rejected`の2キーを両言語に追加)
- `tests/p21_010_claim_crisis_e2e_test.rs`・`tests/p21_011_crisis_ultimatum_e2e_test.rs`
  (`handle_daily_diplomacy`の新パラメータ`ClaimRegistry`をテストハーネスへ追加する
  フィクスチャ修正のみ、既存アサーションは無変更)

新規:
- `tests/p21_012_ai_crisis_response_e2e_test.rs`(27件、詳細は次節)

**変更していない**: `assets/data/states.ron`、`assets/data/resources.ron`、
`land_war_combat_peace_test.rs`、`DailySimulationSet`の順序、P21-008のArmy攻勢処理、
P21-009のMagicCrystal資源チェーン、P21-011のプレイヤー向け受諾・拒否・撤回UI、
ヘッドレス描画テストの閾値定数。UIには一切手を加えていない(既存の`crisis_line`描画が
`current_phase`をそのまま反映するため、AI応答後のphase変更は追加コードなしで外交パネル
に自動反映される)。

## 10. 新規・更新テストの内訳

`tests/p21_012_ai_crisis_response_e2e_test.rs`: 27件

- AI判断(6件): 受諾条件で受諾/拒否条件で拒否/固定バイパスでないことの確認/player target
  は自動応答しない/AI initiator・player initiatorどちらでもAI targetが応答/第三国が
  playerでもAI targetの応答を妨げない
- 日次処理(7件): DayChangedMessageなしで無応答/Pause中無応答/1日で一度だけ応答/同日
  重複Updateで二重処理なし/期限前はAI判断使用/期限到達時はタイムアウト優先/terminal
  phaseは変更されない
- 受諾(5件): 州所有権移転/関連Claimなしでもクラッシュしない/War・Justification未作成/
  翌日再処理されない、+関連Claim消費の暗黙確認
- 拒否(2件): Escalating遷移+完成済みJustification1件のみ/AI応答単独ではWar未開始
- 決定論・複数件(6件): 挿入順が異なっても同一結果/受諾拒否混在で各1回のみ処理/無関係・
  terminal Crisisへの影響なし/target国参照dangling時panicしない/対象State参照dangling時
  panicしない/1000件規模で完走
- 実データE2E(2件、`real_map_e2e`サブモジュール): player initiator→AI target→
  `calculate_demand_acceptance`自身の判定と一致する結果でsave往復/AI initiator→AI
  target→拒否→既存宣戦AI→`WarStarted`→dangling参照なし→save往復

既存テストは一切削除・弱体化していない(P21-008/P21-009/P21-010/P21-011の全E2Eを含め
現存)。

## 11. 開始時と終了時の全テスト件数

| | 開始時(実測) | 終了時 | 純増 |
|---|---|---|---|
| `cargo test --lib` | 656 | 656 | +0 |
| `cargo test --tests`(lib込み合計) | 770 | 797 | **+27** |
| うちintegration | 114 | 141 | +27 |

lib側の純増が0件なのは、`evaluate_ai_crisis_responses`が`MessageWriter<GameNotification>`
を直接引数に取るためBevy Appなしでは単体テストできず、このプロジェクトの既存慣習
(`diplomacy_panel.rs`等、`MessageWriter`を取る関数は常に軽量/実Appを介してテストする)
に従い、全テストを`tests/p21_012_ai_crisis_response_e2e_test.rs`(integration)側へ
配置したため。

## 12. 品質ゲート結果

- `cargo check --all-targets`: 成功
- `cargo test --lib`: 656 passed, 0 failed
- `cargo test --tests`(デフォルト並列、ヘッドレスGPUテスト含む全件): 797 passed
  (lib 656 + integration 141), 0 failed
- `cargo clippy --all-targets -- -D warnings`: 0 warnings
- `cargo build --release`: 成功
- `git diff --check`: 実質的な空白エラーなし(CRLF/LF正規化警告のみ、pre-existing)
- `cargo fmt --check`: 0 hunks / 0 files(開始時基準を維持、新しいdriftなし)
- ヘッドレス描画テスト実行のたびにチェックイン済みスクリーンショット8件が副作用でdirtyに
  なった(既知の挙動)。毎回`git checkout --`で元に戻した。

## 13. 性能測定結果

生データ: `verification_logs/phase-21/p21012_ai_crisis_response_verify/`
(`summary.txt`/`results.csv`)。専用の一時プロファイラ(`src/bin/`へ一時登録して実行後、
`Cargo.toml`ごと削除、本流には残していない)を使用。2000州固定、AI target評価
(`calculate_demand_acceptance`)を実際に伴うDemandSent Crisisを0/1/100/1000件注入。

`calculate_demand_acceptance`は決定的な計算のため、対象となる全Crisisは初日で必ず受諾/
拒否いずれかへ解決される(複数日にわたって同じN件がDemandSentのまま滞留することはない)。
そのため60日平均だけでは初日のコストが希釈されてしまうため、初日(1日目)の所要時間を
明示的に報告する。

| crisis_count | 全体day_tick(1日目, ms) | Diplomacy SystemSet(1日目, ms) | 期限到達せず残留 |
|---|---|---|---|
| 0 | 1.009 | 0.097 | 0 |
| 1 | 0.931 | 0.086 | 0 |
| 100 | 1.067 | 0.147 | 1 |
| 1000 | 2.983 | 1.206 | 10 |

1000件でもDiplomacy SystemSet単体で1.2ms程度に収まっており、破綻的な非線形増加や
panicは発生していない(0→1000件でおおよそ線形、crisis 1件あたり約1.2μs)。

「期限到達せず残留」列は`still_demand_sent_after_day1`(初日評価後もDemandSentのまま
残った件数)。値は`crisis_count / 100`(=100国家中のプレイヤー国家CountryId(0)が
targetになる合成Crisisの割合)と正確に一致しており、「プレイヤー国家は自動応答しない」
ロジックが1000件規模でも意図通り機能していることの副次的な確認になった(バグではなく
正しい除外)。

`crises × countries`の総当たりは避けている: `country_by_id: HashMap<CountryId,
&CountryData>`と`ally_power_by_country: HashMap<CountryId, f32>`を日次1回だけ構築し、
Crisis単位ではO(1)で引く設計にした(`CountryRegistry::get`のO(n)線形探索を
crisis件数分繰り返さない)。

外部負荷によるノイズの疑いについて: 今回はP21-011のCrisis日次進行(既にnoiseと確認済みの
規模)と桁が近い変化であり、かつ新規コード自体が「AI評価対象0件のときはHashMap構築のみ」
という軽量な構造のため、単発測定でも十分に判断可能と判断し、P21-010-PERF-VERIFYのような
本格的な複数回A/B比較までは行っていない。

## 14. fmt比較

このタスクで実際に触れたファイル(`src/diplomacy/update.rs`、
`tests/p21_010_claim_crisis_e2e_test.rs`、`tests/p21_011_crisis_ultimatum_e2e_test.rs`、
新規`tests/p21_012_ai_crisis_response_e2e_test.rs`)にのみ`rustfmt --edition 2024 <file>`
を個別適用した。適用後、リポジトリ全体を対象に`cargo fmt --check`を実行したところ
**hunks=0, files=0**(開始時基準を維持、新しいdriftなし)。

## 15. verification_logs差分

新規追加(削除・上書きなし):
- `verification_logs/phase-21/p21-012/p21-012_completion_report.md`(本報告書)
- `verification_logs/phase-21/p21012_ai_crisis_response_verify/`
  (`summary.txt`/`results.csv`、性能測定の生データ)

開始時点で存在していた未追跡ディレクトリ(P21-010-PERF-VERIFY・P21-011の成果物一式)は
一切削除・変更していない。

## 16. 実GUI確認の実施有無

**実施していない。** 本タスクの検証はすべて自動化テスト(Bevy `App`を用いた軽量/実7か国
28州マップE2Eテスト)によるものであり、実際のウィンドウでの操作確認は行っていない。
GUI手動確認項目(7項目)はいずれも未実施。

## 17. 発見した既存不具合または仕様上の曖昧さ

- **不具合ではない**が、実データ上でCountryId(1)/(2)間に既存の`NonAggressionPact`が
  設定されており、Crisis拒否から発生したJustificationがあっても`declare_war`自身の
  既存条約チェックにより宣戦がブロックされる組み合わせが存在することを確認した(§6参照)。
  これはP21-011までの既存仕様どおりの正しい挙動であり、修正の必要はない。
- `calculate_demand_acceptance`の`target_allies_power`/`initiator_allies_power`引数を
  実際に計算するヘルパーがP21-011までのコードベースに存在しなかった点は、仕様上明示
  されていなかった実装詳細のため§1で判断根拠を明記した。曖昧さとして`NEEDS USER
  DECISION`にするほどではないと判断したが、もし意図と異なる場合は指摘してほしい。

## 18. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

自動テスト・品質ゲートはすべて成功しているが、実GUI操作確認(§16参照)が未実施のため、
「完全に検証済み」とは言えない。ロジック面・決定論・性能面・save整合性については十分な
自動検証を行った。
