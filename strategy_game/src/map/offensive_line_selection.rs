/// P21-007: Armyが割り当てられたFrontlineに対して「攻勢線」(計画データのみ)を
/// 複数クリックで組み立てるための編集モード。
///
/// `map::frontline_selection::FrontlineSelectMode`(単発クリックで即座に割当・自動終了)とは
/// 異なり、こちらは複数クリックにまたがる編集セッションを保持する: `活性化 → 複数回の
/// クリックでdraftへ地点を追加/除去 → UI側の確定ボタンで`war::frontline::
/// FrontlineRegistry::set_offensive_line`へ反映 → モード終了`という流れになる。
///
/// `FrontlineSelectMode`と同じ理由で`crate::ui`へは一切依存しない
/// (`Interaction`コンポーネント型だけを使う。`map::division_selection`/
/// `map::selection`/`map::frontline_selection`が既に同じ理由で使っている型)。
/// 「軍事パネルを閉じたらキャンセルする」という1条件だけは`ui::military_panel`側から
/// `OffensiveLineEditMode::cancel`を呼ぶ形で実現する(ui -> mapの向きなので問題ない)。
///
/// draftは確定(`FrontlineRegistry::set_offensive_line`)まで一切`FrontlineRegistry`へ
/// 反映されない。クリックによるdraftへの追加/除去自体も、常に「有効な対象地点」
/// (実在する州・陸地・このFrontlineの戦争における敵国支配地域)だけを受け付ける
/// (`is_valid_offensive_line_candidate`)。連結性はdraft全体に対する性質であり、
/// 個々のクリックでは検証しない(確定時に`set_offensive_line`が検証する)。
use crate::app::game_state::GameState;
use crate::common::{ArmyId, CountryId, FrontlineId, StateId};
use crate::map::camera::GameCamera;
use crate::map::division_selection::{DragSelectState, screen_to_world};
use crate::military::army::SelectedArmy;
use crate::state::data::StateRegistry;
use crate::war::frontline::FrontlineRegistry;
use bevy::prelude::*;

/// 攻勢線編集モードの状態。`army_id`がSomeの間だけアクティブになる。
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct OffensiveLineEditMode {
    pub army_id: Option<ArmyId>,
    pub frontline_id: Option<FrontlineId>,
    pub country_id: Option<CountryId>,
    /// 編集中のdraft。StateId昇順に保つ(`toggle_region`が挿入時にソートする)。
    pub draft: Vec<StateId>,
}

impl OffensiveLineEditMode {
    pub fn is_active(&self) -> bool {
        self.army_id.is_some()
    }

    /// 編集モードを開始する。`initial`は既存の確定済み攻勢線(再編集時)、
    /// 未設定なら空Vecを渡す。
    pub fn activate(
        &mut self,
        army_id: ArmyId,
        frontline_id: FrontlineId,
        country_id: CountryId,
        initial: Vec<StateId>,
    ) {
        self.army_id = Some(army_id);
        self.frontline_id = Some(frontline_id);
        self.country_id = Some(country_id);
        self.draft = initial;
    }

    /// 編集を破棄する。draftは一切永続化されない。
    pub fn cancel(&mut self) {
        self.army_id = None;
        self.frontline_id = None;
        self.country_id = None;
        self.draft.clear();
    }

    /// draft内に`region_id`が既にあれば除去、無ければ追加する(StateId昇順を維持)。
    pub fn toggle_region(&mut self, region_id: StateId) {
        if let Some(pos) = self.draft.iter().position(|&r| r == region_id) {
            self.draft.remove(pos);
        } else {
            self.draft.push(region_id);
            self.draft.sort_by_key(|s| s.0);
        }
    }
}

/// クリック対象の州が、指定Frontline・国にとって有効な攻勢線候補地点かどうかを判定する。
/// `FrontlineRegistry::set_offensive_line`の単一地点検証(実在・陸地・
/// 「このFrontlineの戦争における敵国支配」)と同じ規則を、クリック時にも先出しで使う。
pub fn is_valid_offensive_line_candidate(
    frontline_registry: &FrontlineRegistry,
    state_registry: &StateRegistry,
    frontline_id: FrontlineId,
    country_id: CountryId,
    state_id: StateId,
) -> bool {
    let Some(frontline) = frontline_registry.frontlines.get(&frontline_id) else {
        return false;
    };
    let enemy_id = if country_id == frontline.attacker_country_id {
        frontline.defender_country_id
    } else {
        frontline.attacker_country_id
    };
    let Some(state) = state_registry.get(state_id) else {
        return false;
    };
    !state.is_sea && state.controller() == enemy_id
}

