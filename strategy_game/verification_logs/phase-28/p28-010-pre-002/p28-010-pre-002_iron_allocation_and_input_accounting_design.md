# P28-010-PRE-002：Iron入力配分と生産会計の設計監査

本書は設計監査である。本番コード・データ・既存テストは恒久変更していない。測定に使った一時パッチ(`src/economy/production.rs`、`src/economy/mod.rs`)と一時テスト(`tests/p28_010_pre002_temp_diag.rs`)は最終検証前に削除・復元した。

## 0. 基準状態と判定

| 項目 | 値 |
|---|---|
| 期待HEAD | `be1443ef0a5323f66804dd0ca5cb322291ba03bd` |
| 開始時の実HEAD | `be1443ef0a5323f66804dd0ca5cb322291ba03bd`(一致) |
| 開始時 `git status --short` | 空 |
| 開始時のテスト基準 | 2794 passed / 0 failed / 1 ignored、clippy警告0(**前回報告からの引用**。今回の実測値は §11) |

```text
P28-010-PRE-002 ANALYSIS COMPLETE
INPUT ACCOUNTING ROOT CAUSE CONFIRMED
CANDIDATE D EFFECT CONFIRMED（測定範囲内。採否は保留、§8）
DESIGN DECIDED（会計補正を先行。候補Dは本タスクでは不採用）
NEXT ATOMIC TASK READY（会計補正1件）
```

### 要旨

1. **会計欠陥は独立して存在する。** 原因は「入力需要量(Step 1)が稼働率係数 `t = min(労働力比, 物流比)` を掛けた値なのに、実消費(Step 3)は `base × level × min(t, sat)` で `t` を掛けない」ことである。`sat < t` の月、国家備蓄が足りなくなる。
2. **失敗すると産出だけが発生する。** `consume` は全額判定で、戻り値は無視されている。備蓄が要求量未満だと全エントリが失敗し、備蓄が減らないまま満額産出が毎月続く(Arcadia は Iron 備蓄 0.975 が53か月固定)。
3. **Dwarf は影響を受けない。** 240か月で失敗0件。Mine の当月産出が消費前に加算されるため。
4. **会計補正は「エントリ単位の原子的クランプ」(FIX1)で足りる。** 失敗が無い国(Dwarf・Passguard)の結果は1ビットも変わらない。
5. **候補D(MilitaryFactory優先)の効果は条件付き。** Iron が律速で IndustrialGoods が余っている国(Dwarf・Ferrowyn・Elfin)では軍需が増える。Passguard では財政と生活水準が悪化する。
6. **候補Aの「生活水準低下」の主因はFactoryの総量ではなく、小州1つの雇用率**である(§7)。

## 1. 参照した資料とコード

| 種別 | パス |
|---|---|
| 報告書 | `verification_logs/phase-28/p28-009/p28-009_phase_28_final_integration_audit.md` |
| 報告書 | `verification_logs/phase-28/p28-010-pre-001/p28-010-pre-001_iron_input_balance_and_military_production_audit.md` |
| 報告書 | `verification_logs/phase-26/p26-det-001-rev-001/`(生産処理順の固定) |
| コード | `src/economy/production.rs`(`process_country_production`、`calculate_operation_ratio`) |
| コード | `src/economy/resources.rs`(`CountryStockpile::consume` / `add`) |
| コード | `src/economy/mod.rs`(`handle_monthly_economy`:人口→物流→生産→財政→福祉の順) |
| コード | `src/economy/finance.rs`(維持費は `maintenance_cost × level × op`、福祉の各項) |
| コード | `src/population/update.rs`(雇用は `required_workforce × level`。稼働率に依存しない) |
| データ | `assets/data/buildings.ron`(Factory: Iron20/Coal20→IG30、MilitaryFactory: Iron30/IG15→ME20/MS100) |

### 1.1 実コードで確認した事実

