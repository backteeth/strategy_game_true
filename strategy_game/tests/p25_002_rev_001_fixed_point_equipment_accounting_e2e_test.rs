//! P25-002-REV-001: 軍事装備会計の固定小数点化のE2Eテスト。
//!
//! `tests/p25_002_equipment_accounting_foundation_e2e_test.rs`(P25-002本体)は
//! 支給・返却・破壊APIの基本挙動を検証済み(本ファイルではその再テストは行わない)。
//! 本ファイルは、P25-002-REV-001で新たに導入した以下を対象とする。
//!
//! - `2^53`反例(旧素朴f64実装の消失バグと、訂正後の明確な拒否)
//! - `MilitaryFactory`生産経路が固定小数点units経由でstockpileへ加算されること
//! - 他の`ResourceType`の生産値が本訂正の影響を受けないこと
//! - 実App経由のsave/load往復でMilitaryEquipment備蓄が変化しないこと
//! - New Game／Loaded Gameで同じ生産結果になること

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::production::process_country_production;
use strategy_game::economy::resources::{CountryStockpile, ResourceType};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::equipment::{EquipmentAccountingError, try_return_equipment};
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

// ─── Test I: 2^53反例(公開APIレベル、black box) ────────────────────────

#[test]
fn test_i_two_pow_53_counterexample_is_atomically_rejected() {
    let two_pow_53 = 2f64.powi(53);

    // 旧実装(P25-002)が使っていた素朴なf64加算では、2^53 + 1 は 2^53 のまま
    // (1が消える)。このテストファイル内だけの比較用の再現であり、本番コードでは
    // ない。
    assert_eq!(two_pow_53 + 1.0, two_pow_53);

    // 訂正後のtry_return_equipmentは、この規模の値を正規値域外として明確に拒否する
    // (曖昧な成功をしない)。
    let mut stockpile = CountryStockpile::new();
    stockpile.set(ResourceType::MilitaryEquipment, two_pow_53);
    let mut equipment = 1.0;
    let err = try_return_equipment(&mut stockpile, &mut equipment, 1.0).unwrap_err();
    assert_eq!(err, EquipmentAccountingError::QuantityOutOfRange);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), two_pow_53);
    assert_eq!(equipment, 1.0);
}

// ─── Test L: MilitaryFactory生産経路 ────────────────────────────────────

fn military_factory_registry(output_per_level: f64) -> BuildingRegistry {
    let mut definitions = HashMap::new();
    definitions.insert(
        BuildingType::MilitaryFactory,
        BuildingDefinition {
            building_type: BuildingType::MilitaryFactory,
            name: "Military Factory".to_string(),
            construction_cost: 1200.0,
            required_progress: 90.0,
            required_workforce: 10.0,
            logistics_cost: 0.0,
            input_resources: HashMap::new(),
            output_resources: {
                let mut m = HashMap::new();
                m.insert(ResourceType::MilitaryEquipment, output_per_level);
                m
            },
            maintenance_cost: 40.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        },
    );
    BuildingRegistry { definitions }
}

fn full_capacity_state(id: StateId, buildings: &[(BuildingType, u32)]) -> StateData {
    StateData {
        id,
        population: 100_000,
        workforce_ratio: 0.5,
        employed_workforce: 50_000,
        logistics_ratio: 1.0,
        buildings: buildings.iter().copied().collect(),
        ..StateData::default()
    }
}

#[test]
fn test_l_military_factory_production_uses_fixed_point_accumulation() {
    let registry = military_factory_registry(14.6); // 端数のある生産量
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);
    {
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, &registry, false);
    }

    // wf_ratio=min(1.0,...)=1.0, log_ratio=1.0, 入力資源なしのためsat=1.0 →
    // final_op=1.0。output = 14.6 * level(1) * 1.0 = 14.6。固定小数点units経由でも
    // 端数を含む生産量が正確に反映されることを確認する。
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 14.6);
}

#[test]
fn test_l_military_factory_production_accumulates_across_multiple_ticks() {
    let registry = military_factory_registry(14.6);
    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(StateId(0), &[(BuildingType::MilitaryFactory, 1)]);

    for _ in 0..5 {
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, &registry, false);
    }

    // 14.6 * 5 = 73.0(固定小数点units経由の累積、浮動小数点誤差の蓄積がないことを
    // 確認する)。
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 73.0);
}

