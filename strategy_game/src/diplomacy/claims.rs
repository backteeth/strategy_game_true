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
}
