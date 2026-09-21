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
use crate::common::{CountryId, DivisionDefinitionId, DivisionId, StateId};
use crate::country::{CountryData, CountryRegistry};
use crate::economy::resources::{
    MilitarySuppliesUnitsError, ResourceType, military_supplies_units_to_value,
    military_supplies_value_to_units,
};
use crate::military::capital_connectivity::connected_states_from_capital;
use crate::military::data::{Division, DivisionDefinition, DivisionStatus, MilitaryRegistry};
use crate::military::supply_initialization::{
    SupplyDemandError, country_daily_demand_units, division_daily_demand_units, is_living,
};
use crate::state::data::StateRegistry;
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
    /// `D_c = Σ D_i`(首都連結性に関わらず、生存する全Divisionの需要合計)。
    /// P26-004-PRE-005の`country_daily_demand_units`と常に一致する
    /// (`debug_assert_eq!`で突き合わせる、既存の不変条件をP26-006でも維持する)。
    pub total_demand_units: u128,
    /// P26-006: `eligible_demand_c = Σ D_i`(首都と陸路で連結しているDivisionのみ)。
    /// 国家備蓄からの実消費・Hamilton配分は、この値だけを基準にする
    /// (spec §10)。全Divisionが連結している通常時は`total_demand_units`と一致する。
    pub eligible_demand_units: u128,
    /// `C_c = min(eligible_demand_c, A_c)`(P26-006で連結Divisionの需要だけを基準に
    /// 変更、切断Divisionの需要は国家備蓄から一切消費しない、spec §10)。
    pub consumed_units: u128,
    /// `DivisionId`昇順。国家が保有する生存Division(`is_living`、P26-004-PRE-005と
    /// 同じ判定式)全員を含む――manpower 0・DestroyedのDivisionはここに現れない
    /// (需要0として自然に除外される、spec §12)。P26-006により、連結Division
    /// (Hamilton配分・需要0なら1.0)と切断Division(配分0、需要正なら0.0・需要0なら
    /// 1.0、spec §11)の両方をこの1つのリストへ統合する。
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

// ─── P27-005: 攻撃側前線補給ブリッジ ─────────────────────────────────────────

