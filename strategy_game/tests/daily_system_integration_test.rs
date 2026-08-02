#![allow(clippy::too_many_arguments)]

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DailySimulationSet, GameDate, GamePaused};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::construction::{ConstructionQueueItem, ConstructionStatus};
use strategy_game::building::data::BuildingType;
use strategy_game::common::{ArmyId, BattleId, CountryId, DivisionId, FrontlineId, StateId, WarId};
use strategy_game::country::country_ai::{
    CountryAiDecisionReason, CountryAiMode, CountryAiRegistry,
};
use strategy_game::country::{
    CountryPlugin, CountryRegistry, EconomicSystem, GovernmentType, PlayerCountry,
};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::diplomacy::crisis::{WarGoal, WarGoalType};
use strategy_game::diplomacy::data::{
    ActiveDiplomaticActivity, ActiveTreaty, DiplomacyRegistry, DiplomaticActivityType, TreatyType,
};
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::economic_state::EconomicState;
use strategy_game::economy::resources::{ResourceType, StateResourceDeposit};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::{BattleRegistry, BattleStatus};
use strategy_game::military::data::{
    ArmyStatus, DivisionDefinition, DivisionSize, DivisionType, MilitaryRegistry,
};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::politics::interest_groups::{InterestGroupState, InterestGroupType};
use strategy_game::politics::reform::PoliticalReform;
use strategy_game::politics::values::ValueAxis;
use strategy_game::research::ResearchPlugin;
use strategy_game::research::allocation::{CountryResearchState, InProgressTech};
use strategy_game::research::data::TechnologyField;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::{OccupationPolicy, StateRegistry};
use strategy_game::war::WarPlugin;
use strategy_game::war::capitulation::CapitulationResult;
use strategy_game::war::data::{WarRegistry, WarStatus};
use strategy_game::war::frontline::{FrontlineRegistry, FrontlineStance};
use strategy_game::war::justification::WarJustificationRegistry;
use strategy_game::war::military_ai::{MilitaryAiDecisionReason, MilitaryAiRegistry};

// ─────────────────────────────────────────────────────────────────────────────
// SetBoundarySnapshot: 全フィールド・全関連情報を保持する境界観測スナップショット
// ─────────────────────────────────────────────────────────────────────────────

/// 建設キューアイテムの全フィールドスナップショット (ConstructionQueueItem 全6フィールド対応)
#[derive(Debug, Clone, PartialEq)]
pub struct ConstructionQueueItemSnapshot {
    pub state_id: StateId,
    pub building_type: BuildingType,
    pub target_level: u32,
    pub progress: f64,
    pub required_progress: f64,
    pub paid_cost: f64,
    pub status: ConstructionStatus,
}

