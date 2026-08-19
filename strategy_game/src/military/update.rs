use crate::app::time::DayChangedMessage;
use crate::common::{BattleId, DivisionId, StateId};
use crate::country::CountryRegistry;
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::combat_calc::{
    ORG_RECOVERY_PER_DAY, RETREAT_MANPOWER_LOSS_RATIO, calculate_terrain_defense_bonus,
    resolve_combat_day_multi,
};
use crate::military::data::{Division, DivisionStatus, MilitaryRegistry};
use crate::military::invasion::{find_retreat_destination, occupy_state};
use crate::military::movement::{process_movement, validate_and_stop_invalid_movements};
use crate::state::data::StateRegistry;
use crate::war::data::{WarRegistry, WarStatus};
use bevy::prelude::*;

pub fn handle_daily_military(
    mut day_events: MessageReader<DayChangedMessage>,
    mut country_registry: ResMut<CountryRegistry>,
    mut military_registry: ResMut<MilitaryRegistry>,
    mut state_registry: ResMut<StateRegistry>,
    mut battle_registry: ResMut<BattleRegistry>,
    war_registry: Res<WarRegistry>,
) {
    let events: Vec<DayChangedMessage> = day_events.read().copied().collect();

    for event in events {
        let current_date = format!("{:04}/{:02}/{:02}", event.year, event.month, event.day);

        // 1. 募集処理
        crate::military::recruitment::process_recruitment(
            &mut country_registry,
            &mut military_registry,
        );

        // 3. 経路の事前検証（戦争終了等で無効になった移動を停止）
        validate_and_stop_invalid_movements(&mut military_registry, &state_registry, &war_registry);

        // 4. 移動処理（侵攻・戦闘開始を含む）
        process_movement(
            &mut military_registry,
            &mut state_registry,
            &war_registry,
            &mut battle_registry,
            &current_date,
        );

        // 4. 日次戦闘計算（戦闘IDの昇順で処理）
        process_daily_battles(
            &mut military_registry,
            &mut battle_registry,
            &state_registry,
            &war_registry,
        );

        // 5. 戦闘終了判定と後処理
        resolve_finished_battles(
            &mut military_registry,
            &mut state_registry,
            &mut battle_registry,
            &war_registry,
        );

        // 6. 終了・キャンセル済み戦闘のクリーンアップ
        battle_registry.cleanup_finished_battles();

        // 7. 組織率回復
        process_org_recovery(&mut military_registry, &state_registry);
    }
}

/// 進行中の全戦闘を1日分計算する（BattleId昇順で決定的に処理）
/// P21-siege: 攻撃側・防御側とも複数師団が参加できるため、各陣営の実効戦力を
/// 参加師団の合計として計算し(`resolve_combat_day_multi`)、生じた被害を
/// 貢献度に応じて個々の師団へ配分する。
fn process_daily_battles(
    military_registry: &mut MilitaryRegistry,
    battle_registry: &mut BattleRegistry,
    state_registry: &StateRegistry,
    war_registry: &WarRegistry,
) {
    // 戦闘IDの安定した処理順序（昇順）
    let mut battle_ids: Vec<BattleId> = battle_registry.battles.keys().copied().collect();
    battle_ids.sort_by_key(|id| id.0);

    for battle_id in battle_ids {
        let battle = match battle_registry.battles.get(&battle_id) {
            Some(b) => b.clone(),
            None => continue,
        };

        if battle.status != BattleStatus::Ongoing {
            continue;
        }

        // 戦争がまだ有効か確認
        let war_active = war_registry
            .wars
            .get(&battle.war_id)
            .map(|w| w.status == WarStatus::Active)
            .unwrap_or(false);

        if !war_active {
            // 戦争終了 → 戦闘キャンセル
            if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
                b.status = BattleStatus::Cancelled;
            }
            cleanup_battle_units(&battle.attacker_division_ids, military_registry);
            cleanup_battle_units(&battle.defender_division_ids, military_registry);
            continue;
        }

        // 参加ユニットのうち既に消滅済みのものを除去する
        let attacker_ids: Vec<DivisionId> = battle
            .attacker_division_ids
            .iter()
            .copied()
            .filter(|id| military_registry.divisions.contains_key(id))
            .collect();
        let defender_ids: Vec<DivisionId> = battle
            .defender_division_ids
            .iter()
            .copied()
            .filter(|id| military_registry.divisions.contains_key(id))
            .collect();

        let participants_changed = attacker_ids.len() != battle.attacker_division_ids.len()
            || defender_ids.len() != battle.defender_division_ids.len();
        if participants_changed && let Some(b) = battle_registry.battles.get_mut(&battle_id) {
            b.attacker_division_ids = attacker_ids.clone();
            b.defender_division_ids = defender_ids.clone();
        }

        if attacker_ids.is_empty() || defender_ids.is_empty() {
            // どちらかの陣営が全滅済み → resolve_finished_battlesで決着処理させる
            continue;
        }

        // 地形補正取得
        let terrain_bonus = state_registry
            .get(battle.state_id)
            .map(calculate_terrain_defense_bonus)
            .unwrap_or(0);

        // 戦闘計算（クローンして借用衝突を避ける）
        let attacker_divisions: Vec<Division> = attacker_ids
            .iter()
            .filter_map(|id| military_registry.divisions.get(id).cloned())
            .collect();
        let defender_divisions: Vec<Division> = defender_ids
            .iter()
            .filter_map(|id| military_registry.divisions.get(id).cloned())
            .collect();
        let attacker_refs: Vec<&Division> = attacker_divisions.iter().collect();
        let defender_refs: Vec<&Division> = defender_divisions.iter().collect();

        let (atk_results, def_results) =
            resolve_combat_day_multi(&attacker_refs, &defender_refs, terrain_bonus);

        for (division_id, (manpower_loss, org_loss)) in atk_results {
            if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                division.manpower = division.manpower.saturating_sub(manpower_loss);
                division.organization = (division.organization - org_loss).max(0.0);
            }
        }
        for (division_id, (manpower_loss, org_loss)) in def_results {
            if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                division.manpower = division.manpower.saturating_sub(manpower_loss);
                division.organization = (division.organization - org_loss).max(0.0);
            }
        }

        // 経過日数更新
        if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
            b.elapsed_days += 1;
        }
    }
}

