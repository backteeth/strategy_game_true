use crate::common::{CountryId, StateId};
/// 州データ定義モジュール
/// StateData とデータ専用の州レジストリを定義する
/// 表示用 Entity とは分離して管理する
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// 1州分のゲームデータ（RONデシリアライズ対応、表示Entityとは分離）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateData {
    pub id: StateId,
    pub name: String,
    /// 所有国ID
    pub owner_country_id: CountryId,
    /// 人口（州単位で管理）
    pub population: u64,
    /// 労働力（就労可能人口の割合 0.0〜1.0）
    pub workforce: f32,
    /// 教育水準（0.0〜1.0）
    pub education: f32,
    /// 生活水準（0.0〜1.0）
    pub living_standard: f32,
    /// 不満度（0.0〜1.0、高いほど反乱リスク大）
    pub unrest: f32,
    /// ワールド座標での中心位置
    pub world_position: [f32; 2],
    /// 州の矩形サイズ（ワールド座標）
    pub size: [f32; 2],
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
}

/// 全州データを保持するリソース
/// StateId → インデックスの直接参照で O(1) 検索が可能
#[derive(Resource, Default)]
pub struct StateRegistry {
    pub states: Vec<StateData>,
    /// StateId.0 → Vec インデックスのルックアップテーブル
    /// 将来 1000 州以上に拡張しても検索が O(1) になる
    index_map: std::collections::HashMap<usize, usize>,
}

impl StateRegistry {
    /// データを挿入してインデックスを再構築する
    pub fn build(states: Vec<StateData>) -> Self {
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
}
