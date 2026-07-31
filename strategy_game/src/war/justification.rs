use crate::common::{CountryId, StateId};
use crate::country::CountryRegistry;
use crate::diplomacy::data::{DiplomacyRegistry, TreatyType};
use crate::state::data::StateRegistry;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 正当化に必要なデフォルト日数
pub const DEFAULT_JUSTIFICATION_DAYS: u32 = 30;

/// 戦争正当化データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarJustification {
    pub id: usize,
    pub initiator: CountryId,
    pub target: CountryId,
    pub target_state: StateId,
    pub start_date: String,
    pub required_days: u32,
    pub days_passed: u32,
    pub is_ready: bool,
}

#[derive(Resource, Default, Debug, Serialize, Deserialize)]
pub struct WarJustificationRegistry {
    pub justifications: HashMap<usize, WarJustification>,
    next_id: usize,
}

impl WarJustificationRegistry {
    /// 正当化が開始可能かを検証し、エラーメッセージを返す
    pub fn can_start_justification(
        &self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        country_registry: &CountryRegistry,
        state_registry: &StateRegistry,
        diplomacy_registry: &DiplomacyRegistry,
    ) -> Result<(), &'static str> {
        self.can_start_justification_with_date(
            initiator,
            target,
            target_state,
            country_registry,
            state_registry,
            diplomacy_registry,
            None,
        )
    }

    /// 日付指定付きで正当化が開始可能かを検証する
    #[allow(clippy::too_many_arguments)]
    pub fn can_start_justification_with_date(
        &self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        country_registry: &CountryRegistry,
        state_registry: &StateRegistry,
        diplomacy_registry: &DiplomacyRegistry,
        current_date_str: Option<&str>,
    ) -> Result<(), &'static str> {
        // 自国に対する正当化は不可
        if initiator == target {
            return Err("Cannot justify war against own country");
        }

        // 国家の存在チェック
        if country_registry.get(initiator).is_none() {
            return Err("Initiator country does not exist");
        }
        if country_registry.get(target).is_none() {
            return Err("Target country does not exist");
        }

        // 州の存在チェックと所有権確認
        let state = state_registry
            .get(target_state)
            .ok_or("Target state does not exist")?;
        if state.owner_country_id != target {
            return Err("Target state is not owned by target country");
        }

        // 同盟・不可侵条約・休戦のチェック
        if let Some(rel) = diplomacy_registry.get(initiator, target) {
            if rel.has_treaty(TreatyType::Alliance) {
                return Err("Cannot justify war against an ally");
            }
            if rel.has_treaty(TreatyType::NonAggressionPact) {
                return Err("Cannot justify war against non-aggression pact partner");
            }
        }

        if let Some(date_str) = current_date_str {
            if diplomacy_registry.is_in_truce(initiator, target, date_str) {
                return Err("Cannot justify war during truce");
            }
        } else if matches!(diplomacy_registry.get(initiator, target), Some(rel) if rel.truce_until.is_some())
        {
            return Err("Cannot justify war during truce");
        }

        // 重複正当化のチェック
        let is_duplicate = self.justifications.values().any(|j| {
            j.initiator == initiator && j.target == target && j.target_state == target_state
        });
        if is_duplicate {
            return Err("Justification for this state already exists");
        }

        Ok(())
    }

    /// 正当化を開始する
    #[allow(clippy::too_many_arguments)]
    pub fn start_justification(
        &mut self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        start_date: String,
        country_registry: &CountryRegistry,
        state_registry: &StateRegistry,
        diplomacy_registry: &DiplomacyRegistry,
    ) -> Result<usize, &'static str> {
        self.can_start_justification(
            initiator,
            target,
            target_state,
            country_registry,
            state_registry,
            diplomacy_registry,
        )?;

        let id = self.next_id;
        self.next_id += 1;

        let justification = WarJustification {
            id,
            initiator,
            target,
            target_state,
            start_date,
            required_days: DEFAULT_JUSTIFICATION_DAYS,
            days_passed: 0,
            is_ready: false,
        };

        self.justifications.insert(id, justification);
        Ok(id)
    }

    /// 日次進行処理（時間連動）
    pub fn process_daily_justifications(&mut self, state_registry: &StateRegistry) {
        let mut invalid_ids = Vec::new();

        for justification in self.justifications.values_mut() {
            // 対象州の所有者が変わった等の安全チェック
            if let Some(state) = state_registry.get(justification.target_state) {
                if state.owner_country_id != justification.target {
                    invalid_ids.push(justification.id);
                    continue;
                }
            } else {
                invalid_ids.push(justification.id);
                continue;
            }

            if !justification.is_ready {
                justification.days_passed += 1;
                if justification.days_passed >= justification.required_days {
                    justification.is_ready = true;
                }
            }
        }

        // 無効になった正当化を削除
        for id in invalid_ids {
            self.justifications.remove(&id);
        }
    }

    /// 準備完了した特定正当化を取得
    pub fn get_ready_justification(
        &self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
    ) -> Option<&WarJustification> {
        self.justifications.values().find(|j| {
            j.initiator == initiator
                && j.target == target
                && j.target_state == target_state
                && j.is_ready
        })
    }

    /// 正当化を消費（削除）
    pub fn consume_justification(
        &mut self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
    ) -> bool {
        if let Some(id) = self.justifications.iter().find_map(|(&id, j)| {
            if j.initiator == initiator
                && j.target == target
                && j.target_state == target_state
                && j.is_ready
            {
                Some(id)
            } else {
                None
            }
        }) {
            self.justifications.remove(&id);
            true
        } else {
            false
        }
    }
}
