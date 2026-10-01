# P28-008-PRE-001：複数戦争下の講和と占領状態正常化の設計監査

## 0. 結論（先出し）

```
P28-008-PRE-001 ANALYSIS COMPLETE
HARM REPRODUCED (関数レベル・実App制御シナリオ・自然運用600か月の3層すべてで再現)
CONTRACT DECIDED (候補B: 参加国スコープ + 他Active War裏付け除外)
P28-008 IMPLEMENTATION READY
```

- `execute_peace_settlement`の占領状態正常化は、終結Warの`war_id`で絞れておらず、**「終結Warの主要当事国が所有する、現在占領中の全州」を占領者に関係なく所有国へ戻す**。同時進行中の別Warの占領と、そのWarの前線Divisionまで巻き込む。
- **現行の州データは占領元Warを識別できない**。本番の占領経路は`controller_country`しか書かず、`state.war_id`を書く唯一の関数`process_occupation`は本番Systemから一度も呼ばれない。したがって「`war_id`で単純に絞る」案は、本番では**何も正常化しない**修正になる（§4で実測）。
- 自然運用の標準シナリオ600か月で**1件**実際に発火した（war3のE2白紙講和がwar2の占領州s9を消去）。P28-007報告書が「E2による決定論的な連鎖ずれ」と分類した差分（war2の終結が6日遅延、war5の開戦遅延、`p27_002_c`のDwarf・Oceanic月120財政値の変化）は、**E2自体ではなくこの副作用が原因**だったと実測で判明した（候補B/Cでは全て講和前の値に戻る）。
- 採用候補は **B**（既存フィールドだけで足りる）。次の原子的タスクは P28-008（Bの実装）。Save V1は維持、migration不要。

## 1. 基準状態

- 開始時 `git rev-parse HEAD`: `b92d96e13f2d4efe2cd72f53232731d73ee2684e`（期待値と一致）
- 開始時 `git status --short`: 出力なし（クリーン）
- 前タスク最終基準（P28-007の最終実測値、今回の開始時には再実行していない）: 2755 passed / 0 failed / 1 ignored、clippy warnings 0
- 今回の最終実測値は§10に記載（同値）。

## 2. 実コードで確認した事実（過去報告書の記述はコードの代わりに使っていない）

