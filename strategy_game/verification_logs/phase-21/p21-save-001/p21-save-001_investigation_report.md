# P21-SAVE-001: セーブ/ロード基礎 事前調査報告

**実施日**: 2026-08-12
**性質**: 調査のみ。ゲームコード・アセット・既存テストは一切変更していない(`git status`で
本タスク開始前と同一のdirty差分のみであることを確認済み)。

---

## 1. 現在のセーブ/ロード実装状況

**セーブ/ロード機構は存在しない。** `src/app/loader.rs`は以下の理由により、セーブ/ロードとは
性質が異なる**静的マスターデータの起動時読み込み**専用モジュールである:

- `load_game_data`は`Startup`で一度だけ実行され、`assets/data/{buildings,technologies,
  world_stages,diplomacy,resources,divisions,countries,states}.ron`を読み込む
- これらのRONファイルは**設計データ**(ゲームデザイナーが編集する初期配置・定義)であり、
  プレイヤーの操作結果を反映しない。何度起動しても同じ内容を読む
- 読み込み失敗時は`panic!`する設計(起動時の設定ミス検出が目的で、ユーザー操作起因の
  ファイル欠損を想定した安全な失敗処理ではない)
- ロード失敗時のリトライ・スロット選択・バージョン互換性などの概念が一切ない

`ron`/`serde`は既にCargo依存として存在し(`Cargo.toml`)、多数の型が`Serialize`/
`Deserialize`を実装済みである(§6以降で詳述)。**技術的な前提は既に大部分揃っている**。

---

## 2. New GameとWorld初期化経路

実際のコードから確認した起動シーケンス:

```
Startup:
  load_game_data (静的マスターデータをRONから読み込み、CountryRegistry/StateRegistry等に反映)
  → transition_to_country_selection (即座に GameState::MainMenu → CountrySelection)

GameState::CountrySelection:
  プレイヤーが国家を選択し、開始ボタンを押す
  → handle_start_button (src/ui/country_selection.rs:286)
    - PlayerCountry.0 = Some(selected_id)
    - next_state.set(GameState::Playing)

OnEnter(GameState::Playing):
  spawn_debug_divisions (各国の首都にInfantry 1個師団を配置)
  setup_map (州スプライトの生成)
  setup_camera 等の各種UI初期化
```

**重要な発見**: `GameState`は`MainMenu → CountrySelection → Playing`の**一方通行**であり、
`Playing`から`CountrySelection`や`MainMenu`へ戻る遷移はコード中に一切存在しない
(`grep`で確認: `NextState<GameState>`を書き込む箇所は`loader.rs`と`country_selection.rs`の
2箇所のみ)。つまり:

- 現在「New Game」を再度行う手段がない(プロセス再起動が必須)
- 「ロード画面からゲームを始める」フローも「プレイ中にロードする」フローも、**今の状態遷移には
  組み込む先がない**。ロード機能を追加する場合、(a) `Playing`状態のまま内部データだけを
  安全に差し替える方式、または(b) 新しい`GameState`(例: `Loading`)を追加する方式のどちらかを
  選ぶ必要がある(§9で最小実装として(a)を推奨)

`spawn_debug_divisions`は`OnEnter(GameState::Playing)`にひも付いているため、**ロード時に
Playing状態への遷移を再度発火させると、ロードしたセーブデータに加えて初期師団が二重に
配置されてしまう**危険がある(方式(a)を取ればこの問題自体を回避できる)。

---

## 3. 保存対象一覧(正規データ)

実コードから確認した、**保存すべき正規データ**の一覧。全て実際に存在するフィールド名。

### 3-1. 世界と進行

| データ | 型・場所 | Serde対応 |
|---|---|---|
| ゲーム内日付 | `GameDate { year, month, day, accumulator }` (`app/time.rs:47`) | ❌ 未対応(`Resource, Debug, Clone, PartialEq`のみ) |
| 一時停止状態 | `GamePaused(bool)` (`app/time.rs:171`) | ❌ 未対応 |
| ゲーム速度 | `GameSpeed(u8)` (`app/time.rs:154`) | ❌ 未対応 |
| プレイヤー国家 | `PlayerCountry(Option<CountryId>)` (`country/mod.rs:253`) | ❌ 未対応 |
| RNG状態/乱数シード | **存在しない**。`combat_calc.rs`冒頭コメント通り「乱数を使わず、整数演算で決定的な戦闘計算を行う」設計であり、AI判断も含め全シミュレーションが決定的。保存不要 |
| 次回ID発行値 | 各レジストリの`next_id`系フィールド(§6で一覧化) | レジストリの対応状況に従う |
| 世界文明段階の進行 | `WorldCivilizationState.current_stage`/`milestone_countries`/`last_advanced_date` (`research/world_stage.rs:19`) | ❌ 未対応。**ただし同じ構造体内の`stage_definitions`は静的マスターデータであり保存対象外**(§5) |

### 3-2. 国家・州

| データ | 型・場所 | Serde対応 |
|---|---|---|
| 国家データ全体 | `CountryData`(`country/mod.rs:83`) — 資金・人的資源・税率・経済/政治/研究状態・建設キュー・募兵キューを含む | ✅ 完全対応(`countries.ron`読み込みで実証済み) |
| 州データ全体 | `StateData`(`state/data.rs:13`) — 所有国・支配国・占領進捗・人口・建物・隣接等 | ✅ 完全対応(`states.ron`読み込みで実証済み) |
| 国家色/州色 | `CountryData.map_color`のみが正規データ。**州の表示色は保存しない**(§5参照、`state.controller()`から`update_state_colors_on_controller_change`が毎フレーム再計算する派生値) |

### 3-3. 外交・戦争

| データ | 型・場所 | Serde対応 |
|---|---|---|
| 二国間関係 | `DiplomaticRelation`(`diplomacy/data.rs:86`)、`DiplomacyRegistry.relations: HashMap<DiplomaticPairKey, _>` | 要素は✅、レジストリ自体は❌ |
| 戦争正当化 | `WarJustification`/`WarJustificationRegistry`(`war/justification.rs`) | ✅ **レジストリごと**完全対応 |
| 戦争 | `War`(`war/data.rs:22`)、`WarRegistry.wars` | 要素は✅、レジストリ自体は❌ |
| 前線・作戦命令 | `Frontline`/`FrontlinePlan`/`FrontlineRegistry`(`war/frontline.rs`) | ✅ **レジストリごと**完全対応 |
| 領土請求(未使用機能) | `TerritorialClaim`/`ClaimRegistry`(`diplomacy/claims.rs`) | 要素は✅、レジストリ自体は❌。**`add_claim`は`diplomacy/tests.rs`以外から一切呼ばれておらず、実プレイでは常に空**(grep確認済み) |
| 外交危機(未使用機能) | `DiplomaticCrisis`/`CrisisRegistry`(`diplomacy/crisis.rs`) | 同上。実プレイでは常に空 |
| 国家AI状態 | `CountryAiState`/`CountryAiRegistry`(`country/country_ai.rs:146`) | ✅ **レジストリごと**完全対応 |
| 軍事AI状態 | `MilitaryAiState`/`MilitaryAiRegistry`(`war/military_ai.rs:80`) | ✅ **レジストリごと**完全対応 |

