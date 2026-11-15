//! P32-034 (M6-a): 新経路の師団の初期配置と、**自国支配の陸上プロヴィンス間の日単位の移動**(純粋な Rust。Bevy・経済・保存・軍の旧型に依存しない)。
//!
//! 設計は `verification_logs/phase-32/p32-033/p32-033_m6_design.md`(§5・§6・§10)。**戦争関係・戦闘・占領・日次補給・前線/AI・UI・保存は含まない**(後続の単位)。
//! 旧経路(`military::movement`・`military::pathfinding`)の挙動のうち、次を引き継ぐ(旧コードを呼ばず、同じ規則を新しい型で実装する。比較は `tests/p32_034_map_movement_test.rs`):
//! - 移動命令は冪等: 現在地(Idle)・移動中の同じ目的地への命令は no-op。新しい命令は、経路が確定した時点で `destination`/`path`/`target=None`/`progress=0`/`Moving` を**まとめて**適用し、
//!   その時点では現在地を動かさない。拒否したときは何も変更しない。
//! - 経路: 辺の費用は 1、同費用の候補は `ProvinceId` 昇順(旧の `(費用, StateId)` 順と同じ)。通れるのは**所属国が支配するプロヴィンスだけ**(戦争関係が入る M6-b までは、敵支配も通れない)。
//!   河川は費用にも可否にも加えない。範囲外の陸・水域には `ProvinceId` が無いので通れない。
//! - 日次: `MapDivisionId` 昇順。日の初めに経路を検証し(次の移動先が通れなければ進入前に停止、残りの経路に通れない所があれば経路を切って現在のホップだけ続ける)、
//!   その後に移動する。1ホップの日数 = `max(1, 基準日数 × 費用 / (速度 × 補給係数))`(f32、旧と同じ式)。進捗が 1.0 に達した日に現在地を更新する。補給係数は **1.0 固定**(M6-d で補給計画に接続する)。
//!
//! 実行時状態であり、保存しない(M8 まで)。`ProvinceId`・`MapStateId`・旧 `StateId` を数値の一致で変換しない。

use super::connectivity::ProvinceGraph;
use super::ids::ProvinceId;
use crate::common::CountryId;
use std::collections::{BTreeMap, HashMap};

/// 新経路の師団ID(旧 `DivisionId` とは別の型。数値の一致で対応づけない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MapDivisionId(pub u32);

impl std::fmt::Display for MapDivisionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MapDivision#{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapDivisionStatus {
    Idle,
    Moving,
}

/// 新経路の師団(M6-a の範囲。旧 `Division` とは並行する別の型で、自動変換は無い)。
#[derive(Debug, Clone, PartialEq)]
pub struct MapDivision {
    pub id: MapDivisionId,
    pub owner: CountryId,
    /// 現在地。
    pub province: ProvinceId,
    /// 最終目的地(移動中だけ)。
    pub destination: Option<ProvinceId>,
    /// 今向かっているホップより先の残りの経路(先頭が次のホップ先)。
    pub path: Vec<ProvinceId>,
    /// 今向かっている1ホップ先。
    pub target: Option<ProvinceId>,
    /// 今のホップの進捗(0.0..1.0)。
    pub progress: f32,
    pub status: MapDivisionStatus,
    pub speed: f32,
}

impl MapDivision {
    /// 初期配置(Idle・命令なし)。
    pub fn new(id: MapDivisionId, owner: CountryId, province: ProvinceId, speed: f32) -> MapDivision {
        MapDivision { id, owner, province, destination: None, path: Vec::new(), target: None, progress: 0.0, status: MapDivisionStatus::Idle, speed }
    }
}

/// プロヴィンスごとの実効支配国(検証用の入力から作る実行時状態。**正式な所有・実効支配のデータではない**)。
/// 載っていないプロヴィンスは支配国なし(通れない)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProvinceControl(BTreeMap<ProvinceId, CountryId>);

