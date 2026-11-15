//! P32-034 (M6-a): 師団の初期配置と初期の支配を与える**検証用シナリオ**(RON)の読込・検証・解決。
//!
//! **このファイルの所有・支配は検証入力であり、正式な全世界の所有・実効支配データではない。** 経済シナリオ(`countries[].states`)や
//! 補給検証用シナリオ(`supply_connectivity_scenario.ron`)の通行範囲から、所有・支配を推定しない(別のデータ)。
//! `ProvinceId` / `MapStateId` / 旧 `StateId` / 国家ラベル / `CountryId` は、このファイルが書いた明示の対応と、所属情報(`WorldModel` が正本)だけで解決する。
//!
//! ## 入力形式(`format_version: 1`、未知のフィールドは拒否)
//! ```text
//! ( format_version: 1, description: "...", base_days: 5.0,
//!   countries: [ (label: 1, country_id: 1), ... ],            // 国家ラベル ↔ CountryId(旧経路と同じ型)の対応表
//!   state_owners: [ (state: 20, country: 1), ... ],          // ステートごとの所有国(country は国家ラベル)
//!   controller_overrides: [ (province: 17903, country: 1) ],  // プロヴィンスごとの初期支配国の上書き(任意)
//!   divisions: [ (id: 1, country: 1, province: 18309, speed: 1.0), ... ] )
//! ```
//! ## 初期の支配(プロヴィンスごと。優先順)
//! 1. `controller_overrides` に明示があれば、それ。
//! 2. 所属ステートがあり、そのステートの `state_owners` があれば、その所有国。
//! 3. それ以外(ステート未所属で明示なし、または所有国の記述がないステート)は**支配国なし**(通れない)。
//!
//! ## 検証(すべての問題を集めて報告。種類は `code`)
//! 形式(読めない / RON として不正・未知のフィールド・必須項目の欠落・型違い / 非対応の版)は解決の前に失敗する。
//! 内容: 国家ラベルの重複・`country_id` の重複、ステートの所有国・上書き・師団の重複、解決できない国家ラベル、未知のステート・プロヴィンス、
//! 初期位置が所属国の支配でない、非有限・非正の速度・基準日数。

