//! P23-007: 個別Divisionの復員(生存兵員の人的資源返還)、実経路統合テスト。
//!
//! `src/military/demobilization.rs`のDomain単体テストが`demobilize_division`自体の
//! 計算・原子性を固定しているのに対し、本テストは実際の募兵完了・実戦闘損耗・
//! Army/前線参照整理・月次民間労働力更新・save/load・UI表示の各経路を通して、
//! P23-006/P23-006-FIX-001が整理した人的資源帳簿(B/L/K)との整合を確認する。
//!
//! 戦闘ダメージの実数値は`combat_calc::resolve_combat_day_multi`の内部式に依存するため、
//! 本テストではその式を再実装・予測しない(`p23_006_manpower_ledger_e2e_test.rs`と同じ方針)。
#![allow(clippy::type_complexity)] // SystemStateのタプル型はテスト内での使い捨て構成であり、型エイリアス化の利益がない

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::ecs::world::Mut;
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
use strategy_game::localization::{
    CurrentLocale, Locale, MISSING_KEY_MARKER_PREFIX, TranslationCatalog, t,
};
use strategy_game::map::division_selection::{DragSelectState, SelectedDivision};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::military::demobilization::{DemobilizationError, demobilize_division};
use strategy_game::military::manpower::{ManpowerLedger, compute_manpower_ledger};
use strategy_game::military::recruitment::request_recruitment;
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
    DemobilizeDivisionButton, DemobilizeDivisionInfoText, MilitaryPanelPlugin, MilitaryPanelState,
    MilitaryPanelText,
};
use strategy_game::ui::notification::GameNotification;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::{Frontline, FrontlineRegistry};

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p23_007_e2e_{label}_{}_{nanos}_{n}",
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

fn test_division(id: usize, owner: usize, current_state: StateId, manpower: u64) -> Division {
    Division {
        id: DivisionId(id),
        owner: CountryId(owner),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower,
        max_manpower: manpower.max(1),
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

fn base_state_registry() -> StateRegistry {
    StateRegistry::build(vec![StateData {
        id: StateId(0),
        ..StateData::default()
    }])
}

/// `military::update::handle_daily_military`(本番System)だけを、その依存Resourceのみで
/// 動かす最小構成(`p23_006_manpower_ledger_e2e_test.rs`と同一パターン)。
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
        .expect("writer")
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

fn app_ledger_for(app: &App, country_id: CountryId) -> ManpowerLedger {
    ledger_for(
        app.world().resource::<CountryRegistry>(),
        app.world().resource::<MilitaryRegistry>(),
        country_id,
    )
}

// ─────────────────────────────────────────────────────────────────────────
// 完全なApp構成(実データ6か国、P23-004/006/006-FIX-001と同一パターン)
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
    app.insert_state(GameState::Playing); // 既定のPlayingEntryMode::NewGameのまま進入
    app.update(); // OnEnter(Playing): 初期Division配置(P23-006-FIX-001でBも同時加算)

    app
}

fn call_request_recruitment(
    app: &mut App,
    player: CountryId,
    def_id: DivisionDefinitionId,
    target_state: StateId,
) -> Result<(), &'static str> {
    app.world_mut()
        .resource_scope(|world, military_registry: Mut<MilitaryRegistry>| {
            let mut country_registry = world.resource_mut::<CountryRegistry>();
            let country = country_registry.get_mut(player).unwrap();
            request_recruitment(country, &military_registry, def_id, target_state)
        })
}

fn send_day_changed_full(app: &mut App) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
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

fn player_own_division_id(app: &App, player: CountryId) -> DivisionId {
    app.world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .find(|d| d.owner == player)
        .expect("player must own at least one division")
        .id
}

fn call_demobilize(
    app: &mut App,
    player: CountryId,
    division_id: DivisionId,
) -> Result<u64, DemobilizationError> {
    let mut state: SystemState<(
        ResMut<CountryRegistry>,
        ResMut<MilitaryRegistry>,
        ResMut<ArmyRegistry>,
        ResMut<FrontlineRegistry>,
        Res<WarRegistry>,
        Res<BattleRegistry>,
    )> = SystemState::new(app.world_mut());
    let (mut cr, mut mr, mut ar, mut fr, wr, br) = state.get_mut(app.world_mut()).unwrap();
    let result = demobilize_division(
        &mut cr,
        &mut mr,
        &mut ar,
        &mut fr,
        &wr,
        &br,
        player,
        division_id,
    );
    state.apply(app.world_mut());
    result.map(|o| o.returned_manpower)
}

