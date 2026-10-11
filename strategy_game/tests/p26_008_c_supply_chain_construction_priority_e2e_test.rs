//! P26-008-C: MilitaryFactoryが要求するIndustrialGoods生産経路の構造的欠損を、
//! 既存の事前開発Mine規則(P26-004-PRE-002)へ最小限に拡張した建設AIの恒久テスト。
//!
//! 純粋な判定ロジック(IndustrialGoods不足検出・Factory構造判定・重複防止・
//! 段階的修復)の単体テストは`src/country/country_ai.rs`(11件)に既にあるため、
//! 本ファイルは**実App・実データ経由の統合**、Save V1継続性、決定性・国家間分離、
//! 6か国の90日/36か月実測、および実データでは分岐が発火しない場合に備えた
//! 合成Passguard型シナリオに集中する。

#[path = "common/sim_calendar.rs"]
mod sim_calendar;

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
use strategy_game::country::country_ai::{CountryAiDecisionReason, CountryAiRegistry};
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
// App-level fixtures(P26-004-PRE-002と同じ構成方針)
// ─────────────────────────────────────────────────────────────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p26_008_c_e2e_{label}_{}_{nanos}_{n}",
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

/// P26-004-PRE-002/P26-008-PRE-002が確立した、day1〜30を含む実カレンダー相当の
/// 簡略暦(`game_day % 30 == country_id % 30`の判定式ではday1/day30を含めないと
/// 一部国家が定期評価から漏れる、P26-008-PRE-002 §該当コメント参照)。
fn advance_one_month(app: &mut App, current_month_index: u8) {
    for day in 1..=30u8 {
        tick_day(app, 1800, current_month_index, day);
    }
    tick_month(app, 1800, current_month_index.wrapping_add(1));
}

/// Dwarf(CountryId(2))の建設AI月次評価日(`current_day % 30 == country_id % 30`)。
const DWARF_MONTHLY_EVAL_DAY: u8 = 2;

/// `CountryAiState.decision_reason`は建設AI(月次)・研究/戦争準備AI(週次)が
/// **同じフィールドを共有**するため、1か月分すべてを一気に進めてから読むと、
/// Dwarfの週次評価日(`day % 7 == 2` → day 2,9,16,23,30)が月次評価日(day 2)より
/// 後にも複数回発生し、月末時点では週次AI(戦争準備AI等)の理由で上書きされて
/// しまう。建設AI自身の判断理由を検証したいテストは、その月の最初の建設AI
/// 評価の直後に読み取る必要がある。
///
/// さらに1か月目(New Game直後)は、`CountryAiState::new`の初期値`dirty=true`
/// により**day 1の時点で既に**cadence条件を無視して全AIが評価される
/// (P26-008-PRE-002が復元した正規挙動)。Dwarf(CountryId(2))自身の月次cadence
/// (`day % 30 == 2`)がたまたまday 2にも一致するため、1か月目だけはday 1で
/// 建設AIが既に評価済みとなり、day 2の評価は「既に対応済み」を確認する
/// **2回目**の(この規則にとっては無意味な)評価になってしまう――そのday 2の
/// 評価では、IndustrialGoods不足は既にqueue内のFactoryで解消済みと判定され、
/// 優先度3([`PeacetimeDevelopment`]、材料十分なら経済成長のためFactoryを選ぶ
/// 既存の無関係な判断)が代わりに発火し、`decision_reason`を上書きしてしまう。
/// そのため1か月目はday 1で、2か月目以降はDwarfの正規月次評価日(day 2)で
/// 読み取る。
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

