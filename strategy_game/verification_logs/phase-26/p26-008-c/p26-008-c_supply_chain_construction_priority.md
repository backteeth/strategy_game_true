# P26-008-C: 補給生産鎖を修復するAI建設優先順位拡張

## 1. 総合判定

```
P26-008-C COMPLETE
FOOD EMERGENCY PRIORITY PRESERVED
PROACTIVE MINE PRIORITY PRESERVED
INDUSTRIAL GOODS GAP DETECTION VALID
PROACTIVE FACTORY CONSTRUCTION VALID
DUPLICATE FACTORY PREVENTION VALID
STAGED SUPPLY CHAIN REPAIR VALID
DETERMINISM PRESERVED
SAVE CONTINUITY PRESERVED
SAVE V1 PRESERVED
AI CADENCE PRESERVED
P26-008 COMPLETE
NEXT ATOMIC TASK READY
```

## 2. 開始HEAD

`01d9139`(完全形: `01d9139af309164df70cdeebb99befc97ae97ab3`、`docs: record final commit hash in p26-008-b report`)。開始時`git status --short`はクリーン。開始時テスト基準を実測: `cargo test --all-targets` → **2476 passed / 0 failed / 1 ignored**。`cargo clippy --all-targets --all-features -- -D warnings` → **0 warnings**。

## 3. 実際の建設AI入口

`src/country/country_ai.rs::process_construction_ai`。呼び出し元は国家AI統合Systemの月次評価分岐(`is_monthly_due = current_day % 30 == country_id.0 % 30 || ai_state.dirty`)で、P26-004-PRE-002確立・P26-008-PRE-002が復元したイベント駆動cadenceのまま。本タスクはこの呼び出し経路・cadence判定コードを一切変更していない。

## 4. AI評価cadence

無変更。月次(`day % 30 == country_id % 30`または`dirty`)。`mark_ai_registries_dirty_daily`は復活させていない。`AiChangeSnapshot`の意味も変更していない。

## 5. 変更前の建設優先順位

```
(1) Food緊急(Farm) — 既存仕様のまま
(2) 保有建物が要求する鉱床資源の事前開発(Mine) — P26-004-PRE-002
(3) 既存の通常判断(原材料不足→Mine、それ以外→Factory)
```

## 6. 変更後の建設優先順位

```
(1) Food緊急(Farm) — 無変更
(2) 保有建物が要求する入力資源のうち、構造的な生産経路が存在しないものの
    事前開発:
      鉱床由来(Iron/Coal/Wood等) → Mine(無変更)
      IndustrialGoods            → Factory(新規、P26-008-C)
    `ResourceType::ALL`宣言順(Food,Wood,Iron,Coal,MagicCrystal,
    IndustrialGoods,...)でtie-breakする点も無変更のまま拡張。
(3) 既存の通常判断 — 無変更
```

新しい優先順位階層は追加していない。既存の優先度2の内部だけを汎用化した。

## 7. Food緊急の保護

無変更。単体テスト`food_emergency_takes_priority_over_the_industrial_goods_gap`(`src/country/country_ai.rs`)、E2E `test_02_food_emergency_still_takes_priority_over_the_gap`で、IndustrialGoods構造的欠損が同時に存在してもFood緊急(Farm)が優先されることを確認した。

## 8. 既存Mine事前開発の保護

`has_active_mine_for_resource`・`find_undeveloped_deposit_state`は無変更のまま再利用。単体テスト`deposit_resource_priority_order_is_preserved_ahead_of_industrial_goods`で、Iron鉱床の事前開発がIndustrialGoods(Factory)より`ResourceType::ALL`順で先に評価されることを確認。既存の`p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs`(11件、36か月snapshotを含む)が全て無変更のまま通過することを確認した(§37参照)。

## 9. IndustrialGoods不足の定義

```
1. 生存国家が所有する建物(level>0)の少なくとも1つがIndustrialGoodsを
   入力として要求する(既存の`resource_types_required_by_owned_buildings`を
   そのまま再利用、特定CountryId・特定建物種別のハードコードなし)
2. その国家の所有する有効な州のいずれにもFactory(level>0)が存在しない
```

両方を満たす場合のみ、新規`has_active_factory_for_industrial_goods`が`false`を返し、事前開発対象になる。

