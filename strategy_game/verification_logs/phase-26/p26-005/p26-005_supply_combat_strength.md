# P26-005: 補給充足率の戦闘力反映 実装報告書

## 1. 総合判定

**COMPLETE**

P26-004が確立した`MilitaryRegistry::supply_fulfillment_of(DivisionId) -> f64`(日次で
確定するDivisionごとの補給充足率、欠落時default 1.0)を、実際にゲームプレイへ影響する
戦闘力計算(`military::combat_calc`、`military::update::process_daily_battles`経由で
`handle_daily_military`から呼ばれる正規経路)へ接続した。既存の戦闘準備率
(`combat_readiness = min(manpower_ratio, equipment_ratio)`、P25-007)へ、補給充足率を
3つ目の要素として`min`で1回だけ合成する式
(`combat_readiness = min(manpower_ratio, equipment_ratio, supply_fulfillment)`)へ拡張した。

既存の公開関数(`calculate_combat_readiness`・`resolve_combat_day`・
`resolve_combat_day_multi`)はいずれもsignatureを変更せず、内部で新設した
`_with_supply`版(`calculate_combat_readiness_with_supply`・
`resolve_combat_day_with_supply`・`resolve_combat_day_multi_with_supply`、
いずれも補給率引数を明示的に取る)を`supply_fulfillment = 1.0`で呼ぶ互換wrapperとして
再定義した。これにより`tests/land_war_combat_peace_test.rs`(変更禁止)を含む既存の
全呼び出し元は無変更のままコンパイル・動作し、補給率1.0の場合は数学的に既存結果と
完全に一致する(`min(m, e, 1.0) = min(m, e)`、1.0はmanpower/equipment比率の最大値
1,000,000units以上になり得ないため)。

実際に稼働している唯一の戦闘経路は`military::update::process_daily_battles`
(`handle_daily_military`から呼ばれ、`DailySimulationSet::MilitaryAction`に登録済み)
であることをコード監査で確認し、`resolve_combat_day_multi_with_supply`をそこへ接続した。
`military::supply::process_supply`(旧state.logistics_ratioベース)・
`war::combat::process_combat`(P25-001監査で死んだ経路と確認済み)はいずれも
本タスクでも一切変更・再活性化していない(未接続のままであることを新規E2Eテストで
再確認した)。

新規単体テスト30件(`src/military/combat_calc.rs`24件+`src/military/update.rs`6件)+
新規E2Eテスト10件(`tests/p26_005_supply_combat_strength_e2e_test.rs`)で、3要素min合成・
積合成でないことの悉皆確認・境界値・攻撃側/防御側対称適用・国家間分離・実Systemでの
同日順序・Save V1互換性・save/load分岐・決定性・下流(撃破可否・州支配権)への伝播・
P26-004が実測したOceanicの供給不足(36か月972日中418日)が実際に戦闘力低下として
現れることを検証した。全テスト2274 passed/0 failed/1 ignored(開始時2234から+40)、
clippy 0 warnings。

## 2. 開始時HEAD

```text
c97ae7f3a1d56e603ef5d8363fdb7a6e60c53315
```

`git rev-parse HEAD`・`git status --short`・`git show --stat --oneline HEAD`で確認し、
想定(`feat: add daily military supply allocation`、P26-004のコミット)と一致していた。
既存差分は` M saves/savegame_v1.ron`のみで、内容は一切参照・変更・コミットしていない。

## 3. 監査対象HEAD

`c97ae7f3a1d56e603ef5d8363fdb7a6e60c53315`(開始時HEADと同一。本タスクはこのコミットの
上に作業ツリー上で実装し、末尾に1コミットとして記録する)。

## 4. 実Appの正規戦闘経路

コード監査(`rg`による全呼び出し元探索)で確認した実際の日次時系列:

