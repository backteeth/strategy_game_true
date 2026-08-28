# P25-005: 日次装備補充の実装

## 1. 総合判定

**COMPLETE**

## 2. 開始時HEAD

```text
0c57bebfe72d2786aa3f8eb54c945316dada8574
```

## 3. 監査対象コード

実装前に以下を確認した(実関数名・実際の呼び出し順序)。

* `src/military/update.rs::handle_daily_military`(登録済みSystem、`DailySimulationSet::MilitaryAction`)。実際の呼び出し順序: `process_recruitment` → `apply_daily_reinforcement` → (P25-005で追加)`apply_daily_equipment_reinforcement` → `validate_and_stop_invalid_movements` → `process_movement` → `process_daily_battles` → `resolve_finished_battles` → `cleanup_finished_battles` → `process_org_recovery`。
* `src/military/recruitment.rs::process_recruitment`: 完了済みキュー項目を`try_spawn_division`(`try_issue_equipment`使用)でDivision化。装備不足時は項目をキューに残す(`days_remaining=0`のまま)。
* `src/military/reinforcement.rs::apply_daily_reinforcement`/`is_eligible_for_reinforcement`/`apportion_reinforcements`: manpower日次補充の既存実装。`apportion_reinforcements(budget: u64, requests: &[(DivisionId, u64)]) -> HashMap<DivisionId, u64>`は資源に依存しない汎用最大剰余按分関数であることを確認(そのままequipment unitsへ再利用可能)。
* `src/military/update.rs::process_daily_battles`/`resolve_finished_battles`: P25-004で確認済みの稼働中戦闘経路、無変更。
* `CountryStockpile`(`src/economy/resources.rs`): `get`/`set`/`clone`(deriveされたClone)でアクセス。
* Divisionの所有国判定: `Division.owner: CountryId`フィールド、および`StateData.controller()`(占領含む支配国判定)。
* `CountryStatus::Eliminated`: `CountryData::is_active()`が`false`を返す。既存の`apply_reinforcement_for_country`と同じガードを再利用。
* `RecruitmentQueueItem.days_remaining`: `process_recruitment`が`0`に達した項目のみDivision化を試みる。既存フィールド構造は無変更。
* P25-003の装備不足待機処理: `try_spawn_division`が`Err`を返した場合、`completed_indices`へ追加せずキューに残留。
* P25-004の`apply_combat_casualty`(`src/military/update.rs`): 無変更、そのまま呼び出される。
* equipment固定小数点変換: `economy::resources::{military_equipment_value_to_units, military_equipment_units_to_value, MILITARY_EQUIPMENT_SCALE, MILITARY_EQUIPMENT_MAX_UNITS}`、`military::equipment::try_issue_equipment`。
* Save V1: `Division`・`CountryStockpile`・`RecruitmentQueueItem`いずれも既存フィールドのみで、DTO層を経由せず直接保存される(P25-002-REV-001/P25-003で確認済みの構造を再確認)。

## 4. 稼働中のSystem順序

`DailySimulationSet`のチェーン順序(`TimeUpdate → Economy → ... → MilitaryAction → ...`)は無変更。`MilitaryAction`セット内、`handle_daily_military`内部の日次処理順序を以下のとおりにした(既存の1〜2番目は無変更、2.5として新規挿入)。

```text
1. process_recruitment(募兵完了処理)
2. apply_daily_reinforcement(manpower日次補充)
2.5. apply_daily_equipment_reinforcement(★新規: equipment日次補充)
3. validate_and_stop_invalid_movements
4. process_movement
5. process_daily_battles(戦闘処理)
6. resolve_finished_battles
7. cleanup_finished_battles
8. process_org_recovery
```

新しいSystemSetやDayChangedMessage経路は追加していない。全て同一の`handle_daily_military`内のprivate関数呼び出しである。

## 5. 補充対象Division

`src/military/reinforcement.rs`の`is_eligible_for_reinforcement`から、所有・存続(manpower>0)・戦闘状態(Idle/Occupying、`combat_id`なし、進行中戦闘の参加者一覧に含まれない)・支配地域(`state.controller() == division.owner`)という条件を`is_eligible_for_daily_reinforcement_by_position`として`pub(crate)`に切り出し、manpower補充とequipment補充の両方で共有した。manpower補充は従来どおりこれに`manpower < max_manpower`を追加適用し、equipment補充(`is_eligible_for_equipment_reinforcement`)は`max_equipment > 0.0`を追加適用するのみで、manpowerの過不足は一切見ない。

