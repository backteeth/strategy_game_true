# P25-004: 戦闘損耗による軍事装備損失

## 1. 総合判定

**COMPLETE**

## 2. 開始時HEAD

```text
17f54aecc78c747b47c64cb5f2639d71ed604451
```

## 3. 監査対象コードHEAD

本タスクの実装前監査(§3〜§4)は、上記開始時HEADのワークツリー(コミット前の作業状態)に対して実施した。

## 4. 稼働中の戦闘経路

以下が実際にBevy Appへ登録され、`DailySimulationSet::MilitaryAction`で毎日実行される稼働中の経路である。

```text
handle_daily_military (src/military/update.rs:16、src/military/mod.rs:41で登録)
  └─ process_daily_battles (src/military/update.rs:85、private fn)
       └─ resolve_combat_day_multi (src/military/combat_calc.rs:159)
       └─ [P25-004で追加] apply_combat_casualty (src/military/update.rs、private fn)
  └─ resolve_finished_battles (src/military/update.rs:193、private fn)
       └─ resolve_participant_outcome (src/military/update.rs:308)
```

`process_daily_battles`/`resolve_finished_battles`はどちらも`military::update`のprivate関数であり、Bevy Systemとして個別登録されているわけではない。実際に登録されているのは`handle_daily_military`(`src/military/mod.rs:38-46`)のみで、これが日次イベントごとに上記の関数群を同期的に呼び出す。

## 5. 死んだ戦闘経路

`src/war/combat.rs::process_combat`は、P25-001監査で既に「死んだ経路」と確認済みであり、本タスクでも**一切変更していない**(§37でgit diff実測を確認)。

* `src/war/mod.rs:2`で`pub mod combat;`のためコンパイルはされるが、`.add_systems(...)`には一度も登録されていない。
* 呼び出し元は`src/war/tests.rs`内のユニットテストのみ。
* 同ファイル内の`sync_battle_results_to_wars`(戦闘勝敗をWarへ反映する別関数)は`src/war/update.rs:128`から実際に呼ばれている稼働中の関数だが、装備には一切触れないため本タスクと無関係。

`src/military/combat.rs::process_combat()`(引数なし)も空のTODOスタブのままで、未接続・未変更。

## 6. actual casualtyの取得方法

`src/military/update.rs`の`process_daily_battles`内、`resolve_combat_day_multi`が返す`(manpower_loss, org_loss)`を適用する箇所(旧実装は178-180行目付近の2つのループ)を、新設した`apply_combat_casualty`ヘルパー呼び出しに置き換えた。

```rust
let manpower_before = division.manpower;
let manpower_after = manpower_before.saturating_sub(manpower_loss);
let actual_casualty = manpower_before - manpower_after;
```

`manpower_loss`(combat_calcが要求した予定damage)に対し、既存の`saturating_sub`によるclamp後の減少量を`actual_casualty`として使う。予定damageがmanpowerを超えても、`actual_casualty`は必ず`manpower_before`を超えない。

## 7. 装備損失の正規式

`src/military/equipment.rs`に純粋関数`calculate_proportional_equipment_loss(current_equipment, manpower_before, actual_casualty) -> Result<EquipmentLoss, EquipmentAccountingError>`を追加した。

```text
D = floor((E × C + floor(M / 2)) / M)
```

* `E`: `current_equipment`を正規units化した値(`max_equipment`ではない、§10参照)
* `C`: `actual_casualty`
* `M`: `manpower_before`

`actual_casualty == 0`または`equipment_units_before == 0`の場合は、この式に触れず`destroyed_units = 0`を返す(0除算・不要な計算を避ける)。

## 8. half-up丸め

`+ floor(M/2)`を分子へ加算してから整数除算する、決定的なhalf-up(四捨五入)。`floor`のみを使う切り捨て丸めと比較して、端数の過小計上(装備が実際より多く残り続ける系統的バイアス)を抑制する。新しいRNGは一切使用しない。既存の日次戦闘イベント適用単位(1日1回の`process_daily_battles`呼び出し)がそのまま正規のイベント単位であり、端数のcarry(次回への持ち越し)は導入していない。同じ入力・同じ日次イベント列であれば結果は常に一致する(Test R参照)。