use super::connectivity::ProvinceGraph;
use super::ids::{MapStateId, ProvinceId};
use super::model::WorldModel;
use super::movement::{CombatStats, MapDivision, MapDivisionId, MapMilitary, MovementParams, ProvinceControl};
use super::supply::SupplyCountryConfig;
use crate::common::CountryId;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub const DIVISION_SCENARIO_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountryEntry {
    pub label: u32,
    pub country_id: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateOwnerEntry {
    pub state: u32,
    /// 国家ラベル。
    pub country: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerOverrideEntry {
    pub province: u32,
    /// 国家ラベル。
    pub country: u32,
}

/// 戦闘能力(省略した項目は標準値 = `CombatStats::standard()`。M6-b)。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CombatEntry {
    pub manpower: u64,
    pub max_manpower: u64,
    pub equipment: f64,
    pub max_equipment: f64,
    pub organization: f32,
    pub max_organization: f32,
    pub attack_power: i32,
    pub defense_power: i32,
}

impl Default for CombatEntry {
    fn default() -> CombatEntry {
        let c = CombatStats::standard();
        CombatEntry {
            manpower: c.manpower,
            max_manpower: c.max_manpower,
            equipment: c.equipment,
            max_equipment: c.max_equipment,
            organization: c.organization,
            max_organization: c.max_organization,
            attack_power: c.attack_power,
            defense_power: c.defense_power,
        }
    }
}

impl CombatEntry {
    pub fn to_stats(&self) -> CombatStats {
        CombatStats {
            manpower: self.manpower,
            max_manpower: self.max_manpower,
            equipment: self.equipment,
            max_equipment: self.max_equipment,
            organization: self.organization,
            max_organization: self.max_organization,
            attack_power: self.attack_power,
            defense_power: self.defense_power,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DivisionEntry {
    pub id: u32,
    /// 国家ラベル。
    pub country: u32,
    pub province: u32,
    pub speed: f32,
    /// 戦闘能力(省略可。省略した項目は標準値)。
    #[serde(default)]
    pub combat: CombatEntry,
}

/// 国家ごとの補給の設定(M6-d)。首都の `ProvinceId` と、初期の軍需物資の備蓄。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyCountryEntry {
    /// 国家ラベル。
    pub country: u32,
    pub capital: u32,
    pub military_supplies: f64,
}

/// 師団の補給消費量(旧 `DivisionDefinition::supply_usage` に相当。既存の需要計算の入力)。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyUsageEntry {
    pub division: u32,
    pub supply_usage: f32,
}

/// 補給の設定(M6-d。省略または `countries` が空なら補給なしで、M6-a〜c と同じ挙動)。
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct SupplySetup {
    pub countries: Vec<SupplyCountryEntry>,
    pub divisions: Vec<SupplyUsageEntry>,
}

/// 交戦する国の組(国家ラベル。双方向。順序は意味を持たない)。M6-b。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WarEntry {
    pub a: u32,
    pub b: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DivisionScenarioFile {
    pub format_version: u32,
    #[serde(default)]
    pub description: String,
    pub base_days: f32,
    pub countries: Vec<CountryEntry>,
    pub state_owners: Vec<StateOwnerEntry>,
    #[serde(default)]
    pub controller_overrides: Vec<ControllerOverrideEntry>,
    /// 交戦する国の組(省略可。省略したシナリオは M6-a と同じく交戦なし)。
    #[serde(default)]
    pub wars: Vec<WarEntry>,
    pub divisions: Vec<DivisionEntry>,
    /// 国家ごとの首都・初期の備蓄と、師団の補給消費量(省略可。M6-d)。
    #[serde(default)]
    pub supply: SupplySetup,
}

/// どこに書かれた参照か(問題の報告用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefSite {
    StateOwner(u32),
    Override(u32),
    Division(u32),
    War(usize),
    SupplyCountry(u32),
}

impl std::fmt::Display for RefSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefSite::StateOwner(s) => write!(f, "state_owners (state {s})"),
            RefSite::Override(p) => write!(f, "controller_overrides (province {p})"),
            RefSite::Division(d) => write!(f, "divisions (id {d})"),
            RefSite::War(i) => write!(f, "wars (entry {i})"),
            RefSite::SupplyCountry(l) => write!(f, "supply.countries (country {l})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DivisionScenarioIssue {
    DuplicateCountryLabel(u32),
    DuplicateCountryId(usize),
    UnresolvedCountry { site: RefSite, label: u32 },
    DuplicateStateOwner(MapStateId),
    UnknownState(MapStateId),
    DuplicateOverride(ProvinceId),
    UnknownProvince { site: RefSite, province: ProvinceId },
    DuplicateDivision(u32),
    /// 初期位置の支配国が、師団の所属国と一致しない(`actual` は支配国なしなら `None`)。
    InitialPositionNotControlled { division: u32, province: ProvinceId, expected: CountryId, actual: Option<CountryId> },
    InvalidSpeed { division: u32 },
    InvalidBaseDays,
    /// 自国との交戦(同じ国家ラベル、または同じ `CountryId` に解決される組)。
    SelfWar { index: usize },
    /// 同じ組(逆順を含む)の重複。
    DuplicateWar { index: usize },
    /// 戦闘能力の値が不正(有限でない・範囲外)。
    InvalidCombat { division: u32 },
    /// 補給の設定で、同じ国家が複数ある。
    SupplyDuplicateCountry(u32),
    /// 補給の設定の首都が、世界に無い `ProvinceId`。
    SupplyUnknownCapital { country: u32, province: ProvinceId },
    /// 補給の設定の首都が、初期に所属国の支配下にない(`actual` は支配国なしなら `None`)。
    SupplyCapitalNotControlled { country: u32, province: ProvinceId, actual: Option<CountryId> },
    /// 補給の設定の備蓄が不正(有限でない・負・値域外)。
    SupplyInvalidStockpile { country: u32 },
    /// 補給消費量の設定が、存在しない師団を指している。
    SupplyUnknownDivision(u32),
    /// 同じ師団の補給消費量が複数ある。
    SupplyDuplicateUsage(u32),
    /// 補給消費量が不正(有限でない・負・値域外)。
    SupplyInvalidUsage { division: u32 },
    /// 補給の設定がある国家の師団に、補給消費量の設定がない(不足項目)。
    SupplyMissingUsage { division: u32 },
    /// 補給の設定がない国家の師団に、補給消費量が設定されている。
    SupplyUsageWithoutCountry { division: u32 },
}

impl DivisionScenarioIssue {
    pub fn code(&self) -> &'static str {
        match self {
            Self::DuplicateCountryLabel(_) => "duplicate-country-label",
            Self::DuplicateCountryId(_) => "duplicate-country-id",
            Self::UnresolvedCountry { .. } => "unresolved-country",
            Self::DuplicateStateOwner(_) => "duplicate-state-owner",
            Self::UnknownState(_) => "unknown-state",
            Self::DuplicateOverride(_) => "duplicate-override",
            Self::UnknownProvince { .. } => "unknown-province",
            Self::DuplicateDivision(_) => "duplicate-division",
            Self::InitialPositionNotControlled { .. } => "initial-position-not-controlled",
            Self::InvalidSpeed { .. } => "invalid-speed",
            Self::InvalidBaseDays => "invalid-base-days",
            Self::SelfWar { .. } => "self-war",
            Self::DuplicateWar { .. } => "duplicate-war",
            Self::InvalidCombat { .. } => "invalid-combat",
            Self::SupplyDuplicateCountry(_) => "supply-duplicate-country",
            Self::SupplyUnknownCapital { .. } => "supply-unknown-capital",
            Self::SupplyCapitalNotControlled { .. } => "supply-capital-not-controlled",
            Self::SupplyInvalidStockpile { .. } => "supply-invalid-stockpile",
            Self::SupplyUnknownDivision(_) => "supply-unknown-division",
            Self::SupplyDuplicateUsage(_) => "supply-duplicate-usage",
            Self::SupplyInvalidUsage { .. } => "supply-invalid-usage",
            Self::SupplyMissingUsage { .. } => "supply-missing-usage",
            Self::SupplyUsageWithoutCountry { .. } => "supply-usage-without-country",
        }
    }
}

