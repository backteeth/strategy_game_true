# P26-003-REV-003: 軍用補給物資追加による国力評価の単位依存の訂正

## 1. 総合判定

**COMPLETE**

`src/country/power.rs::compute_state_aggregates`の経済力算出式を、建物の`output_resources`(出力資源の物量の生合計)を参照する方式から、建物種類ごとの明示的な基礎能力点(`base_economic_capacity_points`、`output_resources`を一切参照しない)を参照する方式へ訂正した。既存建物(Farm/LoggingCamp/Factory/MilitaryFactory/CrystalMine/CrystalRefinery)の基礎点は、MilitarySuppliesが追加される直前のコミット`973445c30d920c80908635f22acc45ffb5655dac`時点の`output_resources.values().sum()`をそのまま校正基準として採用しており、既存建物の経済力評価を一切変えていない(校正であり、国別の順位調整ではない)。実データ6か国28州マップで、P26-003-REV-001が実測したElfin Republic/Dwarf FederationのPowerTire入れ替わり(GreatPower⇔RegionalPower)が、973445c時点と同じ側へ正しく戻ることを実測で確認した。生産会計(`src/economy/production.rs`・`src/economy/resources.rs`・MilitarySupplies/MilitaryEquipmentの会計)は一切変更していない。全テスト2025 passed/0 failed/1 ignored(開始時基準2003 passed/0 failed/1 ignoredに対し、新規テスト22件の追加分のみ増加、既存テストは1件も失敗・削除・弱体化なし)、clippy警告0。

## 2. 開始時HEAD・訂正対象コードHEAD・比較基準HEAD

```text
開始時HEAD:       c03bd3ef25c50a936f2a3d5164e8eac20ccb042d (P26-003-REV-002コミット)
訂正対象コードHEAD: c03bd3ef25c50a936f2a3d5164e8eac20ccb042d(このコミット上に実装)
比較基準HEAD:      973445c30d920c80908635f22acc45ffb5655dac (MilitarySupplies追加前)
```

開始直後に確認:

```text
$ git rev-parse HEAD
c03bd3ef25c50a936f2a3d5164e8eac20ccb042d

$ git status --short
 M saves/savegame_v1.ron

$ git log --oneline --decorate -15
c03bd3e (HEAD -> main) fix: aggregate military supplies production in integer units
2acd9a5 docs: audit military supplies coproduction design
56e61b6 feat: produce military supplies
973445c fix: add military supplies localization
4e63355 feat: add military supplies accounting foundation
b44619a docs: design military supply and logistics system
150e2ae docs: finalize phase 25 equipment lifecycle audit
...(以下、P25系列と一致)
```

このコミット自身の最終ハッシュは、コミット後に別途`git rev-parse HEAD`で確認する(本報告書には記載しない、§26参照)。

## 3. 問題の再現

実データ(`assets/data/states.ron`等、`AppPlugin`経由の実App)を使い、`evaluate_country_power`(公開API)を直接呼び出し、「973445c時点相当(MilitaryFactory: MilitaryEquipment:20.0のみ)」と「現行(MilitaryFactory: MilitaryEquipment:20.0+MilitarySupplies:100.0)」を比較した(訂正前コードで実測、`tests/tmp_rev003_probe_test.rs`という一時テストで検証、確認後に削除済み・本コミットには含まれない)。

```text
country 0 (Arcadia):  BEFORE eco_raw=1130.0 total=92.4957 rank=1 GreatPower    | BUG(訂正前) eco_raw=1330.0 total=92.8368 rank=1 GreatPower   (変化なし)
country 1 (Elfin):    BEFORE eco_raw=1200.0 total=90.0255 rank=2 GreatPower    | BUG(訂正前) eco_raw=1300.0 total=88.0299 rank=3 RegionalPower
  >>> TIER CHANGED: GreatPower -> RegionalPower
country 2 (Dwarf):    BEFORE eco_raw=1230.0 total=89.8695 rank=3 RegionalPower | BUG(訂正前) eco_raw=1430.0 total=89.8695 rank=2 GreatPower
  >>> TIER CHANGED: RegionalPower -> GreatPower
country 3 (Oceanic):  BEFORE eco_raw=600.0  total=84.6341 rank=4 RegionalPower | BUG(訂正前) eco_raw=600.0  total=82.5874 rank=4 RegionalPower (絶対値不変、他国の伸びにより相対的に低下)
country 4 (Passguard):BEFORE eco_raw=400.0  total=61.0355 rank=5 MinorPower   | BUG(訂正前) eco_raw=600.0  total=63.8668 rank=5 MinorPower (変化なし)
country 5 (Ferrowyn): BEFORE eco_raw=150.0  total=56.7395 rank=6 MinorPower   | BUG(訂正前) eco_raw=250.0  total=58.3257 rank=6 MinorPower (変化なし)
```

