# P21-SAVE-002C: セーブ読込・バージョン・参照整合性検証 完了報告

**実施日**: 2026-08-13
**性質**: `SaveGameV1`の読込(ファイル読み取り・RON解析・version検証)と、内部の全ID・参照
整合性および静的マスターデータ参照の検証を実装した。検証済みデータを表す`ValidatedSaveGameV1`
型を新設し、未検証の`SaveGameV1`を直接Resourceへ適用する経路は作っていない。
SaveGameV1からResourceへの適用・現在のゲーム状態の変更・AI dirtyの復元・`LoadRequest`・
Bevy System・`LoadGamePlugin`・UIボタン・通知・`main.rs`登録・GameState変更・起動直後ロード・
セーブスロット一覧・自動修復・sanitizeはいずれも実装していない(P21-SAVE-002D以降)。

---

## 1. 最終判定

**COMPLETE**

不正セーブは「修復して通す」のではなく、問題点を構造化して収集し全体を拒否する設計とした。
1件でも検証に失敗すれば`SaveGameV1`全体を返さない(部分適用不可)。

---

## 2. ID・参照フィールドの全棚卸し

実コード(`src/country/`, `src/state/`, `src/diplomacy/`, `src/war/`, `src/military/`,
`src/research/`, `src/building/`, `src/economy/`, `src/politics/`)を再帰的に確認し、
`SaveGameV1`が保持する全型のIDフィールド・相互参照を洗い出した。要点のみ記す
(詳細は`src/save/validate.rs`各関数のdocコメントおよび実装を参照)。

