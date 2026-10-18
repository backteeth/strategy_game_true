//! P32-001: ヘックスマップの読み込み・検証・座標変換・選択のテスト。
//! 実データ(`assets/data/hex_atlas_map.json`)に対する期待値は「今回の入力の確認値」であり、
//! 汎用ローダーの仕様ではない(ローダーは件数・集計を固定しない)。

use strategy_game::hexmap::data::*;
use strategy_game::hexmap::geometry::*;
use strategy_game::hexmap::render::{HexViewMode, build_fill_mesh, plan_chunks};

fn real() -> HexMap {
    load_from_path(DEFAULT_MAP_PATH).expect("real map must load")
}

fn doc(cells: &str, rivers: &str) -> String {
    format!(
        r#"{{"schemaVersion":9,"kind":"hex-atlas-game-map",
        "grid":{{"orientation":"pointy","size":2.5,"origin":{{"x":2.0,"y":2.5}},
        "imageWidth":100,"imageHeight":100,"extent":{{"minQ":-3,"maxQ":5,"minR":0,"maxR":4}}}},
        "cells":[{cells}],"rivers":[{rivers}]}}"#
    )
}
fn cell(q: i32, r: i32, t: &str, c: &str, v: &str) -> String {
    format!(r#"{{"q":{q},"r":{r},"terrain":"{t}","climate":"{c}","vegetation":"{v}"}}"#)
}
fn parse(s: &str) -> Result<HexMap, HexMapError> {
    parse_hex_map(s.as_bytes())
}

#[test]
fn real_map_counts_and_attributes() {
    let m = real();
    assert_eq!(m.cells().len(), 96_760);
    assert_eq!(m.rivers().len(), 3_297);
    let c = m.attribute_counts();
    let t = |x: Terrain| c.terrain[Terrain::ALL.iter().position(|y| *y == x).unwrap()];
    assert_eq!(t(Terrain::Sea), 62_055);
    assert_eq!(t(Terrain::Lake), 194);
    assert_eq!(t(Terrain::Plains), 17_276);
    assert_eq!(t(Terrain::Hills), 7_553);
    assert_eq!(t(Terrain::Plateau), 4_793);
    assert_eq!(t(Terrain::Highland), 2_997);
    assert_eq!(t(Terrain::Mountain), 1_892);
    assert_eq!(t(Terrain::Unclassified), 0);
    assert_eq!(c.climate, [4_996, 7_523, 11_686, 8_337, 1_945, 62_273]);
    assert_eq!(c.vegetation, [4_355, 5_886, 17_487, 5_529, 63_503]);
    assert_eq!(c.land_cells, 34_511);
    assert_eq!(c.land_climate_unclassified, 24);
    assert_eq!(c.land_vegetation_unclassified, 1_254);
}

#[test]
fn real_map_has_negative_q_sparse_grid_and_river_sources() {
    let m = real();
    assert!(m.cells().iter().any(|c| c.q < 0));
    let e = m.grid.extent;
    let slots = (e.max_q - e.min_q + 1) as usize * (e.max_r - e.min_r + 1) as usize;
    assert!(
        m.cells().len() < slots,
        "grid is sparse, not a full rectangle"
    );
    for r in m.rivers() {
        assert!(m.cell(r.q, r.r).is_some());
    }
    for (i, c) in m.cells().iter().enumerate() {
        assert_eq!(m.cell_index(c.q, c.r), Some(i as u32));
    }
}

#[test]
fn real_map_centres_inside_image_and_bounds_cover_all_vertices() {
    // 制作ツールは「セル中心が画像内」のセルを出力している(頂点は最大で半セル分はみ出す)。
    // 描画側はセルを削らず、頂点を含む外接矩形を全体表示の基準にする。
    let m = real();
    let (w, h) = (m.grid.image_width as f64, m.grid.image_height as f64);
    let half_w = m.grid.size * 3f64.sqrt() / 2.0;
    for c in m.cells() {
        let (x, y) = m.grid.center_image(c.q, c.r);
        assert!(
            x >= -1e-6 && x <= w + 1e-6 && y >= -1e-6 && y <= h + 1e-6,
            "{:?}",
            (c.q, c.r)
        );
    }
    assert!(
        image_overflow_cells(&m) > 0,
        "boundary-cell vertices do stick out of the image"
    );
    let (x0, y0, x1, y1) = world_bounds(&m).unwrap();
    assert!(x0 >= -half_w - 1e-6 && x1 <= w + half_w + 1e-6);
    assert!(-y1 >= -m.grid.size - 1e-6 && -y0 <= h + m.grid.size + 1e-6);
    for c in m.cells() {
        for i in 0..6 {
            let (x, y) = m.grid.corner_world(c.q, c.r, i);
            assert!(x >= x0 && x <= x1 && y >= y0 && y <= y1);
        }
    }
}

#[test]
fn real_map_center_roundtrip_and_pick_every_cell() {
    let m = real();
    for c in m.cells() {
        let (x, y) = m.grid.center_world(c.q, c.r);
        assert_eq!(m.grid.axial_from_world(x, y), (c.q, c.r));
        assert_eq!(pick_cell_world(&m, x, y), m.cell_index(c.q, c.r));
    }
}

#[test]
fn interior_points_pick_their_own_cell() {
    let m = real();
    let inradius = m.grid.size * 3f64.sqrt() / 2.0;
    let mut samples: Vec<_> = m
        .cells()
        .iter()
        .filter(|c| c.q < 0)
        .take(5)
        .copied()
        .collect();
    samples.extend(m.cells().iter().take(5).copied());
    samples.extend(m.cells().iter().rev().take(5).copied());
    for c in samples {
        let (cx, cy) = m.grid.center_world(c.q, c.r);
        for k in 0..12 {
            let a = (k as f64 * 30.0).to_radians();
            let (x, y) = (cx + 0.9 * inradius * a.cos(), cy + 0.9 * inradius * a.sin());
            assert_eq!(
                m.grid.axial_from_world(x, y),
                (c.q, c.r),
                "cell {:?} dir {k}",
                (c.q, c.r)
            );
        }
        for k in 0..6 {
            let (vx, vy) = m.grid.corner_world(c.q, c.r, k);
            let (x, y) = (cx + (vx - cx) * 0.95, cy + (vy - cy) * 0.95);
            assert_eq!(m.grid.axial_from_world(x, y), (c.q, c.r));
        }
    }
}

#[test]
fn y_flip_is_single_inverse_pair() {
    let (wx, wy) = image_to_world(12.5, 40.0);
    assert_eq!((wx, wy), (12.5, -40.0));
    assert_eq!(world_to_image(wx, wy), (12.5, 40.0));
}

#[test]
fn edge_midpoints_resolve_deterministically_to_self_or_neighbour() {
    let m = real();
    let c = m.cells()[m.cells().len() / 2];
    for k in 0..6 {
        let (ax, ay) = m.grid.corner_world(c.q, c.r, k);
        let (bx, by) = m.grid.corner_world(c.q, c.r, (k + 1) % 6);
        let mid = ((ax + bx) / 2.0, (ay + by) / 2.0);
        let first = m.grid.axial_from_world(mid.0, mid.1);
        assert_eq!(first, m.grid.axial_from_world(mid.0, mid.1));
        let (dq, dr) = (first.0 - c.q, first.1 - c.r);
        assert!(dq.abs().max(dr.abs()).max((dq + dr).abs()) <= 1);
    }
}

#[test]
fn missing_cells_and_outside_are_not_selected() {
    let m = real();
    let e = m.grid.extent;
    let mut missing = None;
    'o: for r in e.min_r..=e.max_r {
        for q in e.min_q..=e.max_q {
            if m.cell(q, r).is_none() {
                missing = Some((q, r));
                break 'o;
            }
        }
    }
    let (q, r) = missing.expect("sparse grid has holes");
    let (x, y) = m.grid.center_world(q, r);
    assert_eq!(pick_cell_world(&m, x, y), None);
    assert_eq!(pick_cell_world(&m, -5000.0, 5000.0), None);
    assert_eq!(pick_cell_world(&m, 1e12, -1e12), None);
    assert_eq!(pick_cell_world(&m, f64::NAN, 0.0), None);
    assert_eq!(pick_cell_world(&m, 0.0, f64::INFINITY), None);
}

