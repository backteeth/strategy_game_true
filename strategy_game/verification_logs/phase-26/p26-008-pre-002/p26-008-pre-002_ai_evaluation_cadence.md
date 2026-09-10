# P26-008-PRE-002: AI評価cadence・イベント駆動dirty化・Save連続性修正

## 1. 総合判定

```text
P26-008-PRE-002 COMPLETE
ROOT CAUSE CONFIRMED
COUNTRY AI CADENCE RESTORED
MILITARY AI CADENCE RESTORED
EVENT-DRIVEN DIRTY VALID
SAVE CONTINUITY PRESERVED
SAVE V1 PRESERVED
P26-008-A READY
```

## 2. 開始時HEAD

`bd73b0295a2139ae2f2d9e4fd90d36c828f9bbed`(期待値`bd73b02`と一致、`git rev-parse HEAD`で確認)。開始時`git status --short`は空(想定されていた`M saves/savegame_v1.ron`は、P26-008-PRE-001終了時点で既に復元済みのため存在しなかった――実際の状態をそのまま採用し、差分を作る操作は行っていない)。開始時テスト基準`2360 passed / 0 failed / 1 ignored`、clippy 0警告を実測で再確認した。

## 3. 修正前の評価回数

`verification_logs/phase-26/p26-008-pre-001/`が既に6か国36か月の実測で記録済み(216/216サンプルで`dirty==true`)。本タスクではこれを新たな計測はせず、その実測結果をそのまま根本原因の実証として引き継いだ(§4参照)。

## 4. 根本原因

`war::update::mark_ai_registries_dirty_daily`(`DailySimulationSet::WarResolution`、`handle_daily_war_resolution`の直後)が`DayChangedMessage`のたびに`CountryAiRegistry`/`MilitaryAiRegistry`の**全国家**を無条件で`mark_all_dirty()`していた。`process_daily_country_ai`の`is_weekly_due`/`is_monthly_due`判定、および`handle_daily_military_ai`の`is_evaluation_due`判定はいずれも`(周期条件) || dirty`という形をしており、`dirty`が常に`true`であればこの周期条件自体が無意味になる。

## 5. `dirty`の正規契約

`dirty == true`は「通常の定期評価日を待つと判断が古くなるような意味的状態変化が発生したため、次の評価機会で即時再評価が必要」を表す。日付が1日進んだだけ、または何らかのSystemが実行されただけでは`true`にしない。評価完了後に`false`へ戻り、次の実変化または定期評価まで`false`を維持する(既存の`is_weekly_due`/`is_monthly_due`/`is_evaluation_due`が評価後に`dirty=false`へ戻す既存ロジックはそのまま維持、変更なし)。

## 6. Country AIの正規cadence

- 建設AI(`process_construction_ai`): `current_day % 30 == country_id % 30 || dirty`(月次、既存式のまま変更なし)。
- 研究AI・戦争準備AI: 既存の週次stagger条件 `|| dirty`(既存式のまま変更なし)。
- 宣戦布告AI(`process_war_declaration_ai`): 日次・無条件(dirty/cadence判定の対象外、変更なし)。

## 7. Military AIの正規cadence

戦線計画AI(`evaluate_military_ai_decision`関連、`handle_daily_military_ai`): `current_day - last_evaluated_day >= 3 || dirty`(3日おき、既存式のまま変更なし)。

## 8. 募兵AI・宣戦布告AIの非変更

募兵AI(`handle_monthly_ai_recruitment`)は`MonthChangedMessage`駆動で`dirty`と無関係な独立Systemのため、変更していない(既存どおり正しく月次1回)。宣戦布告AIも日次無条件のまま変更していない。いずれも本タスクの対象外(§2どおり)。

## 9. 全状態変化イベントの監査

最低限、次の分類を行った(タスク文書§7の観点別)。

