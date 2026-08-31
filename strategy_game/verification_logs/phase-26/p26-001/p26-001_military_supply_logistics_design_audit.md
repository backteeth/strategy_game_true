# P26-001: 軍事補給・兵站システム設計監査 報告書

## 1. 総合判定

**DESIGN DECIDED**

## 2. 設計確定可否

確定可。複数案の比較だけで終わらせず、実コード監査(§7〜11)に基づいて単一の
推奨設計を選定した(§38)。特に、現行コードには「未接続だが概念的に既に存在する
補給の下地」が2種類眠っている(§10)ことが監査の中心的発見であり、これが設計の
選択(§13〜16, §23〜24)を強く方向づけた。

## 3. 開始時HEAD

```
150e2aea23cd44819d2e2c997857be987f9951af
```

`git rev-parse HEAD`で確認し、想定(Phase 25最終監査コミット)と一致していた。

## 4. 監査対象コードHEAD

```
150e2aea23cd44819d2e2c997857be987f9951af
```

本タスクは設計監査(報告書のみ追加)のため、本番コード・テストコードは一切変更
しておらず、監査対象コードのHEADは開始時から変化していない。

## 5. ユーザーセーブの保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージ・コミット
していない。作業開始時・全検証コマンド実行後・コミット直前のいずれの
`git status --short`でも` M saves/savegame_v1.ron`のみが残り続けたことを確認した
(§46)。本監査ではこのファイルの内容には一度も触れていない。

## 6. Phase 25固定状態

Phase 25は`AUDIT COMPLETE, PHASE 25 READY`(HEAD=`150e2aea`)で確定済みの基準線
であり、本タスクではその意味を一切変更していない。`cargo test --all-targets`は
開始時基準どおり**1833 passed / 0 failed / 1 ignored**、`cargo clippy --all-targets
--all-features -- -D warnings`は**0 warnings**であることを本タスク末尾(§46)で
再確認し、開始時基準と完全一致した。

## 7. 現行資源・生産・備蓄モデル

`src/economy/resources.rs`。`ResourceType`は8variant:

```rust
pub enum ResourceType {
    Food, Wood, Iron, Coal, MagicCrystal,
    IndustrialGoods, MilitaryEquipment, RawMagicCrystal,
}
```

`CountryStockpile { amounts: HashMap<ResourceType, f64> }`。`get()`は存在しない
キーを`0.0`として返す(`unwrap_or(&0.0)`)。New Game(`app::loader.rs`)は初期備蓄を
一切seedしない(`CountryData::default()`の`CountryStockpile::default()`は空map)ため、
どの資源も生産が始まるまで実質0からスタートする。

生産・消費の周期は**月次**である。`economy::mod::EconomyPlugin`は
`DailySimulationSet::Economy`で`handle_daily_construction`(日次、建設進捗のみ)と
`handle_monthly_economy`(`MonthChangedMessage`購読)を`chain()`実行するが、資源の
生産・消費(`process_country_production`)は`handle_monthly_economy`の中でのみ月1回
走る。日次で資源が動く経路は存在しない。

`process_country_production`(`src/economy/production.rs`)は建物ごとに
`operation_ratio = min(workforce_ratio, logistics_ratio, input_resource_ratio)
×(is_bankrupt?0.8:1.0)`を計算し、`actual_output = base_output × level ×
operation_ratio`で生産、`actual_input = base_input × level × operation_ratio`で
`CountryStockpile::consume`により消費する(2段階: 需要集計→充足率算出→確定消費)。
`BuildingDefinition { input_resources: HashMap<ResourceType,f64>,
output_resources: HashMap<ResourceType,f64>, logistics_cost, maintenance_cost,
max_level, ... }`(`src/building/data.rs`)。`MilitaryFactory`定義
(`assets/data/buildings.ron`): `input_resources: {Iron:30.0,
IndustrialGoods:15.0}`, `output_resources: {MilitaryEquipment:20.0}`,
`max_level:10`。1建物1主力出力という慣用パターンが確立している(Farm→Food、
Mine→鉱床資源、MilitaryFactory→MilitaryEquipmentのみ)。

`MilitaryEquipment`だけは、通常の`add()`(生の`f64`加算)ではなく専用の固定小数点
API(P25-002-REV-001)を経由する:

```rust
pub const MILITARY_EQUIPMENT_SCALE: u64 = 1_000_000;
pub const MILITARY_EQUIPMENT_MAX_UNITS: u64 = 1_000_000_000_000_000;
pub fn military_equipment_value_to_units(value: f64) -> Result<u64, MilitaryEquipmentUnitsError>;
pub fn military_equipment_units_to_value(units: u64) -> f64;
CountryStockpile::add_military_equipment_produced(&mut self, produced: f64)
    -> Result<(), MilitaryEquipmentUnitsError>;
```
NaN/Inf/負数/値域超過/往復変換失敗をすべて`Err`として拒否し、備蓄を変更しない
(§7監査コメントに150年シミュレーション・28州・MilitaryFactory最大レベル10での
理論最大値算出根拠が明記されている)。他の資源(`Food`含む)はこの固定小数点経路を
経ない生の`add`/`consume`のままである。

`Food`の用途は**存在するが「消費」ではない**。月次で`has_food =
stockpile.get(Food) > 0.0`という真偽値判定のみが行われ
(`update_state_welfare_and_unrest`)、生活水準に`±0.5〜1.0`のboolean的ボーナス/
ペナルティを与えるだけで、人口比例のFood消費・配給・不足量計算は一切実装されて
いない。Foodを「消費」した実績・経路はコード中に存在しない(§13でこの事実が
案C却下の主因になる)。

国家消滅時の備蓄: `country::elimination`のテスト
`apply_destroys_all_equipment_of_disbanded_divisions_and_leaves_stockpiles_untouched`
が明示的に固定する仕様として、**消滅国のstockpileは凍結され変化せず、併合国の
stockpileにも加算されない**(Division内equipmentは破壊されるが、国家備蓄は単に
「取り残される」)。

Save V1: `CountryStockpile`はDTO変換なしにそのまま`Serialize`/`Deserialize`される
(`amounts: HashMap<ResourceType, f64>`)。`save::validate.rs`は存在するキーだけを
走査してfinite・非負を検証し(§416-432)、`ResourceType::ALL`全キーの存在を要求
しない。**したがって、`ResourceType`へ新variantを追加しても、旧Saveの
`amounts`にそのキーは単に存在せず、`get()`は`0.0`を返す**——構文的にも意味的にも
無条件で後方互換になる(§33で詳述)。

## 8. 現行Divisionモデル

`src/military/data.rs`。

```rust
pub struct DivisionDefinition {
    pub id: DivisionDefinitionId,
    pub division_type: DivisionType,      // Infantry, Artillery, Mage
    pub size: DivisionSize,               // Light, Standard, Heavy
    pub required_manpower: u64,
    pub required_equipment: f64,
    pub recruitment_days: u32,
    pub movement_speed, attack, defense, breakthrough,
    pub organization, morale, supply_usage: f32,   // ← 既存の未使用フィールド
    pub maintenance_cost: f64,
}

pub struct Division {
    pub id: DivisionId,
    pub owner: CountryId,
    pub current_state: StateId,
    pub destination: Option<StateId>,      // #[serde(default)]
    pub current_path: Vec<StateId>,        // #[serde(default)]
    pub target_state: Option<StateId>,
    pub manpower, max_manpower: u64,
    pub equipment, max_equipment: f64,
    pub organization, max_organization, morale, max_morale, experience: f32,
    pub supply_ratio: f32,                 // ← 既存だが死んだフィールド(§10)
    pub movement_progress: f32,
    pub status: DivisionStatus,            // Idle/Moving/Fighting/Retreating/
                                            //   Occupying/Disbanding/Destroyed
    pub def_id: DivisionDefinitionId,
    pub attack_power, defense_power: i32,  // #[serde(default=...)]
    pub combat_id: Option<BattleId>,       // #[serde(default)]
}
```

**`assets/data/divisions.ron`には`supply_usage`が既にDivisionType別に差別化された
値として存在する**: Infantry=1.0, Artillery(Heavy)=3.0, Mage(Light)=2.0。ただし
`DivisionDefinitionId(0)`(標準Infantry)だけが実際の募兵・New Game初期配置で使われ
(`STANDARD_INFANTRY_DEFINITION_ID`)、id(1)(Artillery)・id(2)(Mage)は現状
テストフィクスチャ以外の本番spawn経路から一切参照されない(全ファイル横断で
grepしても`war::tests`/`military::tests`/`ui::military_panel`のテストコード内
使用のみ)。つまり**現在のゲーム規模は実質的に単一師団種別**であり、兵科別差別化
(§13案B)を今すぐ実装する実データ上の根拠は薄い。

