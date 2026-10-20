//! P32-007 (M4-A): 人口・雇用・物流・生産施設の既存計算が必要とする、地域データへの読み書きの窓口。
//!
//! 既存の月次経済計算(雇用・物流・生産)は、これまで旧`StateData`だけを直接読み書きしていた。
//! 新しいステート(`MapStateId`)側のデータ(`economy::map_state::MapStateEconomy`)でも
//! **同じ中核計算**を使えるよう、計算が実際に必要とするフィールドだけをこのトレイトへ切り出した。
//! 計算式を新旧で複製せず、`StateData`も`MapStateEconomy`も同じ関数を通る。
//!
//! `economy_key`は、同じ種類の地域の集合の中で処理順と按分の同率判定を決めるための安定した
//! 数値で、旧`StateId`と`MapStateId`の**相互変換や同一視には使わない**(別々の集合の中でだけ意味を持つ)。

use crate::building::data::{BuildingRegistry, BuildingType};
use crate::economy::resources::StateResourceDeposit;
use crate::logistics::update::compute_logistics;
use crate::population::mobilization::apportion_mobilized_manpower_by;
use crate::population::update::compute_employment;
use crate::state::data::{StateData, canonical_total_workforce};
use std::collections::HashMap;

pub trait StateEconomyAccess {
    /// 所属する集合(旧州の集合、または新ステートの集合)の中での安定した順序キー。
    fn economy_key(&self) -> u64;

    fn population(&self) -> u64;
    fn set_population(&mut self, population: u64);
    fn workforce_ratio(&self) -> f32;
    fn buildings(&self) -> &HashMap<BuildingType, u32>;

    fn employed_workforce(&self) -> u64;
    fn unemployed_workforce(&self) -> u64;
    fn set_employment(&mut self, employed: u64, unemployed: u64);

    fn base_logistics_capacity(&self) -> f32;
    fn logistics_ratio(&self) -> f32;
    fn logistics_capacity(&self) -> f32;
    fn set_logistics(&mut self, capacity: f32, usage: f32, ratio: f32);

    /// 生活水準(0.0〜100.0)と不満度(0.0〜100.0)。福祉更新(`economy::finance`)が読み書きする。
    fn living_standard(&self) -> f32;
    fn unrest(&self) -> f32;
    fn set_welfare(&mut self, living_standard: f32, unrest: f32);

    /// 建物の稼働率(生産処理が書き込んだ値。未記録は0.0)。
    fn building_operation_ratio(&self, b_type: BuildingType) -> f32;
    /// 建物ごとの稼働率の記録(生産処理が書き込んだ値)。
    fn operation_ratios(&self) -> &HashMap<BuildingType, f32>;

    /// 生産で使う、この地域で利用可能な鉱床(入力データ)。
    fn resource_deposits(&self) -> &[StateResourceDeposit];

    fn set_building_operation_ratios(&mut self, ratios: HashMap<BuildingType, f32>);

    /// 人口と労働力比率から導く潜在総労働力(`StateData::total_workforce`と同じ正規計算)。
    fn total_workforce(&self) -> u64 {
        canonical_total_workforce(self.population(), self.workforce_ratio())
    }
    /// 動員負担控除後の民間労働力(`employed + unemployed`)。
    fn civilian_workforce(&self) -> u64 {
        self.employed_workforce()
            .saturating_add(self.unemployed_workforce())
    }
    fn building_level(&self, b_type: BuildingType) -> u32 {
        self.buildings().get(&b_type).copied().unwrap_or(0)
    }
}

impl StateEconomyAccess for StateData {
    fn economy_key(&self) -> u64 {
        self.id.0 as u64
    }
    fn population(&self) -> u64 {
        self.population
    }
    fn set_population(&mut self, population: u64) {
        self.population = population;
    }
    fn workforce_ratio(&self) -> f32 {
        self.workforce_ratio
    }
    fn buildings(&self) -> &HashMap<BuildingType, u32> {
        &self.buildings
    }
    fn employed_workforce(&self) -> u64 {
        self.employed_workforce
    }
    fn unemployed_workforce(&self) -> u64 {
        self.unemployed_workforce
    }
    fn set_employment(&mut self, employed: u64, unemployed: u64) {
        self.employed_workforce = employed;
        self.unemployed_workforce = unemployed;
    }
    fn base_logistics_capacity(&self) -> f32 {
        self.base_logistics_capacity
    }
    fn logistics_ratio(&self) -> f32 {
        self.logistics_ratio
    }
    fn set_logistics(&mut self, capacity: f32, usage: f32, ratio: f32) {
        self.logistics_capacity = capacity;
        self.logistics_usage = usage;
        self.logistics_ratio = ratio;
    }
    fn logistics_capacity(&self) -> f32 {
        self.logistics_capacity
    }
    fn living_standard(&self) -> f32 {
        self.living_standard
    }
    fn unrest(&self) -> f32 {
        self.unrest
    }
    fn set_welfare(&mut self, living_standard: f32, unrest: f32) {
        self.living_standard = living_standard;
        self.unrest = unrest;
    }
    fn building_operation_ratio(&self, b_type: BuildingType) -> f32 {
        self.building_operation(b_type)
    }
    fn operation_ratios(&self) -> &HashMap<BuildingType, f32> {
        &self.building_operation_ratios
    }
    fn resource_deposits(&self) -> &[StateResourceDeposit] {
        &self.resource_deposits
    }
    fn set_building_operation_ratios(&mut self, ratios: HashMap<BuildingType, f32>) {
        self.building_operation_ratios = ratios;
    }
}

/// 国家の月次処理のうち、人口・雇用・物流の更新(動員負担の按分 → 雇用 → 物流)。
/// 旧`economy::handle_monthly_economy`と、新ステートの検証経路の両方がこの1つの関数を通る。
///
/// `mobilized_manpower`は国家全体の動員済み人的資源(既存の意味のまま)。所有する地域の
/// 人口比で最大剰余法により按分し、民間労働力から控除する(`population::mobilization`)。
pub fn update_monthly_workforce_and_logistics<S: StateEconomyAccess>(
    states: &mut [&mut S],
    mobilized_manpower: u64,
    building_registry: &BuildingRegistry,
) {
    let pairs: Vec<(u64, u64)> = states
        .iter()
        .map(|s| (s.economy_key(), s.population()))
        .collect();
    let burdens = apportion_mobilized_manpower_by(mobilized_manpower, &pairs, |k| k);
    for s in states.iter_mut() {
        let burden = burdens.get(&s.economy_key()).copied().unwrap_or(0);
        update_employment(&mut **s, building_registry, burden);
        update_logistics(&mut **s, building_registry);
    }
}

/// 地域の雇用者数・失業者数を更新する(`compute_employment`の結果を書き込む)。
pub fn update_employment<S: StateEconomyAccess>(
    state: &mut S,
    building_registry: &BuildingRegistry,
    mobilized_burden: u64,
) {
    let r = compute_employment(
        state.population(),
        state.workforce_ratio(),
        state.buildings(),
        building_registry,
        mobilized_burden,
    );
    state.set_employment(r.employed, r.unemployed);
}

/// 地域の物流容量・使用量・充足率を更新する(`compute_logistics`の結果を書き込む)。
pub fn update_logistics<S: StateEconomyAccess>(
    state: &mut S,
    building_registry: &BuildingRegistry,
) {
    let r = compute_logistics(
        state.base_logistics_capacity(),
        state.buildings(),
        building_registry,
    );
    state.set_logistics(r.capacity, r.usage, r.ratio);
}
