//! P26-004: Division日次補給需要の確定・国家備蓄からの実消費・不足時の決定的な
//! Hamilton配分を、実App(実Bevy schedule)経由で検証する。
//!
//! 純粋関数レベルの網羅的な検証(需要の再利用・Hamilton配分・overflow・原子性・
//! 決定性)は`src/military/daily_supply.rs`の単体テスト(24件)が担う。本ファイルは
//! それらが実際に本番System(`military::daily_supply::process_daily_military_supply_
//! consumption`、`DailySimulationSet::Economy`に登録)として実Bevy scheduleへ接続
//! されていること、正規の日次処理順序(spec §6: 前日終値→需要→消費→充足率確定→
//! 当日の移動・戦闘・募兵完了)が実際に成立すること、Save V1互換性、90日・36か月の
//! 実App検証を担う。
//!
//! `verification_logs/phase-26/p26-004-pre-005/`・`p26-004-pre-004/`・`p26-det-001*/`が
//! 確立した実App harness(`setup_app_in_playing`等)を踏襲する(実データ・実App、
//! 診断用の無限資金フィクスチャは使用しない)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::{ResourceType, military_supplies_units_to_value};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::{Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry, STANDARD_INFANTRY_DEFINITION_ID};
use strategy_game::military::recruitment::request_recruitment;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::dto::SaveGameV1;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
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
// App-level fixtures(P26-004-PRE-005/PRE-004/DET-001と同じ構成方針)
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
        "strategy_game_p26_004_e2e_{label}_{}_{nanos}_{n}",
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

    fn final_path(&self) -> std::path::PathBuf {
        self.0.join("savegame_v1.ron")
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
    app.update(); // OnEnter(Playing): spawn_debug_divisions -> ブリッジ付与 -> 国力再評価

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

/// 通常の日次ティック(`DayChangedMessage`のみ)。
fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn tick_month_only(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

/// P26-004-PRE-002〜005と同じ簡略暦(1か月=27日[day2〜28]+月境界tick)で1か月進める。
fn advance_one_month(app: &mut App, current_month_index: u8) {
    for day in 2..=28u8 {
        tick_day(app, 1800, current_month_index, day);
    }
    tick_month_only(app, 1800, current_month_index.wrapping_add(1));
}

/// spec §6: 月境界日には、実際の`advance_game_date`と同じく`DayChangedMessage`と
/// `MonthChangedMessage`が**同一フレーム**(同一`app.update()`)内で両方発行される。
/// 既存の合成harness(`tick_day`と`tick_month_only`を別々のフレームで呼ぶ)は、この
/// 「同日に両方発行される」実際の月境界日の挙動を再現していない――本ファイルは
/// この正確な挙動を専用に再現し、月次生産→日次消費の順序を直接検証する。
fn tick_month_boundary_day(app: &mut App, year: i32, new_month: u8, new_day: u8) {
    let mut day_state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    day_state
        .get_mut(app.world_mut())
        .unwrap()
        .write(DayChangedMessage { year, month: new_month, day: new_day });
    day_state.apply(app.world_mut());

    let mut month_state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    month_state
        .get_mut(app.world_mut())
        .unwrap()
        .write(MonthChangedMessage { year, month: new_month });
    month_state.apply(app.world_mut());

    app.update();
}

fn stockpile_of(app: &App, country_id: CountryId, resource: ResourceType) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(resource)
}

fn fulfillment_of(app: &App, division_id: DivisionId) -> f64 {
    app.world().resource::<MilitaryRegistry>().supply_fulfillment_of(division_id)
}

fn division_ids_of(app: &App, country_id: CountryId) -> Vec<DivisionId> {
    let mut ids: Vec<DivisionId> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == country_id)
        .map(|d| d.id)
        .collect();
    ids.sort_by_key(|id| id.0);
    ids
}

