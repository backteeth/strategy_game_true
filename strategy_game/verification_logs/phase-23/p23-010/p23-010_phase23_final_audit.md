# P23-010 完了報告: Phase 23 人口・人的資源ライフサイクル最終統合監査

## 1. 監査実施判定

**AUDIT COMPLETE**

## 2. Phase 23健全性判定

**PHASE 23 READY WITH MANUAL VERIFICATION PENDING**

自動検証(単体・統合・長期実行・save/load分岐比較・決定性再実行)はすべて成功し、
Critical/High欠陥は発見されなかった。実ウィンドウでの目視確認は本セッションでは
実施できなかったため、この一段階だけを理由に「WITH MANUAL VERIFICATION PENDING」を
付す(§17参照)。

## 3. 開始時HEAD

本タスクのプロンプトが指定した基準コミットは`a6d5009efcf8d58ecb777eb4fba49b0526154ceb`
(P23-009完了報告書コミット)だったが、監査作業を開始した実際のHEADは
`3299d43ac2ec4d379083f1ec874d6c2a12609a4a`であった。これは基準コミットの正当な子孫
であり、内容は本セッション中にユーザーから直接報告・承認を得て実施した1件のバグ修正
(`fix: allow AI/frontline pathfinding to route through allied territory`、多国間戦争
での経路探索が同盟国領を誤って通過不可とし、無関係なDivisionが動けなくなる不具合の
修正)である。Phase 23の対象範囲(人口・人的資源ライフサイクル)そのものではないが
AI/前線移動に関わるため、無関係な変更ではなく監査対象コードの一部として扱った。
`git log`で来歴を確認済みで、無許可の変更や基準状態の不整合は検出されなかった。

## 4. 監査対象となったコードHEAD

テスト固定コミット: `66bcf6650c019b255f48556855e481282b5016de`
(`test: lock phase 23 manpower lifecycle`)

## 5. ユーザーセーブの保護

`saves/savegame_v1.ron`は監査を通じて一切開かず、読まず、変更していない。全ての
セーブ/ロード検証は`std::env::temp_dir()`配下の一時パス(`TempTestDir`)だけを使用した。
監査終了時点でも`git status --short`は同ファイルの`M`表示のみを保持している
(§23参照)。

## 6. 実際のSystem順序とライフサイクル

`app::time::DailySimulationSet`は`app::time`の`configure_sets(Update, (...).chain())`
により、以下の順序が**Plugin登録順に関係なく**強制されることを実コードで確認した。

```text
TimeUpdate → Economy → Research → Diplomacy → CountryAi → WarPreparation
→ MilitaryAi → FrontlineOrders → MilitaryAction → WarResolution → UiUpdate
```

各Setの内訳(実際にコードへ登録されているSystemのみ、未登録の同名関数・死んだコードは
除外):

```text
Economy Set (MonthChangedMessage/DayChangedMessage駆動):
  population::handle_monthly_population_growth
    (州人口成長 + 国家available_manpower回復、月次)
    .before(economy::handle_monthly_economy) で明示的に順序固定
  → economy::(handle_daily_construction, handle_monthly_economy).chain()
    (建設進捗は日次、雇用・生産・welfare/unrestは月次。
     handle_monthly_economyが population::update::update_state_population_and_employment
     を内部で呼ぶ)

CountryAi Set:
  country::handle_daily_country_ai (日次、外交AI・戦争準備AI・宣戦)
  → country::emit_ai_war_declaration_notifications
  diplomacy::handle_monthly_ai_claim_generation (月次、P21-018)
  diplomacy側の月次Crisis開始AI
  military::handle_monthly_ai_recruitment (月次、P23-009、
    evaluated_this_frameフラグでフレーム内の複数MonthChangedMessageを1回に限定)

MilitaryAction Set (DayChangedMessage駆動):
  military::handle_daily_military:
    1. recruitment::process_recruitment (募兵キュー進行・完成)
    2. reinforcement::apply_daily_reinforcement (P23-008日次自動補充)
    3. movement::validate_and_stop_invalid_movements
    4. movement::process_movement (移動・侵攻・戦闘開始)
    4. update::process_daily_battles (日次戦闘計算、BattleId昇順)
    5. update::resolve_finished_battles (戦闘終了判定・占領・撤退)
    6. BattleRegistry::cleanup_finished_battles
    7. update::process_org_recovery (組織率回復)
  → military::handle_daily_army_maintenance (.after、Army参照整理)
```

