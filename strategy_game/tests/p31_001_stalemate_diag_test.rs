//! P31-001: 双方に師団がある停滞戦争(戦力拮抗)の診断(特性化テスト)。
//!
//! 現在の標準シナリオ(全6か国AI、1800/01/01 開始)から、実際の日次・月次処理を通して進め、
//! 当事国・宣戦日から照合した戦争について、軍事的進展・双方の攻勢判断・師団・戦力比・
//! 既存の和平判定の結果を記録する。状態の直接書換えは行わない。
//! **現状の記録であり、望ましい挙動の固定ではない**(仕様書 p31-001 を参照)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::CountryId;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::{DivisionStatus, MilitaryRegistry};
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
use strategy_game::war::military_ai::{MilitaryAiRegistry, evaluate_division_power, evaluate_powers};
use strategy_game::war::peace::{PeaceOffer, PeaceTerm, can_accept_peace_offer};

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

/// 標準シナリオで Dwarf(攻撃側)対 Ferrowyn(防御側)の、1801/07/02 に宣戦された戦争を、
/// WarId ではなく当事国・宣戦日で見つける。
fn find_target_war(app: &App) -> Option<strategy_game::common::WarId> {
    let wr = app.world().resource::<WarRegistry>();
    wr.wars
        .values()
        .find(|w| {
            w.start_date == "1801/07/02"
                && w.attackers.len() == 1
                && w.attackers.contains(&DWARF)
                && w.defenders.len() == 1
                && w.defenders.contains(&FERROWYN)
        })
        .map(|w| w.id)
}

type DivRow = (usize, u64, f32, DivisionStatus, usize, u64);

fn living(app: &App, c: CountryId) -> Vec<DivRow> {
    let mil = app.world().resource::<MilitaryRegistry>();
    let mut v: Vec<_> = mil
        .divisions
        .values()
        .filter(|d| d.owner == c && d.manpower > 0 && d.status != DivisionStatus::Destroyed)
        .map(|d| {
            (
                d.id.0,
                d.manpower,
                d.organization,
                d.status,
                d.current_state.0,
                evaluate_division_power(d),
            )
        })
        .collect();
    v.sort_by_key(|r| r.0);
    v
}

/// 仕様 M1 の「両陣営の攻勢不能・領土の係争なし」条件を、公開データから評価する制御診断
/// (本番コードではない。仕様の条件が実際の戦争で成立することを確かめるためのもの)。
fn m1_predicate(app: &App, id: strategy_game::common::WarId) -> bool {
    let wr = app.world().resource::<WarRegistry>();
    let w = &wr.wars[&id];
    let mil = app.world().resource::<MilitaryRegistry>();
    let fr = app.world().resource::<FrontlineRegistry>();
    let ai = app.world().resource::<MilitaryAiRegistry>();
    let sr = app.world().resource::<strategy_game::state::data::StateRegistry>();
    let sides = [&w.attackers, &w.defenders];
    let armed = |side: &std::collections::HashSet<CountryId>| {
        mil.divisions.values().any(|d| {
            side.contains(&d.owner) && d.manpower > 0 && d.status != DivisionStatus::Destroyed
        })
    };
    if !(armed(sides[0]) && armed(sides[1])) {
        return false;
    }
    let moving = mil.divisions.values().any(|d| {
        d.manpower > 0
            && (sides[0].contains(&d.owner) || sides[1].contains(&d.owner))
            && matches!(
                d.status,
                DivisionStatus::Moving
                    | DivisionStatus::Fighting
                    | DivisionStatus::Retreating
                    | DivisionStatus::Occupying
            )
    });
    if moving {
        return false;
    }
    let fid = fr.get_frontline_for_war(id).map(|f| f.frontline_id);
    for side in sides {
        for c in side.iter() {
            if let Some(f) = fid
                && fr
                    .plans
                    .get(&(f, *c))
                    .is_some_and(|p| p.stance == strategy_game::war::frontline::FrontlineStance::Offensive)
            {
                return false;
            }
            let Some(s) = ai.ai_states.get(c) else { return false };
            use strategy_game::war::military_ai::MilitaryAiDecisionReason as R;
            if s.last_decision_reason == R::SufficientAdvantage {
                return false;
            }
            if s.last_decision_reason == R::SupplyShortage
                && (s.estimated_own_power as u128) * 1000 >= (s.estimated_enemy_power as u128) * 1100
            {
                return false;
            }
        }
    }
    // 領土の係争がない(参加国が関わる所有者≠支配者の州がなく、占領もない)。
    let disputed = sr.states.iter().any(|s| {
        s.owner_country_id != s.controller()
            && (w.attackers.contains(&s.owner_country_id)
                || w.defenders.contains(&s.owner_country_id)
                || w.attackers.contains(&s.controller())
                || w.defenders.contains(&s.controller()))
    });
    !disputed && w.occupied_states.is_empty()
}

