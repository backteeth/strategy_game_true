# P21-SAVE-002D: 検証済みDTOのResourceへの原子的適用 完了報告

**実施日**: 2026-08-13
**性質**: `ValidatedSaveGameV1`をBevy Worldへ原子的に適用するPrepare→Commit二段階の
処理を実装した。ファイル読込と適用を結ぶBevy System・`LoadRequest`・`LoadGamePlugin`・
UIボタン・通知表示・`main.rs`への登録・起動直後ロード・`GameState`変更・複数スロット・
オートセーブはいずれも実装していない。P21-SAVE-002A〜002CのDTO・保存・読込・検証処理は
変更していない(`src/save/validate.rs`への変更は新規関数`check_static_master_compatibility`
の追加のみであり、既存の検証ロジックは1行も変更していない)。

---

## 1. 最終判定

**COMPLETE**

---

## 2. prepare→commit構造

`src/save/apply.rs`に実装。

```rust
pub fn apply_validated_save(world: &mut World, validated: ValidatedSaveGameV1) -> ApplyLoadOutcome {
    match prepare_load(validated, world) {
        Ok(prepared) => {
            commit_load(world, prepared);
            ApplyLoadOutcome::Success
        }
        Err(error) => ApplyLoadOutcome::Failure(error),
    }
}
```

- **Prepare**(`fn prepare_load(validated: ValidatedSaveGameV1, world: &World) -> Result<PreparedLoadGameV1, ApplyLoadError>`):
  `&World`(共有参照)しか受け取らない。必須Resourceの存在確認・ランタイム互換性確認・
  静的マスター互換性確認・全Resource値の構築(`StateRegistry::build`によるindex_map再構築、
  静的マスターデータの複製移植、AI dirtyのtrue初期化を含む)をここで完了する。
  失敗時は`&World`しか渡していないため、Worldは型システムレベルで変更不可能。
- **Commit**(`fn commit_load(world: &mut World, prepared: PreparedLoadGameV1)`):
  `&mut World`(排他参照)を受け取り、`PreparedLoadGameV1`の29個のフィールドを
  `World::insert_resource`で置き換えるだけ。戻り値なし(失敗し得ない)。
  ファイルI/O・RON解析・version検証・参照検証・fallibleな変換・sanitize・新規ID発行・
  ゲームロジック実行のいずれも行わない。

---

## 3. 原子的適用の保証範囲

過剰な保証は主張しない。実際に成立するのは次の3点だけ:

1. **Prepare失敗時は変更ゼロ**: `prepare_load`のシグネチャが`&World`しか取らないため、
   コンパイル時点で保証される(`missing_required_resource_fails_and_leaves_other_resources_unchanged`
   等のテストで実行時にも再確認)。
2. **Commit中に他システムが途中状態を観測しない**: `commit_load`は`&mut World`という
   排他参照の下でのみ呼ばれるため、Rustの借用規則により他のBevy Systemがこの関数の
   実行途中に同じWorldへアクセスすることは構造的に不可能。これは本ラウンドで実装した
   独自のロック機構ではなく、Rust/Bevyの既存の排他参照規則がそのまま提供する性質である。
3. **Commit開始後に通常の失敗分岐を持たない**: `commit_load`は`Result`を返さない
   (戻り値`()`)。全ての失敗し得る処理はPrepare側に寄せてある。

OS/DBのトランザクション(ロールバック・複数プロセス間の分離等)とは無関係であり、
そのような保証は一切主張しない。

---

## 4. ApplyLoadError一覧

```rust
pub enum ApplyLoadError {
    MissingRuntimeResource { name: &'static str },
    StaticDataMismatch { detail: String },
    RuntimeCompatibility { detail: String },
}
```

- `MissingRuntimeResource`: 適用先Worldに必須のランタイムResourceが存在しない
  (29種類全てを`prepare_load`冒頭で確認)。
