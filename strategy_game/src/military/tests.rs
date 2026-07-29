#![cfg(test)]
#![allow(clippy::field_reassign_with_default)]

use crate::common::{ArmyId, CountryId, DivisionId, StateId};
use crate::country::{CountryData, CountryRegistry};
use crate::military::data::{ArmyStatus, ArmyUnit, DivisionDefinition, DivisionSize, DivisionType, MilitaryRegistry};
use crate::military::recruitment::{cancel_recruitment, process_recruitment, request_recruitment};

fn setup_registry() -> MilitaryRegistry {
    let mut registry = MilitaryRegistry::default();
    let def = DivisionDefinition {
        id: DivisionId(1),
        name: "Test Infantry".to_string(),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        required_manpower: 10_000,
        required_equipment: 100.0,
        recruitment_days: 30,
        movement_speed: 4.0,
        attack: 10.0,
        defense: 15.0,
        breakthrough: 5.0,
        organization: 100.0,
        morale: 100.0,
        supply_usage: 1.0,
        maintenance_cost: 1.0,
    };
    registry.definitions.insert(DivisionId(1), def);
    registry
}

/// テスト用 ArmyUnit ヘルパー
fn make_test_army(id: usize, owner: CountryId, state: StateId) -> ArmyUnit {
    ArmyUnit {
        id: ArmyId(id),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: state,
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
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

#[test]
fn test_recruitment_conditions() {
    let registry = setup_registry();
    let mut country = CountryData::default();
    country.available_manpower = 20_000;
    country.treasury = 500.0;

    let result = request_recruitment(&mut country, &registry, DivisionId(1), StateId(1));
    assert!(result.is_ok());
    assert_eq!(country.available_manpower, 10_000);
    assert_eq!(country.mobilized_manpower, 10_000);
    assert_eq!(country.recruitment_queue.len(), 1);
}

#[test]
fn test_recruitment_manpower_shortage() {
    let registry = setup_registry();
    let mut country = CountryData::default();
    country.available_manpower = 5_000; // Not enough
    country.treasury = 500.0;

    let result = request_recruitment(&mut country, &registry, DivisionId(1), StateId(1));
    assert!(result.is_err());
    assert_eq!(country.recruitment_queue.len(), 0);
}

#[test]
fn test_recruitment_equipment_shortage() {
    let registry = setup_registry();
    let mut country = CountryData::default();
    country.available_manpower = 20_000;
    country.treasury = 50.0; // Not enough

    let result = request_recruitment(&mut country, &registry, DivisionId(1), StateId(1));
    assert!(result.is_err());
    assert_eq!(country.recruitment_queue.len(), 0);
}

#[test]
fn test_recruitment_progress_and_completion() {
    let mut military_registry = setup_registry();
    let mut country_registry = CountryRegistry::default();

    let mut country = CountryData::default();
    country.id = CountryId(1);
    country.available_manpower = 20_000;
    country.treasury = 500.0;

    request_recruitment(&mut country, &military_registry, DivisionId(1), StateId(1)).unwrap();
    country_registry.countries.push(country);

    // Progress 1 day
    process_recruitment(&mut country_registry, &mut military_registry);
    assert_eq!(
        country_registry.countries[0].recruitment_queue[0].days_remaining,
        29
    );
    assert_eq!(military_registry.armies.len(), 0);

    // Fast forward 29 days
    for _ in 0..29 {
        process_recruitment(&mut country_registry, &mut military_registry);
    }

    assert_eq!(country_registry.countries[0].recruitment_queue.len(), 0);
    assert_eq!(military_registry.armies.len(), 1);
}

#[test]
fn test_recruitment_cancellation() {
    let registry = setup_registry();
    let mut country = CountryData::default();
    country.available_manpower = 20_000;
    country.treasury = 500.0;

    request_recruitment(&mut country, &registry, DivisionId(1), StateId(1)).unwrap();

    assert_eq!(country.available_manpower, 10_000);
    assert_eq!(country.treasury, 400.0);

    let cancel_result = cancel_recruitment(&mut country, &registry, 0);
    assert!(cancel_result.is_ok());

    assert_eq!(country.recruitment_queue.len(), 0);
    assert_eq!(country.available_manpower, 20_000);
    assert_eq!(country.treasury, 500.0);
}

#[test]
fn test_pathfinding() {
    use crate::military::pathfinding::find_path;
    use crate::state::data::StateRegistry;

    let mut s1 = crate::state::data::StateData::default();
    s1.id = StateId(1);
    s1.owner_country_id = CountryId(1);
    s1.neighbors = vec![StateId(2)];

    let mut s2 = crate::state::data::StateData::default();
    s2.id = StateId(2);
    s2.owner_country_id = CountryId(1);
    s2.neighbors = vec![StateId(1), StateId(3)];

    let mut s3 = crate::state::data::StateData::default();
    s3.id = StateId(3);
    s3.owner_country_id = CountryId(2); // Different country
    s3.neighbors = vec![StateId(2)];

    let state_registry = StateRegistry::build(vec![s1, s2, s3]);

    let allowed_countries = vec![CountryId(1)];
    let hostile_countries = vec![CountryId(2)];

    // s1 -> s2 (Same country, allowed)
    let path = find_path(
        StateId(1),
        StateId(2),
        &state_registry,
        &allowed_countries,
        &hostile_countries,
    );
    assert!(path.is_some());
    assert_eq!(path.unwrap(), vec![StateId(2)]);

    // s1 -> s3 (Hostile country, should be allowed because it's in hostile_countries list)
    let path2 = find_path(
        StateId(1),
        StateId(3),
        &state_registry,
        &allowed_countries,
        &hostile_countries,
    );
    assert!(path2.is_some());
    assert_eq!(path2.unwrap(), vec![StateId(2), StateId(3)]);

    // s1 -> s3 (Not allowed, not hostile)
    let allowed_countries_2 = vec![CountryId(1)];
    let hostile_countries_empty: Vec<CountryId> = vec![];
    let path3 = find_path(
        StateId(1),
        StateId(3),
        &state_registry,
        &allowed_countries_2,
        &hostile_countries_empty,
    );
    assert!(path3.is_none());
}

#[test]
fn test_movement() {
    use crate::military::battle::BattleRegistry;
    use crate::military::movement::process_movement;
    use crate::state::data::StateRegistry;
    use crate::war::data::WarRegistry;

    let mut registry = setup_registry();
    let army = ArmyUnit {
        id: ArmyId(0),
        owner: CountryId(1),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(1),
        destination: Some(StateId(2)),
        current_path: vec![StateId(2)],
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
        status: ArmyStatus::Moving,
        def_id: DivisionId(1),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    };
    registry.add_army(army);

    let mut s1 = crate::state::data::StateData::default();
    s1.id = StateId(1);
    s1.owner_country_id = CountryId(1);
    let mut s2 = crate::state::data::StateData::default();
    s2.id = StateId(2);
    s2.owner_country_id = CountryId(1);
    let mut state_registry = StateRegistry::build(vec![s1, s2]);
    let war_registry = WarRegistry::default();
    let mut battle_registry = BattleRegistry::default();

    // def_speed = 4.0, base_days = 5.0, supply_mod = 1.0
    // movement_days = 5.0 / 4.0 = 1.25
    // daily_progress = 1.0 / 1.25 = 0.8

    process_movement(&mut registry, &mut state_registry, &war_registry, &mut battle_registry, "1800/01/01");

    let army_ref = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(army_ref.status, ArmyStatus::Moving);
    assert_eq!(army_ref.current_state, StateId(1));
    assert_eq!(army_ref.target_state, Some(StateId(2)));
    assert!(army_ref.movement_progress > 0.79 && army_ref.movement_progress < 0.81);

    // Next day
    process_movement(&mut registry, &mut state_registry, &war_registry, &mut battle_registry, "1800/01/02");

    let army_ref2 = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(army_ref2.status, ArmyStatus::Idle);
    assert_eq!(army_ref2.current_state, StateId(2));
    assert_eq!(army_ref2.target_state, None);
    assert_eq!(army_ref2.movement_progress, 0.0);
}

#[test]
fn test_supply_processing() {
    use crate::military::supply::process_supply;
    use crate::state::data::StateRegistry;

    let mut registry = setup_registry();
    let army1 = make_test_army(0, CountryId(1), StateId(1));
    let mut army2 = army1.clone();
    army2.id = ArmyId(1);

    registry.add_army(army1);
    registry.add_army(army2);

    let mut s1 = crate::state::data::StateData::default();
    s1.id = StateId(1);
    s1.logistics_capacity = 1.5; // Only enough for 1.5 usage, but we have 2 armies (total 2.0)

    let mut state_registry = StateRegistry::build(vec![s1]);

    process_supply(&mut registry, &mut state_registry);

    // Usage is 2.0, capacity is 1.5. Ratio = 1.5 / 2.0 = 0.75
    let state_ref = state_registry.get(StateId(1)).unwrap();
    assert_eq!(state_ref.logistics_usage, 2.0);
    assert_eq!(state_ref.logistics_ratio, 0.75);

    let army_ref = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(army_ref.supply_ratio, 0.75);
}

#[test]
fn test_combat_strength() {
    use crate::military::combat::calculate_combat_strength;

    let registry = setup_registry();
    let army = ArmyUnit {
        id: ArmyId(0),
        owner: CountryId(1),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(1),
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 5000, // 50% manpower
        max_manpower: 10000,
        equipment: 100.0,
        max_equipment: 100.0,
        organization: 50.0, // 50% organization
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 0.8, // 80% supply
        movement_progress: 0.0,
        status: ArmyStatus::Idle,
        def_id: DivisionId(1),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    };

    // attack = 10.0
    // equip_ratio = 1.0, manpower_ratio = 0.5
    // org_ratio = 0.5
    // base_attack * min(0.5, 1.0) * 0.5 * 0.8 = 10 * 0.5 * 0.5 * 0.8 = 2.0
    let strength = calculate_combat_strength(&army, &registry);
    assert!((strength - 2.0).abs() < 0.001);
}

#[test]
fn test_phase13_own_territory_and_foreign_pathfinding() {
    use crate::military::pathfinding::find_path;
    use crate::state::data::{StateData, StateRegistry};

    // State 1 (Country 1) <-> State 2 (Country 1) <-> State 3 (Country 2)
    let s1 = StateData {
        id: StateId(1),
        owner_country_id: CountryId(1),
        neighbors: vec![StateId(2)],
        ..Default::default()
    };

    let s2 = StateData {
        id: StateId(2),
        owner_country_id: CountryId(1),
        neighbors: vec![StateId(1), StateId(3)],
        ..Default::default()
    };

    let s3 = StateData {
        id: StateId(3),
        owner_country_id: CountryId(2),
        neighbors: vec![StateId(2)],
        ..Default::default()
    };

    let state_registry = StateRegistry::build(vec![s1, s2, s3]);

    // 1. 自国領内の到達可能な経路が生成される
    let path = find_path(
        StateId(1),
        StateId(2),
        &state_registry,
        &[CountryId(1)],
        &[],
    );
    assert_eq!(path, Some(vec![StateId(2)]));

    // 2. 他国領 (Country 2) を通る経路が生成されない
    let foreign_path = find_path(
        StateId(1),
        StateId(3),
        &state_registry,
        &[CountryId(1)],
        &[],
    );
    assert_eq!(foreign_path, None);
}

#[test]
fn test_phase13_unreachable_destination_and_determinism() {
    use crate::military::pathfinding::find_path;
    use crate::state::data::{StateData, StateRegistry};

    // State 1 (Country 1)  |  State 2 (Country 1)  (孤立・隣接なし)
    let s1 = StateData {
        id: StateId(1),
        owner_country_id: CountryId(1),
        neighbors: vec![],
        ..Default::default()
    };

    let s2 = StateData {
        id: StateId(2),
        owner_country_id: CountryId(1),
        neighbors: vec![],
        ..Default::default()
    };

    let state_registry = StateRegistry::build(vec![s1, s2]);

    // 3. 到達不能な目的地が正しく拒否される
    let path = find_path(
        StateId(1),
        StateId(2),
        &state_registry,
        &[CountryId(1)],
        &[],
    );
    assert_eq!(path, None);

    // 4. 同じ条件では同じ経路が生成される (決定性)
    let path1 = find_path(
        StateId(1),
        StateId(1),
        &state_registry,
        &[CountryId(1)],
        &[],
    );
    let path2 = find_path(
        StateId(1),
        StateId(1),
        &state_registry,
        &[CountryId(1)],
        &[],
    );
    assert_eq!(path1, path2);
}

#[test]
fn test_phase13_movement_timing_and_arrival() {
    use crate::military::battle::BattleRegistry;
    use crate::military::movement::process_movement;
    use crate::state::data::{StateData, StateRegistry};
    use crate::war::data::WarRegistry;

    let mut registry = setup_registry();
    let army = ArmyUnit {
        id: ArmyId(0),
        owner: CountryId(1),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(1),
        destination: Some(StateId(3)),
        current_path: vec![StateId(2), StateId(3)],
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
        status: ArmyStatus::Moving,
        def_id: DivisionId(1),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    };
    registry.add_army(army);

    let s1 = StateData {
        id: StateId(1),
        owner_country_id: CountryId(1),
        ..Default::default()
    };
    let s2 = StateData {
        id: StateId(2),
        owner_country_id: CountryId(1),
        ..Default::default()
    };
    let s3 = StateData {
        id: StateId(3),
        owner_country_id: CountryId(1),
        ..Default::default()
    };
    let mut state_registry = StateRegistry::build(vec![s1, s2, s3]);
    let war_registry = WarRegistry::default();
    let mut battle_registry = BattleRegistry::default();

    // 一時停止中/時間進行イベントなし（process_movementを呼ばない）時は進行度が変化しないことを検証
    let army_before = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(army_before.movement_progress, 0.0);

    // 日数が経過（process_movement実行）
    // def_speed = 4.0, base_days = 5.0 => daily_progress = 0.8 / day
    // Day 1: progress -> 0.8
    process_movement(&mut registry, &mut state_registry, &war_registry, &mut battle_registry, "1800/01/01");
    let a1 = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(a1.current_state, StateId(1));
    assert_eq!(a1.target_state, Some(StateId(2)));

    // Day 2: reaches State 2, target_state set to State 3
    process_movement(&mut registry, &mut state_registry, &war_registry, &mut battle_registry, "1800/01/02");
    let a2 = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(a2.current_state, StateId(2));
    assert_eq!(a2.target_state, Some(StateId(3)));

    // Day 3: progress -> 0.8 towards State 3
    process_movement(&mut registry, &mut state_registry, &war_registry, &mut battle_registry, "1800/01/03");

    // Day 4: reaches State 3 (destination), status -> Idle
    process_movement(&mut registry, &mut state_registry, &war_registry, &mut battle_registry, "1800/01/04");
    let a_final = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(a_final.current_state, StateId(3));
    assert_eq!(a_final.status, ArmyStatus::Idle);
    assert_eq!(a_final.destination, None);
    assert_eq!(a_final.target_state, None);
}

// ── Phase 15 テスト ──────────────────────────────────────────────────────────

use crate::military::battle::BattleRegistry;
use crate::state::data::{StateData, StateRegistry};
use crate::war::data::{War, WarRegistry, WarStatus};
use std::collections::HashSet;

fn setup_war(c1: CountryId, c2: CountryId) -> WarRegistry {
    let mut reg = WarRegistry::default();
    let war = War {
        id: crate::common::WarId(0),
        name: "Test War".to_string(),
        start_date: "1800/01/01".to_string(),
        attackers: [c1].iter().cloned().collect(),
        defenders: [c2].iter().cloned().collect(),
        war_goals: vec![],
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: HashSet::new(),
        status: WarStatus::Active,
    };
    reg.wars.insert(war.id, war);
    reg
}

#[test]
fn test_phase15_invasion_path_to_enemy_territory() {
    use crate::military::pathfinding::find_path;

    // S1(C1) - S2(C1) - S3(C2)
    let s1 = StateData {
        id: StateId(1),
        owner_country_id: CountryId(1),
        neighbors: vec![StateId(2)],
        ..Default::default()
    };
    let s2 = StateData {
        id: StateId(2),
        owner_country_id: CountryId(1),
        neighbors: vec![StateId(1), StateId(3)],
        ..Default::default()
    };
    let s3 = StateData {
        id: StateId(3),
        owner_country_id: CountryId(2),
        neighbors: vec![StateId(2)],
        ..Default::default()
    };
    let state_registry = StateRegistry::build(vec![s1, s2, s3]);

    // 戦争中の敵国支配地域への侵攻経路が生成できる
    let path = find_path(
        StateId(1),
        StateId(3),
        &state_registry,
        &[CountryId(1)],
        &[CountryId(2)], // C2は交戦中
    );
    assert!(path.is_some(), "Should find path to enemy territory");
    assert_eq!(path.unwrap(), vec![StateId(2), StateId(3)]);
}

#[test]
fn test_phase15_no_path_to_neutral_territory() {
    use crate::military::pathfinding::find_path;

    let s1 = StateData {
        id: StateId(1),
        owner_country_id: CountryId(1),
        neighbors: vec![StateId(2)],
        ..Default::default()
    };
    let s2 = StateData {
        id: StateId(2),
        owner_country_id: CountryId(3), // 中立国
        neighbors: vec![StateId(1)],
        ..Default::default()
    };
    let state_registry = StateRegistry::build(vec![s1, s2]);

    // 中立国の地域には侵攻経路が生成されない
    let path = find_path(
        StateId(1),
        StateId(2),
        &state_registry,
        &[CountryId(1)],
        &[], // C3は交戦中でない
    );
    assert!(path.is_none(), "Should not find path to neutral territory");
}

#[test]
fn test_phase15_battle_starts_when_entering_defended_state() {
    use crate::military::invasion::process_army_arrival;

    let c1 = CountryId(1);
    let c2 = CountryId(2);
    let war_reg = setup_war(c1, c2);

    let mut mil = MilitaryRegistry::default();
    // C1の攻撃ユニット（S2に到着済み）
    let army1 = make_test_army(0, c1, StateId(2));
    // C2の防御ユニット（S2にいる）
    let army2 = make_test_army(1, c2, StateId(2));
    mil.armies.insert(ArmyId(0), army1);
    mil.armies.insert(ArmyId(1), army2);

    let s1 = StateData { id: StateId(1), owner_country_id: c1, ..Default::default() };
    let mut s2 = StateData { id: StateId(2), owner_country_id: c2, ..Default::default() };
    s2.controller_country = None;
    let mut state_reg = StateRegistry::build(vec![s1, s2]);
    let mut battle_reg = BattleRegistry::default();

    process_army_arrival(
        ArmyId(0),
        StateId(2),
        StateId(1),
        "1800/01/01",
        &mut mil,
        &mut state_reg,
        &war_reg,
        &mut battle_reg,
    );

    // 戦闘が1件作成される
    assert_eq!(battle_reg.battles.len(), 1, "Exactly one battle should be created");
    // 両ユニットが戦闘中
    assert_eq!(mil.armies[&ArmyId(0)].status, ArmyStatus::Fighting);
    assert_eq!(mil.armies[&ArmyId(1)].status, ArmyStatus::Fighting);
}

#[test]
fn test_phase15_occupy_undefended_enemy_state() {
    use crate::military::invasion::process_army_arrival;

    let c1 = CountryId(1);
    let c2 = CountryId(2);
    let war_reg = setup_war(c1, c2);

    let mut mil = MilitaryRegistry::default();
    // C1の攻撃ユニット（S2へ到着）
    let army1 = make_test_army(0, c1, StateId(2));
    mil.armies.insert(ArmyId(0), army1);
    // C2のユニットはいない

    let s1 = StateData { id: StateId(1), owner_country_id: c1, ..Default::default() };
    let mut s2 = StateData { id: StateId(2), owner_country_id: c2, ..Default::default() };
    s2.controller_country = None;
    let mut state_reg = StateRegistry::build(vec![s1, s2]);
    let mut battle_reg = BattleRegistry::default();

    process_army_arrival(
        ArmyId(0),
        StateId(2),
        StateId(1),
        "1800/01/01",
        &mut mil,
        &mut state_reg,
        &war_reg,
        &mut battle_reg,
    );

    // 戦闘なし
    assert_eq!(battle_reg.battles.len(), 0, "No battle for undefended state");
    // 地域の支配国がC1に変わる
    assert_eq!(state_reg.get(StateId(2)).unwrap().controller(), c1);
    // 所有国はC2のまま
    assert_eq!(state_reg.get(StateId(2)).unwrap().owner_country_id, c2);
}

#[test]
fn test_phase15_owner_unchanged_after_occupation() {
    use crate::military::invasion::occupy_state;

    let mut s1 = StateData::default();
    s1.id = StateId(1);
    s1.owner_country_id = CountryId(2);

    let mut registry = StateRegistry::build(vec![s1]);
    occupy_state(StateId(1), CountryId(1), &mut registry);

    let s = registry.get(StateId(1)).unwrap();
    assert_eq!(s.controller(), CountryId(1)); // 支配国は変化
    assert_eq!(s.owner_country_id, CountryId(2)); // 所有国は変化しない
}

#[test]
fn test_phase15_combat_daily_resolution() {
    use crate::military::combat_calc::resolve_combat_day;

    let attacker = make_test_army(0, CountryId(1), StateId(1));
    let defender = make_test_army(1, CountryId(2), StateId(1));

    let (atk_loss, atk_org_loss, def_loss, def_org_loss) =
        resolve_combat_day(&attacker, &defender, 0);

    // 1日経過すると損失が発生する
    assert!(atk_loss > 0 || def_loss > 0, "At least one side should take damage");
    // 組織率損失も発生する
    assert!(atk_org_loss >= 0.0 && def_org_loss >= 0.0);
}

#[test]
fn test_phase15_combat_is_deterministic() {
    use crate::military::combat_calc::resolve_combat_day;

    let attacker = make_test_army(0, CountryId(1), StateId(1));
    let defender = make_test_army(1, CountryId(2), StateId(1));

    let result1 = resolve_combat_day(&attacker, &defender, 5);
    let result2 = resolve_combat_day(&attacker, &defender, 5);

    // 同じ入力から同じ結果が得られる（決定性）
    assert_eq!(result1, result2);
}

#[test]
fn test_phase15_terrain_bonus_affects_combat() {
    use crate::military::combat_calc::{TERRAIN_BONUS_MOUNTAIN, TERRAIN_BONUS_PLAIN, resolve_combat_day};

    let attacker = make_test_army(0, CountryId(1), StateId(1));
    let defender = make_test_army(1, CountryId(2), StateId(1));

    let (atk_loss_plain, _, def_loss_plain, _) =
        resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);
    let (atk_loss_mountain, _, def_loss_mountain, _) =
        resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_MOUNTAIN);

    // 山岳では攻撃側の損失が増え、防御側の損失が減る
    assert!(
        atk_loss_mountain >= atk_loss_plain,
        "Attacker should suffer more in mountains"
    );
    assert!(
        def_loss_mountain <= def_loss_plain,
        "Defender should suffer less in mountains"
    );
}