/// 研究進行中の全フィールドスナップショット (InProgressTech 全フィールド対応)
#[derive(Debug, Clone, PartialEq)]
pub struct InProgressTechSnapshot {
    pub field: TechnologyField,
    pub tech_id: String,
    pub progress: f64,
    pub cost: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResearchStateSnapshot {
    pub completed_technologies: Vec<String>,
    pub in_progress: Vec<InProgressTechSnapshot>,
    pub allocation: [f32; 4],
    pub preferred_fields: Vec<TechnologyField>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterestGroupSnapshot {
    pub group_type: InterestGroupType,
    pub influence: f32,
    pub approval: f32,
    pub support_for_current_reform: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PoliticalReformSnapshot {
    pub axis: ValueAxis,
    pub start_value: f32,
    pub target_value: f32,
    pub progress: f32,
    pub required_progress: f32,
    pub monthly_progress: f32,
    pub support_score: f32,
    pub opposition_score: f32,
    pub clergy_resistance: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecruitmentQueueItemSnapshot {
    pub division_id: DivisionId,
    pub target_state: StateId,
    pub days_remaining: u32,
    pub total_days: u32,
}

/// 国家詳細スナップショット (CountryData 主要フィールド全対応)
#[derive(Debug, Clone, PartialEq)]
pub struct CountryDetailSnapshot {
    pub id: CountryId,
    pub name: String,
    pub map_color: [f32; 3],
    pub capital_state_id: StateId,
    pub treasury: f64,
    pub government_type: GovernmentType,
    pub economic_system: EconomicSystem,
    pub stockpile: Vec<(ResourceType, f64)>,
    pub tax_rate: f32,
    pub construction_queue: Vec<ConstructionQueueItemSnapshot>,
    pub monthly_income: f64,
    pub monthly_expenses: f64,
    pub monthly_balance: f64,
    pub science_research_capacity: f64,
    pub magic_research_capacity: f64,
    pub economic_state: EconomicState,
    pub research_state: ResearchStateSnapshot,
    pub country_values: [f32; 3],
    pub interest_groups: Vec<InterestGroupSnapshot>,
    pub current_reform: Option<PoliticalReformSnapshot>,
    pub available_manpower: u64,
    pub mobilized_manpower: u64,
    pub total_military_equipment_required: f64,
    pub total_military_equipment_available: f64,
    pub monthly_military_expenses: f64,
    pub recruitment_queue: Vec<RecruitmentQueueItemSnapshot>,
}

/// 正当化スナップショット (WarJustification 全フィールド)
#[derive(Debug, Clone, PartialEq)]
pub struct JustificationDetailSnapshot {
    pub id: usize,
    pub initiator: CountryId,
    pub target: CountryId,
    pub target_state: StateId,
    pub start_date: String,
    pub days_passed: u32,
    pub required_days: u32,
    pub is_ready: bool,
}

/// 国家AIスナップショット (CountryAiState 主要フィールド)
#[derive(Debug, Clone, PartialEq)]
pub struct CountryAiDetailSnapshot {
    pub country_id: CountryId,
    pub mode: CountryAiMode,
    pub decision_reason: CountryAiDecisionReason,
    pub last_daily_evaluation_day: u32,
    pub last_weekly_evaluation_day: u32,
    pub last_monthly_evaluation_day: u32,
    pub cooldown_until_day: u32,
    pub dirty: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveTreatySnapshot {
    pub treaty_type: TreatyType,
    pub countries: (CountryId, CountryId),
    pub signed_date: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveDiplomaticActivitySnapshot {
    pub activity_type: DiplomaticActivityType,
    pub initiator: CountryId,
    pub target: CountryId,
    pub days_remaining: u32,
    pub daily_opinion_change: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiplomaticRelationSnapshot {
    pub countries: (CountryId, CountryId),
    pub opinion: f32,
    pub tension: f32,
    pub trust: f32,
    pub threat: f32,
    pub at_war: bool,
    pub has_military_access: bool,
    pub truce_until: Option<String>,
    pub treaties: Vec<ActiveTreatySnapshot>,
    pub cooldowns: Vec<(CountryId, u32)>,
    pub active_activity: Option<ActiveDiplomaticActivitySnapshot>,
    pub last_updated_date: String,
}

/// 軍事AIスナップショット (MilitaryAiState 全フィールド)
#[derive(Debug, Clone, PartialEq)]
pub struct MilitaryAiDetailSnapshot {
    pub country_id: CountryId,
    pub last_evaluated_day: u32,
    pub last_decision_reason: MilitaryAiDecisionReason,
    pub estimated_own_power: u64,
    pub estimated_enemy_power: u64,
    pub dirty: bool,
}

/// 前線プランスナップショット (FrontlinePlan 全フィールド)
#[derive(Debug, Clone, PartialEq)]
pub struct FrontlinePlanSnapshot {
    pub frontline_id: FrontlineId,
    pub commanding_country_id: CountryId,
    pub stance: FrontlineStance,
    pub objective_region_id: Option<StateId>,
    pub assigned_army_ids: Vec<ArmyId>,
}

/// 前線スナップショット (Frontline 全フィールド + plans + army_frontline_map)
#[derive(Debug, Clone, PartialEq)]
pub struct FrontlineDetailSnapshot {
    pub frontline_id: FrontlineId,
    pub war_id: WarId,
    pub attacker_country_id: CountryId,
    pub defender_country_id: CountryId,
    pub attacker_front_regions: Vec<StateId>,
    pub defender_front_regions: Vec<StateId>,
    pub border_region_pairs: Vec<(StateId, StateId)>,
    /// この前線に紐づく全プラン (FrontlineId, CountryId キーでソート済み)
    pub plans: Vec<FrontlinePlanSnapshot>,
}

/// 陸軍スナップショット (ArmyUnit 全フィールド)
#[derive(Debug, Clone, PartialEq)]
pub struct ArmyDetailSnapshot {
    pub id: ArmyId,
    pub owner: CountryId,
    pub division_type: DivisionType,
    pub size: DivisionSize,
    pub current_state: StateId,
    pub destination: Option<StateId>,
    pub current_path: Vec<StateId>,
    pub target_state: Option<StateId>,
    pub manpower: u64,
    pub max_manpower: u64,
    pub equipment: f64,
    pub max_equipment: f64,
    pub organization: f32,
    pub max_organization: f32,
    pub morale: f32,
    pub max_morale: f32,
    pub experience: f32,
    pub supply_ratio: f32,
    pub movement_progress: f32,
    pub status: ArmyStatus,
    pub def_id: DivisionId,
    pub attack_power: i32,
    pub defense_power: i32,
    pub combat_id: Option<BattleId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DivisionDefinitionSnapshot {
    pub id: DivisionId,
    pub name: String,
    pub division_type: DivisionType,
    pub size: DivisionSize,
    pub required_manpower: u64,
    pub required_equipment: f64,
    pub recruitment_days: u32,
    pub movement_speed: f32,
    pub attack: f32,
    pub defense: f32,
    pub breakthrough: f32,
    pub organization: f32,
    pub morale: f32,
    pub supply_usage: f32,
    pub maintenance_cost: f64,
}

/// 戦闘スナップショット (Battle 全フィールド)
#[derive(Debug, Clone, PartialEq)]
pub struct BattleDetailSnapshot {
    pub id: BattleId,
    pub war_id: WarId,
    pub state_id: StateId,
    pub attacker_country: CountryId,
    pub defender_country: CountryId,
    pub attacker_army_id: ArmyId,
    pub defender_army_id: ArmyId,
    pub start_date: String,
    pub elapsed_days: u32,
    pub status: BattleStatus,
    pub attacker_origin_state: StateId,
}

/// 戦争スナップショット (War 全関連フィールド)
#[derive(Debug, Clone, PartialEq)]
pub struct WarDetailSnapshot {
    pub id: WarId,
    pub name: String,
    pub attackers: Vec<CountryId>,
    pub defenders: Vec<CountryId>,
    pub war_goals: Vec<WarGoalSnapshot>,
    pub start_date: String,
    pub end_date: Option<String>,
    pub duration_days: u32,
    pub war_score: f32,
    pub attacker_war_exhaustion: f32,
    pub defender_war_exhaustion: f32,
    pub occupied_states: Vec<StateId>,
    pub status: WarStatus,
    pub winner: Option<CountryId>,
    pub end_reason: Option<String>,
    pub applied_terms: Vec<String>,
    pub won_attacker_battles: u32,
    pub won_defender_battles: u32,
    pub processed_battle_ids: Vec<BattleId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WarGoalSnapshot {
    pub attacker: CountryId,
    pub defender: CountryId,
    pub goal_type: WarGoalType,
    pub target_states: Vec<StateId>,
    pub base_peace_cost: f32,
    pub international_concern: f32,
    pub completion: f32,
    pub is_primary: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateResourceDepositSnapshot {
    pub resource_type: ResourceType,
    pub base_output: f64,
    pub discovered: bool,
    pub development_level: u32,
}

/// 州スナップショット (StateData 主要フィールド)
#[derive(Debug, Clone, PartialEq)]
pub struct StateDetailSnapshot {
    pub id: StateId,
    pub name: String,
    pub owner_country_id: CountryId,
    pub population: u64,
    pub workforce_ratio: f32,
    pub employed_workforce: u64,
    pub unemployed_workforce: u64,
    pub education: f32,
    pub living_standard: f32,
    pub unrest: f32,
    pub base_logistics_capacity: f32,
    pub logistics_capacity: f32,
    pub logistics_usage: f32,
    pub logistics_ratio: f32,
    pub buildings: Vec<(BuildingType, u32)>,
    pub building_operation_ratios: Vec<(BuildingType, f32)>,
    pub resource_deposits: Vec<StateResourceDepositSnapshot>,
    pub world_position: [f32; 2],
    pub size: [f32; 2],
    pub neighbors: Vec<StateId>,
    pub integration: f32,
    pub controller_country: Option<CountryId>,
    pub occupation_progress: f32,
    pub original_owner: Option<CountryId>,
    pub occupation_policy: Option<OccupationPolicy>,
    pub war_id: Option<WarId>,
    pub is_sea: bool,
}

/// ゲーム全体状態スナップショット (Test B 完全不変性検証用)
#[derive(Debug, Clone, PartialEq)]
pub struct GameStateSnapshot {
    pub date: GameDate,
    pub paused: bool,
    pub player_country: Option<CountryId>,
    pub countries: Vec<CountryDetailSnapshot>,
    pub diplomatic_relations: Vec<DiplomaticRelationSnapshot>,
    pub justifications: Vec<JustificationDetailSnapshot>,
    pub justification_next_id: usize,
    pub country_ai: Vec<CountryAiDetailSnapshot>,
    pub country_ai_registry_dirty: bool,
    pub military_ai: Vec<MilitaryAiDetailSnapshot>,
    pub military_ai_registry_dirty: bool,
    pub division_definitions: Vec<DivisionDefinitionSnapshot>,
    pub armies: Vec<ArmyDetailSnapshot>,
    pub military_next_army_id: usize,
    pub battles: Vec<BattleDetailSnapshot>,
    pub battle_next_id: usize,
    pub wars: Vec<WarDetailSnapshot>,
    pub war_next_id: usize,
    pub frontlines: Vec<FrontlineDetailSnapshot>,
    pub frontline_plans: Vec<FrontlinePlanSnapshot>,
    pub next_frontline_id: usize,
    pub army_frontline_map: Vec<(ArmyId, FrontlineId)>,
    pub frontline_generated_movements: Vec<ArmyId>,
    pub states: Vec<StateDetailSnapshot>,
    pub state_index_entries: Vec<(usize, usize)>,
}

// ─────────────────────────────────────────────────────────────────────────────
// SetBoundarySnapshot: 境界観測の軽量スナップショット (各 Observer が保持)
// ─────────────────────────────────────────────────────────────────────────────

/// Set境界で保持する軽量観測スナップショット
///
/// # フィールド説明
/// - `frontline_plan_count`: `frontline_reg.plans.len()` として算出
/// - `capitulation_result`: war.status と war.end_reason から事後的に導出した判定値
///   (DefenderCapitulated / AttackerCapitulated / None)
#[derive(Debug, Clone, PartialEq)]
pub struct SetBoundarySnapshot {
    pub war_record_count: usize,
    pub active_war_count: usize,
    pub frontline_count: usize,
    pub ai_army_destinations: Vec<Option<StateId>>,
    pub ai_army_paths: Vec<Vec<StateId>>,
    pub ai_army_positions: Vec<StateId>,
    pub ai_army_progresses: Vec<f32>,
    pub occupied_states_count: usize,
    pub war_score: f32,
    pub war_status: Option<WarStatus>,
    pub justification_is_ready: Option<bool>,
    pub winner: Option<CountryId>,
    pub territory_owner: Option<CountryId>,
    pub ai_has_evaluated: bool,
    pub military_ai_last_eval_day: Option<u32>,
    pub military_ai_dirty: Option<bool>,
    pub military_ai_decision_reason: Option<MilitaryAiDecisionReason>,
    pub military_ai_stance: Option<FrontlineStance>,
    pub military_ai_assigned_army_ids: Vec<ArmyId>,
    pub military_ai_assigned_army_owners: Vec<(ArmyId, Option<CountryId>)>,
    pub frontline_plan_count: usize,
    pub capitulation_result: Option<CapitulationResult>,
    pub military_ai_estimated_own_power: Option<u64>,
    pub military_ai_estimated_enemy_power: Option<u64>,
    pub player_construction_queue: Vec<ConstructionQueueItemSnapshot>,
    pub player_research_state: Option<ResearchStateSnapshot>,
    pub player_armies: Vec<ArmyDetailSnapshot>,
}

/// Set境界観測の全結果を蓄積するリソース
#[derive(Resource, Default, Debug, Clone)]
pub struct SetBoundaryObserver {
    pub economy_snap: Option<SetBoundarySnapshot>,
    pub diplomacy_snap: Option<SetBoundarySnapshot>,
    pub country_ai_snap: Option<SetBoundarySnapshot>,
    pub war_prep_snap: Option<SetBoundarySnapshot>,
    pub military_ai_snap: Option<SetBoundarySnapshot>,
    pub frontline_orders_snap: Option<SetBoundarySnapshot>,
    pub military_action_snap: Option<SetBoundarySnapshot>,
    pub war_resolution_snap: Option<SetBoundarySnapshot>,
    pub research_snap: Option<SetBoundarySnapshot>,
}

fn snapshot_construction_queue(
    queue: &[ConstructionQueueItem],
) -> Vec<ConstructionQueueItemSnapshot> {
    queue
        .iter()
        .map(|item| ConstructionQueueItemSnapshot {
            state_id: item.state_id,
            building_type: item.building_type,
            target_level: item.target_level,
            progress: item.progress,
            required_progress: item.required_progress,
            paid_cost: item.paid_cost,
            status: item.status,
        })
        .collect()
}

fn snapshot_research_state(state: &CountryResearchState) -> ResearchStateSnapshot {
    let mut completed_technologies: Vec<String> =
        state.completed_technologies.iter().cloned().collect();
    completed_technologies.sort();

    let mut in_progress: Vec<InProgressTechSnapshot> = state
        .in_progress
        .iter()
        .map(|(&field, tech)| InProgressTechSnapshot {
            field,
            tech_id: tech.tech_id.clone(),
            progress: tech.progress,
            cost: tech.cost,
        })
        .collect();
    in_progress.sort_by_key(|tech| tech.field as u8);

    ResearchStateSnapshot {
        completed_technologies,
        in_progress,
        allocation: [
            state.allocation.science,
            state.allocation.magic,
            state.allocation.military,
            state.allocation.fusion,
        ],
        preferred_fields: state.preferred_fields.clone(),
    }
}

fn snapshot_reform(reform: &PoliticalReform) -> PoliticalReformSnapshot {
    PoliticalReformSnapshot {
        axis: reform.axis,
        start_value: reform.start_value,
        target_value: reform.target_value,
        progress: reform.progress,
        required_progress: reform.required_progress,
        monthly_progress: reform.monthly_progress,
        support_score: reform.support_score,
        opposition_score: reform.opposition_score,
        clergy_resistance: reform.clergy_resistance,
    }
}

fn snapshot_interest_group(
    group_type: InterestGroupType,
    state: &InterestGroupState,
) -> InterestGroupSnapshot {
    InterestGroupSnapshot {
        group_type,
        influence: state.influence,
        approval: state.approval,
        support_for_current_reform: state.support_for_current_reform,
    }
}

fn snapshot_army(army: &strategy_game::military::data::ArmyUnit) -> ArmyDetailSnapshot {
    ArmyDetailSnapshot {
        id: army.id,
        owner: army.owner,
        division_type: army.division_type,
        size: army.size,
        current_state: army.current_state,
        destination: army.destination,
        current_path: army.current_path.clone(),
        target_state: army.target_state,
        manpower: army.manpower,
        max_manpower: army.max_manpower,
        equipment: army.equipment,
        max_equipment: army.max_equipment,
        organization: army.organization,
        max_organization: army.max_organization,
        morale: army.morale,
        max_morale: army.max_morale,
        experience: army.experience,
        supply_ratio: army.supply_ratio,
        movement_progress: army.movement_progress,
        status: army.status,
        def_id: army.def_id,
        attack_power: army.attack_power,
        defense_power: army.defense_power,
        combat_id: army.combat_id,
    }
}

fn snapshot_country(country: &strategy_game::country::CountryData) -> CountryDetailSnapshot {
    let mut stockpile: Vec<(ResourceType, f64)> = country
        .stockpile
        .amounts
        .iter()
        .map(|(&resource, &amount)| (resource, amount))
        .collect();
    stockpile.sort_by_key(|(resource, _)| *resource as u8);

    let mut interest_groups: Vec<InterestGroupSnapshot> = country
        .politics
        .interest_groups
        .iter()
        .map(|(&group_type, state)| snapshot_interest_group(group_type, state))
        .collect();
    interest_groups.sort_by_key(|group| group.group_type as u8);

    CountryDetailSnapshot {
        id: country.id,
        name: country.name.clone(),
        map_color: country.map_color,
        capital_state_id: country.capital_state_id,
        treasury: country.treasury,
        government_type: country.government_type,
        economic_system: country.economic_system,
        stockpile,
        tax_rate: country.tax_rate,
        construction_queue: snapshot_construction_queue(&country.construction_queue),
        monthly_income: country.monthly_income,
        monthly_expenses: country.monthly_expenses,
        monthly_balance: country.monthly_balance,
        science_research_capacity: country.science_research_capacity,
        magic_research_capacity: country.magic_research_capacity,
        economic_state: country.economic_state,
        research_state: snapshot_research_state(&country.research_state),
        country_values: [
            country.politics.values.science_magic,
            country.politics.values.individual_state,
            country.politics.values.secular_religious,
        ],
        interest_groups,
        current_reform: country.current_reform.as_ref().map(snapshot_reform),
        available_manpower: country.available_manpower,
        mobilized_manpower: country.mobilized_manpower,
        total_military_equipment_required: country.total_military_equipment_required,
        total_military_equipment_available: country.total_military_equipment_available,
        monthly_military_expenses: country.monthly_military_expenses,
        recruitment_queue: country
            .recruitment_queue
            .iter()
            .map(|item| RecruitmentQueueItemSnapshot {
                division_id: item.division_id,
                target_state: item.target_state,
                days_remaining: item.days_remaining,
                total_days: item.total_days,
            })
            .collect(),
    }
}

fn snapshot_division(definition: &DivisionDefinition) -> DivisionDefinitionSnapshot {
    DivisionDefinitionSnapshot {
        id: definition.id,
        name: definition.name.clone(),
        division_type: definition.division_type,
        size: definition.size,
        required_manpower: definition.required_manpower,
        required_equipment: definition.required_equipment,
        recruitment_days: definition.recruitment_days,
        movement_speed: definition.movement_speed,
        attack: definition.attack,
        defense: definition.defense,
        breakthrough: definition.breakthrough,
        organization: definition.organization,
        morale: definition.morale,
        supply_usage: definition.supply_usage,
        maintenance_cost: definition.maintenance_cost,
    }
}

fn snapshot_war_goal(goal: &WarGoal) -> WarGoalSnapshot {
    WarGoalSnapshot {
        attacker: goal.attacker,
        defender: goal.defender,
        goal_type: goal.goal_type,
        target_states: goal.target_states.clone(),
        base_peace_cost: goal.base_peace_cost,
        international_concern: goal.international_concern,
        completion: goal.completion,
        is_primary: goal.is_primary,
    }
}

fn snapshot_treaty(treaty: &ActiveTreaty) -> ActiveTreatySnapshot {
    ActiveTreatySnapshot {
        treaty_type: treaty.treaty_type,
        countries: treaty.countries,
        signed_date: treaty.signed_date.clone(),
        is_active: treaty.is_active,
    }
}

fn snapshot_activity(activity: &ActiveDiplomaticActivity) -> ActiveDiplomaticActivitySnapshot {
    ActiveDiplomaticActivitySnapshot {
        activity_type: activity.activity_type,
        initiator: activity.initiator,
        target: activity.target,
        days_remaining: activity.days_remaining,
        daily_opinion_change: activity.daily_opinion_change,
    }
}

fn snapshot_resource_deposit(deposit: &StateResourceDeposit) -> StateResourceDepositSnapshot {
    StateResourceDepositSnapshot {
        resource_type: deposit.resource_type,
        base_output: deposit.base_output,
        discovered: deposit.discovered,
        development_level: deposit.development_level,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// capture_snapshot: SetBoundarySnapshot の算出関数
// ─────────────────────────────────────────────────────────────────────────────

/// SetBoundarySnapshot の算出関数
///
/// # capitulation_result の導出について
/// Observer 実行時点での永続化されたゲーム状態 (war.status と war.end_reason) から事後的に導出する。
/// - war.status == AttackerVictory && end_reason == "Defender Capitulation" → DefenderCapitulated
/// - war.status == DefenderVictory && end_reason == "Attacker Capitulation" → AttackerCapitulated
/// - war.status == Active → evaluate_war_capitulation() をリアルタイム評価
/// - それ以外 → None
fn capture_snapshot(
    country_reg: &CountryRegistry,
    war_reg: &WarRegistry,
    frontline_reg: &FrontlineRegistry,
    military_reg: &MilitaryRegistry,
    state_reg: &StateRegistry,
    just_reg: &WarJustificationRegistry,
    military_ai_reg: &MilitaryAiRegistry,
) -> SetBoundarySnapshot {
    let mut ai_armies: Vec<_> = military_reg
        .armies
        .values()
        .filter(|a| a.owner == CountryId(1))
        .collect();
    ai_armies.sort_by_key(|a| a.id.0);

    let occupied = state_reg
        .states
        .iter()
        .filter(|s| s.controller() != s.owner_country_id)
        .count();

    let first_war = war_reg.wars.values().min_by_key(|war| war.id.0);

    let cap_res = first_war.map(|w| {
        if w.status == WarStatus::Active {
            strategy_game::war::capitulation::evaluate_war_capitulation(w, state_reg, military_reg)
        } else if w.status == WarStatus::AttackerVictory
            && w.end_reason.as_deref() == Some("Defender Capitulation")
        {
            CapitulationResult::DefenderCapitulated
        } else if w.status == WarStatus::DefenderVictory
            && w.end_reason.as_deref() == Some("Attacker Capitulation")
        {
            CapitulationResult::AttackerCapitulated
        } else {
            CapitulationResult::None
        }
    });

    let first_just = just_reg
        .justifications
        .values()
        .filter(|j| j.initiator == CountryId(1) && j.target == CountryId(0))
        .min_by_key(|j| j.id);

    let state0 = state_reg.get(StateId(0));

    let active_count = war_reg
        .wars
        .values()
        .filter(|w| w.status == WarStatus::Active)
        .count();

    let ai_state_c1 = military_ai_reg.ai_states.get(&CountryId(1));

    let mut c1_plans: Vec<_> = frontline_reg
        .plans
        .values()
        .filter(|p| p.commanding_country_id == CountryId(1))
        .collect();
    c1_plans.sort_by_key(|p| (p.frontline_id.0, p.commanding_country_id.0));
    let first_c1_plan = c1_plans.first();

    let total_plans_count = frontline_reg.plans.len();

    let mut assigned_ids: Vec<ArmyId> = c1_plans
        .iter()
        .flat_map(|plan| plan.assigned_army_ids.iter().copied())
        .collect();
    assigned_ids.sort_by_key(|id| id.0);
    assigned_ids.dedup();

    let assigned_army_owners: Vec<(ArmyId, Option<CountryId>)> = assigned_ids
        .iter()
        .map(|&army_id| {
            (
                army_id,
                military_reg.armies.get(&army_id).map(|army| army.owner),
            )
        })
        .collect();

    let player_construction_queue = country_reg
        .get(CountryId(0))
        .map(|country| snapshot_construction_queue(&country.construction_queue))
        .unwrap_or_default();
    let player_research_state = country_reg
        .get(CountryId(0))
        .map(|country| snapshot_research_state(&country.research_state));
    let mut player_armies: Vec<ArmyDetailSnapshot> = military_reg
        .armies
        .values()
        .filter(|army| army.owner == CountryId(0))
        .map(snapshot_army)
        .collect();
    player_armies.sort_by_key(|army| army.id.0);

    SetBoundarySnapshot {
        war_record_count: war_reg.wars.len(),
        active_war_count: active_count,
        frontline_count: frontline_reg.frontlines.len(),
        ai_army_destinations: ai_armies.iter().map(|a| a.destination).collect(),
        ai_army_paths: ai_armies.iter().map(|a| a.current_path.clone()).collect(),
        ai_army_positions: ai_armies.iter().map(|a| a.current_state).collect(),
        ai_army_progresses: ai_armies.iter().map(|a| a.movement_progress).collect(),
        occupied_states_count: occupied,
        war_score: first_war.map(|w| w.war_score).unwrap_or(0.0),
        war_status: first_war.map(|w| w.status),
        capitulation_result: cap_res,
        justification_is_ready: first_just.map(|j| j.is_ready),
        winner: first_war.and_then(|w| w.winner),
        territory_owner: state0.map(|s| s.owner_country_id),
        ai_has_evaluated: ai_state_c1.is_some(),
        military_ai_last_eval_day: ai_state_c1.map(|s| s.last_evaluated_day),
        military_ai_dirty: ai_state_c1.map(|s| s.dirty),
        military_ai_decision_reason: ai_state_c1.map(|s| s.last_decision_reason),
        military_ai_stance: first_c1_plan.map(|p| p.stance),
        military_ai_assigned_army_ids: assigned_ids,
        military_ai_assigned_army_owners: assigned_army_owners,
        frontline_plan_count: total_plans_count,
        military_ai_estimated_own_power: ai_state_c1.map(|s| s.estimated_own_power),
        military_ai_estimated_enemy_power: ai_state_c1.map(|s| s.estimated_enemy_power),
        player_construction_queue,
        player_research_state,
        player_armies,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Observer システム関数: 各 Set の直後に登録し SetBoundarySnapshot を保存する
// ─────────────────────────────────────────────────────────────────────────────

pub fn observe_economy_boundary(
    country_reg: Res<CountryRegistry>,
    war_reg: Res<WarRegistry>,
    frontline_reg: Res<FrontlineRegistry>,
    military_reg: Res<MilitaryRegistry>,
    state_reg: Res<StateRegistry>,
    just_reg: Res<WarJustificationRegistry>,
    military_ai_reg: Res<MilitaryAiRegistry>,
    mut observer: ResMut<SetBoundaryObserver>,
) {
    observer.economy_snap = Some(capture_snapshot(
        &country_reg,
        &war_reg,
        &frontline_reg,
        &military_reg,
        &state_reg,
        &just_reg,
        &military_ai_reg,
    ));
}

pub fn observe_research_boundary(
    country_reg: Res<CountryRegistry>,
    war_reg: Res<WarRegistry>,
    frontline_reg: Res<FrontlineRegistry>,
    military_reg: Res<MilitaryRegistry>,
    state_reg: Res<StateRegistry>,
    just_reg: Res<WarJustificationRegistry>,
    military_ai_reg: Res<MilitaryAiRegistry>,
    mut observer: ResMut<SetBoundaryObserver>,
) {
    observer.research_snap = Some(capture_snapshot(
        &country_reg,
        &war_reg,
        &frontline_reg,
        &military_reg,
        &state_reg,
        &just_reg,
        &military_ai_reg,
    ));
}

pub fn observe_diplomacy_boundary(
    country_reg: Res<CountryRegistry>,
    war_reg: Res<WarRegistry>,
    frontline_reg: Res<FrontlineRegistry>,
    military_reg: Res<MilitaryRegistry>,
    state_reg: Res<StateRegistry>,
    just_reg: Res<WarJustificationRegistry>,
    military_ai_reg: Res<MilitaryAiRegistry>,
    mut observer: ResMut<SetBoundaryObserver>,
) {
    observer.diplomacy_snap = Some(capture_snapshot(
        &country_reg,
        &war_reg,
        &frontline_reg,
        &military_reg,
        &state_reg,
        &just_reg,
        &military_ai_reg,
    ));
}

pub fn observe_country_ai_boundary(
    country_reg: Res<CountryRegistry>,
    war_reg: Res<WarRegistry>,
    frontline_reg: Res<FrontlineRegistry>,
    military_reg: Res<MilitaryRegistry>,
    state_reg: Res<StateRegistry>,
    just_reg: Res<WarJustificationRegistry>,
    military_ai_reg: Res<MilitaryAiRegistry>,
    mut observer: ResMut<SetBoundaryObserver>,
) {
    observer.country_ai_snap = Some(capture_snapshot(
        &country_reg,
        &war_reg,
        &frontline_reg,
        &military_reg,
        &state_reg,
        &just_reg,
        &military_ai_reg,
    ));
}

pub fn observe_war_prep_boundary(
    country_reg: Res<CountryRegistry>,
    war_reg: Res<WarRegistry>,
    frontline_reg: Res<FrontlineRegistry>,
    military_reg: Res<MilitaryRegistry>,
    state_reg: Res<StateRegistry>,
    just_reg: Res<WarJustificationRegistry>,
    military_ai_reg: Res<MilitaryAiRegistry>,
    mut observer: ResMut<SetBoundaryObserver>,
) {
    observer.war_prep_snap = Some(capture_snapshot(
        &country_reg,
        &war_reg,
        &frontline_reg,
        &military_reg,
        &state_reg,
        &just_reg,
        &military_ai_reg,
    ));
}

pub fn observe_military_ai_boundary(
    country_reg: Res<CountryRegistry>,
    war_reg: Res<WarRegistry>,
    frontline_reg: Res<FrontlineRegistry>,
    military_reg: Res<MilitaryRegistry>,
    state_reg: Res<StateRegistry>,
    just_reg: Res<WarJustificationRegistry>,
    military_ai_reg: Res<MilitaryAiRegistry>,
    mut observer: ResMut<SetBoundaryObserver>,
) {
    observer.military_ai_snap = Some(capture_snapshot(
        &country_reg,
        &war_reg,
        &frontline_reg,
        &military_reg,
        &state_reg,
        &just_reg,
        &military_ai_reg,
    ));
}

pub fn observe_frontline_orders_boundary(
    country_reg: Res<CountryRegistry>,
    war_reg: Res<WarRegistry>,
    frontline_reg: Res<FrontlineRegistry>,
    military_reg: Res<MilitaryRegistry>,
    state_reg: Res<StateRegistry>,
    just_reg: Res<WarJustificationRegistry>,
    military_ai_reg: Res<MilitaryAiRegistry>,
    mut observer: ResMut<SetBoundaryObserver>,
) {
    observer.frontline_orders_snap = Some(capture_snapshot(
        &country_reg,
        &war_reg,
        &frontline_reg,
        &military_reg,
        &state_reg,
        &just_reg,
        &military_ai_reg,
    ));
}

pub fn observe_military_action_boundary(
    country_reg: Res<CountryRegistry>,
    war_reg: Res<WarRegistry>,
    frontline_reg: Res<FrontlineRegistry>,
    military_reg: Res<MilitaryRegistry>,
    state_reg: Res<StateRegistry>,
    just_reg: Res<WarJustificationRegistry>,
    military_ai_reg: Res<MilitaryAiRegistry>,
    mut observer: ResMut<SetBoundaryObserver>,
) {
    observer.military_action_snap = Some(capture_snapshot(
        &country_reg,
        &war_reg,
        &frontline_reg,
        &military_reg,
        &state_reg,
        &just_reg,
        &military_ai_reg,
    ));
}

pub fn observe_war_resolution_boundary(
    country_reg: Res<CountryRegistry>,
    war_reg: Res<WarRegistry>,
    frontline_reg: Res<FrontlineRegistry>,
    military_reg: Res<MilitaryRegistry>,
    state_reg: Res<StateRegistry>,
    just_reg: Res<WarJustificationRegistry>,
    military_ai_reg: Res<MilitaryAiRegistry>,
    mut observer: ResMut<SetBoundaryObserver>,
) {
    observer.war_resolution_snap = Some(capture_snapshot(
        &country_reg,
        &war_reg,
        &frontline_reg,
        &military_reg,
        &state_reg,
        &just_reg,
        &military_ai_reg,
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// capture_game_state_snapshot: Test B 用の全フィールド・全リソース完全スナップショット
// ─────────────────────────────────────────────────────────────────────────────

/// Test B 用の全リソース全フィールドスナップショット生成関数
///
/// - CountryRegistry: CountryData、建設キュー全7フィールド、研究状態全フィールドを保存
/// - DiplomacyRegistry: 全関係・条約・cooldown・活動を保存
/// - FrontlineRegistry: frontlines 全フィールド・plans 全フィールド・army_frontline_map を保存
/// - WarRegistry: War、war_goals、processed_battle_idsを含む全フィールドを保存
/// - MilitaryRegistry: definitions と ArmyUnit の全フィールドを保存
/// - BattleRegistry: Battle 全フィールドを保存
/// - StateRegistry: StateData 全フィールドを保存
/// - 全 HashMap は対応キーで昇順ソートして決定論的比較を保証
pub fn capture_game_state_snapshot(app: &App) -> GameStateSnapshot {
    let date = app.world().resource::<GameDate>().clone();
    let paused = app.world().resource::<GamePaused>().0;
    let player_country = app.world().resource::<PlayerCountry>().0;

    let mut countries: Vec<CountryDetailSnapshot> = app
        .world()
        .resource::<CountryRegistry>()
        .countries
        .iter()
        .map(snapshot_country)
        .collect();
    countries.sort_by_key(|c| c.id.0);

    let mut diplomatic_relations: Vec<DiplomaticRelationSnapshot> = app
        .world()
        .resource::<DiplomacyRegistry>()
        .relations
        .iter()
        .map(|(&key, relation)| {
            let mut cooldowns: Vec<(CountryId, u32)> = relation
                .cooldowns
                .iter()
                .map(|(&country_id, &days)| (country_id, days))
                .collect();
            cooldowns.sort_by_key(|(country_id, _)| country_id.0);
            DiplomaticRelationSnapshot {
                countries: (key.0, key.1),
                opinion: relation.opinion,
                tension: relation.tension,
                trust: relation.trust,
                threat: relation.threat,
                at_war: relation.at_war,
                has_military_access: relation.has_military_access,
                truce_until: relation.truce_until.clone(),
                treaties: relation.treaties.iter().map(snapshot_treaty).collect(),
                cooldowns,
                active_activity: relation.active_activity.as_ref().map(snapshot_activity),
                last_updated_date: relation.last_updated_date.clone(),
            }
        })
        .collect();
    diplomatic_relations.sort_by_key(|relation| (relation.countries.0.0, relation.countries.1.0));

    let justification_registry = app.world().resource::<WarJustificationRegistry>();
    let justification_next_id = justification_registry.next_id();
    let mut justifications: Vec<JustificationDetailSnapshot> = justification_registry
        .justifications
        .values()
        .map(|j| JustificationDetailSnapshot {
            id: j.id,
            initiator: j.initiator,
            target: j.target,
            target_state: j.target_state,
            start_date: j.start_date.clone(),
            days_passed: j.days_passed,
            required_days: j.required_days,
            is_ready: j.is_ready,
        })
        .collect();
    justifications.sort_by_key(|j| j.id);

    let country_ai_registry = app.world().resource::<CountryAiRegistry>();
    let country_ai_registry_dirty = country_ai_registry.dirty;
    let mut country_ai: Vec<CountryAiDetailSnapshot> = country_ai_registry
        .ai_states
        .values()
        .map(|s| CountryAiDetailSnapshot {
            country_id: s.country_id,
            mode: s.mode,
            decision_reason: s.decision_reason,
            last_daily_evaluation_day: s.last_daily_evaluation_day,
            last_weekly_evaluation_day: s.last_weekly_evaluation_day,
            last_monthly_evaluation_day: s.last_monthly_evaluation_day,
            cooldown_until_day: s.cooldown_until_day,
            dirty: s.dirty,
        })
        .collect();
    country_ai.sort_by_key(|s| s.country_id.0);

    let military_ai_registry = app.world().resource::<MilitaryAiRegistry>();
    let military_ai_registry_dirty = military_ai_registry.dirty;
    let mut military_ai: Vec<MilitaryAiDetailSnapshot> = military_ai_registry
        .ai_states
        .values()
        .map(|s| MilitaryAiDetailSnapshot {
            country_id: s.country_id,
            last_evaluated_day: s.last_evaluated_day,
            last_decision_reason: s.last_decision_reason,
            estimated_own_power: s.estimated_own_power,
            estimated_enemy_power: s.estimated_enemy_power,
            dirty: s.dirty,
        })
        .collect();
    military_ai.sort_by_key(|s| s.country_id.0);

    let military_registry = app.world().resource::<MilitaryRegistry>();
    let military_next_army_id = military_registry.next_army_id();
    let mut division_definitions: Vec<DivisionDefinitionSnapshot> = military_registry
        .definitions
        .values()
        .map(snapshot_division)
        .collect();
    division_definitions.sort_by_key(|definition| definition.id.0);

    let mut armies: Vec<ArmyDetailSnapshot> = military_registry
        .armies
        .values()
        .map(snapshot_army)
        .collect();
    armies.sort_by_key(|a| a.id.0);

    let battle_registry = app.world().resource::<BattleRegistry>();
    let battle_next_id = battle_registry.next_id();
    let mut battles: Vec<BattleDetailSnapshot> = battle_registry
        .battles
        .values()
        .map(|b| BattleDetailSnapshot {
            id: b.id,
            war_id: b.war_id,
            state_id: b.state_id,
            attacker_country: b.attacker_country,
            defender_country: b.defender_country,
            attacker_army_id: b.attacker_army_id,
            defender_army_id: b.defender_army_id,
            start_date: b.start_date.clone(),
            elapsed_days: b.elapsed_days,
            status: b.status,
            attacker_origin_state: b.attacker_origin_state,
        })
        .collect();
    battles.sort_by_key(|b| b.id.0);

    let war_registry = app.world().resource::<WarRegistry>();
    let war_next_id = war_registry.next_id();
    let mut wars: Vec<WarDetailSnapshot> = war_registry
        .wars
        .values()
        .map(|w| {
            let mut attackers: Vec<CountryId> = w.attackers.iter().copied().collect();
            attackers.sort_by_key(|c| c.0);
            let mut defenders: Vec<CountryId> = w.defenders.iter().copied().collect();
            defenders.sort_by_key(|c| c.0);
            let mut occupied_states: Vec<StateId> = w.occupied_states.iter().copied().collect();
            occupied_states.sort_by_key(|s| s.0);
            let mut processed_battle_ids: Vec<BattleId> =
                w.processed_battle_ids.iter().copied().collect();
            processed_battle_ids.sort_by_key(|battle_id| battle_id.0);
            WarDetailSnapshot {
                id: w.id,
                name: w.name.clone(),
                attackers,
                defenders,
                war_goals: w.war_goals.iter().map(snapshot_war_goal).collect(),
                start_date: w.start_date.clone(),
                end_date: w.end_date.clone(),
                duration_days: w.duration_days,
                war_score: w.war_score,
                attacker_war_exhaustion: w.attacker_war_exhaustion,
                defender_war_exhaustion: w.defender_war_exhaustion,
                occupied_states,
                status: w.status,
                winner: w.winner,
                end_reason: w.end_reason.clone(),
                applied_terms: w.applied_terms.clone(),
                won_attacker_battles: w.won_attacker_battles,
                won_defender_battles: w.won_defender_battles,
                processed_battle_ids,
            }
        })
        .collect();
    wars.sort_by_key(|w| w.id.0);

    let frontline_reg = app.world().resource::<FrontlineRegistry>();
    let mut frontlines: Vec<FrontlineDetailSnapshot> = frontline_reg
        .frontlines
        .values()
        .map(|f| {
            let mut attacker_front_regions = f.attacker_front_regions.clone();
            attacker_front_regions.sort_by_key(|s| s.0);
            let mut defender_front_regions = f.defender_front_regions.clone();
            defender_front_regions.sort_by_key(|s| s.0);
            let mut border_region_pairs = f.border_region_pairs.clone();
            border_region_pairs.sort_by_key(|(a, b)| (a.0, b.0));

            let mut plans: Vec<FrontlinePlanSnapshot> = frontline_reg
                .plans
                .values()
                .filter(|p| p.frontline_id == f.frontline_id)
                .map(|p| {
                    let mut assigned = p.assigned_army_ids.clone();
                    assigned.sort_by_key(|id| id.0);
                    FrontlinePlanSnapshot {
                        frontline_id: p.frontline_id,
                        commanding_country_id: p.commanding_country_id,
                        stance: p.stance,
                        objective_region_id: p.objective_region_id,
                        assigned_army_ids: assigned,
                    }
                })
                .collect();
            plans.sort_by_key(|p| (p.frontline_id.0, p.commanding_country_id.0));

            FrontlineDetailSnapshot {
                frontline_id: f.frontline_id,
                war_id: f.war_id,
                attacker_country_id: f.attacker_country_id,
                defender_country_id: f.defender_country_id,
                attacker_front_regions,
                defender_front_regions,
                border_region_pairs,
                plans,
            }
        })
        .collect();
    frontlines.sort_by_key(|f| f.frontline_id.0);

    let mut frontline_plans: Vec<FrontlinePlanSnapshot> = frontline_reg
        .plans
        .values()
        .map(|plan| {
            let mut assigned_army_ids = plan.assigned_army_ids.clone();
            assigned_army_ids.sort_by_key(|army_id| army_id.0);
            FrontlinePlanSnapshot {
                frontline_id: plan.frontline_id,
                commanding_country_id: plan.commanding_country_id,
                stance: plan.stance,
                objective_region_id: plan.objective_region_id,
                assigned_army_ids,
            }
        })
        .collect();
    frontline_plans.sort_by_key(|plan| (plan.frontline_id.0, plan.commanding_country_id.0));
    let next_frontline_id = frontline_reg.next_frontline_id;

    let mut army_frontline_map: Vec<(ArmyId, FrontlineId)> = frontline_reg
        .army_frontline_map
        .iter()
        .map(|(&army_id, &fl_id)| (army_id, fl_id))
        .collect();
    army_frontline_map.sort_by_key(|(aid, _)| aid.0);

    let mut frontline_generated_movements: Vec<ArmyId> = frontline_reg
        .frontline_generated_movements
        .iter()
        .copied()
        .collect();
    frontline_generated_movements.sort_by_key(|army_id| army_id.0);

    let state_registry = app.world().resource::<StateRegistry>();
    let state_index_entries = state_registry.sorted_index_entries();
    let mut states: Vec<StateDetailSnapshot> = state_registry
        .states
        .iter()
        .map(|state| {
            let mut buildings: Vec<(BuildingType, u32)> = state
                .buildings
                .iter()
                .map(|(&building_type, &level)| (building_type, level))
                .collect();
            buildings.sort_by_key(|(building_type, _)| *building_type as u8);
            let mut building_operation_ratios: Vec<(BuildingType, f32)> = state
                .building_operation_ratios
                .iter()
                .map(|(&building_type, &ratio)| (building_type, ratio))
                .collect();
            building_operation_ratios.sort_by_key(|(building_type, _)| *building_type as u8);

            StateDetailSnapshot {
                id: state.id,
                name: state.name.clone(),
                owner_country_id: state.owner_country_id,
                population: state.population,
                workforce_ratio: state.workforce_ratio,
                employed_workforce: state.employed_workforce,
                unemployed_workforce: state.unemployed_workforce,
                education: state.education,
                living_standard: state.living_standard,
                unrest: state.unrest,
                base_logistics_capacity: state.base_logistics_capacity,
                logistics_capacity: state.logistics_capacity,
                logistics_usage: state.logistics_usage,
                logistics_ratio: state.logistics_ratio,
                buildings,
                building_operation_ratios,
                resource_deposits: state
                    .resource_deposits
                    .iter()
                    .map(snapshot_resource_deposit)
                    .collect(),
                world_position: state.world_position,
                size: state.size,
                neighbors: state.neighbors.clone(),
                integration: state.integration,
                controller_country: state.controller_country,
                occupation_progress: state.occupation_progress,
                original_owner: state.original_owner,
                occupation_policy: state.occupation_policy,
                war_id: state.war_id,
                is_sea: state.is_sea,
            }
        })
        .collect();
    states.sort_by_key(|s| s.id.0);

    GameStateSnapshot {
        date,
        paused,
        player_country,
        countries,
        diplomatic_relations,
        justifications,
        justification_next_id,
        country_ai,
        country_ai_registry_dirty,
        military_ai,
        military_ai_registry_dirty,
        division_definitions,
        armies,
        military_next_army_id,
        battles,
        battle_next_id,
        wars,
        war_next_id,
        frontlines,
        frontline_plans,
        next_frontline_id,
        army_frontline_map,
        frontline_generated_movements,
        states,
        state_index_entries,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// setup_test_app: テスト用 App の初期化 + Observer システム登録
// ─────────────────────────────────────────────────────────────────────────────

fn setup_test_app() -> App {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<SetBoundaryObserver>()
        .add_plugins(AppPlugin)
        .add_plugins(CountryPlugin)
        .add_plugins(StatePlugin)
        .add_plugins(BuildingPlugin)
        .add_plugins(EconomyPlugin)
        .add_plugins(ResearchPlugin)
        .add_plugins(PoliticsPlugin)
        .add_plugins(DiplomacyPlugin)
        .add_plugins(WarPlugin)
        .add_plugins(MilitaryPlugin);

    // 観測用 Observer システムを各 Set に登録
    app.add_systems(
        Update,
        (
            observe_economy_boundary
                .in_set(DailySimulationSet::Economy)
                .after(strategy_game::economy::handle_monthly_economy),
            observe_research_boundary
                .in_set(DailySimulationSet::Research)
                .after(strategy_game::research::handle_npc_auto_research),
            observe_diplomacy_boundary
                .in_set(DailySimulationSet::Diplomacy)
                .after(strategy_game::diplomacy::update::handle_daily_diplomacy),
            observe_country_ai_boundary
                .in_set(DailySimulationSet::CountryAi)
                .after(strategy_game::country::country_ai::handle_daily_country_ai),
            observe_war_prep_boundary
                .in_set(DailySimulationSet::WarPreparation)
                .after(strategy_game::war::update::handle_daily_war_prep),
            observe_military_ai_boundary
                .in_set(DailySimulationSet::MilitaryAi)
                .after(strategy_game::war::military_ai::handle_daily_military_ai),
            observe_frontline_orders_boundary
                .in_set(DailySimulationSet::FrontlineOrders)
                .after(strategy_game::war::frontline::handle_daily_frontline_plans),
            observe_military_action_boundary
                .in_set(DailySimulationSet::MilitaryAction)
                .after(strategy_game::military::update::handle_daily_military),
            observe_war_resolution_boundary
                .in_set(DailySimulationSet::WarResolution)
                .after(strategy_game::war::update::handle_daily_war_resolution),
        ),
    );

    app.update();

    app.insert_resource(PlayerCountry(Some(CountryId(0))));
    app.insert_state(GameState::Playing);

    app.update();

    app
}

/// テスト用ヘルパー: add_accumulator で日付を 1 日進め App::update() を実行する
fn advance_day_by_system(app: &mut App) {
    app.world_mut().resource_mut::<GamePaused>().0 = false;
    app.world_mut()
        .resource_mut::<GameDate>()
        .add_accumulator(1.0);
    app.update();
}

fn boundary_army(snapshot: &SetBoundarySnapshot, army_id: ArmyId) -> ArmyDetailSnapshot {
    snapshot
        .player_armies
        .iter()
        .find(|army| army.id == army_id)
        .unwrap_or_else(|| panic!("境界SnapshotにArmyId({:?})が存在すること", army_id))
        .clone()
}

/// 正当化をセットアップし、日次処理直前に「残り1日・未完了」状態にするヘルパー
fn setup_justification(
    app: &mut App,
    initiator: CountryId,
    target: CountryId,
    target_state: StateId,
) {
    let date_str = app.world().resource::<GameDate>().display();
    let mut system_state = SystemState::<(
        Res<CountryRegistry>,
        Res<StateRegistry>,
        Res<DiplomacyRegistry>,
        ResMut<WarJustificationRegistry>,
    )>::new(app.world_mut());

    let Ok((country_reg, state_reg, diplomacy_reg, mut just_reg)) =
        system_state.get_mut(app.world_mut())
    else {
        panic!("SystemState get_mut failed")
    };

    just_reg
        .start_justification(
            initiator,
            target,
            target_state,
            date_str,
            &country_reg,
            &state_reg,
            &diplomacy_reg,
        )
        .unwrap();

    if let Some(j) = just_reg
        .justifications
        .values_mut()
        .find(|j| j.initiator == initiator)
    {
        j.is_ready = false;
        j.days_passed = j.required_days.saturating_sub(1);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test A: 日付発行から経済まで
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_a_daily_processing_order() {
    let mut app = setup_test_app();

    let initial_date = app.world().resource::<GameDate>().clone();

    advance_day_by_system(&mut app);

    let updated_date = app.world().resource::<GameDate>().clone();
    assert_ne!(initial_date, updated_date, "システムにより日付が進むこと");
    assert_eq!(
        updated_date.day,
        initial_date.day + 1,
        "日付が1日分増加していること"
    );

    let c0 = app
        .world()
        .resource::<CountryRegistry>()
        .get(CountryId(0))
        .unwrap();
    assert!(c0.treasury > 0.0, "プレイヤー国の国庫が正の値であること");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test B: ゲーム一時停止時の全リソース・全フィールド完全不変性アサート
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_b_game_paused_no_state_change() {
    let mut app = setup_test_app();

    app.insert_resource(GamePaused(true));

    let snapshot_before = capture_game_state_snapshot(&app);

    for _ in 0..5 {
        app.update();
    }

    let snapshot_after = capture_game_state_snapshot(&app);

    // (1) GameDate: year, month, day が完全不変
    assert_eq!(
        snapshot_before.date, snapshot_after.date,
        "1. GameDate (year/month/day) が完全不変であること"
    );
    assert_eq!(
        snapshot_before.paused, snapshot_after.paused,
        "GamePaused が不変"
    );
    assert_eq!(
        snapshot_before.player_country, snapshot_after.player_country,
        "PlayerCountry が不変"
    );
    // (2) CountryRegistry: treasury/tax_rate/建設キュー全6フィールド/研究全フィールド/manpower/income
    assert_eq!(
        snapshot_before.countries, snapshot_after.countries,
        "2. CountryRegistry (CountryData全フィールド、建設キュー全7フィールド、研究状態全フィールドを含む) が完全不変であること"
    );
    assert_eq!(
        snapshot_before.diplomatic_relations, snapshot_after.diplomatic_relations,
        "2b. DiplomacyRegistry の全関係・条約・cooldown・活動が完全不変であること"
    );
    // (3) WarJustificationRegistry
    assert_eq!(
        snapshot_before.justifications, snapshot_after.justifications,
        "3. WarJustificationRegistry が完全不変であること"
    );
    // (4) CountryAiRegistry
    assert_eq!(
        snapshot_before.country_ai, snapshot_after.country_ai,
        "4. CountryAiRegistry が完全不変であること"
    );
    assert_eq!(
        snapshot_before.country_ai_registry_dirty, snapshot_after.country_ai_registry_dirty,
        "4b. CountryAiRegistry.dirty が完全不変であること"
    );
    // (5) MilitaryAiRegistry
    assert_eq!(
        snapshot_before.military_ai, snapshot_after.military_ai,
        "5. MilitaryAiRegistry (last_evaluated_day/decision_reason/power/dirty) が完全不変であること"
    );
    assert_eq!(
        snapshot_before.military_ai_registry_dirty, snapshot_after.military_ai_registry_dirty,
        "5b. MilitaryAiRegistry.dirty が完全不変であること"
    );
    assert_eq!(
        snapshot_before.division_definitions, snapshot_after.division_definitions,
        "5c. MilitaryRegistry.definitions の全フィールドが完全不変であること"
    );
    // (6) MilitaryRegistry: 全ArmyUnit 全フィールド
    assert_eq!(
        snapshot_before.armies, snapshot_after.armies,
        "6. MilitaryRegistry (全ArmyUnit全フィールド) が完全不変であること"
    );
    // (7) BattleRegistry: 全Battle 全フィールド
    assert_eq!(
        snapshot_before.battles, snapshot_after.battles,
        "7. BattleRegistry (全Battle全フィールド) が完全不変であること"
    );
    // (8) WarRegistry: 全War 全関連フィールド
    assert_eq!(
        snapshot_before.wars, snapshot_after.wars,
        "8. WarRegistry (全War全関連フィールド) が完全不変であること"
    );
    // (9) FrontlineRegistry: frontlines/plans全フィールド
    assert_eq!(
        snapshot_before.frontlines, snapshot_after.frontlines,
        "9. FrontlineRegistry (frontlines/plans全フィールド) が完全不変であること"
    );
    assert_eq!(
        snapshot_before.frontline_plans, snapshot_after.frontline_plans,
        "9a. FrontlineRegistry.plans の全要素・全フィールドが完全不変であること"
    );
    assert_eq!(
        snapshot_before.next_frontline_id, snapshot_after.next_frontline_id,
        "9aa. FrontlineRegistry.next_frontline_id が完全不変であること"
    );
    // (9b) FrontlineRegistry.army_frontline_map
    assert_eq!(
        snapshot_before.army_frontline_map, snapshot_after.army_frontline_map,
        "9b. FrontlineRegistry.army_frontline_map が完全不変であること"
    );
    assert_eq!(
        snapshot_before.frontline_generated_movements, snapshot_after.frontline_generated_movements,
        "9c. FrontlineRegistry.frontline_generated_movements が完全不変であること"
    );
    // (10) StateRegistry
    assert_eq!(
        snapshot_before.states, snapshot_after.states,
        "10. StateRegistry.states の全要素・全フィールドが完全不変であること"
    );
    assert_eq!(
        (
            snapshot_before.justification_next_id,
            snapshot_before.military_next_army_id,
            snapshot_before.battle_next_id,
            snapshot_before.war_next_id,
            &snapshot_before.state_index_entries,
        ),
        (
            snapshot_after.justification_next_id,
            snapshot_after.military_next_army_id,
            snapshot_after.battle_next_id,
            snapshot_after.war_next_id,
            &snapshot_after.state_index_entries,
        ),
        "10b. Registry内部の次IDカウンタとState索引キャッシュが完全不変であること"
    );
    assert_eq!(
        snapshot_before, snapshot_after,
        "11. GameStateSnapshot 全体が完全不変であること"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test C: 本番国家AI経路による同日正当化完了・宣戦布告・同日前線生成・同日移動ゼロ
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_c_war_declaration_and_frontline_generation() {
    let mut app = setup_test_app();

    setup_justification(&mut app, CountryId(1), CountryId(0), StateId(0));

    {
        let just_reg = app.world().resource::<WarJustificationRegistry>();
        let j = just_reg
            .justifications
            .values()
            .find(|j| j.initiator == CountryId(1))
            .unwrap();
        assert!(!j.is_ready, "Diplomacy実行前はis_ready == falseであること");
    }

    advance_day_by_system(&mut app);

    let observer = app.world().resource::<SetBoundaryObserver>().clone();
    let diplomacy_snap = observer
        .diplomacy_snap
        .expect("Diplomacy Observer が実行されていること");
    let country_ai_snap = observer
        .country_ai_snap
        .expect("CountryAi Observer が実行されていること");
    let war_prep_snap = observer
        .war_prep_snap
        .expect("WarPrep Observer が実行されていること");
    let orders_snap = observer
        .frontline_orders_snap
        .expect("FrontlineOrders Observer が実行されていること");
    let action_snap = observer
        .military_action_snap
        .expect("MilitaryAction Observer が実行されていること");

    // 1. Diplomacy直後: is_ready == true へ同日遷移
    assert_eq!(
        diplomacy_snap.justification_is_ready,
        Some(true),
        "Diplomacy直後にis_ready == trueへ同日遷移すること"
    );

    // 2. CountryAi直後: 本番AI経由で宣戦布告が実行され戦争が1件生成
    assert_eq!(
        country_ai_snap.active_war_count, 1,
        "同日のCountryAiで宣戦布告が実行されること (active_war_count == 1)"
    );
    assert_eq!(
        country_ai_snap.war_record_count, 1,
        "同日のCountryAiで宣戦布告が実行されること (war_record_count == 1)"
    );

    // 3. WarPreparation直後: 前線が同日中に1件以上生成
    assert!(
        war_prep_snap.frontline_count >= 1,
        "WarPreparationで同日前線が生成されること (frontline_count >= 1)"
    );

    // 4. MilitaryAiRegistry に AI 国家が登録されていること
    let mil_ai = app.world().resource::<MilitaryAiRegistry>();
    assert!(
        mil_ai.ai_states.contains_key(&CountryId(1)),
        "AI国家 CountryId(1) が MilitaryAiRegistry に登録されていること"
    );

    // 5. 宣戦布告当日のFrontlineOrders直後: 移動命令なし
    for dest in &orders_snap.ai_army_destinations {
        assert_eq!(*dest, None, "宣戦布告当日はdestinationがNoneであること");
    }
    for path in &orders_snap.ai_army_paths {
        assert!(path.is_empty(), "宣戦布告当日はcurrent_pathが空であること");
    }

    // 6. 宣戦布告当日のMilitaryAction直後: 部隊位置・進捗が変わらない
    assert_eq!(
        orders_snap.ai_army_positions, action_snap.ai_army_positions,
        "宣戦布告当日は部隊位置が不変であること"
    );
    assert_eq!(
        orders_snap.ai_army_progresses, action_snap.ai_army_progresses,
        "宣戦布告当日は移動進捗が不変であること"
    );

    // 7. 2回目のupdate でも戦争数・前線数が重複しない
    advance_day_by_system(&mut app);
    assert_eq!(
        app.world().resource::<WarRegistry>().wars.len(),
        1,
        "2日目も戦争レコードが1件のみであること"
    );
    assert_eq!(
        app.world().resource::<FrontlineRegistry>().frontlines.len(),
        war_prep_snap.frontline_count,
        "2日目も前線数が変わらないこと"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test D: 翌日の軍事行動 (MilitaryAi境界での事前・事後比較・実在部隊検証)
//
// MilitaryAi境界の Observer で以下を事前保存し、MilitaryAi 実行後に境界観測で確認する:
//   - last_evaluated_day (> 実行前)
//   - dirty (== false)
//   - decision_reason (== SufficientAdvantage)
//   - stance (== Offensive)
//   - assigned_army_ids (実行前から更新・全割当IDがCountryId(1)所有の実在部隊)
//
// FrontlineOrders や MilitaryAction による後続状態とは明確に分離する。
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_d_next_day_military_action() {
    let mut app = setup_test_app();

    setup_justification(&mut app, CountryId(1), CountryId(0), StateId(0));

    // AI 陸軍の兵力を 50000 に増強し SufficientAdvantage (130%以上の優勢) を確定
    {
        let mut military_registry = app.world_mut().resource_mut::<MilitaryRegistry>();
        if let Some(army) = military_registry
            .armies
            .values_mut()
            .find(|a| a.owner == CountryId(1))
        {
            army.manpower = 50000;
            army.max_manpower = 50000;
        }
    }

    // 日N: 宣戦布告・前線生成
    advance_day_by_system(&mut app);

    // 日N+1: アップデート実行
    advance_day_by_system(&mut app);

    // MilitaryAi 境界の Observer データを取得 (後続の FrontlineOrders・MilitaryAction とは別)
    let observer = app.world().resource::<SetBoundaryObserver>().clone();
    let before_military_ai_snap = observer
        .war_prep_snap
        .expect("WarPreparation Observer がMilitaryAi直前に実行されていること");
    let mil_ai_snap = observer
        .military_ai_snap
        .expect("MilitaryAi Observer が実行されていること");
    let orders_snap = observer
        .frontline_orders_snap
        .expect("FrontlineOrders Observer が実行されていること");
    let action_snap = observer
        .military_action_snap
        .expect("MilitaryAction Observer が実行されていること");

    // ─── MilitaryAi 境界での事前・事後アサート ───────────────────────────────

    let eval_day_before = before_military_ai_snap
        .military_ai_last_eval_day
        .expect("MilitaryAi直前: CountryId(1)のAI状態が存在すること");
    let dirty_before = before_military_ai_snap.military_ai_dirty;
    let decision_reason_before = before_military_ai_snap.military_ai_decision_reason;
    let stance_before = before_military_ai_snap.military_ai_stance;
    let assigned_ids_before = before_military_ai_snap
        .military_ai_assigned_army_ids
        .clone();

    // (1) last_evaluated_day > eval_day_before (MilitaryAi 境界での厳密な実行証明)
    let last_eval = mil_ai_snap.military_ai_last_eval_day.unwrap_or(0);
    assert!(
        last_eval > eval_day_before,
        "MilitaryAi直後のlast_evaluated_day ({}) > 実行前 ({})",
        last_eval,
        eval_day_before
    );

    // (2) dirty == false (MilitaryAi 境界での評価完了証明)
    assert_eq!(
        mil_ai_snap.military_ai_dirty,
        Some(false),
        "MilitaryAi直後はdirty == false であること"
    );
    assert_ne!(
        dirty_before, mil_ai_snap.military_ai_dirty,
        "MilitaryAi境界でdirtyが実行前から更新されること"
    );

    // (3) decision_reason == SufficientAdvantage (130%以上優勢で攻勢選択)
    assert_eq!(
        mil_ai_snap.military_ai_decision_reason,
        Some(MilitaryAiDecisionReason::SufficientAdvantage),
        "MilitaryAi直後のdecision_reason == SufficientAdvantage であること"
    );
    assert_ne!(
        decision_reason_before, mil_ai_snap.military_ai_decision_reason,
        "MilitaryAi境界でdecision_reasonが実行前から更新されること"
    );

    // (4) stance == Offensive (攻勢姿勢が確定)
    assert_eq!(
        mil_ai_snap.military_ai_stance,
        Some(FrontlineStance::Offensive),
        "MilitaryAi直後のstance == Offensive であること"
    );
    assert_ne!(
        stance_before, mil_ai_snap.military_ai_stance,
        "MilitaryAi境界でstanceが実行前から更新されること"
    );

    // (5) assigned_army_ids が実行前から更新されていること
    let assigned_ids_after = mil_ai_snap.military_ai_assigned_army_ids.clone();
    assert!(
        !assigned_ids_after.is_empty(),
        "MilitaryAi直後のassigned_army_idsが空ではないこと"
    );
    assert_ne!(
        assigned_ids_after, assigned_ids_before,
        "MilitaryAi直後のassigned_army_ids ({:?}) が実行前 ({:?}) から更新されていること",
        assigned_ids_after, assigned_ids_before
    );

    // (6) MilitaryAi Observer が同じ境界で記録した所有国により、全IDの実在・所有を検証
    let assigned_owners = &mil_ai_snap.military_ai_assigned_army_owners;
    assert_eq!(
        assigned_owners.len(),
        assigned_ids_after.len(),
        "MilitaryAi直後: 全assigned_army_idsに所有国観測値があること"
    );
    for (&army_id, &(observed_id, owner)) in assigned_ids_after.iter().zip(assigned_owners) {
        assert_eq!(
            observed_id, army_id,
            "MilitaryAi直後: assigned_army_idsと所有国観測のIDが一致すること"
        );
        assert_eq!(
            owner,
            Some(CountryId(1)),
            "MilitaryAi直後: ArmyId({:?})がCountryId(1)所有の実在部隊であること",
            army_id
        );
    }

    // MilitaryAi 直後 (WarResolution 前) には戦争状態・戦勝点が変化しないこと
    assert_eq!(
        mil_ai_snap.occupied_states_count, 0,
        "MilitaryAi直後は占領数が 0 であること"
    );
    assert_eq!(
        mil_ai_snap.war_score, 0.0,
        "MilitaryAi直後はwar_score が 0.0 であること"
    );
    assert_eq!(
        mil_ai_snap.war_status,
        Some(WarStatus::Active),
        "MilitaryAi直後はwar_status == Active であること"
    );

    // ─── FrontlineOrders 境界 (MilitaryAi の後続) ────────────────────────────
    // FrontlineOrders直後: AI由来の移動命令が生成されていること (destination or path)
    let has_dest = orders_snap.ai_army_destinations.iter().any(|d| d.is_some());
    let has_path = orders_snap.ai_army_paths.iter().any(|p| !p.is_empty());
    assert!(
        has_dest || has_path,
        "N+1日のFrontlineOrders直後にAI移動命令が生成されていること"
    );

    // FrontlineOrders直後はまだ MilitaryAction 前なのでwar_scoreが変わらないこと
    assert_eq!(
        orders_snap.war_status,
        Some(WarStatus::Active),
        "FrontlineOrders直後はwar_status == Active であること"
    );
    assert_eq!(
        orders_snap.war_score, 0.0,
        "FrontlineOrders直後はwar_score == 0.0 であること"
    );

    // ─── MilitaryAction 境界 (FrontlineOrders の後続) ─────────────────────────
    // MilitaryAction直後: 移動処理が行われたこと (進捗増加または位置変化)
    let mut is_movement_processed = false;
    for i in 0..orders_snap.ai_army_progresses.len() {
        let p_before = orders_snap.ai_army_progresses[i];
        let p_after = action_snap.ai_army_progresses[i];
        let pos_before = orders_snap.ai_army_positions[i];
        let pos_after = action_snap.ai_army_positions[i];
        if p_after > p_before || pos_after != pos_before {
            is_movement_processed = true;
            break;
        }
    }
    assert!(
        is_movement_processed,
        "N+1日のMilitaryAction直後に移動処理(進捗増加または位置変化)が行われること"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test E: プレイヤー保護とAI評価の境界別分離検証
//
// プレイヤーには明示的に以下を設定する:
//   - 空でない建設キュー (Farm Lv.1 建設中, state_id=0)
//   - 空でない研究対象 (Science 分野の仮技術, progress=10.0, cost=100.0)
//   - 手動移動経路 ([StateId(1)], destination=StateId(1), current_state=StateId(0))
//
// 境界ごとに以下を個別に分離・検証する:
//   - Economy直後: 建設progressが0.0から1.0へ正規進捗したこと
//   - Research直後: 占領数・戦争数が 0 であること
//   - MilitaryAi直後: Economy直後の建設キュー全内容とResearch直後の研究全内容が不変
//   - FrontlineOrders直後: AI命令でプレイヤー陣営が上書きされていないこと
//   - MilitaryAction直後: StateId(1)への経路、位置、進捗0.2を具体値で検証
//
// `movement_progress >= before` は禁止。MilitaryAction直後の具体的な進捗または位置を検証する。
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_e_player_protection_and_ai_evaluation() {
    let mut app = setup_test_app();

    // プレイヤー陸軍 ID を取得し、手動移動経路を設定する
    let player_army_id: ArmyId = {
        let mut military_registry = app.world_mut().resource_mut::<MilitaryRegistry>();
        let army_id = military_registry
            .armies
            .values()
            .find(|a| a.owner == CountryId(0))
            .map(|a| a.id)
            .expect("CountryId(0) 所有の陸軍が存在すること");
        if let Some(army) = military_registry.armies.get_mut(&army_id) {
            army.current_state = StateId(0);
            army.destination = Some(StateId(1));
            army.current_path = vec![StateId(1)];
            army.target_state = None;
            army.movement_progress = 0.0;
            army.status = ArmyStatus::Moving;
        }
        army_id
    };

    // プレイヤー国に建設キュー (Farm, state_id=0, level=1) を明示的にセット
    {
        let mut country_registry = app.world_mut().resource_mut::<CountryRegistry>();
        let player_country = country_registry
            .get_mut(CountryId(0))
            .expect("CountryId(0)が存在すること");
        player_country.construction_queue.clear();
        player_country
            .construction_queue
            .push(ConstructionQueueItem {
                state_id: StateId(0),
                building_type: BuildingType::Farm,
                target_level: 1,
                progress: 0.0,
                required_progress: 100.0,
                paid_cost: 50.0,
                status: ConstructionStatus::InProgress,
            });
    }

    // プレイヤー国に研究対象 (Science 分野, tech_id="agri_basic", progress=10.0, cost=100.0) をセット
    {
        let mut country_registry = app.world_mut().resource_mut::<CountryRegistry>();
        let player_country = country_registry
            .get_mut(CountryId(0))
            .expect("CountryId(0)が存在すること");
        player_country.research_state.in_progress.clear();
        player_country.research_state.in_progress.insert(
            TechnologyField::Science,
            InProgressTech {
                tech_id: "agri_basic".to_string(),
                progress: 10.0,
                cost: 100.0,
            },
        );
    }

    let eval_day_before: u32 = app
        .world()
        .resource::<MilitaryAiRegistry>()
        .ai_states
        .get(&CountryId(1))
        .map(|s| s.last_evaluated_day)
        .unwrap_or(0);

    // ─── システムを1日進める ─────────────────────────────────────────────────
    advance_day_by_system(&mut app);

    // ─── 各境界のObserver データ取得 ─────────────────────────────────────────
    let observer = app.world().resource::<SetBoundaryObserver>().clone();
    let economy_snap = observer
        .economy_snap
        .expect("Economy Observer が実行されていること");
    let research_snap = observer
        .research_snap
        .expect("Research Observer が実行されていること");
    let mil_ai_snap = observer
        .military_ai_snap
        .expect("MilitaryAi Observer が実行されていること");
    let orders_snap = observer
        .frontline_orders_snap
        .expect("FrontlineOrders Observer が実行されていること");
    let action_snap = observer
        .military_action_snap
        .expect("MilitaryAction Observer が実行されていること");

    // ─── Economy 直後の検証 ──────────────────────────────────────────────────
    let expected_construction_after_economy = vec![ConstructionQueueItemSnapshot {
        state_id: StateId(0),
        building_type: BuildingType::Farm,
        target_level: 1,
        progress: 1.0,
        required_progress: 100.0,
        paid_cost: 50.0,
        status: ConstructionStatus::InProgress,
    }];
    assert_eq!(
        economy_snap.player_construction_queue, expected_construction_after_economy,
        "Economy直後: 建設キュー全要素・全7フィールドが期待値であること"
    );

    // ─── Research 直後の検証 ─────────────────────────────────────────────────
    assert_eq!(
        research_snap.occupied_states_count, 0,
        "Research直後は占領数が 0 であること"
    );
    assert_eq!(
        research_snap.active_war_count, 0,
        "Research直後は戦争が 0 件であること"
    );
    let expected_research_state = ResearchStateSnapshot {
        completed_technologies: Vec::new(),
        in_progress: vec![InProgressTechSnapshot {
            field: TechnologyField::Science,
            tech_id: "agri_basic".to_string(),
            progress: 10.0,
            cost: 100.0,
        }],
        allocation: [0.4, 0.4, 0.2, 0.0],
        preferred_fields: Vec::new(),
    };
    assert_eq!(
        research_snap.player_research_state,
        Some(expected_research_state.clone()),
        "Research直後: 研究対象・進捗・コストを含む研究状態全フィールドが期待値であること"
    );

    // ─── MilitaryAi 直後の検証 ───────────────────────────────────────────────
    assert_eq!(
        mil_ai_snap.player_construction_queue, economy_snap.player_construction_queue,
        "Economy直後とMilitaryAi直後: 建設キューのprogressを含む全要素・全フィールドが完全不変であること"
    );
    assert_eq!(
        mil_ai_snap.player_research_state, research_snap.player_research_state,
        "Research直後とMilitaryAi直後: 研究対象・進捗・コストを含む全研究状態が完全不変であること"
    );

    // AI国家のMilitaryAi評価が実行されたこと (eval_day_before より増加)
    let eval_after = mil_ai_snap.military_ai_last_eval_day.unwrap_or(0);
    assert!(
        eval_after > eval_day_before,
        "MilitaryAi直後のlast_evaluated_day ({}) > eval_day_before ({})",
        eval_after,
        eval_day_before
    );
    assert_eq!(
        mil_ai_snap.military_ai_dirty,
        Some(false),
        "MilitaryAi直後はdirty == false であること"
    );
    // Test E では戦争がないため decision_reason == NoActiveWar
    assert_eq!(
        mil_ai_snap.military_ai_decision_reason,
        Some(MilitaryAiDecisionReason::NoActiveWar),
        "Test Eでは戦争なし: decision_reason == NoActiveWar であること"
    );
    assert_eq!(
        mil_ai_snap.military_ai_stance, None,
        "Test Eでは戦争なし: stanceが存在しないこと"
    );
    assert!(
        mil_ai_snap.military_ai_assigned_army_ids.is_empty(),
        "Test Eでは戦争なし: assigned_army_idsが空であること"
    );
    assert!(
        mil_ai_snap.military_ai_assigned_army_owners.is_empty(),
        "Test Eでは戦争なし: assigned_army_idsの所有国観測も空であること"
    );

    let research_player_army = boundary_army(&research_snap, player_army_id);
    let military_ai_player_army = boundary_army(&mil_ai_snap, player_army_id);
    assert_eq!(
        military_ai_player_army, research_player_army,
        "Research直後とMilitaryAi直後: プレイヤー手動移動命令を含むArmy全フィールドが完全不変であること"
    );
    assert_eq!(military_ai_player_army.current_state, StateId(0));
    assert_eq!(military_ai_player_army.destination, Some(StateId(1)));
    assert_eq!(military_ai_player_army.current_path, vec![StateId(1)]);
    assert_eq!(military_ai_player_army.target_state, None);
    assert_eq!(military_ai_player_army.movement_progress, 0.0);
    assert_eq!(military_ai_player_army.status, ArmyStatus::Moving);

    // プレイヤーは MilitaryAiRegistry に登録されていないこと
    let mil_ai_reg = app.world().resource::<MilitaryAiRegistry>();
    assert!(
        !mil_ai_reg.ai_states.contains_key(&CountryId(0)),
        "MilitaryAiRegistry に CountryId(0) が登録されていないこと"
    );

    // ─── FrontlineOrders 直後の検証 ──────────────────────────────────────────
    let orders_player_army = boundary_army(&orders_snap, player_army_id);
    assert_eq!(
        orders_player_army, military_ai_player_army,
        "MilitaryAi直後とFrontlineOrders直後: プレイヤー手動移動命令を含むArmy全フィールドが完全不変であること"
    );
    assert_eq!(orders_player_army.current_state, StateId(0));
    assert_eq!(orders_player_army.destination, Some(StateId(1)));
    assert_eq!(orders_player_army.current_path, vec![StateId(1)]);
    assert_eq!(orders_player_army.target_state, None);
    assert_eq!(orders_player_army.movement_progress, 0.0);
    assert_eq!(orders_player_army.status, ArmyStatus::Moving);

    // ─── MilitaryAction 直後の検証 ──────────────────────────────────────────
    let action_player_army = boundary_army(&action_snap, player_army_id);
    assert_eq!(
        action_player_army.current_state,
        StateId(0),
        "MilitaryAction直後: 1日ではStateId(1)へ未到着で位置はStateId(0)"
    );
    assert_eq!(action_player_army.destination, Some(StateId(1)));
    assert_eq!(
        action_player_army.current_path,
        Vec::<StateId>::new(),
        "MilitaryAction直後: StateId(1)がcurrent_pathからtarget_stateへ移されたこと"
    );
    assert_eq!(action_player_army.target_state, Some(StateId(1)));
    assert_eq!(
        action_player_army.movement_progress, 0.2,
        "MilitaryAction直後: base_days=5、step_cost=1、speed=1、supply=1なので進捗は正確に0.2"
    );
    assert_eq!(action_player_army.status, ArmyStatus::Moving);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test F: 本番移動による占領・同日戦勝点計算・降伏判定・自動講和・戦後整理 (全文)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_f_same_day_battle_result_reflection() {
    let mut app = setup_test_app();

    // 正当化セットアップ: Britannia (CountryId(1)) vs Arcadia (CountryId(0)), 目標州 StateId(0)
    setup_justification(&mut app, CountryId(1), CountryId(0), StateId(0));

    // 1日目: 宣戦布告・前線自動生成
    advance_day_by_system(&mut app);

    // 2日目直前のセットアップ:
    // AI 陸軍 (CountryId(1)) を StateId(3) に配置し、StateId(0) への到達直前 (progress=0.99) に設定
    // Arcadia (CountryId(0)) の防衛陸軍を全滅させ降伏条件2 (防御側に戦闘可能な軍なし) を満たす
    // テスト専用の occupy_state は呼ばず、MilitaryAction による本番移動フローを通す
    {
        let mut mil_reg = app.world_mut().resource_mut::<MilitaryRegistry>();
        if let Some(army) = mil_reg
            .armies
            .values_mut()
            .find(|a| a.owner == CountryId(1))
        {
            army.current_state = StateId(3);
            army.destination = Some(StateId(0));
            army.target_state = Some(StateId(0));
            army.current_path = Vec::new();
            army.movement_progress = 0.99;
            army.status = ArmyStatus::Moving;
        }
        for army in mil_reg
            .armies
            .values_mut()
            .filter(|a| a.owner == CountryId(0))
        {
            army.manpower = 0;
            army.status = ArmyStatus::Destroyed;
        }
    }

    // 2日目 update 前の状態を計測
    let occupied_states_count_before: usize = app
        .world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .filter(|s| s.controller() != s.owner_country_id)
        .count();
    let war_score_before: f32 = app
        .world()
        .resource::<WarRegistry>()
        .wars
        .values()
        .next()
        .map(|w| w.war_score)
        .unwrap_or(0.0);
    let war_status_before: Option<WarStatus> = app
        .world()
        .resource::<WarRegistry>()
        .wars
        .values()
        .next()
        .map(|w| w.status);

    assert_eq!(
        occupied_states_count_before, 0,
        "2日目 update 前は占領数 0 であること"
    );
    assert_eq!(
        war_score_before, 0.0,
        "2日目 update 前はwar_score 0.0 であること"
    );
    assert_eq!(
        war_status_before,
        Some(WarStatus::Active),
        "2日目 update 前はwar_status == Active であること"
    );

    // 2日目: advance_day_by_system を実行
    // 処理順: MilitaryAction (占領) → WarResolution (戦勝点・降伏・講和・整理)
    advance_day_by_system(&mut app);

    // Observer データを取得
    let observer = app.world().resource::<SetBoundaryObserver>().clone();
    let action_snap = observer
        .military_action_snap
        .expect("MilitaryAction Observer が実行されていること");
    let resolution_snap = observer
        .war_resolution_snap
        .expect("WarResolution Observer が実行されていること");

    // ─── MilitaryAction 直後の検証 ──────────────────────────────────────────
    // (WarResolution 前) 部隊が StateId(0) へ到着し占領数が増加すること
    assert!(
        action_snap.occupied_states_count > occupied_states_count_before,
        "MilitaryAction直後: 占領数が増加していること ({} → {})",
        occupied_states_count_before,
        action_snap.occupied_states_count
    );
    // MilitaryAction 直後は WarResolution 前のため war_score は不変
    assert_eq!(
        action_snap.war_score, war_score_before,
        "MilitaryAction直後: WarResolution前のためwar_score == 0.0 であること"
    );
    assert_eq!(
        action_snap.war_status,
        Some(WarStatus::Active),
        "MilitaryAction直後: war_status == Active であること"
    );
    assert_eq!(
        action_snap.active_war_count, 1,
        "MilitaryAction直後: active_war_count == 1 であること"
    );

    // ─── WarResolution 直後の 11 個の個別アサート ──────────────────────────

    // (1) capitulation_result == Some(DefenderCapitulated) (事後導出値)
    // capture_snapshot 内で war.status と war.end_reason から以下の規則で導出:
    // - AttackerVictory + "Defender Capitulation" → DefenderCapitulated
    assert_eq!(
        resolution_snap.capitulation_result,
        Some(CapitulationResult::DefenderCapitulated),
        "WarResolution直後: capitulation_result == DefenderCapitulated であること"
    );

    // (2) end_reason == Some("Defender Capitulation") (本番データ直接検証)
    let war_reg = app.world().resource::<WarRegistry>();
    let ended_war = war_reg
        .wars
        .values()
        .next()
        .expect("終戦レコードが存在すること");
    let mut ended_attackers: Vec<CountryId> = ended_war.attackers.iter().copied().collect();
    ended_attackers.sort_by_key(|country_id| country_id.0);
    let mut ended_defenders: Vec<CountryId> = ended_war.defenders.iter().copied().collect();
    ended_defenders.sort_by_key(|country_id| country_id.0);
    assert_eq!(ended_attackers, vec![CountryId(1)]);
    assert_eq!(ended_defenders, vec![CountryId(0)]);
    assert_eq!(ended_war.status, WarStatus::AttackerVictory);
    assert_eq!(ended_war.winner, Some(CountryId(1)));
    assert_eq!(
        ended_war.end_reason.as_deref(),
        Some("Defender Capitulation"),
        "WarResolution直後: end_reason == \"Defender Capitulation\" であること"
    );
    assert_eq!(
        ended_war.end_date.as_deref(),
        Some("1800/01/03"),
        "WarResolution直後: end_dateが同日の日付で保存されること"
    );

    // (3) war_status == Some(AttackerVictory)
    assert_eq!(
        resolution_snap.war_status,
        Some(WarStatus::AttackerVictory),
        "WarResolution直後: war_status == AttackerVictory であること"
    );

    // (4) winner == Some(CountryId(1))
    assert_eq!(
        resolution_snap.winner,
        Some(CountryId(1)),
        "WarResolution直後: winner == CountryId(1) (Britannia) であること"
    );

    // (5) territory_owner == Some(CountryId(1))
    assert_eq!(
        resolution_snap.territory_owner,
        Some(CountryId(1)),
        "WarResolution直後: StateId(0) の所有権が CountryId(1) (Britannia) へ割譲されること"
    );
    let transferred_state = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(0))
        .expect("StateId(0)が存在すること");
    assert_eq!(transferred_state.owner_country_id, CountryId(1));
    assert_eq!(transferred_state.controller_country, Some(CountryId(1)));
    assert_eq!(transferred_state.original_owner, None);
    assert_eq!(transferred_state.occupation_progress, 0.0);
    assert_eq!(transferred_state.war_id, None);

    // (6) active_war_count == 0
    assert_eq!(
        resolution_snap.active_war_count, 0,
        "WarResolution直後: Active な戦争は 0 件であること"
    );

    // (7) war_record_count == 1
    assert_eq!(
        resolution_snap.war_record_count, 1,
        "WarResolution直後: 終戦済みの戦争レコードが履歴として 1 件保持されること"
    );

    // (8) frontline_count == 0
    assert_eq!(
        resolution_snap.frontline_count, 0,
        "WarResolution直後: frontline_count == 0 であること"
    );

    // (9) frontline_plan_count == 0
    // frontline_plan_count は capture_snapshot 内で frontline_reg.plans.len() として算出
    assert_eq!(
        resolution_snap.frontline_plan_count, 0,
        "WarResolution直後: frontline_plan_count == 0 であること"
    );

    // (10) war_score > action_snap.war_score (戦勝点が加算されること)
    assert!(
        resolution_snap.war_score > action_snap.war_score,
        "WarResolution直後: 戦勝点が加算されること ({} → {})",
        action_snap.war_score,
        resolution_snap.war_score
    );

    // (11) FrontlineRegistry の戦後整理が完全に行われたこと
    let frontline_reg = app.world().resource::<FrontlineRegistry>();
    assert!(
        frontline_reg.frontlines.is_empty(),
        "WarResolution直後: frontline_reg.frontlines.is_empty() であること"
    );
    assert!(
        frontline_reg.plans.is_empty(),
        "WarResolution直後: frontline_reg.plans.is_empty() であること"
    );
    assert!(
        frontline_reg.army_frontline_map.is_empty(),
        "WarResolution直後: frontline_reg.army_frontline_map.is_empty() であること"
    );
    assert!(
        frontline_reg.frontline_generated_movements.is_empty(),
        "WarResolution直後: frontline_generated_movements.is_empty() であること"
    );
    // army_frontline_map に関係部隊が残っていないこと
    let war_related_army_ids: Vec<ArmyId> = app
        .world()
        .resource::<MilitaryRegistry>()
        .armies
        .keys()
        .copied()
        .collect();
    for army_id in &war_related_army_ids {
        assert!(
            !frontline_reg.army_frontline_map.contains_key(army_id),
            "WarResolution直後: army_frontline_map に ArmyId({:?}) が残っていないこと",
            army_id
        );
    }
}