/// New Gameの満編制Infantryと同じ規格の追加Divisionを、指定した国家の首都州へ
/// 直接World経由で挿入する(募兵パイプラインを経由しない、テスト専用の高速な
/// シナリオ構築)。`MilitaryRegistry::add_division`が実際の`DivisionId`を採番する。
fn add_full_strength_infantry(app: &mut App, owner: CountryId, state: StateId) -> DivisionId {
    let manpower = {
        let military_registry = app.world().resource::<MilitaryRegistry>();
        military_registry
            .definitions
            .get(&STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .required_manpower
    };
    let division = Division {
        id: DivisionId(0),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: state,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower,
        max_manpower: manpower,
        equipment: 100.0,
        max_equipment: 100.0,
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
    };
    let mut military_registry = app.world_mut().resource_mut::<MilitaryRegistry>();
    military_registry.add_division(division)
}

fn set_stockpile(app: &mut App, country_id: CountryId, value: f64) {
    let mut registry = app.world_mut().resource_mut::<CountryRegistry>();
    registry.get_mut(country_id).unwrap().stockpile.set(ResourceType::MilitarySupplies, value);
}

fn capital_of(app: &App, country_id: CountryId) -> StateId {
    app.world().resource::<CountryRegistry>().get(country_id).unwrap().capital_state_id
}

fn rewrite_save_file(path: &std::path::Path, edit: impl FnOnce(&mut SaveGameV1)) {
    let contents = std::fs::read_to_string(path).expect("read save file");
    let mut save: SaveGameV1 = ron::from_str(&contents).expect("parse SaveGameV1");
    edit(&mut save);
    let ron_str = ron::to_string(&save).expect("serialize SaveGameV1");
    std::fs::write(path, ron_str).expect("write save file");
}

// ─────────────────────────────────────────────────────────────────────────
// Category A: 正規需要関数の再利用が実Systemへ接続されていること
// ─────────────────────────────────────────────────────────────────────────

/// New Gameの満編制Infantry1個師団は、P26-004-PRE-005が確立した正規式どおり
/// 1.0/日を消費する。単一の日次ティック(月次生産を挟まない)でこれを直接確認する
/// ことで、`division_daily_demand_units`(P26-004-PRE-005)が複製されず実Systemへ
/// そのまま再利用されていることを実証する。
#[test]
fn test_a1_full_strength_infantry_consumes_exactly_one_per_day_via_real_system() {
    let dir = TempTestDir::new("a1");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let before = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);
    assert_eq!(before, 31.0);

    tick_day(&mut app, 1800, 1, 2);

    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 30.0);
    let ids = division_ids_of(&app, DWARF);
    assert_eq!(ids.len(), 1);
    assert_eq!(fulfillment_of(&app, ids[0]), 1.0);
}

/// 複数Division(Infantry+Infantry)を保有する国家の日次消費は、単一Divisionの
/// 単純な倍数として実App上でも一致する(`country_daily_demand_units`の合算式が
/// そのまま使われていることの実証)。
#[test]
fn test_a2_two_divisions_consume_exactly_double_via_real_system() {
    let dir = TempTestDir::new("a2");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let capital = capital_of(&app, DWARF);
    add_full_strength_infantry(&mut app, DWARF, capital);

    let before = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);
    assert_eq!(before, 31.0, "New Game stockpile must still be the untouched bridge grant");

    tick_day(&mut app, 1800, 1, 2);

    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 29.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Category B/C: 備蓄十分・不足時のHamilton配分(実App)
// ─────────────────────────────────────────────────────────────────────────

/// 備蓄十分(31.0)なら、2個Divisionとも需要全量(1.0ずつ)が配分され、充足率は
/// 両方とも1.0。
#[test]
fn test_b1_sufficient_stockpile_fully_allocates_both_real_divisions() {
    let dir = TempTestDir::new("b1");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let capital = capital_of(&app, DWARF);
    let extra = add_full_strength_infantry(&mut app, DWARF, capital);
    let ids = division_ids_of(&app, DWARF);
    assert_eq!(ids.len(), 2);

    tick_day(&mut app, 1800, 1, 2);

    for id in &ids {
        assert_eq!(fulfillment_of(&app, *id), 1.0);
    }
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 29.0);
    let _ = extra;
}