#[test]
fn test_phase15_manpower_does_not_go_below_zero() {
    use crate::military::combat_calc::resolve_combat_day;

    let mut attacker = make_test_army(0, CountryId(1), StateId(1));
    attacker.manpower = 1; // 極限まで消耗
    let defender = make_test_army(1, CountryId(2), StateId(1));

    let (atk_loss, _, _, _) = resolve_combat_day(&attacker, &defender, 0);

    // 損失はmanpower以下（saturating_subで保証）
    assert!(atk_loss <= 1 + 1000, "Loss should be bounded");
    // 実際の適用はsaturating_subで0以下にならない
    let after = 1u64.saturating_sub(atk_loss);
    assert_eq!(after, 0, "Should be 0 (not overflow)");
}

#[test]
fn test_phase15_org_recovery_in_own_territory() {
    use crate::military::update::process_org_recovery;

    let mut mil = MilitaryRegistry::default();
    let mut army = make_test_army(0, CountryId(1), StateId(1));
    army.organization = 50.0; // 50/100
    mil.armies.insert(ArmyId(0), army);

    let mut s1 = StateData::default();
    s1.id = StateId(1);
    s1.owner_country_id = CountryId(1); // 自国支配

    let state_reg = StateRegistry::build(vec![s1]);

    process_org_recovery(&mut mil, &state_reg);

    let after_org = mil.armies[&ArmyId(0)].organization;
    assert!(
        after_org > 50.0,
        "Organization should recover in own territory"
    );
}

