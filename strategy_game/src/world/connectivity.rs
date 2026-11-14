//! P32-030 (M5-a): プロヴィンス間の**陸上**隣接グラフと、明示的な通行条件による純粋な到達判定。
//!
//! 設計は `verification_logs/phase-32/p32-029/`(§3 責務の分離、§4 最小構成)。この段階が扱うのは、
//! 連結性の4層のうち **L1(地理的な経路の存在)** と、**L2(通行条件)の受け口**だけである:
//! - 輸送能力(L3)・実際に届く補給量(L4)は計算しない。**「到達可能」は「十分な補給が届く」ではない。**
//! - 経済・保存・軍には接続しない(この型はどれも `MapEconomySimulation`・保存形式・師団を知らない)。
//! - 通行条件は**呼び出し元が通行可能な `ProvinceId` を明示する**。国家の経済上の担当ステートから推測しない。
//!   正式な所有・実効支配・通行権のルールは決めていない(D6 等は後続)。
//! - 河川は通行を妨げない(辺の事実として記録するだけ)。海路・港・鉄道・施設は到達条件に使わない。
//!
//! ## グラフ
//! - ノードは `ProvinceId`(ID昇順)。辺は、2つのプロヴィンスのセルが六角形の辺で接している場合に1本(無向。
//!   プロヴィンスの組ごとに1本だけで、接触箇所が複数あっても重複しない。自己辺は作らない)。六角セルの隣接の計算は
//!   既存の `hexmap::data::hex_neighbors` を使い、別実装は持たない。
//! - **水域(海・湖)にノードは無く、水域を経由する辺は作らない**(辺は陸のセル同士の直接の接触だけ)。
//! - 1プロヴィンスが複数セルを持つ場合(現在の `WorldModel` は1陸セル=1プロヴィンスだが、型は複数セルを扱える)は、
//!   **セルがすべて地図上の陸で、互いに隣接でつながっていること**を構築時に検証する。つながっていない定義は拒否する
//!   (区画内を瞬間移動できるグラフにしない)。1つのセルを複数のプロヴィンスが持つ定義も拒否する。
//! - **範囲外の陸**(地図上の陸セルで、どのプロヴィンスにも属さないもの)は、勝手に `ProvinceId` を発行せず、経路の橋渡しにも
//!   使わない。ただし、その存在は「情報が不完全」であることの根拠として保持する(§到達判定の `Unknown`)。
//! - 完全性と範囲(`GraphScope`)を保持する: `Coverage`、ノード数、ステート未割当のノード数、範囲外の陸セル数。
//!
//! ## 到達判定(純粋関数)
//! 入力は グラフ・起点・明示的な通行条件。時刻・乱数・グローバル状態に依存せず、入力を変更しない。出力はすべて `ProvinceId` 昇順。
//! 4区分(`Reachability`): 到達可能 / 到達不能(理由つき)/ 不明 / 対象外。
//! - **到達可能**: 通行可能なプロヴィンスだけを既知の辺でたどって着ける。
//! - **到達不能**(確定): 対象が明示条件で通行不可、または、範囲外の陸をすべて通れると**仮定しても**経路が無い。
//! - **不明**: 既知の辺では着けないが、範囲外の陸をすべて通れると仮定すれば着ける(未知の領域を通る経路があり得る)。
//!   「不完全なグラフだから全部不明」ではなく、この具体的な条件のときだけ不明になる。
//! - **対象外**: ノードでないもの(未知の `ProvinceId`、水域・範囲外の陸・地図外のセル)。
//!
//! 計算量は O(ノード + 辺 + 範囲外の陸セル)。

use super::files::Coverage;
use super::fingerprint::{GeographyFingerprint, compute_fingerprint};
use super::ids::{HexCoord, MapStateId, ProvinceId};
use super::model::WorldModel;
use super::validate::{ValidationMode, validate_against_geography};
use crate::hexmap::data::{HexMap, hex_neighbors, opposite_direction};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

// ---------------------------------------------------------------- 入力の定義

/// グラフの入力となる、1プロヴィンスの定義(複数セル可)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvinceCells {
    pub id: ProvinceId,
    /// 所属ステート(ラベル。通行の可否には使わない。ステート境界をまたぐ辺の事実と、要約のための情報)。
    pub state: Option<MapStateId>,
    /// 所属セル(1つ以上。順序は結果に影響しない)。
    pub cells: Vec<HexCoord>,
}

