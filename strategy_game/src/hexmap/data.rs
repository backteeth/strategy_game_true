//! P32-001: ヘックスマップJSON(`hex-atlas-game-map` schemaVersion 9)の読み込みと検証。
//!
//! 構成は「DTO(JSONそのまま) → 検証 → `HexMap`(検証済み不変データ)」の一方向。
//! 件数や属性集計はローダーへ固定しない(実データに対する期待値はテスト側が持つ)。
//! 既存の州データ(`StateRegistry`)とは独立しており、国家・州への割り当ては後続Phaseの対象。

use serde::Deserialize;
use std::fmt;

pub const EXPECTED_KIND: &str = "hex-atlas-game-map";
pub const SUPPORTED_SCHEMA_VERSION: u32 = 9;
pub const SUPPORTED_ORIENTATION: &str = "pointy";
/// 実行時に読み込むJSONの既存アセット規約上の位置(`assets/data/*.ron`と同じくcwd相対)。
pub const DEFAULT_MAP_PATH: &str = "assets/data/hex_atlas_map.json";
/// 密な座標検索表の最大セル数(異常なextentによるメモリ枯渇を防ぐ)。
const MAX_LOOKUP_SLOTS: u64 = 64_000_000;
const NO_CELL: u32 = u32::MAX;

// ---------------------------------------------------------------- DTO

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MapDto {
    schema_version: u32,
    kind: String,
    grid: GridDto,
    cells: Vec<CellDto>,
    #[serde(default)]
    rivers: Vec<RiverDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GridDto {
    orientation: String,
    size: f64,
    origin: OriginDto,
    image_width: u32,
    image_height: u32,
    extent: ExtentDto,
}

#[derive(Debug, Deserialize)]
struct OriginDto {
    x: f64,
    y: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtentDto {
    min_q: i32,
    max_q: i32,
    min_r: i32,
    max_r: i32,
}

#[derive(Debug, Deserialize)]
struct CellDto {
    q: i32,
    r: i32,
    terrain: String,
    climate: String,
    vegetation: String,
}

#[derive(Debug, Deserialize)]
struct RiverDto {
    q: i32,
    r: i32,
    direction: i64,
}

// ---------------------------------------------------------------- 属性

/// 地形。`Unclassified`は制作ツール上の有効な値(今回の実データには存在しない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Terrain {
    Sea,
    Lake,
    Plains,
    Hills,
    Plateau,
    Highland,
    Mountain,
    Unclassified,
}

/// ケッペン気候区分。`Unclassified`は有効な値(水域は通常これ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Climate {
    Tropical,
    Arid,
    Temperate,
    Continental,
    Polar,
    Unclassified,
}

/// 植生。`None`(植生なし)と`Unclassified`(未分類)は別の値。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Vegetation {
    Rainforest,
    Forest,
    Grassland,
    None,
    Unclassified,
}

impl Terrain {
    pub const ALL: [Terrain; 8] = [
        Terrain::Sea,
        Terrain::Lake,
        Terrain::Plains,
        Terrain::Hills,
        Terrain::Plateau,
        Terrain::Highland,
        Terrain::Mountain,
        Terrain::Unclassified,
    ];
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "sea" => Self::Sea,
            "lake" => Self::Lake,
            "plains" => Self::Plains,
            "hills" => Self::Hills,
            "plateau" => Self::Plateau,
            "highland" => Self::Highland,
            "mountain" => Self::Mountain,
            "unclassified" => Self::Unclassified,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Sea => "sea",
            Self::Lake => "lake",
            Self::Plains => "plains",
            Self::Hills => "hills",
            Self::Plateau => "plateau",
            Self::Highland => "highland",
            Self::Mountain => "mountain",
            Self::Unclassified => "unclassified",
        }
    }
    /// 水域(海・湖)か。
    pub fn is_water(self) -> bool {
        matches!(self, Self::Sea | Self::Lake)
    }
}

impl Climate {
    pub const ALL: [Climate; 6] = [
        Climate::Tropical,
        Climate::Arid,
        Climate::Temperate,
        Climate::Continental,
        Climate::Polar,
        Climate::Unclassified,
    ];
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "A" => Self::Tropical,
            "B" => Self::Arid,
            "C" => Self::Temperate,
            "D" => Self::Continental,
            "E" => Self::Polar,
            "unclassified" => Self::Unclassified,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Tropical => "A",
            Self::Arid => "B",
            Self::Temperate => "C",
            Self::Continental => "D",
            Self::Polar => "E",
            Self::Unclassified => "unclassified",
        }
    }
}