#[test]
fn diag_even_force_stalemate_war() {
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p31_001_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    let mut war_id = None;
    let mut last_battles = 0usize;
    let mut last_controls = String::new();
    let mut last_change_day = String::new();
    let mut last_pred = false;
    let mut first_m1_ready: Option<String> = None;
    while app.world().resource::<GameDate>().year < 1806 {
        advance_one_day(&mut app);
        if war_id.is_none() {
            war_id = find_target_war(&app);
            if war_id.is_some() {
                println!("TARGET_WAR_FOUND {} id={:?}", date(&app), war_id);
            }
        }
        let Some(id) = war_id else { continue };
        let today = date(&app);
        let wr = app.world().resource::<WarRegistry>();
        let w = &wr.wars[&id];
        if w.status != WarStatus::Active {
            println!("TARGET_WAR_ENDED {today} {:?} {:?}", w.status, w.end_reason);
            break;
        }
        let sr = app.world().resource::<strategy_game::state::data::StateRegistry>();
        let controls: String = sr
            .states
            .iter()
            .filter(|s| s.owner_country_id != s.controller())
            .map(|s| format!("{}:{}->{}", s.id.0, s.owner_country_id.0, s.controller().0))
            .collect::<Vec<_>>()
            .join(",");
        let battles = w.processed_battle_ids.len();
        if battles != last_battles || controls != last_controls {
            last_battles = battles;
            last_controls = controls.clone();
            last_change_day = today.clone();
            println!(
                "PROGRESS {today} battles={battles} won_att={} won_def={} controls=[{controls}]",
                w.won_attacker_battles, w.won_defender_battles
            );
        }
        let day = app.world().resource::<GameDate>().day;
        let month = app.world().resource::<GameDate>().month;
        {
            let pred = m1_predicate(&app, id);
            if pred != last_pred {
                println!("M1_PREDICATE {today} -> {pred}");
                last_pred = pred;
            }
            if pred && first_m1_ready.is_none() {
                let since = wr
                    .stalemate_records
                    .get(&id)
                    .and_then(|r| GameDate::from_string(&r.last_progress_date))
                    .map(|d| app.world().resource::<GameDate>().days_since(&d))
                    .unwrap_or(0);
                if since >= 730 {
                    first_m1_ready = Some(today.clone());
                    println!("M1_READY {today} days_without_progress={since}");
                }
            }
        }
        if day == 1 && (month == 1 || month == 7) {
            let ai = app.world().resource::<MilitaryAiRegistry>();
            let fr = app.world().resource::<FrontlineRegistry>();
            let reasons: Vec<String> = [DWARF, FERROWYN]
                .iter()
                .map(|c| {
                    ai.ai_states
                        .get(c)
                        .map(|s| {
                            format!(
                                "c{}:{:?} own={} enemy={}",
                                c.0, s.last_decision_reason, s.estimated_own_power, s.estimated_enemy_power
                            )
                        })
                        .unwrap_or_default()
                })
                .collect();
            let plans: Vec<String> = fr
                .plans
                .iter()
                .filter(|((fid, _), _)| fr.frontlines.get(fid).is_some_and(|f| f.war_id == id))
                .map(|((_, c), p)| {
                    format!(
                        "c{}:{:?} assigned={} blocked={}",
                        c.0,
                        p.stance,
                        p.assigned_division_ids.len(),
                        p.offensive_supply_blocked
                    )
                })
                .collect();
            let rec = wr.stalemate_records.get(&id).map(|r| r.last_progress_date.clone());
            println!(
                "MONTH {today} dwarf={} ferro={} {reasons:?} {plans:?} score={} exh=({},{}) e1_since={rec:?}",
                living(&app, DWARF).len(),
                living(&app, FERROWYN).len(),
                w.war_score,
                w.attacker_war_exhaustion,
                w.defender_war_exhaustion
            );
        }
        if today == "1802/08/01" {
            println!("DETAIL_DWARF {:?}", living(&app, DWARF));
            println!("DETAIL_FERRO {:?}", living(&app, FERROWYN));
            let fr = app.world().resource::<FrontlineRegistry>();
            let mil = app.world().resource::<MilitaryRegistry>();
            for c in [DWARF, FERROWYN] {
                for row in living(&app, c) {
                    let st = sr.get(strategy_game::common::StateId(row.4));
                    println!(
                        "DIV c{} id={} state={} owner={:?} sea={:?} in_frontline={:?}",
                        c.0,
                        row.0,
                        row.4,
                        st.map(|s| s.owner_country_id.0),
                        st.map(|s| s.is_sea),
                        fr.division_frontline_map.get(&strategy_game::common::DivisionId(row.0))
                    );
                }
            }
            if let Some(f) = fr.frontlines.values().find(|f| f.war_id == id) {
                // 未割当の Dwarf 師団(州 8)から、前線地域への経路が存在するか。
                let own_side: Vec<CountryId> = w.own_side_of(DWARF);
                let enemy_side: Vec<CountryId> = w.opponents_of(DWARF);
                for &fr_region in &f.attacker_front_regions {
                    let p = strategy_game::military::pathfinding::find_path(
                        strategy_game::common::StateId(8),
                        fr_region,
                        sr,
                        &own_side,
                        &enemy_side,
                    );
                    println!("PATH_FROM_8 to {} = {:?}", fr_region.0, p.map(|v| v.len()));
                }
                let s8 = sr.get(strategy_game::common::StateId(8));
                println!("STATE8 {:?}", s8.map(|s| (s.owner_country_id.0, s.controller().0)));
                println!(
                    "FRONTLINE pairs={:?} att_regions={:?} def_regions={:?}",
                    f.border_region_pairs, f.attacker_front_regions, f.defender_front_regions
                );
                for c in [DWARF, FERROWYN] {
                    if let Some(p) = fr.plans.get(&(f.frontline_id, c)) {
                        let (own, enemy) = evaluate_powers(f, p, c, mil, sr);
                        println!(
                            "POWER c{} own={own} enemy={enemy} ratio={:.3} stance={:?} objective={:?}",
                            c.0,
                            own as f64 / (enemy.max(1)) as f64,
                            p.stance,
                            p.objective_region_id.map(|s| s.0)
                        );
                    }
                }
            }
            let cr = app.world().resource::<CountryRegistry>();
            for (proposer, recipient) in [(DWARF, FERROWYN), (FERROWYN, DWARF)] {
                let offer = PeaceOffer {
                    war_id: id,
                    proposer_country_id: proposer,
                    recipient_country_id: recipient,
                    term: PeaceTerm::WhitePeace,
                    created_date: today.clone(),
                };
                let r = can_accept_peace_offer(&offer, wr, Some(sr), Some(mil), Some(cr), &today);
                println!("WHITE_PEACE_CHECK c{}->c{} {:?}", proposer.0, recipient.0, r);
            }
            let goal = w.war_goals.first().and_then(|g| g.target_states.first().copied());
            println!(
                "WAR goal={:?} occupied={} score={} start={} last_progress={last_change_day}",
                goal.map(|s| s.0),
                w.occupied_states.len(),
                w.war_score,
                w.start_date
            );
        }
    }
    println!("END {} id={:?}", date(&app), war_id);
    assert!(war_id.is_some(), "対象戦争が標準シナリオで宣戦される");
    // P31-002(M1)の導入後: 診断の予測(M1 の条件が進展のない期間 730 日に達する 1803/07/02)どおりに、
    // 対象戦争がその日に合意白紙講和で終結する(診断の制御ログは、終結した日のループで止まる)。
    // 戦闘・支配変更の進展は一度もなかった。
    let id = war_id.unwrap();
    {
        let w = &app.world().resource::<WarRegistry>().wars[&id];
        assert_eq!(w.status, WarStatus::WhitePeace);
        assert_eq!(w.end_date.as_deref(), Some("1803/07/02"));
        assert_eq!(w.end_reason.as_deref(), Some("Mutual Stalemate White Peace"));
    }
    assert!(last_change_day.is_empty(), "戦闘・支配変更の進展が一度もない");
}