| 項目 | 確認結果（場所） |
|---|---|
| 正規化の条件 | `execute_peace_settlement`ステップ2: `!is_sea && (state.war_id == Some(war_id) \|\| state.controller_country.is_some()) && (owner == primary_attacker \|\| owner == primary_defender)` → `controller=owner`、`occupation_progress=0`、`original_owner=None`、`war_id=None`（`src/war/peace.rs`、`// 2. 占領状態の正常化`）。`war_id`の一致は**OR**の片側にすぎず、実質的な条件は「`controller_country`が設定済み」＋「所有国が主要当事国」。占領者がどの国かは見ない。 |
| 当事国スコープ | 主要当事国（`primary_attacker_id()`/`primary_defender_id()`）の**所有州のみ**。支持国（`attackers`/`defenders`の他の要素）所有の州は対象外。 |
| `state.war_id`の書込 | `war/occupation.rs:38`の`process_occupation`のみ。**本番Systemの呼び出し元は存在せず**、呼ぶのは`war/tests.rs:1264`の1テストだけ。クリア側は`peace.rs`・`country/elimination.rs:456`・`state/data.rs:346`。 |
| 本番の占領経路 | `military::invasion::occupy_state`（`invasion.rs:273`）は`controller_country`のみを書く。呼び出しは`invasion.rs:105`（敵軍なしの即占領）と`military/update.rs:420`（戦闘勝利）の2箇所。`original_owner`・`occupation_progress`・`occupation_policy`も本番では書かれない。 |
| 実Appでの証拠 | §3.2の実Appログで、占領中のs3は常に`war_id=None`（占領元Warを記録していない）。 |
| 占領の前提 | `invasion.rs:47`: 占領は`are_countries_at_war(division_owner, 直前のcontroller)`が必要。つまり占領者は**何らかのActive Warの当事国**。同じ2国が別々のActive Warで敵対することは宣言時に拒否される（`war/data.rs`の`already_at_war`）。 |
| 講和ステップ5 | `relocate_hostile_territory_divisions`（`peace.rs:177`）は**全Division**を走査し、「所有国でもcontrollerでもない州」にいるDivisionを、Warを問わず自国領へ移動して`Idle`にする。`Battle`・`combat_id`には触れない。戦闘中止（ステップ3）は`battle.war_id == 終結war_id`のみ。 |
| 国家消滅 | `elimination.rs:451`の占領整理は`controller == 消滅国`の州のみ。本問題の影響を受けない。 |
| E1 | `invalidate_other_wars_targeting_state`は戦争目標州が同じWarのみ（今回の問題とは別経路）。 |
| 同日終結の順序 | `handle_daily_war_resolution`ステップ3は`WarRegistry.wars.values()`（HashMap、プロセスごとに反復順が変わる）をソートせずに走査し、**ループ内で各Warの`evaluate_war_capitulation`を直前の講和の後に評価**する。ステップ3.5（E2）も同様に未ソート。E1の連鎖だけが`WarId`昇順。 |
| Save V1 | `SaveGameV1.states: Vec<StateData>`（`save/dto.rs:84`）。`controller_country`・`war_id`・`occupation_progress`・`original_owner`・`occupation_policy`は**すべて既にV1に含まれ**（`#[serde(default)]`）、`save/validate.rs:784`が`war_id`の実在を検証する（Warは削除されないため実在し続ける）。どの候補もSaveフィールドの追加・削除を要しない。 |

## 3. 再現結果

### 3.1 関数レベル（実`execute_peace_settlement`、手組みレジストリ、一時テスト）

国: X=c0, Y=c1, Z=c2, W=c3, V=c4。州: s0,s1=X / s10,s11,s12=Y / s2=Z / s3=W / s4=V。判定は終結直後のowner・controller・War・Division・Battleの実測。**「候補A」=現行コード。**

| # | 状況（War ID・当事国・占領） | 現行(A)の結果 | 実害 |
|---|---|---|---|
| S1 | 単一War0(X対Y, 目標s10)。X占領: s10,s11。X師団はs11。War0を白紙講和 | s10,s11→Y支配、X師団→s0、休戦c0-c1 | なし（正しい） |
| S2 | War0(X対Y)とWar1(Z対Y, 目標s11)。X占領s10、Z占領s11。**War0のみ**白紙講和（理由`Frontline Unreachable`、E2相当） | s10→Y（正）。**s11もY支配に戻る**。Z師団s11→s2へ移動。**War1はActiveのまま** | **あり**: 継続中War1の占領と師団位置が消える |
| S2b | S2に加え、War1の戦闘(Battle0, s12, Z師団=攻撃/Y師団=防御, 両者Fighting)が進行中。War0のみ白紙講和 | **Z師団がs12→s2へ移動しIdleに**。Battle0は`Ongoing`のままZ師団を参照、`combat_id=Some(0)`も残る | **あり**: 戦闘中Divisionが戦場から消え、Battleが不整合。Save検証は`combat_id`の逆参照しか見ないため**検出されず永続化される** |
| S3a | War0(X対Y)とWar2(W対X, 目標s1)。W占領s1(X所有)。War0のみ白紙講和 | s1がXに戻り、W師団s1→s3へ移動。War2はActive | **あり**（当事国を共有: Xが一方で攻撃国・他方で防御国） |
| S3b | War0(X対Y)が降伏→割譲(s10→X)、War1(Z対Y, 目標s11, Z占領s11) | s10はXへ移転（正）。**s11のZ占領が消え**Z師団s11→s2。War1はActive | **あり**: 割譲経路でも発生 |
| S3b2 | S3bでWar1の目標もs10 | War1はE1で`War Goal No Longer Achievable`白紙、s11もYへ（War1自身の終結として正しい） | なし |
| S3c | Yの最後の陸州s10をX割譲→国家消滅。War1(Z対Y, 目標s10)は`Participant Country Eliminated`でZ勝利。別War2(V対X, V占領s1) | 消滅処理は正常。**s1のV占領が消える** | **あり**（War2の占領のみ。消滅処理本体は無問題） |
| S9 | War0の防御側に支持国W参加(防御{Y,W})。W所有s3をXが占領中。War0白紙講和 | **s3はX支配のまま残留**（`war_id`は終結済みWar0を指す）| **あり（逆方向）**: 終結済みWarの占領が残る既存不具合（支持国所有州は主要当事国でないため対象外）|
| S10 | 連鎖占領: W所有s3をYがWar E(Y対W)で占領→XがWar0(X対Y)で奪取。War0白紙講和 | **s3はX支配のまま残留** | あり（稀。所有国Wが終結Warの当事国でない） |