| 参照元 | 参照先ID型 | セーブ内部だけで検証可 | 静的マスター必要 | Nullable | 不変条件 |
|---|---|---|---|---|---|
| `CountryData.capital_state_id` | `StateId` | ○ | - | × | 州が存在し、その州の`owner_country_id`が自国であること(`app/loader.rs::validate_data`と同じ不変条件) |
| `CountryData.construction_queue[].state_id/building_type` | `StateId`/`BuildingType` | 州はセーブ内、建物種別は静的 | ○(BuildingRegistry) | × | `target_level <= max_level` |
| `CountryData.recruitment_queue[].division_id/target_state` | `DivisionDefinitionId`/`StateId` | 州はセーブ内、師団定義は静的 | ○(MilitaryRegistry.definitions) | × | - |
| `CountryData.research_state.completed_technologies/in_progress` | `String`(技術ID) | × | ○(TechnologyRegistry) | - | - |
| `StateData.owner_country_id` | `CountryId` | ○ | - | × | - |
| `StateData.controller_country/original_owner` | `CountryId` | ○ | - | ○ | - |
| `StateData.war_id` | `WarId` | ○ | - | ○ | - |
| `StateData.neighbors` | `StateId` | ○ | - | - | 双方向・自己隣接禁止・重複禁止(既存統合テスト`tests/land_war_combat_peace_test.rs`と同一不変条件を再確認して採用) |
| `StateData.buildings` | `BuildingType` | × | ○(BuildingRegistry) | - | `level <= max_level` |
| `DiplomaticPairKey(a,b)` | `CountryId`×2 | ○ | - | × | `a != b`かつ`a.0 < b.0`(`DiplomaticPairKey::new`の正規化契約) |
| `DiplomaticRelation.cooldowns` | `CountryId` | ○ | - | - | - |
| `DiplomaticRelation.treaties[].countries` | `CountryId`×2 | ○ | - | - | ペアキーの{a,b}と(順不同で)一致 |
| `DiplomaticRelation.active_activity` | `CountryId`×2 | ○ | - | ○ | 同上 |
| `WarJustification.initiator/target/target_state` | `CountryId`/`CountryId`/`StateId` | ○ | - | × | `initiator != target` |
| `War.attackers/defenders` | `CountryId`集合 | ○ | - | × | 空集合禁止(`frontline.rs`が`.iter().next().unwrap()`する実コードあり)、attacker∩defender=∅ |
| `War.occupied_states` | `StateId` | ○ | - | - | - |
| `War.winner` | `CountryId` | ○ | - | ○ | attackers∪defendersに含まれる(`war/peace.rs`の実装通り) |
| `War.war_goals[].attacker/defender/target_states` | `CountryId`/`CountryId`/`StateId` | ○ | - | - | - |
| `War.processed_battle_ids` | `BattleId` | **検証しない** | - | - | `BattleRegistry::cleanup_finished_battles`(`military/update.rs`から日次呼出)が完了戦闘を`battles`から削除する仕様のため、存在しないIDを指すのが正常(下記6項参照) |
| `TerritorialClaim.claimant_country/target_state` | `CountryId`/`StateId` | ○ | - | × | - |
| `DiplomaticCrisis.initiator/target` | `CountryId` | ○ | - | × | `initiator != target` |
| `DiplomaticCrisis.third_party_reactions` | `CountryId` | ○ | - | - | - |
| `SavedCountryAiState.country_id`/`SavedMilitaryAiState.country_id` | `CountryId` | ○ | - | × | Mapキーと一致 |
| `Division.owner` | `CountryId` | ○ | - | × | - |
| `Division.current_state/destination/target_state/current_path[]` | `StateId` | ○ | - | 一部○ | - |
| `Division.def_id` | `DivisionDefinitionId` | × | ○(MilitaryRegistry.definitions) | × | - |
| `Division.combat_id` | `BattleId` | ○ | - | ○ | Someなら対応するOngoing Battleの参加者リストに自分が含まれる(双方向) |
| `Battle.war_id/state_id/attacker_country/defender_country` | 各種 | ○ | - | × | - |
| `Battle.attacker_division_ids/defender_division_ids` | `DivisionId` | Ongoingのみ検証 | - | - | Ongoing戦闘の参加師団は存在し`combat_id`が双方向一致、同一師団が複数Ongoing戦闘に同時参加しない |
| `Army.owner` | `CountryId` | ○ | - | × | - |
| `Army.member_division_ids` | `DivisionId` | ○ | - | × | 空Army禁止、所有者一致、`division_army_map`と双方向一致、同一師団の複数Army所属禁止 |
| `ArmyRegistry.division_army_map` | `DivisionId`→`ArmyId` | ○ | - | - | 双方向一致 |
| `ArmyRegistry.next_army_number` | `CountryId`→採番値 | ○ | - | - | 参照国が存在し、`create_army`の命名規則("Army N")から見て既存最大Nと衝突しない |
| `FrontlineRegistry.frontlines[].war_id/attacker_country_id/defender_country_id` | 各種 | ○ | - | × | attacker/defenderは対応する`War`のattackers/defendersに実際に含まれる |
| `FrontlineRegistry.plans`キー`(FrontlineId,CountryId)` | 複合 | ○ | - | × | Mapキーと要素内部の`(frontline_id, commanding_country_id)`が一致、commanding_country_idは前線の参加国 |
| `FrontlinePlan.assigned_division_ids` | `DivisionId` | ○ | - | × | 所有者一致、`division_frontline_map`と双方向一致、同一師団の複数前線所属禁止 |
| `FrontlineRegistry.division_frontline_map` | `DivisionId`→`FrontlineId` | ○ | - | - | 双方向一致 |
| `FrontlineRegistry.frontline_generated_movements` | `DivisionId` | ○ | - | - | - |
| `WorldCivilizationState.current_stage` | `WorldStage` | × | ○(stage_definitions) | × | - |
| `WorldCivilizationState.milestone_countries` | `WorldStage`→`usize`集合 | 値はセーブ内、キーは静的 | ○(stage_definitions) | - | 値は実在CountryId |

