//! P32-002: 河川directionの定義(ユーザー確認済み)・辺の位置・共有辺の正規化・実データ整合性。
//! 方向: 画面上が北、右辺=0で反時計回り 0=東,1=北東,2=北西,3=西,4=南西,5=南東。

use strategy_game::hexmap::data::*;
use strategy_game::hexmap::geometry::*;
use strategy_game::hexmap::render::{build_river_mesh, plan_chunks};

fn unit_grid() -> HexGrid {
    // 単純な原点(0,0)とsize=10。セル(0,0)の中心は画像座標(0,0)。
    HexGrid {
        size: 10.0,
        origin_x: 0.0,
        origin_y: 0.0,
        image_width: 1000,
        image_height: 1000,
        extent: Extent {
            min_q: -5,
            max_q: 5,
            min_r: -5,
            max_r: 5,
        },
    }
}

fn near(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
}

fn same_segment(a: ((f64, f64), (f64, f64)), b: ((f64, f64), (f64, f64))) -> bool {
    (near(a.0, b.0) && near(a.1, b.1)) || (near(a.0, b.1) && near(a.1, b.0))
}

#[test]
fn six_direction_neighbours_match_spec() {
    let expect = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
    for (d, (dq, dr)) in expect.iter().enumerate() {
        assert_eq!(river_neighbor(10, 5, d as u8), (10 + dq, 5 + dr), "dir {d}");
        assert_eq!(opposite_direction(d as u8), ((d + 3) % 6) as u8);
    }
    assert_eq!(river_neighbor(10, 5, 0), (11, 5));
}

#[test]
fn edge_positions_for_all_six_directions_in_known_geometry() {
    // size=10 の六角形の辺の両端(画像座標、Y下向き、中心(0,0))を手計算で固定。
    let h = 3f64.sqrt() * 5.0;
    let expect_image: [((f64, f64), (f64, f64)); 6] = [
        ((h, -5.0), (h, 5.0)),      // 0 東
        ((0.0, -10.0), (h, -5.0)),  // 1 北東
        ((-h, -5.0), (0.0, -10.0)), // 2 北西
        ((-h, 5.0), (-h, -5.0)),    // 3 西
        ((0.0, 10.0), (-h, 5.0)),   // 4 南西
        ((h, 5.0), (0.0, 10.0)),    // 5 南東
    ];
    let g = unit_grid();
    for d in 0..6u8 {
        let e = expect_image[d as usize];
        let (ea, eb) = (image_to_world(e.0.0, e.0.1), image_to_world(e.1.0, e.1.1));
        assert!(
            same_segment(river_edge_world(&g, 0, 0, d), (ea, eb)),
            "dir {d}"
        );
    }
}

#[test]
fn edge_midpoints_face_the_right_compass_side_with_north_up_on_screen() {
    let g = unit_grid();
    let c = g.center_world(0, 0);
    let mid = |d: u8| {
        let (a, b) = river_edge_world(&g, 0, 0, d);
        ((a.0 + b.0) / 2.0 - c.0, (a.1 + b.1) / 2.0 - c.1)
    };
    // ワールドはY上向き: 北=+y(画面上側)。
    let e = mid(0);
    assert!(e.0 > 8.0 && e.1.abs() < 1e-9, "east {e:?}");
    let ne = mid(1);
    assert!(ne.0 > 0.0 && ne.1 > 0.0, "north-east {ne:?}");
    let nw = mid(2);
    assert!(nw.0 < 0.0 && nw.1 > 0.0, "north-west {nw:?}");
    let w = mid(3);
    assert!(w.0 < -8.0 && w.1.abs() < 1e-9, "west {w:?}");
    let sw = mid(4);
    assert!(sw.0 < 0.0 && sw.1 < 0.0, "south-west {sw:?}");
    let se = mid(5);
    assert!(se.0 > 0.0 && se.1 < 0.0, "south-east {se:?}");
    // 辺の中点は、隣接セル中心との中点(辺は隣接セルとの境界)。
    for d in 0..6u8 {
        let (nq, nr) = river_neighbor(0, 0, d);
        let n = g.center_world(nq, nr);
        let m = mid(d);
        assert!(
            near(
                (m.0 + c.0, m.1 + c.1),
                ((c.0 + n.0) / 2.0, (c.1 + n.1) / 2.0)
            ),
            "dir {d}"
        );
    }
}

