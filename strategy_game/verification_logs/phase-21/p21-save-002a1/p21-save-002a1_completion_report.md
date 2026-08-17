# P21-SAVE-002A1: AI派生状態のセーブ形式からの除外 完了報告

**実施日**: 2026-08-13
**性質**: `SaveGameV1`/補助DTOへの小規模なスキーマ修正のみ。ResourceからDTOへの変換、
DTOからResourceへの復元、ファイルI/O、ロード、UIはP21-SAVE-002Aに引き続き実装していない。
**既存のP21-SAVE-002A報告書(`verification_logs/phase-21/p21-save-002a/`)は上書きせず、
本ファイルを新規作成した。**

---

## 1. 最終判定

**COMPLETE**

---

## 2. 実コード上の全dirtyフィールド

`grep -n "\.dirty\b|dirty:|dirty ="`で`src/`配下を全文検索し、以下4箇所を確認した
(いずれも本タスク開始前の既存コード。今回一切変更していない):

| フィールド | 定義場所 | 読み取り箇所 | 書き込み箇所 |
|---|---|---|---|
| `CountryAiRegistry.dirty: bool`(レジストリ直下) | `country/country_ai.rs:149` | **なし**(全文検索で1件も発見できず) | `mark_all_dirty`(163行目)/`mark_country_dirty`(170行目) |
| `CountryAiState.dirty: bool`(国家ごと) | `country/country_ai.rs:127` | `process_daily_country_ai`の`is_weekly_due`(699行目)/`is_monthly_due`(735行目)判定 | `CountryAiState::new`(初期値`true`、140行目)、`mark_all_dirty`/`mark_country_dirty`(161/168行目)、`process_daily_country_ai`末尾で毎回`false`へリセット(748行目) |
| `MilitaryAiRegistry.dirty: bool`(レジストリ直下) | `war/military_ai.rs:83` | **なし**(全文検索で1件も発見できず) | `mark_all_dirty`(99行目)/`mark_country_dirty`(107行目) |
| `MilitaryAiState.dirty: bool`(国家ごと) | `war/military_ai.rs:63` | `process_daily_military_ai`の`is_evaluation_due`判定(565行目) | `MilitaryAiState::new`(初期値`true`、74行目)、`mark_all_dirty`/`mark_country_dirty`(97/105行目)、`process_daily_military_ai`末尾で`skipped_due_to_start_date`の値を書き込み(642行目) |

---

## 3. 各dirtyフィールドの意味分類

指示の2択に対する判定:

| フィールド | 分類 | 根拠 |
|---|---|---|
| `CountryAiRegistry.dirty` | **(1) 派生状態(キャッシュ無効化・再評価要求)** — かつ実質未使用 | 書き込み箇所は存在するが、読み取り箇所が実コード中に一切存在しない。値が何であってもその後のいかなる判断・処理にも影響しないブックキーピング用の残骸である |
| `CountryAiState.dirty` | **(1) 派生状態(キャッシュ無効化・再評価要求)** | `is_weekly_due`/`is_monthly_due`の**OR条件**としてのみ使われ、AIの週次/月次評価タイミングを早めるだけの役割。実際の判断内容(`mode`/`decision_reason`/`cooldown_until_day`等)には一切書き込まれない。評価が終わるたびに`false`へリセットされる典型的なキャッシュ無効化フラグのライフサイクル |
| `MilitaryAiRegistry.dirty` | **(1) 派生状態(キャッシュ無効化・再評価要求)** — かつ実質未使用 | `CountryAiRegistry.dirty`と同一パターン。読み取り箇所なし |
| `MilitaryAiState.dirty` | **(1) 派生状態(キャッシュ無効化・再評価要求)** | `is_evaluation_due`の**OR条件**としてのみ使われ、3日おきの通常評価間隔を早めるだけの役割。実際の判断内容(`last_decision_reason`/`estimated_own_power`/`estimated_enemy_power`)には一切書き込まれない |

**P21-SAVE-001報告の判断(dirtyは派生状態であり、ロード直後は保守的にtrueへ初期化する方針)は、
実コードと完全に一致することを確認した。** ゲーム結果(資金・研究・戦争準備・戦闘等の
実際の判断内容)に影響する正規状態は`mode`/`decision_reason`/各`last_*_evaluation_day`/
`cooldown_until_day`(国家AI)、`last_evaluated_day`/`last_decision_reason`/
`estimated_own_power`/`estimated_enemy_power`(軍事AI)にのみ現れ、`dirty`自体がゲーム結果に
影響するコード経路は存在しない。「単にSerde対応済みであることは保存理由にはならない」という
指示に照らしても、`dirty`を保存対象へ残す実コード上の根拠は見つからなかった。

