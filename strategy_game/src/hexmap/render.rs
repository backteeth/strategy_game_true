//! P32-001: 表示モード別の色定義と、チャンク単位のメッシュ生成。
//!
//! 描画方式: 全セルをワールド座標の六角形(頂点6・三角形4)として、空間的に近いセルを
//! 約128ワールド単位のチャンクにまとめ、**チャンクごとに1メッシュ**にする(頂点色で塗り分け)。
//! 表示モード切替は頂点色属性だけを書き換え、ジオメトリは再生成しない。
//! セルごとのEntity/Mesh/Materialは作らない。

use super::data::{Climate, HexCell, HexMap, Terrain, Vegetation};
use super::geometry::image_to_world;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use std::collections::BTreeMap;

/// チャンク一辺(ワールド単位)。約98チャンク(実データ)になり、視錐台カリングが効く。
pub const CHUNK_WORLD_SIZE: f64 = 128.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HexViewMode {
    #[default]
    Terrain,
    Climate,
    Vegetation,
}

impl HexViewMode {
    pub const ALL: [HexViewMode; 3] = [Self::Terrain, Self::Climate, Self::Vegetation];
    pub fn label(self) -> &'static str {
        match self {
            Self::Terrain => "地形",
            Self::Climate => "気候",
            Self::Vegetation => "植生",
        }
    }
}

// 色はsRGB。未分類は陸上=マゼンタ系、水域=モードに関わらず青系で見分ける。
const UNCLASSIFIED_LAND: [f32; 3] = [0.85, 0.15, 0.75];
const SEA: [f32; 3] = [0.08, 0.20, 0.45];
const LAKE: [f32; 3] = [0.30, 0.60, 0.85];

pub fn terrain_label(t: Terrain) -> &'static str {
    match t {
        Terrain::Sea => "海",
        Terrain::Lake => "湖・池",
        Terrain::Plains => "平原",
        Terrain::Hills => "丘陵",
        Terrain::Plateau => "台地",
        Terrain::Highland => "高原",
        Terrain::Mountain => "山岳",
        Terrain::Unclassified => "未分類",
    }
}

pub fn climate_label(c: Climate) -> &'static str {
    match c {
        Climate::Tropical => "A 熱帯",
        Climate::Arid => "B 乾燥帯",
        Climate::Temperate => "C 温帯",
        Climate::Continental => "D 亜寒帯",
        Climate::Polar => "E 寒帯",
        Climate::Unclassified => "未分類",
    }
}

pub fn vegetation_label(v: Vegetation) -> &'static str {
    match v {
        Vegetation::Rainforest => "熱帯雨林",
        Vegetation::Forest => "森林",
        Vegetation::Grassland => "草原",
        Vegetation::None => "植生なし",
        Vegetation::Unclassified => "未分類",
    }
}

fn terrain_color(t: Terrain) -> [f32; 3] {
    match t {
        Terrain::Sea => SEA,
        Terrain::Lake => LAKE,
        Terrain::Plains => [0.62, 0.80, 0.42],
        Terrain::Hills => [0.42, 0.58, 0.28],
        Terrain::Plateau => [0.84, 0.64, 0.34],
        Terrain::Highland => [0.52, 0.36, 0.40],
        Terrain::Mountain => [0.90, 0.90, 0.92],
        Terrain::Unclassified => UNCLASSIFIED_LAND,
    }
}

fn climate_color(c: Climate) -> [f32; 3] {
    match c {
        Climate::Tropical => [0.90, 0.30, 0.20],
        Climate::Arid => [0.93, 0.78, 0.35],
        Climate::Temperate => [0.45, 0.75, 0.40],
        Climate::Continental => [0.30, 0.55, 0.65],
        Climate::Polar => [0.88, 0.93, 0.97],
        Climate::Unclassified => UNCLASSIFIED_LAND,
    }
}

