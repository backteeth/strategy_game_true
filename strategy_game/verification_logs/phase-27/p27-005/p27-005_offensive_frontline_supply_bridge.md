# P27-005 攻撃側前線補給ブリッジ 実装報告

## 1. 結論

**P27-005 COMPLETE WITH MANUAL VERIFICATION PENDING**

P27-005-PRE-001(1895cfa)の候補1「首都連結済みの自国州に隣接する敵支配州で、進行中の戦闘の攻撃側である師団へ、1ホップだけ前線補給を届ける」を実装した。国家備蓄が潤沢でも、攻撃側が敵支配州で戦うと`fulfillment=0`→戦闘力0になる問題(High)は解消した。`distribute_damage`の実効戦力0側が損害を受けない別件は本タスクの対象外で、Medium 1件として残る。

## 2. 変更ファイル

| ファイル | 内容 |
|---|---|
| src/military/daily_supply.rs | 資格判定関数、`plan_country_daily_supply_with_frontline`、`run_daily_military_supply_consumption_with_battles`、Bevyシステムへ`Res<BattleRegistry>`追加、単体テスト11件 |
| tests/p27_005_offensive_frontline_supply_bridge_e2e_test.rs(新規) | 実Appによる10件のE2E |
| tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs | 90日テストの最終アサーションを実測の新正規値へ更新 |
| 本レポート | |

`tests/land_war_combat_peace_test.rs`と`saves/savegame_v1.ron`は未変更・未ステージ。

整形について: `rustfmt`を`daily_supply.rs`単体へ実行した結果、未変更の領域にも整形差分(既存コードの折り返し変更)が出ている。挙動への影響はない。workspace全体の整形は行っていない。

## 3. 資格契約(実装どおり)

`is_frontline_supply_eligible(division, connected, state_registry, battle_registry)`は次の全てを満たすときだけtrue。

1. 師団の現在州が通常の首都連結集合に含まれない
2. 現在州のcontrollerが師団のownerと異なる
3. `BattleStatus::Ongoing`の戦闘で、`attacker_country == owner`かつ`state_id == current_state`かつ`attacker_division_ids`に師団自身が含まれる
4. 現在州の隣接州のうち、controllerがownerで、かつ通常の首都連結集合に含まれるものが1つ以上ある

攻撃側の判定にMoving・stance・戦争・目的地は使わない。敵州は連結集合へ挿入しない(連鎖投影なし)。

## 4. 不変条件

- 需要は師団ごとに1回だけ数える(`eligible`へ1回のみpush)。有効な隣接州が複数あっても二重配分しない。
- 既存のHamilton(最大剰余)配分、固定小数点会計、国家備蓄を再利用。不足時は通常師団と前線師団が既存規則で按分する。
- 防御側の契約は無変更(首都連結外なら従来どおり補給0)。
- 戦闘準備度式`min(manpower, equipment, supply)`は無変更。
- controller変更条件は無変更。無防備な敵州は従来どおり到着時に即`occupy_state`。
- システム順序は無変更。到着(d10)は前日補給で戦い、ブリッジは翌日(d11)に効く順序が保たれる。
- 新規永続フィールドなし。Save V1のまま、マイグレーションなし。
- プレイヤーとAIで同一規則。

## 5. 単体テスト(11件、`military::daily_supply` 計44件全通過)

- 実際の攻撃側が資格を得る
- すでに通常接続なら不使用
- 実際の`Ongoing`戦闘が必要
- 防御側と誤った攻撃側記録を除外
- 隣接する自国州が必要
- 隣接州が切断・敵支配・中立なら失う
- 首都陥落で失う
- 参照欠落(州・戦闘)に耐える
- 近隣順序に依存しない
- 複数の有効な隣接州でも需要は1回
- 不足時は既存Hamilton規則で通常師団と前線師団が按分

## 6. E2Eテスト(新規10件、全通過)

実測の正規値は、PRE-001の一時パッチで測定した値と完全に一致した。

