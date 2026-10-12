# P28-010-IMPL-002：IG備蓄で保護したIron軍需優先配分(D4)

P28-010-PRE-003 で選定された D4 を `src/economy/production.rs` に実装した。FIX1(P28-010-IMPL-001 の原子的入力会計)は維持している。D1・D3 の無条件優先、Factory 増設抑制、募兵制限、建設AI、補給ゲート、戦闘式、RON データ、UI、Save 形式は変更していない。

## 0. 基準状態と判定

| 項目 | 値 |
|---|---|
| 期待HEAD | `1a7d717c6cfd2fe6e56411f6d907ee7a8eccaf36` |
| 開始時の実HEAD | `1a7d717c6cfd2fe6e56411f6d907ee7a8eccaf36`(一致) |
| 開始時のテスト基準 | 2811 passed / 0 failed / 1 ignored、clippy 警告 0(**前回報告からの引用**。今回の実測は §11) |

```text
P28-010-IMPL-002 IMPLEMENTATION COMPLETE
FIX1 ACCOUNTING PRESERVED
D4 CONTRACT IMPLEMENTED（予算に忠実な按分で実装。PRE-003 の実測 D4 とは §6 の差がある）
D1/D3 STARVATION OSCILLATION NOT REPRODUCED（Passguard、240か月まで D0 と完全一致）
KNOWN LIMITATION: 閾値(3か月分)付近の月次ゲート切り替え(有界、§7)。閾値は調整していない
```

## 1. 実装(コードと処理順)

### 1.1 D0 と D4 の定義(実コード上の条件)

Step 1 は州を `StateId` 昇順に、建物を `BuildingType::ALL` 順に走査し、需要 `want = base × level × t`(`t = min(経済用労働力比, 物流比)`、f32→f64)を資源別・**建物種別 × 資源別**に集計する(新規: `wanted_by_building`)。

| 名称 | 規則(Iron の入力充足率) |
|---|---|
| **D0** | `sat = clamp(月初備蓄 / Σ需要, 0, 1)`(全消費者で共通)。現行 |
| **D4 の適用条件** | **全て満たすとき**: (1) MilitaryFactory の Iron 需要 > 0、(2) Iron の需要合計 > Iron の月初備蓄(Iron 不足)、(3) `IG の月初備蓄 ≥ 3.0 × MilitaryFactory の IG 需要`(**境界と等しい場合は適用**)。成立しなければ D0 |
| **D4 の配分** | `m = min(1, 各 MilitaryFactory 入力の 備蓄/需要 の最小値)`(Iron 自身と IG を含む)。MilitaryFactory の Iron 充足率 = `m`、予約量 = `需要 × m`(使えない分は予約しない)。他の Iron 消費者の充足率 = `clamp((Iron備蓄 − 予約量) / 他の需要合計, 0, 1)` |
| **他の消費者の稼働率** | `planned_op = min(労働力比, 物流比, 資源 sat の最小, ...)` のうち **Iron の項は `他の充足率 × t`** とする(§6) |

### 1.2 「月間IG需要」の定義

| 項目 | 定義 |
|---|---|
| 対象 | 全ての MilitaryFactory(全州・全 Level)。国家内の合計 |
| 稼働率の扱い | `t`(労働力比・物流比の小さい方)を**含む**(Step 1 の `needed` と同じ式)。Iron/IG の入力充足率や破産ペナルティは**含まない** |
| 備蓄 | Step 2 と同じ時点の月初備蓄(当月の Mine 産出・Factory 産出は含まない) |
| 量子化 | しない。f64 の比較 `ig_stock < 3.0 × want`(境界は `≥` で適用)。加算は `StateId` 昇順の固定順なので、同じ入力から同じ値になる |
| 浮動小数点の境界 | 備蓄が 3.0×需要 とビット単位で等しい場合は D4、`next_down` は D0(テスト: `gate_is_exact_for_a_non_dyadic_operation_ratio`、`ig_reserve_boundary_is_inclusive`) |

### 1.3 配分順

