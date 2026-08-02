use crate::building::construction::{ConstructionQueueItem, ConstructionStatus};
use crate::building::data::BuildingType;
use crate::common::{CountryId, StateId};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::data::DiplomacyRegistry;
use crate::economy::resources::ResourceType;
use crate::military::data::MilitaryRegistry;
use crate::military::pathfinding::find_path;
use crate::research::allocation::InProgressTech;
use crate::research::data::{TechnologyDefinition, TechnologyRegistry};
use crate::research::world_stage::WorldCivilizationState;
use crate::state::data::StateRegistry;
use crate::war::data::{WarRegistry, WarStatus};
use crate::war::frontline::update_all_frontlines;
use crate::war::justification::WarJustificationRegistry;
use crate::war::military_ai::{MilitaryAiRegistry, evaluate_army_power};
use crate::app::time::DayChangedMessage;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 高位国家AIモード
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CountryAiMode {
    #[default]
    Developing,
    Recovering,
    PreparingWar,
    AtWar,
    Disabled,
}

impl CountryAiMode {
    pub fn display_name(self) -> &'static str {
        match self {
            CountryAiMode::Developing => "Developing (Peacetime)",
            CountryAiMode::Recovering => "Recovering (Post-war/Economic)",
            CountryAiMode::PreparingWar => "Preparing War (Justifying)",
            CountryAiMode::AtWar => "At War",
            CountryAiMode::Disabled => "Disabled (Player)",
        }
    }
}

/// 国家AIの判断理由
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CountryAiDecisionReason {
    #[default]
    PeacetimeDevelopment,
    FoodShortageFarmPriority,
    RawMaterialMinePriority,
    SteelShortageSteelMillPriority,
    FundsShortage,
    NoBuildableState,
    NoResearchableTech,
    PostWarCooldown,
    NoAvailableArmies,
    NoReachableTargetCountry,
    InsufficientPowerAdvantage,
    TruceInEffect,
    JustificationInProgress,
    WarDeclarationPending,
    WarInProgress,
    PlayerControlled,
}

impl CountryAiDecisionReason {
    pub fn display_name(self) -> &'static str {
        match self {
            CountryAiDecisionReason::PeacetimeDevelopment => "Peacetime Development",
            CountryAiDecisionReason::FoodShortageFarmPriority => "Food Shortage (Farm Priority)",
            CountryAiDecisionReason::RawMaterialMinePriority => {
                "Raw Material Shortage (Mine Priority)"
            }
            CountryAiDecisionReason::SteelShortageSteelMillPriority => {
                "Steel Shortage (Steel Mill Priority)"
            }
            CountryAiDecisionReason::FundsShortage => "Insufficient Funds for Construction",
            CountryAiDecisionReason::NoBuildableState => "No Valid State for Construction",
            CountryAiDecisionReason::NoResearchableTech => "No Available Tech to Research",
            CountryAiDecisionReason::PostWarCooldown => "Post-war Cooldown (365 Days)",
            CountryAiDecisionReason::NoAvailableArmies => "No Armies Available for Invasion",
            CountryAiDecisionReason::NoReachableTargetCountry => "No Reachable Neighbor Country",
            CountryAiDecisionReason::InsufficientPowerAdvantage => "Power Advantage < 130%",
            CountryAiDecisionReason::TruceInEffect => "Truce or Pact in Effect",
            CountryAiDecisionReason::JustificationInProgress => "Justification in Progress",
            CountryAiDecisionReason::WarDeclarationPending => "War Declaration Pending",
            CountryAiDecisionReason::WarInProgress => "War in Progress (Military AI active)",
            CountryAiDecisionReason::PlayerControlled => "Disabled (Player Controlled)",
        }
    }
}

/// 1国家ごとの国家AI状態
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountryAiState {
    pub country_id: CountryId,
    pub mode: CountryAiMode,
    pub decision_reason: CountryAiDecisionReason,
    pub last_daily_evaluation_day: u32,
    pub last_weekly_evaluation_day: u32,
    pub last_monthly_evaluation_day: u32,
    pub cooldown_until_day: u32,
    pub dirty: bool,
}