| 変化 | 分類 | 採用可否 |
|---|---|---|
| 戦争開始・終了(status/参加国変化) | 2(次の日に即時再評価が必要)、Country AI・Military AI両方 | 採用 |
| 州controller変更・州owner変更 | 2、Country AI・Military AI両方 | 採用 |
| 首都占領・首都奪還 | 州controller変更の特殊ケースとして自動的に包含(専用ロジック不要) | 採用(汎用化) |
| 前線作成・削除 | 6(既存の別Systemが既に処理している) — 戦争開始・終了と1:1で連動するため、戦争status差分が既に間接的にカバーする | 不採用(冗長) |
| 戦線目標変更・Division前線割当変更 | 6 — Military AI自身の書き込み結果であり、自身の再トリガーは不要 | 不採用 |
| Division生成・全滅・解散 | 1(定期評価まで待ってよい) — `assigned_division_ids`が空になっても`process_offensive_plan`等は何もしないため実害がなく、3日cadenceを再度無効化するリスクの方が大きい(§8参照) | 不採用 |
| 国家消滅 | 州owner変更(併合による所有権移転)+ 戦争status変更(関連戦争の強制終了)の組み合わせとして間接的に包含される | 不採用(冗長) |
| 同盟成立・解消、Crisis開始・終了、Claim追加・削除 | 1 — 週次cadenceで十分、外交プロセス自体が日単位で即応を要求しない | 不採用 |
| 建設完了・研究完了 | 1 — AI自身の出力であり、次の定期評価で拾えば十分 | 不採用 |
| 備蓄変化・treasury変化・manpower変化・戦闘損失・MilitarySupplies補給率変化 | 1 — 連続的な経済的drift。P26-008-Aが定期評価で消費する設計(P26-008-PRE-001)であり、event-drivenトリガー化するとcadenceの意図(負荷分散)を損なう | 不採用 |

## 10. 採用したdirty trigger

`war::ai_change_tracking::AiChangeSnapshot`による日次スナップショット差分方式(タスク文書§10の案C相当)。

- **州支配差分**: `StateData`ごとに`(controller(), owner_country_id)`を比較。差分があれば、変更前後の`controller`・`owner`の和集合(重複除去)をdirty化対象とする。
- **戦争差分**: `War`ごとに`(status, 参加国集合)`を比較。差分があれば、変更前後の参加国(攻撃側・防御側を区別しない和集合、重複除去)をdirty化対象とする。
- 対象は`CountryAiRegistry`と`MilitaryAiRegistry`の両方(§9の分類で「両方」に該当する2種類のみを採用したため)。

## 11. 不採用としたdirty trigger

前線作成・削除、戦線目標・割当変更、Division生成・全滅・解散、国家消滅単体、同盟・Crisis・Claim変化、経済的drift全般(§9の分類表参照)。単一の州支配差分・戦争差分の2系統だけで、これらの外部から見た結果(州の所有・支配が変わる、戦争が始まる/終わる)を漏れなく間接的にカバーできることを確認したため、追加のtrigger種別を増やさなかった(タスク文書§10「重複ロジックを作らない」の基準)。

## 12. 状態変化時の対象国家

- 州支配変化: 変更前controller・変更後controller・legal owner(3者のうち実際に異なる値を持つものの重複除去和集合)。
- 戦争変化: 変更前・変更後の参加国全員(攻撃側・防御側を区別しない和集合)。

いずれも`CountryId`昇順で処理する(§15参照)。

## 13. 無関係な第三国の扱い

上記2系統いずれも、変化に無関係な第三国は対象国家集合に一切含まれない(`AiChangeSnapshot::diff_affected_countries`の実装そのものが、変化があった`StateId`/`WarId`エントリの関係者だけを`HashSet`へ追加する構造のため、構造的に第三国を含み得ない)。単体テスト`unrelated_third_country_is_not_dirtied_by_a_controller_change_elsewhere`/`unrelated_third_country_is_not_dirtied_by_a_war_between_two_others`、およびE2E Test D/E/F/L(§16)で確認済み。

