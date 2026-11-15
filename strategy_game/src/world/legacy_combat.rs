//! P32-035 (M6-b): 旧経路の戦闘計算(`military::combat_calc`・`military::equipment`)を、新経路の師団の**戦闘能力の値だけ**で再利用するための隔離アダプタ。
//!
//! 旧の計算関数は旧 `Division`(位置・命令・`StateId` を含む)と `MilitaryRegistry`(補給充足率)を受け取る。新経路の師団(`MapDivision`)は旧型へ変換しない
//! (識別・位置は渡さない)。ここでは、**計算の間だけ**、戦闘能力の値(`CombatStats`)から旧 `Division` の一時的な値を作って既存の関数を呼び、結果を `MapDivisionId` に戻す
//! (一時的な `DivisionId` は `MapDivisionId` の数値をそのまま使うが、旧のレジストリ・戦闘・状態へは一切渡さない)。補給充足率は **1.0 固定**(M6-d で接続)。
//! このモジュールだけが旧の戦闘計算に依存する。計算式・損害・撤退時の追加損失・装備の破壊は旧の関数をそのまま呼ぶ(複製しない)。
//!
//! 再利用できた範囲: 日次の戦闘計算(陸上の1対1・複数師団)、準備率の検証、戦闘損耗(人員・装備・組織率)、撤退時の追加損失と装備の破壊、組織率の回復量。
//! 再利用していない範囲(後続の課題): 日次補給の充足率の接続(`supply_fulfillment`)、地形補正(旧は `TERRAIN_BONUS_PLAIN` = 0 固定)、補充・装備補充・募兵、戦争点数。

use super::movement::{CombatStats, MapDivisionId};
use crate::common::{CountryId, DivisionDefinitionId, DivisionId, StateId};
use crate::military::combat_calc::{
    ORG_RECOVERY_PER_DAY, RETREAT_MANPOWER_LOSS_RATIO, TERRAIN_BONUS_PLAIN, calculate_combat_readiness_with_supply, resolve_combat_day_multi_with_supply,
};
use crate::military::data::{Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry};
use crate::military::equipment::{calculate_proportional_equipment_loss, try_destroy_equipment};
use std::collections::BTreeMap;

/// 1日分の戦闘計算の結果。
#[derive(Debug, Clone, PartialEq)]
pub struct DayDamage {
    /// 師団ごとの (人員損失, 組織率損失)。
    pub attackers: BTreeMap<MapDivisionId, (u64, f32)>,
    pub defenders: BTreeMap<MapDivisionId, (u64, f32)>,
    /// 両陣営とも人員損失が 0(旧 P27-007 の「ゼロ損害」)。
    pub zero_damage: bool,
}

/// 計算の間だけの旧 `Division` の値(戦闘能力のみ。位置・命令は持たない)。
fn stats_view(id: MapDivisionId, owner: CountryId, c: &CombatStats) -> Division {
    Division {
        id: DivisionId(id.0 as usize),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(0),
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower: c.manpower,
        max_manpower: c.max_manpower,
        equipment: c.equipment,
        max_equipment: c.max_equipment,
        organization: c.organization,
        max_organization: c.max_organization,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Fighting,
        def_id: DivisionDefinitionId(0),
        attack_power: c.attack_power,
        defense_power: c.defense_power,
        combat_id: None,
    }
}

