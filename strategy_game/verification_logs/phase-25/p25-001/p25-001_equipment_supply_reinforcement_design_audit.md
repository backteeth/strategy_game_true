# P25-001: 装備・補給を消費する補充システム設計監査 報告書

## 1. 総合判定

**DESIGN DECIDED**

現行コードには、想定より進んだ(だが未接続の)装備関連の下地が既に存在する。
`ResourceType::MilitaryEquipment`という国別備蓄資源、`Division.equipment`/
`max_equipment`、`DivisionDefinition.required_equipment`は全て実装済みだが、
互いに接続されておらず、`required_equipment`は「装備は金で買う、Simplified」という
コード自身のコメントが認める仮置きの財務コストとして使われているのみである。
本監査では、この既存の下地を最大限再利用する設計を1つ選定し、P25全体を原子的な
タスク列へ分割した。

## 2. 設計確定可否

確定可。複数案を並べるだけで終わらせず、実コードの制約(単一の装備生産源
`MilitaryFactory`、DivisionType3種、既存`CountryStockpile`インフラ)に基づいて
単一の推奨設計(§16)を選定した。

## 3. 開始時HEAD

```
b568779c018ac99290be58a4eb38119995668ad2
```

想定と一致していた(Phase 24最終監査コミット)。

## 4. 監査対象コードHEAD

```
b568779c018ac99290be58a4eb38119995668ad2
```

本タスクは設計監査(報告書のみ追加)のため、監査対象コード自体はこのHEADのまま
変化しない。

## 5. Phase 24基準状態

`cargo test --all-targets`: 1602 passed / 0 failed / 1 ignored、
`cargo clippy --all-targets --all-features -- -D warnings`: 0 warnings を本タスク
末尾(§21)で再確認し、開始時基準と完全一致することを確認した。

## 6. ユーザーセーブの保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージしていない。
作業開始時・検証コマンド実行後・コミット直前のいずれの`git status --short`でも
` M saves/savegame_v1.ron`のみが残り続けたことを確認した。

## 7. 現行Divisionモデル

`src/military/data.rs`。

```rust
pub struct DivisionDefinition {
    pub id: DivisionDefinitionId,
    pub division_type: DivisionType,   // Infantry, Artillery, Mage の3種
    pub size: DivisionSize,             // Light, Standard, Heavy
    pub required_manpower: u64,
    pub required_equipment: f64,        // ← 現在は「治安コスト(治安=お金)」として使用(§9)
    pub recruitment_days: u32,
    pub attack, defense, breakthrough, organization, morale, supply_usage: f32,
    pub maintenance_cost: f64,
}

pub struct Division {
    pub id: DivisionId,
    pub owner: CountryId,
    pub division_type: DivisionType,
    pub manpower: u64,
    pub max_manpower: u64,
    pub equipment: f64,       // ← 既存フィールド、実戦闘では未使用(§10)
    pub max_equipment: f64,   // ← 既存フィールド、実戦闘では未使用(§10)
    pub organization, max_organization, morale, max_morale, experience: f32,
    pub supply_ratio: f32,
    pub status: DivisionStatus,
    pub def_id: DivisionDefinitionId,
    pub combat_id: Option<BattleId>,   // #[serde(default)]、後年追加フィールドの前例
    // ...
}
```

`MilitaryRegistry { definitions: HashMap<DivisionDefinitionId, DivisionDefinition>,
divisions: HashMap<DivisionId, Division>, next_division_id: usize }`。
`definitions`はSaveに含まれず、起動時に`assets/data/divisions.ron`から常に再構築
される(`MilitaryRegistry::from_saved_parts`のコメントで確認)。Division/Army/Battle
の合体・分割(merge/split)機能は存在しない。デバッグ初期配置は
`app::loader::spawn_debug_divisions`が`STANDARD_INFANTRY_DEFINITION_ID`のみを使用。

## 8. 現行募兵処理

`src/military/recruitment.rs`。`RecruitmentQueueItem { division_id, target_state,
days_remaining, total_days }`(装備予約フィールドなし)。`request_recruitment`:

```rust
if country.available_manpower < def.required_manpower { return Err(...) }
if country.treasury < def.required_equipment {
    return Err("Insufficient equipment/treasury");  // コード自身のコメント:
}                                                     // "Assuming equipment costs
country.available_manpower -= def.required_manpower;  // money for now"
country.mobilized_manpower += def.required_manpower;
country.treasury -= def.required_equipment;  // コメント: "Simplified"
```

`cancel_recruitment`は`treasury += def.required_equipment`で返金。
`process_recruitment`(日次)は`days_remaining`が0になったキュー項目から
`Division { equipment: def.required_equipment, max_equipment:
def.required_equipment, ... }`を生成——**新規Divisionは常に完全装備で誕生し、
国別装備備蓄`CountryStockpile[MilitaryEquipment]`は一切参照・消費されない**。
AI募兵(`military::ai::evaluate_ai_monthly_recruitment`)も同じ`request_recruitment`
を経由するため同じ制約を受ける。UI(`ui::military_panel.rs:1003`)は
`def.required_equipment`を`"req_treasury"`という引数名で表示しており、UI自身も
これを財務コストとして扱っていることを裏付ける。

## 9. 現行日次補充処理