`spawn_debug_divisions`(New Game初期配置)は`app::loader::DataLoaderPlugin`の
`OnEnter(GameState::Playing)`チェーン(`(spawn_debug_divisions,
rebuild_country_power_on_enter_playing, reset_playing_entry_mode).chain()`)で
実行され、`PlayingEntryMode != NewGame`(Loaded Game含む)では即座にreturnする
ことをコードで再確認した。

個別Division復員(`military::demobilization::demobilize_division`)は日次/月次
Systemには登録されておらず、UI操作(`ui::military_panel`の「師団を復員」ボタン)
またはテスト/将来のAPI呼び出しからのみ実行される、副作用のないDomain関数である
ことを確認した。

## 7. A・B・L・K・Xの意味

```text
A = available_manpower(国家の即応可能な人的資源プール)
B = mobilized_manpower(累積動員負担。現在生存兵員数ではない)
L = living_military_manpower
  = 募兵キューで予約中の人的資源 + 生存Divisionの現在manpower合計
  (military::manpower::compute_manpower_ledgerで導出、生存判定は
   `manpower>0 && status!=Destroyed`)
K = permanent_military_losses = B.saturating_sub(L)
X = unaccounted_living_excess = L.saturating_sub(B)
```

`compute_manpower_ledger`の実装をP23-006時点から変更していないことを再確認した。
`B==L`が常時成立するという主張は誤りであり(戦闘損耗後は`B>L`が正常)、監査全体を
通じてこの用語で判定した。`A+B<=cap`は既存コードが保証する不変条件ではなく、
本監査でも新しい必須条件として導入していない。population↔K統合は未実装のまま。

## 8. 各操作前後の帳簿遷移表

| 操作 | A | B | L | K | 検証方法 |
|---|---|---|---|---|---|
| New Game初期Division配置 | 不変 | 実配置manpower分 += | += (同量) | 不変(0) | `new_game_gives_every_active_country_a_balanced_ledger` |
| プレイヤー/AI募兵成功(`request_recruitment`) | -= R | += R | += R(キュー予約分) | 不変 | `full_lifecycle_cycle_...`ステージ1 |
| 募兵完成(キュー→Division) | 不変 | 不変 | 不変(内訳がキュー由来→Division由来へ移るだけ) | 不変 | 同ステージ2 |
| 実戦闘損耗C(実`handle_daily_military`経由) | 不変 | 不変 | -= C(実測値) | += C(実測値) | 同ステージ3 |
| 日次補充Q(実`apply_daily_reinforcement`経由) | -= Q | += Q | += Q | 不変 | 同ステージ4.5 |
| Division復員(生存者R、`demobilize_division`) | += R | -= R | -= R | 不変 | 同ステージ5 |
| 募兵キャンセル(`cancel_recruitment`、既存P23-002テストで継続確認) | += R | -= R | -= R | 不変 | 既存`p23_002_...`テスト群(回帰なし) |

いずれも「実測した増減量」を期待値とし、戦闘計算式やbasis-point計算式そのものを
テスト側へ再実装していない。

## 9. New Game／Loaded Game監査結果

* New Game初期Division配置後、全活動国家で`B==L`・`K==0`・`X==0`が成立することを
  実データ(6か国)で確認(`new_game_gives_every_active_country_a_balanced_ledger`)。
* `available_manpower`・population(初期Division配置の副作用)が変更されないことを
  確認(同テスト)。
* 同一Playing進入内で複数フレーム進めても初期Divisionが二重生成されないことを確認
  (`playing_reentry_does_not_duplicate_the_initial_division_setup`)。
* Loaded Gameでは、保存時と同じDivision数・同じ募兵キュー長がロード後も維持され、
  ロード単体ではAI募兵が発生しないことを確認
  (`loaded_game_does_not_duplicate_divisions_or_trigger_immediate_ai_recruitment`)。
* ロード後に次の正規Message(月次・日次)から処理を再開できることは、
  `ai_recruitment_resumes_on_the_next_regular_month_changed_after_load`
  (既存P23-009テスト、再実行して回帰なしを確認)および本タスクの
  save/load分岐比較テスト(§15)でも重ねて確認した。

## 10. プレイヤー募兵／AI募兵監査結果

* プレイヤー募兵は既存`request_recruitment`を経由し、§8の帳簿遷移が成立することを
  実測ベースで確認した。