各Registryの`next_id`系(`WarRegistry`/`ClaimRegistry`/`CrisisRegistry`/`MilitaryRegistry`/
`BattleRegistry`/`ArmyRegistry`/`WarJustificationRegistry`/`FrontlineRegistry`)は全て
「次回発行IDが既存の最大IDより大きいこと」を検証する(実際の`add_x`系メソッドが
`id = next_id; next_id += 1`の形で単調増加させるだけで、削除後に採番し直す経路が
存在しないことを実コードで確認した)。

---

## 3. 読込処理の順序

`src/save/read.rs::read_and_validate_save_file`:

1. `config.final_path`を`fs::read_to_string`で読み取る(`io::ErrorKind::NotFound`は
   `FileNotFound`、それ以外のI/Oエラーは`Read`)
2. 軽量ヘッダー型`SaveVersionHeader { version: u32 }`でversionだけを先に確認する
3. `SaveGameV1`として完全にDeserializeする
4. `validate_save_game_v1`で全参照整合性を検証する
5. 全て成功した場合だけ`ValidatedSaveGameV1`を返す

読み取り中にファイルの変更・`.tmp`の削除・stale修復・現在のResourceへの書き込み・
`GamePaused`の変更・UI状態の変更は一切行わない(関数シグネチャが`&SavePathConfig`/
`&SaveValidationContext`という読み取り専用の入力しか取らないことが構造的な保証)。

---

## 4. version検証方法

`SaveVersionHeader { version: u32 }`という最小構造体を、完全な`SaveGameV1`のRONに対して
先にDeserializeする。serdeの派生`Deserialize`実装は`#[serde(deny_unknown_fields)]`を
付けない限り未知フィールドを既定で読み飛ばすため、`version`以外の全フィールドを持つ
本物のセーブに対してもこのヘッダーだけが正しく読み取れることを、
`tests::version_header_parses_from_a_full_save_game_v1_document`で実際に確認した
(RONのvendoredソース`ron-0.12.2/src/de/mod.rs`の`deserialize_struct`が`CommaSeparated`
という汎用`MapAccess`実装へ委譲する構造であることも確認済み)。

- ヘッダーの`version`が`SAVE_FORMAT_VERSION_V1`(=1)と異なる場合 → `UnsupportedVersion { found }`
  (version 0、version 2のいずれも実テストで確認)
- ヘッダー自体が読み取れない場合(version欠落、またはRON構文が壊れている)は、
  ここでは判定せず後続の完全な`Deserialize`へ委ねる。`SaveGameV1::version`には
  `#[serde(default)]`を付けていないため、version欠落は後続のDeserializeでも同じ理由で
  失敗し、最終的に`LoadSaveError::Deserialize`として扱われる(「壊れたV1」の一種として扱い、
  `UnsupportedVersion`とは区別しない、という仕様で確定した)。
- `SaveGameV1`へ`#[serde(default)]`は追加していない。未来バージョンをV1として強制解釈する
  経路は存在しない。移行処理(マイグレーション)は実装していない。

---

## 5. ValidatedSaveGameV1の構造

```rust
pub struct ValidatedSaveGameV1 {
    save: SaveGameV1, // 非公開フィールド
}

impl ValidatedSaveGameV1 {
    pub fn save(&self) -> &SaveGameV1 { &self.save }       // 読み取り専用アクセサ
    pub(crate) fn into_inner(self) -> SaveGameV1 { self.save } // タスクD用、crate外に公開しない
}
```

- フィールドは非公開。任意の`SaveGameV1`から直接構築する経路は存在しない
  (`validate_save_game_v1(save, context) -> Result<ValidatedSaveGameV1, SaveValidationErrors>`
  を通過した場合だけ生成される)。
- `DerefMut`・可変参照・`Default`・不要な`Clone`は実装していない。
- タスクD(検証済みDTOのResource適用)向けに`pub(crate) into_inner`のみ用意した。

---

## 6. SaveValidationContextの内容

```rust
pub struct SaveValidationContext<'a> {
    pub building_definitions: &'a HashMap<BuildingType, BuildingDefinition>,
    pub technology_definitions: &'a HashMap<String, TechnologyDefinition>,
    pub division_definitions: &'a HashMap<DivisionDefinitionId, DivisionDefinition>,
    pub world_stage_definitions: &'a HashMap<WorldStage, WorldStageDefinition>,
}
```

