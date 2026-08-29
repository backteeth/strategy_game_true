# P25-007: 装備充足率の戦闘力反映

## 1. 総合判定

**COMPLETE**

## 2. 開始時HEAD

```text
6023bd53d39bc12b8dd977c4d79afd3de3397961
```

## 3. 監査対象コード

* `src/military/combat_calc.rs`(戦闘力計算の全式、装備会計は無関係だった)
* `src/military/update.rs::process_daily_battles`/`handle_daily_military`(戦闘の実際の呼び出し経路)
* `src/military/equipment.rs`(P25-002-REV-001/P25-004の既存fixed-point API、`validate_equipment_state`等)
* `src/economy/resources.rs`(`military_equipment_value_to_units`等)

## 4. 稼働中の戦闘経路

```text
handle_daily_military(DailySimulationSet::MilitaryAction、src/military/mod.rsで登録)
  └─ process_daily_battles(src/military/update.rs、private fn)
       └─ resolve_combat_day_multi(src/military/combat_calc.rs、pub fn)
            ├─ 1v1の場合: resolve_combat_day(pub fn)へ完全委譲
            └─ N対Mの場合: effective_power_1000(private fn)を各陣営で合算
```

`resolve_combat_day`/`resolve_combat_day_multi`はいずれも`pub fn`であり、外部テストからも直接呼び出せることを確認済み(実際に本タスクの新規E2Eテストで直接利用した)。

## 5. 変更前のattack／defense式

```text
実効攻撃力 = attack_power × (現在manpower/max_manpower) × (現在organization/max_organization)
※ 固定小数点(整数×1000)で計算
```

`resolve_combat_day`(1v1)・`effective_power_1000`(N対M)のどちらも、上記と全く同じ式をそれぞれ独立にインライン実装していた。装備(`equipment`/`max_equipment`)は一切参照されていなかった。

## 6. 変更前のmanpower補正

`atk_manpower_ratio_1000 = (attacker.manpower × 1000) / attacker.max_manpower`という形で、既に**線形にeffective powerへ直接乗算**されていた(`atk_effective_1000 = attack_power × atk_manpower_ratio_1000 × atk_org_ratio_1000 / 1_000_000`)。したがって本タスクは、この既存のmanpower比率単独の箇所を、`min(manpower比率, 装備比率)`(戦闘充足率)へ**そのまま置き換える**という単一の代入で実現できた(§6の要求どおり、既存式を二重適用していない)。

## 7. 採用した戦闘充足率

```text
m = 現在manpower / max_manpower
e = 現在equipment / max_equipment
r = min(m, e)
```

`r`をorganization比率とは独立した、既存の「manpower比率」の代替として、`atk_effective_1000`/`def_effective_1000`(および`effective_power_1000`)の計算式へ直接組み込んだ。

## 8. `min(m,e)`を採用した理由

manpowerとequipmentが同じ戦闘損失によって同じ割合減った場合(例: どちらも50%)、`min(50%,50%)=50%`となり、単一の損失原因を2経路で二重評価しない。`50%×50%=25%`という積の式は採用していない(§5の要求どおり)。

## 9. 二重低下を防いだこと

`test_18_4`(§18.4)で、`manpower=50%, equipment=50%`のとき`combat_readiness_units=500,000`(50%)であり、250,000(25%)にならないことを明示的に確認した。

## 10. per-million固定小数点式

新規`pub fn calculate_combat_readiness(manpower, max_manpower, equipment, max_equipment) -> Result<CombatReadiness, CombatReadinessError>`(`src/military/combat_calc.rs`)。

```text
COMBAT_READINESS_SCALE = 1,000,000

manpower_readiness_units = floor(manpower × SCALE / max_manpower)   (checked u128)
equipment_readiness_units = floor(equipment_units × SCALE / max_equipment_units)  (checked u128)
combat_readiness_units = min(manpower_readiness_units, equipment_readiness_units)
```

装備数量のfixed-point変換自体は既存の`military_equipment_value_to_units`(P25-002-REV-001)をそのまま再利用し、装備量のscale・validation式を複製していない。`COMBAT_READINESS_SCALE`は装備数量のscale(`MILITARY_EQUIPMENT_SCALE`)とたまたま同じ1,000,000だが、意味の異なる独立した定数として定義した(§7の要求どおり)。