この数値はP26-003-REV-001報告書§15が実測した概算値(Elfin total=90.03等)を独立に高精度で再現しており、報告書間で一貫性がある。P26-003-REV-001の指摘どおり、`tests/p21_014_country_power_e2e_test.rs`の既存テストはTier「個数」分布(2,2,2)のみを検証しており、国別のTier識別を検証していなかったため、この入れ替わりを検出できなかった。

## 4. 変更前の国力評価式

```rust
} else if let Some(def) = building_registry.get(b_type)
    && !def.output_resources.is_empty()
{
    let per_level: f64 = def.output_resources.values().sum();
    state_economic += per_level * level as f64;
}
```

建物定義`output_resources`(`HashMap<ResourceType, f64>`)の値の合計を、建物レベルに乗じて経済力へ加算していた。この式は、建物が産出する資源の「種類数」が増えるだけで(1レベルあたりの実際の生産価値とは無関係に)経済力が線形に増大するという構造的欠陥を持っていた。

## 5. 国力の全下流参照

`rg -n "CountryPowerRegistry|power_tier|PowerTier" src`で再確認した本番参照箇所(16ファイル、うち3ファイルはテストコード自身のfixtureで実質的な下流ではない):

- `src/country/elimination.rs` — 国家消滅時のPowerTireランキング除外処理
- `src/country/mod.rs` — Registry定義・再構築フック
- `src/diplomacy/crisis_response.rs` — Crisis応答判断
- `src/diplomacy/update.rs` — P21-015: GreatPowerのAI国家によるCrisis自動支持(`evaluate_ai_crisis_support`)
- `src/diplomacy/ai_crisis_start.rs` — AI Crisis開始判断
- `src/diplomacy/ai_claim_generation.rs` — AI Claim生成の候補判断
- `src/diplomacy/claims.rs` — Claim関連ロジック
- `src/war/peace.rs`・`src/war/update.rs` — 講和・戦争関連の国力参照
- `src/ui/diplomacy_panel.rs`・`src/ui/peace_panel.rs` — UI表示
- `src/save/validate.rs` — Save検証コンテキスト(建物定義の妥当性検証、経済力の再計算自体は行わない)

いずれも`CountryPowerRegistry`(公開API `get`/`ordered_country_ids`)を読むだけであり、本タスクはこれらのファイルを一切変更していない。訂正は`evaluate_country_power`が返す値そのものを正すため、これら全下流は次回の再評価(月次・Load直後・OnEnter Playing)から自動的に訂正後の値を参照する。

## 6. 潜在能力と実生産の区別

訂正後も、経済力は「潜在生産能力の評価点」(建物レベル×基礎能力点)のままであり、実際の月次生産量・市場価格・GDP・現在の資源備蓄・軍隊への補給充足率とは異なる。原材料不足で実際には生産が停止していても(P26-003-REV-001§10-13で実測したElfin/Oceanic等の恒久的生産不能国)、この評価点は変わらない — これは訂正前の`compute_state_aggregates`の性質を維持したものであり、今回新たに導入した性質ではない。実際の稼働率・資源不足・市場価格を国力へ反映する設計変更(P26-003-REV-001§15「訂正候補」で触れられていた案)は、本タスクのスコープ外として意図的に混ぜていない。

## 7. 比較した訂正方式

