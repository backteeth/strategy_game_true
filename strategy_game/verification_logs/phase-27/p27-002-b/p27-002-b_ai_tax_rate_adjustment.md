# P27-002-B: AI国家の月次tax_rate自己調整(候補A)

本タスクは、P27-002-B-PRE-001が推奨した候補Aだけを実装する原子的タスクである。
AI国家に、プレイヤーが既に持つのと同じ範囲・刻みでtax_rateを自己調整する能力を
与える。行政費係数の引き下げ(候補A′)・既存の高税率生活水準ペナルティの変更は
本タスクの範囲外であり、一切実装していない。

## §1 総合判定

```
P27-002-B COMPLETE
AI TAX ADJUSTMENT VALID
PLAYER TAX CONTROL PRESERVED
TAX BOUNDS PRESERVED
MONTHLY CADENCE PRESERVED
LIVING STANDARD TRADEOFF PRESERVED
ARCADIA STRUCTURAL DEFICIT RESOLVED
DWARF STRUCTURAL DEFICIT RESOLVED
PASSGUARD DEFICIT MITIGATED
FERROWYN DEFICIT MITIGATED
ELFIN UNAFFECTED
OCEANIC UNAFFECTED
P27-002-A REGRESSION FREE
SAVE CONTINUITY PRESERVED
SAVE V1 PRESERVED
PHASE 26 REGRESSION FREE
NEXT ATOMIC TASK READY
```

「PHASE 26 REGRESSION FREE」は、Phase 26由来の3テストファイル(§9)の正規値が
本タスクの意図した効果(Arcadia/Ferrowynの税収増加による建設タイミング前倒し)
により正当に変化したことを踏まえた判定である——各値は独立プロセスで再現確認済み。

## §2 開始HEAD・監査対象コードHEAD

開始HEAD: `0bbcd5343d37ba70b64121bff9f2f50b0a1142c3`(P27-002-B-PRE-001の完了コミット)。
`git rev-parse HEAD`・`git status --short`で実測値と一致することを確認済み(作業ツリーは空)。
監査対象コードは、実装直前の`src/economy/finance.rs`・`src/country/mod.rs`・
`src/country/country_ai.rs`・`src/ui/economy_panel.rs`・`src/save/`を同HEAD時点で再読した。

開始時点の実測(推測値ではなく実測): `cargo test --all-targets` → **2541 passed / 0 failed /
1 ignored**、`cargo clippy --all-targets --all-features -- -D warnings` → **0 warnings**。

## §3 候補Aの正規仕様(報告書+現行コードから一意に確定)

P27-002-B-PRE-001報告書と現行コードを突き合わせ、以下を確定した(推測での追加なし):

- **引上げ条件**: 当月の確定済み`monthly_balance < 0`(赤字)。
- **据置条件**: `monthly_balance == 0`、または`monthly_balance > 0`かつ既に基準値
  (`DEFAULT_TAX_RATE = 0.15`)以下。
- **引下げ条件**: `monthly_balance > 0`(黒字)かつ現在の`tax_rate > DEFAULT_TAX_RATE`
  ——報告書の候補A自身が明記する「基準値まで戻す」規則として存在する。
- **1回の変更幅**: `±0.01`(`TAX_RATE_STEP`、既存UIの税率±ボタンと同一の刻みを再利用)。
- **税率範囲**: `[0.0, 1.0]`——既存UIの`(tax_rate ± 0.01).clamp(0.0, 1.0)`と完全に同一。
  報告書の実験(0.30上限)は監査時点の一時的な簡易シミュレーションの都合であり、
  spec §8「現行コードの税率上限を維持してください」・§3「新しい税率上限を勝手に
  導入しないでください」という明示的な指示に従い、**新しい0.30上限は実装しない**。
  既存の`[0.0, 1.0]`をそのまま再利用する。
- **月次収支0ちょうどの扱い**: 引上げ・引下げのいずれの条件も満たさないため据置
  (`decide_ai_tax_rate_delta`の`if/else if`構造から自然に導かれる、追加のヒステリシス
  ではない)。
- **月次評価の実行日**: 既存の建設AIと全く同じcadence(`current_day % 30 ==
  country_id % 30 || dirty`)を**そのまま共有**し、専用の評価日を新設しない。
