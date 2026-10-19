# P32-003 ステート・プロヴィンス制への移行設計

**設計と調査のみ。** ゲーム本体のデータ構造・地理データ・シミュレーション・セーブ形式・旧セーブは変更していない(本コミットは `verification_logs/phase-32/p32-003/` への記録追加のみ)。
**この文書の完成は、ステート・プロヴィンス制の実装完了を意味しない。** 開始 HEAD: `ce3ae92`(main)。Phase 32 は継続する。

「採用案(推奨)」「代替案」「未決」を区別して書く。**未決のゲームルールは実装者の判断で確定しない**(§11 に集約)。

---

## 0. 用語の衝突(最初に決める)

既存コードの `State`/`StateId`/`StateRegistry`/`StateData`(日本語コメントでは「州」「地域」「region」と混在)は、**新設計の「ステート(複数プロヴィンスの集計単位)」と同じ意味ではない**。実体は「1 つの矩形領域に、所有・占領・人口・建物・資源・物流・隣接・位置を全部持たせた単一レコード」(`src/state/data.rs` の `StateData`)であり、新設計の
プロヴィンス(軍・占領の単位)とステート(人口・施設・生産の集計単位)の**両方の性質を 1 つに潰したもの**に当たる。
このため、移行中は次の呼び分けを提案する(名称は §11 の決定事項 D1):

| 本書での呼称 | 意味 | 現コードでの該当 |
|---|---|---|
| **旧州 / `StateData`** | 現行の 1 レコード単位(28 件、`assets/data/states.ron`) | `StateId`・`StateRegistry`・`StateData` |
| **ヘックス(セル)** | 制作ツール JSON の地理セル `(q,r)`(96,760 件) | `hexmap::data::HexCell` |
| **プロヴィンス** | ゲームが扱う細かな領域(第一案: 1 ヘックス = 1 プロヴィンス) | 未実装 |
| **ステート(新)** | プロヴィンスの集合。人口・雇用・施設・生産の集計単位 | 未実装(旧州の一部の役割を継承) |

---

## 1. 現状の整理(実コードに基づく)

### 1.1 新マップ側(P32-001/002 実装済み)
`HexMap`(`src/hexmap/data.rs`)は不変・検証済みの地理データ。座標 `(q,r)` から `cell(q,r)` で O(1) 検索。属性は地形/気候/植生。河川は正規化済み辺(`river_edges`)。**所属・所有・名称は持たない。**
実データの地理的事実(`hex_map_geography_stats.txt`、本コミットで追加した一時計測の出力):
- セル 96,760: 陸 34,503 / 海 62,055 / 湖 202。
- 陸の連結成分(6 近傍)110 個(最大 17,475 セル。単一セル 18 個、10 セル以下 52 個)。海は連結 1 成分。湖は 10 個(いずれも単一セルではなく、海に隣接しない=内陸湖)。海岸の陸セル 4,882。
- 河川の辺 3,398: 陸|陸 3,275、陸|水域 119、**水域|水域 4**(参照元セルは存在するが両側とも水域の辺が実データにある)。

### 1.2 旧州側の規模
- 旧州 28 件(`states.ron`)、国家 6、初期データに `is_sea: true` の州は 0 件(`is_sea` フラグ自体はコード側に存在)。
- `StateId`/`StateRegistry`/`StateData`/`current_state`/`target_state` を参照するファイルは 96(`src` 配下、テスト含む)。参照数が多い順: `country/country_ai.rs` 345、`war/frontline.rs` 296、`war/tests.rs` 239、`military/tests.rs` 213、`save/validate.rs` 189、`military/invasion.rs` 175、`save/apply.rs` 121、`military/movement.rs` 116、`military/update.rs` 113、`map/division_selection.rs` 100。→ **一括置換は現実的でなく、段階移行が必須**。

### 1.3 `StateData` の内訳(新構造での置き場の候補)
`id` / `name` / `owner_country_id` / `population` / `workforce_ratio` / `employed_workforce` / `unemployed_workforce` / `education` / `living_standard` / `unrest` / 物流 4 項目(`base_logistics_capacity`・`logistics_capacity`・`logistics_usage`・`logistics_ratio`)/ `buildings` / `building_operation_ratios` / `resource_deposits` / `world_position`・`size`(矩形)/ `neighbors`(有向リスト)/ `integration` / `controller_country`・`occupation_progress`・`original_owner`・`occupation_policy`・`war_id`(占領系 5 項目)/ `is_sea`。

---

## 2. 現在の構造と新構造の対応表

凡例: **P**=プロヴィンス側 / **S**=ステート(新)側 / **C**=国家側に残す / **H**=ヘックス(地理、不変)

