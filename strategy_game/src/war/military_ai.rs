use crate::app::time::DayChangedMessage;
use crate::common::{ArmyId, CountryId, FrontlineId, StateId};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::military::data::{ArmyStatus, ArmyUnit, MilitaryRegistry};
use crate::military::pathfinding::find_path;
use crate::state::data::StateRegistry;
use crate::war::data::{War, WarRegistry, WarStatus};
use crate::war::frontline::{
    Frontline, FrontlinePlan, FrontlineRegistry, FrontlineStance, determine_offensive_objective,
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 軍事AIの判断理由
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MilitaryAiDecisionReason {
    #[default]
    NoActiveWar,
    NoValidFrontline,
    NoAvailableArmy,
    NoReachableFrontline,
    NoValidTarget,
    Recovering,
    EnemyStronger,
    ComparableStrength,
    SufficientAdvantage,
    TargetCaptured,
}

impl MilitaryAiDecisionReason {
    pub fn display_name(self) -> &'static str {
        match self {
            MilitaryAiDecisionReason::NoActiveWar => "No Active War",
            MilitaryAiDecisionReason::NoValidFrontline => "No Valid Frontline",
            MilitaryAiDecisionReason::NoAvailableArmy => "No Available Army",
            MilitaryAiDecisionReason::NoReachableFrontline => "No Reachable Frontline",
            MilitaryAiDecisionReason::NoValidTarget => "No Valid Target",
            MilitaryAiDecisionReason::Recovering => "Recovering",
            MilitaryAiDecisionReason::EnemyStronger => "Enemy Stronger (Defend)",
            MilitaryAiDecisionReason::ComparableStrength => "Comparable Strength (Defend)",
            MilitaryAiDecisionReason::SufficientAdvantage => "Sufficient Advantage (Offensive)",
            MilitaryAiDecisionReason::TargetCaptured => "Target Captured (Re-evaluating)",
        }
    }
}

/// AI国家ごとの作戦状態
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilitaryAiState {
    pub country_id: CountryId,
    pub last_evaluated_day: u32,
    pub last_decision_reason: MilitaryAiDecisionReason,
    pub estimated_own_power: u64,
    pub estimated_enemy_power: u64,
    pub dirty: bool,
}

impl MilitaryAiState {
    pub fn new(country_id: CountryId) -> Self {
        Self {
            country_id,
            last_evaluated_day: 0,
            last_decision_reason: MilitaryAiDecisionReason::NoActiveWar,
            estimated_own_power: 0,
            estimated_enemy_power: 0,
            dirty: true,
        }
    }
}

/// 軍事AIを集中管理するリソース
#[derive(Resource, Default, Debug, Clone, Serialize, Deserialize)]
pub struct MilitaryAiRegistry {
    pub ai_states: HashMap<CountryId, MilitaryAiState>,
    pub dirty: bool,
}

impl MilitaryAiRegistry {
    /// 指定国家のAI状態を取得または作成
    pub fn get_or_create_mut(&mut self, country_id: CountryId) -> &mut MilitaryAiState {
        self.ai_states
            .entry(country_id)
            .or_insert_with(|| MilitaryAiState::new(country_id))
    }

    /// 全AI状態を dirty にする
    pub fn mark_all_dirty(&mut self) {
        for state in self.ai_states.values_mut() {
            state.dirty = true;
        }
        self.dirty = true;
    }

    /// 特定国家のAI状態を dirty にする
    pub fn mark_country_dirty(&mut self, country_id: CountryId) {
        if let Some(state) = self.ai_states.get_mut(&country_id) {
            state.dirty = true;
        }
        self.dirty = true;
    }
}

/// 単一陸軍の有効戦力計算 (manpower * org / max_org)
pub fn evaluate_army_power(army: &ArmyUnit) -> u64 {
    if army.manpower == 0 || army.status == ArmyStatus::Destroyed {
        return 0;
    }
    if army.max_organization <= 0.0 {
        return army.manpower;
    }
    let org_ratio = (army.organization / army.max_organization).clamp(0.0, 1.0);
    ((army.manpower as f64) * (org_ratio as f64)) as u64
}

