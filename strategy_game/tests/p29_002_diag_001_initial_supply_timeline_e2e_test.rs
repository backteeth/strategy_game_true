//! P29-002-DIAG-001: Dwarf Federation の New Game における、軍用補給物資(MilitarySupplies)の
//! 初回・2回目の月次生産の診断(特性化テスト)。
//!
//! 観測(P29-002-VER-001): 2月1日の月次生産実績 0.0・備蓄 0、3月の月次生産実績 47.1。
//!
//! このテストは実際の New Game 初期化(`OnEnter(Playing)`)から、実際の日次→月次の
//! メッセージ順(`app::time::advance_game_date`と同じ: 日次メッセージ、月が変わる日は
//! 続けて月次メッセージ)で 1800/03/02 まで進め、月次生産の入力・稼働率・実産出・備蓄を
//! 記録する。状態の直接書換えは行わない。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::BuildingType;
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::economy::supply_balance::{
    MilitarySupplyProductionLedger, MonthlyProductionRecord,
};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::SaveFileConfig;
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::war::WarPlugin;

const DWARF: CountryId = CountryId(2);
/// Southern Industry(Factory Lv3 + MilitaryFactory Lv2、Dwarf の唯一の軍需工場)。
const SOUTHERN_INDUSTRY: StateId = StateId(6);

fn setup_dwarf_new_game() -> (App, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p29_002_diag_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
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
    app.insert_resource(PlayerCountry(Some(DWARF)));
    app.insert_state(GameState::Playing);
    app.update();
    (app, dir)
}

// 診断ログ(`println!`のDebug出力)でだけ読むフィールドがある。
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct Snapshot {
    ms: f64,
    me: f64,
    iron: f64,
    coal: f64,
    ig: f64,
    mf_level: u32,
    factory_level: u32,
    mf_op: Option<f32>,
    factory_op: Option<f32>,
    employed: u64,
    total_workforce: u64,
    logistics: f32,
    division_demand: f64,
    min_fulfillment: f64,
    ledger: Option<(i32, u8, Option<f64>)>,
}

fn snapshot(app: &App) -> Snapshot {
    let c = app
        .world()
        .resource::<CountryRegistry>()
        .get(DWARF)
        .unwrap()
        .stockpile
        .clone();
    let st = app
        .world()
        .resource::<StateRegistry>()
        .get(SOUTHERN_INDUSTRY)
        .unwrap();
    let mil = app.world().resource::<MilitaryRegistry>();
    let owned: Vec<_> = mil.divisions.values().filter(|d| d.owner == DWARF).collect();
    let min_fulfillment = owned
        .iter()
        .map(|d| mil.supply_fulfillment_of(d.id))
        .fold(1.0_f64, f64::min);
    let demand = strategy_game::military::supply_initialization::country_daily_demand_units(
        DWARF,
        &mil.divisions,
        &mil.definitions,
    )
    .unwrap() as f64
        / 1_000_000.0;
    let ledger = app
        .world()
        .resource::<MilitarySupplyProductionLedger>()
        .get(DWARF)
        .map(|e| {
            (
                e.year,
                e.month,
                match e.record {
                    MonthlyProductionRecord::Produced(v) => Some(v),
                    MonthlyProductionRecord::Unavailable => None,
                },
            )
        });
    Snapshot {
        ms: c.get(ResourceType::MilitarySupplies),
        me: c.get(ResourceType::MilitaryEquipment),
        iron: c.get(ResourceType::Iron),
        coal: c.get(ResourceType::Coal),
        ig: c.get(ResourceType::IndustrialGoods),
        mf_level: st.building_level(BuildingType::MilitaryFactory),
        factory_level: st.building_level(BuildingType::Factory),
        mf_op: st
            .building_operation_ratios
            .get(&BuildingType::MilitaryFactory)
            .copied(),
        factory_op: st
            .building_operation_ratios
            .get(&BuildingType::Factory)
            .copied(),
        employed: st.employed_workforce,
        total_workforce: st.total_workforce(),
        logistics: st.logistics_ratio,
        division_demand: demand,
        min_fulfillment,
        ledger,
    }
}

/// `advance_game_date`と同じ順序で 1 日進める(日次メッセージ → 月が変わる日は月次メッセージ)。
fn advance_one_day(app: &mut App, year: i32, month: u8, day: u8) {
    {
        let mut date = app.world_mut().resource_mut::<GameDate>();
        date.year = year;
        date.month = month;
        date.day = day;
    }
    let mut state: SystemState<(
        MessageWriter<DayChangedMessage>,
        MessageWriter<MonthChangedMessage>,
    )> = SystemState::new(app.world_mut());
    {
        let (mut days, mut months) = state.get_mut(app.world_mut()).unwrap();
        days.write(DayChangedMessage { year, month, day });
        if day == 1 {
            months.write(MonthChangedMessage { year, month });
        }
    }
    state.apply(app.world_mut());
    app.update();
}

fn days_in_month(month: u8) -> u8 {
    [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][(month - 1) as usize]
}

