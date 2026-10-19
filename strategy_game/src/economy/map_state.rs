//! P32-007 (M4-A): 新ステート(`MapStateId`)単位の経済シミュレーション状態。
//!
//! 静的な世界定義(`world::MapStateDef`: 名称・所属プロヴィンス)と、時間経過で変わる
//! シミュレーション状態(この`MapStateEconomy`)は別のデータとして持つ。所属ファイルや
//! ステート境界を経済データの保存先として流用しない。
//!
//! D9の方針に沿い、人口・雇用・経済施設(農場・工場・採掘施設など)の規模は**ステート単位**で
//! 1回だけ保持する(プロヴィンス数ぶん複製しない)。天然資源の所在地(プロヴィンス単位)と
//! 位置が重要な施設(港・要塞・鉄道駅)は今回実装しておらず、`resource_deposits`は
//! 「このステートで生産処理が利用できる鉱床(入力データ)」をステート単位に集計済みとして
//! 受け取るだけである。
//!
//! 計算は旧`StateData`と同じ中核関数(`economy::state_access`、`economy::production`)を通る。
//! ここに計算式の複製はない。セーブ形式・正式な世界設定・ゲームバランスとは無関係で、
//! 検証用シナリオ(`EconomyScenario`)の固定入力から作る。

use crate::building::data::{BuildingRegistry, BuildingType};
use crate::economy::production::{
    CountryProductionOutcome, process_country_production_with_outcome,
};
use crate::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use crate::economy::state_access::{StateEconomyAccess, update_monthly_workforce_and_logistics};
use crate::world::{MapStateId, WorldModel};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const ECONOMY_SCENARIO_FORMAT_VERSION: u32 = 1;

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
    fn set_logistics(&mut self, capacity: f32, usage: f32, ratio: f32) {
        self.logistics_capacity = capacity;
        self.logistics_usage = usage;
        self.logistics_ratio = ratio;
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

/// 検証用の経済シナリオ(1国家分の固定入力)。ゲームバランスや正式な世界設定とは区別する。
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
        }
    }
}

pub fn parse_economy_scenario(text: &str) -> Result<EconomyScenario, String> {
    #[derive(Deserialize)]
    struct Header {
        format_version: u32,
    }
    let h: Header = ron::from_str(text).map_err(|e| e.to_string())?;
    if h.format_version != ECONOMY_SCENARIO_FORMAT_VERSION {
        return Err(EconomyDataIssue::UnsupportedVersion {
            found: h.format_version,
        }
        .to_string());
    }
    ron::from_str(text).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- 実行時データ

/// `MapStateId`をキーにした、ステート経済データの集合。同じステートは1件だけ。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapStateEconomyRegistry {
    states: BTreeMap<MapStateId, MapStateEconomy>,
}

fn finite_nonneg(v: f64) -> bool {
    v.is_finite() && v >= 0.0
}

impl MapStateEconomyRegistry {
    /// 検証用シナリオから作る。ワールド定義にないステート・重複・不正値・未定義の施設は
    /// 一括でエラーとして返し、部分的なデータを採用しない。雇用・物流の導出値は0で初期化し、
    /// 月次計算(`run_month`)が書き込む。
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
        for (res, amount) in &scenario.stockpile {
            if !finite_nonneg(*amount) {
                issues.push(EconomyDataIssue::InvalidValue {
                    state: None,
                    what: format!("stockpile {res:?} = {amount} (must be finite and >= 0)"),
                });
            }
        }
        let mut out: BTreeMap<MapStateId, MapStateEconomy> = BTreeMap::new();
        for inp in &scenario.states {
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
            if !(inp.workforce_ratio.is_finite() && (0.0..=1.0).contains(&inp.workforce_ratio)) {
                issues.push(EconomyDataIssue::InvalidValue {
                    state: Some(id),
                    what: format!(
                        "workforce_ratio {} must be finite and within [0.0, 1.0]",
                        inp.workforce_ratio
                    ),
                });
                ok = false;
            }
            if !finite_nonneg(inp.base_logistics_capacity as f64) {
                issues.push(EconomyDataIssue::InvalidValue {
                    state: Some(id),
                    what: format!(
                        "base_logistics_capacity {} must be finite and >= 0",
                        inp.base_logistics_capacity
                    ),
                });
                ok = false;
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
            for d in &inp.resource_deposits {
                if !finite_nonneg(d.base_output) {
                    issues.push(EconomyDataIssue::InvalidValue {
                        state: Some(id),
                        what: format!(
                            "deposit {:?} base_output {} must be finite and >= 0",
                            d.resource_type, d.base_output
                        ),
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
                    buildings: inp.buildings.clone(),
                    building_operation_ratios: HashMap::new(),
                    resource_deposits: inp.resource_deposits.clone(),
                },
            );
        }
        if issues.is_empty() {
            Ok(Self { states: out })
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

    /// 1か月分の人口・雇用・物流・生産(旧経路と同じ中核計算)を実行する。
    /// 財政・福祉・人口成長・生産チェーンの外側の処理(国家AI等)は含まない。
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
}

/// シナリオの備蓄(`ResourceType`→量)から国家備蓄を作る。
pub fn stockpile_from_scenario(scenario: &EconomyScenario) -> CountryStockpile {
    let mut s = CountryStockpile::new();
    for (res, amount) in &scenario.stockpile {
        s.set(*res, *amount);
    }
    s
}
