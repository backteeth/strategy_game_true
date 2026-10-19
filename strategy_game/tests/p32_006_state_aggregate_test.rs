//! P32-006 (M3): ステートの地理集計とステート間隣接の導出のテスト(GUIなし)。
//! 小さな合成地図(期待値を手で確認できるもの)と、実地図+検証用ワールド設定の両方を使う。

use std::collections::BTreeSet;
use strategy_game::hexmap::data::{
    Climate, HexMap, Terrain, Vegetation, load_from_path, parse_hex_map, river_neighbor,
};
use strategy_game::hexmap::world_view::{
    LoadedWorld, catalog_names, describe_cell, state_summary_text,
};
use strategy_game::world::*;

type Cell<'a> = (i32, i32, &'a str, &'a str, &'a str);

const LAND: (&str, &str, &str) = ("plains", "C", "grassland");

fn map_from(cells: &[Cell], rivers: &[(i32, i32, u8)]) -> HexMap {
    let c: Vec<String> = cells
        .iter()
        .map(|(q, r, t, cl, v)| {
            format!(r#"{{"q":{q},"r":{r},"terrain":"{t}","climate":"{cl}","vegetation":"{v}"}}"#)
        })
        .collect();
    let rv: Vec<String> = rivers
        .iter()
        .map(|(q, r, d)| format!(r#"{{"q":{q},"r":{r},"direction":{d}}}"#))
        .collect();
    let json = format!(
        r#"{{"schemaVersion":9,"kind":"hex-atlas-game-map",
        "grid":{{"orientation":"pointy","size":2.5,"origin":{{"x":2.0,"y":2.5}},"imageWidth":200,"imageHeight":100,
        "extent":{{"minQ":-8,"maxQ":8,"minR":0,"maxR":8}}}},"cells":[{}],"rivers":[{}]}}"#,
        c.join(","),
        rv.join(",")
    );
    parse_hex_map(json.as_bytes()).expect("synthetic map must parse")
}

/// 同じ陸(平原)の横一列 `q = from..=to`(行 r)。
fn row(r: i32, from: i32, to: i32) -> Vec<Cell<'static>> {
    (from..=to)
        .map(|q| (q, r, LAND.0, LAND.1, LAND.2))
        .collect()
}

type StateSpec<'a> = (u32, &'a [(i32, i32)]);

/// 表を地図から生成し(`in_table`が`Some`ならその座標だけを載せたPartial)、座標でステートへ割り当てる。
fn world(map: &HexMap, in_table: Option<&[(i32, i32)]>, states: &[StateSpec]) -> WorldModel {
    let full = generate_initial_province_table(map, Coverage::World);
    let mut table = full.clone();
    if let Some(keep) = in_table {
        table.coverage = Coverage::Partial;
        table.provinces.retain(|p| keep.contains(&(p.q, p.r)));
    }
    let id_at = |c: &(i32, i32)| {
        full.provinces
            .iter()
            .find(|p| (p.q, p.r) == *c)
            .unwrap_or_else(|| panic!("no land province at {c:?}"))
            .id
    };
    let layout = StateLayoutFile {
        format_version: WORLD_FORMAT_VERSION,
        states: states
            .iter()
            .map(|(id, cells)| StateEntry {
                id: *id,
                name_key: format!("test.{id}"),
                provinces: cells.iter().map(id_at).collect(),
                representative: None,
                allow_disconnected: true,
            })
            .collect(),
    };
    WorldModel::build(table, layout).expect("synthetic world must build")
}

fn derive(model: &WorldModel, map: &HexMap) -> WorldGeoDerived {
    derive_world_geo(model, map).expect("derivation must succeed")
}
fn s(id: u32) -> MapStateId {
    MapStateId(id)
}

// ---------------------------------------------------------------- 集計

