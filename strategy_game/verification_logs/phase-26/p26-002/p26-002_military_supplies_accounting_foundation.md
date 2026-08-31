# P26-002: MilitarySupplies固定小数点会計基盤 実装報告書

## 1. 総合判定

**BLOCKED**

会計基盤自体(`ResourceType::MilitarySupplies`とその固定小数点API群)の実装は完了しており、
新規テスト65件すべてがpassし、Phase 21〜25の既存回帰テストも1件残らず無変更のまま合格した。
しかし、§6が必須とする`ResourceType::ALL`への追加が、本タスクの変更禁止範囲
(`src/ui/`・`locales/`相当の`assets/localization/`)にある既存コード
(`src/ui/economy_panel.rs`のstockpile表示ループ)と既存の回帰テスト
(`tests/p20_009_localization_headless_render_test.rs`)を通じて相互作用し、
スコープ外の修正なしには解消できない1件のテスト失敗を確定的に引き起こすことが判明した
(§28〜29・§34で詳述)。矛盾を隠して`COMPLETE`と報告することはできないため、
コード変更を必要最小限(会計基盤本体と新規テストのみ)に留め、`BLOCKED`として報告する。

## 2. 開始時HEAD

```
b44619a08b96b56d548607c769a8cf8895e0a3e4
```

`git rev-parse HEAD`で確認し、想定(P26-001設計監査コミット)と一致していた。
`git status --short`は開始時点で` M saves/savegame_v1.ron`のみであることを確認した。

## 3. 実装対象コードHEAD

```
b44619a08b96b56d548607c769a8cf8895e0a3e4
```

本タスクは`BLOCKED`のため未コミットであり、実装対象コードのHEADは開始時から
変化していない(作業ツリー上の差分として存在する)。

## 4. ResourceType追加内容

`src/economy/resources.rs`の`ResourceType` enumへ`MilitarySupplies`を9番目の
variantとして追加した(既存8variantの末尾に追加、既存並び順は一切変更していない)。
`ResourceType::ALL`(`[ResourceType; 8]` → `[ResourceType; 9]`)へ正確に1回追加し、
`display_name()`のmatchへ`"resource.military_supplies"`を1件追加した(翻訳カタログ
自体への追加は§5・§28のとおり本タスクの範囲外のため行っていない)。

## 5. MilitarySuppliesの意味

弾薬・燃料・軍用食料・医療整備物資・その他日次消耗品を抽象化した単一の消費補給物資
(P26-001設計監査§12・§13で確定した案A)。P26-002時点では内部分割しない。国家備蓄
(`CountryStockpile`)方式のみを対象とし、Division携行在庫は追加しない
(P26-001§14〜16のモデル1相当)。

## 6. MilitaryEquipment・Foodとの分離

型レベルで完全に分離した別`ResourceType` variant。定数
(`MILITARY_SUPPLIES_SCALE`/`MILITARY_SUPPLIES_MAX_UNITS`)・エラー型
(`MilitarySuppliesUnitsError`)・変換関数(`military_supplies_value_to_units`/
`military_supplies_units_to_value`)・`CountryStockpile`の生産加算API
(`add_military_supplies_produced`)/消費API(`try_consume_military_supplies`)は、
既存の`MilitaryEquipment`用API(`MILITARY_EQUIPMENT_SCALE`等・
`MilitaryEquipmentUnitsError`・`military_equipment_value_to_units`/
`military_equipment_units_to_value`・`add_military_equipment_produced`)とは
独立した名前空間として並存させ、共通化リファクタは一切行っていない
(§18で既存API側の無変更を確認)。Foodとは元々別variantであり、Foodの「消費」
機構自体が存在しない(P26-001§7・§13)ことも変更していない。

## 7. fixed-point scale

`MILITARY_SUPPLIES_SCALE = 1_000_000`(`MilitaryEquipment`と同じ、小数点以下6桁)。

## 8. scaleの精度根拠

P26-001設計監査§21の推奨値をそのまま採用した。既存資源生産は`f64`で小数第1〜2桁
程度の精度しか使わず、`DivisionDefinition.supply_usage`(Infantry=1.0/
Artillery=3.0/Mage=2.0、既存assets/data/divisions.ron)も同程度の精度である。
P25-002-REV-001が確立した`SCALE=1,000,000`は既にこの精度を大幅に上回り、Save V1の
f64往復・将来のP26-004日次配分(比例配分)で生じ得る丸め誤差蓄積を防ぐのに十分な
余裕を持つ。反証となる実データは監査中に見つからなかったため、P25-002-REV-001との
一貫性を優先してそのまま採用した。

