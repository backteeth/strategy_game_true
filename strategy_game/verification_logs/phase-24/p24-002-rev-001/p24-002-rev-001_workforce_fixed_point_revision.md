# P24-002-REV-001: workforce固定小数点計算への訂正 実装・検証報告書

この報告書はP24-002の数値仕様を訂正する。
P24-002の計算集約構造は維持する。
P24-002の監査・実装履歴自体は削除または改変しない。

## 1. 総合判定

**COMPLETE**

P24-002が固定してしまっていた浮動小数点境界誤差(`population * workforce_ratio`の
素朴なf64乗算による誤差)を、ratioを100万分率へ量子化してから`u128`中間値で乗除算する
固定小数点方式へ訂正した。P24-002の「計算の真実源を1か所へ集約する」構造(`canonical_
total_workforce`への委譲)は維持している。

## 2. P24-002の判定

**COMPLETE WITH CORRECTION REQUIRED**

構造(真実源の集約)は正しかったが、採用した数値計算(`f64`乗算→`as u64`切り捨て)が
`workforce_ratio: f32`の丸め誤差をpopulationの規模に応じて増幅する欠陥を持っていた。
P24-002のコミット(`d4c397d1`)はrevert/amend/resetせず、そのまま履歴に残す。

## 3. 訂正後の判定

**P24-002-REV-001でDESIGN CORRECTED、実装完了。**

`state::data::canonical_total_workforce`は固定小数点方式へ置き換わり、以下の必須受け入れ
条件を満たす。

```
canonical_total_workforce(100, 0.45) == 45
canonical_total_workforce(100_000, 0.45) == 45_000
canonical_total_workforce(99, 0.45) == 44
```

## 4. 開始時HEAD

```
d4c397d1bf059dbcefa64bf07206944a0bcd3009
```

想定と一致していた。

## 5. 監査対象コードHEAD

```
d4c397d1bf059dbcefa64bf07206944a0bcd3009
```

作業開始時点で他のコミットは存在せず、開始時HEADと同一。

## 6. `f32 → f64 → floor`が不正確だった理由

P24-002の実装は次の式だった。

```rust
((population as f64) * (workforce_ratio as f64)) as u64
```

`workforce_ratio: f32`の`0.45`は2進浮動小数点で正確な0.45を表現できず、実際には
`0.449999988079071044921875`というわずかに小さい値として保持される。この誤差
(`0.45`との差は約`1.19 × 10^-8`)は非常に小さいが、`population`との乗算によって
populationの規模に比例して拡大される。

- `population=100`のとき: `100 × 0.449999988... = 44.9999988...` → 切り捨てで**44**
  (数学的に正しい45との差は1)。
- `population=100,000`のとき: `100,000 × 0.449999988... = 44,999.9988...` → 切り捨てで
  **44,999**(数学的に正しい45,000との差は1)。

誤差の絶対量はpopulationにほぼ比例して拡大するため、population規模が大きい実データ
(`assets/data/states.ron`は数十万〜百万人規模)では、この誤差が常態的に発生し得る
構造的な欠陥だった。

## 7. 100×0.45の変更前後

```
変更前(P24-002): canonical_total_workforce(100, 0.45) = 44
変更後(P24-002-REV-001): canonical_total_workforce(100, 0.45) = 45
```

## 8. 100,000×0.45の変更前後

```
変更前(P24-002): canonical_total_workforce(100_000, 0.45) = 44,999
変更後(P24-002-REV-001): canonical_total_workforce(100_000, 0.45) = 45,000
```

## 9. 採用した固定小数点スケール

```
WORKFORCE_RATIO_SCALE = 1,000,000 (100万分率、小数点以下6桁)
```

`workforce_ratio: f32`は0.0〜1.0付近で約7桁の有効精度を持つが、§13の調査により
ゲーム上意味を持つratioは`assets/data/states.ron`(小数点以下2桁)・デフォルト値
`0.45`・テスト用の最大6桁(`0.333333`・`0.999999`)までであり、6桁を超える精度が
必要な箇所は見つからなかった(詳細は§13/§14)。

## 10. ratio量子化式

```rust
pub fn canonical_workforce_ratio_units(workforce_ratio: f32) -> u64 {
    debug_assert!(
        workforce_ratio.is_finite() && (0.0..=1.0).contains(&workforce_ratio),
        "workforce_ratio must be finite and within [0.0, 1.0], got {workforce_ratio}"
    );
    ((workforce_ratio as f64) * (WORKFORCE_RATIO_SCALE as f64)).round() as u64
}
```

