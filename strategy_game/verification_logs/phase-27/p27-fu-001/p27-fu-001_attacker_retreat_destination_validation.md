# P27-FU-001 攻撃側撤退先の検証と代替退却先探索 実装報告

## 1. 総合判定

**P27-FU-001 COMPLETE**。P27-FU-001-PRE-001で確定した候補2(記録済みoriginが現在も有効ならそのまま使用、無効なら戦闘州の隣接州から代替を探索、代替もなければ既存の撤退不能時Division消滅処理)を実装した。候補「controller変化履歴を保存する狭い変種」は、Save/State契約を不必要に拡張し、かつ開始時点から無効だったoriginを誤って保護してしまうため不採用とした(PRE-001の判断を変更していない)。

## 2. 開始時HEAD

`78bd51a`(docs: design attacker retreat destination validation)。作業開始前の`git status --short`は空。

## 3. 実装対象ファイル

- `src/military/invasion.rs`: `neighboring_retreat_candidate`(新規、防御側・攻撃側で共有する候補探索の純粋関数)、`attacker_retreat_destination`(新規、公開関数)、`find_retreat_destination`(既存関数を上記共有ヘルパーへ委譲するようリファクタ、挙動不変)。
- `src/military/update.rs`: `resolve_finished_battles`内の2箇所(P27-007ゼロ損害撤退分岐、通常組織率0撤退分岐)を、`battle.attacker_origins.get(&division_id)`の直接使用から`attacker_retreat_destination(...)`呼び出しへ変更。`force_retreat_or_destroy`自体は無変更。
- 既存テスト5ファイルの修正(§6参照)。
- 新規テスト2ファイル追加(§8参照)。

`combat_calc.rs`・`daily_supply.rs`・Save DTO・`Battle`/`Division`構造体はいずれも変更していない。

## 4. 実装方針の遵守

- `neighboring_retreat_candidate`は状態を変更しない純粋関数で、防御側の既存`find_retreat_destination`と攻撃側の新規`attacker_retreat_destination`の双方から共有する(ロジックの複製なし)。
- originの優先判定(有効ならそのまま使用)には、`battle_state`への隣接性チェックを含めない。本番では`attacker_origins`は常に直前の`current_state`(構造的に隣接)から記録されるため、この判定はPRE-001での設計どおり。
- controller変化履歴の保存、新規`Battle`/`Division`フィールド、Save DTO変更、国別特別処理、非隣接州へのBFS、非隣接への即時撤退、撤退の通常Movement化は、いずれも実装していない。

## 5. 13件の既存テスト精査結果

PRE-001の一時パッチにより、実装前に予測された13件の失敗が実装後も1件残らず同一集合で再現した(設計と実装の一致を確認)。

### 分類A(fixtureが単に無効・撤退先仕様と無関係): 8件、5ファイル

| ファイル | テスト | 問題 | 修正 |
|---|---|---|---|
| `p25_004_combat_equipment_loss_e2e_test.rs` | `test_y_zero_equipment_divisions_reproduce_the_pre_p25_004_manpower_and_organization_behavior` | `create_battle`が`state_id`(`TEST_STATE_ID`、実在しない合成州)をoriginとして流用 | `attacker_origin`引数を追加、`player_home`(実マップ・自国支配州)を渡す。撤退先アサーションも`TEST_STATE_ID`→`player_home`へ修正 |
| `p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs` | `test_the_real_registered_active_path_applies_retreat_equipment_loss`ほか、`setup_retreating_battle`を使う全テスト(5件) | `create_battle`が`state_id`(`TEST_STATE_ID`)をoriginとして流用 | `create_battle`に`attacker_origin`引数を追加、呼び出し元の`player_home`を渡す |
| `p25_007_equipment_combat_effectiveness_e2e_test.rs` | `test_18`・`test_42`・`test_43_44`(3件) | `create_battle`が`TEST_STATE_ID`をoriginとして流用 | `create_battle`内部で`state_owned_by(app, attacker_country)`を呼び、実マップの自国支配州をoriginとして使う(呼び出し元8箇所は無変更) |
| `p26_005_supply_combat_strength_e2e_test.rs` | `test_d5_two_independent_app_instances_produce_identical_combat_results_with_partial_supply` | 同上(`TEST_STATE_ID`流用) | `create_battle`内部で新設`state_owned_by(app, attacker_country)`をoriginとして使う |
| `p26_006_capital_connectivity_supply_e2e_test.rs` | `test_fully_disconnected_division_in_real_battle_does_not_panic_or_produce_nan` | `create_battle`がBattleの場所(`ARCADIA_FRONTIER`)自身をoriginとして流用——origin自身に登録中のBattleが存在するため常に無効判定になり、遮断済みで代替も無い前線では消滅していた | `create_battle`内部で`ARCADIA_CAPITAL`(常に自国支配・非戦闘州)をoriginとして使う |

