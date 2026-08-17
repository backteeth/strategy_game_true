# P21-SAVE-002A: セーブ形式とDTO型の定義 完了報告

**実施日**: 2026-08-13
**性質**: DTO専用モジュール(`src/save/`)の新設のみ。ResourceからDTOへの変換、DTOから
Resourceへの復元、参照整合性検証、ファイルI/O、SaveRequest/LoadRequest、UIボタン、通知、
GameState変更、実際のセーブ/ロードは一切実装していない(次回タスクB以降)。

---

## 1. 最終判定

**COMPLETE**

---

## 2. SaveGameV1の全フィールド

`src/save/dto.rs`で定義。`version`に`#[serde(default)]`は付けていない(§7参照)。
`SaveGameV1`自体へ`Default`は実装していない(§9参照)。

| フィールド | 型 | 区分 |
|---|---|---|
| `version` | `u32` | 形式バージョン(必須) |
| `date` | `SavedGameDate` | 世界と進行 |
| `game_speed` | `u8` | 世界と進行 |
| `player_country` | `Option<CountryId>` | 世界と進行 |
| `world_civilization` | `SavedWorldCivilizationState` | 世界と進行 |
| `countries` | `Vec<CountryData>` | 国家・州 |
| `states` | `Vec<StateData>` | 国家・州 |
| `diplomacy` | `SavedDiplomacyRegistry` | 外交・戦争 |
| `war_justifications` | `SavedWarJustificationRegistry` | 外交・戦争 |
| `wars` | `SavedWarRegistry` | 外交・戦争 |
| `claims` | `SavedClaimRegistry` | 外交・戦争 |
| `crises` | `SavedCrisisRegistry` | 外交・戦争 |
| `country_ai` | `SavedCountryAiRegistry` | 外交・戦争 |
| `military_ai` | `SavedMilitaryAiRegistry` | 外交・戦争 |
| `military` | `SavedMilitaryRegistry` | Division・Army・Battle |
| `battles` | `SavedBattleRegistry` | Division・Army・Battle |
| `armies` | `SavedArmyRegistry` | Division・Army・Battle |
| `frontlines` | `SavedFrontlineRegistry` | 前線 |

---

## 3. 補助DTO一覧

すべて`src/save/dto.rs`に定義。

| DTO | フィールド | 対応するランタイムRegistry |
|---|---|---|
| `SavedGameDate` | `year: i32, month: u8, day: u8, accumulator: f64` | `app::time::GameDate` |
| `SavedWorldCivilizationState` | `current_stage: WorldStage, milestone_countries: HashMap<WorldStage, HashSet<usize>>, last_advanced_date: String` | `research::world_stage::WorldCivilizationState`(可変部分のみ) |
| `SavedDiplomacyRegistry` | `relations: HashMap<DiplomaticPairKey, DiplomaticRelation>` | `diplomacy::data::DiplomacyRegistry` |
| `SavedWarJustificationRegistry` | `justifications: HashMap<usize, WarJustification>, next_id: usize` | `war::justification::WarJustificationRegistry` |
| `SavedWarRegistry` | `wars: HashMap<WarId, War>, next_id: usize` | `war::data::WarRegistry` |
| `SavedClaimRegistry` | `claims: HashMap<ClaimId, TerritorialClaim>, next_id: usize` | `diplomacy::claims::ClaimRegistry` |
| `SavedCrisisRegistry` | `crises: HashMap<DiplomaticCrisisId, DiplomaticCrisis>, next_id: usize` | `diplomacy::crisis::CrisisRegistry` |
| `SavedCountryAiRegistry` | `ai_states: HashMap<CountryId, CountryAiState>, dirty: bool` | `country::country_ai::CountryAiRegistry` |
| `SavedMilitaryAiRegistry` | `ai_states: HashMap<CountryId, MilitaryAiState>, dirty: bool` | `war::military_ai::MilitaryAiRegistry` |
| `SavedMilitaryRegistry` | `divisions: HashMap<DivisionId, Division>, next_division_id: usize` | `military::data::MilitaryRegistry`(可変部分のみ) |
| `SavedBattleRegistry` | `battles: HashMap<BattleId, Battle>, next_id: usize` | `military::battle::BattleRegistry` |
| `SavedArmyRegistry` | `armies: HashMap<ArmyId, Army>, division_army_map: HashMap<DivisionId, ArmyId>, next_id: usize, next_army_number: HashMap<CountryId, u32>` | `military::army::ArmyRegistry` |
| `SavedFrontlineRegistry` | `frontlines: HashMap<FrontlineId, Frontline>, plans: HashMap<(FrontlineId, CountryId), FrontlinePlan>, next_frontline_id: usize, division_frontline_map: HashMap<DivisionId, FrontlineId>, frontline_generated_movements: HashSet<DivisionId>` | `war::frontline::FrontlineRegistry`(全5フィールド) |