| 順序 | 内容 |
|---|---|
| Step 3 外側 | `BuildingType::ALL`(Mine → Factory → MilitaryFactory)。変更なし |
| Step 3 内側 | `StateId` 昇順。変更なし |
| MilitaryFactory 入力の最小比 | `ResourceType::ALL` 順に走査(HashMap 反復順に依存しない) |
| 他の消費者の需要和 | `BuildingType::ALL` 順に加算 |
| 引落し | FIX1: `limit_operation_ratio_to_stock` → `consume_inputs_atomically`(`consume` の戻り値を必ず扱う)。`ResourceType::ALL` 順 |

州の挿入順を全720通りに変えて(ゲートの両側、5つの備蓄水準)結果が完全一致する(テスト `insertion_order_does_not_change_the_result`)。

### 1.4 変更ファイル(本番コードは1ファイルのみ)

`src/economy/production.rs`: 定数 `MILITARY_IRON_PRIORITY_IG_RESERVE_MONTHS`、純粋関数 `military_iron_priority_satisfaction`、Step 1 の建物別需要、Step 2 直後の判定、Step 3 の Iron 充足率の選択。月次 System 順序・Mine の産出順・建物の生産式・研究力・Iron 以外の配分は変更していない。

## 2. FIX1 の保護

| 契約 | 結果 |
|---|---|
| 入力は消費可能な量にクランプ | 維持(`limit_operation_ratio_to_stock`)。D4 の充足率は `min_res_sat` に入るだけで、クランプは常に後段で働く |
| 全入力の引落しが成立した分だけ産出 | 維持(`consume_inputs_atomically`) |
| 一部入力だけ消費して失敗しない | 維持 |
| 無償の MS・IG・研究力 | 再発しない(下記の台帳テスト) |
| Iron・Coal・IG・MagicCrystal の保存則 | 全6か国・240か月と 600か月で残差 < 1e-3、備蓄 ≥ 0、Iron の据え置きなし |
| `consume` の戻り値を無視しない | 維持(FIX1 のコードに変更なし) |

保存則の恒等式は、IG・Iron・Coal を 12×3×12 通りに掃引してゲートの両側(44.999 / 45 / 45.001 など)で確認した(`fix1_identities_hold_across_the_gate`)。

## 3. 境界・回帰テスト

### 3.1 fixture(`tests/p28_010_impl_002_ig_guarded_iron_priority_test.rs`、12件)と単体テスト(`production.rs`、7件)

| 観点 | テスト |
|---|---|
| IG が3か月分の直下 / ちょうど / 直上 | `ig_just_below_three_months_uses_the_proportional_rule`、`ig_at_or_above_three_months_prioritizes_the_military_factory` |
| Iron 不足 / 充足 | 同上、`iron_sufficient_is_identical_for_every_ig_level`、単体 `iron_sufficient_keeps_the_proportional_rule` |
| IG 不足で D0 へ戻り、回復後に D4 | `guard_releases_iron_to_the_factory_and_priority_returns_after_ig_recovers`(40か月) |
| IG 律速の MilitaryFactory が Iron を過剰予約しない | 単体 `iron_is_not_reserved_beyond_what_another_military_input_allows`、`military_is_capped_by_stock_and_others_get_nothing` |
| 複数州・複数 Level | `multiple_states_use_the_national_totals` |
| MilitaryFactory 0件 / 需要 0 / 備蓄 0 | `no_military_factory_keeps_the_legacy_result`、`zero_stock_and_zero_demand_are_well_defined` |
| 丸め・境界 | `gate_is_exact_for_a_non_dyadic_operation_ratio`、単体 `ig_reserve_boundary_is_inclusive` |
| 登録順 | `insertion_order_does_not_change_the_result`(720順列) |
| D0 の条件では従来と一致 | 上記の D0 側のアサート + IMPL-001 の fixture(全て合格) |
| `t < 1` でも予算を超えて消費しない | `others_never_spend_more_than_their_budget_when_t_is_below_one` |
| 保存則 | `fix1_identities_hold_across_the_gate` |

### 3.2 実データ E2E(`tests/p28_010_impl_002_ig_guarded_iron_priority_e2e_test.rs`、6件)

