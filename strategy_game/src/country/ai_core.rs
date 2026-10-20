//! P32-010 (M4-C): 経済AI(平時の建設判断)の中核。旧`StateId`/`CountryData`と、新ステート
//! (`MapStateId`)の検証用国家データの**両方が同じ関数を通る**ように、`country_ai.rs`に
//! あった判断ロジックを、国家(`AiCountry`)と地域(`AiRegions`)の2つのトレイトの上へ移した。
//! 判断の優先順位・予備資金・候補の選び方・同点の扱いは従来と同一(式・順序は変更していない)。
//!
//! 乱数は使わない。候補の列挙は呼び出し側が渡す昇順の`owned`リスト、`ResourceType::ALL`の
//! 宣言順、`BuildingType`の固定順だけで決まり、入力の列挙順やHashMapの反復順に依存しない。
//!
//! 軍事に関わる優先度(戦時MilitaryFactory建設=2.5、Iron供給のMine=2.8、軍需基盤の再建=2.9)は
//! `ConstructionHooks`の差し込み口として旧経路だけが実装する。新経路は何も差し込まない
//! (比較対象外。未対応を別の施設で黙って置き換えない)。

use crate::building::data::{BuildingRegistry, BuildingType};
use crate::country::country_ai::{CountryAiDecisionReason, CountryAiState};
use crate::economy::resources::{ResourceType, StateResourceDeposit, is_crystal_only_state};
use std::collections::{HashMap, HashSet};

/// 優先度3(任意的な平時開発)が、建設費支払い後に国家の運転資金(直近の月次経常支出の何か月分)を
/// 侵食しないよう要求する予備月数(P27-002-A。正規定数はここ1か所だけ)。
pub const PEACETIME_CONSTRUCTION_RESERVE_MONTHS: f64 = 3.0;

/// 注文を受け付けられない理由(候補評価・注文受付の前提)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderBlock {
    /// 完成後の月次経済が必要とする資源入力(プロヴィンス別の資源定義)が未設定・不整合。
    ResourcesNotConfigured,
    /// その他(説明付き)。
    Other(String),
}

/// 国家側のデータ(国庫・備蓄・建設キュー)。地域のデータとは分離する。
pub trait AiCountry {
    type Key: Copy + Eq;
    fn treasury(&self) -> f64;
    fn set_treasury(&mut self, value: f64);
    fn monthly_expenses(&self) -> f64;
    fn stock(&self, resource: ResourceType) -> f64;
    fn queue_len(&self) -> usize;
    fn queue_has(&self, region: Self::Key, building: BuildingType) -> bool;
    fn queue_has_building(&self, building: BuildingType) -> bool;
    fn push_order(
        &mut self,
        region: Self::Key,
        building: BuildingType,
        target_level: u32,
        required_progress: f64,
        paid_cost: f64,
    );
}

/// 地域側のデータ(施設・鉱床・物流)の読み取り窓口。
pub trait AiRegions<K: Copy> {
    fn exists(&self, region: K) -> bool;
    fn building_level(&self, region: K, building: BuildingType) -> u32;
    fn buildings(&self, region: K) -> Option<&HashMap<BuildingType, u32>>;
    fn deposits(&self, region: K) -> &[StateResourceDeposit];
    fn logistics_ratio(&self, region: K) -> f32;
    /// 注文の前提条件。`Err`の候補は採用せず、他の候補の評価を続ける。既定は常に受理。
    fn order_precondition(&self, _region: K, _building: BuildingType) -> Result<(), OrderBlock> {
        Ok(())
    }
}

/// 建設を試みた結果。
pub enum ConstructOutcome {
    /// 建設キューへ1件追加し国庫を減算した。
    Built,
    /// 建設可能な候補が存在しない(資金は足りているが、対象が見つからない)。
    NoCandidate,
    /// 国庫が建設費に満たない。
    InsufficientFunds,
}