### 3-4. Division(個別師団)

`Division`(`military/data.rs:74`)は既に**フィールド完全**に`Serialize`/`Deserialize`対応済み。

| フィールド | 保存要否 |
|---|---|
| `id`(DivisionId) | 必須 |
| `owner` | 必須 |
| `current_state`/`status`/`destination`/`target_state`/`current_path`/`movement_progress` | 必須(P21-004Aで実装した移動停止状態も含め、これらが移動中断・戦闘参加の全状態を表す) |
| `manpower`/`max_manpower`/`equipment`/`max_equipment`/`organization`/`max_organization`/`morale`/`max_morale`/`experience`/`supply_ratio` | 必須(能力値・消耗状態) |
| `def_id`/`attack_power`/`defense_power` | 必須(定義参照+実数値のスナップショット。定義自体は静的マスターデータだが、`attack_power`/`defense_power`は`def_id`から都度算出せず個体に保持する設計のため、この2値自体を保存する必要がある) |
| `combat_id` | 必須(`Some`なら戦闘中。§5で戦闘中セーブの扱いを検討) |
| `MilitaryRegistry.next_division_id` | 必須(ID再利用防止) |
| Bevy Entityとの対応 | **なし**。`Division`はプレーンなデータであり、対応するBevy Entityは存在しない(§7で詳述) |

### 3-5. Army(編成)

`ArmyRegistry`(`military/army.rs:37`)は**既にリソースごと**`Serialize`/`Deserialize`対応済み。

| フィールド | 保存要否 |
|---|---|
| `armies: HashMap<ArmyId, Army>`(`id`/`owner`/`name`/`member_division_ids`) | 必須 |
| `division_army_map` | 必須(逆引きマップ、`armies`から再構築も可能だが既にSerde対応済みのため素直に含めてよい) |
| `next_id` | 必須 |
| `next_army_number`(国家ごとの命名カウンタ) | 必須(ロード後に作成した新規Armyの名前が既存と衝突しないため) |
| 作成順/表示順 | `ArmyId`が単調増加のため、`ArmyId`昇順が作成順そのもの。別途保存する表示順フィールドは存在しない(§6で詳述) |
| 選択状態(`SelectedArmy`) | **保存しない**(§3-6/§5参照、UI状態) |
| 将来の前線割り当て用データ | 現時点で`Army`は`FrontlineRegistry`から一切参照されていない(§11で詳述) |

### 3-6. UI・表示(保存しないデータの一覧は§4)

---

## 4. 保存しないデータ一覧(一時的・UI上のデータ)

実コードで確認した、**保存すべきでない**データ:

| データ | 型・場所 | 理由 |
|---|---|---|
| 選択中Division | `SelectedDivision`(`map/division_selection.rs:16`) | UI選択状態。ロード後は空にすべき(既存の`prune_selected_division`もmilitary_registry変化時に毎回内容を検証し直す設計であり、永続化を前提としていない) |
| 選択中Army | `SelectedArmy`(`military/army.rs:33`) | 同上。`update_army_ui`が毎フレーム有効性を再検証する設計(P21-004実装済み) |
| 選択中州 | `SelectedState`(`state/mod.rs:11`) | 同上 |
| 開いているパネル | `MilitaryPanelState`/`DiplomacyPanelState`/`PeacePanelState`/`PoliticsPanelState`/`ResearchPanelState`/`ActivePanel`(各`ui/*.rs`) | 単純な開閉bool/enum。ロード後は全て閉じた状態に初期化すべき |
| カメラ位置・ズーム | `GameCamera`マーカーが付いた`Entity`の`Transform`(`map/camera.rs`) | **Resourceではなく描画Entity側のComponent**。`Startup`で`setup_camera`により毎回新規生成される。保存する場合は別途Resourceへコピーする実装が必要(今回は見送りを推奨、§9) |
| ホバー状態 | 各パネルの`Interaction`はBevy標準コンポーネントで、フレームごとに再計算される | 保存不可能かつ不要 |
| 州・師団の描画Entity | `StateVisual`/`DivisionVisual`/`DivisionVisualCluster`等(`map/rendering.rs`/`map/division_render.rs`) | `OnEnter(Playing)`や毎フレーム更新で再生成される描画専用Entity。§7で詳述 |
| 通知履歴 | `NotificationHistory`(`ui/notification.rs:11`) | 表示済みログの文字列リスト。ゲームプレイに影響しない。保存しなくてもゲームの再現性に問題はない |
| AI宣戦布告の通知キュー | `PendingAiWarDeclarations`(`country/country_ai.rs`) | 単一フレーム内で生成・消費される一時キュー。次フレームには必ず空になる |
| 州の表示色 | 保存不要(派生データ、§5) |
| RNGシード | 存在しない(決定的シミュレーション) |

**Armyの選択状態とカメラ位置をロード後に復元すべきか**という論点について: 現在の設計は
「選択状態は毎フレーム`MilitaryRegistry`/`ArmyRegistry`と照合して有効性を検証し直す」という
一貫した方針(`prune_selected_division`、`update_army_ui`の`SelectedArmy`再検証ロジック)を
既に持っている。ロード直後に古い選択IDが残っていても実害はない(存在しないIDは次のフレームで
自動的に無効化される)が、**保存しない方が実装がシンプルであり、既存の「毎フレーム検証」思想とも
整合する**。カメラについても、ロード直後にプレイヤーが手動で視点を合わせ直すコストは低く、
Resourceでない(Entity Component)ため保存の実装コストの方が高い。両方とも**V1では保存せず、
ロード後にデフォルト状態へ初期化する**ことを推奨する。

---

## 5. 派生データと再構築方法

**州色から再計算できる州色を、独立した正規データとして二重保存しない構造**については、
今回のセッションで実装済みの`update_state_colors_on_controller_change`
(`map/rendering.rs`)が既に「`StateRegistry`が変化したら`state.controller()`から
`CountryRegistry`の色を引いて再計算する」という**派生値として実装されている**ため、
追加の設計判断は不要である。ロード時に`StateRegistry`を`ResMut`経由で書き換えれば
(`state_registry.is_changed()`が真になる)、このシステムが次フレームで自動的に
全州のスプライト色を再計算する。**州色は保存対象に含めない。**