### 3.2 実App制御シナリオ（P26-008-Bの90日シナリオから隔離を外した再現）

実データ上でArcadia(c0)対Elfin(c1)の制御War0（戦争目標なしの合成Warを`add_war`、Arcadiaの陸軍を増強）を進める。隔離（`isolate_elfin_from_unrelated_wars`）は適用しない。

| 日付 | 状態 |
|---|---|
| 1800/03/13 | War0 Active（c0対c1）。Arcadia占領中のElfin所有州: **s3,s4,s14,s16,s18の5州**。d0(Arcadia, 兵力78400)はs18にIdle。Battleなし。 |
| 1800/03/14 | **同日に宣戦されたWar1(c3対c1)がE2で白紙講和**（理由`Frontline Unreachable`）。この1tickで**5州すべてがElfin支配へ戻り**、**d0がs18→s0（自国領）へ移動してIdle**。War0はActiveのまま。 |

（宣戦と同日にE2が成立するのは、`WarPreparation`が同日内で前線を生成し、国境が最初から無いWarはその日のうちにステップ3.5で終結するため。P28-007報告書§3.2末尾で訂正済みの挙動。）

### 3.3 自然運用（標準シナリオ・全AI・600か月、B0基準の一時診断ハーネスで実測）

占領正常化の発生箇所を「その日に終結したWarの中に、占領者と所有国が敵対するものがあるか」で分類した。

| 指標 | 実測 |
|---|---|
| 同時Activeの最大数 | 3 |
| 当事国を共有する同時Warが存在した日数 | 2303日（m180以降は増えない: war6とwar9は当事国を共有しない） |
| 終結したWar（講和）総数 / うち他のActive Warと当事国を共有していた時の終結 | 8 / 2 |
| 占領正常化（州数）: 正当(終結Warの敵対ペア) | 5州 |
| 占領正常化: **別Warの占領を巻き込んだ(COLLATERAL)** | **1州**（1801/03/22, war3のE2が所有国c3のs9・占領者c2を消去。c2はActiveのwar2の攻撃国） |
| その他（終結ペアに無関係な正常化） | 0 |
| 敵領残存陸軍の移動のうち、終結Warに無関係なもの | 0（正当な2件は1801/04/12のwar2自身の終結） |

この1件の影響: war2はs9を再占領しなければならず、**終結が1801/04/06→04/12に6日遅延**、war5の開戦が05/07→05/13へ遅れ、`p27_002_c`の月120財政値（Dwarf・Oceanic）が変化した。36・120・240・600か月の各時点でActive数・終結数は同じ（m36: 1/5、m120: 2/6、m240: 2/8、m600: 2/8）で、war6・war9の継続状態も不変。

