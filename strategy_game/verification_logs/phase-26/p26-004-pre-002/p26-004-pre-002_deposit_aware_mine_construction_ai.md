# P26-004-PRE-002: 鉱床を考慮したMine建設AIの実装

## 1. 総合判定

**COMPLETE WITH FINDINGS**

対象範囲(Mine建設AIの誤判断の是正: 鉱床皆無国家への浪費防止・実在鉱床への事前開発)は実装・実App実測(6か国36か月)の双方で確認できた。Elfin・Oceanic・Passguard・Ferrowynの供給問題そのもの(領内鉱床の欠如、PassguardのIndustrialGoods生産経路の欠如)は本タスクの対象外として明示的に未解決のまま残っており、spec §14の「COMPLETE WITH FINDINGS」の定義(対象範囲の実装は正しいが既知の残存問題がある)に一致する。資金・データ・Save形式の変更は一切行っていない。

## 2. 開始時HEAD

```
723ac1913cfcd08d950d203c1e3da838b85e83b0
```

`git status --short`(開始時): ` M saves/savegame_v1.ron` のみ(想定と一致)。

## 3. 監査対象コードHEAD

`723ac1913cfcd08d950d203c1e3da838b85e83b0`(開始時HEADと同一。実装前の監査は全てこのコミットのソースを対象に行った)。

## 4. 最終コードHEAD

本報告書はコミット前に作成しているため、未来のコミットハッシュはここに記載しない(spec §15の明示的な禁止事項)。本タスクの最終コードHEADは、このファイルを含む1件のコミット自体のハッシュであり、`git log -1`で確認できる。

## 5. 変更前の建設AI

`src/country/country_ai.rs::process_construction_ai`(変更前、723ac19時点):

- 建設キュー2件までの制限は変更前から存在。
- 優先度: `Food<50` なら Farm、そうでなく `Wood/Iron/Coal<30` のいずれかなら Mine、それ以外は Factory(民需)。
- Mineの建設先選定(`selected_state`): `owned_states`(StateId昇順)を先頭から走査し、(a) 同一建物が既にキュー中でない、(b) `is_crystal_only_state`によりクリスタル専用州でない、(c) `current_level < max_level` の3条件のみで最初に一致した州を選ぶ。**実際に採掘可能な鉱床(discoveredかつ非MagicCrystal)が1件でも存在するかどうかは一切見ていなかった**。これが本タスクの核心的なバグであり、鉱床が皆無の国家(Elfin・Oceanic)でも、非クリスタル専用のいずれかの州が(鉱床の有無に関係なく)常に選ばれてしまっていた。
- 資源不足の閾値は固定値30.0のみで、Arcadiaのように鉱床はあるが未開発の国家について、財政破綻前に先回りしてMineを建てる仕組みは存在しなかった。

## 6. 鉱床の正規データ構造

`src/economy/resources.rs::StateResourceDeposit { resource_type: ResourceType, base_output: f64, discovered: bool, development_level: u32 }`(`StateData.resource_deposits: Vec<StateResourceDeposit>`)。

「実際に採掘可能」の正規条件は、生産モデルの権威である`src/economy/production.rs::process_country_production`のMine分岐(`if b_type == BuildingType::Mine { for deposit in &state.resource_deposits { if deposit.discovered && deposit.resource_type != ResourceType::MagicCrystal { ... } } }`)と完全に同一の条件(`discovered && resource_type != MagicCrystal`)を採用した(P21-009のMagicCrystal専用CrystalMine分離方針を踏襲、`is_crystal_only_state`の判定とも整合)。この条件を`src/country/country_ai.rs`内の新規ヘルパーで読み取り専用に再利用し、`assets/data/*.ron`・`src/economy/resources.rs`・`src/economy/production.rs`は一切変更していない。

## 7. Mine候補州の条件

`find_candidate_state`(新設、既存の`selected_state`クロージャを関数化)に、Mine建設固有の条件として以下を追加した:

1. 対象国家が所有(`owner_country_id`)かつ実効支配(`controller()`)する陸上州である(既存)。
2. 同一建物が既に建設キュー中でない(既存)。
3. クリスタル専用州でない(既存、P21-009-FIX-001)。
4. **新設**: `resource_deposits`のうち `discovered && resource_type != MagicCrystal` を満たす鉱床が1件以上存在する。
5. `building_level(Mine) < max_level`(既存)。