- `CountryStockpile::consume(res, amount)` は `current >= amount` のときだけ減算して `true`、そうでなければ **何も引かず `false`**(部分消費なし)。
- `process_country_production` Step 3 は `stockpile.consume(res, actual_in);` の**戻り値を捨てる**。続く産出(`stockpile.add` / `add_military_equipment_produced` / MS の units 集計)と研究力出力は、consume の成否と無関係に実行される。
- Step 3 は `BuildingType::ALL` 順(Mine → Factory → MilitaryFactory …)の外側ループ、州は `StateId` 昇順の内側ループ。Mine の産出は同月の消費より先に備蓄へ加算されるが、Step 2 の `sat` は**月初備蓄**で計算される。
- 複数入力の建物は、入力ごとに1回ずつ `consume` する。1つ目が成功し2つ目が失敗すると、1つ目は引かれ、産出は満額になる。
- 金庫への他の出入り(貿易・イベント・建設費)は無い。Iron・Coal・IndustrialGoods は生産系統以外で動かない(`IndustrialGoods` を参照するのは `resources.rs` の定義と AI の建設判断だけで、福祉・財政・人口は参照しない)。

## 2. 会計問題の根本原因

Step 1(需要)と Step 3(消費)の式:

```text
Step 1: need_i   = base × level × t_i                 (t_i = min(労働力比, 物流比))
Step 2: sat      = clamp(月初備蓄 / Σ need_i, 0, 1)
Step 3: op_i     = min(t_i, sat, ...)                  (破産時 ×0.8)
        消費_i   = base × level × op_i                 ← t_i を掛けていない
```

`sat < t_i` のとき `消費_i = base × level × sat = need_i × sat / t_i`。`t_i = t` で揃っていれば **Σ消費 = 備蓄 / t**。`t < 1` なら備蓄を超える。

### 2.1 実測による確認(Arcadia 1800年 m019、Iron)

| 項目 | 値 |
|---|---|
| 国家備蓄 | 0.9750 |
| Factory(州1) | level1、労働力比 0.158、需要 3.1551 |
| MilitaryFactory(州12) | level1、労働力比 0.330、需要 9.9060 |
| 需要合計(`needAll`) | 13.0611 |
| `sat` = 0.9750 / 13.0611 | **0.0747** |
| 稼働率 `op` | min(t, sat) = **0.0747**(両方 sat 側) |
| 要求消費 | Factory 1.4931 + MilitaryFactory 2.2396 = **3.7327** |
| 備蓄に対する倍率 | **3.83倍** |

両エントリが**単独でも備蓄を超える**ため、2件とも失敗する。備蓄は 0.9750 のまま、満額産出が続く。

## 3. 再現条件(依頼 §3 の5分類)

全6か国・240か月、`P28_FIX=0`(現行会計)で、失敗した `consume` をすべて記録した(エントリ=1州×1建物×1入力資源)。

| 国 | 建物 | 入力 | 失敗エントリ数(240か月) | 最初の失敗 |
|---|---|---|---|---|
| Arcadia(c0) | Factory | Iron | 245 | m019 |
| Arcadia(c0) | MilitaryFactory | Iron | 134 | m019(MS無償産出は m002 から) |
| Arcadia(c0) | MilitaryFactory | IndustrialGoods | 2 | m002 |
| Elfin(c1) | MilitaryFactory | Iron | 1 | m149 |
| Dwarf(c2) | — | — | **0** | — |
| Oceanic(c3) | Factory | Coal | 70 | m026 |
| Oceanic(c3) | Factory | Iron | 22 | m026 |
| Oceanic(c3) | MagicAcademy | MagicCrystal | 316 | 未記録 |
| Passguard(c4) | — | — | **0** | — |
| Ferrowyn(c5) | MilitaryFactory | Iron | 2 | m204 |

失敗時の `要求量 / 備蓄` の分布(全 792 件): `<1.001` が **0件**、1.001〜1.1 が 33件、1.1〜2 が 269件、2〜10 が 180件、10以上(または備蓄0)が 310件。

