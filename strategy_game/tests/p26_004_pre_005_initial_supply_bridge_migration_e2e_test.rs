//! P26-004-PRE-005: 初期MilitarySuppliesブリッジ備蓄付与と旧Save意味的移行を、実App経由で
//! 検証する。
//!
//! 対象: New Game初期化時の全6か国への一度限りブリッジ付与
//! (`src/military/supply_initialization.rs::grant_initial_military_supplies_bridge`)、
//! 旧Save意味的migration(`apply_military_supplies_bridge_migration`、
//! `src/save/apply.rs::prepare_load`から呼ばれる)、二重付与防止、save/load分岐比較、
//! New Game/Loaded Gameの論理的一致、カレンダー境界(月初・月途中・月末・最短/最長月)、
//! 消滅国家・Divisionなし国家の扱い。
//!
//! `verification_logs/phase-26/p26-004-pre-003/
//! p26-004-pre-003_minimum_supply_chains_and_initialization_design.md`§20〜27・
//! `verification_logs/phase-26/p26-004-pre-004/`が確立した設計・データを踏襲する
//! (実データ・実App、診断用の無限資金フィクスチャは使用しない)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::{RunSystemOnce, SystemState};
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::CountryId;
use strategy_game::country::{CountryPlugin, CountryRegistry, CountryStatus, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::military::supply_initialization::grant_initial_military_supplies_bridge;
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
// App-level fixtures (P26-004-PRE-004と同じ構成方針)
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
        "strategy_game_p26_004_pre_005_e2e_{label}_{}_{nanos}_{n}",
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

/// P26-004-PRE-002〜004と同じ簡略暦(1か月=27日[day2〜28]+月境界tick)で、
/// `current_month_index`か月分(1始まり)を1か月進める。実際のカレンダー(31日等)は
/// `src/military/supply_initialization.rs`のunit testで別途検証する
/// (このE2Eテストの目的は、生産System・save/loadパイプラインとの実際の相互作用の
/// 検証であり、暦の日数そのものの検証ではない)。
fn advance_one_month(app: &mut App, current_month_index: u8) {
    for day in 2..=28u8 {
        tick_day(app, 1800, current_month_index, day);
    }
    tick_month(app, 1800, current_month_index.wrapping_add(1));
}

fn stockpile_of(app: &App, country_id: CountryId, resource: ResourceType) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(resource)
}

fn marker_of(app: &App, country_id: CountryId) -> bool {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .military_supplies_initial_grant_applied
}

fn set_military_supplies_stockpile(app: &mut App, country_id: CountryId, value: f64) {
    let mut registry = app.world_mut().resource_mut::<CountryRegistry>();
    registry
        .get_mut(country_id)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitarySupplies, value);
}

/// P26-004: 全Divisionを取り除く(需要0、日次消費が常に0になる)。本タスクが新たに
/// 導入した日次消費が、P26-004-PRE-005自身の関心事(月次生産と初期grantの関係)とは
/// 無関係な交絡因子として混入するのを避けるため、Category B(月次生産の二重計上なし)
/// の比較シナリオでのみ使う。
fn clear_all_divisions(app: &mut App) {
    app.world_mut().resource_mut::<MilitaryRegistry>().divisions.clear();
}

/// セーブファイルを`SaveGameV1`として読み込み、渡した関数で編集した上で書き戻す
/// (P26-004-PRE-004と同じ手法: 実際のRONラウンドトリップを通した「合成された旧Save」を
/// 構築する。実ユーザーセーブ[`saves/savegame_v1.ron`]は一切使用しない)。
fn rewrite_save_file(path: &std::path::Path, edit: impl FnOnce(&mut SaveGameV1)) {
    let contents = std::fs::read_to_string(path).expect("read save file");
    let mut save: SaveGameV1 = ron::from_str(&contents).expect("parse SaveGameV1");
    edit(&mut save);
    let ron_string = ron::ser::to_string_pretty(&save, ron::ser::PrettyConfig::default())
        .expect("serialize SaveGameV1");
    std::fs::write(path, ron_string).expect("write save file");
}

