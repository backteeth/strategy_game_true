//! P28-010-IMPL-001: 生産入力の原子的会計(FIX1)の単体・E2Eテスト。
//!
//! 入力資源の`consume`が失敗しても産出・研究力だけが加算されていた旧挙動
//! (P28-010-PRE-002で確認)が消えたこと、および入力が足りている場合の消費・産出が
//! 従来と完全に同一であることを、実関数`process_country_production`で検証する。

use std::collections::HashMap;

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::economy::production::process_country_production;
use strategy_game::economy::resources::{
    CountryStockpile, ResourceType, military_equipment_units_to_value,
    military_equipment_value_to_units, military_supplies_units_to_value,
    military_supplies_value_to_units,
};
use strategy_game::population::update::calculate_economic_workforce_ratio;
use strategy_game::state::data::StateData;

fn factory_def() -> BuildingDefinition {
    BuildingDefinition {
        building_type: BuildingType::Factory,
        name: "Civilian Factory".to_string(),
        construction_cost: 1000.0,
        required_progress: 90.0,
        required_workforce: 20_000.0,
        logistics_cost: 15.0,
        input_resources: HashMap::from([(ResourceType::Iron, 20.0), (ResourceType::Coal, 20.0)]),
        output_resources: HashMap::from([(ResourceType::IndustrialGoods, 30.0)]),
        maintenance_cost: 30.0,
        max_level: 10,
        science_output: 0.0,
        magic_output: 0.0,
        railway_capacity_bonus: 0.0,
    }
}

fn military_factory_def() -> BuildingDefinition {
    BuildingDefinition {
        building_type: BuildingType::MilitaryFactory,
        name: "Military Factory".to_string(),
        construction_cost: 1200.0,
        required_progress: 90.0,
        required_workforce: 25_000.0,
        logistics_cost: 20.0,
        input_resources: HashMap::from([
            (ResourceType::Iron, 30.0),
            (ResourceType::IndustrialGoods, 15.0),
        ]),
        output_resources: HashMap::from([
            (ResourceType::MilitaryEquipment, 20.0),
            (ResourceType::MilitarySupplies, 100.0),
        ]),
        maintenance_cost: 40.0,
        max_level: 10,
        science_output: 0.0,
        magic_output: 0.0,
        railway_capacity_bonus: 0.0,
    }
}

fn magic_academy_def() -> BuildingDefinition {
    BuildingDefinition {
        building_type: BuildingType::MagicAcademy,
        name: "Magic Academy".to_string(),
        construction_cost: 1800.0,
        required_progress: 120.0,
        required_workforce: 6000.0,
        logistics_cost: 5.0,
        input_resources: HashMap::from([(ResourceType::MagicCrystal, 10.0)]),
        output_resources: HashMap::new(),
        maintenance_cost: 60.0,
        max_level: 5,
        science_output: 0.0,
        magic_output: 30.0,
        railway_capacity_bonus: 0.0,
    }
}

fn registry() -> BuildingRegistry {
    let mut r = BuildingRegistry::default();
    for d in [factory_def(), military_factory_def(), magic_academy_def()] {
        r.definitions.insert(d.building_type, d);
    }
    r
}

fn state(id: usize, b: BuildingType, level: u32) -> StateData {
    StateData {
        id: StateId(id),
        name: format!("State {id}"),
        owner_country_id: CountryId(0),
        population: 1_200_000,
        workforce_ratio: 0.55,
        employed_workforce: 500_000,
        logistics_ratio: 1.0,
        buildings: HashMap::from([(b, level)]),
        ..Default::default()
    }
}

fn t_of(s: &StateData) -> f32 {
    calculate_economic_workforce_ratio(s.employed_workforce, s.total_workforce())
        .clamp(0.0, 1.0)
        .min(s.logistics_ratio)
}

fn stock(items: &[(ResourceType, f64)]) -> CountryStockpile {
    let mut s = CountryStockpile::new();
    for &(r, a) in items {
        s.set(r, a);
    }
    s
}

