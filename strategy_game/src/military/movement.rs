use crate::military::battle::BattleRegistry;
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use crate::war::data::WarRegistry;

pub fn process_movement(
    military_registry: &mut MilitaryRegistry,
    state_registry: &mut StateRegistry,
    war_registry: &WarRegistry,
    battle_registry: &mut BattleRegistry,
    current_date: &str,
) {
    let base_days = 5.0; // 1セグメントあたりの基本移動日数

    // 処理するユニットのIDリストを事前収集（借用衝突を避けるため）
    let division_ids: Vec<_> = military_registry.divisions.keys().copied().collect();

    for division_id in division_ids {
        // 戦闘中ユニットは移動しない
        {
            let division = match military_registry.divisions.get(&division_id) {
                Some(a) => a,
                None => continue,
            };
            if division.status == DivisionStatus::Fighting || division.status == DivisionStatus::Retreating {
                continue;
            }
            if division.status != DivisionStatus::Moving {
                continue;
            }
        }

        // target_stateが未設定だが current_path が残っている場合、最初のノードをセット
        {
            let division = match military_registry.divisions.get_mut(&division_id) {
                Some(a) => a,
                None => continue,
            };

            if division.target_state.is_none() {
                if !division.current_path.is_empty() {
                    let next_state = division.current_path.remove(0);
                    division.target_state = Some(next_state);
                    division.movement_progress = 0.0;
                } else {
                    // 目的地に到達
                    division.status = DivisionStatus::Idle;
                    division.destination = None;
                    division.movement_progress = 0.0;
                    continue;
                }
            }
        }

        let (target, current_state, owner) = {
            let division = match military_registry.divisions.get(&division_id) {
                Some(a) => a,
                None => continue,
            };
            match division.target_state {
                Some(t) => (t, division.current_state, division.owner),
                None => continue,
            }
        };

        // 移動先の支配国を確認
        let target_controller = match state_registry.get(target) {
            Some(s) => s.controller(),
            None => {
                // 無効な地域 → 移動停止
                if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                    division.status = DivisionStatus::Idle;
                    division.target_state = None;
                    division.destination = None;
                    division.current_path.clear();
                    division.movement_progress = 0.0;
                }
                continue;
            }
        };

        // 自国領かどうかを判定
        let is_own_territory = target_controller == owner;
        let is_at_war_with_controller = war_registry.are_countries_at_war(owner, target_controller);

        // 通過可能か判定（自国 or 交戦中の敵国）
        if !is_own_territory && !is_at_war_with_controller {
            // 通過不可（中立国・非交戦国）→ 移動停止
            if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                division.status = DivisionStatus::Idle;
                division.target_state = None;
                division.destination = None;
                division.current_path.clear();
                division.movement_progress = 0.0;
            }
            continue;
        }

        // 移動速度計算
        let (def_speed, movement_progress) = {
            let division = match military_registry.divisions.get(&division_id) {
                Some(a) => a,
                None => continue,
            };
            let speed = military_registry
                .definitions
                .get(&division.def_id)
                .map(|d| d.movement_speed)
                .unwrap_or(1.0);
            (speed, division.movement_progress)
        };

        let step_cost = crate::military::pathfinding::calculate_step_cost(
            current_state,
            target,
            state_registry,
        ) as f32;
        let supply_mod = {
            let division = military_registry
                .divisions
                .get(&division_id)
                .map(|a| a.supply_ratio)
                .unwrap_or(0.5);
            division.max(0.5)
        };
        let movement_days = (base_days * step_cost / (def_speed * supply_mod)).max(1.0);
        let daily_progress = 1.0 / movement_days;

        let new_progress = movement_progress + daily_progress;

        if new_progress >= 1.0 {
            // セグメント移動完了 → 到着処理
            // まず現在地を更新
            {
                let division = match military_registry.divisions.get_mut(&division_id) {
                    Some(a) => a,
                    None => continue,
                };
                division.current_state = target;
                division.movement_progress = 0.0;
            }

            // 次のセグメントをセット（または到達完了）
            {
                let division = match military_registry.divisions.get_mut(&division_id) {
                    Some(a) => a,
                    None => continue,
                };
                if !division.current_path.is_empty() {
                    let next = division.current_path.remove(0);
                    division.target_state = Some(next);
                } else {
                    division.target_state = None;
                    division.destination = None;
                    // ステータスはinvasionで決定するため、ここでIdleにしない
                }
            }

            // 侵攻判定（敵領か自国領か）
            if is_own_territory {
                // 自国領への移動完了 → Idle（目的地到着時のみ）
                let arrived_at_destination = {
                    let division = military_registry.divisions.get(&division_id);
                    division.map(|a| a.target_state.is_none() && a.destination.is_none())
                        .unwrap_or(false)
                };
                if arrived_at_destination
                    && let Some(division) = military_registry.divisions.get_mut(&division_id)
                {
                    division.status = DivisionStatus::Idle;
                }
            } else {
                // 敵支配地域への到着 → 侵攻処理
                crate::military::invasion::process_division_arrival(
                    division_id,
                    target,
                    current_state,
                    current_date,
                    military_registry,
                    state_registry,
                    war_registry,
                    battle_registry,
                );
            }
        } else {
            // まだ移動中
            if let Some(division) = military_registry.divisions.get_mut(&division_id) {
                division.movement_progress = new_progress;
            }
        }
    }
}

/// 経路の各段階で通行可能かを再検証し、問題があれば移動を停止する
/// 戦争終了や支配国変更で無効になった経路を安全に処理する
pub fn validate_and_stop_invalid_movements(
    military_registry: &mut MilitaryRegistry,
    state_registry: &StateRegistry,
    war_registry: &WarRegistry,
) {
    for division in military_registry.divisions.values_mut() {
        if division.status != DivisionStatus::Moving {
            continue;
        }

        // 現在の target_state が通過可能か確認
        if let Some(target) = division.target_state
            && let Some(target_state) = state_registry.get(target)
        {
            let controller = target_state.controller();
            let is_own = controller == division.owner;
            let is_enemy = war_registry.are_countries_at_war(division.owner, controller);

            if !is_own && !is_enemy {
                // 無効になった → 停止
                division.status = DivisionStatus::Idle;
                division.target_state = None;
                division.destination = None;
                division.current_path.clear();
                division.movement_progress = 0.0;
            }
        }

        // 残りの経路を検証
        if division.status == DivisionStatus::Moving {
            let path_valid = division.current_path.iter().all(|&sid| {
                if let Some(s) = state_registry.get(sid) {
                    let controller = s.controller();
                    controller == division.owner
                        || war_registry.are_countries_at_war(division.owner, controller)
                } else {
                    false
                }
            });

            if !path_valid {
                // 経路の一部が無効 → 安全停止（現在地で待機）
                division.current_path.clear();
                division.destination = None;
                // target_stateへの移動は継続（既に進行中の場合）
                // 次の到着時に再判定される
            }
        }
    }
}