## 6. 日次補充予算

```text
D = Σ_i max(0, max_equipment_i - equipment_i)  (units)
T = min(stockpile_units, D)
```

不足がなくなるか備蓄が尽きるまで1日で支給可能。新しい日次上限定数は導入していない。

## 7. 新しい輸送上限を追加しなかった理由

補給網・輸送能力・修理速度に相当するデータが現状のゲームに一切存在しない(州・国家データに輸送関連フィールドがない)ため、新しい定数を発明すると根拠のない数値になる。本タスクは「装備ストック間の移動」という会計操作として実装し、備蓄全量を1日で使用可能とする暫定仕様(§3で確定済み)をそのまま踏襲した。

## 8. fixed-point unitsの使用箇所

`apply_equipment_reinforcement_for_country`内、以下の全てを正規units(`military_equipment_value_to_units`/`military_equipment_units_to_value`)で実施する。

* 国家備蓄・各Divisionの`equipment`/`max_equipment`のunits変換とvalidation
* 不足量(`shortfall`)の算出(`max_units - equipment_units`)
* `apportion_reinforcements`への`budget`/`requests`(いずれも`u64` units)
* 按分結果を`f64`へ戻すのは`try_issue_equipment`呼び出し直前の1箇所のみ(会計の真実源はunits側)

新しいscale定数はいずれのファイルにも追加していない。

## 9. `apportion_reinforcements`再利用方法

`src/military/reinforcement.rs::apportion_reinforcements(budget: u64, requests: &[(DivisionId, u64)]) -> HashMap<DivisionId, u64>`は資源の種類に依存しない汎用シグネチャのまま`pub`で存在していたため、`equipment_reinforcement.rs`から`use crate::military::reinforcement::apportion_reinforcements;`で直接importして呼び出した。按分の中核ロジックを複製した箇所は一切ない。

## 10. 最大剰余式

既存実装をそのまま使用: `base_i = floor(A×r_i/R)`(`A=min(budget,total_request)`)、`remainder_i = (A×r_i) mod R`、`Σ allocation_i = A`。

## 11. DivisionId tie-break

既存`apportion_reinforcements`の実装どおり、`remainder`降順→`DivisionId`昇順。equipment補充専用の別実装は作っていない。

## 12. HashMap順序非依存

`apply_daily_equipment_reinforcement`は、対象Divisionを`owner`別に`HashMap`へ集約した後、`owners`・各国の`division_ids`をどちらも明示的に`sort_by_key(|c| c.0)`/`sort_by_key(|id| id.0)`でソートしてから処理する(manpower補充と同じパターン)。Test「result_does_not_depend_on_insertion_order」でECS挿入順を入れ替えても結果が一致することを確認した。

## 13. 国家単位の装備補充原子性

`apply_equipment_reinforcement_for_country`は以下の二段階構成:

```text
1. snapshot: country.stockpile.get(MilitaryEquipment)をunits化(不正ならこの国を中断)
2. 全対象Divisionのequipment/max_equipmentをunits化・validation
   (equipment_units > max_unitsなら不正状態としてこの国を中断)
3. apportion_reinforcementsで按分計画
4. country.stockpile.clone()した一時値へ、各Divisionの一時equipmentコピーを
   使ってtry_issue_equipmentを順に適用(事前検証)
5. 全件成功した場合のみ、実CountryStockpileと実Division.equipmentへcommit
```

1つのDivisionまたは国家備蓄が不正状態の場合、その国の備蓄・Divisionのequipment・`max_equipment`はいずれも一切変更されない。Test「invalid_division_equipment_state_aborts_the_whole_country_atomically」「invalid_country_stockpile_aborts_atomically」「one_countrys_abort_does_not_affect_another_country」で確認済み。

## 14. manpowerとequipmentの独立性

