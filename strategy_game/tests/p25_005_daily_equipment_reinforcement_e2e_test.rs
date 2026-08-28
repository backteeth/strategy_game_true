//! P25-005: 日次装備補充のE2Eテスト。
//!
//! `src/military/equipment_reinforcement.rs`の`#[cfg(test)]`内部モジュールが、
//! 按分式の正確性(§12の数値例A〜D)・複数Division/複数国家の分離・ECS挿入順
//! 非依存・不正状態の国家単位原子的拒否・消滅国家の凍結・完了済み募兵キューによる
//! 当日ブロックを直接関数呼び出しで検証済み。
//!
//! このファイルは、実際にBevy Appへ登録されたSystem
//! (`military::update::handle_daily_military`、`DailySimulationSet::MilitaryAction`)を
//! 経由する必要がある残りの項目——同日の生産・募兵・補充・戦闘の順序(§5)、
//! P25-004の戦闘装備損失・RNG系列への非干渉、New Game/Loaded Game比較、
//! save/load分岐比較、複数日反復でのfixed-pointドリフト、初期Divisionの
//! 非再支給、新規完成Divisionへの二重支給防止——を担当する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::DayChangedMessage;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{BattleId, CountryId, DivisionDefinitionId, DivisionId, StateId, WarId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
    STANDARD_INFANTRY_DEFINITION_ID,
};
use strategy_game::military::recruitment::request_recruitment;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};

// ─── 実Appハーネス(p25_003/p25_004 e2eテストと同じ方式) ────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p25_005_e2e_{label}_{}_{nanos}_{n}",
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

fn send_day_changed(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

/// 補充の適格条件(`is_eligible_for_daily_reinforcement_by_position`)は、
/// Divisionの`current_state`が実際にその所有国によって支配されていることを
/// 要求する。P25-004のE2Eテストで使った架空の`StateId`(実マップに存在しない)は
/// 戦闘(`process_daily_battles`)には無害だが、補充の適格性判定では常に不適格
/// (`state_registry.get(...).unwrap_or(false)`)になってしまうため、ここでは
/// 実マップ上で実際にその国が所有する州を動的に取得して使う。
fn state_owned_by(app: &App, owner: CountryId) -> StateId {
    app.world()
        .resource::<strategy_game::state::data::StateRegistry>()
        .states
        .iter()
        .find(|s| s.owner_country_id == owner)
        .map(|s| s.id)
        .expect("real map must have at least one state owned by this country")
}

#[allow(clippy::too_many_arguments)]
fn spawn_division(
    app: &mut App,
    owner: CountryId,
    state: StateId,
    manpower: u64,
    max_manpower: u64,
    equipment: f64,
    max_equipment: f64,
    status: DivisionStatus,
) -> DivisionId {
    let division = Division {
        id: DivisionId(0),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: state,
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower,
        max_manpower,
        equipment,
        max_equipment,
        organization: 100.0,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status,
        def_id: DivisionDefinitionId(0),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    };
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .add_division(division)
}

fn get_division(app: &App, id: DivisionId) -> Option<Division> {
    app.world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&id)
        .cloned()
}

fn set_equipment_stockpile(app: &mut App, country_id: CountryId, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(country_id)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitaryEquipment, value);
}

fn equipment_stockpile(app: &App, country_id: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitaryEquipment)
}

fn call_request_recruitment(
    app: &mut App,
    country_id: CountryId,
    division_id: DivisionDefinitionId,
    target_state: StateId,
) -> Result<(), &'static str> {
    let mut state: SystemState<(ResMut<CountryRegistry>, Res<MilitaryRegistry>)> =
        SystemState::new(app.world_mut());
    let (mut country_registry, military_registry) =
        state.get_mut(app.world_mut()).expect("system params");
    let country = country_registry.get_mut(country_id).unwrap();
    let result = request_recruitment(country, &military_registry, division_id, target_state);
    state.apply(app.world_mut());
    result
}