pub struct OffensiveLineSelectionPlugin;

impl Plugin for OffensiveLineSelectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OffensiveLineEditMode>().add_systems(
            Update,
            (
                cancel_offensive_line_edit_on_context_change,
                handle_offensive_line_region_click,
            )
                .chain()
                .run_if(in_state(GameState::Playing)),
        );
    }
}

/// Escape・右クリック・対象Army選択の変更・対象Armyの前線割当の変化(解除・前線削除・
/// 和平等、いずれも`frontline_for_army`の結果が変わる)のいずれかが起きたら編集モードを
/// 解除する。解除はdraftを破棄するだけで、`FrontlineRegistry`側の確定済みデータは
/// 一切変更しない。
pub(crate) fn cancel_offensive_line_edit_on_context_change(
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    selected_army: Res<SelectedArmy>,
    frontline_registry: Res<FrontlineRegistry>,
    mut mode: ResMut<OffensiveLineEditMode>,
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

    if frontline_registry.frontline_for_army(army_id) != mode.frontline_id {
        mode.cancel();
    }
}

/// 編集モード中の左クリックを処理する。有効な対象地点ならdraftへ追加/除去のトグルを行う。
/// 無効なクリック(州に当たらない、海、自国領、非敵対領域)はdraftを一切変更しない
/// (`map::selection::handle_state_click`と同じUI/ドラッグ選択ガードを踏襲)。
#[allow(clippy::too_many_arguments)]
fn handle_offensive_line_region_click(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_q: Query<&Transform, With<GameCamera>>,
    state_registry: Res<StateRegistry>,
    frontline_registry: Res<FrontlineRegistry>,
    ui_interactions_q: Query<&Interaction>,
    drag_state: Res<DragSelectState>,
    mut mode: ResMut<OffensiveLineEditMode>,
) {
    let (Some(frontline_id), Some(country_id)) = (mode.frontline_id, mode.country_id) else {
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

    let Some(state_id) = clicked_state else {
        return;
    };

    if is_valid_offensive_line_candidate(
        &frontline_registry,
        &state_registry,
        frontline_id,
        country_id,
        state_id,
    ) {
        mode.toggle_region(state_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DivisionDefinitionId, DivisionId, StateId, WarId};
    use crate::military::army::ArmyRegistry;
    use crate::military::data::{
        Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
    };
    use crate::state::data::StateData;
    use crate::war::data::{War, WarRegistry, WarStatus};
    use crate::war::frontline::update_all_frontlines;
    use std::collections::HashSet;

    fn setup() -> (
        StateRegistry,
        WarRegistry,
        MilitaryRegistry,
        FrontlineRegistry,
        FrontlineId,
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
        let s3 = StateData {
            id: StateId(3),
            owner_country_id: CountryId(0),
            neighbors: vec![],
            is_sea: true,
            ..Default::default()
        };
        let state_registry = StateRegistry::build(vec![s1, s2, s3]);

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
        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(0))
            .unwrap()
            .frontline_id;

        (
            state_registry,
            war_registry,
            military_registry,
            frontline_registry,
            fl_id,
        )
    }

    #[test]
    fn activate_and_cancel_toggle_mode_state() {
        let mut mode = OffensiveLineEditMode::default();
        assert!(!mode.is_active());
        mode.activate(ArmyId(0), FrontlineId(0), CountryId(1), vec![StateId(2)]);
        assert!(mode.is_active());
        assert_eq!(mode.draft, vec![StateId(2)]);
        mode.cancel();
        assert!(!mode.is_active());
        assert!(mode.draft.is_empty());
        assert_eq!(mode.frontline_id, None);
        assert_eq!(mode.country_id, None);
    }

    #[test]
    fn toggle_region_adds_then_removes_and_keeps_sorted_order() {
        let mut mode = OffensiveLineEditMode::default();
        mode.activate(ArmyId(0), FrontlineId(0), CountryId(1), Vec::new());
        mode.toggle_region(StateId(5));
        mode.toggle_region(StateId(2));
        assert_eq!(mode.draft, vec![StateId(2), StateId(5)]);
        mode.toggle_region(StateId(2));
        assert_eq!(mode.draft, vec![StateId(5)]);
    }

    #[test]
    fn is_valid_offensive_line_candidate_accepts_enemy_land_only() {
        let (state_registry, _, _, frontline_registry, fl_id) = setup();

        // C1(攻撃側)からはC2所有のState2が有効。
        assert!(is_valid_offensive_line_candidate(
            &frontline_registry,
            &state_registry,
            fl_id,
            CountryId(1),
            StateId(2),
        ));
        // 自国領(State1)は無効。
        assert!(!is_valid_offensive_line_candidate(
            &frontline_registry,
            &state_registry,
            fl_id,
            CountryId(1),
            StateId(1),
        ));
        // 海(State3)は無効。
        assert!(!is_valid_offensive_line_candidate(
            &frontline_registry,
            &state_registry,
            fl_id,
            CountryId(1),
            StateId(3),
        ));
    }

    /// 要求テスト項目10: 対象Army選択が変わると編集モードが解除される。
    #[test]
    fn cancel_on_context_change_fires_when_selected_army_differs() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let (_, war_registry, _, frontline_registry, _fl_id) = setup();
        app.insert_resource(war_registry);
        app.insert_resource(frontline_registry);
        app.insert_resource(SelectedArmy(Some(ArmyId(99))));
        app.insert_resource(ButtonInput::<KeyCode>::default());
        app.insert_resource(ButtonInput::<MouseButton>::default());
        let mut mode = OffensiveLineEditMode::default();
        mode.activate(ArmyId(0), FrontlineId(0), CountryId(1), vec![StateId(2)]);
        app.insert_resource(mode);
        app.add_systems(Update, cancel_offensive_line_edit_on_context_change);

        app.update();

        assert!(
            !app.world().resource::<OffensiveLineEditMode>().is_active(),
            "選択中Armyがモードのarmy_idと異なる場合は編集モードが解除されるはず"
        );
    }

    /// 要求テスト項目16の一部: 対象Armyの前線割当が消滅(前線削除/解除)すると
    /// 編集モードが解除される。
    #[test]
    fn cancel_on_context_change_fires_when_frontline_assignment_gone() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let (_, war_registry, _, frontline_registry, fl_id) = setup();
        app.insert_resource(war_registry);
        app.insert_resource(frontline_registry);
        app.insert_resource(SelectedArmy(Some(ArmyId(0))));
        app.insert_resource(ButtonInput::<KeyCode>::default());
        app.insert_resource(ButtonInput::<MouseButton>::default());
        let mut mode = OffensiveLineEditMode::default();
        // army_idは選択中と一致するが、frontline_for_army(0)はNone(未割当)なので
        // モードのfrontline_id(FrontlineId(0))と一致せず解除されるはず。
        mode.activate(ArmyId(0), fl_id, CountryId(1), vec![StateId(2)]);
        app.insert_resource(mode);
        app.add_systems(Update, cancel_offensive_line_edit_on_context_change);

        app.update();

        assert!(
            !app.world().resource::<OffensiveLineEditMode>().is_active(),
            "対象Armyの前線割当が失われていれば編集モードは解除されるはず"
        );
    }

    /// 要求テスト項目11: Escapeキーでdraftが破棄される。
    #[test]
    fn escape_key_cancels_mode_and_discards_draft() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let (_, war_registry, _, mut frontline_registry, fl_id) = setup();
        let mut army_registry = ArmyRegistry::default();
        let mut military_registry = MilitaryRegistry::default();
        let division = Division {
            id: DivisionId(0),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(1),
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
        };
        military_registry.divisions.insert(division.id, division);
        let army_id = army_registry
            .create_army(CountryId(1), &[DivisionId(0)], &military_registry)
            .unwrap();
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        app.insert_resource(war_registry);
        app.insert_resource(frontline_registry);
        app.insert_resource(SelectedArmy(Some(army_id)));
        let mut mode = OffensiveLineEditMode::default();
        mode.activate(army_id, fl_id, CountryId(1), vec![StateId(2)]);
        app.insert_resource(mode);
        app.add_systems(Update, cancel_offensive_line_edit_on_context_change);

        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::Escape);
        app.insert_resource(keys);
        app.insert_resource(ButtonInput::<MouseButton>::default());

        app.update();

        let mode_after = app.world().resource::<OffensiveLineEditMode>();
        assert!(!mode_after.is_active());
        assert!(mode_after.draft.is_empty());
    }
}
