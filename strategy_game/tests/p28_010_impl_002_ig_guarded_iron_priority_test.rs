//! P28-010-IMPL-002: IG備蓄で保護したIron軍需優先配分(D4)の境界・回帰テスト。
//!
//! 契約: Ironが不足し、かつIndustrialGoods備蓄がMilitaryFactoryの月間IG需要
//! (`base × level × t`の州合計)の3か月分以上あるときだけ、MilitaryFactoryへIronを優先配分する。
//! それ以外は現行の需要比例配分(D0)。FIX1(原子的な入力会計)は常に維持される。
//! 以下のfixtureは労働力比・物流比を1.0にして`t = 1`とし、需要を整数で扱う。

use std::collections::HashMap;

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::economy::production::{
    MILITARY_IRON_PRIORITY_IG_RESERVE_MONTHS, process_country_production,
};
use strategy_game::economy::resources::{
    CountryStockpile, ResourceType, military_supplies_units_to_value,
    military_supplies_value_to_units,
};
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

fn registry() -> BuildingRegistry {
    let mut r = BuildingRegistry::default();
    for d in [factory_def(), military_factory_def()] {
        r.definitions.insert(d.building_type, d);
    }
    r
}

/// `employed == total_workforce` で労働力比 1.0、物流比 1.0 → `t = 1`。
fn state(id: usize, b: BuildingType, level: u32) -> StateData {
    state_with_employment(id, b, level, 500_000)
}

fn state_with_employment(id: usize, b: BuildingType, level: u32, employed: u64) -> StateData {
    StateData {
        id: StateId(id),
        name: format!("State {id}"),
        owner_country_id: CountryId(0),
        population: 1_000_000,
        workforce_ratio: 0.5,
        employed_workforce: employed,
        logistics_ratio: 1.0,
        buildings: HashMap::from([(b, level)]),
        ..Default::default()
    }
}

fn stock(items: &[(ResourceType, f64)]) -> CountryStockpile {
    let mut s = CountryStockpile::new();
    for &(r, a) in items {
        s.set(r, a);
    }
    s
}

struct Out {
    states: Vec<StateData>,
    stock: CountryStockpile,
}

fn run(mut states: Vec<StateData>, mut stockpile: CountryStockpile) -> Out {
    let reg = registry();
    {
        let mut refs: Vec<&mut StateData> = states.iter_mut().collect();
        process_country_production(&mut refs, &mut stockpile, &reg, false);
    }
    Out { states, stock: stockpile }
}

fn op(o: &Out, b: BuildingType) -> f64 {
    // 同じ建物種別の州の稼働率(全て同じ値になるfixtureのみで使う)。
    o.states
        .iter()
        .find_map(|s| s.building_operation_ratios.get(&b).copied())
        .unwrap() as f64
}

fn iron_used(o: &Out, b: BuildingType) -> f64 {
    let base = if b == BuildingType::Factory { 20.0 } else { 30.0 };
    o.states
        .iter()
        .map(|s| {
            let l = s.building_level(b) as f64;
            let p = s.building_operation_ratios.get(&b).copied().unwrap_or(0.0) as f64;
            base * l * p
        })
        .sum()
}

fn next_down(x: f64) -> f64 {
    f64::from_bits(x.to_bits() - 1)
}

fn next_up(x: f64) -> f64 {
    f64::from_bits(x.to_bits() + 1)
}

const IRON_SCARCE: f64 = 50.0; // 需要: Factory(L2)=40 + MilitaryFactory(L1)=30 = 70 > 50
const IG_GATE: f64 = 45.0; // MilitaryFactory(L1)の月間IG需要 15 × 3か月

fn scarce_case(ig: f64) -> Out {
    run(
        vec![state(1, BuildingType::Factory, 2), state(2, BuildingType::MilitaryFactory, 1)],
        stock(&[
            (ResourceType::Iron, IRON_SCARCE),
            (ResourceType::Coal, 1000.0),
            (ResourceType::IndustrialGoods, ig),
        ]),
    )
}