/// 日次の戦闘計算(旧 `resolve_combat_day_multi_with_supply`)。参加者の準備率が不正(旧の検証が `Err`)なら `None`(旧と同じく、その戦闘の当日の解決を見送る)。
/// 引数の順序(`MapDivisionId` 昇順)が旧の「参加師団の ID 昇順」に対応する。
pub fn resolve_day(attackers: &[(MapDivisionId, CountryId, &CombatStats)], defenders: &[(MapDivisionId, CountryId, &CombatStats)]) -> Option<DayDamage> {
    let atk: Vec<Division> = attackers.iter().map(|(id, o, c)| stats_view(*id, *o, c)).collect();
    let def: Vec<Division> = defenders.iter().map(|(id, o, c)| stats_view(*id, *o, c)).collect();
    let invalid = atk
        .iter()
        .chain(def.iter())
        .any(|d| calculate_combat_readiness_with_supply(d.manpower, d.max_manpower, d.equipment, d.max_equipment, 1.0).is_err());
    if invalid {
        return None;
    }
    let atk_refs: Vec<&Division> = atk.iter().collect();
    let def_refs: Vec<&Division> = def.iter().collect();
    let (a, d) = resolve_combat_day_multi_with_supply(&atk_refs, &def_refs, TERRAIN_BONUS_PLAIN, &MilitaryRegistry::default());
    let back = |m: std::collections::HashMap<DivisionId, (u64, f32)>| -> BTreeMap<MapDivisionId, (u64, f32)> { m.into_iter().map(|(k, v)| (MapDivisionId(k.0 as u32), v)).collect() };
    let attackers = back(a);
    let defenders = back(d);
    let zero_damage = !attackers.values().any(|(m, _)| *m > 0) && !defenders.values().any(|(m, _)| *m > 0);
    Some(DayDamage { attackers, defenders, zero_damage })
}

/// 1体へ戦闘損耗を適用する(旧 `apply_combat_casualty` と同じ手順。装備の損失は人員損失に比例)。計算が失敗したときは何も変えない(旧と同じ)。
pub fn apply_casualty(c: &mut CombatStats, manpower_loss: u64, org_loss: f32) {
    let before = c.manpower;
    let after = before.saturating_sub(manpower_loss);
    let actual = before - after;
    let Ok(loss) = calculate_proportional_equipment_loss(c.equipment, before, actual) else {
        return;
    };
    if try_destroy_equipment(&mut c.equipment, loss.destroyed).is_err() {
        return;
    }
    c.manpower = after;
    c.organization = (c.organization - org_loss).max(0.0);
}

/// 脱落した参加師団の結末(旧 `resolve_participant_outcome` / `force_retreat_or_destroy`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticipantOutcome {
    /// 戦闘に留まる(組織率が残っている、または一時的な計算エラーで見送った)。
    Stays,
    /// 退路へ撤退する(追加の人員損失と比例する装備損失を適用済み)。
    Retreats,
    /// 撃破される(人員が 0、退路なし、または撤退の追加損失で人員が尽きる)。装備は全損させてある。
    Destroyed,
}

/// 組織率・人員から結末を判定する。`has_destination` は退路があるか。
pub fn participant_outcome(c: &mut CombatStats, has_destination: bool) -> ParticipantOutcome {
    if c.manpower == 0 {
        let remaining = c.equipment;
        if try_destroy_equipment(&mut c.equipment, remaining).is_err() {
            return ParticipantOutcome::Stays;
        }
        return ParticipantOutcome::Destroyed;
    }
    if c.organization > 0.0 {
        return ParticipantOutcome::Stays;
    }
    force_retreat_or_destroy(c, has_destination)
}

/// 組織率 0 や両陣営ゼロ損害による強制撤退(旧 `force_retreat_or_destroy`)。
pub fn force_retreat_or_destroy(c: &mut CombatStats, has_destination: bool) -> ParticipantOutcome {
    let before = c.manpower;
    let retreat_loss = (c.max_manpower as f32 * RETREAT_MANPOWER_LOSS_RATIO) as u64;
    let remaining = before.saturating_sub(retreat_loss);
    let actual = before - remaining;
    if has_destination && remaining > 0 {
        let Ok(plan) = calculate_proportional_equipment_loss(c.equipment, before, actual) else {
            return ParticipantOutcome::Stays;
        };
        if try_destroy_equipment(&mut c.equipment, plan.destroyed).is_err() {
            return ParticipantOutcome::Stays;
        }
        c.manpower = remaining;
        return ParticipantOutcome::Retreats;
    }
    let all = c.equipment;
    if try_destroy_equipment(&mut c.equipment, all).is_err() {
        return ParticipantOutcome::Stays;
    }
    ParticipantOutcome::Destroyed
}

/// 非戦闘・非移動の師団が自国支配の地域にいるときの組織率の回復(旧 `process_org_recovery` の1体分)。人員 0 は回復しない。
pub fn recover_organization(c: &mut CombatStats) {
    if c.manpower == 0 {
        return;
    }
    c.organization = (c.organization + ORG_RECOVERY_PER_DAY).min(c.max_organization);
}