| 領域 | 現在の管理単位(実ファイル・型) | 新構造での置き場の候補 | 移行で影響を受ける処理 | 状態 |
|---|---|---|---|---|
| 地域 ID・隣接 | `StateId(usize)`(`common/mod.rs`)、`StateData.neighbors: Vec<StateId>`(手書きの**有向**リスト。`capital_connectivity` は有向として扱う) | 隣接は **H/P**(ヘックスの 6 近傍から導出。手書きしない)。ステート間隣接は P 隣接から導出 | `military/pathfinding.rs`(`neighbors` 走査のダイクストラ、コスト固定 1)、`capital_connectivity.rs`、`daily_supply.rs`、`diplomacy/ai_claim_generation.rs`、`war/frontline.rs` | 確定案(導出) |
| 国家の領有 | `StateData.owner_country_id`。移転は `state/ownership.rs` の `preflight_transfer_state_ownership`→`apply_ownership_transfer`(首都再配置・原子性・TOCTOU 再検証を持つ共通 API) | **S**(法的所有はステート単位を第一案、§6) | 和平(`war/peace.rs` `CedeWarGoalRegion`)、国家消滅(`country/elimination.rs`)、外交(`claims.rs` の `target_state`、危機・戦争目標 `target_states`) | 要判断 D5 |
| 実効支配・占領 | `controller_country`・`occupation_progress`(0〜100)・`original_owner`・`occupation_policy`・`war_id`。`war/occupation.rs` が師団 1 個あたり +5.0/日、100 で支配国交代 | **P**(占領=支配はプロヴィンス単位)。ステートの支配国は P から**導出**(規則は未決 D6)。`occupation_policy` はステート | `war/occupation.rs`、`war/war_score.rs`、`War.occupied_states`、`state.controller()` を使う全箇所(`capital_connectivity` ほか多数) | 要判断 D6 |
| 人口・人的資源 | `StateData.population`、`workforce_ratio`、就業/失業、`population/growth.rs`(州ごとに成長)、`population/mobilization.rs`(`(StateId, 人口)` ペアで動員負担を配分)、`military/manpower.rs` | **S** | 人口成長・動員配分・`country/power.rs` の人口合計は「S の一覧」を回すだけ。**P 側へ人口を配らない**(第一案) | 確定案 |
| 雇用 | `employed_workforce`/`unemployed_workforce`、`population/update.rs`(建物ごとの必要労働力で配分) | **S** | `economy/mod.rs` の雇用・生産ループ | 確定案 |
| 資源 | `resource_deposits: Vec<StateResourceDeposit>`(`economy/resources.rs`)、`app/loader.rs` が `resources.ron` から州へ注入、`building/mine_migration.rs` | 第一案 **S**(現行どおり集計単位に鉱床)。将来「どの P にあるか」を持たせる拡張を妨げない形にする(資源配置は今回対象外) | 鉱山・生産(`economy/production.rs`、`building/supply_chain_migration.rs`) | 確定案(P への下ろし方は後続) |
| 施設(建物) | `buildings: HashMap<BuildingType,u32>`、`building_operation_ratios`、建設 `economy/mod.rs`(`item.state_id`) | **S**(第一案)。港・要塞など場所依存の施設は将来 P へ(未決 D9) | `economy/*`、`ui/state_panel.rs`(建設ボタン)、`country_ai.rs`(建設 AI) | 確定案+未決 |
| 生産 | `economy/production.rs`(国家ごとに所有州の slice を処理。順序は `BuildingType` 外側→`StateId` 昇順の決定論) | **S** 集計 → **C**(国家の備蓄) | 決定論の並び順の基準 ID を、新ステート ID の昇順へ置き換える必要(`P26-DET-001` 系テスト多数) | 確定案 |
| 物流・補給 | 物流容量/使用量/充足率は州フィールド(`logistics/update.rs`)。軍の補給は首都からの連結性(`military/capital_connectivity.rs`: 首都→自国支配下の**陸上州**→`neighbors`→師団所在州。`is_traversable = !is_sea && controller()==country`)、`military/daily_supply.rs` | 物流容量: **S**。軍補給の連結性: **P グラフ上の BFS**(または S レベルで計算して P へ展開、未決 D10) | 連結性は毎日 国家数×ノード数のBFS。ノードが 28 → 約 34,500(陸)になるため**計算量の再評価が必要** | 要判断 D10 |
| 財政 | `economy/finance.rs`: 州ごとの税(人口×生活水準×税率)・建物維持費・物流維持費を国家へ集計 | 税・維持費の元は **S**、財布は **C**(`CountryData.stockpile`) | 集計のループ単位の入れ替えのみ | 確定案 |
| 軍の位置・移動 | `Division.current_state`/`destination`/`current_path: Vec<StateId>`/`target_state`、`movement_progress`、`military/movement.rs`・`pathfinding.rs`・`invasion.rs`(上陸/侵攻) | **P**(`ProvinceId`)。経路は P グラフ | 1 ホップあたりの所要日数(現在は州間 1 ホップ)の再調整、経路探索のノード数増(28→数万)、軍の編成・補給所在地 | 確定案(速度は未決 D8) |
| 戦闘 | `BattleId`・戦闘は `battle.state_id`(`military/update.rs`、`combat_calc.rs`) | **P**(戦闘は軍と同じ場所で起きる)。地形は P の H 属性を参照可能(地形効果は今回対象外) | 戦闘登録のキー、占領判定への接続 | 確定案 |
| 前線 | `war/frontline.rs`(296 参照): `attacker_front_regions`・`defender_front_regions`・`border_region_pairs: Vec<(StateId,StateId)>`、`objective_region_id`、攻勢線の占領判定 | 計画・目標は **S** 単位、実行(移動・占領)は **P** 単位(案)。P 単位だけにすると前線が数千ペアになり UI も AI も重い | 最大の改修点。`frontline_render`/`frontline_selection`/`offensive_line_selection` も連動 | 要判断 D7 |
| 占領→戦争点数 | `War.occupied_states: HashSet<StateId>`、`war_score.rs` が `controller_country` から更新 | S の占領度(P の支配状況の集計)から導出 | `war_score.rs`、降伏判定 `war/capitulation.rs` | 要判断 D6 |
| 和平・領土移転 | `war/peace.rs`(`CedeWarGoalRegion` 等)が `state::ownership` の共通 API で移転。首都再配置あり | **S 単位の移転**(第一案)。移転 API は**ステートを渡す**形で維持し、内部で配下 P の所有/支配を更新 | `war/peace.rs`、`WarGoal.target_states`、請求 `claims.rs`、AI | 要判断 D5 |
| 国家消滅・勝敗 | `country/elimination.rs`(「最後の所有陸州」の移転で消滅。`preflight_final_state_transfer_for_elimination`)、`ui/player_elimination.rs`、`player_victory.rs`、`war/stalemate.rs` | **S** 単位(「所有する陸ステートが 0」)で判定。水域は除外 | 最終ステート判定、消滅時の師団配置換え(`peace.rs` が `current_state` を最小 StateId の自国州へ移す処理がある) → **P の再配置先を決める規則が必要** | 確定案+要規則 |
| 国家データ | `CountryData.capital_state_id: StateId`(`country/mod.rs`)、`loader.rs` が「首都は実在かつ自国所有」を検証 | 首都は **S**(施設・人口)。補給起点が必要なため首都ステート内の **P**(`capital_province_id`)も持つ(案) | `loader.rs` の検証、`ownership.rs` の首都再配置 | 確定案 |
| 地域選択 UI | `map/selection.rs`(`StateVisual` の矩形クリック、`SelectedState`)、`map/rendering.rs`(矩形スプライト+暗い枠)、`ui/state_panel.rs`(建設ボタンつき) | 選択は **P**(ヘックスのクリック=P)→ 所属 **S** を表示。描画は P0 のチャンクメッシュ(P32-001 と同方式)に所有色・ステート境界を重ねる | `map/*` 全般(division_selection 100 参照)、`ui/state_panel.rs`、`diplomacy_panel.rs` | 確定案 |
| 軍の描画 | `map/division_render.rs`: `state.position()`(`world_position`)に表示 | P の中心(`HexGrid::center_world`) | 位置の取得元の差し替えのみ | 確定案 |
| セーブ/ロード | `save/dto.rs`: `SaveGameV1` が `states: Vec<StateData>` ほかを丸ごと保持。`SAVE_FORMAT_VERSION_V1`、`save/validate.rs`(`StateId` 参照 110 箇所の整合性検証)、`apply.rs`、`runtime.rs`。`saves/savegame_v1.ron` はリポジトリで追跡されている | 動的な状態(所有・支配・占領・人口…)だけを保存し、静的な定義は保存しない。地図指紋(§8)を保存して不一致を拒否 | 新形式(V2)が必要。旧セーブの扱いは §10 | 要判断 D11 |
| AI | `country/country_ai.rs`(345 参照): 隣接州・所有州の一覧から建設・請求・宣戦などを決める。`military/ai.rs`、`war/military_ai.rs` | S 一覧を基本に回す(P を直接全走査しない) | S レベルの「隣接ステート」API を先に用意する必要 | 確定案 |

