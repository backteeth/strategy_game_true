//! P27-002-A: AI平時建設(優先度3)への3か月経常支出予備判定の実App統合テスト。
//!
//! 純粋な判定ロジック(境界値・優先度1/2バイパス・数値安全性・国家間分離)の
//! 単体テストは`src/country/country_ai.rs`(9件)に既にあるため、本ファイルは
//! **実App・実データ経由の統合**、Bevy state遷移harnessの健全性確認、
//! Save V1継続性、AI評価cadence保護、P26-008-Cの優先度1/2保護、6か国の
//! 90日/36か月実測、および国家総合力/Tierへの影響測定に集中する。
//!
//! `feedback_bevy_state_transition_harness.md`の教訓により、`setup_app`は
//! `tests/p26_008_c_supply_chain_construction_priority_e2e_test.rs`と同じ
//! 実証済みパターン(デフォルトstateのまま1回`app.update()`してから
//! `PlayerCountry`/`GameState::Playing`を挿入)を使う。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::BuildingType;
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::country_ai::{
    CountryAiDecisionReason, CountryAiRegistry, PEACETIME_CONSTRUCTION_RESERVE_MONTHS,
};
use strategy_game::country::power::CountryPowerRegistry;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
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

// 実データ上のCountryId(`assets/data/countries.ron`)。
const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const DWARF: CountryId = CountryId(2);
const OCEANIC: CountryId = CountryId(3);
const PASSGUARD: CountryId = CountryId(4);
const FERROWYN: CountryId = CountryId(5);
const ALL_SIX: [CountryId; 6] = [ARCADIA, ELFIN, DWARF, OCEANIC, PASSGUARD, FERROWYN];

// ─────────────────────────────────────────────────────────────────────────
// App-level fixtures(p26_008_c_supply_chain_construction_priority_e2e_testと
// 同じ構成方針・同じ実証済みharnessパターン)
// ─────────────────────────────────────────────────────────────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p27_002_a_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
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
    // feedback_bevy_state_transition_harness.md: デフォルトstateのままこの最初の
    // updateを呼ばないと、以降のPlaying挿入がstate「遷移」として認識されず
    // OnEnter(Playing)systemが一切発火しない(=シミュレーション全体が死んで見える)。
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

fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn tick_month(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

/// P26-004-PRE-002/P26-008-PRE-002が確立した、day1〜30を含む実カレンダー相当の簡略暦。
/// 月末に実際の`MonthChangedMessage`(=`handle_monthly_economy`実行=`monthly_expenses`
/// 更新)を発火させる、本テストの根幹となるヘルパー。
fn advance_one_month(app: &mut App, current_month_index: u8) {
    for day in 1..=30u8 {
        tick_day(app, 1800, current_month_index, day);
    }
    tick_month(app, 1800, current_month_index.wrapping_add(1));
}

fn advance_months(app: &mut App, months: u8) {
    for m in 1..=months {
        advance_one_month(app, m);
    }
}

/// Dwarf(CountryId(2))の建設AI月次評価日(`current_day % 30 == country_id % 30`)。
const DWARF_MONTHLY_EVAL_DAY: u8 = 2;

/// P26-008-Cのモジュールdocと同じ理由(decision_reasonは月次/週次AIが共有フィールド)
/// により、その月の最初の建設AI評価直後に読み取る必要がある。1か月目はNew Game
/// 直後の`dirty=true`によりday 1で既に評価済みなためday 1、2か月目以降は
/// Dwarfの正規月次評価日(day 2)で読み取る。
fn advance_one_month_and_read_dwarf_construction_reason(
    app: &mut App,
    current_month_index: u8,
) -> Option<CountryAiDecisionReason> {
    let check_day: u8 = if current_month_index == 1 { 1 } else { DWARF_MONTHLY_EVAL_DAY };
    for day in 1..=check_day {
        tick_day(app, 1800, current_month_index, day);
    }
    let reason = decision_reason_of(app, DWARF);
    for day in (check_day + 1)..=30u8 {
        tick_day(app, 1800, current_month_index, day);
    }
    tick_month(app, 1800, current_month_index.wrapping_add(1));
    reason
}

/// `advance_one_month_and_read_dwarf_construction_reason`と異なり、その月の
/// 建設AI評価日までだけ日次tickし、月の残り(月末の`handle_monthly_economy`を
/// 含む)へは進めない。国庫・建設queueへの副作用を、建設AI**そのものだけ**の
/// 結果として厳密に検証したい(月次経済処理由来の税収/経費変動を混入させない)
/// テスト専用のヘルパー。
fn tick_dwarf_up_to_construction_evaluation(app: &mut App, current_month_index: u8) -> Option<CountryAiDecisionReason> {
    let check_day: u8 = if current_month_index == 1 { 1 } else { DWARF_MONTHLY_EVAL_DAY };
    for day in 1..=check_day {
        tick_day(app, 1800, current_month_index, day);
    }
    decision_reason_of(app, DWARF)
}

fn building_level(app: &App, state_id: StateId, b_type: BuildingType) -> u32 {
    app.world().resource::<StateRegistry>().get(state_id).map(|s| s.building_level(b_type)).unwrap_or(0)
}

fn set_building_level(app: &mut App, state_id: StateId, b_type: BuildingType, level: u32) {
    app.world_mut().resource_mut::<StateRegistry>().get_mut(state_id).unwrap().buildings.insert(b_type, level);
}

fn set_deposit(app: &mut App, state_id: StateId, resource: ResourceType, discovered: bool) {
    let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
    let state = state_registry.get_mut(state_id).unwrap();
    state.resource_deposits.push(strategy_game::economy::resources::StateResourceDeposit {
        resource_type: resource,
        base_output: 10.0,
        discovered,
        development_level: 0,
    });
}

fn decision_reason_of(app: &App, country_id: CountryId) -> Option<CountryAiDecisionReason> {
    app.world().resource::<CountryAiRegistry>().ai_states.get(&country_id).map(|s| s.decision_reason)
}

fn construction_queue_of(app: &App, country_id: CountryId) -> Vec<(StateId, BuildingType)> {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .map(|c| c.construction_queue.iter().map(|i| (i.state_id, i.building_type)).collect())
        .unwrap_or_default()
}

fn set_treasury(app: &mut App, country_id: CountryId, value: f64) {
    app.world_mut().resource_mut::<CountryRegistry>().get_mut(country_id).unwrap().treasury = value;
}

fn treasury_of(app: &App, country_id: CountryId) -> f64 {
    app.world().resource::<CountryRegistry>().get(country_id).unwrap().treasury
}

fn set_monthly_expenses(app: &mut App, country_id: CountryId, value: f64) {
    app.world_mut().resource_mut::<CountryRegistry>().get_mut(country_id).unwrap().monthly_expenses = value;
}

fn monthly_expenses_of(app: &App, country_id: CountryId) -> f64 {
    app.world().resource::<CountryRegistry>().get(country_id).unwrap().monthly_expenses
}

fn set_stockpile(app: &mut App, country_id: CountryId, resource: ResourceType, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(country_id)
        .unwrap()
        .stockpile
        .set(resource, value);
}

/// Dwarfを、優先度1(Food十分)・優先度2(MilitaryFactory自体を持たないので
/// IndustrialGoods供給網修復規則は対象外)のいずれも発火せず、優先度3の
/// Mine分岐(Wood/Iron/Coal<30)が発火する構成に整える。既存のFactory/Mineと
/// 鉱床を全て取り除き、このテスト専用のIron+Coal鉱床だけを与える。
fn make_dwarf_priority_3_mine_country(app: &mut App) -> StateId {
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
            state.buildings.remove(&BuildingType::Mine);
            state.buildings.remove(&BuildingType::MilitaryFactory);
            state.resource_deposits.clear();
        }
    }
    set_deposit(app, capital, ResourceType::Iron, true);
    set_deposit(app, capital, ResourceType::Coal, true);
    set_stockpile(app, DWARF, ResourceType::Food, 1000.0);
    set_stockpile(app, DWARF, ResourceType::Wood, 1000.0);
    set_stockpile(app, DWARF, ResourceType::Iron, 0.0);
    set_stockpile(app, DWARF, ResourceType::Coal, 0.0);
    capital
}

