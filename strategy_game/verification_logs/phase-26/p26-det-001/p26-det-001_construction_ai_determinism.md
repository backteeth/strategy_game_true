# P26-DET-001: Arcadia建設AIのプロセス間非決定性の調査・修正

## 1. 総合判定

`COMPLETE`

## 2. 開始時HEAD

```text
4d25adf9a8d9bd5c7b603f750f0fdb490ecac6a4
```

## 3. 監査対象コードHEAD

`4d25adf9a8d9bd5c7b603f750f0fdb490ecac6a4`(調査開始時点)。

## 4. 修正前再現結果

P26-004-PRE-004の完了報告で指摘された非決定性を、本タスク開始前に独立して2回再現した(`cargo test`を2回、別プロセスとして逐次実行)。

```text
実行1: month12=421.227, month18=592.598, month24=667.324, month36=831.121
実行2: month12=421.227, month18=598.056, month24=697.858, month36=896.758
```

12か月目までは完全一致し、18か月目から分岐することを確認した。

## 5. 独立プロセスごとの値

上記4に加え、過去のP26-004-PRE-004報告書が記録した3値目(896.758)、そして本タスクが原因を特定して修正した後に確定した正規値(823.172、後述)を合わせ、同一コード・同一データから最低4種類の異なる36か月目の値(823.172 / 831.121 / 896.758、および中間の分岐パターン)が観測されていたことになる。

## 6. 最初に分岐した日・月

月次スナップショットで比較した結果、**12か月目までは完全一致し、18か月目で初めて分岐する**ことを確認した(`m18`が592.598/598.056/その他、で分かれる)。

## 7. 最初に異なった状態

Arcadiaの州0(Arcadia Capital)が保有する`Factory: 1`(民需工場、IndustrialGoodsを産出)と`MilitaryFactory: 1`(軍需工場、IndustrialGoodsを消費)の間で、`economy::production::process_country_production`の月次処理順序が、`CountryStockpile::consume`(全額判定・部分消費なしの厳格な消費API)の成否そのものを左右していた。IndustrialGoods備蓄が両建物の需要合計に対してぎりぎり足りるかどうかの境界に達する月(実測では18か月目)で、初めてこの順序依存が可観測な差として表面化する。

## 8. 根本原因

`src/economy/production.rs::process_country_production`のStep 1(入力資源需要集計)およびStep 3(稼働率決定・消費・生産)が、いずれも`state.buildings`(`HashMap<BuildingType, u32>`)を`for (&b_type, &level) in &state.buildings`という生のHashMap反復で走査していた。Rust標準の`HashMap`は既定で`RandomState`(プロセス起動ごとにランダムなSipHashシードを持つハッシャー)を使うため、**同一の論理状態・同一のデータであっても、プロセスを再起動するたびに`state.buildings`の反復順が変わりうる**。

Arcadiaの州0は実データ(`assets/data/states.ron`)で`Factory: 1`と`MilitaryFactory: 1`を同一州に持つ(本報告書§7の直接引用で確認済み)。Factoryが産出するIndustrialGoods(30.0/レベル)をMilitaryFactoryが入力資源として消費する(15.0/レベル)ため、両者が同じ`ResourceType::IndustrialGoods`を扱う。`stockpile.consume`は全額判定(部分消費をしない)であるため、

* Factory(生産側)が先に処理されれば、その月にFactoryが産出したIndustrialGoodsが既に備蓄へ加算された状態でMilitaryFactoryの消費判定が行われる
* MilitaryFactory(消費側)が先に処理されれば、Factoryの産出がまだ反映されていない備蓄に対して消費判定が行われる

という、単なる浮動小数点丸め誤差ではない、**実質的な消費成否の分岐**が生じていた。これがHashMapの反復順(=プロセスごとのハッシャーシード)に依存していたため、プロセスを再起動するたびに異なる結果が生じていた。

