# P21-SAVE-002B: Resource→DTO変換と安全な単一スロット保存 完了報告

**実施日**: 2026-08-13
**性質**: タスクB(Resource→DTO変換・PostUpdateでのSnapshot取得・RONシリアライズ・安全な
単一スロット書き込み)の実装。ロード処理・バージョン検証・参照整合性検証・DTOからResourceへの
適用・UIボタン・通知表示・起動直後ロード・複数スロット・オートセーブ・非同期I/O・
P21-005前線拡張はいずれも実装していない。

---

## 1. 最終判定

**COMPLETE**

---

## 2. Resource→DTO変換経路

```
Bevy World (Res<T> × 17)
  → runtime::SaveGameResourceParams<'w> (#[derive(SystemParam)]、Res<T>を束ねるだけ)
    → .as_sources() で export::SaveGameSources<'a> (プレーンな&'a Xxx の束、Bevy非依存) へ変換
      → export::build_save_game_v1(&sources) -> SaveGameV1 (純粋関数、Bevy非依存)
        → write::write_save_file(&save, &config) -> SaveOutcome (ファイルI/O、Bevy非依存)
```

`export.rs`と`write.rs`はいずれも`bevy`を一度もimportしていない(ファイル冒頭のuse文一覧で
確認可能)。Bevy固有の型(`Res<T>`/`SystemParam`/`Message`/`Plugin`/スケジュール接続)は
`runtime.rs`だけに閉じ込めた。`build_save_game_v1`は`&SaveGameSources`(共有参照の束)のみを
受け取り、`&mut`を一切取らないため、ランタイムResourceを変更できないことが関数シグネチャ
自体によってコンパイル時に保証される。

`build_save_game_v1`は`SaveGameSources`(プレーンなRust構造体、`Res<T>`ではなく`&'a Xxx`の束)
を引数に取るため、Bevy World/Appを一切起動せずに直接テストできる(`export.rs`の全17テストが
これを実証)。Bevy Systemとして実際に呼び出す経路(`runtime::handle_save_requests`)は、
`SaveGameResourceParams`(17個の`Res<T>`をまとめた`#[derive(SystemParam)]`構造体。
`src/localization.rs`の`Loc<'w>`と同じ確立済みパターンを踏襲)を介して`SaveGameSources`を
組み立て、`build_save_game_v1`へ委譲するだけの薄いラッパーになっている。

---

## 3. SaveGameV1へ保存した全Resource

| Resource | 抽出内容 |
|---|---|
| `GameDate` | `year`/`month`/`day`/`accumulator`(全て) |
| `GameSpeed` | `.0` |
| `PlayerCountry` | `.0` |
| `WorldCivilizationState` | `current_stage`/`milestone_countries`/`last_advanced_date`のみ(`stage_definitions`除外) |
| `CountryRegistry` | `countries`(全体) |
| `StateRegistry` | `states`(全体、`index_map`除外) |
| `DiplomacyRegistry` | `relations`(全体) |
| `WarJustificationRegistry` | `justifications`+`next_id()` |
| `WarRegistry` | `wars`+`next_id()` |
| `ClaimRegistry` | `claims`+`next_id()`(空でも必須フィールドとして含む) |
| `CrisisRegistry` | `crises`+`next_id()`(空でも必須フィールドとして含む) |
| `CountryAiRegistry` | `ai_states`の正規状態のみ(`dirty`除外、要素ごとに`SavedCountryAiState`へ変換) |
| `MilitaryAiRegistry` | `ai_states`の正規状態のみ(`dirty`除外、要素ごとに`SavedMilitaryAiState`へ変換) |
| `MilitaryRegistry` | `divisions`+`next_division_id()`のみ(`definitions`除外) |
| `BattleRegistry` | `battles`+`next_id()` |
| `ArmyRegistry` | `armies`+`division_army_map`+`next_id()`+`next_army_number`(全4フィールド) |
| `FrontlineRegistry` | `frontlines`+`plans`+`next_frontline_id`+`division_frontline_map`+`frontline_generated_movements`(全5フィールド) |

