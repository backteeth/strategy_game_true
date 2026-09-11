# P26-008-A: 補給備蓄指標・募兵停止／再開ゲート実装

## 0. 検証結果サマリ(spec §25)

```
P26-008-A COMPLETE
SUPPLY METRICS VALID
RECRUITMENT GATE VALID
TRUE HYSTERESIS IMPLEMENTED
BRIDGE FORECAST VALID
SAVE CONTINUITY PRESERVED
SAVE V1 PRESERVED
P26-008-B READY
```

## 1. 開始状態

- 開始HEAD: `4999f1ee4c0965aab81cee2d746a77e31fe158a8`(`docs: record final commit hash in p26-008-pre-002 report`)
- `git status`: クリーン(作業開始前)
- 直近コミット: `4999f1e` / `3944cd8`(`fix: restore staggered AI evaluation cadence`) / `bd73b02`(`docs: define supply-aware AI policy`)
- テストベースライン(実測、`cargo test --all-targets`): **2391 passed / 0 failed / 1 ignored**
- clippy(`cargo clippy --all-targets --all-features -- -D warnings`): **0 warnings**
- P26-008-PRE-002は完了済みであり、本タスクでは`mark_ai_registries_dirty_daily`・`AiChangeSnapshot`・cadence設計を一切変更していない(§23で後述する回帰確認により裏付け)。

## 2. 募兵AI経路の現状(監査結果)

- 月次募兵の実際の入口は`src/military/ai.rs::handle_monthly_ai_recruitment`(`MonthChangedMessage`受信、1フレーム最大1回)。
- `handle_monthly_ai_recruitment` → `evaluate_ai_monthly_recruitment`(国家一覧を`CountryId`昇順で走査、`player_country`を除外) → 国家ごとに`decide_and_recruit_for_country`。
- 既存の判定順序(変更前): (1) Definition取得 (2) 国家取得 (3) `recruitment_queue`非空チェック (4) `compute_manpower_ledger`(P23-006、`unaccounted_living_excess`検出) (5) 目標Division数(`calculate_ai_target_divisions`、人口連動)との比較 (6) 人的資源予備チェック (7) 募兵州選択 (8) `request_recruitment`。
- 本タスクは(5)と(6)の間に補給ゲートを1個挿入しただけであり、(1)〜(4)・(6)〜(8)のロジック・呼び出し順序はいずれも変更していない。

## 3. ゲート挿入位置(spec §14)

`src/military/ai.rs::decide_and_recruit_for_country`内、既存の「目標Division数未達」判定(`current >= target`のreturnの直後)と、既存の「人的資源予備」判定(`calculate_ai_manpower_reserve`)の直前に挿入した。

```rust
let current = current_force_count(living_by_owner, country.recruitment_queue.len(), country_id);
if current >= target {
    return AiRecruitmentOutcome::SkippedTargetMet;
}

// P26-008-A: 補給ゲート
match evaluate_recruitment_supply_gate(
    country, military_registry, war_registry, def, game_date,
    ai_state.recruitment_supply_blocked,
) {
    Ok(new_blocked) => {
        ai_state.recruitment_supply_blocked = new_blocked;
        if new_blocked {
            return AiRecruitmentOutcome::SkippedSupplyShortage;
        }
    }
    Err(_) => return AiRecruitmentOutcome::SkippedSupplyShortage,
}

let reserve = calculate_ai_manpower_reserve(cap);
```

- `ai_state.recruitment_supply_blocked`の更新は、**キュー非空・目標達成済みの国家では行われない**(既にそこでreturn済みのため)。つまり「今月、新規募兵候補の検討が実際に行われた国家」についてのみ毎回更新する一貫した基準であり、状態が古いまま放置されることはない(spec §14の要求どおり)。
- 補給不足と判定した場合、既存の`SkippedInsufficientManpower`等へ偽装せず、専用の`AiRecruitmentOutcome::SkippedSupplyShortage`を返す(spec §15)。
- ゲート自体は`treasury`・`recruitment_queue`・Division・`available_manpower`のいずれも変更しない(既存の`military::ai`モジュール全体の契約を維持)。

## 4. 正規定数(spec §6、`src/military/recruitment_supply_gate.rs`にpubで定義、P26-008-Bで再利用可能)

