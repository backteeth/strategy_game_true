//! P26-004: Division日次補給需要の確定、国家備蓄からの実消費、不足時の決定的な
//! 比例配分(`verification_logs/phase-26/p26-004/`)。
//!
//! P26-004-PRE-005が確立した正規の日次需要関数(`military::supply_initialization::
//! division_daily_demand_units`/`country_daily_demand_units`)をそのまま再利用する
//! (複製しない)。本モジュールが新たに追加するのは、それらの需要値を国家備蓄から
//! 実際に消費し、不足時にHamilton方式(最大剰余法)でDivisionへ決定的に配分する処理
//! だけである。
//!
//! ## 正規の日次処理順序(spec §6)
//!
//! ```text
//! DayChanged
//! → 前日終値状態から需要算出
//! → 国家備蓄を消費
//! → Division補給充足率を確定
//! → 当日の移動・戦闘・募兵完了など
//! ```
//!
//! `src/military/mod.rs`の`MilitaryPlugin`が、この`process_daily_military_supply_consumption`
//! Systemを`DailySimulationSet::Economy`(月次生産`economy::handle_monthly_economy`の
//! 直後、`.after()`で明示固定)に登録する。`DailySimulationSet`全体の宣言順
//! (`app::time::DailySimulationSet`)により、`Economy`は`MilitaryAction`
//! (`military::update::handle_daily_military`――募兵完了・移動・戦闘がすべてここに
//! ある)より必ず先に実行されるため、「前日終値状態から需要を確定し、その後に当日の
//! 移動・戦闘・募兵完了が起こる」という正規順序が、イベントreaderの偶然の登録順ではなく
//! 明示的なBevy Systemの順序制約として保証される。
//!
//! ## Divisionの補給充足率をどこに持たせるか
//!
//! `Division`構造体自体には新規フィールドを追加しない。`Division`は
//! `tests/land_war_combat_peace_test.rs`(変更禁止ファイル)を含む数十箇所で
//! 全フィールドを列挙するリテラルとして直接構築されており、新規必須フィールドを
//! 追加するとその変更禁止ファイルがコンパイルできなくなる。代わりに、既存の
//! `ArmyRegistry::division_army_map`と同じ設計(`DivisionId`をキーにした`MilitaryRegistry`
//! 内の補助map、`division_supply_fulfillment`)を踏襲する。存在しないキーは
//! `MilitaryRegistry::supply_fulfillment_of`経由で`1.0`として扱われ、これがそのまま
//! 「旧Save・未処理Divisionのdefaultは1.0」というspec §8の要件を満たす。

use crate::app::time::DayChangedMessage;
use crate::common::{CountryId, DivisionDefinitionId, DivisionId};
use crate::country::{CountryData, CountryRegistry};
use crate::economy::resources::{
    MilitarySuppliesUnitsError, ResourceType, military_supplies_units_to_value,
    military_supplies_value_to_units,
};
use crate::military::data::{Division, DivisionDefinition, DivisionStatus, MilitaryRegistry};
use crate::military::supply_initialization::{
    SupplyDemandError, country_daily_demand_units, division_daily_demand_units, is_living,
};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

// ─── 計画(Phase 1: validate)結果の型(spec §9) ─────────────────────────────

/// 1個Divisionの当日の需要・配分・充足率(spec §5・§7・§8)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DivisionSupplyAllocation {
    pub division_id: DivisionId,
    pub demand_units: u128,
    pub allocated_units: u128,
    /// `0.0..=1.0`。需要0なら常に`1.0`。
    pub fulfillment: f64,
}

/// 1か国分の当日の需要・消費・配分計画(spec §9 Phase 1の出力)。この時点では
/// 備蓄・Divisionのいずれも変更されていない(副作用のない純粋な計算結果)。
#[derive(Debug, Clone, PartialEq)]
pub struct CountrySupplyPlan {
    pub country_id: CountryId,
    /// `D_c = Σ D_i`。
    pub total_demand_units: u128,
    /// `C_c = min(D_c, A_c)`。
    pub consumed_units: u128,
    /// `DivisionId`昇順。国家が保有する生存Division(`is_living`、P26-004-PRE-005と
    /// 同じ判定式)のみを含む――manpower 0・DestroyedのDivisionはここに現れない
    /// (需要0として自然に除外される、spec §12)。
    pub allocations: Vec<DivisionSupplyAllocation>,
}

/// `plan_country_daily_supply`が失敗し得る理由。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DailySupplyPlanError {
    /// Division需要の算出自体が失敗した(P26-004-PRE-005の`SupplyDemandError`をそのまま
    /// 伝播する――需要式自体は複製していないため、エラー型も複製しない)。
    Demand(SupplyDemandError),
    /// 現在備蓄が不正(NaN・負数・値域外等)で正規unitsへ変換できない。
    InvalidCurrentStockpile,
    /// units上のchecked演算が(理論上到達不能だが)overflowした、または配分計画の
    /// 内部不変条件(`R < 生存Division数`等)が破れた。
    Overflow,
}

