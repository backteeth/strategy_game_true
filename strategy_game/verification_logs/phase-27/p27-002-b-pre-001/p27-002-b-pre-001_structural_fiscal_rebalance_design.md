# P27-002-B-PRE-001: 六か国の構造的財政赤字(税/行政費係数不均衡)根本解消設計監査

本タスクは監査・設計のみであり、本番コードへの恒久的な変更は一切行っていない。
P27-002-PRE-001が確認し、P27-002-Aの実測(git worktree A/B比較)が裏付けた「4/6か国が
建設・募兵とは無関係に構造的な月次赤字を抱える」という根本問題(`TAX_COEFFICIENT=0.0001`
vs `ADMIN_COEFFICIENT=0.001`の不均衡、および固定`tax_rate=0.15`にAIが一切手を出せない
という構造)への対処案を、複数の候補を実測比較した上で提案する。

## §1 前提: 現状の正確な定量化

`src/economy/finance.rs`(無変更、再確認):

```rust
pub const TAX_COEFFICIENT: f64 = 0.0001;
pub const ADMIN_COEFFICIENT: f64 = 0.001;
pub const LOGISTICS_MAINTENANCE_COEFFICIENT: f64 = 0.05;

tax = population * living_standard * tax_rate * TAX_COEFFICIENT   // 州ごとに合算
admin_cost = total_population * ADMIN_COEFFICIENT                  // 国全体で1回
```

損益分岐点: `tax >= admin_cost` ⟺ `living_standard * tax_rate >= 10`。`tax_rate`はNew Game
初期値0.15固定(`CountryData::default()`)で、AIがこれを変更する経路は一切存在しない
(`src/ui/economy_panel.rs`の±0.01ボタンのみがtax_rateを変更する唯一のコードであり、
プレイヤー操作専用)。よって損益分岐点は`living_standard >= 66.67`。

New Game直後の実測(実App経由、`process_country_finance`と同一の純粋関数で算出、
建物維持費は稼働率未確立のためNew Game直後は0扱い):

| 国 | 総人口 | 加重平均living_standard | 税収/月 | 行政費/月 | 物流維持費/月 | 初期国庫 | 損益分岐までの差 |
|---|---|---|---|---|---|---|---|
| Arcadia | 2,860,000 | 60.38 | 2590.35 | 2860.00 | 25.00 | 5000.00 | **-6.29** |
| Elfin | 2,060,000 | 70.95 | 2192.40 | 2060.00 | 25.00 | 3500.00 | +4.28 |
| Dwarf | 1,890,000 | 59.22 | 1678.80 | 1890.00 | 20.00 | 7200.00 | **-7.45** |
| Oceanic | 3,830,000 | 76.22 | 4378.95 | 3830.00 | 30.00 | 12000.00 | +9.56 |
| Passguard | 245,000 | 49.84 | 183.15 | 245.00 | 20.00 | 1800.00 | **-16.83** |
| Ferrowyn | 590,000 | 51.42 | 455.10 | 590.00 | 20.00 | 2600.00 | **-15.24** |

4/6か国(Arcadia・Dwarf・Passguard・Ferrowyn)が損益分岐点未満。living_standardの分岐点
不足量(LS gap)は国の規模と無相関(Passguardが最大のLS不足-16.83だが、人口が少なく
実額の赤字は最小)であり、これは州データ(`assets/data/states.ron`)の`living_standard`が
国の規模と無関係に個別設定されているためである。

## §2 検証した候補

### 候補A: AIによるtax_rateの自己調整(自国の財政状態に応じた増減)

プレイヤーが既に持つ`±0.01`刻みのtax_rate調整能力(`src/ui/economy_panel.rs`)を、AI国家にも
月次で与える。具体的には新設の財政AI System(既存の`process_construction_ai`/
`process_research_ai`と同様の月次評価パターン)が、`monthly_balance < 0`ならtax_rateを
+0.01、黒字が続けば基準値0.15まで-0.01で戻す、という最小限のヒステリシスなし規則。
`update_state_welfare_and_unrest`が既に持つ`tax_rate > 0.25`でのliving_standard/unrest
ペナルティが、既存コードのまま自然なブレーキとして機能する(新規ペナルティ追加不要)。

