use bevy::log::LogPlugin;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::building::BuildingPlugin;
use strategy_game::country::CountryPlugin;
use strategy_game::debug::DebugPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::logistics::LogisticsPlugin;
use strategy_game::map::MapPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::state::StatePlugin;
use strategy_game::ui::UiPlugin;

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Grand Strategy - Prototype v0.2".to_string(),
                        resolution: (1280u32, 720u32).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(LogPlugin {
                    filter: "wgpu=error,bevy_ecs=warn,icu=off,icu_provider=off,icu_segmenter=off,icu_properties=off".to_string(),
                    level: bevy::log::Level::INFO,
                    ..default()
                }),
        )
        .add_plugins(AppPlugin)
        .add_plugins(CountryPlugin)
        .add_plugins(StatePlugin)
        .add_plugins(BuildingPlugin)
        .add_plugins(PopulationPlugin)
        .add_plugins(LogisticsPlugin)
        .add_plugins(EconomyPlugin)
        .add_plugins(MapPlugin)
        .add_plugins(UiPlugin)
        .add_plugins(DebugPlugin)
        .run();
}
