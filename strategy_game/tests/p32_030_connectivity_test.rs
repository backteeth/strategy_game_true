//! P32-030 (M5-a): プロヴィンスの陸上隣接グラフ(`world::connectivity`)と、明示的な通行条件による純粋な到達判定の検証。
//!
//! 小さな人工の地図(軸座標で手計算した期待値)と、実地図の検証用ワールド(29プロヴィンス)を使う。人工の期待値は、
//! 実装の探索処理ではなく、**手で列挙した隣接・成分**から作る(隣接の規則: (q,r) の6近傍は (q+1,r) (q+1,r-1) (q,r-1) (q-1,r) (q-1,r+1) (q,r+1))。
//! 実データの期待値は、テスト内の独立した総当たり(別の隣接計算)でも確認する。時間は合否条件にしない。保存・アセットは書かない。

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::json;
use strategy_game::hexmap::data::{HexMap, load_from_path, parse_hex_map};
use strategy_game::world::fingerprint::FINGERPRINT_ALGORITHM;
use strategy_game::world::*;

// ---------------------------------------------------------------- 人工地図の部品

/// 行 = r、列 = q。`.` 海、`L` 湖、`#` 平原(陸)。
fn map_from(rows: &[&str], rivers: &[(i32, i32, u8)]) -> HexMap {
    let (h, w) = (rows.len(), rows[0].len());
    let mut cells = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (q, ch) in row.chars().enumerate() {
            let (terrain, climate, veg) = match ch {
                '.' => ("sea", "unclassified", "unclassified"),
                'L' => ("lake", "unclassified", "unclassified"),
                _ => ("plains", "C", "grassland"),
            };
            cells.push(json!({ "q": q, "r": r, "terrain": terrain, "climate": climate, "vegetation": veg }));
        }
    }
    let rv: Vec<_> = rivers.iter().map(|(q, r, d)| json!({ "q": q, "r": r, "direction": d })).collect();
    let v = json!({
        "schemaVersion": 9,
        "kind": "hex-atlas-game-map",
        "grid": {
            "orientation": "pointy", "size": 2.5, "origin": { "x": 2.1650635094610964, "y": 2.5 },
            "imageWidth": 100, "imageHeight": 60,
            "extent": { "minQ": 0, "maxQ": w - 1, "minR": 0, "maxR": h - 1 }
        },
        "cells": cells,
        "rivers": rv,
    });
    parse_hex_map(v.to_string().as_bytes()).expect("synthetic map must parse")
}

fn table(map: &HexMap, provinces: &[(u32, i32, i32)], coverage: Coverage) -> ProvinceTableFile {
    let mut geography = compute_fingerprint(map);
    geography.algorithm = FINGERPRINT_ALGORITHM;
    ProvinceTableFile {
        format_version: WORLD_FORMAT_VERSION,
        coverage,
        geography,
        next_province_id: provinces.iter().map(|p| p.0).max().unwrap_or(0) + 1,
        retired: Vec::new(),
        provinces: provinces.iter().map(|&(id, q, r)| ProvinceEntry { id, q, r }).collect(),
    }
}

fn layout(states: &[(u32, &[u32])]) -> StateLayoutFile {
    StateLayoutFile {
        format_version: WORLD_FORMAT_VERSION,
        states: states
            .iter()
            .map(|(id, ps)| StateEntry {
                id: *id,
                name_key: format!("test.{id}"),
                provinces: ps.to_vec(),
                representative: None,
                allow_disconnected: true,
            })
            .collect(),
    }
}

fn world(map: &HexMap, provinces: &[(u32, i32, i32)], states: &[(u32, &[u32])], coverage: Coverage) -> WorldModel {
    WorldModel::build(table(map, provinces, coverage), layout(states)).expect("test world must build")
}

fn graph_of(map: &HexMap, provinces: &[(u32, i32, i32)], states: &[(u32, &[u32])], coverage: Coverage) -> ProvinceGraph {
    ProvinceGraph::build(&world(map, provinces, states, coverage), map).expect("graph must build")
}

fn p(id: u32) -> ProvinceId {
    ProvinceId(id)
}

fn pids(v: &[u32]) -> Vec<ProvinceId> {
    v.iter().map(|x| ProvinceId(*x)).collect()
}

fn edges_of(g: &ProvinceGraph) -> BTreeSet<(u32, u32)> {
    let mut out = BTreeSet::new();
    for a in g.node_ids() {
        for (b, _) in g.neighbors(a).unwrap() {
            if a < b {
                out.insert((a.0, b.0));
            }
        }
    }
    out
}

fn passable(g: &ProvinceGraph, ids: &[u32]) -> ExplicitPassable {
    ExplicitPassable::new(g, ids.iter().map(|x| ProvinceId(*x))).expect("known provinces")
}

fn reached(r: &Reach) -> Vec<u32> {
    r.reached_ids().iter().map(|x| x.0).collect()
}

// ---------------------------------------------------------------- グラフ(人工の期待値)

#[test]
fn a_single_node_has_no_edges_and_one_component() {
    let map = map_from(&["#"], &[]);
    let g = graph_of(&map, &[(7, 0, 0)], &[], Coverage::World);
    assert_eq!((g.node_count(), g.edge_count()), (1, 0));
    assert_eq!(g.components(), vec![pids(&[7])]);
    assert!(g.scope().is_complete());
    let r = reach(&g, &[p(7)], &passable(&g, &[7])).unwrap();
    assert_eq!(reached(&r), vec![7], "a passable source reaches itself");
    assert_eq!(r.reachability(p(7)), Reachability::Reachable);
}

#[test]
fn adjacent_and_non_adjacent_cells_in_a_row() {
    // 1行に4陸: q=0..3。IDは座標の順と無関係(40,10,30,20)。隣接は q が1違うものだけ。
    let map = map_from(&["####"], &[]);
    let g = graph_of(&map, &[(40, 0, 0), (10, 1, 0), (30, 2, 0), (20, 3, 0)], &[], Coverage::World);
    // 期待: (40-10) (10-30) (30-20) → ID順の組で (10,40) (10,30) (20,30)
    let expected: BTreeSet<(u32, u32)> = [(10, 40), (10, 30), (20, 30)].into_iter().collect();
    assert_eq!(edges_of(&g), expected);
    assert_eq!(g.edge_count(), 3);
    assert!(g.edge_between(p(40), p(30)).is_none(), "two cells apart are not adjacent");
    assert!(g.edge_between(p(40), p(10)).is_some());
    assert_eq!(g.components().len(), 1);
    // 隣接リストは ProvinceId 昇順
    let n: Vec<u32> = g.neighbors(p(10)).unwrap().iter().map(|(x, _)| x.0).collect();
    assert_eq!(n, vec![30, 40]);
}