実コード調査の結果、これら4種の静的マスターデータは元々`BuildingRegistry.definitions`/
`TechnologyRegistry.definitions`/`MilitaryRegistry.definitions`/
`WorldCivilizationState.stage_definitions`として**既に`pub`フィールド**だったため、
コアモジュールへ新しいアクセサを追加する必要が一切なかった(単純な共有参照を束ねるだけ)。

- `ResourceDefinition`のような専用の資源マスターレジストリはこのcrateに存在しない
  (`ResourceType`は閉じたenumであり、静的マスター参照チェックの対象にならない)ため、
  候補に挙げられていたが**Contextへ追加しなかった**。
- Country/State/Division/Army等の動的データはContextに含めない。セーブ自身を正とし、
  現在のランタイムResourceとは一切比較しない。
- `Entity`は含まない。privateフィールドの全面的な`pub`化は行っていない。

---

## 7. 全ValidationCode

`SaveValidationCode`(17種)。UIが将来ローカライズできるよう、技術詳細1種類の`String`には
潰さず、問題のカテゴリごとにコードを共有し、具体箇所・詳細は`path`/`detail`で表現する。

| コード | 意味 |
|---|---|
| `DuplicateId` | Vec内で同じ主キーが重複 |
| `MapKeyMismatch` | HashMapキーと要素内部の自己IDが不一致 |
| `MissingValue` | 必須値の欠落(`player_country`がNone等) |
| `DanglingReference` | 存在しない対象への参照(静的マスター参照含む) |
| `InvalidRange` | 既存ロジックの許容範囲外の数値 |
| `NonFiniteValue` | 有限値が前提の箇所にNaN/Infinity |
| `AsymmetricAdjacency` | 州の隣接が双方向でない |
| `SelfAdjacency` | 州が自分自身を隣接に持つ |
| `DuplicateAdjacency` | 同じ隣接州が重複列挙 |
| `NextIdCollision` | 次回発行IDが既存最大IDと衝突し得る |
| `OwnershipMismatch` | 所有者・国籍の不一致 |
| `ParticipantMismatch` | 参加国と命令主体・攻撃側防御側の不一致 |
| `ReverseMapInconsistent` | 双方向マッピングが片方向からしか成立しない |
| `EmptyCollection` | 空であってはならないコレクションが空 |
| `DuplicateMembership` | 同一IDが複数の排他的所属先に同時所属 |
| `SetOverlap` | 互いに素であるべき集合が重なる |

---

## 8. Country/State検証

`validate_countries`/`validate_capital`/`validate_states`(`src/save/validate.rs`)。

- 国家: 重複CountryId、`capital_state_id`の存在確認、首都州の`owner_country_id`が自国と
  一致(`app/loader.rs::validate_data`と同一不変条件)、`treasury`の有限性、
  `construction_queue`(州参照・建物種別・`target_level <= max_level`)、`recruitment_queue`
  (師団定義参照・州参照)、`research_state.completed_technologies`/`in_progress`の技術ID参照。
- 州: 重複StateId、`owner_country_id`/`controller_country`/`original_owner`/`war_id`の
  存在確認、`buildings`(建物種別・レベル上限)、`neighbors`の存在確認・自己隣接禁止・
  重複禁止・双方向性、`living_standard`/`unrest`/`logistics_ratio`/`occupation_progress`
  の有限性。

---

## 9. 外交・戦争検証

`validate_diplomacy`/`validate_war_justifications`/`validate_wars`/`validate_claims_and_crises`。

- `DiplomaticPairKey`の正規化(`a != b`かつ`a.0 < b.0`)・両国の存在、`cooldowns`の参照、
  `treaties[].countries`と`active_activity`の参加国一致(ペアキーの{a,b}と一致すること)、
  `opinion`/`tension`/`trust`/`threat`の有限性。