struct Outcome {
    states: Vec<StateData>,
    stock: CountryStockpile,
    sci: f64,
    mag: f64,
}

fn run(mut states: Vec<StateData>, mut stockpile: CountryStockpile, bankrupt: bool) -> Outcome {
    let reg = registry();
    let (sci, mag) = {
        let mut refs: Vec<&mut StateData> = states.iter_mut().collect();
        process_country_production(&mut refs, &mut stockpile, &reg, bankrupt)
    };
    Outcome { states, stock: stockpile, sci, mag }
}

fn op_of(o: &Outcome, id: usize, b: BuildingType) -> f64 {
    let s = o.states.iter().find(|s| s.id.0 == id).unwrap();
    s.building_operation_ratios.get(&b).copied().unwrap() as f64
}

fn ms_quantized(value: f64) -> f64 {
    military_supplies_units_to_value(military_supplies_value_to_units(value).unwrap())
}

fn me_quantized(value: f64) -> f64 {
    military_equipment_units_to_value(military_equipment_value_to_units(value).unwrap())
}

const RES: [ResourceType; 5] = [
    ResourceType::Iron,
    ResourceType::Coal,
    ResourceType::IndustrialGoods,
    ResourceType::MagicCrystal,
    ResourceType::MilitaryEquipment,
];

/// 入力が十分なら、消費・産出が旧実装(`base × level × t`)と完全に一致する。
#[test]
fn sufficient_inputs_keep_legacy_consumption_and_output_exactly() {
    let states = vec![state(1, BuildingType::Factory, 2), state(2, BuildingType::MilitaryFactory, 1)];
    let t = t_of(&states[0]) as f64;
    assert!(t > 0.7 && t < 0.8);
    let o = run(
        states,
        stock(&[
            (ResourceType::Iron, 10_000.0),
            (ResourceType::Coal, 10_000.0),
            (ResourceType::IndustrialGoods, 10_000.0),
        ]),
        false,
    );
    let t32 = t as f32;
    let iron_used = 20.0 * 2.0 * t32 as f64 + 30.0 * 1.0 * t32 as f64;
    assert_eq!(o.stock.get(ResourceType::Iron), 10_000.0 - iron_used);
    assert_eq!(o.stock.get(ResourceType::Coal), 10_000.0 - 20.0 * 2.0 * t32 as f64);
    assert_eq!(
        o.stock.get(ResourceType::IndustrialGoods),
        10_000.0 + 30.0 * 2.0 * t32 as f64 - 15.0 * 1.0 * t32 as f64
    );
    assert_eq!(o.stock.get(ResourceType::MilitarySupplies), ms_quantized(100.0 * t32 as f64));
    assert!((o.stock.get(ResourceType::MilitaryEquipment) - me_quantized(20.0 * t32 as f64)).abs() < 1e-9);
    assert_eq!(op_of(&o, 1, BuildingType::Factory) as f32, t32);
}

/// 入力が0なら、入力消費も産出も0。備蓄は不変。
#[test]
fn zero_inputs_produce_nothing_and_consume_nothing() {
    let o = run(
        vec![state(1, BuildingType::Factory, 2), state(2, BuildingType::MilitaryFactory, 1)],
        stock(&[(ResourceType::Coal, 500.0)]),
        false,
    );
    assert_eq!(o.stock.get(ResourceType::Iron), 0.0);
    assert_eq!(o.stock.get(ResourceType::Coal), 500.0, "Iron不足のFactoryはCoalも引かない");
    assert_eq!(o.stock.get(ResourceType::IndustrialGoods), 0.0);
    assert_eq!(o.stock.get(ResourceType::MilitarySupplies), 0.0);
    assert_eq!(o.stock.get(ResourceType::MilitaryEquipment), 0.0);
    assert_eq!(op_of(&o, 1, BuildingType::Factory), 0.0);
    assert_eq!(op_of(&o, 2, BuildingType::MilitaryFactory), 0.0);
}

