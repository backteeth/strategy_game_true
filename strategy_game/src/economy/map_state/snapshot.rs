//! P32-015: 新ステート経路の実行時状態の、メモリ上のスナップショット取得と検証付き復元
//! (P32-014 の設計 §4・§6)。**ファイルI/O・RONの封筒・外部静的データの署名判定は含まない**
//! (P32-016・P32-017)。
//!
//! ## 契約
//! - **保存可能な時点は日と日の間**。[`MapEconomySimulation`] は `run_day` を [`advance_day`]
//!   (日付を進めてから1日分の処理を完了する)の中でしか呼ばないので、`&self` を取れる時点は常に
//!   「日次処理が完了し、次の日の処理を始めていない」時点になる(途中の状態は外から見えない)。
//! - **日付の意味**: [`MapEconomySnapshot::date`] と [`MapEconomySimulation::date`] は
//!   **最後に処理を完了した日**。復元後の最初の [`advance_day`] は、その翌日を処理する
//!   (その日を二重に処理しない、飛ばさない)。
//! - **同一コンテキストでの再開に限る**。復元は、同じ検証済みの静的入力(地理・所属・資源・建物定義・
//!   シナリオ構造)から作った [`MapEconomySimulation`] に対して行う。スナップショットが持つ
//!   [`context`](MapEconomySnapshot::context) は、その静的入力の内部的な識別(FNV-1a 64bit)で、
//!   **別の地図・所属・資源・経済定義への復元を拒否する最小限の手段**にすぎない。外部ファイルの署名による
//!   照合(P32-017)の代わりではなく、ファイルから読んだ値を無条件に受け入れるAPIでもない。
//! - 復元は **候補の状態を新しく構築し、全検証が通ったときだけ現在の状態を置き換える**。
//!   失敗時は現在の状態を一切変更しない。復元は注文の受付・支払い・完成・取消し・返金・月次経済・AIの
//!   行動を**呼ばない**(保存した値を書き込むだけ)。
//!
//! ## 保存するもの / しないもの
//! - 保存: 日付、ステートの人口・施設レベル・生活水準・不満度、月次で確定した雇用・物流・稼働率、
//!   国家の国庫・税率・人的資源・備蓄・月次損益・経済状態、建設キュー(注文識別子・進捗・必要進捗・支払済み
//!   費用・状態)、`next_order_seq`、`order_log`(全件)、AIの `dirty`・判断理由・3つの履歴。
//! - 保存しない(静的入力。`context` で束縛): `workforce_ratio`・`base_logistics_capacity`・鉱床・
//!   資源設定・国家とステートの対応・AI国家か否か・軍事状況の入力・建物定義。
//! - 保存しない(出力ログ): `DayReport`・`AiEvaluation` 等。

use super::{
    AiHistories, ConstructionOrderId, MapConstructionOrder, MapStateEconomy,
    MapStateEconomyRegistry, OrderEnd, ScenarioAi, ScenarioCountry, SimDate,
};
use crate::building::construction::ConstructionStatus;
use crate::building::data::{BuildingRegistry, BuildingType};
use crate::country::country_ai::CountryAiDecisionReason;
use crate::economy::economic_state::EconomicState;
use crate::economy::factory_starvation::{FactoryStarvationHistory, FactoryStarvationSample};
use crate::economy::iron_shortage::{IronShortageHistory, IronShortageSample};
use crate::economy::resources::{CountryStockpile, ResourceType};
use crate::military::supply_production_history::{MonthlyProductionSample, SupplyProductionHistory};
use crate::world::{MapStateId, WorldModel};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// スナップショット(節)の版。意味を変える変更で上げる。
pub const SNAPSHOT_SECTION_VERSION: u32 = 2;
/// 経済計算の規則(コード上の式・定数)の改訂番号。**計算式や定数を変えたら手で上げる**(コードの
/// ハッシュは取れないため)。再開の一致は同じ規則のもとでしか保証できないので、不一致は拒否する。
pub const ECONOMY_RULES_REVISION: u32 = 1;

// ---------------------------------------------------------------- 実行時状態の集約

/// 日付を含む実行時状態の集約(スナップショット・復元の単位)。
#[derive(Debug, Clone, PartialEq)]
pub struct MapEconomySimulation {
    /// 最後に処理を完了した日(開始時は、まだ1日も処理していない開始日)。
    pub date: SimDate,
    pub registry: MapStateEconomyRegistry,
    pub countries: Vec<ScenarioCountry>,
}

impl MapEconomySimulation {
    pub fn new(
        date: SimDate,
        registry: MapStateEconomyRegistry,
        countries: Vec<ScenarioCountry>,
    ) -> Self {
        Self {
            date,
            registry,
            countries,
        }
    }