/// グラフを作れない入力。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// 世界定義が地理データと整合しない(`validate_against_geography` の Error。メッセージの先頭の一部)。
    InvalidWorld(Vec<String>),
    DuplicateProvince(ProvinceId),
    ProvinceWithoutCells(ProvinceId),
    DuplicateCellInProvince { province: ProvinceId, cell: HexCoord },
    /// セルが地図に存在しない。
    CellOutsideMap { province: ProvinceId, cell: HexCoord },
    /// セルが水域(プロヴィンスは陸だけに置ける)。
    CellIsWater { province: ProvinceId, cell: HexCoord },
    /// 1つのセルを2つのプロヴィンスが持つ(`a < b`)。
    CellSharedByProvinces { cell: HexCoord, a: ProvinceId, b: ProvinceId },
    /// 複数セルのプロヴィンスで、セルが隣接でつながっていない(区画内の瞬間移動を許さない)。
    ProvinceNotContiguous(ProvinceId),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::InvalidWorld(m) => write!(f, "the world definition does not match the geography: {}", m.join("; ")),
            GraphError::DuplicateProvince(p) => write!(f, "{p} is defined more than once"),
            GraphError::ProvinceWithoutCells(p) => write!(f, "{p} has no cells"),
            GraphError::DuplicateCellInProvince { province, cell } => write!(f, "{province} lists the cell {cell} twice"),
            GraphError::CellOutsideMap { province, cell } => write!(f, "{province}: the cell {cell} is not on the map"),
            GraphError::CellIsWater { province, cell } => write!(f, "{province}: the cell {cell} is water (provinces are land only)"),
            GraphError::CellSharedByProvinces { cell, a, b } => write!(f, "the cell {cell} belongs to both {a} and {b}"),
            GraphError::ProvinceNotContiguous(p) => write!(f, "{p}: its cells are not connected to each other"),
        }
    }
}

impl std::error::Error for GraphError {}

// ---------------------------------------------------------------- グラフ

/// 隣接する2プロヴィンスの辺の事実(通行の可否には使わない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeInfo {
    /// 接している六角形の辺の数(セル単位の接触の数。長さの単位ではない)。
    pub contacts: u32,
    /// そのうち、河川がある辺の数。
    pub river_contacts: u32,
}

/// グラフの範囲と完全性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphScope {
    pub coverage: Coverage,
    pub nodes: usize,
    /// ステートに属さないノード(プロヴィンス表にはあるが未割当。ノードとして存在する)。
    pub unassigned_nodes: usize,
    /// 地図上の陸セルのうち、どのプロヴィンスにも属さないもの(**ノードにしない**。情報が不完全な範囲)。
    pub out_of_scope_land_cells: usize,
}

impl GraphScope {
    /// 地理上の陸がすべてノードに含まれている(`Unknown` が生じない)。
    pub fn is_complete(&self) -> bool {
        self.out_of_scope_land_cells == 0
    }
}

/// グラフの識別(古い結果の検出用。`WorldGeoDerived` と同様の考え方)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphSignature {
    pub geography: GeographyFingerprint,
    /// ノード(ID・セル・所属ステート)の署名。順序に依存しない。
    pub nodes: u64,
}

/// セルの分類(問い合わせ用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellClass {
    Province(ProvinceId),
    /// 地図上の陸だが、プロヴィンス表に無い(検証範囲外)。
    OutOfScopeLand,
    /// 海または湖。
    Water,
    /// 地図に存在しない座標。
    OutsideMap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    id: ProvinceId,
    state: Option<MapStateId>,
    cells: Vec<HexCoord>,
    /// 接している範囲外の陸セルの番号(昇順)。
    oos: Vec<usize>,
}

/// プロヴィンス間の陸上隣接グラフ(導出値。保存しない)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvinceGraph {
    nodes: Vec<Node>,
    index: HashMap<ProvinceId, usize>,
    /// セル → ノード番号(複数セルのプロヴィンスは全セルが載る)。
    cell_owner: HashMap<HexCoord, usize>,
    /// ノードごとの隣接(隣接ノードの番号の昇順 = `ProvinceId` の昇順)。
    adj: Vec<Vec<(usize, EdgeInfo)>>,
    oos_cells: Vec<HexCoord>,
    oos_index: HashMap<HexCoord, usize>,
    /// 範囲外の陸セルごとの隣接: (範囲外セルの番号, ノードの番号)。
    oos_adj_cells: Vec<Vec<usize>>,
    oos_adj_nodes: Vec<Vec<usize>>,
    scope: GraphScope,
    signature: GraphSignature,
}

