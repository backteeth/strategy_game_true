# P21-014 完了報告: 国家総合力評価・国家ランク基盤

最終検証日: 2026-08-19
最終検証コマンド実行環境: Windows 11, cargo (release/debug両方), rustfmt (edition 2024)

**最終ステータス: COMPLETE WITH MANUAL VERIFICATION PENDING**
(GUIでの対話的手動確認は実施していない。理由は §19 参照。)

---

## 1. 事前調査結果 (15項目)

実装着手前に以下をコード読解で確認した(すべて完了):

1. **既存の国家軍事力算出関数**: `src/country/country_ai.rs`に2つ存在する。
   - `pub fn calculate_country_total_power(country_id, military_registry, state_registry) -> u64`
     (行179): 1国家分。配備済み(`manpower > 0`かつ`status != Destroyed`)かつ陸上州
     (`!state.is_sea`)にいるDivisionを`evaluate_division_power`(`war::military_ai`)で
     採点し合計する。ArmyやAvailable Manpowerではなく**配備済みDivision**を数える。
   - `fn compute_total_power_by_country(military_registry, state_registry) -> Vec<u64>`
     (行211、当時private): 全国家一括版。P20-008最適化として既に存在していた
     (`military_registry.divisions`を1パスだけ走査し、`CountryId.0`をインデックスとした
     `Vec`へ振り分ける)。`calculate_country_total_power`の個別呼び出し結果と完全一致する
     ことを保証する既存回帰テスト
     (`test_compute_total_power_by_country_matches_individual_calculation`)が既にある。
2. **`CountryData`の全フィールド**(`src/country/mod.rs`): id/name/map_color/
   capital_state_id/treasury/government_type/economic_system/stockpile/tax_rate/
   monthly_income/monthly_expenses/monthly_balance/science_research_capacity/
   magic_research_capacity/economic_state/construction_queue/research_state/politics/
   current_reform/available_manpower/mobilized_manpower/
   total_military_equipment_required/available/monthly_military_expenses/
   recruitment_queue。「国家生産能力」に相当する集計フィールドは存在しない。
3. **`StateData`の該当フィールド**(`src/state/data.rs`): `owner_country_id`(法的所有)、
   `controller_country: Option<CountryId>`(実効支配、`controller()`は
   `unwrap_or(owner_country_id)`)、`population: u64`、
   `buildings: HashMap<BuildingType, u32>`(レベル)、
   `building_operation_ratios: HashMap<BuildingType, f32>`(稼働率、現在の変動値)、
   `resource_deposits: Vec<StateResourceDeposit>`。
4. **建物レベル・産出量を評価できる既存API**: `BuildingRegistry::get(BuildingType) ->
   Option<&BuildingDefinition>`(O(1))。`BuildingDefinition.output_resources:
   HashMap<ResourceType, f64>`が「レベル1あたりの基礎産出量」。ただし`Mine`だけは
   `assets/data/buildings.ron`上`output_resources: {}`(空)で、実際の産出は
   `economy::production::process_country_production`内で州の`resource_deposits`
   (discovered済み・MagicCrystal以外)を直接参照する特別扱いになっている
   (`CrystalMine`は逆に`output_resources`が非空で、鉱床データを介さず定義値のまま
   使われることも確認した — `BuildingType::requires_crystal_deposit`的なメソッド
   `matches!(self, BuildingType::CrystalMine)`が建設時ゲートとして存在するのみで、
   産出量そのものには影響しない)。
5. **経済力の安定した指標**: `treasury`/`monthly_balance`/`stockpile`はいずれも
   現在の収支・在庫に依存し「安定した生産能力」ではない(spec自身が除外指定)。
   `process_country_production`が使う`calculate_actual_output(base_output, level,
   operation_ratio)`のうち、`operation_ratio`(稼働率、雇用/物流/入力資源充足率/
   国庫赤字に依存し毎月変動する)を除いた`base_output × level`部分こそが「安定した
   生産能力」に相当すると判断した(詳細は§3)。