    /// 翌日を処理する(日付を進めてから、その新しい日付で1日分の処理を完了する)。
    /// 戻り値は処理した日と、その日の出来事。
    pub fn advance_day(
        &mut self,
        building_registry: &BuildingRegistry,
    ) -> (SimDate, super::DayReport) {
        let (next, month_changed) = self.date.next();
        self.date = next;
        let report = self
            .registry
            .run_day(&mut self.countries, next, month_changed, building_registry);
        (next, report)
    }
}

// ---------------------------------------------------------------- DTO(HashMapを持たない。決定的に直列化できる)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderIdSnapshot {
    pub country: u32,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSnapshot {
    pub state: u32,
    pub population: u64,
    pub employed_workforce: u64,
    pub unemployed_workforce: u64,
    pub logistics_capacity: f32,
    pub logistics_usage: f32,
    pub logistics_ratio: f32,
    pub living_standard: f32,
    pub unrest: f32,
    /// `BuildingType::ALL`の順。
    pub buildings: Vec<(BuildingType, u32)>,
    /// `BuildingType::ALL`の順。
    pub building_operation_ratios: Vec<(BuildingType, f32)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderSnapshot {
    pub id: OrderIdSnapshot,
    pub state: u32,
    pub building_type: BuildingType,
    pub target_level: u32,
    pub progress: f64,
    pub required_progress: f64,
    /// 実際に支払った費用(現在の建設費から再計算しない)。
    pub paid_cost: f64,
    pub status: ConstructionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderEndSnapshot {
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiSnapshot {
    pub dirty: bool,
    pub last_decision_reason: CountryAiDecisionReason,
    pub last_monthly_evaluation_day: u32,
    pub supply: Vec<SupplySampleSnapshot>,
    pub iron: Vec<IronSampleSnapshot>,
    pub factory: Vec<FactorySampleSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountrySnapshot {
    pub label: u32,
    pub treasury: f64,
    pub tax_rate: f32,
    pub available_manpower: u64,
    pub mobilized_manpower: u64,
    /// `ResourceType::ALL`の順(全種類)。
    pub stockpile: Vec<(ResourceType, f64)>,
    pub monthly_income: f64,
    pub monthly_expenses: f64,
    pub monthly_balance: f64,
    pub economic_state: EconomicState,
    pub construction_queue: Vec<OrderSnapshot>,
    pub next_order_seq: u64,
    pub order_log: Vec<(OrderIdSnapshot, OrderEndSnapshot)>,
    #[serde(deserialize_with = "required_option")]
    pub ai: Option<AiSnapshot>,
}

/// 新ステート経路の実行時状態のスナップショット(メモリ上。ファイル形式ではない)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapEconomySnapshot {
    pub section_version: u32,
    pub rules_revision: u32,
    /// 静的入力(地理・所属・資源・建物定義・シナリオ構造)の内部的な識別。同一コンテキストの確認用。
    pub context: u64,
    /// 最後に処理を完了した日。
    pub date: SnapshotDate,
    /// `MapStateId`昇順。
    pub states: Vec<StateSnapshot>,
    /// 集約の国家の並び順。
    pub countries: Vec<CountrySnapshot>,
}

// ---------------------------------------------------------------- 復元の拒否理由

#[derive(Debug, Clone, PartialEq)]
pub enum RestoreIssue {
    UnsupportedVersion { found: u32 },
    RulesRevisionMismatch { found: u32 },
    /// 静的入力(地図・所属・資源・建物定義・シナリオ構造)が、取得時と同じではない。
    ContextMismatch,
    InvalidDate(String),
    /// ステートの集合・順序が、復元先の集約と一致しない。
    StateSetMismatch(String),
    /// 国家の集合・順序・AI国家の別が、復元先の集約と一致しない。
    CountrySetMismatch(String),
    InvalidValue(String),
    UndefinedBuilding { state: u32, building: BuildingType },
    InvalidOrder(String),
    InvalidHistory(String),
    /// 候補の状態から取り直したスナップショットが、入力と一致しない。
    RoundTripMismatch,
}

impl std::fmt::Display for RestoreIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreIssue::UnsupportedVersion { found } => {
                write!(f, "unsupported snapshot section version {found} (supported: {SNAPSHOT_SECTION_VERSION})")
            }
            RestoreIssue::RulesRevisionMismatch { found } => write!(
                f,
                "economy rules revision {found} differs from the current {ECONOMY_RULES_REVISION}"
            ),
            RestoreIssue::ContextMismatch => {
                write!(f, "the snapshot was taken under different static inputs (map, ownership, resources, building definitions or scenario structure)")
            }
            RestoreIssue::InvalidDate(s) => write!(f, "invalid date: {s}"),
            RestoreIssue::StateSetMismatch(s) => write!(f, "state set mismatch: {s}"),
            RestoreIssue::CountrySetMismatch(s) => write!(f, "country set mismatch: {s}"),
            RestoreIssue::InvalidValue(s) => write!(f, "invalid value: {s}"),
            RestoreIssue::UndefinedBuilding { state, building } => {
                write!(f, "MapState#{state} refers to undefined building {building:?}")
            }
            RestoreIssue::InvalidOrder(s) => write!(f, "invalid construction order: {s}"),
            RestoreIssue::InvalidHistory(s) => write!(f, "invalid AI history: {s}"),
            RestoreIssue::RoundTripMismatch => {
                write!(f, "the rebuilt state does not reproduce the snapshot")
            }
        }
    }
}

