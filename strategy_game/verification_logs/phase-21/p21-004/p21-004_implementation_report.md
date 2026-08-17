# P21-004: 軍(Army)編成の実装 完了報告

**実施日**: 2026-08-12
**前提**: P21-004R(軍事階層用語の正規化)は完了済み。既存の階層は
Division/DivisionId(個別師団)・Army/ArmyId(複数師団の編成)・ArmyGroup/ArmyGroupId(将来予約、未使用)。
**スコープ**: 将軍、能力値、指揮上限、軍集団(ArmyGroup)、前線への実割当、自動進軍は今回実装しない。

---

## 1. 最終判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

自動検証(cargo check/test/clippy/fmt/build)はすべて green。実機起動確認(データロード成功・
パニックなし)も実施済み。ただし本報告書作成時点では、実際にマウス操作でボタン・編成一覧行を
クリックする人手によるインタラクティブUI確認はまだ行われていない(§11参照)。

---

## 2. ArmyとDivisionの最終データ構造

`src/military/army.rs`(既存、今回は不変条件の一部修正のみ):

```rust
pub struct Army {
    pub id: ArmyId,
    pub owner: CountryId,
    pub name: String,
    pub member_division_ids: Vec<DivisionId>,  // DivisionId昇順で安定保持
}

pub struct ArmyRegistry {
    pub armies: HashMap<ArmyId, Army>,
    pub division_army_map: HashMap<DivisionId, ArmyId>,  // 1師団は1編成のみ、の一元管理
    next_id: usize,
    next_army_number: HashMap<CountryId, u32>,  // 国家ごとの自動命名カウンタ
}
```

今回新規追加(UI選択状態、シミュレーション本体には影響しない):

```rust
// src/military/army.rs
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedArmy(pub Option<ArmyId>);
```

**設計不変条件**(全て`ArmyRegistry`のメソッド内で一元的に保証):
- 1個のDivisionは同時に1個のArmyだけへ所属できる: `division_army_map`が唯一の真実であり、
  所属変更は必ず`detach_division`(内部ヘルパー)を経由する。
- Army側に存在するがDivision側では未所属、という不整合は発生しない: `member_division_ids`の
  追加・削除と`division_army_map`の追加・削除は必ず同一メソッド内でセットで行われる。
- Division側にArmyIdがあるがArmyが存在しない、という不整合は発生しない: `disband`/
  `detach_division`はArmy削除時に必ず`division_army_map`からも該当エントリを除去する。
- 他国のDivisionをArmyへ追加できない: `create_army`/`add_division`/`remove_division`/`disband`
  すべてが所有者検証を行う。
- 消滅済みDivisionがArmyへ残らない: `sanitize_references`(日次)が`division_is_usable`
  (存在・自国所有・manpower>0・非Destroyed)で判定し除去する。
- BevyのEntityは一切使用しない: `Army`/`ArmyRegistry`はプレーンなデータ(`war::frontline::
  FrontlineRegistry`と同じ設計方針)。

---

## 3. ArmyIdの発行方法

`ArmyRegistry.next_id: usize`のカウンタを`create_army`実行時にインクリメントして払い出す
(`ArmyId(self.next_id)`後に`self.next_id += 1`)。`FrontlineId`/`BattleId`等、既存の全IDと
同一の単調増加パターン。解散や自動解散で番号が再利用されることはない。

軍名は`next_army_number: HashMap<CountryId, u32>`により**国家ごとに独立**して"Army 1",
"Army 2", ...と自動採番される(既存実装のまま、変更なし)。

---

## 4. 所属関係を一元管理するコード経路

`ArmyRegistry`内の非公開ヘルパー`detach_division(&mut self, division_id: DivisionId)`が
唯一の「所属解除」実装であり、`create_army`・`add_division`・`remove_division`はすべて
これを経由する。今回、このヘルパーに**空編成の自動解散**を追加した:

```rust
fn detach_division(&mut self, division_id: DivisionId) {
    if let Some(old_group_id) = self.division_army_map.remove(&division_id)
        && let Some(old_group) = self.armies.get_mut(&old_group_id)
    {
        old_group.member_division_ids.retain(|&id| id != division_id);
        if old_group.member_division_ids.is_empty() {
            self.armies.remove(&old_group_id);  // 新規: 空編成を放置しない
        }
    }
}
```

`sanitize_references`(日次)にも同様に「整理の結果メンバー0件になった編成を削除する」処理を
追加した。これにより「空のArmyは自動解散する」という最小仕様の要求(§1/§3/§6)を、手動除外・
編成移動・師団消滅のどの経路からでも一貫して満たす。

**発見した自己参照バグとその修正**(実装中に自動テストで検出): `add_division`が既にその編成の
唯一のメンバーである師団を「追加」しようとすると、`detach_division`が呼ばれた瞬間にその編成が
空になって自動解散され、直後の`get_mut`が存在しない編成を参照してpanicする経路が新たに生まれた
(「選択師団を追加」ボタンは選択中の全師団に対して無条件に`add_division`を呼ぶため、既に
対象編成の一員である師団が選択に混ざっていると再現する)。`add_division`の先頭で「既にこの編成へ
所属済みなら即座にOk(())で返す」ガードを追加して解決した(回帰テスト:
`add_division_already_sole_member_of_target_is_a_safe_no_op`)。

---

## 5. UI操作経路

`src/ui/military_panel.rs`の「編成(軍)」セクションを、旧来の「選択中陸軍から動的に対象編成を
推測する」方式から、**編成一覧をクリックして明示的に選ぶ**方式へ作り替えた(仕様書§4「軍選択後も
既存操作を利用できる入口として実装」に合わせた設計判断)。

- `ArmyListContainer`(Node) + `ArmyListRowButton(pub ArmyId)`: 自国の編成をArmyId昇順で
  一覧表示する。更新のたびに子を全破棄・再構築する(`ui::diplomacy_panel`の
  `DiplomacyContentContainer`と同一パターンを踏襲)。各行は軍名・所属師団数を表示し、選択中の
  編成は背景色でハイライトする。
- `handle_army_list_row_clicks`: 行クリックで`SelectedArmy`を切り替え、その編成の生存中の
  全師団を`SelectedDivision`へ反映する(`SelectedDivision::select_only_many`を新規追加)。
  他国編成のクリックは無視する。
- ボタンは4個に整理: 「編成作成」「選択師団を追加」「選択師団を除外」「軍を解散」
  (旧来の「軍を選択」ボタンは、一覧行クリックに置き換わったため削除)。
- `update_army_ui`: 毎更新時に`SelectedArmy`を再検証し、解散・自動解散で消滅した編成や
  他国編成を指していればNoneへ戻す。