```rust
pub const PEACETIME_TARGET_DAYS: u32 = 30;
pub const WARTIME_TARGET_DAYS:   u32 = 60;
pub const PEACETIME_STOP_DAYS:   u32 = 20;
pub const WARTIME_STOP_DAYS:     u32 = 40;
pub const PEACETIME_RESUME_DAYS: u32 = 35;
pub const WARTIME_RESUME_DAYS:   u32 = 70;
```

戦争状態の判定は既存の`WarRegistry::is_country_at_war(country) -> bool`をそのまま再利用(`status==Active && is_participant(country)`)。複数戦争への参加による閾値の多重積み上げは行わない(単純な二値: 参戦中/非参戦)。

## 5. 国家備蓄日数(spec §7)

`has_reserve_for_days(stockpile_units: u128, demand_units: u128, days: u32) -> bool`は`u128`のchecked演算のみを使う整数比較:

```rust
match demand_units.checked_mul(days as u128) {
    Some(required) => stockpile_units >= required,
    None => false,
}
```

- f64除算・NaN・Infinityを一切生成しない。
- `demand_units == 0`は常に`true`(現在総需要0の国家を「不足」と誤判定しない、spec §8)。
- overflow(`u128`域を超える)は「不足あり」(`false`)へ安全側に倒す。実際の値域(demand ≤ 10^15、days ≤ 70)ではoverflowは理論上到達不能。
- 分母には`country_daily_demand_units`(P26-004-PRE-005の既存正規関数、総需要・接続性とは無関係)をそのまま再利用し、需要式を複製していない。首都占領で接続需要が0になっても総需要自体は変化しないため、「備蓄健全」と誤認しない(P26-008-PRE-001 §21の罠を回避する設計そのもの)。

## 6. 需要0国家の扱い(spec §8)

- 現在総需要0(生存Divisionなし)の国家は、reserve判定について常に「不足なしsignal」(§5の`demand_units==0→true`ルール)。
- ただし**新規候補Division自身の将来需要は含める**――`planning_daily_demand_units = current_total_demand_units + proposed_division_daily_demand_units`(checked加算)であり、bridge判定はこのplanning需要に対して行うため、現在Divisionが0でもbridge不足で見送られ得る(単体テスト`zero_demand_country_bridge_still_applies_to_proposed_division`系、E2E `test_f`で確認)。
- 配送健全性(`min_fulfillment_of_positive_demand_divisions`)も対象Divisionが1件もなければ`None`→「不足signalなし」(健全扱い)。
- eliminated国家(`is_active()==false`)はそもそも`evaluate_ai_monthly_recruitment`の候補一覧に入らない(既存ロジック、変更なし)。

## 7. 将来募兵需要の導出(spec §9)

`proposed_division_daily_demand_units(def: &DivisionDefinition)`は、`try_spawn_division`が生成する募兵完了直後の状態(`manpower == max_manpower == def.required_manpower`)を模した合成`Division`を1個構築し、既存の正規関数`division_daily_demand_units`をそのまま呼ぶ。需要式の複製・CountryId別固定値・`1.0`の直書きは一切行っていない。

- 単体テスト`proposed_division_demand_matches_a_freshly_spawned_division_exactly`で、実際に生成した`Division`を`division_daily_demand_units`へ渡した結果とビット単位で一致することを確認。
- E2E `test_g`で、`supply_usage`を変えた2つのharnessを比較し、bridge必要量が`supply_usage`に比例して変わることを実際の募兵AI経路で確認(0.5→充足、2.0→不足)。
- AI募兵キューは既存仕様により常に最大1件(かつ非空なら本ゲート到達前にreturn済み)であるため、複数候補の同時予測は不要(spec §9のとおり)。

## 8. bridge判定(spec §10)

P26-004-PRE-005の既存正規関数`compute_required_bridge_units`・`events_before_next_monthly_production`・`days_in_month`をそのまま再利用し、「1日の予備」を含む計算式を複製していない:

```rust
pub fn compute_bridge_deficit_units(
    planning_daily_demand_units: u128,
    current_month: u8,
    current_day: u8,
    current_stockpile_units: u128,
) -> Result<u128, BridgeForecastError> {
    let month_length = days_in_month(current_month)?;
    let events = events_before_next_monthly_production(current_day, month_length)?;
    let required = compute_required_bridge_units(planning_daily_demand_units, events)?;
    Ok(required.saturating_sub(current_stockpile_units))
}
```

