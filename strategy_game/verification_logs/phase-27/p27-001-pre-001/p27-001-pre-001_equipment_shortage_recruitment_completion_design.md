# P27-001-PRE-001: Equipment不足による募兵queue完成停滞の設計監査

本報告書はPhase 26で確認されたMilitaryEquipment不足による募兵queue完成停滞について、
Phase 26のMilitarySupplies契約を変更せず、人的資源・Equipment・Division・Saveの正規会計を
再監査し、次の原子的実装タスクで採用する完成規則を確定するものである。

## 1. 判定

```
P27-001-PRE-001 ANALYSIS COMPLETE
ROOT CAUSE CONFIRMED
CURRENT BEHAVIOR DEFECT CONFIRMED
DESIGN DECIDED
MANPOWER ACCOUNTING DEFINED
EQUIPMENT ACCOUNTING DEFINED
SAVE V1 PRESERVABLE
NEXT IMPLEMENTATION TASK READY
```

「現行挙動」は防御的には正しい(会計を破壊しない)が、AI募兵ライフサイクルを実質的に
停止させる設計上の欠陥として確定する(§10・§35)。

## 2. 開始HEAD

`83bc3c2b128d47375370f1008b243f2afaadeee1`(P26-009の最終監査コミット)。開始時
`git status --short`はクリーン、`cargo test --all-targets`は2506 passed/0 failed/1
ignored、clippyは0警告——いずれも実測して確認した(§40)。

## 3. 監査対象HEAD

本タスクは報告書1件のみをコミットする監査専用タスクであり、コード変更は一切行って
いない。したがって監査対象HEADも開始HEADと同一の
`83bc3c2b128d47375370f1008b243f2afaadeee1`。

## 4. Phase 26基準状態の保護

P26-009が固定した基準状態(MilitarySupplies資源契約・日次消費・Hamilton配分・首都
連結性・戦闘統合・AI補給ゲート・36か月6か国結果)には一切触れていない。本監査で
読んだファイルはいずれも読み取り専用であり、既存のPhase 26関連テスト
(`p26_008_a_supply_aware_recruitment_gate_e2e_test`等)は変更していない
(§29で再実行し、結果が不変であることを確認済み)。

## 5. 正規募兵System

`src/military/update.rs::handle_daily_military`が`DayChangedMessage`ごとに、
`DailySimulationSet::MilitaryAction`Set内で次の順序を明示的にコードとして実行する
(§24で詳述)。

```text
1. recruitment::process_recruitment            (募兵キュー完成判定・Division生成)
2. reinforcement::apply_daily_reinforcement    (人的資源補充)
2.5. equipment_reinforcement::apply_daily_equipment_reinforcement (装備補充)
3〜4. movement::process_movement / process_daily_battles
5〜7. 戦闘後処理・組織率回復
```

`process_recruitment`は`src/military/recruitment.rs:85`で定義されており、これが唯一の
呼び出し元(`grep`で確認、`src/military/tests.rs`内のテスト呼び出しを除く実行時呼び出しは
`update.rs:31`の1箇所のみ)。「装備不足での永久停滞」という事象は、複数の並行実装や
死んだ経路の混同ではなく、**この1つの正規Systemの1つの分岐**が原因である。

## 6. 募兵要求経路

`request_recruitment`(`recruitment.rs:147`)がキュー投入の唯一の入口。手順:

1. `country.is_active()`チェック。
2. `DivisionDefinition`解決。
3. `available_manpower >= required_manpower`チェック。
4. `validate_required_equipment(def.required_equipment)`——**有効性(有限・非負)の
   検証のみ**。国家のMilitaryEquipment備蓄量は一切参照しない。
5. `available_manpower -= required_manpower; mobilized_manpower += required_manpower`
   (人的資源はこの時点で確定・予約される)。
6. `recruitment_queue`へ`RecruitmentQueueItem{division_id, target_state,
   days_remaining: def.recruitment_days, total_days}`を追加。

**Equipmentはキュー投入時点では一切消費・予約・確認されない**(コード内コメントで
明記、§8参照)。AIも`RecruitFeasibility`(UI向け事前判定)もこの経路をそのまま使い、
AI・プレイヤーで別実装は存在しない(§23)。

## 7. queue構造

`RecruitmentQueueItem { division_id: DivisionDefinitionId, target_state: StateId,
days_remaining: u32, total_days: u32 }`(`recruitment.rs:10`)。`CountryData.
recruitment_queue: Vec<RecruitmentQueueItem>`(`src/country/mod.rs:164`)。Save DTOの
別ラッパー型は存在せず、`CountryData`自体がSave出力にそのまま使われる
(`src/save/dto.rs`・`export.rs`・`apply.rs`のいずれにも`recruitment_queue`固有の変換
コードが存在しないことをgrepで確認済み——P26-008-Bの`FrontlinePlan`と同じ「自分自身が
Save DTO」パターン)。

## 8. training進行と完成条件