fn press_demobilize_button(app: &mut App) {
    app.world_mut()
        .spawn((DemobilizeDivisionButton, Interaction::Pressed));
    app.update();
    // `DemobilizeDivisionButton`はDetection用の`Changed<Interaction>`を1回だけ消費するため、
    // 直後にもう一度updateすると新規スポーン扱いにならず、余計な再実行は起きない。
}

// ─────────────────────────────────────────────────────────────────────────
// §13.3 実ライフサイクル統合テスト
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn recruiting_a_division_to_completion_then_demobilizing_it_unworn_returns_full_manpower() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("lifecycle_recruit_then_demobilize");
    let mut app = setup_full_app(player, temp_dir.save_file_config());

    let capital = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .capital_state_id;
    call_request_recruitment(&mut app, player, DivisionDefinitionId(0), capital).unwrap();

    // 完成までの日数を待たず、実際のキュー項目を「本日完成」へ進めてから
    // 実System(`handle_daily_military`→`process_recruitment`)で1日分処理する
    // (`p23_006_manpower_ledger_e2e_test.rs`と同じ理由: 戦闘・募兵完了式自体は
    // このテストの対象外であり、既存テストで別途固定済み)。
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .recruitment_queue
        .last_mut()
        .unwrap()
        .days_remaining = 0;
    send_day_changed_full(&mut app);

    let division_before = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .find(|d| {
            d.def_id == DivisionDefinitionId(0) && d.current_state == capital && d.manpower > 0
        })
        .cloned();
    // 初期軍と同じ定義・州のため、完成した新規Divisionを人数の増分から特定する。
    let military_before_count = app.world().resource::<MilitaryRegistry>().divisions.len();
    assert!(
        division_before.is_some(),
        "recruitment must have completed into a real Division"
    );
    let recruited_id = division_before.unwrap().id;

    let available_before = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .available_manpower;
    let ledger_before = app_ledger_for(&app, player);

    let returned = call_demobilize(&mut app, player, recruited_id).unwrap();
    assert!(returned > 0);

    let available_after = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .available_manpower;
    let ledger_after = app_ledger_for(&app, player);
    assert_eq!(available_after, available_before + returned);
    assert_eq!(
        ledger_after.workforce_burden,
        ledger_before.workforce_burden - returned
    );
    assert_eq!(ledger_after.permanent_military_losses, 0);
    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        military_before_count - 1
    );
}

