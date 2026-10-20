//! P32-012 (M4-D): 軍事状況を参照する建設AIの優先度(2.5 戦時MilitaryFactory、2.8 Iron供給のMine、
//! 2.9 軍需基盤の再建)と、履歴に基づく判断(Iron不足の履歴・Factory飢餓による増設抑止・軍需生産の履歴)の
//! **判断本体**。旧`country_ai.rs`にあった関数の本体を、国家(`AiCountry`)と地域(`AiRegions`)の
//! トレイトの上へ移したもので、旧`StateData`も新ステート(`MapStateId`)も同じ関数を通る。
//! 条件・閾値・候補の評価順・同点の扱い・予算の規則は従来と同一(式を変えていない)。乱数は使わない。
//!
//! 軍事状況は`MilitaryAiFacts`として**明示的に渡す**。旧経路は軍隊・戦争の登録簿から作り、新経路は
//! 検証用シナリオが明示する。入力が無い(`None`)ことを「平和」「脅威ゼロ」とは扱わない。
//!
//! 軍隊への命令は一切含まない(建設注文だけ)。位置施設(港・要塞・駅)の建設判断も含まない。

use crate::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use crate::country::ai_core::{AiCountry, AiRegions, PEACETIME_CONSTRUCTION_RESERVE_MONTHS};
use crate::economy::iron_shortage::{IronShortageAssessment, IronShortageHistory};
use crate::economy::resources::{ResourceType, StateResourceDeposit, is_crystal_only_state};
use crate::economy::state_access::StateEconomyAccess;
use crate::military::supply_production_history::SupplyProductionHistory;

/// 戦時MilitaryFactory建設で、建設費の支払い後も国庫に残すべき月次経常支出の月数。
/// 平時の任意開発(優先度3)の`PEACETIME_CONSTRUCTION_RESERVE_MONTHS`とは独立した定数。
pub const WARTIME_SUPPLY_CONSTRUCTION_RESERVE_MONTHS: f64 = 1.0;

/// 戦時MilitaryFactory建設の入力(戦時のAI国家についてだけ構築する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WartimeSupplyContext {
    /// 現在の国家総需要(`country_daily_demand_units`、units/日)。
    pub daily_demand_units: u128,
}

/// Ironの継続的なフロー不足の評価結果(履歴が「継続的な不足」と評価した国家についてだけ構築する)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IronSupplyContext {
    /// 直近の窓の月平均の、MilitaryFactory の Iron 未充足の要求量(Iron/月)。
    pub average_unmet: f64,
}

/// 軍需基盤の再建の入力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MilitaryRebuildContext {
    /// 既存の軍備目標(目標師団数、`calculate_ai_target_divisions`)。
    pub target_divisions: u64,
    /// 現在の戦力(生存師団 + 募兵キュー)。目標より小さい。
    pub current_force: u64,
    /// P29-005 の履歴(連続 4 か月)の、月平均の実産出(units/月)。
    pub average_production_units: u128,
}

/// 国家ごとの軍事状況の入力(既存の判断が実際に参照する値だけ)。旧経路は戦争・軍隊の登録簿から作り、
/// 新経路は検証用シナリオが明示する。新しい脅威評価や戦略は含まない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MilitaryAiFacts {
    /// 国家が現在、有効な戦争状態にあるか(戦時MilitaryFactory建設の前提)。
    pub at_war: bool,
    /// `country_daily_demand_units`の結果(units/日)。算出に失敗した場合は`None`(0とは別)。
    /// RONが128bit整数を扱えないため64bit(1.8e19 units/日を超える値は算出失敗と同じ`None`)。
    pub daily_demand_units: Option<u64>,
    /// 生存している師団の数(`is_living`)。
    pub living_divisions: u64,
    /// 募兵キューの件数。
    pub recruitment_queue_len: u64,
    /// 標準歩兵の師団定義の必要人的資源(定義が無ければ`None`で、再建の需要は認識されない)。
    pub infantry_required_manpower: Option<u64>,
}