impl Vegetation {
    pub const ALL: [Vegetation; 5] = [
        Vegetation::Rainforest,
        Vegetation::Forest,
        Vegetation::Grassland,
        Vegetation::None,
        Vegetation::Unclassified,
    ];
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "rainforest" => Self::Rainforest,
            "forest" => Self::Forest,
            "grassland" => Self::Grassland,
            "none" => Self::None,
            "unclassified" => Self::Unclassified,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Rainforest => "rainforest",
            Self::Forest => "forest",
            Self::Grassland => "grassland",
            Self::None => "none",
            Self::Unclassified => "unclassified",
        }
    }
}

// ---------------------------------------------------------------- 検証済みモデル

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexCell {
    pub q: i32,
    pub r: i32,
    pub terrain: Terrain,
    pub climate: Climate,
    pub vegetation: Vegetation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub min_q: i32,
    pub max_q: i32,
    pub min_r: i32,
    pub max_r: i32,
}

/// 制作ツール座標(画像座標、Y下向き)上のグリッド定義。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HexGrid {
    pub size: f64,
    pub origin_x: f64,
    pub origin_y: f64,
    pub image_width: u32,
    pub image_height: u32,
    pub extent: Extent,
}

/// 河川レコード `(q, r, direction)`。
/// directionは「河川が存在する辺」(0=東,1=北東,2=北西,3=西,4=南西,5=南東)。流下方向ではない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RiverRecord {
    pub q: i32,
    pub r: i32,
    pub direction: u8,
}

/// 河川directionの隣接オフセット(制作ツール仕様・ユーザー確認済み)。
/// 画面上が北。右辺を0として反時計回り: 0=東, 1=北東, 2=北西, 3=西, 4=南西, 5=南東。
/// directionは「河川が存在する辺」を表し、流下方向ではない。
pub const RIVER_NEIGHBOR_OFFSETS: [(i32, i32); 6] =
    [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];

/// 辺の反対側から見たdirection。
pub fn opposite_direction(direction: u8) -> u8 {
    (direction + 3) % 6
}

/// セル`(q,r)`のdirection側の隣接座標。
pub fn river_neighbor(q: i32, r: i32, direction: u8) -> (i32, i32) {
    let (dq, dr) = RIVER_NEIGHBOR_OFFSETS[direction as usize % 6];
    (q + dq, r + dr)
}

/// 正規化済みの河川の辺(物理的な1本)。同じ辺を両側のセルから表した2レコードは1つにまとまる。
/// 正規化は整数座標のみで行い、(q,r)が辞書順で小さい側を`(q, r, direction)`として持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RiverEdge {
    pub q: i32,
    pub r: i32,
    pub direction: u8,
    /// 辺の両側のセルがどちらも存在するか(falseなら地図境界の辺。参照元セルは検証済みで必ず片側は存在する)。存在しない境界辺も削除せず保持する。
    pub neighbor_exists: bool,
    /// この辺を表す入力レコードの数(1または2)。
    pub records: u8,
}

fn normalize_river_edges(map: &HexMap) -> Vec<RiverEdge> {
    use std::collections::BTreeMap;
    let mut edges: BTreeMap<(i32, i32, u8), RiverEdge> = BTreeMap::new();
    for rec in &map.rivers {
        let (nq, nr) = river_neighbor(rec.q, rec.r, rec.direction);
        let (q, r, d) = if (rec.q, rec.r) <= (nq, nr) {
            (rec.q, rec.r, rec.direction)
        } else {
            (nq, nr, opposite_direction(rec.direction))
        };
        let (oq, or) = river_neighbor(q, r, d);
        edges
            .entry((q, r, d))
            .and_modify(|e| e.records += 1)
            .or_insert(RiverEdge {
                q,
                r,
                direction: d,
                neighbor_exists: map.cell(q, r).is_some() && map.cell(oq, or).is_some(),
                records: 1,
            });
    }
    edges.into_values().collect()
}

/// 検証済みの不変ヘックスマップ。
///
/// `cells`のインデックス(`u32`)は**このインスタンスの生存期間中だけ有効**で、
/// JSONの並び順を保存する以外の意味を持たない(座標やIDの代わりに使わない)。
/// 座標からの検索は`cell_index`/`cell`を使う。
#[derive(Debug)]
pub struct HexMap {
    pub grid: HexGrid,
    cells: Vec<HexCell>,
    /// extent矩形に対応する密な検索表。欠損セルは`NO_CELL`(格子は疎でありうる)。
    lookup: Vec<u32>,
    rivers: Vec<RiverRecord>,
    river_edges: Vec<RiverEdge>,
}

