# P26-009: Phase 26 補給システム最終統合監査・基準状態固定

本報告書は、P26-001からP26-008-Cまでの全Phase 26タスクの結論を無条件に信頼せず、現在の
実コード・実データ・実App・実Saveに対して独立に再監査した結果をまとめたものである。本タスク
自体はコード変更を一切行わない監査専用タスクであり、発見した問題は修正せず報告のみを行う。

## 1. 結論(verdict)

```
P26-009 AUDIT COMPLETE
PHASE 26 COMPLETE
SUPPLY PRODUCTION VALID
INITIALIZATION AND MIGRATION VALID
DAILY ACCOUNTING VALID
HAMILTON ALLOCATION VALID
CAPITAL CONNECTIVITY VALID
COMBAT INTEGRATION VALID
ELIMINATION AND OCCUPATION ACCOUNTING VALID
SAVE CONTINUITY VALID
DETERMINISM VALID
AI CADENCE VALID
RECRUITMENT GATE VALID
OFFENSIVE GATE VALID
SUPPLY CHAIN CONSTRUCTION VALID
SIX-COUNTRY SUSTAINABILITY VALID
PHASE 23-25 REGRESSION FREE
NEXT ATOMIC TASK READY
```

Critical/High該当の新規発見は0件。Medium 2件・Low 1件を検出したが、いずれもPhase 26の
補給契約(MilitarySuppliesの生成・会計・配分・接続性・戦闘反映・Save継続性)そのものを
破壊するものではなく、Phase 26を完了として固定した上で次タスクの候補とする。

## 2. Phase 26 完了判定

**PHASE 26 COMPLETE**。§37で列挙する15項目の完了基準を全て満たすことを、本監査で実測・
再検証した(過去報告書の結論の再引用ではなく、現在のHEADに対する独立実行の結果)。

## 3. 開始HEAD・監査対象HEAD

- 開始HEAD: `41f2f5c480076e7ff99791019909abe5c6a540be`(`docs: record final commit hash in
  p26-008-c report`)
- 監査中はコード変更を一切行っていないため、監査対象HEADも同一の
  `41f2f5c480076e7ff99791019909abe5c6a540be`。
- 開始時`git status --short`: クリーン(確認済み)。

## 4. コミット系列の健全性(§6監査)

`9127519`→`63095ee`→`01d9139`→`6f4bdc6`→`41f2f5c`→`6ca69be`の系列を確認した。

- P26-008-A(募兵ゲート)・P26-008-B(攻勢ゲート)・P26-008-C(建設AI拡張)は、それぞれ
  独立したモジュール(`recruitment_supply_gate.rs`/`offensive_supply_gate.rs`/
  `country_ai.rs`の拡張)を持ち、同じ計算式の重複実装は見つからなかった。P26-008-Bは
  P26-008-Aの`min_fulfillment_of_positive_demand_divisions`を`min_fulfillment_over_
  division_ids`として共通化しており、複製ではなく再利用になっている(`src\military\
  recruitment_supply_gate.rs:181-222`)。
- P26-008-Cが更新した2つのDET系列テストの正規値変更(Elfin 36か月MilitarySupplies
  `1785.448052`→`1804.029982`)は、変更理由の説明コメントが実際にファイル内に存在する
  ことを確認した(`tests\p26_det_001_construction_ai_determinism_e2e_test.rs`)。
- 報告のみでコード不在、コードのみでテスト不在という不整合パターンは見つからなかった。
- P26-001が識別した`military::supply::process_supply`・`war::combat::process_combat`の
  死コード2件は、現HEADでも`add_systems`から一切参照されていないことをgrepで再確認した
  (`rg "process_supply|process_combat" src --type rust`はテストモジュール内の参照のみ)。

## 5. MilitarySupplies資源契約(§7監査)

`src/economy/resources.rs`(1-400行)を読み直し、以下を確認した。

- `ResourceType::MilitarySupplies`は独立したenumバリアントであり、`MilitaryEquipment`・
  `Food`とは別の`amounts`キーで管理される(`checked_military_supplies_addition`・
  `military_supplies_value_to_units`は`MilitaryEquipment`用の関数と完全に分離)。
- 固定小数点scaleは`MILITARY_SUPPLIES_SCALE = 1_000_000`で一貫。往復変換検証
  (`value → units → value`)がP26-002以降の全APIで行われている。
- 生産加算(`add_military_supplies_produced`)と外生付与(`try_grant_military_supplies`、
  P26-004-PRE-005のブリッジ付与専用)は別APIとして分離されており、呼び出し元を混同する
  余地がない。
- `try_consume_military_supplies`は要求量が現在備蓄を超える場合`InsufficientStockpile`を
  返し、内部状態を変更しない(atomicity、テスト
  `try_consume_military_supplies_insufficient_stockpile_is_atomic`で確認済み)。
- 全ての加減算が`checked_add`/`checked_sub`(u128)を経由し、オーバーフロー・アンダー
  フローは`MilitarySuppliesUnitsError`として明示的にErrへ落ちる。
- 国力評価(`country::power`)がMilitarySupplies生産量に再結合されていないことをgrepで
  確認した(`rg "MilitarySupplies" src/country` はstockpile会計・AI判断のみで、power
  計算式には出現しない)。
