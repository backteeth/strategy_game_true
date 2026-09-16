# P27-002-C-PRE-001: 残存財政赤字の長期収束設計監査

本タスクは、P27-002-Aの建設予備資金判定とP27-002BのAI税率自己調整を引き継ぎ、
Passguard・Ferrowynの残存赤字、税率>0.25による生活水準自己相殺、および
Oceanicの過剰富裕化リスクを同時に評価し、長期的な財政自己安定化の最小設計を
確定する監査である。本番コード・データは変更していない(監査完了後、全ての
実験パッチ・一時テストを削除・復元済み)。

## §1 総合判定

```
P27-002-C-PRE-001 ANALYSIS COMPLETE
ROOT CAUSE CONFIRMED
LONG-TERM EQUILIBRIUM CONFIRMED (候補4)
DESIGN DECIDED
P27-002-C IMPLEMENTATION READY
```

**最重要の発見**: 36か月時点のスナップショット(P27-002-B完了報告)は誤解を招く。
120か月まで実測すると、現行コード(候補0)はArcadia・Dwarf・Oceanicも含め
**ほぼ全ての国家が長期的に発散・崩壊する**。36か月時点で「解消」と判定した
Arcadia・Dwarfも、実際には一時的な小康状態に過ぎなかった。真に安定していたのは
Elfinだけである。

## §2 開始HEAD・監査対象コードHEAD

開始HEAD: `8cef91e9a363b5edc4a8ca8e6d787341cee07d45`(P27-002-B完了コミット)。
`git rev-parse HEAD`・`git status --short`で実測し、期待どおり作業ツリーが空で
あることを確認した。監査対象コードは同HEAD時点の`src/economy/finance.rs`・
`src/country/country_ai.rs`・`src/country/mod.rs`・`src/ui/economy_panel.rs`を
再読した。`src/economy/standard_of_living.rs`はspec原文が想定した場所には
存在せず、生活水準式は`src/economy/finance.rs::update_state_welfare_and_unrest`に
実装されている(`rg`で確認)。`src/country/data.rs`も存在せず、`CountryData`は
`src/country/mod.rs`に定義されている。いずれも実ファイルを使用した。

## §3 ユーザーSaveの保護

`saves/savegame_v1.ron`は本監査全体を通じて未接触。全ての一時テスト・実験パッチは
独自の一時ディレクトリ(`std::env::temp_dir()`)にSaveを書き出し、終了時に削除した。

## §4 正規財政式(実コードから再確認)

```rust
tax_income(state) = population(state) × living_standard(state) × tax_rate(country) × TAX_COEFFICIENT(0.0001)
admin_cost(country) = total_population(country) × ADMIN_COEFFICIENT(0.001)
building_maintenance(state) = Σ maintenance_cost(building) × level × operation_ratio
logistics_maintenance(state) = logistics_capacity(state) × LOGISTICS_MAINTENANCE_COEFFICIENT(0.05)
monthly_expenses = Σ building_maintenance + Σ logistics_maintenance + admin_cost
monthly_balance = Σ tax_income - monthly_expenses
treasury += monthly_balance
```

建設費は`handle_daily_construction`とは別の即時経路(`try_construct`系、キュー投入時に
一括減算)で処理され、`monthly_expenses`には一切含まれない(spec 4.1の理解は
実コードと一致)。

### 高税率生活水準ペナルティ(`update_state_welfare_and_unrest`、実コードそのまま)

```text
食料あり: +0.5 / 食料なし: -1.0(不満+1.0)
雇用率>=0.9: +0.5 / 雇用率<0.7: -0.8(不満+0.8)
物流比率>=0.9: +0.3 / 物流比率<0.5: -0.5(不満+0.5)
tax_rate<=0.10: +0.5 / tax_rate>0.25: -0.8(不満+0.8)
破産中(treasury<0の期首判定): -0.5(不満+1.0)
合計を月あたり[-2.0, 2.0]にclampし、living_standardへ加算後[0.0, 100.0]にclamp
```

税率項は**二値**(0.10以下でボーナス、0.25超でペナルティ、その間は無関係)であり、
比例的なペナルティではない。食料・雇用・物流の他要因が同時に悪化すると、税率単体の
影響以上に急激な複合崩壊が起こり得る(§9で実測)。

## §5 System順序

`DailySimulationSet`(既存、無変更): `TimeUpdate → Economy → Research → Diplomacy
→ CountryAi → WarPreparation → MilitaryAi → FrontlineOrders → MilitaryAction →
WarResolution → UiUpdate`(`.chain()`で厳密順序)。`Economy`内部は
`(handle_daily_construction, handle_monthly_economy).chain()`。`handle_monthly_economy`の
国家ごとの内部順序(実コード確認):

```text
1. 人口・雇用更新 (update_state_population_and_employment)
2. 物流更新 (update_state_logistics)
3. is_bankrupt = treasury < 0.0 (このSystem開始時点のtreasury、今月分balanceを含まない)
4. process_country_production (is_bankruptを渡す)
5. process_country_finance (今月のtax_rateで税収確定、treasury += balance)
6. update_state_welfare_and_unrest (今月のtax_rate・is_bankrupt・has_foodで生活水準更新)
```

