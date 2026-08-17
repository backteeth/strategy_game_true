# P21-SAVE-002F 最終受入監査報告書

日付: 2026-08-14

## 0. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

P21-SAVE-002A〜002Eで実装された単一スロットセーブ／ロード(DTO定義→Resource変換→
安全なファイル書込→読込→検証→原子的適用→ゲーム内UI)を横断監査した結果、**コード上の
機能欠陥は検出されなかった**(本ラウンドで新たに発見した実装バグはゼロ。002E自身が
このラウンド以前に発見・修正済みだった2件の実バグ[PreIndustrial検証・
`handle_load_requests`多重実行]は、いずれも現在のコードで修正済みであることを
再確認した)。唯一未完了なのは、実ウィンドウでの人間による目視操作確認(§9)であり、
これは監査エージェントに画面操作手段が無いため今回も実施できなかった(偽って
実施済みと報告しない)。

本報告書では、監査中に1件の**設計上の既知の限界**(NEEDS FIX相当ではなく、現行スコープ
[単一ビルド内・単一マップレイアウトでのプレイ]では実害がなく、002F/002F1双方の
スコープ外である「起動直後ロード」「マイグレーション」に本質的に属する論点)を発見し、
§8で詳述する。また、002E完了報告書に2件の軽微な数値・表記の誤りを発見し、本報告書内で
訂正する(002E報告書自体は上書きしていない)。

---

## 1. 監査範囲

- 対象: P21-SAVE-002A〜002E(`src/save/`全体、`src/map/camera.rs`のロード関連部分、
  `src/ui/top_bar.rs`のセーブ/ロードボタン、`src/ui/load_confirm.rs`、`src/main.rs`/
  `src/ui/mod.rs`のプラグイン登録、`assets/localization/*.ron`のセーブ/ロード関連14キー)。
- 対象外(本ラウンドで一切変更しない): Productionコード、静的RONアセット、
  `verification_logs/phase-21/p21-save-00{1,2a,2a1,2b,2b1,2c,2d,2e}/`の既存報告書。
- 本ラウンドはコード変更を行わない監査タスクである。実際にコードへ加えた変更は
  **ゼロ**(`git status`は監査開始時点と完全に一致、下記§9参照)。

---

## 2. 要件追跡表(A〜E)

