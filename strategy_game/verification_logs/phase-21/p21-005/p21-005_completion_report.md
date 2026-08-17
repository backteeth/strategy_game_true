# P21-005 完了報告書: Army単位の前線設定・解除・表示

## 1. 最終判定

**COMPLETE**

Army(編成)の前線への設定・解除・表示・保存が実装され、全自動検証(lib 465件 + 安全な統合テスト71件 = 536件、clippy 0警告、release build成功、実7か国28州データでのE2E往復テスト含む)がグリーン。割当だけでは一切の自動移動・戦闘が発生しないことを直接検証済み。**2026-08-14、ユーザーによる実`cargo run`でのGUI手動確認: セクション22記載の15項目すべてPASS、発見された不具合なし。** これにより判定をCOMPLETE WITH MANUAL VERIFICATION PENDINGからCOMPLETEへ更新した。

## 2. 事前監査結果

実装前に以下を確認した(詳細はセッション内の読み取り調査、コード変更なし)。

### Frontline側
- `Frontline`/`FrontlinePlan`/`FrontlineRegistry`は`src/war/frontline.rs`に実在。フィールドは`frontlines`/`plans`(key: `(FrontlineId, CountryId)`)/`next_frontline_id`/`division_frontline_map`/`frontline_generated_movements`の5つ(旧資料の想定と一致)。
- **`army_frontline_map`は事前監査時点で存在しなかった**(過去資料の想定通り、Division単位のみ)。よって新設が必要と判断。
- `assign_division`/`unassign_division`/`sanitize_references`/`remove_frontline`/`update_all_frontlines`/`process_daily_frontline_plans`/`handle_daily_frontline_plans`は全てDivision単位で動作しており、Army(編成)という概念への参照は一切なかった。
- `frontline_render.rs`の`update_frontline_overlay`は前線の自国側/敵側/攻勢目標を「州ごとの矩形スプライト」として表示しており、線ではなく面(=クリック判定に転用しやすい)である。`FrontlineOverlayVisual`マーカーはFrontlineIdを保持していなかった。

### Army側
- P21-004の`Army`/`ArmyRegistry`(`src/military/army.rs`)は複数師団の永続的グループ化のみを扱い、前線参照フィールドは皆無。`ArmyRegistry::sanitize_references`は日次(`handle_daily_army_maintenance`, `MilitaryAction`セット)に加え、UI操作(解散・除外)でも即座にArmyを消滅させうる(空編成の自動解散)。
- `military_panel.rs`の`ArmyCommand`は`Create`/`AddSelection`/`RemoveSelection`/`Disband`の4種のみ。前線関連のコマンドは存在しなかった。

### 戦争・入力
- `DailySimulationSet`順序: `TimeUpdate → Economy → Research → Diplomacy → CountryAi → WarPreparation → MilitaryAi → FrontlineOrders → MilitaryAction → WarResolution → UiUpdate`。
- War終了(講和・降伏)は`war::peace::execute_peace_settlement`一箇所のみが`War.status`を書き換え、必ず`FrontlineRegistry::remove_frontline`を呼ぶ(捕獲・白紙講和・UI講和いずれも同じ経路)。
- マップクリックは`map::selection::handle_state_click`(左クリック, `just_released`)、`map::division_selection::handle_division_selection`(左クリック/ドラッグ)、`map::division_selection::handle_movement_order`(右クリック, `just_pressed`)が独立にButtonInputを読む設計。UI操作中は`Query<&Interaction>`の`Hovered`/`Pressed`走査で共通にガードされている。

### セーブ
- `SavedFrontlineRegistry`(`src/save/dto.rs`)は当時5フィールドで`FrontlineRegistry`と1:1。`FrontlinePlan`型自体を再利用しているため、型定義へフィールド追加すれば`SavedFrontlineRegistry.plans`にも自動的に反映される構造だった。
- `version`は`#[serde(default)]`なし(意図的、欠落を許さない)。

**結論**: `army_frontline_map`という重複マップは事前監査の時点で存在しなかったため、新設した。既存の`division_frontline_map`と全く同じ設計パターン(逆引きマップ+`assigned_army_ids`)を踏襲することで、コードベース内で既に検証済みのイディオムを再利用した。

## 3. 採用したArmy↔Frontlineデータモデル

`war::frontline::FrontlineRegistry`へ以下を追加:

```rust
pub struct FrontlineRegistry {
    // ...既存5フィールド...
    #[serde(default)]
    pub army_frontline_map: HashMap<ArmyId, FrontlineId>,
}

pub struct FrontlinePlan {
    // ...既存フィールド...
    #[serde(default)]
    pub assigned_army_ids: Vec<ArmyId>,
}
```