| 案 | 内容 | 判定 |
|---|---|---|
| **案1(採用): 建物種類ごとの明示的基礎能力点** | `output_resources`を一切参照せず、`BuildingType`ごとの定数を網羅的`match`で定義 | 採用。校正基準が既存の`output_resources.values().sum()`から機械的に求まり、恣意性がない。将来の新`BuildingType`追加時、ワイルドカードなしの`match`がコンパイルエラーで定義漏れを検出する。 |
| 案2: 資源別の明示的評価係数 | `ResourceType`ごとに係数を定義し、`Σ(quantity × coefficient)`で経済力を算出 | 不採用(比較検討)。経済的価値の根拠(1 Foodと1 Ironが国力へ与える価値の相対比)が未整備であり、係数自体が恣意的になる。仕様書(§7)も「経済価値の根拠が未整備な状態で恣意的な係数を追加するより、明示的な建物能力点を優先」と明記している。 |
| 案3: MilitarySuppliesだけを合計から除外する特例 | `output_resources`から`MilitarySupplies`キーだけ除外して合計 | 不採用(禁止方式)。将来別の資源が追加されるたびに特例が増える対症療法であり、「単位依存」という根本原因を解消しない。 |
| 案4: 軍需工場だけを6で割る補正 | MilitaryFactoryの寄与だけ`/6` | 不採用(禁止方式)。国家ID・建物種別を狙い撃ちした補正であり、他の建物が将来同種の問題を起こした際に再発する。 |
| 案5: Elfinを直接GreatPowerにする補正 | 国家ID別の閾値・加点 | 不採用(明確に禁止)。 |

## 8. 採用方式と理由

案1(建物種類ごとの明示的基礎能力点)を採用した。理由:
1. 校正基準(973445c時点の`output_resources.values().sum()`)が機械的に一意に定まり、恣意的な数値選択の余地がない。
2. 網羅的`match`(ワイルドカードなし)により、将来`BuildingType`へ新variantが追加された際、この関数の実装がコンパイルエラーで定義漏れを検出する(§8要件「新しいBuildingType追加時に評価定義漏れを検出できる」を満たす)。
3. `output_resources`の物量・挿入順・種類数への依存を完全に断ち切れる(実装がそもそもこのHashMapを参照しない)。
4. 既存の正規化・重み(50/30/20)・Tier判定・tie-break・Mine専用分岐など、経済力以外の全ロジックを一切変更せずに済む(スコープを最小限に保てる)。

## 9. 全BuildingTypeの基礎点表

| BuildingType | 旧評価への寄与(973445c時点) | 新基礎点 | level反映 | 採用理由 |
|---|---|---|---|---|
| Farm | Food:100.0 → 100.0 | **100.0** | ×level(線形) | 973445c時点の`output_resources.values().sum()`をそのまま採用 |
| LoggingCamp | Wood:80.0 → 80.0 | **80.0** | ×level | 同上 |
| Mine | `output_resources`は973445c時点から空(実際の産出は州の鉱床データ経由) | **0.0**(この関数は呼び出されない、鉱床ベースの専用分岐で別計算) | 専用分岐(既存のまま変更なし) | 鉱床データはP21-014導入時から`output_resources`と独立した専用ロジックであり、本訂正の対象外 |
| Factory | IndustrialGoods:30.0 → 30.0 | **30.0** | ×level | 973445c時点のまま |
| MilitaryFactory | MilitaryEquipment:20.0 → 20.0(973445c時点、MilitarySupplies追加前) | **20.0** | ×level | 校正基準そのもの。現行`output_resources`はMilitaryEquipment:20.0+MilitarySupplies:100.0(合計120.0)だが、この120.0は一切参照しない |
| Railway | 973445c時点から空 | **0.0** | ×level(常に0) | 物流ボーナスのみで経済生産物ではない(973445c時点から寄与0、変更なし) |
| University | 973445c時点から空 | **0.0** | ×level(常に0) | 研究力のみで経済生産物ではない(同上) |
| MagicAcademy | 973445c時点から空 | **0.0** | ×level(常に0) | 魔法力のみで経済生産物ではない(同上) |
| CrystalMine | RawMagicCrystal:20.0 → 20.0 | **20.0** | ×level | 973445c時点のまま(MilitarySuppliesとは無関係、変更なし) |
| CrystalRefinery | MagicCrystal:10.0 → 10.0 | **10.0** | ×level | 973445c時点のまま |

