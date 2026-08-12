# P21-003 完了監査 + P21-004 事前調査レポート

日付: 2026-08-11
方針: 監査中に発見した不具合は修正しない(報告のみ)。既存のdirty差分・証拠ファイルは一切上書きしない。
`git checkout`/`reset`/`restore`は使用していない。

証拠: `verification_logs/phase-21/p21-003/`
(00_git_status_before.txt〜09_git_status_after.txt, 各種cargoログ)

---

## 1. P21-003の最終判定

**INCOMPLETE**

理由: ドラッグ選択・単一選択・一括移動命令の中核パスは実装済み・テスト済みで正しく動作する。
しかし、明文化された14項目のうち **3項目が明確に未達**、**1項目が部分達成に留まる**。
「ドラッグ選択だけが実装され一括移動が未実装」という最悪ケースではないが、
「一括“停止”が真の意味では未実装」「選択が自国師団に限定されない」
「消滅師団が選択状態に残り続ける」という、ユーザー指定の完了条件に反する実装済みの不具合がある。

14項目の詳細は下表(§4で根拠を詳述):

| # | 項目 | 判定 |
|---|---|---|
| 1 | ドラッグ範囲内の自国師団だけが選択される | ✗ 未達(敵国師団も選択される) |
| 2 | 敵国師団は選択されない | ✗ 未達(同上、単発クリックも同様) |
| 3 | UI上のドラッグがマップ範囲選択として処理されない | ○ 達成(コード上確認、自動テストなし) |
| 4 | クリックによる単一師団選択が引き続き機能する | ○ 達成(テストあり) |
| 5 | 同一州の複数師団の個別選択と両立する | ○ 達成(テストあり) |
| 6 | 複数師団を選択して同じ移動先を指定できる | ○ 達成(テストあり) |
| 7 | 選択中の全師団へ命令が1回ずつ発行される | ○ 達成(テストあり) |
| 8 | 一部師団が移動不能でも他師団の命令を壊さない | ○ 達成(テストあり) |
| 9 | 一括停止が可能である | ✗ 未達(前線Stoppedは手動移動を止めない。既存テストが仕様として明記) |
| 10 | 消滅師団が選択状態から除去される | ✗ 未達(自動除去の仕組みが存在しない) |
| 11 | 選択解除が正しく機能する | △ 部分達成(空振りドラッグのみ。単発空クリックやボタン/Escapeでは不可) |
| 12 | 選択師団数・選択状態を画面で判別できる | △ 部分達成(表示自体はあるが#10のため数がズレ得る) |
| 13 | 単一選択時の移動・接敵戦闘に回帰がない | ○ 達成(既存テスト全PASS) |
| 14 | 資金・人的資源・募兵処理に影響していない | ○ 達成(該当テスト全PASS、コード上も非接触) |

---

## 2. ドラッグ選択から一括命令までのコード経路

```
map::army_selection::handle_army_selection  (Update, GameState::Playing)
  ├─ just_pressed(Left)  → DragSelectState.press_start_screen をセット、is_dragging=false
  ├─ pressed(Left)       → 移動量がDRAG_THRESHOLD_PX(6px)を超えたらis_dragging=true
  └─ just_released(Left) → ここで確定
       ├─ was_dragging=true:
       │    army_render::army_display_positions() で全師団の表示座標を取得
       │    → 矩形内の ArmyId を収集 (military_registry.armies.contains_key のみでフィルタ、
       │       所有者フィルタなし ★不具合1・2)
       │    → Ctrl未押下ならselected_army.army_ids.clear()、押下時は追加
       └─ was_dragging=false:
            最近傍1師団をヒットテスト → select_only / toggle (Ctrl)

map::selection::handle_state_click (Update, GameState::Playing)
  └─ just_released(Left) かつ drag_state.is_dragging==false のときだけ州クリックとして処理
     (ドラッグ選択と競合しないよう明示的にガード)

map::army_selection::handle_movement_order (Update, GameState::Playing, MouseButton::Right)
  └─ selected_army.sorted_ids() をループし、各IDについて
     try_issue_move_order(army_id, target, ...) を独立実行
     (所有者不一致・戦闘中・撃破済み等は個別にwarn!してスキップ、他師団には影響しない)

ui::military_panel::execute_frontline_assign / execute_frontline_unassign
  └─ 同様に selected_army.sorted_ids() をループして各IDに対し
     frontline_registry.assign_army / unassign_army を独立実行
     (unassign_army自身が所有者を再検証するため、選択に敵軍が混ざっていても
      実害は出ない — が、選択できてしまうこと自体は#1・#2の不具合)

ui::military_panel::update_military_panel_ui (表示のみ)
  └─ selected_army.len() > 1 のとき簡易一覧表示、そうでなければ単一詳細表示
```

「一括停止」に該当するのはUI上「停止」ボタン(`FrontlineCommand::SetStance(Stopped)`)のみで、
これは`war/frontline.rs`の`FrontlinePlan.stance`を変更するだけであり、
`process_stopped_plan`(frontline.rs:851)は前線が自動生成した移動
(`frontline_generated_movements`に含まれるもの)しか止めない。
選択中師団が手動移動中(`ArmyStatus::Moving`、プレイヤーの右クリック移動指令由来)の場合、
停止ボタンを押しても移動は継続する。これは`frontline.rs`内の既存テストが
明示的に仕様として記録している(§4で詳述)。

---

## 3. 自動テストと手動確認の状況

### 自動テスト(今回の監査で実行、コードは一切変更していない)

| 対象 | 結果 |
|---|---|
| `cargo check --all-targets` | ✅ 成功 |
| `cargo test --lib` | ✅ 121 passed / 0 failed |
| `cargo test --test economy_tests` | ✅ 14 passed |
| `cargo test --test diplomacy_tests` | ✅ 5 passed |
| `cargo test --test research_and_politics_tests` | ✅ 9 passed |
| `cargo test --test daily_system_integration_test` | ✅ 6 passed |
| `cargo test --test profile_workload_correctness_test` | ✅ 9 passed |
| `cargo test --test p20_009_hardcoded_string_scan_test` | ✅ 4 passed |
| `cargo test --test p20_009_localization_resource_test` | ✅ 8 passed |
| `cargo test --test land_war_combat_peace_test` | ✅ 2 passed |
| `cargo test --test ui_headless_render_test` | **未実施**(下記理由) |
| `cargo test --test p20_009_localization_headless_render_test` | **未実施**(下記理由) |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ 0 warnings |
| `cargo fmt --check` | ✅ 既知の15行ベースライン差分のみ(`land_war_combat_peace_test.rs`、新規差分なし) |
| `cargo build --release --all-targets` | ✅ 成功 |
| 保護ファイル差分(states.ron / divisions.ron / app/time.rs / war/military_ai.rs / land_war_combat_peace_test.rs) | ✅ 空(無変更) |
| `git status`(監査前後で完全一致) | ✅ 一致 |

**headless描画テスト2件を未実施とした理由**: `ui_headless_render_test.rs`(P20-007)と
`p20_009_localization_headless_render_test.rs`(P20-009)は、実行するたびに
`verification_logs/p20-007/screenshots/`・`verification_logs/p20-009/screenshots/`配下の
コミット済みPNGを直接上書きする既知の問題があり(`[[project-strategy-game-headless-test-output-risk]]`
として記録済み)、出力先を変える設定は現状存在せず、テストファイル自体の変更は
今回の監査対象外・変更禁止のため、安全な代替出力先が用意できない。
そのため「既存証拠を上書きしない」という今回の指示に従い、この2件は未実施として報告する
(P21-003のドラッグ選択機能そのものはこの2テストに依存していないため、判定への影響はない)。

### 手動確認

なし。ユーザーは「実機確認を含めてCOMPLETE」と述べているのはP21-002(師団の直接移動・接敵戦闘)
についてであり、ドラッグ選択機能自体の実機確認ログはこのリポジトリ内に見当たらない
(前回セッションの会話内で「ドラッグの実装はよくできてます」という発言はあるが、
これは一括移動・一括停止・敵軍選択制限などを個別に確認した記録ではなく、
見た目の操作感に対するコメントと解釈するのが妥当)。

---

## 4. 発見した不具合・不足機能(修正はしていません)

### 不具合1・2: 選択が所有者を問わない(敵国師団も選択できる)

`src/map/army_selection.rs`の`handle_army_selection`内、矩形選択(197行目付近)と
単発クリック(214行目付近)のどちらも、ヒットテストの条件は
`military_registry.armies.contains_key(id)`(存在するかどうか)のみで、
`army.owner == player_cid`のような所有者チェックが一切ない。

実害の範囲: 移動命令(`try_issue_move_order`)・前線割当/解除(`assign_army`/`unassign_army`)は
それぞれ内部で所有者を再検証して拒否するため、**選択できても実際に操作されることはない**。
ただし:
- 選択されたユニットの見た目(ゴールドのハイライト)は所有者を区別しないため、
  誤って敵軍を選択しても画面上「選択できている」ように見える。
- `military_panel.rs`の複数選択時の簡易一覧(796〜817行目)も所有者列がなく、
  選択数に敵軍が含まれていても気づけない。

これは新しい不具合ではなく、2026-08-10のCtrl+クリック多重選択実装時点から
**既知・既存の意図的な先送り**としてメモリに記録済み("Still NOT done (explicitly deferred...)")。
今回の監査で「未達成のまま」であることを再確認した、という位置づけ。

### 不具合3: 一括「停止」は手動移動を止めない

`src/war/frontline.rs`の`process_stopped_plan`(851行目)は
`frontline_generated_movements`に含まれる(=前線が自動発行した)移動しか止めない。
`test_manual_vs_frontline_priority`相当のテスト(1610行目台、
「停止命令 (Stopped) を出しても、手動移動経路は解除されない」というコメント付き)が
これを仕様として明記しており、P21-002監査時点から変わっていない。

つまり、プレイヤーが複数師団を選択してドラッグで囲い、右クリックで移動指示を出した後、
それらを「止めたい」と思って唯一存在する「停止」ボタンを押しても、
実際には停止しない(手動移動は前線ボタンの管轄外のため)。
選択中師団への直接的な「移動キャンセル」コマンドはコードのどこにも存在しない。

### 不具合4: 消滅した師団が選択状態から自動的に除去されない

`SelectedArmy.army_ids`(`HashSet<ArmyId>`)を、`MilitaryRegistry.remove_army`実行時や
その他どのタイミングでも整理(prune)するコードが存在しない
(全文検索で確認: `army_ids.retain`等の呼び出しは0件)。

対照的に、`war/frontline.rs::sanitize_references`(223行目)は同種の問題
(前線プランに撃破済み陸軍のIDが残り続ける)を、日次処理の一環として
`plan.assigned_army_ids.retain(...)`・`army_frontline_map.retain(...)`で
明示的に解消しており、**同じコードベース内に「正しい前例」が既に存在する**。

再現手順(コード読解ベース、実行はしていない): 複数師団を選択→戦闘でそのうち1師団が撃破される
→`SelectedArmy.army_ids`にはそのArmyIdが残ったまま
→`military_panel.rs`の`selected_army.len()`(796〜799行目)は撃破前の数を表示し続ける
(表示ループ自体は`military_registry.armies.get(&army_id)`でNoneをスキップするため、
一覧の行数と見出しの数字が食い違って見える)
→ 新たに別のユニットを単発クリックするか、空振りドラッグで選択を全解除するまで直らない。

### 不具合5(軽微): 選択解除の手段が1つしかない

選択を完全に空にする方法は「Ctrl非押下での空振りドラッグ」のみ。
単発の空クリック(何もない場所をクリック)では意図的に解除されない仕様
(コード上のコメント: 「ユニットをクリックしなければ選択を解除しない」)。
Escapeキーや専用の「選択解除」ボタンは存在しない。
致命的ではないが、ユーザーが選択解除の方法に気づきにくい可能性がある。

### 未検証(コード上は問題なさそうだが自動テストがない)

- UI要素の上で開始/終了したドラッグがマップ選択に化けないこと(§1の項目3) —
  コードロジック上は`ui_blocked`チェックにより問題なさそうだが、直接のテストケースがない。
- 同一州にスタックした2師団を、両方とも矩形選択で拾えること — オフセット座標を使う
  `army_display_positions`を矩形選択・単発クリック双方で共有しているため機能するはずだが、
  「同一州スタック × ドラッグ選択」を組み合わせたテストは存在しない。

---

## 5. P21-004の現在コードとの接続点

**⚠️ 命名衝突に関する重要な注意** (詳細は§11 NEEDS USER DECISIONの最重要項目):
現在のコードベースでは`ArmyId`/`ArmyUnit`/`MilitaryRegistry.armies`が
**既に「1個の師団(現在の要求仕様でいう1ユニット)」を指す語として"軍隊"(Army)を使っている**
(`common/mod.rs`のコメント: 「軍隊を一意に識別するID型」)。
P21-004が要求する「複数師団の永続的な集合」を同じ「Army」という言葉で呼ぶと、
既存の`ArmyId`(=1師団)と完全に衝突し、コード・会話の両方で混乱する。
以下の調査では衝突を避けるため、新概念を**仮に「編成(ArmyGroup)」**と呼ぶ。
実装時は`ArmyGroupId`/`FormationId`/`CorpsId`等、ユーザーが選ぶ別名を使う必要がある。

接続点:

- `ArmyUnit`(`src/military/data.rs`)は**Bevy Entityではなく**、`MilitaryRegistry.armies: HashMap<ArmyId, ArmyUnit>`
  に格納された**単なるRustの構造体**。ECSの`Component`/`Entity`は一切使っていない
  (画面表示用に`ArmyVisual{ army_id }`という別の軽量Entityが`army_render.rs`で
  `ArmyId`ごとに同期的にspawn/despawnされているだけで、これは描画専用のプロキシ)。
- 既に`ArmyUnit.combat_id: Option<BattleId>`という「今どの戦闘に参加中か」を指す
  Optionフィールドが存在する。編成所属も同じパターン
  (`ArmyUnit.formation_id: Option<ArmyGroupId>`のような1フィールド追加)で
  自然に表現できる。
- `war::frontline::FrontlineRegistry`が「まさに同種の要求」に対する完成した前例:
  複数陸軍を1つの集合(`FrontlinePlan.assigned_army_ids: Vec<ArmyId>`)にまとめ、
  逆引きマップ(`army_frontline_map: HashMap<ArmyId, FrontlineId>`)で「1陸軍は1前線のみ所属」を
  保証し、日次の`sanitize_references`で撃破済み陸軍を自動除去している。
  P21-004の「1師団は同時に1個の軍だけに所属」という制約は、これと全く同じ形で実装できる。
- `map::army_selection::SelectedArmy`(一時的な複数選択)と、P21-004が要求する
  「永続的な編成」は別物である。「軍を選択すると所属師団を一括選択」は、
  `SelectedArmy.army_ids`を編成の`member_army_ids`で置き換える(または追加する)ことで
  自然に実現できる ― 既存の「選択中の全師団へ命令を1回ずつループ発行する」仕組み
  (`handle_movement_order`・`execute_frontline_assign`等)をそのまま再利用可能。
- `ui/military_panel.rs`は単一の大きなテキストブロックとして軍事パネル全体を描画しており
  (リッチな個別ウィジェットツリーではなく`lines: Vec<String>`を`trf`で組み立てる方式)、
  既存の「前線命令」セクションや「陸軍一覧」セクションと同じパターンで
  「編成一覧」セクションを追加するのが最も一貫性のあるやり方。

---

## 6. 案A・B・Cの比較

### 案A: 編成をResource内のデータとして管理
`ArmyGroupRegistry`という新規`Resource`(`HashMap<ArmyGroupId, ArmyGroup>` + 逆引きマップ)を追加し、
`ArmyGroup`はただのRust構造体。`BattleRegistry`/`FrontlineRegistry`/`WarRegistry`と全く同じ形。

### 案B: 編成自体を独立したEntityとして管理
`ArmyGroup`を`Component`として各編成ごとにBevy `Entity`をspawnし、
所属師団は`Entity`参照や`Children`階層で表現する。

### 案C: 各師団にArmyGroupIdだけを付与し、必要時に集計
`ArmyUnit`に`formation_id: Option<ArmyGroupId>`だけを持たせ、
「この編成の所属師団一覧」は毎回`military_registry.armies.values().filter(...)`で都度集計する
(逆引きマップも編成一覧Registryも持たない)。

| 評価軸 | 案A(Resource) | 案B(独立Entity) | 案C(師団側IDのみ+都度集計) |
|---|---|---|---|
| 実装規模 | 小〜中(既存Registryパターンの複製) | 大(このコードベースに前例のないEntity設計を新規導入) | 最小(フィールド1個追加のみ)だが集計コストが各所に分散 |
| データ整合性 | 高(FrontlineRegistryと同じ手法で「1師団1編成」を一元管理・強制できる) | 中(EntityとArmyId二重管理になり同期ズレのリスク) | 低〜中(逆引きが無いため「軍名」「軍単位の状態」等の付帯情報の置き場がなく、整合性チェックが散在) |
| Bevyとの相性 | 高(既存の全シミュレーション状態がこの形式) | 低(既存コードのどこにも前例がなく、Entity相当のIDをsave/loadする際に確実にハマる=§11参照) | 高(既存パターンと矛盾しないが機能不足) |
| UIからの扱いやすさ | 高(`Res<ArmyGroupRegistry>`を1個読むだけ) | 中(Query経由でEntityを辿る必要があり、既存のテキストパネル方式と相性が悪い) | 低(軍名・軍単位の付帯データを持てないため、UIが求める「軍名表示」「所属師団数表示」等を素直に満たせない) |
| 師団消滅時の安全性 | 高(sanitize_references相当のretainで一元的に対処可能、前例あり) | 中(Entity削除タイミングとArmyUnit削除タイミングの二重管理が必要) | 高(師団が消えれば単にフィルタ結果から消えるだけ、壊れようがない) |
| セーブ/ロード対応 | 高(`Serialize`/`Deserialize`を他の全Registryと同様に derive するだけ) | 低(Bevy `Entity`は世代付きインデックスで再起動間の安定性がなく、そのままでは保存不可。安定IDを別途持たせるなら実質案Aと同じ二重構造になる) | 高(usizeのIDをArmyUnitに1個持たせるだけ) |
| 前線システムへの拡張性 | 高(`FrontlineRegistry`と同じ形なので、将来「編成を前線に割り当てる」も同型で追加できる) | 中 | 低(軍単位の状態を持てないため、前線割当のような「編成そのものに紐づく属性」を持たせにくい) |
| AIとの共有可能性 | 高(プレイヤー/AI問わず同じRegistryを読み書きするだけ) | 中 | 中 |

### 推奨: 案A(Resource内のデータとして管理)

理由: このコードベースは`CountryRegistry`/`StateRegistry`/`MilitaryRegistry`/`BattleRegistry`/
`WarRegistry`/`FrontlineRegistry`/`WarJustificationRegistry`など、**シミュレーション状態を
表す全てのオブジェクトが例外なくResource内のプレーンなRust構造体**であり、Bevy `Entity`は
描画・UIノードなど「画面表示の一時的な写像」以外には一切使われていない、極めて一貫した
アーキテクチャ上の慣習が既にある。案Bはこの慣習を破る初めての例になり、
特にセーブ/ロード(調査事項9で要求されている)との相性が著しく悪い
(Bevy `Entity`は世代付きインデックスであり、アプリ再起動をまたいだ安定識別子には使えない
— この危険性は調査事項3で問われている通り)。
案Cは実装コストは最小だが、「軍名」「軍単位の状態」のような編成そのものに紐づく
付帯データの置き場がなく、UI要件(軍名表示・所属師団数表示)を素直に満たせない。

---

## 7. 推奨データ構造

`FrontlineRegistry`と全く同じ形を踏襲する(型名は仮。実装時にユーザーの選ぶ名称に置換):

```rust
// src/common/mod.rs に追加
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArmyGroupId(pub usize); // 名称は要決定。既存ArmyIdと衝突しない名前にする

// src/military/army_group.rs (新規) あるいは war/army_group.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmyGroup {
    pub id: ArmyGroupId,
    pub owner: CountryId,
    pub name: String,                    // UI表示用の軍名(将来編集可能に)
    pub member_army_ids: Vec<ArmyId>,    // 所属師団(ArmyId昇順で保持し決定性を確保)
    // 将来の前線/攻勢線/指揮官/軍集団への接続点として、
    // 今回は「予約」のみ行い値は入れない(調査事項10・12対応):
    // pub assigned_frontline_id: Option<FrontlineId>,
    // pub commander_id: Option<CommanderId>,
    // pub parent_group_id: Option<ArmyGroupId>, // 軍集団を後付けする場合の親リンク
}

#[derive(Resource, Default, Debug)]
pub struct ArmyGroupRegistry {
    pub groups: HashMap<ArmyGroupId, ArmyGroup>,
    /// ArmyId -> ArmyGroupId (1師団は1編成のみ所属、FrontlineRegistry.army_frontline_mapと同型)
    pub army_group_map: HashMap<ArmyId, ArmyGroupId>,
    next_id: usize,
}

impl ArmyGroupRegistry {
    pub fn create_group(&mut self, owner: CountryId, name: String, member_army_ids: Vec<ArmyId>) -> ArmyGroupId { .. }
    pub fn add_army(&mut self, group_id: ArmyGroupId, army_id: ArmyId, owner: CountryId, military_registry: &MilitaryRegistry) -> Result<(), &'static str> { .. }
    pub fn remove_army(&mut self, army_id: ArmyId) { .. } // 軍からの除外(未所属に戻す)
    pub fn disband(&mut self, group_id: ArmyGroupId) { .. } // 解散、所属師団は全員未所属に戻る
    /// FrontlineRegistry::sanitize_referencesと全く同じ形の日次整理。
    /// 消滅・撃破済み師団をmember_army_ids/army_group_mapから除去する。
    pub fn sanitize_references(&mut self, military_registry: &MilitaryRegistry) { .. }
}
```

`ArmyUnit`自体への変更は不要(`combat_id`と違い、編成所属は「多対1」の関係を
`ArmyGroupRegistry`側の逆引きマップで持てば十分で、`ArmyUnit`に
`formation_id`フィールドを追加する必然性は低い ― FrontlineRegistryが
`ArmyUnit`に`frontline_id`フィールドを追加していないのと同じ理由)。

「1個の師団は同時に1個の軍だけに所属」制約は、`add_army`内で
`army_group_map`に既存エントリがあれば古い方から`member_army_ids`ごと除去してから
付け替える(`FrontlineRegistry::assign_army`の158〜161行目と全く同じロジック)ことで保証する。

---

## 8. 最小UI案

`ui/military_panel.rs`のテキストパネル方式(`lines: Vec<String>`)にそのまま乗せる形が
最も既存コードと一貫する:

- 軍事パネル内に新セクション「編成一覧」を追加(「前線命令」セクションと同様の位置づけ)。
  各行: `[軍名] (所属N師団) [選択中師団から新規作成] [選択中師団を追加/除外] [解散]`
  相当のボタン行、または軍名クリックで「軍を選択→所属師団を`SelectedArmy`へ一括反映」。
- 既存の「陸軍一覧」セクション(956〜979行目)の各師団行に、`frontline_tag`と同じパターンで
  `group_tag`(所属軍名の短縮表示)を追加。
- 新規作成ボタンは、`selected_army`が空でない場合のみ有効化(`RecruitButton`/
  `FrontlineCommandButton`と同じ「フィージビリティ評価関数→ボタン色」パターンを流用)。
- 「軍を選択」操作は、`ArmyGroup.member_army_ids`を`SelectedArmy.army_ids`へコピーする
  だけの1関数で実現でき、その後は既存の一括移動・前線命令の仕組みがそのまま使える。

---

## 9. 変更候補ファイル(実装時の見込み、今回は変更していません)

- `src/common/mod.rs` — `ArmyGroupId`(仮称)ニュータイプ追加
- `src/military/army_group.rs`(新規) — `ArmyGroup`/`ArmyGroupRegistry`本体
- `src/military/mod.rs` — 新モジュール登録、Resource初期化、日次`sanitize_references`の
  登録(`DailySimulationSet::MilitaryAction`または`WarResolution`付近が
  `FrontlineOrders`の`sanitize_references`相当と対になる自然な位置)
- `src/ui/military_panel.rs` — 「編成一覧」セクション追加、新規作成/追加/除外/解散ボタン、
  既存陸軍一覧行への所属表示追加
- `assets/localization/ja-JP.ron` / `en-US.ron` — 新規UI文言の追加(キー集合の一致を
  検証する既存テストがあるため両方同時に追加する必要あり)
- (将来のセーブ/ロード実装時)シリアライズ対象Resourceのリストに`ArmyGroupRegistry`を追加

---

## 10. 必要な自動テスト(実装時の見込み)

- 選択中複数師団から編成を新規作成できる
- 既存の編成へ師団を追加できる
- 編成から師団を除外できる
- 同じ師団を2つの編成に同時所属させようとすると、古い方から自動的に外れる
  (`FrontlineRegistry`の同種テストと対になる回帰テスト)
- 編成を選択すると所属師団が`SelectedArmy`へ反映される
- 編成単位で移動命令を発行すると所属師団全員に命令が届く(既存の複数選択一括命令の
  仕組みをそのまま再利用するため、配線確認レベルのテストで足りる)
- 所属師団が撃破されたとき、次回`sanitize_references`実行後に`member_army_ids`/
  `army_group_map`の両方から消える
- 編成を解散すると所属師団全員が未所属に戻り、`army_group_map`にエントリが残らない
- 空の編成(所属師団0)からの一括移動・一括選択が安全にno-opする
- 他国の編成/師団を自国のUIから操作できない(所有者チェック)
  ― この際、§4の不具合1・2(選択自体が所有者を問わない)を編成側で
  そのまま踏襲しないよう、明示的な回帰テストとして設計しておくことを推奨

---

## 11. NEEDS USER DECISION

1. **【最重要】命名衝突**: 既存の`ArmyId`/`ArmyUnit`は「1師団(現行仕様でいう1ユニット)」を指す。
   P21-004の「軍(Army)」を同じ言葉のまま実装すると型名・変数名が確実に衝突する。
   `ArmyGroup`/`Formation`/`Corps`/`軍団`等、どの呼称にするか決定が必要
   (本レポートでは暫定的に「ArmyGroup」を使用)。
2. §4で報告した不具合1・2(選択が自国師団に限定されない)を、P21-004実装の**前提として先に修正するか**、
   それとも編成機能実装後にまとめて修正するか。編成のメンバー追加(「選択中の師団から編成を作る」)は
   `SelectedArmy`を入力にするため、この不具合を放置すると「敵軍を編成に誤って加えようとする」
   UIパスが生まれ得る(実行時に所有者チェックで弾かれるので実害は限定的だが、
   選択できてしまうこと自体はP21-003の不具合の再発になる)。
3. §4の不具合3(一括停止が手動移動を止めない)・不具合4(消滅師団が選択に残り続ける)を、
   P21-003の追加修正として今すぐ着手するか、それとも別タスクとして切り出すか。
4. P21-004の最小範囲案(§本文)には「軍名の表示」が含まれるが、「軍名の編集(リネーム)」は
   含まれていない。名前は作成時に自動採番(例:「第1軍」「第2軍」)で固定するか、
   後からの改名も最小範囲に含めるか。
5. AI国家にも同じ`ArmyGroupRegistry`を使わせるか(調査事項12)。今回のスコープ外
   (「AIによる軍編成」は除外)とのことだが、**データ構造自体は**プレイヤー/AI共有前提で
   設計してよいか、それともプレイヤー専用のResourceとして始めるか
   (後から共有前提に拡張するのは案Aの構造であれば容易だが、念のため確認)。

---

## 12. P21-004実装可否

**READY WITH DECISIONS**

データ構造(案A)・所属の一元管理・消滅師団への対処・複数選択との接続・
最小UIの置き場は、いずれも`FrontlineRegistry`という完成した前例をそのまま
踏襲できるため技術的な障壁はない。ただし着手前に§11の5項目、
特に**命名衝突(項目1)**の決定が必須(決定なしに実装を始めると、
型名を後から全面リネームする手戻りが確実に発生する)。
