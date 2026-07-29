/// 侵攻処理モジュール
/// 陸軍が敵支配地域へ到着した際の判定・占領・戦闘開始処理と
/// 戦闘終了後の撤退先選定を担当する
use crate::common::{ArmyId, StateId};
use crate::military::battle::{Battle, BattleRegistry, BattleStatus};
use crate::military::data::{ArmyStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;

/// 陸軍が新しい地域へ到着したときに呼ぶ
/// 敵軍がいなければ占領し、いれば戦闘を開始する
///
/// # 引数
/// * `army_id` - 到着したユニットID
/// * `arrival_state` - 到着地域
/// * `origin_state` - 出発地域（攻撃側が敗北時の退路）
/// * `current_date` - 戦闘開始日の文字列
pub fn process_army_arrival(
    army_id: ArmyId,
    arrival_state: StateId,
    origin_state: StateId,
    current_date: &str,
    military_registry: &mut MilitaryRegistry,
    state_registry: &mut StateRegistry,
    war_registry: &WarRegistry,
    battle_registry: &mut BattleRegistry,
) {
    let army_owner = match military_registry.armies.get(&army_id) {
        Some(a) => a.owner,
        None => return,
    };

    // 到着地域の現在の支配国を取得
    let controller = match state_registry.get(arrival_state) {
        Some(s) => s.controller(),
        None => return,
    };

    // 支配国が自国なら通常移動完了（占領処理不要）
    if controller == army_owner {
        return;
    }

    // 交戦状態チェック
    if !war_registry.are_countries_at_war(army_owner, controller) {
        // 交戦中でない国家の領土には侵入できない → 移動停止
        if let Some(army) = military_registry.armies.get_mut(&army_id) {
            army.status = ArmyStatus::Idle;
            army.destination = None;
            army.current_path.clear();
            army.target_state = None;
            army.movement_progress = 0.0;
        }
        return;
    }

    // 既存の進行中の戦闘があるか確認（同一地域の重複戦闘防止）
    if battle_registry
        .get_ongoing_battle_in_state(arrival_state)
        .is_some()
    {
        // 戦闘中の地域へは進入できない → 待機
        if let Some(army) = military_registry.armies.get_mut(&army_id) {
            army.status = ArmyStatus::Idle;
            army.destination = None;
            army.current_path.clear();
            army.target_state = None;
            army.movement_progress = 0.0;
        }
        return;
    }

    // 到着地域に敵軍がいるか確認（最小IDを持つユニットを選ぶ）
    let enemy_army_id = find_enemy_army_in_state(arrival_state, army_owner, military_registry, war_registry);

    if let Some(enemy_id) = enemy_army_id {
        // 敵軍あり → 戦闘開始
        start_battle_between(
            army_id,
            enemy_id,
            arrival_state,
            origin_state,
            current_date,
            military_registry,
            war_registry,
            battle_registry,
        );
    } else {
        // 敵軍なし → 即占領（支配国変更）
        occupy_state(arrival_state, army_owner, state_registry);
    }
}

/// 地域内で特定国家の敵軍（最小IDを持つ）を検索する
fn find_enemy_army_in_state(
    state_id: StateId,
    owner: crate::common::CountryId,
    military_registry: &MilitaryRegistry,
    war_registry: &WarRegistry,
) -> Option<ArmyId> {
    let mut ids: Vec<ArmyId> = military_registry
        .armies
        .values()
        .filter(|a| {
            a.current_state == state_id
                && war_registry.are_countries_at_war(owner, a.owner)
                && a.status != ArmyStatus::Retreating
        })
        .map(|a| a.id)
        .collect();

    // 決定的な選択: IDの小さい順
    ids.sort_by_key(|id| id.0);
    ids.into_iter().next()
}

/// 2ユニット間の戦闘を開始する
#[allow(clippy::too_many_arguments)]
fn start_battle_between(
    attacker_id: ArmyId,
    defender_id: ArmyId,
    state_id: StateId,
    attacker_origin: StateId,
    current_date: &str,
    military_registry: &mut MilitaryRegistry,
    war_registry: &WarRegistry,
    battle_registry: &mut BattleRegistry,
) {
    let (attacker_country, defender_country, war_id) = {
        let attacker = match military_registry.armies.get(&attacker_id) {
            Some(a) => a,
            None => return,
        };
        let defender = match military_registry.armies.get(&defender_id) {
            Some(d) => d,
            None => return,
        };

        // 戦争IDを取得
        let war_id = war_registry
            .wars
            .values()
            .find(|w| {
                w.status == crate::war::data::WarStatus::Active
                    && ((w.attackers.contains(&attacker.owner)
                        && w.defenders.contains(&defender.owner))
                        || (w.defenders.contains(&attacker.owner)
                            && w.attackers.contains(&defender.owner)))
            })
            .map(|w| w.id);

        let war_id = match war_id {
            Some(id) => id,
            None => return,
        };

        (attacker.owner, defender.owner, war_id)
    };

    let battle = Battle {
        id: crate::common::BattleId(0), // start_battle で上書き
        war_id,
        state_id,
        attacker_country,
        defender_country,
        attacker_army_id: attacker_id,
        defender_army_id: defender_id,
        start_date: current_date.to_string(),
        elapsed_days: 0,
        status: BattleStatus::Ongoing,
        attacker_origin_state: attacker_origin,
    };

    match battle_registry.start_battle(battle) {
        Ok(battle_id) => {
            // 両ユニットを戦闘状態に設定し、移動を停止
            if let Some(army) = military_registry.armies.get_mut(&attacker_id) {
                army.status = ArmyStatus::Fighting;
                army.combat_id = Some(battle_id);
                army.destination = None;
                army.current_path.clear();
                army.target_state = None;
                army.movement_progress = 0.0;
            }
            if let Some(army) = military_registry.armies.get_mut(&defender_id) {
                army.status = ArmyStatus::Fighting;
                army.combat_id = Some(battle_id);
            }
            bevy::log::info!(
                "[Battle] Started battle {:?} in state {:?}: {:?} vs {:?}",
                battle_id,
                state_id,
                attacker_id,
                defender_id
            );
        }
        Err(e) => {
            bevy::log::warn!("[Battle] Could not start battle: {}", e);
            // 戦闘開始失敗時は攻撃側を元の地域に戻す（現在地は変更済みのため位置はそのまま待機）
            if let Some(army) = military_registry.armies.get_mut(&attacker_id) {
                army.status = ArmyStatus::Idle;
                army.destination = None;
                army.current_path.clear();
                army.target_state = None;
                army.movement_progress = 0.0;
            }
        }
    }
}

/// 地域の支配国を変更する（所有国は変更しない）
pub fn occupy_state(
    state_id: StateId,
    new_controller: crate::common::CountryId,
    state_registry: &mut StateRegistry,
) {
    if let Some(state) = state_registry.get_mut(state_id) {
        state.controller_country = Some(new_controller);
        bevy::log::info!(
            "[Invasion] State {:?} is now controlled by {:?}",
            state_id,
            new_controller
        );
    }
}

/// 防御側の撤退先を決定的に選ぶ
/// 優先順位: 自国支配の隣接陸地 → StateId昇順最小
///
/// # 引数
/// * `army_id` - 撤退するユニット
/// * `current_state` - 現在の地域（撤退元）
/// * `battle_registry` - 戦闘中の地域を除外するため
pub fn find_retreat_destination(
    army_id: ArmyId,
    current_state: StateId,
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
    battle_registry: &BattleRegistry,
) -> Option<StateId> {
    let army = military_registry.armies.get(&army_id)?;
    let owner = army.owner;

    let current_state_data = state_registry.get(current_state)?;

    let mut candidates: Vec<StateId> = current_state_data
        .neighbors
        .iter()
        .filter_map(|&neighbor_id| {
            let neighbor = state_registry.get(neighbor_id)?;

            // 自国支配地域のみ
            if neighbor.controller() != owner {
                return None;
            }

            // 戦闘中の地域は除外
            if battle_registry
                .get_ongoing_battle_in_state(neighbor_id)
                .is_some()
            {
                return None;
            }

            Some(neighbor_id)
        })
        .collect();

    // 決定的な選択: StateId昇順
    candidates.sort_by_key(|id| id.0);
    candidates.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DivisionId};
    use crate::military::battle::BattleRegistry;
    use crate::military::data::{ArmyStatus, ArmyUnit, DivisionSize, DivisionType, MilitaryRegistry};
    use crate::state::data::{StateData, StateRegistry};
    use crate::war::data::{War, WarRegistry, WarStatus};
    use std::collections::HashSet;

    fn make_army(id: usize, owner: CountryId, state: StateId) -> ArmyUnit {
        ArmyUnit {
            id: ArmyId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: state,
            destination: None,
            current_path: vec![],
            target_state: None,
            manpower: 10000,
            max_manpower: 10000,
            equipment: 100.0,
            max_equipment: 100.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 100.0,
            max_morale: 100.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: ArmyStatus::Idle,
            def_id: DivisionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn setup_war_registry(c1: CountryId, c2: CountryId) -> WarRegistry {
        let mut reg = WarRegistry::default();
        let war = War {
            id: crate::common::WarId(0),
            name: "Test War".to_string(),
            start_date: "1800/01/01".to_string(),
            attackers: [c1].iter().cloned().collect(),
            defenders: [c2].iter().cloned().collect(),
            war_goals: vec![],
            war_score: 0.0,
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: HashSet::new(),
            status: WarStatus::Active,
        };
        reg.wars.insert(war.id, war);
        reg
    }

    #[test]
    fn test_occupy_state_changes_controller_not_owner() {
        let mut state = StateData::default();
        state.id = StateId(1);
        state.owner_country_id = CountryId(2);
        state.controller_country = None;

        let mut registry = StateRegistry::build(vec![state]);
        occupy_state(StateId(1), CountryId(1), &mut registry);

        let s = registry.get(StateId(1)).unwrap();
        // 支配国は変化する
        assert_eq!(s.controller(), CountryId(1));
        // 所有国は変化しない
        assert_eq!(s.owner_country_id, CountryId(2));
    }

    #[test]
    fn test_find_retreat_destination_deterministic() {
        // State 1 (owner: C2) has neighbors [2, 3]
        // State 2 (owner: C2, controller: C2)
        // State 3 (owner: C2, controller: C2)
        let mut s1 = StateData::default();
        s1.id = StateId(1);
        s1.owner_country_id = CountryId(2);
        s1.neighbors = vec![StateId(3), StateId(2)]; // 逆順で設定してIDソートをテスト

        let mut s2 = StateData::default();
        s2.id = StateId(2);
        s2.owner_country_id = CountryId(2);

        let mut s3 = StateData::default();
        s3.id = StateId(3);
        s3.owner_country_id = CountryId(2);

        let state_registry = StateRegistry::build(vec![s1, s2, s3]);
        let battle_registry = BattleRegistry::default();

        let mut mil = MilitaryRegistry::default();
        let army = make_army(0, CountryId(2), StateId(1));
        mil.armies.insert(ArmyId(0), army);

        let result = find_retreat_destination(
            ArmyId(0),
            StateId(1),
            &mil,
            &state_registry,
            &battle_registry,
        );

        // StateId(2) の方が小さいのでそちらが選ばれる
        assert_eq!(result, Some(StateId(2)));
    }

    #[test]
    fn test_start_battle_creates_exactly_one_battle() {
        let c1 = CountryId(1);
        let c2 = CountryId(2);
        let war_reg = setup_war_registry(c1, c2);

        let mut mil = MilitaryRegistry::default();
        let army1 = make_army(0, c1, StateId(2)); // 攻撃側（到着済み）
        let army2 = make_army(1, c2, StateId(2)); // 防御側
        mil.armies.insert(ArmyId(0), army1);
        mil.armies.insert(ArmyId(1), army2);

        let s1 = StateData { id: StateId(1), owner_country_id: c1, ..Default::default() };
        let mut s2 = StateData { id: StateId(2), owner_country_id: c2, ..Default::default() };
        s2.controller_country = None;
        let mut state_reg = StateRegistry::build(vec![s1, s2]);
        let mut battle_reg = BattleRegistry::default();

        process_army_arrival(
            ArmyId(0),
            StateId(2),
            StateId(1),
            "1800/01/01",
            &mut mil,
            &mut state_reg,
            &war_reg,
            &mut battle_reg,
        );

        // 戦闘が1件だけ作成される
        assert_eq!(battle_reg.battles.len(), 1);
        // 両ユニットが戦闘中になる
        assert_eq!(mil.armies[&ArmyId(0)].status, ArmyStatus::Fighting);
        assert_eq!(mil.armies[&ArmyId(1)].status, ArmyStatus::Fighting);
    }

    #[test]
    fn test_no_battle_created_between_same_country() {
        let c1 = CountryId(1);
        let war_reg = WarRegistry::default(); // 戦争なし

        let mut mil = MilitaryRegistry::default();
        let army1 = make_army(0, c1, StateId(1));
        let army2 = make_army(1, c1, StateId(1));
        mil.armies.insert(ArmyId(0), army1);
        mil.armies.insert(ArmyId(1), army2);

        let s1 = StateData { id: StateId(1), owner_country_id: c1, ..Default::default() };
        let s2 = StateData { id: StateId(2), owner_country_id: c1, ..Default::default() };
        let mut state_reg = StateRegistry::build(vec![s1, s2]);
        let mut battle_reg = BattleRegistry::default();

        process_army_arrival(
            ArmyId(0),
            StateId(1),
            StateId(0),
            "1800/01/01",
            &mut mil,
            &mut state_reg,
            &war_reg,
            &mut battle_reg,
        );

        // 同一国なので戦闘なし
        assert_eq!(battle_reg.battles.len(), 0);
    }

    #[test]
    fn test_no_duplicate_battle_in_same_state() {
        let c1 = CountryId(1);
        let c2 = CountryId(2);
        let war_reg = setup_war_registry(c1, c2);

        let mut mil = MilitaryRegistry::default();
        let army1 = make_army(0, c1, StateId(2));
        let army2 = make_army(1, c2, StateId(2));
        mil.armies.insert(ArmyId(0), army1);
        mil.armies.insert(ArmyId(1), army2);

        let s1 = StateData { id: StateId(1), owner_country_id: c1, ..Default::default() };
        let mut s2 = StateData { id: StateId(2), owner_country_id: c2, ..Default::default() };
        s2.controller_country = None;
        let mut state_reg = StateRegistry::build(vec![s1, s2]);
        let mut battle_reg = BattleRegistry::default();

        // 1回目
        process_army_arrival(
            ArmyId(0),
            StateId(2),
            StateId(1),
            "1800/01/01",
            &mut mil,
            &mut state_reg,
            &war_reg,
            &mut battle_reg,
        );
        assert_eq!(battle_reg.battles.len(), 1);

        // 同じ命令を再度実行しても重複しない
        process_army_arrival(
            ArmyId(0),
            StateId(2),
            StateId(1),
            "1800/01/02",
            &mut mil,
            &mut state_reg,
            &war_reg,
            &mut battle_reg,
        );
        assert_eq!(battle_reg.battles.len(), 1);
    }
}
