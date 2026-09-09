# P26-007: 国家消滅・占領・海外領土のモデル2版補給会計整理

## 1. 開始状態

- 開始HEAD: `7523286f990d022539073b3a0112e5fac154802d`(`fix: preserve AI reevaluation state across saves` = P26-006-REV-001の成果)
- 開始時ベースライン: `cargo test --all-targets` = 2337 passed / 0 failed / 1 ignored
- 開始時clippy: `cargo clippy --all-targets --all-features -- -D warnings` = 0 warnings
- 本タスクはこのベースラインの上に、新規テストファイル1件のみを追加してクローズした(後述、実装コード変更は不要と判断)。

## 2. タスク種別の結論

**このタスクは実装修正ではなく、既存実装の契約確認とテストによる固定(ロック)である。**

§0で列挙された10個のコア方針(national stockpileは国家消滅・占領で移転しない、Divisionは自国owner stockpileのみ使用、占領は補給接続性のみに影響、首都陥落時に代替補給源は生成されない、海外飛び地は陸路連結がなければ補給不能、消滅国家の備蓄は凍結、Division携行在庫の返却なし、当日確定分は消滅で巻き戻さない、Save V1保持、港湾等は対象外)は、コードレベルの精査により**すべて既存実装で満たされていた**ことを確認した。したがって §24の指示に従い、コミットメッセージは `test: lock model 2 supply accounting boundaries` を採用する。

## 3. 監査対象と確認結果(§3〜§14対応)

### 3.1 国家消滅(`src/country/elimination.rs`)

- `apply_country_elimination`は`CountryData.status`を`CountryStatus::Eliminated{eliminated_date, annexed_by}`に変更するのみで、`CountryData`を`CountryRegistry`から物理削除しない(P21-021の既存設計)。
- 消滅国家に所属する全Divisionは`military_registry.remove_division(division_id)`で除去される(elimination.rs:417付近)。`remove_division`は`divisions`と`division_supply_fulfillment`の両方から該当エントリを削除するため(`src/military/data.rs:175-181`)、danglingな補給率エントリは残らない。
- 既存テスト`apply_destroys_all_equipment_of_disbanded_divisions_and_leaves_stockpiles_untouched`が、消滅国家のstockpileも併合国のstockpileも一切変化しないことを既に固定している。`stockpile`という語自体が`apply_country_elimination`の本体コード中に一切出現しない(grep実測)。
- 呼び出し元は`src/war/peace.rs:531`と`src/diplomacy/crisis_response.rs:204`の2箇所のみで、いずれも`state::ownership::preflight_transfer_state_ownership`が`OwnershipTransferError::LastOwnedLandState`を返した場合の同期的フォールバックとして呼ばれる(バッチスキャン方式ではない)。

### 3.2 日次補給(`src/military/daily_supply.rs`)

- `run_daily_military_supply_consumption`は`country_registry.countries`からCountryId昇順にソートしたVecを反復し(388-389行)、`!country.is_active() { continue; }`(395行)で消滅国家を除外する。
- `country.current_capital_state_id()`は消滅国家に対して`None`を返す設計だが、コードは`is_active()`チェックの後にのみ到達するため、実質的に到達しない安全側フォールバック(空集合)として書かれている。
- `plan_country_daily_supply`内部はDivisionId昇順でソート済みの`ids`から`per_division`を構築し、`eligible`/`disconnected`への分類もそのソート順を保ったまま行われる(144-224行)。Hamilton配分(`allocate_by_largest_remainder`)も剰余降順→DivisionId昇順のタイブレークで完全に決定的(既存の単体テストで保証)。

### 3.3 経済生産(`src/economy/mod.rs`)

- `handle_monthly_economy`は`country.owned_states`を`owner_country_id == country_id`でフィルタし、空なら`continue`する(126-128行)。消滅国家は定義上所有州0であるため、明示的な`is_active()`チェックを追加する必要がない — 既存のフィルタが自動的に生産をゼロにする。
- 占領(controller変更)はowner_country_idを変えないため、占領された州でも「占領された側の国」の生産対象州リストには残り続ける。これは§6/§9の要求(占領は補給接続性のみに影響し、生産・所有権には影響しない)と一致する。