fn advance_months(app: &mut App, months: u8) {
    for m in 1..=months {
        advance_one_month(app, m);
    }
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

fn set_stockpile(app: &mut App, country_id: CountryId, resource: ResourceType, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(country_id)
        .unwrap()
        .stockpile
        .set(resource, value);
}

// ─────────────────────────────────────────────────────────────────────────
// Test 1-3: 正規経路・優先順位(Food緊急・IndustrialGoods検出)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_01_real_construction_ai_system_detects_a_synthetic_industrial_goods_gap() {
    // 実データの6か国のうち、実際にIndustrialGoods構造的欠損を持つ国が
    // 発火するまで待たず、任意の1国(Dwarf、実データ上は既にFactoryを持つ)へ
    // 合成的な欠損を作ってこの経路自体が実App経由で機能することをまず確認する。
    let dir = TempTestDir::new("t01");
    let mut app = setup_app(None, dir.save_file_config());

    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    // Dwarfの既存Factoryを全て除去し、MilitaryFactoryだけ残す(構造的欠損を合成)。
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_treasury(&mut app, DWARF, 1_000_000.0);
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, DWARF, r, 1000.0);
    }

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(reason, Some(CountryAiDecisionReason::SteelShortageSteelMillPriority));
    let queue = construction_queue_of(&app, DWARF);
    assert!(queue.iter().any(|(_, b)| *b == BuildingType::Factory));
}

#[test]
fn test_02_food_emergency_still_takes_priority_over_the_gap() {
    let dir = TempTestDir::new("t02");
    let mut app = setup_app(None, dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_treasury(&mut app, DWARF, 1_000_000.0);
    set_stockpile(&mut app, DWARF, ResourceType::Food, 5.0); // 既存緊急しきい値未満

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(reason, Some(CountryAiDecisionReason::FoodShortageFarmPriority));
}

// ─────────────────────────────────────────────────────────────────────────
// Test 4-6: Factory存在判定・重複防止
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_04_existing_factory_means_no_gap_is_flagged() {
    let dir = TempTestDir::new("t04");
    let mut app = setup_app(None, dir.save_file_config());
    // Arcadiaは実データ上既にFactoryとMilitaryFactoryを同一州に持つ。
    advance_one_month(&mut app, 1);
    assert_ne!(decision_reason_of(&app, ARCADIA), Some(CountryAiDecisionReason::SteelShortageSteelMillPriority));
}

#[test]
fn test_05_zero_monthly_output_does_not_cause_a_duplicate_factory() {
    let dir = TempTestDir::new("t05");
    let mut app = setup_app(None, dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_building_level(&mut app, capital, BuildingType::Factory, 1);
    // Coal入力を枯渇させ、当月のFactory実生産を0にする(構造的存在は変えない)。
    set_stockpile(&mut app, DWARF, ResourceType::Coal, 0.0);
    set_treasury(&mut app, DWARF, 1_000_000.0);

    advance_one_month(&mut app, 1);

    assert_ne!(
        decision_reason_of(&app, DWARF),
        Some(CountryAiDecisionReason::SteelShortageSteelMillPriority),
        "当月の実生産が0でも、Factory自体は存在するため重複建設してはいけない"
    );
}

#[test]
fn test_06_no_duplicate_factory_rule_reactivation_while_one_is_under_construction() {
    let dir = TempTestDir::new("t06");
    let mut app = setup_app(None, dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_treasury(&mut app, DWARF, 1_000_000.0);
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, DWARF, r, 1000.0);
    }

    let first_reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);
    assert_eq!(first_reason, Some(CountryAiDecisionReason::SteelShortageSteelMillPriority));
    let after_first = construction_queue_of(&app, DWARF);
    assert!(after_first.iter().any(|(_, b)| *b == BuildingType::Factory));

    // 2か月目、まだFactoryは未完成(建設中)のまま再評価する。
    let second_reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 2);
    assert_ne!(
        second_reason,
        Some(CountryAiDecisionReason::SteelShortageSteelMillPriority),
        "建設中のFactoryがある間はIndustrialGoods事前開発規則が再発火しないはず"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test 7-9: 段階的修復(Factory完成後にCoal Mineへ接続)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_07_staged_repair_connects_to_coal_mine_after_factory_completes() {
    let dir = TempTestDir::new("t07");
    let mut app = setup_app(None, dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            // 実データのDwarfは既にIron/Coalの完全な供給網を持つ(既存の
            // 恒久テストの前提)。段階的修復(Factory完成→Coal Mine事前開発)を
            // クリーンに検証するため、既存のMine・鉱床を全て取り除いてから、
            // このテスト専用のCoal鉱床だけを与える。
            state.buildings.remove(&BuildingType::Factory);
            state.buildings.remove(&BuildingType::Mine);
            state.resource_deposits.clear();
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    // Factory完成済み(段階1は解決済みという前提から始める)、Coal鉱床は未活用のまま。
    set_building_level(&mut app, capital, BuildingType::Factory, 1);
    set_deposit(&mut app, capital, ResourceType::Coal, true);
    set_treasury(&mut app, DWARF, 1_000_000.0);
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, DWARF, r, 1000.0);
    }

    advance_one_month(&mut app, 1);

    let queue = construction_queue_of(&app, DWARF);
    assert!(
        queue.iter().any(|(sid, b)| *b == BuildingType::Mine && *sid == capital),
        "Factory完成後は、Factory自身が要求するCoal鉱床の事前開発(Mine)へ接続されるはず"
    );
}