/// 備蓄不足時、2個の同一需要Divisionへ1 unitだけ不足する状況を実App上で作り、
/// remainder同値時のDivisionId昇順tie-breakが実際に機能することを確認する
/// (spec §7.1、`src/military/daily_supply.rs`単体テストと同じ数学だが、実Systemの
/// 出力として確認する)。
#[test]
fn test_c1_one_unit_short_real_app_tie_break_favors_lower_division_id() {
    let dir = TempTestDir::new("c1");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let capital = capital_of(&app, DWARF);
    let _extra = add_full_strength_infantry(&mut app, DWARF, capital);
    let ids = division_ids_of(&app, DWARF);
    assert_eq!(ids.len(), 2);
    assert!(ids[0].0 < ids[1].0, "division_ids_of returns ascending DivisionId order");

    // 総需要2.0(units: 2_000_000)からちょうど1 unit不足させる。
    let consumed_units = 2_000_000u128 - 1;
    set_stockpile(&mut app, DWARF, military_supplies_units_to_value(consumed_units as u64));

    tick_day(&mut app, 1800, 1, 2);

    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 0.0);
    assert_eq!(fulfillment_of(&app, ids[0]), 1.0, "lower DivisionId must receive the tie-break unit");
    assert!(fulfillment_of(&app, ids[1]) < 1.0, "the other division absorbs the one-unit shortfall");
}

