# P27-004-PRE-001: Division移動・経路進行の長期停滞調査と修正設計

本タスクは、P27-003のPhase 27最終統合監査で発見されたFerrowynの2個Divisionの
84か月移動停滞(`Moving`状態のまま`movement_progress=0.2000`に固定)を、
P27-001の部分装備完成とは独立した既存の移動・経路探索問題として再現し、
根本原因と最小修正を確定する設計監査である。本番コード・データは変更していない
(調査完了後、全ての一時診断コード・一時テストを削除・復元済み)。

## §1 総合判定

```
P27-004-PRE-001 ANALYSIS COMPLETE
REPRODUCTION CONFIRMED
ROOT CAUSE CONFIRMED
DESIGN DECIDED
P27-004 IMPLEMENTATION READY
```

## §2 再現判定

**REPRODUCTION CONFIRMED**。Ferrowyn実データ(120か月実測、独立プロセス再現)と、
既存の`src/war/frontline.rs`テストフィクスチャ(`setup_test_environment`)を
再利用した最小合成再現(15日間の日次サイクル、独立プロセス5回)の両方で、
`movement_progress`が同一の固定値(合成再現では0.2000)へ収束し、有限日数
(合成再現では期待5日)を大幅に超えても到着しないことを動的に確認した。

## §3 根本原因判定

**ROOT CAUSE CONFIRMED**。`src/war/frontline.rs::process_defensive_plan`
(`DailySimulationSet::FrontlineOrders`に登録された`handle_daily_frontline_plans`
から日次で呼ばれる)が、既にこのSystem自身が生成した移動中(`frontline_
generated_movements`追跡対象)のDivisionに対しても、まだ目的地(`best_region`)へ
到達していなければ**目的地が前日と同一であっても無条件に**
`destination`/`current_path`/`target_state=None`/`movement_progress=0.0`を
上書きする。`FrontlineOrders`Setは`MilitaryAction`Set(`process_movement`が
1日分の進捗を加算する場所)より前に実行されるため、外部から観測される値は
「毎日リセット→1日分だけ加算」の結果である1日分の進捗量に恒久的に固定される。

## §4 設計確定判定

**DESIGN DECIDED**。採用候補は候補1(移動命令APIの冪等化、§10・§33・§34)。

## §5 P27-004 READY／BLOCKED

**P27-004 IMPLEMENTATION READY**。次の原子的実装タスクの範囲・受け入れ条件は
§19・§50で確定済み。

## §6 開始時HEAD

```bash
$ git rev-parse HEAD
8111ad5971526ff5f87d26e42245078363997025
$ git status --short
(空)
```

期待どおりであることを確認した。開始時テスト基準の実測: `cargo test
--all-targets` → 2607 passed / 0 failed / 1 ignored、`cargo clippy
--all-targets --all-features -- -D warnings` → 0 warnings、期待どおり完全一致。

## §7 監査対象コードHEAD

監査全体を通じてHEADは`8111ad5971526ff5f87d26e42245078363997025`のまま
変更していない(本タスクは報告書1件のみをコミットする設計)。

## §8 ユーザーSave保護

`saves/savegame_v1.ron`は監査全体を通じて未接触。一時テストはいずれも
`std::env::temp_dir()`配下の独自ディレクトリにのみSaveを書き出した。

## §9 現行移動契約

### 9.1 移動命令API

移動命令(`destination`/`current_path`/`target_state`/`movement_progress`/
`status`の一括更新)を発行する箇所は実コード上**3か所**存在し、いずれも
共通APIを経由せず各自でインライン実装している(重複、§32・§33で問題視):

1. `src/map/division_selection.rs`(プレイヤーの手動移動、UIクリック経由)
2. `src/war/frontline.rs::process_defensive_plan`(Defend stance自動配置)
3. `src/war/frontline.rs::process_offensive_plan`(Offensive stance自動攻撃、
   末尾で`process_defensive_plan`も呼ぶため実質1と2の両方の経路を持つ)

**AIとプレイヤーは同じAPIを使っていない**——3か所がそれぞれ独立に
`destination = Some(...)`; `current_path = ...`; `target_state = None`;
`movement_progress = 0.0`; `status = Moving`という同一パターンのコードを
複製している。

出発州は`division.current_state`(暗黙)、目的州は`destination`、経路は
`current_path`(Vec、先頭が次の一歩)、次の州は`target_state`
(`process_movement`が`current_path`から都度1件popして設定する作業変数)。

