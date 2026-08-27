use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// プロトタイプで扱う資源の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceType {
    Food,
    Wood,
    Iron,
    Coal,
    /// 精製済みマジッククリスタル(魔法系施設が消費する既存資源、P21-009でRawMagicCrystalと分離)
    MagicCrystal,
    IndustrialGoods,
    MilitaryEquipment,
    /// 未精製のマジッククリスタル原石(P21-009: クリスタル採掘施設が産出し、精製施設が消費する)
    RawMagicCrystal,
}

impl ResourceType {
    pub const ALL: [ResourceType; 8] = [
        ResourceType::Food,
        ResourceType::Wood,
        ResourceType::Iron,
        ResourceType::Coal,
        ResourceType::MagicCrystal,
        ResourceType::IndustrialGoods,
        ResourceType::MilitaryEquipment,
        ResourceType::RawMagicCrystal,
    ];

    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            ResourceType::Food => "resource.food",
            ResourceType::Wood => "resource.wood",
            ResourceType::Iron => "resource.iron",
            ResourceType::Coal => "resource.coal",
            ResourceType::MagicCrystal => "resource.magic_crystal",
            ResourceType::IndustrialGoods => "resource.industrial_goods",
            ResourceType::MilitaryEquipment => "resource.military_equipment",
            ResourceType::RawMagicCrystal => "resource.raw_magic_crystal",
        }
    }
}

/// 国家単位の資源備蓄
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CountryStockpile {
    pub amounts: HashMap<ResourceType, f64>,
}

impl CountryStockpile {
    pub fn new() -> Self {
        let mut amounts = HashMap::new();
        for res in ResourceType::ALL {
            amounts.insert(res, 0.0);
        }
        Self { amounts }
    }

    pub fn get(&self, resource: ResourceType) -> f64 {
        *self.amounts.get(&resource).unwrap_or(&0.0)
    }

    pub fn set(&mut self, resource: ResourceType, amount: f64) {
        self.amounts.insert(resource, amount.max(0.0));
    }

    pub fn add(&mut self, resource: ResourceType, amount: f64) {
        let current = self.get(resource);
        self.amounts.insert(resource, (current + amount).max(0.0));
    }

    pub fn consume(&mut self, resource: ResourceType, amount: f64) -> bool {
        let current = self.get(resource);
        if current >= amount {
            self.amounts.insert(resource, current - amount);
            true
        } else {
            false
        }
    }

    /// `ResourceType::MilitaryEquipment`の生産加算を、固定小数点units経由で原子的に
    /// 行う(P25-002-REV-001)。生の`f64`加算(`add`)は2^53付近の規模で精度が崩れ、
    /// 装備会計の保存則を破る恐れがあるため、`military::equipment`の支給・返却・
    /// 破壊APIと全く同じ量子化規則(`military_equipment_value_to_units`/
    /// `military_equipment_units_to_value`)を共有してこの1関数だけに集約する
    /// (計算式をeconomy側とmilitary側へ複製しない)。他の`ResourceType`には
    /// 一切影響しない特別扱いであり、`add`はそれらに対して従来どおり使われ続ける。
    /// 失敗時(非有限値・値域超過・overflow等)は備蓄を変更せず`Err`を返す。
    pub fn add_military_equipment_produced(
        &mut self,
        produced: f64,
    ) -> Result<(), MilitaryEquipmentUnitsError> {
        let current = self.get(ResourceType::MilitaryEquipment);
        let current_units = military_equipment_value_to_units(current)?;
        let produced_units = military_equipment_value_to_units(produced)?;

        let new_units = current_units
            .checked_add(produced_units)
            .filter(|&u| u <= MILITARY_EQUIPMENT_MAX_UNITS)
            .ok_or(MilitaryEquipmentUnitsError::OutOfRange)?;

        let new_value = military_equipment_units_to_value(new_units);
        if military_equipment_value_to_units(new_value)? != new_units {
            return Err(MilitaryEquipmentUnitsError::RoundTripFailed);
        }

        self.set(ResourceType::MilitaryEquipment, new_value);
        Ok(())
    }
}