// ---------------------------------------------------------------- 文脈の構築(履歴・軍事状況から)

/// 戦時MilitaryFactory建設の文脈。戦時で、需要を算出できたときだけ`Some`。
pub fn wartime_supply_context_from(facts: &MilitaryAiFacts) -> Option<WartimeSupplyContext> {
    if !facts.at_war {
        return None;
    }
    facts
        .daily_demand_units
        .map(|d| WartimeSupplyContext {
            daily_demand_units: u128::from(d),
        })
}

/// 履歴から、Iron不足の文脈を作る。未集計・継続的な不足なしのときは`None`。
/// `mine_levels`は国家が所有する州のMineレベル合計(履歴の各月の値と比べ、完成・喪失が実績に反映済みか判定する)。
pub fn iron_supply_context_from(
    history: &IronShortageHistory,
    now: (i32, u8),
    mine_levels: u32,
) -> Option<IronSupplyContext> {
    match history.assess(now, mine_levels) {
        IronShortageAssessment::Sustained { average_unmet } => {
            Some(IronSupplyContext { average_unmet })
        }
        IronShortageAssessment::Unavailable | IronShortageAssessment::NoSustainedShortage => None,
    }
}

/// 再建需要の認識(戦力・軍需基盤・履歴の条件)。すべて満たすときだけ`Some`。
/// 1. 履歴が判断月を最後とする連続4か月分ある。 2. 所有するMilitaryFactoryのレベル合計が0。
/// 3. 戦力(生存師団 + 募兵キュー)が既存の軍備目標に満たない。
pub fn military_rebuild_context_for<K: Copy, R: AiRegions<K>>(
    regions: &R,
    all_owned: &[K],
    facts: &MilitaryAiFacts,
    history: &SupplyProductionHistory,
    now: (i32, u8),
) -> Option<MilitaryRebuildContext> {
    let average_production_units = history.average_units(now)?;
    let mf_levels: u32 = all_owned
        .iter()
        .map(|k| regions.building_level(*k, BuildingType::MilitaryFactory))
        .sum();
    if mf_levels > 0 {
        return None;
    }
    let required_manpower = facts.infantry_required_manpower?;
    let population: u64 = all_owned
        .iter()
        .filter_map(|k| regions.state(*k))
        .map(|s| s.population())
        .sum();
    let cap = crate::population::growth::compute_manpower_cap(population);
    let target_divisions = crate::military::ai::calculate_ai_target_divisions(cap, required_manpower);
    let current_force = facts.living_divisions + facts.recruitment_queue_len;
    if current_force >= target_divisions {
        return None;
    }
    Some(MilitaryRebuildContext {
        target_divisions,
        current_force,
        average_production_units,
    })
}

/// Factory増設を抑止するか。`entries`は(地域の安定キー, Factoryレベル)でレベル>0のものだけ。
pub fn factory_expansion_blocked_from(
    history: &crate::economy::factory_starvation::FactoryStarvationHistory,
    now: (i32, u8),
    entries: &[(usize, u32)],
) -> bool {
    history.blocks_expansion(
        now,
        crate::economy::factory_starvation::factory_signature(entries),
        entries.len(),
    )
}

// ---------------------------------------------------------------- 共通の数値規則

/// 優先度3と同じ予備資金の要求(P27-002-A): 建設費 + 月次経常支出 × 3 か月分を国庫が満たすこと。
/// 非有限・負の値は安全側(不許可)。
fn peacetime_reserve_satisfied<C: AiCountry>(country: &C, construction_cost: f64) -> bool {
    let treasury = country.treasury();
    let monthly_expenses = country.monthly_expenses();
    let valid = treasury.is_finite()
        && construction_cost.is_finite()
        && construction_cost >= 0.0
        && monthly_expenses.is_finite()
        && monthly_expenses >= 0.0;
    if !valid {
        return false;
    }
    let required = construction_cost + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
    required.is_finite() && treasury >= required
}

