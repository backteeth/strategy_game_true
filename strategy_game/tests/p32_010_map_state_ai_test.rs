//! P32-010 (M4-C): 新ステート(`MapStateId`)の経済状態の分類・建設キュー・経済AI(税率調整と
//! 経済施設の建設判断)が、旧経路(**実際の旧Bevy System** `handle_daily_construction` /
//! `handle_monthly_population_growth` / `handle_monthly_economy` と、旧AIの公開関数
//! `process_tax_rate_ai` / `process_construction_ai_with_contexts`)と日ごとに一致することのテスト。
//! 入力は検証用シナリオで、正式な人口・施設・資源・国家配置・バランスではない。旧`StateId(n+1)`と
//! シナリオ順の`MapStateId`の対応、旧`CountryId(label)`との対応(AIの評価周期の識別のみ)は、
//! テスト内で明示する。**比較範囲**: 旧AIの軍事の優先度(戦時MilitaryFactory・Iron供給・軍需再建)・
//! 履歴に基づく増設抑止・位置施設・プレイヤーの手動注文とキャンセルは比較対象外(新経路は未対応)。

use bevy::prelude::*;
use std::collections::HashMap;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::construction::ConstructionStatus;
use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::ai_core::OrderBlock;
use strategy_game::country::country_ai::{
    CountryAiState, process_construction_ai_with_contexts, process_tax_rate_ai,
};
use strategy_game::country::{CountryData, CountryRegistry, PlayerCountry};
use strategy_game::economy::state_access::StateEconomyAccess;
use strategy_game::economy::economic_state::{
    EconomicMetrics, EconomicState, calculate_economic_state,
};
use strategy_game::economy::map_state::{
    DayReport, EconomyDataIssue, MapStateEconomyRegistry, MultiMonthScenario, OrderError,
    ScenarioCountry, SimDate, is_state_level_economic_facility, parse_multi_month_scenario,
};
use strategy_game::economy::province_resources::{
    ProvinceResourceEntry, ProvinceResourcesFile, derive_state_resources,
    parse_province_resources,
};
use strategy_game::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use strategy_game::localization::{CurrentLocale, Locale, TranslationCatalog};
use strategy_game::population::growth::handle_monthly_population_growth;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::ui::notification::GameNotification;
use strategy_game::world::{MapStateId, WorldModel, load_province_table, load_state_layout};

const SCENARIO: &str = "assets/data/world/preview/economy_scenario_m4c.ron";
const RES_PATH: &str = "assets/data/world/preview/province_resources.ron";

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

fn scenario() -> MultiMonthScenario {
    parse_multi_month_scenario(&std::fs::read_to_string(SCENARIO).unwrap()).unwrap()
}

fn preview_resources() -> ProvinceResourcesFile {
    parse_province_resources(&std::fs::read_to_string(RES_PATH).unwrap()).unwrap()
}

/// 全29プロヴィンスを設定した資源定義(未設定を「資源なし」の明示で埋める)。
/// 新経路のMine前提(全プロヴィンス設定済み)を満たす = 旧経路と比較できる。
fn fully_configured_resources(w: &WorldModel) -> ProvinceResourcesFile {
    let mut f = preview_resources();
    for p in w.provinces() {
        if !f.provinces.iter().any(|e| e.province == p.id.0) {
            f.provinces.push(ProvinceResourceEntry {
                province: p.id.0,
                deposits: vec![],
            });
        }
    }
    f
}

fn dep(r: ResourceType, out: f64, discovered: bool) -> StateResourceDeposit {
    StateResourceDeposit {
        resource_type: r,
        base_output: out,
        discovered,
        development_level: 1,
    }
}

fn s(id: u32) -> MapStateId {
    MapStateId(id)
}

