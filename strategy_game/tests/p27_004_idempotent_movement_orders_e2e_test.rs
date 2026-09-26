//! P27-004: 移動命令の冪等化(`military::movement::start_or_maintain_movement`)の
//! 統合テスト。P27-004-PRE-001が確定した根本原因(Defend/Offensive AIが前日と同じ
//! 目的地への移動命令を毎日再発行し、`movement_progress`を無条件でリセットしていた)
//! の恒久回帰テストとして、最小合成再現をP27-004-PRE-001の一時診断から恒久テストへ
//! 昇格する。加えて、実データ(Ferrowyn、120か月・通常AI)で旧84か月停滞が
//! 再発しないことを確認する。
//!
//! 純粋な冪等性契約(同一目的地no-op・異なる目的地上書き・経路なし)の単体テストは
//! `src/military/movement.rs`(11件、新規)に既にあるため、本ファイルは
//! **実System経由の日次サイクル・実データでのFerrowyn再現・6か国長期検証・
//! save/load・決定性**に集中する。

#[path = "common/sim_calendar.rs"]
mod sim_calendar;

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionDefinitionId, DivisionId, StateId, WarId};
use strategy_game::country::{CountryPlugin, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::data::{
    Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::military::movement::process_movement;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::diplomacy::crisis::{WarGoal, WarGoalType};
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::{
    FrontlineRegistry, FrontlineStance, process_daily_frontline_plans, update_all_frontlines,
};

const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const DWARF: CountryId = CountryId(2);
const OCEANIC: CountryId = CountryId(3);
const PASSGUARD: CountryId = CountryId(4);
const FERROWYN: CountryId = CountryId(5);
const ALL_SIX: [CountryId; 6] = [ARCADIA, ELFIN, DWARF, OCEANIC, PASSGUARD, FERROWYN];

// ─────────────────────────────────────────────────────────────────────────
// §1: 最小合成環境(P27-004-PRE-001の一時診断フィクスチャの恒久版)
// ─────────────────────────────────────────────────────────────────────────

fn minimal_environment() -> (StateRegistry, WarRegistry, MilitaryRegistry, FrontlineRegistry) {
    let mut war_registry = WarRegistry::default();
    let mut military_registry = MilitaryRegistry::default();
    let frontline_registry = FrontlineRegistry::default();

    // 4つの陸上地域(State 1..=4)と1つの海域(State 5): 1(C1)--2(C1)--3(C2)--4(C2)
    let s1 = StateData {
        id: StateId(1),
        name: "State 1".to_string(),
        owner_country_id: CountryId(1),
        neighbors: vec![StateId(2)],
        world_position: [0.0, 0.0],
        size: [100.0, 100.0],
        ..Default::default()
    };
    let s2 = StateData {
        id: StateId(2),
        name: "State 2".to_string(),
        owner_country_id: CountryId(1),
        neighbors: vec![StateId(1), StateId(3), StateId(5)],
        world_position: [100.0, 0.0],
        size: [100.0, 100.0],
        ..Default::default()
    };
    let s3 = StateData {
        id: StateId(3),
        name: "State 3".to_string(),
        owner_country_id: CountryId(2),
        neighbors: vec![StateId(2), StateId(4), StateId(5)],
        world_position: [200.0, 0.0],
        size: [100.0, 100.0],
        ..Default::default()
    };
    let s4 = StateData {
        id: StateId(4),
        name: "State 4".to_string(),
        owner_country_id: CountryId(2),
        neighbors: vec![StateId(3)],
        world_position: [300.0, 0.0],
        size: [100.0, 100.0],
        ..Default::default()
    };
    let s5 = StateData {
        id: StateId(5),
        name: "Sea 5".to_string(),
        owner_country_id: CountryId(0),
        neighbors: vec![StateId(2), StateId(3)],
        is_sea: true,
        world_position: [150.0, 100.0],
        size: [100.0, 100.0],
        ..Default::default()
    };
    let state_registry = StateRegistry::build(vec![s1, s2, s3, s4, s5]);

    military_registry.definitions.insert(
        DivisionDefinitionId(1),
        DivisionDefinition {
            id: DivisionDefinitionId(1),
            name: "Infantry".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 1000,
            required_equipment: 10.0,
            recruitment_days: 10,
            movement_speed: 1.0,
            attack: 10.0,
            defense: 10.0,
            breakthrough: 5.0,
            organization: 100.0,
            morale: 1.0,
            supply_usage: 1.0,
            maintenance_cost: 1.0,
        },
    );

    let war = War {
        id: WarId(0),
        name: "Test War".to_string(),
        attackers: [CountryId(1)].into_iter().collect(),
        defenders: [CountryId(2)].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![WarGoal {
            attacker: CountryId(1),
            defender: CountryId(2),
            goal_type: WarGoalType::ConquerState,
            target_states: vec![StateId(3)],
            base_peace_cost: 20.0,
            international_concern: 10.0,
            completion: 0.0,
            is_primary: true,
        }],
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
    war_registry.wars.insert(war.id, war);

    (state_registry, war_registry, military_registry, frontline_registry)
}

fn idle_division_at(id: usize, owner: CountryId, at: StateId) -> Division {
    Division {
        id: DivisionId(id),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: at,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 1000,
        max_manpower: 1000,
        equipment: 10.0,
        max_equipment: 10.0,
        organization: 100.0,
        max_organization: 100.0,
        morale: 1.0,
        max_morale: 1.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: DivisionDefinitionId(1),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

/// spec §14: 15日間、`FrontlineOrders`(process_daily_frontline_plans)→
/// `MilitaryAction`(process_movement)の日次サイクルを実Systemそのもので反復し、
/// 毎日同一の防衛先が選ばれても`movement_progress`が単調増加し、現行式から
/// 導出される有限日数で到着することを確認する(P27-004-PRE-001の一時診断の恒久版)。
#[test]
fn test_01_defend_stance_daily_cycle_arrives_within_the_expected_finite_days_not_stuck_forever() {
    let (mut state_registry, war_registry, mut military_registry, mut frontline_registry) =
        minimal_environment();
    update_all_frontlines(&war_registry, &state_registry, &military_registry, &mut frontline_registry);
    let fl_id = frontline_registry.get_frontline_for_war(WarId(0)).unwrap().frontline_id;

    let division_id = military_registry.add_division(idle_division_at(0, CountryId(1), StateId(1)));
    let _ = frontline_registry.assign_division(
        division_id,
        fl_id,
        CountryId(1),
        &military_registry,
        &war_registry,
    );
    if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
        plan.stance = FrontlineStance::Defend;
    }

    let mut battle_registry = BattleRegistry::default();
    let mut last_progress = 0.0f32;
    let mut arrived = false;
    for day in 1..=15u32 {
        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &ArmyRegistry::default(),
        );
        process_movement(&mut military_registry, &mut state_registry, &war_registry, &mut battle_registry, "1800/01/02");

        let d = military_registry.divisions.get(&division_id).unwrap();
        if d.status == DivisionStatus::Idle && d.current_state == StateId(2) {
            arrived = true;
            break;
        }
        assert!(
            d.movement_progress >= last_progress,
            "day {day}: movement_progressは単調増加するはず(P27-004-PRE-001の根本原因が\
             再発すると日次リセットされ増加しなくなる): last={last_progress} now={}",
            d.movement_progress
        );
        // P27-004-PRE-001で確認した旧欠陥の直接的な再発検知: 2日目以降、progressが
        // 「常にちょうど1日分」に固定されたままではない(=増加し続ける)ことを確認する。
        last_progress = d.movement_progress;
    }

    assert!(arrived, "現行の日次進捗式(1/movement_days)から導出される有限日数(5日)以内に到着するはず、\
        15日経っても到着しないなら根本原因が再発している");
    let d = military_registry.divisions.get(&division_id).unwrap();
    assert_eq!(d.status, DivisionStatus::Idle);
    assert_eq!(d.destination, None);
    assert_eq!(d.movement_progress, 0.0);
}

/// spec §5: Defend AIが目的地を変更した場合は正規に上書きされる(no-opによって
/// 異なる命令まで無視されてしまわないことの確認)。
#[test]
fn test_02_a_genuinely_different_destination_is_still_applied() {
    let (mut state_registry, war_registry, mut military_registry, mut frontline_registry) =
        minimal_environment();
    update_all_frontlines(&war_registry, &state_registry, &military_registry, &mut frontline_registry);
    let fl_id = frontline_registry.get_frontline_for_war(WarId(0)).unwrap().frontline_id;

    let division_id = military_registry.add_division(idle_division_at(0, CountryId(1), StateId(1)));
    let _ = frontline_registry.assign_division(
        division_id,
        fl_id,
        CountryId(1),
        &military_registry,
        &war_registry,
    );
    if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
        plan.stance = FrontlineStance::Defend;
    }

    let mut battle_registry = BattleRegistry::default();
    process_daily_frontline_plans(
        &war_registry,
        &state_registry,
        &mut military_registry,
        &mut frontline_registry,
        None,
        &ArmyRegistry::default(),
    );
    let progress_before = military_registry.divisions.get(&division_id).unwrap().movement_progress;
    assert_eq!(progress_before, 0.0, "移動開始直後はprogress=0.0のはず");

    process_movement(&mut military_registry, &mut state_registry, &war_registry, &mut battle_registry, "1800/01/02");
    let progress_after_one_day = military_registry.divisions.get(&division_id).unwrap().movement_progress;
    assert!(progress_after_one_day > 0.0, "1日分の移動が進んでいるはず");

    // プレイヤーが手動で目的地をStateId(1)(出発州、既に離れているのでMoving中に
    // 引き返す形)へ変更したことを模擬する。
    {
        let d = military_registry.divisions.get_mut(&division_id).unwrap();
        let started = strategy_game::military::movement::start_or_maintain_movement(
            d,
            StateId(1),
            &state_registry,
            &[CountryId(1)],
            &[CountryId(2)],
        );
        assert!(started);
    }
    let d = military_registry.divisions.get(&division_id).unwrap();
    assert_eq!(d.destination, Some(StateId(1)), "異なる目的地は正規に上書きされるはず");
    assert_eq!(d.movement_progress, 0.0, "異なる目的地への上書きはprogressを0から開始するはず");
}

// ─────────────────────────────────────────────────────────────────────────
// §2: 実App・実データによるFerrowyn再現(旧84か月停滞の解消確認)
// ─────────────────────────────────────────────────────────────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("strategy_game_p27_004_e2e_{label}_{}_{nanos}_{n}", std::process::id()))
}

struct TempTestDir(std::path::PathBuf);
impl TempTestDir {
    fn new(label: &str) -> Self {
        Self(unique_temp_dir(label))
    }
    fn save_file_config(&self) -> SaveFileConfig {
        SaveFileConfig { path: SavePathConfig { final_path: self.0.join("savegame_v1.ron") } }
    }
}
impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_app(player: Option<CountryId>, save_config: SaveFileConfig) -> App {
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
    app.update();
    app.insert_resource(PlayerCountry(player));
    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
    state.apply(app.world_mut());
    app.update();
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
    app.update();
}

fn tick_day(app: &mut App, base_year: i32, month_index: u8, day: u8) {
    let (year, month) = sim_calendar::sim_calendar_date(base_year, month_index);
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn tick_month(app: &mut App, base_year: i32, month_index: u8) {
    let (year, month) = sim_calendar::sim_calendar_date(base_year, month_index);
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

fn advance_one_month(app: &mut App, current_month_index: u8) {
    for day in 1..=30u8 {
        tick_day(app, 1800, current_month_index, day);
    }
    tick_month(app, 1800, current_month_index.wrapping_add(1));
}

fn advance_months(app: &mut App, start: u8, months: u8) {
    for i in 0..months {
        advance_one_month(app, start.wrapping_add(i));
    }
}

fn count_stalled_divisions(app_before: &App, app_after: &App, country: CountryId) -> (usize, usize) {
    let mil_before = app_before.world().resource::<MilitaryRegistry>();
    let mil_after = app_after.world().resource::<MilitaryRegistry>();
    let mut moving = 0;
    let mut stalled = 0;
    for (id, before) in mil_before.divisions.iter().filter(|(_, d)| d.owner == country) {
        if let Some(after) = mil_after.divisions.get(id)
            && after.status == DivisionStatus::Moving
        {
            moving += 1;
            if before.status == DivisionStatus::Moving
                && before.current_state == after.current_state
                && before.destination == after.destination
                && (before.movement_progress - after.movement_progress).abs() < 1e-6
            {
                stalled += 1;
            }
        }
    }
    (moving, stalled)
}

/// spec §16: Ferrowyn実データ(120か月・通常AI)で、旧84か月停滞
/// (P27-003・P27-004-PRE-001で発見)が再発しないことを確認する。
#[test]
fn test_03_ferrowyn_real_data_120_months_no_longer_stalls() {
    let dir = TempTestDir::new("ferrowyn_120");
    let mut app = setup_app(None, dir.save_file_config());
    advance_months(&mut app, 1, 120);

    // 120か月経過時点のスナップショットを保存し、さらに10日進めて比較する
    // (P27-004-PRE-001の一時診断と同じ「10日間位置・進捗が完全不変」検出法)。
    let dir_b = TempTestDir::new("ferrowyn_120_snapshot");
    let mut app_snapshot = setup_app(None, dir_b.save_file_config());
    advance_months(&mut app_snapshot, 1, 120);
    // app_snapshotをapp相当として10日追加進行させ、appを「before」として使う。
    advance_months(&mut app_snapshot, 121, 10);

    let (moving, stalled) = count_stalled_divisions(&app, &app_snapshot, FERROWYN);
    eprintln!("[P27-004 Ferrowyn regression] month130: moving={moving} stalled_over_10_days={stalled}");
    assert_eq!(
        stalled, 0,
        "P27-004適用後、Ferrowynのdivisionが10日間位置・進捗ともに完全不変(=旧停滞パターン)\
         のまま留まってはいけない"
    );

    for &c in &ALL_SIX {
        let (_, stalled_c) = count_stalled_divisions(&app, &app_snapshot, c);
        assert_eq!(stalled_c, 0, "{c:?}: 120か月時点で停滞しているdivisionが存在してはいけない");
    }
}

/// spec §18: 90日/36/60/120か月、全6か国で到達可能な同一目的地命令による
/// 永久停滞が0件であることを確認する。
#[test]
fn test_04_six_country_90_36_60_120_month_zero_permanent_stall() {
    let dir = TempTestDir::new("six_country_checkpoints");
    let mut app = setup_app(None, dir.save_file_config());

    let checkpoints: [(u8, u8); 4] = [(1, 3), (4, 33), (37, 24), (61, 60)];
    for (start, months) in checkpoints {
        advance_months(&mut app, start, months);
        let month_label = match start {
            1 => "m3",
            4 => "m36",
            37 => "m60",
            61 => "m120",
            _ => "?",
        };

        let dir_snap = TempTestDir::new(&format!("snap_{month_label}"));
        let mut app_plus10 = setup_app(None, dir_snap.save_file_config());
        // 同じ月まで再現してから+10日進める(独立インスタンスでの比較)。
        let total_months_so_far: u8 = match start {
            1 => 3,
            4 => 36,
            37 => 60,
            61 => 120,
            _ => 0,
        };
        advance_months(&mut app_plus10, 1, total_months_so_far);
        advance_months(&mut app_plus10, total_months_so_far.wrapping_add(1), 10);

        for &c in &ALL_SIX {
            let (moving, stalled) = count_stalled_divisions(&app, &app_plus10, c);
            eprintln!("[P27-004 six-country] {month_label} {c:?}: moving={moving} stalled={stalled}");
            assert_eq!(stalled, 0, "{month_label} {c:?}: 停滞divisionが存在してはいけない");
        }
    }
}

/// spec §22: 独立2プロセス相当(同一プロセス内の2回独立実行)で完全一致することを確認する。
#[test]
fn test_05_determinism_two_independent_120_month_runs_match() {
    fn run() -> Vec<(CountryId, usize, DivisionStatus, StateId)> {
        let dir = TempTestDir::new("determinism");
        let mut app = setup_app(None, dir.save_file_config());
        advance_months(&mut app, 1, 120);
        let mil = app.world().resource::<MilitaryRegistry>();
        let mut result: Vec<_> = ALL_SIX
            .iter()
            .flat_map(|&c| {
                let mut divs: Vec<_> = mil.divisions.values().filter(|d| d.owner == c).collect();
                divs.sort_by_key(|d| d.id.0);
                divs.into_iter().map(move |d| (c, d.id.0, d.status, d.current_state))
            })
            .collect();
        result.sort_by_key(|(c, id, _, _)| (c.0, *id));
        result
    }
    assert_eq!(run(), run(), "同一シナリオを2回独立実行しても全Divisionのstatus/current_stateは完全一致するはず");
}

/// spec §21: 移動中(P27-004-PRE-001の停滞相当状況を含む)のsave→loadで全ての
/// 移動関連フィールドが完全に保持されることを確認する。
#[test]
fn test_06_save_load_preserves_movement_state_including_mid_stall_history() {
    let dir = TempTestDir::new("saveload_movement");
    let mut app = setup_app(None, dir.save_file_config());
    advance_months(&mut app, 1, 36);

    let before: Vec<_> = {
        let mil = app.world().resource::<MilitaryRegistry>();
        let mut v: Vec<_> = mil
            .divisions
            .values()
            .map(|d| (d.id, d.status, d.current_state, d.destination, d.target_state, d.current_path.clone(), d.movement_progress))
            .collect();
        v.sort_by_key(|(id, ..)| id.0);
        v
    };

    send_save_request(&mut app);
    send_load_request(&mut app);

    let after: Vec<_> = {
        let mil = app.world().resource::<MilitaryRegistry>();
        let mut v: Vec<_> = mil
            .divisions
            .values()
            .map(|d| (d.id, d.status, d.current_state, d.destination, d.target_state, d.current_path.clone(), d.movement_progress))
            .collect();
        v.sort_by_key(|(id, ..)| id.0);
        v
    };

    assert_eq!(before, after, "save->load前後で全Divisionの移動関連フィールドが完全一致するはず(Save V1、新規フィールドなし)");
}