#[test]
fn test_08_no_coal_deposit_never_fabricates_a_mine() {
    let dir = TempTestDir::new("t08");
    let mut app = setup_app(None, dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
            state.resource_deposits.retain(|d| d.resource_type != ResourceType::Coal);
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_building_level(&mut app, capital, BuildingType::Factory, 1);
    set_treasury(&mut app, DWARF, 1_000_000.0);
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, DWARF, r, 1000.0);
    }

    advance_months(&mut app, 3);

    let queue = construction_queue_of(&app, DWARF);
    assert!(
        !queue.iter().any(|(_, b)| *b == BuildingType::Mine),
        "Coal鉱床が皆無ならMineを捏造してはいけない"
    );
}

#[test]
fn test_09_no_militaryfactory_means_no_factory_from_this_rule_alone() {
    let dir = TempTestDir::new("t09");
    let mut app = setup_app(None, dir.save_file_config());
    // Elfin(実データ上MilitaryFactoryを既に持つ)からMilitaryFactoryを除去する。
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == ELFIN) {
            state.buildings.remove(&BuildingType::MilitaryFactory);
            state.buildings.remove(&BuildingType::Factory);
        }
    }
    // 原材料は十分に保ち、優先度3の「経済成長のためFactory」判断とも区別する
    // ため、あえてFactory自体は許容しつつ理由コードだけを確認する。
    set_treasury(&mut app, ELFIN, 1_000_000.0);
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, ELFIN, r, 1000.0);
    }

    advance_one_month(&mut app, 1);

    assert_ne!(
        decision_reason_of(&app, ELFIN),
        Some(CountryAiDecisionReason::SteelShortageSteelMillPriority),
        "IndustrialGoodsを要求する所有建物(MilitaryFactory)がなければ、この規則だけを\
         理由にFactoryを建設してはいけない"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Test 10-11: 資金不足・プレイヤー/消滅国家保護
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_10_insufficient_funds_preserves_existing_outcome() {
    let dir = TempTestDir::new("t10");
    let mut app = setup_app(None, dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_treasury(&mut app, DWARF, 1.0); // Factory建設費(1000)未満
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, DWARF, r, 1000.0);
    }

    advance_one_month(&mut app, 1);

    let queue = construction_queue_of(&app, DWARF);
    assert!(queue.is_empty(), "資金不足時は外生的な資金付与なしに建設してはいけない");
}

#[test]
fn test_11_player_country_receives_no_ai_construction() {
    let dir = TempTestDir::new("t11");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_treasury(&mut app, DWARF, 1_000_000.0);
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, DWARF, r, 1000.0);
    }

    advance_one_month(&mut app, 1);

    let queue = construction_queue_of(&app, DWARF);
    assert!(queue.is_empty(), "プレイヤー国家へAI建設を行ってはいけない");
}