/// 属性ごとの件数(診断用。ローダーの仕様ではない)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeCounts {
    pub terrain: [usize; 8],
    pub climate: [usize; 6],
    pub vegetation: [usize; 5],
    /// 水域以外のセルのうち、気候/植生が未分類の件数。
    pub land_cells: usize,
    pub land_climate_unclassified: usize,
    pub land_vegetation_unclassified: usize,
}

impl HexMap {
    pub fn cells(&self) -> &[HexCell] {
        &self.cells
    }
    /// 正規化後の河川の辺(描画単位)。入力レコード数は`rivers().len()`。
    pub fn river_edges(&self) -> &[RiverEdge] {
        &self.river_edges
    }
    pub fn rivers(&self) -> &[RiverRecord] {
        &self.rivers
    }
    pub fn cell_by_index(&self, index: u32) -> &HexCell {
        &self.cells[index as usize]
    }

    fn slot(&self, q: i32, r: i32) -> Option<usize> {
        let e = &self.grid.extent;
        if q < e.min_q || q > e.max_q || r < e.min_r || r > e.max_r {
            return None;
        }
        let width = (e.max_q as i64 - e.min_q as i64 + 1) as usize;
        Some((r as i64 - e.min_r as i64) as usize * width + (q as i64 - e.min_q as i64) as usize)
    }

    /// 座標からセルの配列インデックスを引く。extent外・欠損セルは`None`。
    pub fn cell_index(&self, q: i32, r: i32) -> Option<u32> {
        let idx = *self.lookup.get(self.slot(q, r)?)?;
        (idx != NO_CELL).then_some(idx)
    }

    pub fn cell(&self, q: i32, r: i32) -> Option<&HexCell> {
        self.cell_index(q, r).map(|i| &self.cells[i as usize])
    }

    pub fn attribute_counts(&self) -> AttributeCounts {
        let mut c = AttributeCounts::default();
        for cell in &self.cells {
            c.terrain[Terrain::ALL
                .iter()
                .position(|t| *t == cell.terrain)
                .unwrap()] += 1;
            c.climate[Climate::ALL
                .iter()
                .position(|t| *t == cell.climate)
                .unwrap()] += 1;
            c.vegetation[Vegetation::ALL
                .iter()
                .position(|t| *t == cell.vegetation)
                .unwrap()] += 1;
            if !cell.terrain.is_water() {
                c.land_cells += 1;
                if cell.climate == Climate::Unclassified {
                    c.land_climate_unclassified += 1;
                }
                if cell.vegetation == Vegetation::Unclassified {
                    c.land_vegetation_unclassified += 1;
                }
            }
        }
        c
    }
}

// ---------------------------------------------------------------- エラー

#[derive(Debug)]
pub enum HexMapError {
    Io {
        path: String,
        message: String,
    },
    Parse(String),
    WrongKind(String),
    UnsupportedSchema(u32),
    UnsupportedOrientation(String),
    InvalidGrid(String),
    CellOutOfExtent {
        q: i32,
        r: i32,
    },
    DuplicateCell {
        q: i32,
        r: i32,
    },
    UnknownValue {
        field: &'static str,
        value: String,
        q: i32,
        r: i32,
    },
    InvalidRiver {
        index: usize,
        reason: String,
    },
}

impl fmt::Display for HexMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "cannot read hex map '{path}': {message}"),
            Self::Parse(m) => write!(f, "hex map JSON is malformed: {m}"),
            Self::WrongKind(k) => {
                write!(f, "unexpected kind '{k}' (expected '{EXPECTED_KIND}')")
            }
            Self::UnsupportedSchema(v) => write!(
                f,
                "unsupported schemaVersion {v} (supported: {SUPPORTED_SCHEMA_VERSION})"
            ),
            Self::UnsupportedOrientation(o) => write!(
                f,
                "unsupported grid.orientation '{o}' (supported: '{SUPPORTED_ORIENTATION}')"
            ),
            Self::InvalidGrid(m) => write!(f, "invalid grid: {m}"),
            Self::CellOutOfExtent { q, r } => {
                write!(f, "cell (q={q}, r={r}) lies outside grid.extent")
            }
            Self::DuplicateCell { q, r } => write!(f, "duplicate cell (q={q}, r={r})"),
            Self::UnknownValue { field, value, q, r } => {
                write!(f, "unknown {field} '{value}' at cell (q={q}, r={r})")
            }
            Self::InvalidRiver { index, reason } => {
                write!(f, "invalid river record #{index}: {reason}")
            }
        }
    }
}

impl std::error::Error for HexMapError {}

// ---------------------------------------------------------------- 読み込み

pub fn load_from_path(path: &str) -> Result<HexMap, HexMapError> {
    let bytes = std::fs::read(path).map_err(|e| HexMapError::Io {
        path: path.to_string(),
        message: e.to_string(),
    })?;
    parse_hex_map(&bytes)
}