/// 備蓄0かつ需要が正なら、配分0・充足率0.0(spec §7.2)。
#[test]
fn test_c2_zero_stockpile_gives_zero_allocation_and_zero_fulfillment_real_app() {
    let dir = TempTestDir::new("c2");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    set_stockpile(&mut app, DWARF, 0.0);

    tick_day(&mut app, 1800, 1, 2);

    let ids = division_ids_of(&app, DWARF);
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 0.0);
    assert_eq!(fulfillment_of(&app, ids[0]), 0.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Category D: 原子性・国家間分離(実App)
// ─────────────────────────────────────────────────────────────────────────

/// 1か国のDivisionが不正(manpower>max_manpower)でも、他国家の日次補給処理は
/// 正常に続行される(spec §9)。不正国家自身の備蓄・充足率は変更されない。
#[test]
fn test_d1_one_countrys_corrupt_division_does_not_disturb_others_real_app() {
    let dir = TempTestDir::new("d1");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());

    // ARCADIA唯一のDivisionを不正化する(manpower > max_manpower)。
    {
        let mut military_registry = app.world_mut().resource_mut::<MilitaryRegistry>();
        let division = military_registry
            .divisions
            .values_mut()
            .find(|d| d.owner == ARCADIA)
            .unwrap();
        division.manpower = division.max_manpower + 1;
    }
    let arcadia_before = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    let dwarf_before = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);

    tick_day(&mut app, 1800, 1, 2);

    assert_eq!(
        stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies),
        arcadia_before,
        "the corrupt country's stockpile must be left completely untouched"
    );
    assert_eq!(
        stockpile_of(&app, DWARF, ResourceType::MilitarySupplies),
        dwarf_before - 1.0,
        "an unrelated healthy country must still be processed normally"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Category E: 決定性(実App、独立した2つのAppインスタンス比較)
// ─────────────────────────────────────────────────────────────────────────

/// 全く同じ操作列(New Game→数日ティック)を独立した2つのAppインスタンスへ適用すると、
/// 全6か国の最終備蓄・全Divisionの充足率がビット単位で一致する。
#[test]
fn test_e1_two_independent_app_instances_produce_identical_results() {
    fn run() -> (HashMap<CountryId, f64>, HashMap<DivisionId, f64>) {
        let dir = TempTestDir::new("e1");
        let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
        for day in 2..=10u8 {
            tick_day(&mut app, 1800, 1, day);
        }
        let stockpiles: HashMap<CountryId, f64> = ALL_SIX
            .iter()
            .map(|&c| (c, stockpile_of(&app, c, ResourceType::MilitarySupplies)))
            .collect();
        let fulfillments: HashMap<DivisionId, f64> = app
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .keys()
            .map(|&id| (id, fulfillment_of(&app, id)))
            .collect();
        (stockpiles, fulfillments)
    }

    let a = run();
    let b = run();
    assert_eq!(a.0, b.0, "world stockpiles must match bit-for-bit across independent runs");
    assert_eq!(a.1, b.1, "division fulfillment values must match bit-for-bit across independent runs");
}

// ─────────────────────────────────────────────────────────────────────────
// Category F: 時系列・System順序(実App)
// ─────────────────────────────────────────────────────────────────────────

/// spec §6: 月境界日には月次生産が同日の日次消費より先に確定していなければならない。
/// 備蓄を「単独では当日の需要を賄えない」水準まで意図的に減らした状態で、実際の
/// 月境界日(`DayChangedMessage`と`MonthChangedMessage`が同一フレームで発行される、
/// `tick_month_boundary_day`)を1回だけ発火させ、当日のDivisionが完全充足(1.0)に
/// なることを確認する――これは月次生産が先に備蓄へ加算され、消費がその後の
/// (production済みの)備蓄から行われた場合にのみ成立する結果である。
#[test]
fn test_f1_month_boundary_day_production_happens_before_same_day_consumption() {
    let dir = TempTestDir::new("f1");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    // 生産が確実に軌道に乗るまで数か月分進める(Arcadiaは既存テストで月12時点で
    // 421.2程度の生産に達することが確認済み、月6時点でも十分正の生産がある)。
    for m in 1..=6u8 {
        advance_one_month(&mut app, m);
    }
    // 当日の需要(1.0)をわずかに下回る水準まで意図的に減らす。月次生産が消費より
    // 先に走らなければ、当日は部分充足(<1.0)になるはずの水準。
    let short_of_demand = military_supplies_units_to_value(999_999);
    set_stockpile(&mut app, ARCADIA, short_of_demand);

    tick_month_boundary_day(&mut app, 1800, 8, 1);

    let ids = division_ids_of(&app, ARCADIA);
    assert_eq!(
        fulfillment_of(&app, ids[0]),
        1.0,
        "on a real month-boundary day, monthly production must land before same-day \
         consumption is computed, so a division whose demand alone would not have been \
         covered by the pre-tick stockpile must still be fully fulfilled"
    );
}

/// spec §6・§12: 前日終値スナップショット。募兵完了によって当日新規生成された
/// Divisionは、生成された当日は需要を持たず(日次補給処理は募兵完了より前に実行
/// 済みのため)、翌日から正規に需要を持つ。
#[test]
fn test_f2_a_division_completed_by_recruitment_today_has_no_demand_until_tomorrow() {
    let dir = TempTestDir::new("f2");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let capital = capital_of(&app, DWARF);

    // 募兵完了(`try_spawn_division`)はMilitaryEquipment備蓄からの支給に成功して
    // 初めて完了する(P25-003)。New Gameの初期MilitaryEquipment備蓄は0のため、
    // 十分な量を先に確保しておく(本テストの主旨[募兵完了日の需要扱い]とは無関係な
    // 交絡[装備不足で完了できない]を避けるため)。MilitarySupplies備蓄には触れない。
    {
        let mut country_registry = app.world_mut().resource_mut::<CountryRegistry>();
        country_registry
            .get_mut(DWARF)
            .unwrap()
            .stockpile
            .set(ResourceType::MilitaryEquipment, 9_999.0);
    }

    // 1日で完了する募兵キューを直接投入する(募兵パイプライン自体はP26-004の対象外、
    // 「完了した当日」を作り出すためだけに使う)。
    app.world_mut().resource_scope(|world, military_registry: Mut<MilitaryRegistry>| {
        let mut country_registry = world.resource_mut::<CountryRegistry>();
        let country = country_registry.get_mut(DWARF).unwrap();
        request_recruitment(country, &military_registry, STANDARD_INFANTRY_DEFINITION_ID, capital)
            .unwrap();
        // recruitment_days実データ値に関わらず、テストのために1日で完了させる。
        country.recruitment_queue.last_mut().unwrap().days_remaining = 1;
    });

    let before_ids = division_ids_of(&app, DWARF);
    assert_eq!(before_ids.len(), 1, "precondition: only the New Game division exists so far");
    let before_stockpile = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);

    // Day 2: 日次補給処理(既存Divisionのみ対象)の後に、募兵が完了して新規Division
    // が生成される(MilitaryActionはEconomyより後)。当日はまだ1.0だけの消費のはず。
    tick_day(&mut app, 1800, 1, 2);
    assert_eq!(
        stockpile_of(&app, DWARF, ResourceType::MilitarySupplies),
        before_stockpile - 1.0,
        "the day recruitment completes, only the pre-existing division's demand is charged"
    );
    let after_ids = division_ids_of(&app, DWARF);
    assert_eq!(after_ids.len(), 2, "recruitment must have completed today");
    let new_id = *after_ids.iter().find(|id| !before_ids.contains(id)).unwrap();
    // 今日確定した充足率は、まだ需要判定の対象になっていない(map未登録)ため
    // default(1.0)として読める。
    assert_eq!(fulfillment_of(&app, new_id), 1.0);

    // Day 3: 新規Divisionも需要に含まれ、消費が2.0になる。
    let stockpile_before_day3 = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);
    tick_day(&mut app, 1800, 1, 3);
    assert_eq!(
        stockpile_of(&app, DWARF, ResourceType::MilitarySupplies),
        stockpile_before_day3 - 2.0,
        "from the next day onward, the newly recruited division has real demand too"
    );
}