条件4が今回追加した核心のガードであり、これにより鉱床が一切ない州へのMine選択、および鉱床皆無国家でのMine選択そのものが構造的に不可能になった。

## 8. 事前開発判定

新設関数`find_proactive_mine_state`(および補助関数`resource_types_required_by_owned_buildings`・`has_active_mine_for_resource`・`find_undeveloped_deposit_state`)で実装。

1. `resource_types_required_by_owned_buildings`: 対象国家が所有する全州の`buildings`(`level > 0`のみ)を走査し、`BuildingRegistry`から該当`BuildingDefinition.input_resources`のキー(`ResourceType`)を集める(特定資源名を列挙しない汎用実装、spec §5.3準拠)。
2. `ResourceType::ALL`の宣言順で走査し、`MagicCrystal`(通常Mineの対象外)と、まだ集合に含まれない資源をスキップ。
3. `has_active_mine_for_resource`: 領内のいずれかの州で「discoveredな当該資源鉱床」と「`Mine`レベル>0」が同居していれば、既に稼働中とみなしスキップ(Dwarf型の既存供給網を保護)。
4. `find_undeveloped_deposit_state`: 上記を満たさない資源について、`find_candidate_state`と同じ実採掘可能性・増設余地・重複キュー除外条件を満たす州をStateId昇順で1件探す。
5. 呼び出し元(`process_construction_ai`)で、`country.treasury >= mine_def.construction_cost`(建設費を支払える)を事前開発発火の前提条件として課す(spec §5.3の「建設費を既存ルールで支払える」の解釈: 支払えない場合は事前開発を発火させず優先度3へフォールスルーする)。

## 9. 優先順位

```
(1) 食料緊急(Food<50 → Farm)                      … 既存、変更なし
(2) 未活用鉱床の事前開発(§8の条件を満たせばMine)    … 新設、既存の閾値30より先に評価
(3) 既存の通常判断(Wood/Iron/Coal<30 → Mine、else Factory)
    (3a) このMine選択が§7の条件で候補州0件だった場合、
         Factoryへ安全にフォールバック(spec §5.2)   … 新設
```

spec §5.4が要求する順序関係(緊急Food対策 > 未活用鉱床の事前開発 > 通常建設判断)をそのまま実装した。

## 10. 複数候補のtie-break

`find_proactive_mine_state`: `ResourceType::ALL`の宣言順(Food, Wood, Iron, Coal, MagicCrystal[除外], IndustrialGoods, MilitaryEquipment, RawMagicCrystal, MilitarySupplies)を第1キー、同一資源内は`owned_states`(既にStateId昇順にソート済み)の先頭一致を第2キーとした。spec §5.5の推奨順位(1: 保有建物の入力不足解消、5: StateId昇順、6: ResourceType正規順)に沿う実装であり、`candidate_selection_is_deterministic_regardless_of_state_insertion_order`テストで、同一の論理状態を異なる州挿入順(`[low, high]` と `[high, low]`)で構築しても同じStateId(1)が選ばれることを確認した。

## 11. HashMap順非依存

`resource_types_required_by_owned_buildings`は内部で`HashSet<ResourceType>`を使うが、最終的な選定は呼び出し元が`ResourceType::ALL`(固定配列)の順で走査するため、HashSet/HashMapの反復順は結果に一切影響しない。`state_data.buildings: HashMap<BuildingType, u32>`の走査順も、集合への追加(`insert`)にのみ使われ、順序に依存する分岐は存在しない。上記のTest H(決定性テスト)で実測により再確認済み。

## 12. 原子性

- 資金不足・候補州なしのいずれの場合も、`try_construct`および事前開発パスは`country.treasury`・`country.construction_queue`を一切変更しない(`ConstructOutcome::NoCandidate`/`InsufficientFunds`のいずれも副作用なしで返す)。
- 建設対象の決定(候補州の探索)と実際の支払い・キュー追加は同一の`if let Some(...)`ブロック内で連続して行われ、部分適用(州だけ決まって支払いだけ行われない、支払いだけ行われて州が決まらない等)が発生する経路は存在しない。
- `insufficient_funds_blocks_proactive_mine_without_any_partial_update`(単体テスト)・実App版`test_g1_save_load_immediately_before_construction_does_not_double_charge`で、失敗時に国庫・キューが完全に不変であることを確認した。

