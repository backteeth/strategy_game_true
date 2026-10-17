//! P30-003: 軍需工場の再建の国内供給判定が「何を保証し、何を保証しないか」の確認。
//!
//! 判定(`try_military_rebuild_construction`)は、Iron の総産出と Factory の IG 産出を**独立に**必要量
//! と比べる。Iron を Factory と MilitaryFactory が競合する場合、Factory の IG 産出も変わるため、この
//! 判定は必要条件にとどまる。ここでは、判定を通る条件で、完成後の実際の月次生産(D4 の配分・消費を
//! 含む)を追跡し、判定の見込みと実績の関係を記録する。労働力・物流の制約は t = 1(全員雇用)に
//! 固定し、入力財の競合だけを見る(状態を操作した補助実験。通常シナリオの再現ではない)。

use std::collections::HashMap;

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, DivisionId, StateId};
use strategy_game::country::country_ai::{
    CountryAiRegistry, military_rebuild_context, military_rebuild_gain_units,
    process_construction_ai_with_contexts,
};
use strategy_game::country::{CountryData, CountryRegistry};
use strategy_game::economy::production::process_country_production_with_outcome;
use strategy_game::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use strategy_game::military::data::{
    Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
    STANDARD_INFANTRY_DEFINITION_ID,
};
use strategy_game::military::supply_production_history::SupplyProductionHistory;
use strategy_game::state::data::{StateData, StateRegistry};

const ME: CountryId = CountryId(3);

fn def(
    b: BuildingType,
    cost: f64,
    max_level: u32,
    inputs: &[(ResourceType, f64)],
    outputs: &[(ResourceType, f64)],
) -> BuildingDefinition {
    BuildingDefinition {
        building_type: b,
        name: format!("{b:?}"),
        construction_cost: cost,
        required_progress: 60.0,
        required_workforce: 25_000.0,
        logistics_cost: 0.0,
        input_resources: inputs.iter().copied().collect::<HashMap<_, _>>(),
        output_resources: outputs.iter().copied().collect::<HashMap<_, _>>(),
        maintenance_cost: 10.0,
        max_level,
        science_output: 0.0,
        magic_output: 0.0,
        railway_capacity_bonus: 0.0,
    }
}

/// `assets/data/buildings.ron` と同じ入出力(必要労働力だけ小さくして、全員雇用の t = 1 を保つ)。
fn registry() -> BuildingRegistry {
    let mut r = BuildingRegistry::default();
    for d in [
        def(BuildingType::Mine, 500.0, 10, &[], &[]),
        def(
            BuildingType::Factory,
            1000.0,
            10,
            &[(ResourceType::Iron, 20.0), (ResourceType::Coal, 20.0)],
            &[(ResourceType::IndustrialGoods, 30.0)],
        ),
        def(
            BuildingType::MilitaryFactory,
            1200.0,
            10,
            &[(ResourceType::Iron, 30.0), (ResourceType::IndustrialGoods, 15.0)],
            &[(ResourceType::MilitaryEquipment, 20.0), (ResourceType::MilitarySupplies, 100.0)],
        ),
        def(BuildingType::Railway, 800.0, 5, &[], &[]),
    ] {
        r.definitions.insert(d.building_type, d);
    }
    r
}

/// 全員雇用(労働力 20,000 人 = 雇用 20,000 人、必要労働力はそれ以上)・物流 1.0 の 1 州。
fn state(mine: u32, factory: u32, mf: u32, iron_base: f64) -> StateData {
    let mut s = StateData {
        id: StateId(1),
        name: "S1".to_string(),
        owner_country_id: ME,
        population: 40_000,
        workforce_ratio: 0.5,
        employed_workforce: 20_000,
        unemployed_workforce: 0,
        logistics_ratio: 1.0,
        logistics_capacity: 1.0e6,
        world_position: [0.0, 0.0],
        size: [100.0, 100.0],
        resource_deposits: vec![
            StateResourceDeposit {
                resource_type: ResourceType::Iron,
                base_output: iron_base,
                discovered: true,
                development_level: 1,
            },
            StateResourceDeposit {
                resource_type: ResourceType::Coal,
                base_output: 100.0,
                discovered: true,
                development_level: 1,
            },
        ],
        ..Default::default()
    };
    s.buildings.insert(BuildingType::Mine, mine);
    s.buildings.insert(BuildingType::Factory, factory);
    if mf > 0 {
        s.buildings.insert(BuildingType::MilitaryFactory, mf);
    }
    s
}