いずれも、撤退先そのものの妥当性検証ではなく「装備会計」「実効戦力」「save/load一致性」「決定性」を検証する既存テストであり、修正後も元の検証目的・期待値は変更していない。

### 分類B(退路なし→消滅を検証する既存テスト): 1件

`p27_006_low_effective_power_damage_e2e_test.rs::test_12`(旧名`test_12_missing_origin_destroys_the_attacker_without_panicking`)。旧実装ではorigin欠落=即消滅だったが、新実装ではorigin欠落時に代替探索が走り、既定マップでは州1が有効な代替として見つかってしまい「消滅」ではなく「代替撤退」が起きるため失敗した。修正: `pre_battle_controller_overrides=[(StateId(1), ELFIN)]`を追加し、唯一の既定代替も無効化することで、真に「代替なし」の状況を構築した。テスト名を`test_12_missing_origin_and_no_alternative_destroys_the_attacker_without_panicking`へ変更し、意図を明示した。

### 分類C(無効origin挙動自体を検証する既存テスト): 0件

該当なし。P27-006・P27-007時点では「origin無効」を専用に検証するテストは存在せず、分類Bの`test_12`(欠落)のみが該当した。

### 分類D(過去のfixture誤り): 1件

`p27_006_low_effective_power_damage_e2e_test.rs::test_11_multiple_attackers_with_distinct_origins_retreat_to_their_own_origin`。P27-005の回廊遮断シナリオに引き寄せられ「StateId(4)は既定でArcadia領」という誤った前提を含んでいたが、実マップの既定controllerは`CountryId(1)`(Elfin)であることを診断用一時テストで実測確認した。修正: `pre_battle_controller_overrides=[(StateId(4), ARCADIA)]`を戦闘登録前に追加し、実際にArcadia支配へ変えたうえでoriginとして使う。

**13件全ての個別事情を確認済みで、「テストが落ちたから期待値をこう変える」という機械的な反転は行っていない**(各修正の理由は上表・上記の通り、fixtureの構築ミスの特定に基づく)。`tests/land_war_combat_peace_test.rs`は未変更・未整形。

## 6. `Spec`構造体の拡張(`p27_006_low_effective_power_damage_e2e_test.rs`)

`pre_battle_controller_overrides: Vec<(StateId, CountryId)>`(Battle登録前に適用)、`post_battle_controller_overrides: Vec<(StateId, CountryId)>`(Battle登録後=`attacker_origins`記録済みの後に適用、「開始時点で有効だったoriginが戦闘継続中に敵へ再占領される」を再現)を追加した。以後のP27シリーズのテストでも再利用できる汎用機構。

## 7. 単体テスト(`src/military/invasion.rs`)

新規11件、`fu001_states`ヘルパー(owner/enemyの4州合成マップ)を使用。

1. `attacker_retreat_destination_uses_the_valid_origin_and_does_not_search_alternatives`
2. `attacker_retreat_destination_rejects_an_origin_controlled_by_the_enemy`
3. `attacker_retreat_destination_rejects_an_origin_controlled_by_a_third_country`
4. `attacker_retreat_destination_searches_alternatives_when_origin_is_missing`
5. `attacker_retreat_destination_searches_alternatives_when_the_origin_state_does_not_exist`
6. `attacker_retreat_destination_searches_alternatives_when_a_battle_is_ongoing_at_the_origin`
7. `attacker_retreat_destination_picks_the_smallest_state_id_among_multiple_alternatives`
8. `attacker_retreat_destination_returns_none_when_no_alternative_exists`
9. `attacker_retreat_destination_capital_disconnected_state_is_still_a_valid_candidate`(首都連結性・補給率は判定条件に含めていないことの確認)
10. `attacker_retreat_destination_is_independent_of_hashmap_or_vec_iteration_order`
11. `find_retreat_destination_result_is_unchanged_by_sharing_the_candidate_helper`(防御側の既存結果が共有ヘルパー化で変わらないことの確認)

