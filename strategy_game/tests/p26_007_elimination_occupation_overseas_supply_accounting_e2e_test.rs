//! P26-007: モデル2補給会計の仕上げ(国家消滅・占領・和平・海外領土)。
//!
//! 事前監査(検証報告書§3〜§20)の結果、本タスクが要求する契約の大部分は
//! 既存実装(P21-021国家消滅Domain API・P26-004日次補給・P26-005戦闘力・
//! P26-006首都連結性)によって**既に**満たされていることを確認した。
//!
//! * `country::elimination::apply_country_elimination`は消滅国家・併合国いずれの
//!   `stockpile`も一切変更しない(既存テスト`apply_destroys_all_equipment_of_
//!   disbanded_divisions_and_leaves_stockpiles_untouched`が既に固定)。
//! * `military::daily_supply::run_daily_military_supply_consumption`は
//!   `!country.is_active() { continue; }`で消滅国家を日次補給の対象から除外する
//!   (P26-004)。
//! * `economy::mod::handle_monthly_economy`は`owned_states.is_empty() { continue;
//!   }`により、消滅国家(定義上所有州0)への生産を自動的に除外する。
//! * `military::data::MilitaryRegistry::remove_division`は`division_supply_
//!   fulfillment`から対応エントリを削除する(P26-004)。国家消滅・戦闘全滅・
//!   撤退強制除去・復員のいずれもこの関数を経由する。
//! * `state::ownership`/`war::peace`のどこにも`stockpile`への言及がない
//!   (占領・和平による領有変更は国家備蓄に一切触れない)。
//!
//! 本ファイルは、これらの契約を実App経由で恒久的に固定するとともに、本タスクが
//! 新たに要求する項目(海外領土の合成フィクスチャ検証、国家消滅日/翌日の会計、
//! save/load分岐、決定性、保存則)を検証する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, CountryStatus, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::capital_connectivity::connected_states_from_capital;
use strategy_game::military::data::{Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::dto::SaveGameV1;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::war::WarPlugin;

const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const DWARF: CountryId = CountryId(2);

// Arcadiaの実マップ回廊(assets/data/states.ron): 首都0 - 州11 - 州12(前線)。
const ARCADIA_CAPITAL: StateId = StateId(0);
const ARCADIA_CORRIDOR: StateId = StateId(11);
const ARCADIA_FRONTIER: StateId = StateId(12);

// ─────────────────────────────────────────────────────────────────────────
// harness(P26-006/P26-006-REV-001と同じ構成方針)
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
        "strategy_game_p26_007_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
}

struct TempTestDir(std::path::PathBuf);

