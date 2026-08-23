#![cfg(test)]

use crate::common::{CountryId, DivisionDefinitionId, DivisionId, StateId};
use crate::military::data::{
    Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use crate::state::data::{StateData, StateRegistry};
use crate::war::data::{War, WarRegistry};
use crate::war::war_score::process_war_score;

use crate::diplomacy::claims::{ClaimRegistry, ClaimSource, ClaimStatus, TerritorialClaim};
use crate::diplomacy::crisis::{
    CrisisPhase, CrisisRegistry, DiplomaticCrisis, WarGoal, WarGoalType,
};
use crate::war::peace::{PeaceTerm, execute_peace_settlement};

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
        primary_attacker: None,
        primary_defender: None,
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
        &mut CrisisRegistry::default(),
        &mut ClaimRegistry::default(),
        &mut CountryRegistry::default(),
        &mut crate::military::army::ArmyRegistry::default(),
        &mut WarJustificationRegistry::default(),
        &mut crate::country::power::CountryPowerRegistry::default(),
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

/// P21-021: 消滅済み国家はDomain APIレベルで正当化の開始国になれない
/// (UIのrun_ifだけに依存しない直接検証)。
#[test]
fn p21_021_cannot_justify_as_an_eliminated_initiator() {
    let (mut c_reg, s_reg, d_reg, mut j_reg, _) = setup_phase14_env();
    c_reg.get_mut(CountryId(1)).unwrap().status = crate::country::CountryStatus::Eliminated {
        eliminated_date: "1935/01/01".to_string(),
        annexed_by: CountryId(3),
    };
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
    assert_eq!(res.unwrap_err(), "war_error.justify.initiator_missing");
    assert!(j_reg.justifications.is_empty());
}

/// P21-021: 消滅済み国家はDomain APIレベルで正当化の対象国にもなれない。
#[test]
fn p21_021_cannot_justify_against_an_eliminated_target() {
    let (mut c_reg, s_reg, d_reg, mut j_reg, _) = setup_phase14_env();
    c_reg.get_mut(CountryId(2)).unwrap().status = crate::country::CountryStatus::Eliminated {
        eliminated_date: "1935/01/01".to_string(),
        annexed_by: CountryId(3),
    };
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
    assert_eq!(res.unwrap_err(), "war_error.justify.target_missing");
    assert!(j_reg.justifications.is_empty());
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

/// P21-011要求テスト: `grant_completed_justification`は既存正当化がない場合、
/// 即座にis_ready=trueの新規正当化を作成する。
#[test]
fn grant_completed_justification_creates_new_ready_justification() {
    let (_c_reg, _s_reg, _d_reg, mut j_reg, _w_reg) = setup_phase14_env();

    let id = j_reg.grant_completed_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        None,
        Vec::new(),
        Vec::new(),
    );

    let justification = j_reg.justifications.get(&id).unwrap();
    assert!(justification.is_ready);
    assert_eq!(justification.initiator, CountryId(1));
    assert_eq!(justification.target, CountryId(2));
    assert_eq!(justification.target_state, StateId(2));
    assert_eq!(j_reg.justifications.len(), 1);
}

/// P21-011要求テスト: 既に進行中(未完成)の正当化がある場合、`grant_completed_justification`は
/// 重複作成せず、既存の正当化を即座に完了扱いへ引き上げる。
#[test]
fn grant_completed_justification_upgrades_existing_justification_idempotently() {
    let (c_reg, s_reg, d_reg, mut j_reg, _w_reg) = setup_phase14_env();
    let existing_id = j_reg
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
    assert!(!j_reg.justifications.get(&existing_id).unwrap().is_ready);

    let granted_id = j_reg.grant_completed_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/02/01".to_string(),
        None,
        Vec::new(),
        Vec::new(),
    );

    assert_eq!(
        granted_id, existing_id,
        "must upgrade the existing justification, not create a second one"
    );
    assert_eq!(j_reg.justifications.len(), 1);
    assert!(j_reg.justifications.get(&existing_id).unwrap().is_ready);
}

/// P21-011要求テスト: `cancel_justification`は既存の正当化を削除し、
/// 既に存在しないidに対しては`false`を返す(冪等)。
#[test]
fn cancel_justification_removes_existing_and_is_idempotent_when_missing() {
    let (_c_reg, _s_reg, _d_reg, mut j_reg, _w_reg) = setup_phase14_env();
    let id = j_reg.grant_completed_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        None,
        Vec::new(),
        Vec::new(),
    );

    assert!(j_reg.cancel_justification(id));
    assert!(!j_reg.justifications.contains_key(&id));
    assert!(
        !j_reg.cancel_justification(id),
        "cancelling an already-removed justification must return false, not panic"
    );
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

/// P21-016要求テスト: P21-016より前のSave由来(`primary_attacker`/`primary_defender`
/// フィールドを一切含まない)RONが引き続き正しくデシリアライズされ、
/// `primary_attacker_id()`/`primary_defender_id()`が集合内の最小要素へ
/// 正しくフォールバックする。
#[test]
fn war_deserializes_from_pre_p21_016_ron_without_primary_fields() {
    let ron = r#"(
        id: (0),
        name: "Legacy War",
        attackers: [(1)],
        defenders: [(2)],
        war_goals: [],
        start_date: "1936/01/01",
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: [],
        status: Active,
    )"#;
    let war: War = ron::from_str(ron).expect("pre-P21-016 War RON must still deserialize");
    assert_eq!(war.primary_attacker, None);
    assert_eq!(war.primary_defender, None);
    assert_eq!(war.primary_attacker_id(), CountryId(1));
    assert_eq!(war.primary_defender_id(), CountryId(2));
}

/// P21-016要求テスト: P21-016より前のSave由来(`source_crisis_id`/`committed_attackers`/
/// `committed_defenders`フィールドを一切含まない)`WarJustification`のRONが
/// 引き続き正しくデシリアライズされ、新フィールドは空/`None`になる。
#[test]
fn war_justification_deserializes_from_pre_p21_016_ron_without_snapshot_fields() {
    let ron = r#"(
        id: 0,
        initiator: (0),
        target: (1),
        target_state: (1),
        start_date: "1800/01/01",
        required_days: 30,
        days_passed: 30,
        is_ready: true,
    )"#;
    let justification: crate::war::justification::WarJustification =
        ron::from_str(ron).expect("pre-P21-016 WarJustification RON must still deserialize");
    assert_eq!(justification.source_crisis_id, None);
    assert!(justification.committed_attackers.is_empty());
    assert!(justification.committed_defenders.is_empty());
}

/// P21-016要求テスト: 正当化のスナップショットに記録された支持国が、
/// 正しい陣営(攻撃側/防御側)のWar参加国として追加される。
#[test]
fn declare_war_adds_committed_supporters_to_correct_sides() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    j_reg.grant_completed_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        Some(crate::common::DiplomaticCrisisId(0)),
        vec![CountryId(3)],
        vec![],
    );

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
    assert_eq!(
        war.attackers,
        [CountryId(1), CountryId(3)].into_iter().collect()
    );
    assert_eq!(war.defenders, [CountryId(2)].into_iter().collect());
    assert_eq!(war.primary_attacker, Some(CountryId(1)));
    assert_eq!(war.primary_defender, Some(CountryId(2)));
}

/// P21-016要求テスト(二次レビュー対応): `declare_war`は常に要求国/対象国自身を
/// 無条件で挿入するため、支持国が(存在しない等の理由で)全員除外された最悪の
/// ケースでも、`attackers`/`defenders`が空になることは構造的にあり得ない。
#[test]
fn declare_war_never_produces_empty_attacker_or_defender_sets() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    j_reg.grant_completed_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        None,
        // 支持国は全て存在しない国家IDのみ(宣戦布告時点で個別に除外される)。
        vec![CountryId(997), CountryId(998)],
        vec![CountryId(996)],
    );

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
    assert!(!war.attackers.is_empty(), "attackers must never be empty");
    assert!(!war.defenders.is_empty(), "defenders must never be empty");
    assert_eq!(war.attackers, [CountryId(1)].into_iter().collect());
    assert_eq!(war.defenders, [CountryId(2)].into_iter().collect());
}