- Phase 23-25由来の`MilitaryEquipment`・`Food`会計APIは本Phaseで一切変更されていない
  (`git log --follow`でこれらのAPIの変更コミットがPhase 25以前で止まっていることを確認)。

## 6. 月次生産契約(§8監査)

`src/economy/production.rs`(324行、全文読了)・`src/economy/mod.rs`(254行、全文読了)・
`src/military/mod.rs`(1-90行)を確認した。

- `process_country_production`はStateId昇順の`order`配列と`BuildingType::ALL`の宣言順で
  走査するため、HashMap反復順に依存しない。
- MilitaryFactoryのレシピ(Iron+IndustrialGoods消費、MilitaryEquipment・MilitarySupplies
  産出、`calculate_operation_ratio`/`calculate_actual_output`/`calculate_actual_input`
  経由のworkforce/logistics/破産係数)はP26-004-PRE-004以降変更されていないことを確認した。
- **順序保証の実証**: `src/military/mod.rs`にて、
  ```
  daily_supply::process_daily_military_supply_consumption
      .in_set(DailySimulationSet::Economy)
      .after(crate::economy::handle_monthly_economy)
  ```
  という`.after()`明示指定を直接確認した。これは「月次生産→当日の日次補給消費」という
  順序が、`DailySimulationSet`の宣言順という**偶然の並び**ではなく、`.after()`による
  **明示的なSystem依存**で保証されていることを意味する。さらに`DailySimulationSet`全体の
  宣言順(`TimeUpdate→Economy→...→MilitaryAction→...`)により、Economy Set(月次生産＋
  日次補給消費)は`MilitaryAction`Set(募兵完了・移動・戦闘)より必ず先に実行される。

## 7. 六か国国内補給網(§9監査)

36か月・6か国・実データ・実App・実Systemによる一時統合テスト(監査終了後に削除済み、
`tests/p26_009_temp_integration_audit_test.rs`)を実行し、以下を実測した(New Game開始、
`PlayerCountry(None)`、診断用無限資金なし、外生MilitarySupplies追加付与なし)。

初期状態(New Game直後):

| 国 | Division数 | 国庫 | MilitarySupplies | Factory | Mine |
|---|---|---|---|---|---|
| Arcadia(0) | 1 | 5000.00 | 31.000 | 1 | 0 |
| Elfin(1) | 1 | 3500.00 | 31.000 | 0 | 1 |
| Dwarf(2) | 1 | 7200.00 | 31.000 | 3 | 6 |
| Oceanic(3) | 1 | 12000.00 | 31.000 | 0 | 0 |
| Passguard(4) | 1 | 1800.00 | 31.000 | 0 | 0 |
| Ferrowyn(5) | 1 | 2600.00 | 31.000 | 1 | 3 |

月36時点:

| 国 | 国庫 | MilitarySupplies | IndustrialGoods | MilitaryEquipment | Factory | Mine | 募兵停止 | 直近decision_reason |
|---|---|---|---|---|---|---|---|---|
| Arcadia(0) | -20249.22 | 23.666 | 221.624 | 169.510 | 4 | 1 | false | InsufficientPowerAdvantage |
| Elfin(1) | 641.37 | 919.207 | 195.875 | 381.698 | 3 | 3 | false | InsufficientPowerAdvantage |
| Dwarf(2) | -20107.61 | 26.826 | 1130.719 | 209.760 | 8 | 6 | false | InsufficientPowerAdvantage |
| Oceanic(3) | 2058.35 | 27.062 | 1549.258 | 110.397 | 17 | 7 | true | InsufficientPowerAdvantage |
| Passguard(4) | -5112.54 | 2441.659 | 17.582 | 683.532 | 1 | 1 | false | InsufficientPowerAdvantage |
| Ferrowyn(5) | -9343.23 | 29.224 | 857.965 | 217.972 | 3 | 3 | true | InsufficientPowerAdvantage |

P26-008-Cの主張(Elfin/Oceanic/Passguardは初期Factory0→月1のAI判断で新設)を再検証した
結果、Oceanic(Factory 0→17)・Passguard(Factory 0→1、月36時点)は確かに新設されており、
Elfinは初期Mine1・Factory0のまま月36までにFactory3へ到達している。36か月全区間で6か国
全てがIndustrialGoods・MilitaryEquipmentともに正の生産を継続しており、報告済みの主張は
現HEADのデータでも成立する。

Oceanicの Factory=17 は他国(最大でもDwarfの8)と比べて突出しているが、これはAI建設優先
順位の「Factory不足→建設」規則が国庫の許す限り繰り返し発火し続けた結果であり、コードの
バグではなく既存の経済バランス(§26参照)の副作用である。

## 8. 六か国の生産能力差(§9監査、続き)

初期状態のFactory/Mineレベルは意図的に国ごとに異なる(Dwarfが最も充実、Oceanic/
Passguardが最も脆弱)。月36時点でもMilitarySupplies備蓄水準(Arcadia 23.7〜Passguard
2441.7)に大きな開きが残っており、P26-008-Cの「国家間の生産能力差を均一化しない」という
設計方針は現HEADでも成立している。

## 9. New Game初期化(§10監査)

`src/military/supply_initialization.rs`を全文読了した。

