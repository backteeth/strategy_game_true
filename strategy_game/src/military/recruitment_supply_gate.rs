//! P26-008-A: MilitarySuppliesを考慮した募兵停止／再開ゲート。
//!
//! P26-008-PRE-001が確定した設計(`verification_logs/phase-26/p26-008-pre-001/`)を、
//! 既存のAI月次募兵判断(`military::ai::decide_and_recruit_for_country`)へ
//! **追加条件として**接続する。既存の目標Division数・人的資源予備・募兵州選択・
//! `request_recruitment`内部会計はいずれも変更しない――ここは「新規募兵を今月
//! 開始してよいか」という1個のbool判定だけを提供する。
//!
//! 指標は2系統に分離する(P26-008-PRE-001の設計どおり):
//! * **国家備蓄健全性**(`national_reserve_days`、`bridge_deficit`) — 総需要
//!   (`country_daily_demand_units`、接続性とは無関係)を分母に使う。首都占領で
//!   接続需要が0になっても、総需要は変化しないため「備蓄健全」と誤認しない
//!   (P26-008-PRE-001 §21の罠を回避する設計そのもの)。
//! * **配送健全性**(`min_fulfillment`) — 既存の`MilitaryRegistry::
//!   supply_fulfillment_of`をそのまま読むだけで、新規の補給計算を行わない。
//!
//! 真のヒステリシス(中間帯での状態保持)には、現在の停止/再開のどちらの状態に
//! あったかを覚えている必要があり、備蓄水準だけからは再導出できない
//! (P26-008-PRE-001 §12の論理監査どおり)。このため`CountryAiState`へ新規
//! `recruitment_supply_blocked: bool`を追加した(既存の`cooldown_until_day`と
//! 同種の「世界状態から再導出できないAI決定メモリ」、`decision_reason`や
//! Military AI状態を意味の異なる代替ストレージとして流用してはいない)。

use crate::app::time::GameDate;
use crate::common::CountryId;
use crate::country::CountryData;
use crate::economy::resources::{
    MilitarySuppliesUnitsError, ResourceType, military_supplies_value_to_units,
};
use crate::military::data::{
    Division, DivisionDefinition, DivisionStatus, MilitaryRegistry,
};
use crate::military::supply_initialization::{
    CalendarError, SupplyDemandError, compute_required_bridge_units, country_daily_demand_units,
    days_in_month, division_daily_demand_units, events_before_next_monthly_production,
};
use crate::war::data::WarRegistry;
use std::collections::HashMap;

// ─── 正規定数(spec §6、P26-008-Bでも再利用できるようpubにする) ────────────────

/// 平時の目標備蓄日数(1生産周期)。P26-004-PRE-005の初期ブリッジ値(31.0)の
/// 転用ではなく、「常に最低1か月分の備蓄を維持する」という独立した根拠から
/// 導出した値(P26-008-PRE-001 §24)。
pub const PEACETIME_TARGET_DAYS: u32 = 30;
/// 戦時の目標備蓄日数(平時の2倍、損耗・追加募兵による需要増を見込む)。
pub const WARTIME_TARGET_DAYS: u32 = 60;

/// 平時の募兵停止しきい値(目標の2/3)。
pub const PEACETIME_STOP_DAYS: u32 = 20;
/// 戦時の募兵停止しきい値。
pub const WARTIME_STOP_DAYS: u32 = 40;
/// 平時の募兵再開しきい値(目標の7/6、停止より確実に上)。
pub const PEACETIME_RESUME_DAYS: u32 = 35;
/// 戦時の募兵再開しきい値。
pub const WARTIME_RESUME_DAYS: u32 = 70;

/// 戦争状態から目標備蓄日数を選ぶ。
pub fn target_days_for(is_at_war: bool) -> u32 {
    if is_at_war { WARTIME_TARGET_DAYS } else { PEACETIME_TARGET_DAYS }
}

/// 戦争状態から募兵停止しきい値を選ぶ。
pub fn stop_days_for(is_at_war: bool) -> u32 {
    if is_at_war { WARTIME_STOP_DAYS } else { PEACETIME_STOP_DAYS }
}

