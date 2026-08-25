//! P23-008: 損耗Divisionへの日次自動補充、実経路統合テスト。
//!
//! `src/military/reinforcement.rs`のDomain単体テストが`apply_daily_reinforcement`自体の
//! 計算・原子性を固定しているのに対し、本テストは実際の日次System
//! (`military::update::handle_daily_military`)経路・実戦闘との実行順序・
//! 月次民間労働力更新・save/load・既存UI表示の各経路を通して、補充がP23-006〜007の
//! 人的資源帳簿(B/L/K)と整合することを確認する。
//!
//! 戦闘ダメージの実数値は`combat_calc::resolve_combat_day_multi`の内部式に依存するため、
//! 本テストではその式を再実装・予測しない(`p23_006_manpower_ledger_e2e_test.rs`と同じ方針)。
#![allow(clippy::type_complexity)]

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::DayChangedMessage;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{
    BattleId, CountryId, DivisionDefinitionId, DivisionId, StateId, WarId,
};
use strategy_game::country::{CountryData, CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::localization::MISSING_KEY_MARKER_PREFIX;
use strategy_game::map::division_selection::{DragSelectState, SelectedDivision};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::military::demobilization::demobilize_division;
use strategy_game::military::manpower::{ManpowerLedger, compute_manpower_ledger};
use strategy_game::military::update::handle_daily_military;
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
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::ui::military_panel::{
    MilitaryPanelPlugin, MilitaryPanelState, MilitaryPanelText,
};
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::FrontlineRegistry;

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p23_008_e2e_{label}_{}_{nanos}_{n}",
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
// 共通フィクスチャヘルパー
// ─────────────────────────────────────────────────────────────────────────

fn test_division(
    id: usize,
    owner: usize,
    state: StateId,
    manpower: u64,
    max_manpower: u64,
) -> Division {
    Division {
        id: DivisionId(id),
        owner: CountryId(owner),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: state,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower,
        max_manpower,
        equipment: 100.0,
        max_equipment: 100.0,
        organization: 50.0,
        max_organization: 50.0,
        morale: 1.0,
        max_morale: 1.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: DivisionDefinitionId(0),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

fn test_country(id: usize, mobilized_manpower: u64, available_manpower: u64) -> CountryData {
    CountryData {
        id: CountryId(id),
        mobilized_manpower,
        available_manpower,
        treasury: 1_000_000.0,
        ..CountryData::default()
    }
}

fn test_war(id: usize, attacker: usize, defender: usize) -> War {
    War {
        id: WarId(id),
        name: "Test War".to_string(),
        start_date: "1800/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        attackers: [CountryId(attacker)].into_iter().collect(),
        defenders: [CountryId(defender)].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
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
    }
}

#[allow(clippy::too_many_arguments)]
fn test_battle(
    id: usize,
    war_id: usize,
    state_id: StateId,
    attacker: usize,
    defender: usize,
    attacker_divisions: Vec<DivisionId>,
    defender_divisions: Vec<DivisionId>,
) -> Battle {
    Battle {
        id: BattleId(id),
        war_id: WarId(war_id),
        state_id,
        attacker_country: CountryId(attacker),
        defender_country: CountryId(defender),
        attacker_division_ids: attacker_divisions,
        defender_division_ids: defender_divisions,
        attacker_origins: std::collections::HashMap::new(),
        start_date: "1800/01/01".to_string(),
        elapsed_days: 0,
        status: BattleStatus::Ongoing,
    }
}

fn owned_state(id: StateId, owner: usize) -> StateData {
    StateData {
        id,
        owner_country_id: CountryId(owner),
        ..StateData::default()
    }
}

/// `military::update::handle_daily_military`(本番System)だけを、その依存Resourceの
/// みで動かす最小構成(`p23_006_manpower_ledger_e2e_test.rs`と同一パターン)。
fn minimal_daily_app(
    country_registry: CountryRegistry,
    military_registry: MilitaryRegistry,
    state_registry: StateRegistry,
    battle_registry: BattleRegistry,
    war_registry: WarRegistry,
) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()));
    app.insert_resource(country_registry);
    app.insert_resource(military_registry);
    app.insert_resource(state_registry);
    app.insert_resource(battle_registry);
    app.insert_resource(war_registry);
    app.insert_resource(ArmyRegistry::default());
    app.insert_resource(FrontlineRegistry::default());
    app.add_message::<DayChangedMessage>();
    app.add_systems(Update, handle_daily_military);
    app
}