```text
DayChangedMessage
→ (月境界日のみ) DailySimulationSet::Economy前段: economy::handle_monthly_economy
  (MilitarySupplies等の月次生産、P26-003)
→ DailySimulationSet::Economy後段: daily_supply::process_daily_military_supply_consumption
  (P26-004、Division日次需要確定→国家備蓄から実消費→Hamilton配分→
  MilitaryRegistry::division_supply_fulfillment確定)
→ DailySimulationSet::MilitaryAction: military::update::handle_daily_military
    1. process_recruitment(募兵完了)
    2. apply_daily_reinforcement(manpower補充)
    3. apply_daily_equipment_reinforcement(装備補充)
    4. validate_and_stop_invalid_movements / process_movement(移動・侵攻)
    5. process_daily_battles ← 本タスクでここを変更
         a. 参加Division全員のreadiness妥当性をpreflight検証
            (calculate_combat_readiness_with_supply、補給率を含む)
         b. resolve_combat_day_multi_with_supply(実効戦力算出・被害配分)
         c. apply_combat_casualty(manpower/equipment/organization適用)
    6. resolve_finished_battles(撃破判定・撤退・占領)
    7. battle_registry.cleanup_finished_battles()
    8. process_org_recovery
```

`DailySimulationSet`の宣言順(`TimeUpdate → Economy → ... → MilitaryAction → ...`、
`src/app/time.rs`の`configure_sets(...).chain()`)により、Economy Set全体が
MilitaryAction Setより必ず先に完了することが構造的に保証されている
(P26-004で既に確立済み、本タスクでは一切変更していない)。

## 5. 死んだ旧経路

* `military::supply::process_supply`(`src/military/supply.rs`、`StateRegistry.
  logistics_ratio`ベースの旧補給比率をDivision.supply_ratioへ書き込む経路)は、
  `src/military/tests.rs`のユニットテストから直接呼ばれる以外、どのBevy Systemにも
  登録されていない(`rg -n "process_supply\b" src`で確認)。本タスクでも一切変更・
  再活性化していない。
* `war::combat::process_combat`(`src/war/combat.rs`)は、P25-001監査・P25-004・
  P25-007で繰り返し確認済みの死んだ経路(`military::combat::calculate_combat_strength`
  ―`Division.supply_ratio`を使う旧式―を呼ぶ)であり、`src/military/update.rs`・
  `src/war/tests.rs`のユニットテストから直接呼ばれる以外、どのBevy Systemにも
  登録されていない。本タスクでも一切変更・再活性化していない。新規E2Eテスト
  `test_dead_paths_remain_unconnected_after_p26_005`で、この経路に著しく不正な
  補給率(0.0)を`MilitaryRegistry.division_supply_fulfillment`へ注入しても、
  `process_combat`がその値を一切参照せず通常どおりダメージを与えることを実測し、
  「未接続のまま」であることを直接証拠として確認した。

## 6. 変更前の戦闘力式

```text
combat_readiness_units = min(manpower_readiness_units, equipment_readiness_units)
effective_power = base_power × combat_readiness × org_ratio
```

(`calculate_combat_readiness`、P25-007。補給率は一切関与しない。)

## 7. 変更後の三要素min式

```text
combat_readiness_units = min(
    manpower_readiness_units,
    equipment_readiness_units,
    supply_readiness_units
)
effective_power = base_power × combat_readiness × org_ratio
```

(`calculate_combat_readiness_with_supply`。`supply_readiness_units`は
`supply_fulfillment(0.0..=1.0のf64)`を`COMBAT_READINESS_SCALE(1,000,000)`単位へ
`round()`変換し`.min(COMBAT_READINESS_SCALE)`で明示的にclampした値。)

積合成(`m×e×s`)・部分min×補給率(`min(m,e)×s`)・完成済み戦力への後掛け
(`existing_strength×s`)のいずれでもないことを、`p26_005_is_not_multiplicative_
composition_across_a_sweep`(m/e/sそれぞれ20/40/60/80/100%の全125通りの組み合わせに
ついて、`combat_readiness_units`が3要素の`min`と厳密に一致し、かつ2つ以上が非100%の
場合は積合成の値と一致しないことを悉皆確認)・`p26_005_spec_11_equal_thirds_gives_
fifty_not_twelve_point_five`(M=E=S=0.5 → R=0.5、R=0.125にならないことを明示的に否定)
で恒久的に固定した。

## 8. manpower ratioの取得元

既存の`calculate_combat_readiness_with_supply`内、`u128`のchecked演算による
`manpower / max_manpower`(P25-007から無変更)。数式・丸め規則・境界条件
(`manpower > max_manpower`をErrで拒否、`max_manpower == 0`は0)は一切変更していない。

## 9. equipment ratioの取得元

