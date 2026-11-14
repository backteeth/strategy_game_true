//! P32-031 (M5-b): ステート単位の到達要約(`world::state_reach`)・旧小規模ケースとの一致・測定専用入力(`world::measure`)の検証。
//!
//! 期待値は、手で書いた小さな地図(軸座標。隣接の規則: (q,r) の6近傍は (q+1,r) (q+1,r-1) (q,r-1) (q-1,r) (q-1,r+1) (q,r+1))と、
//! 手で書いた旧モデルの州隣接から作る。実データの期待値は、独立した総当たりでも確認する。**時間は合否条件にしない**
//! (規模測定は `world_reach_bench`。このテストに大規模なグラフ構築・測定は含めない)。保存・アセットは書かない。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::json;
use strategy_game::common::{CountryId, StateId};
use strategy_game::hexmap::data::{HexMap, load_from_path, parse_hex_map};
use strategy_game::military::capital_connectivity::connected_states_from_capital;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::world::fingerprint::FINGERPRINT_ALGORITHM;
use strategy_game::world::measure::{LoadKind, MeasureRule, land_definitions, make_agents};
use strategy_game::world::state_reach::kinds;
use strategy_game::world::*;

// ---------------------------------------------------------------- 人工地図の部品(P32-030 のテストと同じ作り)

