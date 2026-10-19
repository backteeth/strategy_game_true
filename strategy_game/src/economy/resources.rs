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
    /// P26-002: 弾薬・燃料・軍用食料・医療整備物資等を抽象化した単一の消費補給物資。
    /// `MilitaryEquipment`(耐久装備、casualty比例でのみ破壊される)とは意味・型の両面で
    /// 完全に分離した別資源であり、両者を混同・統合しない
    /// (`verification_logs/phase-26/p26-001/`§12参照)。P26-002時点では本番の生産・
    /// 消費Systemへは一切接続されない(会計基盤のみ)。
    MilitarySupplies,
}

impl ResourceType {
    pub const ALL: [ResourceType; 9] = [
        ResourceType::Food,
        ResourceType::Wood,
        ResourceType::Iron,
        ResourceType::Coal,
        ResourceType::MagicCrystal,
        ResourceType::IndustrialGoods,
        ResourceType::MilitaryEquipment,
        ResourceType::RawMagicCrystal,
        ResourceType::MilitarySupplies,
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
            ResourceType::MilitarySupplies => "resource.military_supplies",
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

    /// `ResourceType::MilitarySupplies`の生産加算を、固定小数点units経由で原子的に行う
    /// (P26-002)。`add_military_equipment_produced`と全く同じ手順(read→validate→
    /// units変換→checked_add→上限検証→f64へ正規変換→1回だけcommit)を踏むが、
    /// `MilitaryEquipment`とは意味的に別資源であるため、定数・エラー型・変換関数は
    /// 独立した`military_supplies_*`名前空間として並存させる(共通化しない、
    /// `verification_logs/phase-26/p26-001/`§43参照)。他の`ResourceType`には一切
    /// 影響しない。`produced = 0.0`は成功する意味的no-opとして扱う。失敗時
    /// (非有限値・負数・値域超過・overflow・往復変換失敗)は備蓄を完全に不変のまま
    /// `Err`を返す。P26-002時点では、このAPIをどの本番生産Systemからも呼び出さない
    /// (P26-003の範囲、`verification_logs/phase-26/p26-001/`§43)。
    pub fn add_military_supplies_produced(
        &mut self,
        produced: f64,
    ) -> Result<(), MilitarySuppliesUnitsError> {
        let current = self.get(ResourceType::MilitarySupplies);
        let new_value = checked_military_supplies_addition(current, produced)?;
        self.set(ResourceType::MilitarySupplies, new_value);
        Ok(())
    }

    /// P26-004-PRE-005: New Game初期化・旧Save意味的migrationにおける、MilitarySupplies
    /// 初期ブリッジ備蓄の一度限りgrantを、固定小数点units経由で原子的に行う専用API。
    ///
    /// `add_military_supplies_produced`と内部の演算規律(read→validate→units変換→
    /// checked_add→上限検証→f64正規変換→往復検証→1回だけcommit)は
    /// `checked_military_supplies_addition`として共有するが、意味的には別処理である
    /// ("生産"ではなく"初期化時の外生的grant")ため、生産APIをそのまま呼ばず、
    /// 呼び出し意図が型シグネチャ・関数名の両方から明確な専用APIとして独立させる
    /// (`verification_logs/phase-26/p26-004-pre-005/`spec §9)。`grant = 0.0`は成功する
    /// 意味的no-opとして扱う。失敗時(非有限値・負数・値域超過・overflow・往復変換失敗)は
    /// 備蓄を完全に不変のまま`Err`を返す。他の`ResourceType`には一切影響しない。
    pub fn try_grant_military_supplies(
        &mut self,
        grant: f64,
    ) -> Result<(), MilitarySuppliesUnitsError> {
        let current = self.get(ResourceType::MilitarySupplies);
        let new_value = checked_military_supplies_addition(current, grant)?;
        self.set(ResourceType::MilitarySupplies, new_value);
        Ok(())
    }

    /// `ResourceType::MilitarySupplies`の消費を、固定小数点units経由で原子的に行う
    /// (P26-002)。備蓄が要求量に満たない場合は`Err(InsufficientStockpile)`として
    /// 失敗し、部分消費はしない(P26-004の日次配分側が事前に
    /// `T = min(stockpile, total_demand)`を計算してから本APIを呼ぶ設計のため、本API
    /// 自身は「可能な分だけ部分消費する」挙動を持たない、
    /// `verification_logs/phase-26/p26-001/`§11参照)。`amount = 0.0`は成功する
    /// 意味的no-opとして扱う。失敗時は備蓄を完全に不変のまま`Err`を返す。
    pub fn try_consume_military_supplies(
        &mut self,
        amount: f64,
    ) -> Result<(), MilitarySuppliesUnitsError> {
        let current = self.get(ResourceType::MilitarySupplies);
        let current_units = military_supplies_value_to_units(current)?;
        let requested_units = military_supplies_value_to_units(amount)?;

        let new_units = current_units
            .checked_sub(requested_units)
            .ok_or(MilitarySuppliesUnitsError::InsufficientStockpile)?;

        let new_value = military_supplies_units_to_value(new_units);
        if military_supplies_value_to_units(new_value)? != new_units {
            return Err(MilitarySuppliesUnitsError::RoundTripFailed);
        }

        self.set(ResourceType::MilitarySupplies, new_value);
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

// ─── P26-002: MilitarySupplies固定小数点会計基盤 ─────────────────────────
//
// `MilitaryEquipment`(耐久装備、上記)の固定小数点実装(P25-002-REV-001)と全く
// 同じ設計方針・手順を複製する(`verification_logs/phase-26/p26-001/`§21・§43)。
// ただし`MilitarySupplies`(消費補給物資)は意味的に別資源であり、Phase 25が確立
// した装備会計の保存則・全テストに一切触れないよう、定数・エラー型・変換関数を
// 独立した`military_supplies_*`名前空間として並存させ、共通化リファクタは行わない。

/// `ResourceType::MilitarySupplies`専用の固定小数点scale(P26-002)。
/// `MilitaryEquipment`と同じ`1,000,000`(小数点以下6桁)を採用する。根拠
/// (`verification_logs/phase-26/p26-002/`報告書§7〜8参照): 既存資源生産は`f64`で
/// 小数第1〜2桁程度の精度しか使わず、`DivisionDefinition.supply_usage`
/// (Infantry=1.0/Artillery=3.0/Mage=2.0)も同程度の精度である。P25-002-REV-001が
/// 確立した`SCALE=1,000,000`は既にこの精度を大幅に上回り、Save V1のf64往復・
/// P26-004で想定される日次配分(需要比例配分)の丸め誤差蓄積を防ぐのに十分な
/// 余裕を持つ。反証がないため、P25-002-REV-001との一貫性を優先してそのまま採用する。
pub const MILITARY_SUPPLIES_SCALE: u64 = 1_000_000;

/// 正規化可能な最大units(`units → f64 → units`の往復変換が保証される上限、P26-002)。
///
/// P26-003再監査(`verification_logs/phase-26/p26-003/`報告書§30〜32参照): 実際の
/// 生産式(MilitaryFactoryがMilitarySuppliesを併産、基礎生産100.0/レベル/月、最大
/// レベル10、`verification_logs/phase-26/p26-003/`参照)が確定したため、P26-002時点の
/// 暫定採用値(`MilitaryEquipment`と同じ値をそのまま流用)を実データで再検証した。
/// 現行州28・全州MilitaryFactory最大レベル・稼働率100%・150年間無消費という
/// 非現実的な最悪ケースでの理論最大備蓄は約5,040万(value換算)であり、この値
/// (equipment値換算で10億)の約19.8倍の余裕を持つ。仮に州数が現行の10倍(280)へ
/// 拡張しても理論最大は約5.04億に留まり、この値を下回る(約1.98倍の余裕を維持)。
/// さらにMilitarySuppliesはP26-004以降、装備と異なり毎日恒常的に消費される設計の
/// ため、実プレイでこの無消費理論値へ近づくことはない。`military_equipment_value_to_units`
/// と同じ実測往復変換サンプリング(0からこの値までの代表点・境界値)を
/// `MilitarySupplies`側の量子化式に対しても実行し、失敗が0件であることを確認済み
/// (下記`military_supplies_value_to_units`のテスト参照)。以上によりP26-002の
/// Low finding(暫定値の再監査)は`RESOLVED`と判定する。到達時は`OutOfRange`を
/// 明示的に返しclamp・wraparoundはしない(変更なし)。
pub const MILITARY_SUPPLIES_MAX_UNITS: u64 = 1_000_000_000_000_000;

/// `military_supplies_value_to_units`/`CountryStockpile`のMilitarySupplies会計操作
/// が失敗し得る理由(P26-002)。`MilitaryEquipmentUnitsError`と意味的に類似する
/// バリアントを含むが、意図的に別のenumとして定義する(将来`MilitarySupplies`と
/// `MilitaryEquipment`の操作可能範囲が分岐した場合に備え、安易に共通化しない、
/// `verification_logs/phase-26/p26-001/`§43)。`MilitaryEquipmentUnitsError`にはない
/// `InsufficientStockpile`/`ArithmeticOverflow`を追加で持つ——`MilitarySupplies`には
/// `MilitaryEquipment`のような「Division側の支給・返却」を仲介する上位モジュール
/// (`military::equipment`)が存在せず、消費(`try_consume_military_supplies`)の
/// 不足判定をこの1つのenumで直接表現する必要があるため。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilitarySuppliesUnitsError {
    /// 値がNaNまたは無限大。
    NonFinite,
    /// 値が負。
    Negative,
    /// 量子化後のunitsが`MILITARY_SUPPLIES_MAX_UNITS`を超える、またはscale乗算が
    /// 非有限になった。
    OutOfRange,
    /// `units → f64 → units`の往復変換が元のunitsと一致しなかった。
    RoundTripFailed,
    /// 国家備蓄が消費要求量に対して不足している(部分消費はしない、全量拒否)。
    InsufficientStockpile,
    /// units上のchecked演算(u64の真のwraparound)が失敗した。現実的な値域
    /// (`MILITARY_SUPPLIES_MAX_UNITS`以下同士の加算)では理論上到達不能な防御的
    /// 分岐であり、`OutOfRange`(ドメイン上限超過)とは意味的に区別する。
    ArithmeticOverflow,
}

/// `f64`のMilitarySupplies数量を、正規の整数units(`1 / MILITARY_SUPPLIES_SCALE`
/// 単位)へ量子化する。`military_equipment_value_to_units`と全く同じ規律
/// (量子化段階でのみ`round()`、finite・非負・値域内・往復可能であることを検証)。
pub fn military_supplies_value_to_units(value: f64) -> Result<u64, MilitarySuppliesUnitsError> {
    if !value.is_finite() {
        return Err(MilitarySuppliesUnitsError::NonFinite);
    }
    if value < 0.0 {
        return Err(MilitarySuppliesUnitsError::Negative);
    }
    let scaled = value * (MILITARY_SUPPLIES_SCALE as f64);
    if !scaled.is_finite() {
        return Err(MilitarySuppliesUnitsError::OutOfRange);
    }
    let rounded = scaled.round();
    if rounded > MILITARY_SUPPLIES_MAX_UNITS as f64 {
        return Err(MilitarySuppliesUnitsError::OutOfRange);
    }
    let units = rounded as u64;

    // 往復不変条件の確認: units → f64 → units が元のunitsと一致すること。
    let round_trip_value = military_supplies_units_to_value(units);
    let round_trip_scaled = round_trip_value * (MILITARY_SUPPLIES_SCALE as f64);
    if !round_trip_scaled.is_finite() || round_trip_scaled.round() as u64 != units {
        return Err(MilitarySuppliesUnitsError::RoundTripFailed);
    }

    Ok(units)
}

/// 正規units(`u64`)をMilitarySuppliesの`f64`数量へ変換する。
pub fn military_supplies_units_to_value(units: u64) -> f64 {
    (units as f64) / (MILITARY_SUPPLIES_SCALE as f64)
}

/// `add_military_supplies_produced`(生産)と`try_grant_military_supplies`(初期化時の
/// 一度限りgrant、P26-004-PRE-005)が共有する、加算後の正規値を1つだけ計算する内部
/// ヘルパー。呼び出し側の`set()`コミットより前に完了する検証専用の純粋関数であり、
/// それ自体は`CountryStockpile`を変更しない(公開APIそれぞれが、この関数の成功時のみ
/// `set()`を1回呼ぶ)。checked_add自体のu64 wraparound(現実的な値域では到達不能な
/// 防御的分岐)と、ドメイン上限`MILITARY_SUPPLIES_MAX_UNITS`超過(通常の運用上あり得る
/// 分岐)を意味的に区別する。
fn checked_military_supplies_addition(
    current: f64,
    delta: f64,
) -> Result<f64, MilitarySuppliesUnitsError> {
    let current_units = military_supplies_value_to_units(current)?;
    let delta_units = military_supplies_value_to_units(delta)?;

    let new_units = current_units
        .checked_add(delta_units)
        .ok_or(MilitarySuppliesUnitsError::ArithmeticOverflow)?;
    if new_units > MILITARY_SUPPLIES_MAX_UNITS {
        return Err(MilitarySuppliesUnitsError::OutOfRange);
    }

    let new_value = military_supplies_units_to_value(new_units);
    if military_supplies_value_to_units(new_value)? != new_units {
        return Err(MilitarySuppliesUnitsError::RoundTripFailed);
    }

    Ok(new_value)
}

/// 州の資源鉱床
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

    // ─── P26-002: MilitarySupplies固定小数点会計基盤 ─────────────────────

    #[test]
    fn military_supplies_round_trip_for_representative_units() {
        for units in [
            0u64,
            1,
            MILITARY_SUPPLIES_SCALE - 1,
            MILITARY_SUPPLIES_SCALE,
            MILITARY_SUPPLIES_SCALE + 1,
            1_000_000_000, // 10億units = 1,000供給値、代表的な大規模値
            MILITARY_SUPPLIES_MAX_UNITS,
        ] {
            let value = military_supplies_units_to_value(units);
            let round_trip = military_supplies_value_to_units(value).unwrap();
            assert_eq!(round_trip, units, "round trip failed for units={units}");
        }
    }

    #[test]
    fn military_supplies_converts_fractional_values() {
        assert_eq!(military_supplies_value_to_units(0.0).unwrap(), 0);
        assert_eq!(military_supplies_value_to_units(1.0).unwrap(), 1_000_000);
        assert_eq!(military_supplies_value_to_units(3.7).unwrap(), 3_700_000);
        assert_eq!(military_supplies_value_to_units(0.000001).unwrap(), 1);
        assert_eq!(military_supplies_value_to_units(0.1).unwrap(), 100_000);
        assert_eq!(military_supplies_value_to_units(0.45).unwrap(), 450_000);
        assert_eq!(
            military_supplies_value_to_units(0.333333).unwrap(),
            333_333
        );
        assert_eq!(
            military_supplies_value_to_units(1.000001).unwrap(),
            1_000_001
        );
    }

    #[test]
    fn military_supplies_negative_zero_is_treated_as_zero() {
        // -0.0は`< 0.0`比較上0.0と等価(IEEE754)なので、Negativeとしては拒否されず
        // units=0として扱われることを明示する(spec §8「-0.0の扱いを明示」)。
        assert_eq!(military_supplies_value_to_units(-0.0).unwrap(), 0);
    }

    #[test]
    fn military_supplies_rejects_non_finite_and_negative_values() {
        assert_eq!(
            military_supplies_value_to_units(f64::NAN),
            Err(MilitarySuppliesUnitsError::NonFinite)
        );
        assert_eq!(
            military_supplies_value_to_units(f64::INFINITY),
            Err(MilitarySuppliesUnitsError::NonFinite)
        );
        assert_eq!(
            military_supplies_value_to_units(f64::NEG_INFINITY),
            Err(MilitarySuppliesUnitsError::NonFinite)
        );
        assert_eq!(
            military_supplies_value_to_units(-1.0),
            Err(MilitarySuppliesUnitsError::Negative)
        );
        assert_eq!(
            military_supplies_value_to_units(-0.000001),
            Err(MilitarySuppliesUnitsError::Negative)
        );
    }

    #[test]
    fn military_supplies_rejects_values_beyond_max_units_and_f64_max() {
        assert_eq!(
            military_supplies_value_to_units(f64::MAX),
            Err(MilitarySuppliesUnitsError::OutOfRange)
        );
    }

    #[test]
    fn military_supplies_max_units_round_trips_but_one_unit_beyond_is_rejected() {
        assert!(
            military_supplies_value_to_units(military_supplies_units_to_value(
                MILITARY_SUPPLIES_MAX_UNITS
            ))
            .is_ok()
        );

        let just_over = military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS)
            + (2.0 / MILITARY_SUPPLIES_SCALE as f64);
        assert_eq!(
            military_supplies_value_to_units(just_over),
            Err(MilitarySuppliesUnitsError::OutOfRange)
        );
    }

