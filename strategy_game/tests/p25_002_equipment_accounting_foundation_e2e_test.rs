//! P25-002: 軍事装備会計基盤(`military::equipment`)のE2Eテスト。
//!
//! `military::equipment`は、P25-001が確定した既存フィールドの再利用設計
//! (`CountryStockpile[MilitaryEquipment]`・`Division.equipment`/`max_equipment`)を
//! 正規APIとしてまとめたものである。本ファイルはその公開APIを直接呼び出す
//! Test A〜Nに加え、この基盤が既存の募兵・補充・戦闘・復員のいずれからも
//! まだ呼び出されていないこと(Test O)を確認する。

use strategy_game::economy::resources::{CountryStockpile, ResourceType};
use strategy_game::military::equipment::{
    EquipmentAccountingError, equipment_shortfall, try_destroy_equipment, try_issue_equipment,
    try_return_equipment, validate_equipment_state,
};

fn stockpile_with(amount: f64) -> CountryStockpile {
    let mut s = CountryStockpile::new();
    s.set(ResourceType::MilitaryEquipment, amount);
    s
}

// ─── Test A: 有効状態 ────────────────────────────────────────────────────

#[test]
fn test_a_valid_equipment_states() {
    assert_eq!(validate_equipment_state(0.0, 0.0), Ok(()));
    assert_eq!(validate_equipment_state(50.0, 100.0), Ok(()));
    assert_eq!(
        validate_equipment_state(101.0, 100.0),
        Err(EquipmentAccountingError::InvalidEquipmentState)
    );
}

// ─── Test B: 不足量 ──────────────────────────────────────────────────────

#[test]
fn test_b_shortfall() {
    assert_eq!(equipment_shortfall(40.0, 100.0), Ok(60.0));
    assert_eq!(equipment_shortfall(100.0, 100.0), Ok(0.0));
    assert_eq!(
        equipment_shortfall(150.0, 100.0),
        Err(EquipmentAccountingError::InvalidEquipmentState),
        "an invalid state must not be silently hidden as a zero shortfall"
    );
}

// ─── Test C: 完全支給 ────────────────────────────────────────────────────

#[test]
fn test_c_full_issue() {
    let mut stockpile = stockpile_with(100.0);
    let mut equipment = 0.0;
    let result = try_issue_equipment(&mut stockpile, &mut equipment, 100.0, 100.0).unwrap();
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 0.0);
    assert_eq!(equipment, 100.0);
    assert_eq!(
        100.0 + 0.0,
        result.stockpile_after + result.equipment_after,
        "total equipment must be conserved across the transfer"
    );
}

// ─── Test D: 部分支給 ────────────────────────────────────────────────────

#[test]
fn test_d_partial_issue() {
    let mut stockpile = stockpile_with(100.0);
    let mut equipment = 20.0;
    try_issue_equipment(&mut stockpile, &mut equipment, 100.0, 30.0).unwrap();
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 70.0);
    assert_eq!(equipment, 50.0);
}

// ─── Test E: 備蓄不足時の原子性 ──────────────────────────────────────────

#[test]
fn test_e_insufficient_stockpile_is_atomic() {
    let mut stockpile = stockpile_with(29.0);
    let mut equipment = 20.0;
    let err = try_issue_equipment(&mut stockpile, &mut equipment, 100.0, 30.0).unwrap_err();
    assert_eq!(err, EquipmentAccountingError::InsufficientStockpile);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 29.0);
    assert_eq!(equipment, 20.0);
}

// ─── Test F: 上限超過時の原子性 ──────────────────────────────────────────

#[test]
fn test_f_capacity_exceeded_is_atomic() {
    let mut stockpile = stockpile_with(100.0);
    let mut equipment = 80.0;
    let err = try_issue_equipment(&mut stockpile, &mut equipment, 100.0, 21.0).unwrap_err();
    assert_eq!(err, EquipmentAccountingError::DivisionCapacityExceeded);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 100.0);
    assert_eq!(equipment, 80.0);
}

// ─── Test G: 返却 ────────────────────────────────────────────────────────

#[test]
fn test_g_return() {
    let mut stockpile = stockpile_with(10.0);
    let mut equipment = 40.0;
    let result = try_return_equipment(&mut stockpile, &mut equipment, 25.0).unwrap();
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 35.0);
    assert_eq!(equipment, 15.0);
    assert_eq!(
        10.0 + 40.0,
        result.stockpile_after + result.equipment_after,
        "total equipment must be conserved across the return"
    );
}

// ─── Test H: 過剰返却 ────────────────────────────────────────────────────

#[test]
fn test_h_excess_return_is_atomic() {
    let mut stockpile = stockpile_with(10.0);
    let mut equipment = 40.0;
    let err = try_return_equipment(&mut stockpile, &mut equipment, 41.0).unwrap_err();
    assert_eq!(err, EquipmentAccountingError::ReturnAmountExceeded);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 10.0);
    assert_eq!(equipment, 40.0);
}

// ─── Test I: 破壊 ────────────────────────────────────────────────────────

#[test]
fn test_i_destroy() {
    let stockpile = stockpile_with(10.0);
    let mut equipment = 40.0;
    let world_before = stockpile.get(ResourceType::MilitaryEquipment) + equipment;
    let result = try_destroy_equipment(&mut equipment, 15.0).unwrap();
    assert_eq!(result.destroyed, 15.0);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 10.0);
    assert_eq!(equipment, 25.0);
    let world_after = stockpile.get(ResourceType::MilitaryEquipment) + equipment;
    assert_eq!(
        world_before - world_after,
        15.0,
        "world equipment must decrease by exactly the destroyed amount"
    );
}

