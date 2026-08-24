# P23-003 完了報告: 人口・人的資源・動員負担のUI可視化と表示整合

## 1. 総合判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

自動テスト・検証コマンドはすべて成功したが、実ウィンドウでの目視確認(`cargo run`)は
本セッションでは実施していない(§7参照)。

## 2. 基準状態

- 開始時HEAD: `5f0054d17e1378df4f1373e755f150921ccfbd75`
  (`feat: apply mobilized manpower to civilian labor`、P23-002完了コミット)
- 完了時HEAD: 本報告のコミット(§10参照。この報告作成時点ではまだコミット前)
- ユーザーの既存未コミット変更(`saves/savegame_v1.ron`)は本タスク開始前から存在する
  実プレイデータであり、一切参照・変更・ステージしていない。

## 3. 実装内容

### 3.1 州パネル(`src/ui/state_panel.rs`)

`state_panel.info`の表示に以下を追加・修正した(既存パネルへの統合、新規パネルは作らず)。

- `{workforce}`: 従来は`StateData::total_workforce()`(人口×workforce_ratioから動員を
  考慮せず再導出する潜在労働力)を表示していたが、`StateData::civilian_workforce()`
  (新規、`employed_workforce + unemployed_workforce`。月次雇用計算が確定させた
  動員控除後の実値)へ差し替えた。
- `{employment_rate}`: 新規。`population::update::calculate_employment_rate(employed,
  civilian_workforce)`(新規の純粋関数)を使い、`雇用者 / 動員控除後の民間労働力`を
  百分率表示する。分母0では0を返す(0除算・NaN無し)。
- `{mobilized_burden}`: 新規。選択中州の所有国の`mobilized_manpower`を、
  P23-002の既存按分関数`population::mobilization::apportion_mobilized_manpower`へ
  現在の所有州人口をそのまま渡して呼び出し、結果から該当州の値だけを取り出す。
  UI側で最大剰余法を複製していない。所有国が見つからない異常系では0。

### 3.2 国家情報(`src/ui/military_panel.rs`)

`update_military_panel_ui`の合成テキストに以下を追加した。

- 新規行`military_panel.population`: 現在の所有州人口合計
  (`population::growth::compute_population_by_owner`、P23-001の既存Domain APIを再利用)。
- `military_panel.manpower`行に人的資源の基準上限`{cap}`を追加
  (`population::growth::compute_manpower_cap`、同じく既存Domain APIを再利用)。
  `available`/`mobilized`は既存表示のまま維持し、意味を混同しない3項目構成にした。

人口の桁数表示は`state_panel::format_population`(既存のK/M省略表示関数)を再利用し、
新規の独自フォーマット規則は作っていない。`available`/`mobilized`/`cap`は本パネルの
既存慣習(`military_panel.recruit_cost`等)に合わせ、非省略の生の整数表示のままとした。

### 3.3 `civilian_workforce`と`employment_rate`

- `StateData::civilian_workforce()`(`src/state/data.rs`、新規): `employed_workforce
  .saturating_add(unemployed_workforce)`。既存の`total_workforce()`は削除・改名せず
  そのまま残した(finance/productionが引き続き使用するため、§3.4参照)。
- `population::update::calculate_employment_rate(employed, civilian_workforce) -> f32`
  (新規純粋関数): 分母0では0.0を返す。`calculate_workforce_satisfaction`(需要0なら
  100%扱い)とは意味が異なる別概念であるため、既存関数を流用せず新設した。

### 3.4 `total_workforce()`の扱いと表示整合の範囲

事前調査で`StateData::total_workforce()`の呼び出し元は以下の4箇所と判明した。

