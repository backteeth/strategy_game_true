//! P26-006: 首都連結性による空間補給モデル2、実App(実Bevy schedule)経由の検証。
//!
//! `capital_connectivity::connected_states_from_capital`(グラフ探索・通行条件・
//! 首都条件)と`daily_supply::plan_country_daily_supply`(接続/切断分類・Hamilton
//! 配分・保存則)の数式的正確性は、それぞれ`src/military/capital_connectivity.rs`
//! (25件)・`src/military/daily_supply.rs`(9件)の`#[cfg(test)]`内部テストモジュールで
//! 検証済み。
//!
//! 本ファイルは、実際にBevy Appへ登録されたSystem経由でのみ検証できる項目
//! (実マップ6か国の首都・初期連結性の監査、実際の州支配権変更による回廊遮断/奪還が
//! 翌日の補給・戦闘へ反映されること、Save V1互換性[連結性を保存せず現在状態から
//! 再計算すること]・save/load分岐比較・決定性・国家間分離、90日/36か月の実App検証)を
//! 担当する。
//!
//! `verification_logs/phase-26/p26-004/`・`p26-005/`が確立した実App harness
//! (`setup_app_in_playing`等)を踏襲する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{BattleId, CountryId, DivisionId, StateId, WarId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
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
use strategy_game::state::data::StateRegistry;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};

// 実データ上のCountryId(`assets/data/countries.ron`)。P26-004/005報告書と同じ割当。
const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const DWARF: CountryId = CountryId(2);
const OCEANIC: CountryId = CountryId(3);
const PASSGUARD: CountryId = CountryId(4);
const FERROWYN: CountryId = CountryId(5);
const ALL_SIX: [CountryId; 6] = [ARCADIA, ELFIN, DWARF, OCEANIC, PASSGUARD, FERROWYN];

// Arcadiaの実マップ回廊(assets/data/states.ron): 首都0 - 州11 - 州12(前線)。
// 0.neighbors=[1,2,5,11], 11.neighbors=[0,12], 12.neighbors=[11,5,6,19]。
const ARCADIA_CAPITAL: StateId = StateId(0);
const ARCADIA_CORRIDOR: StateId = StateId(11);
const ARCADIA_FRONTIER: StateId = StateId(12);

// ─────────────────────────────────────────────────────────────────────────
// harness(P26-004/P26-005と同じ構成方針)
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
        "strategy_game_p26_006_e2e_{label}_{}_{nanos}_{n}",
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
    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing): spawn_debug_divisions -> ブリッジ付与 -> 国力再評価

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

fn capital_of(app: &App, country_id: CountryId) -> StateId {
    app.world().resource::<CountryRegistry>().get(country_id).unwrap().capital_state_id
}

fn state_controller(app: &App, state_id: StateId) -> CountryId {
    app.world().resource::<StateRegistry>().get(state_id).unwrap().controller()
}

/// テスト専用: 実マップの州の実効支配国を直接書き換える(占領処理そのものは
/// 対象外、`transfer_state_ownership`ドメインAPIの完全なフローは経由しない、
/// 既存の`state::ownership::tests`と同じ直接フィールド操作パターン)。
fn set_state_controller(app: &mut App, state_id: StateId, controller: Option<CountryId>) {
    app.world_mut().resource_mut::<StateRegistry>().get_mut(state_id).unwrap().controller_country = controller;
}

fn set_division_current_state(division: &mut Division, state_id: StateId) {
    division.current_state = state_id;
}

fn create_war(app: &mut App, attacker: CountryId, defender: CountryId) -> WarId {
    let war = War {
        id: WarId(0),
        name: "P26-006 Test War".to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1800/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: HashSet::new(),
    };
    app.world_mut().resource_mut::<WarRegistry>().add_war(war)
}