fn new_world(sc: &MultiMonthScenario, w: &WorldModel, b: &BuildingRegistry, f: &ProvinceResourcesFile) -> (MapStateEconomyRegistry, Vec<ScenarioCountry>) {
    let (mut reg, cs) = MapStateEconomyRegistry::from_multi_scenario(sc, w, b).expect("scenario");
    let d = derive_state_resources(w, f).expect("resources");
    reg.attach_province_resources(&d, w, f).expect("attach");
    (reg, cs)
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
    available_manpower: u64,
    stockpile: Vec<(ResourceType, f64)>,
    economic_state: EconomicState,
    /// (対象ステートのシナリオ内の添字, 施設, 目標レベル, 進捗, 必要進捗, 支払済み費用, 状態)
    queue: Vec<(usize, BuildingType, u32, f64, f64, f64, ConstructionStatus)>,
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
        countries: cs
            .iter()
            .map(|c| CountrySnap {
                treasury: c.treasury,
                tax_rate: c.tax_rate,
                income: c.monthly_income,
                expenses: c.monthly_expenses,
                balance: c.monthly_balance,
                available_manpower: c.available_manpower,
                stockpile: stock_vec(&c.stockpile),
                economic_state: c.economic_state,
                queue: c
                    .construction_queue
                    .iter()
                    .map(|o| {
                        (
                            ids.iter().position(|i| *i == o.state).unwrap(),
                            o.building_type,
                            o.target_level,
                            o.progress,
                            o.required_progress,
                            o.paid_cost,
                            o.status,
                        )
                    })
                    .collect(),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------- 旧経路(実際の旧System+旧AIの公開関数)

struct OldWorld {
    app: App,
    ai: HashMap<usize, CountryAiState>,
    ai_labels: Vec<usize>,
}

fn old_world(sc: &MultiMonthScenario, b: &BuildingRegistry, f: &ProvinceResourcesFile, w: &WorldModel) -> OldWorld {
    // 鉱床は旧入力(ステート単位)として、プロヴィンス別定義の集計結果と同じ内容を与える。
    let d = derive_state_resources(w, f).unwrap();
    // 対応表(明示): 旧StateId(n+1) ↔ シナリオ内n番目のステート。旧CountryId(label)は評価周期の識別のみ。
    let owner_of: HashMap<u32, usize> = sc
        .countries
        .iter()
        .flat_map(|c| c.states.iter().map(move |st| (*st, c.label as usize)))
        .collect();
    let states: Vec<StateData> = sc
        .states
        .iter()
        .enumerate()
        .map(|(n, i)| StateData {
            id: StateId(n + 1),
            owner_country_id: CountryId(owner_of[&i.state]),
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
            CountryData {
                id: CountryId(c.label as usize),
                treasury: c.treasury,
                tax_rate: c.tax_rate,
                available_manpower: c.available_manpower,
                mobilized_manpower: c.mobilized_manpower,
                stockpile,
                ..CountryData::default()
            }
        })
        .collect();
    let ai_labels: Vec<usize> = {
        let mut v: Vec<usize> = sc.countries.iter().filter(|c| c.ai_controlled).map(|c| c.label as usize).collect();
        v.sort();
        v
    };
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<DayChangedMessage>()
        .add_message::<MonthChangedMessage>()
        .add_message::<GameNotification>()
        .insert_resource(CountryRegistry { countries })
        .insert_resource(StateRegistry::build(states))
        .insert_resource(BuildingRegistry { definitions: b.definitions.clone() })
        .insert_resource(PlayerCountry(None))
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
    let ai = ai_labels.iter().map(|l| (*l, CountryAiState::new(CountryId(*l)))).collect();
    OldWorld { app, ai, ai_labels }
}

impl OldWorld {
    /// 旧スケジュールの1日: Economy Set(建設進捗 → 月次経済) → CountryAi Set(AIの月次評価)。
    fn day(&mut self, date: SimDate, month_changed: bool, b: &BuildingRegistry) {
        self.app.world_mut().write_message(DayChangedMessage { year: date.year, month: date.month, day: date.day });
        if month_changed {
            self.app.world_mut().write_message(MonthChangedMessage { year: date.year, month: date.month });
        }
        self.app.update();
        let day_number = date.day_number();
        for label in self.ai_labels.clone() {
            let state = self.ai.get_mut(&label).unwrap();
            let due = (day_number as usize % 30 == label % 30) || state.dirty;
            if due {
                self.app.world_mut().resource_scope::<CountryRegistry, _>(|world, mut cr| {
                    let sr = world.resource::<StateRegistry>();
                    process_tax_rate_ai(CountryId(label), &mut cr, sr);
                    process_construction_ai_with_contexts(
                        CountryId(label),
                        &mut cr,
                        sr,
                        b,
                        state,
                        None,
                        None,
                        false,
                        None,
                    );
                });
            }
            state.dirty = false;
        }
    }

    fn snap(&self, sc: &MultiMonthScenario) -> DaySnap {
        let sr = self.app.world().resource::<StateRegistry>();
        let cr = self.app.world().resource::<CountryRegistry>();
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
                        available_manpower: c.available_manpower,
                        stockpile: stock_vec(&c.stockpile),
                        economic_state: c.economic_state,
                        queue: c
                            .construction_queue
                            .iter()
                            .map(|o| {
                                (
                                    o.state_id.0 - 1,
                                    o.building_type,
                                    o.target_level,
                                    o.progress,
                                    o.required_progress,
                                    o.paid_cost,
                                    o.status,
                                )
                            })
                            .collect(),
                    }
                })
                .collect(),
        }
    }
}

