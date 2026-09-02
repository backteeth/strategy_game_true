# P26-003-REV-002: 国家生産集計の整数units化と量子化規則の確定

P26-003の国家生産集計に残っていたf64加算を整数units集計へ訂正する。
P26-003-REV-001の「報告書の評価だけが問題で、実装上の問題はない」という結論は、
本タスクで扱う数値集計仕様への不適合について訂正する。

ただし、実際に確認していないデータ破損・非決定性は一切見つかっておらず、
「発生済み」として記載する事象はない——訂正対象は`process_country_production`内の
量子化タイミング(方式)そのものであり、これまでの全E2Eテスト(P26-002/P26-003の
既存94テストを含む)がその瑕疵を検出しなかったのは、既存テストの数値がすべて
sub-unit境界(1/1,000,000未満)を跨がない値だったためである(§22で詳述)。

## 1. 総合判定

**COMPLETE**

`src/economy/production.rs::process_country_production`のMilitarySupplies国家内
集計を、生の`f64`加算(sum-then-quantize、方式B)から、個別生産エントリ単位の
整数units加算(quantize-then-sum、方式A、`MilitaryEquipment`と同じ規律)へ訂正した。
新規テスト56件すべてがpassし、既存の全2003件(1947+56)の回帰テストが無変更のまま
合格、clippy警告0を確認した。本タスクが対象とする「国家生産集計の量子化方式」の
範囲では、新規のCritical/High findingは0件である。

なお、本タスクは国力スコア・生産不能国・全生産取引原子性等の既知の残件(§19)を
一切解消しない——これらは引き続きPhase 26の未解決事項として残る。

## 2. 開始時HEAD・訂正対象コードHEAD

```text
$ git rev-parse HEAD
2acd9a56398af1a7484135110e5704ce1ba5ecb4

$ git status --short
 M saves/savegame_v1.ron

$ git log --oneline --decorate -15
2acd9a5 (HEAD -> main) docs: audit military supplies coproduction design
56e61b6 feat: produce military supplies
973445c fix: add military supplies localization
4e63355 feat: add military supplies accounting foundation
b44619a docs: design military supply and logistics system
150e2ae docs: finalize phase 25 equipment lifecycle audit
0adcfcc feat: apply equipment readiness to combat power
6023bd5 fix: account for equipment on forced division removal
7b20ca4 feat: return surviving equipment on demobilization
26a8629 feat: add daily equipment reinforcement
0c57beb feat: apply combat equipment losses
17f54ae feat: require equipment for division recruitment
9342cdd fix: use fixed-point military equipment accounting
322b33f feat: add military equipment accounting primitives
3f46069 docs: design equipment-gated reinforcement
```

開始直後に独自に確認し、直近の本番コード変更コミット(`56e61b6f...`、P26-003)・
その監査コミット(`2acd9a5...`、P26-003-REV-001)を含め、想定どおりであることを
確認した。実装対象コードHEADも同じ`2acd9a56398af1a7484135110e5704ce1ba5ecb4`
(本タスクの変更はこの上に積む)。ユーザー所有の既存差分は` M saves/savegame_v1.ron`
のみであり、開始時基準(1947 passed / 0 failed / 1 ignored、clippy warning 0)は
`verification_logs/phase-26/p26-003-rev-001/`§28・§31の実測記録と一致することを
確認した(本タスクでも§23で自己再検証した)。

## 3. 旧P26-003／REV-001の訂正箇所

`verification_logs/phase-26/p26-003-rev-001/`§16が特定した非対称性を訂正する:

> **量子化位置の判定**: MilitarySuppliesは方式B(`round(Σ each_output × SCALE)`)を
> 採用している。……MilitaryEquipmentは方式A(`Σ round(each_output × SCALE)`)である。

このREV-001報告書自身は、この非対称性を「報告書の記述精度の問題」(§6「訂正した
もの」の4項目目、実害は量子化最小単位程度で無視できる規模)として扱い、コード修正
までは要求しなかった(§3「現行実装判定: ACCEPTABLE AS IMPLEMENTED」)。本タスクは、
このオーケストレーターからの明示的な指示に基づき、これをコードレベルで実際に
訂正する。訂正するのは`process_country_production`内の量子化タイミング(方式)の
みであり、REV-001が同時に指摘した他の論点(国力スコアへの影響、生産不能国の
偏り、全生産取引原子性の欠如、MAX_UNITSの性質)は本タスクの対象外として残す
(§19)。