---

## 3. 提案するデータモデル

### 3.1 3 層 + 国家
```
HexMap(制作ツール JSON。読み取り専用・地理の唯一の正)
  └─ cells(q,r): 地形/気候/植生/河川
ProvinceTable(ゲーム側。ID の安定化)
  └─ ProvinceId ↔ (q,r)    ※第一案は 1:1
WorldLayout(ゲーム側。所属)
  └─ StateDef: StateId, name_key, provinces: [ProvinceId], 分類・フラグ
動的状態(セーブ対象)
  ├─ Province 動的: controller(支配国), occupation_progress, war_id …
  ├─ State 動的:   owner, population, buildings, … (旧 StateData の大半)
  └─ Country:      stockpile, capital_state/province …(既存)
```

### 3.2 型の案(`u32` 系の新 ID 型。既存 `StateId(usize)` とは別型)
```rust
struct ProvinceId(u32);        // ゲーム側で安定。座標から機械的に決めない(§4)
struct StateAreaId(u32);       // 仮称。D1 で命名決定。旧 StateId とは別型
struct HexCoord { q: i32, r: i32 }

struct ProvinceStatic { id: ProvinceId, cell: HexCoord, kind: ProvinceKind /* Land | Sea | Lake */ }
struct StateDef      { id: StateAreaId, name_key: String, provinces: Vec<ProvinceId>, … }
```
`ProvinceStatic` の地形属性自体は複製せず、`HexMap.cell(q,r)` を参照する(情報の二重管理を避ける)。`kind` は地形から導出可能だが、ID 確定時の地形(陸/水)を記録して、後述の「陸↔水の変更検出」に使う。

