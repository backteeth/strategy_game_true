use crate::common::{CountryId, DiplomaticCrisisId, StateId};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 戦争目的の種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WarGoalType {
    ConquerState,
    Reparations,
    MakePuppet,
    RegimeChange,
    BreakAlliance,
    RestrictScience,
    RestrictMagic,
}

/// 戦争目的データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarGoal {
    pub attacker: CountryId,
    pub defender: CountryId,
    pub goal_type: WarGoalType,
    pub target_states: Vec<StateId>,
    pub base_peace_cost: f32,
    pub international_concern: f32,
    pub completion: f32,
    pub is_primary: bool,
}

/// 外交危機の段階
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrisisPhase {
    Preparing,
    DemandSent,
    Negotiating,
    Escalating,
    ResolvedPeacefully,
    WarStarted,
    Cancelled,
}

/// 第三国の態度
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThirdCountryReaction {
    Neutral,
    SupportsInitiator,
    SupportsTarget,
    CondemnsInitiator,
}

/// 外交危機データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiplomaticCrisis {
    pub id: DiplomaticCrisisId,
    pub initiator: CountryId,
    pub target: CountryId,
    pub war_goals: Vec<WarGoal>,
    pub start_date: String,
    pub current_phase: CrisisPhase,
    pub escalation: f32, // 0.0 〜 100.0
    pub initiator_support: f32,
    pub target_resistance: f32,
    pub days_in_phase: u32,
    pub deadline_date: Option<String>,
    pub international_concern: f32,
    pub third_party_reactions: HashMap<CountryId, ThirdCountryReaction>,
}

/// 全外交危機を管理するリソース
#[derive(Resource, Default, Debug)]
pub struct CrisisRegistry {
    pub crises: HashMap<DiplomaticCrisisId, DiplomaticCrisis>,
    next_id: usize,
}

impl CrisisRegistry {
    pub fn add_crisis(&mut self, mut crisis: DiplomaticCrisis) -> DiplomaticCrisisId {
        crisis.id = DiplomaticCrisisId(self.next_id);
        self.next_id += 1;
        self.crises.insert(crisis.id, crisis.clone());
        crisis.id
    }

    pub fn get_active_crisis_for_country(&self, country: CountryId) -> Option<&DiplomaticCrisis> {
        self.crises.values().find(|c| {
            (c.initiator == country || c.target == country)
                && c.current_phase != CrisisPhase::ResolvedPeacefully
                && c.current_phase != CrisisPhase::WarStarted
                && c.current_phase != CrisisPhase::Cancelled
        })
    }
}