pub fn parse_hex_map(bytes: &[u8]) -> Result<HexMap, HexMapError> {
    let dto: MapDto =
        serde_json::from_slice(bytes).map_err(|e| HexMapError::Parse(e.to_string()))?;
    validate(dto)
}

fn validate(dto: MapDto) -> Result<HexMap, HexMapError> {
    if dto.kind != EXPECTED_KIND {
        return Err(HexMapError::WrongKind(dto.kind));
    }
    if dto.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Err(HexMapError::UnsupportedSchema(dto.schema_version));
    }
    let g = &dto.grid;
    if g.orientation != SUPPORTED_ORIENTATION {
        return Err(HexMapError::UnsupportedOrientation(g.orientation.clone()));
    }
    if !g.size.is_finite() || g.size <= 0.0 {
        return Err(HexMapError::InvalidGrid(format!(
            "size must be positive and finite (got {})",
            g.size
        )));
    }
    if !g.origin.x.is_finite() || !g.origin.y.is_finite() {
        return Err(HexMapError::InvalidGrid("origin must be finite".into()));
    }
    if g.image_width == 0 || g.image_height == 0 {
        return Err(HexMapError::InvalidGrid(format!(
            "imageWidth/imageHeight must be positive (got {}x{})",
            g.image_width, g.image_height
        )));
    }
    let e = &g.extent;
    if e.min_q > e.max_q || e.min_r > e.max_r {
        return Err(HexMapError::InvalidGrid(format!(
            "extent is inverted (q {}..{}, r {}..{})",
            e.min_q, e.max_q, e.min_r, e.max_r
        )));
    }
    let width = (e.max_q as i64 - e.min_q as i64 + 1) as u64;
    let height = (e.max_r as i64 - e.min_r as i64 + 1) as u64;
    if width * height > MAX_LOOKUP_SLOTS {
        return Err(HexMapError::InvalidGrid(format!(
            "extent {width}x{height} is too large"
        )));
    }

    let grid = HexGrid {
        size: g.size,
        origin_x: g.origin.x,
        origin_y: g.origin.y,
        image_width: g.image_width,
        image_height: g.image_height,
        extent: Extent {
            min_q: e.min_q,
            max_q: e.max_q,
            min_r: e.min_r,
            max_r: e.max_r,
        },
    };

    let mut map = HexMap {
        grid,
        cells: Vec::with_capacity(dto.cells.len()),
        lookup: vec![NO_CELL; (width * height) as usize],
        rivers: Vec::with_capacity(dto.rivers.len()),
        river_edges: Vec::new(),
    };

    for c in &dto.cells {
        let slot = map
            .slot(c.q, c.r)
            .ok_or(HexMapError::CellOutOfExtent { q: c.q, r: c.r })?;
        if map.lookup[slot] != NO_CELL {
            return Err(HexMapError::DuplicateCell { q: c.q, r: c.r });
        }
        let unknown = |field: &'static str, value: &str| HexMapError::UnknownValue {
            field,
            value: value.to_string(),
            q: c.q,
            r: c.r,
        };
        let terrain = Terrain::parse(&c.terrain).ok_or_else(|| unknown("terrain", &c.terrain))?;
        let climate = Climate::parse(&c.climate).ok_or_else(|| unknown("climate", &c.climate))?;
        let vegetation =
            Vegetation::parse(&c.vegetation).ok_or_else(|| unknown("vegetation", &c.vegetation))?;
        map.lookup[slot] = map.cells.len() as u32;
        map.cells.push(HexCell {
            q: c.q,
            r: c.r,
            terrain,
            climate,
            vegetation,
        });
    }

    let mut seen = std::collections::HashSet::with_capacity(dto.rivers.len());
    for (index, rv) in dto.rivers.iter().enumerate() {
        if !(0..=5).contains(&rv.direction) {
            return Err(HexMapError::InvalidRiver {
                index,
                reason: format!("direction {} is outside 0..=5", rv.direction),
            });
        }
        if map.cell(rv.q, rv.r).is_none() {
            return Err(HexMapError::InvalidRiver {
                index,
                reason: format!("source cell (q={}, r={}) does not exist", rv.q, rv.r),
            });
        }
        let rec = RiverRecord {
            q: rv.q,
            r: rv.r,
            direction: rv.direction as u8,
        };
        if !seen.insert(rec) {
            return Err(HexMapError::InvalidRiver {
                index,
                reason: format!(
                    "duplicate (q={}, r={}, direction={})",
                    rv.q, rv.r, rv.direction
                ),
            });
        }
        map.rivers.push(rec);
    }

    map.river_edges = normalize_river_edges(&map);
    Ok(map)
}