| 区分 | 主要要件 | 判定 | 根拠 |
|---|---|---|---|
| A. 永続化全数監査 | SaveGameV1の全17正規フィールドがResource⇄DTOで1:1対応 | **PASS** | §3の全数対応表 |
| A | Claim/CrisisがV1必須(省略不可) | **PASS** | `dto.rs`にoptionalフィールドなし、`round_trip_preserves_claim_registry`/`round_trip_preserves_crisis_registry` |
| A | versionに`#[serde(default)]`なし | **PASS** | `dto.rs:74`、`deserialize_fails_when_version_field_is_missing` |
| A | GamePaused/カメラ/UI選択状態/AI dirty 4箇所は非保存 | **PASS** | `SaveGameSources`にフィールド自体が存在しない構造的保証 + `save_game_v1_excludes_paused_camera_and_ui_selection_state` |
| A | CountryAiState/MilitaryAiStateの正規データは欠落なし | **PASS** | `SavedCountryAiState`/`SavedMilitaryAiState`が`dirty`以外の全フィールドを保持、`round_trip_preserves_ai_normative_state` |
| A | Bevy Entityを永続IDとして不使用 | **PASS** | `dto.rs`は`bevy`を一切importせず、`Entity`型フィールドは構造的に存在しえない。`grep`で`Entity`を保持するのはui/map配下(描画プロキシ)のみ、simulation registryには皆無 |
| A | 全永続ID/next_id等が往復後も衝突しない | **PASS** | `round_trip_preserves_all_next_id_counters`(Division/Army/War/Battle/Claim/Crisis/Frontlineの7カウンタ全て) |
| A | 新規追加Resourceの漏れなし | **PASS** | §3参照。`economy`/`population`/`logistics`モジュールはBevy `Resource`を持たず(`CountryStockpile`等は`CountryData`の内包フィールド、既に`countries`経由でカバー済み)、35ファイルの全`#[derive(Resource...)]`を再列挙し漏れなしを確認 |
| B. 保存安全性 | 単一スロット・8手順のアトミック書込・失敗時旧ファイル保護・tmp後始末 | **PASS** | `write.rs`実装 + `write_failure_preserves_existing_final_file_and_does_not_panic`等 |
| B | 起動・Playing遷移だけではI/Oしない | **PASS** | `starting_the_game_does_not_create_a_save_file`(E2E) |
| B | UI明示要求時のみ保存 | **PASS** | `handle_save_requests`は`SaveRequestMessage`受信時のみ動作 |
| C. 読込・検証・原子的適用 | version欠落/未対応/壊れたRON/参照不整合を適用前に拒否 | **PASS** | `read.rs`の5種`LoadSaveError`、`missing_version_field_is_rejected_as_deserialize_error`等 |
| C | 検証・Prepare失敗時はWorld無変更 | **PASS** | `apply.rs`のPrepare→Commit二段階設計、`missing_save_file_preserves_current_state`等7テスト |
| C | PreIndustrial例外は開始段階のみに限定 | **PASS** | `validate.rs:311`/`1938`、`unknown_world_stage_is_rejected`(PreIndustrial以外は引き続き拒否) |
| C | commit_loadは実数31件(17+1+13) | **PASS(再確認)** | §5参照。002Dの「29個」は文書誤り、コード自体は既に31個実装済み |
| C | ロード成功後は必ずGamePaused(true) | **PASS** | `apply.rs:340`で`GamePaused(true)`をハードコード(セーブ側のデータに依存しない) |
| D. Runtime・スケジュール | Plugin登録は本番main.rsへ1回のみ | **PASS** | `main.rs`に`SaveGamePlugin`/`LoadGamePlugin`各1回 |
| D | 既存公開APIを迂回しない | **PASS** | `handle_load_requests`は`read_and_validate_save_file`/`apply_validated_save`のみ呼ぶ |
| D | MessageReaderカーソルがフレーム間で永続化 | **PASS** | `handle_load_requests(world, reader_state: &mut SystemState<...>)`(002E末で修正済み、本ラウンドで再確認) |
| D | 1クリックが複数フレームで再実行されない | **PASS** | 同上修正 + `load_only_executes_load_once`/Headless実描画での実ログ確認 |
| D | Save/Load同時要求はLoadのみ実行、失敗時も次フレームへ残さない | **PASS** | `save_and_load_same_frame_executes_load_only`/`failed_simultaneous_load_still_skips_save` |
| D | システム順序に暗黙依存がない | **PASS(注記あり)** | §8-3参照。`handle_save_requests`/`handle_load_requests`間に明示的`.after()`/`.before()`は無いが、両者ともUpdateスケジュールで既に確定したMessageバッファを読むだけなので実行順序に依存しない。`handle_load_requests`は排他的Systemのため他Systemと並列実行され得ない(Bevyの仕様上保証)。機能上のバグではないが、明示的な順序宣言がない点は改善余地として記録 |
| D | Playing以外ではセーブ/ロード非実行 | **PASS** | 両System共`run_if(in_state(GameState::Playing))` |
| E. UI・通知・カメラ | Save直送/Load確認画面/Confirmのみ発行/Cancel無変更 | **PASS** | `src/ui/top_bar.rs`/`src/ui/load_confirm.rs` + 各4テスト |
| E | ダイアログは成功/失敗/Cancel後に閉じる | **PASS** | `handle_load_confirm_button`が同期的に閉じる設計、Headlessテストで実描画確認 |
| E | UIクリックが地図操作へ非漏出 | **PASS** | 既存汎用ガード([`Interaction`]全走査)を再利用、新規コード不要 |
| E | JA/EN 14キー・プレースホルダ一致 | **PASS(再確認)** | 本ラウンドで両ファイルから該当14キーを再grep、14/14一致 |
| E | ハードコード文字列なし | **PASS** | `p20_009_hardcoded_string_scan_test`4/4 pass |
| E | 通知は1操作正確に1件 | **PASS** | §8-2で言及の修正後、複数テスト+Headless実ログで確認 |
| E | ロード成功通知はNotificationHistory初期化後 | **PASS** | `load_success_notification_appears_after_notification_history_is_cleared` |
| E | カメラは成功時のみ位置/ズーム/DragState初期化、spawn/despawnなし、0/複数体でもpanicしない | **PASS** | `camera.rs`6テスト全pass、Headless実描画でも視覚確認済み |