## 9. 最大units

`MILITARY_SUPPLIES_MAX_UNITS = 1_000_000_000_000_000`(10^15、MilitarySupplies
値換算で10億)。`MilitaryEquipment`と同じ値をそのままドメイン上限として採用した。

## 10. 最大値の値域根拠

P26-002時点では`MilitarySupplies`を生産する建物Systemが未設計のため、正確な理論
最大生産量を算出できる実データが存在しない(存在しない生産式を捏造しないという
P26-001設計監査§21・spec §7の指示に従った)。したがって`MilitaryEquipment`が
実データ監査(現行州28、MilitaryFactory最大レベル10、150年間理論最大備蓄
約1,008万に対し10^15という値域が約99倍の余裕を持つことを確認済み、
`verification_logs/phase-25/p25-002-rev-001/`§9〜11)で確認済みの安全マージンを
そのまま流用した。`MilitarySupplies`の値スケール(師団あたり日次需要が高々数units、
150年間の理論最大備蓄が10^8〜10^9オーダーと見込まれる、P26-001設計監査§21)に
対しても同水準以上の余裕を持つ。`military_equipment_value_to_units`と同じ実測
往復変換サンプリング(0・1・SCALE近傍・MAX_UNITS近傍・境界の代表点)を
`military_supplies_value_to_units`に対しても新たに実行し、失敗が0件であることを
`tests/p26_002_military_supplies_accounting_foundation_e2e_test.rs`
(`test_b_units_to_value_to_units_round_trips_at_boundaries`等)および
`src/economy/resources.rs`内のunit test
(`military_supplies_round_trip_for_representative_units`等)で確認済み。

## 11. 150年間に対する余裕

上記§10のとおり、150年間の理論最大備蓄見込み(10^8〜10^9オーダー)に対し、
採用した`MAX_UNITS=10^15`(値換算10億)は同水準以上の余裕を持つ。ただし正確な
生産式が未設計であるため、P26-003で実際の生産式が確定した時点でこの暫定採用値を
正式な理論最大値監査と照合し直すことを推奨する(`src/economy/resources.rs`の
`MILITARY_SUPPLIES_MAX_UNITS`ドキュメンテーションコメントに明記済み)。

## 12. f64→units式

`units = round(value × MILITARY_SUPPLIES_SCALE)`。`military_supplies_value_to_units`
が実装(`src/economy/resources.rs`)。検証順序: finite→非負→scale乗算後もfinite→
`MILITARY_SUPPLIES_MAX_UNITS`以下→往復変換(`units→f64→units`)一致、の5段階。
`MilitaryEquipmentUnitsError`/`military_equipment_value_to_units`と全く同じ
規律を複製した。

## 13. units→f64式

`value = units / MILITARY_SUPPLIES_SCALE`(`military_supplies_units_to_value`)。
`military_equipment_units_to_value`と同じく、この関数自体はfallibleではない
(常に有限値を返す、`u64`の値域上`units as f64`がoverflowすることはなく、除算も
常にfiniteのため)。呼び出し側(`add_military_supplies_produced`/
`try_consume_military_supplies`)がこの後に往復再検証を行う。

## 14. 量子化格子外値の扱い

量子化(`* SCALE`→`round()`)により、格子外の値は最も近い格子点へ丸められる
(`test_b_just_before_and_after_a_quantization_boundary`で0.5境界の丸め方向に
かかわらず往復可能であることを確認)。丸め後、`units→f64→units`の往復が元の
unitsと一致しない場合は`RoundTripFailed`として拒否し、黙って近似値へフォール
バックしない(P25-002-REV-001と同じ規律)。

## 15. NaN・Infinity・負数の扱い

NaN・+Infinity・-Infinityは`NonFinite`として、負数(`-0.0`を除く)は`Negative`
として、いずれもpanicせず明確に拒否する(`test_c_rejects_nan_and_infinities_without_panicking`/
`test_c_rejects_negative_values`)。`-0.0`はIEEE754上`-0.0 < 0.0`が偽であるため
`Negative`としては拒否されず、`units=0`として受理される
(`test_c_negative_zero_is_accepted_as_zero`/
`military_supplies_negative_zero_is_treated_as_zero`で明示的に固定)。