#[test]
fn edge_matches_spec_formula_in_image_coordinates() {
    // 仕様の検算式: theta=-d*60°, 端点= center + size*(cos(theta∓30°), sin(theta∓30°)) (画像座標)
    use std::f64::consts::PI;
    let g = unit_grid();
    for d in 0..6u8 {
        let th = -(d as f64) * PI / 3.0;
        let c = g.center_image(2, -1);
        let p = |t: f64| (c.0 + 10.0 * t.cos(), c.1 + 10.0 * t.sin());
        let (pa, pb) = (p(th - PI / 6.0), p(th + PI / 6.0));
        let want = (image_to_world(pa.0, pa.1), image_to_world(pb.0, pb.1));
        assert!(
            same_segment(river_edge_world(&g, 2, -1, d), want),
            "dir {d}"
        );
    }
}

#[test]
fn opposite_side_representation_shares_the_same_endpoints() {
    let g = unit_grid();
    for d in 0..6u8 {
        for (q, r) in [(0, 0), (3, -2), (-4, 1)] {
            let (nq, nr) = river_neighbor(q, r, d);
            assert!(
                same_segment(
                    river_edge_world(&g, q, r, d),
                    river_edge_world(&g, nq, nr, opposite_direction(d))
                ),
                "dir {d} at {q},{r}"
            );
        }
    }
}

fn doc(rivers: &str) -> String {
    let cells: Vec<String> = [(0, 0), (1, 0), (0, 1), (1, 1)]
        .iter()
        .map(|(q, r)| {
            format!(
                r#"{{"q":{q},"r":{r},"terrain":"plains","climate":"C","vegetation":"grassland"}}"#
            )
        })
        .collect();
    format!(
        r#"{{"schemaVersion":9,"kind":"hex-atlas-game-map",
        "grid":{{"orientation":"pointy","size":2.5,"origin":{{"x":2.0,"y":2.5}},"imageWidth":100,"imageHeight":100,
        "extent":{{"minQ":-3,"maxQ":5,"minR":0,"maxR":4}}}},"cells":[{}],"rivers":[{rivers}]}}"#,
        cells.join(",")
    )
}

#[test]
fn both_sided_records_collapse_to_one_edge_and_order_does_not_matter() {
    let a =
        r#"{"q":0,"r":0,"direction":0},{"q":1,"r":0,"direction":3},{"q":0,"r":0,"direction":5}"#;
    let b =
        r#"{"q":0,"r":0,"direction":5},{"q":1,"r":0,"direction":3},{"q":0,"r":0,"direction":0}"#;
    let ma = parse_hex_map(doc(a).as_bytes()).unwrap();
    let mb = parse_hex_map(doc(b).as_bytes()).unwrap();
    assert_eq!(ma.rivers().len(), 3, "input records are preserved");
    assert_eq!(ma.river_edges().len(), 2, "drawn edges are deduplicated");
    assert_eq!(ma.river_edges(), mb.river_edges());
    let shared = ma.river_edges().iter().find(|e| e.direction == 0).unwrap();
    assert_eq!((shared.q, shared.r, shared.records), (0, 0, 2));
}

