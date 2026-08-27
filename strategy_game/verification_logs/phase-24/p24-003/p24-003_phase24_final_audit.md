この報告書はPhase 24の最終固定監査である。
P24-001の旧設計結論はP24-001-REV-001によって置換済みである。
P24-002の数値仕様はP24-002-REV-001によって訂正済みである。
戦死者とpopulationの連動は未実装ではなく、正式な保留仕様である。

# P24-003: Phase 24最終固定監査 報告書

## 1. 総合判定

**AUDIT COMPLETE, PHASE 24 READY**

Critical・High・未解決Mediumはいずれもゼロ。全テスト成功、clippy警告0。Phase 24の
正規仕様(人口不変・単一workforce正規関数・100万分率量子化・u128整数切り捨て・
Save V1維持・New Game/Loaded Game一致)と実コードが一致していることを、コード読解と
独立した数値検証の両方で確認した。

## 2. Phase 24固定可否

**固定可(READY)。** 既知の`workforce_ratio`明示validation不足はLowの追跡事項として
残るのみで、現在の全代入元(実データ・デフォルト値・テスト)は有効範囲内の値のみを
使っており、到達可能な無効経路は見つからなかった。次フェーズ開始を妨げない。

## 3. 開始時HEAD

```
1b4e8e38fa5b52ffa3646d635e83301a3dd577f3
```

想定と一致していた(P24-002-REV-001の訂正コミット)。

## 4. 監査対象コードHEAD

```
1b4e8e38fa5b52ffa3646d635e83301a3dd577f3
```

作業開始時点で他のコミットは存在せず、開始時HEADと同一。本タスクは報告書のみを
追加する読み取り専用監査であるため、監査対象コード自体はこのHEADのまま変化しない。

## 5. ユーザーセーブの保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージしていない。
作業開始時・`cargo test --all-targets`実行後・コミット直前のいずれの`git status
--short`でも` M saves/savegame_v1.ron`のみが残り続けたことを確認した(内容は
ユーザーの既存の未コミット差分のまま)。

## 6. Phase 24のコミット履歴

```
99bc1c1 docs: design casualty and population accounting          (P24-001)
d3347f7 docs: revise casualty and population accounting design   (P24-001-REV-001)
d4c397d refactor: unify workforce rounding semantics              (P24-002)
1b4e8e3 fix: use fixed-point workforce ratio arithmetic            (P24-002-REV-001)
```

いずれもrevert・amend・resetされておらず、監査履歴として完全な形で残っていることを
`git log --oneline`で確認した。

## 7. P24-001の最終的な扱い

**SUPERSEDED(現在有効な設計結論ではない)。** `Division.origin_state_id`を追加し
戦死者を出身州の人口から直接減算する案を検討していたが、多州国家での按分再計算・
二重控除・`target_state`の出身州転用・所有権変更時の扱い・旧Save互換性のいずれにも
未解決の問題があった。この報告書自体(`verification_logs/phase-24/p24-001/
p24-001_casualty_population_design_audit.md`)は監査履歴として削除・改変せず残して
いる(`git diff --stat`で無変更を確認済み、§39参照)。

## 8. P24-001-REV-001の正規結論

**現在有効。** 戦死者とpopulationの連動は当面実装せず、Phase 23の人的資源会計
(A・B・L・K・X)は一切変更しない。実データによる多州国家の反例(2州、各population=100、
B=1のとき、1州での死亡が別州への按分シフトを引き起こしcivilian_workforceが44→45へ
増加する)を実際の`apportion_mobilized_manpower`で再現し、P24-001の一般化された
「Bを触らなければ安全」という証明が単一州国家にしか成立しないことを確認した経緯を
踏まえた結論である。

## 9. P24-002の最終的な扱い

**COMPLETE WITH CORRECTION REQUIRED(P24-002-REV-001により訂正済み)。**
`StateData::total_workforce()`と`population::update::calculate_workforce()`を
`canonical_total_workforce`へ集約する構造自体は正しかったが、当初の実装
(`(population as f64) * (workforce_ratio as f64) as u64`)が`workforce_ratio: f32`の
丸め誤差をpopulation規模に応じて増幅する欠陥を持っていた
(`canonical_total_workforce(100, 0.45)`が数学的に正しい45ではなく44になる等)。