const MINE_COST: f64 = 500.0;
const FACTORY_COST: f64 = 1000.0;
const FARM_COST: f64 = 200.0;

// ─────────────────────────────────────────────────────────────────────────
// §1: Bevy state遷移harnessの健全性確認(feedback_bevy_state_transition_harness.md)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_01_harness_sanity_playing_state_actually_entered_and_economy_runs() {
    let dir = TempTestDir::new("t01_harness_sanity");
    let mut app = setup_app(None, dir.save_file_config());

    assert_eq!(*app.world().resource::<State<GameState>>().get(), GameState::Playing);

    // 経常支出が初期値0のまま凍結していないこと(=月次Economy Systemが実際に
    // 発火したことの直接証拠)を、1か月進める前後で確認する。
    for &c in &ALL_SIX {
        assert_eq!(
            monthly_expenses_of(&app, c),
            0.0,
            "{c:?}: New Game直後はまだ月次Economyが1度も走っていないため0のはず"
        );
    }

    advance_one_month(&mut app, 1);

    // 注意: このharnessは`DayChangedMessage`/`MonthChangedMessage`を直接注入して
    // 各Systemを駆動しており、実際に`GameDate`resourceを更新する
    // `app::time::advance_game_date`(メッセージの発行元であって消費先ではない)を
    // 経由しない。したがって`GameDate`表示文字列そのものは進まない
    // (`tests/p26_008_c_supply_chain_construction_priority_e2e_test.rs`を含む
    // 既存の全E2Eテストも同様にGameDateの進行を検証対象にしていない)。
    // ここでの正しい「発火した証拠」はGameDateではなく、以下のmonthly_expenses/
    // decision_reasonの実際の変化である。
    let mut any_nonzero_expenses = false;
    for &c in &ALL_SIX {
        let expenses = monthly_expenses_of(&app, c);
        assert!(expenses.is_finite(), "{c:?}: monthly_expensesがNaN/Infではいけない");
        if expenses > 0.0 {
            any_nonzero_expenses = true;
        }
    }
    assert!(
        any_nonzero_expenses,
        "1か月経過後も全国家のmonthly_expensesが0のままなら、Economy Systemが\
         発火していないharness不具合を疑う(feedback_bevy_state_transition_harness.md)"
    );

    // 月次CountryAi Systemも実際に発火していること(いずれかの国が既定のdecision_reasonを持つ)。
    assert!(decision_reason_of(&app, ARCADIA).is_some());
}

// ─────────────────────────────────────────────────────────────────────────
// §2: 境界値(実App経由)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_02_boundary_exact_allows_priority_3_construction_in_real_app() {
    let dir = TempTestDir::new("t02_boundary_exact");
    let mut app = setup_app(None, dir.save_file_config());
    make_dwarf_priority_3_mine_country(&mut app);

    let monthly_expenses = 100.0;
    let required_treasury = MINE_COST + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
    set_monthly_expenses(&mut app, DWARF, monthly_expenses);
    set_treasury(&mut app, DWARF, required_treasury);

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    // 優先度3のMine分岐は、優先度2のMine分岐と同じ`RawMaterialMinePriority`理由を
    // 共有する(country_ai.rs該当コメント参照: 既存reasonの再利用、spec §13)。
    // このfixtureはDwarfの全建物を除去済みのため`find_proactive_development_target`
    // が何も要求しない(優先度2は不発火が保証されている)。
    assert_eq!(reason, Some(CountryAiDecisionReason::RawMaterialMinePriority));
    let queue = construction_queue_of(&app, DWARF);
    assert!(queue.iter().any(|(_, b)| *b == BuildingType::Mine), "境界値ちょうどでは建設が許可されるはず");
}

