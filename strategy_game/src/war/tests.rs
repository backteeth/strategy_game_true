#![cfg(test)]

use crate::common::{ArmyId, CountryId, DivisionId, StateId};
use crate::military::data::{
    ArmyStatus, ArmyUnit, DivisionDefinition, DivisionSize, DivisionType, MilitaryRegistry,
};
use crate::state::data::{StateData, StateRegistry};
use crate::war::combat::process_combat;
use crate::war::data::{War, WarRegistry};
use crate::war::peace::{PeaceDemand, execute_peace_treaty};
use crate::war::war_score::process_war_score;

fn setup() -> (MilitaryRegistry, WarRegistry, StateRegistry) {
    let mut mil_reg = MilitaryRegistry::default();
    let def = DivisionDefinition {
        id: DivisionId(1),
        name: "Inf".to_string(),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        required_manpower: 10000,
        required_equipment: 100.0,
        recruitment_days: 10,
        movement_speed: 1.0,
        attack: 10.0,
        defense: 10.0,
        breakthrough: 1.0,
        organization: 100.0,
        morale: 100.0,
        supply_usage: 1.0,
        maintenance_cost: 1.0,
    };
    mil_reg.definitions.insert(DivisionId(1), def);

    let mut war_reg = WarRegistry::default();
    let war = War {
        id: crate::common::WarId(1),
        name: "Test War".to_string(),
        start_date: "1936/01/01".to_string(),
        war_score: 0.0,
        attackers: vec![CountryId(1)].into_iter().collect(),
        defenders: vec![CountryId(2)].into_iter().collect(),
        war_goals: vec![],
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: std::collections::HashSet::new(),
        status: crate::war::data::WarStatus::Active,
    };
    war_reg.wars.insert(war.id, war);

    let s1 = StateData {
        id: StateId(1),
        owner_country_id: CountryId(1),
        ..Default::default()
    };

    let s2 = StateData {
        id: StateId(2),
        owner_country_id: CountryId(2),
        ..Default::default()
    };

    let state_reg = StateRegistry::build(vec![s1, s2]);

    (mil_reg, war_reg, state_reg)
}

#[test]
fn test_combat_resolution() {
    let (mut mil_reg, war_reg, _) = setup();

    // Two armies in the same state
    let a1 = ArmyUnit {
        id: ArmyId(1),
        owner: CountryId(1),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(1),
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 10000,
        max_manpower: 10000,
        equipment: 100.0,
        max_equipment: 100.0,
        organization: 100.0,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: ArmyStatus::Idle,
        def_id: DivisionId(1),
    };

    let mut a2 = a1.clone();
    a2.id = ArmyId(2);
    a2.owner = CountryId(2); // Hostile country

    mil_reg.add_army(a1); // id becomes 0
    mil_reg.add_army(a2); // id becomes 1

    process_combat(&mut mil_reg, &war_reg);

    let a1_ref = mil_reg.armies.get(&ArmyId(0)).unwrap();
    let a2_ref = mil_reg.armies.get(&ArmyId(1)).unwrap();

    // Both should take damage
    assert_eq!(a1_ref.status, ArmyStatus::Fighting);
    assert!(a1_ref.manpower < 10000);
    assert_eq!(a2_ref.status, ArmyStatus::Fighting);
    assert!(a2_ref.manpower < 10000);
}

#[test]
fn test_war_score_calculation() {
    let (_, mut war_reg, mut state_reg) = setup();

    // Attacker (1) occupies defender's (2) state (StateId(2))
    let state2 = state_reg.get_mut(StateId(2)).unwrap();
    state2.controller_country = Some(CountryId(1));

    process_war_score(&state_reg, &mut war_reg);

    let war = war_reg.wars.get(&crate::common::WarId(1)).unwrap();
    assert_eq!(war.war_score, 10.0); // 1 state occupied = 10 score
}

