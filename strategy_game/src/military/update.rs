use crate::app::time::DayChangedMessage;
use crate::common::{BattleId, DivisionId, StateId};
use crate::country::CountryRegistry;
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::combat_calc::{
    ORG_RECOVERY_PER_DAY, RETREAT_MANPOWER_LOSS_RATIO, calculate_combat_readiness_with_supply,
    calculate_terrain_defense_bonus, resolve_combat_day_multi_with_supply,
};
use crate::military::data::{Division, DivisionStatus, MilitaryRegistry};
use crate::military::equipment::{calculate_proportional_equipment_loss, try_destroy_equipment};
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
        let blocked_by_equipment_shortage = crate::military::recruitment::process_recruitment(
            &mut country_registry,
            &mut military_registry,
        );

        // 2. 日次自動補充(P23-008)。その日の戦闘開始・損耗より前に行うことで、
        // 「その日の戦闘損失を同日中に補充してしまう」ことを避ける
        // (補充は前日までの損耗のみを対象とする)。
        crate::military::reinforcement::apply_daily_reinforcement(
            &mut country_registry,
            &mut military_registry,
            &state_registry,
            &battle_registry,
        );

        // 2.5. 日次装備補充(P25-005)。manpower補充の直後・戦闘処理より前に行う
        // ことで、その日の戦闘装備損失(P25-004)を同日中に補填してしまわないよう
        // にする。完了済み募兵キューが装備不足で待機した国家は、当日の既存
        // Division向け補充予算を0にして備蓄を保護する(§6)。
        crate::military::equipment_reinforcement::apply_daily_equipment_reinforcement(
            &mut country_registry,
            &mut military_registry,
            &state_registry,
            &battle_registry,
            &blocked_by_equipment_shortage,
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

        // 4. 日次戦闘計算（戦闘IDの昇順で処理）。その日どちらの陣営も損害を
        // 与えられなかった戦闘のIDを、状態変更前に確定した当日結果から返す(P27-007)。
        let zero_damage_battles_today = process_daily_battles(
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
            &zero_damage_battles_today,
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
/// P27-007: 当日の戦闘計算で攻撃側・防御側のどちらも1点もmanpower損害を与え
/// られなかった戦闘のID集合を返す型。Bevy Resourceにも永続フィールドにもせず、
/// `handle_daily_military`内で`process_daily_battles`から`resolve_finished_battles`へ
/// 同一tick内の関数引数として直接受け渡すだけの、非永続なローカル値である。
type ZeroDamageBattles = std::collections::HashSet<BattleId>;

fn process_daily_battles(
    military_registry: &mut MilitaryRegistry,
    battle_registry: &mut BattleRegistry,
    state_registry: &StateRegistry,
    war_registry: &WarRegistry,
) -> ZeroDamageBattles {
    let mut zero_damage_battles: ZeroDamageBattles = ZeroDamageBattles::new();

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

        // P25-007(§15)/P26-005(§9): 戦闘充足率(manpower/装備/補給状態)の妥当性を、
        // この戦闘の当日解決(manpower/equipment/organization/battle statisticsの更新、
        // elapsed_days加算)を一切行う前に全参加Divisionについて検証する。
        // 1件でも不正(NaN装備・manpower>max_manpower・補給充足率が非有限または
        // 0.0..=1.0の範囲外等)なら、このBattleの当日解決を丸ごとskipし(他のBattle・
        // 他国家には影響しない)、次のBattleへ進む。補給充足率は`process_daily_battles`
        // より前(`DailySimulationSet::Economy`)で当日分が確定済みの値を
        // `MilitaryRegistry::supply_fulfillment_of`経由で読み取る(P26-004)。
        // 乱数はこのコードベースの戦闘計算がそもそも使用しないため、
        // 「RNGを消費しない」は構造的に自明に満たされる。
        let readiness_invalid = attacker_divisions
            .iter()
            .chain(defender_divisions.iter())
            .any(|d| {
                calculate_combat_readiness_with_supply(
                    d.manpower,
                    d.max_manpower,
                    d.equipment,
                    d.max_equipment,
                    military_registry.supply_fulfillment_of(d.id),
                )
                .is_err()
            });
        if readiness_invalid {
            error!(
                "[Battle] battle {:?} has a participant with invalid combat readiness state \
                 (manpower/equipment/supply out of the canonical range); skipping resolution this tick",
                battle_id
            );
            continue;
        }

        let attacker_refs: Vec<&Division> = attacker_divisions.iter().collect();
        let defender_refs: Vec<&Division> = defender_divisions.iter().collect();

        let (atk_results, def_results) =
            resolve_combat_day_multi_with_supply(&attacker_refs, &defender_refs, terrain_bonus, military_registry);

        // P27-007(§3.1): 状態を一切変更する前に、今日attacker側・defender側が
        // 実際に受け取った損害(manpower_loss)だけを見て「両陣営ともゼロ損害」を
        // 判定する。`defender`が受ける損害は攻撃側の合計実効戦力に、`attacker`が
        // 受ける損害は防御側の合計実効戦力に、それぞれ比例する(P27-006の
        // `damage_from`)ため、この判定は「双方の合計実効戦力が0」と数学的に同値
        // であり、実効戦力を別途再計算する必要はない。`resolve_finished_battles`側で
        // 適用後のorganizationから事後的に再判定すると、今日の戦闘で両者が同時に
        // 組織率0まで削り合った正常な相討ちを誤って両陣営ゼロ損害と混同する
        // (P27-007-PRE-001で実際に踏んで特定した罠)。
        let attacker_received_damage = atk_results.values().any(|(manpower_loss, _)| *manpower_loss > 0);
        let defender_received_damage = def_results.values().any(|(manpower_loss, _)| *manpower_loss > 0);
        if !attacker_received_damage && !defender_received_damage {
            zero_damage_battles.insert(battle_id);
        }

        for (division_id, (manpower_loss, org_loss)) in atk_results {
            apply_combat_casualty(military_registry, division_id, manpower_loss, org_loss);
        }
        for (division_id, (manpower_loss, org_loss)) in def_results {
            apply_combat_casualty(military_registry, division_id, manpower_loss, org_loss);
        }

        // 経過日数更新
        if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
            b.elapsed_days += 1;
        }
    }
    zero_damage_battles
}

