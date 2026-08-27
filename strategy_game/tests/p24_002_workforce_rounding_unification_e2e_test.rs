//! P24-002: 州の潜在総労働力(`StateData::total_workforce()`と
//! `population::update::calculate_workforce()`)の丸め方式統一の回帰テスト。
//!
//! P24-001-REV-001以前は、同じ「population * workforce_ratio」という概念に対し
//! `total_workforce()`が切り捨て、`calculate_workforce()`が四捨五入という異なる
//! 丸め規則を使っていた。P24-002はこの2つを`state::data::canonical_total_workforce`
//! へ委譲する形で1か所へ集約し、正規規則を切り捨てに統一した。
//!
//! P24-002-REV-001: P24-002当初の実装は`(population as f64) * (workforce_ratio as f64)
//! as u64`という素朴な浮動小数点計算で、`workforce_ratio: f32`の0.45が2進浮動小数点で
//! 正確な0.45よりわずかに小さい値(`0.449999988...`)として保持されることの影響を
//! populationの規模に応じて増幅してしまい、`canonical_total_workforce(100, 0.45)`が
//! 数学的に正しい45ではなく44になっていた。P24-002-REV-001はratioを先に100万分率へ
//! 量子化(`canonical_workforce_ratio_units`)してから`u128`中間値で乗除算する固定小数点
//! 方式へ訂正し、`canonical_total_workforce(100, 0.45) == 45`・
//! `canonical_total_workforce(100_000, 0.45) == 45_000`を正規値とした。本ファイルの
//! Test B/F/G/H/Iの期待値は、この訂正に伴って44→45・34→35・44,999→45,000へ更新している
//! (Test Cの population=99・ratio=0.45→44は訂正後も変わらない)。
//!
//! `rev001_`で始まるテストはP24-002-REV-001で新規追加した固定小数点方式固有の検証
//! (ratio量子化・u128 overflow耐性・production/finance連動)である。
//!
//! 死傷者とpopulationの連動はP24-001-REV-001の決定によりP24-002/P24-002-REV-001の
//! 対象外であり、本テストでも一切扱わない。

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
use strategy_game::state::data::{
    StateData, StateRegistry, canonical_total_workforce, canonical_workforce_ratio_units,
};
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
/// P24-002-REV-001: ratioを先に100万分率(450,000)へ量子化してから`u128`で
/// `100 * 450,000 / 1,000,000`を計算するため、数学的に正しい45が得られる
/// (訂正前は`(100 as f64) * (0.45 as f64) as u64`が44を返していた——0.45f32が
/// 2進浮動小数点で正確な0.45よりわずかに小さいことの影響がpopulationの規模に
/// 応じて増幅されていたため)。
#[test]
fn test_b_representative_value_matches_the_fixed_point_computation() {
    let canonical = canonical_total_workforce(100, 0.45);
    assert_eq!(
        canonical, 45,
        "fixed-point computation (quantize ratio to 450,000 units, then floor(100 * 450,000 / 1,000,000)) must yield the mathematically correct 45"
    );
    assert_eq!(total_workforce_of(100, 0.45), canonical);
    assert_eq!(calculate_workforce(100, 0.45), canonical);
}

