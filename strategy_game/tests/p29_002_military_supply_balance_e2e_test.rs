//! P29-002: MilitarySuppliesの「前回の月次生産(実産出)」の記録(表示専用ledger)の
//! 契約テスト。
//!
//! - 記録される値は、月次生産が実際に備蓄へ反映した量(FIX1・D4・入力不足・量子化後)であり、
//!   建物Levelからの最大能力・稼働率からの見込み・備蓄の純増減ではない。
//! - 集計・commitに失敗した月は「未集計」(0ではない)。
//! - ledgerはSave V1へ含めず、New Game(国家選択画面)・ロード成功で空に戻る。
//! - 表示のための経路(ledgerの記録)は、備蓄・生産結果・研究力を変えない。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::production::{
    process_country_production, process_country_production_with_outcome,
};
use strategy_game::economy::resources::{
    CountryStockpile, MILITARY_SUPPLIES_MAX_UNITS, ResourceType, military_supplies_units_to_value,
    military_supplies_value_to_units,
};
use strategy_game::economy::supply_balance::{
    MilitarySupplyProductionLedger, MonthlyProductionRecord,
};
use strategy_game::military::MilitaryPlugin;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateData;
use strategy_game::war::WarPlugin;

// ─────────────────────────────────────────────────────────────────────────
// Fixture: 入力(Iron/IndustrialGoods)つきMilitaryFactoryの純粋な生産関数
// ─────────────────────────────────────────────────────────────────────────

fn military_factory_registry() -> BuildingRegistry {
    let mut definitions = HashMap::new();
    definitions.insert(
        BuildingType::MilitaryFactory,
        BuildingDefinition {
            building_type: BuildingType::MilitaryFactory,
            name: "Military Factory".to_string(),
            construction_cost: 1200.0,
            required_progress: 90.0,
            required_workforce: 25000.0,
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
        },
    );
    BuildingRegistry { definitions }
}

fn full_capacity_state(id: usize, level: u32) -> StateData {
    StateData {
        id: StateId(id),
        population: 100_000,
        workforce_ratio: 0.5,
        employed_workforce: 50_000,
        logistics_ratio: 1.0,
        buildings: HashMap::from([(BuildingType::MilitaryFactory, level)]),
        ..StateData::default()
    }
}

fn stock(items: &[(ResourceType, f64)]) -> CountryStockpile {
    let mut s = CountryStockpile::new();
    for &(r, a) in items {
        s.set(r, a);
    }
    s
}

fn units(value: f64) -> u64 {
    military_supplies_value_to_units(value).unwrap()
}

/// 契約1: 実産出は備蓄へ反映された量と(units単位で)一致し、複数州を重複なく合計する。
#[test]
fn reported_production_equals_the_exact_amount_added_to_the_stockpile_across_states() {
    let reg = military_factory_registry();
    let mut s0 = full_capacity_state(0, 2);
    let mut s1 = full_capacity_state(1, 3);
    let mut states: Vec<&mut StateData> = vec![&mut s0, &mut s1];
    let mut stockpile = stock(&[
        (ResourceType::Iron, 10_000.0),
        (ResourceType::IndustrialGoods, 10_000.0),
        (ResourceType::MilitarySupplies, 123.456789),
    ]);
    let before = units(stockpile.get(ResourceType::MilitarySupplies));
    let out = process_country_production_with_outcome(&mut states, &mut stockpile, &reg, false);
    let after = units(stockpile.get(ResourceType::MilitarySupplies));
    let produced = out.military_supplies_produced.expect("commit succeeded");
    assert_eq!(units(produced), after - before, "reported == stockpile delta in units");
    assert_eq!(produced, 500.0, "Lv2 + Lv3, each 100/Level at full operation");
}

/// 契約2: 入力不足による生産減少(FIX1の稼働率引下げ)を反映する。能力(Level×100)ではない。
#[test]
fn reported_production_reflects_input_shortage_not_the_capacity() {
    let reg = military_factory_registry();
    let mut s0 = full_capacity_state(0, 2);
    let mut states: Vec<&mut StateData> = vec![&mut s0];
    // Lv2 の月間必要量は Iron 60・IG 30。IG が半分(15)しかない。
    let mut stockpile = stock(&[
        (ResourceType::Iron, 10_000.0),
        (ResourceType::IndustrialGoods, 15.0),
    ]);
    let out = process_country_production_with_outcome(&mut states, &mut stockpile, &reg, false);
    let produced = out.military_supplies_produced.expect("commit succeeded");
    assert!(produced > 0.0 && produced < 200.0, "produced={produced}");
    assert!((produced - 100.0).abs() < 1e-3, "IG halves the operation: produced={produced}");
    assert_eq!(units(produced), units(stockpile.get(ResourceType::MilitarySupplies)));
    // 入力を全く賄えなければ、産出は0(未集計ではなく実績0)。
    let mut s1 = full_capacity_state(1, 2);
    let mut states: Vec<&mut StateData> = vec![&mut s1];
    let mut empty = stock(&[]);
    let out = process_country_production_with_outcome(&mut states, &mut empty, &reg, false);
    assert_eq!(out.military_supplies_produced, Some(0.0));
}

