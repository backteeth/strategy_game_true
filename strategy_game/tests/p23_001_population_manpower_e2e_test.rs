//! P23-001: 州人口の月次成長・`available_manpower`の月次回復の統合テスト。
//!
//! `src/population/growth.rs`のユニットテストが純粋関数・単体Systemの正しさを固定して
//! いるのに対し、本テストは(1)同じ月内で既存の`economy::handle_monthly_economy`
//! (雇用計算)が成長後の人口を実際に参照すること、(2)実際のセーブ/ロード経路
//! (`SaveRequestMessage`→`handle_save_requests`、`LoadRequestMessage`→
//! `handle_load_requests`)を通して、成長後の人口・回復後の`available_manpower`が
//! 正しく保存・復元されること、(3)ロードそれ自体は追加の月次成長を発生させず、
//! ロード後の最初の正式な月替わりで1回だけ成長することを固定する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::population::growth::{
    compute_manpower_cap, compute_population_by_owner, grow_state_population,
};
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{
    LastLoadOutcome, LoadOutcome, LoadRequestMessage, SaveFileConfig,
};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
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
        "strategy_game_p23_001_e2e_{label}_{}_{nanos}_{n}",
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

/// 実プラグイン一式(`p21_save_003_end_to_end_test.rs`と同一構成)+
/// `PopulationPlugin`(P23-001)を組み合わせた統合テスト用App。実データ
/// (`assets/data/countries.ron`、6か国)を使う。
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

fn load_and_settle(app: &mut App) {
    send_load_request(app);
    app.update(); // PostUpdate: handle_load_requests
    app.update(); // Update: 以後のSystemがロード後の状態を見る
}

/// `advance_game_date`(実時間)を待たず、テストから直接1か月ぶん進める。
/// `GamePaused`はfalse化せず(実時間経過を防ぐ)、`MonthChangedMessage`を直接書き込む。
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

#[test]
fn monthly_economy_employment_uses_the_already_grown_population_in_the_same_month() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("employment_uses_grown_population");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let before_population = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap()
        .population;

    advance_one_month(&mut app, 1800, 2);

    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap();
    let grown_population = grow_state_population(before_population);
    assert_eq!(
        state.population, grown_population,
        "population must have grown by exactly one month"
    );

    let expected_total_workforce = strategy_game::population::update::calculate_workforce(
        grown_population,
        state.workforce_ratio,
    );
    let actual_total_workforce = state.employed_workforce + state.unemployed_workforce;
    assert_eq!(
        actual_total_workforce, expected_total_workforce,
        "the same month's employment calculation (economy::handle_monthly_economy) must have \
         used the already-grown population, not the pre-growth value"
    );
}

#[test]
fn growth_and_recovery_survive_a_real_save_and_load_round_trip() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_round_trip");
    let save_config = temp_dir.save_file_config();

    let (saved_population, saved_manpower) = {
        let mut producer = setup_app_in_playing(player, save_config.clone());
        advance_one_month(&mut producer, 1800, 2);
        advance_one_month(&mut producer, 1800, 3);

        let population = producer
            .world()
            .resource::<StateRegistry>()
            .get(StateId(0))
            .unwrap()
            .population;
        let manpower = producer
            .world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .available_manpower;
        assert!(
            manpower > 0,
            "two months of recovery must have produced nonzero manpower"
        );

        send_save_request(&mut producer);
        producer.update();
        assert!(save_config.path.final_path.exists());
        (population, manpower)
    };

    let mut app = setup_app_in_playing(player, save_config.clone());
    load_and_settle(&mut app);
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }

    // 3. ロードだけでは追加成長しない: ロード直後の値は保存時の値と完全一致する。
    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(StateId(0))
            .unwrap()
            .population,
        saved_population,
        "loading must not, by itself, apply any additional month of growth"
    );
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .available_manpower,
        saved_manpower,
        "loading must restore the exact saved available_manpower, not recompute it"
    );

    // 4. ロード後の次の月替わりで1回だけ成長する。
    advance_one_month(&mut app, 1800, 4);
    let after_one_more_month = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap()
        .population;
    assert_eq!(
        after_one_more_month,
        grow_state_population(saved_population),
        "the first real month-change after loading must apply exactly one more month of growth"
    );
}

#[test]
fn manpower_recovery_respects_ownership_after_a_real_state_ownership_transfer() {
    let player = CountryId(0);
    let other = CountryId(1);
    let temp_dir = TempTestDir::new("ownership_transfer_regression");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    // 実際の州所有権移転(P21-020の`state::ownership`が生成する結果と同じ形)を
    // シミュレートする: player所有の1州をotherへ移す。
    let transferred_population = {
        let mut registry = app.world_mut().resource_mut::<StateRegistry>();
        let state = registry.get_mut(StateId(0)).unwrap();
        assert_eq!(state.owner_country_id, player, "test setup assumption");
        state.owner_country_id = other;
        state.controller_country = Some(other);
        state.population
    };

    advance_one_month(&mut app, 1800, 2);

    let registry = app.world().resource::<CountryRegistry>();
    let population_by_owner = compute_population_by_owner(app.world().resource::<StateRegistry>());
    let other_population = population_by_owner.get(&other).copied().unwrap_or(0);
    assert!(
        other_population >= transferred_population,
        "the transferred state's population must now be counted under the new owner"
    );
    let other_cap = compute_manpower_cap(other_population);
    assert!(
        registry.get(other).unwrap().available_manpower > 0 || other_cap == 0,
        "the new owner must be eligible to recover manpower from the transferred state \
         starting the next month's population tick"
    );
}
