# P27-006 低実効戦力の損害連続性 実装報告

## 1. 総合判定

**P27-006 COMPLETE**(P27-005の手動プレイ確認は依然として未実施。本タスクの自動テストは手動確認の代わりにならない)。

整数スケール不整合による損害の不連続(M)、正の入力が整数切り捨てで実効戦力0になる問題(T)、全対象のweightが0のときの損害消滅(C)を、1つの原子的変更として修正した。両陣営の真の実効戦力0(M-2)は本タスクの対象外で、未解決のMedium findingとして残る(§32)。

## 2. 開始時HEAD

`30306b0e30760316b5b4c2ddabab823014882fb2`。`git status --short`は空。開始時基準は2644 passed / 0 failed / 1 ignored、clippy 0。

## 3. 実装コミット

本報告書と同じ1コミット(`fix: stabilize low-power combat damage`)。ハッシュはコミット後の最終報告に記載する。

## 4. 最終HEAD

同上(最終報告に記載)。

## 5. P27-006-PRE-001から採用した設計

M・T・Cの3点をそのまま採用した。監査用の一時パッチは複写せず、現在の本番コードへ最小変更として実装した。候補2(常時manpower比配分)と、×1000スケールへの全面変更は採用していない。

## 6. 変更ファイル

| ファイル | 内容 |
|---|---|
| src/military/combat_calc.rs | M・T・Cの実装、ヘルパー2件、`distribute_damage`の引数追加、単体テスト26件 |
| tests/p27_006_low_effective_power_damage_e2e_test.rs(新規) | 実AppのE2E 10件 |
| tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs | 90日テストの最終部を実測の新正規値へ更新 |
| tests/p27_005_offensive_frontline_supply_bridge_e2e_test.rs | test_03・test_04のfixture調整 |
| 本報告書 | |

`daily_supply.rs`(P27-005の補給資格)、`update.rs`(戦闘終了・撤退)、Save関連、`tests/land_war_combat_peace_test.rs`、`saves/savegame_v1.ron`は変更していない。

## 7. Mの実装

損害総量の計算を新ヘルパー`damage_from(attacker_effective, target_effective)`へ集約し、`attacker_effective × DAMAGE_SCALE / max(target_effective, 1)`とした。1対1(`resolve_combat_day_with_supply`)、複数師団の2関数(`resolve_combat_day_multi`、`resolve_combat_day_multi_with_supply`)の計6か所の分岐を置き換えた。

## 8. Tの実装

新ヘルパー`effective_power_with_floor(power, readiness_1000, org_ratio_1000)`。計算結果が0で、`power>0`かつ`readiness_1000>0`かつ`org_ratio_1000>0`のときだけ1を返す。3か所(1対1の攻撃側・防御側、`effective_power_1000_with_supply`)へ適用した。

## 9. Cの実装

`distribute_damage`に`manpowers`引数を追加した。`total_effective>0`なら従来どおり実効戦力比で配分し、`total_effective<=0`のときだけ、正のcurrent manpowerをweightとして配分する。配分本体は`allocate_by_weight`へ切り出した(IDの昇順、端数は最大IDが受け取る既存規則をそのまま維持。乗算だけi128にした)。

## 10. M・T・Cの分離不能性

PRE-001で実測した。Mだけでは、正の入力が整数切り捨てで0になる境界が残る。Tだけでは、実効戦力0のときの分母1000特例が残る。Cがないと、複数師団の全員のweightが0の場合、損害総量が正でも全量が消える。3点を合わせて初めて、1対1・複数師団の両経路が修正される。

## 11. 旧分母1000分岐の削除

実効戦力が1未満のときに分母を1000にする4系統の`if ... < 1`分岐をすべて削除した。

## 12. 分母下限1

`max(target_effective, 1)`。負値も1になる(単体テスト`m_damage_from_floors_the_denominator_at_one_and_never_goes_negative`)。攻撃側の実効戦力が0なら損害は0のまま(損害の最低値を無条件に1にはしていない)。

## 13. 正の入力のeffective 1化

power・readiness・組織率がすべて正で計算結果が0のときだけ1。例: 10×300×200/1,000,000=0.6→0が1になる。

## 14. 真の0の保護