比率の量子化段階でのみ四捨五入する。`0.45f32`(実際は`0.449999988...`)は
`× 1,000,000`で`449999.988...`となり、四捨五入で正しく`450,000`へ復元される。

## 11. workforce算出式

```rust
pub fn canonical_total_workforce(population: u64, workforce_ratio: f32) -> u64 {
    let ratio_units = canonical_workforce_ratio_units(workforce_ratio) as u128;
    ((population as u128) * ratio_units / (WORKFORCE_RATIO_SCALE as u128)) as u64
}
```

人数の算出段階では四捨五入せず、整数除算による切り捨てのみを行う(P24-002が決定した
「人数は切り捨て」という仕様を維持)。量子化(比率側の四捨五入)と人数算出(切り捨て)を
明確に分離したことが、P24-002の欠陥を修正する核心である。

## 12. `u128`中間値を採用した理由

`population: u64`(最大`u64::MAX ≈ 1.8 × 10^19`)と`ratio_units: u64`(有効範囲では
最大1,000,000)の乗算は、`u64`の範囲(最大`≈ 1.8 × 10^19`)を超える可能性がある
(`u64::MAX × 1,000,000 ≈ 1.8 × 10^25`)。`u128`(最大`≈ 3.4 × 10^38`)であれば、
`u64::MAX`同士の乗算(`≈ 3.4 × 10^38`未満)でもoverflowしないため、中間値として
採用した。除算後の結果は有効なratio範囲(`0.0〜1.0`)では常に`population`以下になり、
`u64`へ安全に戻せる(§19で検証)。

## 13. ratio全代入元の調査結果

コード全体を調査し、`workforce_ratio`への代入元は以下のみであることを確認した。

| 代入元 | 精度 |
|---|---|
| `assets/data/states.ron`(28州の実データ) | 小数点以下2桁(例: 0.55, 0.60, 0.48, 0.70) |
| `state::data::DEFAULT_WORKFORCE_RATIO`(New Game既定値) | `0.45`(小数点以下2桁) |
| Saveからのロード(`#[serde(default)]`でV1互換) | 保存された値をそのまま復元、代入元は上記のいずれか |
| 各テストファイルのフィクスチャ | 最大でも小数点以下6桁(本タスクで追加した`0.333333`・`0.999999`) |

月次更新(`economy::handle_monthly_economy`)・政策/法律/イベント等が`workforce_ratio`を
書き換える箇所は存在しない(`grep`で`.workforce_ratio =`/`workforce_ratio *=`/
`workforce_ratio +=`の代入パターンを検索し、`Default`実装とRONデシリアライズ以外に
該当なしを確認)。`economy::production::calculate_operation_ratio`の引数名`workforce_ratio`
は`StateData.workforce_ratio`とは異なる概念(経済用労働力比率、`E/P`)であり、
混同していないことも確認した。

## 14. 6桁を超える精度の必要性

上記調査の結果、現在のゲーム仕様で小数点以下6桁を超える精度が必要な箇所は見つからな
かった。実データ(2桁)・デフォルト値(2桁)のいずれも100万分率(6桁)で完全に
復元される。したがって100万分率を正式な正規精度として採用した(`NEEDS DESIGN REVIEW`
に該当する事項はなし)。

## 15. validation確認結果

`StateData::sanitize_values()`(`src/state/data.rs`)は`education`・`living_standard`・
`unrest`・`logistics_capacity`・`logistics_usage`・`logistics_ratio`・
`base_logistics_capacity`をclampするが、`workforce_ratio`は対象に含まれていない。
`workforce_ratio`に対する明示的なvalidation(範囲チェック・NaN/infinity拒否)は
コードベース全体を通じて存在しない。

これはP24-002-REV-001が新たに発見した既存のギャップであり、§13で確認したとおり
現在のすべての代入元(RONデータ・デフォルト値・テスト)は`0.0〜1.0`の有効範囲内の
finiteな値のみを使っているため、無効値が本番経路へ到達する具体的な経路は見つかって
いない。したがって本タスクのスコープ(validation設計全体の変更は対象外、§6/§12参照)
には含めず、Lowとして報告するに留める(§28)。