## 10. Factory存在判定

```rust
fn has_active_factory_for_industrial_goods(
    country: &CountryData,
    state_registry: &StateRegistry,
    owned_states: &[StateId],
) -> bool {
    let already_built = owned_states.iter().any(|&sid| {
        state_registry.get(sid).is_some_and(|s| s.building_level(BuildingType::Factory) > 0)
    });
    if already_built { return true; }
    country.construction_queue.iter().any(|item| item.building_type == BuildingType::Factory)
}
```

構造的存在(level>0または建設queue内)だけを見る、当月の実生産量とは無関係な判定。`find_undeveloped_deposit_state`と対になる、Mine版の`has_active_mine_for_resource`と同じ設計方針。

## 11. 当月生産0の扱い

E2E `test_05_zero_monthly_output_does_not_cause_a_duplicate_factory`で確認: Coal入力を0にしてFactoryの当月実生産を0にしても、Factory自体(level>0)は存在するため`has_active_factory_for_industrial_goods`は`true`を返し、重複建設されない。

## 12. Factory建設中の扱い

`has_active_factory_for_industrial_goods`は完成済み(level>0)に加え、`country.construction_queue`内の未完成Factoryも「対応済み」とみなす。単体テスト`duplicate_factory_is_not_queued_while_one_is_already_under_construction`、E2E `test_06_no_duplicate_factory_rule_reactivation_while_one_is_under_construction`で、建設中のFactoryがある間はこの規則(`SteelShortageSteelMillPriority`)が再発火しないことを確認した。

## 13. Factory建設州

既存の汎用候補選択API`find_candidate_state`をFactory向けにそのまま再利用(Mine専用の鉱床チェック分岐は`target_building == BuildingType::Mine`でガードされているため、Factoryには適用されない)。新しい生産グラフ探索・新しい州選択ロジックは実装していない。

## 14. StateId tie-break

`find_candidate_state`の既存実装(`owned_states`をStateId昇順で走査し最初に条件を満たす州を返す)をそのまま利用。単体テスト`factory_candidate_state_excludes_other_countries_and_invalid_states`、E2E `test_13_result_does_not_depend_on_state_registration_order`で、他国州の除外・登録順反転での結果不変を確認した。

## 15. ResourceType tie-break

`find_proactive_development_target`(旧`find_proactive_mine_state`を汎用化)は`ResourceType::ALL`宣言順(Food,Wood,Iron,Coal,MagicCrystal,IndustrialGoods,MilitaryEquipment,RawMagicCrystal,MilitarySupplies)でループし、IndustrialGoodsだけを特別分岐として扱う。MagicCrystalは既存どおり除外。

## 16. Factory候補なしの挙動

単体テスト`no_candidate_state_falls_through_without_panicking`で、候補州がない場合にpanicせず既存の通常判断(優先度3)へ安全にフォールスルーすることを確認した。

## 17. 資金不足の挙動

`find_proactive_development_target`が候補(Mine/Factory)を発見した**後**に、その建物固有の建設費と`country.treasury`を比較する(既存のMine単独ゲート`country.treasury >= mine_def.construction_cost`を、対象建物ごとの費用チェックへ一般化)。不足時はこの規則を発火させず優先度3へフォールスルーする。外生的な資金付与は行っていない。単体テスト`insufficient_funds_for_factory_falls_through_to_priority_three`、E2E `test_10_insufficient_funds_preserves_existing_outcome`で確認。

## 18. MilitaryFactoryなしの挙動

IndustrialGoodsを要求する所有建物(MilitaryFactory等)が存在しなければ、この規則だけを理由にFactoryを建設しない。単体テスト`no_industrial_goods_requiring_building_means_no_gap_is_detected`、E2E `test_09_no_militaryfactory_means_no_factory_from_this_rule_alone`で確認(`decision_reason`が`SteelShortageSteelMillPriority`にならないことを直接検証)。

## 19. 段階的供給鎖修復

単体テスト`staged_repair_moves_from_factory_to_coal_mine_once_factory_exists`、E2E `test_07_staged_repair_connects_to_coal_mine_after_factory_completes`・`test_synthetic_passguard_style_full_supply_chain_repair`で、次の流れを実App・実Systemで確認した:

```
Factoryなし・MilitaryFactoryあり(IndustrialGoods不足検出)
→ Factory建設(SteelShortageSteelMillPriority)
→ Factory完成
→ Factory自身が要求するCoalの鉱床事前開発を検出(既存のMine規則)
→ Coal Mine建設(RawMaterialMinePriority)
→ Mine完成
→ IndustrialGoods生産が正の値になる
→ MilitarySupplies生産が正の値になる
```

## 20. Factory重複防止

§12参照。加えてE2E `test_06`で、建設中Factoryがある月の再評価で理由コードが`SteelShortageSteelMillPriority`へ戻らないことを確認した。

## 21. 実データ6か国の初期状態

実測(`assets/data/states.ron`、2026-09-12時点):

| 国 | 初期Factory合計level | 初期MilitaryFactoryあり | 初期IndustrialGoods構造的欠損 |
|---|---|---|---|
| Arcadia(0) | 1 | あり | なし |
| Elfin(1) | 0 | あり | **あり** |
| Dwarf(2) | 3 | あり | なし |
| Oceanic(3) | 0 | あり | **あり** |
| Passguard(4) | 0 | あり | **あり** |
| Ferrowyn(5) | 1 | あり | なし |

Elfin・Oceanic・Passguardの3か国が、実データ上すでに本タスクの対象となる構造的欠損を持っていた(合成シナリオを待つまでもなく実データで分岐が発火する)。

## 22. 6か国90日結果

E2E `test_90_day_and_36_month_six_country_real_data_verification`の実測(3か月、`PlayerCountry(None)`、診断用無限資金なし):

| 国 | 初回Factory発注月 | その時点の理由 |
|---|---|---|
| Arcadia(0) | 1か月目 | PeacetimeDevelopment(既存Factory保有、無関係な経済成長判断) |
| Elfin(1) | 1か月目 | **SteelShortageSteelMillPriority**(本タスクの新規規則) |
| Dwarf(2) | 1か月目 | PeacetimeDevelopment |
| Oceanic(3) | 1か月目 | **SteelShortageSteelMillPriority** |
| Passguard(4) | 1か月目 | **SteelShortageSteelMillPriority** |
| Ferrowyn(5) | 1か月目 | PeacetimeDevelopment |

初期に構造的欠損を持っていた3か国だけが本タスクの新規規則によってFactoryを得ており、既にFactoryを持っていた3か国のFactory追加は既存の無関係な経済成長判断(優先度3)によるものであることを、日次粒度でのqueue変化監視によって正確に切り分けた(月末にまとめて`decision_reason`を読むと、共有フィールドが後続の週次AI評価で上書きされ誤帰属するため、この切り分けには注意を要した――詳細は§34参照)。

## 23. 6か国36か月結果

| 国 | 36か月後Factory合計level | 36か月後IndustrialGoods構造的欠損 | 最終国庫 | 最終IndustrialGoods | 最終MilitarySupplies |
|---|---|---|---|---|---|
| Arcadia(0) | 4 | なし | -20233.03 | 221.226 | 9.652 |
| Elfin(1) | 3 | なし | 710.88 | 204.948 | 899.210 |
| Dwarf(2) | 8 | なし | -20074.78 | 1104.646 | 41.074 |
| Oceanic(3) | 19 | なし | 406.86 | 1688.904 | 26.191 |
| Passguard(4) | 1 | なし | -5009.82 | 17.580 | 2395.079 |
| Ferrowyn(5) | 3 | なし | -9325.68 | 845.257 | 69.859 |

36か月後には全6か国でIndustrialGoods構造的欠損が解消し、いずれも正のIndustrialGoods・MilitarySupplies生産を実現していることを確認した(実データはこのウィンドウ内で解決可能だった、という実測結果)。E2Eテストの最終アサーションで、初期に欠損を持っていた国(Elfin/Oceanic/Passguard)は`SteelShortageSteelMillPriority`によってFactoryを得たこと、持っていなかった国(Arcadia/Dwarf/Ferrowyn)はそうでないこと、および全6か国で36か月後に欠損が残っていないことを固定した。

一部国家(Arcadia/Dwarf/Ferrowyn)の国庫がマイナスに陥っている点は、Phase 26以前から存在する既知の財政バランス課題であり(P26-003-REV-001等で既出)、本タスクが新規に発生させたものではない。本タスクはFactory建設の要否判定・州選択のみを行い、財政再建・税率調整のロジックには一切触れていない。