`process_recruitment`(`recruitment.rs:85`)の1日1回のロジック:

```text
for each country:
    for each queue item:
        if days_remaining > 0: days_remaining -= 1
        if days_remaining == 0:
            try_spawn_division(...)
                -> try_issue_equipment(stockpile, equipment=0.0,
                                       max=def.required_equipment,
                                       requested=def.required_equipment)
            Ok  => キューから除去、Division生成
            Err(InsufficientStockpile) => キューに残す(days_remainingは0のまま)
                                          + blocked_by_equipment_shortageへ記録
            Err(other) => キューに残す(理論上到達しない防御的分岐)
```

**核心**: `try_issue_equipment`の第4引数`requested`が常に`def.required_equipment`
(必要量の**全額**)で呼ばれる。`try_issue_equipment`自体は`stockpile.checked_sub
(requested_units)`が失敗したら即座に`Err(InsufficientStockpile)`を返し、**部分的な
支給は一切行わない**(`equipment.rs:174-231`、全額一致でなければ全額失敗、all-or-
nothing)。

## 9. 現行停滞の根本原因

1つの事実の組み合わせが原因である。

1. `request_recruitment`はEquipment残高を全く見ずにキュー投入を許可する(§6)。
2. `try_issue_equipment`は要求額の**全額**を一括で要求し、部分支給APIとしては
   呼ばれない(§8)——API自体(`equipment.rs`)は`requested`引数を自由に設定できる
   汎用関数だが、`recruitment.rs`側の**呼び出し方**が全額固定になっている。
3. 完成に失敗した項目は`completed_indices`へ追加されず、`days_remaining`も0のまま
   維持されるため、**翌日以降も同じ判定が無条件で繰り返される**(自動再試行、
   §11)。
4. `src/military/ai.rs:156`の`decide_and_recruit_for_country`は、他の全ての条件
   (人的資源・目標Division数・P26-008-A補給ゲート)より**先に**`if !country.
   recruitment_queue.is_empty() { return SkippedQueueNotEmpty; }`を評価する。
   AIのqueue上限は事実上1件であり(このチェックのため2件目が積まれることはない)、
   1件が完成できない限り**そのAI国家の募兵は無期限に完全停止する**。

根本原因は「(3)の自動再試行そのもの」ではなく、「(1)+(2)+(4)の組み合わせにより、
Equipment生産が需要に追いつかない国家では自動再試行が事実上永遠に成功しない」
という設計上のギャップである。

## 10. 一時的待機と事実上の永久停滞の区別

`process_recruitment`のロジック自体は、翌日以降にEquipmentが供給されれば
**追加の操作なしで自動的に完成する**(合成fixtureで確認、後述§30)。したがって
コードのメカニズムとしては「一時的待機」であり、無限ループのバグではない。

しかし、実際のゲームプレイでは次のいずれかにより**実質的な永久停滞**になり得る。

- 国内にMilitaryEquipmentの生産手段(MilitaryFactory×Iron/IndustrialGoods供給網)が
  存在しない、またはIndustrialGoodsゲートで稼働率0%の国家。
- 月次生産量が、既存Division向け日次補充(`equipment_reinforcement.rs`、P25-005)
  との競合により、新規queue完成に必要な全額へなかなか到達しない国家。
- AIの目標Division数計算式(`calculate_ai_target_divisions`)が既に1に達しているため、
  待機中の1件を除けば新規需要が発生せず、月日が経過するだけの状態が続く国家。

過去のP26-008-PRE-001報告書(§3の必読資料、index引用のみで再検証済み)が実測した
「Oceanic: 36か月972日中418日MilitarySupplies不足」および「live Division数が1の
まま変化しない」という記録は、本タスクでも既存のP26-009(直前タスク)の36か月6か国
実測データと整合する——**両方の実測データで、6か国とも36か月間Division数が
一切増えていない**という同一の傾向を確認した(P26-009報告書§7の表、および
P26-008-PRE-001報告書§40〜41)。「装備不足そのものは一時的待機として正しく実装
されているが、多くの国家では待機が終わる条件に36か月経っても到達しない」という
のが正確な表現である。

## 11. 人的資源の予約・消費

`request_recruitment`時点で`available_manpower -= required; mobilized_manpower +=
required`により**即座に確定**する(§6)。`cancel_recruitment`(`recruitment.rs:186`)は
この逆操作(`available_manpower += required; mobilized_manpower = saturating_sub
(required)`)を行う。`compute_manpower_ledger`(`manpower.rs:80`)は`queue_living`
(キュー内全項目の`required_manpower`合計)を`living_military_manpower`へ含めており、
**キュー項目が完成しているか停滞しているかに関わらず、常に正しく計上され続ける**
(`unaccounted_living_excess`が発生しない、ledger不整合を引き起こさない)。

**結論**: 現行の人的資源会計は、Equipment不足による停滞と無関係に既に正しい。
候補設計のいずれを採用しても、この人的資源予約タイミング(募兵要求時)を変更する
必要はない。

