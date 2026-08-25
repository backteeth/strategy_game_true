# P23-006 完了報告: 現役兵・累積動員負担・恒久軍事損失の意味分離

## 1. 総合判定

**COMPLETE WITH MANUAL VERIFICATION PENDING**

自動テスト・検証コマンドはすべて成功したが、実ウィンドウでの目視確認(`cargo run`)は
本セッションでは実施していない(§7参照)。

## 2. 基準状態

* 開始時HEAD: `8c74d1924a0c288eeb80a8fc4857fb04bfb04664`(P23-005監査完了)
* 実装コミットの完全なハッシュ: `946ff13a8570482fd1b760fbc222c9e815c13135`
  (`feat: separate military manpower ledger semantics`)
* 報告書コミットの完全なハッシュ: 本報告コミット後に確定するため、このコミット自身の
  ハッシュはここへ自己参照的に記載しない
* 最終HEAD: 報告書コミット後に確定(コミット後の最終回答で報告する)
* ユーザーの既存未コミット変更(`saves/savegame_v1.ron`)は本タスク開始前から存在する
  実プレイデータであり、一切参照・変更・ステージ・コミットしていない。

事前確認: P23-005監査報告書が参照した実コード箇所(`CountryData.mobilized_manpower`、
`RecruitmentQueueItem`、`Division.manpower`/`max_manpower`/`owner`/`status`、
`MilitaryRegistry`、`request_recruitment`、`cancel_recruitment`、`process_recruitment`、
`process_daily_battles`、`resolve_participant_outcome`、`apply_country_elimination`、
`apportion_mobilized_manpower`、軍事パネルの人的資源表示、save/load V1)はいずれも
現在のHEADでも監査時点の記述と一致していることを実装開始前に再確認した。重大な
不一致はなかった。

## 3. 実装内容

### 3.1 `mobilized_manpower`の新しい正式な意味

`src/country/mod.rs`のドキュメントコメントを、「軍隊に所属している合計」から
「募兵によって民間労働力から離脱し、まだ民間へ返還されていない人的資源の累積量
(訓練中・現役・恒久損失のいずれも含む)」へ改めた。フィールド名・serde上の名称は
変更していない。書き込み経路(`request_recruitment`の`+=`、`cancel_recruitment`の
`saturating_sub`)自体も変更していない——**戦闘損耗・部隊撃破のコードは今回
`mobilized_manpower`を一切減算しない**(§11の禁止事項どおり)。

### 3.2 現役軍事人的資源(L)の導出方法

新規モジュール`src/military/manpower.rs::compute_manpower_ledger`で、以下の合計として
毎回その場で導出する(永続化しない)。

```text
L = Σ(募兵キュー内の各項目のDivisionDefinition.required_manpower)
  + Σ(生存Divisionの現在manpower、owner==対象国)
```

「生存」の判定は既存コードベース全体で使われている規約(`manpower == 0 ||
status == DivisionStatus::Destroyed`を「死亡」とみなす)の否定を用いた。募兵キューの
項目が参照する`DivisionDefinition`が存在しない場合は0人として扱う
(`save::validate`がロード時点で`unknown_recruitment_division_definition_is_rejected`
により既にこの状態を拒否する契約に依拠する——パニックはしない)。

### 3.3 恒久軍事損失(K)の導出方法

```text
K = mobilized_manpower.saturating_sub(L)
```

新規フィールドは追加していない(`ManpowerLedger`構造体はどこにも永続化しない導出値の
入れ物)。

### 3.4 募兵キューとDivisionの扱い

募兵コマンド発行時点の予約(未完成)は「募兵キュー内の予約済み生存兵員」として`L`へ
計上し、部隊完成時にDivision側の合計へ切り替わる(合計`L`自体は変化しない、実統合
テストで確認済み)。

### 3.5 `L > B`不整合の検出方法

`ManpowerLedger.unaccounted_living_excess = living_military_manpower.saturating_sub(
workforce_burden)`として、`K`とは別のフィールドで検出可能にした。`K`は`saturating_sub`
により負値にならず常に安全な0以上の値を保つ。

**この不整合は理論上の異常系ではなく、実データで実際に発生することを本タスク中に
発見した**(§9参照)。

### 3.6 戦闘損耗時に`mobilized_manpower`を減らさない理由