#[test]
fn all_six_directions_of_a_hex_are_edges_and_the_graph_is_bidirectional() {
    // 中心(1,1)と、6近傍 (2,1) (2,0) (1,0) (0,1) (0,2) (1,2) を全部陸にする(3x3の地図)。(0,0)と(2,2)は海。
    let map = map_from(&[".##", "###", "##."], &[]);
    let cells = [(1, 1), (2, 1), (2, 0), (1, 0), (0, 1), (0, 2), (1, 2)];
    let defs: Vec<(u32, i32, i32)> = cells.iter().enumerate().map(|(i, &(q, r))| (i as u32 + 1, q, r)).collect();
    let g = graph_of(&map, &defs, &[], Coverage::World);
    // 中心(ID1)は6近傍すべてと隣接する。
    let center: Vec<u32> = g.neighbors(p(1)).unwrap().iter().map(|(x, _)| x.0).collect();
    assert_eq!(center, vec![2, 3, 4, 5, 6, 7]);
    // 周囲の隣り合う2つも隣接(輪になる): (2,1)-(2,0) (2,0)-(1,0) (1,0)-(0,1) (0,1)-(0,2) (0,2)-(1,2) (1,2)-(2,1)
    let ring: BTreeSet<(u32, u32)> = [(2, 3), (3, 4), (4, 5), (5, 6), (6, 7), (2, 7)].into_iter().collect();
    let expected: BTreeSet<(u32, u32)> = (2..=7).map(|x| (1, x)).chain(ring).collect();
    assert_eq!(edges_of(&g), expected);
    // 双方向: a→b があれば b→a がある(辺の事実も一致)
    for a in g.node_ids() {
        for (b, e) in g.neighbors(a).unwrap() {
            assert_eq!(g.edge_between(b, a), Some(e));
            assert_ne!(a, b, "no self edge");
        }
    }
}

#[test]
fn a_two_by_two_block_has_five_edges_and_no_diagonal_through_the_missing_pair() {
    // (0,0) (1,0) (0,1) (1,1): (0,0)-(1,1) は隣接しない。他の5組は隣接する。
    let map = map_from(&["##", "##"], &[]);
    let g = graph_of(&map, &[(1, 0, 0), (2, 1, 0), (3, 0, 1), (4, 1, 1)], &[], Coverage::World);
    let expected: BTreeSet<(u32, u32)> = [(1, 2), (1, 3), (2, 3), (2, 4), (3, 4)].into_iter().collect();
    assert_eq!(edges_of(&g), expected);
}

#[test]
fn water_never_bridges_two_land_cells() {
    // 海・湖をはさんだ陸は隣接しない。水域にはノードも辺も無い。
    for mid in ['.', 'L'] {
        let row = format!("#{mid}#");
        let map = map_from(&[row.as_str()], &[]);
        let g = graph_of(&map, &[(1, 0, 0), (2, 2, 0)], &[], Coverage::World);
        assert_eq!((g.node_count(), g.edge_count()), (2, 0), "mid={mid}");
        assert_eq!(g.components().len(), 2);
        let r = reach(&g, &[p(1)], &ExplicitPassable::all_nodes(&g)).unwrap();
        assert_eq!(reached(&r), vec![1]);
        assert_eq!(r.reachability(p(2)), Reachability::Unreachable(UnreachableReason::NoRoute));
    }
}

#[test]
fn contact_across_states_is_an_edge_and_is_counted_as_cross_state() {
    let map = map_from(&["####"], &[]);
    let g = graph_of(
        &map,
        &[(1, 0, 0), (2, 1, 0), (3, 2, 0), (4, 3, 0)],
        &[(10, &[1, 2]), (20, &[3])],
        Coverage::Partial,
    );
    assert_eq!(g.edge_count(), 3);
    assert_eq!(g.cross_state_edge_count(), 1, "only 2-3 joins two different assigned states");
    assert_eq!(g.state_of(p(4)), None, "province 4 is in the table but unassigned (still a node)");
    assert_eq!(g.scope().unassigned_nodes, 1);
    // 未割当ノード(4)は辺を持つが、ステート間の辺には数えない(片側が未割当)
    assert!(g.edge_between(p(3), p(4)).is_some());
}

#[test]
fn rivers_are_recorded_on_edges_but_never_block_a_route() {
    // (0,0)-(1,0) の辺(direction 0 = 東)に河川。 (1,0)-(2,0) には無い。
    let map = map_from(&["###"], &[(0, 0, 0)]);
    let g = graph_of(&map, &[(1, 0, 0), (2, 1, 0), (3, 2, 0)], &[], Coverage::World);
    assert_eq!(g.edge_between(p(1), p(2)), Some(EdgeInfo { contacts: 1, river_contacts: 1 }));
    assert_eq!(g.edge_between(p(2), p(3)), Some(EdgeInfo { contacts: 1, river_contacts: 0 }));
    assert_eq!(g.river_edge_count(), 1);
    let r = reach(&g, &[p(1)], &ExplicitPassable::all_nodes(&g)).unwrap();
    assert_eq!(reached(&r), vec![1, 2, 3], "a river does not stop the route");
}

#[test]
fn a_river_recorded_from_the_other_side_is_the_same_edge() {
    // 同じ辺を、反対側のセルから(direction 3 = 西)記録しても、同じ1本の辺として河川が付く。
    let map = map_from(&["##"], &[(1, 0, 3)]);
    let g = graph_of(&map, &[(1, 0, 0), (2, 1, 0)], &[], Coverage::World);
    assert_eq!(g.edge_between(p(1), p(2)), Some(EdgeInfo { contacts: 1, river_contacts: 1 }));
}