`MilitaryRegistry.definitions`はSaveへ含まれず、起動時に常に
`assets/data/divisions.ron`から再構築される。募兵→Divisionの生成経路は
`military::recruitment::process_recruitment`が日次のキュー消化時点で
`Division{ manpower: def.required_manpower, equipment: (P25-003の
`try_issue_equipment`経由で充足量), ... }`を生成する1経路のみ。New Game初期
DivisionとLoaded Game Divisionは同じ`MilitaryRegistry::from_saved_parts`/
Save DTOの往復で再構成される(P25-002-REV-001のE2Eテストが両者の一致を検証
済み)。Division分割・統合(merge/split)機能は存在しない。

補給需要を導出できる既存の値: `manpower`/`max_manpower`(現在数・定員)、
`equipment`/`max_equipment`、`status`(活動状態、7値、既存enum、新規フィールド
不要で活動倍率のkeyに転用できる)、`def.supply_usage`(師団種別ごとの基礎率、
既存フィールド、現在は死んだ`military::supply::process_supply`からのみ参照)。
**新しく保存しなければ表現できない値**: 携行在庫量そのもの(補給専用の
`current_supply`/`max_supply`)——これは既存フィールドに相当するものがなく、
Model 4(§14案4)を採用する場合にのみ必要になる新規フィールドである。

## 9. 現行地理・隣接・支配モデル

`src/state/data.rs`。`StateData`は`neighbors: Vec<StateId>`(隣接州リスト、
`#[serde(default)]`)、`owner_country_id`(法的所有)、`controller_country:
Option<CountryId>`(占領時の実効支配、`controller()`で
`unwrap_or(owner_country_id)`に正規化)、`is_sea: bool`(海域フラグ)、
`base_logistics_capacity`/`logistics_capacity`/`logistics_usage`/
`logistics_ratio`(§10で詳述)を持つ。`StateRegistry`は`StateId.0 → Vecインデックス`
のO(1)ルックアップを持ち、州は消滅・併合時も**物理削除されず**
(`state::ownership`は所有権のみ書き換える)、`StateId`は恒久的に安定している。

`CountryData.capital_state_id: StateId`(`src/country/mod.rs`)が首都を保持する
(消滅国では`current_capital_state_id()`が`None`を返す設計で、生の
`capital_state_id`フィールド自体は履歴として残す)。

移動・経路探索: `military::pathfinding::find_path`(Dijkstra、`StateNode`の
`Ord`で決定的tie-break)は`StateData.neighbors`のみをグラフとして使い、
`allowed_countries`/`hostile_countries`でフィルタする。**海上輸送・港・
輸送船・外交通行権を表すデータは一切存在しない**——`is_sea`フラグは
`state::ownership`/`war::frontline`/`diplomacy::claims`/`war::peace`等
多数箇所で「所有権移転・請求権・前線形成の対象外」として参照されるが、
`pathfinding::find_path`自体は`is_sea`を一切参照しない(海州もグラフの一部として
同じcost=1で通行可能に見える)。歩兵の海上移動を明示的に禁止する仕組みは
`pathfinding.rs`の中には存在しない。

## 10. 補給へ再利用できる既存データ

本監査で判明した最大の事実は、**「補給」を表す下地がコード中に2系統、
どちらも未接続のまま既に存在する**ことである。

**(A) 州インフラ物流(`src/logistics/`)**: `update_state_logistics`
(`src/logistics/update.rs`)が月次で`state.logistics_capacity =
base_logistics_capacity + railway_level × railway_bonus`、
`state.logistics_usage = Σ building.logistics_cost × level`、
`state.logistics_ratio = (capacity/usage).clamp(0,1)`を計算する。これは**建物の
稼働可否を左右する州インフラ容量**であり、`process_country_production`の
`operation_ratio`と`update_state_welfare_and_unrest`の生活水準計算に実際に
使われている、生きた仕組みである。しかしこれは弾薬・燃料のような
**消費フロー資源ではなく**、鉄道インフラに対する建物需要の充足率に過ぎない。

**(B) `military::supply::process_supply` + `Division.supply_ratio`**:
```rust
pub fn process_supply(military_registry: &mut MilitaryRegistry,
                       state_registry: &mut StateRegistry) {
    // 州ごとにDivision.supply_usage(定義値)を合計 → 州logistics_ratioとして
    // division.supply_ratio へ書き込む
}
```
`military::combat::calculate_combat_strength`は
`base_attack * manpower_ratio.min(equip_ratio) * org_ratio *
division.supply_ratio`という式で、**構文的には「装備充足率と並ぶ第3の補給
乗数」という、まさにPhase 26が欲しい形の合成式が既に書かれている**。

しかし、この2つは**確認済みの死んだ経路**である。`process_supply`は
`military/tests.rs`のテスト以外から一度も呼ばれず、`LogisticsPlugin::build`は
空実装(`fn build(&self, _app: &mut App) {}`)。`calculate_combat_strength`を
使う`war::combat::process_combat`も、`military::update.rs`自身のコメントと
固定テストが明示するとおり**未接続**である:
```rust
// military::update.rs:1532
/// Test U: `war::combat::process_combat`(P25-001監査で死んだ経路と確認済み)は、
```
実際に稼働している戦闘力式は`military::combat_calc::calculate_combat_readiness`
(P25-007、`military::update::process_daily_battles`から呼ばれる)であり、こちらは
`min(manpower_readiness, equipment_readiness)`のみで、`supply_ratio`を一切
参照しない。したがって現在のライブなDivisionの`supply_ratio`は初期値`1.0`から
一度も更新されず、**恒久的な無効な乗数(1.0固定)**として存在するに過ぎない。

この発見の設計上の意味(§23で詳述): `Division.supply_ratio: f32`は
**新規フィールドを追加せずとも、Phase 26が書き込む先として転用できる、
既にSaveスキーマに存在する空きスロット**である。ただし現在それを読む
死んだ経路(`military::combat::calculate_combat_strength`)・書く死んだ経路
(`military::supply::process_supply`)は、意味が異なる旧概念(インフラ物流比率の
コピー)を前提にしており、これらを「復活」させるのではなく、Phase 26の新しい
生きた計算で**上書き**する形にする必要がある(§23, §39)。

その他の再利用可能データ: `apportion_reinforcements`(最大剰余法、`u128`中間値、
`DivisionId`昇順tie-break、§19)、`EquipmentAccountingError`/
`try_issue_equipment`/`try_return_equipment`/`try_destroy_equipment`/
`quantization_delta`パターン(§21の型)、`CombatReadiness`構造体と
`calculate_combat_readiness`の2要素min設計(§24)、`DailySimulationSet`の
固定順序(§25)、`DivisionStatus`(活動倍率のkey、§18)、`def.supply_usage`
(基礎需要率、§17)。

## 11. 不足しているデータ

* 港・海上輸送船・輸送容量を表すデータ(`is_sea`はあるが輸送能力値はない)
* 州ごとの補給容量・距離減衰・鉄道/道路インフラの「軍事補給」専用値
  (`logistics_capacity`は建物用インフラであり意味が異なる、§10)
* Division携行在庫フィールド(`current_supply`/`max_supply`相当)
* 同盟国領を通した補給共有・外交的通行権の判定ロジック
* 首都占領時の代替補給源選定ロジック
* 補給専用の生産建物(`SupplyDepot`相当)、または既存`MilitaryFactory`への
  複数出力割当

現行コードに存在しない補給網を、存在するものとして設計に含めない(§13〜14の
選定はこの制約を厳格に反映している)。

## 12. 耐久装備と消費補給物資の分離

**MilitaryEquipment(既存、Phase 25で確定済み)**: 耐久ストック。募兵時に
`try_issue_equipment`で完全支給必須、casualty比例で`calculate_proportional_
equipment_loss`により破壊、非戦闘時は減らない、日次補充は`try_issue_equipment`で
不足分を備蓄から補充、復員時は`try_return_equipment`で生存装備を返却、国家消滅時は
Division装備を破壊(国家備蓄は凍結)。固定小数点scale=1,000,000、
MAX_UNITS=10^15。

