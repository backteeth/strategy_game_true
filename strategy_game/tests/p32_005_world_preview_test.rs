//! P32-005 (M1): 検証用ワールド設定(小地域のステート配置)と、境界・所属表示データのテスト(GUIなし)。
//! 実地図 + `assets/data/world/preview/*.ron`(検証用。正式な世界設定ではない)と、
//! メモリ上の合成配置・既存フィクスチャ(`tests/fixtures/p32_004/`)を使う。

use std::collections::{BTreeSet, HashSet};
use strategy_game::hexmap::data::{HexMap, load_from_path, parse_hex_map, river_neighbor};
use strategy_game::hexmap::geometry::river_edge_world;
use strategy_game::hexmap::render::{
    HexViewMode, boundary_edge_color, build_edge_mesh, plan_chunks, vertex_colors_world,
};
use strategy_game::hexmap::world_view::{
    LoadedWorld, WorldLoadError, border_width, catalog_names, describe_cell, region_edge_width,
    state_legend, status_lines,
};
use strategy_game::world::*;

const PROVINCES: &str = "assets/data/world/preview/provinces.ron";
const STATES: &str = "assets/data/world/preview/states.ron";

fn real_map() -> HexMap {
    load_from_path("assets/data/hex_atlas_map.json").unwrap()
}
fn preview(map: &HexMap) -> WorldPreviewData {
    load_world_preview(PROVINCES, STATES, map).expect("preview world must load")
}
fn loaded(map: &HexMap) -> LoadedWorld {
    let data = preview(map);
    let names = catalog_names(&data);
    LoadedWorld { data, names }
}
fn state_entry(id: u32, provinces: &[u32]) -> StateEntry {
    StateEntry {
        id,
        name_key: format!("test.{id}"),
        provinces: provinces.to_vec(),
        representative: None,
        allow_disconnected: true,
    }
}
fn layout(states: Vec<StateEntry>) -> StateLayoutFile {
    StateLayoutFile {
        format_version: WORLD_FORMAT_VERSION,
        states,
    }
}
fn midpoint(e: ((f64, f64), (f64, f64))) -> (f64, f64) {
    ((e.0.0 + e.1.0) / 2.0, (e.0.1 + e.1.1) / 2.0)
}
fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("p32_005_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

// ---------------------------------------------------------------- 検証用RONの読み込みと検証

#[test]
fn preview_files_load_and_authoring_validation_succeeds() {
    let map = real_map();
    let d = preview(&map);
    assert_eq!(d.model.coverage(), Coverage::Partial);
    assert_eq!(d.model.states().len(), 3);
    assert_eq!(d.model.provinces().len(), 29);
    assert_eq!(d.report.unassigned_province_count, 3);
    assert!(d.report.is_ok());
    // Partialであること・全域の検証が完了していないことが報告に出る
    assert!(!d.report.world_coverage_verified);
    assert!(d.report.has(IssueCode::PartialCoverage));
    assert_eq!(
        d.report.severity_of(IssueCode::UnassignedProvince),
        Some(Severity::Warning)
    );
}

#[test]
fn preview_world_fails_game_start_validation() {
    let map = real_map();
    let d = preview(&map);
    let r = validate_against_geography(&d.model, &map, ValidationMode::GameStart);
    assert!(!r.is_ok());
    assert!(r.has(IssueCode::PartialCoverage));
    assert!(r.has(IssueCode::UnassignedProvince));
}

#[test]
fn ids_are_stored_in_the_file_and_stay_stable_across_reloads() {
    let map = real_map();
    let a = preview(&map);
    let b = preview(&map);
    assert_eq!(a.model, b.model);
    // ファイルに明示された値がそのまま使われる(読み込み時に生成されない)
    assert_eq!(a.model.province_at(83, 78).unwrap().id, ProvinceId(17266));
    assert_eq!(a.model.state_of(ProvinceId(17266)), Some(MapStateId(22)));
    // 全世界を網羅した場合の初回採番と一致する(将来の本番表と衝突しない)
    let full = generate_initial_province_table(&map, Coverage::World);
    for p in a.model.provinces() {
        let f = full
            .provinces
            .iter()
            .find(|x| x.q == p.cell.q && x.r == p.cell.r)
            .unwrap();
        assert_eq!(f.id, p.id.0);
    }
    assert_eq!(a.model.next_province_id() as usize, full.provinces.len());
    // 水域にはプロヴィンスもステートも無い
    for p in a.model.provinces() {
        assert!(!map.cell(p.cell.q, p.cell.r).unwrap().terrain.is_water());
    }
}

// ---------------------------------------------------------------- 所属・選択表示データ

#[test]
fn cell_status_distinguishes_assigned_unassigned_out_of_scope_and_water() {
    let map = real_map();
    let d = preview(&map);
    let m = &d.model;
    let (mut a, mut u, mut o, mut w) = (0, 0, 0, 0);
    for c in map.cells() {
        if (c.q - 83).abs() > 9 || (c.r - 81).abs() > 9 {
            continue;
        }
        match cell_status(m, &map, c.q, c.r) {
            CellStatus::Assigned { province, state } => {
                a += 1;
                assert_eq!(m.province_at(c.q, c.r).unwrap().id, province);
                assert_eq!(m.state_of(province), Some(state));
                assert!(m.provinces_of(state).unwrap().contains(&province));
            }
            CellStatus::Unassigned { province } => {
                u += 1;
                assert_eq!(m.state_of(province), None);
                assert!([17903, 18110, 18501].contains(&province.0));
            }
            CellStatus::OutOfScope => {
                o += 1;
                assert!(m.province_at(c.q, c.r).is_none());
                assert!(!c.terrain.is_water());
            }
            CellStatus::Water => {
                w += 1;
                assert!(c.terrain.is_water());
                assert!(m.province_at(c.q, c.r).is_none());
            }
            CellStatus::NoCell => unreachable!("cells() only returns existing cells"),
        }
    }
    assert_eq!((a, u), (26, 3));
    assert!(
        o > 0 && w > 0,
        "region has out-of-scope land ({o}) and water ({w})"
    );
    assert_eq!(cell_status(m, &map, 9999, 9999), CellStatus::NoCell);
}

#[test]
fn selection_text_matches_membership_and_never_invents_ids() {
    let map = real_map();
    let w = loaded(&map);
    let m = &w.data.model;
    // A: 所属済み → ID・ステートID・表示名
    let a = describe_cell(&w, &map, 83, 78);
    assert!(
        a.contains("プロヴィンス: 17266") && a.contains("MapState#22") && a.contains("所属済み"),
        "{a}"
    );
    assert!(
        a.contains("検証ステートC"),
        "name resolved from localization: {a}"
    );
    // B: 未割当 → ID あり、ステートなし
    let p = m
        .provinces()
        .iter()
        .find(|p| p.id == ProvinceId(18110))
        .unwrap();
    let b = describe_cell(&w, &map, p.cell.q, p.cell.r);
    assert!(
        b.contains("18110") && b.contains("未割当") && !b.contains("MapState#"),
        "{b}"
    );
    // C: 範囲外 → IDなし
    let oos = map
        .cells()
        .iter()
        .find(|c| {
            !c.terrain.is_water() && m.province_at(c.q, c.r).is_none() && (c.q - 83).abs() < 8
        })
        .unwrap();
    let c = describe_cell(&w, &map, oos.q, oos.r);
    assert!(
        c.contains("検証範囲外") && !c.contains("プロヴィンス:") && !c.contains("未割当"),
        "{c}"
    );
    // D: 水域
    let sea = map
        .cells()
        .iter()
        .find(|c| c.terrain.is_water() && (c.q - 83).abs() < 6 && (c.r - 81).abs() < 6)
        .unwrap();
    let d = describe_cell(&w, &map, sea.q, sea.r);
    assert!(
        d.contains("水域") && !d.contains("プロヴィンス:") && !d.contains("検証範囲外"),
        "{d}"
    );
    // 4状態の表示が互いに異なる
    let set: HashSet<&str> = [a.as_str(), b.as_str(), c.as_str(), d.as_str()]
        .into_iter()
        .collect();
    assert_eq!(set.len(), 4);
}

#[test]
fn state_colors_are_stable_and_fills_distinguish_categories() {
    assert_eq!(state_color(MapStateId(20)), state_color(MapStateId(20)));
    let cols: BTreeSet<String> = [20, 21, 22]
        .iter()
        .map(|i| format!("{:?}", state_color(MapStateId(*i))))
        .collect();
    assert_eq!(cols.len(), 3, "different states get different colours");
    // 値の固定(起動のたびに変わらないこと): HSL 黄金比色相
    let c = state_color(MapStateId(0));
    assert_eq!(c, state_color(MapStateId(0)));
    let map = real_map();
    let w = loaded(&map);
    let all: Vec<u32> = (0..map.cells().len() as u32).collect();
    let colors = vertex_colors_world(&map, &all, HexViewMode::State, Some(&w.data.model));
    let unassigned = strategy_game::hexmap::render::UNASSIGNED_FILL;
    let _ = unassigned;
    // 同じステートのセルは同じ色、未割当・範囲外・水域はステート色と異なる
    let color_of = |q, r| {
        let i = map.cell_index(q, r).unwrap() as usize;
        format!("{:?}", colors[i * 6])
    };
    assert_eq!(color_of(83, 78), color_of(84, 78), "same state (22)");
    assert_ne!(color_of(83, 78), color_of(86, 81));
    let un = w.data.model.province(ProvinceId(18110)).unwrap().cell;
    let un_col = color_of(un.q, un.r);
    for s in [20u32, 21, 22] {
        let p = w.data.model.provinces_of(MapStateId(s)).unwrap()[0];
        let cell = w.data.model.province(p).unwrap().cell;
        assert_ne!(color_of(cell.q, cell.r), un_col);
    }
    // 他モードは従来どおり(ワールドがあっても地形モードの色は変わらない)
    let terr = vertex_colors_world(&map, &all, HexViewMode::Terrain, Some(&w.data.model));
    assert_eq!(
        terr,
        strategy_game::hexmap::render::vertex_colors(&map, &all, HexViewMode::Terrain)
    );
    // ワールド設定が無いステートモードは地形色へフォールバック
    let none = vertex_colors_world(&map, &all, HexViewMode::State, None);
    assert_eq!(
        none,
        strategy_game::hexmap::render::vertex_colors(&map, &all, HexViewMode::State)
    );
}

#[test]
fn legend_lists_every_state_unassigned_out_of_scope_and_water() {
    let map = real_map();
    let w = loaded(&map);
    let l = state_legend(&w, &map);
    let text: Vec<&str> = l.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        text.iter()
            .any(|t| t.contains("検証ステートA") && t.contains("(7)"))
    );
    assert!(
        text.iter()
            .any(|t| t.contains("検証ステートB") && t.contains("(13)"))
    );
    assert!(
        text.iter()
            .any(|t| t.contains("検証ステートC") && t.contains("(6)"))
    );
    assert!(text.iter().any(|t| t.contains("未割当 (3)")));
    assert!(text.iter().any(|t| t.contains("検証範囲外")));
    assert!(text.iter().any(|t| t.contains("水域")));
}