/// Mine を 1 レベル追加したときの Iron 産出の見込み(Iron/月)。
fn iron_mine_yield(state: &dyn StateEconomyAccess, fallback_op: f32) -> f64 {
    let deposits: f64 = state
        .resource_deposits()
        .iter()
        .filter(|d: &&StateResourceDeposit| d.discovered && d.resource_type == ResourceType::Iron)
        .map(|d| d.base_output)
        .sum();
    let op = if state.building_level(BuildingType::Mine) > 0 {
        state
            .operation_ratios()
            .get(&BuildingType::Mine)
            .copied()
            .unwrap_or(fallback_op)
    } else {
        fallback_op
    };
    deposits * op.clamp(0.0, 1.0) as f64
}

/// この州のMilitaryFactoryが前月に実際に生産したMilitarySupplies(月産)。
fn realised_monthly_military_supplies(
    state: &dyn StateEconomyAccess,
    def: &BuildingDefinition,
) -> f64 {
    let output = def
        .output_resources
        .get(&ResourceType::MilitarySupplies)
        .copied()
        .unwrap_or(0.0);
    let level = state.building_level(BuildingType::MilitaryFactory);
    output * f64::from(level) * f64::from(state.building_operation_ratio(BuildingType::MilitaryFactory))
}

/// この州にMilitaryFactoryを1レベル追加したときの、MilitarySupplies月産の増加見込み。
/// 上限levelの州、労働力0の州、出力を持たない定義は`None`。
pub fn projected_military_factory_supply_gain_for(
    state: &dyn StateEconomyAccess,
    def: &BuildingDefinition,
    building_registry: &BuildingRegistry,
) -> Option<f64> {
    let output = *def.output_resources.get(&ResourceType::MilitarySupplies)?;
    let level = state.building_level(BuildingType::MilitaryFactory);
    if level >= def.max_level {
        return None;
    }
    let potential_workforce = state.total_workforce();
    if potential_workforce == 0 {
        return None;
    }
    let civilian_workforce = state.civilian_workforce();
    let required_jobs: u64 = BuildingType::ALL
        .iter()
        .filter_map(|&b| {
            let lv = state.building_level(b);
            (lv > 0)
                .then(|| building_registry.get(b))
                .flatten()
                .map(|d| (d.required_workforce as u64) * u64::from(lv))
        })
        .sum();
    let operation = |jobs: u64, logistics_usage: f32| -> f64 {
        let employed = jobs.min(civilian_workforce);
        let workforce_ratio =
            crate::population::update::calculate_economic_workforce_ratio(employed, potential_workforce)
                .clamp(0.0, 1.0);
        let logistics_ratio = crate::logistics::update::calculate_logistics_ratio(
            state.logistics_capacity(),
            logistics_usage,
        );
        f64::from(workforce_ratio.min(logistics_ratio))
    };
    let before = output * f64::from(level) * operation(required_jobs, state.logistics_usage());
    let after = output
        * f64::from(level + 1)
        * operation(
            required_jobs.saturating_add(def.required_workforce as u64),
            state.logistics_usage() + def.logistics_cost,
        );
    Some(after - before)
}