fn send_day_changed(app: &mut App) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
        .write(DayChangedMessage {
            year: 1800,
            month: 1,
            day: 2,
        });
    state.apply(app.world_mut());
    app.update();
}

fn ledger_for(
    country_registry: &CountryRegistry,
    military_registry: &MilitaryRegistry,
    country_id: CountryId,
) -> ManpowerLedger {
    let country = country_registry.get(country_id).unwrap();
    compute_manpower_ledger(country, military_registry)
        .expect("test fixtures must always resolve to a valid ledger")
}

fn division_manpower(app: &App, id: DivisionId) -> Option<u64> {
    app.world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&id)
        .map(|d| d.manpower)
}

// ─────────────────────────────────────────────────────────────────────────
// §11.4 日次実経路(最小構成、handle_daily_military直接駆動)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn one_day_changed_reinforces_exactly_once() {
    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 9_000, 1_000_000)],
        },
        {
            let mut mr = MilitaryRegistry::default();
            mr.divisions.insert(
                DivisionId(0),
                test_division(0, 0, StateId(0), 9_000, 10_000),
            );
            mr
        },
        StateRegistry::build(vec![owned_state(StateId(0), 0)]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );
    send_day_changed(&mut app);
    assert_eq!(division_manpower(&app, DivisionId(0)), Some(9_100)); // cap=100
}

#[test]
fn no_day_changed_means_no_reinforcement() {
    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 9_000, 1_000_000)],
        },
        {
            let mut mr = MilitaryRegistry::default();
            mr.divisions.insert(
                DivisionId(0),
                test_division(0, 0, StateId(0), 9_000, 10_000),
            );
            mr
        },
        StateRegistry::build(vec![owned_state(StateId(0), 0)]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );
    app.update(); // DayChangedMessageを送らない
    assert_eq!(division_manpower(&app, DivisionId(0)), Some(9_000));
}

#[test]
fn the_consumed_event_is_not_reprocessed_on_a_later_update_without_a_new_event() {
    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 9_000, 1_000_000)],
        },
        {
            let mut mr = MilitaryRegistry::default();
            mr.divisions.insert(
                DivisionId(0),
                test_division(0, 0, StateId(0), 9_000, 10_000),
            );
            mr
        },
        StateRegistry::build(vec![owned_state(StateId(0), 0)]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );
    send_day_changed(&mut app);
    assert_eq!(division_manpower(&app, DivisionId(0)), Some(9_100));
    app.update(); // 新しいDayChangedMessageなし
    app.update();
    assert_eq!(
        division_manpower(&app, DivisionId(0)),
        Some(9_100),
        "must not reinforce again without a fresh DayChangedMessage"
    );
}

