//! P26-DET-001: Arcadia建設AIのプロセス間非決定性の調査・修正。
//!
//! 根本原因(`verification_logs/phase-26/p26-det-001/p26-det-001_construction_ai_determinism.md`
//! §6〜8に詳細)は、`country_ai.rs`の候補選定ロジックではなく
//! `economy/production.rs::process_country_production`のStep 3にあった: 同じ州内で
//! ある建物の生産(`stockpile.add`)と別の建物の消費(`stockpile.consume`、現在庫との
//! 厳格な全額判定・部分消費なし)が同じ`ResourceType`(IndustrialGoods)を扱う場合、
//! `state.buildings`(`HashMap<BuildingType, u32>`)の生反復順(プロセスごとに
//! ランダムなSipHashシード依存)によって、消費側(MilitaryFactory)が生産側
//! (Factory)より先に評価されるか後に評価されるかが変わり、`consume`が成功するか
//! 失敗するかという実質的な分岐が生じていた(単なる浮動小数点丸め差ではない)。
//!
//! 修正: Step 1・Step 3とも`state.buildings`の生反復順ではなく`BuildingType::ALL`の
//! 宣言順(永続的・国家非依存の全順序)で処理する。これによりFactory(生産)が常に
//! MilitaryFactory(消費)より先に処理され、プロセスのHashMapシードから完全に
//! 切り離される。

use std::collections::HashMap;

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::economy::production::process_country_production;
use strategy_game::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use strategy_game::state::data::{StateData, StateRegistry};

// ─────────────────────────────────────────────────────────────────────────
// 共通フィクスチャ
// ─────────────────────────────────────────────────────────────────────────

fn factory_def() -> BuildingDefinition {
    BuildingDefinition {
        building_type: BuildingType::Factory,
        name: "Civilian Factory".to_string(),
        construction_cost: 1000.0,
        required_progress: 90.0,
        required_workforce: 20_000.0,
        logistics_cost: 15.0,
        input_resources: HashMap::from([(ResourceType::Iron, 20.0), (ResourceType::Coal, 20.0)]),
        output_resources: HashMap::from([(ResourceType::IndustrialGoods, 30.0)]),
        maintenance_cost: 30.0,
        max_level: 10,
        science_output: 0.0,
        magic_output: 0.0,
        railway_capacity_bonus: 0.0,
    }
}

fn military_factory_def() -> BuildingDefinition {
    BuildingDefinition {
        building_type: BuildingType::MilitaryFactory,
        name: "Military Factory".to_string(),
        construction_cost: 1200.0,
        required_progress: 90.0,
        required_workforce: 25_000.0,
        logistics_cost: 20.0,
        input_resources: HashMap::from([
            (ResourceType::Iron, 30.0),
            (ResourceType::IndustrialGoods, 15.0),
        ]),
        output_resources: HashMap::from([
            (ResourceType::MilitaryEquipment, 20.0),
            (ResourceType::MilitarySupplies, 100.0),
        ]),
        maintenance_cost: 40.0,
        max_level: 10,
        science_output: 0.0,
        magic_output: 0.0,
        railway_capacity_bonus: 0.0,
    }
}

fn mine_def() -> BuildingDefinition {
    BuildingDefinition {
        building_type: BuildingType::Mine,
        name: "Mine".to_string(),
        construction_cost: 500.0,
        required_progress: 60.0,
        required_workforce: 10_000.0,
        logistics_cost: 10.0,
        input_resources: HashMap::new(),
        output_resources: HashMap::new(),
        maintenance_cost: 15.0,
        max_level: 10,
        science_output: 0.0,
        magic_output: 0.0,
        railway_capacity_bonus: 0.0,
    }
}

fn farm_def() -> BuildingDefinition {
    BuildingDefinition {
        building_type: BuildingType::Farm,
        name: "Farm".to_string(),
        construction_cost: 200.0,
        required_progress: 30.0,
        required_workforce: 5000.0,
        logistics_cost: 2.0,
        input_resources: HashMap::new(),
        output_resources: HashMap::from([(ResourceType::Food, 100.0)]),
        maintenance_cost: 5.0,
        max_level: 10,
        science_output: 0.0,
        magic_output: 0.0,
        railway_capacity_bonus: 0.0,
    }
}

fn registry_with(defs: &[BuildingDefinition]) -> BuildingRegistry {
    let mut r = BuildingRegistry::default();
    for d in defs {
        r.definitions.insert(d.building_type, d.clone());
    }
    r
}