## 10. P24-002-REV-001の正規結論

**現在有効、Phase 24の最終workforce仕様。** ratioを100万分率へ量子化してから
`u128`中間値で乗除算する固定小数点方式(§13〜15)へ訂正し、`100 × 0.45 = 45`・
`100,000 × 0.45 = 45,000`を含む全境界値が数学的に正しい結果になることを確認した。

## 11. 戦死者とpopulationが未接続である証拠

`src/military/update.rs`(日次戦闘処理)・`src/state/ownership.rs`(州所有権変更)・
`src/country/elimination.rs`(国家消滅)のいずれにも`.population`への参照が一切
存在しないことを`grep`で確認した(該当なし、0件)。コードベース全体で
`origin_state_id`という識別子は1件も存在しない(`grep -r "origin_state_id" src/`が
0件)。`RecruitmentQueueItem.target_state`・`Division.target_state`(別概念、
移動先/配置先を表すフィールド)を出身州として人口減算に使うコードも存在しない。
戦死による`saturating_sub`ベースの人口減算処理も存在しない。§16の全経路調査
(population/growth.rsの月次成長のみが書き込み経路)と合わせ、戦死者とpopulationの
連動が実装されていないことを確認した。

## 12. population全書き換え経路

`grep`で`StateData.population`への代入パターン(`.population =`/`.population +=`/
`.population -=`)を全探索し、以下のみを確認した。

| ファイル | 内容 | 種別 |
|---|---|---|
| `src/population/growth.rs:50` | `grow_all_state_populations`、月次人口成長(`grow_state_population`、basis point固定小数点、0.05%/月) | 本番、唯一の書き込み経路 |
| `src/diplomacy/tests.rs:165` | テストフィクスチャ | テスト専用 |
| `src/ui/state_panel.rs:936, 954` | テストフィクスチャ | テスト専用 |

New Game初期化(`app/loader.rs`)・Saveロード(`save/`配下)には`.population`への
直接代入は存在せず(RONデシリアライズ/構造体構築時のフィールド初期化のみ)、
戦闘処理・補充・募兵・復員・国家消滅・州所有権変更のいずれにも population への
書き込みは存在しない。デバッグ処理にも該当する書き込みはない。

## 13. 正規workforce式

```
N = population (u64)
r = workforce_ratio (f32、想定範囲0.0〜1.0)
S = WORKFORCE_RATIO_SCALE = 1,000,000
Q = round(r × S)                              (量子化、ここでのみ四捨五入)
W = floor((N as u128) × (Q as u128) / S)      (人数算出、整数除算による切り捨て)
```

`src/state/data.rs`の`canonical_total_workforce`/`canonical_workforce_ratio_units`が
そのまま上式であることをコード読解で確認した(§14/§15/§17参照)。

## 14. ratio量子化式

```rust
pub fn canonical_workforce_ratio_units(workforce_ratio: f32) -> u64 {
    debug_assert!(
        workforce_ratio.is_finite() && (0.0..=1.0).contains(&workforce_ratio),
        "workforce_ratio must be finite and within [0.0, 1.0], got {workforce_ratio}"
    );
    ((workforce_ratio as f64) * (WORKFORCE_RATIO_SCALE as f64)).round() as u64
}
```

量子化段階でのみ`round()`を使用し、`f32`→`f64`変換の後にスケール乗算・四捨五入する。

## 15. `u128`整数演算

```rust
pub fn canonical_total_workforce(population: u64, workforce_ratio: f32) -> u64 {
    let ratio_units = canonical_workforce_ratio_units(workforce_ratio) as u128;
    ((population as u128) * ratio_units / (WORKFORCE_RATIO_SCALE as u128)) as u64
}
```

`population`と量子化済み`ratio_units`の乗算・除算はすべて`u128`中間値で行われ、
最終結果のみ`u64`へキャストする。`f32`とpopulationの直接乗算(`population as f32 *
workforce_ratio`のような精度不足の経路)、および`f32 → f64 → population乗算`の
経路(P24-002の欠陥そのもの)はいずれも存在しないことを確認した。epsilon補正
(`+ 0.000001`等)も存在しない。

