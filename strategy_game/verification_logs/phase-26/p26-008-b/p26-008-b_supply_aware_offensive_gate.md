# P26-008-B: 補給状態に基づくAI攻勢停止・再開ゲート実装

## 1. 総合判定

```
P26-008-B COMPLETE
SUPPLY METRICS PRESERVED
OFFENSIVE START GATE VALID
OFFENSIVE CONTINUATION GATE VALID
TRUE HYSTERESIS IMPLEMENTED
FRONTLINE ISOLATION VALID
DEFENSE AND RETREAT PRESERVED
SAVE CONTINUITY PRESERVED
SAVE V1 PRESERVED
AI CADENCE PRESERVED
P26-008-C READY
```

## 2. 開始HEAD

`9127519` (完全形: `91275197cd33691b5062a8058f335a02cc203941`、`docs: record final commit hash in p26-008-a report`)。開始時`git status --short`はクリーン。開始時テスト基準を実測: `cargo test --all-targets` → **2441 passed / 0 failed / 1 ignored**。`cargo clippy --all-targets --all-features -- -D warnings` → **0 warnings**。

## 3. 監査した実際の軍事AI経路

`src/war/military_ai.rs::handle_daily_military_ai`(`DayChangedMessage`受信、`DailySimulationSet::MilitaryAi`) → `process_daily_military_ai`(国家単位のcadence判定、`current_day - last_evaluated_day >= 3 || dirty`) → 各国×各Active War×対応するFrontlineについて `evaluate_powers` → `select_ai_offensive_objective` → **`evaluate_military_ai_decision`**(スタンス決定本体)。

`FrontlinePlan`は`(FrontlineId, CountryId)`をキーとする——1つの前線の両陣営がそれぞれ独立したPlanを持つ既存構造であり、本タスクの新規状態(§17)を配置する単位として自然に一致することを確認した。

`DailySimulationSet`の既存順序(`Economy → Research → Diplomacy → CountryAi → WarPreparation → MilitaryAi → FrontlineOrders → MilitaryAction → WarResolution → UiUpdate`)により、P26-004の日次補給消費(`Economy`)は常にMilitaryAiより前に確定している——本タスクのために変更する必要はなかった(§21)。

## 4. 攻勢の正規定義

`FrontlinePlan.stance == FrontlineStance::Offensive`。個別移動命令・進行中Battle・退却処理を別の「攻勢ゲート」入口として追加していない。`handle_daily_frontline_plans`(`process_offensive_plan`/`process_defensive_plan`/`process_stopped_plan`への分岐)は無変更のまま——本タスクは`evaluate_military_ai_decision`が書き込む`plan.stance`の値だけを変え、その後続処理には触れていない。

## 5. 既存110%/90%条件

`can_start_offensive`(`own*1000 >= enemy*1100`)・`can_continue_offensive`(`own*1000 >= enemy*900`)は一切変更していない。補給ゲートはこれらへのAND条件として追加した。

## 6. 国家備蓄指標の再利用方法

`src/war/offensive_supply_gate.rs::evaluate_offensive_supply_status`が、P26-008-Aの`military::recruitment_supply_gate`から次をそのまま再利用する: `has_reserve_for_days`・`country_daily_demand_units`(総需要、接続性とは無関係)・`update_recruitment_supply_gate`(ヒステリシス状態機械本体)。攻勢ゲートにはbridge予測という概念自体が存在しないため、`update_recruitment_supply_gate`へ`bridge_deficit_units=0`(常に「bridge不足なし」)を渡す形で、ヒステリシス状態機械のコードを複製せずに再利用した。

## 7. stop/resumeしきい値の再利用方法

`stop_days_for(is_at_war)`・`resume_days_for(is_at_war)`(P26-008-Aで定義済みのpub関数、値は平時20/35日・戦時40/70日)をそのまま呼ぶ。攻勢コード側に40や70を直接書いていない。`war_registry.is_country_at_war(country.id)`(既存API)で`is_at_war`を導出する。

## 8. 前線別min fulfillmentの対象集合

