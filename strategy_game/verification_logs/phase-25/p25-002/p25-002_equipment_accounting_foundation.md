# P25-002: 軍事装備会計基盤 実装・検証報告書

## 1. 総合判定

**COMPLETE**

P25-001が確定した設計(新しい`EquipmentKind`・`ResourceType`・Save フィールドを
追加せず、既存の`CountryStockpile[MilitaryEquipment]`・`Division.equipment`/
`max_equipment`をそのまま再利用する)に従い、`military::equipment`モジュールを
新設した。支給・返却・破壊の3操作はいずれも原子的で、失敗時は一切mutateしない。
既存の募兵・補充・戦闘・復員のいずれからも呼び出されておらず、Phase 23/24の
会計・既存テストは全て無変更のまま合格した。

## 2. 開始時HEAD

```
3f46069986031833bfc990f65afe76f4de6910bb
```

想定と一致していた(P25-001設計監査報告書のコミット)。

## 3. 監査対象コードHEAD

```
3f46069986031833bfc990f65afe76f4de6910bb
```

作業開始時点で他のコミットは存在せず、開始時HEADと同一。

## 4. P25-001設計との一致

以下をすべて確認した(§17参照)。

- 新しい`EquipmentKind`は追加していない。
- 新しい`ResourceType`variantは追加していない。
- 装備カテゴリ別Mapは追加していない。
- 新しいDivision保存フィールドは追加していない。
- `RecruitmentQueueItem`の装備予約フィールドは追加していない。
- Save V2・migrationは導入していない。
- 既存の`ResourceType::MilitaryEquipment`・`CountryStockpile`・
  `Division.equipment`/`max_equipment`・`DivisionDefinition.required_equipment`
  のみを使用している。

## 5. 既存装備型・数量型

実装前に再確認した(全て`f64`で統一されている、新しい整数型は導入していない)。

```rust
// src/economy/resources.rs
pub struct CountryStockpile { pub amounts: HashMap<ResourceType, f64> }
impl CountryStockpile {
    pub fn get(&self, resource: ResourceType) -> f64
    pub fn set(&mut self, resource: ResourceType, amount: f64)  // .max(0.0)で負値防止
    pub fn add(&mut self, resource: ResourceType, amount: f64)
    pub fn consume(&mut self, resource: ResourceType, amount: f64) -> bool
}

// src/military/data.rs
pub struct Division { pub equipment: f64, pub max_equipment: f64, ... }
pub struct DivisionDefinition { pub required_equipment: f64, ... }
```

初期Division(`app::loader::spawn_debug_divisions`)・募兵完了時
(`military::recruitment::process_recruitment`)ともに
`equipment: def.required_equipment, max_equipment: def.required_equipment`
(完全装備)で生成されることを再確認した。`SaveGameV1.military: SavedMilitaryRegistry
{ divisions: HashMap<DivisionId, Division>, ... }`が`Division`を直接保存するため
(§13で既に確認済み)、`equipment`/`max_equipment`は元から必須フィールドとして
Save V1に含まれている。New Game/Loaded Gameいずれも、これらのフィールドの
初期化・復元経路に本タスクによる変更はない。`required_equipment`は現在も
`military::recruitment.rs`の`request_recruitment`/`cancel_recruitment`で
treasury(財務)コストとして使われたままであること、`war::combat.rs`が
`DailySimulationSet`のいずれにも登録されず実行されないこと、
`ResourceType::MilitaryEquipment`を消費する処理がコードベース全体に存在しない
ことをそれぞれ`grep`で再確認した。

## 6. 新設したモジュール

```
src/military/equipment.rs
```

`src/military/mod.rs`のアルファベット順(`demobilization`と`invasion`の間)へ
`pub mod equipment;`を追加した(既存のモジュール宣言順の慣例に従った)。

## 7. 公開した正規API

```rust
pub enum EquipmentAccountingError { InvalidEquipmentState, InsufficientStockpile,
    DivisionCapacityExceeded, ReturnAmountExceeded, DestroyAmountExceeded,
    ArithmeticOverflow }
pub struct EquipmentTransferResult { transferred, stockpile_before, stockpile_after,
    equipment_before, equipment_after: f64 }
pub struct EquipmentDestructionResult { destroyed, equipment_before,
    equipment_after: f64 }

pub fn validate_equipment_state(equipment: f64, max_equipment: f64)
    -> Result<(), EquipmentAccountingError>
pub fn equipment_shortfall(equipment: f64, max_equipment: f64)
    -> Result<f64, EquipmentAccountingError>
pub fn validate_required_equipment(required_equipment: f64)
    -> Result<(), EquipmentAccountingError>
pub fn try_issue_equipment(stockpile: &mut CountryStockpile, equipment: &mut f64,
    max_equipment: f64, requested: f64)
    -> Result<EquipmentTransferResult, EquipmentAccountingError>
pub fn try_return_equipment(stockpile: &mut CountryStockpile, equipment: &mut f64,
    returned: f64) -> Result<EquipmentTransferResult, EquipmentAccountingError>
pub fn try_destroy_equipment(equipment: &mut f64, destroyed: f64)
    -> Result<EquipmentDestructionResult, EquipmentAccountingError>
```