---

## 3. 永続化対象 全数対応表

| SaveGameV1フィールド | 元Resource | export.rs変換 | DTO型 | read/deserialize | validation | apply先 | テスト証拠 |
|---|---|---|---|---|---|---|---|
| `version` | (定数) | `SAVE_FORMAT_VERSION_V1`固定 | `u32` | ヘッダ先読み+完全Deserialize | `version != V1`→`InvalidRange` | 適用対象外(検証のみ) | `deserialize_fails_when_version_field_is_missing`, `serialized_ron_contains_version_field` |
| `date` | `GameDate` | year/month/day/accumulator() | `SavedGameDate` | 通常Deserialize | `date.day/month`範囲, `accumulator∈[0,1)` | `world.insert_resource(prepared.game_date)` | `round_trip_preserves_world_and_progress_fields` |
| `game_speed` | `GameSpeed(u8)` | `.0` | `u8` | 同上 | `1..=4`範囲チェック | 同上 | `game_speed_zero_is_rejected`/`game_speed_above_four_is_rejected` |
| `player_country` | `PlayerCountry` | `.0` | `Option<CountryId>` | 同上 | (countries内に存在するかは`validate_world_state`で別途) | 同上 | 同上 |
| `world_civilization` | `WorldCivilizationState`(可変部のみ) | current_stage/milestone_countries/last_advanced_date(`stage_definitions`除外) | `SavedWorldCivilizationState` | 同上 | current_stage(PreIndustrial例外あり)/milestone_countriesキー | `stage_definitions`は現在Worldの値を保持したまま`current_stage`等だけ差し替え | `round_trip_preserves_world_and_progress_fields`, PreIndustrial系2テスト |
| `countries` | `CountryRegistry.countries` | `.clone()` | `Vec<CountryData>` | 同上 | 空チェック/ID重複/capital存在等(`validate_countries`) | `world.insert_resource(prepared.country_registry)` | `round_trip_preserves_country_and_state_representative_fields` |
| `states` | `StateRegistry.states` | `.clone()`(`index_map`除外=静的) | `Vec<StateData>` | 同上 | owner/controller参照/occupation範囲(`validate_states`) | `world.insert_resource(prepared.state_registry)` | 同上 |
| `diplomacy` | `DiplomacyRegistry` | `.relations.clone()` | `SavedDiplomacyRegistry` | 同上 | `validate_diplomacy` | `world.insert_resource(prepared.diplomacy_registry)` | dto/apply往復テスト |
| `war_justifications` | `WarJustificationRegistry` | justifications+next_id() | `SavedWarJustificationRegistry` | 同上 | `validate_war_justifications` | 同上 | 同上 |
| `wars` | `WarRegistry` | wars+next_id() | `SavedWarRegistry` | 同上 | `validate_wars` | 同上 | `round_trip_preserves_all_next_id_counters` |
| `claims` | `ClaimRegistry` | claims+next_id() | `SavedClaimRegistry` | 同上 | `validate_claims_and_crises` | 同上 | `round_trip_preserves_claim_registry` |
| `crises` | `CrisisRegistry` | crises+next_id() | `SavedCrisisRegistry` | 同上 | 同上 | 同上 | `round_trip_preserves_crisis_registry` |
| `country_ai` | `CountryAiRegistry`(dirty除外) | `ai_states`を`SavedCountryAiState`へ変換(dirty破棄) | `SavedCountryAiRegistry` | 同上 | `validate_ai` | 適用時dirty=true強制(4箇所) | `round_trip_preserves_ai_normative_state`, `all_four_ai_dirty_flags_are_true_after_apply` |
| `military_ai` | `MilitaryAiRegistry`(dirty除外) | 同上 | `SavedMilitaryAiRegistry` | 同上 | 同上 | 同上 | 同上 |
| `military` | `MilitaryRegistry`(`.divisions`のみ、`.definitions`は静的除外) | divisions+next_division_id() | `SavedMilitaryRegistry` | 同上 | `validate_divisions` | `world.insert_resource(prepared.military_registry)`(`.definitions`は現在値を保持したまま構築) | `round_trip_preserves_division_in_progress_movement_fields` |
| `battles` | `BattleRegistry` | battles+next_id() | `SavedBattleRegistry` | 同上 | `validate_battles` | 同上 | dto往復テスト |
| `armies` | `ArmyRegistry` | armies+division_army_map+next_id()+next_army_number | `SavedArmyRegistry` | 同上 | `validate_armies` | 同上 | `round_trip_preserves_army_membership` |
| `frontlines` | `FrontlineRegistry` | frontlines+plans(タプルキー)+next_frontline_id+division_frontline_map+frontline_generated_movements | `SavedFrontlineRegistry` | 同上 | `validate_frontlines` | 同上 | `round_trip_preserves_frontline_registry_including_tuple_keyed_plans` |