impl CountryAiState {
    pub fn new(country_id: CountryId) -> Self {
        Self {
            country_id,
            mode: CountryAiMode::Developing,
            decision_reason: CountryAiDecisionReason::PeacetimeDevelopment,
            last_daily_evaluation_day: 0,
            last_weekly_evaluation_day: 0,
            last_monthly_evaluation_day: 0,
            cooldown_until_day: 0,
            dirty: true,
        }
    }
}

/// 全国家AI状態を管理するリソース
#[derive(Resource, Default, Debug, Clone, Serialize, Deserialize)]
pub struct CountryAiRegistry {
    pub ai_states: HashMap<CountryId, CountryAiState>,
    pub dirty: bool,
}

impl CountryAiRegistry {
    pub fn get_or_create_mut(&mut self, country_id: CountryId) -> &mut CountryAiState {
        self.ai_states
            .entry(country_id)
            .or_insert_with(|| CountryAiState::new(country_id))
    }

    pub fn mark_all_dirty(&mut self) {
        for state in self.ai_states.values_mut() {
            state.dirty = true;
        }
        self.dirty = true;
    }

    pub fn mark_country_dirty(&mut self, country_id: CountryId) {
        if let Some(state) = self.ai_states.get_mut(&country_id) {
            state.dirty = true;
        }
        self.dirty = true;
    }
}

/// 国家の全生存有効陸戦力を計算
pub fn calculate_country_total_power(
    country_id: CountryId,
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
) -> u64 {
    military_registry
        .armies
        .values()
        .filter(|a| {
            a.owner == country_id
                && a.manpower > 0
                && a.status != crate::military::data::ArmyStatus::Destroyed
                && state_registry
                    .get(a.current_state)
                    .map(|s| !s.is_sea)
                    .unwrap_or(false)
        })
        .map(evaluate_army_power)
        .sum()
}

/// 1. 建設AI (月次評価: game_day % 30 == country_id % 30 または dirty)
pub fn process_construction_ai(
    country_id: CountryId,
    country_registry: &mut CountryRegistry,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    ai_state: &mut CountryAiState,
) {
    let country = match country_registry.get_mut(country_id) {
        Some(c) => c,
        None => return,
    };

    // 最大2件までの建設キュー制限
    if country.construction_queue.len() >= 2 {
        return;
    }

    // 自国が法的所有かつ実効支配する陸上地域を取得 (StateId昇順)
    let mut owned_states: Vec<StateId> = state_registry
        .states
        .iter()
        .filter(|s| !s.is_sea && s.owner_country_id == country_id && s.controller() == country_id)
        .map(|s| s.id)
        .collect();
    owned_states.sort_by_key(|s| s.0);

    if owned_states.is_empty() {
        ai_state.decision_reason = CountryAiDecisionReason::NoBuildableState;
        return;
    }

    // 優先度判定: 食料(農場) -> 原材料(鉱山) -> 工場
    let target_building = if country.stockpile.get(ResourceType::Food) < 50.0 {
        ai_state.decision_reason = CountryAiDecisionReason::FoodShortageFarmPriority;
        BuildingType::Farm
    } else if country.stockpile.get(ResourceType::Wood) < 30.0
        || country.stockpile.get(ResourceType::Iron) < 30.0
        || country.stockpile.get(ResourceType::Coal) < 30.0
    {
        ai_state.decision_reason = CountryAiDecisionReason::RawMaterialMinePriority;
        BuildingType::Mine
    } else {
        ai_state.decision_reason = CountryAiDecisionReason::PeacetimeDevelopment;
        BuildingType::Factory
    };

    let def = match building_registry.get(target_building) {
        Some(d) => d,
        None => return,
    };

    if country.treasury < def.construction_cost {
        ai_state.decision_reason = CountryAiDecisionReason::FundsShortage;
        return;
    }

    // 候補地域を選択 (同種建物が少なく StateId 最小)
    let selected_state = owned_states.into_iter().find(|&sid| {
        let in_queue = country
            .construction_queue
            .iter()
            .any(|item| item.state_id == sid && item.building_type == target_building);
        if in_queue {
            return false;
        }
        if let Some(state_data) = state_registry.get(sid) {
            let current_level = state_data.building_level(target_building);
            current_level < def.max_level
        } else {
            false
        }
    });

    if let Some(state_id) = selected_state {
        let current_level = state_registry
            .get(state_id)
            .map(|s| s.building_level(target_building))
            .unwrap_or(0);

        country.treasury -= def.construction_cost;
        country.construction_queue.push(ConstructionQueueItem {
            state_id,
            building_type: target_building,
            target_level: current_level + 1,
            progress: 0.0,
            required_progress: def.required_progress,
            paid_cost: def.construction_cost,
            status: ConstructionStatus::InQueue,
        });
    }
}