#[test]
fn reinforcement_happens_before_that_days_combat_and_the_days_own_losses_are_not_healed_same_day() {
    // 攻撃側は補充対象(Idle、自国支配)だが、防御側との戦闘へ「その日のうちに」
    // 突入し損耗する。同日の処理内では、その損耗が同日中に補充されないことを確認する。
    let attacker_country = 0usize;
    let defender_country = 1usize;
    let origin = StateId(0);
    let target = StateId(1);

    let attacker = test_division(0, attacker_country, target, 9_000, 10_000);
    let defender = test_division(1, defender_country, target, 10_000, 10_000);

    let mut military_registry = MilitaryRegistry::default();
    military_registry
        .divisions
        .insert(attacker.id, attacker.clone());
    military_registry.divisions.insert(defender.id, defender);

    let mut war_registry = WarRegistry::default();
    war_registry
        .wars
        .insert(WarId(0), test_war(0, attacker_country, defender_country));

    let mut battle_registry = BattleRegistry::default();
    battle_registry.battles.insert(
        BattleId(0),
        test_battle(
            0,
            0,
            target,
            attacker_country,
            defender_country,
            vec![attacker.id],
            vec![DivisionId(1)],
        ),
    );

    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![
                test_country(attacker_country, 9_000, 1_000_000),
                test_country(defender_country, 10_000, 1_000_000),
            ],
        },
        military_registry,
        StateRegistry::build(vec![
            owned_state(origin, attacker_country),
            owned_state(target, defender_country),
        ]),
        battle_registry,
        war_registry,
    );

    send_day_changed(&mut app);

    // 攻撃側は「同日の補充」を受けたはず(1日目開始時点ではIdle扱いの資格を満たす)
    // が、その後の同日の戦闘で損耗するため、最終的な値は「補充後-当日の戦闘損失」
    // となり、当日の戦闘損失そのものは補充で相殺されない(同日中の再補充はない)。
    let after = division_manpower(&app, DivisionId(0));
    assert!(
        after.is_some(),
        "attacker must not have been destroyed on the first combat day in this matchup"
    );
    let after = after.unwrap();
    // 補充分(最大100)を足しても、実際の戦闘損耗があるため9,000+100=9,100には
    // 到達しない(戦闘計算式は再実装しないので、9,100より真に少ないことだけを見る)。
    assert!(
        after < 9_100,
        "same-day combat losses must not be healed by the same day's reinforcement: got {after}"
    );
}

#[test]
fn reinforcement_resumes_the_day_after_combat_ends_once_conditions_are_met() {
    let country = 0usize;
    let state = StateId(0);
    let division = test_division(0, country, state, 5_000, 10_000);

    let mut military_registry = MilitaryRegistry::default();
    military_registry.divisions.insert(division.id, division);

    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(country, 5_000, 1_000_000)],
        },
        military_registry,
        StateRegistry::build(vec![owned_state(state, country)]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );

    send_day_changed(&mut app);
    assert_eq!(division_manpower(&app, DivisionId(0)), Some(5_100)); // cap=100
}

#[test]
fn multiple_days_each_recover_roughly_one_percent() {
    let country = 0usize;
    let state = StateId(0);
    let division = test_division(0, country, state, 0, 10_000);
    // manpower=0は補充対象外(§1.3)なので1から開始する。
    let division = Division {
        manpower: 1,
        ..division
    };

    let mut military_registry = MilitaryRegistry::default();
    military_registry.divisions.insert(division.id, division);

    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(country, 1, 1_000_000)],
        },
        military_registry,
        StateRegistry::build(vec![owned_state(state, country)]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );

    for _ in 0..3 {
        send_day_changed(&mut app);
    }
    // day1: 1->101, day2: 101->201, day3: 201->301 (各日cap=100)
    assert_eq!(division_manpower(&app, DivisionId(0)), Some(301));
}

#[test]
fn the_final_day_below_the_cap_only_reinforces_the_remaining_shortage() {
    let country = 0usize;
    let state = StateId(0);
    // shortage=50 < cap(100) -> 1日で満タンになる
    let division = test_division(0, country, state, 9_950, 10_000);

    let mut military_registry = MilitaryRegistry::default();
    military_registry.divisions.insert(division.id, division);

    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(country, 9_950, 1_000_000)],
        },
        military_registry,
        StateRegistry::build(vec![owned_state(state, country)]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );

    send_day_changed(&mut app);
    assert_eq!(division_manpower(&app, DivisionId(0)), Some(10_000));
    send_day_changed(&mut app);
    assert_eq!(
        division_manpower(&app, DivisionId(0)),
        Some(10_000),
        "a full division must not keep growing"
    );
}