/// New Game→即save→「旧Save化」編集(対象国家のマーカーをfalseへ戻し、指定した
/// MilitarySupplies備蓄・カレンダー日付へ書き換える)→loadという手順で、旧Save
/// migrationシナリオを実App経由で構築する。他の5か国はNew Game時点の状態
/// (マーカーtrue・31.0)のまま変更しない。
fn old_save_scenario(
    label: &str,
    day: u8,
    month: u8,
    target_country: CountryId,
    target_stockpile: f64,
) -> (TempTestDir, App) {
    let dir = TempTestDir::new(label);
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    send_save_request(&mut app);
    rewrite_save_file(&dir.final_path(), |save| {
        save.date.day = day;
        save.date.month = month;
        for country in save.countries.iter_mut() {
            if country.id == target_country {
                country.military_supplies_initial_grant_applied = false;
                country
                    .stockpile
                    .amounts
                    .insert(ResourceType::MilitarySupplies, target_stockpile);
            }
        }
    });
    send_load_request(&mut app);
    (dir, app)
}

// ─────────────────────────────────────────────────────────────────────────
// Category A: New Game初期化
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_a1_all_six_countries_get_exactly_31_and_marker_true() {
    let dir = TempTestDir::new("a1");
    let app = setup_app_in_playing(DWARF, dir.save_file_config());

    for cid in ALL_SIX {
        let value = stockpile_of(&app, cid, ResourceType::MilitarySupplies);
        assert_eq!(value, 31.0, "{cid:?} must have exactly 31.0 MilitarySupplies at New Game");
        assert!(marker_of(&app, cid), "{cid:?} marker must be true after New Game grant");
    }
}

/// P26-004本体(`military::daily_supply`)が日次消費を実際に接続したため、日次Tick
/// だけではMilitarySuppliesが変化しない、という本テストの元の主旨(P26-004-PRE-005
/// 完了時点)はもう成り立たない――これは退行ではなく、P26-004が意図して追加した
/// 接続そのものである。代わりに、31.0のブリッジ備蓄が、まさにこの日次消費を
/// 初回月次生産までちょうど賄うために設計された値であることを、実際に日を追って
/// 検証する: New Gameの満編制Infantry1個師団の日次需要は1.0/日(P26-004-PRE-005
/// §14で導出済み)であり、day2〜28(27回)の日次ティックで31.0から27.0が消費され、
/// 常に不足なく1.0/日が充足される(初回月次生産[月2]より前の1日分の予備を含む
/// 31.0という設計どおり、day28終了時点でもちょうど4.0が残っている)。
#[test]
fn test_a2_stockpile_decreases_by_exactly_the_known_daily_demand_before_first_monthly_production() {
    let dir = TempTestDir::new("a2");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let before = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);
    assert_eq!(before, 31.0);
    for day in 2..=28u8 {
        tick_day(&mut app, 1800, 1, day);
        let expected = before - (day as f64 - 1.0);
        assert_eq!(
            stockpile_of(&app, DWARF, ResourceType::MilitarySupplies),
            expected,
            "stockpile must decrease by exactly 1.0/day (the single Infantry division's daily \
             demand, P26-004) through day {day} of month 1"
        );
    }
    // day28終了時点で27日分(day2..=28)の消費を終え、31.0 - 27.0 = 4.0(=予備1日分+
    // 月末残り0日分)が残る――P26-004-PRE-005が31.0を導出した根拠(30イベント+1日
    // 予備)と整合する。
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 4.0);
}