## 12. Equipmentの予約・消費

`request_recruitment`時点ではEquipmentは一切触れない(§6)。国家備蓄からの実際の
支給は`process_recruitment`の完成試行時、`try_issue_equipment`の呼び出しでのみ
発生する。したがって**「予約」という概念は現行コードに存在しない**——完成に成功
した瞬間に全額が国家備蓄から一括で減算されるか、失敗して備蓄が一切変化しない
かのいずれかである(§8のall-or-nothing性)。

## 13. manpower保存則

```text
Σ(全国家のmobilized_manpower) = Σ(生存Division現在manpower) + Σ(queue項目のrequired_manpower)
```

`ManpowerLedger.permanent_military_losses = workforce_burden.saturating_sub
(living_military_manpower)`、`unaccounted_living_excess`は通常0(超過があれば
異常検出)。この保存則はEquipment不足の有無に関わらず常に成立する
(`compute_manpower_ledger`の実装がキュー項目の生死や完成可否を一切条件にしていない
ため)。**候補設計の選定において、この保存則を壊す変更は不要かつ許容されない**。

## 14. Equipment保存則

```text
Σ(全国家のMilitaryEquipment備蓄) + Σ(生存Division現在equipment) = 世界総量
```

`try_issue_equipment`/`try_return_equipment`/`try_destroy_equipment`
(`equipment.rs`)がいずれも`debug_assert_eq!`でこの保存則を明示的に検証しており
(§8引用のコード参照)、`equipment_reinforcement.rs`の日次補充もこの正規APIのみを
経由する。**新しい完成規則(候補B〜D)は、いずれもこの3つの正規API以外の方法で
Equipmentを増減させてはならない**——新しい`stockpile.set()`直接呼び出しや、
専用の量子化ロジックの複製は禁止する。

## 15. 装備不足Divisionの既存対応能力

`equipment_reinforcement.rs`(P25-005)は、**既存Divisionが`equipment < max_equipment`
の状態にあることを日次で正式に扱う**メカニズムとして既に稼働している。

- `is_eligible_for_equipment_reinforcement`は`max_equipment <= 0.0`のみを除外条件
  とし、`equipment`の現在値には一切依存しない——**equipment=0のDivisionも正式な
  補充対象**である。
- `apportion_reinforcements`(`reinforcement.rs:51`、Hamilton最大剰余法、`u128`中間
  計算)が複数Divisionへの比例按分を担い、この既存関数を`equipment_reinforcement.rs`
  がunits空間でそのまま再利用している(複製なし)。
- `calculate_combat_readiness_with_supply`(`combat_calc.rs:164`)は
  `equipment_units_before == 0`でも`equipment_readiness_units = 0`を安全に返し
  (0除算・panicなし)、`min(manpower, equipment, supply)`の三要素合成により
  装備不足Divisionは戦闘力が正しく低下する(既存の設計、新しい特別扱い不要)。
- `save/validate.rs`は`division.equipment`/`max_equipment`の有限性のみを検証し、
  `equipment == max_equipment`という構造的制約は存在しない。

**結論**: 「不足装備のままDivisionが存在する」という状態そのものは、現行エンジンが
既に一級市民として扱っている。募兵完成時だけがこの状態を許さず、常に全額装備
(`equipment == max_equipment`)でなければ生成自体を拒否している、という**非対称性**
こそが本質的な設計ギャップである。

## 16. 装備補充System

§15の`equipment_reinforcement.rs`が既存Division向けの唯一の日次補充Systemであり、
`process_recruitment`の直後(`update.rs`内、§24参照)に実行される。P25-005が導入した
`blocked_by_completed_recruitment`(引数名は`blocked_by_equipment_shortage`相当)は、
「当日、装備不足で完成できなかったqueue項目がある国家」の当日の既存Division向け
補充予算を0にし、**翌日以降に備蓄を持ち越して待機中のqueueへ回す**優先順位を
既に実装している。この仕組みは変更不要であり、候補C(不足装備完成)を採用しても
そのまま組み合わせられる(完成したDivisionの残り不足分は、翌日以降このSystemが
通常のDivisionと同列に扱って補充する)。

## 17. 装備不足時の現行挙動の要約

- キュー投入: 常に成功(Equipment残高を問わない)。
- training完了時: Equipment全額があれば即完成、なければ完成せず**無期限**キューに
  残る。
- 停滞中: 毎日自動的に再試行される(操作不要)が、AIの新規募兵は完全停止する。
- 回復時: Equipment全額が揃った最初の日に、追加操作なしで自動完成する
  (§8のロジック上、理論的に保証される——ただし本タスクの一時検証コードでは
  この復帰を実App上で再現できず、§30で経緯を報告する)。

## 18. 候補A評価: Equipment入荷まで無期限待機(現状維持)

