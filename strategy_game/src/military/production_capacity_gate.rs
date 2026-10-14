//! P29-004: AI 募兵の「持続的な軍用補給物資の生産能力」ゲート(P29-003-DIAG-001 の E3)。
//!
//! 既存の募兵補給ゲート(`recruitment_supply_gate`、備蓄=ストック判定)は一過性の備蓄だけを
//! 見るため、月産が月需要を下回る数まで師団を増やせた(Ferrowyn 1814〜1819 年、Dwarf
//! 1808 年)。本ゲートは、**新しい師団が完成した後の日次需要を、実際の月次産出で賄えるか**
//! というフロー判定を、既存のストック判定の**後に追加**する(既存の判定・ヒステリシス状態は
//! 一切変更しない。備蓄が潤沢でも、生産が足りなければ許可しない。生産が足りても、既存の
//! 備蓄・人的資源等の条件を満たさなければ許可しない)。
//!
//! ## 生産能力の見積もり
//!
//! 国家が所有する州の MilitaryFactory について、**直近の月次生産で実際に記録された稼働率**
//! (`StateData::building_operation_ratios`。FIX1・D4・入力不足・労働力・物流を反映した
//! 最終稼働率で、Save V1 に既に永続化されている)に、現在の Level と基本出力を掛け、
//! 月次生産と同じく entry ごとに正規 units へ量子化してから合計する
//! (`economy::production`の産出式 `calculate_actual_output` と量子化をそのまま使う)。
//! 理論上の最大出力ではなく、直近の実績を再構成した値である。
//!
//! - **P29-005 による変更(採用期間)**: 通常経路の能力上限は、上の「現在設備による推定」(月次の
//!   稼働率の 1 点推定)と、**直近 N か月(`SUPPLY_HISTORY_WINDOW`)の実際の月次産出の平均**
//!   (`supply_production_history`)の**小さい方**。平均は周期的な高稼働月の過大評価を防ぎ、
//!   現在設備の推定は急な減産への追従を担保する。履歴が N か月分揃うまで(旧 Save・New Game 直後)
//!   通常経路の新規募兵は保留する(最小常備戦力の免除は履歴に依存しない)。
//! - **未集計と生産 0 の区別**: 稼働率が記録されていない MilitaryFactory(Level > 0 で、まだ
//!   月次生産を経ていない新設分)は「未集計」(`unreported_entries`)として数え、能力には
//!   **0 として加算する**(無限大とはみなさない)。稼働率が記録済みで 0 の場合は「生産 0」。
//!   どちらも能力に寄与しないが、診断では区別して返す。
//! - **New Game・Load 直後**: 月次生産の同一フレームで、生産(Economy)が AI(CountryAi)より
//!   先に実行されるため、記録済みの稼働率で判断する。Load 後は Save 内の稼働率で判断する
//!   (ledger などの非永続の履歴に依存しない)。
//!
//! ## 需要への換算
//!
//! 日次需要(units/日)を、月平均日数(うるう年なしの暦で年 365 日 ÷ 12 か月 = 30.4166…)で
//! 月次(units/月)へ換算する(`ceil(daily × 365 / 12)`、整数演算)。月ごとの日数(28〜31)で
//! 判定が揺れないよう、固定の月平均を使う。
//!
//! ## 将来需要
//!
//! 現在の生存師団(**満員時**の需要。損耗中の師団も、補充後は満員の需要になるため)+ 訓練中
//! (募兵キュー)の師団 + 今回新たに募兵する師団。いずれも既存の正規関数
//! (`division_daily_demand_units`・`proposed_division_daily_demand_units`)で算出する
//! (需要式を複製しない)。同一更新で先に許可した注文は`recruitment_queue`に入るため、
//! 後続の判定に自動的に反映される。

use crate::building::data::{BuildingRegistry, BuildingType};
use crate::common::CountryId;
use crate::country::CountryData;
use crate::economy::production::calculate_actual_output;
use crate::economy::resources::{ResourceType, military_supplies_value_to_units};
use crate::military::data::{Division, DivisionDefinition, MilitaryRegistry};
use crate::military::recruitment_supply_gate::proposed_division_daily_demand_units;
use crate::military::supply_initialization::{
    SupplyDemandError, division_daily_demand_units, is_living,
};
use crate::military::supply_production_history::SupplyProductionHistory;
use crate::state::data::StateRegistry;

