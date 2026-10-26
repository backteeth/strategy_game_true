//! P32-007 (M4-A) / P32-008 (M4-B): 新ステート(`MapStateId`)単位の経済シミュレーション状態と、
//! 検証用の月次実行経路。
//!
//! 静的な世界定義(`world::MapStateDef`: 名称・所属プロヴィンス)と、時間経過で変わる
//! シミュレーション状態(この`MapStateEconomy`)は別のデータとして持つ。所属ファイルや
//! ステート境界を経済データの保存先として流用しない。
//!
//! D9の方針に沿い、人口・雇用・経済施設(農場・工場・採掘施設など)の規模は**ステート単位**で
//! 1回だけ保持する(プロヴィンス数ぶん複製しない)。天然資源の所在地(プロヴィンス単位)と
//! 位置が重要な施設(港・要塞・鉄道駅)は未実装で、`resource_deposits`は
//! 「このステートで生産処理が利用できる鉱床(入力データ)」をステート単位に集計済みとして
//! 受け取るだけである。
//!
//! 計算は旧`StateData`と同じ中核関数(`economy::state_access`、`economy::production`、
//! `economy::finance`、`population::growth`)を通る。ここに計算式の複製はない。
//! セーブ形式・正式な世界設定・ゲームバランスとは無関係で、検証用シナリオの固定入力から作る。
//!
//! **国家との対応関係**(M4-B): 財政・人的資源は国家単位の値である。どのステートをどの国家の
//! 経済計算に含めるかは、検証用シナリオ(`MultiMonthScenario`)の`countries[].states`が
//! 明示する「経済計算上の対応関係」であり、法的所有・占領のモデルではない(それらは未決で、
//! この対応関係を正式な領有モデルとして固定しない)。

use crate::building::construction::{
    CompletionOutcome, ConstructionOrder, ConstructionStatus, advance_construction_queue,
};
use crate::building::data::{BuildingRegistry, BuildingType};
use crate::common::CountryId;
use crate::country::ai_core::{
    AiCountry, AiRegions, OrderBlock, next_ai_tax_rate, run_construction_priorities,
};
use crate::country::country_ai::{CountryAiDecisionReason, CountryAiState};
use crate::country::ai_military::{
    MilitaryAiFacts, MilitaryHooks, MilitaryPriority, factory_expansion_blocked_from,
    iron_supply_context_from, military_rebuild_context_for, wartime_supply_context_from,
};
use crate::economy::factory_starvation::FactoryStarvationHistory;
use crate::economy::iron_shortage::IronShortageHistory;
use crate::military::supply_production_history::SupplyProductionHistory;
use crate::economy::economic_state::{
    EconomicState, calculate_economic_state, compute_economic_metrics,
};
use crate::economy::province_resources::StateResourceSetup;
use crate::economy::finance::{compute_country_finance, update_welfare_and_unrest};
use crate::economy::production::{
    CountryProductionOutcome, process_country_production_with_outcome,
};
use crate::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use crate::economy::state_access::{StateEconomyAccess, update_monthly_workforce_and_logistics};
use crate::population::growth::{grow_populations, recover_country_manpower};
use crate::world::{MapStateId, WorldModel};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub mod snapshot;
pub mod static_identity;

pub const ECONOMY_SCENARIO_FORMAT_VERSION: u32 = 1;
pub const MULTI_MONTH_SCENARIO_FORMAT_VERSION: u32 = 1;

/// 新ステートの経済シミュレーション状態(実行時のみ。セーブしない)。
#[derive(Debug, Clone, PartialEq)]
pub struct MapStateEconomy {
    pub state: MapStateId,
    pub population: u64,
    pub workforce_ratio: f32,
    pub employed_workforce: u64,
    pub unemployed_workforce: u64,
    pub base_logistics_capacity: f32,
    pub logistics_capacity: f32,
    pub logistics_usage: f32,
    pub logistics_ratio: f32,
    /// 生活水準(0.0〜100.0)。税収と福祉更新が使う。
    pub living_standard: f32,
    /// 不満度(0.0〜100.0)。
    pub unrest: f32,
    pub buildings: HashMap<BuildingType, u32>,
    pub building_operation_ratios: HashMap<BuildingType, f32>,
    /// このステートで生産処理が利用できる鉱床(ステート単位に集計済みの入力)。
    pub resource_deposits: Vec<StateResourceDeposit>,
}

impl StateEconomyAccess for MapStateEconomy {
    fn economy_key(&self) -> u64 {
        self.state.0 as u64
    }
    fn population(&self) -> u64 {
        self.population
    }
    fn set_population(&mut self, population: u64) {
        self.population = population;
    }
    fn workforce_ratio(&self) -> f32 {
        self.workforce_ratio
    }
    fn buildings(&self) -> &HashMap<BuildingType, u32> {
        &self.buildings
    }
    fn employed_workforce(&self) -> u64 {
        self.employed_workforce
    }
    fn unemployed_workforce(&self) -> u64 {
        self.unemployed_workforce
    }
    fn set_employment(&mut self, employed: u64, unemployed: u64) {
        self.employed_workforce = employed;
        self.unemployed_workforce = unemployed;
    }
    fn base_logistics_capacity(&self) -> f32 {
        self.base_logistics_capacity
    }
    fn logistics_ratio(&self) -> f32 {
        self.logistics_ratio
    }
    fn logistics_capacity(&self) -> f32 {
        self.logistics_capacity
    }
    fn logistics_usage(&self) -> f32 {
        self.logistics_usage
    }
    fn set_logistics(&mut self, capacity: f32, usage: f32, ratio: f32) {
        self.logistics_capacity = capacity;
        self.logistics_usage = usage;
        self.logistics_ratio = ratio;
    }
    fn living_standard(&self) -> f32 {
        self.living_standard
    }
    fn unrest(&self) -> f32 {
        self.unrest
    }
    fn set_welfare(&mut self, living_standard: f32, unrest: f32) {
        self.living_standard = living_standard;
        self.unrest = unrest;
    }
    fn building_operation_ratio(&self, b_type: BuildingType) -> f32 {
        self.building_operation_ratios
            .get(&b_type)
            .copied()
            .unwrap_or(0.0)
    }
    fn operation_ratios(&self) -> &HashMap<BuildingType, f32> {
        &self.building_operation_ratios
    }
    fn resource_deposits(&self) -> &[StateResourceDeposit] {
        &self.resource_deposits
    }
    fn set_building_operation_ratios(&mut self, ratios: HashMap<BuildingType, f32>) {
        self.building_operation_ratios = ratios;
    }
}

// ---------------------------------------------------------------- 検証用シナリオ(入力)

/// 検証用シナリオ内の1ステート分の入力。**正式な人口分布・施設配置ではない。**
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapStateEconomyInput {
    /// 対象の`MapStateId`(旧`StateId`とは無関係)。
    pub state: u32,
    pub population: u64,
    pub workforce_ratio: f32,
    /// 生活水準(0.0〜100.0)。明示が必要(既定値で補わない)。
    pub living_standard: f32,
    /// 不満度(0.0〜100.0)。明示が必要。
    pub unrest: f32,
    #[serde(default = "default_base_logistics")]
    pub base_logistics_capacity: f32,
    #[serde(default)]
    pub buildings: HashMap<BuildingType, u32>,
    #[serde(default)]
    pub resource_deposits: Vec<StateResourceDeposit>,
}

fn default_base_logistics() -> f32 {
    100.0
}

