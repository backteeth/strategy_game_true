# P26-DET-001-REV-001: 州をまたぐ国家生産順序の決定性監査・訂正

## 1. 総合判定

`COMPLETE`

州間の最小反例(スペックCase 1)は実関数(`process_country_production`)で実際に再現することを本タスクで確認した(§6・§7)。根本原因はP26-DET-001と全く同じメカニズム(`CountryStockpile::consume`の現在庫との厳格な全額判定)が州境界をまたいでも成立することにあり(§8)、国家全体の生産順序を`BuildingType::ALL`(外側)→`StateId`昇順(内側)という明示的な全順序へ修正した(§9)。

## 2. P26-DET-001の最終的な扱い

P26-DET-001(州内、`BuildingType::ALL`宣言順の導入)はそのまま有効。本タスクはそのループ構造をさらに一段階外側(州の処理順)まで拡張したものであり、P26-DET-001が導入した州内の`BuildingType::ALL`順自体は変更していない(`BuildingType::ALL`の宣言順そのものは無変更)。P26-DET-001が確定させたArcadia月12・月18の正規値(421.2265230000 / 592.5978000000)は本タスク後も不変。ただし月24・月36の値は、本タスクのループ構造変更(州外側→BuildingType外側への反転)自体が州をまたぐ`consume`/`add`の実行順序を変えたため、新しい値(667.8538710000 / 822.4577790000)に変化した(§16で詳述、退行ではなく意図した修正の結果)。

## 3. 開始時HEAD

```text
72757dbd9c7f75779d4342ad12b5fd5135dac4f5
```

開始時`git status --short`は`M saves/savegame_v1.ron`のみで、スペック記載の想定と一致していた。

## 4. 監査対象コードHEAD

`72757dbd9c7f75779d4342ad12b5fd5135dac4f5`(調査開始時点、P26-DET-001適用済みの`src/economy/production.rs`)。

## 5. 現行ループ構造

調査開始時点の`process_country_production`(`src/economy/production.rs`)は、Step 1・Step 3とも次の構造だった(候補A):

```text
Step 1: for state in states.iter() {              // ← states外側(caller順)
            for b_type in BuildingType::ALL { ... } // ← BuildingType内側(既にP26-DET-001が全順序化)
        }

Step 3: for state in states.iter_mut() {           // ← states外側(caller順)
            for b_type in BuildingType::ALL { ... } // ← BuildingType内側
            state.building_operation_ratios = op_ratios;
        }
```

呼び出し元`handle_monthly_economy`(`src/economy/mod.rs`116〜153行)は次のように`owned_states`を構築する:

```rust
let mut owned_states: Vec<&mut StateData> = state_registry
    .states
    .iter_mut()
    .filter(|s| s.owner_country_id == country_id)
    .collect();
```

`state_registry.states`(`Vec<StateData>`)は`StateRegistry::build`(`src/state/data.rs`)がpushされた順をそのまま保持する。`StateRegistry::build`自体は値のサニタイズと`index_map`(O(1)検索用)の構築のみを行い、`Vec`の並び替えは一切行わない(確認済み)。したがって`owned_states`の順序は、実質的に`assets/data/states.ron`のファイル記載順そのものである。

`assets/data/states.ron`を実際に確認したところ、国家ごとの物語的グルーピング(王国Arcadia本体→Elfin→Dwarf→Oceanic→「Arcadia追加州」→「Elfin追加州」→…→Passguard→Ferrowyn、という順)であり、StateId昇順ではない。例えばArcadiaの所有州は本体(id 0,1,2)と「追加州」(id 11,12)がファイル内で大きく離れた位置に出現する。国家によっては(例:Oceanic)本体の州と追加州のStateIdの大小関係が入り組んでいる。この順序はゲームデザイン上の意図された順序ではなく、単にファイル編集時の追記履歴を反映した恣意的な順序であることを確認した。

すなわち現行構造は「候補A(Country→State→BuildingType)」であり、州の並びはStateId順で保証されていなかった。

## 6. 州間最小反例

`tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs`の`test_a_cross_state_minimal_counterexample_insertion_order_does_not_change_result`として実装。P26-DET-001報告書§8が実際に非決定性を発現させたのと同じ境界残高(Iron=170.0, Coal=198.0, IndustrialGoods=0.5025064945)を使い、State 10(Factory level2のみ)とState 20(MilitaryFactory level1のみ)を別々の州に分離し、`states`スライスへの挿入順を反転させた(Run A: State10→State20、Run B: State20→State10)。

修正前のコード(候補A)で実際に実行した結果、両ランで異なる値を観測した(本タスクの開発過程での実測、修正前コードに対する実行ログ):

```text
Run A (Factory州を先に挿入): IndustrialGoods = 45.29374295843418
Run B (MilitaryFactory州を先に挿入): IndustrialGoods = 45.957051515557126
```

`test_d_state_id_order_does_not_override_building_type_dependency_order`では、MilitaryFactoryをStateId10(小)、FactoryをStateId20(大)に配置し、単純なStateId全体ソートだけでは不十分であること(依存順がStateIdより優先されるべきこと)も確認した。

## 7. 修正前結果

上記§6の反例を、修正前のコード(調査開始時点のHEAD)に対して実際に`cargo test`で実行し、以下を確認した:

```text
test_a_cross_state_minimal_counterexample_insertion_order_does_not_change_result ... FAILED
  assertion `left == right` failed: resource IndustrialGoods must match regardless of
  cross-state insertion order (Run A got 45.29374295843418, Run B got 45.957051515557126)

test_d_state_id_order_does_not_override_building_type_dependency_order ... FAILED
  assertion `left == right` failed: result must be independent of slice insertion order
  once canonical ordering is applied
  left: 45.957051515557126
  right: 45.29374295843418

test_b_three_stage_supply_chain_all_six_permutations_match ... FAILED
  (3州構成でも、挿入順によって依存連鎖(Mine→Factory→MilitaryFactory)が成立したり
   しなかったりすることを別の形で確認)
```

以上により、州間の反例が理論上の懸念ではなく実関数で実際に再現する非決定性であることを確認した。

## 8. 根本原因

P26-DET-001の根本原因(`CountryStockpile::consume`——`src/economy/resources.rs`82〜90行——が現在庫との厳格な全額判定・部分消費なしであること)が、州境界をまたいでも全く同じ形で成立する。

```rust
pub fn consume(&mut self, resource: ResourceType, amount: f64) -> bool {
    let current = self.get(resource);
    if current >= amount {
        self.amounts.insert(resource, current - amount);
        true
    } else {
        false
    }
}
```

重要な追加知見(P26-DET-001の州内ケースと共通するが、本タスクで明示的に確認した点): Step 2で計算される`resource_satisfaction`(充足率)は月初の1回だけのスナップショットであり、Step 3の途中で他州・他建物の同月産出を反映して再計算されることはない。したがって`final_op`(operation ratio)自体は処理順に依存しない——ある州のFactoryが同月にどれだけIndustrialGoodsを産出しても、その月のうちに別州のMilitaryFactoryの`final_op`が引き上がることはない。

非決定性が生じるのは、`final_op`から導出された`actual_in`を実際に`stockpile.consume()`へ渡す際、その呼び出し時点の**実時間の**(月初スナップショットではない)備蓄残高に対して全額判定が行われる一点である。ある州のFactoryの産出(`stockpile.add`)が、別の州のMilitaryFactoryの消費判定(`stockpile.consume`)より**先に**実行されていれば、消費判定は既に加算済みの備蓄に対して行われ(成功しやすい)、**後に**実行されていれば、まだ加算されていない備蓄に対して行われる(失敗しやすい)。`consume`の戻り値(成否)は捨てられており生産量自体はゲートされないため、観測される差は「本来消費されるべきだった量が実際に備蓄から差し引かれたかどうか」という会計上の差(=最終的なIndustrialGoods備蓄の差)として現れる。

