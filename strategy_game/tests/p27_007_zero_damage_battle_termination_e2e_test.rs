//! P27-007: 両陣営ゼロ損害戦闘の即日終了・攻撃側撤退の、実App・実AI統合テスト。
//!
//! 判定式・退却先・退路なし・複数Division・save/load・決定性の単体寄りの検証は
//! `tests/p27_006_low_effective_power_damage_e2e_test.rs`(test_08・test_11〜test_13)に
//! あるため、本ファイルは実際のAI(`war::military_ai::process_daily_military_ai`、
//! P26-008-Bの攻勢供給ゲート)を通した長期統合、特にAIの即時再攻撃ループが起きない
//! ことに集中する(P27-007-PRE-001 §31・§34で一時パッチにより実測した内容の恒久化)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::CountryId;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::{BattleRegistry, BattleStatus};
use strategy_game::military::data::MilitaryRegistry;
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
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::{FrontlineRegistry, FrontlineStance, update_all_frontlines};

const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p27_007_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
}

struct TempTestDir(std::path::PathBuf);
impl TempTestDir {
    fn new(label: &str) -> Self {
        Self(unique_temp_dir(label))
    }
    fn save_file_config(&self) -> SaveFileConfig {
        SaveFileConfig {
            path: SavePathConfig {
                final_path: self.0.join("savegame_v1.ron"),
            },
        }
    }
}
impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_app(player: Option<CountryId>, save_config: SaveFileConfig) -> App {
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
    app.insert_resource(save_config);
    app.update();
    app.insert_resource(PlayerCountry(player));
    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn set_game_date(app: &mut App, year: i32, month: u8, day: u8) {
    let mut gd = app.world_mut().resource_mut::<GameDate>();
    gd.year = year;
    gd.month = month;
    gd.day = day;
}

fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn advance_days(app: &mut App, start_year: i32, start_month: u8, start_day: u8, count: u32) {
    let (mut year, mut month, mut day) = (start_year, start_month, start_day);
    for _ in 0..count {
        set_game_date(app, year, month, day);
        tick_day(app, year, month, day);
        day += 1;
        if day > 28 {
            day = 1;
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
    }
}

fn set_app_supplies(app: &mut App, c: CountryId, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(c)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitarySupplies, value);
}

fn new_war_no_goals(attacker: CountryId, defender: CountryId) -> War {
    War {
        id: strategy_game::common::WarId(0),
        name: "Test War".to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1799/12/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: std::collections::HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: std::collections::HashSet::new(),
    }
}

/// 実データ・実AI(`MilitaryPlugin`込み)でArcadia対Elfinの戦争を作り、Arcadiaの陸軍を
/// 大幅増強して110%条件を満たす攻勢を開始させる(P26-008-B 90日テストと同じ方針)。
fn setup_offensive_war() -> (App, TempTestDir, strategy_game::common::WarId) {
    let dir = TempTestDir::new("reattack");
    let mut app = setup_app(None, dir.save_file_config());
    let war_id = app
        .world_mut()
        .resource_mut::<WarRegistry>()
        .add_war(new_war_no_goals(ARCADIA, ELFIN));
    app.world_mut().resource_scope(|world, mut fr: Mut<FrontlineRegistry>| {
        let w = world.resource::<WarRegistry>();
        let s = world.resource::<StateRegistry>();
        let m = world.resource::<MilitaryRegistry>();
        update_all_frontlines(w, s, m, &mut fr);
    });
    {
        let mut military_registry = app.world_mut().resource_mut::<MilitaryRegistry>();
        for division in military_registry.divisions.values_mut() {
            if division.owner == ARCADIA {
                division.manpower = 100_000;
                division.max_manpower = 100_000;
            }
        }
    }
    set_app_supplies(&mut app, ARCADIA, 1_000_000.0);
    (app, dir, war_id)
}

fn ongoing_battle_count(app: &App) -> usize {
    app.world()
        .resource::<BattleRegistry>()
        .battles
        .values()
        .filter(|b| b.status == BattleStatus::Ongoing)
        .count()
}

/// P27-007-PRE-001 §31で一時パッチにより実測した内容の恒久化: 攻撃側が敵州へ到達して
/// 交戦したところで両国のMilitarySupplies備蓄を同時に0へ落とし、両陣営ゼロ損害を
/// 発生させる。P27-007により即日終了・攻撃側撤退するはずで、その後90日間、AIが
/// 同じ攻撃を即座に再発行して同じ膠着を繰り返す「撤退→再攻撃→再撤退」ループが
/// 起きないことを確認する(新しいcooldownを追加していないことの回帰証拠)。
#[test]
fn test_01_ai_does_not_form_a_retreat_reattack_loop_after_zero_damage_termination() {
    let (mut app, _dir, war_id) = setup_offensive_war();
    // 到達(概ねd10)まで健全な状態で進める。
    advance_days(&mut app, 1800, 1, 2, 15);
    // 両国の補給を同時に0にして両陣営ゼロ損害を発生させる。
    set_app_supplies(&mut app, ARCADIA, 0.0);
    set_app_supplies(&mut app, ELFIN, 0.0);

    let mut terminations = 0usize;
    let mut prev_ongoing = ongoing_battle_count(&app);
    let (mut year, mut month, mut day) = (1800, 1, 17);
    for i in 0..90u32 {
        set_game_date(&mut app, year, month, day);
        tick_day(&mut app, year, month, day);
        day += 1;
        if day > 28 {
            day = 1;
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
        let ongoing = ongoing_battle_count(&app);
        if ongoing == 0 && prev_ongoing > 0 {
            terminations += 1;
        }
        prev_ongoing = ongoing;
        // 30日目に両国の備蓄を回復させ、その後も再攻撃が起きないことを観測する。
        if i == 30 {
            set_app_supplies(&mut app, ARCADIA, 1_000_000.0);
            set_app_supplies(&mut app, ELFIN, 1_000_000.0);
        }
    }

    assert_eq!(terminations, 1, "90日間で終了は1回だけ(撤退→再攻撃ループが起きない)");
    assert_eq!(ongoing_battle_count(&app), 0, "90日後も戦闘は再開していない");

    let fl_id = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_frontline_for_war(war_id)
        .unwrap()
        .frontline_id;
    let stance = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_plan(fl_id, ARCADIA)
        .map(|p| p.stance);
    assert_ne!(
        stance,
        Some(FrontlineStance::Offensive),
        "P26-008-Bの既存ヒステリシスにより、備蓄回復直後にOffensiveへは戻らない"
    );
}

/// 両陣営ゼロ損害の終了処理が、他国の戦闘・備蓄・controllerへ影響しないことを確認する
/// (第三国Passguardは今回の戦争と無関係)。Passguard自身の経済は通常どおり毎日進行する
/// ため、「終了イベントを起こした場合」と「起こさない場合」を同じ日数だけ進めて比較する
/// (完全な凍結ではなく、この戦争・終了処理からの独立を検証する)。
#[test]
fn test_02_termination_does_not_affect_an_unrelated_third_country() {
    let passguard = CountryId(4);

    let (mut with_termination, _dir1, _war_id) = setup_offensive_war();
    advance_days(&mut with_termination, 1800, 1, 2, 15);
    set_app_supplies(&mut with_termination, ARCADIA, 0.0);
    set_app_supplies(&mut with_termination, ELFIN, 0.0);
    advance_days(&mut with_termination, 1800, 1, 17, 5);
    let with_termination_units = with_termination
        .world()
        .resource::<CountryRegistry>()
        .get(passguard)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitarySupplies);

    let dir2 = TempTestDir::new("no_war");
    let mut without_war = setup_app(None, dir2.save_file_config());
    advance_days(&mut without_war, 1800, 1, 2, 20);
    let without_war_units = without_war
        .world()
        .resource::<CountryRegistry>()
        .get(passguard)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitarySupplies);

    assert_eq!(
        with_termination_units, without_war_units,
        "Arcadia対Elfinの戦争・両陣営ゼロ損害終了は、無関係な第三国の備蓄に影響しない"
    );
}