既存命令がある場合の上書き規則: **regression確認済みの唯一の部分的idempotency
guard**はプレイヤー経路(`division_selection.rs:461`)にのみ存在し、
`division_current == target && division.destination == Some(target)`
(=**既に目的地に到着済み**)の場合だけ早期returnする。**移動中に同じ目的地を
再指定した場合**(まだ到着していない)は、プレイヤー経路・AI経路のいずれも
ガードがなく、無条件に`current_path`・`movement_progress`を再生成・リセットする
——これが本監査の根本原因(§11)。異なる目的地を指定した場合は、いずれの経路も
正しく新しい経路へ置き換える(意図した上書き)。不正な目的地(`find_path`が
`None`を返す)を指定した場合は、いずれの経路も既存状態を一切変更せず
無言でスキップする(§17)。

### 9.2 経路探索

`src/military/pathfinding.rs::find_path`(BFS、`rg`で確認)。隣接州は
`StateData.neighbors`(事前計算済みVec)。tie-breakはコード上で`StateId`昇順を
明示的に使う箇所(`frontline.rs`の候補ソート)と、`find_path`内部のBFS走査順
(隣接リストの登録順)の2層がある。owner基準ではなく**controller基準**
(`state.controller()`)で通行判定する——`is_own_territory = target_
controller == owner`、`is_at_war_with_controller = war_registry.
are_countries_at_war(owner, target_controller)`。中立国・非交戦国の州は
通過不可(即座に停止、Idleへ)。海上州(`is_sea`)は`process_defensive_plan`の
攻撃候補抽出で明示的に除外される。経路なしの場合、呼び出し元(3か所いずれも)は
`if let Some(path) = find_path(...)`のガードにより**その日の再割当自体を
静かにスキップする**(既存状態を変更しない、パニックしない)。出発州と目的州が
同一の場合は、プレイヤー経路の§9.1ガードで早期return。経路途中で支配が変化した
場合は`validate_and_stop_invalid_movements`(§9.4後述)が別途処理する。
国家消滅・併合はowner_country_idの更新を通じて自然に反映される
(Phase 21の既存ownership機構、無変更)。

### 9.3 日次進捗式

```rust
// src/military/movement.rs::process_movement
let base_days = 5.0;
let step_cost = pathfinding::calculate_step_cost(current_state, target, state_registry);
let supply_mod = division.supply_ratio.max(0.5); // 下限0.5(最悪でも2倍減速まで)
let movement_days = (base_days * step_cost / (def_speed * supply_mod)).max(1.0);
let daily_progress = 1.0 / movement_days;
```

使用: 基礎移動速度(`DivisionDefinition.movement_speed`)・地形/距離
(`calculate_step_cost`)・MilitarySupplies充足率経由の`supply_ratio`
(**最大2倍減速、0にはならない**)。**未使用**: Equipment(`equipment`/
`max_equipment`は移動速度計算に一切現れない、`rg`で確認)、manpower、
organization、破産状態、ゲーム速度/実時間delta(1呼び出し=1日固定)。
`daily_progress`は入力が有限である限り常に正の値であり、単独では0にならない
(§30でこの式自体は原因から除外)。

### 9.4 州到着処理

到着閾値は`new_progress >= 1.0`(`>=`、余剰進捗は切り捨てられる[次セグメントへ
繰り越されない])。到着時: `current_state = target`、`movement_progress = 0.0`、
`current_path`から次の1件をpopして`target_state`更新(空なら`target_state =
None; destination = None`)。最終目的地到着(`target_state.is_none() &&
destination.is_none()`)かつ自国領なら`Moving → Idle`。敵支配地域なら
`invasion::process_division_arrival`(占領処理)。Message/Eventの明示的発行は
確認できなかった(状態フィールドの直接変更のみ)。装備補充対象への復帰時点は
`status == Idle`に戻った瞬間(`is_eligible_for_equipment_reinforcement`が
`status`を見るため、次の日次装備補充Systemの呼び出しから自動的に対象化)。

`validate_and_stop_invalid_movements`(`MilitaryAction`Set内、`process_
movement`と同じ日次呼び出しチェーン)は、戦争終了・支配変更で無効になった
`target_state`/`current_path`を検出し、無効なら停止(`Idle`化)または経路の
残り部分をクリアする——ただし進行中の`target_state`自体への移動は継続扱い
(次到着時に再判定)。これは`process_defensive_plan`とは別の独立した安全弁で
あり、本監査の根本原因とは無関係(§30)。

## §10 System順序