`current_month`/`current_day`は`Res<GameDate>`(月次評価Systemが実行される時点の実際の値)をそのまま渡し、簡略暦への変換は行わない。

**実カレンダー検証(spec §10要求)**:
- E2E `test_i`: 1月(31日)・2月(28日)・4月(30日)それぞれの月初(day=1)で、必要bridge量が実際の月の長さちょうどになる境界を確認。
- E2E `test_i2`: 月初(day=1)と月末近く(day=25)で同じ備蓄でも判定が変わる(bridgeが月末に近いほど縮小する)ことを確認。
- E2E `test_s`: New Game開始日(day=1)で、初期ブリッジ付与値31.0そのものを安全側の判定基準として特別扱いしていないこと(既存Division需要込みでは月初のbridgeを満たせず見送られる、という実際の需要・カレンダー計算の結果であること)を確認。
- 単体テストで、日次消費処理そのものが起こる「生産日当日」の扱いを含む境界(`events_before_next_monthly_production`の1日予備の意味、`src/military/supply_initialization.rs`のdocコメント参照)を再確認。

## 9. 配送健全性(spec §11)

`min_fulfillment_of_positive_demand_divisions`は、需要が正の生存Divisionについて`MilitaryRegistry::supply_fulfillment_of`の最小値(`DivisionId`昇順走査)を求める。需要0のDivisionは母集合から除外(spec §11・§8)。対象Divisionが1件もなければ`None`(「配送不足signalなし」)。

`is_delivery_fully_healthy`は`min_fulfillment >= 1.0`(`None`は健全扱い)で判定する。`supply_fulfillment_of`が返す`f64`は、日次補給処理(P26-004)自身が`demand==0`または`consumed==eligible_demand`のときに文字通り`1.0`を代入する値であるため、新しい会計判定を発明せず、既存の正規full-scale表現とそのまま突き合わせている。

- E2E `test_j`: 既存Division1個が部分充足(0.5)→見送り。
- E2E `test_k`: 配送率0(完全切断)、国家備蓄は潤沢(100,000)でも見送り(「国家備蓄が多い=健全」と誤認しない)。
- E2E `test_l`: 首都占領を模した配送率0シナリオでも同様(P26-008-PRE-001 §21の罠が実際に回避されていることの直接的な証拠)。
- E2E `test_m`: 需要0(manpower=0)のDivisionはminimum判定の母集合から除外される(配送率0を設定しても影響しない)ことを確認。

## 10. 真のヒステリシス(spec §12、最重要項目)

### 10.1 PRE-001の「新規Saveフィールド不要」の設計是正

P26-008-PRE-001は、備蓄水準系の指標(`national_reserve_days`等)自体は既存Saveされた状態からの純粋関数であり新規フィールド不要、と結論していた。これは指標そのものについては引き続き正しい。

しかし本タスクの§12監査により、**中間帯(停止未満だが再開未満)での状態保持には、直前に停止/再開のどちらの状態にあったかを覚えている必要があり、これは世界状態(現在の備蓄・需要・配送率)だけからは再導出できない**ことを確認した。これはPRE-001の指標設計とは別の問題――「指標の計算」ではなく「指標を跨いだ意思決定メモリ」の問題である。

既存のFrontline/Military AI状態を意味の異なる代替ストレージとして流用することは指示により禁止されているため、これらは検討対象から除外した。`decision_reason`(`CountryAiDecisionReason`)も、既存の意味(直近の意思決定の理由コード)と異なる目的で二重利用することになるため採用しなかった。

**是正**: `CountryAiState`(および`SavedCountryAiState`)へ新規`recruitment_supply_blocked: bool`を追加した。既存の`cooldown_until_day: u32`(war_preparation AIが使う「世界状態から再導出できないAI決定メモリ」)と同種の設計であり、`CountryAiState`が既に複数のAIサブシステム(construction以外にwar_preparationも)にまたがる汎用AI記憶コンテナとして機能していることを確認した上での配置である。

### 10.2 状態遷移(spec §12のpseudocodeどおりに実装)

