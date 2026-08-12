use crate::app::game_state::GameState;
use crate::common::DivisionId;
use crate::country::PlayerCountry;
use crate::map::division_render::division_visual_clusters;
use crate::map::camera::GameCamera;
use crate::military::army::ArmyRegistry;
use crate::military::battle::BattleRegistry;
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::state::SelectedState;
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;
use bevy::prelude::*;
use std::collections::HashSet;

/// 選択中の陸軍(複数可)。Ctrl+クリックで追加/解除できる。
#[derive(Resource, Default, Debug)]
pub struct SelectedDivision {
    pub division_ids: HashSet<DivisionId>,
}

impl SelectedDivision {
    pub fn is_selected(&self, id: DivisionId) -> bool {
        self.division_ids.contains(&id)
    }

    /// 選択を`id`単体へ置き換える(通常クリック)。
    pub fn select_only(&mut self, id: DivisionId) {
        self.division_ids.clear();
        self.division_ids.insert(id);
    }

    /// `id`が選択中なら解除し、未選択なら追加する(Ctrl+クリック)。
    pub fn toggle(&mut self, id: DivisionId) {
        if !self.division_ids.remove(&id) {
            self.division_ids.insert(id);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.division_ids.is_empty()
    }

    pub fn len(&self) -> usize {
        self.division_ids.len()
    }

    /// 単一選択を前提とする従来コードとの互換用: 選択中で最小のDivisionIdを返す。
    /// 複数選択時は代表として1件だけ表示・判定したい箇所で使う。
    pub fn primary(&self) -> Option<DivisionId> {
        self.division_ids.iter().min_by_key(|id| id.0).copied()
    }

    /// DivisionId昇順でソート済みのVecを返す(表示・判定の決定性を保つため)。
    pub fn sorted_ids(&self) -> Vec<DivisionId> {
        let mut ids: Vec<DivisionId> = self.division_ids.iter().copied().collect();
        ids.sort_by_key(|id| id.0);
        ids
    }
}

/// P21-004: 左ドラッグによる矩形選択の状態。単発クリックとドラッグを区別するため、
/// 押下時点では確定させず、`DRAG_THRESHOLD_PX`を超えて動いた場合のみドラッグ扱いにする。
/// `map::selection::handle_state_click`もこれを参照し、ドラッグ中/直後は州選択を
/// 発生させないようにする(押下位置がたまたま州の上でも、ドラッグ選択の開始点として
/// 誤って州が選ばれてしまうのを防ぐ)。
#[derive(Resource, Default, Debug)]
pub struct DragSelectState {
    /// `map::division_render::draw_drag_select_rect`がドラッグ中の矩形を描画するために参照する。
    pub(crate) press_start_screen: Option<Vec2>,
    pub is_dragging: bool,
}

/// ドラッグとみなす最小移動量(スクリーンピクセル)。単発クリックの手ブレを吸収する。
const DRAG_THRESHOLD_PX: f32 = 6.0;

/// P21-004: `map::division_render::draw_drag_select_rect`からも同じ変換式を使うため`pub(crate)`。
pub(crate) fn screen_to_world(screen: Vec2, window_size: Vec2, cam_transform: &Transform) -> Vec2 {
    let ndc = (screen / window_size) * 2.0 - Vec2::ONE;
    let ndc = Vec2::new(ndc.x, -ndc.y);
    let scale = cam_transform.scale.x;
    let half_size = window_size * 0.5 * scale;
    cam_transform.translation.xy() + ndc * half_size
}

pub struct DivisionSelectionPlugin;

impl Plugin for DivisionSelectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SelectedDivision>()
            .init_resource::<DragSelectState>()
            .add_systems(
                Update,
                (
                    handle_division_selection,
                    handle_movement_order,
                    prune_selected_division,
                )
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

/// P21-003監査で発見: 撃破・消滅した師団のDivisionIdが`SelectedDivision`に残り続け、
/// UI上の選択数表示が実際より多くなる不具合の修正。`war::frontline::FrontlineRegistry::
/// sanitize_references`と同じ考え方で、`MilitaryRegistry`に存在しないIDを毎フレーム除去する。
fn prune_selected_division(
    military_registry: Res<MilitaryRegistry>,
    mut selected_division: ResMut<SelectedDivision>,
) {
    if !military_registry.is_changed() {
        return;
    }
    selected_division
        .division_ids
        .retain(|id| military_registry.divisions.contains_key(id));
}

/// 左クリック/左ドラッグで師団を選択する。
/// - 単発クリック(移動量が`DRAG_THRESHOLD_PX`未満): 従来通りクリック位置に最も近い
///   1師団を選択(Ctrl+クリックで追加/解除)。
/// - ドラッグ(押下→`DRAG_THRESHOLD_PX`超の移動→解放): 矩形内の全師団を選択
///   (Ctrl押下時は既存選択に追加、非押下時は矩形内の師団だけに置き換える。
///   矩形内が空でCtrl非押下なら選択解除になる)。
#[allow(clippy::too_many_arguments)]
fn handle_division_selection(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window>,
    camera_q: Query<&Transform, With<GameCamera>>,
    player_country: Res<PlayerCountry>,
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    army_registry: Res<ArmyRegistry>,
    ui_interactions_q: Query<&Interaction>,
    mut selected_division: ResMut<SelectedDivision>,
    mut selected_state: ResMut<SelectedState>,
    mut drag_state: ResMut<DragSelectState>,
) {
    let ui_blocked = ui_interactions_q
        .iter()
        .any(|i| *i == Interaction::Hovered || *i == Interaction::Pressed);

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(cam_transform) = camera_q.single() else {
        return;
    };
    // P21-003監査で発見: 所有者チェックがなく敵国師団も選択できてしまっていた不具合の修正。
    // 自国が未確定(国選択前など)の場合は師団選択自体を行わない。
    let Some(player_cid) = player_country.0 else {
        return;
    };
    let window_size = Vec2::new(window.width(), window.height());

    if mouse_buttons.just_pressed(MouseButton::Left) {
        // UI操作中に始まった押下はドラッグ選択の開始点として扱わない
        drag_state.press_start_screen = if ui_blocked {
            None
        } else {
            window.cursor_position()
        };
        drag_state.is_dragging = false;
        return;
    }

    if mouse_buttons.pressed(MouseButton::Left) {
        let (Some(start), Some(current)) =
            (drag_state.press_start_screen, window.cursor_position())
        else {
            return;
        };
        if !drag_state.is_dragging && start.distance(current) > DRAG_THRESHOLD_PX {
            drag_state.is_dragging = true;
        }
        // ドラッグ中の矩形の可視化は`division_render::draw_drag_select_rect`が担う
        // (`Gizmos`はGizmoPluginの提供するリソースを要求しMinimalPluginsに
        // 含まれないため、単体テスト可能なこのSystemからは分離している)。
        return;
    }

    if !mouse_buttons.just_released(MouseButton::Left) {
        return;
    }

    let Some(start_screen) = drag_state.press_start_screen.take() else {
        return;
    };
    // `is_dragging`はここではリセットしない: `map::selection::handle_state_click`が
    // 同じフレームの解放イベントでこの値を参照する(システム実行順に依存しないよう、
    // 次の押下(just_pressed)まで値を保持し、そこで改めてfalseにする)。
    let was_dragging = drag_state.is_dragging;

    if ui_blocked {
        return;
    }
    let Some(end_screen) = window.cursor_position() else {
        return;
    };

    let ctrl_held = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);

    // 同一州に複数師団が駐留する場合、描画側 (division_render::division_visual_clusters) と
    // 同じ座標を使って判定することで、地図上に見えているアイコンとクリック判定を一致させる。
    // P21-004: 編成(Army)所属かつ同州の師団は1アイコンにまとめられているため、
    // それをクリックすると所属師団がまとめて選択される(=見た目通り「1個の師団」として扱える)。
    let clusters = division_visual_clusters(&military_registry, &state_registry, &army_registry);
    let cluster_is_own = |cluster: &crate::map::division_render::DivisionVisualCluster| {
        cluster
            .members
            .first()
            .and_then(|id| military_registry.divisions.get(id))
            .is_some_and(|a| a.owner == player_cid)
    };

    if was_dragging {
        // 矩形選択: 矩形内にアイコンがあるクラスタの全メンバーを収集
        let start_world = screen_to_world(start_screen, window_size, cam_transform);
        let end_world = screen_to_world(end_screen, window_size, cam_transform);
        let min = start_world.min(end_world);
        let max = start_world.max(end_world);

        let hit_members: Vec<DivisionId> = clusters
            .values()
            .filter(|cluster| {
                cluster_is_own(cluster)
                    && cluster.position.x >= min.x
                    && cluster.position.x <= max.x
                    && cluster.position.y >= min.y
                    && cluster.position.y <= max.y
            })
            .flat_map(|cluster| cluster.members.iter().copied())
            .collect();

        if !ctrl_held {
            selected_division.division_ids.clear();
        }
        for division_id in hit_members {
            selected_division.division_ids.insert(division_id);
        }
        selected_state.0 = None;
    } else {
        // 単発クリック: クリック位置に最も近いクラスタ(アイコン)を選ぶ
        let world_pos = screen_to_world(end_screen, window_size, cam_transform);
        let division_radius = 16.0;

        let mut hit_cluster = None;
        let mut hit_distance = f32::MAX;
        for cluster in clusters.values() {
            // P21-003監査で発見: 所有者チェックがなく敵国師団も選択できてしまっていた不具合の修正。
            // 自国以外のクラスタはそもそも候補に入れない。
            if !cluster_is_own(cluster) {
                continue;
            }
            let distance = world_pos.distance(cluster.position);
            if distance < division_radius && distance < hit_distance {
                hit_distance = distance;
                hit_cluster = Some(cluster);
            }
        }

        if let Some(cluster) = hit_cluster {
            if ctrl_held {
                // Ctrl+クリック: クラスタ全メンバーをまとめてトグル(全員選択済みなら全員解除、
                // そうでなければ全員追加)。見た目上「1個の師団」として扱う一貫性のため、
                // メンバーの一部だけを選択/解除する中途半端な状態にはしない。
                let all_selected = cluster
                    .members
                    .iter()
                    .all(|id| selected_division.is_selected(*id));
                for &division_id in &cluster.members {
                    if all_selected {
                        selected_division.division_ids.remove(&division_id);
                    } else {
                        selected_division.division_ids.insert(division_id);
                    }
                }
            } else {
                // 通常クリック: 選択をクリックしたクラスタのメンバーへ置き換え
                selected_division.division_ids.clear();
                for &division_id in &cluster.members {
                    selected_division.division_ids.insert(division_id);
                }
            }
            selected_state.0 = None;
        } else {
            // ユニットをクリックしなければ選択を解除しない（状態UIパネルへの影響を避ける）
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_movement_order(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_q: Query<&Transform, With<GameCamera>>,
    player_country: Res<PlayerCountry>,
    state_registry: Res<StateRegistry>,
    war_registry: Res<WarRegistry>,
    selected_division: Res<SelectedDivision>,
    mut military_registry: ResMut<MilitaryRegistry>,
    battle_registry: Res<BattleRegistry>,
    ui_interactions_q: Query<&Interaction>,
) {
    if !mouse_buttons.just_pressed(MouseButton::Right) {
        return;
    }

    for interaction in ui_interactions_q.iter() {
        if *interaction == Interaction::Hovered || *interaction == Interaction::Pressed {
            return;
        }
    }

    if selected_division.is_empty() {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(cam_transform) = camera_q.single() else {
        return;
    };

    let Some(cursor_screen) = window.cursor_position() else {
        return;
    };

    let window_size = Vec2::new(window.width(), window.height());
    let cursor_ndc = (cursor_screen / window_size) * 2.0 - Vec2::ONE;
    let cursor_ndc = Vec2::new(cursor_ndc.x, -cursor_ndc.y);
    let scale = cam_transform.scale.x;
    let half_size = window_size * 0.5 * scale;
    let world_pos = cam_transform.translation.xy() + cursor_ndc * half_size;

    let mut target_state = None;
    for state in &state_registry.states {
        let pos = state.position();
        let size = state.rect_size();
        let min = pos - size * 0.5;
        let max = pos + size * 0.5;

        if world_pos.x >= min.x
            && world_pos.x <= max.x
            && world_pos.y >= min.y
            && world_pos.y <= max.y
        {
            target_state = Some(state.id);
            break;
        }
    }

    let Some(target) = target_state else {
        return;
    };

    // 選択中の各陸軍へ独立に移動命令を発行する。1個の失敗(所有者不一致・戦闘中・
    // 経路なし等)は他の陸軍の命令発行を妨げない(P21-003の複数選択調査で決めた方針)。
    for division_id in selected_division.sorted_ids() {
        try_issue_move_order(
            division_id,
            target,
            player_cid,
            &state_registry,
            &war_registry,
            &mut military_registry,
            &battle_registry,
        );
    }
}

/// 単一陸軍への移動命令発行を試みる(所有者・戦闘状態・撃破済み・領有関係・
/// 経路の検証込み)。`handle_movement_order`が選択中の全陸軍に対して呼び出す。
#[allow(clippy::too_many_arguments)]
fn try_issue_move_order(
    division_id: DivisionId,
    target: crate::common::StateId,
    player_cid: crate::common::CountryId,
    state_registry: &StateRegistry,
    war_registry: &WarRegistry,
    military_registry: &mut MilitaryRegistry,
    battle_registry: &BattleRegistry,
) {
    // ユニット存在確認（撃破済み選択防止）
    let Some(division) = military_registry.divisions.get(&division_id) else {
        warn!(
            "[DivisionMovement] Selected division {:?} no longer exists",
            division_id
        );
        return;
    };

    // 他国ユニットへの命令防止
    if division.owner != player_cid {
        warn!(
            "[DivisionMovement] Cannot command Division {}: Unit belongs to country {:?}",
            division.id.0, division.owner
        );
        return;
    }

    // 戦闘中ユニットへの移動命令を拒否
    if division.status == DivisionStatus::Fighting {
        warn!(
            "[DivisionMovement] Division {} is in combat, cannot move",
            division.id.0
        );
        return;
    }

    // 撃破済みユニットへの命令を拒否
    if division.manpower == 0 {
        warn!(
            "[DivisionMovement] Division {} has no manpower, cannot move",
            division.id.0
        );
        return;
    }

    let division_current = division.current_state;
    let division_owner = division.owner;

    if division_current == target && division.destination == Some(target) {
        return;
    }

    // 移動先の確認
    let target_state_data = match state_registry.get(target) {
        Some(s) => s,
        None => return,
    };

    let target_controller = target_state_data.controller();

    // 移動先が自国支配地域か確認
    let is_own_territory = target_controller == division_owner;

    // 移動先が敵国支配地域（戦争中）か確認
    let is_enemy_territory = war_registry.are_countries_at_war(division_owner, target_controller);

    if !is_own_territory && !is_enemy_territory {
        warn!(
            "[DivisionMovement] Division {} cannot move to state {:?}: neutral or non-hostile territory",
            division.id.0, target
        );
        return;
    }

    // 移動先が戦闘中の地域なら拒否（既に戦闘が行われている）
    if let Some(_battle) = battle_registry.get_ongoing_battle_in_state(target)
        && !is_own_territory
    {
        warn!(
            "[DivisionMovement] Division {} cannot move to state {:?}: battle already ongoing",
            division.id.0, target
        );
        return;
    }

    // 交戦中の敵国リストを構築
    let hostile_countries: Vec<crate::common::CountryId> = war_registry
        .wars
        .values()
        .filter(|w| {
            w.status == crate::war::data::WarStatus::Active
                && (w.attackers.contains(&division_owner) || w.defenders.contains(&division_owner))
        })
        .flat_map(|w| {
            if w.attackers.contains(&division_owner) {
                w.defenders.iter().copied().collect::<Vec<_>>()
            } else {
                w.attackers.iter().copied().collect::<Vec<_>>()
            }
        })
        .collect();

    // 経路探索（自国 + 交戦中の敵国を通過可能）
    let path = crate::military::pathfinding::find_path(
        division_current,
        target,
        state_registry,
        &[division_owner],
        &hostile_countries,
    );

    if let Some(path) = path {
        let Some(division) = military_registry.divisions.get_mut(&division_id) else {
            return;
        };
        division.destination = Some(target);
        division.current_path = path;
        division.target_state = None;
        division.movement_progress = 0.0;
        division.status = DivisionStatus::Moving;
        info!(
            "[DivisionMovement] Division {} path set to {:?} ({} steps)",
            division.id.0,
            target,
            division.current_path.len()
        );
    } else {
        warn!(
            "[DivisionMovement] Division {} cannot reach state {:?}: No valid path",
            division.id.0, target
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DivisionDefinitionId, DivisionId, StateId};
    use crate::map::division_render::division_display_positions;
    use crate::military::data::{DivisionStatus, Division, DivisionSize, DivisionType};
    use crate::state::data::StateData;
    use bevy::window::WindowResolution;

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

    /// 州の中心座標が同一でも、`division_display_positions`のスタックオフセットにより
    /// 同一州にいる2師団は異なる表示座標を持つ（=クリックで区別できる前提条件）。
    #[test]
    fn division_display_positions_offsets_stacked_divisions() {
        let mut military_registry = MilitaryRegistry::default();
        let a1 = make_test_division(1, CountryId(1), StateId(1));
        let a2 = make_test_division(2, CountryId(1), StateId(1));
        military_registry.divisions.insert(a1.id, a1);
        military_registry.divisions.insert(a2.id, a2);

        let state_registry = StateRegistry::build(vec![state_at(1, Vec2::ZERO)]);

        let positions = division_display_positions(&military_registry, &state_registry);
        assert_ne!(
            positions[&DivisionId(1)],
            positions[&DivisionId(2)],
            "stacked divisions in the same state must get distinct display positions"
        );
    }

    /// 画面座標→ワールド座標の逆算(カメラ位置(0,0)・scale=1.0前提)。
    /// `handle_division_selection`内の`screen_to_world`と対になるテスト用ヘルパー。
    fn screen_for_world(world: Vec2, window_size: Vec2) -> Vec2 {
        let half_size = window_size * 0.5;
        let ndc = world / half_size;
        let raw_ndc = Vec2::new(ndc.x, -ndc.y);
        (raw_ndc + Vec2::ONE) * 0.5 * window_size
    }

    /// 既存のWindowエンティティがあれば解放してから、指定位置に新しいカーソル位置で
    /// Windowをspawnする(単発クリックはpress→releaseの2フレームで確定するため、
    /// フレームをまたいでカーソル位置を保持する必要がある)。
    fn respawn_window_at(app: &mut App, screen_pos: Vec2) {
        let to_despawn: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, With<Window>>()
            .iter(app.world())
            .collect();
        for e in to_despawn {
            app.world_mut().entity_mut(e).despawn();
        }
        let mut window = Window {
            resolution: WindowResolution::new(800, 600),
            ..default()
        };
        window.set_cursor_position(Some(screen_pos));
        app.world_mut().spawn(window);
    }

    /// 左ボタンを押す(1フレーム進める)。ドラッグ選択かどうかはこの時点では未確定。
    fn press_left(app: &mut App) {
        let mut mouse = ButtonInput::<MouseButton>::default();
        mouse.press(MouseButton::Left);
        app.insert_resource(mouse);
        app.update();
    }

    /// カーソル位置を更新しつつ、左ボタンを押したまま1フレーム進める(ドラッグ中)。
    fn hold_left_at(app: &mut App, screen_pos: Vec2) {
        let mut query = app.world_mut().query::<&mut Window>();
        if let Some(mut window) = query.iter_mut(app.world_mut()).next() {
            window.set_cursor_position(Some(screen_pos));
        }
        {
            let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            mouse.clear();
            mouse.press(MouseButton::Left);
        }
        app.update();
    }

    /// 左ボタンを離す(1フレーム進める)。単発クリック/ドラッグ選択はここで確定する。
    fn release_left(app: &mut App) {
        {
            let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            mouse.clear();
            mouse.release(MouseButton::Left);
        }
        app.update();
    }

    /// press→(移動なし)→releaseの2フレームで単発クリックを再現する。
    fn simulate_click(app: &mut App, screen_pos: Vec2) {
        respawn_window_at(app, screen_pos);
        press_left(app);
        release_left(app);
    }

    /// 同一州に2師団が重なっている場合でも、クリック位置に近い方の師団を
    /// それぞれ個別に選択できることを検証する（従来は州中心の同一座標で
    /// 判定していたため、常に走査順で決まる片方しか選べなかった）。
    #[test]
    fn handle_division_selection_can_pick_either_stacked_division() {
        fn build_app(military_registry: MilitaryRegistry, state_registry: &StateRegistry) -> App {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins);
            app.add_systems(Update, handle_division_selection);

            app.insert_resource(military_registry);
            app.insert_resource(StateRegistry::build(state_registry.states.clone()));
            app.insert_resource(crate::country::PlayerCountry(Some(CountryId(1))));
            app.init_resource::<SelectedDivision>();
            app.init_resource::<DragSelectState>();
            app.init_resource::<ArmyRegistry>();
            app.init_resource::<crate::state::SelectedState>();
            app.init_resource::<ButtonInput<KeyCode>>();
            app.init_resource::<ButtonInput<MouseButton>>();

            app.world_mut()
                .spawn((Camera2d, GameCamera, Transform::default()));

            app
        }

        fn make_two_division_registry() -> MilitaryRegistry {
            let mut military_registry = MilitaryRegistry::default();
            let a1 = make_test_division(1, CountryId(1), StateId(1));
            let a2 = make_test_division(2, CountryId(1), StateId(1));
            military_registry.divisions.insert(a1.id, a1);
            military_registry.divisions.insert(a2.id, a2);
            military_registry
        }

        let state_registry = StateRegistry::build(vec![state_at(1, Vec2::ZERO)]);
        let window_size = Vec2::new(800.0, 600.0);

        // division_display_positions と同じロジックで、それぞれの師団の
        // 実際の表示座標(スタックオフセット込み)を求めておく。
        let positions = division_display_positions(&make_two_division_registry(), &state_registry);
        let pos_division1 = positions[&DivisionId(1)];
        let pos_division2 = positions[&DivisionId(2)];
        assert_ne!(pos_division1, pos_division2);

        // 師団1の座標をクリック → 師団1が選択される
        {
            let mut app = build_app(make_two_division_registry(), &state_registry);
            simulate_click(&mut app, screen_for_world(pos_division1, window_size));
            let selected = app.world().resource::<SelectedDivision>();
            assert!(selected.is_selected(DivisionId(1)));
            assert_eq!(selected.len(), 1);
        }

        // 師団2の座標をクリック → 師団2が選択される
        {
            let mut app = build_app(make_two_division_registry(), &state_registry);
            simulate_click(&mut app, screen_for_world(pos_division2, window_size));
            let selected = app.world().resource::<SelectedDivision>();
            assert!(selected.is_selected(DivisionId(2)));
            assert_eq!(selected.len(), 1);
        }
    }

    /// P21-003: Ctrl+クリックで選択集合に追加できる。通常クリックは単一選択への
    /// 置換のままであること、Ctrl+クリックで既に選択中のユニットを再クリックすると
    /// 解除(トグル)されることも確認する。
    #[test]
    fn ctrl_click_toggles_multi_selection() {
        let mut military_registry = MilitaryRegistry::default();
        let a1 = make_test_division(1, CountryId(1), StateId(1));
        let a2 = make_test_division(2, CountryId(1), StateId(2));
        military_registry.divisions.insert(a1.id, a1);
        military_registry.divisions.insert(a2.id, a2);

        let state_registry = StateRegistry::build(vec![
            state_at(1, Vec2::new(0.0, 0.0)),
            state_at(2, Vec2::new(300.0, 0.0)),
        ]);
        let window_size = Vec2::new(800.0, 600.0);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, handle_division_selection);
        app.insert_resource(military_registry);
        app.insert_resource(state_registry);
        app.insert_resource(crate::country::PlayerCountry(Some(CountryId(1))));
        app.init_resource::<SelectedDivision>();
        app.init_resource::<DragSelectState>();
        app.init_resource::<ArmyRegistry>();
        app.init_resource::<crate::state::SelectedState>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.world_mut()
            .spawn((Camera2d, GameCamera, Transform::default()));

        // 1. 通常クリックで師団1のみ選択
        simulate_click(&mut app, screen_for_world(Vec2::new(0.0, 0.0), window_size));
        assert_eq!(app.world().resource::<SelectedDivision>().len(), 1);
        assert!(
            app.world()
                .resource::<SelectedDivision>()
                .is_selected(DivisionId(1))
        );

        // 2. Ctrl+クリックで師団2を追加選択(師団1は選択されたまま)
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ControlLeft);
        simulate_click(
            &mut app,
            screen_for_world(Vec2::new(300.0, 0.0), window_size),
        );
        let selected = app.world().resource::<SelectedDivision>();
        assert_eq!(selected.len(), 2);
        assert!(selected.is_selected(DivisionId(1)));
        assert!(selected.is_selected(DivisionId(2)));

        // 3. 既に選択中の師団2を再度Ctrl+クリック → 解除(師団1のみ残る)
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ControlLeft);
        simulate_click(
            &mut app,
            screen_for_world(Vec2::new(300.0, 0.0), window_size),
        );
        let selected = app.world().resource::<SelectedDivision>();
        assert_eq!(selected.len(), 1);
        assert!(selected.is_selected(DivisionId(1)));
        assert!(!selected.is_selected(DivisionId(2)));
    }

    /// P21-004: 左ドラッグで矩形を作り、矩形内の複数師団をまとめて選択できる。
    /// Ctrl非押下の空振りドラッグ(矩形内に師団なし)では選択が解除されることも確認する。
    #[test]
    fn drag_select_picks_up_all_divisions_in_rectangle() {
        let mut military_registry = MilitaryRegistry::default();
        let a1 = make_test_division(1, CountryId(1), StateId(1));
        let a2 = make_test_division(2, CountryId(1), StateId(2));
        // 矩形の外にいる師団3は選択されないはず
        let a3 = make_test_division(3, CountryId(1), StateId(3));
        military_registry.divisions.insert(a1.id, a1);
        military_registry.divisions.insert(a2.id, a2);
        military_registry.divisions.insert(a3.id, a3);

        let state_registry = StateRegistry::build(vec![
            state_at(1, Vec2::new(0.0, 0.0)),
            state_at(2, Vec2::new(100.0, 0.0)),
            state_at(3, Vec2::new(1000.0, 1000.0)),
        ]);
        let window_size = Vec2::new(800.0, 600.0);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, handle_division_selection);
        app.insert_resource(military_registry);
        app.insert_resource(state_registry);
        app.insert_resource(crate::country::PlayerCountry(Some(CountryId(1))));
        app.init_resource::<SelectedDivision>();
        app.init_resource::<DragSelectState>();
        app.init_resource::<ArmyRegistry>();
        app.init_resource::<crate::state::SelectedState>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.world_mut()
            .spawn((Camera2d, GameCamera, Transform::default()));

        // (-50,-50) から (150,50) までドラッグ → 師団1・師団2を包含、師団3は含まない
        respawn_window_at(
            &mut app,
            screen_for_world(Vec2::new(-50.0, -50.0), window_size),
        );
        press_left(&mut app);
        hold_left_at(
            &mut app,
            screen_for_world(Vec2::new(150.0, 50.0), window_size),
        );
        release_left(&mut app);

        let selected = app.world().resource::<SelectedDivision>();
        assert_eq!(
            selected.len(),
            2,
            "drag rectangle must select both divisions inside it"
        );
        assert!(selected.is_selected(DivisionId(1)));
        assert!(selected.is_selected(DivisionId(2)));
        assert!(!selected.is_selected(DivisionId(3)));

        // 師団のいない場所への空振りドラッグ(Ctrl非押下) → 選択解除
        // (座標はウィンドウ範囲内(800x600、カメラ原点中心)に収まるよう選ぶ。
        // ウィンドウ範囲外はcursor_position()がNoneになり得るため)
        respawn_window_at(
            &mut app,
            screen_for_world(Vec2::new(150.0, 150.0), window_size),
        );
        press_left(&mut app);
        hold_left_at(
            &mut app,
            screen_for_world(Vec2::new(250.0, 250.0), window_size),
        );
        release_left(&mut app);

        assert!(
            app.world().resource::<SelectedDivision>().is_empty(),
            "an empty drag box without Ctrl must clear the selection"
        );
    }

    /// P21-003監査で発見した不具合の回帰テスト: 単発クリックでは自国師団のみ選択でき、
    /// 敵国師団はクリック位置に最も近くても選択されない(誤って選択できてしまう不具合の修正確認)。
    #[test]
    fn handle_division_selection_ignores_foreign_division_on_click() {
        let mut military_registry = MilitaryRegistry::default();
        let enemy = make_test_division(1, CountryId(2), StateId(1));
        military_registry.divisions.insert(enemy.id, enemy);

        let state_registry = StateRegistry::build(vec![state_at(1, Vec2::ZERO)]);
        let window_size = Vec2::new(800.0, 600.0);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, handle_division_selection);
        app.insert_resource(military_registry);
        app.insert_resource(state_registry);
        app.insert_resource(crate::country::PlayerCountry(Some(CountryId(1))));
        app.init_resource::<SelectedDivision>();
        app.init_resource::<DragSelectState>();
        app.init_resource::<ArmyRegistry>();
        app.init_resource::<crate::state::SelectedState>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.world_mut()
            .spawn((Camera2d, GameCamera, Transform::default()));

        simulate_click(&mut app, screen_for_world(Vec2::ZERO, window_size));

        assert!(
            app.world().resource::<SelectedDivision>().is_empty(),
            "clicking directly on an enemy-owned division must not select it"
        );
    }