**保存対象外(意図的、構造的に検証済み)**: `GamePaused`、`GameCamera`Entityの`Transform`、
`CameraDragState`、`SelectedDivision`/`SelectedArmy`/`SelectedState`、`ActivePanel`および
4パネルState、`NotificationHistory`、`PendingAiWarDeclarations`(以上13個は commit_load 側で
`Default`初期化のみ)、`CountryAiRegistry.dirty`/`CountryAiState.dirty`/
`MilitaryAiRegistry.dirty`/`MilitaryAiState.dirty`(4箇所)、`BuildingRegistry.definitions`/
`TechnologyRegistry.{definitions,sorted_ids}`/`MilitaryRegistry.definitions`/
`WorldCivilizationState.stage_definitions`/`StateRegistry.index_map`(静的マスター、
起動時にRONから再構築)、`TranslationCatalog`/`CurrentLocale`(言語設定、ワールド状態ではない)、
`CameraSettings`/`FrontlineRenderSettings`/`SetTimings`(操作設定・デバッグ用、ゲーム状態ではない)、
`PreviewCountry`(国選択画面専用、Playing中は無関係)、`Save*`/`Load*`系メタResource
(`LastSaveOutcome`/`SaveExecutionCount`/`SaveFileConfig`/`LastLoadOutcome`/`LoadExecutionCount`/
`LoadConfirmState`、セーブ/ロード操作自体のメタ状態でありゲーム世界の一部ではない)。

**新規Resource漏れの確認**: `src/`全体で`#[derive(...Resource...)]`を持つ型を再列挙(35ファイル)し、
上記のいずれか(正規保存対象17/一時初期化13/AI dirty 4/静的5/設定・メタ・UI専用)に
全て分類できることを確認した。分類不能なResourceは発見されなかった。`economy`/
`population`/`logistics`各モジュールはBevy `Resource`を一切定義しておらず(経済備蓄
`CountryStockpile`は`CountryData.stockpile`として既存の`countries`経由で保存済み)、
見落としのリスクは低いと判断した。

---