/// 1体のDivisionへ、戦闘計算が要求したmanpower/organization損失を原子的に適用する
/// (P25-004)。`saturating_sub`によって実際に減少したmanpower量(actual casualty、
/// 予定damageが現在manpowerを超えても現在manpowerでclampされた後の値)に比例して、
/// damage適用直前の現在装備量(`max_equipment`ではない)を基準に装備を破壊する。
///
/// manpower・equipment・organizationは同一の成功処理として扱う: 装備損失の計算
/// または`try_destroy_equipment`が失敗した場合(現在の正常ゲーム経路では発生しない
/// はずの異常状態)、この師団のmanpower/organizationも一切更新せず、次の日次
/// イベントへ持ち越す。国家のmilitary equipment備蓄には一切触れない
/// (破壊された装備は備蓄へ戻らない、§12)。
fn apply_combat_casualty(
    military_registry: &mut MilitaryRegistry,
    division_id: DivisionId,
    manpower_loss: u64,
    org_loss: f32,
) {
    let Some(division) = military_registry.divisions.get_mut(&division_id) else {
        return;
    };

    let manpower_before = division.manpower;
    let manpower_after = manpower_before.saturating_sub(manpower_loss);
    let actual_casualty = manpower_before - manpower_after;

    let loss = match calculate_proportional_equipment_loss(
        division.equipment,
        manpower_before,
        actual_casualty,
    ) {
        Ok(loss) => loss,
        Err(err) => {
            error!(
                "[Battle] equipment loss calculation failed for {:?} (equipment={}, manpower_before={}, actual_casualty={}): {:?}; casualty not applied this tick",
                division_id, division.equipment, manpower_before, actual_casualty, err
            );
            return;
        }
    };

    if let Err(err) = try_destroy_equipment(&mut division.equipment, loss.destroyed) {
        error!(
            "[Battle] try_destroy_equipment failed for {:?} (destroyed={}): {:?}; casualty not applied this tick",
            division_id, loss.destroyed, err
        );
        return;
    }

    division.manpower = manpower_after;
    division.organization = (division.organization - org_loss).max(0.0);
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
    zero_damage_battles_today: &ZeroDamageBattles,
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

        // P27-007: 今日どちらの陣営も損害を与えられなかった戦闘は、防御側勝利
        // として即日終了し、攻撃側全員を出撃元(`attacker_origins`)へ撤退させる
        // (既存の組織率0撤退・10%ペナルティ・比例装備損失をそのまま再利用する)。
        // 防御側は既存の「攻撃側全滅→防御側勝利」分岐と同じ状態変更を受ける。
        if zero_damage_battles_today.contains(&battle_id) {
            if let Some(b) = battle_registry.battles.get_mut(&battle_id) {
                b.status = BattleStatus::DefenderWon;
            }
            for &division_id in &battle.attacker_division_ids {
                let origin = battle.attacker_origins.get(&division_id).copied();
                force_retreat_or_destroy(division_id, origin, military_registry);
            }
            for &division_id in &battle.defender_division_ids {
                if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                    division.status = DivisionStatus::Idle;
                    division.combat_id = None;
                }
            }
            info!(
                "[Battle] {:?} both sides dealt zero damage today in state {:?}; \
                 defender holds, attacker retreats",
                battle_id, battle.state_id
            );
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
        // P25-006-REV-001: 本番のDivision削除直前は必ずequipment==0でなければ
        // ならない(§5共通不変条件)。通常戦闘経路(P25-004の`apply_combat_casualty`)
        // は既にこれを保証しているため、ここでの`remaining_equipment > 0.0`は
        // 通常到達しないはずの防御的分岐だが、将来の経路変更やロード直後の
        // 不整合データに備え、削除前に必ず全損させてから削除する
        // (返却・回収は行わない)。
        let Some(d) = military_registry.divisions.get_mut(&division_id) else {
            return false;
        };
        // 常にtry_destroy_equipmentを呼ぶ(destroyed=0.0は既存どおり安全なno-op)。
        // `equipment > 0.0`だけをガードにすると、NaNは`NaN > 0.0`が常にfalseと
        // なるため誤って検証をスキップしてしまう(不正状態が未検証のまま
        // 削除されてしまうバグの温床)。
        let remaining_equipment = d.equipment;
        if let Err(err) = try_destroy_equipment(&mut d.equipment, remaining_equipment) {
            error!(
                "[Battle] failed to destroy remaining equipment for annihilated division {:?}: {:?}; removal deferred this tick",
                division_id, err
            );
            return true;
        }
        military_registry.remove_division(division_id);
        info!("[Battle] Division {:?} destroyed", division_id);
        return false;
    }

    if division.organization > 0.0 {
        return true; // まだ戦闘継続可能
    }

    // 組織率0 → 撤退または撃破(P27-007で`force_retreat_or_destroy`として
    // 独立させ、両陣営ゼロ損害による強制撤退とも共有する)。
    force_retreat_or_destroy(division_id, retreat_destination, military_registry)
}

