//! P32-007 (M4-A): 新ステート(`MapStateId`)の人口・雇用・物流・生産施設が、旧`StateData`と
//! **同じ中核計算**を通ることのテスト。旧`StateId`と`MapStateId`は同じ数値でも相互変換せず、
//! 比較するケースでは対応表をテスト内に明示する。入力は検証用シナリオで、正式な配置・バランスではない。

use std::collections::HashMap;
use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::StateId;
use strategy_game::economy::map_state::{
    EconomyDataIssue, EconomyScenario, MapStateEconomyInput, MapStateEconomyRegistry,
    parse_economy_scenario, stockpile_from_scenario,
};
use strategy_game::economy::production::process_country_production_with_outcome;
use strategy_game::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use strategy_game::economy::state_access::{
    StateEconomyAccess, update_monthly_workforce_and_logistics,
};
use strategy_game::hexmap::data::load_from_path;
use strategy_game::population::mobilization::{
    apportion_mobilized_manpower, apportion_mobilized_manpower_by,
};
use strategy_game::state::data::StateData;
use strategy_game::world::{MapStateId, WorldModel, load_world_preview};

const SCENARIO: &str = "assets/data/world/preview/economy_scenario.ron";

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

fn scenario() -> EconomyScenario {
    parse_economy_scenario(&std::fs::read_to_string(SCENARIO).unwrap()).unwrap()
}

fn input(state: u32, pop: u64, ratio: f32, b: &[(BuildingType, u32)]) -> MapStateEconomyInput {
    MapStateEconomyInput {
        state,
        population: pop,
        workforce_ratio: ratio,
        base_logistics_capacity: 100.0,
        buildings: b.iter().copied().collect(),
        resource_deposits: vec![],
    }
}

fn scenario_with(states: Vec<MapStateEconomyInput>, mobilized: u64) -> EconomyScenario {
    EconomyScenario {
        format_version: 1,
        description: "test".into(),
        country_mobilized_manpower: mobilized,
        is_bankrupt: false,
        stockpile: [(ResourceType::Iron, 2000.0), (ResourceType::Coal, 2000.0)]
            .into_iter()
            .collect(),
        states,
    }
}

/// 旧`StateData`で同じ入力を表す(明示した対応表: 旧StateId(i) ↔ MapStateId)。
fn old_state(old_id: usize, i: &MapStateEconomyInput) -> StateData {
    StateData {
        id: StateId(old_id),
        population: i.population,
        workforce_ratio: i.workforce_ratio,
        base_logistics_capacity: i.base_logistics_capacity,
        logistics_capacity: i.base_logistics_capacity,
        buildings: i.buildings.clone(),
        resource_deposits: i.resource_deposits.clone(),
        ..StateData::default()
    }
}

struct Run {
    employed: Vec<u64>,
    unemployed: Vec<u64>,
    logistics: Vec<(f32, f32, f32)>,
    ops: Vec<Vec<(BuildingType, f32)>>,
    stock: Vec<(ResourceType, f64)>,
    science: f64,
    magic: f64,
    supplies: Option<f64>,
}

fn sorted_ops(m: &HashMap<BuildingType, f32>) -> Vec<(BuildingType, f32)> {
    BuildingType::ALL
        .iter()
        .filter_map(|b| m.get(b).map(|r| (*b, *r)))
        .collect()
}
fn stock_vec(s: &CountryStockpile) -> Vec<(ResourceType, f64)> {
    ResourceType::ALL.iter().map(|r| (*r, s.get(*r))).collect()
}

