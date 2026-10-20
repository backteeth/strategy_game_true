//! P32-006 (M3): ステートの地理集計とステート間隣接の導出(Bevy 非依存)。
//!
//! **地理情報の導出であり、ゲームのシミュレーション集計(人口・雇用・施設・生産・税収)ではない。**
//! 結果は再生成可能なデータで、保存形式を持たない(`Serialize`を実装せず、RONへ書き戻さない)。
//!
//! - 集計対象: ステートに**所属済み**のプロヴィンスのみ。地形・気候・植生は地理データ(`HexMap`)を
//!   参照し、所属ファイルには保存しない。未割当・検証範囲外・水域は、どのステートの集計にも入らず、
//!   架空のステートへも合算しない。
//! - 「プロヴィンス数」「共有辺数」は**数**であり、面積・距離ではない(km²やkmへ換算しない)。
//! - 隣接は**六角形の辺を共有する**所属済みプロヴィンスの、異なるステート同士のみ。
//!   (六角格子では、頂点を共有するセルは必ず互いに辺も共有するため、「頂点だけの接触」という
//!   ケースは存在しない。距離2以上離れたセルは隣接ではない。)
//!   海・湖・未割当・検証範囲外を挟んだ関係は隣接としない。河川は隣接を切断しない。
//! - 共有辺は`compute_boundaries`の`StateBorder`(整数座標で正規化済み、1辺1件)をそのまま使い、
//!   河川は`HexMap::river_edges`(両側表現を統合した正規化済みの辺)と同じ正規形のキー
//!   `(q, r, direction)`で突き合わせるため、両側から数えても二重計上しない。
//! - 完全性: 結果は「今回読み込んだ地理と所属データ」に対するものである。`Completeness`で
//!   `Coverage`・未割当・範囲外の陸セルの有無を返し、不完全な結果では「隣接が記録されていない」を
//!   「世界に隣接が存在しない」とは扱えない(`AdjacencyLookup::NotRecorded`)。

use super::files::Coverage;
use super::fingerprint::{GeographyFingerprint, compute_fingerprint};
use super::ids::MapStateId;
use super::issue::{Severity, WorldIssue};
use super::model::WorldModel;
use super::overlay::{BoundaryKind, compute_boundaries};
use super::validate::{ValidationMode, validate_against_geography};
use crate::hexmap::data::{Climate, HexMap, Terrain, Vegetation};
use std::collections::{BTreeMap, HashSet};

/// ステート1つの地理集計。各内訳の合計は`province_count`と一致する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateGeoSummary {
    pub state: MapStateId,
    /// 所属プロヴィンス数(1陸ヘックス=1プロヴィンス。面積ではない)。
    pub province_count: usize,
    /// `Terrain::ALL`の順。
    pub terrain: [usize; 8],
    /// `Climate::ALL`の順。
    pub climate: [usize; 6],
    /// `Vegetation::ALL`の順。
    pub vegetation: [usize; 5],
}

impl StateGeoSummary {
    pub fn terrain_count(&self, t: Terrain) -> usize {
        self.terrain[Terrain::ALL.iter().position(|x| *x == t).unwrap()]
    }
    pub fn climate_count(&self, c: Climate) -> usize {
        self.climate[Climate::ALL.iter().position(|x| *x == c).unwrap()]
    }
    pub fn vegetation_count(&self, v: Vegetation) -> usize {
        self.vegetation[Vegetation::ALL.iter().position(|x| *x == v).unwrap()]
    }
    /// 3種の内訳の合計がどれも`province_count`と一致するか。
    pub fn is_consistent(&self) -> bool {
        self.terrain.iter().sum::<usize>() == self.province_count
            && self.climate.iter().sum::<usize>() == self.province_count
            && self.vegetation.iter().sum::<usize>() == self.province_count
    }
}

/// 隣接する2ステート(無向。`a < b`で1組を1件だけ保持する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateAdjacency {
    pub a: MapStateId,
    pub b: MapStateId,
    /// 共有している六角形の辺の数(長さの単位ではない)。
    pub shared_edges: usize,
    /// 共有辺のうち、河川が存在する辺の数(`<= shared_edges`)。
    pub shared_river_edges: usize,
}

