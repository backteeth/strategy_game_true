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
    /// 次回発行されるDiplomaticCrisisIdの値（P21-SAVE-002B: セーブ用の読み取り専用アクセサ。
    /// `WarRegistry`/`BattleRegistry`/`WarJustificationRegistry`の`next_id()`と同じ形）
    pub fn next_id(&self) -> usize {
        self.next_id
    }

    /// 保存されていた要素・次回ID発行値から`CrisisRegistry`を復元する（P21-SAVE-002D）。
    /// `crate::save`のDTO型ではなく、通常のコレクションとカウンタだけを引数にとる。
    pub(crate) fn from_saved_parts(
        crises: HashMap<DiplomaticCrisisId, DiplomaticCrisis>,
        next_id: usize,
    ) -> Self {
        Self { crises, next_id }
    }

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

    /// P21-010: 領土請求(`TerritorialClaim`)を根拠に外交危機を開始できるかを検証する
    /// (実際には開始しない)。戻り値は表示用の英語原文ではなく安定した翻訳キー
    /// (diplomacy_error.crisis.*)。呼び出し元UI(diplomacy_panel)が
    /// `localization::t()`で表示直前に言語へ解決する。
    ///
    /// `claim`自体には対象国フィールドがないため、`claim.target_state`の現在の
    /// 所有国(`state_registry`から取得)を対象国とみなす(所有権が請求後に変化していても
    /// 常に最新の状態で検証する)。
    pub fn can_start_crisis(
        &self,
        claim: &crate::diplomacy::claims::TerritorialClaim,
        claimant: CountryId,
        target: CountryId,
        state_registry: &crate::state::data::StateRegistry,
    ) -> Result<(), &'static str> {
        if claim.claimant_country != claimant {
            return Err("diplomacy_error.crisis.not_your_claim");
        }

        let state = state_registry
            .get(claim.target_state)
            .ok_or("diplomacy_error.crisis.state_missing")?;
        if state.owner_country_id != target {
            return Err("diplomacy_error.crisis.claim_target_mismatch");
        }

        // 同じClaim、または同じ当事国ペア・対象州を含む進行中Crisisを拒否する
        // (DiplomaticCrisisはclaim_idを保持しないため、initiator/target/war_goalsの
        // target_statesで同一性を判定する。これは「同じClaim」も内包する: 同じClaimから
        // 開始されるCrisisは必ず同じinitiator/target/target_stateの組になるため)。
        let is_duplicate = self.crises.values().any(|c| {
            c.initiator == claimant
                && c.target == target
                && c.current_phase != CrisisPhase::ResolvedPeacefully
                && c.current_phase != CrisisPhase::WarStarted
                && c.current_phase != CrisisPhase::Cancelled
                && c.war_goals
                    .iter()
                    .any(|g| g.target_states.contains(&claim.target_state))
        });
        if is_duplicate {
            return Err("diplomacy_error.crisis.duplicate");
        }

        Ok(())
    }

    /// P21-010: 検証込みで、領土請求を根拠に外交危機を開始する。検証に失敗した場合、
    /// Registry(`crises`・`next_id`)は一切変更しない。既存のWarJustification・War・
    /// 州所有権はここでは一切変更しない(接続のみ、新しい外交ルールは実装しない)。
    pub fn start_crisis(
        &mut self,
        claim: &crate::diplomacy::claims::TerritorialClaim,
        claimant: CountryId,
        target: CountryId,
        start_date: String,
        state_registry: &crate::state::data::StateRegistry,
    ) -> Result<DiplomaticCrisisId, &'static str> {
        self.can_start_crisis(claim, claimant, target, state_registry)?;

        let crisis = DiplomaticCrisis {
            id: DiplomaticCrisisId(0), // add_crisisが正規のnext_id発行値へ上書きする
            initiator: claimant,
            target,
            war_goals: vec![WarGoal {
                attacker: claimant,
                defender: target,
                goal_type: WarGoalType::ConquerState,
                target_states: vec![claim.target_state],
                base_peace_cost: 0.0,
                international_concern: 0.0,
                completion: 0.0,
                is_primary: true,
            }],
            start_date,
            current_phase: CrisisPhase::Preparing,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: None,
            international_concern: 0.0,
            third_party_reactions: HashMap::new(),
        };

        Ok(self.add_crisis(crisis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diplomacy::claims::{ClaimSource, TerritorialClaim};
    use crate::state::data::{StateData, StateRegistry};

    fn claim(claimant: usize, target_state: usize) -> TerritorialClaim {
        TerritorialClaim {
            id: crate::common::ClaimId(0),
            claimant_country: CountryId(claimant),
            target_state: StateId(target_state),
            strength: 50.0,
            created_date: "1800/01/01".to_string(),
            is_permanent: false,
            source: ClaimSource::Strategic,
        }
    }

    fn state_registry_with(state_id: usize, owner: usize) -> StateRegistry {
        StateRegistry::build(vec![StateData {
            id: StateId(state_id),
            owner_country_id: CountryId(owner),
            ..Default::default()
        }])
    }

    /// 要求テスト項目9: 有効なClaimからCrisis開始。
    #[test]
    fn start_crisis_succeeds_for_valid_claim() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let mut registry = CrisisRegistry::default();

        let id = registry
            .start_crisis(
                &c,
                CountryId(0),
                CountryId(1),
                "1800/01/01".to_string(),
                &states,
            )
            .expect("valid claim must allow crisis start");

        let crisis = registry.crises.get(&id).unwrap();
        assert_eq!(crisis.initiator, CountryId(0));
        assert_eq!(crisis.target, CountryId(1));
        assert_eq!(crisis.current_phase, CrisisPhase::Preparing);
        assert_eq!(crisis.war_goals.len(), 1);
        assert_eq!(crisis.war_goals[0].target_states, vec![StateId(1)]);
    }

    /// 要求テスト項目10: ClaimなしでCrisis開始拒否(状態不整合: 州が存在しない)。
    #[test]
    fn start_crisis_rejects_when_claim_state_missing() {
        let states = StateRegistry::build(vec![]);
        let c = claim(0, 1);
        let mut registry = CrisisRegistry::default();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &states,
        );

        assert_eq!(result, Err("diplomacy_error.crisis.state_missing"));
        assert!(registry.crises.is_empty());
    }

    /// 要求テスト項目11: 他国Claimからの開始拒否。
    #[test]
    fn start_crisis_rejects_when_claimant_does_not_own_the_claim() {
        let states = state_registry_with(1, 1);
        let c = claim(2, 1); // claimantはCountryId(2)、CountryId(0)のClaimではない
        let mut registry = CrisisRegistry::default();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &states,
        );

        assert_eq!(result, Err("diplomacy_error.crisis.not_your_claim"));
    }

    #[test]
    fn start_crisis_rejects_when_target_no_longer_owns_claimed_state() {
        let states = state_registry_with(1, 2); // 現在の所有国はCountryId(2)
        let c = claim(0, 1);
        let mut registry = CrisisRegistry::default();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &states,
        );

        assert_eq!(result, Err("diplomacy_error.crisis.claim_target_mismatch"));
    }

    /// 要求テスト項目12: 重複Crisis拒否。
    #[test]
    fn start_crisis_rejects_duplicate_for_same_parties_and_state() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let mut registry = CrisisRegistry::default();
        registry
            .start_crisis(
                &c,
                CountryId(0),
                CountryId(1),
                "1800/01/01".to_string(),
                &states,
            )
            .unwrap();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/02".to_string(),
            &states,
        );

        assert_eq!(result, Err("diplomacy_error.crisis.duplicate"));
        assert_eq!(registry.crises.len(), 1);
    }

    /// 要求テスト項目13: 不正要求時にCrisis next_id不変。
    #[test]
    fn invalid_crisis_request_does_not_advance_next_id() {
        let states = state_registry_with(1, 2);
        let c = claim(0, 1);
        let mut registry = CrisisRegistry::default();
        let before = registry.next_id();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &states,
        );

        assert!(result.is_err());
        assert_eq!(registry.next_id(), before);
        assert!(registry.crises.is_empty());
    }

    /// terminal state(ResolvedPeacefully等)の既存Crisisは重複判定から除外される
    /// (再度同じ当事国・対象州でCrisisを開始し直せる)。
    #[test]
    fn resolved_crisis_does_not_block_a_new_crisis_for_the_same_claim() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let mut registry = CrisisRegistry::default();
        let first_id = registry
            .start_crisis(
                &c,
                CountryId(0),
                CountryId(1),
                "1800/01/01".to_string(),
                &states,
            )
            .unwrap();
        registry.crises.get_mut(&first_id).unwrap().current_phase = CrisisPhase::ResolvedPeacefully;

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/02".to_string(),
            &states,
        );

        assert!(result.is_ok());
        assert_eq!(registry.crises.len(), 2);
    }
}