---

## 4. ID と安定性

### 4.1 採用案: 「安定 ID テーブル」方式
- `ProvinceId` は **ゲーム側のテーブル(`province_table`)に保存した値**。JSON 配列の並び順・件数・重複排除の順序から導出**しない**(ご要望の「配列の並び替えだけで識別が変わらない」を満たす)。
- 初回のテーブル生成(ツールまたはテスト用ヘルパー)だけ、座標を `(r, q)` 昇順に整列して採番する。以後は**追記専用**: 新セルには新しい最大値+1 を割り当て、削除されたセルの ID は**再利用せず**「廃止(retired)」リストへ移す。
- ID は 0 始まりの連番でも良いが、保存・参照・外部データはすべて ID(座標ではない)で行う。座標は**テーブル内の 1 属性**。

### 4.2 代替案: 座標から導出する ID
`ProvinceId = pack(q, r)`(q は負もあるのでオフセット付き)。テーブル不要で単純。ただし(1)制作ツールが原点・extent を変更すると**全 ID が変わる**、(2)「1 ヘックス = 1 プロヴィンス」が将来崩れる(複数ヘックスで 1 プロヴィンス、またはヘックスの分割)と ID の意味がなくなる、(3)セル削除で参照が宙に浮く。→ **推奨しない**。ただし採否は D2。

### 4.3 地図指紋(再出力検知)
テーブルに、同期時点の次を保存する:
- `grid_fingerprint`: orientation・size・origin・imageWidth/Height・extent。**不一致は常にエラー**(座標系そのものが変わったため、座標ベースの対応が成り立たない)。
- `geometry_hash`: `(q, r, 水域か否か)` の整列済み集合のハッシュ(座標の追加/削除、または陸↔水の変更で変わる)。
- `attribute_hash`: 地形・気候・植生・河川を含む全体のハッシュ(属性だけの修正で変わる)。
→ **属性だけの修正**(`geometry_hash` 同一・`attribute_hash` 違い)は情報通知のみで所属は無傷。**座標の追加/削除や陸↔水の変更**(`geometry_hash` 違い)は「所属設定が壊れうる」ため、検証を必須にし、差分(追加セル・削除セル・種別変更セル)を列挙して失敗させる。同期コマンド(明示操作)でのみテーブルを更新する。

### 4.4 国家 ID との関係
`CountryId` は既存のまま不変(`countries.ron`)。プロヴィンス/ステートは国家 ID に依存しない。所有は**動的状態**で、定義ファイルに「初期所有国」を別ファイルとして置く(§8)。

---

## 5. 所属関係と検証

- 所属の正は `StateDef.provinces`(ステート→プロヴィンス)。**逆引き**(プロヴィンス→ステート)は読み込み時に導出する索引(保存しない)。
- 読み込み時の検証:

| 検出対象 | 扱い |
|---|---|
| 二重所属(同じ `ProvinceId` が 2 ステートに出現) | **エラー**(両ステート ID を報告) |
| 存在しない `ProvinceId` の参照 | **エラー** |
| 廃止(retired)済み ID の参照 | **エラー** |
| 未割当の陸プロヴィンス | 段階による(§11 D4): 検証用小地域では**警告+一覧**、本番ワールドでは**エラー**を推奨 |
| 水域プロヴィンスのステート所属 | 第一案は**不可(エラー)**(§6) |
| 空のステート | **エラー** |
| ステート ID 重複 | **エラー** |

