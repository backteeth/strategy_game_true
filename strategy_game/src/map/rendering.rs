use crate::app::game_state::GameState;
use crate::common::StateId;
use crate::country::CountryRegistry;
use crate::state::data::StateRegistry;
/// マップ描画モジュール
/// 海の背景、州の矩形スプライトを生成・管理する
/// 将来的に州ID画像やポリゴン描画へ差し替えられる設計にする
use bevy::prelude::*;

/// 州を表示するエンティティのマーカーコンポーネント
/// StateId を保持することで、クリック判定やUI更新に使用する
#[derive(Component, Debug, Clone, Copy)]
pub struct StateVisual {
    pub state_id: StateId,
    /// 州の矩形サイズ（クリック判定に使用）
    pub size: Vec2,
    /// 基本色（選択前の色）
    pub base_color: Color,
}

/// マップ描画の定数
const SEA_COLOR: Color = Color::srgb(0.1, 0.3, 0.55);
const MAP_WIDTH: f32 = 2400.0;
const MAP_HEIGHT: f32 = 1600.0;

/// マップ描画プラグイン（MapPlugin から呼び出す）
pub struct RenderingPlugin;

impl Plugin for RenderingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), setup_map);
    }
}

/// Playing 状態に入ったときにマップを生成する
fn setup_map(
    mut commands: Commands,
    state_registry: Res<StateRegistry>,
    country_registry: Res<CountryRegistry>,
) {
    // --- 海の背景 ---
    commands.spawn((
        Sprite {
            color: SEA_COLOR,
            custom_size: Some(Vec2::new(MAP_WIDTH, MAP_HEIGHT)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    // --- 各州のスプライトを生成 ---
    // 将来的にはここを州ID画像やポリゴンに差し替える
    for state_data in &state_registry.states {
        // 所有国から色を取得（見つからなければグレー）
        let base_color = country_registry
            .get(state_data.owner_country_id)
            .map(|c| c.bevy_color())
            .unwrap_or(Color::srgb(0.5, 0.5, 0.5));

        let pos = state_data.position();
        let size = state_data.rect_size();

        // 州スプライト本体（Z=1.0 で海の上に描画）
        commands.spawn((
            Sprite {
                color: base_color,
                custom_size: Some(size),
                ..default()
            },
            Transform::from_xyz(pos.x, pos.y, 1.0),
            StateVisual {
                state_id: state_data.id,
                size,
                base_color,
            },
        ));

        // 州の枠線（暗い色のスプライトを少し大きく描画して重ねる）
        let border_size = size + Vec2::new(4.0, 4.0);
        commands.spawn((
            Sprite {
                color: Color::srgba(0.0, 0.0, 0.0, 0.6),
                custom_size: Some(border_size),
                ..default()
            },
            Transform::from_xyz(pos.x, pos.y, 0.9),
        ));
    }
}