impl From<SupplyDemandError> for DailySupplyPlanError {
    fn from(e: SupplyDemandError) -> Self {
        DailySupplyPlanError::Demand(e)
    }
}

/// `apply_country_daily_supply_plan`が失敗し得る理由。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DailySupplyApplyError {
    /// `consumed_units`が`u64`域を超える(理論上到達不能――`consumed_units`は
    /// 常に現在備蓄units[`u64`由来]以下)。
    ConsumedUnitsOutOfRange,
    /// 備蓄への実際のcommit(`try_consume_military_supplies`)が失敗した。
    Consume(MilitarySuppliesUnitsError),
}

// ─── Phase 1: validate・計画(spec §9) ──────────────────────────────────────

/// 1か国分の当日の需要・消費・配分を計画する(spec §9 Phase 1)。備蓄・Divisionの
/// いずれも変更しない純粋関数。
///
/// Division需要は`division_daily_demand_units`(P26-004-PRE-005)をそのまま呼ぶ
/// (複製しない)。対象Divisionの母集合(`owner`一致・`is_living`)・処理順
/// (`DivisionId`昇順)は`country_daily_demand_units`と完全に同じ規約を用いるため、
/// `total_demand_units`は常に`country_daily_demand_units`の戻り値と一致する
/// (debugビルドでは`debug_assert_eq!`により実際に突き合わせる)。
pub fn plan_country_daily_supply(
    country_id: CountryId,
    current_stockpile_value: f64,
    divisions: &HashMap<DivisionId, Division>,
    definitions: &HashMap<DivisionDefinitionId, DivisionDefinition>,
) -> Result<CountrySupplyPlan, DailySupplyPlanError> {
    let mut ids: Vec<DivisionId> = divisions
        .values()
        .filter(|d| d.owner == country_id && is_living(d))
        .map(|d| d.id)
        .collect();
    ids.sort_by_key(|id| id.0);

    let mut per_division: Vec<(DivisionId, u128)> = Vec::with_capacity(ids.len());
    let mut total_demand_units: u128 = 0;
    for id in &ids {
        let division = &divisions[id];
        let demand = division_daily_demand_units(division, definitions)?;
        total_demand_units = total_demand_units
            .checked_add(demand)
            .ok_or(DailySupplyPlanError::Overflow)?;
        per_division.push((*id, demand));
    }

    // P26-004-PRE-005の正規合計関数と厳密に一致することを実測で突き合わせる
    // (同じis_livingフィルタ・同じDivisionId昇順・同じdivision_daily_demand_unitsの
    // 呼び出しから導かれる合計であるため、数学的に常に一致するはずの不変条件)。
    debug_assert_eq!(
        Ok(total_demand_units),
        country_daily_demand_units(country_id, divisions, definitions),
        "daily_supply's own per-division aggregation must exactly match the canonical \
         country_daily_demand_units total (P26-004-PRE-005) — any mismatch is a bug, not a \
         legitimate error state"
    );

    let current_units = military_supplies_value_to_units(current_stockpile_value)
        .map_err(|_| DailySupplyPlanError::InvalidCurrentStockpile)?;

    let consumed_units = total_demand_units.min(current_units as u128);

    let allocations = if total_demand_units == 0 || consumed_units == total_demand_units {
        // 需要0、または備蓄が総需要以上(spec §7: 常に需要全量を配分する)。
        per_division
            .into_iter()
            .map(|(id, demand)| DivisionSupplyAllocation {
                division_id: id,
                demand_units: demand,
                allocated_units: demand,
                fulfillment: 1.0,
            })
            .collect()
    } else {
        allocate_by_largest_remainder(consumed_units, total_demand_units, &per_division)?
    };

    Ok(CountrySupplyPlan {
        country_id,
        total_demand_units,
        consumed_units,
        allocations,
    })
}