**軍事補給物資(新規、MilitarySupplies)**: 日次消費フロー。募兵時は初期携行量
(採用する空間モデル次第、§14〜16)、生産源は新設の専用建物(§16)、戦闘活動・
非戦闘活動それぞれで消費(§17〜18)、不足分は日次配分(§19)、復員時は
Division携行在庫を採用する場合のみ返却(§16では非採用のため対象外)、国家消滅時は
Phase 25と同じ「消滅国stockpile凍結」方針を踏襲(§29)。

**この2つを同じ資源として扱わない**。MilitaryEquipmentを弾薬・燃料として
再消費する案(spec §5で言及)は、Phase 25が確立した「装備は耐久財であり
casualty比例でのみ破壊される」という保存則・全テスト群(P25-002〜P25-007、
88+件)の意味を破壊するため**却下する**。この却下を覆すだけの実コード上の
根拠(例えば装備とは別に弾薬概念を持つ既存データがある等)は監査中に一切
見つからなかった。

## 13. 補給資源粒度の比較

| 候補 | 内容 | 実コード適合性 |
|---|---|---|
| **案A: 単一`MilitarySupplies`** | 弾薬・燃料・食料を1本化 | **採用**。§8で確認したとおり本番spawn経路で使われるDivisionDefinitionは実質1種のみ、兵科別差別化の実データ根拠が薄い。§7のMilitaryEquipment固定小数点実装(P25-002-REV-001)をそのまま複製できる最小実装。 |
| 案B: 弾薬/燃料/軍用食料を最初から分離 | 兵科別需要 | 却下(次点でもない)。`DivisionDefinitionId(1)/(2)`が本番未使用の現状で3資源分の生産・備蓄・UI・AIを同時に設計するのは過剰実装。§8の実データが根拠を示さない。 |
| 案C: 既存Food + 新規Ammunition/Fuel | Foodの流用 | 却下。§7で確認したとおり**Foodには現在「消費」自体が実装されていない**(boolean存在判定のみ)。軍事補給の基盤を作る前に、まずFood自体に消費・配給という前例のない機能を同時発明することになり、民生・軍事需要が同じ未検証の新機構を奪い合う——P26-002の原子性(§43)を破壊する。 |
| 案D: 補給値だけを計算し物理資源を持たない | 抽象比率のみ | 却下。不変条件#2/#3(生産以外で増えない・消費以外で減らない)を検証する対象実体が存在せず、§7で確認したMilitaryEquipmentの固定小数点保存則パターン(checked u128、MAX_UNITS、往復検証)を一切再利用できない。「補給が無から生成される」spec自身の懸念どおりの構造になる。 |

**結論: 案Aを採用**(§7・§8の実コード監査が明確な反証を示した案B/C/Dを排除し、
spec §6の「明確な反証がなければ案Aを第一候補」という原則どおりの結果になった)。

## 14. 空間モデルの比較