`src/military/reinforcement.rs`(P23-008で確立)。`apply_daily_reinforcement`は
`calculate_daily_reinforcement_cap`(上限)・`calculate_reinforcement_request`・
`apportion_reinforcements`(最大剰余法、`u128`中間値、`DivisionId`昇順tie-break)で
`manpower`のみを回復する。既存の恒久回帰テスト
`reinforcement_never_touches_treasury_equipment_organization_or_experience`が、
**補充がequipment・treasury・organization・experienceのいずれにも触れないことを
意図的な仕様として明示的に固定している**。日次、戦闘より前(その日の損耗は翌日以降に
補充)。

## 10. 現行戦闘損失処理

実際に稼働している戦闘は`military::update::process_daily_battles`
(`BattleRegistry`ベース、`military::combat_calc::resolve_combat_day_multi`使用、
BattleId昇順)であり、その内部処理・`combat_calc.rs`のいずれにも`equipment`への
参照は存在しない(テストフィクスチャのみ)。**equipmentは実戦闘で一切減少しない。**

一方、`military::combat::calculate_combat_strength`(`equip_ratio =
division.equipment / division.max_equipment`をmanpower_ratioとのminで使用)と
`war::combat::process_combat`(`equipment -= damage * 0.1`)という**別の、より古い
1対1簡易戦闘実装**が存在するが、これは`war::tests`からのみ呼び出される未登録の
死んだコードであり(`DailySimulationSet`のいずれにも登録されていない)、実ゲームプレイ
では一切実行されない。この2つの戦闘経路の並存(死んだ経路がequipmentを扱い、生きている
経路が扱わない)は、将来equipmentを実装する際に誤って死んだ経路を活性化させると
二重の戦闘解決が走る危険がある——今回はこのコードを削除しないが(無関係な差分を
変更しない方針のため)、Lowの技術負債として記録する(§39)。

## 11. 現行復員処理

`src/military/demobilization.rs`(P23-007)。`demobilize_division`は生存
manpowerを`available_manpower`(A)へ返却し、B(`mobilized_manpower`)を減算、K
(`permanent_military_losses`)は不変。既存の恒久回帰テスト
`demobilization_never_touches_treasury_or_equipment_related_fields`が、**復員も
equipment・treasuryのいずれにも触れないことを意図的な仕様として明示的に固定して
いる**。

## 12. 現行経済・資源・備蓄モデル

`src/economy/resources.rs`。

```rust
pub enum ResourceType {
    Food, Wood, Iron, Coal, MagicCrystal, IndustrialGoods,
    MilitaryEquipment,      // ← 既存、国別備蓄あり
    RawMagicCrystal,
}

pub struct CountryStockpile { pub amounts: HashMap<ResourceType, f64> }
impl CountryStockpile {
    pub fn get(&self, resource) -> f64        // 未登録キーは0.0を返す(安全)
    pub fn add(&mut self, resource, amount)    // 負値へは落ちない(.max(0.0))
    pub fn consume(&mut self, resource, amount) -> bool  // 不足時false、控除なし
}
```

`BuildingType::MilitaryFactory`(`assets/data/buildings.ron`)は
`output_resources: { MilitaryEquipment: 20.0 }`(Iron 30.0・IndustrialGoods 15.0
消費、required_workforce 25,000)を持ち、`economy::production`の標準経路
(`process_country_production`、月次)を通じて**実際に`CountryStockpile
[MilitaryEquipment]`へ生産物が加算されている**(建物種別による特別扱いなし、
Farm/Factory等と同一コードパス)。

**しかし、`MilitaryEquipment`を消費する経路はコードベース全体に1つも存在しない**
(`grep`で`ResourceType::MilitaryEquipment`の全参照を確認、資源定義・研究効果名
(`MilitaryEquipmentOutputMultiplier`、乗数適用先の生産自体は存在するので技術的には
機能する)・建物出力の3箇所のみ)。したがって現状、`MilitaryFactory`を建てた国は
`MilitaryEquipment`を無限に蓄積し続けるだけで、それがゲームプレイのどこにも
影響しない——**装備の「入口」(生産)は実装済みだが「出口」(消費)が存在しない**、
というのが現状の正確な姿である。

州別備蓄は存在しない(`CountryStockpile`のみ、`StateData`には備蓄フィールドなし)。
数量型は`f64`。負数防止は`add`/`consume`の`.max(0.0)`/`if current >= amount`で
担保。国消滅時の備蓄処理・州併合時の備蓄処理は、`country::elimination`・
`state::ownership`のいずれにも`stockpile`への参照がなく、**国消滅時に備蓄は
消滅せず放置され、併合時に州備蓄という概念自体が存在しないため移管の必要もない**
(国単位の備蓄は所有権と無関係にCountryData自体に紐づく)。

## 13. 現行Saveモデル