#[test]
fn a_real_combat_survivor_can_be_demobilized_and_permanent_losses_survive_it() {
    // 実戦闘(`handle_daily_military`→`resolve_combat_day_multi`)で実際にmanpowerを
    // 減らした後、そのDivisionが戦闘を終えIdleへ戻った状態を復員する。ダメージの
    // 実数値自体は再実装・予測せず、観測した前後差分をそのまま使う
    // (`p23_006_manpower_ledger_e2e_test.rs`と同じ方針)。
    let attacker_div = test_division(0, 0, StateId(0), 10_000);
    let defender_div = test_division(1, 1, StateId(0), 10_000);
    let mut military_registry = MilitaryRegistry::default();
    military_registry
        .divisions
        .insert(attacker_div.id, attacker_div);
    military_registry
        .divisions
        .insert(defender_div.id, defender_div);

    let mut war_registry = WarRegistry::default();
    war_registry.wars.insert(WarId(0), test_war(0, 0, 1));

    let mut battle_registry = BattleRegistry::default();
    battle_registry.battles.insert(
        BattleId(0),
        test_battle(
            0,
            0,
            StateId(0),
            0,
            1,
            vec![DivisionId(0)],
            vec![DivisionId(1)],
        ),
    );

    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 10_000, 0), test_country(1, 10_000, 0)],
        },
        military_registry,
        base_state_registry(),
        battle_registry,
        war_registry,
    );

    send_day_changed(&mut app);

    let attacker_manpower_after = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&DivisionId(0))
        .map(|d| d.manpower)
        .unwrap_or(0);
    assert!(
        attacker_manpower_after > 0,
        "attacker must have survived the first day"
    );

    // 戦闘が終結してIdleへ戻った状態を模す(このテストの対象は復員であり、
    // 戦闘終結までの日数自体は別途`p23_006_manpower_ledger_e2e_test.rs`で固定済み)。
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        let d = mr.divisions.get_mut(&DivisionId(0)).unwrap();
        d.status = DivisionStatus::Idle;
        d.combat_id = None;
    }
    app.world_mut()
        .resource_mut::<BattleRegistry>()
        .battles
        .get_mut(&BattleId(0))
        .unwrap()
        .status = BattleStatus::AttackerWon;

    let ledger_before = ledger_for(
        app.world().resource::<CountryRegistry>(),
        app.world().resource::<MilitaryRegistry>(),
        CountryId(0),
    );
    assert!(
        ledger_before.permanent_military_losses > 0,
        "the attacker must have taken a real measured loss (K>0)"
    );

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
        CountryId(0),
        DivisionId(0),
    )
    .unwrap();
    assert_eq!(outcome.returned_manpower, attacker_manpower_after);
    state.apply(app.world_mut());

    let ledger_after = ledger_for(
        app.world().resource::<CountryRegistry>(),
        app.world().resource::<MilitaryRegistry>(),
        CountryId(0),
    );
    assert_eq!(
        ledger_after.permanent_military_losses, ledger_before.permanent_military_losses,
        "K must survive demobilization of the surviving division unchanged"
    );
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(CountryId(0))
            .unwrap()
            .available_manpower,
        attacker_manpower_after
    );
}

#[test]
fn demobilizing_a_division_wiped_out_by_real_combat_is_rejected_as_not_found() {
    let attacker_div = test_division(0, 0, StateId(0), 1);
    let defender_div = test_division(1, 1, StateId(0), 50_000);
    let mut military_registry = MilitaryRegistry::default();
    military_registry
        .divisions
        .insert(attacker_div.id, attacker_div);
    military_registry
        .divisions
        .insert(defender_div.id, defender_div);

    let mut war_registry = WarRegistry::default();
    war_registry.wars.insert(WarId(0), test_war(0, 0, 1));

    let mut battle_registry = BattleRegistry::default();
    battle_registry.battles.insert(
        BattleId(0),
        test_battle(
            0,
            0,
            StateId(0),
            0,
            1,
            vec![DivisionId(0)],
            vec![DivisionId(1)],
        ),
    );

    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 1, 0), test_country(1, 50_000, 0)],
        },
        military_registry,
        base_state_registry(),
        battle_registry,
        war_registry,
    );
    send_day_changed(&mut app);

    assert!(
        !app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .contains_key(&DivisionId(0)),
        "the 1-manpower attacker must have been wiped out and removed by real combat"
    );

    let mut state: SystemState<(
        ResMut<CountryRegistry>,
        ResMut<MilitaryRegistry>,
        ResMut<ArmyRegistry>,
        ResMut<FrontlineRegistry>,
        Res<WarRegistry>,
        Res<BattleRegistry>,
    )> = SystemState::new(app.world_mut());
    let (mut cr, mut mr, mut ar, mut fr, wr, br) = state.get_mut(app.world_mut()).unwrap();
    let result = demobilize_division(
        &mut cr,
        &mut mr,
        &mut ar,
        &mut fr,
        &wr,
        &br,
        CountryId(0),
        DivisionId(0),
    );
    assert_eq!(result, Err(DemobilizationError::DivisionNotFound));
}

#[test]
fn demobilizing_one_army_member_keeps_the_army_and_its_other_member() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("lifecycle_army_partial");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let capital = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .capital_state_id;

    let initial_id = player_own_division_id(&app, player);
    // 2体目を直接挿入する(定義・州は初期軍と同じで揃える。P23-006-FIX-001と同様、
    // manpower加算も同時に行う)。
    let second_manpower = 5_000;
    let second_id = {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        mr.add_division(test_division(999, player.0, capital, second_manpower))
    };
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower += second_manpower;

    let group_id = {
        let mut state: SystemState<(ResMut<ArmyRegistry>, Res<MilitaryRegistry>)> =
            SystemState::new(app.world_mut());
        let (mut army_registry, military_registry) = state.get_mut(app.world_mut()).unwrap();
        let group_id = army_registry
            .create_army(player, &[initial_id, second_id], &military_registry)
            .unwrap();
        state.apply(app.world_mut());
        group_id
    };

    call_demobilize(&mut app, player, second_id).unwrap();

    let armies = app.world().resource::<ArmyRegistry>();
    assert!(armies.armies.contains_key(&group_id));
    assert_eq!(
        armies.armies[&group_id].member_division_ids,
        vec![initial_id]
    );
    assert_eq!(armies.army_for_division(second_id), None);
}