// ---------------------------------------------------------------- 境界

/// 独立した期待値: 全陸プロヴィンスの全隣接ペアを総当たりし、異なるステート同士のペアを数える。
fn expected_state_border_pairs(w: &WorldModel, map: &HexMap) -> BTreeSet<((i32, i32), (i32, i32))> {
    let mut s = BTreeSet::new();
    for p in w.provinces() {
        for d in 0..6u8 {
            let n = river_neighbor(p.cell.q, p.cell.r, d);
            if let (Some(a), Some(np)) = (w.state_of(p.id), w.province_at(n.0, n.1))
                && let Some(b) = w.state_of(np.id)
                && a != b
                && map.cell(n.0, n.1).is_some()
            {
                let k = ((p.cell.q, p.cell.r), n);
                let k = if k.0 <= k.1 { k } else { (k.1, k.0) };
                s.insert(k);
            }
        }
    }
    s
}

#[test]
fn state_borders_match_independent_brute_force_and_are_emitted_once() {
    let map = real_map();
    let d = preview(&map);
    let borders: Vec<&BoundaryEdge> = d
        .boundaries
        .iter()
        .filter(|b| b.kind == BoundaryKind::StateBorder)
        .collect();
    let expected = expected_state_border_pairs(&d.model, &map);
    assert!(!expected.is_empty());
    let got: BTreeSet<((i32, i32), (i32, i32))> = borders
        .iter()
        .map(|b| {
            let n = river_neighbor(b.q, b.r, b.direction);
            let k = ((b.q, b.r), n);
            if k.0 <= k.1 { k } else { (k.1, k.0) }
        })
        .collect();
    assert_eq!(
        got, expected,
        "exactly the shared edges between different states"
    );
    assert_eq!(
        borders.len(),
        expected.len(),
        "one edge per shared edge, none duplicated"
    );
    for b in &borders {
        let n = river_neighbor(b.q, b.r, b.direction);
        let a = d
            .model
            .state_of(d.model.province_at(b.q, b.r).unwrap().id)
            .unwrap();
        let c = d
            .model
            .state_of(d.model.province_at(n.0, n.1).unwrap().id)
            .unwrap();
        assert_ne!(a, c, "a state's internal edge must not be a border");
        assert_eq!((b.state, b.other_state), (a, Some(c)));
        // 隣接方向と位置: 辺の中点は、両セル中心の中点
        let m = midpoint(river_edge_world(&map.grid, b.q, b.r, b.direction));
        let (c1, c2) = (
            map.grid.center_world(b.q, b.r),
            map.grid.center_world(n.0, n.1),
        );
        let mid = ((c1.0 + c2.0) / 2.0, (c1.1 + c2.1) / 2.0);
        assert!((m.0 - mid.0).abs() < 1e-9 && (m.1 - mid.1).abs() < 1e-9);
    }
    // 4種の辺が実データで実際に現れる(区別して扱っている)
    for kind in [
        BoundaryKind::ToUnassigned,
        BoundaryKind::ToOutOfScope,
        BoundaryKind::Coast,
    ] {
        assert!(d.boundaries.iter().any(|b| b.kind == kind), "{kind:?}");
    }
    assert!(
        boundary_edge_color(BoundaryKind::Coast).is_none(),
        "coast is not drawn"
    );
    assert!(boundary_edge_color(BoundaryKind::StateBorder).is_some());
}