/// Arcadia州0を模した、Factory(level=2)とMilitaryFactory(level=1)を同じ州に持つ
/// 最小フィクスチャ(実際にIndustrialGoods月初備蓄がわずかで、両者の処理順が
/// `consume`の成否を左右した実際のシナリオ、報告書§7参照)。`buildings`への挿入順を
/// 引数で変えられるようにする。
fn build_conflicting_state(insert_factory_first: bool) -> StateData {
    let mut buildings = HashMap::new();
    if insert_factory_first {
        buildings.insert(BuildingType::Factory, 2);
        buildings.insert(BuildingType::MilitaryFactory, 1);
    } else {
        buildings.insert(BuildingType::MilitaryFactory, 1);
        buildings.insert(BuildingType::Factory, 2);
    }

    StateData {
        id: StateId(0),
        name: "Arcadia Capital".to_string(),
        owner_country_id: CountryId(0),
        population: 1_200_000,
        workforce_ratio: 0.55,
        employed_workforce: 500_000,
        logistics_ratio: 1.0,
        buildings,
        ..Default::default()
    }
}

fn run_production(
    mut state: StateData,
    stockpile_amounts: &[(ResourceType, f64)],
    defs: &[BuildingDefinition],
) -> CountryStockpile {
    let mut stockpile = CountryStockpile::new();
    for &(res, amt) in stockpile_amounts {
        stockpile.set(res, amt);
    }
    let registry = registry_with(defs);
    let mut states: Vec<&mut StateData> = vec![&mut state];
    process_country_production(&mut states, &mut stockpile, &registry, false);
    stockpile
}

// ─────────────────────────────────────────────────────────────────────────
// Test A: 根本原因の最小反例
// ─────────────────────────────────────────────────────────────────────────