---

## 4. 保存対象外データ

`GamePaused`は`SaveGameSources`のフィールドとして存在せず、`build_save_game_v1`の変換元
として一切受け取っていない(構造的に不可能: 関数シグネチャにそのような入力経路がない)。
`MilitaryRegistry.definitions`・`WorldCivilizationState.stage_definitions`・
`StateRegistry.index_map`(いずれも静的マスターデータ/派生キャッシュ)は読み取っていない
(`export::tests::military_registry_definitions_are_not_converted`/
`only_mutable_part_of_world_civilization_state_is_converted`/
`state_registry_index_map_is_not_converted`で、元Resourceには実際にそのデータが
入っていることを確認したうえで、変換結果に含まれないことを検証済み)。
`bevy::prelude::Entity`・描画Entity・UI選択状態(`SelectedDivision`/`SelectedArmy`/
`SelectedState`)・カメラは、`export.rs`が`bevy`を一切importしていないことに加え、
`SaveGameSources`のフィールドとしても存在しない。

---

## 5. AI dirty除外の実装

`export::export_country_ai_state`/`export::export_military_ai_state`が、ランタイムの
`CountryAiState`/`MilitaryAiState`から`dirty`以外の全フィールドだけを個別にコピーして
`SavedCountryAiState`/`SavedMilitaryAiState`を構築する(P21-SAVE-002A1で確定した分類・
DTO定義をそのまま使用)。レジストリ直下の`CountryAiRegistry.dirty`/`MilitaryAiRegistry.dirty`
も、`SavedCountryAiRegistry`/`SavedMilitaryAiRegistry`にそもそもそのフィールドが存在しない
(P21-SAVE-002A1で削除済み)ため、読み込みコード自体に触れようがない。

要求された「dirtyだけ異なる2つのランタイムAI状態から意味的に同一のSaved AI DTOが生成される」
ことは、`export::tests::dirty_only_difference_produces_semantically_identical_saved_country_ai_dto`/
`_saved_military_ai_dto`で、`SavedCountryAiState`/`SavedMilitaryAiState`が既に持つ`PartialEq`
(P21-SAVE-002A1で獲得済み)による値全体の`==`比較で確認した。セーブ処理がランタイムAI状態の
`dirty`を変更しないことは、`export::tests::conversion_does_not_mutate_source_resources`で、
変換前後の`dirty`値を直接比較して確認した(加えて、`build_save_game_v1`が共有参照しか
取らないという型レベルの保証もある)。

---

## 6. privateフィールドへ追加したアクセサ

指示§4の通り、既存フィールドを`pub`化せず、フィールドを所有するモジュールへ最小限の
読み取り専用アクセサ/正規表現の生成メソッドを個別に追加した。全て**なぜ必要だったか**込みで報告する:

| 追加箇所 | 種別 | 理由 |
|---|---|---|
| `app/time.rs`: `GameDate::accumulator(&self) -> f64`(`pub(crate)`) | 読み取り専用アクセサ | `accumulator`はGameDateの唯一のprivateフィールドで、既存の`next_id()`系メソッドのような同一モジュール内の類似先例がなかったため、指示が示す選択肢のうち最も保守的な`pub(crate)`を採用した |
| `diplomacy/claims.rs`: `ClaimRegistry::next_id(&self) -> usize`(`pub`) | 読み取り専用アクセサ | 同じ`WarRegistry`/`BattleRegistry`/`WarJustificationRegistry`が既に`pub fn next_id(&self) -> usize`という全く同じ形のアクセサを持っており(P21-SAVE-002A以前から存在する確立済みパターン)、それに合わせて`pub`にした。`pub(crate)`にすると、隣接する同型アクセサ群だけ可視性が異なる一貫性のなさが生じるため |
| `diplomacy/crisis.rs`: `CrisisRegistry::next_id(&self) -> usize`(`pub`) | 読み取り専用アクセサ | 上記と同一理由 |
| `military/army.rs`: `ArmyRegistry::next_army_number_map(&self) -> &HashMap<CountryId, u32>`(`pub(crate)`) | 読み取り専用アクセサ | `next_army_number`(国家ごとの命名カウンタ)は`HashMap`であり、既存の`next_id()`(`usize`を値で返す)とは形が異なる。この形の借用アクセサに対応する既存の先例が同モジュールになかったため、`pub(crate)`を採用した(`&HashMap`を返し、呼び出し側`export.rs`で`.clone()`する設計、ハッシュマップ全体を不用意に公開APIへ晒さないため) |