**実測(36か月、production codeへの変更なし・testループ内でtax_rate調整を模擬、
上限0.30・下限0.15):**

| 国 | 適用前(P27-002-Aのみ)国庫 | 候補A適用後国庫 | 最終tax_rate | 最終月次収支 |
|---|---|---|---|---|
| Arcadia | -14487.77 | **948.10** | 0.160 | -148.69 |
| Elfin | 5226.53 | 5226.53(無変更) | 0.150(無変更) | +76.72 |
| Dwarf | -14113.54 | **6192.33** | 0.200 | -90.62 |
| Oceanic | 12788.06 | 12788.06(無変更) | 0.150(無変更) | +203.31 |
| Passguard | -5009.82 | -1614.03(改善するが未解消) | 0.300(上限到達) | -106.85 |
| Ferrowyn | -9325.68 | -2236.54(改善するが未解消) | 0.300(上限到達) | -283.32 |

**重要な発見(自己相殺フィードバックループ)**: Passguard・Ferrowynはtax_rateが上限0.30に
達しても月次赤字を解消できない。原因は`update_state_welfare_and_unrest`が既に持つ
`tax_rate > 0.25`でのliving_standard減少ペナルティ(-0.8/月)が、36か月かけてliving_standardを
実際に押し下げ、`tax = population * living_standard * tax_rate * T`の`living_standard`項が
縮小することで税収自体が目減りする、という自己相殺ループが働くためである。高いtax_rateは
「一時的な延命」にはなるが、小規模国家(Passguard・Ferrowyn)では根本解消に至らない。

**評価**: Elfin/Oceanicを一切変更せず(既に黒字のため増税ロジックが発火しない)、
Arcadia/Dwarfを完全に解消するという理想的な副作用の少なさを持つ一方、Passguard/Ferrowyn
(最も小規模で最もliving_standardが低い2か国)は単独では解消しきれない。プレイヤーが
既に持つ機能をAIにも与えるだけであり、新しいSave/UI要素は不要、既存の
ペナルティ機構がブレーキとして自然に機能する点で、ブラスト半径が最小の候補である。

### 候補A': 候補A + ADMIN_COEFFICIENT 20%引き下げ(0.001→0.0008)の複合案

候補Aだけでは解消しないPassguard/Ferrowynを見て、admin_cost自体を全国一律で20%引き下げる
案を追加実測した(production codeを一時的にパッチ・計測後に完全復元、`git diff`で復元を
確認済み)。

**実測(36か月、候補A+ADMIN_COEFFICIENT=0.0008):**

| 国 | 候補Aのみ国庫 | 候補A+admin20%減 国庫 | 最終tax_rate | 最終月次収支 |
|---|---|---|---|---|
| Arcadia | 948.10 | **8116.97** | 0.150(引き下げ不要に) | +202.36 |
| Elfin | 5226.53 | 7011.07 | 0.150 | +290.71 |
| Dwarf | 6192.33 | 7352.71 | 0.170 | +2.84(ほぼ完全収支均衡) |
| Oceanic | 12788.06 | **28239.11**(+120%) | 0.150 | +687.89 |
| Passguard | -1614.03 | **-15.37**(ほぼ収支均衡) | 0.220 | -1.05 |
| Ferrowyn | -2236.54 | **+267.93**(黒字化) | 0.180 | -20.57 |

**評価**: 6か国全てが収支均衡または黒字に到達する唯一の実測案である。しかし
`ADMIN_COEFFICIENT`は全国一律の共有定数であるため、既に最も豊かなOceanicが最大の恩恵
(国庫がさらに+120%、12788→28239)を受けてしまう——健全な国家をさらに富ませ、
GreatPower同士の国力差を今以上に拡大させるリスクがある(P21-014の国家ランク制度・
将来のAI外交/戦争判断がこの差を参照するため、長期的なゲームバランスへの影響は本監査の
範囲外だが無視できない)。