/// 戦争状態から募兵再開しきい値を選ぶ。
pub fn resume_days_for(is_at_war: bool) -> u32 {
    if is_at_war { WARTIME_RESUME_DAYS } else { PEACETIME_RESUME_DAYS }
}

// ─── 国家備蓄日数(spec §7) ─────────────────────────────────────────────────

/// `stockpile_units`が`demand_units`を`days`日分以上支えられるかを、整数の
/// checked演算だけで判定する(f64除算・NaN・Infinityを一切生成しない)。
///
/// `demand_units == 0`の場合は常に`true`(現在総需要が0の国家を「不足」と
/// 誤判定しない、spec §8)。`demand_units × days`がu128をoverflowする場合は
/// 「不足なし」とは扱わず`false`を返す(spec §7: overflowを安全側=不足ありに倒す。
/// 実際の値域[demand<=10^15、days<=70]ではoverflowは理論上到達不能)。
pub fn has_reserve_for_days(stockpile_units: u128, demand_units: u128, days: u32) -> bool {
    match demand_units.checked_mul(days as u128) {
        Some(required) => stockpile_units >= required,
        None => false,
    }
}

// ─── 将来募兵需要(spec §9) ─────────────────────────────────────────────────

/// 新規候補Division(募兵完了直後の状態: `manpower == max_manpower ==
/// def.required_manpower`、`try_spawn_division`と同じ前提)の日次需要unitsを、
/// 既存の正規関数`division_daily_demand_units`をそのまま呼んで導出する
/// (需要式を複製しない、CountryId別固定値や`1.0`の直書きをしない、spec §9)。
pub fn proposed_division_daily_demand_units(
    def: &DivisionDefinition,
) -> Result<u128, SupplyDemandError> {
    let synthetic = Division {
        id: crate::common::DivisionId(0),
        owner: CountryId(0),
        division_type: def.division_type,
        size: def.size,
        current_state: crate::common::StateId(0),
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: def.required_manpower,
        max_manpower: def.required_manpower,
        equipment: 0.0,
        max_equipment: 0.0,
        organization: 0.0,
        max_organization: 0.0,
        morale: 0.0,
        max_morale: 0.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: def.id,
        attack_power: 0,
        defense_power: 0,
        combat_id: None,
    };
    let mut definitions = HashMap::with_capacity(1);
    definitions.insert(def.id, def.clone());
    division_daily_demand_units(&synthetic, &definitions)
}

// ─── bridge判定(spec §10、P26-004-PRE-005の既存関数を再利用) ──────────────────

/// bridge判定が失敗し得る理由。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BridgeForecastError {
    Demand(SupplyDemandError),
    Calendar(CalendarError),
    /// 現在備蓄がNaN・負数等で正規unitsへ変換できない。
    InvalidCurrentStockpile,
}

impl From<SupplyDemandError> for BridgeForecastError {
    fn from(e: SupplyDemandError) -> Self {
        BridgeForecastError::Demand(e)
    }
}
impl From<CalendarError> for BridgeForecastError {
    fn from(e: CalendarError) -> Self {
        BridgeForecastError::Calendar(e)
    }
}

/// 現在日から次回月次生産までに、`planning_daily_demand_units`(現在総需要＋
/// 新規候補Division需要)を支えるために必要な備蓄unitsと、現在備蓄との不足分を
/// 計算する。`compute_required_bridge_units`/`events_before_next_monthly_
/// production`(いずれもP26-004-PRE-005の既存正規関数)をそのまま呼び、
/// 「1日の予備」を含む既存の計算式を複製しない。
///
/// `current_month`は1〜12、`current_day`は`GameDate.day`(月内の実際の日、
/// New Game/月境界直後は1)をそのまま渡す――簡略暦への変換は行わない
/// (spec §10: 簡略月テストだけでなく実カレンダー式を直接検証する)。
pub fn compute_bridge_deficit_units(
    planning_daily_demand_units: u128,
    current_month: u8,
    current_day: u8,
    current_stockpile_units: u128,
) -> Result<u128, BridgeForecastError> {
    let month_length = days_in_month(current_month)?;
    let events = events_before_next_monthly_production(current_day, month_length)?;
    let required = compute_required_bridge_units(planning_daily_demand_units, events)?;
    Ok(required.saturating_sub(current_stockpile_units))
}

