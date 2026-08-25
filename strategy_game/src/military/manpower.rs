//! P23-006: 国家単位の人的資源帳簿。
//!
//! P23-005の監査により、`CountryData.mobilized_manpower`は「募兵で加算され、
//! 戦闘損耗・部隊撃破では意図的に減算されない累積値」であり、フィールド名が示唆する
//! 「現在生存して軍に所属している人数」とは異なることが判明した。本モジュールは、
//! この既存フィールドを**累積動員負担**として正式に扱い、現在生存する軍事人的資源と
//! 恒久軍事損失を、既存のCountry/Division/募兵キューから毎回導出する。
//!
//! 新しい永続フィールドは追加しない。3値の関係は次のとおり:
//!
//! ```text
//! B (workforce_burden)          = CountryData.mobilized_manpower(既存フィールドそのもの)
//! L (living_military_manpower)  = 募兵キュー内の予約済み生存兵員 + 生存Divisionの現在manpower合計
//! K (permanent_military_losses) = B.saturating_sub(L)
//! ```
//!
//! `L`が`B`を上回る(本来あり得ないはずの)異常状態も`unaccounted_living_excess`
//! (`L.saturating_sub(B)`)として検出可能にする——`K`を負値にする代わりに0のまま
//! 安全に保ち、異常の有無は別フィールドで表現する。
//!
//! **本モジュールは`mobilized_manpower`を一切書き換えない。** 帳簿の導出のみを行う
//! 読み取り専用の集計であり、戦闘・募兵・save/loadのいずれのコードにも書き込み経路を
//! 追加しない(書き込みは引き続き`military::recruitment`と、New Game初期軍配置分のみ
//! `app::loader::spawn_debug_divisions`が行う——P23-006-FIX-001)。
//!
//! `compute_manpower_ledger`は`Result<ManpowerLedger, ManpowerLedgerError>`を返す
//! (P23-006-FIX-001)。募兵キューが未解決の`DivisionDefinitionId`を参照している場合、
//! その分を0人として黙って`Ok`を返さない——未解決分を恒久軍事損失として誤分類する
//! ことを防ぐため。

use crate::common::DivisionDefinitionId;
use crate::country::CountryData;
use crate::military::data::{DivisionStatus, MilitaryRegistry};

/// 国家単位の人的資源帳簿(P23-006)。全フィールドは既存データからの導出値であり、
/// どこにも永続化しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ManpowerLedger {
    /// 累積動員負担(`CountryData.mobilized_manpower`そのもの)。
    pub workforce_burden: u64,
    /// 現在生存する軍事人的資源(募兵キュー予約分 + 生存Division現在manpower合計)。
    pub living_military_manpower: u64,
    /// 恒久軍事損失。`workforce_burden.saturating_sub(living_military_manpower)`。
    pub permanent_military_losses: u64,
    /// 異常検出用。本来`living_military_manpower <= workforce_burden`のはずだが、
    /// 何らかの理由で上回っていた場合の超過量(`saturating_sub`により通常は0)。
    pub unaccounted_living_excess: u64,
}

/// `compute_manpower_ledger`が帳簿を完全に導出できなかったことを表す
/// (P23-006-FIX-001)。募兵キューが`military_registry.definitions`に存在しない
/// `DivisionDefinitionId`を参照している場合に発生する。
///
/// この状態を`ManpowerLedger`の一部として(例えば該当分を0人として)返すと、
/// 未解決の予約人数が「恒久軍事損失」として誤って確定してしまう
/// (`K = B.saturating_sub(L)`の`L`が実際より小さくなるため)。そのため、帳簿全体を
/// 計算失敗として明示的に表現する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManpowerLedgerError {
    /// 未解決の`DivisionDefinitionId`一覧。昇順にソート・重複除去済みであり、
    /// `recruitment_queue`(`Vec`)内の出現順や`military_registry.definitions`
    /// (`HashMap`)の走査順のいずれにも依存しない、決定的な結果になる。
    pub unknown_definitions: Vec<DivisionDefinitionId>,
}