/// 旧経路: 旧`StateData` + 旧公開API(`update_state_population_and_employment`・
/// `update_state_logistics`・`apportion_mobilized_manpower`・`process_country_production_with_outcome`)。
fn run_old(sc: &EconomyScenario, reg: &BuildingRegistry) -> Run {
    let mut states: Vec<StateData> = sc
        .states
        .iter()
        .enumerate()
        .map(|(n, i)| old_state(n + 1, i))
        .collect();
    let pairs: Vec<(StateId, u64)> = states.iter().map(|s| (s.id, s.population)).collect();
    let burdens = apportion_mobilized_manpower(sc.country_mobilized_manpower, &pairs);
    for s in states.iter_mut() {
        let b = burdens.get(&s.id).copied().unwrap_or(0);
        strategy_game::population::update::update_state_population_and_employment(s, reg, b);
        strategy_game::logistics::update::update_state_logistics(s, reg);
    }
    let mut stock = stockpile_from_scenario(sc);
    let mut refs: Vec<&mut StateData> = states.iter_mut().collect();
    let out = process_country_production_with_outcome(&mut refs, &mut stock, reg, sc.is_bankrupt);
    Run {
        employed: states.iter().map(|s| s.employed_workforce).collect(),
        unemployed: states.iter().map(|s| s.unemployed_workforce).collect(),
        logistics: states
            .iter()
            .map(|s| (s.logistics_capacity, s.logistics_usage, s.logistics_ratio))
            .collect(),
        ops: states
            .iter()
            .map(|s| sorted_ops(&s.building_operation_ratios))
            .collect(),
        stock: stock_vec(&stock),
        science: out.science,
        magic: out.magic,
        supplies: out.military_supplies_produced,
    }
}

/// 新経路: `MapStateEconomyRegistry`(`MapStateId`キー)。
fn run_new(sc: &EconomyScenario, reg: &BuildingRegistry, w: &WorldModel) -> Run {
    let mut r = MapStateEconomyRegistry::from_scenario(sc, w, reg).expect("valid scenario");
    let mut stock = stockpile_from_scenario(sc);
    let out = r.run_month(
        sc.country_mobilized_manpower,
        &mut stock,
        reg,
        sc.is_bankrupt,
    );
    let v: Vec<_> = r.iter().collect();
    Run {
        employed: v.iter().map(|s| s.employed_workforce).collect(),
        unemployed: v.iter().map(|s| s.unemployed_workforce).collect(),
        logistics: v
            .iter()
            .map(|s| (s.logistics_capacity, s.logistics_usage, s.logistics_ratio))
            .collect(),
        ops: v
            .iter()
            .map(|s| sorted_ops(&s.building_operation_ratios))
            .collect(),
        stock: stock_vec(&stock),
        science: out.science,
        magic: out.magic,
        supplies: out.military_supplies_produced,
    }
}

fn assert_same(a: &Run, b: &Run) {
    assert_eq!(a.employed, b.employed, "employed");
    assert_eq!(a.unemployed, b.unemployed, "unemployed");
    assert_eq!(a.logistics, b.logistics, "logistics");
    assert_eq!(a.ops, b.ops, "operation ratios");
    assert_eq!(a.stock, b.stock, "stockpile after production");
    assert_eq!(
        (a.science, a.magic, a.supplies),
        (b.science, b.magic, b.supplies)
    );
}

// ---------------------------------------------------------------- シナリオとデータの所属

#[test]
fn scenario_file_loads_and_maps_to_existing_world_states() {
    let w = world();
    let sc = scenario();
    let reg = MapStateEconomyRegistry::from_scenario(&sc, &w, &buildings()).unwrap();
    let ids: Vec<u32> = reg.iter().map(|s| s.state.0).collect();
    assert_eq!(ids, vec![20, 21, 22], "ascending MapStateId order");
    for s in reg.iter() {
        assert!(w.state(s.state).is_some());
    }
    assert!(sc.description.contains("validation scenario"));
}