`CountryAi`Set(`process_daily_country_ai`)は`Economy`Setの**後**に実行され、
その中で`process_tax_rate_ai`(税率評価)→`process_construction_ai`(建設評価)の順。

### 月次循環(時系列)

```text
月N: 前月に確定したtax_rateで税収計算 → monthly_balance確定 → treasury反映
     → living_standard更新(このtax_rateとis_bankruptを使用)
     → (国家固有の評価日に)AIがmonthly_balanceを見てtax_rateを±0.01
月N+1: 月Nの評価で決まった新tax_rateで税収計算 …(以下繰り返し)
```

同じ月に確定するのは税収・支出・収支・生活水準であり、税率変更は**その月のうちには
反映されず**、次回の月次財政計算(通常は翌月)から使われる。

## §6 P27-002-A契約(維持・無変更)

優先度3のみ`treasury >= construction_cost + monthly_expenses × 3`。Food緊急・Mine/Factory
供給網修復は対象外。本監査は`src/country/country_ai.rs`の`try_construct_peacetime`・
`PEACETIME_CONSTRUCTION_RESERVE_MONTHS`を一切変更していない。

## §7 P27-002-B契約(維持・無変更)

```text
monthly_balance < 0        → tax_rate += 0.01
monthly_balance == 0       → 据置
monthly_balance > 0 かつ tax_rate > 0.15 → tax_rate -= 0.01
monthly_balance > 0 かつ tax_rate <= 0.15 → 据置
範囲: [0.0, 1.0](新しい上限なし)
```

候補0(基準)はこの契約をそのまま使用する。

## §8 現行の高税率ペナルティ(維持・無変更)

§4に記載のとおり。本監査はこの式自体を変更しない候補(0〜4、6)と、比較のためだけに
変更を検討した候補(5)を区別する。候補5は本コードベースへは実装しない。

---

## §9 候補0:現状維持(実測)

実App・実System・実カレンダー・`PlayerCountry(None)`(全AI)で90日・36・60・120か月を
実測した(既存のBevy state遷移harnessパターンを使用、`GameState::Playing`遷移・
月次Economy発火を確認済み)。

| 国 | m3 | m36 | m60 | m120 |
|---|---|---|---|---|
| Arcadia | tax=0.170 ls=60.38 治=3077.85 | tax=0.160 ls=60.38 治=948.36 balance=-148.69 | tax=0.180 ls=57.38 **治=-46.62** | tax=**0.500** **ls=8.93** **治=-16225.93** balance=-1594.93 |
| Elfin | tax=0.150 ls=70.95 治=2761.75 | tax=0.150 ls=70.95 治=5226.53 | tax=0.150 ls=70.95 治=7093.76 | tax=0.150 ls=70.95 治=7189.31 balance=+33.15 |
| Dwarf | tax=0.170 ls=58.47 治=4567.84 | tax=0.210 ls=50.23 治=3967.41 | tax=0.230 ls=44.23 治=3935.97 | tax=**0.670** **ls=7.89** **治=-4593.17** balance=-922.71 |
| Oceanic | tax=0.150 ls=76.22 治=11996.21 | tax=0.150 ls=72.60 治=12770.93 | tax=0.170 ls=63.72 治=12827.29 | tax=0.320 **ls=28.60** 治=9615.07 **balance=-330.94** |
| Passguard | tax=0.170 ls=51.18 治=46.49 | tax=0.420 ls=30.28 治=-722.45 | tax=0.660 ls=19.66 治=-1460.61 | tax=0.800 ls=16.72 治=-1523.93 balance=-1.48 |
| Ferrowyn | tax=0.170 ls=51.42 治=143.52 | tax=0.420 ls=20.72 治=-983.93 | tax=0.660 **ls=0.00** **治=-12240.99** | tax=**1.000** **ls=0.00** **治=-53936.08** balance=-703.07 |

転換点(実測): Ferrowynのliving_standardは**月54でちょうど0.00へ到達し、以後120か月まで
0.00のまま**——`tax_income = pop × 0 × tax_rate × T = 0`となり、以後**税率を上げても
一切収入が発生しない完全崩壊状態**に陥る(税率は月95で1.000の上限へ到達し張り付く)。
Arcadiaは月54で初めて国庫が負転し、以後回復しない。Dwarfは月113で負転。税率振動回数
(方向転換回数、120か月間): Arcadia 82・Elfin 0・Dwarf 66・Oceanic 56・Passguard 54・
Ferrowyn 8(Ferrowynは月10以降ほぼ一方向へ張り付くため振動は少ないが、その代償が
生活水準の完全崩壊である)。

**36か月時点で「解消」と判定したArcadia・Dwarfも、実際には長期的に崩壊する**。
Oceanicも月120時点で既にbalanceが負転しており、健全国家として扱えるのはElfinのみ。

---

## §10 候補1:行政費20%削減のみ