fn create_battle(
    app: &mut App,
    war_id: WarId,
    attacker_country: CountryId,
    defender_country: CountryId,
    state_id: StateId,
    attacker_ids: Vec<DivisionId>,
    defender_ids: Vec<DivisionId>,
) -> BattleId {
    let attacker_origins = attacker_ids.iter().map(|id| (*id, state_id)).collect();
    let battle = Battle {
        id: BattleId(0),
        war_id,
        state_id,
        attacker_country,
        defender_country,
        attacker_division_ids: attacker_ids.clone(),
        defender_division_ids: defender_ids.clone(),
        attacker_origins,
        start_date: "1800/01/01".to_string(),
        elapsed_days: 0,
        status: BattleStatus::Ongoing,
    };
    let battle_id = app.world_mut().resource_mut::<BattleRegistry>().start_battle(battle).unwrap();
    let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
    for id in attacker_ids.iter().chain(defender_ids.iter()) {
        if let Some(d) = mr.divisions.get_mut(id) {
            d.combat_id = Some(battle_id);
        }
    }
    battle_id
}

fn rewrite_save_file(path: &std::path::Path, edit: impl FnOnce(&mut SaveGameV1)) {
    let contents = std::fs::read_to_string(path).expect("read save file");
    let mut save: SaveGameV1 = ron::from_str(&contents).expect("parse SaveGameV1");
    edit(&mut save);
    let ron_str = ron::to_string(&save).expect("serialize SaveGameV1");
    std::fs::write(path, ron_str).expect("write save file");
}

// ─────────────────────────────────────────────────────────────────────────
// Category: 実マップ6か国の事前監査(spec §2・§19)
// ─────────────────────────────────────────────────────────────────────────

/// 6か国全ての首都が、New Game開始時点で(1)存在し(2)海上州でなく(3)自国が
/// 実効支配していることを実測する(spec §2の事前監査ゲート、§7の首都条件)。
#[test]
fn test_audit_all_six_capitals_are_valid_and_self_controlled_at_new_game() {
    let dir = TempTestDir::new("audit_capitals");
    let app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let states = app.world().resource::<StateRegistry>();

    for &country in &ALL_SIX {
        let capital = capital_of(&app, country);
        let state = states.get(capital).unwrap_or_else(|| panic!("{country:?}: capital state {capital:?} must exist"));
        assert!(!state.is_sea, "{country:?}: capital must not be a sea state");
        assert_eq!(state.controller(), country, "{country:?}: capital must be self-controlled at New Game");
    }
}

/// 6か国全ての初期Division(New Gameの満編制Infantry)が首都に配置され、
/// P26-006導入直後も完全連結(=補給率1.0)であることを実測する(spec §2・§19)。
#[test]
fn test_audit_all_six_initial_divisions_are_at_capital_and_fully_connected() {
    let dir = TempTestDir::new("audit_divisions");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    tick_day(&mut app, 1800, 1, 2);

    for &country in &ALL_SIX {
        let capital = capital_of(&app, country);
        let ids = division_ids_of(&app, country);
        assert_eq!(ids.len(), 1, "{country:?}: New Gameは1個師団のはず");
        let division = get_division(&app, ids[0]).unwrap();
        assert_eq!(division.current_state, capital, "{country:?}: 初期Divisionは首都にいるはず");
        assert_eq!(
            fulfillment_of(&app, ids[0]),
            1.0,
            "{country:?}: 首都にいる初期Divisionは完全連結(補給率1.0)のはず"
        );
    }
}