/// 契約3: 軍需工場がない(産出0)は実績0であり、未集計ではない。
#[test]
fn no_military_factory_reports_a_real_zero() {
    let reg = military_factory_registry();
    let mut s0 = full_capacity_state(0, 0);
    let mut states: Vec<&mut StateData> = vec![&mut s0];
    let mut stockpile = stock(&[(ResourceType::MilitarySupplies, 7.0)]);
    let out = process_country_production_with_outcome(&mut states, &mut stockpile, &reg, false);
    assert_eq!(out.military_supplies_produced, Some(0.0));
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 7.0);
}

/// 契約4: commitに失敗した月(備蓄が上限を超える)は`None`(未集計)で、備蓄は不変。
#[test]
fn commit_failure_is_unavailable_not_zero_and_leaves_the_stockpile_unchanged() {
    let reg = military_factory_registry();
    let mut s0 = full_capacity_state(0, 1);
    let mut states: Vec<&mut StateData> = vec![&mut s0];
    let near_max = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS - 1);
    let mut stockpile = stock(&[
        (ResourceType::Iron, 100.0),
        (ResourceType::IndustrialGoods, 100.0),
        (ResourceType::MilitarySupplies, near_max),
    ]);
    let out = process_country_production_with_outcome(&mut states, &mut stockpile, &reg, false);
    assert_eq!(out.military_supplies_produced, None);
    assert_eq!(stockpile.get(ResourceType::MilitarySupplies), near_max);
}

/// 契約5: 実績を返す経路は、従来の経路と備蓄・研究力・稼働率が完全に同一(表示のために
/// 生産結果を変えない)。
#[test]
fn outcome_variant_is_bit_identical_to_the_legacy_entry_point() {
    let reg = military_factory_registry();
    let make = || {
        (
            vec![full_capacity_state(0, 2), full_capacity_state(1, 1)],
            stock(&[
                (ResourceType::Iron, 70.0),
                (ResourceType::IndustrialGoods, 40.0),
                (ResourceType::MilitarySupplies, 3.0),
            ]),
        )
    };
    let (mut a_states, mut a_stock) = make();
    let (sci_a, mag_a) = {
        let mut refs: Vec<&mut StateData> = a_states.iter_mut().collect();
        process_country_production(&mut refs, &mut a_stock, &reg, false)
    };
    let (mut b_states, mut b_stock) = make();
    let out = {
        let mut refs: Vec<&mut StateData> = b_states.iter_mut().collect();
        process_country_production_with_outcome(&mut refs, &mut b_stock, &reg, false)
    };
    assert_eq!((sci_a, mag_a), (out.science, out.magic));
    for r in ResourceType::ALL {
        assert_eq!(a_stock.get(r).to_bits(), b_stock.get(r).to_bits(), "{r:?}");
    }
    for (a, b) in a_states.iter().zip(b_states.iter()) {
        assert_eq!(a.building_operation_ratios, b.building_operation_ratios);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 実App: 月次システム → ledger、New Game・ロードでの消去
// ─────────────────────────────────────────────────────────────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p29_002_{label}_{}_{nanos}_{n}",
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
    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing)

    app
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

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(SaveRequestMessage);
    state.apply(app.world_mut());
    app.update();
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
}

fn supplies_units_of_all(app: &App) -> Vec<(CountryId, u64)> {
    app.world()
        .resource::<CountryRegistry>()
        .countries
        .iter()
        .map(|c| (c.id, units(c.stockpile.get(ResourceType::MilitarySupplies))))
        .collect()
}