* AI募兵(`military::ai::handle_monthly_ai_recruitment`)がプレイヤー国家を対象に
  含めないことを、10年間の長期実行全体で確認した
  (`ai_recruitment_stays_bounded_and_consistent_over_a_ten_year_run`、
  最後にプレイヤーのqueueが常に0であることを明示的にassert)。
* 1国家1か月最大1件の制限が、120か月・全AI国家にわたって一度も破られないことを
  確認(同テスト)。
* 帳簿計算不能・帳簿不整合・キュー非空・予備人的資源不足・treasury不足・募兵州なし
  のスキップ条件は、P23-009自身の28件のDomain単体テスト(継続してPASS)が個別に
  固定済みであり、本監査では再重複せず、長期実行での「破綻しないこと」の確認に
  範囲を絞った。

## 11. 戦闘損耗／補充／復員監査結果

`full_lifecycle_cycle_preserves_the_manpower_ledger_at_every_stage`で、実際の
`handle_daily_military`経路(実War/Battleを登録し、実日次Systemで1日分の戦闘計算を
実行)による損耗を実測し、§8の関係が成立することを確認した。生存していれば
その日以降の日次補充で`A`が減り`B`/`L`が実測値分だけ同量増え`K`は不変、復員では
`Division.manpower`(`max_manpower`ではない)がそのまま`A`へ返還され`B`/`L`が同量
減り`K`は不変であることを実測で確認した。復員後の月次雇用更新で民間労働力が
悪化しない(復員によりBが減るため)ことも同テストで確認した。他国の帳簿が
戦闘に巻き込まれず`X==0`のまま維持されることも確認した。

## 12. 月次人口・雇用・経済監査結果

`handle_monthly_population_growth`が`handle_monthly_economy`より先に実行される
明示的な`.before()`制約をコードで確認した(§6)。10年間の長期実行・全国家人口0の
境界テストを通じて、雇用率計算等での0除算・NaN・無限大が発生しないことを確認した
(`a_country_that_loses_all_population_does_not_panic_or_produce_nan_over_several_
months`)。動員負担がBを基準にすること(LやKではない)は`compute_manpower_ledger`と
`apportion_mobilized_manpower`(P23-002、既存)の実装そのものから既に保証されており、
本監査で変更していない。`civilian_workforce()`アクセサ・E/P式(P23-004)も未変更。

## 13. 国家消滅・領有変更監査結果

本監査では国家消滅・領有変更専用の新規統合テストを追加していない。既存の
P21-021(国家消滅Domain API)・P21-020(所有権移転共通API)の広範なテスト群
(継続して全件PASS、回帰なし、§16のコマンド結果参照)が、danglingなDivision/
募兵キュー/Army/前線参照の除去、消滅国家の以後の除外(AI募兵候補集合からの除外は
`military::ai::evaluate_ai_monthly_recruitment`が`is_active()`フィルタで保証、
P23-009で既に固定済み)を検証済みであり、本監査ではそれらの継続PASSを再確認する
に留めた。所有権変更後の人口・動員負担の追従はP23-002自身のテスト
(`ownership_transfer_moves_the_burden_to_the_new_owner_on_the_next_tick`)が
既に固定している。人的資源の没収・返還について、現行コードはそのような処理を
一切定義していないことをコード上再確認した(`country::elimination`は
`recruitment_queue.clear()`と`MilitaryRegistry`からのDivision削除のみを行い、
`available_manpower`/`mobilized_manpower`自体は変更しない)。

## 14. 1年・5年・10年の長期監査結果

`ai_recruitment_stays_bounded_and_consistent_over_a_ten_year_run`が120か月
(1800日相当のDayChangedMessage、実際には30日/月×120=3600件)を1回のテスト内で
実行し、12・60・120か月時点でチェックポイント検証した。

結果: 全期間を通じて全AI国家の募兵キューが1件を超えず、12/60/120か月時点で全AI国家
の帳簿が計算可能かつ`X==0`、treasuryが有限値のまま(NaN/無限大なし)、
`available_manpower`が`u64::MAX`へ張り付く異常もなかった。プレイヤー自身は最後まで
一度もAI募兵の対象にならなかった。テスト自体の実行時間は0.81秒(§18で後述)。

観測事項(Phase 23対象外、§18参照): AI国家には人為的に十分なtreasury/
available_manpowerを与えて実際に募兵サイクルが回るようにした一方、プレイヤー国家
(実データの初期値のまま)は60か月時点でtreasuryが負値になり得ることを観測した。
これは既存の生産・税収式(Phase 23対象外)によるものであり、`available_manpower`
(u64)側の異常や人的資源保存則の破れではない。