impl ProvinceControl {
    pub fn new(map: BTreeMap<ProvinceId, CountryId>) -> ProvinceControl {
        ProvinceControl(map)
    }

    pub fn controller(&self, p: ProvinceId) -> Option<CountryId> {
        self.0.get(&p).copied()
    }

    /// 支配国を設定する(`None` で支配国なし)。M6-c の占領の更新口でもある。
    pub fn set(&mut self, p: ProvinceId, c: Option<CountryId>) {
        match c {
            Some(c) => {
                self.0.insert(p, c);
            }
            None => {
                self.0.remove(&p);
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (ProvinceId, CountryId)> + '_ {
        self.0.iter().map(|(p, c)| (*p, *c))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// 移動の検証用パラメータ。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementParams {
    /// 1ホップの基準日数(旧経路は 5.0)。
    pub base_days: f32,
}

/// 旧経路の既定の基準日数。
pub const DEFAULT_BASE_DAYS: f32 = 5.0;

/// 補給係数(M6-a は 1.0 固定。旧経路は `max(supply_ratio, 0.5)`)。
pub const SUPPLY_MODIFIER: f32 = 1.0;

/// 辺の費用(旧経路の `calculate_step_cost` と同じく常に 1。河川・地形は加えない)。
pub const STEP_COST: f32 = 1.0;

/// 構築の失敗(シナリオの検証を通った入力では生じない。直接構築の防御)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapMilitaryError {
    DuplicateDivision(MapDivisionId),
    InvalidSpeed(MapDivisionId),
    InvalidBaseDays,
}

impl std::fmt::Display for MapMilitaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateDivision(d) => write!(f, "{d} is defined more than once"),
            Self::InvalidSpeed(d) => write!(f, "{d} has a non-finite or non-positive speed"),
            Self::InvalidBaseDays => write!(f, "base_days must be finite and positive"),
        }
    }
}

impl std::error::Error for MapMilitaryError {}

/// 命令の結果(受理)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderOutcome {
    /// 新しい移動を開始した(`hops` は経路のホップ数)。
    Started { hops: usize },
    /// 既に目的地にいる(Idle)。何も変更していない。
    AlreadyThere,
    /// 既に同じ目的地へ移動中。何も変更していない。
    AlreadyMoving,
}

/// 命令の拒否(正常な結果。拒否しても師団の状態は一切変わらない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderRejection {
    UnknownDivision(MapDivisionId),
    /// グラフのノードでない(水域・範囲外の陸のセルには ID が無い)。
    UnknownProvince(ProvinceId),
    /// 目的地が所属国の支配でない(他国の支配、または支配国なし)。
    DestinationNotPassable { province: ProvinceId, controller: Option<CountryId> },
    /// 自国支配のプロヴィンスだけでは経路が無い。
    NoPath,
}

impl std::fmt::Display for OrderRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownDivision(d) => write!(f, "unknown-division: {d} does not exist"),
            Self::UnknownProvince(p) => write!(f, "unknown-province: {p} is not a province of the graph"),
            Self::DestinationNotPassable { province, controller: Some(c) } => {
                write!(f, "destination-not-passable: {province} is controlled by country {}", c.0)
            }
            Self::DestinationNotPassable { province, controller: None } => {
                write!(f, "destination-not-passable: {province} has no controller")
            }
            Self::NoPath => write!(f, "no-path: no route through provinces controlled by the division's country"),
        }
    }
}

/// 日次処理で起きたこと(表示・テスト用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveEvent {
    /// 1ホップを終えて `province` に着いた(最終目的地ではない場合を含む)。
    HopCompleted { division: MapDivisionId, province: ProvinceId },
    /// 最終目的地に着いて Idle になった。
    Arrived { division: MapDivisionId, province: ProvinceId },
    /// 次の移動先 `blocked` が通れなくなったので、`at` で進入前に停止した。
    Stopped { division: MapDivisionId, at: ProvinceId, blocked: ProvinceId },
    /// 残りの経路に通れない所ができたので、経路を切った(今のホップだけ続ける)。
    PathTruncated { division: MapDivisionId },
}

