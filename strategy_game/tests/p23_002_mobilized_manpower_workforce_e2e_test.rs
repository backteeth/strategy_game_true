//! P23-002: 動員人的資源(`mobilized_manpower`)による民間労働力圧迫の統合テスト。
//!
//! `src/population/mobilization.rs`のユニットテストが按分アルゴリズムそのものを固定して
//! いるのに対し、本テストは実際の月次雇用計算経路(`economy::handle_monthly_economy`、
//! `DailySimulationSet::Economy`)を通して、動員が実際に雇用可能人数を減らし、動員解除で
//! 回復し、他国へ波及せず、州所有権移転後は新しい所有関係で再計算され、セーブ/ロード
//! 後も同じ結果を再導出できることを固定する。

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
use strategy_game::military::MilitaryPlugin;
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
        "strategy_game_p23_002_e2e_{label}_{}_{nanos}_{n}",
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

/// `p23_001_population_manpower_e2e_test.rs`と同一構成。実データ
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

/// テスト対象の州(`StateId(0)`)に、雇用計算が実際に非ゼロの`total_required_workforce`を
/// 持つよう建物を1つ与える(建物が無いと`total_required_wf == 0`のため、常に
/// `employed_workforce == 0`のまま動員の影響が観測できない)。`max_level`(実データで
/// Farmは10)を超えると`save::validate`が拒否するため、レベルはその範囲内に収める
/// (本テスト作成時に実際にsave/loadテストで発見)。
fn ensure_state_has_labor_demand(app: &mut App, state_id: StateId) {
    let mut registry = app.world_mut().resource_mut::<StateRegistry>();
    let state = registry.get_mut(state_id).unwrap();
    state.buildings.insert(BuildingType::Farm, 10);
}

#[test]
fn without_mobilization_employment_reaches_the_full_civilian_workforce() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("baseline_no_mobilization");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    ensure_state_has_labor_demand(&mut app, StateId(0));
    // P23-006-FIX-001: New Gameの初期軍は実際にmobilized_manpowerへ計上されるため、
    // このテストが意図する「動員なし」を明示的な前提として作る
    // (実データの初期軍による付随的な負担に左右されないようにする)。
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 0;

    advance_one_month(&mut app, 1800, 2);

    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap();
    let total_wf = strategy_game::population::update::calculate_workforce(
        state.population,
        state.workforce_ratio,
    );
    assert_eq!(
        state.employed_workforce + state.unemployed_workforce,
        total_wf
    );
}

#[test]
fn mobilizing_manpower_reduces_available_workforce_and_employable_count_next_month() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("mobilization_reduces_workforce");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    ensure_state_has_labor_demand(&mut app, StateId(0));

    advance_one_month(&mut app, 1800, 2);
    let (total_wf_before, employed_before) = {
        let state = app
            .world()
            .resource::<StateRegistry>()
            .get(StateId(0))
            .unwrap();
        (
            state.employed_workforce + state.unemployed_workforce,
            state.employed_workforce,
        )
    };

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 200_000;

    advance_one_month(&mut app, 1800, 3);

    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap();
    let total_wf_after = state.employed_workforce + state.unemployed_workforce;
    assert!(
        total_wf_after < total_wf_before,
        "available workforce must shrink once manpower is mobilized: before={total_wf_before}, after={total_wf_after}"
    );
    assert!(
        state.employed_workforce <= employed_before,
        "employable count must not increase once workforce shrinks"
    );
}

#[test]
fn the_workforce_reduction_matches_the_population_weighted_apportionment() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("reduction_matches_apportionment");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    ensure_state_has_labor_demand(&mut app, StateId(0));

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

    advance_one_month(&mut app, 1800, 2);

    // advance_one_month内でP23-001の人口成長が先に実行されるため、按分は成長後の
    // 人口を基準にする(実Systemの順序をそのまま踏襲する)。
    let grown_owned: Vec<(StateId, u64)> = {
        let registry = app.world().resource::<StateRegistry>();
        owned
            .iter()
            .map(|&(id, _)| (id, registry.get(id).unwrap().population))
            .collect()
    };
    let expected_burdens = strategy_game::population::mobilization::apportion_mobilized_manpower(
        150_000,
        &grown_owned,
    );

    let registry = app.world().resource::<StateRegistry>();
    for (id, population_before_growth) in &owned {
        let _ = population_before_growth;
        let state = registry.get(*id).unwrap();
        let total_wf = strategy_game::population::update::calculate_workforce(
            state.population,
            state.workforce_ratio,
        );
        let expected_burden = expected_burdens.get(id).copied().unwrap_or(0);
        let expected_total_available = total_wf.saturating_sub(expected_burden);
        assert_eq!(
            state.employed_workforce + state.unemployed_workforce,
            expected_total_available,
            "state {id:?} available workforce must match the population-weighted apportionment"
        );
    }
}

#[test]
fn a_burden_larger_than_the_workforce_clamps_employment_to_zero_without_underflow() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("burden_exceeds_workforce");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    ensure_state_has_labor_demand(&mut app, StateId(0));

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = u64::MAX / 4;

    advance_one_month(&mut app, 1800, 2);

    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap();
    assert_eq!(state.employed_workforce, 0);
    assert_eq!(state.unemployed_workforce, 0);
}