- UI操作がマップ操作へ漏れない件: 新設した`ArmyListRowButton`も他の全ボタンと同じく
  `Button`+`Interaction`を持つ通常のUIノードであるため、`map::selection::handle_state_click`/
  `map::division_selection::handle_movement_order`の既存の汎用ガード
  (`Query<&Interaction>`で`Hovered`/`Pressed`を検出したら即return)がそのまま働く。
  この点は専用の回帰テストで確認した(§9 #17)。

---

## 6. 一括移動・停止との接続

「Army選択は既存のDivision選択を置き換えるのではなく、一括選択する入口として実装する」という
制約を、`handle_army_list_row_clicks`が`SelectedDivision`を書き換えるだけの薄い処理として
実装することで満たした。**移動・停止・接敵戦闘の各システム自体は一切変更していない**:

- 一括移動(`map::division_selection::handle_movement_order`): 変更なし。テストのために
  `pub(crate)`可視性を追加しただけ。既存の「選択中の各陸軍へ独立に移動命令を発行する」ロジックが
  そのまま、Army選択で埋まった`SelectedDivision`に対して動く。
- 前線コマンド(`ui::military_panel::handle_frontline_command_buttons`他): 完全に無変更。
- 実証テスト: `army_selection_feeds_into_existing_bulk_movement_order`(軍クリック→右クリック
  移動で2師団とも移動命令が発行される)、`army_selection_feeds_into_existing_frontline_division_command`
  (軍クリック→前線Assignボタンで選択依存コマンドが機能する)。

**「一括停止」についての補足**: この既存コードベースで「停止」に対応する唯一のUI操作は
`FrontlineCommand::SetStance(FrontlineStance::Stopped)`だが、これは選択中師団ではなく
**国家の前線プラン全体**に効く設定であり、そもそも選択集合に依存しない(この設計は今回変更して
いない)。そのため「Army選択後に一括停止できる」の検証は、選択集合に実際に依存する前線コマンド
(`Assign`)を代表として使用した。この解釈上の判断は透明に記録しておく。

---

## 7. Division消滅時の処理

`src/military/mod.rs::handle_daily_army_maintenance`(既存、日次)が`ArmyRegistry::
sanitize_references`を呼ぶ経路は変更していない。今回の修正で、この経路が以下をすべて満たす:

1. 消滅済み師団を所属編成の`member_division_ids`から除去(既存)
2. `division_army_map`からも対応エントリを除去(既存)
3. 所属師団が0件になった編成を`self.armies`から自動削除(**新規**、§4参照)

`SelectedDivision`からの除去は本タスクの範囲外の既存機構(`map::division_selection::
prune_selected_division`、`military_registry.is_changed()`ゲート)がそのまま担当し、今回は
未変更・無影響であることを確認した。`SelectedArmy`が指す編成が日次整理で消滅した場合は、
`update_army_ui`が毎回`army_registry.armies`に対して再検証してNoneへ戻す(§5)。

この経路は`division_is_usable`(存在・所有者・manpower>0・非Destroyed)による汎用判定であり、
戦闘による消滅に限らず、Divisionが消滅する将来のどの経路(講和による返還・行政消滅等)にも
そのまま適用される共通処理である。

---

## 8. 変更ファイル一覧

```
 M assets/localization/en-US.ron         (+14/-9行相当、army_select_button/army_list_line/
                                           army_list_header削除、army_list_row追加、文言更新)
 M assets/localization/ja-JP.ron         (同上、日本語側)
 M src/map/division_selection.rs         (SelectedDivision::select_only_many追加、
                                           handle_movement_orderをpub(crate)化)
 M src/military/army.rs                  (SelectedArmy追加、空編成自動解散、
                                           自己参照トラップ修正、テスト6件追加)
 M src/military/mod.rs                   (SelectedArmyをリソース登録)
 M src/ui/military_panel.rs              (Army UIの大幅書き換え、テスト多数追加・修正)
```

`git diff --stat`(実行時点): 8ファイル変更、+752/-187行。

**保護ファイル**(`states.ron`/`countries.ron`/`divisions.ron`/`app/time.rs`): 空diff、無変更を確認。

---

## 9. 追加・更新したテスト

**`src/military/army.rs`**(11→17件、+6):
- `army_ids_are_unique_across_creations` (spec #2)
- `removing_last_division_auto_disbands_the_army` (spec #11)
- `sanitize_references_auto_disbands_army_that_becomes_empty` (spec #14)
- `moving_last_division_to_another_army_auto_disbands_the_source_army` (空編成自動解散の
  移動経路での確認、spec #11/#14を補強)
- `add_and_remove_reject_nonexistent_division_id_safely` (spec #15)
- `add_division_already_sole_member_of_target_is_a_safe_no_op` (§4で発見した自己参照バグの回帰テスト)
- `add_division_moves_it_from_previous_group`: 既存テストを、自動解散の副作用と分離できるよう
  移動元編成を2師団編成に変更して更新(spec #4/#5)

**`src/ui/military_panel.rs`**(Army関連 約15→25件、既存frontlineテストは無変更):
- `army_command_buttons_are_spawned_in_military_panel_ui_tree`: ボタン数を4個・
  `ArmyListContainer`存在確認に更新
- `handle_army_command_buttons_create_success`: 作成後`SelectedArmy`が設定されることの
  確認を追加 (spec #1)
- `handle_army_command_buttons_add_selection_adds_ungrouped_member`: `SelectedArmy`を
  明示的に設定するよう更新 (spec #3)
- `clicking_army_list_row_selects_army_and_expands_division_selection`(新規、旧
  `handle_army_command_buttons_select_group_expands_selection`を置換): spec #7
- `clicking_foreign_army_list_row_is_ignored`(新規): spec #16
- `army_selection_feeds_into_existing_bulk_movement_order`(新規): spec #8
- `army_selection_feeds_into_existing_frontline_division_command`(新規): spec #9
- `handle_army_command_buttons_disband_returns_members_to_unassigned`: `SelectedArmy`の
  設定・解散後Noneへ戻ることの確認を追加 (spec #5/#12)

**`src/map/selection.rs`**(新規テストモジュール、0→1件):
- `ui_button_press_blocks_map_state_click_from_firing`: spec #17

**`src/map/rendering.rs`**(既存モジュールへ追加、+1件):
- `state_sprite_color_updates_to_new_owner_after_peace_cession`: spec #20
  (占領による色変化は前回セッションで実装・テスト済み。今回は`StateRegistry::
  transfer_region_ownership`を直接使う講和割譲経路を別途検証した)

---

## 10. 検証コマンドと終了コード

| コマンド | 結果 |
|---|---|
| `cargo fmt --check` | 今回変更したコードの新規差分はゼロ(手動整形済み)。`military_panel.rs`の
一部プレ既存フォーマット債務(P21-004Rから継続、フロントラインテスト関数内)は今回触れていない
関数のため意図的に未整形のまま |
| `cargo check --all-targets` | 0エラー、exit 0 |
| `cargo test -- --list` | 170 libテストを一覧表示、exit 0 |
| `cargo test`(lib) | **170 passed**; 0 failed、exit 0 |
| 安全な統合テストバイナリ8種(`economy_tests`/`diplomacy_tests`/`research_and_politics_tests`/
`p20_009_hardcoded_string_scan_test`/`p20_009_localization_resource_test`/
`daily_system_integration_test`/`profile_workload_correctness_test`/`land_war_combat_peace_test`) |
**59 passed**; 0 failed、exit 0(各々) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 0警告、exit 0 |
| `cargo build --release --all-targets` | 成功、exit 0(初回約3分) |
| `git diff --check` | 既存のLF/CRLF警告のみ、実質問題なし、exit 0 |
| 実機`cargo run --release`起動確認 | `[DataLoader] Successfully loaded 8 buildings, 22
technologies, 6 countries, 28 states, 3 diplomatic relations`、パニックなし
(タイムアウトによる強制終了 exit 143 は想定通り) |

**テスト総数**: 170(lib) + 59(統合) = **229**(今回開始前218から+11)。
Headless描画テスト(`ui_headless_render_test`/`p20_009_localization_headless_render_test`)は
既定の証跡保護方針により今回も未実行(安全な一時出力先の設定が存在しないため)。

---

## 11. 手動確認が必要な項目

実機で以下を確認してください:

- 複数師団を選択した状態で「編成作成」を押すと、新しい編成が作られ、編成一覧にすぐ表示される
- 編成一覧の行をクリックすると、その編成に所属する師団だけがマップ上で選択される(ハイライト)
- 軍選択後、右クリックでの移動・前線コマンドボタンが選択師団に対して機能する
- 選択中に別の師団を追加選択(Ctrl+クリック/ドラッグ)してから「選択師団を追加」を押すと、
  その師団が選択中編成へ加わる
- 「選択師団を除外」で師団が編成から外れ、一覧の所属数表示が即座に更新される
- 最後の1師団を除外すると、編成が一覧から自動的に消える
- 「軍を解散」で編成が消え、所属していた師団は個別に選択可能な状態へ戻る(移動命令や戦闘状態は
  変化しない)
- 戦闘で編成中の師団が消滅しても、UIが壊れたりフリーズしたりしない(一覧の数字が自動で減る)
- 敵国の編成は一覧に表示されず、誤って操作できない
- 編成一覧のボタンをクリックしている最中に、裏でマップの州選択や師団移動が誤発火しない

---

## 12. 発見した問題

1. **自己参照バグ(修正済み)**: §4参照。「選択師団を追加」で既に対象編成の唯一のメンバーである
   師団を含む選択を渡すと、空編成自動解散との組み合わせでpanicする経路があった。自動テストで
   検出・修正・回帰テスト追加済み。
2. **「一括停止」の仕様上の曖昧さ**: §6参照。既存の「停止」操作(前線Stoppedスタンス)は選択集合に
   依存しないプラン全体設定であるため、字義通りには「Army選択後に一括停止できる」の検証対象になり
   得ない。選択依存の前線コマンド(Assign)で代替検証し、この解釈をここに明記した。ユーザーの意図と
   異なる場合はご指摘いただきたい。
3. **`military_panel.rs`のプレ既存フォーマット債務**: P21-004Rセッションで蓄積された
   `cargo fmt`未適用差分のうち、今回のArmy機能改修で触れなかったフロントライン系テスト関数の分は
   意図的に手を付けていない(スコープ外の広範な変更を避けるため)。今回新たに書いた・変更した
   コードの分はすべて手動整形済み。

---

## 13. P21-005前線システムへの接続可否

**READY**

現状の`FrontlineRegistry`(`assigned_division_ids: Vec<DivisionId>`、
`division_frontline_map: HashMap<DivisionId, FrontlineId>`)は個別師団を保持する設計であり、
今回`Army`側へは一切手を加えていない(仕様書の「今回の実装だけを理由に前線システムを大規模変更
しない」制約を遵守)。

将来、以下の移行が可能であることをコードレベルで確認した(実装はしていない):
- `Army`は既に`member_division_ids: Vec<DivisionId>`を安定して保持しており、これを
  `FrontlineRegistry`側が「Armyの全所属師団を前線へ展開する」形で参照する拡張は、既存の
  `sanitize_references`パターンをもう一段(Army起点)重ねるだけで実現できる構造になっている。
- `war::frontline`モジュールと`military::army`モジュールの間に現状コンパイル時の依存関係は
  存在しない(疎結合)。`FrontlineId`を`Army`側が持つか、`ArmyId`を`FrontlineRegistry`側が
  持つか、どちらの方向でも後付け可能。
- `ArmyId`は`Army`/`ArmyRegistry`の外で意味を持つ安定した永続IDであり(BevyのEntityではない)、
  セーブ/ロード実装時にも他の全IDと同様にそのままシリアライズ可能。

以上より、P21-005(前線システムへのArmy接続)着手の技術的な障害はない。ただし「Armyへの前線
割当時、内部の個別師団への展開をいつ・どこで行うか」(Army選択時に静的展開するか、日次で
動的に再展開するか)は製品設計上の判断であり、着手前にユーザーの決定が必要。

---
---

# 追補: P21-004A(選択Division向け「移動停止」機能の追加)

**実施日**: 2026-08-12(P21-004完了報告と同日、直後の追加受入対応)
**トリガー**: P21-004受入時、「Army選択後に一括停止できる」の検証が、選択に依存しない既存の
前線スタンス「停止」(`FrontlineStance::Stopped`)で代替されており、字義通りの受入条件を
満たしていないと指摘された。この節は**上記P21-004本体の§1〜§13を書き換えず**、追補として
末尾に追加する(過去の監査記録を上書きしない)。

## A1. 実装した機能

選択中の自国Divisionに対してのみ、**現在の移動命令だけ**を解除する新規操作
「移動停止」(`map::division_selection::stop_division_movement`)を実装した。既存の前線
スタンス「停止」(`FrontlineCommand::SetStance(FrontlineStance::Stopped)`、国家の前線プラン
**全体**に効く設定で選択に依存しない)とは完全に独立した別機能であり、ボタンラベルも
「移動停止」/"Stop Movement"として明確に区別した。

```rust
// src/map/division_selection.rs
pub(crate) fn stop_division_movement(
    division_id: DivisionId,
    player_cid: CountryId,
    military_registry: &mut MilitaryRegistry,
) {
    let Some(division) = military_registry.divisions.get_mut(&division_id) else { return; };
    if division.owner != player_cid { return; }

    division.destination = None;
    division.current_path.clear();
    division.target_state = None;
    division.movement_progress = 0.0;

    if division.status == DivisionStatus::Moving {
        division.status = DivisionStatus::Idle;
    }
}
```

**仕様§1の各項目の実装結果**:
- `current_state`は変更しない ✓(このフィールドには一切触れていない)
- `destination`/`target_state`を解除、`current_path`を空に、`movement_progress`を0へ ✓
- 移動中(`Moving`)なら`Idle`へ戻す ✓。**それ以外の状態(`Fighting`/`Occupying`等)は
  一切変更しない**ため、戦闘中師団の戦闘を強制終了させることはない
- Army所属(`ArmyRegistry`)には一切触れない(関数はそもそも`ArmyRegistry`を引数に取らない) ✓
- 敵国師団は所有者検証で無視される ✓
- Division選択(`SelectedDivision`)自体はこの関数もそれを呼ぶUIハンドラも変更しない ✓

## A2. UI

`ui/military_panel.rs`に新規コンポーネント`StopMovementButton`/`StopMovementInfoText`を追加し、
前線命令ボタン列とArmyセクションの間に単独の行として配置した(既存の前線「停止」ボタンとは
視覚的にも別の行)。`handle_stop_movement_button`は選択中の`SelectedDivision`全件に対して
`stop_division_movement`を呼ぶだけの薄いハンドラであり、Army一覧のクリックで埋まった選択・
手動クリック/ドラッグ選択のどちらに対しても同一に動作する(スコープを問わない)。UI操作が
マップ操作へ漏れない点は、既存の全ボタンと同じ`Button`+`Interaction`ノードとして実装している
ため、P21-004本体で確認済みの汎用ガード(`map::selection::handle_state_click`/
`map::division_selection::handle_movement_order`の`Query<&Interaction>`走査)がそのまま
適用される(専用の追加対応は不要だった)。

## A3. 追加したテスト(spec §3の1〜11すべてに対応)

**`src/map/division_selection.rs`**(純粋関数レベル、+4件):
1. `stop_division_movement_clears_order_and_resets_moving_to_idle` — spec #1/#8
   (単体停止、current_state不変を同時検証)
2. `stop_division_movement_ignores_foreign_division` — spec #6
3. `stop_division_movement_does_not_change_army_membership` — spec #7
4. `stop_division_movement_does_not_interrupt_fighting_division` — spec #9

**`src/ui/military_panel.rs`**(UI/Systemレベル、+5件):
5. `handle_stop_movement_button_stops_multiple_selected_divisions` — spec #2
6. `handle_stop_movement_button_leaves_unselected_division_unaffected` — spec #5
7. `handle_stop_movement_button_cannot_stop_enemy_division` — spec #6(UI層での多重防御確認)
8. `army_selection_then_stop_movement_stops_all_members` — spec #3/#4
   (Army選択直後に停止操作が機能すること、かつ停止後もArmy所属が変化しないこと(spec #7)を
   同時に検証)
9. `handle_stop_movement_button_does_not_change_frontline_stance` — spec #10
   (前線スタンスを`Offensive`に設定した状態で移動停止を実行し、スタンスが変化しないことを確認)

**spec #11(回帰なし)**: 既存の一括移動(`army_selection_feeds_into_existing_bulk_movement_order`)、
Assign(`army_selection_feeds_into_existing_frontline_division_command`)、Army作成・追加・除外・
解散の全既存テストが今回のセッションでも無変更のまま green であることを、フルテストスイート実行
(§A4)で確認した。

**テスト総数**: 179(lib、170→179) + 59(統合、無変更) = **238**(P21-004完了報告時点の229から+9)。

## A4. 検証コマンドと終了コード

| コマンド | 結果 |
|---|---|
| `cargo fmt --check` | 今回変更したコード(`map/division_selection.rs`/`ui/military_panel.rs`の
新規追加分)に新規差分はゼロ(手動整形済み)。両ファイルに残る少量の差分はP21-004R由来の
プレ既存債務で、今回のP21-004A実装で触れていない箇所(既存フロントラインテスト関数等)のみ |
| `cargo check --all-targets` | 0エラー、exit 0 |
| `cargo test -- --list` | 179 libテストを一覧表示、exit 0 |
| `cargo test`(lib) | **179 passed**; 0 failed、exit 0 |
| 安全な統合テストバイナリ8種 | **59 passed**; 0 failed、exit 0(各々、P21-004完了報告時点と
同一件数、無変更) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 0警告、exit 0 |
| `cargo build --release --all-targets` | 成功、exit 0 |
| `git diff --check` | 既存のLF/CRLF警告のみ、exit 0 |
| 実機`cargo run --release`起動確認 | `[DataLoader] Successfully loaded ... 6 countries, 28
states ...`、パニックなし(タイムアウトによる強制終了 exit 143 は想定通り) |

保護ファイル(`states.ron`/`countries.ron`/`divisions.ron`/`app/time.rs`)は今回も空diffを確認。

## A5. 手動確認が必要な項目(P21-004 §11に追加)

- Army一覧から軍を選択 → 所属Divisionへ一括移動を指示 → 移動途中で「移動停止」を実行 →
  全Divisionが**現在いる州でその場停止**すること(目的地へワープしたり消えたりしない)
- 停止後もArmy所属・Division選択状態の両方が維持されていること
- 選択していない他のDivisionは、移動停止ボタンを押しても移動を継続すること
- 前線パネルの既存「停止」(スタンス)ボタンを押しても、今回の「移動停止」ボタンの見た目
  (有効/無効の色)や表示テキストが連動して変化しないこと(完全に独立していることの目視確認)
- 戦闘中のDivisionを選択して「移動停止」を押しても、戦闘が中断されないこと

## A6. 発見した問題

なし。P21-004本体で発見済みだった2件(§12参照)以外に、今回新たな不具合は見つからなかった。

## A7. 最終判定(更新)

**COMPLETE WITH MANUAL VERIFICATION PENDING**

一括停止(移動停止)が選択Division専用の実装として完了し、自動テスト・回帰確認まですべて green
になったため、INCOMPLETEの状態からは解消された。実機での人手インタラクティブ確認(§11+§A5)が
未実施である点のみが、COMPLETEに至っていない理由として残っている。

## A8. 次タスクの判定: P21-005 か「セーブ/ロード基礎」か

**「セーブ/ロード基礎」への移行を推奨する。P21-005は引き続きREADY(§13）だが、今すぐ着手する
必然性はない。**

理由:
- P21-004(A込み)により、`Division`(個別師団)・`Army`(編成)の両方のデータ構造・不変条件・
  UI操作が安定した。この2層構造は、セーブ/ロードが最初に対象とすべき「保存すべき状態」の
  中核部分そのものである。
- `ArmyId`/`DivisionId`/`FrontlineId`等、全IDが最初からBevy Entityと無関係な
  安定・単調増加のnewtypeとして設計されており(§13で確認済み)、セーブ/ロード実装の技術的
  前提はすでに整っている。逆に言えば、これ以上「保存対象になるデータ構造」を増やす前に
  一度セーブ/ロードの基礎(シリアライズ形式・保存タイミング・互換性方針)を固めておく方が、
  後続のP21-005(前線⇔Army接続)やその先の機能を「最初からセーブ/ロード対応で作る」ことが
  でき、手戻りが少ない。
- P21-005自体は前線への機能追加であり、セーブ/ロードの有無に技術的に依存しない
  (§13で確認した疎結合設計のため、どちらを先にやっても後から差し替え可能)。ただし
  「先にセーブ/ロードの型を固める→その後で前線拡張を型に組み込みながら作る」方が、
  「先に前線拡張→後からセーブ/ロード対応で全部見直す」より手戻りが少ないという実装順序の
  経済性から、セーブ/ロード基礎を先に置くことを推奨する。

ただし、これは実装順序についての推奨であり、最終的な優先順位はユーザーの製品判断による。