// ─── 配送健全性(spec §11) ──────────────────────────────────────────────────

/// 需要が正の生存Divisionについて、`MilitaryRegistry::supply_fulfillment_of`の
/// 最小値を求める。需要0のDivisionは母集合から除外する(spec §11・§8)。
/// 対象Divisionが1件もなければ`None`(「配送不足signalなし」)。
///
/// `DivisionId`昇順で走査してから最小値を取るため、`HashMap`反復順には
/// 依存しない(最小値自体はどの順で走査しても数学的に同じ値になるが、この
/// コードベース全体の既存規約[決定的な走査順を明示する]に合わせる)。
pub fn min_fulfillment_of_positive_demand_divisions(
    country_id: CountryId,
    military_registry: &MilitaryRegistry,
) -> Result<Option<f64>, SupplyDemandError> {
    let mut ids: Vec<_> = military_registry
        .divisions
        .values()
        .filter(|d| d.owner == country_id && d.manpower > 0 && d.status != DivisionStatus::Destroyed)
        .map(|d| d.id)
        .collect();
    ids.sort_by_key(|id| id.0);
    min_fulfillment_over_division_ids(&ids, military_registry)
}

/// 需要が正の生存Division群(呼び出し側が対象範囲を確定した`DivisionId`の
/// 昇順スライス)について、`MilitaryRegistry::supply_fulfillment_of`の最小値を求める
/// 共通ヘルパー(spec: P26-008-A国家全体版・P26-008-B前線範囲版のいずれからも、
/// 需要式・最小値計算そのものを複製せずに再利用できるよう抽出)。需要0の
/// Divisionは母集合から除外し、対象が1件もなければ`None`(「配送不足signalなし」)。
/// 生存判定・対象範囲の絞り込みは呼び出し側の責務(このヘルパーは受け取った
/// `division_ids`をそのまま信頼する)。
pub fn min_fulfillment_over_division_ids(
    division_ids: &[crate::common::DivisionId],
    military_registry: &MilitaryRegistry,
) -> Result<Option<f64>, SupplyDemandError> {
    let mut min_fulfillment: Option<f64> = None;
    for &id in division_ids {
        let Some(division) = military_registry.divisions.get(&id) else {
            continue;
        };
        let demand = division_daily_demand_units(division, &military_registry.definitions)?;
        if demand == 0 {
            continue;
        }
        let fulfillment = military_registry.supply_fulfillment_of(id);
        min_fulfillment = Some(match min_fulfillment {
            Some(current_min) => current_min.min(fulfillment),
            None => fulfillment,
        });
    }
    Ok(min_fulfillment)
}

/// 配送が完全に健全か("全ての需要正Divisionが完全充足")を判定する。
/// 対象Divisionなし(`None`)は「配送不足signalなし」として健全扱いにする
/// (spec §8・§11: 需要0国家を不健全と誤判定しない)。
///
/// `supply_fulfillment_of`が返すf64は、日次補給処理(P26-004)自身が
/// `demand==0`または`consumed==eligible_demand`のときに文字通り`1.0`を
/// 代入する(比率計算を経由しない)ため、Hamilton配分で完全充足したDivisionも
/// 含めて、ここでの`>= 1.0`比較はfloatの丸め誤差を新たに持ち込まない
/// (新しい正規会計判定を発明するのではなく、既存の正規full-scale表現[`1.0`]と
/// 突き合わせているだけである、spec §11)。
pub fn is_delivery_fully_healthy(min_fulfillment: Option<f64>) -> bool {
    match min_fulfillment {
        Some(f) => f >= 1.0,
        None => true,
    }
}

// ─── 真のヒステリシス状態遷移(spec §12) ────────────────────────────────────