#[test]
fn unknown_duplicate_and_invalid_inputs_are_rejected() {
    let w = world();
    let b = buildings();
    let ok = input(20, 1000, 0.5, &[(BuildingType::Farm, 1)]);
    let err = |states: Vec<MapStateEconomyInput>| {
        MapStateEconomyRegistry::from_scenario(&scenario_with(states, 0), &w, &b).unwrap_err()
    };
    // 存在しないステート
    assert!(
        err(vec![input(99, 1000, 0.5, &[])]).contains(&EconomyDataIssue::UnknownState {
            state: MapStateId(99)
        })
    );
    // 同じステートの重複定義
    assert!(
        err(vec![ok.clone(), ok.clone()]).contains(&EconomyDataIssue::DuplicateState {
            state: MapStateId(20)
        })
    );
    // 不正値: 労働力比率が範囲外・非有限、物流が負・非有限、鉱床の産出が負
    for ratio in [1.5f32, -0.1, f32::NAN, f32::INFINITY] {
        let issues = err(vec![input(20, 1000, ratio, &[])]);
        assert!(
            issues.iter().any(|i| matches!(
                i,
                EconomyDataIssue::InvalidValue {
                    state: Some(MapStateId(20)),
                    ..
                }
            )),
            "{ratio}"
        );
    }
    let mut bad = ok.clone();
    bad.base_logistics_capacity = -1.0;
    assert!(!err(vec![bad]).is_empty());
    let mut bad = ok.clone();
    bad.resource_deposits = vec![StateResourceDeposit {
        resource_type: ResourceType::Iron,
        base_output: f64::NAN,
        discovered: true,
        development_level: 1,
    }];
    assert!(!err(vec![bad]).is_empty());
    // 未定義の施設種別
    let mut reg_missing = buildings();
    reg_missing.definitions.remove(&BuildingType::Farm);
    let e = MapStateEconomyRegistry::from_scenario(&scenario_with(vec![ok], 0), &w, &reg_missing)
        .unwrap_err();
    assert!(e.contains(&EconomyDataIssue::UnknownBuilding {
        state: MapStateId(20),
        building: BuildingType::Farm
    }));
    // 不正な備蓄
    let mut sc = scenario_with(vec![], 0);
    sc.stockpile.insert(ResourceType::Food, -5.0);
    assert!(MapStateEconomyRegistry::from_scenario(&sc, &w, &b).is_err());
    // 非対応のバージョン
    let text = std::fs::read_to_string(SCENARIO)
        .unwrap()
        .replace("format_version: 1", "format_version: 9");
    assert!(
        parse_economy_scenario(&text)
            .unwrap_err()
            .contains("unsupported format_version 9")
    );
}

// ---------------------------------------------------------------- 旧経路との一致

#[test]
fn verification_scenario_gives_identical_results_through_old_and_new_paths() {
    let (sc, b, w) = (scenario(), buildings(), world());
    assert_same(&run_old(&sc, &b), &run_new(&sc, &b, &w));
}

#[test]
fn employment_and_logistics_match_hand_computed_values() {
    // 手計算(旧経路のコードを使わない): 動員 30000 を人口 600k/100k/250k で按分(最大剰余法)
    //   → 18947/3157/7894 の合計 29998、残り2人は剰余の大きい順(21: .89, 22: .74)へ → 18947/3158/7895。
    // 20: 潜在 300000 - 18947 = 281053、必要 3*5000+2*10000+20000+3000 = 58000 → 雇用 58000 / 失業 223053
    // 21: 潜在 40000 - 3158 = 36842、必要 6*5000+20000 = 50000 > 36842 → 雇用 36842 / 失業 0
    // 22: 潜在 112500 - 7895 = 104605、施設なし → 雇用 0 / 失業 104605
    let (sc, b, w) = (scenario(), buildings(), world());
    let r = run_new(&sc, &b, &w);
    assert_eq!(r.employed, vec![58_000, 36_842, 0]);
    assert_eq!(r.unemployed, vec![223_053, 0, 104_605]);
    // 物流: 20 = 100 + 50(鉄道1) = 150、使用 = 3*2 + 2*10 + 15 + 0 = 41。22 は施設なしで使用 0。
    assert_eq!((r.logistics[0].0, r.logistics[0].1), (150.0, 41.0));
    assert_eq!(
        (r.logistics[2].0, r.logistics[2].1, r.logistics[2].2),
        (100.0, 0.0, 1.0)
    );
    // 21: 容量 60、使用 6*2 + 15 = 27
    assert_eq!((r.logistics[1].0, r.logistics[1].1), (60.0, 27.0));
}

