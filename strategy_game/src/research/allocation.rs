use crate::building::data::BuildingType;
use crate::economy::resources::ResourceType;
use crate::research::data::{TechnologyEffect, TechnologyField, TechnologyRegistry};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 4分野の研究力配分 (合計 1.0 = 100%)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchAllocation {
    pub science: f32,
    pub magic: f32,
    pub military: f32,
    pub fusion: f32,
}

impl Default for ResearchAllocation {
    fn default() -> Self {
        Self {
            science: 0.40,
            magic: 0.40,
            military: 0.20,
            fusion: 0.0,
        }
    }
}

impl ResearchAllocation {
    pub fn get(&self, field: TechnologyField) -> f32 {
        match field {
            TechnologyField::Science => self.science,
            TechnologyField::Magic => self.magic,
            TechnologyField::Military => self.military,
            TechnologyField::Fusion => self.fusion,
        }
    }

    pub fn set(&mut self, field: TechnologyField, val: f32) {
        let clamped = val.clamp(0.0, 1.0);
        match field {
            TechnologyField::Science => self.science = clamped,
            TechnologyField::Magic => self.magic = clamped,
            TechnologyField::Military => self.military = clamped,
            TechnologyField::Fusion => self.fusion = clamped,
        }
        self.normalize();
    }

    /// 合計が1.0になるように正規化
    pub fn normalize(&mut self) {
        let sum = self.science + self.magic + self.military + self.fusion;
        if sum > 0.0001 {
            self.science /= sum;
            self.magic /= sum;
            self.military /= sum;
            self.fusion /= sum;
        } else {
            self.science = 0.25;
            self.magic = 0.25;
            self.military = 0.25;
            self.fusion = 0.25;
        }
    }
}

/// 進行中の1技術
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InProgressTech {
    pub tech_id: String,
    pub progress: f64,
    pub cost: f64,
}

/// 国家集計済みの技術倍率・解放フラグ
#[derive(Debug, Clone)]
pub struct CountryTechnologyModifiers {
    pub food_output_mult: f64,
    pub wood_output_mult: f64,
    pub iron_output_mult: f64,
    pub coal_output_mult: f64,
    pub magic_crystal_output_mult: f64,
    pub industrial_goods_output_mult: f64,
    pub military_equipment_output_mult: f64,
    pub building_output_mult: f64,
    pub logistics_capacity_mult: f32,
    pub construction_speed_mult: f32,
    pub science_research_mult: f64,
    pub magic_research_mult: f64,
    pub tax_income_mult: f32,
    pub fusion_research_mult: f64,
    pub monthly_unrest_mod: f32,
    pub unlocked_buildings: HashSet<BuildingType>,
    pub revealed_resources: HashSet<ResourceType>,
}

impl Default for CountryTechnologyModifiers {
    fn default() -> Self {
        Self {
            food_output_mult: 1.0,
            wood_output_mult: 1.0,
            iron_output_mult: 1.0,
            coal_output_mult: 1.0,
            magic_crystal_output_mult: 1.0,
            industrial_goods_output_mult: 1.0,
            military_equipment_output_mult: 1.0,
            building_output_mult: 1.0,
            logistics_capacity_mult: 1.0,
            construction_speed_mult: 1.0,
            science_research_mult: 1.0,
            magic_research_mult: 1.0,
            tax_income_mult: 1.0,
            fusion_research_mult: 1.0,
            monthly_unrest_mod: 0.0,
            unlocked_buildings: HashSet::new(),
            revealed_resources: HashSet::new(),
        }
    }
}

/// 国家単位の研究状態
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CountryResearchState {
    pub completed_technologies: HashSet<String>,
    pub in_progress: HashMap<TechnologyField, InProgressTech>,
    pub allocation: ResearchAllocation,
    pub preferred_fields: Vec<TechnologyField>,
}

impl CountryResearchState {
    /// 取得済み技術から効果倍率を再計算する
    pub fn rebuild_modifiers(&self, registry: &TechnologyRegistry) -> CountryTechnologyModifiers {
        let mut mods = CountryTechnologyModifiers::default();

        for tech_id in &self.completed_technologies {
            if let Some(def) = registry.get(tech_id) {
                for effect in &def.effects {
                    match effect {
                        TechnologyEffect::FoodOutputMultiplier(v) => mods.food_output_mult *= v,
                        TechnologyEffect::WoodOutputMultiplier(v) => mods.wood_output_mult *= v,
                        TechnologyEffect::IronOutputMultiplier(v) => mods.iron_output_mult *= v,
                        TechnologyEffect::CoalOutputMultiplier(v) => mods.coal_output_mult *= v,
                        TechnologyEffect::MagicCrystalOutputMultiplier(v) => {
                            mods.magic_crystal_output_mult *= v
                        }
                        TechnologyEffect::IndustrialGoodsOutputMultiplier(v) => {
                            mods.industrial_goods_output_mult *= v
                        }
                        TechnologyEffect::MilitaryEquipmentOutputMultiplier(v) => {
                            mods.military_equipment_output_mult *= v
                        }
                        TechnologyEffect::BuildingOutputMultiplier(v) => {
                            mods.building_output_mult *= v
                        }
                        TechnologyEffect::LogisticsCapacityMultiplier(v) => {
                            mods.logistics_capacity_mult *= v
                        }
                        TechnologyEffect::ConstructionSpeedMultiplier(v) => {
                            mods.construction_speed_mult *= v
                        }
                        TechnologyEffect::ScienceResearchMultiplier(v) => {
                            mods.science_research_mult *= v
                        }
                        TechnologyEffect::MagicResearchMultiplier(v) => {
                            mods.magic_research_mult *= v
                        }
                        TechnologyEffect::TaxIncomeMultiplier(v) => mods.tax_income_mult *= v,
                        TechnologyEffect::FusionResearchMultiplier(v) => {
                            mods.fusion_research_mult *= v
                        }
                        TechnologyEffect::MonthlyUnrestModifier(v) => mods.monthly_unrest_mod += v,
                        TechnologyEffect::UnlockBuilding(b) => {
                            mods.unlocked_buildings.insert(*b);
                        }
                        TechnologyEffect::RevealResourceType(r) => {
                            mods.revealed_resources.insert(*r);
                        }
                    }
                }
            }
        }
        mods
    }
}
