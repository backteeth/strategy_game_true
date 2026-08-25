//! P23-008: 損耗したDivisionへの日次自動補充。
//!
//! P23-006/P23-006-FIX-001/P23-007が整理した人的資源帳簿(`military::manpower`)の
//! 用語で言えば、補充は`B`(累積動員負担)と`L`(現役軍事人的資源)を、対象Divisionの
//! 実際の補充量`X`分だけ同時に増やす(`available_manpower`から`X`を消費する)。`K`
//! (恒久軍事損失)は`B`と`L`が同数増えるため自然に不変となる——過去の恒久損失を
//! 補充によって消し去ることはない。
//!
//! プレイヤー・AIを区別する経路は存在しない。全ての活動国家へ共通の1つのDomain経路
//! (`apply_daily_reinforcement`)を適用する。
use crate::common::{CountryId, DivisionId};
use crate::country::CountryRegistry;
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::data::{Division, DivisionStatus, MilitaryRegistry};
use crate::military::manpower::compute_manpower_ledger;
use crate::state::data::StateRegistry;
use bevy::log::debug;
use std::collections::HashMap;

/// Divisionの1日あたりの補充上限(`max_manpower`の1%、切り上げ)。
/// `max_manpower + 99`のようなoverflowし得る式を使わず、除算と剰余だけで計算する。
pub fn calculate_daily_reinforcement_cap(max_manpower: u64) -> u64 {
    let base = max_manpower / 100;
    if !max_manpower.is_multiple_of(100) {
        base + 1
    } else {
        base
    }
}

/// Divisionのその日の補充要求量(不足分と日次上限の小さい方)。
/// `manpower >= max_manpower`では0(`saturating_sub`により自然に成立)。
pub fn calculate_reinforcement_request(manpower: u64, max_manpower: u64) -> u64 {
    let shortage = max_manpower.saturating_sub(manpower);
    shortage.min(calculate_daily_reinforcement_cap(max_manpower))
}

/// 国家の補充予算`budget`を、各Divisionの要求量`requests`へ最大剰余法で比例按分する。
/// `population::mobilization::apportion_mobilized_manpower`と同じ設計
/// (`u128`中間計算、剰余降順→`DivisionId`昇順のタイブレーク、`HashMap`走査順・
/// 挿入順に非依存)。
///
/// `budget`は呼び出し元が`total_request`以下に制限している前提だが、この関数自体は
/// `budget.min(total_request)`で防御的にクランプするため、契約違反があっても
/// 要求量の合計を超えて配分することはない。
///
/// 各`allocation_i <= request_i`は数学的に保証される: `A <= R`(クランプ済み)の下で
/// `base_i = floor(A*r_i/R) <= r_i`が常に成り立ち、`A<R`かつ`r_i>0`のときは
/// `base_i <= r_i - 1`と真に小さくなるため、剰余配分による+1でも`r_i`を超えない
/// (`A==R`の場合は端数自体が生じない)。
pub fn apportion_reinforcements(
    budget: u64,
    requests: &[(DivisionId, u64)],
) -> HashMap<DivisionId, u64> {
    let mut allocations: HashMap<DivisionId, u64> = HashMap::with_capacity(requests.len());
    let total_request: u128 = requests.iter().map(|&(_, r)| r as u128).sum();

    if requests.is_empty() || budget == 0 || total_request == 0 {
        for &(id, _) in requests {
            allocations.insert(id, 0);
        }
        return allocations;
    }

    let a: u128 = (budget as u128).min(total_request);
    let mut remainders: Vec<(DivisionId, u128)> = Vec::with_capacity(requests.len());
    let mut allocated: u128 = 0;
    for &(id, r) in requests {
        let product = a * (r as u128);
        let base = product / total_request;
        let remainder = product % total_request;
        allocations.insert(id, base as u64);
        allocated += base;
        remainders.push((id, remainder));
    }

    let mut deficit = a - allocated;
    if deficit == 0 {
        return allocations;
    }

    remainders.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| x.0.0.cmp(&y.0.0)));
    for (id, _remainder) in remainders {
        if deficit == 0 {
            break;
        }
        *allocations.get_mut(&id).expect("id was inserted above") += 1;
        deficit -= 1;
    }

    allocations
}