/// 移動中・戦闘中のDivisionも、日次補給処理からは除外されない(本タスクは
/// 移動速度・organization・戦闘力への接続を一切行わないため、status別の倍率は
/// 存在せず、Idle/Moving/Fightingいずれも同じ需要式で処理される)。
#[test]
fn test_f3_moving_and_fighting_divisions_are_still_processed_for_supply() {
    let dir = TempTestDir::new("f3");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let capital = capital_of(&app, DWARF);
    let moving = add_full_strength_infantry(&mut app, DWARF, capital);
    let fighting = add_full_strength_infantry(&mut app, DWARF, capital);
    {
        let mut military_registry = app.world_mut().resource_mut::<MilitaryRegistry>();
        military_registry.divisions.get_mut(&moving).unwrap().status = DivisionStatus::Moving;
        military_registry.divisions.get_mut(&fighting).unwrap().status = DivisionStatus::Fighting;
    }
    let before = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);

    tick_day(&mut app, 1800, 1, 2);

    // 3個Division(New Game分+moving+fighting)分、3.0消費される。
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), before - 3.0);
    assert_eq!(fulfillment_of(&app, moving), 1.0);
    assert_eq!(fulfillment_of(&app, fighting), 1.0);
}

/// 同じ日に日次補給処理が二重発火しない(1回の`DayChangedMessage`につき、
/// 消費はちょうど1回だけ適用される)。
#[test]
fn test_f4_a_single_day_changed_message_consumes_exactly_once() {
    let dir = TempTestDir::new("f4");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let before = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);

    tick_day(&mut app, 1800, 1, 2);

    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), before - 1.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Category G: Save V1互換性(実App)
// ─────────────────────────────────────────────────────────────────────────

/// `division_supply_fulfillment`マップを持たない合成旧Save(P26-004以前の形式)を
/// ロードしても、既存Divisionの充足率はdefault(1.0)として安全に読める。
#[test]
fn test_g1_old_v1_save_without_fulfillment_map_defaults_to_one() {
    let dir = TempTestDir::new("g1");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    send_save_request(&mut app);
    rewrite_save_file(&dir.final_path(), |save| {
        save.military.division_supply_fulfillment.clear();
    });
    send_load_request(&mut app);

    let ids = division_ids_of(&app, DWARF);
    assert_eq!(fulfillment_of(&app, ids[0]), 1.0);
}

/// 日次消費を経て非自明な充足率になった状態でsave→loadしても、Division充足率が
/// 正確に往復する。
#[test]
fn test_g2_nontrivial_fulfillment_round_trips_through_save_load() {
    let dir = TempTestDir::new("g2");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    // 総需要1.0をわずかに下回る備蓄にして、非自明(<1.0)な充足率を作る。
    set_stockpile(&mut app, DWARF, military_supplies_units_to_value(500_000));
    tick_day(&mut app, 1800, 1, 2);
    let ids = division_ids_of(&app, DWARF);
    let before = fulfillment_of(&app, ids[0]);
    assert!((0.0..1.0).contains(&before), "precondition: partial fulfillment");

    send_save_request(&mut app);
    send_load_request(&mut app);

    assert_eq!(fulfillment_of(&app, ids[0]), before);
    assert_eq!(
        stockpile_of(&app, DWARF, ResourceType::MilitarySupplies),
        military_supplies_units_to_value(0)
    );
}

/// 2回連続のsave→loadでも充足率・備蓄が安定する(同じ日を二重消費しない)。
#[test]
fn test_g3_double_save_load_round_trip_is_stable() {
    let dir = TempTestDir::new("g3");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    tick_day(&mut app, 1800, 1, 2);
    let ids = division_ids_of(&app, DWARF);
    let after_first_tick = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);
    let fulfillment_after_first_tick = fulfillment_of(&app, ids[0]);

    for _ in 0..2 {
        send_save_request(&mut app);
        send_load_request(&mut app);
    }

    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), after_first_tick);
    assert_eq!(fulfillment_of(&app, ids[0]), fulfillment_after_first_tick);
}

