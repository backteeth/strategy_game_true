use crate::common::{ClaimId, CountryId, StateId};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 領土請求の発生源
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimSource {
    Historical,
    BorderDispute,
    Cultural,
    Strategic,
    Debug,
    /// P21-018: AI国家が月次評価で自動生成したClaim。旧Save(このvariant追加前)には
    /// 出現しないため後方互換上の懸念はない(読み込み側は既存variantのみを期待する
    /// 旧バージョンでのみ問題になり得るが、本バージョンの`ClaimSource`はenum全体が
    /// `#[serde(default)]`無しの通常デシリアライズのため、新規に書き出したSaveを
    /// 古いバイナリで読む場合を除き影響しない)。
    AiExpansion,
}

impl ClaimSource {
    /// 表示用の翻訳キー(P20-009の`display_name`規約、`PowerTier`/`CountryAiMode`と同じ形)。
    /// UI側で`localization::t()`により言語ごとの表示名へ解決する。
    /// レビュー指摘対応(P21-018): それまで`diplomacy_panel.rs`は`format!("{:?}",
    /// claim.source)`でDebug表示していたため、JA UIでも"AiExpansion"等の生の
    /// Rust識別子がそのまま出ていた。全variantをここで一括して翻訳キー化する。
    pub fn display_name(self) -> &'static str {
        match self {
            ClaimSource::Historical => "claim_source.historical",
            ClaimSource::BorderDispute => "claim_source.border_dispute",
            ClaimSource::Cultural => "claim_source.cultural",
            ClaimSource::Strategic => "claim_source.strategic",
            ClaimSource::Debug => "claim_source.debug",
            ClaimSource::AiExpansion => "claim_source.ai_expansion",
        }
    }
}

/// 領土請求の状態(P21-011)。`#[serde(default)]`によりP21-010以前のセーブ
/// (このフィールド自体が存在しない)は全て`Active`として読み込まれる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ClaimStatus {
    /// 有効。新しいCrisisの根拠として使用可能。
    #[default]
    Active,
    /// Crisisの平和的受諾により消費済み。対象州は既に譲渡されており、
    /// 新しいCrisisの根拠として再利用できない(Claim自体は履歴として残す)。
    Consumed,
    /// P21-021: claimant国家が消滅したため無効化された。対象州が譲渡されて
    /// 「達成された」`Consumed`とは意味が異なる(請求者自身がもう存在しない
    /// ため、根拠として使いようがない)ため区別する。
    Invalidated,
}

impl ClaimStatus {
    /// 表示用の翻訳キー(P20-009の`display_name`規約)。P21-019: War画面のCrisis由来
    /// 戦争目標表示で、元Claimの状態(Active/Consumed)を示すために使う。
    pub fn display_name(self) -> &'static str {
        match self {
            ClaimStatus::Active => "claim_status.active",
            ClaimStatus::Consumed => "claim_status.consumed",
            ClaimStatus::Invalidated => "claim_status.invalidated",
        }
    }
}

/// 領土請求データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerritorialClaim {
    pub id: ClaimId,
    pub claimant_country: CountryId,
    pub target_state: StateId,
    pub strength: f32, // 0.0 〜 100.0
    pub created_date: String,
    pub is_permanent: bool,
    pub source: ClaimSource,
    /// P21-011: 追加。既存(P21-010以前)Claimは`#[serde(default)]`により`Active`として読む。
    #[serde(default)]
    pub status: ClaimStatus,
}

/// 全領土請求を管理するリソース
#[derive(Resource, Default, Debug)]
pub struct ClaimRegistry {
    pub claims: HashMap<ClaimId, TerritorialClaim>,
    next_id: usize,
}

impl ClaimRegistry {
    /// 次回発行されるClaimIdの値（P21-SAVE-002B: セーブ用の読み取り専用アクセサ。
    /// `WarRegistry`/`BattleRegistry`/`WarJustificationRegistry`の`next_id()`と同じ形）
    pub fn next_id(&self) -> usize {
        self.next_id
    }