#[test]
fn every_direction_pair_normalizes_to_one_edge() {
    for d in 0..6u8 {
        let (nq, nr) = river_neighbor(0, 0, d);
        let o = opposite_direction(d);
        // 隣接先が存在するセルだけで確認するため、(0,0)から見て存在する(1,0),(0,1)側のみ両側表現が作れる。
        if ![(1, 0), (0, 1)].contains(&(nq, nr)) {
            continue;
        }
        let rivers =
            format!(r#"{{"q":0,"r":0,"direction":{d}}},{{"q":{nq},"r":{nr},"direction":{o}}}"#);
        let m = parse_hex_map(doc(&rivers).as_bytes()).unwrap();
        assert_eq!((m.rivers().len(), m.river_edges().len()), (2, 1), "dir {d}");
    }
}

#[test]
fn all_six_directions_load_and_missing_neighbour_is_kept_but_distinguished() {
    // (0,0)から6方向。(1,0)(0,1)は存在するセル、他は隣接先なし(地図境界)。
    let all = (0..6)
        .map(|d| format!(r#"{{"q":0,"r":0,"direction":{d}}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let m = parse_hex_map(doc(&all).as_bytes()).unwrap();
    assert_eq!(m.river_edges().len(), 6);
    assert_eq!(
        m.river_edges().iter().filter(|e| e.neighbor_exists).count(),
        2
    );
    assert_eq!(
        m.river_edges()
            .iter()
            .filter(|e| !e.neighbor_exists)
            .count(),
        4
    );
    // 隣接先なしの辺もメッシュから削除されない。
    assert_eq!(build_river_mesh(&m, 0.5).count_vertices(), 6 * 4);
}

#[test]
fn invalid_river_input_is_rejected_and_missing_source_differs_from_missing_neighbour() {
    for bad in [
        r#"{"q":0,"r":0,"direction":6}"#,
        r#"{"q":0,"r":0,"direction":-1}"#,
        r#"{"q":0.5,"r":0,"direction":1}"#,
        r#"{"q":4,"r":4,"direction":1}"#,
        r#"{"q":0,"r":0,"direction":2},{"q":0,"r":0,"direction":2}"#,
    ] {
        assert!(parse_hex_map(doc(bad).as_bytes()).is_err(), "{bad}");
    }
    // 参照元は存在し、隣接先だけ無い → 正常
    assert!(parse_hex_map(doc(r#"{"q":0,"r":0,"direction":3}"#).as_bytes()).is_ok());
    // 参照元が無い → InvalidRiver
    assert!(matches!(
        parse_hex_map(doc(r#"{"q":3,"r":3,"direction":0}"#).as_bytes()),
        Err(HexMapError::InvalidRiver { .. })
    ));
}

#[test]
fn real_map_river_edges_are_consistent() {
    let m = load_from_path(DEFAULT_MAP_PATH).unwrap();
    let edges = m.river_edges();
    // 件数は固定せず、入力から独立に導出して照合する。
    let mut keys = std::collections::HashSet::new();
    for r in m.rivers() {
        let n = river_neighbor(r.q, r.r, r.direction);
        let k = if (r.q, r.r) <= n {
            (r.q, r.r, r.direction)
        } else {
            (n.0, n.1, opposite_direction(r.direction))
        };
        keys.insert(k);
        assert!(m.cell(r.q, r.r).is_some(), "source exists");
    }
    assert_eq!(edges.len(), keys.len());
    assert!(edges.len() <= m.rivers().len());
    let total: usize = edges.iter().map(|e| e.records as usize).sum();
    assert_eq!(total, m.rivers().len());
    for e in edges {
        let n = river_neighbor(e.q, e.r, e.direction);
        assert_eq!(e.neighbor_exists, m.cell(e.q, e.r).is_some() && m.cell(n.0, n.1).is_some());
    }
    assert_eq!(build_river_mesh(&m, 0.5).count_vertices(), edges.len() * 4);
}

#[test]
fn real_map_rivers_crossing_chunk_boundaries_are_kept_once() {
    let m = load_from_path(DEFAULT_MAP_PATH).unwrap();
    let chunks = plan_chunks(&m);
    let mut chunk_of = vec![0usize; m.cells().len()];
    for (ci, ch) in chunks.iter().enumerate() {
        for &i in ch {
            chunk_of[i as usize] = ci;
        }
    }
    let crossing = m
        .river_edges()
        .iter()
        .filter(|e| {
            let n = river_neighbor(e.q, e.r, e.direction);
            match (m.cell_index(e.q, e.r), m.cell_index(n.0, n.1)) {
                (Some(a), Some(b)) => chunk_of[a as usize] != chunk_of[b as usize],
                _ => false,
            }
        })
        .count();
    // 河川は全チャンクに跨る単一メッシュなので、跨ぐ辺があっても本数は変わらない。
    let mut seen = std::collections::HashSet::new();
    for e in m.river_edges() {
        assert!(seen.insert((e.q, e.r, e.direction)), "no duplicate edge");
    }
    println!(
        "edges crossing chunk boundaries: {crossing} of {}",
        m.river_edges().len()
    );
    assert!(crossing > 0, "real data has rivers across chunk boundaries");
    assert_eq!(
        build_river_mesh(&m, 0.5).count_vertices(),
        m.river_edges().len() * 4
    );
}

#[test]
fn river_width_is_bounded_in_overview_and_thin_in_closeup() {
    use strategy_game::hexmap::view::{river_width, river_zoom_level};
    let size = 2.5;
    let far = river_width(size, river_zoom_level(1.7));
    let near_ = river_width(size, river_zoom_level(0.05));
    assert!(far <= 0.4 * size + 1e-6);
    assert!(near_ < 0.2, "closeup width {near_}");
    assert_eq!(river_zoom_level(1.0), 0);
}