その他の派生/再構築可能データ:

| 派生データ | 再構築方法 |
|---|---|
| `StateRegistry.index_map`(StateId→Vecインデックス) | `StateRegistry::build(states)`が毎回再構築する(既存コンストラクタ、`loader.rs`の初期読み込みと同一経路が使える) |
| Division/Armyの表示クラスタ(`DivisionVisualCluster`) | `division_visual_clusters()`が`MilitaryRegistry`+`ArmyRegistry`から毎フレーム計算する純粋関数。保存不要 |
| 前線境界線描画 | `FrontlineRegistry`から毎フレーム再計算(`frontline_render.rs`) | 保存不要 |
| UIパネルのテキスト内容 | 各`update_*_ui`系システムが毎フレーム(または該当リソース変化時)再構築 | 保存不要 |
| `military_ai`/`country_ai`の`dirty`フラグ | 実は`MilitaryAiState.dirty`/`CountryAiRegistry`はキャッシュ無効化フラグであり、ロード直後は**保守的に`true`(全員dirty)にリセットする方が安全**(次の評価タイミングで確実に再評価される。現状値をそのまま保存してもよいが、`true`に上書きする方が「ロード後の初回評価漏れ」を防げる) |

**この設計の強み**: このコードベースは「毎フレーム/変化時に正規データから再計算する」慣習が
既に徹底されており(P21-004でのApp一覧UI、今回のstate色更新も同じ思想)、**キャッシュ済み
派生データをResourceとして保持しているケースがほぼ存在しない**。これはセーブ/ロード実装の
複雑さを大きく下げる、既存アーキテクチャの実質的な利点である。

---

## 6. 全永続ID一覧

`src/common/mod.rs`で定義される、全ID型(全て`usize`ラップの newtype、全て
`Serialize`/`Deserialize`対応済み、**全てBevy Entityと無関係**):

| ID型 | 発行場所 | 一意性の範囲 | 次回発行値の管理 | Serde | Bevy Entity依存 | ロード後も同一性維持可能か | 欠損参照検出 |
|---|---|---|---|---|---|---|---|
| `CountryId` | `assets/data/countries.ron`(設計時に手動採番) | ゲーム全体 | 発行カウンタなし(静的データの一部) | ✅ | なし | ✅(RONの値をそのまま保存・復元) | `country_registry.get(id)`がNoneで検出可 |
| `StateId` | `assets/data/states.ron`(設計時に手動採番) | ゲーム全体 | 同上 | ✅ | なし | ✅ | `state_registry.get(id)`がNoneで検出可 |
| `DivisionDefinitionId` | `assets/data/divisions.ron`(設計時に手動採番) | ゲーム全体 | 同上(静的マスターデータ、セーブ対象外) | ✅ | なし | ✅(RONから毎回同じ値が復元される) | `military_registry.definitions.get(id)`で検出可 |
| `DivisionId` | `MilitaryRegistry::add_division`が`next_division_id`から発行 | ゲーム全体 | `MilitaryRegistry.next_division_id: usize`(非pub、**要セーブ**) | ✅ | **なし**(§7で確認済み) | ✅(`next_division_id`を保存すればID重複なく再開できる) | `military_registry.divisions.get(id)`で検出可 |
| `ArmyId` | `ArmyRegistry::create_army`が`next_id`から発行 | ゲーム全体 | `ArmyRegistry.next_id: usize`(非pub、**要セーブ**。既にレジストリごとSerde対応のため自動的に含まれる) | ✅ | なし | ✅ | `army_registry.armies.get(id)`で検出可 |
| `WarId` | `WarRegistry::add_war`が`next_id`から発行 | ゲーム全体 | `WarRegistry.next_id: usize`(非pub、**要セーブ**) | ✅ | なし | ✅ | `war_registry.wars.get(id)`で検出可 |
| `BattleId` | `BattleRegistry::start_battle`が`next_id`から発行(P21-siegeで確認済み) | ゲーム全体 | `BattleRegistry.next_id: usize`(非pub、**要セーブ**) | ✅ | なし | ✅ | `battle_registry.battles.get(id)`で検出可 |
| `FrontlineId` | `FrontlineRegistry::generate_id`が`next_frontline_id`から発行 | ゲーム全体 | `next_frontline_id`(pub、既にレジストリごとSerde対応) | ✅ | なし | ✅ | `frontline_registry.frontlines.get(id)`で検出可 |
| `DiplomaticCrisisId` | `CrisisRegistry::add_crisis` | ゲーム全体 | `next_id`(非pub) | ✅ | なし | ✅(ただし実プレイでは常に未使用) | 検出可 |
| `ClaimId` | `ClaimRegistry::add_claim` | ゲーム全体 | `next_id`(非pub) | ✅ | なし | ✅(同上、常に未使用) | 検出可 |
| `TreatyId` | **定義のみ存在し、コード中で一切使用されていない**(grep確認: `common/mod.rs`以外に出現なし) | - | - | ✅ | なし | - | - |

**BevyのEntityを永続IDとして保存する設計は存在しない。** `Division`/`Army`/`War`/`Battle`/
`Frontline`/`CountryData`/`StateData`のいずれも、対応するBevy Entityを持たない(§7で全数確認)。
このコードベースは元々「シミュレーション状態はプレーンなResource内データ、Bevy Entityは
描画/UI専用」という設計方針を一貫して守っており(P21-004投資調査で既に確認済み、今回の調査で
再確認)、「安定ID → 新しいEntityの対応表」を構築する必要のあるケースは**一切存在しない**。

**次回発行値のうち、`pub`でないためレジストリ自体をSerde対応させない限り保存されないもの**:
`MilitaryRegistry.next_division_id`、`WarRegistry.next_id`、`BattleRegistry.next_id`、
`ClaimRegistry.next_id`、`DiplomaticCrisisRegistry`の`next_id`。**これらのレジストリに
`Serialize`/`Deserialize`を追加する際は、privateフィールドもderiveマクロにより正しく
シリアライズされる**(Rustの可視性はコンパイル時の呼び出し制限であり、同一モジュール内で
展開されるderiveマクロは非pubフィールドへも問題なくアクセスできる)。

---

## 7. Bevy Entity依存一覧

`Entity`/`Option<Entity>`/`Vec<Entity>`/`HashMap<Entity, _>`/`EntityHashMap`等の使用箇所を
全文検索した結果、該当は以下10ファイルのみ:

`ui/military_panel.rs`、`map/division_selection.rs`、`map/division_render.rs`、
`ui/peace_panel.rs`、`ui/diplomacy_panel.rs`、`ui/research_panel.rs`、`ui/politics_panel.rs`、
`ui/country_selection.rs`、`ui/top_bar.rs`、`map/frontline_render.rs`

個別に用例を確認したところ、**全て以下のパターンのいずれかであり、Resource/Componentの
フィールドとして長期保持されているEntityは1件も存在しない**:

1. `Query<Entity, With<SomeMarker>>`を使った「既存の子要素を全破棄してから再構築する」
   使い捨てクエリ(例: `map/division_selection.rs:637`の`to_despawn: Vec<Entity>`、
   `ui/diplomacy_panel.rs`のパネル再構築、P21-004で追加した`ArmyListContainer`の子破棄も同型)
2. `Query<(Entity, &XxxVisual)>`のように、そのシステム内だけで使われクエリ結果を
   その場で処理する一時変数(例: `map/division_render.rs:50`)

**分類結果(仕様書の5分類に対応)**:

| 分類 | 該当 |
|---|---|
| 1. 保存不要な表示Entity | 上記全て(`StateVisual`/`DivisionVisual`/`DivisionVisualCluster`/`FrontlineOverlayVisual`/各UIパネルのルートEntity等) |
| 2. ロード時に再spawnするゲームEntity | 該当なし(そもそも「ゲームEntity」という概念がこのコードベースに存在しない) |
| 3. 安定IDへ変換して保存する必要がある参照 | 該当なし |
| 4. セーブ/ロードを阻害する危険な依存 | **該当なし** |
| 5. 意味不明・複数用途混在 | 該当なし |

**結論**: Entity依存に起因するセーブ/ロードの技術的障害はない。これは本調査で最も重要な
ポジティブな発見であり、実装の複雑さを大きく下げる。

---

## 8. セーブSnapshotを取得する推奨タイミング

### 8-1. 危険な取得タイミングの分析

`DailySimulationSet`(`app/time.rs:5`)は以下の順序で`.chain()`されている
(`run_if(in_state(GameState::Playing))`):

```
TimeUpdate → Economy → Research → Diplomacy → CountryAi → WarPreparation
→ MilitaryAi → FrontlineOrders → MilitaryAction → WarResolution → UiUpdate
```

`MilitaryAction`セット内では`handle_daily_military`(移動処理・戦闘計算・戦闘決着処理を
含む)の直後に`handle_daily_army_maintenance.after(handle_daily_military)`
(Army整合性の日次清掃、P21-004で確認済み)が続く。この間の**途中**でSnapshotを取ると、
例えば「戦闘は決着したがArmyの空編成清掃がまだ」「講和で州の所有権は移ったが
`update_state_colors_on_controller_change`(通常のUpdateスケジュール、Setに属さない)が
未実行」といった**過渡的な不整合状態**を保存してしまう危険がある。

ただし重要な事実として、**`handle_daily_military`をはじめとする`DailySimulationSet`内の
実処理は、すべて`DayChangedMessage`を読み取った場合のみ実行される**(`for event in
day_events.read() { ... }`という実装パターンを全箇所で確認)。`DayChangedMessage`は
`advance_game_date`(`app/time.rs:223`)が`!paused.0`の場合のみ発行するため、
**`GamePaused(true)`の間は`DailySimulationSet`のいずれの処理も実質的に何もしない**
(イベントが空なのでループ本体が走らない)。

一方、UI操作由来の即時処理(移動命令の発行、Army作成、前線コマンド、募兵ボタン等)は
`GamePaused`と無関係に、通常の`Update`スケジュールで毎フレーム動作する。ただしこれらは
全て`ResMut<T>`への直接書き込みであり(BevyのCommand経由の遅延Entity生成ではない)、
**1つのシステム関数の実行が完了した時点で、そのシステムが書き込んだ内容は完全に反映済み**
になる(Commandのような「次のCommand適用ポイントまで反映されない」型の遅延は存在しない)。
つまりこのコードベースには「Commandsが未適用の途中状態」という懸念事項自体が実質的に
発生しない(Entity spawn/despawnが絡む描画/UIコードでは起こり得るが、ゲームロジックの
Resourceには起こらない)。

### 8-2. 比較

| 方式 | 評価 |
|---|---|
| セーブ要求を受けた同フレームで即座に保存 | **リスクあり**。要求を受けたシステムの実行順序が`DailySimulationSet`のどこに位置するか次第で、同一フレーム内の「まだ実行されていない後続システム」の結果を含められない可能性がある |
| Update終了後の専用SystemSetで保存 | 有力。ただし「Update終了後」を`DailySimulationSet`の最後(`UiUpdate`)の**後**に明示的に順序付ける必要がある |
| FixedUpdateまたは日次処理完了後に保存 | 本プロジェクトは`FixedUpdate`を使用しておらず(`app/time.rs`は独自の`accumulator`方式)、新しい概念を持ち込むコストがある。「日次処理完了後」は上記と実質同じ効果を、既存の`PostUpdate`スケジュールで代替できる |
| セーブ要求時に一時停止し、Commands適用後に保存 | 一時停止自体は`GamePaused(true)`にするだけで良い(既存の仕組みをそのまま使える)が、Commands適用待ちは§8-1の通りこのコードベースでは本質的に不要 |

### 8-3. 推奨

**Bevyの`PostUpdate`スケジュールに、セーブ実行システムを登録する。** `PostUpdate`は
Bevy標準のスケジュールで、その回のフレームの`Update`スケジュール(`DailySimulationSet`の
全チェーンを含む)が完全に終了した後にのみ実行される。これにより:

- 明示的な`.after(...)`指定を`DailySimulationSet`の全メンバーに対して行う必要がない
  (スケジュール自体が後続保証する)
- ポーズ中・非ポーズ中のどちらでプレイヤーがセーブを要求しても、その要求を受けた
  フレームの全ゲームロジックが完全に確定した状態を保存できる
- 「セーブ要求」自体はResource(例: `SaveRequest`のような1フレームのみ有効なフラグ/Message)
  として`Update`側のUIボタンハンドラから発行し、`PostUpdate`側の保存システムがそれを
  読んで実行する、という単純な2段構成にできる

**戦闘・移動途中でのセーブについて**: §8-1の分析通り、`PostUpdate`のタイミングであれば
「州境界データ不整合」のような技術的な壊れは起きない(Division/War/Battleの各フィールドは
それぞれ完結した値を持つ)。ただし「移動命令の残り経路が中途半端」「戦闘が数日分進行中」
という状態そのものは**正常な保存対象データ**であり(§9で詳述)、技術的な問題ではない。

