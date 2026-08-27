//! P25-002/P25-002-REV-001: 軍事装備会計基盤。
//!
//! P25-001の設計監査が確定したとおり、新しい`EquipmentKind`や新しい`ResourceType`は
//! 追加せず、既存の3つのフィールドをそのまま正規の装備会計として再利用する。
//!
//! ```text
//! 国家備蓄: economy::resources::CountryStockpile[ResourceType::MilitaryEquipment]
//! Division現在装備: military::data::Division.equipment
//! Division装備上限: military::data::Division.max_equipment
//! ```
//!
//! **P25-002-REV-001**: P25-002当初の実装は`f64`を直接加減算していたため、
//! `2^53`付近の規模で原子性・保存則を保証できなかった(例:
//! `stockpile = 2^53, equipment = 1, return = 1`のとき、`2^53 + 1`が`f64`で
//! 表現できず装備1個が消滅し得る)。本モジュールは、保存上の型(`f64`)は維持しつつ、
//! 会計の真実源を正規の整数units(`economy::resources::
//! military_equipment_value_to_units`/`military_equipment_units_to_value`、
//! `MILITARY_EQUIPMENT_SCALE`分の1単位)へ切り替えた。全ての操作は
//! 「1. f64入力を正規unitsへ変換 → 2. units上でvalidation → 3. units上で
//! checked演算 → 4. units上で保存則を確認 → 5. 成功後だけf64へ戻す →
//! 6. 往復不変条件を再確認 → 7. すべて成功した場合だけmutate」という順で処理する。
//!
//! `f64 → units`/`units → f64`の変換自体は`economy::resources`にある
//! (`CountryStockpile`が既に属する中立的な下位モジュールであり、`economy`側の
//! 生産経路[`economy::production`が呼ぶ`CountryStockpile::
//! add_military_equipment_produced`]と本モジュールの両方が同じ関数を共有するため、
//! 計算式を複製しない。`economy`が`military`へ依存する向きの新しい循環依存を
//! 作らないための配置でもある)。
//!
//! 本モジュールは、依然として募兵(`military::recruitment`)・日次補充
//! (`military::reinforcement`)・戦闘損失(`military::update`/`combat_calc`)・
//! 復員(`military::demobilization`)のいずれからも呼び出されない
//! (`DivisionDefinition.required_equipment`は依然として`recruitment.rs`側で
//! 財務コストとして使われたままであり、この意味の重複はP25-003で解消する)。
//! P25-002-REV-001で唯一新たに本番接続したのは、`economy::production`の
//! `MilitaryFactory`生産加算のみである(§12参照)。

use crate::economy::resources::{
    CountryStockpile, MilitaryEquipmentUnitsError, ResourceType, military_equipment_units_to_value,
    military_equipment_value_to_units,
};

/// 装備会計操作が失敗した理由。呼び出し元(将来のP25-003以降・テスト)が理由ごとに
/// 安全に分岐できるよう、意味のある区別を個別のバリアントとして表現する
/// (`military::demobilization::DemobilizationError`と同じ設計方針)。
///
/// P25-002-REV-001で、以前`InvalidEquipmentState`に一括されていたNaN/負値/値域超過を
/// より細かいバリアントへ分離した(§10)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EquipmentAccountingError {
    /// 操作前のDivision装備状態が不正(`equipment > max_equipment`)。
    InvalidEquipmentState,
    /// 国家備蓄が要求量に対して不足している。
    InsufficientStockpile,
    /// 支給後の装備量がDivisionの上限(`max_equipment`)を超える。
    DivisionCapacityExceeded,
    /// 返却量がDivisionの現在装備量を超える。
    ReturnAmountExceeded,
    /// 破壊量がDivisionの現在装備量を超える。
    DestroyAmountExceeded,
    /// units演算がoverflowした(正規最大unitsを超えた等)。
    ArithmeticOverflow,
    /// 値がNaNまたは無限大。
    NonFiniteQuantity,
    /// 値が負。
    NegativeQuantity,
    /// 量子化後のunitsが正規値域(`MILITARY_EQUIPMENT_MAX_UNITS`)を超える。
    QuantityOutOfRange,
    /// `units → f64 → units`の往復変換が一致しなかった。
    FixedPointRoundTripFailed,
}