power 0、readiness 0(Equipment 0、manpower 0、supply 0のいずれか)、組織率0は0のまま(単体テスト`t_true_zero_inputs_stay_zero`、`t_true_zero_readiness_from_equipment_manpower_or_supply_stays_zero`、E2E test_05・06・07・08)。readiness・supply・Equipment・manpowerの各比率へ最低値は与えていない。

## 15. 通常weight配分

`total_effective>0`なら従来どおり。1対2で防御A(補給1)・B(Equipment 0)の場合、(66, 0)を維持する(単体テスト`multi_partial_zero_defender_keeps_the_audited_canonical_split`、E2E test_04)。常時manpower比なら(33, 33)になるため、それを不採用とした設計どおり。

## 16. manpowerフォールバック

`total_effective<=0`のとき、manpower>0の対象だけがweightを持つ。同manpowerなら端数は最大IDが受け取る(2と3に分かれる例をテスト済み)。manpower 0の師団はフォールバックの対象外。

## 17. damage保存則

フォールバック時も`Σ配分=total_loss`(単体テスト`c_damage_is_conserved_and_independent_of_input_order`で5通りの損害量、E2E test_03で実際の適用結果を確認)。overkillは既存の`saturating_sub`で切り捨てられる(変更なし)。

## 18. manpower保存則

損害適用(`apply_combat_casualty`)は無変更。E2Eで、防御側の戦闘損害合計が総損失と一致することを確認した(test_03: 1000)。負値・NaN・無限大は発生しない(E2E test_01で全日のfulfillmentが有限)。

## 19. Equipment保存則

無変更。実際の人員損失に比例して装備を破壊する既存式のまま。

## 20. 256件グリッド

P27-006-PRE-001の256件(攻撃側・防御側とも manpower {10000,8000,5000,3000}×組織率{50,35,20,10})を、旧実装の損害式(テスト内に複写)と新実装で比較する単体テスト`grid_256_only_truncation_to_zero_cases_change`として恒久化した。

## 21. 差分31件

**31件**(監査値と一致)。31件はすべて、旧実装で片側の実効戦力が整数の切り捨てで0になる領域(テスト内で検証)。

## 22. 通常領域225件不変

**225件が完全一致**。

## 23. 0.066/0.067境界

旧: 防御側の補給0〜0.066では損害2、0.067から2000(1000倍のジャンプ)。新: 0、0.000001、0.05、0.065、0.066、0.067、0.0671、0.1のすべてで2000。0.3で500、1.0で133へ連続的に下がる(単体テスト`m_supply_boundary_066_067_has_no_thousandfold_jump`)。

## 24. 1対1結果(E2E test_01、test_02)

- 低正値(双方 manpower 3000・装備比0.3・組織率10、補給は双方正): 旧は両者の実効戦力が0で損害0のまま膠着。新は実効戦力1同士で日次100の損害となり、初日に双方の組織率が0になって撤退する(各1900)。controllerは変わらない。
- P26-008-Bの膠着状態(攻撃側の実効戦力3、防御側の補給0): 1日目に防御側は損害300(組織率20)、2日目に撤退して州3をArcadiaが占領する。

## 25. 複数Division結果

1対2で、防御側が補給0の場合は500ずつ(E2E test_03、単体テスト)。2対1、2対2(攻撃補給あり・防御全員0)でも損害が入る(PRE-001の測定と同一)。

## 26. 全weight 0結果

E2E test_03: 各防御師団が500の損害(manpower比)。組織率0で撤退し各-1000(最大manpowerの10%)。有限日数(1日)で戦闘が終わり、州3をArcadiaが占領する。

## 27. 一部weight 0結果

E2E test_04: 防御A(15)が66の損害を全て受け、B(Equipment 0)は無傷。攻撃側は150の損害。戦闘継続。

## 28. Equipment 0

E2E test_05: 防御側は真のreadiness 0。攻撃側は無傷(outgoing 0)、防御側は1000の損害を受けて撤退する。Tは1にしない。

## 29. supply 0

E2E test_06: 防御側の補給0なら防御側だけが損害(攻撃側は無傷)。攻撃側の補給0(国家備蓄0で前線補給も届かない)なら攻撃側が1500の損害を受けて撤退し、防御側は無傷。どちらも無敵にならない。

## 30. manpower 0

E2E test_07: manpower 0の師団は既存の消滅処理で戦闘初日に除去される。架空のweightは作られず、生存師団が通常の損害(66)を全て受ける。panicなし。

## 31. 両陣営真の0