6. **国家所有州の集計API**: `StateRegistry::get_owned_states(country_id) ->
   Vec<&StateData>`が既存するが、これは呼び出しごとに`states`全件を`filter`する
   ため、国家ごとに呼ぶと`countries × states`になってしまう。P21-014では使わず、
   `state_registry.states`を1パスだけ走査する専用の集計関数を新設した(§8)。
7. **月次SystemSet順序・`MonthChangedMessage`発行条件**(`src/app/time.rs`):
   `DailySimulationSet`は`TimeUpdate → Economy → Research → Diplomacy → CountryAi →
   WarPreparation → MilitaryAi → FrontlineOrders → MilitaryAction → WarResolution →
   UiUpdate`の11Set・`.chain()`。`advance_game_date`(`TimeUpdate`内)が
   `date.month != old_month`の日に`MonthChangedMessage`を`DayChangedMessage`と
   同一フレームで発行する。
8. **Pause中は月次処理が実行されない保証**: `advance_game_date`は`if paused.0 {
   return; }`を最初に行うため、Pause中は`GameDate`のaccumulatorすら進まず、
   `DayChangedMessage`/`MonthChangedMessage`のどちらも一切発行されない
   (`GamePaused`の既定値は`true`)。
9. **New Game・Load Gameの`Playing`進入経路**(`src/app/game_state.rs`、
   `src/app/loader.rs`、`src/save/runtime.rs`): `OnEnter(GameState::Playing)`は
   `(spawn_debug_divisions, reset_playing_entry_mode).chain()`で、New Gameと
   「起動直後の続きから」(`handle_load_requests`が`CountrySelection`中に
   `PlayingEntryMode::LoadedGame`を設定してから`next_state.set`する経路)の
   **両方**を通る。一方、**既にPlaying中からの「ロード」**
   (`LoadConfirmDialog`経由)は状態遷移を伴わないため`OnEnter(Playing)`を
   **経由しない**ことを確認した(この発見がP21-014の再構築フック設計を決定した、
   §9参照)。
10. **Save適用直後に派生Resourceを再構築する既存パターン**: 調査の結果、
    **存在しなかった**。`save::apply::commit_load`は保存対象Resourceを
    `world.insert_resource`で流し込むだけで、派生データの再構築ロジックは
    どこにも無い。P21-014が今回、`apply_validated_save`内に
    `commit_load`直後の明示的な再構築呼び出しを追加することで、この種の
    派生Resourceを扱う初めての前例となった(§9)。
11. **外交パネルでの選択国追加情報表示箇所**: `DiplomacyPanelState.target_country:
    Option<CountryId>`が既に「現在選択中の(自国以外を含む)国家」を保持しており、
    `update_diplomacy_panel_ui`の見出し(`DiplomacyHeaderText`)がこれを表示に使う。
    ただし自国選択時(`target_cid == p_cid`)は早期returnして子要素を一切
    描画しない実装だったため、この経路に依存せず完全に独立したコンテナ・
    システムを新設する方針にした(§11)。
12. **実6か国28州データの軍事・人口・建物データ有無**: `assets/data/countries.ron`は
    **6か国**(Kingdom of Arcadia / Elfin Republic / Dwarf Federation / Oceanic Magic
    Empire / Passguard March / Ferrowyn League)であり、仕様書が前提としていた
    「7か国」ではなかった(§20で詳述)。`assets/data/states.ron`は28州すべてに
    `buildings:`(Farm/Mine/Factory/MilitaryFactory/MagicAcademy/University等の
    組み合わせ)を持つことを確認した。
13. **2000州プロファイラの国家・州・軍事データ生成**(`src/profiling.rs`):
    `country_count_for(state_count) = (state_count / 20).max(8)`。各州へ
    `Farm`(1〜3レベル)・`Mine`(1〜2レベル)・1件の`resource_deposits`
    (discovered=true固定、`ResourceType::ALL`を巡回、CrystalMineを含まない
    タイプも含む)を機械的に生成しており、経済力評価のFarm/Mine両経路を
    性能計測で実際に演習できることを確認した。
