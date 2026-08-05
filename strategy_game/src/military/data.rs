use crate::common::{ArmyId, BattleId, CountryId, DivisionId, StateId};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DivisionType {
    Infantry,
    Artillery,
    Mage,
}

impl DivisionType {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            DivisionType::Infantry => "division_type.infantry",
            DivisionType::Artillery => "division_type.artillery",
            DivisionType::Mage => "division_type.mage",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DivisionSize {
    Light,
    Standard,
    Heavy,
}

impl DivisionSize {
    /// 表示用の翻訳キー(P20-009)。現状はUIから未参照だが、将来の表示追加に備えて維持する。
    pub fn display_name(self) -> &'static str {
        match self {
            DivisionSize::Light => "division_size.light",
            DivisionSize::Standard => "division_size.standard",
            DivisionSize::Heavy => "division_size.heavy",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivisionDefinition {
    pub id: DivisionId,
    pub name: String,
    pub division_type: DivisionType,
    pub size: DivisionSize,
    pub required_manpower: u64,
    pub required_equipment: f64,
    pub recruitment_days: u32,
    pub movement_speed: f32,
    pub attack: f32,
    pub defense: f32,
    pub breakthrough: f32,
    pub organization: f32,
    pub morale: f32,
    pub supply_usage: f32,
    pub maintenance_cost: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ArmyStatus {
    #[default]
    Idle,
    Moving,
    Fighting,
    Retreating,
    Occupying,
    Disbanding,
    Destroyed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmyUnit {
    pub id: ArmyId,
    pub owner: CountryId,
    pub division_type: DivisionType,
    pub size: DivisionSize,
    pub current_state: StateId,
    #[serde(default)]
    pub destination: Option<StateId>,
    #[serde(default)]
    pub current_path: Vec<StateId>,
    pub target_state: Option<StateId>,
    pub manpower: u64,
    pub max_manpower: u64,
    pub equipment: f64,
    pub max_equipment: f64,
    pub organization: f32,
    pub max_organization: f32,
    pub morale: f32,
    pub max_morale: f32,
    pub experience: f32,
    pub supply_ratio: f32,
    pub movement_progress: f32,
    pub status: ArmyStatus,
    pub def_id: DivisionId,
    /// 攻撃力（整数、決定的な戦闘計算に使用）
    #[serde(default = "default_attack_power")]
    pub attack_power: i32,
    /// 防御力（整数）
    #[serde(default = "default_defense_power")]
    pub defense_power: i32,
    /// 参加中の戦闘ID
    #[serde(default)]
    pub combat_id: Option<BattleId>,
}

#[derive(Resource, Default, Debug)]
pub struct MilitaryRegistry {
    pub definitions: HashMap<DivisionId, DivisionDefinition>,
    pub armies: HashMap<ArmyId, ArmyUnit>,
    next_army_id: usize,
}

impl MilitaryRegistry {
    pub fn next_army_id(&self) -> usize {
        self.next_army_id
    }

    pub fn add_army(&mut self, mut army: ArmyUnit) -> ArmyId {
        army.id = ArmyId(self.next_army_id);
        self.next_army_id += 1;
        self.armies.insert(army.id, army.clone());
        army.id
    }

    pub fn remove_army(&mut self, id: ArmyId) {
        self.armies.remove(&id);
    }

    pub fn get_armies_in_state(&self, state: StateId) -> Vec<&ArmyUnit> {
        self.armies
            .values()
            .filter(|a| a.current_state == state)
            .collect()
    }
}

fn default_attack_power() -> i32 {
    10
}

fn default_defense_power() -> i32 {
    10
}