## 15. save/load分岐比較結果

`a_save_and_reload_partway_through_matches_an_uninterrupted_run`で、経路A
(中断せず6か月連続実行)と経路B(3か月実行→save→新しいAppへload→残り3か月実行)を
比較した。国家別スナップショット(available/mobilized/treasury丸め/queue長/帳簿
B・L・K・X)、州別スナップショット(所有国・人口・employed・unemployed)、
Division別スナップショット(owner・manpower・status)のいずれも、`DivisionId`/
`StateId`昇順に正規化した上で完全一致した。AI国家には両経路とも同じ人為的
treasury/available_manpowerブーストを与えたため、AI募兵結果も含めて比較対象に
含まれている。

## 16. 決定性検証結果

`the_same_scenario_run_twice_independently_produces_identical_snapshots`で、
同一シナリオ(4か月、AI国家へ同じ人為的ブースト)を独立した2つの`App`インスタンスで
実行し、正規化スナップショット(国家・州・Division)が完全一致することを確認した。
`HashMap`走査順・Entity ID・ログ順など無関係な値は比較対象から除外した(§18で、
このテストファイル自体の初稿にあったHashMap順依存の非決定性バグとその修正を報告)。

## 17. 手動GUI確認結果

**未実施。** 本セッションには実ウィンドウを操作する手段がなく、P23-003/006/007で
保留されていた確認(州パネル・軍事パネルの表示一致、復員ボタンの配置・活性状態、
JA/EN切替、ロード後の表示継続更新)は今回もまとめて実施できなかった。これが
「PHASE 23 READY WITH MANUAL VERIFICATION PENDING」とした唯一の理由である。

## 18. 発見事項とCritical／High／Medium／Low分類

**Critical: 0件、High: 0件、Medium: 0件、Low: 2件、参考(対象外の観測): 1件**

