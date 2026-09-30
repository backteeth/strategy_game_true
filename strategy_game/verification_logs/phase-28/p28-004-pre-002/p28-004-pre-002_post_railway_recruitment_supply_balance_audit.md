# P28-004-PRE-002 Railway実装後の戦時AI募兵・補給生産均衡の再監査

P28-005でAIのRailway優先建設が実装された後の現在の本番コードを基準に、P28-004-PRE-001が確認した「AI募兵ゲートが月次生産(フロー)を見ない」欠陥について、候補B1(交戦中のみ、募兵後需要≤実月産)・B2(常時、診断用)を実測し直した設計監査である。本番コード・RONデータ・既存テストは恒久的に変更していない。一時パッチと一時診断は検証後に完全に除去した(§9)。

## 1. 総合判定・根本原因判定・設計判定

```text
P28-004-PRE-002 ANALYSIS COMPLETE
RECRUITMENT ROOT CAUSE REVISED
POST-P28-005 EFFECT CONFIRMED
DESIGN USER DECISION REQUIRED
P28-004 IMPLEMENTATION BLOCKED
```

- **RECRUITMENT ROOT CAUSE REVISED**: 「募兵ゲートが月次生産と募兵後需要を比較しない」という機構自体の欠陥はP28-004-PRE-001の確認どおり不変である。しかし、Railway実装後の世界ではこの欠陥の**実害の性質が変わった**。P28-004-PRE-001の時間軸では、この欠陥は「備蓄を持続可能性と誤認して軍拡を続け、入力資源が枯渇して生産が崩壊する」という**入力資源枯渇型**の停止を引き起こしていた。Railway実装後は、財政・物流の長期安定化により入力資源枯渇型の崩壊(Dwarfの1809年Iron枯渇に相当する事象)は観測されなくなった一方、B1候補自体が**新しい種類の停止**(1個Divisionしか維持できない国が、貧弱な戦力のまま前線に固定され続ける恒久的な補給不足)を生むことを本監査で確認した。
- **POST-P28-005 EFFECT CONFIRMED**: B1を適用した効果を現在の本番コードで精密に再測定した。前回監査(P28-004-PRE-001)と異なり、**B1は6か国合計の補給不足日数を長期的に悪化させる**(240か月で11,134→15,812日、+42%)。開戦初期の戦争(旧war0、Dwarf→Arcadia)の勝敗自体は変わらない(両条件ともDwarf勝利)という点はPRE-001の主な懸念を解消したが、新たに継続戦争数が240〜600か月にわたって高止まりする(3件で固定、PRE-001時点は120か月3件→240か月1件)ことを確認した。
- **DESIGN USER DECISION REQUIRED**: 「募兵後需要≤実月産」という判定式自体は生産と需要を正しく比較する原理的に正しい式である。しかし、Railway実装後の世界でこの式を交戦中に適用すると、総合的な補給不足日数と継続戦争数の両方で**現状(B0)より悪化する**ことが判明した。原理的な正しさと、適用範囲(B1)の採用可否は別の問題であり、後者はゲームデザイン上の判断が必要である(§7)。
- **P28-004 IMPLEMENTATION BLOCKED**: 上のユーザー判断待ち。

## 2. 開始HEAD・開始時テスト基準

- `git rev-parse HEAD` → `3d76c74304daa7236c88be7285aaf774b2145df7`(期待値と一致)
- `git status --short` → 空
- 開始時に実測: `cargo test --all-targets -j 2` → **2752 passed / 0 failed / 1 ignored**(85バイナリ)。`cargo clippy --all-targets --all-features -- -D warnings` → **警告0**。前タスク(P28-005)の最終実測と完全一致することを確認した。

## 3. 実コードの再確認(募兵判断・月次生産・補給ゲートの順序)

`rg`で定義場所を確認し、以下の順序が実コードと一致することを確認した(前回報告書の式を証拠として代用していない)。