P26-008-Aの`min_fulfillment_of_positive_demand_divisions`から、需要0除外・最小値計算のループを`min_fulfillment_over_division_ids`(新規、`src/military/recruitment_supply_gate.rs`に追加、既存20テスト無変更で全通過を確認済み)として抽出し、`offensive_supply_gate::frontline_min_fulfillment`がこれを前線の`assigned_division_ids`だけに絞って呼ぶ。対象: 前線へ割当済みかつ生存中(`manpower>0 && status!=Destroyed`)かつ需要正。対象外: 別前線のDivision(そもそも`assigned_division_ids`に含まれない、`FrontlineRegistry`の既存契約により1 Division=1 Frontline)、消滅Division、需要0のDivision。

## 9. 需要0Divisionの扱い

`division_daily_demand_units`が0を返すDivisionは最小値の母集合から除外する(既存のP26-008-Aと同じ規約)。

## 10. 対象Divisionなしの扱い

`frontline_min_fulfillment`が`None`を返す場合、`is_frontline_delivery_healthy(None) == true`(「配送不足signalなし」、健全扱い)。

## 11. 0.34境界

`MIN_OFFENSIVE_SUPPLY_FULFILLMENT: f64 = 0.34`。単体テストで境界を確認: `0.339999...`→不健全、`0.34`ちょうど→健全、`1.0`→健全。CountryId・戦争・前線・国家Tierによって変化しない単一の正規値。

## 12. 新規攻勢開始条件

既存110%判定に加え、`!offensive_supply_blocked`(更新後の値)をAND条件として要求する。`offensive_supply_blocked`は`update_recruitment_supply_gate`の結果そのもの(国家備蓄・前線配送の両方を内包)であり、これ単独で「国家備蓄stop以上」と「前線配送0.34以上」の両方を表現する——冗長な二重チェックを避けるため、コード上は`blocked_after`1個の判定に統一している(§16参照)。

## 13. 既存攻勢継続条件

既存90%判定に加え、`delivery_fully_healthy`(その評価時点の前線配送健全性、`offensive_supply_blocked`とは別に毎回フレッシュに計算)を直接参照する。**永続化された`offensive_supply_blocked`は継続判定には使わない**——これがspec §7の核心である。

## 14. 国家備蓄不足だけでは既存攻勢を止めないこと

E2E `test_06_national_reserve_dropping_below_stop_alone_does_not_stop_an_ongoing_offensive`で確認: 攻勢開始後に国家備蓄をstop未満へ落としても(配送は健全なまま)、スタンスは`Offensive`のまま維持される。ただし`offensive_supply_blocked`自体は(将来の新規開始をゲートするため)`true`になる——これは意図した仕様であり、バグではない。

## 15. 配送不足時の攻勢停止

E2E `test_05_ongoing_offensive_stops_when_frontline_fulfillment_drops_below_0_34`で確認: 前線配送率が0.34未満になると、次の正規AI評価(cadence境界)でスタンスが`Defend`へ変わる。

## 16. 真のヒステリシス

### 16.1 状態の保存単位

`FrontlinePlan.offensive_supply_blocked: bool`(`#[serde(default)]`)。`CountryAiState.recruitment_supply_blocked`(募兵停止、P26-008-A)とは完全に独立した別状態であり、一方を他方の代替として流用していない。

### 16.2 PRE-001/P26-008-Aの設計との整合性確認

P26-008-A自身が既に「国家備蓄ヒステリシスの中間帯保持には専用状態が必要」と結論づけており、本タスクはその同じ設計判断を前線単位へ適用しただけである——新たな設計是正は発生していない。ただし配置は`CountryAiState`(国家単位)ではなく`FrontlinePlan`(前線×国家単位)であり、`FrontlineRegistry::remove_frontline`が既存どおりPlan全体を削除するため、専用の「dangling clean-up」ロジックを追加する必要がなかった(§20)。

### 16.3 状態遷移

```rust
pub fn update_recruitment_supply_gate(
    currently_blocked, is_at_war,
    current_stockpile_units, current_total_demand_units,
    bridge_deficit_units, // 攻勢ゲートでは常に0
    delivery_fully_healthy,
) -> bool
```
(P26-008-Aの既存関数を無変更のまま再利用、`src/war/offensive_supply_gate.rs::evaluate_offensive_supply_status`から呼ぶ)。

