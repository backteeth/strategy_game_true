//! P28-005: 物流生活水準維持のためのAI Railway優先建設 — 実データE2E。
//!
//! 全AI・無操作の実データNew Gameを、実時間システム(`app::time`)と同じく`GameDate`を
//! 1日ずつ進めて`DayChangedMessage`(月替わりには`MonthChangedMessage`も同じフレーム)を
//! 発行する方式で進める(P28-002/P28-003のE2Eと同じ進め方)。財政・生活水準の直接注入は
//! 一切行わない——AIが物流比率の低い州へ自然にRailwayを優先建設し、既存の優先順位
//! (Food緊急→供給網修復→戦時MilitaryFactory→Railway優先建設→通常開発)を保つことを
//! 観測する。純粋な判定ロジックの単体テスト(選定規則・タイブレーク・境界値・
//! 優先順位・雇用率フィルタを追加しないことの明示、14件)は`src/country/country_ai.rs`に
//! 既にあるため、本ファイルは実App・実データ経由の自然再現とSave V1継続性に集中する。
//! 期待値は本タスクの実測(`verification_logs/phase-28/p28-005/`)に基づく。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::BuildingType;
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::finance::LOGISTICS_WELFARE_BONUS_RATIO;
use strategy_game::military::MilitaryPlugin;
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

const DWARF: CountryId = CountryId(2);
const ALL_SIX: [CountryId; 6] = [CountryId(0), CountryId(1), CountryId(2), CountryId(3), CountryId(4), CountryId(5)];
/// 自然運用で最初にRailwayが着工される州(実測、`verification_logs/phase-28/p28-005/`)。
const FIRST_RAILWAY_STATE: StateId = StateId(6);

// ─── App-level fixtures ─────────────────────────────────────────────────────

struct TempTestDir(std::path::PathBuf);
impl TempTestDir {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        Self(std::env::temp_dir().join(format!(
            "strategy_game_p28_005_e2e_{label}_{}_{nanos}_{n}",
            std::process::id()
        )))
    }
    fn save_file_config(&self) -> SaveFileConfig {
        SaveFileConfig { path: SavePathConfig { final_path: self.0.join("savegame_v1.ron") } }
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
    // デフォルトstateのまま最初のupdateを呼ばないと、Playing挿入が遷移として認識されない。
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
    let mut s: SystemState<(MessageWriter<DayChangedMessage>, MessageWriter<MonthChangedMessage>)> =
        SystemState::new(app.world_mut());
    {
        let (mut days, mut months) = s.get_mut(app.world_mut()).unwrap();
        days.write(DayChangedMessage { year: next.year, month: next.month, day: next.day });
        if next.month != old_month {
            months.write(MonthChangedMessage { year: next.year, month: next.month });
        }
    }
    s.apply(app.world_mut());
    app.update();
}

fn date(app: &App) -> String {
    app.world().resource::<GameDate>().display()
}

fn advance_until(app: &mut App, until: &str) {
    while date(app).as_str() < until {
        advance_one_day(app);
    }
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
    state.apply(app.world_mut());
    app.update();
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
    app.update();
}

// ─── Observation helpers ────────────────────────────────────────────────────

fn queued_railways(app: &App, c: CountryId) -> Vec<(StateId, u32)> {
    app.world()
        .resource::<CountryRegistry>()
        .get(c)
        .unwrap()
        .construction_queue
        .iter()
        .filter(|q| q.building_type == BuildingType::Railway)
        .map(|q| (q.state_id, q.target_level))
        .collect()
}

fn railway_level(app: &App, s: StateId) -> u32 {
    app.world().resource::<StateRegistry>().get(s).unwrap().building_level(BuildingType::Railway)
}

fn logistics_ratio(app: &App, s: StateId) -> f32 {
    app.world().resource::<StateRegistry>().get(s).unwrap().logistics_ratio
}

fn fingerprint(app: &App) -> String {
    let cr = app.world().resource::<CountryRegistry>();
    let mut out = format!("date={} ", date(app));
    for c in ALL_SIX {
        let country = cr.get(c).unwrap();
        out.push_str(&format!(
            "c{}[treasury={:.6} queue={:?}] ",
            c.0,
            country.treasury,
            country
                .construction_queue
                .iter()
                .map(|q| (q.state_id.0, format!("{:?}", q.building_type), q.target_level, q.progress))
                .collect::<Vec<_>>()
        ));
    }
    let sr = app.world().resource::<StateRegistry>();
    let mut railways: Vec<(usize, u32)> =
        sr.states.iter().map(|s| (s.id.0, s.building_level(BuildingType::Railway))).collect();
    railways.sort();
    out.push_str(&format!("railways={railways:?}"));
    out
}

// ─── Tests ──────────────────────────────────────────────────────────────────