/// 部隊がAI割当対象として使用可能か検証する
pub fn is_army_available_for_ai(
    army: &ArmyUnit,
    commanding_country: CountryId,
    frontline_registry: &FrontlineRegistry,
) -> bool {
    if army.owner != commanding_country {
        return false;
    }
    if army.manpower == 0 || army.status == ArmyStatus::Destroyed {
        return false;
    }
    // 戦闘中・撤退中の部隊は新規割当対象にしない（割当済み前線への所属維持は可）
    if army.status == ArmyStatus::Fighting || army.status == ArmyStatus::Retreating {
        return false;
    }
    // 既に別の前線に所属している場合は対象外
    if frontline_registry.army_frontline_map.contains_key(&army.id) {
        return false;
    }
    true
}

/// 攻勢目標候補の決定的な選択（首都・グラフ距離考慮）
pub fn select_ai_offensive_objective(
    war: &War,
    commanding_country: CountryId,
    state_registry: &StateRegistry,
    country_registry: &CountryRegistry,
    frontline: &Frontline,
) -> Option<StateId> {
    // まず Phase 17 の標準目標選択（戦争目標、奪回領土）を試す
    if let Some(primary_obj) =
        determine_offensive_objective(war, commanding_country, state_registry)
        && let Some(s) = state_registry.get(primary_obj)
        && !s.is_sea
        && s.controller() != commanding_country
    {
        return Some(primary_obj);
    }

    let enemy_id = if commanding_country == frontline.attacker_country_id {
        frontline.defender_country_id
    } else {
        frontline.attacker_country_id
    };

    let front_regions = if commanding_country == frontline.attacker_country_id {
        &frontline.attacker_front_regions
    } else {
        &frontline.defender_front_regions
    };

    if front_regions.is_empty() {
        return None;
    }

    // 候補1: 敵国の首都（到達可能かつ未支配の場合）
    if let Some(enemy_country) = country_registry.get(enemy_id) {
        let cap_id = enemy_country.capital_state_id;
        if let Some(cap_state) = state_registry.get(cap_id)
            && !cap_state.is_sea
            && cap_state.controller() == enemy_id
        {
            let reachable = front_regions.iter().any(|&fr| {
                find_path(
                    fr,
                    cap_id,
                    state_registry,
                    &[commanding_country],
                    &[enemy_id],
                )
                .is_some()
            });
            if reachable {
                return Some(cap_id);
            }
        }
    }

    // 候補2: 前線からの最短グラフ距離にある敵国支配地域（StateId昇順で確定）
    let mut enemy_states: Vec<StateId> = state_registry
        .states
        .iter()
        .filter(|s| !s.is_sea && s.controller() == enemy_id)
        .map(|s| s.id)
        .collect();
    enemy_states.sort_by_key(|s| s.0);

    let mut candidates_with_dist: Vec<(StateId, usize)> = Vec::new();

    for &enemy_state_id in &enemy_states {
        let min_dist = front_regions
            .iter()
            .filter_map(|&fr| {
                find_path(
                    fr,
                    enemy_state_id,
                    state_registry,
                    &[commanding_country],
                    &[enemy_id],
                )
                .map(|p| p.len())
            })
            .min();

        if let Some(dist) = min_dist {
            candidates_with_dist.push((enemy_state_id, dist));
        }
    }

    candidates_with_dist.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.0.cmp(&b.0.0)));

    candidates_with_dist.first().map(|(sid, _)| *sid)
}