## 9. u128中間演算

```rust
let numerator = u128::from(equipment_units_before).checked_mul(u128::from(actual_casualty))...;
let rounded = numerator.checked_add(u128::from(manpower_before / 2))...;
let destroyed_units: u64 = (rounded / u128::from(manpower_before)).try_into()...;
```

全て`checked_*`を使用。装備の正規最大値(`MILITARY_EQUIPMENT_MAX_UNITS = 10^15`)と`casualty: u64`の最大理論値の積は`u128`の範囲(`約3.4×10^38`)に対して十分小さく、実運用でoverflowすることはないが、defense-in-depthとしてcheckedのままにしている。Test Nで正規最大値付近の規模を検証済み。

## 10. current equipment基準

`calculate_proportional_equipment_loss`は`max_equipment`を引数に一切取らない。装備損失は必ず`damage`適用直前の`current_equipment`を基準にする(§19例B: `max_equipment=100, current=40, casualty=10%` → `destroyed=4, after=36`、`max_equipment`は無関係)。

## 11. casualty=0

`actual_casualty == 0`のとき、`destroyed_units = 0`をそのまま返す(装備・stockpileとも一切触れない)。

## 12. equipment=0

`equipment_units_before == 0`のとき、`actual_casualty`の値に関わらず`destroyed_units = 0`を返す。manpower casualty自体は既存どおり適用される(装備損失計算とmanpower casualtyは別経路)。

## 13. manpower_before=0

`actual_casualty > manpower_before`(0を含む)のとき`InvalidCasualtyInput`エラーを返す。`manpower_before == 0`かつ`actual_casualty == 0`の場合のみ安全なno-opとして許容し、0除算を起こす経路には到達しない(`actual_casualty > 0`が保証された分岐でのみ`M`による除算を実行するようコードで保証)。

## 14. overkill damage

`process_daily_battles`側の既存`saturating_sub`により、予定damageがmanpowerを超えても`manpower_after = 0`、`actual_casualty = manpower_before`に自動的にclampされる。Test Iで、攻撃力を極端に大きくして1日で予定damageが守備側manpower(50)を大幅に超えるシナリオを検証し、`actual_casualty = 50`(= manpower_before)、装備が全損(`destroyed = equipment_before`)することを確認した。

## 15. Division全滅

`actual_casualty == manpower_before`のとき、上記の式は追加の特別扱いなしに数学的に必ず`destroyed_units == equipment_units_before`になる(`D = E + floor(floor(M/2)/M) = E`、`floor(M/2) < M`が`M >= 1`で常に成立するため)。Test H(純粋関数)・Test I(実戦闘経路経由)の両方で確認済み。

全滅処理の順序(`process_daily_battles`→`resolve_finished_battles`)は既存どおりで、装備の全損は`process_daily_battles`内で確定してから`resolve_finished_battles`のdespawn判定(`manpower == 0`)に到達する。

## 16. proportional loss API

```rust
pub fn calculate_proportional_equipment_loss(
    current_equipment: f64,
    manpower_before: u64,
    actual_casualty: u64,
) -> Result<EquipmentLoss, EquipmentAccountingError>
```

値をmutateしない純粋関数。`EquipmentLoss { equipment_units_before, actual_casualty, destroyed_units, equipment_units_after, destroyed: f64 }`を返す。

## 17. try_destroy_equipment再利用

損失式を戦闘コードへ複製せず、実際の破壊は既存の`try_destroy_equipment(&mut division.equipment, loss.destroyed)`(P25-002-REV-001で確立済み)をそのまま呼び出す。`try_destroy_equipment`自体には一切変更を加えていない。

## 18. manpower+equipment原子性

新設した`apply_combat_casualty`(`src/military/update.rs`)が、1師団分のcasualty適用を以下の順で原子的に扱う。

```text
1. manpower_before取得
2. actual_casualty決定(saturating_subのclamp後)
3. calculate_proportional_equipment_loss(非破壊で損失計画)
4. try_destroy_equipment(実際の破壊)
5. どちらかが失敗したら、この師団のmanpower・organizationも一切更新せずreturn
6. 両方成功した場合のみ、manpower・organizationをまとめて確定
```