/// MilitaryFactoryを1レベル追加した後の月次入力需要に対し、各入力資源の国家備蓄が1か月分以上あるか。
fn wartime_supply_inputs_covered<C: AiCountry, R: AiRegions<C::Key>>(
    country: &C,
    regions: &R,
    building_registry: &BuildingRegistry,
    def: &BuildingDefinition,
    all_owned: &[C::Key],
) -> bool {
    for resource in ResourceType::ALL {
        let Some(&new_level_need) = def.input_resources.get(&resource) else {
            continue;
        };
        let mut monthly_need = new_level_need;
        for k in all_owned {
            for building in BuildingType::ALL {
                let level = regions.building_level(*k, building);
                if level == 0 {
                    continue;
                }
                if let Some(need) = building_registry
                    .get(building)
                    .and_then(|d| d.input_resources.get(&resource))
                {
                    monthly_need += need * f64::from(level);
                }
            }
        }
        let stock = country.stock(resource);
        if stock.is_nan() || stock < monthly_need {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------- 優先度2.5 / 2.8 / 2.9

/// 優先度2.5(P28-002): 戦時MilitaryFactory建設。着工した場合だけ`true`(国庫・建設キューを変更する)。
/// `false`のときは国庫・建設キューを一切変更しない。条件は旧実装と同一:
/// 1. MilitaryFactoryが建設キューにない 2. 所有州の実月産 < 日次需要 × 30 3. 入力資源の備蓄が1か月分以上
/// 4. `buildable`のうち増産見込みが正で最大の州(同値は先頭=ID昇順) 5. 国庫 ≥ 建設費 + 月次経常支出 × 1か月
pub fn try_wartime_supply_construction<C: AiCountry, R: AiRegions<C::Key>>(
    country: &mut C,
    regions: &R,
    building_registry: &BuildingRegistry,
    all_owned: &[C::Key],
    buildable: &[C::Key],
    context: WartimeSupplyContext,
) -> bool {
    let Some(def) = building_registry.get(BuildingType::MilitaryFactory) else {
        return false;
    };
    if country.queue_has_building(BuildingType::MilitaryFactory) {
        return false;
    }
    let Some(monthly_demand_units) = context.daily_demand_units.checked_mul(30) else {
        return false;
    };
    let realised: f64 = all_owned
        .iter()
        .filter_map(|k| regions.state(*k))
        .map(|s| realised_monthly_military_supplies(s, def))
        .sum();
    let Ok(realised_units) = crate::economy::resources::military_supplies_value_to_units(realised)
    else {
        return false;
    };
    if u128::from(realised_units) >= monthly_demand_units {
        return false;
    }
    if !wartime_supply_inputs_covered(country, regions, building_registry, def, all_owned) {
        return false;
    }

    let mut best: Option<(f64, C::Key, u32)> = None;
    for &key in buildable {
        let Some(state) = regions.state(key) else {
            continue;
        };
        // 完成後の月次経済が入力不備で失敗する経路を塞ぐ前提(新経路のみ。旧経路は常に受理)。
        if regions
            .order_precondition(key, BuildingType::MilitaryFactory)
            .is_err()
        {
            continue;
        }
        let Some(gain) = projected_military_factory_supply_gain_for(state, def, building_registry)
        else {
            continue;
        };
        if gain > 0.0 && best.is_none_or(|(best_gain, _, _)| gain > best_gain) {
            best = Some((gain, key, state.building_level(BuildingType::MilitaryFactory)));
        }
    }
    let Some((_, key, current_level)) = best else {
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
    let required_treasury = cost + monthly_expenses * WARTIME_SUPPLY_CONSTRUCTION_RESERVE_MONTHS;
    if !required_treasury.is_finite() || treasury < required_treasury {
        return false;
    }

    country.set_treasury(treasury - cost);
    country.push_order(
        key,
        BuildingType::MilitaryFactory,
        current_level + 1,
        def.required_progress,
        cost,
    );
    true
}

/// 優先度2.8(P29-009): Ironの継続的な不足を補うMineを1レベル着工する。着工したら`true`。
/// 需要は履歴の月平均の未充足量。供給見込みは候補州のIron鉱床の基本出力 × Mine稼働率。建設中のMineの
/// 増分は差し引く。残りの不足が1レベル分の見込みの半分未満なら着工しない。予算は優先度3と同じ予備資金。
/// 資源入力が未設定の州は候補にしない(他の候補の評価は続ける)。
pub fn try_iron_supply_construction<C: AiCountry, R: AiRegions<C::Key>>(
    country: &mut C,
    regions: &R,
    building_registry: &BuildingRegistry,
    owned: &[C::Key],
    context: IronSupplyContext,
) -> bool {
    let Some(def) = building_registry.get(BuildingType::Mine) else {
        return false;
    };
    if !context.average_unmet.is_finite() || context.average_unmet <= 0.0 {
        return false;
    }
    let (sum, n) = owned
        .iter()
        .filter_map(|k| regions.state(*k))
        .filter(|s| s.building_level(BuildingType::Mine) > 0)
        .filter_map(|s| s.operation_ratios().get(&BuildingType::Mine).copied())
        .fold((0.0f32, 0u32), |(a, c), v| (a + v, c + 1));
    let fallback_op = if n > 0 { sum / n as f32 } else { 1.0 };

    let mut best: Option<(C::Key, f64)> = None;
    for &key in owned {
        if country.queue_has(key, BuildingType::Mine) {
            continue;
        }
        let Some(state) = regions.state(key) else {
            continue;
        };
        if regions.order_precondition(key, BuildingType::Mine).is_err() {
            continue;
        }
        if is_crystal_only_state(state.resource_deposits()) {
            continue;
        }
        if state.building_level(BuildingType::Mine) >= def.max_level {
            continue;
        }
        let y = iron_mine_yield(state, fallback_op);
        if y <= 0.0 {
            continue;
        }
        if best.is_none_or(|(_, by)| y > by) {
            best = Some((key, y));
        }
    }
    let Some((key, yield_per_level)) = best else {
        return false;
    };

    let pending: f64 = country
        .queued_regions(BuildingType::Mine)
        .into_iter()
        .filter_map(|k| regions.state(k))
        .map(|s| iron_mine_yield(s, fallback_op))
        .sum();
    let deficit = context.average_unmet - pending;
    if deficit < 0.5 * yield_per_level {
        return false;
    }
    if !peacetime_reserve_satisfied(country, def.construction_cost) {
        return false;
    }
    let current_level = regions.building_level(key, BuildingType::Mine);
    country.set_treasury(country.treasury() - def.construction_cost);
    country.push_order(
        key,
        BuildingType::Mine,
        current_level + 1,
        def.required_progress,
        def.construction_cost,
    );
    true
}

/// 優先度2.9(P30-002): 軍需基盤の再建: MilitaryFactory lv1 を 1 件着工する。着工したら`true`。
/// 条件は旧実装と同一(キューに無い、増産見込みが正で最大の州、国内供給の見込み、優先度3と同じ予備資金)。
/// `false`のときは国庫・建設キューを一切変更しない。
pub fn try_military_rebuild_construction<C: AiCountry, R: AiRegions<C::Key>>(
    country: &mut C,
    regions: &R,
    building_registry: &BuildingRegistry,
    all_owned: &[C::Key],
    owned: &[C::Key],
    _context: MilitaryRebuildContext,
) -> bool {
    let Some(def) = building_registry.get(BuildingType::MilitaryFactory) else {
        return false;
    };
    if country.queue_has_building(BuildingType::MilitaryFactory) {
        return false;
    }
    let Some(&ms_output) = def.output_resources.get(&ResourceType::MilitarySupplies) else {
        return false;
    };
    if ms_output <= 0.0 {
        return false;
    }

    let mut best: Option<(f64, C::Key)> = None;
    for &key in owned {
        let Some(state) = regions.state(key) else {
            continue;
        };
        if state.building_level(BuildingType::MilitaryFactory) != 0 {
            continue;
        }
        if regions
            .order_precondition(key, BuildingType::MilitaryFactory)
            .is_err()
        {
            continue;
        }
        let Some(gain) = projected_military_factory_supply_gain_for(state, def, building_registry)
        else {
            continue;
        };
        if gain > 0.0 && best.is_none_or(|(g, _)| gain > g) {
            best = Some((gain, key));
        }
    }
    let Some((gain, key)) = best else {
        return false;
    };
    let projected_op = (gain / ms_output).clamp(0.0, 1.0);

    let mut iron_supply = 0.0f64;
    let mut ig_supply = 0.0f64;
    let factory_ig_output = building_registry
        .get(BuildingType::Factory)
        .and_then(|d| d.output_resources.get(&ResourceType::IndustrialGoods))
        .copied()
        .unwrap_or(0.0);
    for state in all_owned.iter().filter_map(|k| regions.state(*k)) {
        let mine_level = state.building_level(BuildingType::Mine);
        if mine_level > 0 {
            let op = state
                .operation_ratios()
                .get(&BuildingType::Mine)
                .copied()
                .unwrap_or(0.0);
            let deposits: f64 = state
                .resource_deposits()
                .iter()
                .filter(|d| d.discovered && d.resource_type == ResourceType::Iron)
                .map(|d| d.base_output)
                .sum();
            iron_supply += deposits * f64::from(mine_level) * f64::from(op.clamp(0.0, 1.0));
        }
        let factory_level = state.building_level(BuildingType::Factory);
        if factory_level > 0 {
            let op = state
                .operation_ratios()
                .get(&BuildingType::Factory)
                .copied()
                .unwrap_or(0.0);
            ig_supply += factory_ig_output * f64::from(factory_level) * f64::from(op.clamp(0.0, 1.0));
        }
    }
    for (&resource, &base_input) in &def.input_resources {
        let need = base_input * projected_op;
        let supply = match resource {
            ResourceType::Iron => iron_supply,
            ResourceType::IndustrialGoods => ig_supply,
            _ => return false,
        };
        if supply.is_nan() || supply < need || !supply.is_finite() {
            return false;
        }
    }

    if !peacetime_reserve_satisfied(country, def.construction_cost) {
        return false;
    }
    country.set_treasury(country.treasury() - def.construction_cost);
    country.push_order(
        key,
        BuildingType::MilitaryFactory,
        1,
        def.required_progress,
        def.construction_cost,
    );
    true
}

// ---------------------------------------------------------------- 差し込み(旧経路・新経路で共通)

/// 軍事に関わる優先度のうち、どれが着工したか(検証・表示用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilitaryPriority {
    /// 優先度2.5
    WartimeSupply,
    /// 優先度2.8
    IronSupply,
    /// 優先度2.9
    MilitaryRebuild,
}

/// `ConstructionHooks`の共通実装。文脈が`None`の優先度は評価しない(従来と同じ)。各文脈は1回の判断で
/// 最大1回だけ使う(`take`)。着工した優先度を`fired`に残す。
pub struct MilitaryHooks<'a, C: AiCountry, R> {
    pub regions: &'a R,
    pub building_registry: &'a BuildingRegistry,
    /// 国家が所有する全地域(昇順)。供給の実績・備蓄需要の集計に使う。
    pub all_owned: &'a [C::Key],
    /// 建設の候補にできる地域(所有かつ実効支配する陸上地域、昇順)。
    pub owned: &'a [C::Key],
    pub wartime_supply: Option<WartimeSupplyContext>,
    pub iron_supply: Option<IronSupplyContext>,
    pub military_rebuild: Option<MilitaryRebuildContext>,
    pub fired: Option<MilitaryPriority>,
}

impl<C: AiCountry, R: AiRegions<C::Key>> crate::country::ai_core::ConstructionHooks<C>
    for MilitaryHooks<'_, C, R>
{
    fn wartime_supply(&mut self, country: &mut C) -> bool {
        let Some(context) = self.wartime_supply.take() else {
            return false;
        };
        let built = try_wartime_supply_construction(
            country,
            self.regions,
            self.building_registry,
            self.all_owned,
            self.owned,
            context,
        );
        if built {
            self.fired = Some(MilitaryPriority::WartimeSupply);
        }
        built
    }
    fn iron_supply(&mut self, country: &mut C) -> bool {
        let Some(context) = self.iron_supply.take() else {
            return false;
        };
        let built = try_iron_supply_construction(
            country,
            self.regions,
            self.building_registry,
            self.owned,
            context,
        );
        if built {
            self.fired = Some(MilitaryPriority::IronSupply);
        }
        built
    }
    fn military_rebuild(&mut self, country: &mut C) -> bool {
        let Some(context) = self.military_rebuild.take() else {
            return false;
        };
        let built = try_military_rebuild_construction(
            country,
            self.regions,
            self.building_registry,
            self.all_owned,
            self.owned,
            context,
        );
        if built {
            self.fired = Some(MilitaryPriority::MilitaryRebuild);
        }
        built
    }
}