E2E test_08、単体テスト`multi_both_sides_truly_zero_deal_and_take_no_damage_known_followup`: 双方の補給0(国家備蓄0)で40日間、戦闘は維持され、manpower・組織率・controllerは不変、Divisionは消滅しない、架空のdamageも勝者も生まれない。

Known follow-up: P27-007-PRE-001で終了・撤退契約を決定する。

## 32. M-2延期

上記のとおり。P27-006には含めていない。未解決のMedium findingとして維持する。

## 33. P26-008-B更新

最終60日を1日ずつ観測するようにし、補給・損害・戦闘終了・controller・stance・blockedを個別に検証する。旧アサーション(Defend、blocked=true、州3の戦闘が未決着)は、新しい実測値へ置き換えた。

## 34. 決着の実測日

補給: 攻撃側は戦闘中ずっと正、防御側は0。戦闘の再開: offset 19(1800-01-17起点)。損害: 防御側は最小7600まで減少。決着とcontroller変更: 翌日offset 20に州3をArcadiaが占領。防御師団の消滅(追撃後): offset 30。最終stance=Stopped、blocked=false。

PRE-001報告書の「d19決着」は、戦闘が再開する日をd19と呼んだものに相当する。決着日(占領)はd20だった。この差は本タスクで実測して確定した。

## 35. P27-005 test_03更新

**理由**: P27-006以降、通常の戦闘は数日で決着するため、遮断開始(d20)より前に戦闘が終わる。以前は防御側の損害がほぼ0で戦闘が25日以上膠着していたことに依存していた。3種類の戦闘日の組合せ(遮断開始日d10〜d28、防御側追加0/1)を試したが、正常な戦闘は高々数日しか続かず、25日間の交戦を維持できなかった。

**調整**: 攻撃側の攻撃力と防御側の防御力を0にする(`Cfg::neutral_power`)ことで、損害の生じない状態で戦闘を保持した。調整したのは攻撃力・防御力のみ。戦闘開始日・遮断日・防御側manpowerは変更していない。本番コードへのテスト専用例外はない。P27-005の補給資格判定は変更していない。

**検証内容(弱めていない)**: d11〜d19は前線補給で満額(9日)、d20〜d44の25日間だけ補給0、補給0の日は遮断期間と完全に一致、d45以降は自然回復。

**注意**: このfixtureは、両陣営のoutgoingが0のときに戦闘が維持される現行挙動(M-2の既知状態)に依存する。P27-007-PRE-001で終了契約が決まったら再評価する(テストのコメントに明記)。

## 36. P27-005 test_04更新

同じfixture調整。首都陥落後(d20〜)に補給0が40日以上維持されること(実測値をメッセージに含める)、陥落前は正であることを検証する。

## 37. 回廊遮断25日

上記のとおりexact 25日を直接検証している。

## 38. 回廊再接続

d45以降の戦闘中の全日で補給が0.99を超える(自然回復)。

## 39. 90日結果

P26-008-Bの90日シナリオ(実App)と、新規E2E(§24〜§31)で確認した。

## 40. 36か月結果

P27-002-A/B(36か月)の実App検証は変更後も通過。一時的な発火計測(計測コードは削除済み)で、M・T・Cの発火は**0件**だった。自然な通常AI運用では戦闘が発生しない。

## 41. 120か月結果

P27-002-C(90日・36・60・120か月、16件)は通過(debug、独立プロセス5回)。計測の結果、M・T・Cは0件発火。制御シナリオでは同じ計測でM 83回、T 48回、C 38回が発火し、計測自体が機能することを確認した。制御シナリオの成立と自然発火は区別する。

## 42. Save V1/V2判断

Save V1を維持する。シリアライズされるフィールドの追加・変更はなく、migrationは不要。

## 43. save/load結果

E2E test_10: 低実効戦力、実効戦力3、全weight 0、一部weight 0、両陣営0(維持)の5種のシナリオについて、戦闘開始前・損害適用後・controller変更後・維持中の分岐点で、save→loadした結果が連続実行と完全一致する。同じ分岐点で2回連続のsave→loadも一致する。`save::`の305件も通過。

## 44. 決定性

独立プロセス5回、全テスト一致。E2E test_09で、独立2回の完全一致と、防御側の追加順を反転した場合(6:4のmanpowerで600/400に配分)の結果が同一であることを確認した。配分はDivisionId昇順で、HashMap反復順に依存しない。

## 45. 国家間分離