## 16. 最大値超過の扱い

量子化後のunitsが`MILITARY_SUPPLIES_MAX_UNITS`を超える場合、および`value * SCALE`
の乗算結果自体がoverflowしてfiniteでなくなる場合は、いずれも`OutOfRange`として
拒否する(`test_c_rejects_value_beyond_max_and_f64_max`/
`test_c_multiplication_overflow_to_infinity_is_rejected`)。2^53近傍の曖昧な
巨大値(P25-002-REV-001が発見した問題の再導入防止、2^52・2^53・2^54・2^60を
個別に検証)もすべて`OutOfRange`として明確に拒否されることを確認した
(`test_c_rejects_ambiguous_two_pow_53_neighborhood_values`)。

## 17. エラー型

`MilitarySuppliesUnitsError`(`src/economy/resources.rs`)。バリアント:
`NonFinite`/`Negative`/`OutOfRange`/`RoundTripFailed`/`InsufficientStockpile`/
`ArithmeticOverflow`の6種。`MilitaryEquipmentUnitsError`(4種、
`NonFinite`/`Negative`/`OutOfRange`/`RoundTripFailed`)より2種多い。これは
`MilitaryEquipment`が「経済側の中立的なunits変換エラー
(`MilitaryEquipmentUnitsError`)」と「`military::equipment`モジュールの上位
会計エラー(`EquipmentAccountingError`、`InsufficientStockpile`等を含む)」の
2階層構造を持つのに対し、`MilitarySupplies`はP26-002の範囲では独立した
`military`側モジュールを新設しない(spec §5の変更範囲限定)ため、消費APIの
不足判定を`economy::resources`内の1つのenumで直接表現する必要があったためである。
`InsufficientStockpile`(消費時の備蓄不足、部分消費せず全量拒否)と
`ArithmeticOverflow`(u64の真のwraparound、現実的な値域では到達不能な防御的分岐)
を`OutOfRange`(ドメイン上限超過)と意味的に区別した。`InvalidStockpileState`
(spec §9が例示)に相当する概念はP26-002では該当箇所がない(国家備蓄自体には
Division側の`max_equipment`のような上限フィールドが存在しないため)ため、
このバリアントは追加していない。全バリアントがpanicしない設計。

## 18. 生産加算API

`CountryStockpile::add_military_supplies_produced(&mut self, produced: f64)
-> Result<(), MilitarySuppliesUnitsError>`。手順: (1)現在備蓄を正規unitsへ変換
(2)生産量を正規unitsへ変換 (3)units上でchecked_add(u64真のoverflowは
`ArithmeticOverflow`、`MAX_UNITS`超過は`OutOfRange`と区別) (4)成功時のみf64へ
正規変換 (5)往復再検証 (6)すべて成功した場合のみ`set()`で1回だけcommit。
失敗時は備蓄を完全に不変のまま返す。`produced=0.0`は成功する意味的no-op
(`test_d_zero_produced_is_a_deterministic_no_op`)。このAPIをどの本番生産
Systemからも呼び出していない(§28で確認)。

## 19. 消費API

`CountryStockpile::try_consume_military_supplies(&mut self, amount: f64)
-> Result<(), MilitarySuppliesUnitsError>`。手順: (1)現在備蓄を正規unitsへ変換
(2)消費要求量を正規unitsへ変換 (3)units上でchecked_sub(失敗時
`InsufficientStockpile`) (4)成功時のみf64へ正規変換 (5)往復再検証
(6)すべて成功した場合のみ`set()`で1回だけcommit。`requested > stockpile`の
場合は全量拒否し、部分消費はしない(`test_e_insufficient_stockpile_rejects_
the_entire_request_atomically`)。`amount=0.0`は成功する意味的no-op。

## 20. 原子性

生産加算・消費のいずれも「read→validate→units変換→checked演算→結果検証→
f64正規変換→1回だけcommit」の順を厳守し、途中で失敗した場合は`CountryStockpile`
全体がbit-for-bit不変であることをテストで確認した
(`test_d_invalid_current_stockpile_is_rejected_without_mutation`/
`test_d_invalid_produced_amount_is_rejected_without_mutation`/
`test_e_invalid_current_stockpile_is_rejected_without_mutation`/
`test_e_invalid_requested_amount_is_rejected_without_mutation`/
`test_d_exceeding_max_units_is_rejected_atomically`/
`test_e_underflow_beyond_stockpile_is_rejected_not_saturated`)。生のf64加減算・
saturating演算・minでの黙った丸めは一切使用していない(`grep`で
`add_military_supplies_produced`/`try_consume_military_supplies`の実装を
確認済み、両関数とも`checked_add`/`checked_sub`のみを使用)。