fn vegetation_color(v: Vegetation) -> [f32; 3] {
    match v {
        Vegetation::Rainforest => [0.05, 0.40, 0.18],
        Vegetation::Forest => [0.30, 0.60, 0.25],
        Vegetation::Grassland => [0.78, 0.85, 0.40],
        Vegetation::None => [0.78, 0.74, 0.64],
        Vegetation::Unclassified => UNCLASSIFIED_LAND,
    }
}

fn water_color(t: Terrain) -> [f32; 3] {
    if t == Terrain::Sea { SEA } else { LAKE }
}

/// セルの表示色(sRGB)。
/// 気候・植生モードでも水域は水色系で識別でき、水域の未分類(=水色)と
/// 陸上の未分類(=マゼンタ)は別の色になる。水域に実際の気候/植生値が入っている場合は、
/// その値の色を水色と半々に混ぜて両方が読み取れるようにする。
pub fn cell_color(mode: HexViewMode, cell: &HexCell) -> [f32; 3] {
    let own = match mode {
        HexViewMode::Terrain => return terrain_color(cell.terrain),
        HexViewMode::Climate => (
            cell.climate == Climate::Unclassified,
            climate_color(cell.climate),
        ),
        HexViewMode::Vegetation => (
            cell.vegetation == Vegetation::Unclassified,
            vegetation_color(cell.vegetation),
        ),
    };
    if !cell.terrain.is_water() {
        return own.1;
    }
    let w = water_color(cell.terrain);
    if own.0 {
        w
    } else {
        [
            (w[0] + own.1[0]) * 0.5,
            (w[1] + own.1[1]) * 0.5,
            (w[2] + own.1[2]) * 0.5,
        ]
    }
}

/// 凡例: (ラベル, sRGB色)。陸上の未分類と水域の未分類を別行にする。
pub fn legend(mode: HexViewMode) -> Vec<(String, [f32; 3])> {
    let mut v: Vec<(String, [f32; 3])> = Vec::new();
    match mode {
        HexViewMode::Terrain => {
            for t in Terrain::ALL {
                v.push((terrain_label(t).to_string(), terrain_color(t)));
            }
        }
        HexViewMode::Climate => {
            for c in Climate::ALL {
                let label = if c == Climate::Unclassified {
                    "未分類(陸上)"
                } else {
                    climate_label(c)
                };
                v.push((label.to_string(), climate_color(c)));
            }
            v.push(("海(水域は未分類を含む)".into(), SEA));
            v.push(("湖・池(同上)".into(), LAKE));
        }
        HexViewMode::Vegetation => {
            for x in Vegetation::ALL {
                let label = if x == Vegetation::Unclassified {
                    "未分類(陸上)"
                } else {
                    vegetation_label(x)
                };
                v.push((label.to_string(), vegetation_color(x)));
            }
            v.push(("海(水域は未分類を含む)".into(), SEA));
            v.push(("湖・池(同上)".into(), LAKE));
        }
    }
    v
}

fn lin(c: [f32; 3]) -> [f32; 4] {
    let l = Color::srgb(c[0], c[1], c[2]).to_linear();
    [l.red, l.green, l.blue, 1.0]
}

/// セルを空間チャンクへ分ける(決定的な順序)。返す値は`HexMap::cells`のインデックス
/// (そのマップインスタンスの生存期間中のみ有効)。
pub fn plan_chunks(map: &HexMap) -> Vec<Vec<u32>> {
    let mut chunks: BTreeMap<(i64, i64), Vec<u32>> = BTreeMap::new();
    for (i, c) in map.cells().iter().enumerate() {
        let (x, y) = map.grid.center_world(c.q, c.r);
        let key = (
            (x / CHUNK_WORLD_SIZE).floor() as i64,
            (y / CHUNK_WORLD_SIZE).floor() as i64,
        );
        chunks.entry(key).or_default().push(i as u32);
    }
    chunks.into_values().collect()
}