/// 戦闘終了条件を判定し、後処理を行う
/// P21-siege: 個々の参加師団ごとに「戦闘継続可能か(組織率>0かつ戦力>0)」を判定し、
/// 脱落した師団は撤退または撃破する。どちらかの陣営の参加者が全員脱落した時点で
/// 戦闘そのものが終了する(=複数師団で押し切れば必ず州を落とせるようになる)。
fn resolve_finished_battles(
    military_registry: &mut MilitaryRegistry,
    state_registry: &mut StateRegistry,
    battle_registry: &mut BattleRegistry,
    war_registry: &WarRegistry,
) {
    let battle_ids: Vec<BattleId> = battle_registry.battles.keys().copied().collect();

    for battle_id in battle_ids {
        let battle = match battle_registry.battles.get(&battle_id) {
            Some(b) => b.clone(),
            None => continue,
        };

        if battle.status != BattleStatus::Ongoing {
            continue;
        }

        // 攻撃側: 脱落者は各自の出撃元地域へ撤退する(常に退路あり)
        let mut remaining_attackers = Vec::new();
        for &division_id in &battle.attacker_division_ids {
            let origin = battle.attacker_origins.get(&division_id).copied();
            if resolve_participant_outcome(division_id, origin, military_registry) {
                remaining_attackers.push(division_id);
            }
        }

        // 防御側: 脱落者は隣接する自国支配地域へ撤退を試みる(なければ包囲撃破)
        let mut remaining_defenders = Vec::new();
        for &division_id in &battle.defender_division_ids {
            let retreat_dest = find_retreat_destination(
                division_id,
                battle.state_id,
                military_registry,
                state_registry,
                battle_registry,
            );
            if resolve_participant_outcome(division_id, retreat_dest, military_registry) {
                remaining_defenders.push(division_id);
            }
        }

        if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
            b.attacker_division_ids = remaining_attackers.clone();
            b.defender_division_ids = remaining_defenders.clone();
        }

        match (
            remaining_attackers.is_empty(),
            remaining_defenders.is_empty(),
        ) {
            (false, false) => {
                // 両陣営とも生存者あり → 戦闘継続
            }
            (false, true) => {
                // 防御側全滅 → 攻撃側勝利、州を占領
                if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
                    b.status = BattleStatus::AttackerWon;
                }
                for &division_id in &remaining_attackers {
                    if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                        division.status = DivisionStatus::Idle;
                        division.combat_id = None;
                        division.current_state = battle.state_id;
                    }
                }
                occupy_state(battle.state_id, battle.attacker_country, state_registry);
                info!(
                    "[Battle] {:?} captured state {:?} with {} surviving division(s)",
                    battle.attacker_country,
                    battle.state_id,
                    remaining_attackers.len()
                );
                update_war_occupied_states(
                    battle.attacker_country,
                    battle.state_id,
                    military_registry,
                    war_registry,
                );
            }
            (true, false) => {
                // 攻撃側全滅 → 防御側勝利、支配権は変更しない
                if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
                    b.status = BattleStatus::DefenderWon;
                }
                for &division_id in &remaining_defenders {
                    if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                        division.status = DivisionStatus::Idle;
                        division.combat_id = None;
                    }
                }
                info!(
                    "[Battle] Defender {:?} held state {:?}. Territory unchanged.",
                    battle.defender_country, battle.state_id
                );
            }
            (true, true) => {
                // 両陣営とも全滅 → 防御側有利（地域支配変更なし）
                if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
                    b.status = BattleStatus::DefenderWon;
                }
                info!(
                    "[Battle] Both sides annihilated in state {:?}. Territory unchanged.",
                    battle.state_id
                );
            }
        }
    }
}