既存の`military_equipment_value_to_units`(fixed-point変換、Phase 25)による
`equipment_units / max_equipment_units`(P25-007から無変更)。数式・丸め規則・
境界条件(`max_equipment == 0`は100%扱い、超過はErr)は一切変更していない。

## 10. supply fulfillmentの取得元

`MilitaryRegistry::supply_fulfillment_of(DivisionId) -> f64`(P26-004が確立した
読み取り専用API)。`process_daily_battles`のpreflight検証・`resolve_combat_day_multi_
with_supply`のいずれも、DivisionIdをキーにこのアクセサ経由でのみ取得しており、
本番コード(`process_daily_battles`のpreflight・`resolve_combat_day_multi_with_
supply`)が`division_supply_fulfillment`マップを直接参照する箇所・Bevy Entity IDを
キーにする箇所・本タスクでこのマップへ書き込む箇所は存在しない。テストコード内では、
シナリオ構築のため`MilitaryRegistry.division_supply_fulfillment`(`pub`フィールド、
専用setterなし)へ直接`.insert()`することがあるが、これは既存のP26-004自身のテスト
群と同じ慣用パターンであり、本番の読み取り経路とは無関係である。

## 11. 補給率欠落時のdefault

`supply_fulfillment_of`自体の契約(P26-004)どおり`1.0`。新規Division・旧Save・
まだ日次補給処理を経ていないDivisionのいずれも、`resolve_combat_day_multi_with_
supply`が`registry.supply_fulfillment_of(id)`を呼ぶ際に自然に`1.0`を受け取る
(本タスク側で追加のdefault処理は行っていない、単体テスト
`p26_005_resolve_combat_day_multi_with_supply_missing_registry_entries_default_to_full`・
E2Eテスト`test_d4_old_save_without_fulfillment_field_defaults_combat_to_full_supply`
で確認)。

## 12. 攻撃側への適用

`resolve_combat_day_with_supply`の`atk_supply_fulfillment`引数、
`resolve_combat_day_multi_with_supply`のN対M経路では攻撃側の各Divisionについて
`registry.supply_fulfillment_of(a.id)`を`effective_power_1000_with_supply`へ渡す。
単体テスト`p26_005_resolve_combat_day_attacker_supply_shortage_reduces_damage_dealt`・
`p26_005_real_path_attacker_supply_shortage_reduces_damage_dealt_to_defender`(実
production経路)、E2Eテスト`test_c1_real_daily_supply_shortage_reduces_same_day_
combat_damage_dealt`(実App)で確認。

## 13. 防御側への適用

攻撃側と全く同じ`calculate_combat_readiness_with_supply`/
`effective_power_1000_with_supply`を防御側にも適用する(役割ごとの特別扱いなし)。
単体テスト`p26_005_resolve_combat_day_defender_supply_shortage_reduces_defense_
contribution`・`p26_005_real_path_defender_supply_shortage_reduces_damage_dealt_
to_attacker`で確認。

## 14. 二重適用防止

補給率は`calculate_combat_readiness_with_supply`内の`min`へ1回だけ入り、その結果
(`combat_readiness_units`)だけが`effective_power`の乗算へ使われる。呼び出し階層上、
補給率が2箇所以上で参照される経路は存在しない(`resolve_combat_day_with_supply`→
`combat_readiness_1000_with_supply`→`calculate_combat_readiness_with_supply`の
単一経路、`effective_power_1000_with_supply`も同じ単一経路)。

## 15. 積合成を採用しないこと

§7参照。悉皆テスト(125通りの組み合わせ)と、spec §11の再発防止ケース(M=E=S=0.5が
0.125にならないこと)の両方で恒久的に固定した。

## 16. System順序

変更していない。P26-004が確立した`daily_supply::process_daily_military_supply_
consumption`(`DailySimulationSet::Economy`、`economy::handle_monthly_economy`の
`.after()`)と`handle_daily_military`(`DailySimulationSet::MilitaryAction`)の
Set順序制約(`DailySimulationSet`の宣言順`.chain()`)がそのまま「同日の日次補給結果を
戦闘が読む」ことを保証しているため、本タスクでSystem登録・順序を一切変更する必要が
なかった(§9のBLOCKED条件`同日補給→戦闘の順序を保証できない`には該当しない)。

## 17. 同日補給→戦闘の確認

