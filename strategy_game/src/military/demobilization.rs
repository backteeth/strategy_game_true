//! P23-007: 個別Divisionの復員(生存兵員の人的資源返還)。
//!
//! P23-006/P23-006-FIX-001が整理した人的資源帳簿(`military::manpower`)の用語で言えば、
//! 本モジュールは`B`(累積動員負担)と`L`(現役軍事人的資源)を、対象Divisionの現在
//! 生存兵員`R`分だけ同時に減らし、その`R`を`available_manpower`へ返還する。`K`(恒久軍事損失)
//! は`B`と`L`が同数減るため自然に不変となる(新しい保存フィールドは追加しない)。
//!
//! Army編成の解散(`military::army::ArmyRegistry::disband`)とは明確に別の操作である。
//! Army解散は編成というグループ分けを解くだけでDivision実体・人的資源には一切触れない
//! (`B`/`L`/`K`/`available_manpower`すべて不変)。本モジュールの復員は逆に、Division実体を
//! `MilitaryRegistry`から削除し、人的資源を実際に動かす。
//!
//! 復員可能なDivisionは`DivisionStatus::Idle`のものに限る。移動・戦闘・撤退・占領・
//! (未使用の)解散中ステータスはいずれも、他の日次処理やレジストリ参照との安全な
//! 整理を保証できないため一律で拒否する(`Occupying`についても、州占領処理
//! [`war::occupation`]との相互作用を個別に検証していないため、推測で許可しない)。
use crate::common::{CountryId, DivisionId};
use crate::country::CountryRegistry;
use crate::military::army::ArmyRegistry;
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::military::manpower::{ManpowerLedgerError, compute_manpower_ledger};
use crate::war::data::WarRegistry;
use crate::war::frontline::FrontlineRegistry;

/// `demobilize_division`が失敗し得る理由。呼び出し元(UI・テスト)が理由ごとに
/// 安全に分岐できるよう、UI上で意味のある区別(戦闘中/移動中/撤退中など)は
/// 個別のバリアントとして表現する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemobilizationError {
    /// 指定した国家が`CountryRegistry`に存在しない。
    CountryNotFound,
    /// 指定した国家が消滅済み。
    CountryEliminated,
    /// 指定したDivisionが`MilitaryRegistry`に存在しない。
    DivisionNotFound,
    /// Divisionの所有国が要求国家と一致しない。
    NotOwner,
    /// Divisionが既に生存判定(`manpower>0 && status!=Destroyed`)を満たさない。
    DivisionAlreadyDead,
    /// Divisionが戦闘中。
    DivisionFighting,
    /// Divisionが移動中。
    DivisionMoving,
    /// Divisionが撤退中。
    DivisionRetreating,
    /// 上記以外の理由でIdleではない(Occupying等、安全性を個別確認していない状態)。
    DivisionNotIdle,
    /// `status`はIdleだが、`BattleRegistry`の進行中戦闘がこのDivisionを参加者として
    /// 参照している(本来あり得ない不整合。statusだけを見て復員を許可しない)。
    DivisionReferencedByOngoingBattle,
    /// 募兵キューが未解決の`DivisionDefinitionId`を参照しており、人的資源帳簿を
    /// 計算できない(`military::manpower::compute_manpower_ledger`参照)。
    LedgerUnavailable(ManpowerLedgerError),
    /// 帳簿は計算できたが`unaccounted_living_excess != 0`(本来あり得ない異常状態)。
    /// この状態で復員を許可すると`B`を`R`分減らした結果が信頼できない。
    LedgerInconsistent,
    /// 累積動員負担`B`が、返還しようとしている生存兵員`R`より小さい
    /// (通常状態では`B=L+K`により必ず`B>=R`のはずの不変条件違反)。
    InsufficientBurden,
    /// `available_manpower + R`が`u64`の範囲を超える。
    AvailableManpowerOverflow,
}

/// 復員成功時の結果。UI・テストが復員による人的資源帳簿の遷移を確認できるよう、
/// 前後の`workforce_burden`(=`mobilized_manpower`)を含める。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemobilizationOutcome {
    pub country_id: CountryId,
    pub division_id: DivisionId,
    /// 返還した生存兵員数`R`(Division復員実行時点の実際の`manpower`。`max_manpower`ではない)。
    pub returned_manpower: u64,
    pub workforce_burden_before: u64,
    pub workforce_burden_after: u64,
}