#[test]
fn demobilizing_restores_workforce_on_the_next_monthly_tick() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("demobilization_restores_workforce");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    ensure_state_has_labor_demand(&mut app, StateId(0));

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 300_000;
    advance_one_month(&mut app, 1800, 2);
    let reduced_total_wf = {
        let state = app
            .world()
            .resource::<StateRegistry>()
            .get(StateId(0))
            .unwrap();
        state.employed_workforce + state.unemployed_workforce
    };

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 0;
    advance_one_month(&mut app, 1800, 3);

    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap();
    let restored_total_wf = state.employed_workforce + state.unemployed_workforce;
    assert!(
        restored_total_wf > reduced_total_wf,
        "demobilizing to zero must restore workforce on the next monthly tick: reduced={reduced_total_wf}, restored={restored_total_wf}"
    );
}

#[test]
fn one_countrys_mobilization_does_not_affect_another_countrys_employment() {
    let player = CountryId(0);
    let other = CountryId(1);
    let temp_dir = TempTestDir::new("mobilization_does_not_cross_countries");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let other_state_id = {
        let registry = app.world().resource::<StateRegistry>();
        registry
            .states
            .iter()
            .find(|s| s.owner_country_id == other)
            .map(|s| s.id)
            .expect("the real map must give CountryId(1) at least one state")
    };
    ensure_state_has_labor_demand(&mut app, other_state_id);

    advance_one_month(&mut app, 1800, 2);
    let baseline_other_total_wf = {
        let state = app
            .world()
            .resource::<StateRegistry>()
            .get(other_state_id)
            .unwrap();
        state.employed_workforce + state.unemployed_workforce
    };

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 500_000;
    advance_one_month(&mut app, 1800, 3);

    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(other_state_id)
        .unwrap();
    let other_total_wf_after = state.employed_workforce + state.unemployed_workforce;
    // 他国自身の人口も成長しているため完全一致ではなく、プレイヤー国の動員による
    // "圧迫"が起きていない(=通常の成長ぶんだけ変化する)ことを、大幅な減少が
    // 無いことで確認する。
    assert!(
        other_total_wf_after >= baseline_other_total_wf,
        "another country's employment must not shrink because of player's mobilization: \
         before={baseline_other_total_wf}, after={other_total_wf_after}"
    );
}

#[test]
fn ownership_transfer_moves_the_burden_to_the_new_owner_on_the_next_tick() {
    let player = CountryId(0);
    let other = CountryId(1);
    let temp_dir = TempTestDir::new("ownership_transfer_moves_burden");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    ensure_state_has_labor_demand(&mut app, StateId(0));

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 100_000;
    // P23-006-FIX-001: `other`も実データの初期軍によりmobilized_manpowerが既に
    // 非ゼロになり得るため、「移転後は誰の負担も負わない」ことを検証するこのテストの
    // 意図どおり、移転先の動員負担も明示的にゼロへ揃える。
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(other)
        .unwrap()
        .mobilized_manpower = 0;
    advance_one_month(&mut app, 1800, 2);

    // StateId(0)をplayerからotherへ移転する(実際の州所有権移転Domain APIの結果と同じ形)。
    {
        let mut registry = app.world_mut().resource_mut::<StateRegistry>();
        let state = registry.get_mut(StateId(0)).unwrap();
        state.owner_country_id = other;
        state.controller_country = Some(other);
    }

    advance_one_month(&mut app, 1800, 3);

    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap();
    let total_wf = strategy_game::population::update::calculate_workforce(
        state.population,
        state.workforce_ratio,
    );
    assert_eq!(
        state.employed_workforce + state.unemployed_workforce,
        total_wf,
        "the transferred state must no longer bear the former owner's mobilization burden"
    );
}

#[test]
fn a_real_save_and_load_round_trip_reproduces_the_same_workforce_result() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_reproduces_workforce");
    let save_config = temp_dir.save_file_config();

    let saved_total_wf = {
        let mut producer = setup_app_in_playing(player, save_config.clone());
        ensure_state_has_labor_demand(&mut producer, StateId(0));
        producer
            .world_mut()
            .resource_mut::<CountryRegistry>()
            .get_mut(player)
            .unwrap()
            .mobilized_manpower = 120_000;
        advance_one_month(&mut producer, 1800, 2);

        let state = producer
            .world()
            .resource::<StateRegistry>()
            .get(StateId(0))
            .unwrap();
        let total_wf = state.employed_workforce + state.unemployed_workforce;
        assert!(total_wf > 0);

        send_save_request(&mut producer);
        producer.update();
        assert!(save_config.path.final_path.exists());
        total_wf
    };

    let mut app = setup_app_in_playing(player, save_config);
    load_and_settle(&mut app);
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }

    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap();
    assert_eq!(
        state.employed_workforce + state.unemployed_workforce,
        saved_total_wf,
        "the saved employed/unemployed split must be restored exactly by loading"
    );

    // ロード後の次の正式な月替わりでも、同じ按分ロジックが同じ結果を再導出できる
    // (mobilized_manpower自体はセーブ済みのCountryDataから復元されるため)。
    let mobilized_after_load = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .mobilized_manpower;
    assert_eq!(mobilized_after_load, 120_000);
    advance_one_month(&mut app, 1800, 3);
    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap();
    assert!(state.employed_workforce + state.unemployed_workforce > 0);
}