/// 行 = r、列 = q。`.` 海、`#` 平原(陸)。
fn map_from(rows: &[&str]) -> HexMap {
    let (h, w) = (rows.len(), rows.iter().map(|r| r.len()).max().unwrap());
    let mut cells = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (q, ch) in row.chars().enumerate() {
            let (terrain, climate, veg) = match ch {
                '.' => ("sea", "unclassified", "unclassified"),
                _ => ("plains", "C", "grassland"),
            };
            cells.push(json!({ "q": q, "r": r, "terrain": terrain, "climate": climate, "vegetation": veg }));
        }
    }
    let v = json!({
        "schemaVersion": 9,
        "kind": "hex-atlas-game-map",
        "grid": {
            "orientation": "pointy", "size": 2.5, "origin": { "x": 2.1650635094610964, "y": 2.5 },
            "imageWidth": 100, "imageHeight": 60,
            "extent": { "minQ": 0, "maxQ": w - 1, "minR": 0, "maxR": h - 1 }
        },
        "cells": cells,
        "rivers": [],
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

fn setup(map: &HexMap, provinces: &[(u32, i32, i32)], states: &[(u32, &[u32])], coverage: Coverage) -> (WorldModel, ProvinceGraph) {
    let w = world(map, provinces, states, coverage);
    let g = ProvinceGraph::build(&w, map).expect("graph must build");
    (w, g)
}

fn pass(g: &ProvinceGraph, ids: &[u32]) -> ExplicitPassable {
    ExplicitPassable::new(g, ids.iter().map(|x| ProvinceId(*x))).expect("known provinces")
}

fn s(id: u32) -> MapStateId {
    MapStateId(id)
}

/// 要約の不変条件: 各件数の合計が所属総数に一致し、全ステート+未所属の総数がグラフのノード数に一致する。
fn assert_invariants(sum: &StateReachSummary, g: &ProvinceGraph) {
    for st in sum.states() {
        assert!(st.counts.is_consistent(), "{:?}", st);
    }
    assert!(sum.unassigned().is_consistent());
    assert_eq!(sum.grand_totals().total, g.node_count(), "every node is counted exactly once (states + unassigned)");
    let ids: Vec<MapStateId> = sum.states().iter().map(|x| x.state).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "states are in MapStateId order");
}

// ---------------------------------------------------------------- 区分の判定(全域・一部・不能・不明・対象外・空)

/// 9陸の1行。ID = 10 + q。S5={10,11,12} S6={13,14,15} S7={16,17,18}。13 だけ通行不可、起点 10。
#[test]
fn full_unreachable_and_blocked_states_on_a_complete_map() {
    let map = map_from(&["#########"]);
    let provs: Vec<(u32, i32, i32)> = (0..9).map(|q| (10 + q as u32, q, 0)).collect();
    let (w, g) = setup(&map, &provs, &[(5, &[10, 11, 12]), (6, &[13, 14, 15]), (7, &[16, 17, 18])], Coverage::World);
    let p = pass(&g, &[10, 11, 12, 14, 15, 16, 17, 18]);
    let r = reach(&g, &[ProvinceId(10)], &p).unwrap();
    let sum = summarize_by_state(&g, &r).unwrap();
    assert_invariants(&sum, &g);
    let s5 = sum.state(s(5)).unwrap();
    assert_eq!((s5.kind, s5.counts.total, s5.counts.reachable), (StateReachKind::Full, 3, 3));
    // S6: 13 は通行不可、14・15 は通行可能だが経路が無い(確定)。到達可能が0件で、不明は無い → 到達不能
    let s6 = sum.state(s(6)).unwrap();
    assert_eq!(s6.kind, StateReachKind::Unreachable);
    assert_eq!((s6.counts.reachable, s6.counts.unreachable_not_passable, s6.counts.unreachable_no_route, s6.counts.unknown), (0, 1, 2, 0));
    let s7 = sum.state(s(7)).unwrap();
    assert_eq!((s7.kind, s7.counts.unreachable_no_route), (StateReachKind::Unreachable, 3));
    assert_eq!(sum.unassigned().total, 0);
    // モデルの所属から導いた要約も同じ
    assert_eq!(summarize_model(&w, &r).unwrap(), sum);
}

/// ステート内部の分断(旧ステート単位では表せない): 一部に到達できても、ステート全域に到達したとは判定しない。
#[test]
fn a_state_split_inside_is_partial_never_full() {
    let map = map_from(&["#####"]);
    let provs: Vec<(u32, i32, i32)> = (0..5).map(|q| (10 + q as u32, q, 0)).collect();
    // S1 は 10,11,13,14(間の 12 は別ステート S2)。12 が通行不可なので S1 は内部で2つに分かれる。
    let (_w, g) = setup(&map, &provs, &[(1, &[10, 11, 13, 14]), (2, &[12])], Coverage::World);
    let r = reach(&g, &[ProvinceId(10)], &pass(&g, &[10, 11, 13, 14])).unwrap();
    let sum = summarize_by_state(&g, &r).unwrap();
    assert_invariants(&sum, &g);
    let s1 = sum.state(s(1)).unwrap();
    assert_eq!(s1.kind, StateReachKind::Partial, "reaching 2 of 4 is partial");
    assert_eq!((s1.counts.total, s1.counts.reachable, s1.counts.unreachable_no_route, s1.counts.unreachable_not_passable), (4, 2, 2, 0));
    assert_eq!(sum.state(s(2)).unwrap().kind, StateReachKind::Unreachable);
    // 通行条件を変えて 12 を通れるようにすると、S1 は全域到達、S2 も全域到達になる(通行条件変更後の要約)
    let r2 = reach(&g, &[ProvinceId(10)], &pass(&g, &[10, 11, 12, 13, 14])).unwrap();
    let sum2 = summarize_by_state(&g, &r2).unwrap();
    assert_eq!(kinds(&sum2), [(s(1), StateReachKind::Full), (s(2), StateReachKind::Full)].into_iter().collect::<BTreeMap<_, _>>());
    // 元の結果は変わらない(純粋)
    assert_eq!(summarize_by_state(&g, &r).unwrap(), sum);
}

/// 不明を到達不能へ丸めない。範囲外の陸の先は「不明」、海で隔てられた先は確定した「到達不能」。
#[test]
fn unknown_is_never_rounded_to_unreachable() {
    // q0,q1 = 区画あり(S1)、q2 = 範囲外の陸、q3,q4 = 区画あり(S2)、q5 = 海、q6,q7 = 区画あり(S3)
    let map = map_from(&["#####.##"]);
    let provs = [(10, 0, 0), (11, 1, 0), (13, 3, 0), (14, 4, 0), (16, 6, 0), (17, 7, 0)];
    let (w, g) = setup(&map, &provs, &[(1, &[10, 11]), (2, &[13, 14]), (3, &[16, 17])], Coverage::Partial);
    assert_eq!(g.scope().out_of_scope_land_cells, 1);
    let r = reach(&g, &[ProvinceId(10)], &ExplicitPassable::all_nodes(&g)).unwrap();
    let sum = summarize_by_state(&g, &r).unwrap();
    assert_invariants(&sum, &g);
    assert_eq!(sum.state(s(1)).unwrap().kind, StateReachKind::Full);
    let s2 = sum.state(s(2)).unwrap();
    assert_eq!(s2.kind, StateReachKind::Unknown, "reachable only through land outside the table");
    assert_eq!((s2.counts.unknown, s2.counts.unreachable()), (2, 0));
    let s3 = sum.state(s(3)).unwrap();
    assert_eq!(s3.kind, StateReachKind::Unreachable, "separated by sea: definitely unreachable");
    assert_eq!((s3.counts.unknown, s3.counts.unreachable_no_route), (0, 2));
    assert_eq!(summarize_model(&w, &r).unwrap(), sum);
}

/// 一部が到達可能で残りが不明のステートは一部到達(全域にしない)。到達可能0で不明と不能が混じれば不明。
#[test]
fn mixed_unknown_states_are_partial_or_unknown() {
    let map = map_from(&["######"]);
    // q0,q1 区画あり(S1)/ q2 範囲外 / q3,q4 区画あり(S2: q4 は通行不可)/ q5 区画あり(S3: 通行可能だが S2 の q4 に遮られる)
    let provs = [(10, 0, 0), (11, 1, 0), (13, 3, 0), (14, 4, 0), (15, 5, 0)];
    let (_w, g) = setup(&map, &provs, &[(1, &[10, 11]), (2, &[13, 14]), (3, &[15])], Coverage::Partial);
    let r = reach(&g, &[ProvinceId(10)], &pass(&g, &[10, 11, 13, 15])).unwrap();
    let sum = summarize_by_state(&g, &r).unwrap();
    assert_invariants(&sum, &g);
    // S2: 13 は範囲外経由で到達できる可能性(不明)、14 は通行不可(確定) → 到達可能0・不明1・不能1 → 不明
    let s2 = sum.state(s(2)).unwrap();
    assert_eq!(s2.kind, StateReachKind::Unknown);
    assert_eq!((s2.counts.unknown, s2.counts.unreachable_not_passable, s2.counts.reachable), (1, 1, 0));
    // S3: 15 は 14(通行不可)に遮られ、範囲外の陸にも接しない → 確定した到達不能
    assert_eq!(sum.state(s(3)).unwrap().kind, StateReachKind::Unreachable);
    // 一部が到達可能で、残りが不明のステート(10 は到達可能、13 は範囲外経由の不明)→ 一部到達(全域にしない)
    let (_w2, g2) = setup(&map, &provs, &[(1, &[10, 13]), (2, &[11, 14]), (3, &[15])], Coverage::Partial);
    let r2 = reach(&g2, &[ProvinceId(10)], &pass(&g2, &[10, 11, 13, 15])).unwrap();
    let sum2 = summarize_by_state(&g2, &r2).unwrap();
    assert_invariants(&sum2, &g2);
    let m = sum2.state(s(1)).unwrap();
    assert_eq!(m.kind, StateReachKind::Partial);
    assert_eq!((m.counts.reachable, m.counts.unknown), (1, 1));
}

/// 空のステート・判定できないステートを全域到達にしない。所属表にだけ書かれた未知のIDは対象外。
#[test]
fn empty_and_not_applicable_states_are_never_full() {
    let map = map_from(&["###"]);
    let provs = [(10, 0, 0), (11, 1, 0), (12, 2, 0)];
    let (_w, g) = setup(&map, &provs, &[(1, &[10, 11, 12])], Coverage::World);
    let r = reach(&g, &[ProvinceId(10)], &pass(&g, &[10, 11])).unwrap();
    let members: Vec<(MapStateId, Vec<ProvinceId>)> = vec![
        (s(1), vec![ProvinceId(10)]),
        (s(2), vec![]),                                  // 空
        (s(3), vec![ProvinceId(997), ProvinceId(998)]),  // グラフに無いID → すべて対象外
        (s(5), vec![ProvinceId(11), ProvinceId(999)]),   // 到達可能1 + 対象外1 → 一部
        (s(6), vec![ProvinceId(12)]),                    // 通行不可
    ];
    let refs: Vec<(MapStateId, &[ProvinceId])> = members.iter().map(|(a, b)| (*a, b.as_slice())).collect();
    let sum = summarize_membership(&r, &refs, &[]).unwrap();
    let k = kinds(&sum);
    assert_eq!(k[&s(1)], StateReachKind::Full);
    assert_eq!(k[&s(2)], StateReachKind::Empty);
    assert_eq!(k[&s(3)], StateReachKind::NotApplicable);
    assert_eq!(k[&s(5)], StateReachKind::Partial, "an unjudgeable member prevents `full`");
    assert_eq!(k[&s(6)], StateReachKind::Unreachable);
    assert_eq!(sum.state(s(3)).unwrap().counts.not_applicable, 2);
    assert!(sum.states().iter().all(|x| x.counts.is_consistent()));
    // 不正な所属表(同じステートが2回、同じプロヴィンスが2つのステート)は拒否
    let dup_state: Vec<(MapStateId, &[ProvinceId])> = vec![(s(1), &[ProvinceId(10)]), (s(1), &[ProvinceId(11)])];
    assert_eq!(summarize_membership(&r, &dup_state, &[]), Err(SummaryError::DuplicateState(s(1))));
    let dup_prov: Vec<(MapStateId, &[ProvinceId])> = vec![(s(1), &[ProvinceId(10)]), (s(2), &[ProvinceId(10)])];
    assert_eq!(summarize_membership(&r, &dup_prov, &[]), Err(SummaryError::ProvinceInMultipleStates(ProvinceId(10))));
    assert_eq!(summarize_membership(&r, &refs, &[ProvinceId(11)]), Err(SummaryError::ProvinceInMultipleStates(ProvinceId(11))));
}

// ---------------------------------------------------------------- ステート未所属

/// 4陸の1行: S1={10,11} / 12 は未所属 / S2={13}。
fn unassigned_world() -> (WorldModel, ProvinceGraph) {
    let map = map_from(&["####"]);
    setup(&map, &[(10, 0, 0), (11, 1, 0), (12, 2, 0), (13, 3, 0)], &[(1, &[10, 11]), (2, &[13])], Coverage::World)
}

#[test]
fn an_unassigned_province_can_bridge_only_when_explicitly_allowed_and_is_never_counted_in_a_state() {
    let (w, g) = unassigned_world();
    // 明示的に許可 → 橋渡しになる。未所属は、どのステートの件数にも入らない。
    let r = reach(&g, &[ProvinceId(10)], &ExplicitPassable::all_nodes(&g)).unwrap();
    let sum = summarize_by_state(&g, &r).unwrap();
    assert_invariants(&sum, &g);
    assert_eq!(sum.state(s(2)).unwrap().kind, StateReachKind::Full, "reached through the unassigned province");
    assert_eq!((sum.state(s(1)).unwrap().counts.total, sum.state(s(2)).unwrap().counts.total), (2, 1));
    assert_eq!((sum.unassigned().total, sum.unassigned().reachable), (1, 1));
    assert_eq!(sum.assigned_totals().total, 3);
    assert_eq!(summarize_model(&w, &r).unwrap(), sum);
    // 許可しない → 橋渡しにならない(S2 は通行可能でも着けない)
    let r2 = reach(&g, &[ProvinceId(10)], &pass(&g, &[10, 11, 13])).unwrap();
    let sum2 = summarize_by_state(&g, &r2).unwrap();
    assert_invariants(&sum2, &g);
    assert_eq!(sum2.state(s(1)).unwrap().kind, StateReachKind::Full);
    assert_eq!(sum2.state(s(2)).unwrap().kind, StateReachKind::Unreachable);
    assert_eq!((sum2.unassigned().unreachable_not_passable, sum2.unassigned().reachable), (1, 0));
    assert_eq!(sum2.state(s(2)).unwrap().counts.unreachable_no_route, 1);
}

// ---------------------------------------------------------------- 入力順・ID・古い結果

#[test]
fn the_summary_does_not_depend_on_input_order() {
    let map = map_from(&["#########"]);
    let a: Vec<(u32, i32, i32)> = (0..9).map(|q| (10 + q as u32, q, 0)).collect();
    let mut b = a.clone();
    b.reverse();
    b.swap(1, 6);
    let (_, ga) = setup(&map, &a, &[(5, &[10, 11, 12]), (6, &[13, 14, 15]), (7, &[16, 17])], Coverage::World);
    let (_, gb) = setup(&map, &b, &[(7, &[17, 16]), (6, &[15, 14, 13]), (5, &[12, 11, 10])], Coverage::World);
    let ra = reach(&ga, &[ProvinceId(10), ProvinceId(11)], &pass(&ga, &[10, 11, 12, 13, 14, 16, 17])).unwrap();
    let rb = reach(&gb, &[ProvinceId(11), ProvinceId(10)], &pass(&gb, &[17, 16, 14, 13, 12, 11, 10])).unwrap();
    let (sa, sb) = (summarize_by_state(&ga, &ra).unwrap(), summarize_by_state(&gb, &rb).unwrap());
    assert_eq!(sa, sb);
    // 18 は未所属(どのステートにも載っていない)
    assert_eq!(sa.unassigned().total, 1);
    assert_eq!(sa.state(s(6)).unwrap().kind, StateReachKind::Partial);
}

#[test]
fn province_ids_and_state_ids_are_never_converted_by_numeric_equality() {
    // ProvinceId 1,2,3 と MapStateId 1,2 が同じ数値。S2 に属するのは P1 と P3、S1 に属するのは P2。
    let map = map_from(&["###"]);
    let (_w, g) = setup(&map, &[(1, 0, 0), (2, 1, 0), (3, 2, 0)], &[(2, &[1, 3]), (1, &[2])], Coverage::World);
    let r = reach(&g, &[ProvinceId(1)], &ExplicitPassable::all_nodes(&g)).unwrap();
    let sum = summarize_by_state(&g, &r).unwrap();
    assert_eq!(sum.state(s(2)).unwrap().counts.total, 2, "S2 = {{P1, P3}}");
    assert_eq!(sum.state(s(1)).unwrap().counts.total, 1, "S1 = {{P2}}");
    assert!(sum.state(s(3)).is_none(), "P3 does not make a state 3");
    assert_eq!(g.state_of(ProvinceId(1)), Some(s(2)));
}

#[test]
fn a_reach_from_another_graph_is_rejected() {
    let map = map_from(&["###"]);
    let (_w1, g1) = setup(&map, &[(1, 0, 0), (2, 1, 0), (3, 2, 0)], &[(1, &[1, 2, 3])], Coverage::World);
    let (_w2, g2) = setup(&map, &[(1, 0, 0), (2, 1, 0), (3, 2, 0)], &[(1, &[1, 2]), (2, &[3])], Coverage::World);
    let r = reach(&g1, &[ProvinceId(1)], &ExplicitPassable::all_nodes(&g1)).unwrap();
    assert_eq!(summarize_by_state(&g2, &r), Err(SummaryError::ReachNotForGraph));
    assert!(summarize_by_state(&g1, &r).is_ok());
}

#[test]
fn the_summary_code_does_not_search_or_touch_economy_saving_or_the_military() {
    let src = std::fs::read_to_string("src/world/state_reach.rs").unwrap();
    let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    for banned in [
        "reach(", "VecDeque", "BinaryHeap", "components", "MapEconomy", "economy::", "snapshot", "SaveLocation", "military::", "Division", "StateRegistry", "std::fs",
        "std::time", "Instant", "rand::",
    ] {
        assert!(!code.contains(banned), "state_reach.rs must only aggregate: found {banned}");
    }
}

// ---------------------------------------------------------------- 旧 connected_states_from_capital との一致(小規模)

/// 旧ステート(`StateId`)・新プロヴィンス(`ProvinceId`)・新ステート(`MapStateId`)の対応表(数値は意図的に互いに異なる)。
struct Pair {
    old: usize,
    province: u32,
    new_state: u32,
    cell: (i32, i32),
}

struct OldCase {
    rows: &'static [&'static str],
    pairs: Vec<Pair>,
    /// 旧モデルの州隣接(手で書いた対称な組)。
    old_edges: Vec<(usize, usize)>,
    /// 旧モデルで自国(US)が支配する州。
    controlled_by_us: Vec<usize>,
    capital: usize,
}

