#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use strategy_game::country::GovernmentType;
use strategy_game::politics::interest_groups::{
    CountryPoliticsData, InterestGroupState, InterestGroupType,
};
use strategy_game::politics::reform::{
    PoliticalReform, apply_reform_completion, update_reform_progress_step,
};
use strategy_game::politics::values::{CountryValues, ValueAxis};
use strategy_game::research::allocation::{CountryResearchState, ResearchAllocation};
use strategy_game::research::data::{
    TechnologyDefinition, TechnologyEffect, TechnologyField, TechnologyRegistry, WorldStage,
};
use strategy_game::research::progress::calculate_field_monthly_points;
use strategy_game::research::world_stage::WorldCivilizationState;

#[test]
fn test_research_allocation_normalization() {
    let mut alloc = ResearchAllocation {
        science: 0.5,
        magic: 0.5,
        military: 0.5,
        fusion: 0.5,
    };
    alloc.normalize();
    let sum = alloc.science + alloc.magic + alloc.military + alloc.fusion;
    assert!((sum - 1.0).abs() < 1e-5);
    assert!((alloc.science - 0.25).abs() < 1e-5);
}

#[test]
fn test_calculate_field_monthly_points() {
    let alloc = ResearchAllocation::default(); // sci 0.4, mag 0.4, mil 0.2, fus 0.0
    let (total_cap, points) = calculate_field_monthly_points(20.0, 20.0, 5.0, &alloc, 0.0, 1.0);

    assert!((total_cap - 45.0).abs() < 1e-5);
    assert!((points[0] - 18.0).abs() < 1e-5);
    assert!((points[1] - 18.0).abs() < 1e-5);
}

#[test]
fn test_fusion_efficiency_imbalance_penalty() {
    let alloc = ResearchAllocation {
        science: 0.0,
        magic: 0.0,
        military: 0.0,
        fusion: 1.0,
    };
    let (_, pts_balanced) = calculate_field_monthly_points(10.0, 10.0, 0.0, &alloc, 0.0, 1.0);
    let (_, pts_imbalanced) = calculate_field_monthly_points(1.0, 19.0, 0.0, &alloc, 0.0, 1.0);

    assert!(pts_balanced[3] > pts_imbalanced[3] * 5.0);
}

#[test]
fn test_rebuild_technology_modifiers() {
    let mut reg = TechnologyRegistry::default();
    reg.definitions.insert(
        "Agriculture1".to_string(),
        TechnologyDefinition {
            id: "Agriculture1".to_string(),
            name: "Agri 1".to_string(),
            description: "".to_string(),
            field: TechnologyField::Science,
            cost: 100.0,
            prerequisites: vec![],
            minimum_world_stage: WorldStage::PreIndustrial,
            effects: vec![TechnologyEffect::FoodOutputMultiplier(1.2)],
            is_world_milestone: false,
            milestone_target_stage: None,
            ui_order: 1,
        },
    );

    let mut state = CountryResearchState::default();
    state
        .completed_technologies
        .insert("Agriculture1".to_string());

    let mods = state.rebuild_modifiers(&reg);
    assert!((mods.food_output_mult - 1.2).abs() < 1e-5);
    assert!((mods.industrial_goods_output_mult - 1.0).abs() < 1e-5);
}

#[test]
fn test_country_values_clamping() {
    let mut values = CountryValues {
        science_magic: 150.0,
        individual_state: -200.0,
        secular_religious: 50.0,
    };
    values.clamp_all();
    assert_eq!(values.science_magic, 100.0);
    assert_eq!(values.individual_state, -100.0);
    assert_eq!(values.secular_religious, 50.0);
}

#[test]
fn test_interest_group_influence_normalization() {
    let mut pol = CountryPoliticsData::new_default();
    pol.interest_groups
        .get_mut(&InterestGroupType::Nobility)
        .unwrap()
        .influence = 50.0;
    pol.interest_groups
        .get_mut(&InterestGroupType::Clergy)
        .unwrap()
        .influence = 50.0;
    pol.interest_groups
        .get_mut(&InterestGroupType::Military)
        .unwrap()
        .influence = 100.0;

    pol.normalize_influence();

    let sum: f32 = pol.interest_groups.values().map(|ig| ig.influence).sum();
    assert!((sum - 100.0).abs() < 1e-4);
}

#[test]
fn test_clergy_resistance_to_secular_reform() {
    let mut reform = PoliticalReform::new(ValueAxis::SecularReligious, 50.0, -10.0);
    let mut igs = HashMap::new();

    let mut clergy_ig = InterestGroupState::default();
    clergy_ig.influence = 50.0;
    igs.insert(InterestGroupType::Clergy, clergy_ig);

    update_reform_progress_step(&mut reform, GovernmentType::Theocracy, &mut igs);

    assert!(reform.clergy_resistance > 0.0);
    assert!(reform.monthly_progress < 10.0);
}

#[test]
fn test_political_reform_completion() {
    let reform = PoliticalReform {
        axis: ValueAxis::ScienceMagic,
        start_value: 0.0,
        target_value: -10.0,
        progress: 100.0,
        required_progress: 100.0,
        monthly_progress: 10.0,
        support_score: 50.0,
        opposition_score: 10.0,
        clergy_resistance: 0.0,
    };
    let mut values = CountryValues::default();
    apply_reform_completion(&reform, &mut values);

    assert_eq!(values.science_magic, -10.0);
}

#[test]
fn test_world_civilization_stage_progression() {
    let state = WorldCivilizationState::default();
    assert_eq!(state.current_stage, WorldStage::PreIndustrial);
    assert_eq!(
        state.get_next_stage(),
        Some(WorldStage::IndustrialRevolution)
    );
}