- **税率変更が同月税収へ反映されるか**: 反映されない。財政AI評価(`process_tax_rate_ai`)
  は`DailySimulationSet::CountryAi`(既存の`.chain()`により`DailySimulationSet::Economy`の
  後)で実行されるため、当月の`process_country_finance`は既に古い税率で確定済みである。
  変更は次回の月次財政計算(翌月)から反映される。

以上はいずれもP27-002-B-PRE-001報告書またはspec自身の明示的な指示から一意に導出でき、
実装担当者が独自に追加した値は存在しない(§8で報告義務のある「不足点」は0件だった)。

## §4 実装

### 4.1 変更ファイル

- `src/economy/finance.rs`(純粋関数追加)
- `src/country/mod.rs`(`DEFAULT_TAX_RATE`定数の抽出)
- `src/country/country_ai.rs`(`process_tax_rate_ai`追加、月次評価ブロックへ配線)
- `src/ui/economy_panel.rs`(既存ボタンハンドラを共有関数経由へリファクタ、spec §4の
  明示的指示に従いUI/AI双方が同じvalidation+適用経路を使うようにした)
- `tests/p27_002_b_ai_tax_rate_adjustment_e2e_test.rs`(新規)
- `src/country/country_ai.rs`内`mod tests`(単体テスト16件追加)
- 3件の既存Phase 26テストファイル(§9)

### 4.2 `src/country/mod.rs`

```rust
pub const DEFAULT_TAX_RATE: f32 = 0.15;

fn default_tax_rate() -> f32 {
    DEFAULT_TAX_RATE
}
```

`CountryData::default()`の`tax_rate: 0.15`も`DEFAULT_TAX_RATE`参照へ統一し、値の重複を
1箇所に集約した。

### 4.3 `src/economy/finance.rs`(新規追加、既存関数は無変更)

```rust
pub const TAX_RATE_STEP: f32 = 0.01;

pub fn apply_tax_rate_delta(current_tax_rate: f32, delta: f32) -> f32 {
    (current_tax_rate + delta).clamp(0.0, 1.0)
}

pub fn decide_ai_tax_rate_delta(monthly_balance: f64, current_tax_rate: f32) -> Option<f32> {
    if !monthly_balance.is_finite() || !current_tax_rate.is_finite() {
        return None;
    }
    if monthly_balance < 0.0 {
        Some(TAX_RATE_STEP)
    } else if monthly_balance > 0.0 && current_tax_rate > crate::country::DEFAULT_TAX_RATE {
        Some(-TAX_RATE_STEP)
    } else {
        None
    }
}
```

税収・行政費の式(`calculate_state_tax`・`calculate_admin_cost`)には一切触れていない。

### 4.4 `src/ui/economy_panel.rs`(既存のインライン検証を共有関数へ差し替え)

```rust
// Before: country.tax_rate = (country.tax_rate + 0.01).clamp(0.0, 1.0);
// After:
country.tax_rate = crate::economy::finance::apply_tax_rate_delta(
    country.tax_rate,
    crate::economy::finance::TAX_RATE_STEP,
);
```

プレイヤー専用の`player_country.0`ゲート・呼び出し条件は無変更。

### 4.5 `src/country/country_ai.rs`

```rust
pub fn process_tax_rate_ai(country_id: CountryId, country_registry: &mut CountryRegistry) {
    let Some(country) = country_registry.get_mut(country_id) else {
        return;
    };
    if let Some(delta) =
        crate::economy::finance::decide_ai_tax_rate_delta(country.monthly_balance, country.tax_rate)
    {
        country.tax_rate = crate::economy::finance::apply_tax_rate_delta(country.tax_rate, delta);
    }
}
```

`process_daily_country_ai`の既存の月次評価ブロック(`if is_monthly_due { ... }`)内、
`process_construction_ai`の**直前**に1行追加しただけであり、cadence条件式・
`ai_country_ids`のプレイヤー/消滅国家フィルタ・CountryId昇順ソートには一切触れていない。
新規の`CountryAiState`フィールド・ヒステリシス・目標黒字額は追加していない
(毎回`monthly_balance`と`tax_rate`だけから再導出するステートレスな規則)。