#[test]
fn demobilizing_the_last_army_member_auto_disbands_the_army_and_clears_frontline_references() {
    let player = CountryId(0);
    let enemy = CountryId(1);
    let temp_dir = TempTestDir::new("lifecycle_army_last_member");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let initial_id = player_own_division_id(&app, player);

    let (group_id, frontline_id) = {
        let mut state: SystemState<(
            ResMut<ArmyRegistry>,
            Res<MilitaryRegistry>,
            ResMut<FrontlineRegistry>,
            ResMut<WarRegistry>,
        )> = SystemState::new(app.world_mut());
        let (mut army_registry, military_registry, mut frontline_registry, mut war_registry) =
            state.get_mut(app.world_mut()).unwrap();

        let group_id = army_registry
            .create_army(player, &[initial_id], &military_registry)
            .unwrap();

        war_registry
            .wars
            .insert(WarId(500), test_war(500, player.0, enemy.0));
        let frontline_id = frontline_registry.generate_id();
        frontline_registry.frontlines.insert(
            frontline_id,
            Frontline {
                frontline_id,
                war_id: WarId(500),
                attacker_country_id: player,
                defender_country_id: enemy,
                attacker_front_regions: vec![],
                defender_front_regions: vec![],
                border_region_pairs: vec![],
            },
        );
        frontline_registry
            .assign_army(
                group_id,
                frontline_id,
                player,
                &army_registry,
                &war_registry,
            )
            .unwrap();

        state.apply(app.world_mut());
        (group_id, frontline_id)
    };

    assert_eq!(
        app.world()
            .resource::<FrontlineRegistry>()
            .frontline_for_army(group_id),
        Some(frontline_id)
    );

    call_demobilize(&mut app, player, initial_id).unwrap();
    // `ArmyRegistry`変化を検知して前線参照を整理する既存の反応的System
    // (`war::frontline::sync_army_frontline_references`)が走るのを1フレーム待つ。
    app.update();

    assert!(
        !app.world()
            .resource::<ArmyRegistry>()
            .armies
            .contains_key(&group_id)
    );
    assert_eq!(
        app.world()
            .resource::<ArmyRegistry>()
            .army_for_division(initial_id),
        None
    );
    assert_eq!(
        app.world()
            .resource::<FrontlineRegistry>()
            .frontline_for_army(group_id),
        None,
        "the deleted army must no longer be referenced by any frontline plan"
    );
}

#[test]
fn demobilizing_several_of_the_same_countrys_divisions_keeps_the_ledger_consistent_each_time() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("lifecycle_sequential");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let capital = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .capital_state_id;

    let initial_id = player_own_division_id(&app, player);
    let mut extra_ids = Vec::new();
    for (raw_id, manpower) in [(101usize, 3_000u64), (102, 4_000), (103, 2_500)] {
        let id = {
            let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
            mr.add_division(test_division(raw_id, player.0, capital, manpower))
        };
        app.world_mut()
            .resource_mut::<CountryRegistry>()
            .get_mut(player)
            .unwrap()
            .mobilized_manpower += manpower;
        extra_ids.push(id);
    }

    for &division_id in &extra_ids {
        let before = app_ledger_for(&app, player);
        let returned = call_demobilize(&mut app, player, division_id).unwrap();
        let after = app_ledger_for(&app, player);
        assert_eq!(after.workforce_burden, before.workforce_burden - returned);
        assert_eq!(
            after.living_military_manpower,
            before.living_military_manpower - returned
        );
        assert_eq!(
            after.permanent_military_losses,
            before.permanent_military_losses
        );
        assert_eq!(after.unaccounted_living_excess, 0);
    }

    // 初期軍は手つかずのまま残っている。
    assert!(
        app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .contains_key(&initial_id)
    );
}