impl From<MilitaryEquipmentUnitsError> for EquipmentAccountingError {
    fn from(err: MilitaryEquipmentUnitsError) -> Self {
        match err {
            MilitaryEquipmentUnitsError::NonFinite => EquipmentAccountingError::NonFiniteQuantity,
            MilitaryEquipmentUnitsError::Negative => EquipmentAccountingError::NegativeQuantity,
            MilitaryEquipmentUnitsError::OutOfRange => EquipmentAccountingError::QuantityOutOfRange,
            MilitaryEquipmentUnitsError::RoundTripFailed => {
                EquipmentAccountingError::FixedPointRoundTripFailed
            }
        }
    }
}

/// `f64`のMilitaryEquipment数量を正規unitsへ変換する(本モジュール内のショートハンド)。
fn to_units(value: f64) -> Result<u64, EquipmentAccountingError> {
    Ok(military_equipment_value_to_units(value)?)
}

/// 正規unitsを`f64`へ戻す(本モジュール内のショートハンド)。
fn to_value(units: u64) -> f64 {
    military_equipment_units_to_value(units)
}

/// 支給・返却が成功した際の結果(国家備蓄とDivision装備の両方が動く操作用)。
/// `stockpile_before`/`equipment_before`は正規化(量子化)後の値であり、演算前の
/// 生の`f64`入力そのものとは、量子化差分(§8)がある場合はビット単位で一致しない
/// ことがある(その場合は`quantization_delta`で差分を確認できる)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquipmentTransferResult {
    pub transferred: f64,
    pub stockpile_before: f64,
    pub stockpile_after: f64,
    pub equipment_before: f64,
    pub equipment_after: f64,
}

/// 破壊が成功した際の結果(国家備蓄には触れないDivision単独の操作用)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquipmentDestructionResult {
    pub destroyed: f64,
    pub equipment_before: f64,
    pub equipment_after: f64,
}

/// 与えられた生の`f64`値を正規化(量子化)した際の差分(`raw_value - canonical_value`)を
/// 返す診断用関数(P25-002-REV-001 §8)。値がscale格子上に完全一致する場合は`0.0`。
pub fn quantization_delta(raw_value: f64) -> Result<f64, EquipmentAccountingError> {
    let units = to_units(raw_value)?;
    Ok(raw_value - to_value(units))
}

/// Division装備状態(`0 <= equipment <= max_equipment`)を、正規units上で検証する。
pub fn validate_equipment_state(
    equipment: f64,
    max_equipment: f64,
) -> Result<(), EquipmentAccountingError> {
    let equipment_units = to_units(equipment)?;
    let max_units = to_units(max_equipment)?;
    if equipment_units > max_units {
        return Err(EquipmentAccountingError::InvalidEquipmentState);
    }
    Ok(())
}

/// 不足量(`max_equipment - equipment`)を、正規units上で計算してから`f64`へ戻す。
/// 無効状態は`0`へ黙って丸めず、`Err`として明示的に伝える。
pub fn equipment_shortfall(
    equipment: f64,
    max_equipment: f64,
) -> Result<f64, EquipmentAccountingError> {
    let equipment_units = to_units(equipment)?;
    let max_units = to_units(max_equipment)?;
    if equipment_units > max_units {
        return Err(EquipmentAccountingError::InvalidEquipmentState);
    }
    Ok(to_value(max_units - equipment_units))
}

/// `DivisionDefinition.required_equipment`が、Division生成時の装備上限として
/// 正規units上で有効な値かどうかを検証する純粋関数。P25-002/REV-001の時点では
/// `recruitment.rs`から呼び出されない(§23、P25-003で接続する)。
pub fn validate_required_equipment(
    required_equipment: f64,
) -> Result<(), EquipmentAccountingError> {
    to_units(required_equipment)?;
    Ok(())
}