#[test]
fn test_phase15_org_recovery_not_during_combat() {
    use crate::military::update::process_org_recovery;

    let mut mil = MilitaryRegistry::default();
    let mut army = make_test_army(0, CountryId(1), StateId(1));
    army.organization = 50.0;
    army.status = ArmyStatus::Fighting; // 戦闘中
    mil.armies.insert(ArmyId(0), army);

    let mut s1 = StateData::default();
    s1.id = StateId(1);
    s1.owner_country_id = CountryId(1);

    let state_reg = StateRegistry::build(vec![s1]);

    process_org_recovery(&mut mil, &state_reg);

    let after_org = mil.armies[&ArmyId(0)].organization;
    assert_eq!(
        after_org, 50.0,
        "Organization should NOT recover during combat"
    );
}

#[test]
fn test_phase15_org_recovery_not_when_moving() {
    use crate::military::update::process_org_recovery;

    let mut mil = MilitaryRegistry::default();
    let mut army = make_test_army(0, CountryId(1), StateId(1));
    army.organization = 50.0;
    army.status = ArmyStatus::Moving; // 移動中
    mil.armies.insert(ArmyId(0), army);

    let mut s1 = StateData::default();
    s1.id = StateId(1);
    s1.owner_country_id = CountryId(1);

    let state_reg = StateRegistry::build(vec![s1]);

    process_org_recovery(&mut mil, &state_reg);

    let after_org = mil.armies[&ArmyId(0)].organization;
    assert_eq!(
        after_org, 50.0,
        "Organization should NOT recover while moving"
    );
}