const US: CountryId = CountryId(1);
const THEM: CountryId = CountryId(2);

impl OldCase {
    fn province_of(&self, old: usize) -> ProvinceId {
        ProvinceId(self.pairs.iter().find(|p| p.old == old).unwrap().province)
    }

    fn old_of(&self, province: ProvinceId) -> usize {
        self.pairs.iter().find(|p| p.province == province.0).unwrap().old
    }

    fn state_of_old(&self, old: usize) -> MapStateId {
        MapStateId(self.pairs.iter().find(|p| p.old == old).unwrap().new_state)
    }

    fn registry(&self) -> StateRegistry {
        let states = self
            .pairs
            .iter()
            .map(|p| StateData {
                id: StateId(p.old),
                owner_country_id: if self.controlled_by_us.contains(&p.old) { US } else { THEM },
                neighbors: self
                    .old_edges
                    .iter()
                    .filter_map(|&(a, b)| if a == p.old { Some(StateId(b)) } else if b == p.old { Some(StateId(a)) } else { None })
                    .collect(),
                ..Default::default()
            })
            .collect();
        StateRegistry::build(states)
    }

    fn run(&self) {
        // 新: 1ステート=1プロヴィンス(1セル)。ステート配置は対応表から作る。
        let map = map_from(self.rows);
        let provs: Vec<(u32, i32, i32)> = self.pairs.iter().map(|p| (p.province, p.cell.0, p.cell.1)).collect();
        let st: Vec<(u32, Vec<u32>)> = self.pairs.iter().map(|p| (p.new_state, vec![p.province])).collect();
        let st_refs: Vec<(u32, &[u32])> = st.iter().map(|(a, b)| (*a, b.as_slice())).collect();
        let (_w, g) = setup(&map, &provs, &st_refs, Coverage::World);

        // (1) 同じ隣接関係: 新グラフの辺を対応表で旧IDへ変換した集合が、手で書いた旧の辺と一致する。
        let mut new_edges: BTreeSet<(usize, usize)> = BTreeSet::new();
        for a in g.node_ids() {
            for (b, _) in g.neighbors(a).unwrap() {
                let (x, y) = (self.old_of(a), self.old_of(b));
                new_edges.insert((x.min(y), x.max(y)));
            }
        }
        let old_edges: BTreeSet<(usize, usize)> = self.old_edges.iter().map(|&(a, b)| (a.min(b), a.max(b))).collect();
        assert_eq!(new_edges, old_edges, "the fixture expresses the same adjacency in both models");

        // (2) 同じ起点・同じ通行条件で探索し、到達集合を対応表で変換して比較する。
        let old_reached: BTreeSet<usize> =
            connected_states_from_capital(StateId(self.capital), US, &self.registry()).into_iter().map(|s| s.0).collect();
        let passable: Vec<u32> = self.controlled_by_us.iter().map(|o| self.province_of(*o).0).collect();
        let r = reach(&g, &[self.province_of(self.capital)], &pass(&g, &passable)).unwrap();
        let new_reached: BTreeSet<usize> = r.reached_ids().into_iter().map(|p| self.old_of(p)).collect();
        assert_eq!(new_reached, old_reached, "reach sets agree after translating through the table");

        // (3) ステート要約: 全域到達のステートを旧IDへ変換した集合が旧の到達集合と一致し、他は到達不能(1プロヴィンスなので一部は無い)。
        let sum = summarize_by_state(&g, &r).unwrap();
        assert_invariants(&sum, &g);
        let full: BTreeSet<usize> = self.pairs.iter().filter(|p| sum.state(MapStateId(p.new_state)).unwrap().kind == StateReachKind::Full).map(|p| p.old).collect();
        assert_eq!(full, old_reached);
        for p in &self.pairs {
            let k = sum.state(self.state_of_old(p.old)).unwrap().kind;
            assert!(matches!(k, StateReachKind::Full | StateReachKind::Unreachable), "one province per state: no partial/unknown, got {k:?}");
        }
    }
}