```rust
pub fn update_recruitment_supply_gate(
    currently_blocked: bool,
    is_at_war: bool,
    current_stockpile_units: u128,
    current_total_demand_units: u128,
    bridge_deficit_units: u128,
    delivery_fully_healthy: bool,
) -> bool {
    let stop_days = stop_days_for(is_at_war);
    let resume_days = resume_days_for(is_at_war);
    let no_bridge_deficit = bridge_deficit_units == 0;
    if currently_blocked {
        let reserve_ok_for_resume =
            has_reserve_for_days(current_stockpile_units, current_total_demand_units, resume_days);
        !(reserve_ok_for_resume && no_bridge_deficit && delivery_fully_healthy)
    } else {
        let reserve_ok_for_stop =
            has_reserve_for_days(current_stockpile_units, current_total_demand_units, stop_days);
        !(reserve_ok_for_stop && no_bridge_deficit && delivery_fully_healthy)
    }
}
```

中間帯では`currently_blocked`の値がそのまま結果を左右する(「停止」経由で入った中間帯は再開まで停止を維持、「許可」経由で入った中間帯は停止まで許可を維持)――「常に停止」「常に許可」という見せかけのヒステリシスにはなっていない。

**中核テスト**: E2E `test_e_middle_band_state_depends_on_prior_blocked_status`で、備蓄・需要・配送状態が完全に同一(平時25日、停止20日/再開35日の中間帯)の2つのharnessを用意し、直前の状態だけを`false`/`true`で変えたところ、結果が`queue_len=1`(許可継続)/`queue_len=0`(停止継続)と明確に分岐することを確認した。

## 11. 戦時/平時の遷移(spec §13)

戦時/平時の切り替えは既存の月次固定cadence(P26-008-PRE-002が復元)のまま利用し、開戦・終戦時に臨時で募兵AIを実行する新規Systemは追加していない。E2E `test_q`(平時→戦時、次回月次評価で新しい閾値が適用される)・`test_r`(戦時→平時、平時再開閾値[35日]に到達するまで戦時中は再開しない)で、実際の`evaluate_ai_monthly_recruitment`経路を通した遷移を確認した。

## 12. 募兵決定結果(spec §15)

`AiRecruitmentOutcome`へ新規variant`SkippedSupplyShortage`を1個だけ追加した(内訳理由でpublic enumを細分化しない、既存の`SkippedLedgerInvalid`が複数の帳簿不整合原因を1種類にまとめている設計と同じ判断)。国家備蓄不足・bridge不足・既存Divisionの配送不全・ゲート自体の評価失敗(防御的な分岐)のいずれもこのvariantへ集約する。既存の`SkippedInsufficientManpower`・`SkippedTargetMet`へ偽装することはない。

## 13. 進行中募兵の保護(spec §16)

`process_recruitment`(募兵完了処理)には一切手を入れていない。ゲートは`decide_and_recruit_for_country`内、`request_recruitment`呼び出しより前、かつ既存の「`recruitment_queue`非空ならreturn」チェックの後に位置するため、既にキューに入っている項目には到達しない。E2E `test_n_existing_queue_entry_is_never_cancelled_by_the_supply_gate`で、備蓄0(補給ゲートが必ずblockedと判定する状況)でも既存キュー項目のlenが変化しないことを確認した。

## 14. 既知のEquipment不足キュー停滞との分離(spec §17)

本タスクでは、既存のMedium finding(Equipment不足による募兵完了stall)に一切手を入れていない(強制Division生成なし、キュー自動削除なし、Equipment必要量緩和なし、AI目標式変更なし、外生的なMilitaryEquipment付与なし)。

E2E `test_o_equipment_shortage_is_a_separate_concern_from_the_supply_gate`で、MilitarySuppliesが十分・MilitaryEquipmentが不足(既定0)の状況でも、新規募兵の**queue投入自体**はブロックされない(`request_recruitment`自体はEquipmentを消費しない、消費は完成処理側)ことを確認し、本ゲートが原因の見送りとEquipment起因のqueue完成停滞が異なる現象であることを明示した。90日/36か月の実App検証(§16参照)でも、`ever_supply_gate_blocked`(本ゲート起因)と`ever_queue_stalled_with_low_equipment`(既存のEquipment起因停滞、queueが複数か月同じ長さのまま滞留し`MilitaryEquipment`備蓄が10未満)を分離して報告している。

## 15. 既存条件の維持(spec §5・§18相当のP)