| 箇所 | 用途 | 今回の扱い |
|---|---|---|
| `src/ui/state_panel.rs` | 表示専用 | `civilian_workforce()`へ置き換え(§3.1) |
| `src/economy/mod.rs`(国全体`employment_rate`集計) | `economic_state`表示指標の算出に使用 | `civilian_workforce()`へ置き換え(下記) |
| `src/economy/finance.rs` | 州の`emp_rate`→税収計算に使用 | **変更せず**(対象外) |
| `src/economy/production.rs`(2箇所) | `wf_ratio`→実際の生産量ゲートに使用 | **変更せず**(対象外) |

`economy/mod.rs`の国全体`employment_rate`(`handle_monthly_economy`内)は、
`country.economic_state`(`ui/economy_panel.rs`に表示される経済状態ラベル)へ
直結する**表示指標**であるため、州パネルと同じ理由で`civilian_workforce()`へ
差し替えた(P23-002完了報告の残課題として指摘されていた不整合の解消)。

一方`finance.rs`(税収計算)と`production.rs`(生産量ゲート)は、表示ではなく
実際のシミュレーション出力(税収・生産量)を左右する数値ロジックであり、
ここを変更すると「表示整合」の範囲を超えたバランス変更になる。ユーザー指示の
「§4. `StateData::total_workforce()`の表示整合」の対象は表示(および表示指標)に
限定されると判断し、この2箇所は意図的に変更していない(§9の後続候補として記録)。

### 3.5 UI更新経路

`update_state_panel`(州パネル)・`update_military_panel_ui`(国家情報)はいずれも
既存どおり`Update`スケジュールで毎フレーム実行され、フォーマット済み文字列が前回と
異なる場合のみ`Text`を書き換える設計を維持した。月次更新・動員/解除・領有変更・
ロード後のいずれでも、次に実行されるフレームで`Res<StateRegistry>`/
`Res<CountryRegistry>`から再計算されるため、新しいイベント購読やタイマーは
追加していない。

ただし、実装中に統合テストで実際に検出した問題として、`update_state_panel`と
`update_military_panel_ui`のどちらも`DailySimulationSet`に属しておらず、
月替わりを処理する`Economy` Setより同一フレーム内で先に実行される可能性が
あった(Bevyの`Update`スケジュールは、明示的な`.in_set`/`.before`/`.after`が
なければ実行順序を保証しない)。これにより、月替わりの瞬間のフレームだけ表示が
1フレーム古い値になり得る、既存コードの潜在的なギャップだった。本タスクが表示する
値(州の労働力・雇用率・動員負担、国家人口・人的資源上限)はまさにこの経路で
更新される値であり、成立の前提条件そのものだったため、両System を
`DailySimulationSet::UiUpdate`(既存の`economy::handle_notifications`と同じSet)へ
明示的に割り当てて修正した。ボタン操作系System(建設・募兵・前線命令等)は
本タスクの対象外のため変更していない。

### 3.6 セーブV1を維持した方法

新規セーブフィールド・DTO変更・バージョン変更は無い。表示に使う値
(`employed_workforce`/`unemployed_workforce`/`population`/`available_manpower`/
`mobilized_manpower`)はすべて既存のセーブ対象フィールドであり、`civilian_workforce`・
`employment_rate`・`mobilized_burden`(州別)・所有州人口合計・人的資源上限は
いずれも表示のたびに再計算する派生値として、一切永続化していない
(P23-001/P23-002と同じ設計判断)。

## 4. 変更ファイル一覧

| ファイル | 役割 |
|---|---|
| `src/state/data.rs`(変更) | `StateData::civilian_workforce()`を追加 |
| `src/population/update.rs`(変更) | `calculate_employment_rate`を追加、単体テスト5件追加 |
| `src/economy/mod.rs`(変更) | 国全体`employment_rate`集計を`civilian_workforce()`基準へ変更 |
| `src/ui/state_panel.rs`(変更) | 民間労働力・雇用率・動員負担の表示追加、UI統合テスト7件追加、`update_state_panel`を`DailySimulationSet::UiUpdate`へ |
| `src/ui/military_panel.rs`(変更) | 国家総人口・人的資源上限の表示追加、`update_military_panel_ui`を`DailySimulationSet::UiUpdate`へ |
| `assets/localization/en-US.ron`(変更) | `state_panel.info`更新、`military_panel.population`新規、`military_panel.manpower`更新 |
| `assets/localization/ja-JP.ron`(変更) | 同上(日本語) |
| `tests/p23_003_population_manpower_ui_e2e_test.rs`(新規) | 実UI spawn/update経路の統合テスト8件 |
| 本完了報告書(新規) | — |