struct Month {
    mf_op: f64,
    factory_op: f64,
    ms_out: f64,
    iron: f64,
    ig: f64,
}

/// 月次生産を `months` か月進める(`process_country_production_with_outcome` の実際の D4・配分・消費)。
fn run(s: &mut StateData, stock: &mut CountryStockpile, months: usize) -> Vec<Month> {
    let reg = registry();
    let mut out = Vec::new();
    for _ in 0..months {
        let o = {
            let mut refs: Vec<&mut StateData> = vec![&mut *s];
            process_country_production_with_outcome(&mut refs, stock, &reg, false)
        };
        let op = |b: BuildingType| s.building_operation_ratios.get(&b).copied().unwrap_or(0.0) as f64;
        out.push(Month {
            mf_op: op(BuildingType::MilitaryFactory),
            factory_op: op(BuildingType::Factory),
            ms_out: o.military_supplies_produced.unwrap_or(0.0),
            iron: stock.get(ResourceType::Iron),
            ig: stock.get(ResourceType::IndustrialGoods),
        });
    }
    out
}

fn stock() -> CountryStockpile {
    let mut st = CountryStockpile::new();
    st.set(ResourceType::Coal, 1.0e9);
    st
}

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

#[allow(dead_code)]
fn _division() -> Division {
    Division {
        id: DivisionId(0),
        owner: ME,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(1),
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 10_000,
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

/// 再建前(MilitaryFactory なし)に 12 か月生産して稼働率を記録させ、再建の判断を通るか(= 国内供給の
/// 見込みを満たすか)を、実際の判断経路で確認する。通れば、MilitaryFactory lv1 を足して 36 か月進める。
fn trace(mine: u32, factory: u32, iron_base: f64) -> (bool, f64, Vec<Month>) {
    let mut st = stock();
    let mut s = state(mine, factory, 0, iron_base);
    let pre = run(&mut s, &mut st, 12);
    let _ = pre;
    // 判断: 4 か月の履歴(生産 0)・師団 0・目標未達。
    let mut hist = SupplyProductionHistory::default();
    for m in 8..=11u8 {
        hist.record(1801, m, Some(0));
    }
    let mut mil = MilitaryRegistry::default();
    mil.definitions.insert(STANDARD_INFANTRY_DEFINITION_ID, infantry_def());
    // 目標師団数を確保するための、人口の大きい州(建物なし。増産見込みは州 1 より小さい)。
    let mut big = state(0, 0, 0, 0.0);
    big.id = StateId(2);
    big.name = "S2".to_string();
    big.population = 3_000_000;
    big.employed_workforce = 0;
    big.unemployed_workforce = 1_500_000;
    big.resource_deposits.clear();
    big.buildings.clear();
    big.world_position = [100.0, 0.0];
    let sr = StateRegistry::build(vec![s.clone(), big]);
    let mut c = CountryData {
        id: ME,
        name: "Fixture".to_string(),
        capital_state_id: StateId(1),
        treasury: 50_000.0,
        monthly_expenses: 100.0,
        ..Default::default()
    };
    c.stockpile.set(ResourceType::Food, 5000.0);
    c.stockpile.set(ResourceType::Wood, 5000.0);
    c.stockpile.set(ResourceType::Coal, 1.0e9);
    c.stockpile.set(ResourceType::Iron, 1000.0);
    let mut cr = CountryRegistry { countries: vec![c] };
    let ctx = military_rebuild_context(cr.get(ME).unwrap(), &sr, &mil, &hist, "1801/11/15");
    let mut ai = CountryAiRegistry::default();
    process_construction_ai_with_contexts(
        ME,
        &mut cr,
        &sr,
        &registry(),
        ai.get_or_create_mut(ME),
        None,
        None,
        false,
        ctx,
    );
    let queued_mf = cr
        .get(ME)
        .unwrap()
        .construction_queue
        .iter()
        .any(|q| q.building_type == BuildingType::MilitaryFactory);
    let predicted_units =
        military_rebuild_gain_units(&s, &registry()).map(|u| u as f64 / 1.0e6).unwrap_or(0.0);
    // 完成後(MilitaryFactory lv1 を追加)の実際の月次生産。判断を通らなくても、競合の程度を見るために進める。
    s.buildings.insert(BuildingType::MilitaryFactory, 1);
    let post = run(&mut s, &mut st, 120);
    (queued_mf, predicted_units, post)
}

fn tail_mean(post: &[Month], f: impl Fn(&Month) -> f64) -> f64 {
    let t = &post[post.len() - 12..];
    t.iter().map(&f).sum::<f64>() / t.len() as f64
}

/// 判定を通る 3 つの条件で、完成後の実績(定常)と見込みを記録する。
/// A: Iron が十分。B: Iron の総産出(50)は MilitaryFactory の必要量(30)以上だが、Factory(Iron 需要 60)と
/// 競合。C: Factory の Iron 需要が大きく(200)、総産出(50)の大半を Factory が使っている。
#[test]
fn rebuild_check_passes_under_competition_and_the_realised_output_is_recorded() {
    for (name, mine, factory, base) in [
        ("A rich", 4u32, 2u32, 50.0),
        ("B tight (Factory lv3)", 1, 3, 50.0),
        ("C heavy (Factory lv10)", 1, 10, 50.0),
        ("D marginal (S just above need)", 1, 10, 32.0),
    ] {
        let (fires, predicted, post) = trace(mine, factory, base);
        let steady_ms = tail_mean(&post, |m| m.ms_out);
        let steady_op = tail_mean(&post, |m| m.mf_op);
        let steady_fop = tail_mean(&post, |m| m.factory_op);
        let first_ms = post[0].ms_out;
        let min_iron = post.iter().map(|m| m.iron).fold(f64::MAX, f64::min);
        let yearly: Vec<String> = [12usize, 24, 36, 48, 72, 96, 120]
            .iter()
            .map(|&i| format!("m{i}:op={:.2},ig={:.0}", post[i - 1].mf_op, post[i - 1].ig))
            .collect();
        println!("COMP_TRACK {name}: {}", yearly.join(" "));
        println!(
            "COMP {name}: check_passes={fires} predicted_ms={predicted:.1} first_month_ms={first_ms:.1} steady_ms(last12 mean)={steady_ms:.1} steady_mf_op={steady_op:.2} steady_factory_op={steady_fop:.2} min_iron_stock={min_iron:.0} ig_end={:.0}",
            post.last().unwrap().ig
        );
        // 保存則・非負(配分の二重消費・負の備蓄がない)。
        assert!(post.iter().all(|m| m.iron >= 0.0 && m.ig >= 0.0));
        assert!(fires, "{name}: 供給の判定を通る(総産出が必要量以上)");
        match name {
            // A〜C: D4 の優先配分(MilitaryFactory が Iron を先に確保)で競合が解消し、見込みどおり持続する。
            // Factory の稼働率が下がっても、IG 産出(月 30 以上)が MilitaryFactory の月 15 を上回る。
            n if n.starts_with('A') || n.starts_with('B') || n.starts_with('C') => {
                assert!((steady_ms - predicted).abs() < 1.0, "{name}: {steady_ms} vs {predicted}");
            }
            // D: 総産出(32)が必要量(30)をわずかに上回るだけ。IG を Factory が Iron から作る必要がある
            // ため、実際に持続できる MilitaryFactory の稼働率は Iron 総産出 ÷ 40(MilitaryFactory の Iron 30 +
            // IG 15 を作る Iron 10)= 0.8 で、見込み(1.0)より低い。完成後の約 3 年は、再建前の Factory が
            // 蓄えた有限の IG 備蓄を取り崩して稼働率 1.0 を保つ(一時的稼働)。判定は Iron と IG を独立に
            // 比べるため必要条件にとどまる(設計上の限界の記録。実装の不具合ではない)。
            _ => {
                assert_eq!(post[23].mf_op, 1.0, "取り崩しの間は見込みどおり");
                assert!(steady_op < 1.0 && (steady_op - base / 40.0).abs() < 0.02, "steady {steady_op}");
                assert!(steady_ms < predicted * 0.9);
            }
        }
    }
}
