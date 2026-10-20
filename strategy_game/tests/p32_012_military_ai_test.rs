//! P32-012 (M4-D): 軍事状況を参照する経済AI(優先度2.5 戦時MilitaryFactory・2.8 Iron供給・2.9 再建)と、
//! 履歴に基づく判断(Iron不足の履歴・Factory飢餓による増設抑止・軍需生産の履歴)を新ステート経路へ
//! 接続した結果のテスト。旧経路は**実際の旧System**(`handle_daily_construction`・
//! `handle_monthly_population_growth`・`handle_monthly_economy`。履歴は`CountryAiRegistry`へ記録される)と、
//! 旧AIの公開関数(`process_tax_rate_ai`・`process_construction_ai_with_contexts`・
//! `military_rebuild_context`・`iron_supply_context`・`factory_expansion_blocked`)で駆動する。
//! 入力は検証用シナリオで、正式な人口・施設・資源・国家配置・バランスではない。軍事状況は**明示した
//! fixture**(新マップ上の軍隊・戦争・前線のシミュレーションではない)。
//!
//! 旧`StateId(n+1)`とシナリオ順の`MapStateId`の対応、旧`CountryId(label)`との対応(評価周期と履歴の
//! 国家識別)は、テスト内で明示する(数値一致で他の用途へ推測しない)。

use bevy::prelude::*;
use std::collections::HashMap;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, DivisionId, StateId};
use strategy_game::country::ai_military::{MilitaryAiFacts, MilitaryPriority};
use strategy_game::country::country_ai::{
    CountryAiRegistry, CountryAiState, factory_expansion_blocked, iron_supply_context,
    military_ai_facts, military_rebuild_context, process_construction_ai_with_contexts,
    process_tax_rate_ai,
};
use strategy_game::country::{CountryData, CountryRegistry, PlayerCountry};
use strategy_game::economy::map_state::{
    AiEvaluation, DayReport, MapStateEconomyInput, MapStateEconomyRegistry, MultiMonthScenario,
    ScenarioCountry, ScenarioCountryInput, SimDate,
};
use strategy_game::economy::province_resources::{
    ProvinceResourceEntry, ProvinceResourcesFile, derive_state_resources, parse_province_resources,
};
use strategy_game::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use strategy_game::localization::{CurrentLocale, Locale, TranslationCatalog};
use strategy_game::military::data::{
    Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
    STANDARD_INFANTRY_DEFINITION_ID,
};
use strategy_game::military::recruitment::RecruitmentQueueItem;
use strategy_game::population::growth::handle_monthly_population_growth;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::ui::notification::GameNotification;
use strategy_game::world::{MapStateId, WorldModel, load_province_table, load_state_layout};

const RES_PATH: &str = "assets/data/world/preview/province_resources.ron";
const START: SimDate = SimDate { year: 1800, month: 1, day: 1 };

fn buildings() -> BuildingRegistry {
    let defs: Vec<BuildingDefinition> =
        ron::from_str(&std::fs::read_to_string("assets/data/buildings.ron").unwrap()).unwrap();
    BuildingRegistry {
        definitions: defs.into_iter().map(|d| (d.building_type, d)).collect(),
    }
}

fn world() -> WorldModel {
    WorldModel::build(
        load_province_table("assets/data/world/preview/provinces.ron").unwrap(),
        load_state_layout("assets/data/world/preview/states.ron").unwrap(),
    )
    .unwrap()
}

fn s(id: u32) -> MapStateId {
    MapStateId(id)
}

fn dep(r: ResourceType, out: f64, discovered: bool) -> StateResourceDeposit {
    StateResourceDeposit {
        resource_type: r,
        base_output: out,
        discovered,
        development_level: 1,
    }
}

/// 全プロヴィンスを設定した資源定義(未設定は「資源なし」の明示で埋める)。`extra`は上書きする鉱床。
fn resources(w: &WorldModel, extra: &[(u32, Vec<StateResourceDeposit>)]) -> ProvinceResourcesFile {
    let mut f = parse_province_resources(&std::fs::read_to_string(RES_PATH).unwrap()).unwrap();
    for p in w.provinces() {
        if !f.provinces.iter().any(|e| e.province == p.id.0) {
            f.provinces.push(ProvinceResourceEntry { province: p.id.0, deposits: vec![] });
        }
    }
    for (province, deposits) in extra {
        let e = f.provinces.iter_mut().find(|e| e.province == *province).unwrap();
        e.deposits = deposits.clone();
    }
    f
}

// ---------------------------------------------------------------- 軍事状況のfixture(旧・新で同じ出典)

fn infantry_def() -> DivisionDefinition {
    DivisionDefinition {
        id: STANDARD_INFANTRY_DEFINITION_ID,
        name: "Standard Infantry".to_string(),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        required_manpower: 10_000,
        required_equipment: 1000.0,
        recruitment_days: 30,
        movement_speed: 1.0,
        attack: 10.0,
        defense: 10.0,
        breakthrough: 5.0,
        organization: 50.0,
        morale: 50.0,
        supply_usage: 1.0,
        maintenance_cost: 5.0,
    }
}

