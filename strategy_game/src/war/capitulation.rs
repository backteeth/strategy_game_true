use crate::common::CountryId;
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use crate::war::data::{War, WarStatus};

/// Capitulation check result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapitulationResult {
    None,
    DefenderCapitulated,
    AttackerCapitulated,
}

/// 指定国家に戦闘可能な陸軍ユニットが残っているか判定
pub fn has_combat_ready_divisions(
    country_id: CountryId,
    military_registry: &MilitaryRegistry,
) -> bool {
    military_registry.divisions.values().any(|division| {
        division.owner == country_id && division.status != DivisionStatus::Destroyed && division.manpower > 0
    })
}

/// 防御側の降伏条件判定
pub fn check_defender_capitulation(
    war: &War,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
) -> bool {
    let attacker = match war.attackers.iter().next() {
        Some(&c) => c,
        None => return false,
    };
    let defender = match war.defenders.iter().next() {
        Some(&c) => c,
        None => return false,
    };

    // 戦争目標地域を攻撃側が支配していることが前提
    let target_state_id = match war
        .war_goals
        .first()
        .and_then(|g| g.target_states.first().copied())
    {
        Some(id) => id,
        None => return false,
    };

    let target_state = match state_registry.get(target_state_id) {
        Some(s) => s,
        None => return false,
    };

    if target_state.controller() != attacker {
        return false;
    }

    let defender_land: Vec<_> = state_registry
        .get_owned_states(defender)
        .into_iter()
        .filter(|s| !s.is_sea)
        .collect();
    let total_defender_states = defender_land.len();

    if total_defender_states == 0 {
        return true;
    }

    // 条件1: 防御側所有地域の60%以上を攻撃側が支配
    let attacker_occupied_count = defender_land
        .iter()
        .filter(|s| s.controller() == attacker)
        .count();

    let occupied_ratio = attacker_occupied_count as f32 / total_defender_states as f32;
    if occupied_ratio >= 0.60 {
        return true;
    }

    // 条件2: 防御側に戦闘可能な陸軍が残っていない
    if !has_combat_ready_divisions(defender, military_registry) {
        return true;
    }

    false
}

/// 攻撃側の降伏条件判定
pub fn check_attacker_capitulation(
    war: &War,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
) -> bool {
    let attacker = match war.attackers.iter().next() {
        Some(&c) => c,
        None => return false,
    };
    let defender = match war.defenders.iter().next() {
        Some(&c) => c,
        None => return false,
    };

    let attacker_land: Vec<_> = state_registry
        .get_owned_states(attacker)
        .into_iter()
        .filter(|s| !s.is_sea)
        .collect();
    let total_attacker_states = attacker_land.len();

    if total_attacker_states == 0 {
        return true;
    }

    let defender_occupied_count = attacker_land
        .iter()
        .filter(|s| s.controller() == defender)
        .count();

    // 条件1: 攻撃側所有地域の60%以上を防御側が支配
    let occupied_ratio = defender_occupied_count as f32 / total_attacker_states as f32;
    if occupied_ratio >= 0.60 {
        return true;
    }

    // 条件2: 攻撃側に戦闘可能な陸軍がなく、かつ防御側が攻撃側地域を1つ以上支配
    if !has_combat_ready_divisions(attacker, military_registry) && defender_occupied_count >= 1 {
        return true;
    }

    false
}

/// 戦争ごとの総合降伏判定（同時条件満了時の決定論的判定含む）
pub fn evaluate_war_capitulation(
    war: &War,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
) -> CapitulationResult {
    if war.status != WarStatus::Active {
        return CapitulationResult::None;
    }

    let defender_cap = check_defender_capitulation(war, state_registry, military_registry);
    let attacker_cap = check_attacker_capitulation(war, state_registry, military_registry);

    if defender_cap && attacker_cap {
        // 双方が同時に満たした場合の決定論的規則:
        // 戦勝点 > 0 なら防御側敗北
        // 戦勝点 < 0 なら攻撃側敗北
        // 戦勝点 == 0 なら攻撃側敗北
        if war.war_score > 0.0 {
            CapitulationResult::DefenderCapitulated
        } else {
            CapitulationResult::AttackerCapitulated
        }
    } else if defender_cap {
        CapitulationResult::DefenderCapitulated
    } else if attacker_cap {
        CapitulationResult::AttackerCapitulated
    } else {
        CapitulationResult::None
    }
}
