# 戦闘バグB「敵の最後の州を落とせず戦争に勝てない」— 設計案

**作成日**: 2026-08-12
**ステータス**: 設計案のみ・未実装。ユーザーの方向決定待ち。
**関連する既存メモリ**: `project_phase21_status.md` 2026-08-11「enemy divisions don't disappear」修正
(RETREAT_MANPOWER_LOSS_RATIO導入の経緯)

---

## 1. 問題

ユーザーが実際にプレイしたログ(2026-08-12)で確認された事象:

```
[Battle] Started battle BattleId(1) in state StateId(5): DivisionId(0) vs DivisionId(2)
[Battle] Attacker Division DivisionId(0) repelled, returns to StateId(0) (manpower: 7838)
[Battle] Defender won. Territory unchanged.
[Battle] Started battle BattleId(2) in state StateId(5): DivisionId(3) vs DivisionId(2)
[Battle] Attacker Division DivisionId(3) repelled, returns to StateId(6) (manpower: 7841)
[Battle] Started battle BattleId(3) in state StateId(5): DivisionId(3) vs DivisionId(2)
[Battle] Attacker Division DivisionId(3) repelled, returns to StateId(6) (manpower: 6104)
[Battle] Started battle BattleId(4) in state StateId(5): DivisionId(3) vs DivisionId(2)
```

同じ守備側 `DivisionId(2)` が StateId(5) に居座り続け、プレイヤーが送り込んだ複数の攻撃師団
(DivisionId(0)、DivisionId(3))を個別に何度でも撃退している。守備側自身の消耗はログ上一度も
「撃破」に至らず、州は最後まで占領できない。

## 2. 根本原因(コード上で確認済み)

2つの独立した仕様が組み合わさって「守備側1個師団が無限に持ちこたえる」状態を生んでいる。

### 2-1. 戦闘は常に1師団 対 1師団、かつ州につき同時に1戦闘まで

`src/military/invasion.rs::process_division_arrival` (58-72行目):

```rust
if battle_registry.get_ongoing_battle_in_state(arrival_state).is_some() {
    // 戦闘中の地域へは進入できない → 待機
    ...
    return;
}
```

`find_enemy_division_in_state` (96-117行目) も敵師団を **最小IDの1体だけ** 選ぶ。
`Battle` 構造体 (`src/military/battle.rs:23-46`) 自体が
`attacker_division_id: DivisionId` / `defender_division_id: DivisionId` と単一IDしか持てない。

→ 結果: プレイヤーが何個師団を同じ州に向かわせても、既に戦闘中ならその他は
「待機」させられるだけで合流できない。守備側は常に1体のみを相手にすればよい。

### 2-2. 攻撃力(10) < 防御力(15)の非対称性により、ほぼ必ず攻撃側が先に組織率0になる

`src/military/combat_calc.rs::resolve_combat_day` のダメージ式は
「自分の実効値が高いほど相手に与えるダメージが増え、自分が受けるダメージは減る」ため、
同一部隊(Standard Infantry: atk10/def15)同士が戦うと、**守備側は常に攻撃側より少ない
ダメージ**を受ける。組織率(倒れる速さ)は manpower損失に比例するため、攻撃側が先に
`organization <= 0` に達して「撃退される」側になる。

### 2-3. 撤退税(RETREAT_MANPOWER_LOSS_RATIO)は「負けた側」だけが払う

`src/military/update.rs::resolve_finished_battles` (209-211行目)は
`organization <= 0` になった側を「その戦闘の敗者」と判定し、撤退時に
`max_manpower`の10%を追加で失わせる(2026-08-11に「無限撤退→回復ループ」防止のため導入)。
2-2の非対称性により **ほぼ常に攻撃側だけがこの税を払い**、守備側は通常のわずかな
manpower損失しか受けない。

### 2-4. 複合効果

守備側1個師団は、対等な攻撃側師団を相手にするたびに「小さな損害を受けつつ確実に勝つ」を
繰り返せる。攻撃側は5回前後の敗北で全滅する(2026-08-11の実測: 10000→8475→6834→4974→
1203→撃破)一方、守備側の消耗ペースはその何分の1かに留まる。**攻撃側が何個師団を
「順番に」送り込んでも、合流できない(2-1)ため、守備側を数の力で圧倒する手段が存在しない。**

これは細かいバランス調整の問題ではなく、「複数師団による共同攻撃」という仕組み自体が
まだ存在しないという構造的な欠落である。

---

## 3. 対応案

### 案1(推奨): 複数師団の同時参戦を許可する「合流型」戦闘への拡張

**概要**: 同じ州に向かった自軍の複数師団が、既存の戦闘に「合流」できるようにする。
`Battle`が単一IDではなく参加師団のリストを持ち、双方の実効攻撃力・防御力を
参加師団の合計(または加重平均)として毎日の`resolve_combat_day`を計算する。
manpower/組織率の損失は各参加師団に(貢献度に応じて、あるいは均等に)配分する。

