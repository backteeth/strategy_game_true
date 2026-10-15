//! P29-008-DIAG-001: Iron 不足に対する建設 AI の判断(着工・完成)と、その時点の経済状態の診断。
//! 標準全 AI シナリオを 600 か月進め、対象国(既定 Elfin)の着工・完成を日次で記録する。
use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::BTreeMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::CountryId;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
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



/// 診断対象の国(既定 Elfin = 1。環境変数 `P29_DIAG_COUNTRY` で他国へ変更できる)。
fn target_id() -> CountryId {
    CountryId(
        std::env::var("P29_DIAG_COUNTRY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
    )
}

/// P29-008-DIAG-001: 建設 AI の判断(着工)と、その時点の経済状態を日次で記録する。
/// 着工 = 建設キューの新しい (州, 建物, 目標レベル)。完成 = 州の建物レベルの増加。
#[test]
fn p29_008_construction_trace() {
    use strategy_game::building::data::BuildingType as B;
    use strategy_game::country::country_ai::CountryAiRegistry;
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p29_008_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    let me = target_id();
    let mut seen: std::collections::BTreeSet<(usize, String, u32)> = Default::default();
    let mut levels: BTreeMap<(usize, String), u32> = BTreeMap::new();
    let mut starts = 0;
    while app.world().resource::<GameDate>().year < 1850 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        let cr = app.world().resource::<CountryRegistry>();
        let sr = app.world().resource::<StateRegistry>();
        let ai = app.world().resource::<CountryAiRegistry>();
        let Some(c) = cr.get(me) else { continue };
        let reason = ai
            .ai_states
            .get(&me)
            .map(|s| format!("{:?}", s.decision_reason))
            .unwrap_or_default();
        for q in &c.construction_queue {
            let key = (q.state_id.0, format!("{:?}", q.building_type), q.target_level);
            if seen.insert(key.clone()) {
                starts += 1;
                // 着工直後の備蓄・国庫(着工コストは国庫から引かれた後)。
                let st = &c.stockpile;
                let lv = |b: B| -> u32 {
                    sr.states
                        .iter()
                        .filter(|s| s.owner_country_id == me)
                        .map(|s| s.building_level(b))
                        .sum()
                };
                println!(
                    "START {} state={} {}→lv{} reason={} treasury={:.0} queue={} stock[Fe={:.0} Co={:.0} Wd={:.0} IG={:.0} Food={:.0}] lv[Mine={} F={} MF={}]",
                    date.display(), key.0, key.1, key.2, reason, c.treasury, c.construction_queue.len(),
                    st.get(ResourceType::Iron), st.get(ResourceType::Coal), st.get(ResourceType::Wood),
                    st.get(ResourceType::IndustrialGoods), st.get(ResourceType::Food),
                    lv(B::Mine), lv(B::Factory), lv(B::MilitaryFactory)
                );
            }
        }
        for s in sr.states.iter().filter(|s| s.owner_country_id == me) {
            for b in [B::Mine, B::Factory, B::MilitaryFactory] {
                let key = (s.id.0, format!("{b:?}"));
                let l = s.building_level(b);
                let old = levels.insert(key.clone(), l).unwrap_or(0);
                if l > old && date.display().as_str() > "1800/01/02" {
                    println!("DONE  {} state={} {}→lv{}", date.display(), key.0, key.1, l);
                }
            }
        }
    }
    println!("STARTS total={starts}");
}