`canonical_workforce_ratio_units`には`debug_assert!`を追加し、開発時(debug build)に
無効値が渡された場合は可視化されるようにした。release buildでは`f64::round`と
`as u64`の飽和キャスト(NaN・負数→0、極端な超過値→u64::MAX)により、無効値が渡っても
panicはしない(黙ってclampして正常値に見せかけることはしていない——値は「誤った
計算結果」になるが、これは意図的にvalidationを行っていないことの帰結であり、本関数が
独自に不正値を修復しているわけではない)。

## 16. `total_workforce()`と`calculate_workforce()`の一致

両関数とも`canonical_total_workforce`(またはそれが呼ぶ`canonical_workforce_ratio_units`)
へ委譲する構造はP24-002から無変更のため、自動的に一致し続ける。
`test_total_workforce_and_calculate_workforce_agree_on_every_boundary_input`(populations
0〜101 × ratios 0.0/0.1/0.45/0.5/1.0の全50組)で再検証し、全件一致を確認した。

## 17. population単調性

`test_d_workforce_is_monotonically_non_decreasing_in_population`(ratio 0.0/0.1/0.45/0.5/1.0
× population 0..=1000の全区間)で再検証し、populationの増加によってworkforceが
減少するケースは存在しないことを確認した。

## 18. ratio単調性

`test_d2_workforce_is_monotonically_non_decreasing_in_ratio`(population
0/1/2/3/9/10/11/99/100/101/1000 × ratio 0.0/0.1/0.3/0.45/0.5/1.0の全組み合わせ)で
再検証し、ratioの増加によってworkforceが減少するケースは存在しないことを確認した。
量子化段階の四捨五入(`round`)はratio自体に対して単調非減少であり、後段の整数除算
(定数分母での切り捨て)も分子に対して単調非減少であるため、合成された`canonical_
total_workforce`もratioに対して単調非減少であることが構造的に保証される。

## 19. overflow検証

`rev001_test_g_overflow_safety_near_u64_max`で以下を確認した。

```
canonical_total_workforce(u64::MAX, 0.0) == 0
canonical_total_workforce(u64::MAX, 1.0) == u64::MAX
canonical_total_workforce(u64::MAX - 1, 1.0) == u64::MAX - 1
canonical_total_workforce(u64::MAX, 0.5) == 独立したu128式の結果と一致、かつ population以下
```

いずれもpanic・wraparound・縮退なく完了した。§12で述べたとおり、`u128`中間値は
`u64::MAX`同士の乗算でもoverflowしない(数学的に`(2^64-1)^2 < 2^128-1`)。有効な
ratio範囲(`ratio_units <= 1,000,000`)では、除算結果は常に`population`以下になり
`u64`への最終キャストも安全である。

## 20. production・financeへの影響

`StateData::total_workforce()`(`production.rs`・`finance.rs`が参照)の実装自体が
`canonical_total_workforce`への委譲であるため、境界値が意図的に変わる。

| テスト名 | population | workforce_ratio | 変更前workforce | 変更後workforce | production/financeへの影響 | 変更理由 |
|---|---|---|---|---|---|---|
| `tests::test_calculate_workforce`(economy_tests.rs) | 100,000 | 0.45 | 44,999 | 45,000 | このテスト自体はproduction/finance計算を含まないため直接の影響なし。ただし`calculate_workforce`の戻り値が変わるため期待値を更新した | §6の固定小数点誤差訂正の直接的な結果 |
| `test_b_representative_value_matches_the_fixed_point_computation`(p24_002 e2e) | 100 | 0.45 | 44 | 45 | 同上(純粋関数レベル) | 同上 |
| `test_f_monthly_employment_update_uses_the_canonical_computation`(p24_002 e2e) | 100 | 0.45 | employed+unemployed=44 | employed+unemployed=45 | `update_state_population_and_employment`経由でemployed/unemployedの合計が1増加 | 同上 |
| `test_g_zero_mobilization_monthly_update_uses_the_canonical_fixed_point_value`(p24_002 e2e) | 100 | 0.45 | civilian_workforce=44 | civilian_workforce=45 | 実App経路(`economy::handle_monthly_economy`)で同様に1増加 | 同上 |
| `test_h_mobilized_case_stays_deterministic_on_the_unified_canonical_value`(p24_002 e2e) | 100 | 0.45 | civilian_workforce(動員10控除後)=34 | civilian_workforce(動員10控除後)=35 | 同上 | 同上 |
| `test_i_save_load_branch_comparison_matches_at_the_boundary_population`(p24_002 e2e) | 100 | 0.45 | civilian_workforce=44 | civilian_workforce=45 | 同上、save/load両分岐で一致 | 同上 |

