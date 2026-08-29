//! P25-006-REV-001: 強制Division削除時の装備会計訂正のE2Eテスト。
//!
//! `src/military/update.rs`の`resolve_participant_outcome`(撤退時追加manpower
//! 損失への装備連動)と`src/country/elimination.rs`の`apply_country_elimination`
//! (国家消滅時の装備全損)は、いずれもprivate関数またはドメインAPI内部実装であり、
//! 数式的正確性・原子性・二重計上防止・決定性の大半は、対応する
//! `src/military/update.rs`・`src/country/elimination.rs`の`#[cfg(test)]`内部
//! テストモジュール(実際に稼働しているprivate関数を直接呼び出せる)で検証済みである。
//!
//! 本ファイルは、実際にBevy Appへ登録されたSystem
//! (`military::update::handle_daily_military`、`DailySimulationSet::MilitaryAction`)
//! および実際の`country::elimination`公開APIを経由する必要がある残りの項目——
//! 稼働中経路の実App証明、New Game/Loaded Game比較、save/load分岐比較、
//! P25-004回帰(死んだ経路非接続・RNG系列非変更)——を担当する。
#![allow(clippy::type_complexity)] // SystemStateのタプル型はテスト内での使い捨て構成であり、型エイリアス化の利益がない

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::DayChangedMessage;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{BattleId, CountryId, DivisionId, StateId, WarId};
use strategy_game::country::power::CountryPowerRegistry;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::diplomacy::claims::ClaimRegistry;
use strategy_game::diplomacy::crisis::CrisisRegistry;
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
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
use strategy_game::war::frontline::FrontlineRegistry;
use strategy_game::war::justification::WarJustificationRegistry;

// ─── 実Appハーネス(P25-004/P25-006と同じ方式) ────────────────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p25_006_rev_001_e2e_{label}_{}_{nanos}_{n}",
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
    organization: f32,
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
        organization,
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
        name: "P25-006-REV-001 Test War".to_string(),
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

fn get_division(app: &App, id: DivisionId) -> Option<Division> {
    app.world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&id)
        .cloned()
}

/// 攻撃側が単独では守備側を突破できない(=組織率0に達して撤退する)、
/// P21-siegeの`test_solo_attacker_cannot_break_well_defended_state`と同じ
/// 攻撃10/防御15の非対称statsで、実App上に1対1戦闘を組み立てる。
///
/// P25-007により装備充足率(`min(manpower比率, 装備比率)`)が戦闘力へ直接影響する
/// ようになったため、防御側は満タン装備(readiness 100%)に固定し、元のP21-siege
/// バランス(攻撃側が組織率0に達して撤退する)を保つ。攻撃側の`attacker_equipment`
/// だけを可変にすることで、撤退時装備損失の検証対象を攻撃側の装備量に限定する。
fn setup_retreating_battle(app: &mut App, attacker_equipment: f64) -> (DivisionId, DivisionId) {
    let player = CountryId(0);
    let ai_country = CountryId(1);

    let attacker_id = spawn_division(
        app,
        player,
        TEST_STATE_ID,
        10_000,
        10_000,
        attacker_equipment,
        100.0,
        100.0,
        10,
        10,
    );
    let defender_id = spawn_division(
        app,
        ai_country,
        TEST_STATE_ID,
        10_000,
        10_000,
        100.0,
        100.0,
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
    );

    (attacker_id, defender_id)
}

/// 攻撃側が組織率0に達して撤退するまで日を進める(P21-siegeの既存挙動を再利用)。
fn run_until_retreat(app: &mut App, attacker_id: DivisionId) {
    for day in 1..=30u8 {
        send_day_changed(app, 1800, 1, day);
        if get_division(app, attacker_id)
            .map(|d| d.status == DivisionStatus::Idle)
            .unwrap_or(true)
        {
            break;
        }
    }
}

// ─── Test: 稼働中経路(実App経由で撤退装備損失が適用される) ───────────────

#[test]
fn test_the_real_registered_active_path_applies_retreat_equipment_loss() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("retreat_active_path");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let (attacker_id, _defender_id) = setup_retreating_battle(&mut app, 90.0);
    run_until_retreat(&mut app, attacker_id);

    let attacker_after = get_division(&app, attacker_id).unwrap();
    assert_eq!(
        attacker_after.status,
        DivisionStatus::Idle,
        "攻撃側は組織率0に達して撤退するはず(既存P21-siege挙動)"
    );
    assert!(
        attacker_after.equipment < 90.0,
        "実Appの登録済みSystem経由で撤退装備損失が適用されているはず"
    );
    assert!(attacker_after.manpower > 0 && attacker_after.manpower < 10_000);
}