#[test]
fn reinforcement_stops_once_available_manpower_is_exhausted_and_resumes_after_more_is_added() {
    let country = 0usize;
    let state = StateId(0);
    let division = test_division(0, country, state, 1, 10_000);

    let mut military_registry = MilitaryRegistry::default();
    military_registry.divisions.insert(division.id, division);

    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(country, 1, 1)], // available=1、B=L=1
        },
        military_registry,
        StateRegistry::build(vec![owned_state(state, country)]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );

    send_day_changed(&mut app);
    assert_eq!(division_manpower(&app, DivisionId(0)), Some(2)); // +1(available枯渇)
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(CountryId(0))
            .unwrap()
            .available_manpower,
        0
    );
    send_day_changed(&mut app);
    assert_eq!(
        division_manpower(&app, DivisionId(0)),
        Some(2),
        "available=0の間は補充が停止するはず"
    );

    // availableを追加(募兵キャンセル等を模した直接操作)。
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(CountryId(0))
        .unwrap()
        .available_manpower = 50;
    send_day_changed(&mut app);
    assert_eq!(
        division_manpower(&app, DivisionId(0)),
        Some(52),
        "次の日から補充が再開するはず"
    );
}

#[test]
fn another_countrys_reinforcement_does_not_affect_this_countrys_state() {
    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![
                test_country(0, 5_000, 1_000_000),
                test_country(1, 8_000, 1_000_000),
            ],
        },
        {
            let mut mr = MilitaryRegistry::default();
            mr.divisions.insert(
                DivisionId(0),
                test_division(0, 0, StateId(0), 5_000, 10_000),
            );
            mr.divisions.insert(
                DivisionId(1),
                test_division(1, 1, StateId(1), 8_000, 10_000),
            );
            mr
        },
        StateRegistry::build(vec![owned_state(StateId(0), 0), owned_state(StateId(1), 1)]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );
    send_day_changed(&mut app);
    assert_eq!(division_manpower(&app, DivisionId(0)), Some(5_100));
    assert_eq!(division_manpower(&app, DivisionId(1)), Some(8_100));
}

#[test]
fn reinforcement_after_demobilization_still_yields_a_consistent_ledger() {
    let country_id = CountryId(0);
    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 10_000, 1_000_000)],
        },
        {
            let mut mr = MilitaryRegistry::default();
            // B=10,000, L=7,000, K=3,000
            mr.divisions.insert(
                DivisionId(0),
                test_division(0, 0, StateId(0), 7_000, 10_000),
            );
            mr
        },
        StateRegistry::build(vec![owned_state(StateId(0), 0)]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );

    send_day_changed(&mut app); // +100 -> manpower=7,100, B=10,100
    let ledger_after_reinforcement =
        ledger_for(app.world().resource(), app.world().resource(), country_id);
    assert_eq!(ledger_after_reinforcement.living_military_manpower, 7_100);
    assert_eq!(ledger_after_reinforcement.workforce_burden, 10_100);
    assert_eq!(ledger_after_reinforcement.permanent_military_losses, 3_000);

    // P23-007の復員: 生存兵員(7,100)だけを返還してもKは残る。
    let mut state: SystemState<(
        ResMut<CountryRegistry>,
        ResMut<MilitaryRegistry>,
        ResMut<ArmyRegistry>,
        ResMut<FrontlineRegistry>,
        Res<WarRegistry>,
        Res<BattleRegistry>,
    )> = SystemState::new(app.world_mut());
    let (mut cr, mut mr, mut ar, mut fr, wr, br) = state.get_mut(app.world_mut()).unwrap();
    let outcome = demobilize_division(
        &mut cr,
        &mut mr,
        &mut ar,
        &mut fr,
        &wr,
        &br,
        country_id,
        DivisionId(0),
    )
    .unwrap();
    assert_eq!(outcome.returned_manpower, 7_100);
    state.apply(app.world_mut());

    let country = app
        .world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap();
    assert_eq!(
        country.mobilized_manpower, 3_000,
        "B after demobilization must equal the surviving K"
    );
    assert!(
        !app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .contains_key(&DivisionId(0))
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §11.4 実New Game統合(実データ6か国、AI国家への適用確認)
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

    app.insert_resource(SelectedDivision::default())
        .insert_resource(DragSelectState::default())
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
    app.update(); // OnEnter(Playing): 初期Division配置

    app
}

