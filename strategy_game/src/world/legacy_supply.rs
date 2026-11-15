//! P32-037 (M6-d): 旧経路の日次補給(`military::daily_supply`・`military::supply_initialization`・`economy::resources`)を、新経路の師団の**戦闘能力と補給消費量の値だけ**で
//! 再利用するための隔離アダプタ(`legacy_combat` と同じ作法)。
//!
//! U8 の B 案: 旧の日次補給から、**需要の計算・備蓄の消費・最大剰余法(Hamilton 方式)による配分**を、ID の型を知らない共通の核
//! (`daily_supply::plan_supply_core`・`commit_supply_consumption`)として切り出した。到達資格は呼び出し側が判定する(旧は `StateId` の連結集合、新経路は `ProvinceId`)。
//! このアダプタは、旧の需要関数(`division_daily_demand_units`)を呼ぶために、計算の間だけ戦闘能力と補給消費量から旧 `Division` と `DivisionDefinition` の一時的な値を作り、
//! 旧の `CountryStockpile` を一時的に作って備蓄の消費(`try_consume_military_supplies`)を呼ぶ。旧のレジストリ・戦闘・状態へは何も渡さない。
//! このモジュールだけが旧の補給の計算に依存する。計算式は旧の関数をそのまま呼ぶ(複製しない)。

use super::legacy_combat::stats_view;
use super::movement::{CombatStats, MapDivisionId, SupplyFailure};
use crate::common::{CountryId, DivisionDefinitionId};
use crate::economy::resources::{CountryStockpile, ResourceType, military_supplies_value_to_units};
use crate::military::daily_supply::{DailySupplyApplyError, DailySupplyPlanError, SupplyCorePlan, SupplyDemandEntry, commit_supply_consumption, plan_supply_core};
use crate::military::data::{DivisionDefinition, DivisionSize, DivisionType};
use crate::military::supply_initialization::{SupplyDemandError, division_daily_demand_units};
use std::collections::HashMap;

/// 呼び出し側が到達資格を判定済みの需要(旧の共通の核の入力)。
pub type DemandEntry = SupplyDemandEntry;
/// 共通の核の計画(旧の `CountrySupplyPlan` の後半と同じ内容)。
pub type CorePlan = SupplyCorePlan;

/// 1つの師団のその日の需要(単位は旧の固定小数点)。旧の `division_daily_demand_units`(人員 0 は 0、人員比に比例、補給消費量の検証)をそのまま呼ぶ。
pub fn demand_units(id: MapDivisionId, owner: CountryId, stats: &CombatStats, supply_usage: f32) -> Result<u128, SupplyDemandError> {
    let division = stats_view(id, owner, stats);
    let def = DivisionDefinition {
        id: DivisionDefinitionId(0),
        name: String::new(),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        required_manpower: 1,
        required_equipment: 1.0,
        recruitment_days: 1,
        movement_speed: 1.0,
        attack: 1.0,
        defense: 1.0,
        breakthrough: 1.0,
        organization: 1.0,
        morale: 1.0,
        supply_usage,
        maintenance_cost: 0.0,
    };
    let defs: HashMap<DivisionDefinitionId, DivisionDefinition> = [(def.id, def)].into_iter().collect();
    division_daily_demand_units(&division, &defs)
}

/// 到達資格を判定済みの需要から、消費量と最大剰余法の配分を決める(旧の共通の核)。
pub fn plan(stockpile_value: f64, entries: &[DemandEntry]) -> Result<CorePlan, DailySupplyPlanError> {
    plan_supply_core(stockpile_value, entries)
}

/// 計画した消費量を備蓄から消費して、消費後の備蓄の値を返す(旧の `commit_supply_consumption`。失敗したら備蓄は変えない)。
pub fn consume(stockpile_value: f64, consumed_units: u128) -> Result<f64, DailySupplyApplyError> {
    let mut stockpile = CountryStockpile::default();
    stockpile.set(ResourceType::MilitarySupplies, stockpile_value);
    commit_supply_consumption(&mut stockpile, consumed_units)?;
    Ok(stockpile.get(ResourceType::MilitarySupplies))
}

/// 備蓄の値を旧の固定小数点の単位へ(正規化できない値は `None`)。
pub fn stockpile_units(value: f64) -> Option<u64> {
    military_supplies_value_to_units(value).ok()
}

/// 補給消費量が旧の規則で正規化できるか。
pub fn usage_is_valid(supply_usage: f32) -> bool {
    military_supplies_value_to_units(supply_usage as f64).is_ok()
}

/// 計画の失敗を、`Eq` を保てる分類へ。
pub fn plan_failure(e: &DailySupplyPlanError) -> SupplyFailure {
    match e {
        DailySupplyPlanError::Demand(_) => SupplyFailure::Demand,
        DailySupplyPlanError::InvalidCurrentStockpile => SupplyFailure::InvalidStockpile,
        DailySupplyPlanError::Overflow => SupplyFailure::Overflow,
    }
}