#[test]
fn contract_constant_is_three_months() {
    assert_eq!(MILITARY_IRON_PRIORITY_IG_RESERVE_MONTHS, 3.0);
}

/// 境界の直上・ちょうど: MilitaryFactoryがIronを優先確保し、Factoryは残りの按分だけを受け取る。
#[test]
fn ig_at_or_above_three_months_prioritizes_the_military_factory() {
    for ig in [IG_GATE, next_up(IG_GATE), 500.0] {
        let o = scarce_case(ig);
        assert_eq!(op(&o, BuildingType::MilitaryFactory), 1.0, "ig={ig}");
        assert!((iron_used(&o, BuildingType::MilitaryFactory) - 30.0).abs() < 1e-9, "ig={ig}");
        // 残り 50-30=20 を Factory の需要 40 に按分 → 稼働率 0.5、Iron 20。
        assert!((op(&o, BuildingType::Factory) - 0.5).abs() < 1e-6, "ig={ig}");
        assert!((iron_used(&o, BuildingType::Factory) - 20.0).abs() < 1e-5, "ig={ig}");
        assert!(o.stock.get(ResourceType::Iron) < 1e-5, "備蓄は使い切られる ig={ig}");
    }
}

/// 境界の直下: 現行の需要比例配分(D0)。両者とも 50/70 の稼働率。
#[test]
fn ig_just_below_three_months_uses_the_proportional_rule() {
    let o = scarce_case(next_down(IG_GATE));
    let sat = 50.0 / 70.0;
    assert!((op(&o, BuildingType::Factory) - sat).abs() < 1e-5);
    assert!((iron_used(&o, BuildingType::Factory) - 40.0 * sat).abs() < 1e-4);
    assert!((iron_used(&o, BuildingType::MilitaryFactory) - 30.0 * sat).abs() < 1e-4);
}

/// Ironが足りている場合は、IG備蓄に関わらず従来と同じ(全員が計画稼働率 t = 1)。
#[test]
fn iron_sufficient_is_identical_for_every_ig_level() {
    for ig in [0.0, 10.0, IG_GATE, 1.0e6] {
        let o = run(
            vec![state(1, BuildingType::Factory, 2), state(2, BuildingType::MilitaryFactory, 1)],
            stock(&[
                (ResourceType::Iron, 1000.0),
                (ResourceType::Coal, 1000.0),
                (ResourceType::IndustrialGoods, ig),
            ]),
        );
        assert_eq!(op(&o, BuildingType::Factory), 1.0, "ig={ig}");
        // MilitaryFactoryはIG備蓄で律速される(15 に対し ig)が、Ironの配分規則は変わらない。
        let want_mf = (ig / 15.0).min(1.0);
        assert!((op(&o, BuildingType::MilitaryFactory) - want_mf).abs() < 1e-6, "ig={ig}");
        assert!((o.stock.get(ResourceType::Iron) - (1000.0 - 40.0 - 30.0 * want_mf)).abs() < 1e-4);
    }
}

/// MilitaryFactoryが無い国は、Iron不足でも常に従来の比例配分(D0)。
#[test]
fn no_military_factory_keeps_the_legacy_result() {
    let o = run(
        vec![state(1, BuildingType::Factory, 2)],
        stock(&[(ResourceType::Iron, 20.0), (ResourceType::Coal, 1000.0), (ResourceType::IndustrialGoods, 1e6)]),
    );
    assert!((op(&o, BuildingType::Factory) - 0.5).abs() < 1e-6);
    assert!((iron_used(&o, BuildingType::Factory) - 20.0).abs() < 1e-5);
}

