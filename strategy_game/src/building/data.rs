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
    /// マジッククリスタル採掘施設(P21-009)。クリスタル鉱床を持つ州でのみ建設可能。
    CrystalMine,
    /// マジッククリスタル精製施設(P21-009)。RawMagicCrystalを消費しMagicCrystalを生産する。
    CrystalRefinery,
}

impl BuildingType {
    pub const ALL: [BuildingType; 10] = [
        BuildingType::Farm,
        BuildingType::LoggingCamp,
        BuildingType::Mine,
        BuildingType::Factory,
        BuildingType::MilitaryFactory,
        BuildingType::Railway,
        BuildingType::University,
        BuildingType::MagicAcademy,
        BuildingType::CrystalMine,
        BuildingType::CrystalRefinery,
    ];

    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            BuildingType::Farm => "building.farm",
            BuildingType::LoggingCamp => "building.logging_camp",
            BuildingType::Mine => "building.mine",
            BuildingType::Factory => "building.factory",
            BuildingType::MilitaryFactory => "building.military_factory",
            BuildingType::Railway => "building.railway",
            BuildingType::University => "building.university",
            BuildingType::MagicAcademy => "building.magic_academy",
            BuildingType::CrystalMine => "building.crystal_mine",
            BuildingType::CrystalRefinery => "building.crystal_refinery",
        }
    }

    /// クリスタル鉱床(discoveredなMagicCrystal鉱床)が州に存在する場合のみ建設可能な施設か。
    pub fn requires_magic_crystal_deposit(self) -> bool {
        matches!(self, BuildingType::CrystalMine)
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

    /// 建物定義のRONテキストを解析し、定義ごとの値を検証して登録簿を作る(P32-019: 旧`app::loader`の検証を
    /// 純粋関数へ切り出したもの。旧ゲームの読み込みも新経路のセッションも、これを共有する)。
    /// 同じ`building_type`が複数ある場合は後の定義が残る(従来の読み込みと同じ)。
    pub fn from_ron(text: &str) -> Result<BuildingRegistry, String> {
        let defs: Vec<BuildingDefinition> =
            ron::from_str(text).map_err(|e| format!("Failed to parse: {e}"))?;
        let mut definitions = HashMap::new();
        for b in defs {
            if b.construction_cost < 0.0 {
                return Err(format!(
                    "Building '{:?}' has negative construction cost: {}",
                    b.building_type, b.construction_cost
                ));
            }
            if b.required_progress <= 0.0 {
                return Err(format!(
                    "Building '{:?}' has invalid required_progress (<= 0): {}",
                    b.building_type, b.required_progress
                ));
            }
            if b.max_level == 0 {
                return Err(format!("Building '{:?}' has max_level of 0", b.building_type));
            }
            definitions.insert(b.building_type, b);
        }
        Ok(BuildingRegistry { definitions })
    }
}