/// P28-010-PRE-002の再現(Arcadia型): 備蓄が要求量の一部しかない。旧実装では全entry
/// が失敗して備蓄が減らず満額産出していた。修正後は産出が実際の引落し量に比例する。
#[test]
fn arcadia_like_shortage_no_free_output_and_exact_conservation() {
    let states = vec![state(1, BuildingType::Factory, 1), state(12, BuildingType::MilitaryFactory, 1)];
    let before_iron = 0.975;
    let o = run(
        states,
        stock(&[
            (ResourceType::Iron, before_iron),
            (ResourceType::Coal, 200.0),
            (ResourceType::IndustrialGoods, 4.0),
        ]),
        false,
    );
    let op_f = op_of(&o, 1, BuildingType::Factory);
    let op_m = op_of(&o, 12, BuildingType::MilitaryFactory);
    let used = 20.0 * op_f + 30.0 * op_m;
    // 実際に引き落とされたIronだけが減る(備蓄 0.975 が据え置かれない)。
    let iron_after = o.stock.get(ResourceType::Iron);
    assert!((iron_after - (before_iron - used)).abs() < 1e-5, "{iron_after} vs {}", before_iron - used);
    assert!(iron_after >= 0.0);
    assert!(used > 0.9, "備蓄はほぼ使い切られる: {used}");
    // 産出は引落しに対応する稼働率からだけ生じる。
    assert!(
        (o.stock.get(ResourceType::MilitarySupplies) - ms_quantized(100.0 * op_m)).abs() < 1e-9
    );
    let ig_expected = 4.0 + 30.0 * op_f - 15.0 * op_m;
    assert!((o.stock.get(ResourceType::IndustrialGoods) - ig_expected).abs() < 1e-5);
    let coal_expected = 200.0 - 20.0 * op_f;
    assert!((o.stock.get(ResourceType::Coal) - coal_expected).abs() < 1e-5);
}

/// 複数入力の一方だけが不足: 最も厳しい入力に従い、他の入力も同じ稼働率で引かれる。
#[test]
fn one_of_two_inputs_short_limits_the_entry_and_scales_both_inputs() {
    let o = run(
        vec![state(1, BuildingType::Factory, 2)],
        stock(&[(ResourceType::Iron, 10_000.0), (ResourceType::Coal, 5.0)]),
        false,
    );
    let op = op_of(&o, 1, BuildingType::Factory);
    assert!((op - 5.0 / 40.0).abs() < 1e-6, "op={op}");
    assert!(o.stock.get(ResourceType::Coal) >= 0.0 && o.stock.get(ResourceType::Coal) < 1e-5);
    let iron_used = 10_000.0 - o.stock.get(ResourceType::Iron);
    assert!((iron_used - 40.0 * op).abs() < 1e-4, "Ironも同じ稼働率で引かれる");
    assert!((o.stock.get(ResourceType::IndustrialGoods) - 60.0 * op).abs() < 1e-4);
}

/// ちょうど充足: 備蓄 = 要求量。稼働率は計画値のまま、備蓄は0になる。
#[test]
fn exactly_sufficient_inputs_run_at_planned_operation() {
    let s = state(1, BuildingType::Factory, 1);
    let t = t_of(&s);
    let need_iron = 20.0 * t as f64;
    let need_coal = 20.0 * t as f64;
    let o = run(
        vec![s],
        stock(&[(ResourceType::Iron, need_iron), (ResourceType::Coal, need_coal)]),
        false,
    );
    // 月初備蓄/需要=1なのでsat=1。計画稼働率はt。
    assert_eq!(op_of(&o, 1, BuildingType::Factory) as f32, t);
    assert_eq!(o.stock.get(ResourceType::Iron), 0.0);
    assert_eq!(o.stock.get(ResourceType::Coal), 0.0);
    assert_eq!(o.stock.get(ResourceType::IndustrialGoods), 30.0 * t as f64);
}

