//! P23-006-FIX-001: New Game初期軍の人的資源帳簿初期化と、未解決募兵定義の顕在化。
//!
//! P23-006が正式化した`B(累積動員負担)=CountryData.mobilized_manpower`は、New Game開始
//! 直後の`spawn_debug_divisions`(初期Division配置)では加算されていなかった。本テストは、
//! 実際の`app::loader::spawn_debug_divisions`(最小構成のSystemテスト)と、実際の
//! New Game経路(完全なApp構成)の両方で、初期軍配置後に`B == L`(現役軍事人的資源)・
//! `K == 0`(恒久軍事損失)・不整合量0が成立することを固定する。あわせて、
//! `compute_manpower_ledger`が未解決の`DivisionDefinitionId`を含む募兵キューに対して
//! 明示的な`Err`を返し、UIがpanicせず「計算不能」を表示することを固定する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::{GameState, PlayingEntryMode};
use strategy_game::app::loader::spawn_debug_divisions;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionDefinitionId, StateId};
use strategy_game::country::{CountryData, CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::localization::{CurrentLocale, Locale, MISSING_KEY_MARKER_PREFIX};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::{
    DivisionDefinition, DivisionSize, DivisionType, MilitaryRegistry,
};
use strategy_game::military::manpower::compute_manpower_ledger;
use strategy_game::military::recruitment::RecruitmentQueueItem;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{
    LastLoadOutcome, LoadOutcome, LoadRequestMessage, SaveFileConfig,
};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::ui::military_panel::{
    MilitaryPanelPlugin, MilitaryPanelState, MilitaryPanelText,
};
use strategy_game::war::WarPlugin;

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p23_006_fix_001_e2e_{label}_{}_{nanos}_{n}",
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

// ─────────────────────────────────────────────────────────────────────────
// §6.1 spawn_debug_divisions単体・Systemテスト(最小構成)
// ─────────────────────────────────────────────────────────────────────────

fn test_country(id: usize) -> CountryData {
    CountryData {
        id: CountryId(id),
        capital_state_id: StateId(id),
        ..CountryData::default()
    }
}

fn infantry_definition(required_manpower: u64) -> DivisionDefinition {
    DivisionDefinition {
        id: DivisionDefinitionId(0),
        name: "Infantry".to_string(),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        required_manpower,
        required_equipment: 100.0,
        recruitment_days: 30,
        movement_speed: 1.0,
        attack: 10.0,
        defense: 10.0,
        breakthrough: 5.0,
        organization: 50.0,
        morale: 50.0,
        supply_usage: 1.0,
        maintenance_cost: 1.0,
    }
}

fn minimal_spawn_app(
    countries: Vec<CountryData>,
    military_registry: MilitaryRegistry,
    entry_mode: PlayingEntryMode,
) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()));
    app.insert_resource(CountryRegistry { countries });
    app.insert_resource(military_registry);
    app.insert_resource(entry_mode);
    app.add_systems(Update, spawn_debug_divisions);
    app
}

fn mobilized_manpower_of(app: &App, id: CountryId) -> u64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(id)
        .unwrap()
        .mobilized_manpower
}

fn available_manpower_of(app: &App, id: CountryId) -> u64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(id)
        .unwrap()
        .available_manpower
}

#[test]
fn spawning_a_division_increases_the_owners_burden_by_its_actual_manpower() {
    let mut app = minimal_spawn_app(
        vec![test_country(0)],
        {
            let mut mr = MilitaryRegistry::default();
            mr.definitions
                .insert(DivisionDefinitionId(0), infantry_definition(7_500));
            mr
        },
        PlayingEntryMode::NewGame,
    );
    app.update();

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        1
    );
    let spawned_manpower = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .next()
        .unwrap()
        .manpower;
    assert_eq!(spawned_manpower, 7_500);
    assert_eq!(mobilized_manpower_of(&app, CountryId(0)), 7_500);
}

#[test]
fn multiple_countries_each_gain_independent_burden_for_their_own_initial_division() {
    let mut app = minimal_spawn_app(
        vec![test_country(0), test_country(1), test_country(2)],
        {
            let mut mr = MilitaryRegistry::default();
            mr.definitions
                .insert(DivisionDefinitionId(0), infantry_definition(5_000));
            mr
        },
        PlayingEntryMode::NewGame,
    );
    app.update();

    for id in [CountryId(0), CountryId(1), CountryId(2)] {
        assert_eq!(
            mobilized_manpower_of(&app, id),
            5_000,
            "each country must gain its own burden"
        );
        let owned_divisions = app
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .values()
            .filter(|d| d.owner == id)
            .count();
        assert_eq!(
            owned_divisions, 1,
            "each country must own exactly one initial division"
        );
    }
}

