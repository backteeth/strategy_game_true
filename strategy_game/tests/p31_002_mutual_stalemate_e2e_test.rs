//! P31-002: 双方武装停滞戦争の合意白紙講和(M1)を、実際の New Game 初期化・日次システム・
//! 実際の `handle_daily_war_resolution`・Save/Load を通して検証する(E1 の p29_003 e2e と同じ構成)。
//!
//! 制御 fixture: 実マップで Dwarf(c2)が Arcadia(c0)の州2を目標に宣戦した AI 戦争を 1 件追加し、
//! 両国に同じ戦力の師団を 2 個ずつ与えて毎日その戦力を保つ(戦力比 1.0 で、攻勢の開始条件に届かない)。
//! 判定ロジックの個別条件は `war::stalemate::tests`(単体)が網羅する。
//! 自然運用の 600 か月・1950 年の比較は報告書と `p29_011_long_horizon_audit_test` を参照。


use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, StateId, WarId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::diplomacy::crisis::{WarGoal, WarGoalType};
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::MilitaryRegistry;
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
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::FrontlineRegistry;

const DWARF: CountryId = CountryId(2);
const ARCADIA: CountryId = CountryId(0);
/// Save は Playing 状態ではプレイヤー国家を要求するため、Save/Load の検証では、戦争に無関係な
/// 国(Passguard)をプレイヤーにする(戦争の当事国はいずれも AI)。
const BYSTANDER: CountryId = CountryId(4);
const GOAL_STATE: StateId = StateId(2); // Western Mage Province(Arcadia 所有)
const OTHER_ARCADIA_STATE: StateId = StateId(1); // Northern Frontier(Arcadia 所有、戦争目標ではない)

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "strategy_game_p31_002_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )))
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_app(dir: &std::path::Path, player: Option<CountryId>) -> App {
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
    app.insert_resource(PlayerCountry(player));
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

fn date(app: &App) -> String {
    app.world().resource::<GameDate>().display()
}
fn send_save(app: &mut App) {
    let mut s: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut()).unwrap().write(SaveRequestMessage);
    s.apply(app.world_mut());
    app.update();
}

fn send_load(app: &mut App) {
    let mut s: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut()).unwrap().write(LoadRequestMessage);
    s.apply(app.world_mut());
    app.update();
    app.update();
}

use strategy_game::common::{DivisionDefinitionId, DivisionId};
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::military::data::{Division, DivisionSize, DivisionStatus, DivisionType};

fn fixture_division(def: DivisionDefinitionId, owner: CountryId, at: StateId) -> Division {
    Division {
        id: DivisionId(0), // add_division が上書きする
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: at,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 10_000,
        max_manpower: 10_000,
        equipment: 1000.0,
        max_equipment: 1000.0,
        organization: 50.0,
        max_organization: 50.0,
        morale: 50.0,
        max_morale: 50.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: def,
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

/// 制御 fixture: Dwarf と Arcadia に、同じ戦力の師団を 2 個ずつ与える(戦力比 1.0 = 攻勢の開始条件
/// 1.10 に届かない)。自然運用では AI が募兵・攻勢するため、双方武装の停滞を決定論的に作るための操作。
fn add_even_war(app: &mut App) -> (WarId, Vec<DivisionId>) {
    let (def, caps) = {
        let mr = app.world().resource::<MilitaryRegistry>();
        let def = mr
            .divisions
            .values()
            .next()
            .map(|d| d.def_id)
            .unwrap_or(DivisionDefinitionId(0));
        let cr = app.world().resource::<CountryRegistry>();
        (
            def,
            [
                (DWARF, cr.get(DWARF).unwrap().capital_state_id),
                (ARCADIA, cr.get(ARCADIA).unwrap().capital_state_id),
            ],
        )
    };
    let mut ids = Vec::new();
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        let old: Vec<_> = mr
            .divisions
            .values()
            .filter(|d| d.owner == DWARF || d.owner == ARCADIA)
            .map(|d| d.id)
            .collect();
        for id in old {
            mr.remove_division(id);
        }
        for (owner, cap) in caps {
            for _ in 0..2 {
                ids.push(mr.add_division(fixture_division(def, owner, cap)));
            }
        }
    }
    let war = War {
        id: WarId(0), // add_war が上書きする
        name: "P31-002 mutual stalemate fixture".to_string(),
        attackers: [DWARF].into_iter().collect(),
        defenders: [ARCADIA].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![WarGoal {
            attacker: DWARF,
            defender: ARCADIA,
            goal_type: WarGoalType::ConquerState,
            target_states: vec![GOAL_STATE],
            base_peace_cost: 20.0,
            international_concern: 10.0,
            completion: 0.0,
            is_primary: true,
        }],
        start_date: "1799/12/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: HashSet::new(),
    };
    let id = app.world_mut().resource_mut::<WarRegistry>().add_war(war);
    (id, ids)
}

/// 毎日: fixture 以外の師団を取り除き、募兵・動員可能人員を止める(戦力比を保つ)。
fn maintain(app: &mut App, keep: &[DivisionId]) {
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        let extra: Vec<_> = mr
            .divisions
            .values()
            .filter(|d| (d.owner == DWARF || d.owner == ARCADIA) && !keep.contains(&d.id))
            .map(|d| d.id)
            .collect();
        for id in extra {
            mr.remove_division(id);
        }
    }
    let mut cr = app.world_mut().resource_mut::<CountryRegistry>();
    for c in [DWARF, ARCADIA] {
        let country = cr.get_mut(c).unwrap();
        country.available_manpower = 0;
        country.recruitment_queue.clear();
    }
}

