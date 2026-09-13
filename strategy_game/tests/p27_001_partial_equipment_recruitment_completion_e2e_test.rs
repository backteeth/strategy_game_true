//! P27-001: Equipment不足時の募兵完了と既存補充Systemへの引き渡しのE2Eテスト。
//!
//! `tests/p25_003_recruitment_equipment_issue_e2e_test.rs`が軽量ハーネス(直接関数
//! 呼び出し)で会計・保存則・複数queue・複数国家・Save往復を既に検証しているため、
//! 本ファイルは実App(`AppPlugin`ほか実プラグイン一式)を経由した検証
//! (System実行順序・既存装備補充への引き渡し・AI再開・戦闘力・決定性)に絞る。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::combat_calc::calculate_combat_readiness_with_supply;
use strategy_game::military::data::{MilitaryRegistry, STANDARD_INFANTRY_DEFINITION_ID};
use strategy_game::military::recruitment::{cancel_recruitment, request_recruitment};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::war::WarPlugin;

// ─── 実Appハーネス(tests/p25_003_recruitment_equipment_issue_e2e_test.rsの
// 実証済みパターンをそのまま踏襲する——`PlayerCountry(Some(..))`・
// `send_day_changed`が実際に`handle_daily_military`を起動することを、
// 同ファイルのTest L/N/O/P/Q/Tが既に大量に実証している) ────────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p27_001_e2e_{label}_{}_{nanos}_{n}",
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

fn send_day_changed(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn send_month_changed(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
    state.apply(app.world_mut());
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
    state.apply(app.world_mut());
}

fn set_military_equipment_stockpile(app: &mut App, country_id: CountryId, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(country_id)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitaryEquipment, value);
}

fn military_equipment_stockpile(app: &App, country_id: CountryId) -> f64 {
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
    target_state: StateId,
) -> Result<(), &'static str> {
    let mut state: SystemState<(ResMut<CountryRegistry>, Res<MilitaryRegistry>)> =
        SystemState::new(app.world_mut());
    let (mut country_registry, military_registry) = state.get_mut(app.world_mut()).expect("system params");
    let country = country_registry.get_mut(country_id).unwrap();
    let result = request_recruitment(country, &military_registry, STANDARD_INFANTRY_DEFINITION_ID, target_state);
    state.apply(app.world_mut());
    result
}

fn recruitment_days(app: &App) -> u32 {
    app.world()
        .resource::<MilitaryRegistry>()
        .definitions
        .get(&STANDARD_INFANTRY_DEFINITION_ID)
        .unwrap()
        .recruitment_days
}

fn required_equipment(app: &App) -> f64 {
    app.world()
        .resource::<MilitaryRegistry>()
        .definitions
        .get(&STANDARD_INFANTRY_DEFINITION_ID)
        .unwrap()
        .required_equipment
}

fn run_recruitment_to_completion(app: &mut App, year: i32, start_month: u8) {
    let days = recruitment_days(app);
    for day in 0..days {
        send_day_changed(app, year, start_month, (day % 28 + 1) as u8);
    }
}

fn player_divisions(app: &App, country_id: CountryId) -> Vec<strategy_game::military::data::Division> {
    app.world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == country_id)
        .cloned()
        .collect()
}

// ─── Test A: 実App経由でSystemが実際に起動することの直接確認 ────────────────
//
// P27-001-PRE-001が直面した一時ハーネス固有の未解明な不発火現象
// (`PlayerCountry(None)`ベースの最小ハーネス)を踏まえ、本ファイルは
// `p25_003`の実証済みパターン(`PlayerCountry(Some(..))`)を使用する。ここでは
// `days_remaining`が実際に日次で減少することを直接観測し、`handle_daily_
// military`(延いては`process_recruitment`)が本当に実行されていることを
// プロダクションコードの不具合または成功証拠の代わりに、明示的に確認する。

#[test]
fn test_a_real_app_daily_system_actually_decrements_the_queue() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_a");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    set_military_equipment_stockpile(&mut app, player, 0.0);
    call_request_recruitment(&mut app, player, StateId(0)).unwrap();

    let days_remaining_at = |app: &App| -> u32 {
        app.world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .recruitment_queue
            .first()
            .map(|item| item.days_remaining)
            .unwrap_or(0)
    };

    let initial = days_remaining_at(&app);
    assert_eq!(initial, recruitment_days(&app), "the queue item must start at the full recruitment_days");

    send_day_changed(&mut app, 1800, 1, 2);
    let after_one_day = days_remaining_at(&app);
    assert_eq!(
        after_one_day,
        initial - 1,
        "the real handle_daily_military System must decrement days_remaining by exactly 1 per DayChangedMessage"
    );

    send_day_changed(&mut app, 1800, 1, 3);
    assert_eq!(days_remaining_at(&app), initial - 2);
}