全10 `BuildingType`variantを網羅。既存で経済寄与0だった建物(Mine自身の`output_resources`分岐/Railway/University/MagicAcademy)へ根拠なく加点していない。既存で評価対象だった建物(Farm/LoggingCamp/Factory/MilitaryFactory/CrystalMine/CrystalRefinery)を漏らしていない。実装は`src/country/power.rs::base_economic_capacity_points`(網羅的`match`、ワイルドカードなし)。

## 10. 校正の根拠

`git show 973445c30d920c80908635f22acc45ffb5655dac:strategy_game/assets/data/buildings.ron`を直接読み取り、各`BuildingType`の`output_resources`を実データから確認した(推測・概算ではなく実ファイルの値をそのまま転記)。校正対象外はMineのみ(鉱床専用分岐、`output_resources`ベースの校正の対象外である旨は973445c時点でも現行でも変わらない)。`tests/p26_003_rev_003_economic_power_capacity_e2e_test.rs::category_c_calibration_against_pre_p26_003_definitions`で、全9建物種別(Mine除く)について、973445c時点の定義から計算した経済力と、訂正後コードが現行(MilitarySupplies入り)定義から計算した経済力が一致することを直接検証している。

## 11. 訂正後の正規式

```rust
fn base_economic_capacity_points(building_type: BuildingType) -> f64 {
    match building_type {
        BuildingType::Farm => 100.0,
        BuildingType::LoggingCamp => 80.0,
        BuildingType::Mine => 0.0, // 鉱床専用分岐、ここは呼び出されない
        BuildingType::Factory => 30.0,
        BuildingType::MilitaryFactory => 20.0,
        BuildingType::Railway => 0.0,
        BuildingType::University => 0.0,
        BuildingType::MagicAcademy => 0.0,
        BuildingType::CrystalMine => 20.0,
        BuildingType::CrystalRefinery => 10.0,
    }
}
```

呼び出し側(`compute_state_aggregates`):

```rust
} else if building_registry.get(b_type).is_some() {
    state_economic += base_economic_capacity_points(b_type) * level as f64;
}
```

`building_registry.get(b_type).is_some()`のガードは訂正前と同じ意味(その`BuildingType`が`BuildingRegistry`に定義されていることの確認)を維持しており、`def`の中身(`output_resources`)は一切参照しない。

## 12. 資源物量単位からの独立性

`tests/p26_003_rev_003_economic_power_capacity_e2e_test.rs::category_b_independence_from_output_quantities`(5テスト)で以下を検証:
- MilitarySuppliesの出力量を100.0→1.0→10,000.0→0.0001へ変えても経済力が不変。
- MilitaryEquipment(既存資源)の出力量を1.0→1,000.0→999,999.0へ変えても経済力が不変。
- 出力資源を1種類→2種類→3種類へ増やしても(併産追加)経済力が不変。
- `output_resources`の挿入順序を変えても経済力が不変(HashMap挿入順非依存)。
- 出力mapが空でも、その建物種別の基礎点(例: Farm=100.0)がそのまま適用される契約。

いずれも全て合格(実測、5/5 passed)。加えて`category_a_base_points_coverage::every_building_type_has_an_explicit_calibrated_contribution_at_level_1`では、各建物種別に本来の出力とは異なる(意図的に不一致な)出力資源を登録した状態でも、期待どおりの基礎点が返ることを検証しており、「output_resourcesの中身とは無関係」という契約をさらに強く裏付けている。

## 13. 国家別の変更前・現行・訂正後比較

実データ6か国(`tests/p26_003_rev_003_economic_power_capacity_e2e_test.rs::real_map::all_six_countries_match_the_calibrated_pre_p26_003_scores_ranks_and_tiers`で固定):