/// `ResourceType::MilitaryEquipment`専用の固定小数点scale(P25-002-REV-001)。
/// `military::equipment`の装備会計APIと、本ファイルの`CountryStockpile::
/// add_military_equipment_produced`(MilitaryFactory生産経路)の両方がこのscaleを
/// 共有する。他の`ResourceType`には適用しない。
///
/// 実データ監査(`verification_logs/phase-25/p25-002-rev-001/`報告書§9〜11参照)の
/// 結論: 現行データ(州28、MilitaryFactory最大レベル10、基礎生産20.0/レベル/月)での
/// 150年間無消費理論最大備蓄は約1,008万。正規化可能な最大値(equipment値換算で
/// 10億、`MILITARY_EQUIPMENT_MAX_UNITS`参照)はこれを桁違いに上回るため、この
/// 精度・scaleを採用する。
pub const MILITARY_EQUIPMENT_SCALE: u64 = 1_000_000;

/// 正規化可能な最大units(`units → f64 → units`の往復変換が保証される上限)。
///
/// `f64`は2^53までの整数を誤差なく表現できるが、`units as f64 / SCALE`という除算
/// (`SCALE`に2の冪でない素因数5^6を含む)自体が丸め誤差を持つため、往復変換の
/// 安全な上限は2^53よりも実際には低い(2^53-1ちょうどは往復変換に失敗することを
/// 実測で確認した——`verification_logs/phase-25/p25-002-rev-001/`報告書§9参照)。
/// `1,000,000,000,000,000`(10^15、equipment値換算で10億)を採用する: この値および
/// その近傍(前後5,000)、さらに0からこの値までを大きな素数間隔で約100万点
/// サンプリングした実測往復変換テストのいずれでも失敗が0件だったことを確認済み。
/// equipment値換算で10億は、現行データ(州28)での150年間無消費理論最大備蓄
/// (約1,008万)の約99倍、大幅な将来拡張(州10倍=280)を仮定した場合の理論最大
/// (約1億)の約10倍の余裕を持つ。
pub const MILITARY_EQUIPMENT_MAX_UNITS: u64 = 1_000_000_000_000_000;

/// `military_equipment_value_to_units`が失敗し得る理由。`military::equipment::
/// EquipmentAccountingError`へ変換して使われる、経済モジュール側の中立的なエラー型
/// (economy側がmilitary側のエラー型へ依存しないようにするための分離)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilitaryEquipmentUnitsError {
    /// 値がNaNまたは無限大。
    NonFinite,
    /// 値が負。
    Negative,
    /// 量子化後のunitsが`MILITARY_EQUIPMENT_MAX_UNITS`を超える、またはscale乗算が
    /// 非有限になった。
    OutOfRange,
    /// `units → f64 → units`の往復変換が元のunitsと一致しなかった。
    RoundTripFailed,
}

/// `f64`のMilitaryEquipment数量を、正規の整数units(`1 / MILITARY_EQUIPMENT_SCALE`
/// 単位)へ量子化する。量子化段階でのみ四捨五入し(`round()`)、finite・非負・
/// 値域内・往復可能(`U(V(u)) = u`)であることを検証する。
pub fn military_equipment_value_to_units(value: f64) -> Result<u64, MilitaryEquipmentUnitsError> {
    if !value.is_finite() {
        return Err(MilitaryEquipmentUnitsError::NonFinite);
    }
    if value < 0.0 {
        return Err(MilitaryEquipmentUnitsError::Negative);
    }
    let scaled = value * (MILITARY_EQUIPMENT_SCALE as f64);
    if !scaled.is_finite() {
        return Err(MilitaryEquipmentUnitsError::OutOfRange);
    }
    let rounded = scaled.round();
    if rounded > MILITARY_EQUIPMENT_MAX_UNITS as f64 {
        return Err(MilitaryEquipmentUnitsError::OutOfRange);
    }
    let units = rounded as u64;

    // 往復不変条件の確認(§7): units → f64 → units が元のunitsと一致すること。
    let round_trip_value = military_equipment_units_to_value(units);
    let round_trip_scaled = round_trip_value * (MILITARY_EQUIPMENT_SCALE as f64);
    if !round_trip_scaled.is_finite() || round_trip_scaled.round() as u64 != units {
        return Err(MilitaryEquipmentUnitsError::RoundTripFailed);
    }

    Ok(units)
}