/// 検証用の経済シナリオ(1国家分の固定入力。M4-A)。ゲームバランスや正式な世界設定とは区別する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EconomyScenario {
    pub format_version: u32,
    /// 検証用シナリオであることを示す説明。
    pub description: String,
    /// この国家の動員済み人的資源(既存の`mobilized_manpower`と同じ意味)。
    pub country_mobilized_manpower: u64,
    pub is_bankrupt: bool,
    /// 国家の備蓄(生産処理へ明示的に渡す入力。不足を黙って補わない)。
    pub stockpile: HashMap<ResourceType, f64>,
    pub states: Vec<MapStateEconomyInput>,
}

/// 検証用シナリオ内の1国家分の入力(国家単位の値)。`states`は、この国家の経済計算に含める
/// ステートの`MapStateId`(**経済計算上の対応関係**。法的所有・占領ではない)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioCountryInput {
    /// シナリオ内だけで使うラベル(旧`CountryId`との対応は持たない)。
    pub label: u32,
    pub treasury: f64,
    pub tax_rate: f32,
    pub available_manpower: u64,
    pub mobilized_manpower: u64,
    pub stockpile: HashMap<ResourceType, f64>,
    pub states: Vec<u32>,
    /// trueならAI国家(税率調整と経済施設の建設判断を月次の周期で実行する。評価周期の識別には`label`を使う)。
    /// falseはプレイヤー等で、AIは何も変更しない。明示が必要(既定値で補わない)。
    pub ai_controlled: bool,
    /// 軍事状況の明示的な入力(P32-012。出典は検証用シナリオで、新マップ上の軍隊・戦争の状態ではない)。
    /// `None` = 入力なし(「平和」「脅威ゼロ」ではなく、軍事に関わる優先度2.5・2.9は評価されない)。
    #[serde(default)]
    pub military: Option<MilitaryAiFacts>,
}

/// 複数国家・複数月の検証用シナリオ(M4-B)。**正式な人口・税率・国家所属ではない。**
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiMonthScenario {
    pub format_version: u32,
    pub description: String,
    /// 実行する月数。
    pub months: u32,
    pub countries: Vec<ScenarioCountryInput>,
    pub states: Vec<MapStateEconomyInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EconomyDataIssue {
    UnsupportedVersion {
        found: u32,
    },
    UnknownState {
        state: MapStateId,
    },
    DuplicateState {
        state: MapStateId,
    },
    InvalidValue {
        state: Option<MapStateId>,
        what: String,
    },
    UnknownBuilding {
        state: MapStateId,
        building: BuildingType,
    },
    /// 国家への対応づけの不備(複数国家に属する・どの国家にも属さない・重複ラベルなど)。
    InvalidCountryMapping {
        what: String,
    },
    /// ステートの鉱床が、シナリオ入力(ステート単位)とプロヴィンス別の資源定義の両方にある
    /// (二重入力。プロヴィンス別の資源定義を正本とする場合、ステート単位の入力は空でなければならない)。
    ResourceDoubleSource {
        state: MapStateId,
    },
    /// 鉱床を必要とする施設(Mine)があるのに、所属プロヴィンスの資源が未設定。
    ResourcesNotConfigured {
        state: MapStateId,
        unset_provinces: Vec<u32>,
    },
    /// 導出済みの資源集計が、現在の所属・資源定義に対して古い。
    StaleResources,
}

impl std::fmt::Display for EconomyDataIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion { found } => write!(
                f,
                "economy scenario has unsupported format_version {found} (supported: {ECONOMY_SCENARIO_FORMAT_VERSION})"
            ),
            Self::UnknownState { state } => write!(
                f,
                "economy data refers to unknown {state} (not defined in the world layout)"
            ),
            Self::DuplicateState { state } => {
                write!(f, "economy data for {state} is defined more than once")
            }
            Self::InvalidValue { state, what } => match state {
                Some(s) => write!(f, "invalid value for {s}: {what}"),
                None => write!(f, "invalid value: {what}"),
            },
            Self::UnknownBuilding { state, building } => write!(
                f,
                "{state} uses building type {building:?} that is not defined in the building registry"
            ),
            Self::InvalidCountryMapping { what } => {
                write!(f, "invalid country mapping: {what}")
            }
            Self::ResourceDoubleSource { state } => write!(
                f,
                "{state} has deposits in both the scenario state input and the province resources (single source required)"
            ),
            Self::ResourcesNotConfigured { state, unset_provinces } => write!(
                f,
                "{state} has a Mine but {} province(s) have no resource setup: {:?}",
                unset_provinces.len(),
                unset_provinces.iter().take(10).collect::<Vec<_>>()
            ),
            Self::StaleResources => write!(
                f,
                "derived province resources are stale for the current membership or resource file"
            ),
        }
    }
}

fn header_version(text: &str) -> Result<u32, String> {
    #[derive(Deserialize)]
    struct Header {
        format_version: u32,
    }
    let h: Header = ron::from_str(text).map_err(|e| e.to_string())?;
    Ok(h.format_version)
}

pub fn parse_economy_scenario(text: &str) -> Result<EconomyScenario, String> {
    let v = header_version(text)?;
    if v != ECONOMY_SCENARIO_FORMAT_VERSION {
        return Err(EconomyDataIssue::UnsupportedVersion { found: v }.to_string());
    }
    ron::from_str(text).map_err(|e| e.to_string())
}

pub fn parse_multi_month_scenario(text: &str) -> Result<MultiMonthScenario, String> {
    let v = header_version(text)?;
    if v != MULTI_MONTH_SCENARIO_FORMAT_VERSION {
        return Err(format!(
            "multi-month economy scenario has unsupported format_version {v} (supported: {MULTI_MONTH_SCENARIO_FORMAT_VERSION})"
        ));
    }
    ron::from_str(text).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- 実行時データ

/// `MapStateId`をキーにした、ステート経済データの集合。同じステートは1件だけ。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapStateEconomyRegistry {
    states: BTreeMap<MapStateId, MapStateEconomy>,
    /// `attach_province_resources`で接続した、ステートごとの資源設定の状況(接続前は`None`)。
    /// Mineの注文受付・完成時に、完成後の月次経済に必要な資源入力の整合を確認するために使う。
    resource_setup: Option<BTreeMap<MapStateId, StateResourceSetup>>,
}

fn finite_nonneg(v: f64) -> bool {
    v.is_finite() && v >= 0.0
}

fn is_ambiguous_legacy_scale(v: f32) -> bool {
    v > 0.0 && v <= 1.0
}