## 8. 装備状態validation

`validate_equipment_state(equipment, max_equipment)`は、両値が有限かつ非負、
かつ`equipment <= max_equipment`であることを検証する。`equipment=0, max_equipment=0`
は有効(Test A)。`equipment > max_equipment`はいずれの理由でも
`InvalidEquipmentState`を返す。

## 9. 不足量計算

`equipment_shortfall`は`validate_equipment_state`を内部で呼んでから
`max_equipment - equipment`を返す。無効状態は`Ok(0.0)`のような値へ黙って
丸めず、`Err(InvalidEquipmentState)`として明示的に伝える(Test B)。

## 10. 支給処理

`try_issue_equipment`は、開始状態の検証→要求量の検証→備蓄取得の検証→
`equipment_after`/`stockpile_after`の算出→備蓄不足/上限超過の検証→
(全て成功した場合のみ)両方をmutate、という順で処理する。`requested=0.0`は
成功するno-opとして扱う(Test K)。

## 11. 返却処理

`try_return_equipment`は、返却量がDivisionの現在装備を超えないことを検証してから、
備蓄への加算とDivision装備の減算を同時に(全検証成功後にのみ)行う。
`max_equipment`は引数に取らない(P25-001の概念シグネチャに従った)ため、
返却後の状態が上限を超えるかどうかは検証しない——返却は装備を減らす操作のため、
数学的に`equipment_after <= equipment_before <= max_equipment`が保証され、
上限超過は原理的に発生しない。

## 12. 破壊処理

`try_destroy_equipment`は国家備蓄に一切触れない(引数にも取らない)。破壊量が
現在装備を超えないことのみを検証し、超える場合は`DestroyAmountExceeded`を返す。

## 13. 原子性

支給・返却・破壊のいずれも「1. 現状態を検証 → 2. 要求量を検証 →
3. 変更後の値をローカル変数として算出・検証 → 4. 全て成功した場合にのみ
実際の`&mut`引数をmutate」という順序で実装されている。検証のいずれかに
失敗した時点で即座に`Err`を返し、その時点までに`&mut`引数への書き込みは
一切発生していない(Rustの所有権/借用モデル上、`stockpile.set(...)`・
`*equipment = ...`の呼び出し自体が各関数内で最後の成功パスにしか存在しないため、
コンパイラのdead-storeも含めて構造的に保証される)。Test E・F・H・Jで
「失敗時に両方の値が変化しない」ことを直接確認した。

## 14. エラー型

`EquipmentAccountingError`(`Debug, Clone, Copy, PartialEq`)は、要求された
6種類のバリアント(`InvalidEquipmentState`・`InsufficientStockpile`・
`DivisionCapacityExceeded`・`ReturnAmountExceeded`・`DestroyAmountExceeded`・
`ArithmeticOverflow`)をそのまま実装した。過剰な文字列生成・UI用文言は
含めていない(`military::demobilization::DemobilizationError`と同じ設計方針)。

## 15. checked arithmetic

数量型が既存どおり`f64`であるため、Rust標準の`checked_add`等(整数型専用)は
使用できない。代わりに、演算後の値の`is_finite()`検査をoverflow検出の代替とした
(§7のモジュールdocコメントに理由を明記)。要求量自体も`validate_amount`で
`is_finite() && >= 0.0`を検証しており、NaN・無限大・負の要求量はいずれも
`ArithmeticOverflow`として拒否される。

## 16. 装備保存則

支給: `stockpile_before + equipment_before == stockpile_after + equipment_after`
(Test C/D)。返却も同じ保存則(Test G)。いずれも実際に`assert_eq!`で数値を
検証した。

## 17. 世界装備減少式

破壊: `world_equipment_before - world_equipment_after == destroyed`
(Test I、`stockpile`は不変のためworld合計の減少分は`equipment`の減少分と
正確に一致する)。破壊された装備が備蓄へ戻らないことも同じテストで確認した。

## 18. 国家間分離

`try_issue_equipment`等はいずれも`&mut CountryStockpile`・`&mut f64`を
呼び出し元から明示的に受け取る設計であり、グローバル状態・共有可変状態を
一切持たない。Rustの借用モデル上、ある国家の`CountryStockpile`を操作する
呼び出しが別の国家の`CountryStockpile`インスタンスへ影響を与えることは
構造的に不可能である。Test Mで2つの独立した`CountryStockpile`インスタンスを
用い、片方への操作がもう片方に影響しないことを直接確認した。

