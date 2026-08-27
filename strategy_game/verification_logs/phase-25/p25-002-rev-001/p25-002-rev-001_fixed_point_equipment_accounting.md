この報告書はP25-002の数値仕様を訂正する。
P25-002の装備会計モジュールと公開API構造は維持する。
P25-002の直接f64演算による保存則は一般には成立しなかった。
P25-002の実装・報告履歴自体は削除または改変しない。

# P25-002-REV-001: 軍事装備会計の固定小数点化 実装・検証報告書

## 1. 総合判定

**COMPLETE**

P25-002の`military::equipment`モジュール・公開API名(`try_issue_equipment`・
`try_return_equipment`・`try_destroy_equipment`・`validate_equipment_state`・
`equipment_shortfall`・`validate_required_equipment`)・`EquipmentAccountingError`・
既存のMilitaryEquipment備蓄・既存のDivision装備フィールド・Save V1はすべて維持した
うえで、内部の会計処理を`f64`直接演算から正規の固定小数点整数units(スケール
1,000,000)へ訂正した。`2^53`反例を実際に再現し、訂正後は曖昧な成功なく明確に
拒否されることを確認した。MilitaryFactory生産経路も同じ正規unitsを共有するよう
接続した(本タスクで唯一新たに本番接続した経路)。

## 2. P25-002の最終的な扱い

**COMPLETE WITH CORRECTION REQUIRED**

構造(モジュール・公開API・エラー型の骨格)は正しかったが、内部で`f64`を直接
加減算していたため、`stockpile = 2^53, equipment = 1, return = 1`のような規模で
装備が消滅し得る欠陥があった。P25-002のコミット(`322b33f`)・報告書は
revert/amend/reset/編集せず、そのまま履歴に残す。

## 3. 訂正後の判定

**P25-002-REV-001でDESIGN CORRECTED、実装完了。**

会計の真実源を、`economy::resources`にある正規の固定小数点整数units
(`military_equipment_value_to_units`/`military_equipment_units_to_value`、
scale=1,000,000)へ切り替えた。保存上の型(`f64`)・Save V1は変更していない。

## 4. 開始時HEAD

```
322b33f798d09c7318f48669a80bf1816f5641e2
```

想定と一致していた(P25-002の実装コミット)。

## 5. 監査対象コードHEAD

```
322b33f798d09c7318f48669a80bf1816f5641e2
```

作業開始時点で他のコミットは存在せず、開始時HEADと同一。

## 6. 直接f64演算の問題

P25-002当初の`try_issue_equipment`/`try_return_equipment`は、`stockpile_after =
stockpile_before - requested`/`+ returned`のように`f64`を直接加減算していた。
`f64`が連続する整数を誤差なく表現できるのは`2^53`未満に限られるため、
両オペランドの規模がこの近傍に達すると、和や差が正確に表現できず、
小さい方の値(1装備等)が丸めで消えることがある。「演算結果が有限か
(`is_finite()`)」のチェックだけでは、この種の精度崩壊(結果は有限だが不正確)を
検出できなかった。

## 7. `2^53`反例

実際に以下を再現し、訂正前の欠陥メカニズムを確認した(`src/military/equipment.rs`
の`two_pow_53_counterexample_is_rejected_not_silently_lossy`、
`tests/p25_002_rev_001_fixed_point_equipment_accounting_e2e_test.rs`の
`test_i_two_pow_53_counterexample_is_atomically_rejected`)。

```
let two_pow_53 = 2f64.powi(53);       // 9007199254740992.0
two_pow_53 + 1.0 == two_pow_53        // true(1が消える、f64の限界)
```

訂正後の`try_return_equipment(stockpile=2^53, equipment=1, return=1)`は、
`2^53`という規模がP25-002-REV-001が定めた正規値域(equipment値換算で最大10億、
§14参照)を大幅に超えるため、`QuantityOutOfRange`として原子的に拒否される
(stockpile・equipmentのいずれも不変)。曖昧な成功(装備が静かに消える)は排除された。

## 8. 小量加算消失の反例

