//! P32-008 (M4-B): 新ステート(`MapStateId`)の人口成長・財政・福祉・国家集計が、旧経路
//! (**実際の旧Bevy System** `handle_monthly_population_growth` → `handle_monthly_economy`)と
//! 複数月にわたって一致することのテスト。入力は検証用シナリオで、正式な人口・税率・国家所属・
//! ゲームバランスではない。旧`StateId`と`MapStateId`は同じ数値でも相互変換せず、対応表を
//! テスト内で明示する(旧`StateId(n+1)` ↔ シナリオ順のステート)。

use bevy::prelude::*;
use std::collections::HashMap;
use strategy_game::app::time::MonthChangedMessage;
use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryData, CountryRegistry, PlayerCountry};
use strategy_game::economy::map_state::{
    EconomyDataIssue, MapStateEconomyInput, MapStateEconomyRegistry, MultiMonthScenario,
    ScenarioCountry, ScenarioCountryInput, parse_multi_month_scenario,
};
use strategy_game::economy::resources::{CountryStockpile, ResourceType};
use strategy_game::hexmap::data::load_from_path;
use strategy_game::localization::{CurrentLocale, Locale, TranslationCatalog};
use strategy_game::population::growth::{
    grow_state_population, handle_monthly_population_growth, recover_country_manpower,
};
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::ui::notification::GameNotification;
use strategy_game::world::{MapStateId, WorldModel, load_world_preview};

const SCENARIO: &str = "assets/data/world/preview/economy_scenario_m4b.ron";

fn buildings() -> BuildingRegistry {
    let defs: Vec<BuildingDefinition> =
        ron::from_str(&std::fs::read_to_string("assets/data/buildings.ron").unwrap()).unwrap();
    BuildingRegistry {
        definitions: defs.into_iter().map(|d| (d.building_type, d)).collect(),
    }
}

fn world() -> WorldModel {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    load_world_preview(
        "assets/data/world/preview/provinces.ron",
        "assets/data/world/preview/states.ron",
        &map,
    )
    .unwrap()
    .model
}

fn scenario() -> MultiMonthScenario {
    parse_multi_month_scenario(&std::fs::read_to_string(SCENARIO).unwrap()).unwrap()
}

fn st(
    state: u32,
    pop: u64,
    ratio: f32,
    ls: f32,
    b: &[(BuildingType, u32)],
) -> MapStateEconomyInput {
    MapStateEconomyInput {
        state,
        population: pop,
        workforce_ratio: ratio,
        living_standard: ls,
        unrest: 10.0,
        base_logistics_capacity: 100.0,
        buildings: b.iter().copied().collect(),
        resource_deposits: vec![],
    }
}

fn country(
    label: u32,
    treasury: f64,
    tax: f32,
    avail: u64,
    mob: u64,
    states: &[u32],
) -> ScenarioCountryInput {
    ScenarioCountryInput {
        label,
        treasury,
        tax_rate: tax,
        available_manpower: avail,
        mobilized_manpower: mob,
        stockpile: [
            (ResourceType::Iron, 2000.0),
            (ResourceType::Coal, 2000.0),
            (ResourceType::Food, 50.0),
        ]
        .into_iter()
        .collect(),
        states: states.to_vec(),
        ai_controlled: false,
        military: None,
    }
}

fn multi(
    states: Vec<MapStateEconomyInput>,
    countries: Vec<ScenarioCountryInput>,
    months: u32,
) -> MultiMonthScenario {
    MultiMonthScenario {
        format_version: 1,
        description: "test".into(),
        months,
        countries,
        states,
    }
}

// ---------------------------------------------------------------- 状態のスナップショット

#[derive(Debug, Clone, PartialEq)]
struct StateSnap {
    population: u64,
    employed: u64,
    unemployed: u64,
    logistics: (f32, f32, f32),
    living_standard: f32,
    unrest: f32,
    ops: Vec<(BuildingType, f32)>,
}