Test Mで、`equipment = f64::NAN`(現在の正常経路では発生しないはずの破損状態)を持つDivisionに対して1日分の戦闘を適用し、装備会計が失敗した場合に**manpower・organization・equipmentのいずれも変化しない**ことを確認した(部分状態が残らない)。

両陣営(攻撃側・防御側)のdamageは、既存どおり各陣営ごとに独立したループでcommitされる(このループ構造自体はP25-004で変更していない)。巨大な汎用transaction frameworkは追加していない。

## 19. L/K整合

`L`(生存軍事人的資源)・`K`(恒久軍事損失)は`military::manpower::compute_manpower_ledger`が`CountryData.mobilized_manpower`と`Division.manpower`から都度導出する値であり、戦闘コードが直接書き込むフィールドは存在しない。したがって`division.manpower`の減少がそのまま`L`の減少・`K`の増加として反映される。

Test Lで、`compute_manpower_ledger`の戦闘前後の差分が、実際に観測した`actual_casualty`と厳密に一致することを確認した(装備量がKへ混入しない)。

## 20. 国家備蓄不変

`process_daily_battles`/`resolve_finished_battles`はどちらも`CountryStockpile`(国家備蓄)を引数に一切取らない。したがって戦闘で破壊された装備が国家備蓄へ戻ることは構造的にあり得ない。Test Kで、戦闘関数へ渡していない`CountryStockpile`のビット列が戦闘前後で完全に不変であることを明示的に確認した。

## 21. 世界装備減少量

装備の破壊は`try_destroy_equipment`が唯一の書き込み経路であり、`equipment_before - equipment_after == destroyed`が同関数内の往復検証で保証される。国家備蓄には触れないため(§20)、世界全体のMilitaryEquipment量の減少は各Divisionでの`destroyed`の合計と一致する。

## 22. attacker/defender

`apply_combat_casualty`は攻撃側・防御側の区別なく同一の関数として、`atk_results`・`def_results`それぞれのループから同じ引数(`division_id`, `manpower_loss`, `org_loss`)で呼び出される。Test Qで、非対称なstats(攻撃10/防御8 vs 攻撃9/防御14)を持つ双方について、`calculate_proportional_equipment_loss`を独立に再適用した結果が実際のDivision状態と一致することを確認した。

## 23. 複数Division

Test Oで、現在装備量が異なる2つの攻撃側Division(100.0と50.0)が同じ防御側と戦闘した際、各々が自身の`current_equipment`のみを基準に独立して損失を計算し、プールされないことを確認した。

## 24. 複数国家

Test Pで、戦闘に参加していない第三国のDivisionのmanpower・equipmentが完全に不変であることを確認した(`Battle`の参加者リストに含まれないDivisionへはそもそも到達しない構造)。

## 25. 決定性

Test Rで、同一の初期条件から独立に2回シナリオを実行し、5日間の結果(manpower・equipment)がbit-for-bitで完全一致することを確認した。

## 26. RNG系列非変更

装備損失計算(`calculate_proportional_equipment_loss`)は乱数を一切使用せず、`resolve_combat_day_multi`が返すmanpower/organization損失の値にも一切フィードバックしない(引数として渡さない・戻り値も読まない)。Test Sで、現在装備量だけが異なる(100.0 vs 0.0)以外は同一の2つのシナリオで、manpower・organizationの結果が完全に一致することを確認した。

## 27. イベント分割と丸め

新しい端数carryフィールドやSaveフィールドは追加していない。1日1回の`process_daily_battles`呼び出しが既存の正規イベント単位であり、half-up丸めにより端数の過小計上を抑制している。Division全滅時(`actual_casualty == manpower_before`)は丸めに関わらず必ず現在装備を全損させることを§15で数学的に証明・テストで確認済み。同じ入力・同じ日次イベント列であれば結果は必ず一致する(§25)。

## 28. System順序