### 3.4 領有権移転・和平(`src/state/ownership.rs`, `src/war/peace.rs`)

- 両ファイルを`grep -c stockpile`した結果、出現数0。州の占領・割譲・完全併合のいずれの経路もMilitarySuppliesを一切参照しない。

### 3.5 Save/Load(`src/save/dto.rs`, `src/save/apply.rs`, `src/save/validate.rs`)

- `SavedCountryRegistry.countries: Vec<CountryData>`は`CountryData`を直接使うため、`CountryStatus::Eliminated{...}`と凍結された`stockpile`はserde deriveでそのまま往復する。特別分岐は不要かつ存在しない。
- `division_supply_fulfillment`はP26-004で導入されたdangling参照チェック(`src/save/validate.rs`)が既にあり、旧SaveにフィールドがなくてもDefaultで空Mapとして読み込まれ、翌日の日次補給処理で正しく再計算される(本タスクのTestで再確認、§5参照)。
- 首都連結性自体はP26-006の方針通りSaveに一切persistされず、load後も`connected_states_from_capital`で毎日再計算される。

### 3.6 同日決定性(§13)

- `run_daily_military_supply_consumption`(CountryId昇順)、`plan_country_daily_supply`(DivisionId昇順)、`allocate_by_largest_remainder`(剰余降順→DivisionId昇順)は全て既存のソート済み処理であることをコードで再確認した。
- 国家消滅のトリガー自体はイベント駆動の同期呼び出しであり、複数国家が「同じHashMapを反復して」消滅させられるバッチ処理は存在しないため、この経路にHashMap順序非決定性のリスクはない。
- 本タスクのTest Oで、Arcadia→Elfin消滅処理順とElfin→Arcadia消滅処理順が同じ最終状態(生存フラグ・両国のstockpile)に到達することを直接検証した(挿入順序非依存)。

### 3.7 Division除去経路の網羅性

`remove_division`の呼び出し元をgrepで確認: `src/military/army.rs`(復員)、`src/military/update.rs:430,511`(戦闘全滅・強制除去)、`src/country/elimination.rs`(国家消滅)。全経路が同一の安全なクリーンアップ(`divisions`と`division_supply_fulfillment`の両方から削除)を通る。`src/military/data.rs`自体には国籍・消滅・占領に関するロジックは一切存在しない(grep実測でヒット0件)ため、低レベルデータコンテナとしての責務分離が保たれている。

### 3.8 Systemスケジューリング(§4)

- `DailySimulationSet`の順序は`TimeUpdate → Economy → Research → Diplomacy → CountryAi → WarPreparation → MilitaryAi → FrontlineOrders → MilitaryAction → WarResolution → UiUpdate`(既存、変更なし)。
- 国家消滅を引き起こす`execute_peace_settlement`(`war/peace.rs`)と`accept_demand`/`reject_demand`(`diplomacy/crisis_response.rs`)は、プレイヤー操作またはAI意思決定によって呼ばれるDomain API関数であり、日次Economy Set内の日次補給消費(`process_daily_military_supply_consumption`)より後(WarPreparation以降、あるいはUI操作起点)に実行される。したがって、その日確定した補給消費が消滅処理によって巻き戻されることは構造的になく、変更(占領・消滅)は次の日のEconomy Setで初めて補給計算に反映される。`src/app/`にこの順序に反する記述は見つからなかった。

## 4. 上記監査の結論: 実装ギャップなし

§3の全項目にわたり、既存実装が§0〜§14の要求を既に満たしていることを確認した。したがって本タスクでは**プロダクションコードの変更を一切行っていない**。変更されたのは新規テストファイル1件のみである。

## 5. 追加した検証(新規E2Eテストファイル)

`tests/p26_007_elimination_occupation_overseas_supply_accounting_e2e_test.rs`(23テスト、Test A〜T相当+2件)を新規作成した。