pub fn vertex_colors(map: &HexMap, cells: &[u32], mode: HexViewMode) -> Vec<[f32; 4]> {
    let mut out = Vec::with_capacity(cells.len() * 6);
    for &i in cells {
        let c = lin(cell_color(mode, map.cell_by_index(i)));
        out.extend_from_slice(&[c; 6]);
    }
    out
}

/// チャンクの塗りメッシュ(1セル=頂点6・三角形4)。
pub fn build_fill_mesh(map: &HexMap, cells: &[u32], mode: HexViewMode) -> Mesh {
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(cells.len() * 6);
    let mut idx: Vec<u32> = Vec::with_capacity(cells.len() * 12);
    for (n, &i) in cells.iter().enumerate() {
        let c = map.cell_by_index(i);
        for k in 0..6 {
            let (x, y) = map.grid.corner_world(c.q, c.r, k);
            pos.push([x as f32, y as f32, 0.0]);
        }
        let b = (n * 6) as u32;
        // ワールド(Y上向き)で反時計回り。
        idx.extend_from_slice(&[
            b,
            b + 2,
            b + 1,
            b,
            b + 3,
            b + 2,
            b,
            b + 4,
            b + 3,
            b,
            b + 5,
            b + 4,
        ]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vertex_colors(map, cells, mode));
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// 軸座標で隣接セル(辺`i`の向こう側)を返す。**グリッド線の重複除去専用**で、
/// 河川directionの定義とは無関係。辺0=東, 1=南東, 2=南西, 3=西, 4=北西, 5=北東
/// (corner_imageの角度 60°·i−30° の頂点iとi+1を結ぶ辺)。
fn edge_neighbor(q: i32, r: i32, edge: usize) -> (i32, i32) {
    match edge {
        0 => (q + 1, r),
        1 => (q, r + 1),
        2 => (q - 1, r + 1),
        3 => (q - 1, r),
        4 => (q, r - 1),
        _ => (q + 1, r - 1),
    }
}

/// チャンクのヘックス境界線(LineList)。共有辺は二重に引かない:
/// 辺0〜2は常に、辺3〜5は隣接セルが存在しない場合のみ描く。
pub fn build_grid_mesh(map: &HexMap, cells: &[u32]) -> Mesh {
    let mut pos: Vec<[f32; 3]> = Vec::new();
    for &i in cells {
        let c = map.cell_by_index(i);
        for e in 0..6 {
            if e >= 3 {
                let (nq, nr) = edge_neighbor(c.q, c.r, e);
                if map.cell(nq, nr).is_some() {
                    continue;
                }
            }
            let (ax, ay) = map.grid.corner_world(c.q, c.r, e);
            let (bx, by) = map.grid.corner_world(c.q, c.r, (e + 1) % 6);
            pos.push([ax as f32, ay as f32, 0.0]);
            pos.push([bx as f32, by as f32, 0.0]);
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh
}

/// 六角形の輪(半径`r_in`〜`r_out`)のメッシュ。選択枠用の小さなメッシュ(頂点12)。
/// `center`はワールド座標。
pub fn build_ring_mesh(center: (f64, f64), r_in: f64, r_out: f64) -> Mesh {
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(12);
    let mut idx: Vec<u32> = Vec::with_capacity(36);
    for k in 0..6 {
        // image座標の角度 60°k−30° をワールドへ(Y反転は image_to_world に集約)。
        let a = (60.0 * k as f64 - 30.0).to_radians();
        for rad in [r_in, r_out] {
            let (x, y) = image_to_world(rad * a.cos(), rad * a.sin());
            pos.push([(center.0 + x) as f32, (center.1 + y) as f32, 0.0]);
        }
    }
    for k in 0..6u32 {
        let (i0, o0) = (k * 2, k * 2 + 1);
        let (i1, o1) = (((k + 1) % 6) * 2, ((k + 1) % 6) * 2 + 1);
        idx.extend_from_slice(&[i0, o1, o0, i0, i1, o1]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}