/// 指定した国家の人的資源帳簿を、`CountryData`と`MilitaryRegistry`の現在値から
/// 導出する(読み取り専用、副作用なし)。
///
/// 募兵キューの全項目が参照する`DivisionDefinition`が`military_registry.definitions`に
/// 存在する場合のみ`Ok`を返す。`save::validate`はセーブ/ロード時点でこの不変条件を
/// 既に検証する(`unknown_recruitment_division_definition_is_rejected`)ため、
/// New Gameおよび検証済みV1セーブでは常に`Ok`になるはずだが、この関数自体は
/// その外部の不変条件を信頼せず、違反時は`Err`を返す(未解決分を0人として
/// 静かに扱うと、実際には確定していない人数が恒久軍事損失として誤分類されるため)。
///
/// 生存Divisionの判定は既存コードベース全体で使われている規約
/// (`manpower == 0 || status == DivisionStatus::Destroyed`を「死亡」とみなす)の
/// 否定を用いる。`HashMap`の走査順には依存しない(合計は可換な加算のみ、エラー時の
/// ID一覧はソート済み)。
pub fn compute_manpower_ledger(
    country: &CountryData,
    military_registry: &MilitaryRegistry,
) -> Result<ManpowerLedger, ManpowerLedgerError> {
    let mut unknown_definitions: Vec<DivisionDefinitionId> = country
        .recruitment_queue
        .iter()
        .map(|item| item.division_id)
        .filter(|id| !military_registry.definitions.contains_key(id))
        .collect();
    if !unknown_definitions.is_empty() {
        unknown_definitions.sort_by_key(|id| id.0);
        unknown_definitions.dedup();
        return Err(ManpowerLedgerError {
            unknown_definitions,
        });
    }

    let queue_living: u128 = country
        .recruitment_queue
        .iter()
        .filter_map(|item| military_registry.definitions.get(&item.division_id))
        .map(|def| def.required_manpower as u128)
        .sum();

    let division_living: u128 = military_registry
        .divisions
        .values()
        .filter(|d| d.owner == country.id && is_living(d))
        .map(|d| d.manpower as u128)
        .sum();

    let living_military_manpower = (queue_living + division_living).min(u64::MAX as u128) as u64;
    let workforce_burden = country.mobilized_manpower;

    Ok(ManpowerLedger {
        workforce_burden,
        living_military_manpower,
        permanent_military_losses: workforce_burden.saturating_sub(living_military_manpower),
        unaccounted_living_excess: living_military_manpower.saturating_sub(workforce_burden),
    })
}