impl std::fmt::Display for DivisionScenarioIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] ", self.code())?;
        match self {
            Self::DuplicateCountryLabel(l) => write!(f, "country label {l} is defined more than once"),
            Self::DuplicateCountryId(c) => write!(f, "country_id {c} is used by more than one label"),
            Self::UnresolvedCountry { site, label } => write!(f, "{site}: country label {label} is not defined in `countries`"),
            Self::DuplicateStateOwner(s) => write!(f, "{s} has more than one owner entry"),
            Self::UnknownState(s) => write!(f, "state_owners names {s}, which is not a state of the world"),
            Self::DuplicateOverride(p) => write!(f, "controller_overrides lists {p} more than once"),
            Self::UnknownProvince { site, province } => write!(f, "{site}: {province} is not a province of the world"),
            Self::DuplicateDivision(d) => write!(f, "division id {d} is defined more than once"),
            Self::InitialPositionNotControlled { division, province, expected, actual: Some(a) } => {
                write!(f, "division {division} starts at {province}, which is controlled by country {} (not its own country {})", a.0, expected.0)
            }
            Self::InitialPositionNotControlled { division, province, expected, actual: None } => {
                write!(f, "division {division} starts at {province}, which has no controller (its country is {})", expected.0)
            }
            Self::InvalidSpeed { division } => write!(f, "division {division}: speed must be finite and positive"),
            Self::InvalidBaseDays => write!(f, "base_days must be finite and positive"),
            Self::SelfWar { index } => write!(f, "wars (entry {index}): a country cannot be at war with itself"),
            Self::DuplicateWar { index } => write!(f, "wars (entry {index}): the same pair (in either order) is listed more than once"),
            Self::InvalidCombat { division } => write!(f, "division {division}: combat stats must be finite and in range (manpower <= max_manpower, 0 <= equipment <= max_equipment, 0 <= organization <= max_organization, maxima positive)"),
            Self::SupplyDuplicateCountry(l) => write!(f, "supply.countries: country label {l} is listed more than once"),
            Self::SupplyUnknownCapital { country, province } => write!(f, "supply.countries (country {country}): the capital {province} is not a province of the world"),
            Self::SupplyCapitalNotControlled { country, province, actual: Some(a) } => {
                write!(f, "supply.countries (country {country}): the capital {province} is initially controlled by country {} (not by the country itself)", a.0)
            }
            Self::SupplyCapitalNotControlled { country, province, actual: None } => write!(f, "supply.countries (country {country}): the capital {province} initially has no controller"),
            Self::SupplyInvalidStockpile { country } => write!(f, "supply.countries (country {country}): military_supplies must be finite, non-negative and within the supported range"),
            Self::SupplyUnknownDivision(d) => write!(f, "supply.divisions names division {d}, which is not defined in `divisions`"),
            Self::SupplyDuplicateUsage(d) => write!(f, "supply.divisions: division {d} is listed more than once"),
            Self::SupplyInvalidUsage { division } => write!(f, "supply.divisions (division {division}): supply_usage must be finite, non-negative and within the supported range"),
            Self::SupplyMissingUsage { division } => write!(f, "division {division} belongs to a country with a supply setup but has no entry in supply.divisions"),
            Self::SupplyUsageWithoutCountry { division } => write!(f, "supply.divisions names division {division}, whose country has no entry in supply.countries"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DivisionScenarioError {
    Io { path: String, message: String },
    /// RON として不正・未知のフィールド・必須項目の欠落・型違い。
    Parse(String),
    UnsupportedVersion { found: u32, supported: u32 },
    Invalid(Vec<DivisionScenarioIssue>),
}

impl std::fmt::Display for DivisionScenarioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "cannot read '{path}': {message}"),
            Self::Parse(m) => write!(f, "the division scenario is malformed: {m}"),
            Self::UnsupportedVersion { found, supported } => {
                write!(f, "the division scenario has unsupported format_version {found} (supported: {supported})")
            }
            Self::Invalid(v) => {
                write!(f, "the division scenario is invalid ({} issue(s)):", v.len())?;
                for i in v {
                    write!(f, "\n  {i}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for DivisionScenarioError {}

/// 解決済みのシナリオ(検証用の入力。正式な所有・支配データではない)。
#[derive(Debug, Clone, PartialEq)]
pub struct DivisionScenario {
    pub description: String,
    pub params: MovementParams,
    /// 国家ラベル → `CountryId`(ラベル昇順)。
    pub countries: BTreeMap<u32, CountryId>,
    /// 初期の実効支配(プロヴィンスごと)。
    pub control: ProvinceControl,
    /// ステートごとの法的所有国(検証入力。M6-e のステート単位の集計に使う。`MapMilitary` には入らない)。
    pub state_owners: BTreeMap<MapStateId, CountryId>,
    /// 交戦する国の組(双方向。`CountryId`)。
    pub wars: Vec<(CountryId, CountryId)>,
    /// 国家ごとの補給の設定(首都・初期の備蓄。空なら補給なし)。師団の補給消費量は各師団の `supply_usage`。
    pub supply: Vec<SupplyCountryConfig>,
    /// 師団(`MapDivisionId` 昇順)。
    pub divisions: Vec<MapDivision>,
}

impl DivisionScenario {
    /// 実行時状態を作る(検証済みの入力なので失敗しない)。
    pub fn into_military(self) -> MapMilitary {
        MapMilitary::new(self.divisions, self.control, self.params).and_then(|m| m.with_wars(self.wars)).and_then(|m| m.with_supply(self.supply)).expect("a validated scenario builds")
    }

    /// 国家ラベル(`CountryId` から)。対応表に無ければ `None`。
    pub fn label_of(&self, c: CountryId) -> Option<u32> {
        self.countries.iter().find(|(_, id)| **id == c).map(|(l, _)| *l)
    }
}

#[derive(Deserialize)]
struct Header {
    format_version: u32,
}

pub fn parse_division_scenario(text: &str) -> Result<DivisionScenarioFile, DivisionScenarioError> {
    let h: Header = ron::from_str(text).map_err(|e| DivisionScenarioError::Parse(e.to_string()))?;
    if h.format_version != DIVISION_SCENARIO_FORMAT_VERSION {
        return Err(DivisionScenarioError::UnsupportedVersion { found: h.format_version, supported: DIVISION_SCENARIO_FORMAT_VERSION });
    }
    ron::from_str(text).map_err(|e| DivisionScenarioError::Parse(e.to_string()))
}

pub fn load_division_scenario(path: &str, graph: &ProvinceGraph, model: &WorldModel) -> Result<DivisionScenario, DivisionScenarioError> {
    let text = std::fs::read_to_string(path).map_err(|e| DivisionScenarioError::Io { path: path.to_string(), message: e.to_string() })?;
    let file = parse_division_scenario(&text)?;
    resolve_division_scenario(&file, graph, model).map_err(DivisionScenarioError::Invalid)
}

/// 未解決の内容を検証・解決する。問題はすべて集めて返す。
pub fn resolve_division_scenario(file: &DivisionScenarioFile, graph: &ProvinceGraph, model: &WorldModel) -> Result<DivisionScenario, Vec<DivisionScenarioIssue>> {
    let mut issues: Vec<DivisionScenarioIssue> = Vec::new();
    if !(file.base_days.is_finite() && file.base_days > 0.0) {
        issues.push(DivisionScenarioIssue::InvalidBaseDays);
    }
    // 国家の対応表
    let mut countries: BTreeMap<u32, CountryId> = BTreeMap::new();
    let mut seen_ids: BTreeSet<usize> = BTreeSet::new();
    let mut dup_labels: BTreeSet<u32> = BTreeSet::new();
    let mut dup_ids: BTreeSet<usize> = BTreeSet::new();
    for c in &file.countries {
        if countries.insert(c.label, CountryId(c.country_id)).is_some() {
            dup_labels.insert(c.label);
        }
        if !seen_ids.insert(c.country_id) {
            dup_ids.insert(c.country_id);
        }
    }
    issues.extend(dup_labels.into_iter().map(DivisionScenarioIssue::DuplicateCountryLabel));
    issues.extend(dup_ids.into_iter().map(DivisionScenarioIssue::DuplicateCountryId));
    let resolve = |site: RefSite, label: u32, issues: &mut Vec<DivisionScenarioIssue>| -> Option<CountryId> {
        let r = countries.get(&label).copied();
        if r.is_none() {
            issues.push(DivisionScenarioIssue::UnresolvedCountry { site, label });
        }
        r
    };
    // ステートの所有国
    let mut owners: BTreeMap<MapStateId, CountryId> = BTreeMap::new();
    let mut seen_states: BTreeSet<MapStateId> = BTreeSet::new();
    for e in &file.state_owners {
        let st = MapStateId(e.state);
        let c = resolve(RefSite::StateOwner(e.state), e.country, &mut issues);
        if !seen_states.insert(st) {
            issues.push(DivisionScenarioIssue::DuplicateStateOwner(st));
            continue;
        }
        if model.state(st).is_none() {
            issues.push(DivisionScenarioIssue::UnknownState(st));
            continue;
        }
        if let Some(c) = c {
            owners.insert(st, c);
        }
    }
    // プロヴィンスごとの上書き
    let mut overrides: BTreeMap<ProvinceId, CountryId> = BTreeMap::new();
    let mut seen_over: BTreeSet<ProvinceId> = BTreeSet::new();
    for e in &file.controller_overrides {
        let p = ProvinceId(e.province);
        let c = resolve(RefSite::Override(e.province), e.country, &mut issues);
        if !seen_over.insert(p) {
            issues.push(DivisionScenarioIssue::DuplicateOverride(p));
            continue;
        }
        if !graph.contains(p) {
            issues.push(DivisionScenarioIssue::UnknownProvince { site: RefSite::Override(e.province), province: p });
            continue;
        }
        if let Some(c) = c {
            overrides.insert(p, c);
        }
    }
    // 交戦する国の組(双方向。自国との組・重複(逆順を含む)を区別する)
    let mut wars: Vec<(CountryId, CountryId)> = Vec::new();
    let mut seen_wars: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (i, w) in file.wars.iter().enumerate() {
        let a = resolve(RefSite::War(i), w.a, &mut issues);
        let b = resolve(RefSite::War(i), w.b, &mut issues);
        let (Some(a), Some(b)) = (a, b) else { continue };
        if a == b {
            issues.push(DivisionScenarioIssue::SelfWar { index: i });
            continue;
        }
        if !seen_wars.insert((a.0.min(b.0), a.0.max(b.0))) {
            issues.push(DivisionScenarioIssue::DuplicateWar { index: i });
            continue;
        }
        wars.push((a, b));
    }
    // 初期の実効支配(上書き > ステートの所有国 > なし)。所属は所属情報(WorldModel)から解決する。
    let mut control: BTreeMap<ProvinceId, CountryId> = BTreeMap::new();
    for p in graph.node_ids() {
        let c = overrides.get(&p).copied().or_else(|| model.state_of(p).and_then(|s| owners.get(&s).copied()));
        if let Some(c) = c {
            control.insert(p, c);
        }
    }
    let control = ProvinceControl::new(control);
    // 師団
    let mut divisions: Vec<MapDivision> = Vec::new();
    let mut seen_div: BTreeSet<u32> = BTreeSet::new();
    for e in &file.divisions {
        let owner = resolve(RefSite::Division(e.id), e.country, &mut issues);
        if !seen_div.insert(e.id) {
            issues.push(DivisionScenarioIssue::DuplicateDivision(e.id));
            continue;
        }
        let p = ProvinceId(e.province);
        let known = graph.contains(p);
        if !known {
            issues.push(DivisionScenarioIssue::UnknownProvince { site: RefSite::Division(e.id), province: p });
        }
        if !(e.speed.is_finite() && e.speed > 0.0) {
            issues.push(DivisionScenarioIssue::InvalidSpeed { division: e.id });
        }
        if !e.combat.to_stats().is_valid() {
            issues.push(DivisionScenarioIssue::InvalidCombat { division: e.id });
        }
        if let (Some(owner), true) = (owner, known) {
            let actual = control.controller(p);
            if actual != Some(owner) {
                issues.push(DivisionScenarioIssue::InitialPositionNotControlled { division: e.id, province: p, expected: owner, actual });
            } else {
                divisions.push(MapDivision::new(MapDivisionId(e.id), owner, p, e.speed).with_combat(e.combat.to_stats()));
            }
        }
    }
    // 補給の設定(M6-d。`countries` が空なら補給なし = 従来の挙動。設定があれば、未知ID・重複・無効な数値・不足項目を検証する)
    let mut supply: Vec<SupplyCountryConfig> = Vec::new();
    let mut supply_labels: BTreeMap<u32, CountryId> = BTreeMap::new();
    let mut seen_supply: BTreeSet<u32> = BTreeSet::new();
    for e in &file.supply.countries {
        let c = resolve(RefSite::SupplyCountry(e.country), e.country, &mut issues);
        if !seen_supply.insert(e.country) {
            issues.push(DivisionScenarioIssue::SupplyDuplicateCountry(e.country));
            continue;
        }
        let cap = ProvinceId(e.capital);
        let mut ok = true;
        if !graph.contains(cap) {
            issues.push(DivisionScenarioIssue::SupplyUnknownCapital { country: e.country, province: cap });
            ok = false;
        } else if let Some(c) = c {
            let actual = control.controller(cap);
            if actual != Some(c) {
                issues.push(DivisionScenarioIssue::SupplyCapitalNotControlled { country: e.country, province: cap, actual });
                ok = false;
            }
        }
        if super::legacy_supply::stockpile_units(e.military_supplies).is_none() {
            issues.push(DivisionScenarioIssue::SupplyInvalidStockpile { country: e.country });
            ok = false;
        }
        if let (Some(c), true) = (c, ok) {
            supply.push(SupplyCountryConfig { country: c, capital: cap, stockpile: e.military_supplies });
            supply_labels.insert(e.country, c);
        }
    }
    let mut usage: BTreeMap<u32, f32> = BTreeMap::new();
    let mut seen_usage: BTreeSet<u32> = BTreeSet::new();
    let division_country: BTreeMap<u32, u32> = file.divisions.iter().map(|d| (d.id, d.country)).collect();
    for e in &file.supply.divisions {
        if !seen_usage.insert(e.division) {
            issues.push(DivisionScenarioIssue::SupplyDuplicateUsage(e.division));
            continue;
        }
        let Some(label) = division_country.get(&e.division) else {
            issues.push(DivisionScenarioIssue::SupplyUnknownDivision(e.division));
            continue;
        };
        if !super::legacy_supply::usage_is_valid(e.supply_usage) {
            issues.push(DivisionScenarioIssue::SupplyInvalidUsage { division: e.division });
            continue;
        }
        if !file.supply.countries.iter().any(|c| c.country == *label) {
            issues.push(DivisionScenarioIssue::SupplyUsageWithoutCountry { division: e.division });
            continue;
        }
        usage.insert(e.division, e.supply_usage);
    }
    for d in &file.divisions {
        if file.supply.countries.iter().any(|c| c.country == d.country) && !usage.contains_key(&d.id) && !seen_usage.contains(&d.id) {
            issues.push(DivisionScenarioIssue::SupplyMissingUsage { division: d.id });
        }
    }
    for d in &mut divisions {
        if let Some(u) = usage.get(&d.id.0) {
            d.supply_usage = *u;
        }
    }
    if !issues.is_empty() {
        return Err(issues);
    }
    divisions.sort_by_key(|d| d.id);
    Ok(DivisionScenario { description: file.description.clone(), params: MovementParams { base_days: file.base_days }, countries, control, state_owners: owners, wars, supply, divisions })
}