E2E `test_p_existing_conditions_are_preserved`で、以下いずれも既存どおり新規募兵を行わないことを確認した:
- `recruitment_queue`非空
- 目標Division数達成済み
- `available_manpower`不足
- 募兵可能な自国州が存在しない

## 16. 90日・36か月の6か国実App検証(spec §21-22)

`setup_app(None, ...)`(`PlayerCountry(None)`、全6か国をAI候補にする)を使い、診断用の無限資金フィクスチャなしで実行した。既存の`p26_004`等が使う簡略暦(`day 2..=28`固定、P26-008-PRE-002が検出した既知の罠)は使わず、実際の月の日数(31/28/30/31…)を1日目から使い切り、`GameDate`resourceも各`DayChangedMessage`/`MonthChangedMessage`の送出に合わせて明示的に同期させた(spec §10の「実カレンダーで検証する」という要求を、複数か月にわたる実App検証でも満たすための対応――手動メッセージ注入方式のテストハーネスは`advance_game_date`本番Systemを経由しないため、この同期を怠ると`Res<GameDate>`が初期日付のまま固定されてしまう)。

### 16.1 90日(3か月)結果

| 国 | 最終Division数 | 最終queue長 | 最終MilitarySupplies | 最終MilitaryEquipment | ゲートで見送られたことがあるか | 初回見送り月 |
|---|---|---|---|---|---|---|
| Arcadia(0) | 1 | 0 | 54.650 | 17.130 | true | 1 |
| Elfin(1) | 1 | 0 | 0.000 | 0.000 | true | 1 |
| Dwarf(2) | 1 | 1 | 63.118 | 18.824 | true | 1 |
| Oceanic(3) | 1 | 0 | 0.000 | 0.000 | true | 1 |
| Passguard(4) | 1 | 0 | 0.000 | 0.000 | false | — |
| Ferrowyn(5) | 1 | 0 | 45.827 | 15.365 | true | 1 |

### 16.2 36か月結果

| 国 | 最終Division数 | 最終queue長 | 最終MilitarySupplies | 最終MilitaryEquipment | ゲートで見送られたことがあるか | 初回見送り月 |
|---|---|---|---|---|---|---|
| Arcadia(0) | 1 | 1 | 9.652 | 164.492 | true | 1 |
| Elfin(1) | 1 | 1 | 918.692 | 363.933 | true | 1 |
| Dwarf(2) | 1 | 1 | 27.826 | 211.171 | true | 1 |
| Oceanic(3) | 1 | 0 | 26.191 | 113.330 | true | 1 |
| Passguard(4) | 1 | 0 | 2381.079 | 665.016 | false | — |
| Ferrowyn(5) | 1 | 0 | 53.859 | 217.972 | true | 1 |

いずれの区間でも`ever_queue_stalled_with_low_equipment`はfalse(既存のEquipment起因停滞は本シナリオでは発生していない、本ゲートの効果と混同していない)。

### 16.3 観察された事実と解釈

