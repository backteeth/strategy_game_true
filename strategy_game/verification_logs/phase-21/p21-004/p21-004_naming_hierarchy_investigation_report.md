# P21-004 事前調査: 「軍(Army)」編成の命名・階層構造

日付: 2026-08-11
方針: 本調査はコード・RONデータ・テスト・固定証拠を一切変更していません(調査のみ)。
実装は行っていません。

前提: `src/military/army_group.rs`(`ArmyGroup`/`ArmyGroupRegistry`)は、以前のセッションで
`P21-003`の一部として既に実装済み(未コミット、`git status`で確認可能)。今回はこの
既存実装を含め、実コードから命名・階層構造の実態を再調査した。

---

## 1. 現状のID体系(実コード確認)

`src/common/mod.rs`に定義されている全ニュータイプID:

```rust
pub struct CountryId(pub usize);
pub struct StateId(pub usize);
pub struct ArmyId(pub usize);          // ← 実体は「1個師団」
pub struct DivisionId(pub usize);      // ← 実体は「師団の定義(テンプレート)」
pub struct WarId(pub usize);
pub struct DiplomaticCrisisId(pub usize);
pub struct ClaimId(pub usize);
pub struct TreatyId(pub usize);
pub struct BattleId(pub usize);
pub struct FrontlineId(pub usize);
pub struct ArmyGroupId(pub usize);     // ← P21-004: 複数師団の永続編成
```

**重要な既存の紛らわしさ(P21-004以前から存在)**: `ArmyId`/`ArmyUnit`
(`src/military/data.rs:74`)は、名前に反して**個別の1師団**を表す型である。
一方`DivisionId`は師団の「定義」(`DivisionDefinition`、例:「標準歩兵」というテンプレート)
のIDであり、個々の師団インスタンスのIDではない。つまり:

- 個々の師団インスタンス → `ArmyId`/`ArmyUnit`(名前は「軍」だが実体は「師団」)
- 師団の種類・定義 → `DivisionId`/`DivisionDefinition`(名前は「師団」で実体もテンプレートとしての「師団」)

この「Army」という名前を個体の師団に使ってしまっている状態は、`MilitaryRegistry`
(`armies: HashMap<ArmyId, ArmyUnit>`)を筆頭に、コードベース全体で**255箇所・18ファイル**
(下記§3)に深く定着している。

---

## 2. `ArmyGroup`(既存実装)の実態

`src/military/army_group.rs`(冒頭コメントで明記):

```rust
/// P21-004: 複数師団の永続的な集合(いわゆる「軍」)を管理するモジュール。
/// 既存の`ArmyId`(1師団)と紛らわしいため、この集合は「ArmyGroup」と呼ぶ
```

- `ArmyGroup { id: ArmyGroupId, owner: CountryId, name: String, member_army_ids: Vec<ArmyId> }`
- `ArmyGroupRegistry { groups: HashMap<ArmyGroupId, ArmyGroup>, army_group_map: HashMap<ArmyId, ArmyGroupId>, .. }`
  — `war::frontline::FrontlineRegistry`と同一の設計パターン(逆引きマップ、
  `sanitize_references`による日次整理)。
- 表示名は`create_group`内で自動生成: `format!("Army {number}")`(国ごとに1,2,3...と採番、
  未ローカライズの決め打ち英語文字列)。

**UI/ローカライズでの露出(`assets/localization/en-US.ron:153`, `ja-JP.ron:152`)**:

```
en-US: ("military_panel.army_group_header", "── Army Groups ──"),
ja-JP: ("military_panel.army_group_header", "── 編成(軍) ──"),
```

英語UIは**すでに「Army Groups」と表示している**。これは今回の懸念そのもの — HoI4用語の
「Army Group(軍集団)」は複数の「Army」を束ねる、さらに上位の階層を指す語であり、
現在の実装が指しているのは「複数師団=軍」の層(HoI4でいう「Army」相当)である。
つまり**命名の問題はすでにプレイヤー向けの英語UI文言にまで漏れ出している**。

### この命名になった経緯(過去の投資調査との整合性確認)

以前の投資調査(`verification_logs/phase-21/p21-003/p21-003_audit_and_p21-004_investigation_report.md`
§11)では、この命名衝突を最重要のNEEDS USER DECISIONとして明記し、
「`ArmyGroup`/`Formation`/`Corps`等、どの呼称にするかは決定が必要」「本レポートでは
**暫定的に**`ArmyGroup`を使用する」と明記していた。しかしユーザーは実装承認時
(「はい、実装をすすめてください」)にこの項目を個別確認せず、実装セッション側が
「(未確認のまま)自分の判断でArmyGroupという名前をデフォルト採用した」という経緯が
記録されている(この判断はセッション側の自己申告する仮決定であり、ユーザーの確定承認
ではなかった)。今回ユーザーが「ArmyGroupという名称を安易に使うな」と再度指摘したのは、
**まさにこの未確定だった暫定名に対する妥当な差し戻し**である。

---

## 3. 名称衝突の実態(定量)