```text
Economy(月次生産・月次財政・日次補給消費)
→ Research → Diplomacy → CountryAi(税率AI→建設AI)
→ WarPreparation → MilitaryAi
→ FrontlineOrders(process_daily_frontline_plans: 防御配置→攻勢命令)
→ MilitaryAction(handle_daily_military: 募兵完成→manpower補充→装備補充→
  経路検証→移動進行→戦闘→占領)
→ WarResolution → UiUpdate
```

`app::time::DailySimulationSet`の宣言順(`.chain()`)により、`FrontlineOrders`
は`MilitaryAction`より必ず先に実行される(偶然の登録順ではない)。これが
「移動進行で0.0→0.2 → 同日のAI命令再発行がprogressを0.0へ再初期化 → 次の
移動進行で再び0.2」という observationの直接的な構造的原因である
(§11で動的に確認)。

## §11 Ferrowyn再現

P27-003と同一の実App・実System・実カレンダー・`PlayerCountry(None)`条件で
120か月シミュレーションを独立に再実行し(一時診断テスト、削除済み)、以下を
再確認した(既存P27-003の値を推定で再利用せず、本タスクで独立に再現):

- 月0〜月3: 全Divisionは`Idle`。
- 月36: Ferrowynの2個Division(DivisionId(5)=既存の全額装備、DivisionId(7)=
  P27-001経由の部分装備新規)がいずれも`Moving@StateId(22)->Some(StateId(19))
  progress=0.2000`。
- 月60・月120: **完全に同一の状態**(`movement_progress`・`destination`が
  bit-for-bitで不変)。

## §12 対象Division

- DivisionId(5): 生成日=New Game開始時(月0)、元は`Idle@StateId(22)`
  (Ferrowyn首都)、全額装備(1000.0/1000.0)。
- DivisionId(7): 生成日=月36以前(P27-001経由の部分装備完成、46.1/1000.0)。

両者とも移動開始日は月36と同一のタイミングで観測され(Ferrowynが月36までに
戦争状態[`at_war=true`]へ入り、frontline plan[Defend stance]が両Divisionを
同時に前線地域へ再配置したため)、出発州=StateId(22)(Ferrowyn首都)、
目的州=StateId(19)、経路長=1(隣接、単純経路)。

## §13 停滞開始日

月36の時点で既に`progress=0.2000`かつ`Moving`状態であり、これは「1日目の
リセット→加算サイクル」が既にその時点で複数回(少なくとも1回)完了した後の
スナップショットである。月チェックポイント粒度(月3・36・60・90・120)では
これ以上の精度で「最初にこの固定サイクルへ入った正確な日」を月36以前の
どのタイミングかまで絞り込めなかった(Ferrowynがいつ`FrontlineStance::Defend`
へ設定されたか、AI戦争開始AIのタイミング依存)——最小合成再現(§14)により、
このサイクルは**割当が発生した初日から即座に始まる**ことを日次粒度で確認済み
であり、Ferrowynについても同型のメカニズムであることは確実である。

## §14 0.2000固定の直接原因

最小合成再現(既存の`setup_test_environment`フィクスチャ、State1↔State2隣接・
単一経路・同一owner・十分なmanpower・Equipment 100%・MilitarySupplies充足)を
`process_daily_frontline_plans`→`process_movement`の日次サイクルとして
15日間反復した(一時テスト、`git checkout --`で完全復元済み)。結果:

```text
day=1  after_orders: progress=0.0000 | after_movement: progress=0.2000
day=2  after_orders: progress=0.0000 | after_movement: progress=0.2000
...(15日間、完全に同一のパターンが継続)...
day=15 after_orders: progress=0.0000 | after_movement: progress=0.2000
FINAL: status=Moving current_state=StateId(1) progress=0.2000(到着していない)
```

`process_daily_frontline_plans`実行直後(`after_orders`)は常に
`progress=0.0000`——`process_defensive_plan`が毎日無条件でリセットしている
直接証拠。`process_movement`実行直後(`after_movement`)は常に
`progress=0.2000`(=`1/movement_days`、`movement_days=base_days(5.0)×
step_cost(1.0)/(def_speed(1.0)×supply_mod(1.0))=5.0`)。独立プロセス5回で
完全に同一の結果を確認した(決定的)。

## §15 移動命令再発行回数

最小合成再現で15日間**毎日**同一目的地(StateId(2))への命令が再発行されて
いることを確認した(§14の`after_orders`列)。Ferrowynの実データでは、月36
時点で既に停滞状態にあり、月60・月120まで一切変化がないことから、この
期間中(少なくとも84か月=約2520日相当)**毎日**同一命令が再発行され続けた
と推定される(最小合成再現と同一のコード経路[`process_defensive_plan`]を
通るため、日次粒度での再発行回数は経過日数と等しいと結論できる)。