const START: SimDate = SimDate { year: 1800, month: 1, day: 1 };

/// 同じ初期条件・同じ日付の流れで旧新を1日ずつ実行し、毎日の全状態が一致することを確認する。
/// 戻り値は新経路の日ごとの出来事。
fn assert_equal_daily(sc: &MultiMonthScenario, f: &ProvinceResourcesFile, days: u32) -> Vec<(SimDate, DayReport)> {
    let (b, w) = (buildings(), world());
    let mut old = old_world(sc, &b, f, &w);
    let (mut reg, mut cs) = new_world(sc, &w, &b, f);
    let mut date = START;
    let mut reports = Vec::new();
    for day in 1..=days {
        let (d, changed) = date.next();
        date = d;
        old.day(date, changed, &b);
        let rep = reg.run_day(&mut cs, date, changed, &b);
        let (o, n) = (old.snap(sc), snap_new(&reg, &cs));
        assert_eq!(o.states, n.states, "states differ on day {day} ({date:?})");
        assert_eq!(o.countries, n.countries, "countries differ on day {day} ({date:?})");
        reports.push((date, rep));
    }
    reports
}

// ---------------------------------------------------------------- 旧新一致(120日)

#[test]
fn four_month_verification_scenario_matches_the_old_path_every_day() {
    let w = world();
    let sc = scenario();
    let f = fully_configured_resources(&w);
    let reports = assert_equal_daily(&sc, &f, sc.months * 30);
    // 少なくとも1件の建設が、通常の期間で完成している
    let completed: usize = reports.iter().map(|(_, r)| r.completed.len()).sum();
    assert!(completed >= 3, "completed {completed}");
    assert!(reports.iter().any(|(_, r)| !r.tax_changes.is_empty()), "tax AI acted");
}

#[test]
fn boundary_scenarios_match_the_old_path_daily() {
    let w = world();
    let f = fully_configured_resources(&w);
    let farm = |n| (BuildingType::Farm, n);
    let mut base = scenario();
    // 人口ゼロ・施設ゼロ・動員ゼロ・失業者あり
    base.states[0].population = 0;
    base.states[1].buildings = Default::default();
    base.states[2].buildings = [farm(1)].into_iter().collect();
    base.countries[0].mobilized_manpower = 0;
    base.months = 3;
    assert_equal_daily(&base, &f, 90);
    // 高い動員負担・低い国庫(予算不足)・高税率
    let mut poor = scenario();
    poor.countries[0].mobilized_manpower = 900_000;
    poor.countries[0].treasury = 300.0;
    poor.countries[1].treasury = 100.0;
    poor.months = 3;
    assert_equal_daily(&poor, &f, 90);
}

// ---------------------------------------------------------------- 独立した期待値(旧経路を使わない)