| # | テスト関数 | 対応する要求 |
|---|---|---|
| A | `test_a_occupied_enemy_state_is_traversable_via_controller_not_owner` | 占領した敵州(owner不変)への連結 |
| B | `test_b_own_state_occupied_by_enemy_is_not_traversable` | 自国州が敵占領されると通行不能 |
| C | `test_c_capital_occupation_halts_consumption_without_touching_stockpile_while_country_survives` | 首都占領→0充足・0消費(存続国家) |
| D | `test_d_capital_recapture_restores_connectivity_and_consumption_the_next_day` | 首都奪還→翌日復帰 |
| E | `test_e_overseas_territory_across_a_sea_state_is_unreachable_even_when_self_owned_and_controlled` | 海外領土は自国owner/controllerでも補給不能(合成フィクスチャ) |
| F | `test_f_sea_state_is_never_traversable_even_when_self_owned_and_self_controlled` | is_sea州自体が常に通行不能 |
| F2(補助) | `test_f2_land_locked_enclave_cut_off_by_a_foreign_controlled_corridor_is_unreachable` | 陸上飛び地が第三国支配の回廊で切断される追加境界 |
| G | `test_g_state_controller_change_alone_does_not_transfer_military_supplies_between_countries` | 占領だけでは備蓄が移転しない |
| H | `test_h_peace_style_owner_change_does_not_transfer_military_supplies_between_countries` | 割譲だけでは備蓄が移転しない |
| I | `test_i_same_day_completed_consumption_is_not_rolled_back_by_a_later_elimination` | 当日確定消費は消滅で巻き戻されない |
| J | `test_j_no_production_consumption_or_allocation_the_day_after_elimination` | 消滅翌日以降は生産・消費・配分ゼロ |
| K | `test_k_full_annexation_never_transfers_the_eliminated_countrys_stockpile_to_the_annexer` | 完全併合でも備蓄は移転しない |
| L | `test_l_division_annihilation_returns_no_supplies_and_leaves_no_dangling_fulfillment_entry` | Division全滅で返却なし・dangling entryなし |
| M | `test_m_division_demobilization_has_the_same_no_refund_no_dangling_contract` | Division解散も同じ契約 |
| N | `test_n_zero_demand_division_defaults_stay_correct_across_connectivity_boundaries` | 需要0 Divisionの境界でdefault=1.0が保たれる |
| O | `test_o_multiple_simultaneous_eliminations_are_order_independent` | 複数国家同日消滅の処理順序非依存 |
| P | `test_p_per_country_conservation_law_holds_through_a_disconnection_and_elimination_sequence` | 国別保存則 |
| Q | `test_q_world_conservation_law_no_phantom_creation_or_destruction_across_elimination` | 世界保存則 |
| R | `test_r_save_load_branch_matches_continuous_run_across_disconnection_and_elimination` | save/load分岐が連続プレイと一致 |
| S | `test_s_double_save_load_does_not_double_apply_supply_consumption_or_elimination_cleanup` | 2回連続save/loadで二重適用なし |
| 補助 | `test_old_save_without_fulfillment_field_still_handles_a_disconnected_scenario_safely` | 旧Save(フィールド欠落)の安全な取り扱い |
| T | `test_t_controller_change_and_elimination_are_reflected_starting_only_the_next_supply_pass` | 実App System順序で「翌日反映」を確認 |
| 90日 | `test_90_day_corridor_and_capital_occupation_scenario_with_save_load_branch` | 90日実戦争シナリオ(§17) |

### テスト構築方針

- `country::elimination::apply_country_elimination`(14引数、`CountryEliminationPlan`の事前構築が必要)は複雑すぎるため、本タスクの新規テストでは直接呼び出さなかった。elimination自体の内部ロジック(原子性・戦争/外交整理等)は既存の`src/country/elimination.rs`内テスト(20件超)がすでに検証済みである。
- 代わりに`eliminate_country_directly`という**テスト専用ヘルパー**を実装し、「消滅の観測可能な結果」(`CountryData.status`変更 + 所有Division全除去)だけを直接再現して、**その下流の補給会計**を検証した。これはP26-005/006/006-REV-001で確立した`set_state_controller`(直接フィールド操作でDomain APIを迂回する)と同じ検証方針であり、新しい手法ではない。
- Test E/F(海外領土・海上州)は実マップに`is_sea: true`の州が0件であるため(P26-006で確認済みの事実、再確認済み)、P26-006自身の前例に従い`StateRegistry::build(...)`による合成フィクスチャを使用した。