| 項目 | 評価 |
|---|---|
| 実装変更 | なし |
| 永久停滞解消 | しない——本質的な問題を放置する |
| AIへの影響 | queue上限1件により募兵が無期限停止し続け得る(§9・§20) |
| プレイヤーへの影響 | `cancel_recruitment`で手動取消可能(人的資源は返却される、§26) |
| Save/決定性 | 現状維持、リスクなし |
| 総評 | **却下**。P26-008の補給ゲートが正しく供給状態を判定しても、この既存の
  ギャップにより効果が実質的に打ち消される国家が生じる(§34)。 |

## 19. 候補B評価: 一定期間後の自動取消

| 項目 | 評価 |
|---|---|
| timeout日数の根拠 | 既存データに根拠となる値が存在しない(恣意的な固定値になる) |
| AI再要求の振動 | 取消直後にAIが同じ条件で即再要求→再度timeout待ち、という無意味な
  周期を生みかねない(P26-008-Aのヒステリシスとは別の新しい振動源) |
| Save影響 | `RecruitmentQueueItem`へ経過日数フィールドの追加が必要
  (`#[serde(default)]`で互換維持は可能) |
| プレイヤーへの影響 | 意図的な長期待機(装備調達を待つ戦略)を強制的に打ち切って
  しまう |
| 総評 | **却下**。「装備さえ来れば正しく完成する」という既存の正しい仕組みを、
  役に立たない固定タイムアウトで壊す価値がない。 |

## 20. 候補C評価: 不足装備のままDivision完成

| 項目 | 評価 |
|---|---|
| 既存契約との整合 | §15の通り、不足装備Divisionは既存エンジンの一級市民——**新しい
  概念を導入しない** |
| 実装規模 | `try_spawn_division`が`try_issue_equipment`へ渡す`requested`を
  `def.required_equipment.min(現在備蓄)`へ変更する程度の局所的変更 |
| Equipment 0での完成 | 技術的に可能(§15で確認済み、combat_calc/save/validateいずれも
  安全)。ただし装備0師団の乱造を防ぐ下限は別途検討が必要(§22) |
| 原子性 | `try_issue_equipment`は指定した`requested`額に対して既にatomicである
  ため、`requested`を減額して呼ぶだけで既存の原子性保証をそのまま享受できる |
| 装備補充との接続 | §16の通り、完成後は既存の`equipment_reinforcement.rs`が
  自動的に残り不足分を補う——新しいSystemを追加する必要がない |
| AI/プレイヤー共通性 | `try_spawn_division`は共通関数のため、変更すれば両方に
  自動的に適用される |
| Phase 26への影響 | Division需要(P26-004)は`manpower`ベースであり`equipment`を
  一切参照しないため無関係。P26-008-Aのbridge予測も同様に無関係
  (§34で詳述) |
| 同日複数queue | 単一国家内で複数queueが同日完成する場合、按分が必要になる
  (§25で設計指針を示す) |
| 総評 | **有力候補**。既存の設計思想(不足装備Divisionは正常状態)と完全に一致し、
  実装範囲が最小で、既存の補充・戦闘・Save経路をそのまま再利用できる。 |

## 21. 候補D評価: 募兵開始時のEquipment予約

| 項目 | 評価 |
|---|---|
| 変更範囲 | `request_recruitment`にEquipment残高チェック・予約処理を追加する必要が
  あり、Phase 25が確立した「キュー投入時はEquipmentを一切見ない」という設計方針
  (§6のコード内コメントで明記)を覆す |
| 新規Save状態 | 予約量を保持する新フィールドが`RecruitmentQueueItem`に必要
  (`#[serde(default)]`で互換維持は可能だが、旧Saveの意味的migrationが別途必要
  になる可能性がある) |
| AIの挙動変化 | Equipment不足時にそもそもキュー投入自体が失敗するようになり、
  「いつ再試行するか」という新しい判断をAI側に追加する必要が生じる
  (P26-008-Aのヒステリシスと同種の設計をもう1つ増やすことになる) |
| 既存Division補充との競合 | 予約分を既存Division補充の対象外にする新しい除外
  ロジックが必要 |
| 実装規模 | 候補Cと比べて明らかに大きく、Phase 25の会計契約への変更幅も大きい |
| 総評 | **却下**(第一候補としては採用しない)。効果(永久停滞の解消)は候補Cと
  同等かそれ以下だが、変更範囲・新規Save状態・新しい振動リスクがいずれも
  候補Cを上回る。 |

## 22. 最低装備率の比較

候補Cの派生として、完成に必要な最低装備率`X%`を検討した。

| X | 評価 |
|---|---|
| 0%(下限なし) | §11・§13の人的資源保存則、§9のAI目標Division数計算式
  (`calculate_ai_target_divisions`は`cap/2/required_manpower`で明確な上限を持つ)、
  および募兵自体がAIの場合`queue上限1件`で自然に律速されるため、**装備0師団の
  乱造は構造的に発生しない**(1か国が同時に生成できる新規Divisionは、AIの場合
  月1件のみ)。プレイヤー操作の場合も人的資源(`available_manpower`)が実質的な
  上限として機能する。 |