| # | 依頼の分類 | 結果 |
|---|---|---|
| 1 | 稼働率算定時は足りるが引落し時に足りない | **確認(根本原因)。** §2 の `t` 不整合。`sat` は月初備蓄で `t` 込みの需要に対して計算するが、消費は `t` 抜き。失敗の大半は要求量が備蓄の1.1〜10倍。 |
| 2 | Factory と MilitaryFactory が同じ Iron を要求 | **確認。** Arcadia m019〜m071 は両者が同時に失敗する(§2.1)。MilitaryFactory のみが失敗する月もある。 |
| 3 | 複数州・複数エントリが同じ国家備蓄を要求 | **確認。** Oceanic m027 は州9と州23の Factory が同じ Coal(備蓄 1.3254)で両方失敗する。 |
| 4 | fixed-point の量子化・丸めが境界に作用 | **未観測。** 要求量/備蓄 < 1.001 の失敗は0件。入力の `consume` は f64 比較で、MS/ME の fixed-point は産出側だけ。稼働率が f32 であることによる丸め由来の失敗は、今回の実測範囲では発生していない。 |
| 5 | Iron 以外の入力が不足 | **確認。** Factory の Coal(Oceanic)、MilitaryFactory の IndustrialGoods(Arcadia m002/m015)、MagicAcademy の MagicCrystal(Oceanic)。 |

### 3.1 Dwarf が失敗しない理由

Mine の産出(約109/月)は同月の消費より先に備蓄へ加算される。Dwarf の定常状態は `月初備蓄 S0 ≈ t × 収入` で、消費 `S0 / t ≈ 収入` が備蓄(S0 + 収入)に収まる。そのため `consume` は常に成功する。稼働率 0.21 はこの式の結果でもある(P28-010-PRE-001 §5)。

### 3.2 部分払いと全額無償

| パターン | 例 | 内容 |
|---|---|---|
| 部分払い | Arcadia m002 | MilitaryFactory の Iron 12.62 は引かれ、IndustrialGoods 4.995(備蓄 3.95)は引かれない。MS 42.1 が満額産出 |
| 全額無償 | Arcadia m019〜m071 | Iron 備蓄 0.9750 が53か月固定。Factory・MilitaryFactory とも失敗し、毎月 MS 3.5〜7.5 と IG 2〜5 を無償産出 |

## 4. 保存式(月次・国家別・資源別)

`期末 = 期首 + 実際の産出 − 実際の引落し`(明示された他の移動なし)。

- **実際の引落し** = `consume` が `true` を返した量の合計。
- **要求された投入量** = `base × level × op` の合計(失敗分を含む)。

### 4.1 120か月の検証結果(`P28_FIX=0`)

| 国 | 保存式(実際の産出・引落しで) | 要求と実引落しの差(Iron、120か月) | 備考 |
|---|---|---|---|
| Arcadia | 残差 0.000 | 要求−引落し **190.8**(月0.01超の差: 57か月) | 失敗月(あらゆる資源): 104か月 |
| Elfin | 残差 0.000 | 0(120か月まで)。m149 に1件 | |
| Dwarf | 残差 0.000 | **0** | 失敗0件 |
| Oceanic | 残差 0.000 | 要求−引落し 16.4(12か月) | Coal・MagicCrystal の失敗は別(Iron 以外) |
| Passguard | 残差 0.000 | 0 | |
| Ferrowyn | 残差 0.000 | 0(120か月まで) | m204 に2件 |

Iron・Coal・IndustrialGoods のどれも、**実際の引落しを使えば全国・全月で残差 0.000**。P28-010-PRE-001 が Arcadia で観測した「帳簿が閉じない」現象は、保存式の破れではなく「要求量を引落しと見なしていた」ことに由来する。

P28-010-PRE-001 の「Arcadia 54か月・Oceanic 2か月」との対応: Arcadia の Iron が **0.9750 固定になる m019〜m071 は53か月**で、前回の54か月とほぼ一致する。ここに 0.01 以下の微小残差の月(m072〜)が加わって57か月になる。Oceanic の Iron の失敗は m026 以降の12か月で、前回の2か月(閾値による数え方の違い)とは一致しない。**この差は、数え方(閾値・対象資源)の違いと考えられるが、前回の数え方を再現して確かめてはいない。**

### 4.2 無償産出の量(120か月、`P28_FIX=0`)

「失敗したエントリの産出」の合計。部分払いの月(Iron は払い済みで IG だけ失敗)も含むため**上限値**である。

| 国 | MS(無償 / 総産出) | ME | IG | 備考 |
|---|---|---|---|---|
| Arcadia | **367 / 974(37.7%)** | 73.4 | 151 / 245(61.6%) | |
| Oceanic | 0 | 0 | 40 / 304(13.2%) | 軍需工場なし。MagicAcademy の研究力は未定量 |
| Elfin(240か月) | 56 / 9,375(0.6%) | — | — | |
| Ferrowyn(240か月) | 182 / 19,680(0.9%) | — | — | |
| Dwarf・Passguard | 0 | 0 | 0 | |