`apply_daily_equipment_reinforcement`は`apply_daily_reinforcement`(manpower)と完全に別関数・別呼び出しであり、共有するのは読み取り専用の適格条件関数(§5)と按分関数(§9)のみ。equipment補充が失敗してもmanpower補充をrollbackせず、その逆もない。Test「full_manpower_division_still_receives_equipment」(manpower満員でもequipment補充される)、「zero_equipment_stockpile_does_not_prevent_manpower_reinforcement_call」で確認済み。

## 15. 装備保存則

```text
stockpile_units_before + Σ equipment_units_before
=
stockpile_units_after + Σ equipment_units_after
```

`try_issue_equipment`自体が支給ごとにこの保存則を`debug_assert_eq!`で検証している(P25-002-REV-001から無変更)。Test「stockpile_and_division_equipment_conserve_the_world_total」(単体)、「repeated_days_do_not_drift_the_world_equipment_total」(20日反復、E2E)で確認済み。

## 16. `max_equipment`不変

`apply_equipment_reinforcement_for_country`は`Division.max_equipment`を一切書き込まない(読み取りのみ)。Test「max_equipment_is_unchanged_and_equipment_never_exceeds_it」で確認済み。

## 17. 完了済み募兵キューの飢餓反例

タスク仕様§6が示した反例(完了済みキューの必要装備100・備蓄60・既存Division不足60のとき、単純な補充では備蓄が既存Divisionへ全消費され翌日以降も届かない)を、実際にE2Eテスト「blocked_completed_queue_protects_stockpile_and_resolves_the_next_day」で再現・検証した。

## 18. 採用した待機キュー保護方式

`process_recruitment`の戻り値を`()`から`HashSet<CountryId>`へ変更し、その日`try_spawn_division`が純粋な`EquipmentAccountingError::InsufficientStockpile`で失敗した国のIDを集める(不正なdefinition・不正数量による他のエラー種別は含めない、§6.2)。`handle_daily_military`がこの集合を`apply_daily_equipment_reinforcement`へ渡し、該当国はその日の既存Division向け装備補充予算を丸ごと0にする。新しい永続予約フィールド・`RecruitmentQueueItem`への新フィールドは一切追加していない——当日の処理結果から都度決定する一時的な判定である。

## 19. 募兵と既存Divisionの優先順位

既存のSystem順序(募兵→既存Division補充)は変更していない。募兵が完了した日は、消費後の余剰備蓄がそのまま同日中に既存Divisionへ配分される(§12.6の数値例どおり)。募兵が装備不足で待機した日は、既存Divisionへの配分が0になる(§17〜18)。

## 20. 同日生産・募兵・補充・戦闘の結果

E2Eテスト「completed_recruitment_leftover_stockpile_reinforces_existing_divisions_the_same_day」(募兵消費後の余剰が同日中に既存Divisionへ配分される)、「equipment_reinforced_the_same_day_can_still_be_lost_to_combat_that_day」(Idle Divisionへの補充と、戦闘参加中Divisionの同日装備損失が両方作動する)、「a_newly_completed_recruit_is_not_double_supplied_on_its_completion_day」(募兵で完成したDivisionは既に満タンのため、同日の装備補充で二重加算されない)で、実際に登録されたSystem経由の同日内相互作用を確認した。

## 21. 複数国家分離

`apply_equipment_reinforcement_for_country`は`owner`ごとに独立して呼び出され、他国の`CountryStockpile`・`Division`へは一切アクセスしない。Test「multiple_countries_are_fully_isolated」「one_countrys_abort_does_not_affect_another_country」「blocking_one_country_does_not_block_another」で確認済み。

## 22. 消滅国家の扱い

`country.is_active()`(`CountryStatus::Eliminated`で`false`)を`apply_equipment_reinforcement_for_country`冒頭で確認し、非活動国は備蓄・Divisionとも一切変更しない(既存のmanpower補充と同じガード)。Test「eliminated_country_is_frozen」で確認済み。

## 23. 不正値・overflow処理

* `NaN`/負値/値域超過の国家備蓄・Division装備値: `military_equipment_value_to_units`が`Err`を返し、その国全体を中断(§13)。
* `equipment > max_equipment`という不正状態: 明示的に検出し中断。
* `checked_mul`/`checked_add`(`apportion_reinforcements`内、既存実装のまま`u128`中間計算)によりoverflowしない。

## 24. New Game/Loaded Game比較