## 24. 合成Passguard型シナリオ

実データでも自然に発火することを§21-23で確認済みだが、spec §18の要求どおり、合成的にDwarf(実データ上は既に完全な供給網を持つ)からFactory・Mine・鉱床を全て取り除き、Passguard型(MilitaryFactoryあり・Factoryなし・Coal鉱床未活用)の構造的欠損をゼロから作った上で、E2E `test_synthetic_passguard_style_full_supply_chain_repair`が次を実証した:

1. Factory候補なし状態から`SteelShortageSteelMillPriority`でFactory建設開始
2. 建設完了(既存の建設進捗Systemに委ねる、期間は改変せず)
3. Factory完成後、Factory自身が要求するCoalの鉱床事前開発(Mine)へ既存規則が接続
4. Mine完成
5. 最終的にIndustrialGoods・MilitarySuppliesの両方が正の値になることを実証

## 25. IndustrialGoods生産結果

§23の表を参照。全6か国で36か月後に正の値(17.580〜1688.904)。

## 26. MilitarySupplies生産結果

§23の表を参照。全6か国で36か月後に正の値(9.652〜2395.079)。

## 27. 国庫への影響

Factory建設費(1000.0)分の一時的な支出が発生する以外、新規の恒常的なコスト増加要因は導入していない。§23で述べたとおり一部国家の国庫がマイナスなのは既存の財政バランス課題であり、本タスクが原因ではない(Factory建設1回・1000.0の一時支出が36か月間の財政破綻トレンドを新たに引き起こしたとは考えにくい規模であり、Arcadia/Dwarf/Ferrowynの財政悪化はP26-003-REV-001以来の既知の傾向と一致する)。

## 28. 国力・順位・Tierへの影響

本タスクはFactoryの建設判断のみを行い、`src/country/power.rs`(P26-003-REV-003で経済力算出を`output_resources`の量から独立させ済み)には一切触れていない。Factoryの追加建設が経済力スコアへ与える影響は、既存の`base_economic_capacity_points(BuildingType::Factory) = 30.0`という固定値のみに基づき、追加Factory1棟につき固定30.0ポイントが加算されるだけである(P26-003-REV-003の既存契約どおり、資源量には依存しない)。個別のTier遷移までは本タスクの検証範囲外としたが、既存の`p21_014_country_power_e2e_test`等の回帰テストは全て無変更のまま通過している(§37参照)。

## 29. 募兵AIへの影響

`src/military/ai.rs`・`src/military/recruitment_supply_gate.rs`は一切変更していない。P26-008-Aの回帰テスト(`p26_008_a_supply_aware_recruitment_gate_e2e_test.rs`、30件)は無変更のまま全て通過した。

## 30. 攻勢AIへの影響

`src/war/military_ai.rs`・`src/war/offensive_supply_gate.rs`は一切変更していない。P26-008-Bの回帰テスト(`p26_008_b_supply_aware_offensive_gate_e2e_test.rs`、19件)は無変更のまま全て通過した。

## 31. Save V1判断

維持。新規Saveフィールドは追加していない(判断材料は所有建物・建物レベル・建設queue・必要入力資源・鉱床・資源備蓄・国家の生存状態のいずれも既存の永続状態から再導出可能)。E2E `test_16_old_save_without_any_new_field_loads_and_evaluates_normally`で、既存Saveがそのまま正常にロードできることを確認した。

## 32. save→load結果

E2E `test_14_decision_is_reproduced_identically_after_save_and_load`・`test_15_double_save_load_does_not_change_the_outcome`で、save→load(および2回連続save→load)後も同じ入力から同じ判断(`SteelShortageSteelMillPriority`、Factory建設)が再現されることを確認した。

## 33. 決定性

`find_proactive_development_target`は`ResourceType::ALL`の固定宣言順で走査し、`resource_types_required_by_owned_buildings`の内部集合(`HashSet`)反復順には依存しない(既存のP26-004-PRE-002設計を維持)。E2E `test_13_result_does_not_depend_on_state_registration_order`で、`StateRegistry.states`の登録順反転が結果を変えないことを確認した。全対象ファイルを5回ずつ独立プロセスで実行し、結果が完全に一致することも確認した(§39参照)。