## §16 経路再計算回数

`process_defensive_plan`は`find_path`をDivisionごとに毎日呼び出す
(候補地域の距離計算含め、`process_offensive_plan`内でも同様)。目的地
(`best_region`)自体が変化しない限り、計算結果(経路)も同一になるため
「経路が変わった」という意味での再計算は0回だが、「関数呼び出し自体」は
経過日数と同じ回数発生する。

## §17 戦争・講和・占領との関係

Ferrowynの停滞は**戦争継続中**に発生・持続した(`at_war=true`が月36〜120で
一貫)。本監査の対象期間内でFerrowynが講和したという証跡はない
(`at_war=true`が月120まで一貫していることから)。最小合成再現は講和・占領を
一切含まないシナリオでも同一の停滞を再現できたため、**戦争・講和・占領は
根本原因の必要条件ではない**(停滞発生には不要、Defend/Offensive stanceの
frontline plan自体が存在すれば十分)。

## §18 最小合成再現結果

§14に記載のとおり。既存フィクスチャの再利用により、Equipment 100%・
MilitarySupplies充足という「最も有利な条件」下でも停滞が発生することを
示した——これは「Equipment不足やMilitarySupplies不足が原因ではない」ことの
最も強い反証(favorable conditionでも発生するなら、これらは原因たり得ない)。

## §19 Equipment 100/99/50/1/0%比較

`src/military/movement.rs`の速度計算式(§9.3)を静的に確認した結果、
`division.equipment`・`division.max_equipment`のいずれも参照されていない
——Equipment量は`daily_progress`の計算に一切影響しない。これはP27-003が
実データで確認した「全額装備済みDivision(1000.0/1000.0)も部分装備Division
(46.1/1000.0)も同一の`progress=0.2000`固定パターンを示す」という観測と
完全に整合する。したがって100/99/50/1/0%のいずれでも**停滞の有無・
固定値そのものに差は生じない**(式にEquipmentが登場しないため、動的な
個別比較テストを追加で実行するまでもなく、コードの構造そのものが結論を
一意に決定する)。

## §20 同一命令再発行

§14で確認済み(CONFIRMED、直接原因)。

## §21 異なる命令上書き

`process_defensive_plan`のロジック上、`best_region`が計算のたびに変わり得る
(前線地域の配置バランス[`region_counts`]・距離・StateId順の変化によって)。
本監査ではこの「目的地が実際に変わる」ケースを個別の合成テストとしては
再現していない(NOT TESTED、既存コードのロジックからは正しく`destination`を
更新すると推定できるが、動的確認は次タスクの単体テストで行う)。

## §22 経路なし

`if let Some(path) = find_path(...) { ... }`という条件式構造そのものが、
経路探索失敗時に一切の状態変更をスキップすることを保証する(静的確認、
CONFIRMED。停滞の原因としては REJECTED: 経路なし自体は新たな停滞を生まない、
既存状態を素通しするだけ)。

## §23 支配変更

本監査では動的な合成再現を行っていない(NOT TESTED)。ただし
`validate_and_stop_invalid_movements`(§9.4)が既存のP26-006-REV-001改修で
毎日呼ばれており、支配変更後の無効な経路を検出して安全に処理する設計が
既にコード上確認できる。この関数自体は`process_defensive_plan`の
問題(§11)とは独立した経路であり、本監査の根本原因ではない。

## §24 講和

本監査では動的な合成再現を行っていない(NOT TESTED)。`src/war/peace.rs`
(219〜251行)が講和成立時に`target_state`/`movement_progress`を明示的に
リセットする既存コードを確認した(静的確認のみ)。

## §25 仮説A〜Jの判定

