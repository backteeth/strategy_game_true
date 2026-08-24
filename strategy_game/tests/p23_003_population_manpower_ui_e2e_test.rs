//! P23-003: 人口・人的資源・動員負担のUI可視化と表示整合の統合テスト。
//!
//! `src/ui/state_panel.rs`・`src/ui/military_panel.rs`のユニットテストが表示値の
//! 算出ロジックそのものを固定しているのに対し、本テストは実際の月次処理経路
//! (`app::AppPlugin`+`PopulationPlugin`+`EconomyPlugin`等)を通して、州パネル・
//! 軍事パネルの表示が人口成長・動員・動員解除・州所有権変更・セーブ/ロード後に
//! 正しく更新されることを固定する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::BuildingType;
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::localization::{
    CurrentLocale, Locale, LocalizedText, MISSING_KEY_MARKER_PREFIX,
};
use strategy_game::military::MilitaryPlugin;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::population::growth::compute_manpower_cap;
use strategy_game::population::mobilization::apportion_mobilized_manpower;
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
use strategy_game::ui::state_panel::{StatePanelPlugin, StatePanelText, format_population};
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
        "strategy_game_p23_003_e2e_{label}_{}_{nanos}_{n}",
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

/// `p23_002_mobilized_manpower_workforce_e2e_test.rs`と同一構成に、UI系プラグインが
/// 実際にspawnする各種パネルSystemも含める(`AppPlugin`が`UiPlugin`一式を内包する)。
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

    // P22-003等の既存E2Eと同じ理由で`UiPlugin`/`MapPlugin`全体は登録せず、本テストが
    // 実際に読む州パネル・軍事パネルの2つだけを個別に登録する
    // (`TranslationCatalog`/`CurrentLocale`は`EconomyPlugin`が内包する
    // `TranslationCorePlugin`経由で既に用意される)。
    app.add_plugins(StatePanelPlugin);
    app.add_plugins(MilitaryPanelPlugin);

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

fn load_and_settle(app: &mut App) {
    send_load_request(app);
    app.update();
    app.update();
}

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

/// P23-002のテストと同じ理由(`total_required_workforce`を非ゼロにする)で必要。
fn ensure_state_has_labor_demand(app: &mut App, state_id: StateId) {
    let mut registry = app.world_mut().resource_mut::<StateRegistry>();
    let state = registry.get_mut(state_id).unwrap();
    state.buildings.insert(BuildingType::Farm, 10);
}

fn select_state(app: &mut App, state_id: StateId) {
    app.insert_resource(SelectedState(Some(state_id)));
}

fn read_state_panel_args(app: &mut App) -> Vec<(&'static str, String)> {
    app.world_mut()
        .query_filtered::<&LocalizedText, With<StatePanelText>>()
        .single(app.world())
        .expect("StatePanelText must be spawned")
        .args
        .clone()
}

fn state_panel_arg(args: &[(&'static str, String)], key: &str) -> String {
    args.iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| panic!("expected arg '{key}' to be present, got: {args:?}"))
}

fn read_state_panel_text(app: &mut App) -> String {
    app.world_mut()
        .query_filtered::<&Text, With<StatePanelText>>()
        .single(app.world())
        .expect("StatePanelText must be spawned")
        .0
        .clone()
}

fn open_military_panel(app: &mut App) {
    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;
    app.update();
}

fn read_military_panel_text(app: &mut App) -> String {
    app.world_mut()
        .query_filtered::<&Text, With<MilitaryPanelText>>()
        .single(app.world())
        .expect("MilitaryPanelText must be spawned")
        .0
        .clone()
}

#[test]
fn state_panel_workforce_and_employment_rate_match_the_real_monthly_result() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("state_panel_matches_real_monthly_result");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    ensure_state_has_labor_demand(&mut app, StateId(0));
    select_state(&mut app, StateId(0));

    advance_one_month(&mut app, 1800, 2);

    let (employed, unemployed) = {
        let state = app
            .world()
            .resource::<StateRegistry>()
            .get(StateId(0))
            .unwrap();
        (state.employed_workforce, state.unemployed_workforce)
    };
    let civilian_wf = employed + unemployed;

    let args = read_state_panel_args(&mut app);
    assert_eq!(
        state_panel_arg(&args, "workforce"),
        format_population(civilian_wf)
    );
    assert_eq!(
        state_panel_arg(&args, "employed"),
        format_population(employed)
    );
    assert_eq!(
        state_panel_arg(&args, "unemployed"),
        format_population(unemployed)
    );
    let expected_rate = ((employed as f64 / civilian_wf as f64) * 100.0).round() as i64;
    assert_eq!(
        state_panel_arg(&args, "employment_rate"),
        format!("{expected_rate}")
    );
}