/// 正規units(`u64`)をMilitaryEquipmentの`f64`数量へ変換する。
pub fn military_equipment_units_to_value(units: u64) -> f64 {
    (units as f64) / (MILITARY_EQUIPMENT_SCALE as f64)
}

/// 州の資源鉱床
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateResourceDeposit {
    pub resource_type: ResourceType,
    pub base_output: f64,
    pub discovered: bool,
    pub development_level: u32,
}

impl Default for StateResourceDeposit {
    fn default() -> Self {
        Self {
            resource_type: ResourceType::Iron,
            base_output: 10.0,
            discovered: true,
            development_level: 1,
        }
    }
}

/// 指定した資源種別のdiscovered済み鉱床が1件以上含まれるかを判定する(P21-009)。
/// クリスタル採掘施設のような鉱床ゲート付き建物の建設可否判定に使う。
pub fn has_discovered_deposit(deposits: &[StateResourceDeposit], resource: ResourceType) -> bool {
    deposits
        .iter()
        .any(|d| d.discovered && d.resource_type == resource)
}

/// 州がクリスタル専用(discoveredなMagicCrystal鉱床を持ち、それ以外のdiscovered鉱床を
/// 持たない)かどうかを判定する(P21-009-FIX-001)。このような州では通常Mineは対象鉱床
/// (MagicCrystal以外)を持たないため、新規建設を許可しない(既存のMineから
/// MagicCrystal種別鉱床を除外する仕様[production.rs]と対になる判定)。
pub fn is_crystal_only_state(deposits: &[StateResourceDeposit]) -> bool {
    has_discovered_deposit(deposits, ResourceType::MagicCrystal)
        && !deposits
            .iter()
            .any(|d| d.discovered && d.resource_type != ResourceType::MagicCrystal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_discovered_deposit_true_only_for_matching_discovered_type() {
        let deposits = vec![
            StateResourceDeposit {
                resource_type: ResourceType::Iron,
                base_output: 10.0,
                discovered: true,
                development_level: 1,
            },
            StateResourceDeposit {
                resource_type: ResourceType::MagicCrystal,
                base_output: 30.0,
                discovered: false,
                development_level: 1,
            },
        ];
        assert!(!has_discovered_deposit(
            &deposits,
            ResourceType::MagicCrystal
        ));
        assert!(has_discovered_deposit(&deposits, ResourceType::Iron));
        assert!(!has_discovered_deposit(&deposits, ResourceType::Coal));
    }

    #[test]
    fn has_discovered_deposit_true_when_discovered_and_matching() {
        let deposits = vec![StateResourceDeposit {
            resource_type: ResourceType::MagicCrystal,
            base_output: 30.0,
            discovered: true,
            development_level: 1,
        }];
        assert!(has_discovered_deposit(
            &deposits,
            ResourceType::MagicCrystal
        ));
    }

    #[test]
    fn is_crystal_only_state_true_when_only_magic_crystal_deposit_present() {
        let deposits = vec![StateResourceDeposit {
            resource_type: ResourceType::MagicCrystal,
            base_output: 30.0,
            discovered: true,
            development_level: 1,
        }];
        assert!(is_crystal_only_state(&deposits));
    }

    #[test]
    fn is_crystal_only_state_false_when_no_magic_crystal_deposit() {
        let deposits = vec![StateResourceDeposit {
            resource_type: ResourceType::Iron,
            base_output: 10.0,
            discovered: true,
            development_level: 1,
        }];
        assert!(!is_crystal_only_state(&deposits));
    }

    #[test]
    fn is_crystal_only_state_false_when_mixed_with_other_discovered_deposit() {
        let deposits = vec![
            StateResourceDeposit {
                resource_type: ResourceType::MagicCrystal,
                base_output: 30.0,
                discovered: true,
                development_level: 1,
            },
            StateResourceDeposit {
                resource_type: ResourceType::Iron,
                base_output: 10.0,
                discovered: true,
                development_level: 1,
            },
        ];
        assert!(!is_crystal_only_state(&deposits));
    }

    #[test]
    fn is_crystal_only_state_true_when_other_deposit_is_not_discovered() {
        let deposits = vec![
            StateResourceDeposit {
                resource_type: ResourceType::MagicCrystal,
                base_output: 30.0,
                discovered: true,
                development_level: 1,
            },
            StateResourceDeposit {
                resource_type: ResourceType::Iron,
                base_output: 10.0,
                discovered: false,
                development_level: 1,
            },
        ];
        assert!(is_crystal_only_state(&deposits));
    }

    // ─── P25-002-REV-001: MilitaryEquipment固定小数点 ─────────────────────

    #[test]
    fn military_equipment_round_trip_for_representative_units() {
        for units in [
            0u64,
            1,
            MILITARY_EQUIPMENT_SCALE - 1,
            MILITARY_EQUIPMENT_SCALE,
            MILITARY_EQUIPMENT_SCALE + 1,
            1_000_000_000, // 10億units = 1,000装備、代表的な大規模値
            MILITARY_EQUIPMENT_MAX_UNITS,
        ] {
            let value = military_equipment_units_to_value(units);
            let round_trip = military_equipment_value_to_units(value).unwrap();
            assert_eq!(round_trip, units, "round trip failed for units={units}");
        }
    }

    #[test]
    fn military_equipment_converts_fractional_values() {
        assert_eq!(military_equipment_value_to_units(0.1).unwrap(), 100_000);
        assert_eq!(military_equipment_value_to_units(0.45).unwrap(), 450_000);
        assert_eq!(
            military_equipment_value_to_units(0.333333).unwrap(),
            333_333
        );
        assert_eq!(
            military_equipment_value_to_units(1.000001).unwrap(),
            1_000_001
        );
    }

    #[test]
    fn military_equipment_rejects_non_finite_and_negative_values() {
        assert_eq!(
            military_equipment_value_to_units(f64::NAN),
            Err(MilitaryEquipmentUnitsError::NonFinite)
        );
        assert_eq!(
            military_equipment_value_to_units(f64::INFINITY),
            Err(MilitaryEquipmentUnitsError::NonFinite)
        );
        assert_eq!(
            military_equipment_value_to_units(-1.0),
            Err(MilitaryEquipmentUnitsError::Negative)
        );
    }

    #[test]
    fn military_equipment_max_units_round_trips_but_one_unit_beyond_is_rejected() {
        assert!(
            military_equipment_value_to_units(military_equipment_units_to_value(
                MILITARY_EQUIPMENT_MAX_UNITS
            ))
            .is_ok()
        );

        let just_over = military_equipment_units_to_value(MILITARY_EQUIPMENT_MAX_UNITS)
            + (2.0 / MILITARY_EQUIPMENT_SCALE as f64);
        assert_eq!(
            military_equipment_value_to_units(just_over),
            Err(MilitaryEquipmentUnitsError::OutOfRange)
        );
    }

    #[test]
    fn add_military_equipment_produced_accumulates_in_units() {
        let mut stockpile = CountryStockpile::new();
        stockpile.add_military_equipment_produced(14.6).unwrap();
        stockpile.add_military_equipment_produced(5.4).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 20.0);
    }

    #[test]
    fn add_military_equipment_produced_does_not_affect_other_resources() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::Iron, 500.0);
        stockpile.add_military_equipment_produced(20.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::Iron), 500.0);
    }
}