新メソッド: `assign_army`/`unassign_army`/`frontline_for_army`/`assignable_frontlines_for_army`/`sanitize_army_references`。`remove_frontline`は`assigned_army_ids`もあわせて清掃するよう拡張。

不変条件の担保:
- 1 Army = 最大1 Frontline: `assign_army`が旧割当を必ず先に外してから新規登録する単一操作。
- 1 Frontline = 複数Army可: `Vec<ArmyId>`で表現、重複挿入は`contains`チェックで防止。
- owner一致: `assign_army`/`unassign_army`双方が`army.owner == country_id`を検証(所有者不問で選択されうるUI層とは独立に、実行系側で必ず再検証)。
- HashMap反復順非依存: `assigned_army_ids`は挿入のたびに`sort_by_key(|id| id.0)`、`assignable_frontlines_for_army`も`FrontlineId`昇順にソートして返す。

## 4. 正規情報を一つに保った方法

`army_frontline_map`(逆引き)と`plans[..].assigned_army_ids`(正引き)の2箇所に情報が存在するが、これは既存の`division_frontline_map`/`assigned_division_ids`と全く同じペアであり、**単一の変更経路(`assign_army`/`unassign_army`/`remove_frontline`/`sanitize_army_references`)からのみ同時更新される**。UI Resourceだけに情報を保持することはなく、Army配下Divisionの複製も一切行わない。`Army`構造体(`military::army.rs`)自体は無変更。

## 5. 前線設定・解除UI

`ui::military_panel.rs`に新規追加:
- `ArmyFrontlineCommand`(`Assign`/`Unassign`)+`ArmyFrontlineCommandButton`コンポーネント(既存`ArmyCommandButton`とは別型。同一エンティティ集合を2つのSystemが取り合わないための設計、`update_army_ui`の既存exhaustive matchも無変更のまま)。
- 「前線を設定」ボタン: `execute_army_frontline_assign_toggle`が`map::frontline_selection::FrontlineSelectMode`をトグル(選択モードへ入る/出るだけ、割当は行わない)。
- 「前線を解除」ボタン: `execute_army_frontline_unassign`が`FrontlineRegistry::unassign_army`を直接呼ぶ即時操作。成功時に「解除成功」通知。
- `ArmyFrontlineStatusText`: 「現在の割当前線: 対{enemy国名}」/「前線選択中...」/「有効な前線がない」/「編成を選択してください」/「なし」を状況に応じ表示。
- 未選択・他国Armyでは`evaluate_army_frontline_assign_feasibility`がボタンを無効色にする(実行自体も`assign_army`/`unassign_army`の所有者検証で二重に守られる)。

## 6. 選択モードと入力競合

新規`map::frontline_selection.rs`:
- `FrontlineSelectMode{ army_id: Option<ArmyId> }`(map層が所有。ui層は依存不可のため)。
- `cancel_frontline_select_mode_on_context_change`: Escape/右クリック/対象Army選択変更/Army消滅/割当可能前線の消滅で自動解除。
- `handle_frontline_select_click`: モード中の左クリックのみを処理、有効なFrontline(クリック州が対象Armyの自国側前線地域内)なら`assign_army`を実行しモード終了、無効なら状態変更なし。同一州に複数候補がある場合は`FrontlineId`昇順で決定的に1つを選ぶ。
- 入力競合防止: `handle_division_selection`/`handle_state_click`/`handle_movement_order`の先頭に`FrontlineSelectMode`ガードを追加。`FrontlineSelectionPlugin`側で`cancel_frontline_select_mode_on_context_change`/`handle_frontline_select_click`を`.after(handle_division_selection, handle_movement_order, handle_state_click)`に順序固定し、同一フレーム内で「ガード判定→前線処理」の順を保証(前線操作が先にモードを解除しても、既にガード判定を終えた通常入力処理には影響しない)。
- 「Armyパネルを閉じたら解除」だけはui層(`military_panel::cancel_frontline_select_mode_on_panel_close`)が担当(map→ui依存を作らないための分離)。
- カメラドラッグ(右/中ボタン押しっぱなし)は既存のまま無変更(前線選択は左クリックのみを使うため構造的に非競合)。

## 7. 自動移動を発生させない保証