## 17. 中間帯の2状態比較結果

単体テスト`middle_band_preserves_prior_blocked_state`(`src/war/offensive_supply_gate.rs`)、E2E `test_07_middle_band_keeps_recovery_wait_state`で確認: 国家備蓄・配送率・戦力比が完全に同一(戦時stop=40〜resume=70の中間帯)でも、直前の`offensive_supply_blocked`の値によって、新規攻勢が「継続許可」か「再開待ち維持」かに分岐する。

## 18. 首都切断結果

抽象的な切断表現(`division_supply_fulfillment`を直接0.0にする、P26-008-Aと同じ抽象度)を使う単体・E2Eテスト(`test_10`)に加え、**90日実App検証**では実際のP26-004日次補給System(`daily_supply::process_daily_military_supply_consumption`)自身のHamilton配分(`T_c = min(S_c, ΣD_i)`)を通して、国家備蓄を0へ落とすことで実際に配送率が0へ落ちることを確認した(直接`division_supply_fulfillment`を上書きする方式は、実App上ではEconomy Setの同じフレームで即座に再計算・上書きされてしまうため機能しないことを実際に検出し、修正した——§28参照)。

## 19. 接続回復・再開結果

E2E `test_11_reconnection_does_not_immediately_resume_if_reserve_still_below_resume`で確認: 配送を健全化しても、国家備蓄がresume未満のままなら攻勢を再開しない。E2E `test_08`で確認: 配送・国家備蓄の両方がresume条件を満たし、既存110%条件も満たすと再開する。E2E `test_09`で確認: resumeに達しても110%条件を満たさなければ再開しない(ただしblocked自体は解除される)。

## 20. 防御・退却・Battleの保護

`process_offensive_plan`/`process_defensive_plan`/`process_stopped_plan`・退却処理・`BattleRegistry`関連コードのいずれも変更していない。本タスクの変更は`evaluate_military_ai_decision`が返す`(FrontlineStance, MilitaryAiDecisionReason, bool)`の決定内容のみであり、それを消費する側のSystemには触れていない。

## 21. 複数Frontline分離

E2E `test_12_disconnected_frontline_does_not_contaminate_a_healthy_sibling_frontline`で確認: 同一AI国家が2つの独立した前線(別々の戦争)を持つ場合、一方の配送不足が他方のPlanの`offensive_supply_blocked`/スタンスへ混入しない。

## 22. 国家間分離

E2E `test_13_one_countrys_supply_shortage_does_not_affect_another_country`で確認: ある国家の備蓄不足が、完全に無関係な別の国家×前線の攻勢開始可否へ影響しない。

## 23. 決定性

`min_fulfillment_over_division_ids`・`country_daily_demand_units`はいずれも既存の`DivisionId`昇順ソート後の走査を維持。E2E `test_19_result_does_not_depend_on_country_insertion_order`で`CountryRegistry`への登録順反転が結果を変えないことを確認。

## 24. System順序

`daily_supply`(`Economy`)→`MilitaryAi`の既存順序を確認し(§3)、変更していない。`Res<GameDate>`のようなresourceに依存していない(本ゲートは`Res<CountryRegistry>`/`Res<WarRegistry>`/`Res<MilitaryRegistry>`のみ読む)。

## 25. cadence保護

`process_daily_military_ai`の既存3日/dirty cadence判定コード(`is_evaluation_due`)を変更していない。`AiChangeSnapshot`・`mark_ai_registries_dirty_on_change`のいずれも変更していない。E2E `test_20_supply_recovery_within_the_cadence_window_does_not_trigger_an_extra_evaluation`で、備蓄回復のような補給状態の変化だけではcadence境界を早めないことを確認した。

## 26. 90日実App検証

`setup_app(None, ...)`(`PlayerCountry(None)`)で実データ上にArcadia(CountryId 0)対Elfin(CountryId 1)の制御された戦争を作成(戦争目標なし、`select_ai_offensive_objective`の候補2[前線最短の敵支配地域]へフォールバック)し、以下の一連の流れを実際のSystem経由で確認した:

1. 十分な国家備蓄・戦力比110%以上 → Offensive開始。
2. 国家備蓄を0へ落とす(実際の日次補給Systemが配送率を実際に0へ低下させる) → Offensiveを維持できない。
3. 国家備蓄を回復 → 実際の日次補給Systemが配送率を再計算し、最終的にOffensiveへ復帰。

診断用の無限資金フィクスチャは使用していない。戦力比110%/90%・補給率・本番データ・国家別特例のいずれも変更せず、結果を成立させるために変更した唯一の要素はArcadiaの陸軍manpower(既存のDivision増強の範囲内、110%優位を確実にするための操作)である。

## 27. 既存テスト変更と理由

新ゲート導入により2件の既存テストが影響を受けたため、**フィクスチャ(前提条件)のみ修正し、アサーションは一切変更していない**:

1. `src/war/military_ai.rs::tests::setup_test_env`——C2(AI国家)のMilitarySupplies備蓄が`CountryData::default()`の0のままだったため、新ゲートが即座にblockedと判定し、`test_ai_decision_thresholds_and_hysteresis`が期待する`Offensive`(戦力比2000:500のみに基づく既存の検証意図)を`Defend`に変えてしまっていた。十分なMilitarySupplies備蓄(1,000,000)を明示的に与えるよう修正した。
2. `tests/daily_system_integration_test.rs::test_d_next_day_military_action`——New Game既定の初期MilitarySupplies付与(31.0、P26-004-PRE-005)は、開戦直後の戦時stopしきい値(40日)を満たせず、`SufficientAdvantage`を期待するアサーションが`SupplyShortage`に変わってしまっていた。同じく十分な備蓄(1,000,000)を明示的に与えるよう修正した。

いずれも「このテスト自体は補給状態を検証対象にしていない、旧来の暗黙の前提(補給は無限)だけが原因」に該当し、CountryId特例やゲート無効化、アサーション弱体化のいずれも行っていない。

## 28. 実装過程で発見・修正した3つの罠(記録として)

1. **`SkippedTargetMet`類似の早期returnの罠は本タスクには存在しない**——`evaluate_military_ai_decision`は既存どおり毎回呼ばれるが、供給ゲート評価自体は早期return(`NoValidFrontline`等)の前に置き、全ての分岐で`offensive_supply_blocked`を返すよう設計した(§8.2どおり)。
2. **手動でのWarId割当がSave検証を壊す**——テスト内で`WarId(war_registry.wars.len())`を手で組み立ててから`wars.insert`すると、`WarRegistry`内部の`next_id`カウンタが更新されず、Save時の`NextIdCollision`検証エラーになる。既存の`WarRegistry::add_war`(正規ID発行API)を使うよう修正した。
3. **実App上で`division_supply_fulfillment`を直接上書きしても長続きしない**——実Appでは`Economy` SetのP26-004日次補給Systemが毎フレーム配送率を再計算するため、テストが手動で設定した値は次の`app.update()`で上書きされてしまう。90日検証では、直接上書きではなく国家備蓄そのものを操作し、実Systemに配送率を計算させる方式へ修正した。

## 29. P26-004〜008-A回帰

個別に実行し全て0 failedを確認: `p26_008_a_supply_aware_recruitment_gate_e2e_test`(30 passed)、`p26_008_pre_002_ai_evaluation_cadence_e2e_test`(16 passed)、`p26_007_elimination_occupation_overseas_supply_accounting_e2e_test`(23 passed)、`p26_006_rev_001_ai_save_resume_determinism_e2e_test`(9 passed)、`p26_006_capital_connectivity_supply_e2e_test`(18 passed)、`p26_005_supply_combat_strength_e2e_test`(10 passed)、`p26_004_daily_supply_consumption_allocation_e2e_test`(19 passed)。

## 30. Phase 23〜25回帰

`cargo test --all-targets`の全体実行に包含済み(§32参照)。個別`--lib military::`/`war::`/`country::`/`economy::`/`save::`もこの全体実行に含まれ、いずれも0 failed。`land_war_combat_peace_test.rs`は無変更のまま保護。

## 31. 独立プロセス安定性

