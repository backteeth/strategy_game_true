//! P31-004: 手動通しプレイで E1(停滞した AI 戦争の白紙講和)を画面で確認するための、人工的な確認用 Save の生成。
//!
//! **人工的な確認条件**: 通常のプレイでは E1 を任意の時点で再現できない(AI 同士の戦争で、一方に師団がなく、
//! 365 日間進展がない場合だけ発動する)。このテストは、実 New Game に、Dwarf 対 Arcadia の AI 戦争を
//! 制御 fixture として追加し、両国を毎日「無力(師団 0・募兵 0)」に保って E1 の発動 5 日前まで進め、Save を書き出す。
//! ユーザーの Save(`saves/savegame_v1.ron`)は読み書きしない。生成先は `target/p31_004_e1_fixture/`。
//! 通常のテスト実行では動かさない(`#[ignore]`)。実行: `cargo test --release --test p31_004_e1_manual_fixture_test -- --ignored --nocapture`

#![allow(dead_code, unused_imports)]

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
fn send_save(app: &mut App) {
    let mut s: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut()).unwrap().write(SaveRequestMessage);
    s.apply(app.world_mut());
    app.update();
}

#[test]
#[ignore = "手動確認用の人工的な Save を生成する(通常のテスト実行では動かさない)"]
fn generate_the_artificial_e1_manual_fixture_save() {
    let dir = std::path::PathBuf::from("target/p31_004_e1_fixture");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = setup_app(&dir, Some(BYSTANDER));
    let id = add_stalled_war(&mut app);
    // 停滞記録の起点(最後の進展の日。自然運用の他の戦争の進展でも更新される)から 360 日目
    // (E1 の発動 5 日前)まで進めて Save する。
    let mut last_progress = String::new();
    while date(&app).as_str() < "1803/01/01" {
        keep_powerless(&mut app);
        advance_one_day(&mut app);
        if let Some(r) = app.world().resource::<WarRegistry>().stalemate_records.get(&id) {
            last_progress = r.last_progress_date.clone();
            let due = GameDate::from_string(&last_progress).unwrap().add_days(360).display();
            if date(&app).as_str() >= due.as_str() {
                break;
            }
        }
    }
    assert_eq!(status(&app, id), WarStatus::Active, "Save 時点では戦争は継続している");
    send_save(&mut app);
    let saved = date(&app);
    let path = dir.join("savegame_v1.ron");
    assert!(path.exists(), "Save が書き出される");
    let expected_end = GameDate::from_string(&last_progress).unwrap().add_days(365).display();
    println!(
        "E1_FIXTURE_SAVE path={} date={saved} player=Passguard war_id={id:?} last_progress={last_progress} expected_end={expected_end}",
        path.display()
    );
    // 確認: Load 後は fixture の維持(毎日の無力化)が無い状態で進む。そのまま進めても、最後の進展の
    // 365 日後に E1 で終結する。
    for _ in 0..12 {
        if status(&app, id) != WarStatus::Active {
            break;
        }
        advance_one_day(&mut app);
    }
    let w = &app.world().resource::<WarRegistry>().wars[&id];
    assert_ne!(w.status, WarStatus::Active, "進めると終結する");
    assert_eq!(w.end_date.as_deref(), Some(expected_end.as_str()));
    assert_eq!(w.end_reason.as_deref(), Some("Stalemate White Peace"));
    // 確認: 生成した Save ファイルそのものを、新しい App に Load しても、同じ日に E1 で終結する
    // (ユーザーがゲームで Load するのと同じ経路。fixture の維持なし)。
    let mut app2 = setup_app(&dir, Some(BYSTANDER));
    send_load(&mut app2);
    assert_eq!(date(&app2), saved, "Save 時点の日付へ復元される");
    assert_eq!(status(&app2, id), WarStatus::Active);
    for _ in 0..12 {
        if status(&app2, id) != WarStatus::Active {
            break;
        }
        advance_one_day(&mut app2);
    }
    let (st2, end2, reason2) = app2_end(&app2, id);
    assert_ne!(st2, WarStatus::Active, "Load した Save でも終結する");
    assert_eq!(end2.as_deref(), Some(expected_end.as_str()));
    assert_eq!(reason2.as_deref(), Some("Stalemate White Peace"));
}

fn send_load(app: &mut App) {
    let mut s: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut()).unwrap().write(LoadRequestMessage);
    s.apply(app.world_mut());
    app.update();
    app.update();
}

fn app2_end(app: &App, id: WarId) -> (WarStatus, Option<String>, Option<String>) {
    let w = &app.world().resource::<WarRegistry>().wars[&id];
    (w.status, w.end_date.clone(), w.end_reason.clone())
}