#[test]
fn fixture_summaries_match_hand_computed_values() {
    // P32-004のフィクスチャ: 北={(1,0),(2,0),(3,0),(1,1)} 南={(3,1),(2,2),(3,2)}
    let map = parse_hex_map(include_bytes!("fixtures/p32_004/small_map.json")).unwrap();
    let table = generate_initial_province_table(&map, Coverage::World);
    let model = WorldModel::build(
        table,
        parse_state_layout(include_str!("fixtures/p32_004/small_layout.ron")).unwrap(),
    )
    .unwrap();
    let d = derive(&model, &map);
    let n = d.summary(s(10)).unwrap();
    assert_eq!(n.province_count, 4);
    assert_eq!(
        (
            n.terrain_count(Terrain::Plains),
            n.terrain_count(Terrain::Hills)
        ),
        (3, 1)
    );
    assert_eq!(n.climate_count(Climate::Temperate), 4);
    assert_eq!(
        (
            n.vegetation_count(Vegetation::Grassland),
            n.vegetation_count(Vegetation::Forest),
            n.vegetation_count(Vegetation::None),
            n.vegetation_count(Vegetation::Unclassified)
        ),
        (1, 2, 1, 0)
    );
    let so = d.summary(s(11)).unwrap();
    assert_eq!(so.province_count, 3);
    assert_eq!(
        (
            so.terrain_count(Terrain::Plains),
            so.terrain_count(Terrain::Plateau)
        ),
        (2, 1)
    );
    assert_eq!(so.climate_count(Climate::Arid), 3);
    assert_eq!(
        (
            so.vegetation_count(Vegetation::Grassland),
            so.vegetation_count(Vegetation::None)
        ),
        (1, 2)
    );
    assert!(n.is_consistent() && so.is_consistent());
    // 北と南の共有辺は (3,0)-(3,1) の1本で、そこに河川がある
    assert_eq!(
        d.adjacency_between(s(10), s(11)),
        AdjacencyLookup::Found(&StateAdjacency {
            a: s(10),
            b: s(11),
            shared_edges: 1,
            shared_river_edges: 1
        })
    );
}

#[test]
fn none_vs_unclassified_and_plateau_vs_highland_are_distinct() {
    let cells: Vec<Cell> = vec![
        (0, 0, "plateau", "C", "none"),
        (1, 0, "highland", "unclassified", "unclassified"),
        (2, 0, "plateau", "C", "unclassified"),
        (3, 0, "highland", "C", "none"),
    ];
    let map = map_from(&cells, &[]);
    let model = world(&map, None, &[(1, &[(0, 0), (1, 0), (2, 0), (3, 0)])]);
    let sum = derive(&model, &map).summary(s(1)).unwrap().clone();
    assert_eq!(sum.province_count, 4);
    assert_eq!(
        (
            sum.terrain_count(Terrain::Plateau),
            sum.terrain_count(Terrain::Highland)
        ),
        (2, 2)
    );
    assert_eq!(
        (
            sum.vegetation_count(Vegetation::None),
            sum.vegetation_count(Vegetation::Unclassified)
        ),
        (2, 2)
    );
    assert_eq!(
        (
            sum.climate_count(Climate::Temperate),
            sum.climate_count(Climate::Unclassified)
        ),
        (3, 1)
    );
    assert!(sum.is_consistent());
}

#[test]
fn unassigned_out_of_scope_and_water_never_enter_any_summary() {
    // 横一列: 0,1 = ステート1 / 2 = 未割当(表にある) / 3 = 表にない陸 / 4 = 湖 / 5 = ステート2
    let mut cells = row(0, 0, 3);
    cells.push((4, 0, "lake", "unclassified", "unclassified"));
    cells.push((5, 0, LAND.0, LAND.1, LAND.2));
    let map = map_from(&cells, &[]);
    let in_table = [(0, 0), (1, 0), (2, 0), (5, 0)];
    let model = world(
        &map,
        Some(&in_table),
        &[(1, &[(0, 0), (1, 0)]), (2, &[(5, 0)])],
    );
    let d = derive(&model, &map);
    assert_eq!(d.summary(s(1)).unwrap().province_count, 2);
    assert_eq!(d.summary(s(2)).unwrap().province_count, 1);
    // 合計は所属済みプロヴィンス数と一致(未割当1・範囲外1・湖1は合算されない)
    let total: usize = d.summaries().iter().map(|x| x.province_count).sum();
    assert_eq!(total, 3);
    assert_eq!(d.summaries().len(), 2, "no phantom state for unassigned");
    assert_eq!(d.completeness().unassigned_provinces, 1);
    assert_eq!(d.completeness().out_of_scope_land_cells, 1);
    for sm in d.summaries() {
        assert_eq!(sm.terrain_count(Terrain::Lake), 0);
    }
}