/// P21-016要求テスト(二次レビュー対応): `primary_attacker_id()`/`primary_defender_id()`は
/// `attackers`/`defenders`が空という本来あり得ない不変条件違反状態に対して、架空の
/// `CountryId(0)`を捏造して返すのではなく、明示的にpanicする
/// (`declare_war`は上のテストが示すとおり空集合を作れないため、この状態は
/// `declare_war`を経由しない直接構築でしか再現できない)。
#[test]
#[should_panic(expected = "War.attackers must never be empty")]
fn primary_attacker_id_panics_instead_of_fabricating_a_country_on_a_truly_empty_set() {
    let war = War {
        id: crate::common::WarId(1),
        name: "Corrupt War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attackers: std::collections::HashSet::new(),
        defenders: vec![CountryId(2)].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
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
    };

    let _ = war.primary_attacker_id();
}

/// P21-016要求テスト: 宣戦布告時点で既に存在しない支持国は、War全体を失敗させず
/// 個別に静かに除外される。
#[test]
fn declare_war_excludes_nonexistent_supporter_silently() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    j_reg.grant_completed_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        None,
        vec![CountryId(999)], // setup_phase14_envには存在しない
        vec![],
    );

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
    assert_eq!(war.attackers, [CountryId(1)].into_iter().collect());
}

/// P21-016要求テスト: 同じ国が攻撃側・防御側の両方の支持コミットメントに現れる
/// (データ破損)場合、War全体をアトミックに拒否し、正当化も消費されない。
#[test]
fn declare_war_rejects_atomically_when_supporter_appears_on_both_sides() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    j_reg.grant_completed_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        None,
        vec![CountryId(3)],
        vec![CountryId(3)],
    );

    let result = w_reg.declare_war(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/31".to_string(),
        &c_reg,
        &s_reg,
        &mut d_reg,
        &mut j_reg,
    );

    assert_eq!(result, Err("war_error.declare.corrupt_support_snapshot"));
    assert!(w_reg.wars.is_empty());
    assert!(
        j_reg
            .get_ready_justification(CountryId(1), CountryId(2), StateId(2))
            .is_some(),
        "justification must not be consumed when declaration is atomically rejected"
    );
    assert!(
        d_reg.get(CountryId(1), CountryId(2)).is_none(),
        "diplomacy relation must not be mutated when declaration is atomically rejected"
    );
}

/// P21-016要求テスト: 支持コミットメントが要求国/対象国自身と矛盾する
/// (要求国が防御側支持者として記録されている等)場合もアトミックに拒否する。
#[test]
fn declare_war_rejects_atomically_when_supporter_equals_the_opposing_belligerent() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    j_reg.grant_completed_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        None,
        vec![],
        vec![CountryId(1)], // 攻撃側の要求国自身が防御側支持者として記録されている(矛盾)
    );

    let result = w_reg.declare_war(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/31".to_string(),
        &c_reg,
        &s_reg,
        &mut d_reg,
        &mut j_reg,
    );

    assert_eq!(result, Err("war_error.declare.corrupt_support_snapshot"));
    assert!(w_reg.wars.is_empty());
}

/// P21-016要求テスト: `War`の敵味方判定共有API(`side_of`/`is_participant`/
/// `are_opponents`/`opponents_of`)が多国間参加者を正しく扱う。
#[test]
fn war_shared_enemy_friend_api_handles_multilateral_participants() {
    let war = War {
        id: crate::common::WarId(1),
        name: "Multilateral Test War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attackers: [CountryId(1), CountryId(3)].into_iter().collect(),
        defenders: [CountryId(2), CountryId(4)].into_iter().collect(),
        primary_attacker: Some(CountryId(1)),
        primary_defender: Some(CountryId(2)),
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
    };

    use crate::war::data::WarSide;
    assert_eq!(war.side_of(CountryId(1)), Some(WarSide::Attacker));
    assert_eq!(war.side_of(CountryId(3)), Some(WarSide::Attacker));
    assert_eq!(war.side_of(CountryId(2)), Some(WarSide::Defender));
    assert_eq!(war.side_of(CountryId(4)), Some(WarSide::Defender));
    assert_eq!(war.side_of(CountryId(99)), None);

    assert!(war.is_participant(CountryId(3)));
    assert!(!war.is_participant(CountryId(99)));

    assert!(war.are_opponents(CountryId(1), CountryId(4)));
    assert!(war.are_opponents(CountryId(3), CountryId(2)));
    assert!(!war.are_opponents(CountryId(1), CountryId(3)));
    assert!(!war.are_opponents(CountryId(2), CountryId(4)));
    assert!(!war.are_opponents(CountryId(1), CountryId(99)));

    assert_eq!(
        war.opponents_of(CountryId(1)),
        vec![CountryId(2), CountryId(4)]
    );
    assert_eq!(
        war.opponents_of(CountryId(4)),
        vec![CountryId(1), CountryId(3)]
    );
    assert_eq!(war.opponents_of(CountryId(99)), Vec::<CountryId>::new());

    assert_eq!(war.sorted_attackers(), vec![CountryId(1), CountryId(3)]);
    assert_eq!(war.sorted_defenders(), vec![CountryId(2), CountryId(4)]);
}

/// P21-016要求テスト: `primary_attacker`/`primary_defender`が`None`(旧Save由来)の場合、
/// `primary_attacker_id()`/`primary_defender_id()`は集合内の最小`CountryId`へ
/// 自動的にフォールバックする(旧Saveの`attackers`/`defenders`は常に単一要素)。
#[test]
fn primary_id_accessors_fall_back_to_min_of_set_when_unset() {
    let war = War {
        id: crate::common::WarId(1),
        name: "Legacy Save War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attackers: [CountryId(5)].into_iter().collect(),
        defenders: [CountryId(2)].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
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
    };

    assert_eq!(war.primary_attacker_id(), CountryId(5));
    assert_eq!(war.primary_defender_id(), CountryId(2));
}

/// P21-016要求テスト: `WarRegistry::is_country_at_war`/`wars_for_country`が
/// 多国間参加者(主要国以外の支持国)についても正しく機能する。
#[test]
fn war_registry_is_country_at_war_and_wars_for_country_cover_supporters() {
    let (c_reg, s_reg, mut d_reg, mut j_reg, mut w_reg) = setup_phase14_env();
    j_reg.grant_completed_justification(
        CountryId(1),
        CountryId(2),
        StateId(2),
        "1936/01/01".to_string(),
        None,
        vec![CountryId(3)],
        vec![],
    );
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

    assert!(w_reg.is_country_at_war(CountryId(3)));
    assert_eq!(w_reg.wars_for_country(CountryId(3)), vec![war_id]);
    assert!(!w_reg.is_country_at_war(CountryId(999)));
    assert!(w_reg.wars_for_country(CountryId(999)).is_empty());
}

// ── P21-016(二次レビュー対応): 多国間参加者の実戦闘・占領・講和への接続 ──────────

/// 攻撃側{1,3}・防御側{2,4}の多国間Warを構築するテスト用ヘルパー(1が攻撃側代表国、
/// 2が防御側代表国)。
fn build_multilateral_war() -> War {
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};

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

    War {
        id: crate::common::WarId(1),
        name: "Multilateral Combat Test War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attackers: vec![CountryId(1), CountryId(3)].into_iter().collect(),
        defenders: vec![CountryId(2), CountryId(4)].into_iter().collect(),
        primary_attacker: Some(CountryId(1)),
        primary_defender: Some(CountryId(2)),
        source_crisis_id: None,
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
    }
}

fn make_division(id: usize, owner: CountryId, current_state: StateId) -> Division {
    Division {
        id: DivisionId(id),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state,
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
    }
}

/// 要求テスト(二次レビュー対応): 攻撃側支持国(CountryId(3))の師団が、防御側の師団を
/// 実際に`process_combat`経由で攻撃できる(被害が生じる)ことを直接確認する。
#[test]
fn attacker_side_supporter_division_can_attack_defender_via_process_combat() {
    use crate::war::combat::process_combat;

    let mut war_reg = WarRegistry::default();
    let war = build_multilateral_war();
    war_reg.wars.insert(war.id, war);

    let mut mil_reg = MilitaryRegistry::default();
    mil_reg.definitions.insert(
        DivisionDefinitionId(1),
        DivisionDefinition {
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
        },
    );
    // 攻撃側支持国(3)の師団と、防御側代表国(2)の師団を同じ州に配置する。
    let supporter_div = make_division(1, CountryId(3), StateId(1));
    let defender_div = make_division(2, CountryId(2), StateId(1));
    mil_reg.divisions.insert(supporter_div.id, supporter_div);
    mil_reg.divisions.insert(defender_div.id, defender_div);

    process_combat(&mut mil_reg, &war_reg);

    let supporter_after = mil_reg.divisions.get(&DivisionId(1)).unwrap();
    let defender_after = mil_reg.divisions.get(&DivisionId(2)).unwrap();
    assert_eq!(supporter_after.status, DivisionStatus::Fighting);
    assert_eq!(defender_after.status, DivisionStatus::Fighting);
    assert!(
        supporter_after.manpower < 10000 || defender_after.manpower < 10000,
        "combat between an attacker-side supporter and the defender must cause damage"
    );
}