## 4. コマンドと実測結果(本ラウンドで実行)

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | **PASS**(14.53s) |
| `cargo test --lib` | **389 passed / 0 failed** |
| 安全な統合テスト9本(`daily_system_integration_test`, `diplomacy_tests`, `economy_tests`, `land_war_combat_peace_test`, `p20_009_hardcoded_string_scan_test`, `p20_009_localization_resource_test`, `p21_save_002e_end_to_end_test`, `profile_workload_correctness_test`, `research_and_politics_tests`) | **62 passed / 0 failed**(6+5+14+4+4+8+3+9+9) |
| `cargo test --test p21_save_002e_end_to_end_test`(個別実行、上記に含む) | **3 passed / 0 failed** |
| `p21_save_002e_headless_render_test`(§8-1参照、本ラウンドでは`--no-run`のみ) | コンパイル成功のみ確認、**実行はスキップ**(既存証跡PNG保護のため) |
| `cargo clippy --all-targets --all-features -- -D warnings` | **PASS**(warning 0) |
| `cargo build --release --all-targets` | **PASS**(13.08s、増分ビルド) |
| `cargo fmt --check`(読み取りのみ) | 既知ベースライン**74 diff hunks**、002E完了時点と完全一致(§7参照) |
| `git diff --check` | **PASS**(LF/CRLF警告のみ、実際の空白エラーなし) |
| 一時ファイル/プロセス残留確認 | `saves/`ディレクトリなし、`strategy_game_p21_save_*`一時ディレクトリなし |

**合計テスト数**: `cargo test --lib` 389 + 統合テスト63(9本62件+headless-render 1件、
下記§8-1参照)= **452件**(002E完了報告書の数値と一致、増減なし。本ラウンドはコード
変更を一切行っていないため、これは想定通りの結果)。

### 4-1. Headless実描画テストを本ラウンドで実行しなかった理由

`p21_save_002e_headless_render_test.rs`は`verification_logs/phase-21/p21-save-002e/screenshots/`
の固定パスへ無条件で`save_png`する設計(P20-007以来の既存パターンを踏襲)であり、
再実行すると002Eが生成した5枚の証跡PNGを**上書きしてしまう**。本タスクの指示
「既存証跡を変更しない方法が確保できる場合だけ実行する」に従い、`cargo test --no-run`で
コンパイルが現在のコードに対して通ることだけを確認し(結果: 成功)、実行(証跡再生成)は
見送った。既存の5枚のPNG(§6参照)は002E完了時点のものをそのまま監査対象とした。

### 4-2. システム順序についての補足

`handle_save_requests`(`SaveGamePlugin`)と`handle_load_requests`(`LoadGamePlugin`)は
どちらも`PostUpdate`に登録されているが、相互に明示的な`.after()`/`.before()`を持たない。
検証の結果、これは機能的なバグではないと判断した:
1. 両者が読む`SaveRequestMessage`/`LoadRequestMessage`は、いずれもその1つ前の`Update`
   スケジュールで(UIボタンハンドラにより)書き込み済みであり、`PostUpdate`開始時点で
   メッセージバッファは既に確定している。どちらのSystemが先に走っても、読み取る内容は
   同じ。
2. `handle_load_requests`は排他的System(`&mut World`)であり、Bevyの仕様上、他のどの
   Systemとも並列実行されない(スケジューラが自動的に直列化する)。
3. 実際に`save_and_load_same_frame_executes_load_only`等のテストが継続してpassしている。

改善の余地としては、暗黙の前提を`.after()`/`.before()`で明示宣言する、またはコード
コメントで「なぜ順序に依存しないか」を明記することが考えられるが、**現状は機能上の
欠陥ではない**ため、NEEDS FIXとして002F1へ持ち越す必要はないと判断した。

---

## 5. §3 002Dの`commit_load`再確認(継続監査)

002Eで確認した「実数31件(17正規+1 GamePaused+13一時/UI)」を本ラウンドでも再カウントし、
`src/save/apply.rs::commit_load`が現在も変更されておらず(本ラウンドはコード変更ゼロ)、
`world.insert_resource`呼び出しが正確に31件であることを再確認した。002Dの完了報告書
自体は引き続き上書きしない。

13個の一時/UI Resourceが`prepare_load`(要求)・`commit_load`(適用)の両方に存在することは、
`apply.rs`の`PreparedLoadGameV1`構造体定義と`commit_load`本体を突き合わせて確認した
(§3の全数対応表「保存対象外」欄に列挙した13個と完全一致)。

---

## 6. End-to-End証拠の対応付け