P23-005監査が明らかにしたとおり、`civilian_workforce()`(民間労働力)は
`calculate_workforce(population, workforce_ratio)`(四捨五入)から州別動員負担を
控除して算出される。もし戦闘損耗の都度`mobilized_manpower`を1人単位で減算すると、
`workforce_ratio < 1.0`の州では四捨五入の丸め方によって、兵士の死亡後にかえって
民間労働力が増加する(死亡が民間労働力を回復させる)という数学的な逆説が生じ得る
ことをP23-005が具体的な数値例で示した。本タスクはこの逆説を回避するため、
`mobilized_manpower`(=累積動員負担)を意図的に据え置いたまま、現役/恒久損失を
別の導出値として切り分ける設計を採用した。

### 3.7 民間労働力が戦死によって回復しないこと

`mobilized_manpower`(州別按分の元になる値)が戦闘損耗で変化しないため、
`population::mobilization::apportion_mobilized_manpower`による月次の州別動員負担も
変化せず、`civilian_workforce()`は戦闘の前後で不変のままとなる(実統合テストで確認、
§5参照)。募兵キャンセル(`mobilized_manpower`を実際に減らす唯一の経路)のみが、
次の月次処理で民間労働力を回復させる。

### 3.8 UI表示

`src/ui/military_panel.rs`の`update_military_panel_ui`内、既存の「人口」行に続く
人的資源表示を、重複追加ではなく整理した。

```text
人的資源: 利用可能 {available} / 基準上限 {cap}
軍事人的資源: 現役 {living} / 累積動員負担 {burden}
恒久軍事損失: {losses}
```

`military::manpower::compute_manpower_ledger`を1回だけ呼び出し、UI側で帳簿計算を
複製していない。表示のために戦闘・雇用・月次処理を発火させる副作用は追加していない
(既存の毎フレーム再計算・差分書き込みの設計をそのまま維持)。

### 3.9 セーブV1を維持した方法

新規保存フィールドは追加していない。現役軍事人的資源・恒久軍事損失はいずれも
`CountryData.mobilized_manpower`(既存フィールド、serde名称変更なし)・
`CountryData.recruitment_queue`(既存)・`MilitaryRegistry.divisions`(既存)から
ロード後に毎回再導出する。

## 4. 変更ファイル一覧と役割

| ファイル | 役割 |
|---|---|
| `src/military/manpower.rs`(新規) | `ManpowerLedger`構造体・`compute_manpower_ledger`(唯一の正規集計)・単体テスト13件 |
| `src/military/mod.rs`(変更) | `pub mod manpower;`を追加 |
| `src/country/mod.rs`(変更) | `mobilized_manpower`のドキュメントコメントを新しい正式な意味へ更新 |
| `src/population/mobilization.rs`(変更) | モジュール冒頭のコメントを新しい意味へ整合 |
| `src/ui/military_panel.rs`(変更) | 人的資源表示を「利用可能/基準上限」「現役/累積動員負担」「恒久軍事損失」の3行へ整理 |
| `assets/localization/en-US.ron`・`ja-JP.ron`(変更) | `military_panel.manpower`を整理し、`military_panel.manpower_military`・`military_panel.manpower_losses`を新規追加 |
| `tests/p23_006_manpower_ledger_e2e_test.rs`(新規) | 実経路統合テスト22件 |
| 本完了報告書(新規、別コミット) | — |

## 5. 追加・更新したテスト

### 5.1 Domain単体テスト(`src/military/manpower.rs`、13件)

`no_divisions_and_no_queue_yields_zero_living_manpower`、
`queue_only_contributes_the_definitions_required_manpower`、
`divisions_only_contribute_their_current_manpower`、
`queue_and_multiple_divisions_sum_correctly`、
`another_countrys_divisions_are_excluded`、
`a_division_with_zero_manpower_contributes_nothing`、
`a_destroyed_division_is_excluded_even_with_stale_positive_manpower`、
`burden_equal_to_living_yields_zero_permanent_losses`、
`burden_greater_than_living_yields_the_difference_as_permanent_losses`、
`living_greater_than_burden_yields_zero_losses_and_detects_the_excess`、
`extreme_values_near_u64_max_do_not_overflow_or_panic`、
`insertion_order_does_not_affect_the_result`、
`an_unknown_division_definition_in_the_queue_contributes_zero_without_panicking`。

### 5.2 実経路統合テスト(`tests/p23_006_manpower_ledger_e2e_test.rs`、22件)