### 3.4 同日の複数終結・順序

- **S7（同日に2件の降伏条件成立）**: War0(X対Y, 目標s10, X占領)とWar1(Z対Y, 目標s11, Z占領)。評価時点では**両方とも`DefenderCapitulated`**。実ステップ3のループ（評価→即講和）を再現:
  - 順序[War0, War1]: War0のみ勝利。War0の正常化がZのs11占領を消すため、War1は評価がfalseになりActiveのまま。
  - 順序[War1, War0]: War1のみ勝利。War1の正常化がXのs10占領を消し、War0がActiveのまま。
  - つまり**正当な勝利が必ず1件失われ、どちらが失われるかはHashMap反復順（プロセスごとに変わる）で決まる**。自然運用の600か月では同日に2件の降伏が重なった例は無い（到達可能だが未観測）。
- S6（降伏A＋白紙B＋白紙Cを全6順序）: 現行は2通りの結果（A先行ならX師団はs10に残る、B先行ならs0へ移動）。ただし実ハンドラはステップ3（降伏）→ステップ3.5（E2）の順に固定されるため、降伏と白紙の相互順序は実運用では1通り。降伏同士・白紙同士の順序は未ソート。
- S6b（E1が絡む順序）: 結果の差は`end_reason`ラベル（`War Goal No Longer Achievable`か`Frontline Unreachable`）のみで、実ハンドラのステップ順では到達しない。

### 3.5 save→load

実Appシナリオで、E2が成立する日(1800/03/14)の前(1800/03/10)・直前(03/13)・直後(03/14直前)にsave→load→2回連続load。**現行A・候補B・候補Cのいずれも、連続実行と全日次ログが完全一致**。候補Cの`war_id`もload後に保持され、2回目のloadで変化しない。

## 4. 原因と契約の切り分け

1. **占領をWar単位で識別できるか → できない。** 本番の占領は`controller_country`のみ。`war_id`はテスト専用経路でしか書かれない（実Appログで占領中も`war_id=None`）。よって「`war_id`で絞る」は、本番では**終結Warの占領を1つも戻さない**（残留案。不採用）。
2. **識別に使える既存情報**: 州の`owner`・`controller`、各Warの`attackers`/`defenders`、ActiveなWarの一覧。占領は`are_countries_at_war(占領者, 直前のcontroller)`を満たして発生し、同じペアが別Warで同時に敵対することは宣言時に拒否される。したがって「所有国と占領者が同じ終結Warの参加国で、かつ他のActive Warでは敵対していない」占領は、その終結Warに帰属するとみなせる。
3. **War Aの終結でWar Bの占領を保持すべきケース**: 占領者がAの参加国でない、または所有国がAの参加国でない（S2/S3a/S3b/S3c）、あるいは所有国と占領者が別のActive Warでも敵対している（そのWarが継続するため）。**両方を解除すべきケース**: 所有国・占領者がともにAの参加国で、他のActive Warの裏付けがない（S1、S3b2のWar1自身、支持国所有州S9）。
4. **割譲との優先順位**: 割譲（`transfer_region_ownership`）は州の`owner`・`controller`を新所有国に設定し`war_id`を消す（現行のまま、正しい）。正常化は割譲後の状態に対して実行され、割譲された州には何もしない。この順序は候補を問わず変更不要。
5. **国家消滅・講和・E2の同日**: 消滅は自身の占領整理が占領者基準で範囲が狭く無害（S3c）。講和系は共通の`execute_peace_settlement`を通るため、スコープを直せば降伏・白紙・E1・E2・プレイヤー講和の全経路に一括で効く。
6. **War単位の保持が必要か**: 連鎖占領（S10: 所有国が終結Warの当事国でない）だけはBでは識別できない（現行と同じ残留で悪化なし）。自然運用600か月では未観測。必要になった場合のみ候補Cで足りる（§5）。

