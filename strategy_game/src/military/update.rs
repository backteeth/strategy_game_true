use crate::app::time::DayChangedMessage;
use crate::common::{ArmyId, BattleId};
use crate::country::CountryRegistry;
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::combat_calc::{
    ORG_RECOVERY_PER_DAY, calculate_terrain_defense_bonus, resolve_combat_day,
};
use crate::military::data::{ArmyStatus, MilitaryRegistry};
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

        // 2. 経路の事前検証（戦争終了等で無効になった移動を停止）
        validate_and_stop_invalid_movements(&mut military_registry, &state_registry, &war_registry);

        // 3. 移動処理（侵攻・戦闘開始を含む）
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
            cleanup_battle_units(
                battle.attacker_army_id,
                battle.defender_army_id,
                military_registry,
            );
            continue;
        }

        // 参加ユニットが存在するか確認
        let attacker_exists = military_registry
            .armies
            .contains_key(&battle.attacker_army_id);
        let defender_exists = military_registry
            .armies
            .contains_key(&battle.defender_army_id);

        if !attacker_exists || !defender_exists {
            // ユニット消滅 → 戦闘キャンセル
            if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
                b.status = BattleStatus::Cancelled;
            }
            if attacker_exists {
                cleanup_single_unit(battle.attacker_army_id, military_registry);
            }
            if defender_exists {
                cleanup_single_unit(battle.defender_army_id, military_registry);
            }
            continue;
        }

        // 地形補正取得
        let terrain_bonus = state_registry
            .get(battle.state_id)
            .map(calculate_terrain_defense_bonus)
            .unwrap_or(0);

        // 戦闘計算（クローンして借用衝突を避ける）
        let attacker_clone = match military_registry.armies.get(&battle.attacker_army_id) {
            Some(a) => a.clone(),
            None => continue,
        };
        let defender_clone = match military_registry.armies.get(&battle.defender_army_id) {
            Some(d) => d.clone(),
            None => continue,
        };

        let (atk_loss, atk_org_loss, def_loss, def_org_loss) =
            resolve_combat_day(&attacker_clone, &defender_clone, terrain_bonus);

        // 攻撃側ダメージ適用
        if let Some(army) = military_registry.armies.get_mut(&battle.attacker_army_id) {
            army.manpower = army.manpower.saturating_sub(atk_loss);
            army.organization = (army.organization - atk_org_loss).max(0.0);
        }

        // 防御側ダメージ適用
        if let Some(army) = military_registry.armies.get_mut(&battle.defender_army_id) {
            army.manpower = army.manpower.saturating_sub(def_loss);
            army.organization = (army.organization - def_org_loss).max(0.0);
        }

        // 経過日数更新
        if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
            b.elapsed_days += 1;
        }
    }
}

/// 戦闘終了条件を判定し、後処理を行う
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

        // 参加ユニット取得
        let attacker = military_registry
            .armies
            .get(&battle.attacker_army_id)
            .cloned();
        let defender = military_registry
            .armies
            .get(&battle.defender_army_id)
            .cloned();

        let (Some(atk), Some(def)) = (attacker, defender) else {
            // ユニット消滅 → キャンセル
            if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
                b.status = BattleStatus::Cancelled;
            }
            continue;
        };

        // 終了条件: 戦力0 または 組織率0
        let atk_defeated = atk.manpower == 0 || atk.organization <= 0.0;
        let def_defeated = def.manpower == 0 || def.organization <= 0.0;

        if !atk_defeated && !def_defeated {
            continue; // 戦闘継続
        }

        if def_defeated && !atk_defeated {
            // 防御側敗北 → 攻撃側勝利
            if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
                b.status = BattleStatus::AttackerWon;
            }
            handle_attacker_victory(
                battle.attacker_army_id,
                battle.defender_army_id,
                battle.state_id,
                military_registry,
                state_registry,
                battle_registry,
                war_registry,
            );
        } else if atk_defeated && !def_defeated {
            // 攻撃側敗北 → 防御側勝利
            if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
                b.status = BattleStatus::DefenderWon;
            }
            handle_defender_victory(
                battle.attacker_army_id,
                battle.defender_army_id,
                battle.attacker_origin_state,
                military_registry,
                state_registry,
                battle_registry,
                war_registry,
            );
        } else {
            // 両者同時敗北 → 防御側有利（地域支配変更なし）
            if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
                b.status = BattleStatus::DefenderWon;
            }
            handle_defender_victory(
                battle.attacker_army_id,
                battle.defender_army_id,
                battle.attacker_origin_state,
                military_registry,
                state_registry,
                battle_registry,
                war_registry,
            );
        }
    }
}

