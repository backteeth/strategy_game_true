//! P21-009: マジッククリスタル資源チェーン(採掘→精製→既存消費先)の統合テスト。
//!
//! 既存の`MagicCrystal`(精製済み、`MagicAcademy`が消費)はそのまま維持し、新規
//! `ResourceType::RawMagicCrystal`(原料)・`BuildingType::CrystalMine`(鉱床ゲート付き
//! 採掘施設)・`BuildingType::CrystalRefinery`(精製施設)を、既存の
//! `economy::production::process_country_production`(月次生産、Step1入力集計→
//! Step2充足率スナップショット→Step3消費・生産)へ最小接続したことを検証する。
//!
//! 前半(pure function群)は`tests/economy_tests.rs`と同じ手法で`StateData`/
//! `CountryStockpile`/`BuildingRegistry`を直接構築し、Bevy Appを起動せずに
//! `process_country_production`を直接呼び出す。後半は実データ(`assets/data/*.ron`、
//! 7か国28州の本番マップ)を`AppPlugin`経由で読み込み、`StateId(2)`
//! ("Western Mage Province"、既存のMagicCrystal鉱床州)で実際に建設→日次進行→
//! 月次生産までを一気通貫させる。

use std::collections::HashMap;

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::economy::production::process_country_production;
use strategy_game::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use strategy_game::state::data::{StateData, StateRegistry};

// ─────────────────────────────────────────────────────────────────────────
// 共通フィクスチャ
// ─────────────────────────────────────────────────────────────────────────

const EPSILON: f64 = 1e-6;

fn building_def(
    building_type: BuildingType,
    input: &[(ResourceType, f64)],
    output: &[(ResourceType, f64)],
) -> BuildingDefinition {
    building_def_with_magic(building_type, input, output, 0.0)
}

fn building_def_with_magic(
    building_type: BuildingType,
    input: &[(ResourceType, f64)],
    output: &[(ResourceType, f64)],
    magic_output: f64,
) -> BuildingDefinition {
    BuildingDefinition {
        building_type,
        name: format!("{building_type:?}"),
        construction_cost: 100.0,
        required_progress: 10.0,
        required_workforce: 10.0,
        logistics_cost: 0.0,
        input_resources: input.iter().copied().collect(),
        output_resources: output.iter().copied().collect(),
        maintenance_cost: 1.0,
        max_level: 10,
        science_output: 0.0,
        magic_output,
        railway_capacity_bonus: 0.0,
    }
}

/// シナリオ専用の定数(本番`buildings.ron`の値とは独立、汎用メカニズムの検証が目的)。
const CRYSTAL_MINE_OUTPUT: f64 = 40.0;
const CRYSTAL_REFINERY_INPUT: f64 = 24.0;
const CRYSTAL_REFINERY_OUTPUT: f64 = 16.0;
const MAGIC_ACADEMY_INPUT: f64 = 8.0;
const MAGIC_ACADEMY_MAGIC_OUTPUT: f64 = 12.0;

fn test_registry() -> BuildingRegistry {
    let mut definitions = HashMap::new();
    definitions.insert(
        BuildingType::Mine,
        building_def(BuildingType::Mine, &[], &[]),
    );
    definitions.insert(
        BuildingType::CrystalMine,
        building_def(
            BuildingType::CrystalMine,
            &[],
            &[(ResourceType::RawMagicCrystal, CRYSTAL_MINE_OUTPUT)],
        ),
    );
    definitions.insert(
        BuildingType::CrystalRefinery,
        building_def(
            BuildingType::CrystalRefinery,
            &[(ResourceType::RawMagicCrystal, CRYSTAL_REFINERY_INPUT)],
            &[(ResourceType::MagicCrystal, CRYSTAL_REFINERY_OUTPUT)],
        ),
    );
    definitions.insert(
        BuildingType::MagicAcademy,
        building_def_with_magic(
            BuildingType::MagicAcademy,
            &[(ResourceType::MagicCrystal, MAGIC_ACADEMY_INPUT)],
            &[],
            MAGIC_ACADEMY_MAGIC_OUTPUT,
        ),
    );
    BuildingRegistry { definitions }
}