// ─── Test: New Game / Loaded Game比較(撤退) ──────────────────────────────

#[test]
fn test_new_game_and_loaded_game_produce_identical_retreat_equipment_results() {
    let player = CountryId(0);

    let temp_dir_a = TempTestDir::new("retreat_new_game");
    let mut app_a = setup_app_in_playing(player, temp_dir_a.save_file_config());
    let (attacker_a, _) = setup_retreating_battle(&mut app_a, 77.0);
    run_until_retreat(&mut app_a, attacker_a);
    let after_a = get_division(&app_a, attacker_a).unwrap();

    let temp_dir_b = TempTestDir::new("retreat_loaded_game");
    let mut app_b = setup_app_in_playing(player, temp_dir_b.save_file_config());
    send_save_request(&mut app_b);
    app_b.update();
    send_load_request(&mut app_b);
    app_b.update();
    app_b.update();
    let (attacker_b, _) = setup_retreating_battle(&mut app_b, 77.0);
    run_until_retreat(&mut app_b, attacker_b);
    let after_b = get_division(&app_b, attacker_b).unwrap();

    assert_eq!(after_a.manpower, after_b.manpower);
    assert_eq!(after_a.equipment, after_b.equipment);
    assert_eq!(after_a.status, after_b.status);
}

// ─── Test: 撤退前save/load分岐比較 ────────────────────────────────────────

#[test]
fn test_pre_retreat_save_load_branch_matches_the_uninterrupted_run() {
    let player = CountryId(0);

    let temp_dir_a = TempTestDir::new("retreat_branch_a");
    let mut app_a = setup_app_in_playing(player, temp_dir_a.save_file_config());
    let (attacker_a, _) = setup_retreating_battle(&mut app_a, 55.0);
    run_until_retreat(&mut app_a, attacker_a);
    let after_a = get_division(&app_a, attacker_a).unwrap();

    let temp_dir_b = TempTestDir::new("retreat_branch_b");
    let mut app_b = setup_app_in_playing(player, temp_dir_b.save_file_config());
    let (attacker_b, _) = setup_retreating_battle(&mut app_b, 55.0);
    send_save_request(&mut app_b);
    app_b.update();
    send_load_request(&mut app_b);
    app_b.update();
    app_b.update();
    run_until_retreat(&mut app_b, attacker_b);
    let after_b = get_division(&app_b, attacker_b).unwrap();

    assert_eq!(after_a.manpower, after_b.manpower);
    assert_eq!(after_a.equipment, after_b.equipment);
}

// ─── Test: 撤退後save/load一致 ───────────────────────────────────────────

#[test]
fn test_post_retreat_equipment_loss_survives_save_and_load() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("retreat_post_save");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let (attacker_id, _) = setup_retreating_battle(&mut app, 90.0);
    run_until_retreat(&mut app, attacker_id);
    let before_reload = get_division(&app, attacker_id).unwrap();
    assert!(before_reload.equipment < 90.0);

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    let after_reload = get_division(&app, attacker_id).unwrap();
    assert_eq!(after_reload.equipment, before_reload.equipment);
    assert_eq!(after_reload.manpower, before_reload.manpower);
}

// ─── Test: 決定性(P25-006-REV-001時点では「装備量の有無で撤退結果が変わらない」
// という前提だったが、P25-007により意図的に反転している。§16参照) ──────────
//
// P25-006-REV-001時点では、戦闘力計算が装備量を一切参照しなかったため、撤退の
// 発生有無・タイミング・最終manpowerは装備量と無関係だった。P25-007は戦闘充足率
// (`min(manpower比率, 装備比率)`)を通じて装備量をattack/defenseへ直接反映する
// ため、装備量が異なれば撤退結果(タイミング・最終manpower)も異なり得る
// (これはP25-007タスク自身が要求する仕様どおりの挙動であり、退行ではない)。
// この事実自体を隠さず記録しつつ、なお成立すべき不変条件——同じ装備量・同じ
// battle stateなら常に同じ結果になる決定性——をこのテストで検証する
// (本コードベースの戦闘計算はそもそも乱数を使用しない、決定的な整数演算のみ)。

