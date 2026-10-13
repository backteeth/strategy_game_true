//! P29-004: AI 募兵の生産能力ゲート(E3)の契約テスト。
//!
//! 前半は実際の AI 月次募兵判断(`evaluate_ai_monthly_recruitment`)を、合成した登録簿で
//! 呼ぶ。後半は実際の New Game・日次/月次システム・Save/Load を通して、「許可された募兵が
//! 持続的な生産能力を超えない」ことと、Load 後も同じ判断になることを確認する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, DivisionId, StateId};
use strategy_game::country::country_ai::CountryAiRegistry;
use strategy_game::country::{CountryData, CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::ai::evaluate_ai_monthly_recruitment;
use strategy_game::military::data::{
    Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
    STANDARD_INFANTRY_DEFINITION_ID,
};
use strategy_game::military::production_capacity_gate::{
    MIN_STANDING_DIVISIONS, monthly_units_from_daily, projected_daily_demand_units,
    sustained_monthly_supply,
};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::war::WarPlugin;
use strategy_game::war::data::WarRegistry;

// ─────────────────────────────────────────────────────────────────────────
// 合成登録簿での AI 募兵判断
// ─────────────────────────────────────────────────────────────────────────

const C: CountryId = CountryId(1);

struct Harness {
    countries: CountryRegistry,
    military: MilitaryRegistry,
    states: StateRegistry,
    war: WarRegistry,
    buildings: BuildingRegistry,
    ai: CountryAiRegistry,
    date: GameDate,
}

fn mf_def() -> BuildingDefinition {
    BuildingDefinition {
        building_type: BuildingType::MilitaryFactory,
        name: "Military Factory".to_string(),
        construction_cost: 1200.0,
        required_progress: 90.0,
        required_workforce: 25000.0,
        logistics_cost: 20.0,
        input_resources: HashMap::new(),
        output_resources: HashMap::from([
            (ResourceType::MilitaryEquipment, 20.0),
            (ResourceType::MilitarySupplies, 100.0),
        ]),
        maintenance_cost: 40.0,
        max_level: 10,
        science_output: 0.0,
        magic_output: 0.0,
        railway_capacity_bonus: 0.0,
    }
}

fn infantry_def() -> DivisionDefinition {
    DivisionDefinition {
        id: STANDARD_INFANTRY_DEFINITION_ID,
        name: "Standard Infantry".to_string(),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        required_manpower: 10_000,
        required_equipment: 1000.0,
        recruitment_days: 30,
        movement_speed: 1.0,
        attack: 10.0,
        defense: 10.0,
        breakthrough: 5.0,
        organization: 50.0,
        morale: 50.0,
        supply_usage: 1.0,
        maintenance_cost: 5.0,
    }
}

fn division(owner: CountryId) -> Division {
    Division {
        id: DivisionId(0),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(1),
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 10_000,
        max_manpower: 10_000,
        equipment: 1000.0,
        max_equipment: 1000.0,
        organization: 50.0,
        max_organization: 50.0,
        morale: 50.0,
        max_morale: 50.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: STANDARD_INFANTRY_DEFINITION_ID,
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

/// 人口 2,000,000(目標 Division 数 10)の自国所有州 1 つに、Level 1 の軍需工場(記録済み稼働率
/// `op`)を置く。既存 Division `divisions` 個、備蓄は十分(10,000)。
fn harness(divisions: usize, op: Option<f32>) -> Harness {
    let mut state = StateData {
        id: StateId(1),
        owner_country_id: C,
        population: 2_000_000,
        ..Default::default()
    };
    state.buildings.insert(BuildingType::MilitaryFactory, 1);
    if let Some(op) = op {
        state
            .building_operation_ratios
            .insert(BuildingType::MilitaryFactory, op);
    }
    let mut country = CountryData {
        id: C,
        capital_state_id: StateId(1),
        available_manpower: 1_000_000,
        mobilized_manpower: divisions as u64 * 10_000,
        treasury: 100_000.0,
        ..Default::default()
    };
    country.stockpile.set(ResourceType::MilitarySupplies, 10_000.0);
    let mut military = MilitaryRegistry::default();
    military
        .definitions
        .insert(STANDARD_INFANTRY_DEFINITION_ID, infantry_def());
    for _ in 0..divisions {
        military.add_division(division(C));
    }
    let mut buildings = BuildingRegistry::default();
    buildings
        .definitions
        .insert(BuildingType::MilitaryFactory, mf_def());
    Harness {
        countries: CountryRegistry {
            countries: vec![country],
        },
        military,
        states: StateRegistry::build(vec![state]),
        war: WarRegistry::default(),
        buildings,
        ai: CountryAiRegistry::default(),
        date: GameDate::new(1800, 6, 15),
    }
}

fn evaluate(h: &mut Harness, player: Option<CountryId>) {
    evaluate_ai_monthly_recruitment(
        &mut h.countries,
        &h.military,
        &h.states,
        &h.buildings,
        &h.war,
        &h.date,
        &mut h.ai,
        player,
    );
}

fn queue_len(h: &Harness) -> usize {
    h.countries.get(C).unwrap().recruitment_queue.len()
}

#[test]
fn a_large_stockpile_does_not_allow_recruitment_beyond_the_production_capacity() {
    // 既存 3 個(将来需要 3.0/日)+ 新規 1 = 4.0/日 → 121.67/月。工場は 90/月(Lv1 × 100 × 0.9)。
    let mut h = harness(3, Some(0.9));
    evaluate(&mut h, None);
    assert_eq!(queue_len(&h), 0, "備蓄が十分でも、生産能力を超える追加募兵はしない");
    // 状態(備蓄・人的資源・既存師団)は変更されない。
    assert_eq!(h.military.divisions.len(), 3);
    assert_eq!(h.countries.get(C).unwrap().available_manpower, 1_000_000);
}

#[test]
fn recruitment_proceeds_when_the_production_covers_the_new_demand() {
    // 4.0/日 → 121.67/月。Lv1 × 100 では足りないので、Level 2 × 0.7 = 140 で足りる。
    let mut h = harness(3, Some(0.7));
    h.states
        .get_mut(StateId(1))
        .unwrap()
        .buildings
        .insert(BuildingType::MilitaryFactory, 2);
    evaluate(&mut h, None);
    assert_eq!(queue_len(&h), 1);
}

#[test]
fn the_existing_stock_conditions_still_apply_when_the_production_is_sufficient() {
    // 生産は十分だが、備蓄ゲート(1 個の需要 1.0/日・平時の停止線 20 日分)を割る備蓄では募兵しない。
    let mut h = harness(3, Some(1.0));
    h.states
        .get_mut(StateId(1))
        .unwrap()
        .buildings
        .insert(BuildingType::MilitaryFactory, 3);
    h.countries
        .countries[0]
        .stockpile
        .set(ResourceType::MilitarySupplies, 5.0);
    evaluate(&mut h, None);
    assert_eq!(queue_len(&h), 0, "生産が足りても備蓄・bridge の既存条件を無視しない");
}

#[test]
fn unreported_production_and_recorded_zero_are_both_held_without_assuming_unlimited_capacity() {
    for op in [None, Some(0.0)] {
        let mut h = harness(2, op);
        evaluate(&mut h, None);
        assert_eq!(queue_len(&h), 0, "op={op:?}: 3 個目の募兵は保留される");
    }
}

#[test]
fn the_minimum_standing_force_is_not_blocked_by_missing_production() {
    assert_eq!(MIN_STANDING_DIVISIONS, 2);
    let mut h = harness(1, None);
    evaluate(&mut h, None);
    assert_eq!(queue_len(&h), 1, "生存 1 個 → 2 個目は免除");
}

#[test]
fn queued_orders_count_toward_the_limit_and_no_second_order_is_placed() {
    // 生存 2 + 訓練中 1(= 3)。新規 1 を加えた 4.0/日 = 121.67/月 を、工場 110/月 では賄えない。
    let mut h = harness(2, Some(1.0));
    h.states
        .get_mut(StateId(1))
        .unwrap()
        .building_operation_ratios
        .insert(BuildingType::MilitaryFactory, 1.1);
    h.countries.countries[0]
        .recruitment_queue
        .push(strategy_game::military::recruitment::RecruitmentQueueItem {
            division_id: STANDARD_INFANTRY_DEFINITION_ID,
            target_state: StateId(1),
            days_remaining: 10,
            total_days: 30,
        });
    h.countries.countries[0].mobilized_manpower += 10_000;
    evaluate(&mut h, None);
    assert_eq!(queue_len(&h), 1, "注文は増えない");
}

#[test]
fn a_drop_in_capacity_never_removes_existing_divisions_or_orders() {
    let mut h = harness(5, Some(0.0));
    h.countries.countries[0]
        .recruitment_queue
        .push(strategy_game::military::recruitment::RecruitmentQueueItem {
            division_id: STANDARD_INFANTRY_DEFINITION_ID,
            target_state: StateId(1),
            days_remaining: 10,
            total_days: 30,
        });
    h.countries.countries[0].mobilized_manpower += 10_000;
    for _ in 0..12 {
        evaluate(&mut h, None);
    }
    assert_eq!(h.military.divisions.len(), 5, "既存師団を解隊しない");
    assert_eq!(queue_len(&h), 1, "既存の注文を削除しない");
    assert_eq!(h.countries.get(C).unwrap().available_manpower, 1_000_000);
}

#[test]
fn the_player_country_is_not_evaluated_and_unaffected() {
    let mut h = harness(3, Some(0.9));
    evaluate(&mut h, Some(C));
    assert_eq!(queue_len(&h), 0);
    // プレイヤーの募兵 API(`request_recruitment`)は生産能力ゲートを通らない。
    let country = h.countries.countries.iter_mut().find(|c| c.id == C).unwrap();
    strategy_game::military::recruitment::request_recruitment(
        country,
        &h.military,
        STANDARD_INFANTRY_DEFINITION_ID,
        StateId(1),
    )
    .expect("player recruitment is not gated by the production capacity");
    assert_eq!(h.countries.get(C).unwrap().recruitment_queue.len(), 1);
}

#[test]
fn the_decision_is_deterministic_and_ordered_by_country_id() {
    let run = || {
        let mut h = harness(3, Some(0.9));
        evaluate(&mut h, None);
        (queue_len(&h), h.military.divisions.len())
    };
    assert_eq!(run(), run());
}

// ─────────────────────────────────────────────────────────────────────────
// 実 New Game・日次/月次システム・Save/Load
// ─────────────────────────────────────────────────────────────────────────

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_app(dir: &std::path::Path) -> App {
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
    app.insert_resource(SaveFileConfig {
        path: SavePathConfig {
            final_path: dir.join("savegame_v1.ron"),
        },
    });
    app.update();
    // Save は Playing 状態でプレイヤー国家を要求するため、Passguard をプレイヤーにする
    // (他の 5 か国が AI。プレイヤー国家は AI 募兵の対象外であることも同時に確認される)。
    app.insert_resource(PlayerCountry(Some(CountryId(4))));
    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn advance_one_day(app: &mut App) {
    let (old_month, next) = {
        let d = app.world().resource::<GameDate>();
        (d.month, d.add_days(1))
    };
    {
        let mut d = app.world_mut().resource_mut::<GameDate>();
        d.year = next.year;
        d.month = next.month;
        d.day = next.day;
    }
    let mut s: SystemState<(
        MessageWriter<DayChangedMessage>,
        MessageWriter<MonthChangedMessage>,
    )> = SystemState::new(app.world_mut());
    {
        let (mut dw, mut mw) = s.get_mut(app.world_mut()).unwrap();
        dw.write(DayChangedMessage {
            year: next.year,
            month: next.month,
            day: next.day,
        });
        if next.month != old_month {
            mw.write(MonthChangedMessage {
                year: next.year,
                month: next.month,
            });
        }
    }
    s.apply(app.world_mut());
    app.update();
}

/// 各 AI 国家について、(生存師団 + 訓練中)の数と、募兵キューの長さを返す。
fn queue_lengths(app: &App) -> HashMap<CountryId, usize> {
    app.world()
        .resource::<CountryRegistry>()
        .countries
        .iter()
        .map(|c| (c.id, c.recruitment_queue.len()))
        .collect()
}

#[test]
fn every_ai_order_in_a_real_run_stays_within_the_sustained_capacity_or_the_minimum_force() {
    let dir = TempDir(std::env::temp_dir().join(format!(
        "strategy_game_p29_004_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    let mut app = setup_app(&dir.0);
    let mut prev = queue_lengths(&app);
    let mut orders = 0u32;
    let mut held_above_minimum = 0u32;
    // 600 か月(1850 年まで)。
    while app.world().resource::<GameDate>().year < 1850 {
        advance_one_day(&mut app);
        let now = queue_lengths(&app);
        let ids: Vec<CountryId> = now.keys().copied().collect();
        for c in ids {
            if c == CountryId(4) {
                continue; // プレイヤー
            }
            if now[&c] > prev[&c] {
                // 新しい注文が入った: その注文を含む将来需要を、同じ月次処理で確定した
                // 稼働率から再構成した能力で賄えているか(または最小常備戦力以内か)。
                orders += 1;
                let cr = app.world().resource::<CountryRegistry>();
                let country = cr.get(c).unwrap();
                let mil = app.world().resource::<MilitaryRegistry>();
                let states = app.world().resource::<StateRegistry>();
                let buildings = app.world().resource::<BuildingRegistry>();
                let demand = projected_daily_demand_units(country, mil).unwrap();
                let required = monthly_units_from_daily(demand).unwrap();
                let capacity = sustained_monthly_supply(c, states, buildings).monthly_units;
                let standing = mil
                    .divisions
                    .values()
                    .filter(|d| d.owner == c && d.manpower > 0)
                    .count() as u128
                    + country.recruitment_queue.len() as u128;
                assert!(
                    standing <= MIN_STANDING_DIVISIONS || required <= capacity,
                    "{c:?}: order beyond capacity at {} (required {required} > capacity {capacity}, standing {standing})",
                    app.world().resource::<GameDate>().display()
                );
                if standing > MIN_STANDING_DIVISIONS {
                    held_above_minimum += 1;
                }
            }
        }
        prev = now;
    }
    eprintln!("orders={orders} above_minimum={held_above_minimum}");
}

fn send_save(app: &mut App) {
    let mut s: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut()).unwrap().write(SaveRequestMessage);
    s.apply(app.world_mut());
    app.update();
}

fn send_load(app: &mut App) {
    let mut s: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut()).unwrap().write(LoadRequestMessage);
    s.apply(app.world_mut());
    app.update();
    app.update();
}

fn capacities(app: &App) -> Vec<(usize, u128, u32)> {
    let states = app.world().resource::<StateRegistry>();
    let buildings = app.world().resource::<BuildingRegistry>();
    (0..6)
        .map(|c| {
            let s = sustained_monthly_supply(CountryId(c), states, buildings);
            (c, s.monthly_units, s.unreported_entries)
        })
        .collect()
}

#[test]
fn capacity_estimate_is_reconstructed_identically_after_save_and_load() {
    let dir = TempDir(std::env::temp_dir().join(format!(
        "strategy_game_p29_004_load_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    let mut app = setup_app(&dir.0);
    // 初回の月次生産の前(未集計)と、後(記録済み)。
    let before_first_tick = capacities(&app);
    assert!(
        before_first_tick.iter().all(|(_, units, _)| *units == 0),
        "New Game 直後は稼働率の記録がなく、能力は無限大ではなく 0 として扱う"
    );
    for _ in 0..(31 + 28 + 31 + 15) {
        advance_one_day(&mut app);
    }
    let saved = capacities(&app);
    assert!(saved.iter().any(|(_, u, _)| *u > 0), "月次生産の後は能力が再構成できる");
    send_save(&mut app);
    // Save 後に進めて値を変えてから Load する。
    for _ in 0..60 {
        advance_one_day(&mut app);
    }
    send_load(&mut app);
    assert_eq!(
        capacities(&app),
        saved,
        "Save に永続化された稼働率だけから、同じ能力が再構成される(追加の保存状態は不要)"
    );
}