/// 導出結果の完全性。今回読み込んだ地理と所属データに対するもので、将来の正式な世界設定の
/// 完成を意味しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Completeness {
    pub coverage: Coverage,
    /// ステート未割当のプロヴィンス数。
    pub unassigned_provinces: usize,
    /// プロヴィンス表に無い陸セル(検証範囲外)の数。
    pub out_of_scope_land_cells: usize,
}

impl Completeness {
    /// 読み込んだ地理の全陸セルが表に載っており、その全てがステートに所属している。
    pub fn is_complete(&self) -> bool {
        self.coverage == Coverage::World
            && self.unassigned_provinces == 0
            && self.out_of_scope_land_cells == 0
    }
}

/// 2ステートの隣接の問い合わせ結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdjacencyLookup<'a> {
    /// 共有辺が確認できた。
    Found(&'a StateAdjacency),
    /// 共有辺は記録されていない。**結果が完全な場合に限り**「隣接しない」が確定する。
    /// 不完全(`complete == false`)なら、未割当・範囲外・表に無い領域の向こうの関係は不明。
    NotRecorded { complete: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldGeoDerived {
    summaries: Vec<StateGeoSummary>,
    adjacencies: Vec<StateAdjacency>,
    completeness: Completeness,
    /// 導出時点の地理識別情報と所属の署名(古い結果の検出用)。
    map_fingerprint: GeographyFingerprint,
    membership_signature: u64,
}

#[derive(Debug)]
pub enum DeriveError {
    /// 地理データとの照合でErrorがあるため、部分的な集計を作らない。
    InvalidWorld(Vec<WorldIssue>),
}

impl std::fmt::Display for DeriveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let DeriveError::InvalidWorld(issues) = self;
        write!(f, "cannot derive state geography: ")?;
        for (i, e) in issues.iter().take(5).enumerate() {
            if i > 0 {
                write!(f, "; ")?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for DeriveError {}

/// 所属関係の署名(ステートID・所属プロヴィンスID・各プロヴィンスの座標)。順序に依存しない。
pub(crate) fn membership_signature(model: &WorldModel) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut put = |v: u64| {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for s in model.states() {
        put(0xFFFF_FFFF_0000_0000 | s.id.0 as u64);
        for p in &s.provinces {
            put(p.0 as u64);
            if let Some(def) = model.province(*p) {
                put(def.cell.q as u32 as u64);
                put(def.cell.r as u32 as u64);
            }
        }
    }
    h
}

/// 地理データと所属データから、ステートの地理集計とステート間隣接を導出する。
/// M0の制作途中(Authoring)検証にErrorがある場合は、部分的な集計を作らず`Err`を返す
/// (存在しないセルや水域を指すプロヴィンスを黙って無視しない)。
pub fn derive_world_geo(model: &WorldModel, map: &HexMap) -> Result<WorldGeoDerived, DeriveError> {
    let report = validate_against_geography(model, map, ValidationMode::Authoring);
    if !report.is_ok() {
        let errors = report
            .issues
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        return Err(DeriveError::InvalidWorld(errors));
    }

    // --- 集計(所属済みプロヴィンスのみ。状態ごとに1回ずつ) ---
    let mut summaries: Vec<StateGeoSummary> = Vec::with_capacity(model.states().len());
    for s in model.states() {
        let mut sum = StateGeoSummary {
            state: s.id,
            province_count: 0,
            terrain: [0; 8],
            climate: [0; 6],
            vegetation: [0; 5],
        };
        // `MapStateDef::provinces`は重複なしのID昇順(M0で正規化済み)。
        for pid in &s.provinces {
            let def = model
                .province(*pid)
                .expect("validated: every state member exists");
            let cell = map
                .cell(def.cell.q, def.cell.r)
                .expect("validated: every province cell exists");
            sum.province_count += 1;
            sum.terrain[Terrain::ALL
                .iter()
                .position(|t| *t == cell.terrain)
                .unwrap()] += 1;
            sum.climate[Climate::ALL
                .iter()
                .position(|c| *c == cell.climate)
                .unwrap()] += 1;
            sum.vegetation[Vegetation::ALL
                .iter()
                .position(|v| *v == cell.vegetation)
                .unwrap()] += 1;
        }
        debug_assert!(sum.is_consistent());
        summaries.push(sum);
    }
    summaries.sort_unstable_by_key(|s| s.state);

    // --- 隣接(StateBorderの辺を無向の組へまとめる) ---
    let rivers: HashSet<(i32, i32, u8)> = map
        .river_edges()
        .iter()
        .map(|e| (e.q, e.r, e.direction))
        .collect();
    let mut pairs: BTreeMap<(MapStateId, MapStateId), (usize, usize)> = BTreeMap::new();
    for e in compute_boundaries(model, map) {
        if e.kind != BoundaryKind::StateBorder {
            continue;
        }
        let Some(other) = e.other_state else { continue };
        let key = if e.state < other {
            (e.state, other)
        } else {
            (other, e.state)
        };
        let entry = pairs.entry(key).or_insert((0, 0));
        entry.0 += 1;
        // 辺は(q,r)が辞書順で小さい側の正規形。河川の正規形と同じキーなので直接照合できる。
        if rivers.contains(&(e.q, e.r, e.direction)) {
            entry.1 += 1;
        }
    }
    let adjacencies: Vec<StateAdjacency> = pairs
        .into_iter()
        .map(|((a, b), (shared, river))| StateAdjacency {
            a,
            b,
            shared_edges: shared,
            shared_river_edges: river,
        })
        .collect();

    let unassigned_provinces = model.unassigned_provinces().len();
    let out_of_scope_land_cells = map
        .cells()
        .iter()
        .filter(|c| !c.terrain.is_water() && model.province_at(c.q, c.r).is_none())
        .count();

    Ok(WorldGeoDerived {
        summaries,
        adjacencies,
        completeness: Completeness {
            coverage: model.coverage(),
            unassigned_provinces,
            out_of_scope_land_cells,
        },
        map_fingerprint: compute_fingerprint(map),
        membership_signature: membership_signature(model),
    })
}

impl WorldGeoDerived {
    pub fn summaries(&self) -> &[StateGeoSummary] {
        &self.summaries
    }
    pub fn adjacencies(&self) -> &[StateAdjacency] {
        &self.adjacencies
    }
    pub fn completeness(&self) -> Completeness {
        self.completeness
    }
    pub fn summary(&self, state: MapStateId) -> Option<&StateGeoSummary> {
        self.summaries.iter().find(|s| s.state == state)
    }
    /// 2ステート間の隣接。順序(A,B / B,A)に依存せず同じ結果を返す。
    pub fn adjacency_between(&self, x: MapStateId, y: MapStateId) -> AdjacencyLookup<'_> {
        let key = if x < y { (x, y) } else { (y, x) };
        match self.adjacencies.iter().find(|a| (a.a, a.b) == key) {
            Some(a) => AdjacencyLookup::Found(a),
            None => AdjacencyLookup::NotRecorded {
                complete: self.completeness.is_complete(),
            },
        }
    }
    /// ステートの隣接一覧: `(相手のステート, その組の隣接情報)`。相手のID昇順。
    pub fn neighbors_of(&self, state: MapStateId) -> Vec<(MapStateId, &StateAdjacency)> {
        let mut v: Vec<(MapStateId, &StateAdjacency)> = self
            .adjacencies
            .iter()
            .filter_map(|a| {
                if a.a == state {
                    Some((a.b, a))
                } else if a.b == state {
                    Some((a.a, a))
                } else {
                    None
                }
            })
            .collect();
        v.sort_unstable_by_key(|(o, _)| *o);
        v
    }
    /// この導出結果が、渡された所属データと地理データに対して最新か。
    /// 地理(識別情報)または所属が変わっていれば`false`(古い結果を最新として使わない)。
    pub fn is_current_for(&self, model: &WorldModel, map: &HexMap) -> bool {
        self.map_fingerprint == compute_fingerprint(map)
            && self.membership_signature == membership_signature(model)
    }
}