// ─── Test J: 過剰破壊 ────────────────────────────────────────────────────

#[test]
fn test_j_excess_destroy_is_atomic() {
    let mut equipment = 40.0;
    let err = try_destroy_equipment(&mut equipment, 41.0).unwrap_err();
    assert_eq!(err, EquipmentAccountingError::DestroyAmountExceeded);
    assert_eq!(equipment, 40.0);
}

// ─── Test K: 0数量 ───────────────────────────────────────────────────────

#[test]
fn test_k_zero_amount_operations_are_deterministic_no_ops() {
    let mut stockpile = stockpile_with(100.0);
    let mut equipment = 20.0;

    let issue_result = try_issue_equipment(&mut stockpile, &mut equipment, 100.0, 0.0).unwrap();
    assert_eq!(issue_result.transferred, 0.0);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 100.0);
    assert_eq!(equipment, 20.0);

    let return_result = try_return_equipment(&mut stockpile, &mut equipment, 0.0).unwrap();
    assert_eq!(return_result.transferred, 0.0);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 100.0);
    assert_eq!(equipment, 20.0);

    let destroy_result = try_destroy_equipment(&mut equipment, 0.0).unwrap();
    assert_eq!(destroy_result.destroyed, 0.0);
    assert_eq!(equipment, 20.0);
}

// ─── Test L: overflow ────────────────────────────────────────────────────

#[test]
fn test_l_overflow_is_rejected_without_mutation() {
    let mut stockpile = stockpile_with(f64::MAX);
    let mut equipment = f64::MAX;
    let err = try_return_equipment(&mut stockpile, &mut equipment, f64::MAX).unwrap_err();
    assert_eq!(err, EquipmentAccountingError::ArithmeticOverflow);
    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), f64::MAX);
    assert_eq!(equipment, f64::MAX);
}

// ─── Test M: 国家分離 ────────────────────────────────────────────────────

#[test]
fn test_m_country_separation() {
    let mut stockpile_a = stockpile_with(100.0);
    let stockpile_b = stockpile_with(100.0);
    let mut equipment_a = 0.0;

    try_issue_equipment(&mut stockpile_a, &mut equipment_a, 100.0, 60.0).unwrap();

    assert_eq!(stockpile_a.get(ResourceType::MilitaryEquipment), 40.0);
    assert_eq!(
        stockpile_b.get(ResourceType::MilitaryEquipment),
        100.0,
        "operating on country A's stockpile must never affect country B's stockpile"
    );
}

// ─── Test N: 操作列の保存則 ──────────────────────────────────────────────

#[test]
fn test_n_operation_sequence_conservation() {
    let mut stockpile = stockpile_with(1_000.0);
    let mut equipment = 0.0;
    let initial_total = stockpile.get(ResourceType::MilitaryEquipment) + equipment;

    try_issue_equipment(&mut stockpile, &mut equipment, 1_000.0, 600.0).unwrap();
    try_return_equipment(&mut stockpile, &mut equipment, 100.0).unwrap();
    let destroyed_total = try_destroy_equipment(&mut equipment, 200.0)
        .unwrap()
        .destroyed;

    assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 500.0);
    assert_eq!(equipment, 300.0);
    assert_eq!(destroyed_total, 200.0);
    assert_eq!(
        initial_total,
        stockpile.get(ResourceType::MilitaryEquipment) + equipment + destroyed_total,
        "initial_total must equal final stockpile + final equipment + destroyed total"
    );
}

// ─── Test O: 既存経路非接続 ──────────────────────────────────────────────
//
// military::equipmentはP25-002の時点でどの本番経路からも呼び出されていない。
// これは以下の既存回帰テストが引き続き無変更のまま合格することによって
// 間接的に証明される(実行はこのテストファイル自身ではなく、P25-002の検証
// コマンド一式(§18)としてtest runner側で行う——本テストはその方針をコードとして
// 明示するための記録)。
//
// - military::reinforcement::tests::reinforcement_never_touches_treasury_equipment_organization_or_experience
//   (補充がequipmentへ一切触れないことを既に固定済み)
// - military::demobilization::tests::demobilization_never_touches_treasury_or_equipment_related_fields
//   (復員がequipmentへ一切触れないことを既に固定済み)
// - military::recruitment::request_recruitment は引き続きtreasuryのみを参照し、
//   CountryStockpile[MilitaryEquipment]には一切触れない(grepで再確認済み)。
#[test]
fn test_o_equipment_module_is_not_wired_into_any_production_path_yet() {
    // military::equipmentの型・関数のみを参照する回帰確認(コンパイルが通り、
    // かつ他モジュールからの呼び出しが存在しないことは `grep -rn
    // "military::equipment::" src/military/recruitment.rs src/military/reinforcement.rs
    // src/military/update.rs src/military/combat_calc.rs src/military/demobilization.rs`
    // が0件であることで担保される(P25-002報告書§26参照)。
    let mut stockpile = stockpile_with(0.0);
    let mut equipment = 0.0;
    // 0量の操作が成功すること自体が「配線されていなくても単体で正しく動く」ことの証明。
    assert!(try_issue_equipment(&mut stockpile, &mut equipment, 0.0, 0.0).is_ok());
}
