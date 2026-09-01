# P26-003: MilitarySupplies生産配線の実装

## 1. 総合判定

`COMPLETE`

## 2. 開始時HEAD

```text
973445c30d920c80908635f22acc45ffb5655dac
```

開始直後の記録:

```text
git rev-parse HEAD
  → 973445c30d920c80908635f22acc45ffb5655dac

git status --short
  →  M saves/savegame_v1.ron

git log --oneline --decorate -15
  973445c (HEAD -> main) fix: add military supplies localization
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
  b568779 docs: complete phase 24 final audit
  1b4e8e3 fix: use fixed-point workforce ratio arithmetic
```

## 3. 実装対象コードHEAD

`973445c30d920c80908635f22acc45ffb5655dac`(実装開始直前の時点。本タスクの変更はこの上に積む)。

## 4. P26-001〜P26-002-REV-001の扱い

3件の既存報告書をすべて読み、無変更のまま保持した(`git diff --stat`で該当パス差分なしを確認済み)。

* `verification_logs/phase-26/p26-001/`: 単一抽象資源`MilitarySupplies`・耐久装備との型/意味分離・国家備蓄方式・Division携行在庫なし・Save V1維持・migrationなし・P26-003は生産配線のみ・dead code`military::supply`不使用、を確定設計として踏襲。
* `verification_logs/phase-26/p26-002/`: fixed-point会計基盤(`MILITARY_SUPPLIES_SCALE=1,000,000`、`MILITARY_SUPPLIES_MAX_UNITS=10^15`暫定値、`add_military_supplies_produced`/`try_consume_military_supplies`)をそのまま利用。
* `verification_logs/phase-26/p26-002-rev-001/`: ローカライズ欠落訂正の経緯を確認。本タスクでは新規`BuildingType`variantを追加しない設計を採用したため、同種のローカライズ欠落を再発させない(§33参照)。

## 5. 現行建物モデル

`src/building/data.rs`の`BuildingType`は10 variant(Farm/LoggingCamp/Mine/Factory/MilitaryFactory/Railway/University/MagicAcademy/CrystalMine/CrystalRefinery)。`BuildingDefinition`は`assets/data/buildings.ron`からRONロードされる完全にデータ駆動の構造体で、`input_resources: HashMap<ResourceType, f64>`・`output_resources: HashMap<ResourceType, f64>`を持つ。重要な発見: **`output_resources`は既に複数の`ResourceType`を同時に持てるHashMapであり、1建物が複数資源を併産する仕組みは新規実装不要で既に存在していた**(例: 将来Farmが将来的にFood以外も出す場合でも、コード変更なくRONデータへ1行追加するだけで済む設計)。建物レベルは`StateData.buildings: HashMap<BuildingType, u32>`(Save対象)。建設は`ConstructionQueueItem`経由でプレイヤー/AI(`process_construction_ai`)どちらも同じ`BuildingRegistry`定義を参照する。

## 6. 現行生産モデル

`src/economy/production.rs::process_country_production`が国家単位で月次生産・消費をまとめて処理する。手順: (1)全州の入力資源需要を集計 (2)入力資源充足率を計算 (3)州・建物ごとに`operation_ratio = min(workforce_ratio, logistics_ratio, input_resource_ratio) × (bankrupt ? 0.8 : 1.0)`を決定し、消費→生産を実行。`MilitaryEquipment`だけは既にP25-002-REV-001でfixed-point専用APIへ分岐済み(生の`stockpile.add`ではなく`add_military_equipment_produced`)。呼び出し元は`src/economy/mod.rs::handle_monthly_economy`のみ(`tests/economy_tests.rs`等のテストからの直接呼び出しを除く)。国家(`CountryRegistry.countries`)・州(`StateRegistry.states`)は共に`Vec`(HashMapではない)であり、外側のイテレーション順序は決定的。

## 7. 生産周期

月次(`MonthChangedMessage`駆動、`DailySimulationSet::Economy`)。日次では建設進捗のみ進み(`handle_daily_construction`)、資源生産は行われない。

## 8. 生産源候補比較

### 案A: 新規専用建物`SupplyDepot`(P26-001推奨案) — 却下