## 21. 世界保存式

`Σstockpile_after = Σstockpile_before + Σproduced - Σconsumed`(P26-002範囲、
Division携行在庫なしのため単純な国家備蓄合計)。生産加算量と世界備蓄増加量の
一致(`test_f_produced_units_increase_matches_world_stockpile_increase`)、
消費量と世界備蓄減少量の一致
(`test_f_consumed_units_decrease_matches_world_stockpile_decrease`)、
生産→消費後の期待値一致(`test_f_produce_then_consume_matches_expected_value`)、
100,000回の生産/消費反復でのドリフトなし
(`test_f_repeated_produce_consume_cycles_do_not_drift`)、複数国家(5か国)での
世界合計一致(`test_f_multiple_countries_world_total_is_conserved`、1国の消費
失敗が世界合計を変えないことも含む)を確認した。

## 22. 複数国家分離

ある国家への生産・消費操作が別国家の`CountryStockpile`に一切影響しないことを
`test_f_operating_on_one_country_never_affects_another`
(独立した`CountryStockpile`インスタンス2つ)、
`test_f_multiple_countries_world_total_is_conserved`(5か国)、
`military_supplies_and_military_equipment_stockpiles_are_independent`
(`src/economy/resources.rs`内unit test)で確認した。実装自体も`&mut self`の
単一`CountryStockpile`のみを操作し、他インスタンスへの参照を一切持たない。

## 23. 他ResourceTypeへの非影響

`test_f_other_resource_types_are_never_touched_by_military_supplies_operations`
(`ResourceType::ALL`の全variantを111.0にセットし、MilitarySupplies操作
(成功・失敗双方)後も他variantが不変であることを確認)、
`test_a_military_supplies_operations_do_not_affect_food_or_equipment`、
`add_military_supplies_produced_does_not_affect_other_resources`
(`src/economy/resources.rs`内unit test)で確認した。

## 24. Save V1構文互換性

`CountryStockpile.amounts`は`HashMap<ResourceType, f64>`の素の`Serialize`/
`Deserialize`のままであり、変更していない。新variant追加により旧SaveのRON
デシリアライズが失敗しないことを、`ResourceType::MilitarySupplies`キーへの
言及が一切ない手書きRON文字列
(`"(amounts:{Iron:10.0,Coal:5.0,MilitaryEquipment:42.0})"`、
`tests/p21_009_magic_crystal_chain_test.rs`の`old_save_ron_without_new_keys_
loads_with_safe_defaults`と同じ手法)を`ron::from_str::<CountryStockpile>`で
直接デシリアライズして確認した
(`test_g_old_v1_stockpile_ron_without_military_supplies_key_loads_successfully`)。

## 25. Save V1意味的互換性

上記の旧形式デシリアライズ結果で`get(MilitarySupplies)`が`0.0`、既存資源
(Iron/Coal/MilitaryEquipment)の値が不変であることを確認した(同テスト)。また
実Appを通したsave→load往復(`SaveGamePlugin`/`LoadGamePlugin`)でも、
量子化済みMilitarySupplies備蓄(小数値123.456789/456.789123で検証)のcanonical
unitsが変化しないこと、他ResourceType(Food)が維持されること、世界合計が
save/load単独では変化しないこと、2回連続save/loadでも変化しないことを
`test_g_app_save_load_round_trip_preserves_quantized_stockpile_and_other_resources`
で確認した。RON往復単体でも
`test_g_stockpile_with_military_supplies_round_trips_through_ron`/
`test_g_double_round_trip_through_ron_does_not_change_units`で確認済み。

## 26. New Game／Loaded Game比較

`test_h_new_game_and_loaded_game_agree_on_identical_stockpile_value`
(継続実行App vs. save→load後Appで同じMilitarySupplies値になることを確認、
P25-002-REV-001のTest Oと同型)、`test_h_new_game_starts_at_zero_for_all_countries`
(New Gameの全6か国がMilitarySupplies=0)、
`test_h_loaded_old_v1_save_also_starts_at_zero`(旧V1相当ロードでも0、他資源
(Iron)は維持)で確認した。P26-002時点では本番Systemが一切接続されていないため、
両者は自明に一致する。