fn nodes_signature(nodes: &[Node]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut put = |v: u64| {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for n in nodes {
        put(0xFFFF_FFFF_0000_0000 | n.id.0 as u64);
        match n.state {
            Some(s) => put(0xAAAA_0000_0000_0000 | s.0 as u64),
            None => put(0xBBBB_0000_0000_0000),
        }
        put(n.cells.len() as u64);
        for c in &n.cells {
            put(c.q as u32 as u64);
            put(c.r as u32 as u64);
        }
    }
    h
}

impl ProvinceGraph {
    /// 検証済みの世界定義と地図から作る(`validate_against_geography` の Authoring 検証に通ることを要求する。
    /// 通らない場合は、部分的なグラフを作らず `Err`)。現在の `WorldModel` は1陸セル=1プロヴィンス。
    pub fn build(model: &WorldModel, map: &HexMap) -> Result<ProvinceGraph, GraphError> {
        let report = validate_against_geography(model, map, ValidationMode::Authoring);
        if !report.is_ok() {
            let errors = report
                .issues
                .iter()
                .filter(|i| i.severity == super::issue::Severity::Error)
                .take(12)
                .map(|i| i.to_string())
                .collect();
            return Err(GraphError::InvalidWorld(errors));
        }
        let defs = Self::definitions_of(model);
        Self::from_definitions(&defs, map, model.coverage())
    }

    fn definitions_of(model: &WorldModel) -> Vec<ProvinceCells> {
        model
            .provinces()
            .iter()
            .map(|p| ProvinceCells { id: p.id, state: model.state_of(p.id), cells: vec![p.cell] })
            .collect()
    }

    /// プロヴィンスの定義(複数セル可)から作る。定義の列挙順に依存しない(内部で `ProvinceId` 昇順に整える)。
    /// 世界定義の検証(`WorldModel`)を通らない入力(複数セル)でも、ここで構造を検証する。
    pub fn from_definitions(defs: &[ProvinceCells], map: &HexMap, coverage: Coverage) -> Result<ProvinceGraph, GraphError> {
        // --- 検証と正規化 ---
        let mut sorted: Vec<&ProvinceCells> = defs.iter().collect();
        sorted.sort_by_key(|d| d.id);
        for w in sorted.windows(2) {
            if w[0].id == w[1].id {
                return Err(GraphError::DuplicateProvince(w[0].id));
            }
        }
        let mut owner: HashMap<HexCoord, usize> = HashMap::new(); // セル → ノード番号
        let mut nodes: Vec<Node> = Vec::with_capacity(sorted.len());
        for (i, d) in sorted.iter().enumerate() {
            if d.cells.is_empty() {
                return Err(GraphError::ProvinceWithoutCells(d.id));
            }
            let mut cells = d.cells.clone();
            cells.sort();
            for w in cells.windows(2) {
                if w[0] == w[1] {
                    return Err(GraphError::DuplicateCellInProvince { province: d.id, cell: w[0] });
                }
            }
            for c in &cells {
                match map.cell(c.q, c.r) {
                    None => return Err(GraphError::CellOutsideMap { province: d.id, cell: *c }),
                    Some(cell) if cell.terrain.is_water() => return Err(GraphError::CellIsWater { province: d.id, cell: *c }),
                    Some(_) => {}
                }
                if let Some(&other) = owner.get(c) {
                    let (a, b) = (sorted[other].id.min(d.id), sorted[other].id.max(d.id));
                    return Err(GraphError::CellSharedByProvinces { cell: *c, a, b });
                }
                owner.insert(*c, i);
            }
            if !cells_contiguous(&cells) {
                return Err(GraphError::ProvinceNotContiguous(d.id));
            }
            nodes.push(Node { id: d.id, state: d.state, cells, oos: Vec::new() });
        }
        let index: HashMap<ProvinceId, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();

        // --- 範囲外の陸セル(地図上の陸で、どのプロヴィンスにも属さない) ---
        let mut oos_cells: Vec<HexCoord> = map
            .cells()
            .iter()
            .filter(|c| !c.terrain.is_water())
            .map(|c| HexCoord::new(c.q, c.r))
            .filter(|c| !owner.contains_key(c))
            .collect();
        oos_cells.sort();
        let oos_index: HashMap<HexCoord, usize> = oos_cells.iter().enumerate().map(|(i, c)| (*c, i)).collect();

        // --- 辺(陸のセル同士の直接の接触だけ。水域・地図外は辺にならない) ---
        let rivers: HashSet<(i32, i32, u8)> = map.river_edges().iter().map(|e| (e.q, e.r, e.direction)).collect();
        let mut edge_map: BTreeMap<(usize, usize), EdgeInfo> = BTreeMap::new();
        let mut node_oos: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); nodes.len()];
        for (i, n) in nodes.iter().enumerate() {
            for c in &n.cells {
                for (d, (nq, nr)) in hex_neighbors(c.q, c.r).into_iter().enumerate() {
                    let nb = HexCoord::new(nq, nr);
                    if let Some(&j) = owner.get(&nb) {
                        if j > i {
                            // 物理的な1辺は、両側のセルから見えるので、小さい番号の側からだけ数える。
                            let key = if (c.q, c.r) <= (nq, nr) { (c.q, c.r, d as u8) } else { (nq, nr, opposite_direction(d as u8)) };
                            let e = edge_map.entry((i, j)).or_insert(EdgeInfo { contacts: 0, river_contacts: 0 });
                            e.contacts += 1;
                            if rivers.contains(&key) {
                                e.river_contacts += 1;
                            }
                        }
                    } else if let Some(&o) = oos_index.get(&nb) {
                        node_oos[i].insert(o);
                    }
                }
            }
        }
        let mut adj: Vec<Vec<(usize, EdgeInfo)>> = vec![Vec::new(); nodes.len()];
        for (&(i, j), info) in &edge_map {
            adj[i].push((j, *info));
            adj[j].push((i, *info));
        }
        for a in &mut adj {
            a.sort_by_key(|(j, _)| *j);
        }
        for (i, set) in node_oos.into_iter().enumerate() {
            nodes[i].oos = set.into_iter().collect();
        }
        // 範囲外セルの隣接(範囲外セル同士・ノードとの接触)
        let mut oos_adj_cells: Vec<Vec<usize>> = vec![Vec::new(); oos_cells.len()];
        let mut oos_adj_nodes: Vec<Vec<usize>> = vec![Vec::new(); oos_cells.len()];
        for (o, c) in oos_cells.iter().enumerate() {
            let mut cells_n: BTreeSet<usize> = BTreeSet::new();
            let mut nodes_n: BTreeSet<usize> = BTreeSet::new();
            for (nq, nr) in hex_neighbors(c.q, c.r) {
                let nb = HexCoord::new(nq, nr);
                if let Some(&j) = owner.get(&nb) {
                    nodes_n.insert(j);
                } else if let Some(&p) = oos_index.get(&nb) {
                    cells_n.insert(p);
                }
            }
            oos_adj_cells[o] = cells_n.into_iter().collect();
            oos_adj_nodes[o] = nodes_n.into_iter().collect();
        }

        let scope = GraphScope {
            coverage,
            nodes: nodes.len(),
            unassigned_nodes: nodes.iter().filter(|n| n.state.is_none()).count(),
            out_of_scope_land_cells: oos_cells.len(),
        };
        let signature = GraphSignature { geography: compute_fingerprint(map), nodes: nodes_signature(&nodes) };
        Ok(ProvinceGraph { nodes, index, cell_owner: owner, adj, oos_cells, oos_index, oos_adj_cells, oos_adj_nodes, scope, signature })
    }

    // ------------------------------------------------------------ 参照

    pub fn scope(&self) -> GraphScope {
        self.scope
    }

    pub fn signature(&self) -> &GraphSignature {
        &self.signature
    }

    /// このグラフが、現在の世界定義と地図から作ったものと同じか(古い結果の検出)。
    pub fn is_current_for(&self, model: &WorldModel, map: &HexMap) -> bool {
        let mut nodes: Vec<Node> = Self::definitions_of(model)
            .into_iter()
            .map(|d| Node { id: d.id, state: d.state, cells: d.cells, oos: Vec::new() })
            .collect();
        nodes.sort_by_key(|n| n.id);
        self.signature.geography == compute_fingerprint(map) && self.signature.nodes == nodes_signature(&nodes)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// 重複を除いた無向の辺の数。
    pub fn edge_count(&self) -> usize {
        self.adj.iter().map(|a| a.len()).sum::<usize>() / 2
    }

    /// ノード(`ProvinceId` 昇順)。
    pub fn node_ids(&self) -> impl Iterator<Item = ProvinceId> + '_ {
        self.nodes.iter().map(|n| n.id)
    }

    pub fn contains(&self, id: ProvinceId) -> bool {
        self.index.contains_key(&id)
    }

    pub fn state_of(&self, id: ProvinceId) -> Option<MapStateId> {
        self.index.get(&id).and_then(|&i| self.nodes[i].state)
    }

    /// 全ノードの所属ステート(`ProvinceId` 昇順。未所属は `None`)。
    pub fn node_states(&self) -> impl Iterator<Item = (ProvinceId, Option<MapStateId>)> + '_ {
        self.nodes.iter().map(|n| (n.id, n.state))
    }

    pub fn cells_of(&self, id: ProvinceId) -> Option<&[HexCoord]> {
        self.index.get(&id).map(|&i| self.nodes[i].cells.as_slice())
    }

    /// `id` の隣接ノード(`ProvinceId` 昇順)と辺の事実。未知のIDは `None`。
    pub fn neighbors(&self, id: ProvinceId) -> Option<Vec<(ProvinceId, EdgeInfo)>> {
        let &i = self.index.get(&id)?;
        Some(self.adj[i].iter().map(|(j, e)| (self.nodes[*j].id, *e)).collect())
    }

    /// 2ノードが隣接するか(辺の事実を返す)。
    pub fn edge_between(&self, a: ProvinceId, b: ProvinceId) -> Option<EdgeInfo> {
        let (&i, &j) = (self.index.get(&a)?, self.index.get(&b)?);
        self.adj[i].binary_search_by_key(&j, |(k, _)| *k).ok().map(|p| self.adj[i][p].1)
    }

    /// 異なるステートに属する(どちらも割り当て済みの)ノード間の辺の数。
    pub fn cross_state_edge_count(&self) -> usize {
        let mut n = 0;
        for (i, a) in self.adj.iter().enumerate() {
            for (j, _) in a.iter().filter(|(j, _)| *j > i) {
                let differs = matches!((self.nodes[i].state, self.nodes[*j].state), (Some(x), Some(y)) if x != y);
                if differs {
                    n += 1;
                }
            }
        }
        n
    }

    /// 河川のある接触を含む辺の数。
    pub fn river_edge_count(&self) -> usize {
        let mut n = 0;
        for (i, a) in self.adj.iter().enumerate() {
            for (j, e) in a {
                if *j > i && e.river_contacts > 0 {
                    n += 1;
                }
            }
        }
        n
    }

    /// 連結成分(各成分の `ProvinceId` 昇順、成分は最小IDの昇順)。範囲外の陸は橋渡しにならない。
    pub fn components(&self) -> Vec<Vec<ProvinceId>> {
        self.components_where(&|_| true)
    }

    /// `pass` が通行可能とするノードだけの部分グラフの連結成分(同じ並び)。
    pub fn components_among(&self, pass: &dyn Passability) -> Vec<Vec<ProvinceId>> {
        self.components_where(&|id| pass.passable(id))
    }

    fn components_where(&self, keep: &dyn Fn(ProvinceId) -> bool) -> Vec<Vec<ProvinceId>> {
        let n = self.nodes.len();
        let mut seen = vec![false; n];
        let mut out = Vec::new();
        for s in 0..n {
            if seen[s] || !keep(self.nodes[s].id) {
                continue;
            }
            let mut comp = Vec::new();
            let mut q = VecDeque::new();
            seen[s] = true;
            q.push_back(s);
            while let Some(x) = q.pop_front() {
                comp.push(self.nodes[x].id);
                for (y, _) in &self.adj[x] {
                    if !seen[*y] && keep(self.nodes[*y].id) {
                        seen[*y] = true;
                        q.push_back(*y);
                    }
                }
            }
            comp.sort();
            out.push(comp);
        }
        out
    }

    /// セルの分類(ノードに属するか、範囲外の陸か、水域か、地図外か)。
    pub fn classify_cell(&self, map: &HexMap, cell: HexCoord) -> CellClass {
        if let Some(&i) = self.cell_owner.get(&cell) {
            return CellClass::Province(self.nodes[i].id);
        }
        if self.oos_index.contains_key(&cell) {
            return CellClass::OutOfScopeLand;
        }
        match map.cell(cell.q, cell.r) {
            None => CellClass::OutsideMap,
            Some(c) if c.terrain.is_water() => CellClass::Water,
            // 地図上の陸で、グラフの構築時の地図と異なる(古いグラフ)。範囲外として扱い、ノードにはしない。
            Some(_) => CellClass::OutOfScopeLand,
        }
    }

}