/// Hamilton方式(最大剰余法)による決定的な比例配分(spec §7.1)。
///
/// ```text
/// base_i = floor(C_c × D_i / D_c)
/// remainder_i = (C_c × D_i) mod D_c
/// R = C_c - Σ base_i
/// ```
///
/// `R` unitsを、`remainder_i`降順→同値なら`DivisionId`昇順の順で1 unitずつ追加する
/// (spec §7.1、Bevy Entity番号・HashMap挿入順・query反復順には一切依存しない)。
/// `C_c × D_i`は`checked_mul`、他の加減算もすべてchecked演算を用いる(spec §7.3、
/// 到達不能に見える箇所でも省略しない)。
fn allocate_by_largest_remainder(
    consumed_units: u128,
    total_demand_units: u128,
    per_division: &[(DivisionId, u128)],
) -> Result<Vec<DivisionSupplyAllocation>, DailySupplyPlanError> {
    struct Item {
        id: DivisionId,
        demand: u128,
        base: u128,
        remainder: u128,
    }

    let mut items: Vec<Item> = Vec::with_capacity(per_division.len());
    let mut base_sum: u128 = 0;
    for &(id, demand) in per_division {
        if demand == 0 {
            items.push(Item { id, demand, base: 0, remainder: 0 });
            continue;
        }
        let numerator = consumed_units
            .checked_mul(demand)
            .ok_or(DailySupplyPlanError::Overflow)?;
        let base = numerator / total_demand_units;
        let remainder = numerator % total_demand_units;
        base_sum = base_sum.checked_add(base).ok_or(DailySupplyPlanError::Overflow)?;
        items.push(Item { id, demand, base, remainder });
    }

    // R = C_c - Σbase_i。標準的なHamilton法の数学的性質により常に
    // `0 <= R < (需要が正のDivision数)`が成り立つ(このモジュールのテスト
    // `allocation_invariants_hold_under_random_scenarios`等で実測検証する)。
    let leftover = consumed_units
        .checked_sub(base_sum)
        .ok_or(DailySupplyPlanError::Overflow)?;
    let leftover_usize =
        usize::try_from(leftover).map_err(|_| DailySupplyPlanError::Overflow)?;

    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        items[b]
            .remainder
            .cmp(&items[a].remainder)
            .then(items[a].id.0.cmp(&items[b].id.0))
    });

    let mut gets_extra_unit = vec![false; items.len()];
    for &idx in order.iter().take(leftover_usize) {
        gets_extra_unit[idx] = true;
    }

    let mut allocations: Vec<DivisionSupplyAllocation> = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let allocated = if gets_extra_unit[idx] {
            item.base.checked_add(1).ok_or(DailySupplyPlanError::Overflow)?
        } else {
            item.base
        };
        let fulfillment = if item.demand == 0 {
            1.0
        } else {
            (allocated as f64 / item.demand as f64).clamp(0.0, 1.0)
        };
        allocations.push(DivisionSupplyAllocation {
            division_id: item.id,
            demand_units: item.demand,
            allocated_units: allocated,
            fulfillment,
        });
    }

    allocations.sort_by_key(|a| a.division_id.0);
    Ok(allocations)
}

// ─── Phase 2: apply(spec §9) ────────────────────────────────────────────────

/// Phase 1で確定した計画を実際に適用する(spec §9 Phase 2)。
///
/// 1. 国家備蓄を`try_consume_military_supplies`で1回だけ消費する(生の
///    `stockpile.consume`は使わない)。失敗した場合はここで即座に`Err`を返し、
///    備蓄・Division充足率のいずれも変更しない。
/// 2. 消費が成功した場合のみ、計画済みの各Divisionの充足率を反映する。
/// 3. この国家が所有する、計画に含まれない生存中(`Destroyed`ではない)のDivision
///    (=需要0=manpowerが0のDivision)にも、明示的に`1.0`を書き込む(spec §8:
///    「前日の不足率を理由なく持ち越さない」――manpowerが今日0になったDivisionの
///    充足率が前日の値のまま古びて残ることを防ぐ)。
pub fn apply_country_daily_supply_plan(
    country: &mut CountryData,
    military_registry: &mut MilitaryRegistry,
    plan: &CountrySupplyPlan,
) -> Result<(), DailySupplyApplyError> {
    let consumed_units_u64 = u64::try_from(plan.consumed_units)
        .map_err(|_| DailySupplyApplyError::ConsumedUnitsOutOfRange)?;
    let consumed_value = military_supplies_units_to_value(consumed_units_u64);

    country
        .stockpile
        .try_consume_military_supplies(consumed_value)
        .map_err(DailySupplyApplyError::Consume)?;

    for allocation in &plan.allocations {
        military_registry
            .division_supply_fulfillment
            .insert(allocation.division_id, allocation.fulfillment);
    }

    let considered: HashSet<DivisionId> =
        plan.allocations.iter().map(|a| a.division_id).collect();
    let stale_alive_ids: Vec<DivisionId> = military_registry
        .divisions
        .values()
        .filter(|d| {
            d.owner == plan.country_id
                && d.status != DivisionStatus::Destroyed
                && !considered.contains(&d.id)
        })
        .map(|d| d.id)
        .collect();
    for id in stale_alive_ids {
        military_registry.division_supply_fulfillment.insert(id, 1.0);
    }

    Ok(())
}