## 14. no-opの扱い

`AiChangeSnapshot`は値そのもの(`PartialEq`)を比較するため、同じcontrollerへの再設定・同じ戦争status/参加国の再確認はいずれも「差分なし」として扱われ、dirty化されない(単体テスト`recapturing_the_same_controller_is_a_no_op`/`reporting_the_same_active_war_again_is_a_no_op`、E2E Test K)。

## 15. 同日複数イベントの扱い

`mark_ai_registries_dirty_on_change`は1日に1回だけ現在スナップショットを構築し、前日スナップショットとの差分を一度に計算するため、同日中に複数の変化(例: 戦争開始+controller変更)が同じ国家へ及んでも、対象国家集合は`HashSet`による重複除去後の和集合となり、`mark_country_dirty`の呼び出しは冪等(何度呼んでも`dirty=true`のまま)。結果として翌日の評価は1回で足りる(E2E Test L)。

## 16. dirtyのSystem順序

`mark_ai_registries_dirty_on_change`は`DailySimulationSet::WarResolution`内、`handle_daily_war_resolution`の直後(旧`mark_ai_registries_dirty_daily`と同じ位置)に据え置いた。`DailySimulationSet`の宣言順(`... → MilitaryAction → WarResolution → ...`)により、その日の`MilitaryAction`(占領等)・`WarResolution`(講和・国家消滅・州所有権移転)で確定した最終状態を比較対象にできる。日Nの`WarResolution`で検出した変化は、日N+1の`CountryAi`/`MilitaryAi` Setで初めて読み取られる(同日中の二重評価・巻き戻しは発生しない、既存のSet順序を変更していないため)。

## 17. イベント取得方式

タスク文書§10の案C(WarResolution前後の差分比較)を採用した。理由:

- 州支配・戦争参加という2つの小さなデータ(28州・数件の戦争)だけを比較する軽量な日次差分であり、「毎日全世界を深く比較する」ものではない。
- `occupy_state`/`execute_peace_settlement`/`apply_country_elimination`/`declare_war`等、個々のDomain API関数を一切変更する必要がない(非侵襲的)――正規の状態変更経路がどこであっても、最終的な`StateRegistry`/`WarRegistry`の値を見るため、経路を漏らす心配がない。
- UI・テストからDomain APIを直接呼んだ場合も、次の日次tick時に同じ比較が働くため、経路によらず一貫して動作する。

## 18. `AiChangeSnapshot`はSaveへ保存しない

`AiChangeSnapshot`は前日終値のキャッシュであり、`capital_connectivity`(P26-006)と同じ「再計算専用」方針を採用した。Save/Loadでは`AiChangeSnapshot::default()`(未初期化)から再開する。未初期化を「まだ観測していない」と扱うブートストラップ規則(§19)により、この方針のままでも継続プレイとの決定性が壊れないことを確認済み(§23〜26)。

## 19. New Game初回評価

New Gameは既存の`CountryAiState::default()`/`MilitaryAiState::default()`の`dirty: true`初期値をそのまま維持した(変更なし)。`AiChangeSnapshot`は空(`state_control`が空)のため、`diff_affected_countries`は「未初期化」と判定して空集合を返す――New Game 1日目に州28件・戦争0件全てが「変化した」と誤認されて無関係な全国家がdirty化されることを防ぐ。初回評価自体は既存の`dirty: true`初期値だけで正しく発生する(E2E Test C)。

## 20. dirty=falseのsave→load

`SavedCountryAiState.dirty`/`SavedMilitaryAiState.dirty`を新設し(§28)、実際の値をそのまま保存・復元するように変更した。継続プレイでdirty=falseだった国家は、ロード後も`dirty=false`のまま復元され、次回評価日は継続プレイと完全に一致する(E2E Test N、§26参照)。

## 21. dirty=trueのsave→load