| # | 検証内容 | 結果 |
|---|---|---|
| 01 | 攻撃4対防御1のベースライン | 初戦d10、d10 fulfillment=1.0、d11>0、補給0の戦闘日0、州3のcontrollerはd11にArcadiaへ |
| 02 | 戦争なしなら第三国は不変、国家備蓄は有限 | Passguardの備蓄が戦争有無で同一 |
| 03 | 回廊遮断(d20〜d45) | 補給0の戦闘日がちょうど25日、その後回復 |
| 04 | 首都陥落 | 補給0の日が40日以上 |
| 05 | 攻撃4対防御3 | ブリッジ自体は阻害されない(補給0の戦闘日0) |
| 06 | 無防備な敵州 | 戦闘なしでd10に即占領 |
| 07 | 戦闘中のセーブ/ロード(d5、d10、d11、d15) | 連続実行と一致 |
| 08 | 回廊遮断+セーブ/ロード | 連続実行と一致 |
| 09 | 独立2回実行の決定性 | 完全一致 |
| 10 | 不足時の按分 | 全Arcadia師団の充足率が0超1未満、備蓄は正確に0.0で保存則成立 |

## 7. P26-008-B 90日テストの更新

弱めていない。旧アサーションは「Defend/blocked=true」(P27-004で導入した、0補給包囲による恒久デッドロック前提)だった。新しい実測値:

- 攻撃師団はStateId(3)に到達し`Fighting`
- 攻撃側の`supply_fulfillment`は1.00(>0を明示検証)
- 防御側は0.00のまま(防御契約は無変更であることを検証)
- `offensive_supply_blocked == Some(false)`
- `stance == Defend`

stanceがDefendである理由を確認した: `evaluate_military_ai_decision`(src/war/military_ai.rs)は、割当師団がすべて`Fighting`で待機・移動中が0のとき`has_ready_divisions==false`となり、補給判定より前の既存分岐でDefend/Recoveringを返す。補給が原因ではない。到達、補給、stance、blockedを別々に検証している。

## 8. 検証結果

- `cargo test --all-targets`: **2644 passed / 0 failed / 1 ignored**(基準2623+新規21)
- `cargo clippy --all-targets --all-features -- -D warnings`: 警告0
- 独立プロセス5回連続(各回とも全通過): 新規E2E(10)、P26-008-B(19)、P26-006(18)、P26-004(19)、P26-005(10)、P27-004(6)、P26-006-REV-001の再開決定性(9)、P27-002-B(18)
- 120か月・長期(P27-002-C、release、16件): 独立プロセス5回連続で全通過
- 9枚の既知の非決定的PNGはテスト後に個別に`git checkout -- <path>`で復元済み

## 9. 長期検証(36か月・120か月)と自然発火の有無

P27-002-Cの90日/36/60/120か月の実6か国テストは、変更後も通過する。ただしPRE-001の監査で自然プレイでは戦闘が0件だったため、**この長期テストではブリッジ経路は発火しない**(通常のAI運用では戦闘が起きないため、無変更で通過するのは当然の結果)。ブリッジが実際に効くことの証拠は、上記の制御されたE2E(§6)であり、自然発火とは区別する。自然発火の頻度は今回測定していない。

## 10. 残課題と分類

| 区分 | 件数 | 内容 |
|---|---|---|
| Critical | 0 | |
| High | 0 | 攻撃側の補給0デッドロック(ブリッジで解決) |
| Medium | 1 | `distribute_damage`: `total_effective<=0`の側が損害を受けないため、多師団戦闘が膠着しうる。P26-008-Bの90日シナリオで、防御側`ful=0.00`のまま州3の1対1戦闘が未決着になる事象として実際に観測 |
| Low | 1 | `daily_supply.rs`の整形差分(§2) |

**P27-006-PRE-001**として、`distribute_damage`の実効戦力0側の扱いを独立に監査することを推奨する。本タスクでは修正していない。

## 11. 手動確認待ち

実プレイでの、AI・プレイヤーの攻勢が敵州で補給を受けて決着する挙動の目視確認は、未実施(自動テストのみ)。

## 12. コミット

メッセージ: `fix: bridge supply to attacking frontline divisions`(実装+テスト+本レポートの単一コミット)。コミットハッシュは、コミット後の最終報告に記載する。