#[test]
fn test_03_boundary_under_defers_priority_3_construction_without_mutation() {
    let dir = TempTestDir::new("t03_boundary_under");
    let mut app = setup_app(None, dir.save_file_config());
    make_dwarf_priority_3_mine_country(&mut app);

    let monthly_expenses = 100.0;
    let required_treasury = MINE_COST + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
    set_monthly_expenses(&mut app, DWARF, monthly_expenses);
    set_treasury(&mut app, DWARF, required_treasury - 0.01);

    // 月末の`handle_monthly_economy`(税収/経費)を経由させず、建設AI評価日
    // までだけ進めることで、国庫への副作用を建設AIそのものだけに限定して検証する。
    let reason = tick_dwarf_up_to_construction_evaluation(&mut app, 1);

    assert_eq!(reason, Some(CountryAiDecisionReason::FundsShortage));
    assert!(construction_queue_of(&app, DWARF).is_empty(), "境界未満では優先度3建設を見送るはず");
    assert_eq!(
        treasury_of(&app, DWARF),
        required_treasury - 0.01,
        "見送り時は国庫が一切変化してはいけない"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §3: 建設の原子性(拒否時に一切の副作用がない)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_04_construction_atomicity_preserved_on_reserve_rejection() {
    let dir = TempTestDir::new("t04_atomicity");
    let mut app = setup_app(None, dir.save_file_config());
    make_dwarf_priority_3_mine_country(&mut app);

    let monthly_expenses = 1000.0;
    set_monthly_expenses(&mut app, DWARF, monthly_expenses);
    // 建設費自体は賄えるが、3か月分の予備には遠く及ばない。
    set_treasury(&mut app, DWARF, MINE_COST + 1.0);
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    let mine_level_before = building_level(&app, capital, BuildingType::Mine);

    tick_dwarf_up_to_construction_evaluation(&mut app, 1);

    assert!(construction_queue_of(&app, DWARF).is_empty());
    assert_eq!(treasury_of(&app, DWARF), MINE_COST + 1.0, "拒否時は国庫が減らないはず(原子性)");
    assert_eq!(building_level(&app, capital, BuildingType::Mine), mine_level_before);
}

#[test]
fn test_05_cost_itself_unaffordable_is_unchanged_from_existing_behavior_in_real_app() {
    let dir = TempTestDir::new("t05_cost_unaffordable");
    let mut app = setup_app(None, dir.save_file_config());
    make_dwarf_priority_3_mine_country(&mut app);
    set_monthly_expenses(&mut app, DWARF, 0.0);
    set_treasury(&mut app, DWARF, MINE_COST - 1.0);

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(reason, Some(CountryAiDecisionReason::FundsShortage));
    assert!(construction_queue_of(&app, DWARF).is_empty());
}

// ─────────────────────────────────────────────────────────────────────────
// §4: 優先度1(Food緊急)・優先度2(供給網修復)の保護(実App経由)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_06_food_emergency_bypasses_the_reserve_gate_in_real_app() {
    let dir = TempTestDir::new("t06_food_emergency");
    let mut app = setup_app(None, dir.save_file_config());
    make_dwarf_priority_3_mine_country(&mut app);
    set_stockpile(&mut app, DWARF, ResourceType::Food, 5.0); // 緊急しきい値未満
    set_monthly_expenses(&mut app, DWARF, 10_000.0); // 3か月分の予備には到底足りない額
    set_treasury(&mut app, DWARF, FARM_COST); // Farm建設費ちょうどのみ

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(
        reason,
        Some(CountryAiDecisionReason::FoodShortageFarmPriority),
        "Food緊急は予備資金判定に関わらず発火するはず"
    );
    assert!(construction_queue_of(&app, DWARF).iter().any(|(_, b)| *b == BuildingType::Farm));
}

#[test]
fn test_07_proactive_supply_chain_repair_bypasses_the_reserve_gate_in_real_app() {
    let dir = TempTestDir::new("t07_proactive_repair");
    let mut app = setup_app(None, dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
            state.resource_deposits.clear();
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_stockpile(&mut app, DWARF, ResourceType::Food, 1000.0);
    set_monthly_expenses(&mut app, DWARF, 10_000.0); // 3か月分の予備には到底足りない額
    set_treasury(&mut app, DWARF, FACTORY_COST); // Factory建設費ちょうどのみ

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(
        reason,
        Some(CountryAiDecisionReason::SteelShortageSteelMillPriority),
        "P26-008-Cの供給網修復(優先度2)は予備資金判定に関わらず発火するはず"
    );
    assert!(construction_queue_of(&app, DWARF).iter().any(|(_, b)| *b == BuildingType::Factory));
}

// ─────────────────────────────────────────────────────────────────────────
// §5: 決定性・国家間分離
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_08_determinism_two_identical_runs_produce_identical_results() {
    fn run() -> (Option<CountryAiDecisionReason>, Vec<(StateId, BuildingType)>, f64) {
        let dir = TempTestDir::new("t08_determinism");
        let mut app = setup_app(None, dir.save_file_config());
        make_dwarf_priority_3_mine_country(&mut app);
        set_monthly_expenses(&mut app, DWARF, 100.0);
        set_treasury(&mut app, DWARF, MINE_COST + 100.0 * PEACETIME_CONSTRUCTION_RESERVE_MONTHS);
        let reason = tick_dwarf_up_to_construction_evaluation(&mut app, 1);
        (reason, construction_queue_of(&app, DWARF), treasury_of(&app, DWARF))
    }

    let a = run();
    let b = run();
    assert_eq!(a, b);
}

#[test]
fn test_09_country_isolation_reserve_gate_does_not_cross_contaminate_in_real_app() {
    let dir = TempTestDir::new("t09_isolation");
    let mut app = setup_app(None, dir.save_file_config());
    make_dwarf_priority_3_mine_country(&mut app);

    // Dwarf: 境界ちょうど(建設される)。
    let monthly_expenses = 100.0;
    let required_treasury = MINE_COST + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
    set_monthly_expenses(&mut app, DWARF, monthly_expenses);
    set_treasury(&mut app, DWARF, required_treasury);

    // Ferrowyn: 極端な高額経常支出を設定し、同じ月に予備判定で見送られる状況を作る
    // (Ferrowynが優先度3のMine/Factory分岐に入るかは実データ依存のため、ここでは
    // Dwarfの結果に影響しないことのみを検証する: 治安上重要なのは相互不干渉)。
    set_monthly_expenses(&mut app, FERROWYN, 1_000_000.0);

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(
        reason,
        Some(CountryAiDecisionReason::RawMaterialMinePriority),
        "他国(Ferrowyn)の極端な経常支出設定がDwarfの判定に影響してはいけない"
    );
    assert!(construction_queue_of(&app, DWARF).iter().any(|(_, b)| *b == BuildingType::Mine));
}

// ─────────────────────────────────────────────────────────────────────────
// §6: 数値安全性(実App経由)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_10_rejects_non_finite_monthly_expenses_without_mutation_in_real_app() {
    let dir = TempTestDir::new("t10_nonfinite");
    let mut app = setup_app(None, dir.save_file_config());
    make_dwarf_priority_3_mine_country(&mut app);
    set_monthly_expenses(&mut app, DWARF, f64::NAN);
    set_treasury(&mut app, DWARF, 1_000_000.0);

    tick_dwarf_up_to_construction_evaluation(&mut app, 1);

    assert!(
        construction_queue_of(&app, DWARF).is_empty(),
        "monthly_expensesがNaNの場合は安全に建設を見送るはず(0コアーションではなく拒否)"
    );
    assert_eq!(treasury_of(&app, DWARF), 1_000_000.0, "拒否時は国庫を一切変更してはいけない");
}

// ─────────────────────────────────────────────────────────────────────────
// §7: Save V1継続性
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_11_decision_is_reproduced_identically_after_save_and_load() {
    let dir = TempTestDir::new("t11_save_load");
    let mut app = setup_app(None, dir.save_file_config());
    make_dwarf_priority_3_mine_country(&mut app);
    let monthly_expenses = 100.0;
    let required_treasury = MINE_COST + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
    set_monthly_expenses(&mut app, DWARF, monthly_expenses);
    set_treasury(&mut app, DWARF, required_treasury);

    send_save_request(&mut app);
    send_load_request(&mut app);

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(reason, Some(CountryAiDecisionReason::RawMaterialMinePriority));
    assert!(construction_queue_of(&app, DWARF).iter().any(|(_, b)| *b == BuildingType::Mine));
}

#[test]
fn test_12_double_save_load_does_not_change_the_outcome() {
    let dir = TempTestDir::new("t12_double_save_load");
    let mut app = setup_app(None, dir.save_file_config());
    make_dwarf_priority_3_mine_country(&mut app);
    let monthly_expenses = 100.0;
    let required_treasury = MINE_COST + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
    set_monthly_expenses(&mut app, DWARF, monthly_expenses);
    set_treasury(&mut app, DWARF, required_treasury - 0.01); // 見送りケース

    send_save_request(&mut app);
    send_load_request(&mut app);
    send_save_request(&mut app);
    send_load_request(&mut app);

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(reason, Some(CountryAiDecisionReason::FundsShortage));
    assert!(construction_queue_of(&app, DWARF).is_empty());
}

#[test]
fn test_13_old_save_without_any_new_field_loads_and_evaluates_normally() {
    // P27-002-Aは新規Saveフィールドを一切追加しないため、既存のSaveがそのまま
    // 正常にロードできることを実際にsave→loadして確認する(特別なmigrationは不要)。
    let dir = TempTestDir::new("t13_old_save");
    let mut app = setup_app(None, dir.save_file_config());
    send_save_request(&mut app);
    send_load_request(&mut app);
    advance_one_month(&mut app, 1);
    assert!(decision_reason_of(&app, ARCADIA).is_some());
}

// ─────────────────────────────────────────────────────────────────────────
// §8: AI評価cadenceが変わっていないこと
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_14_construction_ai_still_follows_the_existing_monthly_cadence() {
    let dir = TempTestDir::new("t14_cadence");
    let mut app = setup_app(None, dir.save_file_config());
    make_dwarf_priority_3_mine_country(&mut app);
    let monthly_expenses = 100.0;
    let required_treasury = MINE_COST + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
    set_monthly_expenses(&mut app, DWARF, monthly_expenses);
    set_treasury(&mut app, DWARF, required_treasury);

    // このテストは既存cadence実装(`day % 30 == country_id % 30 || dirty`、
    // P27-002-Aは無変更)そのものを変えていないことの間接的な確認として、
    // Dwarfの正規月次評価日(day 2、New Game直後の1か月目はday 1)で意図した
    // 結果が出ることを見る。
    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);
    assert_eq!(reason, Some(CountryAiDecisionReason::RawMaterialMinePriority));
}