fn pair(old: usize, province: u32, new_state: u32, cell: (i32, i32)) -> Pair {
    Pair { old, province, new_state, cell }
}

#[test]
fn old_small_case_connected_line() {
    OldCase {
        rows: &["####"],
        pairs: vec![pair(0, 100, 500, (0, 0)), pair(1, 210, 501, (1, 0)), pair(2, 305, 502, (2, 0)), pair(3, 411, 503, (3, 0))],
        old_edges: vec![(0, 1), (1, 2), (2, 3)],
        controlled_by_us: vec![0, 1, 2, 3],
        capital: 0,
    }
    .run();
}

#[test]
fn old_small_case_disconnected_by_sea() {
    // 海で分断された2群。首都側 {0,1,2} だけ到達、{3,4} は別の群。
    OldCase {
        rows: &["###.##"],
        pairs: vec![
            pair(0, 7, 90, (0, 0)),
            pair(1, 3, 91, (1, 0)),
            pair(2, 9, 92, (2, 0)),
            pair(3, 1, 93, (4, 0)),
            pair(4, 5, 94, (5, 0)),
        ],
        old_edges: vec![(0, 1), (1, 2), (3, 4)],
        controlled_by_us: vec![0, 1, 2, 3, 4],
        capital: 0,
    }
    .run();
}

