#![cfg(test)]

use crate::common::{CountryId, DivisionDefinitionId, DivisionId, StateId};
use crate::military::data::{
    DivisionStatus, Division, DivisionDefinition, DivisionSize, DivisionType, MilitaryRegistry,
};
use crate::state::data::{StateData, StateRegistry};
use crate::war::data::{War, WarRegistry};
use crate::war::war_score::process_war_score;

fn setup() -> (MilitaryRegistry, WarRegistry, StateRegistry) {
    let mut mil_reg = MilitaryRegistry::default();
    let def = DivisionDefinition {
        id: DivisionDefinitionId(1),
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
    mil_reg.definitions.insert(DivisionDefinitionId(1), def);

    let mut war_reg = WarRegistry::default();
    let war = War {
        id: crate::common::WarId(1),
        name: "Test War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attackers: vec![CountryId(1)].into_iter().collect(),
        defenders: vec![CountryId(2)].into_iter().collect(),
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
    // 旧 process_combat のテストを combat_calc ベースで更新
    use crate::military::combat_calc::resolve_combat_day;

    let (mil_reg, _, _) = setup();

    // Two divisions
    let a1 = Division {
        id: DivisionId(1),
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
        status: DivisionStatus::Idle,
        def_id: DivisionDefinitionId(1),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    };

    let mut a2 = a1.clone();
    a2.id = DivisionId(2);
    a2.owner = CountryId(2);

    // 1日分の戦闘計算で損失が発生する
    let (atk_loss, _, def_loss, _) = resolve_combat_day(&a1, &a2, 0);
    assert!(atk_loss > 0 || def_loss > 0, "Combat should cause damage");

    // 組織率が適用された後の戦力は元より低い
    let new_manpower_a1 = a1.manpower.saturating_sub(atk_loss);
    let new_manpower_a2 = a2.manpower.saturating_sub(def_loss);
    assert!(new_manpower_a1 < 10000 || new_manpower_a2 < 10000);

    let _ = mil_reg; // suppress unused warning
}

#[test]
fn test_war_score_calculation() {
    let (_, mut war_reg, mut state_reg) = setup();

    // Attacker (1) occupies defender's (2) state (StateId(2))
    let state2 = state_reg.get_mut(StateId(2)).unwrap();
    state2.controller_country = Some(CountryId(1));

    process_war_score(&state_reg, &mut war_reg);

    let war = war_reg.wars.get(&crate::common::WarId(1)).unwrap();
    // 新仕様：目標点や占領点の計算により戦勝点が正の値になる
    assert!(war.war_score > 0.0);
}

#[test]
fn test_peace_treaty_basic() {
    let (_, mut war_reg, mut state_reg) = setup();
    let mut mil_reg = MilitaryRegistry::default();
    let mut battle_reg = crate::military::battle::BattleRegistry::default();
    let mut dip_reg = DiplomacyRegistry::default();
    let mut frontline_reg = crate::war::frontline::FrontlineRegistry::default();

    let result = crate::war::peace::execute_peace_settlement(
        crate::common::WarId(1),
        crate::war::peace::PeaceTerm::CedeWarGoalRegion,
        "Test Victory",
        "1936/02/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut frontline_reg,
    );

    assert!(result.is_ok());

    // 戦争は Ended/AttackerVictory 状態になるが履歴として保持
    let war = war_reg.wars.get(&crate::common::WarId(1)).unwrap();
    assert_eq!(war.status, crate::war::data::WarStatus::AttackerVictory);
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
    // P20-009: エラー戻り値は表示用英語原文ではなく安定した翻訳キー。
    assert_eq!(res.unwrap_err(), "war_error.justify.self");
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
    // P20-009: エラー戻り値は表示用英語原文ではなく安定した翻訳キー。
    assert_eq!(res.unwrap_err(), "war_error.justify.state_not_owned");
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
    // P20-009: エラー戻り値は表示用英語原文ではなく安定した翻訳キー。
    assert_eq!(res.unwrap_err(), "war_error.justify.ally");
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
    // P20-009: エラー戻り値は表示用英語原文ではなく安定した翻訳キー。
    assert_eq!(res.unwrap_err(), "war_error.justify.nap");
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
    // P20-009: エラー戻り値は表示用英語原文ではなく安定した翻訳キー。
    assert_eq!(res2.unwrap_err(), "war_error.justify.duplicate");
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
    // P20-009: エラー戻り値は表示用英語原文ではなく安定した翻訳キー。
    assert_eq!(res.unwrap_err(), "war_error.declare.no_justification");
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
    // P20-009: エラー戻り値は表示用英語原文ではなく安定した翻訳キー。
    assert_eq!(res.unwrap_err(), "war_error.declare.already_at_war");
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

// ── Phase 16 Tests ─────────────────────────────────────────────────────────

#[test]
fn test_phase16_war_score_breakdown_and_clamping() {
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};
    use crate::war::war_score::calculate_war_score;

    let s1 = StateData {
        id: StateId(1),
        owner_country_id: CountryId(1),
        controller_country: Some(CountryId(1)),
        ..Default::default()
    };
    let s2 = StateData {
        id: StateId(2),
        owner_country_id: CountryId(2),
        controller_country: Some(CountryId(1)), // Target state occupied by attacker
        ..Default::default()
    };
    let state_reg = StateRegistry::build(vec![s1, s2]);

    let goal = WarGoal {
        attacker: CountryId(1),
        defender: CountryId(2),
        goal_type: WarGoalType::ConquerState,
        target_states: vec![StateId(2)],
        base_peace_cost: 20.0,
        international_concern: 10.0,
        completion: 0.0,
        is_primary: true,
    };

    let mut war = War {
        id: crate::common::WarId(10),
        name: "Test Score War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attackers: vec![CountryId(1)].into_iter().collect(),
        defenders: vec![CountryId(2)].into_iter().collect(),
        war_goals: vec![goal],
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: std::collections::HashSet::new(),
        status: crate::war::data::WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 3,
        won_defender_battles: 1,
        processed_battle_ids: std::collections::HashSet::new(),
    };

    let bd = calculate_war_score(&war, &state_reg);

    // Goal +40, Attacker Occupy (1/1) +40, Defender Occupy (0/1) 0, Battle 5*(3-1)=+10
    // Total = 40 + 40 + 0 + 10 = 90
    assert_eq!(bd.war_goal_score, 40);
    assert_eq!(bd.attacker_occupation_score, 40);
    assert_eq!(bd.defender_occupation_score, 0);
    assert_eq!(bd.battle_score, 10);
    assert_eq!(bd.total_score, 90);

    // Extreme battle score clamping test
    war.won_attacker_battles = 100;
    let bd2 = calculate_war_score(&war, &state_reg);
    assert_eq!(bd2.battle_score, 20); // Clamped to 20
    assert_eq!(bd2.total_score, 100); // Clamped to 100
}

#[test]
fn test_phase16_battle_results_sync_and_deduplication() {
    use crate::military::battle::{Battle, BattleRegistry, BattleStatus};
    use crate::war::combat::sync_battle_results_to_wars;

    let mut war_reg = WarRegistry::default();
    let war = War {
        id: crate::common::WarId(1),
        name: "Test Sync War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attackers: vec![CountryId(1)].into_iter().collect(),
        defenders: vec![CountryId(2)].into_iter().collect(),
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
    };
    war_reg.wars.insert(war.id, war);

    let mut battle_reg = BattleRegistry::default();
    let b1 = Battle {
        id: crate::common::BattleId(101),
        war_id: crate::common::WarId(1),
        state_id: StateId(2),
        attacker_country: CountryId(1),
        defender_country: CountryId(2),
        attacker_division_ids: vec![crate::common::DivisionId(1)],
        defender_division_ids: vec![crate::common::DivisionId(2)],
        attacker_origins: [(crate::common::DivisionId(1), StateId(1))]
            .into_iter()
            .collect(),
        start_date: "1936/01/01".to_string(),
        elapsed_days: 2,
        status: BattleStatus::AttackerWon,
    };
    battle_reg.battles.insert(b1.id, b1);

    // Sync 1st time
    sync_battle_results_to_wars(&battle_reg, &mut war_reg);
    let war = war_reg.wars.get(&crate::common::WarId(1)).unwrap();
    assert_eq!(war.won_attacker_battles, 1);
    assert_eq!(war.won_defender_battles, 0);

    // Sync 2nd time (Deduplication check)
    sync_battle_results_to_wars(&battle_reg, &mut war_reg);
    let war2 = war_reg.wars.get(&crate::common::WarId(1)).unwrap();
    assert_eq!(war2.won_attacker_battles, 1); // Remains 1
}

#[test]
fn test_phase16_capitulation_rules() {
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};
    use crate::military::data::{DivisionStatus, Division, DivisionSize, DivisionType};
    use crate::war::capitulation::{CapitulationResult, evaluate_war_capitulation};

    let goal = WarGoal {
        attacker: CountryId(1),
        defender: CountryId(2),
        goal_type: WarGoalType::ConquerState,
        target_states: vec![StateId(2)],
        base_peace_cost: 20.0,
        international_concern: 10.0,
        completion: 0.0,
        is_primary: true,
    };

    let s1 = StateData {
        id: StateId(1),
        owner_country_id: CountryId(1),
        controller_country: Some(CountryId(1)),
        ..Default::default()
    };
    let s2 = StateData {
        id: StateId(2),
        owner_country_id: CountryId(2),
        controller_country: Some(CountryId(1)), // Occupied by attacker
        ..Default::default()
    };
    let state_reg = StateRegistry::build(vec![s1, s2]);

    let mut mil_reg = MilitaryRegistry::default();
    // Country 1 has an active division
    let a1 = Division {
        id: crate::common::DivisionId(1),
        owner: CountryId(1),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(1),
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 5000,
        max_manpower: 10000,
        equipment: 50.0,
        max_equipment: 100.0,
        organization: 50.0,
        max_organization: 100.0,
        morale: 50.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: crate::common::DivisionDefinitionId(1),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    };
    mil_reg.divisions.insert(a1.id, a1);

    let war = War {
        id: crate::common::WarId(1),
        name: "Capitulation War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 50.0,
        attackers: vec![CountryId(1)].into_iter().collect(),
        defenders: vec![CountryId(2)].into_iter().collect(),
        war_goals: vec![goal],
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
    };

    // Target state is occupied AND Country 2 has no ready division -> Defender Capitulates
    let res = evaluate_war_capitulation(&war, &state_reg, &mil_reg);
    assert_eq!(res, CapitulationResult::DefenderCapitulated);
}

#[test]
fn test_phase16_peace_offer_validation() {
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};
    use crate::war::peace::{PeaceOffer, PeaceTerm, can_accept_peace_offer};

    let goal = WarGoal {
        attacker: CountryId(1),
        defender: CountryId(2),
        goal_type: WarGoalType::ConquerState,
        target_states: vec![StateId(2)],
        base_peace_cost: 20.0,
        international_concern: 10.0,
        completion: 0.0,
        is_primary: true,
    };

    let mut war_reg = WarRegistry::default();
    let war = War {
        id: crate::common::WarId(1),
        name: "Peace Validation War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 50.0,
        attackers: vec![CountryId(1)].into_iter().collect(),
        defenders: vec![CountryId(2)].into_iter().collect(),
        war_goals: vec![goal],
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
    };
    war_reg.wars.insert(war.id, war);

    // 1. White Peace before 30 days should fail
    let offer_early_wp = PeaceOffer {
        war_id: crate::common::WarId(1),
        proposer_country_id: CountryId(1),
        recipient_country_id: CountryId(2),
        term: PeaceTerm::WhitePeace,
        created_date: "1936/01/10".to_string(), // Only 9 days
    };
    assert!(can_accept_peace_offer(&offer_early_wp, &war_reg, None, None, "1936/01/10").is_err());

    // 2. White Peace after 30 days should succeed
    let offer_valid_wp = PeaceOffer {
        war_id: crate::common::WarId(1),
        proposer_country_id: CountryId(1),
        recipient_country_id: CountryId(2),
        term: PeaceTerm::WhitePeace,
        created_date: "1936/02/05".to_string(), // >30 days
    };
    assert!(can_accept_peace_offer(&offer_valid_wp, &war_reg, None, None, "1936/02/05").is_ok());

    // 3. Cede region with score 50 (< 60) and no capitulation should fail
    let offer_cede = PeaceOffer {
        war_id: crate::common::WarId(1),
        proposer_country_id: CountryId(1),
        recipient_country_id: CountryId(2),
        term: PeaceTerm::CedeWarGoalRegion,
        created_date: "1936/02/05".to_string(),
    };
    assert!(can_accept_peace_offer(&offer_cede, &war_reg, None, None, "1936/02/05").is_err());

    // 4. Update war score to 70 (>= 60) -> Cede region succeeds
    war_reg
        .wars
        .get_mut(&crate::common::WarId(1))
        .unwrap()
        .war_score = 70.0;
    assert!(can_accept_peace_offer(&offer_cede, &war_reg, None, None, "1936/02/05").is_ok());
}