上記と同じメカニズムだが、支給方向(stockpile側)でも同様の消失が起こり得ることを
確認した——`stockpile=2^53, equipment=0, issue=1`のとき、`stockpile_after =
2^53 - 1`は表現できる(`2^53-1`は`2^53`未満で正確に表現可能)が、逆に
`equipment_after = 0 + 1 = 1`は正確だとしても、次の操作でstockpile側が同じ問題を
抱えたまま蓄積していく。固定小数点units化により、この種の「規模依存で消失する」
問題は入口(units変換)で一律に検出されるようになった。

## 9. 実際の装備生産精度

`assets/data/buildings.ron`の`MilitaryFactory`定義を確認した: `output_resources:
{ MilitaryEquipment: 20.0 }`(基礎生産20.0/レベル/月)、`max_level: 10`。
`economy::production::calculate_actual_output`は`base_output * level *
operation_ratio`(`operation_ratio`は`f32`、workforce比率・物流比率・入力充足率の
最小値、bankruptcyペナルティ含む)であり、`operation_ratio`が1.0未満の端数を
持つ場合、実際の生産量(例: `14.6`のような値)は端数を含む。`DivisionDefinition.
required_equipment`の実データ値(`assets/data/divisions.ron`)は`1000.0`・
`2000.0`・`500.0`のいずれも整数だが、生産側は`operation_ratio`由来の端数を
持ち得るため、小数点以下の精度は実際に必要である。`f32`の有効桁数(約7桁)を
踏まえ、6桁精度(scale=1,000,000)で十分と判断した(§12)。

## 10. 到達可能最大備蓄

現行データ(州28、`MilitaryFactory`最大レベル10)での単月最大理論生産量:

```
1州あたり最大: 20.0 × 10(レベル) × 1.0(operation_ratio上限) = 200.0/月
28州全て最大稼働と仮定: 200.0 × 28 = 5,600.0/月
```

## 11. 150年間の理論最大備蓄

```
150年 = 1,800か月
5,600.0/月 × 1,800か月 = 10,080,000(約1,008万)
```

将来の大幅な拡張シナリオ(州数を現行の10倍=280と仮定、他は同一)でも:

```
200.0 × 280 × 1,800 = 100,800,000(約1億)
```

## 12. 採用したscale

```
MILITARY_EQUIPMENT_SCALE = 1,000,000(小数点以下6桁)
```

## 13. scaleの根拠

`f32`の`operation_ratio`(有効桁数約7桁)由来の生産端数を正確に捉えるには
6桁精度で十分であり、これを超える精度が必要な箇所は見つからなかった(§9)。

## 14. 正規最大units

```
MILITARY_EQUIPMENT_MAX_UNITS = 1,000,000,000,000,000(10^15)
equipment値換算で 10^15 / 10^6 = 10^9(10億)
```

### 根拠(値域)

§11の到達可能最大(現行データ約1,008万、10倍州拡張シナリオ約1億)に対し、
10億は前者の約99倍、後者の約10倍の余裕を持つ。

### 根拠(往復変換の安全性)

理論上`f64`は`2^53`(約9.007×10^15)まで整数を誤差なく表現できるが、
`units / SCALE`という除算自体(`SCALE=10^6`は2の冪でない素因数5^6を含む)が
丸め誤差を持つため、`2^53`ちょうどの近傍では往復変換
(`units → f64 → units`)が保証できないことを実測で確認した——具体的には
`2^53 - 1`(9007199254740991)は往復変換に失敗する(`military_equipment_units_
to_value(2^53-1)`を`f64`表現した後、再度`value_to_units`すると`2^53`へ丸まり、
元のunitsと一致しない)ことをテストで捕捉した(この失敗が本タスク実装中に実際に
発生し、当初`2^53-1`を候補としていたscale上限を`10^15`へ修正する直接のきっかけと
なった)。`10^15`については、その前後5,000の全整数、および0から`10^15`までを
大きな素数間隔(999,999,937)で約100万点サンプリングした実測往復変換テストの
いずれでも失敗が0件だったことを確認した(`src/economy/resources.rs`の
`military_equipment_round_trip_for_representative_units`・
`military_equipment_max_units_round_trips_but_one_unit_beyond_is_rejected`、
および実装過程で使用した一時的な確認テスト[コミットには含めず削除済み]で
検証)。