`invasion.rs`モジュール全体は18件(既存7件+新規11件)、全通過。

## 8. E2Eテスト

### 8.1〜8.7(`tests/p27_006_low_effective_power_damage_e2e_test.rs`、新規test_14〜17)

- `test_14_attacker_retreats_to_an_alternative_when_the_origin_is_recaptured_mid_battle`(§8.2、P27-008で確認された根本原因の直接再現・解決確認): originが戦闘継続中に敵へ再占領されても、代替探索によりState4へ正しく撤退することを確認。
- `test_15_mixed_attackers_each_resolve_independently_in_the_same_battle`(§8.5): 同一Battle内の2Divisionが、それぞれ直接origin使用/代替探索経由で独立に解決されることを確認。代替探索は`(owner, battle_state)`のみに依存するため、同一Battle内で「代替あり」と「代替なし」が混在するケースは構造的に作れないことをコード上のコメントで明示した。
- `test_16_multiple_divisions_retreating_to_the_same_alternative_are_not_lost_or_duplicated`(§8.6): 2Divisionが同じ代替先へ撤退しても、消失・重複が起きないことを確認。
- `test_17_save_load_matches_across_the_retreat_destination_branches`(§8.7): 再占領シナリオ(d1)・退路なし消滅シナリオ(d1・d3)の両方でsave/load一致を確認。

§8.1(通常origin)・§8.3(開始時から無効)・§8.4(退路なし消滅)は、既存test_08・修正済みtest_11・test_12で相当する検証が既にカバーされているため、重複する新規テストは追加していない。

### AI統合(新規`tests/p27_fu_001_ai_retreat_alternative_e2e_test.rs`、3件)

- `test_01_ai_does_not_form_a_retreat_reattack_loop_after_alternative_retreat`: 実App・実AI(`setup_offensive_war`、P27-007と同方式)で戦闘を成立させ、進行中Battleの`attacker_origins`を全て敵へ再占領してから両陣営ゼロ損害を発生させ、90日間観測。**終了は1回だけ、以後再攻撃ループなし**。この実マップの前線(StateId(3))はArcadia側隣接州がStateId(1)の1つだけの単一回廊構造であることを実測確認したため(診断用一時テストで確認・削除済み)、この特定の再占領方法では代替が0件になり「退路なし消滅」枝が実行される(バグではなく地形上の結果)。
- `test_02_alternative_retreat_does_not_affect_an_unrelated_third_country`: 上記シナリオが無関係な第三国(Passguard)の備蓄・Division状態に影響しないことを確認。
- `test_03_ai_attacker_survives_via_alternative_when_a_second_foothold_exists`: 「代替が見つかって生存する」経路を実AI統合で確認するため、戦闘州の別隣接州を人為的にArcadia支配へ変えて代替を用意したうえで同じ手順を実行。**代替先へ実際に撤退して生存**、終了は1回だけ、再攻撃ループなし。

### 国家消滅・講和(新規`tests/p27_fu_001_elimination_peace_e2e_test.rs`、3件、§12)

- `test_01_attacker_elimination_after_alternative_retreat_does_not_resurrect_or_double_process`: 代替探索経由で撤退し生存していたDivisionが、攻撃側国家消滅時に他のDivisionと同様に一度だけ解散されることを確認(復活なし)。消滅済み国家への再適用はエラーになり、二重処理は起きない。
- `test_02_defender_elimination_mid_battle_does_not_conflict_with_controller_update`: 代替探索が可能な状態のまま防御側国家が戦闘継続中に消滅しても、当該Warが消滅処理の一部として終結し、以後の日次処理でパニックしないことを確認。
- `test_03_peace_settlement_after_alternative_retreat_does_not_double_apply_or_misplace_division`: 代替探索経由で撤退したDivisionが存在する状態で白紙講和が成立しても、撤退時のmanpower/equipment損失が再適用されないこと、および既存の講和処理(占領状態正常化→敵領残存陸軍の帰還、いずれも無変更の既存ロジック)により、法的所有者へ支配が戻った旧代替先に取り残されず、Arcadia自身の支配/所有地(第三国領ではない)へ正しく帰還することを確認した。