**既存の戦闘式(scale 1000)への変換**: `resolve_combat_day`/`effective_power_1000`は既存どおりscale 1000のi64固定小数点で計算されているため、`combat_readiness_units`(scale 1,000,000)を`combat_readiness_units / 1000`という単純な整数除算でscale 1000へ変換して差し込んだ。この変換が精度を落とさないことは、整数除算の合成則
`floor(floor(a×1,000,000/b)/1,000) = floor(a×1,000/b)`(`1,000,000/1,000=1,000`であるため)
と、`min`が単調非減少関数(floor除算)と可換であること(`floor(min(A,B)/c) = min(floor(A/c), floor(B/c))`)から数学的に証明できる——直接scale 1000で計算した場合と完全に一致する。これが§10(完全充足時の完全互換)がbit-for-bitで成立する根拠である。

## 11. `max_equipment=0`の扱い

装備を必要としないDivision(`max_equipment=0`)は、`equipment_readiness_units = COMBAT_READINESS_SCALE`(100%)として扱う(§8.2)。`max_equipment=0`かつ`equipment>0`は`CombatReadinessError::EquipmentExceedsMax`として明示的に拒否する(黙って100%へclampしない)。

## 12. `equipment=0`の扱い

`max_equipment>0`かつ`equipment=0`は`equipment_readiness_units=0`となり、`combat_readiness_units`も0になる(§8.3)。0除算・NaN・Infinity・panicは発生しない。

## 13. `max_manpower=0`の扱い

`max_manpower=0`かつ`manpower=0`は`manpower_readiness_units=0`として扱う(§8.4、ゼロ除算はしない)。`max_manpower=0`かつ`manpower>0`は`CombatReadinessError::ManpowerExceedsMax`として拒否する。`manpower>max_manpower`・`equipment>max_equipment`も同様に、黙ってclampせず`Err`として拒否する(§8.5)。

## 14. attackへの適用位置

`resolve_combat_day`/`effective_power_1000`内、`atk_effective_1000`(攻撃側の実効攻撃力)の計算式で、旧`atk_manpower_ratio_1000`を新設した`combat_readiness_1000`(private helper)へ置き換えた。

## 15. defenseへの適用位置

同様に、`def_effective_1000`(防御側の実効防御力)の計算式でも、旧`def_manpower_ratio_1000`を`combat_readiness_1000`へ置き換えた。attack側・defense側とも**同一の`calculate_combat_readiness`関数**を通す(§9の要求どおり、attacker/defenderで異なる式を使っていない)。

## 16. 複数Divisionの個別補正

`effective_power_1000`(N対M戦闘用)は、Divisionごとに個別へ`combat_readiness_1000(division.manpower, division.max_manpower, division.equipment, division.max_equipment)`を計算してから、そのDivision自身のattack_power/defense_powerへ乗算する。陣営全体の効果はこの個別補正済みの実効値を`Σ`で合算するだけであり、装備状態の異なるDivisionを先に平均してから一括補正することはしていない(§9の禁止事項を遵守)。Test 24/25で、装備0のDivisionが攻撃側集団に混ざっても、防御側への被害が「平均化」されず装備満タンDivision単独の場合とほぼ同じ(端数のみの差)になることを確認した。

## 17. 完全充足時の完全互換

全参加Divisionが`manpower=max_manpower`かつ`equipment=max_equipment`の場合、`combat_readiness_units=1,000,000`(=旧`manpower_ratio_1000=1000`と完全一致するscale変換後の値)となり、既存の`resolve_combat_day`の全既存単体テスト(§6の8件)が**一切変更せず**全て合格することを確認した。新規テスト`test_16_17_18`でも、既存の`test_resolve_combat_day_basic`/`test_combat_is_deterministic`と全く同じ入力・期待値の再現を確認した。

## 18. 装備不足時の戦闘結果

装備不足はmanpower casualty・equipment casualty・organization damage・勝敗・戦闘期間・撤退発生・最終Division存続へ**結果として**変化して構わない(§11)。これらへ戦闘充足率を直接二重適用してはおらず、`readiness → attack/defense → 既存combat calculation → casualty/organization/outcome`という単一経路のみを通る(既存の`resolve_combat_day`の計算構造自体は変更していない)。