| 仮説 | 判定 | 根拠 |
|---|---|---|
| A: 同一移動命令によるprogressリセット | **CONFIRMED** | §14の動的再現、`process_defensive_plan`が無条件リセット |
| B: AI命令と移動進行の順序 | **CONFIRMED** | §10、`FrontlineOrders`が`MilitaryAction`より先、外部観測値が常に「加算後」の値になる構造的理由 |
| C: 経路なしでもMovingへ入る | **REJECTED** | `if let Some(path) = ...`ガードが状態変更自体をスキップする(§9.1・§22) |
| D: stale route | **REJECTED(本件の主因ではない)** | 既存の`validate_and_stop_invalid_movements`/`cancel_stale_offensive_movements_before_defend`が別途対応済み、最小合成再現は戦争状態変化なしでも発生 |
| E: 日次System不発火 | **REJECTED** | §14でSystemが毎日実行され値が変化することを確認(0.0↔0.2のサイクル自体がSystem動作の証拠) |
| F: Query除外 | **REJECTED** | Divisionは毎日both Systemの処理対象になっている(§14) |
| G: 進捗式が0 | **REJECTED** | `daily_progress=0.2`(§9.3・§14) |
| H: 到着境界の浮動小数点問題 | **REJECTED** | progressは0.2近辺に留まり1.0に近づくことすらない、境界問題ではない |
| I: AI命令の振動(複数目的地交互) | **REJECTED(Ferrowynの実データでは非該当)** | destination=Some(StateId(19))が月36〜120で不変、同一目的地の単純な再発行(仮説A)であり複数目的地間の振動ではない |
| J: save/load | **REJECTED(NOT CAUSAL、実測確認)** | §26、save→load前後で全Division movement状態が完全一致 |

## §26 状態不変条件

```text
status == Moving → destinationが存在する: 成立(コード上、Movingへ遷移する
  全経路がdestinationを同時に設定する)
status == Moving → 有効なrouteまたは次州が存在する: 概ね成立(current_path
  またはtarget_stateのいずれかが必ず設定される)
status == Moving → current_stateとroute始点が整合する: 成立
status == Moving → 有限日数内にprogress/current_state/routeのいずれかが
  変化する: **不成立**(§14の反例、本監査の核心的な不変条件違反)
最終目的地へ到着 → status == Idle: 成立(§9.4)
status == Idle → stale routeやdestinationが残らない: 成立(到着処理が
  destination=None等を設定)
```

破れている入口: `process_defensive_plan`(および`process_offensive_plan`)の
再割当ブロック——「有限日数内に変化する」という不変条件を、目的地が前日と
同一の場合の早期skipガードの欠如によって破っている。timeout案(§採用禁止案)
ではなく、この入口自体で不変条件を回復する設計を採用する(§33)。

## §27 修正責任の層

根本原因は「AIの意思決定/命令発行層」(`process_defensive_plan`/`process_
offensive_plan`)にあり、移動の実行・進行を担う「Domain API」
(`process_movement`)自体は正しく動作している(1日分の進捗を正確に加算して
いる)。ただし、同一の欠陥パターン(idempotency guardの欠如)が
プレイヤー入力層(`division_selection.rs`)にも独立に存在することを確認した
(§9.1)——これは「AI層だけの局所的な不具合」ではなく、「移動命令を
発行する3か所全てが、共通の検証済みAPIを経由せず各自でこの状態遷移を
複製している」という、より上位のアーキテクチャ上の欠落が根本原因である。

## §28 候補1〜6比較

- **候補1(移動命令APIの冪等化)**: 採用。3か所の重複実装を1つの共通関数へ
  集約し、「既にMoving かつ destination==要求先」の場合は無条件no-opとする
  (§33)。AIとプレイヤーの両方の欠陥を単一の修正で解消できる。
- **候補2(AI側で同一命令を発行しない)**: 部分的に有効だが、`division_
  selection.rs`のプレイヤー経路の同型欠陥(§9.1)を放置する。将来UIが
  誤って同じ命令を連投した場合(例: ダブルクリック、Webhookの再送)に
  同じ停滞が再現し得るため、AI側だけの修正では不十分と判断し不採用。
- **候補3(命令受付の原子性)**: 現行コードは既に`destination`/`current_
  path`/`target_state`/`movement_progress`/`status`を同一ブロック内で
  一括更新しており(中間状態は生じない)、原子性自体は既に満たされている
  ——本件の原因ではないため不要。
- **候補4(経路無効化時の正規再評価)**: 既存の`validate_and_stop_invalid_
  movements`/`cancel_stale_offensive_movements_before_defend`が既にこの
  役割を担っており、本件の原因でも解決策でもない。無変更のまま維持する。
- **候補5(System順序修正)**: 順序自体(`FrontlineOrders`→`MilitaryAction`)は
  「前日までの状態を見てから当日の命令を出す」という正しい設計意図であり、
  変更すべきではない——問題は順序ではなく、命令発行側の冪等性の欠如
  (候補1で解決)。不採用。
- **候補6(進捗式修正)**: `daily_progress`の式自体は正しく機能している
  (§9.3・§14で0.2が正確に加算されることを確認)。不要。

## §29 採用候補