fn in_range(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

/// ステート入力の検証と実行時データの作成(単一国家シナリオ・複数国家シナリオで共通)。
fn build_states(
    inputs: &[MapStateEconomyInput],
    world: &WorldModel,
    buildings: &BuildingRegistry,
    issues: &mut Vec<EconomyDataIssue>,
) -> BTreeMap<MapStateId, MapStateEconomy> {
    let mut out: BTreeMap<MapStateId, MapStateEconomy> = BTreeMap::new();
    for inp in inputs {
        let id = MapStateId(inp.state);
        if world.state(id).is_none() {
            issues.push(EconomyDataIssue::UnknownState { state: id });
            continue;
        }
        if out.contains_key(&id) {
            issues.push(EconomyDataIssue::DuplicateState { state: id });
            continue;
        }
        let mut ok = true;
        let mut bad = |what: String| {
            issues.push(EconomyDataIssue::InvalidValue {
                state: Some(id),
                what,
            });
            ok = false;
        };
        if !in_range(inp.workforce_ratio, 0.0, 1.0) {
            bad(format!(
                "workforce_ratio {} must be finite and within [0.0, 1.0]",
                inp.workforce_ratio
            ));
        }
        // 旧経路の読み込み時正規化(`StateData::sanitize_values`)は物流容量0を100へ置き換えるため、
        // 0は「未指定」と区別できない。新経路では黙って置き換えず、正の値の明示を要求する。
        if !(inp.base_logistics_capacity.is_finite() && inp.base_logistics_capacity > 0.0) {
            bad(format!(
                "base_logistics_capacity {} must be finite and > 0 (0 is treated as unspecified by the old loader)",
                inp.base_logistics_capacity
            ));
        }
        // 旧経路の読み込み時正規化は0 < 値 <= 1.0を旧0〜1スケールとして100倍するため、その範囲は曖昧
        // として拒否する(値は0、または1より大きい0〜100のスケールで明示する)。
        if !in_range(inp.living_standard, 0.0, 100.0) || is_ambiguous_legacy_scale(inp.living_standard) {
            bad(format!(
                "living_standard {} must be finite, within [0.0, 100.0], and not in (0.0, 1.0] (ambiguous legacy scale)",
                inp.living_standard
            ));
        }
        if !in_range(inp.unrest, 0.0, 100.0) || is_ambiguous_legacy_scale(inp.unrest) {
            bad(format!(
                "unrest {} must be finite, within [0.0, 100.0], and not in (0.0, 1.0] (ambiguous legacy scale)",
                inp.unrest
            ));
        }
        for d in &inp.resource_deposits {
            if !finite_nonneg(d.base_output) {
                bad(format!(
                    "deposit {:?} base_output {} must be finite and >= 0",
                    d.resource_type, d.base_output
                ));
            }
        }
        for b in inp.buildings.keys() {
            if buildings.get(*b).is_none() {
                issues.push(EconomyDataIssue::UnknownBuilding {
                    state: id,
                    building: *b,
                });
                ok = false;
            }
        }
        if !ok {
            continue;
        }
        out.insert(
            id,
            MapStateEconomy {
                state: id,
                population: inp.population,
                workforce_ratio: inp.workforce_ratio,
                employed_workforce: 0,
                unemployed_workforce: 0,
                base_logistics_capacity: inp.base_logistics_capacity,
                logistics_capacity: inp.base_logistics_capacity,
                logistics_usage: 0.0,
                logistics_ratio: 1.0,
                living_standard: inp.living_standard,
                unrest: inp.unrest,
                buildings: inp.buildings.clone(),
                building_operation_ratios: HashMap::new(),
                resource_deposits: inp.resource_deposits.clone(),
            },
        );
    }
    out
}

fn check_stockpile(
    stockpile: &HashMap<ResourceType, f64>,
    issues: &mut Vec<EconomyDataIssue>,
) {
    for (res, amount) in stockpile {
        if !finite_nonneg(*amount) {
            issues.push(EconomyDataIssue::InvalidValue {
                state: None,
                what: format!("stockpile {res:?} = {amount} (must be finite and >= 0)"),
            });
        }
    }
}

impl MapStateEconomyRegistry {
    /// 検証用シナリオ(単一国家)から作る。ワールド定義にないステート・重複・不正値・未定義の
    /// 施設は一括でエラーとして返し、部分的なデータを採用しない。雇用・物流の導出値は
    /// 初期化し、月次計算(`run_month`)が書き込む。
    pub fn from_scenario(
        scenario: &EconomyScenario,
        world: &WorldModel,
        buildings: &BuildingRegistry,
    ) -> Result<Self, Vec<EconomyDataIssue>> {
        let mut issues = Vec::new();
        if scenario.format_version != ECONOMY_SCENARIO_FORMAT_VERSION {
            issues.push(EconomyDataIssue::UnsupportedVersion {
                found: scenario.format_version,
            });
        }
        check_stockpile(&scenario.stockpile, &mut issues);
        let out = build_states(&scenario.states, world, buildings, &mut issues);
        if issues.is_empty() {
            Ok(Self {
                states: out,
                resource_setup: None,
            })
        } else {
            Err(issues)
        }
    }

    /// 複数国家・複数月シナリオから、ステート経済データと国家ごとの実行時データを作る。
    /// 各ステートは**ちょうど1つの国家**の経済計算に含まれなければならない(二重集計・未集計を
    /// 防ぐ)。この対応関係は検証用であり、正式な領有・占領モデルではない。
    pub fn from_multi_scenario(
        scenario: &MultiMonthScenario,
        world: &WorldModel,
        buildings: &BuildingRegistry,
    ) -> Result<(Self, Vec<ScenarioCountry>), Vec<EconomyDataIssue>> {
        let mut issues = Vec::new();
        if scenario.format_version != MULTI_MONTH_SCENARIO_FORMAT_VERSION {
            issues.push(EconomyDataIssue::UnsupportedVersion {
                found: scenario.format_version,
            });
        }
        let out = build_states(&scenario.states, world, buildings, &mut issues);

        let mut labels = BTreeSet::new();
        let mut owner_of: BTreeMap<MapStateId, u32> = BTreeMap::new();
        let mut countries = Vec::new();
        for c in &scenario.countries {
            if !labels.insert(c.label) {
                issues.push(EconomyDataIssue::InvalidCountryMapping {
                    what: format!("country label {} is defined more than once", c.label),
                });
                continue;
            }
            check_stockpile(&c.stockpile, &mut issues);
            if !c.treasury.is_finite() {
                issues.push(EconomyDataIssue::InvalidValue {
                    state: None,
                    what: format!("country {} treasury {} must be finite", c.label, c.treasury),
                });
            }
            if !in_range(c.tax_rate, 0.0, 1.0) {
                issues.push(EconomyDataIssue::InvalidValue {
                    state: None,
                    what: format!(
                        "country {} tax_rate {} must be finite and within [0.0, 1.0]",
                        c.label, c.tax_rate
                    ),
                });
            }
            let mut member_ids = BTreeSet::new();
            for s in &c.states {
                let id = MapStateId(*s);
                if !out.contains_key(&id) {
                    issues.push(EconomyDataIssue::InvalidCountryMapping {
                        what: format!(
                            "country {} refers to {id}, which has no economy data",
                            c.label
                        ),
                    });
                    continue;
                }
                if let Some(prev) = owner_of.insert(id, c.label) {
                    issues.push(EconomyDataIssue::InvalidCountryMapping {
                        what: format!(
                            "{id} is mapped to both country {prev} and country {}",
                            c.label
                        ),
                    });
                }
                member_ids.insert(id);
            }
            let mut stockpile = CountryStockpile::new();
            for (r, a) in &c.stockpile {
                stockpile.set(*r, *a);
            }
            countries.push(ScenarioCountry {
                label: c.label,
                treasury: c.treasury,
                tax_rate: c.tax_rate,
                available_manpower: c.available_manpower,
                mobilized_manpower: c.mobilized_manpower,
                stockpile,
                states: member_ids.into_iter().collect(),
                monthly_income: 0.0,
                monthly_expenses: 0.0,
                monthly_balance: 0.0,
                construction_queue: Vec::new(),
                next_order_seq: 1,
                order_log: Vec::new(),
                economic_state: EconomicState::default(),
                ai: c.ai_controlled.then(ScenarioAi::new),
                military: c.military,
            });
        }
        for id in out.keys() {
            if !owner_of.contains_key(id) {
                issues.push(EconomyDataIssue::InvalidCountryMapping {
                    what: format!("{id} is not mapped to any country (it would never be calculated)"),
                });
            }
        }
        if issues.is_empty() {
            Ok((
                Self {
                    states: out,
                    resource_setup: None,
                },
                countries,
            ))
        } else {
            Err(issues)
        }
    }

    pub fn get(&self, id: MapStateId) -> Option<&MapStateEconomy> {
        self.states.get(&id)
    }
    /// `MapStateId`昇順のステート経済データ。
    pub fn iter(&self) -> impl Iterator<Item = &MapStateEconomy> {
        self.states.values()
    }
    pub fn len(&self) -> usize {
        self.states.len()
    }
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// プロヴィンス別の資源定義(の所属ステートへの集計)を、生産処理が使う各ステートの鉱床入力へ
    /// 接続する(P32-009)。**プロヴィンス別の資源定義が鉱床入力の唯一の正本**になるため、
    /// シナリオのステート単位の`resource_deposits`が空でないステートはエラー(二重入力・二重計上の防止)。
    /// 鉱床を必要とする施設(Mine)があるステートは、所属プロヴィンスの資源がすべて設定済み
    /// (資源なしの明示を含む)でなければエラー(未設定を資源ゼロへ黙って変換しない)。
    /// 古い集計(所属・資源定義が変わった後のもの)もエラー。エラー時はレジストリを一切変更しない。
    /// 地理上の資源値は国家備蓄へ直接加算しない(産出は生産処理が備蓄へ加える)。
    pub fn attach_province_resources(
        &mut self,
        derived: &crate::economy::province_resources::StateResourcesDerived,
        world: &WorldModel,
        file: &crate::economy::province_resources::ProvinceResourcesFile,
    ) -> Result<(), Vec<EconomyDataIssue>> {
        use crate::economy::province_resources::StateResourceSetup;
        let mut issues = Vec::new();
        if !derived.is_current_for(world, file) {
            return Err(vec![EconomyDataIssue::StaleResources]);
        }
        let mut updates: Vec<(MapStateId, Vec<StateResourceDeposit>)> = Vec::new();
        for (id, s) in &self.states {
            if !s.resource_deposits.is_empty() {
                issues.push(EconomyDataIssue::ResourceDoubleSource { state: *id });
                continue;
            }
            let Some(summary) = derived.state(*id) else {
                issues.push(EconomyDataIssue::UnknownState { state: *id });
                continue;
            };
            let needs = s.building_level(BuildingType::Mine) > 0;
            if needs && summary.setup != StateResourceSetup::Complete {
                issues.push(EconomyDataIssue::ResourcesNotConfigured {
                    state: *id,
                    unset_provinces: if summary.setup == StateResourceSetup::Unconfigured {
                        world
                            .provinces_of(*id)
                            .map(|p| p.iter().map(|x| x.0).collect())
                            .unwrap_or_default()
                    } else {
                        summary.unset_provinces.iter().map(|p| p.0).collect()
                    },
                });
                continue;
            }
            updates.push((*id, summary.deposits.clone()));
        }
        if !issues.is_empty() {
            return Err(issues);
        }
        for (id, deposits) in updates {
            if let Some(s) = self.states.get_mut(&id) {
                s.resource_deposits = deposits;
            }
        }
        // 完成後の月次経済に必要な資源入力の整合(Mine)を、注文受付・完成時に確認するための記録。
        self.resource_setup = Some(
            self.states
                .keys()
                .filter_map(|id| derived.state(*id).map(|s| (*id, s.setup)))
                .collect(),
        );
        Ok(())
    }

    /// 1か月分の人口・雇用・物流・生産(旧経路と同じ中核計算)を実行する(M4-A)。
    /// 国家の備蓄・動員・破産状態は呼び出し側が明示的に渡す(不足を補わない)。
    pub fn run_month(
        &mut self,
        country_mobilized_manpower: u64,
        stockpile: &mut CountryStockpile,
        building_registry: &BuildingRegistry,
        is_bankrupt: bool,
    ) -> CountryProductionOutcome {
        let mut refs: Vec<&mut MapStateEconomy> = self.states.values_mut().collect();
        update_monthly_workforce_and_logistics(
            &mut refs,
            country_mobilized_manpower,
            building_registry,
        );
        process_country_production_with_outcome(
            &mut refs,
            stockpile,
            building_registry,
            is_bankrupt,
        )
    }

    /// 1か月分の経済処理(M4-B): 旧`handle_monthly_population_growth` →
    /// `handle_monthly_economy`と同じ順序で、各国家について
    /// 人口成長 → 人的資源の回復 → (動員按分・雇用・物流) → 生産 → 財政 → 福祉・不満度、
    /// を実行する。各ステートは対応する国家の処理で**ちょうど1回**更新され、国家の値
    /// (国庫・備蓄・人的資源)は国家ごとに1回だけ更新される。
    ///
    /// 含まない処理(旧`handle_monthly_economy`にはあるが今回の対象外): 経済状態の分類
    /// (`economic_state`)、通知、建設キュー、AI履歴、補給実績の記録。
    pub fn run_country_month(
        &mut self,
        country: &mut ScenarioCountry,
        building_registry: &BuildingRegistry,
    ) -> CountryMonthReport {
        let mut refs: Vec<&mut MapStateEconomy> = self
            .states
            .iter_mut()
            .filter(|(id, _)| country.states.contains(id))
            .map(|(_, s)| s)
            .collect();

        // 1. 人口成長(ステートごとに1回) → 所有人口の集計 → 人的資源の回復(国家ごとに1回)
        grow_populations(&mut refs);
        let owned_population: u64 = refs.iter().map(|s| s.population).sum();
        country.available_manpower =
            recover_country_manpower(country.available_manpower, owned_population);

        // 2. 動員按分・雇用・物流(破産判定は月初の国庫で行う。旧経路と同じ)
        let is_bankrupt = country.treasury < 0.0;
        update_monthly_workforce_and_logistics(
            &mut refs,
            country.mobilized_manpower,
            building_registry,
        );

        // 3. 生産
        let production = process_country_production_with_outcome(
            &mut refs,
            &mut country.stockpile,
            building_registry,
            is_bankrupt,
        );

        // 4. 財政(ステートごとの計算 → 国家への集計 → 国庫への反映を1回)
        let finance = compute_country_finance(country.tax_rate, &refs, building_registry);
        country.monthly_income = finance.income;
        country.monthly_expenses = finance.expenses;
        country.monthly_balance = finance.balance;
        country.treasury += finance.balance;

        // 5. 福祉・不満度(各ステート1回。食料の有無は生産後の備蓄で判定。旧経路と同じ)
        let has_food = country.stockpile.get(ResourceType::Food) > 0.0;
        for s in refs.iter_mut() {
            update_welfare_and_unrest(&mut **s, country.tax_rate, has_food, is_bankrupt);
        }

        // 6. 経済状態の分類(旧`handle_monthly_economy`の最後と同じ。福祉の更新後の指標で判定)
        let metrics = compute_economic_metrics(
            &refs,
            country.monthly_balance,
            country.stockpile.get(ResourceType::Food),
        );
        country.economic_state = calculate_economic_state(&metrics);

        CountryMonthReport {
            label: country.label,
            is_bankrupt,
            owned_population,
            finance,
            science: production.science,
            magic: production.magic,
            production,
        }
    }
}

/// 検証用シナリオの国家(国家単位の実行時データ)。法的所有・占領モデルではなく、経済計算上の
/// 対応関係(`states`)だけを持つ。
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioCountry {
    pub label: u32,
    pub treasury: f64,
    pub tax_rate: f32,
    pub available_manpower: u64,
    pub mobilized_manpower: u64,
    pub stockpile: CountryStockpile,
    /// この国家の経済計算に含めるステート(`MapStateId`昇順、重複なし)。
    pub states: Vec<MapStateId>,
    pub monthly_income: f64,
    pub monthly_expenses: f64,
    pub monthly_balance: f64,
    /// ステートを対象にした建設キュー(国家ごとに1つ。ステートへ複製しない)。
    pub construction_queue: Vec<MapConstructionOrder>,
    /// 次に発行する注文識別子の連番(国家ごと。実行時のみ。永続化しない)。
    pub next_order_seq: u64,
    /// 完了・取消しで建設キューを離れた注文の記録(実行時のみ。二度目のキャンセルや完成済みの注文の
    /// キャンセルを、「存在しない注文」と区別して拒否するために使う)。
    pub order_log: Vec<(ConstructionOrderId, OrderEnd)>,
    /// 月次経済の最後に判定される経済状態(`economy::economic_state`)。
    pub economic_state: EconomicState,
    /// AI国家の評価状態(AI国家でなければ`None`)。
    pub ai: Option<ScenarioAi>,
    /// 軍事状況の入力(`None` = 入力なし。暗黙の既定値へ置き換えない)。
    pub military: Option<MilitaryAiFacts>,
}

/// `run_country_month`が返す、その月の国家単位の結果。
#[derive(Debug, Clone, PartialEq)]
pub struct CountryMonthReport {
    pub label: u32,
    pub is_bankrupt: bool,
    pub owned_population: u64,
    pub finance: crate::economy::finance::CountryFinanceResult,
    pub science: f64,
    pub magic: f64,
    /// 生産処理の観測値(軍需生産・Iron未充足・Factoryの飢餓。AIの履歴の元データ)。
    pub production: CountryProductionOutcome,
}

/// シナリオの備蓄(`ResourceType`→量)から国家備蓄を作る。
pub fn stockpile_from_scenario(scenario: &EconomyScenario) -> CountryStockpile {
    let mut s = CountryStockpile::new();
    for (res, amount) in &scenario.stockpile {
        s.set(*res, *amount);
    }
    s
}

// ================================================================ M4-C: 建設キュー・経済AI

/// 新ステートを対象にした建設注文。旧`ConstructionQueueItem`(旧`StateId`が対象)とは別の型で、
/// 対象に`MapStateId`しか渡せない(旧`StateId`との相互変換は無い)。費用・期間・段階の意味は旧と同じ。
#[derive(Debug, Clone, PartialEq)]
pub struct MapConstructionOrder {
    /// 注文の識別子(発行した国家+連番。キュー上の位置が変わっても変わらない)。
    pub id: ConstructionOrderId,
    pub state: MapStateId,
    pub building_type: BuildingType,
    pub target_level: u32,
    pub progress: f64,
    pub required_progress: f64,
    pub paid_cost: f64,
    pub status: ConstructionStatus,
}

/// 建設注文の識別子。発行した国家のラベルと、国家ごとの連番。**配列上の位置ではない**ので、他の注文が
/// 完成・取消しされてキューの位置がずれても変わらない。実行時だけの識別子で、セーブには保存しない。
/// 旧`ConstructionQueueItem`(識別子を持たない)とは別の型で、`ProvinceId`・`MapStateId`・国家IDとも
/// 数値で互換にならない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConstructionOrderId {
    pub country: u32,
    pub seq: u64,
}