## 5. 会計補正の仕様と測定(共通条件)

比較のために次の補正を `process_country_production` に一時適用した。

| 名称 | 内容 |
|---|---|
| **FIX0** | 現行(補正なし) |
| **FIX1(採用案)** | エントリごとに `op_eff = min(op, 各入力の 現在備蓄 / (base×level))`。消費 = `base × level × op_eff`(備蓄を超えない)。産出・研究力・`building_operation_ratios` も `op_eff` で計算する。`consume` は必ず成功する |
| FIX2(不採用) | `op = t × sat`(Step 1 の定義に合わせて係数を掛ける)+ FIX1 のクランプ |

### 5.1 FIX1 は失敗が無い国の結果を変えない

| 条件 | 結果 |
|---|---|
| 失敗件数(全国・240か月) | **0** |
| Dwarf・Passguard | FIX0 と全チェックポイントで一致(区画・備蓄・MS・shortD) |
| 変化する国 | Arcadia、Oceanic、Elfin(m149)、Ferrowyn(m204) |

### 5.2 FIX1 の効果(政策を当該国のみに適用した分離実験、FIX0→FIX1)

| 国 | m120 | m240 |
|---|---|---|
| Arcadia | MS 974→652(**−33.0%**)、補給不足日数 6,049→6,431(+6.3%)、国庫 4,517→4,580 | 補給不足 7,453→7,835。部隊0 |
| Oceanic | IG 287→248、国庫 5,022→4,950 | 平均生活水準 72.0→70.3 |
| Elfin | 同一 | MS 9,375→9,344、補給不足 23,771→23,843 |
| Ferrowyn | 同一 | MS 19,680→19,332(−1.8%)、補給不足 4,614→4,936(+7.0%)、生活水準 68.0→70.4 |

- **補正の効果は「無償産出の除去」であり、補給の改善ではない。** Arcadia の補給不足日数は補正で増える。無償産出を「補給改善」と数えない(依頼 §6)。
- 世界全体の軌跡も変わる(全国同時適用で、戦争総数 m120 が 8→7)。

### 5.3 FIX2 を採用しない理由

FIX2 は失敗が無い Dwarf にも影響し、全体の挙動を変える(`P28_FIX=2, P28_ALLOC=0`、全国同時):

| 項目(Dwarf) | FIX0/FIX1 | FIX2 |
|---|---|---|
| m36 Division数 | 5 | 7 |
| m36 補給不足日数 | 27 | 82 |
| m36 Iron備蓄 | 1,218 | 672 |
| m120 補給不足日数 | 5,857 | 13,914 |
| m120 平均生活水準 | 65.7 | 53.7 |

会計の修復ではなく**経済挙動の変更**になるため、純粋な会計補正としては不適切と判断する。

## 6. 候補Dの比較

配分方式(いずれも FIX1 を共通条件として適用):

| 名称 | 内容 |
|---|---|
| **D0** | 現状。資源ごとの `sat` を全消費者で共通(需要比例の按分) |
| **D1** | MilitaryFactory に先に配分。次に他の建物が残りを按分 |
| **D2** | 需要比例按分 + スラック再配分(他資源が律速で使い切れない配分を戻す) |
| **D3** | D1 + 解放。MilitaryFactory は他の入力が律速の場合は使えない分を解放し、Factory に回す |

### 6.1 D2 は D0 と同一

D2 は FIX0・FIX1 のどちらの条件でも、**全6か国・全チェックポイントで D0 と完全に一致した**。現行の `sat` が既に「需要に応じた按分」であり、D2 に独立した効果は無い。他資源が律速でスラックが生じる場面(MilitaryFactory が IndustrialGoods 不足で Iron を使い切れない等)が、240か月の範囲では Dwarf・Ferrowyn に存在しなかった。

### 6.2 D1・D3 の効果(FIX1共通、当該国のみに適用、m240)