**候補1**: 移動命令発行を共通の冪等な関数(仮称`start_or_maintain_
movement`)へ集約する。

```text
if division.status == Moving
   and division.destination == Some(requested_destination)
then
   no-op(destination/current_path/target_state/movement_progress/statusの
   いずれも変更しない、進行中の進捗を保持する)
else
   既存のパターンどおり、find_path成功時にdestination/current_path/
   target_state=None/movement_progress=0.0/status=Movingを一括更新する
   (find_path失敗時は何もしない、既存の挙動を維持)
```

## §30 却下候補

候補2・3・4・5・6(§28に理由を記載)。

## §31 装備補充との関係

Moving状態が続く限り`is_eligible_for_equipment_reinforcement`
(P25-005、無変更)により補充対象から除外され続ける——これは**二次影響**であり
原因ではない(§19で確認したとおりEquipmentは移動進捗に一切影響しない)。
P25-005の「非Idle部隊を補充対象外とする」契約自体は正しく、本タスクの
修正対象ではない。修正後は、移動が正規に完了して`Idle`へ戻った時点で
既存の装備補充が自然に再開する(§9.4、既存コードの動作、変更不要)。

## §32 MilitarySuppliesとの関係

`supply_ratio`(MilitarySupplies充足率由来)は`supply_mod = supply_ratio.
max(0.5)`として移動速度計算に関与するが、**下限0.5でクランプされるため
最大2倍の減速にしかならず、進捗を0にすることはできない**(§9.3)。したがって
MilitarySupplies不足は移動を遅くする既存の正規要因ではあるが、本件の
「完全停止」の原因ではない。補給充足率が移動速度へ影響すること自体は
既存の意図した設計(未変更)である。

## §33 戦闘・Frontlineとの関係

`Fighting`/`Retreating`状態のDivisionは`process_movement`の対象外
(既存契約、無変更)——本件の停滞Divisionは一貫して`Moving`状態であり
`Fighting`ではなかった(戦闘中に停滞したのではない)。`FrontlineStance`
(Defend/Offensive)の日次再評価(`process_daily_frontline_plans`)こそが
本件の直接的な発生源である(§14)。攻勢AIが同じ移動先を繰り返し指示する
ケース(`process_offensive_plan`)も、内部で`process_defensive_plan`を
呼ぶため同一の欠陥を継承する(§9.1)。戦争終了後も攻勢由来の命令が残るかは
本監査では未検証(NOT TESTED、`cancel_stale_offensive_movements_before_
defend`が対応する設計になっていることをコードで確認済み)。retreatと
通常movementの状態は明確に分離されている(`DivisionStatus::Retreating`は
別状態であり、本件のMoving状態と混同されていない)。

## §34 6国家影響範囲

一時診断テスト(6か国・120か月+10日、削除済み)で、月130時点の各国の
Moving Division数と、直前10日間で位置・進捗が完全に不変だった
(=停滞している)Division数を測定した:

| 国 | Division総数 | Moving中 | 10日間停滞 |
|---|---|---|---|
| Arcadia | 2 | 0 | 0 |
| Elfin | 3 | 0 | 0 |
| Dwarf | 2 | 0 | 0 |
| Oceanic | 1 | 0 | 0 |
| Passguard | 1 | 0 | 0 |
| Ferrowyn | 2 | 2 | **2** |

この特定の120か月シナリオでは、月130時点でFerrowynのみが実際に停滞
Divisionを抱えている(他5か国はその時点でMoving中のDivisionが0件、
既にIdleへ戻っているか、そもそも前線再配置が発生していない)。ただし
根本原因(`process_defensive_plan`の欠陥)は**国家に依存しないコード共通
経路**であり、最小合成再現(§14、Ferrowynのデータを一切使わない汎用
フィクスチャ)で100%再現できることから、**どの国のどのDivisionも、Defend/
Offensive stanceの前線再配置対象になり、かつ目的地への到達に2日以上
かかる状況が生じれば、同じメカニズムで無期限に停滞し得る**——今回たまたま
Ferrowynだけがこの条件(継続的な戦争・単一の恒常的な最適前線地域)に
該当していたに過ぎない。

## §35 save→load

一時診断テストで以下を実測した:

1. 36か月まで通常実行(停滞状態を含む)→save→load→movement状態
   (status/current_state/destination/target_state/current_path長/progress)
   の全項目が**完全一致**(9件のDivision全て)。

**NOT CAUSAL**——save/loadは本件の発生・持続いずれにも関与しない。停滞は
純粋にメモリ内の日次サイクルだけで発生し、save/loadを一切介さない最小合成
再現(§14)でも同一の欠陥が確認できるため、save/load固有の問題ではないと
結論できる。