/// セルの集合が、六角形の辺の隣接で1つにつながっているか(1セルは常にtrue)。
fn cells_contiguous(cells: &[HexCoord]) -> bool {
    if cells.len() <= 1 {
        return true;
    }
    let set: HashSet<HexCoord> = cells.iter().copied().collect();
    let mut seen: HashSet<HexCoord> = HashSet::new();
    let mut stack = vec![cells[0]];
    while let Some(c) = stack.pop() {
        if !seen.insert(c) {
            continue;
        }
        for (q, r) in hex_neighbors(c.q, c.r) {
            let nb = HexCoord::new(q, r);
            if set.contains(&nb) && !seen.contains(&nb) {
                stack.push(nb);
            }
        }
    }
    seen.len() == set.len()
}

// ---------------------------------------------------------------- 通行条件

/// 明示的な通行条件(L2 の受け口)。主体(国家・部隊)ごとに1つ。正式な所有・実効支配・通行権のルールは含まない。
pub trait Passability {
    fn passable(&self, province: ProvinceId) -> bool;
}

/// 通行条件の入力が不正(グラフに存在しない `ProvinceId` を含む)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPassableProvinces(pub Vec<ProvinceId>);

impl std::fmt::Display for UnknownPassableProvinces {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<String> = self.0.iter().take(12).map(|p| p.to_string()).collect();
        write!(f, "the passable set names provinces that are not in the graph: {}", ids.join(", "))
    }
}

