use crate::app::game_state::GameState;
use crate::common::{ArmyId, DivisionId, StateId};
use crate::country::CountryRegistry;
use crate::map::division_selection::{DragSelectState, SelectedDivision, screen_to_world};
use crate::map::camera::GameCamera;
use crate::military::army::ArmyRegistry;
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use bevy::prelude::*;
use std::collections::HashMap;

#[derive(Component)]
pub struct DivisionVisual {
    pub division_id: DivisionId,
}

/// 師団スプライトの既定(未選択・非戦闘)の本体色。
/// 州・国の塗り色(`CountryData::bevy_color`、`map/rendering.rs`の州スプライトと同じ値)とは
/// 意図的に異なる固定色にすることで、自国と同色の州の上でも師団の存在が視認できるようにする。
/// 所有国の識別は`draw_division_owner_markers`が描く枠線(所有国色)側で行う。
const ARMY_BODY_COLOR: Color = Color::srgb(0.08, 0.08, 0.1);

pub struct DivisionRenderPlugin;

impl Plugin for DivisionRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                sync_division_visuals,
                update_division_visuals,
                draw_division_owner_markers,
                draw_division_paths,
                draw_drag_select_rect,
                draw_battle_overlays,
                draw_occupation_overlays,
            )
                .run_if(in_state(GameState::Playing)),
        );
    }
}

fn sync_division_visuals(
    mut commands: Commands,
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    army_registry: Res<ArmyRegistry>,
    selected_division: Res<SelectedDivision>,
    query: Query<(Entity, &DivisionVisual)>,
) {
    // P21-004: 表示単位は「クラスタの代表DivisionId」(編成所属かつ同州の師団はまとめて1個)。
    let clusters = division_visual_clusters(&military_registry, &state_registry, &army_registry);

    let mut rendered = std::collections::HashSet::new();
    for (entity, visual) in query.iter() {
        if !clusters.contains_key(&visual.division_id) {
            commands.entity(entity).despawn();
        } else {
            rendered.insert(visual.division_id);
        }
    }

    for representative in clusters.keys() {
        if !rendered.contains(representative) {
            let is_selected = selected_division.is_selected(*representative);
            let size = if is_selected {
                Vec2::new(20.0, 20.0)
            } else {
                Vec2::new(14.0, 14.0)
            };

            commands.spawn((
                Sprite {
                    color: ARMY_BODY_COLOR,
                    custom_size: Some(size),
                    ..default()
                },
                Transform::from_xyz(0.0, 0.0, 5.0),
                DivisionVisual {
                    division_id: *representative,
                },
            ));
        }
    }
}

/// 州ごとの重ね表示オフセットを反映した各軍隊の表示座標を計算する。
/// 描画 (`update_division_visuals`) とマップクリック判定 (`division_selection::handle_division_selection`)
/// が同じ座標系を参照できるよう、独立関数として切り出している。
pub fn division_display_positions(
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
) -> std::collections::HashMap<DivisionId, Vec2> {
    // 州ごとにグループ化して複数ユニットの重ね表示をオフセット
    let mut state_division_indices: std::collections::HashMap<crate::common::StateId, Vec<DivisionId>> =
        std::collections::HashMap::new();

    for (division_id, division) in military_registry.divisions.iter() {
        state_division_indices
            .entry(division.current_state)
            .or_default()
            .push(*division_id);
    }
    for list in state_division_indices.values_mut() {
        list.sort_by_key(|id| id.0);
    }

    let mut positions = std::collections::HashMap::new();
    for (division_id, division) in military_registry.divisions.iter() {
        let start_pos = state_registry
            .get(division.current_state)
            .map(|s| s.position())
            .unwrap_or(Vec2::ZERO);

        // 重なり回避用オフセット (同じ州にいる場合のズレ)
        let mut offset = Vec2::ZERO;
        if let Some(divisions_in_state) = state_division_indices.get(&division.current_state)
            && divisions_in_state.len() > 1
            && let Some(idx) = divisions_in_state.iter().position(|&id| id == *division_id)
        {
            let col = (idx % 3) as f32 - 1.0;
            let row = (idx / 3) as f32;
            offset = Vec2::new(col * 7.0, row * 7.0);
        }

        let pos = if let Some(target_state) = division.target_state {
            let target_pos = state_registry
                .get(target_state)
                .map(|s| s.position())
                .unwrap_or(Vec2::ZERO);
            start_pos.lerp(target_pos, division.movement_progress)
        } else {
            start_pos + offset
        };

        positions.insert(*division_id, pos);
    }

    positions
}