/// 要求テスト(二次レビュー対応): 同じ陣営に属する師団同士(攻撃側代表国と攻撃側支持国、
/// および防御側代表国と防御側支持国)は`process_combat`で交戦しない
/// (`are_countries_at_war`が同陣営ペアに`false`を返すため)。
#[test]
fn same_side_divisions_never_engage_in_combat() {
    use crate::war::combat::process_combat;

    let mut war_reg = WarRegistry::default();
    let war = build_multilateral_war();
    war_reg.wars.insert(war.id, war);

    let mut mil_reg = MilitaryRegistry::default();
    // 攻撃側代表国(1)と攻撃側支持国(3)を同じ州に配置(同陣営)。
    let atk_primary = make_division(1, CountryId(1), StateId(1));
    let atk_supporter = make_division(2, CountryId(3), StateId(1));
    // 防御側代表国(2)と防御側支持国(4)を別の州に配置(同陣営)。
    let def_primary = make_division(3, CountryId(2), StateId(2));
    let def_supporter = make_division(4, CountryId(4), StateId(2));
    mil_reg.divisions.insert(atk_primary.id, atk_primary);
    mil_reg.divisions.insert(atk_supporter.id, atk_supporter);
    mil_reg.divisions.insert(def_primary.id, def_primary);
    mil_reg.divisions.insert(def_supporter.id, def_supporter);

    process_combat(&mut mil_reg, &war_reg);

    for id in [1, 2, 3, 4] {
        let division = mil_reg.divisions.get(&DivisionId(id)).unwrap();
        assert_eq!(
            division.status,
            DivisionStatus::Idle,
            "division {id} on a same-side pairing must not enter combat"
        );
        assert_eq!(
            division.manpower, 10000,
            "no damage must be applied to division {id}"
        );
    }
}

/// 要求テスト(二次レビュー対応): 攻撃側支持国(CountryId(3))の師団が、防御側代表国が
/// 所有する州を`process_occupation`経由で実際に占領できることを直接確認する
/// (`get_active_war_for_country`/陣営横断チェックが多国間参加者に対しても機能する)。
#[test]
fn supporter_occupation_succeeds_via_process_occupation() {
    use crate::war::occupation::process_occupation;

    let mut war_reg = WarRegistry::default();
    let war = build_multilateral_war();
    war_reg.wars.insert(war.id, war);

    let mut mil_reg = MilitaryRegistry::default();
    let mut supporter_div = make_division(1, CountryId(3), StateId(2));
    supporter_div.status = DivisionStatus::Idle;
    mil_reg.divisions.insert(supporter_div.id, supporter_div);

    // StateId(2)は防御側代表国(2)が所有・支配する敵地。
    let enemy_state = StateData {
        id: StateId(2),
        owner_country_id: CountryId(2),
        controller_country: Some(CountryId(2)),
        ..Default::default()
    };
    let mut state_reg = StateRegistry::build(vec![enemy_state]);

    // 20日分進めると占領完了(1日+5.0、100.0到達)する既存仕様どおり。
    for _ in 0..20 {
        process_occupation(&mut state_reg, &mut mil_reg, &war_reg);
    }

    let state_after = state_reg.get(StateId(2)).unwrap();
    assert_eq!(
        state_after.controller_country,
        Some(CountryId(3)),
        "the attacker-side supporter must end up controlling the occupied enemy state"
    );
    assert_eq!(state_after.occupation_progress, 100.0);
}

/// 要求テスト(二次レビュー対応): 講和成立後、多国間参加者全員(代表国・支持国とも)の
/// 敵対判定が解除されることを直接確認する。
#[test]
fn peace_settlement_clears_hostility_for_all_participants_including_supporters() {
    let mut war_reg = WarRegistry::default();
    let war = build_multilateral_war();
    let war_id = war.id;
    war_reg.wars.insert(war_id, war);

    // 事前条件: 攻撃側・防御側は陣営を問わず互いに敵対している。
    assert!(war_reg.are_countries_at_war(CountryId(1), CountryId(2)));
    assert!(war_reg.are_countries_at_war(CountryId(3), CountryId(4)));
    assert!(war_reg.is_country_at_war(CountryId(3)));
    assert!(war_reg.is_country_at_war(CountryId(4)));

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
    let mut state_reg = StateRegistry::build(vec![s1, s2]);
    let mut mil_reg = MilitaryRegistry::default();
    let mut battle_reg = crate::military::battle::BattleRegistry::default();
    let mut dip_reg = DiplomacyRegistry::default();
    let mut frontline_reg = crate::war::frontline::FrontlineRegistry::default();

    crate::war::peace::execute_peace_settlement(
        war_id,
        crate::war::peace::PeaceTerm::WhitePeace,
        "Test White Peace",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut frontline_reg,
        &mut CrisisRegistry::default(),
        &mut ClaimRegistry::default(),
        &mut CountryRegistry::default(),
        &mut crate::military::army::ArmyRegistry::default(),
        &mut WarJustificationRegistry::default(),
        &mut crate::country::power::CountryPowerRegistry::default(),
    )
    .unwrap();

    for (a, b) in [
        (CountryId(1), CountryId(2)),
        (CountryId(1), CountryId(4)),
        (CountryId(3), CountryId(2)),
        (CountryId(3), CountryId(4)),
    ] {
        assert!(
            !war_reg.are_countries_at_war(a, b),
            "{a:?} and {b:?} must no longer be at war after the peace settlement"
        );
    }
    for cid in [CountryId(1), CountryId(2), CountryId(3), CountryId(4)] {
        assert!(
            !war_reg.is_country_at_war(cid),
            "{cid:?} must not be reported as at war in any active war after the peace settlement"
        );
    }
}

/// 要求テスト(二次レビュー対応): 支持国が別のWarにも参加している場合、片方のWarが
/// 講和で終結しても、もう片方がActiveのままなら敵対判定は維持される
/// (`WarRegistry`は複数Warを独立に走査するため、片方の終結が他方に影響しない)。
#[test]
fn hostility_persists_only_for_a_still_active_second_war() {
    let mut war_reg = WarRegistry::default();
    let war_a = build_multilateral_war(); // 攻撃側{1,3} vs 防御側{2,4}
    let war_a_id = war_a.id;
    war_reg.wars.insert(war_a_id, war_a);

    // 支持国3は、別の戦争War Bでは対CountryId(5)の攻撃側代表国として参戦している。
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};
    let war_b = War {
        id: crate::common::WarId(2),
        name: "Second War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attackers: vec![CountryId(3)].into_iter().collect(),
        defenders: vec![CountryId(5)].into_iter().collect(),
        primary_attacker: Some(CountryId(3)),
        primary_defender: Some(CountryId(5)),
        source_crisis_id: None,
        war_goals: vec![WarGoal {
            attacker: CountryId(3),
            defender: CountryId(5),
            goal_type: WarGoalType::ConquerState,
            target_states: vec![StateId(3)],
            base_peace_cost: 20.0,
            international_concern: 10.0,
            completion: 0.0,
            is_primary: true,
        }],
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
    let war_b_id = war_b.id;
    war_reg.wars.insert(war_b_id, war_b);

    assert!(war_reg.is_country_at_war(CountryId(3)));
    assert!(war_reg.are_countries_at_war(CountryId(3), CountryId(2)));
    assert!(war_reg.are_countries_at_war(CountryId(3), CountryId(5)));

    // War Aだけを講和で終結させる。
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
    let mut state_reg = StateRegistry::build(vec![s1, s2]);
    let mut mil_reg = MilitaryRegistry::default();
    let mut battle_reg = crate::military::battle::BattleRegistry::default();
    let mut dip_reg = DiplomacyRegistry::default();
    let mut frontline_reg = crate::war::frontline::FrontlineRegistry::default();

    crate::war::peace::execute_peace_settlement(
        war_a_id,
        crate::war::peace::PeaceTerm::WhitePeace,
        "Test White Peace",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut frontline_reg,
        &mut CrisisRegistry::default(),
        &mut ClaimRegistry::default(),
        &mut CountryRegistry::default(),
        &mut crate::military::army::ArmyRegistry::default(),
        &mut WarJustificationRegistry::default(),
        &mut crate::country::power::CountryPowerRegistry::default(),
    )
    .unwrap();

    // War Aの相手(2)との敵対は解除されるが、War Bがまだ Active なので、
    // 支持国3自体は依然として「いずれかの戦争に参加中」であり、War Bの相手(5)とは
    // 引き続き敵対したままでなければならない。
    assert!(!war_reg.are_countries_at_war(CountryId(3), CountryId(2)));
    assert!(war_reg.are_countries_at_war(CountryId(3), CountryId(5)));
    assert!(
        war_reg.is_country_at_war(CountryId(3)),
        "country 3 must still be reported at war because War B is still Active"
    );
    assert_eq!(war_reg.wars_for_country(CountryId(3)), vec![war_b_id]);
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
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
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
        primary_attacker: None,
        primary_defender: None,
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
    use crate::military::data::{Division, DivisionSize, DivisionStatus, DivisionType};
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
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
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
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
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
    assert!(
        can_accept_peace_offer(&offer_early_wp, &war_reg, None, None, None, "1936/01/10").is_err()
    );

    // 2. White Peace after 30 days should succeed
    let offer_valid_wp = PeaceOffer {
        war_id: crate::common::WarId(1),
        proposer_country_id: CountryId(1),
        recipient_country_id: CountryId(2),
        term: PeaceTerm::WhitePeace,
        created_date: "1936/02/05".to_string(), // >30 days
    };
    assert!(
        can_accept_peace_offer(&offer_valid_wp, &war_reg, None, None, None, "1936/02/05").is_ok()
    );

    // 3. Cede region with score 50 (< 60) and no capitulation should fail
    let offer_cede = PeaceOffer {
        war_id: crate::common::WarId(1),
        proposer_country_id: CountryId(1),
        recipient_country_id: CountryId(2),
        term: PeaceTerm::CedeWarGoalRegion,
        created_date: "1936/02/05".to_string(),
    };
    assert!(can_accept_peace_offer(&offer_cede, &war_reg, None, None, None, "1936/02/05").is_err());

    // 4. Update war score to 70 (>= 60) -> Cede region succeeds
    war_reg
        .wars
        .get_mut(&crate::common::WarId(1))
        .unwrap()
        .war_score = 70.0;
    assert!(can_accept_peace_offer(&offer_cede, &war_reg, None, None, None, "1936/02/05").is_ok());
}