**募兵ライフサイクル**(`military::update::handle_daily_military`を最小構成で実行):
`recruitment_start_increases_burden_and_living_by_the_same_amount`、
`recruitment_cancellation_decreases_burden_and_living_by_the_same_amount`、
`division_completion_moves_living_from_queue_to_division_without_changing_the_total`。

**戦闘損耗・撃破**(同じく本番Systemを実行、ダメージ量は実際の
`combat_calc::resolve_combat_day_multi`の結果を観測、式を予測・複製していない):
`real_combat_casualties_leave_the_burden_unchanged_and_move_the_measured_loss_from_living_to_permanent_losses`、
`a_fully_wiped_out_division_is_not_double_counted_in_permanent_losses`、
`same_day_multi_division_combat_sums_correctly_into_the_ledger`、
`one_countrys_real_combat_losses_do_not_affect_another_countrys_ledger`。

**戦後孤立部隊破壊・Army解散**(実Domain API直接呼び出し):
`a_stranded_division_destroyed_after_peace_moves_its_manpower_from_living_to_permanent_losses`、
`army_disband_does_not_change_the_ledger`。

**民間労働力**(実月次処理経路。P23-004で確認済みの「人口成長は動員/損耗の有無に
関わらず進む」ため、比較が必要なケースは経過月数を揃えた2つの独立したAppで比較する
設計にした):
`recruitment_reduces_civilian_workforce_after_the_next_monthly_update`、
`real_combat_losses_do_not_restore_civilian_workforce_the_following_month`、
`recruitment_cancellation_restores_civilian_workforce_the_following_month`、
`another_countrys_losses_do_not_affect_the_target_countrys_civilian_workforce`。

**Save/Load**:
`a_save_with_burden_equal_to_living_reloads_with_zero_permanent_losses`、
`a_save_with_permanent_losses_reproduces_the_same_ledger_after_reload`、
`recruitment_queue_is_correctly_reconstructed_into_living_manpower_after_reload`、
`an_inconsistent_l_greater_than_b_state_is_detected_safely_without_panicking_after_reload`。

**UI・JA/EN**:
`military_panel_displays_available_cap_living_burden_and_losses`、
`military_panel_losses_are_zero_before_combat_and_positive_after_a_real_loss_with_burden_unchanged`、
`ja_and_en_render_the_new_manpower_labels_without_missing_key_markers`、
`military_panel_text_entity_is_not_duplicated_after_repeated_updates`、
`military_panel_does_not_panic_with_no_state_selected_or_after_reload`。

## 6. 実行した全コマンドと正確な結果

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | PASS |
| `cargo test --lib military::manpower::` | PASS、13 passed |
| `cargo test --test p23_006_manpower_ledger_e2e_test` | PASS、22 passed |
| `cargo test --test p20_009_localization_resource_test` | PASS、8 passed |
| `cargo test --test p20_009_hardcoded_string_scan_test` | PASS、4 passed |
| `rustfmt --edition 2024 --check`(変更・新規ファイル個別指定) | 初回差分あり→適用後PASS。`src/military/mod.rs`はmod.rsファイルへ直接rustfmtを適用すると、そこが`pub mod`宣言する`movement.rs`/`supply.rs`(いずれも今回無関係・未変更)まで巻き込んでフォーマットしてしまう既知の問題があるため、直接のrustfmt適用は行わず、1行追加のみの差分を目視確認した |
| `cargo clippy --all-targets --all-features -- -D warnings` | 初回`unnecessary_mut_passed`1件→修正後PASS、**警告0** |
| `cargo test --lib` | PASS、1139 passed |
| `cargo test --all-targets`(全体、最終実行) | PASS、**全件PASS**(1396 passed、**0 failed**、1 ignored[既存の`profile_p21_021_elimination_scaling_test`、本タスク無関係]) |

`tests/land_war_combat_peace_test.rs`は今回一切変更・整形していない(既知の保護対象)。
既知の非決定的PNG9枚は`cargo test --all-targets`実行後に想定どおり再生成され、
個別ファイル単位で`git checkout --`により復元した(ディレクトリ単位の復元はしていない)。

## 7. 手動GUI確認

**実施していない。** 本セッションには実ウィンドウを操作する手段が無く、
`cargo run`によるスクリーンショット撮影・実際の目視確認は一切行っていない。

未確認項目:

* 日本語UIで「利用可能人的資源」「基準上限」「現役軍事人的資源」「累積動員負担」
  「恒久軍事損失」が実際に読める配置になっているか(ラベルと値の重なり、パネル幅
  からのはみ出し)
