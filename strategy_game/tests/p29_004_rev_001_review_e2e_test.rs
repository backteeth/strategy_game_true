//! P29-004-REV-001: 生産能力ゲート(E3)の実装レビュー用の実走検証。
//!
//! 標準シナリオ(全 6 か国 AI、1800/01/01〜1850/01/01)を実際の日次・月次システムで進め、
//! (1) 生産能力の再構成値が、同じ月の実際の月次産出と一致すること、
//! (2) AI の注文を「通常許可」と「最小常備戦力免除」に分類した件数・将来需要・生産能力、
//! (3) 能力超過の月を「免除経路による超過」と「許可後の生産低下による超過」に分けた件数、
//! (4) Elfin の月次の備蓄・消費・産出の推移(70 日分に届かない理由の切り分け)
//! を記録する。診断出力(`--nocapture`)と、契約の検査(再構成の一致・分類の整合)を含む。

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
use strategy_game::economy::resources::{ResourceType, military_supplies_value_to_units};
use strategy_game::economy::supply_balance::{
    MilitarySupplyProductionLedger, MonthlyProductionRecord,
};
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
use strategy_game::war::frontline::FrontlineRegistry;
use strategy_game::war::military_ai::MilitaryAiRegistry;

const ELFIN: CountryId = CountryId(1);

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

fn units(v: f64) -> u128 {
    military_supplies_value_to_units(v).unwrap() as u128
}

#[derive(Default, Debug)]
struct OrderStats {
    normal: u32,
    exempt: u32,
    /// (日付, 国, 経路, 将来需要[units/月], 生産能力[units/月], 注文前の生存+訓練中)
    rows: Vec<(String, usize, &'static str, u128, u128, u128)>,
}

#[derive(Default, Debug, Clone)]
struct ElfinMonth {
    date: String,
    production: f64,
    consumption: f64,
    stock_end: f64,
    stock_min: f64,
    divisions: usize,
    daily_demand: f64,
    capacity: f64,
    required: f64,
    cover_days: f64,
    war: bool,
    blocked: bool,
    reason: String,
    iron: f64,
    ig: f64,
    mf_op: f32,
    labor: f32,
    logistics: f32,
    mf_level: u32,
}

#[test]
fn review_reconstruction_orders_and_elfin_trace_over_600_months() {
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p29_004_rev_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);

    let mut orders = OrderStats::default();
    let mut prev_queue: BTreeMap<usize, usize> = BTreeMap::new();
    let mut prev_living: BTreeMap<usize, usize> = BTreeMap::new();
    let mut recon_checked = 0u32;
    let mut recon_mismatch: Vec<String> = Vec::new();
    // 能力超過の月(将来需要 > 能力)。免除経路(生存+訓練中 ≤ 2)と、それを超える師団数に分ける。
    let mut over_exempt = [0u32; 6];
    let mut over_above_min = [0u32; 6];
    let mut months = 0u32;

    // Elfin の月次トレース。
    let mut elfin: Vec<ElfinMonth> = Vec::new();
    let mut cur = ElfinMonth::default();
    let mut first_70_days: Option<String> = None;