// ─────────────────────────────────────────────────────────────────────────
// Test 12-13: 国家間分離・登録順反転
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_12_decisions_are_isolated_across_countries() {
    let dir = TempTestDir::new("t12");
    let mut app = setup_app(None, dir.save_file_config());
    let dwarf_capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
        }
    }
    set_building_level(&mut app, dwarf_capital, BuildingType::MilitaryFactory, 1);
    set_treasury(&mut app, DWARF, 1_000_000.0);
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, DWARF, r, 1000.0);
    }

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(reason, Some(CountryAiDecisionReason::SteelShortageSteelMillPriority));
    // Arcadiaは既にFactoryを持つため、Dwarfの合成欠損とは無関係にこの規則を発火しない。
    assert_ne!(decision_reason_of(&app, ARCADIA), Some(CountryAiDecisionReason::SteelShortageSteelMillPriority));
}

#[test]
fn test_13_result_does_not_depend_on_state_registration_order() {
    fn run(reverse: bool) -> Option<CountryAiDecisionReason> {
        let dir = TempTestDir::new(&format!("t13_{reverse}"));
        let mut app = setup_app(None, dir.save_file_config());
        let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
        {
            let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
            for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
                state.buildings.remove(&BuildingType::Factory);
            }
            if reverse {
                state_registry.states.reverse();
            }
        }
        set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
        set_treasury(&mut app, DWARF, 1_000_000.0);
        for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
            set_stockpile(&mut app, DWARF, r, 1000.0);
        }
        advance_one_month(&mut app, 1);
        decision_reason_of(&app, DWARF)
    }

    assert_eq!(run(false), run(true));
}

// ─────────────────────────────────────────────────────────────────────────
// Test 14-16: Save V1(現行/2回連続/旧Save)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_14_decision_is_reproduced_identically_after_save_and_load() {
    let dir = TempTestDir::new("t14");
    let mut app = setup_app(None, dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_treasury(&mut app, DWARF, 1_000_000.0);
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, DWARF, r, 1000.0);
    }

    send_save_request(&mut app);
    send_load_request(&mut app);

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(reason, Some(CountryAiDecisionReason::SteelShortageSteelMillPriority));
    assert!(construction_queue_of(&app, DWARF).iter().any(|(_, b)| *b == BuildingType::Factory));
}

#[test]
fn test_15_double_save_load_does_not_change_the_outcome() {
    let dir = TempTestDir::new("t15");
    let mut app = setup_app(None, dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_treasury(&mut app, DWARF, 1_000_000.0);
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, DWARF, r, 1000.0);
    }

    send_save_request(&mut app);
    send_load_request(&mut app);
    send_save_request(&mut app);
    send_load_request(&mut app);

    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);

    assert_eq!(reason, Some(CountryAiDecisionReason::SteelShortageSteelMillPriority));
}

#[test]
fn test_16_old_save_without_any_new_field_loads_and_evaluates_normally() {
    // P26-008-Cは新規Saveフィールドを一切追加しないため、既存のSaveがそのまま
    // 正常にロードできることを実際にsave→loadして確認する(特別なmigrationは不要)。
    let dir = TempTestDir::new("t16");
    let mut app = setup_app(None, dir.save_file_config());
    send_save_request(&mut app);
    send_load_request(&mut app);
    advance_one_month(&mut app, 1);
    // パニックしないこと・少なくとも1国は何らかの正規理由を得ることを確認する。
    assert!(decision_reason_of(&app, ARCADIA).is_some());
}

// ─────────────────────────────────────────────────────────────────────────
// Test 17: AI評価cadenceを迂回しない
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_17_construction_ai_still_follows_the_existing_monthly_cadence() {
    let dir = TempTestDir::new("t17");
    let mut app = setup_app(None, dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            state.buildings.remove(&BuildingType::Factory);
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_treasury(&mut app, DWARF, 1_000_000.0);
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, DWARF, r, 1000.0);
    }

    // このテストは既存cadence実装(`day % 30 == country_id % 30 || dirty`、
    // P26-008-PRE-002が復元・P26-008-Cは無変更)そのものを変えていないことの
    // 間接的な確認として、Dwarfの正規月次評価日(day 2)で意図した結果が
    // 出ることを見る。
    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);
    assert_eq!(reason, Some(CountryAiDecisionReason::SteelShortageSteelMillPriority));
}