`src/save/dto.rs`。`SaveGameV1.countries: Vec<CountryData>`(`CountryData`を
DTO変換なしでそのまま保存、`stockpile: CountryStockpile`を含む)、
`SaveGameV1.military: SavedMilitaryRegistry { divisions: HashMap<DivisionId,
Division>, next_division_id }`(`Division`をそのまま保存)。**`Division.equipment`
/`max_equipment`、`CountryData.stockpile[MilitaryEquipment]`は、いずれも
`#[serde(default)]`を伴わない通常の必須フィールドとして、Division構造体が
最初に導入された時点から既にSave V1スキーマに含まれている**——`combat_id`・
`destination`のような後年追加フィールド(`#[serde(default)]`付き)とは異なり、
これらは元から存在するため、旧Save側の欠損フィールド問題は発生しない。
`DivisionDefinition`はSaveに含まれず起動時にRONから再構築されるため、
`required_equipment`の意味を変更してもSave側のmigrationは一切不要である
(値は各Divisionインスタンスが個別に保持しているため)。

## 14. 装備と補給の概念分離

現行コードは既にこの分離を実質的に体現している。`ResourceType`のうち
`Food`・`Wood`・`Iron`・`Coal`・`IndustrialGoods`・`MagicCrystal`・
`RawMagicCrystal`は生産・貿易・消費が継続的に回る経済財(補給物資寄り)であり、
`MilitaryEquipment`だけが唯一「生産されるが誰も消費しない」耐久財候補として
浮いている。本監査でもこの2概念を分離し、**P25全体ではまず耐久装備
(MilitaryEquipmentの消費経路確立)のみを対象とし、日次補給物資(弾薬・燃料・食料の
前線消費)は別フェーズへ完全に分離する**(§18)。

## 15. 候補案比較表

| 評価軸 | 案A(単一汎用) | 案A'(既存MilitaryEquipment再利用) | 案B(カテゴリ別備蓄) | 案C(充足率のみ) | 案E(国家集約のみ) |
|---|---|---|---|---|---|
| 既存経済との整合 | △(新フィールド) | ◎(既存資源そのまま) | △(新Resource分割が必要) | × | △ |
| Division差 | × | △(当面1種のみ、将来拡張可) | ◎ | × | × |
| テンプレート非依存 | ◎ | ◎ | ◎ | ◎ | ◎ |
| 新規募兵 | 可能 | 可能(既存consume再利用) | 可能だが生産源が1つしかない | 弱い | 可能 |
| 戦闘損失 | 追跡可能 | 追跡可能(既存フィールド) | 追跡可能 | 弱い | Division単位で不可 |
| 実装規模(P25-002) | 中(新フィールド+新Resource) | **小(配線のみ)** | 大(Resource分割+新建物差別化) | 小だが情報量不足 | 中 |
| Save影響 | 新フィールド | **なし(既存フィールドのみ)** | 新Resource variant(V1のHashMapは許容するが生産データ変更要) | 新フィールド | なし |

## 16. 採用する推奨設計

**案A': 既存の`ResourceType::MilitaryEquipment`国別備蓄を、Divisionの装備会計の
正規な資源源として採用する。** 新しい`EquipmentKind`列挙体・新しい`ResourceType`
variant・新しいDivisionフィールドは一切追加しない。

理由:

1. `CountryStockpile[MilitaryEquipment]`は既に生産(`MilitaryFactory`)・保存
   (Save V1)まで実装済みで、あとは消費経路を各Systemへ配線するだけで済む。
2. 現在の`DivisionType`は3種類(Infantry/Artillery/Mage)しかなく、それらを
   差別化する装備生産(建物)は`MilitaryFactory`1つしか存在しない。案B
   (`EquipmentKind`別備蓄)を今採用しても、実際に差別化された生産源がなければ
   「すべてのカテゴリが同じ1つの建物からしか供給されない」という見かけ倒しの
   複雑さにしかならない。§7で述べたとおり、タスク指示自体が「既存経済モデルと
   明確に矛盾する場合は無理に案Bを採用しない」ことを認めている。
3. 将来、Mage用の魔法触媒装備やArtillery専用装備を差別化したい場合、
   `ResourceType`へ新variantを追加し新しい生産建物を追加するだけで済む
   (`CountryStockpile`は`HashMap<ResourceType, f64>`であり未登録キーに対して
   `get`が0.0を返す安全な設計のため、後方互換的にvariant追加できる)。
   `Division.equipment`/`max_equipment`はその時点で`HashMap<ResourceType, f64>`
   への拡張が必要になるが、これは将来のP25後半タスクの範囲であり、今この時点で
   前倒しする理由はない。

## 17. 却下案と理由

- **案A(新規汎用フィールド)**: 既にある`ResourceType::MilitaryEquipment`と
  機能的に重複する新フィールドを追加するのは無駄(§16理由1)。
- **案B(カテゴリ別備蓄)**: 現在の生産インフラ(建物1種のみ)がこれを裏付けない
  (§16理由2)。将来必要になった時点で拡張可能な形(§16理由3)を優先する。
- **案C(Division単位の充足率のみ)**: 何が不足しているか追跡できず、国家備蓄から
  何を消費するかも決められない。タスク自身の指摘どおり生産システムとの接続が
  弱く、既存の`Division.equipment`(絶対量)という有用なフィールドを捨てることに
  なるため却下。
- **案D(師団テンプレート単位の詳細装備)**: ユーザー方針(HoI4型テンプレート不採用)
  と明確に矛盾するため検討から除外。