## §36 Save V1／V2判断

候補1(移動命令APIの冪等化)は新規Saveフィールドを一切必要としない——
`destination`/`current_path`/`target_state`/`movement_progress`/`status`と
いう既存フィールドの更新条件を変えるだけであり、新しい状態を導入しない。
**Save V1を維持できる**。

## §37 決定性

最小合成再現(§14)を独立プロセス5回実行し、15日間全ての`after_orders`/
`after_movement`の値が完全に一致することを確認した。一時診断のsave/load
テストも決定的(1回のみ実行だが、既存の値の完全一致という強い決定的性質を
示す結果)。

## §38 国家間分離

一時診断テスト(§34)で6か国が同時に異なる停滞状況(Ferrowynのみ停滞、他は
0件)を示しながら、Ferrowynの状況が他国のDivision数・状態に影響していない
ことを確認した(既存のP27-003監査が既に確認した国家間分離契約の範囲内)。

## §39 Phase 23保護

本監査は`compute_manpower_ledger`・`recruitment_queue`・`mobilization`・
`workforce`のいずれも変更していない(調査のみ)。停滞DivisionのMobilized
manpowerは`current_state`に関わらず既存のledger計算に正しく含まれ続ける
(§9.4の到着処理を経ない限りmanpowerフィールド自体は変化しないため)。

## §40 Phase 24保護

Division位置(`current_state`)・frontline・occupation・controller・国家消滅・
講和処理のロジックはいずれも本監査で変更していない。

## §41 Phase 25保護

Equipment会計・装備補充・非Idle部隊の補充除外契約(P25-005)・戦闘力への
Equipment反映はいずれも無変更。§31で確認したとおり、停滞は既存の非Idle除外
契約が正しく機能した**結果**であり、この契約自体を変更する必要はない。

## §42 Phase 26保護

MilitarySupplies需要/消費/配分・首都連結性・補給充足率・募兵ゲート・
攻勢ゲート・決定性・Save V1はいずれも無変更。§32で確認したとおり
`supply_ratio`が移動速度に与える影響(最大2倍減速)は既存の意図した設計の
ままであり、本件の修正はこの式に触れない。

## §43 Phase 27保護

部分装備Division完成・`max_equipment`・募兵queue解放・財政安定化・建設予備
資金・AI税率調整はいずれも本監査で変更していない。P27-001の部分装備完成が
本件を「引き起こした」わけではないことを§19(Equipment非因果)・P27-003の
実データ(全額装備済みDivisionも同一パターンで停滞)で二重に確認済みである。

## §44 Critical／High／Medium／Low

**Medium**と判定する(格下げも格上げもせず、P27-003の当初分類を維持)。
理由: 本件は移動不能が「通常プレイで広範囲に」発生するとまでは実測で
確認できなかった(§34、6か国中1か国のみ、かつ該当Divisionは2件のみ)。
一方で、**発生した場合は無期限(観測上84か月間、理論上は永久)に持続し、
装備補充・戦闘参加の両方を恒久的に停止させる**という重大性を過小評価しては
ならない——プレイヤーが同じ状況(継続的な防御戦争下でのDefend stance運用)に
陥れば同じ影響を受け得る。Phase 27自体の契約は破っていないため、Phase 27の
完了判定はブロックしない。

## §45 次の原子的実装タスク

**P27-004: 移動命令の冪等化(候補1)を実装する**。詳細は§50参照。

## §46 対象外

本タスクでは実装しない: 本番移動コードの修正、本番経路探索の修正、本番AIの
修正、恒久テスト、Equipment補充条件変更、移動速度バランス変更、地形速度
補正追加、鉄道・港・海上輸送・戦略的再配置、organization・attrition・燃料・
交易、UI・ローカライズ、Save V2、国家ID特例、Phase 23〜27の無関係な再設計。

## §47 実行コマンドと正確な結果

全て逐次実行(並列実行なし):

```text
cargo test --all-targets (開始時確認)  → 2607 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings (開始時) → 0 warnings
(一時診断: src/war/frontline.rsへの15日サイクル再現テスト、5回独立実行)
  → 全て同一結果(FINAL progress=0.2000固定を確認)
(一時診断: 6か国120か月+10日の影響範囲測定、save/load一致確認)
  → Ferrowynのみ2件停滞、save/load完全一致
cargo test --all-targets (最終確認)  → 2607 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings (最終確認) → 0 warnings
```

