#![allow(clippy::field_reassign_with_default)]

use strategy_game::common::CountryId;
use strategy_game::country::CountryData;
use strategy_game::diplomacy::data::{
    ActiveTreaty, DiplomacyRegistry, DiplomaticPairKey, DiplomaticRelation, TreatyType,
};
use strategy_game::diplomacy::proposal::calculate_proposal_score;
use strategy_game::state::data::StateRegistry;

#[test]
fn test_diplomatic_pair_key_normalization() {
    let id1 = CountryId(1);
    let id2 = CountryId(2);

    let key1 = DiplomaticPairKey::new(id1, id2);
    let key2 = DiplomaticPairKey::new(id2, id1);

    assert!(key1.is_some());
    assert_eq!(key1, key2);

    // 自己外交の排除
    let key_self = DiplomaticPairKey::new(id1, id1);
    assert!(key_self.is_none());
}

#[test]
fn test_diplomatic_opinion_clamping() {
    let mut relation = DiplomaticRelation::default();
    relation.opinion = 150.0;
    relation.clamp_opinion();
    assert_eq!(relation.opinion, 100.0);

    relation.opinion = -200.0;
    relation.clamp_opinion();
    assert_eq!(relation.opinion, -100.0);
}

#[test]
fn test_treaty_duplication_prevention_and_removal() {
    let mut relation = DiplomaticRelation::default();
    assert!(!relation.has_treaty(TreatyType::NonAggressionPact));

    relation.treaties.push(ActiveTreaty {
        treaty_type: TreatyType::NonAggressionPact,
        countries: (CountryId(1), CountryId(2)),
        signed_date: "1800/01/01".to_string(),
        is_active: true,
    });

    assert!(relation.has_treaty(TreatyType::NonAggressionPact));

    // 破棄テスト
    let removed = relation.remove_treaty(TreatyType::NonAggressionPact);
    assert!(removed);
    assert!(!relation.has_treaty(TreatyType::NonAggressionPact));
}

#[test]
fn test_deterministic_proposal_score_calculation() {
    let mut proposer = CountryData::default();
    proposer.id = CountryId(1);
    proposer.name = "Proposer".to_string();

    let mut target = CountryData::default();
    target.id = CountryId(2);
    target.name = "Target".to_string();

    let mut relation = DiplomaticRelation::default();
    relation.opinion = 60.0; // 高関係値 (+30.0)

    let state_registry = StateRegistry::default();

    // 1. 高関係値時の不可侵条約提案 -> 承認
    let breakdown = calculate_proposal_score(
        TreatyType::NonAggressionPact,
        &proposer,
        &target,
        &relation,
        &state_registry,
    );

    assert!(breakdown.accepted);
    assert!(breakdown.total_score >= breakdown.required_score);

    // 2. 低関係値時の同盟提案 -> 拒否
    relation.opinion = -50.0; // 低関係値 (-25.0)
    let breakdown_alliance = calculate_proposal_score(
        TreatyType::Alliance,
        &proposer,
        &target,
        &relation,
        &state_registry,
    );

    assert!(!breakdown_alliance.accepted);
    assert!(breakdown_alliance.total_score < breakdown_alliance.required_score);
}

#[test]
fn test_diplomacy_registry_access() {
    let mut reg = DiplomacyRegistry::default();
    let c1 = CountryId(1);
    let c2 = CountryId(2);

    let rel = reg.get_or_create_mut(c1, c2).unwrap();
    rel.opinion = 35.0;

    let retrieved = reg.get(c2, c1).unwrap();
    assert_eq!(retrieved.opinion, 35.0);
}