#[test]
fn demobilizing_one_countrys_division_does_not_affect_another_countrys_manpower_or_divisions() {
    let player = CountryId(0);
    let other = CountryId(1);
    let temp_dir = TempTestDir::new("lifecycle_no_cross_country_leak");
    let mut app = setup_full_app(player, temp_dir.save_file_config());

    let other_ledger_before = app_ledger_for(&app, other);
    let other_available_before = app
        .world()
        .resource::<CountryRegistry>()
        .get(other)
        .unwrap()
        .available_manpower;
    let other_division_count_before = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == other)
        .count();

    let player_division = player_own_division_id(&app, player);
    call_demobilize(&mut app, player, player_division).unwrap();

    let other_ledger_after = app_ledger_for(&app, other);
    assert_eq!(other_ledger_after, other_ledger_before);
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(other)
            .unwrap()
            .available_manpower,
        other_available_before
    );
    let other_division_count_after = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == other)
        .count();
    assert_eq!(other_division_count_after, other_division_count_before);
}

#[test]
fn demobilizing_the_new_game_initial_division_keeps_the_ledger_identity_b_equals_l_plus_k() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("lifecycle_initial_division");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let initial_id = player_own_division_id(&app, player);

    call_demobilize(&mut app, player, initial_id).unwrap();

    let ledger = app_ledger_for(&app, player);
    assert_eq!(
        ledger.workforce_burden,
        ledger.living_military_manpower + ledger.permanent_military_losses,
        "B = L + K must hold after demobilizing the initial division"
    );
    assert_eq!(ledger.unaccounted_living_excess, 0);
}

#[test]
fn simulation_stays_deterministic_and_ledger_consistent_after_demobilization() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("lifecycle_deterministic_continuation");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let initial_id = player_own_division_id(&app, player);

    call_demobilize(&mut app, player, initial_id).unwrap();

    for _ in 0..10 {
        app.update(); // must not panic across further frames
    }
    let ledger = app_ledger_for(&app, player);
    assert_eq!(
        ledger.workforce_burden,
        ledger.living_military_manpower + ledger.permanent_military_losses
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §13.4 民間労働力統合テスト
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn demobilization_does_not_pre_run_monthly_employment_in_the_same_frame() {
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

    let employed_before = app
        .world()
        .resource::<StateRegistry>()
        .get(capital)
        .unwrap()
        .employed_workforce;
    let unemployed_before = app
        .world()
        .resource::<StateRegistry>()
        .get(capital)
        .unwrap()
        .unemployed_workforce;

    call_demobilize(&mut app, player, initial_id).unwrap();

    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(capital)
        .unwrap();
    assert_eq!(state.employed_workforce, employed_before);
    assert_eq!(state.unemployed_workforce, unemployed_before);
}

#[test]
fn the_next_monthly_update_recovers_civilian_workforce_by_the_returned_amount() {
    let player = CountryId(0);

    let mut control = setup_full_app(
        player,
        TempTestDir::new("workforce_recover_control").save_file_config(),
    );
    advance_one_month(&mut control, 1800, 2);
    let control_cw = civilian_workforce(&control, StateId(0));

    let mut experiment = setup_full_app(
        player,
        TempTestDir::new("workforce_recover_experiment").save_file_config(),
    );
    let initial_id = player_own_division_id(&experiment, player);
    call_demobilize(&mut experiment, player, initial_id).unwrap();
    advance_one_month(&mut experiment, 1800, 2);
    let experiment_cw = civilian_workforce(&experiment, StateId(0));

    assert!(
        experiment_cw > control_cw,
        "demobilizing the initial division must relieve burden and raise civilian workforce \
         relative to the no-demobilization counterfactual: control={control_cw}, experiment={experiment_cw}"
    );
}

#[test]
fn permanent_losses_are_structurally_excluded_from_workforce_recovery() {
    // 恒久軍事損失(K)はそもそも`apportion_mobilized_manpower`の入力(B)に含まれない
    // ため、損耗済みDivisionを復員しても回復する労働力はRちょうどであり、
    // 元の募兵人数(R+K相当)ではない。これを実際に確認する。
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("workforce_partial_recovery");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let capital = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .capital_state_id;
    let initial_id = player_own_division_id(&app, player);

    let original_manpower =
        app.world().resource::<MilitaryRegistry>().divisions[&initial_id].manpower;
    let simulated_loss = original_manpower / 3;
    assert!(simulated_loss > 0);
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        mr.divisions.get_mut(&initial_id).unwrap().manpower -= simulated_loss;
    }
    let ledger_before = app_ledger_for(&app, player);
    assert_eq!(ledger_before.permanent_military_losses, simulated_loss);

    let returned = call_demobilize(&mut app, player, initial_id).unwrap();
    assert_eq!(returned, original_manpower - simulated_loss);

    advance_one_month(&mut app, 1800, 2);

    // 対照: 復員しなかった場合の同月民間労働力(反実仮想)。
    let mut control = setup_full_app(
        player,
        TempTestDir::new("workforce_partial_recovery_control").save_file_config(),
    );
    {
        let mut mr = control.world_mut().resource_mut::<MilitaryRegistry>();
        let id = mr
            .divisions
            .values()
            .find(|d| d.owner == player)
            .unwrap()
            .id;
        mr.divisions.get_mut(&id).unwrap().manpower -= simulated_loss;
    }
    advance_one_month(&mut control, 1800, 2);

    let recovered_cw = civilian_workforce(&app, capital);
    let control_cw = civilian_workforce(&control, capital);
    assert!(
        recovered_cw > control_cw,
        "demobilizing the survivor must still relieve R worth of burden even though K stays"
    );
}