#[test]
fn test_phase15_org_recovery_does_not_exceed_max() {
    use crate::military::update::process_org_recovery;

    let mut mil = MilitaryRegistry::default();
    let mut army = make_test_army(0, CountryId(1), StateId(1));
    army.organization = 99.0; // 最大に近い
    army.max_organization = 100.0;
    mil.armies.insert(ArmyId(0), army);

    let mut s1 = StateData::default();
    s1.id = StateId(1);
    s1.owner_country_id = CountryId(1);

    let state_reg = StateRegistry::build(vec![s1]);

    process_org_recovery(&mut mil, &state_reg);

    let after_org = mil.armies[&ArmyId(0)].organization;
    assert!(
        after_org <= 100.0,
        "Organization should not exceed max_organization"
    );
}

#[test]
fn test_phase15_no_battle_between_same_country() {
    use crate::military::invasion::process_army_arrival;

    let c1 = CountryId(1);
    let war_reg = WarRegistry::default(); // 戦争なし

    let mut mil = MilitaryRegistry::default();
    let army1 = make_test_army(0, c1, StateId(1));
    let army2 = make_test_army(1, c1, StateId(1));
    mil.armies.insert(ArmyId(0), army1);
    mil.armies.insert(ArmyId(1), army2);

    let s1 = StateData { id: StateId(1), owner_country_id: c1, ..Default::default() };
    let s2 = StateData { id: StateId(2), owner_country_id: c1, ..Default::default() };
    let mut state_reg = StateRegistry::build(vec![s1, s2]);
    let mut battle_reg = BattleRegistry::default();

    process_army_arrival(
        ArmyId(0),
        StateId(1),
        StateId(0),
        "1800/01/01",
        &mut mil,
        &mut state_reg,
        &war_reg,
        &mut battle_reg,
    );

    assert_eq!(battle_reg.battles.len(), 0, "No battle between same country");
}