#[test]
fn test_a3_reinvoking_the_grant_system_does_not_double_grant() {
    let dir = TempTestDir::new("a3");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    for cid in ALL_SIX {
        assert_eq!(stockpile_of(&app, cid, ResourceType::MilitarySupplies), 31.0);
    }

    // 同じSystemを実App上でもう一度直接実行する(PlayingEntryModeは
    // reset_playing_entry_modeにより既にNewGameへ戻っているため、entry_modeの
    // ガードではなく国家単位のマーカーだけが二重付与を防いでいることを検証する)。
    app.world_mut()
        .run_system_once(grant_initial_military_supplies_bridge)
        .expect("re-running the same system must not error");

    for cid in ALL_SIX {
        assert_eq!(
            stockpile_of(&app, cid, ResourceType::MilitarySupplies),
            31.0,
            "{cid:?}: re-invoking the grant system must not grant a second time"
        );
        assert!(marker_of(&app, cid));
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Category B: 月次生産との相互作用(二重計上なし)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_b1_first_monthly_production_adds_normally_without_double_counting_the_bridge() {
    // 「初期31.0付与あり」の通常経路と、「同じ状態から初期付与だけを人為的に
    // ゼロへ戻した」比較経路を、同じ月数分だけ進める。両者の最終差分が常に
    // ちょうど31.0であれば、(1)月次生産量は初期付与の有無に左右されず、
    // (2)初期付与が生産として二重計上されていないことが実証される。
    //
    // P26-004: 日次消費が実際に接続されたため、Divisionが存在すると
    // `consumed = min(demand, stockpile)`という非線形なclampが働き、「without_grant」
    // 側(stockpile=0スタート)は"with_grant"側より少なく消費されることがあるため、
    // 単純な固定オフセット比較が成り立たなくなる(本タスクの関心事[月次生産と
    // 初期grantの二重計上なし]とは無関係な交絡)。両アプリからDivisionを取り除き
    // (需要0=日次消費0)、この交絡を排除した上で、月次生産だけの比較を維持する。
    let dir_a = TempTestDir::new("b1_with_grant");
    let mut app_with_grant = setup_app_in_playing(DWARF, dir_a.save_file_config());
    clear_all_divisions(&mut app_with_grant);

    let dir_b = TempTestDir::new("b1_without_grant");
    let mut app_without_grant = setup_app_in_playing(DWARF, dir_b.save_file_config());
    clear_all_divisions(&mut app_without_grant);
    for cid in ALL_SIX {
        set_military_supplies_stockpile(&mut app_without_grant, cid, 0.0);
    }

    for m in 1..=6u8 {
        advance_one_month(&mut app_with_grant, m);
        advance_one_month(&mut app_without_grant, m);
    }

    for cid in ALL_SIX {
        let with_grant = stockpile_of(&app_with_grant, cid, ResourceType::MilitarySupplies);
        let without_grant = stockpile_of(&app_without_grant, cid, ResourceType::MilitarySupplies);
        assert!(
            (with_grant - without_grant - 31.0).abs() < 1e-6,
            "{cid:?}: with-grant trajectory must differ from the ungranted baseline by \
             exactly the flat 31.0 bridge (with={with_grant}, without={without_grant})"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Category C: 旧Save意味的migration・カレンダー境界
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_c1_old_save_month_start_zero_stockpile_gets_topped_up_to_31() {
    let (_dir, app) = old_save_scenario("c1", 1, 1, DWARF, 0.0);
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 31.0);
    assert!(marker_of(&app, DWARF));
}

#[test]
fn test_c2_old_save_partial_stockpile_gets_topped_up_not_replaced() {
    let (_dir, app) = old_save_scenario("c2", 1, 1, DWARF, 10.0);
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 31.0);
    assert!(marker_of(&app, DWARF));
}

#[test]
fn test_c3_old_save_stockpile_already_at_requirement_grants_zero() {
    let (_dir, app) = old_save_scenario("c3", 1, 1, DWARF, 31.0);
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 31.0);
    assert!(marker_of(&app, DWARF));
}

#[test]
fn test_c4_old_save_stockpile_above_requirement_is_never_reduced() {
    let (_dir, app) = old_save_scenario("c4", 1, 1, DWARF, 50.0);
    assert_eq!(
        stockpile_of(&app, DWARF, ResourceType::MilitarySupplies),
        50.0,
        "existing stockpile above the requirement must never be decreased on load"
    );
    assert!(marker_of(&app, DWARF));
}

#[test]
fn test_c5_old_save_mid_month_tops_up_the_remaining_days_worth() {
    // 1月20日(31日の月): events=31-20=11、required=(11+1)=12.0、current=5.0 -> grant=7.0。
    let (_dir, app) = old_save_scenario("c5", 20, 1, DWARF, 5.0);
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 12.0);
    assert!(marker_of(&app, DWARF));
}

#[test]
fn test_c6_old_save_last_day_of_month_only_one_day_of_reserve() {
    // 1月31日(月末): events=0、required=1.0。
    let (_dir, app) = old_save_scenario("c6", 31, 1, DWARF, 0.0);
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 1.0);
    assert!(marker_of(&app, DWARF));
}

#[test]
fn test_c7_old_save_shortest_month_february_start() {
    // 2月1日(28日の月、最短月): events=28-1=27、required=28.0。
    let (_dir, app) = old_save_scenario("c7", 1, 2, DWARF, 0.0);
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 28.0);
}

#[test]
fn test_c7_old_save_longest_month_january_start() {
    // 1月1日(31日の月、最長月): events=31-1=30、required=31.0(New Gameと同一)。
    let (_dir, app) = old_save_scenario("c7b", 1, 1, DWARF, 0.0);
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 31.0);
}

#[test]
fn test_c7_old_save_thirty_day_month_start() {
    // 4月1日(30日の月): events=30-1=29、required=30.0。
    let (_dir, app) = old_save_scenario("c7c", 1, 4, DWARF, 0.0);
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 30.0);
}