## 9. 防御側保護

`find_retreat_destination`の既存単体テスト(`test_find_retreat_destination_deterministic`)は無変更で通過。新規単体テスト11番目(`find_retreat_destination_result_is_unchanged_by_sharing_the_candidate_helper`)で、共有ヘルパー化後も結果が変わらないことを直接確認した。`p26_006`・`p26_005`など防御側の撤退・補給関連の既存テストは全件無変更で通過。

## 10. P27-004〜008保護

- P27-004(`p27_004_idempotent_movement_orders_e2e_test.rs`、6件): 無変更で全通過。
- P27-005(`p27_005_offensive_frontline_supply_bridge_e2e_test.rs`、10件): 無変更で全通過。
- P27-006(`p27_006_low_effective_power_damage_e2e_test.rs`): test_01〜test_10は無変更で通過、test_11・test_12は§5の理由で修正、test_13は無変更で通過。計17件全通過。
- P27-007(`p27_007_zero_damage_battle_termination_e2e_test.rs`、2件): 無変更で全通過。
- P27-008で確認された根本原因(戦闘継続中に出撃元が敵支配へ変わるとorigin検証なしで撤退していた)は、§8のtest_14で直接再現・解決を確認した。

## 11. AI副作用観測結果

§8のAI統合3テストで実測(90日以上)。再攻撃ループは形成されず、新しいcooldown・戦略的撤退AIは追加していない。実マップの前線(StateId(3)↔StateId(1))が単一回廊構造であるため、実AIの自然な進軍だけでは「代替が見つかって生存する」経路への到達が地形上できず、この経路は人為的に第二の橋頭堡を与えたtest_03で確認した(自然発火0件・制御下E2Eを主根拠とする方針は§14と同じ扱い)。

## 12. 国家消滅・講和結果

§8参照。攻撃側消滅・防御側消滅(戦闘継続中)・講和(白紙講和)のいずれも、既存の`country::elimination`・`war::peace`公開APIをそのまま呼び出すだけで、クラッシュ・Division復活・二重処理・第三国領への誤配置は発生しなかった。既存の講和処理§2(占領状態正常化)・§5(敵国領残存陸軍の帰還)は、いずれも無変更の既存ロジックがこのシナリオでも正しく機能することを確認した。

## 13. Save・決定性・国家間分離結果

- Save V1を維持。新規永続フィールド・Save DTO変更なし。
- StateId昇順タイブレーク: 単体テスト7番目で確認。
- HashMap/Vecイテレーション順序非依存: 単体テスト10番目で確認。
- 登録順反転での同一結果: `p27_006`のtest_13(既存、無変更)で確認。
- save/load一致性: `p27_006`のtest_17(新規、§8.7)で複数分岐で確認。
- 国家間分離: `p27_fu_001_ai_retreat_alternative_e2e_test.rs`のtest_02(第三国Passguardの備蓄・Division状態が影響を受けない)で確認。

## 14. 長期回帰結果

`P27-002-C`(90/36/60/120か月、六か国、16件)は本タスクの変更後も全件通過し、監査済みの正規値と一致した(`cargo test --all-targets`の一部として再実行・確認)。自然な通常AI運用では戦闘自体がほぼ発生しないため(既存Phase 27各タスクで確立済みの知見)、新しい代替探索経路の自然発火も実測上0回である。この「0件」自体を根拠とせず、§8・§11の制御下E2E・AI統合テストを主根拠として扱った(spec §14の方針どおり)。

## 15. 対象外(実装していないことの確認)

controller変化履歴の保存、開始時controller保存、owner条件追加、首都連結性条件追加、補給率条件追加、外交的通行権、同盟領への撤退、非隣接州へのBFS、非隣接への即時撤退、撤退の通常Movement化、「撤退中」状態、戦略的撤退AI、cooldown、UI通知、ローカライズ、POW、降伏、Equipment/MilitarySupplies返還、Save V2。いずれも実装していない。P27-005の手動プレイ検証は本タスクでも実施していない(自動検証のみ)。

## 16. 検証コマンドと結果