impl std::fmt::Display for ConstructionOrderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Order#{}-{}", self.country, self.seq)
    }
}

/// 建設キューを離れた注文の終わり方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderEnd {
    /// 完成し、施設へ反映された。
    Completed,
    /// 取り消され、返金された。
    Cancelled,
}

/// キャンセルの対象。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelTarget {
    /// 識別子で指定した注文。
    Order(ConstructionOrderId),
    /// その国家の最後に登録された注文(旧UIの「C」キーの動作)。
    Latest,
}

/// キャンセルの結果(返金の内訳)。
#[derive(Debug, Clone, PartialEq)]
pub struct CancelReceipt {
    pub id: ConstructionOrderId,
    pub state: MapStateId,
    pub building_type: BuildingType,
    pub progress: f64,
    pub required_progress: f64,
    pub paid_cost: f64,
    /// 返金額(`building::construction::cancel_refund` = 支払済み費用 × `REFUND_RATIO`)。
    pub refund: f64,
}

/// キャンセルの拒否理由。拒否時は国庫・キュー・施設・記録を一切変更しない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelError {
    UnknownCountry(u32),
    /// 注文主体の建設キューが空(`Latest`)。
    EmptyQueue,
    /// 別の国家が発行した注文(キャンセルできない。その国家の国庫へも触れない)。
    NotIssuersOrder(ConstructionOrderId),
    /// 完成済み(未完成として返金しない)。
    AlreadyCompleted(ConstructionOrderId),
    /// 取消し済み(二重に返金しない)。
    AlreadyCancelled(ConstructionOrderId),
    /// この国家が発行した記録のない識別子。
    NoSuchOrder(ConstructionOrderId),
}

