//! P23-004: `economy::finance`・`economy::production`が共通で使う経済用労働力比率
//! (`population::update::calculate_economic_workforce_ratio`)の実経路統合テスト。
//!
//! `tests/economy_tests.rs`のcharacterization testが純粋関数レベルで生産量・生活水準・
//! 不満度の厳密な数値一致を固定しているのに対し、本テストは実際の月次処理経路
//! (`economy::handle_monthly_economy`、`DailySimulationSet::Economy`)を通して、
//! 動員が余剰労働力で吸収される間は経済結果が変化せず、建物需要を割り込むと生産・
//! 生活水準・不満度が悪化方向へ動き、動員解除・他国非波及・save/load後の再現性が
//! いずれも成立することを固定する。
//!
//! P23-001の月次人口成長は動員の有無に関わらず毎月必ず実行されるため、同一Appを
//! 複数月にわたって進めて比較すると、人口成長そのものによる差分と動員による差分が
//! 混ざってしまう。本テストでは「動員の影響だけ」を切り分けるため、比較が必要な
//! ケースは常に、経過月数を揃えた2つの独立したAppを用意して比較する
//! (人口成長は動員に依存しない決定的な関数のため、経過月数さえ揃えれば両App間で
//! 完全に一致する)。

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
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::population::update::calculate_economic_workforce_ratio;
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
        "strategy_game_p23_004_e2e_{label}_{}_{nanos}_{n}",
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

/// `p23_002_mobilized_manpower_workforce_e2e_test.rs`と同一構成。実データ
/// (`assets/data/countries.ron`、6か国)を使う。UIパネルは本テストの対象外のため
/// 登録しない。
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

fn set_population(app: &mut App, state_id: StateId, population: u64) {
    let mut registry = app.world_mut().resource_mut::<StateRegistry>();
    let state = registry.get_mut(state_id).unwrap();
    state.population = population;
}

fn set_mobilized_manpower(app: &mut App, country_id: CountryId, mobilized: u64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(country_id)
        .unwrap()
        .mobilized_manpower = mobilized;
}

fn food_stockpile(app: &App, country_id: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::Food)
}

fn living_standard(app: &App, state_id: StateId) -> f32 {
    app.world()
        .resource::<StateRegistry>()
        .get(state_id)
        .unwrap()
        .living_standard
}

fn unrest(app: &App, state_id: StateId) -> f32 {
    app.world()
        .resource::<StateRegistry>()
        .get(state_id)
        .unwrap()
        .unrest
}

fn economic_workforce_ratio(app: &App, state_id: StateId) -> f32 {
    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(state_id)
        .unwrap();
    calculate_economic_workforce_ratio(state.employed_workforce, state.total_workforce())
}

/// population=120,000・workforce_ratio既定0.45で total_workforce()=54,000。
/// Farm level10で required_workforce=50,000。動員0では
/// civilian_workforce=54,000 >= 50,000 のため employed=50,000(需要で頭打ち)、
/// emp_rate=50,000/54,000 ≈ 0.9259(健全域、>=0.9)。
const CONTROLLED_POPULATION: u64 = 120_000;
const FARM_LEVEL: u32 = 10;

/// 対象州を「人口120,000・Farm10のみ」の制御下に置く。実データの2つの副作用を
/// 打ち消す必要がある。
///
/// 1. 実データの州は既にいくつか建物を保有しており、そのままFarmを追加すると
///    合計required_workforceが意図した50,000からずれる——`buildings.clear()`してから
///    Farmだけを設定する。
/// 2. `apportion_mobilized_manpower`(P23-002)は国家の`mobilized_manpower`を、
///    その国が所有する「全州」の人口比で按分する。対象州以外にも実データの
///    (人口の大きい)州を所有したままだと、指定した動員量のごく一部しか対象州へ
///    届かない——同じ国家が所有する他の州の人口を0にし、按分結果が対象州へ
///    100%集中するようにする(所有権自体は変更しない)。
fn setup_controlled_state(app: &mut App, player: CountryId, state_id: StateId) {
    {
        let mut registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in registry.states.iter_mut() {
            if state.owner_country_id == player && state.id != state_id {
                state.population = 0;
            }
        }
    }
    set_population(app, state_id, CONTROLLED_POPULATION);
    {
        let mut registry = app.world_mut().resource_mut::<StateRegistry>();
        let state = registry.get_mut(state_id).unwrap();
        state.buildings.clear();
        state.buildings.insert(BuildingType::Farm, FARM_LEVEL);
        state.workforce_ratio = strategy_game::state::data::DEFAULT_WORKFORCE_RATIO;
    }
}