/// 復元の失敗。問題はすべて集めて返す。失敗時、現在の状態は変更されていない。
#[derive(Debug, Clone, PartialEq)]
pub struct RestoreError {
    pub issues: Vec<RestoreIssue>,
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, issue) in self.issues.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{issue}")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- 内部の識別(FNV-1a 64bit)

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        for b in s.bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn f32(&mut self, v: f32) {
        self.u64(v.to_bits() as u64);
    }
    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
}

pub(super) fn building_index(b: BuildingType) -> u64 {
    BuildingType::ALL.iter().position(|x| *x == b).unwrap_or(usize::MAX) as u64
}

pub(super) fn resource_index(r: ResourceType) -> u64 {
    ResourceType::ALL.iter().position(|x| *x == r).unwrap_or(usize::MAX) as u64
}

impl MapEconomySimulation {
    /// 静的入力(地理・所属・資源・建物定義・シナリオ構造)の内部的な識別。実行中に変わる値を含まない。
    fn context_id(&self, world: &WorldModel, buildings: &BuildingRegistry) -> u64 {
        let mut h = Fnv::new();
        let g = world.geography();
        h.u64(g.algorithm as u64);
        // 属性(地形・気候・植生)と河川は含めない: 現在の保存対象の計算が読まない項目で、判定表(`static_identity::RULES`)が
        // 警告で許す。ここに入れると、警告で許可した差分が内部照合で再び拒否される。
        for s in [&g.grid, &g.coordinates, &g.land_set] {
            h.str(s);
        }
        for (id, s) in &self.registry.states {
            h.u64(0xA000_0000 | id.0 as u64);
            if let Some(provinces) = world.provinces_of(*id) {
                h.u64(provinces.len() as u64);
                for p in provinces {
                    h.u64(p.0 as u64);
                }
            } else {
                h.u64(u64::MAX);
            }
            h.f32(s.workforce_ratio);
            h.f32(s.base_logistics_capacity);
            h.u64(s.resource_deposits.len() as u64);
            for d in &s.resource_deposits {
                h.u64(resource_index(d.resource_type));
                h.f64(d.base_output);
                h.u64(d.discovered as u64);
                h.u64(d.development_level as u64);
            }
        }
        match &self.registry.resource_setup {
            None => h.u64(0),
            Some(map) => {
                h.u64(1);
                for (id, setup) in map {
                    h.u64(id.0 as u64);
                    h.u64(*setup as u64);
                }
            }
        }
        for c in &self.countries {
            h.u64(0xB000_0000 | c.label as u64);
            h.u64(c.ai.is_some() as u64);
            h.u64(c.states.len() as u64);
            for s in &c.states {
                h.u64(s.0 as u64);
            }
            match &c.military {
                None => h.u64(0),
                Some(m) => {
                    h.u64(1);
                    h.u64(m.at_war as u64);
                    match m.daily_demand_units {
                        None => h.u64(0),
                        Some(v) => {
                            h.u64(1);
                            h.u64(v);
                        }
                    }
                    h.u64(m.living_divisions);
                    h.u64(m.recruitment_queue_len);
                    match m.infantry_required_manpower {
                        None => h.u64(0),
                        Some(v) => {
                            h.u64(1);
                            h.u64(v);
                        }
                    }
                }
            }
        }
        for b in BuildingType::ALL {
            h.u64(0xC000_0000 | building_index(b));
            match buildings.get(b) {
                None => h.u64(0),
                Some(d) => {
                    h.u64(1);
                    h.f64(d.construction_cost);
                    h.f64(d.required_progress);
                    h.f32(d.required_workforce);
                    h.f32(d.logistics_cost);
                    h.f64(d.maintenance_cost);
                    h.u64(d.max_level as u64);
                    h.f64(d.science_output);
                    h.f64(d.magic_output);
                    h.f32(d.railway_capacity_bonus);
                    for (side, map) in [(1u64, &d.input_resources), (2u64, &d.output_resources)] {
                        h.u64(side);
                        for r in ResourceType::ALL {
                            if let Some(v) = map.get(&r) {
                                h.u64(resource_index(r));
                                h.f64(*v);
                            }
                        }
                    }
                }
            }
        }
        h.0
    }

    // ------------------------------------------------------------ 取得