    while app.world().resource::<GameDate>().year < 1850 {
        let stock_before = app
            .world()
            .resource::<CountryRegistry>()
            .get(ELFIN)
            .unwrap()
            .stockpile
            .get(ResourceType::MilitarySupplies);
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        let is_month_start = date.day == 1;

        let cr = app.world().resource::<CountryRegistry>();
        let mil = app.world().resource::<MilitaryRegistry>();
        let states = app.world().resource::<StateRegistry>();
        let buildings = app.world().resource::<BuildingRegistry>();
        let ledger = app.world().resource::<MilitarySupplyProductionLedger>();

        // ── 注文の分類と、月初の再構成・能力超過の集計 ──────────────────────
        for c in 0..6usize {
            let id = CountryId(c);
            let country = cr.get(id).unwrap();
            let living = mil
                .divisions
                .values()
                .filter(|d| d.owner == id && d.manpower > 0)
                .count();
            let queue = country.recruitment_queue.len();
            let cap = sustained_monthly_supply(id, states, buildings).monthly_units;
            let demand = projected_daily_demand_units(country, mil).unwrap();
            let required = monthly_units_from_daily(demand).unwrap();

            if queue > *prev_queue.get(&c).unwrap_or(&0) {
                // この日に新しい注文が入った。注文前の師団数は前日の値。
                let before = (*prev_living.get(&c).unwrap_or(&0) + *prev_queue.get(&c).unwrap_or(&0)) as u128;
                let path = if before < MIN_STANDING_DIVISIONS {
                    orders.exempt += 1;
                    "exempt"
                } else {
                    orders.normal += 1;
                    assert!(
                        required <= cap,
                        "{id:?}: 通常経路の注文は能力内のはず(required {required} > cap {cap}) at {}",
                        date.display()
                    );
                    "normal"
                };
                orders.rows.push((date.display(), c, path, required, cap, before));
            }
            prev_queue.insert(c, queue);
            prev_living.insert(c, living);

            if is_month_start {
                if c == 0 {
                    months += 1;
                }
                // 再構成値 = 同じ月の実際の月次産出(同一フレーム、AI の判断時点と同じ状態)。
                if let Some(e) = ledger.get(id)
                    && let MonthlyProductionRecord::Produced(v) = e.record
                {
                    recon_checked += 1;
                    if units(v) != cap {
                        recon_mismatch.push(format!(
                            "{} c{c}: actual {} vs reconstructed {}",
                            date.display(),
                            units(v),
                            cap
                        ));
                    }
                }
                let standing = (living + queue) as u128;
                if required > cap {
                    if standing <= MIN_STANDING_DIVISIONS {
                        over_exempt[c] += 1;
                    } else {
                        over_above_min[c] += 1;
                    }
                }
            }
        }

        // ── Elfin の月次トレース(暦月ごと) ───────────────────────────────
        {
            let country = cr.get(ELFIN).unwrap();
            let stock = country.stockpile.get(ResourceType::MilitarySupplies);
            let production = if is_month_start {
                ledger
                    .get(ELFIN)
                    .and_then(|e| match e.record {
                        MonthlyProductionRecord::Produced(v) => Some(v),
                        _ => None,
                    })
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            if is_month_start {
                // 前月を確定する。
                if !cur.date.is_empty() {
                    elfin.push(cur.clone());
                }
                cur = ElfinMonth {
                    date: date.display(),
                    stock_min: f64::MAX,
                    ..Default::default()
                };
                cur.production = production;
            }
            // その日の実消費 = 産出 − 備蓄の増減(月初は産出が入る日)。
            let consumed = production - (stock - stock_before);
            cur.consumption += consumed.max(0.0);
            cur.stock_end = stock;
            cur.stock_min = cur.stock_min.min(stock);
            let demand_units = country_daily_demand_units(ELFIN, &mil.divisions, &mil.definitions).unwrap();
            cur.daily_demand = demand_units as f64 / 1_000_000.0;
            cur.divisions = mil.divisions.values().filter(|d| d.owner == ELFIN && d.manpower > 0).count();
            cur.capacity = sustained_monthly_supply(ELFIN, states, buildings).monthly_units as f64 / 1_000_000.0;
            cur.required = monthly_units_from_daily(projected_daily_demand_units(country, mil).unwrap()).unwrap() as f64 / 1_000_000.0;
            cur.cover_days = if cur.daily_demand > 0.0 { stock / cur.daily_demand } else { f64::INFINITY };
            let wr = app.world().resource::<WarRegistry>();
            cur.war = wr.wars.values().any(|w| w.status == WarStatus::Active && w.is_participant(ELFIN));
            let fr = app.world().resource::<FrontlineRegistry>();
            cur.blocked = fr.plans.iter().any(|((_, c), p)| *c == ELFIN && p.offensive_supply_blocked);
            cur.reason = app
                .world()
                .resource::<MilitaryAiRegistry>()
                .ai_states
                .get(&ELFIN)
                .map(|s| format!("{:?}", s.last_decision_reason))
                .unwrap_or_default();
            cur.iron = country.stockpile.get(ResourceType::Iron);
            cur.ig = country.stockpile.get(ResourceType::IndustrialGoods);
            if let Some(st) = states.states.iter().find(|s| s.owner_country_id == ELFIN && s.building_level(strategy_game::building::data::BuildingType::MilitaryFactory) > 0) {
                cur.mf_level = st.building_level(strategy_game::building::data::BuildingType::MilitaryFactory);
                cur.mf_op = st.building_operation(strategy_game::building::data::BuildingType::MilitaryFactory);
                cur.labor = st.employed_workforce as f32 / st.total_workforce().max(1) as f32;
                cur.logistics = st.logistics_ratio;
            }
            if cur.war && cur.cover_days >= 70.0 && first_70_days.is_none() {
                first_70_days = Some(date.display());
            }
        }
    }
    if !cur.date.is_empty() {
        elfin.push(cur);
    }

    // ── 出力 ───────────────────────────────────────────────────────────
    println!("MONTHS {months}");
    println!(
        "ORDERS normal={} exempt={} total={}",
        orders.normal,
        orders.exempt,
        orders.normal + orders.exempt
    );
    for r in &orders.rows {
        println!(
            "ORDER {} c{} {} required={:.2} capacity={:.2} standing_before={}",
            r.0, r.1, r.2, r.3 as f64 / 1e6, r.4 as f64 / 1e6, r.5
        );
    }
    println!("OVER_EXEMPT {over_exempt:?}");
    println!("OVER_ABOVE_MIN {over_above_min:?}");
    println!("RECON checked={recon_checked} mismatches={}", recon_mismatch.len());
    for m in recon_mismatch.iter().take(10) {
        println!("RECON_MISMATCH {m}");
    }
    println!("ELFIN_FIRST_WAR_COVER_70 {first_70_days:?}");
    for m in elfin.iter().filter(|m| {
        m.date.as_str() >= "1817/01/01" && m.date.as_str() <= "1819/12/01"
            || m.date.ends_with("/01/01") && m.date.as_str() >= "1820/01/01"
    }) {
        println!(
            "ELFIN {} prod={:.1} cons={:.1} net={:+.1} end={:.1} min={:.1} cover={:.1}d div={} demand={:.1} cap={:.1} req={:.1} war={} blocked={} reason={}",
            m.date, m.production, m.consumption, m.production - m.consumption, m.stock_end,
            m.stock_min, m.cover_days, m.divisions, m.daily_demand, m.capacity, m.required,
            m.war, m.blocked, m.reason
        );
        println!(
            "ELFIN_INPUT {} iron={:.0} ig={:.0} mf_lv={} mf_op={:.2} labor={:.2} logistics={:.2}",
            m.date, m.iron, m.ig, m.mf_level, m.mf_op, m.labor, m.logistics
        );
    }

    {
        let war: Vec<&ElfinMonth> = elfin.iter().filter(|m| m.date.as_str() >= "1817/04/01").collect();
        let n = war.len() as f64;
        let mean_prod = war.iter().map(|m| m.production).sum::<f64>() / n;
        let full = war.iter().filter(|m| m.production >= 273.75).count();
        let zero_stock = war.iter().filter(|m| m.stock_end < 1.0).count();
        let max_cover = war.iter().map(|m| m.cover_days).fold(0.0f64, f64::max);
        let net_year: f64 = war.iter().map(|m| m.production - m.consumption).sum::<f64>();
        println!(
            "ELFIN_SUMMARY months={} mean_prod={:.1} required={:.2} months_prod_ge_required={} months_stock_end_lt1={} max_cover={:.1} sum_net={:.1}",
            war.len(), mean_prod, 273.75, full, zero_stock, max_cover, net_year
        );
    }
    // ── 契約の検査 ─────────────────────────────────────────────────────
    assert!(recon_checked > 1000, "再構成の照合件数: {recon_checked}");
    assert!(
        recon_mismatch.is_empty(),
        "月初の再構成値は同じ月の実産出と一致するはず: {:?}",
        &recon_mismatch[..recon_mismatch.len().min(5)]
    );
    let _ = std::fs::remove_dir_all(dir);
}