## 4. 変更前の集計型と処理順

```rust
let mut military_supplies_total_value: f64 = 0.0;
// ...州・建物ループ内...
military_supplies_total_value += out_amount;  // 生f64の合算
// ...ループ完了後、国家について1回だけ...
stockpile.add_military_supplies_produced(military_supplies_total_value);
```

処理順: (1)各州・各建物のMilitarySupplies出力(`out_amount: f64`)を生のf64加算で
国家合計へ集計する(量子化は一切行わない) (2)全州処理完了後、国家合計の生f64値を
`add_military_supplies_produced`へ1回だけ渡し、その内部で初めて量子化
(`military_supplies_value_to_units`)が行われる(方式B、sum-then-quantize)。

## 5. 変更後の集計型と処理順

```rust
enum SuppliesUnitsAccumulation {
    Accumulating(u128),
    Failed,
}
// ...州・建物ループ内...
military_supplies_units = military_supplies_units.accumulate(out_amount);
// accumulate内部: military_supplies_value_to_units(out_amount)で個別量子化 →
// u128のchecked_addで加算 → いずれか失敗でFailedへ遷移(以後回復しない)
// ...ループ完了後、国家について1回だけ...
match military_supplies_units {
    Failed => { /* warn!、commitしない */ }
    Accumulating(total_units) if total_units > 0 => {
        // verified_military_supplies_value(total_units)でu64範囲・往復検証
        // 成功時のみ stockpile.add_military_supplies_produced(value) を1回呼ぶ
    }
    Accumulating(0) => { /* 意図的にcommitしない(spec §9) */ }
}
```

処理順: (1)各州・各建物のMilitarySupplies出力(`out_amount: f64`)を、その場で
`military_supplies_value_to_units`により個別に正規unitsへ量子化する
(2)量子化に成功したunitsを`u128`のchecked_addで国家合計へ加算する(失敗した瞬間に
`Failed`状態へ遷移し、以後の全エントリを無視する) (3)全州処理完了後、
国家合計が`Failed`でなく、かつ0より大きい場合のみ、`u128`合計を`u64`範囲・
往復変換で検証してから`add_military_supplies_produced`へ1回だけ渡す
(方式A、quantize-then-sum)。合計0の場合、および`Failed`の場合は
`CountryStockpile`を一切変更しない。`add_military_supplies_produced`を国家単位で
1回だけ呼ぶ点(spec §12「国家単位で原子的」)はP26-003から維持している。

## 6. 個別量子化単位

spec §5.2の指示どおり、既存の生産処理が扱う「1州内の1建物生産エントリ」の
MilitarySupplies出力ごとに量子化する。実コードでは、`process_country_production`
の州ループ×建物ループの内側、`def.output_resources`を辿る`for (&res, &base_out)`
ループの1反復(=1建物種別の1回のMilitarySupplies出力計算)がそのまま「1エントリ」
に対応する。`calculate_actual_output(base_output, level, operation_ratio)`が
levelを1回の乗算で織り込んでいるため、buildingレベルを1ずつ分割して量子化する
処理は追加していない(`test_a10_level_multiplier_is_one_entry_not_split_per_level`
で検証)。1建物1種別につき1つの`out_amount`しか計算されないため、重複生産の
可能性はない(建物定義`output_resources: HashMap<ResourceType, f64>`のキーが
`ResourceType`である以上、同一建物・同一資源のエントリが2回現れることは型的に
不可能)。

## 7. 正規式・丸め方式

個別エントリの量子化式は`U_i = Q(out_amount_i)`(`Q` =
`military_supplies_value_to_units`、P26-002で確立済みのround-to-nearest、
finite・非負・値域内・往復可能性を検証)。国家合計は`T = Σ U_i`
(checked_add、`u128`)。丸め方式自体(round-to-nearest、`f64::round()`)は
P26-002から一切変更していない——変更したのは「いつ丸めるか」(個別エントリごと)
であり、「どう丸めるか」ではない。