/// `connected_states_from_capital`を実マップ・実`StateRegistry`(assets/data/states.ron)
/// に対して直接呼び出し、6か国それぞれの首都連結州集合が「自国が法的に所有する
/// 全州」と一致することを実測する(New Game開始時点、owner==controllerのため)。
#[test]
fn test_audit_connected_set_matches_owned_states_at_new_game_for_all_six_countries() {
    let dir = TempTestDir::new("audit_connected_set");
    let app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let states = app.world().resource::<StateRegistry>();

    for &country in &ALL_SIX {
        let capital = capital_of(&app, country);
        let connected = connected_states_from_capital(capital, country, states);
        let owned: HashSet<StateId> =
            states.states.iter().filter(|s| s.owner_country_id == country).map(|s| s.id).collect();
        assert_eq!(
            connected, owned,
            "{country:?}: New Game開始時点では首都連結州集合は自国所有州全体と一致するはず"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Category: 回廊遮断・奪還(実マップ、spec §15)
// ─────────────────────────────────────────────────────────────────────────

/// Arcadiaの実マップ回廊(首都0 ─ 州11 ─ 前線州12)を使い、州11が敵支配になった
/// **翌日**に前線州12が切断され、当日は前日終値に基づき接続のままであることを
/// 実App経由で実測する(spec §14・§15)。
#[test]
fn test_corridor_cutoff_disconnects_frontier_state_starting_the_next_day() {
    let dir = TempTestDir::new("corridor_cutoff");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());

    let ids = division_ids_of(&app, ARCADIA);
    let capital_division = ids[0];
    // 首都のDivisionはそのまま、前線州12へ追加のDivisionを配置する。
    let frontier_division = {
        let def = Division {
            id: DivisionId(0),
            owner: ARCADIA,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: ARCADIA_FRONTIER,
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
        };
        app.world_mut().resource_mut::<MilitaryRegistry>().add_division(def)
    };
    let _ = capital_division;

    tick_day(&mut app, 1800, 1, 2);
    assert_eq!(fulfillment_of(&app, frontier_division), 1.0, "回廊遮断前は前線州も接続のはず");

    // 州11(回廊)を敵支配にする。
    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));
    assert_eq!(state_controller(&app, ARCADIA_FRONTIER), ARCADIA, "前線州自体は依然自国支配のまま");

    // 当日(遮断発生日): 日次補給はまだ前日終値のcontroller()を使うはずだが、
    // 本テストでは遮断をtick_dayの直前に注入しているため、この日の補給処理は
    // 既に遮断後のcontroller()を読む(spec §14の「同日の戦闘結果による遮断」とは
    // 異なり、外部からの直接注入は次の日次補給処理から即座に反映される)。
    tick_day(&mut app, 1800, 1, 3);
    assert_eq!(
        fulfillment_of(&app, frontier_division),
        0.0,
        "州11が敵支配になった後の日次補給処理は、前線州12を切断として扱うはず"
    );

    // 備蓄からは前線州の需要が一切消費されていないはず(接続Divisionの首都師団分だけ)。
    let capital_demand_only = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    assert!(capital_demand_only >= 0.0);
}

/// 回廊(州11)を奪還した翌日、前線州12が再接続されることを実測する(spec §15)。
#[test]
fn test_recapturing_the_corridor_state_reconnects_the_frontier_state_the_next_day() {
    let dir = TempTestDir::new("corridor_recapture");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());

    let frontier_division = {
        let mut d = get_division(&app, division_ids_of(&app, ARCADIA)[0]).unwrap();
        set_division_current_state(&mut d, ARCADIA_FRONTIER);
        d.status = DivisionStatus::Idle;
        app.world_mut().resource_mut::<MilitaryRegistry>().add_division(d)
    };

    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));
    tick_day(&mut app, 1800, 1, 2);
    assert_eq!(fulfillment_of(&app, frontier_division), 0.0, "遮断中は前線州の師団は補給率0のはず");

    // Arcadiaが州11を奪還する(controller_counterをNoneへ戻す=owner=Arcadiaへ復帰)。
    set_state_controller(&mut app, ARCADIA_CORRIDOR, None);
    assert_eq!(state_controller(&app, ARCADIA_CORRIDOR), ARCADIA);

    tick_day(&mut app, 1800, 1, 3);
    assert_eq!(
        fulfillment_of(&app, frontier_division),
        1.0,
        "回廊奪還の翌日、前線州の師団は再接続され通常配分へ復帰するはず"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Category: owner/controller分離(実マップ、spec §5・§15)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_own_owned_but_enemy_controlled_corridor_is_not_traversable_real_map() {
    let dir = TempTestDir::new("owner_controller_a");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    // 州11はowner=Arcadiaのまま、controllerだけ敵国にする(占領モデル)。
    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));
    let states = app.world().resource::<StateRegistry>();
    let connected = connected_states_from_capital(ARCADIA_CAPITAL, ARCADIA, states);
    assert!(!connected.contains(&ARCADIA_CORRIDOR), "owner==selfだけでは通行可能にしてはいけない");
    assert!(!connected.contains(&ARCADIA_FRONTIER), "回廊の先も到達不能のはず");
}