14. **国家総合力・国家ランク・大国フラグに相当する既存の型・死コード**:
    `GreatPower`/`RegionalPower`/`MinorPower`/`CountryPower`/`PowerTier`/
    `NationalPower`/`world_rank`等でリポジトリ全体を検索したが**1件もヒットしなかった**。
    完全新規実装で問題ない。
15. **`CountryId`の順序付け・HashMap反復順の決定論**: `CountryId(pub usize)`は
    `Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize`のみ派生しており
    `Ord`/`PartialOrd`は無い。順位の同点判定では`CountryId`自体ではなく内部の`.0:
    usize`を直接`cmp`することで昇順比較を行い、`CountryId`型自体への変更は
    一切行わなかった。

既存の国家ランク機能は存在しなかったため、新規実装に進んだ。

---

## 2. 軍事力へ再利用した既存関数

`src/country/country_ai.rs`の`compute_total_power_by_country`(P20-008で既に存在した
全国家一括集計API)をそのまま再利用した。可視性を`fn`(private)から`pub(super)`へ
最小限変更しただけで、**関数の中身は1文字も変更していない**(既存の
`test_compute_total_power_by_country_matches_individual_calculation`がそのまま
無改造で通り続けることでこれを保証している — 要求テスト6はこの既存テストで
既に満たされている)。

`evaluate_country_power`はこの`Vec<u64>`を`CountryId.0`でインデックス参照するだけで、
`countries × military`の総当たりを一切発生させない(§8)。

---

## 3. 経済力の正確な計算方法

「安定した生産能力」= 建物レベル × 建物定義の基礎産出量(`operation_ratio`・
`treasury`・`stockpile`には一切依存しない)。`src/country/power.rs`の
`compute_state_aggregates`が`state_registry.states`を1パスしながら計算する。

建物種別ごとの扱い(優先順位2「所有州に存在する生産建物の能力合計」を基本とし、
`Mine`のみ優先順位3「建物定義に生産能力が無い場合の代替」に該当):

- `output_resources`が非空の建物(Farm/LoggingCamp/Factory/MilitaryFactory/
  CrystalMine/CrystalRefinery): `Σ(output_resources.values()) × level`
- `Mine`(建物定義自体は`output_resources`が空): 州の`resource_deposits`のうち
  `discovered == true`かつ`resource_type != MagicCrystal`のものの`base_output`合計
  × Mineのlevel(`economy::production`のMine特別扱いと同じ規約 — MagicCrystalは
  専用のCrystalMineの管轄のため除外)
- 上記いずれにも該当しない建物(Railway/University/MagicAcademy: 出力が
  物流ボーナス・研究力・魔法力であり経済生産物ではない): 0(除外)

異なる資源タイプ(Food/Wood/IndustrialGoods等)の産出量をそのまま単純加算している。
本プロジェクトには価格・GDP変換の仕組みが存在しない(意図的に対象外)ため、
新しい経済重み付け体系を発明せず、既存データをそのまま使う最小限の設計とした
(要求テスト10/11「国庫残高・Stockpileだけを変えても経済力が変わらない」で、
これらが一切参照されないことを直接確認済み)。

---

## 4. 人口の正確な集計方法

`state_registry.states`を1パスする際、`state.owner_country_id`(**controllerではなく
owner**)ごとに`population: u64`を合計する。占領中の州(`controller_country`が
`owner_country_id`と異なる)でも、人口は法的所有国側へ計上される
(要求テスト15で直接確認)。存在しないownerを指す州(参照不整合)は、`HashMap`への
集計自体は行うが、最終的な組み立てが`country_registry.countries`だけを走査するため
自然に無視され、幽霊の評価エントリを作らない(要求テスト: 追加の
`dangling_owner_reference_does_not_create_phantom_assessment_or_panic`)。

---

## 5. 正規化と総合力計算式