/// mobilized_manpowerを最初の月替わり「前」に設定し、1か月だけ進めたAppを返す
/// (人口成長は動員の有無に関わらず決定的なため、経過月数を1で揃えれば
/// 動員量だけを変数として比較できる)。
///
/// P23-006-FIX-001: New Gameの初期軍は実際に`mobilized_manpower`へ計上されるため、
/// `mobilized == 0`(「動員なし」の制御された基準点)の場合でも必ず明示的に0へ
/// 上書きする——`if mobilized > 0`でガードすると、実データの初期軍による付随的な
/// 負担がそのまま残ってしまう。
fn app_after_one_month_with_mobilization(label: &str, player: CountryId, mobilized: u64) -> App {
    let temp_dir = TempTestDir::new(label);
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    setup_controlled_state(&mut app, player, StateId(0));
    set_mobilized_manpower(&mut app, player, mobilized);
    advance_one_month(&mut app, 1800, 2);
    app
}

#[test]
fn zero_mobilization_yields_a_healthy_workforce_ratio_and_positive_production() {
    let player = CountryId(0);
    let app = app_after_one_month_with_mobilization("zero_mobilization_baseline", player, 0);

    let ratio = economic_workforce_ratio(&app, StateId(0));
    assert!(
        ratio >= 0.9,
        "the controlled fixture must start in the healthy employment band, got {ratio}"
    );
    assert!(
        food_stockpile(&app, player) > 0.0,
        "production must be non-zero at baseline"
    );
}

#[test]
fn mobilization_absorbed_by_surplus_labor_does_not_change_production_or_welfare() {
    let player = CountryId(0);
    // civilian_workforce = 54,000 - 3,000 = 51,000 >= required(50,000): まだ需要を満たせる。
    let unmobilized = app_after_one_month_with_mobilization("surplus_control", player, 0);
    let surplus_mobilized =
        app_after_one_month_with_mobilization("surplus_mobilized", player, 3_000);

    assert_eq!(
        economic_workforce_ratio(&surplus_mobilized, StateId(0)),
        economic_workforce_ratio(&unmobilized, StateId(0)),
        "employed_workforce stays pinned at demand while civilian_workforce >= demand, so \
         the economic workforce ratio (denominator is unaffected by mobilization) must not \
         change"
    );
    assert_eq!(
        food_stockpile(&surplus_mobilized, player),
        food_stockpile(&unmobilized, player),
        "production must be unaffected while mobilization is absorbed by surplus labor"
    );
    assert_eq!(
        living_standard(&surplus_mobilized, StateId(0)),
        living_standard(&unmobilized, StateId(0))
    );
    assert_eq!(
        unrest(&surplus_mobilized, StateId(0)),
        unrest(&unmobilized, StateId(0))
    );
}

#[test]
fn mobilization_below_required_workforce_reduces_production() {
    let player = CountryId(0);
    // civilian_workforce = 54,000 - 20,000 = 34,000 < required(50,000): 需要を満たせない。
    let unmobilized = app_after_one_month_with_mobilization("shortage_control", player, 0);
    let shortage = app_after_one_month_with_mobilization("shortage_mobilized", player, 20_000);

    let ratio_unmobilized = economic_workforce_ratio(&unmobilized, StateId(0));
    let ratio_shortage = economic_workforce_ratio(&shortage, StateId(0));
    assert!(
        ratio_shortage < ratio_unmobilized,
        "the economic workforce ratio must drop once mobilization pushes civilian workforce \
         below building demand: unmobilized={ratio_unmobilized}, shortage={ratio_shortage}"
    );
    assert!(
        ratio_shortage < 0.9,
        "the ratio must leave the healthy band once demand is unmet, got {ratio_shortage}"
    );
    assert!(
        food_stockpile(&shortage, player) < food_stockpile(&unmobilized, player),
        "production must drop once mobilization pushes civilian workforce below demand"
    );
}

#[test]
fn heavier_mobilization_never_improves_welfare_and_can_push_it_into_the_degraded_band() {
    let player = CountryId(0);

    let mut living_standards = Vec::new();
    let mut ratios = Vec::new();
    for (idx, mobilized) in [0u64, 10_000, 20_000].into_iter().enumerate() {
        let app = app_after_one_month_with_mobilization(
            &format!("welfare_tier_{idx}"),
            player,
            mobilized,
        );
        living_standards.push(living_standard(&app, StateId(0)));
        ratios.push(economic_workforce_ratio(&app, StateId(0)));
    }

    for pair in ratios.windows(2) {
        assert!(
            pair[0] >= pair[1],
            "the economic workforce ratio must be monotonically non-increasing as mobilization \
             grows: {ratios:?}"
        );
    }
    for pair in living_standards.windows(2) {
        assert!(
            pair[0] >= pair[1],
            "living standard must never improve as mobilization grows: {living_standards:?}"
        );
    }
    assert!(
        ratios[0] >= 0.9,
        "mobilized=0 must sit in the healthy band, got {}",
        ratios[0]
    );
    assert!(
        ratios[2] < 0.7,
        "the heaviest mobilization tier must fall into the degraded band, got {}",
        ratios[2]
    );
}