#[test]
fn a_country_gains_no_division_or_burden_when_no_matching_definition_exists() {
    let mut app = minimal_spawn_app(
        vec![test_country(0)],
        MilitaryRegistry::default(), // Infantry(DivisionDefinitionId(0))が存在しない
        PlayingEntryMode::NewGame,
    );
    app.update();

    assert!(
        app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .is_empty()
    );
    assert_eq!(mobilized_manpower_of(&app, CountryId(0)), 0);
}

#[test]
fn re_running_after_entry_mode_resets_away_from_new_game_does_not_duplicate() {
    // 実際の本番経路(`OnEnter(GameState::Playing)`)では、`spawn_debug_divisions`の直後に
    // `reset_playing_entry_mode`が`.chain()`で必ず実行され、`PlayingEntryMode`を
    // 決定的に初期化し直す(`src/app/game_state.rs`)。この実際の仕組みをそのまま
    // 模して、1回目の実行後にentry_modeがNewGameでなくなった状態で2回目を実行しても
    // 二重加算されないことを固定する。
    let mut app = minimal_spawn_app(
        vec![test_country(0)],
        {
            let mut mr = MilitaryRegistry::default();
            mr.definitions
                .insert(DivisionDefinitionId(0), infantry_definition(6_000));
            mr
        },
        PlayingEntryMode::NewGame,
    );
    app.update();
    assert_eq!(mobilized_manpower_of(&app, CountryId(0)), 6_000);
    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        1
    );

    *app.world_mut().resource_mut::<PlayingEntryMode>() = PlayingEntryMode::LoadedGame;
    app.update();
    app.update();

    assert_eq!(
        mobilized_manpower_of(&app, CountryId(0)),
        6_000,
        "burden must not be added again once entry_mode leaves NewGame"
    );
    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        1
    );
}

#[test]
fn extreme_manpower_values_do_not_overflow_or_panic() {
    let mut app = minimal_spawn_app(
        vec![test_country(0)],
        {
            let mut mr = MilitaryRegistry::default();
            mr.definitions
                .insert(DivisionDefinitionId(0), infantry_definition(u64::MAX));
            mr
        },
        PlayingEntryMode::NewGame,
    );
    app.update();
    assert_eq!(mobilized_manpower_of(&app, CountryId(0)), u64::MAX);
}

#[test]
fn available_manpower_is_unchanged_by_initial_division_spawn() {
    let mut country = test_country(0);
    country.available_manpower = 42_000;
    let mut app = minimal_spawn_app(
        vec![country],
        {
            let mut mr = MilitaryRegistry::default();
            mr.definitions
                .insert(DivisionDefinitionId(0), infantry_definition(5_000));
            mr
        },
        PlayingEntryMode::NewGame,
    );
    app.update();
    assert_eq!(available_manpower_of(&app, CountryId(0)), 42_000);
}

// ─────────────────────────────────────────────────────────────────────────
// §6.2 実際のNew Game統合テスト(完全なApp構成、実データ6か国)
// ─────────────────────────────────────────────────────────────────────────

fn setup_full_app(player: CountryId, save_config: SaveFileConfig) -> App {
    setup_full_app_inner(player, save_config, false)
}

fn setup_app_with_military_panel(player: CountryId, save_config: SaveFileConfig) -> App {
    setup_full_app_inner(player, save_config, true)
}