/// 新経路の師団と支配の集合(M6-a の実行時状態)。
#[derive(Debug, Clone, PartialEq)]
pub struct MapMilitary {
    divisions: BTreeMap<MapDivisionId, MapDivision>,
    control: ProvinceControl,
    params: MovementParams,
}

/// 所属国が支配するプロヴィンスだけを通る最短経路(費用 1、同費用は `ProvinceId` 昇順)。終点は `start` を含まず `end` を含む。
/// `start == end` は空の経路。終点の支配は呼び出し側が確認する(ここでは通行条件として扱うだけ)。経路が無ければ `None`。
///
/// 同費用の tie-break: 各階層の候補を `ProvinceId` 昇順に展開し、最初に見つけた親を採る(旧 `find_path` の `(費用, ID)` 順のヒープと同じ結果)。
pub fn find_land_path(graph: &ProvinceGraph, control: &ProvinceControl, owner: CountryId, start: ProvinceId, end: ProvinceId) -> Option<Vec<ProvinceId>> {
    if start == end {
        return Some(Vec::new());
    }
    let mut pred: HashMap<ProvinceId, ProvinceId> = HashMap::new();
    let mut frontier = vec![start];
    while !frontier.is_empty() {
        let mut next: Vec<ProvinceId> = Vec::new();
        for &x in &frontier {
            for n in graph.neighbor_ids(x) {
                if n == start || pred.contains_key(&n) || control.controller(n) != Some(owner) {
                    continue;
                }
                pred.insert(n, x);
                if n == end {
                    let mut path = vec![end];
                    let mut cur = end;
                    while let Some(&p) = pred.get(&cur) {
                        if p == start {
                            break;
                        }
                        path.push(p);
                        cur = p;
                    }
                    path.reverse();
                    return Some(path);
                }
                next.push(n);
            }
        }
        next.sort_unstable();
        frontier = next;
    }
    None
}

impl MapMilitary {
    pub fn new(divisions: Vec<MapDivision>, control: ProvinceControl, params: MovementParams) -> Result<MapMilitary, MapMilitaryError> {
        if !(params.base_days.is_finite() && params.base_days > 0.0) {
            return Err(MapMilitaryError::InvalidBaseDays);
        }
        let mut map = BTreeMap::new();
        for d in divisions {
            if !(d.speed.is_finite() && d.speed > 0.0) {
                return Err(MapMilitaryError::InvalidSpeed(d.id));
            }
            let id = d.id;
            if map.insert(id, d).is_some() {
                return Err(MapMilitaryError::DuplicateDivision(id));
            }
        }
        Ok(MapMilitary { divisions: map, control, params })
    }

    pub fn params(&self) -> MovementParams {
        self.params
    }

    pub fn control(&self) -> &ProvinceControl {
        &self.control
    }

    /// 支配国を設定する(命令後の通行条件の変更の検証用。占領は M6-c)。
    pub fn set_controller(&mut self, p: ProvinceId, c: Option<CountryId>) {
        self.control.set(p, c);
    }

    pub fn division(&self, id: MapDivisionId) -> Option<&MapDivision> {
        self.divisions.get(&id)
    }