`DailySimulationSet`のチェーン順序は一切変更していない。`handle_daily_military`は既存どおり`DailySimulationSet::MilitaryAction`のまま。P25-004は既存の`process_daily_battles`内部の処理内容(2つの適用ループ)を`apply_combat_casualty`ヘルパー呼び出しへ置き換えただけで、呼び出し順序・タイミングは変更していない。

## 29. 戦闘力への非接続

`attack_power`・`defense_power`・`resolve_combat_day`/`resolve_combat_day_multi`の計算式・`organization`損失・`movement`・`battle width`・戦闘RNG・`targeting`・`retreat`判定・勝敗判定のいずれも変更していない。装備が損耗しても、P25-007までは戦闘力へ一切影響しない(P25-001で既知のMedium追跡事項どおり)。

## 30. Attritionへの非接続

`try_destroy_equipment`・`calculate_proportional_equipment_loss`のいずれも、Attrition関連コードから呼び出されるよう変更していない。疾病・事故による装備損失、Attrition人口連動、AttritionのSystem順序変更は一切行っていない。

## 31. Save V1維持

新規保存フィールドは追加していない。`Division.equipment`・`Division.max_equipment`・`Division.manpower`・国家人的資源会計は既存のままSave V1で保存される。Test X(戦闘後save/load)で、損耗済みの装備値がsave/loadを経ても正しく保持されることを確認した。

## 32. New Game/Loaded Game比較

Test Vで、New Game直後に戦闘を実行した結果と、New Game→save→load後に同じ戦闘を実行した結果(manpower・equipment)が完全に一致することを確認した。

## 33. save/load分岐比較

Test Wで、戦闘開始直後(1日目適用前)にsave/loadを挟んだ場合と、中断せず継続実行した場合とで、2日分の戦闘後の結果(manpower・equipment)が完全に一致することを確認した。

## 34. 更新した既存テスト

**既存テストは1件も変更していない。** `src/military/equipment.rs`・`src/military/update.rs`いずれも、既存のテスト関数には一切手を加えず、新規テストを追加しただけである(装備を持つDivisionが登場する既存の内部テスト`test_solo_attacker_cannot_break_well_defended_state`等は、manpower/organization/勝敗のみを検証しており、装備値そのものは検証していなかったため、影響を受けずそのまま合格した)。`tests/land_war_combat_peace_test.rs`(保護ファイル)・P23/P24/P25-002/P25-003の既存E2Eテストも全て無変更のまま合格した(§40参照)。

## 35. Phase 24会計の保護

`workforce_ratio`関連のfixed-point計算(Phase 24)には一切触れていない。`cargo test --test p24_002_workforce_rounding_unification_e2e_test`が全17件合格することを確認済み(§40)。

## 36. P25-005へ引き継ぐ内容

* 日次装備補充(国家備蓄からのDivisionへの補充)は依然未実装。P25-004で装備が損耗する経路ができたが、同日中に補充で相殺される経路はまだ存在しない(§14で述べたとおり、既存の`DailySimulationSet`順序では仮に補充を追加しても補充は戦闘より前に実行されるため、その日の戦闘損失は翌日以降まで残り続ける)。
* 復員時装備返却・全滅時装備返却・装備回収・修理・鹵獲は未実装のまま。
* 装備充足率の戦闘力反映(P25-007)は依然未接続。

## 37. 対象外を先行実装していないこと

装備補充・国家備蓄からの補充・復員時装備返却・全滅時装備返却・装備回収・修理・鹵獲・Attrition装備損失・装備充足率の戦闘力反映・combat stat変更・AI変更・UI/ローカライズ変更・新しい装備統計Saveフィールド・新しいEquipmentKind/ResourceType・Save V2/migration・population/workforce/manpower cap変更・戦闘casualty式変更・戦闘RNG変更・`war::combat.rs`の活性化・P25-005以降の先行実装は、いずれも行っていない。