/// 2. 研究AI (週次評価: game_day % 7 == country_id % 7 または dirty)
pub fn process_research_ai(
    country_id: CountryId,
    country_registry: &mut CountryRegistry,
    tech_registry: &TechnologyRegistry,
    world_state: &WorldCivilizationState,
    is_at_war: bool,
    ai_state: &mut CountryAiState,
) {
    let country = match country_registry.get_mut(country_id) {
        Some(c) => c,
        None => return,
    };

    // 利用可能な未研究技術を取得
    let mut available_techs: Vec<&TechnologyDefinition> = tech_registry
        .definitions
        .values()
        .filter(|tech| {
            !country
                .research_state
                .completed_technologies
                .contains(&tech.id)
                && !country.research_state.in_progress.contains_key(&tech.field)
                && tech.minimum_world_stage <= world_state.current_stage
                && tech
                    .prerequisites
                    .iter()
                    .all(|pre| country.research_state.completed_technologies.contains(pre))
        })
        .collect();

    if available_techs.is_empty() {
        ai_state.decision_reason = CountryAiDecisionReason::NoResearchableTech;
        return;
    }

    // 優先順位ソート (コスト昇順 -> ID昇順)
    available_techs.sort_by(|a, b| {
        let a_is_military =
            a.id.contains("military") || a.id.contains("weapon") || a.id.contains("army");
        let b_is_military =
            b.id.contains("military") || b.id.contains("weapon") || b.id.contains("army");

        if is_at_war && a_is_military != b_is_military {
            return b_is_military.cmp(&a_is_military);
        }

        a.cost
            .partial_cmp(&b.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });

    if let Some(chosen_tech) = available_techs.first() {
        country.research_state.in_progress.insert(
            chosen_tech.field,
            InProgressTech {
                tech_id: chosen_tech.id.clone(),
                progress: 0.0,
                cost: chosen_tech.cost,
            },
        );
    }
}

/// 3. 戦争準備・正当化AI (週次評価)
#[allow(clippy::too_many_arguments)]
pub fn process_war_preparation_ai(
    current_day: u32,
    country_id: CountryId,
    current_date_str: &str,
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    _war_registry: &WarRegistry,
    diplomacy_registry: &DiplomacyRegistry,
    justification_registry: &mut WarJustificationRegistry,
    ai_state: &mut CountryAiState,
) {
    // 既存の正当化が進行中かチェック
    let is_justifying = justification_registry
        .justifications
        .values()
        .any(|j| j.initiator == country_id);

    if is_justifying {
        ai_state.mode = CountryAiMode::PreparingWar;
        ai_state.decision_reason = CountryAiDecisionReason::JustificationInProgress;
        return;
    }

    // クールダウン中チェック
    if current_day < ai_state.cooldown_until_day {
        ai_state.decision_reason = CountryAiDecisionReason::PostWarCooldown;
        return;
    }

    // 自国の総有効戦力
    let own_power = calculate_country_total_power(country_id, military_registry, state_registry);
    if own_power == 0 {
        ai_state.decision_reason = CountryAiDecisionReason::NoAvailableArmies;
        return;
    }

    // 攻撃対象国候補の抽出 (陸上隣接国)
    let mut candidates: Vec<(CountryId, u64, StateId)> = Vec::new();

    let my_states: Vec<StateId> = state_registry
        .states
        .iter()
        .filter(|s| !s.is_sea && s.controller() == country_id)
        .map(|s| s.id)
        .collect();

    for other_country in &country_registry.countries {
        let tid = other_country.id;
        if tid == country_id {
            continue;
        }

        let target_power = calculate_country_total_power(tid, military_registry, state_registry);

        // 130% 戦力条件 (own * 1000 >= target * 1300)
        let has_power_advantage = if target_power == 0 {
            own_power > 0
        } else {
            (own_power as u128) * 1000 >= (target_power as u128) * 1300
        };

        if !has_power_advantage {
            continue;
        }

        // 隣接確認 & 到達可能な戦争目標地域の探索
        let enemy_states: Vec<StateId> = state_registry
            .states
            .iter()
            .filter(|s| !s.is_sea && s.controller() == tid)
            .map(|s| s.id)
            .collect();

        let mut valid_target_state: Option<StateId> = None;

        for &esid in &enemy_states {
            let reachable = my_states.iter().any(|&msid| {
                find_path(msid, esid, state_registry, &[country_id], &[tid]).is_some()
            });

            if reachable
                && justification_registry
                    .can_start_justification_with_date(
                        country_id,
                        tid,
                        esid,
                        country_registry,
                        state_registry,
                        diplomacy_registry,
                        Some(current_date_str),
                    )
                    .is_ok()
            {
                valid_target_state = Some(esid);
                break;
            }
        }

        if let Some(target_state) = valid_target_state {
            candidates.push((tid, target_power, target_state));
        }
    }

    // 優先度順ソート (対象国戦力が弱く, CountryId最小)
    candidates.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.0.cmp(&b.0.0)));

    if let Some((target_cid, _, target_sid)) = candidates.first() {
        let res = justification_registry.start_justification(
            country_id,
            *target_cid,
            *target_sid,
            current_date_str.to_string(),
            country_registry,
            state_registry,
            diplomacy_registry,
        );

        if res.is_ok() {
            ai_state.mode = CountryAiMode::PreparingWar;
            ai_state.decision_reason = CountryAiDecisionReason::JustificationInProgress;
        }
    } else {
        ai_state.decision_reason = CountryAiDecisionReason::InsufficientPowerAdvantage;
    }
}