- **案E(国家集約値のみ、Division個別状態なし)**: 既に`Division.equipment`/
  `max_equipment`が個別に存在し、個々のDivisionの損耗差(§10の`equip_ratio`計算式
  も個別Division単位)を表現する設計が既にコードに埋め込まれているため、これを
  捨てて国家集約のみにする理由がない。

## 18. Division必要装備の由来

**候補1: `DivisionDefinition`に装備必要量を持たせる(現状の`required_equipment: f64`
をそのまま、意味だけを財務コストから物理MilitaryEquipment数量へ変更する)を採用。**

- 必要装備は絶対数(`f64`、既存の型のまま)。manpower最大値に比例させる新計算は
  導入しない——既に`required_manpower`と`required_equipment`は`DivisionDefinition`
  上で独立した値として個別設定可能であり、この独立性を維持する。
- `Division.max_equipment`は`required_equipment`のスナップショット(生成時点の
  DivisionDefinitionの値をコピー、現行`process_recruitment`の実装パターンを
  踏襲)。したがって技術更新等で将来`DivisionDefinition.required_equipment`が
  変わっても、既存Divisionの`max_equipment`は変化しない(現在の`max_manpower`と
  同じ扱い)。
- 戦闘可能装備量と予備装備の区別は導入しない(現行の`manpower`/`max_manpower`と
  同様、単一の現在値のみを保持する)。
- 装備カテゴリID(=`ResourceType::MilitaryEquipment`という既存enumのvariant)は
  Rustのenumとして既に安定している。

## 19. 新規募兵時の装備処理

**完全充足時のみ生成(即時atomic予約)を採用**——現行の`available_manpower`/
`treasury`チェックと全く同じパターンをそのまま踏襲する。

```rust
// P25-003で想定する request_recruitment の拡張(本タスクでは実装しない)
if country.available_manpower < def.required_manpower { return Err(...) }
if country.stockpile.get(MilitaryEquipment) < def.required_equipment {
    return Err("Insufficient equipment")
}
country.available_manpower -= def.required_manpower;
country.mobilized_manpower += def.required_manpower;
country.stockpile.consume(MilitaryEquipment, def.required_equipment);  // 既存API再利用
```

新しい予約フィールドを`RecruitmentQueueItem`へ追加する必要はない——manpowerと
同様、キュー投入時に即座に国家備蓄から控除し、キャンセル時に`stockpile.add(...)`
で返却する(既存の`treasury`返金パターンをそのまま踏襲)。

確認事項への回答:
- 募集キュー投入時に装備を予約する(即時控除、上記のとおり)。
- 完成時には新たな控除を行わない(投入時に既に控除済み)。
- キャンセル時に全額返却する。
- 装備不足時はキューへ投入せず`Err`を返す(現行の`available_manpower`不足時と
  同じ挙動)。
- AIも同じ`request_recruitment`を経由するため自動的に同じ制約を受ける。
- 複数キュー間の優先順位: 既存どおり呼び出し順(先着順)、新たな優先順位機構は
  導入しない。
- 国家間分離: `country.stockpile`は`CountryData`のフィールドであるため構造的に
  分離される(既存のmanpower/treasuryと同じ)。
- DivisionDefinition解決失敗: 既存の`process_recruitment`は`definitions.get`が
  `None`の場合そのキュー項目を静かにドロップする(既存の未解決defバグ、
  P23-009で確認済みの既存挙動)——装備会計もこれと同じ経路を通るため新たな
  例外処理は不要。
- target_state消失・所有権変更、country消滅時の扱い: 既存のmanpower予約と
  同じ挙動(現状、target_stateの消失は`process_recruitment`側では検査されない
  既存の設計——本監査で新たに悪化させない)。
- save/load後の予約: `RecruitmentQueueItem`は既にSave V1に含まれており
  (`recruitment_queue: Vec<RecruitmentQueueItem>`)、装備は既に控除済みの状態で
  `country.stockpile`へ反映されているため、追加のSave対応は不要。
- 同一フレーム競合: 既存の`request_recruitment`はSystem内で同期的に処理される
  単一呼び出しであり、Bevyのexclusive system実行モデル上、同時実行による競合は
  発生しない(既存のmanpower制御と同条件)。
- manpowerと装備の控除は既存コードと同じ関数内で順次行われるため原子的
  (両方成功するかどちらも実行されないかの2択、Rustの通常の関数呼び出し順で
  保証される)。

## 20. 戦闘時の装備損失

**manpower casualtyから比例導出する方式を採用**、damaged/repair/captureは
将来へ保留。

```
equipment_loss_division = floor(
    (manpower_loss_division as u128 * division.max_equipment_fixed_point)
    / division.max_manpower
)
```

(`max_manpower`に対する損耗比率を`max_equipment`へ適用する、workforce計算と
同様の固定小数点整数演算を推奨——`f64`の直接乗算はP24-002と同じ誤差増幅リスクを
持つため)。

決定事項:
- manpower casualtyから比例導出する(§10で述べたとおり、実戦闘は既に
  `manpower_loss`を確定させているため、それを唯一の入力として使い、装備損失を
  独立乱数で別途ロールしない——同じ死傷が2つの独立経路で二重に評価される
  P24-001-REV-001型の問題を最初から回避する)。
- Division全滅時の残存装備は0とする(manpower=0と同時にequipment=0、
  端数を残さない)。