`assigned_army_ids`/`army_frontline_map`は`process_defensive_plan`/`process_offensive_plan`/`process_stopped_plan`/`process_daily_frontline_plans`のいずれからも一切参照されない(コード上の理由: これらの関数は`plan.assigned_division_ids`のみを読む)。よって設計上、Army割当だけでは`frontline_generated_movements`への追加も移動Message発行も発生しえない。

直接検証:
- `test_army_assignment_does_not_mutate_any_division_fields`(war/frontline.rs): 割当前後でDivisionの`current_state`/`destination`/`target_state`/`current_path`/`movement_progress`/`status`/`combat_id`が完全一致、`frontline_generated_movements`に追加されないことを確認。解除でも同様。
- `test_army_assignment_alone_does_not_trigger_frontline_orders_automation`: Defendスタンス設定後に`process_daily_frontline_plans`を実行してもDivisionはIdleのまま。
- E2Eテスト(`tests/p21_005_army_frontline_e2e_test.rs`)でも実データ上で同条件を再確認、さらに`advance_one_day`で実際の日次SystemSetを1回通過させても不動であることを確認。

## 8. 描画・強調表示

`map::frontline_render.rs`の`update_frontline_overlay`を拡張(既存の色・スタイルは変更せず、追加のみ):
- 選択モード中: 割当可能な全Frontlineの自国側地域に半透明白の強調オーバーレイ。
- hover中の州はさらに強調(不透明度を上げる)。
- 選択中Armyが割当済みなら、モードの有無に関わらず紫系(Army UIの既存配色系統)の識別オーバーレイを常時表示。
- 既存同様、毎フレーム全`FrontlineOverlayVisual`をdespawnしてから再構築するため、Entity数はフレームごとに増えない。Frontline削除後は次フレームで自動的に消える(直接テストで確認)。

## 9. Army/Frontline/War削除時の清掃

- **Army削除/自動解散**: `sync_army_frontline_references`(war/frontline.rs, war::mod::WarPluginへ登録)が`ArmyRegistry.is_changed()`をガードに毎フレーム`sanitize_army_references`を呼ぶ(`map::division_selection::prune_selected_division`と同じ確立済みイディオム)。UI操作による即時解散にも日次待ちせず追従。
- **Frontline削除**: `remove_frontline`が該当Planごと`assigned_army_ids`を読み取り`army_frontline_map`からも同時に除去するよう拡張。
- **War終了**: `execute_peace_settlement`(唯一の終了経路)が既に`remove_frontline`を呼ぶため、Army割当も自動的に清掃される(新規呼び出し追加は不要、既存経路の拡張のみで完結)。

## 10. セーブ互換性

- `SavedFrontlineRegistry`(`src/save/dto.rs`)へ`#[serde(default)] pub army_frontline_map`を追加。`FrontlinePlan`は型再利用のため`assigned_army_ids`はDTO側の追加フィールド不要。
- `export.rs`: `army_frontline_map.clone()`を追加するのみ。
- `apply.rs`: `FrontlineRegistry`構造体リテラルへ`army_frontline_map: save.frontlines.army_frontline_map`を追加。
- `validate.rs`: `assigned_army_ids`(dangling ArmyId・owner不一致・複数Frontline二重登録)と`army_frontline_map`(dangling参照・逆引き整合性)を、既存のDivision版と同じ検証パターンで追加。
- 旧形式(フィールド自体が存在しないRON)からの読込を、実際にシリアライズ結果から対象フィールド文字列を除去して確認するテストで検証(`old_format_ron_without_army_frontline_fields_loads_as_empty`)。

## 11. SaveGameV1トップレベル不変の確認

`SaveGameV1`構造体定義(`src/save/dto.rs`)を変更していないことを確認: フィールドは`version`/`date`/`game_speed`/`player_country`/`world_civilization`/`countries`/`states`/`diplomacy`/`war_justifications`/`wars`/`claims`/`crises`/`country_ai`/`military_ai`/`military`/`battles`/`armies`/`frontlines`の18個(トップレベル)のまま。`version`のserde(default)も追加していない。`army_frontline_map`は`frontlines`フィールド(`SavedFrontlineRegistry`型)の**内部**フィールドであり、トップレベル構造には影響しない。E2Eテストで`resaved.version == 1`を直接確認済み。

## 12. ローカライゼーション

10キーをJA/EN両方へ追加(`assets/localization/{en-US,ja-JP}.ron`):
`army_frontline_assign_button`/`army_frontline_unassign_button`/`army_frontline_status_none_selected`/`army_frontline_status_selecting`/`army_frontline_status_assigned`/`army_frontline_status_no_assignable`/`army_frontline_status_unassigned`/`army_frontline_assign_success`/`army_frontline_unassign_success`/`army_frontline_invalid_click`。