/// 通行可能な `ProvinceId` の明示的な集合。**載っていないプロヴィンスは通れない**(既定は拒否)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExplicitPassable(BTreeSet<ProvinceId>);

impl ExplicitPassable {
    /// グラフに存在するプロヴィンスだけを受け付ける。存在しないIDがあれば、黙って捨てずにエラー(入力不正)。
    pub fn new(graph: &ProvinceGraph, ids: impl IntoIterator<Item = ProvinceId>) -> Result<ExplicitPassable, UnknownPassableProvinces> {
        let set: BTreeSet<ProvinceId> = ids.into_iter().collect();
        let unknown: Vec<ProvinceId> = set.iter().copied().filter(|p| !graph.contains(*p)).collect();
        if unknown.is_empty() { Ok(ExplicitPassable(set)) } else { Err(UnknownPassableProvinces(unknown)) }
    }

    /// 利用者の明示的な指定としての「グラフの全ノードを通行可能にする」(国家や範囲を列挙順から選ばない)。
    pub fn all_nodes(graph: &ProvinceGraph) -> ExplicitPassable {
        ExplicitPassable(graph.node_ids().collect())
    }

    /// 利用者が明示した `MapStateId` に所属するノードを通行可能にする(**名前つきの変換**。検証用の便宜で、所有・支配の導出ではない)。
    /// 存在しない(所属ノードが無い)ステートIDはエラー。
    pub fn from_states(graph: &ProvinceGraph, states: &[MapStateId]) -> Result<ExplicitPassable, Vec<MapStateId>> {
        let wanted: BTreeSet<MapStateId> = states.iter().copied().collect();
        let known: BTreeSet<MapStateId> = graph.nodes.iter().filter_map(|n| n.state).collect();
        let missing: Vec<MapStateId> = wanted.iter().copied().filter(|s| !known.contains(s)).collect();
        if !missing.is_empty() {
            return Err(missing);
        }
        Ok(ExplicitPassable(graph.nodes.iter().filter(|n| n.state.is_some_and(|s| wanted.contains(&s))).map(|n| n.id).collect()))
    }