/// Test C: 切り捨て境界。population=99, workforce_ratio=0.45は
/// `floor(99 * 450,000 / 1,000,000) = floor(44.55) = 44`となり、固定小数点方式へ
/// 訂正した後もTest Bとは異なり44のままである(端数0.55が切り捨てられるだけの、
/// 意図した「人数は切り捨て」仕様どおりの結果)。
#[test]
fn test_c_floor_boundary_matches_the_fixed_point_computation() {
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
/// employed=0・unemployed=canonical(=45、P24-002-REV-001の固定小数点訂正後の値)となり、
/// employed+unemployed==canonicalが成立する。
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
    assert_eq!(canonical, 45);
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
/// 単独制御下に置く(Test Bの境界(canonical=45、P24-002-REV-001訂正後)を実App経路で
/// 再現するため)。同じ国家の他州の人口を0にし、動員按分が対象州へ集中するようにする
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
/// 正規の固定小数点計算(canonical=45、P24-002-REV-001訂正後)を使うこと。
/// 訂正前の素朴な浮動小数点計算(44)にはならない。
#[test]
fn test_g_zero_mobilization_monthly_update_uses_the_canonical_fixed_point_value() {
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
        45,
        "monthly employment update must use the corrected fixed-point value (45), not the pre-REV-001 naive floating-point value (44)"
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
    // canonical(100, 0.45) = 45(P24-002-REV-001訂正後); mobilized_manpower(10)が
    // この州単独へ全額按分される(他州人口0)ため、civilian_workforce = 45 - 10 = 35。
    assert_eq!(first_run, 35);
}

/// Test I: save/load分岐比較。統一後の境界population(canonical=45、
/// P24-002-REV-001訂正後)について、「継続実行」と「save→load→実行」で
/// workforce・civilian_workforceが一致すること。Saveスキーマは変更していない。
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
    assert_eq!(continuous_wf, 45);

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

// ─── P24-002-REV-001: 固定小数点方式固有の検証 ─────────────────────────────

/// REV-001 Test A: ratio量子化(`canonical_workforce_ratio_units`)が要求された表と
/// 一致すること。
#[test]
fn rev001_test_a_ratio_quantization_matches_the_required_table() {
    assert_eq!(canonical_workforce_ratio_units(0.0), 0);
    assert_eq!(canonical_workforce_ratio_units(0.1), 100_000);
    assert_eq!(canonical_workforce_ratio_units(0.45), 450_000);
    assert_eq!(canonical_workforce_ratio_units(0.5), 500_000);
    assert_eq!(canonical_workforce_ratio_units(0.333333), 333_333);
    assert_eq!(canonical_workforce_ratio_units(0.999999), 999_999);
    assert_eq!(canonical_workforce_ratio_units(1.0), 1_000_000);
}

/// REV-001 Test B/C: §7の必須数値例表を、独立した`u128`式(本番の
/// `canonical_total_workforce`を呼ばずに`canonical_workforce_ratio_units`の
/// リテラル期待値から再計算する)で照合してから、本番関数の結果とも一致することを
/// 確認する。期待値の計算に本番関数を使った自己証明は行わない。
#[test]
fn rev001_test_bc_required_numeric_table_verified_independently() {
    let cases: [(u64, f32, u64, u64); 12] = [
        (0, 0.45, 450_000, 0),
        (1, 0.5, 500_000, 0),
        (2, 0.5, 500_000, 1),
        (10, 0.1, 100_000, 1),
        (99, 0.45, 450_000, 44),
        (100, 0.45, 450_000, 45),
        (101, 0.45, 450_000, 45),
        (100_000, 0.45, 450_000, 45_000),
        (1_000_000, 0.333333, 333_333, 333_333),
        (1_000_000, 0.999999, 999_999, 999_999),
        (u64::MAX, 0.0, 0, 0),
        (u64::MAX, 1.0, 1_000_000, u64::MAX),
    ];
    for (population, ratio, expected_units, expected_workforce) in cases {
        assert_eq!(
            canonical_workforce_ratio_units(ratio),
            expected_units,
            "ratio units mismatch for ratio={ratio}"
        );
        // 独立したu128式(本番のcanonical_total_workforceは呼ばない)で人数を再計算する。
        let independent = ((population as u128) * (expected_units as u128) / 1_000_000u128) as u64;
        assert_eq!(
            independent, expected_workforce,
            "independent u128 formula mismatch for population={population}, ratio={ratio}"
        );
        assert_eq!(
            canonical_total_workforce(population, ratio),
            expected_workforce,
            "canonical_total_workforce mismatch for population={population}, ratio={ratio}"
        );
    }
}

// REV-001 Test E/F: population単調性・ratio単調性・上限は既存のTest D/D2/Eが
// 固定小数点方式に対しても引き続き有効であることを確認済み(構造は無変更、
// `canonical_total_workforce`を通して自動的に新方式で再検証される)。

/// REV-001 Test G: `u64::MAX`付近のpopulationでoverflow・panic・wraparoundが
/// 発生しないこと。
#[test]
fn rev001_test_g_overflow_safety_near_u64_max() {
    assert_eq!(canonical_total_workforce(u64::MAX, 0.0), 0);
    assert_eq!(canonical_total_workforce(u64::MAX, 1.0), u64::MAX);

    let population = u64::MAX;
    let ratio_units = canonical_workforce_ratio_units(0.5) as u128;
    assert_eq!(ratio_units, 500_000);
    let expected = ((population as u128) * ratio_units / 1_000_000u128) as u64;
    let actual = canonical_total_workforce(population, 0.5);
    assert_eq!(actual, expected);
    assert!(
        actual <= population,
        "workforce must never exceed population even near u64::MAX"
    );

    // u64::MAX - 1 でも同様にoverflowしないこと。
    assert_eq!(canonical_total_workforce(u64::MAX - 1, 1.0), u64::MAX - 1);
}

/// REV-001 Test H: save/load分岐比較でworkforceだけでなくproduction(食料生産)も
/// 一致すること(§9: production・financeへの影響がある場合、その影響が
/// save/load間で一貫していることを確認する)。Farmを1つ持つ境界population(100,
/// ratio=0.45、canonical=45)で検証する。
#[test]
fn rev001_test_h_save_load_branch_comparison_matches_production() {
    use strategy_game::building::data::BuildingType;
    use strategy_game::economy::resources::ResourceType;

    fn setup(app: &mut App, player: CountryId, state_id: StateId) {
        let mut registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in registry.states.iter_mut() {
            if state.owner_country_id == player {
                state.population = if state.id == state_id { 100 } else { 0 };
            }
            if state.id == state_id {
                state.buildings.clear();
                state.buildings.insert(BuildingType::Farm, 1);
                state.workforce_ratio = 0.45;
            }
        }
    }

    let player = CountryId(0);

    let temp_dir_continuous = TempTestDir::new("rev001_h_continuous");
    let mut continuous = setup_app_in_playing(player, temp_dir_continuous.save_file_config());
    setup(&mut continuous, player, StateId(0));
    advance_one_month(&mut continuous, 1800, 2);

    let temp_dir_branch = TempTestDir::new("rev001_h_branch");
    let mut branch = setup_app_in_playing(player, temp_dir_branch.save_file_config());
    setup(&mut branch, player, StateId(0));
    advance_one_month(&mut branch, 1800, 2);
    send_save_request(&mut branch);
    branch.update();
    send_load_request(&mut branch);
    branch.update();
    branch.update();

    let continuous_workforce = continuous
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap()
        .total_workforce();
    let branch_workforce = branch
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .unwrap()
        .total_workforce();
    assert_eq!(continuous_workforce, 45);
    assert_eq!(continuous_workforce, branch_workforce);

    let continuous_food = continuous
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .stockpile
        .get(ResourceType::Food);
    let branch_food = branch
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .stockpile
        .get(ResourceType::Food);
    assert_eq!(
        continuous_food, branch_food,
        "production output derived from the corrected workforce value must match between the continuous and save/load branches"
    );
}

// REV-001 Test I: New Game経路(`setup_app_in_playing`が直接生成する状態)と
// Loaded Game経路(save→load後の状態)は、同じ州状態であれば固定小数点方式でも
// 同じ結果になる。既存のTest Iおよび`rev001_test_h_...`が実質的にこれを検証している
// (継続実行 = New Game経路そのもの、save→load後の実行 = Loaded Game経路)。