## 27. save/load分岐比較

上記§25・§26のApp経由テスト(`test_g_app_save_load_round_trip_...`/
`test_h_new_game_and_loaded_game_agree_on_identical_stockpile_value`)が、
実際の`SaveRequestMessage`/`LoadRequestMessage`経路(P25-002-REV-001のTest N/O
と同じ実装済みSave/Loadパイプライン)を通した分岐比較に該当する。

## 28. 本番Systemへ未接続であること

`rg -n "MilitarySupplies" src`・`rg -n "add_military_supplies_produced|
try_consume_military_supplies" src`を実行し、`src/economy/resources.rs`
(定義本体)以外に一切の参照がないことを確認した(UI・ローカライズへの参照も
追加していない——`display_name()`のmatch腕1件のみ)。`rg -n "military::supply|
process_supply" src`は`src/military/supply.rs`(定義)と`src/military/tests.rs`
(テストからの呼び出しのみ)のみがヒットし、`LogisticsPlugin::build`は依然
空実装であることを確認した。実App経由の追加テスト
`test_h_production_app_ticks_alone_do_not_change_military_supplies`
(日次ティック9回+月次ティック1回)、
`test_i_equipment_and_manpower_accounting_is_unaffected_by_new_resource_type`
(MilitarySupplies APIを直接呼び出してもMilitaryEquipment/manpowerに影響
しないこと)でも、本番経路からの生産・消費が一切発生しないことを確認した。

## 29. dead supply経路を再活性化していないこと

`src/military/supply.rs`(`process_supply`)・`src/military/combat.rs`
(`calculate_combat_strength`)・`src/war/combat.rs`(`process_combat`)は
1バイトも変更していない(§18のS23参照)。実App経由の新規テスト
`test_i_division_supply_ratio_remains_untouched_by_daily_ticks`で、New Game
時点でDivision.supply_ratioが既存デフォルト値1.0であること、5日分の日次
ティック後も1.0のまま変化しないこと(もし`process_supply`が誤って本番接続
されていれば、州の`logistics_ratio`で上書きされ失敗するはずのテスト)を確認
した。`src/military/update.rs`の既存回帰テスト
`test_u_dead_war_combat_process_combat_path_remains_unconnected`も無変更の
まま合格した(§37の`cargo test --lib military::`で283 passed、baseline通り)。

## 30. Phase 25会計の保護

`src/military/equipment.rs`・`src/economy/resources.rs`内の`MilitaryEquipment`
関連コード(`MILITARY_EQUIPMENT_SCALE`/`MILITARY_EQUIPMENT_MAX_UNITS`/
`MilitaryEquipmentUnitsError`/`military_equipment_value_to_units`/
`military_equipment_units_to_value`/`add_military_equipment_produced`)は
1文字も変更していない(`git diff`で該当範囲に差分がないことを確認)。P25の
全E2Eテスト8ファイル(`p25_002`〜`p25_007`、`p25_002_rev_001`/
`p25_006_rev_001`含む)を個別に実行し、全件baseline通りのpass数で無変更の
まま合格した(§37参照)。

## 31. Phase 23・24会計の保護

`src/military/manpower.rs`・`src/population/`等のA・B・L・K・X・manpower cap・
population・workforce・workforce fixed-point丸め・mobilized manpower按分・
Save V1既存フィールドは一切変更していない。P23関連E2Eテスト7ファイル
(`p23_002`/`p23_004`/`p23_006`/`p23_007`/`p23_008`/`p23_009`/`p23_010`)・
P24関連E2Eテスト1ファイル(`p24_002`)を個別に実行し、全件baseline通りの
pass数で無変更のまま合格した(§37参照)。

## 32. P26-003へ引き継ぐ内容

- `MilitarySupplies`会計基盤(定数・変換関数・生産加算/消費API)はP26-003
  (補給物資生産の月次経済への配線)からそのまま呼び出し可能な状態にある。
- `MILITARY_SUPPLIES_MAX_UNITS`は暫定値(§10〜11参照)であり、P26-003で
  実際の生産式(新設`SupplyDepot`相当の建物)が確定した時点で理論最大値監査を
  やり直すことを推奨する。
- §1の`BLOCKED`原因(`resource.military_supplies`翻訳キー未登録)は、
  P26-003着手前(または独立した極小タスク)で解消することを強く推奨する
  (§33〜34に具体的な最小修正案を記載)。