/// 自軍割当戦力と敵軍保有戦力の集計・評価
pub fn evaluate_powers(
    frontline: &Frontline,
    plan: &FrontlinePlan,
    commanding_country: CountryId,
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
) -> (u64, u64) {
    let enemy_id = if commanding_country == frontline.attacker_country_id {
        frontline.defender_country_id
    } else {
        frontline.attacker_country_id
    };

    // 自軍割り当て部隊の有効戦力合計
    let own_power: u64 = plan
        .assigned_army_ids
        .iter()
        .filter_map(|&id| military_registry.armies.get(&id))
        .filter(|a| a.owner == commanding_country && a.status != ArmyStatus::Destroyed)
        .map(evaluate_army_power)
        .sum();

    // 敵国の全戦闘可能部隊の有効戦力合計
    let enemy_power: u64 = military_registry
        .armies
        .values()
        .filter(|a| {
            a.owner == enemy_id
                && a.manpower > 0
                && a.status != ArmyStatus::Destroyed
                && state_registry
                    .get(a.current_state)
                    .map(|s| !s.is_sea)
                    .unwrap_or(false)
        })
        .map(evaluate_army_power)
        .sum();

    (own_power, enemy_power)
}

/// 前線に対するAI命令スタンスと理由の決定（ヒステリシス考慮）
pub fn evaluate_military_ai_decision(
    frontline: &Frontline,
    plan: &FrontlinePlan,
    own_power: u64,
    enemy_power: u64,
    has_objective: bool,
    military_registry: &MilitaryRegistry,
) -> (FrontlineStance, MilitaryAiDecisionReason) {
    if frontline.border_region_pairs.is_empty() {
        return (
            FrontlineStance::Stopped,
            MilitaryAiDecisionReason::NoValidFrontline,
        );
    }

    if plan.assigned_army_ids.is_empty() {
        return (
            FrontlineStance::Stopped,
            MilitaryAiDecisionReason::NoAvailableArmy,
        );
    }

    // 即応可能な待機・移動中陸軍があるか確認
    let has_ready_armies = plan.assigned_army_ids.iter().any(|&id| {
        military_registry
            .armies
            .get(&id)
            .map(|a| a.status == ArmyStatus::Idle || a.status == ArmyStatus::Moving)
            .unwrap_or(false)
    });

    if !has_ready_armies {
        return (
            FrontlineStance::Defend,
            MilitaryAiDecisionReason::Recovering,
        );
    }

    if !has_objective {
        return (
            FrontlineStance::Defend,
            MilitaryAiDecisionReason::NoValidTarget,
        );
    }

    // 攻勢条件: 110% (own * 1000 >= enemy * 1100)
    // 攻勢維持条件: 90% (own * 1000 >= enemy * 900)
    let current_stance = plan.stance;

    let can_start_offensive = if enemy_power == 0 {
        own_power > 0
    } else {
        (own_power as u128) * 1000 >= (enemy_power as u128) * 1100
    };

    let can_continue_offensive = if enemy_power == 0 {
        own_power > 0
    } else {
        (own_power as u128) * 1000 >= (enemy_power as u128) * 900
    };

    if current_stance == FrontlineStance::Offensive {
        if can_continue_offensive {
            (
                FrontlineStance::Offensive,
                MilitaryAiDecisionReason::SufficientAdvantage,
            )
        } else {
            (
                FrontlineStance::Defend,
                MilitaryAiDecisionReason::ComparableStrength,
            )
        }
    } else if can_start_offensive {
        (
            FrontlineStance::Offensive,
            MilitaryAiDecisionReason::SufficientAdvantage,
        )
    } else if enemy_power > own_power {
        (
            FrontlineStance::Defend,
            MilitaryAiDecisionReason::EnemyStronger,
        )
    } else {
        (
            FrontlineStance::Defend,
            MilitaryAiDecisionReason::ComparableStrength,
        )
    }
}