## 13. 資金不足時の挙動

- 事前開発規則: `country.treasury >= mine_def.construction_cost` を満たさない場合、そもそも候補探索自体を行わず優先度3へフォールスルーする(発火条件の一部として扱う、事後的な失敗処理ではない)。
- 優先度3(通常判断)でMineが選ばれたが資金不足の場合: `decision_reason = FundsShortage` を設定し、**Factoryへのフォールバックは行わない**(spec Test Fの要求どおり、「他建物を部分更新しない」)。フォールバックはあくまで「候補州が存在しない(鉱床皆無)」場合専用であり、「資金が足りない」場合とは明確に区別した。

## 14. Arcadiaの変更前後

**変更前**(P26-004-PRE-001実測値の引用): 州0(Iron50/Coal40の実在鉱床、Mine未建設)を持ちながら、既存AIは`Iron/Coal<30`の閾値が破れるまでMineを検討しない。実際には財政が月1終了時点で既に赤字(`monthly_income=2591.65` vs `monthly_expenses=2892.92`)であり、月6時点で治療-924.12まで悪化。無制限資金の診断でようやく月13からMine選択が始まる(閾値到達のタイミング)。通常の財政ではこの閾値到達前に建設不能な状態へ陥り、Iron枯渇・Supplies恒久停止(月18以降373.96で固定)。

**変更後**(本タスクで実測、通常の財政条件、`countries.ron`のtreasury=5000.0を無変更):
- 州0のMilitaryFactory(lv1)・Factory(lv1)が既にIron/Coal/IndustrialGoodsを入力として必要としており、州0のIron/Coal鉱床は未開発(Mineレベル0)。事前開発規則がこれを検出し、New Game開始直後の最初の日次AI評価(month1, day2)で即座に州0へMine建設を着手(`decision_reason = RawMaterialMinePriority`、国庫は5000.0の初期値からMine建設費500.0だけ正確に減少)。
- 建設進捗(required_progress=60.0、既存の日次+1進捗)により、月3の時点で州0のMineレベルが1(完成)に到達。
- Iron/Coal備蓄は以後、生産と消費が拮抗する形で0付近を上下し続け、恒久的な枯渇には至らない(月別実測値の一部: 月16 Iron=0.81、月17 Iron=5.42、月36 Iron=0.75)。
- MilitarySupplies: 月18時点で598.06(旧来の恒久停止値373.96を明確に上回る)、月36時点で896.76まで単調に増加を継続。旧来のような「月18以降373.96で完全固定」というパターンは解消された。

## 15. Dwarfへの影響

事前開発規則はDwarf(州5にIron100/Coal80鉱床とMine lv5が同居)には一切発火しない(`has_active_mine_for_resource`がIron・Coalともに既に稼働中と判定するため)。実App36か月実測値は、P26-004-PRE-001が記録した改修前の値と完全に一致した:

| 月 | 12 | 18 | 24 | 36 |
|---|---|---|---|---|
| MilitarySupplies(改修前記録値) | 517.65 | 800.00 | 1035.29 | 1487.06 |
| MilitarySupplies(本タスク実測値) | 517.65 | 800.00 | 1035.29 | 1487.06 |

既存の唯一の持続可能国の供給網に対する回帰は一切ない。

## 16. Elfin/OceanicのMine浪費防止

- **Elfin**: 実データ上、鉱床とは無関係な初期建物としてStateId(16)に`Mine: 1`が既に配置されている(`assets/data/states.ron`)。この初期値は本タスクの対象外(既存マップデータ、鉱床の有無と無関係)であり、除去も追加もしていない。36か月間AI操作させた実測で、Mine総レベルは初期値1のまま一切増加しなかった(改修前はP26-004-PRE-001が「月30時点でMine総lv10=上限」まで浪費し続けたことを実測済み)。MilitarySuppliesは36か月間0のまま。
- **Oceanic**: 初期Mine総レベル0(初期建物も存在しない)。36か月間AI操作させても総レベルは0のまま一切増加しなかった(改修前は「月36時点でMine総lv32」まで浪費)。MilitarySuppliesは36か月間0のまま。

