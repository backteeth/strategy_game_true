//! P24-002: 州の潜在総労働力(`StateData::total_workforce()`と
//! `population::update::calculate_workforce()`)の丸め方式統一の回帰テスト。
//!
//! P24-001-REV-001以前は、同じ「population * workforce_ratio」という概念に対し
//! `total_workforce()`が切り捨て、`calculate_workforce()`が四捨五入という異なる
//! 丸め規則を使っていた(境界例: population=100, workforce_ratio=0.45で
//! 切り捨て=44、四捨五入=45)。P24-002はこの2つを`state::data::canonical_total_workforce`
//! へ委譲する形で1か所へ集約し、正規規則を切り捨てに統一した。
//!
//! 死傷者とpopulationの連動はP24-001-REV-001の決定によりP24-002の対象外であり、
//! 本テストでも一切扱わない。

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
use strategy_game::population::update::calculate_workforce;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::{StateData, StateRegistry, canonical_total_workforce};
use strategy_game::war::WarPlugin;

// ─── 純粋関数レベル (Test A〜F) ─────────────────────────────────────────────

fn total_workforce_of(population: u64, workforce_ratio: f32) -> u64 {
    let state = StateData {
        population,
        workforce_ratio,
        ..StateData::default()
    };
    state.total_workforce()
}

/// Test A: 最小不一致例。population=1, workforce_ratio=0.5では
/// 切り捨て=0・四捨五入=1が食い違っていた。統一後は両方0(正規=切り捨て)。
#[test]
fn test_a_minimum_disagreement_example_unifies_to_floor() {
    assert_eq!(canonical_total_workforce(1, 0.5), 0);
    assert_eq!(total_workforce_of(1, 0.5), 0);
    assert_eq!(calculate_workforce(1, 0.5), 0);
}

/// Test B: 代表値。population=100, workforce_ratio=0.45。
/// `workforce_ratio: f32`の0.45は2進浮動小数点で正確に表現できず、実際には
/// 0.45よりわずかに小さい値として保持される。そのため
/// `canonical_total_workforce(100, 0.45)`は数学的な100*0.45=45ではなく44になる
/// (既存の`StateData::total_workforce()`が元々使っていた乗算順序・キャスト方法を
/// 正規基準として踏襲した結果であり、P24-002のために新たに導入した誤差ではない)。
#[test]
fn test_b_representative_value_matches_the_existing_floor_computation() {
    let canonical = canonical_total_workforce(100, 0.45);
    assert_eq!(
        canonical, 44,
        "0.45f32 is slightly below 0.45 in binary floating point, so floor(100 * 0.45f32) is 44, not the mathematically-naive 45"
    );
    assert_eq!(total_workforce_of(100, 0.45), canonical);
    assert_eq!(calculate_workforce(100, 0.45), canonical);
}

/// Test C: 切り捨て境界。population=99, workforce_ratio=0.45も同じ理由で44になる
/// (Test Bと同じ丸め誤差の帯域に入るため)。
#[test]
fn test_c_floor_boundary_matches_the_existing_floor_computation() {
    let canonical = canonical_total_workforce(99, 0.45);
    assert_eq!(canonical, 44);
    assert_eq!(total_workforce_of(99, 0.45), canonical);
    assert_eq!(calculate_workforce(99, 0.45), canonical);
}

/// Test D: 単調性。populationが増えてもworkforceが減少しないことを、
/// P24-001-REV-001が問題にした比率を含む代表的なratioの範囲で確認する。
#[test]
fn test_d_workforce_is_monotonically_non_decreasing_in_population() {
    for &ratio in &[0.0f32, 0.1, 0.45, 0.5, 1.0] {
        let mut previous = canonical_total_workforce(0, ratio);
        for population in 0..=1000u64 {
            let current = canonical_total_workforce(population, ratio);
            assert!(
                current >= previous,
                "workforce decreased at population={population}, ratio={ratio}: {previous} -> {current}"
            );
            previous = current;
        }
    }
}