#[test]
fn mobilization_shrinks_state_panel_workforce_and_shows_a_matching_burden() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("mobilization_shrinks_workforce_display");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    ensure_state_has_labor_demand(&mut app, StateId(0));
    select_state(&mut app, StateId(0));

    advance_one_month(&mut app, 1800, 2);
    let baseline_workforce = {
        let args = read_state_panel_args(&mut app);
        state_panel_arg(&args, "workforce")
    };

    let owned: Vec<(StateId, u64)> = {
        let registry = app.world().resource::<StateRegistry>();
        registry
            .states
            .iter()
            .filter(|s| s.owner_country_id == player)
            .map(|s| (s.id, s.population))
            .collect()
    };
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 150_000;
    advance_one_month(&mut app, 1800, 3);

    let expected_burdens = apportion_mobilized_manpower(150_000, &owned);
    let expected_burden = expected_burdens.get(&StateId(0)).copied().unwrap_or(0);

    let args = read_state_panel_args(&mut app);
    assert_ne!(
        state_panel_arg(&args, "workforce"),
        baseline_workforce,
        "workforce display must change once manpower is mobilized"
    );
    assert_eq!(
        state_panel_arg(&args, "mobilized_burden"),
        format_population(expected_burden)
    );
    assert_ne!(expected_burden, 0);
}

#[test]
fn demobilizing_zeroes_the_burden_display_on_the_next_tick() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("demobilizing_zeroes_burden_display");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    ensure_state_has_labor_demand(&mut app, StateId(0));
    select_state(&mut app, StateId(0));

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 200_000;
    advance_one_month(&mut app, 1800, 2);
    let mobilized_args = read_state_panel_args(&mut app);
    assert_ne!(
        state_panel_arg(&mobilized_args, "mobilized_burden"),
        format_population(0)
    );

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 0;
    advance_one_month(&mut app, 1800, 3);

    let args = read_state_panel_args(&mut app);
    assert_eq!(
        state_panel_arg(&args, "mobilized_burden"),
        format_population(0)
    );
}

#[test]
fn military_panel_shows_population_available_and_cap_manpower_after_opening() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("military_panel_shows_population_and_cap");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    open_military_panel(&mut app);

    let owned_population: u64 = {
        let registry = app.world().resource::<StateRegistry>();
        registry
            .states
            .iter()
            .filter(|s| s.owner_country_id == player)
            .map(|s| s.population)
            .sum()
    };
    let cap = compute_manpower_cap(owned_population);
    let country = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .clone();

    let text = read_military_panel_text(&mut app);
    assert!(
        text.contains(&format_population(owned_population)),
        "military panel must show the player's total owned population, got: {text}"
    );
    assert!(
        text.contains(&cap.to_string()),
        "military panel must show the population-derived manpower cap, got: {text}"
    );
    assert!(
        text.contains(&country.available_manpower.to_string()),
        "military panel must still show available manpower, got: {text}"
    );
    assert!(
        text.contains(&country.mobilized_manpower.to_string()),
        "military panel must still show mobilized manpower, got: {text}"
    );
}