`tests/p20_009_localization_resource_test.rs`(キー集合一致・プレースホルダ一致・重複無し・空値無し)、`tests/p20_009_hardcoded_string_scan_test.rs`(ハードコード検出、`military_panel.rs`は既存の走査対象)いずれも green。表示文字列のRustコード直書きは無し(全て`t()`/`tf()`経由)。

## 13. 変更ファイル一覧

**実装(既存ファイル編集)**
- `src/war/frontline.rs` — データモデル・ロジック・21件の単体テスト追加
- `src/war/mod.rs` — `sync_army_frontline_references`システム登録
- `src/map/mod.rs` — `FrontlineSelectionPlugin`登録
- `src/map/division_selection.rs` — `FrontlineSelectMode`ガード追加、回帰テスト追加
- `src/map/selection.rs` — 同上
- `src/map/frontline_render.rs` — 強調表示描画追加、5件のテスト新規追加(テストモジュール自体が新規)
- `src/ui/military_panel.rs` — Army前線UI・ハンドラ・テスト追加
- `src/save/dto.rs` — `army_frontline_map`フィールド追加、テスト追加
- `src/save/export.rs` — 変換ロジック追加、既存テスト拡張
- `src/save/apply.rs` — 復元ロジック追加、既存テスト拡張
- `src/save/validate.rs` — 検証ロジック追加、6件のテスト新規追加
- `assets/localization/en-US.ron` / `ja-JP.ron` — 10キー追加

**新規ファイル**
- `src/map/frontline_selection.rs` — 前線選択モード・入力処理・クリック判定
- `tests/p21_005_army_frontline_e2e_test.rs` — E2Eテスト2件

## 14. 追加テスト一覧(概数)

| ファイル | 新規テスト数 |
|---|---|
| `src/war/frontline.rs` | 17 |
| `src/map/frontline_selection.rs`(新規ファイル) | 4 |
| `src/map/division_selection.rs` | 1 |
| `src/map/selection.rs` | 1 |
| `src/map/frontline_render.rs`(このファイルへの初のテスト追加) | 5 |
| `src/ui/military_panel.rs` | 10 |
| `src/save/dto.rs` | 1 |
| `src/save/export.rs` | 0(既存テストの拡張のみ、新規関数なし) |
| `src/save/apply.rs` | 0(既存テストの拡張のみ、新規関数なし) |
| `src/save/validate.rs` | 6 |
| `tests/p21_005_army_frontline_e2e_test.rs`(新規ファイル) | 2 |
| **合計** | **47** |

(初版報告時に`war/frontline.rs`を21件・`frontline_selection.rs`を5件と誤記していたが、関数名を1件ずつ突き合わせて再集計し47件に訂正。実測合計536件から開始時489件を引いた差分と一致することを確認済み。)

## 15. 68項目との対応表

