/// 陸上戦闘の計算ロジックモジュール
/// 乱数を使わず、整数演算で決定的な戦闘計算を行う
use crate::common::DivisionId;
use crate::economy::resources::{MilitaryEquipmentUnitsError, military_equipment_value_to_units};
use crate::military::data::Division;
use crate::state::data::StateData;
use std::collections::HashMap;

// ─── 戦闘定数 ────────────────────────────────────────────────────────────────
/// 1日あたりの基準ダメージスケール（実効攻撃力を100倍してから適用）
pub const DAMAGE_SCALE: i32 = 100;

/// 組織率ダメージ比率の分子
pub const ORG_DAMAGE_NUMERATOR: i32 = 1;

/// 組織率ダメージ比率の分母（manpower損失の 1/10 を組織率損失とする）
pub const ORG_DAMAGE_DENOMINATOR: i32 = 10;

/// 戦闘不能とみなす最小組織率（これ未満で撤退）
pub const MIN_ORGANIZATION: f32 = 0.0;

/// 1日あたりの組織率回復量（非戦闘・非移動・自国支配地域）
pub const ORG_RECOVERY_PER_DAY: f32 = 5.0;

/// 撤退時に追加で失うmanpowerの割合（最大manpowerに対する比率）
/// 組織率0での敗走は削除されず生存し続けるため、これがないと同じ師団が
/// 「撤退→組織率回復→再戦」を無限に繰り返してしまう。撤退のたびにこの分だけ
/// 追加でmanpowerを失わせることで、負け続ける師団は数回の敗北で消耗しきる。
pub const RETREAT_MANPOWER_LOSS_RATIO: f32 = 0.1;

// ─── 地形防御補正 ─────────────────────────────────────────────────────────────
/// 平地の防御補正（補正なし）
pub const TERRAIN_BONUS_PLAIN: i32 = 0;

/// 森林の防御補正（小さな有利）
pub const TERRAIN_BONUS_FOREST: i32 = 5;

/// 丘陵・高原の防御補正（中程度の有利）
pub const TERRAIN_BONUS_HILLS: i32 = 10;

/// 山岳の防御補正（大きな有利）
pub const TERRAIN_BONUS_MOUNTAIN: i32 = 20;

/// 地域の地形から防御補正を取得する
/// 現段階では地形フィールドが存在しないため全て平地扱い（TERRAIN_BONUS_PLAIN）
/// 将来 StateData に terrain フィールドが追加された際はここで分岐する
pub fn calculate_terrain_defense_bonus(_state: &StateData) -> i32 {
    // TODO: Phase-N で地形フィールドを追加した際にここを拡張する
    // match state.terrain {
    //     Terrain::Forest => TERRAIN_BONUS_FOREST,
    //     Terrain::Hills  => TERRAIN_BONUS_HILLS,
    //     Terrain::Mountain => TERRAIN_BONUS_MOUNTAIN,
    //     _ => TERRAIN_BONUS_PLAIN,
    // }
    TERRAIN_BONUS_PLAIN
}

// ─── 戦闘充足率(P25-007) ──────────────────────────────────────────────────

/// 戦闘充足率専用の固定小数点scale。装備数量のscale
/// (`economy::resources::MILITARY_EQUIPMENT_SCALE`)とたまたま同じ1,000,000だが、
/// 意味の異なる別概念(manpower/equipmentのうち小さい方が戦闘力を制限する度合い)
/// であるため、独立した定数として持つ(値を複製しているわけではない)。
pub const COMBAT_READINESS_SCALE: u64 = 1_000_000;

/// `calculate_combat_readiness`が失敗し得る理由。呼び出し元
/// (`military::update::process_daily_battles`)が、戦闘のRNG消費・状態更新前に
/// 全参加Divisionをこの関数で検証し、1件でも`Err`ならそのBattleの当日解決を
/// 丸ごとskipする(原子性、§15)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CombatReadinessError {
    /// 装備量が非有限(NaN/Infinity)。
    NonFiniteEquipment,
    /// 装備量が負。
    NegativeEquipment,
    /// 装備量が正規最大unitsを超える、または固定小数点往復変換に失敗。
    EquipmentOutOfRange,
    /// `equipment > max_equipment`(不正状態、黙ってclampしない)。
    EquipmentExceedsMax,
    /// `manpower > max_manpower`(不正状態、黙ってclampしない)。
    ManpowerExceedsMax,
    /// units演算がoverflowした(正規最大値の範囲内では到達しないはず)。
    ArithmeticOverflow,
}