この処理順序が、州の処理順(=呼び出し元が渡した`states`スライスの要素順=`assets/data/states.ron`のファイル記載順という恣意的だが同一プロセス内では固定の順序)に依存していた。ファイル順は決定的(常に同じ)であるため、これはP26-DET-001のようなプロセスごとのHashMapシード依存ではなく、**同一プロセス内では再現するが、コードのループ構造上「意味が明示的に定義されていない」順序**という性質の非決定性である。したがって「起動するたびに違う値になる」わけではないが、「なぜこの順序でなければならないのか」という根拠がコード上どこにも存在せず、ファイルの編集履歴(narrative grouping)が生産結果を左右してしまっていた。

## 9. 採用した国家内生産順

`src/economy/production.rs::process_country_production`のStep 1・Step 3を、次の全順序へ変更した:

```text
for building_type in BuildingType::ALL:      // 外側: 産業依存順
    for state in owned_states sorted by StateId:  // 内側: 永続ID昇順
        process(state, building_type)
```

具体的には:

* 関数冒頭で`states`スライスの添字を`state.id.0`(`StateId`のusize値)昇順にソートした`order: Vec<usize>`を1度だけ構築し、Step 1・Step 3の両方でこの`order`を反復に使う。
* Step 1(入力資源需要集計)は`for &i in &order { let state = &*states[i]; for b_type in BuildingType::ALL { ... } }`という State外側/BuildingType内側の構造を維持しつつ、State側の反復順を`order`(StateId昇順)に固定した(集計自体は交換法則を満たす単純な合計だが、浮動小数点加算は結合則を満たさないため、順序を明示的に固定することで極小さな丸め差の余地も閉じた)。
* Step 3(稼働率決定・消費・生産)は、ループ構造そのものを`for b_type in BuildingType::ALL { for &i in &order { ... } }`(BuildingType外側/State内側)へ反転させた。これにより、依存順で先に処理されるべきBuildingType(例:Mine)は、どの州にあるかに関わらず全州分先に処理されてから、次のBuildingType(例:Factory)が全州分処理される。
* 各州の`building_operation_ratios`は、ループ構造の反転により州ごとに閉じたループの末尾で書き戻せなくなったため、`op_ratios_per_state: Vec<HashMap<BuildingType, f32>>`(states配列と同じ長さ、添字対応)に一時的に蓄積し、Step 3の全BuildingType処理が終わった後に1回だけ各州へ`std::mem::take`で書き戻す。書き込まれる内容自体は変更前と同一(各BuildingTypeについて、その州がその建物を保有していれば`final_op`または`0.0`を記録し、保有していなければ一切キーを追加しない)。

数値仕様(生産量の式、operation ratio算出式、入力消費量の式、fixed-point scale、丸め方式)は一切変更していない。変更したのは処理順序のみである。

## 10. BuildingType順

`BuildingType::ALL`(`src/building/data.rs`)の宣言順は`Farm, LoggingCamp, Mine, Factory, MilitaryFactory, Railway, University, MagicAcademy, CrystalMine, CrystalRefinery`であり、本タスクで確認すべき依存関係(`Mine → Factory → MilitaryFactory`、`CrystalMine → CrystalRefinery`)の順序と一致していることを確認した。この列挙順自体は、UI表示順(P20-009)と生産処理順を兼ねている既存の設計であり、本タスクでは変更していない(生産専用の別順序定数を新設する必要はなかった)。

## 11. StateId tie-break

同じBuildingType内の複数州間のtie-breakとして、永続的な`StateId`(`state.id.0`、`usize`)の昇順を採用した。Bevy Entity ID、メモリアドレス、HashMap内部順、生成順のいずれにも依存しない。`test_c_same_building_type_scarcity_prefers_lower_state_id`で、2州が同じBuildingTypeで不足資源を競合する場合でも、挿入順(スライスへの渡し方)に関わらず結果が一致することを確認した。

## 12. ResourceType Map順監査