---

## 4. ランタイム型を再利用した箇所

既にSerde対応済みの「純粋なデータ要素」型を、そのままフィールド型として再利用した
(新しい型を作らず既存の型をimportして使用):

`CountryData`、`StateData`、`Division`、`Army`、`War`、`Battle`、`Frontline`、
`FrontlinePlan`、`DiplomaticRelation`、`DiplomaticPairKey`、`WarJustification`、
`TerritorialClaim`、`DiplomaticCrisis`、`CountryAiState`(`country::country_ai`)、
`MilitaryAiState`(`war::military_ai`)、`WorldStage`(`research::data`)、および全11種の
永続ID型(`CountryId`/`StateId`/`DivisionId`/`ArmyId`/`WarId`/`BattleId`/`ClaimId`/
`DiplomaticCrisisId`/`FrontlineId`他)。

いずれも既存ファイルへの変更は一切行っていない(importして使うのみ)。

---

## 5. ランタイム型と分離した箇所

以下はランタイムの`Resource`(Registry)をそのまま埋め込まず、`src/save/dto.rs`内に
独立したSaved DTOとして新規定義した:

- `SavedDiplomacyRegistry`/`SavedWarJustificationRegistry`/`SavedWarRegistry`/
  `SavedClaimRegistry`/`SavedCrisisRegistry`/`SavedCountryAiRegistry`/
  `SavedMilitaryAiRegistry`/`SavedMilitaryRegistry`/`SavedBattleRegistry`/
  `SavedArmyRegistry`/`SavedFrontlineRegistry` — いずれもランタイムのRegistry型
  (Bevy `Resource`)をSaveGameV1へ直接埋め込まず、独立したSaved型を介する設計にした
  (§3節「ランタイムのBevy World/Resource全体を直接保存する設計は禁止」に対応)。
  `WarJustificationRegistry`/`FrontlineRegistry`/`MilitaryAiRegistry`/`ArmyRegistry`/
  `CountryAiRegistry`は実際には既にResourceごと`Serialize`/`Deserialize`対応済みで
  技術的には直接埋め込み可能だったが、指示の例示リスト(`SavedArmyRegistry`/
  `SavedFrontlineRegistry`等)に従い、全Registryコンテナを一貫して`Saved*`型経由にした
  (SaveGameV1のフィールドが常に`Saved*`または純粋要素型のみになり、ランタイムの
  `Resource`型を直接参照する箇所がゼロになる)。
- `SavedGameDate` — `app::time::GameDate`の`accumulator`はランタイム側でprivateフィールド
  だが、DTOとして独立定義したためpubフィールドとして表現できた。
- `SavedWorldCivilizationState` — `research::world_stage::WorldCivilizationState`から
  静的マスターデータの`stage_definitions`を除いた可変部分のみを抽出。
- `SavedMilitaryRegistry` — `military::data::MilitaryRegistry`から静的マスターデータの
  `definitions`(師団定義)を除いた可変部分(`divisions`/`next_division_id`)のみを抽出。

これらの新規DTOは、ランタイムのprivateフィールドへは一切アクセスしていない
(全フィールドをテスト内で直接構築するだけで、`GameDate`/`MilitaryRegistry`等の実インスタンス
から値を取り出すコードはまだ存在しない)。

---

## 6. 保存対象外データ

指示§4のリストに挙げられた全項目を、`SaveGameV1`および全補助DTOのいずれのフィールドにも
含めていない: `bevy::prelude::Entity`及びそれを含むコレクション、`SelectedDivision`、
`SelectedArmy`、`SelectedState`、`MilitaryPanelState`、`DiplomacyPanelState`、
`PeacePanelState`、`PoliticsPanelState`、`ResearchPanelState`、`ActivePanel`、
`CameraDragState`、カメラ`Transform`、`GameCamera` Entity、`NotificationHistory`、
`PendingAiWarDeclarations`、`StateVisual`、`DivisionVisual`、`DivisionVisualCluster`、
UI Entity、州の表示色、`StateRegistry.index_map`、`MilitaryRegistry.definitions`、
`BuildingRegistry`、`TechnologyRegistry`、`WorldCivilizationState.stage_definitions`、
`GamePaused`。