/// `target_building`の建設候補地域を1件探す(`owned`の昇順で最初に条件を満たす地域)。
/// Mineはクリスタル専用地域を除外し、実際に採掘可能な鉱床(discoveredかつ非MagicCrystal)を
/// 要求する(P21-009-FIX-001、P26-004-PRE-002)。注文の前提条件(`order_precondition`)を満たさない
/// 候補は飛ばして次を評価する。
pub fn find_candidate_region<C: AiCountry, R: AiRegions<C::Key>>(
    country: &C,
    regions: &R,
    owned: &[C::Key],
    target_building: BuildingType,
    max_level: u32,
) -> Option<C::Key> {
    owned.iter().copied().find(|&sid| {
        if country.queue_has(sid, target_building) {
            return false;
        }
        if !regions.exists(sid) {
            return false;
        }
        if regions.order_precondition(sid, target_building).is_err() {
            return false;
        }
        if target_building == BuildingType::Mine {
            if is_crystal_only_state(regions.deposits(sid)) {
                return false;
            }
            let has_usable_deposit = regions
                .deposits(sid)
                .iter()
                .any(|d| d.discovered && d.resource_type != ResourceType::MagicCrystal);
            if !has_usable_deposit {
                return false;
            }
        }
        regions.building_level(sid, target_building) < max_level
    })
}

/// `target_building`の建設を1件だけ試みる。資金不足・候補なしのいずれの場合も国庫・建設キューを
/// 一切変更しない(原子性)。
pub fn try_construct<C: AiCountry, R: AiRegions<C::Key>>(
    country: &mut C,
    regions: &R,
    building_registry: &BuildingRegistry,
    owned: &[C::Key],
    target_building: BuildingType,
) -> ConstructOutcome {
    let Some(def) = building_registry.get(target_building) else {
        return ConstructOutcome::NoCandidate;
    };
    if country.treasury() < def.construction_cost {
        return ConstructOutcome::InsufficientFunds;
    }
    match find_candidate_region(country, regions, owned, target_building, def.max_level) {
        Some(region) => {
            let current_level = regions.building_level(region, target_building);
            country.set_treasury(country.treasury() - def.construction_cost);
            country.push_order(
                region,
                target_building,
                current_level + 1,
                def.required_progress,
                def.construction_cost,
            );
            ConstructOutcome::Built
        }
        None => ConstructOutcome::NoCandidate,
    }
}

/// 優先度3専用のラッパー: 建設費に加えて月次経常支出の`PEACETIME_CONSTRUCTION_RESERVE_MONTHS`か月分を
/// 国庫が賄えることを要求する(数値が非有限・負なら安全側=資金不足)。予備判定に落ちたら
/// `try_construct`自体を呼ばない(候補探索・国庫・キューの変更なし)。
pub fn try_construct_peacetime<C: AiCountry, R: AiRegions<C::Key>>(
    country: &mut C,
    regions: &R,
    building_registry: &BuildingRegistry,
    owned: &[C::Key],
    target_building: BuildingType,
) -> ConstructOutcome {
    if let Some(def) = building_registry.get(target_building) {
        let treasury = country.treasury();
        let construction_cost = def.construction_cost;
        let monthly_expenses = country.monthly_expenses();

        let values_valid = treasury.is_finite()
            && construction_cost.is_finite()
            && construction_cost >= 0.0
            && monthly_expenses.is_finite()
            && monthly_expenses >= 0.0;
        if !values_valid {
            return ConstructOutcome::InsufficientFunds;
        }
        let reserve = monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
        let required_treasury = construction_cost + reserve;
        if !required_treasury.is_finite() || treasury < required_treasury {
            return ConstructOutcome::InsufficientFunds;
        }
    }
    try_construct(country, regions, building_registry, owned, target_building)
}