- `WarJustification`: `initiator != target`、initiator/target/target_stateの存在
  (実装漏れをテストで検出し、`validate_war_justifications`として追加実装した。詳細は
  §22参照)。
- `War`: attackers/defendersの非空(`frontline.rs`の`.unwrap()`前提を根拠に採用)、
  互いに素であること、参加国の存在、`occupied_states`の存在、`winner`が参加者であること
  (`war/peace.rs`の実装通り)、`war_goals`内の参照、`war_score`等の有限性。
  **`processed_battle_ids`は意図的に検証しない**(§2参照、実コードの仕様に基づく判断)。
- `TerritorialClaim`/`DiplomaticCrisis`: 参照国・州の存在、`initiator != target`、
  `third_party_reactions`・`war_goals`内参照、関連数値の有限性。

---

## 10. Division/Battle検証

`validate_divisions`/`validate_battles`。

- Division: owner/current_state/destination/target_state/current_pathの存在確認、
  `def_id`の静的マスター参照、`combat_id`(Someなら存在確認)、equipment/organization/
  morale/experience/supply_ratio/movement_progressの有限性。移動途中(Moving+destination
  +current_path)や戦闘途中(Fighting+combat_id)を不当に拒否しないことをテストで確認済み。
- Battle: war_id/state_id/attacker_country/defender_countryの存在確認。
  **Ongoingの戦闘のみ**、参加師団の存在・`Division.combat_id`との双方向一致・同一師団の
  複数Ongoing戦闘への重複参加禁止を検証する。終了・中止済み(Ongoingでない)戦闘の参加師団は
  既に消滅済みでも正常(過去の記録)として扱う。

---

## 11. Army検証

`validate_armies`。owner存在、空Army禁止、`member_division_ids`各要素の存在確認・
所有者一致、`division_army_map`との双方向整合(片方向だけの参照・存在しないDivision/Army
への参照を検出)、同一師団の複数Army重複所属禁止、`next_army_number`の参照国存在確認、
および`create_army`の命名規則("Army N")から実際に使われている最大Nを解析し、
`next_army_number`がそれを超えていること(将来の命名衝突を防止)を検証する。

---

## 12. AI検証

`validate_ai`。`CountryAiRegistry`/`MilitaryAiRegistry`のMapキーCountryIdが実在すること、
および§7の`MapKeyMismatch`によりMapキーと要素内部の`country_id`の一致を検証する
(`validate_registry_keys_and_next_ids`内で実施)。`dirty`は元々`SavedCountryAiState`/
`SavedMilitaryAiState`に存在しないため(P21-SAVE-002A1)、保存形式レベルで構造的に
検証不要である。AI状態が存在しない国家(未評価)を許容するか・全国家必須かは、
実コード(`get_or_create_mut`による遅延生成)が「初回評価まで存在しなくて正常」である
ことを示しているため、存在しない国家分を必須としない(未検証エラーにしない)方針で確定した。

---

## 13. Frontline検証

`validate_frontlines`。`FrontlineRegistry`の全5フィールドを検証する:
`frontlines`(war_id存在・attacker/defenderの存在と重複禁止・対応する`War`の
attackers/defendersに実際に含まれること・国境地域/ペアの州参照)、
`plans`(タプルキーと要素内部の一致・commanding_country_idが前線参加国であること・
assigned_division_idsの存在確認と所有者一致・同一師団の複数前線重複所属禁止)、
`division_frontline_map`(双方向整合)、`frontline_generated_movements`(存在確認)、
`next_frontline_id`(次回ID衝突)。`FrontlinePlan`が個別Divisionを保持する既存仕様は
変更していない(ArmyId参照へは置き換えていない)。

---

## 14. 静的マスター参照検証

§6の`SaveValidationContext`経由で、`BuildingType`(construction_queue・buildings)、
技術ID文字列(completed_technologies・in_progress)、`DivisionDefinitionId`
(recruitment_queue・Division.def_id)、`WorldStage`(current_stage・milestone_countries
のキー)を検証する。いずれも「セーブ内の動的データを比較する」のではなく、
「起動時にRONから構築される不変のマスターデータに実在するか」だけを確認する。