## 16. 数値監査結果

一時テスト(監査終了後に削除済み、コミットに含めていない)で、実際の`canonical_
total_workforce`・`StateData::total_workforce()`・`population::update::
calculate_workforce`を直接呼び出し、以下の必須数値例をすべて確認した。

| population | ratio | expected | 実測値 |
|---:|---:|---:|---:|
| 0 | 0.45 | 0 | 0 |
| 1 | 0.5 | 0 | 0 |
| 2 | 0.5 | 1 | 1 |
| 10 | 0.1 | 1 | 1 |
| 99 | 0.45 | 44 | 44 |
| 100 | 0.45 | 45 | 45 |
| 101 | 0.45 | 45 | 45 |
| 100,000 | 0.45 | 45,000 | 45,000 |
| 1,000,000 | 0.333333 | 333,333 | 333,333 |
| 1,000,000 | 0.999999 | 999,999 | 999,999 |
| `u64::MAX` | 0.0 | 0 | 0 |
| `u64::MAX` | 1.0 | `u64::MAX` | `u64::MAX` |

全12件が一致した(既存の`tests/p24_002_workforce_rounding_unification_e2e_test.rs`の
`rev001_test_bc_required_numeric_table_verified_independently`が同じ表を恒久テストと
して既に固定している)。

## 17. population単調性

一時テストで、ratio ∈ {0.0, 0.000001, 0.1, 0.333333, 0.45, 0.5, 0.999999, 1.0}の
各値について、population 0..=1000および99,990..=100,010の全区間でworkforceが
非減少であることを確認した(全件通過)。既存の恒久テスト`test_d_workforce_is_
monotonically_non_decreasing_in_population`(p24_002 e2e、ratio 5値 × population
0..=1000)がこれを部分的に恒久検証している。

## 18. ratio単調性

同じ一時テストで、population ∈ {0,1,2,3,9,10,11,99,100,101,1000,100000}の各値に
ついて、上記8種類のratioを昇順に適用したときworkforceが非減少であることを確認した
(全件通過)。既存の恒久テスト`test_d2_workforce_is_monotonically_non_decreasing_
in_ratio`がこれを部分的に恒久検証している。

## 19. overflow検証

`canonical_total_workforce(u64::MAX, 0.0) == 0`・`canonical_total_workforce(u64::MAX,
1.0) == u64::MAX`・`canonical_total_workforce(u64::MAX - 1, 1.0) == u64::MAX - 1`の
いずれもpanicなく完了することを確認した(既存の恒久テスト`rev001_test_g_overflow_
safety_near_u64_max`が固定済み)。`u128`中間値は`u64::MAX`同士の乗算
(`(2^64-1)^2 ≈ 3.4 × 10^38`)でも`u128::MAX`(`≈ 3.4 × 10^38 + わずかな余裕`、
正確には`2^128-1 > (2^64-1)^2`)を超えないことを数学的に再確認した。

## 20. 州別workforce会計

`update_state_population_and_employment`(`src/population/update.rs`)を読み、以下を
確認した。

```
potential_workforce_s = canonical_total_workforce(population_s, workforce_ratio_s)
total_wf_s = potential_workforce_s.saturating_sub(mobilized_burden_s)
civilian_workforce_s = employed_workforce_s + unemployed_workforce_s = total_wf_s
```

- B=0(`mobilized_burden_s=0`)のとき: `saturating_sub(0)`は無変化のため
  `civilian_workforce_s == potential_workforce_s`(既存テスト`without_mobilization_
  employment_reaches_the_full_civilian_workforce`で確認)。
- B>0のとき: `saturating_sub`により`civilian_workforce_s <= potential_workforce_s`が
  常に成立(下限は0)。
- 州別負担が州のpotential workforceを超えた場合: `saturating_sub`がアンダーフロー
  せず0へ飽和する(既存テスト`a_burden_larger_than_the_workforce_clamps_employment_
  to_zero_without_underflow`で確認)。
