//! P32-032 (M5-c): 補給連結性の**検証用シナリオ**(RON)の読込・検証と、既存の到達判定・ステート要約への接続。
//!
//! **このファイルの通行範囲は検証入力であり、正式な国境・所有・実効支配のデータではない。** 経済シナリオ
//! (`countries[].states`)の担当ステートや配列順から通行範囲を推定しない。経済・AI・軍・保存には接続しない
//! (輸送能力・補給量も計算しない)。到達可能は「十分な補給が届く」ではない。
//!
//! ## 入力形式(`format_version: 1`)
//! ```text
//! ( format_version: 1, description: "...",
//!   agents: [ ( label: 1, sources: [18309], passable_states: [20, 22], passable_provinces: [17903] ), ... ] )
//! ```
//! - `label`(必須): 主体の識別用ラベル(数値)。結果は**ラベル昇順**で出す(ファイルの記述順に依存しない)。
//! - `sources`(必須・1件以上): 補給源の `ProvinceId`。複数可。**すべて通行可能集合に含まれること**。
//! - `passable_provinces` / `passable_states`(少なくとも一方が必須。空リストの明示は可): 通行可能な範囲。
//!   `passable_states` は `MapStateId` の所属(`WorldModel` が正本)を `ExplicitPassable::from_states` で展開する
//!   (**名前つきの変換**。所有・支配の導出ではない)。
//! - 未知のフィールドは拒否する(`deny_unknown_fields`)。ID は型ごとに解決する(`ProvinceId`・`MapStateId` を数値一致で変換しない)。
//!
//! ## 検証規則(すべての問題を集めて報告する。種類は `SupplyScenarioIssue::code`)
//! - 形式: 読めない(`Io`)/ RON として不正・未知のフィールド・型違い(`Parse`)/ 非対応の版(`UnsupportedVersion`)は、解決の前に失敗する。
//! - 必須項目の欠落(`missing-field`)、主体なし(`no-agents`)、ラベルの重複(`duplicate-label`)。
//! - 補給源: 空(`no-sources`)、重複(`duplicate-source`)、未知の `ProvinceId`(`unknown-province`。退役済みの ID は `retired` を併記)。
//! - `passable_provinces`: 同じ ID の重複(`duplicate-passable-province`)、未知の ID(`unknown-province`)。
//! - `passable_states`: 同じ ID の重複(`duplicate-passable-state`)、未知のステート(`unknown-state`)、所属が空のステート(`empty-state`)。
//! - **併用時の規則**: 両方を併用してよく、通行範囲は和集合。ただし**展開後に重複する指定はエラー**
//!   (`overlap-province-in-state`: `passable_provinces` の ID が、同じ主体の `passable_states` に挙げたステートの所属でもある)。
//!   ステートに属さない(未所属の)既知の `ProvinceId` を `passable_provinces` に明示すれば、通行できる。
//! - 展開後の通行範囲が空(`empty-passable`)、補給源が通行可能集合に含まれない(`source-not-passable`)。
//! - 水域・範囲外の陸セルには `ProvinceId` が無いので、名指しできない(その番号は未知の `ProvinceId` として拒否される)。
//!   範囲外の陸の扱いは既存の判定規則(橋渡しにしない。「不明」の根拠にだけ使う)のまま。

use super::connectivity::{ExplicitPassable, Passability, ProvinceGraph, Reach, ReachError, reach};
use super::ids::{MapStateId, ProvinceId};
use super::model::WorldModel;
use super::state_reach::{StateReachSummary, SummaryError, summarize_model};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub const SUPPLY_SCENARIO_FORMAT_VERSION: u32 = 1;

/// 存在すれば `Some`、省略なら `None`(RON の `Some(..)` を書かせずに「欠落」と「空」を区別する)。
fn present<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

/// 1主体の記述(未解決。ID は数値のまま)。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEntry {
    #[serde(default, deserialize_with = "present")]
    pub label: Option<u32>,
    #[serde(default, deserialize_with = "present")]
    pub sources: Option<Vec<u32>>,
    #[serde(default, deserialize_with = "present")]
    pub passable_provinces: Option<Vec<u32>>,
    #[serde(default, deserialize_with = "present")]
    pub passable_states: Option<Vec<u32>>,
}

/// ファイルの内容(未解決)。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyScenarioFile {
    pub format_version: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default, deserialize_with = "present")]
    pub agents: Option<Vec<AgentEntry>>,
}

/// 問題のある主体の指し方(ラベルが無い主体は、ファイル内の位置)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentRef {
    Label(u32),
    /// 0始まりの記述位置(ラベルを持たない主体)。
    Index(usize),
}