/// 保有する生産建物(level>0)が入力資源として要求する`ResourceType`集合。
fn resource_types_required_by_owned_buildings<K: Copy, R: AiRegions<K>>(
    regions: &R,
    building_registry: &BuildingRegistry,
    owned: &[K],
) -> HashSet<ResourceType> {
    let mut needed = HashSet::new();
    for &sid in owned {
        let Some(buildings) = regions.buildings(sid) else {
            continue;
        };
        for (&b_type, &level) in buildings.iter() {
            if level == 0 {
                continue;
            }
            if let Some(def) = building_registry.get(b_type) {
                needed.extend(def.input_resources.keys().copied());
            }
        }
    }
    needed
}

/// 領内のいずれかの地域で、指定`resource`のdiscovered鉱床とMineレベル>0が同居しているか。
fn has_active_mine_for_resource<K: Copy, R: AiRegions<K>>(
    regions: &R,
    owned: &[K],
    resource: ResourceType,
) -> bool {
    owned.iter().any(|&sid| {
        regions.exists(sid)
            && regions.building_level(sid, BuildingType::Mine) > 0
            && regions
                .deposits(sid)
                .iter()
                .any(|d| d.discovered && d.resource_type == resource)
    })
}

/// 事前開発対象の未活用鉱床地域を、指定`resource`について1件探す(`owned`の昇順で最初のもの)。
fn find_undeveloped_deposit_region<C: AiCountry, R: AiRegions<C::Key>>(
    country: &C,
    regions: &R,
    owned: &[C::Key],
    mine_max_level: u32,
    resource: ResourceType,
) -> Option<C::Key> {
    owned.iter().copied().find(|&sid| {
        if country.queue_has(sid, BuildingType::Mine) {
            return false;
        }
        if !regions.exists(sid) {
            return false;
        }
        if regions.order_precondition(sid, BuildingType::Mine).is_err() {
            return false;
        }
        if is_crystal_only_state(regions.deposits(sid)) {
            return false;
        }
        let has_resource_deposit = regions
            .deposits(sid)
            .iter()
            .any(|d| d.discovered && d.resource_type == resource);
        has_resource_deposit && regions.building_level(sid, BuildingType::Mine) < mine_max_level
    })
}

/// 領内にFactory(level>0)があるか、または建設キューにFactoryがあるか。
fn has_active_factory_for_industrial_goods<C: AiCountry, R: AiRegions<C::Key>>(
    country: &C,
    regions: &R,
    owned: &[C::Key],
) -> bool {
    let already_built = owned
        .iter()
        .any(|&sid| regions.exists(sid) && regions.building_level(sid, BuildingType::Factory) > 0);
    already_built || country.queue_has_building(BuildingType::Factory)
}

/// 事前開発の対象(鉱床由来はMine、IndustrialGoodsはFactory)。
pub enum ProactiveDevelopmentTarget<K> {
    Mine(K),
    Factory(K),
}

/// 保有する生産建物の入力資源のうち、対応する構造的生産経路がまだ存在しない資源について、
/// 事前開発すべき建物種別と候補地域を1件だけ決定的に選ぶ(`ResourceType::ALL`の宣言順がtie-break)。
pub fn find_proactive_development_target<C: AiCountry, R: AiRegions<C::Key>>(
    country: &C,
    regions: &R,
    building_registry: &BuildingRegistry,
    owned: &[C::Key],
    mine_max_level: u32,
    factory_max_level: u32,
) -> Option<ProactiveDevelopmentTarget<C::Key>> {
    let needed = resource_types_required_by_owned_buildings(regions, building_registry, owned);
    if needed.is_empty() {
        return None;
    }
    for &resource in ResourceType::ALL.iter() {
        if resource == ResourceType::MagicCrystal || !needed.contains(&resource) {
            continue;
        }
        if resource == ResourceType::IndustrialGoods {
            if has_active_factory_for_industrial_goods(country, regions, owned) {
                continue;
            }
            if let Some(sid) = find_candidate_region(
                country,
                regions,
                owned,
                BuildingType::Factory,
                factory_max_level,
            ) {
                return Some(ProactiveDevelopmentTarget::Factory(sid));
            }
            continue;
        }
        if has_active_mine_for_resource(regions, owned, resource) {
            continue;
        }
        if let Some(sid) =
            find_undeveloped_deposit_region(country, regions, owned, mine_max_level, resource)
        {
            return Some(ProactiveDevelopmentTarget::Mine(sid));
        }
    }
    None
}