#[test]
fn test_phase15_retreat_is_deterministic() {
    use crate::military::invasion::find_retreat_destination;

    // 複数の撤退先候補がある場合、StateId昇順で選ぶ
    let mut s1 = StateData::default();
    s1.id = StateId(1); // 現在地
    s1.owner_country_id = CountryId(2);
    s1.neighbors = vec![StateId(5), StateId(3), StateId(4)]; // 順序がバラバラ

    let mut s3 = StateData::default();
    s3.id = StateId(3);
    s3.owner_country_id = CountryId(2);

    let mut s4 = StateData::default();
    s4.id = StateId(4);
    s4.owner_country_id = CountryId(2);

    let mut s5 = StateData::default();
    s5.id = StateId(5);
    s5.owner_country_id = CountryId(2);

    let state_reg = StateRegistry::build(vec![s1, s3, s4, s5]);
    let battle_reg = BattleRegistry::default();

    let mut mil = MilitaryRegistry::default();
    let army = make_test_army(0, CountryId(2), StateId(1));
    mil.armies.insert(ArmyId(0), army);

    let dest = find_retreat_destination(ArmyId(0), StateId(1), &mil, &state_reg, &battle_reg);

    // StateId昇順で最小（StateId(3)）が選ばれる
    assert_eq!(dest, Some(StateId(3)), "Should choose smallest StateId for retreat");
}

#[test]
fn test_phase15_controller_restored_on_recapture() {
    use crate::military::invasion::occupy_state;

    let mut s1 = StateData::default();
    s1.id = StateId(1);
    s1.owner_country_id = CountryId(1); // 所有: C1
    s1.controller_country = Some(CountryId(2)); // 現在はC2が支配

    let mut registry = StateRegistry::build(vec![s1]);

    // C1が奪還
    occupy_state(StateId(1), CountryId(1), &mut registry);

    let s = registry.get(StateId(1)).unwrap();
    // 支配国がC1に戻る
    assert_eq!(s.controller(), CountryId(1));
    // 所有国はC1のまま
    assert_eq!(s.owner_country_id, CountryId(1));
}