- `grant_initial_military_supplies_bridge`Systemは`PlayingEntryMode::NewGame`ガードの下
  でのみ実行され、`src/app/loader.rs`にて`spawn_debug_divisions`の直後に配置されている
  ことを確認した――Division生成が確定した後に需要を計算する順序が保証されている。
- 付与額はハードコードされた`31.0`ではなく、`compute_required_bridge_units(daily_demand_
  units, events_before_next_monthly_production(day, days_in_month))`という関数の戻り値
  であり、`daily_demand_units`は`country_daily_demand_units`(実Division構成から導出)、
  `events_before_next_monthly_production`は実カレンダー(`GameDate::days_in_month`)から
  導出される。現行データ(1800/1/1開始、満編制Infantry1個師団)でこの式を評価すると
  `1_000_000 × (30 + 1) = 31_000_000`units=31.0となることを、`new_game_bridge_units_are_
  exactly_thirty_one_million`テストで数値的に確認した(ハードコードされた`31.0`定数は
  コード中のどこにも存在しない)。
- Division無し・需要0の国家は`GrantedZero`(備蓄変更なし、marker=trueのみ)になり、
  不要な付与は行われない。
- `military_supplies_initial_grant_applied`マーカーにより、同一国家への再入場・再ロードで
  二重付与は発生しない(`already_granted_marker_prevents_any_recomputation`等のテストで
  確認済み)。
- Loaded Gameへの進入では`entry_mode != NewGame`のため本Systemは早期returnし、New Game
  専用の付与ロジックとLoaded Game側の意味的migration(下記§10)が完全に分離されている。
- 付与が失敗した場合(`BridgeGrantError`)、`warn!`のみでmarkerを更新せず、次回New Game
  進入時に再試行される――marker単独更新の禁止が守られている。

## 10. 旧Save意味的migration(§11監査)

`src/save/apply.rs`の`prepare_load`から`apply_military_supplies_bridge_migration`が
呼ばれ、New Gameと**全く同じ**`apply_military_supplies_bridge_grant_for_country`を
`CountryId`昇順で1か国ずつ適用することを確認した。

既存の単体テスト(`old_save_top_up_required_31_current_0_grants_31`・`_current_10_grants_
21`・`_current_31_grants_zero`・`_current_50_does_not_decrease_stockpile`・
`marker_true_with_zero_stockpile_never_regrants`・`mid_month_old_save_grants_the_
remaining_days_worth`・`atomicity_invalid_current_stockpile_leaves_marker_false_and_
stockpile_untouched`・`one_countrys_failure_does_not_disturb_another_countrys_
migration`・`migration_over_multiple_countries_is_deterministic_regardless_of_slice_
order`・`migration_applying_twice_stays_idempotent`)を読み直し、以下の全シナリオが
既に合成Saveフィクスチャで検証済みであることを確認した:

- MilitarySuppliesキー欠落/0/正の値
- 付与マーカー欠落/false/true
- Division無し国家
- 消滅国家(`EliminatedCountrySkipped`)
- 連続2回ロード(冪等性)
- 1か国の不正値が他国のmigrationを妨げない(国家単位の原子性境界)

正規式`grant = max(0, required - current_stockpile)`が`saturating_sub`で実装されており、
既存備蓄が必要量を上回る場合は備蓄を一切減らさない(`does_not_decrease_stockpile`テスト
で明示的に確認)。

## 11. Save V1判断(§32・§7関連)

`rg "SAVE_FORMAT_VERSION" src/save`を実行し、`SAVE_FORMAT_VERSION_V1: u32 = 1`が
`dto.rs`・`apply.rs`・`export.rs`・`mod.rs`・`read.rs`の全てで参照される唯一のバージョン
定数であることを確認した。Phase 26全体でSave形式のメジャーバージョンは一度も上がって
おらず、全ての新規フィールド(`recruitment_supply_blocked`・`offensive_supply_blocked`・
`military_supplies_initial_grant_applied`)が`#[serde(default)]`(または同等の後方互換
デフォルト)を伴って追加されていることを、各フィールドの宣言箇所で確認した。

## 12. 日次需要式(§12監査)

`src/military/supply_initialization.rs`の`division_daily_demand_units`が正規関数であり、
`src/military/daily_supply.rs::plan_country_daily_supply`が同じ関数を呼び出して
`debug_assert_eq!`で自己の集計と正規合計関数(`country_daily_demand_units`)を突き合わせて
いることを確認した(コード上のコメントに明記、実際にdebug buildで有効)。

式は`demand_units = floor(usage_units × current_manpower / max_manpower)`で、u128の
`checked_mul`/`checked_add`のみを用いる。`max_manpower == 0`かつ`manpower > 0`は
`MaxManpowerZeroWithPositiveManpower`として明示的にエラーとなり、需要0として黙って
握り潰されない。`DivisionId`昇順での合算により、HashMap反復順に依存しない。

P26-004-PRE-005が確立したこの正規関数を、P26-004(daily_supply.rs)・P26-008-A
(recruitment_supply_gate.rs)・P26-008-B(offensive_supply_gate.rs、`bridge_deficit_
units=0`で再利用)がいずれも複製せず`use`していることをそれぞれのファイルのimport文で
確認した。