```
sanitize_raw(x) = x.is_finite() ? max(x, 0.0) : 0.0   // NaN・無限大・負値を排除
normalized(raw, world_max) = world_max > 0.0 ? clamp(raw / world_max * 100.0, 0, 100) : 0.0
total_score = clamp(
    military_normalized * 0.50 + economic_normalized * 0.30 + population_normalized * 0.20,
    0.0, 100.0
)
```

`world_max`は各要素(軍事力raw/経済力raw/人口raw)ごとに、その時点の全評価対象国家
(`country_registry.countries`)内での最大値。世界最大国はその要素について必ず
ちょうど100になる。全国家がその要素で0なら、`world_max <= 0.0`となり全員0.0
(0除算を回避)。`sanitize_raw`により、集計元データが万一NaN/無限大を含んでいても
総合力へは絶対に伝播しない。

---

## 6. 国家ランク人数計算

浮動小数点の丸め誤差を避けるため、`f64`のceilではなく`usize::div_ceil`による
整数演算で実装した:

```rust
fn compute_tier_counts(n: usize) -> (usize, usize) {
    if n == 0 { return (0, 0); }
    let great = (n * 20).div_ceil(100).clamp(1, 8).min(n);
    let remaining = n - great;
    let regional = (n * 30).div_ceil(100).min(remaining);
    (great, regional)
}
```

仕様書記載の例示表(n=1,2,7,10,50,100)すべてに対し、この式が完全一致することを
単体テスト`tier_counts_match_the_specified_table`で確認済み。

---

## 7. 同点時の順位決定

`total_score`降順 → `military_normalized`降順 → `economic_normalized`降順 →
`population_normalized`降順 → `CountryId.0`昇順、の順で`Vec::sort_by`する。
浮動小数点比較には`f32::total_cmp`を使用した(通常の`partial_cmp().unwrap()`は
NaN入力でpanicし得るが、`total_cmp`はNaNを含む場合でも常に決定論的な全順序を
返すため、`sanitize_raw`によりこの時点でNaNが存在し得ないことを踏まえてもなお
防御的にpanicの可能性そのものを排除できる)。`CountryId`自体は`Ord`を実装していない
ため、`.0: usize`を直接比較する。

---

## 8. 計算量と全走査回避方法

```
O(countries + states + military entities + countries log countries)
```

- 軍事力: `country_ai::compute_total_power_by_country`(既存のP20-008最適化API)を
  そのまま再利用。`military_registry.divisions`を1パスするだけで、`countries ×
  military`の総当たりは発生しない。
- 経済力・人口: 新設の`compute_state_aggregates`が`state_registry.states`を
  1パスするだけ(`countries × states`にならない — 既存の`get_owned_states`
  [国家ごとに全州filter]は意図的に使わなかった、§1事前調査項目6参照)。
- 建物定義の参照は`BuildingRegistry::get`(既存のO(1) HashMap引き)。
- 最後に国家数分だけ正規化・ソート(`countries log countries`)。

実測(§16)でも、月が変わらない日の`UiUpdate`Set所要時間が2000州(国家数100)まで
ほぼ一定(0.0033〜0.0036ms)であることを確認しており、`countries × states`/
`countries × military`のような二次的スケーリングが存在しないことを裏付けている。

---

## 9. New Game・月次・Load Gameへの接続

3箇所のトリガーがすべて同一の純粋関数`evaluate_country_power`を呼ぶ(ロジックの
重複は無い):

1. **New Game**: `app::loader::DataLoaderPlugin`の`OnEnter(GameState::Playing)`
   チェーンへ、`spawn_debug_divisions`の直後・`reset_playing_entry_mode`の直前に
   `country::power::rebuild_country_power_on_enter_playing`を追加した
   (デバッグ初期師団配置後の軍事力を初回評価に含めるため)。この経路は
   「起動直後の続きから」ロード(`PlayingEntryMode::LoadedGame`)も通るが、
   トリガー2により既に再構築済みのため冪等な二重評価に留まる(実害なし)。