| # | 項目 | 状態 | 実装/テスト |
|---|---|---|---|
| 1 | 有効なArmyをFrontlineへ割当 | ✅ | `test_assign_army_to_frontline_succeeds` |
| 2 | 1つのFrontlineへ複数Army | ✅ | `test_multiple_armies_can_share_one_frontline` |
| 3 | 1つのArmyは最大1Frontline | ✅ | `test_army_belongs_to_at_most_one_frontline_reassignment_removes_old` |
| 4 | 再割当で旧Frontlineから消える | ✅ | 同上 |
| 5 | 解除で割当が消える | ✅ | `test_unassign_army_removes_assignment_and_is_idempotent` |
| 6 | 解除の複数回安全性 | ✅ | 同上 |
| 7 | 存在しないArmyId拒否 | ✅ | `test_assign_army_rejects_nonexistent_army_id` |
| 8 | 存在しないFrontlineId拒否 | ✅ | `test_assign_army_rejects_nonexistent_frontline_id` |
| 9 | 他国Armyを拒否 | ✅ | `test_assign_army_rejects_foreign_owned_army` |
| 10 | 非交戦国側Frontline拒否 | ✅ | `test_assign_army_rejects_non_participant_country` |
| 11 | 終了済みWarのFrontline拒否 | ✅ | `test_assign_army_rejects_ended_war_frontline` |
| 12 | HashMap挿入順非依存 | ✅ | `test_assign_army_ordering_is_deterministic_not_hashmap_order`, `find_assignable_frontline_for_click_picks_lowest_frontline_id_deterministically` |
| 13 | Armyメンバー増減後も割当維持 | ✅ | `test_army_member_division_changes_do_not_affect_frontline_assignment` |
| 14 | Army削除で割当清掃 | ✅ | `test_army_disband_cleans_up_frontline_assignment` |
| 15 | Frontline削除で割当清掃 | ✅ | `test_frontline_removal_cleans_up_army_assignment` |
| 16 | War終了・講和で割当清掃 | ✅ | `test_war_end_cleans_up_army_assignment` |
| 17 | 割当前後でDivision全フィールド不変 | ✅ | `test_army_assignment_does_not_mutate_any_division_fields` |
| 18 | `frontline_generated_movements`へ非追加 | ✅ | 同上 |
| 19 | 翌日FrontlineOrders後も非移動 | ✅ | `test_army_assignment_alone_does_not_trigger_frontline_orders_automation`, E2E |
| 20 | 割当だけでは戦闘非開始 | ✅ | 同上(combat_id不変を確認) |
| 21 | 解除でも移動・戦闘状態非変更 | ✅ | `test_army_assignment_does_not_mutate_any_division_fields`(後半) |
| 22 | 既存の直接移動→自動戦闘は継続動作 | ✅(回帰) | 既存`land_war_combat_peace_test.rs`/`daily_system_integration_test.rs`が無変更のまま green(このタスクでは対象コード非変更) |
| 23 | 設定ボタン表示(選択中プレイヤーArmy) | ✅ | `army_frontline_buttons_are_spawned_in_military_panel_ui_tree` |
| 24 | 未選択時は操作不能 | ✅ | `army_frontline_buttons_disabled_without_selection` |
| 25 | 他国Armyでは操作不能 | ✅ | `army_frontline_assign_button_disabled_for_foreign_army` |
| 26 | 設定ボタンで選択モードへ | ✅ | `assign_button_toggles_frontline_select_mode` |
| 27 | 有効Frontlineクリックで割当 | ✅ | `find_assignable_frontline_for_click_matches_own_front_region` + E2E(`assign_army`直接呼出) |
| 28 | 無効Frontlineクリックで無変更 | ✅ | `find_assignable_frontline_for_click_matches_own_front_region`(敵側地域はNone) |
| 29 | Escapeでキャンセル | ✅ | `escape_key_cancels_frontline_select_mode` |
| 30 | 右クリックでキャンセル | ✅ | `frontline_select_mode_blocks_movement_order_from_firing`(移動非発行を確認)+ `cancel_frontline_select_mode_on_context_change`の右クリック分岐 |
| 31 | Army変更でキャンセル | ✅ | `changing_selected_army_cancels_frontline_select_mode` |
| 32 | 解除ボタンで選択Armyだけ解除 | ✅ | `unassign_button_removes_only_selected_army_assignment_and_notifies` |
| 33 | 現在割当がArmyパネルへ表示 | ✅ | `update_army_frontline_ui_reflects_current_assignment_state` |
| 34 | 前線クリックがDivision移動へ非漏出 | ✅ | `frontline_select_mode_blocks_movement_order_from_firing` |
| 35 | 通常モードの移動操作に回帰なし | ✅(回帰) | 既存`handle_movement_order_applies_to_all_selected_divisions`等 green |
| 36 | JA/EN切替後も表示更新 | ✅ | `!state.open && !locale.is_changed()`の既存ガードパターンを踏襲(専用の自動テストは未追加)。2026-08-14実GUI確認(セクション22項目13)でJA/EN切替が正常であることを直接確認済み |
| 37 | UI文字列ハードコードなし | ✅ | `p20_009_hardcoded_string_scan_test`全4件green |
| 38 | Frontline Overlay数が正しい | ✅ | `overlay_count_does_not_grow_across_unchanged_frames` |
| 39 | 選択モードで有効前線を強調 | ✅ | `select_mode_adds_highlight_overlays_for_assignable_frontlines` |
| 40 | hover前線を強調 | ✅ | `hovered_state_id`実装。Window非依存の単体テストではhover分岐を直接検証していないが、2026-08-14実GUI確認(セクション22項目5)で有効前線の強調表示を直接確認済み |
| 41 | 割当前線を強調 | ✅ | `assigned_army_frontline_is_highlighted_even_without_select_mode` |
| 42 | モード終了で通常表示へ戻る | ✅ | オーバーレイが`FrontlineSelectMode`の現在値からのみ導出されるため、モード解除後の次フレームで自動的に通常表示。専用の自動テストは未追加だが、2026-08-14実GUI確認(セクション22項目10)でEscape/右クリック後の表示復帰を直接確認済み |
| 43 | Frontline削除後に古いOverlay非残存 | ✅ | `overlay_disappears_after_frontline_removed` |
| 44 | 描画Entity数がフレームごとに非増加 | ✅ | `overlay_count_does_not_grow_across_unchanged_frames` |
| 45 | Army前線割当をRON往復保持 | ✅ | `round_trip_preserves_frontline_registry_including_tuple_keyed_plans`(dto.rs) |
| 46 | 複数Army割当を保持 | ✅ | E2Eテスト(複数州の前線+単一Army、`assigned_army_ids`往復) |
| 47 | 旧形式RON(フィールド無し)を空として受理 | ✅ | `old_format_ron_without_army_frontline_fields_loads_as_empty` |
| 48 | 存在しないArmyId参照を拒否 | ✅ | `frontline_plan_assigned_army_unknown_id_is_rejected`, `army_frontline_map_dangling_army_id_is_rejected` |
| 49 | owner不一致を拒否 | ✅ | `frontline_plan_assigned_army_wrong_owner_is_rejected` |
| 50 | 同一Armyの複数Frontline割当を拒否 | ✅ | `army_assigned_to_two_frontlines_is_rejected` |
| 51 | Load後に割当表示を再構築 | ✅ | E2E(`frontline_for_army`/`assigned_army_ids`復元確認) |
| 52 | Load後に解除・再割当可能 | ✅ | E2E(65番) |
| 53 | Load後に再保存可能 | ✅ | E2E(66番) |
| 54 | Apply失敗時にWorld不変 | ✅ | `load_rejects_inconsistent_army_frontline_save_and_preserves_world_state`(E2E) |
| 55 | `version = 1`を維持 | ✅ | E2E内で直接assert |
| 56 | SaveGameV1トップレベル18フィールド不変※ | ✅ | 構造体定義非変更を確認(※spec原文は17だが、P21-SAVE-003完了時点で既に18フィールドが正。本タスクでは追加も削除もしていない) |
| 57 | 実7か国28州でWar/Frontline/Army用意 | ✅ | E2E(`army_frontline_assignment_survives_save_load_round_trip_with_real_map_data`) |
| 58 | ArmyをFrontlineへ割当 | ✅ | 同上 |
| 59 | Division非移動確認 | ✅ | 同上 |
| 60 | セーブ | ✅ | 同上 |
| 61 | 状態変更(解除) | ✅ | 同上 |
| 62 | ロード | ✅ | 同上 |
| 63 | Army割当復元 | ✅ | 同上 |
| 64 | 描画・Armyパネル同期 | ✅(代理) | Army/Division/FrontlineRegistryの整合性を直接確認(実描画・実パネルはWindow非依存のためUI単体テスト側で別途担保) |
| 65 | 解除・再割当 | ✅ | 同上 |
| 66 | 再セーブ | ✅ | 同上 |
| 67 | 新規ID非衝突 | ✅ | 同上 |
| 68 | 既存489件へ回帰なし | ✅ | lib 465件 + 安全な統合71件、全green(内訳はセクション16) |