/// 通常の首都連結補給を受けられない攻撃側戦闘Divisionが、1ホップの前線補給を受ける
/// 資格を持つかを判定する副作用のない関数(P27-005-PRE-001 候補1)。
///
/// 次を**すべて**満たす場合だけ`true`:
/// 1. 現在州が通常の首都連結集合(`connected`、controllerベース、無変更)に含まれない
/// 2. 現在州が存在し、controllerが所属国以外(敵対側controller)
/// 3. 進行中(`Ongoing`)の戦闘の`attacker_division_ids`に登録され、その戦闘の
///    `attacker_country`が所属国で、戦闘州が現在州と一致する(攻撃側の真実源は
///    `Battle`であり、Moving・stance・戦争中・destinationからは推定しない)
/// 4. 現在州に隣接する州のうち、controllerが所属国**かつ**`connected`に含まれる州が
///    1つ以上ある
///
/// `connected`だけを供給元の真実源とするため、前線補給を受ける敵州は別の判定の
/// 供給元にならず(連鎖投射なし)、回廊遮断・首都陥落・包囲では隣接州が`connected`から
/// 外れて自然に`false`になる。存在しない州・戦闘参照は`false`(panicしない)。
pub fn is_frontline_supply_eligible(
    division: &Division,
    connected: &HashSet<StateId>,
    state_registry: &StateRegistry,
    battle_registry: &crate::military::battle::BattleRegistry,
) -> bool {
    if connected.contains(&division.current_state) {
        return false;
    }
    let Some(state) = state_registry.get(division.current_state) else {
        return false;
    };
    if state.controller() == division.owner {
        return false;
    }
    let is_attacker_in_ongoing_battle = battle_registry.battles.values().any(|b| {
        b.status == crate::military::battle::BattleStatus::Ongoing
            && b.attacker_country == division.owner
            && b.state_id == division.current_state
            && b.attacker_division_ids.contains(&division.id)
    });
    if !is_attacker_in_ongoing_battle {
        return false;
    }
    state.neighbors.iter().any(|n| {
        connected.contains(n)
            && state_registry.get(*n).is_some_and(|s| s.controller() == division.owner)
    })
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
///
/// `connected_states`(P26-006、`capital_connectivity::connected_states_from_capital`が
/// 前日終値のcontroller()から算出した集合)に`division.current_state`が含まれる
/// Divisionだけが「連結」として国家備蓄からの実消費・Hamilton配分の対象になる
/// (spec §10)。含まれないDivisionは「切断」として配分0を受け、需要が正なら
/// 補給率0.0・需要0なら補給率1.0になる(spec §11、国家備蓄からは一切消費しない)。
pub fn plan_country_daily_supply(
    country_id: CountryId,
    current_stockpile_value: f64,
    divisions: &HashMap<DivisionId, Division>,
    definitions: &HashMap<DivisionDefinitionId, DivisionDefinition>,
    connected_states: &HashSet<StateId>,
) -> Result<CountrySupplyPlan, DailySupplyPlanError> {
    plan_country_daily_supply_with_frontline(
        country_id,
        current_stockpile_value,
        divisions,
        definitions,
        connected_states,
        &HashSet::new(),
    )
}

/// P27-005: 攻撃側前線補給ブリッジ対応版。`frontline_eligible`は
/// `is_frontline_supply_eligible`が真になったDivisionIdの集合で、通常の首都連結
/// 集合(`connected_states`、無変更)から外れたDivisionのうち、この集合に含まれる
/// ものだけを既存の`eligible`(需要1回計上・Hamilton配分・備蓄消費の対象)へ加える。
/// 前線補給対象の州を連結集合へ挿入しない(連鎖投射の防止)。
pub fn plan_country_daily_supply_with_frontline(
    country_id: CountryId,
    current_stockpile_value: f64,
    divisions: &HashMap<DivisionId, Division>,
    definitions: &HashMap<DivisionDefinitionId, DivisionDefinition>,
    connected_states: &HashSet<StateId>,
    frontline_eligible: &HashSet<DivisionId>,
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
        total_demand_units =
            total_demand_units.checked_add(demand).ok_or(DailySupplyPlanError::Overflow)?;
        per_division.push((*id, demand));
    }

    // P26-004-PRE-005の正規合計関数と厳密に一致することを実測で突き合わせる
    // (同じis_livingフィルタ・同じDivisionId昇順・同じdivision_daily_demand_unitsの
    // 呼び出しから導かれる合計であるため、数学的に常に一致するはずの不変条件)。
    // P26-006: この総需要は首都連結性に関わらない値のままであり続ける(P26-006が
    // 変更するのは「そのうちどれだけを実際に配分・消費するか」だけである)。
    debug_assert_eq!(
        Ok(total_demand_units),
        country_daily_demand_units(country_id, divisions, definitions),
        "daily_supply's own per-division aggregation must exactly match the canonical \
         country_daily_demand_units total (P26-004-PRE-005) — any mismatch is a bug, not a \
         legitimate error state"
    );

    // P26-006(spec §10): 連結/切断への分類。DivisionId昇順を維持したまま分割する。
    let mut eligible: Vec<(DivisionId, u128)> = Vec::with_capacity(per_division.len());
    let mut disconnected: Vec<(DivisionId, u128)> = Vec::new();
    for &(id, demand) in &per_division {
        let division = &divisions[&id];
        if connected_states.contains(&division.current_state) || frontline_eligible.contains(&id) {
            eligible.push((id, demand));
        } else {
            disconnected.push((id, demand));
        }
    }
    let eligible_demand_units: u128 = eligible
        .iter()
        .try_fold(0u128, |acc, &(_, demand)| acc.checked_add(demand))
        .ok_or(DailySupplyPlanError::Overflow)?;

    let current_units = military_supplies_value_to_units(current_stockpile_value)
        .map_err(|_| DailySupplyPlanError::InvalidCurrentStockpile)?;

    let consumed_units = eligible_demand_units.min(current_units as u128);

    let mut allocations = if eligible_demand_units == 0 || consumed_units == eligible_demand_units {
        // 連結Divisionの需要0、または備蓄が連結Divisionの総需要以上
        // (spec §7/§10: 常に連結Divisionの需要全量を配分する)。
        eligible
            .iter()
            .map(|&(id, demand)| DivisionSupplyAllocation {
                division_id: id,
                demand_units: demand,
                allocated_units: demand,
                fulfillment: 1.0,
            })
            .collect()
    } else {
        allocate_by_largest_remainder(consumed_units, eligible_demand_units, &eligible)?
    };

    // P26-006(spec §11): 切断Divisionは配分0。需要が正なら補給率0.0、需要0なら
    // 「要求がない状態は完全充足として扱う」という既存契約(spec §11・P26-004 §8)を
    // 維持して補給率1.0とする。国家備蓄への影響は一切ない(このループは
    // `consumed_units`計算より後にあり、`disconnected`の需要をどこにも加算しない)。
    for &(id, demand) in &disconnected {
        let fulfillment = if demand == 0 { 1.0 } else { 0.0 };
        allocations.push(DivisionSupplyAllocation {
            division_id: id,
            demand_units: demand,
            allocated_units: 0,
            fulfillment,
        });
    }
    allocations.sort_by_key(|a| a.division_id.0);

    Ok(CountrySupplyPlan {
        country_id,
        total_demand_units,
        eligible_demand_units,
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
        let numerator = consumed_units.checked_mul(demand).ok_or(DailySupplyPlanError::Overflow)?;
        let base = numerator / total_demand_units;
        let remainder = numerator % total_demand_units;
        base_sum = base_sum.checked_add(base).ok_or(DailySupplyPlanError::Overflow)?;
        items.push(Item { id, demand, base, remainder });
    }

    // R = C_c - Σbase_i。標準的なHamilton法の数学的性質により常に
    // `0 <= R < (需要が正のDivision数)`が成り立つ(このモジュールのテスト
    // `allocation_invariants_hold_under_random_scenarios`等で実測検証する)。
    let leftover = consumed_units.checked_sub(base_sum).ok_or(DailySupplyPlanError::Overflow)?;
    let leftover_usize = usize::try_from(leftover).map_err(|_| DailySupplyPlanError::Overflow)?;

    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        items[b].remainder.cmp(&items[a].remainder).then(items[a].id.0.cmp(&items[b].id.0))
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

    let considered: HashSet<DivisionId> = plan.allocations.iter().map(|a| a.division_id).collect();
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
///
/// P26-006: 各国家の首都連結州集合(`state_registry`が保持する前日終値の
/// `controller()`から算出)を需要算定より先に求め、`plan_country_daily_supply`へ
/// 渡す(spec §10・§14)。
pub fn run_daily_military_supply_consumption(
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
    state_registry: &StateRegistry,
) {
    run_daily_military_supply_consumption_with_battles(
        country_registry,
        military_registry,
        state_registry,
        &crate::military::battle::BattleRegistry::default(),
    );
}

/// P27-005: 戦闘情報を渡す版。攻撃側前線補給ブリッジ(`is_frontline_supply_eligible`)を
/// 適用する。戦闘のない既存呼び出しは上の薄いラッパーで従来と同一の挙動になる。
pub fn run_daily_military_supply_consumption_with_battles(
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
    state_registry: &StateRegistry,
    battle_registry: &crate::military::battle::BattleRegistry,
) {
    let mut country_ids: Vec<CountryId> = country_registry.countries.iter().map(|c| c.id).collect();
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

        // P26-006(spec §7): 首都が存在しない・海上州・敵支配等でtraversableでない
        // 場合、`connected_states_from_capital`は空集合を返す――このとき正の需要を
        // 持つ全Divisionは下の`plan_country_daily_supply`によって補給率0として
        // 扱われる。`current_capital_state_id()`は`is_active()`を満たす国家では
        // 常に`Some`だが、将来の変更に対して安全側(空集合)に倒す。
        let connected_states = match country.current_capital_state_id() {
            Some(capital) => connected_states_from_capital(capital, country_id, state_registry),
            None => HashSet::new(),
        };

        let frontline_eligible: HashSet<DivisionId> = military_registry
            .divisions
            .values()
            .filter(|d| d.owner == country_id && is_living(d))
            .filter(|d| {
                is_frontline_supply_eligible(d, &connected_states, state_registry, battle_registry)
            })
            .map(|d| d.id)
            .collect();

        let plan = match plan_country_daily_supply_with_frontline(
            country_id,
            stockpile_value,
            &military_registry.divisions,
            &military_registry.definitions,
            &connected_states,
            &frontline_eligible,
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
        if let Err(err) = apply_country_daily_supply_plan(country_mut, military_registry, &plan) {
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
    state_registry: Res<StateRegistry>,
    battle_registry: Res<crate::military::battle::BattleRegistry>,
) {
    for _event in day_events.read() {
        run_daily_military_supply_consumption_with_battles(
            &mut country_registry,
            &mut military_registry,
            &state_registry,
            &battle_registry,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::military::data::{DivisionSize, DivisionType};
    use crate::state::data::StateData;

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
        CountryData { id: CountryId(id), ..CountryData::default() }
    }

    /// P26-006以前の全既存テストが暗黙に前提としていた「全Divisionが常に連結」
    /// 状態を再現するヘルパー。このモジュールの`division()`フィクスチャは常に
    /// `current_state: StateId(0)`を使うため、`{StateId(0)}`を渡せば
    /// P26-006導入前と数学的に同一の(`eligible_demand_units == total_demand_units`
    /// となる)計画になる。
    fn connected_all() -> HashSet<StateId> {
        [StateId(0)].into_iter().collect()
    }

    // ─── A. 需要(reuse, not duplicated) ──────────────────────────────

    #[test]
    fn plan_reuses_the_canonical_demand_function_for_a_single_full_strength_infantry() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let plan = plan_country_daily_supply(
            CountryId(0),
            100.0,
            &divisions,
            &definitions,
            &connected_all(),
        )
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
        let expected = country_daily_demand_units(CountryId(0), &divisions, &definitions).unwrap();
        let plan = plan_country_daily_supply(
            CountryId(0),
            100.0,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap();
        assert_eq!(plan.total_demand_units, expected);
    }

    #[test]
    fn half_manpower_division_demands_half() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 5_000, 10_000)]);
        let plan = plan_country_daily_supply(
            CountryId(0),
            100.0,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap();
        assert_eq!(plan.total_demand_units, 500_000);
    }

    #[test]
    fn zero_manpower_division_is_excluded_from_demand_and_gets_full_fulfillment_on_apply() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 0, 10_000)]);
        let plan = plan_country_daily_supply(
            CountryId(0),
            100.0,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap();
        assert_eq!(plan.total_demand_units, 0);
        assert!(plan.allocations.is_empty(), "manpower 0 division is not a living division");

        let mut country = test_country(0);
        country.stockpile.set(ResourceType::MilitarySupplies, 100.0);
        let mut military_registry =
            MilitaryRegistry::from_saved_parts(definitions, divisions, 1, HashMap::new());
        apply_country_daily_supply_plan(&mut country, &mut military_registry, &plan).unwrap();
        assert_eq!(military_registry.supply_fulfillment_of(DivisionId(0)), 1.0);
    }

    #[test]
    fn unknown_definition_propagates_as_an_explicit_demand_error() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 99, 10_000, 10_000)]);
        let err = plan_country_daily_supply(
            CountryId(0),
            100.0,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap_err();
        assert!(matches!(err, DailySupplyPlanError::Demand(_)));
    }

    #[test]
    fn invalid_manpower_exceeding_max_propagates_as_an_explicit_error() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 11_000, 10_000)]);
        let err = plan_country_daily_supply(
            CountryId(0),
            100.0,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap_err();
        assert!(matches!(err, DailySupplyPlanError::Demand(_)));
    }

    // ─── B. 備蓄十分 ───────────────────────────────────────────────────

    #[test]
    fn sufficient_stockpile_allocates_full_demand_to_every_division() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions =
            divs(vec![division(0, 0, 0, 10_000, 10_000), division(1, 0, 0, 10_000, 10_000)]);
        let plan = plan_country_daily_supply(
            CountryId(0),
            100.0,
            &divisions,
            &definitions,
            &connected_all(),
        )
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
        let plan = plan_country_daily_supply(
            CountryId(0),
            1.0,
            &divisions,
            &definitions,
            &connected_all(),
        )
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
        let plan = plan_country_daily_supply(
            CountryId(0),
            42.0,
            &divisions,
            &definitions,
            &connected_all(),
        )
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
        let plan = plan_country_daily_supply(
            CountryId(0),
            0.0,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap();
        assert_eq!(plan.consumed_units, 0);
        assert_eq!(plan.allocations[0].allocated_units, 0);
        assert_eq!(plan.allocations[0].fulfillment, 0.0);
    }

    #[test]
    fn fifty_percent_stockpile_allocates_half_to_a_single_division() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let plan = plan_country_daily_supply(
            CountryId(0),
            0.5,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap();
        assert_eq!(plan.consumed_units, 500_000);
        assert_eq!(plan.allocations[0].allocated_units, 500_000);
        assert_eq!(plan.allocations[0].fulfillment, 0.5);
    }

    #[test]
    fn one_unit_short_of_two_equal_divisions_gives_the_extra_unit_to_the_lower_division_id() {
        // demand=2, stockpile value corresponds to consumed_units = 2*1_000_000 - 1.
        let definitions = defs(vec![infantry_def(0)]);
        let divisions =
            divs(vec![division(0, 0, 0, 10_000, 10_000), division(1, 0, 0, 10_000, 10_000)]);
        let consumed_units = 2_000_000u128 - 1;
        let stockpile_value = military_supplies_units_to_value(consumed_units as u64);
        let plan = plan_country_daily_supply(
            CountryId(0),
            stockpile_value,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap();
        assert_eq!(plan.consumed_units, consumed_units);
        let sum: u128 = plan.allocations.iter().map(|a| a.allocated_units).sum();
        assert_eq!(sum, consumed_units);
        // Base = floor(1_999_999 * 1_000_000 / 2_000_000) = 999_999 for each (remainder
        // 500_000 each — tied), so R=1 goes to the lower DivisionId (id 0).
        let by_id: HashMap<DivisionId, u128> =
            plan.allocations.iter().map(|a| (a.division_id, a.allocated_units)).collect();
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
        let plan = plan_country_daily_supply(
            CountryId(0),
            stockpile_value,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap();
        assert_eq!(plan.consumed_units, 1);
        let sum: u128 = plan.allocations.iter().map(|a| a.allocated_units).sum();
        assert_eq!(sum, 1);
        let by_id: HashMap<DivisionId, u128> =
            plan.allocations.iter().map(|a| (a.division_id, a.allocated_units)).collect();
        assert_eq!(by_id[&DivisionId(0)], 1);
        assert_eq!(by_id[&DivisionId(1)], 0);
        assert_eq!(by_id[&DivisionId(2)], 0);
    }

    #[test]
    fn unequal_demand_largest_remainder_allocation_matches_hand_computed_expectation() {
        // Divisions with demand 1_000_000 (infantry) and 3_000_000 (artillery),
        // total demand = 4_000_000; stockpile = 2_500_000 units (0.625 value).
        let definitions = defs(vec![infantry_def(0), artillery_def(1)]);
        let divisions =
            divs(vec![division(0, 0, 0, 10_000, 10_000), division(1, 0, 1, 10_000, 10_000)]);
        let stockpile_value = military_supplies_units_to_value(2_500_000);
        let plan = plan_country_daily_supply(
            CountryId(0),
            stockpile_value,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap();
        assert_eq!(plan.consumed_units, 2_500_000);
        // base_0 = floor(2_500_000*1_000_000/4_000_000) = 625_000, remainder 0
        // base_1 = floor(2_500_000*3_000_000/4_000_000) = 1_875_000, remainder 0
        // sum = 2_500_000 = consumed, R = 0 exactly (exact division case).
        let by_id: HashMap<DivisionId, u128> =
            plan.allocations.iter().map(|a| (a.division_id, a.allocated_units)).collect();
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
            let plan = plan_country_daily_supply(
                CountryId(0),
                value,
                &divisions,
                &definitions,
                &connected_all(),
            )
            .unwrap();
            let sum: u128 = plan.allocations.iter().map(|a| a.allocated_units).sum();
            assert_eq!(sum, plan.consumed_units, "allocation sum must equal consumed units");
            for a in &plan.allocations {
                assert!(a.allocated_units <= a.demand_units, "allocation must never exceed demand");
                assert!((0.0..=1.0).contains(&a.fulfillment));
            }
        }
    }

    // ─── D. 原子性 ─────────────────────────────────────────────────────

    #[test]
    fn invalid_current_stockpile_is_an_explicit_planning_error() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let err = plan_country_daily_supply(
            CountryId(0),
            f64::NAN,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap_err();
        assert_eq!(err, DailySupplyPlanError::InvalidCurrentStockpile);
    }

    #[test]
    fn non_finite_stockpile_is_rejected_without_a_plan() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let err = plan_country_daily_supply(
            CountryId(0),
            f64::INFINITY,
            &divisions,
            &definitions,
            &connected_all(),
        )
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
            eligible_demand_units: 1_000_000,
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
        let err = apply_country_daily_supply_plan(&mut country, &mut military_registry, &bad_plan)
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
            eligible_demand_units: 1_000_000,
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
        let err = plan_country_daily_supply(
            CountryId(0),
            10.0,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap_err();
        assert!(matches!(err, DailySupplyPlanError::Demand(_)));
    }

    #[test]
    fn one_countrys_planning_failure_does_not_disturb_another_countrys_processing() {
        let definitions = defs(vec![infantry_def(0)]);
        let mut division_1 = division(1, 1, 0, 10_000, 10_000); // country 1: healthy
        division_1.current_state = StateId(1); // P26-006: country 1's own state, not country 0's
        let divisions = divs(vec![
            division(0, 0, 0, 11_000, 10_000), // country 0: corrupt (manpower>max)
            division_1,
        ]);
        // P26-006: 各国家が自国の首都を実効支配している最小のStateRegistry
        // (country 0の破損検証自体は接続性と無関係だが、country 1が正常に
        // 処理されるためには自国の首都と繋がっている必要がある)。
        let state_registry = StateRegistry::build(vec![
            StateData { id: StateId(0), owner_country_id: CountryId(0), ..Default::default() },
            StateData { id: StateId(1), owner_country_id: CountryId(1), ..Default::default() },
        ]);
        let mut country_registry = CountryRegistry {
            countries: vec![
                CountryData { capital_state_id: StateId(0), ..test_country(0) },
                CountryData { capital_state_id: StateId(1), ..test_country(1) },
            ],
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

        run_daily_military_supply_consumption(
            &mut country_registry,
            &mut military_registry,
            &state_registry,
        );

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
            &connected_all(),
        )
        .unwrap();
        let plan_b = plan_country_daily_supply(
            CountryId(0),
            stockpile_value,
            &divs(list_b),
            &definitions,
            &connected_all(),
        )
        .unwrap();
        assert_eq!(plan_a, plan_b);
    }

    #[test]
    fn country_insertion_order_does_not_change_the_outcome() {
        let definitions = defs(vec![infantry_def(0)]);
        let mut division_1 = division(1, 1, 0, 10_000, 10_000);
        division_1.current_state = StateId(1); // P26-006: country 1's own state
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000), division_1]);
        // P26-006: 両国とも自国の首都を実効支配している最小のStateRegistry
        // (国家処理順序を変えても、connected_states_from_capitalの入力自体は
        // country registryの反復順に一切依存しないため、この決定性テストの
        // 前提は影響を受けない)。
        let state_registry = StateRegistry::build(vec![
            StateData { id: StateId(0), owner_country_id: CountryId(0), ..Default::default() },
            StateData { id: StateId(1), owner_country_id: CountryId(1), ..Default::default() },
        ]);
        let mut order_a = CountryRegistry {
            countries: vec![
                CountryData { capital_state_id: StateId(0), ..test_country(0) },
                CountryData { capital_state_id: StateId(1), ..test_country(1) },
            ],
        };
        let mut order_b = CountryRegistry {
            countries: vec![
                CountryData { capital_state_id: StateId(1), ..test_country(1) },
                CountryData { capital_state_id: StateId(0), ..test_country(0) },
            ],
        };
        for reg in [&mut order_a, &mut order_b] {
            for id in [0usize, 1] {
                reg.get_mut(CountryId(id))
                    .unwrap()
                    .stockpile
                    .set(ResourceType::MilitarySupplies, 0.5);
            }
        }
        let mut mil_a = MilitaryRegistry::from_saved_parts(
            definitions.clone(),
            divisions.clone(),
            2,
            HashMap::new(),
        );
        let mut mil_b =
            MilitaryRegistry::from_saved_parts(definitions, divisions, 2, HashMap::new());

        run_daily_military_supply_consumption(&mut order_a, &mut mil_a, &state_registry);
        run_daily_military_supply_consumption(&mut order_b, &mut mil_b, &state_registry);

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
            plan_country_daily_supply(
                CountryId(0),
                stockpile_value,
                &divisions,
                &definitions,
                &connected_all(),
            )
            .unwrap()
        }
        let a = run();
        let b = run();
        assert_eq!(a, b);
    }

    // ─── P26-006: 首都連結性による分類(spec §10・§11・§21 Category D) ────────

    /// `division()`と同じ規格だが、`current_state`を指定できる版
    /// (P26-006の接続/切断分類テスト専用)。
    fn division_at(
        div_id: usize,
        owner: usize,
        def_id: usize,
        manpower: u64,
        max_manpower: u64,
        state: usize,
    ) -> Division {
        let mut d = division(div_id, owner, def_id, manpower, max_manpower);
        d.current_state = StateId(state);
        d
    }

    #[test]
    fn p26_006_all_divisions_connected_matches_pre_p26_006_result_exactly() {
        // connected_all()(={StateId(0)})は全Divisionのcurrent_stateと一致するため、
        // eligible_demand_units == total_demand_unitsとなり、P26-004時点と数学的に
        // 同一の計画になる(既存の24件がそのまま無変更で合格していることの直接証拠、
        // ここでは明示的にeligible==totalを確認する)。
        let definitions = defs(vec![infantry_def(0)]);
        let divisions =
            divs(vec![division(0, 0, 0, 10_000, 10_000), division(1, 0, 0, 5_000, 10_000)]);
        let plan = plan_country_daily_supply(
            CountryId(0),
            0.5,
            &divisions,
            &definitions,
            &connected_all(),
        )
        .unwrap();
        assert_eq!(plan.eligible_demand_units, plan.total_demand_units);
    }

    #[test]
    fn p26_006_disconnected_division_with_positive_demand_gets_zero_allocation_and_zero_fulfillment()
     {
        let definitions = defs(vec![infantry_def(0)]);
        // division 0 は連結州(0)、division 1 は切断州(99)
        let divisions = divs(vec![
            division_at(0, 0, 0, 10_000, 10_000, 0),
            division_at(1, 0, 0, 10_000, 10_000, 99),
        ]);
        let connected: HashSet<StateId> = [StateId(0)].into_iter().collect();
        let plan =
            plan_country_daily_supply(CountryId(0), 100.0, &divisions, &definitions, &connected)
                .unwrap();

        let d0 = plan.allocations.iter().find(|a| a.division_id == DivisionId(0)).unwrap();
        let d1 = plan.allocations.iter().find(|a| a.division_id == DivisionId(1)).unwrap();
        assert_eq!(d0.fulfillment, 1.0, "接続Divisionは通常どおり満タン充足のはず");
        assert_eq!(d1.allocated_units, 0, "切断Divisionへの配分は常に0のはず");
        assert_eq!(d1.fulfillment, 0.0, "切断・正需要Divisionの補給率は0.0のはず");
    }

    #[test]
    fn p26_006_all_divisions_disconnected_consumes_nothing_from_stockpile() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division_at(0, 0, 0, 10_000, 10_000, 99)]);
        let connected: HashSet<StateId> = [StateId(0)].into_iter().collect(); // division 0(state 99)は含まれない
        let plan =
            plan_country_daily_supply(CountryId(0), 100.0, &divisions, &definitions, &connected)
                .unwrap();
        assert_eq!(plan.eligible_demand_units, 0);
        assert_eq!(plan.consumed_units, 0, "切断Divisionの需要は国家備蓄から一切消費しないはず");
        assert_eq!(plan.allocations[0].fulfillment, 0.0);
        // total_demand_unitsは接続性に関わらず正のまま(P26-004-PRE-005の正規合計)。
        assert!(plan.total_demand_units > 0);
    }

    #[test]
    fn p26_006_disconnected_division_with_zero_demand_gets_full_fulfillment() {
        // manpower 0 は is_living で除外されるため、「需要0の切断Division」は
        // 生存(manpower>0)かつsupply_usage=0.0のDefinitionを使って作る。
        let mut zero_usage_def = infantry_def(0);
        zero_usage_def.supply_usage = 0.0;
        let definitions = defs(vec![zero_usage_def]);
        let divisions = divs(vec![division_at(0, 0, 0, 10_000, 10_000, 99)]);
        let connected: HashSet<StateId> = [StateId(0)].into_iter().collect();
        let plan =
            plan_country_daily_supply(CountryId(0), 100.0, &divisions, &definitions, &connected)
                .unwrap();
        assert_eq!(plan.allocations[0].demand_units, 0);
        assert_eq!(plan.allocations[0].allocated_units, 0);
        assert_eq!(
            plan.allocations[0].fulfillment, 1.0,
            "切断・需要0のDivisionは「要求がない状態は完全充足」の既存契約どおり1.0のはず"
        );
    }

    #[test]
    fn p26_006_shortage_is_computed_against_connected_demand_only_not_total_demand() {
        // 接続Division 1体(需要1.0)+切断Division 1体(需要1.0、需要は多いが対象外)。
        // 備蓄0.6は接続分の需要(1.0)には足りないが、総需要(2.0)よりは少ない
        // ――もし誤って総需要を基準にすると備蓄不足の判定自体が変わってしまう。
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division_at(0, 0, 0, 10_000, 10_000, 0),  // 接続
            division_at(1, 0, 0, 10_000, 10_000, 99), // 切断
        ]);
        let connected: HashSet<StateId> = [StateId(0)].into_iter().collect();
        let stockpile_value = military_supplies_units_to_value(600_000); // 0.6 units
        let plan = plan_country_daily_supply(
            CountryId(0),
            stockpile_value,
            &divisions,
            &definitions,
            &connected,
        )
        .unwrap();
        assert_eq!(plan.eligible_demand_units, 1_000_000);
        assert_eq!(plan.total_demand_units, 2_000_000);
        assert_eq!(plan.consumed_units, 600_000, "消費量は接続分の需要だけを基準にするはず");
        let d0 = plan.allocations.iter().find(|a| a.division_id == DivisionId(0)).unwrap();
        assert_eq!(d0.fulfillment, 0.6);
    }

    #[test]
    fn p26_006_disconnected_divisions_are_excluded_from_the_hamilton_denominator() {
        // 接続2体(需要1.0ずつ)+切断1体(需要1.0)。備蓄1.0(接続分の需要2.0の半分)を
        // 接続2体だけで50%ずつ均等配分するはず――切断分の需要が分母(D_c)へ
        // 紛れ込んでいたら按分比率が変わってしまう。
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division_at(0, 0, 0, 10_000, 10_000, 0),
            division_at(1, 0, 0, 10_000, 10_000, 0),
            division_at(2, 0, 0, 10_000, 10_000, 99),
        ]);
        let connected: HashSet<StateId> = [StateId(0)].into_iter().collect();
        let stockpile_value = military_supplies_units_to_value(1_000_000); // 1.0 units
        let plan = plan_country_daily_supply(
            CountryId(0),
            stockpile_value,
            &divisions,
            &definitions,
            &connected,
        )
        .unwrap();
        let d0 = plan.allocations.iter().find(|a| a.division_id == DivisionId(0)).unwrap();
        let d1 = plan.allocations.iter().find(|a| a.division_id == DivisionId(1)).unwrap();
        let d2 = plan.allocations.iter().find(|a| a.division_id == DivisionId(2)).unwrap();
        assert_eq!(d0.allocated_units, 500_000);
        assert_eq!(d1.allocated_units, 500_000);
        assert_eq!(d2.allocated_units, 0);
        assert_eq!(d0.fulfillment, 0.5);
        assert_eq!(d1.fulfillment, 0.5);
        assert_eq!(d2.fulfillment, 0.0);
    }

    #[test]
    fn p26_006_sum_of_connected_allocations_equals_consumed_units_disconnected_stays_out() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division_at(0, 0, 0, 10_000, 10_000, 0),
            division_at(1, 0, 0, 7_000, 10_000, 0),
            division_at(2, 0, 0, 10_000, 10_000, 99),
        ]);
        let connected: HashSet<StateId> = [StateId(0)].into_iter().collect();
        let stockpile_value = military_supplies_units_to_value(1_500_000);
        let plan = plan_country_daily_supply(
            CountryId(0),
            stockpile_value,
            &divisions,
            &definitions,
            &connected,
        )
        .unwrap();
        let sum_allocated: u128 = plan.allocations.iter().map(|a| a.allocated_units).sum();
        assert_eq!(
            sum_allocated, plan.consumed_units,
            "配分合計は消費量と一致するはず(切断分は常に0)"
        );
        let disconnected_total: u128 = plan
            .allocations
            .iter()
            .filter(|a| a.division_id == DivisionId(2))
            .map(|a| a.allocated_units)
            .sum();
        assert_eq!(disconnected_total, 0);
    }

    #[test]
    fn p26_006_hamilton_tie_break_by_division_id_is_preserved_among_connected_divisions() {
        // P26-004の既存tie-break(remainder降順→DivisionId昇順)が、接続Divisionだけの
        // 母集合でも維持されることを確認する(P26-004-PRE-005/P26-004のtie-breakテストと
        // 同じ数値設定、切断Divisionを1体追加しただけ)。
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division_at(0, 0, 0, 10_000, 10_000, 0),
            division_at(1, 0, 0, 10_000, 10_000, 0),
            division_at(2, 0, 0, 10_000, 10_000, 99), // 切断、tie-breakに関与しない
        ]);
        let connected: HashSet<StateId> = [StateId(0)].into_iter().collect();
        // 総需要2.0のうち1単位不足(1_999_999 units)。remainderが同値なら
        // DivisionId昇順(0が優先)で端数を受け取るはず。
        let stockpile_value = military_supplies_units_to_value(1_999_999);
        let plan = plan_country_daily_supply(
            CountryId(0),
            stockpile_value,
            &divisions,
            &definitions,
            &connected,
        )
        .unwrap();
        let d0 = plan.allocations.iter().find(|a| a.division_id == DivisionId(0)).unwrap();
        let d1 = plan.allocations.iter().find(|a| a.division_id == DivisionId(1)).unwrap();
        assert_eq!(d0.allocated_units, 1_000_000, "DivisionId昇順で端数を受け取るのは0のはず");
        assert_eq!(d1.allocated_units, 999_999);
    }

    #[test]
    fn p26_006_one_countrys_disconnected_division_does_not_change_another_countrys_plan() {
        // plan_country_daily_supplyは国家単位の純粋関数のため、他国家の切断状態は
        // そもそも入力に現れない――country_idでフィルタされるdivisionsの母集合が
        // 国家間分離を構造的に保証することを明示的に確認する。
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division_at(0, 0, 0, 10_000, 10_000, 0),  // country 0: 接続
            division_at(1, 1, 0, 10_000, 10_000, 99), // country 1: 切断(country 0視点では無関係)
        ]);
        let connected_for_country_0: HashSet<StateId> = [StateId(0)].into_iter().collect();
        let plan = plan_country_daily_supply(
            CountryId(0),
            100.0,
            &divisions,
            &definitions,
            &connected_for_country_0,
        )
        .unwrap();
        assert_eq!(plan.allocations.len(), 1);
        assert_eq!(plan.allocations[0].division_id, DivisionId(0));
        assert_eq!(plan.allocations[0].fulfillment, 1.0);
    }

    // ─── P27-005: 攻撃側前線補給ブリッジ ─────────────────────────────────────

    use crate::military::battle::{Battle, BattleRegistry, BattleStatus};
    use crate::military::capital_connectivity::connected_states_from_capital as csfc;

    /// 0(自国首都)-1(自国)-2(敵)-3(敵)、加えて4(自国、0と2に隣接)・
    /// 5(自国だが首都から孤立、2に隣接)。
    fn fl_states() -> StateRegistry {
        let mk = |id: usize, owner: usize, n: &[usize]| StateData {
            id: StateId(id),
            owner_country_id: CountryId(owner),
            neighbors: n.iter().map(|&x| StateId(x)).collect(),
            ..StateData::default()
        };
        StateRegistry::build(vec![
            mk(0, 0, &[1, 4]),
            mk(1, 0, &[0, 2]),
            mk(2, 1, &[1, 3, 4, 5]),
            mk(3, 1, &[2]),
            mk(4, 0, &[0, 2]),
            mk(5, 0, &[2]),
        ])
    }

    fn fl_battle(state: usize, attacker: usize, atk_ids: &[usize], status: BattleStatus) -> Battle {
        Battle {
            id: crate::common::BattleId(0),
            war_id: crate::common::WarId(0),
            state_id: StateId(state),
            attacker_country: CountryId(attacker),
            defender_country: CountryId(1 - attacker.min(1)),
            attacker_division_ids: atk_ids.iter().map(|&i| DivisionId(i)).collect(),
            defender_division_ids: vec![],
            attacker_origins: HashMap::new(),
            start_date: "1800/01/01".to_string(),
            elapsed_days: 0,
            status,
        }
    }

    fn fl_registry(b: Battle) -> BattleRegistry {
        let mut r = BattleRegistry::default();
        r.start_battle(b).unwrap();
        r
    }

    fn atk_at(state: usize) -> Division {
        Division {
            current_state: StateId(state),
            status: DivisionStatus::Fighting,
            ..division(0, 0, 1, 10_000, 10_000)
        }
    }

    fn eligible(d: &Division, sr: &StateRegistry, br: &BattleRegistry) -> bool {
        let connected = csfc(StateId(0), CountryId(0), sr);
        is_frontline_supply_eligible(d, &connected, sr, br)
    }

    #[test]
    fn frontline_bridge_grants_eligibility_to_a_real_attacker_next_to_a_connected_friendly_state() {
        let sr = fl_states();
        let br = fl_registry(fl_battle(2, 0, &[0], BattleStatus::Ongoing));
        assert!(eligible(&atk_at(2), &sr, &br));
    }

    #[test]
    fn frontline_bridge_is_not_used_when_the_division_is_already_normally_connected() {
        let sr = fl_states();
        let br = fl_registry(fl_battle(1, 0, &[0], BattleStatus::Ongoing));
        assert!(!eligible(&atk_at(1), &sr, &br), "通常連結州ではフォールバックを使わない");
    }

    #[test]
    fn frontline_bridge_requires_an_actual_ongoing_battle() {
        let sr = fl_states();
        let moving =
            Division { status: DivisionStatus::Moving, destination: Some(StateId(2)), ..atk_at(2) };
        assert!(
            !eligible(&moving, &sr, &BattleRegistry::default()),
            "戦争中・敵州・Movingだけでは資格なし"
        );
        for st in [BattleStatus::AttackerWon, BattleStatus::DefenderWon, BattleStatus::Cancelled] {
            let br = fl_registry(fl_battle(2, 0, &[0], st));
            assert!(!eligible(&atk_at(2), &sr, &br), "Ongoing以外の戦闘は対象外");
        }
    }

    #[test]
    fn frontline_bridge_excludes_defenders_and_wrong_attacker_records() {
        let sr = fl_states();
        // 攻撃側国家が相手国の戦闘: 自国Divisionは防御側扱い。
        let br = fl_registry(fl_battle(2, 1, &[7], BattleStatus::Ongoing));
        assert!(!eligible(&atk_at(2), &sr, &br));
        // 攻撃側国家は自国だが、参加者一覧に登録されていない。
        let br = fl_registry(fl_battle(2, 0, &[9], BattleStatus::Ongoing));
        assert!(!eligible(&atk_at(2), &sr, &br));
        // 戦闘州が現在州と異なる。
        let br = fl_registry(fl_battle(3, 0, &[0], BattleStatus::Ongoing));
        assert!(!eligible(&atk_at(2), &sr, &br));
    }

    #[test]
    fn frontline_bridge_needs_an_adjacent_friendly_state() {
        let sr = fl_states();
        let br = fl_registry(fl_battle(3, 0, &[0], BattleStatus::Ongoing));
        assert!(!eligible(&atk_at(3), &sr, &br), "隣接が敵州2だけなら資格なし(連鎖投射なし)");
    }

    #[test]
    fn frontline_bridge_is_lost_when_the_friendly_neighbour_is_disconnected_enemy_or_neutral() {
        let mut sr = fl_states();
        let br = fl_registry(fl_battle(2, 0, &[0], BattleStatus::Ongoing));
        // 回廊遮断: 1と4を敵controllerへ → 5は自国だが首都非連結。
        sr.get_mut(StateId(1)).unwrap().controller_country = Some(CountryId(1));
        sr.get_mut(StateId(4)).unwrap().controller_country = Some(CountryId(1));
        assert!(!eligible(&atk_at(2), &sr, &br));
        // 中立controllerでも同様。
        sr.get_mut(StateId(1)).unwrap().controller_country = Some(CountryId(2));
        assert!(!eligible(&atk_at(2), &sr, &br));
        // 再接続すれば自然に回復する。
        sr.get_mut(StateId(1)).unwrap().controller_country = Some(CountryId(0));
        assert!(eligible(&atk_at(2), &sr, &br));
    }

    #[test]
    fn frontline_bridge_is_lost_when_the_capital_falls() {
        let mut sr = fl_states();
        let br = fl_registry(fl_battle(2, 0, &[0], BattleStatus::Ongoing));
        sr.get_mut(StateId(0)).unwrap().controller_country = Some(CountryId(1));
        assert!(!eligible(&atk_at(2), &sr, &br));
    }

    #[test]
    fn frontline_bridge_tolerates_missing_state_references() {
        let sr = fl_states();
        let br = fl_registry(fl_battle(99, 0, &[0], BattleStatus::Ongoing));
        assert!(!eligible(&atk_at(99), &sr, &br));
    }

    #[test]
    fn frontline_bridge_result_is_independent_of_neighbour_order() {
        let mut sr = fl_states();
        let br = fl_registry(fl_battle(2, 0, &[0], BattleStatus::Ongoing));
        let a = eligible(&atk_at(2), &sr, &br);
        sr.get_mut(StateId(2)).unwrap().neighbors.reverse();
        assert_eq!(a, eligible(&atk_at(2), &sr, &br));
    }

    #[test]
    fn plan_counts_a_frontline_division_demand_once_even_with_several_valid_neighbours() {
        let divisions = divs(vec![atk_at(2)]);
        let definitions = defs(vec![infantry_def(1)]);
        let mut fl = HashSet::new();
        fl.insert(DivisionId(0));
        let plan = plan_country_daily_supply_with_frontline(
            CountryId(0),
            100.0,
            &divisions,
            &definitions,
            &HashSet::new(),
            &fl,
        )
        .unwrap();
        assert_eq!(plan.total_demand_units, plan.eligible_demand_units);
        assert_eq!(plan.allocations.len(), 1);
        assert_eq!(plan.allocations[0].allocated_units, plan.allocations[0].demand_units);
        assert_eq!(plan.allocations[0].fulfillment, 1.0);
        let without = plan_country_daily_supply(
            CountryId(0),
            100.0,
            &divisions,
            &definitions,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(without.allocations[0].fulfillment, 0.0, "通常経路(無変更)では従来どおり0");
    }

    #[test]
    fn plan_shares_a_shortage_between_normal_and_frontline_divisions_by_the_existing_hamilton_rule()
    {
        let d_normal = Division { current_state: StateId(0), ..division(0, 0, 1, 10_000, 10_000) };
        let d_front = Division { current_state: StateId(2), ..division(1, 0, 1, 10_000, 10_000) };
        let divisions = divs(vec![d_normal, d_front]);
        let definitions = defs(vec![infantry_def(1)]);
        let mut connected = HashSet::new();
        connected.insert(StateId(0));
        let mut fl = HashSet::new();
        fl.insert(DivisionId(1));
        let full = plan_country_daily_supply_with_frontline(
            CountryId(0),
            100.0,
            &divisions,
            &definitions,
            &connected,
            &fl,
        )
        .unwrap();
        let half_stock = military_supplies_units_to_value(full.total_demand_units as u64 / 2);
        let plan = plan_country_daily_supply_with_frontline(
            CountryId(0),
            half_stock,
            &divisions,
            &definitions,
            &connected,
            &fl,
        )
        .unwrap();
        let allocated: u128 = plan.allocations.iter().map(|a| a.allocated_units).sum();
        assert_eq!(allocated, plan.consumed_units, "保存則: 支給総量=備蓄消費量");
        assert_eq!(plan.consumed_units, full.total_demand_units / 2);
        assert!(
            plan.allocations.iter().all(|a| a.allocated_units > 0),
            "通常・前線の両方へ按分される"
        );
    }
}