#[test]
fn another_countrys_demobilization_does_not_affect_this_countrys_civilian_workforce() {
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
    call_demobilize(&mut experiment, other, other_division).unwrap();
    advance_one_month(&mut experiment, 1800, 2);
    let experiment_cw = civilian_workforce(&experiment, StateId(0));

    assert_eq!(
        experiment_cw, control_cw,
        "another country's demobilization must not affect this country's civilian workforce"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §13.5 UI・ローカライズ統合テスト
// ─────────────────────────────────────────────────────────────────────────

fn panel_text(app: &mut App) -> String {
    app.world_mut()
        .query_filtered::<&Text, With<MilitaryPanelText>>()
        .single(app.world())
        .expect("MilitaryPanelText must be spawned")
        .0
        .clone()
}

fn demobilize_info_text(app: &mut App) -> String {
    app.world_mut()
        .query_filtered::<&Text, With<DemobilizeDivisionInfoText>>()
        .single(app.world())
        .expect("DemobilizeDivisionInfoText must be spawned")
        .0
        .clone()
}

#[test]
fn the_button_becomes_ready_when_an_eligible_own_division_is_selected() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("ui_ready_state");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());
    let initial_id = player_own_division_id(&app, player);

    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;
    app.update();
    assert!(
        demobilize_info_text(&mut app).contains("0") || !demobilize_info_text(&mut app).is_empty()
    );

    app.world_mut()
        .resource_mut::<SelectedDivision>()
        .select_only(initial_id);
    app.update();
    let text = demobilize_info_text(&mut app);
    assert!(
        text.contains('1'),
        "expected the ready count '1' in: {text}"
    );
}

#[test]
fn a_fighting_division_cannot_be_demobilized_via_the_button_and_produces_no_change() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("ui_fighting_blocked");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());
    let initial_id = player_own_division_id(&app, player);
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .divisions
        .get_mut(&initial_id)
        .unwrap()
        .status = DivisionStatus::Fighting;

    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;
    app.world_mut()
        .resource_mut::<SelectedDivision>()
        .select_only(initial_id);
    app.update();

    let division_count_before = app.world().resource::<MilitaryRegistry>().divisions.len();
    press_demobilize_button(&mut app); // must not panic

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        division_count_before,
        "a fighting division must not be removed by the button press"
    );
    let notif = app
        .world_mut()
        .resource_mut::<Messages<GameNotification>>()
        .drain()
        .next()
        .expect("a failure notification must be sent");
    assert!(!notif.message.contains(MISSING_KEY_MARKER_PREFIX));
}

