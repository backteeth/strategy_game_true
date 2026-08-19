/// 侵攻処理モジュール
/// 陸軍が敵支配地域へ到着した際の判定・占領・戦闘開始処理と
/// 戦闘終了後の撤退先選定を担当する
use crate::common::{DivisionId, StateId};
use crate::military::battle::{Battle, BattleRegistry, BattleStatus};
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;
use std::collections::HashMap;

/// 陸軍が新しい地域へ到着したときに呼ぶ
/// 敵軍がいなければ占領し、いれば戦闘を開始する
///
/// # 引数
/// * `division_id` - 到着したユニットID
/// * `arrival_state` - 到着地域
/// * `origin_state` - 出発地域（攻撃側が敗北時の退路）
/// * `current_date` - 戦闘開始日の文字列
#[allow(clippy::too_many_arguments)]
pub fn process_division_arrival(
    division_id: DivisionId,
    arrival_state: StateId,
    origin_state: StateId,
    current_date: &str,
    military_registry: &mut MilitaryRegistry,
    state_registry: &mut StateRegistry,
    war_registry: &WarRegistry,
    battle_registry: &mut BattleRegistry,
) {
    let division_owner = match military_registry.divisions.get(&division_id) {
        Some(a) => a.owner,
        None => return,
    };

    // 到着地域の現在の支配国を取得
    let controller = match state_registry.get(arrival_state) {
        Some(s) => s.controller(),
        None => return,
    };

    // 支配国が自国なら通常移動完了（占領処理不要）
    if controller == division_owner {
        return;
    }

    // 交戦状態チェック
    if !war_registry.are_countries_at_war(division_owner, controller) {
        // 交戦中でない国家の領土には侵入できない → 移動停止
        if let Some(division) = military_registry.divisions.get_mut(&division_id) {
            division.status = DivisionStatus::Idle;
            division.destination = None;
            division.current_path.clear();
            division.target_state = None;
            division.movement_progress = 0.0;
        }
        return;
    }

    // 既存の進行中の戦闘があるか確認（同一地域の重複戦闘防止）
    if let Some(existing) = battle_registry.get_ongoing_battle_in_state(arrival_state) {
        if existing.attacker_country == division_owner {
            // 自軍(攻撃側)の戦闘に合流 — 複数師団による共同攻撃を可能にする
            join_battle_as_attacker(
                division_id,
                arrival_state,
                origin_state,
                military_registry,
                battle_registry,
            );
        } else {
            // 防御側・無関係国 → 従来通り待機（この州へは進入できない）
            if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                division.status = DivisionStatus::Idle;
                division.destination = None;
                division.current_path.clear();
                division.target_state = None;
                division.movement_progress = 0.0;
            }
        }
        return;
    }

    // 到着地域に敵軍がいるか確認（その州にいる敵師団を全員取得）
    let enemy_division_ids = find_enemy_divisions_in_state(
        arrival_state,
        division_owner,
        military_registry,
        war_registry,
    );

    if !enemy_division_ids.is_empty() {
        // 敵軍あり → 戦闘開始（敵師団は全員まとめて防御側として参戦）
        start_battle_between(
            division_id,
            enemy_division_ids,
            arrival_state,
            origin_state,
            current_date,
            military_registry,
            war_registry,
            battle_registry,
        );
    } else {
        // 敵軍なし → 即占領（支配国変更）
        occupy_state(arrival_state, division_owner, state_registry);
    }
}

/// 進行中の戦闘に、後から到着した自軍(攻撃側)師団を合流させる
fn join_battle_as_attacker(
    division_id: DivisionId,
    state_id: StateId,
    origin_state: StateId,
    military_registry: &mut MilitaryRegistry,
    battle_registry: &mut BattleRegistry,
) {
    let battle_id = match battle_registry.get_ongoing_battle_in_state(state_id) {
        Some(b) => b.id,
        None => return,
    };

    if let Some(battle) = battle_registry.battles.get_mut(&battle_id) {
        if !battle.attacker_division_ids.contains(&division_id) {
            battle.attacker_division_ids.push(division_id);
            battle.attacker_division_ids.sort_by_key(|id| id.0);
        }
        battle.attacker_origins.insert(division_id, origin_state);
    }

    if let Some(division) = military_registry.divisions.get_mut(&division_id) {
        division.status = DivisionStatus::Fighting;
        division.combat_id = Some(battle_id);
        division.destination = None;
        division.current_path.clear();
        division.target_state = None;
        division.movement_progress = 0.0;
    }

    bevy::log::info!(
        "[Battle] Division {:?} joined ongoing battle {:?} in state {:?} as reinforcement",
        division_id,
        battle_id,
        state_id
    );
}