`src/save/dto.rs`は`bevy`クレートを一度もimportしていない(ファイル冒頭のuse文一覧を参照)。

---

## 7. version必須化の方法

`SaveGameV1::version: u32`へ`#[serde(default)]`を付けていない。serdeのderive
Deserializeは、`#[serde(default)]`が付いていない必須フィールドがRON上に存在しない場合、
自動的に`missing field 'version'`エラーを返す(手動のバリデーションコードは不要)。

`save::dto::tests::deserialize_fails_when_version_field_is_missing`で実証済み:
代表的な`SaveGameV1`をRONへシリアライズした文字列から`version`エントリのみを文字列置換で
除去し、`ron::from_str::<SaveGameV1>(...)`が`Err`を返すことを確認している。

---

## 8. Claim/Crisisを含めた方法

`SavedClaimRegistry`/`SavedCrisisRegistry`を`SaveGameV1.claims`/`SaveGameV1.crises`
フィールドとして必須(非`Option`)で含めた。実プレイでは`ClaimRegistry::add_claim`/
`CrisisRegistry::add_crisis`は本番コード経路から一度も呼ばれず常に空
(P21-SAVE-001調査で確認済み)だが、可変な世界状態として省略せず、各々の`next_id`
(次回ID発行値)も含めて表現している。`round_trip_preserves_claim_registry`/
`round_trip_preserves_crisis_registry`テストで、要素の内容(`strength`/`source`/
`current_phase`/`escalation`等)と`next_id`の両方が往復で維持されることを確認済み。

---

## 9. GamePausedを含めていないことの確認

`SaveGameV1`のいずれのフィールドにも`GamePaused`/`paused`/`paused_at_save`に相当する
ものは存在しない(§2の全フィールド一覧を参照。`bool`型の一時停止状態フィールドは
`SavedCountryAiRegistry.dirty`/`SavedMilitaryAiRegistry.dirty`のみであり、これらは
AI再評価要否フラグであって一時停止状態ではない)。指示通り、保存時の一時停止状態は
DTOへ含めていない。`save_game_v1_excludes_paused_camera_and_ui_selection_state`テストで、
全フィールドを埋めた値が構築できること(=構造体定義に存在しないフィールドを設定しようと
すればコンパイルエラーになる、という構造的な保証)を確認している。

---

## 10. Entity依存がないことの確認

`src/save/dto.rs`は`use bevy::...`を一度も含まない(ファイル冒頭のuse文一覧を参照。
`serde`と`std::collections`、および本プロジェクトの純粋データ型のみをimportしている)。
`save_game_v1_never_references_bevy_entity`テストで、代表値の構築とRONシリアライズが
Entity型を一切経由せずに成立することを確認している。

---

## 11. 変更ファイル一覧

正直に、新規作成・変更した全ファイルを列挙する(「報告書のみ変更」ではない):

| ファイル | 種別 | 内容 |
|---|---|---|
| `src/save/mod.rs` | 新規 | `save`モジュールの宣言、`dto`サブモジュールの公開 |
| `src/save/dto.rs` | 新規 | `SaveGameV1`及び全補助DTOの定義+DTO単体RON往復テスト(13件) |
| `src/lib.rs` | 変更(+1行) | `pub mod save;`をアルファベット順の位置に追加 |
| `verification_logs/phase-21/p21-save-002a/p21-save-002a_completion_report.md` | 新規 | 本報告書 |

上記以外のファイル(ゲームコード・アセット・既存テスト)は一切変更していない。
既存の未コミット差分(`assets/localization/{en-US,ja-JP}.ron`、`src/map/{division_selection,
rendering,selection}.rs`、`src/military/{army,mod}.rs`、`src/ui/military_panel.rs`、
`verification_logs/phase-21/p21-004/p21-004_implementation_report.md`、
`verification_logs/phase-21/p21-save-001/`)はP21-004/P21-004A/P21-SAVE-001由来の
既存作業であり、本タスクでは一切手を加えていない(`git status`で確認済み、§14参照)。

---

## 12. 追加テスト一覧

すべて`src/save/dto.rs`内の`#[cfg(test)] mod tests`に追加(ファイルI/Oを一切使わない
DTO単体テスト、13件):

1. `representative_save_serializes_to_ron` — 代表的なSaveGameV1がRONへSerializeできる
2. `round_trip_preserves_world_and_progress_fields` — version/date/game_speed/
   player_country/world_civilizationが往復で維持される