---

## 4. セーブ対象から除外したフィールド

- `CountryAiRegistry.dirty`(レジストリ直下)
- `CountryAiState.dirty`(要素型内部)
- `MilitaryAiRegistry.dirty`(レジストリ直下)
- `MilitaryAiState.dirty`(要素型内部)

いずれも`SaveGameV1`及び全補助DTOのどのフィールドにも存在しない
(`saved_ai_registries_have_no_derived_dirty_field`/`saved_ai_state_dtos_never_hold_a_dirty_field`
テストで確認、§10参照)。

---

## 5. AIの正規状態として残したフィールド

**`SavedCountryAiState`**(新規DTO、`CountryAiState`から`dirty`を除いた全フィールド):
`country_id`、`mode`(`CountryAiMode`)、`decision_reason`(`CountryAiDecisionReason`)、
`last_daily_evaluation_day`、`last_weekly_evaluation_day`、`last_monthly_evaluation_day`、
`cooldown_until_day`。

**`SavedMilitaryAiState`**(新規DTO、`MilitaryAiState`から`dirty`を除いた全フィールド):
`country_id`、`last_evaluated_day`、`last_decision_reason`(`MilitaryAiDecisionReason`)、
`estimated_own_power`、`estimated_enemy_power`。

---

## 6. 追加・変更したDTO

`src/save/dto.rs`を変更(新規ファイルの追加はなし):

- **新規追加**: `SavedCountryAiState`(`Debug, Clone, PartialEq, Serialize, Deserialize`)
- **新規追加**: `SavedMilitaryAiState`(`Debug, Clone, PartialEq, Serialize, Deserialize`)
- **変更**: `SavedCountryAiRegistry` — `ai_states: HashMap<CountryId, CountryAiState>`(実行時型を
  直接埋め込み)から`ai_states: HashMap<CountryId, SavedCountryAiState>`(新規DTO)へ変更。
  `dirty: bool`フィールドを削除
- **変更**: `SavedMilitaryAiRegistry` — 同様に`ai_states`の要素型を`SavedMilitaryAiState`へ変更、
  `dirty: bool`フィールドを削除

**予期しなかった副次効果**: `SavedCountryAiRegistry`/`SavedMilitaryAiRegistry`は、要素型が
`CountryAiState`/`MilitaryAiState`(いずれも`PartialEq`非対応)から自前定義の
`SavedCountryAiState`/`SavedMilitaryAiState`(`PartialEq`対応)に変わったことで、レジストリ
DTO自体にも`PartialEq`を実装できるようになった(§8で詳述)。

ランタイム型(`country::country_ai::CountryAiState`/`war::military_ai::MilitaryAiState`/
`CountryAiRegistry`/`MilitaryAiRegistry`)自体は一切変更していない。

---

## 7. 最終的な補助DTO数

P21-SAVE-002A報告書の「12個」という記述は誤りで(実際は13個列挙されていた)、
本ラウンドで`SavedCountryAiState`/`SavedMilitaryAiState`を追加したため、
**最終的な補助DTO数は15個**になった:

1. `SavedGameDate`
2. `SavedWorldCivilizationState`
3. `SavedDiplomacyRegistry`
4. `SavedWarJustificationRegistry`
5. `SavedWarRegistry`
6. `SavedClaimRegistry`
7. `SavedCrisisRegistry`
8. `SavedCountryAiRegistry`
9. `SavedMilitaryAiRegistry`
10. `SavedCountryAiState`(新規)
11. `SavedMilitaryAiState`(新規)
12. `SavedMilitaryRegistry`
13. `SavedBattleRegistry`
14. `SavedArmyRegistry`
15. `SavedFrontlineRegistry`

(P21-SAVE-002A報告書本文の「12個」の記述はそのまま残し、本ファイルで訂正する。
指示通り既存報告書は上書きしていない。)

---

## 8. PartialEqを追加しなかったことの確認

`CountryData`/`StateData`/`Division`/`Army`/`War`/`Battle`/`Frontline`/`FrontlinePlan`/
`DiplomaticRelation`/`WarJustification`/`TerritorialClaim`/`DiplomaticCrisis`、及びこれらが
依存する型(`CountryStockpile`/`EconomicState`/`CountryResearchState`/`CountryPoliticsData`/
`PoliticalReform`/`ConstructionQueueItem`/`RecruitmentQueueItem`等)へは、今回も一切
`PartialEq`を追加していない(`git status`で該当ファイルの無変更を確認済み、§9参照)。