- 鹵獲(勝者による装備奪取)は将来へ保留する(§10のP25-007以降)。
- 敗走時の追加損失は当面導入しない(既存の`RETREAT_MANPOWER_LOSS_RATIO`が
  manpowerにのみ適用される現状のパターンを踏襲、装備版は将来検討)。
- 同じcasualtyを複数回装備損失へ変換しないよう、実装は必ず
  `military::combat_calc`(実際に稼働している経路)にのみ接続し、
  `war::combat.rs`の死んだ経路には触れない(§10で述べた技術負債)。
- 装備損失量は`division.equipment.saturating_sub`相当(f64なので`.max(0.0)`)で
  現在保有量を超えないことを保証する。
- 装備損失後の補充要求は§21の共同制約へ自然に反映される
  (`division.equipment < division.max_equipment`が補充対象の条件になる)。
- 決定的な整数丸め: `f64`のまま演算せず、`u128`固定小数点(workforce/manpowerの
  既存パターン踏襲)で切り捨てる。

## 21. 日次補充の共同制約

```
Q_manpower  = 既存のcalculate_reinforcement_request相当(manpower不足分)
Q_equipment = floor(country.stockpile.get(MilitaryEquipment) /
                     equipment_per_manpower_unit(division))
Q_daily     = 既存のcalculate_daily_reinforcement_cap(manpower基準、変更なし)

Q = min(Q_manpower, Q_daily, missing_manpower)   ← manpower補充量(既存のまま変更しない)
Q_eq_actual = min(missing_equipment, available_equipment_stock)  ← equipment補充量(独立)
```

**manpowerとequipmentの補充を独立した2本の経路として扱うことを推奨する**
(単一の`Q = min(...)`へ統合しない)。理由:

- 既存の`apply_daily_reinforcement`のmanpower補充ロジック(P23-008、21件の
  恒久回帰テストで固定済み)は、装備の有無に関わらず現在の数値をそのまま維持
  すべきであり、新しいequipment制約によって既存のmanpower補充数値が変化する
  設計は、Phase 23で確立した回帰テスト資産を不必要に壊す。
- §12で述べる「manpower充足・equipment不足」という状態を実際に成立させるためには、
  manpowerとequipmentが互いをブロックしない独立した制約である必要がある。
- 複数装備カテゴリの不足時の優先順位: 当面`MilitaryEquipment`は単一カテゴリの
  ため該当しない(§16)。将来複数カテゴリ化した場合は、各カテゴリを独立に
  `min`する拡張になる。
- 部分装備の兵士は許可する(`division.equipment < division.max_equipment`のまま
  戦闘可能、§12)。
- 補充可能人数の丸めは`u128`整数演算(§20と同じ方式)。
- 複数Divisionへの配分は既存の`apportion_reinforcements`(最大剰余法、
  `DivisionId`昇順tie-break)をequipment版としても再利用する(新しい配分アルゴリズム
  を発明しない)。
- プレイヤーとAIの優先度: 既存どおり区別しない(現行のmanpower補充も区別しない)。
- 国家備蓄の控除は`apply_daily_reinforcement`内で同期的に行われるため原子的。
- 同日生産物(月次生産のため、月替わり当日のみ)を補充に使えるかは、既存の
  `DailySimulationSet`順序(Economy→...→MilitaryAction)により**使える**
  (Economyが先に走るため、月替わり当日に生産された`MilitaryEquipment`はその日の
  補充で既に利用可能な備蓄として見える)。
- 同日戦闘後には補充しない(既存のmanpower補充と同じく「前日までの損耗のみ対象」、
  §9のSystem順序どおり補充は戦闘より前)。
- 全滅Division・country消滅後の国には補充しない(既存の`apply_reinforcement_for_
  country`のフィルタ条件をそのまま踏襲)。
- HashMap/Bevy query反復順への非依存は、既存の`apportion_reinforcements`の
  `DivisionId`昇順ソートをそのまま再利用することで保証する。

## 22. 復員・全滅時の装備処理

**生存者復員**: 現存装備(`division.equipment`)を`country.stockpile`
(`MilitaryEquipment`)へ`add`で返却する。破壊済み分(`max_equipment -
equipment`)は返却しない(既に失われているため)。返却先はDivision所有国
(`division.owner`、target_stateではない)。Kは不変(既存どおりmanpowerのみに
関わる値)。

**Division全滅**: manpower=0と同時にequipment=0(§20)。したがって全滅時に
返却すべき残存装備は存在しない(既に戦闘処理で0まで減少している前提)。
Division entity削除前の装備会計は、全滅処理自体が`equipment`を0にしてから
`remove_division`を呼ぶため、二重返却の余地がない。同日復員との競合は、
既存の`demobilize_division`が対象Divisionの状態(`DivisionStatus`)を検査して
戦闘参加中を除外する仕組み(P23-007で確立)をそのまま再利用すれば防げる。

## 23. 国家消滅時の装備処理