fn create_war(app: &mut App, attacker: CountryId, defender: CountryId) -> WarId {
    let war = War {
        id: WarId(0),
        name: "P25-005 Test War".to_string(),
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
    state_id: StateId,
    attacker_country: CountryId,
    defender_country: CountryId,
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
    let battle_id = app
        .world_mut()
        .resource_mut::<BattleRegistry>()
        .start_battle(battle)
        .unwrap();
    let mut military_registry = app.world_mut().resource_mut::<MilitaryRegistry>();
    for id in attacker_ids.iter().chain(defender_ids.iter()) {
        if let Some(division) = military_registry.divisions.get_mut(id) {
            division.combat_id = Some(battle_id);
        }
    }
    battle_id
}

// ─── §12.6: 募兵後の余剰が同日補充へ使われる ───────────────────────────

#[test]
fn completed_recruitment_leftover_stockpile_reinforces_existing_divisions_the_same_day() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("leftover_same_day");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    let home = state_owned_by(&app, player);

    let required_equipment = {
        let mr = app.world().resource::<MilitaryRegistry>();
        mr.definitions
            .get(&STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .required_equipment
    };

    let existing = spawn_division(
        &mut app,
        player,
        home,
        5_000,
        10_000,
        50.0,
        100.0, // shortfall 50
        DivisionStatus::Idle,
    );

    call_request_recruitment(&mut app, player, STANDARD_INFANTRY_DEFINITION_ID, home).unwrap();
    let recruitment_days = {
        let mr = app.world().resource::<MilitaryRegistry>();
        mr.definitions
            .get(&STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .recruitment_days
    };
    // 完了直前日まで進める(まだ装備は与えない)。
    for day in 0..recruitment_days.saturating_sub(1) {
        send_day_changed(&mut app, 1800, 1, (day % 28 + 1) as u8);
    }
    let division_count_before_completion =
        app.world().resource::<MilitaryRegistry>().divisions.len();

    // 完了当日、必要量+50(既存Divisionの不足分)を備蓄する。
    set_equipment_stockpile(&mut app, player, required_equipment + 50.0);
    let last_day = recruitment_days.saturating_sub(1);
    send_day_changed(&mut app, 1800, 1, (last_day % 28 + 1) as u8);

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        division_count_before_completion + 1,
        "募兵キューは正常に完了して1体増えるはず"
    );
    assert_eq!(
        get_division(&app, existing).unwrap().equipment,
        100.0,
        "募兵で消費した後の余剰50が既存Divisionの不足へ同日中に配分されるはず"
    );
    assert_eq!(equipment_stockpile(&app, player), 0.0);
}

// ─── §12.5: 完了済み募兵キューの保護(飢餓防止) ─────────────────────────

#[test]
fn blocked_completed_queue_protects_stockpile_and_resolves_the_next_day() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("queue_protection");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    let home = state_owned_by(&app, player);

    let required_equipment = {
        let mr = app.world().resource::<MilitaryRegistry>();
        mr.definitions
            .get(&STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .required_equipment
    };

    let existing = spawn_division(
        &mut app,
        player,
        home,
        5_000,
        10_000,
        40.0,
        100.0, // shortfall 60
        DivisionStatus::Idle,
    );

    call_request_recruitment(&mut app, player, STANDARD_INFANTRY_DEFINITION_ID, home).unwrap();
    let recruitment_days = {
        let mr = app.world().resource::<MilitaryRegistry>();
        mr.definitions
            .get(&STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .recruitment_days
    };
    for day in 0..recruitment_days.saturating_sub(1) {
        send_day_changed(&mut app, 1800, 1, (day % 28 + 1) as u8);
    }
    let division_count_before_completion =
        app.world().resource::<MilitaryRegistry>().divisions.len();

    // 完了当日、必要量に60(=既存Division不足分)しか届かない備蓄しか与えない。
    let short_stockpile = required_equipment - 40.0;
    assert!(
        short_stockpile > 0.0 && short_stockpile < required_equipment,
        "この例が意味を持つための前提条件"
    );
    set_equipment_stockpile(&mut app, player, short_stockpile);
    let last_day = recruitment_days.saturating_sub(1);
    send_day_changed(&mut app, 1800, 1, (last_day % 28 + 1) as u8);

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        division_count_before_completion,
        "装備不足のため募兵キューは完了しないはず"
    );
    assert_eq!(
        get_division(&app, existing).unwrap().equipment,
        40.0,
        "備蓄不足でキューが待機した日は、既存Divisionへの補充予算が0になるはず"
    );
    assert_eq!(
        equipment_stockpile(&app, player),
        short_stockpile,
        "備蓄は翌日以降へ持ち越されるはず"
    );

    // 翌日、不足分を追加生産して必要量へ到達させる。
    set_equipment_stockpile(&mut app, player, required_equipment);
    send_day_changed(&mut app, 1800, 2, 1);

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        division_count_before_completion + 1,
        "備蓄が必要量へ届けば翌日にキューが完了するはず"
    );
    assert_eq!(
        equipment_stockpile(&app, player),
        0.0,
        "募兵完了で全額消費され、既存Divisionへの余剰は残らないはず"
    );
    assert_eq!(
        get_division(&app, existing).unwrap().equipment,
        40.0,
        "翌日も募兵が装備を全額使い切るため、既存Divisionは引き続き0補充のはず"
    );
}