/// 4. 宣戦布告AI (日次判定)
#[allow(clippy::too_many_arguments)]
pub fn process_war_declaration_ai(
    country_id: CountryId,
    current_date_str: &str,
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    war_registry: &mut WarRegistry,
    diplomacy_registry: &mut DiplomacyRegistry,
    justification_registry: &mut WarJustificationRegistry,
    frontline_registry: &mut crate::war::frontline::FrontlineRegistry,
    ai_registry: &mut MilitaryAiRegistry,
    ai_state: &mut CountryAiState,
) {
    let completed_justifications: Vec<(usize, CountryId, StateId)> = justification_registry
        .justifications
        .values()
        .filter(|j| j.initiator == country_id && j.is_ready)
        .map(|j| (j.id, j.target, j.target_state))
        .collect();

    for (j_id, target_cid, target_sid) in completed_justifications {
        let can_declare = war_registry
            .can_declare_war_with_date(
                country_id,
                target_cid,
                target_sid,
                country_registry,
                state_registry,
                diplomacy_registry,
                justification_registry,
                Some(current_date_str),
            )
            .is_ok();

        if can_declare {
            let res = war_registry.declare_war(
                country_id,
                target_cid,
                target_sid,
                current_date_str.to_string(),
                country_registry,
                state_registry,
                diplomacy_registry,
                justification_registry,
            );

            if res.is_ok() {
                // 正当化の削除
                justification_registry.justifications.remove(&j_id);

                // 戦争生成後の前線再構築 & 軍事AI dirty 化
                update_all_frontlines(
                    war_registry,
                    state_registry,
                    military_registry,
                    frontline_registry,
                );
                ai_registry.mark_all_dirty();

                ai_state.mode = CountryAiMode::AtWar;
                ai_state.decision_reason = CountryAiDecisionReason::WarInProgress;
                break;
            }
        }
    }
}