#[test]
fn test_phase16_sea_state_exclusion() {
    use crate::war::war_score::calculate_war_score;

    let s1 = StateData {
        id: StateId(1),
        owner_country_id: CountryId(1),
        controller_country: Some(CountryId(1)),
        is_sea: false,
        ..Default::default()
    };
    let s2 = StateData {
        id: StateId(2),
        owner_country_id: CountryId(2),
        controller_country: Some(CountryId(1)),
        is_sea: false,
        ..Default::default()
    };
    let s_sea = StateData {
        id: StateId(3),
        owner_country_id: CountryId(2),
        controller_country: Some(CountryId(1)),
        is_sea: true,
        ..Default::default()
    };
    let mut state_reg = StateRegistry::build(vec![s1, s2, s_sea]);

    let war = War {
        id: crate::common::WarId(1),
        name: "Sea Test War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attackers: vec![CountryId(1)].into_iter().collect(),
        defenders: vec![CountryId(2)].into_iter().collect(),
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
    };

    let bd = calculate_war_score(&war, &state_reg);
    // Sea state s_sea (StateId 3) is ignored; only s2 is counted -> 40 * (1/1) = 40
    assert_eq!(bd.attacker_occupation_score, 40);

    // Transferring sea region should fail
    let res = state_reg.transfer_region_ownership(StateId(3), CountryId(1));
    assert!(res.is_err());
    assert_eq!(res.unwrap_err(), "Cannot transfer sea region ownership");
}