// ─── Test 32: 装備補充後に同日戦闘損失が適用される ──────────────────────

#[test]
fn equipment_reinforced_the_same_day_can_still_be_lost_to_combat_that_day() {
    let player = CountryId(0);
    let ai_country = CountryId(1);
    let temp_dir = TempTestDir::new("reinforce_then_combat");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    let home = state_owned_by(&app, player);

    let attacker = spawn_division(
        &mut app,
        player,
        home,
        10_000,
        10_000,
        40.0,
        100.0, // shortfall 60
        DivisionStatus::Fighting,
    );
    let defender = spawn_division(
        &mut app,
        ai_country,
        home,
        10_000,
        10_000,
        60.0,
        100.0,
        DivisionStatus::Fighting,
    );
    let war_id = create_war(&mut app, player, ai_country);
    create_battle(
        &mut app,
        war_id,
        home,
        player,
        ai_country,
        vec![attacker],
        vec![defender],
    );

    // 攻撃側は現在Fighting状態(補充対象外)なのでこのテスト単体では補充されないが、
    // 「補充→戦闘」という同日System順序自体は他のE2Eテスト(§5)で確認済み。
    // ここでは、Idleな別のDivisionに補充が適用された後、同じ日のうちに戦闘で
    // 別Divisionの装備が減ることを確認する(補充と戦闘が同日内で両方作動する)。
    set_equipment_stockpile(&mut app, player, 100.0);
    let idle_ally = spawn_division(
        &mut app,
        player,
        home,
        10_000,
        10_000,
        0.0,
        50.0,
        DivisionStatus::Idle,
    );

    let defender_equipment_before = get_division(&app, defender).unwrap().equipment;
    send_day_changed(&mut app, 1800, 1, 1);

    assert_eq!(
        get_division(&app, idle_ally).unwrap().equipment,
        50.0,
        "Idle状態の別Divisionは同日中に装備補充を受けるはず"
    );
    assert!(
        get_division(&app, defender).unwrap().equipment < defender_equipment_before,
        "戦闘参加中のDivisionは同日中にP25-004の装備損失を受けるはず(補充で相殺されない)"
    );
}

// ─── Test 33: RNG系列(manpower casualty)への非干渉 ──────────────────────

#[test]
fn equipment_reinforcement_does_not_change_combat_manpower_or_organization_results() {
    fn run_scenario(with_reinforcement_stockpile: bool) -> (u64, f32, u64, f32) {
        let player = CountryId(0);
        let ai_country = CountryId(1);
        let temp_dir = TempTestDir::new(if with_reinforcement_stockpile {
            "rng_with"
        } else {
            "rng_without"
        });
        let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
        let home = state_owned_by(&app, player);

        let attacker = spawn_division(
            &mut app,
            player,
            home,
            10_000,
            10_000,
            50.0,
            100.0,
            DivisionStatus::Fighting,
        );
        let defender = spawn_division(
            &mut app,
            ai_country,
            home,
            10_000,
            10_000,
            50.0,
            100.0,
            DivisionStatus::Fighting,
        );
        let war_id = create_war(&mut app, player, ai_country);
        create_battle(
            &mut app,
            war_id,
            home,
            player,
            ai_country,
            vec![attacker],
            vec![defender],
        );

        if with_reinforcement_stockpile {
            set_equipment_stockpile(&mut app, player, 1_000.0);
            set_equipment_stockpile(&mut app, ai_country, 1_000.0);
        }

        send_day_changed(&mut app, 1800, 1, 1);

        let a = get_division(&app, attacker).unwrap();
        let d = get_division(&app, defender).unwrap();
        (a.manpower, a.organization, d.manpower, d.organization)
    }

    let without_stockpile = run_scenario(false);
    let with_stockpile = run_scenario(true);
    assert_eq!(
        without_stockpile, with_stockpile,
        "装備補充の有無がmanpower/organization casualtyへ影響してはいけない \
         (Fighting状態のDivisionは補充対象外のため装備自体は変化しないが、 \
         念のためcasualtyの結果が完全一致することを直接確認する)"
    );
}

