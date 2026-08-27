//! P25-002: 軍事装備会計基盤。
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
//! 本モジュールは、この3つの値を変更する唯一の正規APIを提供する。ただし、
//! P25-002の時点では募兵(`military::recruitment`)・日次補充
//! (`military::reinforcement`)・戦闘損失(`military::update`/`combat_calc`)・
//! 復員(`military::demobilization`)のいずれからもまだ呼び出されない
//! (`DivisionDefinition.required_equipment`は依然として`recruitment.rs`側で
//! 財務コストとして使われたままであり、この意味の重複はP25-003で解消する)。
//!
//! 数量型は既存フィールドに合わせて`f64`のまま扱う(新しい整数型を導入しない)。
//! `f64`にはRustの`checked_add`等が存在しないため、本モジュールでは「演算後の
//! 値が有限か(`is_finite()`)」を検査することをoverflow検出の代替とする——
//! 演算前に全ての前提条件を検査し、演算後の値も含めて全て有限かつ制約を満たす
//! 場合にのみ実際にmutateする(§7の「validate→checked演算→全成功時のみmutate」
//! パターンをf64向けに適用したもの)。

use crate::economy::resources::{CountryStockpile, ResourceType};

/// 装備会計操作が失敗した理由。呼び出し元(将来のP25-003以降・テスト)が理由ごとに
/// 安全に分岐できるよう、意味のある区別を個別のバリアントとして表現する
/// (`military::demobilization::DemobilizationError`と同じ設計方針)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EquipmentAccountingError {
    /// 操作前のDivision装備状態が不正(`equipment > max_equipment`、負値、非有限値)。
    InvalidEquipmentState,
    /// 国家備蓄が要求量に対して不足している。
    InsufficientStockpile,
    /// 支給後の装備量がDivisionの上限(`max_equipment`)を超える。
    DivisionCapacityExceeded,
    /// 返却量がDivisionの現在装備量を超える。
    ReturnAmountExceeded,
    /// 破壊量がDivisionの現在装備量を超える。
    DestroyAmountExceeded,
    /// 演算結果が有限でなくなった(overflow/NaN)、または要求量自体が負値・非有限値。
    ArithmeticOverflow,
}

/// 支給・返却が成功した際の結果(国家備蓄とDivision装備の両方が動く操作用)。
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

/// Division装備状態(`0 <= equipment <= max_equipment`、両方とも有限)を検証する。
pub fn validate_equipment_state(
    equipment: f64,
    max_equipment: f64,
) -> Result<(), EquipmentAccountingError> {
    if !equipment.is_finite() || !max_equipment.is_finite() {
        return Err(EquipmentAccountingError::InvalidEquipmentState);
    }
    if equipment < 0.0 || max_equipment < 0.0 {
        return Err(EquipmentAccountingError::InvalidEquipmentState);
    }
    if equipment > max_equipment {
        return Err(EquipmentAccountingError::InvalidEquipmentState);
    }
    Ok(())
}

/// 不足量(`max_equipment - equipment`)を計算する。無効状態は`0`へ黙って丸めず、
/// `Err`として明示的に伝える。
pub fn equipment_shortfall(
    equipment: f64,
    max_equipment: f64,
) -> Result<f64, EquipmentAccountingError> {
    validate_equipment_state(equipment, max_equipment)?;
    Ok(max_equipment - equipment)
}

/// `DivisionDefinition.required_equipment`が、Division生成時の装備上限として
/// 有効な値(有限・非負)かどうかを検証する純粋関数。P25-002の時点では
/// `recruitment.rs`から呼び出されない(§23、P25-003で接続する)。
pub fn validate_required_equipment(
    required_equipment: f64,
) -> Result<(), EquipmentAccountingError> {
    if !required_equipment.is_finite() || required_equipment < 0.0 {
        return Err(EquipmentAccountingError::InvalidEquipmentState);
    }
    Ok(())
}

fn validate_amount(amount: f64) -> Result<(), EquipmentAccountingError> {
    if !amount.is_finite() || amount < 0.0 {
        return Err(EquipmentAccountingError::ArithmeticOverflow);
    }
    Ok(())
}