// ---------------------------------------------------------------- 複数セルのプロヴィンス(検証済みの世界定義の外側の入力)

/// (ID, 所属ステート, セル)。
type DefSpec<'a> = (u32, Option<u32>, &'a [(i32, i32)]);

fn defs(v: &[DefSpec]) -> Vec<ProvinceCells> {
    v.iter()
        .map(|(id, st, cells)| ProvinceCells {
            id: ProvinceId(*id),
            state: st.map(MapStateId),
            cells: cells.iter().map(|&(q, r)| HexCoord::new(q, r)).collect(),
        })
        .collect()
}

#[test]
fn multi_cell_provinces_get_one_edge_per_pair_with_every_contact_counted() {
    // A = {(0,0),(1,0)}、B = {(0,1),(1,1)}。接触: (0,0)-(0,1) (1,0)-(1,1) (1,0)-(0,1) の3か所 = 辺は1本で contacts=3。(0,0)-(1,1)は隣接しない。
    let map = map_from(&["##", "##"], &[]);
    let d = defs(&[(1, None, &[(0, 0), (1, 0)]), (2, None, &[(0, 1), (1, 1)])]);
    let g = ProvinceGraph::from_definitions(&d, &map, Coverage::World).unwrap();
    assert_eq!((g.node_count(), g.edge_count()), (2, 1));
    assert_eq!(g.edge_between(p(1), p(2)), Some(EdgeInfo { contacts: 3, river_contacts: 0 }));
    assert!(g.neighbors(p(1)).unwrap().iter().all(|(x, _)| *x != p(1)), "no self edge from touching cells of the same province");
}

#[test]
fn a_multi_cell_province_connects_to_each_neighbour_through_any_of_its_cells() {
    // A = {(0,0),(1,0),(2,0)} が、B=(0,1) と C=(2,1) の両方に隣接する(それぞれ別のセルで接する)。B と C は隣接しない。
    let map = map_from(&["###", "#.#"], &[]);
    let d = defs(&[(1, None, &[(2, 0), (0, 0), (1, 0)]), (2, None, &[(0, 1)]), (3, None, &[(2, 1)])]);
    let g = ProvinceGraph::from_definitions(&d, &map, Coverage::World).unwrap();
    let expected: BTreeSet<(u32, u32)> = [(1, 2), (1, 3)].into_iter().collect();
    assert_eq!(edges_of(&g), expected);
    let r = reach(&g, &[p(2)], &ExplicitPassable::all_nodes(&g)).unwrap();
    assert_eq!(reached(&r), vec![1, 2, 3], "B reaches C by passing through A");
}

#[test]
fn definitions_that_would_teleport_inside_a_province_are_rejected() {
    let map = map_from(&["###", "###"], &[]);
    // 隣接でつながっていない2セル(間のセル(1,0)は別のプロヴィンス)。
    let d = defs(&[(1, None, &[(0, 0), (2, 0)]), (2, None, &[(1, 0)])]);
    assert_eq!(ProvinceGraph::from_definitions(&d, &map, Coverage::Partial), Err(GraphError::ProvinceNotContiguous(p(1))));
    // セルを2つのプロヴィンスが持つ
    let d = defs(&[(1, None, &[(0, 0), (1, 0)]), (2, None, &[(1, 0)])]);
    assert_eq!(
        ProvinceGraph::from_definitions(&d, &map, Coverage::Partial),
        Err(GraphError::CellSharedByProvinces { cell: HexCoord::new(1, 0), a: p(1), b: p(2) })
    );
    // 同じ ID の二重定義
    let d = defs(&[(1, None, &[(0, 0)]), (1, None, &[(1, 0)])]);
    assert_eq!(ProvinceGraph::from_definitions(&d, &map, Coverage::Partial), Err(GraphError::DuplicateProvince(p(1))));
    // セルなし、同じセルの重複
    assert_eq!(ProvinceGraph::from_definitions(&defs(&[(1, None, &[])]), &map, Coverage::Partial), Err(GraphError::ProvinceWithoutCells(p(1))));
    assert_eq!(
        ProvinceGraph::from_definitions(&defs(&[(1, None, &[(0, 0), (0, 0)])]), &map, Coverage::Partial),
        Err(GraphError::DuplicateCellInProvince { province: p(1), cell: HexCoord::new(0, 0) })
    );
    // 地図外のセル、水域のセル
    assert_eq!(
        ProvinceGraph::from_definitions(&defs(&[(1, None, &[(9, 9)])]), &map, Coverage::Partial),
        Err(GraphError::CellOutsideMap { province: p(1), cell: HexCoord::new(9, 9) })
    );
    let wmap = map_from(&["#.#"], &[]);
    assert_eq!(
        ProvinceGraph::from_definitions(&defs(&[(1, None, &[(1, 0)])]), &wmap, Coverage::Partial),
        Err(GraphError::CellIsWater { province: p(1), cell: HexCoord::new(1, 0) })
    );
    // 水域をはさんだ2セルを1つのプロヴィンスにはできない(水域経由の瞬間移動の禁止)
    assert_eq!(
        ProvinceGraph::from_definitions(&defs(&[(1, None, &[(0, 0), (2, 0)])]), &wmap, Coverage::Partial),
        Err(GraphError::ProvinceNotContiguous(p(1)))
    );
}

#[test]
fn an_invalid_world_definition_does_not_produce_a_partial_graph() {
    // プロヴィンス表の指紋(地理)が地図と合わない(別の地図で作った表)→ 検証エラー → グラフを作らない
    let map_a = map_from(&["##"], &[]);
    let map_b = map_from(&["#."], &[]);
    let w = world(&map_a, &[(1, 0, 0), (2, 1, 0)], &[], Coverage::World);
    match ProvinceGraph::build(&w, &map_b) {
        Err(GraphError::InvalidWorld(msgs)) => assert!(!msgs.is_empty()),
        other => panic!("expected InvalidWorld, got {other:?}"),
    }
}

// ---------------------------------------------------------------- 入力順に依存しない