## 19. 決定性

全ての関数は純粋(グローバル状態・乱数・時刻を参照しない)であり、同じ入力に
対して常に同じ出力を返す。HashMap反復順・Bevy query順・entity生成順・OS・
実行タイミングのいずれにも依存しない。複数Divisionへの配分(P25-005で
`apportion_reinforcements`と接続予定)は本タスクの範囲外であり実装していない。

## 20. CountryStockpileの再利用

新しい国別装備備蓄フィールドは追加していない。`try_issue_equipment`/
`try_return_equipment`は`stockpile.get(ResourceType::MilitaryEquipment)`/
`stockpile.set(ResourceType::MilitaryEquipment, ...)`のみを使用し、既存の
`CountryStockpile`をそのまま真実源として扱う。

## 21. Division既存フィールドの再利用

`Division.equipment`/`max_equipment`はどちらも呼び出し元が`&mut f64`として
直接渡す設計とした(新しいラッパー型・newtypeは導入していない)。これは
`Division`構造体自体をこのモジュールに依存させないための意図的な選択であり、
将来P25-003以降でSystemがこの関数を呼ぶ際も
`try_issue_equipment(&mut stockpile, &mut division.equipment, division.max_equipment,
requested)`のように既存フィールドへ直接`&mut`を渡すだけで済む。

## 22. DivisionDefinitionとの関係

`validate_required_equipment(required_equipment: f64)`は、
`DivisionDefinition.required_equipment`が将来Division生成時の装備上限として
使われる際に有限・非負であることを検証する純粋関数として追加した。P25-002の
時点では`recruitment.rs`から呼び出していない(§17の対象外リストどおり)。

## 23. required_equipmentの一時的な意味重複

**本タスク完了後も、`DivisionDefinition.required_equipment`は
`military::recruitment.rs`側で依然として財務コスト(treasury減算)として
使われたままである。** 本モジュールが定義する「物理MilitaryEquipment必要量」
という意味とは、この時点でまだ重複・並存している。この重複はP25-003で
`request_recruitment`が`try_issue_equipment`を呼ぶよう置き換えられた時点で
解消される予定であり、P25-002ではtreasury減算処理を意図的に変更していない
(§4/§17)。

## 24. Save V1維持

`SAVE_FORMAT_VERSION_V1`・`SaveGameV1`構造体・`Division`/`CountryData`の
スナップショット・serde属性・migration・load apply・validationのいずれも
変更していない(`git status --short`で`src/save/`配下に変更がないことを確認、
§34)。`military::equipment`の型(`EquipmentAccountingError`等)はいずれも
`Serialize`/`Deserialize`を実装しておらず、保存対象ではない。

## 25. Phase 24会計の保護

`military::equipment`は`military::manpower`(A/B/L/K/X)・
`state::data::canonical_total_workforce`(workforce)のいずれのモジュールも
`use`していない。`src/military/manpower.rs`・`src/state/data.rs`・
`src/population/`配下は本タスクで一切変更していない(`git status --short`で確認)。

## 26. 本番経路へ未接続であること

`grep -rn "military::equipment::" src/military/recruitment.rs
src/military/reinforcement.rs src/military/update.rs src/military/combat_calc.rs
src/military/demobilization.rs`は0件(該当なし)であることを確認した。
既存の`reinforcement_never_touches_treasury_equipment_organization_or_experience`
・`demobilization_never_touches_treasury_or_equipment_related_fields`の両テストは
本タスクで一切変更しておらず、無変更のまま合格した(§32)。

## 27. P25-003へ引き継ぐAPI

`try_issue_equipment`(募兵時の装備支給、Test Dの部分支給パターンは既にP25-005の
部分補充でも再利用可能な形になっている)、`try_return_equipment`(復員時の返却、
P25-006)、`try_destroy_equipment`(戦闘損失、P25-004)、`equipment_shortfall`
(補充要求量の算出、P25-005)、`validate_required_equipment`(募兵前の
DivisionDefinition検証、P25-003)。

## 28. 変更したファイル

本番コード:

- `src/military/equipment.rs`(新規、正規API+26件のunit test)
- `src/military/mod.rs`(`pub mod equipment;`の1行追加のみ)

テスト:

- `tests/p25_002_equipment_accounting_foundation_e2e_test.rs`(新規、15件)

報告書:

- `verification_logs/phase-25/p25-002/p25-002_equipment_accounting_foundation.md`
  (本書)

## 29. 対象外を先行実装していないこと

