//! P25-004: 戦闘損耗による軍事装備損失のE2Eテスト。
//!
//! Test A〜S・Uは、正規の損失計算式(`military::equipment::
//! calculate_proportional_equipment_loss`)そのものの正確性(§19の数値例)、
//! および実際の稼働中戦闘経路(`military::update::process_daily_battles`/
//! `resolve_finished_battles`。privateのためこのファイルからは直接呼び出せない)
//! への配線・原子性・決定性・国家分離・死んだ経路(`war::combat::process_combat`)
//! 非接続を検証しており、`src/military/equipment.rs`と`src/military/update.rs`
//! それぞれの`#[cfg(test)]`内部テストモジュールに配置済みである(内部private関数へ
//! 直接アクセスできるため、より正確かつ高速に検証できる)。
//!
//! このファイルは、実際にBevy Appへ登録されたSystem
//! (`military::update::handle_daily_military`、`DailySimulationSet::MilitaryAction`)
//! を経由する必要がある残りの項目——Test T(稼働中経路の実App証明)・V(New Game/
//! Loaded Game比較)・W(戦闘前save/load分岐比較)・X(戦闘後save/load)・
//! Y(装備0での既存戦闘回帰)——を担当する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::DayChangedMessage;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{BattleId, CountryId, DivisionId, StateId, WarId};
use strategy_game::country::{CountryPlugin, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};

// ─── 実Appハーネス(p25_003_recruitment_equipment_issue_e2e_testと同じ方式) ──

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p25_004_e2e_{label}_{}_{nanos}_{n}",
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

/// 州IDは実マップに存在しない値を使う(`state_registry.get`はOptionを返し、
/// 見つからない場合は地形補正0・州占領は無視されるだけで安全なため、テストの
/// 装備損失検証には実マップの州構成が不要)。
const TEST_STATE_ID: StateId = StateId(999_999);

#[allow(clippy::too_many_arguments)]
fn spawn_division(
    app: &mut App,
    owner: CountryId,
    state: StateId,
    manpower: u64,
    max_manpower: u64,
    equipment: f64,
    max_equipment: f64,
    attack: i32,
    defense: i32,
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
        status: DivisionStatus::Fighting,
        def_id: strategy_game::common::DivisionDefinitionId(0),
        attack_power: attack,
        defense_power: defense,
        combat_id: None,
    };
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .add_division(division)
}

