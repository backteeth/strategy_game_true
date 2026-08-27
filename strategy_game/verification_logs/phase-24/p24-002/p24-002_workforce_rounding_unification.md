# P24-002: workforce丸め方式の統一 実装・検証報告書

## 1. 総合判定

**COMPLETE**

州の潜在総労働力(`StateData::total_workforce()`と`population::update::calculate_workforce()`)
が使っていた2つの異なる丸め規則(切り捨て／四捨五入)を、切り捨てを正規とする1つの
純粋関数へ統一した。死傷者とpopulationの連動は実装していない。

## 2. 開始時HEAD

```
d3347f73ac3ee9325c80d7f70d2656ea0da743d9
```

（P24-001-REV-001の報告書コミット。想定と一致していた。）

## 3. 監査対象コードHEAD

```
d3347f73ac3ee9325c80d7f70d2656ea0da743d9
```

作業開始時点で他のコミットは存在せず、開始時HEADと同一。

## 4. P24-001-REV-001の結論を維持していること

以下をすべて確認した。

- 戦死による`StateData.population`減算は実装していない。
- Phase 23の人的資源会計A・B・L・K・Xの意味・計算式は一切変更していない。
- `RecruitmentQueueItem.target_state`を出身州として扱うコードは追加していない。
- `Division`へ`origin_state_id`を追加していない。
- Save V1のスキーマを変更していない(`StateData`のフィールド構成・`#[serde(default)]`は
  無変更)。
- Save V2・migrationは導入していない。
- AttritionとPopulationの接続は実装していない。
- 本タスクはworkforceの丸め方式統一だけに限定した。

## 5. 変更前の2つの丸め方式

- `StateData::total_workforce()`(`src/state/data.rs`): `((population as f64) *
  (workforce_ratio as f64)) as u64` — Rustの浮動小数点→整数キャストは0方向への
  切り捨て(かつ負値は0へ飽和)であるため、実質「切り捨て」。
- `population::update::calculate_workforce()`(`src/population/update.rs`):
  `((population as f64) * (workforce_ratio as f64)).round().max(0.0) as u64` —
  「四捨五入」。

両者は同じ「population * workforce_ratio」という概念(州の潜在総労働力)を計算して
いたが、丸め規則だけが異なっていた。

## 6. 最小不一致例

実際の関数で再現した(変更前の値)。

```
population = 1, workforce_ratio = 0.5
  total_workforce()      (floor) = 0
  calculate_workforce()  (round) = 1   ← 不一致
```

## 7. 正規の丸め規則

**切り捨て(floor)を正規規則として採用した。**

```rust
pub fn canonical_total_workforce(population: u64, workforce_ratio: f32) -> u64 {
    ((population as f64) * (workforce_ratio as f64)) as u64
}
```

これは変更前の`StateData::total_workforce()`が使っていた数値型(`f64`中間計算)・
乗算順序・キャスト方法をそのまま踏襲したものであり、P24-002のために新たな誤差を
導入していない。

**重要な副次的発見**: `workforce_ratio: f32`の`0.45`は2進浮動小数点で正確な0.45より
わずかに小さい値(`0.449999988...`)として保持されるため、`canonical_total_workforce
(100, 0.45)`は数学的な`100*0.45=45`ではなく`44`になる。同様に`canonical_total_workforce
(100_000, 0.45)`は`45_000`ではなく`44_999`になる。これはP24-002が導入した誤差ではなく、
`total_workforce()`が元から持っていた挙動をそのまま正規基準として採用した結果である
(§2で指示された「既存結果を正規基準とする」方針に従った)。

## 8. 正規計算を配置した場所

`src/state/data.rs`に自由関数`pub fn canonical_total_workforce(population: u64,
workforce_ratio: f32) -> u64`を追加した。

- `StateData::total_workforce(&self)`はこの関数へ委譲するよう変更した(戻り値は
  無変更)。
- `population::update::calculate_workforce(population, workforce_ratio)`もこの関数へ
  委譲するよう変更した(戻り値は切り捨てへ変更)。

配置理由: `population::update`は既に`use crate::state::data::StateData;`で
`state`モジュールへ依存しているため、正規計算を`state::data`側に置き
`population::update`がそれへ委譲する向きにすることで、新たな依存方向
(`state` → `population`)を作らずに済む。これにより循環依存を回避した。両関数の
公開シグネチャ・戻り値の型はどちらも変更していない。

## 9. 全呼び出し箇所の確認結果

`total_workforce()`の呼び出し箇所(本番コード):