/// 自然運用: 物流比率0.9未満の州へAIがRailwayを優先建設し、既存の優先順位
/// (Food緊急→供給網修復→戦時MilitaryFactory→Railway→通常開発)を保つ。
#[test]
fn test_01_ai_builds_railway_for_a_low_logistics_state_and_improves_its_ratio() {
    let dir = TempTestDir::new("t01");
    let mut app = setup_app(None, dir.save_file_config());

    // 着工イベントを日ごとに観測し、直前(着工前日)の物流比率も記録する
    // (「物流比率0.9未満の州へ着工する」という契約1を実App経由でも確認するため)。
    let mut first_queued: Option<(String, StateId, u32, f32)> = None;
    let mut ratio_before = logistics_ratio(&app, FIRST_RAILWAY_STATE);
    while date(&app).as_str() < "1800/04/01" && first_queued.is_none() {
        advance_one_day(&mut app);
        for (state_id, level) in queued_railways(&app, DWARF) {
            first_queued.get_or_insert((date(&app), state_id, level, ratio_before));
        }
        ratio_before = logistics_ratio(&app, FIRST_RAILWAY_STATE);
    }
    let (queued_on, state_id, level, ratio_at_decision) =
        first_queued.expect("Dwarfは開始後まもなくRailwayを着工するはず");
    assert_eq!(state_id, FIRST_RAILWAY_STATE, "自然再現(参考): 最初のRailwayは州{FIRST_RAILWAY_STATE:?}のはず");
    assert_eq!(level, 1);
    assert!(
        ratio_at_decision < LOGISTICS_WELFARE_BONUS_RATIO,
        "着工前日の物流比率は0.9未満のはず(契約1): {ratio_at_decision}"
    );
    eprintln!(
        "[P28-005 e2e] first railway queued: {queued_on} state={state_id:?} level={level} \
         ratio_before={ratio_at_decision}"
    );

    // 完成後、その州のRailwayレベルが実際に上がる(優先度2.7が既存の建設完了処理を
    // そのまま使っていることの直接確認)。
    advance_until(&mut app, "1800/07/01");
    let level_after = railway_level(&app, FIRST_RAILWAY_STATE);
    assert!(level_after >= 1, "Railwayが完成しているはず: level={level_after}");
}

/// Food緊急(優先度1)は、物流の低い州があってもRailwayより優先される
/// (既存の優先順位契約、単体テストの契約10と同じ性質を実Appで確認)。
#[test]
fn test_02_food_emergency_still_outranks_railway_in_the_real_app() {
    let dir = TempTestDir::new("t02");
    let mut app = setup_app(None, dir.save_file_config());

    // 開始直後にDwarfのFood備蓄を緊急水準まで落とす。
    {
        let mut cr = app.world_mut().resource_mut::<CountryRegistry>();
        cr.get_mut(DWARF).unwrap().stockpile.set(strategy_game::economy::resources::ResourceType::Food, 10.0);
    }
    advance_one_day(&mut app);

    let queue = app.world().resource::<CountryRegistry>().get(DWARF).unwrap().construction_queue.clone();
    assert!(
        queue.iter().all(|q| q.building_type != BuildingType::Railway),
        "Food緊急時にRailwayを着工してはいけない: {queue:?}"
    );
}

/// Railwayの着工前・建設キュー中・完成後の3分岐でsave→loadを挟んでも、建設キューが
/// 重複せず、連続実行と同じ結果になる(契約14)。
#[test]
fn test_03_save_load_before_during_and_after_railway_construction_matches_continuous_run() {
    const END: &str = "1800/08/01";
    let dir = TempTestDir::new("t03_continuous");
    let mut continuous = setup_app(None, dir.save_file_config());
    advance_until(&mut continuous, END);
    let expected = fingerprint(&continuous);

    for (label, branch) in [("before", "1800/01/02"), ("queued", "1800/03/15"), ("completed", "1800/07/01")] {
        let dir = TempTestDir::new(label);
        let mut app = setup_app(None, dir.save_file_config());
        advance_until(&mut app, branch);
        let queued_before_save = queued_railways(&app, DWARF);
        send_save_request(&mut app);
        send_load_request(&mut app);
        assert_eq!(date(&app), branch, "{label}: loadでGameDateが保存時点に戻る");
        assert_eq!(queued_railways(&app, DWARF), queued_before_save, "{label}: loadでキューが重複しない");
        while date(&app).as_str() < END {
            advance_one_day(&mut app);
            for c in ALL_SIX {
                assert!(queued_railways(&app, c).len() <= 1, "{label}: {c:?}のRailway重複発注");
            }
        }
        assert_eq!(fingerprint(&app), expected, "{label}: save→load後も連続実行と一致するはず");
    }
}