**スコープ判断の補足(retreat時追加manpower損失)**: `resolve_participant_outcome`(`src/military/update.rs:308`)には、組織率0での撤退時に追加でmanpowerを失わせる既存の別メカニクス(`RETREAT_MANPOWER_LOSS_RATIO`)が存在する。これは`resolve_combat_day`/`resolve_combat_day_multi`が計算する「intended casualty」パイプラインの一部ではなく、タスク仕様§3〜§4が明示的に指す「戦闘計算が要求したcasualty→actual casualty」の対象外である(仕様の数値例§19もすべて`process_daily_battles`側の適用を指している)。したがって本タスクでは`resolve_participant_outcome`のretreat処理には一切変更を加えていない。この撤退ペナルティへの装備損失接続が必要であれば、別タスクとして扱うべきと考える。

## 38. Critical/High/Medium/Low

**Critical: 0件、High: 0件、Medium: 0件、Low: 0件**

新規に導入した懸念事項はない。§36(P25-005への引き継ぎ)は本タスクのスコープ内で意図的に対象外とした既知の未接続であり、欠陥ではない。

## 39. 実行した全コマンド

```bash
cargo check --lib
cargo test --lib military::equipment::
cargo test --lib military::update::
cargo test --lib military::
cargo test --lib war::
cargo test --test p25_002_equipment_accounting_foundation_e2e_test
cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test
cargo test --test p25_003_recruitment_equipment_issue_e2e_test
cargo test --test p25_004_combat_equipment_loss_e2e_test
cargo test --test land_war_combat_peace_test
cargo test --test p23_006_manpower_ledger_e2e_test
cargo test --test p23_007_division_demobilization_e2e_test
cargo test --test p23_008_daily_reinforcement_e2e_test
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test
cargo test --test p24_002_workforce_rounding_unification_e2e_test
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
git status --short
git checkout -- (非決定的PNG 9枚を個別に)
```

すべて逐次実行し、並列実行は行っていない。

## 40. 全テストのpassed/failed/ignored件数

* 開始時基準: `1680 passed / 0 failed / 1 ignored`
* `cargo test --all-targets`最終結果: **`1709 passed / 0 failed / 1 ignored`**
* 新規追加テスト: **29件**
  * `src/military/equipment.rs`(内部ユニットテスト、`calculate_proportional_equipment_loss`): 13件
  * `src/military/update.rs`(内部テスト、実際の`process_daily_battles`/`resolve_finished_battles`private関数を直接呼び出し): 11件(Test I/J/K/L/M/O/P/Q/R/S/U)
  * `tests/p25_004_combat_equipment_loss_e2e_test.rs`(新規E2Eファイル、実App経由): 5件(Test T/V/W/X/Y)
* 上記いずれのカテゴリの既存テストも0件失敗(全数値は本報告書の各節に記載)。

## 41. clippy警告数

**0件**(`cargo clippy --all-targets --all-features -- -D warnings`)。

## 42. コミット直前のgit status --short

```text
 M saves/savegame_v1.ron
 M src/military/equipment.rs
 M src/military/update.rs
?? tests/p25_004_combat_equipment_loss_e2e_test.rs
```

(このレポート自体もこの後`??`として追加されるが、コミット対象は本レポート冒頭の該当節に記載のファイルのみで、`saves/savegame_v1.ron`は一切ステージしない。)

## 43. 補足: テスト設計の分割方針

`process_daily_battles`/`resolve_finished_battles`はどちらも`military::update`のprivate関数であり、外部の`tests/`ディレクトリからは直接呼び出せない。そのため、以下の3層構成でテストを配置した。

1. `src/military/equipment.rs`の`#[cfg(test)]`: `calculate_proportional_equipment_loss`単体の数式的正確性(§19の数値例・境界値・u128境界)。
2. `src/military/update.rs`の`#[cfg(test)]`: 既存の`make_division`/`setup_war`/`run_one_day`ヘルパーを再利用し、`process_daily_battles`/`resolve_finished_battles`という**実際に稼働している private関数そのもの**を直接呼び出して、原子性・国家分離・多師団独立性・attacker/defender対称性・決定性・RNG非干渉・死んだ経路非接続を検証。
3. `tests/p25_004_combat_equipment_loss_e2e_test.rs`(新規): 実際にBevy Appへ登録された`handle_daily_military`System経由での稼働確認、New Game/Loaded Game比較、save/load分岐比較。

この分割により、いずれのテストも実際の本番コード(モック・再実装ではない)を経由して検証している。