#[test]
fn military_panel_population_updates_after_state_ownership_transfer() {
    let player = CountryId(0);
    let other = CountryId(1);
    let temp_dir = TempTestDir::new("military_panel_population_after_transfer");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    open_military_panel(&mut app);
    let before_text = read_military_panel_text(&mut app);

    {
        let mut registry = app.world_mut().resource_mut::<StateRegistry>();
        let state = registry.get_mut(StateId(0)).unwrap();
        assert_eq!(
            state.owner_country_id, player,
            "test assumes the real map gives StateId(0) to CountryId(0)"
        );
        state.owner_country_id = other;
        state.controller_country = Some(other);
    }
    app.update();

    let after_text = read_military_panel_text(&mut app);
    assert_ne!(
        before_text, after_text,
        "losing a state must change the displayed owned population"
    );

    let new_owned_population: u64 = {
        let registry = app.world().resource::<StateRegistry>();
        registry
            .states
            .iter()
            .filter(|s| s.owner_country_id == player)
            .map(|s| s.population)
            .sum()
    };
    assert!(
        after_text.contains(&format_population(new_owned_population)),
        "population line must reflect the state's new owner immediately, got: {after_text}"
    );
}

#[test]
fn save_and_load_round_trip_restores_the_state_and_military_panel_display() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_restores_panel_display");
    let save_config = temp_dir.save_file_config();

    let (saved_state_args, saved_military_text) = {
        let mut producer = setup_app_in_playing(player, save_config.clone());
        ensure_state_has_labor_demand(&mut producer, StateId(0));
        select_state(&mut producer, StateId(0));
        producer
            .world_mut()
            .resource_mut::<CountryRegistry>()
            .get_mut(player)
            .unwrap()
            .mobilized_manpower = 120_000;
        advance_one_month(&mut producer, 1800, 2);
        open_military_panel(&mut producer);

        let state_args = read_state_panel_args(&mut producer);
        let military_text = read_military_panel_text(&mut producer);

        send_save_request(&mut producer);
        producer.update();
        assert!(save_config.path.final_path.exists());
        (state_args, military_text)
    };

    let mut app = setup_app_in_playing(player, save_config);
    load_and_settle(&mut app);
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }

    select_state(&mut app, StateId(0));
    open_military_panel(&mut app);
    app.update();

    let loaded_state_args = read_state_panel_args(&mut app);
    let loaded_military_text = read_military_panel_text(&mut app);

    assert_eq!(
        state_panel_arg(&loaded_state_args, "workforce"),
        state_panel_arg(&saved_state_args, "workforce"),
        "the displayed civilian workforce must be identical after a load"
    );
    assert_eq!(
        state_panel_arg(&loaded_state_args, "mobilized_burden"),
        state_panel_arg(&saved_state_args, "mobilized_burden"),
        "the displayed mobilization burden must be identical after a load"
    );
    assert_eq!(
        loaded_military_text, saved_military_text,
        "the military panel's population/manpower text must be identical after a load"
    );
}

#[test]
fn ja_and_en_locales_render_the_new_placeholders_without_missing_key_markers() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("locales_render_without_missing_markers");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    ensure_state_has_labor_demand(&mut app, StateId(0));
    select_state(&mut app, StateId(0));
    open_military_panel(&mut app);
    advance_one_month(&mut app, 1800, 2);

    for locale in [Locale::JaJp, Locale::EnUs] {
        app.insert_resource(CurrentLocale(locale));
        app.update();

        let state_text = read_state_panel_text(&mut app);
        let military_text = read_military_panel_text(&mut app);
        assert!(
            !state_text.contains(MISSING_KEY_MARKER_PREFIX),
            "state panel must not expose an unresolved key for {locale:?}, got: {state_text}"
        );
        assert!(
            !military_text.contains(MISSING_KEY_MARKER_PREFIX),
            "military panel must not expose an unresolved key for {locale:?}, got: {military_text}"
        );
    }
}

#[test]
fn military_panel_text_entity_is_not_duplicated_across_repeated_updates() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("military_panel_not_duplicated");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    open_military_panel(&mut app);
    for month in 2..6u8 {
        advance_one_month(&mut app, 1800, month);
    }

    let count = app
        .world_mut()
        .query_filtered::<Entity, With<MilitaryPanelText>>()
        .iter(app.world())
        .count();
    assert_eq!(
        count, 1,
        "repeated monthly updates must not spawn additional MilitaryPanelText entities"
    );
}