#[test]
fn results_do_not_depend_on_input_order() {
    let mut cells = row(0, -2, 3);
    cells.extend(row(1, -2, 3));
    let map = map_from(&cells, &[(0, 0, 5), (1, 1, 0), (2, 0, 0)]);
    let states: Vec<StateSpec> = vec![
        (7, &[(-2, 0), (-1, 0), (-2, 1)]),
        (3, &[(0, 0), (1, 0), (-1, 1), (0, 1)]),
        (9, &[(2, 0), (3, 0), (1, 1), (2, 1), (3, 1)]),
    ];
    let base = derive(&world(&map, None, &states), &map);
    // 並び替えたステート配置・表から導出しても完全に同じ
    let full = generate_initial_province_table(&map, Coverage::World);
    let mut t2 = full.clone();
    t2.provinces.reverse();
    let mut layout = StateLayoutFile {
        format_version: WORLD_FORMAT_VERSION,
        states: states
            .iter()
            .rev()
            .map(|(id, cs)| StateEntry {
                id: *id,
                name_key: format!("test.{id}"),
                provinces: cs
                    .iter()
                    .rev()
                    .map(|c| full.provinces.iter().find(|p| (p.q, p.r) == *c).unwrap().id)
                    .collect(),
                representative: None,
                allow_disconnected: true,
            })
            .collect(),
    };
    layout.states.reverse();
    layout.states.reverse();
    let shuffled = derive(&WorldModel::build(t2, layout).unwrap(), &map);
    assert_eq!(base, shuffled);
    assert_eq!(base.adjacencies(), shuffled.adjacencies());
}

// ---------------------------------------------------------------- 隣接

#[test]
fn internal_edges_are_not_adjacency_and_shared_edges_are_counted_once() {
    // A = 3セルの横一列 (0..2, r=0)、B = その下の2セル (r=1)。A内部の辺は隣接に含めない。
    let mut cells = row(0, 0, 2);
    cells.extend(row(1, -1, 0));
    let map = map_from(&cells, &[]);
    let a: &[(i32, i32)] = &[(0, 0), (1, 0), (2, 0)];
    let b: &[(i32, i32)] = &[(-1, 1), (0, 1)];
    let model = world(&map, None, &[(1, a), (2, b)]);
    let d = derive(&model, &map);
    // 総当たりで「異なるステートのセル組」を独立に数える(無向・1組1回)
    let mut pairs = BTreeSet::new();
    for &(q, r) in a {
        for dir in 0..6u8 {
            let n = river_neighbor(q, r, dir);
            if b.contains(&n) {
                pairs.insert(((q, r).min(n), (q, r).max(n)));
            }
        }
    }
    assert!(!pairs.is_empty());
    match d.adjacency_between(s(1), s(2)) {
        AdjacencyLookup::Found(adj) => assert_eq!(adj.shared_edges, pairs.len()),
        other => panic!("{other:?}"),
    }
    // 1つのステートしか無ければ隣接は無い(内部の辺は含まれない)
    let single = world(
        &map,
        None,
        &[(1, &[(0, 0), (1, 0), (2, 0), (-1, 1), (0, 1)])],
    );
    assert!(derive(&single, &map).adjacencies().is_empty());
}

#[test]
fn adjacency_is_symmetric_and_stored_once() {
    let mut cells = row(0, 0, 1);
    cells.extend(row(1, 0, 1));
    let map = map_from(&cells, &[]);
    let model = world(
        &map,
        None,
        &[(5, &[(0, 0), (1, 0)]), (2, &[(0, 1), (1, 1)])],
    );
    let d = derive(&model, &map);
    assert_eq!(d.adjacencies().len(), 1);
    let adj = d.adjacencies()[0];
    assert!(adj.a < adj.b, "stored once with a < b");
    assert_eq!(
        d.adjacency_between(s(5), s(2)),
        d.adjacency_between(s(2), s(5))
    );
    assert_eq!(d.neighbors_of(s(5))[0].0, s(2));
    assert_eq!(d.neighbors_of(s(2))[0].0, s(5));
    assert_eq!(d.neighbors_of(s(5))[0].1, d.neighbors_of(s(2))[0].1);
}