| 国 | 指標 | D0 | D1 | D3 |
|---|---|---|---|---|
| **Dwarf** | MS産出(累計) | 50,917 | 76,294(**+49.8%**) | 76,294(=D1) |
| | 補給不足日数 | 19,266 | 14,633(−24.0%) | 同左 |
| | 不足率(日数/Division日数) | 27.3% | 16.1% | 同左 |
| | Division数 | 11 | **16** | 同左 |
| | IG備蓄 / Factory Level | 23,481 / 26 | 10,916 / **33** | 同左 |
| | 国庫 / 平均生活水準(州単純平均) | 16,523 / 66.1 | 16,942 / 65.8 | 同左 |
| **Ferrowyn** | MS産出 | 19,332 | 22,874(+18.3%) | =D1 |
| | 補給不足日数 | 4,936 | 2,080(−57.9%) | 同左 |
| | 不足率 | 20.3% | 8.5% | 同左 |
| | Division数 | 7 | 8 | 同左 |
| | 国庫 / 生活水準 | 4,350 / 70.4 | 5,503 / **66.0** | 同左 |
| **Elfin** | MS産出 | 9,344 | 14,678(+57.1%) | 16,385(**+75.4%**) |
| | 補給不足日数 | 23,843 | 18,362(−23.0%) | 16,742(−29.8%) |
| | 不足率 | 71.7% | 55.2% | 50.3% |
| | IG備蓄 | 3,879 | 5 | 745 |
| | 国庫 / 生活水準 | 8,239 / 69.0 | 7,455(**−9.5%**)/ 68.9 | 8,235 / 69.0 |
| **Passguard** | MS産出 | 27,387 | 27,272(−0.4%) | 26,817(**−2.1%**) |
| | 国庫 | 1,842 | 1,292(**−29.9%**) | 1,159(**−37.1%**) |
| | 平均生活水準 | 70.0 | 68.5 | **63.5** |
| | 破産月数(240か月) | 17 | 23 | 43 |
| **Arcadia** | MS産出 | 652 | 660(+1.2%) | 652 |
| **Oceanic** | (軍需工場なし) | — | 変化なし | 変化なし |

- Arcadia・Oceanic は Iron 自体がほぼ無いため、配分方式の効果が出ない。
- Dwarf の補給不足日数の減少は、Division を11→16へ増やした上での値である。Division数が増えれば絶対日数は増えやすいため、不足率で併記した。
- D3 は Elfin で D1 より良く、Passguard で D1 より悪い。Dwarf・Ferrowyn では D1 と同一(IG が潤沢で解放が起きない)。

### 6.3 効果が出る時期

D1 の効果は**Iron 備蓄が尽きてから**始まる。90日・36か月では全国で D0 と同一(Passguard を除く。m36 で MS 3,673→3,412)。

Dwarf の転換点(D0 と D1、`FIX1`、月次):

| 月 | D0 Iron備蓄 / MF引落し / Factory引落し | D1 同 |
|---|---|---|
| m096 | 335 / 105.7 / 65.0 | 同一 |
| m100 | 89 / 87.6 / 58.9 | 91 / **105.6 / 0** |
| m120 | 45 / 49.9 / 58.6 | 100 / **110.6 / 0** |

D1 は m100 以降、Iron 収入約109を MilitaryFactory に使い切る(D0 の約2倍)。Factory は Iron 0 となり IG 産出が止まる。IG備蓄は m100 の 3,942 から m120 の 2,853 へ月約55減る。

### 6.4 D1 の副作用

| 副作用 | 内容 | 根拠 |
|---|---|---|
| Factory 過剰増設 | Dwarf の Factory Level が 26→33。IG備蓄が減ると AI が「IG不足」で Factory を増設する(P26-008-C)が、増設した Factory は Iron の残りしか得られない | m240 の Level |
| Division 増加 | 供給が増えると AI が Division を増やす(Dwarf 11→16、Ferrowyn 7→8)。動員で民間労働力が減る | m120/m240 |
| 破産境界への影響 | Passguard は国庫が0付近で、破産月数が 17→23(D1)→43(D3)。破産は生活水準 −0.5/月、稼働率 ×0.8 | 国庫・破産月数 |
| IG備蓄の枯渇 | Elfin は IG が 3,879→5。以後の MilitaryFactory は IG で律速される(D3 は解放して 745 を保つ) | m240 |

## 7. 生活水準の因果

### 7.1 生活水準の計算

`update_state_welfare_and_unrest`(州ごと、毎月)の増減:

| 項 | 条件 | 増減 |
|---|---|---|
| 食料 | あり / なし | +0.5 / −1.0 |
| 雇用(`E/P` = 雇用者数 / 潜在労働力) | ≥0.9 / <0.7 | +0.5 / −0.8 |
| 物流 | ≥ボーナス閾値 / <0.5 | +0.3 / −0.5 |
| 税率 | ≤0.10 / >0.25 | +0.5 / −0.8 |
| 破産 | 国庫<0 | −0.5 |

- 雇用者数は `required_workforce × level`(Factory 20,000、MilitaryFactory 25,000)で決まり、**稼働率に依存しない**。つまり Factory の Level が雇用を決める。
- IndustrialGoods は福祉・財政・人口のどこでも消費されない。MilitaryFactory の入力にしか使われない。
- 維持費は `maintenance_cost × level × op`(MilitaryFactory 40、Factory 30)で、稼働率に比例する。

### 7.2 候補A(Factory上限6)の低下の原因

Dwarf、m120、7州。州単純平均 65.7→58.8(−6.9)の内訳:

| 州 | 人口 | 基準(雇用率 / LS) | 上限6(雇用率 / LS) |
|---|---|---|---|
| id=19 | 159,216 | F1 MF2、**E/P=0.948 / LS 96.4** | F0 MF0、E/P=0.171 / **LS 50.0** |
| id=6 | 626,415 | F3 MF4、E/P=0.390 / 37.6 | F0 MF7、0.432 / 35.5 |
| 他5州 | — | LS 70.0/60.0/62.0/82.0/52.0 | 同一 |

- **LS の差の合計は 48.5。うち id=19 の 1州が 46.4(95.7%)**を占める。
- id=19 は人口が全体の約2.8%の小州で、Factory と MilitaryFactory の雇用により雇用率 0.948(≥0.9)に達し、毎月 +0.5 を得ていた。上限6でこの州の建物が消え、雇用率が 0.171 に下がった。
- **人口加重の平均では 69.4→67.9(−1.5)**。州単純平均の −6.9 は、主に1つの小州の指標差である。
- Dwarf の他州は雇用率 0.10〜0.39 で、常に雇用の −0.8 区分に入る。

**結論:** 候補Aの「平均生活水準 −7ポイント」は、Factory の総量ではなく**小州の雇用率閾値(0.9)跨ぎ**が主因である。人口加重では −1.5。

### 7.3 D1 による低下の原因

| 国 | 低下 | 原因 | 確度 |
|---|---|---|---|
| Ferrowyn(70.4→66.0) | −4.4 | m230 に Division が1つ増え、動員 +10,000。以後 LS が横ばいになる(基準は月+0.2)。平均雇用率は 0.444→0.439 でほぼ同じ | 時系列で確認 |
| Passguard(70.0→68.5) | −1.5 | 国庫が0付近の期間(m21〜m33)で破産月が増え(D0 の2か月→D1 の8か月、いずれも m21〜m33)、破産月は −0.5/月 | 時系列で確認 |
| Dwarf(66.1→65.8) | −0.3 | 小さい | — |
| Elfin(69.0→68.9) | −0.1 | 小さい | — |

- 低下は「IG の不足」や「Factory の稼働率」から直接生じていない。**IG は福祉に関与しない。**
- 国庫が0付近にある国の生活水準は、破産境界を跨ぐかどうかの履歴依存性が強い。**Passguard の D1 と D3 の差(LS 68.5 と 63.5)を原因まで特定してはいない。**

### 7.4 測定の限界

- 他国が無関係な構成でも、世界全体の軌跡の差により平均生活水準が最大 ±4 変わる(Oceanic は政策を適用していないが、全国同時適用で 70.3 と 74.3。単独適用では同一)。
- 平均生活水準は**州の単純平均**で、人口加重ではない。
- D1 の州別・時系列(Dwarf の各州の雇用率・IG・LS)は、今回はダンプしていない(候補A のみ州別)。

## 8. 採否と次タスク

### 8.1 会計補正(FIX1)