## 6. 90日戦争シナリオ検証(§17)

Arcadia(陸上連結国家、実マップ)を用い、以下のシナリオを継続プレイ版とsave/load分岐版の両方で90日間実行した:
- day1-9: 通常運用
- day10: 回廊州(StateId(11))をElfinが占領
- day20: 首都(StateId(0))をElfinが占領
- 分岐版はmonth2 day10でsave→load

結果:
- 継続版とsave/load分岐版は90日分の(stockpile, is_active, capital_division_fulfillment)ログが完全一致(`assert_eq!`で直接比較)。
- day20の首都占領以降、備蓄は**減少しない**(単調非減少)ことを確認した。これは「占領後は消費ゼロだが生産(owner_country_idベース)は占領の影響を受けず継続する」という正しい会計モデルの結果であり、当初「変化しない」という誤った仮定で書いたアサーションを実測値(day20時点11.0 → day90時点53.079133)を見て修正した。
- day90時点で首都に取り残されたDivisionの補給充足率は0.0のまま。
- Arcadiaは本シナリオ中消滅しないため「annexerへの備蓄移転」はこのシナリオでは非該当だが、Test K/Qで別途、完全消滅シナリオにおける「annexerのstockpileが増加しない」ことを直接的に0との比較で確認済み。

## 7. 保存則の検証

- **国家別保存則(P)**: 切断→首都占領→消滅の系列を通し、消費が発生しない期間は備蓄が不変であること、消滅処理自体も備蓄を凍結するだけで変化させないことを確認。
- **世界保存則(Q)**: 消滅処理前後で全6か国の備蓄総和が完全一致(浮動小数点の`==`比較、加算や移転操作を一切経由しないため誤差も発生しない)。

## 8. Save V1の扱い

- **Save V1 PRESERVED**(変更なし)。`SavedCountryRegistry`, `SavedMilitaryRegistry`(`division_supply_fulfillment`含む)のいずれのフィールドも追加・削除・型変更していない。
- 首都連結性は今回も一切persistしない(P26-006の方針を継続)。
- 旧Save(`division_supply_fulfillment`欠落)を読み込んでも、load直後はDefaultの1.0として扱われ、load後最初の日次補給処理で正しい値に上書きされることをテストで再確認した(`test_old_save_without_fulfillment_field_still_handles_a_disconnected_scenario_safely`)。

## 9. 回帰保護の確認結果(§18)

全て個別に実行し、以下の通り全件パスした(コマンドは§10参照)。

| 対象 | 結果 |
|---|---|
| P26-004(`p26_004_daily_supply_consumption_allocation_e2e_test`ほかPRE系) | 全パス |
| P26-005(`p26_005_supply_combat_strength_e2e_test`) | 全パス(10件) |
| P26-006(`p26_006_capital_connectivity_supply_e2e_test`) | 全パス(18件) |
| P26-006-REV-001(`p26_006_rev_001_ai_save_resume_determinism_e2e_test`) | 全パス(9件、20回反復テスト含む) |
| P26-DET-001/REV-001 | 全パス(10件+12件) |
| Phase 23(`p23_010`、5回独立実行) | 5/5パス |
| Phase 24(`p24_002_workforce_rounding_unification_e2e_test`) | 全パス(17件) |
| Phase 25(equipment系7ファイル) | 全パス(15+6+22+5+10+29+8+21件) |
| Save V1(New/Loaded Game) | `--lib`と`--all-targets`内のsave関連テストで確認、全パス |
| `tests/land_war_combat_peace_test.rs` | 変更なし、4件全パス(保護対象、読み取りのみ) |

## 10. 実行したコマンドと結果

全て逐次実行(並列cargo実行なし)。