2. **Load Game(apply成功直後)**: `save::apply::apply_validated_save`内、
   `commit_load`成功直後に`country::power::rebuild_country_power_registry_from_world`
   を直接呼ぶ。事前調査項目9で判明した「既にPlaying中からのロードは
   `OnEnter(Playing)`を経由しない」という抜けを塞ぐため、`apply_validated_save`
   という**すべてのロード経路が必ず通る唯一の公開API**へ直接埋め込んだ
   (これにより新Save・旧Save・New Game中の再ロードいずれでも取りこぼしなく動作する)。
3. **月次進行**: `country::power::rebuild_country_power_monthly`を
   `DailySimulationSet::UiUpdate`(11Set中最後)に登録した。`MessageReader<
   MonthChangedMessage>`が空ならO(1)で即returnする。`UiUpdate`を選んだ理由は、
   同一フレーム内のEconomy/Diplomacy/CountryAi/War各Setでのその日の変更を
   必ず反映した状態で評価するため。

---

## 10. Save形式変更の有無

**変更なし。** `src/save/dto.rs`・`src/save/export.rs`・`src/save/validate.rs`は
P21-014で一切変更していない(`git diff`で確認済み — これらのファイルに残る差分は
すべてP21-013など先行タスクによるもの)。国家総合力・国家ランクはCountry/State/
Military/Buildingから毎回再構築する派生データとして扱い、Save DTOへの
フィールド追加は一切行わなかった。バージョン番号の変更も不要と判断した。

---

## 11. UI表示とJA/EN対応

`src/ui/diplomacy_panel.rs`に、既存の`DiplomacyContentContainer`(P21-013の
支持UI・クライシス一覧を含み、`update_diplomacy_panel_ui`が毎回全破棄・再構築する)
とは**完全に独立**した新規`CountryPowerInfoContainer`を追加し、専用の新規System
`update_country_power_info_ui`だけが管理する。配置は`DiplomacyHeaderText`の直後・
`DiplomacyContentContainer`の直前(=国家基本情報に近く、P21-013支持UIより上)。

表示内容: プレイヤー自国は常に表示し、`DiplomacyPanelState.target_country`が
自国と異なる外国を指していれば、その国の評価も追加で表示する(見出しに国名を
含めるため、内部`CountryId`だけの表示にはならない)。各ブロックは
国家ランク・世界順位/国家数・国家総合力・軍事力/経済力/人口の6行。

安全策: `format_power_value`がNaN/無限大を検出したら`"-"`を表示し(通常発生し得ないが
UI層でも二重に防御)、負値は0へclamp、小数は`{:.1}`固定(1桁)。
`CountryPowerRegistry`にエントリが無い国家(評価未構築)は「評価中...」表示に
フォールバックし、存在しない`CountryId`は静かに何も描画しない(panicしない)。
JA/EN両方に対応するローカライズキーを追加し、`CurrentLocale`変更時に即時反映される
ことをテストで確認済み。ランクは`power_tier.*`キーによる文字表示であり、色だけに
依存していない。

---

## 12. 変更ファイル一覧

**P21-014で新規に変更/追加したファイル:**
- `src/country/power.rs`(新規、評価ロジック本体+3つのBevy Systemフック+26単体テスト)
- `src/country/country_ai.rs`(`compute_total_power_by_country`の可視性変更のみ)
- `src/country/mod.rs`(`CountryPowerRegistry`登録・月次System配線)
- `src/app/loader.rs`(`OnEnter(Playing)`チェーンへNew Game用フック追加)
- `src/save/apply.rs`(`apply_validated_save`内にLoad用フック追加)
- `src/ui/diplomacy_panel.rs`(国家総合力UI追加、+7テスト)
- `assets/localization/ja-JP.ron` / `en-US.ron`(各13キー追加)
- `tests/p21_014_country_power_e2e_test.rs`(新規、9 E2Eテスト)