---

## 6. 水域(海・湖)

### 6.1 第一案
- ヘックスは海・湖も**地理セルとして保持**する(既に `HexMap` が保持)。
- **ステートの所属は陸プロヴィンスのみ**。海・湖はステートに属さない。
- 軍の位置は陸の `ProvinceId`。海軍・海上国家は将来、**水域は座標 `HexCoord` で位置を表す**拡張(`enum Location { Province(ProvinceId), WaterCell(HexCoord) }` のような統合型)を想定し、今回の `ProvinceId` を陸専用に固定しない(`ProvinceKind` に Sea/Lake を持たせておく)。
- 湖: 実データの 10 湖は内陸湖(海に隣接しない)。第一案は「通行不可・ステート非所属・所属必須でもない」。湖の資源・漁業・水運は未決。

### 6.2 代替案
(a) 海・湖も全部プロヴィンスにして **海域ステート(SeaZone)** へ所属(62k プロヴィンス分の動的状態と AI 走査が増える。海軍を早く入れるなら有利)。(b) 海はまとまった海域だけをプロヴィンス化。→ いずれも海軍仕様が未確定のため、現時点で選ばない。D3。

### 6.3 既存コードとの関係
旧 `is_sea` フラグは「海域の州を陸と区別する」ための既存概念で、`capital_connectivity`・請求(`claims.rs`: 海州への請求禁止)・AI が参照している。新構造では `ProvinceKind` と「ステートは陸のみ」へ置き換わるため、`is_sea` を使うガード(約 10 箇所)は移行時にそのまま不要になるか、`kind != Land` の判定へ置換する。

---

## 7. 所有と占領

### 7.1 採用案(推奨)
- **法的所有(owner)はステート単位**。和平による割譲・消滅・請求・戦争目標は、現在の API(`state::ownership`)の設計思想(1 ユニットを原子的に移す)をそのまま「ステート」に適用できる。
- **実効支配(controller)と占領進捗はプロヴィンス単位**。師団がいるプロヴィンスで占領が進む。
- ステートの支配国は P の支配から**導出**する。導出規則(多数決/首都プロヴィンスの支配国/全プロヴィンスが同一支配のときのみ変更 など)は**未決(D6)**。
- 占領政策(`OccupationPolicy`)・占領による統合度低下などはステート単位の属性とする(案)。

### 7.2 一つのステートが複数国に分かれた場合
- **所有**: 第一案では「分かれない」(ステートは常に 1 国が所有)。部分割譲を許すかが D5。許すなら所有をプロヴィンス単位にする必要があり、人口・建物・生産の按分ルールが新たに必要になる。
- **支配**: 戦時中は 1 ステート内で複数国が支配しうる(前線がステート内部を通る)。この間の人口・生産・税の帰属は未決(D6): (a) 所有国のものとして計算し占領度で減衰、(b) 支配国が割合で得る、(c) 支配が割れている間は生産停止、など。
- 和平時: 所有ステートを移転するとき、配下プロヴィンスの支配国も新所有国へ揃える(現行 `apply_ownership_transfer` が `controller`/`occupation_progress`/`original_owner`/`war_id` を正常化する挙動に相当)。

### 7.3 既存の和平処理との整合
`CedeWarGoalRegion` は「戦争目標の州を丸ごと割譲」であり、**ステート単位の移転**ならそのまま一致する。`War.occupied_states` と占領による戦争点数は「ステートの占領率」から作る(閾値は D6)。師団の再配置(`peace.rs` が首都不在・消滅時に所有州の最小 `StateId` へ移す)は「最小のステート内の代表プロヴィンス」へ置き換える規則が必要(代表プロヴィンスは `StateDef` に持たせる案)。

---

## 8. データファイルの分離

### 8.1 提案する 3 層ファイル(いずれも**ゲーム側ファイルは制作ツールの再出力で上書きされない**)
| ファイル(案) | 所有者 | 内容 | 再出力時 |
|---|---|---|---|
| `assets/data/hex_atlas_map.json` | **制作ツール** | 地形・気候・植生・河川・グリッド定義 | 全置換される(現行どおり) |
| `assets/data/world/province_table.ron` | ゲーム(生成ツール+手動の同期操作) | `ProvinceId`↔`(q,r)`、種別、廃止リスト、`grid_fingerprint`・`geometry_hash`・`attribute_hash` | 触れない。同期は明示操作のみ |
| `assets/data/world/state_layout.ron` | ゲーム(手編集) | ステート定義: `StateAreaId`、`name_key`、所属 `ProvinceId` 列、代表プロヴィンス、フラグ(`allow_disconnected` など) | 触れない |
| `assets/data/world/start_setup.ron` | ゲーム(手編集) | 初期の所有国(ステート→国)、首都、人口・建物などの初期値(旧 `states.ron` の役割) | 触れない |