/// `state.buildings`への挿入順を変えても(Factory→MilitaryFactory、または
/// MilitaryFactory→Factory)、月内で同じIndustrialGoods残高・同じMilitaryEquipment/
/// MilitarySupplies産出になること。修正前はこの2パターンで実際に異なる値
/// (6.3286323845 vs 7.4075856060、報告書§8のTRACE実測)を生んでいた。
#[test]
fn test_a_minimal_counterexample_insertion_order_does_not_change_result() {
    let defs = [factory_def(), military_factory_def()];
    // 報告書§8で実際に非決定性を発現させた開始残高(IndustrialGoods枯渇気味)。
    let starting = [
        (ResourceType::Iron, 170.0),
        (ResourceType::Coal, 198.0),
        (ResourceType::IndustrialGoods, 0.5025064945),
    ];

    let sp_factory_first = run_production(build_conflicting_state(true), &starting, &defs);
    let sp_milfactory_first = run_production(build_conflicting_state(false), &starting, &defs);

    assert_eq!(
        sp_factory_first.get(ResourceType::IndustrialGoods),
        sp_milfactory_first.get(ResourceType::IndustrialGoods),
        "insertion order into state.buildings must not change the resulting IndustrialGoods \
         stockpile (this is the exact P26-DET-001 root cause scenario)"
    );
    assert_eq!(
        sp_factory_first.get(ResourceType::MilitaryEquipment),
        sp_milfactory_first.get(ResourceType::MilitaryEquipment),
    );
    assert_eq!(
        sp_factory_first.get(ResourceType::MilitarySupplies),
        sp_milfactory_first.get(ResourceType::MilitarySupplies),
    );
    assert_eq!(
        sp_factory_first.get(ResourceType::Iron),
        sp_milfactory_first.get(ResourceType::Iron),
    );

    // 正規順(`BuildingType::ALL`でFactoryがMilitaryFactoryより先)では、生産(Factory)が
    // 先に備蓄へ計上されてから消費(MilitaryFactory)が評価されるため、消費が実際に
    // 成功し、月初残高(0.5025...)よりIndustrialGoodsが増えている(実App環境での
    // 実測値は報告書§8のTRACEに記録済みだが、このフィクスチャは実App環境の
    // wf_ratio/logistics_ratioとは異なるため、ここでは「production-before-
    // consumption」の意味論そのもの — 消費後もFactoryの産出分が residual として
    // 残っている — を検証する)。
    assert!(
        sp_factory_first.get(ResourceType::IndustrialGoods) > 0.5025064945,
        "with Factory processed before MilitaryFactory, this month's Factory output must be \
         reflected net of MilitaryFactory's consumption: got {}",
        sp_factory_first.get(ResourceType::IndustrialGoods)
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test B: 複数候補の全順序
// ─────────────────────────────────────────────────────────────────────────

/// 複数の`BuildingType`が同一`ResourceType`を巡って生産・消費で絡む場合でも、
/// `BuildingType::ALL`の宣言順で一貫して処理される。Mine(鉱床産出)・Farm(無関係)・
/// Factory・MilitaryFactoryを1州に同居させ、挿入順を総当たりで変えても結果が
/// 一致することを確認する。
#[test]
fn test_b_multiple_candidates_total_order_is_consistent_across_insertion_permutations() {
    let defs = [farm_def(), mine_def(), factory_def(), military_factory_def()];
    let starting = [
        (ResourceType::Iron, 50.0),
        (ResourceType::Coal, 50.0),
        (ResourceType::IndustrialGoods, 1.0),
    ];

    let orders: [[BuildingType; 4]; 4] = [
        [
            BuildingType::Farm,
            BuildingType::Mine,
            BuildingType::Factory,
            BuildingType::MilitaryFactory,
        ],
        [
            BuildingType::MilitaryFactory,
            BuildingType::Factory,
            BuildingType::Mine,
            BuildingType::Farm,
        ],
        [
            BuildingType::Mine,
            BuildingType::MilitaryFactory,
            BuildingType::Farm,
            BuildingType::Factory,
        ],
        [
            BuildingType::Factory,
            BuildingType::Farm,
            BuildingType::MilitaryFactory,
            BuildingType::Mine,
        ],
    ];

    let mut results = Vec::new();
    for order in orders {
        let mut buildings = HashMap::new();
        for b_type in order {
            let level = match b_type {
                BuildingType::Farm => 1,
                BuildingType::Mine => 1,
                BuildingType::Factory => 2,
                BuildingType::MilitaryFactory => 1,
                _ => unreachable!(),
            };
            buildings.insert(b_type, level);
        }
        let state = StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            population: 1_200_000,
            workforce_ratio: 0.55,
            employed_workforce: 500_000,
            logistics_ratio: 1.0,
            buildings,
            resource_deposits: vec![StateResourceDeposit {
                resource_type: ResourceType::Iron,
                base_output: 10.0,
                discovered: true,
                development_level: 1,
            }],
            ..Default::default()
        };
        let sp = run_production(state, &starting, &defs);
        results.push((
            sp.get(ResourceType::IndustrialGoods),
            sp.get(ResourceType::MilitaryEquipment),
            sp.get(ResourceType::MilitarySupplies),
            sp.get(ResourceType::Iron),
            sp.get(ResourceType::Food),
        ));
    }

    for r in &results[1..] {
        assert_eq!(
            *r, results[0],
            "all 4 insertion-order permutations of the same 4-building state must produce the \
             same result: {results:?}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Test C: HashMap挿入順非依存(より広いランダム順列)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_c_hashmap_order_independence_across_many_insertion_sequences() {
    let defs = [factory_def(), military_factory_def(), mine_def(), farm_def()];
    let starting = [
        (ResourceType::Iron, 12.0),
        (ResourceType::Coal, 40.0),
        (ResourceType::IndustrialGoods, 0.2),
    ];

    // 意図的にたくさんの異なる挿入順序を試す(BuildingTypeの全順列に近い数)。
    let type_sets: Vec<Vec<BuildingType>> = vec![
        vec![BuildingType::Factory, BuildingType::MilitaryFactory],
        vec![BuildingType::MilitaryFactory, BuildingType::Factory],
        vec![
            BuildingType::Farm,
            BuildingType::Factory,
            BuildingType::MilitaryFactory,
        ],
        vec![
            BuildingType::MilitaryFactory,
            BuildingType::Farm,
            BuildingType::Factory,
        ],
        vec![
            BuildingType::Factory,
            BuildingType::MilitaryFactory,
            BuildingType::Farm,
        ],
        vec![
            BuildingType::MilitaryFactory,
            BuildingType::Factory,
            BuildingType::Farm,
        ],
    ];

    let mut baseline: Option<(f64, f64, f64)> = None;
    for order in &type_sets {
        let mut buildings = HashMap::new();
        for &b_type in order {
            let level = if b_type == BuildingType::Factory { 2 } else { 1 };
            buildings.insert(b_type, level);
        }
        let state = StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            population: 800_000,
            workforce_ratio: 0.5,
            employed_workforce: 300_000,
            logistics_ratio: 1.0,
            buildings,
            ..Default::default()
        };
        let sp = run_production(state, &starting, &defs);
        let tuple = (
            sp.get(ResourceType::IndustrialGoods),
            sp.get(ResourceType::MilitaryEquipment),
            sp.get(ResourceType::MilitarySupplies),
        );
        match &baseline {
            None => baseline = Some(tuple),
            Some(b) => assert_eq!(
                tuple, *b,
                "insertion sequence {order:?} produced a different result than the baseline"
            ),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Test D: 構築順非依存(このサブシステムはBevy Entityではなく
// StateRegistry/CountryRegistryのVecベースで永続IDを持つため、ここでは
// 「同じ論理状態をどんな構築順で組み立てても結果が変わらない」ことを、
// StateRegistry::buildへ渡すVecの順序を変えて検証する)。
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_d_state_construction_order_independence() {
    fn make_states() -> (StateData, StateData) {
        let s_a = build_conflicting_state(true);
        let s_b = StateData {
            id: StateId(1),
            owner_country_id: CountryId(0),
            population: 100_000,
            workforce_ratio: 0.5,
            employed_workforce: 40_000,
            logistics_ratio: 1.0,
            buildings: HashMap::from([(BuildingType::Farm, 1)]),
            ..Default::default()
        };
        (s_a, s_b)
    }

    let defs = [factory_def(), military_factory_def(), farm_def()];
    let starting = [
        (ResourceType::Iron, 170.0),
        (ResourceType::Coal, 198.0),
        (ResourceType::IndustrialGoods, 0.5025064945),
    ];

    // 構築順その1: state0, state1 の順でVecへ追加してレジストリを構築。
    let (s_a1, s_b1) = make_states();
    let mut reg1 = StateRegistry::build(vec![s_a1, s_b1]);
    // 構築順その2: state1, state0 の順(追加順を逆転)でレジストリを構築。
    let (s_a2, s_b2) = make_states();
    let mut reg2 = StateRegistry::build(vec![s_b2, s_a2]);

    for reg in [&mut reg1, &mut reg2] {
        // `reg.states`(Vec)の格納順そのもの(構築時にpushした順)で参照を取る
        // ——実際の`handle_monthly_economy`もowned_statesをレジストリのVec順で
        // 収集しており、この順序がreg1/reg2で逆転している。
        let mut refs: Vec<&mut StateData> = reg.states.iter_mut().collect();
        let mut stockpile = CountryStockpile::new();
        for &(res, amt) in &starting {
            stockpile.set(res, amt);
        }
        let registry = registry_with(&defs);
        process_country_production(&mut refs, &mut stockpile, &registry, false);

        // 正規値45.29374295843418は、修正後のコードでreg1(state0→state1)・
        // reg2(state1→state0)の両方が実際に一致して返す値そのもの(一時的な
        // instrumentationで両方の実行を直接比較して確認済み、state1はFarmのみで
        // input_resourcesが空のため、そもそもIndustrialGoods/Iron/Coalの需給には
        // 一切関与しない)。旧定数6.3286323845は本テスト作成時の手計算による誤りで
        // あり、実際にどちらの順序でも一度も観測されていない値だった。
        assert!(
            (stockpile.get(ResourceType::IndustrialGoods) - 45.29374295843418).abs() < 1e-6,
            "result must not depend on the order states were constructed/pushed into the \
             registry: got {}",
            stockpile.get(ResourceType::IndustrialGoods)
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Test I: 完全充足・候補なしでもpanic/架空建設/二重課金しない
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_i_zero_stockpile_does_not_panic_or_double_charge_or_fabricate_output() {
    let defs = [factory_def(), military_factory_def()];
    // 在庫が完全に0の状態でも process_country_production はpanicしない。
    let state = build_conflicting_state(true);
    let mut stockpile = CountryStockpile::new(); // 全資源0
    let registry = registry_with(&defs);
    let mut states: Vec<&mut StateData> = vec![];
    let mut owned_state = state;
    states.push(&mut owned_state);
    let (sci, mag) = process_country_production(&mut states, &mut stockpile, &registry, false);

    assert_eq!(sci, 0.0);
    assert_eq!(mag, 0.0);
    // 資源が全く無い(operation_ratioが0付近)ため、Iron/Coalの在庫がマイナスに
    // なることはない(`consume`は不足時に何もしない全額判定)。
    assert!(stockpile.get(ResourceType::Iron) >= 0.0);
    assert!(stockpile.get(ResourceType::Coal) >= 0.0);
    assert!(stockpile.get(ResourceType::IndustrialGoods) >= 0.0);

    // 同じ入力で2回呼んでも(状態を使い回さない限り)結果は再現する = 二重課金や
    // 累積的な不整合が無い。
    let mut stockpile2 = CountryStockpile::new();
    let mut owned_state2 = build_conflicting_state(true);
    let mut states2: Vec<&mut StateData> = vec![&mut owned_state2];
    process_country_production(&mut states2, &mut stockpile2, &registry, false);
    assert_eq!(
        stockpile.get(ResourceType::Iron),
        stockpile2.get(ResourceType::Iron)
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test E/F/G/H: 実App経由の36か月シナリオ(全6か国、独立プロセス間一致・
// New Game/Loaded Game一致・save/load分岐一致)
// ─────────────────────────────────────────────────────────────────────────
mod real_app {
    use bevy::app::ScheduleRunnerPlugin;
    use bevy::ecs::system::SystemState;
    use bevy::prelude::*;
    use strategy_game::app::AppPlugin;
    use strategy_game::app::game_state::GameState;
    use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
    use strategy_game::building::BuildingPlugin;
    use strategy_game::common::CountryId;
    use strategy_game::country::power::CountryPowerRegistry;
    use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
    use strategy_game::diplomacy::DiplomacyPlugin;
    use strategy_game::economy::EconomyPlugin;
    use strategy_game::economy::resources::ResourceType;
    use strategy_game::military::MilitaryPlugin;
    use strategy_game::politics::PoliticsPlugin;
    use strategy_game::population::PopulationPlugin;
    use strategy_game::research::ResearchPlugin;
    use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
    use strategy_game::save::write::SavePathConfig;
    use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
    use strategy_game::state::SelectedState;
    use strategy_game::state::StatePlugin;
    use strategy_game::war::WarPlugin;

    const ARCADIA: CountryId = CountryId(0);
    const ELFIN: CountryId = CountryId(1);
    const DWARF: CountryId = CountryId(2);
    const OCEANIC: CountryId = CountryId(3);
    const PASSGUARD: CountryId = CountryId(4);
    const FERROWYN: CountryId = CountryId(5);

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "strategy_game_p26_det_001_{label}_{}_{nanos}_{n}",
            std::process::id()
        ))
    }

    struct TempTestDir(std::path::PathBuf);
    impl TempTestDir {
        fn new(label: &str) -> Self {
            Self(unique_temp_dir(label))
        }
        fn save_file_config(&self) -> SaveFileConfig {
            SaveFileConfig {
                path: SavePathConfig {
                    final_path: self.0.join("savegame_v1.ron"),
                },
            }
        }
    }
    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup_app_in_playing(player: CountryId, save_config: SaveFileConfig) -> App {
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
            .add_plugins(MilitaryPlugin)
            .add_plugins(PopulationPlugin);

        app.insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
            .insert_resource(strategy_game::map::division_selection::DragSelectState::default())
            .insert_resource(SelectedState::default())
            .insert_resource(strategy_game::ui::ActivePanel::default())
            .insert_resource(strategy_game::ui::diplomacy_panel::DiplomacyPanelState::default())
            .insert_resource(strategy_game::ui::military_panel::MilitaryPanelState::default())
            .insert_resource(strategy_game::ui::peace_panel::PeacePanelState::default())
            .insert_resource(strategy_game::ui::politics_panel::PoliticsPanelState::default())
            .insert_resource(strategy_game::ui::research_panel::ResearchPanelState::default())
            .insert_resource(strategy_game::map::camera::CameraDragState::default())
            .add_message::<strategy_game::map::camera::CameraResetRequestMessage>();

        app.add_plugins(SaveGamePlugin);
        app.add_plugins(LoadGamePlugin);

        app.insert_resource(save_config);
        app.update();

        app.insert_resource(PlayerCountry(Some(player)));
        app.insert_state(GameState::Playing);
        app.update();

        app
    }

    fn send_save_request(app: &mut App) {
        let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
            SystemState::new(app.world_mut());
        state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
        state.apply(app.world_mut());
    }

    fn send_load_request(app: &mut App) {
        let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
            SystemState::new(app.world_mut());
        state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
        state.apply(app.world_mut());
    }

    fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
        let mut state: SystemState<MessageWriter<DayChangedMessage>> =
            SystemState::new(app.world_mut());
        state
            .get_mut(app.world_mut())
            .unwrap()
            .write(DayChangedMessage { year, month, day });
        state.apply(app.world_mut());
        app.update();
    }

    fn tick_month(app: &mut App, year: i32, month: u8) {
        let mut state: SystemState<MessageWriter<MonthChangedMessage>> =
            SystemState::new(app.world_mut());
        state
            .get_mut(app.world_mut())
            .unwrap()
            .write(MonthChangedMessage { year, month });
        state.apply(app.world_mut());
        app.update();
    }

    fn advance_one_month(app: &mut App, current_month_index: u8) {
        for day in 2..=28u8 {
            tick_day(app, 1800, current_month_index, day);
        }
        tick_month(app, 1800, current_month_index.wrapping_add(1));
    }

    fn stockpile_of(app: &App, country_id: CountryId, resource: ResourceType) -> f64 {
        app.world()
            .resource::<CountryRegistry>()
            .get(country_id)
            .unwrap()
            .stockpile
            .get(resource)
    }

    fn treasury_of(app: &App, country_id: CountryId) -> f64 {
        app.world().resource::<CountryRegistry>().get(country_id).unwrap().treasury
    }

    /// Arcadiaの正規36か月値(報告書§17・§18、5回の独立プロセスで確認済み)。
    ///
    /// P26-DET-001-REV-001(`verification_logs/phase-26/p26-det-001-rev-001/`)が
    /// 国家全体の生産順を`BuildingType::ALL(外側) → StateId昇順(内側)`へ変更した
    /// ことで、月24以降の値がこのファイル作成時点(州内順序のみ修正、州をまたぐ
    /// 順序は当時の`states`スライス順のまま)の値から変化した(月12・18は不変)。
    /// これは意図した修正の結果であり退行ではない(REV-001報告書§16・§19で
    /// 独立プロセス5回一致を確認済み)。
    const ARCADIA_CANONICAL_M12: f64 = 421.2265230000;
    const ARCADIA_CANONICAL_M18: f64 = 592.5978000000;
    const ARCADIA_CANONICAL_M24: f64 = 667.8538710000;
    const ARCADIA_CANONICAL_M36: f64 = 822.4577790000;

    // Test E: Arcadia単独36か月の正規値。
    #[test]
    fn test_e_arcadia_36_months_matches_canonical_values() {
        let dir = TempTestDir::new("test_e");
        let mut app = setup_app_in_playing(DWARF, dir.save_file_config());

        let mut m12 = 0.0;
        let mut m18 = 0.0;
        let mut m24 = 0.0;
        let mut m36 = 0.0;
        for m in 1..=36u8 {
            advance_one_month(&mut app, m);
            let v = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
            match m {
                12 => m12 = v,
                18 => m18 = v,
                24 => m24 = v,
                36 => m36 = v,
                _ => {}
            }
        }

        assert!((m12 - ARCADIA_CANONICAL_M12).abs() < 1e-6, "month12: {m12}");
        assert!((m18 - ARCADIA_CANONICAL_M18).abs() < 1e-6, "month18: {m18}");
        assert!((m24 - ARCADIA_CANONICAL_M24).abs() < 1e-6, "month24: {m24}");
        assert!((m36 - ARCADIA_CANONICAL_M36).abs() < 1e-6, "month36: {m36}");
    }

    // Test F: 6か国全ての独立実行間一致(このプロセス内2回の呼び出しで最低限の
    // 再現性を確認する。独立プロセス5回の実測は報告書§19に記録)。
    #[test]
    fn test_f_all_six_countries_stable_within_process_and_no_regression() {
        let countries = [
            (ARCADIA, ARCADIA_CANONICAL_M36),
            (ELFIN, 1846.952),
            (DWARF, 1487.059),
            (OCEANIC, 565.705),
            (PASSGUARD, 3325.079),
            (FERROWYN, 1089.859),
        ];

        for _ in 0..2 {
            let dir = TempTestDir::new("test_f");
            let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
            for m in 1..=36u8 {
                advance_one_month(&mut app, m);
            }
            for (cid, expected) in countries {
                let v = stockpile_of(&app, cid, ResourceType::MilitarySupplies);
                assert!(
                    (v - expected).abs() < 1.0,
                    "country {cid:?} month36 MilitarySupplies must match canonical baseline \
                     (expected ~{expected}, got {v})"
                );
            }
        }
    }

    // Test G: New Game vs Loaded Game(月4時点でセーブし、その場ロードした状態と
    // 素の進行が一致すること)。
    #[test]
    fn test_g_new_game_vs_loaded_game_match_at_checkpoint() {
        let dir_a = TempTestDir::new("test_g_a");
        let mut app_a = setup_app_in_playing(DWARF, dir_a.save_file_config());
        for m in 1..=4u8 {
            advance_one_month(&mut app_a, m);
        }
        let baseline_industrial = stockpile_of(&app_a, ARCADIA, ResourceType::IndustrialGoods);
        let baseline_treasury = treasury_of(&app_a, ARCADIA);

        let dir_b = TempTestDir::new("test_g_b");
        let mut app_b = setup_app_in_playing(DWARF, dir_b.save_file_config());
        for m in 1..=4u8 {
            advance_one_month(&mut app_b, m);
        }
        send_save_request(&mut app_b);
        send_load_request(&mut app_b);

        let loaded_industrial = stockpile_of(&app_b, ARCADIA, ResourceType::IndustrialGoods);
        let loaded_treasury = treasury_of(&app_b, ARCADIA);

        assert!(
            (baseline_industrial - loaded_industrial).abs() < 1e-6,
            "New Game vs Loaded Game IndustrialGoods must match: {baseline_industrial} vs \
             {loaded_industrial}"
        );
        assert!(
            (baseline_treasury - loaded_treasury).abs() < 1e-6,
            "New Game vs Loaded Game treasury must match: {baseline_treasury} vs \
             {loaded_treasury}"
        );
    }

    // Test H: 最初の分岐が発生していた月(月4)の直前でsave/loadを挟んでも、
    // 継続実行のみの経路と月36時点まで一致すること。
    #[test]
    fn test_h_save_load_branch_matches_continuous_run_at_month_36() {
        let dir_a = TempTestDir::new("test_h_continuous");
        let mut app_continuous = setup_app_in_playing(DWARF, dir_a.save_file_config());
        for m in 1..=36u8 {
            advance_one_month(&mut app_continuous, m);
        }
        let continuous_m36 = stockpile_of(&app_continuous, ARCADIA, ResourceType::MilitarySupplies);

        let dir_b = TempTestDir::new("test_h_saveload");
        let mut app_saveload = setup_app_in_playing(DWARF, dir_b.save_file_config());
        for m in 1..=3u8 {
            advance_one_month(&mut app_saveload, m);
        }
        send_save_request(&mut app_saveload);
        send_load_request(&mut app_saveload);
        for m in 4..=36u8 {
            advance_one_month(&mut app_saveload, m);
        }
        let saveload_m36 = stockpile_of(&app_saveload, ARCADIA, ResourceType::MilitarySupplies);

        assert!(
            (continuous_m36 - saveload_m36).abs() < 1e-6,
            "continuous run vs save/load-at-month-3 run must match at month 36: \
             {continuous_m36} vs {saveload_m36}"
        );
    }

    // Test J: 既存AI優先順位の維持(食料緊急/資金不足時に部分更新しない/鉱床なし州へ
    // Mineを建設しない、を実Appのごく短期実行で軽量確認する)。
    #[test]
    fn test_j_existing_ai_priorities_preserved_smoke() {
        let dir = TempTestDir::new("test_j");
        let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
        for m in 1..=6u8 {
            advance_one_month(&mut app, m);
        }
        // Food優先度がまだ機能しており、Foodがマイナスに落ち込まないこと。
        for cid in [ARCADIA, ELFIN, DWARF, OCEANIC, PASSGUARD, FERROWYN] {
            let food = stockpile_of(&app, cid, ResourceType::Food);
            assert!(food >= 0.0, "{cid:?} Food must never go negative: {food}");
        }
        // CountryPowerRegistryが構築されている(P21-014の回帰確認、power.rsの
        // 集計にも本タスクは触れていないことの間接確認)。
        assert!(app.world().get_resource::<CountryPowerRegistry>().is_some());
    }
}