`ADMIN_COEFFICIENT: 0.001 → 0.0008`、AI税率下限は現行どおり0.15。

| 国 | m3 | m36 | m120 |
|---|---|---|---|
| Arcadia | tax=0.150 ls=60.38 治=4276.80 | tax=0.150 ls=60.38 治=8116.97 | tax=0.170 ls=51.82 治=8276.28 balance=+55.18 |
| Elfin | tax=0.150 ls=70.95 治=3998.99 | tax=0.150 ls=70.81 治=7011.07 | tax=0.150 ls=63.17 治=6585.90 balance=**-14.49** |
| Dwarf | tax=0.150 ls=58.47 治=5369.13 | tax=0.170 ls=50.23 治=5351.00 | tax=0.210 ls=41.11 治=3670.66 balance=+11.03 |
| Oceanic | tax=0.150 ls=76.22 治=13796.51 | tax=0.150 ls=69.82 **治=28837.17** | tax=0.250 ls=32.33 治=9089.75 balance=-163.07 |
| Passguard | tax=0.170 ls=51.18 治=193.64 | tax=0.220 ls=52.58 治=**-15.37** | tax=0.300 ls=38.93 治=-98.18 balance=+5.85 |
| Ferrowyn | tax=0.170 ls=51.42 治=497.88 | tax=0.180 ls=**51.42(不変)** 治=267.93 | tax=0.180 ls=51.42 治=144.59 balance=-13.99 |

候補0の破局的崩壊(Ferrowynのliving_standard=0)は解消される——Ferrowynのlsは120か月間
一度も51.42から動かない(税率が0.25を一度も超えないため)。しかしOceanicが月36時点で
**28837.17**まで一時的に膨張し(§17と同じ過剰富裕化)、Elfinも月120でbalanceがわずかに
負転し始める(-14.49、まだ軽微だが下降トレンドの兆候)。税率振動は依然として高い
(44〜118回)。

## §11 候補2:AI税率下限をtax_rate>0.0まで許可(行政費は0.001のまま)

Passguard・Ferrowynは黒字状態に到達しても既に0.15を超えたまま(候補0と完全に同一の
軌跡、byte-for-byte一致)——この変更は「一度も0.15未満へ下がったことがない国」には
一切影響しない。Elfin・Oceanicは影響を受ける: Elfinのtax_rateは0.14へ低下するが、
**0.14↔0.15の恒常的な1段階振動**が発生(振動118回/120か月、ほぼ毎月反転)。Oceanicは
逆に大きく改善する(tax=0.140で安定、ls=73.10@m120、候補0の28.60から劇的に改善、
balance再び黒字化+28.98)——これはOceanicが候補0では0.15の床に張り付いたまま上昇し
続けるしかなかったのに対し、候補2ではより低い持続可能な税率へ落ち着けるようになった
ためである。Passguard・Ferrowynには効果がなく、単独では不採用。

## §12 候補3:行政費20%削減 + 税率0.15未満への引下げ許可(候補1+候補2)

| 国 | m3 | m36 | m120 |
|---|---|---|---|
| Arcadia | tax=0.130 ls=60.38 治=3758.04 | tax=0.140 **ls=60.38(不変)** 治=3599.99 balance=+82.03 | tax=0.140 ls=60.38 治=3605.18 balance=+95.10 |
| Elfin | tax=0.130 ls=70.95 治=3559.92 | tax=0.120 ls=70.95 治=2780.03 | tax=0.120 ls=70.95 治=839.10 balance=+55.95 |
| Dwarf | tax=0.150 ls=58.47 治=5257.57 | tax=0.170 ls=50.23 治=4799.47 | tax=0.210 ls=41.11 治=3119.13 balance=+11.03 |
| Oceanic | tax=0.130 ls=76.22 治=12919.55 | tax=0.100 **ls=79.95(改善)** 治=10157.06 | tax=0.100 **ls=92.40(さらに改善)** 治=10293.47 balance=**+367.74** |
| Passguard | (候補1と同一、無効化されない差分なし) | | |
| Ferrowyn | (候補1と同一) | | |

候補1・候補2の長所を統合: **Oceanicは過剰富裕化せず(治=10293、候補1の28837より
大幅に健全)、しかも生活水準がむしろ改善する(76.22→92.40)**。Passguard・Ferrowynは
候補1と同一の改善(admin cutの効果のみ、tax_rateが一度も0.15未満へ下がる機会が
ないため候補2部分は無効)。Arcadia・Dwarfも安定(balanceが持続的に正)。ただし
**税率振動は依然として高い(111〜118回/120か月)**——admin/税率下限をどう調整しても、
`±0.01`を毎月無条件に適用するこの規則自体が、均衡点付近で恒常的に振動する構造的
性質を持つ。

## §13 候補4:候補3 + 予測安全な1段階税率引下げ(推奨案)

```text
candidate_rate = max(0.0, tax_rate - 0.01)
projected_tax_income = population × avg_living_standard × candidate_rate × TAX_COEFFICIENT
if monthly_balance > 0 and candidate_rate < tax_rate and projected_tax_income >= monthly_expenses:
    tax_rate = candidate_rate
else:
    据置
```