/// 攻撃側勝利後の処理
fn handle_attacker_victory(
    attacker_id: ArmyId,
    defender_id: ArmyId,
    battle_state: crate::common::StateId,
    military_registry: &mut MilitaryRegistry,
    state_registry: &mut StateRegistry,
    battle_registry: &BattleRegistry,
    war_registry: &WarRegistry,
) {
    let attacker_owner = military_registry.armies.get(&attacker_id).map(|a| a.owner);

    // 防御側ユニットの処理（撤退または撃破）
    if let Some(def) = military_registry.armies.get(&defender_id).cloned() {
        let retreat_dest = find_retreat_destination(
            defender_id,
            def.current_state,
            military_registry,
            state_registry,
            battle_registry,
        );

        if def.manpower == 0 {
            // 戦力0 → 撃破（削除）
            military_registry.remove_army(defender_id);
            info!("[Battle] Defender Army {:?} destroyed", defender_id);
        } else if let Some(dest) = retreat_dest {
            // 撤退可能な地域あり
            if let Some(army) = military_registry.armies.get_mut(&defender_id) {
                army.current_state = dest;
                army.status = ArmyStatus::Idle;
                army.combat_id = None;
                army.target_state = None;
                army.destination = None;
                army.current_path.clear();
                army.movement_progress = 0.0;
            }
            info!(
                "[Battle] Defender Army {:?} retreated to {:?}",
                defender_id, dest
            );
        } else {
            // 撤退先なし → 包囲・撃破
            military_registry.remove_army(defender_id);
            info!(
                "[Battle] Defender Army {:?} surrounded and destroyed",
                defender_id
            );
        }
    }

    // 攻撃側を戦闘状態から解除し、対象地域へ進入
    if let Some(owner) = attacker_owner {
        if let Some(army) = military_registry.armies.get_mut(&attacker_id) {
            army.status = ArmyStatus::Idle;
            army.combat_id = None;
            army.current_state = battle_state;
        }

        // 地域の支配国を攻撃側に変更
        occupy_state(battle_state, owner, state_registry);
        info!(
            "[Battle] Attacker {:?} captured state {:?}",
            attacker_id, battle_state
        );
    }

    // WarRegistryのoccupied_statesを更新
    if let Some(attacker_owner) = attacker_owner {
        update_war_occupied_states(
            attacker_owner,
            battle_state,
            military_registry,
            war_registry,
        );
    }
}

/// 防御側勝利後の処理
fn handle_defender_victory(
    attacker_id: ArmyId,
    defender_id: ArmyId,
    attacker_origin: crate::common::StateId,
    military_registry: &mut MilitaryRegistry,
    state_registry: &mut StateRegistry,
    battle_registry: &BattleRegistry,
    war_registry: &WarRegistry,
) {
    // 攻撃側の処理（戦力0なら撃破、そうでなければ出発地点に留まる）
    if let Some(atk) = military_registry.armies.get(&attacker_id).cloned() {
        if atk.manpower == 0 {
            // 撃破
            military_registry.remove_army(attacker_id);
            info!("[Battle] Attacker Army {:?} destroyed", attacker_id);
        } else {
            // 出発地点へ戻る（既に出発地点にいる想定だが念のため）
            if let Some(army) = military_registry.armies.get_mut(&attacker_id) {
                army.current_state = attacker_origin;
                army.status = ArmyStatus::Idle;
                army.combat_id = None;
                army.destination = None;
                army.current_path.clear();
                army.target_state = None;
                army.movement_progress = 0.0;
            }
            info!(
                "[Battle] Attacker Army {:?} repelled, returns to {:?}",
                attacker_id, attacker_origin
            );
        }
    }

    // 防御側ユニットの後処理
    if let Some(def) = military_registry.armies.get(&defender_id).cloned() {
        if def.manpower == 0 {
            // 両者敗北の場合は防御側も削除されることがある（この関数は両者敗北でも呼ばれる）
            military_registry.remove_army(defender_id);
        } else if let Some(army) = military_registry.armies.get_mut(&defender_id) {
            army.status = ArmyStatus::Idle;
            army.combat_id = None;
        }
    }

    // 地域の支配国は変更しない
    let _ = (state_registry, battle_registry, war_registry);
    info!("[Battle] Defender won. Territory unchanged.");
}

/// 戦闘後に両ユニットの戦闘状態をリセットする
fn cleanup_battle_units(
    attacker_id: ArmyId,
    defender_id: ArmyId,
    military_registry: &mut MilitaryRegistry,
) {
    cleanup_single_unit(attacker_id, military_registry);
    cleanup_single_unit(defender_id, military_registry);
}

fn cleanup_single_unit(army_id: ArmyId, military_registry: &mut MilitaryRegistry) {
    if let Some(army) = military_registry.armies.get_mut(&army_id) {
        if army.status == ArmyStatus::Fighting {
            army.status = ArmyStatus::Idle;
        }
        army.combat_id = None;
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
    for army in military_registry.armies.values_mut() {
        // 条件: 戦闘中でない、移動中でない、戦力 > 0
        if army.status == ArmyStatus::Fighting
            || army.status == ArmyStatus::Moving
            || army.status == ArmyStatus::Retreating
            || army.manpower == 0
        {
            continue;
        }

        // 自国支配地域にいるか確認
        let in_own_territory = state_registry
            .get(army.current_state)
            .map(|s| s.controller() == army.owner)
            .unwrap_or(false);

        if in_own_territory {
            army.organization =
                (army.organization + ORG_RECOVERY_PER_DAY).min(army.max_organization);
        }
    }
}