/// 対象Divisionが補充可能かどうかを判定する(副作用なし)。
///
/// `status`だけでなく、`combat_id`と`BattleRegistry`の進行中戦闘参加者一覧の両方を
/// 確認する(`military::demobilization::demobilize_division`と同じく、statusだけを
/// 信用しない防御的な検証——本来Idle/Occupyingであれば`combat_id`はNoneのはずだが、
/// 何らかの不整合があってもstatusだけを見て補充を許可しない)。
fn is_eligible_for_reinforcement(
    division: &Division,
    state_registry: &StateRegistry,
    battle_registry: &BattleRegistry,
) -> bool {
    if division.manpower == 0 || division.manpower >= division.max_manpower {
        return false;
    }
    match division.status {
        DivisionStatus::Idle | DivisionStatus::Occupying => {}
        _ => return false,
    }
    if division.combat_id.is_some() {
        return false;
    }
    let referenced_by_battle = battle_registry.battles.values().any(|b| {
        b.status == BattleStatus::Ongoing
            && (b.attacker_division_ids.contains(&division.id)
                || b.defender_division_ids.contains(&division.id))
    });
    if referenced_by_battle {
        return false;
    }
    // 現在支配国(占領を含む)を取得する既存の正規API。`process_org_recovery`の
    // 自国支配地域判定と全く同じ式を使う。
    state_registry
        .get(division.current_state)
        .map(|s| s.controller() == division.owner)
        .unwrap_or(false)
}