## 16. テスト数の変更前後

- 開始前(メモリ記録、2026-08-14朝時点): 489件
- 完了時点: lib 465件 + 統合テスト(実行分)71件 = **536件**(headless-render系3件は既存方針により実行せず、コンパイルのみ確認)
- 新規追加: **47件**(内訳はセクション14)、削除0件

## 17. E2E結果

`tests/p21_005_army_frontline_e2e_test.rs`(実7か国28州データ、`AppPlugin`/`WarPlugin`/`MilitaryPlugin`/`SaveGamePlugin`/`LoadGamePlugin`等の本番プラグイン構成):
- `army_frontline_assignment_survives_save_load_round_trip_with_real_map_data`: PASS — Arcadia(0) vs Elfin(1)の実War・実国境(State1↔State3)から生成された実Frontlineへ、実Divisionを含むArmyを割当→Division不動確認(直後・翌日SystemSet通過後)→セーブ→解除→ロードで復元→解除・再割当→再セーブ→新規Division/Army ID非衝突、を一気通貫で確認。
- `load_rejects_inconsistent_army_frontline_save_and_preserves_world_state`: PASS — 存在しないArmyIdを参照する破損`army_frontline_map`を注入したセーブがLoad失敗になり、Worldが一切変更されないことを確認。

## 18. 全検証結果