/// 国家備蓄(`CountryStockpile[MilitaryEquipment]`)からDivisionへ`requested`だけ
/// 正規units上で原子的に支給する。全ての前提条件を満たす場合にのみ両方の値を
/// mutateする——途中で失敗した場合はどちらの元の`f64`値もビット単位で不変。
/// `requested = 0.0`は成功するno-opとして扱う。
pub fn try_issue_equipment(
    stockpile: &mut CountryStockpile,
    equipment: &mut f64,
    max_equipment: f64,
    requested: f64,
) -> Result<EquipmentTransferResult, EquipmentAccountingError> {
    // 1. 全f64入力を正規unitsへ変換
    let equipment_units_before = to_units(*equipment)?;
    let max_units = to_units(max_equipment)?;
    let requested_units = to_units(requested)?;
    let stockpile_units_before = to_units(stockpile.get(ResourceType::MilitaryEquipment))?;

    // 2. units上で状態validation
    if equipment_units_before > max_units {
        return Err(EquipmentAccountingError::InvalidEquipmentState);
    }

    // 3. units上でchecked arithmetic
    let stockpile_units_after = stockpile_units_before
        .checked_sub(requested_units)
        .ok_or(EquipmentAccountingError::InsufficientStockpile)?;
    let equipment_units_after = equipment_units_before
        .checked_add(requested_units)
        .ok_or(EquipmentAccountingError::ArithmeticOverflow)?;
    if equipment_units_after > max_units {
        return Err(EquipmentAccountingError::DivisionCapacityExceeded);
    }

    // 4. units上で保存則を確認(構成上自明だが、defense-in-depthとして明示的に確認する)
    debug_assert_eq!(
        stockpile_units_before + equipment_units_before,
        stockpile_units_after + equipment_units_after
    );

    // 5. 成功後だけf64へ戻す
    let stockpile_value_after = to_value(stockpile_units_after);
    let equipment_value_after = to_value(equipment_units_after);

    // 6. 往復不変条件を再確認
    if to_units(stockpile_value_after)? != stockpile_units_after
        || to_units(equipment_value_after)? != equipment_units_after
    {
        return Err(EquipmentAccountingError::FixedPointRoundTripFailed);
    }

    // 7. すべて成功した場合だけmutate
    let equipment_value_before = to_value(equipment_units_before);
    stockpile.set(ResourceType::MilitaryEquipment, stockpile_value_after);
    *equipment = equipment_value_after;

    Ok(EquipmentTransferResult {
        transferred: to_value(requested_units),
        stockpile_before: to_value(stockpile_units_before),
        stockpile_after: stockpile_value_after,
        equipment_before: equipment_value_before,
        equipment_after: equipment_value_after,
    })
}

/// Divisionから国家備蓄(`CountryStockpile[MilitaryEquipment]`)へ`returned`だけ
/// 正規units上で原子的に返却する。`returned = 0.0`は成功するno-opとして扱う。
pub fn try_return_equipment(
    stockpile: &mut CountryStockpile,
    equipment: &mut f64,
    returned: f64,
) -> Result<EquipmentTransferResult, EquipmentAccountingError> {
    let equipment_units_before = to_units(*equipment)?;
    let returned_units = to_units(returned)?;
    let stockpile_units_before = to_units(stockpile.get(ResourceType::MilitaryEquipment))?;

    if returned_units > equipment_units_before {
        return Err(EquipmentAccountingError::ReturnAmountExceeded);
    }

    let equipment_units_after = equipment_units_before - returned_units;
    let stockpile_units_after = stockpile_units_before
        .checked_add(returned_units)
        .ok_or(EquipmentAccountingError::ArithmeticOverflow)?;

    debug_assert_eq!(
        stockpile_units_before + equipment_units_before,
        stockpile_units_after + equipment_units_after
    );

    let stockpile_value_after = to_value(stockpile_units_after);
    let equipment_value_after = to_value(equipment_units_after);

    if to_units(stockpile_value_after)? != stockpile_units_after
        || to_units(equipment_value_after)? != equipment_units_after
    {
        return Err(EquipmentAccountingError::FixedPointRoundTripFailed);
    }

    let equipment_value_before = to_value(equipment_units_before);
    stockpile.set(ResourceType::MilitaryEquipment, stockpile_value_after);
    *equipment = equipment_value_after;

    Ok(EquipmentTransferResult {
        transferred: to_value(returned_units),
        stockpile_before: to_value(stockpile_units_before),
        stockpile_after: stockpile_value_after,
        equipment_before: equipment_value_before,
        equipment_after: equipment_value_after,
    })
}

