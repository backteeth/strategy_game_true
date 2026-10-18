//! P31-003: 手動通しプレイで M1(双方武装停滞の合意白紙講和)を画面で確認するための、人工的な確認用 Save の生成。
//!
//! **人工的な確認条件**: 通常のプレイでは M1 を任意の時点で再現できない(AI 同士の戦争で、両陣営に師団があり、
//! 730 日間進展がない場合だけ発動する)。このテストは、実 New Game に、Dwarf 対 Arcadia の AI 戦争(両国に
//! 同じ戦力の師団 2 個ずつ)を制御 fixture として追加し、M1 の発動 5 日前まで進めて Save を書き出す。
//! ユーザーの Save(`saves/savegame_v1.ron`)は読み書きしない。生成先は `target/p31_003_manual_fixture/`。
//! 通常のテスト実行では動かさない(`#[ignore]`)。実行: `cargo test --release --test p31_003_manual_fixture_test -- --ignored --nocapture`

#![allow(dead_code)]


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
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};

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
#[ignore = "手動確認用の人工的な Save を生成する(通常のテスト実行では動かさない)"]
fn generate_the_artificial_m1_manual_fixture_save() {
    let dir = std::path::PathBuf::from("target/p31_003_manual_fixture");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = setup_app(&dir, Some(BYSTANDER));
    let (id, keep) = add_even_war(&mut app);
    // 停滞記録の起点(最後の進展の日)から 725 日目(M1 の発動 5 日前)まで進めて Save する。
    let mut last_progress = String::new();
    while date(&app).as_str() < "1805/01/01" {
        maintain(&mut app, &keep);
        advance_one_day(&mut app);
        if let Some(r) = app.world().resource::<WarRegistry>().stalemate_records.get(&id) {
            last_progress = r.last_progress_date.clone();
            let due = GameDate::from_string(&last_progress).unwrap().add_days(725).display();
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
    let expected_end = GameDate::from_string(&last_progress).unwrap().add_days(730).display();
    println!(
        "M1_FIXTURE_SAVE path={} date={saved} player=Passguard war_id={id:?} last_progress={last_progress} expected_end={expected_end}",
        path.display()
    );
    // 確認: Load 後は fixture の維持(毎日の戦力の固定)が無い状態で進む。そのまま進めても、最後の進展の
    // 730 日後に M1 で終結する。
    for _ in 0..12 {
        if status(&app, id) != WarStatus::Active {
            break;
        }
        advance_one_day(&mut app);
    }
    let (st, end, reason) = war_end(&app, id);
    assert_ne!(st, WarStatus::Active, "進めると終結する");
    assert_eq!(end.as_deref(), Some(expected_end.as_str()));
    assert_eq!(reason.as_deref(), Some("Mutual Stalemate White Peace"));
    // 確認: 生成した Save ファイルそのものを、新しい App に Load しても、同じ日に M1 で終結する
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
    let (st2, end2, reason2) = war_end(&app2, id);
    assert_ne!(st2, WarStatus::Active, "Load した Save でも終結する");
    assert_eq!(end2.as_deref(), Some(expected_end.as_str()));
    assert_eq!(reason2.as_deref(), Some("Mutual Stalemate White Peace"));
}