    /// 師団(`MapDivisionId` 昇順)。
    pub fn divisions(&self) -> impl Iterator<Item = &MapDivision> + '_ {
        self.divisions.values()
    }

    /// 移動命令(旧 `start_or_maintain_movement` の新経路版)。受理されたときだけ状態を変える。
    pub fn issue_move_order(&mut self, graph: &ProvinceGraph, id: MapDivisionId, destination: ProvinceId) -> Result<OrderOutcome, OrderRejection> {
        let Some(d) = self.divisions.get(&id) else {
            return Err(OrderRejection::UnknownDivision(id));
        };
        if !graph.contains(destination) {
            return Err(OrderRejection::UnknownProvince(destination));
        }
        // 旧の冪等契約: 移動中は「同じ目的地」だけで判定する(現在地ではなく)。Idle は現在地で判定する。
        if d.status == MapDivisionStatus::Moving {
            if d.destination == Some(destination) {
                return Ok(OrderOutcome::AlreadyMoving);
            }
        } else if d.province == destination {
            return Ok(OrderOutcome::AlreadyThere);
        }
        let controller = self.control.controller(destination);
        if controller != Some(d.owner) {
            return Err(OrderRejection::DestinationNotPassable { province: destination, controller });
        }
        let Some(path) = find_land_path(graph, &self.control, d.owner, d.province, destination) else {
            return Err(OrderRejection::NoPath);
        };
        let hops = path.len();
        let d = self.divisions.get_mut(&id).expect("checked");
        d.destination = Some(destination);
        d.path = path;
        d.target = None;
        d.progress = 0.0;
        d.status = MapDivisionStatus::Moving;
        Ok(OrderOutcome::Started { hops })
    }

    /// 1日分の処理(旧 `MilitaryAction` の経路検証 → 移動)。`MapDivisionId` 昇順。
    pub fn advance_day(&mut self) -> Vec<MoveEvent> {
        let mut events = Vec::new();
        let ids: Vec<MapDivisionId> = self.divisions.keys().copied().collect();
        // 1. 経路の事前検証(旧 `validate_and_stop_invalid_movements`)
        for id in &ids {
            let d = self.divisions.get_mut(id).expect("id from keys");
            if d.status != MapDivisionStatus::Moving {
                continue;
            }
            if let Some(t) = d.target
                && self.control.controller(t) != Some(d.owner)
            {
                stop(d, t, &mut events);
                continue;
            }
            if d.path.iter().any(|p| self.control.controller(*p) != Some(d.owner)) {
                d.path.clear();
                d.destination = None;
                events.push(MoveEvent::PathTruncated { division: d.id });
            }
        }
        // 2. 移動(旧 `process_movement`)
        for id in &ids {
            let d = self.divisions.get_mut(id).expect("id from keys");
            if d.status != MapDivisionStatus::Moving {
                continue;
            }
            if d.target.is_none() {
                if !d.path.is_empty() {
                    d.target = Some(d.path.remove(0));
                    d.progress = 0.0;
                } else {
                    d.status = MapDivisionStatus::Idle;
                    d.destination = None;
                    d.progress = 0.0;
                    continue;
                }
            }
            let target = d.target.expect("set above");
            if self.control.controller(target) != Some(d.owner) {
                stop(d, target, &mut events);
                continue;
            }
            let movement_days = (self.params.base_days * STEP_COST / (d.speed * SUPPLY_MODIFIER)).max(1.0);
            let new_progress = d.progress + 1.0 / movement_days;
            if new_progress >= 1.0 {
                d.province = target;
                d.progress = 0.0;
                events.push(MoveEvent::HopCompleted { division: d.id, province: target });
                if !d.path.is_empty() {
                    d.target = Some(d.path.remove(0));
                } else {
                    d.target = None;
                    d.destination = None;
                    d.status = MapDivisionStatus::Idle;
                    events.push(MoveEvent::Arrived { division: d.id, province: target });
                }
            } else {
                d.progress = new_progress;
            }
        }
        events
    }
}

fn stop(d: &mut MapDivision, blocked: ProvinceId, events: &mut Vec<MoveEvent>) {
    d.status = MapDivisionStatus::Idle;
    d.target = None;
    d.destination = None;
    d.path.clear();
    d.progress = 0.0;
    events.push(MoveEvent::Stopped { division: d.id, at: d.province, blocked });
}