#[test]
fn boundary_cases_match_the_old_path_exactly() {
    let (b, w) = (buildings(), world());
    let farm = |n| (BuildingType::Farm, n);
    let cases: Vec<(&str, Vec<MapStateEconomyInput>, u64)> = vec![
        ("zero population", vec![input(20, 0, 0.5, &[farm(2)])], 0),
        ("zero facilities", vec![input(20, 500_000, 0.5, &[])], 0),
        (
            "unemployed present",
            vec![input(20, 800_000, 0.6, &[farm(1)])],
            0,
        ),
        (
            "zero mobilization",
            vec![
                input(20, 90_000, 0.5, &[farm(3)]),
                input(21, 60_000, 0.5, &[farm(2)]),
            ],
            0,
        ),
        (
            "high mobilization (burden exceeds workforce)",
            vec![
                input(20, 100_000, 0.5, &[farm(5)]),
                input(21, 100_000, 0.5, &[farm(5)]),
            ],
            500_000,
        ),
        (
            "different inputs across states",
            vec![
                input(20, 700_000, 0.5, &[farm(4), (BuildingType::Factory, 1)]),
                input(21, 50_000, 0.3, &[farm(1)]),
                input(22, 0, 0.5, &[]),
            ],
            12_345,
        ),
    ];
    for (name, states, m) in cases {
        let sc = scenario_with(states, m);
        let (old, new) = (run_old(&sc, &b), run_new(&sc, &b, &w));
        assert_same(&old, &new);
        let _ = name;
    }
    // 人口ゼロ・動員ゼロの具体値(既存仕様)
    let sc = scenario_with(vec![input(20, 0, 0.5, &[(BuildingType::Farm, 2)])], 0);
    let r = run_new(&sc, &b, &w);
    assert_eq!((r.employed[0], r.unemployed[0]), (0, 0));
    // 高い動員: 負担が潜在労働力を超えても民間労働力は0で止まる(saturating)
    let sc = scenario_with(
        vec![input(20, 100_000, 0.5, &[(BuildingType::Farm, 5)])],
        500_000,
    );
    let r = run_new(&sc, &b, &w);
    assert_eq!((r.employed[0], r.unemployed[0]), (0, 0));
}

#[test]
fn shared_apportionment_matches_the_old_state_id_function() {
    let pairs_new = [(20u64, 600_000u64), (21, 100_000), (22, 250_000)];
    let pairs_old = [
        (StateId(20), 600_000u64),
        (StateId(21), 100_000),
        (StateId(22), 250_000),
    ];
    for m in [0u64, 1, 2, 29_998, 30_000, 999_999, 5_000_000] {
        let a = apportion_mobilized_manpower_by(m, &pairs_new, |k| k);
        let b = apportion_mobilized_manpower(m, &pairs_old);
        for ((k, _), (s, _)) in pairs_new.iter().zip(&pairs_old) {
            assert_eq!(a[k], b[s], "m={m}");
        }
        assert_eq!(a.values().sum::<u64>(), if m == 0 { 0 } else { m });
    }
}

// ---------------------------------------------------------------- 重複計上の防止

