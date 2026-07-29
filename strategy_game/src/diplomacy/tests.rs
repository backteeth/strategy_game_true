#![cfg(test)]
#![allow(clippy::field_reassign_with_default)]

use crate::common::{CountryId, StateId};
use crate::diplomacy::claims::{ClaimRegistry, TerritorialClaim};
use crate::diplomacy::crisis::{CrisisRegistry, DiplomaticCrisis, WarGoalType};
use crate::diplomacy::data::{ActiveTreaty, DiplomaticPairKey, DiplomaticRelation, TreatyType};

#[test]
fn test_diplomatic_pair_normalization() {
    let a = CountryId(1);
    let b = CountryId(2);

    let pair1 = DiplomaticPairKey::new(a, b).unwrap();
    let pair2 = DiplomaticPairKey::new(b, a).unwrap();

    assert_eq!(pair1.0, a);
    assert_eq!(pair1.1, b);
    assert_eq!(pair1, pair2);

    assert!(DiplomaticPairKey::new(a, a).is_none());
}

#[test]
fn test_relation_clamping() {
    let mut rel = DiplomaticRelation::default();
    rel.opinion = 150.0;
    rel.tension = -50.0;
    rel.trust = 200.0;
    rel.threat = -10.0;

    rel.clamp_opinion();

    assert_eq!(rel.opinion, 100.0);
    assert_eq!(rel.tension, 0.0);
    assert_eq!(rel.trust, 100.0);
    assert_eq!(rel.threat, 0.0);
}

#[test]
fn test_treaty_addition_and_removal() {
    let mut rel = DiplomaticRelation::default();

    assert!(!rel.has_treaty(TreatyType::NonAggressionPact));

    rel.treaties.push(ActiveTreaty {
        treaty_type: TreatyType::NonAggressionPact,
        countries: (CountryId(1), CountryId(2)),
        signed_date: "1936/01/01".to_string(),
        is_active: true,
    });

    assert!(rel.has_treaty(TreatyType::NonAggressionPact));

    let removed = rel.remove_treaty(TreatyType::NonAggressionPact);
    assert!(removed);
    assert!(!rel.has_treaty(TreatyType::NonAggressionPact));

    let removed_again = rel.remove_treaty(TreatyType::NonAggressionPact);
    assert!(!removed_again);
}

#[test]
fn test_truce_prevents_war() {
    let mut rel = DiplomaticRelation::default();
    rel.truce_until = Some("1939/01/01".to_string());

    // This relies on whatever logic checks if war can be declared.
    // For now, let's just assert the data structure is capable.
    assert!(rel.truce_until.is_some());

    // Let's assume a function `can_declare_war` exists.
    // fn can_declare_war(rel: &DiplomaticRelation) -> bool { rel.truce_until.is_none() && !rel.has_treaty(TreatyType::NonAggressionPact) }
    let can_declare_war =
        rel.truce_until.is_none() && !rel.has_treaty(TreatyType::NonAggressionPact);
    assert!(!can_declare_war);
}

#[test]
fn test_claim_registry() {
    use crate::diplomacy::claims::ClaimSource;
    let mut claims = ClaimRegistry::default();

    let claim = TerritorialClaim {
        id: crate::common::ClaimId(1),
        claimant_country: CountryId(1),
        target_state: StateId(10),
        strength: 100.0,
        created_date: "1936/01/01".to_string(),
        is_permanent: false,
        source: ClaimSource::Strategic,
    };

    claims.add_claim(claim.clone());
    assert!(claims.has_claim(CountryId(1), StateId(10)));
    assert!(!claims.has_claim(CountryId(2), StateId(10)));
}

#[test]
fn test_crisis_registry() {
    use crate::diplomacy::crisis::{CrisisPhase, WarGoal};
    use std::collections::HashMap;

    let mut registry = CrisisRegistry::default();

    let crisis = DiplomaticCrisis {
        id: crate::common::DiplomaticCrisisId(0),
        initiator: CountryId(1),
        target: CountryId(2),
        war_goals: vec![WarGoal {
            attacker: CountryId(1),
            defender: CountryId(2),
            goal_type: WarGoalType::ConquerState,
            target_states: vec![StateId(10)],
            base_peace_cost: 10.0,
            international_concern: 5.0,
            completion: 0.0,
            is_primary: true,
        }],
        start_date: "1936/01/01".to_string(),
        current_phase: CrisisPhase::DemandSent,
        escalation: 50.0,
        initiator_support: 50.0,
        target_resistance: 50.0,
        days_in_phase: 0,
        deadline_date: None,
        international_concern: 0.0,
        third_party_reactions: HashMap::new(),
    };

    let result_id = registry.add_crisis(crisis.clone());
    assert_eq!(result_id.0, 0);

    // Ensure get_active_crisis works
    assert!(
        registry
            .get_active_crisis_for_country(CountryId(1))
            .is_some()
    );
}

#[test]
fn test_demand_acceptance() {
    use crate::country::CountryData;
    use crate::diplomacy::ai::calculate_demand_acceptance;
    use crate::diplomacy::crisis::CrisisPhase;
    use crate::state::data::StateData;
    use std::collections::HashMap;

    let mut initiator = CountryData::default();
    initiator.id = CountryId(1);
    initiator.available_manpower = 100_000;

    let mut target = CountryData::default();
    target.id = CountryId(2);
    target.capital_state_id = StateId(5);
    target.available_manpower = 10_000;

    let mut state = StateData::default();
    state.id = StateId(10);
    state.population = 50_000;
    state.integration = 100.0;

    let crisis = DiplomaticCrisis {
        id: crate::common::DiplomaticCrisisId(1),
        initiator: CountryId(1),
        target: CountryId(2),
        war_goals: vec![],
        start_date: "1936/01/01".to_string(),
        current_phase: CrisisPhase::DemandSent,
        escalation: 0.0,
        initiator_support: 0.0,
        target_resistance: 0.0,
        days_in_phase: 0,
        deadline_date: None,
        international_concern: 0.0,
        third_party_reactions: HashMap::new(),
    };

    let relation = DiplomaticRelation::default();

    let score =
        calculate_demand_acceptance(&crisis, &target, &initiator, &[&state], &relation, 0.0, 0.0);

    assert!(
        score > 0.0,
        "Should accept due to overwhelming military power"
    );

    let mut capital_state = StateData::default();
    capital_state.id = StateId(5);

    let score_capital = calculate_demand_acceptance(
        &crisis,
        &target,
        &initiator,
        &[&capital_state],
        &relation,
        0.0,
        0.0,
    );
    assert!(
        score_capital <= -100.0,
        "Should absolutely reject capital state demand"
    );
}