`total_inputs_needed`・`resource_satisfaction`・`def.input_resources`・`def.output_resources`はいずれも`HashMap<ResourceType, f64>`だが、以下の構造的理由により、これらのMap反復順は結果に影響しないことを確認した:

* Step 1・Step 2は、個々の`ResourceType`ごとに独立した集計・比率計算であり、他の`ResourceType`のエントリと相互作用しない(単純な合計・独立した`avail/needed`比率)。
* Step 3の`min_res_sat`算出は`def.input_resources.keys()`を反復して`min`を取るだけであり、`min`は交換法則を満たす。
* Step 3の消費ループ(`for (&res, &base_in) in &def.input_resources`)は、1つの建物につき複数の異なる`ResourceType`をそれぞれ独立に`consume`する。同じ建物内で複数の入力資源が互いに生産・消費の関係を持つことはない(ある建物の入力資源同士が互いを産出することはない)ため、州間・BuildingType間で見られたような「消費側が先か生産側が先か」という順序依存の余地が構造的に存在しない。

`test_f_resource_type_map_insertion_order_independence`で、`input_resources`/`output_resources`への挿入順を変えても結果が一致することを直接確認した。このため、`ResourceType::ALL`による明示的な全順序化は本タスクでは必要ないと判断した(コード変更なし)。

## 13. 同月中間財の意味

Step 2の`resource_satisfaction`は月初の1回だけ計算されるスナップショットであり、Step 3の途中で他州・他BuildingTypeの同月産出を反映して再計算されることはない。したがって、「同月内の中間財」が意味を持つのは**operation ratio(稼働率)そのものの引き上げ**としてではなく、**既に(月初スナップショットに基づいて)決定された消費量が、実際に備蓄から差し引かれるかどうか**という一点だけである。本タスクの正規順序(`BuildingType::ALL`外側→`StateId`昇順内側)の下では、依存順で先に処理されるBuildingType(例:Factory)の同月産出が、全州のFactory処理が完了した後に評価される後続BuildingType(例:MilitaryFactory)の消費判定に対して、常に「既に加算済み」の状態で反映される。この規則は全州で統一されており、どの州のペアであってもこの規則が適用される。

## 14. 同種建物不足時の意味

同じBuildingType内で複数州が同じ資源(例:IndustrialGoods)の不足を巡って競合する場合、`StateId`昇順の州が先に処理される。`final_op`自体は全州共通の月初充足率スナップショットに基づいて計算されるため、複数州が同じ理論上の`actual_in`を要求する状況では、`StateId`が小さい州の`consume`呼び出しが先に実時間の備蓄へアクセスし、それ以降の州の`consume`はより減少した実時間残高に対して判定される。すなわち、不足時には`StateId`昇順の州が実際の消費(会計上の差し引き)において優先される。`test_c_same_building_type_scarcity_prefers_lower_state_id`で、この規則が挿入順に依存せず安定していることを確認した。

## 15. 世界資源差分

`process_country_production`は国家ごとに独立した`&mut CountryStockpile`を受け取り、国家をまたいだ状態の共有は一切行わない(構造上、国家間分離は既存のまま)。`test_g_cross_country_isolation_processing_order_does_not_matter`で、2つの独立した国家をどちらの呼び出し順で処理しても、各国家自身の最終備蓄が変わらないことを確認した。生産・消費される量そのもの(式)は本タスクで変更していないため、世界全体の資源差分(生産量−消費量)は実際の生産・消費内容と一致し続ける。

## 16. Arcadiaの正規値

修正後のコードで、Arcadia(`CountryId(0)`)単独の36か月MilitarySupplies値を実App(`real_app`モジュール、`AppPlugin`一式を使った統合実行)で計測し、以下を正規値として確定した(月12・18はP26-DET-001と不変、月24・36は本タスクのループ構造変更により新しい値に変化——§2参照):

```text
月12: 421.2265230000  (P26-DET-001と同値)
月18: 592.5978000000  (P26-DET-001と同値)
月24: 667.8538710000  (P26-DET-001の667.3241700000から変化)
月36: 822.4577790000  (P26-DET-001の823.1722540000から変化)
```