## 19. organizationへの直接非適用

`organization`・`max_organization`・organization回復速度(`ORG_RECOVERY_PER_DAY`、`process_org_recovery`)には一切触れていない。`atk_org_loss = atk_manpower_loss / ORG_DAMAGE_DENOMINATOR`という既存式のまま(readinessは`atk_manpower_loss`を経由した間接的な影響のみ、直接乗算していない)。Test 29で、organization損失が引き続き`manpower_loss/10`の式のままであることを確認した。

## 20. movementへの非適用

`movement_progress`・`destination`・`current_path`・`target_state`はいずれも`calculate_combat_readiness`/`resolve_combat_day`から一切参照・変更されない。Test 30で、これらのフィールドが戦闘計算前後で不変であることを確認した。

## 21. RNG系列の維持

このコードベースの戦闘計算は乱数を一切使用しない(`combat_calc.rs`自身のモジュールdoc「乱数を使わず、整数演算で決定的な戦闘計算を行う」、P25-007でも変更せず)。したがって「RNG呼び出し回数・系列が変わらない」という要件は、同一入力に対して常に同一出力を返す決定性の確認に帰着する。Test 27/28で、装備量を0〜100%まで変化させた5パターンそれぞれについて、同一入力を2回評価した結果が完全一致することを確認した。

## 22. 0戦闘力の安全性

装備0またはmanpower0により戦闘充足率が0になっても、`attack`/`defense`寄与は安全に0となり、NaN・Infinity・panicは発生しない(Test 31/32/33)。双方の戦闘充足率が0の場合、casualty自体が0になることを確認した——「戦闘が停滞する可能性」をarbitraryな最低戦闘力で隠さない(§14の要求どおり)。

## 23. 戦闘前validationと原子性

`process_daily_battles`(`src/military/update.rs`)へ新設したpreflightステップが、`resolve_combat_day_multi`呼び出し前に全参加Divisionへ`calculate_combat_readiness`を適用する。1件でも`Err`なら、そのBattleの当日解決(manpower/equipment/organization/battle statistics更新、`elapsed_days`加算)を丸ごとskipし、他のBattle・他国家へは影響しない。RNGは元々消費されないため「消費しない」という要求は構造的に自明。Test(`test_preflight_skips_battle_resolution_atomically_when_a_participant_has_invalid_equipment`等)で、NaN装備・manpower超過のいずれについても、原子的にskipされ部分的な状態変化が残らないことを確認した。

`resolve_combat_day`/`effective_power_1000`自身は、内部で`calculate_combat_readiness`を呼び失敗時は`unwrap_or(0)`で安全にフォールバックするが、これは`process_daily_battles`のpreflightが既に全参加者を検証済みであることを前提とした防御的な到達不能コードである(通常経路では発生しない)。

## 24. P25-005補充後の値を使うこと

既存System順序(`MilitaryEquipment生産→募兵完了→manpower補充→equipment補充→移動→戦闘`)は一切変更していない。`resolve_combat_day`は`Division.equipment`を都度読むため、同日中に日次装備補充(P25-005)で更新された値をそのまま使う。Test 34で、Idle状態(戦闘未参加)のDivisionが日次補充で満タンへ回復した後、その満タン状態を基準に後続の戦闘力計算が行われることを確認した(装備補充の適格条件`is_eligible_for_daily_reinforcement_by_position`が進行中戦闘参照Divisionを除外する既存設計のため、「同じDivisionが同日中に初めて交戦する」ケースの再現は、Idle・未参戦のまま補充を受けた後にBattleへ組み込む形で行った——この既存の除外設計自体はP25-005のドキュメント化済みの仕様であり、本タスクで変更していない)。

## 25. P25-004装備損失との接続

戦闘充足率反映後の実効attack/defenseから導出される`manpower_loss`(P25-004の`apply_combat_casualty`が受け取る値)は、既存どおり`actual_casualty`(`saturating_sub`後の実減少量)から比例装備損失を計算する。装備不足でcasualtyが変わっても、その新しいactual casualtyから既存のhalf-up丸め式で装備損失が計算されるだけであり、損失式・丸め方式自体は変更していない(§17)。Test 35で、装備充足率の影響を受けた戦闘でも装備が実際に損耗することを確認した。

## 26. P25-006-REV-001撤退・強制削除との接続