E2Eテスト「new_game_and_loaded_game_produce_identical_equipment_reinforcement」で、New Game直後の実行結果とNew Game→save→load後の実行結果が完全に一致することを確認した。

## 25. save/load分岐比較

E2Eテスト「pre_reinforcement_save_load_branch_matches_the_uninterrupted_run」(補充前にsave/loadを挟んだ場合と継続実行の比較)、「post_reinforcement_stockpile_and_equipment_survive_save_and_load」(補充後の値がsave/loadで保持されるか)で確認した。

## 26. Phase 23・24会計の保護

`cargo test --test p23_008_daily_reinforcement_e2e_test`(21件)・`p23_010_phase23_manpower_lifecycle_audit_test`(9件)・`p24_002_workforce_rounding_unification_e2e_test`(17件)いずれも無変更のまま全件合格を確認した(§33)。

## 27. P25-003回帰結果

`cargo test --test p25_003_recruitment_equipment_issue_e2e_test`(22件)全件合格。`process_recruitment`の戻り値型変更(`()` → `HashSet<CountryId>`)は、既存の呼び出し側(戻り値を束縛しない文として呼ぶだけ)に影響しないことを確認した。

## 28. P25-004回帰結果

`cargo test --test p25_004_combat_equipment_loss_e2e_test`(5件)全件合格。装備補充が戦闘の前に走るようになったことで、戦闘関連のRNG系列・manpower/organization casualtyの結果に変化がないことをE2Eテスト「equipment_reinforcement_does_not_change_combat_manpower_or_organization_results」で直接確認した。

## 29. 撤退時追加損失を未接続のまま維持したこと

`resolve_participant_outcome`の`RETREAT_MANPOWER_LOSS_RATIO`による追加manpower損失は、P25-004から引き続き未接続のまま(既知の追跡事項)。本タスクでも変更していない。

## 30. 対象外を先行実装していないこと

新しい装備カテゴリ・ResourceType・Division保存フィールド・Save V2/migration・装備予約フィールド・輸送時間・補給線・鉄道/港/トラック・日次補充率/修理速度定数・装備の自然回復/修理/回収/鹵獲・復員時装備返却・全滅時追加返却・国家消滅時の装備移転・戦闘力への装備充足率反映・UI表示・AIの募兵/補充優先度変更・AIの装備在庫目標・Attrition・負傷者/疾病・戦死者population連動・弾薬/燃料/食料などの消耗品補給は、いずれも実装していない。

## 31. Critical/High/Medium/Low

**Critical: 0件、High: 0件、Medium: 0件、Low: 0件**

本タスクの実装自体に新規の懸念事項はない。§17〜18の飢餓反例は仕様書自身が事前に指摘していた既知のリスクであり、確定仕様どおりの方式(当日ブロック)で解消済み。

なお、本タスクの**検証プロセス自体**で2件の実装ミスを発見・その場で修正した(いずれも本番コードではなく、このタスクで新規に書いたテストコード自身のバグ):

1. **生f64加算による浮動小数点不一致**: `equipment_reinforcement.rs`の按分テストで、期待値を`96.3 + 1.85`のような生の`f64`加算リテラルで書いたところ、`98.14999999999999`(実際の正規units変換結果`98.15`と1ulp不一致)でテスト自体が失敗した。P25-002-REV-001が回避しようとした精度問題そのものがテストコードに紛れ込んでいたため、正規の小数リテラル(`98.15`)へ直接比較する形に修正した。
2. **架空StateIdによる補充対象外の見落とし**: E2Eテストの初期案で、既存の`land_war_combat_peace_test.rs`パターン(実マップに存在しない架空`StateId`)をそのまま流用したところ、`is_eligible_for_daily_reinforcement_by_position`の支配地域判定(`state_registry.get(...).map(|s| s.controller()==owner).unwrap_or(false)`)が常に`false`を返し、対象Divisionが全て補充対象外になっていた。3件のE2Eテストが「装備が変化しない」ことを誤って合格させていた(バグを検出できていなかった)。実マップから対象国が実際に所有する州を動的に取得する`state_owned_by()`ヘルパーを追加して修正し、修正後に該当テストが実際に補充の発生を検証していることを確認した。

## 32. 追加・変更したファイル