## 8. 境界値の変更前後

`verification_logs/phase-26/p26-003-rev-001/`§16の例と同型の境界を実測した
(`tests/p26_003_rev_002_supply_production_units_e2e_test.rs`のtest_a8/test_a9/
test_a11で固定)。

| ケース | 個別エントリ | 旧(方式B: `round(Σ)`) | 新(方式A: `Σ round`) |
|---|---|---|---|
| 0.0000006 × 2 | 0.6 units, 0.6 units | round(1.2)=1 unit | round(0.6)+round(0.6)=1+1=**2 units** |
| 0.0000004 × 2 | 0.4 units, 0.4 units | round(0.8)=1 unit | round(0.4)+round(0.4)=0+0=**0 units** |
| 0.0000004 × 3 | 0.4×3 units | round(1.2)=1 unit | round(0.4)×3=0+0+0=**0 units** |

この差は訂正後の正式な量子化仕様である(spec §6)。旧合計へ戻す補正は加えていない。
通常の整数・小数第1〜2位程度の生産量(実データのMilitaryFactory出力
Equipment=20.0/Supplies=100.0を含む、P26-003の既存49テストの全入力値)は
sub-unit境界(1/1,000,000未満の端数)を一切含まないため、これらは新旧どちらの
方式でも同じ結果になる——このため既存49テストは無変更のまま全件合格した
(§22で詳述)。

## 9. checked演算・最大値検証

- 個別エントリ: `military_supplies_value_to_units`内で、finite検証→非負検証→
  scale乗算後のfinite検証→`MILITARY_SUPPLIES_MAX_UNITS`以下検証→往復変換検証、
  の5段階(P26-002から無変更)。
- 国家合計: `total.checked_add(units as u128)`(`u128`、真のwraparoundからは
  実質的に無縁——後述§11参照)。
- 最終commit: `verified_military_supplies_value`が`u128`合計を`u64::try_from`で
  範囲検証してから`military_supplies_units_to_value`でf64化し、
  `military_supplies_value_to_units`で往復再検証(一致しなければ`None`)。
  成功した場合のみ`add_military_supplies_produced`を呼び、その内部で
  現在備蓄との合算・`MILITARY_SUPPLIES_MAX_UNITS`上限検証・往復再検証が
  再度行われる(P26-002の既存規律、無変更)。

## 10. 最終変換の往復保証

`military_supplies_units_to_value`(P26-002、無検査関数)を国家合計`u128→u64`へ
適用した直後に、`military_supplies_value_to_units`で逆変換し、得られたunitsが
元の`u64`合計と一致することを`verified_military_supplies_value`内で検証している
(spec §9「units_to_valueが無検査関数なら、呼び出し前後の保証を明示する」への
対応)。一致しない場合(例: 合計がドメイン上限を超えていた場合)は`None`を返し、
呼び出し元は`add_military_supplies_produced`を呼ばずに`warn!`のみ記録する。

## 11. 不正エントリ後の失敗状態維持

`SuppliesUnitsAccumulation::Failed`は一度遷移すると、`accumulate`の実装上
(`Failed`分岐で即`return SuppliesUnitsAccumulation::Failed`)、以降どれだけ
有効なエントリが続いても回復しない。`test_d4_valid_entries_after_invalid_do_not_
revive_success`で、先頭州が不正(NaN)・後続2州が合計300.0相当の有効な生産を
持つケースでも、国家全体のMilitarySupplies加算が0のまま(失敗状態維持)である
ことを確認した。`test_d1`〜`test_d3`で不正エントリの位置(先頭・中間・末尾)を
変えても同じ結果になることも確認した。