#[test]
fn chunk_plan_covers_each_cell_exactly_once_and_meshes_build() {
    let m = real();
    let chunks = plan_chunks(&m);
    let mut seen = vec![false; m.cells().len()];
    for ch in &chunks {
        for &i in ch {
            assert!(!seen[i as usize]);
            seen[i as usize] = true;
        }
    }
    assert!(seen.iter().all(|s| *s));
    assert!(
        chunks.len() < 400,
        "chunks must be coarse, got {}",
        chunks.len()
    );
    let mesh = build_fill_mesh(&m, &chunks[0], HexViewMode::Terrain);
    assert_eq!(mesh.count_vertices(), chunks[0].len() * 6);
}

// ------------------------------------------------------------ 合成データ

#[test]
fn preserves_negative_q_unclassified_none_plateau_highland() {
    let cells = [
        cell(-2, 1, "plateau", "C", "none"),
        cell(-1, 1, "highland", "unclassified", "unclassified"),
        cell(0, 0, "sea", "unclassified", "unclassified"),
        cell(1, 0, "unclassified", "A", "grassland"),
    ]
    .join(",");
    let m = parse(&doc(&cells, "")).unwrap();
    assert_eq!(m.cell(-2, 1).unwrap().terrain, Terrain::Plateau);
    assert_eq!(m.cell(-1, 1).unwrap().terrain, Terrain::Highland);
    assert_eq!(m.cell(-2, 1).unwrap().vegetation, Vegetation::None);
    assert_eq!(m.cell(-1, 1).unwrap().vegetation, Vegetation::Unclassified);
    assert_eq!(m.cell(-1, 1).unwrap().climate, Climate::Unclassified);
    assert_eq!(m.cell(1, 0).unwrap().terrain, Terrain::Unclassified);
    assert!(m.cell(2, 2).is_none());
}