#[derive(Debug, Clone, PartialEq)]
struct CountrySnap {
    treasury: f64,
    income: f64,
    expenses: f64,
    balance: f64,
    available_manpower: u64,
    stockpile: Vec<(ResourceType, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
struct MonthSnap {
    states: Vec<StateSnap>,
    countries: Vec<CountrySnap>,
}

fn ops_vec(m: &HashMap<BuildingType, f32>) -> Vec<(BuildingType, f32)> {
    BuildingType::ALL
        .iter()
        .filter_map(|b| m.get(b).map(|r| (*b, *r)))
        .collect()
}
fn stock_vec(s: &CountryStockpile) -> Vec<(ResourceType, f64)> {
    ResourceType::ALL.iter().map(|r| (*r, s.get(*r))).collect()
}

// ---------------------------------------------------------------- 新経路

fn run_new(sc: &MultiMonthScenario, b: &BuildingRegistry, w: &WorldModel) -> Vec<MonthSnap> {
    let (mut reg, mut countries) =
        MapStateEconomyRegistry::from_multi_scenario(sc, w, b).expect("valid scenario");
    let mut out = Vec::new();
    for _ in 0..sc.months {
        for c in countries.iter_mut() {
            reg.run_country_month(c, b);
        }
        out.push(MonthSnap {
            states: reg
                .iter()
                .map(|s| StateSnap {
                    population: s.population,
                    employed: s.employed_workforce,
                    unemployed: s.unemployed_workforce,
                    logistics: (s.logistics_capacity, s.logistics_usage, s.logistics_ratio),
                    living_standard: s.living_standard,
                    unrest: s.unrest,
                    ops: ops_vec(&s.building_operation_ratios),
                })
                .collect(),
            countries: countries.iter().map(country_snap_new).collect(),
        });
    }
    out
}

fn country_snap_new(c: &ScenarioCountry) -> CountrySnap {
    CountrySnap {
        treasury: c.treasury,
        income: c.monthly_income,
        expenses: c.monthly_expenses,
        balance: c.monthly_balance,
        available_manpower: c.available_manpower,
        stockpile: stock_vec(&c.stockpile),
    }
}

// ---------------------------------------------------------------- 旧経路(実際の旧Bevy System)

fn run_old(sc: &MultiMonthScenario, b: &BuildingRegistry) -> Vec<MonthSnap> {
    // 明示した対応表: 旧StateId(n+1) ↔ シナリオ順(MapStateId昇順)のステート。
    // 旧CountryId(i) ↔ シナリオの国家(i番目)。数値を新旧で流用・変換しない。
    let owner_of: HashMap<u32, usize> = sc
        .countries
        .iter()
        .enumerate()
        .flat_map(|(i, c)| c.states.iter().map(move |s| (*s, i)))
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
            resource_deposits: i.resource_deposits.clone(),
            ..StateData::default()
        })
        .collect();
    let countries: Vec<CountryData> = sc
        .countries
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut stockpile = CountryStockpile::new();
            for (r, a) in &c.stockpile {
                stockpile.set(*r, *a);
            }
            CountryData {
                id: CountryId(i),
                treasury: c.treasury,
                tax_rate: c.tax_rate,
                available_manpower: c.available_manpower,
                mobilized_manpower: c.mobilized_manpower,
                stockpile,
                ..CountryData::default()
            }
        })
        .collect();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<MonthChangedMessage>()
        .add_message::<GameNotification>()
        .insert_resource(CountryRegistry { countries })
        .insert_resource(StateRegistry::build(states))
        .insert_resource(BuildingRegistry {
            definitions: b.definitions.clone(),
        })
        .insert_resource(PlayerCountry(None))
        .insert_resource(CurrentLocale(Locale::JaJp))
        .insert_resource(TranslationCatalog::load().expect("catalog"))
        .add_systems(
            Update,
            handle_monthly_population_growth.before(strategy_game::economy::handle_monthly_economy),
        )
        .add_systems(Update, strategy_game::economy::handle_monthly_economy);
    app.update(); // 初期化

    let mut out = Vec::new();
    for m in 0..sc.months {
        app.world_mut().write_message(MonthChangedMessage {
            year: 1800 + (m / 12) as i32,
            month: (m % 12 + 1) as u8,
        });
        app.update();
        let sr = app.world().resource::<StateRegistry>();
        let cr = app.world().resource::<CountryRegistry>();
        out.push(MonthSnap {
            states: sr
                .states
                .iter()
                .map(|s| StateSnap {
                    population: s.population,
                    employed: s.employed_workforce,
                    unemployed: s.unemployed_workforce,
                    logistics: (s.logistics_capacity, s.logistics_usage, s.logistics_ratio),
                    living_standard: s.living_standard,
                    unrest: s.unrest,
                    ops: ops_vec(&s.building_operation_ratios),
                })
                .collect(),
            countries: cr
                .countries
                .iter()
                .map(|c| CountrySnap {
                    treasury: c.treasury,
                    income: c.monthly_income,
                    expenses: c.monthly_expenses,
                    balance: c.monthly_balance,
                    available_manpower: c.available_manpower,
                    stockpile: stock_vec(&c.stockpile),
                })
                .collect(),
        });
    }
    out
}