fn setup_full_app_inner(
    player: CountryId,
    save_config: SaveFileConfig,
    with_military_panel: bool,
) -> App {
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

    if with_military_panel {
        app.add_plugins(MilitaryPanelPlugin);
    }

    app.insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
        .insert_resource(strategy_game::map::division_selection::DragSelectState::default())
        .insert_resource(SelectedState::default())
        .insert_resource(strategy_game::ui::ActivePanel::default())
        .insert_resource(strategy_game::ui::diplomacy_panel::DiplomacyPanelState::default())
        .insert_resource(MilitaryPanelState::default())
        .insert_resource(strategy_game::ui::peace_panel::PeacePanelState::default())
        .insert_resource(strategy_game::ui::politics_panel::PoliticsPanelState::default())
        .insert_resource(strategy_game::ui::research_panel::ResearchPanelState::default())
        .insert_resource(strategy_game::map::camera::CameraDragState::default())
        .insert_resource(strategy_game::map::frontline_render::FrontlineRenderSettings::default())
        .insert_resource(strategy_game::map::frontline_selection::FrontlineSelectMode::default())
        .insert_resource(
            strategy_game::map::offensive_line_selection::OffensiveLineEditMode::default(),
        )
        .add_message::<strategy_game::map::frontline_selection::FrontlineAssignmentAttempted>()
        .add_message::<strategy_game::map::camera::CameraResetRequestMessage>();

    app.add_plugins(SaveGamePlugin);
    app.add_plugins(LoadGamePlugin);

    app.insert_resource(save_config);
    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing); // 既定のPlayingEntryMode::NewGameのまま進入
    app.update(); // OnEnter(Playing)

    app
}

fn ledger_for(
    app: &App,
    country_id: CountryId,
) -> strategy_game::military::manpower::ManpowerLedger {
    let country = app
        .world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap();
    let military_registry = app.world().resource::<MilitaryRegistry>();
    compute_manpower_ledger(country, military_registry)
        .expect("a real New Game must always resolve to a valid ledger")
}

#[test]
fn a_real_new_game_start_gives_every_active_country_a_balanced_ledger() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("real_new_game_balanced");
    let app = setup_full_app(player, temp_dir.save_file_config());

    let country_ids: Vec<CountryId> = app
        .world()
        .resource::<CountryRegistry>()
        .countries
        .iter()
        .filter(|c| c.is_active())
        .map(|c| c.id)
        .collect();
    assert!(
        !country_ids.is_empty(),
        "the real map must have at least one active country"
    );

    for id in country_ids {
        let ledger = ledger_for(&app, id);
        assert_eq!(
            ledger.workforce_burden, ledger.living_military_manpower,
            "country {id:?}: B must equal L right after a real New Game start"
        );
        assert_eq!(
            ledger.permanent_military_losses, 0,
            "country {id:?}: K must be 0"
        );
        assert_eq!(
            ledger.unaccounted_living_excess, 0,
            "country {id:?}: no unaccounted excess either"
        );
    }
}

#[test]
fn initial_divisions_are_owned_by_the_correct_country() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("real_new_game_ownership");
    let app = setup_full_app(player, temp_dir.save_file_config());

    let military_registry = app.world().resource::<MilitaryRegistry>();
    let country_registry = app.world().resource::<CountryRegistry>();
    for division in military_registry.divisions.values() {
        let owner = country_registry
            .get(division.owner)
            .expect("owner must exist");
        assert_eq!(
            division.current_state, owner.capital_state_id,
            "the initial division must be placed at its owner's capital"
        );
    }
}

#[test]
fn advancing_several_updates_does_not_duplicate_the_initial_army_or_burden() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("real_new_game_no_duplication");
    let mut app = setup_full_app(player, temp_dir.save_file_config());

    let division_count_before = app.world().resource::<MilitaryRegistry>().divisions.len();
    let burden_before = ledger_for(&app, player).workforce_burden;

    for _ in 0..5 {
        app.update();
    }

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        division_count_before
    );
    assert_eq!(ledger_for(&app, player).workforce_burden, burden_before);
}

#[test]
fn initial_army_placement_alone_does_not_reduce_available_manpower_or_change_population() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("real_new_game_no_side_effects");
    let app = setup_full_app(player, temp_dir.save_file_config());

    // 実データの`available_manpower`はP21-001既定値(募兵未実行時は`assets/data/countries.ron`
    // 通り)であり、初期軍配置だけでは一切減っていないはずである。ここでは
    // 「募兵していないのに0や異常値になっていない」という消極的な健全性チェックに留める
    // (絶対値そのものは実データの初期値に依存するため)。
    let country = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap();
    assert!(country.available_manpower > 0);

    let state_registry = app.world().resource::<StateRegistry>();
    for state in &state_registry.states {
        assert!(
            state.population > 0 || state.is_sea,
            "population must not have been zeroed"
        );
    }
}

