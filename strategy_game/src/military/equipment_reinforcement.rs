//! P25-005: 国家備蓄(`CountryStockpile[MilitaryEquipment]`)から既存Divisionへの
//! 日次自動装備補充。
//!
//! 人的資源補充(`military::reinforcement`)と同じ所有・存続・戦闘状態・支配地域の
//! 適格条件(`is_eligible_for_daily_reinforcement_by_position`)を再利用しつつ、
//! 装備の過不足判定はmanpowerから完全に独立させる(§3・§7)。按分にも新しい式を
//! 発明せず、`military::reinforcement::apportion_reinforcements`(最大剰余法、
//! `u128`中間計算、剰余降順→`DivisionId`昇順のタイブレーク)をunits空間でそのまま
//! 再利用する。
//!
//! 会計の真実源は引き続きP25-002-REV-001の正規固定小数点units・`try_issue_equipment`
//! のみであり、新しいscale定数やvalidationを複製しない。国家ごとに
//! 「snapshot→全対象状態をvalidation→units変換→按分計画→全支給の事前検証
//! (cloneした一時値へ`try_issue_equipment`を適用)→成功時のみcommit」という
//! 二段階(検証→適用)構成を取り、1つのDivisionまたは国家備蓄が不正状態の場合は
//! その国全体を変更せず中断する(§10.1)。

use crate::common::{CountryId, DivisionId};
use crate::country::CountryRegistry;
use crate::economy::resources::{
    ResourceType, military_equipment_units_to_value, military_equipment_value_to_units,
};
use crate::military::battle::BattleRegistry;
use crate::military::data::{Division, MilitaryRegistry};
use crate::military::equipment::try_issue_equipment;
use crate::military::reinforcement::{
    apportion_reinforcements, is_eligible_for_daily_reinforcement_by_position,
};
use crate::state::data::StateRegistry;
use bevy::log::debug;
use std::collections::{HashMap, HashSet};

/// 対象Divisionがequipment補充可能かどうかを判定する(副作用なし)。manpower
/// 補充と同じ所有・存続・戦闘状態・支配地域条件を使うが、manpowerの過不足は
/// 一切見ない(§7: 「manpowerが満員でも装備補充可能」「その日のmanpower補充量が
/// 0でも装備補充可能」)。`max_equipment<=0.0`のDivisionは需要が構造的に存在
/// しないため対象外にする(§7)。
fn is_eligible_for_equipment_reinforcement(
    division: &Division,
    state_registry: &StateRegistry,
    battle_registry: &BattleRegistry,
) -> bool {
    if division.max_equipment <= 0.0 {
        return false;
    }
    is_eligible_for_daily_reinforcement_by_position(division, state_registry, battle_registry)
}

/// 全ての活動国家へ日次装備補充を適用する(`military::update::handle_daily_military`の
/// manpower補充の直後、移動・戦闘処理より前に呼び出す想定、§5)。
///
/// `blocked_by_completed_recruitment`は、同日の`process_recruitment`が完了済み
/// キューを純粋な装備不足で生成できなかった国家の集合(§6)。この集合に含まれる
/// 国家は、当日の既存Division向け装備補充予算を0にする(備蓄を翌日以降へ持ち越し、
/// 待機中のキューが必要量へ到達できるようにするため)。
pub fn apply_daily_equipment_reinforcement(
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
    state_registry: &StateRegistry,
    battle_registry: &BattleRegistry,
    blocked_by_completed_recruitment: &HashSet<CountryId>,
) {
    let mut by_owner: HashMap<CountryId, Vec<DivisionId>> = HashMap::new();
    for division in military_registry.divisions.values() {
        if is_eligible_for_equipment_reinforcement(division, state_registry, battle_registry) {
            by_owner
                .entry(division.owner)
                .or_default()
                .push(division.id);
        }
    }
    if by_owner.is_empty() {
        return;
    }

    let mut owners: Vec<CountryId> = by_owner.keys().copied().collect();
    owners.sort_by_key(|c| c.0);

    for owner in owners {
        if blocked_by_completed_recruitment.contains(&owner) {
            debug!(
                "[EquipmentReinforcement] country {owner:?}: skipped today, a completed \
                 recruitment item is waiting on equipment stockpile"
            );
            continue;
        }
        let mut division_ids = by_owner.remove(&owner).unwrap_or_default();
        division_ids.sort_by_key(|id| id.0);
        apply_equipment_reinforcement_for_country(country_registry, military_registry, owner, &division_ids);
    }
}

