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

use crate::building::data::{BuildingRegistry, BuildingType};
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
            Ok(Self { states: out })
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
            Ok((Self { states: out }, countries))
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

        CountryMonthReport {
            label: country.label,
            is_bankrupt,
            owned_population,
            finance,
            science: production.science,
            magic: production.magic,
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
}

/// シナリオの備蓄(`ResourceType`→量)から国家備蓄を作る。
pub fn stockpile_from_scenario(scenario: &EconomyScenario) -> CountryStockpile {
    let mut s = CountryStockpile::new();
    for (res, amount) in &scenario.stockpile {
        s.set(*res, *amount);
    }
    s
}