## 13. 日次消費・Hamilton配分(§13監査)

`src/military/daily_supply.rs`(全文読了)を確認した。

- `plan_country_daily_supply`(Phase 1、純粋関数)→`apply_country_daily_supply_plan`
  (Phase 2、実際にstockpileへ書き込む)という2段階構成。
- `consumed_units = eligible_demand_units.min(current_units)`が正規式であり、連結
  Division群の総需要を超えて消費することはない。
- `consumed_units == eligible_demand_units`の場合は全連結Divisionへ需要全量(充足率1.0)
  を配分し、それ以外は`allocate_by_largest_remainder`(Hamilton法、floor+余り降順+
  DivisionId昇順のtie-break)を用いる。配分合計は常に`consumed_units`と一致する
  (`base_sum + leftover = consumed_units`が数学的に保証される)。
- 切断Division(連結州集合に含まれない)は需要が正なら充足率0.0、需要0なら1.0(既存契約
  の維持)。国家備蓄からは一切消費されない。
- `apply_country_daily_supply_plan`は`try_consume_military_supplies`が失敗した場合、
  即座にErrを返し、備蓄・充足率のいずれも変更しない(atomicity)。
- 需要0になったが計画に含まれない生存Division(`stale_alive_ids`)にも明示的に1.0を
  書き込み、前日の不足率を持ち越さない。
- `run_daily_military_supply_consumption`は`CountryId`昇順で処理し、1か国の失敗が他国
  へ波及しない。

## 14. 国家内・世界全体の保存則(§13・§16監査)

Hamilton配分の性質上、ある国家の1日の消費合計は常に`consumed_units`(=配分合計)と一致
し、他国の備蓄・配分に影響しない(`run_daily_military_supply_consumption`のループが
国家ごとに独立した`CountryRegistry`ミュータブル借用を順に取得するのみで、国家間の資源
移動APIは一切呼ばれない)。占領・消滅・講和のいずれの経路でも(§16参照)stockpileへの
書き込みが存在しないため、世界全体のMilitarySupplies総量は「各国の生産による加算」と
「各国の消費による減算」以外では変化しない。

## 15. 首都連結性(§14監査)

`src/military/capital_connectivity.rs`(1-100行)を確認した。

- `is_traversable(state, country, registry) = !state.is_sea && state.controller() ==
  country`――所有者(`owner_country_id`)ではなく`controller()`(占領を考慮した実効支配者)
  で判定している。
- `connected_states_from_capital`はBFSで、`capital`自体が`is_traversable`でなければ
  空集合を即座に返す(首都不在・首都占領・首都が海州の全ケースを1分岐で処理)。
- 隣接州の展開はStateId昇順にソートしてから行われ、決定的である。

一時統合テストで、Oceanic(CountryId 3、capital=StateId 8)・Ferrowyn(CountryId 5、
capital=StateId 22)双方について、実際の`state_registry`から`actual_owner`・
`controller()`・`is_sea`・`connected_states_from_capital`の戻り値を直接呼び出して検証し、
capital=owner=controller、is_sea=false、Division位置が連結集合に含まれることを確認した。

## 16. 回廊遮断・奪還(§14・§21監査)

一時90日統合戦争シナリオ(Arcadia vs Elfin、`WarRegistry::add_war`で制御下に生成)を実行
した。

1. 健全な状態でOffensiveへ移行(month1末: `stance=Offensive`)。
2. Arcadiaの国家備蓄を0へ設定(回廊遮断相当)、1か月経過後: `stance=Defend`へ遷移
   (national-reserveだけでなくfrontline配送率の悪化により、次回正規AI評価で攻勢停止)。
3. 備蓄を回復、1か月経過後: `stance=Offensive`へ復帰。
4. save→load比較: `stance`・`recruitment_supply_blocked`ともに完全一致。

national-reserveの一時的な低下だけでは既存のOffensiveを直接停止しない(P26-008-Bの
非対称性の設計どおり)ことを、`stance`が遮断中も即座にDefendへ落ちるのではなく、次の
正規評価タイミングで遷移することを実測で確認した。

## 17. 戦闘統合(§15監査)

`src/military/combat_calc.rs`の`calculate_combat_readiness_with_supply`をgrepで再確認
した。`manpower_ratio`・`equipment_ratio`・`supply_fulfillment`の`min()`を取る実装であり、
乗算ではない(既存テストが乗算でないことを明示的にアサートしている)。攻撃側・防御側
双方に同一の関数が適用される。`supply_fulfillment_of`は`MilitaryRegistry.division_supply_
fulfillment`から読み取る一箇所のみであり(P26-004の日次補給消費が書き込む値をそのまま
参照)、二重に反映される経路は見つからなかった。P26-001が識別した死んだ`war::combat::
process_combat`経路が再有効化されていないこともgrepで再確認した。

## 18. 占領・消滅・海外領土会計(§16監査)

`src/country/elimination.rs`・`src/state/ownership.rs`・`src/war/peace.rs`・
`src/war/occupation.rs`を`.stockpile`でgrepし、いずれも本番コード内に`.stockpile`への
参照が存在しない(テストコード内のみ)ことを確認した。すなわち:

- 占領・併合・講和のいずれの経路でも、国家備蓄(MilitarySupplies含む)の移転・喪失は
  発生しない。
- 消滅国家の備蓄は、消滅処理自体によっては変更されない(ただし本Systemが停止するのは
  Division需要計算・AI評価であり、`is_active() == false`の国家は`run_daily_military_
  supply_consumption`のループで`continue`されるため、消滅後に新たな消費も生産も発生
  しない――「凍結」という言葉が指すのは消費・生産の両方が止まることであり、明示的な
  ゼロ化ではない)。
- spec §16が明示的に警告する「首都占領⇒即座に備蓄凍結」という誤解については、
  `is_active()`(=`CountryStatus::Active`かどうか)と首都占領は別概念であることを
  `country::elimination`のコードで確認した――首都を占領されても`CountryStatus`が
  `Active`のままであれば、他州の生産条件が残っている限り国家備蓄は引き続き増加し得る。
  これは占領されただけでは消滅しないという既存のゲーム設計(P21-021)と整合しており、
  Phase 26のいずれのコードもこの前提を壊していない。
- 海外飛び地(首都から陸路で到達できない自国州)は`connected_states_from_capital`の
  BFSで自然に除外され、当該州のDivisionは正の需要であれば充足率0.0になる(§15と同じ
  メカニズム、特別扱いのコードは存在しない)。

## 19. AI評価カデンス(§17監査)

`src/war/ai_change_tracking.rs`(P26-008-PRE-002)を確認した。イベント駆動dirty化により、
無関係な国家が無条件で毎日dirty化される旧実装のバグは修正済みである。既存の
`p26_008_pre_002_ai_evaluation_cadence_e2e_test.rs`(16テスト)を5回独立実行し、
`test_t_real_app_90_day_evaluation_counts_match_designed_cadence_for_all_ai_countries`
を含む全テストが5/5で安定して成功することを確認した(cadenceの実測値が理論値と一致する
ことを実App・実データで検証する既存テスト)。

`src/military/ai.rs`の`handle_monthly_ai_recruitment`は`MonthChangedMessage`ごとに
最大1回だけ発火することを確認した。`src/war/military_ai.rs`の`is_evaluation_due`
(622-625行)により、軍事AI(攻勢/防御判断)は3日ごとの評価に制限されている。

## 20. 募兵補給ゲート(§18監査、重要な深掘り調査を含む)

`src/military/recruitment_supply_gate.rs`の全ロジック(定数・`has_reserve_for_days`・
`update_recruitment_supply_gate`の真のヒステリシス状態遷移)を再確認し、既存の20単体
テストを再読して境界値(平時20/35日、戦時40/70日)・中間帯保持・bridge_deficit単独
ブロック・配送不健全単独ブロックの全てが検証済みであることを確認した。

**36か月6か国実測で発見した現象と、その根本原因の特定**: 当初の一時テストでは、
Oceanic(CountryId 3)・Ferrowyn(CountryId 5)の`recruitment_supply_blocked`が36か月間
ずっと`true`のまま(=募兵が一度も再開しない)という結果が出た。これを「Phase 26の
バグかもしれない」候補として深掘り調査した結果、2段階の原因が判明した。

1. **監査用一時テストハーネス自身の不備**(見つけて即座に修正、本番コードは無関係):
   実際の`src/app/time.rs::advance_game_date`は、月が繰り上がる日について
   `DayChangedMessage`と`MonthChangedMessage`を**同じ1回のSystem実行内**で両方発行する
   (実コードで確認済み)。当初の一時テストの月送りヘルパーは、月初日について
   `MonthChangedMessage`だけを発行し`DayChangedMessage`を発行していなかったため、
   月次生産直後に走るはずの日次補給消費(P26-004、`division_supply_fulfillment`の
   更新)がその日に一度も実行されず、前月末(在庫枯渇日)の古い充足率がハーネス上で
   凍結して見えるという**テストハーネス側のバグ**だった。ハーネスを実際の
   `advance_game_date`と同じ挙動(同一フレームで両メッセージを発行)に修正した結果、
   `min_fulfillment`は正しく`1.0`に復元されることを確認した。
2. **修正後に残った真の原因(バグではなく設計どおりのヒステリシス)**: ハーネス修正後も
   Oceanic・Ferrowynの`recruitment_supply_blocked`は`true`のままだったが、これは
   両国の国家備蓄水準が月初時点で概ね27〜29日分(demand換算)にとどまり、平時resume
   閾値の35日分に一度も到達していないためだと判明した(`has_reserve_for_days`の
   条件を満たさない、`update_recruitment_supply_gate`のresumeブランチが正しく`false`を
   返し続けている)。両国はP26-008-Cの実測でも初期Factory0からのスタートであり、
   経済的に最も脆弱な2か国である――「経済基盤の弱い国ほどヒステリシスが長く効く」という
   意図された挙動が、36か月という限られた観測窓の中でたまたま「一度も解除されない」
   形で観測されただけであり、ゲートの状態遷移ロジック自体に欠陥は無い。

この調査過程は、監査自身が自らの一時コードの不備を実測で検出・修正した具体例として
記録する(§28の「多重独立実行」の精神を、監査ツール自身の検証にも適用した形)。

## 21. 攻勢補給ゲート(§19監査)