---

## 9. ロード時の復元順序

実コードの依存関係(どのRegistryが他のRegistryのデータを参照して検証するか)に基づき設計。

```
1. ファイル読み込み(std::fs::read_to_string)
2. RONパース(ron::from_str::<SaveGameV1>) — 失敗したら即座に拒否、現在の状態には一切触れない
3. バージョン検証(SaveGameV1.version フィールドを確認) — 不一致なら拒否
4. 全参照整合性の事前検証(§13で詳述、loader.rs::validate_dataと同型の設計):
   - 全Country/State/Division/Army/War/Battle/FrontlineのIDに重複がないか
   - 各Divisionのownerが実在するCountryIdか
   - 各DivisionのArmy所属(あれば)が実在するArmyIdで、かつそのArmyのmember_division_idsに
     自分自身が含まれているか(相互参照の整合性)
   - 各WarのattackersP/defendersが実在するCountryIdか
   - 各FrontlinePlanのassigned_division_idsが実在するDivisionIdか
   - 検証は「読み取り専用」で行い、1件でも失敗したらロード全体を中止する
5. (検証成功後、ここから初めて現在のPlaying状態を書き換え始める)
   現在のプレイ用Resourceを安全にリセット:
   - CountryRegistry/StateRegistry/MilitaryRegistry.divisions/ArmyRegistry/WarRegistry/
     DiplomacyRegistry/FrontlineRegistry/WarJustificationRegistry/CountryAiRegistry/
     MilitaryAiRegistry/BattleRegistry を Default::default() に戻す
   - SelectedDivision/SelectedArmy/SelectedState を空にする(§4)
   - 静的マスターデータ(BuildingRegistry/TechnologyRegistry/WorldCivilizationState.
     stage_definitions/MilitaryRegistry.definitions)は**再読み込みしない**
     (Startup時点で既に読み込み済みで、プレイヤー操作の影響を受けないため)
6. GameDate/GamePaused/GameSpeed/PlayerCountryを復元
7. CountryRegistry.countriesを復元
8. StateRegistry::build(states)で復元(既存コンストラクタを再利用、index_mapを自動再構築)
9. DiplomacyRegistry.relations、WarJustificationRegistryを復元
10. WarRegistry.wars + next_idを復元
11. MilitaryRegistry.divisions + next_division_idを復元
    (`add_division`のような新規ID発行経路は使わず、保存されていたDivisionIdをそのまま
    HashMapへ直接挿入する専用ロード経路が必要)
12. BattleRegistry.battles + next_idを復元
13. ArmyRegistry全体を復元(既にSerde対応済みのため、DTOへそのまま格納→そのまま復元で済む)
14. FrontlineRegistry全体を復元(同上)
15. CountryAiRegistry/MilitaryAiRegistryを復元。**ただし全`dirty`フラグをtrueへ上書き**
    (§5の通り、ロード直後の保守的な再評価を保証するため)
16. 欠損参照の最終検出(ステップ4の事前検証と基本的に同一のチェックをロード後の実データに
    対しても再実行し、万一の不整合を検出する。通常はステップ4で防がれるため到達しないはず)
17. 派生キャッシュの再構築 — **本コードベースには明示的な再構築が必要な派生キャッシュが
    存在しない**(§5)。`StateRegistry::build`のindex_map再構築のみが該当し、これはステップ8で
    完了済み
18. 州色など描画状態の再計算 — 何もしなくてよい。ステップ8の`StateRegistry`書き換えにより
    `state_registry.is_changed()`が真になり、既存の`update_state_colors_on_controller_change`
    が次フレームで自動的に全州を再着色する
19. UIの再構築 — 何もしなくてよい。`update_army_ui`等の既存システムは`MilitaryPanelState.open`
    または`Res<T>::is_changed()`に応じて次フレームで自動的に再描画する。UIパネルの開閉状態自体は
    §4の通り保存しないため、ロード後は全パネル閉じた状態から始まる
20. ゲーム進行の再開 — `GamePaused`は**保存されていた値に関わらず`true`で開始することを推奨**
    (ロード直後にプレイヤーが状況を確認する時間を確保するため。既存の初期値`GamePaused(true)`
    (`app/time.rs:182`)とも一貫する)
```

**ロード失敗時に現在のゲーム状態を半壊させない方法**: ステップ2〜4を**「現在のPlaying状態に
一切書き込まない、読み取り専用の検証フェーズ」**として明確に分離することで実現する。
ステップ5(実際の書き込み開始)に到達するのは、ファイル形式・バージョン・全参照整合性の
検証が**すべて**通過した場合のみ。これはRustの型システムと相性が良く、「検証済みDTO」を
表す型(例: `ValidatedSaveGame`)を経由させ、検証を経ていない`SaveGameV1`から直接
Resourceへ書き込む経路をコンパイル時に作れないようにする設計が可能(§18の実装タスク分割で詳述)。

---

## 10. 推奨ファイル形式

**RON**を推奨する。理由:

- 既存の全静的マスターデータ(`assets/data/*.ron`)、既存の全Serialize/Deserialize対応型
  (`Division`/`War`/`FrontlinePlan`等)が既にRON前提で設計・テストされている
  (`ron::from_str`のエラーメッセージ処理パターンが`loader.rs`に確立済み)
  ため、追加の依存関係が不要
- JSONと比較して、Rustの`enum`(`DivisionStatus`/`WarStatus`/`FrontlineStance`等、
  本コードベースでは多数使用)をより自然に(かつ既存のRONファイルと同じ書式で)表現できる
- バイナリ形式(bincode等)は人間が調査・デバッグしづらく、新規依存クレートが必要になる。
  Prototype v0.2の段階でデバッグ容易性を犠牲にする理由がない

**ランタイム構造体を直接保存しない**方針を推奨する。理由は本調査の§3〜6で確認した通り、
各Registryの中には「静的マスターデータ(再読み込みすればよい)」と「セーブすべき可変状態」が
混在するケース(`MilitaryRegistry.definitions` vs `divisions`、`WorldCivilizationState.
stage_definitions` vs `current_stage`)が実在するため、**Resourceへ機械的に`Serialize`を
追加してWorld全体をダンプする方式は、静的データの二重保存や、UI選択状態のような
保存すべきでないデータの混入を招く**。専用のSave DTO(`SaveGameV1`)を型として明示的に
定義し、各Resourceから必要なフィールドだけを詰め替える設計が安全である。

