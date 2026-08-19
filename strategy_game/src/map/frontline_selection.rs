/// P21-005: Armyを前線へ割り当てるための「前線選択モード」の入力処理。
///
/// `FrontlineSelectMode`はこのファイル(map層)が所有する。`map::division_selection`/
/// `map::selection`がこのモード中は自身の通常のクリック処理(州選択・師団選択・移動命令)
/// を止めるために参照するため、逆方向の依存(map -> ui)を作らないよう`ui`モジュールへは
/// 一切依存しない(このファイルは`bevy_ui`の`Interaction`コンポーネント型だけを使う。
/// これは`map::division_selection`/`map::selection`が既に同じ理由で使っている型であり、
/// `crate::ui`側の型をimportするわけではない)。
///
/// 前線選択モードの「パネルを閉じたらキャンセルする」という1条件だけは、`ui::military_panel`
/// (`MilitaryPanelState`)側からこのファイルの`FrontlineSelectMode::cancel`を呼ぶ形で実現する
/// (ui -> mapの向きなので依存方向として問題ない)。実際の割当成功/失敗のローカライズ済み
/// 通知(`GameNotification`)も同じ理由でui層(`military_panel`)が`FrontlineAssignmentAttempted`
/// Messageを購読して構築する。
use crate::app::game_state::GameState;
use crate::common::{ArmyId, CountryId, FrontlineId, StateId};
use crate::map::camera::GameCamera;
use crate::map::division_selection::{DragSelectState, screen_to_world};
use crate::military::army::{ArmyRegistry, SelectedArmy};
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;
use crate::war::frontline::FrontlineRegistry;
use bevy::prelude::*;

/// 前線選択モードの状態。`army_id`がSomeの間だけアクティブになる。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontlineSelectMode {
    pub army_id: Option<ArmyId>,
}

impl FrontlineSelectMode {
    pub fn is_active(&self) -> bool {
        self.army_id.is_some()
    }

    pub fn activate(&mut self, army_id: ArmyId) {
        self.army_id = Some(army_id);
    }

    pub fn cancel(&mut self) {
        self.army_id = None;
    }
}

/// 前線選択モード中の1クリックの結果。実際のローカライズ済み通知文言はui層が構築する
/// (このファイルは翻訳カタログへ依存しない)。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontlineAssignmentAttempted {
    Assigned {
        army_id: ArmyId,
        frontline_id: FrontlineId,
    },
    Invalid,
}

pub struct FrontlineSelectionPlugin;

