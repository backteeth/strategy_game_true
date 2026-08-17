use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// プロトタイプで扱う資源の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceType {
    Food,
    Wood,
    Iron,
    Coal,
    /// 精製済みマジッククリスタル(魔法系施設が消費する既存資源、P21-009でRawMagicCrystalと分離)
    MagicCrystal,
    IndustrialGoods,
    MilitaryEquipment,
    /// 未精製のマジッククリスタル原石(P21-009: クリスタル採掘施設が産出し、精製施設が消費する)
    RawMagicCrystal,
}

impl ResourceType {
    pub const ALL: [ResourceType; 8] = [
        ResourceType::Food,
        ResourceType::Wood,
        ResourceType::Iron,
        ResourceType::Coal,
        ResourceType::MagicCrystal,
        ResourceType::IndustrialGoods,
        ResourceType::MilitaryEquipment,
        ResourceType::RawMagicCrystal,
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
            ResourceType::RawMagicCrystal => "resource.raw_magic_crystal",
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

/// 指定した資源種別のdiscovered済み鉱床が1件以上含まれるかを判定する(P21-009)。
/// クリスタル採掘施設のような鉱床ゲート付き建物の建設可否判定に使う。
pub fn has_discovered_deposit(deposits: &[StateResourceDeposit], resource: ResourceType) -> bool {
    deposits
        .iter()
        .any(|d| d.discovered && d.resource_type == resource)
}

/// 州がクリスタル専用(discoveredなMagicCrystal鉱床を持ち、それ以外のdiscovered鉱床を
/// 持たない)かどうかを判定する(P21-009-FIX-001)。このような州では通常Mineは対象鉱床
/// (MagicCrystal以外)を持たないため、新規建設を許可しない(既存のMineから
/// MagicCrystal種別鉱床を除外する仕様[production.rs]と対になる判定)。
pub fn is_crystal_only_state(deposits: &[StateResourceDeposit]) -> bool {
    has_discovered_deposit(deposits, ResourceType::MagicCrystal)
        && !deposits
            .iter()
            .any(|d| d.discovered && d.resource_type != ResourceType::MagicCrystal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_discovered_deposit_true_only_for_matching_discovered_type() {
        let deposits = vec![
            StateResourceDeposit {
                resource_type: ResourceType::Iron,
                base_output: 10.0,
                discovered: true,
                development_level: 1,
            },
            StateResourceDeposit {
                resource_type: ResourceType::MagicCrystal,
                base_output: 30.0,
                discovered: false,
                development_level: 1,
            },
        ];
        assert!(!has_discovered_deposit(
            &deposits,
            ResourceType::MagicCrystal
        ));
        assert!(has_discovered_deposit(&deposits, ResourceType::Iron));
        assert!(!has_discovered_deposit(&deposits, ResourceType::Coal));
    }

    #[test]
    fn has_discovered_deposit_true_when_discovered_and_matching() {
        let deposits = vec![StateResourceDeposit {
            resource_type: ResourceType::MagicCrystal,
            base_output: 30.0,
            discovered: true,
            development_level: 1,
        }];
        assert!(has_discovered_deposit(
            &deposits,
            ResourceType::MagicCrystal
        ));
    }

    #[test]
    fn is_crystal_only_state_true_when_only_magic_crystal_deposit_present() {
        let deposits = vec![StateResourceDeposit {
            resource_type: ResourceType::MagicCrystal,
            base_output: 30.0,
            discovered: true,
            development_level: 1,
        }];
        assert!(is_crystal_only_state(&deposits));
    }

    #[test]
    fn is_crystal_only_state_false_when_no_magic_crystal_deposit() {
        let deposits = vec![StateResourceDeposit {
            resource_type: ResourceType::Iron,
            base_output: 10.0,
            discovered: true,
            development_level: 1,
        }];
        assert!(!is_crystal_only_state(&deposits));
    }

    #[test]
    fn is_crystal_only_state_false_when_mixed_with_other_discovered_deposit() {
        let deposits = vec![
            StateResourceDeposit {
                resource_type: ResourceType::MagicCrystal,
                base_output: 30.0,
                discovered: true,
                development_level: 1,
            },
            StateResourceDeposit {
                resource_type: ResourceType::Iron,
                base_output: 10.0,
                discovered: true,
                development_level: 1,
            },
        ];
        assert!(!is_crystal_only_state(&deposits));
    }

    #[test]
    fn is_crystal_only_state_true_when_other_deposit_is_not_discovered() {
        let deposits = vec![
            StateResourceDeposit {
                resource_type: ResourceType::MagicCrystal,
                base_output: 30.0,
                discovered: true,
                development_level: 1,
            },
            StateResourceDeposit {
                resource_type: ResourceType::Iron,
                base_output: 10.0,
                discovered: false,
                development_level: 1,
            },
        ];
        assert!(is_crystal_only_state(&deposits));
    }
}
