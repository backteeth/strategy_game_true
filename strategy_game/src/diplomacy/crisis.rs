use crate::app::time::GameDate;
use crate::common::{ClaimId, CountryId, DiplomaticCrisisId, StateId, WarId};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// P21-011: Claim受諾/拒否の要求期間(日数)。この期間内にtargetが応答しなければ、
/// 期限切れとして自動的に拒否扱い(`Escalating`)になる。
pub const CRISIS_DEMAND_PERIOD_DAYS: u32 = 30;

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
    /// P21-011: 追加。このCrisisの根拠となったTerritorialClaimのid。受諾時にこのClaimを
    /// `ClaimRegistry::mark_consumed`で消費済みにする。既存(P21-010以前)セーブは
    /// `#[serde(default)]`により`None`として読む(根拠不明のCrisisは受諾処理で
    /// `related_claim_id.is_none()`により拒否される=対象外操作として扱う)。
    #[serde(default)]
    pub related_claim_id: Option<ClaimId>,
    /// P21-011: 追加。拒否/期限切れによりinitiatorへ付与されたWarJustificationのid
    /// (`WarJustificationRegistry.justifications`のキー)。`Escalating`フェーズでのみ
    /// `Some`。既存(P21-010以前)セーブは`#[serde(default)]`により`None`として読む。
    #[serde(default)]
    pub related_justification_id: Option<usize>,
    /// P21-011: 追加。宣戦により`WarStarted`へ遷移した際の`WarId`。
    /// 既存(P21-010以前)セーブは`#[serde(default)]`により`None`として読む。
    #[serde(default)]
    pub related_war_id: Option<WarId>,
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
        if claim.status != crate::diplomacy::claims::ClaimStatus::Active {
            return Err("diplomacy_error.crisis.claim_consumed");
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

        // P21-011: 最後通牒として即座にDemandSentへ入り、開始日からCRISIS_DEMAND_PERIOD_DAYS
        // 日後を要求期限とする。開始日文字列が不正で解析できない場合(実運用では起こり
        // 得ないが、想定外の呼び出し元による壊れた日付文字列に対する保険として)は
        // 期限なし(`None`)とし、日次進行側の期限判定を素通りさせる。
        let deadline_date = GameDate::from_string(&start_date)
            .map(|d| d.add_days(CRISIS_DEMAND_PERIOD_DAYS).display());

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
            current_phase: CrisisPhase::DemandSent,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date,
            international_concern: 0.0,
            third_party_reactions: HashMap::new(),
            related_claim_id: Some(claim.id),
            related_justification_id: None,
            related_war_id: None,
        };

        Ok(self.add_crisis(crisis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diplomacy::claims::{ClaimSource, ClaimStatus, TerritorialClaim};
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
            status: ClaimStatus::Active,
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
        assert_eq!(
            crisis.current_phase,
            CrisisPhase::DemandSent,
            "P21-011: start_crisis must immediately issue an ultimatum (DemandSent), not sit in Preparing"
        );
        assert_eq!(
            crisis.deadline_date,
            Some("1800/01/31".to_string()),
            "deadline must be start_date + CRISIS_DEMAND_PERIOD_DAYS(30)"
        );
        assert_eq!(crisis.related_claim_id, Some(c.id));
        assert_eq!(crisis.related_justification_id, None);
        assert_eq!(crisis.related_war_id, None);
        assert_eq!(crisis.war_goals.len(), 1);
        assert_eq!(crisis.war_goals[0].target_states, vec![StateId(1)]);
    }

    /// P21-011要求テスト項目: 消費済み(Consumed)Claimからの新規Crisis開始は拒否される。
    #[test]
    fn start_crisis_rejects_consumed_claim() {
        let states = state_registry_with(1, 1);
        let mut c = claim(0, 1);
        c.status = ClaimStatus::Consumed;
        let mut registry = CrisisRegistry::default();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &states,
        );

        assert_eq!(result, Err("diplomacy_error.crisis.claim_consumed"));
        assert!(registry.crises.is_empty());
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

    /// P21-011要求テスト: 旧形式(`related_claim_id`/`related_justification_id`/
    /// `related_war_id`フィールド自体が存在しないRON、P21-010以前のセーブ相当)を
    /// 読み込んだ場合、全て`None`として復元される(`#[serde(default)]`)。
    #[test]
    fn old_format_ron_without_p21_011_fields_loads_as_none() {
        let crisis = DiplomaticCrisis {
            id: DiplomaticCrisisId(0),
            initiator: CountryId(0),
            target: CountryId(1),
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            current_phase: CrisisPhase::Negotiating,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: None,
            international_concern: 0.0,
            third_party_reactions: HashMap::new(),
            related_claim_id: Some(crate::common::ClaimId(5)),
            related_justification_id: Some(7),
            related_war_id: Some(WarId(9)),
        };
        let crisis_ron = ron::to_string(&crisis).unwrap();
        assert!(crisis_ron.contains("related_claim_id"));
        let without_new_fields = crisis_ron
            .replacen(",related_claim_id:Some((5))", "", 1)
            .replacen(",related_justification_id:Some(7)", "", 1)
            .replacen(",related_war_id:Some((9))", "", 1);
        assert_ne!(
            without_new_fields, crisis_ron,
            "test setup must actually remove the P21-011 fields"
        );
        let restored: DiplomaticCrisis = ron::from_str(&without_new_fields).expect(
            "DiplomaticCrisis RON missing P21-011 fields must still deserialize (serde default)",
        );
        assert_eq!(restored.related_claim_id, None);
        assert_eq!(restored.related_justification_id, None);
        assert_eq!(restored.related_war_id, None);
        assert_eq!(restored.current_phase, CrisisPhase::Negotiating);
    }

    /// P21-013要求テスト項目40-42: `third_party_reactions`(P21-013の支持データが
    /// 再利用するフィールド)に複数の支持国(要求国側・対象国側それぞれ)を入れた状態が
    /// RON往復で完全に保持される。このフィールドは P21-010 時点から`#[serde(default)]`
    /// 無しの必須フィールドとして存在するため、新しいフィールド追加は不要
    /// (旧セーブは既に空の`{}`を含んでいるため後方互換の懸念自体がない)。
    #[test]
    fn third_party_support_data_round_trips_through_ron() {
        use crate::common::ClaimId;

        let mut third_party_reactions = HashMap::new();
        third_party_reactions.insert(CountryId(2), ThirdCountryReaction::SupportsInitiator);
        third_party_reactions.insert(CountryId(3), ThirdCountryReaction::SupportsTarget);
        let crisis = DiplomaticCrisis {
            id: DiplomaticCrisisId(0),
            initiator: CountryId(0),
            target: CountryId(1),
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            current_phase: CrisisPhase::DemandSent,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: Some("1800/01/31".to_string()),
            international_concern: 0.0,
            third_party_reactions,
            related_claim_id: Some(ClaimId(1)),
            related_justification_id: None,
            related_war_id: None,
        };

        let ron_text = ron::to_string(&crisis).unwrap();
        let restored: DiplomaticCrisis = ron::from_str(&ron_text).unwrap();

        assert_eq!(restored.third_party_reactions.len(), 2);
        assert_eq!(
            restored.third_party_reactions.get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsInitiator)
        );
        assert_eq!(
            restored.third_party_reactions.get(&CountryId(3)),
            Some(&ThirdCountryReaction::SupportsTarget)
        );
    }

    /// P21-013要求テスト項目43: terminal Crisis(ResolvedPeacefully)でも支持履歴が
    /// RON往復で保持される。
    #[test]
    fn support_history_round_trips_after_terminal_resolution() {
        let mut third_party_reactions = HashMap::new();
        third_party_reactions.insert(CountryId(2), ThirdCountryReaction::SupportsTarget);
        let crisis = DiplomaticCrisis {
            id: DiplomaticCrisisId(0),
            initiator: CountryId(0),
            target: CountryId(1),
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            current_phase: CrisisPhase::ResolvedPeacefully,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 5,
            deadline_date: Some("1800/01/31".to_string()),
            international_concern: 0.0,
            third_party_reactions,
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        };

        let ron_text = ron::to_string(&crisis).unwrap();
        let restored: DiplomaticCrisis = ron::from_str(&ron_text).unwrap();

        assert_eq!(restored.current_phase, CrisisPhase::ResolvedPeacefully);
        assert_eq!(
            restored.third_party_reactions.get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsTarget),
            "support history must survive a save round trip even after the crisis resolved"
        );
    }
}