- `src/economy/production.rs:49, 84`
- `src/economy/finance.rs:90`

いずれも`calculate_economic_workforce_ratio(employed_workforce, state.total_workforce())`
の形で使われており、`total_workforce()`自体の実装(切り捨て、既存のまま無変更)を
経由するため、production.rs・finance.rsの計算結果は本タスクによってビット単位でも
変化しない。

`calculate_workforce()`の呼び出し箇所(本番コード):

- `src/population/update.rs:71`(`update_state_population_and_employment`内、唯一の
  呼び出し箇所)。この経路だけが実際に丸め規則の変更(四捨五入→切り捨て)の影響を
  受ける。

テストからの直接呼び出し(`calculate_workforce`または`total_workforce()`)は
`tests/economy_tests.rs`・`tests/p23_001_population_manpower_e2e_test.rs`・
`tests/p23_002_mobilized_manpower_workforce_e2e_test.rs`・
`tests/p23_004_economic_workforce_ratio_e2e_test.rs`に存在する。p23_001/p23_002の
呼び出しは実際の関数を呼んで期待値を導出する形(自己整合)のため無修正で通過した。
economy_tests.rsの1件のみ、関数の戻り値をハードコードした数値と直接比較していたため、
本タスクによる意図した挙動変化(§7参照)により失敗し、修正した(§22参照)。

## 10. `employed`・`unemployed`との関係

`update_state_population_and_employment`(`src/population/update.rs`)の計算式は
無変更のまま(式自体は変更していない、内部で呼ぶ`calculate_workforce`の丸め規則
だけが変わった):

```
total_wf = calculate_workforce(population, workforce_ratio) [今回から切り捨て] - mobilized_burden
employed = min(total_wf, total_required_workforce)
unemployed = total_wf - employed
```

したがって`employed_workforce + unemployed_workforce == canonical_total_workforce
(population, workforce_ratio) - mobilized_burden`(`mobilized_burden <=
canonical_total_workforce`のとき)が常に成立する。Test Fで直接検証した。

## 11. `civilian_workforce()`との関係

`civilian_workforce() = employed_workforce + unemployed_workforce`(定義は無変更)。
§10の関係により、`civilian_workforce()`は統一後の正規値(切り捨て)から動員負担を
控除した値と一致する。Test F・Test G・Test Hで実App経路を含めて確認した。

## 12. production・financeへの影響

**影響なし。** `production.rs`・`finance.rs`はどちらも`state.total_workforce()`を
経由しており、`total_workforce()`自体の実装(切り捨て)はP24-002前後で完全に同一
(delegateしただけ)。`p23_004_economic_workforce_ratio_e2e_test`(7件)・
`economy_tests.rs`のproduction/welfare characterization test群は、
`test_calculate_workforce`の1件を除き全てP24-002前後で無変更のまま合格した。

## 13. manpower capへの影響

**影響なし。** manpower capは`A + B <= cap`のsoft targetであり(`military::manpower`)、
`workforce`とは独立した計算系である。`military::`配下のlibテスト(177件)・
`p23_006`〜`p23_010`のe2eテスト群はすべてP24-002前後で結果が完全一致した。

## 14. New Game／Loaded Game比較

`StateData.employed_workforce`・`unemployed_workforce`は、New Game生成時
(`app/loader.rs`)にもLoad時(`save`モジュール)にも明示的な初期化・再計算処理が
存在せず、`#[serde(default)]`によりNew Gameでは0、Loadでは保存値がそのまま入る。
どちらの経路でも、その後の最初の月替わり(`economy::handle_monthly_economy`)で
`update_state_population_and_employment`が呼ばれ、その時点の正規計算(切り捨て)へ
上書きされる。したがって「同じ州状態(population・workforce_ratio)」であれば、
New GameかLoaded Gameかに関わらず、最初の月替わり後は同一の結果になる。
`p23_010`の`new_game_gives_every_active_country_a_balanced_ledger`・
`loaded_game_does_not_duplicate_divisions_or_trigger_immediate_ai_recruitment`は
P24-002前後で無変更のまま合格した。

## 15. save/load分岐比較

Test I(`test_i_save_load_branch_comparison_matches_at_the_boundary_population`)で、
境界population(100・workforce_ratio=0.45、正規値44)について「継続実行」と
「save→load→実行」の`civilian_workforce()`・`workforce_ratio`が完全一致することを
確認した(両方とも44、`workforce_ratio`もbit-for-bitで往復)。Saveスキーマは
変更していない。