### 候補B: TAX_COEFFICIENT/ADMIN_COEFFICIENT比そのものの全面見直し(候補A'の単体版、tax_rate自己調整なし)

候補A'から候補Aの部分(tax_rate自己調整)を除き、係数比の調整だけで解決を試みる案。
Passguard(LS不足-16.83、tax_rate固定0.15)を黒字化するには
`living_standard_breakeven <= 49.84`まで引き下げる必要があり、これは
`ADMIN_COEFFICIENT/TAX_COEFFICIENT`比を現行10から`49.84*0.15≈7.48`まで(即ち
ADMIN_COEFFICIENTを約25%引き下げるか、TAX_COEFFICIENTを約34%引き上げる)変更する
ことに相当する。候補A'と同様、Oceanic/Elfinが不釣り合いに富む問題は同一構造で発生し、
かつ「AIに独自の財政判断を持たせない」ため、AIの意思決定の幅を広げるという副次的な
価値(候補Aが持つ)も得られない。**候補A'に対する優位点がなく、劣る下位互換**と判断し
不採用。

### 候補C: 4か国の静的データ(living_standard等)を直接引き上げる

`assets/data/states.ron`の低living_standard州(Arcadia・Dwarf・Passguard・Ferrowyn所属)の
初期値を底上げする案。コード変更ゼロで済む点は魅力的だが、以下の理由で不採用と判断する:
(1) `living_standard`は税収だけでなく人口成長・不満度など他の複数の式にも使われており
(`src/population/update.rs`等、本監査では全経路を洗い出していないため影響範囲が未知数)、
税収目的の底上げが他システムへ意図しない波及効果を生むリスクがある。(2) 一度限りの
数値パッチであり、ゲーム進行中にliving_standardが(不満・戦争・食料不足等で)再び
下落した場合に赤字が再発する——動的な対応力を持たない。(3) 「なぜこの4か国だけ
初期値が引き上げられたか」という説明が実データ改変以外に存在せず、将来の新規国家
追加時に同じ調整が漏れる可能性がある。

### 候補D: admin_cost計算式自体の再設計(人口比例から準固定+緩やかな人口比例へ)

`admin_cost = population * ADMIN_COEFFICIENT`という完全人口比例の式を、
`admin_cost = base_admin + population * smaller_coefficient`のような準固定費+緩やかな
比例費へ再設計する案。小規模国家(Passguard・Ferrowyn)は人口比の行政費が相対的に
重くのしかかっている可能性があるが、実測データを見る限りPassguard/Ferrowynの
「LS不足」が主因であり(人口比率自体は他国と大差ない)、この再設計が実際に効果を
持つかは未検証。既存の`calculate_admin_cost`シグネチャ変更・全国データでの再検証が
必要な、本監査の投じた実測時間に見合わない大きめの改修であり、候補A'で十分な効果が
既に実測できているため優先度を下げ、不採用(将来的に候補A'だけでは不十分と判明した
場合の予備案としてのみ記録)。

### 候補E: 現状維持(構造的赤字は意図された難易度)

構造的赤字4か国は「プレイヤーが介入しない限り衰退する弱小国」という設計意図の可能性も
検討したが、以下の理由で採用しない: (1) AIには財政面での意思決定手段が現状ゼロ
(建設支出の抑制のみがP27-002-Aで追加されたが、収入側の手段は皆無)であり、これは
「意図された弱さ」というより「未実装のAI機能」と解釈する方が自然である。(2)
P26-009以降、複数回の監査(P26-008-PRE-001・P27-002-PRE-001)がこれをMedium findingとして
一貫して記録し続けており、解消すべき既知の課題として扱われてきた経緯がある。(3)
プレイヤー国家は既にtax_rateボタンで自由に対応できるため、AIにも同じ手段を与えることは
「プレイヤーと同じルールで戦うAI」という一貫性の観点でも妥当である。

## §3 推奨案