impl ConstructionOrder for MapConstructionOrder {
    fn advance_one_day(&mut self) -> bool {
        self.status = ConstructionStatus::InProgress;
        self.progress += 1.0;
        self.progress >= self.required_progress
    }
}

/// AI国家の評価状態(`CountryAiState`のうち、検証用国家が保持する分)。
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioAi {
    /// 初回は評価周期に関わらず評価する(旧`CountryAiState::new`と同じ)。
    pub dirty: bool,
    pub last_decision_reason: CountryAiDecisionReason,
    pub last_monthly_evaluation_day: u32,
    /// 判断に使う実行時の履歴(国家ごと。永続化しない: ロードで空、New Gameで空、寿命はこの実行の間)。
    pub history: AiHistories,
}

/// 国家1つ分の、AIの履歴(旧`CountryAiRegistry`の3つの履歴と同じ型・同じ更新の規則)。国家ごとに
/// 独立しており、他国家の履歴とは混ざらない。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AiHistories {
    /// 軍需物資の月次産出の履歴(優先度2.9)。
    pub supply: SupplyProductionHistory,
    /// MilitaryFactoryのIron未充足の履歴(優先度2.8)。
    pub iron: IronShortageHistory,
    /// Factoryの飢餓の履歴(増設の抑止)。
    pub factory: FactoryStarvationHistory,
}

impl ScenarioAi {
    pub fn new() -> Self {
        Self {
            dirty: true,
            last_decision_reason: CountryAiDecisionReason::default(),
            last_monthly_evaluation_day: 0,
            history: AiHistories::default(),
        }
    }
}

impl Default for ScenarioAi {
    fn default() -> Self {
        Self::new()
    }
}