## 33. 対象外を先行実装していないこと

Division日次需要計算・日次配分・戦闘力補正(`CombatReadiness`拡張)・移動速度
補正・経路探索/空間モデル・携行在庫・Attrition・AI接続・UI表示・Save V2/
migrationのいずれも実装していない。`src/military/data.rs`(Division/
DivisionDefinition定義)・`src/state/data.rs`(StateData)は1バイトも変更して
いない。P26-004で予定されている`largest_remainder_allocate`共通ヘルパーの
抽出も行っていない。

## 34. 発見したブロッカーの詳細

**根本原因**: spec §6が必須とする`ResourceType::ALL`への`MilitarySupplies`
追加(§4)は、`src/ui/economy_panel.rs`のstockpile表示ループ
(`for res in ResourceType::ALL { ... t(&catalog, locale.0, res.display_name()) ... }`、
行200付近)を経由して、翻訳カタログ(`assets/localization/en-US.ron`/
`assets/localization/ja-JP.ron`)に存在しないキー`"resource.military_supplies"`
を要求する。`translate()`(`src/localization.rs`)は未登録キーに対して
panicはしないが、`⟦MISSING:resource.military_supplies⟧`という検出可能な
マーカー文字列を返す設計になっている(意図的なfail-safe)。

既存の回帰テスト`tests/p20_009_localization_headless_render_test.rs::
localization_headless_render_ja_en_switch_and_back`(P20-009、Phase 20で
確立された「全パネルを開いた状態で未翻訳キーが1件もないこと」を検証する
ハード assertion)が、このマーカーの出現を検出して`panic!`する:

```
thread 'localization_headless_render_ja_en_switch_and_back' panicked at
tests\p20_009_localization_headless_render_test.rs:890:9:
[P20-009] missing keys after opening all panels (ja-JP): ["key='economy_panel.info'
text='...  軍需物資: 0\n  ⟦MISSING:resource.military_supplies⟧: 0\n...'"]
```

この失敗は`cargo test --test p20_009_localization_headless_render_test`を
独立実行しても100%再現する決定論的な失敗であり、フレーク(flaky)ではない
(2回連続で同一の失敗を確認済み)。実装前の`git status --short`基準
(baseline: 1833 passed/0 failed/1 ignored)ではこのテストはpassしていた
(このセッション冒頭で独自に再実行して確認済み)ため、この失敗は本タスクの
`ResourceType::ALL`変更によって新たに生じたものであり、既存の欠陥や無関係な
flakinessではない。

**解決に必要な変更とその禁止**: 解決には次のいずれかが必要だが、いずれも
本タスクの変更禁止範囲(spec §5「次は変更しないでください」が明示する
`src/ui/`・`locales/`相当のローカライズデータ)に該当するため実施しなかった。

1. `assets/localization/en-US.ron`・`assets/localization/ja-JP.ron`へ
   `("resource.military_supplies", "Military Supplies")`/
   `("resource.military_supplies", "軍需補給物資")`相当の1行を追加する
   (既存の`resource.military_equipment`/`resource.raw_magic_crystal`と
   同じ形式、`assets/localization/en-US.ron:464-467`・
   `assets/localization/ja-JP.ron:463-466`付近)。
2. `src/ui/economy_panel.rs`のstockpile表示ループで`MilitarySupplies`を
   一時的にスキップする。

いずれも経済・軍事会計ロジックには一切触れない1〜数行の変更で解決可能だが、
本タスクの明示的なスコープ外である。`tests/p20_009_localization_headless_
render_test.rs`自体を緩和する案は採用しなかった——これは本物のUI完全性
リグレッションを覆い隠すことになり、「発見した不具合を自己判断でスコープ外
と処理しない」という方針に反するため。

**P26-001設計監査との関係**: この事象はP26-001設計監査§36が「UIパネルが
未知のResourceTypeへ即座にpanicする実装でない限り、表示欄が1つ増えないだけで
済むはずである……本タスクではUIコード自体は読んでいないため、この点は
P26-002実装時の受け入れ条件に含めることを推奨する」と明記して未検証のまま
残していたリスクそのものである。実際にはUIはpanicしないが、既存の
翻訳完全性回帰テストが未翻訳キーの出現を確定的に検出して失敗する、という
P26-001が想定していなかった具体的な失敗モードが判明した。

## 35. Critical／High／Medium／Low