/// 生存Divisionかどうかを既存の規約(各所の生存判定の否定)で判定する。
fn is_living(division: &crate::military::data::Division) -> bool {
    division.manpower > 0 && division.status != DivisionStatus::Destroyed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DivisionDefinitionId, DivisionId, StateId};
    use crate::military::data::{Division, DivisionDefinition, DivisionSize, DivisionType};
    use crate::military::recruitment::RecruitmentQueueItem;

    fn test_country(id: usize, mobilized_manpower: u64) -> CountryData {
        CountryData {
            id: CountryId(id),
            mobilized_manpower,
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
            max_manpower: manpower.max(1),
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

    fn test_definition(id: usize, required_manpower: u64) -> DivisionDefinition {
        DivisionDefinition {
            id: DivisionDefinitionId(id),
            name: "Test".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower,
            required_equipment: 100.0,
            recruitment_days: 30,
            movement_speed: 1.0,
            attack: 10.0,
            defense: 10.0,
            breakthrough: 5.0,
            organization: 50.0,
            morale: 50.0,
            supply_usage: 1.0,
            maintenance_cost: 1.0,
        }
    }

    fn registry_with(
        definitions: Vec<DivisionDefinition>,
        divisions: Vec<Division>,
    ) -> MilitaryRegistry {
        let mut registry = MilitaryRegistry::default();
        for def in definitions {
            registry.definitions.insert(def.id, def);
        }
        for division in divisions {
            registry.divisions.insert(division.id, division);
        }
        registry
    }

    #[test]
    fn no_divisions_and_no_queue_yields_zero_living_manpower() {
        let country = test_country(1, 0);
        let registry = MilitaryRegistry::default();
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(ledger.living_military_manpower, 0);
        assert_eq!(ledger.permanent_military_losses, 0);
        assert_eq!(ledger.unaccounted_living_excess, 0);
    }

    #[test]
    fn queue_only_contributes_the_definitions_required_manpower() {
        let mut country = test_country(1, 10_000);
        country.recruitment_queue.push(RecruitmentQueueItem {
            division_id: DivisionDefinitionId(0),
            target_state: StateId(0),
            days_remaining: 10,
            total_days: 30,
        });
        let registry = registry_with(vec![test_definition(0, 10_000)], vec![]);
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(ledger.living_military_manpower, 10_000);
    }

    #[test]
    fn divisions_only_contribute_their_current_manpower() {
        let country = test_country(1, 8_000);
        let registry = registry_with(
            vec![],
            vec![test_division(0, 1, 8_000, DivisionStatus::Idle)],
        );
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(ledger.living_military_manpower, 8_000);
    }

    #[test]
    fn queue_and_multiple_divisions_sum_correctly() {
        let mut country = test_country(1, 25_000);
        country.recruitment_queue.push(RecruitmentQueueItem {
            division_id: DivisionDefinitionId(0),
            target_state: StateId(0),
            days_remaining: 5,
            total_days: 30,
        });
        let registry = registry_with(
            vec![test_definition(0, 10_000)],
            vec![
                test_division(0, 1, 6_000, DivisionStatus::Idle),
                test_division(1, 1, 9_000, DivisionStatus::Fighting),
            ],
        );
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(ledger.living_military_manpower, 10_000 + 6_000 + 9_000);
    }

    #[test]
    fn another_countrys_divisions_are_excluded() {
        // `recruitment_queue`は`CountryData`自身のフィールドのため、そもそも
        // 「別国のqueueを対象国のqueueへ混入させる」経路は型として存在しない。
        // 実際に混入し得るのは`MilitaryRegistry.divisions`(全国家共有)側の
        // `owner`フィルタのみであるため、ここではそちらを検証する。
        let country = test_country(1, 0);
        let registry = registry_with(
            vec![],
            vec![
                test_division(0, 2, 5_000, DivisionStatus::Idle), // 他国(CountryId(2))
                test_division(1, 3, 7_000, DivisionStatus::Idle), // 他国(CountryId(3))
            ],
        );
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(
            ledger.living_military_manpower, 0,
            "another country's divisions must not be counted"
        );
    }

    #[test]
    fn a_division_with_zero_manpower_contributes_nothing() {
        let country = test_country(1, 5_000);
        let registry = registry_with(
            vec![],
            vec![test_division(0, 1, 0, DivisionStatus::Fighting)],
        );
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(ledger.living_military_manpower, 0);
    }

    #[test]
    fn a_destroyed_division_is_excluded_even_with_stale_positive_manpower() {
        // war::peace::relocate_hostile_territory_divisionsはstatus=Destroyedと
        // manpower=0を同時に設定するのが正規の経路だが、既存の生存判定規約
        // (manpower==0 || status==Destroyed)に合わせ、statusだけでも除外されることを固定する。
        let country = test_country(1, 5_000);
        let registry = registry_with(
            vec![],
            vec![test_division(0, 1, 999, DivisionStatus::Destroyed)],
        );
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(
            ledger.living_military_manpower, 0,
            "a Destroyed division must not count as living even if manpower wasn't zeroed"
        );
    }

    #[test]
    fn burden_equal_to_living_yields_zero_permanent_losses() {
        let country = test_country(1, 8_000);
        let registry = registry_with(
            vec![],
            vec![test_division(0, 1, 8_000, DivisionStatus::Idle)],
        );
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(ledger.permanent_military_losses, 0);
        assert_eq!(ledger.unaccounted_living_excess, 0);
    }

    #[test]
    fn burden_greater_than_living_yields_the_difference_as_permanent_losses() {
        let country = test_country(1, 10_000);
        let registry = registry_with(
            vec![],
            vec![test_division(0, 1, 6_000, DivisionStatus::Idle)],
        );
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(ledger.permanent_military_losses, 4_000);
        assert_eq!(ledger.unaccounted_living_excess, 0);
    }

    #[test]
    fn living_greater_than_burden_yields_zero_losses_and_detects_the_excess() {
        // 本来あり得ない異常状態(手動でmobilized_manpowerを不整合な値にした場合など)
        // でもpanicせず、K=0のまま超過量を別フィールドで検出できることを固定する。
        let country = test_country(1, 3_000);
        let registry = registry_with(
            vec![],
            vec![test_division(0, 1, 8_000, DivisionStatus::Idle)],
        );
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(ledger.permanent_military_losses, 0);
        assert_eq!(ledger.unaccounted_living_excess, 5_000);
    }

    #[test]
    fn extreme_values_near_u64_max_do_not_overflow_or_panic() {
        let country = test_country(1, u64::MAX);
        let registry = registry_with(
            vec![],
            vec![
                test_division(0, 1, u64::MAX, DivisionStatus::Idle),
                test_division(1, 1, u64::MAX, DivisionStatus::Idle),
            ],
        );
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(ledger.living_military_manpower, u64::MAX);
        assert_eq!(ledger.permanent_military_losses, 0);
        assert_eq!(ledger.unaccounted_living_excess, 0);
    }

    #[test]
    fn insertion_order_does_not_affect_the_result() {
        let country = test_country(1, 30_000);
        let forward = registry_with(
            vec![],
            vec![
                test_division(0, 1, 5_000, DivisionStatus::Idle),
                test_division(1, 1, 7_000, DivisionStatus::Idle),
                test_division(2, 1, 9_000, DivisionStatus::Idle),
            ],
        );
        let reversed = registry_with(
            vec![],
            vec![
                test_division(2, 1, 9_000, DivisionStatus::Idle),
                test_division(1, 1, 7_000, DivisionStatus::Idle),
                test_division(0, 1, 5_000, DivisionStatus::Idle),
            ],
        );
        assert_eq!(
            compute_manpower_ledger(&country, &forward).unwrap(),
            compute_manpower_ledger(&country, &reversed).unwrap()
        );
    }

    #[test]
    fn an_unknown_division_definition_in_the_queue_returns_an_explicit_error_without_panicking() {
        // save::validateがロード時点でunknown_recruitment_division_definitionを
        // 既に拒否するため、本来この状態は正規のゲーム状態には現れない。それでも
        // このDomain関数自体は防御的にpanicせず、未解決分を0人の正常値として
        // 黙って返さないことを固定する(P23-006-FIX-001)。
        let mut country = test_country(1, 0);
        country.recruitment_queue.push(RecruitmentQueueItem {
            division_id: DivisionDefinitionId(999), // 存在しない定義
            target_state: StateId(0),
            days_remaining: 5,
            total_days: 30,
        });
        let registry = MilitaryRegistry::default(); // definitionsは空
        let result = compute_manpower_ledger(&country, &registry);
        assert_eq!(
            result,
            Err(ManpowerLedgerError {
                unknown_definitions: vec![DivisionDefinitionId(999)]
            })
        );
    }

    #[test]
    fn multiple_unknown_definitions_are_reported_sorted_and_deduplicated() {
        let mut country = test_country(1, 0);
        for (id, days) in [(999usize, 5u32), (5, 10), (999, 20), (5, 1)] {
            country.recruitment_queue.push(RecruitmentQueueItem {
                division_id: DivisionDefinitionId(id),
                target_state: StateId(0),
                days_remaining: days,
                total_days: 30,
            });
        }
        let registry = MilitaryRegistry::default();
        let result = compute_manpower_ledger(&country, &registry);
        assert_eq!(
            result,
            Err(ManpowerLedgerError {
                unknown_definitions: vec![DivisionDefinitionId(5), DivisionDefinitionId(999)]
            }),
            "the unknown id list must be sorted ascending and deduplicated regardless of \
             queue insertion order"
        );
    }

    #[test]
    fn a_queue_mixing_known_and_unknown_definitions_still_errors() {
        let mut country = test_country(1, 10_000);
        country.recruitment_queue.push(RecruitmentQueueItem {
            division_id: DivisionDefinitionId(0),
            target_state: StateId(0),
            days_remaining: 5,
            total_days: 30,
        });
        country.recruitment_queue.push(RecruitmentQueueItem {
            division_id: DivisionDefinitionId(42),
            target_state: StateId(0),
            days_remaining: 5,
            total_days: 30,
        });
        let registry = registry_with(vec![test_definition(0, 10_000)], vec![]);
        let result = compute_manpower_ledger(&country, &registry);
        assert_eq!(
            result,
            Err(ManpowerLedgerError {
                unknown_definitions: vec![DivisionDefinitionId(42)]
            }),
            "even one unknown definition must fail the whole ledger, not just skip it"
        );
    }

    #[test]
    fn a_fully_resolved_queue_is_unaffected_by_the_error_path() {
        // 正常データではP23-006の値を一切変えないことを確認する回帰。
        let mut country = test_country(1, 10_000);
        country.recruitment_queue.push(RecruitmentQueueItem {
            division_id: DivisionDefinitionId(0),
            target_state: StateId(0),
            days_remaining: 5,
            total_days: 30,
        });
        let registry = registry_with(vec![test_definition(0, 10_000)], vec![]);
        let ledger = compute_manpower_ledger(&country, &registry).unwrap();
        assert_eq!(ledger.living_military_manpower, 10_000);
        assert_eq!(ledger.workforce_burden, 10_000);
        assert_eq!(ledger.permanent_military_losses, 0);
        assert_eq!(ledger.unaccounted_living_excess, 0);
    }
}