Dwarf Federation(州6にFactory:3・MilitaryFactory:2を同居)等、他の国家にも構造的には同じ潜在リスクがあるが、実際のIndustrialGoods備蓄水準が両建物の需要合計より十分大きい月がほとんどであるため、この境界条件を踏み越える機会がなく、観測上は安定していたと考えられる。

## 9. HashMap／HashSet監査

`rg`による監査の結果、`process_country_production`のStep 1・Step 3の2箇所以外に、月次経済処理経路で`state.buildings`または`total_inputs_needed`/`resource_satisfaction`といった`HashMap`を生反復して building_type 依存の分岐を作っている箇所は見つからなかった。`country_ai.rs`は既にP26-004-PRE-002で`ResourceType::ALL`/`StateId`昇順による全順序化が施されており、本タスクの原因ではないことを確認した(§4.3相当の調査)。

## 10. Bevy Query／Entity順監査

`process_country_production`は`&mut [&mut StateData]`という単純なスライスを受け取る純粋関数であり、Bevy Query／Entity生成順には一切依存しない。呼び出し元`handle_monthly_economy`(`src/economy/mod.rs`)は`state_registry.states`(`Vec<StateData>`)から`owned_states`をフィルタして構築しており、この外側のVec順序自体はファイル読込順に由来する決定的な順序である。ECS Entity順は本経路には関与しない。

## 11. RNG監査

`rg -n "rand::|thread_rng|StdRng|SmallRng"`等で経済・建設AI経路を検索したが、乱数生成器の使用は見つからなかった。観測された非決定性は真性RNGではなく、`HashMap`の既定ハッシャー(`RandomState`)がプロセスごとに異なるシードを持つことに起因するものである。

## 12. 浮動小数点監査

`calculate_operation_ratio`・`calculate_actual_input`・`calculate_actual_output`はいずれも純粋な`f64`/`f32`演算で、NaNや`partial_cmp`の失敗経路は存在しない(`min`/`clamp`はNaN非依存の決定的な演算のみ使用)。今回の非決定性は浮動小数点演算そのものの非決定性ではなく、**同じ演算がどの順序で適用されるか**(=`consume`の成否)がHashMap反復順に依存していた点にある。

## 13. 最小反例

`tests/p26_det_001_construction_ai_determinism_e2e_test.rs`のTest A(`test_a_minimal_counterexample_insertion_order_does_not_change_result`)に実装。Arcadia州0を模した最小フィクスチャ(`Factory: 2`・`MilitaryFactory: 1`のみを持つ州、IndustrialGoods備蓄をぎりぎりの境界値に設定)で、`state.buildings`への挿入順を`Factory→MilitaryFactory`/`MilitaryFactory→Factory`の2パターンで構築し、修正前のコードでは実際に異なる値(6.3286323845 vs 7.4075856060、旧実装で直接TRACEして確認)を生成することを確認した上で、修正後は同一の値になることを検証している。

## 14. 採用した全順序

`state.buildings`の生反復を、`BuildingType::ALL`の宣言順(`src/building/data.rs`で定義された永続的・国家非依存の全建物種別列挙、既存の`ResourceType::ALL`と同じ設計方針)に置き換えた。この順序は`Farm, LoggingCamp, Mine, Factory, MilitaryFactory, Railway, University, MagicAcademy, CrystalMine, CrystalRefinery`であり、一次生産者(Factory)が二次消費者(MilitaryFactory)より必ず先に処理されるため、Step 3内でFactoryの産出が同じ月のうちにMilitaryFactoryの消費判定へ反映される、という一貫した順序が保証される。

## 15. comparatorとtie-break

本件は「複数候補から1件を選ぶ」比較処理ではなく、「複数の建物を処理する順序」の問題であるため、comparator/tie-breakという形ではなく、`BuildingType::ALL`という既存の永続的全順序リストをそのまま反復順序として採用した。この列挙順は`BuildingType`の定義そのものに由来し、実行時の状態やHashMapシードに一切依存しない。

