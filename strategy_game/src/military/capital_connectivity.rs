//! P26-006: 首都連結性による空間補給モデル2(`verification_logs/phase-26/p26-006/`)。
//!
//! P26-001が確定した次のマイルストーンを実装する。
//!
//! ```text
//! 国家首都 → 自国支配下の陸上州 → StateData.neighbors → Division所在州
//! ```
//!
//! 港・海上輸送・鉄道容量・同盟国通行権・携行在庫・補給距離は本モジュールの対象外
//! (spec §0・§22)。本モジュールは「首都から自国が現在実効支配する陸上州だけを
//! たどって到達できる州の集合」を求める純粋関数だけを提供し、日次補給消費への接続は
//! `military::daily_supply`が担う。
//!
//! 連結性はSaveへ保存しない――`capital_state_id`・`StateData.neighbors`・
//! `StateData.controller()`・Division`current_state`のいずれも既存のSave済みデータ
//! から導出できる状態であり、load後に現在状態から再計算する(spec §18)。

use crate::common::{CountryId, StateId};
use crate::state::data::StateRegistry;
use std::collections::{HashSet, VecDeque};

/// 州`state`が国家`country`にとって陸上で通行可能か(spec §5)。
///
/// ```text
/// traversable(c, s) = state exists && !state.is_sea && state.controller() == c
/// ```
///
/// `owner_country_id`(法的所有)ではなく`controller()`(現在の実効支配、占領を含む)を
/// 使う――敵に占領された自国の法的領土は通行不能、逆に自国が占領した敵の法的領土は
/// 通行可能になる(spec §5「ownerとcontrollerの違い」)。
fn is_traversable(state_id: StateId, country: CountryId, state_registry: &StateRegistry) -> bool {
    match state_registry.get(state_id) {
        Some(state) => !state.is_sea && state.controller() == country,
        None => false,
    }
}