| 項目 | 判断 |
|---|---|
| 独立の欠陥か | **はい。** 候補D と無関係に、Arcadia(m002〜)・Oceanic(m026〜)に無償産出が存在する |
| D の評価を妨げるか | 一部。Dwarf・Ferrowyn では失敗が無いため D の効果は補正に依存しない。Arcadia・Oceanic・Elfin(m149〜)では補正の有無で結果が変わる |
| 補正で他国が変わるか | 失敗が無い国は変わらない(実測) |
| 採否 | **採用(次の原子的タスク)** |

### 8.2 候補D

| 項目 | 判断 |
|---|---|
| 軍需産出(Iron律速の国) | Dwarf +49.8%、Ferrowyn +18.3%、Elfin +57.1%(D1)/+75.4%(D3) |
| 補給の改善 | Dwarf 不足率 27.3→16.1%、Ferrowyn 20.3→8.5%、Elfin 71.7→50.3%(D3) |
| 副作用 | Passguard の国庫 −30〜37%・破産月数 17→23〜43、Division 増による動員、Factory の過剰増設 |
| 判定 | **本タスクでは不採用(保留)** |

不採用の理由:

1. 副作用(Passguard の財政悪化、Division増による動員)の因果が、時系列まで特定できていない(§7.3)。
2. 会計補正が世界全体の軌跡を変えるため(戦争総数 8→7)、補正後の基準で再測定せずに採否を決められない。
3. 「軍需月産が増えた」だけでは採用しない(依頼 §7)。
4. 測定は決定論的な1軌跡で、複数seedによる分散を持たない。

D1 と D3 のどちらを採るかは、再測定で決める。D3 は Elfin で D1 より良く、Passguard で悪い。

### 8.3 次の原子的タスク(1件)

**P28-010-IMPL-001: 生産入力の原子的会計**

| 項目 | 内容 |
|---|---|
| 対象 | `src/economy/production.rs` の Step 3 のみ |
| 仕様 | エントリごとに `op_eff = min(op, 各入力の現在備蓄 / (base × level))`(f64)。消費 = `base × level × op_eff`(備蓄を超えない)。産出・研究力・`building_operation_ratios` は `op_eff` で計算する。`consume` の戻り値は検査し、`false` なら `debug_assert` と `warn!` |
| 不変条件 | 失敗が無い国は変えない。処理順(`BuildingType::ALL` × `StateId`)を維持する。Save V1 とフォーマットを変えない。Division 側の補給ゲート(40/70日・前線戦力比)を変えない。国家IDによる特例を作らない |
| 検証 | 失敗0件の e2e(Arcadia の Iron 備蓄が 0.975 に固定されないこと、無償MSが0)。Dwarf・Passguard のチェックポイントが変わらないこと。保存式の残差 0.000 |
| 範囲外 | 候補D・Factory の増設制限・AI 建設判断・基礎値の変更 |

候補D の実装は、本タスクの後に再測定し(別タスク)、結果を見て決める。両者を一度に実装する必要はない: FIX1 は配分方式と独立に適用できる(D0・D1・D3 のすべてで失敗0件)。

## 9. 契約の確認

| 契約 | 結果 |
|---|---|
| P28-004 の募兵制限案を再導入しない | 触れていない |
| 補給ゲート 40/70日・前線戦力比を変更しない | 変更なし |
| 国家IDの特例を追加しない | 一時パッチの `P28_ONLY` は測定用(国別分離)で、恒久コードに入れていない。削除済み |
| プレイヤーの建設操作を変更しない | 変更なし |
| Save V1・既存ユーザーSave | `saves/savegame_v1.ron` は読み書きしていない |
| 無償産出を「補給改善」と数えない | §5.2 で FIX0→FIX1 の補給不足は増えると明記 |
| 約20%稼働率の断定 | Dwarf の再現値(0.208〜0.221)は手動プレイの値の原因と断定しない |

## 10. 未確認事項

| 項目 | 状況 |
|---|---|
| 手動プレイの「約20%」 | 未確認。Dwarf の再現値(0.208〜0.221)と同じ原因とは断定しない |
| UI実ウィンドウ確認 | MANUAL VERIFICATION PENDING |
| P27-005 手動確認 | PENDING |
| MagicAcademy の無償研究力 | Oceanic で316エントリ失敗。研究力の量は未定量 |
| D1 の州別・時系列(Dwarf) | 未ダンプ |
| 国力・Tier と AI 建設判断の変化 | Level の推移(Factory 26→33 等)のみ。Tier は未測定 |
| 複数seed・複数開始条件 | 未実施(決定論的な1軌跡) |
| Passguard の D1/D3 差の原因 | 破産月数の増加までは確認。LS 68.5 と 63.5 の差の原因は未特定 |
| 前回報告書の「Oceanic 2か月」 | 今回の測定(Iron失敗12か月)と数え方が異なる可能性。前回の数え方の再現は未実施 |