/// 日次国家AI処理のエントリーポイント
#[allow(clippy::too_many_arguments)]
pub fn process_daily_country_ai(
    current_day: u32,
    current_date_str: &str,
    player_country: &PlayerCountry,
    country_registry: &mut CountryRegistry,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    military_registry: &MilitaryRegistry,
    war_registry: &mut WarRegistry,
    diplomacy_registry: &mut DiplomacyRegistry,
    tech_registry: &TechnologyRegistry,
    world_state: &WorldCivilizationState,
    justification_registry: &mut WarJustificationRegistry,
    frontline_registry: &mut crate::war::frontline::FrontlineRegistry,
    military_ai_registry: &mut MilitaryAiRegistry,
    country_ai_registry: &mut CountryAiRegistry,
) {
    let player_cid = player_country.0;

    let mut ai_country_ids: Vec<CountryId> = country_registry
        .countries
        .iter()
        .map(|c| c.id)
        .filter(|&cid| Some(cid) != player_cid)
        .collect();
    ai_country_ids.sort_by_key(|c| c.0);

    for country_id in ai_country_ids {
        let is_at_war = war_registry.wars.values().any(|w| {
            w.status == WarStatus::Active
                && (w.attackers.contains(&country_id) || w.defenders.contains(&country_id))
        });

        let ai_state = country_ai_registry.get_or_create_mut(country_id);

        // 高位モードの設定
        if is_at_war {
            ai_state.mode = CountryAiMode::AtWar;
            ai_state.decision_reason = CountryAiDecisionReason::WarInProgress;
        } else if ai_state.mode == CountryAiMode::AtWar {
            ai_state.mode = CountryAiMode::Recovering;
            ai_state.decision_reason = CountryAiDecisionReason::PostWarCooldown;
        }

        // 1. 日次宣戦布告AI
        process_war_declaration_ai(
            country_id,
            current_date_str,
            country_registry,
            state_registry,
            military_registry,
            war_registry,
            diplomacy_registry,
            justification_registry,
            frontline_registry,
            military_ai_registry,
            ai_state,
        );

        // 2. 週次評価 (game_day % 7 == country_id.0 % 7 または dirty)
        let is_weekly_due = (current_day as usize % 7 == country_id.0 % 7) || ai_state.dirty;
        if is_weekly_due {
            process_research_ai(
                country_id,
                country_registry,
                tech_registry,
                world_state,
                is_at_war,
                ai_state,
            );

            if !is_at_war {
                process_war_preparation_ai(
                    current_day,
                    country_id,
                    current_date_str,
                    country_registry,
                    state_registry,
                    military_registry,
                    war_registry,
                    diplomacy_registry,
                    justification_registry,
                    ai_state,
                );
            }
            ai_state.last_weekly_evaluation_day = current_day;
        }

        // 3. 月次評価 (game_day % 30 == country_id.0 % 30 または dirty)
        let is_monthly_due = (current_day as usize % 30 == country_id.0 % 30) || ai_state.dirty;
        if is_monthly_due {
            process_construction_ai(
                country_id,
                country_registry,
                state_registry,
                building_registry,
                ai_state,
            );
            ai_state.last_monthly_evaluation_day = current_day;
        }

        ai_state.last_daily_evaluation_day = current_day;
        ai_state.dirty = false;
    }
}