名称(`name_key`)は既存の翻訳キー方式(`assets/localization/*.ron`)に載せる。座標ではなく **ID** で参照する(JSON の並び順・座標系のずれの影響を受けない)。

### 8.2 再出力に対する検出(起動時/検証時)
1. `grid_fingerprint` 不一致 → **エラー**(座標系が変わった)。
2. `geometry_hash` 不一致 → **エラー**+差分一覧(追加セル/削除セル/陸↔水の変更)。追加セルは「未登録(ID 未発行)」、削除セルは「所属から参照されている/いない」を区別。**ステート所属に影響する削除・陸↔水変更だけを致命的**、影響のない追加は警告として、同期コマンドで ID を発行するまで保留。
3. `attribute_hash` のみ不一致 → **情報**(属性だけの修正。所属は無傷。河川の変更もここに含まれる)。
4. セーブには `map_fingerprint`(1 と 2 の要約)を保存し、ロード時に不一致なら拒否(§10)。

---

## 9. ステートの形(提案)

| 事象 | 提案 | 備考 |
|---|---|---|
| 空のステート、存在しない/廃止済み/二重所属のプロヴィンス、水域プロヴィンスの所属 | **エラー** | §5 |
| 6 近傍で連結していない(飛び地・島を含む) | **警告**(成分数・各成分の代表座標を表示)。`allow_disconnected: true` を定義に書けば抑止 | 島国・列島・飛び地を表現可能にする。連結性を強制しない |
| 水域を挟んで離れた構成 | 上と同じ(許容。ただし陸上補給は通らない — 現行の `capital_connectivity` と同じく陸の連結のみ) | 海軍による接続は将来 |
| 単一セルのステート | **許容**(実データに単一陸セルが 18 個ある) | 小島の扱いを後で調整 |
| 巨大/極小サイズ | **情報**(しきい値は未決) | |
| 未割当の陸プロヴィンス | D4 | |
| 河川をステート境界にするか | **ルールとして追加しない**(ご指示どおり)。河川は地理属性のまま。境界と一致させるかは制作者の自由 | |
| 湖をまたぐステート | 許容(内陸湖の周囲を 1 ステートにする等) | |

---

## 10. セーブ互換性の影響と提案

- 現行 `SaveGameV1`(`SAVE_FORMAT_VERSION_V1`)は `states: Vec<StateData>`(28 旧州)・師団の `current_state`/`current_path`・戦争の `occupied_states`・前線の `border_region_pairs`・国家の `capital_state_id` など、旧 `StateId` を**広範に**含み、`validate.rs` が 110 箇所で整合性を検証している。旧セーブの中身は「旧 28 州の世界」に結び付いている。
- 新ワールド(ヘックス 34,503 陸プロヴィンス)は旧 28 州と**地理が別物**なので、旧 `StateId`→新 ID の対応表は存在しない。互換変換は意味のある結果を作れない。
- **提案**: 本体切り替え(M9)で `SaveGameV2` を導入し、**V1 は明示的に非対応**にする(ロード時に「旧形式は非対応」と明確なメッセージを出し、ファイルは変更しない)。V1 の読み取り・検証コードと `saves/savegame_v1.ron` は、**切り替え前は一切削除・変更しない**。切り替え時に V1 コードを残すか消すかは D11。
- 今回: セーブ形式の変更・旧セーブの削除は行っていない。

---

## 11. ユーザー判断が必要な事項

「次の実装タスクを始めるために決める必要があるもの」には ★ を付けた。