/// Iron備蓄 0・需要 0(労働力 0)・両方 0 でもpanicせず、備蓄は負にならない。
#[test]
fn zero_stock_and_zero_demand_are_well_defined() {
    let o = run(
        vec![state(1, BuildingType::Factory, 2), state(2, BuildingType::MilitaryFactory, 1)],
        stock(&[(ResourceType::IndustrialGoods, 1000.0), (ResourceType::Coal, 100.0)]),
    );
    assert_eq!(o.stock.get(ResourceType::Iron), 0.0);
    assert_eq!(o.stock.get(ResourceType::MilitarySupplies), 0.0);
    assert_eq!(o.stock.get(ResourceType::Coal), 100.0);

    // 労働力比 0 → 需要 0。
    let o = run(
        vec![
            state_with_employment(1, BuildingType::Factory, 2, 0),
            state_with_employment(2, BuildingType::MilitaryFactory, 1, 0),
        ],
        stock(&[(ResourceType::Iron, 10.0), (ResourceType::IndustrialGoods, 1000.0), (ResourceType::Coal, 100.0)]),
    );
    assert_eq!(o.stock.get(ResourceType::Iron), 10.0);
    assert_eq!(o.stock.get(ResourceType::MilitarySupplies), 0.0);
}

/// 複数州・複数levelでも同じ定義(州合計の需要)で判定し、合計がIron備蓄を超えない。
#[test]
fn multiple_states_use_the_national_totals() {
    let states = || {
        vec![
            state(1, BuildingType::Factory, 1),
            state(5, BuildingType::Factory, 2),
            state(9, BuildingType::Factory, 3),
            state(2, BuildingType::MilitaryFactory, 1),
            state(7, BuildingType::MilitaryFactory, 2),
        ]
    };
    // MilitaryFactory 合計 L3: Iron 90、IG 45。ゲート = 135。Factory 合計 L6: Iron 120。
    let gate = 3.0 * 45.0;
    let on = run(
        states(),
        stock(&[(ResourceType::Iron, 100.0), (ResourceType::Coal, 1000.0), (ResourceType::IndustrialGoods, gate)]),
    );
    // 稼働率はf32のため、Factoryが予算を超えない範囲で丸め誤差(相対 1e-7)が出る。
    assert!((iron_used(&on, BuildingType::MilitaryFactory) - 90.0).abs() < 1e-5);
    assert!((iron_used(&on, BuildingType::Factory) - 10.0).abs() < 1e-5);
    let off = run(
        states(),
        stock(&[(ResourceType::Iron, 100.0), (ResourceType::Coal, 1000.0), (ResourceType::IndustrialGoods, next_down(gate))]),
    );
    let sat = 100.0 / 210.0;
    assert!((iron_used(&off, BuildingType::MilitaryFactory) - 90.0 * sat).abs() < 1e-3);
    assert!((iron_used(&off, BuildingType::Factory) - 120.0 * sat).abs() < 1e-3);
}

/// 浮動小数点: `t`が2進で割り切れない場合でも、ゲートは「ちょうど3か月分」で包含的に切り替わる。
#[test]
fn gate_is_exact_for_a_non_dyadic_operation_ratio() {
    let employed = 333_333u64; // t = 333333 / 500000(f32)
    let mk = || {
        vec![
            state_with_employment(1, BuildingType::Factory, 2, employed),
            state_with_employment(2, BuildingType::MilitaryFactory, 1, employed),
        ]
    };
    let t = (employed as f32 / 500_000f32) as f64;
    let want_ig = 15.0 * 1.0 * t; // Step 1と同じ式(単一entry、0.0 + x = x)
    let gate = MILITARY_IRON_PRIORITY_IG_RESERVE_MONTHS * want_ig;
    let iron = 0.5 * (30.0 + 40.0) * t; // 需要合計の半分 = 不足
    let run_with = |ig: f64| {
        run(
            mk(),
            stock(&[(ResourceType::Iron, iron), (ResourceType::Coal, 1000.0), (ResourceType::IndustrialGoods, ig)]),
        )
    };
    let at = run_with(gate);
    let below = run_with(next_down(gate));
    // D4: MilitaryFactoryが需要 30t を全額確保(優先)。D0: 50% の按分。
    let mf_at = iron_used(&at, BuildingType::MilitaryFactory);
    let mf_below = iron_used(&below, BuildingType::MilitaryFactory);
    assert!((mf_at - 30.0 * t).abs() < 1e-5, "at gate: {mf_at}");
    // D0: 稼働率 min(t, 0.5) = 0.5(`t`なしの消費)。Factoryが先に 20 を消費し、MilitaryFactoryは
    // 残り(備蓄 − 20)をFIX1のクランプで受け取る。at-gate(30t)とは異なる。
    assert!((iron_used(&below, BuildingType::Factory) - 20.0).abs() < 1e-4, "below gate factory");
    assert!((mf_below - (iron - 20.0)).abs() < 1e-4, "below gate: {mf_below}");
    assert!(mf_at > mf_below);
}