## 15. `f64 → units`式

```rust
pub fn military_equipment_value_to_units(value: f64) -> Result<u64, MilitaryEquipmentUnitsError> {
    if !value.is_finite() { return Err(NonFinite); }
    if value < 0.0 { return Err(Negative); }
    let scaled = value * (MILITARY_EQUIPMENT_SCALE as f64);
    if !scaled.is_finite() { return Err(OutOfRange); }
    let rounded = scaled.round();
    if rounded > MILITARY_EQUIPMENT_MAX_UNITS as f64 { return Err(OutOfRange); }
    let units = rounded as u64;
    // 往復不変条件を確認(§17)、失敗時はRoundTripFailed
    Ok(units)
}
```

(`src/economy/resources.rs`)

## 16. `units → f64`式

```rust
pub fn military_equipment_units_to_value(units: u64) -> f64 {
    (units as f64) / (MILITARY_EQUIPMENT_SCALE as f64)
}
```

## 17. 往復不変条件

`military_equipment_value_to_units`内部で、算出した`units`を
`military_equipment_units_to_value`で`f64`へ戻し、再度scale乗算・四捨五入して
元の`units`と一致するか確認する(`U(V(u)) = u`)。一致しない場合は
`RoundTripFailed`を返す。`try_issue_equipment`/`try_return_equipment`/
`try_destroy_equipment`は、演算後の値を`f64`へ戻した直後にもこの確認を再実行する
(§9)。全テストでこの不変条件違反は発生しなかった(§14の実測サンプリングでも
0件)。

## 18. 量子化規則

保存された`f64`値は、`military_equipment_value_to_units`の`round()`段階でのみ
四捨五入して正規units化する。支給・返却・破壊の演算自体はunits上の`checked_add`/
`checked_sub`(整数演算、四捨五入なし)のみで行う。epsilon加算・
`saturating_add/sub`による隠蔽はいずれも使用していない。`quantization_delta`
関数(`raw_value - canonical_value`)を新設し、量子化差分を診断・テストで
確認できるようにした(格子上に完全一致する値では`0.0`を返すことを
`quantization_delta_is_zero_for_grid_aligned_values`で確認済み)。

## 19. 支給の正規式

```
equipment_units_before = U(equipment)
max_units = U(max_equipment)
requested_units = U(requested)
stockpile_units_before = U(stockpile)

stockpile_units_after = checked_sub(stockpile_units_before, requested_units)
    または InsufficientStockpile
equipment_units_after = checked_add(equipment_units_before, requested_units)
    または ArithmeticOverflow
(equipment_units_after > max_units の場合 DivisionCapacityExceeded)
```

## 20. 返却の正規式

```
equipment_units_before = U(equipment)
returned_units = U(returned)
stockpile_units_before = U(stockpile)

(returned_units > equipment_units_before の場合 ReturnAmountExceeded)
equipment_units_after = equipment_units_before - returned_units
stockpile_units_after = checked_add(stockpile_units_before, returned_units)
    または ArithmeticOverflow
```

## 21. 破壊の正規式

```
equipment_units_before = U(equipment)
destroyed_units = U(destroyed)

(destroyed_units > equipment_units_before の場合 DestroyAmountExceeded)
equipment_units_after = equipment_units_before - destroyed_units
```

国家備蓄には一切触れない(P25-002から変更なし)。

## 22. units上の保存則

支給・返却とも`stockpile_units_before + equipment_units_before ==
stockpile_units_after + equipment_units_after`が整数演算により構造的に保証される
(`debug_assert_eq!`で明示的にも確認)。破壊は
`equipment_units_before - equipment_units_after == destroyed_units`
(世界units減少量が破壊量と正確に一致、stockpileは不変)。いずれも`f64`の
raw合計ではなく正規units上で保証している。

## 23. 失敗時のビット単位原子性