P21-013までの成果物(`crisis_response.rs`、`diplomacy_panel.rs`のP21-013部分、
save/dto.rs等)には一切触れていない。

---

## 13. 新規・更新テスト内訳

- `src/country/power.rs`: 26テスト(軍事力6、経済力7、人口5、正規化・総合力7、
  ランク5相当[27-32を1つのテーブル駆動テストへ統合]、エラー処理1 — 数え方により
  重複するがテスト関数としては26個)
- `src/ui/diplomacy_panel.rs`: 7テスト(own_country表示、選択外国表示、評価未構築
  フォールバック、JA/EN切替、`format_power_value`の小数1桁/NaN・inf非表示、
  スクロール維持、P21-013支持ボタン維持)
- `tests/p21_014_country_power_e2e_test.rs`: 9テスト(軽量App側4: 月変化なし
  再評価なし/月変化で再評価・順位反映/Pause中再評価なし/同一月重複再評価なし、
  実データ側5: 全国家評価/実6か国のランク人数/決定論性+save往復/月次再評価/
  ロードでの再構築)

合計: **42テスト新規追加**(既存テストの削除・弱体化は無し)。

---

## 14. 開始時と終了時のテスト数

| | lib | integration | 合計 |
|---|---:|---:|---:|
| 開始時(実測、P21-013修正後基準どおり) | 707 | 153 | 860 |
| 終了時(実測) | 740 | 162 | 902 |
| 差分 | +33 | +9 | +42 |

開始時の実測値は仕様書記載の基準(lib 707・全体860)と完全に一致しており、
差異は無かった。終了時の差分(+42)は§13の内訳と完全に一致している
(P21-013の完了報告で報告した「+50 vs 実測+51」のような1件差は、今回は発生していない)。

---

## 15. 品質ゲート結果

- `cargo check --all-targets`: 成功
- `cargo test --lib`: 740 passed, 0 failed
- `cargo test --tests`(デフォルト並列、headless GPU含む全バイナリ): 162 passed,
  0 failed
- `cargo clippy --all-targets --all-features -- -D warnings`: 初回2件検出・修正
  (`src/country/power.rs`の`(n*20).div_ceil(100).max(1).min(8)`パターンを
  `clippy::manual_clamp`が指摘 → `.clamp(1,8)`へ書き換え、`src/ui/
  diplomacy_panel.rs`の`update_country_power_info_ui`が8引数で
  `clippy::too_many_arguments`に抵触 → 同ファイル内の既存関数と同じ
  `#[allow(clippy::too_many_arguments)]`を付与)。修正後は**0警告**。
- `cargo build --release`: 成功
- `git diff --check`: 実質エラー0件(既存のLF→CRLF警告のみ)
- headless render系テストが書き換えたスクリーンショットは`git checkout --`で
  都度復元済み。

---

## 16. 性能測定

一時バイナリ`src/bin/profile_country_power.rs`(計測後に削除・`Cargo.toml`の
登録も削除済み)で、`state_count ∈ {100, 500, 1000, 2000}`固定、開始日
(1800/01/01)から35日分を1日ずつ計測し、31日目(1800/01/31→02/01、月次境界)と
それ以外の日を分けて`UiUpdate`Set(`rebuild_country_power_monthly`が登録されている
Set)所要時間を比較した。

生データ: `verification_logs/phase-21/p21-014/perf/{summary.txt,results.csv}`

| 州数 | 国家数 | 建物(生産)数 | UiUpdate非月変化日平均 | UiUpdate月変化日 | overall非月変化日平均 | overall月変化日 |
|---:|---:|---:|---:|---:|---:|---:|
| 100 | 8 | 200 | 0.00334ms | 0.02080ms | 0.11801ms | 0.23660ms |
| 500 | 25 | 1000 | 0.00361ms | 0.03600ms | 0.12566ms | 0.31160ms |
| 1000 | 50 | 2000 | 0.00350ms | 0.07030ms | 0.17093ms | 0.77420ms |
| 2000 | 100 | 4000 | 0.00348ms | 0.24330ms | 0.25944ms | 1.20440ms |