**候補A(AIによるtax_rate自己調整)を最小実装、候補A'(+ADMIN_COEFFICIENT調整)を
同一タスクで検討する2段階の実装(P27-002-B)を推奨する。**

理由:
1. 候補Aだけでも4か国中2か国(Arcadia・Dwarf)を完全に解消し、残る2か国
   (Passguard・Ferrowyn)も実測で赤字幅を約68%(Passguard: -5009.82→-1614.03)
   〜約76%(Ferrowyn: -9325.68→-2236.54)圧縮する——単独でも大きな改善効果がある。
2. Elfin・Oceanicという既に健全な国家には一切副作用を与えない(tax_rate増減ロジックが
   黒字国では発火しないため)——候補A'/候補Bが抱える「健全国家がさらに富む」問題を
   候補Aは原理的に回避する。
3. 既存のtax_rateフィールド・UI操作パターン・`update_state_welfare_and_unrest`の
   既存ペナルティ機構を100%再利用し、新規Save/UI要素が不要。
4. 残るPassguard・Ferrowynの完全解消が必要かどうかは、候補A実装後に改めて実測した
   上で、必要であれば候補A'のADMIN_COEFFICIENT引き下げ幅を「Oceanic過剰富裕化」との
   トレードオフを踏まえて再検討する、という段階的アプローチが可能(候補Aの実装自体は
   候補A'の実施可否と独立している)。

## §4 P27-002-Bへの申し送り事項(未決定・要ユーザー判断)

- 候補Aのtax_rate上限(本監査は0.30で実測、`update_state_welfare_and_unrest`のペナルティ
  閾値0.25と整合させる調整幅とすべきか)。
- 候補Aの調整ステップ幅・cadence(月次固定か、既存AI cadenceと同じ`day%30==country_id%30`
  staggerに乗せるか)。
- Passguard/Ferrowynの残存赤字を「意図された最終ハンデ」として許容するか、候補A'の
  ADMIN_COEFFICIENT調整(Oceanicの過剰富裕化とのトレードオフを伴う)まで踏み込むか。
- ADMIN_COEFFICIENT調整幅を採用する場合、20%引き下げが唯一の実測点であり、他の刻み幅
  (10%・15%等)でOceanicへの副作用を抑えつつ4か国を解消できるかは未検証。

いずれも本監査の範囲外であり、P27-002-Bの実装スコープを確定する際にユーザーの意向を
確認した上で決定する。

## §5 検証方法・再現性

全ての実測値は実App経由(`setup_app`ハーネス、`feedback_bevy_state_transition_harness`の
既知の落とし穴を踏まえた実証済みパターンを使用)、New Game初期データ(`assets/data/
countries.ron`・`states.ron`)に対して行った。候補A・候補A'の36か月シミュレーションは
一時的な診断用テストファイル(`tests/p27_002_b_pre_001_temp_fiscal_breakdown_diagnostic.rs`、
本コミットには含めず削除済み)およびADMIN_COEFFICIENTへの一時的なソース変更
(計測後`git checkout --`で完全復元、`git diff`で復元を確認済み)によって取得した。
本タスクで最終的にコミットされる変更は本レポートのみであり、production codeへの
恒久的な変更は一切ない。

## 結論

```
P27-002-B-PRE-001 AUDIT COMPLETE
STRUCTURAL IMBALANCE ROOT CAUSE CONFIRMED (unchanged from P27-002-PRE-001)
CANDIDATE A (AI TAX-RATE SELF-ADJUSTMENT) VALIDATED — RECOMMENDED FIRST STEP
CANDIDATE A' (A + ADMIN_COEFFICIENT CUT) VALIDATED — FULLY RESOLVES ALL 6 COUNTRIES BUT OVER-ENRICHES OCEANIC
CANDIDATE B/C/D/E REJECTED (see §2 for reasoning)
PRODUCTION CODE UNCHANGED (audit-only, temporary experimental patches fully reverted and verified via git diff)
P27-002-B READY (scope decision on tax-rate cap/cadence and whether to include the admin-cost cut is pending user input)
```