- `StaticDataMismatch`: セーブが参照する静的マスター(BuildingType/技術ID/
  DivisionDefinitionId/WorldStage)が、適用先Worldの現在の静的マスターに存在しない。
- `RuntimeCompatibility`: セーブのState/CountryId集合が、適用先Worldの現在の地図構造と
  一致しない。

`ApplyLoadOutcome::{Success, Failure(ApplyLoadError)}`。失敗はPrepare段階だけで発生し、
panicは一切しない(全テストがpanicなく完了することで確認)。

---

## 5. 復元した全Resource

`commit_load`が置き換える29個のResource:

**正規状態(17個、P21-SAVE-002Bの`SaveGameSources`と対になる)**:
`GameDate`, `GameSpeed`, `PlayerCountry`, `WorldCivilizationState`, `CountryRegistry`,
`StateRegistry`, `DiplomacyRegistry`, `WarJustificationRegistry`, `WarRegistry`,
`ClaimRegistry`, `CrisisRegistry`, `CountryAiRegistry`, `MilitaryAiRegistry`,
`MilitaryRegistry`, `BattleRegistry`, `ArmyRegistry`, `FrontlineRegistry`。

**進行状態(1個)**: `GamePaused`(常に`true`へ)。

**一時状態の初期化(11個)**: `SelectedDivision`, `DragSelectState`, `SelectedArmy`,
`SelectedState`, `ActivePanel`, `DiplomacyPanelState`, `MilitaryPanelState`,
`PeacePanelState`, `PoliticsPanelState`, `ResearchPanelState`, `CameraDragState`,
`NotificationHistory`, `PendingAiWarDeclarations`(数えると13個。詳細は§9)。

`BuildingRegistry`/`TechnologyRegistry`/`MilitaryRegistry.definitions`/
`WorldCivilizationState.stage_definitions`は置き換えない(§6参照)。

---

## 6. 静的マスター維持方法

`BuildingRegistry`/`TechnologyRegistry`は`commit_load`の置換対象に含めない
(`SaveGameV1`にそもそも含まれないデータのため、Prepare/Commitのどちらでも一切触れない)。

`MilitaryRegistry.definitions`と`WorldCivilizationState.stage_definitions`は、
セーブされた可変部分(`divisions`/`current_stage`等)と同じResource型の中に同居している
ため、Prepare段階で適用先Worldの現在値を`.clone()`し、Commit用の完成済み値へ
組み込む(`MilitaryRegistry::from_saved_parts(preserved_division_definitions, ...)`、
`WorldCivilizationState { stage_definitions: preserved_stage_definitions, .. }`)。
起動時RONの再読込は一切行わない。

---

## 7. privateフィールド用の追加経路

コアモジュールへ追加した`pub(crate)`コンストラクタ(全て通常のコレクション・カウンタを
引数にとり、`crate::save`のDTO型には依存しない):

| モジュール | 追加した関数 |
|---|---|
| `app/time.rs` | `GameDate::from_saved_parts(year, month, day, accumulator)` |
| `diplomacy/claims.rs` | `ClaimRegistry::from_saved_parts(claims, next_id)` |
| `diplomacy/crisis.rs` | `CrisisRegistry::from_saved_parts(crises, next_id)` |
| `war/data.rs` | `WarRegistry::from_saved_parts(wars, next_id)` |
| `war/justification.rs` | `WarJustificationRegistry::from_saved_parts(justifications, next_id)` |
| `military/battle.rs` | `BattleRegistry::from_saved_parts(battles, next_id)` |
| `military/data.rs` | `MilitaryRegistry::from_saved_parts(definitions, divisions, next_division_id)` |
| `military/army.rs` | `ArmyRegistry::from_saved_parts(armies, division_army_map, next_id, next_army_number)` |