    /// 指定した集合を取り除いた新しい通行条件(元は変更しない)。
    pub fn without(&self, remove: &ExplicitPassable) -> ExplicitPassable {
        ExplicitPassable(self.0.difference(&remove.0).copied().collect())
    }

    pub fn union(&self, other: &ExplicitPassable) -> ExplicitPassable {
        ExplicitPassable(self.0.union(&other.0).copied().collect())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 通行可能なID(昇順)。
    pub fn ids(&self) -> impl Iterator<Item = ProvinceId> + '_ {
        self.0.iter().copied()
    }
}

impl Passability for ExplicitPassable {
    fn passable(&self, province: ProvinceId) -> bool {
        self.0.contains(&province)
    }
}

// ---------------------------------------------------------------- 到達判定

/// 起点の問題。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceProblem {
    /// グラフにノードとして存在しない。
    UnknownProvince,
    /// 通行条件が起点自身を通行不可としている(**黙って通行可能にしない**)。
    NotPassable,
}

/// 入力不正(到達不能という正常な判定とは別)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReachError {
    /// 起点が1つも指定されていない。
    NoSources,
    /// 起点に問題がある(`ProvinceId` 昇順。すべての問題を返す)。
    InvalidSources(Vec<(ProvinceId, SourceProblem)>),
}

impl std::fmt::Display for ReachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReachError::NoSources => write!(f, "no source province was given"),
            ReachError::InvalidSources(v) => {
                let parts: Vec<String> = v
                    .iter()
                    .map(|(p, why)| match why {
                        SourceProblem::UnknownProvince => format!("{p} is not a province of the graph"),
                        SourceProblem::NotPassable => format!("{p} is not passable under the given condition"),
                    })
                    .collect();
                write!(f, "invalid source: {}", parts.join("; "))
            }
        }
    }
}

impl std::error::Error for ReachError {}