/// 募兵補給ゲートの1回分の状態遷移。中間帯(停止未満だが再開未満)では、
/// 呼び出し時点の`currently_blocked`をそのまま維持する――同じ現在値でも
/// 以前の状態によって結果が変わる、真のヒステリシスとして実装する
/// (spec §12: 中間帯を常に停止または常に許可する見せかけのヒステリシスを禁止)。
pub fn update_recruitment_supply_gate(
    currently_blocked: bool,
    is_at_war: bool,
    current_stockpile_units: u128,
    current_total_demand_units: u128,
    bridge_deficit_units: u128,
    delivery_fully_healthy: bool,
) -> bool {
    let stop_days = stop_days_for(is_at_war);
    let resume_days = resume_days_for(is_at_war);
    let no_bridge_deficit = bridge_deficit_units == 0;

    if currently_blocked {
        let reserve_ok_for_resume =
            has_reserve_for_days(current_stockpile_units, current_total_demand_units, resume_days);
        !(reserve_ok_for_resume && no_bridge_deficit && delivery_fully_healthy)
    } else {
        let reserve_ok_for_stop =
            has_reserve_for_days(current_stockpile_units, current_total_demand_units, stop_days);
        !(reserve_ok_for_stop && no_bridge_deficit && delivery_fully_healthy)
    }
}

// ─── 1国家分の統合評価 ──────────────────────────────────────────────────────

/// `update_recruitment_supply_gate`が失敗し得る理由(いずれも防御的な分岐、
/// 通常の正常なゲーム状態では到達しない)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SupplyGateEvaluationError {
    Demand(SupplyDemandError),
    Bridge(BridgeForecastError),
    InvalidCurrentStockpile,
}

impl From<SupplyDemandError> for SupplyGateEvaluationError {
    fn from(e: SupplyDemandError) -> Self {
        SupplyGateEvaluationError::Demand(e)
    }
}
impl From<BridgeForecastError> for SupplyGateEvaluationError {
    fn from(e: BridgeForecastError) -> Self {
        SupplyGateEvaluationError::Bridge(e)
    }
}

