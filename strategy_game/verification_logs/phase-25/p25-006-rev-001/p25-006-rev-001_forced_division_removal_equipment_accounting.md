# P25-006-REV-001: 強制Division削除時の装備会計訂正

**この報告書は、P25-001およびP25-006の国家消滅時Division装備に関する結論を訂正・置換する。旧報告書は監査履歴として削除・変更しない。**

## 1. 総合判定

**COMPLETE**

## 2. P25-006の最終的な扱い

**COMPLETE WITH CORRECTION APPLIED**

P25-006の復員実装(生存Divisionの装備返却)自体は正しく、本タスクでも一切変更していない。P25-006のコミット(`7b20ca4`)・コード(`src/military/demobilization.rs`)・報告書(`verification_logs/phase-25/p25-006/`)はrevert・amend・書き換えのいずれも行っていない。本タスクは新しいコミット・新しい報告書として追加した。

## 3. 開始時HEAD

```text
7b20ca46b8d86cc83b9d05da6c56fa44bad5855a
```

## 4. 監査対象コード

* `src/country/elimination.rs::apply_country_elimination`(国家消滅、P21-021)
* `src/military/update.rs::resolve_participant_outcome`(戦闘全滅・撤退追加manpower損失)
* `src/military/equipment.rs`(P25-002-REV-001/P25-004の既存API、`calculate_proportional_equipment_loss`/`try_destroy_equipment`/`validate_equipment_state`)
* `src/military/demobilization.rs`(P25-006、変更なし)
* `src/war/peace.rs`・`src/diplomacy/crisis_response.rs`(`preflight_country_elimination`の実呼び出し元)

## 5. P25-006で確認されたHigh 2件

1. 国家消滅時に、非ゼロ装備を持つDivisionが装備会計APIを一切経由せず物理削除される(`country::elimination::apply_country_elimination`)。
2. 撤退時追加manpower損失により、非ゼロ装備を持つDivisionが装備会計APIを経由せず削除され得る(`military::update::resolve_participant_outcome`)。

本タスクで両方とも訂正した(§8以降参照)。

## 6. P25-001の誤っていた国家消滅前提

P25-001は「国家消滅時、Division装備も凍結して残存する」と想定していた。しかし実コード(P21-021由来、Phase 25より前から存在)は、消滅国家の全Divisionを`MilitaryRegistry::remove_division`で物理削除しており、Division実体が消える以上「凍結」は成立し得なかった。この前提はP25-001の設計段階での見落としであり、実装が後から不整合を起こしたものではない。

## 7. 国家消滅Divisionを凍結せず全損削除とした理由

本タスクではDivisionを凍結する設計(inactive Division状態の新設)を採用せず、既存のDivision物理削除を維持したうえで、削除前に現在装備を全損させる設計へ訂正した。理由は、凍結設計を採用すると次の広範な影響が避けられないため。

* 消滅国家のDivisionがマップ上へ残り続ける
* 移動・戦闘・描画・選択対象への波及を個別に遮断する必要がある
* 既存のP21・P22国家消滅仕様(勝敗判定・世界終結条件等)の見直しが必要
* 国家復活や占領との新しい意味定義が必要になる
* 新しいinactive Division状態自体を設計・実装・検証する必要がある

これらはいずれも本タスクの宣言スコープ(装備会計)を大きく超えるため、既存のDivision削除アーキテクチャを尊重し、削除される装備の会計処理だけを是正する方針を採った。

## 8. 正規Division削除分類

| Division終了理由 | current equipment | 国家備蓄 | Division削除 |
| --- | --- | --- | --- |
| 正式な生存者復員(P25-006) | 全量返却(`try_return_equipment`) | 返却分増加 | 削除 |
| 通常戦闘による全滅(P25-004) | 全量破壊(`try_destroy_equipment`、既に確立済み) | 不変 | 削除 |
| 撤退追加casualtyによる全滅(本タスクで新規) | 全量破壊 | 不変 | 削除 |
| 撤退失敗等による強制解散(本タスクで新規) | 残存分も全量破壊 | 不変 | 削除 |
| 国家消滅(本タスクで新規) | 残存分を全量破壊(preflight済み、原子的) | 消滅国・併合国とも不変 | 削除 |
| save/load時のワールド置換 | 会計遷移ではない(保存値をそのまま復元) | 保存値を復元 | 旧ワールド破棄 |
| テスト・セットアップ削除 | 本番会計対象外 | テスト仕様による | 削除 |