| 要求されたシナリオ | 自動assert/スクリーンショットで確認可能 | 人間の目視が必要 |
|---|---|---|
| 起動だけではセーブファイルを作らない | ✅ `starting_the_game_does_not_create_a_save_file`(E2E)、Headlessテスト冒頭のassert | — |
| Save→状態Bへ変更→Loadで状態Aを復元 | ✅ `save_change_load_restores_state_a_new_ids_do_not_collide_and_resave_works` | — |
| 日付・国庫・師団所在地 | ✅ 同上(日付/国庫/師団の`current_state`を直接assert) | — |
| 選択状態(SelectedArmy/SelectedState) | ✅ 同上(ロード後に`None`/空へリセットされることをassert) | — |
| ID採番(Division/Army衝突なし) | ✅ 同上(新規発行IDが既存セットと非交差であることをassert) | — |
| ロード後の再セーブ | ✅ 同上 + Headlessテストの最終ステップ | — |
| 失敗ロードで実ゲーム状態を維持 | ✅ `failed_load_preserves_the_real_running_game_state` | — |
| Load確認ダイアログの表示 | ✅ Headlessテストのピクセル差分+実PNG(`03_load_confirm_dialog_open.png`) | — |
| Cancelで無変更 | ✅ Headlessテストのassert+実PNG(`04_after_cancel_dialog_closed.png`) | — |
| Confirmで実ロード実行 | ✅ Headlessテストのassert+実PNG(`05_after_load_success.png`) | — |
| GamePaused(true)化 | ✅ Headlessテスト`assert!(...GamePaused>().0)` | — |
| カメラの視覚的リセット | ✅ Headlessテストで`Transform::IDENTITY`への復帰をResource経由でassert(ピクセル差分でも確認) | — |
| 通知1件ずつ(Save成功/Load成功) | ✅ Headlessテストの実ログ("[Notification] セーブが完了しました。"/"ロードが完了しました。"各1回) | — |
| **州色のロード後同期** | ❌ 自動テストなし(`update_state_colors_on_controller_change`はis_changed()ゲートのみ確認済み、実際に色が変わる様子はテスト対象外) | **要目視** |
| **師団スプライトのロード後同期** | ❌ Headlessテストは師団の位置変更シナリオを含まない(カメラ・日付のみ変更) | **要目視** |
| **Army(編成)表示のロード後同期** | ❌ 同上 | **要目視** |
| **前線(Frontline)表示のロード後同期** | ❌ 同上 | **要目視** |
| トップバー表示(日付/一時停止)の即時同期 | 🟡 部分的(Headlessテストは全体ピクセル差分のみ、日付テキストの内容自体はピクセル単位で読み取っていない) | **要目視(文字内容の確認)** |
| ウィンドウ終了後のプロセス残留 | ❌ 実施不可(実ウィンドウ操作手段なし) | **要目視** |

既存5枚のPNG(`verification_logs/phase-21/p21-save-002e/screenshots/`)は本ラウンドで
再確認(目視)し、上書きしていない。州色/師団/Army/前線の視覚同期については、
E2Eテスト・Headlessテストいずれも意図的に検証範囲外(Headlessテストのシナリオが
カメラ・日付のみを変更対象としたため)であり、**このラウンドで新たに判明した
自動化ギャップ**として記録する(機能欠陥ではなく、テストカバレッジの限界)。

---

## 7. 002E完了報告書の数値訂正(002E報告書自体は変更していない)