配分は戦闘ごと・陣営ごとに閉じており、当該戦闘の参加者リストだけを対象とする(単体テスト`c_manpowers_of_unrelated_divisions_are_ignored`)。

## 46. P27-005保護

`daily_supply.rs`は無変更。P27-005の10件のE2E、単体44件が通過。攻撃側のbridge補給(E2E test_02で防御側0・攻撃側1.0を確認)、1ホップ、連鎖なし、回廊遮断と再接続、首都陥落はP27-005のテストが引き続き検証する。手動プレイ確認は未実施。

## 47. Phase 23〜27回帰

全体テスト2680件が通過し、変更した既存テストは§52の3件のみ。

## 48. Critical/High/Medium/Low

- Critical: 0
- High: 0
- Medium: 1(M-2: 両陣営の真の実効戦力0で戦闘が維持される件。解消したM-1とは別)
- Low: 1(`daily_supply.rs`の既存整形差分。P27-005から継続、変更なし)

M-1(損害式の不連続と全weight 0)は解消した。

## 49. 次の原子的タスク

**P27-007-PRE-001**: 両陣営の真の実効戦力0に対する、戦闘終了・撤退・回復待ち契約の設計監査。候補: (a)補給が復帰するまで待つ、(b)両陣営0のとき攻撃側を出撃元へ撤退させる、(c)最低戦闘力を設ける。

## 50. 単体テスト

26件(`military::combat_calc::p27_006_tests`): M 6件、T 6件、C 11件、複数師団の実際の経路3件、256件グリッド1件(M: 分母下限1、負値、攻撃側0、オーバーフロー、境界0.066/0.067。T: 正入力、真の0、既存値不変、最大値、境界、Equipment/manpower/supply 0。C: 通常weight、全0、単一0、tie-break、一部0で通常維持、全manpower 0、manpower 0の除外、空・損害0、保存則と順序反転、無関係な師団)。

## 51. E2Eテスト

10件(`tests/p27_006_low_effective_power_damage_e2e_test.rs`): 低正値1対1、P26-008-B再現、全weight 0、一部weight 0、Equipment 0、supply 0(防御側・攻撃側)、manpower 0、両陣営0、決定性、save→load。

## 52. 既存テスト変更と理由

3件のみ。(1) P26-008-B 90日テスト: 膠着が解消するため、最終アサーションを新実測値へ更新(§33)。(2)(3) P27-005 test_03・test_04: 戦闘が遮断前に決着するため、fixtureを調整(§35)。それ以外の既存テストは変更していない。

## 53. 全テスト件数

**2680 passed / 0 failed / 1 ignored**(開始時2644 + 単体26 + E2E 10)。

## 54. clippy

`cargo clippy --all-targets --all-features -- -D warnings`: 警告0。

## 55. Cargo逐次実行

`military::combat_calc` 75、`military::combat` 75(部分一致で同じ75件)、`military::daily_supply` 44、`military::movement` 12、`military::frontline` 0(該当テストなし)、`save::` 305、新規E2E 10、P26-005 10、P26-006-REV-001 9、P26-008-B 19、P27-002-C 16、P27-004 6、P27-005 10、land war 4、すべて通過。並列実行はしていない。全体実行中に一度、Windowsのページファイル不足(os error 1455)でコンパイルが中断したため、`-j 2`で並列度を下げて再実行した(コードの問題ではない)。

## 56. 独立プロセス

P27-006 E2E、P26-008-B、P27-005、P26-005、P26-006-REV-001、land war、P27-002-C(120か月、debug)、P27-004を各5回、独立プロセスで実行し、全回で件数・結果が一致した。

## 57. P27-005手動確認未実施

P27-005の手動プレイ確認は未実施のまま。

## 58. ユーザーSave保護

`saves/savegame_v1.ron`は読み書き・復元・ステージのいずれも行っていない。

## 59. PNG差分処理

テストで再生成された既知のPNGを、個別指定の`git checkout -- <path>`で復元した(全体テスト・独立実行の各後)。コミットには含めていない。

## 60. コミット対象

`src/military/combat_calc.rs`、`tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs`、`tests/p27_005_offensive_frontline_supply_bridge_e2e_test.rs`、`tests/p27_006_low_effective_power_damage_e2e_test.rs`、本報告書。

## 61. 最終`git status --short`

コミット後は空(最終報告に実測値を記載する)。