この値は、修正後コードを別プロセスとして5回逐次実行し、5回とも同一の値であることを確認した上で採用した(§18)。既存の`tests/p26_det_001_construction_ai_determinism_e2e_test.rs`の`ARCADIA_CANONICAL_M24`/`ARCADIA_CANONICAL_M36`定数も、この新しい正規値へ更新した(月12・18は不変のため変更なし)。

## 17. 6国家36か月結果

`test_i_all_six_countries_stable_within_process_and_no_regression`(既存のP26-DET-001 Test Fと同じ形式、許容誤差1.0)で、Elfin(1846.952)・Dwarf(1487.059)・Oceanic(565.705)・Passguard(3325.079)・Ferrowyn(1089.859)がいずれもP26-DET-001時点の基準値と一致することを確認した(Arcadiaのみ本タスクの正規値822.4577790000で比較)。これら5か国はArcadiaと異なり、本タスクのループ構造変更による観測可能な影響(許容誤差1.0を超える差)が出ていない——具体的には、これらの国の所有州の並びが、ファイル順とStateId順とで生産処理順に実質的な差を生まない構成になっている(または、生じたとしても許容誤差1.0未満の丸め差にとどまる)ことを意味する。既存の`p26_004_pre_002`・`p26_004_pre_004`テストファイルの6か国36か月スナップショット系テスト(`test_m1_six_country_36_month_snapshot_matches_expected_deposit_aware_outcomes`等)も、本タスクの修正後コードに対して全て成功することを確認した(§24)。

## 18. 独立プロセス5回結果

`cargo test --test p26_det_001_rev_001_cross_state_production_order_e2e_test`(州間最小反例・Arcadia 36か月・6か国36か月を含む全12テストを1つのバイナリで実行)を、**5回、別々のプロセスとして逐次実行**した。

```text
実行1: test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
実行2: test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
実行3: test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
実行4: test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
実行5: test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

加えて、Arcadia単独36か月値についても、診断用出力(月12/18/24/36の実値を`panic!`で出力する一時的な変更を加えて実行後、正規のassertへ戻した)を独立プロセスで5回取得し、全て同一の値(m12=421.2265230000, m18=592.5978000000, m24=667.8538710000, m36=822.4577790000)であることを確認した。並列実行は行わず、全て逐次に独立起動した。

## 19. New Game／Loaded Game比較

`test_j_new_game_vs_loaded_game_match_at_checkpoint`で、New Game経由の4か月連続実行と、save→load経由の分岐実行が、チェックポイント時点(月4)で同一のIndustrialGoods・treasury値に到達することを確認した(許容誤差1e-6)。

## 20. save/load分岐比較

`test_k_save_load_branch_matches_continuous_run_at_month_36`で、修正後の新しい分岐月(月24、§16で確認した月24からの分岐)を跨ぐよう月17時点でsave→loadを挟んだ実行と、継続実行を月36まで比較し、完全一致(許容誤差1e-6)することを確認した。

## 21. P26-004-PRE-004保護

`git diff --stat`で、本タスクが変更したのは`src/economy/production.rs`と`tests/p26_det_001_construction_ai_determinism_e2e_test.rs`(既存テストの正規値更新)、および新規追加した`tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs`とこの報告書のみであることを確認した。`assets/data/states.ron`・`assets/data/resources.ron`・`src/building/supply_chain_migration.rs`はいずれも本タスクで変更していない。Elfin/Ferrowyn/Passguard/Oceanicの供給網データとmigrationロジックは無傷である。`cargo test --test p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test`(15 passed; 0 failed)で回帰がないことを確認した。

## 22. Phase 23～25回帰

`cargo test --lib economy::`(32 passed)・`cargo test --lib country::`(89 passed)を個別実行し、Phase 23〜25関連の会計・国力ロジックへの回帰がないことを確認した。`cargo test --test p26_003_rev_003_economic_power_capacity_e2e_test`(21 passed)でも国力算出ロジックへの回帰がないことを確認した。

## 23. Critical／High／Medium／Low

```text
Critical: 0
High:     0 (根本原因を特定し修正、独立プロセス5回で再現性を確認済み)
Medium:   0
Low:      1 (新規テストTest B[3段階供給網]の当初設計に誤り があった——
             Step 2の充足率が月初スナップショットであり、同月内の産出で
             即座に再計算されないという実際の仕様を誤解し、「初期在庫0から
             Mine→Factory→MilitaryFactoryが同月内で連鎖して産出が出る」
             という誤った前提でテストを書いていた。修正後コードに対して
             実行して初めてこの前提の誤りに気づき、Test Aと同じ境界残高を
             使う形へ設計を修正した。production.rs自体のバグではなく、
             テスト設計時の仕様理解の誤りだった。§9本文および
             `tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs`
             のTest Bコメント参照)