1. **§12/§15の統合テスト本数表記の不一致**: §12は統合テスト内訳として
   `daily_system_integration_test`(6)+`diplomacy_tests`(5)+`economy_tests`(14)+
   `land_war_combat_peace_test`(4)+`p20_009_hardcoded_string_scan_test`(4)+
   `p20_009_localization_resource_test`(8)+`p21_save_002e_end_to_end_test`(3)+
   `p21_save_002e_headless_render_test`(1)+`profile_workload_correctness_test`(9)+
   `research_and_politics_tests`(9)の**10本**を正しく列挙し合計63件としていたが、
   §15の検証結果表では「安全な統合テスト**9本**(headless-render PNG2本を除く)」と
   記載しており、本数が1本(`p21_save_002e_headless_render_test`)分ズレている
   (合計63件という数値自体は正しい)。正しくは「9本を本ラウンドの一括コマンドで実行
   + `p21_save_002e_headless_render_test`は個別に実行済みの計10本、合計63件」であり、
   `p21_save_002e_headless_render_test`は他の2本のheadless-renderテスト
   (`ui_headless_render_test`/`p20_009_localization_headless_render_test`、既存の
   固定PNGを上書きするため除外)とは異なり、専用の新規ディレクトリへ出力するため
   除外対象ではない。本ラウンドの§4の表はこの点を正しく「9本」(本ラウンドの一括
   コマンド分)と「headless-render個別1本」を分けて記載している。
2. **rustfmtベースラインの表記**: 002E報告書§16は事後ベースラインを「74 diff hunks」と
   正しく記載しており、本ラウンドの再計測(§4)でも同じ74件・同じファイル/行番号の
   完全一致を確認した。誤りではなく、**正しさの再確認**として記録する。
3. 上記以外の002E報告書の数値(テスト数411→452/+41、変更ファイル一覧、§9のPreIndustrial
   /handle_load_requestsバグの記述等)は、本ラウンドで実コードと突き合わせた結果、
   全て正確であることを確認した。

---

## 8. 発見事項と重大度

| # | 内容 | 重大度 | 分類 | 対応 |
|---|---|---|---|---|
| 1 | `check_static_master_compatibility`(002D)・`validate_save_game_v1`(002C)いずれも、
セーブファイルの`countries`/`states`集合が**現在起動中のWorldの`CountryRegistry`/
`StateRegistry`と一致するか**を検証しない(内部参照整合性[save自身の中でのID相互参照]は
検証されるが、外部[起動中のWorld]との集合比較は行われない)。現行スコープ(単一ビルド・
単一マップレイアウトでの同一セッション内プレイ)では、セーブ・ロード双方が同じ
`assets/data/countries.ron`/`states.ron`から起動するため実害はない。しかし、将来
マップ(`P21-MAP-001`のような州追加)やcountries.ronの変更後に**古いセーブを新しい
ビルドへロード**した場合、この不一致は検出されず、`commit_load`によって新World全体が
古い州/国家集合へ**無条件に置き換わる**(部分的に見えなくなる州や、他の静的データ
[`world_stages.ron`の`required_country_count`等]との不整合を招きうる)。 | **低(現行スコープ外)** | 設計上の既知の限界。マイグレーション/バージョン互換性の領域であり、002F/002F1双方の
明示的スコープ外(「マイグレーションを実装しない」)。 | **修正不要(002F1へ持ち越さない)**。将来「起動直後ロード」または別の
マイグレーションタスクで対応を検討する候補として記録するに留める。 |
| 2 | `handle_save_requests`/`handle_load_requests`間に明示的なSystem順序宣言がない
(§4-2で詳述、機能上のバグではないと確認済み)。 | **情報(Info)** | コードの自己文書化性の改善余地。 | 修正不要。将来のリファクタリングで`.after()`/コメント追加を検討してもよい。 |
| 3 | 002E報告書§12/§15の統合テスト本数表記の不一致(§7参照)。 | **軽微(報告書表記のみ)** | ドキュメント上の誤記。 | 本報告書で訂正済み(002E報告書自体は不変)。 |
| 4 | 州色/師団/Army/前線のロード後視覚同期について、自動テスト(pixel-level)が
存在しない(§6参照)。 | **低(テストカバレッジの限界、機能欠陥ではない)** | 既存のHeadlessテストが日付・カメラのみを変更対象としているため。 | 修正不要。将来Headlessテストを拡張する場合の候補として記録。 |

**本ラウンドで新たに発見した実装バグ: 0件。** 002Eが同ラウンド内で発見・修正した
2件(PreIndustrial検証の常時失敗、`handle_load_requests`の多重実行)は、いずれも
本ラウンドの監査で現在のコードに正しく反映されていることを再確認した(§2要件追跡表
C区分、§8-1〜4のコード再読で確認)。