| 段階 | 実コード | 確認結果 |
|---|---|---|
| System順序 | `app::time::DailySimulationSet`(`Economy`→`Research`→`Diplomacy`→`CountryAi`→...) | `handle_monthly_ai_recruitment`は`CountryAi`、月次生産(`economy::production`)は`Economy`に属する。`Economy`は`CountryAi`より**先**に実行される(`configure_sets`の宣言順)ため、月替わりの同一フレーム内で、月次生産が確定した**直後**の`building_operation_ratios`を募兵判断が読む。P28-004-PRE-001の「前月の稼働率」という記述は、月をまたいだ一般的な参照範囲の説明であり、募兵判断自体は**その月の生産確定直後**の最新値を見ることを本監査で確認した(§4の境界監査で実測により裏付け) |
| 募兵判断本体 | `military::ai::decide_and_recruit_for_country` | ①募兵キュー非空→見送り ②manpower帳簿不整合→見送り ③目標Division数到達→見送り ④補給ゲート(`evaluate_recruitment_supply_gate`)→停止なら見送り ⑤manpower不足→見送り ⑥募兵州選択失敗→見送り、の順。前回報告書の記述と完全一致 |
| 補給ゲート | `military::recruitment_supply_gate::evaluate_recruitment_supply_gate` | 停止20日(平時)/40日(戦時)、再開35日/70日の真のヒステリシス。bridge不足0、配送充足1.0の3条件をすべて満たさないと再開しない。生産量(フロー)は見ない、という記述は不変 |
| 訓練完了 | `military::recruitment::request_recruitment`→`MilitaryAction`(日次)`process_recruitment` | 歩兵30日。完成翌日の日次補給から需要1.0/日が加わる |
| 建設完了 | `country_ai::process_construction_ai_with_wartime_supply`(優先度2.5 MilitaryFactory・優先度2.7 Railway、P28-005で追加) | 建設完了は月次生産の`building_operation_ratios`を次月以降に反映する。募兵ゲート自体は変更していない |
| 月次生産 | `economy::production::process_country_production` | 実月産=100×level×稼働率(P28-002・P28-005と同一式)。将来生産(建設中の工場)は記録されない、という記述は不変 |

## 4. B1／B2の一時パッチ

`src/military/ai.rs`の`decide_and_recruit_for_country`内、既存の補給ゲート判定の直後(人的資源条件の直前、§3の順序どおり)に、環境変数`P28RECRUIT`で有効化する一時パッチを追加した。

```text
判定: actual_monthly_production(units) >= (現在総需要units + 新規1Division需要units) × 30
実装: crate::economy::resources::military_supplies_value_to_units(actual_monthly_production)
      >= (country_daily_demand_units + proposed_division_daily_demand_units) × 30
```

- `actual_monthly_production` = Σ所有州(100 × MilitaryFactory level × 今月の稼働率)。100.0は`assets/data/buildings.ron`のMilitaryFactory出力値(P28-002の`realised_monthly_military_supplies`と同一式)。一時パッチではBuildingRegistryを新規引数として通さず、RON確定値を直接使用した(本番実装時はBuildingRegistryから読む)。
- `country_daily_demand_units`・`proposed_division_daily_demand_units`は既存の正規関数(`military::supply_initialization`・`military::recruitment_supply_gate`)をそのまま呼び、需要式を複製していない。
- **B1**: `war_registry.is_country_at_war(country_id)`が真の場合だけ適用。
- **B2**(診断用): 常時適用。
- 条件を満たさない場合、既存の`AiRecruitmentOutcome::SkippedSupplyShortage`を返す(P28-004-PRE-001の実装提案どおり、新しいoutcome種別を追加しない)。
- 平時の募兵判断、プレイヤー国家(呼び出し元`evaluate_ai_monthly_recruitment`の既存除外)、消滅国家、既存の備蓄ゲート・manpower条件は一切変更していない。

## 5. 募兵判断の境界監査