| テスト | 内容 |
|---|---|
| `d4_240_months_keeps_the_ledger_closed_and_matches_the_audited_outcomes` | 台帳の残差 < 1e-3、Dwarf・Elfin・Ferrowyn の改善、Passguard が D0 と一致、Arcadia 不変、WarId(8) が 1819/02/07 に決着 |
| `passguard_does_not_enter_the_d1_d3_starvation_oscillation` | D1/D3 の振動(Factory の Iron 2.5、MS 27〜153)が1〜250か月で再発しない(Factory の最小引落し > 5、MilitaryFactory の Iron 引落しと MS の最小/最大 > 0.7、税率反転 ≤ 8、破産月 ≤ 17) |
| `long_horizon_600_months_has_no_new_insolvency_or_tax_runaway` | 600か月: 新たな破産なし、税率 ≤ 0.25、税率反転の観測上限 |
| `player_country_uses_the_same_rule_as_ai_countries` | Dwarf をプレイヤーにして、AI のときと配分が同一であること、IG 十分/不足で Factory の取り分が切り替わること |
| `save_load_around_the_ig_gate_matches_continuous_run` | m98 と m264 で save→load を挟み、連続実行と国庫・税率・備蓄が一致 |
| `d4_is_deterministic_across_two_runs` | 130か月を2回(同一プロセス)で一致 |

旧コード(HEAD の `production.rs`)に対して実データ E2E を実行すると、D4 を検証する 3 件(`player_country_...`、`d4_240_months_...`、`long_horizon_...`)が失敗し、Passguard 系・決定性・save/load が成功する(D0 と D4 の差が実際にこれらのテストで検出できる)。

## 4. プレイヤー国家への適用

`process_country_production` は `handle_monthly_economy` が全国家に対して同じ関数で呼ぶ。国家IDの分岐・プレイヤー判定は存在しない。実データのテストで、Dwarf をプレイヤーにした場合と AI の場合で、同じ初期条件(Iron 15、IG 5 万 / 5)の初月の Factory・MilitaryFactory の Iron 引落しが一致することを確認した。プレイヤーが建てた Factory・MilitaryFactory にも、建物種別だけで同じ D4 の条件が働く。

## 5. 6か国の結果(自然運用、全6か国・全AI・プレイヤー操作なし、開始 1800/01/01、同一 HEAD 上の D0 と D4)

Division 日数あたりの補給不足率 = 補給充足率 < 0.34 の Division 日数 / Division 日数。MS・ME・IG は累計産出。LSw は人口加重生活水準。

### 5.1 90日・36か月

90日・36か月は、D0 と D4 が全6か国で**完全に同一**(Iron 備蓄が枯渇する前で、優先配分が成立しても結果が変わらない)。

### 5.2 120か月

| 国 | D0 | D4 |
|---|---|---|
| Arcadia | Div 2 / 不足率 90.3% / MS 652 / 国庫 4,580 | 同一 |
| Elfin | Div 3 / 32.2% / MS 3,605 / 6,270 | 同一 |
| **Dwarf** | Div 11 / 19.3% / MS 24,272 / IG 備蓄 5,074 / 国庫 15,176 | **Div 16 / 14.3% / MS 27,830 / IG 2,838 / 15,340** |
| Oceanic | Div 0 / 70.2% / 国庫 4,950 | 同一 |
| Passguard | Div 1 / 3.3% / MS 13,422 / 国庫 904 / LS 70.0 | 同一 |
| Ferrowyn | Div 2 / 30.1% / MS 6,077 / 686 | 同一 |

### 5.3 240か月

| 国 | D0 | D4 |
|---|---|---|
| Arcadia | Div 0 / 91.9% / 国庫 3,731 / 順位 5 | 同一(順位 4) |
| **Elfin** | Div 9 / 71.8% / MS 9,302 / 国庫 8,128 | **51.3% / MS 16,087 / 8,187** |
| **Dwarf** | Div 11 / 27.3% / MS 50,917 / IG 23,481 / 16,523 | **Div 16 / 18.8% / MS 73,702 / IG 9,425 / 16,462** |
| Oceanic | 70.2% / 国庫 3,351 / LS 70.3 | 国庫 2,217 / LS 74.3(戦争経過による) |
| Passguard | Div 1 / 1.6% / MS 27,387 / 国庫 1,842 / LS 70.0 | **完全に同一** |
| **Ferrowyn** | Div 7 / 20.8% / MS 19,211 / 国庫 4,359 / LS 68.4(LSw 67.0) | **Div 8 / 8.5% / MS 22,960 / 3,899 / 65.4(LSw 65.4)** |