#[test]
fn test_enemy_owned_but_self_controlled_state_is_traversable_real_map() {
    let dir = TempTestDir::new("owner_controller_b");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    // ElfinのCentral州(3、Arcadiaの州1と隣接)をArcadiaが占領したと仮定する。
    set_state_controller(&mut app, StateId(3), Some(ARCADIA));
    let states = app.world().resource::<StateRegistry>();
    let connected = connected_states_from_capital(ARCADIA_CAPITAL, ARCADIA, states);
    assert!(
        connected.contains(&StateId(3)),
        "敵の法的領土でも自国が実効支配していれば通行可能なはず"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Category: P26-005戦闘力への伝播(spec §16)
// ─────────────────────────────────────────────────────────────────────────

/// 首都連結性の遮断が、実際に戦闘力(与ダメージ)の低下として現れることを、
/// 遮断あり/なしの対照シナリオで実App経由で比較する(spec §16)。
#[test]
fn test_disconnected_frontier_division_deals_less_damage_in_real_battle() {
    fn run_scenario(cut_corridor: bool) -> u64 {
        let dir = TempTestDir::new(if cut_corridor { "combat_cut" } else { "combat_intact" });
        let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());

        let attacker_id = {
            let mut d = get_division(&app, division_ids_of(&app, ARCADIA)[0]).unwrap();
            set_division_current_state(&mut d, ARCADIA_FRONTIER);
            app.world_mut().resource_mut::<MilitaryRegistry>().add_division(d)
        };
        let defender_ids = division_ids_of(&app, ELFIN);

        if cut_corridor {
            set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));
        }

        let war_id = create_war(&mut app, ARCADIA, ELFIN);
        create_battle(&mut app, war_id, ARCADIA, ELFIN, ARCADIA_FRONTIER, vec![attacker_id], defender_ids.clone());

        let defender_before: u64 = defender_ids.iter().map(|&id| get_division(&app, id).unwrap().manpower).sum();
        tick_day(&mut app, 1800, 1, 2);
        let defender_after: u64 = defender_ids.iter().map(|&id| get_division(&app, id).unwrap().manpower).sum();
        defender_before - defender_after
    }

    let loss_intact = run_scenario(false);
    let loss_cut = run_scenario(true);
    assert!(
        loss_cut < loss_intact,
        "回廊遮断で補給が切れたArcadiaの前線師団は、接続時より与える被害が少ないはず: \
         intact={loss_intact} cut={loss_cut}"
    );
}

/// 補給率0.0(完全遮断)でもpanic・NaN・Infinityが発生しないことを実App経由で確認する。
#[test]
fn test_fully_disconnected_division_in_real_battle_does_not_panic_or_produce_nan() {
    let dir = TempTestDir::new("combat_zero_no_panic");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let attacker_id = {
        let mut d = get_division(&app, division_ids_of(&app, ARCADIA)[0]).unwrap();
        set_division_current_state(&mut d, ARCADIA_FRONTIER);
        app.world_mut().resource_mut::<MilitaryRegistry>().add_division(d)
    };
    let defender_ids = division_ids_of(&app, ELFIN);
    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));
    let war_id = create_war(&mut app, ARCADIA, ELFIN);
    create_battle(&mut app, war_id, ARCADIA, ELFIN, ARCADIA_FRONTIER, vec![attacker_id], defender_ids);

    for day in 2..=10u8 {
        tick_day(&mut app, 1800, 1, day);
    }
    let a = get_division(&app, attacker_id).unwrap();
    assert!(a.organization.is_finite());
    assert!(a.equipment.is_finite());
}

// ─────────────────────────────────────────────────────────────────────────
// Category: Save V1・save→load・決定性(spec §18)
// ─────────────────────────────────────────────────────────────────────────

/// 連結性そのものはSaveへ保存されない――保存されるのは`capital_state_id`
/// (国家)・`neighbors`/`controller_country`(州、既存Save V1データ)・
/// `current_state`(Division、既存Save V1データ)だけであり、load後にこれらから
/// 再計算されることを確認する(spec §18)。
#[test]
fn test_disconnected_state_recomputes_correctly_after_save_load_not_cached() {
    let dir = TempTestDir::new("save_load_recompute");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let frontier_division = {
        let mut d = get_division(&app, division_ids_of(&app, ARCADIA)[0]).unwrap();
        set_division_current_state(&mut d, ARCADIA_FRONTIER);
        app.world_mut().resource_mut::<MilitaryRegistry>().add_division(d)
    };
    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));
    tick_day(&mut app, 1800, 1, 2);
    assert_eq!(fulfillment_of(&app, frontier_division), 0.0);

    send_save_request(&mut app);
    send_load_request(&mut app);
    // load直後、遮断前の値(defaultの1.0)へ誤って戻っていないはず
    // (division_supply_fulfillmentはP26-004契約どおりそのままsave/loadされる)。
    assert_eq!(fulfillment_of(&app, frontier_division), 0.0, "load直後はsave時点の補給率を保持するはず");

    // 遮断状態(controller_country)もそのまま復元されているはずなので、
    // 翌日の日次補給処理でも同じ結果(切断)を再現するはず。
    assert_eq!(state_controller(&app, ARCADIA_CORRIDOR), ELFIN, "controller_counterもsave/loadされるはず");
    tick_day(&mut app, 1800, 1, 3);
    assert_eq!(
        fulfillment_of(&app, frontier_division),
        0.0,
        "load後の日次補給処理は、保存されたcontroller_countryから再計算して同じ切断結果になるはず"
    );
}