観測:
- 月が変わらない日の`UiUpdate`所要時間は2000州(国家数100)まで**ほぼ一定**
  (0.0033〜0.0036ms)であり、`MessageReader`が空の日は評価コストが実質ゼロという
  設計どおりの結果。**日次通常tickの性能へ回帰を追加していない**ことを裏付ける。
- 月が変わる日の評価コストは州数・国家数の増加とともに増えるが、100→2000州
  (12.5倍)で0.021ms→0.243ms(約11.6倍)と、ほぼ線形でありそれ以上には悪化して
  いない。`countries × states`/`countries × military`のような二次的スケーリングは
  見られない。
- 各スケールで`assessment_count == country_count`、NaN件数0、順位重複0を
  プロファイラ自身がassertし、全スケールで成功した(正しさの検証も兼ねる)。

---

## 17. fmt比較

開始時点(P21-013修正後基準): `cargo fmt --all -- --check`は58箇所の
`Diff in`ハンク(P21-014と無関係な既存drift、24ファイル程度)。

P21-014で変更した7つのRustファイル(`power.rs`は新規のため対象外の概念、
`country_ai.rs`/`country/mod.rs`/`app/loader.rs`/`save/apply.rs`/
`ui/diplomacy_panel.rs`/`tests/p21_014_country_power_e2e_test.rs`)に対して
個別に`rustfmt --edition 2024`を実行した。

終了時点: `cargo fmt --all -- --check`は**51箇所**の`Diff in`ハンクに減少した。
これは`src/app/loader.rs`がP21-014の変更対象ファイルであり、同ファイルへの
個別`rustfmt`実行がファイル全体を対象とするため、以前から残っていた
無関係な整形崩れ(7ハンク相当)も同時に解消されたことによる
(意図的な「リポジトリ全体の一括fmt修正」ではなく、変更ファイル単位の
`rustfmt`実行の自然な副作用。他の21ファイル分の既存driftには一切触れていない)。
残存する51ハンクはすべてP21-014と無関係な既存ファイル
(`division_render.rs`/`map/mod.rs`/`movement.rs`/`recruitment.rs`/`supply.rs`/
`military/tests.rs`/`profiling.rs`/`save/runtime.rs`/`capitulation.rs`/
`military_ai.rs`/`peace.rs`/`daily_system_integration_test.rs`/
`land_war_combat_peace_test.rs`[保護対象]/`p21_save_003_end_to_end_test.rs`/
`profile_workload_correctness_test.rs`)であり、P21-014の変更ファイルは
1件もこの一覧に含まれない。

---

## 18. verification_logsディレクトリの差分

新規追加: `verification_logs/phase-21/p21-014/p21-014_completion_report.md`
(本ファイル)、`verification_logs/phase-21/p21-014/perf/{summary.txt,results.csv}`。
既存のP20-007/P20-009/P21-save-002eスクリーンショットはheadless renderテストにより
一時的に書き換わったが、`git checkout --`で都度復元済み(§15)。既存の証拠ファイルは
一切削除・上書きしていない。

---

## 19. 実GUI確認の実施有無

**手動GUI確認は実施していない。** 対話的なゲーム画面での国家ランク表示・世界順位・
JA/EN切替・スクロール・P21-013支持UIとの共存の目視確認は、このセッションでは
一度も行っていない。要求されている12項目のGUI手動確認チェックリストはいずれも
未実施であり、必要であればユーザー自身または別セッションでの手動確認を推奨する:

1. ゲーム開始直後にプレイヤー国家のランクが表示される
2. 外国を選択すると、その国の評価へ切り替わる
3. 実7か国(実際は6か国)で大国2・地域大国2・小国2になっている
   (仕様書は7か国前提の「2/3/2」だったが、実データは6か国であり正しい期待値は
   「2/2/2」— §20参照)