5回ずつ独立プロセスで実行し、いずれも完全に同一の結果:
- `p26_008_b_supply_aware_offensive_gate_e2e_test`: 19 passed ×5
- `p26_008_pre_002_ai_evaluation_cadence_e2e_test`: 16 passed ×5
- `p26_006_rev_001_ai_save_resume_determinism_e2e_test`: 9 passed ×5
- `p23_010_phase23_manpower_lifecycle_audit_test`: 9 passed ×5

## 32. 全テストpassed/failed/ignored

| 項目 | 開始時 | 完了時 |
|---|---|---|
| `cargo test --all-targets` | 2441 passed / 0 failed / 1 ignored | **2476 passed / 0 failed / 1 ignored** |
| 新規単体テスト(`src/war/offensive_supply_gate.rs`) | — | 16件 |
| 新規E2Eテスト(`tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs`) | — | 19件 |

差分検算: 2441 + 16 + 19 = 2476(一致)。

## 33. clippy警告数

`cargo clippy --all-targets --all-features -- -D warnings`: 開始時0 warnings → 完了時**0 warnings**。

## 34. Cargo逐次実行

全てのcargoコマンドを逐次(単一プロセス)で実行した。並列実行は行っていない。

## 35. Critical/High/Medium/Low

Critical/High findingなし。Medium findingなし(§28の3件は実装過程で発見し、いずれもその場で修正済みであり、残存する既知の問題として記録するものではない)。Low finding: なし(既知のEquipment起因queue停滞[P26-008-A §17由来]は本タスクの対象外のまま非変更、新たな影響は確認されなかった)。

## 36. 対象外を先行実装していないこと

P26-008-C(建設優先度)、募兵ゲート(P26-008-A)のしきい値変更、`recruitment_supply_blocked`の流用、`mark_ai_registries_dirty_daily`の復活、宣戦布告AI、戦闘式、補給会計式そのもの、港/海上輸送/鉄道/補給距離、自国領内移動の禁止、退却制限、進行中Battleの中断、プレイヤー操作への補給ゲート、UIのいずれも変更していない。

## 37. ユーザーSave保護

`saves/savegame_v1.ron`へは一切アクセスしていない。

## 38. PNG差分処理

`cargo test`実行に伴い、既知の非決定的スクリーンショット9ファイルが再生成され`git status`上で変更として現れたが、個別に`git checkout --`で復元した。

## 39. コミット対象

```
 M assets/localization/en-US.ron
 M assets/localization/ja-JP.ron
 M src/military/recruitment_supply_gate.rs
 M src/save/apply.rs
 M src/save/dto.rs
 M src/save/export.rs
 M src/save/validate.rs
 M src/war/frontline.rs
 M src/war/military_ai.rs
 M src/war/mod.rs
 M tests/daily_system_integration_test.rs
?? src/war/offensive_supply_gate.rs
?? tests/p26_008_b_supply_aware_offensive_gate_e2e_test.rs
```

`assets/localization/{en-US,ja-JP}.ron`への1行追加(`military_ai_reason.supply_shortage`)は、新規`MilitaryAiDecisionReason::SupplyShortage`variantが`src/ui/military_panel.rs`で実際にレンダリングされる既存の網羅的パイプライン(P26-002-REV-001と同じ構造の要件)を満たすために必要な最小限の追加であり、UI機能自体の追加ではない――「既存の`SkippedInsufficientManpower`等へ偽装しない」という設計方針(P26-008-A由来)を、レンダリング可能な既存enumへ拡張する際の必然的な結果として説明する。

## 40. 最終`git status --short`

コミット前は上記§39の13ファイルのみ(意図した変更以外なし、非決定的PNG・ユーザーSaveは含まれない)。

## 41. P26-008-Cへの引き継ぎ

本タスクでP26-008-Aの`recruitment_supply_gate.rs`から抽出した`min_fulfillment_over_division_ids`(共通ヘルパー)、および`update_recruitment_supply_gate`の「bridge_deficit_units=0で無効化して再利用する」パターンは、いずれも公開APIとして既に存在し、P26-008-C(建設優先度)が必要とする形で追加の抽出なしに再利用可能な状態にある。ただし本タスクではP26-008-C自体には一切着手していない。P26-008-C READY。
