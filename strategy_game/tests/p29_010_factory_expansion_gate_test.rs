//! P29-010: 入力(Iron)不足で低稼働の Factory の増設抑止(B2)の、実際の判定経路での検証。
//!
//! 履歴の窓・署名・復帰の単体検証は `economy::factory_starvation` の単体テスト(5 件)を参照
//! (重複して追加しない)。ここでは (1) 月次生産が飢餓 entry を正しく観測すること、
//! (2) 建設 AI の実際の判断経路で除外・保留・復帰・B1 の維持が成立することを確認する。

use std::collections::HashMap;

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::country_ai::{
    CountryAiRegistry, IronSupplyContext, factory_expansion_blocked,
    process_construction_ai_with_contexts,
};
use strategy_game::country::{CountryData, CountryRegistry};
use strategy_game::economy::factory_starvation::{FactoryStarvationHistory, factory_signature};
use strategy_game::economy::production::process_country_production_with_outcome;
use strategy_game::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use strategy_game::state::data::{StateData, StateRegistry};

const ME: CountryId = CountryId(1);

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
        required_workforce: 1000.0,
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

// ── 月次生産の観測 ────────────────────────────────────────────────────

fn factory_state(id: usize, level: u32, employed: u64) -> StateData {
    StateData {
        id: StateId(id),
        owner_country_id: ME,
        population: 1_000_000,
        workforce_ratio: 0.5,
        employed_workforce: employed, // 500_000 で労働力比 1.0
        logistics_ratio: 1.0,
        buildings: HashMap::from([(BuildingType::Factory, level)]),
        ..Default::default()
    }
}

fn observe(states: Vec<StateData>, iron: f64, bankrupt: bool) -> (u32, u32) {
    let reg = registry();
    let mut states = states;
    let mut stock = CountryStockpile::new();
    stock.set(ResourceType::Iron, iron);
    stock.set(ResourceType::Coal, 1.0e9);
    let o = {
        let mut refs: Vec<&mut StateData> = states.iter_mut().collect();
        process_country_production_with_outcome(&mut refs, &mut stock, &reg, bankrupt)
    };
    (o.factory_entries, o.factory_iron_starved_entries)
}

/// Iron 不足が実際の制約で稼働率が 0.1 未満の entry だけを数える。0.1 ちょうどは含めない。
#[test]
fn only_iron_bound_entries_below_one_tenth_are_counted() {
    // Factory lv1: Iron 需要 20。備蓄 1.9 → 充足率 0.095(< 0.1)、2.0 → 0.1 ちょうど。
    assert_eq!(observe(vec![factory_state(1, 1, 500_000)], 1.9, false), (1, 1));
    assert_eq!(
        observe(vec![factory_state(1, 1, 500_000)], 2.0, false),
        (1, 0),
        "0.1 ちょうどは「0.1 未満」に含めない"
    );
    assert_eq!(observe(vec![factory_state(1, 1, 500_000)], 100.0, false), (1, 0));
    // 実績としての稼働率 0(Iron 備蓄 0)でも、Iron が制約なら数える。
    assert_eq!(observe(vec![factory_state(1, 1, 500_000)], 0.0, false), (1, 1));
}

/// 労働力・破産による低稼働は、入力不足が制約ではないので数えない。複数の制約が同率なら
/// Iron が「厳密な制約」ではないため数えない。
#[test]
fn low_operation_from_other_constraints_is_not_counted() {
    // 労働力比 0.05(25,000 / 500,000): 稼働率 0.05 は労働力による。Iron は十分。
    assert_eq!(observe(vec![factory_state(1, 1, 25_000)], 1000.0, false), (1, 0));
    // 労働力比 0.05 と Iron 充足率 0.05 が同率(同時に律速): Iron は厳密な制約ではない。
    assert_eq!(
        observe(vec![factory_state(1, 1, 25_000)], 1.0, false),
        (1, 0),
        "Iron の充足率 0.05 と労働力 0.05 が同率なら数えない"
    );
    // 労働力が 0.05 で、Iron がそれより厳しい(0.01): Iron が厳密な制約 → 数える。
    assert_eq!(observe(vec![factory_state(1, 1, 25_000)], 0.2, false), (1, 1));
    // 労働力が 0(稼働率 0): Iron がなくても 0。Iron の制約ではない。
    assert_eq!(observe(vec![factory_state(1, 1, 0)], 0.0, false), (1, 0));
}

/// 国家の全 entry が対象でなければ「飢餓の月」にならない(1 つでも他の理由で低稼働なら除外しない)。
/// ここでは entry ごとの観測数で、全 entry 一致の条件(entries == starved)を確認する。
#[test]
fn a_mixed_fleet_is_not_entirely_starved() {
    // 州 1: Iron 不足で飢餓、州 2: 労働力 0(Iron の制約ではない)。国家としては全 entry ではない。
    let (entries, starved) = observe(
        vec![factory_state(1, 1, 500_000), factory_state(2, 1, 0)],
        0.0,
        false,
    );
    assert_eq!(entries, 2);
    assert_eq!(starved, 1);
    assert!(starved != entries);
}

// ── 建設 AI の判断経路 ────────────────────────────────────────────────

