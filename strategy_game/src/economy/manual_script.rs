//! P32-013: 新ステート(`MapStateId`)の手動建設注文・キャンセルを、**明示した要求の列**として与えて
//! 実行する検証用の経路(Bevy非依存)。入力(要求の列)は読み取り専用で、実行時の状態
//! (国庫・建設キュー・注文識別子・施設)は実行ごとに作られ、入力ファイルへ書き戻さない。
//! 通常起動へは注入しない。プレイヤーUIへは接続していない。
//!
//! 要求は「その日数が経過した後、次の日の処理の前」に順番どおり実行される。キャンセルの対象は、
//! 要求の通し番号(`Request(i)` = i番目の要求が発行した注文)・最後の注文(`Latest`)で指定する
//! (注文識別子は実行時に発行されるため、入力ファイルには書けない)。

use crate::building::data::{BuildingRegistry, BuildingType};
use crate::economy::map_state::{
    CancelError, CancelReceipt, CancelTarget, ConstructionOrderId, DayReport, EconomyDataIssue,
    MapStateEconomyRegistry, MultiMonthScenario, OrderError, ScenarioCountry, SimDate,
};
use crate::economy::province_resources::{ProvinceResourcesFile, derive_state_resources};
use crate::world::{MapStateId, WorldModel};
use serde::{Deserialize, Serialize};