`u128`のchecked_add自体が真にoverflowする(`checked_add`が`None`を返す)経路は、
`test_c7_true_u128_overflow_is_unreachable_at_any_realistic_building_count`で
純粋計算により「1エントリあたり最大`MILITARY_SUPPLIES_MAX_UNITS`(10^15)を
`u128::MAX / 10^15`(約3.4×10^23)件以上積む必要がある」ことを確認した——これは
現実のどの生産経路(全州×全建物種別でも高々数百件)からも到達不能な規模であり、
この分岐は防御的コードとして扱う(実際には通らない経路をテスト済みと主張しない、
spec §14-C)。実際にテストで再現できた失敗経路は、個別エントリの量子化失敗
(NaN・Infinity・負数・値域超過)と、`verified_military_supplies_value`の
ドメイン上限超過(`u128`合計がMAX_UNITSを超える、`test_c2`・`test_c3`)の2種類
である。

## 12. 原子性の保証範囲

本タスクが保証するのは、P26-003から維持している「1国家の1か月分の
MilitarySupplies出力」の原子性——国家全体で成功か失敗かのいずれかであり、
一部の州・建物分だけが反映される部分適用は起きない
(`test_d1`〜`test_d4`、`test_g1`〜`test_g3`)。本タスクの変更は、この境界自体は
変えず、境界内部の集計方式(生f64合計→整数units合計)だけを訂正した。

## 13. 全生産取引原子性が対象外であること

`test_g1_supplies_failure_leaves_inputs_and_equipment_committed_as_before`・
`test_g3_whole_transaction_atomicity_across_all_outputs_remains_unimplemented`
で、MilitarySuppliesのcommitがoverflowで失敗しても、同じ建物が消費した
Iron/IndustrialGoods、同じ建物が生産したMilitaryEquipmentはMilitarySuppliesの
成否と無関係に確定してしまう(既存の残件、`verification_logs/phase-26/
p26-003-rev-001/`§18〜20が特定した状態)ことを実測で再確認した。本タスクは
「MilitarySupplies出力単体の国家単位原子性」だけを保証し、「原材料消費・
MilitaryEquipment出力・MilitarySupplies出力・その他資源出力を含む工場生産取引
全体の原子性」は確立していない——この表現の区別を維持する(spec §10)。

## 14. 世界保存則

```text
ΣMilitarySupplies_after - ΣMilitarySupplies_before = Σaccepted_production_units
```

成功した国: `accepted_production_units = Σ Q(actual_output_i)`(個別量子化の
整数合計)。失敗した国: `accepted_production_units = 0`(拒否された予定生産量は
保存則へ含まれない、`test_c2`・`test_c3`・`test_d1`〜`test_d4`で確認)。
新規の永続統計・Saveフィールドは追加していない。

## 15. 順序不変性・最小検証例

`test_b1`(2州の順序反転)・`test_b2`(5州、4通りの明示的な並べ替え)・
`test_b3`(sub-unit境界エントリを含む3エントリの3通りの並べ替え)で、
同一の個別出力集合について、州の処理順序を変えても最終units合計が完全に
一致することを確認した。整数の`checked_add`は結合則・交換則が厳密に成り立つ
ため、この不変性は方式Aの構造そのものから保証される(生f64加算のような
浮動小数点固有の順序依存リスクが原理的に排除される)。旧実装(方式B)の
順序依存については、`verification_logs/phase-26/p26-003-rev-001/`§17が
「State/Building挿入順序を変えても最終的なMilitarySupplies備蓄は完全に一致する
ことを確認した」と既に報告しており、本タスクでも新たな反例は見つからなかった
(確認した範囲: 上記test_b1〜test_b3の組み合わせ)。

## 16. 5倍比率の訂正

`test_h1`(整数・完全稼働、比率は厳密に5.0)・`test_h2`(分数operation_ratio、
比率5.0との差<1e-3)・`test_h4`(Equipment側の値を変えてもSupplies側の結果が
不変であることを確認、`supplies_units = equipment_units × 5`という派生計算が
存在しないことを実証)を確認した。加えて`test_h3`で、意図的に丸め境界へ近い
基礎出力(equipment=0.00000082、supplies=0.0000041=5×equipment)を選び、
Equipment側が0.82→1 unit、Supplies側が4.10→4 unitsへそれぞれ独立に丸まり、
最終比率が4.0(5.0ではない)になることを確認した——基礎出力比は常に5だが、
最終的な整数units比を厳密に5.0であると誤って保証しない(spec §7)。
実データ(buildings.ron、Equipment=20.0/Supplies=100.0)は整数比のため
この種の乖離は起きないが、任意の建物レベル・operation_ratioの組み合わせに対して
厳密な5.0を要求しないという仕様上の宣言をテストで固定した。