// ─────────────────────────────────────────────────────────────────────────
// §9: New Game初月の未初期化monthly_expenses(0.0既定値)の安全な扱い
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_15_new_game_first_month_does_not_misuse_uninitialized_monthly_expenses() {
    // 建設AIの評価cadence(`current_day % 30 == country_id % 30`)は、月次Economyの
    // 実際の月末更新タイミングとは独立している。そのため、New Game直後の初回
    // 評価時点では対象国家のmonthly_expensesがまだ一度も更新されておらず既定値
    // 0.0であり得る(§4想定シナリオ)。この場合、予備判定は
    // `required_treasury = cost + 0.0 * 3 = cost`に自然に縮退し、旧来の
    // 「建設費のみ判定」と完全に一致する安全側の挙動になるはずで、パニックや
    // 誤った拒否をしてはいけない。
    let dir = TempTestDir::new("t15_new_game_first_month");
    let mut app = setup_app(None, dir.save_file_config());
    make_dwarf_priority_3_mine_country(&mut app);
    assert_eq!(monthly_expenses_of(&app, DWARF), 0.0, "New Game直後はまだ月次Economyが走っていないはず");
    set_treasury(&mut app, DWARF, MINE_COST); // 建設費ちょうどのみ、予備の上乗せなし

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(
        reason,
        Some(CountryAiDecisionReason::RawMaterialMinePriority),
        "monthly_expenses未初期化(0.0)時は旧来の建設費のみ判定に縮退し、建設が許可されるはず"
    );
    assert!(construction_queue_of(&app, DWARF).iter().any(|(_, b)| *b == BuildingType::Mine));
}

