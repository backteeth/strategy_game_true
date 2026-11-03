//! P32-024: 新ステート経済の**規模測定ハーネス**(合成シナリオ)。
//!
//! 本番の `MapEconomySimulation` と共有の保存API(`map_economy_file::save/load`)をそのまま動かし、ステート数を
//! 変えたときの処理時間・実行の決定性・保存再開後の継続一致を測る。簡略化した別シミュレーターは使わない。
//!
//! ## 合成入力(ハーネス側にまとめる)
//! 実地図の陸セルを幅優先で区切って `MapState` を作り([`generate`])、既存のプロヴィンス表・ステート配置の型へ入れて
//! `WorldModel::build` と Authoring 検証([`build_world`])を通す。経済シナリオ・資源設定は既存の入力型で、
//! 組み立ては共有の `map_session::build_simulation_from_data`(検証は既存のものがそのまま走る)。
//! **同じ [`ScaleSpec`](seed を含む)からは常に同じ入力**ができる(乱数は SplitMix64 を seed とID から導出し、
//! 列挙順や `HashMap` の順序に依存しない)。
//!
//! ## 限界(必ず併記すること)
//! 合成モデルは既存の検証シナリオ(m4d/m4e)と同一条件ではない。正式な全世界のステート数・人口・資源・施設・国家配置・
//! ゲームバランスを代表しない。位置施設(`LocationFacilityRegistry`)は経済に効果を持たないので載せない。
//! 軍事状況の入力(`military`)は与えない(優先度2.5・2.9は評価されない)。
//!
//! ## 計測の範囲
//! - 日次進行の時間: `advance_day` の呼び出しだけ(`Instant` で前後を挟む)。手動注文の実行、状態の比較、ダイジェスト計算、
//!   出力の整形は含めない(それぞれ別項目)。
//! - 入力生成・世界の検証・セッション組み立て・保存・復元は別項目。
//! - 1ケース全体(これらすべて+比較)に壁時計の上限を設け、超過は**未完了**として記録する(正しさの失敗ではない)。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::building::data::{BuildingRegistry, BuildingType};
use crate::economy::map_economy_file::{self, SaveLocation};
use crate::economy::map_session::build_simulation_from_data;
use crate::economy::map_state::snapshot::MapEconomySimulation;
use crate::economy::map_state::static_identity::StaticInputs;
use crate::economy::map_state::{
    DayReport, MULTI_MONTH_SCENARIO_FORMAT_VERSION, MapStateEconomyInput, MultiMonthScenario, ScenarioCountryInput, SimDate,
};
use crate::economy::province_resources::{PROVINCE_RESOURCES_FORMAT_VERSION, ProvinceResourceEntry, ProvinceResourcesFile, ResourceScope};
use crate::economy::resources::{ResourceType, StateResourceDeposit};
use crate::hexmap::data::{HexMap, hex_neighbors};
use crate::world::files::{Coverage, ProvinceEntry, ProvinceTableFile, StateEntry, StateLayoutFile, WORLD_FORMAT_VERSION};
use crate::world::{
    MapStateId, ValidationMode, WorldModel, compute_fingerprint, validate_against_geography,
};
use crate::world::files::generate_initial_province_table;

/// 開始日(m4d/m4e と同じ。まだ1日も処理していない日)。
pub const START: SimDate = SimDate { year: 1800, month: 1, day: 1 };
/// 既定の測定点(主系列。国家数は3に固定)。
pub const MAIN_SERIES: [u32; 5] = [3, 30, 100, 300, 1000];
pub const MAIN_SERIES_COUNTRIES: u32 = 3;
/// 国家数を増やした別条件のケース(主系列とは別に表示する)。
pub const WIDE_CASE: (u32, u32) = (1000, 100);
pub const DEFAULT_DAYS: u32 = 360;
pub const DEFAULT_SPLIT_DAY: u32 = 180;
pub const DEFAULT_RUNS: u32 = 3;
pub const DEFAULT_SEED: u64 = 24;
pub const DEFAULT_PROVINCES_PER_STATE: u32 = 6;
pub const DEFAULT_TIME_LIMIT_SECS: u64 = 120;
/// 手動国家(ラベル1)が建設注文を出す日(処理済みの日数。次の日の処理の前)。
pub const SCRIPT_DAYS: [u32; 5] = [0, 45, 170, 200, 300];
/// 1回の注文日に手動国家が発注を試みるステート数の上限。
pub const SCRIPT_STATES_PER_DAY: usize = 5;
const SAVE_NAME: &str = "p32_024_scale";
const MAX_STATES: u32 = 20_000;
const MAX_DAYS: u32 = 3_650;

// ---------------------------------------------------------------- 指定

/// 1ケースの指定。同じ指定から同じ入力・同じ測定手順になる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleSpec {
    pub states: u32,
    pub countries: u32,
    pub provinces_per_state: u32,
    pub seed: u64,
    pub days: u32,
    /// 保存して再開する日(処理済みの日数)。`0 < split_day < days`。
    pub split_day: u32,
    /// 同一条件での連続実行の回数(決定性比較。1以上)。
    pub runs: u32,
    pub time_limit: Duration,
    /// 手動国家(ラベル1)の建設注文の台本を使うか。
    pub scripted_orders: bool,
}

impl ScaleSpec {
    pub fn new(states: u32, countries: u32) -> Self {
        ScaleSpec {
            states,
            countries,
            provinces_per_state: DEFAULT_PROVINCES_PER_STATE,
            seed: DEFAULT_SEED,
            days: DEFAULT_DAYS,
            split_day: DEFAULT_SPLIT_DAY,
            runs: DEFAULT_RUNS,
            time_limit: Duration::from_secs(DEFAULT_TIME_LIMIT_SECS),
            scripted_orders: true,
        }
    }