#[test]
fn construction_cost_progress_and_completion_match_hand_values() {
    let (b, w) = (buildings(), world());
    let sc = scenario();
    let (mut reg, mut cs) = new_world(&sc, &w, &b, &fully_configured_resources(&w));
    let reports = reg.run_days(&mut cs, START, 100, &b);
    // 国家1: 1800/01/02 に Mine(費用 500、必要進捗 60日)を MapState#20 へ。支払いは1回だけ。
    let day2 = &reports[0];
    assert_eq!(day2.0, SimDate { year: 1800, month: 1, day: 2 });
    assert!(day2.1.ordered.contains(&(1, s(20), BuildingType::Mine, 500.0)));
    // 国家2: Food 0 < 50 の食料緊急で Farm(費用 200、30日)を MapState#21 へ
    assert!(day2.1.ordered.contains(&(2, s(21), BuildingType::Farm, 200.0)));
    // Farm: 1/02 + 30日 = 2/01 に Lv6→7 へ、ちょうど1回
    let farm_done: Vec<_> = reports
        .iter()
        .flat_map(|(d, r)| r.completed.iter().map(move |c| (*d, *c)))
        .filter(|(_, c)| c.2 == BuildingType::Farm)
        .collect();
    assert!(!farm_done.is_empty());
    assert_eq!(farm_done[0].0, SimDate { year: 1800, month: 2, day: 1 });
    assert_eq!(farm_done[0].1, (2, s(21), BuildingType::Farm, 7));
    // Mine: 1/02 + 60日(Jan 2 → Jan 31 = +29、Feb 28 = +57、Mar 3 = +60)= 3/03 に Lv2→3、ちょうど1回
    let mine_done: Vec<_> = reports
        .iter()
        .flat_map(|(d, r)| r.completed.iter().map(move |c| (*d, *c)))
        .filter(|(_, c)| c.2 == BuildingType::Mine && c.1 == s(20))
        .collect();
    assert_eq!(mine_done.len(), 1, "{mine_done:?}");
    assert_eq!(mine_done[0].0, SimDate { year: 1800, month: 3, day: 3 });
    assert_eq!(mine_done[0].1, (1, s(20), BuildingType::Mine, 3));
    assert_eq!(reg.get(s(20)).unwrap().building_level(BuildingType::Mine), 3);
    // 国庫(国家1): 5000 - 500(Mine) - 1000(Factory, 1/31の2件目)。月次の収支は別に加わるが、
    // 建設費は注文の時点で1回ずつだけ引かれ、完成時には引かれない。
    let paid: f64 = cs[0].construction_queue.iter().map(|o| o.paid_cost).sum();
    assert!(paid >= 1000.0, "factory order still queued: {paid}");
    // キューは最大2件
    assert!(cs[0].construction_queue.len() <= 2);
}

#[test]
fn employment_after_completion_matches_the_hand_value() {
    let (b, w) = (buildings(), world());
    let sc = scenario();
    let (mut reg, mut cs) = new_world(&sc, &w, &b, &fully_configured_resources(&w));
    // 3/03 の Mine 完成の後の最初の月次経済(4/01)まで
    reg.run_days(&mut cs, START, 90, &b);
    // 必要労働力: Farm3×5000 + Mine3×10000 + Factory1×20000 + Railway1×3000 = 68000(潜在労働力は十分)
    assert_eq!(reg.get(s(20)).unwrap().employed_workforce, 68_000);
}

#[test]
fn economic_state_thresholds_match_the_existing_scores() {
    let m = |emp: f32, ls: f32, log: f32, op: f32, bal: f64, food: f64| EconomicMetrics {
        employment_rate: emp,
        avg_living_standard: ls,
        avg_logistics_ratio: log,
        monthly_balance: bal,
        avg_building_operation: op,
        food_stockpile: food,
    };
    // スコア = 雇用率×25 + 生活水準×0.25 + 物流×15 + 稼働率×15 ± 収支10 ± 食料10(閾値 80/60/40/20)
    assert_eq!(calculate_economic_state(&m(1.0, 100.0, 1.0, 1.0, 0.0, 100.0)), EconomicState::Prosperity); // 80
    assert_eq!(calculate_economic_state(&m(1.0, 99.0, 1.0, 1.0, 0.0, 100.0)), EconomicState::Boom); // 79.75
    assert_eq!(calculate_economic_state(&m(1.0, 40.0, 1.0, 1.0, 0.0, 100.0)), EconomicState::Boom); // 65
    assert_eq!(calculate_economic_state(&m(0.0, 0.0, 1.0, 1.0, 10.0, 100.0)), EconomicState::Stable); // 15+15+10 = 40
    assert_eq!(calculate_economic_state(&m(0.0, 0.0, 1.0, 1.0, -10.0, 100.0)), EconomicState::Recession); // 20
    assert_eq!(calculate_economic_state(&m(0.0, 0.0, 1.0, 0.9, -10.0, 100.0)), EconomicState::Depression); // 18.5
}