/// Test D(補): ratio増加だけでworkforceが減少しないこと。
#[test]
fn test_d2_workforce_is_monotonically_non_decreasing_in_ratio() {
    for &population in &[0u64, 1, 2, 3, 9, 10, 11, 99, 100, 101, 1000] {
        let ratios: [f32; 6] = [0.0, 0.1, 0.3, 0.45, 0.5, 1.0];
        let mut previous = canonical_total_workforce(population, ratios[0]);
        for &ratio in &ratios[1..] {
            let current = canonical_total_workforce(population, ratio);
            assert!(
                current >= previous,
                "workforce decreased at population={population} when ratio increased: {previous} -> {current}"
            );
            previous = current;
        }
    }
}

/// Test E: 上限。有効なratio(<=1.0)ではworkforceがpopulationを超えない。
#[test]
fn test_e_workforce_never_exceeds_population_for_valid_ratios() {
    for &ratio in &[0.0f32, 0.1, 0.45, 0.5, 1.0] {
        for population in 0..=1000u64 {
            let workforce = canonical_total_workforce(population, ratio);
            assert!(
                workforce <= population,
                "workforce {workforce} exceeded population {population} at ratio {ratio}"
            );
        }
    }
}

/// 境界条件: population=0は常に0、ratio=0は常に0、ratio=1はpopulationと一致。
#[test]
fn test_boundary_conditions() {
    for &ratio in &[0.0f32, 0.1, 0.45, 0.5, 1.0] {
        assert_eq!(canonical_total_workforce(0, ratio), 0);
    }
    for &population in &[0u64, 1, 100, 1_000_000] {
        assert_eq!(canonical_total_workforce(population, 0.0), 0);
        assert_eq!(canonical_total_workforce(population, 1.0), population);
    }
}

/// 決定性: 同じ入力を複数回計算しても結果が変わらない。
#[test]
fn test_repeated_calls_are_deterministic() {
    let a = canonical_total_workforce(12_345, 0.37);
    let b = canonical_total_workforce(12_345, 0.37);
    let c = canonical_total_workforce(12_345, 0.37);
    assert_eq!(a, b);
    assert_eq!(b, c);
}

/// `total_workforce()`と`calculate_workforce()`が全検証値で一致すること
/// (計算式の真実源が1か所であることの直接証明)。
#[test]
fn test_total_workforce_and_calculate_workforce_agree_on_every_boundary_input() {
    let populations: [u64; 10] = [0, 1, 2, 3, 9, 10, 11, 99, 100, 101];
    let ratios: [f32; 5] = [0.0, 0.1, 0.45, 0.5, 1.0];
    for &r in &ratios {
        for &p in &populations {
            assert_eq!(
                total_workforce_of(p, r),
                calculate_workforce(p, r),
                "total_workforce() and calculate_workforce() disagreed at population={p}, ratio={r}"
            );
        }
    }
}

/// Test F: 月次更新(`update_state_population_and_employment`)が正規計算を使うこと。
/// population=100, workforce_ratio=0.45, 建物なし(required_workforce=0)では、
/// employed=0・unemployed=canonical(=44)となり、employed+unemployed==canonicalが
/// 成立する。
#[test]
fn test_f_monthly_employment_update_uses_the_canonical_computation() {
    use strategy_game::building::data::BuildingRegistry;
    use strategy_game::population::update::update_state_population_and_employment;

    let mut state = StateData {
        population: 100,
        workforce_ratio: 0.45,
        ..StateData::default()
    };
    let building_registry = BuildingRegistry::default();
    update_state_population_and_employment(&mut state, &building_registry, 0);

    let canonical = canonical_total_workforce(100, 0.45);
    assert_eq!(canonical, 44);
    assert_eq!(
        state.employed_workforce + state.unemployed_workforce,
        canonical
    );
    assert_eq!(state.civilian_workforce(), canonical);
}

// ─── 実経路レベル (Test G〜I) ──────────────────────────────────────────────
// `tests/p23_004_economic_workforce_ratio_e2e_test.rs`と同一のApp構成パターンを再利用する。

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p24_002_e2e_{label}_{}_{nanos}_{n}",
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

/// 対象州を「人口100・workforce_ratio=0.45・建物なし(required_workforce=0)」の
/// 単独制御下に置く(Test B/Cの境界(canonical=44)を実App経路で再現するため)。
/// 同じ国家の他州の人口を0にし、動員按分が対象州へ集中するようにする
/// (`p23_004`の`setup_controlled_state`と同じ理由)。
fn setup_controlled_boundary_state(app: &mut App, player: CountryId, state_id: StateId) {
    let mut registry = app.world_mut().resource_mut::<StateRegistry>();
    for state in registry.states.iter_mut() {
        if state.owner_country_id == player {
            state.population = if state.id == state_id { 100 } else { 0 };
        }
        if state.id == state_id {
            state.buildings.clear();
            state.workforce_ratio = 0.45;
        }
    }
}

