//! P29-003-DIAG-001: Phase 28 の継続戦争 ISS-1・ISS-2 の原因診断(特性化テスト)。
//!
//! 実際の New Game 初期化(全6か国AI・プレイヤーなし、1800/01/01 開始)から、
//! 実際の日次・月次・AI・戦闘・講和処理を通して 1850/01/01(600 か月)まで進め、
//! 継続している戦争の当事国・宣戦日・補給・需要・判断理由を記録する。
//!
//! **このテストは現状の挙動の記録であり、戦争が継続することを望ましい挙動として固定する
//! ものではない**(診断報告書 p29-003-diag-001 を参照)。状態の直接書換えは行わない。

#![allow(dead_code)]
use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::BTreeMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::CountryId;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::economy::supply_balance::{
    MilitarySupplyProductionLedger, MonthlyProductionRecord,
};
use strategy_game::military::MilitaryPlugin;
use strategy_game::building::data::BuildingRegistry;
use strategy_game::military::production_capacity_gate::{
    MIN_STANDING_DIVISIONS, monthly_units_from_daily, projected_daily_demand_units,
    sustained_monthly_supply,
};
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::military::supply_initialization::country_daily_demand_units;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::SaveFileConfig;
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{WarRegistry, WarStatus};
use strategy_game::war::frontline::FrontlineRegistry;
use strategy_game::war::military_ai::{MilitaryAiDecisionReason, MilitaryAiRegistry};

const DWARF: CountryId = CountryId(2);
const FERROWYN: CountryId = CountryId(5);

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
        let (mut days, mut months) = s.get_mut(app.world_mut()).unwrap();
        days.write(DayChangedMessage {
            year: next.year,
            month: next.month,
            day: next.day,
        });
        if next.month != old_month {
            months.write(MonthChangedMessage {
                year: next.year,
                month: next.month,
            });
        }
    }
    s.apply(app.world_mut());
    app.update();
}

fn date(app: &App) -> String {
    app.world().resource::<GameDate>().display()
}

#[derive(Clone, Debug)]
struct SupplyRow {
    date: String,
    stock: f64,
    demand: f64,
    days: Option<f64>,
    divisions: usize,
    produced: Option<f64>,
}

fn supply_row(app: &App, c: CountryId) -> SupplyRow {
    let mil = app.world().resource::<MilitaryRegistry>();
    let reg = app.world().resource::<CountryRegistry>();
    let stock = reg
        .get(c)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitarySupplies);
    let demand = country_daily_demand_units(c, &mil.divisions, &mil.definitions).unwrap() as f64
        / 1_000_000.0;
    let produced = app
        .world()
        .resource::<MilitarySupplyProductionLedger>()
        .get(c)
        .and_then(|e| match e.record {
            MonthlyProductionRecord::Produced(v) => Some(v),
            MonthlyProductionRecord::Unavailable => None,
        });
    SupplyRow {
        date: date(app),
        stock,
        demand,
        days: if demand > 0.0 { Some(stock / demand) } else { None },
        divisions: mil
            .divisions
            .values()
            .filter(|d| d.owner == c && d.manpower > 0)
            .count(),
        produced,
    }
}

#[derive(Default, Debug)]
struct WarTrace {
    start: String,
    end: Option<String>,
    end_reason: Option<String>,
    attackers: Vec<usize>,
    defenders: Vec<usize>,
    /// 指揮国(主要攻撃国)の判断理由の日数集計。
    reasons: BTreeMap<String, u32>,
    /// 攻勢開始(新規 Offensive)になった日数。
    offensive_days: u32,
    blocked_days: u32,
    days: u32,
    /// 主要攻撃国の備蓄日数が 70 日分以上だった日(戦争中)。
    days_at_or_above_70: Vec<String>,
    max_days_cover: f64,
}