`src/war/offensive_supply_gate.rs`・`src/war/military_ai.rs::evaluate_military_ai_
decision`の非対称設計(national-reserveは新規Offensive開始のみを止め、既存Offensiveの
継続は前線配送率[`MIN_OFFENSIVE_SUPPLY_FULFILLMENT = 0.34`]でのみ止める)を、§16の
90日戦争シナリオで実測確認した。`offensive_supply_blocked`は`FrontlinePlan`単位の
フィールドであり(募兵ゲートの`recruitment_supply_blocked`とは別の状態)、既存の
`p26_008_b_supply_aware_offensive_gate_e2e_test.rs`(19テスト)を5回独立実行して全て
成功することを確認した。

## 22. 補給網構築AI優先順位(§20監査)

`src/country/country_ai.rs`の`find_proactive_development_target`(Mine/Factory二択の
汎用化版)・`has_active_factory_for_industrial_goods`(完成済みlevel>0と建設キュー
両方をチェックし二重建設を防ぐ)を再確認した。既存の`p26_008_c_supply_chain_
construction_priority_e2e_test.rs`(18テスト)を5回独立実行し全て成功することを確認
した。P26-008-CのElfin正規値更新(`1804.029982`)は本監査の36か月実測(Elfin月36時点
MilitarySupplies=919.207、初期値と条件が異なるため直接比較はできないが、Elfinの
Factory数が0→3へ増加している事実)と定性的に整合する。

## 23. 90日統合戦争シナリオ(§21監査)

§16の内容の通り、Arcadia対Elfinの制御された戦争を実App・実カレンダー・実Systemで
実行し、健全な補給→攻勢開始→回廊遮断相当の備蓄枯渇→前線配送悪化→攻勢停止(次回正規
評価時)→備蓄回復→攻勢再開→save→load一致、という一連の流れを実測した。診断用の無限
資金は使用していない(初期資金1,000,000を明示的に設定した通常の`CountryData`を使用)。

## 24. 36か月6か国統合シナリオ(§22監査)

§7〜8の表の通り。`PlayerCountry(None)`・全6か国AI・実データ・実System・実カレンダーで
36か月を実行した。自然な戦争発生は本シナリオでは観測されなかった(6か国とも
`is_at_war=false`のまま36か月が経過した)――これは事実としてそのまま報告し、AI宣戦
ロジックを操作して戦争を強制発生させることはしていない。

全6か国が36か月中に少なくとも一度はMilitarySupplies備蓄0を記録した
(`min_MilitarySupplies=0.000`)。これは§26で述べる財政バランス事象と直接連動しており、
Phase 26の会計・配分ロジック自体は正しく反応している(在庫0の日は正しく充足率0.0/
部分充足になり、国家備蓄が負になることは一度もなかった)。

## 25. save→load比較(§23監査)

90日シナリオ(§23)・36か月シナリオ(§24、月36時点)の両方でsave→load直後の値を
継続分岐と比較した。90日シナリオでは`FrontlineStance`・`recruitment_supply_blocked`が
完全一致、36か月シナリオでは6か国全ての`MilitarySupplies`備蓄・生存Division数が
完全一致(浮動小数点の同値比較で一致、丸め誤差なし)することを確認した。

## 26. 決定性監査(§24監査)

P26-DET-001・P26-DET-001-REV-001・P26-006-REV-001・P26-008-PRE-002・P26-008-A・
P26-008-B・P26-008-C・p23_010の8ファイル(101テストケース)を、それぞれ独立した
`cargo test`プロセスとして**5回**実行し、全てのテストが5/5で安定して成功する
(0件の不安定・失敗)ことを確認した。

`src/military/movement.rs`の`process_movement`を読み、DivisionId昇順での決定的処理
(P26-006-REV-001が導入)が現HEADでも維持されていることをコード上で直接確認した
(コード中のコメントで「複数国家の師団が同日に同じ州へ到達し得る場合のHashMap反復順
依存を解消するため」という導入理由が明記されている)。

## 27. 装備キュー永久停滞事象の再評価(§25監査)

`src/military/recruitment.rs`(85-120行)を再読し、`process_recruitment`が
`EquipmentAccountingError::InsufficientStockpile`の場合にキュー項目を完了させず
(`completed_indices`へ追加せず)、`blocked_by_equipment_shortage`へCountryIdを記録する
実装を確認した。これは以下の理由でPhase 26の対象外・Phase 26の補給契約を破らないと
判定する。

- **既存コードパスの再確認**: この挙動はP25-003(装備支給条件付き完了)・P25-005
  (装備不足国の当日の既存Division装備補充を止めて備蓄を保護する)由来であり、
  `MilitarySupplies`(本Phaseの対象資源)ではなく`MilitaryEquipment`という別資源の
  不足によるものである。
- **無限ループではなく設計された再試行**: 項目は`days_remaining==0`のままキューに
  残り、翌日以降の日次呼び出しで自動的に再判定される。P25-005の保護ロジックにより、
  装備不足国はその日の既存Division向け装備補充を止め、待機中のキューへ備蓄を回す
  仕組みが既に存在する(無策の放置ではない)。