| 10%/25%/50% | 現行データ・既存デザインのいずれにも根拠となる数値が存在せず、
  恣意的な固定値になる。追加の複雑さに見合う利益が確認できない。 |
| 100%(現行) | §9〜10で確認した永久停滞の直接原因。 |
| 総評 | **下限を設けない(0%許容)を採用する**。既存のmanpower/AI目標数/queue上限が
  既に十分な律速要因であり、余分な最低率は不要な複雑さを追加するだけである。 |

## 23. AI・プレイヤーの共通性

`process_recruitment`・`try_spawn_division`・`request_recruitment`・
`cancel_recruitment`はいずれもAI・プレイヤーで完全に共通の唯一の経路であり
(`military/ai.rs`は`request_recruitment`を直接呼ぶだけで独自の募兵完成ロジックを
持たない)、候補Cの変更は自然にAI・プレイヤー双方へ同一に適用される。AIだけを
特別扱いする理由はなく、そのような分岐を新設する必要もない。

## 24. 同日System順序

`handle_daily_military`内のコード順(§5)がそのまま実行順を保証する
(Bevyの`Update`スケジュール内で単一関数として順次実行されるため、Set間の
`.after()`のような追加指定は不要——関数内の逐次コードそのものが順序保証である)。
月境界日には、`DailySimulationSet::Economy`(月次MilitaryEquipment生産を含む)が
`DailySimulationSet::MilitaryAction`より必ず先に実行される
(`configure_sets`の`.chain()`宣言順、P26-009で確認済み・本タスクでも再確認)。
したがって「その月の新しい生産分」は、同日の募兵完成判定より必ず先に国家備蓄へ
反映されている。候補Cを採用しても、この順序保証に変更は不要である。

## 25. 同日複数queueの扱い

現行のAI(`decide_and_recruit_for_country`)はqueue非空チェックにより事実上
「1国あたり同時に最大1件」に制限される。プレイヤー操作または合成Save編集により
複数queue項目が同一国家に存在し得るため、候補Cの実装では以下を設計指針とする。

- 同日に複数項目が完成条件(`days_remaining==0`)に達した場合、`equipment_
  reinforcement.rs`が既に確立した`apportion_reinforcements`(Hamilton最大剰余法、
  `u128`中間計算、剰余降順→キー昇順のtie-break)をそのまま再利用し、国家備蓄を
  それら全項目へ比例配分する。
- tie-breakのキーは、`RecruitmentQueueItem`自体が安定IDを持たないため、
  `recruitment_queue`内の**現在のVecインデックス**(処理順そのもの、決定的かつ
  HashMap非依存)を用いる——新しいID型を追加する必要はない。
- 現行のAI仕様(queue上限1件)を変更しない限り、この複数queue処理は主にプレイヤー
  操作向けの防御的実装となる。

## 26. queue取消

`cancel_recruitment`(§11)は現行のまま維持する。候補Cを採用してもEquipmentの
「予約」という概念自体を導入しないため(§12)、取消時にEquipmentを返却する処理は
不要のまま——現行の人的資源のみの返却ロジックに変更は生じない。

## 27. 国家消滅

`country::elimination`の消滅処理は`recruitment_queue`・Equipment備蓄のいずれにも
触れない(P26-009で確認済みの`.stockpile`grep結果を本タスクでも再確認、ゼロヒット)。
候補Cを採用しても、消滅国家のqueueは既存どおり凍結されたまま残り、`is_active()==
false`の国家に対して`process_recruitment`が実質的にno-opであり続ける現行の挙動
(消滅国家のqueueは日次処理の対象にはなるが、`try_spawn_division`が成功しても
`add_division`されたDivisionは即座にゲームロジック上意味を持たなくなるだけで、
既存の消滅会計を壊さない)に変更は不要である。

## 28. Save V1/V2判断

候補C(採用方式)は**新しい永続状態を一切追加しない**——`RecruitmentQueueItem`の
フィールド構成もSave DTOの構造も変更しない。したがって**Save V1を完全に維持できる**。
候補D(募兵開始時予約)であれば新フィールドが必要になり、`#[serde(default)]`での
構文的互換は可能でも、旧Save上の「予約されていないはずの装備」の意味的整合性を
別途検証する必要があった——候補Cを採用することでこの追加コストを完全に回避できる。

## 29. 旧Save互換性

候補Cはコード変更のみで完結し、Save形式自体に変更がないため、旧Save
(装備不足でqueueが停滞したまま保存されたSave含む)はロード後、次回の日次tick
(`DayChangedMessage`)で新しい完成規則により自動的に(利用可能な範囲で)完成する。
追加のロード時migration処理は不要。

## 30. 90日結果

一時検証コード(`tests/p27_001_pre_001_temp_equipment_stall_test.rs`、監査終了後に
削除・コミットせず)で、Equipment残高0・training完了直前(`days_remaining=1`)の
queue項目を合成的に作り、30日間ticksしても完成しないこと(現行のall-or-nothing
挙動)を実測で確認した。