`FrontlineRegistry`・`DiplomacyRegistry`・`CountryRegistry`・`CountryAiRegistry`・
`MilitaryAiRegistry`・`CountryAiState`・`MilitaryAiState`は全フィールドが既に`pub`
だったため、新しいアクセサは不要(構造体リテラルで直接構築)。`StateRegistry`は
既存の`StateRegistry::build(states)`をそのまま再利用し(`index_map`をセーブ値から
復元する経路は作らなかった)、`WorldCivilizationState`も全フィールド`pub`のため
構造体リテラルで直接構築した。

---

## 8. AI dirty初期化

`prepare_load`内で`Saved(Country|Military)AiState`から`(Country|Military)AiState`へ
変換する際、`dirty`フィールド(セーブに存在しない)を常に`true`で埋める。
`CountryAiRegistry.dirty`/`MilitaryAiRegistry.dirty`(レジストリ直下のdirty)も
Commitで置き換える値の構築時に`true`で初期化する。AIをその場で評価・実行する処理は
一切呼ばない(`all_four_ai_dirty_flags_are_true_after_apply`で4つ全てを確認)。

---

## 9. 一時状態・キューの処理

実コード全体(`insert_resource`/`init_resource`呼び出し)を棚卸しし、次のように分類した:

**ロード後に空/初期値へリセットする一時状態(13個)**:
`SelectedDivision`(空)、`DragSelectState`(既定値)、`SelectedArmy`(None)、
`SelectedState`(None)、`ActivePanel`(None)、`DiplomacyPanelState`(閉)、
`MilitaryPanelState`(閉)、`PeacePanelState`(閉)、`PoliticsPanelState`(閉)、
`ResearchPanelState`(閉)、`CameraDragState`(既定値)、`NotificationHistory`(空)、
`PendingAiWarDeclarations`(空、実コード全体を検索した結果、旧ゲームの状態を次フレームへ
持ち越す一時Resourceはこの1つだけだった)。

**ロード後も維持する設定**: `SaveFileConfig`(commit_loadの置換対象に含めない)。

**用途調査の結論としてリセットしないもの**: `LastSaveOutcome`/`SaveExecutionCount`。
いずれも「セーブ」という別操作についての事実であり、ロードによって無効化される情報では
ない(前者は直近のセーブが成功したかどうかの履歴、後者はセーブ実行回数の診断カウンタ)。
ロードのたびにこれらを消す積極的な理由が実コード上見当たらなかったため、意図的に
触れないと判断した(`save_settings_are_left_untouched_by_apply`で確認)。

**次フレームに再計算される派生状態**: 州色(`update_state_colors_on_controller_change`)、
Division/Frontline表示(§11)。

**今回の適用APIから触れない、UI・入力だけの一時状態**: `PreviewCountry`
(国家選択画面専用、`GameState::Playing`中のロードでは無関係)。

---

## 10. カメラ初期化

カメラ位置・ズームはV1に保存されていない。既存方針通り、ロード後はデフォルト表示へ
戻すことを基本とする。`CameraDragState`(ドラッグ操作の一時状態)をリセットするに留め、
`GameCamera`Entity自体の再spawnは行わない(`map::camera::setup_camera`は`Startup`で
1度だけ実行される既存の仕組みであり、apply.rsが新しいカメラEntityを重複spawnすることは
ない)。カメラの`Transform`(位置・ズーム)自体は本ラウンドでは変更しない
(既存Entityの`Transform`は、ゲーム起動時に一度spawnされたまま残り続ける。これを
明示的にデフォルト値へ戻す処理は、Entityの直接操作が必要になるため今回のResource
置換ベースの設計には含まれない。影響は限定的[次にWASD/ドラッグ操作すれば通常通り
動作する]と判断し、タスクEの残件として扱う)。

---

## 11. 描画・UI同期方法

実コードを確認した結果、以下の2パターンで自動的に追従することを確認した:

1. **無条件の毎フレーム全件再構築**(`sync_division_visuals`/`update_frontline_overlay`、
   `map/division_render.rs`/`map/frontline_render.rs`): `is_changed()`ガードを一切持たず、
   毎フレーム現在の`MilitaryRegistry`/`ArmyRegistry`/`StateRegistry`/`WarRegistry`/
   `FrontlineRegistry`から全エンティティを再構築する(既存Entityとの差分をとり、
   不要なものをdespawn・不足分をspawn)。Resourceが丸ごと置き換わっても、次のフレームで
   自動的に新データへ追従する。
2. **`Res<T>::is_changed()`ガード**(`update_state_colors_on_controller_change`
   [`map/rendering.rs:107`]、上部UIの日付/速度/一時停止表示
   [`ui/top_bar.rs:254`、`date`/`paused`/`speed`/`locale`のいずれかが変化した場合に更新]):
   `commit_load`の`World::insert_resource`によるResource置換が、この`is_changed()`判定を
   満たすことを実際に検証した(`resource_replacement_is_observed_as_changed_by_downstream_systems`)。

Army一覧パネル(`ui/military_panel.rs`)は`!state.open && !locale.is_changed()`の場合だけ
更新処理をスキップする設計であり、パネルを開くたびに`ArmyRegistry`から全行を再構築する
(ロード後は全パネルが閉じているため、次に開いた瞬間に新データで再構築される)。

**実施しなかったこと(正直な申告)**: `DivisionRenderPlugin`/`FrontlineRenderPlugin`
そのものを実際にBevy Appへ登録して自動テストする、という文字通りの統合テストは
実施しなかった。これらのPluginは`Window`/`GameCamera`Entityへ依存する系統のシステムを
含み、ヘッドレスなユニットテスト環境で安全に模擬する既存の前例がこのリポジトリに
存在しない(このプロジェクトが2つのheadless-render統合テストバイナリを固定PNG上書き
リスクのため意図的に除外している既存方針と同じ理由)。代わりに、これらのシステムが
依存する土台となるBevyの仕組み(`Res<T>::is_changed()`とResource置換の関係)を
実際に検証し、かつ本番システムのソースコードを直接確認することで、実装ではなく
検証方法を安全側に倒した。

---

## 12. ランタイム互換性検査

Prepare段階で、セーブの`StateId`集合・`CountryId`集合が、適用先Worldの現在の
`StateRegistry`/`CountryRegistry`の集合と**完全に一致**することを確認する
(部分一致・上位互換は許容しない)。一致しなければCommitへ進む前に
`ApplyLoadError::RuntimeCompatibility`で拒否する。

理由: 州の描画Entity(`StateVisual`等)はゲーム起動時に静的マップデータから1度だけ
spawnされ、Division/Frontlineのように毎フレーム全再構築されないため、セーブの州集合が
現在のマップと異なると、一部の州が正しく同期できない状態が生まれ得る。既存の
7か国・28州マップから生成された通常のセーブは、常にこのマップ自身と同じID集合を
持つため、この検査は通常ケースを妨げない
(`map_or_render_incompatibility_is_rejected_before_applying`で異なる州集合を注入し拒否を確認)。

静的マスター参照の互換性は別のチェック(§4の`StaticDataMismatch`)として分離した。
002Cの`SaveValidationContext`構築経路をそのまま再利用し、`validate.rs`へ新規追加した
小さな`check_static_master_compatibility`関数(既存の参照整合性検証ロジックは複製せず、
静的マスターの`contains_key`確認だけを独立して持つ)で判定する。

---

## 13. 変更ファイル一覧

**新規作成**:
- `src/save/apply.rs`(Prepare/Commit/`ApplyLoadError`/`ApplyLoadOutcome`/
  `PreparedLoadGameV1`、37テスト)
- `verification_logs/phase-21/p21-save-002d/p21-save-002d_completion_report.md`(本ファイル)

**変更**:
- `src/save/mod.rs`(`apply`モジュールの登録・re-export追加のみ)
- `src/save/validate.rs`(新規関数`check_static_master_compatibility`の追加のみ。
  既存の検証ロジック・`ValidatedSaveGameV1`・`SaveValidationContext`等は無変更)