fn create_war(app: &mut App, attacker: CountryId, defender: CountryId) -> WarId {
    let war = War {
        id: WarId(0),
        name: "P25-004 Test War".to_string(),
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

#[allow(clippy::too_many_arguments)]
fn create_battle(
    app: &mut App,
    war_id: WarId,
    state_id: StateId,
    attacker_country: CountryId,
    defender_country: CountryId,
    attacker_ids: Vec<DivisionId>,
    defender_ids: Vec<DivisionId>,
    attacker_origin: StateId,
) -> BattleId {
    // P27-FU-001: originはBattleの場所(`state_id`、本ファイルでは実在しない
    // `TEST_STATE_ID`)ではなく、攻撃側Divisionが実際に出撃した実州でなければ
    // ならない。同一値を使うと新しい出撃元検証で「無効なorigin」と判定され、
    // 代替探索も`state_id`不在で即失敗し、Divisionが誤って消滅する。
    let attacker_origins = attacker_ids.iter().map(|id| (*id, attacker_origin)).collect();
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

fn get_division(app: &App, id: DivisionId) -> Option<Division> {
    app.world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&id)
        .cloned()
}

/// P26-006: 首都連結性が戦闘力へ影響するため、Divisionを自国が実際に支配する
/// 実マップ州(`TEST_STATE_ID`ではなく)へ配置するためのヘルパー(P25-007と同じ方式)。
fn state_owned_by(app: &App, owner: CountryId) -> StateId {
    app.world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .find(|s| s.owner_country_id == owner)
        .map(|s| s.id)
        .expect("real map must have at least one state owned by this country")
}

/// player(CountryId(0))・ai_country(CountryId(1))間の1対1戦闘を実App上に組み立てる。
fn setup_standard_battle(app: &mut App) -> (DivisionId, DivisionId) {
    let player = CountryId(0);
    let ai_country = CountryId(1);
    let player_home = state_owned_by(app, player);
    let ai_home = state_owned_by(app, ai_country);

    let attacker_id = spawn_division(
        app,
        player,
        player_home,
        10_000,
        10_000,
        90.0,
        100.0,
        10,
        10,
    );
    let defender_id = spawn_division(
        app,
        ai_country,
        ai_home,
        10_000,
        10_000,
        60.0,
        100.0,
        10,
        15,
    );

    let war_id = create_war(app, player, ai_country);
    create_battle(
        app,
        war_id,
        TEST_STATE_ID,
        player,
        ai_country,
        vec![attacker_id],
        vec![defender_id],
        player_home,
    );

    (attacker_id, defender_id)
}

// ─── Test T: 稼働中経路 ───────────────────────────────────────────────────

#[test]
fn test_t_the_real_registered_active_combat_path_reduces_equipment() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_t");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let (attacker_id, defender_id) = setup_standard_battle(&mut app);
    let attacker_equipment_before = get_division(&app, attacker_id).unwrap().equipment;
    let defender_equipment_before = get_division(&app, defender_id).unwrap().equipment;

    // DayChangedMessageは実際に登録されたSystem(handle_daily_military、
    // DailySimulationSet::MilitaryAction)経由でのみ消費される。
    send_day_changed(&mut app, 1800, 1, 1);

    let attacker_after = get_division(&app, attacker_id).unwrap();
    let defender_after = get_division(&app, defender_id).unwrap();
    assert!(
        attacker_after.equipment < attacker_equipment_before,
        "実Appの登録済みSystem経由で攻撃側の装備が減っているはず"
    );
    assert!(
        defender_after.equipment < defender_equipment_before,
        "実Appの登録済みSystem経由で防御側の装備が減っているはず"
    );
}

// ─── Test V: New Game / Loaded Game比較 ──────────────────────────────────

#[test]
fn test_v_new_game_and_loaded_game_produce_identical_equipment_losses() {
    let player = CountryId(0);

    // New Game由来。
    let temp_dir_a = TempTestDir::new("test_v_a");
    let mut app_a = setup_app_in_playing(player, temp_dir_a.save_file_config());
    let (atk_a, def_a) = setup_standard_battle(&mut app_a);
    send_day_changed(&mut app_a, 1800, 1, 1);
    send_day_changed(&mut app_a, 1800, 1, 2);
    let atk_a_after = get_division(&app_a, atk_a).unwrap();
    let def_a_after = get_division(&app_a, def_a).unwrap();

    // Loaded Game由来: New Game直後に一度save/loadを経てから同じ戦闘を組み立てる。
    let temp_dir_b = TempTestDir::new("test_v_b");
    let mut app_b = setup_app_in_playing(player, temp_dir_b.save_file_config());
    send_save_request(&mut app_b);
    app_b.update();
    send_load_request(&mut app_b);
    app_b.update();
    app_b.update();
    let (atk_b, def_b) = setup_standard_battle(&mut app_b);
    send_day_changed(&mut app_b, 1800, 1, 1);
    send_day_changed(&mut app_b, 1800, 1, 2);
    let atk_b_after = get_division(&app_b, atk_b).unwrap();
    let def_b_after = get_division(&app_b, def_b).unwrap();

    assert_eq!(atk_a_after.manpower, atk_b_after.manpower);
    assert_eq!(atk_a_after.equipment, atk_b_after.equipment);
    assert_eq!(def_a_after.manpower, def_b_after.manpower);
    assert_eq!(def_a_after.equipment, def_b_after.equipment);
}

// ─── Test W: 戦闘前save/load分岐比較 ─────────────────────────────────────

#[test]
fn test_w_pre_combat_save_load_branch_matches_the_uninterrupted_run() {
    let player = CountryId(0);

    // 分岐1: 中断なしで最後まで実行する。
    let temp_dir_a = TempTestDir::new("test_w_a");
    let mut app_a = setup_app_in_playing(player, temp_dir_a.save_file_config());
    let (atk_a, def_a) = setup_standard_battle(&mut app_a);
    send_day_changed(&mut app_a, 1800, 1, 1);
    send_day_changed(&mut app_a, 1800, 1, 2);
    let atk_a_after = get_division(&app_a, atk_a).unwrap();
    let def_a_after = get_division(&app_a, def_a).unwrap();

    // 分岐2: 戦闘開始直後(1日目適用前)にsave/loadを挟んでから続行する。
    let temp_dir_b = TempTestDir::new("test_w_b");
    let mut app_b = setup_app_in_playing(player, temp_dir_b.save_file_config());
    let (atk_b, def_b) = setup_standard_battle(&mut app_b);
    send_save_request(&mut app_b);
    app_b.update();
    send_load_request(&mut app_b);
    app_b.update();
    app_b.update();
    send_day_changed(&mut app_b, 1800, 1, 1);
    send_day_changed(&mut app_b, 1800, 1, 2);
    let atk_b_after = get_division(&app_b, atk_b).unwrap();
    let def_b_after = get_division(&app_b, def_b).unwrap();

    assert_eq!(atk_a_after.manpower, atk_b_after.manpower);
    assert_eq!(atk_a_after.equipment, atk_b_after.equipment);
    assert_eq!(def_a_after.manpower, def_b_after.manpower);
    assert_eq!(def_a_after.equipment, def_b_after.equipment);
}

// ─── Test X: 戦闘後save/load ─────────────────────────────────────────────

#[test]
fn test_x_post_combat_equipment_losses_survive_save_and_load() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_x");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let (attacker_id, defender_id) = setup_standard_battle(&mut app);
    send_day_changed(&mut app, 1800, 1, 1);
    send_day_changed(&mut app, 1800, 1, 2);

    let attacker_before_reload = get_division(&app, attacker_id).unwrap();
    let defender_before_reload = get_division(&app, defender_id).unwrap();
    assert!(attacker_before_reload.equipment < 90.0);
    assert!(defender_before_reload.equipment < 60.0);

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    let attacker_after_reload = get_division(&app, attacker_id).unwrap();
    let defender_after_reload = get_division(&app, defender_id).unwrap();
    assert_eq!(
        attacker_after_reload.equipment, attacker_before_reload.equipment,
        "戦闘で損耗済みの装備量がsave/loadを経ても保持されるはず"
    );
    assert_eq!(
        defender_after_reload.equipment, defender_before_reload.equipment
    );
    assert_eq!(attacker_after_reload.manpower, attacker_before_reload.manpower);
    assert_eq!(defender_after_reload.manpower, defender_before_reload.manpower);
}