戦争開始等でdirty=trueのままsaveされた場合、ロード後もその値がそのまま復元されるため、次の評価機会(週次/月次/3日おきのいずれか)で正しく1回だけ評価される(E2E Test O)。

## 22. 旧Save

`SavedCountryAiState`/`SavedMilitaryAiState`に`#[serde(default = "default_dirty_true")]`を付与したため、このフィールドを持たない旧Saveは安全側の`true`(=次回評価機会で必ず1回再評価)から復元される。RON文字列に`dirty`フィールドを含まない手書きデータから直接デシリアライズする単体テストで確認済み(`saved_country_ai_state_without_the_dirty_field_deserializes_as_dirty_true`/同military版)、およびE2E Test P(実際のSaveファイルを模擬的に「旧形式」へ変換してロード)。

## 23. 2回連続save→load

`AiChangeSnapshot`は毎回のロードで確実に未初期化へリセットされ(§18)、`dirty`自体は忠実に往復するだけの単純な値であるため、2回連続のsave→loadでも状態が累積的に変化することはない(E2E Test Q)。

## 24. Save V1／V2判断

**Save V1を維持した(V2への移行なし)。** `SavedCountryAiState`/`SavedMilitaryAiState`への`dirty: bool`追加は、既存の`assigned_army_ids`/`offensive_line_region_ids`(`FrontlinePlan`)と同じ`#[serde(default)]`による加法的フィールドであり、`SAVE_FORMAT_VERSION_V1`の値そのものは変更していない。`AiChangeSnapshot`はSaveへ一切含まれないため、`SaveGameV1`のトップレベル構造(フィールド一覧)にも変更はない。

## 25. 新規Saveフィールド

`SavedCountryAiState.dirty: bool`(`#[serde(default = "default_dirty_true")]`)、`SavedMilitaryAiState.dirty: bool`(同上)の2フィールドのみ。`AiChangeSnapshot`自体はSaveされない(§18)。

## 26. 決定性

`AiChangeSnapshot`の内部格納には`HashMap`/`HashSet`を用いている(`CountryId`/`StateId`/`WarId`が`Ord`を実装しないため、このコードベース全体の既存規約に合わせて`BTreeMap`ではなく`HashMap`+明示的ソートを採用)。`diff_affected_countries`が返す`Vec<CountryId>`は、内部で一度`HashSet`へ集約した後、必ず`.sort_by_key(|c| c.0)`で確定的に整列してから返す。`mark_ai_registries_dirty_on_change`はこの整列済みVecを順に`mark_country_dirty`するため、呼び出し順序自体も決定的。

## 27. イベント順序非依存

単体テスト`affected_country_set_does_not_depend_on_hashmap_insertion_order`で、州・戦争の構築順序(`WarRegistry.wars`への挿入順)を逆にしても、最終的なdirty対象国家集合が完全一致することを確認済み。E2E Test M相当は、この単体テストでより厳密に(pureな関数レベルで)検証しているため、E2Eでの重複検証は行っていない(既存のP26-006-REV-001シナリオ再現[E2E Test S]で、実App経由の決定性も別途確認)。

## 28. 36か月建設履歴への影響・29. 研究履歴への影響

建設AIの評価回数が設計意図の頻度(月次)へ戻った結果、**一部の国家(Arcadia・Elfin)の36か月建設履歴・生産の絶対値が変化した**。これは§32〜35で詳述する、テスト側の暦簡略化(`day2〜28`)がArcadia(`CountryId(0)`、月次stagger残余`day%30==0`)・Elfin(`CountryId(1)`、残余`day%30==1`)の評価機会を実質的に「New Game直後の初回評価のみ」に制限してしまっていたことによる副作用であり、実ゲームプレイ(`DayChangedMessage`がday1〜31全てで発行される)では再現しない、テスト固有の問題である(§32以降で全面的に扱う)。研究AI自体の履歴は本タスクの直接の検証対象にしていないが、cadence自体の式は建設AIと同じ`|| dirty`パターンであり、同じ原理で正規cadenceへ復帰している。