| # | 判断事項 | 推奨 | 代替 |
|---|---|---|---|
| ★D1 | 新概念の型名・モジュール名。旧 `State*` との衝突回避 | 移行中は旧 `State*` を一切触らず、新規を `world`(モジュール)・`ProvinceId`・`StateAreaId`(仮)で導入。最終 M9 で旧→`Legacy*`、新→`State*` へ改名 | 最初から旧 `State*` を `Province*` に改名(全 96 ファイルに波及・非推奨) |
| ★D2 | `ProvinceId` の方式 | 安定 ID テーブル(§4.1) | 座標導出 ID(§4.2) |
| ★D3 | 水域を ID(プロヴィンス)化するか | 第一案: 陸のみステート所属、水域は `ProvinceKind` を持つが所属なし(§6) | 海域ステート化 |
| ★D4 | 未割当の陸プロヴィンスの扱い | 検証用: 警告+一覧 / 本番: エラー | 未割当を「無所属地(荒野)」として許容 |
| ★D12 | 地図指紋と同期の運用(制作ツール再出力の手順) | §4.3・§8.2 の通り、明示の同期操作でのみテーブルを更新 | 起動時に自動で ID 発行(事故の元になるため非推奨) |
| D5 | 所有の粒度・部分割譲の可否 | ステート単位のみ(§7.1) | プロヴィンス単位所有(按分ルールが必要) |
| D6 | ステートの支配国の導出規則、戦時中の人口・生産・税の帰属、占領率→戦争点数 | 後続(M7 前)に決定 | — |
| D7 | 前線・戦争目標・AI の計画単位(S か P か) | 計画は S、実行は P | P のみ(前線数が膨大) |
| D8 | 移動速度・占領速度のスケール(ヘックス間 1 ホップの所要日数、現行 +5.0/日・20 日占領) | 実測しながら調整(M6) | — |
| D9 | 施設・資源の P への下ろし方(港、要塞、鉱床の位置) | 第一案は S に集約、P 配置は後続 | — |
| D10 | 補給連結性の計算単位(P グラフ BFS か、S グラフで計算か) | M5 で計算量を実測して決める | — |
| D11 | 旧セーブ(V1)の扱い | 明示的に非対応(§10) | 変換ツールを作る(意味のある変換が作れず非推奨) |
| D13 | 湖(内陸湖 10 個)の扱い(通行不可・所属なしが第一案) | §6.1 | 隣接ステートへ所属 |

---

## 12. 段階的な移行計画(既存ゲームの一括置換はしない)

各段階は**単独で検証でき、既存ゲームの挙動を変えない**ことを目標とする。

| 段階 | 目的 | 変更対象 | 前提 | 検証内容 | 完了条件 |
|---|---|---|---|---|---|
| **M0** 新 ID と所属データの導入 | プロヴィンス ID・ステート定義の型、ファイル形式、ローダー、検証、指紋 | 新規 `src/world/`(純粋 Rust。Bevy 非依存)、新規データ用ディレクトリ。**既存コードは変更しない** | D1〜D4・D12 の決定 | 単体テスト: ID 安定性(JSON 並び替えで不変)、二重所属・存在しない参照・水域所属・空ステートのエラー、`geometry_hash`/`attribute_hash`、属性だけの修正と座標追加/削除の区別 | テスト合格。既存全体テスト・clippy 変更なし(既存挙動に影響なし) |
| **M1** 小地域の検証用ステート配置 | 実地図の一部(例: 島 1 つ)に数ステートを手配置して M0 のデータで通す | `assets/data/world/` の**検証用**ファイル(本番配置ではない) | M0 | 検証用配置が全検証を通る。再出力シミュレーション(属性だけ/座標追加/座標削除の合成差分)で期待どおり検出 | 検証用ワールドが読み込め、検出が仕様どおり |
| **M2** プレビューでの境界・所属表示 | ステート境界・所属・所有色を `hex_map_preview` で確認 | `src/hexmap/view.rs` 周辺(表示モード追加)。ゲーム本体は変更しない | M1 | 境界が正しい辺に出る、選択で所属ステートが出る、ヘックス/ステートの 2 層選択、再入場で重複しない。**GUI は実操作の確認をユーザーに依頼** | 見た目と所属が一致(ユーザー確認) |
| **M3** 地域集計処理の分離 | 「ステートの集計」を、旧州とは独立の純粋関数として用意(人口合計・隣接ステート導出・ステート間隣接) | `src/world/` に集計 API | M1 | 隣接導出がヘックス近傍と一致、ステート間隣接が対称、P 数万での性能測定 | API とテスト |
| **M4** 人口・施設・生産の接続 | 新ステートを経済ループの単位にする(旧 `StateData` からの移行用アダプタ) | `population/*`、`economy/*`、`building/*`、`logistics/update.rs` を「ステート一覧を受け取る」形へ。**旧州でも動くまま**(二重実装期間) | M3、D9 | 既存の決定論テスト(P26-DET-001 系)が新ステート ID 昇順で同じ結果を出す | 既存テスト群が新旧どちらでも合格 |
| **M5** 補給連結性 | 首都連結性を P グラフに載せ替え、計算量を測る | `military/capital_connectivity.rs`、`daily_supply.rs` | M3、D10 | P34k ノードでの毎日の BFS 時間、旧小規模ケースと同結果 | D10 を確定できる性能データ |
| **M6** 軍位置・移動・占領 | `Division.current_state` 系を `ProvinceId` へ、経路探索を P グラフへ、占領を P 単位へ | `military/*`、`war/occupation.rs`、`war/war_score.rs`、`map/division_*`、描画の位置 | M4・M5、D6・D8 | 経路探索の決定論・性能、占領進捗、既存の軍/戦闘テストの移植 | 小地域で移動・戦闘・占領が成立 |
| **M7** 和平・領土移転・勝敗 | 移転・消滅・戦争点数をステート単位で接続 | `state/ownership.rs`(相当 API)、`war/peace.rs`、`country/elimination.rs`、`diplomacy/claims.rs`、前線(D7) | M6、D5・D6・D7 | 共通 API の原子性/TOCTOU 再検証テストの移植、消滅時の師団再配置、勝敗 UI | 既存の P21-020/021 系 E2E 相当が新構造で合格 |
| **M8** セーブ/ロード対応 | `SaveGameV2` の設計・実装、地図指紋の保存と検証 | `save/*` | M7、D11 | 往復、検証エラー、指紋不一致の拒否 | V2 の保存/ロード合格 |
| **M9** 本体への切り替え | 本番ワールド配置・旧州の廃止・名称の最終整理・旧セーブの非対応化 | 全体、`app/loader.rs`、`main.rs` | M8 | 全体テスト・clippy・長期監査(切り替え時のみ)・実機確認 | ユーザー承認 |