| CountryId | 国家名 | 973445c時点(BEFORE) | P26-003バグ状態(訂正前、参考) | 本訂正後(AFTER FIX) |
|---|---|---|---|---|
| 0 | Kingdom of Arcadia | eco=1130.0 total=92.4957 rank=1 GreatPower | eco=1330.0 total=92.8368 rank=1 GreatPower | eco=1130.0 total=92.4957 rank=1 GreatPower |
| 1 | Elfin Republic | eco=1200.0 total=90.0255 rank=2 **GreatPower** | eco=1300.0 total=88.0299 rank=3 **RegionalPower** | eco=1200.0 total=90.0255 rank=2 **GreatPower** |
| 2 | Dwarf Federation | eco=1230.0 total=89.8695 rank=3 **RegionalPower** | eco=1430.0 total=89.8695 rank=2 **GreatPower** | eco=1230.0 total=89.8695 rank=3 **RegionalPower** |
| 3 | Oceanic Magic Empire | eco=600.0 total=84.6341 rank=4 RegionalPower | eco=600.0 total=82.5874 rank=4 RegionalPower | eco=600.0 total=84.6341 rank=4 RegionalPower |
| 4 | Passguard March | eco=400.0 total=61.0355 rank=5 MinorPower | eco=600.0 total=63.8668 rank=5 MinorPower | eco=400.0 total=61.0355 rank=5 MinorPower |
| 5 | Ferrowyn League | eco=150.0 total=56.7395 rank=6 MinorPower | eco=250.0 total=58.3257 rank=6 MinorPower | eco=150.0 total=56.7395 rank=6 MinorPower |

「本訂正後(AFTER FIX)」列は、現行(MilitarySuppliesを含む)`buildings.ron`をそのまま使い、訂正後コードで`evaluate_country_power`を呼び出した実測値。「973445c時点(BEFORE)」列と完全一致していることを確認した(economic_raw/military_raw/population_raw/world_rank/power_tierは完全一致、total_scoreはf32丸め誤差の範囲(<0.01)で一致)。全6か国の`military_raw`は10000.0、`population_raw`は国家ごとに異なる(2,860,000/2,060,000/1,890,000/3,830,000/245,000/590,000)。

## 14. Tier逆転の訂正結果

`tests/p26_003_rev_003_economic_power_capacity_e2e_test.rs::real_map::elfin_and_dwarf_tier_flip_from_p26_003_rev_001_is_reversed`および`tests/p21_014_country_power_e2e_test.rs::real_map_e2e::real_map_tier_assignment_is_identified_per_country_not_just_counted`(新規追加)で確認:
- Elfin Republic(CountryId(1)): 訂正後は**GreatPower**(世界順位2位)。P26-003バグでは誤ってRegionalPower(3位)へ格下げされていた。
- Dwarf Federation(CountryId(2)): 訂正後は**RegionalPower**(世界順位3位)。P26-003バグでは誤ってGreatPower(2位)へ格上げされていた。
- Tier「個数」分布(2,2,2)は訂正前後で不変(仕様どおり、`compute_tier_counts`は変更していない)。

これは「旧順位を復元しただけ」ではなく、「単位に依存しない明示的な能力点を校正した結果、たまたま973445c時点と同じ順位に戻った」という位置づけである(§7-11参照、国別補正は一切行っていない)。

## 15. 下流判断の結果

`tests/p26_003_rev_003_economic_power_capacity_e2e_test.rs::real_map::downstream_ai_crisis_support_uses_the_corrected_great_power_tier_and_is_map_size_independent`で、P21-015の本番Domain関数`diplomacy::update::evaluate_ai_crisis_support`を実データベースの`CountryPowerRegistry`(訂正後の経済力を反映)で直接呼び出し、Elfin Republic(CountryId(1)、訂正後は正しくGreatPower)がinitiatorの同盟国としてCrisisへ自動支持を表明することを確認した(`third_party_reactions`にエントリが追加される)。この経路は`diplomacy/update.rs`のコードを一切変更せず、訂正後の`CountryPowerRegistry`を渡すだけで正しく動作する — 外交・AIの判断式自体は変更していない(spec§12の制約どおり)。既存の`tests/p21_015_ai_crisis_support_e2e_test.rs`(11 passed、無変更)も引き続き合格しており、実App経由のGreatPower数上限(`real_map_has_at_most_two_great_powers`)等の既存ガードも壊れていない。

## 16. 正規化・重み・tie-breakの維持