#[test]
fn old_small_case_blocked_by_another_controller() {
    // 3番目の州を他国が支配 → 旧は通行不可。新は通行可能集合から外す。先の州は着けない。
    OldCase {
        rows: &["#####"],
        pairs: vec![
            pair(0, 20, 600, (0, 0)),
            pair(1, 21, 601, (1, 0)),
            pair(2, 22, 602, (2, 0)),
            pair(3, 23, 603, (3, 0)),
            pair(4, 24, 604, (4, 0)),
        ],
        old_edges: vec![(0, 1), (1, 2), (2, 3), (3, 4)],
        controlled_by_us: vec![0, 1, 3, 4],
        capital: 0,
    }
    .run();
}

#[test]
fn old_small_case_cycle_with_a_tail() {
    // 三角形(0,1,2: (0,0)(1,0)(0,1) は互いに隣接)+ 尾 3: (2,0) は (1,0) に隣接。
    OldCase {
        rows: &["###", "#.."],
        pairs: vec![pair(0, 31, 700, (0, 0)), pair(1, 30, 701, (1, 0)), pair(2, 33, 702, (0, 1)), pair(3, 32, 703, (2, 0))],
        old_edges: vec![(0, 1), (0, 2), (1, 2), (1, 3)],
        controlled_by_us: vec![0, 1, 2, 3],
        capital: 2,
    }
    .run();
}

