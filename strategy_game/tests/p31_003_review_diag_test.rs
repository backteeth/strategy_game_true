//! P31-003: M1 統合レビュー用の診断(特性化テスト)。標準シナリオ(全6か国AI、1800/01/01 開始)を
//! 実際の日次・月次処理で 1850/01/01 まで進め、財政・領土・戦争・停滞記録の更新を記録する。
//! 状態の直接書換えは行わない。**現状の記録であり、望ましい挙動の固定ではない**。
#![allow(clippy::collapsible_if, clippy::map_entry)]

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
use strategy_game::military::data::MilitaryRegistry;
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

/// 比較・分類用の診断。標準シナリオ(全 AI)を 1850/01/01 まで進め、次を記録する。
/// - 財政: 国別の月次の国庫・税率・収入・支出・収支・州数・師団数(赤字が長い期間の前後)
/// - 領土: 州の所有者の変更(日付・州・旧所有者→新所有者)
/// - 戦争: 宣戦・終結、同じ組の休戦中の宣戦・同時の重複する宣戦の検出
/// - 停滞記録: Passguard 対 Arcadia(1801/04/30)の起点の更新と、その時の支配変更
use std::collections::{BTreeMap, HashMap};
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::state::data::StateRegistry as SR;

fn country_row(app: &App, c: usize) -> String {
    let cr = app.world().resource::<CountryRegistry>();
    let sr = app.world().resource::<SR>();
    let mil = app.world().resource::<MilitaryRegistry>();
    let ct = cr.get(CountryId(c)).unwrap();
    let states = sr.states.iter().filter(|s| s.owner_country_id == CountryId(c)).count();
    let divs = mil
        .divisions
        .values()
        .filter(|d| d.owner == CountryId(c) && d.manpower > 0)
        .count();
    format!(
        "c{c} st={states} div={divs} tr={:.0} tax={:.2} inc={:.1} exp={:.1} bal={:.1}",
        ct.treasury, ct.tax_rate, ct.monthly_income, ct.monthly_expenses, ct.monthly_balance
    )
}

