use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// プロトタイプで扱う資源の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceType {
    Food,
    Wood,
    Iron,
    Coal,
    MagicCrystal,
    IndustrialGoods,
    MilitaryEquipment,
}

impl ResourceType {
    pub const ALL: [ResourceType; 7] = [
        ResourceType::Food,
        ResourceType::Wood,
        ResourceType::Iron,
        ResourceType::Coal,
        ResourceType::MagicCrystal,
        ResourceType::IndustrialGoods,
        ResourceType::MilitaryEquipment,
    ];

    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            ResourceType::Food => "resource.food",
            ResourceType::Wood => "resource.wood",
            ResourceType::Iron => "resource.iron",
            ResourceType::Coal => "resource.coal",
            ResourceType::MagicCrystal => "resource.magic_crystal",
            ResourceType::IndustrialGoods => "resource.industrial_goods",
            ResourceType::MilitaryEquipment => "resource.military_equipment",
        }
    }
}

/// 国家単位の資源備蓄
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CountryStockpile {
    pub amounts: HashMap<ResourceType, f64>,
}

impl CountryStockpile {
    pub fn new() -> Self {
        let mut amounts = HashMap::new();
        for res in ResourceType::ALL {
            amounts.insert(res, 0.0);
        }
        Self { amounts }
    }

    pub fn get(&self, resource: ResourceType) -> f64 {
        *self.amounts.get(&resource).unwrap_or(&0.0)
    }

    pub fn set(&mut self, resource: ResourceType, amount: f64) {
        self.amounts.insert(resource, amount.max(0.0));
    }

    pub fn add(&mut self, resource: ResourceType, amount: f64) {
        let current = self.get(resource);
        self.amounts.insert(resource, (current + amount).max(0.0));
    }

    pub fn consume(&mut self, resource: ResourceType, amount: f64) -> bool {
        let current = self.get(resource);
        if current >= amount {
            self.amounts.insert(resource, current - amount);
            true
        } else {
            false
        }
    }
}

/// 州の資源鉱床
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateResourceDeposit {
    pub resource_type: ResourceType,
    pub base_output: f64,
    pub discovered: bool,
    pub development_level: u32,
}

impl Default for StateResourceDeposit {
    fn default() -> Self {
        Self {
            resource_type: ResourceType::Iron,
            base_output: 10.0,
            discovered: true,
            development_level: 1,
        }
    }
}