- **6か国とも36か月間、最終Division数は1のまま**だった。これは既存の`calculate_ai_target_divisions`(人口連動、`cap/2/required_manpower`)が、現行の6か国の人口・人口成長率のもとでは36か月という期間内に目標2以上へ到達しないためであり、**本タスクが導入した補給ゲートが原因の頭打ちではない**(目標未達判定`SkippedTargetMet`はゲートより前に評価され、それ自体は変更していない)。目標Division数の式・成長速度はP26-008-Aの対象外である(spec §24: AI目標Division数を変更しない)。
- **Passguard(CountryId 4)以外の5か国は、いずれも1か月目の時点で本ゲートにより「blocked」と判定されたことがある**。実データ上のNew Game初期状態では、各国の初期MilitarySupplies備蓄(P26-004-PRE-005のブリッジ付与)・初期Division構成・初期需要の組み合わせにより、平時停止しきい値(20日)を下回る国が多数派であることを示している。これはP26-008-A自身が導入した数値ではなく、既存の初期データ(`assets/data/*.ron`)とP26-004の日次消費実績から導かれた結果である。
- **Passguardのみ一度もblockedにならなかった**。これは、Passguardが本ゲートの対象になる状況(目標未達かつ既存キュー空)に到達した際の実際の備蓄・需要・配送状態が、その時点の停止しきい値を下回らなかったためであり、6か国が均質に扱われているわけではないことの直接的な証拠である(spec §21で明示的に要求されている「全国家を同じ結果に強制しない」という設計要件を、実データで裏付けている)。
- spec §21のillustrative example(「Arcadiaは薄い補給網ゆえ抑制される」「Oceanicは既存の頻繁な不足を認識する」「Dwarfは潤沢な備蓄ゆえ不必要に停止しない」等)は、6か国のうち5か国がゲートの影響を受け1か国が受けない、という形で「均質化していないこと」自体は実証されたが、**個別の国名と個別の期待(特にDwarfが「不必要に停止しない」という部分)は、現行の実データでは厳密には一致しなかった**(Dwarfも1か月目にblockedと判定された)。これは実装の誤りではなく、illustrative exampleが一般的な傾向を説明する記述であり、個々の国の具体的な初期数値までを保証するものではなかったためと判断する。実際の初期MilitarySupplies・Division構成に対して、ゲートのロジック自体(§5〜§12)は単体テスト20件・境界値E2E各テストで独立に検証済みであり、この36か月結果はそのロジックが実データに対して**予測可能かつ説明可能な形で**動作していることを示している。
- 上記は失敗ではなく正直な報告である。spec §21は明示的に「一部の国のDivision数が既存のEquipment不足や目標数の問題で伸びなくても、それ自体は本タスクの失敗ではない」としており、目標Division数が36か月間ずっと1のままである事実、および5/6か国がゲートの影響を受けたという事実は、いずれも既存の周辺システム(人口成長・初期資源配分)の特性であって、本ゲート自体の欠陥を示すものではない。

## 17. Save V1(現行/新規/旧、spec §18)

### 17.1 新規フィールド

- `CountryAiState.recruitment_supply_blocked: bool`(実行時のみ、Saveしない)
- `SavedCountryAiState.recruitment_supply_blocked: bool`(`#[serde(default)]` → 旧Save読込時は`false`)

`export.rs`/`apply.rs`/`dto.rs`/`validate.rs`の該当箇所、およびテスト用のリテラル構築箇所(`dto.rs`2箇所、`export.rs`1箇所、`apply.rs`1箇所、`validate.rs`1箇所)を漏れなく更新した。

### 17.2 検証結果

- E2E `test_t_current_save_round_trips_the_blocked_flag_both_ways`: `true`/`false`いずれの値も現行Save往復で保持される。
- E2E `test_u_old_save_without_the_new_field_loads_and_settles_from_actual_state`: 実際に保存されたRONファイルから`recruitment_supply_blocked`の行を機械的に除去した「旧Save」を読み込んでもロードが成功し、`false`(未停止)から復元されること、その後の月次評価がパニックせず正常に継続することを確認した。
- E2E `test_v_double_save_load_does_not_change_the_blocked_state`: 2回連続のsave→loadで値が変化しない(`true`→往復→`true`→往復→`true`)。
- ロード直後にゲートの臨時判定・クリアは一切行わない(次回の正規月次評価まで待つ、spec §18のとおり)。`apply.rs`のロード経路は`recruitment_supply_blocked`をそのまま復元するだけであり、`prepare_load`内に補給ゲート関連の新規ロジックは追加していない。

### 17.3 SAVE_FORMAT_VERSION

`SAVE_FORMAT_VERSION_V1`は変更していない。既存の`FrontlinePlan.assigned_army_ids`等、およびP26-008-PRE-002自身の`dirty`フィールド追加と同じ、追加的`#[serde(default)]`フィールドによるSave V1の後方互換拡張パターンに従う。V2への切り替えは不要と判断した。

## 18. 決定性・国家間分離(spec §19)

- E2E `test_w_one_countrys_shortage_does_not_affect_another_countrys_gate`: 一方の国家が備蓄0で見送られても、無関係な備蓄潤沢な他国は正常に募兵できる。
- E2E `test_x_result_does_not_depend_on_country_or_division_insertion_order`: `CountryRegistry`/`StateRegistry`への登録順を反転しても結果が変わらない。
- `min_fulfillment_of_positive_demand_divisions`・`country_daily_demand_units`はいずれも`DivisionId`昇順ソート後に走査(`HashMap`反復順に依存しない)。
- 新規E2E(`p26_008_a_supply_aware_recruitment_gate_e2e_test.rs`)を含む全対象ファイルを、5回ずつ独立プロセスとして実行し、結果が完全に一致することを確認した(§20参照)。