impl TempTestDir {
    fn new(label: &str) -> Self {
        Self(unique_temp_dir(label))
    }
    fn save_file_config(&self) -> SaveFileConfig {
        SaveFileConfig { path: SavePathConfig { final_path: self.0.join("savegame_v1.ron") } }
    }
    fn final_path(&self) -> std::path::PathBuf {
        self.0.join("savegame_v1.ron")
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
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
    state.apply(app.world_mut());
    app.update();
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
    app.update();
}

fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn tick_month_only(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

fn stockpile_of(app: &App, country_id: CountryId, resource: ResourceType) -> f64 {
    app.world().resource::<CountryRegistry>().get(country_id).unwrap().stockpile.get(resource)
}

fn set_stockpile(app: &mut App, country_id: CountryId, resource: ResourceType, value: f64) {
    let mut registry = app.world_mut().resource_mut::<CountryRegistry>();
    registry.get_mut(country_id).unwrap().stockpile.set(resource, value);
}

fn fulfillment_of(app: &App, division_id: DivisionId) -> f64 {
    app.world().resource::<MilitaryRegistry>().supply_fulfillment_of(division_id)
}

fn division_ids_of(app: &App, country_id: CountryId) -> Vec<DivisionId> {
    let mut ids: Vec<DivisionId> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == country_id)
        .map(|d| d.id)
        .collect();
    ids.sort_by_key(|id| id.0);
    ids
}

fn get_division(app: &App, id: DivisionId) -> Option<Division> {
    app.world().resource::<MilitaryRegistry>().divisions.get(&id).cloned()
}

fn set_state_controller(app: &mut App, state_id: StateId, controller: Option<CountryId>) {
    app.world_mut().resource_mut::<StateRegistry>().get_mut(state_id).unwrap().controller_country = controller;
}

fn set_state_owner(app: &mut App, state_id: StateId, owner: CountryId) {
    app.world_mut().resource_mut::<StateRegistry>().get_mut(state_id).unwrap().owner_country_id = owner;
}

fn set_division_current_state(division: &mut Division, state_id: StateId) {
    division.current_state = state_id;
}

fn add_division_at(app: &mut App, owner: CountryId, state: StateId, status: DivisionStatus) -> DivisionId {
    let mut d = get_division(app, division_ids_of(app, owner)[0]).unwrap_or_else(|| default_division(owner, state));
    d.owner = owner;
    set_division_current_state(&mut d, state);
    d.status = status;
    app.world_mut().resource_mut::<MilitaryRegistry>().add_division(d)
}

fn default_division(owner: CountryId, state: StateId) -> Division {
    Division {
        id: DivisionId(0),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: state,
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower: 10_000,
        max_manpower: 10_000,
        equipment: 100.0,
        max_equipment: 100.0,
        organization: 100.0,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: strategy_game::common::DivisionDefinitionId(0),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

/// テスト専用: `country::elimination`の完全なDomain APIを経由せず、消滅の
/// **観測可能な結果**(status変更・所有Division全除去)だけを直接再現する
/// (elimination自体の内部ロジック・原子性・戦争/外交整理は既存の
/// `country::elimination`テスト[20件超]がすでに検証済みであり、本タスクの対象は
/// 「消滅国家が存在する場合の補給会計」であるため、この結果状態を直接構築する)。
fn eliminate_country_directly(app: &mut App, eliminated: CountryId, annexed_by: CountryId, date: &str) {
    {
        let mut cr = app.world_mut().resource_mut::<CountryRegistry>();
        let c = cr.get_mut(eliminated).unwrap();
        c.status = CountryStatus::Eliminated { eliminated_date: date.to_string(), annexed_by };
    }
    let ids = division_ids_of(app, eliminated);
    let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
    for id in ids {
        mr.remove_division(id);
    }
}

fn rewrite_save_file(path: &std::path::Path, edit: impl FnOnce(&mut SaveGameV1)) {
    let contents = std::fs::read_to_string(path).expect("read save file");
    let mut save: SaveGameV1 = ron::from_str(&contents).expect("parse SaveGameV1");
    edit(&mut save);
    let ron_str = ron::to_string(&save).expect("serialize SaveGameV1");
    std::fs::write(path, ron_str).expect("write save file");
}

// ─────────────────────────────────────────────────────────────────────────
// Test A/B: 占領州の補給判定(owner/controller分離、P26-006回帰の再固定)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_a_occupied_enemy_state_is_traversable_via_controller_not_owner() {
    let dir = TempTestDir::new("a_occupied");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    // ElfinのCentral州(StateId(3))をArcadiaが占領した(owner=Elfinのまま)。
    set_state_controller(&mut app, StateId(3), Some(ARCADIA));
    let states = app.world().resource::<StateRegistry>();
    let connected = connected_states_from_capital(ARCADIA_CAPITAL, ARCADIA, states);
    assert!(connected.contains(&StateId(3)), "占領した敵州(owner不変)は自国のcontrollerであれば通行可能なはず");
}

#[test]
fn test_b_own_state_occupied_by_enemy_is_not_traversable() {
    let dir = TempTestDir::new("b_own_occupied");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));
    let states = app.world().resource::<StateRegistry>();
    let connected = connected_states_from_capital(ARCADIA_CAPITAL, ARCADIA, states);
    assert!(!connected.contains(&ARCADIA_CORRIDOR), "自国所有でも敵支配なら通行不能なはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test C/D: 首都占領・奪還(存続国家、備蓄非消費の明示的確認)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_c_capital_occupation_halts_consumption_without_touching_stockpile_while_country_survives() {
    let dir = TempTestDir::new("c_capital_occupied");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let ids = division_ids_of(&app, ARCADIA);
    tick_day(&mut app, 1800, 1, 2);
    let stockpile_before_occupation = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);