#[test]
fn states_two_cells_apart_are_not_adjacent_even_across_gaps() {
    // 0 = A / 1 = 隙間 / 2 = B。隙間が 水域・未割当・表にない陸 のどれでも、A-Bは隣接しない。
    for gap in [
        ("lake", "unclassified", "unclassified"),
        LAND, // 隙間が陸: 未割当(または範囲外)
    ] {
        let cells = vec![
            (0, 0, LAND.0, LAND.1, LAND.2),
            (1, 0, gap.0, gap.1, gap.2),
            (2, 0, LAND.0, LAND.1, LAND.2),
        ];
        let map = map_from(&cells, &[]);
        let two: &[StateSpec] = &[(1, &[(0, 0)]), (2, &[(2, 0)])];
        // 隙間が陸の場合: (a) 未割当として表にある (b) 表に無い
        let variants: Vec<Option<Vec<(i32, i32)>>> = if gap.0 == "lake" {
            vec![None]
        } else {
            vec![None, Some(vec![(0, 0), (2, 0)])]
        };
        for v in variants {
            let model = world(&map, v.as_deref(), two);
            let d = derive(&model, &map);
            assert!(d.adjacencies().is_empty(), "gap {gap:?} {v:?}");
            assert!(matches!(
                d.adjacency_between(s(1), s(2)),
                AdjacencyLookup::NotRecorded { .. }
            ));
        }
    }
    // 隙間のセルが第三のステートなら A-C と C-B は隣接するが、A-B は隣接しない。
    let map = map_from(&row(0, 0, 2), &[]);
    let model = world(
        &map,
        None,
        &[(1, &[(0, 0)]), (3, &[(1, 0)]), (2, &[(2, 0)])],
    );
    let d = derive(&model, &map);
    assert!(matches!(
        d.adjacency_between(s(1), s(3)),
        AdjacencyLookup::Found(_)
    ));
    assert!(matches!(
        d.adjacency_between(s(3), s(2)),
        AdjacencyLookup::Found(_)
    ));
    assert!(matches!(
        d.adjacency_between(s(1), s(2)),
        AdjacencyLookup::NotRecorded { complete: true }
    ));
}

#[test]
fn negative_q_adjacency_is_derived_correctly() {
    let mut cells = row(2, -4, -1);
    cells.extend(row(3, -5, -2));
    let map = map_from(&cells, &[]);
    let model = world(
        &map,
        None,
        &[
            (1, &[(-4, 2), (-3, 2), (-2, 2), (-1, 2)]),
            (2, &[(-5, 3), (-4, 3), (-3, 3), (-2, 3)]),
        ],
    );
    let d = derive(&model, &map);
    let adj = match d.adjacency_between(s(1), s(2)) {
        AdjacencyLookup::Found(a) => *a,
        o => panic!("{o:?}"),
    };
    // r=2 の (q,2) の南西(-1,+1)=(q-1,3)と南東(0,+1)=(q,3)が r=3 の行にある
    let mut expected = 0;
    for q in -4..=-1 {
        for d in [4u8, 5] {
            let n = river_neighbor(q, 2, d);
            if (-5..=-2).contains(&n.0) && n.1 == 3 {
                expected += 1;
            }
        }
    }
    assert_eq!(adj.shared_edges, expected);
    assert!(expected >= 6);
}

#[test]
fn a_state_with_separated_parts_uses_only_real_shared_edges() {
    // A = (0,0) と (4,0) に離れて存在、B = (1,0) と (3,0)、間の (2,0) は未割当。
    let map = map_from(&row(0, 0, 4), &[]);
    let model = world(
        &map,
        None,
        &[(1, &[(0, 0), (4, 0)]), (2, &[(1, 0), (3, 0)])],
    );
    let d = derive(&model, &map);
    match d.adjacency_between(s(1), s(2)) {
        // (0,0)-(1,0) と (3,0)-(4,0) の2辺。(1,0)-(3,0) は同じステート内の離れた部分で隣接しない。
        AdjacencyLookup::Found(a) => assert_eq!(a.shared_edges, 2),
        o => panic!("{o:?}"),
    }
    assert_eq!(d.summary(s(1)).unwrap().province_count, 2);
}