/// 脱落した参加師団を、指定した退路(Some、かつ撤退後manpower>0)なら撤退、
/// それ以外(退路なし、または撤退追加casualtyでmanpowerが尽きる)なら撃破する。
/// 撤退時には追加でmanpower損失(`RETREAT_MANPOWER_LOSS_RATIO`)を与え、
/// 同じ師団が「撤退→組織率回復→再戦」を無限に繰り返せないようにする。
///
/// 組織率0による脱落(`resolve_participant_outcome`)と、P27-007の両陣営
/// ゼロ損害による強制撤退(`resolve_finished_battles`)の両方から呼ばれる
/// 共通処理。一時的なエラー(装備破壊失敗)で処理を見送る場合だけ`true`
/// (戦闘継続扱い、次のtickで再試行)を返し、それ以外は常に`false`
/// (この師団は戦闘参加者ではなくなった)を返す。
fn force_retreat_or_destroy(
    division_id: DivisionId,
    retreat_destination: Option<StateId>,
    military_registry: &mut MilitaryRegistry,
) -> bool {
    let Some(division) = military_registry.divisions.get(&division_id).cloned() else {
        return false; // 既に消滅済み
    };

    let manpower_before = division.manpower;
    let retreat_loss = (division.max_manpower as f32 * RETREAT_MANPOWER_LOSS_RATIO) as u64;
    let remaining_manpower = manpower_before.saturating_sub(retreat_loss);
    // P25-006-REV-001: 予定retreat_lossではなく、実際に減少した量(既存の
    // saturating_subによるclamp後の値)をactual casualtyとして装備損失へ使う
    // (P25-004のapply_combat_casualtyと同じ「実際の減少量を使う」原則)。
    let actual_casualty = manpower_before - remaining_manpower;

    match retreat_destination {
        Some(dest) if remaining_manpower > 0 => {
            // 撤退成功: 撤退追加casualtyに比例する装備だけをP25-004と同じ式
            // (calculate_proportional_equipment_loss)で破壊する。通常戦闘損失
            // (このDivisionが同日既に受けたcombat casualty)には一切触れない
            // (既にprocess_daily_battles側で独立して処理済み、二重計上しない)。
            let loss_plan = match calculate_proportional_equipment_loss(
                division.equipment,
                manpower_before,
                actual_casualty,
            ) {
                Ok(plan) => plan,
                Err(err) => {
                    error!(
                        "[Battle] retreat equipment loss calculation failed for {:?}: {:?}; retreat not applied this tick",
                        division_id, err
                    );
                    return true;
                }
            };
            let Some(d) = military_registry.divisions.get_mut(&division_id) else {
                return false;
            };
            if let Err(err) = try_destroy_equipment(&mut d.equipment, loss_plan.destroyed) {
                error!(
                    "[Battle] try_destroy_equipment failed during retreat for {:?}: {:?}; retreat not applied this tick",
                    division_id, err
                );
                return true;
            }
            d.manpower = remaining_manpower;
            d.current_state = dest;
            d.status = DivisionStatus::Idle;
            d.combat_id = None;
            d.target_state = None;
            d.destination = None;
            d.current_path.clear();
            d.movement_progress = 0.0;
            info!(
                "[Battle] Division {:?} retreated to {:?} (manpower: {})",
                division_id, dest, remaining_manpower
            );
            false
        }
        _ => {
            // 強制削除(撤退先なし、または撤退追加casualtyでmanpowerが0になった)。
            // §5の共通不変条件どおり、削除前に現在装備を全損させる
            // (返却・回収は行わない。国家備蓄には一切触れない)。
            let Some(d) = military_registry.divisions.get_mut(&division_id) else {
                return false;
            };
            // 常にtry_destroy_equipmentを呼ぶ(destroyed=0.0は既存どおり安全な
            // no-op)。`equipment > 0.0`だけをガードにすると、NaNは`NaN > 0.0`が
            // 常にfalseとなるため誤って検証をスキップしてしまう。
            let remaining_equipment = d.equipment;
            if let Err(err) = try_destroy_equipment(&mut d.equipment, remaining_equipment) {
                error!(
                    "[Battle] failed to destroy remaining equipment for {:?} during forced removal: {:?}; removal deferred this tick",
                    division_id, err
                );
                return true;
            }
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

    /// P25-004: `make_division`より細かく状態を制御できる版(manpower/equipment/
    /// max_equipmentを個別に指定できる)。既存の`make_division`はP21-siegeの
    /// テストが直接依存しているため、シグネチャを変えずこちらを新設する。
    #[allow(clippy::too_many_arguments)]
    fn make_division_ex(
        id: usize,
        owner: CountryId,
        state: StateId,
        manpower: u64,
        max_manpower: u64,
        equipment: f64,
        max_equipment: f64,
        atk: i32,
        def: i32,
    ) -> Division {
        Division {
            id: DivisionId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: state,
            destination: None,
            current_path: vec![],
            target_state: None,
            manpower,
            max_manpower,
            equipment,
            max_equipment,
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

    /// P25-004: 単一州での1つの戦闘を組み立てる(`BattleRegistry::start_battle`が
    /// 実際のIDを採番するため、`id`フィールドの値自体は使われない)。
    fn make_battle(
        attacker_country: CountryId,
        defender_country: CountryId,
        state: StateId,
        attacker_ids: Vec<DivisionId>,
        defender_ids: Vec<DivisionId>,
    ) -> Battle {
        let attacker_origins = attacker_ids.iter().map(|id| (*id, state)).collect();
        Battle {
            id: BattleId(0),
            war_id: WarId(0),
            state_id: state,
            attacker_country,
            defender_country,
            attacker_division_ids: attacker_ids,
            defender_division_ids: defender_ids,
            attacker_origins,
            start_date: "1800/01/01".to_string(),
            elapsed_days: 0,
            status: BattleStatus::Ongoing,
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
            source_crisis_id: None,
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
        let zero_damage_battles_today = process_daily_battles(
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
            &zero_damage_battles_today,
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

    // ─── P25-004: 戦闘損耗による軍事装備損失 ─────────────────────────────
    // process_daily_battles/resolve_finished_battlesは`military::update`の
    // 稼働中の戦闘経路そのもの(privateのため、この内部テストモジュールからのみ
    // 直接呼び出せる)。

    use crate::economy::resources::military_equipment_units_to_value;

    /// Test I: 予定damageがmanpowerを超えても、実際に減少した量(manpower_before)
    /// だけを装備損失の基準に使い、Division全滅時は現在装備を全損させる。
    #[test]
    fn test_i_overkill_damage_uses_the_manpower_clamped_actual_casualty() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let state_reg = StateRegistry::build(vec![]);

        // 攻撃力を極端に大きくし、1日で守備側のmanpower(50)を大幅に上回る
        // 予定damageを発生させる(defenderの実効戦力はほぼ0)。
        let attacker = make_division_ex(
            0,
            attacker_country,
            target,
            10_000,
            10_000,
            100.0,
            100.0,
            100_000,
            10,
        );
        let defender =
            make_division_ex(1, defender_country, target, 50, 10_000, 20.0, 100.0, 10, 10);

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(
                attacker_country,
                defender_country,
                target,
                vec![attacker.id],
                vec![defender.id],
            ))
            .unwrap();

        let zero_damage_battles_today = process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        let defender_after = mil.divisions.get(&defender.id).unwrap();
        assert_eq!(
            defender_after.manpower, 0,
            "予定damageがmanpowerを大幅に超えるため、1日でmanpowerは0になるはず"
        );
        assert_eq!(
            defender_after.equipment, 0.0,
            "actual casualty(=manpower_before)が現在装備を全損させるはず"
        );

        let mut state_reg = state_reg;
        resolve_finished_battles(
            &mut mil,
            &mut state_reg,
            &mut battle_reg,
            &war_reg,
            &zero_damage_battles_today,
        );
        assert!(
            !mil.divisions.contains_key(&defender.id),
            "manpower=0のDivisionはresolve_finished_battlesで除去されるはず"
        );
    }

    /// Test J: 装備損失は`max_equipment`を変更しない(この関数はそもそも
    /// `max_equipment`を引数に取らない)。
    #[test]
    fn test_j_max_equipment_is_never_changed_by_combat_losses() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let state_reg = StateRegistry::build(vec![]);

        let attacker = make_division(0, attacker_country, target, 10, 10);
        let defender = make_division(1, defender_country, target, 10, 10);
        let max_equipment_before = defender.max_equipment;

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(
                attacker_country,
                defender_country,
                target,
                vec![attacker.id],
                vec![defender.id],
            ))
            .unwrap();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        let defender_after = mil.divisions.get(&defender.id).unwrap();
        assert_eq!(defender_after.max_equipment, max_equipment_before);
        assert!(
            defender_after.equipment < defender.equipment,
            "同条件の戦闘なので現在装備は実際に減っているはず"
        );
    }

    /// Test K: 戦闘で破壊された装備は国家備蓄(`CountryStockpile[MilitaryEquipment]`)
    /// へ一切戻らない。`process_daily_battles`/`resolve_finished_battles`は
    /// そもそも`CountryStockpile`を引数に取らないため、構造的に不変であることを
    /// ここで明示的に確認する。
    #[test]
    fn test_k_national_equipment_stockpile_is_never_touched_by_combat() {
        use crate::economy::resources::{CountryStockpile, ResourceType};

        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let state_reg = StateRegistry::build(vec![]);

        let attacker = make_division(0, attacker_country, target, 10, 10);
        let defender = make_division(1, defender_country, target, 10, 15);

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(
                attacker_country,
                defender_country,
                target,
                vec![attacker.id],
                vec![defender.id],
            ))
            .unwrap();

        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::MilitaryEquipment, 500.0);
        let stockpile_bits_before = stockpile.get(ResourceType::MilitaryEquipment).to_bits();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        assert_eq!(
            stockpile.get(ResourceType::MilitaryEquipment).to_bits(),
            stockpile_bits_before,
            "国家備蓄は戦闘関数へ一切渡されないため、ビット単位で不変のはず"
        );
    }

    /// Test L: L/K整合。破壊された装備量とは無関係に、Kの増分・Lの減分は
    /// actual casualtyと厳密に一致する(装備がKへ混入しない)。
    #[test]
    fn test_l_manpower_ledger_k_and_l_move_by_exactly_the_actual_casualty() {
        use crate::country::CountryData;
        use crate::military::manpower::compute_manpower_ledger;

        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let state_reg = StateRegistry::build(vec![]);

        let attacker = make_division(0, attacker_country, target, 10, 10);
        let defender = make_division(1, defender_country, target, 10, 15);

        let defender_country_data = CountryData {
            id: defender_country,
            mobilized_manpower: defender.manpower,
            ..CountryData::default()
        };

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let ledger_before = compute_manpower_ledger(&defender_country_data, &mil).unwrap();

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(
                attacker_country,
                defender_country,
                target,
                vec![attacker.id],
                vec![defender.id],
            ))
            .unwrap();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        let actual_casualty = defender.manpower - mil.divisions[&defender.id].manpower;
        assert!(actual_casualty > 0, "この対戦条件では守備側も損耗するはず");

        let ledger_after = compute_manpower_ledger(&defender_country_data, &mil).unwrap();
        assert_eq!(
            ledger_before.living_military_manpower - ledger_after.living_military_manpower,
            actual_casualty,
            "Lの減少量はactual casualtyと厳密に一致するはず"
        );
        assert_eq!(
            ledger_after.permanent_military_losses - ledger_before.permanent_military_losses,
            actual_casualty,
            "Kの増加量はactual casualtyと厳密に一致するはず(装備量が混入しない)"
        );
    }

    /// Test M: 装備状態が不正(NaN)な場合、装備・manpower・organizationのいずれも
    /// mutateされず、この師団に対する当日のcasualty適用全体がスキップされる。
    #[test]
    fn test_m_corrupted_equipment_state_blocks_the_entire_casualty_application_atomically() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let state_reg = StateRegistry::build(vec![]);

        let attacker = make_division(0, attacker_country, target, 10, 10);
        let mut defender = make_division(1, defender_country, target, 10, 15);
        defender.equipment = f64::NAN; // 現在の正常経路では発生しないはずの破損状態

        let manpower_before = defender.manpower;
        let organization_before = defender.organization;

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(
                attacker_country,
                defender_country,
                target,
                vec![attacker.id],
                vec![defender.id],
            ))
            .unwrap();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        let defender_after = mil.divisions.get(&defender.id).unwrap();
        assert_eq!(
            defender_after.manpower, manpower_before,
            "装備会計が失敗した場合、manpowerも変化してはいけない"
        );
        assert_eq!(
            defender_after.organization, organization_before,
            "装備会計が失敗した場合、organizationも変化してはいけない"
        );
        assert!(
            defender_after.equipment.is_nan(),
            "元の(不正な)装備状態がそのまま残るはず(黙ってclampされない)"
        );
    }

    /// Test O: 複数師団はそれぞれ自身の現在装備量を基準に独立して損失を計算する
    /// (プールされた/共有された値を使わない)。
    #[test]
    fn test_o_multiple_divisions_use_their_own_current_equipment_independently() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let state_reg = StateRegistry::build(vec![]);

        let attacker_a = make_division_ex(
            0,
            attacker_country,
            target,
            10_000,
            10_000,
            100.0,
            100.0,
            10,
            10,
        );
        let attacker_b = make_division_ex(
            3,
            attacker_country,
            target,
            10_000,
            10_000,
            50.0,
            50.0,
            10,
            10,
        );
        let defender = make_division_ex(
            1,
            defender_country,
            target,
            10_000,
            10_000,
            100.0,
            100.0,
            10,
            10,
        );

        let mut mil = MilitaryRegistry::default();
        for d in [&attacker_a, &attacker_b, &defender] {
            mil.divisions.insert(d.id, d.clone());
        }

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(
                attacker_country,
                defender_country,
                target,
                vec![attacker_a.id, attacker_b.id],
                vec![defender.id],
            ))
            .unwrap();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        for before in [&attacker_a, &attacker_b] {
            let after = mil.divisions.get(&before.id).unwrap();
            let actual_casualty = before.manpower - after.manpower;
            let expected = calculate_proportional_equipment_loss(
                before.equipment,
                before.manpower,
                actual_casualty,
            )
            .unwrap();
            assert_eq!(
                after.equipment,
                military_equipment_units_to_value(expected.equipment_units_after),
                "division {:?} must use its own current equipment, not a shared/pooled value",
                before.id
            );
        }
        assert_ne!(
            mil.divisions[&attacker_a.id].equipment,
            mil.divisions[&attacker_b.id].equipment,
            "開始時の現在装備が異なれば、結果も異なるはず(プールされていないことの証明)"
        );
    }

    /// Test P: 戦闘に参加していない別国家のDivisionは、装備・manpowerとも一切
    /// 変化しない。
    #[test]
    fn test_p_uninvolved_countrys_division_is_never_touched() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let bystander_country = CountryId(3);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let elsewhere = StateId(99);
        let state_reg = StateRegistry::build(vec![]);

        let attacker = make_division(0, attacker_country, target, 10, 10);
        let defender = make_division(1, defender_country, target, 10, 15);
        let bystander =
            make_division_ex(5, bystander_country, elsewhere, 10_000, 10_000, 77.0, 77.0, 10, 10);

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());
        mil.divisions.insert(bystander.id, bystander.clone());

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(
                attacker_country,
                defender_country,
                target,
                vec![attacker.id],
                vec![defender.id],
            ))
            .unwrap();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        let bystander_after = mil.divisions.get(&bystander.id).unwrap();
        assert_eq!(bystander_after.manpower, bystander.manpower);
        assert_eq!(bystander_after.equipment, bystander.equipment);
    }

    /// Test Q: attacker/defenderは同一の式で装備損失を計算する(特別扱いがない)。
    #[test]
    fn test_q_attacker_and_defender_use_the_identical_loss_formula() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let state_reg = StateRegistry::build(vec![]);

        // combat_calc.rsのtest_combat_is_deterministicと同じ非対称なstatsを使い、
        // 両陣営が異なるcasualtyを受けることを保証する。
        let attacker =
            make_division_ex(0, attacker_country, target, 8_000, 10_000, 90.0, 100.0, 12, 8);
        let defender =
            make_division_ex(1, defender_country, target, 6_000, 10_000, 60.0, 100.0, 9, 14);

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(
                attacker_country,
                defender_country,
                target,
                vec![attacker.id],
                vec![defender.id],
            ))
            .unwrap();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        for before in [&attacker, &defender] {
            let after = mil.divisions.get(&before.id).unwrap();
            let actual_casualty = before.manpower - after.manpower;
            assert!(actual_casualty > 0, "両陣営とも損耗するはずの非対称statsのはず");
            let expected = calculate_proportional_equipment_loss(
                before.equipment,
                before.manpower,
                actual_casualty,
            )
            .unwrap();
            assert_eq!(
                after.equipment,
                military_equipment_units_to_value(expected.equipment_units_after),
                "division {:?} must follow the same canonical formula regardless of attacker/defender role",
                before.id
            );
        }
    }

    /// Test R: 同一の初期条件・同一日数なら、独立した2回の実行結果は完全に一致する
    /// (追加のRNGを消費しない決定的な計算であることの確認)。
    #[test]
    fn test_r_repeated_runs_from_identical_inputs_are_bit_for_bit_deterministic() {
        fn run_scenario() -> (u64, f64, u64, f64) {
            let attacker_country = CountryId(1);
            let defender_country = CountryId(2);
            let war_reg = setup_war(attacker_country, defender_country);
            let target = StateId(2);
            let mut state_reg = StateRegistry::build(vec![]);

            let attacker = make_division_ex(
                0,
                attacker_country,
                target,
                10_000,
                10_000,
                83.3,
                100.0,
                11,
                9,
            );
            let defender = make_division_ex(
                1,
                defender_country,
                target,
                9_500,
                10_000,
                61.7,
                100.0,
                9,
                11,
            );

            let mut mil = MilitaryRegistry::default();
            mil.divisions.insert(attacker.id, attacker.clone());
            mil.divisions.insert(defender.id, defender.clone());

            let mut battle_reg = BattleRegistry::default();
            battle_reg
                .start_battle(make_battle(
                    attacker_country,
                    defender_country,
                    target,
                    vec![attacker.id],
                    vec![defender.id],
                ))
                .unwrap();

            for _ in 0..5 {
                run_one_day(&mut mil, &mut state_reg, &mut battle_reg, &war_reg);
                if !battle_reg.battles.contains_key(&BattleId(0))
                    || battle_reg.battles[&BattleId(0)].status != BattleStatus::Ongoing
                {
                    break;
                }
            }

            let a = mil.divisions.get(&attacker.id);
            let d = mil.divisions.get(&defender.id);
            (
                a.map(|d| d.manpower).unwrap_or(0),
                a.map(|d| d.equipment).unwrap_or(0.0),
                d.map(|x| x.manpower).unwrap_or(0),
                d.map(|x| x.equipment).unwrap_or(0.0),
            )
        }

        let run1 = run_scenario();
        let run2 = run_scenario();
        assert_eq!(run1, run2);
    }

    /// Test S(P25-007で意図的に反転): P25-004時点では、現在装備の有無・多寡は
    /// manpower/organization casualtyの結果に一切影響しなかった(装備損失の
    /// 計算[事後会計]が戦闘力計算[事前]へフィードバックしないことの確認だった)。
    /// P25-007は、この非接続を意図的に解消し、戦闘充足率(`min(manpower比率,
    /// 装備比率)`)を通じてDivisionのattack/defenseへ装備状態を反映するため、
    /// 現在ではequipment量がmanpower/organization casualtyへ直接影響する
    /// (これはP25-007タスク自身が要求する仕様どおりの挙動であり、退行ではない)。
    /// 新しい期待値: 双方装備0(readiness=0)なら戦闘力0となりcasualty自体が
    /// 発生しない(膠着)。双方装備満タン(readiness=100%)ならP25-004時点と
    /// 同じ通常の被害が生じる。
    #[test]
    fn test_s_equipment_amount_now_intentionally_affects_manpower_and_organization_casualties() {
        fn run_scenario(attacker_equipment: f64, defender_equipment: f64) -> (u64, f32, u64, f32) {
            let attacker_country = CountryId(1);
            let defender_country = CountryId(2);
            let war_reg = setup_war(attacker_country, defender_country);
            let target = StateId(2);
            let state_reg = StateRegistry::build(vec![]);

            let attacker = make_division_ex(
                0,
                attacker_country,
                target,
                10_000,
                10_000,
                attacker_equipment,
                100.0,
                10,
                10,
            );
            let defender = make_division_ex(
                1,
                defender_country,
                target,
                10_000,
                10_000,
                defender_equipment,
                100.0,
                10,
                15,
            );

            let mut mil = MilitaryRegistry::default();
            mil.divisions.insert(attacker.id, attacker.clone());
            mil.divisions.insert(defender.id, defender.clone());

            let mut battle_reg = BattleRegistry::default();
            battle_reg
                .start_battle(make_battle(
                    attacker_country,
                    defender_country,
                    target,
                    vec![attacker.id],
                    vec![defender.id],
                ))
                .unwrap();

            process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

            let a = &mil.divisions[&attacker.id];
            let d = &mil.divisions[&defender.id];
            (a.manpower, a.organization, d.manpower, d.organization)
        }

        let with_equipment = run_scenario(100.0, 100.0);
        let without_equipment = run_scenario(0.0, 0.0);

        assert_eq!(
            without_equipment,
            (10_000, 100.0, 10_000, 100.0),
            "双方の戦闘充足率が0(装備0)なら、attack/defenseとも0になり\
             casualty自体が発生しない(膠着、P25-007で追加した新しい仕様)"
        );
        assert!(
            with_equipment.0 < 10_000,
            "装備満タン(readiness 100%)なら、P25-004時点と同じ通常のcasualtyが生じるはず"
        );
        assert_ne!(
            with_equipment, without_equipment,
            "P25-007により、装備量の違いはmanpower/organization casualtyへ意図的に反映されるはず"
        );
    }

    /// Test U: `war::combat::process_combat`(P25-001監査で死んだ経路と確認済み)は、
    /// P25-004で一切変更・接続されていない。新しい固定小数点API経由なら
    /// `QuantityOutOfRange`として拒否されるはずの規模の値でも、この関数は依然として
    /// 素朴な`f64`減算のまま処理を完了する(=別経路のままであることの確認)。
    #[test]
    fn test_u_dead_war_combat_process_combat_path_remains_unconnected() {
        use crate::economy::resources::military_equipment_value_to_units;
        use crate::military::data::DivisionDefinition;
        use crate::war::combat::process_combat;

        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);

        // MILITARY_EQUIPMENT_MAX_UNITS/SCALE(=1e9)を超える規模。新しいfixed-point
        // APIならQuantityOutOfRangeとして即座に拒否されるはずの値であることを確認する。
        let out_of_range_equipment = 2_000_000_000.0;
        assert!(
            military_equipment_value_to_units(out_of_range_equipment).is_err(),
            "この値は新しいfixed-point APIの正規値域を超えているはず(前提条件の確認)"
        );

        let a = make_division_ex(
            0,
            attacker_country,
            target,
            10_000,
            10_000,
            out_of_range_equipment,
            out_of_range_equipment,
            10,
            10,
        );
        let b = make_division_ex(
            1,
            defender_country,
            target,
            10_000,
            10_000,
            out_of_range_equipment,
            out_of_range_equipment,
            10,
            10,
        );

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(a.id, a.clone());
        mil.divisions.insert(b.id, b.clone());
        mil.definitions.insert(
            DivisionDefinitionId(0),
            DivisionDefinition {
                id: DivisionDefinitionId(0),
                name: "Test".to_string(),
                division_type: DivisionType::Infantry,
                size: DivisionSize::Standard,
                required_manpower: 10_000,
                required_equipment: 0.0,
                recruitment_days: 30,
                movement_speed: 4.0,
                attack: 100.0,
                defense: 100.0,
                breakthrough: 5.0,
                organization: 100.0,
                morale: 100.0,
                supply_usage: 1.0,
                maintenance_cost: 1.0,
            },
        );

        // 新しいfixed-point APIを一切経由せず、panicもエラーも起こさず完了する。
        process_combat(&mut mil, &war_reg);

        let a_after = mil.divisions.get(&a.id).unwrap();
        let b_after = mil.divisions.get(&b.id).unwrap();
        // 旧来の生のf64減算がそのまま動いていること(=新APIの値域検証を経由せず、
        // 正規値域外の値でも普通にmutateされること)を確認する。
        assert_eq!(a_after.manpower, 9_990);
        assert_eq!(b_after.manpower, 9_990);
        assert_eq!(a_after.equipment, out_of_range_equipment - 1.0);
        assert_eq!(b_after.equipment, out_of_range_equipment - 1.0);
        assert_eq!(a_after.organization, 95.0);
        assert_eq!(b_after.organization, 95.0);
    }

    // ─── P25-006-REV-001: 撤退時追加manpower損失への装備連動 ────────────────
    // `resolve_participant_outcome`は本モジュールのprivate関数のため、ここでも
    // P25-004と同じく直接呼び出して数式的正確性・原子性・二重計上防止を検証する。

    fn retreat_ready_division(
        id: usize,
        owner: CountryId,
        state: StateId,
        manpower: u64,
        max_manpower: u64,
        equipment: f64,
    ) -> Division {
        let mut d = make_division_ex(id, owner, state, manpower, max_manpower, equipment, 999.0, 10, 10);
        d.organization = 0.0; // 撤退分岐(組織率<=0)へ入る
        d
    }

    /// Test(§16.1相当): 撤退部分損失。M=100(max_manpower=100→retreat_loss=10)、E=40
    /// → D=4, M_after=90, E_after=36, Divisionは生存。
    #[test]
    fn retreat_test_16_1_partial_loss_matches_the_p25_004_formula() {
        let owner = CountryId(1);
        let origin = StateId(1);
        let target = StateId(2);
        let mut mil = MilitaryRegistry::default();
        let division_id = DivisionId(0);
        mil.divisions.insert(
            division_id,
            retreat_ready_division(0, owner, target, 100, 100, 40.0),
        );

        let survived = resolve_participant_outcome(division_id, Some(origin), &mut mil);
        assert!(!survived, "戻り値は常にfalse(戦闘ロスターからは外れる、撃破ではなく撤退)");

        let after = &mil.divisions[&division_id];
        assert_eq!(after.manpower, 90);
        assert_eq!(after.equipment, 36.0);
        assert_eq!(after.current_state, origin);
        assert_eq!(after.status, DivisionStatus::Idle);
    }

    /// Test(§16.2相当): 小数装備3.7の正確な按分。
    #[test]
    fn retreat_test_16_2_fractional_equipment_is_exact_in_fixed_point_units() {
        let owner = CountryId(1);
        let target = StateId(2);
        let mut mil = MilitaryRegistry::default();
        let division_id = DivisionId(0);
        mil.divisions.insert(
            division_id,
            retreat_ready_division(0, owner, target, 100, 100, 3.7),
        );

        resolve_participant_outcome(division_id, Some(StateId(1)), &mut mil);

        let after = &mil.divisions[&division_id];
        assert_eq!(after.equipment, 3.33);
    }

    /// Test(§16.3相当): half-up境界。manpower=2, max_manpower=10(retreat_loss=1)、
    /// E_units=1 → 1×1/2=0.5 → 切り上げてD_units=1。
    #[test]
    fn retreat_test_16_3_half_up_boundary_rounds_up() {
        let owner = CountryId(1);
        let target = StateId(2);
        let mut mil = MilitaryRegistry::default();
        let division_id = DivisionId(0);
        let equipment_one_unit = military_equipment_units_to_value(1);
        mil.divisions.insert(
            division_id,
            retreat_ready_division(0, owner, target, 2, 10, equipment_one_unit),
        );

        resolve_participant_outcome(division_id, Some(StateId(1)), &mut mil);

        let after = &mil.divisions[&division_id];
        assert_eq!(after.manpower, 1, "retreat_loss=1のみ減少するはず");
        assert_eq!(after.equipment, 0.0, "1 unitの装備がhalf-upで全損するはず");
    }

    /// Test(§16.4相当): 撤退casualtyそのものでmanpowerが0になる場合、丸めに
    /// 関わらず必ず現在装備が全損する(P25-004のD=E恒等式と同じ)。
    #[test]
    fn retreat_test_16_4_full_annihilation_by_retreat_casualty_destroys_all_equipment() {
        let owner = CountryId(1);
        let target = StateId(2);
        let mut mil = MilitaryRegistry::default();
        let division_id = DivisionId(0);
        // max_manpower=100→retreat_loss=10 > manpower(1) → remaining=0(saturating_sub)。
        mil.divisions.insert(
            division_id,
            retreat_ready_division(0, owner, target, 1, 100, 3.7),
        );

        resolve_participant_outcome(division_id, Some(StateId(1)), &mut mil);

        assert!(
            !mil.divisions.contains_key(&division_id),
            "manpower=0になったDivisionは削除されるはず"
        );
    }

    /// Test(§16.5相当、本実装の設計判断): 撤退追加casualty自体は一部生存させる
    /// 量でも、撤退先が存在しない(包囲)ため強制解散される場合は、残存装備を
    /// 含め現在装備を全損させる(比例分だけでなく全量)。
    #[test]
    fn retreat_test_16_5_forced_removal_without_a_destination_destroys_all_remaining_equipment() {
        let owner = CountryId(1);
        let target = StateId(2);
        let mut mil = MilitaryRegistry::default();
        let division_id = DivisionId(0);
        mil.divisions.insert(
            division_id,
            retreat_ready_division(0, owner, target, 100, 100, 40.0),
        );

        // 撤退先なし(包囲)。retreat_loss(10)自体はmanpowerを0にはしないが、
        // 撤退先が無いため強制解散される。
        resolve_participant_outcome(division_id, None, &mut mil);

        assert!(
            !mil.divisions.contains_key(&division_id),
            "撤退先が無いため強制解散されるはず"
        );
    }

    /// Test: casualty=0(retreat_lossがmax_manpowerに対して0になる)なら装備は不変。
    #[test]
    fn retreat_test_casualty_zero_leaves_equipment_untouched() {
        let owner = CountryId(1);
        let target = StateId(2);
        let mut mil = MilitaryRegistry::default();
        let division_id = DivisionId(0);
        // max_manpower=5 → retreat_loss=(5*0.1) as u64 = 0。
        mil.divisions.insert(
            division_id,
            retreat_ready_division(0, owner, target, 5, 5, 40.0),
        );

        resolve_participant_outcome(division_id, Some(StateId(1)), &mut mil);

        let after = &mil.divisions[&division_id];
        assert_eq!(after.manpower, 5);
        assert_eq!(after.equipment, 40.0);
    }

    /// Test: equipment=0でも撤退manpower処理は既存どおり適用される。
    #[test]
    fn retreat_test_zero_equipment_does_not_block_manpower_processing() {
        let owner = CountryId(1);
        let target = StateId(2);
        let mut mil = MilitaryRegistry::default();
        let division_id = DivisionId(0);
        mil.divisions.insert(
            division_id,
            retreat_ready_division(0, owner, target, 100, 100, 0.0),
        );

        resolve_participant_outcome(division_id, Some(StateId(1)), &mut mil);

        let after = &mil.divisions[&division_id];
        assert_eq!(after.manpower, 90, "装備0でもmanpower撤退損失は既存どおり適用されるはず");
        assert_eq!(after.equipment, 0.0);
    }

    /// Test: 撤退(生存)時、装備会計が失敗した場合はmanpower・状態遷移とも
    /// 一切適用されない(原子性)。
    #[test]
    fn retreat_test_atomicity_survives_branch_rolls_back_manpower_on_equipment_failure() {
        let owner = CountryId(1);
        let target = StateId(2);
        let mut mil = MilitaryRegistry::default();
        let division_id = DivisionId(0);
        let mut division = retreat_ready_division(0, owner, target, 100, 100, 40.0);
        division.equipment = f64::NAN;
        mil.divisions.insert(division_id, division);

        let result = resolve_participant_outcome(division_id, Some(StateId(1)), &mut mil);
        assert!(result, "会計失敗時はtrue(現状維持、まだ戦闘継続扱い)を返すはず");

        let after = &mil.divisions[&division_id];
        assert_eq!(after.manpower, 100, "装備会計が失敗した場合、manpowerも変化してはいけない");
        assert!(after.equipment.is_nan());
        assert_eq!(
            after.status,
            DivisionStatus::Fighting,
            "make_division_exの初期状態(Fighting)のまま(status変更もロールバック)"
        );
    }

    /// Test: 強制削除分岐でも、装備会計が失敗した場合はDivisionを削除しない。
    #[test]
    fn retreat_test_atomicity_forced_removal_branch_does_not_remove_on_equipment_failure() {
        let owner = CountryId(1);
        let target = StateId(2);
        let mut mil = MilitaryRegistry::default();
        let division_id = DivisionId(0);
        let mut division = retreat_ready_division(0, owner, target, 100, 100, 40.0);
        division.equipment = f64::NAN;
        mil.divisions.insert(division_id, division);

        let result = resolve_participant_outcome(division_id, None, &mut mil);
        assert!(result);

        assert!(
            mil.divisions.contains_key(&division_id),
            "装備破壊に失敗した場合、Divisionを削除してはいけない"
        );
        assert!(mil.divisions[&division_id].equipment.is_nan());
    }

    /// Test: manpower=0の防御的分岐(通常は既にequipment=0のはずだが、念のため
    /// 破損データを想定して確認する)でも、削除前に装備が全損する。
    #[test]
    fn retreat_test_manpower_zero_defensive_branch_destroys_stale_nonzero_equipment() {
        let owner = CountryId(1);
        let target = StateId(2);
        let mut mil = MilitaryRegistry::default();
        let division_id = DivisionId(0);
        // 現在の正常経路では発生しないはずの状態(manpower=0なのにequipment>0)を
        // 意図的に構築する。
        let mut division = make_division_ex(0, owner, target, 0, 100, 40.0, 100.0, 10, 10);
        division.organization = 50.0; // manpower==0チェックはorganizationより先に評価される
        mil.divisions.insert(division_id, division);

        let survived = resolve_participant_outcome(division_id, Some(StateId(1)), &mut mil);
        assert!(!survived);
        assert!(!mil.divisions.contains_key(&division_id));
    }

    /// Test: 通常戦闘損失と撤退追加損失が同日に発生しても、それぞれ1回だけ
    /// 適用され、二重計上されない(process_daily_battlesとresolve_finished_battlesを
    /// 分けて呼び、中間状態を自己検証する)。
    #[test]
    fn retreat_test_no_double_counting_with_same_day_normal_combat_loss() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let mut state_reg = StateRegistry::build(vec![]);

        // 攻撃側は最初からorganization=0(既に前日消耗済みの想定)。防御側からの
        // 通常戦闘casualtyは受けるが、org=0のため自身は防御側へダメージを与えない。
        // このテストは撤退損失の逐次適用(二重計上防止)を検証するものであり、
        // P25-007の戦闘充足率の影響を受けないよう、equipment=max_equipment
        // (readiness 100%)にする(999.0のような`max_equipment`にすると、
        // P25-007の`min(m,e)`によって意図せず戦力が減少してしまう)。
        let mut attacker = make_division_ex(
            0,
            attacker_country,
            target,
            10_000,
            10_000,
            100.0,
            100.0,
            10,
            10,
        );
        attacker.organization = 0.0;
        let defender = make_division_ex(
            1,
            defender_country,
            target,
            10_000,
            10_000,
            100.0,
            100.0,
            10,
            15,
        );

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(
                attacker_country,
                defender_country,
                target,
                vec![attacker.id],
                vec![defender.id],
            ))
            .unwrap();

        // 1. 通常戦闘のみ適用(まだ撤退は処理していない)。
        let zero_damage_battles_today =
            process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);
        let after_combat = mil.divisions.get(&attacker.id).cloned();
        let Some(after_combat) = after_combat else {
            panic!("攻撃側は通常戦闘だけでは全滅しないはずの設定");
        };
        assert!(
            after_combat.manpower < attacker.manpower,
            "org=0でも防御側からの通常戦闘casualtyは受けるはず"
        );
        let combat_casualty = attacker.manpower - after_combat.manpower;
        let expected_combat_loss = calculate_proportional_equipment_loss(
            attacker.equipment,
            attacker.manpower,
            combat_casualty,
        )
        .unwrap();
        assert_eq!(
            after_combat.equipment,
            military_equipment_units_to_value(expected_combat_loss.equipment_units_after),
            "通常戦闘段階の装備損失はP25-004の式と一致するはず"
        );

        // 2. 撤退追加損失を適用(通常戦闘後のmanpower/equipmentを基準にするはず)。
        resolve_finished_battles(
            &mut mil,
            &mut state_reg,
            &mut battle_reg,
            &war_reg,
            &zero_damage_battles_today,
        );
        let after_retreat = mil.divisions.get(&attacker.id);

        if let Some(after_retreat) = after_retreat {
            let retreat_casualty = after_combat.manpower - after_retreat.manpower;
            let expected_retreat_loss = calculate_proportional_equipment_loss(
                after_combat.equipment,
                after_combat.manpower,
                retreat_casualty,
            )
            .unwrap();
            assert_eq!(
                after_retreat.equipment,
                military_equipment_units_to_value(expected_retreat_loss.equipment_units_after),
                "撤退段階の装備損失は、通常戦闘後の値を基準にした比例損失と一致するはず \
                 (元のM/Eから再計算していない=二重計上していない)"
            );
        } else {
            // 撤退追加casualtyで全滅した場合も、装備は既にprocess_daily_battles段階の
            // 値からさらに全損しているはずであり、二重計上はしていない
            // (このassert自体は「削除された」という事実のみを確認する)。
        }
    }

    /// Test: 装備量の違いが撤退のmanpower/organization/勝敗結果へ一切影響しない
    /// (P25-004のTest Sと同じ原則、撤退経路版)。
    #[test]
    fn retreat_test_equipment_amount_does_not_affect_manpower_or_status_outcome() {
        fn run_scenario(equipment: f64) -> (u64, DivisionStatus, Option<StateId>) {
            let owner = CountryId(1);
            let target = StateId(2);
            let mut mil = MilitaryRegistry::default();
            let division_id = DivisionId(0);
            mil.divisions.insert(
                division_id,
                retreat_ready_division(0, owner, target, 100, 100, equipment),
            );
            resolve_participant_outcome(division_id, Some(StateId(1)), &mut mil);
            match mil.divisions.get(&division_id) {
                Some(d) => (d.manpower, d.status, Some(d.current_state)),
                None => (0, DivisionStatus::Destroyed, None),
            }
        }

        let with_equipment = run_scenario(100.0);
        let without_equipment = run_scenario(0.0);
        assert_eq!(with_equipment, without_equipment);
    }

    // ─── P25-007: process_daily_battlesの戦闘前preflight(§15) ─────────────

    /// Test(§15): 参加Divisionの1件でも戦闘充足率が計算不能(不正状態)なら、
    /// そのBattleの当日解決を丸ごとskipし、manpower・equipment・organization・
    /// elapsed_daysのいずれも変更しない(原子性)。
    #[test]
    fn test_preflight_skips_battle_resolution_atomically_when_a_participant_has_invalid_equipment() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let state_reg = StateRegistry::build(vec![]);

        let attacker = make_division(0, attacker_country, target, 10, 10);
        let mut defender = make_division(1, defender_country, target, 10, 15);
        defender.equipment = f64::NAN; // 現在の正常経路では発生しないはずの破損状態

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(
                attacker_country,
                defender_country,
                target,
                vec![attacker.id],
                vec![defender.id],
            ))
            .unwrap();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        let attacker_after = &mil.divisions[&attacker.id];
        let defender_after = &mil.divisions[&defender.id];
        assert_eq!(attacker_after.manpower, attacker.manpower, "攻撃側もmanpowerを変更してはいけない");
        assert_eq!(attacker_after.equipment, attacker.equipment);
        assert_eq!(attacker_after.organization, attacker.organization);
        assert_eq!(defender_after.manpower, defender.manpower);
        assert!(defender_after.equipment.is_nan(), "不正な装備状態はそのまま残る(黙って0クランプされない)");
        assert_eq!(defender_after.organization, defender.organization);
        assert_eq!(
            battle_reg.battles[&BattleId(0)].elapsed_days, 0,
            "battle statistics(elapsed_days)も更新してはいけない"
        );
        assert_eq!(battle_reg.battles[&BattleId(0)].status, BattleStatus::Ongoing);
    }

    /// Test(§15): manpower > max_manpowerという不正状態(黙ってclampされるべき
    /// ではない)でも、同様にそのBattleの当日解決全体をskipする。
    #[test]
    fn test_preflight_skips_battle_resolution_when_a_participant_has_manpower_exceeding_max() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let state_reg = StateRegistry::build(vec![]);

        let mut attacker = make_division(0, attacker_country, target, 10, 10);
        attacker.manpower = attacker.max_manpower + 1; // 不正状態
        let defender = make_division(1, defender_country, target, 10, 15);

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(
                attacker_country,
                defender_country,
                target,
                vec![attacker.id],
                vec![defender.id],
            ))
            .unwrap();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        assert_eq!(mil.divisions[&attacker.id].manpower, attacker.manpower);
        assert_eq!(mil.divisions[&defender.id].manpower, defender.manpower);
    }

    /// Test: 1つのBattleが不正状態でskipされても、他のBattle(別の国家ペア)は
    /// 通常どおり解決される(他Battle・他国家まで停止させない)。
    #[test]
    fn test_preflight_failure_in_one_battle_does_not_block_another_battle() {
        let country_a = CountryId(1);
        let country_b = CountryId(2);
        let country_c = CountryId(3);
        let country_d = CountryId(4);
        let mut war_reg = setup_war(country_a, country_b);
        // country_c/dの戦争も同じWarRegistryへ追加する。
        war_reg.wars.insert(
            WarId(1),
            War {
                id: WarId(1),
                name: "Second War".to_string(),
                attackers: [country_c].into_iter().collect(),
                defenders: [country_d].into_iter().collect(),
                primary_attacker: None,
                primary_defender: None,
                source_crisis_id: None,
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
            },
        );

        let target_ab = StateId(2);
        let target_cd = StateId(3);
        let state_reg = StateRegistry::build(vec![]);

        let attacker_a = make_division(0, country_a, target_ab, 10, 10);
        let mut defender_b = make_division(1, country_b, target_ab, 10, 15);
        defender_b.equipment = f64::NAN; // Battle 1(a vs b)は不正状態

        let attacker_c = make_division_ex(2, country_c, target_cd, 10_000, 10_000, 100.0, 100.0, 10, 10);
        let defender_d = make_division_ex(3, country_d, target_cd, 10_000, 10_000, 100.0, 100.0, 10, 15);

        let mut mil = MilitaryRegistry::default();
        for d in [&attacker_a, &defender_b, &attacker_c, &defender_d] {
            mil.divisions.insert(d.id, d.clone());
        }

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(country_a, country_b, target_ab, vec![attacker_a.id], vec![defender_b.id]))
            .unwrap();
        battle_reg
            .start_battle(make_battle(country_c, country_d, target_cd, vec![attacker_c.id], vec![defender_d.id]))
            .unwrap();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        // Battle 1(不正状態)は変更されない。
        assert_eq!(mil.divisions[&attacker_a.id].manpower, attacker_a.manpower);
        // Battle 2(正常)は通常どおり解決される。
        assert!(
            mil.divisions[&defender_d.id].manpower < defender_d.manpower,
            "不正状態のBattleとは無関係な別Battleは通常どおり解決されるはず"
        );
    }

    // ─── P26-005: 補給充足率の戦闘力反映(実production経路: process_daily_battles/
    // resolve_finished_battles、`MilitaryRegistry::supply_fulfillment_of`経由) ────

    /// process_daily_battlesのpreflightは、補給充足率が非有限・範囲外の場合も
    /// 装備/manpowerの既存不正状態と同じく、そのBattleの当日解決を丸ごとskipする
    /// (spec §9、P25-007のpreflight方針をP26-005の補給率にも拡張)。
    #[test]
    fn p26_005_preflight_skips_battle_resolution_when_a_participant_has_invalid_supply_fulfillment() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let state_reg = StateRegistry::build(vec![]);

        let attacker = make_division(0, attacker_country, target, 10, 10);
        let defender = make_division(1, defender_country, target, 10, 15);

        let mut mil = MilitaryRegistry::default();
        mil.divisions.insert(attacker.id, attacker.clone());
        mil.divisions.insert(defender.id, defender.clone());
        // 到達不能のはずの不正状態(NaN)を明示的に注入し、preflightが黙って
        // 戦闘計算へ流さないことを確認する(spec §8の防御方針)。
        mil.division_supply_fulfillment.insert(attacker.id, f64::NAN);

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(attacker_country, defender_country, target, vec![attacker.id], vec![defender.id]))
            .unwrap();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        assert_eq!(mil.divisions[&attacker.id].manpower, attacker.manpower, "不正な補給充足率を持つBattleは当日解決をskipするはず");
        assert_eq!(mil.divisions[&defender.id].manpower, defender.manpower);
    }

    /// 同じmanpower/equipment/base strengthの2個師団で、補給充足率だけが異なるとき、
    /// その違いが実production経路(process_daily_battles)で対称的に戦闘力へ反映される
    /// ことを確認する(spec §10)。
    #[test]
    fn p26_005_real_path_attacker_supply_shortage_reduces_damage_dealt_to_defender() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);
        let target = StateId(2);
        let state_reg = StateRegistry::build(vec![]);

        fn run_scenario(atk_supply: f64) -> u64 {
            let attacker_country = CountryId(1);
            let defender_country = CountryId(2);
            let war_reg = setup_war(attacker_country, defender_country);
            let target = StateId(2);
            let state_reg = StateRegistry::build(vec![]);
            let attacker = make_division_ex(0, attacker_country, target, 10_000, 10_000, 100.0, 100.0, 10, 10);
            let defender = make_division_ex(1, defender_country, target, 10_000, 10_000, 100.0, 100.0, 10, 10);
            let mut mil = MilitaryRegistry::default();
            mil.divisions.insert(attacker.id, attacker.clone());
            mil.divisions.insert(defender.id, defender.clone());
            mil.division_supply_fulfillment.insert(attacker.id, atk_supply);
            let mut battle_reg = BattleRegistry::default();
            battle_reg
                .start_battle(make_battle(attacker_country, defender_country, target, vec![attacker.id], vec![defender.id]))
                .unwrap();
            process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);
            defender.manpower - mil.divisions[&defender.id].manpower
        }

        let def_loss_full_supply = run_scenario(1.0);
        let def_loss_half_supply = run_scenario(0.5);
        assert!(
            def_loss_half_supply < def_loss_full_supply,
            "攻撃側の補給不足(50%)は、実production経路でも防御側への被害を減らすはず"
        );
        let _ = (attacker_country, defender_country, war_reg, target, state_reg);
    }

    /// 上と対称: 防御側だけが補給不足の場合も、反撃力(攻撃側が受ける被害)が
    /// 実production経路で低下する(spec §10、陣営による特別扱いをしていないことの確認)。
    #[test]
    fn p26_005_real_path_defender_supply_shortage_reduces_damage_dealt_to_attacker() {
        fn run_scenario(def_supply: f64) -> u64 {
            let attacker_country = CountryId(1);
            let defender_country = CountryId(2);
            let war_reg = setup_war(attacker_country, defender_country);
            let target = StateId(2);
            let state_reg = StateRegistry::build(vec![]);
            let attacker = make_division_ex(0, attacker_country, target, 10_000, 10_000, 100.0, 100.0, 10, 10);
            let defender = make_division_ex(1, defender_country, target, 10_000, 10_000, 100.0, 100.0, 10, 15);
            let mut mil = MilitaryRegistry::default();
            mil.divisions.insert(attacker.id, attacker.clone());
            mil.divisions.insert(defender.id, defender.clone());
            mil.division_supply_fulfillment.insert(defender.id, def_supply);
            let mut battle_reg = BattleRegistry::default();
            battle_reg
                .start_battle(make_battle(attacker_country, defender_country, target, vec![attacker.id], vec![defender.id]))
                .unwrap();
            process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);
            attacker.manpower - mil.divisions[&attacker.id].manpower
        }

        let atk_loss_full_supply = run_scenario(1.0);
        let atk_loss_half_supply = run_scenario(0.5);
        assert!(
            atk_loss_half_supply < atk_loss_full_supply,
            "防御側の補給不足(50%)は反撃力を弱め、攻撃側が受ける被害を実production経路でも減らすはず"
        );
    }

    /// spec §16: 一国の補給不足が別の同時進行Battle(別国家の組)の結果へ波及しない。
    #[test]
    fn p26_005_real_path_one_countrys_supply_shortage_does_not_leak_into_an_unrelated_battle() {
        let country_a = CountryId(1);
        let country_b = CountryId(2);
        let country_c = CountryId(3);
        let country_d = CountryId(4);

        let mut war_reg = WarRegistry::default();
        for (id, attackers, defenders) in [
            (0usize, [country_a], [country_b]),
            (1usize, [country_c], [country_d]),
        ] {
            war_reg.wars.insert(
                WarId(id),
                War {
                    id: WarId(id),
                    name: "Test War".to_string(),
                    attackers: attackers.into_iter().collect(),
                    defenders: defenders.into_iter().collect(),
                    primary_attacker: None,
                    primary_defender: None,
                    source_crisis_id: None,
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
                },
            );
        }

        let target_ab = StateId(2);
        let target_cd = StateId(3);
        let state_reg = StateRegistry::build(vec![]);

        let attacker_a = make_division_ex(0, country_a, target_ab, 10_000, 10_000, 100.0, 100.0, 10, 10);
        let defender_b = make_division_ex(1, country_b, target_ab, 10_000, 10_000, 100.0, 100.0, 10, 10);
        let attacker_c = make_division_ex(2, country_c, target_cd, 10_000, 10_000, 100.0, 100.0, 10, 10);
        let defender_d = make_division_ex(3, country_d, target_cd, 10_000, 10_000, 100.0, 100.0, 10, 10);

        let mut mil = MilitaryRegistry::default();
        for d in [&attacker_a, &defender_b, &attacker_c, &defender_d] {
            mil.divisions.insert(d.id, d.clone());
        }
        // country_aの攻撃側だけが深刻な補給不足。country_c/dは無関係(supply指定なし=1.0)。
        mil.division_supply_fulfillment.insert(attacker_a.id, 0.1);

        let mut battle_reg = BattleRegistry::default();
        battle_reg
            .start_battle(make_battle(country_a, country_b, target_ab, vec![attacker_a.id], vec![defender_b.id]))
            .unwrap();
        battle_reg
            .start_battle(make_battle(country_c, country_d, target_cd, vec![attacker_c.id], vec![defender_d.id]))
            .unwrap();

        process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);

        // Battle(c vs d、両者とも補給充足)は、補給充足率が同一の別途独立計算した
        // 基準シナリオと完全に一致する(country_aの不足が漏れていない)。
        let mut reference_mil = MilitaryRegistry::default();
        reference_mil.divisions.insert(attacker_c.id, attacker_c.clone());
        reference_mil.divisions.insert(defender_d.id, defender_d.clone());
        let mut reference_battle_reg = BattleRegistry::default();
        reference_battle_reg
            .start_battle(make_battle(country_c, country_d, target_cd, vec![attacker_c.id], vec![defender_d.id]))
            .unwrap();
        process_daily_battles(&mut reference_mil, &mut reference_battle_reg, &state_reg, &war_reg);

        assert_eq!(
            mil.divisions[&defender_d.id].manpower,
            reference_mil.divisions[&defender_d.id].manpower,
            "country_aの補給不足はcountry_c/dの独立したBattleへ波及してはいけない"
        );
        assert_eq!(
            mil.divisions[&attacker_c.id].manpower,
            reference_mil.divisions[&attacker_c.id].manpower
        );
    }

    /// spec §13/§21: 供給不足による戦闘力低下が、実際に下流(撃破可否・州支配権)へ
    /// 伝播することを示す。既存の`test_combined_multiple_attackers_can_break_well_defended_state`
    /// と全く同じ3個師団攻撃側の構成だが、補給率を著しく下げると同じ攻撃側でも
    /// 突破できなくなる(=補給不足が実際にゲームプレイの勝敗へ影響する)。
    #[test]
    fn p26_005_severe_supply_shortage_prevents_an_attack_that_would_otherwise_break_the_defender() {
        let attacker_country = CountryId(1);
        let defender_country = CountryId(2);
        let war_reg = setup_war(attacker_country, defender_country);

        let origin = StateId(1);
        let target = StateId(2);
        let mut state_reg = StateRegistry::build(vec![
            StateData { id: origin, owner_country_id: attacker_country, ..Default::default() },
            StateData { id: target, owner_country_id: defender_country, ..Default::default() },
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
        // 3個師団とも深刻な補給不足(10%)。既存テストでは補給を考慮していなかった
        // ため常に1.0扱いで突破できていたが、P26-005導入後は突破できなくなるはず。
        for id in [attacker_a.id, attacker_b.id, attacker_c.id] {
            mil.division_supply_fulfillment.insert(id, 0.1);
        }

        let mut battle_reg = BattleRegistry::default();
        let battle = make_battle(
            attacker_country,
            defender_country,
            target,
            vec![attacker_a.id, attacker_b.id, attacker_c.id],
            vec![defender.id],
        );
        battle_reg.start_battle(battle).unwrap();

        for _ in 0..30 {
            run_one_day(&mut mil, &mut state_reg, &mut battle_reg, &war_reg);
            if battle_reg.battles[&BattleId(0)].status != BattleStatus::Ongoing {
                break;
            }
        }

        let battle = &battle_reg.battles[&BattleId(0)];
        assert_ne!(
            battle.status,
            BattleStatus::AttackerWon,
            "深刻な補給不足(10%)により、同じ30日以内では本来突破できたはずの3個師団攻撃側も\
             突破できなくなるはず(Ongoing継続またはDefenderWonのいずれか、AttackerWonにはならない)"
        );
        assert_eq!(
            state_reg.get(target).unwrap().controller(),
            defender_country,
            "補給不足により突破できなかった以上、州の支配権は変化しないはず"
        );
    }

    /// spec §19: 補給率map挿入順・Division列挙順・国家挿入順を変えても、
    /// 実production経路の結果はbit-for-bit一致する。
    #[test]
    fn p26_005_real_path_result_is_deterministic_regardless_of_supply_map_insertion_order() {
        fn run_scenario(insert_defender_supply_first: bool) -> (u64, u64) {
            let attacker_country = CountryId(1);
            let defender_country = CountryId(2);
            let war_reg = setup_war(attacker_country, defender_country);
            let target = StateId(2);
            let state_reg = StateRegistry::build(vec![]);
            let attacker = make_division_ex(0, attacker_country, target, 10_000, 10_000, 100.0, 100.0, 10, 10);
            let defender = make_division_ex(1, defender_country, target, 10_000, 10_000, 100.0, 100.0, 10, 15);
            let mut mil = MilitaryRegistry::default();
            mil.divisions.insert(attacker.id, attacker.clone());
            mil.divisions.insert(defender.id, defender.clone());
            if insert_defender_supply_first {
                mil.division_supply_fulfillment.insert(defender.id, 0.6);
                mil.division_supply_fulfillment.insert(attacker.id, 0.8);
            } else {
                mil.division_supply_fulfillment.insert(attacker.id, 0.8);
                mil.division_supply_fulfillment.insert(defender.id, 0.6);
            }
            let mut battle_reg = BattleRegistry::default();
            battle_reg
                .start_battle(make_battle(attacker_country, defender_country, target, vec![attacker.id], vec![defender.id]))
                .unwrap();
            process_daily_battles(&mut mil, &mut battle_reg, &state_reg, &war_reg);
            (mil.divisions[&attacker.id].manpower, mil.divisions[&defender.id].manpower)
        }

        assert_eq!(run_scenario(true), run_scenario(false));
    }
}