## 9. 削除直前equipment=0不変条件

本番のDivision削除経路(復員を除く全て)で、削除直前に必ず`division.equipment == 0`となることを保証するよう実装した。0への遷移方法は理由によって区別する。

```text
正式復員:   try_return_equipment → 国家備蓄へ移動 → equipment=0 → remove
強制削除:   try_destroy_equipment → 国家備蓄は不変 → equipment=0 → remove
```

## 10. 正式復員と強制削除の差

`try_return_equipment`(国家備蓄が増える)と`try_destroy_equipment`(国家備蓄に一切触れない)は、既存のP25-002-REV-001/P25-004/P25-006で確立済みの別APIのままであり、本タスクで混同・複製していない。強制削除用の呼び出しは、正式復員経路(`military::demobilization::demobilize_division`)から一切呼ばれないことを確認済み(`demobilization.rs`は本タスクで無変更、§21で再検証)。

## 11. 国家消滅時の全装備破壊

`apply_country_elimination`の既存Division解散ループ内で、`military_registry.remove_division(division_id)`を呼ぶ直前に、常に`try_destroy_equipment(&mut division.equipment, division.equipment)`を呼ぶよう変更した(`destroyed = 0.0`は既存どおり安全なno-op)。

**実装上の注意**: 当初`if remaining_equipment > 0.0 { try_destroy_equipment(...) }`という条件付き呼び出しにしていたが、これは実際のバグだった(§20参照)。最終的には常に呼び出す形へ修正した。

## 12. 消滅国stockpile凍結

`apply_country_elimination`は`CountryStockpile`へ一切書き込まない(§0.5の新規preflightも含め、読み取りのみ)。消滅国家の`CountryStockpile[MilitaryEquipment]`はP25-001設計どおり凍結されたまま(この部分はP25-006時点から一貫して正しかった)。

## 13. 併合国stockpile不変

同様に、併合国の`CountryStockpile`にも一切触れない。テスト(`apply_destroys_all_equipment_of_disbanded_divisions_and_leaves_stockpiles_untouched`)で確認済み。

## 14. 世界装備減少量

破壊は`try_destroy_equipment`が唯一の書き込み経路であり、国家備蓄には一切触れないため、世界装備総量の減少量は、消滅国家が所有していた全Divisionの`current_equipment`の合計と厳密に一致する(fixed-point units単位で完全一致、テストで確認)。

## 15. 国家消滅preflightと原子性

`apply_country_elimination`内、既存の「§0 アトミック性事前検証(他Warのlineage検証)」の直後、いかなるRegistryも変更する前に、新設した「§0.5」ステップとして次を実施する。

```text
消滅国家所有の全DivisionをDivisionId昇順で収集
→ 各Divisionのequipment状態をvalidate_equipment_stateで検証
→ 1件でも不正ならCountryEliminationError::InvalidDivisionEquipment(DivisionId, EquipmentAccountingError)を返す
→ この時点でいかなるRegistryも変更されていない
```

**設計判断**: `preflight_country_elimination`関数自体(公開APIで20箇所から呼ばれる)の署名は変更しなかった。`apply_country_elimination`は元々`military_registry: &mut MilitaryRegistry`を既に引数に持っていたため、新しいDivision装備検証ステップを`apply_country_elimination`の内部(既存の「TOCTOU再検証」と同じ地点)へ追加するだけで、`preflight_country_elimination`の呼び出し元20箇所(内部テスト18件+`war::peace.rs`+`diplomacy::crisis_response.rs`)を一切変更せずに済んだ。preflight失敗時、州所有権移転・国家status遷移・PowerTier除外・Claim/Crisis/Justification清掃・Division削除のいずれも実行されないことをテストで確認済み。

## 16. 撤退追加casualtyの取得方法

`resolve_participant_outcome`内、既存の`retreat_loss = (max_manpower × RETREAT_MANPOWER_LOSS_RATIO) as u64`計算はそのまま維持し、`remaining_manpower = manpower_before.saturating_sub(retreat_loss)`によるclamp後の実減少量を`actual_casualty = manpower_before - remaining_manpower`として使う。P25-004の`apply_combat_casualty`と同じ「予定値ではなく実際に適用された量を使う」原則をそのまま踏襲した。

## 17. 撤退装備損失式