/// P21-004: 地図上で1つのアイコンとして扱う単位。編成(Army)所属かつ同じ州にいる
/// 師団同士はまとめて1つのクラスタになる(「編成したら一個の師団に見える」要望への対応)。
/// 内部データ・戦闘・移動命令は個々の師団のまま変わらない — これは表示層のみの集約。
/// 同じ編成でも別の州にいる師団(移動などではぐれた場合)はまとめず個別のクラスタになる
/// (でないと、はぐれた師団が地図上から見えなくなってしまう)。
pub struct DivisionVisualCluster {
    pub position: Vec2,
    /// このクラスタが代表する全DivisionId(DivisionId昇順)。未編成の師団は自分自身のみを含む。
    pub members: Vec<DivisionId>,
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
enum ClusterKey {
    Grouped(StateId, ArmyId),
    Solo(DivisionId),
}

/// 全師団を`DivisionVisualCluster`へ集約する。描画 (`sync_division_visuals`等) とマップクリック判定
/// (`division_selection::handle_division_selection`) が同じクラスタ・座標を参照できるよう、
/// `division_display_positions`と対になる独立関数として切り出している。
pub fn division_visual_clusters(
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
    army_registry: &ArmyRegistry,
) -> HashMap<DivisionId, DivisionVisualCluster> {
    let mut raw_clusters: HashMap<ClusterKey, Vec<DivisionId>> = HashMap::new();
    for (&division_id, division) in military_registry.divisions.iter() {
        let key = match army_registry.army_for_division(division_id) {
            Some(group_id) => ClusterKey::Grouped(division.current_state, group_id),
            None => ClusterKey::Solo(division_id),
        };
        raw_clusters.entry(key).or_default().push(division_id);
    }

    // 代表(クラスタ内最小DivisionId)ごとの所属州・州内での重ね表示オフセット計算用に、
    // 州ごとの代表一覧をまとめる(既存`division_display_positions`と同じロジックをクラスタ単位で適用)。
    let mut state_representatives: HashMap<StateId, Vec<DivisionId>> = HashMap::new();
    let mut cluster_members: HashMap<DivisionId, Vec<DivisionId>> = HashMap::new();
    for mut members in raw_clusters.into_values() {
        members.sort_by_key(|id| id.0);
        let representative = members[0];
        let state = military_registry.divisions[&representative].current_state;
        state_representatives
            .entry(state)
            .or_default()
            .push(representative);
        cluster_members.insert(representative, members);
    }
    for list in state_representatives.values_mut() {
        list.sort_by_key(|id| id.0);
    }

    let mut result = HashMap::new();
    for (representative, members) in cluster_members {
        let division = &military_registry.divisions[&representative];
        let start_pos = state_registry
            .get(division.current_state)
            .map(|s| s.position())
            .unwrap_or(Vec2::ZERO);

        let mut offset = Vec2::ZERO;
        if let Some(reps_in_state) = state_representatives.get(&division.current_state)
            && reps_in_state.len() > 1
            && let Some(idx) = reps_in_state.iter().position(|&id| id == representative)
        {
            let col = (idx % 3) as f32 - 1.0;
            let row = (idx / 3) as f32;
            offset = Vec2::new(col * 7.0, row * 7.0);
        }

        let pos = if let Some(target_state) = division.target_state {
            let target_pos = state_registry
                .get(target_state)
                .map(|s| s.position())
                .unwrap_or(Vec2::ZERO);
            start_pos.lerp(target_pos, division.movement_progress)
        } else {
            start_pos + offset
        };

        result.insert(
            representative,
            DivisionVisualCluster {
                position: pos,
                members,
            },
        );
    }

    result
}

fn update_division_visuals(
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    army_registry: Res<ArmyRegistry>,
    selected_division: Res<SelectedDivision>,
    mut query: Query<(&DivisionVisual, &mut Transform, &mut Sprite)>,
) {
    let clusters = division_visual_clusters(&military_registry, &state_registry, &army_registry);

    for (visual, mut transform, mut sprite) in query.iter_mut() {
        let Some(cluster) = clusters.get(&visual.division_id) else {
            continue;
        };
        let is_selected = cluster
            .members
            .iter()
            .any(|id| selected_division.is_selected(*id));
        let is_fighting = cluster.members.iter().any(|id| {
            military_registry
                .divisions
                .get(id)
                .is_some_and(|a| a.status == DivisionStatus::Fighting)
        });
        // P21-004: 複数師団をまとめて表す代表アイコンは、単一師団と見分けが付くよう
        // 一回り大きく表示する(「編成したら一個の師団に見える」要望の一部)。
        let merged_bonus = if cluster.members.len() > 1 { 4.0 } else { 0.0 };

        transform.translation.x = cluster.position.x;
        transform.translation.y = cluster.position.y;
        transform.translation.z = if is_selected { 6.0 } else { 5.0 };

        if is_fighting {
            // 戦闘中: 赤みがかった色
            sprite.custom_size = Some(Vec2::splat(16.0 + merged_bonus));
            sprite.color = Color::srgb(1.0, 0.2, 0.2);
        } else if is_selected {
            // 強調表示: サイズ拡大 & 黄金色
            sprite.custom_size = Some(Vec2::splat(20.0 + merged_bonus));
            sprite.color = Color::srgb(1.0, 0.9, 0.3);
        } else {
            // 既定色: 州・国の塗り色とは別の固定色(ARMY_BODY_COLOR)。
            // 所有国は`draw_division_owner_markers`が描く枠線側で示す。
            sprite.custom_size = Some(Vec2::splat(14.0 + merged_bonus));
            sprite.color = ARMY_BODY_COLOR;
        }
    }
}

/// P21-003: 師団本体の色を州・国の塗り色と分離した(`ARMY_BODY_COLOR`)ため、
/// どの国の師団かを示す枠線を所有国色で描画する。`draw_battle_overlays`/
/// `draw_occupation_overlays`と同じ4本の`line_2d`による矩形枠パターン。
fn draw_division_owner_markers(
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    country_registry: Res<CountryRegistry>,
    army_registry: Res<ArmyRegistry>,
    selected_division: Res<SelectedDivision>,
    mut gizmos: Gizmos,
) {
    let clusters = division_visual_clusters(&military_registry, &state_registry, &army_registry);

    for (representative, cluster) in clusters.iter() {
        let pos = cluster.position;
        let Some(division) = military_registry.divisions.get(representative) else {
            continue;
        };

        let owner_color = country_registry
            .get(division.owner)
            .map(|c| c.bevy_color())
            .unwrap_or(Color::WHITE);

        let is_selected = cluster
            .members
            .iter()
            .any(|id| selected_division.is_selected(*id));
        let half = if is_selected { 11.0 } else { 8.0 }
            + if cluster.members.len() > 1 { 2.0 } else { 0.0 };

        gizmos.line_2d(
            pos + Vec2::new(-half, -half),
            pos + Vec2::new(half, -half),
            owner_color,
        );
        gizmos.line_2d(
            pos + Vec2::new(half, -half),
            pos + Vec2::new(half, half),
            owner_color,
        );
        gizmos.line_2d(
            pos + Vec2::new(half, half),
            pos + Vec2::new(-half, half),
            owner_color,
        );
        gizmos.line_2d(
            pos + Vec2::new(-half, half),
            pos + Vec2::new(-half, -half),
            owner_color,
        );
    }
}

/// 選択中ユニット(複数可)の移動経路を Gizmos でライン描画
fn draw_division_paths(
    selected_division: Res<SelectedDivision>,
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    mut gizmos: Gizmos,
) {
    for division_id in selected_division.sorted_ids() {
        let Some(division) = military_registry.divisions.get(&division_id) else {
            continue;
        };

        let start_pos = state_registry
            .get(division.current_state)
            .map(|s| s.position())
            .unwrap_or(Vec2::ZERO);

        let mut current_pos = if let Some(target_state) = division.target_state {
            let target_pos = state_registry
                .get(target_state)
                .map(|s| s.position())
                .unwrap_or(Vec2::ZERO);
            start_pos.lerp(target_pos, division.movement_progress)
        } else {
            start_pos
        };

        // target_state への線
        if let Some(target_state) = division.target_state
            && let Some(target_st) = state_registry.get(target_state)
        {
            let next_pos = target_st.position();
            gizmos.line_2d(current_pos, next_pos, Color::srgb(0.2, 0.9, 0.3));
            current_pos = next_pos;
        }

        // current_path の残りのノードへの線
        for &path_state_id in &division.current_path {
            if let Some(st) = state_registry.get(path_state_id) {
                let next_pos = st.position();
                gizmos.line_2d(current_pos, next_pos, Color::srgb(0.9, 0.8, 0.2));
                current_pos = next_pos;
            }
        }

        // 目的地のハイライトマーク
        if division.destination.is_some() || division.target_state.is_some() {
            gizmos.circle_2d(current_pos, 8.0, Color::srgb(0.9, 0.2, 0.2));
        }
    }
}

/// P21-004: ドラッグ選択中の矩形をGizmosで可視化する。実際の選択判定
/// (`division_selection::handle_division_selection`)からはこの描画部分だけを分離してある。
/// `Gizmos`はGizmoPluginが提供するリソースを要求し`MinimalPlugins`には含まれないため、
/// `handle_division_selection`をそのまま単体テストできるようにする狙い。
fn draw_drag_select_rect(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_q: Query<&Transform, With<GameCamera>>,
    drag_state: Res<DragSelectState>,
    mut gizmos: Gizmos,
) {
    if !drag_state.is_dragging || !mouse_buttons.pressed(MouseButton::Left) {
        return;
    }
    let Some(start_screen) = drag_state.press_start_screen else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(cam_transform) = camera_q.single() else {
        return;
    };
    let Some(current_screen) = window.cursor_position() else {
        return;
    };
    let window_size = Vec2::new(window.width(), window.height());

    let start_world = screen_to_world(start_screen, window_size, cam_transform);
    let current_world = screen_to_world(current_screen, window_size, cam_transform);
    let min = start_world.min(current_world);
    let max = start_world.max(current_world);
    let color = Color::srgba(0.9, 0.9, 0.3, 0.9);
    gizmos.line_2d(Vec2::new(min.x, min.y), Vec2::new(max.x, min.y), color);
    gizmos.line_2d(Vec2::new(max.x, min.y), Vec2::new(max.x, max.y), color);
    gizmos.line_2d(Vec2::new(max.x, max.y), Vec2::new(min.x, max.y), color);
    gizmos.line_2d(Vec2::new(min.x, max.y), Vec2::new(min.x, min.y), color);
}

/// 戦闘中の地域を Gizmos でハイライト表示（赤い枠）
fn draw_battle_overlays(
    battle_registry: Res<BattleRegistry>,
    state_registry: Res<StateRegistry>,
    mut gizmos: Gizmos,
) {
    for battle in battle_registry.battles.values() {
        if battle.status != BattleStatus::Ongoing {
            continue;
        }
        if let Some(state) = state_registry.get(battle.state_id) {
            let pos = state.position();
            let size = state.rect_size();
            let half = size * 0.5;
            // 赤い枠で戦闘中を表示
            gizmos.line_2d(
                pos + Vec2::new(-half.x, -half.y),
                pos + Vec2::new(half.x, -half.y),
                Color::srgb(1.0, 0.1, 0.1),
            );
            gizmos.line_2d(
                pos + Vec2::new(half.x, -half.y),
                pos + Vec2::new(half.x, half.y),
                Color::srgb(1.0, 0.1, 0.1),
            );
            gizmos.line_2d(
                pos + Vec2::new(half.x, half.y),
                pos + Vec2::new(-half.x, half.y),
                Color::srgb(1.0, 0.1, 0.1),
            );
            gizmos.line_2d(
                pos + Vec2::new(-half.x, half.y),
                pos + Vec2::new(-half.x, -half.y),
                Color::srgb(1.0, 0.1, 0.1),
            );
        }
    }
}

/// 所有国と支配国が異なる地域（占領地）を Gizmos でハイライト表示（黄色の枠）
fn draw_occupation_overlays(state_registry: Res<StateRegistry>, mut gizmos: Gizmos) {
    for state in &state_registry.states {
        let controller = state.controller();
        if controller == state.owner_country_id {
            continue; // 所有国=支配国なら表示不要
        }

        let pos = state.position();
        let size = state.rect_size();
        let half = size * 0.5 + Vec2::new(2.0, 2.0); // 少し外側

        let color = Color::srgb(1.0, 0.85, 0.1);
        gizmos.line_2d(
            pos + Vec2::new(-half.x, -half.y),
            pos + Vec2::new(half.x, -half.y),
            color,
        );
        gizmos.line_2d(
            pos + Vec2::new(half.x, -half.y),
            pos + Vec2::new(half.x, half.y),
            color,
        );
        gizmos.line_2d(
            pos + Vec2::new(half.x, half.y),
            pos + Vec2::new(-half.x, half.y),
            color,
        );
        gizmos.line_2d(
            pos + Vec2::new(-half.x, half.y),
            pos + Vec2::new(-half.x, -half.y),
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DivisionDefinitionId, DivisionId};
    use crate::military::data::{DivisionStatus, Division, DivisionSize, DivisionType};
    use crate::state::data::StateData;

    fn make_test_division(id: usize, owner: CountryId, state: StateId) -> Division {
        Division {
            id: DivisionId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: state,
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 10_000,
            max_manpower: 10_000,
            equipment: 100.0,
            max_equipment: 100.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 100.0,
            max_morale: 100.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn state_at(id: usize, pos: Vec2) -> StateData {
        StateData {
            id: StateId(id),
            world_position: [pos.x, pos.y],
            ..Default::default()
        }
    }

    /// 編成に属さない師団は、これまで通り1師団=1クラスタになる
    /// (`division_display_positions`と同じ座標を返す)。
    #[test]
    fn ungrouped_divisions_form_solo_clusters() {
        let mut military_registry = MilitaryRegistry::default();
        let a1 = make_test_division(1, CountryId(1), StateId(1));
        let a2 = make_test_division(2, CountryId(1), StateId(1));
        military_registry.divisions.insert(a1.id, a1);
        military_registry.divisions.insert(a2.id, a2);

        let state_registry = StateRegistry::build(vec![state_at(1, Vec2::ZERO)]);
        let army_registry = ArmyRegistry::default();

        let clusters =
            division_visual_clusters(&military_registry, &state_registry, &army_registry);
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[&DivisionId(1)].members, vec![DivisionId(1)]);
        assert_eq!(clusters[&DivisionId(2)].members, vec![DivisionId(2)]);
    }

    /// 編成に属し、かつ同じ州にいる師団は1クラスタにまとめられる。
    #[test]
    fn grouped_divisions_in_same_state_merge_into_one_cluster() {
        let mut military_registry = MilitaryRegistry::default();
        let a1 = make_test_division(1, CountryId(1), StateId(1));
        let a2 = make_test_division(2, CountryId(1), StateId(1));
        military_registry.divisions.insert(a1.id, a1);
        military_registry.divisions.insert(a2.id, a2);

        let mut army_registry = ArmyRegistry::default();
        army_registry
            .create_army(CountryId(1), &[DivisionId(1), DivisionId(2)], &military_registry)
            .unwrap();

        let state_registry = StateRegistry::build(vec![state_at(1, Vec2::ZERO)]);
        let clusters =
            division_visual_clusters(&military_registry, &state_registry, &army_registry);

        assert_eq!(
            clusters.len(),
            1,
            "grouped same-state divisions must merge into one cluster"
        );
        let cluster = clusters.values().next().unwrap();
        assert_eq!(cluster.members, vec![DivisionId(1), DivisionId(2)]);
    }

    /// 同じ編成でも州が異なれば、はぐれた師団として個別のクラスタのままになる
    /// (地図上から見えなくなることを防ぐ)。
    #[test]
    fn grouped_divisions_in_different_states_stay_separate_clusters() {
        let mut military_registry = MilitaryRegistry::default();
        let a1 = make_test_division(1, CountryId(1), StateId(1));
        let a2 = make_test_division(2, CountryId(1), StateId(2));
        military_registry.divisions.insert(a1.id, a1);
        military_registry.divisions.insert(a2.id, a2);

        let mut army_registry = ArmyRegistry::default();
        army_registry
            .create_army(CountryId(1), &[DivisionId(1), DivisionId(2)], &military_registry)
            .unwrap();

        let state_registry = StateRegistry::build(vec![
            state_at(1, Vec2::new(0.0, 0.0)),
            state_at(2, Vec2::new(300.0, 0.0)),
        ]);
        let clusters =
            division_visual_clusters(&military_registry, &state_registry, &army_registry);

        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[&DivisionId(1)].members, vec![DivisionId(1)]);
        assert_eq!(clusters[&DivisionId(2)].members, vec![DivisionId(2)]);
    }
}