## 34. 国家間分離

E2E `test_12_decisions_are_isolated_across_countries`で、一方の国家(Dwarf)の合成的なIndustrialGoods欠損が、既にFactoryを持つ別の国家(Arcadia)の判断へ影響しないことを確認した。

## 35. 既存テスト変更と理由

`CountryAiState.decision_reason`は建設AI(月次)・研究/戦争準備AI(週次)が同じフィールドを共有するため、月末にまとめて読み取ると後続の週次評価で上書きされ得るという、本タスクの新規E2Eテスト作成中に判明した既存の測定上の注意点があった(実装側のバグではなく、テスト側の観測方法の問題)。これに対応するため、テストヘルパー`advance_one_month_and_read_dwarf_construction_reason`を新設し、Dwarf(CountryId(2))の1か月目はNew Game直後の`dirty=true`により`day1`時点で既に建設AIが評価済みとなる(Dwarfの正規月次cadence`day%30==2`とday1が同月内で偶然2回评価を生む)特殊性を踏まえて、月ごとに正しい評価直後のタイミングで理由コードを読み取るようにした。これはテストコードのみの変更であり、`src/country/country_ai.rs`の実装ロジックには影響しない。

既存テストファイルの変更は2件、いずれも実装が正しく検出した新しい正当な帰結を反映するための、正規値(canonical baseline)の更新である(アサーション構造の弱体化ではない):

1. `tests/p26_det_001_construction_ai_determinism_e2e_test.rs::test_f_all_six_countries_stable_within_process_and_no_regression` — Elfin(CountryId(1))の36か月後MilitarySupplies生産分canonical値を`1785.448052`→`1804.029982`へ更新。原因: Elfinの既存Mine(StateId 16、P26-004-PRE-004でIron/Coal鉱床を追加済み)が既にIron側の構造的経路を満たしているため、本タスクの新規IndustrialGoods事前開発規則により、従来は2か月かかっていたFactory建設判断が初回の月次評価(month 1)で直接完了するようになった。結果としてFactory稼働開始が早まり、36か月間の累積生産量がわずかに増加する(実測1804.029982、独立プロセス3回で再現確認済み、決定的)。
2. `tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs::test_i_all_six_countries_stable_within_process_and_no_regression` — 同一の理由・同一の新値。

いずれも既存の正規値がP26-008-PRE-002によって過去に更新された同じ形式のコメント(理由の説明)を追加した上で更新しており、アサーション自体(許容誤差`1.0`未満での一致確認)は変更していない。

## 36. P26-004〜008-B回帰

個別に実行し全て0 failedを確認: `p26_008_b_supply_aware_offensive_gate_e2e_test`(19 passed)、`p26_008_a_supply_aware_recruitment_gate_e2e_test`(30 passed)、`p26_008_pre_002_ai_evaluation_cadence_e2e_test`(16 passed)、`p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test`(11 passed)、`p26_004_daily_supply_consumption_allocation_e2e_test`(19 passed)、`p26_005_supply_combat_strength_e2e_test`(10 passed)、`p26_006_capital_connectivity_supply_e2e_test`(18 passed)、`p26_006_rev_001_ai_save_resume_determinism_e2e_test`(9 passed)、`p26_007_elimination_occupation_overseas_supply_accounting_e2e_test`(23 passed)。

## 37. Phase 23〜25回帰

`cargo test --all-targets`の全体実行に包含済み(§40参照)。`land_war_combat_peace_test.rs`は無変更のまま保護。

## 38. Critical/High/Medium/Low

Critical/High findingなし。Medium findingなし。Low finding: なし(既知のEquipment起因queue停滞[P26-008-A §17由来]は本タスクの対象外のまま非変更)。

## 39. 対象外

MilitaryFactory建設AI、MilitarySupplies専用建物、生産レシピ変更、建物データ変更、鉱床データ変更、初期資源変更、補給生産量調整、補給備蓄の外生的付与、募兵ゲート変更(P26-008-A)、攻勢ゲート変更(P26-008-B)、AI評価cadence変更、財政救済、鉱床発見AI、交易、輸送、港・鉄道、UI・ローカライズ、AI目標Division数変更、Equipment不足による募兵queue停滞修正——いずれも変更していない。