## 5. 追加・更新したテスト

### Domain・表示値テスト(`src/population/update.rs`、5件)

1. `calculate_employment_rate_returns_zero_when_civilian_workforce_is_zero`
2. `calculate_employment_rate_computes_the_expected_ratio`
3. `calculate_employment_rate_clamps_when_employed_exceeds_civilian_workforce`
4. `civilian_workforce_sums_employed_and_unemployed`
5. `civilian_workforce_does_not_overflow_at_the_u64_boundary`

### 州パネルUI統合テスト(`src/ui/state_panel.rs`、7件、実`update_state_panel`System経由)

1. `workforce_display_uses_civilian_workforce_not_total_workforce`
2. `employment_rate_uses_employed_over_civilian_workforce`
3. `employment_rate_is_zero_when_civilian_workforce_is_zero`
4. `mobilized_burden_matches_the_population_weighted_apportionment`
5. `mobilized_burden_is_zero_when_country_has_no_mobilized_manpower`
6. `mobilized_burden_is_zero_for_a_state_whose_owner_country_is_missing`
7. `state_panel_display_updates_when_the_selected_state_changes`

### P23-003統合テスト(`tests/p23_003_population_manpower_ui_e2e_test.rs`、8件)

実`AppPlugin`+`CountryPlugin`+`StatePlugin`+`BuildingPlugin`+`EconomyPlugin`+
`PopulationPlugin`+`StatePanelPlugin`+`MilitaryPanelPlugin`+`SaveGamePlugin`/
`LoadGamePlugin`(実データ6か国)を組み合わせ、実際の月次処理・save/load経路を通した。

1. `state_panel_workforce_and_employment_rate_match_the_real_monthly_result`
2. `mobilization_shrinks_state_panel_workforce_and_shows_a_matching_burden`
3. `demobilizing_zeroes_the_burden_display_on_the_next_tick`
4. `military_panel_shows_population_available_and_cap_manpower_after_opening`
5. `military_panel_population_updates_after_state_ownership_transfer`
6. `save_and_load_round_trip_restores_the_state_and_military_panel_display`
7. `ja_and_en_locales_render_the_new_placeholders_without_missing_key_markers`
8. `military_panel_text_entity_is_not_duplicated_across_repeated_updates`

このうち#1は、`update_state_panel`のSet割り当てが無かった当初実装で実際に
`workforce`表示が"0"になる形で失敗し、§3.5のSet修正の必要性を検出した
(統合テストが実際に効果を発揮した実例)。

## 6. 実行した全コマンドと正確な結果

| コマンド | 結果 |
|---|---|
| `rustfmt --edition 2024 --check`(変更・新規8ファイル個別指定) | 初回差分あり→`rustfmt --edition 2024`適用後PASS |
| `cargo check --all-targets` | PASS |
| `cargo clippy --all-targets --all-features -- -D warnings` | 初回`field_reassign_with_default`2件→修正後PASS、**警告0** |
| `cargo test --lib ui::state_panel::` | PASS、15 passed(既存8+新規7) |
| `cargo test --test p23_003_population_manpower_ui_e2e_test` | PASS、8 passed |
| `cargo test --test p20_009_hardcoded_string_scan_test` | 初回1件FAILED(テスト内`Text::new("")`が誤検出)→`Text::new(String::new())`へ修正後PASS、4 passed |
| `cargo test --lib`(全lib) | PASS、**1119 passed** |
| `cargo test --all-targets`(全体、最終実行) | PASS、**全件PASS**(lib 1119 + 統合38ファイル合計1339、1 ignored[既存の`profile_p21_021_elimination_scaling_test`、本タスク無関係]) |