**変更範囲**(見積り):
- `src/military/battle.rs`: `Battle.attacker_division_id`/`defender_division_id` →
  `attacker_division_ids: Vec<DivisionId>` / `defender_division_ids: Vec<DivisionId>`
- `src/military/invasion.rs`: `process_division_arrival`の「戦闘中は待機」分岐を、
  「自軍・同じ敵側の戦闘なら合流」分岐に変更。`find_enemy_division_in_state`も
  複数体を返せるように。
- `src/military/combat_calc.rs`: `resolve_combat_day`を単一Divisionではなく
  Division集合を受け取る形に変更(実効値の合算ロジックが新規に必要)。
- `src/military/update.rs`: `resolve_finished_battles`/`handle_attacker_victory`/
  `handle_defender_victory`を複数師団の勝敗・撤退処理に対応させる(現状最も複雑な部分)。
- `src/map/division_render.rs`: 戦闘中オーバーレイの描画が単一`combat_id`前提の箇所を確認・調整。
- 影響するテスト: `military/tests.rs`、`war/tests.rs`、`tests/land_war_combat_peace_test.rs`
  など、Battle構造・1v1前提のアサーションを持つテストの洗い出しが必要。

**規模感**: 中〜大。コアの戦闘データモデルを変える構造変更のため、P21-004Rのような
命名変更より本質的に大きい。HoI4的な「複数師団の共同攻撃」を実現する本筋の改修。

**利点**: 根本原因を直接解決する。数を集めれば必ず落とせるようになり、
「最後の州が落とせず戦争に勝てない」問題を完全に解消する。今後の前線・攻勢システム
(このPhase 21の最終目標)にも必須になる可能性が高い機能。

**リスク**: 実装量が大きく、既存の1v1前提のテスト・描画コードに広く手を入れる必要がある。

---

### 案2: 最小変更 — 守備側にも「籠城税」を課す(構造は1v1のまま)

**概要**: `Division`に「同一戦闘/同一州で連勝した回数」を記録するカウンタを追加し、
一定回数(例: 3〜5連勝)ごとに守備側にも撤退税相当のmanpower減少を与える。
「守り続けるほど徐々に消耗する」という簡易的な包囲戦(siege)表現。

**変更範囲**: `Division`構造体にフィールド追加、`resolve_finished_battles`の
勝利判定部分に連勝カウンタの加算・リセットとペナルティ適用を追加する程度。

**規模感**: 小。1〜2ファイル、数十行程度。

**利点**: 実装が速く、既存の1v1データモデル・テスト・描画を壊さない。
「守備側が無限には耐えられない」という結果だけは即座に得られる。

**欠点**: 「複数師団で攻める」という直感的なプレイ体験には応えられない
(結局1体ずつ順番に送り続けることになる)。根本原因(2-1)は残ったまま。
本筋の複数師団共同攻撃機能(案1)を後で作るときに、この籠城税ロジックが
不要になり書き直しになる可能性がある(先行投資が無駄になるリスク)。

---

### 案3: 州側に「攻城時間」概念を追加する

**概要**: `StateData`に「この州が何日間戦闘状態にあるか」を追跡するフィールドを追加し、
長期間の攻城(同一州での連続戦闘日数)に応じて守備側の実効防御力を徐々に下げる
(要塞が消耗していくイメージ)。案2と似ているが、対象が「師団」ではなく「州」。

**規模感**: 小〜中。`StateData`へのフィールド追加、`combat_calc.rs`の防御ボーナス計算に
攻城日数を反映する処理。

**利点**: 案2と同様に実装が速い。かつ「防御側の地の利」という既存の地形ボーナス概念
(`TERRAIN_BONUS_*`、現状は未使用)と自然に統合できる。

**欠点**: 案2と同じく、複数師団を同時に投入する体験は依然として提供しない。

---

## 4. 推奨

**短期的に「勝てない」を解消したいだけなら案2または案3**(小規模・低リスク)、
**このPhaseの本来の目的(前線・攻勢システム)まで見据えるなら案1が本筋**、というのが
このセッションでの見立て。案1は今後どのみち必要になる可能性が高い一方、実装規模が
大きいため、着手前にユーザーの optioned な判断(今すぐ本格実装するか、まず案2/3で
「勝てない」だけを塞いでおくか)を仰ぎたい。

## 5. ユーザーへの確認事項

1. 案1(本格的な複数師団共同攻撃)・案2(籠城税)・案3(攻城日数)・複合(例: 案2を
   先に小さく入れてから案1に着手)のどれで進めるか。
2. 案1を選ぶ場合、「合流できる上限師団数」(HoI4の"combat width"相当)を設けるか、
   無制限にするか。
3. 案1を選ぶ場合、既存の1v1前提テスト群の書き換えを本タスクのスコープに含めるか
   (含めない場合、テストが赤くなった時点で別途相談)。