## 16. 州単位の不変条件(§6要求分)

| # | 不変条件 | 検証テスト |
|---|---|---|
| 1 | `total_workforce()`と`calculate_workforce()`が常に一致 | `test_total_workforce_and_calculate_workforce_agree_on_every_boundary_input` |
| 2 | 同じ入力から同じ結果 | `test_repeated_calls_are_deterministic` |
| 3 | population増加でworkforceが減少しない | `test_d_workforce_is_monotonically_non_decreasing_in_population` |
| 4 | ratio増加でworkforceが減少しない | `test_d2_workforce_is_monotonically_non_decreasing_in_ratio` |
| 5 | population=0でworkforce=0 | `test_boundary_conditions` |
| 6 | ratio=0でworkforce=0 | `test_boundary_conditions` |
| 7 | ratio=1でworkforce=population | `test_boundary_conditions` |
| 8 | 有効範囲でworkforceがpopulationを超えない | `test_e_workforce_never_exceeds_population_for_valid_ratios` |
| 9 | 複数回計算しても結果不変 | `test_repeated_calls_are_deterministic` |
| 10 | New Game/Loaded Gameで同じ州状態なら同じ結果 | §14の分析＋`p23_010`既存テスト |

## 17. 国家単位の不変条件

| # | 不変条件 | 検証結果 |
|---|---|---|
| 1 | 国家総潜在労働力=所属州の正規値の合計 | `economy/mod.rs`は各州の`total_workforce()`/`civilian_workforce()`を個別に参照するのみで、国家単位の合計を別途キャッシュ・再計算していないため、合計は自動的に各州の正規値の総和になる(コード上単一のfor loop構造で確認) |
| 2 | 動員0で国家民間労働力=総潜在労働力 | `p23_004::zero_mobilization_yields_a_healthy_workforce_ratio_and_positive_production`、Test G |
| 3 | 動員ありで既存の決定的按分を維持 | `apportion_mobilized_manpower`は`population`を直接受け取り`workforce`を経由しないため無変更(`p23_002`全8件が無変更のまま合格)。Test H |
| 4 | HashMap反復順に非依存 | Test H(2つの独立したApp実行が同一結果) |
| 5 | 他国の人口・B・L・Kが混入しない | `p23_002::one_countrys_mobilization_does_not_affect_another_countrys_employment`、`p23_004::one_countrys_mobilization_does_not_affect_another_countrys_production_or_welfare`(両方無変更のまま合格) |

## 18. Phase 23互換性

以下をすべて無変更で確認した(該当テストスイートが全件、変更前後で一致する結果で
合格したことによる)。

- 募兵でBが増加する(`military::`lib 177件、`p23_009`22件)
- 補充でBは増えるがKは減らない(`p23_008`21件)
- 戦死でLが減少しKが増加する(`p23_006`22件)
- 戦死でpopulationは減少しない(コード上、`process_daily_battles`等が
  `StateData.population`を書き換える箇所は存在しないことを確認済み——P24-001-REV-001
  時点から変更なし)
- 生存者復員でBが減少する(`p23_007`24件)
- 戦死者は復員できない(`p23_007`24件)
- manpower capはsoft target、`A+B<=cap`を強制しない(`military::`lib 177件)
- population増加時のavailable manpower回復規則(`population::growth`)は無変更
  (`population::`lib 46件)

## 19. 変更したファイル

本番コード:

- `src/state/data.rs` — `canonical_total_workforce`を新規追加、`total_workforce()`を
  それへ委譲するよう変更(戻り値は無変更)。
- `src/population/update.rs` — `calculate_workforce()`を`canonical_total_workforce`へ
  委譲するよう変更(戻り値の丸め規則が四捨五入→切り捨てへ変更)。

テスト:

- `tests/economy_tests.rs` — `test_calculate_workforce`の期待値を、正規化(切り捨て)に
  伴う意図した変化に合わせて`45_000`→`44_999`へ更新(§22参照)。
- `tests/p24_002_workforce_rounding_unification_e2e_test.rs`(新規) — Test A〜Iの13件。

報告書:

- `verification_logs/phase-24/p24-002/p24-002_workforce_rounding_unification.md`(本書)。

## 20. 対象外を先行実装していないこと

`git diff`・追加ファイル一覧を確認し、以下がいずれも実装されていないことを確認した。