#[test]
fn old_small_case_capital_not_controlled_gives_empty() {
    // 旧: 首都が自国支配でなければ空集合。新: 起点が通行不可なら入力不正(黙って通行可能にしない)。
    let case = OldCase {
        rows: &["###"],
        pairs: vec![pair(0, 40, 800, (0, 0)), pair(1, 41, 801, (1, 0)), pair(2, 42, 802, (2, 0))],
        old_edges: vec![(0, 1), (1, 2)],
        controlled_by_us: vec![1, 2],
        capital: 0,
    };
    assert!(connected_states_from_capital(StateId(0), US, &case.registry()).is_empty());
    let map = map_from(case.rows);
    let provs: Vec<(u32, i32, i32)> = case.pairs.iter().map(|p| (p.province, p.cell.0, p.cell.1)).collect();
    let (_w, g) = setup(&map, &provs, &[(800, &[40]), (801, &[41]), (802, &[42])], Coverage::World);
    let e = reach(&g, &[ProvinceId(40)], &pass(&g, &[41, 42])).unwrap_err();
    assert_eq!(e, ReachError::InvalidSources(vec![(ProvinceId(40), SourceProblem::NotPassable)]));
}

// ---------------------------------------------------------------- 実データ(検証用ワールド: 29プロヴィンス・3ステート)

fn real() -> (HexMap, WorldPreviewData) {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let w = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).unwrap();
    (map, w)
}

const STATE_20: [u32; 7] = [18109, 18308, 18309, 18310, 18311, 18499, 18500];
const STATE_21: [u32; 13] = [17269, 17486, 17487, 17694, 17695, 17696, 17904, 17905, 17906, 18111, 18112, 18312, 18502];
const STATE_22: [u32; 6] = [17266, 17267, 17268, 17484, 17485, 17693];
const UNASSIGNED: [u32; 3] = [17903, 18110, 18501];

fn ids(v: &[ProvinceId]) -> Vec<u32> {
    v.iter().map(|x| x.0).collect()
}

/// テスト内の独立した総当たり: 既知の辺だけ(範囲外の陸は通らない)で、`allowed` を通って起点から着けるプロヴィンス。
fn independent_known_reach(w: &WorldPreviewData, allowed: &HashSet<u32>, source: u32) -> BTreeSet<u32> {
    const OFFSETS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
    let cell_of: HashMap<(i32, i32), u32> = w.model.provinces().iter().map(|d| ((d.cell.q, d.cell.r), d.id.0)).collect();
    let src = *cell_of.iter().find(|(_, id)| **id == source).unwrap().0;
    let mut seen = HashSet::new();
    let mut stack = vec![src];
    let mut out = BTreeSet::new();
    while let Some(c) = stack.pop() {
        if !seen.insert(c) {
            continue;
        }
        out.insert(cell_of[&c]);
        for (dq, dr) in OFFSETS {
            let n = (c.0 + dq, c.1 + dr);
            if let Some(id) = cell_of.get(&n)
                && allowed.contains(id)
                && !seen.contains(&n)
            {
                stack.push(n);
            }
        }
    }
    out
}