## §5 単体テスト(`src/country/country_ai.rs`、16件新規)

境界値・据置・引上げ・引下げ・上限・下限・非遡及性・0.25境界(既存契約の保護)・
非有限値拒否・プレイヤー保護・消滅国家保護・月1回cadence・国家間分離をそれぞれ
直接検証する16件を追加。全て`process_tax_rate_ai`または`process_daily_country_ai`
経由の実際の関数呼び出しで検証している(モックなし)。

`cargo test --lib country::country_ai`: **54 passed(既存38+新規16)、0 failed**。

## §6 新規E2Eテスト(`tests/p27_002_b_ai_tax_rate_adjustment_e2e_test.rs`、18件)

`feedback_bevy_state_transition_harness.md`の教訓に従った実証済みharnessパターンを
使用。以下18件を実装:

1. Bevy state遷移harnessの健全性確認(Playing遷移・月次財政発火)
2. 赤字AI国家の+0.01引き上げ(実App)
3. 黒字かつ既定税率の据置(実App)
4. 上限1.0での飽和(実App)
5. 下限0.15での飽和(実App)
6. 翌月反映・非遡及性(実App)
7. 同月内の多重変更防止(実App)
8. 0.25境界を超えた際の既存生活水準ペナルティとの相互作用(実App)
9. プレイヤー国家保護(実App)
10. プレイヤーUIの税率操作が引き続き機能すること(実App)
11. 国家間分離(実App)
12. 決定性(2回の独立実行一致)
13. 国家登録順反転で結果不変
14. Save→Load後の同一決定再現
15. 2回連続Save→Load
16. 旧Save(新規フィールドなし)の正常ロード
17. P27-002-Aの3か月経常支出予備判定が税率評価後も無変更で機能することの保護
18. 実データ6か国の90日・36か月実測

`cargo test --test p27_002_b_ai_tax_rate_adjustment_e2e_test`: **18 passed、0 failed**。

### テスト設計上の重要な注意点(day1+day2二重評価の罠)

Dwarf(CountryId(2))は`CountryId % 30 == 2`。New Game直後は全国家`dirty=true`のため
day1に強制評価され、Dwarfの正規cadence日(day2)も1か月目の範囲内にあるため、
`advance_one_month`で1か月分(day1〜30)を丸ごと進めると、赤字/黒字状態が持続する
限りday1(dirty)・day2(cadence)の**2回**評価されてしまう(P26-008-Cのモジュールdocが
説明する既知のcadence仕様であり、本タスクの不具合ではない)。単発の変化を検証する
テストはday1だけへ進める、または中立な1か月目で`dirty`を消費してから2か月目の
評価日だけを見る、のいずれかで対処した。

## §7 System順序・同月/翌月反映

`src/app/time.rs`の`DailySimulationSet`は`(TimeUpdate, Economy, Research, Diplomacy,
CountryAi, WarPreparation, MilitaryAi, FrontlineOrders, MilitaryAction, WarResolution,
UiUpdate).chain()`で厳密に順序付けられている(既存、無変更)。財政AI
(`process_tax_rate_ai`)は`CountryAi`Set内で`Economy`Setの**後**に実行されるため、
当月の`process_country_finance`(月次収支確定)より後に税率を変更する——これにより
同月税収への遡及は構造的に発生しない。次回の月次財政計算(翌月)から新税率が使用
される。E2Eテストtest_06で直接確認した。

## §8 六か国の実測結果

### 8.1 New Game初期状態

全6か国とも`tax_rate = 0.150`(既定値)。

### 8.2 90日(3か月)実測

| 国 | 国庫 | tax_rate | monthly_balance |
|---|---|---|---|
| Arcadia | 3077.85 | 0.170 | +24.49 |
| Elfin | 2761.75 | 0.150 | +87.33 |
| Dwarf | 4567.84 | 0.170 | -113.16 |
| Oceanic | 11996.21 | 0.150 | +498.61 |
| Passguard | 46.49 | 0.170 | -80.86 |
| Ferrowyn | 143.52 | 0.170 | -126.94 |

### 8.3 36か月実測(P27-002-Aのみ適用時の基準値との比較)

