//! P23-002: 国家の`mobilized_manpower`を所有州へ人口比で按分し、月次雇用計算が
//! 使う民間労働力から控除する。
//!
//! `mobilized_manpower`(募兵で加算、キャンセルで全額返還、P21-001由来の既存フィールド)を
//! 正規の情報源とし、按分結果自体は永続化しない派生値として毎月再計算する
//! (`P23-001`の`available_manpower`基準上限と同じ設計判断)。

use crate::common::StateId;
use std::collections::HashMap;

/// 国家の`mobilized_manpower`を、所有州の人口比で按分する(最大剰余法)。
///
/// - `states`は`(StateId, 人口)`のペア(呼び出し元が現在の所有州から構築する、順序は
///   結果に影響しない——後述)。
/// - 各州の基礎配分は`floor(mobilized_manpower * population_i / population_total)`。
/// - 端数切り捨てで生じた不足分(`mobilized_manpower - Σbase_i`)は、剰余
///   (`mobilized_manpower * population_i % population_total`)が大きい州から1人ずつ
///   割り当てる。剰余が同じ場合は`StateId`昇順(安定した既存の順序基準)で決定するため、
///   `states`スライス自体の並び順やHashMapの内部走査順には一切依存しない。
/// - `population_total == 0`(所有州なし、または全所有州の人口が0)の場合は0除算を避け、
///   全州の負担を0として返す。
/// - `mobilized_manpower == 0`の場合も全州0(自然にそうなるが明示不要な特別扱いはしない)。
/// - 人口0の州は`base_i`・剰余とも常に0のため、剰余配分でも選ばれない
///   (人口0の州には負担を配分しない、という要求を数式的に満たす)。
/// - `mobilized_manpower * population_i`は`u128`で計算し、通常の`u64`演算での
///   オーバーフローを避ける(結果は`mobilized_manpower`以下に収まるため`u64`へ
///   安全に戻せる)。
///
/// 通常ケースでは、戻り値の合計は`mobilized_manpower`と正確に一致する。
pub fn apportion_mobilized_manpower(
    mobilized_manpower: u64,
    states: &[(StateId, u64)],
) -> HashMap<StateId, u64> {
    let mut burdens: HashMap<StateId, u64> = HashMap::with_capacity(states.len());
    if states.is_empty() || mobilized_manpower == 0 {
        for &(id, _) in states {
            burdens.insert(id, 0);
        }
        return burdens;
    }

    let population_total: u128 = states.iter().map(|&(_, pop)| pop as u128).sum();
    if population_total == 0 {
        for &(id, _) in states {
            burdens.insert(id, 0);
        }
        return burdens;
    }

    let m = mobilized_manpower as u128;
    let mut remainders: Vec<(StateId, u64, u128)> = Vec::with_capacity(states.len());
    let mut allocated: u128 = 0;
    for &(id, population) in states {
        let product = m * (population as u128);
        let base = product / population_total;
        let remainder = product % population_total;
        burdens.insert(id, base as u64);
        allocated += base;
        remainders.push((id, population, remainder));
    }

    let mut deficit = m - allocated;
    if deficit == 0 {
        return burdens;
    }

    // 剰余降順、同率はStateId昇順(安定した既存の順序基準)でソートする。
    // 人口0の州は剰余が常に0なので、人口>0かつ剰余>0の州が存在する限り選ばれない。
    remainders.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.0.cmp(&b.0.0)));

    for (id, _population, _remainder) in remainders {
        if deficit == 0 {
            break;
        }
        *burdens.get_mut(&id).expect("id was inserted above") += 1;
        deficit -= 1;
    }

    burdens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum_burdens(burdens: &HashMap<StateId, u64>) -> u64 {
        burdens.values().sum()
    }

    #[test]
    fn zero_mobilized_manpower_yields_zero_burden_for_every_state() {
        let states = [(StateId(0), 100_000), (StateId(1), 200_000)];
        let burdens = apportion_mobilized_manpower(0, &states);
        assert_eq!(burdens.get(&StateId(0)).copied(), Some(0));
        assert_eq!(burdens.get(&StateId(1)).copied(), Some(0));
    }

    #[test]
    fn apportions_proportionally_to_population_across_multiple_states() {
        // 人口比1:3、mobilized=4000 -> 1000:3000ちょうど(端数なし)。
        let states = [(StateId(0), 100_000), (StateId(1), 300_000)];
        let burdens = apportion_mobilized_manpower(4_000, &states);
        assert_eq!(burdens.get(&StateId(0)).copied(), Some(1_000));
        assert_eq!(burdens.get(&StateId(1)).copied(), Some(3_000));
    }

    #[test]
    fn state_totals_sum_exactly_to_the_original_mobilized_manpower() {
        let states = [
            (StateId(0), 333_333),
            (StateId(1), 333_333),
            (StateId(2), 333_334),
        ];
        for m in [1u64, 2, 3, 7, 1_000, 999_999] {
            let burdens = apportion_mobilized_manpower(m, &states);
            assert_eq!(
                sum_burdens(&burdens),
                m,
                "apportioned totals must sum exactly to mobilized_manpower={m}"
            );
        }
    }

    #[test]
    fn remainder_deficit_is_distributed_without_losing_a_single_person() {
        // population_total=3、mobilized=10 -> 各州base=3、剰余1ずつ、不足1人を配分。
        let states = [(StateId(0), 1), (StateId(1), 1), (StateId(2), 1)];
        let burdens = apportion_mobilized_manpower(10, &states);
        assert_eq!(sum_burdens(&burdens), 10);
    }

    #[test]
    fn equal_remainders_are_broken_by_ascending_state_id() {
        // 人口が全て等しいので剰余も全て等しい。StateId昇順で1人ずつ配分されることを固定。
        let states = [(StateId(5), 100), (StateId(2), 100), (StateId(8), 100)];
        // population_total=300, mobilized=301 -> base=100ずつ(計300)、残り1人をStateId最小へ。
        let burdens = apportion_mobilized_manpower(301, &states);
        assert_eq!(burdens.get(&StateId(2)).copied(), Some(101));
        assert_eq!(burdens.get(&StateId(5)).copied(), Some(100));
        assert_eq!(burdens.get(&StateId(8)).copied(), Some(100));
    }

    #[test]
    fn a_zero_population_state_never_receives_burden() {
        let states = [(StateId(0), 0), (StateId(1), 100_000)];
        let burdens = apportion_mobilized_manpower(7, &states);
        assert_eq!(burdens.get(&StateId(0)).copied(), Some(0));
        assert_eq!(burdens.get(&StateId(1)).copied(), Some(7));
    }

    #[test]
    fn all_owned_states_with_zero_population_does_not_panic_or_divide_by_zero() {
        let states = [(StateId(0), 0), (StateId(1), 0)];
        let burdens = apportion_mobilized_manpower(5_000, &states);
        assert_eq!(burdens.get(&StateId(0)).copied(), Some(0));
        assert_eq!(burdens.get(&StateId(1)).copied(), Some(0));
    }

    #[test]
    fn no_owned_states_returns_an_empty_map_without_panicking() {
        let burdens = apportion_mobilized_manpower(5_000, &[]);
        assert!(burdens.is_empty());
    }

    #[test]
    fn does_not_leak_burden_to_states_outside_the_provided_slice() {
        // 呼び出し元が「他国の州」を含めずに渡す契約であることを、渡した集合の外に
        // burdenが一切現れないことで確認する。
        let states = [(StateId(0), 100_000)];
        let burdens = apportion_mobilized_manpower(1_000, &states);
        assert_eq!(burdens.len(), 1);
        assert_eq!(burdens.get(&StateId(0)).copied(), Some(1_000));
        assert_eq!(burdens.get(&StateId(99)), None);
    }

    #[test]
    fn extreme_values_do_not_overflow_or_panic() {
        let states = [(StateId(0), u64::MAX / 2), (StateId(1), u64::MAX / 2)];
        let burdens = apportion_mobilized_manpower(u64::MAX, &states);
        assert_eq!(sum_burdens(&burdens), u64::MAX);
    }

    #[test]
    fn insertion_order_does_not_affect_the_result() {
        let forward = [
            (StateId(0), 111_111),
            (StateId(1), 222_222),
            (StateId(2), 333_333),
        ];
        let reversed = [
            (StateId(2), 333_333),
            (StateId(1), 222_222),
            (StateId(0), 111_111),
        ];
        assert_eq!(
            apportion_mobilized_manpower(50_000, &forward),
            apportion_mobilized_manpower(50_000, &reversed),
        );
    }
}