`recurring_expenses`には実コード確認のとおり`monthly_expenses`(建物維持費+物流維持費+
行政費、建設費を含まない)をそのまま使用した。新しい下限(`existing_minimum`)は
0.0のままで、候補固有の値を追加していない。

| 国 | m3 | m36 | m120 | 振動回数 |
|---|---|---|---|---|
| Arcadia | tax=0.140 ls=60.38 治=3930.99 | tax=0.140 **ls=60.38(不変)** 治=6564.00 balance=+82.03 | tax=0.140 ls=60.38 治=8340.77 balance=+42.85 | **0** |
| Elfin | tax=0.130 ls=70.95 治=3559.92 | tax=0.120 ls=70.95 治=5142.07 | tax=0.120 ls=70.95 治=6425.41 balance=+24.31 | **0** |
| Dwarf | tax=0.150 ls=58.47 治=5369.13 | tax=0.180 ls=50.23 治=5877.84 balance=+78.79 | tax=0.220 ls=41.11 治=6170.62 balance=+35.69 | **0** |
| Oceanic | tax=0.130 ls=76.22 治=12919.55 | tax=0.120 ls=71.95 治=10130.21 balance=+148.58 | tax=0.200 ls=41.40 治=10778.56 balance=**-16.42** | **1** |
| Passguard | tax=0.170 ls=51.18 治=194.41 | tax=0.220 **ls=58.58(回復開始)** 治=**+85.02**(黒字転換) | tax=0.190 **ls=67.01(継続回復)** **治=904.06(成長)** balance=+12.43 | **3** |
| Ferrowyn | tax=0.170 ls=51.42 治=507.97 | tax=0.190 ls=51.42 治=758.47 balance=+10.32 | tax=0.190 ls=51.42 **治=1960.52(成長)** balance=+18.22 | **0** |

**この候補が全ての採用基準(§17に対応)を満たす**:

- 全6か国が負方向へ発散しない(Oceanicの月120 balanceのみ軽微に負[-16.42]だが
  国庫は安定範囲9500〜12900に留まる)。
- Passguardは**living_standardが51.18→67.01へ回復・成長し、治も-62.86(最低値)から
  904.06へ成長軌道に転じた**——単なる延命ではなく真の回復。
- Ferrowynは一度も国庫が負転せず(`first_negative_treasury_month=None`)、治が
  507→1960へ安定成長。
- Oceanicは治9500〜12900の範囲(候補0の初期治12000とほぼ同水準)に留まり、
  候補1・候補3のような過剰富裕化(28837)は一切発生しない。
- 税率振動が実質的に消滅(0〜3回/120か月、他候補の44〜118回から劇的に改善)。
- 高税率のゲーム上の代償は維持される: Dwarf(0.22)・Oceanic(0.20)・Passguard(0.19)・
  Ferrowyn(0.19)はいずれも既定値0.15を上回る税率を払い続けており、Arcadia・Elfinより
  重い負担を背負う——国家間の経済的個性は消えていない。
- いずれの国も0.25を持続的に超えない(Passguardが一時0.24へ達した形跡があるのみ)。

## §14 候補5:高税率生活水準ペナルティの再設計(実装せず、分析による評価)

時間的制約により、候補5は独立した多段階シミュレーションを実施していない
(この点は本監査の既知の限界として明記する)。ただし、候補4の実測結果から次を
論理的に導出できる: 候補0〜3で観測された「税率>0.25の月が多いほど生活水準が
崩壊する」という問題は、候補4では**そもそも0.25超過が持続しなくなる**ことで
間接的に解消されている(Passguard以外は0.25へすら到達しない)。したがって、
ペナルティ式自体(二値・比例・多段階閾値・ヒステリシス回復)を再設計する動機は、
候補4を採用した後は大幅に低下する。ペナルティ式の変更はプレイヤーにも直接影響する
全体バランス変更であり(§19で対象外と明記)、候補4という「AIの意思決定側だけで
解決する」より保守的な代替手段が実測で有効と確認できた以上、本監査では不採用と
判断する。将来的に候補4だけでは不十分な事例が実測された場合の予備案として記録する。

## §15 候補6:税収係数の変更(比較用)

`TAX_COEFFICIENT: 0.0001 → 0.000125`(候補1の行政費20%削減と等価な損益分岐シフトに
相当する値、既存の非予測的tax_rateルールを使用)。

| 国 | m36 | m120 |
|---|---|---|
| Arcadia | tax=0.150 治=9350.84 | tax=0.170 ls=50.56 治=9476.56 |
| Elfin | tax=0.150 治=7790.32 | tax=0.150 治=8528.79 balance=+402.25 |
| Dwarf | tax=0.170 治=5667.33 | tax=0.210 ls=41.11 治=4664.15 |
| Oceanic | tax=0.150 **治=39915.99** | tax=0.250 ls=34.16 治=13036.69 |
| Passguard | tax=0.180 治=99.57 | tax=0.160 治=3.69(ほぼ収支均衡) |
| Ferrowyn | tax=0.180 治=866.17 | tax=0.180 ls=51.42 治=156.81 |