impl From<MilitaryEquipmentUnitsError> for CombatReadinessError {
    fn from(err: MilitaryEquipmentUnitsError) -> Self {
        match err {
            MilitaryEquipmentUnitsError::NonFinite => CombatReadinessError::NonFiniteEquipment,
            MilitaryEquipmentUnitsError::Negative => CombatReadinessError::NegativeEquipment,
            MilitaryEquipmentUnitsError::OutOfRange | MilitaryEquipmentUnitsError::RoundTripFailed => {
                CombatReadinessError::EquipmentOutOfRange
            }
        }
    }
}

/// manpower充足率・装備充足率・戦闘充足率(両者のうち小さい方)を
/// `COMBAT_READINESS_SCALE`単位のper-million固定小数点で保持する(P25-007)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatReadiness {
    pub manpower_readiness_units: u64,
    pub equipment_readiness_units: u64,
    pub combat_readiness_units: u64,
}

impl CombatReadiness {
    /// `combat_readiness_units`を`0.0`〜`1.0`の`f64`係数へ変換する
    /// (§7: 「最終的にattack／defenseへ適用するときだけ」既存の戦闘数値型へ変換する)。
    pub fn factor(self) -> f64 {
        self.combat_readiness_units as f64 / COMBAT_READINESS_SCALE as f64
    }
}

/// manpower充足率(`m = M/Mmax`)と装備充足率(`e = E/Emax`)のうち小さい方
/// (`r = min(m, e)`)を、正規units上でchecked `u128`演算により算出する(P25-007)。
///
/// `m`と`e`をそれぞれ独立に計算してから`min`を取ることで、同じ損失を
/// manpowerと装備の2経路で二重評価しない(`min(50%,50%)=50%`、`50%×50%=25%`には
/// しない、§5)。
///
/// # 特別扱い
/// * `max_equipment == 0`(装備を必要としないDivision)は`equipment_readiness_units
///   = COMBAT_READINESS_SCALE`(100%)として扱う(§8.2)。
/// * `max_manpower == 0`は`manpower_readiness_units = 0`として扱う(§8.4、
///   ゼロ除算はしない)。
/// * `manpower > max_manpower`または`equipment > max_equipment`は、黙って
///   100%へclampせず`Err`として明示的に拒否する(§8.5)。
pub fn calculate_combat_readiness(
    manpower: u64,
    max_manpower: u64,
    equipment: f64,
    max_equipment: f64,
) -> Result<CombatReadiness, CombatReadinessError> {
    if manpower > max_manpower {
        return Err(CombatReadinessError::ManpowerExceedsMax);
    }
    let manpower_readiness_units = if max_manpower == 0 {
        0
    } else {
        let numerator = u128::from(manpower)
            .checked_mul(u128::from(COMBAT_READINESS_SCALE))
            .ok_or(CombatReadinessError::ArithmeticOverflow)?;
        (numerator / u128::from(max_manpower)) as u64
    };

    let equipment_units = military_equipment_value_to_units(equipment)?;
    let max_equipment_units = military_equipment_value_to_units(max_equipment)?;
    if equipment_units > max_equipment_units {
        return Err(CombatReadinessError::EquipmentExceedsMax);
    }
    let equipment_readiness_units = if max_equipment_units == 0 {
        COMBAT_READINESS_SCALE
    } else {
        let numerator = u128::from(equipment_units)
            .checked_mul(u128::from(COMBAT_READINESS_SCALE))
            .ok_or(CombatReadinessError::ArithmeticOverflow)?;
        (numerator / u128::from(max_equipment_units)) as u64
    };

    let combat_readiness_units = manpower_readiness_units.min(equipment_readiness_units);

    Ok(CombatReadiness {
        manpower_readiness_units,
        equipment_readiness_units,
        combat_readiness_units,
    })
}