// ─────────────────────────────────────────────────────────────────────────
// §10: P26-008-C保護 — 実データでElfin/Oceanic/Passguardが初月Factoryを得る
// ─────────────────────────────────────────────────────────────────────────

fn factory_level_sum(app: &App, country_id: CountryId) -> u32 {
    app.world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .filter(|s| s.owner_country_id == country_id)
        .map(|s| s.building_level(BuildingType::Factory))
        .sum()
}

fn mine_level_sum(app: &App, country_id: CountryId) -> u32 {
    app.world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .filter(|s| s.owner_country_id == country_id)
        .map(|s| s.building_level(BuildingType::Mine))
        .sum()
}

fn militaryfactory_needs_industrial_goods_but_lacks_factory(app: &App, country_id: CountryId) -> bool {
    let state_registry = app.world().resource::<StateRegistry>();
    let has_mf = state_registry
        .states
        .iter()
        .any(|s| s.owner_country_id == country_id && s.building_level(BuildingType::MilitaryFactory) > 0);
    let has_factory = state_registry
        .states
        .iter()
        .any(|s| s.owner_country_id == country_id && s.building_level(BuildingType::Factory) > 0);
    has_mf && !has_factory
}

#[test]
fn test_16_real_data_countries_with_structural_gap_still_get_first_month_factory() {
    let dir = TempTestDir::new("t16_p26_008_c_protection");
    let mut app = setup_app(None, dir.save_file_config());

    let mut initial_gap: HashMap<CountryId, bool> = HashMap::new();
    for &c in &ALL_SIX {
        initial_gap.insert(c, militaryfactory_needs_industrial_goods_but_lacks_factory(&app, c));
    }
    eprintln!("[P27-002-A P26-008-C protection] initial structural gap: {initial_gap:?}");

    let mut queued_factory_first_month: HashMap<CountryId, bool> =
        ALL_SIX.iter().map(|&c| (c, false)).collect();
    for day in 1..=30u8 {
        tick_day(&mut app, 1800, 1, day);
        for &c in &ALL_SIX {
            if construction_queue_of(&app, c).iter().any(|(_, b)| *b == BuildingType::Factory) {
                queued_factory_first_month.insert(c, true);
            }
        }
    }
    tick_month(&mut app, 1800, 2);

    eprintln!("[P27-002-A P26-008-C protection] first-month factory queued: {queued_factory_first_month:?}");

    for &c in &ALL_SIX {
        if initial_gap[&c] {
            assert!(
                queued_factory_first_month[&c],
                "{c:?}: 初期状態でIndustrialGoods構造的欠損を持つ国は、P27-002-A後も\
                 P26-008-Cの供給網修復(優先度2、予備資金判定の対象外)によって\
                 初月にFactoryを得るはず(回帰していないこと)"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// §11: 6か国90日/36か月実測 + 国家総合力・Tierへの影響測定
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_17_real_six_country_90_day_and_36_month_fiscal_reserve_verification() {
    let dir = TempTestDir::new("t17_six_country");
    let mut app = setup_app(None, dir.save_file_config());

    eprintln!("[P27-002-A six-country report] initial state:");
    for &c in &ALL_SIX {
        let reg = app.world().resource::<CountryRegistry>();
        let country = reg.get(c).unwrap();
        eprintln!(
            "  {c:?}: initial_treasury={:.2} initial_monthly_expenses={:.2}",
            country.treasury, country.monthly_expenses
        );
    }

    let mut fundsshortage_count_90d: HashMap<CountryId, u32> = ALL_SIX.iter().map(|&c| (c, 0)).collect();

    for m in 1..=3u8 {
        for day in 1..=30u8 {
            tick_day(&mut app, 1800, m, day);
            for &c in &ALL_SIX {
                if decision_reason_of(&app, c) == Some(CountryAiDecisionReason::FundsShortage) {
                    *fundsshortage_count_90d.get_mut(&c).unwrap() += 1;
                }
            }
        }
        tick_month(&mut app, 1800, m.wrapping_add(1));
    }

    eprintln!("[P27-002-A six-country report] 90-day (3-month) results:");
    for &c in &ALL_SIX {
        let reg = app.world().resource::<CountryRegistry>();
        let country = reg.get(c).unwrap();
        eprintln!(
            "  {c:?}: treasury={:.2} monthly_expenses={:.2} monthly_income={:.2} \
             fundsshortage_observations={}",
            country.treasury, country.monthly_expenses, country.monthly_income, fundsshortage_count_90d[&c]
        );
    }

    advance_months(&mut app, 33); // 合計36か月

    eprintln!("[P27-002-A six-country report] 36-month results:");
    for &c in &ALL_SIX {
        let reg = app.world().resource::<CountryRegistry>();
        let country = reg.get(c).unwrap();
        let power = app.world().resource::<CountryPowerRegistry>().get(c).copied();
        eprintln!(
            "  {c:?}: final_treasury={:.2} final_monthly_expenses={:.2} \
             final_industrial_goods={:.3} final_military_supplies={:.3} \
             factory_level_sum={} mine_level_sum={} \
             power_total_score={:?} power_tier={:?} power_world_rank={:?}",
            country.treasury,
            country.monthly_expenses,
            country.stockpile.get(ResourceType::IndustrialGoods),
            country.stockpile.get(ResourceType::MilitarySupplies),
            factory_level_sum(&app, c),
            mine_level_sum(&app, c),
            power.map(|p| p.total_score),
            power.map(|p| p.power_tier),
            power.map(|p| p.world_rank),
        );
    }

    // 参考値(P27-002-PRE-001実測、固定期待値ではない): Arcadia約-14511、Elfin約+4465、
    // Dwarf約-14123、Oceanic約+206、Passguard/Ferrowynはほぼ変化なし。本テストは
    // これらと厳密一致することを要求せず、あくまで有限値・破綻していないことのみ
    // 固定する(実測値の解釈はレポート側で行う)。
    for &c in &ALL_SIX {
        let reg = app.world().resource::<CountryRegistry>();
        let country = reg.get(c).unwrap();
        assert!(country.treasury.is_finite(), "{c:?}: treasuryが有限値であるはず");
        assert!(country.monthly_expenses.is_finite(), "{c:?}: monthly_expensesが有限値であるはず");
        assert!(country.monthly_expenses >= 0.0, "{c:?}: monthly_expensesが非負であるはず");
    }
    assert_eq!(
        app.world().resource::<CountryPowerRegistry>().country_count(),
        6,
        "36か月後も6か国全てが国家総合力評価の対象であり続けるはず(消滅していない)"
    );
}