fn division(owner: CountryId, manpower: u64) -> Division {
    Division {
        id: DivisionId(0),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(1),
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower,
        max_manpower: 10_000,
        equipment: 1000.0,
        max_equipment: 1000.0,
        organization: 50.0,
        max_organization: 50.0,
        morale: 50.0,
        max_morale: 50.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: STANDARD_INFANTRY_DEFINITION_ID,
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

/// 軍事状況の明示fixture: `living`個の標準歩兵師団、募兵キュー`queue`件、戦争中か。
#[derive(Debug, Clone, Copy)]
struct MilitaryFixture {
    at_war: bool,
    living: usize,
    queue: usize,
}

fn military_registry(label: u32, fx: &MilitaryFixture) -> MilitaryRegistry {
    let mut m = MilitaryRegistry::default();
    m.definitions.insert(STANDARD_INFANTRY_DEFINITION_ID, infantry_def());
    for _ in 0..fx.living {
        m.add_division(division(CountryId(label as usize), 10_000));
    }
    m
}

fn recruitment_queue(n: usize) -> Vec<RecruitmentQueueItem> {
    (0..n)
        .map(|_| RecruitmentQueueItem {
            division_id: STANDARD_INFANTRY_DEFINITION_ID,
            target_state: StateId(1),
            days_remaining: 10,
            total_days: 30,
        })
        .collect()
}

/// 新経路へ渡す軍事状況: **旧経路の実際の関数**(`military_ai_facts`)に同じfixtureを通した結果。
fn facts_of(label: u32, fx: &MilitaryFixture) -> MilitaryAiFacts {
    let c = CountryData {
        id: CountryId(label as usize),
        recruitment_queue: recruitment_queue(fx.queue),
        ..CountryData::default()
    };
    military_ai_facts(&c, fx.at_war, &military_registry(label, fx))
}

// ---------------------------------------------------------------- シナリオの組み立て

fn country_input(
    label: u32,
    treasury: f64,
    tax: f32,
    stock: &[(ResourceType, f64)],
    states: &[u32],
    fx: Option<MilitaryFixture>,
) -> ScenarioCountryInput {
    ScenarioCountryInput {
        label,
        treasury,
        tax_rate: tax,
        available_manpower: 0,
        mobilized_manpower: 0,
        stockpile: stock.iter().copied().collect(),
        states: states.to_vec(),
        ai_controlled: true,
        military: fx.map(|f| facts_of(label, &f)),
    }
}

fn state_input(
    id: u32,
    population: u64,
    buildings: &[(BuildingType, u32)],
) -> MapStateEconomyInput {
    MapStateEconomyInput {
        state: id,
        population,
        workforce_ratio: 0.5,
        living_standard: 60.0,
        unrest: 10.0,
        base_logistics_capacity: 200.0,
        buildings: buildings.iter().copied().collect(),
        resource_deposits: vec![],
    }
}

fn scenario(
    months: u32,
    countries: Vec<ScenarioCountryInput>,
    states: Vec<MapStateEconomyInput>,
) -> MultiMonthScenario {
    MultiMonthScenario {
        format_version: 1,
        description: "P32-012 test scenario (not official data or balance)".to_string(),
        months,
        countries,
        states,
    }
}

// ---------------------------------------------------------------- 旧経路(実際の旧System+旧AIの公開関数)

struct OldWorld {
    app: App,
    ai: HashMap<u32, CountryAiState>,
    labels: Vec<u32>,
    fixtures: HashMap<u32, MilitaryFixture>,
}

fn old_world(
    sc: &MultiMonthScenario,
    b: &BuildingRegistry,
    f: &ProvinceResourcesFile,
    w: &WorldModel,
    fixtures: &HashMap<u32, MilitaryFixture>,
) -> OldWorld {
    let d = derive_state_resources(w, f).unwrap();
    // 対応表(明示): 旧StateId(n+1) ↔ シナリオ内n番目のステート。旧CountryId(label)は評価周期と履歴の国家識別。
    let owner_of: HashMap<u32, u32> = sc
        .countries
        .iter()
        .flat_map(|c| c.states.iter().map(move |st| (*st, c.label)))
        .collect();
    let states: Vec<StateData> = sc
        .states
        .iter()
        .enumerate()
        .map(|(n, i)| StateData {
            id: StateId(n + 1),
            owner_country_id: CountryId(owner_of[&i.state] as usize),
            population: i.population,
            workforce_ratio: i.workforce_ratio,
            living_standard: i.living_standard,
            unrest: i.unrest,
            base_logistics_capacity: i.base_logistics_capacity,
            logistics_capacity: i.base_logistics_capacity,
            buildings: i.buildings.clone(),
            resource_deposits: d.state(MapStateId(i.state)).unwrap().deposits.clone(),
            ..StateData::default()
        })
        .collect();
    let countries: Vec<CountryData> = sc
        .countries
        .iter()
        .map(|c| {
            let mut stockpile = CountryStockpile::new();
            for (r, a) in &c.stockpile {
                stockpile.set(*r, *a);
            }
            let q = fixtures.get(&c.label).map(|f| f.queue).unwrap_or(0);
            CountryData {
                id: CountryId(c.label as usize),
                treasury: c.treasury,
                tax_rate: c.tax_rate,
                available_manpower: c.available_manpower,
                mobilized_manpower: c.mobilized_manpower,
                stockpile,
                recruitment_queue: recruitment_queue(q),
                ..CountryData::default()
            }
        })
        .collect();
    let mut labels: Vec<u32> = sc.countries.iter().filter(|c| c.ai_controlled).map(|c| c.label).collect();
    labels.sort();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<DayChangedMessage>()
        .add_message::<MonthChangedMessage>()
        .add_message::<GameNotification>()
        .insert_resource(CountryRegistry { countries })
        .insert_resource(StateRegistry::build(states))
        .insert_resource(BuildingRegistry { definitions: b.definitions.clone() })
        .insert_resource(PlayerCountry(None))
        .insert_resource(CountryAiRegistry::default())
        .insert_resource(CurrentLocale(Locale::JaJp))
        .insert_resource(TranslationCatalog::load().expect("catalog"))
        .add_systems(
            Update,
            (
                strategy_game::economy::handle_daily_construction,
                strategy_game::economy::handle_monthly_economy,
            )
                .chain(),
        )
        .add_systems(
            Update,
            handle_monthly_population_growth.before(strategy_game::economy::handle_monthly_economy),
        );
    app.update();
    let ai = labels.iter().map(|l| (*l, CountryAiState::new(CountryId(*l as usize)))).collect();
    OldWorld { app, ai, labels, fixtures: fixtures.clone() }
}

impl OldWorld {
    /// 旧スケジュールの1日: Economy Set(建設進捗 → 月次経済+履歴の記録) → CountryAi Set(月次評価)。
    fn day(&mut self, date: SimDate, month_changed: bool, b: &BuildingRegistry) {
        self.app.world_mut().write_message(DayChangedMessage { year: date.year, month: date.month, day: date.day });
        if month_changed {
            self.app.world_mut().write_message(MonthChangedMessage { year: date.year, month: date.month });
        }
        self.app.update();
        let date_str = format!("{}/{:02}/{:02}", date.year, date.month, date.day);
        let day_number = date.day_number();
        for label in self.labels.clone() {
            let cid = CountryId(label as usize);
            let state = self.ai.get_mut(&label).unwrap();
            let due = (day_number as usize % 30 == label as usize % 30) || state.dirty;
            if due {
                let fx = self.fixtures.get(&label).copied();
                let mr = fx.map(|f| military_registry(label, &f));
                let history = self
                    .app
                    .world()
                    .resource::<CountryAiRegistry>()
                    .clone();
                self.app.world_mut().resource_scope::<CountryRegistry, _>(|world, mut cr| {
                    let sr = world.resource::<StateRegistry>();
                    process_tax_rate_ai(cid, &mut cr, sr);
                    let (wartime, rebuild) = match (&fx, &mr) {
                        (Some(f), Some(mr)) => {
                            let facts = military_ai_facts(cr.get(cid).unwrap(), f.at_war, mr);
                            let mut country = cr.get(cid).unwrap().clone();
                            country.recruitment_queue = recruitment_queue(f.queue);
                            (
                                strategy_game::country::ai_military::wartime_supply_context_from(&facts),
                                military_rebuild_context(
                                    &country,
                                    sr,
                                    mr,
                                    &history.supply_production_history.get(&cid).cloned().unwrap_or_default(),
                                    &date_str,
                                ),
                            )
                        }
                        _ => (None, None),
                    };
                    let iron = iron_supply_context(
                        &history.iron_shortage_history.get(&cid).cloned().unwrap_or_default(),
                        &date_str,
                        cid,
                        sr,
                    );
                    let blocked = factory_expansion_blocked(
                        &history.factory_starvation_history.get(&cid).cloned().unwrap_or_default(),
                        &date_str,
                        cid,
                        sr,
                    );
                    process_construction_ai_with_contexts(
                        cid, &mut cr, sr, b, state, wartime, iron, blocked, rebuild,
                    );
                });
            }
            state.dirty = false;
        }
    }

    fn snap(&self, sc: &MultiMonthScenario) -> DaySnap {
        let sr = self.app.world().resource::<StateRegistry>();
        let cr = self.app.world().resource::<CountryRegistry>();
        let ar = self.app.world().resource::<CountryAiRegistry>();
        DaySnap {
            states: sr
                .states
                .iter()
                .map(|st| StateSnap {
                    population: st.population,
                    employed: st.employed_workforce,
                    unemployed: st.unemployed_workforce,
                    logistics: (st.logistics_capacity, st.logistics_usage, st.logistics_ratio),
                    living_standard: st.living_standard,
                    unrest: st.unrest,
                    buildings: sorted_b(&st.buildings),
                    ops: sorted_f(&st.building_operation_ratios),
                })
                .collect(),
            countries: sc
                .countries
                .iter()
                .map(|sc_c| {
                    let c = cr.countries.iter().find(|c| c.id.0 == sc_c.label as usize).unwrap();
                    CountrySnap {
                        treasury: c.treasury,
                        tax_rate: c.tax_rate,
                        income: c.monthly_income,
                        expenses: c.monthly_expenses,
                        balance: c.monthly_balance,
                        stockpile: stock_vec(&c.stockpile),
                        economic_state: format!("{:?}", c.economic_state),
                        queue: c
                            .construction_queue
                            .iter()
                            .map(|o| (o.state_id.0 - 1, o.building_type, o.target_level, o.progress, o.paid_cost))
                            .collect(),
                        history: if sc_c.ai_controlled {
                            let id = CountryId(sc_c.label as usize);
                            Some((
                                format!("{:?}", ar.supply_production_history.get(&id)),
                                format!("{:?}", ar.iron_shortage_history.get(&id)),
                                mask_signature(&format!("{:?}", ar.factory_starvation_history.get(&id))),
                            ))
                        } else {
                            None
                        },
                    }
                })
                .collect(),
        }
    }
}

// ---------------------------------------------------------------- スナップショット

#[derive(Debug, Clone, PartialEq)]
struct StateSnap {
    population: u64,
    employed: u64,
    unemployed: u64,
    logistics: (f32, f32, f32),
    living_standard: f32,
    unrest: f32,
    buildings: Vec<(BuildingType, u32)>,
    ops: Vec<(BuildingType, f32)>,
}

#[derive(Debug, Clone, PartialEq)]
struct CountrySnap {
    treasury: f64,
    tax_rate: f32,
    income: f64,
    expenses: f64,
    balance: f64,
    stockpile: Vec<(ResourceType, f64)>,
    economic_state: String,
    /// (対象ステートのシナリオ内の添字, 施設, 目標レベル, 進捗, 支払済み費用)
    queue: Vec<(usize, BuildingType, u32, f64, f64)>,
    /// 履歴3種(軍需生産・Iron未充足・Factory飢餓)の内容。AI国家のみ。
    history: Option<(String, String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
struct DaySnap {
    states: Vec<StateSnap>,
    countries: Vec<CountrySnap>,
}

fn sorted_b(m: &HashMap<BuildingType, u32>) -> Vec<(BuildingType, u32)> {
    BuildingType::ALL.iter().filter_map(|b| m.get(b).map(|l| (*b, *l))).collect()
}
fn sorted_f(m: &HashMap<BuildingType, f32>) -> Vec<(BuildingType, f32)> {
    BuildingType::ALL.iter().filter_map(|b| m.get(b).map(|l| (*b, *l))).collect()
}
fn stock_vec(st: &CountryStockpile) -> Vec<(ResourceType, f64)> {
    ResourceType::ALL.iter().map(|r| (*r, st.get(*r))).collect()
}

fn snap_new(reg: &MapStateEconomyRegistry, cs: &[ScenarioCountry]) -> DaySnap {
    let ids: Vec<MapStateId> = reg.iter().map(|x| x.state).collect();
    DaySnap {
        states: reg
            .iter()
            .map(|st| {
                use strategy_game::economy::state_access::StateEconomyAccess as _;
                StateSnap {
                    population: st.population,
                    employed: st.employed_workforce,
                    unemployed: st.unemployed_workforce,
                    logistics: (st.logistics_capacity, st.logistics_usage, st.logistics_ratio),
                    living_standard: st.living_standard,
                    unrest: st.unrest,
                    buildings: sorted_b(&st.buildings),
                    ops: sorted_f(st.operation_ratios()),
                }
            })
            .collect(),
        countries: cs
            .iter()
            .map(|c| CountrySnap {
                treasury: c.treasury,
                tax_rate: c.tax_rate,
                income: c.monthly_income,
                expenses: c.monthly_expenses,
                balance: c.monthly_balance,
                stockpile: stock_vec(&c.stockpile),
                economic_state: format!("{:?}", c.economic_state),
                queue: c
                    .construction_queue
                    .iter()
                    .map(|o| {
                        (
                            ids.iter().position(|i| *i == o.state).unwrap(),
                            o.building_type,
                            o.target_level,
                            o.progress,
                            o.paid_cost,
                        )
                    })
                    .collect(),
                history: c.ai.as_ref().map(|a| {
                    let none_if_empty = |dbg: String, empty: bool| if empty { "None".to_string() } else { format!("Some({dbg})") };
                    (
                        none_if_empty(format!("{:?}", a.history.supply), a.history.supply.is_empty()),
                        none_if_empty(format!("{:?}", a.history.iron), a.history.iron.is_empty()),
                        none_if_empty(mask_signature(&format!("{:?}", a.history.factory)), a.history.factory.is_empty()),
                    )
                }),
            })
            .collect(),
    }
}

/// 同じ初期条件・同じ日付の流れで旧新を1日ずつ実行し、毎日の全状態と履歴が一致することを確認する。
fn compare(
    sc: &MultiMonthScenario,
    f: &ProvinceResourcesFile,
    fixtures: &HashMap<u32, MilitaryFixture>,
    days: u32,
) -> Vec<(SimDate, DayReport)> {
    let (b, w) = (buildings(), world());
    let mut old = old_world(sc, &b, f, &w, fixtures);
    let (mut reg, mut cs) = MapStateEconomyRegistry::from_multi_scenario(sc, &w, &b).expect("scenario");
    let d = derive_state_resources(&w, f).unwrap();
    reg.attach_province_resources(&d, &w, f).unwrap();
    let mut date = START;
    let mut reports = Vec::new();
    for day in 1..=days {
        let (n, changed) = date.next();
        date = n;
        old.day(date, changed, &b);
        let rep = reg.run_day(&mut cs, date, changed, &b);
        let (o, n) = (old.snap(sc), snap_new(&reg, &cs));
        assert_eq!(o.states, n.states, "states differ on day {day} ({date:?})");
        for (a, b2) in o.countries.iter().zip(n.countries.iter()) {
            assert_eq!(a.queue, b2.queue, "queue differs on day {day} ({date:?})");
            assert_eq!(a.history, b2.history, "history differs on day {day} ({date:?})");
            assert_eq!(a, b2, "country differs on day {day} ({date:?})");
        }
        reports.push((date, rep));
    }
    reports
}

fn fired(reports: &[(SimDate, DayReport)]) -> Vec<(SimDate, u32, MilitaryPriority)> {
    reports
        .iter()
        .flat_map(|(d, r)| r.evaluations.iter().filter_map(move |e| e.fired.map(|p| (*d, e.label, p))))
        .collect()
}

fn evals(reports: &[(SimDate, DayReport)]) -> Vec<(SimDate, &AiEvaluation)> {
    reports.iter().flat_map(|(d, r)| r.evaluations.iter().map(move |e| (*d, e))).collect()
}

/// 設備署名の数値は、旧`StateId`と`MapStateId`という別の識別空間のキーから作られるため、新旧で値が
/// 異なる(同じ国家の履歴の中での一致比較にだけ使われる値で、意味は同じ)。比較では数値を伏せる。
fn mask_signature(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("signature: ") {
        out.push_str(&rest[..i]);
        out.push_str("signature: _");
        let after = &rest[i + "signature: ".len()..];
        let digits = after.chars().take_while(|c| c.is_ascii_digit()).count();
        rest = &after[digits..];
    }
    out.push_str(rest);
    out
}

// ================================================================ テスト

use BuildingType::{Factory, Farm, Mine, MilitaryFactory, Railway};
use ResourceType::{Coal, Food, IndustrialGoods, Iron, Wood};

const M4D_SCENARIO: &str = "assets/data/world/preview/economy_scenario_m4d.ron";
const M4D_RESOURCES: &str = "assets/data/world/preview/province_resources_m4d.ron";

fn m4d() -> (MultiMonthScenario, ProvinceResourcesFile, HashMap<u32, MilitaryFixture>) {
    use strategy_game::economy::map_state::parse_multi_month_scenario;
    let sc = parse_multi_month_scenario(&std::fs::read_to_string(M4D_SCENARIO).unwrap()).unwrap();
    let f = parse_province_resources(&std::fs::read_to_string(M4D_RESOURCES).unwrap()).unwrap();
    // 軍事状況のfixture(出典): RONの入力が、同じfixtureに旧経路の`military_ai_facts`を通した結果と一致することも検証する。
    let fixtures = [
        (1, MilitaryFixture { at_war: true, living: 10, queue: 0 }),
        (2, MilitaryFixture { at_war: false, living: 5, queue: 0 }),
        (3, MilitaryFixture { at_war: false, living: 0, queue: 0 }),
    ]
    .into_iter()
    .collect();
    (sc, f, fixtures)
}

fn date(y: i32, m: u8, d: u8) -> SimDate {
    SimDate { year: y, month: m, day: d }
}

#[test]
fn scenario_military_inputs_match_the_old_path_function_output() {
    let (sc, _, fixtures) = m4d();
    for c in &sc.countries {
        let fx = fixtures[&c.label];
        assert_eq!(c.military, Some(facts_of(c.label, &fx)), "country {}", c.label);
    }
    // 手計算: 標準歩兵10個の日次需要 = 10 × 1,000,000 units/日(1個あたり)
    assert_eq!(sc.countries[0].military.unwrap().daily_demand_units, Some(10_000_000));
}

#[test]
fn military_priorities_fire_and_match_the_old_path_every_day_for_300_days() {
    let (sc, f, fixtures) = m4d();
    let reports = compare(&sc, &f, &fixtures, sc.months * 30);
    let fired = fired(&reports);
    let at = |d: SimDate, label: u32, p: MilitaryPriority| fired.contains(&(d, label, p));
    // 優先度2.5: 戦時の国家1。1か月目の月次経済(2/01)の後の評価で、MilitaryFactoryを着工。以後、建設キューが空き、
    // 実月産が需要に届かない間は繰り返す(同時に2件は置かない)。
    assert!(at(date(1800, 2, 1), 1, MilitaryPriority::WartimeSupply));
    assert!(at(date(1800, 5, 31), 1, MilitaryPriority::WartimeSupply));
    // 優先度2.8: 国家2。4か月連続(同じMineレベル)のIron未充足が履歴に揃った7/02にMineを着工。
    assert!(at(date(1800, 7, 2), 2, MilitaryPriority::IronSupply));
    // 優先度2.9: 国家3。生産履歴4か月(2〜5月)が揃い、5/03から再建の文脈ができ、先行する建設が空いた8/03に着工。
    assert!(at(date(1800, 8, 3), 3, MilitaryPriority::MilitaryRebuild));
    // 国家3は5/03より前に再建の文脈を持たない(履歴4か月の境界)
    let rebuild_days: Vec<SimDate> = evals(&reports)
        .iter()
        .filter(|(_, e)| e.label == 3 && e.rebuild_context)
        .map(|(d, _)| *d)
        .collect();
    assert_eq!(rebuild_days[0], date(1800, 5, 3));
    // 優先度の取り違えがない: 国家ごとに発火した種別は1種類
    assert!(fired.iter().filter(|x| x.1 == 1).all(|x| x.2 == MilitaryPriority::WartimeSupply));
    assert!(fired.iter().filter(|x| x.1 == 2).all(|x| x.2 == MilitaryPriority::IronSupply));
    assert!(fired.iter().filter(|x| x.1 == 3).all(|x| x.2 == MilitaryPriority::MilitaryRebuild));
}

#[test]
fn history_boundaries_and_factory_expansion_gate_are_followed_exactly() {
    let (sc, f, fixtures) = m4d();
    let reports = compare(&sc, &f, &fixtures, sc.months * 30);
    let e2: Vec<(SimDate, &AiEvaluation)> = evals(&reports).into_iter().filter(|(_, e)| e.label == 2).collect();
    let at = |d: SimDate| e2.iter().find(|(x, _)| *x == d).map(|(_, e)| *e).unwrap();
    // Factory増設の抑止: 履歴が4か月連続(2〜5月)で全Factoryが飢餓、かつ設備が同じ 5/02 だけ。3か月(4/02)ではまだ抑止しない。
    assert!(!at(date(1800, 4, 2)).factory_blocked);
    assert!(at(date(1800, 5, 2)).factory_blocked);
    // 5/03にFactoryが完成して設備が変わると抑止は外れる(6/02)
    assert!(!at(date(1800, 6, 2)).factory_blocked);
    // Iron不足の履歴: 7/02で初めて「継続的な不足」と認識される(Mineレベルが変わった3/03以降の4か月 = 4〜7月)
    for (d, e) in &e2 {
        if *d < date(1800, 7, 2) {
            assert!(e.iron_shortage_average.is_none(), "{d:?}");
        }
    }
    assert!(at(date(1800, 7, 2)).iron_shortage_average.unwrap() > 0.0);
    // 着工した7/02の次の評価(8/02)は、建設中のMineの増分を差し引いた残りの不足で再判断され、重複して着工しない
    assert_eq!(at(date(1800, 8, 2)).fired, None);
}

#[test]
fn histories_are_recorded_per_country_at_the_monthly_production_and_stay_independent() {
    let (sc, f, _) = m4d();
    let (b, w) = (buildings(), world());
    let (mut reg, mut cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap();
    let d = derive_state_resources(&w, &f).unwrap();
    reg.attach_province_resources(&d, &w, &f).unwrap();
    // 月次生産の前(1月中)は履歴が空。月が変わった日(2/01)に、各国家へちょうど1件ずつ記録される。
    let r = reg.run_days(&mut cs, START, 30, &b);
    assert!(cs.iter().all(|c| c.ai.as_ref().unwrap().history.supply.samples().is_empty()));
    assert!(r.iter().all(|(_, x)| !x.monthly_ran));
    reg.run_days(&mut cs, date(1800, 1, 31), 1, &b);
    for c in &cs {
        let h = &c.ai.as_ref().unwrap().history;
        assert_eq!((h.supply.samples().len(), h.iron.samples().len(), h.factory.samples().len()), (1, 1, 1));
        assert_eq!((h.iron.samples()[0].year, h.iron.samples()[0].month), (1800, 2));
    }
    // 国家2のMilitaryFactory(4レベル)は月初のIron備蓄が0で稼働できず、初月からIron未充足が記録される。
    // 他国家は同じ月でも別の値(履歴は国家ごとに独立。混ざらない)。
    let unmet: Vec<Option<u64>> = cs.iter().map(|c| c.ai.as_ref().unwrap().history.iron.samples()[0].unmet_milli).collect();
    assert_eq!(unmet[0], Some(0), "country 1 has no MilitaryFactory yet");
    assert!(unmet[1].unwrap() > 0, "country 2 has MilitaryFactory without Iron");
    assert_eq!(unmet[2], Some(0));
    // Mineレベル合計の記録: 国家1=2、国家2=1、国家3=1(各国家の所有ステートだけから算出)
    let mines: Vec<u32> = cs.iter().map(|c| c.ai.as_ref().unwrap().history.iron.samples()[0].mine_levels).collect();
    assert_eq!(mines, vec![2, 1, 1]);
}

#[test]
fn missing_or_incomplete_military_input_is_never_treated_as_peace() {
    let w = world();
    let f = resources(&w, &[]);
    let stock = [(Iron, 3000.0), (Coal, 3000.0), (Food, 900.0), (Wood, 900.0), (IndustrialGoods, 900.0)];
    let war = MilitaryFixture { at_war: true, living: 10, queue: 0 };
    let run = |input: Option<MilitaryAiFacts>| {
        let mut c = country_input(1, 9000.0, 0.15, &stock, &[20], Some(war));
        c.military = input;
        let sc = scenario(
            3,
            vec![c],
            vec![state_input(20, 600001, &[(Farm, 3), (Mine, 2), (Factory, 1), (Railway, 1)])],
        );
        run_new_only(&sc, &f, 90)
    };
    // 入力あり: 2.5が発火する
    let given = facts_of(1, &war);
    assert!(fired(&run(Some(given))).iter().any(|x| x.2 == MilitaryPriority::WartimeSupply));
    // 入力なし: 2.5は評価されず、評価の記録は「入力なし」を明示する(平和・脅威ゼロへ置き換えない)
    let none = run(None);
    assert!(fired(&none).is_empty());
    assert!(evals(&none).iter().all(|(_, e)| e.military_input.is_none() && !e.wartime_context && !e.rebuild_context));
    // 戦時だが需要を算出できなかった(None): 戦時の文脈を作らない(0として扱わない)
    let mut broken = given;
    broken.daily_demand_units = None;
    let r = run(Some(broken));
    assert!(fired(&r).is_empty());
    assert!(evals(&r).iter().all(|(_, e)| e.military_input.is_some() && !e.wartime_context));
    // 戦時でない: 需要があっても2.5は評価されない
    let mut peace = given;
    peace.at_war = false;
    assert!(fired(&run(Some(peace))).is_empty());
    // 需要0(戦時・師団なし): 実月産(0) >= 需要(0) なので着工しない(旧実装と同じ)
    let mut zero = given;
    zero.daily_demand_units = Some(0);
    assert!(fired(&run(Some(zero))).is_empty());
    // 標準歩兵の定義が無い: 再建の需要を認識しない
    let mut nodef = given;
    nodef.infantry_required_manpower = None;
    let r = run(Some(nodef));
    assert!(evals(&r).iter().all(|(_, e)| !e.rebuild_context));
}

/// 新経路だけで実行する(軍事入力の変種は旧経路のfixtureでは表せない。旧経路との一致は標準のfixtureを使う
/// 各テストの`compare`で確認している)。
fn run_new_only(
    sc: &MultiMonthScenario,
    f: &ProvinceResourcesFile,
    days: u32,
) -> Vec<(SimDate, DayReport)> {
    let (b, w) = (buildings(), world());
    let (mut reg, mut cs) = MapStateEconomyRegistry::from_multi_scenario(sc, &w, &b).unwrap();
    let d = derive_state_resources(&w, f).unwrap();
    reg.attach_province_resources(&d, &w, f).unwrap();
    reg.run_days(&mut cs, START, days, &b)
}

#[test]
fn wartime_priority_budget_and_input_boundaries_match_the_old_path() {
    let w = world();
    let f = resources(&w, &[]);
    let war = MilitaryFixture { at_war: true, living: 10, queue: 0 };
    let build = |treasury: f64, iron: f64| {
        let stock = [(Iron, iron), (Coal, 3000.0), (Food, 900.0), (Wood, 900.0), (IndustrialGoods, 900.0)];
        scenario(
            2,
            vec![country_input(1, treasury, 0.15, &stock, &[20], Some(war))],
            vec![state_input(20, 600001, &[(Farm, 3), (Mine, 2), (Factory, 1), (Railway, 1)])],
        )
    };
    let fixtures: HashMap<u32, MilitaryFixture> = [(1, war)].into_iter().collect();
    let wartime_orders = |sc: &MultiMonthScenario| {
        let r = compare(sc, &f, &fixtures, 60, );
        r.iter()
            .flat_map(|(d, x)| x.ordered.iter().map(move |o| (*d, *o)))
            .filter(|(_, o)| o.2 == MilitaryFactory)
            .count()
    };
    // 条件を満たす: 着工する(費用1200)
    assert!(wartime_orders(&build(9000.0, 3000.0)) >= 1);
    // 予算不足: 国庫が建設費(1200) + 月次経常支出 × 1か月分に満たない
    assert_eq!(wartime_orders(&build(1200.0, 3000.0)), 0);
    // 入力資源の備蓄不足: Ironが追加後の月次需要(1か月分)に届かない
    assert_eq!(wartime_orders(&build(9000.0, 10.0)), 0);
}

#[test]
fn one_treasury_is_never_reused_and_orders_do_not_update_histories() {
    let (sc, f, fixtures) = m4d();
    let reports = compare(&sc, &f, &fixtures, sc.months * 30);
    // 1日の国家ごとの注文費用の合計は、その日の評価前の国庫を超えない(同じ国庫を複数の判断で重複利用しない)
    let (b, w) = (buildings(), world());
    let (mut reg, mut cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap();
    let d = derive_state_resources(&w, &f).unwrap();
    reg.attach_province_resources(&d, &w, &f).unwrap();
    let mut dt = START;
    for _ in 0..sc.months * 30 {
        let before: Vec<f64> = cs.iter().map(|c| c.treasury).collect();
        let (n, ch) = dt.next();
        dt = n;
        let r = reg.run_day(&mut cs, dt, ch, &b);
        for (i, c) in cs.iter().enumerate() {
            let spent: f64 = r.ordered.iter().filter(|o| o.0 == c.label).map(|o| o.3).sum();
            // 月次経済の収支は別に加わるので、注文の合計が「月初の国庫+その日の収支」を超えないことを確認する
            assert!(spent <= before[i] + c.monthly_balance.max(0.0) + 1e-9, "{dt:?} c{}", c.label);
            // 同じ国家のキューにMilitaryFactoryが2件同時に存在しない(重複注文の防止)
            assert!(c.construction_queue.iter().filter(|o| o.building_type == MilitaryFactory).count() <= 1);
            assert!(c.construction_queue.len() <= 2);
        }
    }
    // 履歴は注文の成否では更新されない(月次生産の観測値だけ): 注文の有無にかかわらず、毎日の履歴の内容は旧経路と
    // 一致している(上の`compare`が毎日の履歴を含めて比較済み)。
    assert!(reports.iter().any(|(_, r)| !r.ordered.is_empty()));
}

#[test]
fn an_invalid_mine_candidate_is_skipped_while_other_candidates_are_still_evaluated_for_iron_supply() {
    // 国家2が2つのステート(21・22)を持つ。22は鉱床の見込み(Iron 80)が最大だが、資源の設定が不完全(1件未設定)なので
    // Mineの候補から外れ、評価が続いて21(Iron 45)にMineが置かれる。22の設定を完全にすると22が選ばれる。
    let w = world();
    let (b, _) = (buildings(), ());
    let stock = [(Coal, 3000.0), (Food, 900.0), (Wood, 900.0), (IndustrialGoods, 900.0)];
    let peace = MilitaryFixture { at_war: false, living: 5, queue: 0 };
    let sc = scenario(
        9,
        vec![country_input(2, 9000.0, 0.15, &stock, &[21, 22], Some(peace))],
        vec![
            state_input(21, 400000, &[(Farm, 2), (Mine, 1), (Factory, 1), (MilitaryFactory, 4), (Railway, 1)]),
            state_input(22, 250000, &[(Farm, 2)]),
        ],
    );
    let run = |omit_province: bool| {
        let mut f = resources(&w, &[(17696, vec![dep(Iron, 25.0, true)]), (17266, vec![dep(Iron, 80.0, true)])]);
        if omit_province {
            f.provinces.retain(|e| e.province != 17268); // MapState#22の1プロヴィンスを未設定にする
        }
        let (mut reg, mut cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap();
        let d = derive_state_resources(&w, &f).unwrap();
        reg.attach_province_resources(&d, &w, &f).unwrap();
        let reports = reg.run_days(&mut cs, START, 270, &b);
        let mine_orders: Vec<(MapStateId, SimDate)> = reports
            .iter()
            .flat_map(|(d, r)| r.ordered.iter().filter(|o| o.2 == Mine).map(move |o| (o.1, *d)))
            .collect();
        let iron_fired = fired(&reports).iter().any(|x| x.2 == MilitaryPriority::IronSupply);
        (mine_orders, iron_fired)
    };
    let (orders_unset, _) = run(true);
    assert!(!orders_unset.is_empty());
    assert!(orders_unset.iter().all(|(st, _)| *st == s(21)), "unset state must never receive a Mine: {orders_unset:?}");
    let (orders_set, _) = run(false);
    assert!(orders_set.iter().any(|(st, _)| *st == s(22)), "complete resources make state 22 a candidate: {orders_set:?}");
}

#[test]
fn military_priorities_never_order_location_facilities_or_use_old_state_ids() {
    // 新経路の注文は MapStateId だけを対象にし、すべてステート単位の経済施設(位置施設ではない)
    let (sc, f, fixtures) = m4d();
    let reports = compare(&sc, &f, &fixtures, sc.months * 30);
    for (_, r) in &reports {
        for o in &r.ordered {
            assert!(strategy_game::economy::map_state::is_state_level_economic_facility(o.2));
            assert!([s(20), s(21), s(22)].contains(&o.1));
        }
    }
}

#[test]
fn wartime_and_rebuild_contexts_together_never_double_order_the_military_factory() {
    // 戦時(師団2個 < 目標3個)でMilitaryFactoryが無い国家。戦時の文脈(2.5)と再建の文脈(2.9)が同時に成立する月がある。
    // 2.5が先に評価され、建設キューにMilitaryFactoryがある間は2.5も2.9も重複して着工しない(旧経路と同じ)。
    let w = world();
    let f = resources(&w, &[]);
    let fx = MilitaryFixture { at_war: true, living: 2, queue: 0 };
    let stock = [(Iron, 3000.0), (Coal, 3000.0), (Food, 900.0), (Wood, 900.0)];
    let sc = scenario(
        10,
        vec![country_input(1, 9000.0, 0.15, &stock, &[20], Some(fx))],
        vec![state_input(20, 600001, &[(Farm, 3), (Mine, 2), (Factory, 1), (Railway, 1)])],
    );
    let fixtures: HashMap<u32, MilitaryFixture> = [(1, fx)].into_iter().collect();
    let reports = compare(&sc, &f, &fixtures, 300);
    let both: Vec<_> = evals(&reports).into_iter().filter(|(_, e)| e.wartime_context && e.rebuild_context).collect();
    assert!(!both.is_empty(), "the two contexts must coexist at least once");
    // 同時に成立している間に、MilitaryFactoryの二重注文は起きない
    assert!(both.iter().all(|(_, e)| e.fired.is_none()));
    // 最初の着工は2.5(4/01)。再建(2.9)は、2.5の文脈がある国家では先に評価されないため発火しない
    let f = fired(&reports);
    assert_eq!(f[0], (date(1800, 4, 1), 1, MilitaryPriority::WartimeSupply));
    assert!(f.iter().all(|x| x.2 == MilitaryPriority::WartimeSupply));
}

#[test]
fn rebuild_need_boundary_force_equal_to_target_is_not_a_need() {
    // MapState#22(人口250000): 上限 = 25000、目標師団 = (25000/2)/10000 = 1(手計算)。
    let w = world();
    let f = resources(&w, &[(17266, vec![dep(Iron, 40.0, true)])]);
    let stock = [(Iron, 3000.0), (Coal, 3000.0), (Food, 900.0), (Wood, 900.0)];
    let run = |living: usize, queue: usize| {
        let fx = MilitaryFixture { at_war: false, living, queue };
        let sc = scenario(
            6,
            vec![country_input(3, 9000.0, 0.15, &stock, &[22], Some(fx))],
            vec![state_input(22, 250000, &[(Farm, 2), (Mine, 1), (Factory, 1), (Railway, 1)])],
        );
        let fixtures: HashMap<u32, MilitaryFixture> = [(3, fx)].into_iter().collect();
        let reports = compare(&sc, &f, &fixtures, 180);
        evals(&reports).iter().any(|(_, e)| e.rebuild_context)
    };
    assert!(run(0, 0), "force 0 < target 1");
    assert!(!run(1, 0), "force 1 == target 1: no need");
    assert!(!run(0, 1), "recruitment queue counts as force: 0 + 1 == target 1");
    assert!(!run(2, 0), "force above target");
}

#[test]
fn lakes_exist_in_the_full_geography_but_not_around_the_preview_region() {
    // P32-011報告の訂正の根拠: 全体の地理JSONには湖セルがあるが、検証地域(previewの29プロヴィンス)とその
    // 近傍6セルには湖がない(近傍の水域はすべて海)。
    use strategy_game::hexmap::data::{Terrain, hex_neighbors, load_from_path};
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let lakes = map.cells().iter().filter(|c| c.terrain == Terrain::Lake).count();
    assert_eq!(lakes, 202, "whole map");
    let w = world();
    let mut lake_near_preview = 0;
    let mut sea_near_preview = 0;
    for p in w.provinces() {
        assert_ne!(map.cell(p.cell.q, p.cell.r).unwrap().terrain, Terrain::Lake);
        for (q, r) in hex_neighbors(p.cell.q, p.cell.r) {
            match map.cell(q, r).map(|c| c.terrain) {
                Some(Terrain::Lake) => lake_near_preview += 1,
                Some(Terrain::Sea) => sea_near_preview += 1,
                _ => {}
            }
        }
    }
    assert_eq!(lake_near_preview, 0, "preview region");
    assert!(sea_near_preview > 0);
}
