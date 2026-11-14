//! P32-031 (M5-b): `Reach`(プロヴィンス単位の到達判定。P32-030)から、`MapStateId` 単位の要約を**導出**する。
//!
//! - **到達判定の正本は `Reach`**。ここはその値を所属で数え直すだけで、ステート隣接グラフ上で探索をやり直さない
//!   (`reach` を呼ばない。入力は `Reach` と所属だけ)。
//! - 経済・保存・軍には接続しない。到達可能は「十分な補給が届く」ではない(輸送能力・補給量は計算しない)。
//! - `ProvinceId` / `MapStateId` / 旧 `StateId` を数値の一致で変換しない。所属は `ProvinceGraph` のノードのラベル
//!   または `WorldModel` の所属(ステート配置ファイルが正本)だけから得る。
//!
//! ## 集約規則(1ステートの所属プロヴィンスを、`Reachability` の4区分で数える)
//! 件数: `total`(所属総数)= `reachable` + `unreachable_not_passable` + `unreachable_no_route` + `unknown` + `not_applicable`。
//!
//! | 条件(上から順に最初に当てはまるもの) | 区分 |
//! |---|---|
//! | `total == 0`(所属が空) | `Empty`(**全域到達にしない**) |
//! | `not_applicable == total`(所属が1つも判定できない) | `NotApplicable` |
//! | `reachable == total` | `Full`(全域到達。**所属の全件が到達可能**のときだけ) |
//! | `reachable > 0` | `Partial`(一部到達。残りが不明・不能・対象外のどれかは件数で見る) |
//! | `unknown > 0` | `Unknown`(到達可能は0件だが、範囲外の陸を通る経路があり得る。**到達不能へ丸めない**) |
//! | それ以外(`unreachable > 0`) | `Unreachable`(判定できた所属が、すべて確定した到達不能) |
//!
//! ステートに属さないプロヴィンスは、どのステートの件数にも加えない。別に `unassigned`(同じ件数の内訳)として持つ。
//! 出力はすべて `MapStateId` 昇順。入力(ノード・所属・通行条件)の列挙順に依存しない。

use super::connectivity::{ProvinceGraph, Reach, Reachability, UnreachableReason};
use super::ids::{MapStateId, ProvinceId};
use super::model::WorldModel;
use std::collections::{BTreeMap, HashMap, HashSet};

/// 1つの集合(ステートの所属、または未所属)の、`Reachability` の区分ごとの件数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReachCounts {
    /// 所属総数(下の5つの合計)。
    pub total: usize,
    pub reachable: usize,
    /// 確定した到達不能のうち、対象自身が明示条件で通行不可のもの。
    pub unreachable_not_passable: usize,
    /// 確定した到達不能のうち、範囲外の陸を通れると仮定しても経路が無いもの。
    pub unreachable_no_route: usize,
    pub unknown: usize,
    pub not_applicable: usize,
}

impl ReachCounts {
    fn add(&mut self, r: Reachability) {
        self.total += 1;
        match r {
            Reachability::Reachable => self.reachable += 1,
            Reachability::Unreachable(UnreachableReason::NotPassable) => self.unreachable_not_passable += 1,
            Reachability::Unreachable(UnreachableReason::NoRoute) => self.unreachable_no_route += 1,
            Reachability::Unknown => self.unknown += 1,
            Reachability::NotApplicable(_) => self.not_applicable += 1,
        }
    }

    /// 確定した到達不能の合計。
    pub fn unreachable(&self) -> usize {
        self.unreachable_not_passable + self.unreachable_no_route
    }

    /// 区分ごとの件数の合計が所属総数と一致するか(不変条件)。
    pub fn is_consistent(&self) -> bool {
        self.total == self.reachable + self.unreachable() + self.unknown + self.not_applicable
    }