| 境界 | 確認結果 |
|---|---|
| 平時↔交戦中の切替日 | B1は`war_registry.is_country_at_war`を毎回再評価する純粋条件のため、宣戦・終戦の当日から即座に適用/非適用が切り替わる。永続状態を持たないため、既存の`recruitment_supply_blocked`ヒステリシスとは独立に働く |
| 募兵キューが空でない場合 | 既存の①判定が先に`SkippedQueueNotEmpty`を返すため、B1/B2はキュー空の月にしか評価されない(既存の1か月1件の契約を壊さない) |
| 直近月産が0の場合 | `actual_monthly_production=0`なら、既存Division由来の需要が正である限り`D'>0`となり、`0 >= D'`は常に偽 → **MilitaryFactoryを1つも持たない交戦国は、B1適用下でその戦争が続く限り新規募兵が恒久的に不可能になる**(意図した保守的な挙動だが、鋭い境界であることを確認した) |
| 直近月産が欠落・未確定の場合 | `military_supplies_value_to_units`がErrを返す入力(NaN・負数等)は安全側の`SkippedSupplyShortage`に倒す(既存の防御的分岐と同じ規律) |
| 月次生産直前と直後 | §3で確認したSystem順序により、募兵判断は**その月の生産確定直後**の値を見る(古い月のデータを参照する期間はない)。90日診断(§6)でも、月替わり当日のMONログで生産値が即座に更新されていることを確認した |
| 募兵後需要と月産が等しい/わずかに上回る場合 | `>=`で判定するため等号は許可(P28-002の実装規約と同じ)。わずかに下回る場合はB1がブロックする(§6のArcadia c0の実例で確認、P=46.5 < D'=60で1個目の追加Divisionを恒久的に抑止) |
| 既存Divisionが0個の場合 | `country_daily_demand_units=0`となり、新規1個分の需要だけがD'になる。実測でOceanic(c3、0個Division)が該当するケースを確認したが、同国は交戦しないため本監査ではB1が実際に評価されなかった |
| Divisionごとの`supply_usage`が異なる場合 | `proposed_division_daily_demand_units`は常に`STANDARD_INFANTRY_DEFINITION_ID`(募兵対象は歩兵のみ、既存の制約)を使うため、この境界は現行の募兵経路では発生しない |
| 30日訓練中の生産・所有州・稼働率・戦争状態の変化 | B1は募兵**要求時点**だけを判定し、訓練中の変化を再評価しない(既存の`request_recruitment`一回限りの設計を維持)。訓練完了時に需要が既に増えていても、次回月次判断で改めてB1を評価する |
| 複数AI国家が同日に判断する場合 | `evaluate_ai_monthly_recruitment`は`CountryId`昇順で1か国ずつ独立に評価し、B1もその国自身のstate/military registryだけを読む(既存の国家間分離を維持) |
| save→load後の最初の判断 | `actual_monthly_production`は毎回`state_registry`から再計算する純粋関数のため、save→loadを挟んでも同じ状態からは同じ判定になる(§8で決定性を確認) |
| 戦時MilitaryFactory・Railway完成前後 | 完成前は`actual_monthly_production`が低く抑止されやすい。完成直後の月から即座に評価に反映される(永続状態なし、P28-005のRailway再評価と同じ設計原則) |

## 6. 自然運用での数値検証(6か国、全AI・無操作)

一時診断(`tests/p28_004_pre_002_temp_diagnostic.rs`、P28-005の長期診断ハーネスに`rail=`欄を追加したもの。実カレンダー)で、B0(現行HEAD)・B1(交戦中のみ)・B2(常時、診断用)を測定した。

### 6.1 戦争の進捗(累計/継続中/終結)

| 観測点 | B0 | B1 | B2 |
|---|---|---|---|
| 90日 | 0/0/0 | 0/0/0 | 0/0/0 |
| 36か月 | 6/1/5 | 7/2/5 | 0/0/0 |
| 120か月 | 8/2/6 | 10/3/7 | 0/0/0 |
| 180か月 | 9/1/8 | 10/3/7 | 0/0/0 |
| 240か月 | 10/2/8 | 10/3/7 | 0/0/0 |

B1の継続戦争数は**120か月から600か月まで一貫して3件のまま変化しない**(B0は1〜2件の間で変動)。B2は診断用(平時にも同条件を課す)で、P28-004-PRE-001と同じく戦争が起きない(0/0/0が600か月まで継続、既知の副作用)。

### 6.2 6か国合計の補給不足日数(<0.34)

| 観測点 | B0 | B1 | 差分 |
|---|---|---|---|
| 90日 | 258 | 258 | ±0 |
| 36か月 | 1,668 | 1,487 | **−11%** |
| 120か月 | 6,233 | 6,593 | +6% |
| 240か月 | 11,134 | 15,812 | **+42%** |
| 360か月 | 17,287 | 25,374 | +47% |
| 480か月 | 24,694 | 34,983 | +42% |
| 600か月 | 32,798 | 44,255 | +35% |

短期(36か月まで)はB1がわずかに改善するが、**中長期(120か月以降)ではB1が一貫して悪化し、悪化幅は時間とともに拡大する**。P28-004-PRE-001が測定した「A_war適用で16%改善」(240か月、24,948→20,979)とは方向が逆転している。原因は§6.4で特定する。

### 6.3 財政(P28-005の長期安定性との整合、120〜600か月)

B0・B1とも、P28-005が確認した長期財政安定性は維持されている。

| 月 | 国 | B0(税率/加重LS/国庫) | B1(税率/加重LS/国庫) |
|---|---|---|---|
| 120 | Arcadia | 0.16/52/4517 | 0.19/45/3291 |
| 240 | Arcadia | 0.15/55/3722 | 0.19/45/3542 |
| 600 | Arcadia | 0.19/45/4763 | 0.24/36/3516 |
| 600 | Dwarf | 0.13/72/33031 | 0.16/57/15060 |

B0・B1ともLS<32の月は6か国とも0、最大税率は0.24(Passguard/Arcadia)以内。B1の数値はP28-005報告書の「A_war一時診断」の数値と完全に一致する(独立した実測での再現性を確認)。B1はB0よりArcadia・Dwarfの加重LSが低め・税率が高めで推移するが、崩壊の兆候(LS<32、税率>0.25)には至らない。

### 6.4 開戦初期の戦争(旧war0、Dwarf→Arcadia)の分岐点

| 条件 | Arcadia(防御側) | Dwarf(攻撃側) | 結末 |
|---|---|---|---|
| B0 | 1800年内に2個目のDivisionを募兵(月産46.5<月需要60)。以後、補給不足日が増加し続ける | 2個Division | **Dwarf勝利**(1801/01/30、Defender Capitulation) |
| B1 | 2個目のDivisionの募兵をB1が恒久的に抑止(月産46.5<募兵後需要60)。1個Division・備蓄が数百日分まで蓄積、完全補給 | 2個Division(B1の影響なし、Dwarf自身は月産が需要を上回っていた) | **Dwarf勝利**(1800/12/30、Defender Capitulation、終結が1か月早まる) |

**P28-004-PRE-001が懸念した「開戦初期の勝敗が変わる」問題は、Railway実装後の世界では発生しない**(両条件ともDwarf勝利)。原因を`evaluate_military_ai_decision`の判定理由の集計で確認したところ、B1下のArcadiaは`EnemyStronger`判定が186日(B0は96日)、`ComparableStrength`判定が2日(B0は117日)と、**1個の完全補給Divisionの方が2個の半補給Divisionより戦力評価で劣後する**ことが直接の原因である。戦力比較(`evaluate_powers`)はDivision数・manpowerに基づく生の戦力を見ており、補給充足率による実効力の低下を評価に織り込まないため、「募兵を抑止して1個を健全に保つ」戦略が、この評価式の上では「2個を半健康に保つ」戦略より劣る帰結になる。これはB1自体の欠陥ではなく、既存の戦力評価式(P28-004-PRE-001のLow L-2「敵国の全Divisionで数える戦力比」と同系統の、供給を評価に織り込まない設計)の限界である。

### 6.5 継続戦争3件(B1、120〜600か月で不変)の内訳と原因

| 戦争 | 攻撃側→防御側 | 開戦日 | 主な停止理由(240か月時点の日数集計) | 分類 |
|---|---|---|---|---|
| war4 | Elfin→Arcadia | 1801/01/24 | `c0:ComparableStrength` 6899日、`c1:ComparableStrength` 6899日 | **M-1(既存、継続)**: 両者の戦力が拮抗したまま膠着し、非戦闘の終結経路がないため終わらない。B1固有の原因ではない |
| war5 | Ferrowyn→Oceanic | 1801/01/30 | `c5:SupplyShortage` 6896日、`c3:NoAvailableDivision` 6899日 | **B1固有の新しい停止**: B1がFerrowynを1個Divisionに固定した結果、その1個が前線の補給充足率0.34を継続的に満たせず、攻勢供給ゲート(P26-008-B)が開かない。B0では複数Divisionを維持できたため、この特定の停滞は発生しなかった |
| war9 | Dwarf→Oceanic | 1808/07/02 | `c2:SupplyShortage` 4198日、`c3:NoAvailableDivision` 4199日 | **B1で未解消**: Dwarfは**平時**に11個Divisionまで拡大した(B1は交戦中しか制限しないため無関係)。月産(104〜238)が固定需要330を恒常的に下回る構造的な赤字を平時に抱え込んだまま開戦し、B1は12個目以降の増員だけを防いだが、既存の供給過大な11個の状態そのものは是正できない |

war9は、P28-004-PRE-001が指摘した「備蓄が多い時期に需要だけが増え、後で工場稼働率や補給が崩れる経路」(本監査§5・調査項目3に相当)が**平時の募兵を通じてRailway実装後も残る**ことの直接証拠である。B1は交戦中の増員だけを止めるため、平時に築かれた供給過大な軍備を遡って是正しない。

## 7. 設計判断

§6の実測により、B1は原理的に正しい判定式でありながら、Railway実装後の世界では次の理由で単純な採用を推奨できない。

- 6か国合計の補給不足日数が中長期で悪化する(240か月で+42%、600か月で+35%)。
- 継続戦争数が120か月以降3件で高止まりし、B0のように減少する局面がない。
- 悪化の主因(war5)は、B1が防御的な国を「少数精鋭」に固定した結果、既存の戦力評価式(供給を見ない)によってかえって弱く評価され、単独で膠着する新しい停止パターンである。

一方、当初PRE-001が懸念した「開戦初期の勝敗が変わる」問題は解消されており(§6.4)、原理(生産と需要の比較)自体の妥当性は揺らいでいない。既存の欠陥(war9型、平時の過剰募兵)も未解消のまま残っている。以上より、**A(実装推奨)ではなく、B(ユーザー判断)**を選ぶ。

### 判断が必要な具体的な質問(最大2件)

**質問1: B1を交戦中の募兵ゲートとして採用するか、それとも現状(B0)を維持するか。**

| 指標(240か月) | 案1: B1を採用 | 案2: 現状(B0)を維持(推奨) |
|---|---|---|
| 6か国合計 補給不足日数 | 15,812(悪化+42%) | 11,134(基準) |
| 継続戦争数(120〜600か月) | 3件で高止まり | 1〜2件で変動 |
| 開戦初期(旧war0)の勝敗 | Dwarf勝利(不変) | Dwarf勝利(基準) |
| 長期財政(P28-005安定性) | 維持(LS<32月0、税率≤0.24) | 維持(基準) |
| 新たに生じる副作用 | war5型(単独駐留国が戦力評価で劣後し膠着) | なし(war4型の既存M-1のみ) |

推奨理由: B1は原理的に正しい式だが、実測ではRailway実装後の総合的な補給健全性・戦争終結率のいずれも改善しない。既存の戦力評価式(供給非考慮)を先に見直さない限り、B1は「少数精鋭」を不当に不利にする副作用の方が大きい。

**質問2: war9型(平時の過剰募兵による恒久停止)を、B1とは別の独立した対策(例: 平時にも生産余力に応じた上限を設ける、緩やかな上限案)として今後扱うか、それとも当面は許容するか。**

- 今後扱う場合: 平時専用の緩やかな上限(P28-004-PRE-001のB案「備蓄で不足を許容」やD案「工場完成待ち」のような、硬い下限ではない設計)を新たな候補として次の監査で検討する。
- 当面許容する場合: war9型の停止はM-2として記録を継続し、次の原子的タスクの対象にはしない。

## 8. 既存契約への影響・保護の確認

- **P28-002の戦時MilitaryFactory建設**: `try_wartime_supply_construction`は無変更。§6の診断でDwarf・Ferrowynとも戦時MilitaryFactory建設が引き続き発生することを確認した(補給生産を増やす経路は維持)。
- **P28-003の飛び地Division除外・P27-005の攻撃中Division補給**: `offensive_delivery_division_ids`・前線補給は無変更。B1は募兵判断だけに介入し、前線・補給計算には一切触れていない。
- **P28-005のRailway優先建設・長期財政安定**: §6.3で確認したとおり、B0・B1とも600か月まで安定を維持する。Railway建設自体もB1の影響を受けない(建設AIと募兵AIは独立した判断経路)。
- **平時の募兵・プレイヤーの募兵・消滅国家**: B1は`war_registry.is_country_at_war`が真の場合だけ発火するため、平時の募兵判断は無変更。プレイヤー国家は`evaluate_ai_monthly_recruitment`の既存の除外(`Some(c.id) != player_country`)によりB1の対象外。消滅国家は`is_active()`フィルタにより対象外(いずれも変更していない既存経路)。
- **manpower・MilitaryEquipment・MilitarySuppliesの保存則**: B1は`request_recruitment`を呼ぶかどうかを判定するだけで、いずれの資源も直接変更しない(既存の契約どおり)。
- **Save V1**: 新規永続フィールドは不要(`actual_monthly_production`は`StateRegistry`から毎回再計算する純粋関数)。既存の`SkippedSupplyShortage`を再利用するため、`AiRecruitmentOutcome`にも新しい値は不要。
- **連続実行・save→loadの一致、決定性**: 600か月診断を独立プロセスで2回実行し(B1)、FIS行が完全一致することを確認した(決定的)。B1・B0とも国家・州の登録順を変更する追加検証は行っていない(既存の`evaluate_ai_monthly_recruitment`のCountryId昇順処理を変更していないため、P28-004-PRE-001・P23-009の既存決定性テストがそのまま保護として機能する)。

**B1を本番実装した場合の既存テストへの影響**: 本監査では一時パッチのみで検証し、既存テストは実行していない(§9で個別実行したのは無変更のままの現状)。実装時に変化が予想されるのは、開戦初期のタイミングに依存する自然再現テスト(現状P28-002・P28-003のようなE2E)であり、これらは既にP28-005で「州を問わない機能上の契約」と「自然再現の参考値」を分離する設計パターンが確立されている(`verification_logs/phase-28/p28-005/`§7)。B1採用時も同じ分離を踏襲することを推奨する。

## 9. 実行コマンドと正確なテスト・clippy結果

Cargoはすべて逐次実行した(並列起動なし)。

- 開始時: `git rev-parse HEAD` → `3d76c74304daa7236c88be7285aaf774b2145df7`、`git status --short` → 空
- 開始時実測: `cargo test --all-targets -j 2` → 2752 passed / 0 failed / 1 ignored(85バイナリ)。`cargo clippy --all-targets --all-features -- -D warnings` → 警告0
- 一時診断(`tests/p28_004_pre_002_temp_diagnostic.rs`、実カレンダー): B0・B1を90日・36・120・180・240・360・600か月、B2を診断用に360か月。B1の600か月を独立プロセスで再実行し、FIS行が完全一致(決定的)
- 一時変更の除去後、個別実行(いずれも0 failed):

  | テスト | 件数 |
  |---|---|
  | `p23_009_ai_recruitment_e2e_test` | 22 |
  | `p26_008_a_supply_aware_recruitment_gate_e2e_test` | 30 |
  | `p26_008_b_supply_aware_offensive_gate_e2e_test` | 19 |
  | `p26_008_c_supply_chain_construction_priority_e2e_test` | 18 |
  | `p26_004_daily_supply_consumption_allocation_e2e_test` | 19 |
  | `p26_003_rev_002_supply_production_units_e2e_test` | 56 |
  | `p26_003_rev_003_economic_power_capacity_e2e_test` | 21 |
  | `p26_003_military_supplies_production_e2e_test` | 49 |
  | `p27_001_partial_equipment_recruitment_completion_e2e_test` | 9 |
  | `p27_002_a_ai_construction_fiscal_reserve_e2e_test` | 17 |
  | `p27_002_b_ai_tax_rate_adjustment_e2e_test` | 18 |
  | `p27_002_c_residual_fiscal_self_stabilization_e2e_test` | 20 |
  | `p28_002_wartime_military_factory_construction_e2e_test` | 3 |
  | `p28_003_disconnected_friendly_frontline_delivery_e2e_test` | 2 |
  | `p28_005_ai_railway_priority_e2e_test` | 3 |
  | `land_war_combat_peace_test`(無変更) | 4 |
  | `--lib military::` | 508 |
  | `--lib country::country_ai` | 76 |
  | `--lib save::` | 305 |

- 最終`cargo test --all-targets -j 2` → **2752 passed / 0 failed / 1 ignored**(85バイナリ、exit 0、開始時基準と完全一致——恒久変更なしを裏付ける)。1 ignoredは既存の`profile_p21_021_elimination_scaling`。再生成された既知の非決定的PNG9件は個別に復元
- 最終`cargo clippy --all-targets --all-features -- -D warnings` → **警告0**(exit 0)

## 10. 一時変更の除去・保護事項

- 一時パッチ: `src/military/ai.rs`(候補B1/B2、環境変数`P28RECRUIT`で有効化)。差分をスクラッチに保存してから逆適用し、`git hash-object`がHEADのblobと一致することを確認した。
- 一時診断テスト: `tests/p28_004_pre_002_temp_diagnostic.rs`。スクラッチへ控えを取ってから削除した。
- 本番Rustコード・RONデータ・既存テスト・既存報告書: 変更なし。恒久テストの追加なし。
- `saves/savegame_v1.ron`: 読み書き・復元・ステージなし。
- `tests/land_war_combat_peace_test.rs`: 変更・整形なし。ワークスペース全体のformatなし。
- 既知の非決定的PNG: 再生成されたものは`git status --short`で確認して1件ずつ個別に復元した。
- P27-005の手動プレイ確認: **未実施のまま引き継ぐ**(本タスクの自動診断をもって手動確認済みとは記述しない)。

## 11. Critical/High/Medium/Low

```text
Critical: 0
High: 0
Medium: 3
Low: 3
```

- **Medium M-2(継続、性質が変化)**: 募兵ゲートが生産量を見ない。Railway実装後は入力資源枯渇型の崩壊は観測されなくなったが、平時の過剰募兵による恒久的な補給不足(war9型)は残る。
- **Medium M-4(新規)**: B1のような「募兵後需要≤実月産」の硬い下限を交戦中に課すと、既存の戦力評価式(供給を見ない、L-2と同系統)により、少数精鋭の防御国が過小評価され、単独駐留による新しい膠着(war5型)を生む。B1を採用する場合はこの副作用を許容するか、戦力評価式の見直しを合わせて検討する必要がある。
- **Medium M-1(継続)**: 非戦闘の終結経路の欠如(war4型)。
- **Low L-2(継続)**: 敵国の全Divisionで数える戦力比(M-4の根本原因と同系統)。
- **Low L-5・L-6(継続)**: P28-004-PRE-001のとおり。

## 12. 次の原子的タスク

ユーザーが§7の質問1・2に回答した後の分岐:

- **質問1で「現状維持」を選んだ場合**: 募兵ゲートの変更は行わない。次の原子的タスクは、既存の戦力評価式(`evaluate_powers`、L-2)が補給充足率を考慮しない問題を単独で監査すること(M-4・L-2の根本原因)。これは募兵条件とは独立に着手できる。
- **質問1で「B1採用」を選んだ場合**: `src/military/ai.rs`へB1を本番実装し、境界(§5)を最低限の恒久テストで固定する。あわせて質問2で「war9型を今後扱う」と回答された場合は、平時専用の緩やかな上限案を別の原子的タスクとして設計監査する。

いずれの分岐でも、P27-005の手動プレイ確認は未実施のまま引き継ぐ。

## 13. 最終`git status --short`

コミット前: `?? verification_logs/phase-28/p28-004-pre-002/`(本報告書のみ)。コミット後は空。