```
cargo test --test p26_007_elimination_occupation_overseas_supply_accounting_e2e_test
  → 23 passed; 0 failed (×5回独立プロセス実行、全て23 passed)

cargo clippy --test p26_007_elimination_occupation_overseas_supply_accounting_e2e_test --all-features -- -D warnings
  → 0 warnings(1件のcollapsible_if検出→修正後 再実行で0 warnings)

cargo test --test p26_002_... --test p26_003_... (Phase26関連13ファイル一括)
  → 全パス

cargo test --test p24_002_... --test p25_002_... ... --test land_war_combat_peace_test
  → 全パス

cargo test --lib
  → 1554 passed; 0 failed; 0 ignored

cargo test --all-targets
  → 2360 passed; 0 failed; 1 ignored (ベースライン2337 + 新規23 = 2360、完全一致)

cargo clippy --all-targets --all-features -- -D warnings
  → 0 warnings

p23_010を独立プロセスで5回実行 → 5/5 stable (9 passed each)
p26_007新規テストファイルを独立プロセスで5回実行 → 5/5 stable (23 passed each)
```

## 11. 見つかった問題(Critical/High/Medium/Low)

- **Critical: 0件**
- **High: 0件**
- **Medium: 0件**
- **Low: 0件**(新規)

本タスクで新たに見つかった実装ギャップ・バグは0件だった。§0〜§14で要求された契約は、P21-021(国家消滅Domain API)・P26-004(日次補給)・P26-005(戦闘力)・P26-006(首都連結性)・P26-006-REV-001(決定性修正)の積み重ねによって、この時点で既に完全に満たされていた。

§19で明示された既知のLow finding(`mark_ai_registries_dirty_daily`によるAI週次/月次段階分けの実質無効化)には一切触れていない(変更・削除・最適化のいずれも行っていない)。

## 12. 除外事項の確認

以下は本タスクで一切実装していない(§2の除外事項通り):
- 港湾・海軍輸送・輸送船・鉄道・補給デポ
- 首都移転・代替首都
- 同盟通行権・軍事通行権
- 補給範囲・距離
- Division携行在庫・戦利品・備蓄鹵獲
- 移動速度修正・Organization・Attrition
- UI変更
- P26-008(AI連結性対応)

## 13. PNG/ユーザーSaveの取り扱い

テスト実行により以下が再生成された(既知の非決定的スクリーンショット + ユーザーのSaveファイル):
- `saves/savegame_v1.ron`
- `verification_logs/p20-007/screenshots/03_playing_topbar.png`
- `verification_logs/p20-009/screenshots/{04,05,06}_*.png`
- `verification_logs/phase-21/p21-save-002e/screenshots/{01,02,03,04,05}_*.png`

全て`git checkout --`で個別に(ブロードなcheckoutは使わず、ファイル名を1件ずつ列挙して)復元した。復元後`git status --short`は新規テストファイル1件のみを示す。

## 14. コミット内容

以下の1ファイルのみをステージしてコミットする:
- `tests/p26_007_elimination_occupation_overseas_supply_accounting_e2e_test.rs`(新規)
- 本レポート `verification_logs/phase-26/p26-007/p26-007_elimination_occupation_overseas_supply_accounting.md`(新規)

プロダクションコード(`src/`)の変更は0ファイル。

## 15. 最終検証(コミット前)

- `git status --short` → 上記2ファイルの新規追加のみ
- `git diff --check` → 該当なし(新規ファイルのみ)
- ユーザーSave(`saves/savegame_v1.ron`)・非決定的PNGはステージ対象に含まれない

## 16. 判定

- **P26-007 COMPLETE**
- **MODEL-2 ACCOUNTING COMPLETE**
- **ELIMINATION ACCOUNTING VALID**
- **OCCUPATION ACCOUNTING VALID**
- **OVERSEAS RULE VALID**
- **SAVE V1 PRESERVED**
- **P26-008 READY**

本タスクはモデル3の海上・多拠点補給を実装するものではない。モデル2の境界(国家消滅・占領・併合・海外領土)における補給会計が、既存実装によって既に決定的かつ保存則に従っていることをコード監査で確認し、23件の新規E2Eテストでその契約を恒久的に固定した。