いずれも実App・実データ経由の36か月シミュレーション(`test_k1_elfin_never_wastes_treasury_on_a_mine_over_36_months`・`test_k2_oceanic_never_wastes_treasury_on_a_mine_over_36_months`)で確認した。

## 17. Passguard/Ferrowynに残る問題

- **Passguard**: 4州いずれにも登録鉱床がなく、かつIndustrialGoods生産経路(Factory)自体が存在しないため、MilitaryFactoryの稼働率が恒久的に0のまま。実測でIron備蓄は初期値100.0から36か月間一度も消費されず(改修前のP26-004-PRE-001実測と完全に一致)、MilitarySuppliesは36か月間0のまま。本タスクのMine建設AI修正はこの問題を一切解決しない(spec対象外、意図的に未解決のまま残す)。
- **Ferrowyn**: 4州いずれにも登録鉱床がない。実データ上、鉱床とは無関係な初期建物としてStateId(19)に`Mine: 1`、StateId(21)に`Mine: 2`(合計3)が既に配置されている。36か月間、Mine総レベルは初期値3のまま一切増加しなかった(本タスクが新たな浪費を追加しなかったことの確認)。一方、初期Iron/Coal備蓄(鉱床ではなく単なる初期在庫)を使い切るまでの間だけMilitarySuppliesが一時的に生産され、改修前と同じ約320.84付近で固定化した(本タスク実測: 320.837、Iron/Coalは月16〜20頃に枯渇)。これは鉱床の欠如という根本問題が未解決のままであることを示しており、本タスクのAI修正だけでは解決できない(spec対象外として明示)。

いずれも「供給可能と誤判定していないこと」を実測で確認済み(`test_l1_passguard_remains_unresolved_by_this_task`・`test_l2_ferrowyn_remains_unresolved_by_this_task`)。

## 18. 6か国36か月比較

| 国家 | Mine総レベル(改修前→改修後、36か月時点) | MilitarySupplies(改修前記録値→本タスク実測値、36か月時点) | 備考 |
|---|---|---|---|
| Arcadia(0) | 0→**1**(州0で新規完成) | 373.96(月18以降固定)→**896.76**(増加継続) | 事前開発規則により財政破綻前に建設着手(月1)、月3に完成 |
| Elfin(1) | 1(初期値、月30時点で改修前は10まで浪費)→**1(不変)** | 0→0 | 浪費防止を確認、供給問題自体は未解決(対象外) |
| Dwarf(2) | 5(不変) | 1487.06(改修前記録値)→**1487.06**(完全一致) | 無影響、回帰なし |
| Oceanic(3) | 0(改修前は月36時点で32まで浪費)→**0(不変)** | 0→0 | 浪費防止を確認、供給問題自体は未解決(対象外) |
| Passguard(4) | 0(不変) | 0→0 | 変化なし、IndustrialGoodsチェーン欠如は未解決(対象外) |
| Ferrowyn(5) | 3(不変) | 320.84(改修前記録値)→**320.837**(ほぼ完全一致) | 一時的生産の挙動は不変、鉱床欠如は未解決(対象外) |

比較対象の「改修前記録値」は`verification_logs/phase-26/p26-004-pre-001/`が実App実測で記録した値であり、旧AIを本タスク内で再実行して比較したものではない(spec §7 Test Mの要求どおり明記)。

## 19. New Game/Loaded Game比較

`test_h1_new_game_and_loaded_game_agree_on_arcadia_mine_construction`: 3か月連続で進めたケースと、同じく3か月進めた後にsave→loadを挟んだケースとで、Arcadiaの州0Mineレベル・国庫・MilitarySupplies備蓄が完全に一致することを確認した。`process_construction_ai`は既存の国家・州・建物・資源のみを読み取り、新規保存フィールドを一切参照しないため、決定論的に同一の判断になる。

## 20. save/load分岐比較

- `test_g1_save_load_immediately_before_construction_does_not_double_charge`: 建設着手前(キュー0件)にsave→loadし、その後日次tickを進めた。建設キューは(既存仕様どおり)上限2件(Mine@州0、Factory@州0)まで埋まり、国庫の減少額はキュー内の`paid_cost`合計と完全に一致(二重支払いなし、Mineの重複なし)。
- `test_g2_save_load_after_construction_starts_preserves_state_exactly`: 3か月経過後(既に建設着手済み)にsave→loadし、国庫・Mineレベル・キュー件数が完全に一致することを確認。