#[test]
fn test_c8_eliminated_country_gets_no_grant_but_marker_becomes_true() {
    let dir = TempTestDir::new("c8");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    send_save_request(&mut app);
    rewrite_save_file(&dir.final_path(), |save| {
        // P21-021の`CountryStatus`不変条件(`save::validate`)を満たすため、
        // Eliminatedへ遷移させる国家は陸州・Divisionを一切所有しない状態に
        // しておく必要がある(さもなければ save::validate がこの合成Saveそのものを
        // 拒否し、テストの意図(消滅国家の扱い)を検証できなくなる)。
        for state in save.states.iter_mut() {
            if state.owner_country_id == ELFIN {
                state.owner_country_id = ARCADIA;
            }
        }
        save.military.divisions.retain(|_, division| division.owner != ELFIN);
        for country in save.countries.iter_mut() {
            if country.id == ELFIN {
                country.military_supplies_initial_grant_applied = false;
                country
                    .stockpile
                    .amounts
                    .insert(ResourceType::MilitarySupplies, 5.0);
                country.status = CountryStatus::Eliminated {
                    eliminated_date: "1800/06/01".to_string(),
                    annexed_by: ARCADIA,
                };
            }
        }
    });
    send_load_request(&mut app);

    assert_eq!(
        stockpile_of(&app, ELFIN, ResourceType::MilitarySupplies),
        5.0,
        "an eliminated country's existing stockpile must not be topped up"
    );
    assert!(marker_of(&app, ELFIN), "an eliminated country's marker must still become true");
}

#[test]
fn test_v1_syntax_compatibility_no_new_save_format_version() {
    // Save format versionを変更していないこと、V2を追加していないことの実証。
    let dir = TempTestDir::new("v1_compat");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    send_save_request(&mut app);
    let contents = std::fs::read_to_string(dir.final_path()).expect("read save file");
    let save: SaveGameV1 = ron::from_str(&contents).expect("parse SaveGameV1");
    assert_eq!(save.version, strategy_game::save::dto::SAVE_FORMAT_VERSION_V1);
    assert_eq!(strategy_game::save::dto::SAVE_FORMAT_VERSION_V1, 1);
}

// ─────────────────────────────────────────────────────────────────────────
// Category D: 冪等性(save/loadの繰り返し)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_d1_save_load_twice_in_a_row_is_stable() {
    let dir = TempTestDir::new("d1");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 31.0);

    send_save_request(&mut app);
    send_load_request(&mut app);
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 31.0);
    assert!(marker_of(&app, DWARF));

    send_save_request(&mut app);
    send_load_request(&mut app);
    assert_eq!(
        stockpile_of(&app, DWARF, ResourceType::MilitarySupplies),
        31.0,
        "a second save/load round trip must not grant again"
    );
    assert!(marker_of(&app, DWARF));
}

#[test]
fn test_d2_old_save_migration_then_resave_then_reload_never_double_grants() {
    // 旧Save(マーカーfalse・備蓄0)をロードしてmigrationを起こした後、その状態を
    // 再保存し、再ロードしても二重付与されないことを確認する(spec §11「save→load後に
    // 再付与されない」「save→loadを2回繰り返しても安定」)。
    let (dir, mut app) = old_save_scenario("d2", 1, 1, DWARF, 0.0);
    assert_eq!(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies), 31.0);
    assert!(marker_of(&app, DWARF));

    send_save_request(&mut app);
    send_load_request(&mut app);
    assert_eq!(
        stockpile_of(&app, DWARF, ResourceType::MilitarySupplies),
        31.0,
        "re-saving and re-loading the already-migrated save must not grant again"
    );
    assert!(marker_of(&app, DWARF));
    drop(dir);
}

#[test]
fn test_d3_month_crossing_save_load_is_stable() {
    let dir = TempTestDir::new("d3");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    advance_one_month(&mut app, 1);
    let before = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);
    // P26-004: 日次消費が実際に接続されたため、月1のブリッジ備蓄(31.0)は27日分の
    // 日次需要(1.0/日)でday28終了時点で4.0まで減り、その後の月2生産が加わる――
    // 「31.0を決して下回らない」という以前の前提はもう成り立たない。ここでは
    // stockpileが決して負にならないという、より基本的な不変条件だけを確認する
    // (本テストの主旨は次の行の「月境界を跨いだsave/loadが値を完全保持する」こと)。
    assert!(before >= 0.0, "MilitarySupplies stockpile must never go negative: {before}");

    send_save_request(&mut app);
    send_load_request(&mut app);
    let after = stockpile_of(&app, DWARF, ResourceType::MilitarySupplies);
    assert_eq!(after, before, "a save/load crossing a month boundary must preserve the value exactly");
    assert!(marker_of(&app, DWARF));
}