#[test]
fn test_peace_treaty() {
    let (_, mut war_reg, mut state_reg) = setup();

    // Execute peace: Country 1 annexes StateId 2
    let demands = vec![PeaceDemand::AnnexState(StateId(2))];
    let result = execute_peace_treaty(
        crate::common::WarId(1),
        CountryId(1),
        demands,
        &mut state_reg,
        &mut war_reg,
    );

    assert!(result.is_ok());

    // War should be removed
    assert!(war_reg.wars.is_empty());

    // State 2 owner should be Country 1
    let state2 = state_reg.get(StateId(2)).unwrap();
    assert_eq!(state2.owner_country_id, CountryId(1));
}

// ── Phase 14 Tests ─────────────────────────────────────────────────────────
use crate::country::{CountryData, CountryRegistry};
use crate::diplomacy::data::{ActiveTreaty, DiplomacyRegistry, TreatyType};
use crate::war::justification::WarJustificationRegistry;

fn setup_phase14_env() -> (
    CountryRegistry,
    StateRegistry,
    DiplomacyRegistry,
    WarJustificationRegistry,
    WarRegistry,
) {
    let c1 = CountryData {
        id: CountryId(1),
        name: "Country 1".to_string(),
        ..Default::default()
    };
    let c2 = CountryData {
        id: CountryId(2),
        name: "Country 2".to_string(),
        ..Default::default()
    };
    let c3 = CountryData {
        id: CountryId(3),
        name: "Country 3".to_string(),
        ..Default::default()
    };

    let mut country_reg = CountryRegistry::default();
    country_reg.countries.push(c1);
    country_reg.countries.push(c2);
    country_reg.countries.push(c3);

    let s1 = StateData {
        id: StateId(1),
        name: "State 1".to_string(),
        owner_country_id: CountryId(1),
        ..Default::default()
    };
    let s2 = StateData {
        id: StateId(2),
        name: "State 2".to_string(),
        owner_country_id: CountryId(2),
        ..Default::default()
    };
    let state_reg = StateRegistry::build(vec![s1, s2]);

    let diplo_reg = DiplomacyRegistry::default();
    let just_reg = WarJustificationRegistry::default();
    let war_reg = WarRegistry::default();

    (country_reg, state_reg, diplo_reg, just_reg, war_reg)
}

#[test]
fn test_cannot_justify_against_self() {
    let (c_reg, s_reg, d_reg, mut j_reg, _) = setup_phase14_env();
    let res = j_reg.start_justification(
        CountryId(1),
        CountryId(1),
        StateId(1),
        "1936/01/01".to_string(),
        &c_reg,
        &s_reg,
        &d_reg,
    );
    assert!(res.is_err());
    assert_eq!(res.unwrap_err(), "Cannot justify war against own country");
}

#[test]
fn test_cannot_justify_unowned_state() {
    let (c_reg, s_reg, d_reg, mut j_reg, _) = setup_phase14_env();
    // Country 2 does not own StateId(1) (Country 1 owns it)
    let res = j_reg.start_justification(
        CountryId(1),
        CountryId(2),
        StateId(1),
        "1936/01/01".to_string(),
        &c_reg,
        &s_reg,
        &d_reg,
    );
    assert!(res.is_err());
    assert_eq!(
        res.unwrap_err(),
        "Target state is not owned by target country"
    );
}

#[test]
fn test_cannot_justify_against_ally() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, _) = setup_phase14_env();
    if let Some(rel) = d_reg.get_or_create_mut(CountryId(1), CountryId(2)) {
        rel.treaties.push(ActiveTreaty {
            treaty_type: TreatyType::Alliance,
            countries: (CountryId(1), CountryId(2)),
            signed_date: "1936/01/01".to_string(),
            is_active: true,
        });
    }
    let res = j_reg.start_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        &c_reg,
        &s_reg,
        &d_reg,
    );
    assert!(res.is_err());
    assert_eq!(res.unwrap_err(), "Cannot justify war against an ally");
}