- population=0: `canonical_total_workforce(0, r) == 0`のため`potential_workforce_s
  == 0`、civilian_workforceも0。
- ratio=0/ratio=1: §16の数値監査で確認済み。
- 同一入力での決定性: `canonical_total_workforce`は純粋関数(副作用・可変状態なし)
  であり、同一入力からは常に同一出力になることが構造的に保証される
  (`rev001_test_bc`等で実測確認済み)。

## 21. 国家別workforce会計

`economy::handle_monthly_economy`(`src/economy/mod.rs`)を読み、以下を確認した。

```
owned_states = state_registry.states.iter()
    .filter(|s| s.owner_country_id == country_id)   (現在の所有権で毎月再フィルタ)
total_wf(country) = owned_states.map(civilian_workforce).sum()
total_emp(country) = owned_states.map(employed_workforce).sum()
```

- 国家総potential workforceは、各州の`canonical_total_workforce`(正規値)を合算した
  ものであり(該当ループが各州を独立に計算・加算するのみで別の集計式を持たない)、
  州所有権変更後も`owner_country_id`によるフィルタが毎月再評価されるため、現在
  所有している国家の集計へ正しく反映される。
- 国家Bの州別按分合計: `apportion_mobilized_manpower`(§9参照、population基準の
  最大剰余法)の合計は通常ケースで`mobilized_manpower`と正確に一致する
  (既存テスト`state_totals_sum_exactly_to_the_original_mobilized_manpower`で
  確認済み)。
- 加算(合計)は可換なため、`state_registry.states`の走査順・HashMap内部順序に
  依存しない(§7同様、既存テスト`the_same_scenario_run_twice_independently_
  produces_identical_snapshots`が実App経路でこれを確認)。
- 他国のB混入なし: `owned_states`フィルタが`owner_country_id == country_id`である
  ため構造的に混入しない(既存テスト`one_countrys_mobilization_does_not_affect_
  another_countrys_employment`・`another_countrys_reinforcement_does_not_affect_
  this_countrys_civilian_workforce_next_month`で確認済み)。

戦死によるpopulation変化は§11で確認したとおり現在の正規経路には存在しないため、
P24-001が発見した「戦死州のcivilian workforceが44→45へ増える」反例は、現在の
正規経路(戦死→population不変)では発生し得ない。この反例はP24-001が検討していた
`origin_state_id`方式(population減算あり)固有の問題であり、その方式が採用されて
いない現在は再現条件自体が存在しない。

## 22. A・B・L・K・Xの正規式

`military::manpower::compute_manpower_ledger`(`src/military/manpower.rs`)を再読し、
以下を確認した(P24-002/P24-002-REV-001のいずれも`military/`配下のファイルを
一切変更していないため、Phase 23確定時点から無変更)。

```
A (available_manpower)         = CountryData.available_manpower(既存フィールド)
B (workforce_burden)           = CountryData.mobilized_manpower(既存フィールド)
L (living_military_manpower)   = 募兵キュー内の予約済み生存兵員 + 生存Divisionの現在manpower合計
K (permanent_military_losses)  = B.saturating_sub(L)
X (unaccounted_living_excess)  = L.saturating_sub(B)
```

## 23. 募兵・補充・戦死・復員の遷移

`military::`lib(177件)・`p23_006`〜`p23_009`(89件)が該当遷移をすべて実測しており、
本タスクでも全件が変更前と完全一致する結果で再合格した(§35/§36参照)。

- 募兵: Aが減少しBが増加する(`p23_009`)。
- 補充: Bが増加するがKは減らない(`p23_008`)。
- 戦死: Lが減少しKが増加する。B自体は戦死時に再利用可能な状態へ戻らない
  (`p23_006`)。戦死後にavailableが増加する経路は存在しない。
- 生存者復員: Bが減少しKは変化しない(`p23_007`)。戦死者(Kに計上済みの人員)は
  復員対象に含まれない(`demobilize_division`は生存Divisionのみを対象とする)。