生存する場合(撤退先あり、`remaining_manpower > 0`)は、P25-004で確立済みの`calculate_proportional_equipment_loss(equipment_before, manpower_before, actual_casualty)`をそのまま再利用し、損失式(`D = floor((E×C + floor(M/2)) / M)`)を撤退処理へ複製していない。

## 18. 通常戦闘損失との適用順序

同日内で、`process_daily_battles`(通常戦闘casualty+装備損失、既存のP25-004実装)が先に実行され、`resolve_finished_battles`(撤退追加casualty+装備損失)は必ずその後に実行される(既存の`handle_daily_military`呼び出し順序は無変更)。撤退装備損失の計算は、`resolve_participant_outcome`が読み取る時点の`division.manpower`/`division.equipment`(=同日の通常戦闘適用後の値)を基準にする——元のM/Eから再計算しない。テスト(`retreat_test_no_double_counting_with_same_day_normal_combat_loss`)で、`process_daily_battles`単独実行後の中間状態を捕捉し、`calculate_proportional_equipment_loss`で独立に再計算した期待値と実際の最終状態が一致することを確認した(二重計上していないことの直接証明)。

## 19. 撤退全滅時の装備

撤退追加casualty自体で`remaining_manpower == 0`になる場合、丸めに関わらず数学的に必ず`D = E`(現在装備を全損)になることをP25-004と同じ恒等式で保証している(`floor(M/2) < M`が`M >= 1`で常に成立するため)。テスト(`retreat_test_16_4_full_annihilation_by_retreat_casualty_destroys_all_equipment`)で確認済み。

## 20. 強制削除時の残存装備

**実際のバグとその修正**: 当初、強制削除分岐(撤退先なし、または`remaining_manpower==0`)および戦闘全滅の防御的分岐(`manpower==0`)の両方で、`if remaining_equipment > 0.0 { try_destroy_equipment(...) }`という条件付き呼び出しを実装していた。しかし`NaN > 0.0`はRustで常に`false`を返すため、`equipment`がNaN(現在の正常経路では発生しないはずの破損状態)の場合、この条件が偽になって`try_destroy_equipment`の呼び出し自体がスキップされ、検証を一切経ずにDivisionが削除されてしまうという実際のバグだった。本タスク自身が書いた原子性テスト(`retreat_test_atomicity_forced_removal_branch_does_not_remove_on_equipment_failure`)がこれを検出し、常に`try_destroy_equipment`を呼ぶ(destroyed=0.0は既存どおり安全なno-op)形へ修正した。`src/country/elimination.rs`の解散ループにも同型のガードがあったため、同様に修正した(こちらはpreflightで既に排除されるため実際には到達不能だが、同じ安全な呼び出し方へ統一した)。

**設計判断(2段階ではなく1段階の全損)**: 撤退先が無いが撤退追加casualty自体はDivisionを全滅させない場合(`remaining_manpower > 0`かつ`retreat_destination.is_none()`)、「撤退casualtyに比例する分(D)を破壊 → 残った分(E-D)をさらに破壊」という2段階ではなく、「現在装備(E)をまとめて1回で全損」という単純な実装を採用した。両者は最終的な観測可能な状態(`equipment=0`、国家備蓄不変、Division削除)が完全に同一であり、後者の方が呼び出し回数が少なく理論上の失敗窓も小さいため、あえて単純な実装を選んだ(仕様書の数値例§9.3が示す「比例損失適用後に残る装備も全損」という説明は、この2つの事象が必ずしも同一の撤退評価内で発生するとは限らない一般的な説明であり、単一の強制削除イベント内で1回にまとめて全損させても仕様の要求を満たすと判断した)。

## 21. 二重計上防止

通常戦闘損失(`process_daily_battles`)と撤退追加損失(`resolve_finished_battles`)は、それぞれ独立したイベントとして1回だけ適用される。撤退追加損失は必ず「通常戦闘適用後」のmanpower/equipmentを基準に計算するため、元のM/Eから再計算されることはなく、同じcasualty単位が2回計上されることはない(§18のテストで直接証明)。国家消滅の装備破壊も、`apply_country_elimination`という単一のトランザクション内で各Divisionにつき1回だけ`try_destroy_equipment`が呼ばれる。

## 22. casualty・organization・RNG不変