(M3 を M1 の前後どちらに置くかは D1〜D4 決定後に入れ替え可能。M4〜M7 は互いに依存するため、各段階で既存テストを緑のまま保つ「アダプタ」方式を採る。)

---

## 13. 次の実装タスク案

**P32-004(最小単位): ワールドモデルのデータ層(M0 のみ)**
- 範囲: 純粋 Rust の新モジュール(`ProvinceId`・`StateAreaId`・`StateDef`・`ProvinceTable`)、ファイル形式とローダー、§5 の検証、§4.3 の指紋(3 種のハッシュ)、テスト用の小さな合成データ。ゲーム本体・地理 JSON・既存システム・セーブは変更しない。プレビューへの組み込みも次段階(M2)。
- 開始条件: **★D1・D2・D3・D4・D12 の決定**(推奨案で良ければ「推奨のとおり」の一言で足りる)。D5〜D11・D13 は M0 では不要。
- 完了条件: §12 の M0 の検証内容がすべて自動テストで通ること。

---

## 14. 追加・参照した記録
- 本書、`p32-002_user_manual_verification_record.md`(P32-002 のユーザー手動確認と完了判定)、`hex_map_geography_stats.txt` とその生成コード `p32_003_geo_stats_example.rs.txt`(読み取り専用の一時計測)。
- 調査で確認した主なコード: `state/data.rs`・`state/ownership.rs`・`military/{data,pathfinding,movement,capital_connectivity,daily_supply,invasion,update}.rs`・`war/{occupation,frontline,peace,data,war_score,capitulation}.rs`・`country/{mod,elimination}.rs`・`population/*`・`economy/*`・`logistics/update.rs`・`save/{dto,validate,apply}.rs`・`map/{selection,rendering,division_render}.rs`・`ui/state_panel.rs`・`app/loader.rs`。
- 未確認(今回は調査対象を参照数・主要型・主要関数の確認に限った): `country_ai.rs`・`war/military_ai.rs` 内部の個別ロジックの網羅、各 UI パネルの全 `StateId` 利用箇所。これらの詳細は各移行段階の開始時に再調査する。

---

## 15. 追記(P32-007 / 2026-10): D9 の採用方針

ユーザーが D9(施設・資源のプロヴィンス/ステートへの置き場)について、次の管理方針を承認した。本節は P32-003 の当初の推奨(「第一案は S に集約、P 配置は後続」)を、承認された範囲で更新する。**この承認だけでは、下記の「確定しない事項」は決まらない。**

| 対象 | 管理単位(承認) |
|---|---|
| 人口・雇用 | **ステート単位** |
| 工場・農場・採掘施設などの経済施設(規模・雇用・生産) | **ステート単位** |
| 天然資源(鉱床・埋蔵量などの所在地) | **プロヴィンス単位**。経済処理に必要な利用可能量等は**ステートへ集計**して渡す |
| 位置が重要な施設(港・要塞・鉄道駅など) | **プロヴィンス単位** |

確定しない事項(維持): 資源量・施設数の具体的な配置 / 資源の枯渇・再生・採掘速度 / 占領時の施設利用や生産の扱い / 施設の破壊・接収ルール / 法的所有と部分割譲 / 補給の具体的な計算方法 / 旧セーブの廃止と V2 移行。
P32-007 は人口・雇用・経済施設(ステート単位)の接続だけを実装し、天然資源の所在地(プロヴィンス単位)と位置施設は未実装である(`MapStateEconomy.resource_deposits` は「ステート単位に集計済みの利用可能な鉱床」を受け取る入力でしかなく、プロヴィンスごとの所在地は持たない)。