// ---------------------------------------------------------------- 建設の注文(受付・拒否・二重実行の防止)

fn simple_registry(c_states: &[u32]) -> (MapStateEconomyRegistry, Vec<ScenarioCountry>, BuildingRegistry, WorldModel) {
    let (b, w) = (buildings(), world());
    let mut sc = scenario();
    for c in &mut sc.countries {
        c.ai_controlled = false;
    }
    sc.countries = vec![sc.countries[0].clone()];
    sc.countries[0].states = c_states.to_vec();
    sc.states.retain(|st| c_states.contains(&st.state));
    let (reg, cs) = new_world(&sc, &w, &b, &fully_configured_resources(&w));
    (reg, cs, b, w)
}

#[test]
fn place_order_pays_once_and_completes_exactly_once() {
    let (mut reg, mut cs, b, _) = simple_registry(&[20, 21]);
    let t0 = cs[0].treasury;
    reg.place_order(&mut cs[0], s(21), BuildingType::Farm, &b).unwrap();
    assert_eq!(cs[0].treasury, t0 - 200.0);
    assert_eq!(cs[0].construction_queue.len(), 1);
    // 重複注文・上限・存在しない・別国家
    assert_eq!(reg.place_order(&mut cs[0], s(21), BuildingType::Farm, &b), Err(OrderError::AlreadyQueued));
    assert_eq!(reg.place_order(&mut cs[0], s(99), BuildingType::Farm, &b), Err(OrderError::UnknownState(s(99))));
    assert_eq!(reg.place_order(&mut cs[0], s(22), BuildingType::Farm, &b), Err(OrderError::UnknownState(s(22))), "22 has no economy data in this registry");
    assert_eq!(cs[0].treasury, t0 - 200.0, "rejected orders change nothing");
    // 30日で完成し、以降の日では二度と反映されない
    let level0 = reg.get(s(21)).unwrap().building_level(BuildingType::Farm);
    let days = reg.run_days(&mut cs, START, 40, &b);
    assert_eq!(days.iter().map(|(_, r)| r.completed.len()).sum::<usize>(), 1);
    assert_eq!(reg.get(s(21)).unwrap().building_level(BuildingType::Farm), level0 + 1);
    assert!(cs[0].construction_queue.is_empty());
}

#[test]
fn rejected_orders_leave_everything_unchanged() {
    let (reg, mut cs, b, _) = simple_registry(&[20, 21]);
    let before = (reg.clone(), cs.clone());
    // 資金不足
    cs[0].treasury = 100.0;
    let snapshot = (reg.clone(), cs.clone());
    assert!(matches!(reg.place_order(&mut cs[0], s(21), BuildingType::Farm, &b), Err(OrderError::InsufficientFunds { .. })));
    assert_eq!((reg.clone(), cs.clone()), snapshot);
    // 上限: MapState#21のFarmを最大レベルにしたシナリオで試す
    cs[0].treasury = before.1[0].treasury;
    let max = b.get(BuildingType::Farm).unwrap().max_level;
    let w = world();
    let mut sc = scenario();
    sc.countries.truncate(1);
    sc.countries[0].states = vec![20, 21];
    sc.states.truncate(2);
    sc.states[1].buildings = [(BuildingType::Farm, max)].into_iter().collect();
    let (reg2, mut cs2) = new_world(&sc, &w, &b, &fully_configured_resources(&w));
    let snap2 = (reg2.clone(), cs2.clone());
    assert_eq!(reg2.place_order(&mut cs2[0], s(21), BuildingType::Farm, &b), Err(OrderError::MaxLevel { level: max, max }));
    assert_eq!((reg2, cs2), snap2);
    // 未定義の施設(登録簿に無い)
    let mut b2 = buildings();
    b2.definitions.remove(&BuildingType::University);
    assert_eq!(reg.place_order(&mut cs[0], s(20), BuildingType::University, &b2), Err(OrderError::UndefinedBuilding(BuildingType::University)));
    // 位置施設は現在 `BuildingType` に存在しない: すべての現行の施設種別はステート単位の経済施設
    assert!(BuildingType::ALL.iter().all(|x| is_state_level_economic_facility(*x)));
}