## 30. 戦争準備への影響・31. 戦線スタンスへの影響

週次cadence(研究AI・戦争準備AI)・3日おきcadence(戦線計画AI)についても、建設AIと全く同じ理由(`|| dirty`の無条件`true`化)で修正され、全く同じ理由(day1のみ残余1[Elfin]、day30のみ残余0[Arcadia]という、日次tick範囲`day2〜28`に含まれない残余)で、Elfin/Arcadia関連のテストのみ副作用を受けた。詳細測定は行っていないが、修正の性質上、他の4か国(残余2〜5、常に`day2〜28`内)の週次・3日おきcadenceはテスト側の暦簡略化による影響を受けない。

## 32〜35. 回帰調査で発見した4件の追加影響と対処

本タスクの主目的(cadence修正)を実装した後、`cargo test --all-targets`を繰り返し実行し、**cadence修正の直接的な副作用として、既存4テストファイルの5テストが退行した**ことを発見した。いずれも「正規cadenceへ戻ったことの論理的帰結」であり、実装のバグではないと判断し、原因を特定した上でテスト側を修正・更新した(タスク文書§18の明示的な許可事項)。

### 32.1 原因分類A: テストの暦簡略化がArcadia/Elfinの評価機会を奪っていた(3ファイル)

`process_daily_country_ai`の月次stagger判定は`current_day % 30 == country_id % 30`。このテスト群の暦(`event.year*365 + event.month*30 + event.day`、`year`は全テスト共通で1800固定)では、これは`day % 30`と数学的に等価になる。多くのE2Eテストが使う簡略暦`for day in 2..=28u8`(1か月=27日)は、**`day==30`(Arcadia、`CountryId(0)`)と`day==1`(Elfin、`CountryId(1)`)のいずれにも到達しない**。旧`mark_ai_registries_dirty_daily`はこの2か国についても無条件で毎日dirty化していたため、この暦の欠落が問題にならなかった。イベント駆動化した結果、Arcadia/Elfinの建設AI・研究AI・戦争準備AIは、New Game直後の初回評価(`dirty`初期値`true`)以降、二度と定期評価されなくなった(実際に州支配・戦争参加の変化がこれらのテストで一切起きないため)。

**影響を受けたファイルと修正**:

| ファイル | 影響 | 修正 |
|---|---|---|
| `tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs` | `test_k1`(Elfin、MilitarySupplies生産0のまま)、`test_m1`(同じ理由でElfinのassertionが失敗) | `advance_one_month`/`advance_one_month_tracking_supply_consumption`/`test_m1`内のinline loopを`day1〜30`へ拡張 |
| `tests/p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test.rs` | `test_i_j_...`(Elfin、同じ理由) | 同上(2つの共有helperを`day1〜30`へ拡張) |
| `tests/p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs` | `test_f2_arcadia_deterministic_production_values_are_protected_after_the_initial_grant`(Arcadia、旧canonical値421.226523/592.5978と不一致) | `test_f2`内のinline loopを`day1〜30`へ拡張 |

いずれも、暦をday1〜30へ拡張しただけで(canonical値・assertion自体は一切変更せず)、**元の正規値が寸分違わず復元される**ことを確認した(`test_f2`は拡張前378.074…→拡張後421.226523、既存定数と完全一致)。これは「実ゲームプレイと同じ日次網羅性を与えれば、cadence修正後も元の意図通りの結果になる」という強い証拠であり、テスト側の暦簡略化そのものが原因だったことを確定的に裏付ける。

### 32.2 原因分類B: cadence頻度そのものの低下による正当な経済的タイムライン変化(2ファイル)

Oceanic(`CountryId(3)`、残余`day%30==3`)は`day2〜28`の範囲内に元々評価機会を持つため、分類Aの対象ではない。しかし、月1回だけの正規cadence自体が、旧「毎日最大27回評価」より建設判断のタイミング解像度が粗くなるため、**同じ経済的帰結へ到達するまでの時間が実際に変化する**(判断そのものは正しいが、より遅く行われる)。

