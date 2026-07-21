use crate::economy::resources::ResourceType;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// プロトタイプで扱う建物の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BuildingType {
    Farm,
    LoggingCamp,
    Mine,
    Factory,
    MilitaryFactory,
    Railway,
    University,
    MagicAcademy,
}

impl BuildingType {
    pub const ALL: [BuildingType; 8] = [
        BuildingType::Farm,
        BuildingType::LoggingCamp,
        BuildingType::Mine,
        BuildingType::Factory,
        BuildingType::MilitaryFactory,
        BuildingType::Railway,
        BuildingType::University,
        BuildingType::MagicAcademy,
    ];

    /// 表示用英語名
    pub fn display_name(self) -> &'static str {
        match self {
            BuildingType::Farm => "Farm",
            BuildingType::LoggingCamp => "Logging Camp",
            BuildingType::Mine => "Mine",
            BuildingType::Factory => "Factory",
            BuildingType::MilitaryFactory => "Military Factory",
            BuildingType::Railway => "Railway",
            BuildingType::University => "University",
            BuildingType::MagicAcademy => "Magic Academy",
        }
    }
}

/// RONデータから読み込む建物定義
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildingDefinition {
    pub building_type: BuildingType,
    pub name: String,
    pub construction_cost: f64,
    pub required_progress: f64,
    pub required_workforce: f32,
    pub logistics_cost: f32,
    #[serde(default)]
    pub input_resources: HashMap<ResourceType, f64>,
    #[serde(default)]
    pub output_resources: HashMap<ResourceType, f64>,
    pub maintenance_cost: f64,
    pub max_level: u32,
    #[serde(default)]
    pub science_output: f64,
    #[serde(default)]
    pub magic_output: f64,
    #[serde(default)]
    pub railway_capacity_bonus: f32,
}

/// 全建物定義を保持するリソース
#[derive(Resource, Default)]
pub struct BuildingRegistry {
    pub definitions: HashMap<BuildingType, BuildingDefinition>,
}

impl BuildingRegistry {
    pub fn get(&self, b_type: BuildingType) -> Option<&BuildingDefinition> {
        self.definitions.get(&b_type)
    }
}