`RETREAT_MANPOWER_LOSS_RATIO`の値、manpower丸め計算、attacker/defender判定、撤退可否判定、撤退先選択ロジック、organization計算式、勝敗判定、戦闘終了日、movement、乱数呼び出し(本コードベースの戦闘計算はそもそも決定的で乱数を使用しない)は一切変更していない。テスト(`retreat_test_equipment_amount_does_not_affect_manpower_or_status_outcome`、`test_equipment_amount_does_not_change_retreat_manpower_or_status_via_full_app`)で、装備量が異なる2つのシナリオが同一のmanpower/status結果を返すことを確認し、既存のP21-siege回帰テスト(`test_solo_attacker_cannot_break_well_defended_state`等)が無変更のまま全て合格することも確認した。

## 23. P25-006正式復員の維持

`src/military/demobilization.rs`は本タスクで一切変更していない(git diff空)。P25-006の22件のDomain単体テスト+29件のE2Eテスト(`tests/p25_006_demobilization_equipment_return_e2e_test.rs`)は全て無変更のロジックのまま再検証し、全合格した。

## 24. New Game／Loaded Game比較

* 撤退: `test_new_game_and_loaded_game_produce_identical_retreat_equipment_results`で、New Game直後の撤退結果とNew Game→save→load後の撤退結果(manpower・equipment・status)が完全一致することを確認。
* 国家消滅: `test_elimination_new_game_and_loaded_game_produce_identical_equipment_results`で、同様の一致を確認。

## 25. save/load分岐比較

* 撤退前分岐: `test_pre_retreat_save_load_branch_matches_the_uninterrupted_run`
* 撤退後: `test_post_retreat_equipment_loss_survives_save_and_load`(二重適用なし)
* 国家消滅: New Game/Loaded Game比較テスト内でsave/loadを経由した消滅結果も検証済み

## 26. Phase 23・24会計の保護

`p23_006_manpower_ledger_e2e_test`(22件)・`p23_007_division_demobilization_e2e_test`(24件)・`p23_008_daily_reinforcement_e2e_test`(21件)・`p23_010_phase23_manpower_lifecycle_audit_test`(9件)・`p24_002_workforce_rounding_unification_e2e_test`(17件)、いずれも全合格・対象ソース無変更。

## 27. P25-003〜P25-006回帰結果

`p25_002_equipment_accounting_foundation_e2e_test`(15件)・`p25_002_rev_001_fixed_point_equipment_accounting_e2e_test`(6件)・`p25_003_recruitment_equipment_issue_e2e_test`(22件)・`p25_004_combat_equipment_loss_e2e_test`(5件)・`p25_005_daily_equipment_reinforcement_e2e_test`(10件)・`p25_006_demobilization_equipment_return_e2e_test`(29件)、いずれも全合格。

## 28. 対象外を先行実装していないこと

国家消滅Divisionの凍結保持・inactive Division状態・国家復活時のDivision再生成・鹵獲・併合国への装備移転・降伏装備の一部回収・国家消滅時の装備返却・装備損失原因別の永続統計・装備種別/品質・戦闘力への装備充足率反映・UI表示・AI判断変更・補給線・修理/回収・Attrition・負傷者/疾病・戦死者population連動・Save V2/migration・P25-007以降の先行実装は、いずれも行っていない。

## 29. Critical／High／Medium／Low

**Critical: 0件、High: 0件(P25-006で確認された2件は本タスクで解消済み、§30参照)、Medium: 0件、Low: 0件**

## 30. 解消したfinding

1. 国家消滅時、非ゼロ装備を持つDivisionが装備会計を経由せず物理削除される → `try_destroy_equipment`+preflight原子性で解消。
2. 撤退時追加manpower損失により、非ゼロ装備を持つDivisionが装備会計を経由せず削除され得る → `resolve_participant_outcome`の全分岐で`try_destroy_equipment`を経由するよう解消。

さらに、本タスク自身の実装中に新規発見・即時修正した実バグ1件(§20参照、`NaN > 0.0`ガードによる検証スキップ)も記録する。

## 31. 追加・変更したファイル

* `src/country/elimination.rs`(§0.5 preflight追加、`InvalidDivisionEquipment`エラー変種追加、解散ループでの装備破壊追加)
* `src/military/update.rs`(`resolve_participant_outcome`の撤退・全滅防御分岐を装備会計対応へ拡張)
* `assets/localization/en-US.ron`・`assets/localization/ja-JP.ron`(新規エラーキー`country_error.elimination.invalid_division_equipment`)
* `tests/p25_006_demobilization_equipment_return_e2e_test.rs`(既存2テスト+ファイルヘッダのコメントを訂正後の理解へ更新、assertionは無変更)
* `tests/p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs`(新規、8件)
* 本報告書(新規)