- `cargo check --all-targets`: ✅ 成功
- `cargo test --lib`: ✅ 465 passed
- `cargo test`(安全な統合テスト11バイナリ: daily_system_integration/diplomacy/economy/land_war_combat_peace/p20_009_hardcoded_string_scan/p20_009_localization_resource/p21_005_army_frontline_e2e/p21_save_002e_end_to_end/p21_save_003_end_to_end/profile_workload_correctness/research_and_politics): ✅ 71 passed, 0 failed
- headless-render系3バイナリ(`p20_009_localization_headless_render_test`/`p21_save_002e_headless_render_test`/`ui_headless_render_test`): 既存の証拠保護方針に従い`--no-run`でコンパイルのみ確認、実行せず(固定PNG非汚染)
- `cargo clippy --all-targets --all-features -- -D warnings`: ✅ 0警告
- `cargo build --release --all-targets`: ✅ 成功
- `cargo fmt --check`: 変更ファイルは全て整形済み(セクション19参照)。未変更ファイルの既存差分は保護。**ユーザーが2026-08-14に実機で`cargo fmt --all -- --check`を実行しPASSと報告**しているが、本レポート作成時点でのこのセッション自身の`cargo fmt --check`/`cargo fmt --all -- --check`再実行では、本タスクで一切変更していない既存20ファイル分のdiff(セクション19の一覧)が引き続き検出されている。原因は特定できていない(rustfmtツールチェーンのバージョン差、実行ディレクトリ差等の可能性はあるが未調査)。矛盾を隠さずここに記録する。本タスクで変更した10ファイル自体が整形済みであることは、このセッション自身のツール実行で確認済みの事実(セクション19)。
- `git diff --check`: ✅ 空白関連の実質的な問題なし(LF/CRLF変換警告のみ、リポジトリ全体の既存挙動)
- 一時セーブ・tmpディレクトリ: E2Eテストは`Drop`で自動削除、セッション終了時点で残留なし確認済み
- プロセス残留: `tasklist`で`strategy_game`/`cargo`関連プロセスなしを確認

## 19. rustfmtベースライン比較

- **開始時点**: `cargo fmt --check`で83ファイルにdiff(前回記録の「1 hunk」は誤り/古い情報だったため、指示に従い無条件採用せず現在値を再測定した)。
- **完了時点**: 20ファイルにdiff(全て本タスクで一切変更していない既存ファイル: `loader.rs`/`country_ai.rs`/`division_render.rs`/`map/mod.rs`(前線選択Plugin登録行以外は無変更)/`movement.rs`/`recruitment.rs`/`supply.rs`/`military/tests.rs`/`profiling.rs`/`save/runtime.rs`/`country_selection.rs`/`peace_panel.rs`/`capitulation.rs`/`war/military_ai.rs`/`war/peace.rs`/`war/tests.rs`/`daily_system_integration_test.rs`/`land_war_combat_peace_test.rs`/`p21_save_003_end_to_end_test.rs`/`profile_workload_correctness_test.rs`)。
- 本タスクで変更した10ファイル(`war/frontline.rs`/`map/frontline_selection.rs`/`map/division_selection.rs`/`map/selection.rs`/`map/frontline_render.rs`/`ui/military_panel.rs`/`save/dto.rs`/`save/export.rs`/`save/apply.rs`/`save/validate.rs`)は、`rustfmt --edition 2024 <個別ファイル>`(mod.rs/lib.rs/main.rsは対象外、`cargo fmt`はワークスペース全体を巻き込むため未使用)でリーフファイル単位に整形し、いずれも**整形後diff 0**。
- `src/map/mod.rs`/`src/war/mod.rs`は、自身に`pub mod`宣言を含むため`rustfmt`を直接実行するとモジュールツリー全体(未変更ファイル含む)を巻き込む(実際に`--check`で確認済み)ことが判明したため、**直接rustfmtを実行せず手動編集のみ**とした。追加した行(`pub mod frontline_selection;`/`use frontline_selection::FrontlineSelectionPlugin;`等)自体は整形ルールに準拠していることを個別確認済み(`map/mod.rs`の既存diff3箇所はいずれも本タスクで触れていない`camera`/`division_render`/`division_selection`の既存順序に起因、`war/mod.rs`はdiff 0)。

## 20. git status/diff

- 開始時: `M`23ファイル(既存の未コミット作業)+複数の`??`未追跡ディレクトリ(P21-SAVE系の成果物)。
- 終了時: 開始時からの純増分は「本タスクで変更した10ファイルが`M`へ追加」「`map/frontline_selection.rs`と`tests/p21_005_army_frontline_e2e_test.rs`が新規`??`として追加」のみ。既存の`M`/`??`エントリは一切変化なし(`diff`コマンドで前後比較し確認済み)。
- `git diff --check`: 実質的な空白エラーなし。