## 11. 実行コマンドと結果

測定(一時パッチ・一時テスト使用。削除済み):

```text
P_FIX={0,1,2} P_ALLOC={0,1,2,3} P_ONLY={0..5|99} P_MONTHS={120,240} P_DETAIL=... P_FCAP={0|6}
cargo test --test p28_010_pre002_temp_diag -j 2 -- --nocapture --test-threads=1
```

| 実験 | 件数 |
|---|---|
| 全国同時 FIX × ALLOC(3×3、240か月) | 9 |
| 国別分離(6か国 × {FIX0, FIX1×{D0,D1,D2}}、240か月) | 24 |
| D3(単独4か国+同時) | 5 |
| 州別・時系列(Passguard・Ferrowyn・Dwarf の D0/D1/D3、Dwarf の候補A) | 複数 |
| 基準の再現性(FIX0 を2回、全フィールド一致) | 確認済み |

最終検証(一時パッチ削除後、逐次実行):

| コマンド | 結果(今回の実測) |
|---|---|
| `cargo test --test p23_004_economic_workforce_ratio_e2e_test` | 7 passed / 0 failed |
| `cargo test --test p26_003_military_supplies_production_e2e_test` | 49 passed / 0 failed |
| `cargo test --test p26_003_rev_002_supply_production_units_e2e_test` | 56 passed / 0 failed |
| `cargo test --test p26_003_rev_003_economic_power_capacity_e2e_test` | 21 passed / 0 failed |
| `cargo test --test p26_004_daily_supply_consumption_allocation_e2e_test` | 19 passed / 0 failed |
| `cargo test --test p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test` | 11 passed / 0 failed |
| `cargo test --test p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test` | 15 passed / 0 failed |
| `cargo test --test p26_004_pre_005_initial_supply_bridge_migration_e2e_test` | 22 passed / 0 failed |
| `cargo test --test p26_008_c_supply_chain_construction_priority_e2e_test` | 18 passed / 0 failed |
| `cargo test --test p26_det_001_construction_ai_determinism_e2e_test` | 10 passed / 0 failed |
| `cargo test --test p26_det_001_rev_001_cross_state_production_order_e2e_test` | 12 passed / 0 failed |
| `cargo test --test p27_002_a_ai_construction_fiscal_reserve_e2e_test` | 17 passed / 0 failed |
| `cargo test --test p27_002_b_ai_tax_rate_adjustment_e2e_test` | 18 passed / 0 failed |
| `cargo test --test p27_002_c_residual_fiscal_self_stabilization_e2e_test` | 20 passed / 0 failed |
| `cargo test --test p28_002_wartime_military_factory_construction_e2e_test` | 3 passed / 0 failed |
| `cargo test --test p28_005_ai_railway_priority_e2e_test` | 3 passed / 0 failed |
| `cargo test --lib economy::` | 58 passed / 0 failed |
| `cargo test --lib country::country_ai` | 76 passed / 0 failed |
| `cargo test --all-targets -j 2` | **2794 passed / 0 failed / 1 ignored**(88 binaries) |
| `cargo clippy --all-targets --all-features -- -D warnings` | **警告 0** |

- 全体テストの結果は、開始時の基準(前回報告からの引用 2794/0/1、clippy 0)と一致した。
- 全体テストで、既知の非決定的PNG 9件(`p20-007`・`p20-009`・`p21-save-002e` のスクリーンショット)が再生成された。`git status --short` で確認した9件だけを個別に `git checkout --` で復元した。

## 12. 保護事項

- `saves/savegame_v1.ron`: 読み書き・復元・ステージなし。
- `tests/land_war_combat_peace_test.rs`: 変更なし。
- ワークスペース全体の format・`git add .`/`-A`・push・tag・amend・rebase・reset・stash: 行っていない。
- 本番コード・データ・既存テスト・既存報告書: 変更なし。
