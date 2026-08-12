use crate::app::game_state::GameState;
use crate::common::StateId;
use crate::country::CountryRegistry;
use crate::map::selection::brighten_color;
use crate::state::SelectedState;
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
        app.add_systems(OnEnter(GameState::Playing), setup_map)
            .add_systems(
                Update,
                update_state_colors_on_controller_change.run_if(in_state(GameState::Playing)),
            );
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

/// 州の実効支配国(controller)が変わった際に、州スプライトの色を新しい支配国の色へ
/// 更新する。占領した州がすぐに占領側の色に変わるようにする(従来は`owner_country_id`
/// から一度だけ色を決めていたため、占領しても所有国の色のまま変化しなかった)。
/// `StateRegistry`が変化したフレームのみ走査する(戦闘決着・講和・州の支配権変更等)。
fn update_state_colors_on_controller_change(
    state_registry: Res<StateRegistry>,
    country_registry: Res<CountryRegistry>,
    selected: Res<SelectedState>,
    mut state_visuals_q: Query<(&mut Sprite, &mut StateVisual)>,
) {
    if !state_registry.is_changed() {
        return;
    }

    for (mut sprite, mut visual) in state_visuals_q.iter_mut() {
        let Some(state) = state_registry.get(visual.state_id) else {
            continue;
        };

        let new_color = country_registry
            .get(state.controller())
            .map(|c| c.bevy_color())
            .unwrap_or(Color::srgb(0.5, 0.5, 0.5));

        if new_color == visual.base_color {
            continue;
        }
        visual.base_color = new_color;

        sprite.color = if Some(visual.state_id) == selected.0 {
            brighten_color(new_color, 1.5)
        } else {
            new_color
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::CountryData;
    use crate::state::data::StateData;

    fn country(id: usize, color: [f32; 3]) -> CountryData {
        CountryData {
            id: crate::common::CountryId(id),
            map_color: color,
            ..Default::default()
        }
    }

    fn build_app(state: StateData, countries: Vec<CountryData>) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, update_state_colors_on_controller_change);

        let owner_color = countries
            .iter()
            .find(|c| c.id == state.owner_country_id)
            .map(|c| c.bevy_color())
            .unwrap();
        let state_id = state.id;

        app.insert_resource(StateRegistry::build(vec![state]));
        app.insert_resource(CountryRegistry { countries });
        app.init_resource::<SelectedState>();

        app.world_mut().spawn((
            Sprite {
                color: owner_color,
                ..default()
            },
            StateVisual {
                state_id,
                size: Vec2::new(10.0, 10.0),
                base_color: owner_color,
            },
        ));

        app
    }

    /// 占領(controllerが所有国と異なる国に変わる)により、州スプライトの色が
    /// 占領側の色へ更新されることを検証する(元は所有国の色に固定だった)。
    #[test]
    fn state_sprite_color_updates_to_controller_on_occupation() {
        let owner = crate::common::CountryId(1);
        let occupier = crate::common::CountryId(2);
        let state_id = StateId(1);

        let state = StateData {
            id: state_id,
            owner_country_id: owner,
            ..Default::default()
        };
        let countries = vec![country(1, [1.0, 0.0, 0.0]), country(2, [0.0, 1.0, 0.0])];
        let mut app = build_app(state, countries);
        app.update();

        // 占領前: 所有国(赤)の色のまま
        {
            let sprite = app
                .world_mut()
                .query::<&Sprite>()
                .single(app.world())
                .unwrap();
            assert_eq!(sprite.color, Color::srgb(1.0, 0.0, 0.0));
        }

        // 占領: controller_countryを占領側に変更
        {
            let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
            state_registry.get_mut(state_id).unwrap().controller_country = Some(occupier);
        }
        app.update();

        // 占領後: 占領側(緑)の色に変わる
        let sprite = app
            .world_mut()
            .query::<&Sprite>()
            .single(app.world())
            .unwrap();
        assert_eq!(sprite.color, Color::srgb(0.0, 1.0, 0.0));
        let visual = app
            .world_mut()
            .query::<&StateVisual>()
            .single(app.world())
            .unwrap();
        assert_eq!(visual.base_color, Color::srgb(0.0, 1.0, 0.0));
    }

    /// 選択中の州が占領された場合、通常色ではなく選択ハイライト(明るい版)が
    /// 適用されることを検証する。
    #[test]
    fn selected_state_keeps_highlight_after_controller_change() {
        let owner = crate::common::CountryId(1);
        let occupier = crate::common::CountryId(2);
        let state_id = StateId(1);

        let state = StateData {
            id: state_id,
            owner_country_id: owner,
            ..Default::default()
        };
        let countries = vec![country(1, [1.0, 0.0, 0.0]), country(2, [0.0, 1.0, 0.0])];
        let mut app = build_app(state, countries);
        app.insert_resource(SelectedState(Some(state_id)));
        app.update();

        {
            let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
            state_registry.get_mut(state_id).unwrap().controller_country = Some(occupier);
        }
        app.update();

        let sprite = app
            .world_mut()
            .query::<&Sprite>()
            .single(app.world())
            .unwrap();
        assert_eq!(
            sprite.color,
            brighten_color(Color::srgb(0.0, 1.0, 0.0), 1.5)
        );
    }
}