#[test]
fn test_cannot_justify_against_nap_partner() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, _) = setup_phase14_env();
    if let Some(rel) = d_reg.get_or_create_mut(CountryId(1), CountryId(2)) {
        rel.treaties.push(ActiveTreaty {
            treaty_type: TreatyType::NonAggressionPact,
            countries: (CountryId(1), CountryId(2)),
            signed_date: "1936/01/01".to_string(),
            is_active: true,
        });
    }
    let res = j_reg.start_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        &c_reg,
        &s_reg,
        &d_reg,
    );
    assert!(res.is_err());
    assert_eq!(
        res.unwrap_err(),
        "Cannot justify war against non-aggression pact partner"
    );
}

#[test]
fn test_cannot_duplicate_justification() {
    let (c_reg, s_reg, d_reg, mut j_reg, _) = setup_phase14_env();
    let res1 = j_reg.start_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        &c_reg,
        &s_reg,
        &d_reg,
    );
    assert!(res1.is_ok());

    let res2 = j_reg.start_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        &c_reg,
        &s_reg,
        &d_reg,
    );
    assert!(res2.is_err());
    assert_eq!(
        res2.unwrap_err(),
        "Justification for this state already exists"
    );
}

#[test]
fn test_justification_progress_timing() {
    let (c_reg, s_reg, d_reg, mut j_reg, _) = setup_phase14_env();
    let id = j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();

    let just = j_reg.justifications.get(&id).unwrap();
    assert_eq!(just.days_passed, 0);
    assert!(!just.is_ready);

    // Process 15 days
    for _ in 0..15 {
        j_reg.process_daily_justifications(&s_reg);
    }
    let just = j_reg.justifications.get(&id).unwrap();
    assert_eq!(just.days_passed, 15);
    assert!(!just.is_ready);
}

#[test]
fn test_justification_completion_after_required_days() {
    let (c_reg, s_reg, d_reg, mut j_reg, _) = setup_phase14_env();
    let id = j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();

    // Process 30 days (default required days)
    for _ in 0..30 {
        j_reg.process_daily_justifications(&s_reg);
    }

    let just = j_reg.justifications.get(&id).unwrap();
    assert_eq!(just.days_passed, 30);
    assert!(just.is_ready);

    let ready = j_reg.get_ready_justification(CountryId(1), CountryId(2), StateId(2));
    assert!(ready.is_some());
}

#[test]
fn test_cannot_declare_war_before_justification_complete() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    let _id = j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();

    // Progress 10 days (not ready yet)
    for _ in 0..10 {
        j_reg.process_daily_justifications(&s_reg);
    }

    let res = w_reg.declare_war(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/11".to_string(),
        &c_reg,
        &s_reg,
        &mut d_reg,
        &mut j_reg,
    );

    assert!(res.is_err());
    assert_eq!(
        res.unwrap_err(),
        "No completed war justification for this state"
    );
}

#[test]
fn test_can_declare_war_after_justification_complete() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    let _id = j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();

    // Progress 30 days
    for _ in 0..30 {
        j_reg.process_daily_justifications(&s_reg);
    }

    let res = w_reg.declare_war(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/31".to_string(),
        &c_reg,
        &s_reg,
        &mut d_reg,
        &mut j_reg,
    );

    assert!(res.is_ok());
}

#[test]
fn test_justification_consumed_on_war_declaration() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    let _id = j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();

    for _ in 0..30 {
        j_reg.process_daily_justifications(&s_reg);
    }

    assert!(
        j_reg
            .get_ready_justification(CountryId(1), CountryId(2), StateId(2))
            .is_some()
    );

    w_reg
        .declare_war(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/31".to_string(),
            &c_reg,
            &s_reg,
            &mut d_reg,
            &mut j_reg,
        )
        .unwrap();

    // Justification should be consumed (deleted)
    assert!(
        j_reg
            .get_ready_justification(CountryId(1), CountryId(2), StateId(2))
            .is_none()
    );
    assert!(j_reg.justifications.is_empty());
}

