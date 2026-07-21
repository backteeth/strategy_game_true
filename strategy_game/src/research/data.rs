use crate::building::data::BuildingType;
use crate::economy::resources::ResourceType;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 世界文明段階の5段階
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub enum WorldStage {
    #[default]
    PreIndustrial,
    IndustrialRevolution,
    ElectricalAge,
    TotalWarAge,
    MagitechAge,
}

impl WorldStage {
    pub fn display_name(self) -> &'static str {
        match self {
            WorldStage::PreIndustrial => "Pre-Industrial Age",
            WorldStage::IndustrialRevolution => "Industrial Revolution Age",
            WorldStage::ElectricalAge => "Electrical Age",
            WorldStage::TotalWarAge => "Total War Age",
            WorldStage::MagitechAge => "Magitech Age",
        }
    }
}

/// 研究分野
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TechnologyField {
    #[default]
    Science,
    Magic,
    Military,
    Fusion,
}

impl TechnologyField {
    pub const ALL: [TechnologyField; 4] = [
        TechnologyField::Science,
        TechnologyField::Magic,
        TechnologyField::Military,
        TechnologyField::Fusion,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            TechnologyField::Science => "Science",
            TechnologyField::Magic => "Magic",
            TechnologyField::Military => "Military",
            TechnologyField::Fusion => "Fusion",
        }
    }
}

/// 技術効果の種類
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TechnologyEffect {
    FoodOutputMultiplier(f64),
    WoodOutputMultiplier(f64),
    IronOutputMultiplier(f64),
    CoalOutputMultiplier(f64),
    MagicCrystalOutputMultiplier(f64),
    IndustrialGoodsOutputMultiplier(f64),
    MilitaryEquipmentOutputMultiplier(f64),
    BuildingOutputMultiplier(f64),
    LogisticsCapacityMultiplier(f32),
    ConstructionSpeedMultiplier(f32),
    ScienceResearchMultiplier(f64),
    MagicResearchMultiplier(f64),
    TaxIncomeMultiplier(f32),
    UnlockBuilding(BuildingType),
    RevealResourceType(ResourceType),
    FusionResearchMultiplier(f64),
    MonthlyUnrestModifier(f32),
}

/// RONシリアライズ用技術定義
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechnologyDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub field: TechnologyField,
    pub cost: f64,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    #[serde(default)]
    pub minimum_world_stage: WorldStage,
    #[serde(default)]
    pub effects: Vec<TechnologyEffect>,
    #[serde(default)]
    pub is_world_milestone: bool,
    #[serde(default)]
    pub milestone_target_stage: Option<WorldStage>,
    #[serde(default)]
    pub ui_order: u32,
}

/// 全技術定義を保持するリソース
#[derive(Resource, Default)]
pub struct TechnologyRegistry {
    pub definitions: HashMap<String, TechnologyDefinition>,
    pub sorted_ids: Vec<String>,
}

impl TechnologyRegistry {
    pub fn get(&self, id: &str) -> Option<&TechnologyDefinition> {
        self.definitions.get(id)
    }
}