    /// 現在の実行時状態のスナップショットを取る。`&self`を取れる時点は常に日と日の間(日次処理の完了後)。
    /// 状態は一切変更しない。
    pub fn snapshot(&self, world: &WorldModel, buildings: &BuildingRegistry) -> MapEconomySnapshot {
        let order_id = |id: ConstructionOrderId| OrderIdSnapshot {
            country: id.country,
            seq: id.seq,
        };
        MapEconomySnapshot {
            section_version: SNAPSHOT_SECTION_VERSION,
            rules_revision: ECONOMY_RULES_REVISION,
            context: self.context_id(world, buildings),
            date: SnapshotDate {
                year: self.date.year,
                month: self.date.month,
                day: self.date.day,
            },
            states: self
                .registry
                .states
                .values()
                .map(|s| StateSnapshot {
                    state: s.state.0,
                    population: s.population,
                    employed_workforce: s.employed_workforce,
                    unemployed_workforce: s.unemployed_workforce,
                    logistics_capacity: s.logistics_capacity,
                    logistics_usage: s.logistics_usage,
                    logistics_ratio: s.logistics_ratio,
                    living_standard: s.living_standard,
                    unrest: s.unrest,
                    buildings: BuildingType::ALL
                        .iter()
                        .filter_map(|b| s.buildings.get(b).map(|l| (*b, *l)))
                        .collect(),
                    building_operation_ratios: BuildingType::ALL
                        .iter()
                        .filter_map(|b| s.building_operation_ratios.get(b).map(|r| (*b, *r)))
                        .collect(),
                })
                .collect(),
            countries: self
                .countries
                .iter()
                .map(|c| CountrySnapshot {
                    label: c.label,
                    treasury: c.treasury,
                    tax_rate: c.tax_rate,
                    available_manpower: c.available_manpower,
                    mobilized_manpower: c.mobilized_manpower,
                    stockpile: ResourceType::ALL
                        .iter()
                        .map(|r| (*r, c.stockpile.get(*r)))
                        .collect(),
                    monthly_income: c.monthly_income,
                    monthly_expenses: c.monthly_expenses,
                    monthly_balance: c.monthly_balance,
                    economic_state: c.economic_state,
                    construction_queue: c
                        .construction_queue
                        .iter()
                        .map(|o| OrderSnapshot {
                            id: order_id(o.id),
                            state: o.state.0,
                            building_type: o.building_type,
                            target_level: o.target_level,
                            progress: o.progress,
                            required_progress: o.required_progress,
                            paid_cost: o.paid_cost,
                            status: o.status,
                        })
                        .collect(),
                    next_order_seq: c.next_order_seq,
                    order_log: c
                        .order_log
                        .iter()
                        .map(|(id, end)| {
                            (
                                order_id(*id),
                                match end {
                                    OrderEnd::Completed => OrderEndSnapshot::Completed,
                                    OrderEnd::Cancelled => OrderEndSnapshot::Cancelled,
                                },
                            )
                        })
                        .collect(),
                    ai: c.ai.as_ref().map(|a| AiSnapshot {
                        dirty: a.dirty,
                        last_decision_reason: a.last_decision_reason,
                        last_monthly_evaluation_day: a.last_monthly_evaluation_day,
                        supply: a.history.supply.samples().iter().map(SupplySampleSnapshot::from).collect(),
                        iron: a.history.iron.samples().iter().map(IronSampleSnapshot::from).collect(),
                        factory: a.history.factory.samples().iter().map(FactorySampleSnapshot::from).collect(),
                    }),
                })
                .collect(),
        }
    }

    // ------------------------------------------------------------ 復元

    /// スナップショットから候補の状態を**新しく構築**する(`self`は変更しない)。`self`は、スナップショットと
    /// 同じ静的入力から作った集約であること(静的な値・国家の対応・資源設定の持ち主)。動的な値は
    /// すべてスナップショットの値で構築し、静的な値だけを`self`から引き継ぐ。初期シナリオからの
    /// 再初期化や、`run_day`・注文・取消し・返金の再実行は行わない。
    pub fn restore_candidate(
        &self,
        snap: &MapEconomySnapshot,
        world: &WorldModel,
        buildings: &BuildingRegistry,
    ) -> Result<MapEconomySimulation, RestoreError> {
        let mut issues = Vec::new();
        self.validate_snapshot(snap, world, buildings, &mut issues);
        if !issues.is_empty() {
            return Err(RestoreError { issues });
        }
        let candidate = self.build_candidate(snap, &mut issues);
        let Some(candidate) = candidate else {
            return Err(RestoreError { issues });
        };
        // 全体の整合性: 候補から取り直した結果が入力と一致する(入力に無い値で状態が決まっていない)。
        if candidate.snapshot(world, buildings) != *snap {
            return Err(RestoreError {
                issues: vec![RestoreIssue::RoundTripMismatch],
            });
        }
        Ok(candidate)
    }