impl std::fmt::Display for AgentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentRef::Label(l) => write!(f, "agent {l}"),
            AgentRef::Index(i) => write!(f, "agent at position {i} (no label)"),
        }
    }
}

/// 解決の問題(種類は `code`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupplyScenarioIssue {
    MissingField { agent: Option<AgentRef>, field: &'static str },
    NoAgents,
    DuplicateLabel(u32),
    NoSources(AgentRef),
    DuplicateSource { agent: AgentRef, province: ProvinceId },
    UnknownProvince { agent: AgentRef, field: &'static str, province: ProvinceId, retired: bool },
    DuplicatePassableProvince { agent: AgentRef, province: ProvinceId },
    DuplicatePassableState { agent: AgentRef, state: MapStateId },
    UnknownState { agent: AgentRef, state: MapStateId },
    EmptyState { agent: AgentRef, state: MapStateId },
    OverlapProvinceInState { agent: AgentRef, province: ProvinceId, state: MapStateId },
    EmptyPassable(AgentRef),
    SourceNotPassable { agent: AgentRef, province: ProvinceId },
}

impl SupplyScenarioIssue {
    /// 種類の識別子(安定。CLI の出力・テストで使う)。
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingField { .. } => "missing-field",
            Self::NoAgents => "no-agents",
            Self::DuplicateLabel(_) => "duplicate-label",
            Self::NoSources(_) => "no-sources",
            Self::DuplicateSource { .. } => "duplicate-source",
            Self::UnknownProvince { .. } => "unknown-province",
            Self::DuplicatePassableProvince { .. } => "duplicate-passable-province",
            Self::DuplicatePassableState { .. } => "duplicate-passable-state",
            Self::UnknownState { .. } => "unknown-state",
            Self::EmptyState { .. } => "empty-state",
            Self::OverlapProvinceInState { .. } => "overlap-province-in-state",
            Self::EmptyPassable(_) => "empty-passable",
            Self::SourceNotPassable { .. } => "source-not-passable",
        }
    }
}

impl std::fmt::Display for SupplyScenarioIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] ", self.code())?;
        match self {
            Self::MissingField { agent: Some(a), field } => write!(f, "{a}: required field `{field}` is missing"),
            Self::MissingField { agent: None, field } => write!(f, "required field `{field}` is missing"),
            Self::NoAgents => write!(f, "the scenario defines no agents"),
            Self::DuplicateLabel(l) => write!(f, "label {l} is used by more than one agent"),
            Self::NoSources(a) => write!(f, "{a}: `sources` is empty (at least one supply source is required)"),
            Self::DuplicateSource { agent, province } => write!(f, "{agent}: source {province} is listed more than once"),
            Self::UnknownProvince { agent, field, province, retired } => write!(
                f,
                "{agent}: `{field}` names {province}, which is not a province of the world{}",
                if *retired { " (it is a retired id)" } else { "" }
            ),
            Self::DuplicatePassableProvince { agent, province } => write!(f, "{agent}: {province} is listed more than once in `passable_provinces`"),
            Self::DuplicatePassableState { agent, state } => write!(f, "{agent}: {state} is listed more than once in `passable_states`"),
            Self::UnknownState { agent, state } => write!(f, "{agent}: `passable_states` names {state}, which is not a state of the world"),
            Self::EmptyState { agent, state } => write!(f, "{agent}: `passable_states` names {state}, which has no provinces"),
            Self::OverlapProvinceInState { agent, province, state } => write!(
                f,
                "{agent}: `passable_provinces` lists {province}, which already belongs to {state} named in `passable_states` (duplicate after expansion)"
            ),
            Self::EmptyPassable(a) => write!(f, "{a}: the passable range is empty after expansion"),
            Self::SourceNotPassable { agent, province } => write!(f, "{agent}: source {province} is not in the agent's passable range"),
        }
    }
}

/// 読み込み・検証の失敗。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupplyScenarioError {
    Io { path: String, message: String },
    /// RON として不正・未知のフィールド・型違い。
    Parse(String),
    UnsupportedVersion { found: u32, supported: u32 },
    /// 内容の検証に失敗(すべての問題)。
    Invalid(Vec<SupplyScenarioIssue>),
}