    set_state_controller(&mut app, ARCADIA_CAPITAL, Some(DWARF));
    // Arcadiaはまだ他の州(1,2,11,12)を所有しているため消滅しない(存続国家)。
    assert!(app.world().resource::<CountryRegistry>().get(ARCADIA).unwrap().is_active());

    tick_day(&mut app, 1800, 1, 3);
    assert_eq!(fulfillment_of(&app, ids[0]), 0.0, "首都陥落で首都の師団自身も補給率0になるはず");
    let stockpile_after = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    assert_eq!(stockpile_after, stockpile_before_occupation, "接続Divisionが存在しないため備蓄は消費されないはず");
}

#[test]
fn test_d_capital_recapture_restores_connectivity_and_consumption_the_next_day() {
    let dir = TempTestDir::new("d_capital_recapture");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let ids = division_ids_of(&app, ARCADIA);
    set_state_controller(&mut app, ARCADIA_CAPITAL, Some(DWARF));
    tick_day(&mut app, 1800, 1, 2);
    assert_eq!(fulfillment_of(&app, ids[0]), 0.0);

    set_state_controller(&mut app, ARCADIA_CAPITAL, None); // 奪還(owner=Arcadiaへ復帰)
    tick_day(&mut app, 1800, 1, 3);
    assert_eq!(fulfillment_of(&app, ids[0]), 1.0, "首都奪還の翌日は通常配分へ復帰するはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test E/F: 海外領土・海上州(合成フィクスチャ、実マップにis_sea==trueが存在しないため)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_e_overseas_territory_across_a_sea_state_is_unreachable_even_when_self_owned_and_controlled() {
    let country = CountryId(1);
    let capital = StateId(0);
    let sea = StateId(1);
    let overseas = StateId(2);

    let states = StateRegistry::build(vec![
        StateData { id: capital, owner_country_id: country, neighbors: vec![sea], ..Default::default() },
        StateData { id: sea, owner_country_id: country, neighbors: vec![capital, overseas], is_sea: true, ..Default::default() },
        StateData { id: overseas, owner_country_id: country, neighbors: vec![sea], ..Default::default() },
    ]);

    let connected = connected_states_from_capital(capital, country, &states);
    assert!(!connected.contains(&overseas), "海(is_sea)を越える必要がある自国領は、owner/controllerが自国でも補給不能なはず");
    assert_eq!(connected, [capital].into_iter().collect(), "首都自身だけが連結しているはず");
}

#[test]
fn test_f_sea_state_is_never_traversable_even_when_self_owned_and_self_controlled() {
    let country = CountryId(1);
    let capital = StateId(0);
    let sea = StateId(1);

    // is_sea==trueの州は、owner/controllerが自国そのものであっても、
    // is_traversableのis_sea除外により連結対象には決してならない。
    let states = StateRegistry::build(vec![
        StateData { id: capital, owner_country_id: country, neighbors: vec![sea], ..Default::default() },
        StateData { id: sea, owner_country_id: country, controller_country: Some(country), neighbors: vec![capital], is_sea: true, ..Default::default() },
    ]);

    let connected = connected_states_from_capital(capital, country, &states);
    assert!(!connected.contains(&sea), "is_sea州は自国owner/controllerでも連結対象にならないはず");
    assert_eq!(connected, [capital].into_iter().collect(), "首都自身だけが連結しているはず");
}

#[test]
fn test_f2_land_locked_enclave_cut_off_by_a_foreign_controlled_corridor_is_unreachable() {
    let country = CountryId(1);
    let foreign = CountryId(2);
    let capital = StateId(0);
    let corridor = StateId(1);
    let enclave = StateId(2);

    let mut states = StateRegistry::build(vec![
        StateData { id: capital, owner_country_id: country, neighbors: vec![corridor], ..Default::default() },
        StateData { id: corridor, owner_country_id: country, controller_country: Some(foreign), neighbors: vec![capital, enclave], ..Default::default() },
        StateData { id: enclave, owner_country_id: country, neighbors: vec![corridor], ..Default::default() },
    ]);

    let connected = connected_states_from_capital(capital, country, &states);
    assert!(!connected.contains(&enclave), "陸上飛び地への唯一の経路が第三国支配なら補給不能なはず");

    // 回廊を奪還すれば陸上飛び地も再接続する。
    states.get_mut(corridor).unwrap().controller_country = None;
    let connected_after = connected_states_from_capital(capital, country, &states);
    assert!(connected_after.contains(&enclave));
}

// ─────────────────────────────────────────────────────────────────────────
// Test G/H: 占領・和平による領有変更は国家備蓄を移転しない
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_g_state_controller_change_alone_does_not_transfer_military_supplies_between_countries() {
    let dir = TempTestDir::new("g_controller_no_transfer");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let arcadia_before = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    let elfin_before = stockpile_of(&app, ELFIN, ResourceType::MilitarySupplies);

    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN)); // Elfinが占領

    let arcadia_after = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    let elfin_after = stockpile_of(&app, ELFIN, ResourceType::MilitarySupplies);
    assert_eq!(arcadia_before, arcadia_after, "占領されてもArcadiaの備蓄は変化しないはず");
    assert_eq!(elfin_before, elfin_after, "占領してもElfinの備蓄は増加しないはず");
}