経済式(`production.rs`・`finance.rs`・`calculate_economic_workforce_ratio`)自体は
一切変更していない。旧値へ戻す補正値やepsilonは追加していない。既存の
`p23_004_economic_workforce_ratio_e2e_test`(population=120,000、ratio=0.45→
`canonical=54,000`、変更前後で完全一致する境界)・`economy_tests.rs`の
production/welfare characterization test群(population=100,000、ratio=0.45→
`canonical=45,000`、これもP24-002の44,999からP24-002-REV-001の45,000へ変わったが
経済計算自体は元々許容誤差(`EPSILON=1e-3`)の範囲内に収まっていたため、これらの
テストの期待値(比率・生産量)は変更不要だった)は無変更のまま合格した。

## 21. 州別B按分への影響

**影響なし。** `population::mobilization::apportion_mobilized_manpower`は
`(StateId, population: u64)`のペアを直接受け取り、`workforce`を一切経由しない
(コード確認済み、`src/population/mobilization.rs`)。したがって本タスクの
固定小数点訂正は、州別B按分の計算式・結果のいずれにも影響しない。国家合計Bは
当然変更していない。

## 22. Phase 23会計を維持したこと

以下をすべて無変更で確認した(該当テストスイートが全件、変更前後で一致する結果で
合格したことによる)。

- A・B・L・K・Xの意味・計算式(`military::`lib 177件、`p23_006〜010`)は無変更。
- 募兵でBが増える(`p23_009`22件)、補充でBが増えるがKは減らない(`p23_008`21件)、
  戦死でLが減りKが増える(`p23_006`22件)、生存者復員でBが減る・戦死者は復員できない
  (`p23_007`24件)——いずれも無変更のまま合格。
- 戦死でpopulationは減らない(コード上、population書き換え箇所は依然として
  存在しないことを再確認)。
- manpower capはsoft target、`A+B<=cap`を強制しない(`military::`lib 177件)。
- AttritionとPopulationは接続していない。`target_state`を出身州として扱っていない
  (§21で確認したとおり`apportion_mobilized_manpower`はworkforceを経由しない独立した
  population按分のまま)。

## 23. New Game／Loaded Game比較

`rev001_test_h_save_load_branch_comparison_matches_production`で、New Game経路
(継続実行)とLoaded Game経路(save→load後)の`total_workforce()`(45、修正後の正しい値)
と食料生産量(`ResourceType::Food`のstockpile)が完全一致することを確認した。

## 24. save/load分岐比較

`test_i_save_load_branch_comparison_matches_at_the_boundary_population`(civilian_
workforce・workforce_ratioの一致)・`rev001_test_h_save_load_branch_comparison_
matches_production`(total_workforce・production食料生産量の一致)の両方で、継続実行と
save→load→実行が完全一致することを確認した。

## 25. Save V1を維持したこと

`StateData.workforce_ratio`の型は`f32`のまま変更していない。セーブフィールド構成・
`#[serde(default)]`属性は無変更。Save V2・migrationは導入していない。保存された`f32`を
ロードした後の量子化結果(`canonical_workforce_ratio_units`)・workforce算出結果が
save前と完全に一致することを上記save/loadテストで確認した。

## 26. 変更したファイル

本番コード:

- `src/state/data.rs` — `canonical_total_workforce`を固定小数点方式へ書き換え、
  新規`WORKFORCE_RATIO_SCALE`定数と`canonical_workforce_ratio_units`関数を追加。
  `population::update::calculate_workforce`は既にこの関数へ委譲する構造だった
  ため無変更。

テスト:

- `tests/economy_tests.rs` — `test_calculate_workforce`の期待値を`44_999`→`45_000`
  (数学的に正しい値)へ訂正。
- `tests/p24_002_workforce_rounding_unification_e2e_test.rs` — Test B/F/G/H/Iの
  境界値を44/34/44→45/35/45へ更新(Test Cの99×0.45=44は変更なし)。新規に
  `rev001_test_a`(ratio量子化表)・`rev001_test_bc`(§7必須数値例の独立検証)・
  `rev001_test_g`(overflow耐性)・`rev001_test_h`(save/load×production一致)の
  4件を追加。

報告書:

- `verification_logs/phase-24/p24-002-rev-001/p24-002-rev-001_workforce_fixed_point_revision.md`
  (本書)。

