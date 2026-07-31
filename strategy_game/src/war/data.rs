use crate::common::{CountryId, StateId, WarId};
use crate::diplomacy::crisis::WarGoal;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WarStatus {
    /// 進行中
    Active,
    /// 攻撃側勝利
    AttackerVictory,
    /// 防御側勝利
    DefenderVictory,
    /// 白紙講和
    WhitePeace,
    /// 中止・無効化
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct War {
    pub id: WarId,
    pub name: String,
    pub attackers: HashSet<CountryId>,
    pub defenders: HashSet<CountryId>,
    pub war_goals: Vec<WarGoal>,
    pub start_date: String,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub duration_days: u32,
    pub war_score: f32, // -100.0 (Defender winning) ~ 100.0 (Attacker winning)
    pub attacker_war_exhaustion: f32, // 0.0 ~ 100.0
    pub defender_war_exhaustion: f32, // 0.0 ~ 100.0
    pub occupied_states: HashSet<StateId>,
    pub status: WarStatus,
    #[serde(default)]
    pub winner: Option<CountryId>,
    #[serde(default)]
    pub end_reason: Option<String>,
    #[serde(default)]
    pub applied_terms: Vec<String>,

    // 戦闘結果集計用
    #[serde(default)]
    pub won_attacker_battles: u32,
    #[serde(default)]
    pub won_defender_battles: u32,
    #[serde(default)]
    pub processed_battle_ids: HashSet<crate::common::BattleId>,
}

#[derive(Resource, Default, Debug)]
pub struct WarRegistry {
    pub wars: HashMap<WarId, War>,
    next_id: usize,
}

impl WarRegistry {
    pub fn add_war(&mut self, mut war: War) -> WarId {
        war.id = WarId(self.next_id);
        self.next_id += 1;
        self.wars.insert(war.id, war.clone());
        war.id
    }

    pub fn get_active_war_for_country(&self, country: CountryId) -> Option<&War> {
        self.wars.values().find(|w| {
            w.status == WarStatus::Active
                && (w.attackers.contains(&country) || w.defenders.contains(&country))
        })
    }

    pub fn are_countries_at_war(&self, c1: CountryId, c2: CountryId) -> bool {
        if c1 == c2 {
            return false;
        }
        self.wars.values().any(|w| {
            w.status == WarStatus::Active
                && ((w.attackers.contains(&c1) && w.defenders.contains(&c2))
                    || (w.attackers.contains(&c2) && w.defenders.contains(&c1)))
        })
    }

    /// 宣戦布告の前提条件を検証する
    #[allow(clippy::too_many_arguments)]
    pub fn can_declare_war(
        &self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        country_registry: &crate::country::CountryRegistry,
        state_registry: &crate::state::data::StateRegistry,
        diplomacy_registry: &crate::diplomacy::data::DiplomacyRegistry,
        justification_registry: &crate::war::justification::WarJustificationRegistry,
    ) -> Result<(), &'static str> {
        self.can_declare_war_with_date(
            initiator,
            target,
            target_state,
            country_registry,
            state_registry,
            diplomacy_registry,
            justification_registry,
            None,
        )
    }

    /// 日付情報を含めて宣戦布告の前提条件を検証する
    #[allow(clippy::too_many_arguments)]
    pub fn can_declare_war_with_date(
        &self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        country_registry: &crate::country::CountryRegistry,
        state_registry: &crate::state::data::StateRegistry,
        diplomacy_registry: &crate::diplomacy::data::DiplomacyRegistry,
        justification_registry: &crate::war::justification::WarJustificationRegistry,
        current_date_str: Option<&str>,
    ) -> Result<(), &'static str> {
        if initiator == target {
            return Err("Cannot declare war against own country");
        }

        if country_registry.get(initiator).is_none() {
            return Err("Initiator country does not exist");
        }
        if country_registry.get(target).is_none() {
            return Err("Target country does not exist");
        }

        let state = state_registry
            .get(target_state)
            .ok_or("Target state does not exist")?;
        if state.owner_country_id != target {
            return Err("Target state is not owned by target country");
        }

        if let Some(rel) = diplomacy_registry.get(initiator, target) {
            if rel.has_treaty(crate::diplomacy::data::TreatyType::Alliance) {
                return Err("Cannot declare war on an ally");
            }
            if rel.has_treaty(crate::diplomacy::data::TreatyType::NonAggressionPact) {
                return Err("Cannot declare war on non-aggression pact partner");
            }
        }

        // 休戦チェック (日付指定があれば期限切れかどうか検証)
        if let Some(date_str) = current_date_str {
            if diplomacy_registry.is_in_truce(initiator, target, date_str) {
                return Err("Cannot declare war during truce");
            }
        } else if matches!(diplomacy_registry.get(initiator, target), Some(rel) if rel.truce_until.is_some())
        {
            return Err("Cannot declare war during truce");
        }

        if self.are_countries_at_war(initiator, target) {
            return Err("Countries are already at war");
        }

        if justification_registry
            .get_ready_justification(initiator, target, target_state)
            .is_none()
        {
            return Err("No completed war justification for this state");
        }

        Ok(())
    }

    /// 宣戦布告を実行し、新しい戦争データを作成する
    #[allow(clippy::too_many_arguments)]
    pub fn declare_war(
        &mut self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        start_date: String,
        country_registry: &crate::country::CountryRegistry,
        state_registry: &crate::state::data::StateRegistry,
        diplomacy_registry: &mut crate::diplomacy::data::DiplomacyRegistry,
        justification_registry: &mut crate::war::justification::WarJustificationRegistry,
    ) -> Result<WarId, &'static str> {
        self.can_declare_war_with_date(
            initiator,
            target,
            target_state,
            country_registry,
            state_registry,
            diplomacy_registry,
            justification_registry,
            Some(&start_date),
        )?;

        // 正当化を消費
        justification_registry.consume_justification(initiator, target, target_state);

        // 外交関係値の悪化 (-50.0)
        if let Some(rel) = diplomacy_registry.get_or_create_mut(initiator, target) {
            rel.opinion = (rel.opinion - 50.0).clamp(-100.0, 100.0);
        }

        let attacker_name = country_registry
            .get(initiator)
            .map(|c| c.name.as_str())
            .unwrap_or("Attacker");
        let defender_name = country_registry
            .get(target)
            .map(|c| c.name.as_str())
            .unwrap_or("Defender");
        let state_name = state_registry
            .get(target_state)
            .map(|s| s.name.as_str())
            .unwrap_or("State");

        let mut attackers = HashSet::new();
        attackers.insert(initiator);

        let mut defenders = HashSet::new();
        defenders.insert(target);

        let war_goal = crate::diplomacy::crisis::WarGoal {
            attacker: initiator,
            defender: target,
            goal_type: crate::diplomacy::crisis::WarGoalType::ConquerState,
            target_states: vec![target_state],
            base_peace_cost: 20.0,
            international_concern: 10.0,
            completion: 0.0,
            is_primary: true,
        };

        let war = War {
            id: WarId(self.next_id),
            name: format!(
                "{} Conquest of {} ({})",
                attacker_name, state_name, defender_name
            ),
            attackers,
            defenders,
            war_goals: vec![war_goal],
            start_date,
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

        let war_id = self.add_war(war);
        Ok(war_id)
    }
}