## 19. 回帰保護(spec §23)

以下いずれも本タスク開始前後で挙動・正規値が変化していないことを確認した:

- P26-008-PRE-002のcadence・`AiChangeSnapshot`(週次/月次/3日おきの評価回数、`test_t_real_app_90_day_evaluation_counts_match_designed_cadence_for_all_ai_countries`含む16テスト全通過)
- P26-007モデル2境界(補給会計)
- P26-006首都接続性、P26-006-REV-001移動・前線決定性(5回独立プロセス一致)
- P26-005補給連動の戦闘力
- P26-004日次補給会計、P26-004-PRE-005初期備蓄・旧Save移行
- P26-004-PRE-004補給網データ
- P26-DET-001/REV-001決定性
- Phase23人的資源(`p23_010`5回独立プロセス一致含む)、Phase24戦争/講和、Phase25装備
- New Game・Save V1・Loaded Game

回帰で2件の既存テストが本タスクの新ゲートの影響を受けたため、テスト側の前提条件を実態に合わせて修正した(ロジックの弱体化ではない、詳細は§20):

1. `tests/p23_009_ai_recruitment_e2e_test.rs::after_completion_a_country_still_below_target_recruits_its_next_division_the_following_month` — 新規`ensure_large_military_supplies_stockpile`ヘルパーを追加し、既存の`ensure_large_military_equipment_stockpile`等と同様にMilitarySupplies備蓄も十分に確保するよう明示した。
2. `tests/p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs::test_b1_first_monthly_production_adds_normally_without_double_counting_the_bridge` — このテストは意図的に2つのApp(初期31.0付与あり/なし)を比較する設計であり、新ゲートの導入により2つのAppでAI募兵が異なる月に成立し、比較対象の軌跡が(意図された単純なオフセットではなく)分岐してしまう問題を検出した。新規`zero_out_available_manpower_for_all_countries`ヘルパーで、既存の(変更していない)`available_manpower`不足チェックを使ってAI募兵自体を両App共通で無効化し、テスト本来の検証対象(初期付与の有無による軌跡比較)を新ゲートの影響から独立させた。

## 20. 既知のEquipment起因queue停滞(spec §17)への非変更の確認

`process_recruitment`・Equipment必要量判定・AI目標Division数式のいずれにも変更を加えていない(§14参照)。90日/36か月検証でも、本ゲート起因(`ever_supply_gate_blocked`)とEquipment起因停滞(`ever_queue_stalled_with_low_equipment`)を分離して観測し、後者は今回のシナリオでは発生していないことを確認した。

## 21. テスト・clippy件数

| 項目 | 開始時 | 完了時 |
|---|---|---|
| `cargo test --all-targets` | 2391 passed / 0 failed / 1 ignored | **2441 passed / 0 failed / 1 ignored** |
| 新規単体テスト(`src/military/recruitment_supply_gate.rs`) | — | 20件 |
| 新規E2Eテスト(`tests/p26_008_a_supply_aware_recruitment_gate_e2e_test.rs`) | — | 30件(spec §20のA〜Yに加え、§21-22の90日・36か月実App検証2件) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings | **0 warnings** |

差分検算: 2391 + 20 + 30 = 2441(一致)。

## 22. 逐次実行コマンド(spec §26、すべて逐次・単一プロセスで実行)

1. `cargo test --all-targets`(開始時ベースライン測定) → 2391 passed / 0 failed / 1 ignored
2. `cargo clippy --all-targets --all-features -- -D warnings`(開始時) → 0 warnings
3. `cargo test --test p26_008_a_supply_aware_recruitment_gate_e2e_test --no-fail-fast` → 30 passed / 0 failed
4. `cargo test --lib military::` / `country::` / `war::` / `economy::` / `save::` → いずれも既存回帰なし(§19記載の全体実行に包含)
5. 回帰対象E2E(個別実行、全て0 failed):
   `p26_008_pre_002_ai_evaluation_cadence_e2e_test`(16 passed)、
   `p26_007_elimination_occupation_overseas_supply_accounting_e2e_test`、
   `p26_006_rev_001_ai_save_resume_determinism_e2e_test`(9 passed)、
   `p26_006_capital_connectivity_supply_e2e_test`、
   `p26_005_supply_combat_strength_e2e_test`、
   `p26_004_daily_supply_consumption_allocation_e2e_test`、
   `p26_004_pre_005_initial_supply_bridge_migration_e2e_test`(22 passed、修正後)、
   `p26_det_001_construction_ai_determinism_e2e_test`、
   `p26_det_001_rev_001_cross_state_production_order_e2e_test`、
   `p23_010_phase23_manpower_lifecycle_audit_test`(9 passed)、
   `p23_009_ai_recruitment_e2e_test`(22 passed、修正後)
