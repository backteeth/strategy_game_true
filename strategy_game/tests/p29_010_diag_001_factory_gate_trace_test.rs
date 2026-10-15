//! P29-010: Factory 増設抑止(B2)の発動の診断。対象国ごとに、判断日(30 日周期)に抑止が成立した日数と
//! 初日・最終日、抑止中に Factory 着工がなかったことを記録する(新しいコードでのみコンパイルできる)。
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



/// 診断対象の国(環境変数 `P29_DIAG_COUNTRY`、既定 Elfin = 1)。
fn target_id() -> CountryId {
    CountryId(
        std::env::var("P29_DIAG_COUNTRY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
    )
}

#[test]
fn p29_010_factory_gate_trace() {
    use strategy_game::country::country_ai::{CountryAiRegistry, factory_expansion_blocked};
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p29_010_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    let me = target_id();
    let mut blocked_days = 0u32;
    let mut decision_days_blocked = 0u32;
    let mut first: Option<String> = None;
    let mut last: Option<String> = None;
    let mut factory_starts_while_blocked = 0u32;
    let mut seen: std::collections::BTreeSet<(usize, u32)> = Default::default();
    while app.world().resource::<GameDate>().year < 1850 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        let cr = app.world().resource::<CountryRegistry>();
        let sr = app.world().resource::<StateRegistry>();
        let ai = app.world().resource::<CountryAiRegistry>();
        let Some(c) = cr.get(me) else { continue };
        let blocked = ai
            .factory_starvation_history
            .get(&me)
            .is_some_and(|h| factory_expansion_blocked(h, &date.display(), me, sr));
        for q in &c.construction_queue {
            if q.building_type == strategy_game::building::data::BuildingType::Factory
                && seen.insert((q.state_id.0, q.target_level))
                && blocked
            {
                factory_starts_while_blocked += 1;
            }
        }
        if blocked {
            blocked_days += 1;
            if date.day as usize % 30 == me.0 % 30 {
                decision_days_blocked += 1;
            }
            first.get_or_insert(date.display());
            last = Some(date.display());
        }
    }
    println!(
        "B2 c{} blocked_days={blocked_days} decision_days_blocked={decision_days_blocked} first={first:?} last={last:?} factory_starts_while_blocked={factory_starts_while_blocked}",
        me.0
    );
}