### 5.4 360か月・600か月

| 国 | 時点 | D0 | D4 |
|---|---|---|---|
| Elfin | 360 | 77.0% / MS 14,767 / 国庫 8,101 / LS 67.8 | 41.2% / MS 38,440 / 8,172 / 78.6 |
| Elfin | 600 | 82.1% / MS 22,786 / 9,611 / LS 74.9 | 36.1% / MS 83,312 / 11,076 / LS 96.1 |
| Dwarf | 360 | Div 11 / 26.2% / MS 81,014 / 17,793 | Div 16 / 19.7% / MS 119,557 / 17,835 |
| Dwarf | 600 | Div 11 / 39.3% / MS 114,902 / 33,118 | Div 16 / **11.8%** / MS 234,169 / 35,207 |
| Passguard | 360 | 1.1% / MS 38,992 / 1,784 / LS 70.0 | 1.1% / MS 41,301 / 1,768 / LS 70.0 |
| Passguard | 600 | 0.7% / MS 56,832 / 2,146 / LS 70.0 | 0.7% / MS 68,556 / 2,136 / LS 70.0 |
| Ferrowyn | 360 | 49.3% / MS 25,058 / 4,830 / LS 80.2 | 27.5% / MS 38,865 / 5,978 / LS 75.8 |
| Ferrowyn | 600 | 66.5% / MS 33,358 / 6,010 / LS 91.7 | 39.0% / MS 67,969 / 6,537 / LS 85.0 |
| Arcadia・Oceanic | 360/600 | Arcadia MS 652(Iron 枯渇)、Oceanic MS 117(軍需工場なし) | Arcadia 同一、Oceanic は LS 76.7→100.0(軍需と無関係な世界経過。原因未確認) |

### 5.5 国力・順位・戦争

| 項目 | D0 | D4 |
|---|---|---|
| 戦争総数(120 / 240 / 360 / 600か月) | 7 / 9 / 9 / 9 | 7 / 9 / **10** / **10** |
| war8(Ferrowyn 対 Oceanic、1819/02/02〜) | 継続 | **1819/02/07 に決着**(Ferrowyn 勝利) |
| war5(Elfin+Dwarf 対 Arcadia、1806/03/02〜) | 継続(600か月時点も) | **継続**(PRE-003 の実測 D4 は 1823/03/05 に決着。§6) |
| 新規戦争 | なし | war9(1824/04/02 開戦、Ferrowyn 対 Oceanic) |
| 順位・Tier(360か月、Arcadia) | 4 / RegionalPower | **4 / RegionalPower**(PRE-003 の実測 D4 は 6 位) |
| 順位(Dwarf・Elfin・Ferrowyn) | 1 / 2 / 3 | 1 / 2 / 3(不変) |

### 5.6 Dwarf の Iron 枯渇前後(月次、D0 / D4)

| 月 | D0 Iron備蓄 / Factory引落し / MilitaryFactory引落し / MS月産 | D4 同 |
|---|---|---|
| m096 | 274 / 65.0 / 105.7 / 352 | 同一 |
| m099 | 89 / 64.9 / 105.6 / 352 | 109 / **44.8** / 105.6 / 352 |
| m100 | 51 / 58.9 / 87.5 / 292 | 109 / **3.4** / 105.6 / 352 |
| m120 | 49 / 58.6 / 49.9 / 166 | 100.5 / **0.0** / **110.6** / 369 |

D4 は Iron 枯渇の直前(m099)から Factory の取り分を減らし、MilitaryFactory の引落しを維持する(D0 は m100 以降 MS 月産が 352→166 へ低下)。MS の増産は募兵に部分的に吸収される(月間補給需要 330→480、Division 11→16)が、不足率は 19.3→14.3%(120か月)、26.2→19.7%(360か月)、39.3→11.8%(600か月)と改善する。

### 5.7 着工先

| 国 | 着工数(600か月) D0 → D4 | 備考 |
|---|---|---|
| Dwarf | Factory 61 → 61、MilitaryFactory 4 → 4 | 不変。着工先の差は出ない |
| Elfin | Factory 24 → **43**、Mine 5 → 4 | IG 備蓄が減ると AI が Factory を増設する(P26-008-C)。D4 期間中は優先配分で Iron を受け取れない期間がある |
| Oceanic | Factory 23 → **34** | 軍需工場なし。世界経過による |
| Ferrowyn | Factory 20 → 19 | |
| Passguard・Arcadia | 変化なし(Factory 4 / 22) | |