## 5. 候補比較（一時パッチを環境変数で切替、検証後に全削除）

| 候補 | 内容 |
|---|---|
| A | 現状維持 |
| B | 正常化を「所有国・占領者がともに終結Warの**全**参加国、かつ他のActive Warで両者が敵対していない州」に限定。講和ステップ5は「他のActive Warで敵対している国の領内にいるDivision」を移動対象から除外 |
| C | 占領経路2箇所で`state.war_id`に占領元Warを記録し、正常化は`war_id==終結War`の州＋（`war_id==None`の州にはBと同じ規則をフォールバック）に限定 |
| D | 同日の終結順をWarId昇順にソートするのみ（状態帰属は変えない） |

| シナリオ | A(現状) | B | C | D |
|---|---|---|---|---|
| S1 単一War | 正常 | 同左 | 同左 | 同左 |
| S2 同じ防御国に2つのWar | War1の占領・師団が消える | **保持** | **保持** | 消える |
| S2b 戦闘中Division | 戦闘中Divisionが移動、Battle不整合 | **戦場に残り整合** | **同左** | 不整合 |
| S3a 当事国を一部共有 | War2の占領が消える | **保持** | **保持** | 消える |
| S3b 降伏(割譲)＋別War | War1の占領が消える | **保持** | **保持** | 消える |
| S3b2 E1連鎖 | 正常 | 同左 | 同左 | 同左 |
| S3c 国家消滅＋別War | War2の占領が消える | **保持**（消滅処理は同一） | **保持** | 消える |
| S6 同日3Warの順序 | 2通り | **1通り** | **1通り** | 2通り |
| S7 同日2降伏 | 片方の勝利が消滅（順序依存） | **両方勝利（順序非依存）** | **同左** | 決定論だが**常にB側の勝利が消滅** |
| S9 支持国所有州 | 占領が残留 | **解除** | **解除** | 残留 |
| S10 連鎖占領 | 残留 | 残留(現状どおり) | **解除** | 残留 |
| 実App制御(§3.2) | 5州消失＋d0がs18→s0 | **5州保持・d0はs18のまま** | **同左** | 5州消失 |
| 自然600か月のCOLLATERAL | 1件 | **0件** | **0件** | 1件 |
| 自然: war2終結/war5開戦 | 04/12 / 05/13 | **04/06 / 05/07（講和前の値）** | 同左 | 04/12 / 05/13 |
| 自然: war3終結 | 03/22 E2 | 03/22 E2（意図どおり） | 同左 | 03/22 E2 |
| 自然: war6・war9継続・勝敗種別 | 不変 | 不変 | 不変 | 不変 |
| save→load・2回load | 一致 | 一致 | 一致（`war_id`保持） | 未実施(状態帰属変更なし) |
| 全テスト(2755件) | 全通過 | 1件失敗（`p27_002_c`月120正規値が講和前の値へ戻る） | 同左 | 未実施 |

- B/Cの自然運用結果は**完全一致**。両者の差は連鎖占領S10のみ（Cが解除）。
- 月120財政値（`p27_002_c`）はB/Cで全6か国が**P28-005時点の値と一致**（Dwarf (0.140, 15453.41, 206.56)、Oceanic (0.130, 4695.83, 113.36)）。P28-007でDwarf・Oceanicを更新した原因はE2の前倒しではなく占領消去だったことを示す。一方、war7の開戦日（06/02→05/02）はB/Cでも前倒しのままで、こちらはE2による正当な連鎖ずれ。
- **終結済みWarの占領が残留する案は採用していない**: B/Cとも、S1・S3b2のWar1自身・S9では終結Warの占領を解除する（残留しない）。Dは状態帰属を変えないためそもそも問題を解消しない（却下）。