`tests/land_war_combat_peace_test.rs`は今回一切変更・整形していない(既知の保護対象)。

### 非決定的スクリーンショットPNGの扱い

`cargo test --all-targets`実行後、既知の9枚
(`verification_logs/p20-007/screenshots/03_playing_topbar.png`、
`verification_logs/p20-009/screenshots/{04,05,06}_*.png`、
`verification_logs/phase-21/p21-save-002e/screenshots/{01,02,03,04,05}_*.png`)が
想定どおり再生成されたことを`git status`で確認し、個別ファイル単位で
`git checkout -- <path>`により復元した(ディレクトリ単位の復元は行っていない)。
それ以外のファイルへの意図しない変更は無い。

## 7. 手動GUI確認

**実施していない。** 本セッションには実ウィンドウを操作する手段が無く、
`cargo run`によるスクリーンショット撮影・実際の目視確認は一切行っていない
(実施していないことを実施したと偽ることを避けるため明記する)。

未確認項目:

- 日本語UIで州パネルの人口・労働力・雇用率・動員負担、国家情報の人口・人的資源上限が
  実際に読める配置になっているか(ラベルと値の重なり、パネル幅からのはみ出し)
- 英語UIで同様に未翻訳キーが露出していないか、長い英語ラベルでの重なり・省略
- 動員負担がある州とない州で表示が視覚的に区別できるか
- 実際のロード操作(UIボタン経由)後の表示

これらは自動テスト(§5)でSystem経由の実値・ローカライズキー解決・spawn構造は
検証済みだが、実際のレイアウト崩れの有無はテストでは検出できない。

## 8. 対象外を先行実装していないこと

徴兵法・徴兵制度・政治体制連動・`war_exhaustion`・幸福度/不満度/支持率との連動・
AIの動員判断変更・新しい軍事補充システム・雇用優先順位の変更・月次処理頻度の変更・
新しい独立統計画面・グラフ/履歴/人口推移チャート・セーブV2は、いずれも変更ファイル
一覧(§4)に含まれておらず、実装していないことを確認した。`economy/finance.rs`・
`economy/production.rs`の数値ロジックも変更していない(§3.4)。

## 9. 残課題または後続候補

- `economy/finance.rs`の`emp_rate`(税収計算)と`economy/production.rs`の`wf_ratio`
  (生産量ゲート、2箇所)は、依然として`total_workforce()`(動員を考慮しない潜在労働力)
  を分母に使っている。動員中の国はこの2箇所で「実際より高い労働力」を基準に税収・
  生産量が計算され続けるため、動員が財政・生産へ与える圧迫効果は限定的にしか
  反映されない。これは表示上の不整合ではなく、動員のゲームバランス上の実効性に
  関わる論点のため、P23-003(表示整合)のスコープでは意図的に変更していない。
  対応するかはユーザー判断とし、対応する場合は独立したタスクとして切り出すことを
  推奨する。
- `update_state_panel`/`update_military_panel_ui`以外にも、`Economy` Set確定後の値を
  表示する既存System(例: `economy_panel.rs`)が同種のSet未割り当て問題を抱えている
  可能性があるが、本タスクでは実際に表示値が変わった2System(§3.5)のみ調査・修正し、
  他パネルの網羅的な監査は範囲外とした。

## 10. 最終`git status --short`

```
 M assets/localization/en-US.ron
 M assets/localization/ja-JP.ron
 M saves/savegame_v1.ron
 M src/economy/mod.rs
 M src/population/update.rs
 M src/state/data.rs
 M src/ui/military_panel.rs
 M src/ui/state_panel.rs
?? tests/p23_003_population_manpower_ui_e2e_test.rs
```

(コミット前の状態。`saves/savegame_v1.ron`はユーザーの既存未コミット変更であり、
本タスクのコミットには含めない。)
