//! P29-008-DIAG-001: 国庫・収入・支出と背景(生産・雇用・建設・領土)の月次診断(既定 Passguard)。
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



/// 診断対象の国(既定 Passguard = 4。環境変数 `P29_DIAG_COUNTRY` で他国へ変更できる)。
fn target_id() -> CountryId {
    CountryId(
        std::env::var("P29_DIAG_COUNTRY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4),
    )
}

/// P29-008-DIAG-001: 国庫・収入・支出と、その背景(生産・雇用・建設・領土)を月次で記録する。
#[test]
fn p29_008_finance_trace() {
    use strategy_game::building::data::BuildingType as B;
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p29_008f_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    let me = target_id();
    while app.world().resource::<GameDate>().year < 1850 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        if date.day != 1 {
            continue;
        }
        let cr = app.world().resource::<CountryRegistry>();
        let sr = app.world().resource::<StateRegistry>();
        let Some(c) = cr.get(me) else { continue };
        let own: Vec<_> = sr.states.iter().filter(|s| s.owner_country_id == me).collect();
        let lv = |b: B| own.iter().map(|s| s.building_level(b)).sum::<u32>();
        let op = |b: B| {
            own.iter()
                .filter_map(|s| s.building_operation_ratios.get(&b).copied())
                .fold(0.0f32, f32::max)
        };
        let pop: u64 = own.iter().map(|s| s.population).sum();
        let emp: u64 = own.iter().map(|s| s.employed_workforce).sum();
        let ls = if own.is_empty() {
            0.0
        } else {
            own.iter().map(|s| s.living_standard as f64).sum::<f64>() / own.len() as f64
        };
        let st = &c.stockpile;
        println!(
            "FIN {} tr={:.1} inc={:.1} exp={:.1} bal={:.1} tax={:.3} states={} pop={} emp={} ls={:.1} lv[M{} F{} MF{}] op[F={:.2} MF={:.2}] stock[Fe={:.0} IG={:.0} MS={:.0}] queue={}",
            date.display(), c.treasury, c.monthly_income, c.monthly_expenses, c.monthly_balance,
            c.tax_rate, own.len(), pop, emp, ls, lv(B::Mine), lv(B::Factory), lv(B::MilitaryFactory),
            op(B::Factory), op(B::MilitaryFactory),
            st.get(ResourceType::Iron), st.get(ResourceType::IndustrialGoods),
            st.get(ResourceType::MilitarySupplies), c.construction_queue.len()
        );
    }
}