いずれも既存フィールドの可視性そのものは変更していない(`pub`化なし)。Registry全体への
機械的な`Serialize`追加、Bevy World全体のダンプ、`unsafe`によるprivateフィールドアクセス、
静的データの二重保存は一切行っていない。

---

## 7. SaveRequestの方式

`runtime::SaveRequestMessage`(`#[derive(Message, Debug, Clone, Copy)]`の一回性マーカー型、
フィールドなし)を`app.add_message::<SaveRequestMessage>()`で登録した。この`Message`/
`add_message`/`MessageWriter`/`MessageReader`という命名は、既存コードベースで確立済みの
Bevy 0.19 API(`app/time.rs`の`DayChangedMessage`、`ui/*`の`GameNotification`等)と完全に
同じ形であり、旧版のBevy Event APIを推測で使っていない。

`runtime::handle_save_requests`は`MessageReader<SaveRequestMessage>::read()`を`.count()`で
一括消費し、1件以上あれば高々1回だけ`build_save_game_v1`→`write_save_file`を実行する
(同一フレーム内の複数要求の集約)。`.run_if(in_state(GameState::Playing))`で登録するため、
`Playing`以外ではシステム自体が実行されない(システム内で状態を都度チェックする代わりに、
既存コードベース全体で確立済みの`run_if`パターンに合わせた)。

---

## 8. PostUpdateへの登録方法

`SaveGamePlugin::build`が`app.add_systems(PostUpdate, handle_save_requests.run_if(in_state(GameState::Playing)))`
で登録する。`DailySimulationSet`の`configure_sets`(`app/time.rs`)には一切触れておらず、
`Update`スケジュールへの新規登録も行っていない。`PostUpdate`はそのフレームの`Update`
スケジュール全体(`DailySimulationSet`の全チェーンを含む)が完了した後にのみ実行される
Bevy標準スケジュールであり、移動・戦闘・Army清掃・講和処理後の確定状態を保存できる。
`export::tests::resource_mutated_in_update_is_reflected_in_the_same_frame_save`で、
同一フレームの`Update`で変更された値(この場合`GameSpeed`)が同フレームの保存結果へ
正しく反映されることを実際に確認した。

**`main.rs`へは追加していない**: このラウンドではUIボタンも`SaveRequestMessage`の実際の
発行元も存在しないため、`SaveGamePlugin`を本番ゲームバイナリの`.add_plugins(...)`リストへは
追加していない。実際に`cargo run`したゲームの挙動には一切影響がなく、`main.rs`自体の
`git diff`は空である。テストコードが明示的に`.add_plugins(SaveGamePlugin)`した場合のみ動作する。

---

## 9. 既定保存パスとテスト時の差し替え方法

`write::SavePathConfig { final_path: PathBuf }`の`Default`実装が、プロジェクト相対
`saves/savegame_v1.ron`を返す。`runtime::SaveFileConfig { path: SavePathConfig }`
(`#[derive(Resource, Default)]`)が、この設定をBevy Resourceとして保持する。

