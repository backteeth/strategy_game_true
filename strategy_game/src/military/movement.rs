use crate::military::data::{ArmyStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;

pub fn process_movement(military_registry: &mut MilitaryRegistry, state_registry: &StateRegistry) {
    let base_days = 5.0; // 1セグメントあたりの基本移動日数

    for army in military_registry.armies.values_mut() {
        if army.status != ArmyStatus::Moving {
            continue;
        }

        // target_stateが未設定だが current_path が残っている場合、最初のノードをセット
        if army.target_state.is_none() {
            if !army.current_path.is_empty() {
                let next_state = army.current_path.remove(0);
                army.target_state = Some(next_state);
                army.movement_progress = 0.0;
            } else {
                // 目的地に到達
                army.status = ArmyStatus::Idle;
                army.destination = None;
                army.movement_progress = 0.0;
                continue;
            }
        }

        if let Some(target) = army.target_state {
            // 移動先が自国領（または実効支配領域）でなくなった場合の安全停止
            if let Some(target_st) = state_registry.get(target) {
                let controller = target_st
                    .controller_country
                    .unwrap_or(target_st.owner_country_id);

                if controller != army.owner {
                    army.status = ArmyStatus::Idle;
                    army.target_state = None;
                    army.destination = None;
                    army.current_path.clear();
                    army.movement_progress = 0.0;
                    continue;
                }
            }

            let def_speed = military_registry
                .definitions
                .get(&army.def_id)
                .map(|d| d.movement_speed)
                .unwrap_or(1.0);

            let step_cost = crate::military::pathfinding::calculate_step_cost(
                army.current_state,
                target,
                state_registry,
            ) as f32;

            let supply_mod = army.supply_ratio.max(0.5);
            let movement_days = (base_days * step_cost / (def_speed * supply_mod)).max(1.0);
            let daily_progress = 1.0 / movement_days;

            army.movement_progress += daily_progress;

            if army.movement_progress >= 1.0 {
                // セグメント移動完了
                army.current_state = target;
                army.movement_progress = 0.0;

                if !army.current_path.is_empty() {
                    let next = army.current_path.remove(0);
                    army.target_state = Some(next);
                } else {
                    // 最終目的地に到達
                    army.target_state = None;
                    army.destination = None;
                    army.status = ArmyStatus::Idle;
                }
            }
        }
    }
}