支給・返却・破壊のいずれも「1. f64入力をunitsへ変換 → 2. units上でvalidation
→ 3. units上でchecked演算 → 4. 保存則確認 → 5. 成功後だけf64へ戻す →
6. 往復不変条件を再確認 → 7. すべて成功した場合だけmutate」の順で処理し、
`&mut`引数への書き込みは最後の成功パスにしか存在しない。全エラー経路
(`InsufficientStockpile`・`DivisionCapacityExceeded`・`ReturnAmountExceeded`・
`DestroyAmountExceeded`・`ArithmeticOverflow`・`NonFiniteQuantity`・
`NegativeQuantity`・`QuantityOutOfRange`・`FixedPointRoundTripFailed`)について、
元の`f64`値が変化しないことをテストで確認した(`assert_eq!`による値の直接比較、
`f64`のビットパターンが変わればこの比較は失敗するため、実質的にビット単位の
不変性を確認している)。

## 24. 反復ドリフト検証

`repeated_issue_return_cycles_do_not_drift`(`src/military/equipment.rs`)で、
支給3.7→返却3.7を100,000回繰り返し、最終的な`equipment`が正確に`0.0`、
`stockpile`が初期値と完全一致することを確認した(整数units演算のため、
理論上も毎回正確に相殺されドリフトし得ない。実行時間0.02秒、テスト時間への
影響は無視できる)。

## 25. MilitaryEquipment生産経路

`src/economy/resources.rs`に`CountryStockpile::add_military_equipment_produced`
を新設し、`src/economy/production.rs`の`process_country_production`内、
`MilitaryEquipment`を生産するループ(通常の`output_resources`加算箇所)だけを
この関数へ差し替えた(2行の変更、他の分岐・生産式・workforce効果・入力充足率
判定ロジックは無変更)。この関数は`military::equipment`と全く同じ
`military_equipment_value_to_units`/`military_equipment_units_to_value`を
呼び出すことで計算式の複製を避け、かつ`economy`が`military`へ依存する新しい
循環依存を作らない配置(中立的な下位モジュール`economy::resources`)とした。
実App経由の統合テスト(`test_l_military_factory_production_uses_fixed_point_
accumulation`・`test_l_military_factory_production_accumulates_across_
multiple_ticks`)で、端数を含む生産量(14.6)が正確に反映され、複数ティックの
累積でも浮動小数点誤差が蓄積しないことを確認した。

## 26. 他ResourceTypeへの非影響

`process_country_production`内の変更は`res == ResourceType::MilitaryEquipment`
の条件分岐のみで、他の`ResourceType`(Food・Wood・Iron・Coal・MagicCrystal・
IndustrialGoods・RawMagicCrystal)は従来どおり`CountryStockpile::add`(生の`f64`
加算)を使用する。`test_m_other_resource_types_are_unaffected_by_the_fixed_
point_change`で、Farm(Food生産)とMilitaryFactory(MilitaryEquipment生産)を
同一Stateに配置し、Food側の生産値が変更前と同じ計算式で得られることを確認した。
`add_military_equipment_produced_does_not_affect_other_resources`(単体テスト)
でも、Iron備蓄がMilitaryEquipment加算の影響を受けないことを確認済み。

## 27. Save V1維持

`CountryStockpile`・`Division.equipment`/`max_equipment`・`DivisionDefinition.
required_equipment`の保存フィールド型はいずれも`f64`のまま変更していない。
`SAVE_FORMAT_VERSION_V1`・`SaveGameV1`構造体・serde属性・migration・load
apply・validationのいずれも変更していない(`git status --short`で`src/save/`
配下に変更がないことを確認)。

## 28. 旧Saveの意味的扱い

旧Saveの有限・非負・値域内(equipment値換算で10億未満)の値は、装備会計API
(`military::equipment`のいずれかの関数)へ最初に渡された時点で正規unitsへ
量子化される(値そのものの書き換えを伴う自動migrationは導入していない——
値が既に格子上にある場合は量子化差分が`0.0`になるだけで、実質的に変化しない)。
同じ値を繰り返し変換しても同じunitsになる(純粋関数であるため構造的に保証)。
`test_n_save_load_round_trip_preserves_the_quantized_stockpile`で、格子に
完全一致しない値(`123.456789`)を設定し、save→load往復後も値が変化しない
こと、さらに2回目のsave→load往復でも値が変化しないこと(繰り返しロードしても
安定)を実App経由で確認した。