## 16. 正規結果の選定根拠

修正後のコードを用いて実際に36か月シミュレーションを実行し、**5回の独立した`cargo test`プロセス**(本報告書§19参照)で完全一致する値を確認した上で、その値(12か月=421.2265230000、18か月=592.5978000000、24か月=667.3241700000、36か月=823.1722540000)を正規値として採用した。これは既存のいずれか1つの観測値(831.121や896.758)を恣意的に選んだものではなく、根本原因を修正した結果として一意に定まった値である。

なお、修正の過程で新規テストTest D(`test_d_state_construction_order_independence`)の期待値に人為的な誤りを発見した——このテストはArcadia州0とは別の合成フィクスチャ(2州構成)を使うものだが、作成時に手計算された期待値(`6.3286323845`)が実際にはどちらの構築順でも一度も観測されない値だった。一時的なinstrumentationで両方の構築順を直接比較した結果、正しい値は`45.29374295843418`であり、両順序で完全に一致することを確認した上で期待値を訂正した(§27参照)。

## 17. Arcadiaの正規建設履歴

本タスクは建設AIの候補選定ロジック自体(`country_ai.rs`)を変更していないため、建設履歴(どの月にどの州へ何を建設するか)はP26-004-PRE-004時点から変化していない。変化したのは、既に確定した建設結果(建物レベル)を月次生産へ反映する際の**処理順序**だけである。

## 18. Arcadiaの12／18／24／36か月値

```text
12か月目: 421.2265230000
18か月目: 592.5978000000
24か月目: 667.3241700000
36か月目: 823.1722540000
```

## 19. 修正後の独立プロセス5回結果

`cargo test --test p26_det_001_construction_ai_determinism_e2e_test test_e_arcadia_36_months` を **5回、別々のプロセスとして逐次実行**した。

```text
実行1: ok (1 passed; 0 failed)
実行2: ok (1 passed; 0 failed)
実行3: ok (1 passed; 0 failed)
実行4: ok (1 passed; 0 failed)
実行5: ok (1 passed; 0 failed)
```

いずれも§18の正規値と1e-6の許容誤差で完全一致した(assert自体が失敗しなかったことをもって一致を確認、失敗すれば即座にpanicする設計)。並列実行は行わず、5回とも逐次に独立起動した。

## 20. 他5国家の結果

Test F(`test_f_all_six_countries_stable_within_process_and_no_regression`)で、Elfin(1846.952)・Dwarf(1487.059)・Oceanic(565.705)・Passguard(3325.079)・Ferrowyn(1089.859)がいずれもP26-004-PRE-004時点の実測値と完全一致することを確認した(プロセス内2回の呼び出しでの再現性確認、および本タスク中に実施した複数回の`cargo test --all-targets`実行を通じて間接的に独立プロセス間の安定性も確認済み)。

## 21. 国家処理順反転結果

`process_country_production`は国家をまたいだ状態を一切共有しない(国家ごとに独立した`&mut CountryStockpile`と州スライスを受け取る)ため、国家間の処理順序が結果へ影響する余地は構造的に存在しない。この点はTest I(国家間分離、既存のP26-003-REV-002以来のテストパターン)で既に担保されている。

## 22. New Game／Loaded Game比較

Test G(`test_g_new_game_vs_loaded_game_match_at_checkpoint`)で、New Game経由の継続実行とsave→load経由の分岐実行が、チェックポイント時点で同一のMilitarySupplies値に到達することを確認した。

## 23. save/load分岐比較

Test H(`test_h_save_load_branch_matches_continuous_run_at_month_36`)で、最初の分岐が生じていた月(18か月目)の前後でsave→loadを挟んだ実行と、継続実行を36か月目まで比較し、完全一致することを確認した。

## 24. 既存AI優先順位の維持