### Low-1: 未解決DivisionDefinitionの黙った消費(`military::recruitment::process_
recruitment`、既存挙動・本タスクでの変更なし)

`process_recruitment`は、募兵キュー項目の`days_remaining`が0に達した時点で、
対象`DivisionDefinitionId`が`MilitaryRegistry.definitions`に存在しなくても、
その項目を無条件にキューから消費する(Divisionは生成されず、エラーも発生しない)。
`compute_manpower_ledger`が未解決定義を明示的な`Err`として扱う設計(P23-006-FIX-001)
と対照的である。ただし`request_recruitment`は必ずDefinition存在を検証してから
キューへ積むため、この経路は実際のゲームプレイでは到達せず、`save::validate`の
`unknown_recruitment_division_definition_is_rejected`が検証済みSaveでもこの状態を
拒否する。この項目に対応する`mobilized_manpower`加算もそもそも発生していない
(合法な経路で積まれた項目ではないため)ため、人的資源の消失は起きない。
`an_unresolved_recruitment_queue_definition_is_reported_as_an_explicit_error_not_
a_panic_mid_run`でこの挙動自体を固定した(意図的でない仕様として、将来この経路が
到達可能になった場合の回帰検出用)。

### Low-2: 監査テスト自身の初稿にあったHashMap順依存の非決定性(修正済み)

`full_lifecycle_cycle_preserves_the_manpower_ledger_at_every_stage`の初稿は、
敵国のDivisionを`MilitaryRegistry.divisions.values().find(...)`で選択しており、
Rustの`HashMap`のプロセスごとのハッシュシードにより選ばれるDivisionが実行ごとに
変わり得た。これはテストの実装ミスであり、プロダクションコードの欠陥ではない。
`.min_by_key(|d| d.id.0)`による決定的選択へ修正し、独立した5回のプロセス実行で
安定することを確認した(§16参照)。この経緯自体が、実際のゲーム戦闘計算
(`combat_calc::resolve_combat_day_multi`)は真に決定的であり、当初の不安定さは
テストのフィクスチャ選択順にのみ起因していたことの直接的な証拠でもある。

### 参考(Phase 23対象外の観測): 長期実行でのtreasury赤字化

§14に記載のとおり。既存の生産・税収式(Phase 23対象外)の挙動であり、本タスクの
判定には影響しない。

## 19. Phase 23完了を妨げる問題

なし。Critical/High欠陥は発見されなかった。唯一の留保事項は§17の手動GUI未確認のみ。

## 20. 対象外を先行実装していないこと

Attrition、装備・物資を消費する補充、同盟国領での補充、AIの兵種選択・復員判断・
募兵キャンセル、50%目標/25%予備の戦争状態補正、manpower 0 Divisionの復活、
population減少、population↔K統合、`A+B<=cap`の新規強制、徴兵法・動員法、
`war_exhaustion`、戦闘式・生産式・税収式の再設計、新規UI操作、新規セーブフィールド、
セーブV2、Phase 24機能は、いずれも実装していない。変更ファイルはテスト1件
(`tests/p23_010_phase23_manpower_lifecycle_audit_test.rs`)のみであり、本番コード
(`src/`配下)は本タスクで一切変更していない。

## 21. 推奨する次フェーズまたは最小修正タスク

* (任意・Low優先度) §18 Low-1の`process_recruitment`未解決Definition黙殺経路へ、
  ログ出力または明示的なエラー通知を追加する最小修正(緊急性なし、
  `save::validate`が既に実質的な防御になっている)。
* (対象外・観測のみ) 長期プレイでの国家財政バランス(既存の生産・税収式)の
  見直しは、Phase 23とは独立した既存の経済システムの課題として、別途検討候補。
* Phase 24候補: population↔恒久軍事損失(K)の統合、Attrition、装備・補給を
  消費する補充、AIの兵種選択・復員判断、`available+mobilized<=cap`の不変条件を
  導入するかどうかの設計判断。

## 22. 実行した全コマンドと正確な結果

| コマンド | 結果 |
|---|---|
| `cargo check --all-targets` | PASS |
| `cargo test --lib population::` | PASS、46 passed |
| `cargo test --lib military::` | PASS、177 passed |
| `cargo test --lib economy::` | PASS、6 passed |
| `cargo test --lib save::` | PASS、300 passed |
| `cargo test --test p23_001_population_manpower_e2e_test` | PASS、3 passed |
| `cargo test --test p23_002_mobilized_manpower_workforce_e2e_test` | PASS、8 passed |
| `cargo test --test p23_003_population_manpower_ui_e2e_test` | PASS、8 passed |
| `cargo test --test p23_004_economic_workforce_ratio_e2e_test`(注: プロンプト記載名`p23_004_mobilization_economic_impact_e2e_test`は実ファイル名と異なるため実名で実行) | PASS、7 passed |
| `cargo test --test p23_006_manpower_ledger_e2e_test` | PASS、22 passed |
| `cargo test --test p23_006_fix_001_initial_manpower_ledger_e2e_test` | PASS、19 passed |
| `cargo test --test p23_007_division_demobilization_e2e_test` | PASS、24 passed |
| `cargo test --test p23_008_daily_reinforcement_e2e_test` | PASS、21 passed |
| `cargo test --test p23_009_ai_recruitment_e2e_test` | PASS、22 passed |
| `cargo test --test p23_010_phase23_manpower_lifecycle_audit_test` | PASS、**9 passed**(実行時間0.88〜1.03秒、10年長期テストは単独0.81秒) |
| `cargo test --test p20_009_localization_resource_test` | PASS、8 passed |
| `cargo clippy --all-targets --all-features -- -D warnings` | 初回2件(`manual_contains`修正、および決定性修正に伴う再検証)→修正後PASS、**警告0** |
| `rustfmt --edition 2024 --check`(新規ファイル個別指定) | 初回に整形差分あり→`rustfmt --edition 2024`適用後PASS |
| `cargo test --all-targets`(全体、最終実行) | PASS、**全件PASS**(**1585 passed**、**0 failed**、1 ignored[既存の`profile_p21_021_elimination_scaling_test`、本タスク無関係]) |

1585 = 直前の基準1576(P23-009完了時1575 + 多国間戦争経路探索バグ修正で追加した
回帰テスト1件)+ 本タスクの新規9件、と正確に一致することを確認した。
`tests/land_war_combat_peace_test.rs`は今回一切変更・整形していない。既知の
非決定的PNG9枚は`cargo test --all-targets`実行後に想定どおり再生成され、個別
ファイル単位で`git checkout --`により復元した(ディレクトリ単位の復元はしていない)。

## 23. 監査対象コードをコミットした直後の`git status --short`

テスト固定コミット(`66bcf6650c019b255f48556855e481282b5016de`)直後:

```
 M saves/savegame_v1.ron
```

`saves/savegame_v1.ron`はユーザーの既存未コミット変更であり、テスト固定コミットには
含めていない。それ以外の全変更(新規テストファイル1件)はコミット済みで、本番コード
(`src/`配下)は一切変更していない。