/// ノードごとの到達状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeStatus {
    /// 通行可能なプロヴィンスだけを既知の辺でたどって着ける。
    Reached,
    /// 既知の辺では着けないが、範囲外の陸をすべて通れると仮定すれば着ける。
    Possible,
    /// 通行可能だが、範囲外の陸を通れると仮定しても着けない。
    Unreached,
    /// 明示条件で通行不可。
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnreachableReason {
    /// 対象自身が明示条件で通行不可。
    NotPassable,
    /// 範囲外の陸をすべて通れると仮定しても経路が無い(確定)。
    NoRoute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotApplicableReason {
    /// グラフのノードでない `ProvinceId`。
    UnknownProvince,
    /// 海・湖のセル。
    Water,
    /// 範囲外の陸のセル(プロヴィンスを持たない)。
    OutOfScopeLand,
    /// 地図に存在しない座標。
    OutsideMap,
}

fn status_to_reachability(s: NodeStatus) -> Reachability {
    match s {
        NodeStatus::Reached => Reachability::Reachable,
        NodeStatus::Possible => Reachability::Unknown,
        NodeStatus::Unreached => Reachability::Unreachable(UnreachableReason::NoRoute),
        NodeStatus::Blocked => Reachability::Unreachable(UnreachableReason::NotPassable),
    }
}

/// 1つの対象についての判定(4区分)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reachability {
    Reachable,
    Unreachable(UnreachableReason),
    /// 情報が不完全なため不明(範囲外の陸を通る経路があり得る)。
    Unknown,
    NotApplicable(NotApplicableReason),
}

/// 1回の探索の結果(純粋な値。保存しない)。グラフの署名を持ち、別のグラフでの問い合わせを拒否する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reach {
    signature: GraphSignature,
    /// グラフのノードID(昇順)。結果の問い合わせにグラフを要しないよう、結果が持つ。
    ids: Vec<ProvinceId>,
    sources: Vec<ProvinceId>,
    status: Vec<NodeStatus>,
    reached: usize,
    possible: usize,
    /// 到達したノードのうち、範囲外の陸に接するものがあるか。
    touches_out_of_scope: bool,
}

impl Reach {
    /// 正規化された起点(昇順・重複なし)。
    pub fn sources(&self) -> &[ProvinceId] {
        &self.sources
    }

    /// 到達したノードの数(起点を含む)。
    pub fn reached_count(&self) -> usize {
        self.reached
    }

    /// 「不明」(範囲外を通る経路があり得る)のノードの数。
    pub fn unknown_count(&self) -> usize {
        self.possible
    }

    pub fn touches_out_of_scope(&self) -> bool {
        self.touches_out_of_scope
    }

    /// この結果が、`graph` に対するものか(グラフが作り直されたら偽。古い結果の検出)。
    pub fn is_for(&self, graph: &ProvinceGraph) -> bool {
        self.signature == graph.signature
    }

    /// 到達したプロヴィンス(`ProvinceId` 昇順)。
    pub fn reached_ids(&self) -> Vec<ProvinceId> {
        self.collect(NodeStatus::Reached)
    }

    /// 「不明」のプロヴィンス(昇順)。
    pub fn unknown_ids(&self) -> Vec<ProvinceId> {
        self.collect(NodeStatus::Possible)
    }

    fn collect(&self, want: NodeStatus) -> Vec<ProvinceId> {
        self.ids.iter().zip(&self.status).filter(|(_, s)| **s == want).map(|(id, _)| *id).collect()
    }

    /// 対象の判定。グラフのノードでない `ProvinceId` は `NotApplicable(UnknownProvince)`。
    pub fn reachability(&self, target: ProvinceId) -> Reachability {
        match self.ids.binary_search(&target) {
            Err(_) => Reachability::NotApplicable(NotApplicableReason::UnknownProvince),
            Ok(i) => status_to_reachability(self.status[i]),
        }
    }

    /// 全ノードの判定を `ProvinceId` 昇順で返す(P32-031: ステート単位の要約が、再探索せずにこの値だけから導出するための読み取り口)。
    pub fn iter(&self) -> impl Iterator<Item = (ProvinceId, Reachability)> + '_ {
        self.ids.iter().copied().zip(self.status.iter().map(|s| status_to_reachability(*s)))
    }

    /// セルを対象にした判定(水域・範囲外の陸・地図外は対象外)。
    pub fn reachability_of_cell(&self, graph: &ProvinceGraph, map: &HexMap, cell: HexCoord) -> Reachability {
        match graph.classify_cell(map, cell) {
            CellClass::Province(id) => self.reachability(id),
            CellClass::OutOfScopeLand => Reachability::NotApplicable(NotApplicableReason::OutOfScopeLand),
            CellClass::Water => Reachability::NotApplicable(NotApplicableReason::Water),
            CellClass::OutsideMap => Reachability::NotApplicable(NotApplicableReason::OutsideMap),
        }
    }
}