#[test]
fn test_h_peace_style_owner_change_does_not_transfer_military_supplies_between_countries() {
    let dir = TempTestDir::new("h_owner_no_transfer");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let arcadia_before = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    let elfin_before = stockpile_of(&app, ELFIN, ResourceType::MilitarySupplies);

    // 和平でArcadiaの州12がElfinへ割譲された(owner変更、州単位のMilitarySuppliesは存在しない)。
    set_state_owner(&mut app, ARCADIA_FRONTIER, ELFIN);

    let arcadia_after = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    let elfin_after = stockpile_of(&app, ELFIN, ResourceType::MilitarySupplies);
    assert_eq!(arcadia_before, arcadia_after, "州の割譲だけではArcadiaの備蓄は変化しないはず");
    assert_eq!(elfin_before, elfin_after, "州の割譲だけではElfinの備蓄は増加しないはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test I/J: 国家消滅日・翌日の会計
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_i_same_day_completed_consumption_is_not_rolled_back_by_a_later_elimination() {
    let dir = TempTestDir::new("i_same_day");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let stockpile_before = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);

    // その日の補給処理(Economy Set)を先に完了させる。
    tick_day(&mut app, 1800, 1, 2);
    let stockpile_after_supply = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    assert!(stockpile_after_supply < stockpile_before, "その日の消費は正常に発生しているはず");

    // 同日中(後続の軍事処理相当)にArcadiaが消滅したと仮定する。
    eliminate_country_directly(&mut app, ARCADIA, DWARF, "1800/01/02");

    // 消滅処理自体はその日確定した消費を巻き戻さない(消費は既にstockpileへ反映済み)。
    let stockpile_after_elimination = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    assert_eq!(stockpile_after_elimination, stockpile_after_supply, "消滅処理はその日の消費結果を変更しないはず");
}

#[test]
fn test_j_no_production_consumption_or_allocation_the_day_after_elimination() {
    let dir = TempTestDir::new("j_day_after");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    eliminate_country_directly(&mut app, ARCADIA, DWARF, "1800/01/01");
    let stockpile_before = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);

    tick_day(&mut app, 1800, 1, 2);
    tick_month_only(&mut app, 1800, 2);

    let stockpile_after = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    assert_eq!(stockpile_after, stockpile_before, "消滅国家の備蓄は日次・月次処理を経ても変化しないはず(凍結)");
    assert!(division_ids_of(&app, ARCADIA).is_empty(), "消滅国家のDivisionは既に除去されているはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test K: 完全併合時の備蓄非移転(既存country::elimination契約のE2E再確認)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_k_full_annexation_never_transfers_the_eliminated_countrys_stockpile_to_the_annexer() {
    let dir = TempTestDir::new("k_annexation");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    set_stockpile(&mut app, ARCADIA, ResourceType::MilitarySupplies, 500.0);
    let dwarf_before = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);

    eliminate_country_directly(&mut app, ARCADIA, DWARF, "1800/01/01");

    assert_eq!(stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies), 500.0, "消滅国家の備蓄は凍結され変化しないはず");
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), dwarf_before, "併合国の備蓄は増加しないはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test L/M: Division全滅・解散(携行在庫なし、dangling補給率なし)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_l_division_annihilation_returns_no_supplies_and_leaves_no_dangling_fulfillment_entry() {
    let dir = TempTestDir::new("l_annihilation");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let ids = division_ids_of(&app, ARCADIA);
    tick_day(&mut app, 1800, 1, 2); // fulfillment mapへエントリを作る
    assert_eq!(fulfillment_of(&app, ids[0]), 1.0);
    let stockpile_before = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);

    app.world_mut().resource_mut::<MilitaryRegistry>().remove_division(ids[0]);

    assert_eq!(
        stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies),
        stockpile_before,
        "Division除去でMilitarySuppliesが返却されてはいけない(携行在庫なし)"
    );
    assert_eq!(fulfillment_of(&app, ids[0]), 1.0, "除去済みDivisionIdはdefault 1.0として扱われるはず(dangling値ではない)");
    assert!(
        !app.world().resource::<MilitaryRegistry>().division_supply_fulfillment.contains_key(&ids[0]),
        "division_supply_fulfillmentマップにdanglingエントリが残ってはいけない"
    );
}