---

## 15. next_id検証

`check_id_registry`共通ヘルパー(`WarRegistry`/`ClaimRegistry`/`CrisisRegistry`/
`MilitaryRegistry`/`BattleRegistry`/`ArmyRegistry`/`FrontlineRegistry`で共有)と、
`WarJustificationRegistry`専用のインライン処理で、「`next_id`は既存の最大要素IDより
大きくなければならない」ことを検証する。空Registryの場合は最大IDが存在しないため
この検証は常に成立し(`next_id`が0のままでもエラーにならない)、`Default`が生成する
初期状態(空・`next_id=0`)を正しく受理することをテストで確認した。

---

## 16. HashMap重複キーの制限

`tests::duplicate_hashmap_keys_in_ron_are_silently_overwritten_by_the_last_value`で
実際にRON(`"{0: 1, 0: 2}"`)を`ron::from_str`し、挙動を直接確認した:

- 結果は`{0: 2}`(要素数1)。**後勝ちで上書きされ、前の値は消える**。
- Deserialize時点で既に1件へ収束しているため、`validate.rs`が「重複キーがあった」ことを
  検出する余地は構造的に存在しない。「重複キーを検出済み」とは報告しない
  (要求仕様通り、この制限のためだけにV1のMapをVecへ変更するスキーマ変更は行っていない)。
- 実務上の影響: 悪意・破損したセーブファイルで意図的にMapキーを重複させても、RONパーサの
  時点で片方が消えるため、以降の参照整合性検証はその「生き残った」1件だけを見ることになる
  (整合性が壊れるわけではないが、セーブされていたはずのデータが黙って失われる可能性がある、
  という制限として正直に報告する)。

---

## 17. 読み取り専用性の保証

`read_and_validate_save_file(config: &SavePathConfig, context: &SaveValidationContext)`
は`ResMut`/`Commands`/`World`への可変参照/`GamePaused`への可変参照/各動的Registryへの
可変参照のいずれも引数に取らない(シグネチャ自体が構造的な保証)。
`tests::reading_does_not_modify_file_contents_or_mtime`で、読込前後のファイル内容
(バイト列)とmtimeが完全に一致することを実際に確認し、`.tmp`にも一切触れていないことを
確認した。`tests::reading_an_invalid_reference_save_reports_validation_error_without_touching_state`
で、参照不整合セーブを読んだ場合もファイル内容が変化しないことを確認した。
このラウンドではBevy App統合を実装していないため、可変ゲーム状態を受け取れない
純粋な関数シグネチャであることを主な構造的保証とした。

---

## 18. 変更ファイル一覧

**新規作成**:
- `src/save/read.rs`(ファイル読み取り・RON解析・version検証・構造化エラー、11テスト)
- `src/save/validate.rs`(参照整合性検証・`ValidatedSaveGameV1`、74テスト)
- `verification_logs/phase-21/p21-save-002c/p21-save-002c_completion_report.md`(本ファイル)

**変更**:
- `src/save/mod.rs`(`read`/`validate`モジュールの登録・re-export追加のみ)
- `src/app/time.rs`(`GameDate::days_in_month(month: u8) -> Option<u8>`という
  `pub(crate)`の読み取り専用ヘルパーを追加。P21-SAVE-002Bの`accumulator()`アクセサと
  同じパターン。`DAYS_IN_MONTH`自体・既存の日付進行ロジック・`DailySimulationSet`順序・
  `GameState`は一切変更していない)

**変更していない**(要求仕様通り):
`src/save/dto.rs`・`src/save/export.rs`・`src/save/write.rs`・`src/save/runtime.rs`、
`main.rs`、`src/app/loader.rs`、静的RONアセット、UI関連ファイル、P21-005関連ファイル。

---

## 19. 追加テスト一覧