- **Critical: 0件**
- **High: 1件** — §34で詳述した`tests/p20_009_localization_headless_render_test.rs::
  localization_headless_render_ja_en_switch_and_back`の確定的な失敗。プレイヤーが
  実際にeconomy_panelを開いた場合、`⟦MISSING:resource.military_supplies⟧`という
  可視の未翻訳マーカーが両言語で表示される、実際にプレイ体験へ影響する既存回帰の
  破壊であるため`High`と判定した(データ破損やクラッシュを伴わないため`Critical`
  ではない)。本タスクのスコープ内では解決不能(§34)。
- **Medium: 0件**
- **Low: 1件** — `MILITARY_SUPPLIES_MAX_UNITS`が暫定値であり、正式な生産式
  確定後の理論最大値監査再実施が必要(§10〜11・§32)。

`Critical/High findingが0`という完了条件(spec §22)を満たさないため、
総合判定は`COMPLETE`ではなく`BLOCKED`とする。

## 36. 変更したファイル

- `src/economy/resources.rs`(既存ファイル、421行追加・1行削除):
  `ResourceType::MilitarySupplies`追加、`ResourceType::ALL`を9要素化、
  `display_name()`へ1腕追加、`MilitarySupplies`固定小数点定数・エラー型・
  変換関数・`CountryStockpile`の生産加算/消費APIを追加、対応する20件の
  unit testを追加。
- `tests/p26_002_military_supplies_accounting_foundation_e2e_test.rs`
  (新規ファイル、996行、45件のE2Eテスト、Category A〜I)。
- `verification_logs/phase-26/p26-002/p26-002_military_supplies_
  accounting_foundation.md`(本報告書、新規)。

上記以外のファイル(`assets/localization/*.ron`・`src/ui/`・`src/military/`
配下・`src/save/`・`src/country/elimination.rs`等)は一切変更していない。

## 37. 実行したコマンドと正確な結果

すべて逐次実行(並列実行なし)。

| コマンド | 結果 |
|---|---|
| `git rev-parse HEAD`(開始時) | `b44619a08b96b56d548607c769a8cf8895e0a3e4` |
| `git status --short`(開始時) | ` M saves/savegame_v1.ron`のみ |
| `cargo test --all-targets`(開始時baseline、自己再検証) | 1833 passed; 0 failed; 1 ignored |
| `cargo clippy --all-targets --all-features -- -D warnings`(開始時baseline) | 0 warnings |
| `cargo build --lib` | 成功、0エラー |
| `cargo test --lib economy::` | 32 passed; 0 failed(baseline 12 + 新規20) |
| `cargo test --lib military::` | 283 passed; 0 failed(baseline通り無変更) |
| `cargo test --lib save::` | 300 passed; 0 failed(baseline通り無変更) |
| `cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test` | 45 passed; 0 failed |
| `cargo test --test p23_002_mobilized_manpower_workforce_e2e_test` | 8 passed(baseline通り) |
| `cargo test --test p23_004_economic_workforce_ratio_e2e_test` | 7 passed(baseline通り) |
| `cargo test --test p23_006_manpower_ledger_e2e_test` | 22 passed(baseline通り) |
| `cargo test --test p23_007_division_demobilization_e2e_test` | 24 passed(baseline通り) |
| `cargo test --test p23_008_daily_reinforcement_e2e_test` | 21 passed(baseline通り) |
| `cargo test --test p23_009_ai_recruitment_e2e_test` | 22 passed(baseline通り) |
| `cargo test --test p23_010_phase23_manpower_lifecycle_audit_test` | 9 passed(baseline通り) |
| `cargo test --test p24_002_workforce_rounding_unification_e2e_test` | 17 passed(baseline通り) |
| `cargo test --test p25_002_equipment_accounting_foundation_e2e_test` | 15 passed(baseline通り) |
| `cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test` | 6 passed(baseline通り) |
| `cargo test --test p25_003_recruitment_equipment_issue_e2e_test` | 22 passed(baseline通り) |
| `cargo test --test p25_004_combat_equipment_loss_e2e_test` | 5 passed(baseline通り) |
| `cargo test --test p25_005_daily_equipment_reinforcement_e2e_test` | 10 passed(baseline通り) |
| `cargo test --test p25_006_demobilization_equipment_return_e2e_test` | 29 passed(baseline通り) |
| `cargo test --test p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test` | 8 passed(baseline通り) |
| `cargo test --test p25_007_equipment_combat_effectiveness_e2e_test` | 21 passed(baseline通り) |
| `cargo test --all-targets`(実装後、1回目) | ビルドキャッシュ破損(下記§39参照)で失敗 |
| `cargo test --all-targets`(実装後、2回目・逐次再実行) | 同様に失敗、`cargo clean -p strategy_game`実施 |
| `cargo test --all-targets --no-fail-fast`(cache clean後) | **1897 passed; 1 failed; 1 ignored** |
| `cargo test --test p20_009_localization_headless_render_test`(単独、失敗の再現性確認) | 3 passed; 1 failed(2回連続で同一の決定論的失敗を確認) |
| `cargo clippy --all-targets --all-features -- -D warnings`(実装後) | 0 warnings |