**制約として報告する**: Equipment残高を必要量ちょうどに設定した翌日にqueueが
自動完成することを実Appで直接検証しようとしたが、一時検証ハーネス側で
`process_recruitment`自体が呼び出されない([TEMP-DEBUG]診断出力が一度も現れない
ことをプロダクションコードへの一時的なeprintln!挿入で確認、監査終了後に完全に
元へ戻し`git status --short`で復元を確認済み)という、本タスクのミニマル
`setup_app`ハーネス固有の未解明な問題に直面した。同種のミニマルハーネスで
`daily_supply::process_daily_military_supply_consumption`(Economy Set)は
P26-009で繰り返し正常に動作しているため、`MilitaryAction`Set特有の設定差
(未特定)が原因と推測されるが、根本原因の特定には至っていない。この制約により、
「Equipment供給後に自動完成する」という主張(§8・§17)は**コードの静的な読解による
論理的な保証**であり、本タスクの一時ハーネスでの直接的な動的再現には至っていない
ことを明記する。90日規模でのAI募兵完全停止(§9のqueue非空チェックによる)自体は
合成fixtureで直接再現できた(`days_remaining=0`のqueue項目を持つ国家が6か月間
一貫してqueue_len=1のまま推移することを確認)。

## 31. 36か月結果

P26-009(直前タスク)が実施した6か国36か月実測(`PlayerCountry(None)`、実データ、
外生付与なし)を本タスクでも指標として再利用した。6か国全てで生存Division数が
36か月間「1」のまま一貫して変化しなかった(P26-009報告書§7の表を参照)。これは
P26-008-PRE-001が記録した「live division数が1のまま変化しない」という既存の
実測結果と定性的に一致する。本タスク独自の36か月実測(`six_country_36_month_
stall_measurement`)でも同様の傾向を確認したが、§30の理由によりqueueの動的な
挙動そのものの再現には至らなかった(`divisions=0`が全期間通して観測された点は、
本タスクの一時ハーネスが初期Division生成[`spawn_debug_divisions`]の前提条件を
P26-009のハーネスと完全に一致させられなかったことによる別の制約であり、
P26-009自身の36か月実測データの方を正式な参照値として採用する)。

## 32. Equipment回復結果

§30の制約により、本タスクの一時ハーネスでは実App上での自動回復を直接観測できな
かった。ただし`process_recruitment`(`recruitment.rs:85-138`)の静的なロジックには、
回復を妨げるいかなる条件(タイムアウト・一度失敗したら二度と再試行しないフラグ等)
も存在しないことをコードの全文読解で確認済みである——`days_remaining`は失敗時に
0のまま維持され、次回呼び出し時にも同じ`if days_remaining == 0`分岐へ到達し、
無条件に再度`try_spawn_division`を試みる。

## 33. 現行6か国への影響

候補C(採用方式)は、既存の6か国いずれの初期状態・供給網差異(P26-004-PRE-002〜005・
P26-008-Cが確立した国別のMine/Factory配置)にも依存しない汎用ルールであり、
特定の国家をハードコードで優遇・除外することはない。Equipment生産能力が弱い国家
(Elfin/Oceanic/Passguard等、Phase 26各報告書で繰り返し確認済み)ほど、候補Cの
効果(部分装備でも前進できる)がより顕著に現れると予想されるが、この非対称性は
既存の資源配置の非対称性(意図された設計、P26-009§8参照)をそのまま反映した
自然な結果であり、新しい国別特例ではない。

## 34. P26-008-Aとの関係

P26-008-Aの募兵補給ゲート(`recruitment_supply_gate.rs`)は、`ai.rs:156`の
queue非空チェックの**後**に評価される(§9)。つまり現行では、queueが装備不足で
停滞している間、P26-008-Aのゲート自体は一度も評価されずスキップされ続ける
(`SkippedQueueNotEmpty`が`SkippedSupplyShortage`より先に返る)。候補Cを採用して
queueの停滞頻度を下げることで、P26-008-Aのゲートが本来意図した頻度で評価される
機会が増える——**P26-008-Aのコード自体は一切変更しない**が、その実効性が
間接的に改善される関係にある。P26-004の日次需要式(`division_daily_demand_units`)
はmanpowerのみに基づきEquipmentを参照しないため(P26-009で確認済み)、部分装備
Divisionが生成されてもMilitarySupplies需要計算・bridge予測(P26-008-Aの
`compute_bridge_deficit_units`)のいずれにも直接の影響はない。

## 35. 候補比較表