/// New Gameのまま数日進めた場合と、同じだけ進めてからsave→loadを挟んだ場合とで、
/// 備蓄・充足率が完全に一致する(New Game/Loaded Game分岐比較)。
#[test]
fn test_g4_new_game_continuation_matches_save_load_branch() {
    let dir_continuous = TempTestDir::new("g4_continuous");
    let mut continuous = setup_app_in_playing(DWARF, dir_continuous.save_file_config());
    for day in 2..=5u8 {
        tick_day(&mut continuous, 1800, 1, day);
    }

    let dir_branch = TempTestDir::new("g4_branch");
    let mut branch = setup_app_in_playing(DWARF, dir_branch.save_file_config());
    tick_day(&mut branch, 1800, 1, 2);
    send_save_request(&mut branch);
    send_load_request(&mut branch);
    for day in 3..=5u8 {
        tick_day(&mut branch, 1800, 1, day);
    }

    assert_eq!(
        stockpile_of(&continuous, DWARF, ResourceType::MilitarySupplies),
        stockpile_of(&branch, DWARF, ResourceType::MilitarySupplies)
    );
    let ids = division_ids_of(&continuous, DWARF);
    assert_eq!(fulfillment_of(&continuous, ids[0]), fulfillment_of(&branch, ids[0]));
}

/// 月境界を跨いだsave/loadでも、生産・消費が二重発火しない。
#[test]
fn test_g5_month_boundary_save_load_does_not_double_fire_production_or_consumption() {
    let dir = TempTestDir::new("g5");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    advance_one_month(&mut app, 1);
    let before = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);

    send_save_request(&mut app);
    send_load_request(&mut app);

    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), before);
}

// ─────────────────────────────────────────────────────────────────────────
// §15: 90日実App検証(全6か国)
// ─────────────────────────────────────────────────────────────────────────

/// 90日分、全6か国の日次処理を実際に動かし、初期備蓄・初回生産日・日次需要・
/// 日次消費・最低備蓄・不足日数・最初の不足日・月境界値・充足率・世界保存則・
/// New Game/Loaded Game分岐を記録する(spec §15)。
#[test]
fn test_90_day_all_six_countries_real_app_verification() {
    let dir = TempTestDir::new("ninety_day");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());

    let initial: HashMap<CountryId, f64> = ALL_SIX
        .iter()
        .map(|&c| (c, stockpile_of(&app, c, ResourceType::MilitarySupplies)))
        .collect();
    for (c, v) in &initial {
        assert_eq!(*v, 31.0, "{c:?} must start at the P26-004-PRE-005 bridge grant");
    }

    let mut min_stockpile: HashMap<CountryId, f64> =
        ALL_SIX.iter().map(|&c| (c, f64::MAX)).collect();
    let mut shortage_days: HashMap<CountryId, u32> = ALL_SIX.iter().map(|&c| (c, 0)).collect();
    let mut first_shortage_day: HashMap<CountryId, Option<u32>> =
        ALL_SIX.iter().map(|&c| (c, None)).collect();
    let mut total_production: HashMap<CountryId, f64> =
        ALL_SIX.iter().map(|&c| (c, 0.0)).collect();
    let mut total_consumption: HashMap<CountryId, f64> =
        ALL_SIX.iter().map(|&c| (c, 0.0)).collect();

    let mut day_counter: u32 = 0;
    for m in 1..=3u8 {
        for day in 2..=28u8 {
            let before: HashMap<CountryId, f64> = ALL_SIX
                .iter()
                .map(|&c| (c, stockpile_of(&app, c, ResourceType::MilitarySupplies)))
                .collect();
            tick_day(&mut app, 1800, m, day);
            day_counter += 1;
            for &c in &ALL_SIX {
                let after = stockpile_of(&app, c, ResourceType::MilitarySupplies);
                let consumed = (before[&c] - after).max(0.0);
                *total_consumption.get_mut(&c).unwrap() += consumed;
                if after < *min_stockpile.get(&c).unwrap() {
                    *min_stockpile.get_mut(&c).unwrap() = after;
                }
                // Divisionごとの充足率<1.0を「不足日」とみなす(いずれかのDivisionが
                // 部分/未充足)。
                let ids = division_ids_of(&app, c);
                let shortage_today = ids.iter().any(|&id| fulfillment_of(&app, id) < 1.0);
                if shortage_today {
                    *shortage_days.get_mut(&c).unwrap() += 1;
                    first_shortage_day.entry(c).and_modify(|slot| {
                        if slot.is_none() {
                            *slot = Some(day_counter);
                        }
                    });
                }
            }
        }
        let before_month: HashMap<CountryId, f64> = ALL_SIX
            .iter()
            .map(|&c| (c, stockpile_of(&app, c, ResourceType::MilitarySupplies)))
            .collect();
        tick_month_only(&mut app, 1800, m.wrapping_add(1));
        for &c in &ALL_SIX {
            let after = stockpile_of(&app, c, ResourceType::MilitarySupplies);
            let produced = (after - before_month[&c]).max(0.0);
            *total_production.get_mut(&c).unwrap() += produced;
        }
    }

    eprintln!("[P26-004 90-day report]");
    for &c in &ALL_SIX {
        eprintln!(
            "  {c:?}: initial={:.6} min={:.6} shortage_days={} first_shortage_day={:?} \
             total_production={:.6} total_consumption={:.6} final={:.6}",
            initial[&c],
            min_stockpile[&c],
            shortage_days[&c],
            first_shortage_day[&c],
            total_production[&c],
            total_consumption[&c],
            stockpile_of(&app, c, ResourceType::MilitarySupplies)
        );
    }

    // 世界保存則: final = initial + production - consumption(丸め誤差なしで一致)。
    for &c in &ALL_SIX {
        let final_value = stockpile_of(&app, c, ResourceType::MilitarySupplies);
        let expected = initial[&c] + total_production[&c] - total_consumption[&c];
        assert!(
            (final_value - expected).abs() < 1e-9,
            "{c:?}: world conservation must hold exactly (final={final_value}, \
             expected={expected})"
        );
        assert!(final_value >= 0.0, "{c:?}: stockpile must never go negative");
        assert!(min_stockpile[&c] >= 0.0, "{c:?}: minimum stockpile must never go negative");
    }

    // New Game / Loaded Game分岐比較(90日時点でsave→loadした場合と継続した場合で
    // 以後の挙動が一致する)。
    send_save_request(&mut app);
    let after_save = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    send_load_request(&mut app);
    assert_eq!(stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies), after_save);
}