実コード監査の結果、次の理由で採用を見送った(§10で詳述)。

1. 新規`BuildingType`variantは、旧Save・New Gameの双方でレベル0からしか開始できない(既存データから再構築できる根拠がないため、P26-001設計監査自身の禁止事項に抵触せず追加するには初期データへの新規付与が必要だが、それを裏付ける既存データがない)。
2. **決定的な問題**: `src/country/country_ai.rs::process_construction_ai`のAI建設優先度ロジックは`Farm → Mine → Factory(民需工場)`の3種類しか対象にしておらず、`MilitaryFactory`ですら一度もAIの建設対象になっていない(既存の事実、本タスクが作った制約ではない)。したがって新規`SupplyDepot`もAIには一切建設されず、全6か国(プレイヤー含む)が恒久的にレベル0のままになる。これはspec §6・§18が明示的に禁止する「全AI国家が恒久的にMilitarySupplies=0となる設計」に該当し、P26-004/P26-005で消費が接続された瞬間に全軍が補給0になる構造的な罠となる。

### 案B: 既存`MilitaryFactory`がMilitaryEquipmentと併産 — 採用

`output_resources`が既にHashMapであるため、コード変更を伴わずRONデータへ1行追加するだけで実現できる。既存マップの5/6か国が既にMilitaryFactoryを実レベルで所有しており(New Game・旧Saveとも)、新規建物の建設待ちなしに即座に生産が始まる。詳細な採用理由は§10。

### 案C: 既存の別建物・logistics能力の再利用

実コードを監査したが、`MilitarySupplies`の意味(弾薬・燃料・軍用食料等)に適合する既存の未使用建物・未使用出力枠は見つからなかった(案Bで十分に要件を満たすため、無理な流用を避けた)。

### 案D: 国家基礎生産(建物非依存)

原則却下方針どおり不採用。案Bで具体的な生産源が確保できるため、建物・労働力から独立した生成は不要と判断した。

## 9. 採用した生産源

**既存`MilitaryFactory`**(新規建物`SupplyDepot`は追加しない)。`assets/data/buildings.ron`のMilitaryFactory定義の`output_resources`へ`MilitarySupplies: 100.0`を追加した(既存の`MilitaryEquipment: 20.0`はそのまま維持)。

## 10. 専用建物を採用または却下した理由

§8で述べた理由を実データで裏付けた:

* `assets/data/states.ron`実測: 全28州のうち7州がMilitaryFactoryを所有(owner_country_id: 0, 0, 1, 2, 4, 4, 5の7か国分布、6か国中5か国が最低1つ保有)。
* `process_construction_ai`(`src/country/country_ai.rs:276-373`)を実際に読み、AI建設優先度が`Food不足→Farm`「`Wood/Iron/Coal`不足→Mine」「それ以外→Factory(民需)」の3分岐のみで、`MilitaryFactory`を含む一切の軍事施設をAIが自律建設する経路が存在しないことを確認した(既存の事実)。
* この制約下で新規`BuildingType`を追加すると、既存の`MilitaryFactory`保有国と異なり「新規建物を持つ国が0か国」という状態からスタートすることになり、AIには一切到達不能な恒久ゼロ状態を新規に作り出す。
* 一方、既存`MilitaryFactory`への併産は、既に建物を保有している国(プレイヤー・AI問わず)へ即座に生産能力を与える。実App経由でCountryId(2)(Dwarf Federation、AI国家)が実際に非ゼロのMilitarySupplies生産に到達することを`test_h2`で確認した(§14)。

## 11. Save V1構文互換性

新規`BuildingType`variantを追加しないため、`StateData.buildings: HashMap<BuildingType, u32>`のシリアライズ形式は無変更。`assets/data/buildings.ron`はSaveデータではなく起動時ロードされる静的アセットファイルであり、Save互換性の対象外。したがって構文互換性は自明に成立する(新規Save関連コード変更ゼロ)。

## 12. Save V1意味的互換性

`test_g1`〜`test_g4`(実App、実save/load経路)で検証:

* `test_g1_save_load_round_trip_preserves_real_production_result`: 3ヶ月分の実生産を経た後のsave→loadでMilitarySupplies備蓄値が完全一致。
* `test_g2_double_round_trip_is_stable`: 2回連続save/loadでも変化なし。
* `test_g3_production_after_load_continues_from_the_loaded_value_without_double_counting`: load後にさらに生産を進めても、二重加算が起きず単調増加のみ。
* `test_g4_old_v1_shaped_load_without_military_supplies_key_still_produces_normally`: MilitarySuppliesキーへ一切触れずにsave/loadした後でも(旧V1相当)、その後の実生産が正常に0から開始・増加する。

## 13. New Game／Loaded Game比較

`test_h1_new_game_and_loaded_game_agree_after_identical_ticks`で、同じ月数分のティックを進めた「継続プレイ」と「途中でsave→load」の2経路が、同一のMilitarySupplies備蓄値に到達することを確認した。

## 14. AI国家への影響

* `test_h2_ai_country_produces_through_the_same_path_and_is_not_permanently_zero`: CountryId(0)をプレイヤーに設定し、非プレイヤー国家CountryId(2)(Dwarf Federation)が実App経由で実際に非ゼロのMilitarySupplies生産へ到達することを確認した。CountryId(2)は`assets/data/states.ron`+`assets/data/resources.ron`の実データ監査により、Mine(discovered済みIron/Coal鉱床)→Factory(IndustrialGoods)→MilitaryFactory(Equipment/Supplies併産)という完結した供給網を持つ国家であることを確認済み(唯一ではない可能性があるが、少なくとも1か国の実在を実証した)。
* `test_h3_country_without_any_military_factory_mirrors_the_pre_existing_equipment_gap`: CountryId(3)(Oceanic Magic Empire)は`assets/data/states.ron`実測によりMilitaryFactoryを1つも所有していないため、MilitarySuppliesが恒久的に0となる。ただしMilitaryEquipmentも全く同じ理由でPhase 25から既に0のままであり(実App経由で確認)、本タスクが新規に作り出した非対称性ではなく、既存の非対称性をそのまま継承しただけであることを明示的に確認した。
* 実データ監査中に、CountryId(1)(Elfin Republic)がMilitaryFactoryを1つ所有しているにもかかわらず、開始時Iron備蓄が0かつIron生産源(discovered済み鉱床を持つMine)を1つも所有していないため、MilitaryEquipment・MilitarySupplies双方が恒久的に生産不能であることも判明した(`assets/data/countries.ron`の初期備蓄・`assets/data/resources.ron`の鉱床データ実測)。これもPhase 25から既に存在するMilitaryEquipment生産上の制約であり、P26-003が新規に作った制約ではない。

以上により、「全AI国家が恒久的にMilitarySupplies=0となる設計ではない」という必須条件(spec §18)を、少なくとも1か国の反例(CountryId(2))を実際に実証することで満たしている。

## 15. 月次生産と将来の日次消費の関係

生産は月次(`handle_monthly_economy`内)、P26-004以降で導入される消費は日次(`handle_daily_military`内、既存設計どおり)を想定。本タスク時点では消費経路が一切存在しないため、循環依存は発生しない(`test_i3`・`test_f4`で備蓄が単調非減少であることを確認、消費が介在していないことの実証)。P26-004設計時には、月初にまとめて生産された備蓄を月内の日次で取り崩していく非対称な時間粒度(生産:月次、消費:日次)の扱いを別途決定する必要があるが、これはP26-004の範囲であり本タスクでは扱わない。

## 16. 初期備蓄0の評価

初期備蓄は追加していない。New Game・旧Saveロードとも、既存のMilitaryEquipmentと全く同じ意味論(未登録キー→0.0)でMilitarySuppliesも0から開始する(`test_h_new_game_starts_at_zero_for_all_countries`はP26-002の既存テストとして無変更のまま維持されており、本タスクの変更後も引き続き合格することを確認済み)。

## 17. 生産式