## 29. New Game／Loaded Game比較

`test_o_new_game_and_loaded_game_produce_identical_stockpile`で、New Game経路
(継続実行)とLoaded Game経路(save→load後)に同じMilitaryEquipment備蓄値
(42.0)を設定し、両者が完全一致することを実App経由で確認した。

## 30. save/load分岐比較

§28のとおり、`test_n_save_load_round_trip_preserves_the_quantized_stockpile`で
save→load往復・2回連続save→load往復のいずれも値が変化しないことを確認した。

## 31. Phase 24会計の保護

`military::manpower`(A・B・L・K・X)・`state::data`(workforce)・`population::`
配下は本タスクで一切変更していない(`git status --short`で確認)。manpower関連
テスト(`p23_006`〜`p23_010`・`p24_002`)は全件、変更前後で完全一致する結果で
合格した(§38)。

## 32. 本番接続範囲

`economy::production`の`MilitaryFactory`生産加算のみを新たに本番接続した
(§25)。募兵・日次補充・戦闘損失・復員・Division全滅・国家消滅・戦闘力・AI・
UIのいずれにも接続していない——`grep -rn "military::equipment::"
src/military/recruitment.rs src/military/reinforcement.rs src/military/update.rs
src/military/combat_calc.rs src/military/demobilization.rs`は0件のままである
ことを再確認した。

## 33. P25-003へ引き継ぐ内容

固定小数点units化された`try_issue_equipment`/`try_return_equipment`/
`try_destroy_equipment`をそのまま募兵(`request_recruitment`)・復員
(`demobilize_division`)・戦闘損失接続に使用できる。`quantization_delta`は
デバッグ・UI表示での差分確認に利用できる。`add_military_equipment_produced`は
今後他の装備生産源が追加された場合も同じ関数を再利用できる。

## 34. 変更したファイル

本番コード:

- `src/economy/resources.rs`(`MILITARY_EQUIPMENT_SCALE`・
  `MILITARY_EQUIPMENT_MAX_UNITS`・`MilitaryEquipmentUnitsError`・
  `military_equipment_value_to_units`・`military_equipment_units_to_value`・
  `CountryStockpile::add_military_equipment_produced`を新設、6件のunit test追加)
- `src/military/equipment.rs`(内部処理をunits経由へ全面訂正、エラー型に4
  バリアント追加、`quantization_delta`新設、既存テストのエラー期待値を新分類へ
  更新、新規テスト3件追加)
- `src/economy/production.rs`(`MilitaryEquipment`生産加算のみ`add_military_
  equipment_produced`へ差し替え、2行の変更)

テスト:

- `tests/p25_002_equipment_accounting_foundation_e2e_test.rs`(Test Lの期待
  エラー値を`ArithmeticOverflow`→`QuantityOutOfRange`へ訂正、他は無変更)
- `tests/p25_002_rev_001_fixed_point_equipment_accounting_e2e_test.rs`(新規、
  6件)

報告書:

- `verification_logs/phase-25/p25-002-rev-001/
  p25-002-rev-001_fixed_point_equipment_accounting.md`(本書)

## 35. 対象外を先行実装していないこと

`git status --short`・追加ファイル一覧を確認し、以下がいずれも実装されていない
ことを確認した。P25-003募兵装備支給・treasury仮コスト置換・
`RecruitmentQueueItem`変更・日次装備補充・戦闘装備損失・復員装備返却接続・
国家消滅装備処理・装備充足率の戦闘力反映・`EquipmentKind`・新しい
`ResourceType`・新しいSaveフィールド・`CountryStockpile`全体の整数型化・
Divisionフィールド型変更・Save V2・migration・日次補給・兵站・Attrition・UI・
AI・population変更・workforce変更・manpower変更。いずれも該当なし。

