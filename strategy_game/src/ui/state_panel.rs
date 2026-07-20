use crate::app::game_state::GameState;
use crate::country::CountryRegistry;
use crate::state::{SelectedState, StateSelectionChanged, data::StateRegistry};
/// 州情報パネルUIモジュール
/// 選択中の州情報を画面右側に表示する
/// Note: ICU4X日本語セグメントエラー回避のためUI表示テキストはASCII使用
use bevy::prelude::*;

/// UIパネルのルートエンティティを識別するマーカー
#[derive(Component)]
pub struct StatePanelRoot;

/// パネル内のテキストを識別するマーカー
#[derive(Component)]
pub struct StatePanelText;

/// 州情報パネルのUIプラグイン
pub struct StatePanelPlugin;

impl Plugin for StatePanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), setup_state_panel)
            .add_systems(
                Update,
                update_state_panel.run_if(in_state(GameState::Playing)),
            );
    }
}

/// ゲーム開始時にUIパネルを生成する
fn setup_state_panel(mut commands: Commands) {
    // 右側のパネルコンテナ
    commands
        .spawn((
            StatePanelRoot,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(260.0),
                height: Val::Percent(100.0),
                padding: UiRect::all(Val::Px(16.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.1, 0.85)),
        ))
        .with_children(|parent| {
            // パネルタイトル（ASCII）
            parent.spawn((
                Text::new("-- Province Info --"),
                TextColor(Color::srgb(0.9, 0.85, 0.6)),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
            ));

            // 情報テキスト（初期は未選択メッセージ、ASCII）
            parent.spawn((
                StatePanelText,
                Text::new("Click a province to select"),
                TextColor(Color::srgb(0.8, 0.8, 0.85)),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
            ));
        });
}

/// 州の選択状態が変化したときだけパネルのテキストを更新する
fn update_state_panel(
    mut selection_changed: MessageReader<StateSelectionChanged>,
    selected: Res<SelectedState>,
    state_registry: Res<StateRegistry>,
    country_registry: Res<CountryRegistry>,
    mut text_q: Query<&mut Text, With<StatePanelText>>,
) {
    // 選択変更メッセージがなければ何もしない
    if selection_changed.is_empty() {
        return;
    }
    selection_changed.clear();

    let Ok(mut text) = text_q.single_mut() else {
        return;
    };

    match selected.0 {
        None => {
            *text = Text::new("Click a province to select");
        }
        Some(state_id) => {
            if let Some(state) = state_registry.get(state_id) {
                let country_name = country_registry
                    .get(state.owner_country_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");

                let population_str = format_population(state.population);

                // 首都判定
                let is_capital = country_registry
                    .get(state.owner_country_id)
                    .map(|c| c.capital_state_id == state_id)
                    .unwrap_or(false);
                let capital_str = if is_capital { " (Capital)" } else { "" };

                // すべてASCII文字列で構成（ICU4Xエラー回避）
                let info = format!(
                    "Province: {}{}\nID: {}\nOwner: {}\nPop: {}\nWorkforce: {:.0}%\nEducation: {:.0}%\nLiving Std: {:.0}%\nUnrest: {:.0}%",
                    state.name,
                    capital_str,
                    state_id.0,
                    country_name,
                    population_str,
                    state.workforce * 100.0,
                    state.education * 100.0,
                    state.living_standard * 100.0,
                    state.unrest * 100.0,
                );
                *text = Text::new(info);
            }
        }
    }
}

/// 人口を読みやすい形式にフォーマットするヘルパー
pub fn format_population(pop: u64) -> String {
    if pop >= 1_000_000 {
        format!("{:.1}M", pop as f64 / 1_000_000.0)
    } else if pop >= 1_000 {
        format!("{:.0}K", pop as f64 / 1_000.0)
    } else {
        format!("{}", pop)
    }
}