- `src/app/time.rs`(`GameDate::from_saved_parts`追加のみ)
- `src/diplomacy/claims.rs`(`ClaimRegistry::from_saved_parts`追加のみ)
- `src/diplomacy/crisis.rs`(`CrisisRegistry::from_saved_parts`追加のみ)
- `src/war/data.rs`(`WarRegistry::from_saved_parts`追加のみ)
- `src/war/justification.rs`(`WarJustificationRegistry::from_saved_parts`追加のみ)
- `src/military/battle.rs`(`BattleRegistry::from_saved_parts`追加のみ)
- `src/military/data.rs`(`MilitaryRegistry::from_saved_parts`追加のみ)
- `src/military/army.rs`(`ArmyRegistry::from_saved_parts`追加のみ)

**変更していない**(要求仕様通り):
`src/save/dto.rs`・`src/save/export.rs`・`src/save/write.rs`・`src/save/runtime.rs`・
`src/save/read.rs`、`main.rs`、`src/app/loader.rs`、`src/app/game_state.rs`、静的RONアセット、
UI関連ファイル(構造体定義以外)、P21-005関連ファイル。

---

## 14. 追加テスト一覧

`src/save/apply.rs`(37件):

- **型・原子性(7)**: `ValidatedSaveGameV1`のみ適用可能、Prepare完了までWorld不変、
  必須Resource不足時の失敗と他Resource不変、静的マスター不一致時の失敗と状態保持、
  マップ互換性不一致時の事前拒否、Commit一括置換の一貫性、未検証SaveGameV1への
  公開経路がないことの構造的確認。
- **正規状態(13)**: GameDate/accumulator、GameSpeed、PlayerCountry、GamePaused(true)、
  CountryData全体、StateData全体、StateRegistry.index_map再構築、
  Diplomacy/WarJustification/War、Claim/Crisisとnext_id、Divisionとnext_division_id、
  Battleとnext_id、Army全4フィールド、Frontline全5フィールド、
  WorldCivilizationStateの可変部分。
- **静的・派生状態(2)**: 4種の静的マスター定義が変更されないこと、Resource置換が
  `is_changed()`で観測されること(州色/上部UI同期の土台となる仕組みの直接検証)。
- **AI・一時状態(7)**: AI正規状態の復元、4つのdirtyが全てtrue、
  Division/Army/State選択の解除、全パネルが閉じること、カメラ・ドラッグ状態の初期化、
  通知履歴・旧キューのクリア、保存設定(SaveFileConfig/LastSaveOutcome/
  SaveExecutionCount)が維持されること。
- **継続性(8)**: 移動途中Divisionの継続可能性、戦闘途中Division/Battleの継続可能性、
  新規DivisionId/ArmyId/War・BattleIdの非衝突(3件)、ロード直後の再セーブが元DTOと
  意味的に一致すること、同一セーブの別World適用が同一結果になること。

---

## 15. テスト数の変更前後

| モジュール | 変更前 | 変更後 | 差分 |
|---|---|---|---|
| `save::dto` | 16 | 16 | 0 |
| `save::export` | 20 | 20 | 0 |
| `save::write` | 9 | 9 | 0 |
| `save::runtime` | 6 | 6 | 0 |
| `save::read` | 11 | 11 | 0 |
| `save::validate` | 74 | 74 | 0 |
| `save::apply`(新規) | 0 | 37 | +37 |
| **save関連合計** | **136** | **173** | **+37** |
| `cargo test --lib`合計 | 315 | 352 | +37 |
| 安全な統合テスト(headless-render 2件除く) | 59 | 59 | 0 |
| **全安全テスト合計** | **374** | **411** | **+37** |

---

## 16. save→load→saveの意味的一致