3. `round_trip_preserves_country_and_state_representative_fields` — 国家・州の代表フィールドが往復で維持される
4. `serialized_ron_contains_version_field` — RONへversionフィールドが含まれる
5. `deserialize_fails_when_version_field_is_missing` — versionフィールドを削除したRONはDeserializeに失敗する
6. `round_trip_preserves_claim_registry` — Claim DTOが往復で失われない
7. `round_trip_preserves_crisis_registry` — Crisis DTOが往復で失われない
8. `round_trip_preserves_all_next_id_counters` — DivisionId/ArmyId/WarId/BattleId/ClaimId/DiplomaticCrisisId/FrontlineIdの次回発行値が往復で維持される
9. `round_trip_preserves_division_in_progress_movement_fields` — 移動途中Divisionの全移動フィールド(status/destination/current_path/target_state/movement_progress)がRON往復で維持される
10. `round_trip_preserves_army_membership` — Army所属関係(member_division_ids/division_army_map/next_army_number)がRON往復で維持される
11. `round_trip_preserves_frontline_registry_including_tuple_keyed_plans` — タプルキー`HashMap<(FrontlineId, CountryId), FrontlinePlan>`を含むFrontlineRegistryの正規データが往復で維持される
12. `save_game_v1_never_references_bevy_entity` — SaveGameV1/補助DTOがEntityを要求しない
13. `save_game_v1_excludes_paused_camera_and_ui_selection_state` — paused・カメラ・UI選択状態をDTOへ含めていない

比較はすべて`PartialEq`による意味的比較(個別フィールド値・`HashMap::get`によるキー引き)
であり、バイト列比較やHashMap順序に依存する比較は一切使用していない(§16「発見した
設計上の問題」で後述する`SaveGameV1`自体への`PartialEq`非実装との関係も参照)。

---

## 13. テスト数の変更前後

| 項目 | 変更前 | 変更後 | 差分 |
|---|---|---|---|
| `cargo test --lib`(単体テスト) | 179 | 192 | +13 |
| 安全な統合テストスイート合計(headless描画2件を除く8バイナリ) | 59 | 59 | ±0 |
| 合計(単体+安全な統合テスト) | 238 | 251 | +13 |

新規追加13件はすべて`save::dto::tests`配下(§12参照)。既存テストの内容・件数は
一切変更していない。

---

## 14. 全検証コマンドと終了コード

作業ディレクトリ: `strategy_game/`(プロジェクトルート)。

| コマンド | 結果 |
|---|---|
| `cargo fmt --check` | 既知ベースラインFAILのみ(§15参照)。新規ファイル・変更ファイルに新たな整形問題なし |
| `cargo check --all-targets` | 成功(warning 0件) |
| `cargo test --lib save:: -- --list` | 成功、13件検出 |
| `cargo test --lib save::` | 成功、13 passed; 0 failed |
| `cargo test --lib -- --list` | 成功、192件検出(179→192、+13) |
| `cargo test --lib` + 8統合テストバイナリ(daily_system_integration/diplomacy/economy/land_war_combat_peace/p20_009_hardcoded_string_scan/p20_009_localization_resource/profile_workload_correctness/research_and_politics) | 成功、192+59=251 passed; 0 failed(headless描画2バイナリ[`ui_headless_render_test`/`p20_009_localization_headless_render_test`]は固定PNG上書きを避けるため、既存の運用慣習通り今回も未実行) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 成功、warning 0件 |
| `cargo build --release --all-targets` | 成功 |
| `git diff --check` | 終了コード0、空白関連エラーなし(LF/CRLF警告のみ、既存dirtyファイル由来、新規ファイルには出現せず) |

---

## 15. 既知ベースライン問題

`cargo fmt --check`は以下13箇所のDiffを報告するが、いずれも本タスクで一切触れていない
既存ファイルであり(`git status`で無変更または既存dirtyのまま、§11参照)、今回の変更
(`src/save/mod.rs`/`src/save/dto.rs`/`src/lib.rs`)には一切fmt diffが出ていないことを
確認済み:

- `src/app/loader.rs`(1箇所、import順) — HEAD時点でコミット済みの既存フォーマット債務、`git status`上も無変更
- `src/country/country_ai.rs`(3箇所) — 同上、無変更
- `src/map/division_render.rs`(5箇所) — 同上、無変更
- `src/map/division_selection.rs`(4箇所) — P21-004A由来の既存dirty変更(本タスクでは触れていない)

---

## 16. 発見した設計上の問題