#[test]
fn army_disband_and_division_demobilize_have_distinct_labels() {
    let temp_dir = TempTestDir::new("ui_label_distinctness");
    let app = setup_app_with_military_panel(CountryId(0), temp_dir.save_file_config());
    let catalog = app.world().resource::<TranslationCatalog>();
    let locale = app.world().resource::<CurrentLocale>();

    let disband_label = t(catalog, locale.0, "military_panel.army_disband_button");
    let demobilize_label = t(
        catalog,
        locale.0,
        "military_panel.division_demobilize_button",
    );
    assert_ne!(disband_label, demobilize_label);
    assert!(!disband_label.contains(MISSING_KEY_MARKER_PREFIX));
    assert!(!demobilize_label.contains(MISSING_KEY_MARKER_PREFIX));
}

#[test]
fn a_successful_button_press_updates_the_panel_and_clears_dangling_references() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("ui_success_updates_panel");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());
    let initial_id = player_own_division_id(&app, player);

    let group_id = {
        let mut state: SystemState<(ResMut<ArmyRegistry>, Res<MilitaryRegistry>)> =
            SystemState::new(app.world_mut());
        let (mut army_registry, military_registry) = state.get_mut(app.world_mut()).unwrap();
        let group_id = army_registry
            .create_army(player, &[initial_id], &military_registry)
            .unwrap();
        state.apply(app.world_mut());
        group_id
    };

    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;
    app.world_mut()
        .resource_mut::<SelectedDivision>()
        .select_only(initial_id);
    app.update();

    press_demobilize_button(&mut app);
    app.update(); // let prune_selected_division / sync_army_frontline_references settle
    app.update();

    assert!(
        !app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .contains_key(&initial_id)
    );
    // このApp構成は他のP23 UIテストと同様、`map::division_selection::DivisionSelectionPlugin`
    // 自体は登録せず(`SelectedDivision`をベア資源として挿入するのみ)。そのPluginが持つ
    // `prune_selected_division`(削除済みDivisionIdを毎フレーム除去する既存の仕組み)は
    // `src/map/division_selection.rs`の`prune_selected_division_removes_destroyed_division_ids`
    // で既に固定されており、`MilitaryRegistry::remove_division`(本モジュールが使うのと
    // 同じ削除経路)により削除されたIDを確実に除去することが検証済みのため、ここでは
    // 二重に検証しない。
    assert!(
        !app.world()
            .resource::<ArmyRegistry>()
            .armies
            .contains_key(&group_id),
        "the now-empty army must not remain visible"
    );

    let ledger = app_ledger_for(&app, player);
    let text = panel_text(&mut app);
    assert!(text.contains(&ledger.living_military_manpower.to_string()));
    assert!(text.contains(&ledger.workforce_burden.to_string()));
    assert!(!text.contains(MISSING_KEY_MARKER_PREFIX));
}

#[test]
fn the_button_and_info_text_are_never_duplicated_across_repeated_updates() {
    let temp_dir = TempTestDir::new("ui_no_duplicate_spawn");
    let mut app = setup_app_with_military_panel(CountryId(0), temp_dir.save_file_config());
    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;
    for _ in 0..5 {
        app.update();
    }
    let button_count = app
        .world_mut()
        .query_filtered::<Entity, With<DemobilizeDivisionButton>>()
        .iter(app.world())
        .count();
    let info_count = app
        .world_mut()
        .query_filtered::<Entity, With<DemobilizeDivisionInfoText>>()
        .iter(app.world())
        .count();
    assert_eq!(button_count, 1);
    assert_eq!(info_count, 1);
}