**採用: B。却下理由**
- A: 自然運用・実App・同日降伏で実害が再現（勝利の消滅・占領と戦闘中Divisionの消去）。
- C: Bに加えて本番の占領経路2箇所と正常化の変更が必要だが、得られるのは連鎖占領S10のみで、自然運用では未観測。`war_id`をSaveに書き始める意味変更（スキーマ追加は不要、旧Saveの`None`は同じ規則へフォールバック）も伴う。必要になった時点で、Bの上に追加できる（Bの規則はCのフォールバックと同一）。
- D: 順序だけでは占領消去を防げず、S7で「常にB側が負ける」決定論的な誤りになる。ただし反復順を固定する衛生対策としては無害（Low、別件）。

**Cの規模（参考）**: 新規永続フィールド0（`StateData.war_id`は既にSave V1）、変更箇所は占領経路2か所＋正常化1か所（一時パッチ実測で約25行）、旧Saveは`war_id=None`としてBの規則へ自動フォールバックするためmigration不要。

## 6. 状態遷移契約（採用B）

`execute_peace_settlement`（白紙講和・割譲・降伏・E1・E2・プレイヤー講和の共通経路）のステップ2を次のとおり置き換える。`P(W)` = 終結Wの`attackers ∪ defenders`（主要当事国だけでなく全参加国）。

1. 領土移転（`CedeWarGoalRegion`）は現行どおり先に実施する（owner・controllerを新所有国へ、`war_id`消去）。変更しない。
2. 各陸州で`controller != owner`のとき、次を**すべて**満たす場合だけ`controller=owner`、`occupation_progress=0`、`original_owner=None`、`war_id=None`に戻す。
   - `owner ∈ P(W)` かつ `controller ∈ P(W)`
   - **他のActive War**（W以外）で、`owner`と`controller`が敵対側に分かれていない
3. 上記以外の占領は変更しない（別Warの占領、占領者または所有国がWの当事国でない占領）。
4. ステップ5（敵領残存陸軍の帰還）は、**W以外のActive Warで、そのDivisionの所有国が州のcontrollerまたはownerと敵対している**Divisionを移動対象から除外する（その国は別Warで正当に敵領内にいる）。除外以外の挙動（Idle化、経路クリア、帰還先優先順位、Destroyed化）は変更しない。
5. 戦闘中止（ステップ3）・前線削除・休戦作成・終結情報は現行どおり。

遷移の整合: 割譲された州はステップ1でcontrollerがowner＝新所有国になり、ステップ2の対象外。国家消滅は現行の`apply_country_elimination`（`controller==消滅国`の整理）に委ね、変更しない。

**Save V1維持可否: 維持できる。** 判定は`owner`・`controller`・Warの参加国・Warのstatusだけで、すべてSave V1に含まれる。load後に同じ条件で再計算でき、旧Saveも意味が変わらない（`war_id`は参照しない）。migration不要。

**決定性・同日複数War**: Bでは、各Warの正常化が他Warの占領に触れないため、S6は1通り、S7は両勝利となり順序に依存しない（実測）。HashMap反復順のソート（D）は、Bの採用後も結果を変えないことを確認した範囲では必須ではなく、別の衛生対策として残す。

## 7. 設計成果（要求項目の確定）

1. **実害の再現判定と根本原因**: 再現（関数・実App・自然運用）。根本原因は、占領元Warを記録しない本番の占領経路と、終結Warの当事国所有州をすべて戻す無差別な正常化・Division帰還。
2. **影響する例**: §3.1〜3.3（S2/S2b/S3a/S3b/S3c/S7、実App 5州・d0、自然: war3(E2)→war2のs9）。
3. **現行データの限界**: 占領元Warは識別不能（`war_id`は本番で未使用）。識別は`owner`・`controller`・参加国・Active状態の組合せで可能（連鎖占領を除く）。
4. **採用B、却下A/C/D**: §5。
5. **状態遷移契約**: §6。
6. **Save V1**: 維持、migration不要。
7. **決定性**: §3.4・§6。
8. **次の原子的タスク**: P28-008（Bの実装）。具体的な内容は§8。
9. **深刻度**: §9。