既存の`calculate_actual_output(base_output, level, operation_ratio) = base_output × level × operation_ratio`をそのまま再利用する(新しい生産式コードは追加していない)。`MilitaryEquipment`と全く同じ`final_op`を共有するため、両資源は常に同じ稼働率で比例して生産される(`test_b5`・`test_f3`で実測、比率は常にbase_output比=100.0/20.0=5.0倍で一定)。

## 18. level当たり生産量

`MilitaryFactory`のMilitarySupplies基礎生産量として`100.0/レベル/月`を採用した。根拠(`DivisionDefinition.supply_usage`: Infantry=1.0/Artillery=3.0/Mage=2.0を日次需要のベースラインとして使用、strength_factor=1.0・activity_multiplier=1.0の完全編制・待機状態を仮定):

| 兵科 | 日次需要(ベースライン) | 30日換算 | 31日換算 | 支えられる個数(30日) |
| --- | --- | --- | --- | --- |
| Infantry | 1.0 | 30.0 | 31.0 | 100.0/30.0 ≈ 3.33個師団 |
| Artillery | 3.0 | 90.0 | 93.0 | 100.0/90.0 ≈ 1.11個師団 |
| Mage | 2.0 | 60.0 | 62.0 | 100.0/60.0 ≈ 1.67個師団 |

365日平均月(30.44日)換算では、1レベルあたり継続的に約3.29個のInfantry師団を支える計算になる。これらの`strength_factor`/`activity_multiplier`の正確な式はP26-004で確定するため、本タスクの値は内部整合性(単位系の一貫性)を優先した暫定値であり、P26-004でのバランス再検討を妨げない(コメントに明記済み)。

## 19. 生産量のゲーム上の意味

MilitaryFactoryが「装備(耐久ストック)」と「補給物資(消費フロー)」を同一施設・同一稼働率で併産するという設計は、両者が同じ軍需産業基盤から生まれるという直感と整合する。将来的にAI・プレイヤーが両資源の生産配分を選べない(現状どちらの資源にも配分UIが存在しない)限り、この「自動併産」はゲーム上妥当な簡略化である(P26-001§16で既に評価済みの方針)。

## 20. operation ratio

`calculate_operation_ratio`は無変更。MilitarySuppliesもMilitaryEquipmentと全く同じ`operation_ratio`(workforce/logistics/入力資源充足率の最小値、破産時×0.8)を使用する(`test_b1`〜`test_b6`で検証)。

## 21. 丸め方式

生産量計算そのもの(`calculate_actual_output`)には丸めを追加していない(既存どおり連続値`f64`)。最終的な備蓄への反映は、P26-002で既に確立済みの`CountryStockpile::add_military_supplies_produced`が内部で行う量子化(`round()`、四捨五入)をそのまま使用する。spec §9はfloorを推奨候補として挙げているが、P26-002の既存API(round-to-nearest)を変更するとP26-002の全既存テスト・往復変換保証を破壊するおそれがあるため、既存の確立済みAPIとの一貫性を優先しround-to-nearestのまま踏襲した(P26-002自身がP25-002-REV-001のround-to-nearest方式を踏襲した判断と同じ理由)。境界値については`test_c3`(量子化境界ちょうどの値)・`test_c6`〜`test_c8`(MAX_UNITS境界)で検証済み。

## 22. fixed-point会計API接続

`CountryStockpile::add_military_supplies_produced`を、国家ごとに月1回だけ呼び出す(§23参照)。生の`stockpile.add(ResourceType::MilitarySupplies, ...)`は一度も使用していない(`rg -n "MilitarySupplies" src`で確認、§29参照)。

## 23. 国家単位原子性

`process_country_production`内に`military_supplies_total_value: f64`という国家スコープの一時集計変数を追加した。全州・全建物の走査が終わるまでMilitarySuppliesはこの一時変数へのみ加算され(`CountryStockpile`は未変更)、走査完了後に**ただ1回だけ**`add_military_supplies_produced`を呼ぶ。これにより、1国家が複数州・複数MilitaryFactoryを持つ場合でも、部分的な適用(一部の州分だけ反映される)が起きない(`test_d4`で2州構成にて検証)。失敗時(値域超過等)は`CountryStockpile`全体が不変のまま`warn!`ログを出す(`Result`を無視しない、`let _ = ...`は使用していない、spec §12の明示的禁止に対応)。