- 国家間の混入なし: §21同様、所有国基準のフィルタにより構造的に防止。
- save/load後も帳簿一致: `p23_006`の`a_save_with_permanent_losses_reproduces_the_
  same_ledger_after_reload`等で確認。
- New Game/Loaded Gameで同一結果: `p23_010`の`new_game_gives_every_active_country_
  a_balanced_ledger`・`loaded_game_does_not_duplicate_divisions_or_trigger_
  immediate_ai_recruitment`で確認。
- HashMap順非依存: `p23_010`の`the_same_scenario_run_twice_independently_
  produces_identical_snapshots`で確認。
- 動員0でPhase 23経済挙動を維持: `p23_004`の`zero_mobilization_yields_a_healthy_
  workforce_ratio_and_positive_production`で確認(P24-002-REV-001後も無変更で
  合格)。

## 24. manpower capの意味

`population::growth::compute_manpower_cap(population) = population × 1,000 /
10,000`(basis points、population直接参照、`workforce_ratio`を一切経由しない
独立した計算式)であることをコードで確認した。

- `A+B<=cap`をhard invariantとして強制するコードは存在しない(Phase 23から
  無変更、`military::`lib 177件で確認)。
- capが縮小した場合(所有population減少時)でも既存のA・Bを没収する処理は
  存在しない。
- BまたはKがcapを超えても帳簿エラー(`ManpowerLedgerError`)にはならない
  (`compute_manpower_ledger`はcapを参照しない独立した計算)。
- capはpopulation保存則ではなく、`population::growth::recover_manpower`が
  availableをこのcapへ向けて緩やかに回復させる際の目標値(soft target)としてのみ
  機能する。
- 利用箇所: `population::growth`(A回復の目標値)、`military::ai`(AI募兵の
  目標師団数・予備manpower計算の基礎値)、`ui::military_panel`(表示用)の3箇所。
  いずれも`workforce_ratio`とは独立。
- population減少経路が現状存在しないこと(§12)と整合しており、capが時間とともに
  減少する現在の唯一の要因は州所有権の喪失(占領・割譲)のみである。

## 25. production・financeへの接続

`economy::production`・`economy::finance`はいずれも`StateData::total_workforce()`
(=`canonical_total_workforce`への委譲)を`calculate_economic_workforce_ratio`の
分母として参照しており(`src/economy/production.rs:49,84`・
`src/economy/finance.rs:90`)、別式での再計算は存在しない。`0.45 × 100 = 45`・
`0.45 × 100,000 = 45,000`は`total_workforce()`経由でそのまま経済入力へ反映される
(§16で直接確認済み)。浮動小数点補正・旧44/44,999へ戻す処理は存在しない
(§27の対象外確認と併せて確認)。New Game/Loaded Gameでの一致・save/load分岐一致は
既存の`rev001_test_h_save_load_branch_comparison_matches_production`
(total_workforce・食料生産量の一致)、および`p23_004`の`a_real_save_and_load_
round_trip_reproduces_the_same_economic_workforce_ratio`で確認済み(いずれも
本タスクで再合格)。

## 26. Save V1互換性

`SAVE_FORMAT_VERSION_V1: u32 = 1`(`src/save/dto.rs:45`)がSave関連の全モジュール
(`write.rs`・`dto.rs`・`validate.rs`・`apply.rs`・`export.rs`・`runtime.rs`・
`read.rs`)で一貫して使用されていることを確認した。V2やmigrationに関連する識別子は
存在しない。`StateData.workforce_ratio: f32`の型・`#[serde(default)]`属性は
Phase 24を通じて無変更。`origin_state_id`はStateData・Divisionのいずれにも存在
しない。

代表値の確認(population=100, workforce_ratio=0.45, expected total workforce=45)は
§16の数値監査、および既存テスト`test_i_save_load_branch_comparison_matches_at_
the_boundary_population`(civilian_workforce・workforce_ratio一致)・`rev001_
test_h_save_load_branch_comparison_matches_production`(total_workforce・
production一致)で、テスト専用の一時セーブファイルのみを使って確認済み
(ユーザーの`savegame_v1.ron`は使用していない)。