## 21. 既存証拠を上書きしていないこと

- `states.ron`/`countries.ron`/`divisions.ron`/`tests/land_war_combat_peace_test.rs`/`src/war/military_ai.rs`: `git diff --stat`で差分ゼロを確認(完全無変更)。
- `src/app/time.rs`: 差分ありだがこれは開始時点で既に`M`だった既存の未コミット変更であり、本タスクでは一切編集していない(Editツール呼び出し履歴なし、読み取りのみ)。
- 固定PNG(`verification_logs/**/*.png`等)を書き込むheadless-renderテスト3件は`--no-run`のみで実行しておらず、既存スクリーンショットは非汚染。
- P21-SAVE-001〜003の完了報告書・検証ログディレクトリは一切変更していない。

## 22. 人間によるGUI確認(完了)

本セッション自身は実ウィンドウ操作ができないため、報告書の初版では以下15項目を「未実施」として提出した。**2026-08-14、ユーザーが実`cargo run`で全15項目を確認し、15/15 PASS・発見された不具合なし、と報告した。**

1. `cargo run`でNew Game開始 — PASS
2. Armyを作成・選択 — PASS
3. 戦争中のFrontlineが表示される — PASS
4. 「前線を設定」を押す — PASS
5. 有効前線が強調される — PASS
6. 前線をクリックして割当 — PASS
7. Armyパネルに現在の割当が表示される — PASS
8. Divisionが勝手に移動しない — PASS
9. 直接移動命令は従来どおり使える — PASS
10. Escape・右クリックで選択モードを解除できる — PASS
11. 「前線を解除」で割当だけが消える — PASS
12. セーブ→状態変更→ロードで割当が復元される — PASS
13. JA/EN切替が正常 — PASS
14. 前線クリックが移動・選択命令へ漏れない — PASS
15. ウィンドウ終了後にプロセスが残らない — PASS

これにより、自動テスト(536件、ロジック・データ・入力ガード・保存/復元レベル)と実機GUI確認(表示・クリック座標・実際の操作フロー)の両方が完了し、判定を`COMPLETE`へ更新した。

## 23. 発見した問題

- 実装中に自己発見・即時修正した問題(いずれも本タスク内で完結):
  - `division_selection.rs`のテストコードへ`FrontlineSelectMode`初期化を追加する際、`replace_all`編集が同一箇所へ二重適用され、1テスト内に重複行が生じていた(機能上は無害だが、`rustfmt --check`の差分確認時に発覚し修正済み)。
  - E2Eテストで`War`を`WarRegistry.wars`へ直接`insert`し`next_id`を更新しなかったため、セーブ後のロードで`NextIdCollision`検証に失敗(テストのバグ、実装のバグではない)。`WarRegistry::add_war`経由に修正。
  - clippyで5件の指摘(collapsible_match/too_many_arguments/field_reassign_with_default/let_and_return×2)、いずれも軽微でその場で修正、0警告化。
  - `Text::new("")`(テストコード内)がP20-009のハードコード文字列走査に検出され、`Text::new(String::new())`(既存の`country_selection.rs`に前例のあるパターン)へ変更して解消。
- 未解決の既知の限界(スコープ外として明示):
  - hover強調(#40)・モード終了時の表示復帰(#42)・JA/EN切替後の即時再描画(#36)は、Window非依存の単体テストでは直接検証できていない(引き続き専用の自動テストは無い)。2026-08-14の実GUI確認で目視により正常動作が確認されたが、自動回帰テストとしては未整備のため、将来この領域に手を入れる際は目視確認済みの挙動を壊さないよう注意が必要。
  - `cargo fmt --check`/`cargo fmt --all -- --check`の結果がユーザー実機(PASS)とこのセッションの再実行(既存20ファイル分のdiffが残存)で食い違っている(セクション18)。本タスクで変更したファイル自体の整形は完了しているため機能上の実害はないが、原因未特定のまま残っている。

## 24. 次タスクへの移行可否

**可**。P21-005で規定された範囲(Army↔Frontline割当・UI・入力・描画・清掃・セーブ)は自動検証(536件)と実機GUI確認(15/15 PASS)の両方が完了し、判定は`COMPLETE`。P21-006以降の機能(攻勢線・自動進軍・複数Army一括割当等、非対象と明記されたもの)には一切着手していない。