建設AI(`country_ai.rs`)は変更していない。着工数の差は、IG 備蓄・国庫・戦争経過の変化に対する既存の判断の結果である。

## 6. PRE-003 の D4 との差と、その原因

PRE-003 の一時パッチ(D4)は、他の Iron 消費者の稼働率を `min(t, 他の充足率)` で決めていた。実装した D4 は `他の充足率 × t` で決める。

### 6.1 なぜ変えたか

消費は `base × level × op`(`t` を掛けない)なのに、需要・充足率は `t` 込みの値で計算する(Step 1/2)。`op = min(t, sat)` で `sat < t` のとき、消費は予算の `1/t` 倍になる。優先配分では「他の消費者の予算 = 備蓄 − MilitaryFactory の予約」が小さく、この利得(1/t)が閉ループで 1 を超え、月ごとの Iron 備蓄が **2か月周期で振動**した(実測、PRE-003 の D4 を 600か月延長):

| 項目(Dwarf、600か月、税率の反転回数) | D0 | PRE-003 の D4(予算を超えて消費) | 実装した D4(`t × 充足率`) |
|---|---|---|---|
| 税率反転(100か月ごと) | 4 / 0 / 0 / 0 / 0 / 1 | 4 / 26 / 96 / 62 / 1 / 0 | 全期間で計5回 |
| 総計 | 5 | **189** | 5 |

m250 付近の PRE-003 版: 月ごとに Factory の引落しが 0 / 215、Iron 備蓄が 259 / 150 を往復し、維持費(稼働率に比例)の変動で収支が +525 / −210 を往復し、税率が 0.14 / 0.13 を毎月往復した。**国庫は常に正で、破産はなかった**が、依頼 §6 の「継続的な税率振動」に該当する。

### 6.2 修正の範囲

Iron のみ、他の消費者のみ、D4 が成立する月のみ。D0 の月、MilitaryFactory、Iron 以外の資源、閾値(3か月)は変更していない。「残りを共通の割合で按分する」という D4 の契約(PRE-003 §9)を、受け取った予算を超えて消費しない形で実装したもので、閾値や条件は調整していない。

### 6.3 PRE-003 の実測値との照合

| 項目 | PRE-003 の D4 | 実装した D4 | 最初の差 |
|---|---|---|---|
| Dwarf の 120 / 240 / 360か月の Division | 16 / 16 / 22 | 16 / 16 / **16**(600か月も 16) | 月 099(Factory の引落し 62.3→44.8)。以後の Division の増加が 22 に届かない |
| Dwarf の補給不足率(360か月) | 16.7% | 19.7%(D0 26.2%) | 同上 |
| Elfin・Ferrowyn・Passguard(120〜240か月) | — | PRE-003 とほぼ同一(Elfin 240か月 MS 16,086→16,087、Ferrowyn 22,874→22,960) | — |
| war5 の決着 | 1823/03/05 | **決着しない**(600か月時点も継続) | Dwarf の Division が 22 に達しない |
| Arcadia の順位(360か月) | 6 位 | **4 位**(D0 と同じ) | war5 が決着せず、後続の戦争連鎖(4件)が起きない |
| 戦争総数(360か月) | 13 | 10 | 同上 |

**PRE-003 で確認された「war5 の決着」と「Arcadia の順位低下」は、本実装では再現しない**。その2つは予算超過(1/t 倍)の副産物であり、補給改善そのものの効果ではなかった。補給改善の主効果(Dwarf・Elfin・Ferrowyn の不足率低下、Passguard の D0 一致)は再現した。ユーザーが承認した「Division増加・戦争の連鎖・Arcadia の順位変化」のうち、Division 増加(11→16)は発生し、戦争の連鎖と Arcadia の順位低下は発生しない(戦争は 9→10 件)。

## 7. Passguard と長期財政の判定(600か月まで実行)

### 7.1 D1/D3 の振動の不在