#[test]
fn order_cost_and_duration_do_not_depend_on_province_count() {
    // MapState#20(7プロヴィンス)と#21(13プロヴィンス): 同じ施設の費用・必要進捗は同じ
    let (reg, mut cs, b, w) = simple_registry(&[20, 21]);
    assert_ne!(w.provinces_of(s(20)).unwrap().len(), w.provinces_of(s(21)).unwrap().len());
    reg.place_order(&mut cs[0], s(20), BuildingType::LoggingCamp, &b).unwrap();
    reg.place_order(&mut cs[0], s(21), BuildingType::LoggingCamp, &b).unwrap();
    let q = &cs[0].construction_queue;
    assert_eq!((q[0].paid_cost, q[0].required_progress), (q[1].paid_cost, q[1].required_progress));
    assert_eq!(q[0].paid_cost, 250.0);
}

// ---------------------------------------------------------------- 資源入力とMine

fn country1_only(resources: &ProvinceResourcesFile) -> (MapStateEconomyRegistry, Vec<ScenarioCountry>, BuildingRegistry, WorldModel) {
    let (b, w) = (buildings(), world());
    let mut sc = scenario();
    sc.countries.truncate(1);
    sc.countries[0].states = vec![20, 21];
    sc.states.truncate(2);
    sc.states[1].buildings = [(BuildingType::Farm, 6)].into_iter().collect(); // 21: Mineなし
    let (reg, cs) = new_world(&sc, &w, &b, resources);
    (reg, cs, b, w)
}

fn first_day_orders(resources: &ProvinceResourcesFile) -> Vec<(u32, MapStateId, BuildingType, f64)> {
    let (mut reg, mut cs, b, _) = country1_only(resources);
    reg.run_days(&mut cs, START, 1, &b).remove(0).1.ordered
}

#[test]
fn mine_candidate_depends_on_configured_usable_deposits() {
    let w = world();
    // (1) 設定済みで鉱床あり(MapState#20): Mine
    let full = fully_configured_resources(&w);
    assert_eq!(first_day_orders(&full)[0].2, BuildingType::Mine);
    // (2) 設定済みで資源なし: 候補にならず、Factoryへフォールバック(資源なしの明示は有効な設定)
    let mut none = full.clone();
    for e in &mut none.provinces {
        e.deposits.clear();
    }
    assert_eq!(first_day_orders(&none)[0].2, BuildingType::Factory);
    // (3) 未発見の鉱床のみ: 発見済みとして扱わない → Factory
    let mut undiscovered = none.clone();
    for e in &mut undiscovered.provinces {
        if e.province == 18309 {
            e.deposits = vec![dep(ResourceType::Iron, 40.0, false)];
        }
    }
    assert_eq!(first_day_orders(&undiscovered)[0].2, BuildingType::Factory);
}