fn civilian_workforce_of(app: &App, state_id: StateId) -> u64 {
    app.world()
        .resource::<StateRegistry>()
        .get(state_id)
        .unwrap()
        .civilian_workforce()
}

/// Test G: 動員0の州で、実際の月次経路(`economy::handle_monthly_economy`)が
/// 正規の切り捨て計算(canonical=44)を使うこと。四捨五入(45)にはならない。
#[test]
fn test_g_zero_mobilization_monthly_update_uses_the_canonical_floor_value() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_g");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    setup_controlled_boundary_state(&mut app, player, StateId(0));
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 0;

    advance_one_month(&mut app, 1800, 2);

    assert_eq!(
        civilian_workforce_of(&app, StateId(0)),
        44,
        "monthly employment update must use the canonical floor value (44), not the old rounded value (45)"
    );
}

/// Test H: 動員ありでも、境界populationにおいて決定的按分(`apportion_mobilized_manpower`)と
/// `civilian_workforce()`が統一後の正規総労働力を基礎に一貫して動くこと。
#[test]
fn test_h_mobilized_case_stays_deterministic_on_the_unified_canonical_value() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_h");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    setup_controlled_boundary_state(&mut app, player, StateId(0));
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 10;

    advance_one_month(&mut app, 1800, 2);

    let first_run = civilian_workforce_of(&app, StateId(0));

    let temp_dir_2 = TempTestDir::new("test_h_repeat");
    let mut app2 = setup_app_in_playing(player, temp_dir_2.save_file_config());
    setup_controlled_boundary_state(&mut app2, player, StateId(0));
    app2.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 10;
    advance_one_month(&mut app2, 1800, 2);
    let second_run = civilian_workforce_of(&app2, StateId(0));

    assert_eq!(
        first_run, second_run,
        "the same scenario must produce identical civilian_workforce independent of HashMap iteration order"
    );
    // canonical(100, 0.45) = 44; mobilized_manpower(10)がこの州単独へ全額按分される
    // (他州人口0)ため、civilian_workforce = 44 - 10 = 34。
    assert_eq!(first_run, 34);
}

/// Test I: save/load分岐比較。統一後の境界population(canonical=44)について、
/// 「継続実行」と「save→load→実行」でworkforce・civilian_workforceが一致すること。
/// Saveスキーマは変更していない。
#[test]
fn test_i_save_load_branch_comparison_matches_at_the_boundary_population() {
    let player = CountryId(0);

    let temp_dir_continuous = TempTestDir::new("test_i_continuous");
    let mut continuous = setup_app_in_playing(player, temp_dir_continuous.save_file_config());
    setup_controlled_boundary_state(&mut continuous, player, StateId(0));
    continuous
        .world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 0;
    advance_one_month(&mut continuous, 1800, 2);
    advance_one_month(&mut continuous, 1800, 3);

    let temp_dir_branch = TempTestDir::new("test_i_branch");
    let mut branch = setup_app_in_playing(player, temp_dir_branch.save_file_config());
    setup_controlled_boundary_state(&mut branch, player, StateId(0));
    branch
        .world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 0;
    advance_one_month(&mut branch, 1800, 2);
    send_save_request(&mut branch);
    branch.update();
    send_load_request(&mut branch);
    branch.update();
    branch.update();
    advance_one_month(&mut branch, 1800, 3);

    let continuous_wf = civilian_workforce_of(&continuous, StateId(0));
    let branch_wf = civilian_workforce_of(&branch, StateId(0));
    assert_eq!(
        continuous_wf, branch_wf,
        "continuous run and save->load->continue must produce the same civilian_workforce"
    );
    assert_eq!(continuous_wf, 44);

    let continuous_ratio = continuous
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap()
        .workforce_ratio;
    let branch_ratio = branch
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap()
        .workforce_ratio;
    assert_eq!(
        continuous_ratio, branch_ratio,
        "workforce_ratio must round-trip through save/load unchanged (no schema change)"
    );
}