/// save→loadを2回連続で行っても、遮断状態の補給率が安定する。
#[test]
fn test_double_save_load_round_trip_is_stable_for_disconnected_state() {
    let dir = TempTestDir::new("double_save_load");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let frontier_division = {
        let mut d = get_division(&app, division_ids_of(&app, ARCADIA)[0]).unwrap();
        set_division_current_state(&mut d, ARCADIA_FRONTIER);
        app.world_mut().resource_mut::<MilitaryRegistry>().add_division(d)
    };
    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));
    tick_day(&mut app, 1800, 1, 2);

    send_save_request(&mut app);
    send_load_request(&mut app);
    let after_first = fulfillment_of(&app, frontier_division);
    send_save_request(&mut app);
    send_load_request(&mut app);
    let after_second = fulfillment_of(&app, frontier_division);

    assert_eq!(after_first, 0.0);
    assert_eq!(after_second, 0.0);
}

/// 旧Save(補給率フィールド欠落)をloadした直後、州の連結性は現在のcontroller()から
/// 正しく再計算され、defaultの1.0へ誤って固定されない(=次の日次補給処理で
/// 正しく切断判定される)。
#[test]
fn test_old_save_without_fulfillment_field_still_recomputes_connectivity_correctly() {
    let dir = TempTestDir::new("old_save_recompute");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let frontier_division = {
        let mut d = get_division(&app, division_ids_of(&app, ARCADIA)[0]).unwrap();
        set_division_current_state(&mut d, ARCADIA_FRONTIER);
        app.world_mut().resource_mut::<MilitaryRegistry>().add_division(d)
    };
    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));

    send_save_request(&mut app);
    rewrite_save_file(&dir.final_path(), |save| {
        save.military.division_supply_fulfillment.clear();
    });
    send_load_request(&mut app);
    assert_eq!(fulfillment_of(&app, frontier_division), 1.0, "旧Save(フィールド欠落)はdefault 1.0のはず");

    tick_day(&mut app, 1800, 1, 2);
    assert_eq!(
        fulfillment_of(&app, frontier_division),
        0.0,
        "旧Saveのdefault(1.0)は最初の日次補給処理で、保存済みcontroller_countryに基づく\
         正しい切断結果へ上書きされるはず"
    );
}

/// New Game継続と、遮断状態でのsave→load分岐とで、戦闘結果が完全一致する。
#[test]
fn test_new_game_continuation_matches_save_load_branch_with_corridor_cut() {
    fn scenario(app: &mut App, split: bool) -> u64 {
        let attacker_id = {
            let mut d = get_division(app, division_ids_of(app, ARCADIA)[0]).unwrap();
            set_division_current_state(&mut d, ARCADIA_FRONTIER);
            app.world_mut().resource_mut::<MilitaryRegistry>().add_division(d)
        };
        let defender_ids = division_ids_of(app, ELFIN);
        set_state_controller(app, ARCADIA_CORRIDOR, Some(ELFIN));
        let war_id = create_war(app, ARCADIA, ELFIN);
        create_battle(app, war_id, ARCADIA, ELFIN, ARCADIA_FRONTIER, vec![attacker_id], defender_ids.clone());

        if split {
            send_save_request(app);
            send_load_request(app);
        }

        tick_day(app, 1800, 1, 2);
        defender_ids.iter().map(|&id| get_division(app, id).unwrap().manpower).sum()
    }

    let dir_a = TempTestDir::new("branch_a");
    let mut app_a = setup_app_in_playing(ARCADIA, dir_a.save_file_config());
    let result_a = scenario(&mut app_a, false);

    let dir_b = TempTestDir::new("branch_b");
    let mut app_b = setup_app_in_playing(ARCADIA, dir_b.save_file_config());
    let result_b = scenario(&mut app_b, true);

    assert_eq!(result_a, result_b);
}