    /// 検証→候補の構築→全体の確認が成功したときだけ、現在の状態を1回の代入で置き換える。
    /// 失敗時は`self`を一切変更しない。
    pub fn restore(
        &mut self,
        snap: &MapEconomySnapshot,
        world: &WorldModel,
        buildings: &BuildingRegistry,
    ) -> Result<(), RestoreError> {
        let candidate = self.restore_candidate(snap, world, buildings)?;
        *self = candidate;
        Ok(())
    }

    fn validate_snapshot(
        &self,
        snap: &MapEconomySnapshot,
        world: &WorldModel,
        buildings: &BuildingRegistry,
        issues: &mut Vec<RestoreIssue>,
    ) {
        // 1. 版・規則・コンテキスト
        if snap.section_version != SNAPSHOT_SECTION_VERSION {
            issues.push(RestoreIssue::UnsupportedVersion {
                found: snap.section_version,
            });
            return;
        }
        if snap.rules_revision != ECONOMY_RULES_REVISION {
            issues.push(RestoreIssue::RulesRevisionMismatch {
                found: snap.rules_revision,
            });
            return;
        }
        if snap.context != self.context_id(world, buildings) {
            issues.push(RestoreIssue::ContextMismatch);
            return;
        }

        // 2. 日付(最後に処理を完了した日)
        let d = snap.date;
        match crate::app::time::GameDate::days_in_month(d.month) {
            None => issues.push(RestoreIssue::InvalidDate(format!("month {}", d.month))),
            Some(max) if d.day == 0 || d.day > max => issues.push(RestoreIssue::InvalidDate(
                format!("day {} is outside 1..={max} of month {}", d.day, d.month),
            )),
            _ => {}
        }
        if !(1..=9999).contains(&d.year) {
            issues.push(RestoreIssue::InvalidDate(format!("year {}", d.year)));
        }
        let date_ok = !issues.iter().any(|i| matches!(i, RestoreIssue::InvalidDate(_)));

        // 3. ステートの集合と値
        let expected: Vec<u32> = self.registry.states.keys().map(|k| k.0).collect();
        let found: Vec<u32> = snap.states.iter().map(|s| s.state).collect();
        if expected != found {
            let missing: Vec<u32> = expected.iter().filter(|x| !found.contains(x)).copied().collect();
            let unexpected: Vec<u32> = found.iter().filter(|x| !expected.contains(x)).copied().collect();
            let mut seen = BTreeSet::new();
            let duplicated: Vec<u32> = found.iter().filter(|x| !seen.insert(**x)).copied().collect();
            issues.push(RestoreIssue::StateSetMismatch(format!(
                "missing {missing:?}, unexpected {unexpected:?}, duplicated {duplicated:?} (states must be the exact set, in ascending order)"
            )));
        }
        for s in &snap.states {
            self.validate_state(s, world, buildings, issues);
        }

        // 4. 国家の集合・AI国家の別・値
        let exp_labels: Vec<u32> = self.countries.iter().map(|c| c.label).collect();
        let got_labels: Vec<u32> = snap.countries.iter().map(|c| c.label).collect();
        if exp_labels != got_labels {
            issues.push(RestoreIssue::CountrySetMismatch(format!(
                "expected labels {exp_labels:?} in this order, found {got_labels:?}"
            )));
            return;
        }
        let levels: HashMap<(u32, BuildingType), u32> = snap
            .states
            .iter()
            .flat_map(|s| s.buildings.iter().map(move |(b, l)| ((s.state, *b), *l)))
            .collect();
        for (cur, c) in self.countries.iter().zip(&snap.countries) {
            if cur.ai.is_some() != c.ai.is_some() {
                issues.push(RestoreIssue::CountrySetMismatch(format!(
                    "country {}: AI-controlled flag differs from the target context",
                    c.label
                )));
            }
            validate_country(cur, c, &levels, buildings, date_ok.then_some(d), issues);
        }
    }