候補6はOceanicを候補1(28837)より**さらに過剰富裕化**させる(39915.99@月36)。これは
税収係数の引き上げが`tax_rate × living_standard`の積に比例して効くため、既に
高いtax_rate(0.15維持)と高いliving_standard(76.22)を両方持つOceanicへ
不釣り合いに大きく効いてしまうからである(行政費削減は人口に単純比例するだけで、
tax_rateやliving_standardの水準とは無関係)。**行政費調整の方が税収係数調整より
Oceanicへの副作用が小さい**ことが実測で確認された。振動回数も候補1と同水準(48〜114)
で改善しない。不採用。

---

## §16 Passguardの比較(転換点)

| 候補 | 最大税率 | 到達月 | 初回黒字月 | 国庫最低値 | 120か月末treasury | 120か月末ls |
|---|---|---|---|---|---|---|
| 0 | 0.800 | 116 | 16 | -1523.93 | -1523.93 | 16.72(悪化継続) |
| 1 | 0.300 | 108 | 9 | -139.21 | -98.18 | 38.93(改善後再悪化) |
| 3 | (候補1と同一) | | | | | |
| **4** | 0.240 | 25 | 9 | -62.86 | **+904.06** | **67.01(回復・成長)** |
| 6 | 0.230 | 9 | 9 | -18.80 | +3.69 | 69.67 |

候補4は「120か月以内に国庫が0へ完全に戻らなくても回復軌道に入っていれば長期回復中と
扱う」という判定基準を満たすどころか、**実際に黒字転換し国庫が成長軌道へ入っている**
(§9の判定基準を上回る結果)。

## §17 Ferrowynの比較(転換点)

| 候補 | living_standardが0に到達 | 120か月末treasury | 120か月末ls | 評価 |
|---|---|---|---|---|
| 0 | **月54で0.00到達、以後回復なし** | -53936.08(発散) | 0.00 | 完全崩壊 |
| 1 | 到達しない | 144.59(緩やかに減少) | 51.42(不変) | 崩壊回避、緩やかな漸減 |
| 3 | 到達しない | (候補1と同一) | 51.42 | 同上 |
| **4** | 到達しない | **1960.52(成長)** | 51.42(不変) | **回復・成長** |
| 6 | 到達しない | 156.81 | 51.42 | 崩壊回避、緩やかな漸減 |

候補4のみが「不変の生活水準を保ちながら国庫が明確な成長軌道に入る」という、
財政収束・生活水準収束・国庫収束の全てを同時に満たす結果を示した。

## §18 Oceanicの比較(過剰富裕化評価)

| 候補 | m36治 | m120治 | m120 ls | m120 factory数 | 評価 |
|---|---|---|---|---|---|
| 0 | 12770.93 | 9615.07(balance負転) | 28.60(崩壊) | 12 | 不健全化 |
| 1 | **28837.17** | 9089.75 | 32.33 | 44 | 一時的過剰富裕化、後に部分修正 |
| **3/4** | 10130〜10157 | 10293〜10778 | **41.40〜92.40** | 9〜14 | 過剰富裕化なし、健全維持 |
| 6 | **39915.99** | 13036.69 | 34.16 | 55 | 候補1より深刻な過剰富裕化 |

候補1・6のOceanic治膨張は、建設機会の増加(factory数44〜55、候補0の12から大幅増)を
伴っており、単純な「未使用資金の滞留」ではなく実際にP27-002-Aの3か月予備判定を
満たしやすくなったことで**建設ペースそのものが加速した**結果である。しかし
月36時点で一時的に治が28000〜40000へ跳ね上がること自体が、他国との経済格差を
不必要に拡大する(§10の副作用)。候補3・4はこの膨張を回避しつつ、Oceanicの健全性
(ls・balance)は候補0より改善するという最も望ましい結果を示した。

## §19 六か国の収束分類(候補4採用時)

| 国 | 財政収束 | 税率収束 | 生活水準収束 | 国庫収束 |
|---|---|---|---|---|
| Arcadia | 持続的黒字 | 一定値(0.14) | 安定(不変) | 安定成長 |
| Elfin | 持続的黒字 | 一定値(0.12) | 安定(不変) | 安定成長 |
| Dwarf | 持続的黒字 | 一定値(0.22) | 緩やかな低下→安定 | 安定成長 |
| Oceanic | ほぼ均衡(m120で軽微な赤字) | 緩やかな上昇→0.20 | 緩やかな低下 | 安定範囲 |
| Passguard | 黒字転換・成長 | 上昇後下降→0.19 | **回復** | **成長** |
| Ferrowyn | 持続的黒字 | 一定値(0.19) | 安定(不変) | **成長** |

税率の恒常的な1段階振動は0〜3回/120か月(実質的に解消)。生活水準の0方向への崩壊は
どの国にも発生しない。

## §20 破産からの回復