    /// P21-003監査で発見した不具合の回帰テスト: ドラッグ矩形選択でも敵国師団は
    /// 矩形内に含まれていても選択されず、自国師団だけが選択される。
    #[test]
    fn drag_select_ignores_foreign_divisions_in_rectangle() {
        let mut military_registry = MilitaryRegistry::default();
        let own = make_test_division(1, CountryId(1), StateId(1));
        let enemy = make_test_division(2, CountryId(2), StateId(2));
        military_registry.divisions.insert(own.id, own);
        military_registry.divisions.insert(enemy.id, enemy);

        let state_registry = StateRegistry::build(vec![
            state_at(1, Vec2::new(0.0, 0.0)),
            state_at(2, Vec2::new(100.0, 0.0)),
        ]);
        let window_size = Vec2::new(800.0, 600.0);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, handle_division_selection);
        app.insert_resource(military_registry);
        app.insert_resource(state_registry);
        app.insert_resource(crate::country::PlayerCountry(Some(CountryId(1))));
        app.init_resource::<SelectedDivision>();
        app.init_resource::<DragSelectState>();
        app.init_resource::<ArmyRegistry>();
        app.init_resource::<crate::state::SelectedState>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.world_mut()
            .spawn((Camera2d, GameCamera, Transform::default()));