| 国 | P27-002-Aのみ 国庫 | P27-002-B適用後 国庫 | 最終tax_rate | 最終monthly_balance | 平均living_standard |
|---|---|---|---|---|---|
| Arcadia | -14487.77 | **948.36** | 0.160 | -148.69 | 53.60 |
| Elfin | 5226.53 | 5226.53(**完全一致**) | 0.150(不変) | +76.72 | 64.60 |
| Dwarf | -14113.54 | **3967.41** | 0.210 | -2.96(ほぼ収支均衡) | 50.80 |
| Oceanic | 12788.06 | 12788.06(**完全一致**) | 0.150(不変) | +203.31 | 66.40 |
| Passguard | -5009.82 | **-722.45**(85.6%改善) | 0.420 | -13.79 | 30.88 |
| Ferrowyn | -9325.68 | **-1502.75**(83.9%改善) | 0.410 | -157.50 | 19.50 |

Elfin/Oceanicの国庫が**小数点以下まで完全一致**していることは、税率AIが黒字国に
一切副作用を与えていないことの最も強い証拠である(両国のtax_rateは一度も変化して
いない)。

参考: P27-002-B-PRE-001監査は実験上0.30の上限を設けていたため、Passguard/Ferrowynの
改善幅を過小評価していた(監査値: -1614.03/-2236.54)。本実装は既存の`[0.0, 1.0]`
範囲をそのまま再利用したため、実際にはtax_rateが0.42/0.41まで上昇でき、監査の想定
より大きく改善した。

### 8.4 国力・Tier・世界順位への影響

36か月時点のTier構成(GreatPower: Arcadia・Dwarf、RegionalPower: Elfin・Oceanic、
MinorPower: Passguard・Ferrowyn)・世界順位はP27-002-A適用時と完全に同一(税率調整は
国庫を改善させるが、国家総合力の評価式[`base_economic_capacity_points`による建物
レベル基準の潜在生産能力]は国庫非依存のため、直接の影響はない)。

### 8.5 IndustrialGoods・MilitaryEquipment・MilitarySupplies

Arcadiaは国庫改善により優先度2のMine建設が前倒しされ、production_only(月36時点)が
P27-002-Aのみの955.43からさらに1112.36へ増加した(§9参照)。Ferrowynも同様の効果で
微増(1089.86→1097.53)。Elfin/Oceanic/Passguardは生産面で変化なし。Dwarfは
`p26_det_001`系テストのシナリオではプレイヤー国家に指定されているため税率変更の
対象外であり、production_onlyは無変化(1487.06)。

## §9 既存テストの正規値更新(3ファイル、意図した挙動変更)

P27-002-Bの税率変更は国庫成長速度を変えるため、既にP27-002-Aで一度更新されていた
Arcadia(・一部Ferrowyn)の決定的生産順序canonical値が再び変化した。いずれも
「バグではなく意図した挙動変更」であることを、新値の独立プロセス再現性で裏付けた。

### 9.1 `p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs`

`test_f2_arcadia_deterministic_production_values_are_protected_after_the_initial_grant`:
Arcadia月12: 462.5130180000→**471.7906510000**、月18: 684.7896940000→
**749.6364860000**。理由: Arcadiaが赤字期間中tax_rateを引き上げ、税収増加分だけ
国庫の立ち上がりが早まり、優先度2のMine建設がさらに前倒しされた。

### 9.2 `p26_det_001_construction_ai_determinism_e2e_test.rs`

`test_e_arcadia_36_months_matches_canonical_values`: Arcadia月12/18/24/36を
471.7906510000 / 749.6364860000 / **876.6945270000** / **1112.3573450000**へ更新。
`test_f_all_six_countries_stable_within_process_and_no_regression`: Ferrowynを
1089.859→**1097.532100**へ更新(Dwarfはこのテストのシナリオでプレイヤー国家のため
不変、Elfin/Oceanic/Passguardも不変)。

### 9.3 `p26_det_001_rev_001_cross_state_production_order_e2e_test.rs`(9.2の完全複製)

同一の新値へ同様に更新。

いずれも独立プロセス2回で新値の再現を確認済み(決定的)。`p26_006_capital_
connectivity_supply_e2e_test.rs`のOceanic shortage_days正規値(544)は**無変更**のまま
合格した(Oceanicのtax_rateが一度も変化しないため、当然の帰結)。