## 38. 全テストpassed／failed／ignored件数

- 開始時baseline: **1833 passed / 0 failed / 1 ignored**(このセッション冒頭で
  独自に`cargo test --all-targets`を実行して再検証済み)。
- 実装後最終: **1897 passed / 1 failed / 1 ignored**(`cargo test --all-targets
  --no-fail-fast`)。
- 新規追加テスト数: 65件(`src/economy/resources.rs`内unit test 20件 +
  `tests/p26_002_military_supplies_accounting_foundation_e2e_test.rs`45件)。
  すべてpass。
- 差分検証: baseline総試行数1834件(1833+0+1) → 最終総試行数1899件
  (1897+1+1)。差分65件は新規追加テスト数と完全一致し、既存テストのうち
  新たにfailへ転じたのは1件(`localization_headless_render_ja_en_switch_and_back`、
  §34で詳述)のみであることを算術的に確認した(1833−1897+65=1)。この1件を
  除き、既存の全1832件は無変更のままpassし続けている。

## 39. clippy警告数

`cargo clippy --all-targets --all-features -- -D warnings`は実装後も
**0 warnings**(実装前baselineと完全一致)。

## 40. Cargo逐次実行

全てのcargoコマンドを逐次実行した(並列実行なし)。実装後の`cargo test
--all-targets`初回・2回目実行時に`error[E0786]: found invalid metadata files
for crate 'strategy_game'`(不正なmetadataファイル)および
`ページング ファイルが小さすぎるため、この操作を完了できません。(os error 1455)`
というOSレベルのメモリマップ失敗が発生し、これに起因してビルドキャッシュ
(`target/debug/deps/*.rlib`)が破損した。これは並行cargo実行によるものではなく
(本セッションを通じてcargoコマンドは常に単一プロセスで逐次実行し、他の
バックグラウンドプロセスも起動していない)、`--all-targets`が多数の
テストバイナリ(54個)を同時ビルドする際にWindowsのページングファイル容量が
不足したことによる一過性のOSリソース枯渇が原因と判断した。同一コマンドの
単純な再実行(2回目)でも同じ破損状態(`E0786`)が再現したため、通常の逐次
再実行だけでは解決しないキャッシュ破損と判断し、`cargo clean -p strategy_game`
(strategy_gameクレートのみを対象とした限定clean、他クレートの依存キャッシュは
保持)を実行した。この限定cleanの後、`cargo test --all-targets --no-fail-fast`
は正常に完走し(§37・§38)、以降のすべてのコマンドも正常に完了した。

## 41. ユーザーセーブの保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージ・
コミットしていない。開始時・全検証コマンド実行後・本報告書作成時点のいずれの
`git status --short`でも` M saves/savegame_v1.ron`が変化せず残り続けたことを
確認した(下記の最終`git status --short`参照)。既知の非決定的スクリーンショット
PNG(9件)は、`cargo test --all-targets`実行のたびに一部が再生成されたが、
そのつど該当パスのみを個別に`git checkout --`で復元し、`saves/savegame_v1.ron`
以外の差分が残らないことを都度確認した。

## 最終`git status --short`(本報告書作成時点、コミット前)

```
 M saves/savegame_v1.ron
 M src/economy/resources.rs
?? tests/p26_002_military_supplies_accounting_foundation_e2e_test.rs
?? verification_logs/phase-26/p26-002/
```

`saves/savegame_v1.ron`以外は本タスクが意図的に追加・変更したファイルのみ
であり、これらを`git add`で個別指定してステージし、1コミットとして記録する
(BLOCKED判定であっても、実装・テスト・報告書は指示どおり1コミットで記録する)。