E2Eテスト`test_c1_real_daily_supply_shortage_reduces_same_day_combat_damage_dealt`
(実App、備蓄0 vs 備蓄十分の対照シナリオで同日の被害量を比較)・
`test_c2_month_boundary_production_before_same_day_combat_reads_post_production_
fulfillment`(月境界日に月次生産が同日の消費より先に確定し、その回復後の値を
同日の戦闘が読むことを、P26-004 test_f1と同じ技法[Arcadia、月6まで進行後に
`military_supplies_units_to_value(999_999)`まで備蓄を落とす]で実測)で実App経由
確認した。

## 18. 補給率1.0の回帰結果

* `calculate_combat_readiness`(互換wrapper)は全入力について`calculate_combat_
  readiness_with_supply(..., 1.0)`と完全一致(`p26_005_calculate_combat_readiness_
  defaults_supply_to_full_and_matches_with_supply_one`)。
* `resolve_combat_day`は`resolve_combat_day_with_supply(..., 1.0, 1.0)`と完全一致
  (`p26_005_resolve_combat_day_full_supply_matches_legacy_resolve_combat_day`)。
* `resolve_combat_day_multi`は`resolve_combat_day_multi_with_supply`(欠落マップ、
  default 1.0)と完全一致(`p26_005_resolve_combat_day_multi_with_supply_full_
  supply_matches_legacy_multi`)。
* 既存のP25-007 E2Eテスト21件(`tests/p25_007_equipment_combat_effectiveness_e2e_test.rs`)、
  `tests/land_war_combat_peace_test.rs`(変更禁止、実際に無変更)を含む既存回帰は
  全てそのまま合格した(§35参照)。許容誤差を導入する必要は一切生じなかった
  (bit-for-bit一致)。

## 19. 補給率0.5の結果

`p26_005_table_supply_half_limits_readiness_to_fifty_percent`(manpower=1.0,
equipment=1.0, supply=0.5 → readiness=0.5、`combat_readiness_units=500,000`で厳密に
一致)。実production経路でも`p26_005_real_path_attacker_supply_shortage_reduces_
damage_dealt_to_defender`で、攻撃側供給率0.5のとき防御側への被害が供給率1.0より
確実に小さいことを確認。

## 20. 補給率0.0の結果

`p26_005_table_zero_supply_gives_zero_readiness_even_with_full_manpower_and_
equipment`(`combat_readiness_units=0`)。`p26_005_resolve_combat_day_zero_supply_
on_both_sides_does_not_panic_or_produce_nan`で、両者とも被害0・organization損失0.0
(有限値)となり、panic・NaN・Infinityが一切発生しないことを確認。

## 21. manpowerがボトルネックの結果

`p26_005_resolve_combat_day_manpower_bottleneck_dominates_over_better_supply`
(manpower20%・equipment100%・supply100% は、manpower100%・equipment100%・
supply50%より厳しいボトルネックであることを比較実証)。

## 22. equipmentがボトルネックの結果

`p26_005_resolve_combat_day_equipment_bottleneck_dominates_over_better_supply`
(同様にequipment20%が供給50%より厳しいボトルネックであることを比較実証)。

## 23. Oceanic不足日の実測

`test_oceanic_36_month_shortage_is_measured_and_demonstrably_weakens_combat_
readiness`(実App、36か月・972日、DWARFをplayerとした通常AI playシナリオ)の実測:

```text
shortage_days=418
first_shortage_day=32
min_fulfillment=0.000000
first shortage: day=32 stockpile_before_that_days_consumption=0.000000
  division_id=3 fulfillment_that_day=0.000000
final-day readiness: actual_fulfillment=0.204395
  combat_readiness_units(actual)=204395
  combat_readiness_units(if_full_supply)=1000000
diagnostic battle at first shortage day:
  def_loss(shortage_attacker)=0  def_loss(full_supply_attacker)=100
```

`shortage_days=418`・`first_shortage_day=32`は、P26-004報告書§32.2が記録した数値と
完全に一致する(供給会計自体を一切変更していないことの直接的な証拠)。36か月終了時点の
Oceanic師団は補給充足率0.204395まで低下しており、これをそのまま
`calculate_combat_readiness_with_supply`へ通すと`combat_readiness_units`が
1,000,000(満タン仮定)から204,395まで低下することを確認した。さらに、不足発生初日
(day32、fulfillment=0.0)の実測値を使った診断用戦闘(同一manpower/equipment/base
strengthの2師団を対戦させ、補給率だけを実測値/1.0で差し替え)で、補給率0.0の攻撃側は
防御側への被害が0、補給率1.0なら100になることを確認した――P26-004が実測した既知の
供給網の弱さが、実際にゲームプレイ上の戦闘力低下として現れることの直接的な証拠である。
Oceanicを特別扱いする本番コードは一切追加していない(`calculate_combat_readiness_
with_supply`にCountryId分岐は存在しない)。