## 17. 他資源・MilitaryEquipment非変更

`src/military/equipment.rs`・`src/economy/resources.rs`内の`MilitaryEquipment`
関連コード(定数・エラー型・変換関数・`add_military_equipment_produced`)は
1文字も変更していない(`git diff`で確認)。`test_j1`(P26-003)・`test_h1`〜`test_h4`
(本タスク)がMilitaryEquipmentの生産式・結果が無変更であることを確認した。
`src/economy/resources.rs`自体も本タスクでは一切変更していない(§21参照、
production.rs内にローカルヘルパーを追加する形で対応した)。Food等の他
ResourceTypeへの非影響は`test_g2`・`test_j2`(P26-003)・real App経由の
`test_f2`(本タスク、Iron消費・Equipment生産が従来どおり継続することを確認)で
確認した。

## 18. Save V1／New Game／Loaded Game／分岐比較

実App経由のテスト(`test_i1`〜`test_i5`)で: (1)複数州生産後のsave→load往復で
canonical unitsが変化しない(`test_i1`) (2)2回連続save/loadでも安定
(`test_i2`) (3)継続プレイとsave→load分岐後が同一値に到達する(`test_i3`)
(4)load後の継続生産で二重加算が起きない(`test_i4`) (5)旧V1相当(MilitarySupplies
キー未登録)のload後も正常に0から生産が始まる(`test_i5`)ことを確認した。
`StateData.buildings`・`CountryStockpile.amounts`のシリアライズ形式は本タスクで
一切変更していないため、構文互換性は自明に成立する(新規Save関連コード変更ゼロ)。
実プレイヤー所有の`saves/savegame_v1.ron`は一切使用していない(§25参照)。

## 19. 国力・生産不能国等の残件

以下は本タスクでは一切解消していない、既存の残件である(`verification_logs/
phase-26/p26-003-rev-001/`が特定したもの、本タスクの完了判定と混同しない):

- **国力スコア・PowerTier逆転**(REV-001§14〜15、Medium): `compute_state_
  aggregates`が`output_resources`の物量合計をそのまま使うため、MilitaryFactory
  併産により経済力スコアが変化する問題。`src/country/power.rs`は本タスクでは
  変更していない。
- **生産不能国・供給網の疎さ**(REV-001§10〜13、Medium): 6か国中5か国が
  最終的に(初日から、または実測で約18〜24か月以内に)MilitarySuppliesを
  持続生産できない問題。本タスクは生産式・供給網ロジックを一切変更していない。
- **AIの建設・財政停止**(REV-001§10): `process_construction_ai`の
  treasury赤字化によるAI建設全般の機能不全。未修正。
- **初期備蓄0**: New Gameは全国家MilitarySupplies=0から開始する
  (P26-002からの既存設計、未解決の設計事項として残る)。
- **全生産取引原子性**(REV-001§18〜20、本タスク§13で再確認): 未修正、
  意図的な残件として維持。
- **MAX_UNITSの条件付き安全性**(REV-001§22、Low): 「全州最大レベル」という
  仮想最悪ケースからの安全側見積もりである性質は変更していない(数値自体は
  維持)。
- **日次需要・配分・戦闘補正**: 未実装(P26-004/P26-005の範囲)。

## 20. 対象内と対象外を分けたfinding一覧

**対象内(本タスクで訂正・検証済み)**:
- Critical: 0件
- High: 0件
- Medium: 0件
- Low: 0件(新規)

本タスクが対象とした「国家生産集計の量子化方式」の範囲では、新規のfindingは
見つからなかった。個別量子化・国家合計・失敗状態維持・国家間分離・順序不変性・
5倍比率・Save V1のいずれについても、実装は正しく動作している。