    fn merge(&mut self, o: &ReachCounts) {
        self.total += o.total;
        self.reachable += o.reachable;
        self.unreachable_not_passable += o.unreachable_not_passable;
        self.unreachable_no_route += o.unreachable_no_route;
        self.unknown += o.unknown;
        self.not_applicable += o.not_applicable;
    }
}

/// ステート単位の要約区分(表は モジュール先頭)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateReachKind {
    /// 所属の全件が到達可能。
    Full,
    /// 一部だけ到達可能(ステート内部の分断・一部の通行不可・一部が不明、を含む)。
    Partial,
    /// 到達可能は0件で、判定できた所属はすべて確定した到達不能。
    Unreachable,
    /// 到達可能は0件だが、不明が1件以上(到達不能とは確定しない)。
    Unknown,
    /// 所属がすべて判定の対象外。
    NotApplicable,
    /// 所属が空(全域到達にも到達不能にもしない)。
    Empty,
}

fn kind_of(c: &ReachCounts) -> StateReachKind {
    if c.total == 0 {
        StateReachKind::Empty
    } else if c.not_applicable == c.total {
        StateReachKind::NotApplicable
    } else if c.reachable == c.total {
        StateReachKind::Full
    } else if c.reachable > 0 {
        StateReachKind::Partial
    } else if c.unknown > 0 {
        StateReachKind::Unknown
    } else {
        StateReachKind::Unreachable
    }
}

/// 1ステートの要約。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateReach {
    pub state: MapStateId,
    pub counts: ReachCounts,
    pub kind: StateReachKind,
}

/// 要約を作れない入力。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryError {
    /// `Reach` が、渡されたグラフに対するものではない(古い結果・別のグラフ)。
    ReachNotForGraph,
    /// 同じ `MapStateId` が所属表に複数ある。
    DuplicateState(MapStateId),
    /// 1つのプロヴィンスが複数のステートに載っている(見つけた最初のプロヴィンス)。
    ProvinceInMultipleStates(ProvinceId),
}

impl std::fmt::Display for SummaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SummaryError::ReachNotForGraph => write!(f, "the reach result does not belong to this graph (stale or from another graph)"),
            SummaryError::DuplicateState(s) => write!(f, "{s} appears more than once in the membership"),
            SummaryError::ProvinceInMultipleStates(p) => write!(f, "{p} is listed under more than one state"),
        }
    }
}

impl std::error::Error for SummaryError {}

/// ステート単位の要約(導出値。保存しない)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateReachSummary {
    /// `MapStateId` 昇順。
    states: Vec<StateReach>,
    /// どのステートにも属さないプロヴィンスの内訳(**どのステートの件数にも含まれない**)。
    unassigned: ReachCounts,
}

impl StateReachSummary {
    pub fn states(&self) -> &[StateReach] {
        &self.states
    }

    pub fn state(&self, id: MapStateId) -> Option<&StateReach> {
        self.states.binary_search_by_key(&id, |s| s.state).ok().map(|i| &self.states[i])
    }

    pub fn unassigned(&self) -> &ReachCounts {
        &self.unassigned
    }

    /// 全ステートの件数の合計(未所属を含まない)。
    pub fn assigned_totals(&self) -> ReachCounts {
        let mut t = ReachCounts::default();
        for s in &self.states {
            t.merge(&s.counts);
        }
        t
    }

    /// 全ステート+未所属の件数の合計。
    pub fn grand_totals(&self) -> ReachCounts {
        let mut t = self.assigned_totals();
        t.merge(&self.unassigned);
        t
    }

    /// 指定した区分のステート数。
    pub fn count_kind(&self, kind: StateReachKind) -> usize {
        self.states.iter().filter(|s| s.kind == kind).count()
    }

    fn finish(map: HashMap<MapStateId, ReachCounts>, unassigned: ReachCounts) -> StateReachSummary {
        let mut states: Vec<StateReach> = map.into_iter().map(|(state, counts)| StateReach { state, counts, kind: kind_of(&counts) }).collect();
        states.sort_by_key(|s| s.state);
        StateReachSummary { states, unassigned }
    }
}