// ─── Test Y: 既存戦闘回帰(装備0) ─────────────────────────────────────────

#[test]
fn test_y_zero_equipment_divisions_reproduce_the_pre_p25_004_manpower_and_organization_behavior() {
    let player = CountryId(0);
    let ai_country = CountryId(1);
    let temp_dir = TempTestDir::new("test_y");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    // P26-006: 首都連結性が戦闘力へ影響するため、実マップ州(自国支配)へ配置する。
    let player_home = state_owned_by(&app, player);
    let ai_home = state_owned_by(&app, ai_country);

    let attacker_id = spawn_division(
        &mut app,
        player,
        player_home,
        10_000,
        10_000,
        0.0,
        0.0,
        10,
        10,
    );
    let defender_id = spawn_division(
        &mut app,
        ai_country,
        ai_home,
        10_000,
        10_000,
        0.0,
        0.0,
        10,
        15,
    );
    let war_id = create_war(&mut app, player, ai_country);
    create_battle(
        &mut app,
        war_id,
        TEST_STATE_ID,
        player,
        ai_country,
        vec![attacker_id],
        vec![defender_id],
        player_home,
    );

    // src/military/update.rs::tests::test_solo_attacker_cannot_break_well_defended_state
    // と同じ攻撃10/防御15の非対称statsを使う。装備が常に0でも、manpower/organization/
    // 勝敗の挙動はP25-004以前と一致するはず(=攻撃側が組織率0に達して撃退される)。
    //
    // 戦闘が同日中に決着すると、handle_daily_military内でresolve_finished_battles
    // 直後にbattle_registry.cleanup_finished_battles()が走り、当該Battleは
    // BattleRegistryから即座に除去される(BattleStatusを読み取る猶予がない)ため、
    // 決着後に残るDivision状態(生存/撤退/current_state)で決着を判定する。
    for day in 1..=30u8 {
        send_day_changed(&mut app, 1800, 1, day);
        let still_ongoing = app
            .world()
            .resource::<BattleRegistry>()
            .battles
            .get(&BattleId(0))
            .map(|b| b.status == BattleStatus::Ongoing)
            .unwrap_or(false);
        if !still_ongoing {
            break;
        }
    }

    let attacker_after = get_division(&app, attacker_id);
    let defender_after = get_division(&app, defender_id);
    assert!(
        attacker_after.is_some(),
        "撃退された攻撃側は撃破ではなく生存して撤退するはず(装備0でも変わらない)"
    );
    assert!(
        defender_after.is_some(),
        "防御側は撃退に成功して生存し続けるはず(装備0でも変わらない)"
    );
    let attacker_after = attacker_after.unwrap();
    let defender_after = defender_after.unwrap();
    assert_eq!(
        attacker_after.status,
        DivisionStatus::Idle,
        "攻撃側は組織率0に達して撤退し、Idleへ戻るはず"
    );
    assert_eq!(attacker_after.current_state, player_home, "撤退元(=出撃元)へ戻るはず");
    assert_eq!(attacker_after.equipment, 0.0);
    assert!(attacker_after.manpower > 0);
    assert_eq!(defender_after.equipment, 0.0);
    assert!(defender_after.manpower > 0);
}
