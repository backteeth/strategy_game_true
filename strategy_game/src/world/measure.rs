//! P32-031 (M5-b): 規模測定専用の入力(メモリ上だけ。**正式な世界定義・区画・所有・国境・実効支配ではない**)。
//!
//! - 地図の陸セル1つ=1プロヴィンスとして、測定用のグラフ定義を作る(`ProvinceId` は陸セルを `(r, q)` 昇順に0から採番。
//!   `generate_initial_province_table` と同じ並びだが、それは正式な割当ではなく、この関数の出力は**どこにも保存しない**)。
//! - ステートの割り当て規則(測定専用): セルを `block × block` の格子に区切り、1格子=1ステート(`MapStateId` は
//!   `state_base + 格子番号`)。さらに、`ProvinceId % unassigned_modulus == 0` のプロヴィンスは未所属にする
//!   (未所属を含む集約の費用を測るため)。
//! - 探索主体(国家に相当)の起点と通行可能集合は、グラフと主体数だけから決定的に作る(乱数・時刻・列挙順に依存しない)。
//!   **これは負荷の形を作る入力であり、国家の領土・通行権ではない。**
//!
//! 既存の地図・区画アセット・保存済みIDは読むだけで、変更しない。

use super::connectivity::{ExplicitPassable, ProvinceCells, ProvinceGraph};
use super::ids::{HexCoord, MapStateId, ProvinceId};
use crate::hexmap::data::HexMap;

/// 測定専用のステート割り当て規則。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasureRule {
    /// 格子の1辺(セル数)。
    pub block: i32,
    /// `MapStateId` の基準値。
    pub state_base: u32,
    /// `ProvinceId % 値 == 0` を未所属にする(0なら未所属を作らない)。
    pub unassigned_modulus: u32,
}

impl MeasureRule {
    /// 規模測定の既定: 10×10 セルの格子=1ステート、97 番目ごとに未所属。
    pub const DEFAULT: MeasureRule = MeasureRule { block: 10, state_base: 1000, unassigned_modulus: 97 };
}

/// 測定入力の件数(記録用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasureInputFacts {
    pub land_cells: usize,
    pub provinces: usize,
    pub states: usize,
    pub unassigned: usize,
}

/// 陸セル1つ=1プロヴィンスの測定用定義(`ProvinceId` 昇順)と、その件数。
pub fn land_definitions(map: &HexMap, rule: MeasureRule) -> (Vec<ProvinceCells>, MeasureInputFacts) {
    let mut land: Vec<(i32, i32)> = map.cells().iter().filter(|c| !c.terrain.is_water()).map(|c| (c.r, c.q)).collect();
    land.sort_unstable();
    let min_q = land.iter().map(|&(_, q)| q).min().unwrap_or(0);
    let min_r = land.iter().map(|&(r, _)| r).min().unwrap_or(0);
    let max_q = land.iter().map(|&(_, q)| q).max().unwrap_or(0);
    let width = ((max_q - min_q) / rule.block + 1) as u32;
    let mut defs = Vec::with_capacity(land.len());
    let mut states = std::collections::BTreeSet::new();
    let mut unassigned = 0usize;
    for (i, &(r, q)) in land.iter().enumerate() {
        let id = i as u32;
        let state = if rule.unassigned_modulus != 0 && id.is_multiple_of(rule.unassigned_modulus) {
            unassigned += 1;
            None
        } else {
            let br = ((r - min_r) / rule.block) as u32;
            let bq = ((q - min_q) / rule.block) as u32;
            let s = MapStateId(rule.state_base + br * width + bq);
            states.insert(s);
            Some(s)
        };
        defs.push(ProvinceCells { id: ProvinceId(id), state, cells: vec![HexCoord::new(q, r)] });
    }
    let facts = MeasureInputFacts { land_cells: land.len(), provinces: defs.len(), states: states.len(), unassigned };
    (defs, facts)
}

/// 負荷の形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadKind {
    /// 通行範囲が分かれる: 各主体の通行可能集合は、`q` 昇順に並べた全プロヴィンスを主体数で等分した帯(互いに素)。
    Split,
    /// 各主体が広範囲を探索する: 通行可能集合は全プロヴィンスから、主体ごとの決定的な約5%を除いたもの。
    Heavy,
}

/// 1主体の測定入力。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    pub index: usize,
    pub source: ProvinceId,
    /// 通行可能な `ProvinceId`(昇順)。
    pub passable: Vec<ProvinceId>,
}

fn mix(mut x: u64) -> u64 {
    // splitmix64 の1ステップ(決定的なハッシュ。乱数ではない)。
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// 主体 `agents` 個分の測定入力を決定的に作る。起点は、その主体の通行可能部分の最大の連結成分(同数なら最小IDの成分)の
/// 中ほどのプロヴィンス(探索が空振りしないための選び方。国家の首都ではない)。
/// グラフの全ノードが1セルであることを仮定する(測定専用グラフ)。
pub fn make_agents(graph: &ProvinceGraph, kind: LoadKind, agents: usize) -> Vec<AgentSpec> {
    let mut by_q: Vec<(i32, i32, ProvinceId)> = graph
        .node_ids()
        .map(|id| {
            let c = graph.cells_of(id).expect("node")[0];
            (c.q, c.r, id)
        })
        .collect();
    by_q.sort_unstable();
    let n = by_q.len();
    let mut out = Vec::with_capacity(agents);
    for k in 0..agents {
        let mut passable: Vec<ProvinceId> = match kind {
            LoadKind::Split => by_q[n * k / agents..n * (k + 1) / agents].iter().map(|t| t.2).collect(),
            LoadKind::Heavy => graph.node_ids().filter(|id| !mix((id.0 as u64) << 20 ^ (k as u64 + 1)).is_multiple_of(20)).collect(),
        };
        passable.sort_unstable();
        let pass = ExplicitPassable::new(graph, passable.iter().copied()).expect("ids come from the graph");
        let comps = graph.components_among(&pass);
        // 最大の成分(同数なら先頭 = 最小IDの成分)
        let best = comps.iter().fold(None::<&Vec<ProvinceId>>, |acc, c| match acc {
            Some(a) if a.len() >= c.len() => Some(a),
            _ => Some(c),
        });
        let source = best.map(|c| c[c.len() * (2 * k + 1) / (2 * agents)]).unwrap_or(ProvinceId(0));
        out.push(AgentSpec { index: k, source, passable });
    }
    out
}