fn iron_state(id: usize, mine: u32, factory: u32) -> StateData {
    StateData {
        id: StateId(id),
        name: format!("S{id}"),
        owner_country_id: ME,
        logistics_ratio: 1.0,
        buildings: [(BuildingType::Mine, mine), (BuildingType::Factory, factory)]
            .into_iter()
            .filter(|(_, l)| *l > 0)
            .collect(),
        building_operation_ratios: if mine > 0 {
            HashMap::from([(BuildingType::Mine, 1.0)])
        } else {
            HashMap::new()
        },
        resource_deposits: vec![StateResourceDeposit {
            resource_type: ResourceType::Iron,
            base_output: 50.0,
            discovered: true,
            development_level: 1,
        }],
        world_position: [id as f32 * 100.0, 0.0],
        size: [100.0, 100.0],
        ..Default::default()
    }
}

fn decide(
    iron: f64,
    treasury: f64,
    blocked: bool,
    iron_ctx: Option<IronSupplyContext>,
    calls: usize,
) -> (Vec<BuildingType>, f64) {
    let mut c = CountryData {
        id: ME,
        name: "Elfin-like".to_string(),
        capital_state_id: StateId(2),
        treasury,
        monthly_expenses: 100.0,
        ..Default::default()
    };
    for (r, v) in [
        (ResourceType::Food, 5000.0),
        (ResourceType::Wood, 5000.0),
        (ResourceType::Coal, 2000.0),
        (ResourceType::Iron, iron),
        (ResourceType::IndustrialGoods, 100.0),
    ] {
        c.stockpile.set(r, v);
    }
    let mut cr = CountryRegistry { countries: vec![c] };
    let sr = StateRegistry::build(vec![iron_state(1, 3, 5), iron_state(2, 0, 5)]);
    let mut ai = CountryAiRegistry::default();
    for _ in 0..calls {
        process_construction_ai_with_contexts(
            ME,
            &mut cr,
            &sr,
            &registry(),
            ai.get_or_create_mut(ME),
            None,
            iron_ctx,
            blocked,
            None,
        );
    }
    let c = cr.get(ME).unwrap();
    (
        c.construction_queue.iter().map(|q| q.building_type).collect(),
        c.treasury,
    )
}

/// 除外すると、Factory の建設注文は入らず、予算・キュー枠を消費せず、保留になる(何度繰り返しても
/// 同じ候補を選ぶループや、代替建設への支出移動がない)。除外しなければ従来どおり Factory。
#[test]
fn a_blocked_factory_expansion_is_held_without_spending() {
    let (b, t) = decide(90.0, 7000.0, false, None, 1);
    assert_eq!(b, vec![BuildingType::Factory], "従来の判断");
    assert_eq!(t, 6000.0);
    for calls in [1, 2, 5] {
        let (b, t) = decide(90.0, 7000.0, true, None, calls);
        assert!(b.is_empty(), "除外後は何も着工しない(calls={calls})");
        assert_eq!(t, 7000.0, "予算を消費しない(calls={calls})");
    }
}

/// B1 の Mine 判断は維持される(Factory を除外したことを理由に Mine を無条件には建てないが、
/// B1 の文脈があれば従来どおり Mine を着工する)。
#[test]
fn the_b1_mine_decision_is_unaffected_and_not_triggered_by_the_exclusion_alone() {
    let ctx = IronSupplyContext { average_unmet: 46.0 };
    let (b, _) = decide(90.0, 7000.0, true, Some(ctx), 1);
    assert_eq!(b, vec![BuildingType::Mine]);
    // Factory 除外だけでは(B1 の不足がなければ)Mine を建てない。
    let (b, _) = decide(90.0, 7000.0, true, None, 1);
    assert!(b.is_empty());
}

/// 原材料不足(Iron < 30)の Mine 判断は B2 の影響を受けない。
#[test]
fn raw_material_shortage_mine_priority_is_unaffected() {
    let (b, _) = decide(10.0, 7000.0, true, None, 1);
    assert_eq!(b, vec![BuildingType::Mine]);
}

/// 履歴 → 判定: 設備署名が現在と一致する直近 4 か月がすべて飢餓なら抑止。Factory がない国
/// (初回建設)、増設・完成で署名が変わった直後、履歴不足は抑止しない。復帰は直ちに起きる。
#[test]
fn history_to_decision_exclusion_and_recovery() {
    let sr = StateRegistry::build(vec![iron_state(1, 3, 5), iron_state(2, 0, 5)]);
    let sig = factory_signature(&[(1, 5), (2, 5)]);
    let mut h = FactoryStarvationHistory::default();
    for m in 1..=3u8 {
        h.record(1815, m, sig, true);
    }
    assert!(!factory_expansion_blocked(&h, "1815/03/20", ME, &sr), "3 か月では除外しない");
    h.record(1815, 4, sig, true);
    assert!(factory_expansion_blocked(&h, "1815/04/20", ME, &sr), "4 か月で除外");
    // 設備変更(州 2 の Factory が増設された)後は、別の設備状態の実績なので除外しない。
    let upgraded = StateRegistry::build(vec![iron_state(1, 3, 5), iron_state(2, 0, 6)]);
    assert!(!factory_expansion_blocked(&h, "1815/04/20", ME, &upgraded));
    // Factory がない国(初回建設)は除外しない。
    let none = StateRegistry::build(vec![iron_state(1, 3, 0)]);
    assert!(!factory_expansion_blocked(&h, "1815/04/20", ME, &none));
    // 入力不足の解消(飢餓でない月)で直ちに復帰する。
    h.record(1815, 5, sig, false);
    assert!(!factory_expansion_blocked(&h, "1815/05/20", ME, &sr));
    // 日付が解釈できない場合は除外しない。
    assert!(!factory_expansion_blocked(&h, "bad", ME, &sr));
}