/// Divisionの装備`destroyed`分を正規units上で原子的に破壊する。国家備蓄には
/// 一切触れない(破壊された装備は備蓄へ戻らない)。`destroyed = 0.0`は成功する
/// no-opとして扱う。
pub fn try_destroy_equipment(
    equipment: &mut f64,
    destroyed: f64,
) -> Result<EquipmentDestructionResult, EquipmentAccountingError> {
    let equipment_units_before = to_units(*equipment)?;
    let destroyed_units = to_units(destroyed)?;

    if destroyed_units > equipment_units_before {
        return Err(EquipmentAccountingError::DestroyAmountExceeded);
    }
    let equipment_units_after = equipment_units_before - destroyed_units;

    let equipment_value_after = to_value(equipment_units_after);
    if to_units(equipment_value_after)? != equipment_units_after {
        return Err(EquipmentAccountingError::FixedPointRoundTripFailed);
    }

    let equipment_value_before = to_value(equipment_units_before);
    *equipment = equipment_value_after;

    Ok(EquipmentDestructionResult {
        destroyed: to_value(destroyed_units),
        equipment_before: equipment_value_before,
        equipment_after: equipment_value_after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stockpile_with(amount: f64) -> CountryStockpile {
        let mut s = CountryStockpile::new();
        s.set(ResourceType::MilitaryEquipment, amount);
        s
    }

    // ─── validate_equipment_state ──────────────────────────────────────────

    #[test]
    fn validate_accepts_zero_equipment_and_zero_max() {
        assert_eq!(validate_equipment_state(0.0, 0.0), Ok(()));
    }

    #[test]
    fn validate_accepts_equipment_below_max() {
        assert_eq!(validate_equipment_state(50.0, 100.0), Ok(()));
    }

    #[test]
    fn validate_accepts_equipment_equal_to_max() {
        assert_eq!(validate_equipment_state(100.0, 100.0), Ok(()));
    }

    #[test]
    fn validate_rejects_equipment_above_max() {
        assert_eq!(
            validate_equipment_state(101.0, 100.0),
            Err(EquipmentAccountingError::InvalidEquipmentState)
        );
    }

    #[test]
    fn validate_rejects_negative_equipment() {
        assert_eq!(
            validate_equipment_state(-1.0, 100.0),
            Err(EquipmentAccountingError::NegativeQuantity)
        );
    }

    #[test]
    fn validate_rejects_non_finite_values() {
        assert_eq!(
            validate_equipment_state(f64::NAN, 100.0),
            Err(EquipmentAccountingError::NonFiniteQuantity)
        );
        assert_eq!(
            validate_equipment_state(50.0, f64::INFINITY),
            Err(EquipmentAccountingError::NonFiniteQuantity)
        );
    }

    // ─── equipment_shortfall ────────────────────────────────────────────────

    #[test]
    fn shortfall_computes_the_difference() {
        assert_eq!(equipment_shortfall(40.0, 100.0), Ok(60.0));
    }

    #[test]
    fn shortfall_is_zero_when_fully_equipped() {
        assert_eq!(equipment_shortfall(100.0, 100.0), Ok(0.0));
    }

    #[test]
    fn shortfall_does_not_silently_hide_invalid_state_as_zero() {
        assert_eq!(
            equipment_shortfall(150.0, 100.0),
            Err(EquipmentAccountingError::InvalidEquipmentState)
        );
    }

    // ─── validate_required_equipment ───────────────────────────────────────

    #[test]
    fn required_equipment_accepts_non_negative_finite_values() {
        assert_eq!(validate_required_equipment(0.0), Ok(()));
        assert_eq!(validate_required_equipment(100.0), Ok(()));
    }

    #[test]
    fn required_equipment_rejects_negative_or_non_finite_values() {
        assert_eq!(
            validate_required_equipment(-1.0),
            Err(EquipmentAccountingError::NegativeQuantity)
        );
        assert_eq!(
            validate_required_equipment(f64::NAN),
            Err(EquipmentAccountingError::NonFiniteQuantity)
        );
    }

    // ─── try_issue_equipment ────────────────────────────────────────────────

    #[test]
    fn issue_fully_equips_a_division_from_stockpile() {
        let mut stockpile = stockpile_with(100.0);
        let mut equipment = 0.0;
        let result = try_issue_equipment(&mut stockpile, &mut equipment, 100.0, 100.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 0.0);
        assert_eq!(equipment, 100.0);
        assert_eq!(
            result,
            EquipmentTransferResult {
                transferred: 100.0,
                stockpile_before: 100.0,
                stockpile_after: 0.0,
                equipment_before: 0.0,
                equipment_after: 100.0,
            }
        );
        // 総量保存則
        assert_eq!(100.0 + 0.0, result.stockpile_after + result.equipment_after);
    }

    #[test]
    fn issue_partially_reinforces_a_division() {
        let mut stockpile = stockpile_with(100.0);
        let mut equipment = 20.0;
        try_issue_equipment(&mut stockpile, &mut equipment, 100.0, 30.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 70.0);
        assert_eq!(equipment, 50.0);
    }

    #[test]
    fn issue_fails_atomically_when_stockpile_is_insufficient() {
        let mut stockpile = stockpile_with(29.0);
        let mut equipment = 20.0;
        let err = try_issue_equipment(&mut stockpile, &mut equipment, 100.0, 30.0).unwrap_err();
        assert_eq!(err, EquipmentAccountingError::InsufficientStockpile);
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 29.0);
        assert_eq!(equipment, 20.0);
    }

    #[test]
    fn issue_fails_atomically_when_division_capacity_is_exceeded() {
        let mut stockpile = stockpile_with(100.0);
        let mut equipment = 80.0;
        let err = try_issue_equipment(&mut stockpile, &mut equipment, 100.0, 21.0).unwrap_err();
        assert_eq!(err, EquipmentAccountingError::DivisionCapacityExceeded);
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 100.0);
        assert_eq!(equipment, 80.0);
    }

    #[test]
    fn issue_rejects_invalid_starting_division_state() {
        let mut stockpile = stockpile_with(100.0);
        let mut equipment = 150.0;
        let err = try_issue_equipment(&mut stockpile, &mut equipment, 100.0, 10.0).unwrap_err();
        assert_eq!(err, EquipmentAccountingError::InvalidEquipmentState);
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 100.0);
        assert_eq!(equipment, 150.0);
    }

    #[test]
    fn issue_of_zero_is_a_deterministic_no_op() {
        let mut stockpile = stockpile_with(100.0);
        let mut equipment = 20.0;
        let result = try_issue_equipment(&mut stockpile, &mut equipment, 100.0, 0.0).unwrap();
        assert_eq!(result.transferred, 0.0);
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 100.0);
        assert_eq!(equipment, 20.0);
    }

    // ─── try_return_equipment ───────────────────────────────────────────────

    #[test]
    fn return_moves_equipment_back_to_stockpile() {
        let mut stockpile = stockpile_with(10.0);
        let mut equipment = 40.0;
        let result = try_return_equipment(&mut stockpile, &mut equipment, 25.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 35.0);
        assert_eq!(equipment, 15.0);
        // 総量保存則
        assert_eq!(10.0 + 40.0, result.stockpile_after + result.equipment_after);
    }

    #[test]
    fn return_fails_atomically_when_amount_exceeds_current_equipment() {
        let mut stockpile = stockpile_with(10.0);
        let mut equipment = 40.0;
        let err = try_return_equipment(&mut stockpile, &mut equipment, 41.0).unwrap_err();
        assert_eq!(err, EquipmentAccountingError::ReturnAmountExceeded);
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 10.0);
        assert_eq!(equipment, 40.0);
    }

    #[test]
    fn return_of_zero_is_a_deterministic_no_op() {
        let mut stockpile = stockpile_with(10.0);
        let mut equipment = 40.0;
        let result = try_return_equipment(&mut stockpile, &mut equipment, 0.0).unwrap();
        assert_eq!(result.transferred, 0.0);
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 10.0);
        assert_eq!(equipment, 40.0);
    }

    #[test]
    fn return_rejects_out_of_range_quantities_without_mutating_either_value() {
        // f64::MAXは正規値域(MILITARY_EQUIPMENT_MAX_UNITS/SCALE ≈ 90億)を大幅に
        // 超えるため、units変換の入口で拒否される(P25-002-REV-001)。
        let mut stockpile = stockpile_with(f64::MAX);
        let mut equipment = f64::MAX;
        let err = try_return_equipment(&mut stockpile, &mut equipment, f64::MAX).unwrap_err();
        assert_eq!(err, EquipmentAccountingError::QuantityOutOfRange);
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), f64::MAX);
        assert_eq!(equipment, f64::MAX);
    }

    /// P25-002-REV-001の核心的な反例。旧実装(P25-002)は`f64`を直接加減算していたため、
    /// `stockpile=2^53, equipment=1, return=1`のとき`2^53 + 1`が`f64`で正確に
    /// 表現できず、装備1個が消滅する可能性があった。訂正後は、この規模の値が
    /// そもそも正規値域(現実的な到達可能量を大幅に超える)を超えるため、
    /// 曖昧な成功ではなく明確な`QuantityOutOfRange`拒否になることを確認する。
    #[test]
    fn two_pow_53_counterexample_is_rejected_not_silently_lossy() {
        let two_pow_53 = 2f64.powi(53);

        // 旧実装の挙動を再現(このテスト内だけの素朴なf64加算、本番コードではない):
        // 2^53 + 1 は f64 では 2^53 のまま(1が消える)。
        let naive_sum = two_pow_53 + 1.0;
        assert_eq!(
            naive_sum, two_pow_53,
            "demonstrates the raw f64 addition bug that P25-002 was vulnerable to"
        );

        // 訂正後の try_return_equipment は、この規模の値を正規値域外として
        // 原子的に拒否する(曖昧な成功をしない)。
        let mut stockpile = stockpile_with(two_pow_53);
        let mut equipment = 1.0;
        let result = try_return_equipment(&mut stockpile, &mut equipment, 1.0);
        assert!(
            result.is_err(),
            "2^53-scale quantities must be atomically rejected, not silently accepted with a lost unit"
        );
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), two_pow_53);
        assert_eq!(equipment, 1.0);
    }

    // ─── try_destroy_equipment ──────────────────────────────────────────────

    #[test]
    fn destroy_reduces_division_equipment_without_touching_stockpile() {
        let stockpile = stockpile_with(10.0);
        let mut equipment = 40.0;
        let result = try_destroy_equipment(&mut equipment, 15.0).unwrap();
        assert_eq!(equipment, 25.0);
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 10.0);
        assert_eq!(result.destroyed, 15.0);
        assert_eq!(result.equipment_before - result.equipment_after, 15.0);
    }

    #[test]
    fn destroy_fails_atomically_when_amount_exceeds_current_equipment() {
        let mut equipment = 40.0;
        let err = try_destroy_equipment(&mut equipment, 41.0).unwrap_err();
        assert_eq!(err, EquipmentAccountingError::DestroyAmountExceeded);
        assert_eq!(equipment, 40.0);
    }

    #[test]
    fn destroy_of_zero_is_a_deterministic_no_op() {
        let mut equipment = 40.0;
        let result = try_destroy_equipment(&mut equipment, 0.0).unwrap();
        assert_eq!(result.destroyed, 0.0);
        assert_eq!(equipment, 40.0);
    }

    // ─── 操作列の保存則・反復ドリフト ─────────────────────────────────────

    #[test]
    fn a_sequence_of_operations_preserves_the_world_equipment_total() {
        let mut stockpile = stockpile_with(1_000.0);
        let mut equipment = 0.0;
        let initial_total = stockpile.get(ResourceType::MilitaryEquipment) + equipment;

        try_issue_equipment(&mut stockpile, &mut equipment, 1_000.0, 600.0).unwrap();
        try_return_equipment(&mut stockpile, &mut equipment, 100.0).unwrap();
        let destroyed = try_destroy_equipment(&mut equipment, 200.0)
            .unwrap()
            .destroyed;

        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 500.0);
        assert_eq!(equipment, 300.0);
        assert_eq!(destroyed, 200.0);
        assert_eq!(
            initial_total,
            stockpile.get(ResourceType::MilitaryEquipment) + equipment + destroyed
        );
    }

    #[test]
    fn repeated_issue_return_cycles_do_not_drift() {
        let mut stockpile = stockpile_with(1_000_000.0);
        let mut equipment = 0.0;
        let initial_total = stockpile.get(ResourceType::MilitaryEquipment) + equipment;

        for _ in 0..100_000 {
            try_issue_equipment(&mut stockpile, &mut equipment, 1_000_000.0, 3.7).unwrap();
            try_return_equipment(&mut stockpile, &mut equipment, 3.7).unwrap();
        }

        assert_eq!(
            equipment, 0.0,
            "equipment must return to exactly zero after equal issue/return cycles"
        );
        assert_eq!(
            stockpile.get(ResourceType::MilitaryEquipment),
            initial_total,
            "stockpile must not drift after 100,000 issue/return cycles"
        );
    }

    // ─── quantization_delta ─────────────────────────────────────────────────

    #[test]
    fn quantization_delta_is_zero_for_grid_aligned_values() {
        assert_eq!(quantization_delta(0.45).unwrap(), 0.0);
        assert_eq!(quantization_delta(100.0).unwrap(), 0.0);
    }

    // ─── 国家間分離 ─────────────────────────────────────────────────────────

    #[test]
    fn operating_on_one_countrys_stockpile_never_touches_anothers() {
        let mut stockpile_a = stockpile_with(100.0);
        let stockpile_b = stockpile_with(100.0);
        let mut equipment_a = 0.0;

        try_issue_equipment(&mut stockpile_a, &mut equipment_a, 100.0, 60.0).unwrap();

        assert_eq!(stockpile_a.get(ResourceType::MilitaryEquipment), 40.0);
        assert_eq!(
            stockpile_b.get(ResourceType::MilitaryEquipment),
            100.0,
            "country B's stockpile must be untouched by an operation on country A's stockpile"
        );
    }
}