戦死によるpopulation減算、Attritionによるpopulation減算、Divisionの出身州、
`origin_state_id`、州別人的資源由来構成、州別軍事人口ストック、Save V2、
V1→V2 migration、B・L・K・Xの意味変更、manpower capのhard cap化、AI募兵判断の変更、
UI・ローカライズ追加、production・finance式の再設計、`workforce_ratio`の型変更、
人口成長率の変更、移民・負傷者・装備・補給関連の変更、P24-003以降の先行実装。
いずれも該当なし。

## 21. 実行した全コマンド

```bash
cargo build --lib
rustfmt --edition 2024 --check src/state/data.rs src/population/update.rs \
  tests/p24_002_workforce_rounding_unification_e2e_test.rs
cargo test --lib population::
cargo test --lib military::
cargo test --lib economy::
cargo test --test p23_002_mobilized_manpower_workforce_e2e_test
cargo test --test p23_004_economic_workforce_ratio_e2e_test
cargo test --test p23_006_manpower_ledger_e2e_test
cargo test --test p23_007_division_demobilization_e2e_test
cargo test --test p23_008_daily_reinforcement_e2e_test
cargo test --test p23_009_ai_recruitment_e2e_test
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test
cargo test --test p24_002_workforce_rounding_unification_e2e_test
cargo test --test economy_tests   # test_calculate_workforce修正の確認のため個別再実行
cargo test --all-targets          # 1回目(修正前、economy_tests::test_calculate_workforceが失敗)
cargo test --all-targets          # 2回目(修正後、全件成功)
cargo clippy --all-targets --all-features -- -D warnings
git status --short
```

## 22. 各テストの正確な結果

| スイート | 件数 | 結果 |
|---|---|---|
| `cargo test --lib population::` | 46 | 全合格(変更前と完全一致) |
| `cargo test --lib military::` | 177 | 全合格(変更前と完全一致) |
| `cargo test --lib economy::` | 6 | 全合格(変更前と完全一致) |
| `p23_002_mobilized_manpower_workforce_e2e_test` | 8 | 全合格(変更前と完全一致) |
| `p23_004_economic_workforce_ratio_e2e_test` | 7 | 全合格(変更前と完全一致) |
| `p23_006_manpower_ledger_e2e_test` | 22 | 全合格(変更前と完全一致) |
| `p23_007_division_demobilization_e2e_test` | 24 | 全合格(変更前と完全一致) |
| `p23_008_daily_reinforcement_e2e_test` | 21 | 全合格(変更前と完全一致) |
| `p23_009_ai_recruitment_e2e_test` | 22 | 全合格(変更前と完全一致) |
| `p23_010_phase23_manpower_lifecycle_audit_test` | 9 | 全合格(変更前と完全一致) |
| `p24_002_workforce_rounding_unification_e2e_test`(新規) | 13 | 全合格 |
| `economy_tests`(全体) | 22 | 1回目: `test_calculate_workforce`が`left: 44999, right: 45000`で失敗。原因は§7の
意図した丸め規則変更。§19の修正後、2回目は22件全合格 |
| `cargo test --all-targets`(1回目、修正前) | 全体 | 1265 passed / **1 failed** / 0 ignored(失敗は上記`test_calculate_workforce`のみ) |
| `cargo test --all-targets`(2回目、修正後) | 全体 | **1598 passed / 0 failed / 1 ignored** |

## 23. 全テストのpassed／failed／ignored件数(最終)

```
1598 passed / 0 failed / 1 ignored
```

P24-001-REV-001時点の基準(1585 passed / 0 failed / 1 ignored)に対し、P24-002の新規
テストファイル13件を加えた数(1585+13=1598)と正確に一致する。

## 24. clippy警告数

```
0 warnings
```

(`cargo clippy --all-targets --all-features -- -D warnings`)

## 25. Critical／High／Medium／Low

- Critical: 0
- High: 0
- Medium: 1 — `tests/economy_tests.rs::test_calculate_workforce`が、丸め方式統一に
  伴う意図した挙動変化(§7・§9)により期待値`45_000`のまま失敗した。§19の修正
  (`44_999`へ更新、理由をコメントに明記)で解消済み。新たな不具合ではなく、統一の
  直接的な結果として想定内。
- Low: 0

## 26. コミット直前の`git status --short`

```
 M saves/savegame_v1.ron
 M src/population/update.rs
 M src/state/data.rs
 M tests/economy_tests.rs
?? tests/p24_002_workforce_rounding_unification_e2e_test.rs
```

(`saves/savegame_v1.ron`はユーザーの実セーブデータであり、本タスクでは一切読み書き
していない。既存の未コミット差分がそのまま残っているのみで、コミットには含めない。)