#[test]
fn the_graph_does_not_depend_on_the_order_of_the_input() {
    let map = map_from(&["###", "###"], &[]);
    let a = defs(&[(5, Some(1), &[(0, 0), (1, 0)]), (3, Some(2), &[(2, 0)]), (9, None, &[(0, 1)]), (1, Some(2), &[(1, 1), (2, 1)])]);
    let mut b = a.clone();
    b.reverse();
    for d in &mut b {
        d.cells.reverse();
    }
    let ga = ProvinceGraph::from_definitions(&a, &map, Coverage::Partial).unwrap();
    let gb = ProvinceGraph::from_definitions(&b, &map, Coverage::Partial).unwrap();
    assert_eq!(ga, gb, "same content gives an equal graph (order, cell order)");
    assert_eq!(edges_of(&ga), edges_of(&gb));
    // 世界定義(WorldModel)経由でも、プロヴィンス表の並びを入れ替えても同じ
    let p1 = [(2, 0, 0), (1, 1, 0), (3, 2, 0)];
    let p2 = [(3, 2, 0), (2, 0, 0), (1, 1, 0)];
    let m1 = map_from(&["###"], &[]);
    assert_eq!(graph_of(&m1, &p1, &[], Coverage::World), graph_of(&m1, &p2, &[], Coverage::World));
}

#[test]
fn reach_does_not_depend_on_the_order_of_sources_or_passable_ids_and_is_repeatable() {
    let map = map_from(&["######"], &[]);
    let ids: Vec<(u32, i32, i32)> = (0..6).map(|q| (q as u32 + 1, q, 0)).collect();
    let g = graph_of(&map, &ids, &[], Coverage::World);
    let pass_a = ExplicitPassable::new(&g, pids(&[1, 2, 3, 5, 6])).unwrap();
    let pass_b = ExplicitPassable::new(&g, pids(&[6, 5, 3, 2, 1, 1])).unwrap();
    let r1 = reach(&g, &pids(&[1, 6]), &pass_a).unwrap();
    let r2 = reach(&g, &pids(&[6, 1, 6, 1]), &pass_b).unwrap();
    assert_eq!(r1, r2);
    assert_eq!(r1.sources(), pids(&[1, 6]).as_slice());
    assert_eq!(reached(&r1), vec![1, 2, 3, 5, 6]);
    for _ in 0..5 {
        assert_eq!(reach(&g, &pids(&[1, 6]), &pass_a).unwrap(), r1);
    }
}

// ---------------------------------------------------------------- 到達判定

fn line6() -> (HexMap, ProvinceGraph) {
    let map = map_from(&["######"], &[]);
    let ids: Vec<(u32, i32, i32)> = (0..6).map(|q| (q as u32 + 1, q, 0)).collect();
    let g = graph_of(&map, &ids, &[(10, &[1, 2, 3]), (20, &[4, 5, 6])], Coverage::World);
    (map, g)
}

