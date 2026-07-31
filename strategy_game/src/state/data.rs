use crate::building::data::BuildingType;
use crate::common::{CountryId, StateId};
use crate::economy::resources::StateResourceDeposit;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 労働力割合のデフォルト値
pub const DEFAULT_WORKFORCE_RATIO: f32 = 0.45;

/// 1州分のゲームデータ（RONデシリアライズ対応、表示Entityとは分離）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateData {
    pub id: StateId,
    pub name: String,
    /// 所有国ID
    pub owner_country_id: CountryId,
    /// 人口
    pub population: u64,
    /// 労働力比率 (0.0〜1.0)。旧workforceフィールド
    #[serde(default = "default_workforce_ratio")]
    pub workforce_ratio: f32,
    /// 就業者数
    #[serde(default)]
    pub employed_workforce: u64,
    /// 失業者数
    #[serde(default)]
    pub unemployed_workforce: u64,
    /// 教育水準 (0.0〜1.0)
    pub education: f32,
    /// 生活水準 (0.0〜100.0)
    pub living_standard: f32,
    /// 不満度 (0.0〜100.0)
    pub unrest: f32,

    /// 基礎物流容量
    #[serde(default = "default_base_logistics")]
    pub base_logistics_capacity: f32,
    /// 実物流容量
    #[serde(default = "default_base_logistics")]
    pub logistics_capacity: f32,
    /// 物流使用量
    #[serde(default)]
    pub logistics_usage: f32,
    /// 物流充足率 (0.0〜1.0)
    #[serde(default = "default_ratio")]
    pub logistics_ratio: f32,

    /// 建物レベル
    #[serde(default)]
    pub buildings: HashMap<BuildingType, u32>,
    /// 建物稼働率 (0.0〜1.0)
    #[serde(default)]
    pub building_operation_ratios: HashMap<BuildingType, f32>,

    /// 州内の資源鉱床
    #[serde(default)]
    pub resource_deposits: Vec<StateResourceDeposit>,

    /// ワールド座標での中心位置
    pub world_position: [f32; 2],
    /// 州の矩形サイズ（ワールド座標）
    pub size: [f32; 2],

    /// 隣接州リスト
    #[serde(default)]
    pub neighbors: Vec<StateId>,
    /// 統合度 (0.0〜100.0)
    #[serde(default = "default_integration")]
    pub integration: f32,

    /// 実効支配国（占領国、通常はowner_country_idと同じ）
    #[serde(default)]
    pub controller_country: Option<CountryId>,
    /// 占領進捗 (0.0〜100.0)
    #[serde(default)]
    pub occupation_progress: f32,
    /// 元の所有国（占領中に保持）
    #[serde(default)]
    pub original_owner: Option<CountryId>,
    /// 占領政策
    #[serde(default)]
    pub occupation_policy: Option<OccupationPolicy>,
    /// 関連する戦争ID
    #[serde(default)]
    pub war_id: Option<crate::common::WarId>,
    /// 海域フラグ (海域地域の場合は true)
    #[serde(default)]
    pub is_sea: bool,
}