`src/country/power.rs`の以下の関数・定数は本タスクで一切変更していない(diffで確認):
- `normalize`(世界最大値100正規化)
- `evaluate_country_power`内の`military_normalized * 0.50 + economic_normalized * 0.30 + population_normalized * 0.20`(50/30/20の重み)
- `compute_tier_counts`(Tier閾値: `(n*20).div_ceil(100).clamp(1,8).min(n)`等)
- ソート順(総合力→軍事力→経済力→人口→CountryId昇順のtie-break)

既存の`tests/p21_014_country_power_e2e_test.rs`の`total_score_uses_50_30_20_weights`・`tier_counts_match_the_specified_table`・`ties_are_broken_by_military_then_economic_then_population_then_country_id`(いずれも既存assert、無変更)が引き続き合格していることで、これらの不変性を裏付けている。

## 17. 決定性

`src/country/power.rs`内の既存単体テスト`insertion_order_does_not_affect_result`・`same_input_produces_same_result`(無変更、引き続き合格)に加え、`tests/p26_003_rev_003_economic_power_capacity_e2e_test.rs::category_b_independence_from_output_quantities::output_resources_insertion_order_does_not_matter`で、`output_resources`のHashMap挿入順序を変えても新しい経済力算出が完全に決定論的であることを確認した。実データSave/Load往復(`real_map::corrected_scores_survive_a_save_load_round_trip_unchanged`・`corrected_scores_survive_two_consecutive_save_load_round_trips`)でも、economic_raw/total_score/world_rank/power_tierが完全一致することを確認した。新たなNaN・Infinity・overflowは導入していない(`category_a_base_points_coverage::near_max_level_is_numerically_safe`で確認)。

## 18. Save V1・New Game/Loaded Game

`CountryPowerRegistry`はP21-014導入時から派生データであり、Saveへ保存しない(Country/State/Military/Buildingから毎回再構築、`src/country/power.rs`冒頭のdocコメント参照)。本訂正はこの設計を一切変更していない。新規Saveフィールド・Save V2・migrationは追加していない。`base_economic_capacity_points`の戻り値もSaveへ一切保存されない(コード内定数、実行時に毎回計算)。New Game初回評価(`rebuild_country_power_on_enter_playing`)・Loaded Game再評価(`rebuild_country_power_registry_from_world`、`apply_validated_save`から呼ばれる)・月次再評価(`rebuild_country_power_monthly`)のいずれも、訂正後の`evaluate_country_power`を呼び出すだけであり、呼び出し経路自体は無変更。

## 19. 生産会計への非影響

`src/economy/production.rs`・`src/economy/resources.rs`・MilitarySupplies/MilitaryEquipmentの会計APIは本タスクで一切変更していない(`git diff`で確認、これらのファイルへの差分はゼロ)。既存の以下のE2Eテストが無変更で合格していることで、生産会計・数量そのものへの非影響を確認した:
- `p26_002_military_supplies_accounting_foundation_e2e_test`: 45 passed(基準どおり)
- `p26_003_military_supplies_production_e2e_test`: 49 passed(基準どおり)
- `p26_003_rev_002_supply_production_units_e2e_test`: 56 passed(基準どおり、REV-002が確立した整数集計会計も無変更)

これらのテストは、MilitarySupplies/MilitaryEquipmentの実際の生産量・国家備蓄・原材料消費・REV-002の整数集計を検証しており、いずれも国力評価(`src/country/power.rs`)には依存しないため、本訂正の影響を受けない。消費System(需要・配分)は依然として未接続のまま(P26-004のスコープ、本タスクでは触れていない)。

## 20. 既存テスト変更理由

`tests/p21_014_country_power_e2e_test.rs`へ新規テスト1件(`real_map_tier_assignment_is_identified_per_country_not_just_counted`)を追加した。既存のassertは1件も変更・削除していない(`git diff`で確認可能、純粋な追加のみ)。追加理由: P26-003-REV-001が指摘したとおり、既存の`real_map_tier_split_matches_the_formula_for_6_countries`はTier「個数」分布(2,2,2)のみを検証しており、どの国がどのTierかを検証していなかったため、P26-003のバグ(Elfin/Dwarf Tier入れ替わり)を構造的に検出できなかった。この新規テストは、6か国それぞれのTire・世界順位を個別にハードコードして固定し、将来同種の回帰(経済力算出式が資源物量へ再び依存するようになる等)が発生した際に即座に検出できるようにする。