| ファイル | 変化した値 | 旧値 → 新値 |
|---|---|---|
| `tests/p26_006_capital_connectivity_supply_e2e_test.rs::test_36_month_oceanic_shortage_unchanged_when_uncontested_by_capital_connectivity` | Oceanicの36か月shortage_days(`first_shortage_day`は32のまま不変) | 418 → 493 |
| `tests/p26_det_001_construction_ai_determinism_e2e_test.rs::real_app::test_f_all_six_countries_stable_within_process_and_no_regression` | Oceanicの月36 MilitarySupplies production_only | 565.705 → 579.803051 |
| `tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs::real_app::test_i_all_six_countries_stable_within_process_and_no_regression` | 同上(Oceanic) | 565.705 → 579.803051 |

上記2ファイルの同じ2テストは、Elfin(残余1、分類Aにも該当)の値も同時に更新した(`1846.952` → `1785.448052`)。Elfin側は`day1〜30`拡張と値更新の**両方**を要した――拡張だけでは(Arcadiaと違って)旧値に戻らなかった。原因は、Elfinの生産網がIron(Mine、既存の事前開発規則)とIndustrialGoods(Factory、同規則を汎用資源探索で適用)という**2種類の事前開発判定を別々の月次評価で順番に解消する必要がある**ためである(P26-004-PRE-002 `find_proactive_mine_state`が`ResourceType::ALL`宣言順で1回の評価につき1資源しか対応しない設計、既存仕様・変更なし)。旧「毎日評価」下では同一月内に2回以上評価が走るため両方を解消できたが、月1回だけの正規cadenceでは解消に2か月かかる――結果、生産開始が実質1か月遅れ、月36時点の累積値がArcadiaほど正確には旧値と一致しない(Arcadiaは単一資源[Iron]の事前開発だけで完結するため、1回の評価で完結し、旧値と完全一致する)。

いずれの新値も、**独立プロセス3回で完全に再現する**ことを確認した(決定的)。旧`418`/`565.705`/`1846.952`という数値は、いずれも過去複数の報告書(P26-004-PRE-001、P26-006、P26-008-PRE-001)で繰り返し引用されてきた既知の参照値であるため、単純に上書きするのではなく、原因を各テストファイル内のコメントとして明示し、本報告書にも独立してまとめた(§46で改めて要約)。

## 36. Food緊急の保護・37. 鉱床事前開発の保護

本タスクは`process_construction_ai`自体のロジック(優先度1[Food]・優先度2[事前開発Mine]・優先度3[反応的判断])を一切変更していない。§32〜35の変化はいずれも「いつ評価するか」(cadence)の修正、および「テストが実際に何日分シミュレートするか」(暦簡略化の是正)によるものであり、建設AIの判断ロジック自体は無変更である。

## 38. 6か国の現行値

P26-008-PRE-001の実測(6か国・36か月・`PlayerCountry(None)`)は、当時の(修正前)dirty常時trueの状態で得られたものであり、本タスクによる修正でその結果自体が変わり得る。本タスクでは新たな6か国36か月の網羅的再測定は行っていない(必須ではなく、§32〜35で発見・修正した5件の既存テストの再現性確認で十分と判断した)。

## 39〜41. Elfin/Oceanicへの影響、Dwarfとの比較

§32〜35で詳述した通り、Elfin/Oceanicの36か月MilitarySupplies生産軌道はcadence修正により変化した(Elfin: 生産網構築が正しく機能した上で1か月分の遅れが生じる、Oceanic: 同様に月次cadenceの解像度低下による遅れ)。Dwarf(既存の完全な供給網を持つ国家、`CountryId(2)`、残余2、`day2〜28`内)はいずれのテストでも値の変化が報告されておらず、cadence修正の影響を受けていない――Dwarfは月内の早い段階で構築判断が完了し切っているため、評価頻度が減っても結果に影響しないと考えられる。