#[test]
fn real_a_states_20_and_22_only_state_20_is_full_and_state_22_is_unknown() {
    let (map, w) = real();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    assert_eq!(g.node_count(), 29, "no ProvinceId is issued for out-of-scope land");
    assert_eq!(ids(w.model.provinces_of(s(21)).unwrap()), STATE_21.to_vec(), "membership comes from the layout file");
    let p = ExplicitPassable::from_states(&g, &[s(20), s(22)]).unwrap();
    assert_eq!(p.len(), 13);
    let r = reach(&g, &[ProvinceId(18309)], &p).unwrap();
    let sum = summarize_by_state(&g, &r).unwrap();
    assert_invariants(&sum, &g);
    assert_eq!(ids(&r.reached_ids()), STATE_20.to_vec());
    let k = kinds(&sum);
    assert_eq!(k[&s(20)], StateReachKind::Full);
    assert_eq!(k[&s(21)], StateReachKind::Unreachable);
    assert_eq!(k[&s(22)], StateReachKind::Unknown, "reachable only through out-of-scope land: unknown, not unreachable");
    let c20 = sum.state(s(20)).unwrap().counts;
    let c21 = sum.state(s(21)).unwrap().counts;
    let c22 = sum.state(s(22)).unwrap().counts;
    assert_eq!((c20.total, c20.reachable), (7, 7));
    assert_eq!((c21.total, c21.unreachable_not_passable, c21.unreachable_no_route), (13, 13, 0));
    assert_eq!((c22.total, c22.reachable, c22.unknown, c22.unreachable()), (6, 0, 6, 0));
    let u = sum.unassigned();
    assert_eq!((u.total, u.unreachable_not_passable, u.reachable), (3, 3, 0), "unassigned provinces are not passable here and in no state's count");
    assert_eq!(ids(&r.unknown_ids()), STATE_22.to_vec());
    // 独立した総当たり
    let allowed: HashSet<u32> = STATE_20.iter().chain(STATE_22.iter()).copied().collect();
    assert_eq!(independent_known_reach(&w, &allowed, 18309).into_iter().collect::<Vec<_>>(), STATE_20.to_vec());
    // モデル由来の要約と一致
    assert_eq!(summarize_model(&w.model, &r).unwrap(), sum);
}

#[test]
fn real_b_all_but_state_21_the_unassigned_provinces_bridge_to_state_22() {
    let (map, w) = real();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    let p = ExplicitPassable::all_nodes(&g).without(&ExplicitPassable::from_states(&g, &[s(21)]).unwrap());
    assert_eq!(p.len(), 16, "7 + 6 + 3 unassigned");
    let r = reach(&g, &[ProvinceId(18309)], &p).unwrap();
    let sum = summarize_by_state(&g, &r).unwrap();
    assert_invariants(&sum, &g);
    let mut expected: Vec<u32> = STATE_20.iter().chain(STATE_22.iter()).chain(UNASSIGNED.iter()).copied().collect();
    expected.sort();
    assert_eq!(ids(&r.reached_ids()), expected);
    assert_eq!(r.unknown_count(), 0);
    let k = kinds(&sum);
    assert_eq!((k[&s(20)], k[&s(21)], k[&s(22)]), (StateReachKind::Full, StateReachKind::Unreachable, StateReachKind::Full));
    let u = sum.unassigned();
    assert_eq!((u.total, u.reachable), (3, 3));
    // 未所属3件はどのステートの件数にも入らない(20+21+22 = 26、未所属 3)
    assert_eq!(sum.assigned_totals().total, 26);
    // 独立した総当たり(既知の辺だけ)
    let allowed: HashSet<u32> = expected.iter().copied().collect();
    assert_eq!(independent_known_reach(&w, &allowed, 18309).into_iter().collect::<Vec<_>>(), expected);
    // 橋渡しの確認: 未所属を許可しない(= 条件A)と、ステート22は不明になる
    let a = ExplicitPassable::from_states(&g, &[s(20), s(22)]).unwrap();
    let ra = reach(&g, &[ProvinceId(18309)], &a).unwrap();
    assert_eq!(summarize_by_state(&g, &ra).unwrap().state(s(22)).unwrap().kind, StateReachKind::Unknown);
    assert_eq!(summarize_model(&w.model, &r).unwrap(), sum);
}

#[test]
fn real_out_of_scope_land_gets_no_province_and_is_not_applicable() {
    let (map, w) = real();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    let r = reach(&g, &[ProvinceId(18309)], &ExplicitPassable::all_nodes(&g)).unwrap();
    let known: HashSet<(i32, i32)> = w.model.provinces().iter().map(|d| (d.cell.q, d.cell.r)).collect();
    let oos = map.cells().iter().find(|c| !c.terrain.is_water() && !known.contains(&(c.q, c.r))).expect("the map has land outside the preview");
    assert_eq!(
        r.reachability_of_cell(&g, &map, HexCoord::new(oos.q, oos.r)),
        Reachability::NotApplicable(NotApplicableReason::OutOfScopeLand)
    );
    assert_eq!(g.node_count(), 29);
    assert_eq!(summarize_by_state(&g, &r).unwrap().grand_totals().total, 29);
}

// ---------------------------------------------------------------- 測定専用入力(小さな地図で規則と件数を確認。大規模測定は含めない)

fn small_map() -> HexMap {
    map_from(&["##########", "#####.####", "##########", "####..####", "##########", "#.########", "##########", "##########"])
}

fn land_count(rows: &[&str]) -> usize {
    rows.iter().map(|r| r.chars().filter(|c| *c == '#').count()).sum()
}

