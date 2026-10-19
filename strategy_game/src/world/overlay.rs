//! P32-005 (M1): ワールドモデルの表示用データ(GUI非依存)。
//! セルの所属状態の分類、ステート境界の辺の生成、ステート色、検証用ワールド設定の読み込みを担う。
//! Bevyには依存せず、通常のRustテストで検証できる。
//!
//! 境界の方針(描画側もこれに従う):
//! - `StateBorder`: 異なる2ステートに属するプロヴィンスの共有辺。**1辺につき1件**
//!   (`(q,r)`が辞書順で小さい側から出力)。同じステート内部の辺は出力しない。
//! - `ToUnassigned` / `ToOutOfScope` / `MapEdge`: ステートに属するプロヴィンスと、
//!   未割当プロヴィンス / ID表対象外の陸セル / 地図の外側との辺(ステート側から1件)。
//!   ステート同士の境界とは別種として区別する。
//! - `Coast`: ステートと水域(海・湖)との辺。出力はするが、海岸線は地形の塗りで見えるため
//!   通常は描画しない(描画しないという方針の根拠をテストで固定)。

use super::aggregate::{WorldGeoDerived, derive_world_geo};
use super::files::{load_province_table, load_state_layout};
use super::ids::{MapStateId, ProvinceId};
use super::issue::Severity;
use super::model::WorldModel;
use super::validate::{ValidationMode, ValidationReport, validate_against_geography};
use crate::hexmap::data::{HexMap, river_neighbor};
use crate::hexmap::geometry::river_edge_world;

/// 1セルの所属状態(A〜Dの区別)。`NoCell`は地図に存在しない座標。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellStatus {
    /// A: ステートに所属する陸上プロヴィンス。
    Assigned {
        province: ProvinceId,
        state: MapStateId,
    },
    /// B: ID表にはあるがステート未割当。
    Unassigned {
        province: ProvinceId,
    },
    /// C: ID表にない陸セル(検証範囲外)。
    OutOfScope,
    /// D: 海または湖(陸上プロヴィンスの対象外)。
    Water,
    NoCell,
}

pub fn cell_status(model: &WorldModel, map: &HexMap, q: i32, r: i32) -> CellStatus {
    let Some(cell) = map.cell(q, r) else {
        return CellStatus::NoCell;
    };
    if cell.terrain.is_water() {
        return CellStatus::Water;
    }
    match model.province_at(q, r) {
        None => CellStatus::OutOfScope,
        Some(p) => match model.state_of(p.id) {
            Some(state) => CellStatus::Assigned {
                province: p.id,
                state,
            },
            None => CellStatus::Unassigned { province: p.id },
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryKind {
    StateBorder,
    ToUnassigned,
    ToOutOfScope,
    MapEdge,
    Coast,
}

/// 境界の辺。`(q, r, direction)`は`river_edge_world`と同じ辺の定義(0=東,1=北東,2=北西,3=西,4=南西,5=南東)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryEdge {
    pub q: i32,
    pub r: i32,
    pub direction: u8,
    pub kind: BoundaryKind,
    /// 辺の`(q,r)`側のステート。
    pub state: MapStateId,
    /// `StateBorder`のときの反対側のステート。
    pub other_state: Option<MapStateId>,
}

/// すべての境界の辺を、`(q, r, direction)`昇順の決定的な順序で返す。
pub fn compute_boundaries(model: &WorldModel, map: &HexMap) -> Vec<BoundaryEdge> {
    let mut out: Vec<BoundaryEdge> = Vec::new();
    for p in model.provinces() {
        let Some(state) = model.state_of(p.id) else {
            continue;
        };
        let (q, r) = (p.cell.q, p.cell.r);
        for d in 0..6u8 {
            let (nq, nr) = river_neighbor(q, r, d);
            let (kind, other) = match cell_status(model, map, nq, nr) {
                CellStatus::Assigned { state: s2, .. } => {
                    if s2 == state || (q, r) > (nq, nr) {
                        continue; // 内部の辺、または反対側が出力する共有辺
                    }
                    (BoundaryKind::StateBorder, Some(s2))
                }
                CellStatus::Unassigned { .. } => (BoundaryKind::ToUnassigned, None),
                CellStatus::OutOfScope => (BoundaryKind::ToOutOfScope, None),
                CellStatus::Water => (BoundaryKind::Coast, None),
                CellStatus::NoCell => (BoundaryKind::MapEdge, None),
            };
            out.push(BoundaryEdge {
                q,
                r,
                direction: d,
                kind,
                state,
                other_state: other,
            });
        }
    }
    out.sort_unstable_by_key(|e| (e.q, e.r, e.direction));
    out
}

/// ステートIDから安定に決まる色(sRGB)。起動のたびに変わらない(黄金比によるHSL色相)。
pub fn state_color(id: MapStateId) -> [f32; 3] {
    let hue = (id.0 as f64 * 0.618_033_988_749_895).fract();
    hsl_to_rgb(hue, 0.55, 0.55)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h * 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [(r1 + m) as f32, (g1 + m) as f32, (b1 + m) as f32]
}

/// 検証用ワールド設定を読み込んだ結果(表示に使うデータ一式)。
#[derive(Debug)]
pub struct WorldPreviewData {
    pub model: WorldModel,
    pub report: ValidationReport,
    pub boundaries: Vec<BoundaryEdge>,
    /// ステートの地理集計・隣接(読み込み時に1回だけ導出。保存しない)。
    pub derived: WorldGeoDerived,
    /// ID表の対象セルを囲むワールド座標の矩形 `(min_x, min_y, max_x, max_y)`(カメラ移動用)。
    pub focus_bounds: Option<(f64, f64, f64, f64)>,
}

/// 検証用ワールド設定を読み込み、M0のローダーとAuthoring検証を通す。
/// `Partial`や未割当は失敗理由にしない。Errorが1件でもあれば`Err`(不正な所属や境界は一切採用しない)。
/// 入力ファイルは読み取りのみ。
pub fn load_world_preview(
    provinces_path: &str,
    states_path: &str,
    map: &HexMap,
) -> Result<WorldPreviewData, String> {
    let table = load_province_table(provinces_path).map_err(|e| e.to_string())?;
    let layout = load_state_layout(states_path).map_err(|e| e.to_string())?;
    let model = WorldModel::build(table, layout).map_err(|issues| {
        issues
            .iter()
            .take(12)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    let report = validate_against_geography(&model, map, ValidationMode::Authoring);
    if !report.is_ok() {
        return Err(report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .take(12)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n"));
    }
    let boundaries = compute_boundaries(&model, map);
    let derived = derive_world_geo(&model, map).map_err(|e| e.to_string())?;
    let focus_bounds = focus_bounds(&model, map);
    Ok(WorldPreviewData {
        model,
        report,
        boundaries,
        derived,
        focus_bounds,
    })
}

/// ID表の全プロヴィンスの頂点を囲む矩形に、約1.5セル分の余白を足したもの。
pub fn focus_bounds(model: &WorldModel, map: &HexMap) -> Option<(f64, f64, f64, f64)> {
    let mut b: Option<(f64, f64, f64, f64)> = None;
    for p in model.provinces() {
        for d in 0..6u8 {
            let (a, c) = river_edge_world(&map.grid, p.cell.q, p.cell.r, d);
            for (x, y) in [a, c] {
                b = Some(match b {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    let m = map.grid.size * 1.5;
    b.map(|(x0, y0, x1, y1)| (x0 - m, y0 - m, x1 + m, y1 + m))
}