    fn validate_state(
        &self,
        s: &StateSnapshot,
        world: &WorldModel,
        buildings: &BuildingRegistry,
        issues: &mut Vec<RestoreIssue>,
    ) {
        let id = MapStateId(s.state);
        if world.state(id).is_none() {
            issues.push(RestoreIssue::InvalidValue(format!("{id} does not exist in the world")));
        }
        // 実行時の値の検証。入力用の規則(0 < 値 <= 1 を旧スケールとして拒否)は使わない: 実行中に
        // 正当に生じた低い生活水準・不満度を拒否しないため。
        let bad = |what: &str, v: f32, lo: f32, hi: f32| {
            (!v.is_finite() || v < lo || v > hi)
                .then(|| RestoreIssue::InvalidValue(format!("{id} {what} {v} must be finite and within [{lo}, {hi}]")))
        };
        for issue in [
            bad("living_standard", s.living_standard, 0.0, 100.0),
            bad("unrest", s.unrest, 0.0, 100.0),
            bad("logistics_capacity", s.logistics_capacity, 0.0, f32::MAX),
            bad("logistics_usage", s.logistics_usage, 0.0, f32::MAX),
            bad("logistics_ratio", s.logistics_ratio, 0.0, f32::MAX),
        ]
        .into_iter()
        .flatten()
        {
            issues.push(issue);
        }
        let mut seen = BTreeSet::new();
        for (b, _) in &s.buildings {
            if buildings.get(*b).is_none() {
                issues.push(RestoreIssue::UndefinedBuilding {
                    state: s.state,
                    building: *b,
                });
            }
            if !seen.insert(building_index(*b)) {
                issues.push(RestoreIssue::InvalidValue(format!("{id} lists building {b:?} twice")));
            }
        }
        if s.buildings.windows(2).any(|w| building_index(w[0].0) >= building_index(w[1].0)) {
            issues.push(RestoreIssue::InvalidValue(format!(
                "{id} buildings must be in the canonical order without duplicates"
            )));
        }
        for (b, r) in &s.building_operation_ratios {
            if let Some(i) = bad("operation ratio", *r, 0.0, f32::MAX) {
                issues.push(i);
            }
            if buildings.get(*b).is_none() {
                issues.push(RestoreIssue::UndefinedBuilding {
                    state: s.state,
                    building: *b,
                });
            }
        }
        if s
            .building_operation_ratios
            .windows(2)
            .any(|w| building_index(w[0].0) >= building_index(w[1].0))
        {
            issues.push(RestoreIssue::InvalidValue(format!(
                "{id} operation ratios must be in the canonical order without duplicates"
            )));
        }
    }

    fn build_candidate(
        &self,
        snap: &MapEconomySnapshot,
        issues: &mut Vec<RestoreIssue>,
    ) -> Option<MapEconomySimulation> {
        // ステート: 動的な値はスナップショットから、静的な値は復元先から(構造体リテラルで全項目を明示)。
        let mut states: BTreeMap<MapStateId, MapStateEconomy> = BTreeMap::new();
        for s in &snap.states {
            let id = MapStateId(s.state);
            let Some(cur) = self.registry.states.get(&id) else {
                issues.push(RestoreIssue::StateSetMismatch(format!("{id} is not in the target context")));
                return None;
            };
            states.insert(
                id,
                MapStateEconomy {
                    state: id,
                    population: s.population,
                    workforce_ratio: cur.workforce_ratio,
                    employed_workforce: s.employed_workforce,
                    unemployed_workforce: s.unemployed_workforce,
                    base_logistics_capacity: cur.base_logistics_capacity,
                    logistics_capacity: s.logistics_capacity,
                    logistics_usage: s.logistics_usage,
                    logistics_ratio: s.logistics_ratio,
                    living_standard: s.living_standard,
                    unrest: s.unrest,
                    buildings: s.buildings.iter().copied().collect(),
                    building_operation_ratios: s.building_operation_ratios.iter().copied().collect(),
                    resource_deposits: cur.resource_deposits.clone(),
                },
            );
        }
        let registry = MapStateEconomyRegistry {
            states,
            resource_setup: self.registry.resource_setup.clone(),
        };

        let mut countries = Vec::with_capacity(snap.countries.len());
        for (cur, c) in self.countries.iter().zip(&snap.countries) {
            let mut stockpile = CountryStockpile::new();
            for (r, a) in &c.stockpile {
                stockpile.set(*r, *a);
            }
            let ai = match (&cur.ai, &c.ai) {
                (Some(_), Some(a)) => {
                    let history = (|| -> Result<AiHistories, String> {
                        Ok(AiHistories {
                            supply: SupplyProductionHistory::from_samples(a.supply.iter().map(SupplySampleSnapshot::real).collect())?,
                            iron: IronShortageHistory::from_samples(a.iron.iter().map(IronSampleSnapshot::real).collect())?,
                            factory: FactoryStarvationHistory::from_samples(a.factory.iter().map(FactorySampleSnapshot::real).collect())?,
                        })
                    })();
                    match history {
                        Ok(history) => Some(ScenarioAi {
                            dirty: a.dirty,
                            last_decision_reason: a.last_decision_reason,
                            last_monthly_evaluation_day: a.last_monthly_evaluation_day,
                            history,
                        }),
                        Err(e) => {
                            issues.push(RestoreIssue::InvalidHistory(format!("country {}: {e}", c.label)));
                            return None;
                        }
                    }
                }
                _ => None,
            };
            let to_id = |i: &OrderIdSnapshot| ConstructionOrderId {
                country: i.country,
                seq: i.seq,
            };
            countries.push(ScenarioCountry {
                label: c.label,
                treasury: c.treasury,
                tax_rate: c.tax_rate,
                available_manpower: c.available_manpower,
                mobilized_manpower: c.mobilized_manpower,
                stockpile,
                states: cur.states.clone(),
                monthly_income: c.monthly_income,
                monthly_expenses: c.monthly_expenses,
                monthly_balance: c.monthly_balance,
                construction_queue: c
                    .construction_queue
                    .iter()
                    .map(|o| MapConstructionOrder {
                        id: to_id(&o.id),
                        state: MapStateId(o.state),
                        building_type: o.building_type,
                        target_level: o.target_level,
                        progress: o.progress,
                        required_progress: o.required_progress,
                        paid_cost: o.paid_cost,
                        status: o.status,
                    })
                    .collect(),
                next_order_seq: c.next_order_seq,
                order_log: c
                    .order_log
                    .iter()
                    .map(|(id, end)| {
                        (
                            to_id(id),
                            match end {
                                OrderEndSnapshot::Completed => OrderEnd::Completed,
                                OrderEndSnapshot::Cancelled => OrderEnd::Cancelled,
                            },
                        )
                    })
                    .collect(),
                economic_state: c.economic_state,
                ai,
                military: cur.military,
            });
        }
        Some(MapEconomySimulation {
            date: SimDate {
                year: snap.date.year,
                month: snap.date.month,
                day: snap.date.day,
            },
            registry,
            countries,
        })
    }
}