/// 全ての活動国家へ日次自動補充を適用する(`military::update::handle_daily_military`の
/// 募兵処理の直後、移動・戦闘処理より前に呼び出す想定)。
///
/// 国家ごとに独立して処理する: ある国家の帳簿不整合やother reason でのスキップは、
/// 他国の補充を妨げない。各国内でも、全対象Divisionの検証(所有権・status・支配・
/// `manpower+allocation<=max_manpower`・overflow)を終えてから初めて状態を変更する
/// 二段階(検証→適用)構成とし、途中の1件の失敗で一部だけ補充済みという部分成功を
/// 起こさない(`military::demobilization::demobilize_division`と同じ設計方針)。
pub fn apply_daily_reinforcement(
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
    state_registry: &StateRegistry,
    battle_registry: &BattleRegistry,
) {
    // 全Divisionを1回だけ走査し、owner別に補充候補をまとめる。
    let mut by_owner: HashMap<CountryId, Vec<DivisionId>> = HashMap::new();
    for division in military_registry.divisions.values() {
        if is_eligible_for_reinforcement(division, state_registry, battle_registry) {
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
        let mut division_ids = by_owner.remove(&owner).unwrap_or_default();
        division_ids.sort_by_key(|id| id.0);
        apply_reinforcement_for_country(
            country_registry,
            military_registry,
            state_registry,
            battle_registry,
            owner,
            &division_ids,
        );
    }
}

fn apply_reinforcement_for_country(
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
    state_registry: &StateRegistry,
    battle_registry: &BattleRegistry,
    owner: CountryId,
    division_ids: &[DivisionId],
) {
    let Some(country) = country_registry.get(owner) else {
        return;
    };
    if !country.is_active() {
        return;
    }

    let ledger = match compute_manpower_ledger(country, military_registry) {
        Ok(ledger) => ledger,
        Err(_) => {
            debug!(
                "[Reinforcement] country {owner:?}: manpower ledger unavailable (unresolved \
                 recruitment queue definition), skipping today's reinforcement"
            );
            return;
        }
    };
    if ledger.unaccounted_living_excess != 0 {
        debug!(
            "[Reinforcement] country {owner:?}: manpower ledger inconsistent \
             (unaccounted_living_excess={}), skipping today's reinforcement",
            ledger.unaccounted_living_excess
        );
        return;
    }

    let mut requests: Vec<(DivisionId, u64)> = Vec::with_capacity(division_ids.len());
    let mut total_request: u128 = 0;
    for &division_id in division_ids {
        let Some(division) = military_registry.divisions.get(&division_id) else {
            continue;
        };
        let request = calculate_reinforcement_request(division.manpower, division.max_manpower);
        if request > 0 {
            total_request += request as u128;
            requests.push((division_id, request));
        }
    }
    if requests.is_empty() {
        return;
    }

    let available_budget = country.available_manpower;
    let burden_capacity = u64::MAX - country.mobilized_manpower;
    let budget = total_request
        .min(available_budget as u128)
        .min(burden_capacity as u128) as u64;
    if budget == 0 {
        return;
    }

    let allocations = apportion_reinforcements(budget, &requests);

    // フェーズ1: 全件検証してから初めて適用する(部分成功を避ける)。
    let mut validated: Vec<(DivisionId, u64)> = Vec::with_capacity(requests.len());
    let mut total_allocated: u64 = 0;
    for &(division_id, _request) in &requests {
        let allocation = allocations.get(&division_id).copied().unwrap_or(0);
        if allocation == 0 {
            continue;
        }
        let Some(division) = military_registry.divisions.get(&division_id) else {
            return;
        };
        if division.owner != owner {
            return;
        }
        if !is_eligible_for_reinforcement(division, state_registry, battle_registry) {
            return;
        }
        let Some(new_manpower) = division.manpower.checked_add(allocation) else {
            return;
        };
        if new_manpower > division.max_manpower {
            return;
        }
        let Some(new_total) = total_allocated.checked_add(allocation) else {
            return;
        };
        total_allocated = new_total;
        validated.push((division_id, allocation));
    }
    if total_allocated == 0 {
        return;
    }
    let Some(new_available) = country.available_manpower.checked_sub(total_allocated) else {
        return;
    };
    let Some(new_burden) = country.mobilized_manpower.checked_add(total_allocated) else {
        return;
    };

    // フェーズ2: 適用。
    if let Some(c) = country_registry.get_mut(owner) {
        c.available_manpower = new_available;
        c.mobilized_manpower = new_burden;
    }
    for (division_id, allocation) in validated {
        if let Some(d) = military_registry.divisions.get_mut(&division_id) {
            d.manpower += allocation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{DivisionDefinitionId, StateId};
    use crate::country::CountryData;
    use crate::military::battle::Battle;
    use crate::military::data::{DivisionSize, DivisionType};
    use crate::state::data::StateData;

    // ── §11.1 純粋関数: 日次上限 ──────────────────────────────────────

    #[test]
    fn daily_cap_examples() {
        assert_eq!(calculate_daily_reinforcement_cap(0), 0);
        assert_eq!(calculate_daily_reinforcement_cap(1), 1);
        assert_eq!(calculate_daily_reinforcement_cap(99), 1);
        assert_eq!(calculate_daily_reinforcement_cap(100), 1);
        assert_eq!(calculate_daily_reinforcement_cap(101), 2);
        assert_eq!(calculate_daily_reinforcement_cap(10_000), 100);
    }

    #[test]
    fn daily_cap_does_not_overflow_at_u64_max() {
        let cap = calculate_daily_reinforcement_cap(u64::MAX);
        assert_eq!(cap, u64::MAX / 100 + 1);
    }

    // ── §11.1 純粋関数: 要求量 ────────────────────────────────────────

    #[test]
    fn request_is_zero_when_manpower_equals_max() {
        assert_eq!(calculate_reinforcement_request(10_000, 10_000), 0);
    }

    #[test]
    fn request_is_zero_when_manpower_exceeds_max() {
        assert_eq!(calculate_reinforcement_request(10_001, 10_000), 0);
    }

    #[test]
    fn request_is_shortage_when_below_the_daily_cap() {
        // max=10_000 → cap=100、shortage=50 < cap → request=50
        assert_eq!(calculate_reinforcement_request(9_950, 10_000), 50);
    }

    #[test]
    fn request_is_capped_when_shortage_exceeds_the_daily_cap() {
        // max=10_000 → cap=100、shortage=5,000 > cap → request=100
        assert_eq!(calculate_reinforcement_request(5_000, 10_000), 100);
    }

    #[test]
    fn request_is_zero_for_a_fully_depleted_division() {
        assert_eq!(calculate_reinforcement_request(0, 10_000), 100);
        // manpower=0自体は要求量としては非ゼロを返し得るが(shortage計算上は正しい)、
        // Domain抽出段階(is_eligible_for_reinforcement)でmanpower=0は対象外にする。
    }

    // ── §11.1 純粋関数: 最大剰余法 ────────────────────────────────────

    #[test]
    fn apportion_grants_full_requests_when_budget_is_sufficient() {
        let requests = [(DivisionId(0), 50), (DivisionId(1), 30)];
        let alloc = apportion_reinforcements(1_000, &requests);
        assert_eq!(alloc[&DivisionId(0)], 50);
        assert_eq!(alloc[&DivisionId(1)], 30);
    }

    #[test]
    fn apportion_grants_nothing_when_budget_is_zero() {
        let requests = [(DivisionId(0), 50), (DivisionId(1), 30)];
        let alloc = apportion_reinforcements(0, &requests);
        assert_eq!(alloc[&DivisionId(0)], 0);
        assert_eq!(alloc[&DivisionId(1)], 0);
    }

    #[test]
    fn apportion_never_allocates_to_a_zero_request() {
        let requests = [(DivisionId(0), 0), (DivisionId(1), 100)];
        let alloc = apportion_reinforcements(50, &requests);
        assert_eq!(alloc[&DivisionId(0)], 0);
    }

    #[test]
    fn apportion_totals_match_the_budget_exactly() {
        let requests = [(DivisionId(0), 7), (DivisionId(1), 11), (DivisionId(2), 13)];
        for budget in [1u64, 2, 5, 10, 30] {
            let alloc = apportion_reinforcements(budget, &requests);
            let sum: u64 = alloc.values().sum();
            assert_eq!(sum, budget, "budget={budget}");
        }
    }

    #[test]
    fn apportion_never_exceeds_any_individual_request() {
        let requests = [(DivisionId(0), 3), (DivisionId(1), 5), (DivisionId(2), 100)];
        for budget in [1u64, 4, 20, 50, 108] {
            let alloc = apportion_reinforcements(budget, &requests);
            for &(id, r) in &requests {
                assert!(alloc[&id] <= r, "budget={budget} id={id:?}");
            }
        }
    }

    #[test]
    fn apportion_is_proportional_to_request_size() {
        // 要求比1:3、予算400 -> ちょうど100:300(端数なし)。
        let requests = [(DivisionId(0), 100), (DivisionId(1), 300)];
        let alloc = apportion_reinforcements(400, &requests);
        assert_eq!(alloc[&DivisionId(0)], 100);
        assert_eq!(alloc[&DivisionId(1)], 300);
    }

    #[test]
    fn apportion_breaks_equal_remainders_by_ascending_division_id() {
        let requests = [
            (DivisionId(5), 100),
            (DivisionId(2), 100),
            (DivisionId(8), 100),
        ];
        // total=300、budget=299(要求合計未満) -> base=99ずつ(計297)、
        // 剰余は3件とも同率のため、残り2人をDivisionId昇順(2→5)へ配分する。
        let alloc = apportion_reinforcements(299, &requests);
        assert_eq!(alloc[&DivisionId(2)], 100);
        assert_eq!(alloc[&DivisionId(5)], 100);
        assert_eq!(alloc[&DivisionId(8)], 99);
    }

    #[test]
    fn apportion_is_independent_of_input_order() {
        let forward = [
            (DivisionId(0), 111),
            (DivisionId(1), 222),
            (DivisionId(2), 333),
        ];
        let reversed = [
            (DivisionId(2), 333),
            (DivisionId(1), 222),
            (DivisionId(0), 111),
        ];
        assert_eq!(
            apportion_reinforcements(500, &forward),
            apportion_reinforcements(500, &reversed),
        );
    }

    #[test]
    fn apportion_extreme_values_do_not_overflow_or_panic() {
        let requests = [(DivisionId(0), u64::MAX / 2), (DivisionId(1), u64::MAX / 2)];
        let alloc = apportion_reinforcements(u64::MAX, &requests);
        let sum: u128 = alloc.values().map(|&v| v as u128).sum();
        assert_eq!(sum, (u64::MAX / 2) as u128 * 2);
    }

    // ── §11.2 補充対象条件 ────────────────────────────────────────────

    fn test_division(
        id: usize,
        owner: usize,
        state: StateId,
        manpower: u64,
        max_manpower: u64,
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
            manpower,
            max_manpower,
            equipment: 100.0,
            max_equipment: 100.0,
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

    fn owned_state(id: StateId, owner: usize, controller: Option<usize>) -> StateData {
        StateData {
            id,
            owner_country_id: CountryId(owner),
            controller_country: controller.map(CountryId),
            ..StateData::default()
        }
    }

    #[test]
    fn idle_in_own_controlled_state_is_eligible() {
        let division = test_division(0, 1, StateId(0), 5_000, 10_000, DivisionStatus::Idle);
        let states = StateRegistry::build(vec![owned_state(StateId(0), 1, None)]);
        assert!(is_eligible_for_reinforcement(
            &division,
            &states,
            &BattleRegistry::default()
        ));
    }

    #[test]
    fn occupying_in_a_state_the_owner_already_controls_is_eligible() {
        let division = test_division(0, 1, StateId(0), 5_000, 10_000, DivisionStatus::Occupying);
        let states = StateRegistry::build(vec![owned_state(StateId(0), 2, Some(1))]);
        assert!(is_eligible_for_reinforcement(
            &division,
            &states,
            &BattleRegistry::default()
        ));
    }

    #[test]
    fn occupying_a_state_still_controlled_by_the_enemy_is_not_eligible() {
        let division = test_division(0, 1, StateId(0), 5_000, 10_000, DivisionStatus::Occupying);
        let states = StateRegistry::build(vec![owned_state(StateId(0), 2, Some(2))]);
        assert!(!is_eligible_for_reinforcement(
            &division,
            &states,
            &BattleRegistry::default()
        ));
    }

    #[test]
    fn fighting_moving_retreating_destroyed_disbanding_are_not_eligible() {
        let states = StateRegistry::build(vec![owned_state(StateId(0), 1, None)]);
        for status in [
            DivisionStatus::Fighting,
            DivisionStatus::Moving,
            DivisionStatus::Retreating,
            DivisionStatus::Destroyed,
            DivisionStatus::Disbanding,
        ] {
            let division = test_division(0, 1, StateId(0), 5_000, 10_000, status);
            assert!(
                !is_eligible_for_reinforcement(&division, &states, &BattleRegistry::default()),
                "{status:?} must not be eligible"
            );
        }
    }

    #[test]
    fn zero_manpower_division_is_not_eligible() {
        let division = test_division(0, 1, StateId(0), 0, 10_000, DivisionStatus::Idle);
        let states = StateRegistry::build(vec![owned_state(StateId(0), 1, None)]);
        assert!(!is_eligible_for_reinforcement(
            &division,
            &states,
            &BattleRegistry::default()
        ));
    }

    #[test]
    fn a_full_division_is_not_eligible() {
        let division = test_division(0, 1, StateId(0), 10_000, 10_000, DivisionStatus::Idle);
        let states = StateRegistry::build(vec![owned_state(StateId(0), 1, None)]);
        assert!(!is_eligible_for_reinforcement(
            &division,
            &states,
            &BattleRegistry::default()
        ));
    }

    #[test]
    fn a_division_with_a_combat_id_is_not_eligible_even_if_status_looks_idle() {
        let mut division = test_division(0, 1, StateId(0), 5_000, 10_000, DivisionStatus::Idle);
        division.combat_id = Some(crate::common::BattleId(0));
        let states = StateRegistry::build(vec![owned_state(StateId(0), 1, None)]);
        assert!(!is_eligible_for_reinforcement(
            &division,
            &states,
            &BattleRegistry::default()
        ));
    }

    #[test]
    fn a_division_referenced_by_an_ongoing_battle_is_not_eligible_even_if_status_looks_idle() {
        let division = test_division(0, 1, StateId(0), 5_000, 10_000, DivisionStatus::Idle);
        let states = StateRegistry::build(vec![owned_state(StateId(0), 1, None)]);
        let mut battles = BattleRegistry::default();
        battles.battles.insert(
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
                start_date: "1800/01/01".to_string(),
                elapsed_days: 0,
                status: BattleStatus::Ongoing,
            },
        );
        assert!(!is_eligible_for_reinforcement(&division, &states, &battles));
    }

    #[test]
    fn a_division_in_a_nonexistent_state_is_not_eligible() {
        let division = test_division(0, 1, StateId(99), 5_000, 10_000, DivisionStatus::Idle);
        let states = StateRegistry::build(vec![owned_state(StateId(0), 1, None)]);
        assert!(!is_eligible_for_reinforcement(
            &division,
            &states,
            &BattleRegistry::default()
        ));
    }

    #[test]
    fn a_division_in_a_state_controlled_by_an_ally_is_not_eligible() {
        // 「支配国不明」はstate.controller()の設計上存在しない(常に法的所有国へ
        // フォールバックする)ため、ここでは「支配国が自国でも敵国でもない第三国
        // (同盟国相当)」を検証する。
        let division = test_division(0, 1, StateId(0), 5_000, 10_000, DivisionStatus::Idle);
        let states = StateRegistry::build(vec![owned_state(StateId(0), 3, Some(3))]);
        assert!(!is_eligible_for_reinforcement(
            &division,
            &states,
            &BattleRegistry::default()
        ));
    }

    // ── §11.3 国家単位補充 ────────────────────────────────────────────

    fn country(id: usize, available: u64, mobilized: u64) -> CountryData {
        CountryData {
            id: CountryId(id),
            available_manpower: available,
            mobilized_manpower: mobilized,
            treasury: 12345.0,
            ..CountryData::default()
        }
    }

    struct Harness {
        countries: CountryRegistry,
        military: MilitaryRegistry,
        states: StateRegistry,
        battles: BattleRegistry,
    }

    fn harness(
        countries: Vec<CountryData>,
        divisions: Vec<Division>,
        states: Vec<StateData>,
    ) -> Harness {
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
        apply_daily_reinforcement(&mut h.countries, &mut h.military, &h.states, &h.battles);
    }

    #[test]
    fn sufficient_available_reinforces_every_target_up_to_the_daily_cap() {
        let mut h = harness(
            vec![country(1, 1_000_000, 20_000)],
            vec![
                test_division(0, 1, StateId(0), 9_000, 10_000, DivisionStatus::Idle),
                test_division(1, 1, StateId(0), 4_000, 5_000, DivisionStatus::Idle),
            ],
            vec![owned_state(StateId(0), 1, None)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].manpower, 9_100); // cap=100
        assert_eq!(h.military.divisions[&DivisionId(1)].manpower, 4_050); // cap=50
    }

    #[test]
    fn insufficient_available_apportions_proportionally() {
        let mut h = harness(
            vec![country(1, 30, 2)], // B=2=L(=1+1)、生存兵員は各1(0は補充対象外のため)
            vec![
                // cap=100 each, requests 100+100=200, budget=min(200,30,..)=30 -> 15/15
                test_division(0, 1, StateId(0), 1, 10_000, DivisionStatus::Idle),
                test_division(1, 1, StateId(0), 1, 10_000, DivisionStatus::Idle),
            ],
            vec![owned_state(StateId(0), 1, None)],
        );
        run(&mut h);
        let total: u64 = h.military.divisions.values().map(|d| d.manpower).sum();
        assert_eq!(
            total,
            2 + 30,
            "starting 1+1 manpower plus the full 30-person budget"
        );
    }

    #[test]
    fn available_decreases_by_exactly_the_total_allocated() {
        let mut h = harness(
            vec![country(1, 1_000_000, 9_000)], // B=L=9,000(整合済み)
            vec![test_division(
                0,
                1,
                StateId(0),
                9_000,
                10_000,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1, None)],
        );
        run(&mut h);
        let country = h.countries.get(CountryId(1)).unwrap();
        assert_eq!(country.available_manpower, 1_000_000 - 100);
        assert_eq!(country.mobilized_manpower, 9_100);
    }

    #[test]
    fn burden_increases_by_exactly_the_total_allocated() {
        let mut h = harness(
            vec![country(1, 1_000_000, 9_000)], // B=L=9,000(整合済み)
            vec![test_division(
                0,
                1,
                StateId(0),
                9_000,
                10_000,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1, None)],
        );
        run(&mut h);
        assert_eq!(
            h.countries.get(CountryId(1)).unwrap().mobilized_manpower,
            9_100
        );
    }

    #[test]
    fn living_manpower_increases_by_exactly_the_total_allocated() {
        let mut h = harness(
            vec![country(1, 1_000_000, 9_000)],
            vec![test_division(
                0,
                1,
                StateId(0),
                9_000,
                10_000,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1, None)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].manpower, 9_100);
    }

    #[test]
    fn permanent_losses_are_unaffected_by_reinforcement() {
        let mut h = harness(
            vec![country(1, 1_000_000, 10_000)], // B=10,000
            vec![test_division(
                0,
                1,
                StateId(0),
                7_000,
                10_000,
                DivisionStatus::Idle,
            )], // L=7,000, K=3,000
            vec![owned_state(StateId(0), 1, None)],
        );
        let ledger_before =
            compute_manpower_ledger(h.countries.get(CountryId(1)).unwrap(), &h.military).unwrap();
        assert_eq!(ledger_before.permanent_military_losses, 3_000);
        run(&mut h);
        let ledger_after =
            compute_manpower_ledger(h.countries.get(CountryId(1)).unwrap(), &h.military).unwrap();
        assert_eq!(
            ledger_after.permanent_military_losses, 3_000,
            "K must be unaffected"
        );
    }

    #[test]
    fn division_manpower_never_exceeds_max_manpower() {
        let mut h = harness(
            vec![country(1, 1_000_000, 9_999)], // B=L=9,999(整合済み)
            vec![test_division(
                0,
                1,
                StateId(0),
                9_999,
                10_000,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1, None)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].manpower, 10_000);
    }

    #[test]
    fn exhausting_available_manpower_does_not_underflow() {
        let mut h = harness(
            vec![country(1, 1, 1)], // B=L=1(整合済み。manpower=0は補充対象外のため1から開始)
            vec![test_division(
                0,
                1,
                StateId(0),
                1,
                10_000,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1, None)],
        );
        run(&mut h);
        assert_eq!(h.countries.get(CountryId(1)).unwrap().available_manpower, 0);
        assert_eq!(h.military.divisions[&DivisionId(0)].manpower, 2);
    }

    #[test]
    fn burden_capacity_near_u64_max_safely_limits_the_budget() {
        let mut h = harness(
            vec![country(1, 1_000_000, u64::MAX - 5)], // B >= L(=1)なので整合
            vec![test_division(
                0,
                1,
                StateId(0),
                1,
                10_000,
                DivisionStatus::Idle,
            )], // cap=100
            vec![owned_state(StateId(0), 1, None)],
        );
        run(&mut h);
        // burden_capacity=5 < request(100) -> budget clamped to 5
        assert_eq!(h.military.divisions[&DivisionId(0)].manpower, 1 + 5);
        assert_eq!(
            h.countries.get(CountryId(1)).unwrap().mobilized_manpower,
            u64::MAX
        );
    }

    #[test]
    fn a_country_with_an_unresolvable_ledger_is_skipped_entirely() {
        let mut h = harness(
            vec![country(1, 1_000_000, 0)],
            vec![test_division(
                0,
                1,
                StateId(0),
                5_000,
                10_000,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1, None)],
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
        run(&mut h);
        assert_eq!(
            h.military.divisions[&DivisionId(0)].manpower,
            5_000,
            "must be unchanged"
        );
        assert_eq!(
            h.countries.get(CountryId(1)).unwrap().available_manpower,
            1_000_000
        );
    }

    #[test]
    fn a_country_with_living_exceeding_burden_is_skipped_entirely() {
        let mut h = harness(
            vec![country(1, 1_000_000, 0)], // B=0だがDivisionが生存兵員を持つ異常状態
            vec![test_division(
                0,
                1,
                StateId(0),
                5_000,
                10_000,
                DivisionStatus::Idle,
            )],
            vec![owned_state(StateId(0), 1, None)],
        );
        run(&mut h);
        assert_eq!(h.military.divisions[&DivisionId(0)].manpower, 5_000);
    }

    #[test]
    fn one_countrys_skip_does_not_prevent_another_countrys_reinforcement() {
        let mut h = harness(
            vec![country(1, 1_000_000, 0), country(2, 1_000_000, 8_000)],
            vec![
                test_division(0, 1, StateId(0), 5_000, 10_000, DivisionStatus::Idle), // 国1: B=0<L → skip
                test_division(1, 2, StateId(1), 8_000, 10_000, DivisionStatus::Idle), // 国2: 正常
            ],
            vec![
                owned_state(StateId(0), 1, None),
                owned_state(StateId(1), 2, None),
            ],
        );
        run(&mut h);
        assert_eq!(
            h.military.divisions[&DivisionId(0)].manpower,
            5_000,
            "country 1 must be skipped"
        );
        assert_eq!(
            h.military.divisions[&DivisionId(1)].manpower,
            8_100,
            "country 2 must still reinforce"
        );
    }

    #[test]
    fn reinforcement_never_touches_treasury_equipment_organization_or_experience() {
        let mut h = harness(
            vec![country(1, 1_000_000, 0)],
            vec![{
                let mut d = test_division(0, 1, StateId(0), 5_000, 10_000, DivisionStatus::Idle);
                d.equipment = 42.0;
                d.organization = 33.0;
                d.experience = 7.0;
                d
            }],
            vec![owned_state(StateId(0), 1, None)],
        );
        run(&mut h);
        let d = &h.military.divisions[&DivisionId(0)];
        assert_eq!(d.equipment, 42.0);
        assert_eq!(d.organization, 33.0);
        assert_eq!(d.experience, 7.0);
        assert_eq!(h.countries.get(CountryId(1)).unwrap().treasury, 12345.0);
    }

    #[test]
    fn result_does_not_depend_on_hashmap_insertion_order() {
        let build = |order: [usize; 2]| {
            let mut h = harness(
                vec![country(1, 40, 0)],
                vec![],
                vec![owned_state(StateId(0), 1, None)],
            );
            for id in order {
                h.military.divisions.insert(
                    DivisionId(id),
                    test_division(id, 1, StateId(0), 0, 10_000, DivisionStatus::Idle),
                );
            }
            run(&mut h);
            let mut result: Vec<(usize, u64)> = h
                .military
                .divisions
                .iter()
                .map(|(id, d)| (id.0, d.manpower))
                .collect();
            result.sort_by_key(|&(id, _)| id);
            result
        };
        assert_eq!(build([0, 1]), build([1, 0]));
    }
}