// ---------------------------------------------------------------- 河川

#[test]
fn only_rivers_on_shared_edges_are_counted_once_each() {
    // A: (0,0),(1,0)  B: (2,0),(3,0)  C: 未割当(4,0)
    let rivers = [
        (0, 0, 0u8), // A内部((0,0)-(1,0)): 数えない
        (1, 0, 0),   // A|B の共有辺 (1,0)-(2,0)
        (2, 0, 3),   // 同じ辺の反対側表現: 二重計上しない
        (3, 0, 0),   // B|未割当 (3,0)-(4,0): 共有辺ではない
        (2, 0, 5),   // B から南東(2,1)は水域側: 隣接ではない辺の河川
    ];
    let mut cells = row(0, 0, 4);
    cells.push((2, 1, "lake", "unclassified", "unclassified"));
    let map = map_from(&cells, &rivers);
    let model = world(
        &map,
        None,
        &[(1, &[(0, 0), (1, 0)]), (2, &[(2, 0), (3, 0)])],
    );
    let d = derive(&model, &map);
    match d.adjacency_between(s(1), s(2)) {
        AdjacencyLookup::Found(a) => {
            assert_eq!(a.shared_edges, 1);
            assert_eq!(
                a.shared_river_edges, 1,
                "two records of one edge = one river edge"
            );
            assert!(a.shared_river_edges <= a.shared_edges);
        }
        o => panic!("{o:?}"),
    }
    // 河川が無い共有辺は0
    let map2 = map_from(&row(0, 0, 3), &[(0, 0, 0)]);
    let model2 = world(
        &map2,
        None,
        &[(1, &[(0, 0), (1, 0)]), (2, &[(2, 0), (3, 0)])],
    );
    match derive(&model2, &map2).adjacency_between(s(1), s(2)) {
        AdjacencyLookup::Found(a) => assert_eq!((a.shared_edges, a.shared_river_edges), (1, 0)),
        o => panic!("{o:?}"),
    }
}

#[test]
fn rivers_do_not_cut_adjacency() {
    let map = map_from(&row(0, 0, 1), &[(0, 0, 0)]);
    let model = world(&map, None, &[(1, &[(0, 0)]), (2, &[(1, 0)])]);
    match derive(&model, &map).adjacency_between(s(1), s(2)) {
        AdjacencyLookup::Found(a) => assert_eq!((a.shared_edges, a.shared_river_edges), (1, 1)),
        o => panic!("river must not remove adjacency: {o:?}"),
    }
}

// ---------------------------------------------------------------- 完全性・不正データ

#[test]
fn completeness_distinguishes_partial_world_with_unassigned_and_complete() {
    // (a) 完全: World・未割当なし・範囲外なし。隣接が無い組は「存在しない」と確定できる。
    let map = map_from(&row(0, 0, 2), &[]);
    let complete = derive(
        &world(
            &map,
            None,
            &[(1, &[(0, 0)]), (2, &[(1, 0)]), (3, &[(2, 0)])],
        ),
        &map,
    );
    assert!(complete.completeness().is_complete());
    assert_eq!(
        complete.adjacency_between(s(1), s(3)),
        AdjacencyLookup::NotRecorded { complete: true }
    );
    // (b) Worldでも未割当が残る: 不完全
    let unassigned = derive(&world(&map, None, &[(1, &[(0, 0)]), (2, &[(2, 0)])]), &map);
    let c = unassigned.completeness();
    assert_eq!(
        (
            c.coverage,
            c.unassigned_provinces,
            c.out_of_scope_land_cells
        ),
        (Coverage::World, 1, 0)
    );
    assert!(!c.is_complete());
    assert_eq!(
        unassigned.adjacency_between(s(1), s(2)),
        AdjacencyLookup::NotRecorded { complete: false }
    );
    // (c) Partial(表に無い陸あり): 不完全。全て所属していても完全とは扱わない
    let partial = derive(
        &world(
            &map,
            Some(&[(0, 0), (1, 0)]),
            &[(1, &[(0, 0)]), (2, &[(1, 0)])],
        ),
        &map,
    );
    let c = partial.completeness();
    assert_eq!(
        (
            c.coverage,
            c.unassigned_provinces,
            c.out_of_scope_land_cells
        ),
        (Coverage::Partial, 0, 1)
    );
    assert!(!c.is_complete());
    // 確認できた共有辺は、不完全でも増減させない
    assert!(
        matches!(partial.adjacency_between(s(1), s(2)), AdjacencyLookup::Found(a) if a.shared_edges == 1)
    );
}