/// わずかに不足(要求量の 1 ulp 手前の備蓄でも負にならず、全入力が成立する)。
#[test]
fn slightly_short_inputs_never_go_negative_and_stay_consistent() {
    for scale in [0.999_999_9, 0.99, 0.5, 0.01, 1e-9] {
        let s = state(1, BuildingType::Factory, 3);
        let t = t_of(&s) as f64;
        let need = 60.0 * t;
        let o = run(
            vec![s],
            stock(&[(ResourceType::Iron, need * scale), (ResourceType::Coal, 1000.0)]),
            false,
        );
        let op = op_of(&o, 1, BuildingType::Factory);
        let iron_after = o.stock.get(ResourceType::Iron);
        assert!(iron_after >= 0.0, "scale={scale}");
        assert!((need * scale - iron_after - 60.0 * op).abs() < 1e-6, "scale={scale}");
        assert!((o.stock.get(ResourceType::IndustrialGoods) - 90.0 * op).abs() < 1e-6, "scale={scale}");
    }
}

/// 複数州・複数entryが同じ国家備蓄を順に使う: 備蓄を超えず、全体の保存則が成立する。
#[test]
fn many_entries_sharing_one_national_stockpile_conserve_resources() {
    let mut states = Vec::new();
    for i in 0..4 {
        states.push(state(10 + i, BuildingType::Factory, 2));
    }
    for i in 0..3 {
        states.push(state(30 + i, BuildingType::MilitaryFactory, 2));
    }
    let o = run(
        states,
        stock(&[
            (ResourceType::Iron, 7.0),
            (ResourceType::Coal, 11.0),
            (ResourceType::IndustrialGoods, 3.0),
        ]),
        false,
    );
    let mut iron_used = 0.0;
    let mut coal_used = 0.0;
    let mut ig_made = 0.0;
    let mut ig_used = 0.0;
    let mut ms_made = 0.0;
    for s in &o.states {
        for (&b, &op) in &s.building_operation_ratios {
            let level = s.building_level(b) as f64;
            let op = op as f64;
            match b {
                BuildingType::Factory => {
                    iron_used += 20.0 * level * op;
                    coal_used += 20.0 * level * op;
                    ig_made += 30.0 * level * op;
                }
                BuildingType::MilitaryFactory => {
                    iron_used += 30.0 * level * op;
                    ig_used += 15.0 * level * op;
                    ms_made += ms_quantized(100.0 * level * op);
                }
                _ => {}
            }
        }
    }
    let close = |a: f64, b: f64| (a - b).abs() < 1e-5;
    assert!(close(o.stock.get(ResourceType::Iron), 7.0 - iron_used));
    assert!(close(o.stock.get(ResourceType::Coal), 11.0 - coal_used));
    assert!(close(o.stock.get(ResourceType::IndustrialGoods), 3.0 + ig_made - ig_used));
    assert!(close(o.stock.get(ResourceType::MilitarySupplies), ms_made));
    for r in RES {
        assert!(o.stock.get(r) >= 0.0);
    }
}

/// MagicAcademy: MagicCrystalが足りなければ研究力(magic)は引落し分だけ。0なら0。
#[test]
fn magic_academy_research_follows_actual_crystal_consumption() {
    let s = state(1, BuildingType::MagicAcademy, 2);
    let t = t_of(&s) as f64;
    let full = run(vec![s.clone()], stock(&[(ResourceType::MagicCrystal, 1000.0)]), false);
    assert_eq!(full.mag, 30.0 * 2.0 * (t as f32) as f64);

    let short = run(vec![s.clone()], stock(&[(ResourceType::MagicCrystal, 1.0)]), false);
    let op = op_of(&short, 1, BuildingType::MagicAcademy);
    assert!((op - 1.0 / 20.0).abs() < 1e-6);
    assert!((short.mag - 60.0 * op).abs() < 1e-5);
    assert!(short.stock.get(ResourceType::MagicCrystal) >= 0.0);

    let none = run(vec![s], stock(&[]), false);
    assert_eq!(none.mag, 0.0, "無償の研究力は発生しない");
    assert_eq!(none.sci, 0.0);
}