    /// 保存されていた要素・次回ID発行値から`ClaimRegistry`を復元する（P21-SAVE-002D）。
    /// `crate::save`のDTO型ではなく、通常のコレクションとカウンタだけを引数にとる
    /// (コアモジュールを`crate::save`へ依存させないため)。
    pub(crate) fn from_saved_parts(
        claims: HashMap<ClaimId, TerritorialClaim>,
        next_id: usize,
    ) -> Self {
        Self { claims, next_id }
    }

    pub fn add_claim(&mut self, mut claim: TerritorialClaim) -> ClaimId {
        claim.id = ClaimId(self.next_id);
        self.next_id += 1;
        self.claims.insert(claim.id, claim.clone());
        claim.id
    }

    pub fn get_claims_by_country(&self, country: CountryId) -> Vec<&TerritorialClaim> {
        self.claims
            .values()
            .filter(|c| c.claimant_country == country)
            .collect()
    }

    pub fn has_claim(&self, country: CountryId, state: StateId) -> bool {
        self.claims
            .values()
            .any(|c| c.claimant_country == country && c.target_state == state)
    }

    /// P21-010: 領土請求が作成可能かを検証する(実際には作成しない)。
    /// 戻り値は表示用の英語原文ではなく安定した翻訳キー(diplomacy_error.claim.*)。
    /// 呼び出し元UI(diplomacy_panel)が`localization::t()`で表示直前に言語へ解決する。
    pub fn can_create_claim(
        &self,
        claimant: CountryId,
        target: CountryId,
        target_state: StateId,
        country_registry: &crate::country::CountryRegistry,
        state_registry: &crate::state::data::StateRegistry,
    ) -> Result<(), &'static str> {
        if claimant == target {
            return Err("diplomacy_error.claim.self");
        }
        // P21-021: 消滅済み(Eliminated)国家は新しいClaimの請求者にも対象にもなれない
        // (§10: Domain APIレベルでの拒否)。`get(..).is_none_or(...)`により、
        // 存在しない場合と消滅済みの場合の両方を同じ`_missing`翻訳キーで扱う
        // (target側は`state.owner_country_id != target`の所有者検証でも構造的に
        // 保護されるが、claimant側にはそれが無いため明示的な検証が必要)。
        if country_registry
            .get(claimant)
            .is_none_or(|c| !c.is_active())
        {
            return Err("diplomacy_error.claim.claimant_missing");
        }
        if country_registry.get(target).is_none_or(|c| !c.is_active()) {
            return Err("diplomacy_error.claim.target_missing");
        }

        let state = state_registry
            .get(target_state)
            .ok_or("diplomacy_error.claim.state_missing")?;
        if state.is_sea {
            return Err("diplomacy_error.claim.state_is_sea");
        }
        if state.owner_country_id == claimant {
            return Err("diplomacy_error.claim.own_state");
        }
        if state.owner_country_id != target {
            return Err("diplomacy_error.claim.state_not_owned_by_target");
        }

        if self
            .claims
            .values()
            .any(|c| c.claimant_country == claimant && c.target_state == target_state)
        {
            return Err("diplomacy_error.claim.duplicate");
        }