4. 世界順位が重複していない
5. 総合力・軍事力・経済力・人口が有限値で表示される
6. 表示が小数1桁
7. JA/EN切り替えで即時更新
8. 外交パネルのスクロールが機能する
9. P21-013の支持者一覧・支持ボタンが維持されている
10. 月を進めた後も表示が正常
11. セーブ→ロード直後に評価が表示される
12. Pause中に評価が不自然に変化しない

---

## 20. 発見した既存不具合・仕様上の曖昧さ

- **仕様書の「実7か国」前提が実データ(6か国)と一致しない**: `assets/data/
  countries.ron`は6か国(Kingdom of Arcadia / Elfin Republic / Dwarf Federation /
  Oceanic Magic Empire / Passguard March / Ferrowyn League)であり、7か国目は
  存在しない。これはP21-014のバグではなく、既存の`P21-MAP-001`(6か国28州)を
  正しく前提とすべきだった仕様書側の記述ズレと考えられる。仕様書の指示
  「開始時に必ず実測してください。数字が異なる場合は推測で合わせず、差異と原因を
  記録してください」に従い、実測値(6か国、`compute_tier_counts(6) == (2, 2)`、
  結果として大国2・地域大国2・小国2)をそのままテスト・GUIチェックリストへ反映し、
  「7か国」に合わせて数字を作り変えることはしなかった。
- **`get_owned_states`/`get_controlled_states`は個別呼び出し前提のAPI**:
  既存のこの2関数は「国家ごとに全州filter」という設計であり、P21-014のような
  「全国家分を1回でまとめて欲しい」用途には向かない(呼べば`countries × states`に
  なる)。今回はこれらを使わず専用の1パス集計関数を新設したが、将来的に他の
  タスクでも同種の「全国家一括集計」が必要になった場合、`compute_state_aggregates`
  のようなVec/HashMap一括版を都度個別実装するのではなく、共通ヘルパーとして
  切り出すことを検討する価値がある(今回は最小スコープを優先し、切り出しは
  行わなかった)。
- **`Mine`建物定義に産出量情報が無いこと自体は、経済力評価にとって意図的な
  仕様のはず**だが、コード上どこにもコメントで明記されていなかった
  (`economy::production`側のコメントで「鉱床データ経由」と分かる程度)。
  今回`power.rs`のdocコメントで明示的に記録した。

---

## 21. P21-015から利用する公開API

- `strategy_game::country::power::evaluate_country_power(country_registry,
  state_registry, military_registry, building_registry, evaluated_date: String) ->
  CountryPowerRegistry` — 純粋関数、任意の状態から再評価したい場合に直接呼べる。
- `strategy_game::country::power::CountryPowerRegistry`(`Resource`):
  - `.get(CountryId) -> Option<&CountryPowerAssessment>`(O(1))
  - `.ordered_country_ids() -> &[CountryId]`(世界順位順、1位から)
  - `.country_count() -> usize`
  - `.last_evaluated_date() -> Option<&str>`
- `strategy_game::country::power::CountryPowerAssessment`(`Copy`構造体):
  `country_id`/`military_raw`/`economic_raw`/`population_raw`/
  `military_normalized`/`economic_normalized`/`population_normalized`/
  `total_score`/`world_rank`/`power_tier`のすべてのフィールドが`pub`。
- `strategy_game::country::power::PowerTier`(`GreatPower`/`RegionalPower`/
  `MinorPower`、`Copy`・`PartialEq`・`Eq`): P21-015がCrisis支持資格制限の判定に
  `assessment.power_tier == PowerTier::GreatPower`のような形でそのまま使える。

P21-015でAIによる大国支持・国家ランク依存の判断を実装する際は、
`CountryPowerRegistry`を`Res<>`で読み取るだけでよく、新たな評価トリガーを追加する
必要はない(月次更新で十分な鮮度が保たれる設計になっている)。

---

*本レポートはP21-014仕様書の要求に基づき、実測値をそのまま記載した。§20記載の
「実7か国 vs 実6か国」の食い違いは推測で埋め合わせず、判明している事実のみを
記載している。*
