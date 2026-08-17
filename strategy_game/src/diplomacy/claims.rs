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
        if country_registry.get(claimant).is_none() {
            return Err("diplomacy_error.claim.claimant_missing");
        }
        if country_registry.get(target).is_none() {
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
        }))
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
}
