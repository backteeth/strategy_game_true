use bevy::log::{DEFAULT_FILTER, LogPlugin};
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::building::BuildingPlugin;
use strategy_game::country::CountryPlugin;
use strategy_game::debug::DebugPlugin;
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::logistics::LogisticsPlugin;
use strategy_game::map::MapPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin};
use strategy_game::state::StatePlugin;
use strategy_game::ui::UiPlugin;
use strategy_game::war::WarPlugin;

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Grand Strategy - Prototype v0.3".to_string(),
                        resolution: (1280u32, 720u32).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(LogPlugin {
                    // icu_provider(bevy_textが内部で使うParleyの依存)は、日本語向けの
                    // 辞書ベース分節(セグメンテーション)モデルが同梱されていないため、
                    // "No segmentation model for language: ja" という警告をwarnレベルで
                    // 大量に出力する。これは行分割(word-wrap)がフォールバックに
                    // なるだけで、文字のグリフ表示自体には影響しない既知の無害な警告
                    // (P20-009調査済み)なので、ログノイズとして抑制する。
                    filter: format!("{DEFAULT_FILTER}icu_provider=error,"),
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
        .add_plugins(ResearchPlugin)
        .add_plugins(PoliticsPlugin)
        .add_plugins(DiplomacyPlugin)
        .add_plugins(MilitaryPlugin)
        .add_plugins(WarPlugin)
        .add_plugins(MapPlugin)
        .add_plugins(UiPlugin)
        .add_plugins(DebugPlugin)
        // P21-SAVE-002E: 単一スロットのセーブ／ロード導線。ここまでの全プラグインが
        // 登録した後に追加する(SaveGameResourceParams/apply::prepare_loadが読む
        // 全Resourceが出揃っている必要があるため)。起動だけでは一切ファイルI/Oを
        // 行わない(トップバーのボタン操作でSaveRequestMessage/LoadRequestMessageが
        // 発行された場合だけPostUpdateで動作する)。
        .add_plugins(SaveGamePlugin)
        .add_plugins(LoadGamePlugin)
        .run();
}