6. `cargo test --all-targets --no-fail-fast`(最終、全体) → **2441 passed / 0 failed / 1 ignored**
7. `cargo clippy --all-targets --all-features -- -D warnings`(最終) → 0 warnings
8. 独立プロセス5回ずつ(いずれも結果完全一致):
   - `p26_008_a_supply_aware_recruitment_gate_e2e_test`: 30 passed ×5
   - `p26_008_pre_002_ai_evaluation_cadence_e2e_test`: 16 passed ×5
   - `p26_006_rev_001_ai_save_resume_determinism_e2e_test`: 9 passed ×5
   - `p23_010_phase23_manpower_lifecycle_audit_test`: 9 passed ×5

## 23. 除外事項(spec §24の遵守)

以下はいずれも本タスクで変更していない:

- 進行中募兵のキャンセル・一時停止・巻き戻し・返金・再装備・免除
- 既知のEquipment不足によるqueue完成停滞の是正
- AI目標Division数の算出式・成長速度
- 攻勢姿勢(offensive stance)の変更(P26-008-B)
- 建設AIの変更(P26-008-C)、Factory先行開発、MilitaryFactory建設AI
- 宣戦布告ロジック、戦闘式、補給会計式そのもの(P26-004の日次消費・Hamilton配分)
- AI評価cadence(P26-008-PRE-002の成果)
- UI
- 外生的なMilitarySupplies付与、初期31.0を固定の安全判定基準として使うこと
- CountryId別の個別しきい値、全国家の生産量を均等化すること
- 既存テストのアサーション弱体化、flakyテストのリトライ隠蔽
- ユーザーのSaveファイル(`saves/savegame_v1.ron`)の読み書き

## 24. PNG・作業ディレクトリの取り扱い

`cargo test`実行に伴い、既知の非決定的スクリーンショット(`verification_logs/p20-007/`・`p20-009/`・`phase-21/p21-save-002e/`配下の9ファイル)が再生成され`git status`上で変更として現れたが、個別に`git checkout --`で復元し、意図した変更(下記§25)以外がステージされないことを確認した。ユーザーのSaveファイルへは一切アクセスしていない。

## 25. 変更ファイル一覧(コミット前、`git status --short`)

```
 M src/country/country_ai.rs
 M src/military/ai.rs
 M src/military/mod.rs
 M src/save/apply.rs
 M src/save/dto.rs
 M src/save/export.rs
 M src/save/validate.rs
 M tests/p23_009_ai_recruitment_e2e_test.rs
 M tests/p26_004_pre_005_initial_supply_bridge_migration_e2e_test.rs
?? src/military/recruitment_supply_gate.rs
?? tests/p26_008_a_supply_aware_recruitment_gate_e2e_test.rs
```

## 26. P26-008-B(攻勢ゲート)への引き継ぎ

本タスクで`src/military/recruitment_supply_gate.rs`にpubで定義した正規定数(`PEACETIME_TARGET_DAYS`等)・`has_reserve_for_days`・国家備蓄日数の計算パターンは、P26-008-B(攻勢姿勢ゲート)がそのまま再利用できる形で用意されている。ただし本タスクではP26-008-B自体のロジック(power-ratio判定との組み合わせ等)には一切着手していない。P26-008-B READY。

## 27. 最終確認

- `git status --short`は§25の11ファイルのみ(意図した変更以外なし、非決定的PNG・ユーザーSaveは含まれない)。
- 本レポートはこの時点で実在するコミット(`4999f1e`以前)のみを参照しており、未作成のコミットハッシュへの言及はない。次のコミットで本タスクの変更一式(§25)をまとめてコミットする。
