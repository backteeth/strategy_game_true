use crate::app::time::GameDate;
use crate::common::{ClaimId, CountryId, DiplomaticCrisisId, StateId, WarId};
use crate::diplomacy::data::{DiplomacyRegistry, TreatyType};
use crate::war::data::WarRegistry;
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
    /// P21-021: initiatorまたはtargetの国家消滅により、未解決のまま強制終端
    /// された。日次応答・deadline処理・AI支持の対象外(`is_unresolved_phase`
    /// が`false`を返す終端状態の1つ)。
    CancelledByCountryElimination,
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

    /// P21-017(二次レビュー対応): `country`が開始したCrisisで、まだ終結していない
    /// (ResolvedPeacefully/WarStarted/Cancelledでない)ものが1件でも存在するか。
    /// AI月次Crisis開始判定(`diplomacy::ai_crisis_start`)と、既存の生の戦争準備AI
    /// (`country::country_ai::process_war_preparation_ai`)の両方から呼ばれ、
    /// 「未解決の自国開始Crisisがあれば新規の正当化探索を始めない」という判定基準を
    /// 1箇所に統一する(片方だけが知っていて、もう片方が同じ対象へ独立に正当化を
    /// 開始してしまう抜けを防ぐ)。
    pub fn has_unresolved_crisis_initiated_by(&self, country: CountryId) -> bool {
        self.crises
            .values()
            .any(|c| c.initiator == country && is_unresolved_phase(c.current_phase))
    }

    /// P21-018: `has_unresolved_crisis_initiated_by`をCrisis Registry1回の走査で
    /// 全AI国家分まとめて求めるための集合版(`country × crises`の総当たりを避ける)。
    /// 「未解決」の定義は`is_unresolved_phase`を共有するため、単体版とズレない。
    pub(crate) fn unresolved_crisis_initiators(&self) -> std::collections::HashSet<CountryId> {
        self.crises
            .values()
            .filter(|c| is_unresolved_phase(c.current_phase))
            .map(|c| c.initiator)
            .collect()
    }

    /// P21-010: 領土請求(`TerritorialClaim`)を根拠に外交危機を開始できるかを検証する
    /// (実際には開始しない)。戻り値は表示用の英語原文ではなく安定した翻訳キー
    /// (diplomacy_error.crisis.*)。呼び出し元UI(diplomacy_panel)が
    /// `localization::t()`で表示直前に言語へ解決する。
    ///
    /// `claim`自体には対象国フィールドがないため、`claim.target_state`の現在の
    /// 所有国(`state_registry`から取得)を対象国とみなす(所有権が請求後に変化していても
    /// 常に最新の状態で検証する)。
    ///
    /// P21-017(二次レビュー対応): 現在戦争中・同盟中・不可侵条約中・休戦中の相手への
    /// Crisis開始を、共有ドメインAPIであるこの関数自身で拒否する(以前はこれらの検証が
    /// 存在せず、`diplomacy::ai_crisis_start`側だけが独自にAI候補を絞り込んでいたため、
    /// プレイヤーUIとAIとで判定基準が分裂するリスクがあった)。判定ロジック自体は
    /// [`diplomatic_conditions_permit_crisis_start`]へ切り出し、AI候補選定
    /// (`ai_crisis_start::evaluate_pair_eligibility`)からも同じ関数を呼ぶことで、
    /// 両者が将来ズレることを防ぐ。
    #[allow(clippy::too_many_arguments)]
    pub fn can_start_crisis(
        &self,
        claim: &crate::diplomacy::claims::TerritorialClaim,
        claimant: CountryId,
        target: CountryId,
        country_registry: &crate::country::CountryRegistry,
        state_registry: &crate::state::data::StateRegistry,
        diplomacy_registry: &DiplomacyRegistry,
        war_registry: &WarRegistry,
        current_date: &str,
    ) -> Result<(), &'static str> {
        // P21-021: 消滅済み国家はCrisisの開始国にも対象国にもなれない
        // (Domain APIレベルでの拒否。UIのrun_ifだけに依存しない)。
        if country_registry
            .get(claimant)
            .is_none_or(|c| !c.is_active())
        {
            return Err("diplomacy_error.crisis.claimant_missing");
        }
        if country_registry.get(target).is_none_or(|c| !c.is_active()) {
            return Err("diplomacy_error.crisis.target_missing");
        }
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

        if war_registry.are_countries_at_war(claimant, target) {
            return Err("diplomacy_error.crisis.at_war");
        }
        diplomatic_conditions_permit_crisis_start(
            claimant,
            target,
            diplomacy_registry,
            current_date,
        )?;

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
    #[allow(clippy::too_many_arguments)]
    pub fn start_crisis(
        &mut self,
        claim: &crate::diplomacy::claims::TerritorialClaim,
        claimant: CountryId,
        target: CountryId,
        start_date: String,
        country_registry: &crate::country::CountryRegistry,
        state_registry: &crate::state::data::StateRegistry,
        diplomacy_registry: &DiplomacyRegistry,
        war_registry: &WarRegistry,
    ) -> Result<DiplomaticCrisisId, &'static str> {
        self.can_start_crisis(
            claim,
            claimant,
            target,
            country_registry,
            state_registry,
            diplomacy_registry,
            war_registry,
            &start_date,
        )?;

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