## 27. New Game／Loaded Game比較

既存の恒久テストが実App経路(`AppPlugin`ほか実プラグイン一式)で以下を検証しており、
本タスクで全件再合格した。

- `rev001_test_h_save_load_branch_comparison_matches_production`:
  total_workforce・食料生産量(finance/production代理指標)が継続実行(New Game
  相当)とsave→load→実行(Loaded Game相当)で完全一致。
- `test_i_save_load_branch_comparison_matches_at_the_boundary_population`:
  civilian_workforce・workforce_ratioが同様に完全一致。
- `p23_010::new_game_gives_every_active_country_a_balanced_ledger`・
  `loaded_game_does_not_duplicate_divisions_or_trigger_immediate_ai_recruitment`:
  A・B・L・K・Xの整合をNew Game/Loaded Gameの両方で確認。

`employed_workforce`/`unemployed_workforce`はNew Game生成時・Load時のいずれにも
明示的な初期化コードが存在せず(`#[serde(default)]`によりNew Gameでは0、Loadでは
保存値)、最初の月替わり(`economy::handle_monthly_economy`)で正規計算へ上書き
される。したがって、最初の月替わり「前」に限り、New Game(0)とLoaded Game(保存値)
のこれら2フィールドの値が一時的に異なり得る(`total_workforce()`・`population`・
`workforce_ratio`・`available_manpower`等の他の全フィールドはこの一時差異の対象
外)。この一時差異はPhase 23時点から存在する既知の仕様であり、UI上は月替わり前の
`employed_workforce`/`unemployed_workforce`の表示が理論上の初期値のままになり得るが、
`civilian_workforce()`はこの2フィールドの合計として表示専用に使われるのみで、
production/financeの実際の計算(`total_workforce()`を直接参照)には影響しない。
初回月次更新後は両経路が完全に収束する。

## 28. save/load分岐比較

§26/§27の各テストで、継続実行とsave→load→実行の分岐比較が civilian_workforce・
workforce_ratio・total_workforce・production食料生産量のいずれについても完全一致
することを確認した。

## 29. `workforce_ratio` validation評価

再調査の結果、P24-002-REV-001の報告時点から変化なし。

- 全代入元: `assets/data/states.ron`(28州、小数点以下2桁)、
  `state::data::DEFAULT_WORKFORCE_RATIO`(New Game既定値、0.45)、Saveロード
  (`#[serde(default)]`、代入元は上記のいずれか)、テストフィクスチャ(最大6桁)。
- UI・AI・イベント・政策・外部入力からの変更: いずれも不可能(`grep`で
  `.workforce_ratio =`等の代入パターンを全探索し、`Default`実装とRON
  デシリアライズ以外に該当なしを再確認。`src/ui/`配下は読み取り専用参照
  ・テストフィクスチャのみ)。
- NaN・infinity・負数・1.0超過の到達可能性: 上記のとおり具体的な到達経路は
  見つからなかった。

**Lowの追跡事項として据え置く。** READYを妨げるMedium以上への再分類は不要と判断
した(§33)。

## 30. Attritionへの引き継ぎ

- Attritionからpopulationを減らす処理: 存在しない(Attrition自体が未実装)。
- 一時的疾病・負傷をKへ混入させる処理: 存在しない。
- 将来の恒久Attrition損失がL/K会計経路を再利用する方針との矛盾: なし
  (現時点でAttritionのコードが一切存在しないため、矛盾しようがない——設計方針は
  P24-001-REV-001の報告書に記録済みで、コードによる先行実装や矛盾する布石は
  見つからなかった)。
- 人口連動: 引き続き`DESIGN PENDING`(P24-001-REV-001の決定どおり)。
- System順序(`DailySimulationSet`等)のPhase 24による先行変更: なし(Phase 24の
  2コミットはいずれも`src/state/data.rs`のみを変更しており、System登録・実行順序
  に関わるファイルは触れていない)。

## 31. Phase 24完了範囲

- 死傷者・人口連動案(`origin_state_id`方式)の問題点を監査し、多州国家反例で
  実証した。