/// 優先度2.7(P28-005): 物流比率が`LOGISTICS_WELFARE_BONUS_RATIO`未満の地域のうち最も低い地域へ
/// Railwayを1件建設する。着工した場合だけ`true`。`false`のときは国庫・建設キューを変更しない。
pub fn try_logistics_railway_construction<C: AiCountry, R: AiRegions<C::Key>>(
    country: &mut C,
    regions: &R,
    building_registry: &BuildingRegistry,
    owned: &[C::Key],
) -> bool {
    let Some(def) = building_registry.get(BuildingType::Railway) else {
        return false;
    };

    let mut worst: Option<(f32, C::Key, u32)> = None;
    for &state_id in owned {
        if !regions.exists(state_id) {
            continue;
        }
        let level = regions.building_level(state_id, BuildingType::Railway);
        if level >= def.max_level {
            continue;
        }
        let ratio = regions.logistics_ratio(state_id);
        if ratio >= crate::economy::finance::LOGISTICS_WELFARE_BONUS_RATIO {
            continue;
        }
        if country.queue_has(state_id, BuildingType::Railway) {
            continue;
        }
        if worst.is_none_or(|(worst_ratio, _, _)| ratio < worst_ratio) {
            worst = Some((ratio, state_id, level));
        }
    }
    let Some((_, state_id, current_level)) = worst else {
        return false;
    };

    let treasury = country.treasury();
    let cost = def.construction_cost;
    let monthly_expenses = country.monthly_expenses();
    if !(treasury.is_finite()
        && cost.is_finite()
        && cost >= 0.0
        && monthly_expenses.is_finite()
        && monthly_expenses >= 0.0)
    {
        return false;
    }
    let required_treasury = cost + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
    if !required_treasury.is_finite() || treasury < required_treasury {
        return false;
    }

    country.set_treasury(treasury - cost);
    country.push_order(
        state_id,
        BuildingType::Railway,
        current_level + 1,
        def.required_progress,
        cost,
    );
    true
}

/// 軍事に関わる優先度の差し込み口(旧経路だけが実装する)。`true`なら着工済みで判断を終える。
pub trait ConstructionHooks<C> {
    /// 優先度2.5: 戦時MilitaryFactory建設。
    fn wartime_supply(&mut self, _country: &mut C) -> bool {
        false
    }
    /// 優先度2.8: Iron供給のMine建設(`true`のとき判断理由は`RawMaterialMinePriority`)。
    fn iron_supply(&mut self, _country: &mut C) -> bool {
        false
    }
    /// 優先度2.9: 軍需基盤の再建。
    fn military_rebuild(&mut self, _country: &mut C) -> bool {
        false
    }
}

/// 軍事の差し込みを持たない(新経路)。
pub struct NoHooks;
impl<C> ConstructionHooks<C> for NoHooks {}