#[test]
fn demobilizing_restores_the_workforce_ratio_and_that_months_production_on_the_next_tick() {
    let player = CountryId(0);

    // 対照App: 2か月とも動員なし。
    // P23-006-FIX-001: New Gameの初期軍による付随的な負担を明示的にゼロへ上書きする。
    let mut control = setup_app_in_playing(
        player,
        TempTestDir::new("demobilize_control").save_file_config(),
    );
    setup_controlled_state(&mut control, player, StateId(0));
    set_mobilized_manpower(&mut control, player, 0);
    advance_one_month(&mut control, 1800, 2);
    let control_month2_total = food_stockpile(&control, player);
    advance_one_month(&mut control, 1800, 3);
    let control_month3_delta = food_stockpile(&control, player) - control_month2_total;

    // 実験App: 1か月目は動員(需要割れ)、2か月目に動員解除。
    let mut experiment = setup_app_in_playing(
        player,
        TempTestDir::new("demobilize_experiment").save_file_config(),
    );
    setup_controlled_state(&mut experiment, player, StateId(0));
    set_mobilized_manpower(&mut experiment, player, 20_000);
    advance_one_month(&mut experiment, 1800, 2);
    let experiment_month2_total = food_stockpile(&experiment, player);
    set_mobilized_manpower(&mut experiment, player, 0);
    advance_one_month(&mut experiment, 1800, 3);
    let experiment_month3_delta = food_stockpile(&experiment, player) - experiment_month2_total;

    // 人口成長は動員に依存しないため、2か月経過後の人口(=total_workforce())は
    // 両App間で完全に一致する。動員解除後のmonth3は、両Appとも
    // civilian_workforce >= required(demand)のため、employed=required(demand頭打ち)で
    // 一致し、月間生産量も一致するはずである。
    assert_eq!(
        economic_workforce_ratio(&experiment, StateId(0)),
        economic_workforce_ratio(&control, StateId(0)),
        "the workforce ratio must fully recover the same month mobilization ends"
    );
    assert!(
        (experiment_month3_delta - control_month3_delta).abs() < 1e-6,
        "month 3's production alone must match the always-unmobilized control once \
         demobilized: control={control_month3_delta}, experiment={experiment_month3_delta}"
    );
}

#[test]
fn one_countrys_mobilization_does_not_affect_another_countrys_production_or_welfare() {
    let player = CountryId(0);
    let other = CountryId(1);

    fn build(
        label: &str,
        player: CountryId,
        other: CountryId,
        player_mobilized: u64,
    ) -> (App, StateId) {
        let temp_dir = TempTestDir::new(label);
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
        setup_controlled_state(&mut app, other, other_state_id);
        if player_mobilized > 0 {
            set_mobilized_manpower(&mut app, player, player_mobilized);
        }
        advance_one_month(&mut app, 1800, 2);
        (app, other_state_id)
    }

    let (unmobilized, other_state_a) = build("cross_country_control", player, other, 0);
    let (player_mobilized, other_state_b) =
        build("cross_country_mobilized", player, other, 500_000);
    assert_eq!(other_state_a, other_state_b);
    let other_state_id = other_state_a;

    assert_eq!(
        economic_workforce_ratio(&player_mobilized, other_state_id),
        economic_workforce_ratio(&unmobilized, other_state_id),
        "another country's economic workforce ratio must not change because of the player's \
         mobilization"
    );
    assert_eq!(
        living_standard(&player_mobilized, other_state_id),
        living_standard(&unmobilized, other_state_id)
    );
    assert_eq!(
        food_stockpile(&player_mobilized, other),
        food_stockpile(&unmobilized, other)
    );
}

#[test]
fn a_real_save_and_load_round_trip_reproduces_the_same_economic_workforce_ratio() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_reproduces_ratio");
    let save_config = temp_dir.save_file_config();

    let (saved_ratio, saved_ls) = {
        let mut producer = setup_app_in_playing(player, save_config.clone());
        setup_controlled_state(&mut producer, player, StateId(0));
        set_mobilized_manpower(&mut producer, player, 20_000);
        advance_one_month(&mut producer, 1800, 2);

        let ratio = economic_workforce_ratio(&producer, StateId(0));
        let ls = living_standard(&producer, StateId(0));
        assert!(ratio < 0.9);

        send_save_request(&mut producer);
        producer.update();
        assert!(save_config.path.final_path.exists());
        (ratio, ls)
    };

    let mut app = setup_app_in_playing(player, save_config);
    load_and_settle(&mut app);
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }

    assert_eq!(economic_workforce_ratio(&app, StateId(0)), saved_ratio);
    assert_eq!(living_standard(&app, StateId(0)), saved_ls);

    // ロード後の次の正式な月替わりでも、同じ動員量から同じ需要割れの結果が再導出できる
    // (動員量はセーブ済みの`mobilized_manpower`から復元され、需要50,000は変わらない)。
    advance_one_month(&mut app, 1800, 3);
    let ratio_after_reload_tick = economic_workforce_ratio(&app, StateId(0));
    assert!(
        ratio_after_reload_tick < 0.9,
        "the reloaded shortage must still be reflected on the next monthly tick, got {ratio_after_reload_tick}"
    );
}