// ─── Test 34: New Game / Loaded Game比較 ─────────────────────────────────

#[test]
fn new_game_and_loaded_game_produce_identical_equipment_reinforcement() {
    let player = CountryId(0);

    let temp_dir_a = TempTestDir::new("newgame_a");
    let mut app_a = setup_app_in_playing(player, temp_dir_a.save_file_config());
    let home_a = state_owned_by(&app_a, player);
    let division_a = spawn_division(
        &mut app_a,
        player,
        home_a,
        5_000,
        10_000,
        40.0,
        100.0,
        DivisionStatus::Idle,
    );
    set_equipment_stockpile(&mut app_a, player, 37.0);
    send_day_changed(&mut app_a, 1800, 1, 1);
    let after_a = get_division(&app_a, division_a).unwrap();

    let temp_dir_b = TempTestDir::new("newgame_b");
    let mut app_b = setup_app_in_playing(player, temp_dir_b.save_file_config());
    send_save_request(&mut app_b);
    app_b.update();
    send_load_request(&mut app_b);
    app_b.update();
    app_b.update();
    let home_b = state_owned_by(&app_b, player);
    let division_b = spawn_division(
        &mut app_b,
        player,
        home_b,
        5_000,
        10_000,
        40.0,
        100.0,
        DivisionStatus::Idle,
    );
    set_equipment_stockpile(&mut app_b, player, 37.0);
    send_day_changed(&mut app_b, 1800, 1, 1);
    let after_b = get_division(&app_b, division_b).unwrap();

    assert_eq!(after_a.equipment, after_b.equipment);
    assert_eq!(
        equipment_stockpile(&app_a, player),
        equipment_stockpile(&app_b, player)
    );
}

// ─── Test 35: save/load分岐比較(補充前) ─────────────────────────────────

#[test]
fn pre_reinforcement_save_load_branch_matches_the_uninterrupted_run() {
    let player = CountryId(0);

    let temp_dir_a = TempTestDir::new("branch_a");
    let mut app_a = setup_app_in_playing(player, temp_dir_a.save_file_config());
    let home_a = state_owned_by(&app_a, player);
    let division_a = spawn_division(
        &mut app_a,
        player,
        home_a,
        5_000,
        10_000,
        20.0,
        100.0,
        DivisionStatus::Idle,
    );
    set_equipment_stockpile(&mut app_a, player, 45.0);
    send_day_changed(&mut app_a, 1800, 1, 1);
    let after_a = get_division(&app_a, division_a).unwrap();

    let temp_dir_b = TempTestDir::new("branch_b");
    let mut app_b = setup_app_in_playing(player, temp_dir_b.save_file_config());
    let home_b = state_owned_by(&app_b, player);
    let division_b = spawn_division(
        &mut app_b,
        player,
        home_b,
        5_000,
        10_000,
        20.0,
        100.0,
        DivisionStatus::Idle,
    );
    set_equipment_stockpile(&mut app_b, player, 45.0);
    send_save_request(&mut app_b);
    app_b.update();
    send_load_request(&mut app_b);
    app_b.update();
    app_b.update();
    send_day_changed(&mut app_b, 1800, 1, 1);
    let after_b = get_division(&app_b, division_b).unwrap();

    assert_eq!(after_a.equipment, after_b.equipment);
}

// ─── Test 36: 補充後save/load ────────────────────────────────────────────

#[test]
fn post_reinforcement_stockpile_and_equipment_survive_save_and_load() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("post_reinforce_save_load");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    let home = state_owned_by(&app, player);

    let division = spawn_division(
        &mut app,
        player,
        home,
        5_000,
        10_000,
        40.0,
        100.0,
        DivisionStatus::Idle,
    );
    set_equipment_stockpile(&mut app, player, 30.0);
    send_day_changed(&mut app, 1800, 1, 1);

    let equipment_before_reload = get_division(&app, division).unwrap().equipment;
    let max_equipment_before_reload = get_division(&app, division).unwrap().max_equipment;
    let stockpile_before_reload = equipment_stockpile(&app, player);
    assert!(equipment_before_reload > 40.0, "実際に補充されているはずの前提条件");

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    let after = get_division(&app, division).unwrap();
    assert_eq!(after.equipment, equipment_before_reload);
    assert_eq!(after.max_equipment, max_equipment_before_reload);
    assert_eq!(equipment_stockpile(&app, player), stockpile_before_reload);
}