#[test]
fn test_war_data_created_on_declaration() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();

    for _ in 0..30 {
        j_reg.process_daily_justifications(&s_reg);
    }

    let war_id = w_reg
        .declare_war(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/31".to_string(),
            &c_reg,
            &s_reg,
            &mut d_reg,
            &mut j_reg,
        )
        .unwrap();

    assert_eq!(w_reg.wars.len(), 1);
    let war = w_reg.wars.get(&war_id).unwrap();
    assert_eq!(war.start_date, "1936/01/31");
    assert_eq!(war.war_goals.len(), 1);
    assert_eq!(war.war_goals[0].target_states, vec![StateId(2)]);
}

#[test]
fn test_attacker_and_defender_registered_correctly() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();

    for _ in 0..30 {
        j_reg.process_daily_justifications(&s_reg);
    }

    let war_id = w_reg
        .declare_war(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/31".to_string(),
            &c_reg,
            &s_reg,
            &mut d_reg,
            &mut j_reg,
        )
        .unwrap();

    let war = w_reg.wars.get(&war_id).unwrap();
    assert!(war.attackers.contains(&CountryId(1)));
    assert!(!war.attackers.contains(&CountryId(2)));
    assert!(war.defenders.contains(&CountryId(2)));
    assert!(!war.defenders.contains(&CountryId(1)));
}

#[test]
fn test_cannot_start_duplicate_war() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();

    // First war declaration
    j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();
    for _ in 0..30 {
        j_reg.process_daily_justifications(&s_reg);
    }
    w_reg
        .declare_war(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/31".to_string(),
            &c_reg,
            &s_reg,
            &mut d_reg,
            &mut j_reg,
        )
        .unwrap();

    // Second war declaration attempt for same countries
    j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/02/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();
    for _ in 0..30 {
        j_reg.process_daily_justifications(&s_reg);
    }
    let res = w_reg.declare_war(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/03/01".to_string(),
        &c_reg,
        &s_reg,
        &mut d_reg,
        &mut j_reg,
    );

    assert!(res.is_err());
    assert_eq!(res.unwrap_err(), "Countries are already at war");
}

#[test]
fn test_are_countries_at_war_true_for_enemies() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();
    for _ in 0..30 {
        j_reg.process_daily_justifications(&s_reg);
    }
    w_reg
        .declare_war(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/31".to_string(),
            &c_reg,
            &s_reg,
            &mut d_reg,
            &mut j_reg,
        )
        .unwrap();

    assert!(w_reg.are_countries_at_war(CountryId(1), CountryId(2)));
    assert!(w_reg.are_countries_at_war(CountryId(2), CountryId(1)));
}

#[test]
fn test_are_countries_at_war_false_for_allies_or_neutrals() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();
    for _ in 0..30 {
        j_reg.process_daily_justifications(&s_reg);
    }
    w_reg
        .declare_war(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/31".to_string(),
            &c_reg,
            &s_reg,
            &mut d_reg,
            &mut j_reg,
        )
        .unwrap();

    // Neutral country 3 is not at war with country 1 or 2
    assert!(!w_reg.are_countries_at_war(CountryId(1), CountryId(3)));
    assert!(!w_reg.are_countries_at_war(CountryId(2), CountryId(3)));
    assert!(!w_reg.are_countries_at_war(CountryId(1), CountryId(1)));
}

#[test]
fn test_are_countries_at_war_handles_invalid_ids() {
    let (_, _, _, _, w_reg) = setup_phase14_env();
    assert!(!w_reg.are_countries_at_war(CountryId(999), CountryId(1000)));
    assert!(!w_reg.are_countries_at_war(CountryId(0), CountryId(0)));
}

#[test]
fn test_war_declaration_determinism() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    j_reg
        .start_justification(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/01".to_string(),
            &c_reg,
            &s_reg,
            &d_reg,
        )
        .unwrap();
    for _ in 0..30 {
        j_reg.process_daily_justifications(&s_reg);
    }
    let war_id1 = w_reg
        .declare_war(
            CountryId(1),
            CountryId(2),
            StateId(2),
            "1936/01/31".to_string(),
            &c_reg,
            &s_reg,
            &mut d_reg,
            &mut j_reg,
        )
        .unwrap();

    assert_eq!(war_id1.0, 0);
}