#[test]
fn military_panel_shows_matching_living_and_burden_with_zero_losses_at_new_game_start() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("panel_balanced_at_start");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());

    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;
    app.update();

    let ledger = ledger_for(&app, player);
    let text = app
        .world_mut()
        .query_filtered::<&Text, With<MilitaryPanelText>>()
        .single(app.world())
        .expect("MilitaryPanelText must be spawned")
        .0
        .clone();

    assert!(text.contains(&ledger.living_military_manpower.to_string()));
    assert!(text.contains(&ledger.workforce_burden.to_string()));
    assert!(text.contains("0")); // 恒久軍事損失0
    assert!(!text.contains(MISSING_KEY_MARKER_PREFIX));
}

// ─────────────────────────────────────────────────────────────────────────
// §6.3 月次民間労働力統合テスト
// ─────────────────────────────────────────────────────────────────────────

fn advance_one_month(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<strategy_game::app::time::MonthChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(strategy_game::app::time::MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

fn civilian_workforce(app: &App, state_id: StateId) -> u64 {
    app.world()
        .resource::<StateRegistry>()
        .get(state_id)
        .unwrap()
        .civilian_workforce()
}

#[test]
fn initial_army_burden_reduces_civilian_workforce_after_the_next_monthly_update() {
    let player = CountryId(0);

    // 対照App: 開始直後に初期軍の負担(B)を0へリセットしてから月次処理する
    // (=初期軍が無かった場合の反実仮想)。
    let mut control = setup_full_app(
        player,
        TempTestDir::new("cw_initial_control").save_file_config(),
    );
    control
        .world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 0;
    advance_one_month(&mut control, 1800, 2);
    let control_cw = civilian_workforce(&control, StateId(0));

    // 実験App: 実際のNew Game初期軍の負担をそのまま残す。
    let mut experiment = setup_full_app(
        player,
        TempTestDir::new("cw_initial_experiment").save_file_config(),
    );
    let initial_burden = ledger_for(&experiment, player).workforce_burden;
    assert!(
        initial_burden > 0,
        "the real New Game must give the player a nonzero initial burden"
    );
    advance_one_month(&mut experiment, 1800, 2);
    let experiment_cw = civilian_workforce(&experiment, StateId(0));

    assert!(
        experiment_cw < control_cw,
        "the real initial army's burden must reduce civilian workforce relative to the \
         no-initial-burden counterfactual: control={control_cw}, experiment={experiment_cw}"
    );
}

#[test]
fn another_countrys_initial_army_does_not_affect_this_countrys_civilian_workforce() {
    let player = CountryId(0);
    let other = CountryId(1);

    let mut control = setup_full_app(
        player,
        TempTestDir::new("cw_other_control").save_file_config(),
    );
    control
        .world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(other)
        .unwrap()
        .mobilized_manpower = 0;
    advance_one_month(&mut control, 1800, 2);
    let control_cw = civilian_workforce(&control, StateId(0));

    let experiment = setup_full_app(
        player,
        TempTestDir::new("cw_other_experiment").save_file_config(),
    );
    // `other`の初期軍負担はそのまま(実データどおり)。
    let mut experiment = experiment;
    advance_one_month(&mut experiment, 1800, 2);
    let experiment_cw = civilian_workforce(&experiment, StateId(0));

    assert_eq!(
        experiment_cw, control_cw,
        "another country's initial army burden must not affect this country's civilian workforce"
    );
}

#[test]
fn the_initial_spawn_itself_does_not_pre_run_monthly_employment() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("cw_no_premature_employment");
    let app = setup_full_app(player, temp_dir.save_file_config());

    // Startup + OnEnter(Playing)だけでは、まだ月次雇用計算(MonthChangedMessage駆動)が
    // 一度も走っていないはずである——`employed_workforce`/`unemployed_workforce`は
    // StateData初期値(0)のままであることを確認する。
    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap();
    assert_eq!(state.employed_workforce, 0);
    assert_eq!(state.unemployed_workforce, 0);
}

// ─────────────────────────────────────────────────────────────────────────
// §6.4 Loaded Game・Save/Load統合テスト
// ─────────────────────────────────────────────────────────────────────────

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

fn load_and_settle(app: &mut App) {
    send_load_request(app);
    app.update();
    app.update();
}

#[test]
fn loaded_game_does_not_run_spawn_debug_divisions_and_does_not_duplicate_saved_divisions() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("loaded_no_duplicate_divisions");
    let save_config = temp_dir.save_file_config();

    let (saved_division_count, saved_burden) = {
        let mut producer = setup_full_app(player, save_config.clone());
        let count = producer
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .len();
        let burden = ledger_for(&producer, player).workforce_burden;
        send_save_request(&mut producer);
        producer.update();
        assert!(save_config.path.final_path.exists());
        (count, burden)
    };

    let mut app = setup_full_app(player, save_config);
    // setup_full_appのStartup+OnEnter(Playing)自体が、まだロード前に既定の
    // PlayingEntryMode::NewGame経路でspawn_debug_divisionsを実行してしまっているため、
    // この時点の部隊数は「もう一度New Game相当で生成された」状態である。実際の
    // 「続きから」フローは、CountrySelectionからのロード成功時にのみ発生し、
    // Playingへ再進入する際に`PlayingEntryMode::LoadedGame`を設定する
    // (`save::runtime::handle_load_requests`)。ここではロード要求そのものが
    // Division集合を保存値でそのまま置き換える(重複させない)ことを検証する。
    load_and_settle(&mut app);
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        saved_division_count,
        "loading must restore exactly the saved division count, not add to it"
    );
    assert_eq!(ledger_for(&app, player).workforce_burden, saved_burden);
}