`RETREAT_MANPOWER_LOSS_RATIO`・`resolve_participant_outcome`の撤退追加manpower損失計算・強制削除時の残存装備全損処理は、いずれも本タスクで一切変更していない(`src/military/update.rs`の該当関数は無変更、diffなし)。撤退の発生有無・タイミングは、戦闘充足率の影響で通常戦闘の被害が変わった結果としてorganizationの減り方が変わることで**間接的に**変化し得るが、撤退時の追加manpower損失式自体・装備損失式(P25-004と同じ)・強制削除処理はP25-006-REV-001から不変。

## 27. 国家間分離

`calculate_combat_readiness`はDivision自身のmanpower/equipmentのみを引数に取り、国家や他Divisionの状態には一切アクセスしない。Test 41で、第三国のDivisionが戦闘へ混入しないことを確認した。

## 28. New Game／Loaded Game比較

Test 42で、New Game直後の戦闘結果とNew Game→save→load後の戦闘結果(manpower・equipment)が完全一致することを確認した。

## 29. save/load分岐比較

Test 43/44で、戦闘前save/load分岐(継続実行と一致)・戦闘後save/load(値がそのまま保持される)の両方を確認した。

## 30. Save V1維持

新規保存フィールドは一切追加していない。`calculate_combat_readiness`は既存の`Division.manpower`/`max_manpower`/`equipment`/`max_equipment`フィールドのみを参照する。

## 31. Phase 23・24会計の保護

`p23_006`(22件)・`p23_007`(24件)・`p23_008`(21件)・`p23_010`(9件)・`p24_002`(17件)、いずれも全合格・対象ソース無変更。

## 32. P25-003〜P25-006-REV-001回帰結果

`p25_002`(15件)・`p25_002_rev_001`(6件)・`p25_003`(22件)・`p25_004`(5件)・`p25_005`(10件)・`p25_006`(29件)・`p25_006_rev_001`(8件)、いずれも全合格。

**既存テスト2件を意図的に訂正**(装備不足でmanpower/organization casualtyが変化するというP25-007自身の仕様と直接矛盾するため):

1. `src/military/update.rs`のP25-004由来`test_s_equipment_amount_does_not_affect_manpower_or_organization_casualties`(装備量の違いがcasualtyへ漏れ出さないことを検証)→`test_s_equipment_amount_now_intentionally_affects_manpower_and_organization_casualties`(装備0なら双方の戦闘充足率が0となりcasualty自体が0[膠着]になること、装備満タンならP25-004時点と同じ通常のcasualtyが生じることを検証)へ更新。
2. `tests/p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs`の`test_equipment_amount_does_not_change_retreat_manpower_or_status_via_full_app`(装備量の違いが撤退結果へ漏れ出さないことを検証)→`test_retreat_outcome_is_deterministic_for_a_given_equipment_level_even_though_it_now_varies_across_levels`(同じ装備量なら決定的に同じ結果、異なる装備量なら結果が変わり得ることを検証)へ更新。

**同ファイルのフィクスチャも1件修正**: `setup_retreating_battle`の防御側装備が`60.0/100.0`(readiness 60%)だったため、P25-007により防御側の実効防御力が意図せず弱まり、元のP21-siege想定(攻撃側が30日以内に組織率0へ達して撤退する)のタイミングが崩れて2件のテストが失敗した。防御側を`100.0/100.0`(readiness 100%)へ修正し、元のバランスを回復した(このテストの目的は撤退時装備損失の検証であり、戦闘充足率そのものの検証ではないため)。

## 33. Critical／High／Medium／Low

**Critical: 0件、High: 0件、Medium: 0件、Low: 0件**

## 34. 追加・変更したファイル

* `src/military/combat_calc.rs`(`calculate_combat_readiness`/`CombatReadiness`/`CombatReadinessError`/`COMBAT_READINESS_SCALE`新設、`resolve_combat_day`/`effective_power_1000`の内部式置き換え)
* `src/military/update.rs`(`process_daily_battles`へのpreflight追加、既存test_sの訂正)
* `tests/p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs`(フィクスチャ修正1件、テスト1件訂正)
* `tests/p25_007_equipment_combat_effectiveness_e2e_test.rs`(新規、21件)
* 本報告書(新規)