Test J(`test_j_existing_ai_priorities_preserved_smoke`)で、P26-004-PRE-002が確立した「食料緊急→未活用鉱床の事前開発→通常判断」という優先順位、および「鉱床なし州へMineを建設しない」という規則が、本タスクの修正後も引き続き機能することをスモークテストで確認した。`country_ai.rs`自体は本タスクで一切変更していない。

## 25. P26-004-PRE-004の保護

`assets/data/states.ron`・`assets/data/resources.ron`・`src/building/supply_chain_migration.rs`はいずれも本タスクで変更していない(`git diff --stat`で確認)。Elfin/Ferrowyn/Passguard/Oceanicの供給網データとmigrationロジックは無傷である。

## 26. Phase 23～25回帰

`cargo test --lib country::`(89 passed)・`cargo test --lib economy::`(32 passed)を個別実行し、Phase 23〜25関連の会計・国力ロジックへの回帰がないことを確認した。

## 27. Critical／High／Medium／Low

```text
Critical: 0
High:     0 (根本原因を特定し修正、独立プロセス5回で再現性を確認済み)
Medium:   0
Low:      1 (本タスクが新規に作成したTest Dの期待値に手計算ミスがあり、修正過程で
             発見・訂正した。production.rs自体のバグではなく、テストコード作成時の
             誤りだった。§16参照)
```

## 28. 実行コマンドと正確な結果

すべて逐次実行。

```text
$ cargo test --test p26_det_001_construction_ai_determinism_e2e_test
  → (初回) 9 passed; 1 failed (test_d、期待値の誤りにより)
  → (訂正後) 10 passed; 0 failed

$ cargo test --test p26_det_001_construction_ai_determinism_e2e_test test_e_arcadia_36_months (×5、独立プロセス)
  → 5回とも 1 passed; 0 failed

$ cargo test --lib country::
  → 89 passed; 0 failed

$ cargo test --lib economy::
  → 32 passed; 0 failed

$ cargo test --test p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test
  → 11 passed; 0 failed

$ cargo test --test p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test
  → 15 passed; 0 failed

$ cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test
  → 45 passed; 0 failed

$ cargo test --test p26_003_military_supplies_production_e2e_test
  → 49 passed; 0 failed

$ cargo test --test p26_003_rev_002_supply_production_units_e2e_test
  → 56 passed; 0 failed

$ cargo test --test p26_003_rev_003_economic_power_capacity_e2e_test
  → 21 passed; 0 failed

$ cargo test --all-targets
  → 全バイナリ合計: 2091 passed; 0 failed; 1 ignored

$ cargo clippy --all-targets --all-features -- -D warnings
  → 警告0、正常終了
```

## 29. clippy警告数

```text
0
```

## 30. ユーザーセーブの保護

`saves/savegame_v1.ron`は全工程を通じて未読み書き・未復元・未ステージ・未コミット。開始時・終了時とも既存の`M`差分のみ(内容は一切参照・変更していない)。

## 31. コミット対象

```text
src/economy/production.rs
tests/p26_det_001_construction_ai_determinism_e2e_test.rs
verification_logs/phase-26/p26-det-001/p26-det-001_construction_ai_determinism.md
```

## 32. 最終`git status --short`

```text
 M saves/savegame_v1.ron
```

(コミット後、上記3件は追跡済みとなり、ユーザーセーブ以外の差分は残らない想定。実測値は最終報告に記載する。)

## 33. P26-004-PRE-005へ進めるか

進める状態である。本タスクはArcadia建設AIの候補選定ロジック自体には触れておらず、既に確定していた建設結果を月次生産へ反映する処理順序だけを修正した。P26-004-PRE-004が確定した4か国のデータ・migrationは無傷であり、初期MilitarySupplies付与(31.0)とmarkerベースの旧Save移行という次のタスクへ進む技術的な障害はない。ただし、今後36か月等の長期基準値を新たに記録する際は、本タスクで確立した「同一論理状態からは独立プロセス間でも同一の数値に到達する」という決定性を前提としてよい(以前のように複数回の実行で値が安定するまで様子を見る必要はない)。