/// System for daily country AI evaluation
#[allow(clippy::too_many_arguments)]
pub fn handle_daily_country_ai(
    mut day_events: MessageReader<DayChangedMessage>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    building_registry: Res<crate::building::data::BuildingRegistry>,
    military_registry: Res<MilitaryRegistry>,
    mut war_registry: ResMut<WarRegistry>,
    mut diplomacy_registry: ResMut<DiplomacyRegistry>,
    tech_registry: Res<TechnologyRegistry>,
    world_state: Res<WorldCivilizationState>,
    mut justification_registry: ResMut<WarJustificationRegistry>,
    mut frontline_registry: ResMut<crate::war::frontline::FrontlineRegistry>,
    mut military_ai_registry: ResMut<MilitaryAiRegistry>,
    mut country_ai_registry: ResMut<CountryAiRegistry>,
) {
    for event in day_events.read() {
        let current_date = format!("{:04}/{:02}/{:02}", event.year, event.month, event.day);
        let current_day_num =
            (event.year * 365 + event.month as i32 * 30 + event.day as i32) as u32;

        process_daily_country_ai(
            current_day_num,
            &current_date,
            &player_country,
            &mut country_registry,
            &state_registry,
            &building_registry,
            &military_registry,
            &mut war_registry,
            &mut diplomacy_registry,
            &tech_registry,
            &world_state,
            &mut justification_registry,
            &mut frontline_registry,
            &mut military_ai_registry,
            &mut country_ai_registry,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::data::BuildingType;
    use crate::country::CountryData;
    use crate::state::data::StateData;
    use std::collections::HashMap;

    #[allow(clippy::type_complexity)]
    fn setup_test_env() -> (
        PlayerCountry,
        CountryRegistry,
        StateRegistry,
        crate::building::data::BuildingRegistry,
        WarRegistry,
        MilitaryRegistry,
        DiplomacyRegistry,
        TechnologyRegistry,
        WorldCivilizationState,
        WarJustificationRegistry,
        crate::war::frontline::FrontlineRegistry,
        MilitaryAiRegistry,
        CountryAiRegistry,
    ) {
        let player_country = PlayerCountry(Some(CountryId(1)));

        let c1 = CountryData {
            id: CountryId(1),
            name: "Player C1".to_string(),
            capital_state_id: StateId(1),
            treasury: 1000.0,
            ..default()
        };
        let c2 = CountryData {
            id: CountryId(2),
            name: "AI C2".to_string(),
            capital_state_id: StateId(3),
            treasury: 1000.0,
            ..default()
        };

        let country_registry = CountryRegistry {
            countries: vec![c1, c2],
        };

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
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(1), StateId(3)],
            world_position: [100.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s3 = StateData {
            id: StateId(3),
            name: "State 3".to_string(),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(2)],
            world_position: [200.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };

        let state_registry = StateRegistry::build(vec![s1, s2, s3]);
        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry.definitions.insert(
            BuildingType::Farm,
            crate::building::data::BuildingDefinition {
                building_type: BuildingType::Farm,
                name: "Farm".to_string(),
                construction_cost: 100.0,
                required_progress: 10.0,
                required_workforce: 100.0,
                logistics_cost: 0.0,
                input_resources: HashMap::new(),
                output_resources: HashMap::new(),
                maintenance_cost: 1.0,
                max_level: 5,
                science_output: 0.0,
                magic_output: 0.0,
                railway_capacity_bonus: 0.0,
            },
        );

        (
            player_country,
            country_registry,
            state_registry,
            building_registry,
            WarRegistry::default(),
            MilitaryRegistry::default(),
            DiplomacyRegistry::default(),
            TechnologyRegistry::default(),
            WorldCivilizationState::default(),
            WarJustificationRegistry::default(),
            crate::war::frontline::FrontlineRegistry::default(),
            MilitaryAiRegistry::default(),
            CountryAiRegistry::default(),
        )
    }

    #[test]
    fn test_player_country_not_controlled_by_country_ai() {
        let (
            player_country,
            mut country_registry,
            state_registry,
            building_registry,
            mut war_registry,
            military_registry,
            mut diplomacy_registry,
            tech_registry,
            world_state,
            mut justification_registry,
            mut frontline_registry,
            mut military_ai_registry,
            mut country_ai_registry,
        ) = setup_test_env();

        process_daily_country_ai(
            1,
            "1800/01/01",
            &player_country,
            &mut country_registry,
            &state_registry,
            &building_registry,
            &military_registry,
            &mut war_registry,
            &mut diplomacy_registry,
            &tech_registry,
            &world_state,
            &mut justification_registry,
            &mut frontline_registry,
            &mut military_ai_registry,
            &mut country_ai_registry,
        );

        // プレイヤー国家 (CountryId(1)) は国家AI状態が作成されない
        assert!(!country_ai_registry.ai_states.contains_key(&CountryId(1)));

        // AI国家 (CountryId(2)) は作成される
        assert!(country_ai_registry.ai_states.contains_key(&CountryId(2)));
    }

    #[test]
    fn test_ai_construction_validity_and_cost() {
        let (
            _,
            mut country_registry,
            state_registry,
            building_registry,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            mut country_ai_registry,
        ) = setup_test_env();

        let ai_state = country_ai_registry.get_or_create_mut(CountryId(2));
        let initial_treasury = country_registry.get(CountryId(2)).unwrap().treasury;

        process_construction_ai(
            CountryId(2),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state,
        );

        let c2 = country_registry.get(CountryId(2)).unwrap();
        // 建設キューに1件追加され、資金が減っていることを確認
        assert_eq!(c2.construction_queue.len(), 1);
        assert!(c2.treasury < initial_treasury);
    }
}
