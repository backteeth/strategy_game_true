use crate::research::data::WorldStage;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// RONシリアライズ用世界文明段階進行条件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldStageDefinition {
    pub stage: WorldStage,
    pub display_name: String,
    pub description: String,
    pub required_previous_stage: Option<WorldStage>,
    pub milestone_technologies: Vec<String>,
    pub required_country_count: usize,
}

/// ゲーム全体の世界文明段階リソース
#[derive(Resource, Debug, Clone)]
pub struct WorldCivilizationState {
    pub current_stage: WorldStage,
    pub stage_definitions: HashMap<WorldStage, WorldStageDefinition>,
    pub milestone_countries: HashMap<WorldStage, HashSet<usize>>,
    pub last_advanced_date: String,
}

impl Default for WorldCivilizationState {
    fn default() -> Self {
        Self {
            current_stage: WorldStage::PreIndustrial,
            stage_definitions: HashMap::new(),
            milestone_countries: HashMap::new(),
            last_advanced_date: "1800/01/01".to_string(),
        }
    }
}

impl WorldCivilizationState {
    pub fn get_next_stage(&self) -> Option<WorldStage> {
        match self.current_stage {
            WorldStage::PreIndustrial => Some(WorldStage::IndustrialRevolution),
            WorldStage::IndustrialRevolution => Some(WorldStage::ElectricalAge),
            WorldStage::ElectricalAge => Some(WorldStage::TotalWarAge),
            WorldStage::TotalWarAge => Some(WorldStage::MagitechAge),
            WorldStage::MagitechAge => None,
        }
    }
}