## 24. 国家間分離

* 単体テスト`p26_005_real_path_one_countrys_supply_shortage_does_not_leak_into_an_
  unrelated_battle`(country_a[補給10%] vs country_bの戦闘とcountry_c vs country_d
  [両者とも補給100%]の戦闘を同時実行し、後者が独立基準runと完全一致することを確認)。
* E2Eテスト`test_isolation_one_countrys_supply_shortage_does_not_leak_into_
  unrelated_war`(実App、DWARF[備蓄0] vs ELFINの戦争とOCEANIC vs FERROWYNの戦争を
  同日に処理し、後者がDWARFの状態と無関係な独立シナリオと完全一致することを確認、
  P26-004の国家別備蓄保存則[自国消費分しか動かない]も同時に確認)。
* 単体テスト`p26_005_resolve_combat_day_multi_with_supply_n_vs_m_reads_per_
  division_supply`で、同一国家内の複数Divisionについても他Divisionの補給率map
  entryを誤って参照しないことを確認。
* 単体テスト`p26_005_real_path_result_is_deterministic_regardless_of_supply_map_
  insertion_order`で、`division_supply_fulfillment`マップへの挿入順序を入れ替えても
  結果がbit-for-bit一致することを確認。

## 25. Save V1互換性

Save format version(`SAVE_FORMAT_VERSION_V1 = 1`)・`SavedMilitaryRegistry`構造・
`division_supply_fulfillment`フィールド(P26-004で追加済み)は本タスクで一切変更して
いない。本タスクは既存の`supply_fulfillment_of`読み取りAPIを新たに戦闘へ接続した
だけであり、Save DTOへの変更は皆無(`git diff --stat`で`src/save/`配下に変更がない
ことを確認済み)。

## 26. save→load分岐

E2Eテスト`test_d1_partial_supply_combat_state_round_trips_and_continues_
identically`(補給率0.42の戦闘中save→load、load直後の値が保存前と一致し、その後の
戦闘結果が継続run[save/loadなし]と完全一致)・`test_d2_zero_supply_combat_state_
round_trips_through_save_load`(補給率0.0)・`test_d3_double_save_load_round_trip_
is_stable`(2回連続save→load)・`test_d4_old_save_without_fulfillment_field_
defaults_combat_to_full_supply`(補給率フィールド欠落の旧Saveをloadした
Divisionが、明示的に補給率1.0を指定した対照シナリオと完全に同じ戦闘結果になる)
で確認した。

## 27. 決定性

* 単体テスト`p26_005_real_path_result_is_deterministic_regardless_of_supply_map_
  insertion_order`(マップ挿入順序非依存)。
* E2Eテスト`test_d5_two_independent_app_instances_produce_identical_combat_
  results_with_partial_supply`(独立した2つのAppインスタンス、補給率0.5/0.8を
  含むシナリオでmanpower/organizationがbit-for-bit一致)。
* `cargo test`を独立した複数プロセスで実行(§29参照、`--lib`単体・
  `--test <name>`個別・`--all-targets`一括の3通り)し、いずれも同一件数・同一結果。

## 28. 戦闘下流への影響

manpower損失(§21/§22等の比較テストで確認)・equipment損失(既存の
`calculate_proportional_equipment_loss`が実際に適用されたmanpower casualty
[`apply_combat_casualty`]に比例して計算される既存経路をそのまま通るため、
readinessが下がって`manpower_loss`が小さくなれば、それに比例してequipment損失も
小さくなる――本タスクではこの経路自体を変更していない)・撃破可否/州支配権
(単体テスト`p26_005_severe_supply_shortage_prevents_an_attack_that_would_
otherwise_break_the_defender`: 既存テスト`test_combined_multiple_attackers_can_
break_well_defended_state`と全く同じ3個師団攻撃側の構成だが、深刻な補給不足(10%)を
与えると同じ30日以内では突破[AttackerWon]できなくなり、州の支配権も変化しない
ことを確認)。war score・annexation・peaceの式自体は本タスクの対象外であり
変更していないが、これらは全て`resolve_finished_battles`が生成する既存の
`BattleStatus`/manpower/organization変化を入力として使う既存の未変更経路であるため、
本タスクの変更は「入力(戦闘力)が正しく変わる」ことだけを保証すればよく、
下流の式自体を再検証する必要はない(spec §20で明示的に対象外)。