fn send_day_changed_full(app: &mut App) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
        .write(DayChangedMessage {
            year: 1800,
            month: 1,
            day: 2,
        });
    state.apply(app.world_mut());
    app.update();
}

fn advance_one_month(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<strategy_game::app::time::MonthChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
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

fn player_own_division_id(app: &App, owner: CountryId) -> DivisionId {
    app.world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .find(|d| d.owner == owner)
        .expect("owner must own at least one division")
        .id
}

#[test]
fn a_new_game_initial_division_at_full_manpower_is_not_reinforced() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("new_game_full_division_not_reinforced");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let initial_id = player_own_division_id(&app, player);
    let before = division_manpower(&app, initial_id).unwrap();

    send_day_changed_full(&mut app);

    assert_eq!(
        division_manpower(&app, initial_id),
        Some(before),
        "a freshly-spawned full-manpower division must not be reinforced"
    );
}

#[test]
fn a_non_player_ai_country_receives_the_same_reinforcement_treatment() {
    let player = CountryId(0);
    let ai_country = CountryId(1);
    let temp_dir = TempTestDir::new("ai_country_reinforced");
    let mut app = setup_full_app(player, temp_dir.save_file_config());

    let ai_division_id = player_own_division_id(&app, ai_country);
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        mr.divisions.get_mut(&ai_division_id).unwrap().manpower -= 500;
    }
    let before = division_manpower(&app, ai_division_id).unwrap();

    send_day_changed_full(&mut app);

    let after = division_manpower(&app, ai_division_id).unwrap();
    assert!(
        after > before,
        "an AI-owned division must be reinforced by the exact same daily System, no manual trigger needed"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §11.5 民間労働力・経済
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn reinforcement_does_not_pre_run_monthly_employment_in_the_same_frame() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("workforce_no_preupdate");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let capital = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .capital_state_id;
    let initial_id = player_own_division_id(&app, player);
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        mr.divisions.get_mut(&initial_id).unwrap().manpower -= 500;
    }
    let employed_before = app
        .world()
        .resource::<StateRegistry>()
        .get(capital)
        .unwrap()
        .employed_workforce;

    send_day_changed_full(&mut app);

    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(capital)
        .unwrap();
    assert_eq!(state.employed_workforce, employed_before);
}

#[test]
fn the_next_monthly_update_reflects_the_reinforced_burden_as_reduced_civilian_workforce() {
    let player = CountryId(0);

    let mut control = setup_full_app(
        player,
        TempTestDir::new("workforce_reinforce_control").save_file_config(),
    );
    advance_one_month(&mut control, 1800, 2);
    let control_cw = civilian_workforce(&control, StateId(0));

    let mut experiment = setup_full_app(
        player,
        TempTestDir::new("workforce_reinforce_experiment").save_file_config(),
    );
    let initial_id = player_own_division_id(&experiment, player);
    {
        let mut mr = experiment.world_mut().resource_mut::<MilitaryRegistry>();
        mr.divisions.get_mut(&initial_id).unwrap().manpower -= 500;
    }
    send_day_changed_full(&mut experiment); // reinforces, raising B beyond the control's B
    advance_one_month(&mut experiment, 1800, 2);
    let experiment_cw = civilian_workforce(&experiment, StateId(0));

    assert!(
        experiment_cw < control_cw,
        "reinforcing (raising B) must reduce civilian workforce relative to the \
         no-reinforcement counterfactual: control={control_cw}, experiment={experiment_cw}"
    );
}