**対象外(既知の残件、§19の再掲、本タスクでは変更なし)**:
- Medium 2件: 国力Tier逆転、生産不能国の偏り(REV-001由来)
- Low 3件: 全生産取引原子性の欠如、5倍比率プローズの精度(※本タスクで
  §16のとおりテストにより明示的に固定した)、MAX_UNITSの仮想最悪ケース性質
  (REV-001由来)

## 21. 変更ファイル

```text
strategy_game/src/economy/production.rs                                          (既存ファイル、104行追加・22行削除)
strategy_game/tests/p26_003_rev_002_supply_production_units_e2e_test.rs          (新規、56テスト)
strategy_game/verification_logs/phase-26/p26-003-rev-002/p26-003-rev-002_integer_supply_production_accounting.md (本報告書、新規)
```

`src/economy/resources.rs`は変更していない(既存公開API`military_supplies_
value_to_units`/`military_supplies_units_to_value`をそのまま呼び出すだけで
要件を満たせたため、ローカルヘルパー`verified_military_supplies_value`は
`production.rs`内のプライベート関数として実装した)。`assets/data/buildings.ron`・
`src/country/power.rs`・AI/建設/財政関連コード・`src/save/`・`src/military/`・
UI・ローカライズ・Cargo依存関係のいずれも変更していない。

## 22. 既存テスト変更の理由

**既存テストの数値は一切変更していない。** `tests/p26_003_military_supplies_
production_e2e_test.rs`(49テスト)・`tests/p26_002_military_supplies_accounting_
foundation_e2e_test.rs`(45テスト)は無変更のまま全件合格した(§23)。

理由: 既存49テストの入力値(実データのMilitaryFactory出力Equipment=20.0/
Supplies=100.0、または14.6・33.333333・37.25等の小数第1〜6位の値)は、
sub-unit境界(1/1,000,000未満の端数が複数エントリに分散して合計だけが
境界を跨ぐケース)を一切含まない。個別に量子化しても(方式A)、生合計してから
量子化しても(方式B)、これらの値では常に同じ整数unitsに丸まるため、既存の
アサーションはいずれの方式でも成立する。§8の表で示した「差が生じる具体的な
数値例」(0.0000006や0.0000004)は、本タスクの新規テストファイルでのみ導入した。

実App経由のテスト(test_f2/f3/f4等)も、operation_ratioが分数になり得るため
理論上は個別出力がsub-unit境界を跨ぐ可能性があるが、いずれも`> 0.0`・
`>= last`(単調非減少)・`(sup/eq - 5.0).abs() < 0.05`のような許容誤差付きの
アサーションを使っており、方式A/B間のずれ(最大でも数百万分の1程度)は
これらの許容誤差に対して無視できる規模のため、既存App経由テストも無変更で
合格した。

## 23. 実行コマンド・実測件数

すべて逐次実行(並列実行なし)。