テストでは、`SaveFileConfig { path: SavePathConfig { final_path: <一意なOS一時ディレクトリ>.join("savegame_v1.ron") } }`
を明示的に`app.insert_resource(...)`で上書きする(`init_resource`によるデフォルト値挿入後、
`insert_resource`で上書きするBevyの標準パターン)。一時ディレクトリは
`std::env::temp_dir()`配下に、プロセスID・アトミックカウンタ・ナノ秒タイムスタンプを
組み合わせた一意な名前で作成し、`Drop`実装(`TempTestDir`)でテスト終了時(パニック時含む)に
必ず削除する。全50件の`save::*`テストを実行後、リポジトリ相対`saves/`ディレクトリも
OS一時ディレクトリの残骸も一切存在しないことを`find`/`ls`で確認済み(§14参照)。

OS標準ユーザーデータディレクトリへの将来移行は、`SavePathConfig::default()`が返す
`final_path`の決定方法を変えるだけで完結し、`SaveGameV1`・`export.rs`・呼び出し側コードには
一切影響しない(パスをDTOへ保存する設計にしていないため)。新規依存クレートは追加していない
(`std::env::temp_dir()`/`std::process::id()`/`std::time::SystemTime`のみ使用)。

---

## 10. RONシリアライズ方法

`ron::ser::to_string_pretty(save, ron::ser::PrettyConfig::default())`(`ron 0.12.2`の実在API、
`ron`クレートのソースを直接確認して存在を検証済み)を使用。既存のDTO単体テスト
(`dto.rs`)は今まで通り`ron::to_string`(compact)を使い続けており、変更していない
(型レベルの往復検証には整形は無関係なため)。実ファイルへの書き込みだけ、人間が調査可能な
pretty形式にした。HashMapの反復順序を固定する処理は追加していない。シリアライズ失敗時は
`SaveOutcome::Failure { error: SaveError::Serialize(..) }`を返すのみで、`panic!`しない。

---

## 11. 一時ファイルと置換手順

`write::write_save_file`が指示された8ステップを厳守する順序で実装:
1. `final_path`の親ディレクトリを`create_dir_all`
2. `SaveGameV1`をRON文字列へ完全にシリアライズ(この時点ではまだファイルに触れない)
3. `<final_path>.tmp`を`File::create`で開く(既存のstale `.tmp`があっても上書きするため
   安全に再試行できる)
4. RON全体を`write_all`
5. `flush`
6. `sync_all`(OSバッファではなくディスクへの書き込みを保証)
7. `fs::rename(temp, final)`で置換
8. 成功時、一時ファイルは`rename`によって既に移動済みで残らない

失敗時は`fs::remove_file(&temp_path)`のみを試み(結果は`let _ =`で無視、後始末失敗で
`panic!`しない)、`final_path`には一切触れない。`remove_file(final) → rename(tmp, final)`の
経路は実装していない。

---

## 12. Windows上書きテスト結果

`write::tests::second_save_safely_updates_the_same_slot_on_windows`で実証済み:
1回目の保存で`final_path`に内容Aを書き込み、`fs::rename`を経由して成功することを確認。
続けて同じ`final_path`(既に存在する状態)へ2回目の保存(内容B)を行い、`SaveOutcome::Success`
が返ること、`final_path`の内容が完全に内容Bへ置き換わっていること、`.tmp`が残っていないことを
確認した。**この環境(Windows、rustc 1.97.1)の`std::fs::rename`は、既存の最終ファイルを
削除する前処理なしに安全に置換できることを実テストで確認済み。** `remove_file(final)`を
先に呼ぶフォールバックの実装は不要だった。

---

## 13. 保存失敗時の既存ファイル保護

`write::tests::write_failure_preserves_existing_final_file_and_does_not_panic`で実証済み:
1回目の保存を成功させた後、次の一時ファイルパス(`<final>.tmp`)をディレクトリとして
先取りすることで(`File::create`が確実に失敗する、環境非依存の再現性ある失敗注入方法)、
2回目の保存を`SaveError::CreateTempFile`で失敗させた。失敗後も`final_path`の内容が
1回目のまま一切変化していないこと、テスト自体がパニックせず正常終了することを確認した。
`write::tests::save_outcome_records_success_and_failure_structurally`でも、
`CreateDirectory`失敗(親パスを通常ファイルとして塞ぐ)のケースで同様に構造化された
`SaveOutcome::Failure`が返ることを確認済み。