/// 個々の参加師団の組織率/戦力を判定し、脱落していれば撤退または撃破処理を行う。
/// 戦闘に留まる(組織率>0かつ戦力>0)場合のみ true を返す。
///
/// `retreat_destination`は攻撃側なら出撃元(`battle.attacker_origins`、常にSome想定)、
/// 防御側なら`find_retreat_destination`で求めた退路(見つからなければNone)を渡す。
fn resolve_participant_outcome(
    division_id: DivisionId,
    retreat_destination: Option<StateId>,
    military_registry: &mut MilitaryRegistry,
) -> bool {
    let Some(division) = military_registry.divisions.get(&division_id).cloned() else {
        return false; // 既に消滅済み
    };

    if division.manpower == 0 {
        military_registry.remove_division(division_id);
        info!("[Battle] Division {:?} destroyed", division_id);
        return false;
    }

    if division.organization > 0.0 {
        return true; // まだ戦闘継続可能
    }

    // 組織率0 → 撤退を試みる。撤退時にも追加のmanpower損失を与え、
    // 同じ師団が「撤退→組織率回復→再戦」を無限に繰り返せないようにする
    let retreat_loss = (division.max_manpower as f32 * RETREAT_MANPOWER_LOSS_RATIO) as u64;
    let remaining_manpower = division.manpower.saturating_sub(retreat_loss);

    match retreat_destination {
        Some(dest) if remaining_manpower > 0 => {
            if let Some(d) = military_registry.divisions.get_mut(&division_id) {
                d.manpower = remaining_manpower;
                d.current_state = dest;
                d.status = DivisionStatus::Idle;
                d.combat_id = None;
                d.target_state = None;
                d.destination = None;
                d.current_path.clear();
                d.movement_progress = 0.0;
            }
            info!(
                "[Battle] Division {:?} retreated to {:?} (manpower: {})",
                division_id, dest, remaining_manpower
            );
            false
        }
        _ => {
            military_registry.remove_division(division_id);
            info!(
                "[Battle] Division {:?} destroyed during retreat/encirclement",
                division_id
            );
            false
        }
    }
}

/// 戦闘後に参加ユニット全員の戦闘状態をリセットする
fn cleanup_battle_units(division_ids: &[DivisionId], military_registry: &mut MilitaryRegistry) {
    for &division_id in division_ids {
        cleanup_single_unit(division_id, military_registry);
    }
}

fn cleanup_single_unit(division_id: DivisionId, military_registry: &mut MilitaryRegistry) {
    if let Some(division) = military_registry.divisions.get_mut(&division_id) {
        if division.status == DivisionStatus::Fighting {
            division.status = DivisionStatus::Idle;
        }
        division.combat_id = None;
    }
}

/// WarRegistry の occupied_states を最新状態に更新する補助関数
fn update_war_occupied_states(
    _attacker_owner: crate::common::CountryId,
    _battle_state: crate::common::StateId,
    _military_registry: &MilitaryRegistry,
    _war_registry: &WarRegistry,
) {
    // WarRegistry は Res（不変参照）なので、war_score.rs の process_war_score が
    // StateRegistry の controller_country を参照して自動更新する。
    // ここでは何もしない（設計上、war_score.rs 側で処理）
}