/// 指定したDivisionを復員する。
///
/// 検証にすべて成功した場合のみ状態を変更する(部分成功を許さない)。いずれかの
/// 検証に失敗した場合、`available_manpower`・`mobilized_manpower`・`MilitaryRegistry`・
/// `ArmyRegistry`・`FrontlineRegistry`のいずれも変更しない。
///
/// 装備・資金・経験値・Organizationは一切返還しない(P23-007の対象外)。
#[allow(clippy::too_many_arguments)]
pub fn demobilize_division(
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
    army_registry: &mut ArmyRegistry,
    frontline_registry: &mut FrontlineRegistry,
    war_registry: &WarRegistry,
    battle_registry: &BattleRegistry,
    country_id: CountryId,
    division_id: DivisionId,
) -> Result<DemobilizationOutcome, DemobilizationError> {
    let country = country_registry
        .get(country_id)
        .ok_or(DemobilizationError::CountryNotFound)?;
    if !country.is_active() {
        return Err(DemobilizationError::CountryEliminated);
    }

    let division = military_registry
        .divisions
        .get(&division_id)
        .ok_or(DemobilizationError::DivisionNotFound)?;
    if division.owner != country_id {
        return Err(DemobilizationError::NotOwner);
    }
    if division.manpower == 0 || division.status == DivisionStatus::Destroyed {
        return Err(DemobilizationError::DivisionAlreadyDead);
    }
    match division.status {
        DivisionStatus::Idle => {}
        DivisionStatus::Fighting => return Err(DemobilizationError::DivisionFighting),
        DivisionStatus::Moving => return Err(DemobilizationError::DivisionMoving),
        DivisionStatus::Retreating => return Err(DemobilizationError::DivisionRetreating),
        DivisionStatus::Occupying | DivisionStatus::Disbanding | DivisionStatus::Destroyed => {
            return Err(DemobilizationError::DivisionNotIdle);
        }
    }

    let referenced_by_battle = battle_registry.battles.values().any(|b| {
        b.status == BattleStatus::Ongoing
            && (b.attacker_division_ids.contains(&division_id)
                || b.defender_division_ids.contains(&division_id))
    });
    if referenced_by_battle || division.combat_id.is_some() {
        return Err(DemobilizationError::DivisionReferencedByOngoingBattle);
    }

    let returned_manpower = division.manpower;

    let ledger = compute_manpower_ledger(country, military_registry)
        .map_err(DemobilizationError::LedgerUnavailable)?;
    if ledger.unaccounted_living_excess != 0 {
        return Err(DemobilizationError::LedgerInconsistent);
    }
    if ledger.workforce_burden < returned_manpower {
        return Err(DemobilizationError::InsufficientBurden);
    }

    let available_after = country
        .available_manpower
        .checked_add(returned_manpower)
        .ok_or(DemobilizationError::AvailableManpowerOverflow)?;
    let workforce_burden_before = ledger.workforce_burden;
    let workforce_burden_after = workforce_burden_before
        .checked_sub(returned_manpower)
        .ok_or(DemobilizationError::InsufficientBurden)?;

    // ここまでの検証が全て成功したので、以降は失敗し得ない操作のみ実行する。
    let country = country_registry
        .get_mut(country_id)
        .expect("country existence already verified above");
    country.available_manpower = available_after;
    country.mobilized_manpower = workforce_burden_after;

    military_registry.remove_division(division_id);
    army_registry.sanitize_references(military_registry);
    frontline_registry.sanitize_references(military_registry, war_registry);

    Ok(DemobilizationOutcome {
        country_id,
        division_id,
        returned_manpower,
        workforce_burden_before,
        workforce_burden_after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{DivisionDefinitionId, StateId};
    use crate::country::{CountryData, CountryStatus};
    use crate::military::battle::Battle;
    use crate::military::data::Division;
    use crate::military::data::{DivisionSize, DivisionType};

    fn test_country(id: usize, available: u64, mobilized: u64) -> CountryData {
        CountryData {
            id: CountryId(id),
            available_manpower: available,
            mobilized_manpower: mobilized,
            ..CountryData::default()
        }
    }

    fn test_division(id: usize, owner: usize, manpower: u64, status: DivisionStatus) -> Division {
        Division {
            id: DivisionId(id),
            owner: CountryId(owner),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(0),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower,
            max_manpower: manpower.max(1) + 5_000, // max != current, R must use current only
            equipment: 10.0,
            max_equipment: 10.0,
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

    struct Harness {
        countries: CountryRegistry,
        military: MilitaryRegistry,
        armies: ArmyRegistry,
        frontlines: FrontlineRegistry,
        wars: WarRegistry,
        battles: BattleRegistry,
    }

    fn harness(countries: Vec<CountryData>, divisions: Vec<Division>) -> Harness {
        let mut military = MilitaryRegistry::default();
        for d in divisions {
            military.divisions.insert(d.id, d);
        }
        Harness {
            countries: CountryRegistry { countries },
            military,
            armies: ArmyRegistry::default(),
            frontlines: FrontlineRegistry::default(),
            wars: WarRegistry::default(),
            battles: BattleRegistry::default(),
        }
    }

    fn run(
        h: &mut Harness,
        country_id: CountryId,
        division_id: DivisionId,
    ) -> Result<DemobilizationOutcome, DemobilizationError> {
        demobilize_division(
            &mut h.countries,
            &mut h.military,
            &mut h.armies,
            &mut h.frontlines,
            &h.wars,
            &h.battles,
            country_id,
            division_id,
        )
    }

    #[test]
    fn demobilizing_an_unworn_division_returns_its_full_manpower() {
        let mut h = harness(
            vec![test_country(1, 100_000, 10_000)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)],
        );
        let outcome = run(&mut h, CountryId(1), DivisionId(0)).unwrap();
        assert_eq!(outcome.returned_manpower, 10_000);
        assert_eq!(outcome.workforce_burden_before, 10_000);
        assert_eq!(outcome.workforce_burden_after, 0);
        let country = h.countries.get(CountryId(1)).unwrap();
        assert_eq!(country.available_manpower, 110_000);
        assert_eq!(country.mobilized_manpower, 0);
        assert!(!h.military.divisions.contains_key(&DivisionId(0)));
    }

    #[test]
    fn demobilizing_a_worn_division_returns_only_its_surviving_manpower() {
        // 初期募兵10,000、戦闘損失3,000 → 生存7,000だけを復員する例(§2.1の数値例)。
        let mut h = harness(
            vec![test_country(1, 0, 10_000)],
            vec![test_division(0, 1, 7_000, DivisionStatus::Idle)],
        );
        let outcome = run(&mut h, CountryId(1), DivisionId(0)).unwrap();
        assert_eq!(outcome.returned_manpower, 7_000);
        let country = h.countries.get(CountryId(1)).unwrap();
        assert_eq!(country.available_manpower, 7_000);
        assert_eq!(
            country.mobilized_manpower, 3_000,
            "K(=3,000)相当分はBに残る"
        );
    }

    #[test]
    fn max_manpower_is_never_used_as_the_returned_amount() {
        let mut h = harness(
            vec![test_country(1, 0, 6_000)],
            vec![test_division(0, 1, 6_000, DivisionStatus::Idle)], // max_manpower = 11_000
        );
        let outcome = run(&mut h, CountryId(1), DivisionId(0)).unwrap();
        assert_eq!(outcome.returned_manpower, 6_000);
    }

    #[test]
    fn other_divisions_and_other_countries_are_unaffected() {
        let mut h = harness(
            vec![test_country(1, 0, 10_000), test_country(2, 0, 5_000)],
            vec![
                test_division(0, 1, 10_000, DivisionStatus::Idle),
                test_division(1, 2, 5_000, DivisionStatus::Idle),
            ],
        );
        run(&mut h, CountryId(1), DivisionId(0)).unwrap();
        let other_country = h.countries.get(CountryId(2)).unwrap();
        assert_eq!(other_country.available_manpower, 0);
        assert_eq!(other_country.mobilized_manpower, 5_000);
        assert!(h.military.divisions.contains_key(&DivisionId(1)));
    }

    #[test]
    fn demobilizing_a_member_removes_it_from_its_army_and_keeps_other_members() {
        let mut h = harness(
            vec![test_country(1, 0, 20_000)],
            vec![
                test_division(0, 1, 10_000, DivisionStatus::Idle),
                test_division(1, 1, 10_000, DivisionStatus::Idle),
            ],
        );
        let group_id = h
            .armies
            .create_army(CountryId(1), &[DivisionId(0), DivisionId(1)], &h.military)
            .unwrap();

        run(&mut h, CountryId(1), DivisionId(0)).unwrap();

        assert_eq!(h.armies.army_for_division(DivisionId(0)), None);
        assert_eq!(
            h.armies.armies[&group_id].member_division_ids,
            vec![DivisionId(1)]
        );
    }

    #[test]
    fn demobilizing_the_last_member_auto_disbands_the_empty_army() {
        let mut h = harness(
            vec![test_country(1, 0, 10_000)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)],
        );
        let group_id = h
            .armies
            .create_army(CountryId(1), &[DivisionId(0)], &h.military)
            .unwrap();

        run(&mut h, CountryId(1), DivisionId(0)).unwrap();

        assert!(!h.armies.armies.contains_key(&group_id));
        assert_eq!(h.armies.army_for_division(DivisionId(0)), None);
    }

    #[test]
    fn demobilizing_the_same_division_twice_fails_safely_the_second_time() {
        let mut h = harness(
            vec![test_country(1, 0, 10_000)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)],
        );
        run(&mut h, CountryId(1), DivisionId(0)).unwrap();
        let second = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(second, Err(DemobilizationError::DivisionNotFound));
    }

    #[test]
    fn demobilization_never_touches_treasury_or_equipment_related_fields() {
        let mut h = harness(
            vec![CountryData {
                treasury: 555.0,
                ..test_country(1, 0, 10_000)
            }],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)],
        );
        run(&mut h, CountryId(1), DivisionId(0)).unwrap();
        assert_eq!(h.countries.get(CountryId(1)).unwrap().treasury, 555.0);
    }

    // --- 失敗時の原子性 -----------------------------------------------------

    fn assert_nothing_changed(h: &Harness, before: &Harness) {
        assert_eq!(
            h.countries.get(CountryId(1)).map(|c| c.available_manpower),
            before
                .countries
                .get(CountryId(1))
                .map(|c| c.available_manpower)
        );
        assert_eq!(
            h.countries.get(CountryId(1)).map(|c| c.mobilized_manpower),
            before
                .countries
                .get(CountryId(1))
                .map(|c| c.mobilized_manpower)
        );
        assert_eq!(h.military.divisions.len(), before.military.divisions.len());
        assert_eq!(h.armies.armies.len(), before.armies.armies.len());
    }

    #[test]
    fn rejects_a_nonexistent_country() {
        let mut h = harness(
            vec![],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)],
        );
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::CountryNotFound));
    }

    #[test]
    fn rejects_an_eliminated_country() {
        let mut h = harness(
            vec![CountryData {
                status: CountryStatus::Eliminated {
                    eliminated_date: "1936/01/01".to_string(),
                    annexed_by: CountryId(9),
                },
                ..test_country(1, 0, 10_000)
            }],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)],
        );
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::CountryEliminated));
    }

    #[test]
    fn rejects_a_nonexistent_division() {
        let mut h = harness(vec![test_country(1, 0, 0)], vec![]);
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::DivisionNotFound));
    }

    #[test]
    fn rejects_a_foreign_owned_division() {
        let mut h = harness(
            vec![test_country(1, 0, 0), test_country(2, 0, 10_000)],
            vec![test_division(0, 2, 10_000, DivisionStatus::Idle)],
        );
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::NotOwner));
    }

    #[test]
    fn rejects_a_zero_manpower_division() {
        let mut h = harness(
            vec![test_country(1, 0, 0)],
            vec![test_division(0, 1, 0, DivisionStatus::Idle)],
        );
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::DivisionAlreadyDead));
    }

    #[test]
    fn rejects_a_destroyed_division() {
        let mut h = harness(
            vec![test_country(1, 0, 0)],
            vec![test_division(0, 1, 999, DivisionStatus::Destroyed)],
        );
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::DivisionAlreadyDead));
    }

    #[test]
    fn rejects_a_fighting_division() {
        let mut h = harness(
            vec![test_country(1, 0, 10_000)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Fighting)],
        );
        let before = harness(
            vec![test_country(1, 0, 10_000)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Fighting)],
        );
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::DivisionFighting));
        assert_nothing_changed(&h, &before);
    }

    #[test]
    fn rejects_a_retreating_division() {
        let mut h = harness(
            vec![test_country(1, 0, 10_000)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Retreating)],
        );
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::DivisionRetreating));
    }

    #[test]
    fn rejects_a_moving_division() {
        let mut h = harness(
            vec![test_country(1, 0, 10_000)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Moving)],
        );
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::DivisionMoving));
    }

    #[test]
    fn rejects_a_division_referenced_by_an_ongoing_battle_even_if_status_looks_idle() {
        // status/BattleRegistryの不整合を想定した防御的テスト:
        // 通常は起こり得ないが、statusだけを見て許可してはいけない(§6監査要求)。
        let mut h = harness(
            vec![test_country(1, 0, 10_000)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)],
        );
        h.battles.battles.insert(
            crate::common::BattleId(0),
            Battle {
                id: crate::common::BattleId(0),
                war_id: crate::common::WarId(0),
                state_id: StateId(0),
                attacker_country: CountryId(1),
                defender_country: CountryId(2),
                attacker_division_ids: vec![DivisionId(0)],
                defender_division_ids: vec![],
                attacker_origins: Default::default(),
                start_date: "1936/01/01".to_string(),
                elapsed_days: 0,
                status: BattleStatus::Ongoing,
            },
        );
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(
            result,
            Err(DemobilizationError::DivisionReferencedByOngoingBattle)
        );
    }

    #[test]
    fn rejects_when_the_recruitment_queue_has_an_unknown_definition() {
        let mut h = harness(
            vec![test_country(1, 0, 0)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)],
        );
        h.countries
            .get_mut(CountryId(1))
            .unwrap()
            .recruitment_queue
            .push(crate::military::recruitment::RecruitmentQueueItem {
                division_id: DivisionDefinitionId(999),
                target_state: StateId(0),
                days_remaining: 5,
                total_days: 30,
            });
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(
            result,
            Err(DemobilizationError::LedgerUnavailable(
                ManpowerLedgerError {
                    unknown_definitions: vec![DivisionDefinitionId(999)]
                }
            ))
        );
    }

    #[test]
    fn rejects_when_the_ledger_is_inconsistent_with_living_exceeding_burden() {
        // mobilized_manpower(B)=0なのにDivisionが生存兵員を持つ異常状態
        // (New Game初期配置漏れ等、P23-006-FIX-001以前の状態を模す)。
        let mut h = harness(
            vec![test_country(1, 0, 0)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)],
        );
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::LedgerInconsistent));
    }

    #[test]
    fn burden_smaller_than_division_manpower_is_unreachable_once_the_ledger_is_consistent() {
        // §3.2はInsufficientBurden(B<R)をLedgerInconsistent(L>B、excess!=0)とは
        // 別の失敗理由として要求するが、数学的には両立しない: 対象Divisionの
        // manpower Rは常にL(その国の生存軍事人的資源合計)の一部なので、
        // R<=L。unaccounted_living_excess==0(LedgerInconsistentを通過)は
        // L<=Bを意味するため、R<=L<=B、すなわちB<Rには絶対になり得ない。
        // このケース(B=5,000 < R=10,000)は必ずL(=10,000)>B(=5,000)を伴うため、
        // 実際に返るのはInsufficientBurdenではなくLedgerInconsistentである。
        // `InsufficientBurden`はAPI契約上の防御的な安全網(仮に将来excessチェックの
        // 順序が変わっても`checked_sub`がpanicの代わりに安全に失敗する)として維持する。
        let mut h = harness(
            vec![test_country(1, 0, 5_000)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)],
        );
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::LedgerInconsistent));
    }

    #[test]
    fn rejects_when_available_manpower_would_overflow() {
        let mut h = harness(
            vec![test_country(1, u64::MAX, 10_000)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)],
        );
        let before_mobilized = h.countries.get(CountryId(1)).unwrap().mobilized_manpower;
        let result = run(&mut h, CountryId(1), DivisionId(0));
        assert_eq!(result, Err(DemobilizationError::AvailableManpowerOverflow));
        assert_eq!(
            h.countries.get(CountryId(1)).unwrap().mobilized_manpower,
            before_mobilized,
            "overflow検出後もmobilized_manpowerを変更してはいけない"
        );
        assert!(h.military.divisions.contains_key(&DivisionId(0)));
    }
}
