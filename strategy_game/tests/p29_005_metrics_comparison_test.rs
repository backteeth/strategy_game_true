//! P29-005: 変更前後の比較用に、標準全 AI シナリオ(1800/01/01〜1850/01/01、600 か月)の
//! 指標を出力する。変更前の HEAD(P29-004-REV-001 時点)でも、同じファイルでそのまま
//! 実行できる(P29-005 で追加した API を使わない)。診断出力(`--nocapture`)が目的で、
//! 検査は保存則(備蓄が非負)・全期間の完走・国家存続の記録だけに留める
//! (国家存続や戦争件数が望ましい値かどうかは、この検査の対象ではない)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::BTreeMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::BuildingRegistry;
use strategy_game::common::CountryId;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::military::production_capacity_gate::{
    MIN_STANDING_DIVISIONS, monthly_units_from_daily, projected_daily_demand_units,
    sustained_monthly_supply,
};
use strategy_game::military::supply_initialization::country_daily_demand_units;
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
use strategy_game::war::data::{WarRegistry, WarStatus};

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
    app.insert_resource(PlayerCountry(None));
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

#[derive(Default, Clone)]
struct PerCountry {
    normal_orders: u32,
    exempt_orders: u32,
    over_exempt: u32,
    over_above_min: u32,
    shortage_months: u32,
    offense_ready_war_months: u32,
    war_months: u32,
    sum_production_units: u128,
    sum_consumption_units: i128,
    first_order: Option<String>,
}

#[test]
fn p29_005_metrics_over_600_months() {
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p29_005_cmp_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    let mut m: Vec<PerCountry> = vec![PerCountry::default(); 6];
    let mut prev_queue = [0usize; 6];
    let mut prev_standing = [0u128; 6];
    let mut months = 0u32;

    while app.world().resource::<GameDate>().year < 1850 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        let cr = app.world().resource::<CountryRegistry>();
        let mil = app.world().resource::<MilitaryRegistry>();
        let states = app.world().resource::<StateRegistry>();
        let buildings = app.world().resource::<BuildingRegistry>();
        let wars = app.world().resource::<WarRegistry>();
        for (c, row) in m.iter_mut().enumerate() {
            let id = CountryId(c);
            let Some(country) = cr.get(id) else { continue };
            let living = mil
                .divisions
                .values()
                .filter(|d| d.owner == id && d.manpower > 0)
                .count();
            let queue = country.recruitment_queue.len();
            if queue > prev_queue[c] {
                if prev_standing[c] < MIN_STANDING_DIVISIONS {
                    row.exempt_orders += 1;
                } else {
                    row.normal_orders += 1;
                }
                row.first_order.get_or_insert(date.display());
            }
            prev_queue[c] = queue;
            prev_standing[c] = (living + queue) as u128;

            if date.day == 1 {
                if c == 0 {
                    months += 1;
                }
                let cap = sustained_monthly_supply(id, states, buildings).monthly_units;
                let demand = projected_daily_demand_units(country, mil).unwrap_or(0);
                let required = monthly_units_from_daily(demand).unwrap_or(0);
                if required > cap {
                    if (living + queue) as u128 <= MIN_STANDING_DIVISIONS {
                        row.over_exempt += 1;
                    } else {
                        row.over_above_min += 1;
                    }
                }
                let stock = country.stockpile.get(ResourceType::MilitarySupplies);
                let daily = country_daily_demand_units(id, &mil.divisions, &mil.definitions)
                    .unwrap_or(0) as f64
                    / 1e6;
                if daily > 0.0 && stock < daily {
                    row.shortage_months += 1;
                }
                let at_war = wars
                    .wars
                    .values()
                    .any(|w| w.status == WarStatus::Active && w.is_participant(id));
                if at_war {
                    row.war_months += 1;
                    if daily > 0.0 && stock >= 70.0 * daily {
                        row.offense_ready_war_months += 1;
                    }
                }
                row.sum_production_units += cap;
                row.sum_consumption_units += (daily * 30.4166 * 1e6) as i128;
            }
        }
    }

    println!("CMP months={months}");
    let cr = app.world().resource::<CountryRegistry>();
    let mil = app.world().resource::<MilitaryRegistry>();
    let states = app.world().resource::<StateRegistry>();
    for (c, row) in m.iter().enumerate() {
        let id = CountryId(c);
        let country = cr.get(id).unwrap();
        let living = mil
            .divisions
            .values()
            .filter(|d| d.owner == id && d.manpower > 0)
            .count();
        let owned = states.states.iter().filter(|s| s.owner_country_id == id).count();
        let stock = country.stockpile.get(ResourceType::MilitarySupplies);
        assert!(stock >= 0.0);
        println!(
            "CMP c{c} active={} divs={living} states={owned} stock={stock:.1} treasury={:.1} tax={:.3} \
             orders normal={} exempt={} first_order={:?} over_exempt={} over_above_min={} \
             shortage_months={} war_months={} offense_ready_war_months={}",
            country.is_active(),
            country.treasury,
            country.tax_rate,
            row.normal_orders,
            row.exempt_orders,
            row.first_order,
            row.over_exempt,
            row.over_above_min,
            row.shortage_months,
            row.war_months,
            row.offense_ready_war_months,
        );
    }
    let wars = app.world().resource::<WarRegistry>();
    let mut lines: BTreeMap<String, String> = BTreeMap::new();
    for (id, w) in &wars.wars {
        let mut a: Vec<_> = w.attackers.iter().map(|c| c.0).collect();
        let mut d: Vec<_> = w.defenders.iter().map(|c| c.0).collect();
        a.sort();
        d.sort();
        lines.insert(
            format!("{}-{:?}-{:?}", w.start_date, a, d),
            format!(
                "WAR id={} att={a:?} def={d:?} start={} end={:?} status={:?} reason={:?} goals={:?}",
                id.0, w.start_date, w.end_date, w.status, w.end_reason, w.war_goals
            ),
        );
    }
    for l in lines.values() {
        println!("CMP {l}");
    }
}
