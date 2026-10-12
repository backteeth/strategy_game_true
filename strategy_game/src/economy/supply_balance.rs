//! P29-002: MilitarySuppliesの「前回の月次生産」「現在の日次需要」「備蓄日数」の表示専用ビュー。
//!
//! 3項目はいずれも既存の正規値・正規関数から取得する(表示用に生産式・需要式を別実装しない)。
//!
//! | 項目 | 算定元 |
//! |---|---|
//! | 前回の月次生産 | `production::process_country_production_with_outcome`が備蓄へ実際に反映した量(FIX1・D4・入力不足・entryごとの量子化を反映)。`handle_monthly_economy`が[`MilitarySupplyProductionLedger`]へ記録する |
//! | 現在の需要 | `military::supply_initialization::country_daily_demand_units`(日次補給処理と同じ正規関数) |
//! | 備蓄日数 | 国家備蓄units ÷ 日次需要units(将来の月次生産は含めない) |
//!
//! ledgerは表示専用の非永続Resourceで、Save V1へは含めない。New Game(国家選択画面への
//! 進入)・ロード成功で空に戻る。ロード直後は次の月次生産まで「未集計」になる。

use crate::common::{CountryId, DivisionDefinitionId, DivisionId};
use crate::economy::resources::military_supplies_value_to_units;
use crate::military::data::{Division, DivisionDefinition};
use crate::military::supply_initialization::country_daily_demand_units;
use bevy::prelude::Resource;
use std::collections::HashMap;

/// 1回の月次生産で記録された実績。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MonthlyProductionRecord {
    /// 備蓄へ反映された量(0を含む)。
    Produced(f64),
    /// 集計またはcommitに失敗し、実績を確定できなかった(0ではない)。
    Unavailable,
}

/// 月次生産の実績と、その対象年月(月次生産が実行された`MonthChangedMessage`の年月)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonthlySupplyProduction {
    pub year: i32,
    pub month: u8,
    pub record: MonthlyProductionRecord,
}

/// 国家ごとの直近の月次生産実績(表示専用・非永続)。
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct MilitarySupplyProductionLedger {
    entries: HashMap<CountryId, MonthlySupplyProduction>,
}

impl MilitarySupplyProductionLedger {
    /// `produced`が`None`なら`Unavailable`として記録する(0とは区別する)。
    pub fn record(&mut self, country: CountryId, year: i32, month: u8, produced: Option<f64>) {
        let record = match produced {
            Some(v) if v.is_finite() && v >= 0.0 => MonthlyProductionRecord::Produced(v),
            _ => MonthlyProductionRecord::Unavailable,
        };
        self.entries.insert(
            country,
            MonthlySupplyProduction {
                year,
                month,
                record,
            },
        );
    }

    pub fn get(&self, country: CountryId) -> Option<&MonthlySupplyProduction> {
        self.entries.get(&country)
    }

    pub fn remove(&mut self, country: CountryId) {
        self.entries.remove(&country);
    }

    /// New Game・ロードで前のゲームの実績を残さないために使う。
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 備蓄日数の表示区分。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StockDays {
    /// 需要が0(消費なし)。
    NoConsumption,
    /// 現在の需要が続き、追加生産がない場合の備蓄日数。
    Days(f64),
    /// 需要または備蓄が正規変換できない(異常値)。
    Unavailable,
}

/// 表示用ビュー。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MilitarySupplyBalanceView {
    /// 国家のMilitarySupplies備蓄(表示値、丸めていない生の値)。
    pub stock: f64,
    /// 直近の月次生産実績。`None`は未集計(まだ月次生産を観測していない・ロード直後)。
    pub last_production: Option<MonthlySupplyProduction>,
    /// 現在の日次需要(MilitarySupplies/日)。算出できない場合は`None`。
    pub daily_demand: Option<f64>,
    pub stock_days: StockDays,
}