/// 州の挿入順・生産entryの登録順を変えても結果が同じ(全720順列、ゲートの両側)。
#[test]
fn insertion_order_does_not_change_the_result() {
    fn perms(n: usize) -> Vec<Vec<usize>> {
        fn rec(cur: &mut Vec<usize>, used: &mut Vec<bool>, n: usize, out: &mut Vec<Vec<usize>>) {
            if cur.len() == n {
                out.push(cur.clone());
                return;
            }
            for i in 0..n {
                if !used[i] {
                    used[i] = true;
                    cur.push(i);
                    rec(cur, used, n, out);
                    cur.pop();
                    used[i] = false;
                }
            }
        }
        let mut out = Vec::new();
        rec(&mut Vec::new(), &mut vec![false; n], n, &mut out);
        out
    }
    let all = [
        state(1, BuildingType::Factory, 2),
        state(5, BuildingType::Factory, 1),
        state(9, BuildingType::Factory, 3),
        state(2, BuildingType::MilitaryFactory, 1),
        state(7, BuildingType::MilitaryFactory, 2),
        state(11, BuildingType::MilitaryFactory, 3),
    ];
    let snapshot = |perm: &[usize], iron: f64, ig: f64| {
        let o = run(
            perm.iter().map(|&i| all[i].clone()).collect(),
            stock(&[(ResourceType::Iron, iron), (ResourceType::Coal, 100.0), (ResourceType::IndustrialGoods, ig)]),
        );
        let mut v: Vec<(String, f64)> = [
            ResourceType::Iron,
            ResourceType::Coal,
            ResourceType::IndustrialGoods,
            ResourceType::MilitarySupplies,
            ResourceType::MilitaryEquipment,
        ]
        .iter()
        .map(|r| (format!("{r:?}"), o.stock.get(*r)))
        .collect();
        let mut ops: Vec<(usize, String, f64)> = o
            .states
            .iter()
            .flat_map(|s| s.building_operation_ratios.iter().map(move |(b, p)| (s.id.0, format!("{b:?}"), *p as f64)))
            .collect();
        ops.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v.extend(ops.into_iter().map(|(i, b, p)| (format!("s{i}{b}"), p)));
        v
    };
    let all_perms = perms(6);
    assert_eq!(all_perms.len(), 720);
    for &(iron, ig) in &[(60.0, 1000.0), (60.0, 3.0 * 90.0), (60.0, 3.0 * 90.0 - 1e-9), (400.0, 1000.0), (5.0, 1000.0)] {
        let base = snapshot(&all_perms[0], iron, ig);
        for p in &all_perms[1..] {
            assert_eq!(snapshot(p, iron, ig), base, "iron={iron} ig={ig} perm={p:?}");
        }
    }
}