## 24. 世界保存式

```text
ΣMilitarySupplies_after = ΣMilitarySupplies_before + Σactual_production
```

`test_e1`(単一tick)・`test_e2`(複数tick累積)・`test_e3`(複数国家の世界合計=各国生産の総和)で検証。消費経路が存在しないため、`- Σconsumed`項は本タスクでは常に0。

## 25. 複数国家分離

`test_a5`(独立生産)・`test_d3`(片方の国家がoverflowしても他方へ影響しない)で検証。`process_country_production`は国家ごとに独立した`&mut CountryStockpile`を受け取るため、構造的に国家間の混入は不可能。

## 26. 所有権・支配権

既存の`owner_country_id`ベースの所有権規則(`handle_monthly_economy`の`owned_states`フィルタ)をそのまま継承する。P26-003ではこのフィルタリング自体には一切変更を加えていないため、州所有権変更・占領時の生産帰属はMilitaryEquipment・Foodなど既存の全資源と完全に同じ挙動を示す(`git diff`で`handle_monthly_economy`の所有権判定部分に差分がないことを確認済み)。

## 27. 消滅国家

既存の`if owned_states.is_empty() { continue; }`(`handle_monthly_economy`、無変更)により、所有州0件の国家(実質的に消滅した国家)は`process_country_production`自体が呼ばれない。`process_country_production`自身も空スライスを渡された場合に安全にno-opすることを`test_a7`で確認した(関数レベルでの防御的検証)。

## 28. 他ResourceTypeへの非影響

`test_j2_food_and_other_resources_unaffected_by_the_militarysupplies_addition`でFood・MilitaryEquipmentへの非影響を確認。加えて、Phase 25の全E2Eテスト(8ファイル、§36)とPhase 23/24の全E2Eテスト(8ファイル、§37)が無変更のまま合格することでも間接的に確認している。

## 29. MilitaryEquipment生産の維持

`test_j1_military_equipment_production_formula_is_unchanged`(P25-002-REV-001のTest Lと同一の入力・期待値)、`test_j4_real_app_equipment_stockpile_still_grows_via_the_unmodified_path`(実App経由)で確認。`src/military/equipment.rs`・`military::equipment`配下は1バイトも変更していない。

## 30. MAX_UNITS再監査

P26-002のLow finding(暫定値の再監査要)を本タスクで解消した。判定: **`RESOLVED`**。

実際の生産式(MilitaryFactory併産、100.0/レベル/月、最大レベル10)が確定したため、根拠のある理論最大値を算出できるようになった:

* 現行州数28・全州MilitaryFactory最大レベル・稼働率100%・150年間無消費という非現実的な最悪ケース: `28 × 10 × 100.0 × 1800か月 = 50,400,000`(value換算)。ドメイン上限(equipment値換算で10億)の約19.8倍の余裕(`test_max_units_1x_scale_150_year_theoretical_max_has_positive_margin`で純粋計算により検証)。
* 州数が現行の10倍(280)へ拡張した仮想シナリオ: `280 × 10 × 100.0 × 1800か月 = 504,000,000`。この場合でもドメイン上限を下回り、約1.98倍の正の余裕を維持する(`test_max_units_10x_scale_150_year_theoretical_max_still_has_positive_margin`で検証)。

さらに、MilitarySuppliesはP26-004以降、装備と異なり毎日恒常的に消費される設計が想定されているため、実プレイでこの無消費理論値へ近づくことは現実的にない。到達時は`OutOfRange`を明示的に返す(§9で確認済みのclamp・wraparound禁止は無変更)。この監査結果を`src/economy/resources.rs`の`MILITARY_SUPPLIES_MAX_UNITS`ドキュメントコメントへ反映した(値自体は変更せず、根拠だけを暫定値から実測値へ更新)。

## 31. 150年値域

§30参照。1x規模で約19.8倍、10x規模でも約1.98倍の正の余裕。

## 32. Long-term安全余裕

同上。`checked_add`によるu64 wraparound防御(`ArithmeticOverflow`)と、ドメイン上限超過(`OutOfRange`)は意味的に区別されたまま(P26-002から無変更)。