## 36. Critical／High／Medium／Low

- Critical: 0
- High: 1 — P25-002当初の`f64`直接加減算が、`2^53`付近の規模で装備の消失・
  増殖を引き起こし得る構造的欠陥を持っていた。本タスクで固定小数点units化に
  より解消済み。
- Medium: 0
- Low: 0

## 37. 実行した全コマンド

```bash
git rev-parse HEAD
git status --short

# 数値監査
grep -n "required_equipment" assets/data/divisions.ron
grep -n "fn new()" -A6 src/economy/resources.rs
grep -c "workforce_ratio:" assets/data/states.ron
grep -c "building_type: MilitaryFactory" assets/data/buildings.ron
grep -n "max_level" assets/data/buildings.ron

# scale境界の実測(一時テスト、コミットには含めず削除済み)
# probe_round_trip_safety_boundaries / dense_probe_near_candidate_bound

cargo build --lib
cargo test --lib military::equipment::              # 29 passed; 0 failed
cargo test --lib economy::resources::                # 12 passed; 0 failed
cargo build --lib  # production.rs接続後の再ビルド確認

rustfmt --edition 2024 --check src/economy/production.rs src/economy/resources.rs \
  src/military/equipment.rs tests/p25_002_equipment_accounting_foundation_e2e_test.rs \
  tests/p25_002_rev_001_fixed_point_equipment_accounting_e2e_test.rs
rustfmt --edition 2024 <同じファイル群>  # フォーマット適用後、再checkでOKを確認

cargo test --test p25_002_equipment_accounting_foundation_e2e_test    # 15 passed; 0 failed
cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test  # 6 passed; 0 failed
cargo test --test economy_tests                                       # 22 passed; 0 failed
cargo test --test p23_006_manpower_ledger_e2e_test                    # 22 passed; 0 failed
cargo test --test p23_007_division_demobilization_e2e_test            # 24 passed; 0 failed
cargo test --test p23_008_daily_reinforcement_e2e_test                # 21 passed; 0 failed
cargo test --test p23_009_ai_recruitment_e2e_test                     # 22 passed; 0 failed
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test       # 9 passed; 0 failed
cargo test --test p24_002_workforce_rounding_unification_e2e_test     # 17 passed; 0 failed
cargo test --lib military::                                           # 206 passed; 0 failed
cargo test --lib economy::                                            # 12 passed; 0 failed

cargo test --all-targets           # 1658 passed; 0 failed; 1 ignored
cargo clippy --all-targets --all-features -- -D warnings   # 0 warnings

git status --short   # 既知の非決定的PNG9件を個別checkoutで復元
git checkout -- <9 screenshot paths individually>
```

Cargoコマンドはすべて逐次実行した(前回P25-002で複数の並列`cargo test`実行が
incrementalキャッシュを破損させた反省を踏まえ、本タスクでは同一メッセージ内での
複数cargo呼び出しを一切行わなかった)。内部コンパイラエラーの再発はなく、
`cargo clean`は使用しなかった。

## 38. 全テストのpassed／failed／ignored件数

```
1658 passed / 0 failed / 1 ignored
```

P25-002時点の基準(1643 passed / 0 failed / 1 ignored)に対し、新規15件
(economy::resources 6件 + military::equipment 3件 + 新規e2eファイル6件)を
加えた数(1643+15=1658)と正確に一致する。

## 39. clippy警告数

```
0 warnings
```

(`cargo clippy --all-targets --all-features -- -D warnings`)

## 40. コミット直前の`git status --short`

```
 M saves/savegame_v1.ron
 M src/economy/production.rs
 M src/economy/resources.rs
 M src/military/equipment.rs
 M tests/p25_002_equipment_accounting_foundation_e2e_test.rs
?? tests/p25_002_rev_001_fixed_point_equipment_accounting_e2e_test.rs
```

(`saves/savegame_v1.ron`はユーザーの実セーブデータであり、本タスクでは一切
読み書きしていない。既存の未コミット差分がそのまま残っているのみで、
コミットには含めない。)