---

## 14. 保存結果の構造

```rust
pub enum SaveError {
    Serialize(String),
    CreateDirectory(String),
    CreateTempFile(String),
    Write(String),
    FlushOrSync(String),
    Rename(String),
}

pub enum SaveOutcome {
    Success { path: PathBuf },
    Failure { path: PathBuf, error: SaveError },
}
```

`runtime::LastSaveOutcome(pub Option<SaveOutcome>)`(非永続Resource、`SaveGameV1`へは
シリアライズされない)が直近の結果を保持し、将来のUIタスクがそのまま表示に使える。
技術的詳細(各`SaveError`バリアントの`String`)はこのResourceにのみ保持され、
`SaveGameV1`(ゲーム状態の永続DTO)には一切含まれない。診断用の`SaveExecutionCount`
(実際にファイルへ書き込みを試みた回数)も同様に非永続Resourceとして追加した
(同一フレーム複数要求の集約を実テストで検証するために導入)。

---

## 15. 変更ファイル一覧

正直に、新規作成・変更した全ファイルを列挙する:

| ファイル | 種別 | 内容 |
|---|---|---|
| `src/save/export.rs` | 新規 | `SaveGameSources`+`build_save_game_v1`(Bevy非依存の純粋変換)、17件のテスト |
| `src/save/write.rs` | 新規 | `SavePathConfig`/`SaveError`/`SaveOutcome`/`write_save_file`(Bevy非依存のファイルI/O)、9件のテスト |
| `src/save/runtime.rs` | 新規 | `SaveRequestMessage`/`LastSaveOutcome`/`SaveExecutionCount`/`SaveFileConfig`/`SaveGameResourceParams`/`handle_save_requests`/`SaveGamePlugin`(Bevy接続層)、6件のテスト |
| `src/save/mod.rs` | 変更 | 新規3サブモジュールの登録・re-export追加 |
| `src/app/time.rs` | 変更(+5行) | `GameDate::accumulator()`(`pub(crate)`)アクセサ追加 |
| `src/diplomacy/claims.rs` | 変更(+5行) | `ClaimRegistry::next_id()`(`pub`)アクセサ追加 |
| `src/diplomacy/crisis.rs` | 変更(+5行) | `CrisisRegistry::next_id()`(`pub`)アクセサ追加 |
| `src/military/army.rs` | 変更(+7行) | `ArmyRegistry::next_army_number_map()`(`pub(crate)`)アクセサ追加 |
| `verification_logs/phase-21/p21-save-002b/p21-save-002b_completion_report.md` | 新規 | 本報告書 |

上記以外のファイル(ゲームコード・アセット・既存テスト・`main.rs`)は一切変更していない。
既存の未コミット差分(`assets/localization/{en-US,ja-JP}.ron`、`src/map/{division_selection,
rendering,selection}.rs`、`src/military/mod.rs`、`src/ui/military_panel.rs`、
`verification_logs/phase-21/p21-004/`、`verification_logs/phase-21/p21-save-001/`、
`verification_logs/phase-21/p21-save-002a/`、`verification_logs/phase-21/p21-save-002a1/`)は
過去タスク由来の既存作業であり、本タスクでは一切手を加えていない(`git status`で確認済み)。

---

## 16. 追加テスト一覧

50件全て`src/save/{export,write,runtime}.rs`内の`#[cfg(test)] mod tests`に追加(既存の
`dto.rs`側16件は変更なし)。