/// 施設がステート単位の経済施設か(D9)。現在の`BuildingType`はすべて**ステート単位の経済施設**で、
/// 港・要塞・鉄道駅のような位置施設は`BuildingType`に存在しない(プロヴィンス単位の実装は後続)。
/// この関数の網羅的な`match`が、新しい種別を追加する際に置き場所の判断を強制する。
/// 注: `Railway`は現行ではステート単位の物流容量の加算(`railway_capacity_bonus`)として扱われる
/// 経済施設であり、鉄道駅(位置施設)ではない。
pub fn is_state_level_economic_facility(b: BuildingType) -> bool {
    match b {
        BuildingType::Farm
        | BuildingType::LoggingCamp
        | BuildingType::Mine
        | BuildingType::Factory
        | BuildingType::MilitaryFactory
        | BuildingType::Railway
        | BuildingType::University
        | BuildingType::MagicAcademy
        | BuildingType::CrystalMine
        | BuildingType::CrystalRefinery => true,
    }
}

/// 建設注文の受付エラー。エラー時は国庫・キュー・施設を一切変更しない。
#[derive(Debug, Clone, PartialEq)]
pub enum OrderError {
    /// 注文主体のラベルが検証用シナリオに無い。
    UnknownCountry(u32),
    UnknownState(MapStateId),
    /// このステートは、その国家の経済計算に含まれていない(別国家のステートへは注文できない)。
    NotMappedToCountry(MapStateId),
    UndefinedBuilding(BuildingType),
    /// ステート単位の経済施設ではない(位置施設など)。
    UnsupportedFacility(BuildingType),
    /// 発見済みのクリスタル鉱床が無いステートへのクリスタル採掘施設(既存のプレイヤー建設の規則と同じ)。
    NoCrystalDeposit(MapStateId),
    /// クリスタル専用ステートへの汎用Mine(既存のプレイヤー建設の規則と同じ)。
    CrystalOnlyState(MapStateId),
    MaxLevel {
        level: u32,
        max: u32,
    },
    AlreadyQueued,
    InsufficientFunds {
        cost: f64,
        treasury: f64,
    },
    /// 完成後の月次経済に必要な前提(資源入力など)が満たされない。
    Precondition(OrderBlock),
}

/// 1日の処理で起きた出来事(検証・表示用)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DayReport {
    /// (国家ラベル, ステート, 施設, 完成後のレベル)
    pub completed: Vec<(u32, MapStateId, BuildingType, u32)>,
    /// 完成条件を満たしたが、前提(資源入力など)が不足して反映を保留した注文。
    pub blocked: Vec<(u32, MapStateId, BuildingType, OrderBlock)>,
    /// AIが出した注文 (国家ラベル, ステート, 施設, 支払った費用)
    pub ordered: Vec<(u32, MapStateId, BuildingType, f64)>,
    /// AIの税率変更 (国家ラベル, 旧, 新)
    pub tax_changes: Vec<(u32, f32, f32)>,
    /// この日に月次経済を実行したか。
    pub monthly_ran: bool,
    /// この日に評価したAI国家ごとの入力・文脈・判断(検証・表示用)。
    pub evaluations: Vec<AiEvaluation>,
}

/// AI国家1回分の評価の記録(軍事状況の入力、履歴から作られた文脈、判断の結果)。
#[derive(Debug, Clone, PartialEq)]
pub struct AiEvaluation {
    pub label: u32,
    /// 軍事状況の入力。`None` = 入力なし(優先度2.5・2.9は評価されない。平和とは扱わない)。
    pub military_input: Option<MilitaryAiFacts>,
    /// 戦時MilitaryFactory(2.5)の文脈が作られたか。
    pub wartime_context: bool,
    /// Iron供給(2.8)の文脈(履歴の月平均の未充足量)。作られなければ`None`。
    pub iron_shortage_average: Option<f64>,
    /// 軍需基盤の再建(2.9)の文脈が作られたか。
    pub rebuild_context: bool,
    /// 履歴によりFactoryの増設が抑止されたか。
    pub factory_blocked: bool,
    /// 着工した軍事に関わる優先度(なければ`None`)。
    pub fired: Option<MilitaryPriority>,
    pub decision_reason: CountryAiDecisionReason,
}

/// 簡易暦の日付(`app::time::GameDate`と同じ月の日数)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SimDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl SimDate {
    /// AI評価の周期判定に使う通算日(旧`handle_daily_country_ai`と同じ式)。
    pub fn day_number(&self) -> u32 {
        (self.year * 365 + self.month as i32 * 30 + self.day as i32) as u32
    }
    /// 翌日と、月が変わったか。
    pub fn next(self) -> (SimDate, bool) {
        let max_day = crate::app::time::GameDate::days_in_month(self.month).unwrap_or(30);
        let mut d = self;
        d.day += 1;
        if d.day > max_day {
            d.day = 1;
            d.month += 1;
            if d.month > 12 {
                d.month = 1;
                d.year += 1;
            }
            return (d, true);
        }
        (d, false)
    }
}

impl ScenarioCountry {
    /// 次の注文識別子を発行する(国家ごとの連番。再利用しない)。
    fn allocate_order_id(&mut self) -> ConstructionOrderId {
        let id = ConstructionOrderId {
            country: self.label,
            seq: self.next_order_seq,
        };
        self.next_order_seq += 1;
        id
    }
}

impl AiCountry for ScenarioCountry {
    type Key = MapStateId;
    fn treasury(&self) -> f64 {
        self.treasury
    }
    fn set_treasury(&mut self, value: f64) {
        self.treasury = value;
    }
    fn monthly_expenses(&self) -> f64 {
        self.monthly_expenses
    }
    fn stock(&self, resource: ResourceType) -> f64 {
        self.stockpile.get(resource)
    }
    fn queue_len(&self) -> usize {
        self.construction_queue.len()
    }
    fn queue_has(&self, region: MapStateId, building: BuildingType) -> bool {
        self.construction_queue
            .iter()
            .any(|o| o.state == region && o.building_type == building)
    }
    fn queue_has_building(&self, building: BuildingType) -> bool {
        self.construction_queue
            .iter()
            .any(|o| o.building_type == building)
    }
    fn queued_regions(&self, building: BuildingType) -> Vec<MapStateId> {
        self.construction_queue
            .iter()
            .filter(|o| o.building_type == building)
            .map(|o| o.state)
            .collect()
    }
    fn push_order(
        &mut self,
        region: MapStateId,
        building: BuildingType,
        target_level: u32,
        required_progress: f64,
        paid_cost: f64,
    ) {
        let id = self.allocate_order_id();
        self.construction_queue.push(MapConstructionOrder {
            id,
            state: region,
            building_type: building,
            target_level,
            progress: 0.0,
            required_progress,
            paid_cost,
            status: ConstructionStatus::InQueue,
        });
    }
}

/// 経済AIが参照する、新ステート側の読み取り窓口。
pub struct MapAiRegions<'a> {
    states: &'a BTreeMap<MapStateId, MapStateEconomy>,
    resource_setup: Option<&'a BTreeMap<MapStateId, StateResourceSetup>>,
}