/// P21-021: 消滅済み国家はDomain APIレベルで講和提案の提案者にも受領者にも
/// なれない(UIのrun_ifだけに依存しない直接検証)。
#[test]
fn p21_021_can_accept_peace_offer_rejects_an_eliminated_proposer_or_recipient() {
    use crate::war::peace::{PeaceOffer, can_accept_peace_offer};

    let mut war_reg = WarRegistry::default();
    let war = War {
        id: crate::common::WarId(1),
        name: "Elimination Peace War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attackers: vec![CountryId(1)].into_iter().collect(),
        defenders: vec![CountryId(2)].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
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
    };
    war_reg.wars.insert(war.id, war);

    let offer = PeaceOffer {
        war_id: crate::common::WarId(1),
        proposer_country_id: CountryId(1),
        recipient_country_id: CountryId(2),
        term: PeaceTerm::WhitePeace,
        created_date: "1936/02/05".to_string(),
    };

    let mut country_reg = CountryRegistry::default();
    country_reg.countries.push(CountryData {
        id: CountryId(1),
        status: crate::country::CountryStatus::Eliminated {
            eliminated_date: "1936/01/15".to_string(),
            annexed_by: CountryId(9),
        },
        ..Default::default()
    });
    country_reg.countries.push(CountryData {
        id: CountryId(2),
        ..Default::default()
    });

    let result = can_accept_peace_offer(
        &offer,
        &war_reg,
        None,
        None,
        Some(&country_reg),
        "1936/02/05",
    );
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "war_error.peace.proposer_missing");

    // 提案者側は健在だが、受領者側が消滅済みのケースも同様に拒否される。
    let mut country_reg2 = CountryRegistry::default();
    country_reg2.countries.push(CountryData {
        id: CountryId(1),
        ..Default::default()
    });
    country_reg2.countries.push(CountryData {
        id: CountryId(2),
        status: crate::country::CountryStatus::Eliminated {
            eliminated_date: "1936/01/15".to_string(),
            annexed_by: CountryId(9),
        },
        ..Default::default()
    });
    let result2 = can_accept_peace_offer(
        &offer,
        &war_reg,
        None,
        None,
        Some(&country_reg2),
        "1936/02/05",
    );
    assert!(result2.is_err());
    assert_eq!(result2.unwrap_err(), "war_error.peace.recipient_missing");
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
        primary_attacker: None,
        primary_defender: None,
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
    };

    let bd = calculate_war_score(&war, &state_reg);
    // Sea state s_sea (StateId 3) is ignored; only s2 is counted -> 40 * (1/1) = 40
    assert_eq!(bd.attacker_occupation_score, 40);

    // Transferring sea region should fail
    let res = state_reg.test_transfer_region_ownership_unchecked(StateId(3), CountryId(1));
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

// ─────────────────────────────────────────────────────────────────────
// P21-019: Crisis由来War講和のClaim消費・領土移転・lineage検証・原子性
// ─────────────────────────────────────────────────────────────────────

fn p21_019_claim(
    id: usize,
    claimant: usize,
    target_state: usize,
    status: ClaimStatus,
) -> TerritorialClaim {
    TerritorialClaim {
        id: crate::common::ClaimId(id),
        claimant_country: CountryId(claimant),
        target_state: StateId(target_state),
        strength: 50.0,
        created_date: "1936/01/01".to_string(),
        is_permanent: false,
        source: ClaimSource::Strategic,
        status,
    }
}

fn p21_019_war_goal(initiator: usize, target: usize, target_state: usize) -> WarGoal {
    WarGoal {
        attacker: CountryId(initiator),
        defender: CountryId(target),
        goal_type: WarGoalType::ConquerState,
        target_states: vec![StateId(target_state)],
        base_peace_cost: 20.0,
        international_concern: 10.0,
        completion: 0.0,
        is_primary: true,
    }
}

fn p21_019_crisis(
    id: usize,
    initiator: usize,
    target: usize,
    target_state: usize,
    claim_id: Option<crate::common::ClaimId>,
) -> DiplomaticCrisis {
    DiplomaticCrisis {
        id: crate::common::DiplomaticCrisisId(id),
        initiator: CountryId(initiator),
        target: CountryId(target),
        war_goals: vec![p21_019_war_goal(initiator, target, target_state)],
        start_date: "1936/01/01".to_string(),
        current_phase: CrisisPhase::WarStarted,
        escalation: 100.0,
        initiator_support: 0.0,
        target_resistance: 0.0,
        days_in_phase: 0,
        deadline_date: None,
        international_concern: 0.0,
        third_party_reactions: std::collections::HashMap::new(),
        related_claim_id: claim_id,
        related_justification_id: None,
        related_war_id: None,
    }
}

/// attackers/defendersを明示指定できる、Crisis由来War用の汎用builder。
#[allow(clippy::too_many_arguments)]
fn p21_019_war(
    id: usize,
    attackers: &[usize],
    defenders: &[usize],
    primary_attacker: usize,
    primary_defender: usize,
    target_state: usize,
    source_crisis_id: Option<crate::common::DiplomaticCrisisId>,
) -> War {
    War {
        id: crate::common::WarId(id),
        name: "Crisis War".to_string(),
        start_date: "1936/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 100.0,
        attackers: attackers.iter().map(|&c| CountryId(c)).collect(),
        defenders: defenders.iter().map(|&c| CountryId(c)).collect(),
        primary_attacker: Some(CountryId(primary_attacker)),
        primary_defender: Some(CountryId(primary_defender)),
        source_crisis_id,
        war_goals: vec![p21_019_war_goal(
            primary_attacker,
            primary_defender,
            target_state,
        )],
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
    }
}