## 42. Critical/High/Medium/Lowの所見

- **Critical: 0件**
- **High: 0件**
- **Medium: 1件**(新規発見) — 本プロジェクトの多数のE2Eテストファイルが共有する簡略暦(`for day in 2..=28u8`、1か月=27日)は、`CountryId % 30 ∈ {0, 1}`(Arcadia・Elfin)に該当する国家の月次stagger残余(`day==30`・`day==1`)を**構造的に一度も生成しない**。旧`mark_ai_registries_dirty_daily`のバグにより偶然無害化されていたが、本タスクの修正によって顕在化した。本タスクで発見した5テスト(§32〜35)は修正済みだが、**この暦簡略化パターンを使う他の未検証テストファイルに、同種の(まだ顕在化していない)潜在的な弱さが残っている可能性がある**――`cargo test --all-targets --no-fail-fast`を2回実行し、発見された失敗をその都度修正して最終的に0失敗を達成したが、本タスクが実際に触れていないテストファイルまで全数監査したわけではない。将来Arcadia/Elfinの経済的挙動に依存する新しいテストを書く際は、`day1〜30`(またはそれに相当する完全な暦)を使うべきである。
- **Low: 0件**

## 43. 全テストpassed/failed/ignored件数

最終`cargo test --all-targets --no-fail-fast`: **2391 passed / 0 failed / 1 ignored**(66テストバイナリ全て実行、開始時2360 + 新規lib単体テスト15 + 新規E2E 16 = 2391、過不足なく一致)。

## 44. clippy警告数

`cargo clippy --all-targets --all-features -- -D warnings`: **0 warnings**。

## 45. Cargo逐次実行

すべてのcargoコマンドを逐次実行した(並列実行なし)。作業途中で一度、無関係な同時アクセス(IDEのrust-analyzerによるバックグラウンドチェックと推測される)による`STATUS_STACK_BUFFER_OVERRUN`リンカ破損が発生し、`cargo clean -p strategy_game`で復旧した(`feedback_concurrent_cargo_runs_corrupt_cache`の既知パターン、コード上の問題ではない)。

## 46. 独立プロセス結果

- 新規`p26_008_pre_002_ai_evaluation_cadence_e2e_test`: 独立プロセス5回、各16 passed / 0 failed。
- `p26_006_rev_001_ai_save_resume_determinism_e2e_test`: 独立プロセス5回、各9 passed / 0 failed。
- `p23_010_phase23_manpower_lifecycle_audit_test`: 独立プロセス5回、各9 passed / 0 failed。
- §32〜35で修正した5テストの新しい数値(Oceanic 493/579.803051×2、Elfin 1785.448052×2)は、それぞれ独立プロセス3回で再現を確認済み(決定的)。

## 47. 対象外

MilitarySuppliesを読むAI判断(募兵停止・攻勢停止・IndustrialGoods向けFactory優先)、AI募兵ターゲット式の変更、募兵キューの装備不足停滞の修正、宣戦布告AIの仕様変更、戦闘・補給バランスの変更、UIはいずれも実装していない(P26-008-A/B/Cは対象外のまま)。

## 48. 既知のMedium findingの非変更

P26-008-PRE-001で確認した「MilitaryEquipment不足時、募兵キューが完了できず新規募兵も永久停止する」問題には一切触れていない――装備不足Divisionの強制生成、募兵キューの自動削除、Equipment要件の緩和、AI募兵ターゲット式の変更、MilitaryEquipmentの外生付与のいずれも行っていない。

## 49. P26-008-PRE-001保護

P26-008-PRE-001は報告書のみのタスクで本番コード変更が無かったため、直接の回帰対象コードはない。同報告書が確立した設計(§18〜29の指標・目標値・ヒステリシス案)自体には一切変更を加えていない。

## 50. P26-004〜007回帰