fn assert_trajectories_equal(sc: &MultiMonthScenario) {
    let (b, w) = (buildings(), world());
    let (old, new) = (run_old(sc, &b), run_new(sc, &b, &w));
    assert_eq!(old.len(), new.len());
    for (m, (o, n)) in old.iter().zip(&new).enumerate() {
        assert_eq!(o.states, n.states, "states differ at month {}", m + 1);
        assert_eq!(
            o.countries,
            n.countries,
            "countries differ at month {}",
            m + 1
        );
    }
}

// ---------------------------------------------------------------- 旧新一致(複数月)

#[test]
fn six_month_verification_scenario_matches_the_real_old_systems_every_month() {
    assert_trajectories_equal(&scenario());
}

#[test]
fn boundary_scenarios_match_the_real_old_systems_over_several_months() {
    let farm = |n| (BuildingType::Farm, n);
    let cases: Vec<MultiMonthScenario> = vec![
        // 人口ゼロ・施設ゼロ・失業者あり・動員ゼロ
        multi(
            vec![
                st(20, 0, 0.5, 50.0, &[farm(2)]),
                st(21, 300_000, 0.5, 50.0, &[]),
                st(22, 900_000, 0.6, 50.0, &[farm(1)]),
            ],
            vec![country(1, 100.0, 0.15, 0, 0, &[20, 21, 22])],
            4,
        ),
        // 高い動員負担(潜在労働力を超える)
        multi(
            vec![
                st(20, 100_000, 0.5, 50.0, &[farm(5)]),
                st(21, 100_000, 0.5, 50.0, &[farm(5)]),
            ],
            vec![country(1, 100.0, 0.15, 5_000, 500_000, &[20, 21])],
            3,
        ),
        // 複数国家・異なる入力・赤字国家・高税率・低税率
        multi(
            vec![
                st(
                    20,
                    700_000,
                    0.5,
                    80.0,
                    &[farm(4), (BuildingType::Factory, 1)],
                ),
                st(21, 50_000, 0.3, 20.0, &[farm(1)]),
                st(22, 123_457, 0.45, 5.0, &[(BuildingType::Mine, 1)]),
            ],
            vec![
                country(1, 50.0, 0.05, 10, 12_345, &[20]),
                country(2, -1_000.0, 0.40, 0, 0, &[21, 22]),
            ],
            6,
        ),
    ];
    for sc in &cases {
        assert_trajectories_equal(sc);
    }
}

// ---------------------------------------------------------------- 手計算による期待値