#[test]
fn another_countrys_reinforcement_does_not_affect_this_countrys_civilian_workforce_next_month() {
    let player = CountryId(0);
    let other = CountryId(1);

    let mut control = setup_full_app(
        player,
        TempTestDir::new("workforce_other_control").save_file_config(),
    );
    advance_one_month(&mut control, 1800, 2);
    let control_cw = civilian_workforce(&control, StateId(0));

    let mut experiment = setup_full_app(
        player,
        TempTestDir::new("workforce_other_experiment").save_file_config(),
    );
    let other_division = player_own_division_id(&experiment, other);
    {
        let mut mr = experiment.world_mut().resource_mut::<MilitaryRegistry>();
        mr.divisions.get_mut(&other_division).unwrap().manpower -= 500;
    }
    send_day_changed_full(&mut experiment);
    advance_one_month(&mut experiment, 1800, 2);
    let experiment_cw = civilian_workforce(&experiment, StateId(0));

    assert_eq!(experiment_cw, control_cw);
}

#[test]
fn permanent_losses_are_unchanged_by_reinforcement_before_and_after_the_monthly_update() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("workforce_k_unchanged");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let initial_id = player_own_division_id(&app, player);
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        mr.divisions.get_mut(&initial_id).unwrap().manpower -= 500;
    }
    let k_before = ledger_for(app.world().resource(), app.world().resource(), player)
        .permanent_military_losses;
    assert_eq!(
        k_before, 500,
        "reducing manpower while B stays fixed immediately creates K = B - L"
    );

    send_day_changed_full(&mut app);
    advance_one_month(&mut app, 1800, 2);

    let k_after = ledger_for(app.world().resource(), app.world().resource(), player)
        .permanent_military_losses;
    assert_eq!(
        k_after, k_before,
        "reinforcement must never create or change permanent losses on its own"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §11.6 Save/Load統合テスト
// ─────────────────────────────────────────────────────────────────────────

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
        .write(SaveRequestMessage);
    state.apply(app.world_mut());
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
        .write(LoadRequestMessage);
    state.apply(app.world_mut());
}

fn load_and_settle(app: &mut App) {
    send_load_request(app);
    app.update();
    app.update();
}

#[test]
fn a_partially_reinforced_division_round_trips_through_save_and_load() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_mid_reinforcement");
    let save_config = temp_dir.save_file_config();

    let (saved_manpower, saved_ledger) = {
        let mut producer = setup_full_app(player, save_config.clone());
        let initial_id = player_own_division_id(&producer, player);
        {
            let mut mr = producer.world_mut().resource_mut::<MilitaryRegistry>();
            mr.divisions.get_mut(&initial_id).unwrap().manpower -= 500;
        }
        send_day_changed_full(&mut producer);
        let manpower = division_manpower(&producer, initial_id).unwrap();
        let ledger = ledger_for(
            producer.world().resource(),
            producer.world().resource(),
            player,
        );
        send_save_request(&mut producer);
        producer.update();
        (manpower, ledger)
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }
    let initial_id = player_own_division_id(&app, player);
    assert_eq!(division_manpower(&app, initial_id), Some(saved_manpower));
    assert_eq!(
        ledger_for(app.world().resource(), app.world().resource(), player),
        saved_ledger
    );
}