#[test]
fn test_phase16_truce_prevents_justification_and_war() {
    let (c_reg, s_reg, mut d_reg, j_reg, w_reg) = setup_phase14_env();

    d_reg.set_truce(CountryId(1), CountryId(2), "1941/01/01".to_string());

    // Justification should fail during truce
    let res_j = j_reg.can_start_justification_with_date(
        CountryId(1),
        CountryId(2),
        StateId(2),
        &c_reg,
        &s_reg,
        &d_reg,
        Some("1936/06/01"),
    );
    assert!(res_j.is_err());
    // P20-009: エラー戻り値は表示用英語原文ではなく安定した翻訳キー。
    assert_eq!(res_j.unwrap_err(), "war_error.justify.truce");

    // War declaration should fail during truce
    let res_w = w_reg.can_declare_war_with_date(
        CountryId(1),
        CountryId(2),
        StateId(2),
        &c_reg,
        &s_reg,
        &d_reg,
        &j_reg,
        Some("1936/06/01"),
    );
    assert!(res_w.is_err());

    // After truce expires (1941/01/02), justification is allowed
    let res_j2 = j_reg.can_start_justification_with_date(
        CountryId(1),
        CountryId(2),
        StateId(2),
        &c_reg,
        &s_reg,
        &d_reg,
        Some("1941/01/02"),
    );
    assert!(res_j2.is_ok());
}