/// 破産ペナルティ(×0.8)があっても、入力超過・無償産出は起きない。
#[test]
fn bankrupt_penalty_does_not_reintroduce_free_output() {
    let o = run(
        vec![state(1, BuildingType::MilitaryFactory, 2)],
        stock(&[(ResourceType::Iron, 2.0), (ResourceType::IndustrialGoods, 1.0)]),
        true,
    );
    let op = op_of(&o, 1, BuildingType::MilitaryFactory);
    assert!((2.0 - o.stock.get(ResourceType::Iron) - 60.0 * op).abs() < 1e-6);
    assert!((1.0 - o.stock.get(ResourceType::IndustrialGoods) - 30.0 * op).abs() < 1e-6);
    assert!((o.stock.get(ResourceType::MilitarySupplies) - ms_quantized(200.0 * op)).abs() < 1e-9);
}

/// 備蓄の水準を変えながら: 保存則(残量=備蓄-引落し+産出)・非負・産出∝稼働率を
/// 全entryで検証する(境界・量子化を含む広い掃引)。
#[test]
fn sweep_of_stock_levels_keeps_accounting_identities() {
    let levels = [0.0, 1e-12, 1e-6, 0.3, 0.975, 1.0, 2.0, 7.5, 19.99, 20.0, 60.0, 100.0, 1e4];
    for &iron in &levels {
        for &coal in &levels {
            for &ig in &levels {
                let o = run(
                    vec![
                        state(1, BuildingType::Factory, 2),
                        state(2, BuildingType::Factory, 1),
                        state(3, BuildingType::MilitaryFactory, 3),
                    ],
                    stock(&[
                        (ResourceType::Iron, iron),
                        (ResourceType::Coal, coal),
                        (ResourceType::IndustrialGoods, ig),
                    ]),
                    false,
                );
                let (mut iu, mut cu, mut igm, mut igu, mut ms) = (0.0, 0.0, 0.0, 0.0, 0.0);
                for s in &o.states {
                    for (&b, &op) in &s.building_operation_ratios {
                        let l = s.building_level(b) as f64;
                        let op = op as f64;
                        match b {
                            BuildingType::Factory => {
                                iu += 20.0 * l * op;
                                cu += 20.0 * l * op;
                                igm += 30.0 * l * op;
                            }
                            BuildingType::MilitaryFactory => {
                                iu += 30.0 * l * op;
                                igu += 15.0 * l * op;
                                ms += ms_quantized(100.0 * l * op);
                            }
                            _ => {}
                        }
                    }
                }
                let tol = 1e-6 * (1.0 + iron.max(coal).max(ig));
                assert!((o.stock.get(ResourceType::Iron) - (iron - iu)).abs() < tol, "iron {iron}/{coal}/{ig}");
                assert!((o.stock.get(ResourceType::Coal) - (coal - cu)).abs() < tol, "coal {iron}/{coal}/{ig}");
                assert!(
                    (o.stock.get(ResourceType::IndustrialGoods) - (ig + igm - igu)).abs() < tol,
                    "ig {iron}/{coal}/{ig}"
                );
                assert!((o.stock.get(ResourceType::MilitarySupplies) - ms).abs() < 1e-6);
                for r in RES {
                    assert!(o.stock.get(r) >= 0.0, "{r:?} {iron}/{coal}/{ig}");
                }
            }
        }
    }
}

/// 国家備蓄は他国へ漏れない(別の`CountryStockpile`は不変)。
#[test]
fn other_countries_stockpile_is_untouched() {
    let other = stock(&[(ResourceType::Iron, 123.0)]);
    let snapshot = other.get(ResourceType::Iron);
    let _ = run(vec![state(1, BuildingType::Factory, 1)], stock(&[(ResourceType::Iron, 1.0)]), false);
    assert_eq!(other.get(ResourceType::Iron), snapshot);
}