#[test]
fn invalid_geography_is_an_error_not_a_partial_result() {
    let map = map_from(&row(0, 0, 2), &[]);
    let model = world(&map, None, &[(1, &[(0, 0)]), (2, &[(1, 0), (2, 0)])]);
    // (1,0) が湖に変わった地図: 水域を指すプロヴィンスがあるため導出しない
    let mut cells = row(0, 0, 2);
    cells[1] = (1, 0, "lake", "unclassified", "unclassified");
    let changed = map_from(&cells, &[]);
    match derive_world_geo(&model, &changed) {
        Err(DeriveError::InvalidWorld(issues)) => {
            assert!(
                issues
                    .iter()
                    .any(|i| i.code == IssueCode::ProvinceCellIsWater)
            );
        }
        Ok(_) => panic!("must not silently aggregate invalid data"),
    }
}

#[test]
fn stale_results_are_detected() {
    let map = map_from(&row(0, 0, 2), &[]);
    let model = world(&map, None, &[(1, &[(0, 0)]), (2, &[(1, 0), (2, 0)])]);
    let d = derive(&model, &map);
    assert!(d.is_current_for(&model, &map));
    // 属性が変わった地図 → 古い
    let mut cells = row(0, 0, 2);
    cells[0].3 = "B";
    let edited = map_from(&cells, &[]);
    assert!(!d.is_current_for(&model, &edited));
    // 所属が変わった → 古い
    let moved = world(&map, None, &[(1, &[(0, 0), (1, 0)]), (2, &[(2, 0)])]);
    assert!(!d.is_current_for(&moved, &map));
}

// ---------------------------------------------------------------- 実地図の検証用ワールド

fn real_map() -> HexMap {
    load_from_path("assets/data/hex_atlas_map.json").unwrap()
}