セーブテストのためだけにこれらへ`PartialEq`を追加しない、という指示§1の方針を
そのまま維持した。今回追加した`SavedCountryAiState`/`SavedMilitaryAiState`が`PartialEq`を
実装できたのは、これらが上記の非`PartialEq`型を一切参照しない、独立した新規DTO
(`CountryId`+2つの単純enum+`u32`/`u64`のみで構成)だからであり、既存コア型への
`PartialEq`追加とは無関係である。

`SaveGameV1`全体は、`countries`/`states`/`military`/`armies`/`wars`/`battles`/
`frontlines`等が依然として上記の非`PartialEq`型を参照しているため、引き続き`PartialEq`を
実装していない(P21-SAVE-002Aからの既存の制約、今回変更なし)。

今後のセーブ/ロードテストの比較方針(指示§1)を`dto.rs`のコメントへ反映済み:
`PartialEq`対応型は`==`、`HashMap`はキーによる取得後のフィールド比較、コア型は正規データの
フィールド単位比較を用い、`HashMap`の反復順序や RONのバイト列には依存しない。

---

## 9. 変更ファイル一覧

正直に、変更した全ファイルを列挙する:

| ファイル | 種別 | 内容 |
|---|---|---|
| `src/save/dto.rs` | 変更 | `SavedCountryAiState`/`SavedMilitaryAiState`の新規追加、`SavedCountryAiRegistry`/`SavedMilitaryAiRegistry`の`dirty`削除+要素型変更、関連docコメント更新、新規テスト3件追加、テストfixtureの更新(AI正規状態を含む代表データ) |
| `verification_logs/phase-21/p21-save-002a1/p21-save-002a1_completion_report.md` | 新規 | 本報告書 |

上記以外のファイル(ゲームコード・アセット・既存テスト・ランタイムAI型)は一切変更していない。
`src/save/mod.rs`は変更不要だった(`pub use dto::{SAVE_FORMAT_VERSION_V1, SaveGameV1};`の
まま、`SaveGameV1`のフィールド自体は変わっていないため)。

既存の未コミット差分(`assets/localization/{en-US,ja-JP}.ron`、`src/map/{division_selection,
rendering,selection}.rs`、`src/military/{army,mod}.rs`、`src/ui/military_panel.rs`、
`verification_logs/phase-21/p21-004/`、`verification_logs/phase-21/p21-save-001/`、
`verification_logs/phase-21/p21-save-002a/`)はP21-004/P21-004A/P21-SAVE-001/P21-SAVE-002A
由来の既存作業であり、本タスクでは一切手を加えていない(`git status`で確認済み)。

---

## 10. 追加・更新したテスト

すべて`src/save/dto.rs`内の`#[cfg(test)] mod tests`に追加・更新(ファイルI/Oを一切使わない
DTO単体テスト):

**新規追加(3件)**:
1. `round_trip_preserves_ai_normative_state` — AIの正規状態(`SavedCountryAiState`/
   `SavedMilitaryAiState`)がRON往復で維持される(`PartialEq`による値全体の`==`比較)
2. `saved_ai_state_dtos_never_hold_a_dirty_field` — 全フィールドを明示したリテラル構築
   (型構造での確認)+シリアライズ結果に"dirty"が出現しないことの確認(実データでの確認)
3. `saved_ai_registries_have_no_derived_dirty_field` — `SavedCountryAiRegistry`/
   `SavedMilitaryAiRegistry`が`ai_states`のみで構築できること(型構造)+代表的な
   `SaveGameV1`全体をシリアライズしても"dirty"が出現しないこと(実データ)+往復後も
   AI状態のエントリ数が失われないこと、の3点を1テストで確認

いずれも文字列検索だけを唯一の根拠にせず、型構造(フィールドを明示したリテラル構築)と
RON往復データの両方で検証している(指示§6の要求通り)。

**更新(fixtureのみ、既存テストの内容自体は不変)**:
`representative_save()`ヘルパーへ、`CountryId(2)`向けの代表的な`SavedCountryAiState`
(`mode: AtWar`, `decision_reason: WarInProgress`, `cooldown_until_day: 15`等)と
`SavedMilitaryAiState`(`last_decision_reason: SufficientAdvantage`,
`estimated_own_power: 5000`等)を追加した。これにより、既存の12件の往復テスト
(Claim/Crisis/次回IDカウンタ/移動途中Division/Army所属関係/Frontline等)も、
以前の「AI状態が空のデフォルトのまま」というfixtureより実際のゲーム状態に近い
代表データで検証されるようになった(既存テストの検証内容・期待値は変更していない)。