/// 国家備蓄(`CountryStockpile[MilitaryEquipment]`)からDivisionへ`requested`だけ
/// 原子的に支給する。全ての前提条件を満たす場合にのみ両方の値をmutateする——
/// 途中で失敗した場合はどちらの値も変化しない。`requested = 0.0`は成功する
/// no-opとして扱う。
pub fn try_issue_equipment(
    stockpile: &mut CountryStockpile,
    equipment: &mut f64,
    max_equipment: f64,
    requested: f64,
) -> Result<EquipmentTransferResult, EquipmentAccountingError> {
    validate_equipment_state(*equipment, max_equipment)?;
    validate_amount(requested)?;

    let stockpile_before = stockpile.get(ResourceType::MilitaryEquipment);
    if !stockpile_before.is_finite() || stockpile_before < 0.0 {
        return Err(EquipmentAccountingError::InvalidEquipmentState);
    }

    let equipment_before = *equipment;
    let equipment_after = equipment_before + requested;
    let stockpile_after = stockpile_before - requested;

    if !equipment_after.is_finite() || !stockpile_after.is_finite() {
        return Err(EquipmentAccountingError::ArithmeticOverflow);
    }
    if stockpile_before < requested {
        return Err(EquipmentAccountingError::InsufficientStockpile);
    }
    if equipment_after > max_equipment {
        return Err(EquipmentAccountingError::DivisionCapacityExceeded);
    }

    stockpile.set(ResourceType::MilitaryEquipment, stockpile_after);
    *equipment = equipment_after;

    Ok(EquipmentTransferResult {
        transferred: requested,
        stockpile_before,
        stockpile_after,
        equipment_before,
        equipment_after,
    })
}

/// Divisionから国家備蓄(`CountryStockpile[MilitaryEquipment]`)へ`returned`だけ
/// 原子的に返却する。全ての前提条件を満たす場合にのみ両方の値をmutateする。
/// `returned = 0.0`は成功するno-opとして扱う。
pub fn try_return_equipment(
    stockpile: &mut CountryStockpile,
    equipment: &mut f64,
    returned: f64,
) -> Result<EquipmentTransferResult, EquipmentAccountingError> {
    if !equipment.is_finite() || *equipment < 0.0 {
        return Err(EquipmentAccountingError::InvalidEquipmentState);
    }
    validate_amount(returned)?;

    if returned > *equipment {
        return Err(EquipmentAccountingError::ReturnAmountExceeded);
    }

    let stockpile_before = stockpile.get(ResourceType::MilitaryEquipment);
    if !stockpile_before.is_finite() || stockpile_before < 0.0 {
        return Err(EquipmentAccountingError::InvalidEquipmentState);
    }

    let equipment_before = *equipment;
    let equipment_after = equipment_before - returned;
    let stockpile_after = stockpile_before + returned;

    if !equipment_after.is_finite() || !stockpile_after.is_finite() {
        return Err(EquipmentAccountingError::ArithmeticOverflow);
    }

    stockpile.set(ResourceType::MilitaryEquipment, stockpile_after);
    *equipment = equipment_after;

    Ok(EquipmentTransferResult {
        transferred: returned,
        stockpile_before,
        stockpile_after,
        equipment_before,
        equipment_after,
    })
}

/// Divisionの装備`destroyed`分を原子的に破壊する。国家備蓄には一切触れない
/// (破壊された装備は備蓄へ戻らない)。`destroyed = 0.0`は成功するno-opとして扱う。
pub fn try_destroy_equipment(
    equipment: &mut f64,
    destroyed: f64,
) -> Result<EquipmentDestructionResult, EquipmentAccountingError> {
    if !equipment.is_finite() || *equipment < 0.0 {
        return Err(EquipmentAccountingError::InvalidEquipmentState);
    }
    validate_amount(destroyed)?;

    if destroyed > *equipment {
        return Err(EquipmentAccountingError::DestroyAmountExceeded);
    }

    let equipment_before = *equipment;
    let equipment_after = equipment_before - destroyed;

    if !equipment_after.is_finite() {
        return Err(EquipmentAccountingError::ArithmeticOverflow);
    }

    *equipment = equipment_after;

    Ok(EquipmentDestructionResult {
        destroyed,
        equipment_before,
        equipment_after,
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
            Err(EquipmentAccountingError::InvalidEquipmentState)
        );
    }

    #[test]
    fn validate_rejects_non_finite_values() {
        assert_eq!(
            validate_equipment_state(f64::NAN, 100.0),
            Err(EquipmentAccountingError::InvalidEquipmentState)
        );
        assert_eq!(
            validate_equipment_state(50.0, f64::INFINITY),
            Err(EquipmentAccountingError::InvalidEquipmentState)
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
            Err(EquipmentAccountingError::InvalidEquipmentState)
        );
        assert_eq!(
            validate_required_equipment(f64::NAN),
            Err(EquipmentAccountingError::InvalidEquipmentState)
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
    fn return_rejects_overflow_without_mutating_either_value() {
        let mut stockpile = stockpile_with(f64::MAX);
        let mut equipment = f64::MAX;
        let err = try_return_equipment(&mut stockpile, &mut equipment, f64::MAX).unwrap_err();
        assert_eq!(err, EquipmentAccountingError::ArithmeticOverflow);
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), f64::MAX);
        assert_eq!(equipment, f64::MAX);
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

    // ─── 操作列の保存則 ─────────────────────────────────────────────────────

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