| 指標(Passguard、1〜250か月) | D1(PRE-003) | D4 |
|---|---|---|
| Factory の Iron 引落しの最小 | 1.5 | 11.4 以上(> 5 をテストで固定) |
| MS 月産の範囲 | 27〜153 | D0 と同一(92〜116) |
| 240か月の国庫・LS | 1,590 / 68.5 | **1,842 / 70.0(D0 と完全に同一)** |
| 破産月数 | 23 | 17(D0 と同じ。序盤のみ) |

### 7.2 600か月で見つかった新しい挙動(ゲートの月次切り替え)

| 国 | 税率反転 D0 → D4(600か月) | 内容 |
|---|---|---|
| Passguard | 5 → **39** | 月263以降(Factory Level が増えた後)と月491以降 |
| Elfin | 10 → 8 | 改善 |
| Dwarf | 5 → 5 | 不変 |
| Ferrowyn | 41 → 5 | 改善 |
| Arcadia | 3 → 3 | 不変 |

Passguard の 39 回の原因(月262〜282の月次データ): IG 備蓄が「MilitaryFactory の月間IG需要の3か月分」(約 65〜72)付近にあり、ゲートの上では優先配分で Factory が停止して IG が減り(F=0 / MF=46.7 / MS=156)、下では比例配分で Factory が動いて IG が増える(F=18.7 / MF=28 / MS=93)。3か月周期の切り替えで、維持費(稼働率に比例)の変動により月収支が −1 / +18 を往復し、税率が 0.19 / 0.20 を往復する。

- **振幅は 0.01、国庫は増加し続ける(600か月で 2,136。D0 は 2,146)、破産月は D0 と同じ 17、生活水準は不変(70.0)、MS は D0 比 +20.6%**。D1/D3 のような飢餓による MS 停止・財政悪化ではない。
- **これは閾値(境界)を持つゲートの構造的な性質**(ヒステリシスなし)であり、閾値の近傍に IG 備蓄が張り付く国で月ごとに現れる。p27_002_c のハーネス(月を30日単位で進める)では Dwarf でも 100〜120か月に 12回の往復が見られた(IG 備蓄 約 100〜160、ゲート約 120)。
- **閾値は調整していない**。切り替えを滑らかにする(ヒステリシス、もしくは連続的な配分)には、前月の状態を持つ必要があり、Save V1 に永続フィールドを追加できない制約と衝突する可能性がある。**ユーザー判断事項**として §10 に記載する。

### 7.3 判定

- D4 の契約は実装され、合格条件(保存則、Passguard の D0 一致、D1/D3 の振動の不在、新たな破産なし、税率の暴走なし)は満たした。
- 「継続的な税率振動」の再評価: Dwarf の 2か月周期(PRE-003 版の欠陥)は修正済み。Passguard 等のゲートの月次切り替えは**有界で無害**だが、存在する。完了判定は「**完了(既知の制約付き)**」とする。

## 8. Save・永続化

Save V1 を維持。新規の永続フィールド・migration・建設AIの判断理由は追加していない。D4 の判定は月初備蓄だけから導かれる派生値(状態なし)で、save→load を IG 閾値の前後(m98、m264)で挟んでも連続実行と一致する(国庫・税率・Iron・Coal・IG・MS・ME)。`saves/savegame_v1.ron` は読み書きしていない。

## 9. 既存テストへの影響(変更ごとの最初の分岐と因果)

| テスト | 変更 | 最初の分岐と因果 |
|---|---|---|
| `p26_det_001_..._test::test_e / test_f`、`p26_det_001_rev_001_..._test::test_h / test_i` | Arcadia 月24: 810.186777→827.770255、月36: 967.794283→1020.404523。Elfin(test_f/i): 499.874123→508.935259 | **Arcadia 月22**: 月21 の IG 備蓄 22.2 が3か月分以上、Iron 6.6 が不足 → MilitaryFactory が優先確保し、月22 の MS 産出が増える(月12・18 は不変)。Elfin は月36 から同様。他4か国は不変 |
| `p27_002_c_..._test::test_05` | 120か月の財政正規値(6か国)を更新。Dwarf の振動は「2回」→「有界性(≤12回、振幅 ≤0.05、国庫非負、最大税率 ≤0.25)」へ置換 | 最初の分岐: Arcadia 月22、Dwarf 月24、Ferrowyn 月24、Passguard 月25(Iron・IG は同じで収支・税率が分岐)、Oceanic 月27、Elfin 月36。Dwarf の12回は、IG 備蓄がゲート約120の近傍(100〜160)で月ごとに優先配分/比例配分を切り替えることによる(Factory 0 / 35、収支 +200 / −54、税率 0.16/0.17) |
| `p27_002_c_..._test::test_18` | Elfin (0.140, 3410.35, 10.33, 1)→(0.140, 3414.51, 10.33, 1)、Passguard (0.180, 921.35, 8.29, 3)→(0.170, 600.89, 3.98, 3)。開始時の領土の固定を追加 | 上と同じ(Elfin 月36、Passguard 月25) |
| `p28_010_impl_001_real_data_..._test` | 「Dwarf・Passguard が修正前と一致」→「Passguard は一致、Dwarf は D4(Div 16、不足 4,649、IG 2,838、国庫 15,340)」へ改名・更新 | Dwarf の最初の分岐は m099〜m100(Iron 枯渇) |