fn status(app: &App, id: WarId) -> WarStatus {
    app.world().resource::<WarRegistry>().wars[&id].status
}

fn run_until(app: &mut App, keep: &[DivisionId], until: &str) {
    while date(app).as_str() < until {
        maintain(app, keep);
        advance_one_day(app);
    }
}

/// 戦争が終結するまで(上限日まで)進め、終結直前に観測した停滞期間の起点を返す。
fn run_until_ended(app: &mut App, id: WarId, keep: &[DivisionId], cap: &str) -> String {
    let mut last_progress = String::new();
    while date(app).as_str() < cap {
        if status(app, id) != WarStatus::Active {
            break;
        }
        if let Some(r) = app.world().resource::<WarRegistry>().stalemate_records.get(&id) {
            last_progress = r.last_progress_date.clone();
        }
        maintain(app, keep);
        advance_one_day(app);
    }
    assert_ne!(status(app, id), WarStatus::Active, "上限日までに終結する");
    last_progress
}

fn plus_period(d: &str) -> String {
    GameDate::from_string(d).unwrap().add_days(730).display()
}

fn war_end(app: &App, id: WarId) -> (WarStatus, Option<String>, Option<String>) {
    let w = &app.world().resource::<WarRegistry>().wars[&id];
    (w.status, w.end_date.clone(), w.end_reason.clone())
}

const CAP: &str = "1812/01/01";

#[test]
fn an_armed_even_stalemate_between_ai_countries_ends_in_a_mutual_white_peace_after_730_days_only() {
    let dir = TempDir::new("basic");
    let mut app = setup_app(&dir.0, None);
    let (id, keep) = add_even_war(&mut app);
    let last_progress = run_until_ended(&mut app, id, &keep, CAP);
    let (st, end_date, reason) = war_end(&app, id);
    assert_ne!(st, WarStatus::Active);
    assert_eq!(reason.as_deref(), Some("Mutual Stalemate White Peace"));
    // 最後の進展の 730 日後(それ以前には終結しない)。
    let end_date = end_date.unwrap();
    assert_eq!(end_date, plus_period(&last_progress), "最後の進展の 730 日後");
    println!("M1_E2E last_progress={last_progress} end={end_date}");
    // 白紙講和: 領土移転なし(目標州は Arcadia のまま)、戦争状態・前線・停滞記録が整理される。
    let sr = app.world().resource::<StateRegistry>();
    assert_eq!(sr.get(GOAL_STATE).unwrap().owner_country_id, ARCADIA);
    assert_eq!(sr.get(GOAL_STATE).unwrap().controller(), ARCADIA);
    let wr = app.world().resource::<WarRegistry>();
    assert!(!wr.wars_for_country(DWARF).contains(&id));
    assert!(!wr.wars_for_country(ARCADIA).contains(&id));
    assert!(!wr.stalemate_records.contains_key(&id));
    assert!(app.world().resource::<FrontlineRegistry>().get_frontline_for_war(id).is_none());
    // 5 年(1825 日)の休戦が設定され、再宣戦できない。
    assert!(
        app.world()
            .resource::<DiplomacyRegistry>()
            .is_in_truce(DWARF, ARCADIA, &end_date)
    );
    // 両国は師団を失わない(終結は戦力の破棄ではない)。
    let mr = app.world().resource::<MilitaryRegistry>();
    for d in &keep {
        assert!(mr.divisions.get(d).is_some_and(|x| x.manpower > 0));
    }
    // 二重に終結しない(以後も終結情報が変わらない)。
    let again = GameDate::from_string(&end_date).unwrap().add_days(120).display();
    run_until(&mut app, &keep, &again);
    assert_eq!(war_end(&app, id).1.as_deref(), Some(end_date.as_str()));
    assert_eq!(war_end(&app, id).2.as_deref(), Some("Mutual Stalemate White Peace"));
}