#[test]
fn unset_resources_are_not_adopted_but_other_candidates_are_still_evaluated() {
    let w = world();
    // MapState#20 を未設定(プロヴィンス1件を省略)、MapState#21 を完全設定で鉱床あり。
    let mut f = fully_configured_resources(&w);
    f.provinces.retain(|e| e.province != 18500); // 20のプロヴィンスを1つ未設定にする
    for e in &mut f.provinces {
        if e.province == 17694 {
            e.deposits = vec![dep(ResourceType::Iron, 20.0, true)];
        }
    }
    // MapState#20(Mineなし)は設定が不完全、MapState#21 は完全設定で鉱床あり。
    let (b, _) = (buildings(), ());
    let mut sc = scenario();
    sc.countries.truncate(1);
    sc.countries[0].states = vec![20, 21];
    sc.states.truncate(2);
    sc.states[0].buildings.remove(&BuildingType::Mine);
    sc.states[1].buildings = [(BuildingType::Farm, 6)].into_iter().collect();
    let (mut reg, mut cs) = new_world(&sc, &w, &b, &f);
    let orders = reg.run_days(&mut cs, START, 1, &b).remove(0).1.ordered;
    // 20は未設定のため候補から外れ、評価が続いて MapState#21 にMineが置かれる
    assert_eq!((orders[0].1, orders[0].2), (s(21), BuildingType::Mine), "{orders:?}");
    // 未設定の状態への直接注文は、受付時にエラー(完成後の月次経済が入力不備で失敗する経路を塞ぐ)
    let (reg, mut cs) = new_world(&sc, &w, &b, &f);
    assert_eq!(
        reg.place_order(&mut cs[0], s(20), BuildingType::Mine, &b),
        Err(OrderError::Precondition(OrderBlock::ResourcesNotConfigured))
    );
}

#[test]
fn resources_changing_after_an_order_block_the_completion_without_partial_updates() {
    let (b, w) = (buildings(), world());
    // MapState#20 は Mine 0 から始める(資源が後から不備になっても接続=attach を拒否されない)。
    let mut sc = scenario();
    sc.countries.truncate(1);
    sc.countries[0].states = vec![20];
    sc.countries[0].ai_controlled = false;
    sc.states.truncate(1);
    sc.states[0].buildings = [(BuildingType::Farm, 3)].into_iter().collect();
    let good = fully_configured_resources(&w);
    let (reg, mut cs) = new_world(&sc, &w, &b, &good);
    reg.place_order(&mut cs[0], s(20), BuildingType::Mine, &b).unwrap();
    let paid_treasury = cs[0].treasury;
    // 注文後に、MapState#20 の資源が未設定になる(プロヴィンスの設定が削除された)
    let mut bad = good.clone();
    bad.provinces.retain(|e| e.province != 18500);
    let d_bad = derive_state_resources(&w, &bad).unwrap();
    let (mut reg, _) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap();
    reg.attach_province_resources(&d_bad, &w, &bad).unwrap();
    // 60日(通常の建設期間)進める: 完成条件は満たすが、前提が崩れているため反映を保留する
    let days = reg.run_days(&mut cs, START, 61, &b);
    let blocked: Vec<_> = days.iter().flat_map(|(_, r)| r.blocked.iter()).collect();
    assert!(!blocked.is_empty(), "completion must be blocked");
    assert_eq!(blocked[0].3, OrderBlock::ResourcesNotConfigured);
    assert_eq!(reg.get(s(20)).unwrap().building_level(BuildingType::Mine), 0, "no facility without valid inputs");
    assert_eq!(cs[0].construction_queue.len(), 1, "the order stays queued (no refund, no loss)");
    assert!(cs[0].treasury >= paid_treasury - 1.0, "no additional cost was consumed");
    // 資源を設定し直すと、次の日に1回だけ完成する
    let d_ok = derive_state_resources(&w, &good).unwrap();
    let (mut reg, _) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap();
    reg.attach_province_resources(&d_ok, &w, &good).unwrap();
    let next = reg.run_days(&mut cs, SimDate { year: 1800, month: 3, day: 2 }, 3, &b);
    assert_eq!(next.iter().map(|(_, r)| r.completed.len()).sum::<usize>(), 1);
    assert_eq!(reg.get(s(20)).unwrap().building_level(BuildingType::Mine), 1);
    assert!(cs[0].construction_queue.is_empty());
}