## 33. UI・ローカライズ

**変更なし。** 新規`BuildingType`variantを追加しない設計を採用したため、新しいローカライズキーは一切不要であり、P26-002-REV-001で発生したような欠落を構造的に再発させない。既存の`resource.military_supplies`キー(P26-002-REV-001で追加済み)は既にUIの`ResourceType::ALL`表示ループで正しく解決される。`cargo test --test p20_009_localization_headless_render_test`・`p20_009_localization_resource_test`が両方とも無変更で合格することを確認済み(§40)。

## 34. 本番System順序

`handle_monthly_economy`内(`DailySimulationSet::Economy`)の既存呼び出し順序を一切変更していない。新しいSystem・新しいSystemSet順序は追加していない(`process_country_production`の呼び出し位置・引数は無変更、関数内部の実装だけを変更した)。

## 35. P26-004以降を先行実装していないこと

Division需要・日次配分・`try_consume_military_supplies`の呼び出し・戦闘力補正・移動速度・organization回復・Attrition・携行在庫・包囲・首都連結性・港・海上輸送・AI戦略・詳細UI・Save V2は一切実装していない。`rg -n "try_consume_military_supplies" src`は0件(ヒットなし、消費APIはP26-002で定義されたまま未使用)。`test_i1`〜`test_i3`で非接続性を実App経由で確認。

## 36. Phase 25会計の保護

P25関連E2Eテスト8ファイルを個別に逐次実行し、全て無変更・同一件数で合格した:

| テストファイル | 結果 |
| --- | --- |
| p25_002_equipment_accounting_foundation_e2e_test | 15 passed |
| p25_002_rev_001_fixed_point_equipment_accounting_e2e_test | 6 passed |
| p25_003_recruitment_equipment_issue_e2e_test | 22 passed |
| p25_004_combat_equipment_loss_e2e_test | 5 passed |
| p25_005_daily_equipment_reinforcement_e2e_test | 10 passed |
| p25_006_demobilization_equipment_return_e2e_test | 29 passed |
| p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test | 8 passed |
| p25_007_equipment_combat_effectiveness_e2e_test | 21 passed |

全て0 failed。

## 37. Phase 23・24会計の保護

対象8ファイルを個別に逐次実行し、全て合格した:

| テストファイル | 結果 |
| --- | --- |
| p23_002_mobilized_manpower_workforce_e2e_test | 8 passed |
| p23_004_economic_workforce_ratio_e2e_test | 7 passed |
| p23_006_manpower_ledger_e2e_test | 22 passed |
| p23_007_division_demobilization_e2e_test | 24 passed |
| p23_008_daily_reinforcement_e2e_test | 21 passed |
| p23_009_ai_recruitment_e2e_test | 22 passed |
| p23_010_phase23_manpower_lifecycle_audit_test | 9 passed |
| p24_002_workforce_rounding_unification_e2e_test | 17 passed |

全て0 failed。加えて`p21_014_country_power_e2e_test`(9 passed)も個別実行し、§38で述べる副作用がハードコードされたテスト値を破壊していないことを確認した。

## 38. Critical／High／Medium／Low

```text
Critical: 0
High:     0
Medium:   0
Low:      1
```

**Low finding**: `src/country/power.rs::compute_state_aggregates`が国家の「経済力」スコア(国力Tier判定の一要素、P21-014)を`建物定義のoutput_resources値合計 × レベル`という汎用式で算出しており、これは元々`MilitaryEquipment`の20.0を既に含んでいた計算だが、MilitarySupplies(100.0)の追加によりMilitaryFactoryの経済力寄与が20.0→120.0(6倍)へ増加する。これは`compute_state_aggregates`自身のdoc comment(「出力資源ごとの基礎産出量合計 × レベル」)どおりの意図された汎用計算であり、CrystalRefineryのMagicCrystal:10.0が既に同じ式へ寄与しているのと同じ扱いである。`p21_014_country_power_e2e_test`(9 passed、ハードコードされた期待Tier分布を含む)が本タスクの変更後も無変更で合格することを確認済みのため、既存のテスト保護観点では非ブロッキングと判断した。ただし「経済力」の意味合いが変化した(MilitarySupplies生産能力も国力スコアへ加算されるようになった)ことは事実であり、将来国力バランスを調整する際に考慮すべき点として記録する。