        // 両師団を包含する矩形をドラッグしても、敵国師団(師団2)は選択されない
        respawn_window_at(
            &mut app,
            screen_for_world(Vec2::new(-50.0, -50.0), window_size),
        );
        press_left(&mut app);
        hold_left_at(
            &mut app,
            screen_for_world(Vec2::new(150.0, 50.0), window_size),
        );
        release_left(&mut app);

        let selected = app.world().resource::<SelectedDivision>();
        assert_eq!(selected.len(), 1);
        assert!(selected.is_selected(DivisionId(1)));
        assert!(!selected.is_selected(DivisionId(2)));
    }

    /// P21-004: 編成(Army)所属かつ同じ州にいる師団は地図上で1アイコンにまとまるため、
    /// そのアイコンをクリックすると所属師団がまとめて選択される
    /// (「編成したら一個の師団に見える」要望への対応確認)。
    #[test]
    fn clicking_merged_group_icon_selects_all_members_in_same_state() {
        let mut military_registry = MilitaryRegistry::default();
        let a1 = make_test_division(1, CountryId(1), StateId(1));
        let a2 = make_test_division(2, CountryId(1), StateId(1));
        // 同じ州にいるが編成には属さない師団3は、まとめられずクリック対象外のまま
        let a3 = make_test_division(3, CountryId(1), StateId(1));
        military_registry.divisions.insert(a1.id, a1);
        military_registry.divisions.insert(a2.id, a2);
        military_registry.divisions.insert(a3.id, a3);

        let mut army_registry = ArmyRegistry::default();
        army_registry
            .create_army(CountryId(1), &[DivisionId(1), DivisionId(2)], &military_registry)
            .unwrap();

        let state_registry = StateRegistry::build(vec![state_at(1, Vec2::ZERO)]);
        let window_size = Vec2::new(800.0, 600.0);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, handle_division_selection);
        app.insert_resource(military_registry);
        app.insert_resource(state_registry);
        app.insert_resource(army_registry);
        app.insert_resource(crate::country::PlayerCountry(Some(CountryId(1))));
        app.init_resource::<SelectedDivision>();
        app.init_resource::<DragSelectState>();
        app.init_resource::<crate::state::SelectedState>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.world_mut()
            .spawn((Camera2d, GameCamera, Transform::default()));

        // クラスタの代表(師団1、まとまった編成アイコン)の実際の表示座標をクリックする
        let clusters = division_visual_clusters(
            app.world().resource::<MilitaryRegistry>(),
            app.world().resource::<StateRegistry>(),
            app.world().resource::<ArmyRegistry>(),
        );
        let cluster_pos = clusters[&DivisionId(1)].position;
        assert_eq!(
            clusters[&DivisionId(1)].members,
            vec![DivisionId(1), DivisionId(2)],
            "grouped divisions in the same state must merge into one cluster"
        );

        simulate_click(&mut app, screen_for_world(cluster_pos, window_size));

        let selected = app.world().resource::<SelectedDivision>();
        assert_eq!(
            selected.len(),
            2,
            "clicking the merged icon selects both members"
        );
        assert!(selected.is_selected(DivisionId(1)));
        assert!(selected.is_selected(DivisionId(2)));
        assert!(!selected.is_selected(DivisionId(3)));
    }

    /// P21-004: 同じ編成に属していても、州が異なる(移動などではぐれた)師団は
    /// 1アイコンにまとめられず、個別に選択できたままであることを確認する
    /// (でないと、はぐれた師団が地図上から見えなくなってしまう)。
    #[test]
    fn grouped_divisions_in_different_states_remain_separately_selectable() {
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
        let window_size = Vec2::new(800.0, 600.0);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, handle_division_selection);
        app.insert_resource(military_registry);
        app.insert_resource(state_registry);
        app.insert_resource(army_registry);
        app.insert_resource(crate::country::PlayerCountry(Some(CountryId(1))));
        app.init_resource::<SelectedDivision>();
        app.init_resource::<DragSelectState>();
        app.init_resource::<crate::state::SelectedState>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.world_mut()
            .spawn((Camera2d, GameCamera, Transform::default()));

        // 師団1(State 1)だけをクリック → 同じ編成の師団2(State 2)は選択されない
        simulate_click(&mut app, screen_for_world(Vec2::new(0.0, 0.0), window_size));

        let selected = app.world().resource::<SelectedDivision>();
        assert_eq!(selected.len(), 1);
        assert!(selected.is_selected(DivisionId(1)));
        assert!(!selected.is_selected(DivisionId(2)));
    }

    /// P21-003監査で発見した不具合の回帰テスト: 撃破・消滅した師団のDivisionIdが
    /// `SelectedDivision`に残り続けないこと(`prune_selected_division`の動作確認)。
    #[test]
    fn prune_selected_division_removes_destroyed_division_ids() {
        let mut military_registry = MilitaryRegistry::default();
        let a1 = make_test_division(1, CountryId(1), StateId(1));
        let a2 = make_test_division(2, CountryId(1), StateId(1));
        military_registry.divisions.insert(a1.id, a1);
        military_registry.divisions.insert(a2.id, a2);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, prune_selected_division);
        app.insert_resource(military_registry);
        app.insert_resource(SelectedDivision {
            division_ids: [DivisionId(1), DivisionId(2)].into_iter().collect(),
        });

        // 師団1が戦闘等で撃破され、MilitaryRegistryから削除された想定
        app.world_mut()
            .resource_mut::<MilitaryRegistry>()
            .divisions
            .remove(&DivisionId(1));

        app.update();

        let selected = app.world().resource::<SelectedDivision>();
        assert_eq!(selected.len(), 1);
        assert!(!selected.is_selected(DivisionId(1)));
        assert!(selected.is_selected(DivisionId(2)));
    }

    /// P21-003: 複数選択中に右クリック移動命令を出すと、選択中の全陸軍が独立に
    /// 移動命令を受け取る(1個の失敗が他を妨げない設計の確認も兼ねる: 師団2は
    /// 他国の陸軍のため移動命令を拒否されるが、師団1は正常に移動する)。
    #[test]
    fn handle_movement_order_applies_to_all_selected_divisions() {
        let mut military_registry = MilitaryRegistry::default();
        let a1 = make_test_division(1, CountryId(1), StateId(1));
        // CountryId(2)所有の陸軍(選択されても所有者チェックで移動命令は拒否されるはず)
        let a2 = make_test_division(2, CountryId(2), StateId(1));
        let a3 = make_test_division(3, CountryId(1), StateId(1));
        military_registry.divisions.insert(a1.id, a1);
        military_registry.divisions.insert(a2.id, a2);
        military_registry.divisions.insert(a3.id, a3);

        let s1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(2)],
            world_position: [0.0, 0.0],
            size: [100.0, 100.0],
            ..Default::default()
        };
        let s2 = StateData {
            id: StateId(2),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(1)],
            world_position: [300.0, 0.0],
            size: [100.0, 100.0],
            ..Default::default()
        };
        let state_registry = StateRegistry::build(vec![s1, s2]);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, handle_movement_order);

        app.insert_resource(military_registry);
        app.insert_resource(state_registry);
        app.insert_resource(crate::country::PlayerCountry(Some(CountryId(1))));
        app.insert_resource(WarRegistry::default());
        app.insert_resource(BattleRegistry::default());
        app.init_resource::<crate::state::SelectedState>();
        app.insert_resource(SelectedDivision {
            division_ids: [DivisionId(1), DivisionId(2), DivisionId(3)].into_iter().collect(),
        });

        let mut mouse = ButtonInput::<MouseButton>::default();
        mouse.press(MouseButton::Right);
        app.insert_resource(mouse);

        app.world_mut()
            .spawn((Camera2d, GameCamera, Transform::default()));

        let mut window = Window {
            resolution: WindowResolution::new(800, 600),
            ..default()
        };
        // StateId(2)(300, 0)のワールド座標に対応する画面座標を計算(カメラ原点・scale=1前提)
        let window_size = Vec2::new(800.0, 600.0);
        let target_world = Vec2::new(300.0, 0.0);
        let half_size = window_size * 0.5;
        let ndc = target_world / half_size;
        let raw_ndc = Vec2::new(ndc.x, -ndc.y);
        let screen = (raw_ndc + Vec2::ONE) * 0.5 * window_size;
        window.set_cursor_position(Some(screen));
        app.world_mut().spawn(window);

        app.update();

        let military_registry = app.world().resource::<MilitaryRegistry>();
        let division1 = military_registry.divisions.get(&DivisionId(1)).unwrap();
        assert_eq!(division1.destination, Some(StateId(2)));
        assert_eq!(division1.status, DivisionStatus::Moving);

        let division3 = military_registry.divisions.get(&DivisionId(3)).unwrap();
        assert_eq!(division3.destination, Some(StateId(2)));
        assert_eq!(division3.status, DivisionStatus::Moving);

        // 他国の陸軍(師団2)は選択されていても移動命令が発行されない
        let division2 = military_registry.divisions.get(&DivisionId(2)).unwrap();
        assert_eq!(division2.destination, None);
        assert_eq!(division2.status, DivisionStatus::Idle);
    }
}