* `src/military/equipment_reinforcement.rs`(新規): 日次装備補充のDomainロジックと単体テスト21件。
* `src/military/reinforcement.rs`(変更): `is_eligible_for_daily_reinforcement_by_position`を`pub(crate)`として切り出し、manpower補充・equipment補充の両方で共有。
* `src/military/recruitment.rs`(変更): `process_recruitment`の戻り値を`()`から`HashSet<CountryId>`へ変更。
* `src/military/update.rs`(変更): `handle_daily_military`へ`apply_daily_equipment_reinforcement`呼び出しを追加。
* `src/military/mod.rs`(変更): `pub mod equipment_reinforcement;`を追加。
* `tests/p25_005_daily_equipment_reinforcement_e2e_test.rs`(新規): 実App経由のE2Eテスト10件。
* `verification_logs/phase-25/p25-005/p25-005_daily_equipment_reinforcement.md`(新規、本報告書)。

## 33. 実行した全コマンドと正確な結果

```bash
git rev-parse HEAD                              # 0c57bebfe72d2786aa3f8eb54c945316dada8574
git status --short                               #  M saves/savegame_v1.ron のみ
rg --files tests | sort | rg 'p23_008|p23_010|p24_002|p25_002|p25_003|p25_004'

cargo check --lib                                # OK
cargo check --all-targets                        # OK
cargo test --lib military::equipment_reinforcement::   # 21 passed
cargo test --lib military::                      # 251 passed
cargo test --test p23_008_daily_reinforcement_e2e_test           # 21 passed
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test  # 9 passed
cargo test --test p24_002_workforce_rounding_unification_e2e_test # 17 passed
cargo test --test p25_002_equipment_accounting_foundation_e2e_test # 15 passed
cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test # 6 passed
cargo test --test p25_003_recruitment_equipment_issue_e2e_test   # 22 passed
cargo test --test p25_004_combat_equipment_loss_e2e_test         # 5 passed
cargo test --test economy_tests                  # 22 passed
cargo test --test p25_005_daily_equipment_reinforcement_e2e_test # 10 passed
cargo test --all-targets                         # 1740 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings  # 0 warnings
git status --short                               # 9個の既知の非決定的PNGのみ再生成
git checkout -- (9枚のPNGを個別に復元)
git status --short                               # 変更ファイル + saves/savegame_v1.ron のみ
```

全て逐次実行し、並列実行は行っていない。`cargo clean`は使用していない(incremental cache破損は今回発生しなかった)。

## 34. 全テストのpassed/failed/ignored

* 開始時基準: `1709 passed / 0 failed / 1 ignored`
* `cargo test --all-targets`最終結果: **`1740 passed / 0 failed / 1 ignored`**
* 新規追加テスト: **31件**(`equipment_reinforcement.rs`内部ユニットテスト21件 + `tests/p25_005_daily_equipment_reinforcement_e2e_test.rs`10件)

## 35. clippy警告数

**0件**

## 36. Cargoを逐次実行したこと

本報告書§33に記載した全コマンドを、1つずつ完了を確認してから次を実行した。並列実行は一度も行っていない。

## 37. ユーザーセーブの保護

`saves/savegame_v1.ron`は全工程で読み書き・復元・ステージのいずれも行っていない。

## 38. 監査対象コードの`git status --short`

実装完了・コミット直前:

```text
 M saves/savegame_v1.ron
 M src/military/mod.rs
 M src/military/recruitment.rs
 M src/military/reinforcement.rs
 M src/military/update.rs
?? src/military/equipment_reinforcement.rs
?? tests/p25_005_daily_equipment_reinforcement_e2e_test.rs
```

## 39. P25-006への引き継ぎ事項

* 生存Division復員時の装備返却(`try_return_equipment`を正規返却APIとして使う)は未実装。
* 破壊済み装備(P25-004で既に`try_destroy_equipment`により消滅済み)を復員処理が二重に返却しない構造にすること。
* 全滅Division(P25-004で既に現在装備を全損済み)は復員経路に到達しない(既にdespawn済み)ため、二重返却の懸念はない。
* 国家消滅時は既存方針どおり凍結する(`CountryStatus::Eliminated`の国は本タスクの装備補充と同様、復員処理でも一切変更しないこと)。
* `try_return_equipment`は既にP25-002-REV-001で実装済み・未接続のまま(§P25-002レポート参照)。