## 39. 変更したファイル

```text
assets/data/buildings.ron                                          (MilitaryFactoryのoutput_resourcesへ1行追加)
src/economy/production.rs                                          (国家単位原子的commitロジック追加)
src/economy/resources.rs                                           (MILITARY_SUPPLIES_MAX_UNITSのドキュメントコメント更新、値は無変更)
tests/p26_003_military_supplies_production_e2e_test.rs             (新規、49テスト)
verification_logs/phase-26/p26-003/p26-003_military_supplies_production.md  (本報告書)
```

## 40. 実行したコマンドと正確な結果

全てのCargoコマンドを逐次実行した(並列実行なし)。

```text
$ cargo build --lib
  → 成功(警告なし)

$ cargo test --lib economy::
  → ok. 32 passed; 0 failed; 0 ignored

$ cargo test --lib military::
  → ok. 283 passed; 0 failed; 0 ignored

$ cargo test --lib save::
  → ok. 300 passed; 0 failed; 0 ignored

$ cargo test --test p21_014_country_power_e2e_test
  → ok. 9 passed; 0 failed; 0 ignored

$ cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test
  → ok. 45 passed; 0 failed; 0 ignored

$ cargo test --test p26_003_military_supplies_production_e2e_test
  → ok. 49 passed; 0 failed; 0 ignored

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

$ cargo test --all-targets  (2回実行、テストファイル完成後の最終確認含む)
  → 全バイナリ合計: 1947 passed; 0 failed; 1 ignored

$ cargo clippy --all-targets --all-features -- -D warnings
  → 警告0、正常終了
```

`cargo clean`は実行していない(キャッシュ破損は発生しなかった)。

## 41. 全テストpassed／failed／ignored件数

```text
開始時: 1898 passed / 0 failed / 1 ignored
最終:   1947 passed / 0 failed / 1 ignored
```

新規テスト49件すべてが合格し、既存テストの回帰は0件(1898 + 49 = 1947、実測ログから全バイナリの`test result:`行を合算して算出)。

## 42. clippy警告数

```text
0
```

## 43. Cargo逐次実行

全Cargoコマンドを1つずつ順番に実行し、並列実行は一切行っていない。

## 44. PNG差分の処理

`cargo test --test p20_009_localization_headless_render_test`・`cargo test --all-targets`の実行により、既知の非決定的PNG(最大9枚)が都度再生成された。いずれも既知の対象ファイルであることを確認したうえで、個別パス指定の`git checkout --`で復元した(広範なcheckoutは未使用)。最終的にコミット対象PNGは0件。

## 45. ユーザーセーブの保護

`saves/savegame_v1.ron`は全工程を通じて未読み書き・未復元・未ステージ・未コミット。開始時・終了時とも既存の`M`差分のみ。

## 46. 最終`git status --short`

コミット前(本報告書作成完了直後)の状態:

```text
 M assets/data/buildings.ron
 M saves/savegame_v1.ron
 M src/economy/production.rs
 M src/economy/resources.rs
?? tests/p26_003_military_supplies_production_e2e_test.rs
```

(本報告書ファイル自体もこの後`??`として追加される想定。コミット後の状態は最終報告に記載する。)

## 47. P26-004へ進める状態か

進める状態である。MilitarySuppliesは実際に月次生産され国家備蓄へ蓄積されるようになったが、Division需要・日次配分・消費・戦闘力補正はまだ一切接続されていない(意図どおりの会計基盤+生産配線のみの状態)。P26-004(Division需要+日次配分)に着手する際は、本タスクの§15で記録した「生産:月次／消費:日次という時間粒度の非対称性」の扱いを最初に決定する必要がある。またMAX_UNITSはRESOLVEDと判定したが、P26-004で日次配分・按分ロジックが入ることで実際の消費速度が明らかになるため、消費側の実装が固まった時点で再度整合性を確認することが望ましい。