## 8. 次の原子的タスク（1件）: P28-008

`src/war/peace.rs`の占領状態正常化（ステップ2）とDivision帰還（ステップ5）を§6の契約へ変更する（本番コードは`peace.rs`のみ、`relocate_hostile_territory_divisions`は既存の2引数版を残して新しいskip付き版へ委譲する）。恒久テストとして最低限、次を追加する。

- 単一War（`S1`相当）で、所有国・占領者ともにWの参加国の占領がすべて戻る。
- 同じ防御国への別War（`S2`）で、Aのみの白紙講和が、Bの占領・Z師団の位置・状態を変えない。
- 戦闘中Division（`S2b`）が、別Warの講和で戦場から移動されず、Battleと整合する。
- 同日2降伏（`S7`）が、実ステップ3のループ再現で順序によらず両方勝利する。
- 支持国所有州（`S9`）が、Wの終結で解除される。
- 自然運用のE2回帰（war3）: war2の占領s9が消えず、war2が1801/04/06に終結する。

既存テストへの影響（実測済み）: `tests/p27_002_c_residual_fiscal_self_stabilization_e2e_test.rs`の`M120_EXPECTED`のDwarf・Oceanicを、P28-005時点の値へ戻す必要がある（P28-007の更新理由コメントは誤った因果を述べているため、正しい因果へ書き換える）。`tests/p26_008_b_...`のElfin隔離ヘルパーが不要になる可能性があるが、**隔離なしで全アサーションが通ることは本タスクでは未検証**であり、P28-008で確認する。`p27_005`は`war_goals`ガード側の対応で影響なし。

## 9. Critical / High / Medium / Low

- **Critical**: なし。
- **High**: 同日に2件の降伏条件が成立すると、**正当な勝利が必ず1件失われ、どちらかがHashMap反復順（プロセスごとに変わる）で決まる**（S7）。ルール上到達可能だが、自然運用600か月では同日に重なった例は未観測。
- **Medium**:
  - 別Warの占領と前線Divisionが、無関係なWarの講和で消去・移動される（S2/S2b/S3a/S3b/S3c、実App 5州＋d0、自然運用1件）。S2bでは戦闘中Divisionが戦場から消えBattleが不整合になり、Save検証がこれを検出しない。
  - 支持国所有州の占領が、Wの終結後も残留する（S9、既存不具合、逆方向）。
- **Low**:
  - 連鎖占領で所有国が終結Warの当事国でない占領が残留する（S10、自然運用で未観測、Bでも現状から悪化しない）。
  - ステップ3・3.5がHashMap反復順（未ソート）。Bの採用後は結果を変えない範囲だが、衛生対策としてWarId昇順を検討できる。
  - **P28-007の記録の訂正（既存報告書は変更しない）**: P28-007-PRE-001・P28-007の報告書および`p27_002_c`のコメントは、war2・war5のタイミング差とDwarf・Oceanicの月120財政値の変化を「E2の前倒しによる決定論的連鎖ずれ」と説明し、P28-007-PRE-001は「E2は構造的に他Warへ影響しない」としていた。実際には、E2の白紙講和が共通の占領正常化を通じてwar2の占領を消したことが原因である（B/Cでは全て講和前の値へ戻る）。war7の開戦日の前倒し（06/02→05/02）のみがE2自体の連鎖ずれとして残る。

## 10. 検証・実行したコマンド・保護