## 27. 対象外を先行実装していないこと

`git diff`・追加ファイル一覧を確認し、以下がいずれも実装されていないことを確認した。

`workforce_ratio`フィールドの固定小数点型化、Save V2、migration、戦死とpopulationの
連動、Attritionとpopulationの連動、Division出身州、州別軍事人口、B・L・K・Xの意味変更、
manpower capのhard cap化、production・finance式の再設計、人口成長率の変更、
UI・ローカライズ変更、AI判断変更、移民・負傷者・装備・補給関連の変更、P24-003以降の
先行実装、無効なratio全体のvalidation再設計。いずれも該当なし。

## 28. Critical／High／Medium／Low

- Critical: 0
- High: 1 — P24-002当初の`canonical_total_workforce`実装が、`workforce_ratio: f32`の
  丸め誤差をpopulationの規模に応じて増幅する構造的欠陥を持っており、実データ規模
  (数十万人)では常態的に1人単位の誤差を生んでいた。本タスクで固定小数点方式へ
  訂正し解消済み。
- Medium: 0
- Low: 1 — `workforce_ratio`に対する明示的なvalidation(範囲チェック)がコード
  ベース全体に存在しない。現在の全代入元は有効範囲内の値のみを使っており具体的な
  到達経路は見つからなかったため、本タスクのスコープには含めず報告のみ行う
  (§15参照)。

## 29. 実行した全コマンドと正確な結果

```bash
cargo build --lib
rustfmt --edition 2024 --check src/state/data.rs tests/p24_002_workforce_rounding_unification_e2e_test.rs tests/economy_tests.rs
rustfmt --edition 2024 src/state/data.rs tests/p24_002_workforce_rounding_unification_e2e_test.rs tests/economy_tests.rs  # フォーマット差分を適用
cargo test --test p24_002_workforce_rounding_unification_e2e_test  # 17 passed; 0 failed
cargo test --test economy_tests                                    # 22 passed; 0 failed
cargo test --lib population::                                      # 46 passed; 0 failed
cargo test --lib military::                                        # 177 passed; 0 failed
cargo test --lib economy::                                         # 6 passed; 0 failed
cargo test --test p23_002_mobilized_manpower_workforce_e2e_test    # 8 passed; 0 failed
cargo test --test p23_004_economic_workforce_ratio_e2e_test        # 7 passed; 0 failed
cargo test --test p23_006_manpower_ledger_e2e_test                 # 22 passed; 0 failed
cargo test --test p23_007_division_demobilization_e2e_test         # 24 passed; 0 failed
cargo test --test p23_008_daily_reinforcement_e2e_test             # 21 passed; 0 failed
cargo test --test p23_009_ai_recruitment_e2e_test                  # 22 passed; 0 failed
cargo test --test p23_010_phase23_manpower_lifecycle_audit_test    # 9 passed; 0 failed
cargo test --all-targets   # 1回目: clippyではなくビルドエラー(空行入りdoc commentでコンパイル失敗、後述)
cargo clippy --all-targets --all-features -- -D warnings  # 1回目: empty_line_after_doc_comments エラーで失敗
# ↑ tests/p24_002_...rs の孤立したdoc commentを通常コメントへ修正
cargo clippy --all-targets --all-features -- -D warnings  # 2回目: 0 warnings
cargo test --test p24_002_workforce_rounding_unification_e2e_test  # 修正後再実行: 17 passed; 0 failed
cargo test --all-targets   # 最終実行: 1602 passed; 0 failed; 1 ignored
git status --short
```

途中経過として、`tests/p24_002_workforce_rounding_unification_e2e_test.rs`へ追加した
2箇所の説明的doc comment(項目に紐づかない独立したメモ)がRustのコンパイルエラー
(`expected item after doc comment`)およびclippyエラー(`empty_line_after_doc_comments`)
を引き起こしたため、通常コメント(`//`)へ修正した。これはテストファイルの記述ミスで
あり、production/testロジック自体には影響しない。

## 30. コミット直前の`git status --short`

```
 M saves/savegame_v1.ron
 M src/state/data.rs
 M tests/economy_tests.rs
 M tests/p24_002_workforce_rounding_unification_e2e_test.rs
```

(`saves/savegame_v1.ron`はユーザーの実セーブデータであり、本タスクでは一切読み書き
していない。既存の未コミット差分がそのまま残っているのみで、コミットには含めない。)