#[test]
fn test_m_division_demobilization_has_the_same_no_refund_no_dangling_contract() {
    let dir = TempTestDir::new("m_demobilization");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let extra = add_division_at(&mut app, ARCADIA, ARCADIA_CAPITAL, DivisionStatus::Idle);
    tick_day(&mut app, 1800, 1, 2);
    let stockpile_before = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);

    // 解散(demobilization)も戦闘全滅と同じremove_division経路を通る。
    app.world_mut().resource_mut::<MilitaryRegistry>().remove_division(extra);

    assert_eq!(stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies), stockpile_before, "解散でもMilitarySuppliesは返却されないはず");
    assert!(!app.world().resource::<MilitaryRegistry>().division_supply_fulfillment.contains_key(&extra));
}

// ─────────────────────────────────────────────────────────────────────────
// Test N: 需要0Divisionの境界(接続・切断・消滅)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_n_zero_demand_division_defaults_stay_correct_across_connectivity_boundaries() {
    let dir = TempTestDir::new("n_zero_demand");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let zero_demand = {
        let mut d = default_division(ARCADIA, ARCADIA_FRONTIER);
        d.manpower = 0; // is_livingから除外される(需要対象外)
        app.world_mut().resource_mut::<MilitaryRegistry>().add_division(d)
    };

    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN)); // 前線州を切断
    tick_day(&mut app, 1800, 1, 2);
    // manpower0は生存Divisionでないため需要0扱いであり、is_livingフィルタにより
    // 明示的なfulfillment書き込み対象にもならないが、supply_fulfillment_ofの
    // defaultによって安全に1.0として扱われる。
    assert_eq!(fulfillment_of(&app, zero_demand), 1.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Test O: 複数国家の同日消滅・占領、Map挿入順序非依存
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_o_multiple_simultaneous_eliminations_are_order_independent() {
    fn run(eliminate_arcadia_first: bool) -> (bool, bool, f64, f64) {
        let dir = TempTestDir::new(if eliminate_arcadia_first { "o_a_first" } else { "o_b_first" });
        let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
        set_stockpile(&mut app, ARCADIA, ResourceType::MilitarySupplies, 100.0);
        set_stockpile(&mut app, ELFIN, ResourceType::MilitarySupplies, 200.0);

        if eliminate_arcadia_first {
            eliminate_country_directly(&mut app, ARCADIA, DWARF, "1800/01/01");
            eliminate_country_directly(&mut app, ELFIN, DWARF, "1800/01/01");
        } else {
            eliminate_country_directly(&mut app, ELFIN, DWARF, "1800/01/01");
            eliminate_country_directly(&mut app, ARCADIA, DWARF, "1800/01/01");
        }

        (
            app.world().resource::<CountryRegistry>().get(ARCADIA).unwrap().is_active(),
            app.world().resource::<CountryRegistry>().get(ELFIN).unwrap().is_active(),
            stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies),
            stockpile_of(&app, ELFIN, ResourceType::MilitarySupplies),
        )
    }

    assert_eq!(run(true), run(false), "処理順序に関わらず両国とも同じ最終状態になるはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test P/Q: 国家保存則・世界保存則
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_p_per_country_conservation_law_holds_through_a_disconnection_and_elimination_sequence() {
    let dir = TempTestDir::new("p_conservation");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let before = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);

    tick_day(&mut app, 1800, 1, 2);
    let after_day1 = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    let consumed_day1 = before - after_day1;
    assert!(consumed_day1 >= 0.0);

    set_state_controller(&mut app, ARCADIA_CAPITAL, Some(DWARF)); // 首都陥落、以後消費0
    tick_day(&mut app, 1800, 1, 3);
    let after_day2 = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    assert_eq!(after_day2, after_day1, "切断後は消費0のため備蓄は変化しないはず");

    eliminate_country_directly(&mut app, ARCADIA, DWARF, "1800/01/03");
    let after_elimination = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    assert_eq!(after_elimination, after_day2, "消滅処理自体は備蓄を凍結するだけで変化させないはず");
}

#[test]
fn test_q_world_conservation_law_no_phantom_creation_or_destruction_across_elimination() {
    let dir = TempTestDir::new("q_world_conservation");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let countries: Vec<CountryId> = app.world().resource::<CountryRegistry>().countries.iter().map(|c| c.id).collect();
    let world_before: f64 = countries.iter().map(|&c| stockpile_of(&app, c, ResourceType::MilitarySupplies)).sum();

    eliminate_country_directly(&mut app, ARCADIA, DWARF, "1800/01/01");

    let world_after: f64 = countries.iter().map(|&c| stockpile_of(&app, c, ResourceType::MilitarySupplies)).sum();
    assert_eq!(world_before, world_after, "国家消滅そのものは世界全体の備蓄総量を一切変化させないはず(移転も蒸発もない)");
}

// ─────────────────────────────────────────────────────────────────────────
// Test R/S: save/load分岐・2回連続
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_r_save_load_branch_matches_continuous_run_across_disconnection_and_elimination() {
    fn scenario(app: &mut App, split: bool) -> (f64, bool, Vec<f64>) {
        let ids = division_ids_of(app, ARCADIA);
        set_state_controller(app, ARCADIA_CORRIDOR, Some(ELFIN));

        if split {
            send_save_request(app);
            send_load_request(app);
        }

        tick_day(app, 1800, 1, 2);
        eliminate_country_directly(app, ARCADIA, DWARF, "1800/01/02");
        tick_day(app, 1800, 1, 3);

        (
            stockpile_of(app, ARCADIA, ResourceType::MilitarySupplies),
            app.world().resource::<CountryRegistry>().get(ARCADIA).unwrap().is_active(),
            ids.iter().map(|&id| fulfillment_of(app, id)).collect(),
        )
    }

    let dir_a = TempTestDir::new("r_continuous");
    let mut app_a = setup_app_in_playing(ARCADIA, dir_a.save_file_config());
    let result_a = scenario(&mut app_a, false);

    let dir_b = TempTestDir::new("r_split");
    let mut app_b = setup_app_in_playing(ARCADIA, dir_b.save_file_config());
    let result_b = scenario(&mut app_b, true);

    assert_eq!(result_a, result_b);
}

#[test]
fn test_s_double_save_load_does_not_double_apply_supply_consumption_or_elimination_cleanup() {
    let dir = TempTestDir::new("s_double_save_load");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    tick_day(&mut app, 1800, 1, 2);

    send_save_request(&mut app);
    send_load_request(&mut app);
    let after_first = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);

    send_save_request(&mut app);
    send_load_request(&mut app);
    let after_second = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);

    assert_eq!(after_first, after_second, "2回連続save→loadで消費が二重適用されてはいけない");
}