---

## 11. テスト数の変更前後

| 項目 | P21-SAVE-002A完了時点 | 本ラウンド完了後 | 差分 |
|---|---|---|---|
| `cargo test --lib`(単体テスト) | 192 | 195 | +3 |
| `save::dto::tests`のみ | 13 | 16 | +3 |
| 安全な統合テストスイート合計(headless描画2件を除く8バイナリ) | 59 | 59 | ±0 |
| 合計(単体+安全な統合テスト) | 251 | 254 | +3 |

新規追加3件はすべて§10の通り。既存251件の内容・件数は一切変更していない
(全254件が緑、回帰なし)。

---

## 12. 全検証結果

作業ディレクトリ: `strategy_game/`(プロジェクトルート)。

| コマンド | 結果 |
|---|---|
| `cargo fmt --check` | `src/save/dto.rs`に本ラウンドの編集由来の新規diffが7箇所発生したため、rustfmtの提案通りに手動で整形して解消(全て確認・修正済み、§13参照)。修正後は`src/save/dto.rs`・`src/lib.rs`ともdiff0件。それ以外の既知ベースラインFAIL(81件、全て今回未変更の既存ファイル)は残存 |
| `cargo check --all-targets` | 成功(warning 0件) |
| `cargo test --lib save:: -- --list` | 成功、16件検出(13→16、+3) |
| `cargo test --lib save::` | 成功、16 passed; 0 failed |
| `cargo test --lib -- --list` | 成功、195件検出(192→195、+3) |
| `cargo test --lib` + 8統合テストバイナリ | 成功、195+59=254 passed; 0 failed(headless描画2バイナリは既存の運用慣習通り今回も未実行) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 成功、warning 0件 |
| `cargo build --release --all-targets` | 成功 |
| `git diff --check` | 終了コード0、空白関連エラーなし(LF/CRLF警告のみ、既存dirtyファイル由来) |

---

## 13. 既知ベースライン問題(補足)

`cargo fmt --check`のベースランFAIL件数が、P21-SAVE-002A報告書時点の13件から本ラウンドの
確認時点で81件へ拡大していることを発見した。両時点で今回一切触れていない既存ファイル
(`src/military/tests.rs`、`src/profiling.rs`、`src/war/frontline.rs`、
`src/war/military_ai.rs`、`src/war/peace.rs`、`src/war/tests.rs`、
`src/military/{movement,recruitment,supply}.rs`、`src/map/mod.rs`、
`src/ui/peace_panel.rs`、`src/war/capitulation.rs`、`tests/daily_system_integration_test.rs`、
`tests/land_war_combat_peace_test.rs`、`tests/profile_workload_correctness_test.rs`等)を
含んでおり、これらは全て`git status`で無変更(コミット済み状態のまま)であることを確認した
(本タスクで一切手を加えていない)。原因はコードの変化ではなく、環境のrustfmtツールチェーン
バージョン(`rustfmt 1.9.0-stable`)側の何らかの差異と推定されるが、原因の特定は本タスクの
スコープ外のため深追いしていない。**本タスクで実際に変更した`src/save/dto.rs`のみ、
発生した7箇所のdiffを手動で解消済み**(`cargo fmt`コマンド自体は一度も実行していない
— ワークスペース全体を再フォーマットしてしまう既知の落とし穴を避けるため、rustfmtが
提案した差分をEditツールで個別に手動反映した)。

---

## 14. タスクBへの移行可否

**READY**

技術的な障害は見つからなかった。AIのdirty状態除外は、Resource→DTO変換の実装可否には
影響しない(変換はフィールドのコピー/クローンであり、除外対象フィールドを単に変換元から
読まないだけで済む)。タスクB以降でCountryAiRegistry→SavedCountryAiRegistry、
MilitaryAiRegistry→SavedMilitaryAiRegistryへ変換する際は、各要素の`dirty`を読み捨てて
`SavedCountryAiState`/`SavedMilitaryAiState`へ詰め替えるだけでよく、追加の設計判断は不要。

タスクD(ロード実装)では、指示§4の方針通り「保存しなかったdirty状態はすべてtrueで初期化し、
ロード後の最初の評価タイミングで必ず再計算する」実装を予定する。この方針は
`SavedCountryAiState`/`SavedMilitaryAiState`のdocコメントに明記済み(§6参照)。