## 21. 対象内findingと残存finding

**対象内(本タスクで修正した)finding**: P26-003-REV-001が「Medium」と判定した「国力スコアへの影響」(§15、実データで2か国のPowerTireが入れ替わる)を修正した。Critical/High findingなし。

**本タスク後も残る別件(本タスクのCOMPLETEはこれらの解決を意味しない、§16参照)**:
- 生産持続可能国の偏り(未修正、P26-003-REV-001§10-13): 実データ6か国中5か国が、初日から恒久的(Elfin/Oceanic)、または実測で約18〜24か月以内(Arcadia/Passguard/Ferrowyn)にMilitarySuppliesを持続生産できない。P26-004(日次需要・配分)着手前にユーザー判断が必要(REV-001§27で提示済み)。
- 工場・鉱床・原材料不足(未修正)。
- AI建設・財政停止(未修正、treasury赤字化がAI建設全般を阻害する既存の経済バランス問題)。
- 初期備蓄0(未解決の設計事項)。
- 月次生産と日次需要の接続(P26-004のスコープ)。
- 全生産取引の原子性(未修正、P26-003-REV-001§18-19で指摘したMilitarySupplies commit失敗時の入力消費非対称性、到達可能性は極めて低い)。
- 日次需要・配分(未実装)。
- 補給充足率の戦闘反映(未実装、P26-005のスコープ)。

## 22. 変更ファイル

```text
$ git status --short (コミット直前、savegame_v1.ron除く)
 M src/country/power.rs
 M tests/p21_014_country_power_e2e_test.rs
?? tests/p26_003_rev_003_economic_power_capacity_e2e_test.rs
?? verification_logs/phase-26/p26-003-rev-003/p26-003-rev-003_economic_power_capacity_correction.md
```

`assets/data/buildings.ron`・`assets/data/resources.ron`・国家/州の初期データ・`src/economy/production.rs`・`src/economy/resources.rs`・`src/military/`・AI建設・財政・外交判断式・UI・ローカライズ・`src/save/`・既存報告書のいずれも変更していない(`git diff --stat`で確認)。

## 23. 実行コマンド・実測件数

すべて逐次実行(並列実行なし)。

```text
$ cargo test --lib country::
  → ok. 79 passed; 0 failed; 0 ignored (基準どおり、新規lib単体テストは追加していない)

$ cargo test --test p21_014_country_power_e2e_test
  → ok. 10 passed; 0 failed; 0 ignored (基準9→本タスクで+1、既存assertは無変更)

$ cargo test --test p26_003_rev_003_economic_power_capacity_e2e_test
  → ok. 21 passed; 0 failed; 0 ignored (新規ファイル)

$ cargo test --test p26_002_military_supplies_accounting_foundation_e2e_test
  → ok. 45 passed; 0 failed; 0 ignored (基準どおり)

$ cargo test --test p26_003_military_supplies_production_e2e_test
  → ok. 49 passed; 0 failed; 0 ignored (基準どおり)

$ cargo test --test p26_003_rev_002_supply_production_units_e2e_test
  → ok. 56 passed; 0 failed; 0 ignored (基準どおり)

$ cargo test --test p21_015_ai_crisis_support_e2e_test
  → ok. 11 passed; 0 failed; 0 ignored (基準どおり)

$ cargo test --test p21_017_ai_claim_crisis_e2e_test / p21_018_ai_claim_generation_e2e_test /
  p21_019_crisis_war_peace_resolution_e2e_test / p21_021_country_elimination_e2e_test (個別実行)
  → すべて基準どおり(7/5/4/1 passed、0 failed)

$ cargo test --test p20_009_localization_headless_render_test / p20_009_localization_resource_test
  → 4 passed / 8 passed(基準どおり)

$ cargo test --test p23_002/p23_004/p23_006/p23_007/p23_008/p23_009/p23_010/p24_002 (個別実行)
  → すべて基準どおり(8/7/22/24/21/22/9/17 passed、0 failed)

$ cargo test --test p25_002/p25_002_rev_001/p25_003/p25_004/p25_005/p25_006/p25_006_rev_001/p25_007 (個別実行)
  → すべて基準どおり(15/6/22/5/10/29/8/21 passed、0 failed)

$ cargo test --lib economy:: / cargo test --lib military:: / cargo test --lib save::
  → 32 / 283 / 300 passed(いずれも基準どおり)

$ cargo test --all-targets
  → 2025 passed; 0 failed; 1 ignored (開始時基準2003 passed/0 failed/1 ignoredに対し+22件、
    56バイナリ分の`test result:`行を合算して算出、2回実施して同一結果を確認)

$ cargo clippy --all-targets --all-features -- -D warnings
  → 0 warnings
```