#[test]
fn the_measurement_input_has_the_stated_counts_and_rule() {
    let rows = ["##########", "#####.####", "##########", "####..####", "##########", "#.########", "##########", "##########"];
    let map = small_map();
    let rule = MeasureRule { block: 4, state_base: 1000, unassigned_modulus: 7 };
    let (defs, facts) = land_definitions(&map, rule);
    let land = land_count(&rows);
    assert_eq!((facts.land_cells, facts.provinces, defs.len()), (land, land, land));
    // ProvinceId は 0.. の連番で、(r, q) 順。1プロヴィンス=1セル。
    for (i, d) in defs.iter().enumerate() {
        assert_eq!(d.id, ProvinceId(i as u32));
        assert_eq!(d.cells.len(), 1);
    }
    let cells: Vec<(i32, i32)> = defs.iter().map(|d| (d.cells[0].r, d.cells[0].q)).collect();
    let mut sorted = cells.clone();
    sorted.sort();
    assert_eq!(cells, sorted);
    // 未所属は ProvinceId % 7 == 0 のものだけ
    let unassigned = defs.iter().filter(|d| d.state.is_none()).count();
    assert_eq!(unassigned, facts.unassigned);
    assert!(defs.iter().all(|d| d.state.is_none() == d.id.0.is_multiple_of(7)));
    // 同じステートのセルは同じ 4×4 の格子に入る
    let mut by_state: BTreeMap<MapStateId, BTreeSet<(i32, i32)>> = BTreeMap::new();
    for d in &defs {
        if let Some(st) = d.state {
            by_state.entry(st).or_default().insert((d.cells[0].r.div_euclid(4), d.cells[0].q.div_euclid(4)));
        }
    }
    assert_eq!(by_state.len(), facts.states);
    assert!(by_state.values().all(|b| b.len() == 1), "one block per state");
    assert!(by_state.keys().all(|k| k.0 >= 1000));
    // 決定的(2回作って同じ)
    assert_eq!(land_definitions(&map, rule).0, defs);
}

#[test]
fn the_measurement_input_of_the_real_map_matches_the_recorded_counts_without_building_the_graph() {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let (defs, facts) = land_definitions(&map, MeasureRule::DEFAULT);
    assert_eq!((facts.land_cells, facts.provinces), (35221, 35221));
    assert_eq!(defs.len(), 35221);
    assert_eq!((facts.states, facts.unassigned), (573, 364));
    // 正式な全世界の区画定義(ProvinceTableFile)は作らず、どこにも書かない。
    assert_eq!(defs[0].id, ProvinceId(0));
}

#[test]
fn agents_are_deterministic_and_the_search_really_runs_over_the_declared_input() {
    let map = small_map();
    let rule = MeasureRule { block: 4, state_base: 1000, unassigned_modulus: 7 };
    let (defs, facts) = land_definitions(&map, rule);
    let g = ProvinceGraph::from_definitions(&defs, &map, Coverage::World).unwrap();
    assert_eq!(g.node_count(), facts.provinces);
    for kind in [LoadKind::Split, LoadKind::Heavy] {
        for n in [3usize, 5] {
            let a = make_agents(&g, kind, n);
            assert_eq!(a, make_agents(&g, kind, n), "agents are generated deterministically");
            assert_eq!(a.len(), n);
            if kind == LoadKind::Split {
                // 帯は互いに素で、全ノードを覆う
                let all: Vec<ProvinceId> = a.iter().flat_map(|x| x.passable.iter().copied()).collect();
                let set: BTreeSet<ProvinceId> = all.iter().copied().collect();
                assert_eq!(all.len(), set.len(), "split bands are disjoint");
                assert_eq!(set, g.node_ids().collect::<BTreeSet<_>>());
            }
            // 1日相当の1巡(bench と同じ手順)。探索が実際に行われた = 各主体の起点から、通行可能部分の連結成分の全体に着く。
            let mut searched = 0usize;
            let passes: Vec<ExplicitPassable> = a.iter().map(|x| ExplicitPassable::new(&g, x.passable.iter().copied()).unwrap()).collect();
            let results: Vec<Reach> = a.iter().zip(&passes).map(|(x, p)| reach(&g, &[x.source], p).unwrap()).collect();
            for ((x, p), r) in a.iter().zip(&passes).zip(&results) {
                let comp = g.components_among(p).into_iter().find(|c| c.contains(&x.source)).unwrap();
                assert_eq!(r.reached_ids(), comp, "the search from the source covers exactly its passable component");
                assert!(r.reached_count() >= 1);
                searched += r.reached_count();
                // 到達集合は通行可能集合の部分集合
                assert!(r.reached_ids().iter().all(|id| p.passable(*id)));
            }
            assert!(searched > n, "more than the sources themselves were reached");
            let sums: Vec<StateReachSummary> = results.iter().map(|r| summarize_by_state(&g, r).unwrap()).collect();
            for (sum, r) in sums.iter().zip(&results) {
                assert_invariants(sum, &g);
                assert_eq!(sum.grand_totals().reachable, r.reached_count(), "the state summary counts exactly the reached provinces");
                assert_eq!(sum.unassigned().total, facts.unassigned);
                assert_eq!(sum.states().len(), facts.states);
            }
        }
    }
}