#[test]
fn economy_data_is_per_state_not_multiplied_by_province_count() {
    let (b, w) = (buildings(), world());
    // 同じ入力を、所属プロヴィンス数が異なる2ステート(20=7個、21=13個)へ与える。
    let i20 = input(
        20,
        400_000,
        0.5,
        &[(BuildingType::Farm, 2), (BuildingType::Mine, 1)],
    );
    let i21 = MapStateEconomyInput {
        state: 21,
        ..i20.clone()
    };
    let sc = scenario_with(vec![i20, i21], 0);
    assert_ne!(
        w.provinces_of(MapStateId(20)).unwrap().len(),
        w.provinces_of(MapStateId(21)).unwrap().len()
    );
    let r = run_new(&sc, &b, &w);
    assert_eq!(
        r.employed[0], r.employed[1],
        "province count must not change employment"
    );
    assert_eq!(r.unemployed[0], r.unemployed[1]);
    assert_eq!(r.logistics[0], r.logistics[1]);
    // 雇用は施設の必要労働力ちょうど(2*5000 + 1*10000)で、プロヴィンス数倍にならない
    assert_eq!(r.employed[0], 20_000);
    // 人口・施設はMapStateEconomyに1回だけ保持される
    let reg = MapStateEconomyRegistry::from_scenario(&sc, &w, &b).unwrap();
    assert_eq!(reg.get(MapStateId(20)).unwrap().population, 400_000);
    assert_eq!(
        reg.get(MapStateId(20)).unwrap().buildings[&BuildingType::Farm],
        2
    );
}

#[test]
fn repeated_lookups_and_cloned_runs_do_not_change_state() {
    let (sc, b, w) = (scenario(), buildings(), world());
    let reg = MapStateEconomyRegistry::from_scenario(&sc, &w, &b).unwrap();
    let before = reg.clone();
    for _ in 0..5 {
        for id in [20, 21, 22, 99] {
            let _ = reg.get(MapStateId(id));
        }
        let _ = reg.iter().count();
    }
    assert_eq!(reg, before, "reads do not mutate");
    // 同じ入力から2回独立に実行しても結果は同じ(決定的)
    let (mut a, mut c) = (reg.clone(), reg.clone());
    let (mut sa, mut sc2) = (stockpile_from_scenario(&sc), stockpile_from_scenario(&sc));
    let oa = a.run_month(sc.country_mobilized_manpower, &mut sa, &b, false);
    let oc = c.run_month(sc.country_mobilized_manpower, &mut sc2, &b, false);
    assert_eq!(a, c);
    assert_eq!(stock_vec(&sa), stock_vec(&sc2));
    assert_eq!((oa.science, oa.magic), (oc.science, oc.magic));
}

#[test]
fn shared_core_is_used_by_both_state_kinds() {
    // 同じ関数(`update_monthly_workforce_and_logistics`)を、旧StateDataと新MapStateEconomyの
    // 両方へ直接適用して一致することを確認する(トレイト経由の共通実装)。
    let (sc, b, w) = (scenario(), buildings(), world());
    let mut old: Vec<StateData> = sc
        .states
        .iter()
        .enumerate()
        .map(|(n, i)| old_state(n + 1, i))
        .collect();
    let mut oref: Vec<&mut StateData> = old.iter_mut().collect();
    update_monthly_workforce_and_logistics(&mut oref, sc.country_mobilized_manpower, &b);
    let mut new = MapStateEconomyRegistry::from_scenario(&sc, &w, &b).unwrap();
    // run_month は生産まで行うため、雇用・物流のみの比較は run 後の値(生産は雇用・物流を変えない)で行う
    let mut stock = stockpile_from_scenario(&sc);
    new.run_month(sc.country_mobilized_manpower, &mut stock, &b, false);
    for (o, n) in old.iter().zip(new.iter()) {
        assert_eq!(o.employed_workforce(), n.employed_workforce());
        assert_eq!(o.unemployed_workforce(), n.unemployed_workforce());
        assert_eq!(o.logistics_ratio(), n.logistics_ratio());
        assert_eq!(o.total_workforce(), n.total_workforce());
        assert_eq!(o.civilian_workforce(), n.civilian_workforce());
    }
}

#[test]
fn old_and_new_ids_are_distinct_types() {
    use std::any::TypeId;
    assert_ne!(TypeId::of::<StateId>(), TypeId::of::<MapStateId>());
    // 比較の対応表はテスト内で明示している: 旧StateId(n+1) ↔ シナリオ順の MapStateId。数値は流用しない。
}