// ─────────────────────────────────────────────────────────────────────────
// §18: 実データ6か国の90日・36か月実測
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
fn test_90_day_and_36_month_six_country_real_data_verification() {
    let dir = TempTestDir::new("real_six_country");
    let mut app = setup_app(None, dir.save_file_config());

    eprintln!("[P26-008-C real-data report] initial state:");
    let mut initial_gap: HashMap<CountryId, bool> = HashMap::new();
    for &c in &ALL_SIX {
        let gap = militaryfactory_needs_industrial_goods_but_lacks_factory(&app, c);
        initial_gap.insert(c, gap);
        eprintln!(
            "  {c:?}: initial_factory_level_sum={} initial_industrial_goods_gap={}",
            factory_level_sum(&app, c),
            gap
        );
    }

    let mut factory_first_queued_month: HashMap<CountryId, Option<u8>> =
        ALL_SIX.iter().map(|&c| (c, None)).collect();
    // `CountryAiDecisionReason`は建設AI(月次)・研究/戦争準備AI(週次)が同じ
    // フィールドを共有するため(モジュールdoc参照)、月末にまとめて読むと
    // 後続の週次評価で上書きされ得る。ここでは日次でqueueの変化そのものを
    // 監視し、Factoryが実際に現れた**その日**の理由を記録することで、
    // どの国がP26-008-Cの新規規則(SteelShortageSteelMillPriority)によって
    // Factoryを得たのか、既存の優先度3(PeacetimeDevelopment、無関係な
    // 経済成長判断)によって得たのかを正確に切り分ける。
    let mut decision_reason_when_first_queued: HashMap<CountryId, Option<CountryAiDecisionReason>> =
        ALL_SIX.iter().map(|&c| (c, None)).collect();

    for m in 1..=3u8 {
        for day in 1..=30u8 {
            let had_factory_before: HashMap<CountryId, bool> = ALL_SIX
                .iter()
                .map(|&c| (c, construction_queue_of(&app, c).iter().any(|(_, b)| *b == BuildingType::Factory)))
                .collect();
            tick_day(&mut app, 1800, m, day);
            for &c in &ALL_SIX {
                if factory_first_queued_month[&c].is_none()
                    && !had_factory_before[&c]
                    && construction_queue_of(&app, c).iter().any(|(_, b)| *b == BuildingType::Factory)
                {
                    factory_first_queued_month.insert(c, Some(m));
                    decision_reason_when_first_queued.insert(c, decision_reason_of(&app, c));
                }
            }
        }
        tick_month(&mut app, 1800, m.wrapping_add(1));
    }

    eprintln!("[P26-008-C real-data report] 90-day (3-month) results:");
    for &c in &ALL_SIX {
        eprintln!(
            "  {c:?}: first_factory_queued_month={:?} reason_at_that_time={:?} \
             factory_level_sum={} industrial_goods_gap_now={}",
            factory_first_queued_month[&c],
            decision_reason_when_first_queued[&c],
            factory_level_sum(&app, c),
            militaryfactory_needs_industrial_goods_but_lacks_factory(&app, c)
        );
    }

    advance_months(&mut app, 33); // 合計36か月

    eprintln!("[P26-008-C real-data report] 36-month results:");
    for &c in &ALL_SIX {
        eprintln!(
            "  {c:?}: factory_level_sum={} industrial_goods_gap_now={} \
             final_treasury={:.2} final_industrial_goods={:.3} final_military_supplies={:.3}",
            factory_level_sum(&app, c),
            militaryfactory_needs_industrial_goods_but_lacks_factory(&app, c),
            app.world().resource::<CountryRegistry>().get(c).unwrap().treasury,
            app.world().resource::<CountryRegistry>().get(c).unwrap().stockpile.get(ResourceType::IndustrialGoods),
            app.world().resource::<CountryRegistry>().get(c).unwrap().stockpile.get(ResourceType::MilitarySupplies),
        );
    }

    // 実測結果(2026年時点の実データ): Elfin/Oceanic/Passguardは実際に
    // MilitaryFactoryを持ちながらFactoryを持たない構造的欠損を最初から抱えており、
    // 1か月目にP26-008-Cの新規規則(SteelShortageSteelMillPriority)によって
    // Factoryが建設される。Arcadia/Dwarf/Ferrowynは既にFactoryを持つため、
    // 同じ1か月目に建つFactory追加は既存の優先度3(PeacetimeDevelopment、無関係な
    // 経済成長判断)によるものである。この帰属を、実際に構造的欠損を持っていた
    // 国とそうでない国とで正確に一致させ、実データでの発火をレコードとして固定する
    // (spec §18: 発火しない場合は正直に報告する、と同時に、実際に発火する場合は
    // その事実を検証で裏付ける)。
    for &c in &ALL_SIX {
        assert!(app.world().resource::<CountryRegistry>().get(c).unwrap().treasury.is_finite());
        if initial_gap[&c] {
            assert_eq!(
                decision_reason_when_first_queued[&c],
                Some(CountryAiDecisionReason::SteelShortageSteelMillPriority),
                "{c:?}: 初期状態でIndustrialGoods構造的欠損を持つ国は、\
                 P26-008-Cの新規規則によってFactoryを得るはず"
            );
        } else {
            assert_ne!(
                decision_reason_when_first_queued[&c],
                Some(CountryAiDecisionReason::SteelShortageSteelMillPriority),
                "{c:?}: 初期状態でIndustrialGoods構造的欠損がない国のFactory追加は、\
                 この新規規則が理由であってはいけない"
            );
        }
        // 36か月後には、初期に欠損を持っていた国も含め全国家でIndustrialGoods
        // 構造的欠損が解消していることを確認する(実データはこのウィンドウ内で
        // 解決可能だった、という実測結果を固定する)。
        //
        // 【P28-010-IMPL-001により例外を1件追加】: 入力引落しの原子的会計でArcadia等の
        // 無償産出(MilitarySupplies)が消え、36か月までの戦争経過が変わった(戦争総数
        // 7→10)。Passguardは、修正前は保持していた州10(Factory Level1・MilitaryFactory
        // Level1)を戦争で失い、Factoryのない州13(MilitaryFactory Level1)だけが残る。
        // この欠損はAI建設判断の不発ではなくFactory州の喪失によるため、州10の所有者で
        // 確認する。
        if c == CountryId(4) {
            let state10_owner = app
                .world()
                .resource::<strategy_game::state::data::StateRegistry>()
                .states
                .iter()
                .find(|s| s.id == StateId(10))
                .map(|s| s.owner_country_id);
            assert_ne!(
                state10_owner,
                Some(c),
                "{c:?}: Factory州(10)を失ったことだけが構造的欠損の残る理由のはず"
            );
            assert!(
                militaryfactory_needs_industrial_goods_but_lacks_factory(&app, c),
                "{c:?}: Factory州の喪失後はMilitaryFactoryのみが残る"
            );
            continue;
        }
        assert!(
            !militaryfactory_needs_industrial_goods_but_lacks_factory(&app, c),
            "{c:?}: 36か月後もIndustrialGoods構造的欠損が残っている"
        );
    }
}