fn apply_equipment_reinforcement_for_country(
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
    owner: CountryId,
    division_ids: &[DivisionId],
) {
    let Some(country) = country_registry.get(owner) else {
        return;
    };
    if !country.is_active() {
        return;
    }

    let stockpile_value_before = country.stockpile.get(ResourceType::MilitaryEquipment);
    let Ok(stockpile_units_before) = military_equipment_value_to_units(stockpile_value_before)
    else {
        debug!(
            "[EquipmentReinforcement] country {owner:?}: invalid MilitaryEquipment stockpile, \
             skipping today's equipment reinforcement"
        );
        return;
    };
    if stockpile_units_before == 0 {
        return;
    }

    // フェーズ1(検証開始): 全対象Divisionの状態をvalidationし、不足量をunits化する。
    // いずれか1件でも不正(equipment > max_equipment、または量子化不能)なら、
    // この国の装備補充全体を変更せず中断する(§10.1)。
    let mut requests: Vec<(DivisionId, u64)> = Vec::with_capacity(division_ids.len());
    for &division_id in division_ids {
        let Some(division) = military_registry.divisions.get(&division_id) else {
            continue;
        };
        let Ok(equipment_units) = military_equipment_value_to_units(division.equipment) else {
            debug!(
                "[EquipmentReinforcement] country {owner:?}: division {division_id:?} has an \
                 invalid equipment value, aborting this country's equipment reinforcement today"
            );
            return;
        };
        let Ok(max_units) = military_equipment_value_to_units(division.max_equipment) else {
            debug!(
                "[EquipmentReinforcement] country {owner:?}: division {division_id:?} has an \
                 invalid max_equipment value, aborting this country's equipment reinforcement today"
            );
            return;
        };
        if equipment_units > max_units {
            debug!(
                "[EquipmentReinforcement] country {owner:?}: division {division_id:?} has \
                 equipment exceeding max_equipment, aborting this country's equipment \
                 reinforcement today"
            );
            return;
        }
        let shortfall = max_units - equipment_units;
        if shortfall > 0 {
            requests.push((division_id, shortfall));
        }
    }
    if requests.is_empty() {
        return;
    }

    // フェーズ1(按分計画): 既存の最大剰余按分をunits空間でそのまま再利用する。
    let allocations = apportion_reinforcements(stockpile_units_before, &requests);

    // フェーズ1(事前検証): cloneした一時値へ`try_issue_equipment`を適用し、
    // 会計基盤自体(scale・validation・保存則)を唯一の真実源として使う。
    let mut temp_stockpile = country.stockpile.clone();
    let mut validated: Vec<(DivisionId, f64)> = Vec::with_capacity(requests.len());
    for &(division_id, _shortfall) in &requests {
        let allocation_units = allocations.get(&division_id).copied().unwrap_or(0);
        if allocation_units == 0 {
            continue;
        }
        let Some(division) = military_registry.divisions.get(&division_id) else {
            return;
        };
        if division.owner != owner {
            return;
        }
        let requested_value = military_equipment_units_to_value(allocation_units);
        let mut temp_equipment = division.equipment;
        match try_issue_equipment(
            &mut temp_stockpile,
            &mut temp_equipment,
            division.max_equipment,
            requested_value,
        ) {
            Ok(_) => validated.push((division_id, temp_equipment)),
            Err(err) => {
                debug!(
                    "[EquipmentReinforcement] country {owner:?}: division {division_id:?} \
                     allocation failed unexpectedly ({err:?}), aborting this country's \
                     equipment reinforcement today"
                );
                return;
            }
        }
    }
    if validated.is_empty() {
        return;
    }

    // フェーズ2(commit): 全件成功した場合のみ、実状態へ反映する。
    if let Some(c) = country_registry.get_mut(owner) {
        c.stockpile = temp_stockpile;
    }
    for (division_id, new_equipment) in validated {
        if let Some(d) = military_registry.divisions.get_mut(&division_id) {
            d.equipment = new_equipment;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{BattleId, DivisionDefinitionId, StateId, WarId};
    use crate::country::{CountryData, CountryStatus};
    use crate::military::battle::{Battle, BattleStatus};
    use crate::military::data::{DivisionSize, DivisionStatus, DivisionType};
    use crate::state::data::StateData;

    fn test_division(
        id: usize,
        owner: usize,
        state: StateId,
        equipment: f64,
        max_equipment: f64,
        status: DivisionStatus,
    ) -> Division {
        Division {
            id: DivisionId(id),
            owner: CountryId(owner),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: state,
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 5_000,
            max_manpower: 10_000,
            equipment,
            max_equipment,
            organization: 50.0,
            max_organization: 50.0,
            morale: 1.0,
            max_morale: 1.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status,
            def_id: DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn owned_state(id: StateId, owner: usize) -> StateData {
        StateData {
            id,
            owner_country_id: CountryId(owner),
            ..StateData::default()
        }
    }

    fn country_with_stockpile(id: usize, equipment_stockpile: f64) -> CountryData {
        let mut c = CountryData {
            id: CountryId(id),
            ..CountryData::default()
        };
        c.stockpile
            .set(ResourceType::MilitaryEquipment, equipment_stockpile);
        c
    }

    struct Harness {
        countries: CountryRegistry,
        military: MilitaryRegistry,
        states: StateRegistry,
        battles: BattleRegistry,
    }

    fn harness(countries: Vec<CountryData>, divisions: Vec<Division>, states: Vec<StateData>) -> Harness {
        let mut military = MilitaryRegistry::default();
        for d in divisions {
            military.divisions.insert(d.id, d);
        }
        Harness {
            countries: CountryRegistry { countries },
            military,
            states: StateRegistry::build(states),
            battles: BattleRegistry::default(),
        }
    }

    fn run(h: &mut Harness) {
        run_with_blocked(h, &HashSet::new());
    }

    fn run_with_blocked(h: &mut Harness, blocked: &HashSet<CountryId>) {
        apply_daily_equipment_reinforcement(
            &mut h.countries,
            &mut h.military,
            &h.states,
            &h.battles,
            blocked,
        );
    }

    // ─── §12.1 完全補充 ───────────────────────────────────────────────────

    #[test]
    fn full_shortfall_is_covered_when_stockpile_is_sufficient() {
        let mut h = harness(
            vec![country_with_stockpile(1, 60.0)],
            vec![test_division(
                0,
                1,
                StateId(0),
                40.0,
                100.0,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 100.0);
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .stockpile
                .get(ResourceType::MilitaryEquipment),
            0.0
        );
    }

    // ─── §12.2 比例按分 ───────────────────────────────────────────────────

    #[test]
    fn insufficient_stockpile_apportions_proportionally() {
        let mut h = harness(
            vec![country_with_stockpile(1, 50.0)],
            vec![
                test_division(0, 1, StateId(0), 40.0, 100.0, DivisionStatus::Idle), // shortfall 60
                test_division(1, 1, StateId(0), 60.0, 100.0, DivisionStatus::Idle), // shortfall 40
            ],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 70.0);
        assert_eq!(h.military.divisions[&DivisionId(1)].equipment, 80.0);
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .stockpile
                .get(ResourceType::MilitaryEquipment),
            0.0
        );
    }

    // ─── §12.3 小数按分 ───────────────────────────────────────────────────

    #[test]
    fn fractional_shortfall_apportions_exactly_in_units() {
        let mut h = harness(
            vec![country_with_stockpile(1, 2.5)],
            vec![
                test_division(0, 1, StateId(0), 96.3, 100.0, DivisionStatus::Idle), // shortfall 3.7
                test_division(1, 1, StateId(0), 98.7, 100.0, DivisionStatus::Idle), // shortfall 1.3
            ],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        // 生のf64加算(96.3 + 1.85等)は二項ともそれぞれ既に丸め誤差を持つため、
        // 単純な足し算では正規のunits変換結果とビット単位で一致しない場合がある
        // (P25-002-REV-001が避けようとした精度問題そのもの)。ここでは、正規の
        // 小数リテラル(units空間での正確な計算結果)に対して直接比較する。
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 98.15);
        assert_eq!(h.military.divisions[&DivisionId(1)].equipment, 99.35);
    }

    // ─── §12.4 tie-break ─────────────────────────────────────────────────

    #[test]
    fn equal_tiny_remainders_break_by_ascending_division_id() {
        let mut h = harness(
            vec![country_with_stockpile(1, 0.000_001)],
            vec![
                test_division(1, 1, StateId(0), 99.999_999, 100.0, DivisionStatus::Idle),
                test_division(0, 1, StateId(0), 99.999_999, 100.0, DivisionStatus::Idle),
            ],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 100.0);
        assert_eq!(
            h.military.divisions[&DivisionId(1)].equipment,
            99.999_999
        );
    }

    // ─── Test 5: ECS挿入順非依存 ─────────────────────────────────────────

    #[test]
    fn result_does_not_depend_on_insertion_order() {
        let build = |order: [usize; 2]| {
            let mut h = harness(
                vec![country_with_stockpile(1, 50.0)],
                vec![],
                vec![owned_state(StateId(0), 1)],
            );
            let shortfalls = [60.0, 40.0];
            for id in order {
                h.military.divisions.insert(
                    DivisionId(id),
                    test_division(
                        id,
                        1,
                        StateId(0),
                        100.0 - shortfalls[id],
                        100.0,
                        DivisionStatus::Idle,
                    ),
                );
            }
            run(&mut h);
            let mut result: Vec<(usize, String)> = h
                .military
                .divisions
                .iter()
                .map(|(id, d)| (id.0, format!("{:.6}", d.equipment)))
                .collect();
            result.sort_by_key(|&(id, _)| id);
            result
        };
        assert_eq!(build([0, 1]), build([1, 0]));
    }

    // ─── Test 7〜9: 境界 ─────────────────────────────────────────────────

    #[test]
    fn zero_stockpile_does_not_change_anything() {
        let mut h = harness(
            vec![country_with_stockpile(1, 0.0)],
            vec![test_division(
                0,
                1,
                StateId(0),
                40.0,
                100.0,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 40.0);
    }

    #[test]
    fn zero_shortfall_receives_nothing() {
        let mut h = harness(
            vec![country_with_stockpile(1, 500.0)],
            vec![test_division(
                0,
                1,
                StateId(0),
                100.0,
                100.0,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 100.0);
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .stockpile
                .get(ResourceType::MilitaryEquipment),
            500.0
        );
    }

    #[test]
    fn max_equipment_zero_receives_nothing() {
        let mut h = harness(
            vec![country_with_stockpile(1, 500.0)],
            vec![test_division(
                0,
                1,
                StateId(0),
                0.0,
                0.0,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 0.0);
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .stockpile
                .get(ResourceType::MilitaryEquipment),
            500.0
        );
    }

    // ─── Test 10〜11: manpowerからの独立 ──────────────────────────────────

    #[test]
    fn full_manpower_division_still_receives_equipment() {
        let mut h = harness(
            vec![country_with_stockpile(1, 60.0)],
            vec![{
                let mut d = test_division(0, 1, StateId(0), 40.0, 100.0, DivisionStatus::Idle);
                d.manpower = d.max_manpower; // manpower満員
                d
            }],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 100.0);
    }

    // ─── Test 12: equipment備蓄0でもmanpower補充は無関係 ─────────────────

    #[test]
    fn zero_equipment_stockpile_does_not_prevent_manpower_reinforcement_call() {
        // manpower補充自体はapply_daily_reinforcementの責務であり、このモジュールは
        // 一切呼ばない。equipment側の関数を実行しても他資源へは触れないことだけを
        // ここで確認する(manpowerフィールドが不変であること)。
        let mut h = harness(
            vec![country_with_stockpile(1, 0.0)],
            vec![test_division(
                0,
                1,
                StateId(0),
                40.0,
                100.0,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1)],
        );
        let manpower_before = h.military.divisions[&DivisionId(0)].manpower;
        run(&mut h);
        assert_eq!(
            h.military.divisions[&DivisionId(0)].manpower,
            manpower_before
        );
    }

    // ─── Test 15: units保存則 ─────────────────────────────────────────────

    #[test]
    fn stockpile_and_division_equipment_conserve_the_world_total() {
        let mut h = harness(
            vec![country_with_stockpile(1, 37.0)],
            vec![
                test_division(0, 1, StateId(0), 40.0, 100.0, DivisionStatus::Idle),
                test_division(1, 1, StateId(0), 10.0, 100.0, DivisionStatus::Idle),
            ],
            vec![owned_state(StateId(0), 1)],
        );
        let total_before: f64 = 37.0 + 40.0 + 10.0;
        run(&mut h);
        let total_after: f64 = h
            .countries
            .get(CountryId(1))
            .unwrap()
            .stockpile
            .get(ResourceType::MilitaryEquipment)
            + h.military
                .divisions
                .values()
                .map(|d| d.equipment)
                .sum::<f64>();
        assert_eq!(total_before, total_after);
    }

    // ─── Test 16〜17: max_equipment不変・上限超過なし ─────────────────────

    #[test]
    fn max_equipment_is_unchanged_and_equipment_never_exceeds_it() {
        let mut h = harness(
            vec![country_with_stockpile(1, 1_000.0)],
            vec![test_division(
                0,
                1,
                StateId(0),
                40.0,
                100.0,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        let d = &h.military.divisions[&DivisionId(0)];
        assert_eq!(d.max_equipment, 100.0);
        assert!(d.equipment <= d.max_equipment);
    }

    // ─── Test 19: 複数国家分離 ─────────────────────────────────────────────

    #[test]
    fn multiple_countries_are_fully_isolated() {
        let mut h = harness(
            vec![
                country_with_stockpile(1, 0.0),
                country_with_stockpile(2, 100.0),
            ],
            vec![
                test_division(0, 1, StateId(0), 40.0, 100.0, DivisionStatus::Idle),
                test_division(1, 2, StateId(1), 40.0, 100.0, DivisionStatus::Idle),
            ],
            vec![owned_state(StateId(0), 1), owned_state(StateId(1), 2)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 40.0);
        assert_eq!(h.military.divisions[&DivisionId(1)].equipment, 100.0);
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .stockpile
                .get(ResourceType::MilitaryEquipment),
            0.0
        );
    }

    // ─── Test 20: 消滅国家の凍結 ───────────────────────────────────────────

    #[test]
    fn eliminated_country_is_frozen() {
        let mut h = harness(
            vec![{
                let mut c = country_with_stockpile(1, 100.0);
                c.status = CountryStatus::Eliminated {
                    eliminated_date: "1900/01/01".to_string(),
                    annexed_by: CountryId(2),
                };
                c
            }],
            vec![test_division(
                0,
                1,
                StateId(0),
                40.0,
                100.0,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 40.0);
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .stockpile
                .get(ResourceType::MilitaryEquipment),
            100.0
        );
    }

    // ─── Test 21〜22: 不正状態の原子的拒否 ─────────────────────────────────

    #[test]
    fn invalid_division_equipment_state_aborts_the_whole_country_atomically() {
        let mut h = harness(
            vec![country_with_stockpile(1, 100.0)],
            vec![
                // equipment(150) > max_equipment(100): 不正状態。
                test_division(0, 1, StateId(0), 150.0, 100.0, DivisionStatus::Idle),
                test_division(1, 1, StateId(0), 40.0, 100.0, DivisionStatus::Idle),
            ],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 150.0);
        assert_eq!(
            h.military.divisions[&DivisionId(1)].equipment,
            40.0,
            "健全な方のDivisionも一切変更されてはいけない(国家単位で中断)"
        );
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .stockpile
                .get(ResourceType::MilitaryEquipment),
            100.0
        );
    }

    #[test]
    fn invalid_country_stockpile_aborts_atomically() {
        let mut h = harness(
            vec![country_with_stockpile(1, f64::NAN)],
            vec![test_division(
                0,
                1,
                StateId(0),
                40.0,
                100.0,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 40.0);
    }

    // ─── Test 23: 1国の失敗が別国へ波及しない ─────────────────────────────

    #[test]
    fn one_countrys_abort_does_not_affect_another_country() {
        let mut h = harness(
            vec![
                country_with_stockpile(1, 100.0),
                country_with_stockpile(2, 60.0),
            ],
            vec![
                test_division(0, 1, StateId(0), 150.0, 100.0, DivisionStatus::Idle), // 不正
                test_division(1, 2, StateId(1), 40.0, 100.0, DivisionStatus::Idle),
            ],
            vec![owned_state(StateId(0), 1), owned_state(StateId(1), 2)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 150.0);
        assert_eq!(h.military.divisions[&DivisionId(1)].equipment, 100.0);
    }

    // ─── §12.5 完了済み募兵キューの保護 ────────────────────────────────────

    #[test]
    fn blocked_country_receives_no_equipment_reinforcement_today() {
        let mut h = harness(
            vec![country_with_stockpile(1, 60.0)],
            vec![test_division(
                0,
                1,
                StateId(0),
                40.0,
                100.0,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1)],
        );
        let mut blocked = HashSet::new();
        blocked.insert(CountryId(1));
        run_with_blocked(&mut h, &blocked);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 40.0);
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .stockpile
                .get(ResourceType::MilitaryEquipment),
            60.0
        );
    }

    #[test]
    fn blocking_one_country_does_not_block_another() {
        let mut h = harness(
            vec![
                country_with_stockpile(1, 60.0),
                country_with_stockpile(2, 60.0),
            ],
            vec![
                test_division(0, 1, StateId(0), 40.0, 100.0, DivisionStatus::Idle),
                test_division(1, 2, StateId(1), 40.0, 100.0, DivisionStatus::Idle),
            ],
            vec![owned_state(StateId(0), 1), owned_state(StateId(1), 2)],
        );
        let mut blocked = HashSet::new();
        blocked.insert(CountryId(1));
        run_with_blocked(&mut h, &blocked);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 40.0);
        assert_eq!(h.military.divisions[&DivisionId(1)].equipment, 100.0);
    }

    // ─── Test: 進行中戦闘・非Idle状態は対象外 ──────────────────────────────

    #[test]
    fn a_division_referenced_by_an_ongoing_battle_is_not_eligible() {
        let mut h = harness(
            vec![country_with_stockpile(1, 60.0)],
            vec![test_division(
                0,
                1,
                StateId(0),
                40.0,
                100.0,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1)],
        );
        h.battles.battles.insert(
            BattleId(0),
            Battle {
                id: BattleId(0),
                war_id: WarId(0),
                state_id: StateId(0),
                attacker_country: CountryId(1),
                defender_country: CountryId(2),
                attacker_division_ids: vec![DivisionId(0)],
                defender_division_ids: vec![],
                attacker_origins: Default::default(),
                start_date: "1800/01/01".to_string(),
                elapsed_days: 0,
                status: BattleStatus::Ongoing,
            },
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 40.0);
    }

    #[test]
    fn a_fighting_division_is_not_eligible() {
        let mut h = harness(
            vec![country_with_stockpile(1, 60.0)],
            vec![test_division(
                0,
                1,
                StateId(0),
                40.0,
                100.0,
                DivisionStatus::Fighting,
            )],
            vec![owned_state(StateId(0), 1)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].equipment, 40.0);
    }
}