/// 地域内の特定国家の敵軍を全員検索する（IDの小さい順）
fn find_enemy_divisions_in_state(
    state_id: StateId,
    owner: crate::common::CountryId,
    military_registry: &MilitaryRegistry,
    war_registry: &WarRegistry,
) -> Vec<DivisionId> {
    let mut ids: Vec<DivisionId> = military_registry
        .divisions
        .values()
        .filter(|a| {
            a.current_state == state_id
                && war_registry.are_countries_at_war(owner, a.owner)
                && a.status != DivisionStatus::Retreating
        })
        .map(|a| a.id)
        .collect();

    // 決定的な順序: IDの小さい順
    ids.sort_by_key(|id| id.0);
    ids
}

/// 攻撃側1師団と防御側複数師団の間で戦闘を開始する
#[allow(clippy::too_many_arguments)]
fn start_battle_between(
    attacker_id: DivisionId,
    defender_ids: Vec<DivisionId>,
    state_id: StateId,
    attacker_origin: StateId,
    current_date: &str,
    military_registry: &mut MilitaryRegistry,
    war_registry: &WarRegistry,
    battle_registry: &mut BattleRegistry,
) {
    let (attacker_country, defender_country, war_id) = {
        let attacker = match military_registry.divisions.get(&attacker_id) {
            Some(a) => a,
            None => return,
        };
        let defender = match defender_ids
            .first()
            .and_then(|id| military_registry.divisions.get(id))
        {
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

    let mut attacker_origins = HashMap::new();
    attacker_origins.insert(attacker_id, attacker_origin);

    let battle = Battle {
        id: crate::common::BattleId(0), // start_battle で上書き
        war_id,
        state_id,
        attacker_country,
        defender_country,
        attacker_division_ids: vec![attacker_id],
        defender_division_ids: defender_ids.clone(),
        attacker_origins,
        start_date: current_date.to_string(),
        elapsed_days: 0,
        status: BattleStatus::Ongoing,
    };

    match battle_registry.start_battle(battle) {
        Ok(battle_id) => {
            // 参加ユニット全員を戦闘状態に設定し、移動を停止
            if let Some(division) = military_registry.divisions.get_mut(&attacker_id) {
                division.status = DivisionStatus::Fighting;
                division.combat_id = Some(battle_id);
                division.destination = None;
                division.current_path.clear();
                division.target_state = None;
                division.movement_progress = 0.0;
            }
            for defender_id in &defender_ids {
                if let Some(division) = military_registry.divisions.get_mut(defender_id) {
                    division.status = DivisionStatus::Fighting;
                    division.combat_id = Some(battle_id);
                }
            }
            bevy::log::info!(
                "[Battle] Started battle {:?} in state {:?}: {:?} vs {:?}",
                battle_id,
                state_id,
                attacker_id,
                defender_ids
            );
        }
        Err(e) => {
            bevy::log::warn!("[Battle] Could not start battle: {}", e);
            // 戦闘開始失敗時は攻撃側を元の地域に戻す（現在地は変更済みのため位置はそのまま待機）
            if let Some(division) = military_registry.divisions.get_mut(&attacker_id) {
                division.status = DivisionStatus::Idle;
                division.destination = None;
                division.current_path.clear();
                division.target_state = None;
                division.movement_progress = 0.0;
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
/// * `division_id` - 撤退するユニット
/// * `current_state` - 現在の地域（撤退元）
/// * `battle_registry` - 戦闘中の地域を除外するため
pub fn find_retreat_destination(
    division_id: DivisionId,
    current_state: StateId,
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
    battle_registry: &BattleRegistry,
) -> Option<StateId> {
    let division = military_registry.divisions.get(&division_id)?;
    let owner = division.owner;

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
    use crate::common::{CountryId, DivisionDefinitionId, DivisionId};
    use crate::military::battle::BattleRegistry;
    use crate::military::data::{
        Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
    };
    use crate::state::data::{StateData, StateRegistry};
    use crate::war::data::{War, WarRegistry, WarStatus};
    use std::collections::HashSet;

    fn make_division(id: usize, owner: CountryId, state: StateId) -> Division {
        Division {
            id: DivisionId(id),
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
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(0),
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
            end_date: None,
            duration_days: 0,
            attackers: [c1].iter().cloned().collect(),
            defenders: [c2].iter().cloned().collect(),
            primary_attacker: None,
            primary_defender: None,
            war_goals: vec![],
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
        reg.wars.insert(war.id, war);
        reg
    }

    #[test]
    fn test_occupy_state_changes_controller_not_owner() {
        let state = StateData {
            id: StateId(1),
            owner_country_id: CountryId(2),
            controller_country: None,
            ..Default::default()
        };

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
        let s1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(3), StateId(2)], // 逆順で設定してIDソートをテスト
            ..Default::default()
        };

        let s2 = StateData {
            id: StateId(2),
            owner_country_id: CountryId(2),
            ..Default::default()
        };

        let s3 = StateData {
            id: StateId(3),
            owner_country_id: CountryId(2),
            ..Default::default()
        };

        let state_registry = StateRegistry::build(vec![s1, s2, s3]);
        let battle_registry = BattleRegistry::default();

        let mut mil = MilitaryRegistry::default();
        let division = make_division(0, CountryId(2), StateId(1));
        mil.divisions.insert(DivisionId(0), division);

        let result = find_retreat_destination(
            DivisionId(0),
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
        let division1 = make_division(0, c1, StateId(2)); // 攻撃側（到着済み）
        let division2 = make_division(1, c2, StateId(2)); // 防御側
        mil.divisions.insert(DivisionId(0), division1);
        mil.divisions.insert(DivisionId(1), division2);

        let s1 = StateData {
            id: StateId(1),
            owner_country_id: c1,
            ..Default::default()
        };
        let mut s2 = StateData {
            id: StateId(2),
            owner_country_id: c2,
            ..Default::default()
        };
        s2.controller_country = None;
        let mut state_reg = StateRegistry::build(vec![s1, s2]);
        let mut battle_reg = BattleRegistry::default();

        process_division_arrival(
            DivisionId(0),
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
        assert_eq!(
            mil.divisions[&DivisionId(0)].status,
            DivisionStatus::Fighting
        );
        assert_eq!(
            mil.divisions[&DivisionId(1)].status,
            DivisionStatus::Fighting
        );
    }

    #[test]
    fn test_no_battle_created_between_same_country() {
        let c1 = CountryId(1);
        let war_reg = WarRegistry::default(); // 戦争なし

        let mut mil = MilitaryRegistry::default();
        let division1 = make_division(0, c1, StateId(1));
        let division2 = make_division(1, c1, StateId(1));
        mil.divisions.insert(DivisionId(0), division1);
        mil.divisions.insert(DivisionId(1), division2);

        let s1 = StateData {
            id: StateId(1),
            owner_country_id: c1,
            ..Default::default()
        };
        let s2 = StateData {
            id: StateId(2),
            owner_country_id: c1,
            ..Default::default()
        };
        let mut state_reg = StateRegistry::build(vec![s1, s2]);
        let mut battle_reg = BattleRegistry::default();

        process_division_arrival(
            DivisionId(0),
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
        let division1 = make_division(0, c1, StateId(2));
        let division2 = make_division(1, c2, StateId(2));
        mil.divisions.insert(DivisionId(0), division1);
        mil.divisions.insert(DivisionId(1), division2);

        let s1 = StateData {
            id: StateId(1),
            owner_country_id: c1,
            ..Default::default()
        };
        let mut s2 = StateData {
            id: StateId(2),
            owner_country_id: c2,
            ..Default::default()
        };
        s2.controller_country = None;
        let mut state_reg = StateRegistry::build(vec![s1, s2]);
        let mut battle_reg = BattleRegistry::default();

        // 1回目
        process_division_arrival(
            DivisionId(0),
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
        process_division_arrival(
            DivisionId(0),
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

    #[test]
    fn test_second_attacking_division_joins_ongoing_battle_instead_of_waiting() {
        let c1 = CountryId(1);
        let c2 = CountryId(2);
        let war_reg = setup_war_registry(c1, c2);

        let mut mil = MilitaryRegistry::default();
        let division0 = make_division(0, c1, StateId(2)); // 攻撃側1体目（到着済み）
        let division1 = make_division(1, c2, StateId(2)); // 防御側
        let division2 = make_division(2, c1, StateId(2)); // 攻撃側2体目（後から到着）
        mil.divisions.insert(DivisionId(0), division0);
        mil.divisions.insert(DivisionId(1), division1);
        mil.divisions.insert(DivisionId(2), division2);

        let s1 = StateData {
            id: StateId(1),
            owner_country_id: c1,
            ..Default::default()
        };
        let mut s2 = StateData {
            id: StateId(2),
            owner_country_id: c2,
            ..Default::default()
        };
        s2.controller_country = None;
        let mut state_reg = StateRegistry::build(vec![s1, s2]);
        let mut battle_reg = BattleRegistry::default();

        // 1体目が到着 → 戦闘開始
        process_division_arrival(
            DivisionId(0),
            StateId(2),
            StateId(1),
            "1800/01/01",
            &mut mil,
            &mut state_reg,
            &war_reg,
            &mut battle_reg,
        );
        assert_eq!(battle_reg.battles.len(), 1);

        // 2体目(同じ攻撃側)が到着 → 待機させられず、既存戦闘に合流する
        process_division_arrival(
            DivisionId(2),
            StateId(2),
            StateId(1),
            "1800/01/03",
            &mut mil,
            &mut state_reg,
            &war_reg,
            &mut battle_reg,
        );

        // 新しい戦闘は作られず、依然として1件のまま
        assert_eq!(battle_reg.battles.len(), 1);
        let battle = battle_reg.get_ongoing_battle_in_state(StateId(2)).unwrap();
        assert_eq!(
            battle.attacker_division_ids,
            vec![DivisionId(0), DivisionId(2)]
        );
        assert_eq!(battle.defender_division_ids, vec![DivisionId(1)]);
        assert_eq!(
            battle.attacker_origins.get(&DivisionId(2)),
            Some(&StateId(1))
        );
        // 合流した2体目も戦闘状態になり、待機(Idle)扱いにはならない
        assert_eq!(
            mil.divisions[&DivisionId(2)].status,
            DivisionStatus::Fighting
        );
        assert_eq!(mil.divisions[&DivisionId(2)].combat_id, Some(battle.id));
    }

    #[test]
    fn test_battle_starts_with_all_stacked_defenders_not_just_lowest_id() {
        let c1 = CountryId(1);
        let c2 = CountryId(2);
        let war_reg = setup_war_registry(c1, c2);

        let mut mil = MilitaryRegistry::default();
        let division0 = make_division(0, c1, StateId(2)); // 攻撃側
        let division1 = make_division(1, c2, StateId(2)); // 防御側1体目
        let division2 = make_division(2, c2, StateId(2)); // 防御側2体目（同じ州に事前スタック）
        mil.divisions.insert(DivisionId(0), division0);
        mil.divisions.insert(DivisionId(1), division1);
        mil.divisions.insert(DivisionId(2), division2);

        let s1 = StateData {
            id: StateId(1),
            owner_country_id: c1,
            ..Default::default()
        };
        let mut s2 = StateData {
            id: StateId(2),
            owner_country_id: c2,
            ..Default::default()
        };
        s2.controller_country = None;
        let mut state_reg = StateRegistry::build(vec![s1, s2]);
        let mut battle_reg = BattleRegistry::default();

        process_division_arrival(
            DivisionId(0),
            StateId(2),
            StateId(1),
            "1800/01/01",
            &mut mil,
            &mut state_reg,
            &war_reg,
            &mut battle_reg,
        );

        let battle = battle_reg.get_ongoing_battle_in_state(StateId(2)).unwrap();
        // 事前にスタックしていた防御側2体とも初期参加者になる
        assert_eq!(
            battle.defender_division_ids,
            vec![DivisionId(1), DivisionId(2)]
        );
        assert_eq!(
            mil.divisions[&DivisionId(2)].status,
            DivisionStatus::Fighting
        );
    }
}