---

## 11. 推奨SaveGameV1構造

```rust
/// セーブファイルのルート型。バージョンフィールドを必須にすることで、
/// 将来の構造変更時に「読み込めるが意味的に間違って解釈される」事故を防ぐ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveGameV1 {
    /// 常に1。将来のスキーマ変更時にここを2, 3...とインクリメントし、
    /// ロード時にバージョンごとの移行処理を分岐させる
    pub version: u32,

    // ── 世界と進行 ──────────────────────────────────────
    pub date: SavedGameDate,
    pub paused: bool,
    pub speed: u8,
    pub player_country: Option<CountryId>,
    pub world_civilization: SavedWorldCivilizationState,

    // ── 国家・州(既にSerde対応済みの型をそのまま使う) ──────
    pub countries: Vec<CountryData>,
    pub states: Vec<StateData>,

    // ── 外交・戦争 ──────────────────────────────────────
    pub diplomacy: SavedDiplomacyRegistry,
    pub justifications: WarJustificationRegistry,       // 既にSerde対応済みのため丸ごと格納可
    pub wars: SavedWarRegistry,
    pub claims: SavedClaimRegistry,                      // 実プレイでは常に空だが将来のため含める
    pub crises: SavedCrisisRegistry,                     // 同上
    pub country_ai: CountryAiRegistry,                   // 既にSerde対応済み
    pub military_ai: MilitaryAiRegistry,                 // 既にSerde対応済み

    // ── Division / Army ──────────────────────────────
    pub divisions: Vec<Division>,
    pub next_division_id: usize,
    pub battles: SavedBattleRegistry,
    pub armies: ArmyRegistry,                            // 既にSerde対応済み

    // ── 前線(現状は個別Divisionのみを保持) ──────────────
    pub frontlines: FrontlineRegistry,                   // 既にSerde対応済み
}

/// GameDateは`accumulator`(端数の経過時間)を含むが、これは表示に影響しない内部状態。
/// そのまま保存してよい(型を分けるまでもない)ため実際には`GameDate`を直接使う案もあるが、
/// `Serialize`未対応のため、①`GameDate`へderiveを追加する、②専用のSaved型を作る、の
/// どちらかを選ぶ(推奨は①。既存の公開フィールドのみで完結するため副作用がない)。

/// WorldCivilizationStateは静的な`stage_definitions`を除いた可変部分のみを保存する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedWorldCivilizationState {
    pub current_stage: WorldStage,
    pub milestone_countries: HashMap<WorldStage, HashSet<usize>>,
    pub last_advanced_date: String,
}

// SavedDiplomacyRegistry / SavedWarRegistry / SavedClaimRegistry / SavedCrisisRegistry /
// SavedBattleRegistryは、対応するRegistryに`next_id`を含めて全フィールドを写し取るだけの
// 薄いDTO(または対象RegistryにSerialize/Deserializeを直接deriveして丸ごと使ってもよい。
// ただし`WarRegistry`はメソッドが多く実装が長いため、DTO経由の方が「保存対象フィールド」を
// 明示できて安全)。
```

**保存しないもの**(§4の通り): `SelectedDivision`/`SelectedArmy`/`SelectedState`、
各種`*PanelState`、`CameraDragState`及びカメラの`Transform`、`NotificationHistory`、
`PendingAiWarDeclarations`、`BuildingRegistry`/`TechnologyRegistry`/
`MilitaryRegistry.definitions`(静的マスターデータ)、州の表示色(派生値)。

---

## 12. 安全なファイル書き込み方法

Windows環境での安全な手順(調査結果に基づく提案。実装はしていない):

1. **保存先ディレクトリ**: 既存の`assets/data/...`が起動ディレクトリ相対パスである慣習
   (`loader.rs`のエラーメッセージ「Make sure to run the game from the project root
   directory」)に合わせ、V1では相対パス`saves/`ディレクトリを提案する。
   **NEEDS USER DECISION**: OS標準のユーザーデータディレクトリ(`%APPDATA%\strategy_game\`等)
   を使う方が配布後のユーザー体験としては望ましいが、これには新規依存クレート(`directories`等、
   現在`Cargo.toml`に存在しない)の追加が必要になる。プロトタイプ段階では相対パスのままで
   十分と判断するが、正式リリース前には見直しが必要
2. **ファイル名**: `savegame_v1.ron`(V1では単一スロットのみのため固定名)
3. **一時ファイルへの書き込み**: `saves/savegame_v1.ron.tmp`へ書き込む
4. **flush**: `std::fs::File::sync_all()`を呼び、OSバッファではなくディスクへの書き込みを保証する
5. **一時ファイルから本ファイルへの置換**: `std::fs::rename(tmp, final)`。Windows/Unixとも
   同一ファイルシステム内の`rename`はアトミックであり、「書き込み途中の本ファイル」が
   他プロセス(または次回起動時のロード処理)から観測されることはない
6. **既存セーブの破損防止**: 上記の temp→rename 手順自体が、書き込み中のクラッシュに対する
   保護になる(rename前にクラッシュしても旧ファイルはそのまま残る)
7. **書き込み失敗時の処理**: `std::fs::write`/`File::create`が`Err`を返した場合(ディスク容量
   不足・権限エラー等)、ユーザーへ通知し、現在のゲーム状態には一切影響を与えない
   (保存操作の失敗であり、プレイ継続には支障がない設計にする)
8. **不正RON/欠損ファイル/未対応バージョン**: すべて§9のロード検証フェーズで検出し、
   「ロード失敗」として現在のプレイ状態を変更せずに拒否する。ユーザーには
   ローカライズされたエラーメッセージ(既存の`localization`パターンに合わせ、翻訳キー経由)で
   通知する
9. **将来の複数スロット/オートセーブ/クイックセーブ**: V1では実装しないが、ファイル名を
   `savegame_v1.ron`から`saves/slot_{n}.ron`のような形式に拡張するだけで対応でき、
   今回提案する設計(DTO分離・アトミック書き込み)はそのまま流用できる

---

## 13. ロード失敗時の扱い

§9のステップ2〜4を読み取り専用の検証フェーズとして完全に分離することで実現する。
既存の`loader.rs::validate_data`関数(起動時に`capital_state_id`の整合性等を検証し、
問題があれば`panic!`する設計)と同種の検証を行うが、**セーブ/ロードの文脈では`panic!`ではなく
`Result`を返してユーザーに通知する**必要がある点が異なる(起動時データはゲーム開発者の
設定ミスを早期発見する目的、セーブファイルはユーザー操作由来の破損を安全に拒否する目的)。

検証に失敗するケースと扱い:

| ケース | 扱い |
|---|---|
| ファイルが存在しない | 「セーブファイルが見つかりません」を表示、現在の状態を維持 |
| RONとして構文的に不正 | 「セーブファイルが破損しています」を表示、現在の状態を維持 |
| `version`フィールドが現在対応するバージョンと不一致 | 「対応していないセーブバージョンです」を表示、現在の状態を維持(将来的にはバージョンごとの移行処理をここに追加できる) |
| ID重複・存在しない参照(例: 実在しないCountryIdを所有者に持つDivision) | 「セーブデータが破損しています(参照エラー)」を表示、現在の状態を維持 |

**「保存はできるが重要な状態が失われる」不完全なセーブを完成扱いにしない**という要求に対する
判断: §11のSaveGameV1構造は、Division/Army/War/Frontline/Battle/外交関係/国家AI/軍事AIの
**全てを含む**設計としており、「戦闘中セーブ」「移動途中セーブ」「戦争中セーブ」を意図的に
除外していない(§14で個別の自動テストとして明記)。既存の`Division`型が既に`combat_id`
(戦闘参加中フラグ)や`destination`/`current_path`(移動中の経路)を完全な形で保持しているため、
これらを保存対象に含めないという判断こそが「不完全なセーブ」を生む。**V1のスコープから
意図的に除外してよいのは、UI選択状態とカメラ位置(§4)のみ**であり、これらはプレイの
継続性に本質的な影響を与えない付随情報である。

---

## 14. 自動テスト計画

`tests/daily_system_integration_test.rs`に既に存在する`GameStateSnapshot`
(§17参照)は、日次シミュレーションの決定論を検証するために手作りされた「ほぼ全状態を
網羅するスナップショットDTO」であり、**SaveGameV1のフィールド設計の直接的な実証済み前例**
として活用できる。セーブ/ロードのテストは、この既存パターンに倣い、
**意味的なDTO比較(バイト単位一致ではない)**を採用することを推奨する。理由:
`HashMap`のシリアライズ順序はRustの実行ごとに変わりうるため、RON出力のバイト単位一致は
本質的でない不安定要素になる。意味的な構造体比較(`#[derive(PartialEq)]`済みのSnapshot型を
ロード前後で比較)の方が、実装の変更に対して頑健なテストになる。