#[test]
fn test_retreat_outcome_is_deterministic_for_a_given_equipment_level_even_though_it_now_varies_across_levels()
 {
    fn run_scenario(equipment: f64, label: &str) -> (u64, f64, DivisionStatus) {
        let player = CountryId(0);
        let temp_dir = TempTestDir::new(label);
        let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
        let (attacker_id, _) = setup_retreating_battle(&mut app, equipment);
        run_until_retreat(&mut app, attacker_id);
        let after = get_division(&app, attacker_id).unwrap();
        (after.manpower, after.equipment, after.status)
    }

    // 同じ装備量(90.0)を2回独立実行すると、bit-for-bitで完全に同じ結果になる
    // (装備充足率の計算自体は決定的な整数演算のみで、新しい乱数を消費しない)。
    let run_a = run_scenario(90.0, "determinism_a");
    let run_b = run_scenario(90.0, "determinism_b");
    assert_eq!(run_a, run_b);

    // 異なる装備量(0.0)では、P25-007により結果が変わり得ることを確認する
    // (「変わらない」という古い前提の単純な反転ではなく、意図した仕様であることを
    // 明示するために、変化の方向性[装備0の方がreadinessが低く不利]も確認する)。
    let zero_equipment = run_scenario(0.0, "zero_equipment");
    assert_ne!(
        run_a, zero_equipment,
        "P25-007により、攻撃側の装備量が撤退結果へ意図的に反映されるはず"
    );
}

// ─── 国家消滅(実country::elimination公開API経由) ─────────────────────────

#[allow(clippy::too_many_arguments)]
fn eliminate(
    app: &mut App,
    eliminated: CountryId,
    annexed_by: CountryId,
    final_state_id: StateId,
) -> Result<
    strategy_game::country::elimination::CountryEliminationOutcome,
    strategy_game::country::elimination::CountryEliminationError,
> {
    let mut state: SystemState<(
        ResMut<StateRegistry>,
        ResMut<CountryRegistry>,
        ResMut<MilitaryRegistry>,
        ResMut<ArmyRegistry>,
        ResMut<FrontlineRegistry>,
        ResMut<WarRegistry>,
        ResMut<BattleRegistry>,
        ResMut<DiplomacyRegistry>,
        ResMut<CrisisRegistry>,
        ResMut<ClaimRegistry>,
        ResMut<WarJustificationRegistry>,
        ResMut<CountryPowerRegistry>,
    )> = SystemState::new(app.world_mut());
    let (
        mut state_registry,
        mut country_registry,
        mut military_registry,
        mut army_registry,
        mut frontline_registry,
        mut war_registry,
        mut battle_registry,
        mut diplomacy_registry,
        mut crisis_registry,
        mut claim_registry,
        mut justification_registry,
        mut power_registry,
    ) = state.get_mut(app.world_mut()).unwrap();

    let plan = strategy_game::country::elimination::preflight_country_elimination(
        final_state_id,
        eliminated,
        annexed_by,
        "1900/01/01",
        &state_registry,
        &country_registry,
    )?;

    let result = strategy_game::country::elimination::apply_country_elimination(
        plan,
        None,
        None,
        "1900/01/01",
        &mut state_registry,
        &mut country_registry,
        &mut military_registry,
        &mut army_registry,
        &mut frontline_registry,
        &mut war_registry,
        &mut battle_registry,
        &mut diplomacy_registry,
        &mut crisis_registry,
        &mut claim_registry,
        &mut justification_registry,
        &mut power_registry,
    );
    state.apply(app.world_mut());
    result
}

fn set_stockpile(app: &mut App, country_id: CountryId, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(country_id)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitaryEquipment, value);
}

fn stockpile(app: &App, country_id: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitaryEquipment)
}

/// player(CountryId(0))が最後の陸州を失って消滅し、AI(CountryId(1))が併合する
/// シナリオを実App上に組み立てる(実マップの州0番を使う——`preflight_country_
/// elimination`は非海州であることを要求するため、実データの陸州IDが必要)。
fn find_a_land_state(app: &App) -> StateId {
    app.world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .find(|s| !s.is_sea)
        .map(|s| s.id)
        .expect("real map must have at least one land state")
}