## 29. P26-004会計の保護

`src/military/daily_supply.rs`・`src/military/supply_initialization.rs`・
`src/economy/`配下は本タスクで一切変更していない(`git diff --stat`で確認)。
Oceanic 36か月測定(§23)で`shortage_days=418`・`first_shortage_day=32`がP26-004
報告書と完全一致したことが、日次消費・配分ロジック自体が無変更であることの
直接的な実測証拠である。P26-004のテスト群(`p26_004_daily_supply_consumption_
allocation_e2e_test`19件を含む)は全てそのまま合格した(§35)。

## 30. P26-004-PRE-005の保護

`src/military/supply_initialization.rs`(需要関数・初期31.0ブリッジ)は無変更。
関連E2Eテスト(`p26_004_pre_005_initial_supply_bridge_migration_e2e_test`22件)は
そのまま合格した。

## 31. P26-DET-001/REV-001の保護

`src/economy/production.rs`・`src/country/country_ai.rs`は無変更。関連E2Eテスト
(`p26_det_001_construction_ai_determinism_e2e_test`10件・`p26_det_001_rev_001_
cross_state_production_order_e2e_test`12件)はそのまま合格した。

## 32. Phase 23〜25保護

`cargo test --lib military::`(392 passed、P26-004時点の362 + 本タスクの新規単体
テスト30件[combat_calc.rs 24件+update.rs 6件])・
`cargo test --lib war::`(153 passed、P26-004時点と同数)・
`cargo test --lib economy::`(39 passed、同数)・
`cargo test --lib save::`(302 passed)を実行し、Phase 23(manpower)・Phase 24
(workforce)・Phase 25(装備会計・P25-007戦闘充足率)関連コードへの回帰がないことを
確認した。P25-007 E2Eテスト21件(`tests/p25_007_equipment_combat_effectiveness_
e2e_test.rs`)もそのまま全合格した。

## 33. 対象外

移動速度への補給反映・organization/organization回復・Attrition・士気・戦闘継続
時間の独自補正・補給不足による装備/manpower損耗・携行在庫・首都連結性・包囲・港・
海上/鉄道補給・交易・補給生産量の再調整・初期31.0の変更・AI募兵/補給判断・UI・
Save V2・戦争スコア/講和式の再設計・Phase 25装備会計変更、いずれにも触れていない。
`rg -n "process_combat|process_supply\b" src`で、これらが引き続き未登録・未接続の
ままであることを確認済み(§5参照)。

## 34. Critical/High/Medium/Low

* **Critical: 0件**
* **High: 0件**
* **Medium: 0件**
* **Low: 0件**

補給率フィールド欠落時に0除算を回避するための`.min(COMBAT_READINESS_SCALE)`clamp
(§7)は、数学的には`supply_fulfillment<=1.0`を事前検証しているため到達しないはずの
防御的コードであり、既存の`checked_*`スタイルと一貫しているため、findingとしては
計上していない。

## 35. 全テスト結果

```text
開始時baseline: 2234 passed / 0 failed / 1 ignored(自ら再測定、期待値と一致)
実装後最終:     2274 passed / 0 failed / 1 ignored(cargo test --all-targetsで実測)
```

新規テスト40件(`src/military/combat_calc.rs`単体24件+`src/military/update.rs`
単体6件+`tests/p26_005_supply_combat_strength_e2e_test.rs`10件)が新規に合格し、
2234+40=2274と算術的に完全一致する。既存テストの純減・改変は一切ない
(`land_war_combat_peace_test.rs`は無変更、他の既存テストファイルも無変更)。

個別実行結果(§23の検証コマンド、全て逐次実行):