/// `calculate_combat_readiness`の結果を、既存の戦闘式が使う固定小数点scale(×1000)
/// へ変換した`i64`として返す純粋・infallibleなヘルパー。呼び出し元
/// (`resolve_combat_day`/`effective_power_1000`)は、Battleの当日解決前に
/// `process_daily_battles`が既に全参加者を`calculate_combat_readiness`で
/// 検証済みであることを前提とするため、万一失敗した場合(到達不能のはず)は
/// 安全に0を返す(装備0扱いとして戦闘力を持たせない、panicしない)。
///
/// `COMBAT_READINESS_SCALE(1,000,000)`から既存scale(1,000)への変換は、
/// `floor(floor(a×1,000,000/b)/1,000) = floor(a×1,000/b)`という整数除算の
/// 合成則により、直接scale1,000で計算した場合と完全に一致する(精度劣化なし)。
fn combat_readiness_1000(manpower: u64, max_manpower: u64, equipment: f64, max_equipment: f64) -> i64 {
    calculate_combat_readiness(manpower, max_manpower, equipment, max_equipment)
        .map(|r| (r.combat_readiness_units / 1000) as i64)
        .unwrap_or(0)
}

/// 1日分の戦闘計算を行う（乱数なし・決定的）
///
/// # 計算式
/// 実効攻撃力 = attack_power * combat_readiness * (現在organization / max_organization)
/// ※ 固定小数点（整数×1000）で計算して最後に1000で割る
/// combat_readiness = min(現在manpower/max_manpower, 現在equipment/max_equipment)
/// (P25-007: 単純なmanpower比率ではなく、manpower/装備充足率のうち小さい方を使う。
/// `calculate_combat_readiness`参照)
///
/// 防御側が受けるmanpowerダメージ = attacker_effective * DAMAGE_SCALE / max(defender_effective + terrain_bonus, 1)
/// 攻撃側が受けるmanpowerダメージ = (defender_effective + terrain_bonus) * DAMAGE_SCALE / max(attacker_effective, 1)
///
/// 組織率ダメージ = manpower_loss * ORG_DAMAGE_NUMERATOR / ORG_DAMAGE_DENOMINATOR
///
/// # 返り値
/// (攻撃側manpower損失, 攻撃側organization損失×100, 防御側manpower損失, 防御側organization損失×100)
/// organization損失は f32 として返す（整数化は呼び出し側で行う）
///
/// # 引数
/// * `attacker` - 攻撃側ユニット
/// * `defender` - 防御側ユニット
/// * `terrain_defense_bonus` - 地形による防御補正（整数）
pub fn resolve_combat_day(
    attacker: &Division,
    defender: &Division,
    terrain_defense_bonus: i32,
) -> (u64, f32, u64, f32) {
    // ゼロ除算防止
    if attacker.max_manpower == 0 || defender.max_manpower == 0 {
        return (0, 0.0, 0, 0.0);
    }
    if attacker.max_organization <= 0.0 || defender.max_organization <= 0.0 {
        return (0, 0.0, 0, 0.0);
    }

    // 実効攻撃力を固定小数点（×1000）で計算
    // attack_effective = attack_power * combat_readiness * org_ratio（0〜attack_power*1000）
    // P25-007: manpower ratio単独ではなく、manpower/装備充足率のうち小さい方
    // (戦闘充足率、combat_readiness_1000)を使う。
    let atk_readiness_1000 = combat_readiness_1000(
        attacker.manpower,
        attacker.max_manpower,
        attacker.equipment,
        attacker.max_equipment,
    );
    let atk_org_ratio_1000 = (attacker.organization * 1000.0 / attacker.max_organization) as i64;
    let atk_effective_1000 =
        (attacker.attack_power as i64 * atk_readiness_1000 * atk_org_ratio_1000) / 1_000_000;

    let def_readiness_1000 = combat_readiness_1000(
        defender.manpower,
        defender.max_manpower,
        defender.equipment,
        defender.max_equipment,
    );
    let def_org_ratio_1000 = (defender.organization * 1000.0 / defender.max_organization) as i64;
    let def_effective_1000 =
        (defender.defense_power as i64 * def_readiness_1000 * def_org_ratio_1000) / 1_000_000;

    // 地形補正を防御側の実効値に加算（固定小数点×1000）
    let def_effective_with_terrain = def_effective_1000 + terrain_defense_bonus as i64;

    // 防御側が受けるmanpowerダメージ
    let def_manpower_loss = if def_effective_with_terrain < 1 {
        (atk_effective_1000 * DAMAGE_SCALE as i64 / 1000).max(0) as u64
    } else {
        (atk_effective_1000 * DAMAGE_SCALE as i64 / def_effective_with_terrain.max(1)).max(0) as u64
    };

    // 攻撃側が受けるmanpowerダメージ
    let atk_manpower_loss = if atk_effective_1000 < 1 {
        (def_effective_with_terrain * DAMAGE_SCALE as i64 / 1000).max(0) as u64
    } else {
        (def_effective_with_terrain * DAMAGE_SCALE as i64 / atk_effective_1000.max(1)).max(0) as u64
    };

    // 組織率ダメージ（manpower_loss の 1/ORG_DAMAGE_DENOMINATOR）
    let atk_org_loss =
        atk_manpower_loss as f32 * ORG_DAMAGE_NUMERATOR as f32 / ORG_DAMAGE_DENOMINATOR as f32;
    let def_org_loss =
        def_manpower_loss as f32 * ORG_DAMAGE_NUMERATOR as f32 / ORG_DAMAGE_DENOMINATOR as f32;

    (
        atk_manpower_loss,
        atk_org_loss,
        def_manpower_loss,
        def_org_loss,
    )
}