**国家備蓄は消滅させる(既定動作、現状追加の会計処理は行わない)。** 既に§12で
確認したとおり、`country::elimination`は`stockpile`へ一切触れない——国家データ
自体が`CountryStatus::Eliminated`となり非活動化されるだけで、`CountryData`
インスタンス自体(と`stockpile`)はメモリ上に残り続ける(物理削除しない、
P21-021の既存方針どおり)。したがって備蓄は「消滅」ではなく「非活動国家に
凍結されたまま残存」が正確な既定動作であり、P25でもこれを変更しない
(併合国への装備移管は将来の講和・鹵獲仕様へ保留)。残存Divisionの扱いも
既存の国消滅処理のパターンを踏襲(装備会計固有の新規処理は追加しない)。
save/load時に孤立装備を残さないことは、`CountryData`が`Vec<CountryData>`として
そのまま保存されるため(§13)、孤立の概念自体が発生しない。

## 24. 推奨System順序

現行`DailySimulationSet`(TimeUpdate→Economy→Research→Diplomacy→CountryAi→
WarPreparation→MilitaryAi→FrontlineOrders→**MilitaryAction**→WarResolution→
UiUpdate)、および`handle_daily_military`内の既存順序(1.募兵完了→2.日次補充→
3-4.移動・侵攻→5.戦闘→6.戦闘終了処理→7.クリーンアップ→8.組織回復)を**変更せず
そのまま維持する**ことを推奨する。

```
生産(Economy、月次、既存)
→ 備蓄追加(既存、MilitaryFactoryが自動的に行う)
→ 募兵完了・装備支給(既存の1番目のステップへ装備控除を追加するだけ、§19)
→ 日次補充(既存の2番目のステップへ装備補充を追加するだけ、§21)
→ 戦闘・装備損失(既存の5番目のステップへ装備損失を追加するだけ、§20)
→ 戦闘終了処理・クリーンアップ(既存のまま)
→ UI/統計(既存のまま)
```

確認事項への回答:
- 同日生産装備は即時利用可能(§21で確認済み、Economy→MilitaryActionの順序による)。
- 同日戦死者は補充されない(既存の順序どおり、補充が戦闘より先)。
- 同日損失装備も同様に翌日以降の補充対象(既存のmanpowerと同じ扱い)。
- Division削除(全滅)と補充の順序: 補充(ステップ2)は戦闘(ステップ5)より前のため、
  その日全滅したDivisionは既にそのタイミングでは存在しており補充対象になり得るが、
  既存の`apply_reinforcement_for_country`は`DivisionStatus`等でフィルタする
  実装のため、前日以前に全滅済みのDivisionのみが除外対象になる(既存のmanpower
  補充と全く同じ既知の挙動、新規の問題ではない)。
- 復員と補充の競合: 復員はプレイヤー操作起点の別System(`ui::military_panel`
  ハンドラ)であり、日次補充とは別フレームで発生するため実務上の競合はない。
- save要求との順序: 既存のSave/Loadは`DailySimulationSet`とは独立したMessage
  駆動のため、装備会計の追加によってもこの独立性は変わらない。
- 月次経済と日次補充の関係: §21で述べたとおり意図的に許容する(既存の
  population成長(月次)とavailable manpower消費(日次)の粒度差と同じパターン)。
- pause中・New Game初日・Loaded Game初日: 既存のDayChangedMessage/
  MonthChangedMessage駆動の仕組みをそのまま踏襲するため、新たな考慮は不要。

## 25. 国家備蓄の正規式

```
CountryStockpile[MilitaryEquipment] は既存のまま f64、非負(add/consumeで保証済み)

production: MilitaryFactory building level × 20.0 / month (既存、変更なし)
consumption: recruitment時にrequired_equipment (§19)
           + reinforcement時にmissing_equipment分 (§21)
refund: recruitment cancel時・demobilization時 (§19/§22)
```

真実源は既存の`CountryStockpile::consume`/`add`/`get`のみ(P25では新しい国家
備蓄計算式を発明しない)。

## 26. Division装備会計の正規式

```
division.equipment: f64 (現在保有量、既存フィールドそのまま)
division.max_equipment: f64 (必要量、生成時にDivisionDefinition.required_equipmentを
                              スナップショット、既存フィールドそのまま)

不変条件: 0.0 <= division.equipment <= division.max_equipment
```

## 27. 決定性

- 国家備蓄の消費・返却は`CountryData`(国ごとに独立したフィールド)経由のため、
  他国の備蓄と混入しない(既存のtreasury/available_manpowerと同じ構造的保証)。
- Division間の配分は既存の`apportion_reinforcements`(最大剰余法+DivisionId
  昇順tie-break)をそのまま再利用するため、HashMap反復順に依存しない。
- 同じ装備を複数Divisionへ二重支給しないことは、`consume`が呼び出し時点で
  即座に備蓄を減算する(atomic、同期的)ため構造的に保証される。

## 28. 性能

現在の国家数(6)・想定Division数(実データ規模で数十〜百程度)に対し、
`HashMap<ResourceType, f64>`(8 variantの小さい固定集合)の走査・
`apportion_reinforcements`の`O(Division数 log Division数)`ソートは既存のまま
性能上の懸念はない。将来`EquipmentKind`が多数に増える場合は、`BTreeMap`または
固定順配列への切り替えを検討する余地があるが、現時点(単一variant運用)では
過剰最適化になるため導入しない。

## 29. Save V1／V2判断