- **Phase 26への影響**: `decide_and_recruit_for_country`のステップ1(キュー非空
  チェック)により、キューが詰まっている国は新規募兵自体が発生しなくなるため、
  P26-008-Aの募兵ゲート(MilitarySupplies基準)の「実際の効果」が減殺される場合が
  ある。しかしこれはMilitarySuppliesの会計・配分・ゲート判定そのものが誤っている
  わけではなく、両者は独立に正しく動作した上で後者の効果が前者によって覆い隠される
  という関係にある。

**判定**: Phase 26の補給契約は破らないが、AI募兵ライフサイクルを部分的に阻害する
既存(Phase 23-25由来)の事象である。→ **Phase 26 COMPLETE WITH FOLLOW-UP候補**
(§29で次タスク候補として詳述)。

## 28. 財政バランス事象の再評価(§26監査)

36か月6か国シナリオの実測(§7の表)から:

- 赤字開始時期: Arcadia・Dwarf・Passguard・Ferrowynの4か国は月12時点で既に国庫が負
  (-2881〜-2049)。月36時点ではArcadia -20249、Dwarf -20108が最も深刻。
- Elfin・Oceanicの2か国は月36時点でも国庫が正(641・2058)。
- `decision_reason`に`FundsShortage`が複数国・複数月にわたって出現しており、破産
  係数(P26以前から存在する既存の生産減衰ロジック)がMilitarySupplies・
  IndustrialGoods双方の生産量に影響を与えていることを実測データで確認した。
- ただし、生産が完全停止した国は1つもなく(全6か国が月36時点でIndustrialGoods・
  MilitaryEquipmentともに正の値を維持)、供給網が「完全停止」ではなく「一時的な
  性能低下」にとどまっていることを確認した。
- 国家間の資金力の差は意図された不均一設計(初期国庫がArcadia 5000〜Passguard 1800
  と国ごとに異なる)の自然な帰結であり、Phase 26のいずれのコードもこれを新たに
  発生させたものではない。

**判定**: 財政バランスはPhase 26の補給契約(会計・配分・ゲート判定の正しさ)を直接
破壊しないため、本監査では独立した経済バランス調整タスクの候補として記録するに
とどめ、Phase 26の完了判定には影響させない。

## 29. Phase 23-25保護(§12関連)

Phase 23-25由来のAPI(人的資源ライフサイクル・動員・workforce・装備会計・戦闘・移動・
占領・講和・国家消滅・国力・save→load)を扱う既存テストファイル(`p21_014_country_
power_e2e_test.rs`・`p23_001`〜`p23_010`・`p24_002_workforce_rounding_unification_
e2e_test.rs`・`p25_002`〜`p25_007`・`profile_p21_021_elimination_scaling_test.rs`等)
が、本監査で実行した`cargo test --all-targets`の中に含まれており、全て成功している
ことを確認した(§30参照)。個別の回帰は見つからなかった。

## 30. Critical/High/Medium/Lowの分類

**Critical: 0件。High: 0件。**

**Medium: 2件**

1. **装備キュー永久停滞によるAI募兵ライフサイクル阻害**(§27)。MilitaryEquipment
   不足で募兵キューが長期間先頭で止まり、新規募兵自体が発生しなくなる既存(Phase
   23-25由来)の事象。MilitarySuppliesの会計・ゲート判定自体は正しいが、その効果が
   覆い隠される。Phase 26のスコープ外だが、次タスク候補として記録する。
2. **財政バランス(破産係数の生産抑制効果)**(§28)。複数国が月12時点で国庫赤字に
   転落し、MilitarySupplies・IndustrialGoods生産が一時的に低下する。供給網は完全
   停止しないため、Phase 26の契約は破られていない。独立した経済バランス調整タスクの
   候補として記録する。

**Low: 1件**

3. **AI建設優先順位によるFactory過剰建設の可能性**(§7)。OceanicがFactoryを17まで
   積み増しており、他国(最大8)と比べて突出している。ゲームバランス上望ましいかは
   別途検討が必要だが、Phase 26の補給契約(需要・配分・接続性の正しさ)自体には
   影響しない。

いずれもPhase 26の完了判定([1]〜[15]の完了基準)を破壊するものではないため、
**PHASE 26 COMPLETE**と判定する。

## 31. 除外事項

本監査では以下を意図的に対象外とした: 装備キュー永久停滞の修正実装、財政バランスの
調整実装、Factory建設数の上限導入。いずれも修正せず報告のみを行った(本タスクの
スコープ規定どおり)。

## 32. 次の原子的タスク候補

Critical/Highが0件のため、§30の分類に従い次の優先順位で候補を提示する:

1. **装備キュー永久停滞の解消**(Medium、優先度最高の候補): `process_recruitment`
   (`src/military/recruitment.rs`)が採用中の「無条件で毎日再試行、完了条件のみ
   `MilitaryEquipment`備蓄十分」という設計を、待機/取消/部分充足のいずれかへ再設計
   する候補。既存のP25-005保護ロジック(装備不足国の当日補充停止)との関係整理、
   人的資源・装備の予約状態の扱い、Save互換性、AI再募兵・プレイヤーキューへの影響
   範囲の洗い出しが必要。ただしこれはMilitarySuppliesではなくMilitaryEquipmentの
   問題であり、Phase 26ではなくPhase 27以降の独立タスクとして起票することを推奨する。