/// 全国家について、ledgerの実産出が月次生産直前後の備蓄増分(units)と一致する。
/// 日次消費を挟まないため、純増減=実産出になる。国家ごとに独立した値である。
#[test]
fn monthly_system_records_each_countrys_actual_output_and_never_mixes_countries() {
    let dir = TempTestDir::new("ledger");
    let mut app = setup_app_in_playing(CountryId(2), dir.save_file_config());
    assert!(
        app.world().resource::<MilitarySupplyProductionLedger>().is_empty(),
        "no monthly production observed yet: 未集計"
    );
    // 就業人口が立ち上がるまで数か月進める(既存の生産E2Eと同じ手順)。
    for m in 1..=3u8 {
        for day in 2..=28u8 {
            tick_day(&mut app, 1800, m, day);
        }
        tick_month(&mut app, 1800, m + 1);
    }
    let before = supplies_units_of_all(&app);
    tick_month(&mut app, 1800, 5);
    let after = supplies_units_of_all(&app);

    let ledger = app.world().resource::<MilitarySupplyProductionLedger>().clone();
    let mut positive = 0;
    let mut distinct = std::collections::BTreeSet::new();
    for ((id, b), (_, a)) in before.iter().zip(after.iter()) {
        let entry = ledger.get(*id).unwrap_or_else(|| panic!("entry for {id:?}"));
        assert_eq!((entry.year, entry.month), (1800, 5));
        match entry.record {
            MonthlyProductionRecord::Produced(v) => {
                assert_eq!(units(v), a - b, "country {id:?}: reported == stockpile delta");
                if v > 0.0 {
                    positive += 1;
                }
                distinct.insert(units(v));
            }
            MonthlyProductionRecord::Unavailable => panic!("unexpected unavailable for {id:?}"),
        }
    }
    assert!(positive >= 1, "at least one country produces supplies in the real data");
    assert!(distinct.len() >= 2, "countries keep their own values");
}

/// 実産出と備蓄の純増減の区別: 生産後に日次消費で備蓄が減っても、実績は変わらない。
#[test]
fn daily_consumption_after_production_changes_the_stock_but_not_the_reported_output() {
    let dir = TempTestDir::new("net");
    let mut app = setup_app_in_playing(CountryId(2), dir.save_file_config());
    tick_month(&mut app, 1800, 2);
    let reported = *app
        .world()
        .resource::<MilitarySupplyProductionLedger>()
        .get(CountryId(2))
        .unwrap();
    let stock_after_production = supplies_units_of_all(&app)
        .into_iter()
        .find(|(id, _)| *id == CountryId(2))
        .unwrap()
        .1;
    for day in 2..=10u8 {
        tick_day(&mut app, 1800, 2, day);
    }
    let stock_after_days = supplies_units_of_all(&app)
        .into_iter()
        .find(|(id, _)| *id == CountryId(2))
        .unwrap()
        .1;
    assert!(
        stock_after_days < stock_after_production,
        "daily supply consumption lowers the stock ({stock_after_production} -> {stock_after_days})"
    );
    let still = *app
        .world()
        .resource::<MilitarySupplyProductionLedger>()
        .get(CountryId(2))
        .unwrap();
    assert_eq!(still, reported, "the monthly production record is not netted against consumption");
}

/// ロード成功でledgerは空に戻る(未集計)。次の月次生産で再び記録される。
/// ledgerはSave V1へ含まれないため、セーブ・ロードの前後で永続状態は増えない。
#[test]
fn load_clears_the_ledger_until_the_next_monthly_production() {
    let dir = TempTestDir::new("load");
    let mut app = setup_app_in_playing(CountryId(2), dir.save_file_config());
    tick_month(&mut app, 1800, 2);
    assert!(!app.world().resource::<MilitarySupplyProductionLedger>().is_empty());
    send_save_request(&mut app);
    assert!(dir.0.join("savegame_v1.ron").exists(), "temp save written (not the user's save)");

    tick_month(&mut app, 1800, 3);
    let march = *app
        .world()
        .resource::<MilitarySupplyProductionLedger>()
        .get(CountryId(2))
        .unwrap();
    assert_eq!((march.year, march.month), (1800, 3));

    send_load_request(&mut app);
    assert!(
        app.world().resource::<MilitarySupplyProductionLedger>().is_empty(),
        "no stale record from before the load survives (未集計 until the next production)"
    );
    tick_month(&mut app, 1800, 3);
    let again = *app
        .world()
        .resource::<MilitarySupplyProductionLedger>()
        .get(CountryId(2))
        .unwrap();
    assert_eq!((again.year, again.month), (1800, 3));
}

/// New Game(国家選択画面への進入)でledgerは空に戻る。
#[test]
fn entering_country_selection_clears_the_ledger() {
    let dir = TempTestDir::new("newgame");
    let mut app = setup_app_in_playing(CountryId(2), dir.save_file_config());
    tick_month(&mut app, 1800, 2);
    assert!(!app.world().resource::<MilitarySupplyProductionLedger>().is_empty());
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::CountrySelection);
    app.update();
    app.update();
    assert!(app.world().resource::<MilitarySupplyProductionLedger>().is_empty());
}