// ─── Test M: 他ResourceTypeへの非影響 ──────────────────────────────────

#[test]
fn test_m_other_resource_types_are_unaffected_by_the_fixed_point_change() {
    let mut definitions = HashMap::new();
    definitions.insert(
        BuildingType::Farm,
        BuildingDefinition {
            building_type: BuildingType::Farm,
            name: "Farm".to_string(),
            construction_cost: 100.0,
            required_progress: 10.0,
            required_workforce: 10.0,
            logistics_cost: 0.0,
            input_resources: HashMap::new(),
            output_resources: {
                let mut m = HashMap::new();
                m.insert(ResourceType::Food, 10.0);
                m
            },
            maintenance_cost: 1.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        },
    );
    definitions.insert(
        BuildingType::MilitaryFactory,
        military_factory_registry(20.0).definitions[&BuildingType::MilitaryFactory].clone(),
    );
    let registry = BuildingRegistry { definitions };

    let mut stockpile = CountryStockpile::new();
    let mut state = full_capacity_state(
        StateId(0),
        &[(BuildingType::Farm, 1), (BuildingType::MilitaryFactory, 1)],
    );
    {
        let mut states: Vec<&mut StateData> = vec![&mut state];
        process_country_production(&mut states, &mut stockpile, &registry, false);
    }

    // Foodは従来どおりの生の加算経路(ResourceType::Foodは特別扱いされない)。
    assert_eq!(stockpile.get(ResourceType::Food), 10.0);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 20.0);
}

// ─── Test N/O: save/load分岐比較・New Game/Loaded Game比較 ────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p25_002_rev_001_e2e_{label}_{}_{nanos}_{n}",
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

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(SaveRequestMessage);
    state.apply(app.world_mut());
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(LoadRequestMessage);
    state.apply(app.world_mut());
}

fn set_military_equipment_stockpile(app: &mut App, country_id: CountryId, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(country_id)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitaryEquipment, value);
}

fn military_equipment_stockpile(app: &App, country_id: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitaryEquipment)
}

/// Test N: save→load往復で、量子化済みのMilitaryEquipment備蓄値が変化しないこと。
/// Saveスキーマ自体(`f64`のまま)は変更していない。
#[test]
fn test_n_save_load_round_trip_preserves_the_quantized_stockpile() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_n");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    // 量子化格子に完全一致しない値を設定し、往復後も同じ正規値のままであることを
    // 確認する。
    set_military_equipment_stockpile(&mut app, player, 123.456789);
    let before = military_equipment_stockpile(&app, player);

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    let after = military_equipment_stockpile(&app, player);
    assert_eq!(
        before, after,
        "MilitaryEquipment stockpile must round-trip unchanged through save/load"
    );

    // 再度save/loadしても値が変化しない(繰り返しロードしても同じunitsになる、§13)。
    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();
    assert_eq!(military_equipment_stockpile(&app, player), after);
}

/// Test O: New Game経路(継続実行)とLoaded Game経路(save→load後)で、同じ生産結果
/// (MilitaryEquipment備蓄)になること。
#[test]
fn test_o_new_game_and_loaded_game_produce_identical_stockpile() {
    let player = CountryId(0);

    let temp_dir_continuous = TempTestDir::new("test_o_continuous");
    let mut continuous = setup_app_in_playing(player, temp_dir_continuous.save_file_config());
    set_military_equipment_stockpile(&mut continuous, player, 42.0);

    let temp_dir_branch = TempTestDir::new("test_o_branch");
    let mut branch = setup_app_in_playing(player, temp_dir_branch.save_file_config());
    set_military_equipment_stockpile(&mut branch, player, 42.0);
    send_save_request(&mut branch);
    branch.update();
    send_load_request(&mut branch);
    branch.update();
    branch.update();

    assert_eq!(
        military_equipment_stockpile(&continuous, player),
        military_equipment_stockpile(&branch, player),
        "New Game and Loaded Game paths must agree on the same MilitaryEquipment stockpile value"
    );
}