/// 実効戦力(atk_power×戦闘充足率×組織率比率、固定小数点×1000)を算出する。
/// `resolve_combat_day`内のインライン計算と同一の式を共有部品として切り出したもの。
/// P25-007: manpower比率単独ではなく`combat_readiness_1000`(manpower/装備充足率の
/// うち小さい方)を使う。
#[allow(clippy::too_many_arguments)]
fn effective_power_1000(
    power: i32,
    manpower: u64,
    max_manpower: u64,
    equipment: f64,
    max_equipment: f64,
    organization: f32,
    max_organization: f32,
) -> i64 {
    if max_manpower == 0 || max_organization <= 0.0 {
        return 0;
    }
    let readiness_1000 = combat_readiness_1000(manpower, max_manpower, equipment, max_equipment);
    let org_ratio_1000 = (organization * 1000.0 / max_organization) as i64;
    (power as i64 * readiness_1000 * org_ratio_1000) / 1_000_000
}

/// 複数師団が参加する戦闘の1日分を計算する(P21-siege: 複数師団の共同参戦対応)。
///
/// 各陣営の実効戦力は参加する全師団の実効値の合計として算出し、`resolve_combat_day`と
/// 同じ被害率の式を陣営合計に対して適用したうえで、生じた被害を各師団の実効戦力の
/// 貢献度に応じて配分する(貢献度が大きい師団ほど多くの被害を受け持つ)。
/// 攻撃側・防御側とも参加師団が1体ずつの場合は`resolve_combat_day`をそのまま呼び出し、
/// 既存の1体対1体の戦闘結果と完全に同一になることを保証する。
///
/// # 返り値
/// (攻撃側の`DivisionId`→(manpower損失, organization損失)、防御側の同上)
pub fn resolve_combat_day_multi(
    attackers: &[&Division],
    defenders: &[&Division],
    terrain_defense_bonus: i32,
) -> (CombatDamageByDivision, CombatDamageByDivision) {
    if attackers.len() == 1 && defenders.len() == 1 {
        // 1体対1体は既存の`resolve_combat_day`に完全委譲し、既存の戦闘バランスを保つ
        let (atk_loss, atk_org, def_loss, def_org) =
            resolve_combat_day(attackers[0], defenders[0], terrain_defense_bonus);
        let mut atk_result = HashMap::new();
        atk_result.insert(attackers[0].id, (atk_loss, atk_org));
        let mut def_result = HashMap::new();
        def_result.insert(defenders[0].id, (def_loss, def_org));
        return (atk_result, def_result);
    }

    let atk_effectives: Vec<(DivisionId, i64)> = attackers
        .iter()
        .map(|a| {
            (
                a.id,
                effective_power_1000(
                    a.attack_power,
                    a.manpower,
                    a.max_manpower,
                    a.equipment,
                    a.max_equipment,
                    a.organization,
                    a.max_organization,
                ),
            )
        })
        .collect();
    let def_effectives: Vec<(DivisionId, i64)> = defenders
        .iter()
        .map(|d| {
            (
                d.id,
                effective_power_1000(
                    d.defense_power,
                    d.manpower,
                    d.max_manpower,
                    d.equipment,
                    d.max_equipment,
                    d.organization,
                    d.max_organization,
                ),
            )
        })
        .collect();

    let atk_total: i64 = atk_effectives.iter().map(|(_, v)| v).sum();
    let def_total: i64 = def_effectives.iter().map(|(_, v)| v).sum();
    let def_total_with_terrain = def_total + terrain_defense_bonus as i64;

    // 防御側全体が受けるmanpower損失合計
    let def_total_loss = if def_total_with_terrain < 1 {
        (atk_total * DAMAGE_SCALE as i64 / 1000).max(0)
    } else {
        (atk_total * DAMAGE_SCALE as i64 / def_total_with_terrain.max(1)).max(0)
    } as u64;

    // 攻撃側全体が受けるmanpower損失合計
    let atk_total_loss = if atk_total < 1 {
        (def_total_with_terrain * DAMAGE_SCALE as i64 / 1000).max(0)
    } else {
        (def_total_with_terrain * DAMAGE_SCALE as i64 / atk_total.max(1)).max(0)
    } as u64;

    (
        distribute_damage(&atk_effectives, atk_total, atk_total_loss),
        distribute_damage(&def_effectives, def_total, def_total_loss),
    )
}