/// 1国家分のゲート評価をまとめて行い、更新後の`blocked`値を返す
/// (`CountryAiState.recruitment_supply_blocked`への書き込みは呼び出し側が行う)。
/// このAI自身は`MilitarySupplies`備蓄・`available_manpower`・`treasury`・
/// `recruitment_queue`のいずれも変更しない(既存の`military::ai`モジュールの
/// 契約と同じ)。
#[allow(clippy::too_many_arguments)]
pub fn evaluate_recruitment_supply_gate(
    country: &CountryData,
    military_registry: &MilitaryRegistry,
    war_registry: &WarRegistry,
    proposed_def: &DivisionDefinition,
    game_date: &GameDate,
    currently_blocked: bool,
) -> Result<bool, SupplyGateEvaluationError> {
    let is_at_war = war_registry.is_country_at_war(country.id);

    let current_stockpile_value = country.stockpile.get(ResourceType::MilitarySupplies);
    let current_stockpile_units =
        military_supplies_value_to_units(current_stockpile_value).map_err(
            |_: MilitarySuppliesUnitsError| SupplyGateEvaluationError::InvalidCurrentStockpile,
        )? as u128;

    let current_total_demand_units =
        country_daily_demand_units(country.id, &military_registry.divisions, &military_registry.definitions)?;

    let proposed_demand_units = proposed_division_daily_demand_units(proposed_def)?;
    let planning_daily_demand_units = current_total_demand_units
        .checked_add(proposed_demand_units)
        .ok_or(SupplyGateEvaluationError::Demand(SupplyDemandError::Overflow))?;

    let bridge_deficit_units = compute_bridge_deficit_units(
        planning_daily_demand_units,
        game_date.month,
        game_date.day,
        current_stockpile_units,
    )?;

    let min_fulfillment = min_fulfillment_of_positive_demand_divisions(country.id, military_registry)?;
    let delivery_fully_healthy = is_delivery_fully_healthy(min_fulfillment);

    Ok(update_recruitment_supply_gate(
        currently_blocked,
        is_at_war,
        current_stockpile_units,
        current_total_demand_units,
        bridge_deficit_units,
        delivery_fully_healthy,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{DivisionDefinitionId, DivisionId, StateId};
    use crate::military::data::DivisionSize;

    // ── has_reserve_for_days ──────────────────────────────────────────

    #[test]
    fn zero_demand_always_has_reserve() {
        assert!(has_reserve_for_days(0, 0, 30));
        assert!(has_reserve_for_days(0, 0, u32::MAX));
    }

    #[test]
    fn exact_boundary_is_sufficient() {
        assert!(has_reserve_for_days(300, 10, 30));
        assert!(!has_reserve_for_days(299, 10, 30));
    }

    #[test]
    fn overflow_is_treated_as_insufficient_not_as_plenty() {
        assert!(!has_reserve_for_days(u128::MAX, u128::MAX, 2));
    }

    // ── proposed_division_daily_demand_units ──────────────────────────

    fn def(id: usize, required_manpower: u64, supply_usage: f32) -> DivisionDefinition {
        DivisionDefinition {
            id: DivisionDefinitionId(id),
            name: "Test".to_string(),
            division_type: crate::military::data::DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower,
            required_equipment: 100.0,
            recruitment_days: 30,
            movement_speed: 1.0,
            attack: 10.0,
            defense: 10.0,
            breakthrough: 5.0,
            organization: 50.0,
            morale: 50.0,
            supply_usage,
            maintenance_cost: 1.0,
        }
    }

    #[test]
    fn proposed_division_demand_matches_a_freshly_spawned_division_exactly() {
        let d = def(1, 10_000, 1.0);
        let via_helper = proposed_division_daily_demand_units(&d).unwrap();

        // try_spawn_divisionと同じ前提(manpower==max_manpower==required)の
        // 実Divisionを直接構築し、同じ正規関数で需要を求めた場合と一致するはず。
        let spawned = Division {
            id: DivisionId(0),
            owner: CountryId(0),
            division_type: d.division_type,
            size: d.size,
            current_state: StateId(0),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: d.required_manpower,
            max_manpower: d.required_manpower,
            equipment: d.required_equipment,
            max_equipment: d.required_equipment,
            organization: d.organization * 0.1,
            max_organization: d.organization,
            morale: d.morale * 0.5,
            max_morale: d.morale,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: d.id,
            attack_power: d.attack as i32,
            defense_power: d.defense as i32,
            combat_id: None,
        };
        let mut defs = HashMap::new();
        defs.insert(d.id, d.clone());
        let direct = division_daily_demand_units(&spawned, &defs).unwrap();

        assert_eq!(via_helper, direct);
        assert_eq!(via_helper, 1_000_000); // supply_usage=1.0 -> exactly 1.0 units
    }

    #[test]
    fn proposed_division_demand_scales_with_supply_usage() {
        let d = def(1, 10_000, 3.0);
        assert_eq!(proposed_division_daily_demand_units(&d).unwrap(), 3_000_000);
    }

    // ── compute_bridge_deficit_units ───────────────────────────────────

    #[test]
    fn bridge_deficit_matches_the_calendar_at_new_game_start() {
        // New Game開始(1800/01/01)と同じ入力: day=1, 31日の月 ->
        // events=30, required=demand*(30+1)。
        let deficit = compute_bridge_deficit_units(1_000_000, 1, 1, 0).unwrap();
        assert_eq!(deficit, 31_000_000);
    }

    #[test]
    fn bridge_deficit_is_zero_when_stockpile_already_covers_it() {
        let deficit = compute_bridge_deficit_units(1_000_000, 1, 1, 31_000_000).unwrap();
        assert_eq!(deficit, 0);
    }

    #[test]
    fn bridge_deficit_shrinks_as_the_month_progresses() {
        // 月末に近いほど、次回生産までの残り日数(events)が減る。
        let early = compute_bridge_deficit_units(1_000_000, 1, 1, 0).unwrap();
        let late = compute_bridge_deficit_units(1_000_000, 1, 28, 0).unwrap();
        assert!(late < early, "月末に近いほどbridge必要量は小さくなるはず");
    }

    #[test]
    fn bridge_deficit_handles_february_and_thirty_day_months() {
        let feb = compute_bridge_deficit_units(1_000_000, 2, 1, 0).unwrap(); // 28日の月
        assert_eq!(feb, 28_000_000);
        let april = compute_bridge_deficit_units(1_000_000, 4, 1, 0).unwrap(); // 30日の月
        assert_eq!(april, 30_000_000);
    }

    #[test]
    fn bridge_deficit_zero_demand_is_zero() {
        assert_eq!(compute_bridge_deficit_units(0, 6, 15, 0).unwrap(), 0);
    }

    // ── min_fulfillment_of_positive_demand_divisions / is_delivery_fully_healthy ──

    #[test]
    fn no_divisions_means_no_delivery_signal() {
        let military = MilitaryRegistry::default();
        let result = min_fulfillment_of_positive_demand_divisions(CountryId(1), &military).unwrap();
        assert_eq!(result, None);
        assert!(is_delivery_fully_healthy(result));
    }

    #[test]
    fn fully_healthy_and_unhealthy_thresholds() {
        assert!(is_delivery_fully_healthy(Some(1.0)));
        assert!(!is_delivery_fully_healthy(Some(0.999_999)));
        assert!(!is_delivery_fully_healthy(Some(0.0)));
    }

    // ── update_recruitment_supply_gate: 真のヒステリシス ─────────────────

    #[test]
    fn peacetime_stop_boundary_is_exact() {
        // 20日ちょうどは停止しない(has_reserve_for_daysの`>=`規約どおり)。
        assert!(!update_recruitment_supply_gate(false, false, 20 * 1_000_000, 1_000_000, 0, true));
        // 20日未満なら停止する。
        assert!(update_recruitment_supply_gate(false, false, 20 * 1_000_000 - 1, 1_000_000, 0, true));
    }

    #[test]
    fn wartime_stop_boundary_is_exact() {
        assert!(!update_recruitment_supply_gate(false, true, 40 * 1_000_000, 1_000_000, 0, true));
        assert!(update_recruitment_supply_gate(false, true, 40 * 1_000_000 - 1, 1_000_000, 0, true));
    }

    #[test]
    fn peacetime_resume_boundary_is_exact() {
        // blocked中、35日未満では再開しない。
        assert!(update_recruitment_supply_gate(true, false, 35 * 1_000_000 - 1, 1_000_000, 0, true));
        // 35日ちょうどで再開する。
        assert!(!update_recruitment_supply_gate(true, false, 35 * 1_000_000, 1_000_000, 0, true));
    }

    #[test]
    fn wartime_resume_boundary_is_exact() {
        assert!(update_recruitment_supply_gate(true, true, 70 * 1_000_000 - 1, 1_000_000, 0, true));
        assert!(!update_recruitment_supply_gate(true, true, 70 * 1_000_000, 1_000_000, 0, true));
    }

    #[test]
    fn the_middle_band_preserves_prior_state_true_hysteresis() {
        // 平時25日(20日以上35日未満の中間帯)は、以前の状態によって結果が変わる。
        let reserve_units = 25 * 1_000_000u128;
        let was_unblocked =
            update_recruitment_supply_gate(false, false, reserve_units, 1_000_000, 0, true);
        let was_blocked =
            update_recruitment_supply_gate(true, false, reserve_units, 1_000_000, 0, true);
        assert!(!was_unblocked, "以前から許可中なら中間帯では継続許可のはず");
        assert!(was_blocked, "以前に停止していたなら中間帯では再開待ちのはず");
        assert_ne!(
            was_unblocked, was_blocked,
            "同じ備蓄水準でも以前の状態によって結果が異なるのが真のヒステリシス"
        );
    }

    #[test]
    fn bridge_deficit_alone_blocks_even_with_ample_reserve_days() {
        // reserve自体は十分でも、bridge_deficit>0なら停止する。
        assert!(update_recruitment_supply_gate(false, false, 1_000 * 1_000_000, 1_000_000, 1, true));
    }

    #[test]
    fn unhealthy_delivery_alone_blocks_even_with_ample_reserve_and_no_bridge_deficit() {
        assert!(update_recruitment_supply_gate(false, false, 1_000 * 1_000_000, 1_000_000, 0, false));
    }

    #[test]
    fn war_status_changes_the_active_threshold_immediately_within_the_same_call() {
        // 同じ備蓄31日分は、平時では停止しない(>=20)が戦時では停止する(<40)。
        let reserve_units = 31 * 1_000_000u128;
        assert!(!update_recruitment_supply_gate(false, false, reserve_units, 1_000_000, 0, true));
        assert!(update_recruitment_supply_gate(false, true, reserve_units, 1_000_000, 0, true));
    }
}