**Save V1を維持する。** §13で確認したとおり、`Division.equipment`/
`max_equipment`、`CountryData.stockpile[MilitaryEquipment]`はいずれも既に
Save V1スキーマに含まれている既存の必須フィールドであり、P25-002〜P25-006の
いずれの実装でも新規フィールドを追加する必要がない(挙動——それらのフィールドを
実際に読み書きするSystemの追加——のみが変わる)。新規フィールドが必要になるのは
将来`EquipmentKind`を複数化する場合(§16理由3)のみで、その際も
`Division.equipment: HashMap<ResourceType, f64>`のような形に変えるなら
`#[serde(default)]`付きの新フィールド+旧`f64`フィールドの並存期間を設ける
通常のマイグレーション手法で対応可能であり、それ自体もV1の枠内(構造体
フィールドの追加)で収まる可能性が高い——今回のP25-002〜P25-006の範囲では
Save V2は不要と判断する。

## 30. migration方針

不要(§29)。旧Save(現在のsavegame_v1.ronを含む)は既に`equipment`/
`max_equipment`/`stockpile[MilitaryEquipment]`を保持しているため
(これらは元々必須フィールドとして存在していた)、構文上も意味上もそのまま
読み込める。

## 31. New Game／Loaded Game整合

New Game初期配置(`spawn_debug_divisions`)は`equipment: def.required_equipment`
で完全装備のDivisionを生成する(既存のまま)。Loaded Gameはセーブされた
`equipment`値をそのまま復元する。両者とも同じ`Division`構造体・同じSaveスキーマを
経由するため、P25実装後もPhase 24で確立した「同じ状態なら同じ結果」の原則を
自動的に満たす(新規の整合性検証コードは不要)。

## 32. save/load分岐整合

§31と同じ理由により、既存のSave/Loadパイプライン(DTO変換なし、フィールドの
直接シリアライズ)を経由する限り、装備関連フィールドもPhase 23/24で確立した
save/load分岐比較の原則をそのまま満たす。

## 33. Phase 24会計の保護

本設計はA・B・L・K・X(manpower関連)、population、workforce計算のいずれにも
一切触れない——装備は`CountryStockpile`/`Division.equipment`という完全に別系統の
会計であり、`military::manpower::compute_manpower_ledger`・
`state::data::canonical_total_workforce`のどちらの関数も変更を必要としない。
manpower capのhard cap化・戦死者の復員可能化も本設計には含まれない。

## 34. P25全体の推奨分割

```
P25-002: Division/DivisionDefinitionのrequired_equipmentの意味を「治安コスト」
         から「物理MilitaryEquipment必要量」へ再定義する正規会計基盤。
         military::equipment(新規)モジュールに、Division.equipment/max_equipment
         とCountryStockpile[MilitaryEquipment]を結ぶ純粋関数群と不変条件テストを
         追加する。既存の募兵・補充・戦闘・復員コードは一切呼び出し側を変更しない
         (§35で詳細)。

P25-003: 新規募兵時の装備支給・予約(§19の実装、recruitment.rsの変更)。

P25-004: 戦闘による装備損失(§20の実装、combat_calc.rs/updateへの接続)。

P25-005: manpower＋equipment制約付き日次補充(§21の実装、reinforcement.rsの拡張)。

P25-006: 復員・全滅・国家消滅時の装備会計(§22/§23の実装)。

P25-007: 装備充足率の戦闘力反映(現状combat_calc.rsが装備を一切見ない、§10で
         確認したギャップの解消)。

P25-008以降: AI生産・補充優先度、日次補給物資・兵站(§14で述べた別概念)、
             UI・Save最終監査は、既存のP23/P24の分割パターンに準じて後続タスクへ
             回す。
```

## 35. P25-002の最小実装範囲

**タスク名**: 装備会計基盤(`military::equipment`モジュール新設)。

**目的**: `DivisionDefinition.required_equipment`・`Division.equipment`/
`max_equipment`・`CountryStockpile[MilitaryEquipment]`という既存フィールドの
意味を正式に再定義し、それらを結ぶ正規計算(不変条件)を1か所へ集約する。
募兵・補充・戦闘・復員の実際の消費・生成・返却ロジックはP25-003以降で実装し、
P25-002では**呼び出し側コードを一切変更しない**。

**変更予定ファイル**: `src/military/equipment.rs`(新規)、`src/military/mod.rs`
(モジュール登録のみ)。

**新規型・フィールド**: なし(すべて既存フィールドの再利用、§16/§29)。

**正規式**: §25/§26のとおり。

**Save V1／V2**: V1維持、migration不要(§29/§30)。

**validation**: `division.equipment <= division.max_equipment`・
`country.stockpile.get(MilitaryEquipment) >= 0.0`を検証する純粋関数
(既存の`save::validate`パターンに準ずるが、本タスクでは`validate.rs`への統合は
行わず`military::equipment`内の単体関数として留める——統合はP25-003以降で
実際に消費ロジックが入ってから判断する)。

**SystemSet**: 変更なし(P25-002はSystemを追加しない、純粋関数のみ)。

**受け入れ条件**:
- `military::equipment`モジュールが、Division装備充足率(`equipment /
  max_equipment`)・国家備蓄の消費可否判定の純粋関数を提供する。