## 35. 実行した全コマンドと正確な結果

```bash
git rev-parse HEAD                          # 6023bd53d39bc12b8dd977c4d79afd3de3397961
git status --short                          # M saves/savegame_v1.ron のみ
rg -n "combat_calc|attack|defense|max_manpower|max_equipment|equipment" src/military src/war tests
rg --files tests | sort | rg 'p21_|p22_|p23_006|p23_007|p23_008|p23_010|p24_002|p25_00'

cargo check --lib
cargo test --lib military::combat_calc::   # 25 passed
cargo test --lib military::update::        # 29 passed
cargo test --lib military::                # 283 passed
cargo test --lib country::                 # 79 passed
cargo test --lib war::                     # 153 passed

cargo test --test land_war_combat_peace_test          # 4 passed
cargo test --test p21_005_army_frontline_e2e_test     # 2 passed
cargo test --test p21_008_army_offensive_e2e_test     # 2 passed
cargo test --test p21_021_country_elimination_e2e_test # 1 passed
cargo test --test p23_006_manpower_ledger_e2e_test    # 22 passed
cargo test --test p23_007_division_demobilization_e2e_test # 24 passed
cargo test --test p23_008_daily_reinforcement_e2e_test # 21 passed
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test # 9 passed
cargo test --test p24_002_workforce_rounding_unification_e2e_test # 17 passed
cargo test --test p25_002_equipment_accounting_foundation_e2e_test # 15 passed
cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test # 6 passed
cargo test --test p25_003_recruitment_equipment_issue_e2e_test # 22 passed
cargo test --test p25_004_combat_equipment_loss_e2e_test # 5 passed
cargo test --test p25_005_daily_equipment_reinforcement_e2e_test # 10 passed
cargo test --test p25_006_demobilization_equipment_return_e2e_test # 29 passed
cargo test --test p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test # 8 passed
cargo test --test p25_007_equipment_combat_effectiveness_e2e_test # 21 passed

cargo test --all-targets                     # 1833 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings  # 0 warnings

git checkout -- (非決定的PNG 9枚を個別に)
```

全て逐次実行。並列実行なし。`cargo clean`は使用していない(incremental cache破損の兆候はなかった)。

## 36. 全テストpassed／failed／ignored

* 開始時基準: `1792 passed / 0 failed / 1 ignored`
* 最終`cargo test --all-targets`結果: **`1833 passed / 0 failed / 1 ignored`**
* 新規追加: **41件**(`combat_calc.rs`内部17件、`update.rs`内部preflight3件、新規e2e21件)
* 既存テストの意図的な訂正: 2件(§32参照、失敗0件)

## 37. clippy警告数

**0件**(`cargo clippy --all-targets --all-features -- -D warnings`)。

## 38. Cargo逐次実行

厳守。並列実行なし。

## 39. ユーザーセーブの保護

`saves/savegame_v1.ron`は全工程で未読み書き・未ステージ・未復元。

## 40. 監査対象コードの`git status --short`

```text
 M saves/savegame_v1.ron
 M src/military/combat_calc.rs
 M src/military/update.rs
 M tests/p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs
?? tests/p25_007_equipment_combat_effectiveness_e2e_test.rs
```

## 41. P25-008への引き継ぎ事項

P25-008はPhase 25最終監査として、装備の生産(P25-002-REV-001)・募兵支給(P25-003)・戦闘損失(P25-004)・日次補充(P25-005)・復員返却(P25-006)・強制削除時破壊(P25-006-REV-001)・戦闘力反映(P25-007、本タスク)を一続きのライフサイクルとして固定するタスクを推奨する。P25-001の当初段階的計画(P25-001〜P25-007)は本タスクで完全に実装完了しており、残るのはこの最終監査のみである。

## 42. 対象外を先行実装していないこと

organizationへの直接装備補正・movementへの装備補正・装備不足による追加RNG・装備品質/種別・軽装備/重装備・弾薬/燃料/日次消耗品・補給線・輸送能力・修理/回収・鹵獲・国家AIの装備判断・募兵AIの装備在庫目標・UIの装備充足率表示・戦闘画面の装備損失表示・Save V2/migration・Attrition・負傷者/疾病・戦死者population連動・Phase 25最終監査(P25-008)の先行実施は、いずれも行っていない。