#[test]
fn single_month_values_match_hand_computation() {
    // 国家1 = {20: 人口600001, 21: 人口100000}, 国家2 = {22: 人口250000}
    let (b, w) = (buildings(), world());
    let sc = scenario();
    let (mut reg, mut cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap();
    let r1 = reg.run_country_month(&mut cs[0], &b);
    let r2 = reg.run_country_month(&mut cs[1], &b);
    // 人口成長(5bp/月、端数切り捨て): 600001 -> +300 (300.0005), 100000 -> +50, 250000 -> +125
    assert_eq!(reg.get(MapStateId(20)).unwrap().population, 600_301);
    assert_eq!(reg.get(MapStateId(21)).unwrap().population, 100_050);
    assert_eq!(reg.get(MapStateId(22)).unwrap().population, 250_125);
    // 人的資源の回復: 国家1 の所有人口 700351 -> 上限 70035 -> 回復 70035/120 = 583 -> 10583
    assert_eq!(r1.owned_population, 700_351);
    assert_eq!(cs[0].available_manpower, 10_583);
    // 国家2: 所有人口 250125 -> 上限 25012 -> 回復 25012/120 = 208
    assert_eq!(r2.owned_population, 250_125);
    assert_eq!(cs[1].available_manpower, 208);
    // 破産判定は月初の国庫: 国家1 は黒字側(500)、国家2 は負(-200)
    assert!(!r1.is_bankrupt && r2.is_bankrupt);
    // 税収(`人口 × 生活水準 × 税率 × 0.0001`): 国家2 = 250125 × 50 × 0.30 × 0.0001
    let tax2 = 250_125.0_f64 * 50.0 * 0.30_f32 as f64 * 0.0001;
    assert!(
        (r2.finance.income - tax2).abs() < 1e-9,
        "{} vs {}",
        r2.finance.income,
        tax2
    );
    // 国家2 の支出 = 物流維持費(容量100 × 0.05 = 5) + 行政費(250125 × 0.0008 = 200.1)。施設なし。
    assert!((r2.finance.expenses - (5.0 + 250_125.0 * 0.0008)).abs() < 1e-9);
    assert_eq!(cs[1].treasury, -200.0 + r2.finance.balance);
    // 国家への集計は各ステートを1回だけ: 国家1 の総人口 = 20 + 21 のみ(22 を含まない)
    assert_eq!(r1.finance.total_population, 600_301 + 100_050);
    assert_eq!(r2.finance.total_population, 250_125);
}

#[test]
fn simple_case_finance_matches_a_pencil_calculation() {
    // 施設なし・動員なし: 人口 100000 -> 100050。生活水準50、税率0.2。
    let (b, w) = (buildings(), world());
    let sc = multi(
        vec![st(20, 100_000, 0.5, 50.0, &[])],
        vec![country(1, 1_000.0, 0.2, 0, 0, &[20])],
        1,
    );
    let (mut reg, mut cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap();
    let r = reg.run_country_month(&mut cs[0], &b);
    // 税収 100050 × 50 × 0.2 × 0.0001(税率は既存どおりf32の0.2をf64へ拡張した値)、
    // 支出 = 物流 100 × 0.05 = 5.0 + 行政 100050 × 0.0008 = 80.04
    let income = 100_050.0_f64 * 50.0 * (0.2_f32 as f64) * 0.0001;
    assert!((r.finance.income - income).abs() < 1e-9);
    assert!((r.finance.income - 100.05).abs() < 1e-5, "about 100.05");
    assert!((r.finance.expenses - 85.04).abs() < 1e-9);
    assert!((r.finance.balance - (income - 85.04)).abs() < 1e-9);
    assert!((cs[0].treasury - (1_000.0 + income - 85.04)).abs() < 1e-9);
    // 福祉(既存の式): 食料あり(+0.5)、経済用労働力比率 0 < 0.7(生活水準 -0.8・不満度 +0.8)、
    // 物流比率 1.0 >= 0.9(+0.3)、税率 0.2 は中立 → 生活水準 +0.0(50 のまま)、不満度 10.0 + 0.8 = 10.8
    let s = reg.get(MapStateId(20)).unwrap();
    assert!(
        (s.living_standard - 50.0).abs() < 1e-6,
        "{}",
        s.living_standard
    );
    assert!((s.unrest - 10.8).abs() < 1e-5, "{}", s.unrest);
}

#[test]
fn zero_income_case_and_unreachable_zero_expense() {
    let (b, w) = (buildings(), world());
    // 収入ゼロ(税率0・人口0)。支出は物流維持費(容量100 × 0.05 = 5.0)だけ。
    // なお支出ゼロは旧経路では到達できない: `StateRegistry::build`の`sanitize_values`が
    // 物流容量0を100へ置き換えるため(新経路の入力検証も0を拒否する。別テスト参照)。
    let sc = multi(
        vec![st(20, 0, 0.5, 50.0, &[])],
        vec![country(1, 123.0, 0.0, 0, 0, &[20])],
        2,
    );
    let (mut reg, mut cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap();
    let mut treasury = 123.0;
    for _ in 0..2 {
        let r = reg.run_country_month(&mut cs[0], &b);
        assert_eq!(r.finance.income, 0.0);
        assert!((r.finance.expenses - 5.0).abs() < 1e-12);
        treasury -= 5.0;
    }
    assert!((cs[0].treasury - treasury).abs() < 1e-9);
    // 旧経路とも一致
    assert_trajectories_equal(&sc);
}

// ---------------------------------------------------------------- 重複更新の防止

#[test]
fn each_state_grows_once_and_province_count_does_not_amplify() {
    let (b, w) = (buildings(), world());
    // 20(7プロヴィンス)と21(13プロヴィンス)に同じ入力。別国家にして独立に更新する。
    let sc = multi(
        vec![
            st(20, 400_000, 0.5, 50.0, &[]),
            st(21, 400_000, 0.5, 50.0, &[]),
        ],
        vec![
            country(1, 0.0, 0.1, 0, 0, &[20]),
            country(2, 0.0, 0.1, 0, 0, &[21]),
        ],
        1,
    );
    assert_ne!(
        w.provinces_of(MapStateId(20)).unwrap().len(),
        w.provinces_of(MapStateId(21)).unwrap().len()
    );
    let (mut reg, mut cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap();
    // 国家1だけを処理すると、国家2のステートは一切変わらない(他国家のステートを重複更新しない)
    reg.run_country_month(&mut cs[0], &b);
    assert_eq!(
        reg.get(MapStateId(20)).unwrap().population,
        grow_state_population(400_000)
    );
    assert_eq!(
        reg.get(MapStateId(21)).unwrap().population,
        400_000,
        "other country's state untouched"
    );
    assert_eq!(cs[1].available_manpower, 0);
    reg.run_country_month(&mut cs[1], &b);
    let (a, c) = (
        reg.get(MapStateId(20)).unwrap(),
        reg.get(MapStateId(21)).unwrap(),
    );
    assert_eq!(a.population, c.population);
    assert_eq!(a.population, 400_200, "400000 + 400000*5/10000, once");
    assert_eq!((a.living_standard, a.unrest), (c.living_standard, c.unrest));
    assert_eq!(
        cs[0].monthly_income, cs[1].monthly_income,
        "same inputs -> same finance regardless of province count"
    );
}

#[test]
fn reads_do_not_change_state_and_runs_are_deterministic() {
    let (b, w) = (buildings(), world());
    let sc = scenario();
    let (reg, cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap();
    let (before_r, before_c) = (reg.clone(), cs.clone());
    for _ in 0..3 {
        let _ = reg.get(MapStateId(20));
        let _ = reg.iter().count();
    }
    assert_eq!((&reg, &cs), (&before_r, &before_c));
    // 同じ入力から2回独立に実行 → 同じ結果
    assert_eq!(run_new(&sc, &b, &w), run_new(&sc, &b, &w));
}

#[test]
fn shared_manpower_recovery_matches_the_old_formula() {
    // 旧の式(基準上限=人口の10%、回復=上限/120、最低1、不足を超えない)を手計算と比較
    assert_eq!(
        recover_country_manpower(0, 0),
        0,
        "no owned population: unchanged"
    );
    assert_eq!(
        recover_country_manpower(5, 100),
        6,
        "cap 10, recovery max(10/120,1) = 1"
    );
    assert_eq!(recover_country_manpower(10_000, 700_351), 10_583);
    assert_eq!(
        recover_country_manpower(70_035, 700_351),
        70_035,
        "at cap: unchanged"
    );
}

// ---------------------------------------------------------------- 国家への対応づけの検証

#[test]
fn invalid_country_mapping_and_values_are_rejected() {
    let (b, w) = (buildings(), world());
    let ok_states = || vec![st(20, 1000, 0.5, 50.0, &[]), st(21, 1000, 0.5, 50.0, &[])];
    let err = |sc: MultiMonthScenario| {
        MapStateEconomyRegistry::from_multi_scenario(&sc, &w, &b).unwrap_err()
    };
    let has_mapping = |e: &[EconomyDataIssue]| {
        e.iter()
            .any(|i| matches!(i, EconomyDataIssue::InvalidCountryMapping { .. }))
    };
    // 1つのステートが2つの国家に属する(二重集計)
    assert!(has_mapping(&err(multi(
        ok_states(),
        vec![
            country(1, 0.0, 0.1, 0, 0, &[20, 21]),
            country(2, 0.0, 0.1, 0, 0, &[21])
        ],
        1
    ))));
    // どの国家にも属さないステート(計算されない)
    assert!(has_mapping(&err(multi(
        ok_states(),
        vec![country(1, 0.0, 0.1, 0, 0, &[20])],
        1
    ))));
    // 経済データのないステートへの参照
    assert!(has_mapping(&err(multi(
        ok_states(),
        vec![country(1, 0.0, 0.1, 0, 0, &[20, 21, 22])],
        1
    ))));
    // 重複した国家ラベル
    assert!(has_mapping(&err(multi(
        ok_states(),
        vec![
            country(1, 0.0, 0.1, 0, 0, &[20]),
            country(1, 0.0, 0.1, 0, 0, &[21])
        ],
        1
    ))));
    // 存在しないステート・重複したステート
    assert!(
        err(multi(
            vec![st(99, 1, 0.5, 50.0, &[])],
            vec![country(1, 0.0, 0.1, 0, 0, &[99])],
            1
        ))
        .contains(&EconomyDataIssue::UnknownState {
            state: MapStateId(99)
        })
    );
    assert!(
        err(multi(
            vec![st(20, 1, 0.5, 50.0, &[]), st(20, 1, 0.5, 50.0, &[])],
            vec![country(1, 0.0, 0.1, 0, 0, &[20])],
            1
        ))
        .contains(&EconomyDataIssue::DuplicateState {
            state: MapStateId(20)
        })
    );
    // 不正値: 生活水準・不満度の範囲外、税率の範囲外、国庫が非有限
    for bad in [
        multi(
            vec![st(20, 1, 0.5, 120.0, &[])],
            vec![country(1, 0.0, 0.1, 0, 0, &[20])],
            1,
        ),
        multi(
            vec![st(20, 1, 0.5, f32::NAN, &[])],
            vec![country(1, 0.0, 0.1, 0, 0, &[20])],
            1,
        ),
        multi(
            vec![st(20, 1, 0.5, 50.0, &[])],
            vec![country(1, 0.0, 1.5, 0, 0, &[20])],
            1,
        ),
        multi(
            vec![st(20, 1, 0.5, 0.5, &[])],
            vec![country(1, 0.0, 0.1, 0, 0, &[20])],
            1,
        ),
        multi(
            vec![st(20, 1, 0.5, 50.0, &[])],
            vec![country(1, f64::INFINITY, 0.1, 0, 0, &[20])],
            1,
        ),
    ] {
        assert!(!err(bad).is_empty());
    }
    let mut unrest_bad = st(20, 1, 0.5, 50.0, &[]);
    unrest_bad.unrest = -1.0;
    assert!(
        !err(multi(
            vec![unrest_bad],
            vec![country(1, 0.0, 0.1, 0, 0, &[20])],
            1
        ))
        .is_empty()
    );
    // 旧経路の読み込み時正規化と食い違う曖昧な値: 物流容量0(旧は100へ置換)、不満度 (0,1](旧は100倍)
    let mut zero_logistics = st(20, 1, 0.5, 50.0, &[]);
    zero_logistics.base_logistics_capacity = 0.0;
    assert!(
        !err(multi(
            vec![zero_logistics],
            vec![country(1, 0.0, 0.1, 0, 0, &[20])],
            1
        ))
        .is_empty()
    );
    let mut legacy_unrest = st(20, 1, 0.5, 50.0, &[]);
    legacy_unrest.unrest = 1.0;
    assert!(
        !err(multi(
            vec![legacy_unrest],
            vec![country(1, 0.0, 0.1, 0, 0, &[20])],
            1
        ))
        .is_empty()
    );
}

#[test]
fn scenario_file_matches_world_and_declares_its_purpose() {
    let sc = scenario();
    assert!(sc.description.contains("not official"));
    assert_eq!(sc.months, 6);
    let (reg, cs) =
        MapStateEconomyRegistry::from_multi_scenario(&sc, &world(), &buildings()).unwrap();
    assert_eq!(
        reg.iter().map(|s| s.state.0).collect::<Vec<_>>(),
        vec![20, 21, 22]
    );
    assert_eq!(
        cs.iter().map(|c| c.states.len()).collect::<Vec<_>>(),
        vec![2, 1]
    );
    let text = std::fs::read_to_string(SCENARIO)
        .unwrap()
        .replace("format_version: 1", "format_version: 5");
    assert!(
        parse_multi_month_scenario(&text)
            .unwrap_err()
            .contains("unsupported format_version 5")
    );
}
