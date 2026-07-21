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

    /// 日本語表示名
    pub fn display_name(self) -> &'static str {
        match self {
            BuildingType::Farm => "農場",
            BuildingType::LoggingCamp => "伐採所",
            BuildingType::Mine => "鉱山",
            BuildingType::Factory => "民需工場",
            BuildingType::MilitaryFactory => "軍需工場",
            BuildingType::Railway => "鉄道",
            BuildingType::University => "大学",
            BuildingType::MagicAcademy => "魔法学院",
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