新規テスト件数: `tests/p26_003_rev_003_economic_power_capacity_e2e_test.rs`(21件、新規ファイル)+`tests/p21_014_country_power_e2e_test.rs`(+1件、既存ファイルへの追加)=合計+22件。既存テストの削除・弱体化は0件。

## 24. clippy警告数

**0件**(`cargo clippy --all-targets --all-features -- -D warnings`、2回実行して確認)。新規テストファイル作成時に1件(`needless_borrow`、`evaluate_ai_crisis_support`呼び出しで既に`&CountryRegistry`型の変数へ余分な`&`を付けていた)を検出・即修正済み。

## 25. ユーザーセーブ保護

`saves/savegame_v1.ron`は本タスクを通じて一切読み書き・復元・ステージ・コミットしていない。開始時・全検証コマンド実行後・報告書作成前後のいずれの`git status --short`でも` M saves/savegame_v1.ron`のみが変更として残り続けたことを確認した(内容は一度も参照していない)。

`cargo test --all-targets`の実行により、既知の9件の非決定的スクリーンショットPNG(`verification_logs/p20-007/screenshots/03_playing_topbar.png`・`verification_logs/p20-009/screenshots/04_playing_ja_jp.png`・`05_playing_en_us.png`・`06_playing_ja_jp_again.png`・`verification_logs/phase-21/p21-save-002e/screenshots/01`〜`05`)が2回とも都度再生成されたが、そのつど個別パス指定の`git checkout --`で復元し、広範な`checkout`は一切使用していない。一時検証テストファイル(`tests/tmp_rev003_probe_test.rs`)は監査中に作成し、最終確認後に`rm`で削除済み(本コミットには含まれない)。

## 26. P26-004/P26-005の進行条件

**P26-004(日次需要・配分): READY(変更なし)**。本タスクは国力評価の算出式のみを訂正しており、P26-004が依存する会計API(`add_military_supplies_produced`/`try_consume_military_supplies`)・生産配線には一切触れていない。P26-003-REV-001が示した「READY」判定は本タスクによって変わらない。

**P26-005(戦闘力への補給充足率反映): 引き続きユーザー判断待ち(本タスクで自動的にREADYへ変更しない)**。本タスクが修正したのは国力評価(Tire判定)への影響のみであり、P26-003-REV-001§4・§10-13・§27が指摘した「実データ6か国中5か国が最終的にMilitarySuppliesを持続生産できない」という生産能力の偏り自体は一切未解決のまま(§21残存finding参照)。本タスクのCOMPLETEを理由に、この論点の解決を待たずP26-005へ進むべきではない。

## 27. 次に必要な原子的タスク1件

**推奨タスク**: P26-003-REV-001§27がすでに提示した「MilitarySupplies生産能力の偏りに関するユーザー判断の取得、および(判断結果に応じた)最小の対処設計」を、本タスク完了後も引き続き次の1件として推奨する。本タスクは国力評価という別軸の問題(単位依存)を修正したに過ぎず、実データ6か国中5か国が最終的にMilitarySuppliesを持続生産できないという構造的な偏り(P26-003-REV-001§10-13)には一切対処していない。P26-004(日次需要・配分)を実装する前、またはそれと並行して、この論点についてユーザーへ選択肢(1.非対称ゲームバランスとして受け入れる/2.州データへ追加鉱床を配置する/3.建設AIのMine選定ロジックへ鉱床の有無チェックを追加する)を確認し、選ばれた方針に基づく最小実装を1件行うことを推奨する。