- `cargo test --lib military::`: 506 passed
- `cargo test --lib save::`: 305 passed
- `cargo test --lib "military::invasion::"`: 18 passed
- `cargo test --test p27_004_idempotent_movement_orders_e2e_test`: 6 passed
- `cargo test --test p27_005_offensive_frontline_supply_bridge_e2e_test`: 10 passed
- `cargo test --test p27_006_low_effective_power_damage_e2e_test`: 17 passed
- `cargo test --test p27_007_zero_damage_battle_termination_e2e_test`: 2 passed
- `cargo test --test p26_008_b_supply_aware_offensive_gate_e2e_test`: 19 passed
- `cargo test --test p25_004_combat_equipment_loss_e2e_test`: 5 passed
- `cargo test --test p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test`: 8 passed
- `cargo test --test p25_007_equipment_combat_effectiveness_e2e_test`: 21 passed
- `cargo test --test p26_005_supply_combat_strength_e2e_test`: 10 passed
- `cargo test --test p26_006_capital_connectivity_supply_e2e_test`: 18 passed
- `cargo test --test p27_fu_001_ai_retreat_alternative_e2e_test`(新規): 3 passed
- `cargo test --test p27_fu_001_elimination_peace_e2e_test`(新規): 3 passed
- `cargo test --test land_war_combat_peace_test`: 4 passed(未変更・未整形)
- `cargo test --test p27_002_c_residual_fiscal_self_stabilization_e2e_test`: 16 passed
- `cargo test --all-targets --no-fail-fast -j 2`: **2706 passed / 0 failed / 0 ignored**(82バイナリ)
- `cargo clippy --all-targets --all-features -- -D warnings`: 警告0

上記のうち、5・6・8・10・14・15・16・17行目に該当するファイル群(修正5件+新規2件)を独立プロセスで5回連続実行し、全回で件数一致・0 failedを確認した。Windowsのページファイル不足を避けるため、全体実行は`-j 2`で行った。cargoの並行実行は避け、常に逐次実行した。

## 17. 保護事項確認

- `saves/savegame_v1.ron`: 読み書き・復元・ステージのいずれも行っていない。
- `tests/land_war_combat_peace_test.rs`: 未変更・未整形(§16で無変更通過を確認)。
- 本番RONデータ: 変更していない。
- ワークスペース全体の再整形: 行っていない。
- 既存報告書: 変更していない。
- `git add .`・`git add -A`: 使用していない(§19で個別ファイルをステージ)。
- 広範なcheckout/reset/stash、push/tag/amend/rebase: 一切実行していない。
- 一時診断用コードの残留: なし(`tmp_diag_battle_state_and_origins`等の一時テストは全て削除済み)。

## 18. Critical／High／Medium／Low

- Critical: 0
- High: 0
- Medium: 0
- Low: 1
  - 実マップの前線(StateId(3))はArcadia側隣接州が1つだけの単一回廊構造のため、実AI統合だけでは「代替が見つかって生存する」経路の自然な発火経路が非常に限定的(人為的な第二橋頭堡が必要)。ゲームプレイ上の制約であり、本タスクの実装自体の不備ではない。

## 19. 最終テスト件数

**2706 passed / 0 failed / 0 ignored**(82バイナリ)。開始時2700(既存)から、P27-FU-001新規分(単体11・E2E 4+3+3=10、計21)を含む。

## 20. clippy警告数

0(`-D warnings`)。

## 21. 全コミット対象ファイル

- `src/military/invasion.rs`
- `src/military/update.rs`
- `tests/p25_004_combat_equipment_loss_e2e_test.rs`
- `tests/p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs`
- `tests/p25_007_equipment_combat_effectiveness_e2e_test.rs`
- `tests/p26_005_supply_combat_strength_e2e_test.rs`
- `tests/p26_006_capital_connectivity_supply_e2e_test.rs`
- `tests/p27_006_low_effective_power_damage_e2e_test.rs`
- `tests/p27_fu_001_ai_retreat_alternative_e2e_test.rs`(新規)
- `tests/p27_fu_001_elimination_peace_e2e_test.rs`(新規)
- 本報告書

## 22. PNG差分処理

テスト実行で再生成された既知の9件のPNGを、`git checkout -- <path>`で個別に復元した。

## 23. 最終`git status --short`

コミット後は空(最終報告に記載)。