**`SaveGameV1`全体への`#[derive(PartialEq)]`が、既存の生成済みコード変更なしには
機械的に適用できない。** 調査の結果、`SaveGameV1`が(直接・間接に)参照する以下の
既存の「純粋データ要素」型が、いずれも構造体レベルの`PartialEq`を実装していないことが
判明した: `CountryData`、`StateData`、`Division`、`Army`、`War`、`Battle`、`Frontline`、
`FrontlinePlan`、`DiplomaticRelation`、`WarJustification`、`TerritorialClaim`、
`DiplomaticCrisis`、`CountryAiState`、`MilitaryAiState`。

さらに`CountryData`は`CountryStockpile`/`EconomicState`/`CountryResearchState`/
`CountryPoliticsData`/`PoliticalReform`/`ConstructionQueueItem`/`RecruitmentQueueItem`
(いずれも`economy`/`research`/`politics`/`building`/`military`の各モジュールに分散)を
経由してさらに`PartialEq`非対応の型へ連鎖しており、`CountryData`単体へ`PartialEq`を
追加するだけでも本タスクのスコープ外の7ファイル以上への変更が必要になる。

本タスクは「DTO専用モジュール(`src/save/`)の新設」に変更範囲を限定する方針
(指示§1「原則として次を新設してください」、§7「既存設計の保護」)のため、これらの
既存ファイルへ`PartialEq`を追加することは行わなかった。代わりに:
- `SaveGameV1`及び、上記の非`PartialEq`型を(直接・間接に)含む全補助DTO
  (`SavedDiplomacyRegistry`/`SavedWarJustificationRegistry`/`SavedWarRegistry`/
  `SavedClaimRegistry`/`SavedCrisisRegistry`/`SavedCountryAiRegistry`/
  `SavedMilitaryAiRegistry`/`SavedMilitaryRegistry`/`SavedBattleRegistry`/
  `SavedArmyRegistry`/`SavedFrontlineRegistry`)は`PartialEq`を実装していない
- `PartialEq`が実際に実装できたのは、非`PartialEq`型を含まない`SavedGameDate`と
  `SavedWorldCivilizationState`のみ(§2/§3参照)
- 往復テストでの「意味的に同一」の検証は、`PartialEq`が使える箇所は`==`、それ以外は
  `HashMap::get`によるキー引き+個別フィールド比較(いずれも順序に依存しない)で行った
  (§12参照。バイト列比較・HashMap順序依存比較は一切不使用)

この設計上の制約自体は今回の新規DTOコードの欠陥ではなく、既存コードベース全体に
広く存在する既知の未整備状態(そもそも本プロジェクトのコア構造体のほとんどが
`PartialEq`を実装していない)である。将来、`SaveGameV1`全体への`==`比較や、
より簡潔な往復テストを可能にしたい場合は、上記の型群への`PartialEq`追加を
別タスクとして切り出すことを推奨する(NEEDS USER DECISION、下記参照)。

**NEEDS USER DECISION**: 上記の型群(CountryData/StateData/Division/Army/War/Battle/
Frontline/FrontlinePlan等、およびその依存先)へ`PartialEq`を追加する別タスクを
起票するか。追加しない場合、将来のセーブ/ロード関連テストも今回同様の
フィールド単位比較で書き続けることになる(技術的には問題ないが、テストコードが
やや冗長になる)。

---

## 17. タスクB「Resource→DTO変換・セーブ」への移行可否

**READY**

技術的な障害は見つからなかった。`SaveGameV1`及び全補助DTOの型定義はRON往復
(タプルキーHashMapを含む)で実証済みであり、既存のPlaying状態Resource群
(`CountryRegistry`/`StateRegistry`/`DiplomacyRegistry`/`WarJustificationRegistry`/
`WarRegistry`/`ClaimRegistry`/`CrisisRegistry`/`CountryAiRegistry`/`MilitaryAiRegistry`/
`MilitaryRegistry`/`BattleRegistry`/`ArmyRegistry`/`FrontlineRegistry`/`GameDate`/
`GameSpeed`/`PlayerCountry`/`WorldCivilizationState`)から、今回定義した型へ値を
詰め替える変換コード(`From`実装や専用の`to_save_dto`関数群)を書けば、タスクBへ
そのまま進める状態にある。

§16で述べた`PartialEq`の制約は、Resource→DTO変換の実装可否には影響しない
(変換はフィールドのコピー/クローンであり、比較演算とは無関係)。ただし、タスクB以降で
「変換前後の意味的同一性」を検証するテストを書く際は、本タスクの往復テストと同様の
フィールド単位比較パターンを踏襲する必要がある点をタスクB着手時に留意されたい。