| 候補 | 実コード適合性 |
|---|---|
| モデル1: 国家備蓄→全国へ直接供給 | 実装可能(既存`CountryStockpile`のみで完結)。ただし包囲・前線距離を一切表現しない。 |
| **モデル2: 首都から支配州の連結性判定** | 次点。既存`StateData.neighbors`+`controller()`+`capital_state_id`だけで実装可能(§9)——新規データ不要。`pathfinding::find_path`と同じグラフ探索パターンを転用できる。 |
| モデル3: 州ごとの補給容量と距離減衰 | 却下。§11のとおり鉄道・道路の「軍事補給専用」容量データが存在せず、既存`logistics_capacity`(建物用インフラ)を流用するのは意味の混同になる(§20不変条件#14と衝突)。 |
| モデル4: Division携行在庫 | 却下(P26-002範囲では)。§8のとおり新規Divisionフィールドが必要、Save意味的互換性・募兵/復員/全滅/国家消滅の全ライフサイクル会計がPhase 25と同規模で必要になり、本タスクの新規フィールド禁止(ground rule 2)にも抵触する規模。 |
| **モデル5: 段階導入** | **採用**。P26-002ではモデル1相当(空間モデルなし、国家備蓄のみ)から始め、将来モデル2(既存`neighbors`/`controller()`グラフで新規データ不要)へ拡張できるようAPI境界を設計する(§39)。 |

**結論**: モデル5(段階導入)を採用。直近の到達点はモデル1(P26-002〜P26-005相当、
§42)、次のマイルストーンとしてモデル2(P26-006相当)を明記する。モデル3・4は
現時点のデータでは正当化できないため却下する。

## 15. 時間モデルの比較

候補は「戦闘参加によって需要が決まり、その補給率が同じ戦闘結果を決める」という
循環を避けなければならない(spec §12)。§9で確認した`military::update::
handle_daily_military`の実際の日次順序:

```
1. process_recruitment (募兵)
2. apply_daily_reinforcement (manpower補充、当日の戦闘損耗は含めない)
2.5 apply_daily_equipment_reinforcement (equipment補充、同上)
3. validate_and_stop_invalid_movements
4. process_movement (移動・侵攻・戦闘開始を含む)
4. process_daily_battles (当日の戦闘計算)
5. resolve_finished_battles
6. cleanup
7. process_org_recovery
```

**採用する時間モデル**: manpower/equipment補充の直後・移動処理の直前という
既存の空き位置(コメント上の「2.5」相当の直後)に、当日の**移動前・戦闘前**の
Division状態(前日終了時点のmanpower・equipment・status)から需要を確定し、
即座に配分する。これにより:

* 需要は「今日これから起きる移動・戦闘の結果」ではなく「昨日までの状態」から
  決まるため、循環が起きない(manpower/equipment補充が既に採用している
  「前日までの損耗のみを対象とする」設計方針と完全に整合)。
* 当日算出した供給充足率は、その直後に走る`process_movement`/
  `process_daily_battles`へそのまま使われる——「需要と効果は同じ日」を実現
  しつつ、「参照する状態」と「影響を受ける対象」を時間的にずらすことで
  循環を切る(spec §12の解決策「戦闘開始時点の状態から需要を決める」を採用)。
* 移動中に今日侵攻・戦闘へ突入するDivisionは、当日の活動倍率としては
  「移動」(前日終了時点のstatus)を使う。「戦闘」倍率が適用されるのは
  翌日以降。これは既存の補充システムが採用する「今日の損耗は明日以降」と
  同じ非循環規約であり、既存パターンからの逸脱がない。
* 生産(月次)→配分(日次)は既に非同期(月初に生産された備蓄はその月・
  以降のどの日でも使用可能)であり、同月生産分を同日使用できるかという
  spec §12の論点は「はい、既存の月次生産→日次消費の関係と同じ」でよい
  (Iron/IndustrialGoods等、既存資源も同じ非同期関係)。

## 16. 携行在庫の要否

**P26-002〜当面のマイルストーン(モデル1)では不要**。§14で決定したとおり
携行在庫はモデル4専用の概念であり、モデル1・2では国家備蓄から直接需要へ
配分するため、Division側に保存すべき新規状態は生まれない(§8で確認した
既存Divisionフィールドだけで完結する)。

将来モデル4へ拡張する場合の要否は「必要」——ただしspec §15が明記するとおり
Phase 25と同規模のライフサイクル実装(募兵初期支給・日次補充・日次消費・
復員返却・全滅破壊・撤退強制解散破壊・国家消滅破壊・Division分割統合・
Save migration)が必要になり、これは本監査の対象外の将来判断として明記する
に留める(§39)。

## 17. 正規補給需要式

基礎需要は`DivisionDefinition.supply_usage`(§8で確認した既存フィールド、
現在Infantry=1.0/Artillery=3.0/Mage=2.0とunit種別で既に差別化済み)をそのまま
再利用する——新規フィールド不要。

```
strength_factor_units = manpower_readiness_units   // combat_calc.rsと同じ
                                                     // 正規化パターン(§10)
                       = manpower * COMBAT_READINESS_SCALE / max_manpower
                         (max_manpower==0 のときは0、ゼロ除算しない)

base_demand_units = round(supply_usage(def) * SUPPLY_SCALE)

daily_demand_units = base_demand_units
                      * strength_factor_units / COMBAT_READINESS_SCALE
                      * activity_multiplier_units(status) / SUPPLY_SCALE
                      (すべてchecked u128中間演算、§21)
```

`current manpower`比例(`strength_factor`)を使うことで、損耗したDivisionの
需要が自動的に減る。これは「補給不足効果を回避する抜け道」ではなく、
「戦える人数が減れば必要な物資量も減る」という現実的な意味であり、既に
manpower/equipment readinessが戦闘力を制約している(§24)ため**二重の
不足ペナルティにはならない**——需要側の縮小と戦闘力側の制約は別レイヤーで
両立する。`max_manpower`のみの比例(壊滅的損耗Divisionが満編制と同じ需要を
要求する)は不合理なため不採用。

## 18. 活動倍率

`DivisionStatus`(既存enum、新規フィールド不要)をkeyにした倍率テーブルを
提案する:

```
Idle / Occupying    : 1.0倍(基礎維持消費)
Moving               : 1.0倍(P26-002〜モデル1範囲では移動固有の増加は設計しない。
                        将来moving特有の燃料消費を導入する余地は残す)
Fighting             : 2.0倍(戦闘活動による増加消費、spec §11のとおり
                        最初の効果は1箇所に限定する方針と矛盾しないよう、
                        倍率自体は単純な定数から始める)
Retreating           : 1.0倍
Disbanding / Destroyed: 需要計算の対象外(除外——まもなく消える師団に請求しない)
```

浮動小数点はそのまま会計に使わず、`SUPPLY_SCALE`(§21)による整数比率の
正規方式(`activity_multiplier_units: u64`)として実装する。

## 19. 日次配分式

```
S_c = 配分前の国家補給備蓄(units)
D_i = Division iの日次需要(units、§17)
T_c = min(S_c, ΣD_i)
I_i = Division iへの配分(units)
必須条件: 0 <= I_i <= D_i, ΣI_i = T_c, S_after = S_before - T_c
```

`military::reinforcement::apportion_reinforcements`の**アルゴリズム**
(最大剰余法、`u128`中間値、剰余降順→`DivisionId`昇順tie-break、`HashMap`走査順
非依存)はmanpower・equipment・supplyの3ドメインに共通する数学として非常に
優れているが、spec §9自身が指摘するとおり**関数そのものを跨ドメイン呼び出し
しない**。理由: `apportion_reinforcements`は`military::reinforcement`
モジュールの内部実装として、manpower補充の呼び出し規約(適格性判定・
`CountryRegistry`/`MilitaryRegistry`の借用パターン)と密結合しており、
supplyから直接呼ぶとモジュール境界(Phase 21以降繰り返し確立されてきた
`pub(in crate::state)`的な設計原則、§9で確認した`state::ownership`の
先例)を壊す。

**設計**: 最大剰余法の**純粋なアルゴリズム部分**(budget・requests・
tie-break規約のみに依存する部分)を、Idの型に依存しないジェネリックな共有
関数として抽出し(例: `military::apportionment::largest_remainder_allocate
<Id: Ord + Copy>(budget: u64, requests: &[(Id, u64)]) -> HashMap<Id, u64>`)、
`apportion_reinforcements`・equipment補充・新設のsupply配分の3つがそれぞれ
自分のドメイン固有の需要計算・適格性判定を経てこの共有関数を呼ぶ形にする。
これにより数学は1箇所に集約されつつ、各ドメインのAPI境界(意味の混同防止、
spec §9の要求)が保たれる。この抽出自体はP26-002の範囲外(P26-002はDivision
需要・日次配分を対象外とする、§43)であり、日次配分を実装する後続タスク
(§42のP26-004相当)で行う設計として明記する。

## 20. 不足時の按分方式

§19のとおり最大剰余法(全Division比例配分、`DivisionId`昇順の決定的tie-break)
を採用する。DivisionId順先着は早いIDのDivisionを恒常的に優遇し不公平、
戦闘中優先やプレイヤー設定優先度はP26-002〜P26-005(§42)の範囲を超える
将来機能(AI・UIとの接続が必要)として明記するに留め、最初の実装では
「全員に需要比例で公平に配る」最大剰余法のみを採用する。

## 21. fixed-point scale・丸め・最大値

`MilitaryEquipment`の固定小数点実装(P25-002-REV-001、§7)を全く同じ設計方針で
複製する:

```rust
pub const MILITARY_SUPPLIES_SCALE: u64 = 1_000_000;      // MilitaryEquipmentと同じscale
pub const MILITARY_SUPPLIES_MAX_UNITS: u64 = 1_000_000_000_000_000;  // 暫定値、
    // P25-002-REV-001 §9-11と同じ手法(往復変換の実測サンプリング+150年理論最大値
    // の桁違い余裕の確認)による正式な算出をP26-002実装時に行うことを前提とした
    // 暫定採用値。師団あたり日次需要が高々数units、師団数が現状1桁〜将来数百でも
    // 150年間の理論最大備蓄は10^8〜10^9のオーダーに収まり、10^15に対して
    // 十分な余裕がある。
pub fn military_supplies_value_to_units(value: f64) -> Result<u64, MilitarySuppliesUnitsError>;
pub fn military_supplies_units_to_value(units: u64) -> f64;
```

丸めは量子化段階(`value * SCALE`を`round()`)でのみ行う。往復変換
(`units → f64 → units`)が元と一致しない場合は`Err(RoundTripFailed)`として
拒否し、黙って近似値へフォールバックしない——P25-002-REV-001と全く同じ規律。

## 22. 世界補給物資保存式

P26-002の範囲(Division携行在庫なし、モデル1)では単純に:

```
Σ_c stockpile_c(t+1) = Σ_c stockpile_c(t) + Σ production(t→t+1) - Σ consumption(t→t+1)
```
ただし消滅国のstockpileは§7・§29のとおり凍結され、以後は「世界の合計」から
実質的に除外される(MilitaryEquipmentの既存精算方針と同一)。携行在庫
(モデル4)を将来採用する場合は、`Σ_c stockpile_c + Σ_d division_d.current_supply`
という拡張した保存式になる(MilitaryEquipmentの`stockpile + Σ
division.equipment`という既存の保存式と同型、§7監査時に確認した
`p25_006_rev_001`のconservationテストと同じ形)。

## 23. 補給切れの効果

**最初の実装では戦闘力のみに限定する(案A)**。§10で確認した既存の
`CombatReadiness`(P25-007)構造体:

```rust
pub struct CombatReadiness {
    pub manpower_readiness_units: u64,
    pub equipment_readiness_units: u64,
    pub combat_readiness_units: u64,   // = min(manpower, equipment)
}
```
これを自然に3要素へ拡張する:
```rust
pub struct CombatReadiness {
    pub manpower_readiness_units: u64,
    pub equipment_readiness_units: u64,
    pub supply_readiness_units: u64,       // 新規
    pub combat_readiness_units: u64,       // = min(manpower, equipment, supply)
}
```
`min`を採用する根拠は、Phase 25自身のテストが明示的に固定している設計哲学
そのものである:
```rust
// readiness_18_4: min(50%,50%)=50%であり、50%×50%=25%にしてはいけない
```
積による合成(50%×50%=12.5%)はPhase 25が明示的に拒否した設計であり、
Phase 26でも同じ原則(「最も不足している要素が戦闘力を制約する」)を維持する。

移動速度補正(案B)・organization回復停止(案C)・Attrition(案D)は、
ground rule 2(移動速度補正・Attritionの実装禁止)にも抵触するため
**本タスクでは設計するが実装しない**。organization回復経路
(`military::update::process_org_recovery`)は存在確認済みだが、これへの
接続はP26の後続タスク(§42)として明記するに留める。

**既存の死んだ`Division.supply_ratio`フィールド・`military::supply::
process_supply`・`military::combat::calculate_combat_strength`との関係
(重要)**: §10で確認したとおり、これらは確認済みの死んだ経路であり
(`military::update.rs`自身のテストが明示)、Phase 26はこれを「復活」させる
のではなく、`Division.supply_ratio: f32`という**既にSaveスキーマに存在する
空きフィールド**を、新しい生きた計算(§17〜19)の書き込み先として転用する
ことを推奨する。これにより新規フィールド追加なし・Save移行なしで実装できる。
ただし、この転用は**P26-002の範囲外**(P26-002はDivision・combat変更を
対象外とする、§43)であり、実際に`calculate_combat_readiness`へ
`supply_readiness_units`を接続するのはPhase 26の後続タスク(§42の
P26-005相当)である。旧`process_supply`/`calculate_combat_strength`/
`war::combat::process_combat`は削除・改変せず(本タスクは無関係な差分を
変更しない)、Lowの技術負債として記録する(§45)。

## 24. manpower・equipment readinessとの合成

§23で決定したとおり、`combat_readiness_units = min(manpower_readiness_units,
equipment_readiness_units, supply_readiness_units)`という3要素minを採用する。
補給充足率50%を既存のmanpower50%・equipment50%と合成したとき、
**積で12.5%にはせず、min(50%,50%,50%)=50%とする**——Phase 25が明示的に
確立した設計を維持する(§23参照)。装備充足率と補給充足率は別のunits
(`equipment_readiness_units`と`supply_readiness_units`)として個別に保持し、
どちらも`min`の入力としてのみ扱う(不変条件#14「装備充足率と補給充足率を
混同しない」を、この構造体の型レベルでの分離によって保証する、§41)。

## 25. System順序

§15で決定した挿入位置を`DailySimulationSet::MilitaryAction`内、
既存の`handle_daily_military`関数内(Bevyの`SystemSet`順序自体は
`app::time::GameTimePlugin`が`configure_sets`で固定済みであり、新たな
`SystemSet`の追加は不要)に以下のとおり明記する:

```
DailySimulationSet::Economy (月次、handle_monthly_economy内)
  → 新設SupplyFactory相当建物によるMilitarySupplies生産(§16)

DailySimulationSet::MilitaryAction (handle_daily_military内、日次)
  1. process_recruitment
  2. apply_daily_reinforcement (manpower)
  2.5 apply_daily_equipment_reinforcement (equipment)
  2.7 [新設] 補給需要計算 + 日次配分(§17〜19、前日終了時点の状態から算出)
  3. validate_and_stop_invalid_movements
  4. process_movement
  4. process_daily_battles (§23の3要素minを使用)
  5. resolve_finished_battles
  6. cleanup
  7. process_org_recovery
```

## 26. 同日生産・配分・戦闘の意味

生産は月次(§7)、配分は日次(§25の2.7)、戦闘は日次(§25の4)。ある月の1日目に
生産された備蓄は、その月のどの日の配分にも使用できる(既存Iron/
IndustrialGoods等と同じ非同期関係、§15)。配分は移動・戦闘の**前**に前日終了
時点の状態から確定するため、当日の戦闘へその日の配分結果(供給充足率)を
反映しつつ、その戦闘自体が同じ日の需要計算に影響を与えることはない
(§27で詳述)。補給不足はその日の戦闘へ反映し、翌日へ持ち越さない
(一時的な日次値、§33の案1採用に対応)。

## 27. 循環依存の解消方法

§15・§26で述べたとおり、「需要計算の入力」を常に**前日終了時点**の
Division状態(manpower・equipment・status)に固定し、「需要計算の出力
(供給充足率)」を**当日これから起きる**移動・戦闘の入力として使うことで、
同一の状態変化(今日の戦闘結果)が同じ日の需要計算へフィードバックする経路を
物理的に作らない。これは既存のmanpower/equipment日次補充が既に採用している
「今日の損耗は明日以降の補充対象」という規約と完全に同型であり、既存
コードベースに新しい時間モデルの概念を持ち込まない。

## 28. 包囲・占領・海外領土

§14で採用したモデル5(段階導入)のうち、直近マイルストーン(モデル1)では
**空間的な補給可能性判定を行わない**——国家が存在する限り、その国の全
Divisionは国家備蓄から均等に配分対象となる。スペックのシナリオ
(Country AのDivision DがState Sにいる→Sへの陸上経路が敵に遮断→翌日以降の
補給→Sの支配者が変更→Country Aが消滅)は、モデル1では:

* 経路遮断は一切判定に影響しない(Dは引き続きAの国家備蓄から配分される)。
* Sの支配者変更(占領)もDの配分に直接影響しない(Dの`owner`は変わらない
  ため)。
* Country Aが消滅した場合は、§7・§29で確認した既存方針(消滅国stockpile
  凍結、Division装備破壊と同じ扱い)を補給物資にも適用する。

将来モデル2(次のマイルストーン)を導入する際は、`capital_state_id`から
`neighbors`+`controller()`グラフを辿れる州のみを「補給可能」と判定する
(`pathfinding::find_path`と同じ探索パターンの転用、§9・§14)。首都占領時の
代替補給源選定・海外領土・港はモデル2でもまだ扱わず、§11で明記した
不足データが揃うモデル3相当の将来タスクへ持ち越す。

## 29. 国家消滅・復員・削除

P26-002〜モデル1の範囲(携行在庫なし)では、消費物資は常に国家備蓄から
直接消費されるため、**Division側に返却対象の在庫はない**。したがって
募兵・復員・全滅・国家消滅のいずれでも追加の会計は不要——これは携行在庫
方式(モデル4)と比べた際の、モデル1・2最大の実装コスト上の利点である
(spec §15が明記する「携行在庫方式を採用する場合はPhase 25と同程度の
ライフサイクル実装が必要」という重い代償を、この段階では負わない)。

消滅国のstockpileは§7・§28で述べたとおり、既存のMilitaryEquipment凍結方針
(併合国へは移転しない)をそのまま踏襲する。配分済み当日値(§26の一時的な
日次充足率)は、国家消滅の当日に処理中だったとしても、単なる一時計算値
であり永続保存しない(§33)ため特別な清算処理は不要。

## 30. 原子性と失敗処理

Phase 25の`EquipmentAccountingError`/`try_issue_equipment`/
`try_destroy_equipment`/`quantization_delta`/`validate_equipment_state`
パターン(§10)をそのまま踏襲する:

* NaN・Infinity・負数は量子化関数(`military_supplies_value_to_units`)の
  入り口で`Err`として拒否し、備蓄を変更しない。
* overflow/underflowは`checked_add`/`checked_sub`(`u128`中間値)で拒否し、
  `Err(ArithmeticOverflow)`を返す。
* 不足(shortfall)は`Err(InsufficientStockpile)`のような明示的な「通常の
  不足」バリアントとして扱い、`panic!`しない。無効値エラー
  (`NonFinite`/`Negative`/`OutOfRange`/`RoundTripFailed`)とは型レベルで
  区別する列挙値にする(不変条件#7・#8)。
* 国家単位の配分は「全対象Divisionの需要計算・検証を終えてから初めて状態を
  変更する」二段階(検証→適用)構成にする——`apply_daily_reinforcement`
  (§9)と同じ設計方針。1つの不正Division(例: manpower>max_manpower等の
  異常状態)があれば、その国の配分処理全体をスキップし(`debug!`ログのみ、
  panicしない)、他国の配分には一切影響しない。

## 31. 複数国家分離

`apply_daily_reinforcement`(§9・§30)が確立した「国家ごとに独立ループ、
`owners.sort_by_key(|c| c.0)`で決定的な処理順」パターンをそのまま踏襲する。
ある国家の帳簿不整合・不正状態は、その国の配分をスキップするだけで他国の
配分には一切影響しない(不変条件#9・#17)。

## 32. 決定性

* `HashMap`走査順に依存しない: 全ての集計・按分は明示的に`sort_by_key`
  した`Vec`へ変換してから処理する(§9・§19・§31の全パターンが既に
  これを徹底している)。
* 同じ状態から同じ配分結果になる: 最大剰余法(§19)は`u128`中間値の
  純粋関数であり、浮動小数点の非決定性を含まない。tie-breakは
  `DivisionId`昇順で完全に決定的。
* New GameとLoaded Gameで同じ状態なら同じ結果になる: §7・§34で述べた
  とおり、`ResourceType`への新variant追加はSave V1の構文・意味の両方に
  影響しないため、両者は自然に一致する(§34で詳述)。

## 33. Save V1／V2判断

**Save V1を維持する(新V2・migration不要)**。根拠:

* 構文互換性: `CountryStockpile.amounts`は`HashMap<ResourceType, f64>`の
  素の`Serialize`/`Deserialize`であり、旧Save RONに新variantのキーが
  存在しなくても`serde`のデシリアライズは失敗しない(HashMap自体は
  存在キーのみを復元する)。
* 意味的互換性: `CountryStockpile::get()`は不在キーを`0.0`として返す
  (§7)。New Gameも初期stockpileを一切seedしない(§7、`CountryData::
  default()`は空map)ため、**新資源についてNew GameとLoaded Old Saveは
  どちらも「0からスタート」という同じ意味論**になり、旧Saveだけが
  無限や0という不整合な特別扱いにはならない(spec §17の懸念に対する
  直接的な回答)。
* P26-002がDivision・combat・移動を一切変更しないため(§43)、旧Saveの
  ロード後にゲームプレイ上観測できる差異はそもそも生まれない
  (供給充足率がまだ何にも接続されていないため)。
* migration直後の増殖・消失、migration再保存の要否、といったV2特有の
  懸念はP26-002の範囲では発生しない(新フィールドがゼロのため)。

この判断はP26-002限定であり、将来Division携行在庫(モデル4)を採用する
場合は、新規Divisionフィールドが必須になり、そのフィールドを
`#[serde(default)]`で追加できるかどうかを改めてそのタスクの監査時に判断
する必要がある(§16・§39で明記)。

## 34. New Game／Loaded Game意味的互換性

§33で述べたとおり、新資源は両方とも初期値0から始まる。P26-002には
Systemの配線(生産・消費・配分)を一切含めない(§43)ため、New Game・
Loaded Gameのどちらであっても、P26-002時点では新資源のstockpileは
**恒久的に0のまま変化しない**——両者は自明に一致する。後続タスク
(§42のP26-003以降)で生産・消費が配線された時点で、
P25-002-REV-001が確立した検証パターン(`test_o_new_game_and_loaded_
game_produce_identical_stockpile`相当のE2Eテスト)を同じ形で再利用できる。

## 35. AIへの引き継ぎ

実装は対象外だが、**現行AIが新資源を一切生産できず機能停止する設計には
しない**という不変条件(#26)を満たす最小仕様を明記する: §7で確認した
とおり、現在の建物生産は「レベル×稼働率」で自動的に行われ、**プレイヤーも
AIも生産物の配分比率を選択する仕組みが存在しない**(1建物1主力出力の
慣用パターン)。したがって、MilitarySuppliesの生産を既存資源と同じ
自動生産パターン(新設`SupplyFactory`相当の建物、§16)に乗せる限り、
AIは既存の`country_ai`の建設判断ロジック(既存資源と同列に扱われる)だけで
自然に生産できる——AI専用の新しい意思決定を今すぐ実装する必要がない。
将来のAI接続で必要になる最小仕様(実装しない、方針のみ記録):
備蓄目標日数、募兵による将来需要の先読み、戦争中の備蓄目標引き上げ、
不足時の募兵/攻勢停止、装備生産・Food等民間資源との生産競争時の優先度。

## 36. UIへの引き継ぎ

P26-002の範囲ではUIは一切不要(新資源はまだ何のSystemにも接続されない
ため、UIがなくてもテスト可能——`cargo test`のみで完結)。将来UIが
最低限表示すべき値(実装しない、方針のみ記録): 国家補給備蓄、日次生産、
日次総需要、備蓄残存日数、Division補給充足率、補給不足警告、戦闘力への
補正、補給不可能理由、地図上の補給状態、募兵後の予測需要。UI導入前の
中間状態(数System分)はプレイ上「何も変わらない」ため許容できる
(既存`ui::economy_panel`が新resourceType自体をどう扱うかは別途確認が
必要だが、UIパネルが未知のResourceTypeへ即座にpanicする実装でない限り、
表示欄が1つ増えないだけで済むはずである——本タスクではUIコード自体は
読んでいないため、この点はP26-002実装時の受け入れ条件に含めることを
推奨する)。

## 37. 候補案比較表

| 軸 | 案A(単一資源)+モデル5(段階1) |
|---|---|
| 耐久装備との分離 | 型レベルで完全分離(§12) |
| 世界保存則 | P25-002-REV-001と同型の固定小数点保存則(§21・22) |
| 空間性 | 段階1は無し、段階2で首都連結性(§14) |
| 時間性 | 前日状態→当日効果、循環なし(§15・27) |
| 決定性 | 既存パターン完全踏襲(§32) |
| 原子性 | 既存パターン完全踏襲(§30) |
| 国家間分離 | 既存パターン完全踏襲(§31) |
| Save互換 | V1維持、新フィールド0(§33) |
| AI互換 | 既存自動生産パターンに乗るため機能停止なし(§35) |
| UI負担 | P26-002時点でゼロ(§36) |
| Phase25互換 | MilitaryEquipmentの意味・テストに一切触れない(§12) |
| Phase23・24互換 | manpower/workforce計算に一切触れない |
| 実装規模 | P26-002は既存equipment基盤の複製のみ、最小 |
| 将来拡張 | モデル2への境界を最初から意識した設計(§14・39) |
| ゲーム性 | 段階的にしか兵站の緊張感を提供できないが、着実に拡張可能 |

## 38. 採用する設計

* 補給資源粒度: **案A(単一`MilitarySupplies`)**(§13)
* 空間モデル: **モデル5(段階導入)、直近はモデル1相当**(§14)
* 時間モデル: **前日終了時点の状態から需要確定→当日の移動・戦闘へ適用**(§15)
* 携行在庫: **当面不要、モデル4は将来判断**(§16)
* 補給切れの効果: **戦闘力への3要素min拡張のみ(案A)**(§23・24)
* fixed-point: **MilitaryEquipmentと同じSCALE=1,000,000の複製**(§21)
* Save: **V1維持**(§33)
* `Division.supply_ratio`の扱い: **既存の死んだフィールドを新しい意味で
  転用する(P26-002の範囲外、後続タスクで実施)**(§10・23)

## 39. 次点案

* 空間モデルの次のマイルストーンとして**モデル2(首都連結性)**を明記する
  (§14) ——新規データ不要で導入できる最も自然な拡張。
* 補給資源粒度は将来的に**案C(Foodの実消費機構を先に整備した上での統合)**
  を再検討する余地を残す——ただし現状はFood自体に消費機構がなく(§7)、
  時期尚早。
* 携行在庫(モデル4)はゲーム性の観点で最終的に望ましい可能性が高いが、
  Phase 25と同規模の実装が必要になるため、Phase 26の後半または
  Phase 27以降の独立した監査タスクとして扱うことを推奨する。

## 40. 却下案と理由

* 案B(兵科別分離、§13): 本番spawn経路で使われるDivisionDefinitionが
  実質1種のみという実データに反する過剰実装。
* 案C(Food流用、§13): Foodに消費機構自体が存在せず、同時発明はP26-002の
  原子性を破壊する。
* 案D(物理資源なしの比率のみ、§13): 世界保存則を検証する実体が存在せず、
  Phase 25が確立した固定小数点保存パターンを再利用できない。
* モデル3(州補給容量+距離減衰、§14): 鉄道・道路の「軍事補給専用」
  容量データが存在せず、既存`logistics_capacity`(建物インフラ)の流用は
  意味の混同になる。
* モデル4(携行在庫、§14、P26-002範囲): 新規Divisionフィールドが必要
  (ground rule 2違反)、Phase 25同規模のライフサイクル実装が必要。
* 積による戦闘力合成(§23・24): Phase 25自身のテストが明示的に拒否した
  設計(`readiness_18_4`)を継承する。
* MilitaryEquipmentを弾薬として再消費する案(§12、spec §5言及): Phase 25の
  保存則・全テスト群を破壊する。反証根拠は監査中に見つからなかった。

## 41. 必須不変条件

各不変条件を、保証する単位とともに記す。

| # | 不変条件 | 保証単位 |
|---|---|---|
| 1 | 装備と補給物資を混同しない | 型レベル(国家単位/Division単位両方の型定義) |
| 2 | 生産以外で世界の補給物資が増えない | 国家単位(`add_military_supplies_produced`の単一入口) |
| 3 | 消費以外で世界の補給物資が減らない | 国家単位(`consume`系APIの単一入口) |
| 4 | 配分量合計が国家備蓄減少量と一致する | 国家単位(§19の`ΣI_i=T_c`検証) |
| 5 | 国家備蓄を負数にしない | 国家単位(checked演算、§21・30) |
| 6 | 配分が需要を超えない | Division単位(§19の`I_i<=D_i`数学的保証) |
| 7 | 不足をpanicや会計エラーとしない | 国家単位/Division単位(§30のErrバリアント設計) |
| 8 | 無効値と不足を区別する | 型レベル(§30のエラー列挙) |
| 9 | 複数国家間で補給を混入させない | 国家単位(§31の独立ループ) |
| 10 | HashMap順に依存しない | 世界単位(§32の明示的sort) |
| 11 | 同じ状態から同じ配分結果になる | 世界単位(§32の純粋関数設計) |
| 12 | Save/Loadで生産・消費を二重適用しない | 世界単位(P26-002はSystem未配線のため自明に満たす、§34) |
| 13 | New GameとLoaded Gameで同じ状態なら同じ結果になる | 世界単位(§34) |
| 14 | 装備充足率と補給充足率を混同しない | 型レベル(`CombatReadiness`の別フィールド、§23・24) |
| 15 | 補給不足の戦闘補正を二重適用しない | Division単位(`min`合成、1箇所のみ、§23・24) |
| 16 | 需要0で0除算しない | Division単位(§17の`max_manpower==0`ガード) |
| 17 | 消滅国家の補給会計が明確 | 国家単位(§29、MilitaryEquipmentと同一の凍結方針) |
| 18 | Division削除時の未消費補給の扱いが明確 | Division単位(§29、携行在庫なしのため対象自体が存在しない) |
| 19 | manpower・equipment補充と補給配分の関係が明確 | Division単位(§25のSystem順序で明記) |
| 20 | Phase 25の装備保存則を維持する | 世界単位(§12、MilitaryEquipmentへ一切触れない) |
| 21 | Phase 23・24のA・B・L・K・X・population・workforceを変更しない | 世界単位(本設計はこれらの値を一切参照・変更しない) |
| 22 | Attritionを先行実装しない | (ground rule準拠、§23で明記) |
| 23 | 生産・需要・配分・効果のSystem順序が循環しない | 世界単位(§25・27) |
| 24 | fixed-pointのscale・丸め・最大値が明確 | 世界単位(§21) |
| 25 | 150年シミュレーションで値域が安全 | 世界単位(§21、暫定値の根拠と実装時の再検証を明記) |
| 26 | AI国家が新資源を生産できず即座に機能停止しない | 国家単位(§35、既存自動生産パターンへの追従) |
| 27 | 動員0・装備完全充足・補給完全充足時に既存挙動を維持する | Division単位(`min`合成でsupply_readiness=100%のとき既存の2要素minと同じ結果になる、§24) |

## 42. Phase 26の段階分割

P25の段階分割(322b33f原始プリミティブ→9342cdd固定小数点訂正→17f54ae募兵接続
→0c57beb戦闘損失→26a8629日次補充→7b20ca4復員返却→6023bd5強制除隊会計→
0adcfcc戦闘力反映→3f46069/150e2ae監査)と同じ「原始プリミティブ→1本ずつ
配線→最終監査」という粒度を踏襲する。

* **P26-002**: 補給資源とfixed-point会計基盤のみ(§43で詳述)。
* **P26-003**: 補給物資の生産(新設`SupplyDepot`相当の建物、§16)を月次経済へ
  配線。
* **P26-004**: Division日次需要計算+国家単位の日次配分(§17〜20、§19の
  `largest_remainder_allocate`共有ヘルパー抽出を含む)。まだ戦闘力へは
  反映しない。
* **P26-005**: `CombatReadiness`への3要素min接続(§23・24)。Phase 25の
  P25-007と同型の1タスク。
* **P26-006**: 空間モデル2(首都連結性判定、§14・28)への拡張。
* **P26-007**: 国家消滅・占領・海外領土のモデル2版会計整理(§28・29の
  拡張)。
* **P26-008**: AI接続(§35の備蓄目標・募兵/攻勢停止判断)。
* **P26-009**: UI可視化(§36)。
* (将来・Phase26範囲外候補): 携行在庫(モデル4)、移動速度補正、
  organization回復停止、Attritionは、いずれもspec ground ruleにより
  Phase 26として一括実装しない大規模タスクであり、個別の設計監査を
  経てから着手すべき将来候補として記録するに留める。

## 43. P26-002の最小実装範囲

**目的**: `ResourceType::MilitarySupplies`とその固定小数点会計基盤のみを
追加する。Phase 25のP25-002(`322b33f`)と全く同じ粒度・同じ設計方針
(P25-002-REV-001の固定小数点訂正を最初から織り込む)。Division・
combat・movement・AI・UIには一切触れない。

**変更予定ファイル**: `src/economy/resources.rs`(既存の
`MilitaryEquipment`固定小数点実装のすぐ隣に、同型の`MilitarySupplies`
実装を追加。`ResourceType`enumへの新variant追加、`ResourceType::ALL`・
`display_name`の更新を含む)。既存の`src/military/supply.rs`
(死んだ`process_supply`)には触れない——別モジュールとして共存させ、
混同を避ける。

**新規型・API**:
```rust
// src/economy/resources.rs へ追加
ResourceType::MilitarySupplies,   // 9番目のvariant

pub const MILITARY_SUPPLIES_SCALE: u64 = 1_000_000;
pub const MILITARY_SUPPLIES_MAX_UNITS: u64 = /* §21実装時に正式算出 */;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilitarySuppliesUnitsError { NonFinite, Negative, OutOfRange, RoundTripFailed }

pub fn military_supplies_value_to_units(value: f64) -> Result<u64, MilitarySuppliesUnitsError>;
pub fn military_supplies_units_to_value(units: u64) -> f64;

impl CountryStockpile {
    pub fn add_military_supplies_produced(&mut self, produced: f64)
        -> Result<(), MilitarySuppliesUnitsError>;
    pub fn try_consume_military_supplies(&mut self, amount: f64)
        -> Result<(), MilitarySuppliesUnitsError>;  // checked、原子的、
                                                       // 不足時は備蓄を変更せずErr
}
```

**正規式**: §21のとおり。`military_equipment_value_to_units`/
`units_to_value`と全く同じ丸め・往復検証規約(量子化段階でのみ`round()`、
`NaN`/負数/値域超過/往復失敗を`Err`)。

**不変条件**: §41の#2, #3, #5, #7, #8, #10, #11, #16, #24を、
「国家備蓄1個」の粒度で満たす(Division・配分はまだ存在しないため、
#4, #6, #9, #14, #15, #19, #27は本タスクでは自明に満たされる=対象自体が
存在しない)。

**受け入れ条件**:
* `ResourceType::ALL`が9要素になり、既存の8資源のテスト・挙動を一切
  変更しない。
* 新規APIの単体テスト(finite/非負/往復変換/overflow拒否/0円滑な
  no-op)がP25-002-REV-001と同水準で揃う。
* 世界保存則を検証する純粋関数(生産量Σ・消費量Σ・stockpile変化量Σの
  一致を検証する)を持つ。
* 旧Save(このタスク以前に保存されたRON)を読み込んでも`MilitarySupplies`が
  `0.0`として復元され、既存の資源には一切影響しない(E2Eテストで確認)。
* `cargo test --all-targets`が既存1833件+新規テスト分すべてpassし、
  `cargo clippy --all-targets --all-features -- -D warnings`が0警告を
  維持する。

**必要テスト**: 単体テスト(`src/economy/resources.rs`内、P25-002-REV-001の
テスト群と同型)、E2Eテスト(`tests/p26_002_*_e2e_test.rs`、旧Save読込・
New Game/Loaded Gameの一致・保存則検証)。

**対象外**: Division需要計算、日次配分、戦闘力補正、移動速度補正、
経路探索・空間モデル、携行在庫、Attrition、AI、UI(§43自身のタイトル
どおり「会計基盤のみ」)。

**Save判断**: §33のとおりSave V1を維持する。新規フィールドは追加しない
(`ResourceType`という既存のHashMap keyが1つ増えるだけであり、
`#[serde(default)]`相当の後方互換性は`HashMap`の性質上自動的に成立する)。

**Phase 25保護条件**: `src/military/equipment.rs`・
`src/economy/resources.rs`内の`MilitaryEquipment`関連コード・
定数(`MILITARY_EQUIPMENT_SCALE`等)・既存P25 E2Eテスト群を一切変更しない。
新設の`MilitarySupplies`関連コードは独立した定数・関数・型として並存
させる(名前空間の混同を避けるため、既存の`military_equipment_*`関数名
パターンとは明確に区別できる`military_supplies_*`という命名を使う)。

**想定コミット単位**: 1コミット(`feat: add military supplies accounting
primitives`)。P25-002が単独コミットで完結した粒度(§7のP25履歴参照)と
同じ規模を想定する。

## 44. 対象外を先行実装していないこと

本タスクは設計監査のみであり、`src/`配下のファイルは1バイトも変更して
いない(§4)。§43で定義したP26-002の新規型・API・ResourceType variantは
いずれも**この報告書の中でのみ**記述されており、実コードには一切追加して
いない。一時的なRustテストファイル(調査用)は作成・削除のいずれも行って
おらず、本監査は既存コードの`Read`/`Grep`/`cargo test`/`cargo clippy`の
実行のみで完結した(実データの構造確認は既存テスト・既存アセット
(`assets/data/*.ron`)の読み取りだけで十分だったため)。既存テストファイル
(`tests/*.rs`)・既存報告書(`verification_logs/`配下)への変更は一切ない。

## 45. Critical／High／Medium／Low

* **Critical: 0件**
* **High: 0件**
* **Medium: 0件**
* **Low: 2件(いずれも「新たに発見した不具合」ではなく、既存コードの
  既知・意図的な状態についての設計上の留意事項)**
  1. `military::supply::process_supply`・`military::combat::
     calculate_combat_strength`・`war::combat::process_combat`という
     3つの死んだ経路(§10)が、Phase 26実装者が誤って「補給システムの
     既存下地」と誤認し、活性化させてしまうリスクがある。P25-001監査が
     既に同種のリスクをLowとして記録済み(装備版)であり、Phase 26でも
     同じ注意が必要——特に`Division.supply_ratio`フィールドの転用
     (§23)を行う際、これらの死んだ経路を誤って有効化しないよう、
     P26-005実装時に明示的な回帰テスト(`process_supply`/
     `calculate_combat_strength`/`process_combat`が引き続き未接続で
     あることを固定するテスト、P25-001の
     `test_u_dead_war_combat_process_combat_path_remains_unconnected`
     と同型)を追加することを推奨する。
  2. `Food`が現在「消費」を実装していない(§7・§13)ことは、Food自体の
     設計としては意図的(生活水準へのboolean的ボーナスのみ)と見られるが、
     将来Foodの実消費機構を導入する際は、本報告書が却下した案C
     (§13・40)を再評価する余地がある、という設計メモとして記録する。

## 46. 実行したコマンドと正確な結果

すべて逐次実行(並列実行なし)。

```
$ git rev-parse HEAD
150e2aea23cd44819d2e2c997857be987f9951af

$ git status --short
 M saves/savegame_v1.ron

$ rg --files tests | sort
(49ファイル。§46末尾に一覧を記載)
```

個別ライブラリ・E2Eテスト(すべて `0 failed`):

| コマンド | 結果 |
|---|---|
| `cargo test --lib economy::` | 12 passed |
| `cargo test --lib military::` | 283 passed |
| `cargo test --lib state::` | 19 passed |
| `cargo test --lib country::` | 79 passed |
| `cargo test --lib save::` | 300 passed |
| `cargo test --test p23_002_mobilized_manpower_workforce_e2e_test` | 8 passed |
| `cargo test --test p23_004_economic_workforce_ratio_e2e_test` | 7 passed |
| `cargo test --test p23_006_manpower_ledger_e2e_test` | 22 passed |
| `cargo test --test p23_007_division_demobilization_e2e_test` | 24 passed |
| `cargo test --test p23_008_daily_reinforcement_e2e_test` | 21 passed |
| `cargo test --test p23_009_ai_recruitment_e2e_test` | 22 passed |
| `cargo test --test p23_010_phase23_manpower_lifecycle_audit_test` | 9 passed |
| `cargo test --test p24_002_workforce_rounding_unification_e2e_test` | 17 passed |
| `cargo test --test p25_002_equipment_accounting_foundation_e2e_test` | 15 passed |
| `cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test` | 6 passed |
| `cargo test --test p25_003_recruitment_equipment_issue_e2e_test` | 22 passed |
| `cargo test --test p25_004_combat_equipment_loss_e2e_test` | 5 passed |
| `cargo test --test p25_005_daily_equipment_reinforcement_e2e_test` | 10 passed |
| `cargo test --test p25_006_demobilization_equipment_return_e2e_test` | 29 passed |
| `cargo test --test p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test` | 8 passed |
| `cargo test --test p25_007_equipment_combat_effectiveness_e2e_test` | 21 passed |

最終確認(逐次実行):

```
$ cargo test --all-targets
(54個のtest binaryにわたる合計)
test result集計: 1833 passed; 0 failed; 1 ignored

$ git status --short
(9件の既知の非決定的スクリーンショットPNGが M として再生成されたため、
 該当9パスのみを個別に `git checkout --` で復元し、以下のとおり
 savegame_v1.ronのみへ戻したことを確認)
 M saves/savegame_v1.ron

$ cargo clippy --all-targets --all-features -- -D warnings
Finished `dev` profile [optimized + debuginfo] target(s) in 1.73s
(warning 0件、エラーなし)
```

開始時基準(1833 passed / 0 failed / 1 ignored、clippy 0 warnings)と
**完全一致**した。

`rg --files tests | sort`の全49件: `daily_system_integration_test.rs`,
`diplomacy_tests.rs`, `economy_tests.rs`, `land_war_combat_peace_test.rs`,
`p20_009_hardcoded_string_scan_test.rs`,
`p20_009_localization_headless_render_test.rs`,
`p20_009_localization_resource_test.rs`,
`p21_005_army_frontline_e2e_test.rs`, `p21_008_army_offensive_e2e_test.rs`,
`p21_009_fix_001_mine_migration_test.rs`,
`p21_009_magic_crystal_chain_test.rs`, `p21_010_claim_crisis_e2e_test.rs`,
`p21_011_crisis_ultimatum_e2e_test.rs`,
`p21_012_ai_crisis_response_e2e_test.rs`,
`p21_013_crisis_support_e2e_test.rs`, `p21_014_country_power_e2e_test.rs`,
`p21_015_ai_crisis_support_e2e_test.rs`,
`p21_016_multilateral_war_e2e_test.rs`,
`p21_017_ai_claim_crisis_e2e_test.rs`,
`p21_018_ai_claim_generation_e2e_test.rs`,
`p21_019_crisis_war_peace_resolution_e2e_test.rs`,
`p21_020_ownership_transfer_e2e_test.rs`,
`p21_021_country_elimination_e2e_test.rs`,
`p21_save_002e_end_to_end_test.rs`, `p21_save_002e_headless_render_test.rs`,
`p21_save_003_end_to_end_test.rs`, `p22_002_world_conclusion_e2e_test.rs`,
`p22_003_terminal_state_load_e2e_test.rs`,
`p23_001_population_manpower_e2e_test.rs`,
`p23_002_mobilized_manpower_workforce_e2e_test.rs`,
`p23_003_population_manpower_ui_e2e_test.rs`,
`p23_004_economic_workforce_ratio_e2e_test.rs`,
`p23_006_fix_001_initial_manpower_ledger_e2e_test.rs`,
`p23_006_manpower_ledger_e2e_test.rs`,
`p23_007_division_demobilization_e2e_test.rs`,
`p23_008_daily_reinforcement_e2e_test.rs`,
`p23_009_ai_recruitment_e2e_test.rs`,
`p23_010_phase23_manpower_lifecycle_audit_test.rs`,
`p24_002_workforce_rounding_unification_e2e_test.rs`,
`p25_002_equipment_accounting_foundation_e2e_test.rs`,
`p25_002_rev_001_fixed_point_equipment_accounting_e2e_test.rs`,
`p25_003_recruitment_equipment_issue_e2e_test.rs`,
`p25_004_combat_equipment_loss_e2e_test.rs`,
`p25_005_daily_equipment_reinforcement_e2e_test.rs`,
`p25_006_demobilization_equipment_return_e2e_test.rs`,
`p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs`,
`p25_007_equipment_combat_effectiveness_e2e_test.rs`,
`profile_p21_021_elimination_scaling_test.rs`,
`profile_workload_correctness_test.rs`, `research_and_politics_tests.rs`,
`ui_headless_render_test.rs`。

## 47. 監査対象コードの`git status --short`

```
 M saves/savegame_v1.ron
```
(全検証コマンド実行後、9件の非決定的スクリーンショットPNGを個別に復元
した後の最終状態。本報告書コミット前の状態と同一。)

## 48. 最終結論

**DESIGN DECIDED**。監査の結果、Phase 26が実装すべき軍事補給・兵站
システムの全体設計(単一資源`MilitarySupplies`・段階導入の空間モデル・
前日状態基準の時間モデル・3要素min戦闘力合成・Save V1維持)を確定した。
最大の発見は、現行コードに「概念的には正しいが未接続の下地」
(`Division.supply_ratio`・`military::supply::process_supply`・
`military::combat::calculate_combat_strength`)が既に眠っており、これを
復活させるのではなく、Phase 25が確立した固定小数点会計パターンで
置き換える形で再利用する、という設計判断に至った点である。P26-002は
この設計のうち「資源とfixed-point会計基盤のみ」という最小の原子的範囲に
限定し(§43)、Division需要・日次配分・戦闘力補正・移動補正・経路探索・
携行在庫・Attrition・AI・UIはすべて後続タスク(§42)へ明確に切り出した。
検証の結果、開始時基準(1833 passed / 0 failed / 1 ignored、clippy 0
warnings)は本監査を通じて完全に維持された。