- 既存の`reinforcement_never_touches_treasury_equipment_organization_or_experience`
  ・`demobilization_never_touches_treasury_or_equipment_related_fields`の
  両テストが無変更のまま合格し続ける(P25-002が誤って早期に消費ロジックへ
  接続していないことの証明)。
- 全既存テストが無変更のまま合格(1602 passed / 0 failed / 1 ignored)。

**必須テスト**: `military::equipment`の新規純粋関数に対する単体テスト
(充足率計算・不変条件チェックの境界値)。既存の`required_equipment`関連テスト
(recruitment.rs等)は意味変更を伴わないため無修正のまま。

**対象外**: recruitment.rs／reinforcement.rs／combat_calc.rs／
demobilization.rsの変更、`RecruitmentQueueItem`変更、`EquipmentKind`複数化、
Save V2。

**想定コミットメッセージ**: `feat: add equipment accounting foundation module`

## 36. P25-002受け入れ条件

§35に記載のとおり。

## 37. P25-002必須テスト

§35に記載のとおり。

## 38. 対象外を先行実装していないこと

本タスク(P25-001)では本番Rustコード・既存テストコードのいずれも変更していない
(`git status --short`で確認、§41)。一時検証コードは使用していない
(コード読解のみで設計判断に必要な事実確認を行い、実行を伴う一時テストは
不要だった)。

## 39. Critical／High／Medium／Low

- Critical: 0
- High: 0
- Medium: 1 — 実戦闘(`military::combat_calc`)が現在`Division.equipment`を
  一切参照しないため、将来装備損失を実装しても(P25-004)、それが戦闘力へ
  反映されるまでは(P25-007)実質的な影響を持たない。この2段階の分離は意図的な
  設計(§34)だが、途中段階で「装備が減っても戦闘力が変わらない」という
  一時的に不完全な状態が生じることをユーザーへ明示しておく。
- Low: 2 —
  1. `war::combat::process_combat`(死んだ1対1簡易戦闘実装、`equipment`を
     参照する唯一の既存コード)は未登録の死コードであり、将来の実装時に
     誤って有効化すると実戦闘(`process_daily_battles`)と二重に戦闘解決が
     走る危険がある(§10)。削除はP25の範囲外(無関係な差分を変更しない方針)
     だが、次にこのファイルへ触れる機会に削除を検討すべき技術負債として記録。
  2. `ResourceType::MilitaryEquipment`は現在生産されるが誰にも消費されない
     ("死んだ資源"状態)。P25-003実装まではこの状態が継続する。

## 40. 実行した全コマンドと正確な結果

```bash
git rev-parse HEAD
git status --short
git log --oneline -5

# Division/DivisionDefinition/募兵/補充/戦闘/復員/経済/Saveの調査
grep -rn "pub struct Division\b\|pub struct DivisionDefinition\b" src/
grep -n "pub enum ResourceType\|pub struct CountryStockpile\|pub struct StateResourceDeposit" src/economy/resources.rs
grep -rn "MilitaryEquipment" src/
grep -rn "\.equipment\b\|required_equipment\|max_equipment" src/
grep -n "MilitaryEquipment|military_equipment" assets/data/buildings.ron
grep -rn "calculate_combat_strength\|war::combat::process_combat\b\|fn process_combat\b" src/
grep -rn "MilitaryFactory" src/
grep -n "equipment" src/military/demobilization.rs src/military/update.rs
grep -n "struct SaveGameV1\|struct SavedMilitaryRegistry" src/save/dto.rs
grep -n "DailySimulationSet::\|configure_sets(Update" src/app/time.rs
grep -n "handle_daily_military.*in_set|in_set.*MilitaryAction" src/military/mod.rs

# 基準状態の健全性確認
cargo test --lib population::                                       # 46 passed; 0 failed
cargo test --lib military::                                         # 177 passed; 0 failed
cargo test --lib economy::                                          # 6 passed; 0 failed
cargo test --test economy_tests                                     # 22 passed; 0 failed
cargo test --test p23_006_manpower_ledger_e2e_test                  # 22 passed; 0 failed
cargo test --test p23_007_division_demobilization_e2e_test          # 24 passed; 0 failed
cargo test --test p23_008_daily_reinforcement_e2e_test              # 21 passed; 0 failed
cargo test --test p23_009_ai_recruitment_e2e_test                   # 22 passed; 0 failed
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test     # 9 passed; 0 failed
cargo test --test p24_002_workforce_rounding_unification_e2e_test   # 17 passed; 0 failed

cargo test --all-targets           # 1602 passed; 0 failed; 1 ignored
cargo clippy --all-targets --all-features -- -D warnings   # 0 warnings

git status --short   # 既知の非決定的PNG9件が再生成(cargo test実行自体に伴う既知の挙動、
                     # コード変更の有無とは無関係)、個別checkoutで復元
git checkout -- <9 screenshot paths individually>
```

## 41. コミット直前の`git status --short`

```
 M saves/savegame_v1.ron
```

(`saves/savegame_v1.ron`はユーザーの実セーブデータであり、本タスクでは一切読み書き
していない。既存の未コミット差分がそのまま残っているのみで、コミットには含めない。
既知の非決定的PNG9件は`cargo test --all-targets`の実行自体に伴い毎回再生成される
既知の挙動(本タスクのコード変更の有無とは無関係)であり、個別`git checkout --`で
復元した。)