* 英語UIで同様に未翻訳キーが露出していないか、長い英語ラベルでの重なり・省略
* 募兵中・部隊完成後・戦闘損耗後で表示される値の意味が一貫しているか(実際の画面上で)
* パネルを閉じて再表示した際の重複表示の有無(実UI上での見た目)
* 実際のロード操作(UIボタン経由)後の表示

これらは自動テスト(§5)でSystem経由の実値・ローカライズキー解決・spawn構造(重複が
無いこと)は検証済みだが、実際のレイアウト崩れの有無はテストでは検出できない。

## 8. 対象外を先行実装していないこと

個別Divisionの復員・解散、国家全体の動員解除、生存兵員の`available_manpower`への
返還、補充、Attrition、AI募兵、戦死者/負傷者/捕虜/行方不明者の内訳、州別/戦争別/
戦闘別の損失履歴、戦闘損失によるpopulation減少、年齢/性別/職業別人口、
`available + mobilized <= cap`の強制、初期`available_manpower = 100,000`の再調整、
人的資源回復式の変更、戦闘式/生産式/税収式の変更、徴兵法/動員法、`war_exhaustion`、
セーブV2は、いずれも変更ファイル一覧(§4)に含まれておらず、実装していないことを
確認した。戦闘損耗時の`mobilized_manpower`減算、部隊撃破時の`mobilized_manpower`
減算、損耗兵員の`available_manpower`への返還、損耗兵員数だけのpopulation減少、
戦闘ダメージ式の変更、撤退追加損耗率の変更、Divisionの削除条件変更、未使用の
`war::combat::process_combat`のSystem接続、戦闘処理順序の変更のいずれも行っていない
(§11の禁止事項)。

## 9. 残課題または後続候補

* **個別Divisionの復員(P23-005 F2)**: プレイヤーが部隊を解散して人的資源を
  取り戻す機能は依然として存在しない。
* **補充(P23-005 F3)**: 損耗した部隊の兵員を回復する機能は依然として存在しない。
* **Attrition(P23-005 F4)**: 非戦闘損耗は依然として未実装。
* **AI募兵(P23-005 F5)**: AI国家は依然として`request_recruitment`を呼ばない。
* **人口と恒久損失の将来統合**: P23-005の丸め逆説の分析どおり、恒久軍事損失を
  将来州人口へ反映する場合は、population(四捨五入経由)とmobilized_manpower
  (整数直接減算)の丸め方式の非対称性を踏まえた別設計が必要。
* **人的資源上限不変条件(`available + mobilized <= cap`)**: P23-005で確認したとおり
  保証されておらず、今回も強制していない(意図的な対象外)。
* **新発見(本タスク中に発見、重要度High寄り、後続候補として記録)**:
  `src/app/loader.rs::spawn_debug_divisions`は、New Game開始時に**全ての国家**
  (プレイヤー含む)の首都へ初期Infantry Divisionを配置するが、対応する
  `CountryData.mobilized_manpower`の加算を一切行っていないことを、本タスクの
  統合テスト作成中に実際に発見した(テストがアンビエントな実データに依存しない
  設計へ修正する過程で顕在化した)。この結果、**New Gameを開始した瞬間から、
  全国家が`living_military_manpower > mobilized_manpower`
  (`unaccounted_living_excess > 0`)という状態になる**——これはP23-005が
  想定していた「旧セーブや手編集データでのみ起こり得る理論上の異常系」ではなく、
  現在の通常プレイの初期状態そのものである。本タスクの`compute_manpower_ledger`は
  この状態でもpanicせず、`K=0`かつ`unaccounted_living_excess`で安全に表現できる
  ことを確認済みだが、`spawn_debug_divisions`側で`mobilized_manpower`を対応する
  分だけ加算するかどうかは、本タスクの禁止事項(戦闘・削除条件の変更、
  初期値の再調整)に触れない範囲でのユーザー判断が必要な、独立した後続候補とする。

## 10. 実装コミット直後と報告書コミット後の`git status --short`

### 実装コミット(`946ff13a8570482fd1b760fbc222c9e815c13135`)直後

```
 M saves/savegame_v1.ron
```

`saves/savegame_v1.ron`はユーザーの既存未コミット変更であり、実装コミットには
含めていない。それ以外の全変更(§4の実装・テスト・ローカライズファイル)はコミット済み。

### 報告書コミット後

報告書コミット作成後の最終`git status --short`は、コミット完了後の最終回答で報告する。