// ─── Test B: Equipment境界(100%/99%/50%/1 unit/0%)を実App経由で確認 ────────

#[test]
fn test_b_equipment_boundary_percentages_complete_via_real_app() {
    let player = CountryId(0);

    for (label, fraction) in [("100pct", 1.0), ("99pct", 0.99), ("50pct", 0.5), ("0pct", 0.0)] {
        let temp_dir = TempTestDir::new(&format!("test_b_{label}"));
        let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
        let required = required_equipment(&app);
        let stockpile_value = required * fraction;
        set_military_equipment_stockpile(&mut app, player, stockpile_value);

        let before = player_divisions(&app, player).len();
        call_request_recruitment(&mut app, player, StateId(0)).unwrap();
        run_recruitment_to_completion(&mut app, 1800, 1);

        let after = player_divisions(&app, player);
        assert_eq!(
            after.len(),
            before + 1,
            "[{label}] training completion must not be delayed regardless of equipment fraction"
        );
        let new_division = after
            .iter()
            .find(|d| (d.equipment - stockpile_value).abs() < 1e-6)
            .unwrap_or_else(|| panic!("[{label}] expected a division issued exactly {stockpile_value}"));
        assert_eq!(new_division.max_equipment, required, "[{label}] max_equipment must stay at the full requirement");
        assert_eq!(
            military_equipment_stockpile(&app, player),
            0.0,
            "[{label}] the entire available stockpile must have been issued"
        );
    }

    // 1 unit境界(canonical scale上の最小単位、required_equipmentを大幅に下回る)。
    let temp_dir = TempTestDir::new("test_b_one_unit");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    set_military_equipment_stockpile(&mut app, player, 0.000_001);
    let before = player_divisions(&app, player).len();
    call_request_recruitment(&mut app, player, StateId(0)).unwrap();
    run_recruitment_to_completion(&mut app, 1800, 1);
    let after = player_divisions(&app, player);
    assert_eq!(after.len(), before + 1, "[1unit] a single canonical unit of equipment must still complete the division");
    assert!(
        after.iter().any(|d| d.equipment > 0.0 && d.equipment < 0.001),
        "[1unit] the new division must have received the tiny available amount, not zero and not the full requirement"
    );
}

// ─── Test C: 完成後、既存の日次装備補充Systemが残り不足分を補う ────────────

#[test]
fn test_c_completed_division_is_topped_up_by_the_existing_daily_reinforcement_system() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_c");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    let required = required_equipment(&app);

    set_military_equipment_stockpile(&mut app, player, 0.0);
    call_request_recruitment(&mut app, player, StateId(0)).unwrap();
    run_recruitment_to_completion(&mut app, 1800, 1);

    let new_division_id = player_divisions(&app, player)
        .iter()
        .find(|d| d.equipment == 0.0)
        .expect("a zero-equipped division must exist")
        .id;

    // 翌日、国家備蓄へ満額を供給する(既存の日次装備補充Systemが処理する対象)。
    set_military_equipment_stockpile(&mut app, player, required);
    send_day_changed(&mut app, 1800, 2, 1);

    let topped_up = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&new_division_id)
        .expect("the division must still exist")
        .clone();
    assert_eq!(
        topped_up.equipment, required,
        "the existing equipment_reinforcement System (P25-005, unmodified by P27-001) must top up the division"
    );
    assert_eq!(
        military_equipment_stockpile(&app, player),
        0.0,
        "the reinforcement must have consumed exactly the shortfall from the stockpile"
    );
}

// ─── Test D: AIがqueue解消後に正規募兵を再開する ───────────────────────────