/// P21-019テスト用の完全な環境: claimant=0, target=1, target_state=StateId(1)
/// (targetの所有)への、有効なlineageを持つCrisis由来War。全レジストリを返す。
///
/// `target`(1)の`capital_state_id`はあえてStateId(1)(=割譲対象州)以外
/// (StateId(999)、実在しないダミー)にする——このヘルパーを使う大半の
/// 既存テストは「首都以外の州の割譲」を想定しており、もしtargetの首都を
/// StateId(1)にしてしまうと、この環境を使う全既存テストが新設の首都
/// 再割当ロジック(§下記`resolve_capital_reassignment`)へ意図せず
/// 巻き込まれてしまう。首都割譲そのものを検証する専用テストは
/// 別の専用フィクスチャ(`p21_019_capital_env`)を使う。
#[allow(clippy::type_complexity)]
fn p21_019_valid_env() -> (
    WarRegistry,
    CrisisRegistry,
    ClaimRegistry,
    StateRegistry,
    MilitaryRegistry,
    crate::military::battle::BattleRegistry,
    crate::diplomacy::data::DiplomacyRegistry,
    crate::war::frontline::FrontlineRegistry,
    CountryRegistry,
    crate::military::army::ArmyRegistry,
    WarJustificationRegistry,
    crate::country::power::CountryPowerRegistry,
) {
    let claim = p21_019_claim(0, 0, 1, ClaimStatus::Active);
    let claim_id = claim.id;
    let mut claim_reg = ClaimRegistry::default();
    claim_reg.claims.insert(claim_id, claim);

    let crisis = p21_019_crisis(0, 0, 1, 1, Some(claim_id));
    let mut crisis_reg = CrisisRegistry::default();
    crisis_reg.crises.insert(crisis.id, crisis);

    let war = p21_019_war(
        1,
        &[0],
        &[1],
        0,
        1,
        1,
        Some(crate::common::DiplomaticCrisisId(0)),
    );
    let mut war_reg = WarRegistry::default();
    war_reg.wars.insert(war.id, war);

    let s0 = StateData {
        id: StateId(0),
        owner_country_id: CountryId(0),
        ..Default::default()
    };
    let s1 = StateData {
        id: StateId(1),
        owner_country_id: CountryId(1),
        ..Default::default()
    };
    let state_reg = StateRegistry::build(vec![s0, s1]);

    let country_reg = CountryRegistry {
        countries: vec![
            CountryData {
                id: CountryId(0),
                capital_state_id: StateId(0),
                ..CountryData::default()
            },
            CountryData {
                id: CountryId(1),
                capital_state_id: StateId(999),
                ..CountryData::default()
            },
        ],
    };

    (
        war_reg,
        crisis_reg,
        claim_reg,
        state_reg,
        MilitaryRegistry::default(),
        crate::military::battle::BattleRegistry::default(),
        crate::diplomacy::data::DiplomacyRegistry::default(),
        crate::war::frontline::FrontlineRegistry::default(),
        country_reg,
        crate::military::army::ArmyRegistry::default(),
        WarJustificationRegistry::default(),
        crate::country::power::CountryPowerRegistry::default(),
    )
}

/// スナップショット比較用: 講和が失敗した場合に「何も変わっていない」ことを
/// 確認するための最小限の状態抜粋。
#[derive(Debug, PartialEq)]
struct P21019Snapshot {
    war_status: crate::war::data::WarStatus,
    claim_status: ClaimStatus,
    state1_owner: CountryId,
    claims_len: usize,
    crises_len: usize,
    wars_len: usize,
}

fn p21_019_snapshot(
    war_reg: &WarRegistry,
    claim_reg: &ClaimRegistry,
    state_reg: &StateRegistry,
    claim_id: crate::common::ClaimId,
    war_id: crate::common::WarId,
) -> P21019Snapshot {
    P21019Snapshot {
        war_status: war_reg.wars.get(&war_id).unwrap().status,
        claim_status: claim_reg.claims.get(&claim_id).unwrap().status,
        state1_owner: state_reg.get(StateId(1)).unwrap().owner_country_id,
        claims_len: claim_reg.claims.len(),
        crises_len: 0, // crisis_regはlineage検証時のみ参照、変更されないため別途len比較する
        wars_len: war_reg.wars.len(),
    }
}

/// 要求テスト1,3,11: Crisis由来Warが領土割譲(CedeWarGoalRegion)で講和すると、
/// 元Claimが消費され、かつCrisisの要求州(target_state)がCrisis initiatorへ移転する。
#[test]
fn crisis_war_cede_settlement_consumes_claim_and_transfers_target_state_to_initiator() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );
    assert!(result.is_ok());

    let claim = claim_reg.claims.get(&crate::common::ClaimId(0)).unwrap();
    assert_eq!(claim.status, ClaimStatus::Consumed);

    let state1 = state_reg.get(StateId(1)).unwrap();
    assert_eq!(
        state1.owner_country_id,
        CountryId(0),
        "target_state must transfer to the crisis initiator (attacker)"
    );

    let war = war_reg.wars.get(&crate::common::WarId(1)).unwrap();
    assert_eq!(war.status, crate::war::data::WarStatus::AttackerVictory);
}

/// 要求テスト2,14: 白紙講和でも元Claimは消費されるが、target_stateの所有権は
/// 変化しない。
#[test]
fn crisis_war_white_peace_consumes_claim_without_transferring_territory() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();

    execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::WhitePeace,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
    .unwrap();

    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(0))
            .unwrap()
            .status,
        ClaimStatus::Consumed
    );
    assert_eq!(
        state_reg.get(StateId(1)).unwrap().owner_country_id,
        CountryId(1),
        "white peace must not transfer the crisis target_state"
    );
}

/// 要求テスト4,15: 防御側勝利(AttackerConcedes)でも元Claimは消費されるが、
/// 要求州は攻撃側へ移転しない。
#[test]
fn crisis_war_attacker_concedes_consumes_claim_without_transferring_territory() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();

    execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::AttackerConcedes,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
    .unwrap();

    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(0))
            .unwrap()
            .status,
        ClaimStatus::Consumed
    );
    assert_eq!(
        state_reg.get(StateId(1)).unwrap().owner_country_id,
        CountryId(1),
        "defender victory must not transfer the crisis target_state to the attacker"
    );

    let war = war_reg.wars.get(&crate::common::WarId(1)).unwrap();
    assert_eq!(war.status, crate::war::data::WarStatus::DefenderVictory);
}

/// 要求テスト5: 降伏(`evaluate_war_capitulation`が実際に検出し、
/// `handle_daily_war_resolution`と同じ`CedeWarGoalRegion`/`AttackerConcedes`経路)
/// でも元Claimが消費される。防御側が対象州を実効支配されている状態を作り、
/// 実際の`evaluate_war_capitulation`の判定結果をそのまま`execute_peace_settlement`
/// へ渡す(P21-019専用の別経路を作らない)。
#[test]
fn crisis_war_capitulation_path_consumes_claim() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();

    // 防御側(1)の陸軍を全滅させ、防御側降伏条件(戦闘可能な陸軍が残っていない)を満たす。
    // 対象州はattacker(0)がcontrollerであることも前提条件のため、占領済みにしておく。
    state_reg.get_mut(StateId(1)).unwrap().controller_country = Some(CountryId(0));

    let war = war_reg.wars.get(&crate::common::WarId(1)).unwrap().clone();
    let cap = crate::war::capitulation::evaluate_war_capitulation(&war, &state_reg, &mil_reg);
    assert_eq!(
        cap,
        crate::war::capitulation::CapitulationResult::DefenderCapitulated,
        "test setup must actually trigger defender capitulation"
    );

    execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Defender Capitulation",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
    .unwrap();

    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(0))
            .unwrap()
            .status,
        ClaimStatus::Consumed
    );
}

/// 要求テスト6: 通常War(`source_crisis_id: None`)の講和は、無関係な既存Claimを
/// 一切変更しない。
#[test]
fn normal_war_peace_does_not_touch_unrelated_claims() {
    let (mut mil_reg, mut war_reg, mut state_reg) = setup();
    let mut battle_reg = crate::military::battle::BattleRegistry::default();
    let mut dip_reg = crate::diplomacy::data::DiplomacyRegistry::default();
    let mut fl_reg = crate::war::frontline::FrontlineRegistry::default();
    let mut crisis_reg = CrisisRegistry::default();
    let mut country_reg = CountryRegistry::default();
    let mut army_reg = crate::military::army::ArmyRegistry::default();
    let mut justification_reg = WarJustificationRegistry::default();
    let mut power_reg = crate::country::power::CountryPowerRegistry::default();

    let mut claim_reg = ClaimRegistry::default();
    let unrelated = p21_019_claim(0, 5, 6, ClaimStatus::Active);
    claim_reg.claims.insert(unrelated.id, unrelated);

    execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
    .unwrap();

    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(0))
            .unwrap()
            .status,
        ClaimStatus::Active,
        "a normal (non-crisis) war's peace must never touch an unrelated claim"
    );
}