| 型 | 意味 | 参照箇所数 | 参照ファイル数 |
|---|---|---|---|
| `ArmyId`/`ArmyUnit` | **1個師団**(誤解を招く名前) | 255箇所 (`ArmyId`) / 62箇所 (`ArmyUnit`) | 18ファイル |
| `ArmyGroupId`/`ArmyGroup` | 複数師団の永続編成(いわゆる「軍」) | `army_group.rs`内が主(約460行)+`military_panel.rs`のUI数箇所+ローカライズ12キー | 実質2ファイル+ローカライズ2ファイル |

`ArmyId`参照18ファイル: `app/loader.rs`, `common/mod.rs`, `country/country_ai.rs`,
`map/army_render.rs`, `map/army_selection.rs`, `military/army_group.rs`, `military/battle.rs`,
`military/combat_calc.rs`, `military/data.rs`, `military/invasion.rs`, `military/recruitment.rs`,
`military/tests.rs`, `military/update.rs`, `profiling.rs`, `ui/military_panel.rs`,
`war/combat.rs`, `war/frontline.rs`, `war/military_ai.rs`, `war/tests.rs`

Rust型システムにより`ArmyId`と`ArmyGroupId`は異なる型なので**コンパイラレベルでの
取り違えは起きない**(`fn foo(id: ArmyId)`に`ArmyGroupId`を渡すとコンパイルエラーになる)。
問題は人間側の可読性・命名の一貫性であり、型安全性の問題ではない。

---

## 4. `FrontlineRegistry`が現在「Army」として扱っているもの

`src/war/frontline.rs`を実コード確認:

```rust
pub struct FrontlinePlan {
    ...
    pub assigned_army_ids: Vec<ArmyId>,   // ← 個別師団のIDのVec
}

pub struct FrontlineRegistry {
    ...
    pub army_frontline_map: HashMap<ArmyId, FrontlineId>,  // ← 個別師団→前線の逆引き
}

pub fn assign_army(&mut self, ..., army_id: ArmyId, ...) -> ... { .. }
pub fn unassign_army(&mut self, ..., army_id: ArmyId, ...) -> ... { .. }
```

**結論: `FrontlineRegistry`が扱う「Army」は個別師団(`ArmyId`)であり、`ArmyGroup`とは
一切連動していない。** 前線への配置・解除・前線border計算はすべて師団単位で行われる。
「軍(ArmyGroup)をまとめて前線に配属する」という機能は現状存在しない
(`ArmyGroupRegistry`と`FrontlineRegistry`は完全に独立した2つのResourceであり、
互いを参照するフィールドもない)。

## 5. `assigned_army_ids`が指すもの

上記§4で確認済み: **個別師団(`ArmyId`)を指す。`ArmyGroup`(軍)は指していない。**
`ArmyGroup`側の`member_army_ids: Vec<ArmyId>`と型は同じ(`Vec<ArmyId>`)だが、
意味的には別物(前者は「この前線に配属された師団の生リスト」、後者は
「この編成に所属する師団の生リスト」)であり、たまたま同じ`Vec<ArmyId>`という
型を使っているだけで、フィールド名も別(`assigned_army_ids` vs `member_army_ids`)。

---

## 6. セーブ/ロード時のID安定性

コードベース全体を検索した結果、**セーブ/ロード機能(ゲーム状態をファイルへ書き出し/
読み込みする実装)は現状どこにも存在しない**(`load_game_data`は起動時に
`assets/data/*.ron`の初期データを読み込むだけで、プレイ中のセーブ/ロードとは無関係)。
したがって本項目は現時点で壊れているものではなく、**将来セーブ/ロードを実装する際の
設計上の備えがどこまであるか**という観点の調査になる。

- 全ID型は`usize`ベースのニュータイプで、各Registryが`next_id`(または`next_army_id`)という
  単調増加カウンタを内部に持ち、`XxxId(self.next_id); self.next_id += 1;`で払い出す方式
  (`ArmyGroupRegistry`・`MilitaryRegistry`とも同型のパターン)。
- `ArmyGroupRegistry`は`#[derive(..., Serialize, Deserialize)]`済みで、`next_id`
  `next_group_number`を含め**構造体丸ごとをそのままシリアライズすればID体系込みで
  安定的に永続化できる状態**になっている。
- 一方`MilitaryRegistry`(`src/military/data.rs:109`)は`#[derive(Resource, Default, Debug)]`
  のみで、**`Serialize`/`Deserialize`を導出していない**(個々の`ArmyUnit`自体は
  `Serialize`/`Deserialize`導出済みなので、`HashMap<ArmyId, ArmyUnit>`と
  `next_army_id: usize`をシリアライズ対象に含めるのは技術的には容易だが、
  現状は未対応)。

**結論**: ID設計そのもの(newtype+レジストリ内カウンタ)はセーブ/ロードに適合的。
セーブ/ロード実装時に必要な追加作業は「対象Resourceに`Serialize`/`Deserialize`を
足す」程度で、`ArmyGroup`導入がこの設計を壊してはいない。

---

## 7. `Division → Army → ArmyGroup`という将来の階層評価