impl AiRegions<MapStateId> for MapAiRegions<'_> {
    fn exists(&self, region: MapStateId) -> bool {
        self.states.contains_key(&region)
    }
    fn building_level(&self, region: MapStateId, building: BuildingType) -> u32 {
        self.states
            .get(&region)
            .map(|s| s.building_level(building))
            .unwrap_or(0)
    }
    fn buildings(&self, region: MapStateId) -> Option<&HashMap<BuildingType, u32>> {
        self.states.get(&region).map(|s| &s.buildings)
    }
    fn deposits(&self, region: MapStateId) -> &[StateResourceDeposit] {
        self.states
            .get(&region)
            .map(|s| s.resource_deposits.as_slice())
            .unwrap_or(&[])
    }
    fn logistics_ratio(&self, region: MapStateId) -> f32 {
        self.states
            .get(&region)
            .map(|s| s.logistics_ratio)
            .unwrap_or(1.0)
    }
    fn state(&self, region: MapStateId) -> Option<&dyn StateEconomyAccess> {
        self.states.get(&region).map(|s| s as &dyn StateEconomyAccess)
    }
    /// Mineは、プロヴィンス別の資源定義を接続している場合、そのステートの全プロヴィンスの資源が
    /// 設定済み(資源なしの明示を含む)であることを要求する(完成後の月次経済が入力不備で失敗する
    /// 経路を、受付・候補評価の時点で塞ぐ)。未設定と資源なしを区別する。
    fn order_precondition(
        &self,
        region: MapStateId,
        building: BuildingType,
    ) -> Result<(), OrderBlock> {
        if building != BuildingType::Mine {
            return Ok(());
        }
        match self.resource_setup {
            None => Ok(()),
            Some(map) => match map.get(&region) {
                Some(StateResourceSetup::Complete) => Ok(()),
                _ => Err(OrderBlock::ResourcesNotConfigured),
            },
        }
    }
}

impl MapStateEconomyRegistry {
    /// 月次生産の観測値を、AI国家の履歴へ記録する(旧`handle_monthly_economy`が`CountryAiRegistry`の
    /// 3つの履歴へ記録するのと同じ値・同じ規則。記録は月次生産の直後=その月の判断より前)。
    /// 経済計算の対象ステートが無い国家は、履歴を空にする(旧経路が生産を行わない国の履歴を消すのと同じ)。
    /// AI国家でなければ何もしない。履歴は国家ごとに独立で、永続化しない。
    fn record_ai_history(
        &self,
        country: &mut ScenarioCountry,
        month: &CountryMonthReport,
        now: (i32, u8),
    ) {
        let ids = &country.states;
        let Some(ai) = country.ai.as_mut() else {
            return;
        };
        let states: Vec<&MapStateEconomy> = ids.iter().filter_map(|id| self.states.get(id)).collect();
        if states.is_empty() {
            ai.history = AiHistories::default();
            return;
        }
        let mine_levels: u32 = states.iter().map(|s| s.building_level(BuildingType::Mine)).sum();
        let p = &month.production;
        let units = p
            .military_supplies_produced
            .and_then(|v| crate::economy::resources::military_supplies_value_to_units(v).ok());
        ai.history.supply.record(now.0, now.1, units);
        ai.history
            .iron
            .record(now.0, now.1, Some(p.iron_unmet_military), mine_levels);
        let entries: Vec<(usize, u32)> = states
            .iter()
            .map(|s| (s.economy_key() as usize, s.building_level(BuildingType::Factory)))
            .filter(|(_, l)| *l > 0)
            .collect();
        let starved = p.factory_entries > 0 && p.factory_iron_starved_entries == p.factory_entries;
        ai.history.factory.record(
            now.0,
            now.1,
            crate::economy::factory_starvation::factory_signature(&entries),
            starved,
        );
    }