/// 要求テスト7,8: 同じclaimantが複数Claimを保持していても、Crisisが明示的に
/// 参照するClaim IDだけが消費され、他のClaim(同じclaimant・別state)は
/// 変更されない。
#[test]
fn only_the_exact_lineage_claim_is_consumed_not_other_claims_by_the_same_claimant() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();

    // 同じclaimant(0)が持つ、無関係な別Claim(別state向け)を追加する。
    let other_claim = p21_019_claim(1, 0, 2, ClaimStatus::Active);
    claim_reg.claims.insert(other_claim.id, other_claim);
    state_reg
        .get_mut(StateId(1))
        .map(|_| ())
        .expect("state 1 must exist");
    // state 2 も登録しておく(存在しないState IDにならないよう)。
    let s2 = StateData {
        id: StateId(2),
        owner_country_id: CountryId(3),
        ..Default::default()
    };
    let mut states = state_reg.states.clone();
    states.push(s2);
    state_reg = StateRegistry::build(states);

    execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
    .unwrap();

    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(0))
            .unwrap()
            .status,
        ClaimStatus::Consumed
    );
    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(1))
            .unwrap()
            .status,
        ClaimStatus::Active,
        "an unrelated claim by the same claimant must not be consumed"
    );
}

/// 要求テスト9: 既にConsumedのClaimへ講和処理が到達しても、冪等に安全に扱う
/// (`mark_consumed`自体の既存冪等性、二重副作用なし)。
#[test]
fn already_consumed_claim_is_handled_idempotently() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();
    claim_reg
        .claims
        .get_mut(&crate::common::ClaimId(0))
        .unwrap()
        .status = ClaimStatus::Consumed;

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert!(result.is_ok());
    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(0))
            .unwrap()
            .status,
        ClaimStatus::Consumed
    );
    assert_eq!(
        claim_reg.claims.len(),
        1,
        "no duplicate claim must be created"
    );
}

/// 要求テスト12,29: 2対2の多国間Crisis由来Warでも、要求州は
/// primary attacker(Crisis initiator)へ移転し、支持国(supporter)へは移らない。
#[test]
fn multilateral_crisis_war_transfers_territory_to_primary_attacker_not_supporters() {
    let claim = p21_019_claim(0, 0, 1, ClaimStatus::Active);
    let mut claim_reg = ClaimRegistry::default();
    claim_reg.claims.insert(claim.id, claim);

    let crisis = p21_019_crisis(0, 0, 1, 1, Some(crate::common::ClaimId(0)));
    let mut crisis_reg = CrisisRegistry::default();
    crisis_reg.crises.insert(crisis.id, crisis);

    // 攻撃側: initiator(0) + supporter(3)。防御側: target(1) + supporter(4)。
    let war = p21_019_war(
        1,
        &[0, 3],
        &[1, 4],
        0,
        1,
        1,
        Some(crate::common::DiplomaticCrisisId(0)),
    );
    let mut war_reg = WarRegistry::default();
    war_reg.wars.insert(war.id, war);

    let states = vec![
        StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            ..Default::default()
        },
        StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            ..Default::default()
        },
        StateData {
            id: StateId(3),
            owner_country_id: CountryId(3),
            ..Default::default()
        },
        StateData {
            id: StateId(4),
            owner_country_id: CountryId(4),
            ..Default::default()
        },
    ];
    let mut state_reg = StateRegistry::build(states);
    let mut mil_reg = MilitaryRegistry::default();
    let mut battle_reg = crate::military::battle::BattleRegistry::default();
    let mut dip_reg = crate::diplomacy::data::DiplomacyRegistry::default();
    let mut fl_reg = crate::war::frontline::FrontlineRegistry::default();
    let mut country_reg = CountryRegistry {
        countries: vec![0, 1, 3, 4]
            .into_iter()
            .map(|id| crate::country::CountryData {
                id: CountryId(id),
                capital_state_id: StateId(999), // 首都再配置ロジックを誘発しない
                ..Default::default()
            })
            .collect(),
    };
    let mut army_reg = crate::military::army::ArmyRegistry::default();
    let mut justification_reg = WarJustificationRegistry::default();
    let mut power_reg = crate::country::power::CountryPowerRegistry::default();

    execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
    .unwrap();

    assert_eq!(
        state_reg.get(StateId(1)).unwrap().owner_country_id,
        CountryId(0),
        "target_state must go to the primary attacker (crisis initiator), not a supporter"
    );
    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(0))
            .unwrap()
            .status,
        ClaimStatus::Consumed
    );
}

/// 要求テスト13,30: 攻撃側/防御側のHashSet挿入順を変えても結果(領土受領国)が
/// 変わらない(`primary_attacker`/`primary_defender`は明示Optionフィールドであり、
/// HashSet走査順に依存しない)。
#[test]
fn supporter_insertion_order_does_not_affect_the_territory_recipient() {
    fn run_with_order(attacker_order: &[usize], defender_order: &[usize]) -> CountryId {
        let claim = p21_019_claim(0, 0, 1, ClaimStatus::Active);
        let mut claim_reg = ClaimRegistry::default();
        claim_reg.claims.insert(claim.id, claim);
        let crisis = p21_019_crisis(0, 0, 1, 1, Some(crate::common::ClaimId(0)));
        let mut crisis_reg = CrisisRegistry::default();
        crisis_reg.crises.insert(crisis.id, crisis);
        let war = p21_019_war(
            1,
            attacker_order,
            defender_order,
            0,
            1,
            1,
            Some(crate::common::DiplomaticCrisisId(0)),
        );
        let mut war_reg = WarRegistry::default();
        war_reg.wars.insert(war.id, war);
        let states = vec![
            StateData {
                id: StateId(0),
                owner_country_id: CountryId(0),
                ..Default::default()
            },
            StateData {
                id: StateId(1),
                owner_country_id: CountryId(1),
                ..Default::default()
            },
            StateData {
                id: StateId(3),
                owner_country_id: CountryId(3),
                ..Default::default()
            },
            StateData {
                id: StateId(4),
                owner_country_id: CountryId(4),
                ..Default::default()
            },
        ];
        let mut state_reg = StateRegistry::build(states);
        let mut mil_reg = MilitaryRegistry::default();
        let mut battle_reg = crate::military::battle::BattleRegistry::default();
        let mut dip_reg = crate::diplomacy::data::DiplomacyRegistry::default();
        let mut fl_reg = crate::war::frontline::FrontlineRegistry::default();
        let mut country_reg = CountryRegistry {
            countries: vec![0, 1, 3, 4]
                .into_iter()
                .map(|id| crate::country::CountryData {
                    id: CountryId(id),
                    capital_state_id: StateId(999), // 首都再配置ロジックを誘発しない
                    ..Default::default()
                })
                .collect(),
        };
        let mut army_reg = crate::military::army::ArmyRegistry::default();
        let mut justification_reg = WarJustificationRegistry::default();
        let mut power_reg = crate::country::power::CountryPowerRegistry::default();

        execute_peace_settlement(
            crate::common::WarId(1),
            PeaceTerm::CedeWarGoalRegion,
            "Test",
            "1936/06/01",
            &mut state_reg,
            &mut war_reg,
            &mut mil_reg,
            &mut battle_reg,
            &mut dip_reg,
            &mut fl_reg,
            &mut crisis_reg,
            &mut claim_reg,
            &mut country_reg,
            &mut army_reg,
            &mut justification_reg,
            &mut power_reg,
        )
        .unwrap();

        state_reg.get(StateId(1)).unwrap().owner_country_id
    }

    let a = run_with_order(&[0, 3], &[1, 4]);
    let b = run_with_order(&[3, 0], &[4, 1]);
    assert_eq!(a, b);
    assert_eq!(a, CountryId(0));
}

/// 要求テスト16: 対象州以外の無関係な州は講和で一切変更されない。
#[test]
fn unrelated_states_are_not_touched_by_crisis_settlement() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();
    let s9 = StateData {
        id: StateId(9),
        owner_country_id: CountryId(9),
        ..Default::default()
    };
    let mut states = state_reg.states.clone();
    states.push(s9);
    state_reg = StateRegistry::build(states);

    execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
    .unwrap();

    assert_eq!(
        state_reg.get(StateId(9)).unwrap().owner_country_id,
        CountryId(9)
    );
}

