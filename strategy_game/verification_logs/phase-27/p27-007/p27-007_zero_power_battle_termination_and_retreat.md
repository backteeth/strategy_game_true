# P27-007 両陣営ゼロ損害戦闘の終了・攻撃側撤退 実装報告

## 1. 総合判定

**P27-007 COMPLETE**。P27-007-PRE-001で確定した候補1(両陣営ゼロ損害の戦闘を即日・防御側勝利・攻撃側撤退として終了する)を実装した。P27-005 test_03・test_04は補給専用テストへ再設計し、P27-006 test_08は新契約へ更新した。

## 2. 開始時HEAD

`26f443288b9d973ba4d0856236d228d1060fea21`。`git status --short`は空。

## 3. 実装対象

`src/military/update.rs`のみ。`combat_calc.rs`・`daily_supply.rs`の本番ロジックは変更していない。

## 4. 当日損害結果を使う理由

`process_daily_battles`が`resolve_combat_day_multi_with_supply`から受け取る`atk_results`/`def_results`(その日実際に適用される`manpower_loss`)を、状態を一切変更する前にそのまま判定に使う。`atk_results`は攻撃側が受ける損害(防御側の実効戦力に比例)、`def_results`は防御側が受ける損害(攻撃側の実効戦力に比例)であり、両方が全員0であることは「双方の合計実効戦力が0」と数学的に同値である(P27-006の`damage_from`により、分子が0なら分母に関わらず結果は常に0)。実効戦力を別途再計算する必要はない。

## 5. organization事後判定を採用しない理由

`resolve_finished_battles`側で適用後のDivision状態(特にorganization)から事後的に判定すると、その日の戦闘で両者が同時に組織率0まで削り合う正常な相討ちを、誤って両陣営ゼロ損害と混同する。実装中に実際にこの罠を踏んで特定した(初期実装で1v1の相討ちシナリオが1件だけ誤って失敗した)。`process_daily_battles`内で状態変更前の値を使うことで回避した。

## 6. ゼロ損害終了条件

```text
attacker_received_damage = atk_results内のいずれかのmanpower_loss > 0
defender_received_damage = def_results内のいずれかのmanpower_loss > 0
zero_damage_today        = !attacker_received_damage && !defender_received_damage
```

参加者が空(片方全滅済み)または`readiness_invalid`でスキップされた戦闘は、この判定へ到達しない(既存の`continue`により`resolve_combat_day_multi_with_supply`自体が呼ばれない)。

## 7. 防御側勝利処理

既存の「攻撃側全滅→防御側勝利」分岐と同じ状態変更(`BattleStatus::DefenderWon`、防御側`status=Idle`・`combat_id=None`、controller不変)を、ゼロ損害終了でも同じコードパスで適用する(新規分岐だが、同じ状態変更を複製)。

## 8. 攻撃側撤退処理

既存の`resolve_participant_outcome`の撤退分岐(組織率0撤退)を`force_retreat_or_destroy`として独立させ、両方の経路(組織率0による脱落・両陣営ゼロ損害による強制撤退)から共有する。撤退ペナルティ(最大manpowerの10%)・比例装備損失は既存式のまま。

## 9. `attacker_origins`の再利用

新規フィールドは追加していない。既存の`Battle.attacker_origins: HashMap<DivisionId, StateId>`をそのまま退却先として使う。

## 10. 退路なし処理

`attacker_origins.get(&division_id)`が`None`の場合、`force_retreat_or_destroy`の既存の消滅分岐(装備全損→`remove_division`)がそのまま適用される。新しい降伏・捕虜処理は追加していない。

## 11. 状態変更の実行順

`handle_daily_military`内で`process_daily_battles`と`resolve_finished_battles`は同じ関数呼び出し内の通常のRust関数として順に実行される(別Systemではない)。当日の判定結果は`ZeroDamageBattles`(`HashSet<BattleId>`型エイリアス)として関数の戻り値→引数で直接受け渡す。

## 12. 一時情報の受け渡し方法

Bevy ResourceにもComponentにもSave DTOにもしていない。`process_daily_battles`のローカル変数として構築し、`resolve_finished_battles`の引数として渡し、その日のうちに破棄する。

## 13. Save V1判断

Save V1を維持した。新規永続フィールドはない。

## 14. 新規永続状態なし

`Battle`・`Division`のいずれのフィールドも追加・変更していない。

## 15. 単体テスト