    fn ai_regions(&self) -> MapAiRegions<'_> {
        MapAiRegions {
            states: &self.states,
            resource_setup: self.resource_setup.as_ref(),
        }
    }

    /// 手動の建設注文を受け付ける(AIの候補選択を経由しない。注文主体は`country`)。旧`ui::state_panel`の
    /// 受付と**同じ判定**(`building::construction::check_manual_build_order`)を、対象の存在・注文主体との
    /// 対応・ステート単位の経済施設であることの確認の後に通し、さらに完成後の月次経済に必要な前提
    /// (Mineの資源設定)も確認する。**支払いとキューへの追加は、全部通った後に1回だけ**。エラー時は何も変更しない。
    /// 戻り値は発行した注文の識別子。
    pub fn place_order(
        &self,
        country: &mut ScenarioCountry,
        state: MapStateId,
        building: BuildingType,
        building_registry: &BuildingRegistry,
    ) -> Result<ConstructionOrderId, OrderError> {
        use crate::building::construction::{ManualBuildRejection, check_manual_build_order};
        let Some(s) = self.states.get(&state) else {
            return Err(OrderError::UnknownState(state));
        };
        if !country.states.contains(&state) {
            return Err(OrderError::NotMappedToCountry(state));
        }
        if !is_state_level_economic_facility(building) {
            return Err(OrderError::UnsupportedFacility(building));
        }
        let level = s.building_level(building);
        let def = check_manual_build_order(
            building_registry.get(building),
            building,
            &s.resource_deposits,
            level,
            country.queue_has(state, building),
            country.treasury,
        )
        .map_err(|r| match r {
            ManualBuildRejection::UndefinedBuilding => OrderError::UndefinedBuilding(building),
            ManualBuildRejection::NoCrystalDeposit => OrderError::NoCrystalDeposit(state),
            ManualBuildRejection::CrystalOnlyState => OrderError::CrystalOnlyState(state),
            ManualBuildRejection::MaxLevel { level, max } => OrderError::MaxLevel { level, max },
            ManualBuildRejection::AlreadyQueued => OrderError::AlreadyQueued,
            ManualBuildRejection::InsufficientFunds { cost, treasury } => {
                OrderError::InsufficientFunds { cost, treasury }
            }
        })?;
        if let Err(b) = self.ai_regions().order_precondition(state, building) {
            return Err(OrderError::Precondition(b));
        }
        country.treasury -= def.construction_cost;
        country.push_order(
            state,
            building,
            level + 1,
            def.required_progress,
            def.construction_cost,
        );
        Ok(country
            .construction_queue
            .last()
            .map(|o| o.id)
            .expect("order was just pushed"))
    }

    /// 注文主体のラベルで指定する手動の建設注文(要求: 注文主体・対象`MapStateId`・施設)。
    pub fn request_build(
        &self,
        countries: &mut [ScenarioCountry],
        issuer: u32,
        state: MapStateId,
        building: BuildingType,
        building_registry: &BuildingRegistry,
    ) -> Result<ConstructionOrderId, OrderError> {
        let Some(country) = countries.iter_mut().find(|c| c.label == issuer) else {
            return Err(OrderError::UnknownCountry(issuer));
        };
        self.place_order(country, state, building, building_registry)
    }

    /// 建設注文のキャンセルと返金。返金は**注文主体の国庫だけ**へ、旧UIと同じ規則(支払済み費用 ×
    /// `REFUND_RATIO`、進捗・状態に依存せず丸めない)で加える。完成済み・取消し済み・他国家の注文・
    /// 存在しない注文は拒否し、何も変更しない。資源設定の変更で完成を保留されているMineも、同じ規則で
    /// キャンセルできる(保留時の新しいペナルティや返金率は無い)。
    pub fn cancel_order(
        &self,
        countries: &mut [ScenarioCountry],
        issuer: u32,
        target: CancelTarget,
    ) -> Result<CancelReceipt, CancelError> {
        let Some(country) = countries.iter_mut().find(|c| c.label == issuer) else {
            return Err(CancelError::UnknownCountry(issuer));
        };
        let position = match target {
            CancelTarget::Latest => {
                if country.construction_queue.is_empty() {
                    return Err(CancelError::EmptyQueue);
                }
                country.construction_queue.len() - 1
            }
            CancelTarget::Order(id) => {
                if id.country != issuer {
                    return Err(CancelError::NotIssuersOrder(id));
                }
                match country.construction_queue.iter().position(|o| o.id == id) {
                    Some(p) => p,
                    None => {
                        return Err(match country.order_log.iter().find(|(x, _)| *x == id) {
                            Some((_, OrderEnd::Completed)) => CancelError::AlreadyCompleted(id),
                            Some((_, OrderEnd::Cancelled)) => CancelError::AlreadyCancelled(id),
                            None => CancelError::NoSuchOrder(id),
                        });
                    }
                }
            }
        };
        let order = country.construction_queue.remove(position);
        let refund = crate::building::construction::cancel_refund(order.paid_cost);
        country.treasury += refund;
        country.order_log.push((order.id, OrderEnd::Cancelled));
        Ok(CancelReceipt {
            id: order.id,
            state: order.state,
            building_type: order.building_type,
            progress: order.progress,
            required_progress: order.required_progress,
            paid_cost: order.paid_cost,
            refund,
        })
    }

    /// `start`の翌日から`days`日ぶん、`run_day`を実行する(旧`advance_game_date`と同じく、
    /// 日付を進めてからその新しい日付で日次処理を行う)。各日の日付と出来事を返す。
    pub fn run_days(
        &mut self,
        countries: &mut [ScenarioCountry],
        start: SimDate,
        days: u32,
        building_registry: &BuildingRegistry,
    ) -> Vec<(SimDate, DayReport)> {
        let mut date = start;
        let mut out = Vec::with_capacity(days as usize);
        for _ in 0..days {
            let (next, month_changed) = date.next();
            date = next;
            let report = self.run_day(countries, date, month_changed, building_registry);
            out.push((date, report));
        }
        out
    }

    /// 1日分の処理。旧スケジュールと同じ順序: (1) 建設の進捗と完成(`handle_daily_construction`) →
    /// (2) 月が変わった日は月次経済(人口成長・人的資源・雇用・物流・生産・財政・福祉・経済状態) →
    /// (3) AI国家の月次評価(`handle_daily_country_ai`: 税率調整 → 建設判断)。
    /// `date`はこの日の日付、`month_changed`はこの日に月が変わったか。
    pub fn run_day(
        &mut self,
        countries: &mut [ScenarioCountry],
        date: SimDate,
        month_changed: bool,
        building_registry: &BuildingRegistry,
    ) -> DayReport {
        let mut report = DayReport::default();

        // (1) 建設の進捗・完成。完成した施設は、対応するステートへちょうど1回だけ反映する。
        for c in countries.iter_mut() {
            let label = c.label;
            let setup = self.resource_setup.clone();
            let states = &mut self.states;
            let order_log = &mut c.order_log;
            advance_construction_queue(&mut c.construction_queue, |o| {
                let Some(s) = states.get_mut(&o.state) else {
                    report.blocked.push((
                        label,
                        o.state,
                        o.building_type,
                        OrderBlock::Other("state no longer exists".into()),
                    ));
                    return CompletionOutcome::Blocked;
                };
                // 完成時の前提(注文後に資源や所属が変わっていないか)
                if o.building_type == BuildingType::Mine
                    && let Some(map) = &setup
                    && map.get(&o.state) != Some(&StateResourceSetup::Complete)
                {
                    report.blocked.push((
                        label,
                        o.state,
                        o.building_type,
                        OrderBlock::ResourcesNotConfigured,
                    ));
                    return CompletionOutcome::Blocked;
                }
                let current = s.building_level(o.building_type);
                s.buildings.insert(o.building_type, current + 1);
                report
                    .completed
                    .push((label, o.state, o.building_type, current + 1));
                order_log.push((o.id, OrderEnd::Completed));
                CompletionOutcome::Applied
            });
        }

        // (2) 月次経済
        if month_changed {
            report.monthly_ran = true;
            for c in countries.iter_mut() {
                let month = self.run_country_month(c, building_registry);
                self.record_ai_history(c, &month, (date.year, date.month));
            }
        }

        // (3) AI国家の評価(旧: `current_day % 30 == country_id % 30` または初回(dirty))
        let day_number = date.day_number();
        for c in countries.iter_mut() {
            let Some(ai) = c.ai.clone() else { continue };
            let due = (day_number as usize % 30 == c.label as usize % 30) || ai.dirty;
            if due {
                let before_tax = c.tax_rate;
                c.tax_rate = next_ai_tax_rate(
                    c.monthly_balance,
                    c.tax_rate,
                    c.monthly_expenses,
                    c.states
                        .iter()
                        .filter_map(|id| self.states.get(id))
                        .map(|s| (s.population, s.living_standard)),
                );
                if c.tax_rate != before_tax {
                    report.tax_changes.push((c.label, before_tax, c.tax_rate));
                }
                let queue_before = c.construction_queue.len();
                let owned = c.states.clone();
                let mut ai_state = CountryAiState::new(CountryId(c.label as usize));
                ai_state.dirty = ai.dirty;
                // 履歴と軍事状況から、旧経路と同じ文脈を作る(判断本体は`ai_military`で共通)。
                let now = (date.year, date.month);
                let regions = self.ai_regions();
                let mine_levels: u32 = owned
                    .iter()
                    .map(|id| regions.building_level(*id, BuildingType::Mine))
                    .sum();
                let factory_entries: Vec<(usize, u32)> = owned
                    .iter()
                    .filter_map(|id| self.states.get(id))
                    .map(|s| (s.economy_key() as usize, s.building_level(BuildingType::Factory)))
                    .filter(|(_, l)| *l > 0)
                    .collect();
                let iron_supply = iron_supply_context_from(&ai.history.iron, now, mine_levels);
                let factory_blocked =
                    factory_expansion_blocked_from(&ai.history.factory, now, &factory_entries);
                let wartime_supply = c.military.as_ref().and_then(wartime_supply_context_from);
                let military_rebuild = c.military.as_ref().and_then(|facts| {
                    military_rebuild_context_for(&regions, &owned, facts, &ai.history.supply, now)
                });
                let mut hooks = MilitaryHooks {
                    regions: &regions,
                    building_registry,
                    all_owned: &owned,
                    owned: &owned,
                    wartime_supply,
                    iron_supply,
                    military_rebuild,
                    fired: None,
                };
                run_construction_priorities(
                    c,
                    &regions,
                    building_registry,
                    &owned,
                    &mut ai_state,
                    factory_blocked,
                    &mut hooks,
                );
                report.evaluations.push(AiEvaluation {
                    label: c.label,
                    military_input: c.military,
                    wartime_context: wartime_supply.is_some(),
                    iron_shortage_average: iron_supply.map(|x| x.average_unmet),
                    rebuild_context: military_rebuild.is_some(),
                    factory_blocked,
                    fired: hooks.fired,
                    decision_reason: ai_state.decision_reason,
                });
                for o in c.construction_queue.iter().skip(queue_before) {
                    report
                        .ordered
                        .push((c.label, o.state, o.building_type, o.paid_cost));
                }
                if let Some(a) = c.ai.as_mut() {
                    a.last_decision_reason = ai_state.decision_reason;
                    a.last_monthly_evaluation_day = day_number;
                    a.dirty = false;
                }
            } else if let Some(a) = c.ai.as_mut() {
                a.dirty = false;
            }
        }
        report
    }
}