`git status --short`・追加ファイル一覧を確認し、以下がいずれも実装されていない
ことを確認した。募兵時の装備支給・`RecruitmentQueueItem`変更・treasuryコスト
仮実装の置換・日次補充への装備接続・戦闘装備損失・manpower casualtyからの
装備損失計算・復員時の装備返却接続・全滅時の装備処理・国家消滅時の装備処理・
equipment充足率の戦闘力反映・combat stats変更・`war::combat.rs`の活性化・
AI変更・UI・EquipmentKind・新しいResourceType・新しいDivisionフィールド・
新しいCountryDataフィールド・日次補給・兵站・Attrition・Save V2・migration・
population変更・workforce変更・manpower cap変更。いずれも該当なし。

## 30. Critical／High／Medium／Low

- Critical: 0
- High: 0
- Medium: 0
- Low: 0

新規モジュールはP25-001の設計をそのまま実装したものであり、実装過程で新たな
発見・懸念事項は生じなかった。

## 31. 実行した全コマンド

```bash
git rev-parse HEAD
git status --short

grep -rn "military::equipment::" src/military/recruitment.rs src/military/reinforcement.rs \
  src/military/update.rs src/military/combat_calc.rs src/military/demobilization.rs

cargo test --lib military::equipment::              # 26 passed; 0 failed
cargo test --test p25_002_equipment_accounting_foundation_e2e_test   # 15 passed; 0 failed
rustfmt --edition 2024 --check src/military/equipment.rs \
  tests/p25_002_equipment_accounting_foundation_e2e_test.rs   # 差分あり(mod.rsは
  # cascade回避のため除外)、rustfmtを適用して再検証しOKを確認

cargo test --lib military::                                          # 203 passed; 0 failed
cargo test --test p23_006_manpower_ledger_e2e_test                   # 22 passed; 0 failed
cargo test --test p23_007_division_demobilization_e2e_test           # 24 passed; 0 failed
cargo test --test p23_008_daily_reinforcement_e2e_test               # 21 passed; 0 failed
cargo test --test p23_009_ai_recruitment_e2e_test                    # 22 passed; 0 failed
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test      # 9 passed; 0 failed
cargo test --test p24_002_workforce_rounding_unification_e2e_test    # 17 passed; 0 failed

# 並列cargo test実行によりincrementalキャッシュが破損(rustc ICE)したため、
# cargo clean -p strategy_game → 再現せず cargo clean(全体) → 完全リビルドで解消
cargo clean -p strategy_game
cargo clean
cargo test --all-targets    # 1643 passed; 0 failed; 1 ignored
cargo clippy --all-targets --all-features -- -D warnings   # 0 warnings

git status --short   # 既知の非決定的PNG9件を個別checkoutで復元
git checkout -- <9 screenshot paths individually>
```

## 32. 各テストの正確な結果

| スイート | 件数 | 結果 |
|---|---|---|
| `military::equipment::`(新規unit test) | 26 | 全合格 |
| `p25_002_equipment_accounting_foundation_e2e_test`(新規) | 15 | 全合格 |
| `cargo test --lib military::`(全体) | 203 | 全合格(177既存+26新規) |
| `p23_006_manpower_ledger_e2e_test` | 22 | 全合格(変更前と完全一致) |
| `p23_007_division_demobilization_e2e_test` | 24 | 全合格(変更前と完全一致) |
| `p23_008_daily_reinforcement_e2e_test` | 21 | 全合格(変更前と完全一致) |
| `p23_009_ai_recruitment_e2e_test` | 22 | 全合格(変更前と完全一致) |
| `p23_010_phase23_manpower_lifecycle_audit_test` | 9 | 全合格(変更前と完全一致) |
| `p24_002_workforce_rounding_unification_e2e_test` | 17 | 全合格(変更前と完全一致) |
| `cargo test --all-targets` | 全体 | **1643 passed / 0 failed / 1 ignored** |

## 33. 全テストのpassed／failed／ignored件数

```
1643 passed / 0 failed / 1 ignored
```

P25-001時点の基準(1602 passed / 0 failed / 1 ignored)に対し、新規26+15=41件を
加えた数(1602+41=1643)と正確に一致する。

## 34. clippy警告数

```
0 warnings
```

(`cargo clippy --all-targets --all-features -- -D warnings`)

## 35. コミット直前の`git status --short`

```
 M saves/savegame_v1.ron
 M src/military/mod.rs
?? src/military/equipment.rs
?? tests/p25_002_equipment_accounting_foundation_e2e_test.rs
```

(`saves/savegame_v1.ron`はユーザーの実セーブデータであり、本タスクでは一切
読み書きしていない。既存の未コミット差分がそのまま残っているのみで、
コミットには含めない。)