/// 経済AIの建設判断(優先度 1 → 2 → 2.5 → 2.7 → 2.8 → 2.9 → 3)。旧`process_construction_ai_with_contexts`の
/// 本体を、国家・地域のトレイトの上へ移したもの。キューが2件以上、または建設可能な地域が無い場合は
/// 何もしない(従来と同じ)。
pub fn run_construction_priorities<C, R, H>(
    country: &mut C,
    regions: &R,
    building_registry: &BuildingRegistry,
    owned: &[C::Key],
    ai_state: &mut CountryAiState,
    factory_expansion_blocked: bool,
    hooks: &mut H,
) where
    C: AiCountry,
    R: AiRegions<C::Key>,
    H: ConstructionHooks<C>,
{
    // 最大2件までの建設キュー制限
    if country.queue_len() >= 2 {
        return;
    }
    if owned.is_empty() {
        ai_state.decision_reason = CountryAiDecisionReason::NoBuildableState;
        return;
    }

    // 優先度1: 食料緊急
    if country.stock(ResourceType::Food) < 50.0 {
        ai_state.decision_reason = CountryAiDecisionReason::FoodShortageFarmPriority;
        try_construct(country, regions, building_registry, owned, BuildingType::Farm);
        return;
    }

    // 優先度2: 入力資源の構造的な生産経路(Mine/Factory)の事前開発
    let mine_max_level = building_registry
        .get(BuildingType::Mine)
        .map(|d| d.max_level)
        .unwrap_or(0);
    let factory_max_level = building_registry
        .get(BuildingType::Factory)
        .map(|d| d.max_level)
        .unwrap_or(0);
    if let Some(target) = find_proactive_development_target(
        country,
        regions,
        building_registry,
        owned,
        mine_max_level,
        factory_max_level,
    ) {
        let (target_building, state_id, decision_reason) = match target {
            ProactiveDevelopmentTarget::Mine(sid) => (
                BuildingType::Mine,
                sid,
                CountryAiDecisionReason::RawMaterialMinePriority,
            ),
            ProactiveDevelopmentTarget::Factory(sid) => (
                BuildingType::Factory,
                sid,
                CountryAiDecisionReason::SteelShortageSteelMillPriority,
            ),
        };
        let factory_excluded =
            target_building == BuildingType::Factory && factory_expansion_blocked;
        if !factory_excluded
            && let Some(def) = building_registry.get(target_building)
            && country.treasury() >= def.construction_cost
        {
            ai_state.decision_reason = decision_reason;
            let current_level = regions.building_level(state_id, target_building);
            country.set_treasury(country.treasury() - def.construction_cost);
            country.push_order(
                state_id,
                target_building,
                current_level + 1,
                def.required_progress,
                def.construction_cost,
            );
            return;
        }
    }

    // 優先度2.5: 戦時MilitaryFactory建設(旧経路のみ)
    if hooks.wartime_supply(country) {
        ai_state.decision_reason = CountryAiDecisionReason::WarInProgress;
        return;
    }

    // 優先度2.7: 物流が低い地域へRailway
    if try_logistics_railway_construction(country, regions, building_registry, owned) {
        ai_state.decision_reason = CountryAiDecisionReason::PeacetimeDevelopment;
        return;
    }

    // 優先度2.8: Iron供給のためのMine(旧経路のみ)
    if hooks.iron_supply(country) {
        ai_state.decision_reason = CountryAiDecisionReason::RawMaterialMinePriority;
        return;
    }

    // 優先度2.9: 軍需基盤の再建(旧経路のみ)
    if hooks.military_rebuild(country) {
        ai_state.decision_reason = CountryAiDecisionReason::PeacetimeDevelopment;
        return;
    }

    // 優先度3: 通常判断(原材料不足 -> Mine、それ以外 -> Factory)
    let target_building = if country.stock(ResourceType::Wood) < 30.0
        || country.stock(ResourceType::Iron) < 30.0
        || country.stock(ResourceType::Coal) < 30.0
    {
        ai_state.decision_reason = CountryAiDecisionReason::RawMaterialMinePriority;
        BuildingType::Mine
    } else {
        ai_state.decision_reason = CountryAiDecisionReason::PeacetimeDevelopment;
        BuildingType::Factory
    };

    if target_building == BuildingType::Factory && factory_expansion_blocked {
        return;
    }

    match try_construct_peacetime(country, regions, building_registry, owned, target_building) {
        ConstructOutcome::Built => {}
        ConstructOutcome::InsufficientFunds => {
            ai_state.decision_reason = CountryAiDecisionReason::FundsShortage;
        }
        ConstructOutcome::NoCandidate => {
            if target_building == BuildingType::Mine {
                // 鉱床皆無(または資源入力が使えない)の場合は次候補(Factory)へフォールバックする。
                ai_state.decision_reason = CountryAiDecisionReason::PeacetimeDevelopment;
                if factory_expansion_blocked {
                    return;
                }
                if let ConstructOutcome::InsufficientFunds = try_construct_peacetime(
                    country,
                    regions,
                    building_registry,
                    owned,
                    BuildingType::Factory,
                ) {
                    ai_state.decision_reason = CountryAiDecisionReason::FundsShortage;
                }
            }
        }
    }
}

