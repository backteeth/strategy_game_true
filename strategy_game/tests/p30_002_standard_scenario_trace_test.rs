//! P30-002: 標準全 AI シナリオ(通常、状態の操作なし)での軍需基盤の再建の記録。
use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
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



fn temp_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "strategy_game_p30_002_{label}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// 標準全 AI シナリオ(通常、状態の操作なし)で、軍需基盤の再建需要の認識(文脈)と、AI による
/// MilitaryFactory の着工を国別に記録する。再建の対象国がいない場合も、そのまま報告する(発動させる
/// ために条件を変更しない)。
#[test]
fn p30_002_standard_scenario_rebuild_trace() {
    use strategy_game::building::data::BuildingType as B;
    use strategy_game::country::country_ai::{CountryAiRegistry, military_rebuild_context};
    let dir = temp_dir("standard");
    let mut app = setup_app(&dir);
    let mut months_with_context = [0u32; 6];
    let mut first_context: [Option<String>; 6] = Default::default();
    let mut mf_queue_events = [0u32; 6];
    let mut seen: std::collections::BTreeSet<(usize, usize, u32)> = Default::default();
    let mut mf_levels_end = [0u32; 6];
    while app.world().resource::<GameDate>().year < 1850 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        let cr = app.world().resource::<CountryRegistry>();
        let sr = app.world().resource::<StateRegistry>();
        let mil = app.world().resource::<MilitaryRegistry>();
        let ai = app.world().resource::<CountryAiRegistry>();
        for c in &cr.countries {
            let i = c.id.0;
            for q in &c.construction_queue {
                if q.building_type == B::MilitaryFactory && seen.insert((i, q.state_id.0, q.target_level)) {
                    mf_queue_events[i] += 1;
                    println!("MF_QUEUED {} c{i} state={}", date.display(), q.state_id.0);
                }
            }
            if date.day == 1 {
                let empty = Default::default();
                let ctx = military_rebuild_context(
                    c,
                    sr,
                    mil,
                    ai.supply_production_history.get(&c.id).unwrap_or(&empty),
                    &date.display(),
                );
                if ctx.is_some() {
                    months_with_context[i] += 1;
                    first_context[i].get_or_insert(date.display());
                }
            }
        }
    }
    let sr = app.world().resource::<StateRegistry>();
    for (i, level) in mf_levels_end.iter_mut().enumerate() {
        *level = sr
            .states
            .iter()
            .filter(|s| s.owner_country_id.0 == i)
            .map(|s| s.building_level(B::MilitaryFactory))
            .sum();
    }
    for i in 0..6 {
        println!(
            "STD c{i} months_with_rebuild_need={} first={:?} ai_mf_queue_events={} mf_levels_1850={}",
            months_with_context[i], first_context[i], mf_queue_events[i], mf_levels_end[i]
        );
    }
    // 仕様上の結果: Oceanic は国内の Iron 供給源を全て失っており、再建需要を認識しても着工しない。
    assert_eq!(mf_queue_events[3], 0, "Oceanic は国内再建の対象外(供給源なし)");
}