#[test]
fn preview_world_summaries_and_adjacency_match_independent_counts() {
    let map = real_map();
    let data = load_world_preview(
        "assets/data/world/preview/provinces.ron",
        "assets/data/world/preview/states.ron",
        &map,
    )
    .unwrap();
    let d = &data.derived;
    let m = &data.model;
    // 検証用fixtureの確認値(汎用処理には固定していない)
    let counts: Vec<(u32, usize)> = d
        .summaries()
        .iter()
        .map(|x| (x.state.0, x.province_count))
        .collect();
    assert_eq!(counts, vec![(20, 7), (21, 13), (22, 6)]);
    // 独立な数え直し: プロヴィンス表から1つずつ地図を引いて数える
    for sm in d.summaries() {
        assert!(sm.is_consistent());
        let mut terrain = [0usize; 8];
        let mut veg = [0usize; 5];
        for p in m
            .provinces()
            .iter()
            .filter(|p| m.state_of(p.id) == Some(sm.state))
        {
            let cell = map.cell(p.cell.q, p.cell.r).unwrap();
            terrain[Terrain::ALL
                .iter()
                .position(|t| *t == cell.terrain)
                .unwrap()] += 1;
            veg[Vegetation::ALL
                .iter()
                .position(|t| *t == cell.vegetation)
                .unwrap()] += 1;
        }
        assert_eq!(sm.terrain, terrain);
        assert_eq!(sm.vegetation, veg);
    }
    // 隣接: 全プロヴィンスの全方向を総当たりし、異なるステートの組を(無向・1辺1回で)数える
    let mut shared: std::collections::BTreeMap<(u32, u32), (usize, usize)> = Default::default();
    for p in m.provinces() {
        let Some(a) = m.state_of(p.id) else { continue };
        for dir in 0..6u8 {
            let n = river_neighbor(p.cell.q, p.cell.r, dir);
            let Some(np) = m.province_at(n.0, n.1) else {
                continue;
            };
            let Some(b) = m.state_of(np.id) else { continue };
            if a == b || (p.cell.q, p.cell.r) > n {
                continue;
            }
            let river = map
                .river_edges()
                .iter()
                .any(|e| (e.q, e.r, e.direction) == (p.cell.q, p.cell.r, dir));
            let k = if a.0 < b.0 { (a.0, b.0) } else { (b.0, a.0) };
            let e = shared.entry(k).or_default();
            e.0 += 1;
            if river {
                e.1 += 1;
            }
        }
    }
    let got: std::collections::BTreeMap<(u32, u32), (usize, usize)> = d
        .adjacencies()
        .iter()
        .map(|a| ((a.a.0, a.b.0), (a.shared_edges, a.shared_river_edges)))
        .collect();
    assert_eq!(got, shared);
    assert!(!got.is_empty());
    for (e, r) in got.values() {
        assert!(r <= e);
    }
    // 20と22の間には未割当・水域があり直接の共有辺が無い(「確認できた範囲では」)。完全ではないので確定しない。
    assert_eq!(
        d.adjacency_between(s(20), s(22)),
        AdjacencyLookup::NotRecorded { complete: false }
    );
    let c = d.completeness();
    assert_eq!((c.coverage, c.unassigned_provinces), (Coverage::Partial, 3));
    assert!(c.out_of_scope_land_cells > 0 && !c.is_complete());
    assert!(d.is_current_for(m, &map));
}

#[test]
fn preview_text_shows_summary_only_for_assigned_cells() {
    let map = real_map();
    let data = load_world_preview(
        "assets/data/world/preview/provinces.ron",
        "assets/data/world/preview/states.ron",
        &map,
    )
    .unwrap();
    let names = catalog_names(&data);
    let w = LoadedWorld { data, names };
    // 所属済み(ステートC=22): 集計・隣接・Partialの注記
    let a = describe_cell(&w, &map, 83, 78);
    assert!(
        a.contains("ステート集計(地理)") && a.contains("プロヴィンス数: 6"),
        "{a}"
    );
    assert!(
        a.contains("地形:") && a.contains("気候:") && a.contains("植生:"),
        "{a}"
    );
    assert!(
        a.contains("隣接ステート") && a.contains("共有辺") && a.contains("河川辺"),
        "{a}"
    );
    assert!(a.contains("検証用の部分データ"), "{a}");
    // 別ステート(B=21)では内容が切り替わる
    let b = describe_cell(&w, &map, 86, 78);
    assert!(b.contains("プロヴィンス数: 13"), "{b}");
    assert_ne!(a, b);
    // 未割当・範囲外・水域では集計を出さない(前の選択を引きずらない)
    let un = w.data.model.province(ProvinceId(18110)).unwrap().cell;
    let u = describe_cell(&w, &map, un.q, un.r);
    assert!(u.contains("未割当") && !u.contains("ステート集計"), "{u}");
    let oos = map
        .cells()
        .iter()
        .find(|c| {
            !c.terrain.is_water()
                && w.data.model.province_at(c.q, c.r).is_none()
                && (c.q - 83).abs() < 8
        })
        .unwrap();
    let o = describe_cell(&w, &map, oos.q, oos.r);
    assert!(
        o.contains("検証範囲外") && !o.contains("ステート集計"),
        "{o}"
    );
    let sea = map
        .cells()
        .iter()
        .find(|c| c.terrain.is_water() && (c.q - 83).abs() < 6 && (c.r - 81).abs() < 6)
        .unwrap();
    let wt = describe_cell(&w, &map, sea.q, sea.r);
    assert!(wt.contains("水域") && !wt.contains("ステート集計"), "{wt}");
    // 要約テキストの単体呼び出しも、存在しないステートでは空
    assert!(state_summary_text(&w, MapStateId(999)).is_empty());
}