#[test]
fn diag_dwarf_first_two_monthly_productions() {
    let (mut app, dir) = setup_dwarf_new_game();
    let start = snapshot(&app);
    println!("START 1800/01/01: {start:?}");

    let mut at: std::collections::BTreeMap<(u8, u8), Snapshot> = Default::default();
    let mut before_month: std::collections::BTreeMap<(u8, u8), Snapshot> = Default::default();
    for month in 1..=3u8 {
        for day in 1..=days_in_month(month) {
            if month == 1 && day == 1 {
                continue; // 開始日(New Gameの初期状態)
            }
            if day == 1 {
                before_month.insert((month, day), snapshot(&app));
            }
            advance_one_day(&mut app, 1800, month, day);
            if day == 1 || day == 2 || day == days_in_month(month) {
                at.insert((month, day), snapshot(&app));
            }
            if month == 3 && day == 2 {
                break;
            }
        }
    }
    for (k, v) in &before_month {
        println!("BEFORE {:02}/{:02} tick: {v:?}", k.0, k.1);
    }
    for (k, v) in &at {
        println!("AFTER  {:02}/{:02}: {v:?}", k.0, k.1);
    }

    // ── 初期状態(New Game、通常の初期データ) ─────────────────────────────
    assert_eq!(start.ms, 31.0, "初期ブリッジ備蓄 = 次回月次生産までの需要 31日分");
    assert_eq!(start.division_demand, 1.0, "初期師団 1 個 × 1.0/日");
    assert_eq!(start.ig, 0.0, "IndustrialGoods の初期備蓄は 0(countries.ron の Dwarf は Iron・Coal のみ)");
    assert_eq!((start.mf_level, start.factory_level), (2, 3));
    assert_eq!(start.employed, 0, "New Game 直後は月次雇用計算の前(就業者 0)");

    // ── 2月1日の月次処理(初回): MilitaryFactory は IG 入力が 0 で稼働率 0 ──────
    let feb = &at[&(2, 1)];
    assert_eq!(before_month[&(2, 1)].ig, 0.0, "月初の IG 備蓄は 0(入力充足率は月初備蓄で決まる)");
    assert_eq!(feb.mf_op, Some(0.0), "MilitaryFactory の稼働率 = IG 充足率 0");
    assert_eq!(feb.ledger, Some((1800, 2, Some(0.0))), "実産出 0.0(未集計ではなく実績 0)");
    let factory_op = feb.factory_op.expect("Factory は稼働している");
    assert!(
        (factory_op - 20.0 / 85.0).abs() < 1e-6,
        "Factory は物流比(容量 20 / 使用量 85)で稼働: {factory_op}"
    );
    assert!(
        (feb.ig - 30.0 * 3.0 * factory_op as f64).abs() < 1e-3,
        "同じ月次処理で Factory が IG を産出(MilitaryFactory の入力には間に合わない): {}",
        feb.ig
    );
    assert_eq!(feb.ms, 0.0, "備蓄 31 は 1/日 × 31日(1/02〜2/01)で枯渇");
    // 2月中(2/02〜2/28)は備蓄 0 のため、Division の補給充足率は 0。
    assert_eq!(at[&(2, 2)].min_fulfillment, 0.0);
    assert_eq!(at[&(2, 28)].min_fulfillment, 0.0);
    assert_eq!(at[&(2, 28)].ms, 0.0);

    // ── 3月1日の月次処理(2回目): IG が確保済みで MilitaryFactory が稼働 ──────
    let mar = &at[&(3, 1)];
    assert_eq!(before_month[&(3, 1)].ig.to_bits(), feb.ig.to_bits());
    let expected = 100.0 * 2.0 * (20.0_f64 / 85.0);
    assert_eq!(mar.mf_op, Some(20.0 / 85.0), "稼働率 = 物流比(労働力比 0.274・入力充足は不足なし)");
    let (y, m, produced) = mar.ledger.expect("ledger");
    assert_eq!((y, m), (1800, 3));
    let produced = produced.expect("実績");
    assert!((produced - expected).abs() < 1e-3, "{produced} vs {expected}");
    assert!((produced - 47.1).abs() < 0.05, "手動観測 47.1 と一致: {produced}");
    // 実産出と備蓄の関係: 3/01 の備蓄 = 実産出 − 当日の日次消費 1.0。3/02 はさらに −1.0。
    assert!((mar.ms - (produced - 1.0)).abs() < 1e-3, "3/01: {}", mar.ms);
    let mar2 = &at[&(3, 2)];
    assert!((mar2.ms - (produced - 2.0)).abs() < 1e-3, "3/02: {}", mar2.ms);
    assert!((mar2.ms - 45.1).abs() < 0.05, "手動観測 45.1 と一致");
    assert_eq!(mar2.ledger, mar.ledger, "実績は日次消費で変わらない");
    assert_eq!(mar.min_fulfillment, 1.0, "3月は備蓄があり充足率 1.0");
    let _ = std::fs::remove_dir_all(dir);
}