/// グラフのノードのステートラベルから要約を導出する(`reach` は呼ばない。ノードを1回ずつ数えるだけで O(ノード数))。
/// 空のステートはグラフに現れないため、この経路では `Empty` は生じない(`summarize_model` を使う)。
/// `reach` が `graph` に対するものでなければ `Err`。
pub fn summarize_by_state(graph: &ProvinceGraph, reach: &Reach) -> Result<StateReachSummary, SummaryError> {
    if !reach.is_for(graph) {
        return Err(SummaryError::ReachNotForGraph);
    }
    let mut map: HashMap<MapStateId, ReachCounts> = HashMap::new();
    let mut unassigned = ReachCounts::default();
    // 署名が同じなので、両者のノード列(`ProvinceId` 昇順)は同じ順序・同じ長さ。
    for ((id, state), (rid, r)) in graph.node_states().zip(reach.iter()) {
        debug_assert_eq!(id, rid);
        match state {
            Some(s) => map.entry(s).or_default().add(r),
            None => unassigned.add(r),
        }
    }
    Ok(StateReachSummary::finish(map, unassigned))
}

/// 所属表(ステート → プロヴィンス)から要約を導出する。所属表に空のステートがあれば `Empty`、`reach` のグラフに無い
/// `ProvinceId` は `NotApplicable`(`UnknownProvince`)として数える。`unassigned_provinces` は未所属として別に数える。
pub fn summarize_membership(
    reach: &Reach,
    members: &[(MapStateId, &[ProvinceId])],
    unassigned_provinces: &[ProvinceId],
) -> Result<StateReachSummary, SummaryError> {
    let mut map: HashMap<MapStateId, ReachCounts> = HashMap::new();
    let mut seen_states: HashSet<MapStateId> = HashSet::new();
    let mut seen_provinces: HashSet<ProvinceId> = HashSet::new();
    for (state, provinces) in members {
        if !seen_states.insert(*state) {
            return Err(SummaryError::DuplicateState(*state));
        }
        let c = map.entry(*state).or_default();
        for p in *provinces {
            if !seen_provinces.insert(*p) {
                return Err(SummaryError::ProvinceInMultipleStates(*p));
            }
            c.add(reach.reachability(*p));
        }
    }
    let mut unassigned = ReachCounts::default();
    for p in unassigned_provinces {
        if !seen_provinces.insert(*p) {
            return Err(SummaryError::ProvinceInMultipleStates(*p));
        }
        unassigned.add(reach.reachability(*p));
    }
    Ok(StateReachSummary::finish(map, unassigned))
}

/// `WorldModel` の所属(正本)から要約を導出する(`summarize_membership` の薄い包み)。
pub fn summarize_model(model: &WorldModel, reach: &Reach) -> Result<StateReachSummary, SummaryError> {
    let members: Vec<(MapStateId, &[ProvinceId])> = model.states().iter().map(|s| (s.id, s.provinces.as_slice())).collect();
    summarize_membership(reach, &members, &model.unassigned_provinces())
}

/// 区分の表示名(CLI・報告用)。
pub fn kind_label(k: StateReachKind) -> &'static str {
    match k {
        StateReachKind::Full => "全域到達 (full)",
        StateReachKind::Partial => "一部到達 (partial)",
        StateReachKind::Unreachable => "到達不能 (unreachable)",
        StateReachKind::Unknown => "不明 (unknown)",
        StateReachKind::NotApplicable => "対象外 (not applicable)",
        StateReachKind::Empty => "所属なし (empty)",
    }
}

/// 要約を `MapStateId` → 区分の写像として取り出す(比較・テスト用)。
pub fn kinds(summary: &StateReachSummary) -> BTreeMap<MapStateId, StateReachKind> {
    summary.states().iter().map(|s| (s.state, s.kind)).collect()
}
