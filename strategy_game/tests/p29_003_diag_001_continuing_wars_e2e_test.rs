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
#[allow(clippy::type_complexity)]
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

    let mut traces: BTreeMap<usize, WarTrace> = BTreeMap::new();
    let mut monthly_dwarf: Vec<SupplyRow> = Vec::new();
    let mut monthly_ferrowyn: Vec<SupplyRow> = Vec::new();
    let mut dwarf_window: Vec<(String, SupplyRow, bool, String, u32)> = Vec::new();
    let mut first_war_date: Option<String> = None;
    let mut feb_1800_dwarf_zero_days = 0u32;

    while date(&app).as_str() < end {
        advance_one_day(&mut app);
        let today = date(&app);

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

    // ── 特性化の検査(現状の記録。望ましい挙動の固定ではない) ─────────────────
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
    assert_eq!(still_active, vec![5, 9], "ISS-1(WarId 5)・ISS-2(WarId 9)が 600 か月時点で継続");
    let w5 = &traces[&5];
    assert_eq!((w5.start.as_str(), &w5.attackers, &w5.defenders), ("1806/03/02", &vec![1, 2], &vec![0]));
    let w9 = &traces[&9];
    assert_eq!((w9.start.as_str(), &w9.attackers, &w9.defenders), ("1824/04/02", &vec![5], &vec![3]));
    // ISS-1(WarId 5): 攻勢は 600 か月で一度も開始されず(判断理由は開戦日の NoValidFrontline を除き
    // SupplyShortage のみ)、国家備蓄が 70 日分以上だったのは 1808/08/01・08/02 の 2 日だけ。
    assert_eq!(w5.offensive_days, 0);
    assert_eq!(w5.reasons.keys().cloned().collect::<Vec<_>>(), vec!["NoValidFrontline", "SupplyShortage"]);
    assert_eq!(w5.days_at_or_above_70, vec!["1808/08/01".to_string(), "1808/08/02".to_string()]);
    // その 2 日間も、攻勢待ち(blocked)のまま判断理由が更新されていない(評価周期 3 日の間に挟まった)。
    for (d, row, blocked, reason, _) in &dwarf_window {
        if d == "1808/08/01" || d == "1808/08/02" {
            assert!(row.days.unwrap() >= 70.0, "{d}");
            assert!(*blocked && reason == "SupplyShortage", "{d}: not re-evaluated while the stock was above 70 days");
        }
    }
    // 備蓄 70 日分のあと、募兵が再開して Division が 11 → 16 に増え、需要が月産を上回った。
    let aug = monthly_dwarf.iter().find(|r| r.date == "1808/08/01").unwrap();
    let feb09 = monthly_dwarf.iter().find(|r| r.date == "1809/02/01").unwrap();
    assert_eq!((aug.divisions, feb09.divisions), (11, 16));
    assert!(feb09.produced.unwrap() < feb09.demand * 30.0, "月産 < 月需要(30 日)");

    // ISS-2(WarId 9): 備蓄は 600 か月を通じて 70 日分に届かない(最大 20 日分未満)。
    assert_eq!(w9.offensive_days, 0);
    assert!(w9.max_days_cover < 20.0, "{}", w9.max_days_cover);
    let f_end = monthly_ferrowyn.last().unwrap();
    assert!(f_end.produced.unwrap() < f_end.demand * 30.0, "月産 < 月需要(30 日)");
    // 平時の募兵: 備蓄が潤沢な 1814〜1819 年に Division が 3 → 8 に増えた。
    let f14 = monthly_ferrowyn.iter().find(|r| r.date == "1814/02/01").unwrap();
    let f19 = monthly_ferrowyn.iter().find(|r| r.date == "1819/04/01").unwrap();
    assert_eq!((f14.divisions, f19.divisions), (3, 8));
    assert!(f14.days.unwrap() > 1000.0);

    // 両戦争とも、防御側は Division 0 で、戦争目標州は未占領(攻撃側の降伏条件が成立しない)。
    // 多国間戦争の支持国(Elfin)には前線計画がなく、独自には攻勢できない。
    {
        let wr = app.world().resource::<WarRegistry>();
        let fr = app.world().resource::<FrontlineRegistry>();
        for id in [5usize, 9] {
            let w = &wr.wars[&strategy_game::common::WarId(id)];
            assert_eq!(w.occupied_states.len(), 0);
            assert_eq!(w.won_attacker_battles + w.won_defender_battles, 0, "戦闘は一度も起きていない");
            assert_eq!(supply_row(&app, w.primary_defender_id()).divisions, 0);
        }
        assert!(
            !fr.plans.keys().any(|(_, c)| *c == CountryId(1)),
            "Elfin(WarId 5 の支持国)には前線計画がない"
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}