**`export.rs`(20件、Resource→DTO純粋変換)**: `builds_full_save_game_v1_with_correct_version`、
`game_date_including_accumulator_is_converted`、`game_speed_and_player_country_are_converted`、
`country_and_state_normative_state_is_converted`、`diplomacy_war_and_justification_are_converted`、
`claim_and_crisis_are_present_as_fields_even_when_empty`、
`claim_and_crisis_contents_and_next_id_are_converted_when_nonempty`、
`division_and_next_division_id_are_converted`、`battle_and_next_id_are_converted`、
`army_membership_reverse_map_and_naming_counter_are_converted`、
`frontline_registry_all_five_fields_are_converted`、
`only_mutable_part_of_world_civilization_state_is_converted`、
`military_registry_definitions_are_not_converted`、`state_registry_index_map_is_not_converted`、
`conversion_does_not_require_gamepaused_entity_ui_or_camera`、
`conversion_does_not_mutate_source_resources`、
`country_ai_normative_state_is_converted_and_dirty_is_dropped`、
`military_ai_normative_state_is_converted_and_dirty_is_dropped`、
`dirty_only_difference_produces_semantically_identical_saved_country_ai_dto`、
`dirty_only_difference_produces_semantically_identical_saved_military_ai_dto`

**`write.rs`(8件、ファイルI/O)**: `creates_missing_temp_directory_automatically`、
`saved_file_round_trips_through_ron_from_str`、`saved_ron_contains_version_field`、
`no_tmp_file_remains_after_successful_save`、`second_save_safely_updates_the_same_slot_on_windows`、
`write_failure_preserves_existing_final_file_and_does_not_panic`、
`save_outcome_records_success_and_failure_structurally`、
`tests_use_only_os_temp_dir_never_the_repository_saves_directory`

**`runtime.rs`(6件、Bevy接続)**: `no_save_request_creates_no_file`、
`save_request_outside_playing_state_does_not_save`、
`save_request_while_playing_saves_via_post_update`、
`multiple_save_requests_in_one_frame_collapse_to_a_single_save`、
`resource_mutated_in_update_is_reflected_in_the_same_frame_save`、
`save_does_not_change_game_paused`

比較はいずれもフィールド単位の意味比較(`PartialEq`が使えるAI DTOは値全体の`==`、それ以外は
`HashMap::get`によるキー引き+個別フィールド比較)を使用しており、既存コア型
(`CountryData`/`StateData`/`Division`/`Army`/`War`/`Battle`/`Frontline`等)へ`PartialEq`は
一切追加していない(§8で確認済み)。

---

## 17. テスト数の変更前後

| 項目 | P21-SAVE-002A1完了時点 | 本ラウンド完了後 | 差分 |
|---|---|---|---|
| `cargo test --lib`(単体テスト) | 195 | 229 | +34 |
| `save::export::tests` | 0 | 20 | +20 |
| `save::write::tests` | 0 | 8 | +8 |
| `save::runtime::tests` | 0 | 6 | +6 |
| `save::dto::tests`(既存、変更なし) | 16 | 16 | ±0 |
| 安全な統合テストスイート合計(headless描画2件を除く8バイナリ) | 59 | 59 | ±0 |
| 合計(単体+安全な統合テスト) | 254 | 288 | +34 |

新規追加34件はすべて上記の通り。既存254件の内容・件数は一切変更していない
(全288件が緑、回帰なし)。

---

## 18. 全検証結果

作業ディレクトリ: `strategy_game/`(プロジェクトルート)。

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | 成功(warning 0件) |
| `cargo test --lib save:: -- --list` | 成功、50件検出(16→50、+34) |
| `cargo test --lib save::` | 成功、50 passed; 0 failed |
| `cargo test --lib -- --list` | 成功、229件検出(195→229、+34) |
| `cargo test --lib` + 8統合テストバイナリ | 成功、229+59=288 passed; 0 failed(headless描画2バイナリは既存の運用慣習通り今回も未実行) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 成功、warning 0件 |
| `cargo build --release --all-targets` | 成功 |
| `cargo fmt --check` | 開始時81件→本ラウンド編集直後92件(新規2ファイル分)→手動整形後81件(§19参照) |
| `git diff --check` | 終了コード0、空白関連エラーなし(LF/CRLF警告のみ、既存dirtyファイル由来) |
| ファイル残留確認 | `find . -iname saves -type d`で該当なし、OS一時ディレクトリも全てDropガードで削除済み |

