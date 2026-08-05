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
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            WorldStage::PreIndustrial => "world_stage.pre_industrial",
            WorldStage::IndustrialRevolution => "world_stage.industrial_revolution",
            WorldStage::ElectricalAge => "world_stage.electrical_age",
            WorldStage::TotalWarAge => "world_stage.total_war_age",
            WorldStage::MagitechAge => "world_stage.magitech_age",
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

    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            TechnologyField::Science => "technology_field.science",
            TechnologyField::Magic => "technology_field.magic",
            TechnologyField::Military => "technology_field.military",
            TechnologyField::Fusion => "technology_field.fusion",
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