`reloading_after_apply_reproduces_the_original_save_semantically`で、代表的なセーブA
(2か国・2州・移動中/戦闘中Division各1・Army1・War1・外交関係1・AI状態を含む)を検証済み
Worldへ適用した直後に`build_save_game_v1`で再エクスポートし、元のDTOとフィールド単位で
比較した。日付・速度・プレイヤー国家・国家データ・Division数/next_id・Army数・War数・
Battle数・AI正規状態(`SavedCountryAiState`の`PartialEq`による完全一致)が全て一致することを
確認した。`GamePaused`・AI dirty・UI選択状態・カメラはそもそも`SaveGameV1`に存在しない
フィールドのため比較対象から除外した(既存コア型へ`PartialEq`を追加せず、フィールド単位の
意味比較を使うP21-SAVE-002A以降の一貫した方針を踏襲)。

---

## 17. 移動・戦闘継続

`moving_division_keeps_movement_state_and_can_continue_after_reload`:
`status: Moving`・`destination: Some(StateId(0))`・`current_path: [StateId(0)]`・
`movement_progress: 0.35`を持つDivisionが、ロード後も全フィールドを保持することを確認した
(`military::update`等の移動処理システムは変更していないため、次の日次更新でそのまま
移動を継続できる状態になっている)。

`fighting_division_and_battle_keep_combat_state_and_can_continue_after_reload`:
`status: Fighting`・`combat_id: Some(BattleId(0))`のDivisionと、対応する
`status: Ongoing`のBattle(該当Divisionを`defender_division_ids`に含む)が、ロード後も
双方向の整合性を保ったまま復元されることを確認した。

---

## 18. 全検証結果

| 項目 | 結果 |
|---|---|
| `cargo check --all-targets` | ✅ クリーン(警告0) |
| `cargo test --lib`(352件) | ✅ 全件成功 |
| `cargo test --lib save::`(173件) | ✅ 全件成功 |
| 安全な統合テスト8バイナリ(59件、headless-render 2件除く) | ✅ 全件成功 |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ クリーン |
| `cargo build --release --all-targets` | ✅ 成功(約54秒) |
| `cargo fmt --check`(新規差分ファイルのみ整形後) | ✅ 全体81件(既存ベースラインと完全一致、新規差分0) |
| `git diff --check` | ✅ exit 0(既存の追跡済みdirtyファイルのCRLF警告のみ) |
| テスト後の一時ファイル・プロセス残留 | ✅ なし(`saves/`ディレクトリなし。apply.rsのテストは
  全て`World`上のメモリ操作のみでファイルI/Oを一切行わないため、そもそも残留し得ない) |

---

## 19. rustfmtベースライン比較

- rustfmtバージョン: `rustfmt 1.9.0-stable (8bab26f4f6 2026-07-14)`(前ラウンドと同一)。
- 実装直後の`cargo fmt --check`差分ハンク数: **106件**(既存ベースライン81件 + 新規25件:
  `src/diplomacy/claims.rs`1件、`src/save/validate.rs`3件、新規`src/save/apply.rs`21件)。
  `src/app/time.rs`・`src/diplomacy/crisis.rs`・`src/war/data.rs`・
  `src/war/justification.rs`・`src/military/battle.rs`・`src/military/data.rs`・
  `src/military/army.rs`・`src/save/mod.rs`への変更は整形前から既にrustfmt準拠だった。
- `cargo fmt -- <path>`は本リポジトリではワークスペース全体を再整形してしまう既知の制約
  ([[feedback-cargo-fmt-scope]]メモリ参照)があるため、`cargo fmt`を一切使わず、
  `rustfmt`バイナリを新規差分のあった3ファイルだけへ明示的に指定して実行した
  (`rustfmt --edition 2024 src/diplomacy/claims.rs src/save/validate.rs src/save/apply.rs`)。
  実行前後で`git status --short`の追跡済みファイル一覧が完全に同一であることを確認し、
  他ファイルへ影響しなかったことを検証済み。