#[test]
fn a_war_with_a_player_participant_is_never_ended_by_m1() {
    let dir = TempDir::new("player");
    let mut app = setup_app(&dir.0, Some(DWARF));
    let (id, keep) = add_even_war(&mut app);
    run_until(&mut app, &keep, "1803/06/01");
    assert_eq!(status(&app, id), WarStatus::Active);
    assert!(!app.world().resource::<WarRegistry>().stalemate_records.contains_key(&id));
}

#[test]
fn an_occupation_keeps_the_war_out_of_m1_even_after_the_period() {
    let dir = TempDir::new("occupation");
    let mut app = setup_app(&dir.0, None);
    let (id, keep) = add_even_war(&mut app);
    run_until(&mut app, &keep, "1800/03/01");
    // Dwarf が Arcadia の州(戦争目標ではない)を占領している状態。
    app.world_mut()
        .resource_mut::<StateRegistry>()
        .get_mut(OTHER_ARCADIA_STATE)
        .unwrap()
        .controller_country = Some(DWARF);
    run_until(&mut app, &keep, "1803/06/01");
    assert_eq!(status(&app, id), WarStatus::Active, "占領がある戦争は M1 の対象外");
}

#[test]
fn the_period_survives_save_and_load_and_an_old_save_without_it_restarts_counting() {
    // 連続実行での終結日。
    let dir = TempDir::new("continuous");
    let mut cont = setup_app(&dir.0, Some(BYSTANDER));
    let (id, keep) = add_even_war(&mut cont);
    run_until_ended(&mut cont, id, &keep, CAP);
    let continuous_end = war_end(&cont, id).1.expect("ended");

    // 途中(1800/07/01)で Save → Load しても、終結日は連続実行と同じ。軍事 AI の判断記録も復元される。
    let dir2 = TempDir::new("saveload");
    let mut app = setup_app(&dir2.0, Some(BYSTANDER));
    let (id2, keep2) = add_even_war(&mut app);
    run_until(&mut app, &keep2, "1800/07/01");
    let saved_progress = app.world().resource::<WarRegistry>().stalemate_records[&id2]
        .last_progress_date
        .clone();
    send_save(&mut app);
    run_until(&mut app, &keep2, "1800/08/15");
    send_load(&mut app);
    assert_eq!(date(&app), "1800/07/01");
    assert_eq!(
        app.world().resource::<WarRegistry>().stalemate_records[&id2].last_progress_date,
        saved_progress,
        "停滞期間の起点が Save から復元される"
    );
    {
        use strategy_game::war::military_ai::{MilitaryAiDecisionReason, MilitaryAiRegistry};
        let ai = app.world().resource::<MilitaryAiRegistry>();
        for c in [DWARF, ARCADIA] {
            assert!(
                ai.ai_states
                    .get(&c)
                    .is_some_and(|s| s.last_decision_reason != MilitaryAiDecisionReason::NoActiveWar),
                "軍事 AI の判断記録が Load で復元される(未観測として保留されない)"
            );
        }
    }
    run_until_ended(&mut app, id2, &keep2, CAP);
    assert_eq!(war_end(&app, id2).1.as_deref(), Some(continuous_end.as_str()));
    assert_eq!(war_end(&app, id2).2.as_deref(), Some("Mutual Stalemate White Peace"));

    // 古い Save(記録なし)の再現: Load 直後に記録を空にする。ロード直後に期間満了とはみなさず、
    // ロード後の最初の評価日から計測する。連続実行より早く終結しない。
    let dir3 = TempDir::new("oldsave");
    let mut old = setup_app(&dir3.0, Some(BYSTANDER));
    let (id3, keep3) = add_even_war(&mut old);
    run_until(&mut old, &keep3, "1801/12/01");
    send_save(&mut old);
    send_load(&mut old);
    old.world_mut().resource_mut::<WarRegistry>().stalemate_records.clear();
    maintain(&mut old, &keep3);
    advance_one_day(&mut old);
    assert_eq!(status(&old, id3), WarStatus::Active, "記録がない直後には終結しない");
    let restart = old.world().resource::<WarRegistry>().stalemate_records[&id3]
        .last_progress_date
        .clone();
    assert_eq!(restart, "1801/12/02", "ロード後の最初の評価日から計測する");
    let last_progress = run_until_ended(&mut old, id3, &keep3, "1815/01/01");
    assert!(last_progress.as_str() >= "1801/12/02");
    assert_eq!(war_end(&old, id3).1, Some(plus_period(&last_progress)));
    assert!(
        war_end(&old, id3).1.unwrap().as_str() >= "1803/12/02",
        "記録を失ったため、730 日より早く終結しない"
    );
}