    /// 不正な指定を明示的に拒否する(黙って補正しない)。
    pub fn validate(&self) -> Result<(), ScaleError> {
        let bad = |m: String| Err(ScaleError::InvalidSpec(m));
        if self.states == 0 || self.states > MAX_STATES {
            return bad(format!("states must be within 1..={MAX_STATES} (got {})", self.states));
        }
        if self.countries == 0 {
            return bad("countries must be at least 1".into());
        }
        if self.countries > self.states {
            return bad(format!("countries ({}) must not exceed states ({}): every country needs a state", self.countries, self.states));
        }
        if self.provinces_per_state == 0 || self.provinces_per_state > 64 {
            return bad(format!("provinces-per-state must be within 1..=64 (got {})", self.provinces_per_state));
        }
        if self.days == 0 || self.days > MAX_DAYS {
            return bad(format!("days must be within 1..={MAX_DAYS} (got {})", self.days));
        }
        if self.split_day == 0 || self.split_day >= self.days {
            return bad(format!(
                "split must satisfy 0 < split < days (split {}, days {}): an empty half would prove nothing",
                self.split_day, self.days
            ));
        }
        if self.runs == 0 || self.runs > 20 {
            return bad(format!("runs must be within 1..=20 (got {})", self.runs));
        }
        if self.time_limit.is_zero() {
            return bad("time limit must be positive".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScaleError {
    InvalidSpec(String),
    /// 地図の陸セルから、指定数のステートを作れない。
    InsufficientLand { needed_states: u32, built_states: u32, land_cells: usize },
    Data(String),
}

impl std::fmt::Display for ScaleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScaleError::InvalidSpec(m) => write!(f, "invalid specification: {m}"),
            ScaleError::InsufficientLand { needed_states, built_states, land_cells } => write!(
                f,
                "the map cannot supply {needed_states} states of the requested size (built {built_states} from {land_cells} land cells)"
            ),
            ScaleError::Data(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ScaleError {}

// ---------------------------------------------------------------- 乱数(seed と ID から導出。生成順に依存しない)

#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn for_key(seed: u64, domain: u64, key: u64) -> Rng {
        let mut r = Rng(seed ^ domain.wrapping_mul(0xA24B_AED4_963E_E407) ^ key.wrapping_mul(0x9FB2_1C65_1E98_DF25));
        r.next(); // 混ぜる
        r
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// `lo..=hi`
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

const DOMAIN_START: u64 = 1;
const DOMAIN_STATE: u64 = 2;
const DOMAIN_PROVINCE: u64 = 3;

// ---------------------------------------------------------------- 合成入力

/// 生成した入力の要約(記録用)。
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationSummary {
    pub states: usize,
    pub provinces: usize,
    pub countries: usize,
    pub states_per_country_min: usize,
    pub states_per_country_max: usize,
    pub land_cells_available: usize,
    pub regions_discarded: usize,
    pub population_total: u64,
    pub buildings_total: BTreeMap<String, u64>,
    pub provinces_with_deposits: usize,
    pub deposits_iron: usize,
    pub deposits_coal: usize,
    pub treasury_total: f64,
    pub manual_country: u32,
    /// 入力全体の正準な内容ダイジェスト(BLAKE3、先頭16桁)。同じ指定で同じ値になる。
    pub input_digest: String,
}

/// 生成された入力一式。
#[derive(Debug, Clone, PartialEq)]
pub struct SyntheticInput {
    pub table: ProvinceTableFile,
    pub layout: StateLayoutFile,
    pub scenario: MultiMonthScenario,
    pub resources: ProvinceResourcesFile,
    pub summary: GenerationSummary,
}

/// 手動の建設注文を出す国家のラベル(それ以外はAI国家)。
pub const MANUAL_COUNTRY: u32 = 1;

/// 陸セル(`(r, q)` 昇順)を seed で決めた位置から巡り、幅優先で `provinces_per_state` 個ずつのステートに区切る。
/// 目標数に届かない断片は捨てる(`regions_discarded`)。ステートは必ず陸で連結している(幅優先の性質)。
pub fn generate(spec: &ScaleSpec, map: &HexMap) -> Result<SyntheticInput, ScaleError> {
    spec.validate()?;
    let mut land: Vec<(i32, i32)> = map
        .cells()
        .iter()
        .filter(|c| !c.terrain.is_water())
        .map(|c| (c.r, c.q))
        .collect();
    land.sort_unstable();
    if land.is_empty() {
        return Err(ScaleError::Data("the map has no land cells".into()));
    }
    let full = generate_initial_province_table(map, Coverage::World);
    let id_of: HashMap<(i32, i32), u32> = full.provinces.iter().map(|p| ((p.q, p.r), p.id)).collect();

    let start = (Rng::for_key(spec.seed, DOMAIN_START, 0).next() % land.len() as u64) as usize;
    let k = spec.provinces_per_state as usize;
    let mut used: HashSet<(i32, i32)> = HashSet::new(); // 採用済みまたは破棄した(q, r)
    let mut regions: Vec<Vec<(i32, i32)>> = Vec::new();
    let mut discarded = 0usize;
    for off in 0..land.len() {
        if regions.len() as u32 == spec.states {
            break;
        }
        let (r0, q0) = land[(start + off) % land.len()];
        if used.contains(&(q0, r0)) {
            continue;
        }
        let mut region: Vec<(i32, i32)> = Vec::with_capacity(k);
        let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
        let mut seen: HashSet<(i32, i32)> = HashSet::new();
        queue.push_back((q0, r0));
        seen.insert((q0, r0));
        while let Some((q, r)) = queue.pop_front() {
            if region.len() == k {
                break;
            }
            region.push((q, r));
            for (nq, nr) in hex_neighbors(q, r) {
                if used.contains(&(nq, nr)) || seen.contains(&(nq, nr)) {
                    continue;
                }
                if map.cell(nq, nr).is_some_and(|c| !c.terrain.is_water()) {
                    seen.insert((nq, nr));
                    queue.push_back((nq, nr));
                }
            }
        }
        for c in &region {
            used.insert(*c);
        }
        if region.len() == k {
            regions.push(region);
        } else {
            discarded += 1;
        }
    }
    if (regions.len() as u32) < spec.states {
        return Err(ScaleError::InsufficientLand {
            needed_states: spec.states,
            built_states: regions.len() as u32,
            land_cells: land.len(),
        });
    }

    // プロヴィンス表(全世界の初回採番と同じID。表は使用分だけ=Partial)と、ステート配置。
    let mut entries: Vec<ProvinceEntry> = Vec::new();
    let mut states: Vec<StateEntry> = Vec::new();
    for (i, region) in regions.iter().enumerate() {
        let id = i as u32 + 1;
        let mut ids: Vec<u32> = region.iter().map(|c| id_of[c]).collect();
        ids.sort_unstable();
        for &(q, r) in region {
            entries.push(ProvinceEntry { id: id_of[&(q, r)], q, r });
        }
        states.push(StateEntry {
            id,
            name_key: format!("scale.state.{id}"),
            provinces: ids,
            representative: None,
            allow_disconnected: false,
        });
    }
    entries.sort_by_key(|p| p.id);
    let mut geography = compute_fingerprint(map);
    geography.algorithm = full.geography.algorithm;
    let table = ProvinceTableFile {
        format_version: WORLD_FORMAT_VERSION,
        coverage: Coverage::Partial,
        geography,
        next_province_id: full.next_province_id,
        retired: Vec::new(),
        provinces: entries,
    };
    let layout = StateLayoutFile { format_version: WORLD_FORMAT_VERSION, states };

    // 資源(プロヴィンス単位)と、ステートの経済入力。
    let mut resource_entries: Vec<ProvinceResourceEntry> = Vec::new();
    let mut deposits_by_state: HashMap<u32, usize> = HashMap::new();
    let (mut with_dep, mut iron, mut coal) = (0usize, 0usize, 0usize);
    for st in &layout.states {
        for &pid in &st.provinces {
            let mut rng = Rng::for_key(spec.seed, DOMAIN_PROVINCE, pid as u64);
            let mut deposits = Vec::new();
            if rng.chance(25) {
                let resource_type = if rng.chance(50) { ResourceType::Iron } else { ResourceType::Coal };
                deposits.push(StateResourceDeposit {
                    resource_type,
                    base_output: rng.range(10, 40) as f64,
                    discovered: rng.chance(80),
                    development_level: rng.range(1, 2) as u32,
                });
                with_dep += 1;
                *deposits_by_state.entry(st.id).or_default() += 1;
                match resource_type {
                    ResourceType::Iron => iron += 1,
                    _ => coal += 1,
                }
            }
            resource_entries.push(ProvinceResourceEntry { province: pid, deposits });
        }
    }
    resource_entries.sort_by_key(|e| e.province);
    let resources = ProvinceResourcesFile {
        format_version: PROVINCE_RESOURCES_FORMAT_VERSION,
        scope: ResourceScope::Partial,
        provinces: resource_entries,
    };

    let mut scen_states: Vec<MapStateEconomyInput> = Vec::new();
    let mut population_total = 0u64;
    let mut buildings_total: BTreeMap<String, u64> = BTreeMap::new();
    for st in &layout.states {
        let mut rng = Rng::for_key(spec.seed, DOMAIN_STATE, st.id as u64);
        let population = rng.range(150_000, 900_000);
        population_total += population;
        let mut buildings: HashMap<BuildingType, u32> = HashMap::new();
        buildings.insert(BuildingType::Farm, rng.range(1, 5) as u32);
        buildings.insert(BuildingType::Railway, 1);
        let factories = rng.range(0, 2) as u32;
        if factories > 0 {
            buildings.insert(BuildingType::Factory, factories);
        }
        let mines = rng.range(0, 2) as u32;
        if mines > 0 && deposits_by_state.contains_key(&st.id) {
            buildings.insert(BuildingType::Mine, mines);
        }
        for (b, n) in &buildings {
            *buildings_total.entry(format!("{b:?}")).or_default() += *n as u64;
        }
        scen_states.push(MapStateEconomyInput {
            state: st.id,
            population,
            workforce_ratio: rng.range(40, 60) as f32 / 100.0,
            living_standard: rng.range(40, 70) as f32,
            unrest: rng.range(5, 25) as f32,
            base_logistics_capacity: rng.range(150, 300) as f32,
            buildings,
            resource_deposits: Vec::new(), // 鉱床の正本はプロヴィンス別の資源設定
        });
    }

    // 国家: 並び順(ステートID順)に連続した塊で割り当てる(各国家に最低 states/countries 個。列挙順や乱数に依存しない)。
    let n = spec.states as u64;
    let c = spec.countries as u64;
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); spec.countries as usize];
    for (i, st) in layout.states.iter().enumerate() {
        members[((i as u64 * c) / n) as usize].push(st.id);
    }
    let mut countries = Vec::new();
    let mut treasury_total = 0.0;
    for (ci, ms) in members.iter().enumerate() {
        let label = ci as u32 + 1;
        let count = ms.len() as f64;
        let treasury = 3000.0 + 1500.0 * count;
        treasury_total += treasury;
        let amount = 1000.0 + 300.0 * count;
        let stockpile: HashMap<ResourceType, f64> = [ResourceType::Iron, ResourceType::Coal, ResourceType::Food, ResourceType::Wood]
            .into_iter()
            .map(|r| (r, amount))
            .collect();
        countries.push(ScenarioCountryInput {
            label,
            treasury,
            tax_rate: 0.15,
            available_manpower: 0,
            mobilized_manpower: 0,
            stockpile,
            states: ms.clone(),
            ai_controlled: label != MANUAL_COUNTRY,
            military: None,
        });
    }
    let scenario = MultiMonthScenario {
        format_version: MULTI_MONTH_SCENARIO_FORMAT_VERSION,
        description: format!(
            "P32-024 synthetic scale scenario (states {}, countries {}, seed {}); not official data or balance",
            spec.states, spec.countries, spec.seed
        ),
        months: spec.days.div_ceil(30),
        countries,
        states: scen_states,
    };

    let summary = GenerationSummary {
        states: layout.states.len(),
        provinces: table.provinces.len(),
        countries: scenario.countries.len(),
        states_per_country_min: members.iter().map(Vec::len).min().unwrap_or(0),
        states_per_country_max: members.iter().map(Vec::len).max().unwrap_or(0),
        land_cells_available: land.len(),
        regions_discarded: discarded,
        population_total,
        buildings_total,
        provinces_with_deposits: with_dep,
        deposits_iron: iron,
        deposits_coal: coal,
        treasury_total,
        manual_country: MANUAL_COUNTRY,
        input_digest: canonical_digest(&table, &layout, &scenario, &resources),
    };
    Ok(SyntheticInput { table, layout, scenario, resources, summary })
}

/// 入力全体の正準な文字列(`HashMap` は並べ替えて書く)のBLAKE3(先頭16桁)。
fn canonical_digest(table: &ProvinceTableFile, layout: &StateLayoutFile, sc: &MultiMonthScenario, res: &ProvinceResourcesFile) -> String {
    let mut s = String::new();
    for p in &table.provinces {
        let _ = writeln!(s, "P {} {} {}", p.id, p.q, p.r);
    }
    for st in &layout.states {
        let _ = writeln!(s, "S {} {:?}", st.id, st.provinces);
    }
    for e in &res.provinces {
        let _ = writeln!(s, "R {} {:?}", e.province, e.deposits);
    }
    for st in &sc.states {
        let _ = write!(
            s,
            "E {} {} {:?} {:?} {:?} {:?}",
            st.state, st.population, st.workforce_ratio, st.living_standard, st.unrest, st.base_logistics_capacity
        );
        for b in BuildingType::ALL {
            if let Some(n) = st.buildings.get(&b) {
                let _ = write!(s, " {b:?}={n}");
            }
        }
        s.push('\n');
    }
    for c in &sc.countries {
        let mut stock: Vec<(String, f64)> = c.stockpile.iter().map(|(r, a)| (format!("{r:?}"), *a)).collect();
        stock.sort_by(|a, b| a.0.cmp(&b.0));
        let _ = writeln!(
            s,
            "C {} {:?} {:?} {} {} {:?} {:?} ai={}",
            c.label, c.treasury, c.tax_rate, c.available_manpower, c.mobilized_manpower, stock, c.states, c.ai_controlled
        );
    }
    let h = blake3::hash(s.as_bytes());
    h.to_hex().as_str()[..16].to_string()
}

/// 生成した入力から世界(`WorldModel`)を作り、地図との Authoring 検証を通す(`load_world_preview` と同じ検証)。
pub fn build_world(map: &HexMap, input: &SyntheticInput) -> Result<WorldModel, ScaleError> {
    let model = WorldModel::build(input.table.clone(), input.layout.clone()).map_err(|issues| {
        ScaleError::Data(issues.iter().take(12).map(|i| i.to_string()).collect::<Vec<_>>().join("\n"))
    })?;
    let report = validate_against_geography(&model, map, ValidationMode::Authoring);
    if !report.is_ok() {
        return Err(ScaleError::Data(
            report
                .issues
                .iter()
                .filter(|i| i.severity == crate::world::Severity::Error)
                .take(12)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    Ok(model)
}

/// 共有の組み立てでセッションの経済状態を作る。
pub fn assemble(input: &SyntheticInput, world: &WorldModel, buildings: &BuildingRegistry) -> Result<MapEconomySimulation, ScaleError> {
    build_simulation_from_data(&input.scenario, &input.resources, START, world, buildings)
        .map_err(|e| ScaleError::Data(e.to_string()))
}

// ---------------------------------------------------------------- 手動注文の台本

/// 手動国家の建設注文(台本)。同じ日付・同じ順序で、連続実行にも復元後の継続にも同じように適用される。
#[derive(Debug, Clone)]
pub struct Script {
    /// (処理済みの日数, ステート)。注文はこの日数が経過した後、次の日の処理の前に出す。
    orders: Vec<(u32, MapStateId, BuildingType)>,
}

impl Script {
    pub fn new(input: &SyntheticInput, enabled: bool) -> Script {
        let mut orders = Vec::new();
        if enabled
            && let Some(c) = input.scenario.countries.iter().find(|c| c.label == MANUAL_COUNTRY)
        {
            const CYCLE: [BuildingType; 4] = [BuildingType::Farm, BuildingType::Factory, BuildingType::Mine, BuildingType::Railway];
            for (di, &day) in SCRIPT_DAYS.iter().enumerate() {
                for (k, st) in c.states.iter().take(SCRIPT_STATES_PER_DAY).enumerate() {
                    orders.push((day, MapStateId(*st), CYCLE[(di + k) % CYCLE.len()]));
                }
            }
        }
        Script { orders }
    }

    pub fn len(&self) -> usize {
        self.orders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }
}

// ---------------------------------------------------------------- 結果の型

#[derive(Debug, Clone, PartialEq)]
pub enum CaseStatus {
    /// 測定と正しさの確認がすべて完了した。
    Success,
    /// 実行できたが、決定性・継続一致・活動の確認のいずれかが満たされなかった。
    CorrectnessFailure(Vec<String>),
    /// 時間上限に達した(未完了。決定性の不一致や完了済みの測定ではない)。
    TimedOut { phase: String },
    /// 入力の生成・検証・組み立て・保存復元の操作自体が実行できなかった。
    Unrunnable(String),
}

impl CaseStatus {
    pub fn label(&self) -> &'static str {
        match self {
            CaseStatus::Success => "成功",
            CaseStatus::CorrectnessFailure(_) => "正しさの失敗",
            CaseStatus::TimedOut { .. } => "時間超過(未完了)",
            CaseStatus::Unrunnable(_) => "実行不能",
        }
    }
    pub fn is_success(&self) -> bool {
        matches!(self, CaseStatus::Success)
    }
    /// 子プロセスの終了コード(0成功 / 2正しさの失敗 / 3時間超過 / 4実行不能)。
    pub fn exit_code(&self) -> i32 {
        match self {
            CaseStatus::Success => 0,
            CaseStatus::CorrectnessFailure(_) => 2,
            CaseStatus::TimedOut { .. } => 3,
            CaseStatus::Unrunnable(_) => 4,
        }
    }
}

/// 日次進行の累計(`advance_day` の呼び出しだけを計時する)。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunStats {
    pub days: u32,
    pub advance: Duration,
    pub monthly_days: u32,
    pub monthly_time: Duration,
    pub normal_days: u32,
    pub normal_time: Duration,
    pub max_day: Duration,
    /// 手動注文の実行時間(進行時間には含めない)。
    pub script_time: Duration,
    pub script_accepted: u32,
    pub script_rejected: u32,
    pub completed_orders: u64,
    pub blocked_orders: u64,
    pub ai_orders: u64,
    pub tax_changes: u64,
    pub evaluations: u64,
    pub end_date: Option<SimDate>,
}

/// 件数に関わる項目の組(複数回実行の比較用)。
type Counts = (u32, u32, u32, u32, u32, u64, u64, u64, u64, u64, Option<SimDate>);

impl RunStats {
    /// 件数に関わる(時間でない)項目。複数回実行の比較に使う。
    fn counts(&self) -> Counts {
        (
            self.days,
            self.monthly_days,
            self.normal_days,
            self.script_accepted,
            self.script_rejected,
            self.completed_orders,
            self.blocked_orders,
            self.ai_orders,
            self.tax_changes,
            self.evaluations,
            self.end_date,
        )
    }
    pub fn mean_day_ms(&self) -> f64 {
        ms(self.advance) / self.days.max(1) as f64
    }
    pub fn mean_monthly_ms(&self) -> Option<f64> {
        (self.monthly_days > 0).then(|| ms(self.monthly_time) / self.monthly_days as f64)
    }
    pub fn mean_normal_ms(&self) -> Option<f64> {
        (self.normal_days > 0).then(|| ms(self.normal_time) / self.normal_days as f64)
    }
}

pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// 保存再開の継続比較の記録。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Continuity {
    pub split_date: Option<SimDate>,
    pub split_next_day_is_month_change: bool,
    /// 比較した日数(後半の各日)。
    pub compared_days: u32,
    pub first_date: Option<SimDate>,
    pub last_date: Option<SimDate>,
    pub digest_mismatch_days: Vec<u32>,
    pub report_mismatch_days: Vec<u32>,
    pub restored_equals_state_at_save: bool,
    pub final_states_equal: bool,
    pub distinct_digests: usize,
    pub queued_orders_at_save: usize,
    pub monthly_days_in_compared: u32,
    pub save_ms: f64,
    pub restore_ms: f64,
    pub file_bytes: u64,
    /// 参照側(連続実行)のダイジェスト計算時間の合計(検証のための費用。進行時間とは別)。
    pub digest_ms: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaseResult {
    pub spec: ScaleSpec,
    pub status: CaseStatus,
    pub summary: Option<GenerationSummary>,
    pub generate_ms: f64,
    pub world_validate_ms: f64,
    pub session_build_ms: f64,
    /// 連続実行(同一条件)ごとの進行。
    pub runs: Vec<RunStats>,
    pub determinism_runs_equal: bool,
    pub final_digest: Option<String>,
    pub continuity: Option<Continuity>,
    pub total_ms: f64,
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------- 実行

pub(crate) struct Deadline {
    pub(crate) start: Instant,
    pub(crate) limit: Duration,
}

pub(crate) enum Abort {
    Timeout(String),
    Unrunnable(String),
}

impl Deadline {
    pub(crate) fn check(&self, phase: &str) -> Result<(), Abort> {
        if self.start.elapsed() > self.limit {
            Err(Abort::Timeout(phase.to_string()))
        } else {
            Ok(())
        }
    }
}

impl From<ScaleError> for Abort {
    fn from(e: ScaleError) -> Abort {
        Abort::Unrunnable(e.to_string())
    }
}

/// 状態全体(保存対象の動的状態の全体)のダイジェスト。保存と同じスナップショットのRON表現をBLAKE3にかける。
/// 並べ替えや丸めはしない。
pub fn state_digest(sim: &MapEconomySimulation, world: &WorldModel, buildings: &BuildingRegistry) -> Result<[u8; 32], String> {
    let snap = sim.snapshot(world, buildings);
    let text = ron::to_string(&snap).map_err(|e| e.to_string())?;
    Ok(*blake3::hash(text.as_bytes()).as_bytes())
}

fn hex8(d: &[u8; 32]) -> String {
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// `from..to` 日目を処理する。進行時間は `advance_day` の呼び出しだけを計る。`after_day(処理済みの日数, 状態, その日の出来事)` は
/// 計時の外で呼ぶ。
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_days<F>(
    sim: &mut MapEconomySimulation,
    buildings: &BuildingRegistry,
    script: &Script,
    from: u32,
    to: u32,
    deadline: &Deadline,
    phase: &str,
    stats: &mut RunStats,
    mut after_day: F,
) -> Result<(), Abort>
where
    F: FnMut(u32, &MapEconomySimulation, &DayReport) -> Result<(), Abort>,
{
    for done in from..to {
        deadline.check(phase)?;
        if !script.orders.is_empty() {
            let t = Instant::now();
            for (day, state, building) in &script.orders {
                if *day == done {
                    match sim.registry.request_build(&mut sim.countries, MANUAL_COUNTRY, *state, *building, buildings) {
                        Ok(_) => stats.script_accepted += 1,
                        Err(_) => stats.script_rejected += 1,
                    }
                }
            }
            stats.script_time += t.elapsed();
        }
        let t = Instant::now();
        let (date, report) = sim.advance_day(buildings);
        let el = t.elapsed();
        stats.days += 1;
        stats.advance += el;
        stats.max_day = stats.max_day.max(el);
        if report.monthly_ran {
            stats.monthly_days += 1;
            stats.monthly_time += el;
        } else {
            stats.normal_days += 1;
            stats.normal_time += el;
        }
        stats.completed_orders += report.completed.len() as u64;
        stats.blocked_orders += report.blocked.len() as u64;
        stats.ai_orders += report.ordered.len() as u64;
        stats.tax_changes += report.tax_changes.len() as u64;
        stats.evaluations += report.evaluations.len() as u64;
        stats.end_date = Some(date);
        after_day(done + 1, sim, &report)?;
    }
    Ok(())
}

/// 開始日から `days` 日後の日付(独立に数える。期待日数の確認用)。
pub fn date_after(days: u32) -> SimDate {
    let mut d = START;
    for _ in 0..days {
        d = d.next().0;
    }
    d
}

fn check_counts(spec: &ScaleSpec, input: &SyntheticInput, world: &WorldModel, sim: &MapEconomySimulation) -> Vec<String> {
    let mut v = Vec::new();
    let s = spec.states as usize;
    if input.summary.states != s || world.states().len() != s || sim.registry.len() != s {
        v.push(format!(
            "state count mismatch: spec {s}, generated {}, world {}, economy {}",
            input.summary.states,
            world.states().len(),
            sim.registry.len()
        ));
    }
    if sim.countries.len() != spec.countries as usize {
        v.push(format!("country count mismatch: spec {}, economy {}", spec.countries, sim.countries.len()));
    }
    if sim.countries.iter().any(|c| c.states.is_empty()) {
        v.push("a country has no state".into());
    }
    if sim.countries.iter().map(|c| c.states.len()).sum::<usize>() != s {
        v.push("sum of country states differs from the state count".into());
    }
    if sim.registry.iter().any(|st| st.population == 0) {
        v.push("a state has no population".into());
    }
    if sim.registry.iter().all(|st| st.buildings.is_empty()) {
        v.push("no state has any building".into());
    }
    v
}

fn make_temp_guard(base: &Path) -> Result<(), String> {
    if base.components().any(|c| c.as_os_str() == "saves") {
        return Err(format!("measurement save base must not be inside a 'saves' directory: {}", base.display()));
    }
    Ok(())
}

/// 1ケースを測定する。`save_base` は測定用保存の置き場所(一時ディレクトリ。`saves` を含む場所は拒否)。
/// 中の `map_economy/` に1ファイルを作り、終了時に自分が作ったファイルだけを消す。
pub fn run_case(map: &HexMap, buildings: &BuildingRegistry, spec: &ScaleSpec, save_base: &Path) -> CaseResult {
    let t0 = Instant::now();
    let mut result = CaseResult {
        spec: spec.clone(),
        status: CaseStatus::Success,
        summary: None,
        generate_ms: 0.0,
        world_validate_ms: 0.0,
        session_build_ms: 0.0,
        runs: Vec::new(),
        determinism_runs_equal: false,
        final_digest: None,
        continuity: None,
        total_ms: 0.0,
        notes: Vec::new(),
    };
    let deadline = Deadline { start: t0, limit: spec.time_limit };
    let outcome = run_case_inner(map, buildings, spec, save_base, &deadline, &mut result);
    result.status = match outcome {
        Ok(failures) if failures.is_empty() => CaseStatus::Success,
        Ok(failures) => CaseStatus::CorrectnessFailure(failures),
        Err(Abort::Timeout(phase)) => CaseStatus::TimedOut { phase },
        Err(Abort::Unrunnable(m)) => CaseStatus::Unrunnable(m),
    };
    // 測定用保存の後始末(自分が作った1ファイルだけ)。
    let loc = SaveLocation::new(save_base);
    if let Ok(p) = loc.file_path(SAVE_NAME) {
        let _ = std::fs::remove_file(p);
    }
    result.total_ms = ms(t0.elapsed());
    result
}

fn run_case_inner(
    map: &HexMap,
    buildings: &BuildingRegistry,
    spec: &ScaleSpec,
    save_base: &Path,
    deadline: &Deadline,
    result: &mut CaseResult,
) -> Result<Vec<String>, Abort> {
    spec.validate()?;
    make_temp_guard(save_base).map_err(Abort::Unrunnable)?;
    let mut failures: Vec<String> = Vec::new();

    // --- 入力生成 / 世界の検証 / セッション組み立て(それぞれ別計時)
    let t = Instant::now();
    let input = generate(spec, map)?;
    result.generate_ms = ms(t.elapsed());
    result.summary = Some(input.summary.clone());
    deadline.check("generate")?;
    let t = Instant::now();
    let world = build_world(map, &input)?;
    result.world_validate_ms = ms(t.elapsed());
    deadline.check("world validation")?;
    let t = Instant::now();
    let sim0 = assemble(&input, &world, buildings)?;
    result.session_build_ms = ms(t.elapsed());
    failures.extend(check_counts(spec, &input, &world, &sim0));
    let script = Script::new(&input, spec.scripted_orders);
    if spec.scripted_orders && script.is_empty() {
        failures.push("scripted orders were requested but the script is empty".into());
    }

    // --- A. 同一条件での連続実行(進行だけを計時。比較は終了後)
    let mut first_final: Option<MapEconomySimulation> = None;
    let mut all_equal = true;
    let mut sim = sim0;
    for run in 0..spec.runs {
        if run > 0 {
            sim = assemble(&input, &world, buildings)?;
        }
        let mut stats = RunStats::default();
        run_days(&mut sim, buildings, &script, 0, spec.days, deadline, "continuous run", &mut stats, |_, _, _| Ok(()))?;
        match &first_final {
            None => {
                result.final_digest = Some(hex8(&state_digest(&sim, &world, buildings).map_err(Abort::Unrunnable)?));
                first_final = Some(sim.clone());
            }
            Some(f) => {
                if *f != sim {
                    all_equal = false;
                    failures.push(format!("continuous run #{} ended in a different state from run #1", run + 1));
                }
            }
        }
        if let Some(first) = result.runs.first()
            && first.counts() != stats.counts()
        {
            all_equal = false;
            failures.push(format!("continuous run #{} reported different counts from run #1", run + 1));
        }
        if stats.days != spec.days || stats.end_date != Some(date_after(spec.days)) {
            failures.push(format!("run #{} processed {} days ending {:?} (expected {} days ending {:?})", run + 1, stats.days, stats.end_date, spec.days, date_after(spec.days)));
        }
        result.runs.push(stats);
        deadline.check("continuous run comparison")?;
    }
    result.determinism_runs_equal = all_equal;

    // --- B/C/D. 参照(連続)実行 → 保存 → 新しく組み立てたセッションへ復元 → 継続して比較
    let location = SaveLocation::new(save_base);
    let inputs = StaticInputs { map, world: &world, buildings, facilities: None };
    let mut cont = Continuity::default();
    let mut reference = assemble(&input, &world, buildings)?;
    let mut ref_stats = RunStats::default();
    run_days(&mut reference, buildings, &script, 0, spec.split_day, deadline, "reference run (first half)", &mut ref_stats, |_, _, _| Ok(()))?;
    cont.split_date = Some(reference.date);
    cont.split_next_day_is_month_change = reference.date.next().1;
    cont.queued_orders_at_save = reference.countries.iter().map(|c| c.construction_queue.len()).sum();
    let state_at_save = reference.clone();
    let t = Instant::now();
    let receipt = map_economy_file::save(&reference, &inputs, &location, SAVE_NAME)
        .map_err(|e| Abort::Unrunnable(format!("save failed: {e:?}")))?;
    cont.save_ms = ms(t.elapsed());
    cont.file_bytes = std::fs::metadata(&receipt.path).map(|m| m.len()).unwrap_or(0);
    deadline.check("save")?;

    // 後半の参照(連続)側: 各日のダイジェストと出来事を記録する(計時の外)。
    let mut ref_digests: Vec<[u8; 32]> = Vec::new();
    let mut ref_reports: Vec<DayReport> = Vec::new();
    let mut digest_time = Duration::ZERO;
    run_days(&mut reference, buildings, &script, spec.split_day, spec.days, deadline, "reference run (second half)", &mut ref_stats, |_, s, rep| {
        let t = Instant::now();
        let d = state_digest(s, &world, buildings).map_err(Abort::Unrunnable)?;
        digest_time += t.elapsed();
        ref_digests.push(d);
        ref_reports.push(rep.clone());
        Ok(())
    })?;
    cont.digest_ms = ms(digest_time);

    let mut resumed = assemble(&input, &world, buildings)?;
    let t = Instant::now();
    map_economy_file::load(&mut resumed, &inputs, &location, SAVE_NAME).map_err(|e| Abort::Unrunnable(format!("restore failed: {e:?}")))?;
    cont.restore_ms = ms(t.elapsed());
    deadline.check("restore")?;
    cont.restored_equals_state_at_save = resumed == state_at_save;
    if !cont.restored_equals_state_at_save {
        failures.push("the restored state differs from the state at the time of saving".into());
    }
    let mut res_digests: Vec<[u8; 32]> = Vec::new();
    let mut res_reports: Vec<DayReport> = Vec::new();
    let mut res_stats = RunStats::default();
    run_days(&mut resumed, buildings, &script, spec.split_day, spec.days, deadline, "resumed run", &mut res_stats, |_, s, rep| {
        let d = state_digest(s, &world, buildings).map_err(Abort::Unrunnable)?;
        res_digests.push(d);
        res_reports.push(rep.clone());
        Ok(())
    })?;
    let expected = spec.days - spec.split_day;
    cont.compared_days = ref_digests.len().min(res_digests.len()) as u32;
    cont.first_date = Some(date_after(spec.split_day + 1));
    cont.last_date = res_stats.end_date;
    if ref_digests.len() as u32 != expected || res_digests.len() as u32 != expected {
        failures.push(format!(
            "compared {} / {} days but {} were expected for the second half",
            ref_digests.len(),
            res_digests.len(),
            expected
        ));
    }
    for i in 0..cont.compared_days as usize {
        let day = spec.split_day + 1 + i as u32;
        if ref_digests[i] != res_digests[i] {
            cont.digest_mismatch_days.push(day);
        }
        if ref_reports[i] != res_reports[i] {
            cont.report_mismatch_days.push(day);
        }
    }
    cont.monthly_days_in_compared = ref_reports.iter().filter(|r| r.monthly_ran).count() as u32;
    cont.distinct_digests = ref_digests.iter().collect::<HashSet<_>>().len();
    cont.final_states_equal = resumed == reference;
    if !cont.digest_mismatch_days.is_empty() {
        failures.push(format!("state digests differ on {} days (first: day {})", cont.digest_mismatch_days.len(), cont.digest_mismatch_days[0]));
    }
    if !cont.report_mismatch_days.is_empty() {
        failures.push(format!("daily reports differ on {} days (first: day {})", cont.report_mismatch_days.len(), cont.report_mismatch_days[0]));
    }
    if !cont.final_states_equal {
        failures.push("the final state after resuming differs from the continuous run".into());
    }
    // 空・無変化を一致の根拠にしない。
    if cont.compared_days == 0 {
        failures.push("no day was compared".into());
    }
    if cont.distinct_digests < 2 {
        failures.push("the state never changed during the compared half (a match would prove nothing)".into());
    }
    if cont.monthly_days_in_compared == 0 {
        failures.push("no monthly processing happened in the compared half".into());
    }
    if cont.queued_orders_at_save == 0 {
        failures.push("no construction order was in progress at the time of saving".into());
    }
    if ref_stats.completed_orders + ref_stats.ai_orders + (ref_stats.script_accepted as u64) == 0 {
        failures.push("no construction activity happened in the run".into());
    }
    // 参照(連続)実行も、同一条件の連続実行と同じ結果になること(保存を挟んでも進行は変わらない)。
    if first_final.as_ref().is_some_and(|f| *f != reference) {
        failures.push("the reference run (with a save in the middle) ended in a different state from the plain continuous runs".into());
    }
    if result.runs.first().is_some_and(|f| f.counts() != ref_stats.counts()) {
        failures.push("the reference run reported different counts from the plain continuous runs".into());
    }
    result.runs.push(ref_stats);
    result.continuity = Some(cont);
    Ok(failures)
}

// ---------------------------------------------------------------- 出力

impl CaseResult {
    /// 人が読む詳細。
    pub fn render(&self) -> String {
        let mut s = String::new();
        let sp = &self.spec;
        let _ = writeln!(
            s,
            "== ケース: states={} countries={} provinces/state={} seed={} days={} split={} runs={} 上限={}s 手動注文台本={}",
            sp.states, sp.countries, sp.provinces_per_state, sp.seed, sp.days, sp.split_day, sp.runs, sp.time_limit.as_secs(), sp.scripted_orders
        );
        let _ = writeln!(s, "判定: {}", self.status.label());
        match &self.status {
            CaseStatus::CorrectnessFailure(v) => {
                for m in v {
                    let _ = writeln!(s, "  - {m}");
                }
            }
            CaseStatus::TimedOut { phase } => {
                let _ = writeln!(s, "  時間上限に達した工程: {phase}(未完了。以降の測定・比較は行っていない)");
            }
            CaseStatus::Unrunnable(m) => {
                let _ = writeln!(s, "  {m}");
            }
            CaseStatus::Success => {}
        }
        if let Some(g) = &self.summary {
            let _ = writeln!(
                s,
                "入力: ステート{} プロヴィンス{} 国家{}(1国家あたりのステート {}〜{}、手動国家={} 他はAI) 人口合計{} 鉱床あり{}プロヴィンス(Iron{} Coal{}) 国庫合計{:.0} 破棄した断片{} 利用可能な陸セル{} 入力ダイジェスト={}",
                g.states, g.provinces, g.countries, g.states_per_country_min, g.states_per_country_max, g.manual_country,
                g.population_total, g.provinces_with_deposits, g.deposits_iron, g.deposits_coal, g.treasury_total, g.regions_discarded, g.land_cells_available, g.input_digest
            );
            let _ = writeln!(s, "初期施設合計: {:?}", g.buildings_total);
        }
        let _ = writeln!(
            s,
            "時間(ms): 入力生成{:.1} / 世界の検証{:.1} / セッション組み立て{:.1}(各々日次進行には含めない)",
            self.generate_ms, self.world_validate_ms, self.session_build_ms
        );
        let n_runs = self.spec.runs as usize;
        for (i, r) in self.runs.iter().take(n_runs).enumerate() {
            let _ = writeln!(
                s,
                "連続実行#{}: {}日 進行合計{:.1}ms 1日平均{:.3}ms 月次処理日{}日(平均{}) 通常日{}日(平均{}) 最大1日{:.3}ms | 手動注文 受理{}/拒否{}(実行{:.1}msは進行に含めない) 完成{} 保留{} AI注文{} 税率変更{} AI評価{} 終了日{:?}",
                i + 1, r.days, ms(r.advance), r.mean_day_ms(), r.monthly_days,
                r.mean_monthly_ms().map_or("-".into(), |v| format!("{v:.3}ms")),
                r.normal_days, r.mean_normal_ms().map_or("-".into(), |v| format!("{v:.3}ms")),
                ms(r.max_day), r.script_accepted, r.script_rejected, ms(r.script_time),
                r.completed_orders, r.blocked_orders, r.ai_orders, r.tax_changes, r.evaluations, r.end_date
            );
        }
        if !self.runs.is_empty() {
            let _ = writeln!(
                s,
                "決定性: 連続{}回の終了状態(完全比較 ==)と件数が一致={} 終了状態ダイジェスト(先頭8バイト)={}",
                n_runs.min(self.runs.len()), self.determinism_runs_equal, self.final_digest.as_deref().unwrap_or("-")
            );
        }
        if let Some(c) = &self.continuity {
            let _ = writeln!(
                s,
                "保存再開: 保存日={:?}(翌日が月替わり={}) 保存時点の建設中注文{} 保存{:.1}ms 復元{:.1}ms ファイル{}バイト",
                c.split_date, c.split_next_day_is_month_change, c.queued_orders_at_save, c.save_ms, c.restore_ms, c.file_bytes
            );
            let _ = writeln!(
                s,
                "継続比較: 後半{}日を各日比較 {:?}〜{:?} ダイジェスト不一致{}日 出来事の不一致{}日 復元直後=保存時の状態:{} 最終状態の完全比較:{} 異なるダイジェスト数{} 月次処理日{}日 (ダイジェスト計算{:.1}msは検証の費用で進行時間とは別)",
                c.compared_days, c.first_date, c.last_date, c.digest_mismatch_days.len(), c.report_mismatch_days.len(),
                c.restored_equals_state_at_save, c.final_states_equal, c.distinct_digests, c.monthly_days_in_compared, c.digest_ms
            );
        }
        let _ = writeln!(s, "ケース全体の壁時計(生成・比較・保存復元を含む): {:.1}ms", self.total_ms);
        for n in &self.notes {
            let _ = writeln!(s, "注記: {n}");
        }
        s
    }

    /// 駆動側が集計する1行(タブ区切り)。`ROW` で始まる。
    pub fn row(&self) -> String {
        let r0 = self.runs.first();
        let c = self.continuity.as_ref();
        let f = |v: Option<f64>| v.map_or("-".to_string(), |x| format!("{x:.3}"));
        let total = format!("{:.1}", self.total_ms);
        format!(
            "ROW\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.spec.states,
            self.spec.countries,
            self.status.label(),
            f(r0.map(|r| ms(r.advance))),
            f(r0.map(RunStats::mean_day_ms)),
            f(r0.and_then(RunStats::mean_monthly_ms)),
            f(r0.and_then(RunStats::mean_normal_ms)),
            f(c.map(|c| c.save_ms)),
            f(c.map(|c| c.restore_ms)),
            c.map_or("-".to_string(), |c| c.file_bytes.to_string()),
            r0.map_or("-".to_string(), |r| r.days.to_string()),
            c.map_or("-".to_string(), |c| c.compared_days.to_string()),
            self.determinism_runs_equal,
            total,
        )
    }
}

// ---------------------------------------------------------------- コマンド行

/// 実行時の入力の場所。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub map: String,
    pub buildings: String,
}

impl Default for Paths {
    fn default() -> Self {
        Paths { map: "assets/data/hex_atlas_map.json".into(), buildings: "assets/data/buildings.ron".into() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Help,
    /// 1ケースを実行する(子プロセスまたは単独実行)。
    Case { spec: ScaleSpec, paths: Paths },
    /// 主系列(と、指定すれば別条件のケース)を、ケースごとに別プロセスで小さい順に逐次実行する。
    Series { sizes: Vec<u32>, wide: bool, template: ScaleSpec, paths: Paths },
}

pub const USAGE: &str = "\
economy_scale_check — 新ステート経済の規模測定ハーネス(P32-024。合成シナリオ。release ビルドで測ること)

  cargo run --release --bin economy_scale_check -- --states N [--countries C] [オプション]   # 1ケース
  cargo run --release --bin economy_scale_check -- --series [--sizes 3,30,100,300,1000] [--wide] [オプション]

オプション(既定値):
  --states N                  ステート数(必須。単独ケース)
  --countries C               国家数(3)
  --provinces-per-state K     1ステートのプロヴィンス数(6)
  --seed S                    合成入力の seed(24)
  --days D                    処理する日数(360)
  --split D                   保存して再開する日(180)
  --runs R                    同一条件での連続実行の回数(3)
  --time-limit-secs T         1ケース全体の上限秒(120)
  --no-script                 手動国家の建設注文の台本を使わない
  --map PATH                  地理JSON(assets/data/hex_atlas_map.json)
  --buildings PATH            建物定義(assets/data/buildings.ron)
  --series                    主系列を小さい順に、ケースごとに別プロセスで逐次実行
  --sizes a,b,c               主系列のステート数(3,30,100,300,1000。国家数は --countries、既定3)
  --wide                      別条件: 1000ステート・100国家のケースを主系列の後に追加
  --help
測定用の保存は OS の一時ディレクトリにだけ作り(saves/ には書かない)、終了時に自分のファイルを消します。
終了コード: 0成功 / 2正しさの失敗 / 3時間超過 / 4実行不能 / 64引数エラー";

fn parse_u64(name: &str, v: Option<&String>) -> Result<u64, String> {
    let v = v.ok_or_else(|| format!("{name} needs a value"))?;
    v.parse::<u64>().map_err(|_| format!("{name} needs a non-negative integer (got \"{v}\")"))
}

fn parse_u32(name: &str, v: Option<&String>) -> Result<u32, String> {
    let x = parse_u64(name, v)?;
    u32::try_from(x).map_err(|_| format!("{name} is too large (got {x})"))
}

/// 引数を解析する。不明な引数・欠けた値・範囲外は黙って補わず拒否する。
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut spec = ScaleSpec::new(0, MAIN_SERIES_COUNTRIES);
    let mut paths = Paths::default();
    let (mut series, mut wide, mut have_states) = (false, false, false);
    let mut sizes: Vec<u32> = MAIN_SERIES.to_vec();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let v = args.get(i + 1);
        let mut takes_value = true;
        match a {
            "--help" | "-h" => return Ok(Command::Help),
            "--states" => {
                spec.states = parse_u32(a, v)?;
                have_states = true;
            }
            "--countries" => spec.countries = parse_u32(a, v)?,
            "--provinces-per-state" => spec.provinces_per_state = parse_u32(a, v)?,
            "--seed" => spec.seed = parse_u64(a, v)?,
            "--days" => spec.days = parse_u32(a, v)?,
            "--split" => spec.split_day = parse_u32(a, v)?,
            "--runs" => spec.runs = parse_u32(a, v)?,
            "--time-limit-secs" => spec.time_limit = Duration::from_secs(parse_u64(a, v)?),
            "--map" => paths.map = v.ok_or("--map needs a value")?.clone(),
            "--buildings" => paths.buildings = v.ok_or("--buildings needs a value")?.clone(),
            "--sizes" => {
                let text = v.ok_or("--sizes needs a value")?;
                sizes = text
                    .split(',')
                    .map(|p| p.trim().parse::<u32>().map_err(|_| format!("--sizes needs comma-separated integers (got \"{text}\")")))
                    .collect::<Result<_, _>>()?;
                if sizes.is_empty() {
                    return Err("--sizes must not be empty".into());
                }
            }
            "--no-script" => {
                spec.scripted_orders = false;
                takes_value = false;
            }
            "--series" => {
                series = true;
                takes_value = false;
            }
            "--wide" => {
                wide = true;
                takes_value = false;
            }
            other => return Err(format!("unknown argument \"{other}\" (see --help)")),
        }
        i += if takes_value { 2 } else { 1 };
    }
    if series {
        if have_states {
            return Err("--series cannot be combined with --states (use --sizes)".into());
        }
        // 各ケースの指定を先に検証する(途中で不正に気づいて半端に走らせない)。
        for &n in &sizes {
            let mut s = spec.clone();
            s.states = n;
            s.validate().map_err(|e| e.to_string())?;
        }
        if wide {
            let mut s = spec.clone();
            s.states = WIDE_CASE.0;
            s.countries = WIDE_CASE.1;
            s.validate().map_err(|e| e.to_string())?;
        }
        return Ok(Command::Series { sizes, wide, template: spec, paths });
    }
    if wide {
        return Err("--wide is only for --series".into());
    }
    if !have_states {
        return Err("--states N (or --series) is required (see --help)".into());
    }
    spec.validate().map_err(|e| e.to_string())?;
    Ok(Command::Case { spec, paths })
}

/// 測定用保存の置き場所(OSの一時ディレクトリ配下の、プロセスごとに一意なディレクトリ)。
pub fn fresh_temp_base(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("p32_024_{tag}_{}_{n}", std::process::id()))
}