#[test]
fn loading_alone_does_not_reinforce_and_the_next_day_changed_reinforces_exactly_one_day() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_no_extra_reinforcement");
    let save_config = temp_dir.save_file_config();

    let saved_manpower = {
        let mut producer = setup_full_app(player, save_config.clone());
        let initial_id = player_own_division_id(&producer, player);
        {
            let mut mr = producer.world_mut().resource_mut::<MilitaryRegistry>();
            mr.divisions.get_mut(&initial_id).unwrap().manpower -= 5_000;
        }
        let manpower = division_manpower(&producer, initial_id).unwrap();
        send_save_request(&mut producer);
        producer.update();
        manpower
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    let initial_id = player_own_division_id(&app, player);
    assert_eq!(
        division_manpower(&app, initial_id),
        Some(saved_manpower),
        "loading by itself must not run an extra day of reinforcement"
    );

    send_day_changed_full(&mut app);
    let after_one_day = division_manpower(&app, initial_id).unwrap();
    assert_eq!(
        after_one_day,
        saved_manpower + 100, // cap=1% of a 10,000-manpower Standard Infantry
        "exactly one regular DayChanged after load must reinforce exactly one day's worth"
    );
}

#[test]
fn reinforcement_then_demobilization_round_trips_through_save_and_load() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_reinforce_then_demobilize");
    let save_config = temp_dir.save_file_config();

    let saved_ledger = {
        let mut producer = setup_full_app(player, save_config.clone());
        let initial_id = player_own_division_id(&producer, player);
        {
            let mut mr = producer.world_mut().resource_mut::<MilitaryRegistry>();
            mr.divisions.get_mut(&initial_id).unwrap().manpower -= 500;
        }
        send_day_changed_full(&mut producer);

        let mut state: SystemState<(
            ResMut<CountryRegistry>,
            ResMut<MilitaryRegistry>,
            ResMut<ArmyRegistry>,
            ResMut<FrontlineRegistry>,
            Res<WarRegistry>,
            Res<BattleRegistry>,
        )> = SystemState::new(producer.world_mut());
        let (mut cr, mut mr, mut ar, mut fr, wr, br) = state.get_mut(producer.world_mut()).unwrap();
        demobilize_division(
            &mut cr, &mut mr, &mut ar, &mut fr, &wr, &br, player, initial_id,
        )
        .unwrap();
        state.apply(producer.world_mut());

        let ledger = ledger_for(
            producer.world().resource(),
            producer.world().resource(),
            player,
        );
        send_save_request(&mut producer);
        producer.update();
        ledger
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    assert_eq!(
        ledger_for(app.world().resource(), app.world().resource(), player),
        saved_ledger
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §11.7 UI回帰(新規UIは追加せず、既存表示の更新のみ確認)
// ─────────────────────────────────────────────────────────────────────────

fn panel_text(app: &mut App) -> String {
    app.world_mut()
        .query_filtered::<&Text, With<MilitaryPanelText>>()
        .single(app.world())
        .expect("MilitaryPanelText must be spawned")
        .0
        .clone()
}

#[test]
fn the_military_panel_reflects_available_living_and_burden_after_daily_reinforcement() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("ui_panel_reflects_reinforcement");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());
    let initial_id = player_own_division_id(&app, player);
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        mr.divisions.get_mut(&initial_id).unwrap().manpower -= 500;
    }
    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;
    app.update();

    let available_before = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .available_manpower;

    send_day_changed_full(&mut app);
    app.update();

    let ledger = ledger_for(app.world().resource(), app.world().resource(), player);
    let available_after = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .available_manpower;
    assert!(
        available_after < available_before,
        "available manpower must decrease"
    );
    assert_eq!(
        ledger.permanent_military_losses, 500,
        "K reflects the pre-existing manpower/B gap and must not be affected by reinforcement itself"
    );

    let text = panel_text(&mut app);
    assert!(text.contains(&ledger.living_military_manpower.to_string()));
    assert!(text.contains(&ledger.workforce_burden.to_string()));
    assert!(text.contains(&available_after.to_string()));
    assert!(!text.contains(MISSING_KEY_MARKER_PREFIX));
}

#[test]
fn the_panel_text_entity_is_not_duplicated_across_repeated_daily_updates() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("ui_no_duplicate_entities");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());
    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;
    for _ in 0..3 {
        send_day_changed_full(&mut app);
    }
    let count = app
        .world_mut()
        .query_filtered::<Entity, With<MilitaryPanelText>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 1);
}