/// 表示用ビューを構築する(副作用なし)。需要は備蓄が不足していても0にしない
/// (`country_daily_demand_units`は備蓄を参照しない)。
pub fn build_military_supply_balance_view(
    country_id: CountryId,
    stock_value: f64,
    divisions: &HashMap<DivisionId, Division>,
    definitions: &HashMap<DivisionDefinitionId, DivisionDefinition>,
    ledger: &MilitarySupplyProductionLedger,
) -> MilitarySupplyBalanceView {
    let demand_units = country_daily_demand_units(country_id, divisions, definitions).ok();
    let daily_demand = demand_units.map(|u| u as f64 / crate::economy::resources::MILITARY_SUPPLIES_SCALE as f64);

    let stock_days = match (demand_units, military_supplies_value_to_units(stock_value)) {
        (Some(0), _) => StockDays::NoConsumption,
        (Some(demand), Ok(stock_units)) => StockDays::Days(stock_units as f64 / demand as f64),
        _ => StockDays::Unavailable,
    };

    MilitarySupplyBalanceView {
        stock: stock_value,
        last_production: ledger.get(country_id).copied(),
        daily_demand,
        stock_days,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::military::data::DivisionStatus;

    fn definition(id: usize, usage: f32) -> DivisionDefinition {
        DivisionDefinition {
            id: DivisionDefinitionId(id),
            name: "t".to_string(),
            division_type: crate::military::data::DivisionType::Infantry,
            size: crate::military::data::DivisionSize::Standard,
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

    fn division(id: usize, owner: usize, manpower: u64) -> Division {
        Division {
            id: DivisionId(id),
            owner: CountryId(owner),
            division_type: crate::military::data::DivisionType::Infantry,
            size: crate::military::data::DivisionSize::Standard,
            current_state: crate::common::StateId(0),
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
            def_id: DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn world(
        divisions: Vec<Division>,
    ) -> (
        HashMap<DivisionId, Division>,
        HashMap<DivisionDefinitionId, DivisionDefinition>,
    ) {
        let mut defs = HashMap::new();
        defs.insert(DivisionDefinitionId(0), definition(0, 1.0));
        (divisions.into_iter().map(|d| (d.id, d)).collect(), defs)
    }

    #[test]
    fn demand_is_reported_even_when_the_stockpile_is_empty() {
        let (divs, defs) = world(vec![division(0, 1, 10_000), division(1, 1, 5_000)]);
        let view = build_military_supply_balance_view(
            CountryId(1),
            0.0,
            &divs,
            &defs,
            &MilitarySupplyProductionLedger::default(),
        );
        assert_eq!(view.daily_demand, Some(1.5));
        assert_eq!(view.stock_days, StockDays::Days(0.0));
    }

    #[test]
    fn stock_days_is_stock_over_daily_demand_and_ignores_production() {
        let (divs, defs) = world(vec![division(0, 1, 10_000), division(1, 1, 10_000)]);
        let mut ledger = MilitarySupplyProductionLedger::default();
        ledger.record(CountryId(1), 1800, 3, Some(500.0));
        let view = build_military_supply_balance_view(CountryId(1), 180.0, &divs, &defs, &ledger);
        assert_eq!(view.daily_demand, Some(2.0));
        assert_eq!(view.stock_days, StockDays::Days(90.0));
        assert_eq!(
            view.last_production,
            Some(MonthlySupplyProduction {
                year: 1800,
                month: 3,
                record: MonthlyProductionRecord::Produced(500.0)
            })
        );
    }

    #[test]
    fn zero_demand_is_no_consumption_without_division_or_nan() {
        let (divs, defs) = world(vec![division(0, 2, 10_000)]);
        for stock in [0.0, 42.0] {
            let view = build_military_supply_balance_view(
                CountryId(1),
                stock,
                &divs,
                &defs,
                &MilitarySupplyProductionLedger::default(),
            );
            assert_eq!(view.daily_demand, Some(0.0));
            assert_eq!(view.stock_days, StockDays::NoConsumption);
        }
    }

    #[test]
    fn only_the_requested_countrys_living_divisions_are_counted() {
        let mut dead = division(2, 1, 0);
        dead.status = DivisionStatus::Destroyed;
        let (divs, defs) = world(vec![division(0, 1, 10_000), division(1, 2, 10_000), dead]);
        let view = build_military_supply_balance_view(
            CountryId(1),
            10.0,
            &divs,
            &defs,
            &MilitarySupplyProductionLedger::default(),
        );
        assert_eq!(view.daily_demand, Some(1.0));
        assert_eq!(view.stock_days, StockDays::Days(10.0));
    }

    #[test]
    fn invalid_stock_is_unavailable_not_nan() {
        let (divs, defs) = world(vec![division(0, 1, 10_000)]);
        for stock in [f64::NAN, f64::INFINITY, -1.0] {
            let view = build_military_supply_balance_view(
                CountryId(1),
                stock,
                &divs,
                &defs,
                &MilitarySupplyProductionLedger::default(),
            );
            assert_eq!(view.stock_days, StockDays::Unavailable);
        }
    }

    #[test]
    fn unavailable_production_is_distinct_from_zero_and_clear_empties_the_ledger() {
        let mut ledger = MilitarySupplyProductionLedger::default();
        ledger.record(CountryId(1), 1800, 2, Some(0.0));
        assert_eq!(
            ledger.get(CountryId(1)).unwrap().record,
            MonthlyProductionRecord::Produced(0.0)
        );
        ledger.record(CountryId(1), 1800, 3, None);
        assert_eq!(
            ledger.get(CountryId(1)).unwrap().record,
            MonthlyProductionRecord::Unavailable
        );
        ledger.record(CountryId(2), 1800, 3, Some(f64::NAN));
        assert_eq!(
            ledger.get(CountryId(2)).unwrap().record,
            MonthlyProductionRecord::Unavailable
        );
        ledger.clear();
        assert!(ledger.is_empty());
        assert!(ledger.get(CountryId(1)).is_none());
    }
}