// ─────────────────────────────────────────────────────────────────────────
// §15: 36か月実App検証(初期Division固定、通常AI・募兵を含む実プレイ相当)
// ─────────────────────────────────────────────────────────────────────────

/// 36か月、初期Division構成を固定したまま(追加の募兵を行わない)、日次消費だけの
/// 影響を6か国について記録する。
#[test]
fn test_36_month_fixed_initial_divisions_real_app_verification() {
    let dir = TempTestDir::new("thirtysix_fixed");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());

    let initial: HashMap<CountryId, f64> = ALL_SIX
        .iter()
        .map(|&c| (c, stockpile_of(&app, c, ResourceType::MilitarySupplies)))
        .collect();
    let mut min_stockpile: HashMap<CountryId, f64> =
        ALL_SIX.iter().map(|&c| (c, f64::MAX)).collect();
    let mut shortage_days: HashMap<CountryId, u32> = ALL_SIX.iter().map(|&c| (c, 0)).collect();
    let mut first_shortage_day: HashMap<CountryId, Option<u32>> =
        ALL_SIX.iter().map(|&c| (c, None)).collect();

    let mut day_counter: u32 = 0;
    for m in 1..=36u8 {
        for day in 2..=28u8 {
            tick_day(&mut app, 1800, m, day);
            day_counter += 1;
            for &c in &ALL_SIX {
                let after = stockpile_of(&app, c, ResourceType::MilitarySupplies);
                if after < *min_stockpile.get(&c).unwrap() {
                    *min_stockpile.get_mut(&c).unwrap() = after;
                }
                let ids = division_ids_of(&app, c);
                let shortage_today = ids.iter().any(|&id| fulfillment_of(&app, id) < 1.0);
                if shortage_today {
                    *shortage_days.get_mut(&c).unwrap() += 1;
                    first_shortage_day.entry(c).and_modify(|slot| {
                        if slot.is_none() {
                            *slot = Some(day_counter);
                        }
                    });
                }
            }
        }
        tick_month_only(&mut app, 1800, m.wrapping_add(1));
    }

    eprintln!("[P26-004 36-month fixed-division report]");
    for &c in &ALL_SIX {
        let final_value = stockpile_of(&app, c, ResourceType::MilitarySupplies);
        let division_count = division_ids_of(&app, c).len();
        eprintln!(
            "  {c:?}: initial={:.6} min={:.6} shortage_days={} first_shortage_day={:?} \
             final={:.6} division_count={division_count}",
            initial[&c], min_stockpile[&c], shortage_days[&c], first_shortage_day[&c], final_value
        );
        assert!(final_value >= 0.0, "{c:?}: stockpile must never go negative over 36 months");
        // 初期Division数は固定(募兵を一切行っていないため、1個のまま)。
        assert_eq!(division_count, 1, "{c:?}: division count must stay fixed at 1 (no recruitment in this scenario)");
    }
}