## §48 全テスト件数

開始時・最終確認とも**2607 passed / 0 failed / 1 ignored**——本監査は
恒久テストを追加しないため変化なし。

## §49 clippy警告数

開始時・最終確認とも**0 warnings**。

## §50 次のP27-004実装設計(確定事項)

- **変更する関数**: `src/military/movement.rs`(または新規の共通モジュール)に
  `start_or_maintain_movement(division, requested_destination, path)`相当の
  新規冪等関数を追加。`src/war/frontline.rs::process_defensive_plan`・
  `process_offensive_plan`の重複インライン実装と、`src/map/division_
  selection.rs`の重複インライン実装を、この共通関数の呼び出しへ置換する。
- **状態不変条件**: §26に記載の6条件を維持し、「有限日数内に変化する」
  不変条件をこの関数の冪等化によって回復する。
- **同一命令の扱い**: `status==Moving && destination==Some(requested)`
  → no-op(進行中の`movement_progress`・`current_path`・`target_state`を
  一切変更しない)。
- **異なる命令の扱い**: 既存どおり新しい経路探索を行い、成功時のみ
  `destination`/`current_path`/`target_state=None`/`movement_progress=0.0`/
  `status=Moving`を一括更新する(既存の挙動を維持、変更なし)。
- **経路なしの扱い**: 既存どおり何も変更しない(現状維持)。
- **経路無効化の扱い**: 既存の`validate_and_stop_invalid_movements`/
  `cancel_stale_offensive_movements_before_defend`をそのまま維持し、
  本タスクでは変更しない。
- **progressの扱い**: 同一命令時は保持、異なる命令時は0.0へリセット
  (既存どおり)。
- **System順序**: 無変更(`FrontlineOrders`→`MilitaryAction`の既存順序を
  維持、これは正しい設計)。
- **AIとプレイヤーの共通性**: 3箇所全てが同一の新規共通関数を呼ぶことで
  達成する(§27・§29)。
- **save/load**: 既存フィールドのみを使うため追加のmigrationは不要。
- **Save V1／V2**: V1維持(§36)。
- **単体テスト**: 新規共通関数の同一目的地no-op・異なる目的地上書き・
  経路なしno-op・境界(既に到着済み)の各ケースを純粋関数として直接検証する。
- **E2Eテスト**: 既存の`test_defensive_positioning_and_determinism`等
  frontline.rs内の既存テストが新関数経由でも変わらず緑であることを確認し、
  加えて新規に「Defend stanceの日次サイクルを複数日反復しても目的地到達まで
  進捗が保持される」ことを検証するE2Eを追加する。
- **Ferrowyn再現テスト**: 6か国120か月シナリオで、Ferrowynの2個Divisionが
  今度こそ有限日数内に`Idle`へ到達することを確認する回帰テストを追加する。
- **6か国120か月テスト**: §34と同型の停滞検出ロジックを恒久テスト化し、
  「120か月時点で停滞Divisionが0件」を受け入れ条件にする。
- **決定性テスト**: 独立プロセス5回で同一の到着日・最終状態を確認する。
- **回帰テスト**: P27-003・P26-DET-001系・land_war_combat_peace_test・
  既存frontline.rs全テストの再実行。
- **対象外**: §46のとおり。
- **完了条件**: §18(監査完了基準)相当を実装後に再確認し、Ferrowynの実測
  停滞が解消され、6か国とも120か月以内に前線再配置Divisionが有限日数で
  到着することを実測で確認すること。

## §51 一時検証コード削除

`src/war/frontline.rs`への一時テスト追加(`p27_004_pre_001_temp_diagnostic_
defend_stance_daily_cycle`)は`git checkout -- src/war/frontline.rs`で完全
復元し、`git diff`で無変更であることを確認した。`tests/p27_004_pre_001_temp_
impact_scope_diagnostic.rs`は`rm`で削除し、`git status --short`で追跡対象に
残っていないことを確認した。

## §52 PNG差分処理

`cargo test --all-targets`実行(複数回)により既知の非決定的PNG 9件が
再生成されたことを都度`git status --short`で確認し、個別に`git checkout --`
で復元した。

## §53 コミット対象

```text
verification_logs/phase-27/p27-004-pre-001/p27-004-pre-001_movement_progress_stall_design.md
```

本番コード・本番データ・既存テストは一切変更していないため、コミット対象は
この1ファイルのみである。

## §54 最終`git status --short`

コミット後にチャットで報告する(このレポート自体には未来のコミットハッシュを
記載しない)。