- 誤った単一出身州方式を正式に撤回した(P24-001-REV-001)。
- 戦死者人口連動を安全に保留した(コード上の連動なし、§11/§12で確認)。
- Phase 23人的資源会計(A・B・L・K・X)を維持した(§22/§23で確認)。
- workforce丸め方式を統一した(P24-002)。
- 正規計算を`state::data::canonical_total_workforce`1か所へ集約した(§13〜15)。
- f32境界誤差を固定小数点方式(100万分率量子化+u128整数演算)で訂正した
  (P24-002-REV-001)。
- Save V1を維持した(§26)。
- New Game/Loaded Game整合を検証した(§27)。

## 32. 将来への保留事項

- 戦死者によるpopulation減算(`DESIGN PENDING`、P24-001-REV-001が将来の
  `N_s/D_s/H_s`州別明示会計案を設計スケッチとして記録)。
- 州別軍事人口(誰がどの州から動員されたかの明示的追跡)。
- Divisionの州別出身構成。
- Save V2・V1→V2 migration。
- 占領・併合後の軍事人口帰属ルール。
- Attrition人口連動。
- 負傷者・疾病。
- 移民・人口コホート(年齢構成等)。
- `workforce_ratio`の外部入力validation(§29、現状無害、到達経路なし)。

これらはPhase 24の未達として扱わない。実装済みであるかのようにも記載していない。

## 33. Critical／High／Medium／Low

- Critical: 0
- High: 0(P24-002の欠陥はP24-002-REV-001で既に解消済みであり、本監査時点では
  再発なし——過去の欠陥として§9/§10に記録するに留める)
- Medium: 0(未解決なし)
- Low: 1 — `workforce_ratio`の明示的validation不在(§29)。現在到達経路なし、
  READYを妨げない追跡事項。

## 34. 実行した全コマンド

```bash
git rev-parse HEAD
git status --short
git log --oneline -15

# 人口書き込み経路の調査
grep -rn "\.population\s*=\|\.population\s*+=\|\.population\s*-=" src/
grep -rn "origin_state_id" src/
grep -n "\.population" src/military/update.rs src/state/ownership.rs src/country/elimination.rs src/app/loader.rs src/save/

# 正規workforce関数の調査
grep -rn "canonical_total_workforce\|canonical_workforce_ratio_units\|total_workforce()\|calculate_workforce(" src/
grep -rn "workforce_ratio.*as f64\|population as f32.*workforce_ratio" src/

# 数値監査(一時テスト、監査後に削除)
cargo test --test zz_temp_p24_003_numeric_audit -- --nocapture   # 4 passed; 0 failed (required table / monotonicity / agreement / determinism)
rm tests/zz_temp_p24_003_numeric_audit.rs

# A/B/L/K/X・manpower cap・workforce_ratio validationの調査
grep -n "fn compute_manpower_ledger\|struct ManpowerLedger\|workforce_burden\|living_military_manpower" src/military/manpower.rs
grep -rn "fn compute_manpower_cap\|MANPOWER_CAP_RATIO_BP\|manpower_cap" src/
grep -n "workforce_ratio" src/ui/state_panel.rs

# Saveバージョンの調査
grep -rn "SAVE_FORMAT_VERSION" src/save/

# 必須テスト
cargo test --lib population::                                       # 46 passed; 0 failed
cargo test --lib military::                                         # 177 passed; 0 failed
cargo test --lib economy::                                          # 6 passed; 0 failed
cargo test --test economy_tests                                     # 22 passed; 0 failed
cargo test --test p23_002_mobilized_manpower_workforce_e2e_test     # 8 passed; 0 failed
cargo test --test p23_004_economic_workforce_ratio_e2e_test         # 7 passed; 0 failed
cargo test --test p23_006_manpower_ledger_e2e_test                  # 22 passed; 0 failed
cargo test --test p23_007_division_demobilization_e2e_test          # 24 passed; 0 failed
cargo test --test p23_008_daily_reinforcement_e2e_test              # 21 passed; 0 failed
cargo test --test p23_009_ai_recruitment_e2e_test                   # 22 passed; 0 failed
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test     # 9 passed; 0 failed
cargo test --test p24_002_workforce_rounding_unification_e2e_test   # 17 passed; 0 failed

cargo test --all-targets           # 1602 passed; 0 failed; 1 ignored
cargo clippy --all-targets --all-features -- -D warnings   # 0 warnings

git status --short   # 既知の非決定的PNG 9件のみ再生成、個別checkoutで復元
git checkout -- <9 screenshot paths individually>
git diff --stat verification_logs/phase-24/p24-002/p24-002_workforce_rounding_unification.md verification_logs/phase-24/p24-001/p24-001_casualty_population_design_audit.md verification_logs/phase-24/p24-001-rev-001/p24-001-rev-001_casualty_population_design_revision.md verification_logs/phase-24/p24-002-rev-001/p24-002-rev-001_workforce_fixed_point_revision.md
```