受け入れ条件の削除・緩和は上表のとおり(Dwarf の「0〜3回」の代理指標を、既に Oceanic に適用した有界性の条件へ置換。数値は実測の観測上限であり、目標値ではない)。他のテストは変更していない。

## 10. 残課題・ユーザー判断事項

| 項目 | 状況 |
|---|---|
| ゲートの月次切り替え(Passguard の税率反転 39 回、有界) | 既知の制約。解消にはヒステリシスなど状態を持つ設計が要る。**閾値の再較正や設計変更は別タスクとしてユーザー判断を要する** |
| PRE-003 の「war5 の決着・Arcadia の順位低下」が再現しない | 予算超過の副産物だった(§6)。承認済みの副作用のうち、戦争の連鎖と Arcadia の順位低下は発生していない |
| Oceanic の生活水準 100(600か月、D4) | 軍需工場なし。Iron・IG の値は D0 と同一で、世界経過による。原因未確認 |
| Passguard の月25 の分岐(p27 ハーネス) | 収支が分岐(同月の Iron・IG は同じ)。自然運用の実カレンダーでは 240か月まで D0 と一致 |
| 手動プレイの「約20%」 | 未確認 |
| UI 実ウィンドウ確認 | MANUAL VERIFICATION PENDING |
| P27-005 手動確認 | PENDING |
| P28-004(募兵制限案) | ユーザー判断待ち。再導入していない |

## 11. 実行コマンドと結果(今回の実測)

| コマンド | 結果(今回の実測、一時コードなし) |
|---|---|
| 関連テスト21本を個別に実行(p23_004、p26_003 ×3、p26_004 ×4、p26_008_c、p26_det_001 ×2、p27_002 a/b/c、p28_002、p28_005、p28_008、p28_010_impl_001 ×2、p28_010_impl_002 ×2)と `cargo test --lib economy::` | 全て 0 failed(7 / 49 / 56 / 21 / 19 / 11 / 15 / 22 / 18 / 10 / 12 / 17 / 18 / 20 / 3 / 3 / 3 / 11 / 3 / 12 / 6 passed、`economy::` 68 passed) |
| `cargo test --all-targets -j 2 --no-fail-fast` | **2836 passed / 0 failed / 1 ignored**(92 binaries。開始時の引用値 2811 に新規 25 を加えた値: fixture 12 + E2E 6 + 単体 7) |
| `cargo clippy --all-targets --all-features -- -D warnings` | **警告 0** |
| 独立した2プロセスで主要E2E(`p28_010_impl_002_..._e2e_test`)を再実行 | 2回とも 6 passed(正規値を固定したアサートが両プロセスで成立。同一プロセス内の2回実行の一致テストを含む) |
| 600か月の長期診断 | 実行した(E2E `long_horizon_600_months_...`、数値確認用の一時診断でも D0/D4 を比較) |

- 数値確認用の一時パッチ・一時テストは、最終検証前に全て削除した(`git status` は変更ファイルのみ)。
- 全体テストの途中で既知の非決定的PNG 9件が再生成された。`git status --short` で確認した9件を個別に `git checkout --` で復元した。
- 実装の途中で、中断したビルドの残骸によるリンクエラーが一度出たが、`cargo clean -p strategy_game` で解消した(コードの問題ではない)。