#[test]
#[allow(clippy::type_complexity, clippy::needless_range_loop)]
fn diag_continuing_wars_600_months() {
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p29_003_diag_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    let end = "1850/01/01";
    let mut shortage = [0u32; 6];
    let mut over_capacity_months = [0u32; 6];
    let mut months_observed = 0u32;

    let mut traces: BTreeMap<usize, WarTrace> = BTreeMap::new();
    let mut monthly_dwarf: Vec<SupplyRow> = Vec::new();
    let mut monthly_ferrowyn: Vec<SupplyRow> = Vec::new();
    let mut dwarf_window: Vec<(String, SupplyRow, bool, String, u32)> = Vec::new();
    let mut first_war_date: Option<String> = None;
    let mut feb_1800_dwarf_zero_days = 0u32;

    while date(&app).as_str() < end {
        advance_one_day(&mut app);
        let today = date(&app);
        {
            let mil = app.world().resource::<MilitaryRegistry>();
            for d in mil.divisions.values() {
                if mil.supply_fulfillment_of(d.id) < 0.34 {
                    shortage[d.owner.0] += 1;
                }
            }
        }

        // 2月の初期補給切れ(P29-002-DIAG-001)との関係を見るため、1800年2月の補給 0 の日数を数える。
        if today.starts_with("1800/02") {
            let mil = app.world().resource::<MilitaryRegistry>();
            if mil
                .divisions
                .values()
                .filter(|d| d.owner == DWARF)
                .any(|d| mil.supply_fulfillment_of(d.id) < 0.01)
            {
                feb_1800_dwarf_zero_days += 1;
            }
        }

        // 戦争ごとの追跡。
        let wars: Vec<(usize, WarStatus, Vec<usize>, Vec<usize>, CountryId, String, Option<String>, Option<String>)> = {
            let wr = app.world().resource::<WarRegistry>();
            let mut v: Vec<_> = wr
                .wars
                .values()
                .map(|w| {
                    let mut a: Vec<usize> = w.attackers.iter().map(|c| c.0).collect();
                    let mut d: Vec<usize> = w.defenders.iter().map(|c| c.0).collect();
                    a.sort();
                    d.sort();
                    (
                        w.id.0,
                        w.status,
                        a,
                        d,
                        w.primary_attacker_id(),
                        w.start_date.clone(),
                        w.end_date.clone(),
                        w.end_reason.clone(),
                    )
                })
                .collect();
            v.sort_by_key(|x| x.0);
            v
        };
        for (id, status, a, d, primary, start, end_date, end_reason) in wars {
            if first_war_date.is_none() {
                first_war_date = Some(start.clone());
            }
            let t = traces.entry(id).or_default();
            t.start = start;
            t.attackers = a;
            t.defenders = d;
            if status != WarStatus::Active {
                t.end = end_date;
                t.end_reason = end_reason;
                continue;
            }
            t.days += 1;
            let ai = app.world().resource::<MilitaryAiRegistry>();
            let reason = ai
                .ai_states
                .get(&primary)
                .map(|s| s.last_decision_reason)
                .unwrap_or(MilitaryAiDecisionReason::NoActiveWar);
            *t.reasons.entry(format!("{reason:?}")).or_default() += 1;
            if reason == MilitaryAiDecisionReason::SufficientAdvantage {
                t.offensive_days += 1;
            }
            let fr = app.world().resource::<FrontlineRegistry>();
            if fr
                .plans
                .iter()
                .any(|((_, c), p)| *c == primary && p.offensive_supply_blocked)
            {
                t.blocked_days += 1;
            }
            let row = supply_row(&app, primary);
            if let Some(days) = row.days {
                t.max_days_cover = t.max_days_cover.max(days);
                if days >= 70.0 {
                    t.days_at_or_above_70.push(today.clone());
                }
            }
        }

        // 月初の記録(Dwarf・Ferrowyn)。
        if app.world().resource::<GameDate>().day == 1 {
            months_observed += 1;
            for c in 0..6usize {
                let id = CountryId(c);
                let cr = app.world().resource::<CountryRegistry>();
                let country = cr.get(id).unwrap();
                let mil = app.world().resource::<MilitaryRegistry>();
                let standing = mil.divisions.values().filter(|d| d.owner == id && d.manpower > 0).count() as u128
                    + country.recruitment_queue.len() as u128;
                let demand = projected_daily_demand_units(country, mil).unwrap();
                let required = monthly_units_from_daily(demand).unwrap();
                let cap = sustained_monthly_supply(
                    id,
                    app.world().resource::<strategy_game::state::data::StateRegistry>(),
                    app.world().resource::<BuildingRegistry>(),
                )
                .monthly_units;
                if standing > MIN_STANDING_DIVISIONS && required > cap {
                    over_capacity_months[c] += 1;
                }
            }
            monthly_dwarf.push(supply_row(&app, DWARF));
            monthly_ferrowyn.push(supply_row(&app, FERROWYN));
        }
        // Dwarf の 1808/07/28〜08/05 の日次(評価周期との関係)。
        if today.as_str() >= "1808/07/26" && today.as_str() <= "1808/08/06" {
            let ai = app.world().resource::<MilitaryAiRegistry>();
            let st = ai.ai_states.get(&DWARF);
            let reason = st.map(|s| format!("{:?}", s.last_decision_reason)).unwrap_or_default();
            let eval_day = st.map(|s| s.last_evaluated_day).unwrap_or(0);
            let fr = app.world().resource::<FrontlineRegistry>();
            let blocked = fr
                .plans
                .iter()
                .any(|((_, c), p)| *c == DWARF && p.offensive_supply_blocked);
            dwarf_window.push((today.clone(), supply_row(&app, DWARF), blocked, reason, eval_day));
        }
    }

    // ── ログ出力 ───────────────────────────────────────────────────────
    {
        let cr = app.world().resource::<CountryRegistry>();
        let sr = app.world().resource::<strategy_game::state::data::StateRegistry>();
        for c in 0..6usize {
            let id = CountryId(c);
            let row = supply_row(&app, id);
            let ct = cr.get(id).unwrap();
            let own = sr.states.iter().filter(|s| s.owner_country_id == id).count();
            println!(
                "SUMMARY c{c} states={own} divisions={} queue={} stock={:.0} demand={:.0} produced={:?} treasury={:.0} tax={:.2} shortage_div_days={} over_capacity_months={}/{} active={}",
                row.divisions, ct.recruitment_queue.len(), row.stock, row.demand, row.produced, ct.treasury, ct.tax_rate, shortage[c], over_capacity_months[c], months_observed, ct.is_active()
            );
        }
        let wr = app.world().resource::<WarRegistry>();
        let mut by_reason: BTreeMap<String, u32> = BTreeMap::new();
        for w in wr.wars.values() {
            let k = if w.status == WarStatus::Active { "Active".to_string() } else { w.end_reason.clone().unwrap_or_default() };
            *by_reason.entry(k).or_default() += 1;
        }
        println!("SUMMARY_WARS total={} by_end={:?}", wr.wars.len(), by_reason);
    }
    println!("FIRST_WAR_START {:?}", first_war_date);
    println!("DWARF_FEB_1800_ZERO_SUPPLY_DAYS {feb_1800_dwarf_zero_days}");
    for (id, t) in &traces {
        println!(
            "WAR {id}: {}→{:?} ({:?}) A={:?} D={:?} days_active={} offensive_days={} blocked_days={} max_days_cover={:.1} days>=70={:?} reasons={:?}",
            t.start, t.end, t.end_reason, t.attackers, t.defenders, t.days, t.offensive_days,
            t.blocked_days, t.max_days_cover, t.days_at_or_above_70, t.reasons
        );
    }
    for r in monthly_dwarf
        .iter()
        .filter(|r| r.date.as_str() >= "1808/05/01" && r.date.as_str() <= "1809/04/01")
    {
        println!("DWARF_M {r:?}");
    }
    for (d, r, blocked, reason, eval_day) in &dwarf_window {
        println!("DWARF_D {d} stock={:.1} demand={:.1} days={:?} blocked={blocked} reason={reason} last_eval_day={eval_day}", r.stock, r.demand, r.days);
    }
    for r in monthly_ferrowyn.iter().filter(|r| {
        matches!(
            r.date.as_str(),
            "1809/02/01" | "1814/02/01" | "1814/05/01" | "1815/04/01" | "1819/04/01" | "1820/01/01"
                | "1820/06/01" | "1824/03/01" | "1824/05/01" | "1830/01/01" | "1850/01/01"
        )
    }) {
        println!("FERROWYN_M {r:?}");
    }

    // ── 終了時(1850/01/01)の継続戦争の状態 ────────────────────────────
    {
        let wr = app.world().resource::<WarRegistry>();
        let fr = app.world().resource::<FrontlineRegistry>();
        let sr = app.world().resource::<strategy_game::state::data::StateRegistry>();
        for w in wr.wars.values().filter(|w| w.status == WarStatus::Active) {
            let goal = w.war_goals.first().and_then(|g| g.target_states.first().copied());
            let goal_info = goal.and_then(|s| sr.get(s)).map(|s| (s.id.0, s.owner_country_id.0, s.controller().0));
            println!(
                "END_WAR {} goal(state,owner,controller)={:?} primary_attacker={} primary_defender={} won_att={} won_def={} occupied={}",
                w.id.0, goal_info, w.primary_attacker_id().0, w.primary_defender_id().0,
                w.won_attacker_battles, w.won_defender_battles, w.occupied_states.len()
            );
            for ((fid, c), p) in fr.plans.iter().filter(|((fid, _), _)| fr.frontlines.get(fid).is_some_and(|f| f.war_id == w.id)) {
                println!(
                    "  PLAN frontline={} country={} stance={:?} objective={:?} assigned={} blocked={}",
                    fid.0, c.0, p.stance, p.objective_region_id.map(|s| s.0), p.assigned_division_ids.len(), p.offensive_supply_blocked
                );
            }
            for c in w.attackers.iter().chain(w.defenders.iter()) {
                let row = supply_row(&app, *c);
                println!("  PARTICIPANT c{} stock={:.1} demand={:.1} divisions={} produced={:?}", c.0, row.stock, row.demand, row.divisions, row.produced);
            }
        }
    }

    // ── 特性化の検査 ─────────────────────────────────────────────────────
    // P29-003(E1: 長期停滞した AI 戦争の白紙講和)、P29-004(E3: 生産能力に基づく AI 募兵制限)、
    // P29-005(直近 4 か月の実産出の平均に基づく能力上限)の導入後の世界での記録。診断
    // (P29-003-DIAG-001)が示した構造(攻勢開始が 70 日分のストック判定)は同じまま、募兵と終結
    // 手段が変わることで戦争の経過が変わる。戦争は WarId ではなく、開始日・参加国で照合する。
    // **現状の記録であり、望ましい挙動の固定ではない**。
    let find = |start: &str, a: &[usize], d: &[usize]| -> (usize, &WarTrace) {
        traces
            .iter()
            .find(|(_, t)| t.start == start && t.attackers == a && t.defenders == d)
            .map(|(id, t)| (*id, t))
            .unwrap_or_else(|| panic!("war {start} {a:?} vs {d:?} not found"))
    };
    let still_active: Vec<usize> = {
        let wr = app.world().resource::<WarRegistry>();
        let mut v: Vec<usize> = wr
            .wars
            .values()
            .filter(|w| w.status == WarStatus::Active)
            .map(|w| w.id.0)
            .collect();
        v.sort();
        v
    };

    // ISS-1・ISS-2 の戦争(1806/03/02 Dwarf+Elfin 対 Arcadia、1824/04/02 Ferrowyn 対 Oceanic)は、
    // 募兵が生産能力に制限された世界では発生しない(変更前の世界の戦争で、終結したのではなく
    // 発生しなくなった)。
    assert!(!traces.values().any(|t| t.start == "1806/03/02" && t.defenders == [0]));
    assert!(!traces.values().any(|t| t.start == "1824/04/02"));

    // P31-002(M1: 両陣営に師団がある停滞戦争の合意白紙講和)後の世界の記録。E1(一方が無力)は
    // 引き続き 365 日で、M1(双方武装)は 730 日で、いずれも既存の白紙講和で終結する。
    // 世界は分岐し、この 600 か月では継続する戦争がなくなる(結果であり、合格条件ではない)。
    let m1 = "Mutual Stalemate White Peace";
    let e1 = "Stalemate White Peace";
    // E1: 引き続き機能する(365 日)。
    let (_, w_e1a) = find("1806/06/22", &[5], &[0]);
    assert_eq!(w_e1a.end.as_deref(), Some("1807/06/22"));
    assert_eq!(w_e1a.end_reason.as_deref(), Some(e1));
    let (_, w_e1b) = find("1808/04/27", &[0], &[1]);
    assert_eq!(w_e1b.end.as_deref(), Some("1809/04/27"));
    assert_eq!(w_e1b.end_reason.as_deref(), Some(e1));
    // M1: Dwarf 対 Ferrowyn(1801/07/02、P31-001 の対象戦争)は、診断で予測した 1803/07/02(進展のない
    // 期間 730 日)に終結する。戦闘・攻勢は一度もなかった。
    let (_, w_mutual) = find("1801/07/02", &[2], &[5]);
    assert_eq!(w_mutual.end.as_deref(), Some("1803/07/02"));
    assert_eq!(w_mutual.end_reason.as_deref(), Some(m1));
    assert_eq!(w_mutual.offensive_days, 0);
    assert_eq!(w_mutual.blocked_days, 0);
    // Elfin 対 Arcadia(1801/04/27)も 730 日後に M1 で終結する(変更前は 3,872 日続いて降伏)。
    let (_, w_m1b) = find("1801/04/27", &[1], &[0]);
    assert_eq!(w_m1b.end.as_deref(), Some("1803/04/27"));
    assert_eq!(w_m1b.end_reason.as_deref(), Some(m1));
    // Passguard 対 Arcadia(1801/04/30)は、期間の途中で進展(当事国が関わる別の戦争の支配変更)が
    // あり、730 日より後の 1806/05/03 に M1 で終結する(日付だけを合わせていない: 起点の更新による)。
    let (_, w_m1c) = find("1801/04/30", &[4], &[0]);
    assert_eq!(w_m1c.end.as_deref(), Some("1806/05/03"));
    assert_eq!(w_m1c.end_reason.as_deref(), Some(m1));
    let count = |reason: &str| {
        traces
            .values()
            .filter(|t| t.end_reason.as_deref() == Some(reason))
            .count()
    };
    assert_eq!(count(m1), 3);
    assert_eq!(count(e1), 2);
    assert!(still_active.is_empty(), "継続中の戦争: {still_active:?}");

    // 募兵の抑止(E3): Dwarf は月産が月需要を賄える範囲の師団数。
    let dwarf = supply_row(&app, DWARF);
    assert_eq!(dwarf.divisions, 16);
    assert!(dwarf.produced.unwrap() >= dwarf.demand * 30.4);
    let _ = std::fs::remove_dir_all(dir);
}