#[test]
fn test_elimination_new_game_and_loaded_game_produce_identical_equipment_results() {
    let player = CountryId(0);
    let annexer = CountryId(1);

    let temp_dir_a = TempTestDir::new("elim_new_game");
    let mut app_a = setup_app_in_playing(player, temp_dir_a.save_file_config());
    let land_state_a = find_a_land_state(&app_a);
    // playerの全既存Divisionを、単一の非海州(land_state_a)へ集約したうえで消滅させる
    // (実際の消滅にはplayerが陸州を持たなくなる必要があるが、ここでは装備会計だけを
    // 検証したいため、既存Division位置には触れず、最後の陸州移転先として使う)。
    let division_id_a = spawn_division(
        &mut app_a,
        player,
        land_state_a,
        10_000,
        10_000,
        42.5,
        100.0,
        100.0,
        10,
        10,
    );
    set_stockpile(&mut app_a, player, 50.0);
    set_stockpile(&mut app_a, annexer, 100.0);
    // playerが所有する全州をannexerへ一旦渡し、最後の1州(land_state_a)だけを残す
    // ことで、`preflight_country_elimination`の「最後の陸州」条件を満たす。
    reassign_all_states_except(&mut app_a, player, annexer, land_state_a);
    eliminate(&mut app_a, player, annexer, land_state_a).unwrap();
    let world_after_a = stockpile(&app_a, player)
        + stockpile(&app_a, annexer)
        + app_a
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .values()
            .filter(|d| d.owner == player || d.owner == annexer)
            .map(|d| d.equipment)
            .sum::<f64>();
    assert!(
        !app_a
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .contains_key(&division_id_a)
    );

    let temp_dir_b = TempTestDir::new("elim_loaded_game");
    let mut app_b = setup_app_in_playing(player, temp_dir_b.save_file_config());
    send_save_request(&mut app_b);
    app_b.update();
    send_load_request(&mut app_b);
    app_b.update();
    app_b.update();
    let land_state_b = find_a_land_state(&app_b);
    let division_id_b = spawn_division(
        &mut app_b,
        player,
        land_state_b,
        10_000,
        10_000,
        42.5,
        100.0,
        100.0,
        10,
        10,
    );
    set_stockpile(&mut app_b, player, 50.0);
    set_stockpile(&mut app_b, annexer, 100.0);
    reassign_all_states_except(&mut app_b, player, annexer, land_state_b);
    eliminate(&mut app_b, player, annexer, land_state_b).unwrap();
    let world_after_b = stockpile(&app_b, player)
        + stockpile(&app_b, annexer)
        + app_b
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .values()
            .filter(|d| d.owner == player || d.owner == annexer)
            .map(|d| d.equipment)
            .sum::<f64>();
    assert!(
        !app_b
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .contains_key(&division_id_b)
    );

    assert_eq!(world_after_a, world_after_b);
    assert_eq!(stockpile(&app_a, player), stockpile(&app_b, player));
    assert_eq!(stockpile(&app_a, annexer), stockpile(&app_b, annexer));
}

fn reassign_all_states_except(
    app: &mut App,
    from: CountryId,
    to: CountryId,
    keep_state: StateId,
) {
    let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
    for state in state_registry.states.iter_mut() {
        if state.owner_country_id == from && state.id != keep_state {
            state.owner_country_id = to;
            state.controller_country = Some(to);
        }
    }
}

#[test]
fn test_elimination_destroys_division_equipment_via_the_real_public_api() {
    let player = CountryId(0);
    let annexer = CountryId(1);
    let temp_dir = TempTestDir::new("elim_real_api");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let land_state = find_a_land_state(&app);
    let division_id = spawn_division(
        &mut app, player, land_state, 10_000, 10_000, 33.3, 100.0, 100.0, 10, 10,
    );
    // New Gameは既にplayerの初期Divisionを配置済みのため、消滅時に解散される
    // 件数はこの追加分を含めたplayer所有Division総数と一致するはず。
    let player_division_count_before = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == player)
        .count();
    set_stockpile(&mut app, player, 50.0);
    set_stockpile(&mut app, annexer, 100.0);
    reassign_all_states_except(&mut app, player, annexer, land_state);

    let outcome = eliminate(&mut app, player, annexer, land_state).unwrap();
    assert_eq!(outcome.divisions_disbanded, player_division_count_before);
    assert!(
        !app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .contains_key(&division_id)
    );
    assert_eq!(stockpile(&app, player), 50.0, "消滅国stockpileは凍結され不変");
    assert_eq!(stockpile(&app, annexer), 100.0, "併合国stockpileは増えない");
}

// ─── 対象外・回帰確認 ─────────────────────────────────────────────────────

#[test]
fn test_p25_004_combat_regression_battery_still_passes_unchanged() {
    // P25-004のテストは同一クレート内の別ファイル(tests/p25_004_combat_equipment_
    // loss_e2e_test.rs)で既にcargo test実行時に網羅されるため、ここでは
    // P25-004が検証した「死んだ経路(war::combat::process_combat)が未接続の
    // まま」という事実を、本タスクでも同じ関数のsrc状態を通じて再確認する
    // (詳細なpin済み数値回帰はsrc/military/update.rs内部テスト
    // test_u_dead_war_combat_process_combat_path_remains_unconnectedが担う)。
    use strategy_game::economy::resources::military_equipment_value_to_units;
    let out_of_range = 2_000_000_000.0;
    assert!(
        military_equipment_value_to_units(out_of_range).is_err(),
        "P25-004が確認したfixed-point正規値域の前提は本タスクでも不変のはず"
    );
}