impl std::fmt::Display for SupplyScenarioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "cannot read '{path}': {message}"),
            Self::Parse(m) => write!(f, "the supply scenario is malformed: {m}"),
            Self::UnsupportedVersion { found, supported } => {
                write!(f, "the supply scenario has unsupported format_version {found} (supported: {supported})")
            }
            Self::Invalid(v) => {
                write!(f, "the supply scenario is invalid ({} issue(s)):", v.len())?;
                for i in v {
                    write!(f, "\n  {i}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for SupplyScenarioError {}

/// 解決済みの1主体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyAgent {
    pub label: u32,
    /// 補給源(昇順・重複なし。すべて通行可能)。
    pub sources: Vec<ProvinceId>,
    /// `passable_provinces` で明示した ID(昇順)。
    pub explicit_provinces: Vec<ProvinceId>,
    /// `passable_states` で指定したステート(昇順)。
    pub states: Vec<MapStateId>,
    /// 展開後の通行範囲(明示 ∪ ステート所属)。
    pub passable: ExplicitPassable,
}

/// 解決済みのシナリオ(主体は**ラベル昇順**)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyScenario {
    pub description: String,
    pub agents: Vec<SupplyAgent>,
}

/// 1主体の判定結果(既存の `reach` と `summarize_model` の出力)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOutcome {
    pub label: u32,
    pub reach: Reach,
    pub summary: StateReachSummary,
}

/// 評価の失敗(検証済みのシナリオでは生じないはずの、内部の不整合)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluateError {
    Reach { label: u32, error: ReachError },
    Summary { label: u32, error: SummaryError },
}

impl std::fmt::Display for EvaluateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reach { label, error } => write!(f, "agent {label}: {error}"),
            Self::Summary { label, error } => write!(f, "agent {label}: {error}"),
        }
    }
}

impl std::error::Error for EvaluateError {}

#[derive(Deserialize)]
struct Header {
    format_version: u32,
}

/// RON の本文を読み、版を確認する(ID の解決はしない)。
pub fn parse_supply_scenario(text: &str) -> Result<SupplyScenarioFile, SupplyScenarioError> {
    let h: Header = ron::from_str(text).map_err(|e| SupplyScenarioError::Parse(e.to_string()))?;
    if h.format_version != SUPPLY_SCENARIO_FORMAT_VERSION {
        return Err(SupplyScenarioError::UnsupportedVersion { found: h.format_version, supported: SUPPLY_SCENARIO_FORMAT_VERSION });
    }
    ron::from_str(text).map_err(|e| SupplyScenarioError::Parse(e.to_string()))
}

/// ファイルを読み、解決する(読み取りのみ)。
pub fn load_supply_scenario(path: &str, graph: &ProvinceGraph, model: &WorldModel) -> Result<SupplyScenario, SupplyScenarioError> {
    let text = std::fs::read_to_string(path).map_err(|e| SupplyScenarioError::Io { path: path.to_string(), message: e.to_string() })?;
    let file = parse_supply_scenario(&text)?;
    resolve_supply_scenario(&file, graph, model).map_err(SupplyScenarioError::Invalid)
}