---

## 9. git status / diff(監査開始時点、本ラウンド終了時点で完全一致)

```
 M strategy_game/assets/localization/en-US.ron
 M strategy_game/assets/localization/ja-JP.ron
 M strategy_game/src/app/time.rs
 M strategy_game/src/diplomacy/claims.rs
 M strategy_game/src/diplomacy/crisis.rs
 M strategy_game/src/lib.rs
 M strategy_game/src/main.rs
 M strategy_game/src/map/camera.rs
 M strategy_game/src/map/division_selection.rs
 M strategy_game/src/map/rendering.rs
 M strategy_game/src/map/selection.rs
 M strategy_game/src/military/army.rs
 M strategy_game/src/military/battle.rs
 M strategy_game/src/military/data.rs
 M strategy_game/src/military/mod.rs
 M strategy_game/src/ui/military_panel.rs
 M strategy_game/src/ui/mod.rs
 M strategy_game/src/ui/top_bar.rs
 M strategy_game/src/war/data.rs
 M strategy_game/src/war/justification.rs
 M strategy_game/tests/p20_009_hardcoded_string_scan_test.rs
?? strategy_game/src/save/
?? strategy_game/src/ui/load_confirm.rs
?? strategy_game/tests/p21_save_002e_end_to_end_test.rs
?? strategy_game/tests/p21_save_002e_headless_render_test.rs
?? strategy_game/verification_logs/phase-21/(各既存報告書)
```

本ラウンド開始時点(§監査冒頭で記録)と終了時点で**完全に一致**(本ラウンドは
`verification_logs/phase-21/p21-save-002f/`の新規作成以外、一切のファイルを
変更していない)。`cargo fmt`/`rustfmt`は一度も書込モードで実行していない
(`--check`のみ)。`main.rs`/`lib.rs`/`mod.rs`をrustfmtへ渡す操作も行っていない。

---

## 10. 人間による手動確認チェックリスト(未実施、次回`cargo run`時に確認を推奨)

監査エージェントには実ウィンドウ操作の手段がないため、以下は**未実施**として報告する
(実施したと偽らない)。

1. `cargo run`で起動し、Playing前に`saves/`が作られないこと。
2. Save/Loadボタンが正常表示されること。
3. Save後に日付・軍・前線・カメラを変更すること。
4. Load→Cancelで状態が変わらないこと。
5. Load→Confirmで状態Aへ戻り、一時停止すること。
6. **州色・師団・Army・前線・トップバーが即座に同期すること**(§6で自動化ギャップと
   して特定した項目、特に重点確認を推奨)。
7. カメラが既定位置・ズームへ戻ること。
8. 成功・失敗通知が各1回だけ表示されること。
9. 欠落・破損・version=2ファイルでもクラッシュ・状態変更しないこと。
10. ウィンドウ終了後にプロセスが残らないこと。

---

## 11. P21-SAVE-002F1への移行判定

**不要。** 本ラウンドで新たに発見した実装バグはゼロであり、§8の発見事項は
いずれも「現行スコープ外の設計上の既知の限界」または「テストカバレッジの限界」
であって、緊急の修正を要する欠陥ではない。P21-SAVE-002F1(コード修正ラウンド)を
起票する必要はないと判断する。

## 12. 次アクション

1. 可能なタイミングで、ユーザー自身による§10の実ウィンドウ手動確認(特に項目6)を
   実施し、結果を報告いただくことを推奨する。全項目pass後、本報告書の判定を
   「COMPLETE」へ更新できる。
2. その後、別タスクとして「起動直後ロード」を実装する(§8の発見#1で記録した
   州/国家集合の互換性検証の欠如は、このタスクの設計時に考慮することを推奨する)。
3. 「起動直後ロード」の受入後、P21-005へ戻る。

複数スロット・オートセーブ・クイックセーブ・マイグレーションは、本ラウンド・
次回「起動直後ロード」タスクいずれにも含まれない。