/// 非戦闘・非移動・自国支配地域にいるユニットの組織率を回復する
pub fn process_org_recovery(
    military_registry: &mut MilitaryRegistry,
    state_registry: &StateRegistry,
) {
    for division in military_registry.divisions.values_mut() {
        // 条件: 戦闘中でない、移動中でない、戦力 > 0
        if division.status == DivisionStatus::Fighting
            || division.status == DivisionStatus::Moving
            || division.status == DivisionStatus::Retreating
            || division.manpower == 0
        {
            continue;
        }

        // 自国支配地域にいるか確認
        let in_own_territory = state_registry
            .get(division.current_state)
            .map(|s| s.controller() == division.owner)
            .unwrap_or(false);

        if in_own_territory {
            division.organization =
                (division.organization + ORG_RECOVERY_PER_DAY).min(division.max_organization);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DivisionDefinitionId, WarId};
    use crate::military::battle::Battle;
    use crate::military::data::{DivisionSize, DivisionType};
    use crate::state::data::StateData;
    use crate::war::data::War;
    use std::collections::HashSet;

    /// P21-siege: 攻撃10/防御15の同型師団同士では、攻撃側が先に組織率0へ達して
    /// 撃退される(=単独攻撃では守備側1個師団を突破できない)ことを検証する再現テスト。
    /// これは元の1v1戦闘バランスが今回の変更で崩れていないことの確認でもある。
    fn make_division(id: usize, owner: CountryId, state: StateId, atk: i32, def: i32) -> Division {
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
            status: DivisionStatus::Fighting,
            def_id: DivisionDefinitionId(0),
            attack_power: atk,
            defense_power: def,
            combat_id: Some(BattleId(0)),
        }
    }

    fn setup_war(attacker: CountryId, defender: CountryId) -> WarRegistry {
        let mut reg = WarRegistry::default();
        let war = War {
            id: WarId(0),
            name: "Test War".to_string(),
            attackers: [attacker].into_iter().collect(),
            defenders: [defender].into_iter().collect(),
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
        reg.wars.insert(war.id, war);
        reg
    }

    /// 1日分の戦闘処理(process_daily_battles → resolve_finished_battles)をまとめて実行する
    fn run_one_day(
        military_registry: &mut MilitaryRegistry,
        state_registry: &mut StateRegistry,
        battle_registry: &mut BattleRegistry,
        war_registry: &WarRegistry,
    ) {
        process_daily_battles(
            military_registry,
            battle_registry,
            state_registry,
            war_registry,
        );
        resolve_finished_battles(
            military_registry,
            state_registry,
            battle_registry,
            war_registry,
        );
    }

    #[test]
    fn test_solo_attacker_cannot_break_well_defended_state() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);

        let origin = StateId(1);
        let target = StateId(2);
        let mut state_reg = StateRegistry::build(vec![
            StateData {
                id: origin,
                owner_country_id: attacker_country,
                ..Default::default()
            },
            StateData {
                id: target,
                owner_country_id: defender_country,
                ..Default::default()
            },
        ]);

        let attacker = make_division(0, attacker_country, target, 10, 10);
        let defender = make_division(1, defender_country, target, 10, 15);

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let mut battle_reg = BattleRegistry::default();
        let battle = Battle {
            id: BattleId(0),
            war_id: WarId(0),
            state_id: target,
            attacker_country,
            defender_country,
            attacker_division_ids: vec![attacker.id],
            defender_division_ids: vec![defender.id],
            attacker_origins: [(attacker.id, origin)].into_iter().collect(),
            start_date: "1800/01/01".to_string(),
            elapsed_days: 0,
            status: BattleStatus::Ongoing,
        };
        battle_reg.start_battle(battle).unwrap();

        // 十分な日数を回しても単独の攻撃側は守備側を突破できない
        for _ in 0..30 {
            run_one_day(&mut mil, &mut state_reg, &mut battle_reg, &war_reg);
            if battle_reg.battles[&BattleId(0)].status != BattleStatus::Ongoing {
                break;
            }
        }

        let battle = &battle_reg.battles[&BattleId(0)];
        assert_eq!(battle.status, BattleStatus::DefenderWon);
        // 攻撃側は撃退されて出撃元へ撤退し、生き残る(撃破はされない)
        assert!(mil.divisions.contains_key(&attacker.id));
        assert_eq!(mil.divisions[&attacker.id].current_state, origin);
        // 州の支配権は変化しない
        assert_eq!(
            state_reg.get(target).unwrap().controller(),
            defender_country
        );
    }

    /// P21-siege(本題): 同じ守備側1個師団に対して、複数の攻撃側師団が合流して
    /// 攻めれば最終的に突破できることを検証する。単独では絶対に勝てない
    /// (上記`test_solo_attacker_cannot_break_well_defended_state`)のと全く同じ
    /// 兵科・州構成であることが重要。
    #[test]
    fn test_combined_multiple_attackers_can_break_well_defended_state() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);

        let origin = StateId(1);
        let target = StateId(2);
        let mut state_reg = StateRegistry::build(vec![
            StateData {
                id: origin,
                owner_country_id: attacker_country,
                ..Default::default()
            },
            StateData {
                id: target,
                owner_country_id: defender_country,
                ..Default::default()
            },
        ]);

        let attacker_a = make_division(0, attacker_country, target, 10, 10);
        let attacker_b = make_division(3, attacker_country, target, 10, 10);
        let attacker_c = make_division(4, attacker_country, target, 10, 10);
        let defender = make_division(1, defender_country, target, 10, 15);

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker_a.id, attacker_a.clone());
        mil.divisions.insert(attacker_b.id, attacker_b.clone());
        mil.divisions.insert(attacker_c.id, attacker_c.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let mut battle_reg = BattleRegistry::default();
        let battle = Battle {
            id: BattleId(0),
            war_id: WarId(0),
            state_id: target,
            attacker_country,
            defender_country,
            attacker_division_ids: vec![attacker_a.id, attacker_b.id, attacker_c.id],
            defender_division_ids: vec![defender.id],
            attacker_origins: [
                (attacker_a.id, origin),
                (attacker_b.id, origin),
                (attacker_c.id, origin),
            ]
            .into_iter()
            .collect(),
            start_date: "1800/01/01".to_string(),
            elapsed_days: 0,
            status: BattleStatus::Ongoing,
        };
        battle_reg.start_battle(battle).unwrap();

        for _ in 0..30 {
            run_one_day(&mut mil, &mut state_reg, &mut battle_reg, &war_reg);
            if battle_reg.battles[&BattleId(0)].status != BattleStatus::Ongoing {
                break;
            }
        }

        let battle = &battle_reg.battles[&BattleId(0)];
        assert_eq!(
            battle.status,
            BattleStatus::AttackerWon,
            "3体の共同攻撃なら、単独では突破できない同じ守備側でも州を落とせるはず"
        );
        // 守備側は排除され(撤退または撃破)、州の支配権は攻撃側に移る
        assert_eq!(
            state_reg.get(target).unwrap().controller(),
            attacker_country
        );
    }

    /// 複数師団のうち一部だけが組織率0で脱落しても、戦闘全体は終わらず、
    /// 残った師団で継続することを検証する(個別脱落 ≠ 陣営全体の敗北)。
    #[test]
    fn test_one_participant_dropping_out_does_not_end_battle_for_survivors() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);

        let origin = StateId(1);
        let target = StateId(2);
        let mut state_reg = StateRegistry::build(vec![
            StateData {
                id: origin,
                owner_country_id: attacker_country,
                ..Default::default()
            },
            StateData {
                id: target,
                owner_country_id: defender_country,
                ..Default::default()
            },
        ]);

        // 攻撃側1体は既に組織率0(直前の戦闘で消耗済みという想定)、もう1体は満タン
        let mut weak_attacker = make_division(0, attacker_country, target, 10, 10);
        weak_attacker.organization = 0.0;
        let strong_attacker = make_division(3, attacker_country, target, 10, 10);
        let defender = make_division(1, defender_country, target, 10, 10);

        let mut mil = MilitaryRegistry::default();
        mil.divisions
            .insert(weak_attacker.id, weak_attacker.clone());
        mil.divisions
            .insert(strong_attacker.id, strong_attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let mut battle_reg = BattleRegistry::default();
        let battle = Battle {
            id: BattleId(0),
            war_id: WarId(0),
            state_id: target,
            attacker_country,
            defender_country,
            attacker_division_ids: vec![weak_attacker.id, strong_attacker.id],
            defender_division_ids: vec![defender.id],
            attacker_origins: [(weak_attacker.id, origin), (strong_attacker.id, origin)]
                .into_iter()
                .collect(),
            start_date: "1800/01/01".to_string(),
            elapsed_days: 0,
            status: BattleStatus::Ongoing,
        };
        battle_reg.start_battle(battle).unwrap();

        run_one_day(&mut mil, &mut state_reg, &mut battle_reg, &war_reg);

        let battle = &battle_reg.battles[&BattleId(0)];
        // 1日目で弱い方は脱落して参加者リストから外れるが、戦闘自体は続く
        assert_eq!(battle.status, BattleStatus::Ongoing);
        assert!(!battle.attacker_division_ids.contains(&weak_attacker.id));
        assert!(battle.attacker_division_ids.contains(&strong_attacker.id));
        // 脱落した師団は出撃元へ撤退しており、破壊はされていない
        assert!(mil.divisions.contains_key(&weak_attacker.id));
        assert_eq!(
            mil.divisions[&weak_attacker.id].status,
            DivisionStatus::Idle
        );
        assert_eq!(mil.divisions[&weak_attacker.id].current_state, origin);
    }
}