## 35. 各テストの正確な結果

| スイート | 件数 | 結果 |
|---|---|---|
| `cargo test --lib population::` | 46 | 全合格 |
| `cargo test --lib military::` | 177 | 全合格 |
| `cargo test --lib economy::` | 6 | 全合格 |
| `economy_tests` | 22 | 全合格 |
| `p23_002_mobilized_manpower_workforce_e2e_test` | 8 | 全合格 |
| `p23_004_economic_workforce_ratio_e2e_test` | 7 | 全合格 |
| `p23_006_manpower_ledger_e2e_test` | 22 | 全合格 |
| `p23_007_division_demobilization_e2e_test` | 24 | 全合格 |
| `p23_008_daily_reinforcement_e2e_test` | 21 | 全合格 |
| `p23_009_ai_recruitment_e2e_test` | 22 | 全合格 |
| `p23_010_phase23_manpower_lifecycle_audit_test` | 9 | 全合格 |
| `p24_002_workforce_rounding_unification_e2e_test` | 17 | 全合格 |
| `zz_temp_p24_003_numeric_audit`(一時、削除済み) | 4 | 全合格(required table / monotonicity+bound / agreement / determinism) |
| `cargo test --all-targets` | 全体 | **1602 passed / 0 failed / 1 ignored** |

## 36. 全テストのpassed／failed／ignored件数

```
1602 passed / 0 failed / 1 ignored
```

開始時基準(1602 passed / 0 failed / 1 ignored)と完全に一致。本タスクは恒久テストを
追加しないため、コード状態が同じであれば件数も同じになる想定どおりの結果であり、
差異は発生しなかった。

## 37. clippy警告数

```
0 warnings
```

(`cargo clippy --all-targets --all-features -- -D warnings`)

## 38. 変更したファイル

- `verification_logs/phase-24/p24-003/p24-003_phase24_final_audit.md`(新規、本書)

本番コード・既存テストコードは一切変更していない。一時的な検証用テスト
(`tests/zz_temp_p24_003_numeric_audit.rs`)は監査終了前に削除し、コミットへ
含めていない。

## 39. 対象外を先行実装していないこと

`git diff --stat`で以下の既存Phase 24報告書がいずれも無変更であることを確認した。

```
verification_logs/phase-24/p24-001/p24-001_casualty_population_design_audit.md
verification_logs/phase-24/p24-001-rev-001/p24-001-rev-001_casualty_population_design_revision.md
verification_logs/phase-24/p24-002/p24-002_workforce_rounding_unification.md
verification_logs/phase-24/p24-002-rev-001/p24-002-rev-001_workforce_fixed_point_revision.md
```

(いずれも差分ゼロ)。§32に列挙した将来への保留事項(戦死者population減算・
Attrition人口連動・州別軍事人口・Save V2等)について、本タスクを含むPhase 24の
いずれのコミットでも先行実装が行われていないことを、§12(population書き込み経路の
全数調査)・§26(Save V1のみが使用されている確認)と合わせて確認した。

## 40. コミット直前の`git status --short`

```
 M saves/savegame_v1.ron
```

(`saves/savegame_v1.ron`はユーザーの実セーブデータであり、本タスクでは一切読み書き
していない。既存の未コミット差分がそのまま残っているのみで、コミットには含めない。)