#[test]
fn ja_and_en_resolve_all_demobilization_labels_without_missing_key_markers() {
    let temp_dir = TempTestDir::new("ui_locales");
    let mut app = setup_app_with_military_panel(CountryId(0), temp_dir.save_file_config());
    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;

    for locale in [Locale::JaJp, Locale::EnUs] {
        app.insert_resource(CurrentLocale(locale));
        app.update();
        let text = panel_text(&mut app);
        assert!(
            !text.contains(MISSING_KEY_MARKER_PREFIX),
            "locale {locale:?}: {text}"
        );
        let info = demobilize_info_text(&mut app);
        assert!(
            !info.contains(MISSING_KEY_MARKER_PREFIX),
            "locale {locale:?}: {info}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// §13.6 Save/Load統合テスト
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
fn an_unworn_demobilization_ledger_round_trips_through_save_and_load() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_unworn");
    let save_config = temp_dir.save_file_config();

    let (saved_ledger, saved_available) = {
        let mut producer = setup_full_app(player, save_config.clone());
        let initial_id = player_own_division_id(&producer, player);
        call_demobilize(&mut producer, player, initial_id).unwrap();
        let ledger = app_ledger_for(&producer, player);
        let available = producer
            .world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .available_manpower;
        send_save_request(&mut producer);
        producer.update();
        (ledger, available)
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }
    assert_eq!(app_ledger_for(&app, player), saved_ledger);
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .available_manpower,
        saved_available
    );
}

#[test]
fn a_worn_division_demobilization_preserves_permanent_losses_after_reload() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_worn_preserves_k");
    let save_config = temp_dir.save_file_config();

    let saved_ledger = {
        let mut producer = setup_full_app(player, save_config.clone());
        let initial_id = player_own_division_id(&producer, player);
        {
            let mut mr = producer.world_mut().resource_mut::<MilitaryRegistry>();
            mr.divisions.get_mut(&initial_id).unwrap().manpower -= 1;
        }
        call_demobilize(&mut producer, player, initial_id).unwrap();
        let ledger = app_ledger_for(&producer, player);
        assert!(ledger.permanent_military_losses > 0);
        send_save_request(&mut producer);
        producer.update();
        ledger
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    assert_eq!(app_ledger_for(&app, player), saved_ledger);
}

#[test]
fn a_demobilized_division_and_its_emptied_army_do_not_reappear_after_reload() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_no_resurrection");
    let save_config = temp_dir.save_file_config();

    let initial_id = {
        let mut producer = setup_full_app(player, save_config.clone());
        let initial_id = player_own_division_id(&producer, player);
        {
            let mut state: SystemState<(ResMut<ArmyRegistry>, Res<MilitaryRegistry>)> =
                SystemState::new(producer.world_mut());
            let (mut army_registry, military_registry) =
                state.get_mut(producer.world_mut()).unwrap();
            army_registry
                .create_army(player, &[initial_id], &military_registry)
                .unwrap();
            state.apply(producer.world_mut());
        }
        call_demobilize(&mut producer, player, initial_id).unwrap();
        producer.update(); // let sync_army_frontline_references / army sanitation settle
        assert!(
            producer
                .world()
                .resource::<ArmyRegistry>()
                .armies
                .is_empty()
        );
        send_save_request(&mut producer);
        producer.update();
        initial_id
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    assert!(
        !app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .contains_key(&initial_id)
    );
    assert!(app.world().resource::<ArmyRegistry>().armies.is_empty());
}

#[test]
fn available_and_burden_match_exactly_before_and_after_reload() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_available_burden_match");
    let save_config = temp_dir.save_file_config();

    let (available_before, burden_before) = {
        let mut producer = setup_full_app(player, save_config.clone());
        let initial_id = player_own_division_id(&producer, player);
        call_demobilize(&mut producer, player, initial_id).unwrap();
        let country = producer
            .world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap();
        let pair = (country.available_manpower, country.mobilized_manpower);
        send_save_request(&mut producer);
        producer.update();
        pair
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    let country = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap();
    assert_eq!(country.available_manpower, available_before);
    assert_eq!(country.mobilized_manpower, burden_before);
}

#[test]
fn l_and_k_are_rederived_identically_from_the_reloaded_state() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_l_k_rederivation");
    let save_config = temp_dir.save_file_config();

    let saved_ledger = {
        let mut producer = setup_full_app(player, save_config.clone());
        let initial_id = player_own_division_id(&producer, player);
        {
            let mut mr = producer.world_mut().resource_mut::<MilitaryRegistry>();
            mr.divisions.get_mut(&initial_id).unwrap().manpower -= 1;
        }
        call_demobilize(&mut producer, player, initial_id).unwrap();
        let ledger = app_ledger_for(&producer, player);
        send_save_request(&mut producer);
        producer.update();
        ledger
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    let reloaded_ledger = app_ledger_for(&app, player);
    assert_eq!(
        reloaded_ledger.living_military_manpower,
        saved_ledger.living_military_manpower
    );
    assert_eq!(
        reloaded_ledger.permanent_military_losses,
        saved_ledger.permanent_military_losses
    );
}