**`src/save/read.rs`(11件)**: 正常RON読込・ValidatedSaveGameV1取得・FileNotFound・
Read(ディレクトリを代わりに置く失敗注入)・Deserialize(不正RON)・version欠落→Deserialize・
version 0→UnsupportedVersion・version 2→UnsupportedVersion・ヘッダー単体の実挙動確認・
読込前後のファイル内容/mtime不変・参照不整合セーブでのValidation拒否とファイル不変。

**`src/save/validate.rs`(74件、カテゴリ別)**:
- ベースライン/受理系(4件): 正常セーブ受理、空Claim/Crisis受理、空Registry初期next_id受理、
  移動途中Division受理、進行中Battle受理。
- 基本状態(21件): 重複CountryId/StateId、player_country欠落/不明、capital不明/所有者不一致、
  state owner/controller/original_owner不明、隣接不明/非対称/自己隣接/重複、GameDate
  month/day/accumulator(NaN含む)不正、GameSpeed 0/5、WorldStage不明、milestone不明国、
  建物種別不明・レベル超過、募兵定義不明、技術ID不明、師団定義不明。
- 外交・戦争(13件): pair国不明、非正規化キー、treaty/activity参加国不一致、war参加国不明、
  attackers/defenders空、重複、winner非参加者、processed_battle_ids許容(受理系)、
  war_justification参加者不明(実装漏れをここで検出、§22参照)、claim/crisis参照不明。
- Registry(2件): MapKeyMismatch、NextIdCollision(汎用代表ケース)。
- Division/Battle(7件): owner/current_state/経路/combat_id不明、combat_id相互不整合、
  battle参加者相互不整合、重複参戦。
- Army(8件): owner不明、空Army、他国師団混入、二重所属、division_army_map片方向欠落、
  存在しないDivision/Army参照、命名カウンタ衝突。
- AI(3件): CountryAi/MilitaryAiのMapキー不一致、AI状態の未知国参照。
- Frontline(6件、うち1件は受理系): 正常構成受理、所有者不一致、非参加国指揮官、
  division_frontline_map逆引き不整合、未知Division参照の生成移動。
- 安全性(6件): 複数問題の同時収集、繰り返し実行での順序安定性、Context非変更、
  極端に壊れたデータでもpanicしない、RON重複キーの実挙動確認(§16)。

---

## 20. テスト数の変更前後

| モジュール | 変更前 | 変更後 | 差分 |
|---|---|---|---|
| `save::dto` | 16 | 16 | 0 |
| `save::export` | 20 | 20 | 0 |
| `save::write` | 9 | 9 | 0 |
| `save::runtime` | 6 | 6 | 0 |
| `save::read`(新規) | 0 | 11 | +11 |
| `save::validate`(新規) | 0 | 74 | +74 |
| **save関連合計** | **51** | **136** | **+85** |
| `cargo test --lib`合計 | 230 | 315 | +85 |
| 安全な統合テスト(2つのheadless-render binaryを除く) | 59 | 59 | 0 |
| **全安全テスト合計** | **289** | **374** | **+85** |

---

## 21. 全検証結果

| 項目 | 結果 |
|---|---|
| `cargo check --all-targets` | ✅ クリーン(警告0) |
| `cargo test --lib`(315件) | ✅ 全件成功 |
| `cargo test --lib save::`(136件) | ✅ 全件成功 |
| 安全な統合テスト8バイナリ(59件、headless-render 2件除く) | ✅ 全件成功 |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ クリーン(1件の`useless_format`を修正後) |
| `cargo build --release --all-targets` | ✅ 成功(1m30s) |
| `cargo fmt --check`(新規ファイルのみ整形後) | ✅ 全体81件(既存ベースラインと完全一致、新規差分0) |
| `git diff --check` | ✅ exit 0(既存の追跡済みdirtyファイルのCRLF警告のみ、新規ファイルへの言及なし) |
| テスト後の一時ファイル残留 | ✅ なし(`saves/`ディレクトリなし、OS一時ディレクトリにこのラウンドの
  プレフィックス`strategy_game_p21_save_002c_*`のディレクトリなし) |