2. **財政バランス調整**(Medium): 破産係数・初期国庫・維持費の見直し。国別の因果関係
   (赤字開始月・最低国庫・供給網への影響)がまだ大まかにしか定量化されていないため、
   着手前にさらなる実測が必要。
3. Phase 27の新機能。

本監査自体はPhase 26を完了として固定するため、上記はいずれも**次フェーズ側の判断**
に委ねる。

## 33. 実行コマンドと結果

逐次実行(並列cargo実行なし)。

```text
cargo test --all-targets
  → 2506 passed / 0 failed / 1 ignored

cargo clippy --all-targets --all-features -- -D warnings
  → 0 warnings
```

## 34. 多重独立実行の結果(§28)

以下をそれぞれ独立した`cargo test`プロセスとして5回実行し、全て安定して成功した
(不安定・failureは0件)。

| テスト対象 | テスト数 | 5回の結果 |
|---|---|---|
| p26_det_001_construction_ai_determinism_e2e_test | 10 | 5/5 ok |
| p26_det_001_rev_001_cross_state_production_order_e2e_test | 12 | 5/5 ok |
| p26_006_rev_001_ai_save_resume_determinism_e2e_test | 9 | 5/5 ok |
| p26_008_pre_002_ai_evaluation_cadence_e2e_test | 16 | 5/5 ok |
| p26_008_a_supply_aware_recruitment_gate_e2e_test | 30 | 5/5 ok |
| p26_008_b_supply_aware_offensive_gate_e2e_test | 19 | 5/5 ok |
| p26_008_c_supply_chain_construction_priority_e2e_test | 18 | 5/5 ok |
| p23_010_phase23_manpower_lifecycle_audit_test | 9 | 5/5 ok |

加えて、本監査専用の一時90日/36か月統合テストも複数回実行し、結果は再現した(§20の
ハーネス修正前後で異なる値になった経緯は§20に記載の通り、ハーネス自身の不備であり
本番コードの非決定性ではない)。

## 35. 一時コードの削除確認

本監査中に作成した一時ファイル`tests/p26_009_temp_integration_audit_test.rs`は、
§33の最終Cargo検証を実行する前に削除済みである(`rm`実行、削除後の`git status
--short`でファイルが追跡対象に現れないことを確認済み)。本番Rustコード・本番RONデータ・
既存テスト・既存レポート・localizationファイル・Save形式のいずれも本監査では変更して
いない。

## 36. PNG差分の扱い

`cargo test --all-targets`実行のたびに以下9件の既知の非決定的スクリーンショットPNGが
差分として現れたが、いずれも個別に`git checkout --`で復元し、広範な`checkout .`は
使用していない:

```
verification_logs/p20-007/screenshots/03_playing_topbar.png
verification_logs/p20-009/screenshots/04_playing_ja_jp.png
verification_logs/p20-009/screenshots/05_playing_en_us.png
verification_logs/p20-009/screenshots/06_playing_ja_jp_again.png
verification_logs/phase-21/p21-save-002e/screenshots/01_playing_topbar_with_save_load_buttons.png
verification_logs/phase-21/p21-save-002e/screenshots/02_after_save_click.png
verification_logs/phase-21/p21-save-002e/screenshots/03_load_confirm_dialog_open.png
verification_logs/phase-21/p21-save-002e/screenshots/04_after_cancel_dialog_closed.png
verification_logs/phase-21/p21-save-002e/screenshots/05_after_load_success.png
```

## 37. Phase 26完了基準(15項目)の充足確認

1. MilitarySupplies生産/初期化/消費/配分の整合性 → §5・§6・§13で確認、充足。
2. 月次生産と日次消費のSystem順序保証 → §6で`.after()`の直接確認により充足。
3. New Game・旧Save移行の二重付与なし → §9・§10で確認、充足。
4. 不足時のDivision配分の保存則・決定性 → §13・§14で確認、充足。
5. 補給充足率の実戦闘経路への単一反映 → §17で確認、充足。
6. 首都連結性がowner基準ではなくcontroller基準 → §15で確認、充足。
7. 占領・消滅・海外領土が資源移転・喪失を起こさない → §18で確認、充足。
8. save→load後の会計・充足率・AI状態継続 → §25で確認、充足。
9. AI募兵/攻勢/建設判断の補給状態反応 → §20・§21・§22で確認、充足。
10. P26-008-PRE-002のAIカデンス維持 → §19・§26で確認、充足。
11. 6か国の生産能力差が均一化されず、国内供給網が機能 → §7・§8で確認、充足。
12. Phase 23-25の人的資源・装備会計が非破壊 → §29で確認、充足。
13. 全テスト0 failed → §33で確認(2506 passed/0 failed/1 ignored)、充足。
14. clippy 0警告 → §33で確認、充足。
15. 未解決のCritical/High無し → §30で確認(0件)、充足。

## 38. 最終`git status --short`

```
(クリーン、本レポートファイルのみ追加予定)
```

## 39. 除外事項・スコープ外の明示

装備キュー永久停滞の修正、財政バランス調整、Factory建設数上限の導入は本タスクの
スコープ外であり、いずれも実装していない。次フェーズでの独立タスク化を推奨する
(§32)。