/// 師団ID→(manpower損失, organization損失)のマップ。`resolve_combat_day_multi`の
/// 陣営ごとの結果を表す型(clippy::type_complexity対策の型エイリアス)。
pub type CombatDamageByDivision = HashMap<DivisionId, (u64, f32)>;

/// 陣営全体の損失合計を、各師団の実効戦力の貢献度に応じて配分する。
/// 端数はIDの大きい参加者(決定的な順序の末尾)にまとめて渡し、配分合計が
/// `total_loss`と一致するようにする。実効戦力の合計が0(陣営全滅済み)の場合は
/// 誰にも被害を配分しない。
fn distribute_damage(
    effectives: &[(DivisionId, i64)],
    total_effective: i64,
    total_loss: u64,
) -> CombatDamageByDivision {
    let mut result = HashMap::new();
    if effectives.is_empty() {
        return result;
    }
    if total_effective <= 0 {
        for (id, _) in effectives {
            result.insert(*id, (0, 0.0));
        }
        return result;
    }

    let mut sorted = effectives.to_vec();
    sorted.sort_by_key(|(id, _)| id.0);

    let mut distributed = 0u64;
    let last_index = sorted.len() - 1;
    for (i, (id, eff)) in sorted.iter().enumerate() {
        let share = if i == last_index {
            total_loss.saturating_sub(distributed)
        } else {
            let s = ((total_loss as i64 * eff) / total_effective).max(0) as u64;
            distributed += s;
            s
        };
        let org_loss = share as f32 * ORG_DAMAGE_NUMERATOR as f32 / ORG_DAMAGE_DENOMINATOR as f32;
        result.insert(*id, (share, org_loss));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{BattleId, CountryId, DivisionDefinitionId, DivisionId, StateId};
    use crate::military::data::{Division, DivisionSize, DivisionStatus, DivisionType};

    fn make_division(owner: CountryId, manpower: u64, org: f32, atk: i32, def: i32) -> Division {
        Division {
            id: DivisionId(0),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(1),
            destination: None,
            current_path: vec![],
            target_state: None,
            manpower,
            max_manpower: 10000,
            equipment: 100.0,
            max_equipment: 100.0,
            organization: org,
            max_organization: 100.0,
            morale: 100.0,
            max_morale: 100.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Fighting,
            def_id: DivisionDefinitionId(0),
            attack_power: atk,
            defense_power: def,
            combat_id: Some(BattleId(0)),
        }
    }

    #[test]
    fn test_resolve_combat_day_basic() {
        let attacker = make_division(CountryId(1), 10000, 100.0, 10, 10);
        let defender = make_division(CountryId(2), 10000, 100.0, 10, 10);

        let (atk_loss, _atk_org, def_loss, _def_org) =
            resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);

        // 均等な場合、両者のダメージは同程度
        assert_eq!(atk_loss, def_loss);
        assert!(atk_loss > 0);
    }

    #[test]
    fn test_terrain_bonus_favors_defender() {
        let attacker = make_division(CountryId(1), 10000, 100.0, 10, 10);
        let defender = make_division(CountryId(2), 10000, 100.0, 10, 10);

        let (atk_loss_plain, _, def_loss_plain, _) =
            resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);
        let (atk_loss_mountain, _, def_loss_mountain, _) =
            resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_MOUNTAIN);

        // 山岳補正があると攻撃側の損失が増え、防御側の損失が減る
        assert!(atk_loss_mountain > atk_loss_plain);
        assert!(def_loss_mountain < def_loss_plain);
    }

    #[test]
    fn test_combat_is_deterministic() {
        let attacker = make_division(CountryId(1), 8000, 75.0, 12, 8);
        let defender = make_division(CountryId(2), 6000, 60.0, 9, 14);

        let result1 = resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_HILLS);
        let result2 = resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_HILLS);

        assert_eq!(result1, result2);
    }

    #[test]
    fn test_zero_manpower_no_damage() {
        let attacker = make_division(CountryId(1), 0, 100.0, 10, 10);
        let defender = make_division(CountryId(2), 10000, 100.0, 10, 10);

        let (atk_loss, _, def_loss, _) =
            resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);

        // 攻撃側の実効が0なので防御側のダメージも0
        assert_eq!(def_loss, 0);
        // 攻撃側はダメージを受ける
        let _ = atk_loss;
    }

    fn make_division_with_id(
        id: usize,
        owner: CountryId,
        manpower: u64,
        org: f32,
        atk: i32,
        def: i32,
    ) -> Division {
        let mut d = make_division(owner, manpower, org, atk, def);
        d.id = DivisionId(id);
        d
    }

    #[test]
    fn test_resolve_combat_day_multi_matches_single_when_1v1() {
        let attacker = make_division_with_id(0, CountryId(1), 8000, 75.0, 12, 8);
        let defender = make_division_with_id(1, CountryId(2), 6000, 60.0, 9, 14);

        let (single_atk_loss, single_atk_org, single_def_loss, single_def_org) =
            resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_HILLS);

        let (atk_result, def_result) =
            resolve_combat_day_multi(&[&attacker], &[&defender], TERRAIN_BONUS_HILLS);

        // 1体対1体の場合、既存のresolve_combat_dayと完全に同一の結果になる
        assert_eq!(
            atk_result.get(&attacker.id),
            Some(&(single_atk_loss, single_atk_org))
        );
        assert_eq!(
            def_result.get(&defender.id),
            Some(&(single_def_loss, single_def_org))
        );
    }

    #[test]
    fn test_resolve_combat_day_multi_combined_attackers_deal_more_total_damage() {
        // 単独の攻撃側1体 vs 防御側1体
        let solo_attacker = make_division_with_id(0, CountryId(1), 10000, 100.0, 10, 10);
        let defender_vs_solo = make_division_with_id(1, CountryId(2), 10000, 100.0, 10, 15);
        let (_, def_result_solo) =
            resolve_combat_day_multi(&[&solo_attacker], &[&defender_vs_solo], TERRAIN_BONUS_PLAIN);
        let solo_def_loss = def_result_solo[&defender_vs_solo.id].0;

        // 同条件の攻撃側3体が合流した場合
        let attacker_a = make_division_with_id(10, CountryId(1), 10000, 100.0, 10, 10);
        let attacker_b = make_division_with_id(11, CountryId(1), 10000, 100.0, 10, 10);
        let attacker_c = make_division_with_id(12, CountryId(1), 10000, 100.0, 10, 10);
        let defender_vs_group = make_division_with_id(1, CountryId(2), 10000, 100.0, 10, 15);

        let (_, def_result_group) = resolve_combat_day_multi(
            &[&attacker_a, &attacker_b, &attacker_c],
            &[&defender_vs_group],
            TERRAIN_BONUS_PLAIN,
        );
        let group_def_loss = def_result_group[&defender_vs_group.id].0;

        // 3体で合流した方が、単独攻撃よりも防御側に大きな被害を与える
        // (複数師団による共同攻撃が根本的に強くなることの直接的な検証)
        assert!(group_def_loss > solo_def_loss * 2);
    }

    #[test]
    fn test_resolve_combat_day_multi_distributes_damage_across_defenders() {
        let attacker = make_division_with_id(0, CountryId(1), 10000, 100.0, 10, 10);
        let defender_a = make_division_with_id(1, CountryId(2), 10000, 100.0, 10, 10);
        let defender_b = make_division_with_id(2, CountryId(2), 10000, 100.0, 10, 10);

        let (_, def_result) = resolve_combat_day_multi(
            &[&attacker],
            &[&defender_a, &defender_b],
            TERRAIN_BONUS_PLAIN,
        );

        // 実効戦力が同じ2体の防御側には、被害がほぼ均等(端数のみ)に配分される
        let loss_a = def_result[&defender_a.id].0;
        let loss_b = def_result[&defender_b.id].0;
        assert!(loss_a > 0 && loss_b > 0);
        assert!(loss_a.abs_diff(loss_b) <= 1);
    }

    #[test]
    fn test_resolve_combat_day_multi_defeated_participant_takes_no_further_damage() {
        // organization/manpowerが0の師団は実効戦力0となり、被害を受け持たない
        let attacker = make_division_with_id(0, CountryId(1), 10000, 100.0, 10, 10);
        let mut defeated_defender = make_division_with_id(1, CountryId(2), 10000, 0.0, 10, 10);
        defeated_defender.organization = 0.0;
        let alive_defender = make_division_with_id(2, CountryId(2), 10000, 100.0, 10, 10);

        let (_, def_result) = resolve_combat_day_multi(
            &[&attacker],
            &[&defeated_defender, &alive_defender],
            TERRAIN_BONUS_PLAIN,
        );

        assert_eq!(def_result[&defeated_defender.id].0, 0);
        assert!(def_result[&alive_defender.id].0 > 0);
    }

    // ─── P25-007: calculate_combat_readiness(§18の数値例) ─────────────────

    #[test]
    fn readiness_18_1_full_readiness_is_exactly_one_million() {
        let r = calculate_combat_readiness(100, 100, 100.0, 100.0).unwrap();
        assert_eq!(r.manpower_readiness_units, 1_000_000);
        assert_eq!(r.equipment_readiness_units, 1_000_000);
        assert_eq!(r.combat_readiness_units, 1_000_000);
        assert_eq!(r.factor(), 1.0);
    }

    #[test]
    fn readiness_18_2_equipment_shortage_limits_readiness_to_fifty_percent() {
        let r = calculate_combat_readiness(100, 100, 50.0, 100.0).unwrap();
        assert_eq!(r.manpower_readiness_units, 1_000_000);
        assert_eq!(r.equipment_readiness_units, 500_000);
        assert_eq!(r.combat_readiness_units, 500_000);
    }

    #[test]
    fn readiness_18_3_manpower_shortage_limits_readiness_to_fifty_percent() {
        let r = calculate_combat_readiness(50, 100, 100.0, 100.0).unwrap();
        assert_eq!(r.manpower_readiness_units, 500_000);
        assert_eq!(r.equipment_readiness_units, 1_000_000);
        assert_eq!(r.combat_readiness_units, 500_000);
    }

    #[test]
    fn readiness_18_4_equal_shortage_on_both_axes_stays_fifty_percent_not_twenty_five() {
        let r = calculate_combat_readiness(50, 100, 50.0, 100.0).unwrap();
        assert_eq!(r.combat_readiness_units, 500_000, "min(50%,50%)=50%であり、50%×50%=25%にしてはいけない");
    }

    #[test]
    fn readiness_18_5_equipment_is_the_limiting_factor() {
        // manpower比率80%、装備比率50% → 50%が制限要因
        let r = calculate_combat_readiness(80, 100, 50.0, 100.0).unwrap();
        assert_eq!(r.manpower_readiness_units, 800_000);
        assert_eq!(r.equipment_readiness_units, 500_000);
        assert_eq!(r.combat_readiness_units, 500_000);
    }

    #[test]
    fn readiness_18_6_manpower_is_the_limiting_factor() {
        // manpower比率50%、装備比率80% → 50%が制限要因
        let r = calculate_combat_readiness(50, 100, 80.0, 100.0).unwrap();
        assert_eq!(r.manpower_readiness_units, 500_000);
        assert_eq!(r.equipment_readiness_units, 800_000);
        assert_eq!(r.combat_readiness_units, 500_000);
    }

    #[test]
    fn readiness_18_7_fractional_equipment_three_point_seven_over_five() {
        let r = calculate_combat_readiness(100, 100, 3.7, 5.0).unwrap();
        assert_eq!(r.equipment_readiness_units, 740_000);
        assert_eq!(r.combat_readiness_units, 740_000);
        assert_eq!(r.factor(), 0.74);
    }

    #[test]
    fn readiness_18_8_zero_max_equipment_division_is_full_equipment_readiness() {
        let r = calculate_combat_readiness(100, 100, 0.0, 0.0).unwrap();
        assert_eq!(
            r.equipment_readiness_units, 1_000_000,
            "max_equipment=0(装備不要Division)は装備充足率100%として扱う"
        );
        assert_eq!(r.combat_readiness_units, 1_000_000);
    }

    #[test]
    fn readiness_18_9_zero_equipment_with_nonzero_max_is_zero_readiness() {
        let r = calculate_combat_readiness(100, 100, 0.0, 100.0).unwrap();
        assert_eq!(r.equipment_readiness_units, 0);
        assert_eq!(r.combat_readiness_units, 0);
    }

    // ─── 境界条件(§8) ─────────────────────────────────────────────────────

    #[test]
    fn readiness_rejects_equipment_exceeding_max_equipment_instead_of_clamping() {
        let err = calculate_combat_readiness(100, 100, 101.0, 100.0).unwrap_err();
        assert_eq!(err, CombatReadinessError::EquipmentExceedsMax);
    }

    #[test]
    fn readiness_rejects_max_equipment_zero_with_nonzero_equipment() {
        let err = calculate_combat_readiness(100, 100, 1.0, 0.0).unwrap_err();
        assert_eq!(err, CombatReadinessError::EquipmentExceedsMax);
    }

    #[test]
    fn readiness_rejects_manpower_exceeding_max_manpower_instead_of_clamping() {
        let err = calculate_combat_readiness(101, 100, 100.0, 100.0).unwrap_err();
        assert_eq!(err, CombatReadinessError::ManpowerExceedsMax);
    }

    #[test]
    fn readiness_rejects_max_manpower_zero_with_nonzero_manpower() {
        let err = calculate_combat_readiness(1, 0, 100.0, 100.0).unwrap_err();
        assert_eq!(err, CombatReadinessError::ManpowerExceedsMax);
    }

    #[test]
    fn readiness_max_manpower_zero_with_zero_manpower_is_zero_readiness_not_an_error() {
        let r = calculate_combat_readiness(0, 0, 100.0, 100.0).unwrap();
        assert_eq!(r.manpower_readiness_units, 0);
        assert_eq!(r.combat_readiness_units, 0);
    }

    #[test]
    fn readiness_rejects_non_finite_or_negative_equipment() {
        assert_eq!(
            calculate_combat_readiness(100, 100, f64::NAN, 100.0).unwrap_err(),
            CombatReadinessError::NonFiniteEquipment
        );
        assert_eq!(
            calculate_combat_readiness(100, 100, f64::INFINITY, 100.0).unwrap_err(),
            CombatReadinessError::NonFiniteEquipment
        );
        assert_eq!(
            calculate_combat_readiness(100, 100, -1.0, 100.0).unwrap_err(),
            CombatReadinessError::NegativeEquipment
        );
    }

    #[test]
    fn readiness_checked_u128_boundary_near_maximum_scale_does_not_overflow() {
        // 正規最大装備量(MILITARY_EQUIPMENT_MAX_UNITS/SCALE = 1e9)とu64::MAXに近い
        // manpowerでもoverflowせず正確に計算できる。
        let r = calculate_combat_readiness(u64::MAX, u64::MAX, 1_000_000_000.0, 1_000_000_000.0)
            .unwrap();
        assert_eq!(r.combat_readiness_units, 1_000_000);
    }

    #[test]
    fn readiness_is_the_same_function_used_regardless_of_attack_or_defense_role() {
        // §9: 同じDivisionが攻撃側・防御側のどちらでも同じ戦闘充足率関数を使う。
        // calculate_combat_readinessはattacker/defenderの区別を引数に取らない
        // ため、構造的に同一である(この呼び出しが両方の役割で使われることの確認)。
        let a = calculate_combat_readiness(50, 100, 80.0, 100.0).unwrap();
        let b = calculate_combat_readiness(50, 100, 80.0, 100.0).unwrap();
        assert_eq!(a, b);
    }
}
