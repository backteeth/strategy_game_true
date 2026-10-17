//! P30-002: 国内供給が可能な AI 国家の軍需工場(MilitaryFactory)再建の、実際の判断経路での検証。
//!
//! 仕様書(`verification_logs/phase-30/p30-001-diag-001/…spec.md` §5-3)の受け入れ条件を基礎に、
//! 再建需要の認識(`military_rebuild_context`)と、建設判断(`process_construction_ai_with_contexts`
//! の優先度 2.9)を、同じ入力条件で確認する。実際の建設・完成・生産・募兵まで進める実 App の
//! fixture は `p30_002_military_rebuild_e2e_test.rs`。

use std::collections::HashMap;

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, DivisionId, StateId};
use strategy_game::country::country_ai::{
    CountryAiRegistry, IronSupplyContext, MilitaryRebuildContext, military_rebuild_context,
    military_rebuild_gain_units, process_construction_ai_with_contexts,
};
use strategy_game::country::{CountryData, CountryRegistry};
use strategy_game::economy::resources::{ResourceType, StateResourceDeposit};
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
    required_workforce: f32,
    max_level: u32,
    inputs: &[(ResourceType, f64)],
    outputs: &[(ResourceType, f64)],
) -> BuildingDefinition {
    BuildingDefinition {
        building_type: b,
        name: format!("{b:?}"),
        construction_cost: cost,
        required_progress: 60.0,
        required_workforce,
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

fn registry() -> BuildingRegistry {
    let mut r = BuildingRegistry::default();
    for d in [
        def(BuildingType::Mine, 500.0, 10_000.0, 10, &[], &[]),
        def(
            BuildingType::Factory,
            1000.0,
            20_000.0,
            10,
            &[(ResourceType::Iron, 20.0), (ResourceType::Coal, 20.0)],
            &[(ResourceType::IndustrialGoods, 30.0)],
        ),
        def(
            BuildingType::MilitaryFactory,
            1200.0,
            25_000.0,
            10,
            &[(ResourceType::Iron, 30.0), (ResourceType::IndustrialGoods, 15.0)],
            &[(ResourceType::MilitaryEquipment, 20.0), (ResourceType::MilitarySupplies, 100.0)],
        ),
        def(BuildingType::Railway, 800.0, 0.0, 5, &[], &[]),
    ] {
        r.definitions.insert(d.building_type, d);
    }
    r
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

fn division(manpower: u64) -> Division {
    Division {
        id: DivisionId(0),
        owner: ME,
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

fn military(divisions: usize) -> MilitaryRegistry {
    let mut m = MilitaryRegistry::default();
    m.definitions.insert(STANDARD_INFANTRY_DEFINITION_ID, infantry_def());
    for _ in 0..divisions {
        m.add_division(division(10_000));
    }
    m
}

/// 鉱床(Iron 50・Coal 40)・稼働中の Mine lv2 と Factory lv2 がある州 1(供給源)と、高失業率の州 2
/// (増産見込みが正)。MilitaryFactory はない(喪失後・仮想の初期なしを区別しない)。
fn states(mine_op: Option<f32>, factory_op: Option<f32>, mine_lv: u32) -> StateRegistry {
    let base = |id: usize, pop: u64| StateData {
        id: StateId(id),
        name: format!("S{id}"),
        owner_country_id: ME,
        population: pop,
        workforce_ratio: 0.5,
        employed_workforce: 50_000,
        unemployed_workforce: 450_000,
        logistics_ratio: 1.0,
        logistics_capacity: 1000.0,
        world_position: [id as f32 * 100.0, 0.0],
        size: [100.0, 100.0],
        ..Default::default()
    };
    let mut s1 = base(1, 1_000_000);
    s1.buildings.insert(BuildingType::Mine, mine_lv);
    s1.buildings.insert(BuildingType::Factory, 2);
    if let Some(op) = mine_op {
        s1.building_operation_ratios.insert(BuildingType::Mine, op);
    }
    if let Some(op) = factory_op {
        s1.building_operation_ratios.insert(BuildingType::Factory, op);
    }
    s1.resource_deposits = vec![
        StateResourceDeposit {
            resource_type: ResourceType::Iron,
            base_output: 50.0,
            discovered: true,
            development_level: 1,
        },
        StateResourceDeposit {
            resource_type: ResourceType::Coal,
            base_output: 40.0,
            discovered: true,
            development_level: 1,
        },
    ];
    let mut s2 = base(2, 2_000_000);
    s2.unemployed_workforce = 950_000;
    StateRegistry::build(vec![s1, s2])
}

fn history(units: [Option<u64>; 4]) -> SupplyProductionHistory {
    let mut h = SupplyProductionHistory::default();
    for (i, u) in units.into_iter().enumerate() {
        h.record(1801, 6 + i as u8, u);
    }
    h
}

const NOW: &str = "1801/09/15";

fn full_history() -> SupplyProductionHistory {
    history([Some(0); 4])
}

fn country(treasury: f64) -> CountryRegistry {
    let mut c = CountryData {
        id: ME,
        name: "Oceanic-like".to_string(),
        capital_state_id: StateId(2),
        treasury,
        monthly_expenses: 100.0,
        ..Default::default()
    };
    for (r, v) in [
        (ResourceType::Food, 5000.0),
        (ResourceType::Wood, 5000.0),
        (ResourceType::Coal, 2000.0),
        (ResourceType::Iron, 1000.0),
        (ResourceType::IndustrialGoods, 500.0),
        (ResourceType::MilitarySupplies, 0.0),
    ] {
        c.stockpile.set(r, v);
    }
    CountryRegistry { countries: vec![c] }
}

fn built(cr: &CountryRegistry) -> Vec<(BuildingType, StateId)> {
    cr.get(ME)
        .unwrap()
        .construction_queue
        .iter()
        .map(|q| (q.building_type, q.state_id))
        .collect()
}

/// 判断を `calls` 回行う(軍備目標・履歴・MilitaryFactory の有無から再建の文脈を毎回導出する)。
fn decide(
    sr: &StateRegistry,
    mil: &MilitaryRegistry,
    hist: &SupplyProductionHistory,
    treasury: f64,
    iron_ctx: Option<IronSupplyContext>,
    calls: usize,
) -> (Vec<(BuildingType, StateId)>, f64) {
    let mut cr = country(treasury);
    let mut ai = CountryAiRegistry::default();
    for _ in 0..calls {
        let ctx = military_rebuild_context(cr.get(ME).unwrap(), sr, mil, hist, NOW);
        process_construction_ai_with_contexts(
            ME,
            &mut cr,
            sr,
            &registry(),
            ai.get_or_create_mut(ME),
            None,
            iron_ctx,
            false,
            ctx,
        );
    }
    (built(&cr), cr.get(ME).unwrap().treasury)
}

/// 国内供給が可能で目標未達なら、平時・師団 0・補給備蓄 0 でも、MilitaryFactory lv1 を、増産見込みが
/// 最大の州へ着工する。建設費だけが国庫から引かれる。
#[test]
fn a_domestically_supplied_country_below_its_target_rebuilds_even_with_zero_divisions_and_stock() {
    let sr = states(Some(1.0), Some(1.0), 2);
    let mil = military(0);
    let h = full_history();
    // 補給備蓄が募兵条件未満(0)・師団 0・補給消費 0 でも、再建需要を目標から認識する。
    let ctx = military_rebuild_context(country(7000.0).get(ME).unwrap(), &sr, &mil, &h, NOW)
        .expect("再建需要を認識する");
    assert!(ctx.target_divisions > ctx.current_force);
    assert_eq!(ctx.current_force, 0);
    let (b, treasury) = decide(&sr, &mil, &h, 7000.0, None, 1);
    // 州 1 と州 2 のうち、増産見込みが大きい方(人口・失業が大きい州 2)に着工する。
    let reg = registry();
    let gain = |id: usize| military_rebuild_gain_units(sr.get(StateId(id)).unwrap(), &reg).unwrap();
    let expected = if gain(2) > gain(1) { StateId(2) } else { StateId(1) };
    assert!(gain(1) > 0 && gain(2) > 0);
    assert_eq!(b, vec![(BuildingType::MilitaryFactory, expected)]);
    assert_eq!(treasury, 7000.0 - 1200.0, "建設費だけが引かれる");
}

/// 条件を満たさなければ不発(国庫・建設キューを変更しない)。個別の条件ごとに確認する。
#[test]
fn the_rebuild_does_not_fire_unless_every_condition_holds() {
    let good = states(Some(1.0), Some(1.0), 2);
    let none: Vec<(BuildingType, StateId)> = vec![];
    // 従来の判断(優先度 3)は Factory 等を建てうるため、「MilitaryFactory を建てない」ことで確認する。
    let no_mf = |b: &Vec<(BuildingType, StateId)>| {
        b.iter().all(|(t, _)| *t != BuildingType::MilitaryFactory)
    };
    let _ = none;

    // 目標達成(師団が目標以上): 人口 3,000,000 → 目標 (cap/2)/10,000。十分な師団がある。
    let target = military_rebuild_context(country(7000.0).get(ME).unwrap(), &good, &military(0), &full_history(), NOW)
        .unwrap()
        .target_divisions as usize;
    let (b, _) = decide(&good, &military(target), &full_history(), 7000.0, None, 1);
    assert!(no_mf(&b), "目標達成済み");

    // MilitaryFactory あり。
    let mut with_mf = states(Some(1.0), Some(1.0), 2);
    with_mf.get_mut(StateId(2)).unwrap().buildings.insert(BuildingType::MilitaryFactory, 1);
    let (b, _) = decide(&with_mf, &military(0), &full_history(), 7000.0, None, 1);
    assert!(no_mf(&b), "MilitaryFactory あり");

    // 履歴が 4 か月に満たない(旧 Save・New Game 直後)・古い・集計失敗を含む。
    let mut short = SupplyProductionHistory::default();
    for m in 7..=9u8 {
        short.record(1801, m, Some(0));
    }
    let (b, _) = decide(&good, &military(0), &short, 7000.0, None, 1);
    assert!(no_mf(&b), "履歴 3 か月");
    let (b, _) = decide(&good, &military(0), &SupplyProductionHistory::default(), 7000.0, None, 1);
    assert!(no_mf(&b), "履歴なし(旧 Save)");
    let (b, _) = decide(&good, &military(0), &history([Some(0), None, Some(0), Some(0)]), 7000.0, None, 1);
    assert!(no_mf(&b), "集計失敗の月を含む");

    // 国内供給がない: Mine なし・Mine の稼働率が未記録・Factory の稼働率が未記録(= IG の実績なし)。
    for (mine_op, factory_op, mine_lv, why) in [
        (None, Some(1.0), 0, "Mine なし"),
        (None, Some(1.0), 2, "Mine の稼働の実績なし(未集計)"),
        (Some(1.0), None, 2, "Factory の稼働の実績なし(IG の継続供給なし)"),
        (Some(0.0), Some(1.0), 2, "Mine が稼働していない(実績 0)"),
        (Some(1.0), Some(0.0), 2, "Factory が稼働していない(実績 0)"),
    ] {
        let sr = states(mine_op, factory_op, mine_lv);
        let (b, _) = decide(&sr, &military(0), &full_history(), 7000.0, None, 1);
        assert!(no_mf(&b), "{why}");
    }

    // 予備資金: 必要 1,200 + 100 × 3 = 1,500。1,499 では不発、1,500 で着工(包含的)。
    let (b, t) = decide(&good, &military(0), &full_history(), 1499.0, None, 1);
    assert!(no_mf(&b), "予備資金不足");
    assert!(t <= 1499.0);
    let (b, _) = decide(&good, &military(0), &full_history(), 1500.0, None, 1);
    assert!(b.iter().any(|(t, _)| *t == BuildingType::MilitaryFactory), "ちょうど 1,500 は着工");

    // 候補なし(増産見込みが 0 以下: 労働力 0 の州だけ)。
    let mut dead = states(Some(1.0), Some(1.0), 2);
    for s in dead.states.iter_mut() {
        s.employed_workforce = 0;
        s.unemployed_workforce = 0;
    }
    let (b, _) = decide(&dead, &military(0), &full_history(), 7000.0, None, 1);
    assert!(no_mf(&b), "増産見込みが 0 以下の州は選ばない");
}

/// 有限の備蓄だけでは供給源にならない: 備蓄が大量にあっても、稼働する Mine がなければ再建しない
/// (Oceanic への補助実験のように、残存 Iron を使い切って止まるだけの再建は許可しない)。
#[test]
fn a_finite_stock_alone_is_not_a_domestic_supply_source() {
    let sr = states(None, Some(1.0), 0); // 鉱床はあるが Mine なし
    let (b, _) = decide(&sr, &military(0), &full_history(), 7000.0, None, 1);
    assert!(b.iter().all(|(t, _)| *t != BuildingType::MilitaryFactory));
    // 備蓄は十分(Iron 1000・IG 500)だが、それでも再建しない。
    let cr = country(7000.0);
    assert!(cr.get(ME).unwrap().stockpile.get(ResourceType::Iron) >= 1000.0);
}

/// 建設中・同一更新中の先行注文を含め、MilitaryFactory は重複して発注しない。キュー上限(2)も超えない。
#[test]
fn no_duplicate_rebuild_orders_in_one_update_or_while_one_is_queued() {
    let sr = states(Some(1.0), Some(1.0), 2);
    for calls in [2, 5] {
        let (b, _) = decide(&sr, &military(0), &full_history(), 20_000.0, None, calls);
        let n = b.iter().filter(|(t, _)| *t == BuildingType::MilitaryFactory).count();
        assert_eq!(n, 1, "calls={calls}");
        assert!(b.len() <= 2);
    }
}

/// 優先順位: B1(優先度 2.8)の Mine が再建(2.9)より先、再建が通常判断(3)の Factory より先。
#[test]
fn priority_is_after_b1_and_before_the_normal_decision() {
    let sr = states(Some(1.0), Some(1.0), 2);
    let ctx = Some(IronSupplyContext { average_unmet: 46.0 });
    let (b, _) = decide(&sr, &military(0), &full_history(), 20_000.0, ctx, 1);
    assert_eq!(b.first().map(|(t, _)| *t), Some(BuildingType::Mine), "B1 が先");
    let (b, _) = decide(&sr, &military(0), &full_history(), 20_000.0, None, 1);
    assert_eq!(b.first().map(|(t, _)| *t), Some(BuildingType::MilitaryFactory), "再建が通常判断より先");
}

/// 再建が不発のとき、優先度 3 以降の結果は従来と同じ(文脈なしと一致)。
#[test]
fn when_the_rebuild_does_not_fire_the_legacy_decision_is_unchanged() {
    let sr = states(None, Some(1.0), 2); // 供給の実績なし → 不発
    let (with_ctx, _) = decide(&sr, &military(0), &full_history(), 7000.0, None, 1);
    let mut cr = country(7000.0);
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
        None,
    );
    assert_eq!(with_ctx, built(&cr), "不発のとき、従来の判断と同じ");
}

/// 正の産出と、師団を持続的に維持できることの区別: 低雇用の州の見込みは正だが、1 個の月需要
/// (約 30.4)に満たないことがある。診断関数で別に評価できる。
#[test]
fn positive_gain_is_distinct_from_sustaining_a_division() {
    let sr = states(Some(1.0), Some(1.0), 2);
    let reg = registry();
    let one_division = ((1_000_000u128 * 365).div_ceil(12)) as u64; // 日次 1.0 の月換算(units)
    let g1 = military_rebuild_gain_units(sr.get(StateId(1)).unwrap(), &reg).unwrap();
    let g2 = military_rebuild_gain_units(sr.get(StateId(2)).unwrap(), &reg).unwrap();
    assert!(g1 > 0 && g2 > 0, "どちらも正の産出");
    assert!(
        g1 < one_division || g2 < one_division,
        "低雇用の州では 1 個の月需要({one_division})に満たない見込みがありうる(g1={g1}, g2={g2})"
    );
}

#[allow(dead_code)]
fn _unused(_: MilitaryRebuildContext) {}