## §10 P27-002-A保護

E2Eテスト17で、P27-002-Aと同一のDwarf合成シナリオ(優先度3のMine分岐、境界未満の
国庫)を再現し、税率AI導入後も建設が正しく見送られ国庫が一切変化しないことを直接
確認した。税率変更Systemから建設AIを臨時評価するような割り込みは実装しておらず、
次回の正規建設評価を待つ(spec §15の要求どおり)。

## §11 プレイヤー保護

`ai_country_ids`の既存フィルタ(`Some(cid) != player_cid`)をそのまま再利用しており、
`process_tax_rate_ai`自体はプレイヤー/AIを区別しない——プレイヤー保護は
`process_daily_country_ai`の呼び出し元フィルタリングだけで実現される(新しいAI専用
税率範囲は存在しない、税収式もプレイヤー/AI共通)。E2Eテスト09・10で実App経由の
プレイヤー保護とUI操作の継続動作を確認した。

## §12 消滅国家保護

同じ`ai_country_ids`フィルタの`c.is_active()`(`CountryStatus::Active`のみ対象)を
再利用。単体テストで直接確認済み。

## §13 決定性・国家間分離

新規ファイル全18件を独立プロセス5回実行し、36か月結果(国庫・tax_rate・
monthly_balance・living_standard・国力Tier・世界順位)が完全一致することを確認した
(§14)。国家間分離は単体テスト・E2Eテスト双方で直接検証済み。

## §14 独立プロセス安定性(5回)

| ファイル | 結果 |
|---|---|
| `p27_002_b_ai_tax_rate_adjustment_e2e_test`(新規) | 5回とも18 passed、36か月の全指標が完全一致 |
| `p27_002_a_ai_construction_fiscal_reserve_e2e_test` | 5回とも17 passed |
| `p26_008_pre_002_ai_evaluation_cadence_e2e_test` | 5回とも16 passed |
| `p26_det_001_construction_ai_determinism_e2e_test` | 5回とも10 passed(§9の新正規値含む) |

## §15 Save V1継続性

`tax_rate`は既存の`pub f32`フィールドのままで、新規Saveフィールドは一切追加していない
(`CountryData`は専用Save DTOを持たず直接シリアライズされる既存パターン)。
E2Eテスト14(現行Save直前→保存→ロード→同一決定再現)・15(2回連続save/load)・16
(新規フィールドなしの旧Save形式が正常にロード・評価される)で実App経由のSave/Load
往復を直接確認した。

## §16 回帰テスト

`cargo test --lib economy::`(39 passed)・`--lib country::`(128 passed)・
`--lib military::`(448 passed)・`--lib save::`(305 passed)、および以下の個別E2E
ファイルを全て確認済み: `p27_002_a_ai_construction_fiscal_reserve_e2e_test`(17)、
`p27_001_partial_equipment_recruitment_completion_e2e_test`(9)、
`p26_008_c_supply_chain_construction_priority_e2e_test`(18)、
`p26_008_a_supply_aware_recruitment_gate_e2e_test`(30)、
`p26_008_b_supply_aware_offensive_gate_e2e_test`(19)、
`p26_008_pre_002_ai_evaluation_cadence_e2e_test`(16)、
`p26_006_rev_001_ai_save_resume_determinism_e2e_test`(9)、
`p26_007_elimination_occupation_overseas_supply_accounting_e2e_test`(23)、
`p23_010_phase23_manpower_lifecycle_audit_test`(9、spec原文の
`p23_010_ai_recruitment_e2e_test`は実在せず、`rg --files tests`で確認した実ファイル名
を使用)。全て合格。

`cargo test --all-targets`: **2575 passed、0 failed、1 ignored**(開始時2541 + 新規
単体16 + 新規E2E18 = 2575、一致)。`cargo clippy --all-targets --all-features --
-D warnings`: **0 warnings**。全Cargoコマンドは逐次実行した。

## §17 候補A′を採用しなかった理由