#[test]
fn the_unset_to_mine_path_is_closed_at_attach_time_too() {
    // Mineを持つステートの資源が未設定になる接続はエラー(月次経済が入力不備で失敗しない)。
    let (b, w) = (buildings(), world());
    let sc = scenario();
    let (mut reg, _cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap();
    let mut bad = fully_configured_resources(&w);
    bad.provinces.retain(|e| e.province != 18500);
    let d = derive_state_resources(&w, &bad).unwrap();
    let before = reg.clone();
    let e = reg.attach_province_resources(&d, &w, &bad).unwrap_err();
    assert!(e.iter().any(|i| matches!(i, EconomyDataIssue::ResourcesNotConfigured { state, unset_provinces } if *state == s(20) && unset_provinces == &vec![18500])));
    assert_eq!(reg, before, "no partial update on error");
}

// ---------------------------------------------------------------- AI

#[test]
fn ai_is_deterministic_and_independent_of_input_order() {
    let (b, w) = (buildings(), world());
    let sc = scenario();
    let f = fully_configured_resources(&w);
    let run = |sc: &MultiMonthScenario| {
        let (mut reg, mut cs) = new_world(sc, &w, &b, &f);
        let days = reg.run_days(&mut cs, START, 120, &b);
        let mut by_label: Vec<(u32, f64, f32, usize)> = cs
            .iter()
            .map(|c| (c.label, c.treasury, c.tax_rate, c.construction_queue.len()))
            .collect();
        by_label.sort_by_key(|x| x.0);
        (by_label, days.iter().map(|(d, r)| (*d, r.ordered.clone(), r.completed.clone())).collect::<Vec<_>>())
    };
    assert_eq!(run(&sc), run(&sc), "same input, same result (no randomness)");
    // 国家・ステートの記述順を逆にしても、国家ごとの結果は同じ
    let mut rev = sc.clone();
    rev.countries.reverse();
    rev.states.reverse();
    let (a, _) = run(&sc);
    let (c, _) = run(&rev);
    assert_eq!(a, c);
}

#[test]
fn budget_is_not_reused_across_states_or_leaked_between_countries() {
    let (b, w) = (buildings(), world());
    let sc = scenario();
    let (mut reg, mut cs) = new_world(&sc, &w, &b, &fully_configured_resources(&w));
    let before: Vec<f64> = cs.iter().map(|c| c.treasury).collect();
    let r = reg.run_days(&mut cs, START, 1, &b).remove(0).1;
    for (i, c) in cs.iter().enumerate() {
        let spent: f64 = r.ordered.iter().filter(|o| o.0 == c.label).map(|o| o.3).sum();
        assert_eq!(c.treasury, before[i] - spent, "only this country's own orders were paid");
        assert!(spent <= before[i]);
    }
    // 各国家の注文は、その国家の経済計算に含まれるステートだけを対象にする
    for o in &r.ordered {
        let owner = cs.iter().find(|c| c.label == o.0).unwrap();
        assert!(owner.states.contains(&o.1));
    }
}

#[test]
fn old_ai_orders_a_mine_where_the_new_path_refuses_unset_resources() {
    // 旧経路は資源の設定状況を持たない。プレビューの資源定義(MapState#21は未設定が残る)では、
    // 旧AIは鉱床のある#21へMineを置けるが、新経路は受付前提で採用しない(意図した差。比較対象外)。
    let w = world();
    let sc = scenario();
    let f = preview_resources();
    let (b, _) = (buildings(), ());
    let mut old = old_world(&sc, &b, &f, &w);
    let (mut reg, mut cs) = new_world(&sc, &w, &b, &f);
    let mut date = START;
    for _ in 0..120 {
        let (d, ch) = date.next();
        date = d;
        old.day(date, ch, &b);
        reg.run_day(&mut cs, date, ch, &b);
    }
    let old_has_mine_at_21 = old
        .app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(2))
        .map(|st| st.building_level(BuildingType::Mine) > 0)
        .unwrap();
    assert!(old_has_mine_at_21);
    assert_eq!(reg.get(s(21)).unwrap().building_level(BuildingType::Mine), 0);
    assert!(cs.iter().all(|c| c.construction_queue.iter().all(|o| !(o.state == s(21) && o.building_type == BuildingType::Mine))));
}