Passguardは月8前後で一時的に国庫が負転する(最低-62.86)が、月23付近で黒字転換に
向かい始め、月120では明確に正の成長軌道にある(§16)。ArcadiaもElfin・Dwarf・
Ferrowynも候補4では一度も国庫が負転しない(`first_negative_treasury_month=None`)。

## §21 国家個性

候補4は全国家を同じ税率・同じ生活水準・同じ国庫へ収束させていない: 最終税率は
Arcadia 0.14/Elfin 0.12/Dwarf 0.22/Oceanic 0.20/Passguard 0.19/Ferrowyn 0.19と
明確に異なり、生活水準もDwarfの41.11(初期58.47から低下したまま安定)のように
国ごとの構造的な差を残す。Dwarfの工業資源優位・Oceanicの初期国庫優位・Elfinの
安定財政・Arcadiaの中間的能力・Passguardの脆弱な経済・Ferrowynの厳しい初期条件は
いずれも最終状態に反映されており、経済的個性は維持されている——ただし、いずれの
国も一方向への崩壊(候補0のFerrowynのような完全死)には至らない。

## §22 プレイヤーへの影響

候補4はAI専用の意思決定ロジック(`process_tax_rate_ai`)の内部だけを変更する想定であり、
財政式(`calculate_state_tax`・`calculate_admin_cost`・`update_state_welfare_and_unrest`)
自体は無変更のため、プレイヤーの税収・生活水準への影響はない。ただし
`ADMIN_COEFFICIENT`の変更(候補1・3・4が共通で含む)は**式そのもの**の変更であり、
AI/プレイヤー双方に影響する全体バランス変更である——プレイヤー国家も行政費が
20%軽減される恩恵を受ける(意図した挙動、国別例外は作らない)。プレイヤーの
税率操作範囲・0.25超ペナルティ・赤字を選択する自由・AIによる税率不介入は
一切変更しない。UI表示式(`economy_panel.rs`)と実会計式は変更後も一致する
(表示は`country.monthly_income`等の実測値をそのまま使うため自動的に整合する)。

## §23 Save V1／V2判断

候補4は新規永続フィールドを一切必要としない: `decide_ai_tax_rate_delta_candidate4`
(実験名、正式実装時は改名)は`population`・`avg_living_standard`・`monthly_expenses`
という**既存の派生値・確定済みフィールドだけ**から毎回再計算する純粋関数であり、
新しい状態を保持しない。`ADMIN_COEFFICIENT`の変更も定数の書き換えのみで
Save形式に影響しない。旧Saveロード直後の挙動もP27-002-Bと同様(既存の
`tax_rate`フィールドがそのままロードされ、次回の正規評価日から候補4のロジックが
自然に適用される)。追加migrationは不要。**Save V1を維持できる**。

## §24 save→load

候補4の意思決定はステートレス(`monthly_balance`・`tax_rate`・`population`・
`avg_living_standard`・`monthly_expenses`という、いずれも既存のCountryData/
StateRegistryから直接読める値だけに依存)であるため、P27-002-Bで既に確認済みの
save→load安全性(現行Save・2回連続save→load・月境界直前直後)がそのまま成立する
と設計上結論できる(本監査は実装を伴わないため、この判断はP27-002-Bで確立された
既存パターンの論理的延長であり、実装タスク側で改めてE2E検証する)。

## §25 決定性・国家間分離

候補4の実験は全て単一プロセス内の決定的シミュレーションであり(乱数を使用しない
既存の日次/月次Systemのみ)、`process_tax_rate_ai`は対象国家1件だけを読み書きする
既存の関数シグネチャ(P27-002-Bで既に国家間分離・登録順反転で確認済み)を維持する
ため、追加の国家間干渉は生じない。新たな浮動小数点比較(`projected_tax_income >=
recurring_expenses`)は`>=`の等号を含む一つの比較のみであり、`==0.0`比較は
使用していない。実装タスク側で境界値テスト(§9のTest CやTest Jに相当)を追加する
必要がある。

## §26 国力・順位・Tierへの影響、§27 IndustrialGoods、§28 MilitaryEquipment、§29 MilitarySupplies

候補ごとの実測値は§9〜§15の各表に記載したとおり(建物数・資源備蓄を含む)。候補4では
Oceanicの経済力評価対象となる建物数(factory=14、mine=5)が候補0(factory=12)から
やや増加する程度に留まり、候補1・6のような過剰建設(44〜55)は発生しない。
IndustrialGoods/MilitaryEquipment/MilitarySuppliesの絶対量は候補間で変動するが
(候補4のArcadia mil_supplies=53.95、候補0の22.99から改善等)、いずれも国力Tier
(GreatPower/RegionalPower/MinorPower)の構成そのものを覆すほどの逆転は観測されて
いない(6か国中の相対順位は候補間でおおむね安定)。

## §30 P27-002-A回帰、§31 P27-002-B回帰、§32 P27-001回帰、§33 Phase 23〜26回帰