#[test]
fn water_cells_with_unclassified_attributes_load() {
    let m = parse(&doc(
        &cell(0, 0, "lake", "unclassified", "unclassified"),
        "",
    ))
    .unwrap();
    assert_eq!(m.cells().len(), 1);
}

#[test]
fn rejects_malformed_json() {
    assert!(matches!(parse("{ not json"), Err(HexMapError::Parse(_))));
    assert!(matches!(parse(""), Err(HexMapError::Parse(_))));
}

#[test]
fn rejects_wrong_kind_schema_and_orientation() {
    let ok = doc(&cell(0, 0, "sea", "unclassified", "unclassified"), "");
    assert!(matches!(
        parse(&ok.replace("hex-atlas-game-map", "other")),
        Err(HexMapError::WrongKind(_))
    ));
    assert!(matches!(
        parse(&ok.replace("\"schemaVersion\":9", "\"schemaVersion\":8")),
        Err(HexMapError::UnsupportedSchema(8))
    ));
    assert!(matches!(
        parse(&ok.replace("pointy", "flat")),
        Err(HexMapError::UnsupportedOrientation(_))
    ));
}

#[test]
fn rejects_unknown_attribute_values() {
    for (t, c, v, field) in [
        ("swamp", "A", "none", "terrain"),
        ("sea", "Z", "none", "climate"),
        ("sea", "A", "savanna", "vegetation"),
    ] {
        let err = parse(&doc(&cell(0, 0, t, c, v), "")).unwrap_err();
        assert!(
            matches!(err, HexMapError::UnknownValue { field: f, .. } if f == field),
            "{err}"
        );
    }
}

#[test]
fn rejects_duplicate_and_out_of_extent_cells() {
    let dup = [
        cell(0, 0, "sea", "A", "none"),
        cell(0, 0, "lake", "A", "none"),
    ]
    .join(",");
    assert!(matches!(
        parse(&doc(&dup, "")),
        Err(HexMapError::DuplicateCell { q: 0, r: 0 })
    ));
    assert!(matches!(
        parse(&doc(&cell(99, 0, "sea", "A", "none"), "")),
        Err(HexMapError::CellOutOfExtent { .. })
    ));
}

#[test]
fn rejects_invalid_size_origin_and_dimensions() {
    let ok = doc(&cell(0, 0, "sea", "A", "none"), "");
    for bad in ["\"size\":0", "\"size\":-2.5", "\"size\":1e999"] {
        assert!(parse(&ok.replace("\"size\":2.5", bad)).is_err(), "{bad}");
    }
    assert!(matches!(
        parse(&ok.replace("\"imageWidth\":100", "\"imageWidth\":0")),
        Err(HexMapError::InvalidGrid(_))
    ));
    assert!(matches!(
        parse(&ok.replace("\"minQ\":-3", "\"minQ\":9")),
        Err(HexMapError::InvalidGrid(_))
    ));
    assert!(parse(&ok.replace("\"q\":0", "\"q\":0.5")).is_err());
}

#[test]
fn rejects_invalid_rivers() {
    let c = cell(0, 0, "plains", "C", "grassland");
    let ok = parse(&doc(&c, r#"{"q":0,"r":0,"direction":4}"#)).unwrap();
    assert_eq!(ok.rivers().len(), 1);
    for (r, why) in [
        (r#"{"q":0,"r":0,"direction":6}"#, "range"),
        (r#"{"q":0,"r":0,"direction":-1}"#, "range"),
        (r#"{"q":3,"r":3,"direction":0}"#, "missing source"),
        (
            r#"{"q":0,"r":0,"direction":1},{"q":0,"r":0,"direction":1}"#,
            "duplicate",
        ),
    ] {
        assert!(
            matches!(parse(&doc(&c, r)), Err(HexMapError::InvalidRiver { .. })),
            "{why}"
        );
    }
}

#[test]
fn missing_file_reports_path() {
    let e = load_from_path("assets/data/does_not_exist.json").unwrap_err();
    assert!(matches!(e, HexMapError::Io { .. }));
    assert!(e.to_string().contains("does_not_exist.json"));
}

#[test]
fn asset_copy_matches_original_if_present() {
    let orig = std::path::Path::new("../hex_atlas/hex-atlas-map.json");
    if let Ok(o) = std::fs::read(orig) {
        assert_eq!(o, std::fs::read(DEFAULT_MAP_PATH).unwrap());
    }
}