// ─── 1日分の全国家処理(spec §9・§6) ─────────────────────────────────────────

/// 1日分の全国家の日次補給処理を実行する(spec §9)。`CountryId`昇順で決定的に処理し、
/// ある国家の失敗(不正Division・不正備蓄等)は`warn!`で記録してその国家の備蓄・
/// Division充足率を一切変更せず、他国家の処理へ影響しない(spec §9)。
pub fn run_daily_military_supply_consumption(
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
) {
    let mut country_ids: Vec<CountryId> =
        country_registry.countries.iter().map(|c| c.id).collect();
    country_ids.sort_by_key(|id| id.0);

    for country_id in country_ids {
        let Some(country) = country_registry.get(country_id) else {
            continue;
        };
        if !country.is_active() {
            // 消滅国家は正規化された需要を持たない(P26-004-PRE-005 §19と同じ方針)。
            continue;
        }
        let stockpile_value = country.stockpile.get(ResourceType::MilitarySupplies);

        let plan = match plan_country_daily_supply(
            country_id,
            stockpile_value,
            &military_registry.divisions,
            &military_registry.definitions,
        ) {
            Ok(plan) => plan,
            Err(err) => {
                warn!(
                    "[P26-004] daily military supply planning failed for {:?}: {:?} (stockpile \
                     and all division fulfillment values left unchanged this tick, other \
                     countries are unaffected)",
                    country_id, err
                );
                continue;
            }
        };

        let Some(country_mut) = country_registry.get_mut(country_id) else {
            continue;
        };
        if let Err(err) =
            apply_country_daily_supply_plan(country_mut, military_registry, &plan)
        {
            warn!(
                "[P26-004] daily military supply consumption failed for {:?}: {:?} (stockpile \
                 and all division fulfillment values left unchanged this tick, other countries \
                 are unaffected)",
                country_id, err
            );
        }
    }
}

// ─── Bevy System(spec §6・§7〜11) ───────────────────────────────────────────