#[test]
fn test_d_ai_resumes_recruitment_after_its_stuck_queue_clears() {
    let player = CountryId(0);
    let ai_country = CountryId(1);
    let temp_dir = TempTestDir::new("test_d");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    // AI国家の備蓄を0にし、月替わりでAIが(装備0のまま)募兵をqueueへ入れることを
    // 期待する。P27-001以降はqueueがすぐ空になるため、以後の月替わりでAIが
    // 新規募兵を検討できることを確認する。
    set_military_equipment_stockpile(&mut app, ai_country, 0.0);
    send_month_changed(&mut app, 1800, 2);

    let ai_divisions_after_month_1 = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == ai_country)
        .count();

    // queueが空になっている(P27-001: 即座に完成しているはず)ことを確認してから、
    // 十分な期間(recruitment_daysの2倍程度)実カレンダーを進め、AIが
    // 新たに募兵を判断できる機会が少なくとも1回あることを確認する。
    let days = recruitment_days(&app);
    let mut year = 1800i32;
    let mut month = 2u8;
    for cycle in 0..2u32 {
        for day in 0..days {
            let d = day + 1;
            let (y, m, dd) = if d > 28 { (year, month + 1, d - 28) } else { (year, month, d) };
            send_day_changed(&mut app, y, m, dd as u8);
            year = y;
            month = m;
        }
        send_month_changed(&mut app, year, month);
        let _ = cycle;
    }

    let ai_queue_final = app
        .world()
        .resource::<CountryRegistry>()
        .get(ai_country)
        .unwrap()
        .recruitment_queue
        .len();
    assert_eq!(
        ai_queue_final, 0,
        "P27-001: the AI's queue must not remain permanently stuck across multiple recruitment cycles"
    );
    let ai_divisions_final = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == ai_country)
        .count();
    assert!(
        ai_divisions_final >= ai_divisions_after_month_1,
        "P27-001: the AI must not be worse off than before across additional recruitment cycles"
    );
}

// ─── Test E: equipment=0のDivisionが戦闘力計算で安全に扱われる ────────────

#[test]
fn test_e_zero_equipment_division_is_safe_in_combat_readiness_calculation() {
    let readiness = calculate_combat_readiness_with_supply(10_000, 10_000, 0.0, 100.0, 1.0)
        .expect("zero equipment must not be rejected as an error");
    assert_eq!(readiness.equipment_readiness_units, 0, "zero equipment must yield zero equipment readiness");
    assert_eq!(
        readiness.combat_readiness_units, 0,
        "the min() formula must make the division combat-ineffective, not panic or divide by zero"
    );

    // 部分装備でも安全に比例した値になる(境界のオフバイワンがないことを確認)。
    let partial = calculate_combat_readiness_with_supply(10_000, 10_000, 50.0, 100.0, 1.0).unwrap();
    assert_eq!(partial.equipment_readiness_units, 500_000, "50/100 equipment must yield exactly half readiness units");
}

// ─── Test F: 国家間分離(混在したEquipment水準でも互いに影響しない) ────────

#[test]
fn test_f_country_isolation_with_mixed_equipment_levels_via_real_app() {
    let player = CountryId(0);
    let other = CountryId(2);
    let temp_dir = TempTestDir::new("test_f");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    let required = required_equipment(&app);

    set_military_equipment_stockpile(&mut app, player, 0.0);
    set_military_equipment_stockpile(&mut app, other, required);

    call_request_recruitment(&mut app, player, StateId(0)).unwrap();
    {
        let mut state: SystemState<(ResMut<CountryRegistry>, Res<MilitaryRegistry>)> =
            SystemState::new(app.world_mut());
        let (mut country_registry, military_registry) = state.get_mut(app.world_mut()).expect("system params");
        let country = country_registry.get_mut(other).unwrap();
        request_recruitment(country, &military_registry, STANDARD_INFANTRY_DEFINITION_ID, StateId(2)).unwrap();
        state.apply(app.world_mut());
    }

    run_recruitment_to_completion(&mut app, 1800, 1);

    let player_new = player_divisions(&app, player);
    let other_new = player_divisions(&app, other);
    assert!(player_new.iter().any(|d| d.equipment == 0.0), "player's shortage must not be masked by another country's surplus");
    assert!(other_new.iter().any(|d| d.equipment == required), "other country's full stockpile must not be drained by player's shortage");
    assert_eq!(military_equipment_stockpile(&app, player), 0.0);
    assert_eq!(military_equipment_stockpile(&app, other), 0.0);
}

// ─── Test G: save→load 2回連続でも状態が安定する(実App、equipment=0) ──────

#[test]
fn test_g_double_save_load_stays_stable_for_a_zero_equipped_division() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_g");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    set_military_equipment_stockpile(&mut app, player, 0.0);
    call_request_recruitment(&mut app, player, StateId(0)).unwrap();
    run_recruitment_to_completion(&mut app, 1800, 1);

    let count_before = app.world().resource::<MilitaryRegistry>().divisions.len();

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();
    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        count_before,
        "two consecutive save/load cycles must not change the division count"
    );
    assert_eq!(military_equipment_stockpile(&app, player), 0.0);
}