    #[test]
    fn military_supplies_two_pow_53_is_rejected_as_ambiguous() {
        // P25-002-REV-001が発見した2^53近傍の曖昧な巨大値問題を再導入しないことを
        // 確認する(正規値域`MILITARY_SUPPLIES_MAX_UNITS`/`SCALE`=10億を大幅に
        // 超えるため、値域外として明確に拒否される)。
        let two_pow_53 = 2f64.powi(53);
        assert_eq!(
            military_supplies_value_to_units(two_pow_53),
            Err(MilitarySuppliesUnitsError::OutOfRange)
        );
    }

    #[test]
    fn add_military_supplies_produced_accumulates_in_units() {
        let mut stockpile = CountryStockpile::new();
        stockpile.add_military_supplies_produced(14.6).unwrap();
        stockpile.add_military_supplies_produced(5.4).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 20.0);
    }

    #[test]
    fn add_military_supplies_produced_zero_is_a_no_op() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::MilitarySupplies, 10.0);
        stockpile.add_military_supplies_produced(0.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 10.0);
    }

    #[test]
    fn add_military_supplies_produced_does_not_affect_other_resources() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::Iron, 500.0);
        stockpile.set(ResourceType::MilitaryEquipment, 77.0);
        stockpile.add_military_supplies_produced(20.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::Iron), 500.0);
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 77.0);
    }

    #[test]
    fn add_military_supplies_produced_rejects_overflow_without_mutation() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(
            ResourceType::MilitarySupplies,
            military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS),
        );
        let err = stockpile
            .add_military_supplies_produced(military_supplies_units_to_value(
                MILITARY_SUPPLIES_MAX_UNITS,
            ))
            .unwrap_err();
        assert_eq!(err, MilitarySuppliesUnitsError::OutOfRange);
        assert_eq!(
            stockpile.get(ResourceType::MilitarySupplies),
            military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS)
        );
    }

    #[test]
    fn add_military_supplies_produced_rejects_invalid_current_stockpile_without_mutation() {
        let mut stockpile = CountryStockpile::new();
        // `CountryStockpile::set()`自体が`max(0.0)`でNaNを0.0へ正規化してしまうため、
        // 不正な現在備蓄(NaN)を検証するには`amounts`へ直接注入する。
        stockpile
            .amounts
            .insert(ResourceType::MilitarySupplies, f64::NAN);
        let err = stockpile.add_military_supplies_produced(5.0).unwrap_err();
        assert_eq!(err, MilitarySuppliesUnitsError::NonFinite);
        assert!(stockpile.get(ResourceType::MilitarySupplies).is_nan());
    }

    #[test]
    fn add_military_supplies_produced_rejects_invalid_produced_amount_without_mutation() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::MilitarySupplies, 10.0);
        let err = stockpile
            .add_military_supplies_produced(-5.0)
            .unwrap_err();
        assert_eq!(err, MilitarySuppliesUnitsError::Negative);
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 10.0);
    }

    // ─── P26-004-PRE-005: try_grant_military_supplies ───────────────────────

    #[test]
    fn try_grant_military_supplies_accumulates_in_units() {
        let mut stockpile = CountryStockpile::new();
        stockpile.try_grant_military_supplies(31.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 31.0);
        stockpile.try_grant_military_supplies(5.4).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 36.4);
    }

    #[test]
    fn try_grant_military_supplies_zero_is_a_no_op() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::MilitarySupplies, 10.0);
        stockpile.try_grant_military_supplies(0.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 10.0);
    }

    #[test]
    fn try_grant_military_supplies_does_not_affect_other_resources() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::Food, 50.0);
        stockpile.set(ResourceType::MilitaryEquipment, 5.0);
        stockpile.try_grant_military_supplies(31.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::Food), 50.0);
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 5.0);
    }

    #[test]
    fn try_grant_military_supplies_rejects_overflow_without_mutation() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(
            ResourceType::MilitarySupplies,
            military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS),
        );
        let err = stockpile
            .try_grant_military_supplies(military_supplies_units_to_value(
                MILITARY_SUPPLIES_MAX_UNITS,
            ))
            .unwrap_err();
        assert_eq!(err, MilitarySuppliesUnitsError::OutOfRange);
        assert_eq!(
            stockpile.get(ResourceType::MilitarySupplies),
            military_supplies_units_to_value(MILITARY_SUPPLIES_MAX_UNITS)
        );
    }

    #[test]
    fn try_grant_military_supplies_rejects_invalid_current_stockpile_without_mutation() {
        let mut stockpile = CountryStockpile::new();
        stockpile
            .amounts
            .insert(ResourceType::MilitarySupplies, f64::NAN);
        let err = stockpile.try_grant_military_supplies(5.0).unwrap_err();
        assert_eq!(err, MilitarySuppliesUnitsError::NonFinite);
        assert!(stockpile.get(ResourceType::MilitarySupplies).is_nan());
    }

    #[test]
    fn try_grant_military_supplies_rejects_invalid_grant_amount_without_mutation() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::MilitarySupplies, 10.0);
        let err = stockpile.try_grant_military_supplies(-5.0).unwrap_err();
        assert_eq!(err, MilitarySuppliesUnitsError::Negative);
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 10.0);
    }

    #[test]
    fn try_grant_and_produced_apis_compose_additively_and_are_independent_of_order() {
        // spec §9: grantは「生産」とは独立した外生的加算だが、同じ会計基盤(units)を
        // 共有するため、grant→produced と produced→grant の順序によらず合計は一致する。
        let mut a = CountryStockpile::new();
        a.try_grant_military_supplies(31.0).unwrap();
        a.add_military_supplies_produced(42.079).unwrap();

        let mut b = CountryStockpile::new();
        b.add_military_supplies_produced(42.079).unwrap();
        b.try_grant_military_supplies(31.0).unwrap();

        assert_eq!(
            a.get(ResourceType::MilitarySupplies),
            b.get(ResourceType::MilitarySupplies)
        );
        assert_eq!(a.get(ResourceType::MilitarySupplies), 73.079);
    }

    #[test]
    fn try_consume_military_supplies_zero_is_a_no_op() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::MilitarySupplies, 10.0);
        stockpile.try_consume_military_supplies(0.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 10.0);
    }

    #[test]
    fn try_consume_military_supplies_partial_and_full() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::MilitarySupplies, 100.0);
        stockpile.try_consume_military_supplies(30.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 70.0);
        stockpile.try_consume_military_supplies(70.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 0.0);
    }

    #[test]
    fn try_consume_military_supplies_fractional_amount() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::MilitarySupplies, 10.0);
        stockpile.try_consume_military_supplies(3.25).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 6.75);
    }

    #[test]
    fn try_consume_military_supplies_insufficient_stockpile_is_atomic() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::MilitarySupplies, 29.0);
        let err = stockpile
            .try_consume_military_supplies(30.0)
            .unwrap_err();
        assert_eq!(err, MilitarySuppliesUnitsError::InsufficientStockpile);
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 29.0);
    }

    #[test]
    fn try_consume_military_supplies_rejects_invalid_current_stockpile_without_mutation() {
        let mut stockpile = CountryStockpile::new();
        // `CountryStockpile::set()`自体が`max(0.0)`でclampするため、負の内部状態を
        // 検証するには`amounts`へ直接注入する(不正な現在備蓄を模す意図的な迂回)。
        stockpile
            .amounts
            .insert(ResourceType::MilitarySupplies, -1.0);
        let err = stockpile.try_consume_military_supplies(1.0).unwrap_err();
        assert_eq!(err, MilitarySuppliesUnitsError::Negative);
    }

    #[test]
    fn try_consume_military_supplies_rejects_invalid_requested_amount_without_mutation() {
        let mut stockpile = CountryStockpile::new();
        stockpile.set(ResourceType::MilitarySupplies, 10.0);
        let err = stockpile
            .try_consume_military_supplies(f64::NAN)
            .unwrap_err();
        assert_eq!(err, MilitarySuppliesUnitsError::NonFinite);
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 10.0);
    }

    #[test]
    fn military_supplies_and_military_equipment_stockpiles_are_independent() {
        let mut stockpile = CountryStockpile::new();
        stockpile.add_military_equipment_produced(50.0).unwrap();
        stockpile.add_military_supplies_produced(30.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 50.0);
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 30.0);

        stockpile.try_consume_military_supplies(10.0).unwrap();
        assert_eq!(stockpile.get(ResourceType::MilitaryEquipment), 50.0);
        assert_eq!(stockpile.get(ResourceType::MilitarySupplies), 20.0);
    }
}