/// 未解決の内容を、グラフと所属(`model`)に対して検証・解決する。問題はすべて集めて返す。
/// `graph` は `model` から作ったものであること(通行範囲の展開は `ExplicitPassable::from_states` を使う)。
pub fn resolve_supply_scenario(file: &SupplyScenarioFile, graph: &ProvinceGraph, model: &WorldModel) -> Result<SupplyScenario, Vec<SupplyScenarioIssue>> {
    let mut issues: Vec<SupplyScenarioIssue> = Vec::new();
    let Some(entries) = file.agents.as_ref() else {
        return Err(vec![SupplyScenarioIssue::MissingField { agent: None, field: "agents" }]);
    };
    if entries.is_empty() {
        return Err(vec![SupplyScenarioIssue::NoAgents]);
    }
    let retired: BTreeSet<ProvinceId> = model.retired_ids().iter().copied().collect();
    let mut agents: Vec<SupplyAgent> = Vec::new();
    let mut label_count: BTreeMap<u32, usize> = BTreeMap::new();
    for e in entries {
        if let Some(l) = e.label {
            *label_count.entry(l).or_default() += 1;
        }
    }
    for (l, n) in &label_count {
        if *n > 1 {
            issues.push(SupplyScenarioIssue::DuplicateLabel(*l));
        }
    }
    for (index, e) in entries.iter().enumerate() {
        let who = match e.label {
            Some(l) => AgentRef::Label(l),
            None => AgentRef::Index(index),
        };
        let before = issues.len();
        if e.label.is_none() {
            issues.push(SupplyScenarioIssue::MissingField { agent: Some(who), field: "label" });
        }
        if e.sources.is_none() {
            issues.push(SupplyScenarioIssue::MissingField { agent: Some(who), field: "sources" });
        }
        if e.passable_provinces.is_none() && e.passable_states.is_none() {
            issues.push(SupplyScenarioIssue::MissingField { agent: Some(who), field: "passable_provinces or passable_states" });
        }
        let unknown = |field: &'static str, p: ProvinceId| SupplyScenarioIssue::UnknownProvince { agent: who, field, province: p, retired: retired.contains(&p) };

        // 補給源
        let mut sources: BTreeSet<ProvinceId> = BTreeSet::new();
        if let Some(list) = &e.sources {
            if list.is_empty() {
                issues.push(SupplyScenarioIssue::NoSources(who));
            }
            for s in list {
                let p = ProvinceId(*s);
                if !sources.insert(p) {
                    issues.push(SupplyScenarioIssue::DuplicateSource { agent: who, province: p });
                } else if !graph.contains(p) {
                    issues.push(unknown("sources", p));
                }
            }
        }
        // 明示のプロヴィンス
        let mut explicit: BTreeSet<ProvinceId> = BTreeSet::new();
        if let Some(list) = &e.passable_provinces {
            for s in list {
                let p = ProvinceId(*s);
                if !explicit.insert(p) {
                    issues.push(SupplyScenarioIssue::DuplicatePassableProvince { agent: who, province: p });
                } else if !graph.contains(p) {
                    issues.push(unknown("passable_provinces", p));
                }
            }
        }
        // ステート指定
        let mut states: BTreeSet<MapStateId> = BTreeSet::new();
        let mut usable_states: Vec<MapStateId> = Vec::new();
        if let Some(list) = &e.passable_states {
            for s in list {
                let st = MapStateId(*s);
                if !states.insert(st) {
                    issues.push(SupplyScenarioIssue::DuplicatePassableState { agent: who, state: st });
                    continue;
                }
                match model.provinces_of(st) {
                    None => issues.push(SupplyScenarioIssue::UnknownState { agent: who, state: st }),
                    Some([]) => issues.push(SupplyScenarioIssue::EmptyState { agent: who, state: st }),
                    Some(_) => usable_states.push(st),
                }
            }
        }
        // 展開後の重複(明示の ID が、挙げたステートの所属でもある)
        for p in &explicit {
            if let Some(st) = model.state_of(*p)
                && states.contains(&st)
                && usable_states.contains(&st)
            {
                issues.push(SupplyScenarioIssue::OverlapProvinceInState { agent: who, province: *p, state: st });
            }
        }
        if issues.len() != before {
            continue; // 問題のある主体は、通行範囲の検査(空・補給源)で重ねて報告しない
        }
        // 展開と和集合(既存の ExplicitPassable を使う)
        let explicit_vec: Vec<ProvinceId> = explicit.iter().copied().collect();
        let from_explicit = ExplicitPassable::new(graph, explicit_vec.iter().copied()).expect("ids were checked against the graph");
        let from_states = ExplicitPassable::from_states(graph, &usable_states).expect("states were checked to have provinces");
        let passable = from_explicit.union(&from_states);
        if passable.is_empty() {
            issues.push(SupplyScenarioIssue::EmptyPassable(who));
            continue;
        }
        let mut not_passable = false;
        for s in &sources {
            if !passable.passable(*s) {
                issues.push(SupplyScenarioIssue::SourceNotPassable { agent: who, province: *s });
                not_passable = true;
            }
        }
        if not_passable {
            continue;
        }
        agents.push(SupplyAgent {
            label: e.label.expect("checked"),
            sources: sources.into_iter().collect(),
            explicit_provinces: explicit_vec,
            states: usable_states.into_iter().collect::<BTreeSet<_>>().into_iter().collect(),
            passable,
        });
    }
    if !issues.is_empty() {
        return Err(issues);
    }
    agents.sort_by_key(|a| a.label);
    Ok(SupplyScenario { description: file.description.clone(), agents })
}

/// 各主体について、既存の `reach`(P32-030)と `summarize_model`(P32-031)を呼ぶ(ラベル昇順)。
/// 探索・要約の処理はここに持たない。
pub fn evaluate_supply_scenario(graph: &ProvinceGraph, model: &WorldModel, scenario: &SupplyScenario) -> Result<Vec<AgentOutcome>, EvaluateError> {
    scenario
        .agents
        .iter()
        .map(|a| {
            let r = reach(graph, &a.sources, &a.passable).map_err(|error| EvaluateError::Reach { label: a.label, error })?;
            let summary = summarize_model(model, &r).map_err(|error| EvaluateError::Summary { label: a.label, error })?;
            Ok(AgentOutcome { label: a.label, reach: r, summary })
        })
        .collect()
}