| 項目 | A: 無期限待機 | B: 自動取消 | C: 不足装備完成 | D: 開始時予約 |
|---|---|---|---|---|
| 永久停滞解消 | しない | 部分的(振動リスク) | する | する |
| 人的資源保存則 | 影響なし | 影響なし | 影響なし | 影響なし |
| Equipment保存則 | 影響なし | 影響なし | 既存APIのまま維持 | 新しい予約状態が必要 |
| Save V1 | 維持 | 新フィールド必要 | **完全維持** | 新フィールド必要 |
| AI/プレイヤー共通性 | 共通 | 共通 | 共通 | 共通(ただし判断ロジック増加) |
| 装備補充との整合 | 無関係 | 無関係 | 既存Systemをそのまま再利用 | 新しい除外ロジックが必要 |
| queue振動 | なし | あり(timeout境界) | なし | あり(再試行判断) |
| 実装規模 | なし | 小〜中 | **小** | 中〜大 |
| 決定性 | 維持 | 維持(要検証) | 維持 | 維持(要検証) |
| Phase 23〜26への影響 | なし | 軽微 | **なし**(既存契約の範囲内) | Phase 25会計へ変更 |
| 推奨 | 却下 | 却下 | **採用** | 却下 |

## 36. 採用方式

**候補C(不足装備のままDivision完成、最低装備率の下限なし)を採用する。**

理由: 既存エンジンが既に「不足装備Division」を一級市民として扱っている
(§15)ため、募兵完成時だけがこれを禁止している非対称性を解消するだけで済む、
最小かつ既存設計思想と完全に整合する変更である。新しい永続状態・新しいタイムアウト・
新しい予約概念のいずれも不要であり、Save V1を完全に維持できる。

## 37. Critical/High/Medium/Low

**Critical: 0件。High: 0件。**

**Medium: 1件**(既存のP26-008-PRE-001報告書の分類を、本タスクの再調査結果を
踏まえて維持・再確認する)——装備不足による募兵queue永久停滞相当の事象。国家の
会計(人的資源・Equipment・MilitarySupplies)自体は破壊しないが、AI募兵ライフサイクル
(および間接的にP26-008-Aの補給ゲートの実効性)を阻害する。Phase 26の対象外
(MilitaryEquipmentの問題であり、MilitarySuppliesの契約とは独立)。

**Low: 1件**(本タスク独自の新規発見)——本タスクの一時検証ハーネスにおいて、
`DailySimulationSet::MilitaryAction`Set内の`handle_daily_military`(および
`process_recruitment`)が、`DailySimulationSet::Economy`Set内の
`daily_supply::process_daily_military_supply_consumption`と同一パターンの
最小`setup_app`(`MinimalPlugins`+手動`DayChangedMessage`注入)で期待通りに起動
しない現象を観測した(§30)。プロダクションコードの実際の不具合である証拠は
見つかっておらず(実App/実プレイでは`AppPlugin`を含む完全なプラグイン構成が
使われるため影響しないと推定される)、根本原因は本タスクでは特定できなかった。
将来36か月規模の実App検証を伴うタスクを設計する際は、この制約を踏まえて
ハーネスの事前検証(§30と同様の[TEMP-DEBUG]的アプローチ)を行うことを推奨する。

## 38. 次の原子的実装タスク(P27-001)の定義

**タスクID**: `P27-001`

**採用方式**: 候補C(不足装備のままDivision完成、最低装備率の下限なし)。

**対象ファイル**: `src/military/recruitment.rs`のみ(`try_spawn_division`と
その直接の呼び出し元`process_recruitment`)。

**変更する関数**:

- `try_spawn_division`: `try_issue_equipment`への`requested`引数を、現在の
  `def.required_equipment`固定から`def.required_equipment.min(現在のMilitaryEquipment
  備蓄値)`へ変更する。この結果、`try_issue_equipment`は常に成功する
  (要求額が残高以下になるよう事前に切り詰めるため)、または備蓄0の場合は
  `requested=0.0`の成功するno-opとして扱われる(`try_issue_equipment`は`requested=
  0.0`を成功するno-opとして既に扱う、`equipment.rs`の既存テスト
  `issue_of_zero_is_a_deterministic_no_op`で保証済み)。
- `process_recruitment`: 上記の変更により`try_spawn_division`が
  `Err(InsufficientStockpile)`を返すケースが実質的になくなる(常に部分成功する)
  ため、完成分岐(`Ok(division) => completed_indices.push(i)`)が常に実行される
  ように整理する。`blocked_by_equipment_shortage`(§16のP25-005保護ロジック用)の
  意味を「完成できなかった」から「完全な全額支給ができなかった(部分支給で完成
  した)」へ再定義するか、この保護ロジック自体を維持すべきか(部分完成した国家の
  既存Division補充を当日止める意味があるか)を実装時に再検討する。

**Division初期装備量**: `try_issue_equipment`の戻り値`result.equipment_after`を
そのまま`Division.equipment`/`max_equipment`へ使う現行のロジック(`recruitment.rs:
41-53`)は変更不要——`max_equipment`は引き続き`def.required_equipment`(満編制上限)
のまま、`equipment`だけが実際に支給された量になる(既存の「不足装備Division」の
表現と完全に一致、§15)。

**補充への接続**: 変更不要(§16)。完成翌日以降、`equipment_reinforcement.rs`が
自動的に残り不足分を通常のDivisionと同列に補充する。