`p26_007_elimination_occupation_overseas_supply_accounting_e2e_test`(23 passed)、`p26_006_capital_connectivity_supply_e2e_test`(18 passed、うち1件はcanonical値更新済み)、`p26_005_supply_combat_strength_e2e_test`(10 passed)、`p26_004_daily_supply_consumption_allocation_e2e_test`(19 passed)、`p26_004_pre_005_initial_supply_bridge_migration_e2e_test`(22 passed、うち1件は暦拡張のみで既存canonical値を維持)を個別に実行し、全て合格を確認した。

## 51. P26-006-REV-001回帰

`p26_006_rev_001_ai_save_resume_determinism_e2e_test`(9 passed、独立プロセス5回)で、州1の占領競合・Division移動・Frontline処理の決定性が維持されていることを確認した(E2E Test S、`test_s_contested_border_state_outcome_is_stable_across_repeated_runs`でも本タスク自身のE2Eファイル内に同種のシナリオを追加検証)。

## 52. P26-DET回帰

`p26_det_001_construction_ai_determinism_e2e_test`(10 passed)、`p26_det_001_rev_001_cross_state_production_order_e2e_test`(12 passed)。いずれもElfin/Oceanicのcanonical値を§32〜35の通り更新した上で全合格。

## 53. Phase 23〜25回帰

`p23_010_phase23_manpower_lifecycle_audit_test`(9 passed、独立プロセス5回)、`p23_009_ai_recruitment_e2e_test`(21 passed)、`p24_002_workforce_rounding_unification_e2e_test`ほかPhase23/24/25全ファイル、`land_war_combat_peace_test`(保護対象、変更なし、4 passed)を含め、`cargo test --all-targets --no-fail-fast`の最終結果(2391/0/1)で網羅的に確認した。

## 54. 対象外(重複、§47参照)

## 55. PNG差分処理

テスト実行により既知の非決定的PNG(p20-007、p20-009、phase-21/p21-save-002e、計9ファイル)が再生成された。都度`git status --short`で対象を確認し、ファイル名を個別指定した`git checkout --`で復元した(広範なcheckoutは使用していない)。

## 56. ユーザーセーブの保護

`saves/savegame_v1.ron`は開始から終了まで一度も読み書き・ステージ・復元していない(全テストの一時Save/LoadはOS一時ディレクトリ内の`TempTestDir`にのみ書き込む、既存パターンを踏襲)。

## 57. コミット対象・最終`git status --short`・P26-008-Aへ進める状態か

コミット対象は以下13ファイル(全て個別ステージ、`git add -A`/`.`は使用しない):

- 新規: `src/war/ai_change_tracking.rs`
- 新規: `tests/p26_008_pre_002_ai_evaluation_cadence_e2e_test.rs`
- 新規: 本報告書
- 変更: `src/save/apply.rs`, `src/save/dto.rs`, `src/save/export.rs`, `src/save/runtime.rs`, `src/save/validate.rs`, `src/war/mod.rs`, `src/war/update.rs`
- 変更(§32〜35の暦拡張・canonical値更新): `tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs`, `tests/p26_004_pre_004_minimum_supply_chain_data_migration_e2e_test.rs`, `tests/p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs`, `tests/p26_006_capital_connectivity_supply_e2e_test.rs`, `tests/p26_det_001_construction_ai_determinism_e2e_test.rs`, `tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs`

コミット後の最終`git status --short`は空(実測確認済み)。コミットハッシュ: `3944cd8f4b512308384a8e8d4b5eda9737c998a1`。

**P26-008-A READY**: 本タスクにより、AI評価cadenceが正規の週次/月次/3日おき頻度へ復帰し、州支配・戦争変化への即時再評価が関係国だけに限定され、Save連続性(継続プレイとロード後の次回評価タイミング一致)が保証された。P26-008-PRE-001が設計した募兵ゲート・攻勢ゲート・建設優先度拡張は、この正しいcadence基盤の上に実装できる状態にある。