## 21. Save V1維持

`src/country/country_ai.rs`のみを変更し、新規`#[serde]`フィールドは一切追加していない。`CountryData`・`StateData`・`CountryAiState`・`ConstructionQueueItem`等のSave対象データ構造は無変更。`process_construction_ai`は既存フィールド(`stockpile`・`treasury`・`construction_queue`・`state.buildings`・`state.resource_deposits`)のみを読み取る。migrationの追加も不要。

## 22. Phase 23〜25回帰

`cargo test --lib country::`(89 passed / 0 failed、`country::elimination`・`country::power`・`country::tests`を含む)で、Phase 21の国家消滅API・Phase 23〜25の国力計算に回帰がないことを確認。`cargo test --all-targets`全体でも、Phase 25装備会計関連の既存E2Eテスト(P25系)を含め0 failedだった。

## 23. P26-002/003回帰

| テストファイル | 結果 |
|---|---|
| `p26_002_military_supplies_accounting_foundation_e2e_test` | 45 passed / 0 failed |
| `p26_003_military_supplies_production_e2e_test` | 49 passed / 0 failed |
| `p26_003_rev_002_supply_production_units_e2e_test` | 56 passed / 0 failed |
| `p26_003_rev_003_economic_power_capacity_e2e_test` | 21 passed / 0 failed |

いずれもベースラインと同一件数・0 failedであり、`src/economy/production.rs`・`src/economy/resources.rs`・`assets/data/buildings.ron`を一切変更していないことと整合する。

## 24. 対象外を先行実装していないこと

`git diff --stat`(コミット前)は`src/country/country_ai.rs`のみを変更対象とし、新規追加は`tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs`のみである。以下を確認済み:

- `assets/data/*.ron`(鉱床データ・建設費・初期資源)は無変更。
- `src/economy/production.rs`・`src/economy/resources.rs`・`src/military/`・`src/save/`は無変更。
- 新規`BuildingType`・新規`ResourceType`・新規Saveフィールドは追加していない。
- MilitaryFactoryをAI建設対象に追加していない(`process_construction_ai`の3分岐対象はFarm/Mine/Factoryのまま)。
- 日次補給消費・配分・戦闘力補正(P26-004本体・P26-005)は一切配線していない。
- dead `military::supply::process_supply`・`war::combat::process_combat`は再活性化していない。

## 25. Critical/High/Medium/Low

**Critical: 0 / High: 0 / Medium: 0 / Low: 2**

- Low-1: Arcadiaの国庫が36か月の実測で最終的に大きく赤字化する(月36時点で-20293.28)。これは本タスクが新たに作った問題ではなく、P26-004-PRE-001 §7が既に指摘した「建物維持費・物流維持費・行政費と税収のバランス」という既存の経済バランス上の残件であり、本タスクのスコープ外(財政バランス全体の修正は対象外)。次タスクの検討材料として記録するに留める。
- Low-2: 鉱床皆無国家(Elfin/Oceanic)がMine候補なしでFactoryへフォールバックした結果、Factory建設が(既存のFactory上限10まで)継続的に選ばれ続ける。無限ループではない(`max_level`で自然に頭打ちになる)ため、spec §5.2が禁止する「フォールバックによる別建物の無限建設」には該当しないが、フォールバック後の挙動を新たに増やした点はfindingとして記録する(コードは変更していない)。

## 26. 実行したコマンド

以下を逐次実行した(並列実行なし):

```
git rev-parse HEAD
git status --short
cargo build --lib
cargo test --lib country::country_ai::
cargo clippy --lib -- -D warnings
cargo test --lib country::
cargo test --lib economy::
cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test
cargo test --test p26_003_military_supplies_production_e2e_test
cargo test --test p26_003_rev_002_supply_production_units_e2e_test
cargo test --test p26_003_rev_003_economic_power_capacity_e2e_test
cargo test --test p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
git checkout -- <9件の既知の非決定的PNG>
git status --short
```

## 27. 正確なテスト件数

`cargo test --all-targets`の全59個のテストバイナリの`test result:`行を実測合計した結果:

**2046 passed / 0 failed / 1 ignored**(開始時基準2025 passed / 0 failed / 1 ignoredから、新規21件を追加: `src/country/country_ai.rs`内の単体テスト10件 + `tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs`のE2Eテスト11件)。