```text
$ cargo build --lib
  → 成功、0エラー

$ cargo test --lib economy::
  → ok. 32 passed; 0 failed; 0 ignored

$ cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test
  → ok. 45 passed; 0 failed; 0 ignored

$ cargo test --test p26_003_military_supplies_production_e2e_test
  → ok. 49 passed; 0 failed; 0 ignored

$ cargo test --test p26_003_rev_002_supply_production_units_e2e_test
  → ok. 56 passed; 0 failed; 0 ignored

$ cargo test --test p21_014_country_power_e2e_test
  → ok. 9 passed; 0 failed; 0 ignored

$ cargo test --test p20_009_localization_headless_render_test
  → ok. 4 passed; 0 failed; 0 ignored

$ cargo test --test p20_009_localization_resource_test
  → ok. 8 passed; 0 failed; 0 ignored

$ cargo test --test p23_002_mobilized_manpower_workforce_e2e_test
  → ok. 8 passed; 0 failed; 0 ignored

$ cargo test --test p23_004_economic_workforce_ratio_e2e_test
  → ok. 7 passed; 0 failed; 0 ignored

$ cargo test --test p23_006_manpower_ledger_e2e_test
  → ok. 22 passed; 0 failed; 0 ignored

$ cargo test --test p23_007_division_demobilization_e2e_test
  → ok. 24 passed; 0 failed; 0 ignored

$ cargo test --test p23_008_daily_reinforcement_e2e_test
  → ok. 21 passed; 0 failed; 0 ignored

$ cargo test --test p23_009_ai_recruitment_e2e_test
  → ok. 22 passed; 0 failed; 0 ignored

$ cargo test --test p23_010_phase23_manpower_lifecycle_audit_test
  → ok. 9 passed; 0 failed; 0 ignored

$ cargo test --test p24_002_workforce_rounding_unification_e2e_test
  → ok. 17 passed; 0 failed; 0 ignored

$ cargo test --test p25_002_equipment_accounting_foundation_e2e_test
  → ok. 15 passed; 0 failed; 0 ignored

$ cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test
  → ok. 6 passed; 0 failed; 0 ignored

$ cargo test --test p25_003_recruitment_equipment_issue_e2e_test
  → ok. 22 passed; 0 failed; 0 ignored

$ cargo test --test p25_004_combat_equipment_loss_e2e_test
  → ok. 5 passed; 0 failed; 0 ignored

$ cargo test --test p25_005_daily_equipment_reinforcement_e2e_test
  → ok. 10 passed; 0 failed; 0 ignored

$ cargo test --test p25_006_demobilization_equipment_return_e2e_test
  → ok. 29 passed; 0 failed; 0 ignored

$ cargo test --test p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test
  → ok. 8 passed; 0 failed; 0 ignored

$ cargo test --test p25_007_equipment_combat_effectiveness_e2e_test
  → ok. 21 passed; 0 failed; 0 ignored

$ cargo test --all-targets
  → 全バイナリ合計: 2003 passed; 0 failed; 1 ignored
  → (`grep -oE "[0-9]+ passed; [0-9]+ failed; [0-9]+ ignored"`で全`test result:`行を
    集計、開始時baseline 1947+新規56=2003で完全一致)

$ cargo clippy --all-targets --all-features -- -D warnings
  → 0 warnings
```

## 24. clippy警告数

```text
0
```

`cargo clippy --all-targets --all-features -- -D warnings`は実装後も0 warnings
(実装前baselineと完全一致)。

## 25. ユーザーセーブ保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージ・コミット
していない。開始時・全検証コマンド実行後・本報告書作成時点のいずれの
`git status --short`でも` M saves/savegame_v1.ron`のみが残り続けたことを確認した
(内容は一度も参照していない)。既知の非決定的スクリーンショットPNG(9件)は
`cargo test --all-targets`実行のたびに再生成されたが、そのつど該当パスのみを
個別に`git checkout --`で復元し、`saves/savegame_v1.ron`以外の差分が残らないことを
都度確認した。

## 26. P26-004／P26-005の進行判定

**P26-004: READYのまま変更なし。** 本タスクはMilitarySuppliesの会計API契約
(checked演算、往復検証、原子的commit)・国家単位の単一commit境界を一切変更して
いない——量子化タイミングを訂正しただけであり、P26-004(日次需要・配分)の設計を
妨げる新たな欠陥は見つからなかった。

**P26-005: REV-001の推奨(生産能力の偏りへの対処要否をユーザー判断として確認)を
そのまま維持、本タスクにより自動的にREADYへ変更しない。** §19のとおり、
生産不能国・国力Tier逆転等の残件は本タスクの範囲外であり一切解消していない。
Phase 26全体の未解決問題が0になったわけではない——本タスクの`COMPLETE`判定は
「国家生産集計の量子化方式」という狭い範囲に限定される。

## 27. 次に必要な原子的タスク1件

`verification_logs/phase-26/p26-003-rev-001/`§27が推奨した「MilitarySupplies
生産能力の偏りに関するユーザー判断の取得、および(判断結果に応じた)最小の対処
設計」を、引き続き次の原子的タスクとして推奨する。本タスクは量子化方式のみを
訂正したため、この推奨は変化しない——6か国中5か国が最終的にMilitarySuppliesを
持続生産できないという構造的な問題は、量子化方式の訂正では一切解消されず、
P26-004(日次需要・配分)・P26-005(戦闘力反映)に着手する前にユーザー判断を
仰ぐべき論点として残っている。