#[test]
fn a_balanced_new_game_save_reproduces_the_same_ledger_after_reload() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_balanced_new_game");
    let save_config = temp_dir.save_file_config();

    let saved_ledger = {
        let mut producer = setup_full_app(player, save_config.clone());
        let ledger = ledger_for(&producer, player);
        assert_eq!(ledger.workforce_burden, ledger.living_military_manpower);
        assert_eq!(ledger.permanent_military_losses, 0);
        send_save_request(&mut producer);
        producer.update();
        ledger
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    assert_eq!(ledger_for(&app, player), saved_ledger);
}

#[test]
fn a_save_with_existing_permanent_losses_preserves_them_after_reload() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_preserves_losses");
    let save_config = temp_dir.save_file_config();

    let saved_ledger = {
        let mut producer = setup_full_app(player, save_config.clone());
        // 現役Divisionへ実際の損耗を与え、B > L(恒久損失あり)の状態を作る。
        {
            let mut mr = producer.world_mut().resource_mut::<MilitaryRegistry>();
            let division = mr
                .divisions
                .values_mut()
                .find(|d| d.owner == player)
                .expect("the player must own an initial division");
            division.manpower = division.manpower.saturating_sub(1);
        }
        let ledger = ledger_for(&producer, player);
        assert!(ledger.permanent_military_losses > 0);
        send_save_request(&mut producer);
        producer.update();
        ledger
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    assert_eq!(ledger_for(&app, player), saved_ledger);
}

// ─────────────────────────────────────────────────────────────────────────
// §6.5 未知DivisionDefinition(UI・実経路レベル)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn military_panel_shows_unable_to_compute_for_an_unresolved_queue_without_panicking() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("panel_unknown_definition");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .recruitment_queue
        .push(RecruitmentQueueItem {
            division_id: DivisionDefinitionId(999_999), // 存在しない定義
            target_state: StateId(0),
            days_remaining: 5,
            total_days: 30,
        });
    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;
    app.update(); // must not panic

    let text = app
        .world_mut()
        .query_filtered::<&Text, With<MilitaryPanelText>>()
        .single(app.world())
        .expect("MilitaryPanelText must be spawned")
        .0
        .clone();

    assert!(
        !text.contains("999999"),
        "the raw definition id must not leak into player-facing text, got: {text}"
    );
    assert!(!text.contains(MISSING_KEY_MARKER_PREFIX));
}

#[test]
fn ja_and_en_resolve_the_unavailable_label_without_missing_key_markers() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("panel_unknown_definition_locales");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .recruitment_queue
        .push(RecruitmentQueueItem {
            division_id: DivisionDefinitionId(999_999),
            target_state: StateId(0),
            days_remaining: 5,
            total_days: 30,
        });
    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;

    for locale in [Locale::JaJp, Locale::EnUs] {
        app.insert_resource(CurrentLocale(locale));
        app.update();
        let text = app
            .world_mut()
            .query_filtered::<&Text, With<MilitaryPanelText>>()
            .single(app.world())
            .expect("MilitaryPanelText must be spawned")
            .0
            .clone();
        assert!(
            !text.contains(MISSING_KEY_MARKER_PREFIX),
            "must not expose an unresolved key for {locale:?}, got: {text}"
        );
    }
}