- 整形後の差分ハンク数: **81件**(既存ベースラインと完全一致)。新規差分は0件。
- 既存81件は一括修正していない(要求仕様通り)。

---

## 20. 002C報告の数え方の追補

`verification_logs/phase-21/p21-save-002c/p21-save-002c_completion_report.md`は
上書きせず、ここで確認・訂正する。

**§7「全ValidationCode」の件数訂正**: 見出しに「17種」と記載していたが、実コード
(`src/save/validate.rs`の`SaveValidationCode` enum)および同報告書内の表を実際に
数え直した結果、正しくは**16種**だった(見出しの「17種」が誤り、表の16行自体は
当時から正しかった)。列挙: `DuplicateId`, `MapKeyMismatch`, `MissingValue`,
`DanglingReference`, `InvalidRange`, `NonFiniteValue`, `AsymmetricAdjacency`,
`SelfAdjacency`, `DuplicateAdjacency`, `NextIdCollision`, `OwnershipMismatch`,
`ParticipantMismatch`, `ReverseMapInconsistent`, `EmptyCollection`,
`DuplicateMembership`, `SetOverlap`(16種)。

**§19「追加テスト一覧」のカテゴリ別件数訂正**: `src/save/validate.rs`のテスト関数を
実際に定義順のセクションコメント([`// ─── ベースライン ───`]等)ごとに再集計した結果、
報告書記載の内訳(4+21+13+2+7+8+3+6+6=70件)が実際の合計74件と一致していなかった
(6件の計上漏れ)。実コードを正しく数え直した内訳は次の通り(合計74件、変更なし):

| カテゴリ | 報告書記載 | 実際の件数 |
|---|---|---|
| ベースライン/受理系 | 4 | **5** |
| 基本状態 | 21 | **26** |
| 外交・戦争 | 13 | 13(一致) |
| Registry | 2 | 2(一致) |
| Division/Battle | 7 | 7(一致) |
| Army | 8 | 8(一致) |
| AI | 3 | 3(一致) |
| Frontline | 6 | **5** |
| 安全性 | 6 | **5** |
| **合計** | **70** | **74** |

「基本状態」区分に含めていた項目のうち複数(建物種別・技術ID・師団定義の静的マスター
参照テスト等)が当時のカテゴリ内訳の集計から漏れていたことが主な原因である。
テスト総数そのもの(74件)・save関連合計(136件)・grand合計(374件)はいずれも当時から
正しく、変更しない(要求仕様通り、推測で変更していない。今回改めて
`cargo test --lib save::validate:: -- --list`の実出力を数え直して確認した)。

---

## 21. 発見した問題

- 上記2件の002C報告の数値誤り(§20)以外に、実装・検証双方で未解決の問題は
  確認していない。
- カメラの`Transform`(位置・ズーム)自体はロード後もリセットしない(§10で詳述)。
  影響は限定的(次の操作で通常通り追従する)と判断したが、正確な挙動としては
  「ロード前のカメラ位置がロード後も一瞬残る」ことを申告する。将来、UIタスクで
  ロード起点のカメラ再センタリングが必要と判断された場合はタスクEの対象とする。

---

## 22. P21-SAVE-002E「ゲーム内セーブ／ロード導線」への移行可否

**READY**

`apply_validated_save(world, validated)`は`ValidatedSaveGameV1`だけを受け取る唯一の
公開適用APIとして完成しており、`read_and_validate_save_file`(P21-SAVE-002C)の戻り値を
そのまま渡せる形になっている。P21-SAVE-002Eでは、これらを実際に結ぶBevy System・
`LoadRequest`Message・`LoadGamePlugin`・UIボタン・通知表示を設計し、`main.rs`へ
`SaveGamePlugin`と併せて登録することを推奨する。§10・§21で申告したカメラ位置の
リセットについては、UIタスクの一部として明示的に扱うことを推奨する。