impl Default for StateData {
    fn default() -> Self {
        Self {
            id: StateId(0),
            name: "Default State".to_string(),
            owner_country_id: CountryId(0),
            population: 0,
            workforce_ratio: DEFAULT_WORKFORCE_RATIO,
            employed_workforce: 0,
            unemployed_workforce: 0,
            education: 0.0,
            living_standard: 10.0,
            unrest: 0.0,
            base_logistics_capacity: 10.0,
            logistics_capacity: 10.0,
            logistics_usage: 0.0,
            logistics_ratio: 1.0,
            buildings: HashMap::new(),
            building_operation_ratios: HashMap::new(),
            resource_deposits: Vec::new(),
            world_position: [0.0, 0.0],
            size: [100.0, 100.0],
            neighbors: Vec::new(),
            controller_country: None,
            integration: 100.0,
            occupation_progress: 0.0,
            original_owner: None,
            occupation_policy: None,
            war_id: None,
            is_sea: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OccupationPolicy {
    #[default]
    MilitaryRule,
    DirectRule,
    LocalAutonomy,
    ResourceExtraction,
}

fn default_integration() -> f32 {
    100.0
}

fn default_workforce_ratio() -> f32 {
    DEFAULT_WORKFORCE_RATIO
}

fn default_base_logistics() -> f32 {
    100.0
}

fn default_ratio() -> f32 {
    1.0
}

impl StateData {
    /// world_position を Vec2 として取得する
    pub fn position(&self) -> Vec2 {
        Vec2::new(self.world_position[0], self.world_position[1])
    }

    /// size を Vec2 として取得する
    pub fn rect_size(&self) -> Vec2 {
        Vec2::new(self.size[0], self.size[1])
    }

    /// 労働力人口を取得 (workforce = population * workforce_ratio)
    pub fn total_workforce(&self) -> u64 {
        ((self.population as f64) * (self.workforce_ratio as f64)) as u64
    }

    /// 建物レベルを取得
    pub fn building_level(&self, b_type: BuildingType) -> u32 {
        *self.buildings.get(&b_type).unwrap_or(&0)
    }

    /// 建物稼働率を取得
    pub fn building_operation(&self, b_type: BuildingType) -> f32 {
        *self.building_operation_ratios.get(&b_type).unwrap_or(&0.0)
    }

    /// 生活水準・不満度等のクランプと整合性調整
    pub fn sanitize_values(&mut self) {
        self.education = self.education.clamp(0.0, 1.0);
        // RONの旧データ(0.0~1.0)だった場合、100倍スケールへ統一
        if self.living_standard <= 1.0 {
            self.living_standard *= 100.0;
        }
        if self.unrest <= 1.0 && self.unrest > 0.0 {
            self.unrest *= 100.0;
        }
        self.living_standard = self.living_standard.clamp(0.0, 100.0);
        self.unrest = self.unrest.clamp(0.0, 100.0);
        self.logistics_capacity = self.logistics_capacity.max(0.0);
        self.logistics_usage = self.logistics_usage.max(0.0);
        self.logistics_ratio = self.logistics_ratio.clamp(0.0, 1.0);
        if self.base_logistics_capacity == 0.0 {
            self.base_logistics_capacity = 100.0;
        }
    }

    /// 実効支配国を取得する
    pub fn controller(&self) -> CountryId {
        self.controller_country.unwrap_or(self.owner_country_id)
    }
}

/// 全州データを保持するリソース
#[derive(Resource, Default)]
pub struct StateRegistry {
    pub states: Vec<StateData>,
    /// StateId.0 → Vec インデックスのルックアップテーブル
    index_map: std::collections::HashMap<usize, usize>,
}

impl StateRegistry {
    /// データを挿入してインデックスを再構築する
    pub fn build(mut states: Vec<StateData>) -> Self {
        for s in states.iter_mut() {
            s.sanitize_values();
        }
        let index_map = states
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.0, i))
            .collect();
        Self { states, index_map }
    }

    /// ID で州を O(1) 検索する
    pub fn get(&self, id: StateId) -> Option<&StateData> {
        self.index_map.get(&id.0).and_then(|&i| self.states.get(i))
    }

    /// ID で州を O(1) 可変検索する
    pub fn get_mut(&mut self, id: StateId) -> Option<&mut StateData> {
        if let Some(&i) = self.index_map.get(&id.0) {
            self.states.get_mut(i)
        } else {
            None
        }
    }

    /// 該当国家が法的に所有しているすべての州を取得する
    pub fn get_owned_states(&self, country_id: CountryId) -> Vec<&StateData> {
        self.states
            .iter()
            .filter(|s| s.owner_country_id == country_id)
            .collect()
    }

    /// 該当国家が実効支配（占領含む）しているすべての州を取得する
    pub fn get_controlled_states(&self, country_id: CountryId) -> Vec<&StateData> {
        self.states
            .iter()
            .filter(|s| s.controller() == country_id)
            .collect()
    }

    /// 法的領土所有権の変更
    pub fn transfer_region_ownership(
        &mut self,
        state_id: StateId,
        new_owner: CountryId,
    ) -> Result<(), &'static str> {
        let state = self.get_mut(state_id).ok_or("State does not exist")?;

        if state.is_sea {
            return Err("Cannot transfer sea region ownership");
        }

        // 法的所有権と支配権を新所有国へ設定
        state.owner_country_id = new_owner;
        state.controller_country = Some(new_owner);
        state.occupation_progress = 0.0;
        state.original_owner = None;
        state.war_id = None;

        Ok(())
    }
}