/// 36か月、通常AI・募兵を含む実プレイ相当のシナリオで6か国を記録する
/// (`advance_one_month`は`MilitaryPlugin`が登録する月次AI募兵判断
/// [`handle_monthly_ai_recruitment`]・日次自動補充等、既存の全本番Systemをそのまま
/// 稼働させる)。募兵による需要増加・それに伴う不足の発生は、それ自体を自動的に
/// バグと判定しない(spec §15)——原因(division_count増加)と時点(first_shortage_day)
/// を記録するに留める。
#[test]
fn test_36_month_normal_ai_play_real_app_verification() {
    let dir = TempTestDir::new("thirtysix_normal");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());

    let initial: HashMap<CountryId, f64> = ALL_SIX
        .iter()
        .map(|&c| (c, stockpile_of(&app, c, ResourceType::MilitarySupplies)))
        .collect();
    let initial_division_count: HashMap<CountryId, usize> =
        ALL_SIX.iter().map(|&c| (c, division_ids_of(&app, c).len())).collect();

    let mut min_stockpile: HashMap<CountryId, f64> =
        ALL_SIX.iter().map(|&c| (c, f64::MAX)).collect();
    let mut shortage_days: HashMap<CountryId, u32> = ALL_SIX.iter().map(|&c| (c, 0)).collect();
    let mut first_shortage_day: HashMap<CountryId, Option<u32>> =
        ALL_SIX.iter().map(|&c| (c, None)).collect();

    let mut day_counter: u32 = 0;
    for m in 1..=36u8 {
        for day in 2..=28u8 {
            tick_day(&mut app, 1800, m, day);
            day_counter += 1;
            for &c in &ALL_SIX {
                let after = stockpile_of(&app, c, ResourceType::MilitarySupplies);
                if after < *min_stockpile.get(&c).unwrap() {
                    *min_stockpile.get_mut(&c).unwrap() = after;
                }
                let ids = division_ids_of(&app, c);
                let shortage_today = ids.iter().any(|&id| fulfillment_of(&app, id) < 1.0);
                if shortage_today {
                    *shortage_days.get_mut(&c).unwrap() += 1;
                    first_shortage_day.entry(c).and_modify(|slot| {
                        if slot.is_none() {
                            *slot = Some(day_counter);
                        }
                    });
                }
            }
        }
        tick_month_only(&mut app, 1800, m.wrapping_add(1));
    }

    eprintln!("[P26-004 36-month normal-AI report]");
    for &c in &ALL_SIX {
        let final_value = stockpile_of(&app, c, ResourceType::MilitarySupplies);
        let final_division_count = division_ids_of(&app, c).len();
        let equipment = stockpile_of(&app, c, ResourceType::MilitaryEquipment);
        let iron = stockpile_of(&app, c, ResourceType::Iron);
        let coal = stockpile_of(&app, c, ResourceType::Coal);
        let industrial_goods = stockpile_of(&app, c, ResourceType::IndustrialGoods);
        let treasury = app.world().resource::<CountryRegistry>().get(c).unwrap().treasury;
        eprintln!(
            "  {c:?}: initial={:.6} min={:.6} shortage_days={} first_shortage_day={:?} \
             final={:.6} division_count({}->{}) equipment={equipment:.3} iron={iron:.3} \
             coal={coal:.3} industrial_goods={industrial_goods:.3} treasury={treasury:.3}",
            initial[&c],
            min_stockpile[&c],
            shortage_days[&c],
            first_shortage_day[&c],
            final_value,
            initial_division_count[&c],
            final_division_count,
        );
        assert!(final_value >= 0.0, "{c:?}: stockpile must never go negative over 36 months");
        assert!(final_division_count >= 1, "{c:?}: must never lose its only division to a supply-unrelated bug in this scenario");
    }
}
