#![cfg(test)]
#![allow(clippy::field_reassign_with_default)]

use crate::common::{CountryId, DivisionId, StateId};
use crate::country::{CountryData, CountryRegistry};
use crate::military::data::{DivisionDefinition, DivisionSize, DivisionType, MilitaryRegistry};
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
    use crate::common::ArmyId;
    use crate::military::data::{ArmyStatus, ArmyUnit};
    use crate::military::movement::process_movement;
    use crate::state::data::StateRegistry;

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
    };
    registry.add_army(army);

    let mut s1 = crate::state::data::StateData::default();
    s1.id = StateId(1);
    s1.owner_country_id = CountryId(1);
    let mut s2 = crate::state::data::StateData::default();
    s2.id = StateId(2);
    s2.owner_country_id = CountryId(1);
    let state_registry = StateRegistry::build(vec![s1, s2]);

    // def_speed = 4.0, base_days = 5.0, supply_mod = 1.0
    // movement_days = 5.0 / 4.0 = 1.25
    // daily_progress = 1.0 / 1.25 = 0.8

    process_movement(&mut registry, &state_registry);

    let army_ref = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(army_ref.status, ArmyStatus::Moving);
    assert_eq!(army_ref.current_state, StateId(1));
    assert_eq!(army_ref.target_state, Some(StateId(2)));
    assert!(army_ref.movement_progress > 0.79 && army_ref.movement_progress < 0.81);

    // Next day
    process_movement(&mut registry, &state_registry);

    let army_ref2 = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(army_ref2.status, ArmyStatus::Idle);
    assert_eq!(army_ref2.current_state, StateId(2));
    assert_eq!(army_ref2.target_state, None);
    assert_eq!(army_ref2.movement_progress, 0.0);
}

#[test]
fn test_supply_processing() {
    use crate::common::ArmyId;
    use crate::military::data::{ArmyStatus, ArmyUnit};
    use crate::military::supply::process_supply;
    use crate::state::data::StateRegistry;

    let mut registry = setup_registry();
    let army1 = ArmyUnit {
        id: ArmyId(0),
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
    use crate::common::ArmyId;
    use crate::military::combat::calculate_combat_strength;
    use crate::military::data::{ArmyStatus, ArmyUnit};

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
    use crate::common::ArmyId;
    use crate::military::data::{ArmyStatus, ArmyUnit};
    use crate::military::movement::process_movement;
    use crate::state::data::{StateData, StateRegistry};

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
    let state_registry = StateRegistry::build(vec![s1, s2, s3]);

    // 一時停止中/時間進行イベントなし（process_movementを呼ばない）時は進行度が変化しないことを検証
    let army_before = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(army_before.movement_progress, 0.0);

    // 日数が経過（process_movement実行）
    // def_speed = 4.0, base_days = 5.0 => daily_progress = 0.8 / day
    // Day 1: progress -> 0.8
    process_movement(&mut registry, &state_registry);
    let a1 = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(a1.current_state, StateId(1));
    assert_eq!(a1.target_state, Some(StateId(2)));

    // Day 2: reaches State 2, target_state set to State 3
    process_movement(&mut registry, &state_registry);
    let a2 = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(a2.current_state, StateId(2));
    assert_eq!(a2.target_state, Some(StateId(3)));

    // Day 3: progress -> 0.8 towards State 3
    process_movement(&mut registry, &state_registry);

    // Day 4: reaches State 3 (destination), status -> Idle
    process_movement(&mut registry, &state_registry);
    let a_final = registry.armies.get(&ArmyId(0)).unwrap();
    assert_eq!(a_final.current_state, StateId(3));
    assert_eq!(a_final.status, ArmyStatus::Idle);
    assert_eq!(a_final.destination, None);
    assert_eq!(a_final.target_state, None);
}