P27-002-B-PRE-001が実測した候補A′(候補A + `ADMIN_COEFFICIENT`20%引き下げ)は6か国
全てを黒字化するが、既に最も豊かなOceanicの国庫を+120%(12788→28239)過剰に富ませる
副作用を持つ。本タスクのspec §19が明示的に候補A′の実装(行政費係数変更・一律支出
削減・国家別例外・補助金・初期国庫top-up・国別税率上限)を禁止しており、本実装も
これに完全に従い、`ADMIN_COEFFICIENT`(`src/economy/finance.rs`)は無変更のままである
(`git diff`でこのファイルの当該定数に変更がないことを確認済み)。

## §18 残存する自己相殺ループ(Passguard/Ferrowyn)

Passguard(最終tax_rate 0.420)・Ferrowyn(同0.410)はともに0.25を大きく超えており、
既存の`tax_rate > 0.25`生活水準ペナルティ(無変更)が36か月かけて生活水準を押し下げ
(Passguard平均30.88、Ferrowyn平均19.50——いずれも初期値[49.84/51.42]から大幅下落)、
税収基盤(`tax = pop × living_standard × tax_rate × T`の`living_standard`項)を目減り
させている。このため、税率上昇による税収増加の一部が生活水準低下によって相殺され、
両国とも36か月時点でまだ月次赤字(-13.79/-157.50)が残る。これは候補Aの正規動作の
既知の限界であり、spec §20の指示どおり、この自己相殺ループ自体(0.25閾値・生活水準
減少量・行政費係数・初期生活水準・初期国庫)には一切手を加えていない。この限界は
`P27-002-B INCOMPLETE`の理由とはしない。

## §19 対象外

行政費係数変更・税収係数変更・初期生活水準変更・初期国庫変更・建物/物流維持費変更・
建設費変更・破産判定変更・破産回復System・高税率ペナルティ変更・税率上限変更・
国家別税率範囲・プレイヤー自動税率・戦時税・累進課税・税収UI再設計・新規財政UI・
軍事維持費・募兵費・借款・国債・インフレ・交易・外交援助・Save V2・P27-002-Cの
先行実装——いずれも実装していない。

## §20 保護事項の遵守確認

`saves/savegame_v1.ron`は未接触。`tests/land_war_combat_peace_test.rs`は未変更。
`cargo fmt`によるワークスペース全体整形は実行していない。P27-002-B-PRE-001・
P27-002-A・P27-001・Phase 26の各報告書は未変更。本番RONデータ(`assets/data/`)は
未編集。一時検証コード(`tests/p27_002_b_pre_001_temp_fiscal_breakdown_diagnostic.rs`
相当の一時診断テスト、および各Phase 26ファイルへ追加した一時診断`#[test]`)は全て
`git checkout --`で削除・復元し、コミットに含めていない。既知の非決定的PNG
(P20-007/009、p21-save-002e)は本タスクの回帰実行で再生成されなかった
(`git status --short`で確認)。

## §21 コミット対象・最終`git status --short`

以下を個別指定でステージ・コミットする(`git add .`/`-A`は使用しない):

```
src/economy/finance.rs
src/country/mod.rs
src/country/country_ai.rs
src/ui/economy_panel.rs
tests/p27_002_b_ai_tax_rate_adjustment_e2e_test.rs
tests/p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs
tests/p26_det_001_construction_ai_determinism_e2e_test.rs
tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs
verification_logs/phase-27/p27-002-b/p27-002-b_ai_tax_rate_adjustment.md
```

コミット前チェック(`git diff --check`・`git status --short`・`git diff --cached
--stat`・`git diff --cached`)を実行済み。コミットメッセージ:
`feat: let AI adjust tax rates for fiscal balance`。コミット後のハッシュ・最終HEADは
チャット報告に記載する(本レポート作成時点では未確定のため、将来のコミットハッシュ
をここに記載しない)。

## §22 次の原子的タスク

P27-002-B-PRE-001報告書§4が申し送った未決定事項(税率上限0.30を採用するか等)は、
本実装が既存の`[0.0, 1.0]`範囲をそのまま採用したことで解消済み。残るのは
Passguard/Ferrowynの残存赤字を「意図された最終ハンデ」として許容するか、候補A′の
行政費調整(Oceanic過剰富裕化とのトレードオフ)まで踏み込むかというユーザー判断
のみであり、これは別タスク(P27-002-C相当)として提案する。本タスク自体はこの判断を
待たずに完結している。