/// AIの税率調整: 人口加重平均の生活水準を求めて、既存の`decide_ai_tax_rate_delta`へ渡し、
/// 新しい税率を返す(変更なしなら現在の税率)。旧`process_tax_rate_ai`の本体。
/// `regions`は`(人口, 生活水準)`の組(対象国家の地域だけ)。
pub fn next_ai_tax_rate(
    monthly_balance: f64,
    tax_rate: f32,
    monthly_expenses: f64,
    regions: impl Iterator<Item = (u64, f32)>,
) -> f32 {
    let mut total_population = 0u64;
    let mut weighted_living_standard = 0.0f64;
    for (population, living_standard) in regions {
        total_population += population;
        weighted_living_standard += population as f64 * living_standard as f64;
    }
    let average_living_standard = if total_population == 0 {
        0.0
    } else {
        (weighted_living_standard / total_population as f64) as f32
    };
    match crate::economy::finance::decide_ai_tax_rate_delta(
        monthly_balance,
        tax_rate,
        total_population,
        average_living_standard,
        monthly_expenses,
    ) {
        Some(delta) => crate::economy::finance::apply_tax_rate_delta(tax_rate, delta),
        None => tax_rate,
    }
}

// ---------------------------------------------------------------- 旧経路のアダプタ

use crate::building::construction::{ConstructionQueueItem, ConstructionStatus};
use crate::common::StateId;
use crate::country::CountryData;
use crate::state::data::StateRegistry;

impl AiCountry for CountryData {
    type Key = StateId;
    fn treasury(&self) -> f64 {
        self.treasury
    }
    fn set_treasury(&mut self, value: f64) {
        self.treasury = value;
    }
    fn monthly_expenses(&self) -> f64 {
        self.monthly_expenses
    }
    fn stock(&self, resource: ResourceType) -> f64 {
        self.stockpile.get(resource)
    }
    fn queue_len(&self) -> usize {
        self.construction_queue.len()
    }
    fn queue_has(&self, region: StateId, building: BuildingType) -> bool {
        self.construction_queue
            .iter()
            .any(|item| item.state_id == region && item.building_type == building)
    }
    fn queue_has_building(&self, building: BuildingType) -> bool {
        self.construction_queue
            .iter()
            .any(|item| item.building_type == building)
    }
    fn push_order(
        &mut self,
        region: StateId,
        building: BuildingType,
        target_level: u32,
        required_progress: f64,
        paid_cost: f64,
    ) {
        self.construction_queue.push(ConstructionQueueItem {
            state_id: region,
            building_type: building,
            target_level,
            progress: 0.0,
            required_progress,
            paid_cost,
            status: ConstructionStatus::InQueue,
        });
    }
}

impl AiRegions<StateId> for StateRegistry {
    fn exists(&self, region: StateId) -> bool {
        self.get(region).is_some()
    }
    fn building_level(&self, region: StateId, building: BuildingType) -> u32 {
        self.get(region).map(|s| s.building_level(building)).unwrap_or(0)
    }
    fn buildings(&self, region: StateId) -> Option<&HashMap<BuildingType, u32>> {
        self.get(region).map(|s| &s.buildings)
    }
    fn deposits(&self, region: StateId) -> &[StateResourceDeposit] {
        self.get(region).map(|s| s.resource_deposits.as_slice()).unwrap_or(&[])
    }
    fn logistics_ratio(&self, region: StateId) -> f32 {
        self.get(region).map(|s| s.logistics_ratio).unwrap_or(1.0)
    }
}