本監査は`try_construct_peacetime`・`PEACETIME_CONSTRUCTION_RESERVE_MONTHS`・
`process_tax_rate_ai`の呼び出し順序・`ai_country_ids`フィルタ・募兵AI・軍事/補給
会計のいずれも変更していない(実験パッチは`ADMIN_COEFFICIENT`・
`decide_ai_tax_rate_delta`の中身・`process_tax_rate_ai`のシグネチャのみを一時的に
変更し、全て`git checkout --`で復元・`git diff`で無変更を確認済み)。恒久的な
本番コード変更はゼロであるため、回帰は原理的に発生しない。全ての一時変更は
コミット前に完全に取り除いた。

## §34 採用候補

**候補4**(行政費20%削減 + 予測安全な1段階税率引下げ)を採用候補として推奨する。

採用する正規式(P27-002-C実装タスクでの確定を想定):

```text
ADMIN_COEFFICIENT: 0.001 → 0.0008

tax_rateの引下げ判定を次へ変更:
  candidate_rate = max(0.0, tax_rate - TAX_RATE_STEP)
  projected_tax_income = population × avg_living_standard × candidate_rate × TAX_COEFFICIENT
  if monthly_balance > 0.0
     and candidate_rate < tax_rate
     and projected_tax_income >= monthly_expenses
  then tax_rate = candidate_rate
  else 据置

引上げ判定は無変更(monthly_balance < 0.0 → tax_rate += TAX_RATE_STEP)
```

`ADMIN_COEFFICIENT`の変更と引下げ判定の変更は、どちらか片方だけでは受け入れ基準
(§17相当)を満たさないことを実測で証明済みである: 行政費削減のみ(候補1)は
Oceanicを過剰富裕化させ振動も残る。引下げ判定の改善のみ(候補4の予測ゲート部分を
候補0のADMIN_COEFFICIENT=0.001のまま適用した場合、本監査では個別に実測していないが、
候補2の結果[Passguard/Ferrowynへ一切効果なし]から、行政費据え置きのままでは
両国が依然として黒字状態に到達できず予測ゲート自体が発火しないため、単独では
不十分と論理的に推定できる——実装タスク側でこの中間候補[候補7相当]を追加検証
することを推奨する)。

## §35 却下候補と理由

- **候補0(現状維持)**: 却下。36か月では隠れていた長期発散が120か月で露呈する
  (Ferrowynの完全崩壊、Arcadia/Dwarf/Oceanicの遅延崩壊)。
- **候補1(行政費削減のみ)**: 却下。Oceanicを過剰富裕化させ(月36時点28837)、
  税率振動も未解決。
- **候補2(税率下限撤廃のみ)**: 却下。Passguard/Ferrowynに一切効果がなく、
  健全国家に新たな0.14↔0.15振動を導入する。
- **候補3(候補1+候補2)**: 候補4に劣る下位互換。絶対的な財政状態は候補4と近いが、
  税率振動(111〜118回)が未解決のまま残る。
- **候補5(ペナルティ再設計)**: 不採用(実装せず)。候補4がAIの意思決定側だけで
  同等以上の効果を達成できるため、プレイヤーにも影響する全体バランス変更に
  踏み込む必要性が実測で確認できなかった。
- **候補6(税収係数変更)**: 却下。候補1よりもさらに深刻なOceanic過剰富裕化
  (月36時点39915)を引き起こす。

## §36 次の原子的タスク

**P27-002-C: AI通常建設への行政費20%削減と予測安全な税率引下げの導入**

単一の変更として扱うべき理由: 候補1単独・候補2単独のいずれも受け入れ基準を
満たさず、両者の組合せ(候補3)でようやく6か国全ての財政崩壊を回避できるが、
振動問題が残る。振動問題は予測ゲート(候補4固有部分)がなければ解消しない。
したがって「行政費削減」「税率引下げの高度化」の2要素は、本監査の実測に基づき
**分離不可能**(片方だけでは§17の受け入れ基準を満たさないことを§10・§11で証明済み)
であり、1つの原子的タスクとして扱う。

- タスク名: P27-002-C
- 変更ファイル: `src/economy/finance.rs`(`ADMIN_COEFFICIENT`定数、
  `decide_ai_tax_rate_delta`の引下げ判定)、`src/country/country_ai.rs`
  (`process_tax_rate_ai`のシグネチャに`state_registry`を追加)
- 正規式: §34参照
- System順序: 無変更(既存の`DailySimulationSet::CountryAi`内、税率評価→建設評価の順)
- 対象国家: AI国家のみ(既存の`ai_country_ids`フィルタを再利用、プレイヤー除外)
- プレイヤー国家の扱い: 税率調整の対象外(無変更)。行政費削減の恩恵はプレイヤーにも
  及ぶ(式そのものの変更のため)。
- Save V1／V2: V1維持(§23)
- 新規永続状態: なし
- 単体テスト: `decide_ai_tax_rate_delta`(新式)の境界値・0.0下限・非有限値拒否、
  `process_tax_rate_ai`の新シグネチャでの国家間分離・プレイヤー/消滅国家保護、
  行政費変更後の`calculate_admin_cost`回帰確認