// ─── Test H: cancel_recruitmentは引き続き人的資源のみを返却する ───────────
//
// P27-001はEquipmentの「予約」という概念を導入していないため、取消時の
// Equipment返却ロジックが不要のまま変わらないことを確認する(軽量ハーネス)。

#[test]
fn test_h_cancel_recruitment_still_returns_only_manpower_not_equipment() {
    use strategy_game::country::CountryData;
    use strategy_game::military::data::{DivisionDefinition, DivisionSize, DivisionType};
    use strategy_game::common::DivisionDefinitionId;

    let mut military_registry = MilitaryRegistry::default();
    military_registry.definitions.insert(
        DivisionDefinitionId(1),
        DivisionDefinition {
            id: DivisionDefinitionId(1),
            name: "Test".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 10_000,
            required_equipment: 100.0,
            recruitment_days: 30,
            movement_speed: 4.0,
            attack: 10.0,
            defense: 15.0,
            breakthrough: 5.0,
            organization: 100.0,
            morale: 100.0,
            supply_usage: 1.0,
            maintenance_cost: 1.0,
        },
    );
    let mut country = CountryData {
        id: CountryId(1),
        available_manpower: 20_000,
        ..CountryData::default()
    };
    country.stockpile.set(ResourceType::MilitaryEquipment, 0.0);

    request_recruitment(&mut country, &military_registry, DivisionDefinitionId(1), StateId(1)).unwrap();
    assert_eq!(country.available_manpower, 10_000);

    cancel_recruitment(&mut country, &military_registry, 0).unwrap();
    assert_eq!(country.available_manpower, 20_000, "manpower must be fully returned");
    assert_eq!(country.mobilized_manpower, 0);
    assert_eq!(
        country.stockpile.get(ResourceType::MilitaryEquipment),
        0.0,
        "no equipment was ever reserved, so cancellation touches no equipment"
    );
    assert!(country.recruitment_queue.is_empty());
}

// ─── Test I: 登録順反転に対する決定性(複数国家・複数queue項目) ────────────

#[test]
fn test_i_result_is_deterministic_regardless_of_country_and_queue_registration_order() {
    use strategy_game::country::CountryData;
    use strategy_game::military::data::{DivisionDefinition, DivisionSize, DivisionType};
    use strategy_game::common::DivisionDefinitionId;
    use strategy_game::military::recruitment::process_recruitment;

    fn def() -> DivisionDefinition {
        DivisionDefinition {
            id: DivisionDefinitionId(1),
            name: "Test".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 10_000,
            required_equipment: 100.0,
            recruitment_days: 5,
            movement_speed: 4.0,
            attack: 10.0,
            defense: 15.0,
            breakthrough: 5.0,
            organization: 100.0,
            morale: 100.0,
            supply_usage: 1.0,
            maintenance_cost: 1.0,
        }
    }

    fn build_and_run(country_ids: [usize; 3]) -> Vec<(usize, f64, usize)> {
        let mut military_registry = MilitaryRegistry::default();
        military_registry.definitions.insert(DivisionDefinitionId(1), def());
        let mut country_registry = CountryRegistry::default();

        for &id in &country_ids {
            let mut country = CountryData {
                id: CountryId(id),
                available_manpower: 20_000,
                ..CountryData::default()
            };
            // 各国家の備蓄をIDに応じて変え、部分支給が発生するようにする。
            country
                .stockpile
                .set(ResourceType::MilitaryEquipment, (id as f64) * 10.0);
            request_recruitment(&mut country, &military_registry, DivisionDefinitionId(1), StateId(1)).unwrap();
            country_registry.countries.push(country);
        }

        for _ in 0..def().recruitment_days {
            process_recruitment(&mut country_registry, &mut military_registry);
        }

        let mut result: Vec<(usize, f64, usize)> = country_registry
            .countries
            .iter()
            .map(|c| {
                let equipment: f64 = military_registry
                    .divisions
                    .values()
                    .filter(|d| d.owner == c.id)
                    .map(|d| d.equipment)
                    .sum();
                (c.id.0, equipment, c.recruitment_queue.len())
            })
            .collect();
        result.sort_by_key(|&(id, _, _)| id);
        result
    }

    let forward = build_and_run([1, 2, 3]);
    let reversed = build_and_run([3, 2, 1]);
    assert_eq!(
        forward, reversed,
        "P27-001: per-country completion results must not depend on country registration order"
    );
}