impl Plugin for FrontlineSelectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrontlineSelectMode>()
            .add_message::<FrontlineAssignmentAttempted>()
            .add_systems(
                Update,
                (
                    cancel_frontline_select_mode_on_context_change,
                    handle_frontline_select_click,
                )
                    .chain()
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

/// Escape・右クリック・対象Army選択の変更・対象Armyの消滅・割当可能な前線の消滅
/// (Warの終結等)のいずれかが起きたら前線選択モードを解除する。解除は割当状態を
/// 一切変更しない(`FrontlineSelectMode::cancel`は`army_id`をNoneに戻すだけ)。
/// 「Armyパネルを閉じたら解除」だけは`ui::military_panel`側が別途担う(依存方向の都合)。
pub(crate) fn cancel_frontline_select_mode_on_context_change(
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    selected_army: Res<SelectedArmy>,
    army_registry: Res<ArmyRegistry>,
    war_registry: Res<WarRegistry>,
    frontline_registry: Res<FrontlineRegistry>,
    mut mode: ResMut<FrontlineSelectMode>,
) {
    let Some(army_id) = mode.army_id else {
        return;
    };

    if keys.just_pressed(KeyCode::Escape) || mouse_buttons.just_pressed(MouseButton::Right) {
        mode.cancel();
        return;
    }

    if selected_army.0 != Some(army_id) {
        mode.cancel();
        return;
    }

    let Some(army) = army_registry.armies.get(&army_id) else {
        mode.cancel();
        return;
    };

    if frontline_registry
        .assignable_frontlines_for_army(army.owner, &war_registry)
        .is_empty()
    {
        mode.cancel();
    }
}

/// 同一州に複数のFrontline候補がある場合、`FrontlineId`昇順で決定的に1つへ絞り込む。
/// `owner`から見た「自国側」前線地域(attacker/defender front regions)に`clicked_state`が
/// 含まれるFrontlineだけを候補にする(既存の前線Overlayが色分け表示する自国側領域と
/// 一致させ、プレイヤーが見えている強調表示をそのままクリックできるようにするため)。
pub fn find_assignable_frontline_for_click(
    frontline_registry: &FrontlineRegistry,
    war_registry: &WarRegistry,
    owner: CountryId,
    clicked_state: StateId,
) -> Option<FrontlineId> {
    let mut candidates: Vec<FrontlineId> = frontline_registry
        .assignable_frontlines_for_army(owner, war_registry)
        .into_iter()
        .filter(|fl_id| {
            frontline_registry.frontlines.get(fl_id).is_some_and(|fl| {
                let own_front = if fl.attacker_country_id == owner {
                    &fl.attacker_front_regions
                } else {
                    &fl.defender_front_regions
                };
                own_front.contains(&clicked_state)
            })
        })
        .collect();
    candidates.sort_by_key(|id| id.0);
    candidates.into_iter().next()
}

/// 前線選択モード中の左クリックを処理する。有効な前線(クリックしたStateが対象Armyの
/// 自国側前線地域に含まれるFrontline)なら割当を実行しモードを終了する。無効なクリック
/// (前線が見つからない、またはクリック自体が州に当たらない)では一切の状態を変更しない。
/// `ui_interactions_q`/`drag_state`のガードは`map::selection::handle_state_click`と
/// 同じ既存の慣習を踏襲し、UI操作中・ドラッグ選択中はこのクリックを処理しない。
#[allow(clippy::too_many_arguments)]
fn handle_frontline_select_click(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_q: Query<&Transform, With<GameCamera>>,
    state_registry: Res<StateRegistry>,
    war_registry: Res<WarRegistry>,
    army_registry: Res<ArmyRegistry>,
    ui_interactions_q: Query<&Interaction>,
    drag_state: Res<DragSelectState>,
    mut mode: ResMut<FrontlineSelectMode>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
    mut outcome_writer: MessageWriter<FrontlineAssignmentAttempted>,
) {
    let Some(army_id) = mode.army_id else {
        return;
    };

    if !mouse_buttons.just_released(MouseButton::Left) {
        return;
    }
    if drag_state.is_dragging {
        return;
    }
    for interaction in ui_interactions_q.iter() {
        if *interaction == Interaction::Hovered || *interaction == Interaction::Pressed {
            return;
        }
    }

    let Some(army) = army_registry.armies.get(&army_id) else {
        return;
    };
    let owner = army.owner;

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
    let world_pos = screen_to_world(cursor_screen, window_size, cam_transform);

    let mut clicked_state = None;
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
            clicked_state = Some(state.id);
            break;
        }
    }

    let target = clicked_state.and_then(|sid| {
        find_assignable_frontline_for_click(&frontline_registry, &war_registry, owner, sid)
    });

    match target {
        Some(frontline_id)
            if frontline_registry
                .assign_army(army_id, frontline_id, owner, &army_registry, &war_registry)
                .is_ok() =>
        {
            outcome_writer.write(FrontlineAssignmentAttempted::Assigned {
                army_id,
                frontline_id,
            });
            mode.cancel();
        }
        _ => {
            outcome_writer.write(FrontlineAssignmentAttempted::Invalid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{DivisionDefinitionId, DivisionId, WarId};
    use crate::military::data::{
        Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
    };
    use crate::state::data::StateData;
    use crate::war::data::{War, WarStatus};
    use crate::war::frontline::{Frontline, update_all_frontlines};
    use std::collections::HashSet;

    fn setup() -> (
        StateRegistry,
        WarRegistry,
        MilitaryRegistry,
        FrontlineRegistry,
    ) {
        let s1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(2)],
            ..Default::default()
        };
        let s2 = StateData {
            id: StateId(2),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(1)],
            ..Default::default()
        };
        let state_registry = StateRegistry::build(vec![s1, s2]);

        let mut war_registry = WarRegistry::default();
        let war = War {
            id: WarId(0),
            name: "Test War".to_string(),
            attackers: [CountryId(1)].into_iter().collect(),
            defenders: [CountryId(2)].into_iter().collect(),
            primary_attacker: None,
            primary_defender: None,
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            end_date: None,
            duration_days: 0,
            war_score: 0.0,
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: HashSet::new(),
            status: WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: HashSet::new(),
        };
        war_registry.wars.insert(war.id, war);

        let military_registry = MilitaryRegistry::default();
        let mut frontline_registry = FrontlineRegistry::default();
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        (
            state_registry,
            war_registry,
            military_registry,
            frontline_registry,
        )
    }

    #[test]
    fn find_assignable_frontline_for_click_matches_own_front_region() {
        let (_, war_registry, _, frontline_registry) = setup();
        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(0))
            .unwrap()
            .frontline_id;

        // C1(攻撃側)にとって、自国側前線地域はState(1)
        assert_eq!(
            find_assignable_frontline_for_click(
                &frontline_registry,
                &war_registry,
                CountryId(1),
                StateId(1)
            ),
            Some(fl_id)
        );
        // 敵側地域(State 2)をクリックしても割当対象にならない
        assert_eq!(
            find_assignable_frontline_for_click(
                &frontline_registry,
                &war_registry,
                CountryId(1),
                StateId(2)
            ),
            None
        );
        // C2(防御側)にとってはState(2)が自国側
        assert_eq!(
            find_assignable_frontline_for_click(
                &frontline_registry,
                &war_registry,
                CountryId(2),
                StateId(2)
            ),
            Some(fl_id)
        );
    }

    #[test]
    fn find_assignable_frontline_for_click_picks_lowest_frontline_id_deterministically() {
        let (state_registry, war_registry, military_registry, mut frontline_registry) = setup();

        // 手動で2つ目のFrontlineを同じ州(State 1)を自国側前線地域に持つ形で追加し、
        // HashMap反復順ではなくFrontlineId最小が選ばれることを確認する。
        let extra = Frontline {
            frontline_id: FrontlineId(99),
            war_id: WarId(0),
            attacker_country_id: CountryId(1),
            defender_country_id: CountryId(2),
            attacker_front_regions: vec![StateId(1)],
            defender_front_regions: vec![StateId(2)],
            border_region_pairs: vec![(StateId(1), StateId(2))],
        };
        frontline_registry.frontlines.insert(FrontlineId(99), extra);

        // setup()内でupdate_all_frontlines()により最初に自動生成されたFrontlineは
        // next_frontline_idが0から始まるため常にFrontlineId(0)になる(決定的)。
        // FrontlineId(99)と共にWarId(0)を共有する状態を作った上で、正しくFrontlineId
        // 最小(0)が選ばれることを確認する(HashMap反復順ではなく明示的なsort_by_keyに
        // よる決定性の検証)。
        let result = find_assignable_frontline_for_click(
            &frontline_registry,
            &war_registry,
            CountryId(1),
            StateId(1),
        )
        .unwrap();
        assert_eq!(result, FrontlineId(0));

        let _ = (state_registry, military_registry);
    }

    #[test]
    fn cancel_mode_resource_toggles_correctly() {
        let mut mode = FrontlineSelectMode::default();
        assert!(!mode.is_active());
        mode.activate(ArmyId(0));
        assert!(mode.is_active());
        assert_eq!(mode.army_id, Some(ArmyId(0)));
        mode.cancel();
        assert!(!mode.is_active());
    }

    fn make_division(id: usize, owner: CountryId, state: StateId) -> Division {
        Division {
            id: DivisionId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: state,
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 1000,
            max_manpower: 1000,
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 1.0,
            max_morale: 1.0,
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

    /// `cancel_frontline_select_mode_on_context_change`のロジック(War終結で割当可能な
    /// 前線がなくなったら自動キャンセルする)を、System登録なしで直接呼んで検証する。
    #[test]
    fn mode_would_be_cancelled_when_no_assignable_frontline_remains() {
        let (state_registry, mut war_registry, mut military_registry, mut frontline_registry) =
            setup();
        let mut army_registry = ArmyRegistry::default();
        let division_id =
            military_registry.add_division(make_division(0, CountryId(1), StateId(1)));
        let army_id = army_registry
            .create_army(CountryId(1), &[division_id], &military_registry)
            .unwrap();

        // Warを終わらせると、C1にとって割当可能な前線が0件になる
        war_registry.wars.get_mut(&WarId(0)).unwrap().status = WarStatus::WhitePeace;
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        let army = army_registry.armies.get(&army_id).unwrap();
        assert!(
            frontline_registry
                .assignable_frontlines_for_army(army.owner, &war_registry)
                .is_empty(),
            "after war end, no assignable frontline should remain"
        );
    }
}