        Ok(())
    }

    /// P21-010: 検証込みで領土請求を作成する。検証に失敗した場合、Registry(`claims`・
    /// `next_id`)は一切変更しない(`can_create_claim`は`&self`のみを借用するため、
    /// 失敗時に`add_claim`へ到達し得ない設計自体が不変条件を保証する)。
    #[allow(clippy::too_many_arguments)]
    pub fn create_claim(
        &mut self,
        claimant: CountryId,
        target: CountryId,
        target_state: StateId,
        created_date: String,
        source: ClaimSource,
        country_registry: &crate::country::CountryRegistry,
        state_registry: &crate::state::data::StateRegistry,
    ) -> Result<ClaimId, &'static str> {
        self.can_create_claim(
            claimant,
            target,
            target_state,
            country_registry,
            state_registry,
        )?;

        Ok(self.add_claim(TerritorialClaim {
            id: ClaimId(0), // add_claimが正規のnext_id発行値へ上書きする
            claimant_country: claimant,
            target_state,
            strength: 50.0,
            created_date,
            is_permanent: false,
            source,
            status: ClaimStatus::Active,
        }))
    }

    /// P21-011: Crisisの平和的受諾により、Claimを消費済みへ変更する。
    /// 既にConsumedの場合も含め、対象が存在しない場合のみ`false`を返す
    /// (冪等: 同じ受諾処理が誤って複数回走っても安全)。
    ///
    /// P21-021: 既に`Invalidated`(請求者国家が消滅済み)のClaimは`Consumed`へ
    /// 降格させない——`Invalidated`は請求者自身がもう存在しないという、
    /// `Consumed`(目標達成)より強い終端状態であり、一度確定したら後から
    /// 「達成された」に上書きされてはならない。この不変条件により、
    /// `apply_country_elimination`が請求者消滅によるClaim無効化を行った後、
    /// 同じ請求に紐づくCrisis由来War(消滅国家が主要参加国の別War)が
    /// `execute_peace_settlement`経由で無条件に`mark_consumed`を呼んでも、
    /// 呼び出し順に関わらず安全に`Invalidated`のまま保たれる。
    pub fn mark_consumed(&mut self, claim_id: ClaimId) -> bool {
        let Some(claim) = self.claims.get_mut(&claim_id) else {
            return false;
        };
        if claim.status != ClaimStatus::Invalidated {
            claim.status = ClaimStatus::Consumed;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::{CountryData, CountryRegistry};
    use crate::state::data::{StateData, StateRegistry};

    fn country(id: usize) -> CountryData {
        CountryData {
            id: CountryId(id),
            ..CountryData::default()
        }
    }

    fn land_state(id: usize, owner: usize) -> StateData {
        StateData {
            id: StateId(id),
            owner_country_id: CountryId(owner),
            is_sea: false,
            ..Default::default()
        }
    }

    fn registries() -> (CountryRegistry, StateRegistry) {
        let countries = CountryRegistry {
            countries: vec![country(0), country(1)],
        };
        let states = StateRegistry::build(vec![land_state(0, 0), land_state(1, 1)]);
        (countries, states)
    }

    /// 要求テスト項目1: 有効なClaim作成。
    #[test]
    fn create_claim_succeeds_for_valid_target_state() {
        let (countries, states) = registries();
        let mut registry = ClaimRegistry::default();

        let id = registry
            .create_claim(
                CountryId(0),
                CountryId(1),
                StateId(1),
                "1800/01/01".to_string(),
                ClaimSource::Strategic,
                &countries,
                &states,
            )
            .expect("valid claim must succeed");

        assert_eq!(registry.claims.len(), 1);
        let claim = registry.claims.get(&id).unwrap();
        assert_eq!(claim.claimant_country, CountryId(0));
        assert_eq!(claim.target_state, StateId(1));
    }

    /// 要求テスト項目2: 自国州へのClaim拒否。
    #[test]
    fn create_claim_rejects_own_state() {
        let (countries, states) = registries();
        let mut registry = ClaimRegistry::default();

        let result = registry.create_claim(
            CountryId(0),
            CountryId(1),
            StateId(0), // CountryId(0)自身の州
            "1800/01/01".to_string(),
            ClaimSource::Strategic,
            &countries,
            &states,
        );

        assert_eq!(result, Err("diplomacy_error.claim.own_state"));
        assert!(registry.claims.is_empty());
    }

    /// 要求テスト項目3: 対象国外州へのClaim拒否(targetが所有していない州)。
    #[test]
    fn create_claim_rejects_state_not_owned_by_target() {
        let countries = CountryRegistry {
            countries: vec![country(0), country(1), country(2)],
        };
        let states = StateRegistry::build(vec![land_state(0, 0), land_state(5, 2)]);
        let mut registry = ClaimRegistry::default();

        let result = registry.create_claim(
            CountryId(0),
            CountryId(1), // targetはCountryId(1)だが州はCountryId(2)所有
            StateId(5),
            "1800/01/01".to_string(),
            ClaimSource::Strategic,
            &countries,
            &states,
        );

        assert_eq!(
            result,
            Err("diplomacy_error.claim.state_not_owned_by_target")
        );
    }

    /// 要求テスト項目4: 存在しない州の拒否。
    #[test]
    fn create_claim_rejects_nonexistent_state() {
        let (countries, states) = registries();
        let mut registry = ClaimRegistry::default();

        let result = registry.create_claim(
            CountryId(0),
            CountryId(1),
            StateId(999),
            "1800/01/01".to_string(),
            ClaimSource::Strategic,
            &countries,
            &states,
        );

        assert_eq!(result, Err("diplomacy_error.claim.state_missing"));
    }

    /// 要求テスト項目5: 重複Claim拒否。
    #[test]
    fn create_claim_rejects_duplicate() {
        let (countries, states) = registries();
        let mut registry = ClaimRegistry::default();
        registry
            .create_claim(
                CountryId(0),
                CountryId(1),
                StateId(1),
                "1800/01/01".to_string(),
                ClaimSource::Strategic,
                &countries,
                &states,
            )
            .unwrap();

        let result = registry.create_claim(
            CountryId(0),
            CountryId(1),
            StateId(1),
            "1800/01/02".to_string(),
            ClaimSource::Strategic,
            &countries,
            &states,
        );

        assert_eq!(result, Err("diplomacy_error.claim.duplicate"));
        assert_eq!(
            registry.claims.len(),
            1,
            "duplicate attempt must not add a second claim"
        );
    }

    /// 要求テスト項目6: 不正要求時にnext_id不変。
    #[test]
    fn invalid_claim_request_does_not_advance_next_id() {
        let (countries, states) = registries();
        let mut registry = ClaimRegistry::default();
        let before = registry.next_id();

        let result = registry.create_claim(
            CountryId(0),
            CountryId(1),
            StateId(0), // 自国州、拒否される
            "1800/01/01".to_string(),
            ClaimSource::Strategic,
            &countries,
            &states,
        );

        assert!(result.is_err());
        assert_eq!(registry.next_id(), before);
        assert!(registry.claims.is_empty());
    }

    #[test]
    fn create_claim_rejects_self_target() {
        let (countries, states) = registries();
        let mut registry = ClaimRegistry::default();

        let result = registry.create_claim(
            CountryId(0),
            CountryId(0),
            StateId(1),
            "1800/01/01".to_string(),
            ClaimSource::Strategic,
            &countries,
            &states,
        );

        assert_eq!(result, Err("diplomacy_error.claim.self"));
    }

    #[test]
    fn create_claim_rejects_sea_state() {
        let countries = CountryRegistry {
            countries: vec![country(0), country(1)],
        };
        let mut sea_state = land_state(1, 1);
        sea_state.is_sea = true;
        let states = StateRegistry::build(vec![land_state(0, 0), sea_state]);
        let mut registry = ClaimRegistry::default();

        let result = registry.create_claim(
            CountryId(0),
            CountryId(1),
            StateId(1),
            "1800/01/01".to_string(),
            ClaimSource::Strategic,
            &countries,
            &states,
        );

        assert_eq!(result, Err("diplomacy_error.claim.state_is_sea"));
    }

    /// P21-011要求テスト: 旧形式(`status`フィールド自体が存在しないRON、P21-010以前の
    /// セーブ相当)を読み込んだ場合、`Active`として復元される(`#[serde(default)]`)。
    #[test]
    fn old_format_ron_without_status_field_loads_as_active() {
        let claim = TerritorialClaim {
            id: ClaimId(0),
            claimant_country: CountryId(1),
            target_state: StateId(2),
            strength: 42.0,
            created_date: "1800/01/01".to_string(),
            is_permanent: false,
            source: ClaimSource::Strategic,
            status: ClaimStatus::Consumed,
        };
        let claim_ron = ron::to_string(&claim).unwrap();
        assert!(claim_ron.contains("status"));
        let without_status_field = claim_ron.replacen(",status:Consumed", "", 1);
        assert_ne!(
            without_status_field, claim_ron,
            "test setup must actually remove the status field"
        );
        let restored: TerritorialClaim = ron::from_str(&without_status_field)
            .expect("TerritorialClaim RON missing status must still deserialize (serde default)");
        assert_eq!(restored.status, ClaimStatus::Active);
        assert_eq!(restored.target_state, StateId(2));
    }

    /// P21-018要求テスト(§8): 新しい`ClaimSource::AiExpansion`variantがRON経由で
    /// 往復できる(serde追加のみで旧バリアントの読み込みに影響しないことを確認)。
    #[test]
    fn ai_expansion_claim_source_round_trips_through_ron() {
        let claim = TerritorialClaim {
            id: ClaimId(0),
            claimant_country: CountryId(1),
            target_state: StateId(2),
            strength: 50.0,
            created_date: "1800/01/01".to_string(),
            is_permanent: false,
            source: ClaimSource::AiExpansion,
            status: ClaimStatus::Active,
        };
        let ron = ron::to_string(&claim).unwrap();
        let restored: TerritorialClaim = ron::from_str(&ron).unwrap();
        assert_eq!(restored.source, ClaimSource::AiExpansion);
    }

    /// P21-018要求テスト(§8, 要求49-50への対応): `create_claim`はstrengthを
    /// 呼び出し元から受け取らず常に固定値50.0を書き込むため、NaN・値域外strengthは
    /// この経路からは構造的に発生し得ない(そもそも入力として渡す手段が存在しない)。
    /// AI生成Claim(`ClaimSource::AiExpansion`含む)がこの不変条件を満たすことを
    /// 固定して記録する(P21-017完了報告と同じ「既存APIの実際の形を正とする」方針)。
    #[test]
    fn create_claim_always_produces_finite_in_range_strength() {
        let (countries, states) = registries();
        let mut registry = ClaimRegistry::default();
        let id = registry
            .create_claim(
                CountryId(0),
                CountryId(1),
                StateId(1),
                "1800/01/01".to_string(),
                ClaimSource::AiExpansion,
                &countries,
                &states,
            )
            .unwrap();
        let claim = registry.claims.get(&id).unwrap();
        assert!(claim.strength.is_finite());
        assert!((0.0..=100.0).contains(&claim.strength));
        assert_eq!(claim.strength, 50.0);
    }

    /// `mark_consumed`は対象Claimが存在しない場合`false`を返し、存在する場合は
    /// `Consumed`へ変更して`true`を返す(受諾後の再呼び出しに対しても冪等)。
    #[test]
    fn mark_consumed_transitions_active_to_consumed_and_is_idempotent() {
        let (countries, states) = registries();
        let mut registry = ClaimRegistry::default();
        let id = registry
            .create_claim(
                CountryId(0),
                CountryId(1),
                StateId(1),
                "1800/01/01".to_string(),
                ClaimSource::Strategic,
                &countries,
                &states,
            )
            .unwrap();

        assert!(
            !registry.mark_consumed(ClaimId(999)),
            "unknown id must return false"
        );
        assert!(registry.mark_consumed(id));
        assert_eq!(
            registry.claims.get(&id).unwrap().status,
            ClaimStatus::Consumed
        );
        assert!(
            registry.mark_consumed(id),
            "marking an already-consumed claim again must still return true (idempotent)"
        );
    }

    /// P21-021要求: `Invalidated`(請求者国家消滅)は`Consumed`より強い終端状態
    /// であり、後から`mark_consumed`が呼ばれても降格しない。
    #[test]
    fn mark_consumed_does_not_downgrade_an_invalidated_claim() {
        let (countries, states) = registries();
        let mut registry = ClaimRegistry::default();
        let id = registry
            .create_claim(
                CountryId(0),
                CountryId(1),
                StateId(1),
                "1800/01/01".to_string(),
                ClaimSource::Strategic,
                &countries,
                &states,
            )
            .unwrap();
        registry.claims.get_mut(&id).unwrap().status = ClaimStatus::Invalidated;

        assert!(registry.mark_consumed(id));
        assert_eq!(
            registry.claims.get(&id).unwrap().status,
            ClaimStatus::Invalidated,
            "mark_consumed must not overwrite Invalidated back to Consumed"
        );
    }
}