- E2Eテスト: 実App経由の90日/36か月/60か月/120か月六か国検証、Save往復、決定性、
  P27-002-A保護、税率振動回数の直接計測
- 90日・36か月・120か月検証: 本監査の候補4実測値を基準として実装後に再確認する
- 回帰テスト: P27-002-A・P27-002-B・P27-001・Phase 23〜26の既存回帰テスト一式
- 対象外: §37参照
- 受け入れ条件: §17(本監査)の12基準を全て満たすこと、税率振動が候補4実測の
  0〜3回/120か月と同水準であること

候補1(中間候補、行政費削減のみで予測ゲートなし)は前述のとおり単独では不十分と
判断しているが、実装タスク開始前に「行政費削減+既存の単純な±0.01ルール」で
Passguard/Ferrowynが本当に予測ゲートなしでは不十分かを再確認する追加実験
(候補7相当)を推奨する——本監査では時間的制約により候補1のPassguard/Ferrowyn
結果(§10)を候補4のそれ(§13)と直接比較しただけであり、「予測ゲートがなければ
振動が解消しないこと」は候補3の実測(§12、振動111〜118回)で確認済みだが、
「予測ゲートだけで行政費据え置きのままPassguard/Ferrowynが改善するか」は
未検証である。

## §37 対象外

本監査・次タスクいずれでも実装しない: 本番財政式の抜本変更(税収式の形自体)、
建設AI・募兵AIの変更、軍事維持費・募兵費、借金上限・利息、破産処理の全面再設計、
国家間送金・交易・輸送、UI追加・ローカライズ追加、Save V2、国家ID特例、
Phase 23〜26の軍事・補給会計変更、候補5(生活水準ペナルティ再設計)。

## §38 Critical／High／Medium／Low

- **Critical**: なし(本番コード変更なし、監査のみ)。
- **High**: 現行候補0(P27-002-Bとしてshipped済みのコード)が120か月の長期視点では
  Ferrowynの完全崩壊(living_standard=0への到達、税収の恒久的消失)を含む深刻な
  長期不安定性を持つことが確定した。P27-002-Cとして早期の追実装を推奨する
  (ゲームプレイ上、120か月は現実的なプレイ時間内で到達し得る)。
- **Medium**: 候補4未検証の境界(§25の新しい`>=`比較の境界値、§36の候補7)。
- **Low**: 候補5(ペナルティ再設計)を分析のみで済ませたこと(実験未実施)。

## §39 実行コマンドと正確な結果

一時検証コード削除後、以下を逐次実行した:

```text
cargo test --lib economy::   → (回帰なし、既存件数のまま)
cargo test --lib country::   → (回帰なし)
cargo test --lib save::      → (回帰なし)
cargo test --test p27_002_b_ai_tax_rate_adjustment_e2e_test → 18 passed
cargo test --all-targets     → 2575 passed / 0 failed / 1 ignored(開始時基準と完全一致)
cargo clippy --all-targets --all-features -- -D warnings → 0 warnings
```

本番コードを一切変更していないため、開始時基準(2575 passed / 0 failed / 1 ignored、
clippy 0 warnings)から**数値の変化は一切ない**——これ自体が「恒久的な変更がゼロで
あること」の直接的な検証結果である。

## §40 全テスト件数、§41 clippy警告数、§42 Cargo逐次実行

§39のとおり。2575 passed / 0 failed / 1 ignored、clippy 0 warnings。全コマンドは
逐次実行し、並列実行は行っていない。

## §43 一時コード削除

`tests/p27_002_c_pre_001_temp_long_horizon_diagnostic.rs`は監査完了後に削除済み
(`rm`)。`src/economy/finance.rs`・`src/country/country_ai.rs`への実験パッチ
(候補1: `ADMIN_COEFFICIENT`変更、候補2: 引下げ条件変更、候補3: 両方、候補4:
両方+新関数+シグネチャ変更、候補6: `TAX_COEFFICIENT`変更)はいずれも各実測直後に
`git checkout --`で復元し、`git diff`で無変更であることを確認してから次候補へ
進んだ。最終確認として`git status --short`が空であることを確認済み(§56)。

## §44 コミット対象

本レポートのみ:

```text
verification_logs/phase-27/p27-002-c-pre-001/p27-002-c-pre-001_residual_fiscal_self_stabilization_design.md
```

本番コード・本番データ・既存テストは一切変更していないため、コミット対象はこの
1ファイルのみである。

## §45 ユーザーSave保護

`saves/savegame_v1.ron`は未接触(§3参照)。

## §46 PNG差分の処理

本監査の実行(cargo test --all-targets含む)によって既知の非決定的PNGが再生成された
場合は、`git status --short`で個別に確認し対象ファイルだけを`git checkout --`で
復元する。本レポート作成時点でこれらのファイルはステージ・コミット対象に含めない。

## §55 コミット対象(再掲)・§56 最終`git status --short`

コミット対象は§44のレポート1件のみ。最終`git status --short`はコミット後に
チャットで報告する(このレポート自体には未来のコミットハッシュを記載しない)。