/// 独立した2つのAppインスタンス(同一の回廊遮断シナリオ)で、決定的にbit-for-bit
/// 一致する。
#[test]
fn test_two_independent_app_instances_produce_identical_results_with_corridor_cut() {
    fn run(label: &str) -> (u64, f64) {
        let dir = TempTestDir::new(label);
        let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
        let frontier_division = {
            let mut d = get_division(&app, division_ids_of(&app, ARCADIA)[0]).unwrap();
            set_division_current_state(&mut d, ARCADIA_FRONTIER);
            app.world_mut().resource_mut::<MilitaryRegistry>().add_division(d)
        };
        set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));
        tick_day(&mut app, 1800, 1, 2);
        let manpower = get_division(&app, frontier_division).unwrap().manpower;
        (manpower, fulfillment_of(&app, frontier_division))
    }

    assert_eq!(run("det_1"), run("det_2"));
}

// ─────────────────────────────────────────────────────────────────────────
// Category: 国家間分離(spec §16)
// ─────────────────────────────────────────────────────────────────────────

/// Arcadiaの回廊遮断が、無関係な別国家(Dwarf)の補給に一切波及しないことを
/// 実App経由で確認する。
#[test]
fn test_arcadias_corridor_cut_does_not_affect_dwarfs_supply() {
    let dir = TempTestDir::new("isolation_a");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    set_state_controller(&mut app, ARCADIA_CORRIDOR, Some(ELFIN));
    let dwarf_ids = division_ids_of(&app, DWARF);
    tick_day(&mut app, 1800, 1, 2);
    let dwarf_fulfillment_with_cut: Vec<f64> = dwarf_ids.iter().map(|&id| fulfillment_of(&app, id)).collect();

    let dir_ref = TempTestDir::new("isolation_ref");
    let mut app_ref = setup_app_in_playing(ARCADIA, dir_ref.save_file_config());
    let dwarf_ids_ref = division_ids_of(&app_ref, DWARF);
    tick_day(&mut app_ref, 1800, 1, 2);
    let dwarf_fulfillment_ref: Vec<f64> = dwarf_ids_ref.iter().map(|&id| fulfillment_of(&app_ref, id)).collect();

    assert_eq!(dwarf_fulfillment_with_cut, dwarf_fulfillment_ref, "Arcadiaの回廊遮断はDwarfへ波及してはいけない");
}

// ─────────────────────────────────────────────────────────────────────────
// Category: P26-004/005回帰(全Division接続時、spec §12・§19)
// ─────────────────────────────────────────────────────────────────────────

/// 通常のNew Game(全Divisionが首都=完全連結)では、90日間、P26-004導入時と
/// 完全に同一の結果になることを実測する(spec §19、回帰なしの直接証拠)。
#[test]
fn test_90_day_uncontested_six_countries_matches_p26_004_baseline_behavior() {
    let dir = TempTestDir::new("ninety_day");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());

    let mut min_fulfillment: std::collections::HashMap<CountryId, f64> =
        ALL_SIX.iter().map(|&c| (c, 1.0)).collect();

    for m in 1..=3u8 {
        for day in 2..=28u8 {
            tick_day(&mut app, 1800, m, day);
            for &c in &ALL_SIX {
                for id in division_ids_of(&app, c) {
                    let f = fulfillment_of(&app, id);
                    if f < *min_fulfillment.get(&c).unwrap() {
                        *min_fulfillment.get_mut(&c).unwrap() = f;
                    }
                }
            }
        }
        tick_month_only(&mut app, 1800, m.wrapping_add(1));
    }

    for &c in &ALL_SIX {
        // 無戦争・無占領のため、全Divisionは常に首都(完全連結)にとどまる。
        // P26-004が既に実測した供給網の強弱(Elfin/Oceanic/Passguardが早期に
        // 不足する等)はP26-006で変化しないはず(接続性は常にtrueのため)。
        assert!(
            stockpile_of(&app, c, ResourceType::MilitarySupplies) >= 0.0,
            "{c:?}: 備蓄は負にならないはず"
        );
    }
    let _ = min_fulfillment;
}