#[test]
fn p31_003_review_standard_600_months() {
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p31_003_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    let mut prev_owner: HashMap<usize, usize> = HashMap::new();
    let mut seen_wars: BTreeMap<usize, (String, Vec<usize>, Vec<usize>)> = BTreeMap::new();
    let mut ended: BTreeMap<usize, String> = BTreeMap::new();
    let mut prev_record: HashMap<usize, String> = HashMap::new();
    let mut prev_controls: HashMap<usize, Vec<(usize, usize, usize)>> = HashMap::new();
    let mut reset_counts: BTreeMap<usize, u32> = BTreeMap::new();
    let mut prev_trace: Option<String> = None;
    let mut contract_violations: Vec<String> = Vec::new();
    let mut class_counts: BTreeMap<String, u32> = BTreeMap::new();
    while app.world().resource::<GameDate>().year < 1850 {
        advance_one_day(&mut app);
        let today = date(&app);
        let (year, month, day) = {
            let d = app.world().resource::<GameDate>();
            (d.year, d.month, d.day)
        };
        // ── 領土の変更 ──
        {
            let sr = app.world().resource::<SR>();
            for s in &sr.states {
                let o = s.owner_country_id.0;
                if let Some(&p) = prev_owner.get(&s.id.0) {
                    if p != o {
                        println!("OWNER {today} state={} {p}->{o}", s.id.0);
                    }
                }
                prev_owner.insert(s.id.0, o);
            }
        }
        // ── 戦争の宣戦・終結・休戦・重複 ──
        {
            let wr = app.world().resource::<WarRegistry>();
            let dip = app.world().resource::<DiplomacyRegistry>();
            for w in wr.wars.values() {
                let key = w.id.0;
                let mut att: Vec<usize> = w.attackers.iter().map(|c| c.0).collect();
                let mut def: Vec<usize> = w.defenders.iter().map(|c| c.0).collect();
                att.sort();
                def.sort();
                if !seen_wars.contains_key(&key) {
                    seen_wars.insert(key, (today.clone(), att.clone(), def.clone()));
                    // 同じ主要国の組の以前の戦争の終結から 1825 日以内の宣戦か。
                    let (pa, pd) = (w.primary_attacker_id(), w.primary_defender_id());
                    let mut verdict = String::from("ok");
                    for o in wr.wars.values() {
                        if o.id == w.id || o.status == WarStatus::Active {
                            continue;
                        }
                        let same = (o.primary_attacker_id() == pa && o.primary_defender_id() == pd)
                            || (o.primary_attacker_id() == pd && o.primary_defender_id() == pa);
                        if same {
                            if let Some(end) = o.end_date.as_deref() {
                                if end <= today.as_str() {
                                    let e = GameDate::from_string(end).unwrap();
                                    let gap = GameDate::from_string(&today).unwrap().days_since(&e);
                                    if gap < 1825 {
                                        verdict = format!("VIOLATION gap={gap} prev_end={end} prev_reason={:?}", o.end_reason);
                                    } else if gap < 1830 {
                                        verdict = format!("ok(truce expired {gap} days after end, prev_reason={:?})", o.end_reason);
                                    }
                                }
                            }
                        }
                    }
                    // 同時に同じ組の Active な戦争が複数あるか。
                    let dup = wr
                        .wars
                        .values()
                        .filter(|o| {
                            o.status == WarStatus::Active
                                && o.id != w.id
                                && o.primary_attacker_id() == pa
                                && o.primary_defender_id() == pd
                        })
                        .count();
                    if verdict.starts_with("VIOLATION") || dup > 0 {
                        contract_violations.push(format!("{today} war={} {verdict} duplicates_active={dup}", w.id.0));
                    }
                    println!(
                        "DECLARE {today} war={} att={att:?} def={def:?} truce_now={} duplicates_active={dup} {verdict}",
                        w.id.0,
                        dip.is_in_truce(pa, pd, &today)
                    );
                }
                if w.status != WarStatus::Active && !ended.contains_key(&key) {
                    ended.insert(key, today.clone());
                    println!("ENDED {today} war={} att={att:?} def={def:?} reason={:?}", w.id.0, w.end_reason);
                }
            }
        }
        // ── 停滞記録の更新(Passguard 対 Arcadia、1801/04/30 宣戦)──
        {
            let wr = app.world().resource::<WarRegistry>();
            let sr = app.world().resource::<SR>();
            for w in wr.wars.values().filter(|w| w.status == WarStatus::Active) {
                let Some(rec) = wr.stalemate_records.get(&w.id) else { continue };
                let last = prev_record.insert(w.id.0, rec.last_progress_date.clone());
                let who: std::collections::HashSet<CountryId> =
                    w.attackers.iter().chain(w.defenders.iter()).copied().collect();
                let mut controls: Vec<(usize, usize, usize)> = sr
                    .states
                    .iter()
                    .filter(|s| {
                        s.owner_country_id != s.controller()
                            && (who.contains(&s.owner_country_id) || who.contains(&s.controller()))
                    })
                    .map(|s| (s.id.0, s.owner_country_id.0, s.controller().0))
                    .collect();
                controls.sort();
                let before = prev_controls.insert(w.id.0, controls.clone());
                if last.is_some() && last.as_deref() != Some(rec.last_progress_date.as_str()) {
                    let empty = Vec::new();
                    let b = before.as_ref().unwrap_or(&empty);
                    let added: Vec<_> = controls.iter().filter(|c| !b.contains(c)).cloned().collect();
                    let removed: Vec<_> = b.iter().filter(|c| !controls.contains(c)).cloned().collect();
                    let is_own = |c: &(usize, usize, usize)| {
                        let (o, k) = (CountryId(c.1), CountryId(c.2));
                        (w.attackers.contains(&o) && w.defenders.contains(&k))
                            || (w.defenders.contains(&o) && w.attackers.contains(&k))
                    };
                    let changed: Vec<_> = added.iter().chain(removed.iter()).cloned().collect();
                    let own = changed.iter().filter(|c| is_own(c)).count();
                    let foreign = changed.len() - own;
                    let kind = match (changed.is_empty(), own > 0, foreign > 0) {
                        (true, _, _) => "battle_only",
                        (false, true, false) => "own_war_control",
                        (false, false, true) => "FOREIGN_control_only",
                        _ => "mixed",
                    };
                    *class_counts.entry(kind.to_string()).or_default() += 1;
                    if kind == "FOREIGN_control_only" || kind == "mixed" {
                        println!("RESET_CLASS {today} war{} att={:?} def={:?} kind={kind} changed={changed:?}", w.id.0, w.attackers.iter().map(|c| c.0).collect::<Vec<_>>(), w.defenders.iter().map(|c| c.0).collect::<Vec<_>>());
                    }
                }
                if last.as_deref() != Some(rec.last_progress_date.as_str()) {
                    *reset_counts.entry(w.id.0).or_default() += 1;
                    if w.start_date == "1801/04/30" || w.start_date == "1801/04/27" || w.start_date == "1801/07/02" {
                        let other_wars: Vec<String> = wr
                            .wars
                            .values()
                            .filter(|o| o.status == WarStatus::Active && o.id != w.id)
                            .map(|o| {
                                format!(
                                    "war{}:{:?}v{:?}",
                                    o.id.0,
                                    o.attackers.iter().map(|c| c.0).collect::<Vec<_>>(),
                                    o.defenders.iter().map(|c| c.0).collect::<Vec<_>>()
                                )
                            })
                            .collect();
                        println!(
                            "RECORD_UPDATE {today} war{} start={} att={:?} def={:?} last_progress {:?} -> {} controls_before={:?} controls_now={controls:?} battles(won_a,won_d,processed)=({},{},{}) other_active_wars={other_wars:?}",
                            w.id.0,
                            w.start_date,
                            w.attackers.iter().map(|c| c.0).collect::<Vec<_>>(),
                            w.defenders.iter().map(|c| c.0).collect::<Vec<_>>(),
                            last,
                            rec.last_progress_date,
                            before,
                            w.won_attacker_battles,
                            w.won_defender_battles,
                            w.processed_battle_ids.len()
                        );
                    }
                }
            }
        }
        // ── 対象外になっている理由の追跡(Passguard 対 Arcadia、1801/04/30 宣戦)──
        {
            use strategy_game::war::stalemate::{is_mutual_stalemate, is_one_side_powerless_and_other_cannot_attack};
            let wr = app.world().resource::<WarRegistry>();
            if let Some(w) = wr.wars.values().find(|w| w.start_date == "1801/04/30" && w.status == WarStatus::Active) {
                let sr = app.world().resource::<SR>();
                let mil = app.world().resource::<MilitaryRegistry>();
                let fr = app.world().resource::<strategy_game::war::frontline::FrontlineRegistry>();
                let ai = app.world().resource::<strategy_game::war::military_ai::MilitaryAiRegistry>();
                let e1 = is_one_side_powerless_and_other_cannot_attack(w, mil, fr, Some(ai));
                let m1 = is_mutual_stalemate(w, sr, mil, fr, Some(ai));
                let live = |c: usize| mil.divisions.values().filter(|d| d.owner == CountryId(c) && d.manpower > 0 && d.status != strategy_game::military::data::DivisionStatus::Destroyed).count();
                let moving = |c: usize| mil.divisions.values().filter(|d| d.owner == CountryId(c) && d.manpower > 0 && matches!(d.status, strategy_game::military::data::DivisionStatus::Moving | strategy_game::military::data::DivisionStatus::Fighting | strategy_game::military::data::DivisionStatus::Retreating | strategy_game::military::data::DivisionStatus::Occupying)).count();
                let reason = |c: usize| ai.ai_states.get(&CountryId(c)).map(|s| format!("{:?}", s.last_decision_reason)).unwrap_or("none".into());
                let plan_info: Vec<String> = fr
                    .frontlines
                    .values()
                    .filter(|f| f.war_id == w.id)
                    .flat_map(|f| {
                        [CountryId(4), CountryId(0)].into_iter().filter_map(move |c| {
                            fr.plans.get(&(f.frontline_id, c)).map(|p| {
                                let (own, enemy) = strategy_game::war::military_ai::evaluate_powers(f, p, c, mil, sr);
                                format!("c{}:{:?} blocked={} own={own} enemy={enemy}", c.0, p.stance, p.offensive_supply_blocked)
                            })
                        })
                    })
                    .collect();
                let sig = format!("e1={e1} m1={m1} live(c4,c0)=({},{}) moving(c4,c0)=({},{}) reason(c4,c0)=({},{}) occupied={} sinceRec={:?} plans={plan_info:?}", live(4), live(0), moving(4), moving(0), reason(4), reason(0), w.occupied_states.len(), wr.stalemate_records.get(&w.id).map(|r| r.last_progress_date.clone()));
                if prev_trace.as_deref() != Some(sig.as_str()) {
                    println!("TRACE4 {today} {sig}");
                    prev_trace = Some(sig);
                }
            }
        }
        // ── 財政(月初)──
        if day == 1 {
            let in_window = (year == 1837 || year == 1838 || (year == 1839 && month <= 10))
                || (year == 1846 && month >= 6)
                || year == 1847
                || (year == 1848 && month <= 12);
            let yearly = month == 1 && year % 10 == 0;
            if in_window || yearly {
                let rows: Vec<String> = [4usize, 5].iter().map(|&c| country_row(&app, c)).collect();
                println!("FIN {today} {}", rows.join(" | "));
            }
        }
    }
    println!("RESET_COUNTS {reset_counts:?}");
    println!("RESET_CLASS_COUNTS {class_counts:?}");
    // 契約: 戦争の増加は、休戦(白紙講和の 1825 日)を無視した宣戦や、同じ組の重複する宣戦ではない。
    assert!(contract_violations.is_empty(), "{contract_violations:?}");
    let _ = std::fs::remove_dir_all(dir);
}