/// IG律速で優先配分が成立しない(IG < ゲート)月は、MilitaryFactoryがIronを過剰に確保しない:
/// Factoryが通常どおりIronを受け取り、IG備蓄が回復する。IG回復後、ゲートが再び成立してD4に戻る。
#[test]
fn guard_releases_iron_to_the_factory_and_priority_returns_after_ig_recovers() {
    let mut states = vec![state(1, BuildingType::Factory, 2), state(2, BuildingType::MilitaryFactory, 1)];
    let mut ig = 10.0;
    let mut first_priority_month = None;
    let mut factory_iron_before_priority = Vec::new();
    for month in 0..40 {
        let o = run(
            states.clone(),
            stock(&[
                (ResourceType::Iron, IRON_SCARCE),
                (ResourceType::Coal, 1000.0),
                (ResourceType::IndustrialGoods, ig),
            ]),
        );
        let gate_open = ig >= IG_GATE;
        let f = iron_used(&o, BuildingType::Factory);
        if !gate_open {
            // D0: Factoryは必ずIronを受け取る(Factoryが飢えない)。
            assert!(f > 1.0, "month {month}: ig={ig} factory iron={f}");
            factory_iron_before_priority.push(f);
        } else {
            assert!((iron_used(&o, BuildingType::MilitaryFactory) - 30.0).abs() < 1e-6, "month {month}");
            if first_priority_month.is_none() {
                first_priority_month = Some(month);
            }
        }
        ig = o.stock.get(ResourceType::IndustrialGoods);
        states = o.states;
    }
    assert!(first_priority_month.is_some(), "IGが回復して優先配分が成立するはず");
    assert!(!factory_iron_before_priority.is_empty());
}

/// 入力会計の恒等式(FIX1)をIG・Iron・Coalの掃引で確認する(ゲートの両側を含む)。
#[test]
fn fix1_identities_hold_across_the_gate() {
    let levels = [0.0, 1e-9, 1.0, 14.999, 15.0, 44.999, 45.0, 45.001, 90.0, 134.999, 135.0, 1e4];
    for &iron in &levels {
        for &coal in &[0.0, 5.0, 1e4] {
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
                );
                let (mut iu, mut cu, mut igm, mut igu, mut ms) = (0.0, 0.0, 0.0, 0.0, 0.0);
                for s in &o.states {
                    for (&b, &p) in &s.building_operation_ratios {
                        let l = s.building_level(b) as f64;
                        let p = p as f64;
                        match b {
                            BuildingType::Factory => {
                                iu += 20.0 * l * p;
                                cu += 20.0 * l * p;
                                igm += 30.0 * l * p;
                            }
                            BuildingType::MilitaryFactory => {
                                iu += 30.0 * l * p;
                                igu += 15.0 * l * p;
                                ms += military_supplies_units_to_value(
                                    military_supplies_value_to_units(100.0 * l * p).unwrap(),
                                );
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
                for r in [ResourceType::Iron, ResourceType::Coal, ResourceType::IndustrialGoods] {
                    assert!(o.stock.get(r) >= 0.0);
                }
            }
        }
    }
}

/// 優先配分でも、Factoryは受け取った予算(残り)を超えて消費しない(`t < 1`でも)。
/// 労働力比 0.5 の州で、Iron 備蓄 S の月に Factory の消費が `S − MilitaryFactoryの予約` を超えないこと。
#[test]
fn others_never_spend_more_than_their_budget_when_t_is_below_one() {
    // t = 0.5。需要: Factory(L4) = 20×4×0.5 = 40、MilitaryFactory(L2) = 30×2×0.5 = 30。IG需要 = 15。
    let mk = || {
        vec![
            state_with_employment(1, BuildingType::Factory, 4, 250_000),
            state_with_employment(2, BuildingType::MilitaryFactory, 2, 250_000),
        ]
    };
    for iron in [35.0, 50.0, 60.0, 69.0] {
        let o = run(
            mk(),
            stock(&[(ResourceType::Iron, iron), (ResourceType::Coal, 1000.0), (ResourceType::IndustrialGoods, 45.0)]),
        );
        let budget = (iron - 30.0).max(0.0);
        let f = iron_used(&o, BuildingType::Factory);
        assert!(f <= budget + 1e-5, "iron={iron}: factory spent {f} > budget {budget}");
        assert!((iron_used(&o, BuildingType::MilitaryFactory) - 30.0f64.min(iron)).abs() < 1e-5, "iron={iron}");
    }
}