自動テスト項目(仕様書の24項目に対応、全て「セーブ→ロード→意味的なSnapshot比較」の形):

1. New Game直後(師団配置済み、日付未進行)の保存・ロード
2. 日付を複数日進行させた後の保存・ロード
3. 資金(`treasury`)・人的資源(`available_manpower`/`mobilized_manpower`)が完全一致で復元される
4. 州の`owner_country_id`が復元される
5. 占領状態(`controller_country`/`occupation_progress`/`original_owner`)が復元される
6. 講和による領土割譲後、`owner_country_id`と`controller_country`の両方が新所有国になった
   状態が復元され、かつ次フレームで`update_state_colors_on_controller_change`が正しい色を
   再計算する(色そのものは保存しないため、ロード後に色更新システムが実際に動くことまで
   確認する統合テストが必要)
7. Divisionの`current_state`が復元される
8. 複数回のセーブ→ロードを経てもDivisionIdが変化しない(同一個体として追跡できる)
9. 複数回のセーブ→ロードを経てもArmyIdが変化しない
10. Armyの`member_division_ids`/`division_army_map`の相互整合性が保たれたまま復元される
11. `destination`/`current_path`/`target_state`/`movement_progress`が設定された移動途中の
    Divisionが、寸分違わず復元される
12. P21-004Aで実装した「移動停止」直後(`status: Idle`、各移動フィールドが`None`/空)の状態が
    正しく復元される
13. 戦争中(`WarStatus::Active`、`occupied_states`が一部埋まった状態)のセーブ・ロード
14. 戦闘中(`Division.combat_id: Some(_)`、`BattleRegistry`に該当`Battle`が存在)のセーブ・ロード
15. Division消滅後(`MilitaryRegistry.divisions`から除去済み、`ArmyRegistry`が
    `sanitize_references`済みでID残骸がない状態)のセーブ・ロード
16. ロード後に新規募兵したDivisionのIDが、セーブ時点の`next_division_id`と衝突しない
17. ロード後に新規作成したArmyのIDが、セーブ時点の`next_id`(Army)と衝突しない
18. 存在しない参照(例: 削除済みCountryIdを指すDivision)を含む不正データを安全に拒否する
19. 構文的に不正なRON(壊れた括弧等)を安全に拒否する
20. `version`フィールドが未来のバージョン番号のセーブファイルを安全に拒否する
21. ロード失敗(§13の全ケース)の直後も、失敗前のゲーム状態(`GameStateSnapshot`相当)が
    一切変化していないことを確認する
22. セーブ→ロード→再セーブを行い、2回目のセーブ結果が1回目と意味的に同一であることを確認する
    (往復の安定性)
23. 既存238テスト(P21-004A完了時点)への回帰がないことを、本タスクの変更を加えた状態で
    フルスイート実行して確認する
24. `tests/profile_workload_correctness_test.rs`の`same_seed_produces_deterministic_results`
    に代表される既存のSnapshot決定論テストが、セーブ/ロード関連の変更後も無傷で通ることを確認する
    (セーブ/ロード機構の追加が既存の決定的シミュレーションに一切影響しないことの証明)

---

## 15. 手動確認計画

- New Gameを開始し、数日進めてからセーブ→一度ウィンドウを閉じずにロードし、資金・州所有・
  師団配置が変化していないことを目視確認
- 複数のDivisionを選択して編成(Army)を作成した状態でセーブ→ロードし、編成一覧が復元されていることを確認
- 戦争を開始し、州を占領した状態でセーブ→ロードし、占領州の色が(ロード後の初回フレームで)
  正しく再着色されることを確認(§14項目6の自動テストに対応する目視確認)
- Divisionへ移動命令を出した直後にセーブ→ロードし、移動が中断されず(またはP21-004Aの
  移動停止状態が保存されていれば停止状態のまま)継続することを確認
- 意図的に壊したセーブファイル(RONの括弧を1つ削除する等)でロードを試み、エラーメッセージが
  表示されゲームがクラッシュしないことを確認
- 存在しないバージョン番号を書き込んだセーブファイルでロードを試み、拒否されることを確認

---

## 16. P21-005への拡張性

**P21-SAVE-001の範囲では前線を実装しない**(指示通り)。将来のP21-005が想定する拡張
(FrontlineがArmyIdを参照、Armyが引き続きDivisionId一覧を保持、州境界/攻勢線/作戦状態/
実行中命令)について、SaveGameV1への影響を評価する:

- 現状`FrontlineRegistry`は既にリソースごと`Serialize`/`Deserialize`対応済みであり、
  `SaveGameV1.frontlines: FrontlineRegistry`として**フィールドをまるごと**含めている
  (§11)。将来`FrontlinePlan`に`assigned_army_ids: Vec<ArmyId>`のような新フィールドが
  追加されても、`#[serde(default)]`を付与すれば**古いセーブファイル(該当フィールドが
  存在しない)からも安全に読み込める**(既存の`CountryData`/`StateData`が多数のフィールドで
  この手法を採用済みであり、実証済みのパターン)
- `war::frontline`モジュールと`military::army`モジュールの間に現状コンパイル時の依存が
  ないことは、P21-004完了報告(§13)で既に確認済み。前線がArmyIdを参照するようになっても、
  `SaveGameV1`の`armies`と`frontlines`は独立したトップレベルフィールドのままでよく、
  構造そのものの組み替えは不要
- 「州境界/攻勢線/作戦状態/実行中命令」がFrontline側の新フィールドとして追加される場合も、
  同様に`#[serde(default)]`で後方互換に追加できる

**結論**: P21-005実装後にSaveGameV1へフィールドを追加する作業は、**既存フィールドへの
追記(`#[serde(default)]`付きの新規フィールド追加)であり、SaveGameV1のバージョンを
上げる必要すらない**(後方互換性が保たれるため)。これは`version: u32`フィールドを
「構造の非互換な破壊的変更があったときだけ」上げる運用にすれば実現できる。

---

## 17. 変更候補ファイル(将来の実装タスク向け、今回は変更していない)

- **新規**: `src/save/mod.rs`(または`src/persistence/mod.rs`) — `SaveGameV1`及び関連DTO定義
- **新規**: `src/save/serialize.rs` — 各Resourceから`SaveGameV1`への変換ロジック
- **新規**: `src/save/deserialize.rs` — `SaveGameV1`から各Resourceへの復元ロジック(§9の検証フェーズを含む)
- **新規**: `src/save/validate.rs` — ロード前の参照整合性検証(`loader.rs::validate_data`と同型)
- **変更**: `src/app/time.rs` — `GameDate`/`GameSpeed`/`GamePaused`へ`Serialize`/`Deserialize`を追加
- **変更**: `src/military/data.rs` — `MilitaryRegistry`へ`Serialize`/`Deserialize`を追加(または`divisions`+`next_division_id`のみを持つ薄いDTOを別途用意)
- **変更**: `src/military/battle.rs` — `BattleRegistry`へ`Serialize`/`Deserialize`を追加
- **変更**: `src/war/data.rs` — `WarRegistry`へ`Serialize`/`Deserialize`を追加
- **変更**: `src/diplomacy/data.rs` — `DiplomacyRegistry`へ`Serialize`/`Deserialize`を追加
- **変更**: `src/diplomacy/claims.rs`/`src/diplomacy/crisis.rs` — 同上(任意、実プレイでは常に空)
- **変更**: `src/country/mod.rs` — `CountryRegistry`へ`Serialize`/`Deserialize`を追加(または`Vec<CountryData>`をそのまま使う)
- **変更**: `src/ui/`配下 — セーブ/ロードボタンのUI追加(新規パネルまたは既存パネルへの追加)

---

## 18. 実装タスクの分割案

1. **タスクA(型定義)**: `SaveGameV1`及び補助DTOの定義、上記「変更候補ファイル」のうち
   既存Resourceへの`Serialize`/`Deserialize`追加のみを行う(挙動に影響しない、純粋な型追加)
2. **タスクB(セーブ)**: `CountryRegistry`等の現在のResourceから`SaveGameV1`を構築する
   変換関数、§12のアトミック書き込み手順の実装、`PostUpdate`での実行システム登録
3. **タスクC(ロード検証)**: §9ステップ2〜4・§13の検証ロジックのみを実装し、
   実際にResourceへ書き込む処理はまだ実装しない(検証だけを先に固めてテストする)
4. **タスクD(ロード適用)**: タスクCの検証を通過した`SaveGameV1`から、実際に各Resourceを
   書き換える§9ステップ5〜20の実装
5. **タスクE(UI)**: セーブ/ロードボタンの追加、ロード失敗時のエラー表示
6. **タスクF(テスト)**: §14の24項目・§15の手動確認

各タスクは独立してテスト可能であり、A→B→C→D→E→Fの順で1つずつ完了させることを推奨する
(特にC/Dの分離は「検証と適用を混同しない」という§9の設計方針そのものであり、テストの
書きやすさにも直結する)。

---

## 19. NEEDS USER DECISION

1. **保存先ディレクトリ**: 相対パス`saves/`(既存の`assets/data/`慣習と一貫)か、
   OS標準のユーザーデータディレクトリ(`directories`クレート等の新規依存が必要)か
2. **`ClaimRegistry`/`CrisisRegistry`をV1のスコープに含めるか**: 実プレイでは常に空のため
   含めても実害はないが、「未使用機能のためのコード」を先取りして書く価値があるかは
   製品判断。含めない場合、`SaveGameV1`から該当フィールドを省くだけで済む
   (後方互換の追加は容易、§16参照)
3. **カメラ位置・ズームを将来的にでも保存対象に含めるか**: V1では保存しない設計を推奨したが
   (§4)、UXとして「ロード後に前回見ていた場所へ戻る」価値をどう評価するかはユーザー判断
4. **`GamePaused`の復元方針**: 保存されていた値をそのまま復元するか、§9推奨の
   「常に`true`(一時停止)で開始する」を採用するか
5. **「ロード」をどこから呼び出せるようにするか**: §2で確認した通り、現状`GameState`は
   `Playing`への一方通行であり、`MainMenu`/`CountrySelection`からの「ロードして再開」導線は
   別途state遷移の追加が要る。V1では「Playing中のみロード可能(内部データの差し替えのみ)」
   とし、起動直後のロードは将来のタスクに回すことを推奨するが、この優先順位はユーザー判断による

---

## 20. 実装可否

**READY WITH DECISIONS**

技術的な障害は本調査で発見されなかった(Bevy Entity依存の危険な結合なし、全ID型が
Serde対応済みかつEntity非依存、既存の`GameStateSnapshot`テストパターンが設計の直接的な
実証になっている、州色のような派生データも既存実装で完全に分離済み)。着手前に必要なのは
§19の5項目の製品判断のみであり、これらは実装方針を左右するが実装可能性そのものを
妨げるものではない。