/// 生産能力の判定を免除する最小の常備戦力(生存師団 + 訓練中の師団の数がこの数未満のとき、
/// 新規の 1 個は生産能力にかかわらず(既存の備蓄・人的資源等の条件を満たせば)許可する)。
///
/// **今回導入する設計値**。免除なし(0)で標準シナリオを 600 か月進めると、全国が初期の 1 個の
/// ままになり 1820 年まで戦争が発生せず、その後 Dwarf が 1 州・国庫 −33,819・税率 1.00 へ
/// 破綻する世界になった(P29-004 報告書 §7)。2 個の免除では全 6 か国が存続し国庫も正を保った。
/// 2 個を超える追加募兵には生産能力の判定が常に適用される。
pub const MIN_STANDING_DIVISIONS: u128 = 2;

/// 月平均日数(うるう年なしの暦): 年 365 日 ÷ 12 か月。
pub const MONTH_DAYS_NUMERATOR: u128 = 365;
pub const MONTH_DAYS_DENOMINATOR: u128 = 12;

/// 日次需要 units を月次 units へ換算する(`ceil(daily × 365 / 12)`)。overflow は `None`。
pub fn monthly_units_from_daily(daily_units: u128) -> Option<u128> {
    let scaled = daily_units.checked_mul(MONTH_DAYS_NUMERATOR)?;
    Some(scaled.div_ceil(MONTH_DAYS_DENOMINATOR))
}

/// 直近の月次生産の実績から再構成した、国家の持続的な月次 MilitarySupplies 産出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SustainedSupply {
    /// 記録済みの稼働率から再構成した月次産出(units/月)。
    pub monthly_units: u128,
    /// Level > 0 だが稼働率が未記録(まだ月次生産を経ていない)の MilitaryFactory entry 数。
    pub unreported_entries: u32,
}

/// `country_id`が所有する州の MilitaryFactory から、持続的な月次産出を再構成する。
/// 州は`StateRegistry.states`の並び順で走査し、entry ごとに units へ量子化してから合計する
/// (月次生産の「quantize-then-sum」と同じ。合計は可換なので順序に依存しない)。
pub fn sustained_monthly_supply(
    country_id: CountryId,
    states: &StateRegistry,
    buildings: &BuildingRegistry,
) -> SustainedSupply {
    let base_output = buildings
        .get(BuildingType::MilitaryFactory)
        .and_then(|d| d.output_resources.get(&ResourceType::MilitarySupplies).copied())
        .unwrap_or(0.0);
    let mut monthly_units: u128 = 0;
    let mut unreported = 0u32;
    for state in states.states.iter().filter(|s| s.owner_country_id == country_id) {
        let level = state.building_level(BuildingType::MilitaryFactory);
        if level == 0 {
            continue;
        }
        match state
            .building_operation_ratios
            .get(&BuildingType::MilitaryFactory)
        {
            None => unreported += 1,
            Some(&op) => {
                let produced = calculate_actual_output(base_output, level, op);
                if let Ok(units) = military_supplies_value_to_units(produced) {
                    monthly_units = monthly_units.saturating_add(units as u128);
                }
            }
        }
    }
    SustainedSupply {
        monthly_units,
        unreported_entries: unreported,
    }
}

/// 生存師団の、満員時の日次需要(units/日)。師団ごとに自身の定義(`supply_usage`)で算出する。
fn full_strength_daily_demand_units(
    division: &Division,
    military: &MilitaryRegistry,
) -> Result<u128, SupplyDemandError> {
    let mut full = division.clone();
    full.manpower = full.max_manpower;
    division_daily_demand_units(&full, &military.definitions)
}