fn month_ordinal(year: i32, month: u8) -> i64 {
    year as i64 * 12 + (month as i64 - 1)
}

fn validate_country(
    cur: &ScenarioCountry,
    c: &CountrySnapshot,
    levels: &HashMap<(u32, BuildingType), u32>,
    buildings: &BuildingRegistry,
    date: Option<SnapshotDate>,
    issues: &mut Vec<RestoreIssue>,
) {
    let label = c.label;
    let mut bad = |what: String| issues.push(RestoreIssue::InvalidValue(format!("country {label} {what}")));
    // 国庫は赤字(負)になりうる(月次財政)。有限であることだけを要求する。
    if !c.treasury.is_finite() {
        bad(format!("treasury {} must be finite", c.treasury));
    }
    if !(c.tax_rate.is_finite() && (0.0..=1.0).contains(&c.tax_rate)) {
        bad(format!("tax_rate {} must be finite and within [0, 1]", c.tax_rate));
    }
    for (name, v) in [
        ("monthly_income", c.monthly_income),
        ("monthly_expenses", c.monthly_expenses),
        ("monthly_balance", c.monthly_balance),
    ] {
        if !v.is_finite() {
            bad(format!("{name} {v} must be finite"));
        }
    }
    let kinds: Vec<ResourceType> = c.stockpile.iter().map(|(r, _)| *r).collect();
    if kinds != ResourceType::ALL {
        bad("stockpile must list every resource type exactly once in the canonical order".to_string());
    }
    for (r, a) in &c.stockpile {
        if !(a.is_finite() && *a >= 0.0) {
            bad(format!("stockpile {r:?} {a} must be finite and >= 0"));
        }
    }

    // 注文
    let mut order_err = |what: String| issues.push(RestoreIssue::InvalidOrder(format!("country {label}: {what}")));
    if c.next_order_seq == 0 {
        order_err("next_order_seq must be at least 1".to_string());
    }
    let mut seqs = BTreeSet::new();
    let mut note_id = |id: &OrderIdSnapshot, what: &str, order_err: &mut dyn FnMut(String)| {
        if id.country != label {
            order_err(format!("{what} order {}-{} belongs to another country", id.country, id.seq));
        }
        if id.seq == 0 || id.seq >= c.next_order_seq {
            order_err(format!(
                "{what} order seq {} is outside 1..next_order_seq ({})",
                id.seq, c.next_order_seq
            ));
        }
        if !seqs.insert(id.seq) {
            order_err(format!("order seq {} appears more than once (queue/log)", id.seq));
        }
    };
    let mut pairs = BTreeSet::new();
    for o in &c.construction_queue {
        note_id(&o.id, "queued", &mut order_err);
        if !cur.states.contains(&MapStateId(o.state)) {
            order_err(format!("order {} targets MapState#{} which is not mapped to this country", o.id.seq, o.state));
        }
        if buildings.get(o.building_type).is_none() {
            order_err(format!("order {} refers to undefined building {:?}", o.id.seq, o.building_type));
        }
        if !pairs.insert((o.state, building_index(o.building_type))) {
            order_err(format!(
                "MapState#{} {:?} is queued more than once",
                o.state, o.building_type
            ));
        }
        // 注文は(ステート,施設)ごとに1つで、受付時に目標レベル=現在+1。完成した注文はキューに残らない。
        let level = levels.get(&(o.state, o.building_type)).copied().unwrap_or(0);
        if o.target_level != level + 1 {
            order_err(format!(
                "order {} target_level {} != current level {level} + 1 (a completed order must not remain queued)",
                o.id.seq, o.target_level
            ));
        }
        // 超過進捗は、完成保留(Blocked)中の注文で正当に生じるので拒否しない。有限・非負のみ。
        if !(o.progress.is_finite() && o.progress >= 0.0) {
            order_err(format!("order {} progress {} must be finite and >= 0", o.id.seq, o.progress));
        }
        if !(o.required_progress.is_finite() && o.required_progress > 0.0) {
            order_err(format!("order {} required_progress {} must be finite and > 0", o.id.seq, o.required_progress));
        }
        if !(o.paid_cost.is_finite() && o.paid_cost >= 0.0) {
            order_err(format!("order {} paid_cost {} must be finite and >= 0", o.id.seq, o.paid_cost));
        }
    }
    for (id, _) in &c.order_log {
        note_id(id, "logged", &mut order_err);
    }
    // 発行済みの連番(1..next_order_seq)はすべて、キューかログのどちらか1か所に存在する。
    if c.next_order_seq >= 1
        && (c.construction_queue.len() + c.order_log.len()) as u64 != c.next_order_seq - 1
    {
        order_err(format!(
            "queue ({}) + log ({}) do not account for every issued seq below next_order_seq ({})",
            c.construction_queue.len(),
            c.order_log.len(),
            c.next_order_seq
        ));
    }

    // 履歴
    if let Some(a) = &c.ai {
        let mut hist_err = |what: String| issues.push(RestoreIssue::InvalidHistory(format!("country {label}: {what}")));
        if let Some(d) = date {
            let now = month_ordinal(d.year, d.month);
            for (name, last) in [
                ("supply", a.supply.last().map(|s| month_ordinal(s.year, s.month))),
                ("iron", a.iron.last().map(|s| month_ordinal(s.year, s.month))),
                ("factory", a.factory.last().map(|s| month_ordinal(s.year, s.month))),
            ] {
                if last.is_some_and(|l| l > now) {
                    hist_err(format!("{name} history has a sample later than the snapshot date"));
                }
            }
            let day_number = (d.year as i64) * 365 + d.month as i64 * 30 + d.day as i64;
            if a.last_monthly_evaluation_day as i64 > day_number {
                hist_err("last_monthly_evaluation_day is later than the snapshot date".to_string());
            }
        }
        if let Err(e) = SupplyProductionHistory::from_samples(a.supply.iter().map(SupplySampleSnapshot::real).collect()) {
            hist_err(e);
        }
        if let Err(e) = IronShortageHistory::from_samples(a.iron.iter().map(IronSampleSnapshot::real).collect()) {
            hist_err(e);
        }
        if let Err(e) = FactoryStarvationHistory::from_samples(a.factory.iter().map(FactorySampleSnapshot::real).collect()) {
            hist_err(e);
        }
    }
}