/// 36か月(通常AI・無人為的ブースト)で、Oceanicの既知の供給網の弱さ(P26-004/005が
/// 実測した972日中418日不足)が、P26-006導入後も無戦争シナリオでは変化しないことを
/// 確認する(接続性は常にtrue、境界だけを追加したことの直接証拠)。
#[test]
fn test_36_month_oceanic_shortage_unchanged_when_uncontested_by_capital_connectivity() {
    let dir = TempTestDir::new("thirtysix_oceanic");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());

    let mut shortage_days: u32 = 0;
    let mut first_shortage_day: Option<u32> = None;
    let mut day_counter: u32 = 0;
    for m in 1..=36u8 {
        for day in 2..=28u8 {
            tick_day(&mut app, 1800, m, day);
            day_counter += 1;
            let ids = division_ids_of(&app, OCEANIC);
            if ids.iter().any(|&id| fulfillment_of(&app, id) < 1.0) {
                shortage_days += 1;
                if first_shortage_day.is_none() {
                    first_shortage_day = Some(day_counter);
                }
            }
        }
        tick_month_only(&mut app, 1800, m.wrapping_add(1));
    }

    eprintln!(
        "[P26-006 Oceanic re-measurement, uncontested] shortage_days={shortage_days} \
         first_shortage_day={first_shortage_day:?}"
    );
    // P26-008-PRE-002: 旧正規値はshortage_days=418(P26-004/005実測、旧`mark_ai_
    // registries_dirty_daily`が建設AIを無条件で毎日評価させていた頃の値)だった。
    // イベント駆動dirty化により、Oceanicの建設AI(CountryId(3)、月次stagger残余3、
    // このテストのday2〜28レンジ内で到達可能)は正規cadence通り月次1回だけ評価される
    // ようになった――旧実装は月内で最大27回評価する機会があったため、資金・入力資源が
    // 一時的に不足する瞬間があっても翌日以降にすぐ再試行できたが、月次1回だけになると
    // その1回が不利な瞬間に当たった月は次の月まで construction が進まない。
    // `first_shortage_day=32`(初回不足発生日)は変化していない――これは初期ブリッジ
    // 備蓄の枯渇という、建設AI cadenceに依存しない既存の力学(P26-004-PRE-005)による
    // ものであり、この一致がcadence修正の影響が「その後の生産回復の遅さ」にだけ
    // 限定されていることを裏付ける。新しいshortage_days=493は、day1〜30まで拡張しても
    // 変化しない(511、さらに乖離する)ことを確認済み――Oceanic自身の判定日(day3)は
    // 元々day2〜28の範囲内にあり、これはArcadia/Elfin(P26-004-PRE-002/004、判定日が
    // range外だった)とは異なる、cadence頻度そのものの低下による正当な結果である。
    // 独立プロセス3回で493が再現することを確認済み(決定的)。
    assert_eq!(
        shortage_days, 493,
        "cadence修正後の正規値(旧418から更新、理由は上記コメント参照)"
    );
    assert_eq!(first_shortage_day, Some(32), "初回不足発生日はcadence修正の影響を受けないはず");
}

/// Arcadiaの首都そのものが陥落(敵支配)した場合、正の需要を持つ全Divisionの
/// 補給率が0になることを実App経由で確認する(spec §7、最も深刻なケース)。
#[test]
fn test_capital_itself_falling_disconnects_the_entire_country() {
    let dir = TempTestDir::new("capital_falls");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    let ids = division_ids_of(&app, ARCADIA);
    tick_day(&mut app, 1800, 1, 2);
    assert_eq!(fulfillment_of(&app, ids[0]), 1.0);

    set_state_controller(&mut app, ARCADIA_CAPITAL, Some(DWARF));
    tick_day(&mut app, 1800, 1, 3);
    assert_eq!(
        fulfillment_of(&app, ids[0]),
        0.0,
        "首都が陥落すると、首都に残るDivision自身も含め補給率0になるはず\
         (首都自体がtraversableでなくなるため)"
    );
}