---

## 19. rustfmtベースラインの開始時・終了時比較

- **rustfmtバージョン**: `rustfmt 1.9.0-stable (8bab26f4f6 2026-07-14)`(作業開始時に記録)
- **作業開始時の`cargo fmt --check`**: 81件のDiff(全てP21-SAVE-002A1完了時点の既存
  ベースラインと一致するファイル群。今回のタスクでは原因を「環境のrustfmtドリフト」と
  断定せず、原因未確定の既知ベースライン拡大として扱った)
- **本ラウンドで新規ファイルを書いた直後**: 92件(新規追加の`src/save/export.rs`・
  `src/save/write.rs`に伴う新規diff 11件分。`src/save/runtime.rs`・`src/save/mod.rs`・
  `src/app/time.rs`・`src/diplomacy/{claims,crisis}.rs`・`src/military/army.rs`への変更は
  最初から0件で、追加のdiffを生まなかった)
- **手動整形後の最終確認**: 81件(開始時と完全に同数。`src/save/export.rs`・
  `src/save/write.rs`に生じた新規diffは全てrustfmtの提案通りに手動で反映して解消した。
  `cargo fmt`コマンド自体は一度も実行していない)
- 既存81件(`src/app/loader.rs`、`src/country/country_ai.rs`、`src/map/division_render.rs`、
  `src/map/{mod,selection}.rs`、`src/military/{movement,recruitment,supply,tests}.rs`、
  `src/profiling.rs`、`src/ui/{military_panel,peace_panel}.rs`、`src/war/{capitulation,
  frontline,military_ai,peace,tests}.rs`、`tests/{daily_system_integration,
  land_war_combat_peace,profile_workload_correctness}_test.rs`)は、開始時・終了時とも
  `git status`で無変更(コミット済み状態のまま、`division_selection.rs`/`selection.rs`/
  `military_panel.rs`のみP21-004A由来の既存dirty)であることを再確認した。今回のタスクで
  これらへ一切手を加えていない。

---

## 20. 発見した問題

- **rustfmtベースラインFAILが13件(P21-SAVE-002A時点)→81件(P21-SAVE-002A1/002B時点)の
  まま高止まりしている。** 原因未確定(§19参照)。本タスクのスコープ外のため、一括修正・
  `cargo fmt`実行のいずれも行っていない。将来、この81件の原因調査・解消を独立したタスクとして
  切り出すことを推奨する(NEEDS USER DECISION)。
- 上記以外、実装上の新しい設計課題は発見しなかった。`ClaimRegistry`/`CrisisRegistry`への
  `next_id()`アクセサ追加は、既存の`WarRegistry`/`BattleRegistry`/`WarJustificationRegistry`
  と全く同じパターンで自然に収まった。`ArmyRegistry.next_army_number`のような`HashMap`型の
  private フィールドに対する読み取り専用アクセサ(`&HashMap`を返す形)も問題なく機能した。
  `std::fs::rename`によるWindows上での安全な既存ファイル置換も、事前の懸念に反して
  追加のフォールバックなしに機能することを実テストで確認できた。

---

## 21. タスクC「ロード前検証」への移行可否

**READY**

技術的な障害は見つからなかった。`SaveGameV1`を安全に生成・書き込む経路(Resource→DTO変換、
PostUpdateでのSnapshot取得、RONシリアライズ、アトミックなファイル書き込み)は50件のテストで
実証済みであり、実際にRONセーブファイルを安全に生成できる状態にある。タスクC(ロード前の
バージョン検証・参照整合性検証、まだ現在のゲーム状態には一切触れない読み取り専用フェーズ)は、
このセーブファイルを入力として設計を進められる。

`SaveGameV1`の`version`フィールド(P21-SAVE-002Aで確立済み、`#[serde(default)]`なし)は
今回のセーブ経路でも`SAVE_FORMAT_VERSION_V1`が確実に書き込まれることを確認済みであり、
タスクCのバージョン検証はこの値をそのまま利用できる。