/// 国家の「将来(訓練中を含む)の日次需要」: 生存師団(満員時)+ 募兵キューの師団。
pub fn projected_daily_demand_units(
    country: &CountryData,
    military: &MilitaryRegistry,
) -> Result<u128, SupplyDemandError> {
    let mut ids: Vec<_> = military
        .divisions
        .values()
        .filter(|d| d.owner == country.id && is_living(d))
        .map(|d| d.id)
        .collect();
    ids.sort_by_key(|id| id.0);
    let mut total: u128 = 0;
    for id in ids {
        let d = full_strength_daily_demand_units(&military.divisions[&id], military)?;
        total = total.checked_add(d).ok_or(SupplyDemandError::Overflow)?;
    }
    for item in &country.recruitment_queue {
        let Some(def) = military.definitions.get(&item.division_id) else {
            continue;
        };
        let d = proposed_division_daily_demand_units(def)?;
        total = total.checked_add(d).ok_or(SupplyDemandError::Overflow)?;
    }
    Ok(total)
}

/// ゲートの判定結果(診断・テスト用の内訳つき)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductionCapacityVerdict {
    pub allowed: bool,
    /// 通常経路の最終的な能力上限(units/月)= 「直近 N か月の平均実産出」と「現在設備による
    /// 推定」の小さい方。履歴が揃わない間は 0(通常経路の新規募兵を保留)。
    pub capacity_monthly_units: u128,
    /// 新規 1 個を加えた将来需要の月次換算(units/月)。
    pub required_monthly_units: u128,
    pub unreported_entries: u32,
    /// 現在設備による能力推定(P29-004 の再構成値、units/月)。
    pub current_estimate_units: u128,
    /// 直近 [`crate::military::supply_production_history::SUPPLY_HISTORY_WINDOW`] か月の実産出の平均(units/月)。履歴が揃わなければ`None`。
    pub period_average_units: Option<u128>,
    /// 生存師団 + 訓練中の数が [`MIN_STANDING_DIVISIONS`] 未満で、判定を免除した。
    pub within_minimum_standing_force: bool,
}