#[test]
fn non_border_kinds_only_touch_the_right_neighbour_category() {
    let map = real_map();
    let d = preview(&map);
    for b in &d.boundaries {
        let n = river_neighbor(b.q, b.r, b.direction);
        let s = cell_status(&d.model, &map, n.0, n.1);
        let ok = match b.kind {
            BoundaryKind::StateBorder => matches!(s, CellStatus::Assigned { .. }),
            BoundaryKind::ToUnassigned => matches!(s, CellStatus::Unassigned { .. }),
            BoundaryKind::ToOutOfScope => matches!(s, CellStatus::OutOfScope),
            BoundaryKind::Coast => matches!(s, CellStatus::Water),
            BoundaryKind::MapEdge => matches!(s, CellStatus::NoCell),
        };
        assert!(ok, "{b:?} -> {s:?}");
    }
    // 辺は(q,r,direction)昇順で重複がない
    let keys: Vec<_> = d
        .boundaries
        .iter()
        .map(|b| (b.q, b.r, b.direction))
        .collect();
    assert!(keys.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn small_fixture_has_exactly_one_known_shared_edge() {
    // 既存フィクスチャ(P32-004): 北={(1,0),(2,0),(3,0),(1,1)} 南={(3,1),(2,2),(3,2)}。
    // 共有辺は (3,0)-(3,1) の1本だけ。(3,0)から見て南東(direction 5)。
    let map = parse_hex_map(include_bytes!("fixtures/p32_004/small_map.json")).unwrap();
    let table = generate_initial_province_table(&map, Coverage::World);
    let lay = parse_state_layout(include_str!("fixtures/p32_004/small_layout.ron")).unwrap();
    let model = WorldModel::build(table, lay).unwrap();
    let b = compute_boundaries(&model, &map);
    let borders: Vec<_> = b
        .iter()
        .filter(|e| e.kind == BoundaryKind::StateBorder)
        .collect();
    assert_eq!(borders.len(), 1);
    assert_eq!(
        (borders[0].q, borders[0].r, borders[0].direction),
        (3, 0, 5)
    );
    assert_eq!(
        (borders[0].state, borders[0].other_state),
        (MapStateId(10), Some(MapStateId(11)))
    );
}

#[test]
fn borders_are_found_across_chunk_boundaries_and_negative_q() {
    let map = real_map();
    let chunks = plan_chunks(&map);
    let mut chunk_of = vec![0usize; map.cells().len()];
    for (ci, ch) in chunks.iter().enumerate() {
        for &i in ch {
            chunk_of[i as usize] = ci;
        }
    }
    let full = generate_initial_province_table(&map, Coverage::Partial);
    let mut tested_chunk = false;
    let mut tested_negative = false;
    for p in &full.provinces {
        if tested_chunk && tested_negative {
            break;
        }
        let a = map.cell_index(p.q, p.r).unwrap();
        // 東隣の陸セル(direction 0)
        let (nq, nr) = river_neighbor(p.q, p.r, 0);
        let Some(ni) = map.cell_index(nq, nr) else {
            continue;
        };
        if map.cell_by_index(ni).terrain.is_water() {
            continue;
        }
        let crossing = chunk_of[a as usize] != chunk_of[ni as usize];
        let negative = p.q < 0;
        if !((crossing && !tested_chunk) || (negative && !tested_negative)) {
            continue;
        }
        let other = full
            .provinces
            .iter()
            .find(|x| x.q == nq && x.r == nr)
            .unwrap();
        let mut table = full.clone();
        table.provinces = vec![p.clone(), other.clone()];
        let model = WorldModel::build(
            table,
            layout(vec![state_entry(1, &[p.id]), state_entry(2, &[other.id])]),
        )
        .unwrap();
        let borders: Vec<_> = compute_boundaries(&model, &map)
            .into_iter()
            .filter(|e| e.kind == BoundaryKind::StateBorder)
            .collect();
        assert_eq!(borders.len(), 1, "{:?}", (p.q, p.r));
        assert_eq!(
            (borders[0].q, borders[0].r, borders[0].direction),
            (p.q, p.r, 0)
        );
        if crossing {
            tested_chunk = true;
        }
        if negative {
            tested_negative = true;
        }
    }
    assert!(tested_chunk, "found a land pair across a chunk boundary");
    assert!(tested_negative, "found a land pair with negative q");
}

#[test]
fn input_order_does_not_change_membership_or_boundaries() {
    let map = real_map();
    let table = load_province_table(PROVINCES).unwrap();
    let lay = load_state_layout(STATES).unwrap();
    let base = WorldModel::build(table.clone(), lay.clone()).unwrap();
    let mut t2 = table;
    t2.provinces.reverse();
    let mut l2 = lay;
    l2.states.reverse();
    for s in &mut l2.states {
        s.provinces.reverse();
    }
    let shuffled = WorldModel::build(t2, l2).unwrap();
    assert_eq!(base, shuffled);
    assert_eq!(
        compute_boundaries(&base, &map),
        compute_boundaries(&shuffled, &map)
    );
}

#[test]
fn edge_mesh_has_one_quad_per_edge() {
    let map = real_map();
    let d = preview(&map);
    let edges: Vec<_> = d
        .boundaries
        .iter()
        .map(|b| (b.q, b.r, b.direction, [1.0, 1.0, 1.0, 1.0]))
        .collect();
    let mesh = build_edge_mesh(&map, &edges, 0.3);
    assert_eq!(mesh.count_vertices(), edges.len() * 4);
    assert_eq!(build_edge_mesh(&map, &[], 0.0).count_vertices(), 0);
    assert!(border_width(2.5, -6) < 0.5, "thin when zoomed in");
    assert!(
        border_width(2.5, 4) <= 1.25 + 1e-6,
        "capped when zoomed out"
    );
    assert!(region_edge_width(2.5, 4) < border_width(2.5, 4) + 1e-6);
}

// ---------------------------------------------------------------- 読み込み失敗

#[test]
fn load_failures_return_no_display_data_and_name_the_cause() {
    let map = real_map();
    let dir = temp_dir("bad");
    let good_table = std::fs::read_to_string(PROVINCES).unwrap();
    let good_states = std::fs::read_to_string(STATES).unwrap();
    let write = |name: &str, s: &str| {
        let p = dir.join(name);
        std::fs::write(&p, s).unwrap();
        p.to_str().unwrap().to_string()
    };

    // 重複ID
    let dup = good_table.replacen("id: 17267,", "id: 17266,", 1);
    let e = load_world_preview(&write("dup.ron", &dup), STATES, &map).unwrap_err();
    assert!(e.contains("Province#17266"), "{e}");
    // 不正な所属参照
    let bad_ref = good_states.replacen("18109,", "99999999,", 1);
    let e = load_world_preview(PROVINCES, &write("badref.ron", &bad_ref), &map).unwrap_err();
    assert!(e.contains("Province#99999999"), "{e}");
    // 二重所属
    let dbl = good_states.replacen("17269,", "17266,", 1);
    let e = load_world_preview(PROVINCES, &write("dbl.ron", &dbl), &map).unwrap_err();
    assert!(e.contains("belongs to both"), "{e}");
    // 地理との不整合(別の地理の指紋)
    let stale = good_table.replacen("grid: \"", "grid: \"0", 1);
    let e = load_world_preview(&write("stale.ron", &stale), STATES, &map).unwrap_err();
    assert!(e.contains("GridChanged"), "{e}");
    // 非対応バージョン・壊れたファイル・欠落ファイル
    let e = load_world_preview(
        &write(
            "ver.ron",
            &good_table.replacen("format_version: 1", "format_version: 7", 1),
        ),
        STATES,
        &map,
    )
    .unwrap_err();
    assert!(e.contains("unsupported format_version 7"), "{e}");
    assert!(load_world_preview(&write("broken.ron", "(format_version: 1,"), STATES, &map).is_err());
    let e = load_world_preview("assets/data/world/preview/nope.ron", STATES, &map).unwrap_err();
    assert!(e.contains("nope.ron"), "{e}");

    // 入力ファイルは変更されない
    assert_eq!(std::fs::read_to_string(PROVINCES).unwrap(), good_table);
    assert_eq!(std::fs::read_to_string(STATES).unwrap(), good_states);
    // 失敗時の表示: エラー文のみ(所属・境界の表示データを持たない)
    let text = status_lines(None, Some(&WorldLoadError(e)));
    assert!(
        text.contains("読み込みに失敗") && text.contains("地理マップは確認できます"),
        "{text}"
    );
    // 失敗後に正しいファイルを読み直せば、前回の失敗状態を引きずらず成功する
    assert!(load_world_preview(PROVINCES, STATES, &map).is_ok());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn status_text_reports_partial_world() {
    let map = real_map();
    let w = loaded(&map);
    let t = status_lines(Some(&w), None);
    assert!(
        t.contains("ステート 3") && t.contains("プロヴィンス 29") && t.contains("未割当 3"),
        "{t}"
    );
    assert!(t.contains("全域の検証は未完了"), "{t}");
}