/// 労働力比率・物流とも1.0(フル稼働可能)な最小Stateフィクスチャ。
fn full_capacity_state(
    id: StateId,
    owner: CountryId,
    buildings: &[(BuildingType, u32)],
) -> StateData {
    StateData {
        id,
        owner_country_id: owner,
        population: 100_000,
        workforce_ratio: 0.5,
        employed_workforce: 50_000,
        logistics_ratio: 1.0,
        buildings: buildings.iter().copied().collect(),
        resource_deposits: vec![StateResourceDeposit {
            resource_type: ResourceType::MagicCrystal,
            base_output: 30.0,
            discovered: true,
            development_level: 1,
        }],
        ..Default::default()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 1. 採掘: 鉱床あり＋稼働で原料生産 / 建設中・非稼働は生産しない
// ─────────────────────────────────────────────────────────────────────────

/// 要求テスト項目1: 鉱床のある州でCrystalMineが稼働すると原料が生産される。
#[test]
fn crystal_mine_produces_raw_material_when_operational() {
    let registry = test_registry();
    let mut state =
        full_capacity_state(StateId(0), CountryId(0), &[(BuildingType::CrystalMine, 1)]);
    let mut stockpile = CountryStockpile::new();
    let mut states: Vec<&mut StateData> = vec![&mut state];

    process_country_production(&mut states, &mut stockpile, &registry, false);

    assert!(
        (stockpile.get(ResourceType::RawMagicCrystal) - CRYSTAL_MINE_OUTPUT).abs() < EPSILON,
        "expected {CRYSTAL_MINE_OUTPUT} raw crystal, got {}",
        stockpile.get(ResourceType::RawMagicCrystal)
    );
}

/// 要求テスト項目3: 建設中(state.buildingsに未登録)の施設は生産しない。
#[test]
fn building_not_yet_registered_in_state_does_not_produce() {
    let registry = test_registry();
    // CrystalMineをまだ`buildings`へ登録しない = 建設キュー中(未完成)を模す。
    let mut state = full_capacity_state(StateId(0), CountryId(0), &[]);
    let mut stockpile = CountryStockpile::new();
    let mut states: Vec<&mut StateData> = vec![&mut state];

    process_country_production(&mut states, &mut stockpile, &registry, false);

    assert_eq!(stockpile.get(ResourceType::RawMagicCrystal), 0.0);
}

/// 要求テスト項目4: 稼働率0(就業者0)の施設は生産しない。
#[test]
fn zero_operation_ratio_building_does_not_produce() {
    let registry = test_registry();
    let mut state =
        full_capacity_state(StateId(0), CountryId(0), &[(BuildingType::CrystalMine, 1)]);
    state.employed_workforce = 0; // wf_ratio = 0 → operation_ratio = 0
    let mut stockpile = CountryStockpile::new();
    let mut states: Vec<&mut StateData> = vec![&mut state];

    process_country_production(&mut states, &mut stockpile, &registry, false);

    assert_eq!(stockpile.get(ResourceType::RawMagicCrystal), 0.0);
}

// ─────────────────────────────────────────────────────────────────────────
// 2. 精製: 消費して生産 / 部分稼働 / 原料ゼロ
// ─────────────────────────────────────────────────────────────────────────

/// 要求テスト項目5: 精製施設が原料を消費して精製済みを生産する。
#[test]
fn refinery_consumes_raw_and_produces_refined_when_input_sufficient() {
    let registry = test_registry();
    let mut state = full_capacity_state(
        StateId(0),
        CountryId(0),
        &[(BuildingType::CrystalRefinery, 1)],
    );
    let mut stockpile = CountryStockpile::new();
    stockpile.set(ResourceType::RawMagicCrystal, 100.0);
    let mut states: Vec<&mut StateData> = vec![&mut state];

    process_country_production(&mut states, &mut stockpile, &registry, false);

    assert!(
        (stockpile.get(ResourceType::RawMagicCrystal) - (100.0 - CRYSTAL_REFINERY_INPUT)).abs()
            < EPSILON
    );
    assert!((stockpile.get(ResourceType::MagicCrystal) - CRYSTAL_REFINERY_OUTPUT).abs() < EPSILON);
}

/// 要求テスト項目6: 原料不足時は可能な量だけ部分稼働する(消費・生産とも按分)。
#[test]
fn refinery_partially_operates_when_raw_material_is_short() {
    let registry = test_registry();
    let mut state = full_capacity_state(
        StateId(0),
        CountryId(0),
        &[(BuildingType::CrystalRefinery, 1)],
    );
    let mut stockpile = CountryStockpile::new();
    // 必要量24に対し在庫12 → 充足率0.5
    stockpile.set(ResourceType::RawMagicCrystal, 12.0);
    let mut states: Vec<&mut StateData> = vec![&mut state];

    process_country_production(&mut states, &mut stockpile, &registry, false);

    assert!((stockpile.get(ResourceType::RawMagicCrystal) - 0.0).abs() < EPSILON);
    assert!(
        (stockpile.get(ResourceType::MagicCrystal) - CRYSTAL_REFINERY_OUTPUT * 0.5).abs() < EPSILON
    );
}

/// 要求テスト項目7: 原料在庫0では精製量も0になる。
#[test]
fn refinery_produces_zero_when_raw_material_stock_is_zero() {
    let registry = test_registry();
    let mut state = full_capacity_state(
        StateId(0),
        CountryId(0),
        &[(BuildingType::CrystalRefinery, 1)],
    );
    let mut stockpile = CountryStockpile::new();
    let mut states: Vec<&mut StateData> = vec![&mut state];

    process_country_production(&mut states, &mut stockpile, &registry, false);

    assert_eq!(stockpile.get(ResourceType::RawMagicCrystal), 0.0);
    assert_eq!(stockpile.get(ResourceType::MagicCrystal), 0.0);
}

// ─────────────────────────────────────────────────────────────────────────
// 3. 在庫不変条件
// ─────────────────────────────────────────────────────────────────────────

/// 要求テスト項目8: consume/add/setいずれも在庫を負数にしない。
#[test]
fn stockpile_never_goes_negative() {
    let mut stockpile = CountryStockpile::new();

    // consumeは不足時にfalseを返し、在庫は変化しない。
    let ok = stockpile.consume(ResourceType::RawMagicCrystal, 50.0);
    assert!(!ok);
    assert_eq!(stockpile.get(ResourceType::RawMagicCrystal), 0.0);

    // addへ負数を渡しても在庫はmax(0.0)でクランプされる。
    stockpile.set(ResourceType::MagicCrystal, 10.0);
    stockpile.add(ResourceType::MagicCrystal, -100.0);
    assert_eq!(stockpile.get(ResourceType::MagicCrystal), 0.0);

    // setへ負数を渡してもクランプされる。
    stockpile.set(ResourceType::RawMagicCrystal, -5.0);
    assert_eq!(stockpile.get(ResourceType::RawMagicCrystal), 0.0);
}

/// 要求テスト項目9: 複数精製施設が存在しても総消費量が在庫を超えない。
#[test]
fn multiple_refineries_total_consumption_never_exceeds_stockpile() {
    let registry = test_registry();
    let mut state_a = full_capacity_state(
        StateId(0),
        CountryId(0),
        &[(BuildingType::CrystalRefinery, 1)],
    );
    let mut state_b = full_capacity_state(
        StateId(1),
        CountryId(0),
        &[(BuildingType::CrystalRefinery, 1)],
    );
    let mut stockpile = CountryStockpile::new();
    // 需要合計48(24×2)に対し在庫30 → 充足率0.625、按分後の消費合計は在庫を超えない。
    stockpile.set(ResourceType::RawMagicCrystal, 30.0);
    let mut states: Vec<&mut StateData> = vec![&mut state_a, &mut state_b];

    process_country_production(&mut states, &mut stockpile, &registry, false);

    let remaining = stockpile.get(ResourceType::RawMagicCrystal);
    assert!(
        remaining >= -EPSILON,
        "remaining raw crystal must not go negative, got {remaining}"
    );
    let total_produced = stockpile.get(ResourceType::MagicCrystal);
    assert!(
        total_produced <= CRYSTAL_REFINERY_OUTPUT * 2.0 + EPSILON,
        "total refined output must not exceed the fully-supplied maximum"
    );
}

/// 要求テスト項目10: 同一入力状態(HashMap挿入順違い含む)からは常に同じ日次結果になる。
#[test]
fn production_result_is_deterministic_regardless_of_building_insertion_order() {
    let registry = test_registry();

    let mut state_a = full_capacity_state(StateId(0), CountryId(0), &[]);
    state_a.buildings.insert(BuildingType::CrystalMine, 1);
    state_a.buildings.insert(BuildingType::CrystalRefinery, 1);
    let mut stockpile_a = CountryStockpile::new();
    stockpile_a.set(ResourceType::RawMagicCrystal, 50.0);

    let mut state_b = full_capacity_state(StateId(0), CountryId(0), &[]);
    state_b.buildings.insert(BuildingType::CrystalRefinery, 1);
    state_b.buildings.insert(BuildingType::CrystalMine, 1);
    let mut stockpile_b = CountryStockpile::new();
    stockpile_b.set(ResourceType::RawMagicCrystal, 50.0);

    let mut states_a: Vec<&mut StateData> = vec![&mut state_a];
    let mut states_b: Vec<&mut StateData> = vec![&mut state_b];
    process_country_production(&mut states_a, &mut stockpile_a, &registry, false);
    process_country_production(&mut states_b, &mut stockpile_b, &registry, false);

    assert_eq!(
        stockpile_a.get(ResourceType::RawMagicCrystal),
        stockpile_b.get(ResourceType::RawMagicCrystal)
    );
    assert_eq!(
        stockpile_a.get(ResourceType::MagicCrystal),
        stockpile_b.get(ResourceType::MagicCrystal)
    );
}

/// 要求テスト項目11: 占領中(controller_country != owner)でも既存規則通り、
/// 生産は法的所有国(owner_country_id)の在庫へ帰属する(経済処理は所有国基準のまま)。
#[test]
fn occupied_state_production_still_attributes_to_legal_owner_per_existing_rule() {
    let registry = test_registry();
    let mut occupied_state =
        full_capacity_state(StateId(0), CountryId(0), &[(BuildingType::CrystalMine, 1)]);
    occupied_state.controller_country = Some(CountryId(1)); // 占領国は別だが所有権はCountryId(0)のまま

    let mut owner_stockpile = CountryStockpile::new();
    // economy::mod::handle_monthly_economyと同じフィルタ(owner_country_id基準)を模す。
    let mut owned_states: Vec<&mut StateData> = vec![&mut occupied_state];
    process_country_production(&mut owned_states, &mut owner_stockpile, &registry, false);

    assert!(
        owner_stockpile.get(ResourceType::RawMagicCrystal) > 0.0,
        "production must still be credited to the legal owner's stockpile per existing rules"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// 4. 既存消費先(MagicAcademy)接続
// ─────────────────────────────────────────────────────────────────────────

/// 要求テスト項目12: 原料(RawMagicCrystal)は既存消費先(MagicAcademy)の代用にならない。
#[test]
fn raw_material_cannot_substitute_for_refined_in_existing_consumer() {
    let registry = test_registry();
    let mut state =
        full_capacity_state(StateId(0), CountryId(0), &[(BuildingType::MagicAcademy, 1)]);
    let mut stockpile = CountryStockpile::new();
    stockpile.set(ResourceType::RawMagicCrystal, 1000.0); // 原料は潤沢だが精製済みは0
    let mut states: Vec<&mut StateData> = vec![&mut state];

    let (_sci, magic) = process_country_production(&mut states, &mut stockpile, &registry, false);

    assert_eq!(
        *state
            .building_operation_ratios
            .get(&BuildingType::MagicAcademy)
            .unwrap_or(&-1.0),
        0.0,
        "MagicAcademy must not operate when only raw material is available"
    );
    assert_eq!(magic, 0.0);
    assert!(
        (stockpile.get(ResourceType::RawMagicCrystal) - 1000.0).abs() < EPSILON,
        "raw material must remain untouched (not consumed as a substitute)"
    );
}

/// 要求テスト項目13: 精製済みクリスタルは既存消費先(MagicAcademy)が消費する
/// (start-of-month snapshot方式のため、同月内で精製→即時消費はされず、翌月の
/// 処理で反映される。既存の月次経済処理の意味論を変更しないことを合わせて確認する)。
#[test]
fn magic_academy_consumes_refined_crystal_produced_by_refinery_in_a_later_cycle() {
    let registry = test_registry();
    let mut state = full_capacity_state(
        StateId(0),
        CountryId(0),
        &[
            (BuildingType::CrystalRefinery, 1),
            (BuildingType::MagicAcademy, 1),
        ],
    );
    let mut stockpile = CountryStockpile::new();
    stockpile.set(ResourceType::RawMagicCrystal, 1000.0);

    // 月1: 精製済み在庫は0からスタートしているため、このパスのMagicAcademy稼働率は0。
    {
        let mut states: Vec<&mut StateData> = vec![&mut state];
        let (_sci, magic_month1) =
            process_country_production(&mut states, &mut stockpile, &registry, false);
        assert_eq!(
            magic_month1, 0.0,
            "month 1: MagicAcademy must not run before any refined crystal exists"
        );
    }
    let refined_after_month1 = stockpile.get(ResourceType::MagicCrystal);
    assert!(
        refined_after_month1 > 0.0,
        "month 1: refinery must have produced refined crystal into the stockpile"
    );

    // 月2: 月1で精製された在庫を基に、今度はMagicAcademyが実際に稼働・消費する。
    {
        let mut states: Vec<&mut StateData> = vec![&mut state];
        let (_sci, magic_month2) =
            process_country_production(&mut states, &mut stockpile, &registry, false);
        assert!(
            magic_month2 > 0.0,
            "month 2: MagicAcademy must consume last month's refined crystal and produce magic output"
        );
    }
}

/// 要求テスト項目14: 精製済み在庫不足時、既存消費先は既存の按分規則で縮小稼働する。
#[test]
fn magic_academy_partially_operates_when_refined_crystal_is_short() {
    let registry = test_registry();
    let mut state =
        full_capacity_state(StateId(0), CountryId(0), &[(BuildingType::MagicAcademy, 1)]);
    let mut stockpile = CountryStockpile::new();
    // 必要量8に対し在庫4 → 充足率0.5
    stockpile.set(ResourceType::MagicCrystal, 4.0);
    let mut states: Vec<&mut StateData> = vec![&mut state];

    process_country_production(&mut states, &mut stockpile, &registry, false);

    let op = *state
        .building_operation_ratios
        .get(&BuildingType::MagicAcademy)
        .unwrap();
    assert!((op - 0.5).abs() < 1e-3);
}

// ─────────────────────────────────────────────────────────────────────────
// 5. セーブ往復(DTOレベル、実RON経由)
// ─────────────────────────────────────────────────────────────────────────

/// 要求テスト項目20: 原料・精製済み在庫がRON往復で維持される。
#[test]
fn country_stockpile_raw_and_refined_round_trip_through_ron() {
    let mut stockpile = CountryStockpile::new();
    stockpile.set(ResourceType::RawMagicCrystal, 123.5);
    stockpile.set(ResourceType::MagicCrystal, 67.25);

    let ron_str = ron::to_string(&stockpile).expect("CountryStockpile must serialize to RON");
    let restored: CountryStockpile =
        ron::from_str(&ron_str).expect("CountryStockpile RON must deserialize back");

    assert_eq!(restored.get(ResourceType::RawMagicCrystal), 123.5);
    assert_eq!(restored.get(ResourceType::MagicCrystal), 67.25);
}

/// 要求テスト項目21: CrystalMine/CrystalRefineryの建設状態がRON往復で維持される。
#[test]
fn state_crystal_buildings_round_trip_through_ron() {
    let state = full_capacity_state(
        StateId(4),
        CountryId(2),
        &[
            (BuildingType::CrystalMine, 3),
            (BuildingType::CrystalRefinery, 2),
        ],
    );

    let ron_str = ron::to_string(&state).expect("StateData must serialize to RON");
    let restored: StateData = ron::from_str(&ron_str).expect("StateData RON must deserialize back");

    assert_eq!(restored.building_level(BuildingType::CrystalMine), 3);
    assert_eq!(restored.building_level(BuildingType::CrystalRefinery), 2);
    assert!(
        restored
            .resource_deposits
            .iter()
            .any(|d| d.resource_type == ResourceType::MagicCrystal && d.discovered)
    );
}

/// 要求テスト項目22: 新資源・新施設への言及が一切ない旧形式RONは、安全な既定値
/// (在庫0・建物レベル0)として読み込まれる(`HashMap`に未登場キーがあるだけの構造のため、
/// `#[serde(default)]`を新規追加せずとも既存の`unwrap_or`ベースのアクセサで安全に扱える)。
#[test]
fn old_save_ron_without_new_keys_loads_with_safe_defaults() {
    // P21-009より前のセーブを模した、RawMagicCrystal/CrystalMine/CrystalRefineryへの
    // 言及が一切ないCountryStockpile/StateDataのRON(手書きで、旧セーブの実際の出力形式を再現)。
    let old_stockpile_ron = "(amounts:{Iron:10.0,Coal:5.0})";
    let restored_stockpile: CountryStockpile =
        ron::from_str(old_stockpile_ron).expect("old-format stockpile RON must still deserialize");
    assert_eq!(restored_stockpile.get(ResourceType::RawMagicCrystal), 0.0);
    assert_eq!(restored_stockpile.get(ResourceType::MagicCrystal), 0.0);
    assert_eq!(restored_stockpile.get(ResourceType::Iron), 10.0);

    let mut old_state = full_capacity_state(StateId(0), CountryId(0), &[(BuildingType::Mine, 1)]);
    old_state.resource_deposits.clear();
    let ron_str = ron::to_string(&old_state).expect("StateData must serialize to RON");
    let restored_state: StateData =
        ron::from_str(&ron_str).expect("StateData RON without new building keys must deserialize");
    assert_eq!(restored_state.building_level(BuildingType::CrystalMine), 0);
    assert_eq!(
        restored_state.building_level(BuildingType::CrystalRefinery),
        0
    );
}

// ─────────────────────────────────────────────────────────────────────────
// 6. 実データE2E(実7か国28州マップ、StateId(2)="Western Mage Province")
// ─────────────────────────────────────────────────────────────────────────

mod real_map_e2e {
    use super::*;
    use bevy::app::ScheduleRunnerPlugin;
    use bevy::prelude::*;
    use strategy_game::app::AppPlugin;
    use strategy_game::app::game_state::GameState;
    use strategy_game::building::BuildingPlugin;
    use strategy_game::building::construction::{ConstructionQueueItem, ConstructionStatus};
    use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
    use strategy_game::diplomacy::DiplomacyPlugin;
    use strategy_game::economy::EconomyPlugin;
    use strategy_game::military::MilitaryPlugin;
    use strategy_game::politics::PoliticsPlugin;
    use strategy_game::profiling::advance_one_day;
    use strategy_game::research::ResearchPlugin;
    use strategy_game::state::StatePlugin;
    use strategy_game::war::WarPlugin;

    /// `load_game_data`(Startup)がTechnologyRegistry/WorldCivilizationState/
    /// DiplomacyRegistry/MilitaryRegistryへ書き込むため、経済に無関係でも
    /// これらを提供するPluginを揃える(`p21_008_army_offensive_e2e_test.rs`と同じ理由)。
    fn setup_app_in_playing() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_resource::<ButtonInput<KeyCode>>()
            .add_plugins(AppPlugin)
            .add_plugins(CountryPlugin)
            .add_plugins(StatePlugin)
            .add_plugins(BuildingPlugin)
            .add_plugins(EconomyPlugin)
            .add_plugins(ResearchPlugin)
            .add_plugins(PoliticsPlugin)
            .add_plugins(DiplomacyPlugin)
            .add_plugins(WarPlugin)
            .add_plugins(MilitaryPlugin);

        app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

        app.insert_resource(PlayerCountry(Some(CountryId(0))));
        app.insert_state(GameState::Playing);
        app.update(); // OnEnter(Playing)

        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::Playing
        );
        app
    }

    /// 要求テスト項目25: 実データ上のクリスタル鉱床州(StateId(2)、"Western Mage Province"、
    /// `assets/data/resources.ron`でMagicCrystal鉱床が既に定義済み、所有国はCountryId(0))で、
    /// CrystalMine・CrystalRefineryを建設 → 日次建設進捗 → 月次生産(採掘→精製→
    /// 既存消費先MagicAcademyの消費)まで一気通貫することを確認する。
    #[test]
    fn crystal_mine_to_refinery_to_magic_academy_e2e_with_real_map_data() {
        let mut app = setup_app_in_playing();

        let (mine_def, refinery_def) = {
            let registry = app.world().resource::<BuildingRegistry>();
            let mine_def = registry
                .get(BuildingType::CrystalMine)
                .expect("buildings.ron must define CrystalMine")
                .clone();
            let refinery_def = registry
                .get(BuildingType::CrystalRefinery)
                .expect("buildings.ron must define CrystalRefinery")
                .clone();
            (mine_def, refinery_def)
        };

        // StateId(2)には実データ上、既にMagicAcademy:2/Farm:1が建っており、P21-009-FIX-001の
        // 互換移行により旧Mine:1はCrystalMine:1へ正規化済みの状態でロードされる
        // (assets/data/states.ron自体は無変更)。ここではその既存CrystalMineレベルを
        // 起点として、さらに1レベル分の新規建設(CrystalMine・CrystalRefinery)を積む。
        let (crystal_mine_baseline, crystal_refinery_baseline) = {
            let state_registry = app.world().resource::<StateRegistry>();
            let state2 = state_registry
                .get(StateId(2))
                .expect("StateId(2) must exist");
            (
                state2.building_level(BuildingType::CrystalMine),
                state2.building_level(BuildingType::CrystalRefinery),
            )
        };
        assert_eq!(
            crystal_mine_baseline, 1,
            "P21-009-FIX-001 migration must have already normalized the old Mine:1 at \
             StateId(2) into CrystalMine:1 before this test queues any new construction"
        );

        {
            let mut country_registry = app.world_mut().resource_mut::<CountryRegistry>();
            let country = country_registry
                .get_mut(CountryId(0))
                .expect("CountryId(0) must exist in the real map data");
            assert!(
                country.treasury >= mine_def.construction_cost + refinery_def.construction_cost,
                "player country must afford both new buildings from its real starting treasury"
            );
            country.treasury -= mine_def.construction_cost + refinery_def.construction_cost;
            country.construction_queue.push(ConstructionQueueItem {
                state_id: StateId(2),
                building_type: BuildingType::CrystalMine,
                target_level: crystal_mine_baseline + 1,
                progress: 0.0,
                required_progress: mine_def.required_progress,
                paid_cost: mine_def.construction_cost,
                status: ConstructionStatus::InQueue,
            });
            country.construction_queue.push(ConstructionQueueItem {
                state_id: StateId(2),
                building_type: BuildingType::CrystalRefinery,
                target_level: crystal_refinery_baseline + 1,
                progress: 0.0,
                required_progress: refinery_def.required_progress,
                paid_cost: refinery_def.construction_cost,
                status: ConstructionStatus::InQueue,
            });
        }

        let max_required_progress = mine_def
            .required_progress
            .max(refinery_def.required_progress);
        let construction_days = max_required_progress.ceil() as usize + 2;
        for _ in 0..construction_days {
            advance_one_day(&mut app);
        }

        {
            let state_registry = app.world().resource::<StateRegistry>();
            let state2 = state_registry
                .get(StateId(2))
                .expect("StateId(2) must exist");
            assert_eq!(
                state2.building_level(BuildingType::CrystalMine),
                crystal_mine_baseline + 1,
                "CrystalMine construction must complete via the real daily construction system"
            );
            assert_eq!(
                state2.building_level(BuildingType::CrystalRefinery),
                crystal_refinery_baseline + 1,
                "CrystalRefinery construction must complete via the real daily construction system"
            );
        }

        // 建設完了後、さらに複数ヶ月進め、採掘→精製→MagicAcademy消費までを実際の
        // 月次経済Systemに一気通貫させる。`magic_research_capacity`は月次で
        // その月の値へ上書きされるため(累積ではない)、精製済み在庫が需要と近い
        // 水準で「精製する月/消費する月」を行き来する動態になり得る。そのため
        // 全期間を通じて一度でも実際に稼働したかを追跡する(最終スナップショット
        // 1点だけを見ると、たまたま「精製する月」に当たり誤ってfailする)。
        let mut ever_saw_magic_output = false;
        let mut ever_saw_raw_material_mined = false;
        let mut ever_saw_refined_crystal_produced = false;
        for _ in 0..95 {
            advance_one_day(&mut app);
            let cr = app.world().resource::<CountryRegistry>();
            let c = cr.get(CountryId(0)).unwrap();
            if c.magic_research_capacity > 0.0 {
                ever_saw_magic_output = true;
            }
            if c.stockpile.get(ResourceType::RawMagicCrystal) > 0.0 {
                ever_saw_raw_material_mined = true;
            }
            if c.stockpile.get(ResourceType::MagicCrystal) > 0.0 {
                ever_saw_refined_crystal_produced = true;
            }
        }

        let country_registry = app.world().resource::<CountryRegistry>();
        let country = country_registry
            .get(CountryId(0))
            .expect("CountryId(0) must still exist");

        assert!(
            country.stockpile.get(ResourceType::RawMagicCrystal) >= 0.0,
            "raw crystal stockpile must stay non-negative"
        );
        assert!(
            country.stockpile.get(ResourceType::MagicCrystal) >= 0.0,
            "refined crystal stockpile must stay non-negative"
        );
        assert!(
            ever_saw_raw_material_mined,
            "CrystalMine must have mined raw material into the stockpile at some point"
        );
        assert!(
            ever_saw_refined_crystal_produced,
            "CrystalRefinery must have refined raw material into MagicCrystal at some point"
        );
        assert!(
            ever_saw_magic_output,
            "MagicAcademy (existing magic consumer) must have run on refined crystal supplied \
             by the new mining/refining chain at some point, producing magic research capacity"
        );
    }
}