新規単体テスト(10件、`country::country_ai::tests`):
`zero_deposit_country_never_builds_mine_and_falls_back_to_factory`,
`zero_deposit_country_with_no_fallback_building_builds_nothing`,
`mine_is_built_on_the_state_with_a_deposit_not_the_empty_one`,
`proactive_mine_development_fires_before_the_reactive_stockpile_threshold`,
`proactive_rule_ignores_a_deposit_that_does_not_match_the_needed_resource`,
`proactive_rule_does_not_force_an_extra_mine_when_supply_is_already_active`,
`insufficient_funds_blocks_proactive_mine_without_any_partial_update`,
`mine_at_max_level_is_excluded_from_every_candidate_path`,
`candidate_selection_is_deterministic_regardless_of_state_insertion_order`,
`construction_decisions_are_isolated_across_countries`。

新規E2Eテスト(11件、`tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs`):
`test_j1_arcadia_queues_mine_on_state0_within_month_1_under_normal_treasury`,
`test_j2_arcadia_mine_completes_and_iron_production_improves_over_36_months`,
`test_k1_elfin_never_wastes_treasury_on_a_mine_over_36_months`,
`test_k2_oceanic_never_wastes_treasury_on_a_mine_over_36_months`,
`test_l1_passguard_remains_unresolved_by_this_task`,
`test_l2_ferrowyn_remains_unresolved_by_this_task`,
`test_dwarf_supply_chain_is_not_disturbed_by_the_proactive_rule`,
`test_h1_new_game_and_loaded_game_agree_on_arcadia_mine_construction`,
`test_g1_save_load_immediately_before_construction_does_not_double_charge`,
`test_g2_save_load_after_construction_starts_preserves_state_exactly`,
`test_m1_six_country_36_month_snapshot_matches_expected_deposit_aware_outcomes`。

## 28. clippy警告数

`cargo clippy --all-targets --all-features -- -D warnings`: **0 warnings, 0 errors**(`cargo clippy --lib -- -D warnings`単体でも同様に0)。

## 29. ユーザーセーブの保護

`saves/savegame_v1.ron`は本タスクを通じて一度も読み取り・書き込み・ステージングしていない。作業開始時から一貫して`git status --short`に` M saves/savegame_v1.ron`として現れ続けており、この変更は本タスク開始前から存在していた既存の差分である。コミット対象には含めない。

## 30. コミット対象ファイル

- `src/country/country_ai.rs`(実装変更 + 新規単体テスト10件)
- `tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs`(新規E2Eテストファイル)
- `verification_logs/phase-26/p26-004-pre-002/p26-004-pre-002_deposit_aware_mine_construction_ai.md`(本報告書)

`saves/savegame_v1.ron`は含めない。

## 31. 最終`git status --short`

```
 M saves/savegame_v1.ron
 M src/country/country_ai.rs
?? tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs
?? verification_logs/phase-26/p26-004-pre-002/
```

(コミット直前の状態。`saves/savegame_v1.ron`は開始時から不変のユーザーファイルであり、本タスクの変更に含めない。)

## 32. 次の原子的タスクの推奨

1. **P26-004-PRE-001 §21の判断事項1・2の決着**: Elfin/Oceanic/Ferrowynの恒久的な鉱床欠如、およびPassguardのIndustrialGoodsチェーン欠如への対処方針(データ追加か、非対称格差として受容するか)をユーザー判断で決定する。本タスクはこれらのAI浪費だけを止めたに過ぎず、根本的な供給問題は未解決のまま。
2. **財政・維持費バランスの見直し(Low-1の残件)**: 本タスクの実測でも再確認された「Arcadia・Ferrowyn・(長期的には)Dwarfも含む治療の悪化トレンド」は、Mine建設AIとは独立した既存の経済バランス上の課題であり、別タスクとして扱うべき(spec ground ruleにより本タスクでは対応しない)。
3. **P26-004(日次補給需要・配分)の実装**: P26-004-PRE-001 §18・§19が示した実装順序どおり、会計API側は技術的にREADYのまま。本タスクの完了によりArcadiaの供給経路が実際に開通したことが確認できたため、次段階として日次消費の配線に進める条件が整った。