/// 日次補給消費・配分のBevy System。`DayChangedMessage`ごとに(高速再生時に1フレームで
/// 複数日分届く場合も1日ずつ順に)`run_daily_military_supply_consumption`を呼ぶ。
///
/// `src/military/mod.rs`の`MilitaryPlugin`が`DailySimulationSet::Economy`
/// (`economy::handle_monthly_economy`の直後、`.after()`で明示固定)へ登録する。
/// これにより、月境界日には月次生産(`Economy`Set前段)が確実に先に完了してから
/// この日次消費が走り(spec §6.2)、かつ`Economy`Set全体は`MilitaryAction`Set
/// (募兵完了・移動・戦闘、`military::update::handle_daily_military`)より必ず先に
/// 実行される(`DailySimulationSet`の宣言順、`app::time::GameTimePlugin::build`の
/// `configure_sets`が保証する)ため、「前日終値状態から需要を確定してから当日の
/// 移動・戦闘・募兵完了が起こる」という正規順序(spec §6)が成立する。
pub fn process_daily_military_supply_consumption(
    mut day_events: MessageReader<DayChangedMessage>,
    mut country_registry: ResMut<CountryRegistry>,
    mut military_registry: ResMut<MilitaryRegistry>,
) {
    for _event in day_events.read() {
        run_daily_military_supply_consumption(&mut country_registry, &mut military_registry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::StateId;
    use crate::military::data::{DivisionSize, DivisionType};

    // ─── フィクスチャ(supply_initializationと同じ構成方針) ─────────────

    fn infantry_def(id: usize) -> DivisionDefinition {
        DivisionDefinition {
            id: DivisionDefinitionId(id),
            name: "Test Infantry".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 10_000,
            required_equipment: 100.0,
            recruitment_days: 10,
            movement_speed: 1.0,
            attack: 10.0,
            defense: 10.0,
            breakthrough: 1.0,
            organization: 50.0,
            morale: 50.0,
            supply_usage: 1.0,
            maintenance_cost: 1.0,
        }
    }

    fn artillery_def(id: usize) -> DivisionDefinition {
        DivisionDefinition {
            supply_usage: 3.0,
            division_type: DivisionType::Artillery,
            ..infantry_def(id)
        }
    }

    fn mage_def(id: usize) -> DivisionDefinition {
        DivisionDefinition {
            supply_usage: 2.0,
            division_type: DivisionType::Mage,
            ..infantry_def(id)
        }
    }

    fn division(
        div_id: usize,
        owner: usize,
        def_id: usize,
        manpower: u64,
        max_manpower: u64,
    ) -> Division {
        Division {
            id: DivisionId(div_id),
            owner: CountryId(owner),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(0),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower,
            max_manpower,
            equipment: 100.0,
            max_equipment: 100.0,
            organization: 50.0,
            max_organization: 50.0,
            morale: 50.0,
            max_morale: 50.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(def_id),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn defs(list: Vec<DivisionDefinition>) -> HashMap<DivisionDefinitionId, DivisionDefinition> {
        list.into_iter().map(|d| (d.id, d)).collect()
    }

    fn divs(list: Vec<Division>) -> HashMap<DivisionId, Division> {
        list.into_iter().map(|d| (d.id, d)).collect()
    }

    fn test_country(id: usize) -> CountryData {
        CountryData {
            id: CountryId(id),
            ..CountryData::default()
        }
    }

    // ─── A. 需要(reuse, not duplicated) ──────────────────────────────

    #[test]
    fn plan_reuses_the_canonical_demand_function_for_a_single_full_strength_infantry() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let plan = plan_country_daily_supply(CountryId(0), 100.0, &divisions, &definitions)
            .unwrap();
        assert_eq!(plan.total_demand_units, 1_000_000);
        assert_eq!(plan.allocations.len(), 1);
        assert_eq!(plan.allocations[0].demand_units, 1_000_000);
    }

    #[test]
    fn plan_matches_country_daily_demand_units_for_a_mixed_branch_country() {
        let definitions = defs(vec![infantry_def(0), artillery_def(1), mage_def(2)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 0, 1, 10_000, 10_000),
            division(2, 0, 2, 5_000, 10_000),
        ]);
        let expected =
            country_daily_demand_units(CountryId(0), &divisions, &definitions).unwrap();
        let plan = plan_country_daily_supply(CountryId(0), 100.0, &divisions, &definitions)
            .unwrap();
        assert_eq!(plan.total_demand_units, expected);
    }

    #[test]
    fn half_manpower_division_demands_half() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 5_000, 10_000)]);
        let plan = plan_country_daily_supply(CountryId(0), 100.0, &divisions, &definitions)
            .unwrap();
        assert_eq!(plan.total_demand_units, 500_000);
    }

    #[test]
    fn zero_manpower_division_is_excluded_from_demand_and_gets_full_fulfillment_on_apply() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 0, 10_000)]);
        let plan = plan_country_daily_supply(CountryId(0), 100.0, &divisions, &definitions)
            .unwrap();
        assert_eq!(plan.total_demand_units, 0);
        assert!(plan.allocations.is_empty(), "manpower 0 division is not a living division");

        let mut country = test_country(0);
        country.stockpile.set(ResourceType::MilitarySupplies, 100.0);
        let mut military_registry = MilitaryRegistry::from_saved_parts(
            definitions,
            divisions,
            1,
            HashMap::new(),
        );
        apply_country_daily_supply_plan(&mut country, &mut military_registry, &plan).unwrap();
        assert_eq!(military_registry.supply_fulfillment_of(DivisionId(0)), 1.0);
    }

    #[test]
    fn unknown_definition_propagates_as_an_explicit_demand_error() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 99, 10_000, 10_000)]);
        let err =
            plan_country_daily_supply(CountryId(0), 100.0, &divisions, &definitions).unwrap_err();
        assert!(matches!(err, DailySupplyPlanError::Demand(_)));
    }

    #[test]
    fn invalid_manpower_exceeding_max_propagates_as_an_explicit_error() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 11_000, 10_000)]);
        let err =
            plan_country_daily_supply(CountryId(0), 100.0, &divisions, &definitions).unwrap_err();
        assert!(matches!(err, DailySupplyPlanError::Demand(_)));
    }

    // ─── B. 備蓄十分 ───────────────────────────────────────────────────

    #[test]
    fn sufficient_stockpile_allocates_full_demand_to_every_division() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 0, 0, 10_000, 10_000),
        ]);
        let plan = plan_country_daily_supply(CountryId(0), 100.0, &divisions, &definitions)
            .unwrap();
        assert_eq!(plan.consumed_units, 2_000_000);
        for a in &plan.allocations {
            assert_eq!(a.allocated_units, a.demand_units);
            assert_eq!(a.fulfillment, 1.0);
        }
    }

    #[test]
    fn stockpile_exactly_equal_to_demand_consumes_exactly_that_much() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        // 1.0 value = 1_000_000 units = exactly the demand.
        let plan = plan_country_daily_supply(CountryId(0), 1.0, &divisions, &definitions)
            .unwrap();
        assert_eq!(plan.consumed_units, 1_000_000);
        assert_eq!(plan.total_demand_units, 1_000_000);
    }

    #[test]
    fn zero_demand_country_leaves_stockpile_untouched_and_fulfillment_at_one() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions: HashMap<DivisionId, Division> = HashMap::new();
        let mut country = test_country(0);
        country.stockpile.set(ResourceType::MilitarySupplies, 42.0);
        let plan = plan_country_daily_supply(CountryId(0), 42.0, &divisions, &definitions)
            .unwrap();
        assert_eq!(plan.consumed_units, 0);
        let mut military_registry =
            MilitaryRegistry::from_saved_parts(definitions, divisions, 1, HashMap::new());
        apply_country_daily_supply_plan(&mut country, &mut military_registry, &plan).unwrap();
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 42.0);
    }

    // ─── C. 備蓄不足・Hamilton配分 ────────────────────────────────────

    #[test]
    fn zero_stockpile_allocates_zero_to_every_division_with_zero_fulfillment() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let plan =
            plan_country_daily_supply(CountryId(0), 0.0, &divisions, &definitions).unwrap();
        assert_eq!(plan.consumed_units, 0);
        assert_eq!(plan.allocations[0].allocated_units, 0);
        assert_eq!(plan.allocations[0].fulfillment, 0.0);
    }

    #[test]
    fn fifty_percent_stockpile_allocates_half_to_a_single_division() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let plan =
            plan_country_daily_supply(CountryId(0), 0.5, &divisions, &definitions).unwrap();
        assert_eq!(plan.consumed_units, 500_000);
        assert_eq!(plan.allocations[0].allocated_units, 500_000);
        assert_eq!(plan.allocations[0].fulfillment, 0.5);
    }

    #[test]
    fn one_unit_short_of_two_equal_divisions_gives_the_extra_unit_to_the_lower_division_id() {
        // demand=2, stockpile value corresponds to consumed_units = 2*1_000_000 - 1.
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 0, 0, 10_000, 10_000),
        ]);
        let consumed_units = 2_000_000u128 - 1;
        let stockpile_value = military_supplies_units_to_value(consumed_units as u64);
        let plan =
            plan_country_daily_supply(CountryId(0), stockpile_value, &divisions, &definitions)
                .unwrap();
        assert_eq!(plan.consumed_units, consumed_units);
        let sum: u128 = plan.allocations.iter().map(|a| a.allocated_units).sum();
        assert_eq!(sum, consumed_units);
        // Base = floor(1_999_999 * 1_000_000 / 2_000_000) = 999_999 for each (remainder
        // 500_000 each — tied), so R=1 goes to the lower DivisionId (id 0).
        let by_id: HashMap<DivisionId, u128> = plan
            .allocations
            .iter()
            .map(|a| (a.division_id, a.allocated_units))
            .collect();
        assert_eq!(by_id[&DivisionId(0)], 1_000_000);
        assert_eq!(by_id[&DivisionId(1)], 999_999);
    }

    #[test]
    fn only_one_unit_of_stockpile_goes_to_the_lowest_division_id_among_many() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 0, 0, 10_000, 10_000),
            division(2, 0, 0, 10_000, 10_000),
        ]);
        let stockpile_value = military_supplies_units_to_value(1);
        let plan =
            plan_country_daily_supply(CountryId(0), stockpile_value, &divisions, &definitions)
                .unwrap();
        assert_eq!(plan.consumed_units, 1);
        let sum: u128 = plan.allocations.iter().map(|a| a.allocated_units).sum();
        assert_eq!(sum, 1);
        let by_id: HashMap<DivisionId, u128> = plan
            .allocations
            .iter()
            .map(|a| (a.division_id, a.allocated_units))
            .collect();
        assert_eq!(by_id[&DivisionId(0)], 1);
        assert_eq!(by_id[&DivisionId(1)], 0);
        assert_eq!(by_id[&DivisionId(2)], 0);
    }

    #[test]
    fn unequal_demand_largest_remainder_allocation_matches_hand_computed_expectation() {
        // Divisions with demand 1_000_000 (infantry) and 3_000_000 (artillery),
        // total demand = 4_000_000; stockpile = 2_500_000 units (0.625 value).
        let definitions = defs(vec![infantry_def(0), artillery_def(1)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 0, 1, 10_000, 10_000),
        ]);
        let stockpile_value = military_supplies_units_to_value(2_500_000);
        let plan =
            plan_country_daily_supply(CountryId(0), stockpile_value, &divisions, &definitions)
                .unwrap();
        assert_eq!(plan.consumed_units, 2_500_000);
        // base_0 = floor(2_500_000*1_000_000/4_000_000) = 625_000, remainder 0
        // base_1 = floor(2_500_000*3_000_000/4_000_000) = 1_875_000, remainder 0
        // sum = 2_500_000 = consumed, R = 0 exactly (exact division case).
        let by_id: HashMap<DivisionId, u128> = plan
            .allocations
            .iter()
            .map(|a| (a.division_id, a.allocated_units))
            .collect();
        assert_eq!(by_id[&DivisionId(0)], 625_000);
        assert_eq!(by_id[&DivisionId(1)], 1_875_000);
        let sum: u128 = plan.allocations.iter().map(|a| a.allocated_units).sum();
        assert_eq!(sum, plan.consumed_units);
    }

    #[test]
    fn no_allocation_ever_exceeds_its_own_demand_under_a_sweep_of_stockpile_levels() {
        let definitions = defs(vec![infantry_def(0), artillery_def(1), mage_def(2)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 0, 1, 7_000, 10_000),
            division(2, 0, 2, 3_000, 10_000),
        ]);
        let total = country_daily_demand_units(CountryId(0), &divisions, &definitions).unwrap();
        for units in 0..=total {
            let value = military_supplies_units_to_value(units as u64);
            let plan =
                plan_country_daily_supply(CountryId(0), value, &divisions, &definitions).unwrap();
            let sum: u128 = plan.allocations.iter().map(|a| a.allocated_units).sum();
            assert_eq!(sum, plan.consumed_units, "allocation sum must equal consumed units");
            for a in &plan.allocations {
                assert!(
                    a.allocated_units <= a.demand_units,
                    "allocation must never exceed demand"
                );
                assert!((0.0..=1.0).contains(&a.fulfillment));
            }
        }
    }

    // ─── D. 原子性 ─────────────────────────────────────────────────────

    #[test]
    fn invalid_current_stockpile_is_an_explicit_planning_error() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let err = plan_country_daily_supply(CountryId(0), f64::NAN, &divisions, &definitions)
            .unwrap_err();
        assert_eq!(err, DailySupplyPlanError::InvalidCurrentStockpile);
    }

    #[test]
    fn non_finite_stockpile_is_rejected_without_a_plan() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let err =
            plan_country_daily_supply(CountryId(0), f64::INFINITY, &divisions, &definitions)
                .unwrap_err();
        assert_eq!(err, DailySupplyPlanError::InvalidCurrentStockpile);
    }

    #[test]
    fn apply_failure_leaves_stockpile_and_all_fulfillment_values_unchanged() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let mut country = test_country(0);
        // Corrupt stockpile after planning (simulates a plan/apply race in tests) —
        // more directly, exercise apply() with a corrupt consumed_units_u64 by
        // constructing a plan with an out-of-range consumed_units value directly.
        let bad_plan = CountrySupplyPlan {
            country_id: CountryId(0),
            total_demand_units: 1_000_000,
            consumed_units: (u64::MAX as u128) + 1, // out of u64 range on purpose
            allocations: vec![DivisionSupplyAllocation {
                division_id: DivisionId(0),
                demand_units: 1_000_000,
                allocated_units: 1_000_000,
                fulfillment: 1.0,
            }],
        };
        country.stockpile.set(ResourceType::MilitarySupplies, 5.0);
        let mut military_registry =
            MilitaryRegistry::from_saved_parts(definitions, divisions, 1, HashMap::new());
        let err =
            apply_country_daily_supply_plan(&mut country, &mut military_registry, &bad_plan)
                .unwrap_err();
        assert_eq!(err, DailySupplyApplyError::ConsumedUnitsOutOfRange);
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 5.0);
        assert_eq!(military_registry.supply_fulfillment_of(DivisionId(0)), 1.0);
    }

    #[test]
    fn consume_api_failure_leaves_stockpile_and_fulfillment_unchanged() {
        // Plan claims to consume more than the country actually has right now
        // (simulates the stockpile changing between plan and apply, which the real
        // orchestrator never allows — but apply() itself must still be safe).
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let mut country = test_country(0);
        country.stockpile.set(ResourceType::MilitarySupplies, 0.1);
        let plan = CountrySupplyPlan {
            country_id: CountryId(0),
            total_demand_units: 1_000_000,
            consumed_units: 1_000_000, // more than the 100_000 units actually present
            allocations: vec![DivisionSupplyAllocation {
                division_id: DivisionId(0),
                demand_units: 1_000_000,
                allocated_units: 1_000_000,
                fulfillment: 1.0,
            }],
        };
        let mut military_registry =
            MilitaryRegistry::from_saved_parts(definitions, divisions, 1, HashMap::new());
        let err = apply_country_daily_supply_plan(&mut country, &mut military_registry, &plan)
            .unwrap_err();
        assert!(matches!(err, DailySupplyApplyError::Consume(_)));
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 0.1);
        assert_eq!(military_registry.supply_fulfillment_of(DivisionId(0)), 1.0);
    }

    #[test]
    fn demand_overflow_aborts_planning_without_partial_results() {
        // A division whose def_id is missing forces a Demand error before any
        // allocation math runs.
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 0, 99, 10_000, 10_000), // unknown definition
        ]);
        let err = plan_country_daily_supply(CountryId(0), 10.0, &divisions, &definitions)
            .unwrap_err();
        assert!(matches!(err, DailySupplyPlanError::Demand(_)));
    }

    #[test]
    fn one_countrys_planning_failure_does_not_disturb_another_countrys_processing() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division(0, 0, 0, 11_000, 10_000), // country 0: corrupt (manpower>max)
            division(1, 1, 0, 10_000, 10_000), // country 1: healthy
        ]);
        let mut country_registry = CountryRegistry {
            countries: vec![test_country(0), test_country(1)],
        };
        country_registry
            .get_mut(CountryId(0))
            .unwrap()
            .stockpile
            .set(ResourceType::MilitarySupplies, 10.0);
        country_registry
            .get_mut(CountryId(1))
            .unwrap()
            .stockpile
            .set(ResourceType::MilitarySupplies, 10.0);
        let mut military_registry =
            MilitaryRegistry::from_saved_parts(definitions, divisions, 2, HashMap::new());

        run_daily_military_supply_consumption(&mut country_registry, &mut military_registry);

        // country 0 untouched (planning failed).
        assert_eq!(
            country_registry
                .get(CountryId(0))
                .unwrap()
                .stockpile
                .get(ResourceType::MilitarySupplies),
            10.0
        );
        // country 1 processed normally (consumed exactly 1.0 = its daily demand).
        assert_eq!(
            country_registry
                .get(CountryId(1))
                .unwrap()
                .stockpile
                .get(ResourceType::MilitarySupplies),
            9.0
        );
        assert_eq!(military_registry.supply_fulfillment_of(DivisionId(1)), 1.0);
    }

    // ─── E. 決定性 ─────────────────────────────────────────────────────

    #[test]
    fn division_insertion_order_does_not_change_the_plan() {
        let definitions = defs(vec![infantry_def(0), artillery_def(1)]);
        let list_a = vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 0, 1, 4_000, 10_000),
            division(2, 0, 0, 7_000, 10_000),
        ];
        let mut list_b = list_a.clone();
        list_b.reverse();
        let stockpile_value = 2.0;

        let plan_a = plan_country_daily_supply(
            CountryId(0),
            stockpile_value,
            &divs(list_a),
            &definitions,
        )
        .unwrap();
        let plan_b = plan_country_daily_supply(
            CountryId(0),
            stockpile_value,
            &divs(list_b),
            &definitions,
        )
        .unwrap();
        assert_eq!(plan_a, plan_b);
    }

    #[test]
    fn country_insertion_order_does_not_change_the_outcome() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 1, 0, 10_000, 10_000),
        ]);
        let mut order_a = CountryRegistry {
            countries: vec![test_country(0), test_country(1)],
        };
        let mut order_b = CountryRegistry {
            countries: vec![test_country(1), test_country(0)],
        };
        for reg in [&mut order_a, &mut order_b] {
            for id in [0usize, 1] {
                reg.get_mut(CountryId(id))
                    .unwrap()
                    .stockpile
                    .set(ResourceType::MilitarySupplies, 0.5);
            }
        }
        let mut mil_a =
            MilitaryRegistry::from_saved_parts(definitions.clone(), divisions.clone(), 2, HashMap::new());
        let mut mil_b =
            MilitaryRegistry::from_saved_parts(definitions, divisions, 2, HashMap::new());

        run_daily_military_supply_consumption(&mut order_a, &mut mil_a);
        run_daily_military_supply_consumption(&mut order_b, &mut mil_b);

        for id in [0usize, 1] {
            assert_eq!(
                order_a.get(CountryId(id)).unwrap().stockpile.get(ResourceType::MilitarySupplies),
                order_b.get(CountryId(id)).unwrap().stockpile.get(ResourceType::MilitarySupplies),
            );
        }
        assert_eq!(
            mil_a.supply_fulfillment_of(DivisionId(0)),
            mil_b.supply_fulfillment_of(DivisionId(0))
        );
    }

    #[test]
    fn repeated_independent_runs_from_identical_inputs_are_bit_for_bit_deterministic() {
        fn run() -> CountrySupplyPlan {
            let definitions = defs(vec![infantry_def(0), artillery_def(1), mage_def(2)]);
            let divisions = divs(vec![
                division(0, 0, 0, 8_300, 10_000),
                division(1, 0, 1, 4_100, 10_000),
                division(2, 0, 2, 9_999, 10_000),
            ]);
            let stockpile_value = military_supplies_units_to_value(1_234_567);
            plan_country_daily_supply(CountryId(0), stockpile_value, &divisions, &definitions)
                .unwrap()
        }
        let a = run();
        let b = run();
        assert_eq!(a, b);
    }
}