```text
cargo test --lib military::           392 passed
cargo test --lib war::                153 passed
cargo test --lib economy::             39 passed
cargo test --lib save::               302 passed
cargo test --test p26_005_supply_combat_strength_e2e_test       10 passed
cargo test --test p26_004_daily_supply_consumption_allocation_e2e_test  19 passed
cargo test --test p26_004_pre_005_initial_supply_bridge_migration_e2e_test  22 passed
cargo test --test p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test  15 passed
cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test  45 passed
cargo test --test p26_003_military_supplies_production_e2e_test  49 passed
cargo test --test p26_003_rev_002_supply_production_units_e2e_test  56 passed
cargo test --test p26_003_rev_003_economic_power_capacity_e2e_test  21 passed
cargo test --test p26_det_001_construction_ai_determinism_e2e_test  10 passed
cargo test --test p26_det_001_rev_001_cross_state_production_order_e2e_test  12 passed
cargo test --test p25_007_equipment_combat_effectiveness_e2e_test  21 passed
cargo test --test land_war_combat_peace_test               4 passed
cargo test --all-targets              2274 passed / 0 failed / 1 ignored
```

## 36. clippy結果

`cargo clippy --all-targets --all-features -- -D warnings`は実装後**0 warnings**。

## 37. Cargo逐次実行

全てのCargoコマンドを逐次実行した(並列実行なし)。一過性のビルドエラーは発生
しなかった。

## 38. PNG差分の処理

`cargo test --all-targets`実行後、既知の非決定的PNG(headless UI描画テストが
再生成する)8ファイルに差分が生じた
(`verification_logs/p20-007/screenshots/03_playing_topbar.png`・
`p20-009/screenshots/04_playing_ja_jp.png`・`05_playing_en_us.png`・
`06_playing_ja_jp_again.png`・`phase-21/p21-save-002e/screenshots/01`〜`05`)。
これらは本タスクの変更と無関係(戦闘力計算はUI描画に触れない)であり、
`git status --short`で対象を個別に確認したうえで、該当8ファイルだけを
`git checkout -- <path>`で個別に復元した(ワークスペース全体のcheckoutは
行っていない)。

## 39. ユーザーセーブの保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージ・コミット
していない。開始時・全検証コマンド実行後・本報告書作成時点のいずれの
`git status --short`でも` M saves/savegame_v1.ron`のみが変化せず残り続けたことを
確認した(内容は一度も参照していない)。テスト用の合成Save・合成App状態は全て
一時ディレクトリ(`std::env::temp_dir()`配下)または実App内メモリ上に作成し、
実ユーザーセーブへは一切アクセスしていない。

## 40. 変更ファイル

```text
src/military/combat_calc.rs   (calculate_combat_readiness_with_supply/
                                resolve_combat_day_with_supply/
                                effective_power_1000_with_supply/
                                resolve_combat_day_multi_with_supply新設、
                                既存関数を互換wrapper化、CombatReadiness構造体へ
                                supply_readiness_unitsフィールド追加、
                                CombatReadinessErrorへ補給関連の2エラー追加、
                                単体テスト24件追加)
src/military/update.rs        (process_daily_battlesのpreflight検証・
                                resolve_combat_day_multi呼び出しを補給率対応版へ
                                差し替え、単体テスト6件追加)
tests/p26_005_supply_combat_strength_e2e_test.rs  (新規、E2Eテスト10件)
verification_logs/phase-26/p26-005/p26-005_supply_combat_strength.md  (本報告書)
```

`saves/savegame_v1.ron`はコミット対象に含めない。PNGファイルは§38のとおり
個別復元済みでコミット対象に含めない。

## 41. 最終`git status --short`

```text
 M saves/savegame_v1.ron
 M src/military/combat_calc.rs
 M src/military/update.rs
?? tests/p26_005_supply_combat_strength_e2e_test.rs
?? verification_logs/phase-26/p26-005/
```

## 42. P26-006へ進める状態か

進める状態である。補給充足率は実際に日次処理から確定・保存され、
`MilitaryRegistry::supply_fulfillment_of`経由で戦闘力計算(manpower/equipment/
補給の3要素min)へ実際に接続された。P26-004が実測したOceanicの供給不足は、
本タスクにより実際のゲームプレイ上の戦闘力低下として確認可能になった
(§23)。移動速度・organization・Attrition・携行在庫・首都連結性・包囲・海上/
鉄道補給・交易・AI補給判断・UI表示への接続は一切行っていない(意図どおり
本タスクのスコープ外のまま)。