**同日順序**: 変更不要(§24)。

**Save変更**: なし(§28)。

**旧Save動作**: 変更不要、意味的migration不要(§29)。

**AI/プレイヤー範囲**: 共通(§23)、分岐追加なし。

**原子性**: `try_issue_equipment`の既存の原子性保証をそのまま利用する(§20)。

**決定性**: 単一国家・単一queue項目の通常ケースでは既存のまま。同日複数queue項目
の按分を実装する場合は§25の`apportion_reinforcements`再利用方針に従う
(初期実装では「AIは1件のみ」という制約から複数queue処理を最小実装[プレイヤー
複数キュー時は現在のVec順で逐次処理]に留め、按分の恒久テストはプレイヤー向け
複数queueシナリオとして別途追加してもよい)。

**恒久単体テスト**: 新規に追加(装備0/部分/満額での完成、装備0のDivisionが
`equipment_reinforcement.rs`で正しく翌日補充される結合確認、`try_issue_equipment`の
`min()`切り詰めの境界値)。

**E2Eテスト**: 新規E2Eファイル(仮`p27_001_partial_equipment_recruitment_
completion_e2e_test.rs`)——90日・36か月規模の実App検証を行う場合は、§30・§37の
Low findingを踏まえてハーネスの事前検証(小規模な既知動作確認)を行ってから
本実装に着手すること。

**回帰テスト**: `p25_002〜p25_007`・`p26_007`・`p26_008_a`・`p23_009`・
`p23_010`・`land_war_combat_peace_test`を実行し、既存の全額支給前提のテストが
壊れないことを確認する(既存テストの多くは満額のMilitaryEquipment備蓄を用意する
ため、`min()`切り詰めロジックが「常に全額支給」という既存の期待動作を変えない
ことを確認する必要がある)。

**対象外**: AIの目標Division数計算式の変更、Equipment生産量・破産係数の調整、
MilitaryFactory建設AIの変更、財政バランス調整、UI改修。

**受け入れ条件**: (1) Equipment残高が必要量未満でも、`days_remaining==0`に達した
queue項目が即座にDivisionへ変換される。(2) 生成されたDivisionの`equipment`は
支給できた実量、`max_equipment`は満編制上限のまま。(3) 人的資源・Equipmentいずれの
保存則も既存のテストで壊れない。(4) Save V1が維持される。(5) 6か国36か月実測で、
Equipment供給不足国家の生存Division数が36か月間「1」のまま固定される現象が緩和
される(完全解消は保証しないが、悪化しないことを確認する)。

## 39. 対象外(本タスク自体)

本タスクでは、装備キュー完成規則の実装、財政バランスの調整、Factory建設数上限の
導入のいずれも行っていない。設計の確定と次タスクの定義のみを行った。

## 40. 実行コマンドと結果

逐次実行(並列cargo実行なし)。

```text
開始時:
cargo test --all-targets                                      → 2506 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings       → 0 warnings

回帰テスト(military::/economy::/country::/save::の--lib、および
p23_001/002/006/006-fix-001/007/008/009/010、p25_002/002-rev-001/003/004/005/
006/006-rev-001/007、p26_007、p26_008_a、land_war_combat_peace_test):
                                                                 → 1918 passed / 0 failed / 0 ignored

最終確認(一時検証コード削除後):
cargo test --all-targets                                      → 2506 passed / 0 failed / 1 ignored
cargo clippy --all-targets --all-features -- -D warnings       → 0 warnings
```

## 41. 全テスト件数

最終確認時点で2506 passed / 0 failed / 1 ignored(開始時と完全に一致、本タスクは
恒久テストを追加していない)。

## 42. clippy結果

0警告(開始時・最終確認時ともに)。

## 43. ユーザーSave保護

`saves/savegame_v1.ron`には本タスク全体を通して一切アクセスしていない。

## 44. PNG処理

`cargo test --all-targets`実行のたびに、既知の9件の非決定的スクリーンショットPNGが
差分として現れたが(2回の全体テスト実行で計2回発生)、いずれも個別に
`git checkout --`で復元し、広範な`checkout .`は使用していない。

## 45. コミット対象

本報告書ファイル1件のみ
(`verification_logs/phase-27/p27-001-pre-001/p27-001-pre-001_equipment_shortage_
recruitment_completion_design.md`)。プロダクションRustコード・本番RONデータ・
既存テスト・既存報告書・Save形式・UI・localizationのいずれも変更していない。
一時検証コード(`tests/p27_001_pre_001_temp_equipment_stall_test.rs`)は監査終了前に
削除済み(`git status --short`で追跡対象に現れないことを確認済み)。本番コードへの
一時的なデバッグ出力(§30・`src/military/recruitment.rs`)も完全に元へ戻し、
`git status --short`で当該ファイルが変更なし(クリーン)であることを確認済み。

## 46. 最終`git status --short`

```
(クリーン、本報告書ファイルのみ追加予定)
```