/// 起点から、明示的な通行条件の下で到達できるプロヴィンスを求める(純粋関数。入力を変更しない)。
///
/// - 起点は `ProvinceId` の集合(順序・重複は結果に影響しない)。**通行不可の起点・存在しない起点は `Err`**
///   (黙って通行可能にも、到達不能にもしない)。起点が空も `Err`。
/// - 起点自身は、通行可能であれば到達集合に含まれる。
/// - 通行不可のプロヴィンスへは進入しない。通行条件は、グラフの全ノードについて1回ずつ参照される。
/// - 範囲外の陸は通らない(橋渡しにしない)。ただし、到達集合が範囲外の陸に接しているときは、そこを通れると仮定した
///   探索も行い、その結果だけに着けるノードを「不明」とする。
pub fn reach(graph: &ProvinceGraph, sources: &[ProvinceId], pass: &dyn Passability) -> Result<Reach, ReachError> {
    let mut srcs: Vec<ProvinceId> = sources.to_vec();
    srcs.sort();
    srcs.dedup();
    if srcs.is_empty() {
        return Err(ReachError::NoSources);
    }
    let mut problems = Vec::new();
    for s in &srcs {
        if !graph.contains(*s) {
            problems.push((*s, SourceProblem::UnknownProvince));
        } else if !pass.passable(*s) {
            problems.push((*s, SourceProblem::NotPassable));
        }
    }
    if !problems.is_empty() {
        return Err(ReachError::InvalidSources(problems));
    }

    let n = graph.nodes.len();
    let passable: Vec<bool> = graph.nodes.iter().map(|nd| pass.passable(nd.id)).collect();
    let mut status: Vec<NodeStatus> = passable.iter().map(|p| if *p { NodeStatus::Unreached } else { NodeStatus::Blocked }).collect();

    // --- 既知の辺だけで着けるもの ---
    let mut queue: VecDeque<usize> = VecDeque::new();
    for s in &srcs {
        let i = graph.index[s];
        if status[i] != NodeStatus::Reached {
            status[i] = NodeStatus::Reached;
            queue.push_back(i);
        }
    }
    let mut touches = false;
    let mut oos_start: Vec<usize> = Vec::new();
    while let Some(x) = queue.pop_front() {
        if !graph.nodes[x].oos.is_empty() {
            touches = true;
            oos_start.extend(graph.nodes[x].oos.iter().copied());
        }
        for (y, _) in &graph.adj[x] {
            if status[*y] == NodeStatus::Unreached {
                status[*y] = NodeStatus::Reached;
                queue.push_back(*y);
            }
        }
    }
    let reached = status.iter().filter(|s| **s == NodeStatus::Reached).count();

    // --- 範囲外の陸を通れると仮定したときだけ着けるもの(不明) ---
    let mut possible = 0usize;
    if touches {
        let mut oos_seen = vec![false; graph.oos_cells.len()];
        let mut oq: VecDeque<usize> = VecDeque::new();
        for o in oos_start {
            if !oos_seen[o] {
                oos_seen[o] = true;
                oq.push_back(o);
            }
        }
        let mut nq: VecDeque<usize> = VecDeque::new();
        loop {
            if let Some(o) = oq.pop_front() {
                for c in &graph.oos_adj_cells[o] {
                    if !oos_seen[*c] {
                        oos_seen[*c] = true;
                        oq.push_back(*c);
                    }
                }
                for j in &graph.oos_adj_nodes[o] {
                    if status[*j] == NodeStatus::Unreached {
                        status[*j] = NodeStatus::Possible;
                        possible += 1;
                        nq.push_back(*j);
                    }
                }
            } else if let Some(x) = nq.pop_front() {
                for (y, _) in &graph.adj[x] {
                    if status[*y] == NodeStatus::Unreached {
                        status[*y] = NodeStatus::Possible;
                        possible += 1;
                        nq.push_back(*y);
                    }
                }
                for o in &graph.nodes[x].oos {
                    if !oos_seen[*o] {
                        oos_seen[*o] = true;
                        oq.push_back(*o);
                    }
                }
            } else {
                break;
            }
        }
    }
    debug_assert_eq!(status.len(), n);
    Ok(Reach { signature: graph.signature.clone(), ids: graph.nodes.iter().map(|nd| nd.id).collect(), sources: srcs, status, reached, possible, touches_out_of_scope: touches })
}