/// 未割り当ての有効陸軍を評価し、適切な前線へ自動割り当てする
pub fn assign_unassigned_armies_to_frontlines(
    country_id: CountryId,
    war_registry: &WarRegistry,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    frontline_registry: &mut FrontlineRegistry,
    current_date: Option<&str>,
) {
    // AI国家の未割当・使用可能陸軍を取得 (ArmyId昇順)
    let mut available_army_ids: Vec<ArmyId> = military_registry
        .armies
        .values()
        .filter(|a| {
            a.owner == country_id
                && a.manpower > 0
                && a.status != ArmyStatus::Destroyed
                && !frontline_registry.army_frontline_map.contains_key(&a.id)
                && state_registry
                    .get(a.current_state)
                    .map(|s| !s.is_sea)
                    .unwrap_or(false)
        })
        .map(|a| a.id)
        .collect();

    available_army_ids.sort_by_key(|a| a.0);

    if available_army_ids.is_empty() {
        return;
    }

    // 進行中戦争の前線を取得 (WarId昇順, FrontlineId昇順)
    let mut target_frontlines: Vec<Frontline> = frontline_registry
        .frontlines
        .values()
        .filter(|fl| {
            if fl.attacker_country_id != country_id && fl.defender_country_id != country_id {
                return false;
            }
            if let Some(curr) = current_date
                && let Some(war) = war_registry.wars.get(&fl.war_id)
                && war.start_date == curr
            {
                return false;
            }
            war_registry
                .wars
                .get(&fl.war_id)
                .map(|w| w.status == WarStatus::Active)
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    target_frontlines.sort_by(|a, b| {
        a.war_id
            .0
            .cmp(&b.war_id.0)
            .then_with(|| a.frontline_id.0.cmp(&b.frontline_id.0))
    });

    if target_frontlines.is_empty() {
        return;
    }

    for army_id in available_army_ids {
        let army = match military_registry.armies.get(&army_id) {
            Some(a) => a,
            None => continue,
        };

        // 部隊が到達可能な前線を評価して最適前線を選択
        let mut best_fl_id: Option<FrontlineId> = None;

        // 到達可能性と戦力不足度を評価
        // 優先度: 1. 割当0部隊 -> 2. (own * enemy_B < own_B * enemy) 即ち戦力比率最小
        let mut candidates: Vec<(FrontlineId, usize, u64, u64)> = Vec::new();

        for fl in &target_frontlines {
            let enemy_id = if country_id == fl.attacker_country_id {
                fl.defender_country_id
            } else {
                fl.attacker_country_id
            };

            let front_regions = if country_id == fl.attacker_country_id {
                &fl.attacker_front_regions
            } else {
                &fl.defender_front_regions
            };

            let is_reachable = front_regions.iter().any(|&fr| {
                find_path(
                    army.current_state,
                    fr,
                    state_registry,
                    &[country_id],
                    &[enemy_id],
                )
                .is_some()
            });

            if is_reachable {
                let plan = frontline_registry.get_plan(fl.frontline_id, country_id);
                let assigned_count = plan.map(|p| p.assigned_army_ids.len()).unwrap_or(0);
                let (own_p, enemy_p) = plan
                    .map(|p| evaluate_powers(fl, p, country_id, military_registry, state_registry))
                    .unwrap_or((0, 0));

                candidates.push((fl.frontline_id, assigned_count, own_p, enemy_p));
            }
        }

        candidates.sort_by(|a, b| {
            // 割当部隊数0優先
            let a_has_zero = a.1 == 0;
            let b_has_zero = b.1 == 0;
            if a_has_zero != b_has_zero {
                return b_has_zero.cmp(&a_has_zero);
            }

            // 比率比較 (a_own * b_enemy vs b_own * a_enemy)
            let a_cross = (a.2 as u128) * ((b.3 + 1) as u128);
            let b_cross = (b.2 as u128) * ((a.3 + 1) as u128);
            a_cross.cmp(&b_cross).then_with(|| a.0.0.cmp(&b.0.0))
        });

        if let Some((fl_id, _, _, _)) = candidates.first() {
            best_fl_id = Some(*fl_id);
        }

        if let Some(target_fl_id) = best_fl_id {
            let _ = frontline_registry.assign_army(
                army_id,
                target_fl_id,
                country_id,
                military_registry,
                war_registry,
            );
        }
    }
}

/// 日次軍事AI処理のエントリーポイント
#[allow(clippy::too_many_arguments)]
pub fn process_daily_military_ai(
    current_day: u32,
    current_date: &str,
    player_country: &PlayerCountry,
    country_registry: &CountryRegistry,
    war_registry: &WarRegistry,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    frontline_registry: &mut FrontlineRegistry,
    ai_registry: &mut MilitaryAiRegistry,
) {
    let player_cid = player_country.0;

    // AI対象国家の抽出 (CountryId昇順)
    let mut ai_country_ids: Vec<CountryId> = country_registry
        .countries
        .iter()
        .map(|c| c.id)
        .filter(|&cid| Some(cid) != player_cid)
        .collect();
    ai_country_ids.sort_by_key(|c| c.0);

    for country_id in ai_country_ids {
        // 進行中の戦争を取得 (WarId昇順)
        let mut active_wars: Vec<&War> = war_registry
            .wars
            .values()
            .filter(|w| {
                w.status == WarStatus::Active
                    && (w.attackers.contains(&country_id) || w.defenders.contains(&country_id))
            })
            .collect();
        active_wars.sort_by_key(|w| w.id.0);

        if active_wars.is_empty() {
            let ai_state = ai_registry.get_or_create_mut(country_id);
            ai_state.last_evaluated_day = current_day;
            ai_state.last_decision_reason = MilitaryAiDecisionReason::NoActiveWar;
            ai_state.estimated_own_power = 0;
            ai_state.estimated_enemy_power = 0;
            ai_state.dirty = false;
            continue;
        }

        let ai_state = ai_registry.get_or_create_mut(country_id);
        let is_evaluation_due =
            current_day.saturating_sub(ai_state.last_evaluated_day) >= 3 || ai_state.dirty;

        if !is_evaluation_due {
            continue;
        }

        // 1. 未割当陸軍の複数前線分配 (宣戦布告当日の前線は対象外)
        assign_unassigned_armies_to_frontlines(
            country_id,
            war_registry,
            state_registry,
            military_registry,
            frontline_registry,
            Some(current_date),
        );

        let mut total_own_power: u64 = 0;
        let mut total_enemy_power: u64 = 0;
        let mut last_reason = MilitaryAiDecisionReason::NoValidFrontline;
        let mut skipped_due_to_start_date = false;

        for war in &active_wars {
            // 宣戦布告当日は軍事AIの移動命令を発行しない（プレイヤー保護と不意打ち防止）
            if war.start_date == current_date {
                skipped_due_to_start_date = true;
                continue;
            }

            if let Some(fl) = frontline_registry.get_frontline_for_war(war.id).cloned() {
                let mut plan = match frontline_registry
                    .get_plan(fl.frontline_id, country_id)
                    .cloned()
                {
                    Some(p) => p,
                    None => continue,
                };

                let (own_p, enemy_p) =
                    evaluate_powers(&fl, &plan, country_id, military_registry, state_registry);
                total_own_power += own_p;
                total_enemy_power += enemy_p;

                // 攻勢目標の検証または選定
                let objective = select_ai_offensive_objective(
                    war,
                    country_id,
                    state_registry,
                    country_registry,
                    &fl,
                );
                plan.objective_region_id = objective;

                let (new_stance, reason) = evaluate_military_ai_decision(
                    &fl,
                    &plan,
                    own_p,
                    enemy_p,
                    objective.is_some(),
                    military_registry,
                );

                last_reason = reason;
                plan.stance = new_stance;

                // 前線レジストリへ更新を書き戻す
                if let Some(p_mut) = frontline_registry.get_plan_mut(fl.frontline_id, country_id) {
                    p_mut.stance = plan.stance;
                    p_mut.objective_region_id = plan.objective_region_id;
                }
            }
        }

        let ai_state_mut = ai_registry.get_or_create_mut(country_id);
        ai_state_mut.last_evaluated_day = current_day;
        ai_state_mut.last_decision_reason = last_reason;
        ai_state_mut.estimated_own_power = total_own_power;
        ai_state_mut.estimated_enemy_power = total_enemy_power;
        ai_state_mut.dirty = skipped_due_to_start_date;
    }
}

/// System for daily military AI evaluation
#[allow(clippy::too_many_arguments)]
pub fn handle_daily_military_ai(
    mut day_events: MessageReader<DayChangedMessage>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    war_registry: Res<WarRegistry>,
    state_registry: Res<StateRegistry>,
    military_registry: Res<MilitaryRegistry>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
    mut ai_registry: ResMut<MilitaryAiRegistry>,
) {
    for event in day_events.read() {
        let current_date = format!("{:04}/{:02}/{:02}", event.year, event.month, event.day);
        let current_day_num =
            (event.year * 365 + event.month as i32 * 30 + event.day as i32) as u32;

        process_daily_military_ai(
            current_day_num,
            &current_date,
            &player_country,
            &country_registry,
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
            &mut ai_registry,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{ArmyId, CountryId, DivisionId, StateId, WarId};
    use crate::country::CountryData;
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};
    use crate::military::data::{
        ArmyStatus, ArmyUnit, DivisionDefinition, DivisionSize, DivisionType,
    };
    use crate::state::data::StateData;
    use crate::war::frontline::update_all_frontlines;

    fn setup_test_env() -> (
        PlayerCountry,
        CountryRegistry,
        StateRegistry,
        WarRegistry,
        MilitaryRegistry,
        FrontlineRegistry,
        MilitaryAiRegistry,
    ) {
        let player_country = PlayerCountry(Some(CountryId(1)));

        let c1 = CountryData {
            id: CountryId(1),
            name: "Player C1".to_string(),
            capital_state_id: StateId(1),
            ..default()
        };
        let c2 = CountryData {
            id: CountryId(2),
            name: "AI C2".to_string(),
            capital_state_id: StateId(4),
            ..default()
        };
        let c3 = CountryData {
            id: CountryId(3),
            name: "Neutral C3".to_string(),
            capital_state_id: StateId(5),
            ..default()
        };

        let country_registry = CountryRegistry {
            countries: vec![c1, c2, c3],
        };

        // 1(C1) -- 2(C1) -- 3(C2) -- 4(C2)
        let s1 = StateData {
            id: StateId(1),
            name: "State 1".to_string(),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(2)],
            world_position: [0.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s2 = StateData {
            id: StateId(2),
            name: "State 2".to_string(),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(1), StateId(3)],
            world_position: [100.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s3 = StateData {
            id: StateId(3),
            name: "State 3".to_string(),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(2), StateId(4)],
            world_position: [200.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s4 = StateData {
            id: StateId(4),
            name: "State 4".to_string(),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(3)],
            world_position: [300.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s5 = StateData {
            id: StateId(5),
            name: "State 5".to_string(),
            owner_country_id: CountryId(3),
            neighbors: Vec::new(),
            world_position: [400.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };

        let state_registry = StateRegistry::build(vec![s1, s2, s3, s4, s5]);

        let mut military_registry = MilitaryRegistry::default();
        military_registry.definitions.insert(
            DivisionId(1),
            DivisionDefinition {
                id: DivisionId(1),
                name: "Infantry".to_string(),
                division_type: DivisionType::Infantry,
                size: DivisionSize::Standard,
                required_manpower: 1000,
                required_equipment: 10.0,
                recruitment_days: 10,
                movement_speed: 1.0,
                attack: 10.0,
                defense: 10.0,
                breakthrough: 5.0,
                organization: 100.0,
                morale: 1.0,
                supply_usage: 1.0,
                maintenance_cost: 1.0,
            },
        );

        let mut war_registry = WarRegistry::default();
        let war = War {
            id: WarId(1),
            name: "C1 vs C2".to_string(),
            attackers: [CountryId(1)].into_iter().collect(),
            defenders: [CountryId(2)].into_iter().collect(),
            war_goals: vec![WarGoal {
                attacker: CountryId(1),
                defender: CountryId(2),
                goal_type: WarGoalType::ConquerState,
                target_states: vec![StateId(3)],
                base_peace_cost: 20.0,
                international_concern: 10.0,
                completion: 0.0,
                is_primary: true,
            }],
            start_date: "1800/01/01".to_string(),
            end_date: None,
            duration_days: 0,
            war_score: 0.0,
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: std::collections::HashSet::new(),
            status: WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: std::collections::HashSet::new(),
        };
        war_registry.wars.insert(war.id, war);

        let frontline_registry = FrontlineRegistry::default();
        let ai_registry = MilitaryAiRegistry::default();

        (
            player_country,
            country_registry,
            state_registry,
            war_registry,
            military_registry,
            frontline_registry,
            ai_registry,
        )
    }

    #[test]
    fn test_ai_target_country_filtering() {
        let (
            player_country,
            country_registry,
            state_registry,
            war_registry,
            military_registry,
            mut frontline_registry,
            mut ai_registry,
        ) = setup_test_env();

        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        process_daily_military_ai(
            1,
            "1800/01/02",
            &player_country,
            &country_registry,
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
            &mut ai_registry,
        );

        // プレイヤー(CountryId(1)) のAI状態は生成されない
        assert!(!ai_registry.ai_states.contains_key(&CountryId(1)));

        // AI国家(CountryId(2)) の状態が評価される
        assert!(ai_registry.ai_states.contains_key(&CountryId(2)));

        // 戦争非参加の中立国(CountryId(3)) の理由は NoActiveWar
        let c3_state = ai_registry.ai_states.get(&CountryId(3)).unwrap();
        assert_eq!(
            c3_state.last_decision_reason,
            MilitaryAiDecisionReason::NoActiveWar
        );
    }

    #[test]
    fn test_ai_army_assignment_and_power_evaluation() {
        let (
            player_country,
            country_registry,
            state_registry,
            war_registry,
            mut military_registry,
            mut frontline_registry,
            mut ai_registry,
        ) = setup_test_env();

        // C2 (AI国家) の部隊を追加
        let a_c2 = ArmyUnit {
            id: ArmyId(0),
            owner: CountryId(2),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(4),
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
            status: ArmyStatus::Idle,
            def_id: DivisionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let c2_army_id = military_registry.add_army(a_c2);

        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        process_daily_military_ai(
            1,
            "1800/01/02",
            &player_country,
            &country_registry,
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
            &mut ai_registry,
        );

        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(1))
            .unwrap()
            .frontline_id;
        let plan_c2 = frontline_registry.get_plan(fl_id, CountryId(2)).unwrap();

        // C2 の陸軍が前線へ自動割当されたことを確認
        assert!(plan_c2.assigned_army_ids.contains(&c2_army_id));
    }

    #[test]
    fn test_ai_decision_thresholds_and_hysteresis() {
        let (
            player_country,
            country_registry,
            state_registry,
            war_registry,
            mut military_registry,
            mut frontline_registry,
            mut ai_registry,
        ) = setup_test_env();

        // C2 (AI国家) に強力な陸軍を追加 (2000人)
        let a_c2 = ArmyUnit {
            id: ArmyId(0),
            owner: CountryId(2),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(4),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 2000,
            max_manpower: 2000,
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 1.0,
            max_morale: 1.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: ArmyStatus::Idle,
            def_id: DivisionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let c2_army_id = military_registry.add_army(a_c2);

        // C1 (敵国) に弱い陸軍を追加 (500人)
        let a_c1 = ArmyUnit {
            id: ArmyId(0),
            owner: CountryId(1),
            manpower: 500,
            max_manpower: 500,
            organization: 100.0,
            max_organization: 100.0,
            status: ArmyStatus::Idle,
            ..military_registry.armies.get(&c2_army_id).unwrap().clone()
        };
        military_registry.add_army(a_c1);

        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        process_daily_military_ai(
            1,
            "1800/01/02",
            &player_country,
            &country_registry,
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
            &mut ai_registry,
        );

        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(1))
            .unwrap()
            .frontline_id;
        let plan_c2 = frontline_registry.get_plan(fl_id, CountryId(2)).unwrap();

        // 2000 vs 500 (> 110%) なので Offensive になる
        assert_eq!(plan_c2.stance, FrontlineStance::Offensive);
        let ai_c2 = ai_registry.ai_states.get(&CountryId(2)).unwrap();
        assert_eq!(
            ai_c2.last_decision_reason,
            MilitaryAiDecisionReason::SufficientAdvantage
        );
    }
}