現状の実装を、ユーザーの想定する3階層(Division→Army→Army Group)にマッピングすると:

| HoI4的な階層 | 現状コードでの実体 | 命名の適合度 |
|---|---|---|
| Division(師団) | `ArmyUnit`/`ArmyId` | **不適合**(名前が「Army」) |
| Army(軍) | `ArmyGroup`/`ArmyGroupId`(既存実装) | **不適合**(名前が将来の軍集団と衝突) |
| Army Group(軍集団、複数のArmyを束ねる) | 未実装 | 名前が既存の`ArmyGroup`に先取りされている |

構造的な拡張性は高い。`ArmyGroupRegistry`は`FrontlineRegistry`と全く同じ設計
(`HashMap<Id, Entity>` + 逆引きmap + `sanitize_references`)の**2例目の実装**であり、
この同型パターンをもう1段複製すれば(例: `HashMap<HigherId, HigherEntity>` +
`HashMap<ArmyGroupId, HigherId>`の逆引きmap)、技術的には低リスクで軍集団層を
追加できる。**問題は技術的な障壁ではなく、名前が先に使われてしまっていること**。

---

## 8. 名称案の比較

ユーザー指示のとおり、(A)既存型の大規模改名 と (B)新規側を別名にする、の2案を比較する。
いずれも「ArmyGroup」は将来の真の軍集団のために温存する前提。

### 案A: `ArmyUnit`/`ArmyId`を大規模改名し、`ArmyGroup`→`Army`に改名する

最終形が最もクリーン(`Division → Army → ArmyGroup`がそのままコードに反映される)。

- 改修対象: 255箇所(`ArmyId`)+62箇所(`ArmyUnit`)、18ファイル。
- 新たな命名上の課題: 個体としての師団を何と呼ぶか(`DivisionId`は既に
  「師団の定義」に使われているため、そのままでは再度衝突する)。
  `DivisionUnitId`/`DivisionInstanceId`等、別途決定が必要。
- 影響範囲が非常に広いため、今回のスコープ(調査のみ)では**実施しない**。
  実施する場合は改名専用のセッション(かつ大量のdiffレビュー)が必要。

### 案B(推奨): `ArmyUnit`/`ArmyId`は現状維持し、既存`ArmyGroup`側を別名へ改名する

- 改修対象: `src/military/army_group.rs`(1ファイル、約460行、未コミット)、
  `src/ui/military_panel.rs`のUI文言・コマンド名数箇所、
  `assets/localization/{ja-JP,en-US}.ron`の`army_group_*`系12キー。
  いずれも今回のセッションで実装されたばかりで**まだ最終確定していない・使用実績も
  ごく浅い**ため、改修コストは案Aと比べて大幅に低い。
- 候補名: `Formation`(編成、軍事的に広く通用する語)/`Corps`(軍団、ただし史実の
  「軍団」は師団より下・軍より上という中間規模の含意があり、HoI4の「Army」と
  厳密には一致しない)/`FieldArmy`(野戦軍、Army本来の語を保ちつつ既存`ArmyUnit`との
  文字面衝突を避けられる)。
- 最終形は`Division役=ArmyUnit(名前は不整合のまま)→Army役=Formation等(別語)→
  ArmyGroup(将来、真の軍集団)`となり、案Aほど理想的な階層表現にはならないが、
  「ArmyGroupを軍集団のために温存する」というユーザーの制約は満たせる。

### 案C(非推奨、参考として記載): 現状のまま`ArmyGroup`を「軍」として使い続ける

ユーザーが明示的に望んでいない案。将来軍集団を追加する際に必ず改名が必要になる
(現状のまま塩漬けにすると、後になるほど`ArmyGroup`の使用箇所が増え、改名コストが
案Bより悪化していく一方になる)。

---

## 9. NEEDS USER DECISION

1. 案A(大規模改名)と案B(新規側のみ改名)のどちらを取るか(本レポートは案Bを推奨)
2. 案Bを選ぶ場合、`ArmyGroup`に代わる名称(`Formation`/`Corps`/`FieldArmy`等)をどれにするか
3. 案Aを選ぶ場合、個体師団の新しい型名(`DivisionId`が既に別の意味で使われているため)
4. 将来的に本当に「軍集団(Army Group)」階層を実装する計画があるか
   (計画がなければ、今`ArmyGroup`という名前を温存する優先度は下がる)
5. 今回改名する場合、実装作業自体は次回以降のセッションでよいか(今回は調査のみとの
   指示のため、いずれにせよ本セッションでは着手しない)

---

## 10. 判定

**READY WITH DECISIONS**

技術的な障壁は一切ない(型システムによりID取り違えは起きない、`ArmyGroupRegistry`は
`FrontlineRegistry`と同型の実証済みパターン、セーブ/ロードへの適合性も設計上問題ない)。
唯一の論点は名称であり、これは技術判断ではなくユーザー自身の設計判断が必要な項目。
上記§9の5項目、特に1・2が決まり次第、次のセッションで(今回は指示どおり)実装に着手できる。