## 32. 実行した全コマンドと正確な結果

```bash
git rev-parse HEAD                          # 7b20ca46b8d86cc83b9d05da6c56fa44bad5855a
git status --short                          # M saves/savegame_v1.ron のみ
rg -n "RETREAT_MANPOWER_LOSS_RATIO|remove_division|despawn|eliminate_country|Eliminated" src tests
rg --files tests | sort | rg 'p21_|p22_|p23_006|p23_007|p23_008|p23_010|p24_002|p25_00'

cargo check --lib
cargo test --lib military::demobilization::     # 22 passed
cargo test --lib military::update::             # 26 passed
cargo test --lib military::                     # 263 passed
cargo test --lib country::elimination::         # 20 passed
cargo test --lib country::                      # 79 passed
cargo test --lib war::                          # 153 passed
cargo test --lib diplomacy::                    # 201 passed

cargo test --test p21_021_country_elimination_e2e_test               # 1 passed
cargo test --test p22_002_world_conclusion_e2e_test                  # 1 passed
cargo test --test p22_003_terminal_state_load_e2e_test               # 7 passed
cargo test --test p23_006_manpower_ledger_e2e_test                   # 22 passed
cargo test --test p23_007_division_demobilization_e2e_test           # 24 passed
cargo test --test p23_008_daily_reinforcement_e2e_test               # 21 passed
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test      # 9 passed
cargo test --test p24_002_workforce_rounding_unification_e2e_test    # 17 passed
cargo test --test p25_002_equipment_accounting_foundation_e2e_test   # 15 passed
cargo test --test p25_002_rev_001_fixed_point_equipment_accounting_e2e_test # 6 passed
cargo test --test p25_003_recruitment_equipment_issue_e2e_test       # 22 passed
cargo test --test p25_004_combat_equipment_loss_e2e_test             # 5 passed
cargo test --test p25_005_daily_equipment_reinforcement_e2e_test     # 10 passed
cargo test --test p25_006_demobilization_equipment_return_e2e_test   # 29 passed
cargo test --test p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test # 8 passed

cargo test --all-targets                     # 1792 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings  # 0 warnings

# clippyがtype_complexityとquestion_markを検出したため、テストファイルへ
# #![allow(clippy::type_complexity)]追加とmatch→?簡略化を実施後、
# cargo test --all-targetsを再実行して最終確認(結果は同じ1792 passed/0 failed/1 ignored)

git checkout -- (非決定的PNG 9枚を個別に)
```

全て逐次実行。並列実行なし。`cargo clean`は使用していない(incremental cache破損の兆候はなかった)。

## 33. 全テストpassed／failed／ignored

* 開始時基準: `1769 passed / 0 failed / 1 ignored`
* 最終`cargo test --all-targets`結果: **`1792 passed / 0 failed / 1 ignored`**
* 新規追加: **23件**(`src/country/elimination.rs`内部3件、`src/military/update.rs`内部12件、新規e2eファイル8件)

## 34. clippy警告数

**0件**(`cargo clippy --all-targets --all-features -- -D warnings`)。

## 35. Cargo逐次実行

厳守。並列実行なし。

## 36. ユーザーセーブの保護

`saves/savegame_v1.ron`は全工程で未読み書き・未ステージ・未復元。

## 37. 監査対象コードの`git status --short`

```text
 M assets/localization/en-US.ron
 M assets/localization/ja-JP.ron
 M saves/savegame_v1.ron
 M src/country/elimination.rs
 M src/military/update.rs
 M tests/p25_006_demobilization_equipment_return_e2e_test.rs
?? tests/p25_006_rev_001_forced_division_removal_equipment_accounting_e2e_test.rs
```

## 38. P25-007への引き継ぎ事項

* 装備充足率(`equipment / max_equipment`)を戦闘力(attack/defense)へどう反映するか
* `max_equipment == 0`のDivisionの扱い(ゼロ除算回避)
* manpower充足率と装備充足率の合成方法(単一`min()`への統合は避けるか要検討)
* organization・movement・RNGへ影響させないこと
* P25-004の戦闘装備損失、本タスクで追加した撤退装備損失、P25-005の日次装備補充、P25-006の正式復員返却、いずれも維持すること
* 国家消滅・強制削除時は装備全損であること(本タスクで確立)
