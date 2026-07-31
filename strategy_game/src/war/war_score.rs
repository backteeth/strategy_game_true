use crate::state::data::StateRegistry;
use crate::war::data::{War, WarRegistry, WarStatus};

/// 戦勝点の内訳（攻撃側視点）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WarScoreBreakdown {
    pub war_goal_score: i32,
    pub attacker_occupation_score: i32,
    pub defender_occupation_score: i32,
    pub battle_score: i32,
    pub total_score: i32,
}

/// 攻撃側視点の戦勝点を計算する
#[allow(clippy::manual_checked_ops)]
pub fn calculate_war_score(war: &War, state_registry: &StateRegistry) -> WarScoreBreakdown {
    let attacker = match war.attackers.iter().next() {
        Some(&c) => c,
        None => return WarScoreBreakdown::default(),
    };
    let defender = match war.defenders.iter().next() {
        Some(&c) => c,
        None => return WarScoreBreakdown::default(),
    };

    // 1. 戦争目標点 (+40 / 0)
    let target_state_id = war
        .war_goals
        .first()
        .and_then(|g| g.target_states.first().copied());

    let war_goal_score = if let Some(target_id) = target_state_id {
        if let Some(target_state) = state_registry.get(target_id) {
            if target_state.controller() == attacker {
                40
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0
    };

    // 2. 攻撃側占領点（陸上地域のみ対象）
    let defender_land_states: Vec<_> = state_registry
        .get_owned_states(defender)
        .into_iter()
        .filter(|s| !s.is_sea)
        .collect();
    let defender_total_states = defender_land_states.len();

    let attacker_occupied_count = defender_land_states
        .iter()
        .filter(|s| s.controller() == attacker)
        .count();

    let attacker_occupation_score = if defender_total_states > 0 {
        (40 * attacker_occupied_count / defender_total_states) as i32
    } else {
        0
    };

    // 3. 防御側占領点（陸上地域のみ対象）
    let attacker_land_states: Vec<_> = state_registry
        .get_owned_states(attacker)
        .into_iter()
        .filter(|s| !s.is_sea)
        .collect();
    let attacker_total_states = attacker_land_states.len();

    let defender_occupied_count = attacker_land_states
        .iter()
        .filter(|s| s.controller() == defender)
        .count();

    let defender_occupation_score = if attacker_total_states > 0 {
        -((40 * defender_occupied_count / attacker_total_states) as i32)
    } else {
        0
    };

    // 4. 戦闘点: 5 * (攻撃側勝利数 - 防御側勝利数), -20 ~ 20 に制限
    let raw_battle_score = 5 * (war.won_attacker_battles as i32 - war.won_defender_battles as i32);
    let battle_score = raw_battle_score.clamp(-20, 20);

    // 5. 総戦勝点 (-100 ~ 100)
    let raw_total =
        war_goal_score + attacker_occupation_score + defender_occupation_score + battle_score;
    let total_score = raw_total.clamp(-100, 100);

    WarScoreBreakdown {
        war_goal_score,
        attacker_occupation_score,
        defender_occupation_score,
        battle_score,
        total_score,
    }
}

pub fn process_war_score(state_registry: &StateRegistry, war_registry: &mut WarRegistry) {
    for war in war_registry.wars.values_mut() {
        if war.status != WarStatus::Active {
            continue;
        }
        let breakdown = calculate_war_score(war, state_registry);
        war.war_score = breakdown.total_score as f32;
    }
}