/// 新規に 1 個(`def`)募兵した場合の将来需要を、持続的な月次産出で賄えるか判定する。
/// `required <= capacity`(等号は許可)。計算できない(overflow・不正な定義)場合は
/// `Err`を返し、呼び出し側は安全側(見送り)に倒す。
pub fn evaluate_production_capacity_gate(
    country: &CountryData,
    military: &MilitaryRegistry,
    states: &StateRegistry,
    buildings: &BuildingRegistry,
    history: &SupplyProductionHistory,
    now: (i32, u8),
    def: &DivisionDefinition,
) -> Result<ProductionCapacityVerdict, SupplyDemandError> {
    let projected = projected_daily_demand_units(country, military)?;
    let new_division = proposed_division_daily_demand_units(def)?;
    let total_daily = projected
        .checked_add(new_division)
        .ok_or(SupplyDemandError::Overflow)?;
    let required = monthly_units_from_daily(total_daily).ok_or(SupplyDemandError::Overflow)?;
    let supply = sustained_monthly_supply(country.id, states, buildings);
    let standing = military
        .divisions
        .values()
        .filter(|d| d.owner == country.id && is_living(d))
        .count() as u128
        + country.recruitment_queue.len() as u128;
    let within_minimum = standing < MIN_STANDING_DIVISIONS;
    // 通常経路の能力上限: 「直近 N か月の平均実産出」と「現在設備による推定」の小さい方
    // (平均は急な減産に遅れて追従するため、現在設備の推定を上限として併用する)。
    // 履歴が揃わない間は上限を 0 として扱い、通常経路の新規募兵を保留する。
    let period_average = history.average_units(now);
    let capacity = period_average.map_or(0, |avg| avg.min(supply.monthly_units));
    Ok(ProductionCapacityVerdict {
        allowed: within_minimum || (period_average.is_some() && required <= capacity),
        capacity_monthly_units: capacity,
        required_monthly_units: required,
        unreported_entries: supply.unreported_entries,
        current_estimate_units: supply.monthly_units,
        period_average_units: period_average,
        within_minimum_standing_force: within_minimum,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::data::BuildingDefinition;
    use crate::common::{DivisionDefinitionId, DivisionId, StateId};
    use crate::military::data::{DivisionSize, DivisionStatus, DivisionType};
    use crate::military::recruitment::RecruitmentQueueItem;
    use crate::state::data::StateData;
    use std::collections::HashMap;

    const C: CountryId = CountryId(1);

    fn mf_def() -> BuildingDefinition {
        BuildingDefinition {
            building_type: BuildingType::MilitaryFactory,
            name: "Military Factory".to_string(),
            construction_cost: 1200.0,
            required_progress: 90.0,
            required_workforce: 25000.0,
            logistics_cost: 20.0,
            input_resources: HashMap::new(),
            output_resources: HashMap::from([
                (ResourceType::MilitaryEquipment, 20.0),
                (ResourceType::MilitarySupplies, 100.0),
            ]),
            maintenance_cost: 40.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        }
    }

    fn buildings() -> BuildingRegistry {
        let mut r = BuildingRegistry::default();
        r.definitions.insert(BuildingType::MilitaryFactory, mf_def());
        r
    }

    /// `level`・記録済み稼働率`op`(`None`は未記録)の MilitaryFactory を持つ州。
    fn mf_state(id: usize, owner: CountryId, level: u32, op: Option<f32>) -> StateData {
        let mut s = StateData {
            id: StateId(id),
            owner_country_id: owner,
            ..Default::default()
        };
        s.buildings.insert(BuildingType::MilitaryFactory, level);
        if let Some(op) = op {
            s.building_operation_ratios
                .insert(BuildingType::MilitaryFactory, op);
        }
        s
    }

    fn def(usage: f32) -> DivisionDefinition {
        DivisionDefinition {
            id: DivisionDefinitionId(0),
            name: "Standard Infantry".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 10_000,
            required_equipment: 1000.0,
            recruitment_days: 30,
            movement_speed: 1.0,
            attack: 10.0,
            defense: 10.0,
            breakthrough: 5.0,
            organization: 50.0,
            morale: 50.0,
            supply_usage: usage,
            maintenance_cost: 5.0,
        }
    }

    fn division(id: usize, owner: CountryId, manpower: u64, def_id: usize) -> Division {
        Division {
            id: DivisionId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(0),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower,
            max_manpower: 10_000,
            equipment: 1000.0,
            max_equipment: 1000.0,
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

    fn military(divisions: Vec<Division>, defs: Vec<DivisionDefinition>) -> MilitaryRegistry {
        let mut m = MilitaryRegistry::default();
        for d in defs {
            m.definitions.insert(d.id, d);
        }
        for d in divisions {
            m.divisions.insert(d.id, d);
        }
        m
    }

    fn country(queue: usize) -> CountryData {
        let mut c = CountryData {
            id: C,
            ..Default::default()
        };
        for _ in 0..queue {
            c.recruitment_queue.push(RecruitmentQueueItem {
                division_id: DivisionDefinitionId(0),
                target_state: StateId(0),
                days_remaining: 10,
                total_days: 30,
            });
        }
        // 備蓄は判定に使われない(大量にあっても能力は増えない)。
        c.stockpile.set(ResourceType::MilitarySupplies, 1_000_000.0);
        c
    }

    const NOW: (i32, u8) = (1800, 4);

    /// 直近 4 か月の実績が各 `units`(units/月)の履歴(最後の月が NOW)。
    fn history_of(units: [Option<u64>; 4]) -> SupplyProductionHistory {
        let mut h = SupplyProductionHistory::default();
        for (i, u) in units.into_iter().enumerate() {
            h.record(1800, i as u8 + 1, u);
        }
        h
    }

    /// 履歴が十分に大きい(平均が上限にならない)場合の判定 = 現在設備の推定だけで決まる。
    fn gate(
        c: &CountryData,
        m: &MilitaryRegistry,
        s: &StateRegistry,
    ) -> ProductionCapacityVerdict {
        gate_with(c, m, s, &history_of([Some(10_000_000_000); 4]), NOW)
    }

    fn gate_with(
        c: &CountryData,
        m: &MilitaryRegistry,
        s: &StateRegistry,
        h: &SupplyProductionHistory,
        now: (i32, u8),
    ) -> ProductionCapacityVerdict {
        evaluate_production_capacity_gate(c, m, s, &buildings(), h, now, &def(1.0)).unwrap()
    }

    #[test]
    fn capacity_is_rebuilt_from_the_recorded_operation_not_the_theoretical_maximum() {
        let states = StateRegistry::build(vec![
            mf_state(0, C, 2, Some(0.25)),
            mf_state(1, CountryId(9), 5, Some(1.0)), // 他国の工場は数えない
        ]);
        let s = sustained_monthly_supply(C, &states, &buildings());
        assert_eq!(s.monthly_units, 50_000_000, "100 × Lv2 × 0.25 = 50.0");
        assert_eq!(s.unreported_entries, 0);
    }

    #[test]
    fn unreported_is_distinguished_from_a_recorded_zero_and_never_counts_as_unlimited() {
        let states = StateRegistry::build(vec![
            mf_state(0, C, 1, None),      // 未集計
            mf_state(1, C, 3, Some(0.0)), // 生産 0
            mf_state(2, C, 0, None),      // 工場なし
        ]);
        let s = sustained_monthly_supply(C, &states, &buildings());
        assert_eq!(s.unreported_entries, 1);
        assert_eq!(s.monthly_units, 0);
        // 未集計・生産 0 のどちらでも、2 個を超える追加募兵は許可されない。
        let m = military(
            vec![division(0, C, 10_000, 0), division(1, C, 10_000, 0)],
            vec![def(1.0)],
        );
        let v = gate(&country(0), &m, &states);
        assert!(!v.allowed);
        assert_eq!(v.unreported_entries, 1);
    }

    #[test]
    fn daily_demand_is_converted_with_the_calendar_average_month() {
        // 1.0/日 → 365/12 = 30.41666… → 切り上げ。
        assert_eq!(monthly_units_from_daily(1_000_000), Some(30_416_667));
        assert_eq!(monthly_units_from_daily(0), Some(0));
        assert_eq!(monthly_units_from_daily(u128::MAX), None);
    }

    #[test]
    fn the_boundary_is_inclusive_and_one_unit_short_is_held() {
        let m = military(
            vec![division(0, C, 10_000, 0), division(1, C, 10_000, 0)],
            vec![def(1.0)],
        );
        // 3 個の将来需要 = 3.0/日 → 91_250_000 units/月。
        let required = monthly_units_from_daily(3_000_000).unwrap();
        assert_eq!(required, 91_250_000);
        // Level 1 × 100 × op: op = 0.9125 で等号。
        let eq = StateRegistry::build(vec![mf_state(0, C, 1, Some(0.9125))]);
        let v = gate(&country(0), &m, &eq);
        assert_eq!(v.required_monthly_units, required);
        assert!(v.capacity_monthly_units >= required - 1);
        // 十分(0.92)なら許可、0.91 なら保留。
        assert!(gate(&country(0), &m, &StateRegistry::build(vec![mf_state(0, C, 1, Some(0.92))])).allowed);
        assert!(!gate(&country(0), &m, &StateRegistry::build(vec![mf_state(0, C, 1, Some(0.91))])).allowed);
    }

    #[test]
    fn the_training_queue_is_counted_and_orders_do_not_double_count() {
        let states = StateRegistry::build(vec![mf_state(0, C, 1, Some(1.0))]); // 100.0/月
        let m = military(vec![division(0, C, 10_000, 0)], vec![def(1.0)]);
        // 生存 1 + 訓練中 1 + 新規 1 = 3.0/日 → 91.25 ≤ 100 → 許可(ただし最小常備戦力 2 を超える)。
        let v = gate(&country(1), &m, &states);
        assert_eq!(v.required_monthly_units, 91_250_000);
        assert!(v.allowed);
        // 生存 1 + 訓練中 3 + 新規 1 = 5.0/日 → 152.08 > 100 → 保留。
        let v = gate(&country(3), &m, &states);
        assert_eq!(v.required_monthly_units, monthly_units_from_daily(5_000_000).unwrap());
        assert!(!v.allowed);
        // 訓練中の注文を後続の判定へ反映する: 1 つずつ増やしていくと、上限の手前で止まる
        // (100.0/月 ÷ 30.4167 = 3.28 個 → 将来の師団数 3 まで)。
        let mut allowed_total = 1;
        for queued in 0..6 {
            if gate(&country(queued), &m, &states).allowed {
                allowed_total = 1 + queued + 1;
            } else {
                break;
            }
        }
        assert_eq!(allowed_total, 3);
    }

    #[test]
    fn a_minimum_standing_force_is_exempt_but_the_third_division_is_not() {
        let states = StateRegistry::build(vec![mf_state(0, C, 1, Some(0.0))]); // 生産 0
        // 生存 0・1 のとき(+ 新規で 1・2 個)は免除。
        let m0 = military(vec![], vec![def(1.0)]);
        assert!(gate(&country(0), &m0, &states).within_minimum_standing_force);
        assert!(gate(&country(0), &m0, &states).allowed);
        let m1 = military(vec![division(0, C, 10_000, 0)], vec![def(1.0)]);
        assert!(gate(&country(0), &m1, &states).allowed);
        // 生存 2 では免除されず、生産 0 なら保留。
        let m2 = military(
            vec![division(0, C, 10_000, 0), division(1, C, 10_000, 0)],
            vec![def(1.0)],
        );
        let v = gate(&country(0), &m2, &states);
        assert!(!v.within_minimum_standing_force);
        assert!(!v.allowed);
        // 生存 1 + 訓練中 1 でも 2 個として数える。
        assert!(!gate(&country(1), &m1, &states).allowed);
    }

    #[test]
    fn damaged_divisions_count_at_full_strength_and_each_definition_uses_its_own_usage() {
        // 損耗中(兵力 1)の師団も、満員時の需要で数える。定義 1 は需要 2.0/日。
        let m = military(
            vec![division(0, C, 1, 0), division(1, C, 10_000, 1)],
            vec![def(1.0), {
                let mut d = def(2.0);
                d.id = DivisionDefinitionId(1);
                d
            }],
        );
        let p = projected_daily_demand_units(&country(0), &m).unwrap();
        assert_eq!(p, 3_000_000, "1.0(損耗中でも満員時)+ 2.0");
    }

    #[test]
    fn an_empty_or_dead_force_has_zero_projected_demand() {
        let m = military(vec![], vec![def(1.0)]);
        assert_eq!(projected_daily_demand_units(&country(0), &m).unwrap(), 0);
        let mut dead = division(0, C, 0, 0);
        dead.status = DivisionStatus::Destroyed;
        let m = military(vec![dead], vec![def(1.0)]);
        assert_eq!(projected_daily_demand_units(&country(0), &m).unwrap(), 0);
    }
    // ── P29-005: 複数月の実産出に基づく能力上限 ─────────────────────────

    fn m_two_standing() -> MilitaryRegistry {
        military(
            vec![division(0, C, 10_000, 0), division(1, C, 10_000, 0)],
            vec![def(1.0)],
        )
    }

    /// 現在設備の推定は required(3.0/日 → 92 /月)ちょうど。
    fn states_at_required() -> StateRegistry {
        StateRegistry::build(vec![mf_state(0, C, 1, Some(0.92))])
    }

    #[test]
    fn a_high_current_month_is_held_when_the_period_average_is_too_low() {
        let m = m_two_standing();
        let s = states_at_required();
        assert!(gate(&country(0), &m, &s).allowed, "単月(現在設備)の判定なら許可される");
        // 直近 4 か月: 高稼働 2 か月 + 生産 0 の 2 か月 → 平均 46e6 < 92e6。
        let h = history_of([Some(92_000_000), Some(92_000_000), Some(0), Some(0)]);
        let v = gate_with(&country(0), &m, &s, &h, NOW);
        assert_eq!(v.period_average_units, Some(46_000_000));
        assert!(v.current_estimate_units.abs_diff(92_000_000) <= 10, "f32 の丸め誤差のみ");
        assert_eq!(v.capacity_monthly_units, 46_000_000, "小さい方が上限");
        assert!(!v.allowed);
    }

    #[test]
    fn a_stable_history_with_headroom_allows_recruitment() {
        let m = m_two_standing();
        let s = states_at_required();
        let h = history_of([Some(92_000_000); 4]);
        let v = gate_with(&country(0), &m, &s, &h, NOW);
        assert!(v.allowed);
        assert_eq!(v.capacity_monthly_units, 92_000_000);
    }

    #[test]
    fn the_current_estimate_caps_a_good_history_after_a_permanent_cut() {
        let m = m_two_standing();
        // 設備は縮小済み(推定 46)。過去 4 か月の平均は 200 でも、現在設備の推定が上限。
        let s = StateRegistry::build(vec![mf_state(0, C, 1, Some(0.46))]);
        let h = history_of([Some(200_000_000); 4]);
        let v = gate_with(&country(0), &m, &s, &h, NOW);
        assert_eq!(v.period_average_units, Some(200_000_000));
        assert_eq!(v.capacity_monthly_units, v.current_estimate_units);
        assert!(v.current_estimate_units.abs_diff(46_000_000) <= 10);
        assert!(!v.allowed);
    }

    #[test]
    fn zero_production_months_are_part_of_the_average() {
        let m = m_two_standing();
        let s = StateRegistry::build(vec![mf_state(0, C, 5, Some(1.0))]); // 推定 500
        let h = history_of([Some(0), Some(0), Some(0), Some(0)]);
        let v = gate_with(&country(0), &m, &s, &h, NOW);
        assert_eq!(v.period_average_units, Some(0));
        assert!(!v.allowed);
    }

    #[test]
    fn an_incomplete_stale_or_unavailable_history_holds_the_normal_path_only() {
        let m = m_two_standing();
        let s = StateRegistry::build(vec![mf_state(0, C, 5, Some(1.0))]);
        let big = Some(10_000_000_000);
        // 3 か月分しかない
        let mut short = SupplyProductionHistory::default();
        for mth in 2..=4 {
            short.record(1800, mth, big);
        }
        let v = gate_with(&country(0), &m, &s, &short, NOW);
        assert_eq!(v.period_average_units, None);
        assert_eq!(v.capacity_monthly_units, 0);
        assert!(!v.allowed);
        // 古い履歴(最後の月が今月でない)
        let v = gate_with(&country(0), &m, &s, &history_of([big; 4]), (1800, 7));
        assert!(!v.allowed);
        // 集計失敗の月を含む(0 とは区別され、平均は出ない)
        let v = gate_with(&country(0), &m, &s, &history_of([big, None, big, big]), NOW);
        assert_eq!(v.period_average_units, None);
        assert!(!v.allowed);
        // 最小常備戦力の免除は履歴に関係なく、既存条件のまま使える。
        let m0 = military(vec![], vec![def(1.0)]);
        let v = gate_with(&country(0), &m0, &s, &SupplyProductionHistory::default(), NOW);
        assert!(v.within_minimum_standing_force);
        assert!(v.allowed);
        // 3 個目以降は(履歴なしでは)免除されない。
        let v = gate_with(&country(0), &m, &s, &SupplyProductionHistory::default(), NOW);
        assert!(!v.within_minimum_standing_force);
        assert!(!v.allowed);
    }

    #[test]
    fn sequential_orders_in_one_update_cannot_exceed_the_period_cap() {
        let s = StateRegistry::build(vec![mf_state(0, C, 10, Some(1.0))]); // 推定 1000
        let m = m_two_standing();
        // 平均 100e6 → 3.0/日(92)は許可、4.0/日(122)は不許可。キュー 1 件で判定が変わる。
        let h = history_of([Some(100_000_000); 4]);
        assert!(gate_with(&country(0), &m, &s, &h, NOW).allowed);
        assert!(!gate_with(&country(1), &m, &s, &h, NOW).allowed);
    }
}