## 40. 全実行コマンドと正確な件数

| コマンド | 結果 |
|---|---|
| `cargo test --all-targets`(開始時) | 2476 passed / 0 failed / 1 ignored |
| `cargo clippy --all-targets --all-features -- -D warnings`(開始時) | 0 warnings |
| `cargo test --lib country::country_ai::` | 29 passed(既存18 + 新規11、単体テストの分類詳細は§42参照) |
| `cargo test --test p26_008_c_supply_chain_construction_priority_e2e_test` | 18 passed |
| 回帰9ファイル個別実行(§36) | 全て0 failed |
| `cargo test --all-targets --no-fail-fast`(修正前、1回目) | 2504 passed / **2 failed** (Elfin 36か月canonical値の齟齬) |
| 正規値更新後 `cargo test --all-targets --no-fail-fast`(最終) | **2506 passed / 0 failed / 1 ignored** |
| `cargo clippy --all-targets --all-features -- -D warnings`(最終) | 0 warnings |

差分検算: 2476 + 12(単体、`src/country/country_ai.rs`) + 18(E2E) = 2506(一致)。

## 41. clippy結果

開始時・最終ともに0 warnings。

## 42. 全テストpassed/failed/ignored

**2506 passed / 0 failed / 1 ignored**(最終)。新規単体テスト12件(`industrial_goods_gap_is_detected_when_a_militaryfactory_needs_it_and_no_factory_exists`、`no_industrial_goods_requiring_building_means_no_gap_is_detected`、`factory_level_one_means_the_structural_path_already_exists`、`another_countrys_factory_does_not_satisfy_this_countrys_gap`、`food_emergency_takes_priority_over_the_industrial_goods_gap`、`deposit_resource_priority_order_is_preserved_ahead_of_industrial_goods`、`factory_candidate_state_excludes_other_countries_and_invalid_states`、`no_candidate_state_falls_through_without_panicking`、`insufficient_funds_for_factory_falls_through_to_priority_three`、`duplicate_factory_is_not_queued_while_one_is_already_under_construction`、`staged_repair_moves_from_factory_to_coal_mine_once_factory_exists`、`no_coal_deposit_does_not_fabricate_a_mine`)、新規E2Eテスト18件。

## 43. Cargo逐次実行

全てのcargoコマンドを逐次(単一プロセス)で実行した。並列実行は行っていない。

## 44. 対象外を先行実装していないこと

MilitaryFactory建設AI・交易・輸送・港・鉄道・鉱床発見AI・財政救済のいずれにも一切着手していない。

## 45. ユーザーSave保護

`saves/savegame_v1.ron`へは一切アクセスしていない。

## 46. PNG差分処理

`cargo test`実行に伴い、既知の非決定的スクリーンショット9ファイルが再生成され`git status`上で変更として現れたが、個別に`git checkout --`で復元した。

## 47. コミット対象

```
 M src/country/country_ai.rs
 M tests/p26_det_001_construction_ai_determinism_e2e_test.rs
 M tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs
?? tests/p26_008_c_supply_chain_construction_priority_e2e_test.rs
```

## 48. 最終`git status --short`

上記§47の5ファイル(報告書自身を含む)は`6ca69be`(`feat: extend AI construction for supply chains`)としてコミット済み。コミット後の`git status --short`はクリーン。

## 49. P26-008全体の完了判定

P26-008-PRE-001(設計)→P26-008-PRE-002(cadence基盤修正)→P26-008-A(募兵ゲート)→P26-008-B(攻勢ゲート)→P26-008-C(建設優先順位拡張)の全5タスクが完了した。**P26-008 COMPLETE**。

## 50. 次の原子的タスク候補

Phase 26の残存する既知課題(いずれも本タスク・P26-008全体の対象外として一貫して維持):

1. Equipment不足による募兵queue完成停滞(P26-008-A §17由来のMedium finding、未修正のまま)。
2. 一部国家(Arcadia/Dwarf/Ferrowyn)の長期的な財政バランス課題(P26-003-REV-001以来の既知の傾向)。
3. Phase 26全体の最終監査(P26-001からP26-008-Cまでの通し回帰確認、Critical/High/Medium/Lowの棚卸し)。

これらのうちどれを次の原子的タスクとするかは、ユーザーの判断が必要な事項である。