/// spec §18: 実データで分岐が発火しない場合に備え、合成的にPassguard型
/// (MilitaryFactoryあり・Factoryなし・Coal鉱床未活用)の構造的欠損を作り、
/// Factory建設→Coal Mine建設→IndustrialGoods生産→MilitarySupplies生産という
/// 一連の流れを実App・実Systemで最後まで実証する。
#[test]
fn test_synthetic_passguard_style_full_supply_chain_repair() {
    let dir = TempTestDir::new("synthetic_passguard");
    let mut app = setup_app(None, dir.save_file_config());
    let capital = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().capital_state_id;

    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut().filter(|s| s.owner_country_id == DWARF) {
            // 実データのDwarfは既にFactory・Mine・Iron/Coal鉱床を備えた完全な
            // 供給網を持つ(既存の恒久テストの前提)。Passguard型の「Factoryも
            // Mineもない構造的欠損国家」から出発する段階的修復を実App・実
            // Systemでクリーンに実証するため、既存のFactory・Mineと鉱床を
            // 全て取り除いてから、このテスト専用のCoal鉱床だけを与える。
            state.buildings.remove(&BuildingType::Factory);
            state.buildings.remove(&BuildingType::Mine);
            state.resource_deposits.clear();
        }
    }
    set_building_level(&mut app, capital, BuildingType::MilitaryFactory, 1);
    set_deposit(&mut app, capital, ResourceType::Coal, true);
    set_treasury(&mut app, DWARF, 1_000_000.0);
    for r in [ResourceType::Food, ResourceType::Wood, ResourceType::Iron, ResourceType::Coal] {
        set_stockpile(&mut app, DWARF, r, 1000.0);
    }

    assert_eq!(factory_level_sum(&app, DWARF), 0);
    assert_eq!(building_level(&app, capital, BuildingType::Mine), 0);

    // 段階1: Factory建設が選ばれる。
    let reason = advance_one_month_and_read_dwarf_construction_reason(&mut app, 1);
    assert_eq!(reason, Some(CountryAiDecisionReason::SteelShortageSteelMillPriority));
    assert!(construction_queue_of(&app, DWARF).iter().any(|(_, b)| *b == BuildingType::Factory));

    // Factoryが完成するまで、十分な月数進める(建設進捗は既存の日次建設Systemに
    // 委ねる――本テストは建設期間そのものを改変しない)。
    advance_months(&mut app, 5);
    assert!(factory_level_sum(&app, DWARF) > 0, "Factoryが完成しているはず");

    // 段階2: Factory完成後、Coal Mineの事前開発へ接続される。
    let mine_queued_before = building_level(&app, capital, BuildingType::Mine) > 0
        || construction_queue_of(&app, DWARF).iter().any(|(_, b)| *b == BuildingType::Mine);
    if !mine_queued_before {
        advance_one_month(&mut app, 7);
    }
    let mine_eventually_queued_or_built = building_level(&app, capital, BuildingType::Mine) > 0
        || construction_queue_of(&app, DWARF).iter().any(|(_, b)| *b == BuildingType::Mine);
    assert!(
        mine_eventually_queued_or_built,
        "Factory完成後は、Factoryが要求するCoalの鉱床事前開発(Mine)が選ばれるはず"
    );

    // Mineが完成するまで進める。
    advance_months(&mut app, 6);

    eprintln!(
        "[P26-008-C synthetic Passguard] final: factory_level={} mine_level={} \
         industrial_goods={:.3} military_supplies={:.3}",
        factory_level_sum(&app, DWARF),
        building_level(&app, capital, BuildingType::Mine),
        app.world().resource::<CountryRegistry>().get(DWARF).unwrap().stockpile.get(ResourceType::IndustrialGoods),
        app.world().resource::<CountryRegistry>().get(DWARF).unwrap().stockpile.get(ResourceType::MilitarySupplies),
    );

    // 段階3: 最終的にIndustrialGoods・MilitarySuppliesの両方が正の値になる
    // (供給鎖が実際に機能していることの直接証拠)。
    let industrial_goods =
        app.world().resource::<CountryRegistry>().get(DWARF).unwrap().stockpile.get(ResourceType::IndustrialGoods);
    let military_supplies =
        app.world().resource::<CountryRegistry>().get(DWARF).unwrap().stockpile.get(ResourceType::MilitarySupplies);
    assert!(industrial_goods > 0.0, "IndustrialGoods生産経路が最終的に機能しているはず");
    assert!(military_supplies > 0.0, "MilitarySupplies生産経路が最終的に機能しているはず");
}