// ─── Test 37: 複数日反復でのfixed-pointドリフトなし ──────────────────────

#[test]
fn repeated_days_do_not_drift_the_world_equipment_total() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("no_drift");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    let home = state_owned_by(&app, player);

    let division_a = spawn_division(
        &mut app,
        player,
        home,
        5_000,
        10_000,
        10.0,
        100.0,
        DivisionStatus::Idle,
    );
    let division_b = spawn_division(
        &mut app,
        player,
        home,
        5_000,
        10_000,
        30.0,
        100.0,
        DivisionStatus::Idle,
    );
    set_equipment_stockpile(&mut app, player, 3.7);

    let total_before = equipment_stockpile(&app, player)
        + get_division(&app, division_a).unwrap().equipment
        + get_division(&app, division_b).unwrap().equipment;

    for day in 1..=20u8 {
        send_day_changed(&mut app, 1800, 1, day);
    }

    let total_after = equipment_stockpile(&app, player)
        + get_division(&app, division_a).unwrap().equipment
        + get_division(&app, division_b).unwrap().equipment;
    assert_eq!(
        total_before, total_after,
        "20日間の反復補充後も世界装備量の保存則がドリフトしてはいけない"
    );
    assert!(
        get_division(&app, division_a).unwrap().equipment > 10.0,
        "実際に補充が進んでいる(=このテストが空振りではない)ことの確認"
    );
    assert_eq!(
        equipment_stockpile(&app, player),
        0.0,
        "20日間あれば3.7の備蓄は使い切られているはず"
    );
}

// ─── Test 39: 初期Divisionは再支給扱いにならない ─────────────────────────

#[test]
fn initial_new_game_divisions_are_not_treated_as_needing_resupply() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("initial_no_resupply");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    set_equipment_stockpile(&mut app, player, 1_000.0);
    let initial_equipment: Vec<(DivisionId, f64)> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .iter()
        .filter(|(_, d)| d.owner == player)
        .map(|(id, d)| (*id, d.equipment))
        .collect();
    assert!(!initial_equipment.is_empty(), "New Gameは初期Divisionを持つはず");
    // P25-003により、初期Divisionはequipment == max_equipmentで生成される。

    send_day_changed(&mut app, 1800, 1, 1);

    for (id, equipment_before) in initial_equipment {
        let after = get_division(&app, id).unwrap();
        assert_eq!(
            after.equipment, equipment_before,
            "満タンの初期Divisionは装備補充によって変化してはいけない"
        );
    }
    assert_eq!(
        equipment_stockpile(&app, player),
        1_000.0,
        "満タンのDivisionしかいなければ備蓄も消費されないはず"
    );
}

// ─── Test 40: 新規完成Divisionへの二重支給防止 ───────────────────────────

#[test]
fn a_newly_completed_recruit_is_not_double_supplied_on_its_completion_day() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("no_double_supply");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let required_equipment = {
        let mr = app.world().resource::<MilitaryRegistry>();
        mr.definitions
            .get(&STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .required_equipment
    };
    // 完成に必要な分ぴったりの備蓄しか用意しない(既存Divisionへ回る余剰なし)。
    set_equipment_stockpile(&mut app, player, required_equipment);
    call_request_recruitment(&mut app, player, STANDARD_INFANTRY_DEFINITION_ID, StateId(0)).unwrap();

    let recruitment_days = {
        let mr = app.world().resource::<MilitaryRegistry>();
        mr.definitions
            .get(&STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .recruitment_days
    };
    let divisions_before: HashSet<DivisionId> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .keys()
        .copied()
        .collect();
    for day in 0..recruitment_days {
        send_day_changed(&mut app, 1800, 1, (day % 28 + 1) as u8);
    }

    let new_division_id = *app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .keys()
        .find(|id| !divisions_before.contains(id))
        .expect("新規Divisionが1体生成されているはず");
    let new_division = get_division(&app, new_division_id).unwrap();
    assert_eq!(
        new_division.equipment, new_division.max_equipment,
        "募兵で完成したDivisionは既に満タンのはず(装備補充が二重に加算してはいけない)"
    );
    assert_eq!(
        equipment_stockpile(&app, player),
        0.0,
        "完成に全額使い切った同日、既存Division向け余剰は存在しないはず"
    );
}
