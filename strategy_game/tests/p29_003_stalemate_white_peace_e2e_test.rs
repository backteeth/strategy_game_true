//! P29-003: 長期停滞した AI 戦争の白紙講和(E1)を、実際の New Game 初期化・日次システム・
//! 実際の `handle_daily_war_resolution`・Save/Load を通して検証する。
//!
//! 制御 fixture: 実マップで Dwarf(c2)が Arcadia(c0)の州2(Western Mage Province)を目標に
//! 宣戦した AI 戦争を 1 件追加し、両国の師団・募兵を毎日取り除いて「双方が無力」な状態を保つ
//! (自然運用では AI が募兵・攻勢するため、停滞を決定論的に作るための操作)。判定ロジックの
//! 個別条件(両陣営に師団がある・攻勢可能・進行中の戦闘・支持国など)は
//! `war::stalemate::tests`(単体)が網羅する。自然運用の 600 か月の比較は
//! `p29_003_diag_001_continuing_wars_e2e_test` と報告書を参照。

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
            "strategy_game_p29_003_{label}_{}_{}",
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

/// 制御 fixture: 両国を毎日「無力」に保つ(師団・募兵・動員可能人員を取り除く)。
fn keep_powerless(app: &mut App) {
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        let ids: Vec<_> = mr
            .divisions
            .values()
            .filter(|d| d.owner == DWARF || d.owner == ARCADIA)
            .map(|d| d.id)
            .collect();
        for id in ids {
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

fn add_stalled_war(app: &mut App) -> WarId {
    keep_powerless(app);
    let war = War {
        id: WarId(0), // add_war が上書きする
        name: "P29-003 stalemate fixture".to_string(),
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
    app.world_mut().resource_mut::<WarRegistry>().add_war(war)
}

fn status(app: &App, id: WarId) -> WarStatus {
    app.world().resource::<WarRegistry>().wars[&id].status
}

/// 日付が`until`(`YYYY/MM/DD`)になる(その日のtick後)まで、毎日 fixture を保ちながら進める。
fn run_until(app: &mut App, until: &str) {
    while date(app).as_str() < until {
        keep_powerless(app);
        advance_one_day(app);
    }
}

/// 戦争が終結するまで(上限日まで)毎日 fixture を保ちながら進め、終結直前に観測した
/// 停滞期間の起点を返す。
/// 自然運用の世界では、当事国が関わる別の戦争の支配変更も「進展」として起点を更新するため、
/// 終結日は「最後の進展の 365 日後」になる(固定の日付ではない)。
fn run_until_ended(app: &mut App, id: WarId, cap: &str) -> String {
    let mut last_progress = String::new();
    while date(app).as_str() < cap {
        if status(app, id) != WarStatus::Active {
            break;
        }
        if let Some(r) = app.world().resource::<WarRegistry>().stalemate_records.get(&id) {
            last_progress = r.last_progress_date.clone();
        }
        keep_powerless(app);
        advance_one_day(app);
    }
    assert_ne!(status(app, id), WarStatus::Active, "上限日までに終結する");
    last_progress
}

/// `YYYY/MM/DD`の 365 日後。
fn plus_period(d: &str) -> String {
    GameDate::from_string(d).unwrap().add_days(365).display()
}

fn war_end(app: &App, id: WarId) -> (WarStatus, Option<String>, Option<String>) {
    let w = &app.world().resource::<WarRegistry>().wars[&id];
    (w.status, w.end_date.clone(), w.end_reason.clone())
}

// ─────────────────────────────────────────────────────────────────────────

#[test]
fn a_stalled_war_between_powerless_ai_countries_ends_in_white_peace_after_the_period_only() {
    let dir = TempDir::new("basic");
    let mut app = setup_app(&dir.0, None);
    let id = add_stalled_war(&mut app);

    // 期間(365 日)の満了前には終結せず、最後の進展の 365 日後に終結する。
    run_until(&mut app, "1800/12/01");
    assert_eq!(status(&app, id), WarStatus::Active, "最初の 1 年未満では終結しない");
    assert!(
        app.world()
            .resource::<WarRegistry>()
            .stalemate_records
            .contains_key(&id)
    );
    let last_progress = run_until_ended(&mut app, id, "1803/01/01");
    let (st, end_date, reason) = war_end(&app, id);
    assert_ne!(st, WarStatus::Active);
    assert_eq!(end_date, Some(plus_period(&last_progress)), "最後の進展の 365 日後");
    assert!(last_progress.as_str() >= "1800/01/02");
    let end_date = end_date.unwrap();
    assert_eq!(reason.as_deref(), Some("Stalemate White Peace"));

    // 白紙講和: 領土の移転なし。目標州は Arcadia のまま(fixture の副作用で、無力な当事国が
    // 他国の別戦争で州を失うことはあるため、この戦争の目標州だけを確認する)。
    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(GOAL_STATE)
            .unwrap()
            .owner_country_id,
        ARCADIA
    );
    // 戦争は参加国の集合から解消され、前線・停滞記録は残らない。
    let wr = app.world().resource::<WarRegistry>();
    assert!(!wr.wars_for_country(DWARF).contains(&id));
    assert!(!wr.wars_for_country(ARCADIA).contains(&id));
    assert!(!wr.stalemate_records.contains_key(&id));
    assert!(
        app.world()
            .resource::<FrontlineRegistry>()
            .get_frontline_for_war(id)
            .is_none()
    );
    // 二重に終結しない(翌日以降も終結情報が変わらない)。
    run_until(&mut app, "1803/06/01");
    assert_eq!(war_end(&app, id).1.as_deref(), Some(end_date.as_str()));
}

#[test]
fn a_war_with_a_player_participant_is_never_ended_by_the_stalemate_rule() {
    let dir = TempDir::new("player");
    let mut app = setup_app(&dir.0, Some(DWARF));
    let id = add_stalled_war(&mut app);
    run_until(&mut app, "1801/06/01");
    assert_eq!(status(&app, id), WarStatus::Active);
    assert!(
        !app.world()
            .resource::<WarRegistry>()
            .stalemate_records
            .contains_key(&id),
        "プレイヤー参加戦争は停滞期間を計測しない"
    );
}

#[test]
fn military_progress_resets_the_period() {
    let dir = TempDir::new("progress");
    let mut app = setup_app(&dir.0, None);
    let id = add_stalled_war(&mut app);
    run_until(&mut app, "1800/09/01");
    assert_eq!(status(&app, id), WarStatus::Active);
    // 軍事的進展(州の支配変更): Dwarf が Arcadia の州(戦争目標ではない)を支配する。
    app.world_mut()
        .resource_mut::<StateRegistry>()
        .get_mut(OTHER_ARCADIA_STATE)
        .unwrap()
        .controller_country = Some(DWARF);
    run_until(&mut app, "1800/09/03");
    // 本来の満了日(1801/01/02)を過ぎても終結しない。
    run_until(&mut app, "1801/03/01");
    assert_eq!(status(&app, id), WarStatus::Active, "進展により期間がリセットされた");
    // 進展の翌日(1800/09/02 付近)から 365 日後に終結する。
    run_until(&mut app, "1801/09/04");
    let (st, end_date, reason) = war_end(&app, id);
    assert_ne!(st, WarStatus::Active);
    assert_eq!(reason.as_deref(), Some("Stalemate White Peace"));
    let end = end_date.unwrap();
    assert!(
        end.as_str() >= "1801/08/30" && end.as_str() <= "1801/09/03",
        "終結日は進展日の約 365 日後: {end}"
    );
    // 白紙講和により、占領状態は正規化される(支配者が所有者へ戻る)。
    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(OTHER_ARCADIA_STATE)
            .unwrap()
            .controller(),
        ARCADIA
    );
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

#[test]
fn the_period_survives_save_and_load_and_an_old_save_without_it_restarts_counting() {
    // 連続実行での終結日。
    let dir = TempDir::new("continuous");
    let mut cont = setup_app(&dir.0, Some(BYSTANDER));
    let id = add_stalled_war(&mut cont);
    run_until_ended(&mut cont, id, "1803/01/01");
    let continuous_end = war_end(&cont, id).1.expect("ended");

    // 途中(1800/07/01)で Save → Load しても、終結日は連続実行と同じ。
    let dir2 = TempDir::new("saveload");
    let mut app = setup_app(&dir2.0, Some(BYSTANDER));
    let id2 = add_stalled_war(&mut app);
    run_until(&mut app, "1800/07/01");
    let saved_progress = app.world().resource::<WarRegistry>().stalemate_records[&id2]
        .last_progress_date
        .clone();
    send_save(&mut app);
    // Save の後に進めて(記録を進めて)から Load し、Save 時点の記録へ戻ることを確認する。
    run_until(&mut app, "1800/08/15");
    send_load(&mut app);
    assert_eq!(date(&app), "1800/07/01");
    assert_eq!(
        app.world().resource::<WarRegistry>().stalemate_records[&id2].last_progress_date,
        saved_progress,
        "停滞期間の起点が Save から復元される"
    );
    run_until_ended(&mut app, id2, "1803/01/01");
    assert_eq!(war_end(&app, id2).1.as_deref(), Some(continuous_end.as_str()));

    // 古い Save(記録なし)の再現: Load 直後に記録を空にする。ロード直後に期間満了とは
    // みなさず、ロード後の最初の評価日から 365 日(最後の進展から)後に終結する。
    let dir3 = TempDir::new("oldsave");
    let mut old = setup_app(&dir3.0, Some(BYSTANDER));
    let id3 = add_stalled_war(&mut old);
    run_until(&mut old, "1800/12/01");
    send_save(&mut old);
    send_load(&mut old);
    old.world_mut()
        .resource_mut::<WarRegistry>()
        .stalemate_records
        .clear();
    // 開戦(1799/12/01)から 1 年以上経過していても、記録が無い直後には終結しない。
    keep_powerless(&mut old);
    advance_one_day(&mut old);
    assert_eq!(status(&old, id3), WarStatus::Active);
    let restart = old.world().resource::<WarRegistry>().stalemate_records[&id3]
        .last_progress_date
        .clone();
    assert_eq!(restart, "1800/12/02", "ロード後の最初の評価日から計測する");
    let last_progress = run_until_ended(&mut old, id3, "1803/01/01");
    assert!(last_progress.as_str() >= "1800/12/02");
    assert_eq!(war_end(&old, id3).1, Some(plus_period(&last_progress)));
    assert!(
        war_end(&old, id3).1.unwrap().as_str() >= "1801/12/02",
        "記録を失ったため、連続実行より早く終結しない"
    );
}