// ---------------------------------------------------------------- 履歴の標本(DTO)
// 実行時の標本型(他のモジュールの型。旧V1の保存にも使われる)をそのまま直列化せず、このDTO専用の型で持つ。
// `Option`のフィールドは**必須**(欠落を黙って`None`=「集計失敗」へ補完しない。明示された`None`だけを許す)。

/// 欠落を許さない `Option`(serdeは既定で、欠落した `Option` フィールドを `None` にしてしまう)。
fn required_option<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplySampleSnapshot {
    pub year: i32,
    pub month: u8,
    #[serde(deserialize_with = "required_option")]
    pub units: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IronSampleSnapshot {
    pub year: i32,
    pub month: u8,
    #[serde(deserialize_with = "required_option")]
    pub unmet_milli: Option<u64>,
    pub mine_levels: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactorySampleSnapshot {
    pub year: i32,
    pub month: u8,
    pub signature: u64,
    pub starved: bool,
}

impl From<&MonthlyProductionSample> for SupplySampleSnapshot {
    fn from(s: &MonthlyProductionSample) -> Self {
        Self { year: s.year, month: s.month, units: s.units }
    }
}
impl SupplySampleSnapshot {
    fn real(&self) -> MonthlyProductionSample {
        MonthlyProductionSample { year: self.year, month: self.month, units: self.units }
    }
}
impl From<&IronShortageSample> for IronSampleSnapshot {
    fn from(s: &IronShortageSample) -> Self {
        Self { year: s.year, month: s.month, unmet_milli: s.unmet_milli, mine_levels: s.mine_levels }
    }
}
impl IronSampleSnapshot {
    fn real(&self) -> IronShortageSample {
        IronShortageSample {
            year: self.year,
            month: self.month,
            unmet_milli: self.unmet_milli,
            mine_levels: self.mine_levels,
        }
    }
}
impl From<&FactoryStarvationSample> for FactorySampleSnapshot {
    fn from(s: &FactoryStarvationSample) -> Self {
        Self { year: s.year, month: s.month, signature: s.signature, starved: s.starved }
    }
}
impl FactorySampleSnapshot {
    fn real(&self) -> FactoryStarvationSample {
        FactoryStarvationSample {
            year: self.year,
            month: self.month,
            signature: self.signature,
            starved: self.starved,
        }
    }
}