/// P21-017(二次レビュー対応): 同盟・不可侵条約・休戦に関するCrisis開始可否判定。
/// `CrisisRegistry::can_start_crisis`(プレイヤーUI経由)と
/// `diplomacy::ai_crisis_start`(AI候補選定)の両方から呼ばれる唯一の実装であり、
/// 片方だけを直したことによる将来の判定ズレを構造的に防ぐ。「現在戦争中」は
/// 呼び出し元ごとに効率の異なる方法(単発なら`WarRegistry::are_countries_at_war`、
/// 一括評価なら事前集約したペア集合)で判定するため、この関数はそれを含まない。
/// あるCrisisPhaseが「まだ終結していない(未解決)」かどうか。
/// `has_unresolved_crisis_initiated_by`/`unresolved_crisis_initiators`
/// (このモジュール)と`ai_crisis_start`/`ai_claim_generation`双方のCrisis Registry
/// 走査が同じ「未解決」の定義を共有するための唯一の実装。
pub(crate) fn is_unresolved_phase(phase: CrisisPhase) -> bool {
    !matches!(
        phase,
        CrisisPhase::ResolvedPeacefully
            | CrisisPhase::WarStarted
            | CrisisPhase::Cancelled
            | CrisisPhase::CancelledByCountryElimination
    )
}