pub const MANUAL_SCRIPT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CancelRef {
    /// i番目(0始まり)の要求が発行した注文。
    Request(usize),
    /// その国家の最後に登録された注文。
    Latest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ManualAction {
    /// 建設注文: 注文主体(国家ラベル)・対象`MapStateId`・施設。
    Build {
        country: u32,
        state: u32,
        building: BuildingType,
    },
    /// キャンセルと返金。
    Cancel { country: u32, target: CancelRef },
    /// 資源定義の変更(検証用): 指定プロヴィンスの資源設定を外した定義で資源の接続をやり直す
    /// (レジストリを作り直し、国家のキューと国庫は引き継ぐ。入力ファイルは書き換えない)。
    OmitResourceProvince(u32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualStep {
    /// この日数が経過した後に実行する。
    pub day: u32,
    pub action: ManualAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualScript {
    pub format_version: u32,
    pub description: String,
    /// 実行する総日数。
    pub days: u32,
    /// 開始時から資源設定を外すプロヴィンス(検証用の変種)。
    pub omit_resource_provinces: Vec<u32>,
    pub steps: Vec<ManualStep>,
}

pub fn parse_manual_script(text: &str) -> Result<ManualScript, String> {
    #[derive(Deserialize)]
    struct Header {
        format_version: u32,
    }
    let h: Header = ron::from_str(text).map_err(|e| e.to_string())?;
    if h.format_version != MANUAL_SCRIPT_FORMAT_VERSION {
        return Err(format!(
            "manual script has unsupported format_version {} (supported: {MANUAL_SCRIPT_FORMAT_VERSION})",
            h.format_version
        ));
    }
    let s: ManualScript = ron::from_str(text).map_err(|e| e.to_string())?;
    if s.steps.windows(2).any(|w| w[0].day > w[1].day) {
        return Err("manual script steps must be in ascending day order".to_string());
    }
    if s.steps.iter().any(|st| st.day > s.days) {
        return Err("manual script has a step after the last day".to_string());
    }
    Ok(s)
}

/// 要求の結果。
#[derive(Debug, Clone, PartialEq)]
pub enum RequestOutcome {
    BuildAccepted {
        id: ConstructionOrderId,
        cost: f64,
    },
    BuildRejected(OrderError),
    CancelAccepted(CancelReceipt),
    CancelRejected(CancelError),
    /// `Request(i)`が、受理された注文を指していない(入力の誤り)。
    CancelTargetNotIssued(usize),
    ResourcesReplaced,
}

/// 実行ログの1件。
#[derive(Debug, Clone, PartialEq)]
pub enum ManualLogEntry {
    Request {
        elapsed_days: u32,
        index: usize,
        action: ManualAction,
        outcome: RequestOutcome,
        /// 要求の国家の国庫(要求の前→後)。国家が無い場合は`None`。
        treasury: Option<(f64, f64)>,
    },
    Day {
        date: SimDate,
        report: DayReport,
    },
}

#[derive(Debug)]
pub struct ManualRun {
    pub log: Vec<ManualLogEntry>,
    pub registry: MapStateEconomyRegistry,
    pub countries: Vec<ScenarioCountry>,
}

fn build_registry(
    scenario: &MultiMonthScenario,
    world: &WorldModel,
    buildings: &BuildingRegistry,
    resources: &ProvinceResourcesFile,
    omit: &[u32],
) -> Result<(MapStateEconomyRegistry, Vec<ScenarioCountry>), String> {
    let fmt = |e: Vec<EconomyDataIssue>| e.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
    let mut file = resources.clone();
    file.provinces.retain(|e| !omit.contains(&e.province));
    let derived = derive_state_resources(world, &file)
        .map_err(|i| i.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("\n"))?;
    let (mut reg, countries) =
        MapStateEconomyRegistry::from_multi_scenario(scenario, world, buildings).map_err(fmt)?;
    reg.attach_province_resources(&derived, world, &file).map_err(fmt)?;
    Ok((reg, countries))
}

/// 要求の列を、日次処理(AI・建設・月次経済)と一緒に実行する。
pub fn run_manual_script(
    scenario: &MultiMonthScenario,
    world: &WorldModel,
    buildings: &BuildingRegistry,
    resources: &ProvinceResourcesFile,
    script: &ManualScript,
) -> Result<ManualRun, String> {
    let mut omit = script.omit_resource_provinces.clone();
    let (mut reg, mut countries) = build_registry(scenario, world, buildings, resources, &omit)?;
    let mut log = Vec::new();
    let mut issued: Vec<Option<(u32, ConstructionOrderId)>> = Vec::new();
    let mut date = SimDate { year: 1800, month: 1, day: 1 };
    let mut next_step = 0usize;
    for elapsed in 0..=script.days {
        while next_step < script.steps.len() && script.steps[next_step].day == elapsed {
            let step = &script.steps[next_step];
            let index = next_step;
            next_step += 1;
            let treasury_of = |cs: &[ScenarioCountry], label: u32| {
                cs.iter().find(|c| c.label == label).map(|c| c.treasury)
            };
            let (outcome, issuer) = match &step.action {
                ManualAction::Build { country, state, building } => {
                    let cost = buildings.get(*building).map(|d| d.construction_cost).unwrap_or(0.0);
                    let r = reg.request_build(&mut countries, *country, MapStateId(*state), *building, buildings);
                    match r {
                        Ok(id) => {
                            issued.push(Some((*country, id)));
                            (RequestOutcome::BuildAccepted { id, cost }, Some(*country))
                        }
                        Err(e) => {
                            issued.push(None);
                            (RequestOutcome::BuildRejected(e), Some(*country))
                        }
                    }
                }
                ManualAction::Cancel { country, target } => {
                    issued.push(None);
                    let t = match target {
                        CancelRef::Latest => Some(CancelTarget::Latest),
                        CancelRef::Request(i) => issued
                            .get(*i)
                            .copied()
                            .flatten()
                            .map(|(_, id)| CancelTarget::Order(id)),
                    };
                    match t {
                        None => (
                            RequestOutcome::CancelTargetNotIssued(match target {
                                CancelRef::Request(i) => *i,
                                CancelRef::Latest => 0,
                            }),
                            Some(*country),
                        ),
                        Some(t) => match reg.cancel_order(&mut countries, *country, t) {
                            Ok(r) => (RequestOutcome::CancelAccepted(r), Some(*country)),
                            Err(e) => (RequestOutcome::CancelRejected(e), Some(*country)),
                        },
                    }
                }
                ManualAction::OmitResourceProvince(p) => {
                    issued.push(None);
                    omit.push(*p);
                    // 国家の実行時状態(国庫・キュー・識別子)は引き継ぎ、レジストリだけを作り直す。
                    let (new_reg, _fresh) = build_registry(scenario, world, buildings, resources, &omit)?;
                    reg = new_reg;
                    (RequestOutcome::ResourcesReplaced, None)
                }
            };
            // 国庫の前後(要求は上で実行済みなので、前は返金/支払いを逆算せず、結果の記録に使う後の値のみ正確)。
            let after = issuer.and_then(|l| treasury_of(&countries, l));
            let before = match (&outcome, after) {
                (RequestOutcome::BuildAccepted { cost, .. }, Some(a)) => Some(a + cost),
                (RequestOutcome::CancelAccepted(r), Some(a)) => Some(a - r.refund),
                (_, Some(a)) => Some(a),
                _ => None,
            };
            log.push(ManualLogEntry::Request {
                elapsed_days: elapsed,
                index,
                action: step.action.clone(),
                outcome,
                treasury: before.zip(after),
            });
        }
        if elapsed == script.days {
            break;
        }
        let (d, changed) = date.next();
        date = d;
        let report = reg.run_day(&mut countries, date, changed, buildings);
        log.push(ManualLogEntry::Day { date, report });
    }
    Ok(ManualRun { log, registry: reg, countries })
}
