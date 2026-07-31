/// 陸上戦闘の計算ロジックモジュール
/// 乱数を使わず、整数演算で決定的な戦闘計算を行う
use crate::military::data::ArmyUnit;
use crate::state::data::StateData;

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

/// 1日分の戦闘計算を行う（乱数なし・決定的）
///
/// # 計算式
/// 実効攻撃力 = attack_power * (現在manpower / max_manpower) * (現在organization / max_organization)
/// ※ 固定小数点（整数×1000）で計算して最後に1000で割る
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
    attacker: &ArmyUnit,
    defender: &ArmyUnit,
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
    // attack_effective = attack_power * manpower_ratio * org_ratio（0〜attack_power*1000）
    let atk_manpower_ratio_1000 = (attacker.manpower as i64 * 1000) / attacker.max_manpower as i64;
    let atk_org_ratio_1000 = (attacker.organization * 1000.0 / attacker.max_organization) as i64;
    let atk_effective_1000 =
        (attacker.attack_power as i64 * atk_manpower_ratio_1000 * atk_org_ratio_1000) / 1_000_000;

    let def_manpower_ratio_1000 = (defender.manpower as i64 * 1000) / defender.max_manpower as i64;
    let def_org_ratio_1000 = (defender.organization * 1000.0 / defender.max_organization) as i64;
    let def_effective_1000 =
        (defender.defense_power as i64 * def_manpower_ratio_1000 * def_org_ratio_1000) / 1_000_000;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{ArmyId, BattleId, CountryId, DivisionId, StateId};
    use crate::military::data::{ArmyStatus, ArmyUnit, DivisionSize, DivisionType};

    fn make_army(owner: CountryId, manpower: u64, org: f32, atk: i32, def: i32) -> ArmyUnit {
        ArmyUnit {
            id: ArmyId(0),
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
            status: ArmyStatus::Fighting,
            def_id: DivisionId(0),
            attack_power: atk,
            defense_power: def,
            combat_id: Some(BattleId(0)),
        }
    }

    #[test]
    fn test_resolve_combat_day_basic() {
        let attacker = make_army(CountryId(1), 10000, 100.0, 10, 10);
        let defender = make_army(CountryId(2), 10000, 100.0, 10, 10);

        let (atk_loss, _atk_org, def_loss, _def_org) =
            resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);

        // 均等な場合、両者のダメージは同程度
        assert_eq!(atk_loss, def_loss);
        assert!(atk_loss > 0);
    }

    #[test]
    fn test_terrain_bonus_favors_defender() {
        let attacker = make_army(CountryId(1), 10000, 100.0, 10, 10);
        let defender = make_army(CountryId(2), 10000, 100.0, 10, 10);

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
        let attacker = make_army(CountryId(1), 8000, 75.0, 12, 8);
        let defender = make_army(CountryId(2), 6000, 60.0, 9, 14);

        let result1 = resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_HILLS);
        let result2 = resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_HILLS);

        assert_eq!(result1, result2);
    }

    #[test]
    fn test_zero_manpower_no_damage() {
        let attacker = make_army(CountryId(1), 0, 100.0, 10, 10);
        let defender = make_army(CountryId(2), 10000, 100.0, 10, 10);

        let (atk_loss, _, def_loss, _) =
            resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);

        // 攻撃側の実効が0なので防御側のダメージも0
        assert_eq!(def_loss, 0);
        // 攻撃側はダメージを受ける
        let _ = atk_loss;
    }
}