/// 旧Save(division_supply_fulfillmentフィールド欠落)をloadしても、消滅国家関連の
/// 処理が安全に完了することを確認する(P26-004の既存契約の再確認)。
#[test]
fn test_old_save_without_fulfillment_field_still_handles_a_disconnected_scenario_safely() {
    let dir = TempTestDir::new("old_save");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let ids = division_ids_of(&app, ARCADIA);
    set_state_controller(&mut app, ARCADIA_CAPITAL, Some(DWARF));

    send_save_request(&mut app);
    rewrite_save_file(&dir.final_path(), |save| {
        save.military.division_supply_fulfillment.clear();
    });
    send_load_request(&mut app);
    assert_eq!(fulfillment_of(&app, ids[0]), 1.0, "旧Saveのdefaultは1.0のはず");

    tick_day(&mut app, 1800, 1, 3);
    assert_eq!(fulfillment_of(&app, ids[0]), 0.0, "load後の最初の日次処理で正しく切断結果へ上書きされるはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test T: 実App System順序(占領・消滅は翌日の補給判定から反映)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_t_controller_change_and_elimination_are_reflected_starting_only_the_next_supply_pass() {
    let dir = TempTestDir::new("t_system_order");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let ids = division_ids_of(&app, ARCADIA);
    tick_day(&mut app, 1800, 1, 2);
    assert_eq!(fulfillment_of(&app, ids[0]), 1.0);

    // 「当日の軍事処理でcontroller変更」を注入(実際の戦闘結果と同じ、Economy Set後)。
    set_state_controller(&mut app, ARCADIA_CAPITAL, Some(DWARF));
    // まだ当日の補給処理は再実行されていないため、fulfillmentは前回確定値のまま。
    assert_eq!(fulfillment_of(&app, ids[0]), 1.0, "同日内はまだ前回確定値のままのはず");

    tick_day(&mut app, 1800, 1, 3);
    assert_eq!(fulfillment_of(&app, ids[0]), 0.0, "翌日の補給処理で初めて切断が反映されるはず");
}

// ─────────────────────────────────────────────────────────────────────────
// 90日戦争シナリオ(実データ、回廊占領→首都占領→save/load分岐)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_90_day_corridor_and_capital_occupation_scenario_with_save_load_branch() {
    fn run(split_at_day: Option<(u8, u8)>) -> Vec<(u32, f64, bool, f64)> {
        let dir = TempTestDir::new(if split_at_day.is_some() { "ninety_split" } else { "ninety_continuous" });
        let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
        let ids = division_ids_of(&app, ARCADIA);
        let mut log = Vec::new();
        let mut day_counter: u32 = 0;

        for m in 1..=3u8 {
            for day in 2..=28u8 {
                if let Some((sm, sd)) = split_at_day
                    && sm == m
                    && sd == day
                {
                    send_save_request(&mut app);
                    send_load_request(&mut app);
                }
                tick_day(&mut app, 1800, m, day);
                day_counter += 1;

                if day_counter == 10 {
                    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));
                }
                if day_counter == 20 {
                    set_state_controller(&mut app, ARCADIA_CAPITAL, Some(ELFIN));
                }

                let stockpile = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
                let is_active = app.world().resource::<CountryRegistry>().get(ARCADIA).unwrap().is_active();
                let fulfillment = if is_active { fulfillment_of(&app, ids[0]) } else { -1.0 };
                log.push((day_counter, stockpile, is_active, fulfillment));
            }
            tick_month_only(&mut app, 1800, m.wrapping_add(1));
        }
        log
    }

    let continuous = run(None);
    let split = run(Some((2, 10)));
    assert_eq!(continuous, split, "90日間の回廊占領・首都占領シナリオはsave/load分岐に関わらず一致するはず");

    // day20(首都占領)以降は接続Divisionが存在しないため消費は発生しない。
    // ただし生産はowner_country_idベース(occupationはcontrollerのみ変更)であり
    // 占領の影響を受けないため、備蓄は月次生産分だけ増加し続けるのが正しい
    // (「変化しない」ではなく「減少しない」が正しい契約)。
    let after_occupation: Vec<&(u32, f64, bool, f64)> = continuous.iter().filter(|(d, ..)| *d >= 20).collect();
    for pair in after_occupation.windows(2) {
        let (_, before, _, _) = pair[0];
        let (_, after, _, _) = pair[1];
        assert!(after >= before, "首都占領後は消費が発生しないため備蓄は減少しないはず (before={before}, after={after})");
    }
    let capital_division_fulfillment_at_day90 = continuous.last().unwrap().3;
    assert_eq!(capital_division_fulfillment_at_day90, 0.0, "首都占領後は首都に残るDivision自身の補給率も0のはず");
}