**一時パッチ・一時テストの扱い**: 候補B/C/Dは`P28_008_CAND`環境変数で切替える一時パッチ（`peace.rs`・`update.rs`・`invasion.rs`・`military/update.rs`）として適用し、検証後に`git apply -R`で全て復元、4ファイルが`git hash-object`でHEADのblobと一致することを確認した。なお`git apply -R`の後、`src/war/peace.rs`だけは行末(CRLF)由来のstat差で`git status`にmodified表示が残った（`git diff`は0行、blob hashはHEADと一致）。保存すべき未コミット内容が無いことを確認したうえで、同ファイルのみ`git checkout --`で復元した（`git update-index --refresh`では解消しなかった）。一時テスト3件（`tests/p28_008_temp_registry.rs`・`p28_008_temp_app.rs`・`p28_008_temp_natural.rs`）も削除済み。パッチと一時テストはスクラッチパッドに退避しただけで、コミットしていない。

**個別実行（`rg --files tests`で実名確認、復元後に再実行）**:

| テスト | 結果 |
|---|---|
| lib `war::update::`（P28-007の恒久ユニット） | 9 passed |
| p21_019_crisis_war_peace_resolution_e2e_test | 4 passed |
| p21_021_country_elimination_e2e_test | 1 passed |
| p21_016_multilateral_war_e2e_test | 1 passed |
| p28_005_ai_railway_priority_e2e_test | 3 passed |
| p28_003_disconnected_friendly_frontline_delivery_e2e_test | 2 passed |
| p28_002_wartime_military_factory_construction_e2e_test | 3 passed |
| p27_fu_001_elimination_peace_e2e_test | 3 passed |
| p27_fu_001_ai_retreat_alternative_e2e_test | 3 passed |
| p27_007_zero_damage_battle_termination_e2e_test | 2 passed |
| p27_005_offensive_frontline_supply_bridge_e2e_test | 10 passed |
| p26_008_b_supply_aware_offensive_gate_e2e_test | 19 passed |
| p26_008_a_supply_aware_recruitment_gate_e2e_test | 30 passed |
| p27_002_c_residual_fiscal_self_stabilization_e2e_test | 20 passed |
| land_war_combat_peace_test | 4 passed |
| p21_005_army_frontline_e2e_test | 2 passed |
| p21_save_003_end_to_end_test | 4 passed |
| p26_006_rev_001_ai_save_resume_determinism_e2e_test | 9 passed |
| diplomacy_tests | 5 passed |

**Cargo逐次実行（復元後の実測）**:
```
cargo test --all-targets -j 2
→ 2755 passed; 0 failed; 1 ignored

cargo clippy --all-targets --all-features -- -D warnings
→ warning 0件
```
（候補B・Cを有効にした全テスト実行は、いずれも2754 passed / 1 failed で、失敗は`p27_002_c`の月120正規値のみ。上記§5のとおり。）

**検証の範囲と未検証事項**: 自然運用は標準シナリオ1本（固定シード）の600か月のみ（P28-003-PRE-001・P28-007-PRE-001と同じ条件）。複数シードは検証していない。連鎖占領（S10）・同日2降伏（S7）は関数レベルの再現のみで自然運用では未観測。候補Bの下で`p26_008_b`を隔離ヘルパーなしで通すことは未検証。

**保護**: `saves/savegame_v1.ron`は読み書き・復元・ステージしていない。`tests/land_war_combat_peace_test.rs`は変更・整形していない。ワークスペース全体のformatは行っていない。既存報告書は変更していない。非決定的PNG 9件（`verification_logs/p20-007/screenshots/03_playing_topbar.png`、`verification_logs/p20-009/screenshots/{04_playing_ja_jp,05_playing_en_us,06_playing_ja_jp_again}.png`、`verification_logs/phase-21/p21-save-002e/screenshots/{01_playing_topbar_with_save_load_buttons,02_after_save_click,03_load_confirm_dialog_open,04_after_cancel_dialog_closed,05_after_load_success}.png`）は、全テスト実行後に各ファイルを個別に`git checkout --`で復元した。`git add .`・`git add -A`・push・tag・amend・rebase・reset・stashは使用していない。

P27-005の手動プレイ確認は今回も未実施であり、引き継ぎ事項として残す。