```

## 24. 実行コマンドと正確な結果

すべて逐次実行。

```text
$ cargo test --lib economy::
  → 32 passed; 0 failed

$ cargo test --lib country::
  → 89 passed; 0 failed

$ cargo test --test p26_det_001_construction_ai_determinism_e2e_test
  → 10 passed; 0 failed  (正規値更新後)

$ cargo test --test p26_det_001_rev_001_cross_state_production_order_e2e_test
  → 12 passed; 0 failed  (×5、独立プロセス、全て一致)

$ cargo test --test p26_003_military_supplies_production_e2e_test
  → 49 passed; 0 failed

$ cargo test --test p26_003_rev_002_supply_production_units_e2e_test
  → 56 passed; 0 failed

$ cargo test --test p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test
  → 11 passed; 0 failed

$ cargo test --test p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test
  → 15 passed; 0 failed

$ cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test
  → 45 passed; 0 failed

$ cargo test --test p26_003_rev_003_economic_power_capacity_e2e_test
  → 21 passed; 0 failed

$ cargo test --all-targets
  → 全バイナリ合計: 2103 passed; 0 failed; 1 ignored
    (開始時基準2091 passed + 本タスク新規12 passed = 2103、失敗0、ignore数不変)

$ cargo clippy --all-targets --all-features -- -D warnings
  → 警告0、正常終了
```

## 25. clippy警告数

```text
0
```

## 26. ユーザーセーブの保護

`saves/savegame_v1.ron`は全工程を通じて未読み書き・未復元・未ステージ・未コミット。開始時・終了時とも既存の`M`差分のみ(内容は一切参照・変更していない)。

## 27. コミット対象

```text
src/economy/production.rs
tests/p26_det_001_construction_ai_determinism_e2e_test.rs
tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs
verification_logs/phase-26/p26-det-001-rev-001/p26-det-001-rev-001_cross_state_production_order.md
```

## 28. 最終`git status --short`

```text
 M saves/savegame_v1.ron
```

(コミット後、上記4件は追跡済みとなり、ユーザーセーブ以外の差分は残らない想定。`cargo test --all-targets`実行で一時的に再生成された9件のスクリーンショットPNGは、個別に`git checkout --`で元へ復元済み。)

## 29. P26-004-PRE-005へ進めるか

進める状態である。本タスクは建設AIの候補選定ロジック自体(`country_ai.rs`)には触れておらず、P26-004-PRE-004が確定した4か国(Elfin/Ferrowyn/Passguard/Oceanic)の供給網データ・migrationロジックは無傷である。国家全体の生産処理順序が、州の所有集合(`assets/data/states.ron`のファイル記載順という恣意的な順序)に依存せず、産業依存順(`BuildingType::ALL`)と永続的な`StateId`昇順という2つの明示的な規則だけで決定されるようになったため、今後36か月等の長期基準値を新たに記録する際も、この決定性(独立プロセス間・州の追記順・国家処理順のいずれにも依存しない)を前提としてよい。Arcadiaの月24・月36の正規値がP26-DET-001時点から変化した(§16)ことは、今後この値を参照する既存テスト・報告書がある場合は本報告書の値(667.8538710000 / 822.4577790000)を正としてほしい。