#[test]
fn same_adjacent_and_multi_province_routes() {
    let (_, g) = line6();
    let all = ExplicitPassable::all_nodes(&g);
    let r = reach(&g, &[p(1)], &all).unwrap();
    assert_eq!(r.reachability(p(1)), Reachability::Reachable, "same province");
    assert_eq!(r.reachability(p(2)), Reachability::Reachable, "adjacent");
    assert_eq!(r.reachability(p(6)), Reachability::Reachable, "five hops, across the state boundary (3-4)");
    assert_eq!(reached(&r), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(r.reached_count(), 6);
    assert_eq!(r.unknown_count(), 0);
    assert!(!r.touches_out_of_scope());
}

#[test]
fn a_blocked_province_in_the_middle_splits_the_route_and_is_not_entered() {
    let (_, g) = line6();
    let pass = ExplicitPassable::all_nodes(&g).without(&passable(&g, &[3]));
    let r = reach(&g, &[p(1)], &pass).unwrap();
    assert_eq!(reached(&r), vec![1, 2]);
    assert_eq!(r.reachability(p(3)), Reachability::Unreachable(UnreachableReason::NotPassable), "the blocked province itself");
    assert_eq!(r.reachability(p(4)), Reachability::Unreachable(UnreachableReason::NoRoute), "passable but cut off, and the graph is complete");
    assert_eq!(r.reachability(p(6)), Reachability::Unreachable(UnreachableReason::NoRoute));
    // 分断された2成分(通行可能部分)
    let comps = g.components_among(&pass);
    assert_eq!(comps, vec![pids(&[1, 2]), pids(&[4, 5, 6])]);
    // 複数の起点: 両側の起点なら、両方の成分に届く
    let both = reach(&g, &[p(6), p(1)], &pass).unwrap();
    assert_eq!(reached(&both), vec![1, 2, 4, 5, 6]);
}

#[test]
fn changing_the_passable_condition_changes_the_result_with_no_stale_value() {
    let (_, g) = line6();
    let open = ExplicitPassable::all_nodes(&g);
    let closed = open.without(&passable(&g, &[4]));
    let a = reach(&g, &[p(1)], &open).unwrap();
    let b = reach(&g, &[p(1)], &closed).unwrap();
    let c = reach(&g, &[p(1)], &open).unwrap();
    assert_eq!(reached(&a), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(reached(&b), vec![1, 2, 3]);
    assert_eq!(a, c, "the first result is not changed by the second query");
    assert_ne!(a, b);
}

#[test]
fn the_state_membership_is_not_a_passability_rule() {
    // 状態(10, 20)は通行の可否に使われない: ステート20の全プロヴィンスを通行条件に載せなければ通れない。
    let (_, g) = line6();
    let only_10 = ExplicitPassable::from_states(&g, &[MapStateId(10)]).unwrap();
    assert_eq!(only_10.ids().collect::<Vec<_>>(), pids(&[1, 2, 3]));
    let r = reach(&g, &[p(1)], &only_10).unwrap();
    assert_eq!(reached(&r), vec![1, 2, 3]);
    assert_eq!(r.reachability(p(4)), Reachability::Unreachable(UnreachableReason::NotPassable));
    assert_eq!(ExplicitPassable::from_states(&g, &[MapStateId(99)]), Err(vec![MapStateId(99)]), "an unknown state is an input error");
}

#[test]
fn invalid_sources_and_invalid_passable_inputs_are_errors_not_unreachable() {
    let (_, g) = line6();
    let all = ExplicitPassable::all_nodes(&g);
    assert_eq!(reach(&g, &[], &all), Err(ReachError::NoSources));
    assert_eq!(
        reach(&g, &[p(99)], &all),
        Err(ReachError::InvalidSources(vec![(p(99), SourceProblem::UnknownProvince)]))
    );
    // 通行不可の起点を、黙って通行可能にしない
    let no_two = all.without(&passable(&g, &[2]));
    assert_eq!(
        reach(&g, &[p(2)], &no_two),
        Err(ReachError::InvalidSources(vec![(p(2), SourceProblem::NotPassable)]))
    );
    // 問題のある起点がすべて報告される(ID順)。1つでも不正なら結果は返さない
    assert_eq!(
        reach(&g, &[p(2), p(99), p(1)], &no_two),
        Err(ReachError::InvalidSources(vec![(p(2), SourceProblem::NotPassable), (p(99), SourceProblem::UnknownProvince)]))
    );
    // 空の通行条件では起点が通行不可
    assert_eq!(
        reach(&g, &[p(1)], &ExplicitPassable::default()),
        Err(ReachError::InvalidSources(vec![(p(1), SourceProblem::NotPassable)]))
    );
    // 通行条件に存在しないIDが入っていれば、黙って捨てずに入力不正
    assert_eq!(ExplicitPassable::new(&g, pids(&[1, 77, 88])), Err(UnknownPassableProvinces(pids(&[77, 88]))));
    // 妥当な問合せの結果が到達不能であることは、エラーではない
    let r = reach(&g, &[p(1)], &passable(&g, &[1, 6])).unwrap();
    assert_eq!(r.reachability(p(6)), Reachability::Unreachable(UnreachableReason::NoRoute));
}

#[test]
fn unknown_province_ids_water_cells_and_cells_outside_the_map_are_not_applicable() {
    let map = map_from(&["#.##"], &[]);
    // 表: (0,0)=1, (2,0)=2。(3,0) は陸だが表に無い(範囲外)。
    let g = graph_of(&map, &[(1, 0, 0), (2, 2, 0)], &[], Coverage::Partial);
    let r = reach(&g, &[p(2)], &ExplicitPassable::all_nodes(&g)).unwrap();
    use NotApplicableReason as N;
    assert_eq!(r.reachability(p(55)), Reachability::NotApplicable(N::UnknownProvince));
    assert_eq!(r.reachability_of_cell(&g, &map, HexCoord::new(1, 0)), Reachability::NotApplicable(N::Water));
    assert_eq!(r.reachability_of_cell(&g, &map, HexCoord::new(3, 0)), Reachability::NotApplicable(N::OutOfScopeLand));
    assert_eq!(r.reachability_of_cell(&g, &map, HexCoord::new(40, 40)), Reachability::NotApplicable(N::OutsideMap));
    assert_eq!(r.reachability_of_cell(&g, &map, HexCoord::new(2, 0)), Reachability::Reachable);
    // 島 {1}(海で隔てられている)は、範囲外の陸(3,0)をすべて通れると仮定しても着けない → 確定の到達不能
    assert_eq!(r.reachability_of_cell(&g, &map, HexCoord::new(0, 0)), Reachability::Unreachable(UnreachableReason::NoRoute));
}

#[test]
fn unknown_arises_only_when_a_route_through_out_of_scope_land_is_possible() {
    // 行: "#####"(q=0..4)。表: q0=A(1), q1=B(2), q4=E(5)。q2,q3 は陸だが表に無い(範囲外の陸)。
    let map = map_from(&["#####"], &[]);
    let g = graph_of(&map, &[(1, 0, 0), (2, 1, 0), (5, 4, 0)], &[], Coverage::Partial);
    assert_eq!(g.scope().out_of_scope_land_cells, 2);
    assert!(!g.scope().is_complete());
    assert_eq!(g.edge_count(), 1, "the known graph has only A-B; out-of-scope land is never a bridge");
    let all = ExplicitPassable::all_nodes(&g);
    let r = reach(&g, &[p(1)], &all).unwrap();
    assert_eq!(reached(&r), vec![1, 2], "E is not reached through the out-of-scope cells");
    assert!(r.touches_out_of_scope());
    assert_eq!(r.reachability(p(5)), Reachability::Unknown, "B touches out-of-scope land that could lead to E");
    assert_eq!(r.unknown_ids(), pids(&[5]));
    assert_eq!(r.reachability(p(2)), Reachability::Reachable, "a known route is still reachable on an incomplete graph");
    // 明示条件で E を通行不可にすれば、不明ではなく確定の到達不能
    let no_e = all.without(&passable(&g, &[5]));
    let r2 = reach(&g, &[p(1)], &no_e).unwrap();
    assert_eq!(r2.reachability(p(5)), Reachability::Unreachable(UnreachableReason::NotPassable));
    assert_eq!(r2.unknown_count(), 0);
}

#[test]
fn an_incomplete_graph_still_gives_definite_unreachable_when_no_route_is_possible() {
    // "###.##": 島1 = q0..2、海 q3、島2 = q4,5。表: A(1)=q0, B(2)=q1, C(3)=q4。q2 と q5 は範囲外の陸。
    let map = map_from(&["###.##"], &[]);
    let g = graph_of(&map, &[(1, 0, 0), (2, 1, 0), (3, 4, 0)], &[], Coverage::Partial);
    assert!(!g.scope().is_complete());
    let r = reach(&g, &[p(1)], &ExplicitPassable::all_nodes(&g)).unwrap();
    assert_eq!(reached(&r), vec![1, 2]);
    assert!(r.touches_out_of_scope(), "B touches the out-of-scope cell (2,0)");
    // 範囲外の陸をすべて通れると仮定しても、海 q3 を越えられないので C には着けない → 確定
    assert_eq!(r.reachability(p(3)), Reachability::Unreachable(UnreachableReason::NoRoute));
    assert_eq!(r.unknown_count(), 0, "an incomplete graph does not make every target unknown");
}

#[test]
fn unknown_spreads_through_out_of_scope_land_into_known_passable_neighbours_only() {
    // "#####": A=q0, B=q1, 範囲外 q2, C=q3, D=q4(表に C(3), D(4))。D は C 経由でも着ける(不明のまま)が、通行不可の C は通らない。
    let map = map_from(&["#####"], &[]);
    let g = graph_of(&map, &[(1, 0, 0), (2, 1, 0), (3, 3, 0), (4, 4, 0)], &[], Coverage::Partial);
    let all = ExplicitPassable::all_nodes(&g);
    let r = reach(&g, &[p(1)], &all).unwrap();
    assert_eq!(r.unknown_ids(), pids(&[3, 4]));
    let no_c = all.without(&passable(&g, &[3]));
    let r2 = reach(&g, &[p(1)], &no_c).unwrap();
    assert_eq!(r2.reachability(p(3)), Reachability::Unreachable(UnreachableReason::NotPassable));
    assert_eq!(r2.reachability(p(4)), Reachability::Unreachable(UnreachableReason::NoRoute), "the only way in is through the blocked C");
}

#[test]
fn a_province_in_the_table_without_a_state_is_a_node_whose_passability_is_the_input() {
    // 3つ(1,2,3)が1列。2 はステート未割当。通行条件に載せれば橋渡しになり、載せなければならない。
    let map = map_from(&["###"], &[]);
    let g = graph_of(&map, &[(1, 0, 0), (2, 1, 0), (3, 2, 0)], &[(10, &[1]), (20, &[3])], Coverage::Partial);
    assert_eq!(g.state_of(p(2)), None);
    let with = ExplicitPassable::all_nodes(&g);
    assert_eq!(reached(&reach(&g, &[p(1)], &with).unwrap()), vec![1, 2, 3]);
    let without = ExplicitPassable::from_states(&g, &[MapStateId(10), MapStateId(20)]).unwrap();
    let r = reach(&g, &[p(1)], &without).unwrap();
    assert_eq!(reached(&r), vec![1]);
    assert_eq!(r.reachability(p(3)), Reachability::Unreachable(UnreachableReason::NoRoute), "the unassigned node is not used as a bridge unless the condition allows it");
    assert_eq!(r.reachability(p(2)), Reachability::Unreachable(UnreachableReason::NotPassable));
}

// ---------------------------------------------------------------- 入力・既存状態を変更しない/古さの検出/責務

#[test]
fn queries_do_not_modify_the_graph_the_passable_condition_or_the_world() {
    let map = map_from(&["####"], &[]);
    let w = world(&map, &[(1, 0, 0), (2, 1, 0), (3, 2, 0), (4, 3, 0)], &[(10, &[1, 2]), (20, &[3, 4])], Coverage::World);
    let g = ProvinceGraph::build(&w, &map).unwrap();
    let (g0, w0) = (g.clone(), w.clone());
    let pass = ExplicitPassable::all_nodes(&g).without(&passable(&g, &[3]));
    let pass0 = pass.clone();
    let _ = reach(&g, &[p(1)], &pass).unwrap();
    let _ = reach(&g, &[p(99)], &pass);
    let _ = g.components_among(&pass);
    assert_eq!(g, g0);
    assert_eq!(w, w0);
    assert_eq!(pass, pass0);
}

#[test]
fn a_graph_and_a_result_become_stale_when_the_world_changes() {
    let map = map_from(&["###"], &[]);
    let w = world(&map, &[(1, 0, 0), (2, 1, 0), (3, 2, 0)], &[(10, &[1, 2, 3])], Coverage::World);
    let g = ProvinceGraph::build(&w, &map).unwrap();
    assert!(g.is_current_for(&w, &map));
    // 所属の変更(ステートの割り当てが変わる)→ 古い
    let w2 = world(&map, &[(1, 0, 0), (2, 1, 0), (3, 2, 0)], &[(10, &[1, 2]), (20, &[3])], Coverage::World);
    assert!(!g.is_current_for(&w2, &map));
    // プロヴィンスの追加・削除 → 古い
    let w3 = world(&map, &[(1, 0, 0), (2, 1, 0)], &[], Coverage::Partial);
    assert!(!g.is_current_for(&w3, &map));
    // 地図の変更 → 古い
    let map2 = map_from(&["##."], &[]);
    assert!(!g.is_current_for(&w, &map2));
    // 結果は、作ったグラフに対してだけ有効
    let r = reach(&g, &[p(1)], &ExplicitPassable::all_nodes(&g)).unwrap();
    assert!(r.is_for(&g));
    let g3 = ProvinceGraph::build(&w3, &map).unwrap();
    assert!(!r.is_for(&g3));
}

#[test]
fn the_connectivity_code_knows_nothing_about_economy_saving_or_the_military() {
    let src = std::fs::read_to_string("src/world/connectivity.rs").unwrap();
    // コードの部分(コメントを除く)に、経済・保存・軍の型への参照が無い
    let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    for banned in [
        "MapEconomy", "economy::", "snapshot", "map_economy_file", "SaveLocation", "military::", "Division", "StateRegistry", "std::fs", "File::", "std::time", "Instant", "SystemTime", "rand",
    ] {
        assert!(!code.contains(banned), "connectivity.rs must stay pure: found {banned}");
    }
}

// ---------------------------------------------------------------- 実データ(検証用ワールド: 29プロヴィンス)

fn real() -> (HexMap, WorldPreviewData) {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let w = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).unwrap();
    (map, w)
}

const STATE_20: [u32; 7] = [18109, 18308, 18309, 18310, 18311, 18499, 18500];
const STATE_21: [u32; 13] = [17269, 17486, 17487, 17694, 17695, 17696, 17904, 17905, 17906, 18111, 18112, 18312, 18502];
const STATE_22: [u32; 6] = [17266, 17267, 17268, 17484, 17485, 17693];
const UNASSIGNED: [u32; 3] = [17903, 18110, 18501];

/// テスト内の独立した総当たり: 表のセルだけから、別に書いた隣接の規則で辺を数える。
fn brute_force_edges(w: &WorldPreviewData) -> BTreeSet<(u32, u32)> {
    const OFFSETS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
    let cell_of: HashMap<(i32, i32), u32> = w.model.provinces().iter().map(|d| ((d.cell.q, d.cell.r), d.id.0)).collect();
    let mut out = BTreeSet::new();
    for d in w.model.provinces() {
        for (dq, dr) in OFFSETS {
            if let Some(&other) = cell_of.get(&(d.cell.q + dq, d.cell.r + dr))
                && other != d.id.0
            {
                out.insert((d.id.0.min(other), d.id.0.max(other)));
            }
        }
    }
    out
}

/// テスト内の独立した計算: 陸セルだけを使い、(a) 表のセルは `allowed` のものだけ通れる(既知の辺)、
/// (b) さらに表に無い陸セルをすべて通れると仮定したとき、の到達プロヴィンスを返す。実装の探索は使わない。
fn independent_reach(map: &HexMap, w: &WorldPreviewData, allowed: &HashSet<u32>, source: u32) -> (BTreeSet<u32>, BTreeSet<u32>) {
    const OFFSETS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
    let prov_at: HashMap<(i32, i32), u32> = w.model.provinces().iter().map(|d| ((d.cell.q, d.cell.r), d.id.0)).collect();
    let src_cell = *prov_at.iter().find(|(_, id)| **id == source).unwrap().0;
    let land = |q: i32, r: i32| map.cell(q, r).is_some_and(|c| !c.terrain.is_water());
    let run = |through_unknown: bool| -> BTreeSet<u32> {
        let mut seen: HashSet<(i32, i32)> = HashSet::new();
        let mut stack = vec![src_cell];
        let mut out = BTreeSet::new();
        while let Some(c) = stack.pop() {
            if !seen.insert(c) {
                continue;
            }
            if let Some(id) = prov_at.get(&c) {
                out.insert(*id);
            }
            for (dq, dr) in OFFSETS {
                let n = (c.0 + dq, c.1 + dr);
                if seen.contains(&n) || !land(n.0, n.1) {
                    continue;
                }
                let ok = match prov_at.get(&n) {
                    Some(id) => allowed.contains(id),
                    None => through_unknown,
                };
                if ok {
                    stack.push(n);
                }
            }
        }
        out
    };
    (run(false), run(true))
}

#[test]
fn the_validation_region_has_29_nodes_65_edges_and_one_component() {
    let (map, w) = real();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    assert_eq!(g.node_count(), 29);
    assert_eq!(g.edge_count(), 65);
    assert_eq!(g.components().len(), 1);
    assert_eq!(g.components()[0].len(), 29);
    // 独立した総当たりと一致
    let brute = brute_force_edges(&w);
    assert_eq!(brute.len(), 65);
    assert_eq!(edges_of(&g), brute);
    // ステート間の辺(両端が割り当て済みで、ステートが異なる)8、河川の辺 24
    assert_eq!(g.cross_state_edge_count(), 8);
    assert_eq!(g.river_edge_count(), 24);
    // 範囲と完全性: 検証用ワールドは部分的(未割当3、範囲外の陸 = 陸セル総数 - 29)
    let land = map.cells().iter().filter(|c| !c.terrain.is_water()).count();
    let s = g.scope();
    assert_eq!((s.coverage, s.nodes, s.unassigned_nodes), (Coverage::Partial, 29, 3));
    assert_eq!(s.out_of_scope_land_cells, land - 29);
    assert_eq!(s.out_of_scope_land_cells, 35_192, "35,221 land cells - 29 provinces (current map)");
    assert!(!s.is_complete());
    // 未割当ノードの正体(ステート配置に載っていないプロヴィンス)
    let un: Vec<u32> = g.node_ids().filter(|id| g.state_of(*id).is_none()).map(|x| x.0).collect();
    assert_eq!(un, UNASSIGNED.to_vec());
    assert!(g.is_current_for(&w.model, &map));
}

#[test]
fn blocking_state_21_splits_the_assigned_provinces_into_the_two_documented_components() {
    let (map, w) = real();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    // 割り当て済みのうち、ステート 20 と 22 だけを通行可能にする(= ステート21と未割当を通れない)。設計書(§2.4)の「6 と 7 の2成分」。
    let pass = ExplicitPassable::from_states(&g, &[MapStateId(20), MapStateId(22)]).unwrap();
    assert_eq!(pass.len(), 13);
    let comps = g.components_among(&pass);
    assert_eq!(comps.len(), 2);
    let mut sizes: Vec<usize> = comps.iter().map(|c| c.len()).collect();
    sizes.sort();
    assert_eq!(sizes, vec![6, 7]);
    // 成分の中身が、ステートの所属と一致する
    let as_sets: BTreeSet<BTreeSet<u32>> = comps.iter().map(|c| c.iter().map(|x| x.0).collect()).collect();
    let expect: BTreeSet<BTreeSet<u32>> = [STATE_20.iter().copied().collect(), STATE_22.iter().copied().collect()].into_iter().collect();
    assert_eq!(as_sets, expect);
    // 独立した総当たり(別のBFS)でも2成分
    let brute = brute_force_edges(&w);
    let allowed: HashSet<u32> = STATE_20.iter().chain(STATE_22.iter()).copied().collect();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut n = 0;
    for s in &allowed {
        if seen.contains(s) {
            continue;
        }
        n += 1;
        let mut stack = vec![*s];
        while let Some(x) = stack.pop() {
            if !seen.insert(x) {
                continue;
            }
            for (a, b) in &brute {
                let y = if *a == x { *b } else if *b == x { *a } else { continue };
                if allowed.contains(&y) && !seen.contains(&y) {
                    stack.push(y);
                }
            }
        }
    }
    assert_eq!(n, 2);
    // 未割当を通行可能に含めるかは入力次第: 「ステート21だけ通行不可」(未割当は通行可能)の成分数は、独立した総当たりと一致する。
    let s21 = ExplicitPassable::from_states(&g, &[MapStateId(21)]).unwrap();
    assert_eq!(s21.ids().map(|x| x.0).collect::<Vec<_>>(), STATE_21.to_vec(), "the state 21 members match states.ron");
    let all_but_21 = ExplicitPassable::all_nodes(&g).without(&s21);
    assert_eq!(all_but_21.len(), 29 - 13);
    let allowed2: HashSet<u32> = STATE_20.iter().chain(STATE_22.iter()).chain(UNASSIGNED.iter()).copied().collect();
    let mut seen2: HashSet<u32> = HashSet::new();
    let mut n2 = 0;
    for s in &allowed2 {
        if seen2.contains(s) {
            continue;
        }
        n2 += 1;
        let mut stack = vec![*s];
        while let Some(x) = stack.pop() {
            if !seen2.insert(x) {
                continue;
            }
            for (a, b) in &brute {
                let y = if *a == x { *b } else if *b == x { *a } else { continue };
                if allowed2.contains(&y) && !seen2.contains(&y) {
                    stack.push(y);
                }
            }
        }
    }
    assert_eq!(g.components_among(&all_but_21).len(), n2);
}

#[test]
fn concrete_reach_results_on_the_validation_region() {
    let (map, w) = real();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    // 起点 18309(ステート20)、通行可能 = ステート20と22: 既知の辺で着くのはステート20の7プロヴィンスだけ。
    let pass = ExplicitPassable::from_states(&g, &[MapStateId(20), MapStateId(22)]).unwrap();
    let r = reach(&g, &[p(18309)], &pass).unwrap();
    assert_eq!(reached(&r), STATE_20.to_vec());
    // ステート21(通行不可)は確定の到達不能(理由: 通行不可)
    assert_eq!(r.reachability(p(17694)), Reachability::Unreachable(UnreachableReason::NotPassable));
    // 「不明」の集合は、テスト内の独立した計算(陸セルだけを使う別のBFS。範囲外の陸はすべて通れると仮定)と一致する。
    let allowed: HashSet<u32> = STATE_20.iter().chain(STATE_22.iter()).copied().collect();
    let (known, optimistic) = independent_reach(&map, &w, &allowed, 18309);
    assert_eq!(known, STATE_20.iter().copied().collect::<BTreeSet<u32>>());
    let expected_unknown: Vec<u32> = optimistic.difference(&known).copied().collect();
    assert_eq!(r.unknown_ids().iter().map(|x| x.0).collect::<Vec<_>>(), expected_unknown);
    assert!(r.touches_out_of_scope());
    for id in STATE_22 {
        let want = if optimistic.contains(&id) { Reachability::Unknown } else { Reachability::Unreachable(UnreachableReason::NoRoute) };
        assert_eq!(r.reachability(p(id)), want, "Province#{id}");
    }
    // 全ノード通行可能なら、29すべてに到達し、不明は無い。
    let all = ExplicitPassable::all_nodes(&g);
    let r = reach(&g, &[p(18309)], &all).unwrap();
    assert_eq!(r.reached_count(), 29);
    assert_eq!(r.unknown_count(), 0);
    assert_eq!(r.reachability(p(17266)), Reachability::Reachable, "across states 20 -> unassigned/21 -> 22");
    // セル単位の対象: 陸だが表に無いセルは対象外、海のセルも対象外
    let table_cells: HashSet<(i32, i32)> = w.model.provinces().iter().map(|d| (d.cell.q, d.cell.r)).collect();
    let oos = map.cells().iter().find(|c| !c.terrain.is_water() && !table_cells.contains(&(c.q, c.r))).unwrap();
    assert_eq!(r.reachability_of_cell(&g, &map, HexCoord::new(oos.q, oos.r)), Reachability::NotApplicable(NotApplicableReason::OutOfScopeLand));
    let sea = map.cells().iter().find(|c| c.terrain.is_water()).unwrap();
    assert_eq!(r.reachability_of_cell(&g, &map, HexCoord::new(sea.q, sea.r)), Reachability::NotApplicable(NotApplicableReason::Water));
    // 存在しない起点・通行不可の起点は入力不正
    assert_eq!(reach(&g, &[p(1)], &all), Err(ReachError::InvalidSources(vec![(p(1), SourceProblem::UnknownProvince)])));
    assert_eq!(
        reach(&g, &[p(17694)], &pass),
        Err(ReachError::InvalidSources(vec![(p(17694), SourceProblem::NotPassable)]))
    );
}

#[test]
fn the_whole_world_table_gives_a_complete_graph_with_no_unknown_results() {
    // 全世界の初回採番の表(`generate_initial_province_table`。検証用の入力で、保存済みIDではない)とステートなしの配置。
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let t = generate_initial_province_table(&map, Coverage::World);
    let w = WorldModel::build(t, layout(&[])).unwrap();
    let g = ProvinceGraph::build(&w, &map).unwrap();
    let land = map.cells().iter().filter(|c| !c.terrain.is_water()).count();
    assert_eq!(g.node_count(), land);
    assert!(g.scope().is_complete());
    assert_eq!(g.scope().unassigned_nodes, land, "no state layout: every node is unassigned");
    let comps = g.components();
    assert!(comps.len() > 1, "the world has several continents/islands separated by water");
    assert_eq!(comps.iter().map(|c| c.len()).sum::<usize>(), land);
    // 最大の成分から1つの起点を取り、全ノード通行可能で到達 = その成分(完全なグラフなので不明は無い)
    let biggest = comps.iter().max_by_key(|c| c.len()).unwrap();
    let r = reach(&g, &[biggest[0]], &ExplicitPassable::all_nodes(&g)).unwrap();
    assert_eq!(r.reached_ids(), *biggest);
    assert_eq!(r.unknown_count(), 0);
    // 別の成分は確定の到達不能
    let other = comps.iter().find(|c| c[0] != biggest[0]).unwrap();
    assert_eq!(r.reachability(other[0]), Reachability::Unreachable(UnreachableReason::NoRoute));
    // 辺の数の独立確認: セルごとに独立した近傍規則で数える(1陸セル=1プロヴィンス)
    const OFFSETS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
    let lands: HashSet<(i32, i32)> = map.cells().iter().filter(|c| !c.terrain.is_water()).map(|c| (c.q, c.r)).collect();
    let mut contacts = 0usize;
    for (q, r) in &lands {
        for (dq, dr) in OFFSETS {
            if lands.contains(&(q + dq, r + dr)) {
                contacts += 1;
            }
        }
    }
    assert_eq!(g.edge_count(), contacts / 2);
}