/// P21-020 §14要求25,26,27,28: **通常War**(source_crisis_id=None)でも、
/// 対象州の法的所有国が(戦争中に)無関係な第三国へ変わっていた場合、講和を
/// 安全に拒否し、州所有権・首都・War状態・休戦のいずれも変更しない。
/// P21-019まではこの保護がCrisis由来Warにしか適用されない非対称があった
/// (通常Warは共通API `state::ownership::preflight_transfer_state_ownership`の
/// `expected_current_owner`検証で、Crisis由来Warと全く同じ保護を受ける)。
#[test]
fn normal_war_target_state_owned_by_unrelated_third_party_is_safely_rejected() {
    let (
        mut war_reg,
        _crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();
    // 通常War化(Crisis由来ではない)。
    war_reg
        .wars
        .get_mut(&crate::common::WarId(1))
        .unwrap()
        .source_crisis_id = None;
    let mut crisis_reg = CrisisRegistry::default();
    // target_state(1)の法的所有国を、attacker(0)でもdefender(1)でもない
    // 第三国(9)へ差し替える。
    state_reg.get_mut(StateId(1)).unwrap().owner_country_id = CountryId(9);
    country_reg.countries.push(crate::country::CountryData {
        id: CountryId(9),
        capital_state_id: StateId(999),
        ..Default::default()
    });

    let before_war_status = war_reg.wars.get(&crate::common::WarId(1)).unwrap().status;

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert_eq!(result, Err("state_error.ownership.current_owner_mismatch"));
    assert_eq!(
        state_reg.get(StateId(1)).unwrap().owner_country_id,
        CountryId(9),
        "the third party's state must not be taken"
    );
    assert_eq!(
        war_reg.wars.get(&crate::common::WarId(1)).unwrap().status,
        before_war_status,
        "the war must not end"
    );
    assert!(
        dip_reg
            .get(CountryId(0), CountryId(1))
            .is_none_or(|r| r.truce_until.is_none()),
        "no truce must be created"
    );
}

/// 要求テスト17,18: 対象州の法的所有国が(戦争中に)無関係な第三国へ変わっていた
/// 場合、講和自体を安全に拒否し、州所有権もClaim状態も一切変更しない
/// (一部だけ移転する部分適用を残さない)。
#[test]
fn target_state_owned_by_unrelated_third_party_is_safely_rejected() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();
    // target_state(1)の法的所有国を、Crisis targetでもinitiatorでもない第三国(9)へ差し替える。
    state_reg.get_mut(StateId(1)).unwrap().owner_country_id = CountryId(9);

    let before = p21_019_snapshot(
        &war_reg,
        &claim_reg,
        &state_reg,
        crate::common::ClaimId(0),
        crate::common::WarId(1),
    );

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert_eq!(result, Err("state_error.ownership.current_owner_mismatch"));
    let after = p21_019_snapshot(
        &war_reg,
        &claim_reg,
        &state_reg,
        crate::common::ClaimId(0),
        crate::common::WarId(1),
    );
    assert_eq!(
        before, after,
        "a rejected settlement must leave everything unchanged"
    );
    assert_eq!(
        state_reg.get(StateId(1)).unwrap().owner_country_id,
        CountryId(9)
    );
}

/// 要求テスト19: 存在しないCrisis IDを参照するWarの講和は拒否される。
#[test]
fn dangling_source_crisis_id_is_rejected() {
    let (
        mut war_reg,
        _crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();
    let mut empty_crisis_reg = CrisisRegistry::default(); // Crisisを一切登録しない

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut empty_crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert_eq!(result, Err("war_error.peace.crisis_lineage_missing_crisis"));
    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(0))
            .unwrap()
            .status,
        ClaimStatus::Active
    );
    assert_eq!(
        war_reg.wars.get(&crate::common::WarId(1)).unwrap().status,
        crate::war::data::WarStatus::Active
    );
}

/// 要求テスト20: Crisisが参照するClaim IDがClaim Registryに存在しない場合は拒否される。
#[test]
fn dangling_claim_reference_is_rejected() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();
    claim_reg.claims.clear(); // Claim自体を消す(related_claim_idは残ったまま)
    let _ = &mut crisis_reg;

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert_eq!(result, Err("war_error.peace.crisis_lineage_missing_claim"));
    assert_eq!(
        war_reg.wars.get(&crate::common::WarId(1)).unwrap().status,
        crate::war::data::WarStatus::Active
    );
}

/// 要求テスト21: War primary attackerとCrisis initiatorが食い違う場合は拒否される。
#[test]
fn attacker_crisis_initiator_mismatch_is_rejected() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();
    // Warのprimary_attackerだけを、Crisis initiator(0)と異なる国(7)へ差し替える
    // (attackers集合にも7を含めておかないと`primary_attacker_id()`のフォールバック
    // 不変条件検証以前に別のズレを作ってしまうため、両方合わせて差し替える)。
    if let Some(w) = war_reg.wars.get_mut(&crate::common::WarId(1)) {
        w.primary_attacker = Some(CountryId(7));
        w.attackers = vec![CountryId(7)].into_iter().collect();
    }

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert_eq!(
        result,
        Err("war_error.peace.crisis_lineage_attacker_mismatch")
    );
    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(0))
            .unwrap()
            .status,
        ClaimStatus::Active
    );
}

/// 要求テスト22: War primary defenderとCrisis targetが食い違う場合は拒否される。
#[test]
fn defender_crisis_target_mismatch_is_rejected() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();
    if let Some(w) = war_reg.wars.get_mut(&crate::common::WarId(1)) {
        w.primary_defender = Some(CountryId(7));
        w.defenders = vec![CountryId(7)].into_iter().collect();
    }

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert_eq!(
        result,
        Err("war_error.peace.crisis_lineage_defender_mismatch")
    );
}

/// 要求テスト23: Claimのclaimant国がCrisis initiatorと食い違う場合は拒否される。
#[test]
fn claim_claimant_crisis_initiator_mismatch_is_rejected() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();
    claim_reg
        .claims
        .get_mut(&crate::common::ClaimId(0))
        .unwrap()
        .claimant_country = CountryId(7);

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert_eq!(
        result,
        Err("war_error.peace.crisis_lineage_claimant_mismatch")
    );
}

/// 要求テスト24: Claimのtarget_stateがCrisis自身のwar_goal target_stateと
/// 食い違う場合は拒否される。
#[test]
fn claim_target_state_crisis_target_state_mismatch_is_rejected() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();
    claim_reg
        .claims
        .get_mut(&crate::common::ClaimId(0))
        .unwrap()
        .target_state = StateId(99);

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert_eq!(
        result,
        Err("war_error.peace.crisis_lineage_target_state_mismatch")
    );
}

/// 要求テスト25: Crisis自身のtarget_stateがState Registryに存在しない場合は拒否される。
#[test]
fn crisis_target_state_missing_from_state_registry_is_rejected() {
    let claim = p21_019_claim(0, 0, 99, ClaimStatus::Active); // 存在しないState(99)
    let mut claim_reg = ClaimRegistry::default();
    claim_reg.claims.insert(claim.id, claim);
    let crisis = p21_019_crisis(0, 0, 1, 99, Some(crate::common::ClaimId(0)));
    let mut crisis_reg = CrisisRegistry::default();
    crisis_reg.crises.insert(crisis.id, crisis);
    let war = p21_019_war(
        1,
        &[0],
        &[1],
        0,
        1,
        99,
        Some(crate::common::DiplomaticCrisisId(0)),
    );
    let mut war_reg = WarRegistry::default();
    war_reg.wars.insert(war.id, war);

    let states = vec![
        StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            ..Default::default()
        },
        StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            ..Default::default()
        },
        // StateId(99)は存在しない
    ];
    let mut state_reg = StateRegistry::build(states);
    let mut mil_reg = MilitaryRegistry::default();
    let mut battle_reg = crate::military::battle::BattleRegistry::default();
    let mut dip_reg = crate::diplomacy::data::DiplomacyRegistry::default();
    let mut fl_reg = crate::war::frontline::FrontlineRegistry::default();
    let mut country_reg = CountryRegistry::default();
    let mut army_reg = crate::military::army::ArmyRegistry::default();
    let mut justification_reg = WarJustificationRegistry::default();
    let mut power_reg = crate::country::power::CountryPowerRegistry::default();

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert_eq!(result, Err("war_error.peace.crisis_lineage_state_missing"));
}