/// 国家`country`の首都連結州集合を、首都(`capital`)から`StateData.neighbors`を
/// たどって求める(spec §5〜§8)。
///
/// # 首都自体の条件(spec §7)
///
/// 首都が次をすべて満たす場合のみ探索を開始する。
///
/// ```text
/// capital state exists && !capital.is_sea && capital.controller() == country
/// ```
///
/// 満たさない場合(存在しない首都・海上州を首都として参照・敵に占領された首都・
/// controller不明[=owner]だが実際には別国など)、空集合を返す――このとき正の需要を
/// 持つ全Divisionは呼び出し元(`daily_supply`)によって補給率0として扱われる。
///
/// # 決定性(spec §8)
///
/// * 開始点は`capital`固定。
/// * 各州の`neighbors`は展開前に`StateId`昇順へ明示的にソートする
///   (`states.ron`のファイル順・`Vec`挿入順・HashMap反復順のいずれにも依存しない)。
/// * 訪問済み判定は`StateId`をキーにした`HashSet`で行う(Bevy Entity順・
///   query反復順を一切参照しない)。
/// * cycle・self-loop・重複neighbor・存在しないneighborのいずれでも無限ループ・
///   panicをしない(`visited`チェックと`state_registry.get`の`Option`処理で
///   構造的に保証される)。
/// * `neighbors`は片方向(有向)辺として扱う――州`a`が`b`を`neighbors`に含んでいても、
///   `b`が`a`を含み返すとは限らない。探索は各州が実際に列挙する`neighbors`だけを
///   辿る(逆方向を推測して補わない)。実データ(`assets/data/states.ron`)は全ての
///   既知の隣接関係が双方向対称だが、コードはこの対称性を前提にしない。
/// * 到達可能集合(最終的な`HashSet`の中身)は探索順序(挿入順・neighbor列挙順)に
///   一切依存しない――BFS/DFSいずれで実装しても同じ集合になる数学的性質を持つ
///   (spec §8「到達可能集合だけが必要なため、探索順が最終集合を変えないこと」)。
///
/// 世界の州数を`N`とすると、各州は高々1回だけキューへ積まれるため、計算量は
/// `O(N + E)`(`E`は総neighbor辺数)であり、州数に対して線形。
pub fn connected_states_from_capital(
    capital: StateId,
    country: CountryId,
    state_registry: &StateRegistry,
) -> HashSet<StateId> {
    let mut visited: HashSet<StateId> = HashSet::new();

    if !is_traversable(capital, country, state_registry) {
        return visited;
    }

    visited.insert(capital);
    let mut queue: VecDeque<StateId> = VecDeque::new();
    queue.push_back(capital);

    while let Some(current) = queue.pop_front() {
        let Some(state) = state_registry.get(current) else {
            // 到達不能のはず(visitedに入る州は必ずis_traversableを満たし、
            // is_traversableはstate_registry.get(current).is_some()を要求する)。
            // それでも将来の変更に対して安全側(panicしない)に倒す。
            continue;
        };
        let mut neighbors: Vec<StateId> = state.neighbors.clone();
        neighbors.sort_by_key(|s| s.0);
        for neighbor in neighbors {
            if visited.contains(&neighbor) {
                continue; // 重複neighbor・cycle・self-loopのいずれもここで安全に無視される
            }
            if is_traversable(neighbor, country, state_registry) {
                visited.insert(neighbor);
                queue.push_back(neighbor);
            }
        }
    }

    visited
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::data::StateData;

    fn state(id: usize, owner: CountryId, neighbors: &[usize]) -> StateData {
        StateData {
            id: StateId(id),
            owner_country_id: owner,
            neighbors: neighbors.iter().map(|&n| StateId(n)).collect(),
            ..Default::default()
        }
    }

    fn sea_state(id: usize, owner: CountryId, neighbors: &[usize]) -> StateData {
        StateData { is_sea: true, ..state(id, owner, neighbors) }
    }

    fn occupied(mut s: StateData, controller: CountryId) -> StateData {
        s.controller_country = Some(controller);
        s
    }

    const US: CountryId = CountryId(1);
    const THEM: CountryId = CountryId(2);
    const ALLY: CountryId = CountryId(3);

    // ─── Category A: グラフ探索 ────────────────────────────────────────────

    #[test]
    fn a1_capital_alone_with_no_neighbors_is_connected_to_itself_only() {
        let reg = StateRegistry::build(vec![state(0, US, &[])]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert_eq!(connected, [StateId(0)].into_iter().collect());
    }

    #[test]
    fn a2_straight_line_of_own_states_is_fully_connected() {
        let reg = StateRegistry::build(vec![
            state(0, US, &[1]),
            state(1, US, &[0, 2]),
            state(2, US, &[1, 3]),
            state(3, US, &[2]),
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert_eq!(connected, [StateId(0), StateId(1), StateId(2), StateId(3)].into_iter().collect());
    }

    #[test]
    fn a3_branching_graph_reaches_every_own_branch() {
        let reg = StateRegistry::build(vec![
            state(0, US, &[1, 2]),
            state(1, US, &[0, 3]),
            state(2, US, &[0, 4]),
            state(3, US, &[1]),
            state(4, US, &[2]),
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert_eq!(connected.len(), 5);
    }

    #[test]
    fn a4_cycle_does_not_infinite_loop_and_includes_all_members() {
        let reg = StateRegistry::build(vec![
            state(0, US, &[1]),
            state(1, US, &[2]),
            state(2, US, &[0]), // 0へ戻るcycle
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert_eq!(connected, [StateId(0), StateId(1), StateId(2)].into_iter().collect());
    }

    #[test]
    fn a5_self_loop_does_not_infinite_loop() {
        let reg = StateRegistry::build(vec![state(0, US, &[0, 1]), state(1, US, &[0])]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert_eq!(connected, [StateId(0), StateId(1)].into_iter().collect());
    }

    #[test]
    fn a6_duplicate_neighbor_entries_are_processed_once() {
        let reg = StateRegistry::build(vec![state(0, US, &[1, 1, 1]), state(1, US, &[0])]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert_eq!(connected, [StateId(0), StateId(1)].into_iter().collect());
    }

    #[test]
    fn a7_disconnected_component_is_not_reached() {
        let reg = StateRegistry::build(vec![
            state(0, US, &[1]),
            state(1, US, &[0]),
            // 2/3は同じ国が所有するが、0/1のどちらとも隣接していない別コンポーネント
            state(2, US, &[3]),
            state(3, US, &[2]),
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert_eq!(connected, [StateId(0), StateId(1)].into_iter().collect());
    }

    #[test]
    fn a8_neighbor_list_insertion_order_does_not_change_the_final_set() {
        let reg_a = StateRegistry::build(vec![
            state(0, US, &[1, 2, 3]),
            state(1, US, &[0]),
            state(2, US, &[0]),
            state(3, US, &[0]),
        ]);
        let reg_b = StateRegistry::build(vec![
            state(0, US, &[3, 1, 2]),
            state(1, US, &[0]),
            state(2, US, &[0]),
            state(3, US, &[0]),
        ]);
        assert_eq!(
            connected_states_from_capital(StateId(0), US, &reg_a),
            connected_states_from_capital(StateId(0), US, &reg_b)
        );
    }

    #[test]
    fn a9_state_vec_insertion_order_does_not_change_the_final_set() {
        let reg_a = StateRegistry::build(vec![
            state(0, US, &[1, 2]),
            state(1, US, &[0]),
            state(2, US, &[0]),
        ]);
        let reg_b = StateRegistry::build(vec![
            state(2, US, &[0]),
            state(0, US, &[1, 2]),
            state(1, US, &[0]),
        ]);
        assert_eq!(
            connected_states_from_capital(StateId(0), US, &reg_a),
            connected_states_from_capital(StateId(0), US, &reg_b)
        );
    }

    #[test]
    fn a10_nonexistent_neighbor_id_does_not_panic() {
        let reg = StateRegistry::build(vec![state(0, US, &[1, 999])]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert_eq!(connected, [StateId(0)].into_iter().collect());
    }

    #[test]
    fn a11_one_directional_neighbor_only_traverses_the_declared_direction() {
        // 0は1をneighborsに含むが、1は0を含み返さない(片方向)。
        let reg = StateRegistry::build(vec![state(0, US, &[1]), state(1, US, &[])]);
        let from_0 = connected_states_from_capital(StateId(0), US, &reg);
        assert_eq!(from_0, [StateId(0), StateId(1)].into_iter().collect());
        // 逆方向(1が首都)からは0へ到達できない。
        let from_1 = connected_states_from_capital(StateId(1), US, &reg);
        assert_eq!(from_1, [StateId(1)].into_iter().collect());
    }

    // ─── Category B: 通行条件 ──────────────────────────────────────────────

    #[test]
    fn b1_own_controlled_state_is_traversable() {
        let reg = StateRegistry::build(vec![state(0, US, &[1]), state(1, US, &[0])]);
        assert!(connected_states_from_capital(StateId(0), US, &reg).contains(&StateId(1)));
    }

    #[test]
    fn b2_own_owned_but_enemy_controlled_state_is_not_traversable() {
        let reg = StateRegistry::build(vec![
            state(0, US, &[1]),
            occupied(state(1, US, &[0]), THEM), // 自国の法的領土だが敵が占領中
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert!(!connected.contains(&StateId(1)), "owner==selfだけでは通行可能にしてはいけない");
    }

    #[test]
    fn b3_enemy_owned_but_self_controlled_state_is_traversable() {
        let reg = StateRegistry::build(vec![
            state(0, US, &[1]),
            occupied(state(1, THEM, &[0]), US), // 敵の法的領土だが自国が占領中
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert!(connected.contains(&StateId(1)), "自国が実効支配していれば法的所有者に関わらず通行可能なはず");
    }

    #[test]
    fn b4_ally_controlled_state_is_not_traversable() {
        let reg = StateRegistry::build(vec![
            state(0, US, &[1]),
            occupied(state(1, US, &[0]), ALLY),
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert!(!connected.contains(&StateId(1)), "同盟国支配州は本タスクでは通行不能のはず(軍事通行権は対象外)");
    }

    #[test]
    fn b5_neutral_controlled_state_is_not_traversable() {
        const NEUTRAL: CountryId = CountryId(9);
        let reg = StateRegistry::build(vec![
            state(0, US, &[1]),
            occupied(state(1, US, &[0]), NEUTRAL),
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert!(!connected.contains(&StateId(1)));
    }

    #[test]
    fn b6_sea_state_is_never_traversable_even_if_self_controlled() {
        let reg = StateRegistry::build(vec![
            state(0, US, &[1]),
            sea_state(1, US, &[0, 2]),
            state(2, US, &[1]),
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert!(!connected.contains(&StateId(1)), "海上州は自国支配でも通行不能");
        assert!(!connected.contains(&StateId(2)), "海上州の先は本タスクでは到達不能");
    }

    #[test]
    fn b7_unknown_state_reference_is_not_traversable() {
        let reg = StateRegistry::build(vec![state(0, US, &[1])]); // 1は登録されていない
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert_eq!(connected, [StateId(0)].into_iter().collect());
    }

    #[test]
    fn b8_enemy_controlled_corridor_state_cuts_off_the_frontier_state_behind_it() {
        // 首都0 ─ 自国州1 ─ 自国州2(敵支配) ─ 前線州3(自国支配)
        let reg = StateRegistry::build(vec![
            state(0, US, &[1]),
            state(1, US, &[0, 2]),
            occupied(state(2, US, &[1, 3]), THEM),
            state(3, US, &[2]),
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert_eq!(connected, [StateId(0), StateId(1)].into_iter().collect());
        assert!(!connected.contains(&StateId(3)), "回廊(州2)が敵支配になると、その先の自国州3は切断されるはず");
    }

    #[test]
    fn b9_recapturing_the_corridor_state_reconnects_the_frontier_state() {
        let reg = StateRegistry::build(vec![
            state(0, US, &[1]),
            state(1, US, &[0, 2]),
            state(2, US, &[1, 3]), // 奪還済み(controller_country=None=owner=US)
            state(3, US, &[2]),
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert!(connected.contains(&StateId(3)), "回廊を奪還すれば前線州は再接続するはず");
    }

    // ─── Category C: 首都 ──────────────────────────────────────────────────

    #[test]
    fn c1_normal_self_controlled_capital_starts_a_normal_search() {
        let reg = StateRegistry::build(vec![state(0, US, &[1]), state(1, US, &[0])]);
        assert_eq!(connected_states_from_capital(StateId(0), US, &reg).len(), 2);
    }

    #[test]
    fn c2_capital_occupied_by_enemy_yields_empty_connectivity() {
        let reg = StateRegistry::build(vec![
            occupied(state(0, US, &[1]), THEM),
            state(1, US, &[0]),
        ]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert!(connected.is_empty(), "首都が敵占領下なら連結州集合は空のはず");
    }

    #[test]
    fn c3_nonexistent_capital_state_yields_empty_connectivity() {
        let reg = StateRegistry::build(vec![state(1, US, &[])]); // 首都として参照する0は存在しない
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert!(connected.is_empty());
    }

    #[test]
    fn c4_sea_state_referenced_as_capital_yields_empty_connectivity() {
        let reg = StateRegistry::build(vec![sea_state(0, US, &[1]), state(1, US, &[0])]);
        let connected = connected_states_from_capital(StateId(0), US, &reg);
        assert!(connected.is_empty(), "首都が海上州として参照されている場合は空のはず");
    }

    #[test]
    fn c5_capital_controlled_by_a_third_country_is_not_traversable_for_the_asking_country() {
        // c2と同じ状況を、消滅国家が最後の首都をそのまま持ち続けているケースの
        // 代替として扱う(country::mod.rs::current_capital_state_id()が呼び出し元で
        // 消滅国家を除外する契約になっているため、本モジュール自体はcontrollerの
        // 一致だけを見る)。
        let reg = StateRegistry::build(vec![occupied(state(0, US, &[]), THEM)]);
        assert!(connected_states_from_capital(StateId(0), US, &reg).is_empty());
    }
}