| 固定PNG・既存監査証拠の上書き | ✅ なし(headless-render 2バイナリはスキップ) |

---

## 22. rustfmtベースライン比較

- rustfmtバージョン: `rustfmt 1.9.0-stable (8bab26f4f6 2026-07-14)`(P21-SAVE-002B1と同一)。
- 開始時の`cargo fmt --check`差分ハンク数: 実装完了直後に計測すると**137件**(既存ベースライン
  81件 + 新規ファイル`read.rs`/`validate.rs`の56件)。
- `cargo fmt -- <path>`は本リポジトリではワークスペース全体を再整形してしまう既知の
  制約([[feedback-cargo-fmt-scope]]メモリ参照)があるため、代わりに`cargo fmt`(cargo経由)を
  一切使わず、**`rustfmt`バイナリを直接**、このラウンドで新規作成した2ファイルだけへ
  明示的に指定して実行した(`rustfmt --edition 2024 src/save/read.rs src/save/validate.rs`)。
  実行前後で`git status --short`の追跡済みファイル一覧(12件、全て既存dirty分)が完全に
  同一であることを確認し、他ファイルへ影響しなかったことを検証済み。
- 整形後の差分ハンク数: **81件**(既存ベースラインと完全一致)。新規ファイルの差分は0件。
- `src/app/time.rs`・`src/save/mod.rs`への変更は、整形前から既にrustfmt準拠だった
  (差分リストに一度も出現しなかった)。
- 既存81件は一括修正していない(要求仕様通り)。

---

## 23. 発見した問題

- **実装漏れの自己検出**: `WarJustificationRegistry`のMapキー/next_id検証は最初から
  実装していたが、`initiator`/`target`/`target_state`の参照整合性検証(§9)を当初実装し
  忘れていた。要求仕様通り「実際の型に該当フィールドが存在しない項目は削除理由を報告」
  の逆(実際に必要な検証が漏れていた)を、対応するテスト
  (`unknown_war_justification_participant_is_rejected`)の失敗によって検出し、
  `validate_war_justifications`関数を追加して修正した。この経緯自体を正直に本報告書へ
  記録する(実装が最初から完全だったと偽らない)。
- **テスト設計ミスの自己検出**: `frontline_plan_non_participant_commander_is_rejected`
  テストの初版は、意図した「既知の国だがこの前線の参加者ではない」ケースを誤って
  「実際には前線の防御側国家」で構築してしまい、テスト自体が誤っていたために失敗した
  (実装側の不具合ではなかった)。ベースラインに第3国を追加する形でテストを修正した。
- **HashMap重複キーの構造的な検出不能性**(§16): 要求仕様が事前に想定していた制限で、
  実際に確認した通り検出不能であることを確認した。V1のスキーマ変更(MapをVecへ)は
  行っていない。
- 上記以外に、実装・検証双方で未解決の問題は確認していない。

---

## 24. P21-SAVE-002D「検証済みDTOのResource適用」への移行可否

**READY**

`ValidatedSaveGameV1`は検証を通過した場合だけ構築でき、`pub(crate) into_inner`で
crate内部(P21-SAVE-002D)がそのデータを消費できる。`SaveGameV1`のV1スキーマ・
`version`の`#[serde(default)]`なし・既存コア型への`PartialEq`追加なし・
`DailySimulationSet`順序・`GameState`・`main.rs`・`src/app/loader.rs`・静的RONアセットは
いずれも変更していない。`SaveGamePlugin`は本番Appへ未接続のままであり、
P21-SAVE-002B/B1からの状態を維持している。P21-SAVE-002Dでは、この`ValidatedSaveGameV1`
だけを消費してResourceへ適用する経路を設計することを推奨する(未検証の`SaveGameV1`を
直接Resourceへ適用する経路は、今回の型設計により構造的に作れない)。