/// 要求テスト26,27,28: lineage検証失敗時、War/Claim/State Registryはbefore/afterで
/// 完全に一致する(捏造IDが紛れ込んでいないこと自体も、消費されたClaim数・War数が
/// 増減していないことで間接的に確認する)。Warは`Active`のまま終了しない。
#[test]
fn lineage_failure_leaves_every_registry_completely_unchanged() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();
    claim_reg
        .claims
        .get_mut(&crate::common::ClaimId(0))
        .unwrap()
        .claimant_country = CountryId(7);

    let claims_before = format!("{:?}", {
        let mut v: Vec<_> = claim_reg.claims.iter().collect();
        v.sort_by_key(|(k, _)| k.0);
        v
    });
    let wars_before = format!("{:?}", {
        let mut v: Vec<_> = war_reg.wars.iter().collect();
        v.sort_by_key(|(k, _)| k.0);
        v
    });
    let states_before = format!("{:?}", state_reg.states);

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );
    assert!(result.is_err());

    let claims_after = format!("{:?}", {
        let mut v: Vec<_> = claim_reg.claims.iter().collect();
        v.sort_by_key(|(k, _)| k.0);
        v
    });
    let wars_after = format!("{:?}", {
        let mut v: Vec<_> = war_reg.wars.iter().collect();
        v.sort_by_key(|(k, _)| k.0);
        v
    });
    let states_after = format!("{:?}", state_reg.states);

    assert_eq!(
        claims_before, claims_after,
        "ClaimRegistry must be byte-identical"
    );
    assert_eq!(
        wars_before, wars_after,
        "WarRegistry must be byte-identical"
    );
    assert_eq!(
        states_before, states_after,
        "StateRegistry must be byte-identical"
    );
    assert_eq!(
        war_reg.wars.get(&crate::common::WarId(1)).unwrap().status,
        crate::war::data::WarStatus::Active,
        "the war must not end when lineage validation fails"
    );
}

/// 要求テスト32: Crisis由来Warの`source_crisis_id`は講和完了後も維持される
/// (履歴として保持、消去・書き換えされない)。
#[test]
fn source_crisis_id_survives_the_peace_settlement() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env();

    execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
    .unwrap();

    assert_eq!(
        war_reg
            .wars
            .get(&crate::common::WarId(1))
            .unwrap()
            .source_crisis_id,
        Some(crate::common::DiplomaticCrisisId(0))
    );
}

// ─────────────────────────────────────────────────────────────────────
// P21-019フォローアップ(ユーザー指摘対応、2026-08-21): 首都割譲時の
// capital_state_id再割当・残存州なしの安全拒否
// ─────────────────────────────────────────────────────────────────────

/// P21-019フォローアップ用の環境: claimant=0、target=1(首都=StateId(1)、
/// 割譲対象そのもの)。`extra_target_states`でtargetに追加で持たせる陸州を
/// 指定できる(空なら「残存州なし」シナリオ、非空なら「再割当成功」
/// シナリオを作れる)。
#[allow(clippy::type_complexity)]
fn p21_019_capital_env(
    extra_target_states: &[usize],
) -> (
    WarRegistry,
    CrisisRegistry,
    ClaimRegistry,
    StateRegistry,
    MilitaryRegistry,
    crate::military::battle::BattleRegistry,
    crate::diplomacy::data::DiplomacyRegistry,
    crate::war::frontline::FrontlineRegistry,
    CountryRegistry,
    crate::military::army::ArmyRegistry,
    WarJustificationRegistry,
    crate::country::power::CountryPowerRegistry,
) {
    let claim = p21_019_claim(0, 0, 1, ClaimStatus::Active);
    let claim_id = claim.id;
    let mut claim_reg = ClaimRegistry::default();
    claim_reg.claims.insert(claim_id, claim);

    let crisis = p21_019_crisis(0, 0, 1, 1, Some(claim_id));
    let mut crisis_reg = CrisisRegistry::default();
    crisis_reg.crises.insert(crisis.id, crisis);

    let war = p21_019_war(
        1,
        &[0],
        &[1],
        0,
        1,
        1,
        Some(crate::common::DiplomaticCrisisId(0)),
    );
    let mut war_reg = WarRegistry::default();
    war_reg.wars.insert(war.id, war);

    let mut states = vec![
        StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            ..Default::default()
        },
        StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            ..Default::default()
        },
    ];
    for &sid in extra_target_states {
        states.push(StateData {
            id: StateId(sid),
            owner_country_id: CountryId(1),
            ..Default::default()
        });
    }
    let state_reg = StateRegistry::build(states);

    // target(1)の首都は割譲対象そのもの(StateId(1))。
    let country_reg = CountryRegistry {
        countries: vec![
            CountryData {
                id: CountryId(0),
                capital_state_id: StateId(0),
                ..CountryData::default()
            },
            CountryData {
                id: CountryId(1),
                capital_state_id: StateId(1),
                ..CountryData::default()
            },
        ],
    };

    (
        war_reg,
        crisis_reg,
        claim_reg,
        state_reg,
        MilitaryRegistry::default(),
        crate::military::battle::BattleRegistry::default(),
        crate::diplomacy::data::DiplomacyRegistry::default(),
        crate::war::frontline::FrontlineRegistry::default(),
        country_reg,
        crate::military::army::ArmyRegistry::default(),
        WarJustificationRegistry::default(),
        crate::country::power::CountryPowerRegistry::default(),
    )
}

/// 要求(ユーザー指摘対応): 割譲対象州が現所有国の首都であっても、他に残存する
/// 陸州があれば、最小StateIdの州へ決定論的に首都が再割当され、講和自体は
/// 成功する(領土移転・Claim消費も通常どおり行われる)。
#[test]
fn cede_capital_reassigns_to_lowest_remaining_state_id_and_succeeds() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_capital_env(&[5, 3]); // targetはStateId(1)(首都)に加え3・5も所有

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert!(
        result.is_ok(),
        "ceding a capital must still succeed when the owner has other land states: {result:?}"
    );

    // 領土は通常どおりclaimantへ移転し、Claimも消費される。
    assert_eq!(
        state_reg.get(StateId(1)).unwrap().owner_country_id,
        CountryId(0)
    );
    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(0))
            .unwrap()
            .status,
        ClaimStatus::Consumed
    );

    // 首都は残存州のうち最小StateId(3と5のうち3)へ決定論的に再割当される。
    assert_eq!(
        country_reg.get(CountryId(1)).unwrap().capital_state_id,
        StateId(3),
        "capital must be reassigned to the lowest-StateId remaining land state"
    );
}

/// P21-021要求: 首都割譲後に対象国が残存する陸州を一つも持たない場合、
/// 講和は「安全に拒否」ではなく国家消滅(完全併合)をトリガーする——対象州は
/// attackerへ移転し、target(CountryId(1))は`CountryStatus::Eliminated`へ
/// 遷移し(`CountryRegistry`からは物理削除されない)、Claimは消費され、
/// causing warである`WarId(1)`は通常どおり`AttackerVictory`で終結する。
/// P21-020時点の旧挙動("safely rejected")はこのテストが意図的に反転させる
/// (旧テスト名・旧アサーションは本タスクの対象外仕様だったため置き換える)。
#[test]
fn cede_capital_with_no_remaining_states_triggers_country_elimination() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_capital_env(&[]); // targetはStateId(1)(首都)のみ所有

    let result = execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );

    assert!(
        result.is_ok(),
        "ceding the target's last land state must trigger elimination, not fail: {result:?}"
    );

    assert_eq!(
        state_reg.get(StateId(1)).unwrap().owner_country_id,
        CountryId(0),
        "the final state must transfer to the attacker"
    );
    assert_eq!(
        claim_reg
            .claims
            .get(&crate::common::ClaimId(0))
            .unwrap()
            .status,
        ClaimStatus::Consumed,
        "the fulfilling claim must be consumed"
    );
    let eliminated = country_reg.get(CountryId(1)).unwrap();
    assert!(
        eliminated.is_eliminated(),
        "the target must transition to Eliminated, not be dropped from the registry"
    );
    assert_eq!(
        eliminated.capital_state_id,
        StateId(1),
        "capital_state_id is kept as the last historical capital, not cleared"
    );
    assert_eq!(
        war_reg.wars.get(&crate::common::WarId(1)).unwrap().status,
        crate::war::data::WarStatus::AttackerVictory,
        "the causing war must end normally in an attacker victory"
    );
}

/// 要求(ユーザー指摘対応): 首都以外の州を割譲する場合は、これまでどおり
/// capital_state_idを一切変更しない(回帰確認)。
#[test]
fn cede_non_capital_state_does_not_touch_capital_state_id() {
    let (
        mut war_reg,
        mut crisis_reg,
        mut claim_reg,
        mut state_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = p21_019_valid_env(); // このヘルパーはtargetの首都をStateId(999)(割譲対象外)に設定済み

    execute_peace_settlement(
        crate::common::WarId(1),
        PeaceTerm::CedeWarGoalRegion,
        "Test",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
    .unwrap();

    assert_eq!(
        country_reg.get(CountryId(1)).unwrap().capital_state_id,
        StateId(999),
        "capital must be untouched when the ceded state is not the capital"
    );
}