7.1の項目は、既存の`combat_calc.rs`の単体テスト(P27-006、26件、無変更で通過)と、新規E2Eの`p27_006_low_effective_power_damage_e2e_test.rs`のtest_08・test_11〜test_13で、実App経由で相当する検証(両陣営0の即日終了、複数Divisionの一部0での非誤発火、通常相討ちとの非混同)を行っている。純粋な判定式だけを切り出した`update.rs`内の単体テストは追加していない(`process_daily_battles`/`resolve_finished_battles`がBevyの`MilitaryRegistry`等を要求する既存の統合テストスタイルに合わせた)。

## 16. E2Eテスト

- `tests/p27_006_low_effective_power_damage_e2e_test.rs`: test_08(即日終了・防御側勝利・攻撃側撤退)、test_11(複数攻撃師団が個別の出撃元へ撤退)、test_12(出撃元欠落時の安全な消滅)、test_13(登録順反転での決定性)。計13件。
- `tests/p27_007_zero_damage_battle_termination_e2e_test.rs`(新規): test_01(実AI・90日、再攻撃ループなし)、test_02(無関係な第三国への非影響)。計2件。
- `tests/p27_005_offensive_frontline_supply_bridge_e2e_test.rs`: test_03・test_04を補給専用テストへ再設計(§21参照)。

## 17. 通常相討ち保護

`p27_006`のtest_01(実効戦力1同士の相討ち、双方が同日に組織率0で撤退)で、旧実装(状態事後判定)では誤ってP27-007経路に巻き込まれ1件失敗したが、状態変更前の判定に修正後は正しく通常の組織率撤退経路(P27-007ではない)を通ることを確認した。

## 18. 一方だけ0の戦闘保護

`p27_006`のtest_01〜test_07(いずれか一方が正の実効戦力を持つケース)は全て無変更で通過。`update.rs`内の一時計測でも、1体でも正の実効戦力を持つ側がいれば`zero_damage_today`が発火しないことを確認した。

## 19. 複数Division結果

test_11(2攻撃、個別origin)、test_13(2対2、登録順反転)で確認。全員が正のweight配分・一部0の配分(P27-006の既存契約)には影響しない。

## 20. AI再攻撃ループ結果

`p27_007`のtest_01で、実際のAI(`process_daily_military_ai`)を含む実Appにより90日間観測した。両国の備蓄を同時に0にして両陣営ゼロ損害を発生させ、d30に備蓄を回復させた。結果: **90日間で終了は1回だけ**。以後、備蓄回復後も再攻撃は発生しなかった。P26-008-Bの既存の攻勢供給ゲート(ヒステリシス)が`stance`をOffensiveへ戻さないため、新しいcooldownなしで再攻撃ループが自然に防止されることを確認した。

## 21. P27-005 test_03／test_04再設計

現行fixture(`neutral_power`)はP27-007により初日で即終了するため成立しなくなった。P27-006-PRE-001のテスト案Cを採用し、実データ(Arcadia州0/1/3/4)上に`BattleRegistry`へ直接Battleを登録した攻撃側Divisionを置き、`process_daily_battles`/`resolve_finished_battles`(戦闘解決)を一切呼ばず、日次補給計算(`run_daily_military_supply_consumption_with_battles`)だけを`SystemState`経由で直接ループさせる`run_supply_only`ヘルパーへ全面的に書き換えた。実際の`process_daily_battles`を経由する通常のシナリオでは、P27-006後の戦闘が数日で決着してしまい、20日目以降の遮断を観測する前に戦闘が終わってしまうこと(複数の戦力バランスで実測して確認)を根拠に、この再設計を採用した。

## 22. 25日遮断結果

`run_supply_only`により、d1〜d19は満額(>0.99)、**d20〜d44のちょうど25日間だけ補給0**、d45以降は自然回復(>0.99)を確認した。

## 23. 再接続結果

上記のとおり。d45時点で`occupy_state`により州1・州4をArcadiaへ戻すと、即座に補給が回復する。

## 24. P27-006 test_08更新

旧: 「両陣営0を40日間維持する」テストを、新: 「初日に防御側勝利で終了し、攻撃側は`attacker_origins`(州1)へ撤退、防御側は無傷で自陣に留まる」テストへ全面的に書き換えた。以後の日はいずれも戦闘が既に終了していることを確認する。

## 25. P26-008-B回帰

`tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs`は無変更で19件全通過。90日テストのシナリオでは攻撃側・防御側のいずれか一方だけが実効戦力0になるため、P27-007の経路は発火しない(既存のP27-006経路がそのまま適用される)。

## 26. P27-004回帰

`tests/p27_004_idempotent_movement_orders_e2e_test.rs`は無変更で6件全通過。攻撃側撤退時に`destination`・`current_path`・`movement_progress`を明示的にリセットしてから`current_state`を書き換える既存の手順をそのまま再利用しているため、翌日の移動命令発行と衝突しない。

## 27. save/load結果

`p27_006`のtest_10で、`both_zero_spec`(両陣営0)のsave/loadを、終了当日(d1)・終了後(d20)の両方で検証し、連続実行と完全一致することを確認した(候補実装前後で挙動自体は変わるが、save/loadの一致性は影響を受けない)。

## 28. 決定性

test_13(登録順反転)、5回の独立プロセス再実行(§35)で確認した。

## 29. 国家間分離

`p27_007`のtest_02で、Arcadia対Elfinの戦争・両陣営ゼロ損害終了が、無関係な第三国Passguardの備蓄に影響しないことを確認した。

## 30. 36か月結果

`P27-002-A/B`の36か月実App検証は変更後も全件通過。一時計測でゼロ損害終了経路の発火は**0回**(自然な通常AI運用では戦闘自体が発生しないため)。

## 31. 120か月結果

`P27-002-C`(90/36/60/120か月、16件)も全件通過、発火**0回**。自然発火と制御シナリオの証拠は区別している(制御シナリオ=P27-007-PRE-001の一時パッチとP27-007自体のE2Eで実測済み)。

## 32. P27-005手動プレイ未実施

自動検証のみ。実プレイでの手動確認は本タスクでも実施していない。

## 33. Critical／High／Medium／Low

- Critical: 0
- High: 0
- Medium: 0(M-1・M-2とも解消)
- Low: 2(P27-007-PRE-001から継続)
  - 攻撃側撤退先の安全性チェック不足(出撃元が既に敵占領されていても検証しない、既存の組織率撤退から継承)
  - プレイヤーが強制撤退の理由をUI上で把握できない(UI通知は対象外)

## 34. 対象外

撤退先の安全性再設計、最寄り安全州探索、戦略的撤退AI、cooldown、猶予日数、UI通知、ローカライズ、戦闘履歴UI、CombatLog新設、readinessの最低値、effective powerの追加floor、P27-006損害式の再較正、前線補給資格変更、Equipment補充仕様変更、organization回復仕様変更、AI攻勢目標変更、Save V2、新規永続フィールド。いずれも実装していない。

## 35. 全コマンドと正確な結果

- `cargo test --lib military::`: 495 passed
- `cargo test --lib save::`: 305 passed
- `cargo test --test p27_005_offensive_frontline_supply_bridge_e2e_test`: 10 passed
- `cargo test --test p27_006_low_effective_power_damage_e2e_test`: 13 passed
- `cargo test --test p27_007_zero_damage_battle_termination_e2e_test`: 2 passed
- `cargo test --test p26_008_b_supply_aware_offensive_gate_e2e_test`: 19 passed
- `cargo test --test p27_004_idempotent_movement_orders_e2e_test`: 6 passed
- `cargo test --test p26_006_rev_001_ai_save_resume_determinism_e2e_test`: 9 passed
- `cargo test --test land_war_combat_peace_test`: 4 passed
- `cargo test --test p27_002_c_residual_fiscal_self_stabilization_e2e_test`: 16 passed
- `cargo test --all-targets --no-fail-fast -j 2`: **2685 passed / 0 failed / 1 ignored**
- `cargo clippy --all-targets --all-features -- -D warnings`: 警告0
- 上記の主要テストを独立プロセスで5回連続実行し、全回で件数一致を確認した(§35詳細は最終報告に記載)

Windowsのページファイル不足を避けるため、全体実行は`-j 2`で行った。

## 36. 全テスト件数

**2685 passed / 0 failed / 1 ignored**(開始時2680 + P27-006新規3 + P27-007新規2)。

## 37. clippy警告数

0。

## 38. ユーザーSave保護

`saves/savegame_v1.ron`は読み書き・復元・ステージのいずれも行っていない。

## 39. PNG差分処理

テスト実行で再生成された既知の9件のPNGを、`git checkout -- <path>`で個別に復元した。

## 40. 最終`git status --short`

コミット後は空(最終報告に記載する)。