// ─────────────────────────────────────────────────────────────────────────
// Category E: New Game / Loaded Game 論理的一致
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_e1_new_game_and_migrated_old_save_agree_on_the_same_logical_state() {
    // New Gameの正規付与値(31.0)と、同じDivision構成・同じ日付(1800/01/01)から
    // migration経由で導出した値が完全に一致することを確認する。
    let dir_new = TempTestDir::new("e1_new");
    let app_new = setup_app_in_playing(DWARF, dir_new.save_file_config());
    let new_game_value = stockpile_of(&app_new, DWARF, ResourceType::MilitarySupplies);

    let (_dir_old, app_old) = old_save_scenario("e1_old", 1, 1, DWARF, 0.0);
    let migrated_value = stockpile_of(&app_old, DWARF, ResourceType::MilitarySupplies);

    assert_eq!(new_game_value, migrated_value);
    assert_eq!(new_game_value, 31.0);
}

#[test]
fn test_e2_active_country_without_any_division_gets_zero_and_marker_true_via_migration() {
    // Divisionを持たない有効国家(意図的に全Divisionを除去した合成旧Save)は、
    // 必要量0・付与0・マーカーtrueとなる(spec §8.3)。
    let dir = TempTestDir::new("e2");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    send_save_request(&mut app);
    rewrite_save_file(&dir.final_path(), |save| {
        save.military
            .divisions
            .retain(|_, division| division.owner != ELFIN);
        for country in save.countries.iter_mut() {
            if country.id == ELFIN {
                country.military_supplies_initial_grant_applied = false;
                country.stockpile.amounts.insert(ResourceType::MilitarySupplies, 0.0);
            }
        }
    });
    send_load_request(&mut app);

    assert_eq!(stockpile_of(&app, ELFIN, ResourceType::MilitarySupplies), 0.0);
    assert!(marker_of(&app, ELFIN));
}

// ─────────────────────────────────────────────────────────────────────────
// Category F: 世界保存・決定性回帰保護
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_f1_world_total_after_new_game_grant_equals_sum_of_six_grants() {
    let dir = TempTestDir::new("f1");
    let app = setup_app_in_playing(DWARF, dir.save_file_config());
    let total: f64 = ALL_SIX
        .iter()
        .map(|&cid| stockpile_of(&app, cid, ResourceType::MilitarySupplies))
        .sum();
    assert_eq!(total, 31.0 * 6.0);
}

/// P26-DET-001-REV-001が確定したArcadia正規値の保護(初期31.0付与を含む本タスク後も
/// 決定的生産順序修正が壊れていないことを確認する)。この報告書の値は「初期付与を
/// 含まない月次生産の累積」を表すため、本タスクで付与される31.0はこの比較には
/// 含めない(31.0を含めた比較にすると既存の決定性テストの意味を変えてしまうため、
/// 生の生産累積値だけを取り出して比較する: 実測値から31.0を引いた値を正規値と
/// 突き合わせる)。
#[test]
fn test_f2_arcadia_deterministic_production_values_are_protected_after_the_initial_grant() {
    const ARCADIA_CANONICAL_M12: f64 = 421.2265230000;
    const ARCADIA_CANONICAL_M18: f64 = 592.5978000000;

    let dir = TempTestDir::new("f2");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let initial_grant = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
    assert_eq!(initial_grant, 31.0);

    // P26-004: 日次消費(実測)も加え戻して"生産分のみ"を分離する(初期grantを引く
    // だけでは、日次消費も混入したままになるため、spec §20と同じ理由)。
    let mut consumed = 0.0;
    let mut m12 = 0.0;
    let mut m18 = 0.0;
    for m in 1..=18u8 {
        for day in 2..=28u8 {
            let before = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
            tick_day(&mut app, 1800, m, day);
            let after = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies);
            consumed += (before - after).max(0.0);
        }
        tick_month(&mut app, 1800, m.wrapping_add(1));
        let v = stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies) - initial_grant
            + consumed;
        match m {
            12 => m12 = v,
            18 => m18 = v,
            _ => {}
        }
    }

    assert!((m12 - ARCADIA_CANONICAL_M12).abs() < 1e-6, "month12 production only: {m12}");
    assert!((m18 - ARCADIA_CANONICAL_M18).abs() < 1e-6, "month18 production only: {m18}");
}