pub(crate) fn diplomatic_conditions_permit_crisis_start(
    claimant: CountryId,
    target: CountryId,
    diplomacy_registry: &DiplomacyRegistry,
    current_date: &str,
) -> Result<(), &'static str> {
    if let Some(relation) = diplomacy_registry.get(claimant, target) {
        if relation.has_treaty(TreatyType::Alliance) {
            return Err("diplomacy_error.crisis.ally");
        }
        if relation.has_treaty(TreatyType::NonAggressionPact) {
            return Err("diplomacy_error.crisis.non_aggression_pact");
        }
    }
    if diplomacy_registry.is_in_truce(claimant, target, current_date) {
        return Err("diplomacy_error.crisis.truce");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::{CountryData, CountryRegistry};
    use crate::diplomacy::claims::{ClaimSource, ClaimStatus, TerritorialClaim};
    use crate::state::data::{StateData, StateRegistry};

    /// P21-017の`can_start_crisis`/`start_crisis`が要求するclaimant/target存在確認用の
    /// 最小`CountryRegistry`。両国とも`CountryData::default()`のまま(=`Active`)にする。
    fn country_registry_with(claimant: usize, target: usize) -> CountryRegistry {
        CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(claimant),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(target),
                    ..CountryData::default()
                },
            ],
        }
    }

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
        let countries = country_registry_with(0, 1);
        let mut registry = CrisisRegistry::default();

        let id = registry
            .start_crisis(
                &c,
                CountryId(0),
                CountryId(1),
                "1800/01/01".to_string(),
                &countries,
                &states,
                &DiplomacyRegistry::default(),
                &WarRegistry::default(),
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

    /// P21-021: 消滅済み国家はDomain APIレベルでCrisisの開始国になれない
    /// (UIのrun_ifだけに依存しない直接検証)。
    #[test]
    fn start_crisis_rejects_an_eliminated_claimant() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let mut countries = country_registry_with(0, 1);
        countries.get_mut(CountryId(0)).unwrap().status =
            crate::country::CountryStatus::Eliminated {
                eliminated_date: "1799/12/01".to_string(),
                annexed_by: CountryId(9),
            };
        let mut registry = CrisisRegistry::default();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &countries,
            &states,
            &DiplomacyRegistry::default(),
            &WarRegistry::default(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.claimant_missing"));
        assert!(registry.crises.is_empty());
    }

    /// P21-021: 消滅済み国家はDomain APIレベルでCrisisの対象国にもなれない。
    #[test]
    fn start_crisis_rejects_an_eliminated_target() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let mut countries = country_registry_with(0, 1);
        countries.get_mut(CountryId(1)).unwrap().status =
            crate::country::CountryStatus::Eliminated {
                eliminated_date: "1799/12/01".to_string(),
                annexed_by: CountryId(9),
            };
        let mut registry = CrisisRegistry::default();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &countries,
            &states,
            &DiplomacyRegistry::default(),
            &WarRegistry::default(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.target_missing"));
        assert!(registry.crises.is_empty());
    }

    /// P21-011要求テスト項目: 消費済み(Consumed)Claimからの新規Crisis開始は拒否される。
    #[test]
    fn start_crisis_rejects_consumed_claim() {
        let states = state_registry_with(1, 1);
        let mut c = claim(0, 1);
        c.status = ClaimStatus::Consumed;
        let countries = country_registry_with(0, 1);
        let mut registry = CrisisRegistry::default();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &countries,
            &states,
            &DiplomacyRegistry::default(),
            &WarRegistry::default(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.claim_consumed"));
        assert!(registry.crises.is_empty());
    }

    /// 要求テスト項目10: ClaimなしでCrisis開始拒否(状態不整合: 州が存在しない)。
    #[test]
    fn start_crisis_rejects_when_claim_state_missing() {
        let states = StateRegistry::build(vec![]);
        let c = claim(0, 1);
        let countries = country_registry_with(0, 1);
        let mut registry = CrisisRegistry::default();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &countries,
            &states,
            &DiplomacyRegistry::default(),
            &WarRegistry::default(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.state_missing"));
        assert!(registry.crises.is_empty());
    }

    /// 要求テスト項目11: 他国Claimからの開始拒否。
    #[test]
    fn start_crisis_rejects_when_claimant_does_not_own_the_claim() {
        let states = state_registry_with(1, 1);
        let c = claim(2, 1); // claimantはCountryId(2)、CountryId(0)のClaimではない
        let countries = country_registry_with(0, 1);
        let mut registry = CrisisRegistry::default();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &countries,
            &states,
            &DiplomacyRegistry::default(),
            &WarRegistry::default(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.not_your_claim"));
    }

    #[test]
    fn start_crisis_rejects_when_target_no_longer_owns_claimed_state() {
        let states = state_registry_with(1, 2); // 現在の所有国はCountryId(2)
        let c = claim(0, 1);
        let countries = country_registry_with(0, 1);
        let mut registry = CrisisRegistry::default();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &countries,
            &states,
            &DiplomacyRegistry::default(),
            &WarRegistry::default(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.claim_target_mismatch"));
    }

    /// 要求テスト項目12: 重複Crisis拒否。
    #[test]
    fn start_crisis_rejects_duplicate_for_same_parties_and_state() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let countries = country_registry_with(0, 1);
        let mut registry = CrisisRegistry::default();
        registry
            .start_crisis(
                &c,
                CountryId(0),
                CountryId(1),
                "1800/01/01".to_string(),
                &countries,
                &states,
                &DiplomacyRegistry::default(),
                &WarRegistry::default(),
            )
            .unwrap();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/02".to_string(),
            &countries,
            &states,
            &DiplomacyRegistry::default(),
            &WarRegistry::default(),
        );

        assert_eq!(result, Err("diplomacy_error.crisis.duplicate"));
        assert_eq!(registry.crises.len(), 1);
    }

    // ─────────────────────────────────────────────────────────────────
    // P21-017(二次レビュー対応): 共有ドメインAPI(`can_start_crisis`/
    // `start_crisis`)自身が外交条件を拒否することの単体テスト。
    // 以前はこれらの検証が`can_start_crisis`に存在せず、AI候補選定
    // (`diplomacy::ai_crisis_start`)側だけが独自に判定していたため、
    // プレイヤーUIとAIとで判定基準が分裂するリスクがあった。
    // ─────────────────────────────────────────────────────────────────

    fn war_registry_with_active_war(attacker: usize, defender: usize) -> WarRegistry {
        let mut war_registry = WarRegistry::default();
        war_registry.wars.insert(
            crate::common::WarId(0),
            crate::war::data::War {
                id: crate::common::WarId(0),
                name: "Test War".to_string(),
                start_date: "1800/01/01".to_string(),
                end_date: None,
                duration_days: 0,
                war_score: 0.0,
                attackers: [CountryId(attacker)].into_iter().collect(),
                defenders: [CountryId(defender)].into_iter().collect(),
                primary_attacker: Some(CountryId(attacker)),
                primary_defender: Some(CountryId(defender)),
                source_crisis_id: None,
                war_goals: vec![],
                attacker_war_exhaustion: 0.0,
                defender_war_exhaustion: 0.0,
                occupied_states: std::collections::HashSet::new(),
                status: crate::war::data::WarStatus::Active,
                winner: None,
                end_reason: None,
                applied_terms: Vec::new(),
                won_attacker_battles: 0,
                won_defender_battles: 0,
                processed_battle_ids: std::collections::HashSet::new(),
            },
        );
        war_registry
    }

    /// 共有ドメインAPI単体テスト: 現在戦争中の相手へのCrisis開始は拒否される。
    #[test]
    fn can_start_crisis_rejects_when_claimant_and_target_are_at_war() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let countries = country_registry_with(0, 1);
        let registry = CrisisRegistry::default();
        let war_registry = war_registry_with_active_war(0, 1);

        let result = registry.can_start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            &countries,
            &states,
            &DiplomacyRegistry::default(),
            &war_registry,
            "1800/01/01",
        );

        assert_eq!(result, Err("diplomacy_error.crisis.at_war"));
    }

    /// 共有ドメインAPI単体テスト: 同盟中の相手へのCrisis開始は拒否される。
    #[test]
    fn can_start_crisis_rejects_when_claimant_and_target_are_allied() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let countries = country_registry_with(0, 1);
        let registry = CrisisRegistry::default();
        let mut diplomacy_registry = DiplomacyRegistry::default();
        let key =
            crate::diplomacy::data::DiplomaticPairKey::new(CountryId(0), CountryId(1)).unwrap();
        diplomacy_registry.relations.insert(
            key,
            crate::diplomacy::data::DiplomaticRelation {
                treaties: vec![crate::diplomacy::data::ActiveTreaty {
                    treaty_type: TreatyType::Alliance,
                    countries: (CountryId(0), CountryId(1)),
                    signed_date: "1800/01/01".to_string(),
                    is_active: true,
                }],
                ..Default::default()
            },
        );

        let result = registry.can_start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            &countries,
            &states,
            &diplomacy_registry,
            &WarRegistry::default(),
            "1800/01/01",
        );

        assert_eq!(result, Err("diplomacy_error.crisis.ally"));
    }

    /// 共有ドメインAPI単体テスト: 不可侵条約中の相手へのCrisis開始は拒否される。
    #[test]
    fn can_start_crisis_rejects_when_claimant_and_target_have_a_non_aggression_pact() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let countries = country_registry_with(0, 1);
        let registry = CrisisRegistry::default();
        let mut diplomacy_registry = DiplomacyRegistry::default();
        let key =
            crate::diplomacy::data::DiplomaticPairKey::new(CountryId(0), CountryId(1)).unwrap();
        diplomacy_registry.relations.insert(
            key,
            crate::diplomacy::data::DiplomaticRelation {
                treaties: vec![crate::diplomacy::data::ActiveTreaty {
                    treaty_type: TreatyType::NonAggressionPact,
                    countries: (CountryId(0), CountryId(1)),
                    signed_date: "1800/01/01".to_string(),
                    is_active: true,
                }],
                ..Default::default()
            },
        );

        let result = registry.can_start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            &countries,
            &states,
            &diplomacy_registry,
            &WarRegistry::default(),
            "1800/01/01",
        );

        assert_eq!(result, Err("diplomacy_error.crisis.non_aggression_pact"));
    }

    /// 共有ドメインAPI単体テスト: 休戦中の相手へのCrisis開始は拒否される。
    #[test]
    fn can_start_crisis_rejects_when_claimant_and_target_are_in_truce() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let countries = country_registry_with(0, 1);
        let registry = CrisisRegistry::default();
        let mut diplomacy_registry = DiplomacyRegistry::default();
        diplomacy_registry.set_truce(CountryId(0), CountryId(1), "1800/06/01".to_string());

        let result = registry.can_start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            &countries,
            &states,
            &diplomacy_registry,
            &WarRegistry::default(),
            "1800/01/01",
        );

        assert_eq!(result, Err("diplomacy_error.crisis.truce"));
    }

    /// 共有ドメインAPI単体テスト: 休戦期限を過ぎていればCrisis開始できる。
    #[test]
    fn can_start_crisis_succeeds_once_the_truce_has_expired() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let countries = country_registry_with(0, 1);
        let registry = CrisisRegistry::default();
        let mut diplomacy_registry = DiplomacyRegistry::default();
        diplomacy_registry.set_truce(CountryId(0), CountryId(1), "1800/01/01".to_string());

        let result = registry.can_start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            &countries,
            &states,
            &diplomacy_registry,
            &WarRegistry::default(),
            "1800/06/01",
        );

        assert!(result.is_ok());
    }

    /// `has_unresolved_crisis_initiated_by`: 未解決CrisisがあればtrueだがResolvedなら
    /// falseに戻る。AI候補選定(`ai_crisis_start`)と生の戦争準備AI
    /// (`country_ai::process_war_preparation_ai`)の両方が呼ぶ唯一の実装。
    #[test]
    fn has_unresolved_crisis_initiated_by_reflects_current_phase() {
        let states = state_registry_with(1, 1);
        let c = claim(0, 1);
        let countries = country_registry_with(0, 1);
        let mut registry = CrisisRegistry::default();
        assert!(!registry.has_unresolved_crisis_initiated_by(CountryId(0)));

        let id = registry
            .start_crisis(
                &c,
                CountryId(0),
                CountryId(1),
                "1800/01/01".to_string(),
                &countries,
                &states,
                &DiplomacyRegistry::default(),
                &WarRegistry::default(),
            )
            .unwrap();
        assert!(registry.has_unresolved_crisis_initiated_by(CountryId(0)));
        assert!(
            !registry.has_unresolved_crisis_initiated_by(CountryId(1)),
            "target is not the initiator"
        );

        registry.crises.get_mut(&id).unwrap().current_phase = CrisisPhase::ResolvedPeacefully;
        assert!(!registry.has_unresolved_crisis_initiated_by(CountryId(0)));
    }

    /// 要求テスト項目13: 不正要求時にCrisis next_id不変。
    #[test]
    fn invalid_crisis_request_does_not_advance_next_id() {
        let states = state_registry_with(1, 2);
        let c = claim(0, 1);
        let countries = country_registry_with(0, 1);
        let mut registry = CrisisRegistry::default();
        let before = registry.next_id();

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/01".to_string(),
            &countries,
            &states,
            &DiplomacyRegistry::default(),
            &WarRegistry::default(),
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
        let countries = country_registry_with(0, 1);
        let mut registry = CrisisRegistry::default();
        let first_id = registry
            .start_crisis(
                &c,
                CountryId(0),
                CountryId(1),
                "1800/01/01".to_string(),
                &countries,
                &states,
                &DiplomacyRegistry::default(),
                &WarRegistry::default(),
            )
            .unwrap();
        registry.crises.get_mut(&first_id).unwrap().current_phase = CrisisPhase::ResolvedPeacefully;

        let result = registry.start_crisis(
            &c,
            CountryId(0),
            CountryId(1),
            "1800/01/02".to_string(),
            &countries,
            &states,
            &DiplomacyRegistry::default(),
            &WarRegistry::default(),
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
