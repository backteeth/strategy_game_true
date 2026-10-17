//! P29-009: Iron のフロー不足を反映する建設 AI(B1)の、同じ入力条件での直接検証。
//!
//! 旧判定(`process_construction_ai`: 備蓄の瞬間値の閾値 30)と新判定(Iron 不足の文脈つき)を
//! 同じ国家・設備・備蓄・国庫で比較する。通常シナリオでの結果は別(標準シナリオの比較で報告)。

use std::collections::HashMap;

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::country_ai::{
    CountryAiRegistry, IronSupplyContext, iron_supply_context, process_construction_ai,
    process_construction_ai_with_contexts,
};
use strategy_game::country::{CountryData, CountryRegistry};
use strategy_game::economy::iron_shortage::{IronShortageAssessment, IronShortageHistory};
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

fn iron_state(id: usize, mine: u32, factory: u32, iron_base: f64) -> StateData {
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
            base_output: iron_base,
            discovered: true,
            development_level: 1,
        }],
        world_position: [id as f32 * 100.0, 0.0],
        size: [100.0, 100.0],
        ..Default::default()
    }
}

fn no_deposit_state(id: usize, factory: u32) -> StateData {
    StateData {
        id: StateId(id),
        name: format!("S{id}"),
        owner_country_id: ME,
        logistics_ratio: 1.0,
        buildings: [(BuildingType::Factory, factory), (BuildingType::MilitaryFactory, 3)]
            .into_iter()
            .collect(),
        world_position: [id as f32 * 100.0, 0.0],
        size: [100.0, 100.0],
        ..Default::default()
    }
}

fn country(treasury: f64, iron: f64) -> CountryRegistry {
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
    CountryRegistry { countries: vec![c] }
}

fn world(states: Vec<StateData>) -> StateRegistry {
    StateRegistry::build(states)
}

/// 継続的な不足を表す文脈(MilitaryFactory の Iron 未充足 46/月)。
const SUSTAINED_46: IronSupplyContext = IronSupplyContext { average_unmet: 46.0 };

fn built(cr: &CountryRegistry) -> Vec<BuildingType> {
    cr.get(ME)
        .unwrap()
        .construction_queue
        .iter()
        .map(|q| q.building_type)
        .collect()
}

fn old_decision(iron: f64, sr: &StateRegistry) -> Vec<BuildingType> {
    let mut cr = country(7000.0, iron);
    let mut ai = CountryAiRegistry::default();
    process_construction_ai(ME, &mut cr, sr, &registry(), ai.get_or_create_mut(ME));
    built(&cr)
}

fn new_decision(
    iron: f64,
    sr: &StateRegistry,
    ctx: Option<IronSupplyContext>,
    calls: usize,
    treasury: f64,
) -> (Vec<BuildingType>, f64) {
    let mut cr = country(treasury, iron);
    let mut ai = CountryAiRegistry::default();
    for _ in 0..calls {
        process_construction_ai_with_contexts(
            ME,
            &mut cr,
            sr,
            &registry(),
            ai.get_or_create_mut(ME),
            None,
            ctx,
            false,
            None,
        );
    }
    (built(&cr), cr.get(ME).unwrap().treasury)
}

fn two_iron_states() -> StateRegistry {
    world(vec![
        iron_state(1, 3, 5, 50.0),
        iron_state(3, 1, 0, 40.0),
        no_deposit_state(2, 5),
    ])
}

/// 備蓄 30 以上でも、継続的な Iron 不足(文脈あり)なら Mine を着工する(旧: Factory)。
#[test]
fn a_sustained_iron_shortage_selects_mine_even_when_the_stock_is_at_least_30() {
    let sr = two_iron_states();
    for iron in [30.0, 90.0, 124.0] {
        assert_eq!(old_decision(iron, &sr), vec![BuildingType::Factory], "旧判定 iron={iron}");
        let (b, treasury) = new_decision(iron, &sr, Some(SUSTAINED_46), 1, 7000.0);
        assert_eq!(b, vec![BuildingType::Mine], "新判定 iron={iron}");
        assert_eq!(treasury, 7000.0 - 500.0, "Mine の既存の建設費だけが国庫から引かれる");
    }
}

/// 文脈なし(未集計・不足なし・一時的な不足)は、従来の判定と完全に同じ。
#[test]
fn without_a_sustained_shortage_the_decision_is_the_legacy_one() {
    let sr = two_iron_states();
    for iron in [10.0, 29.0, 30.0, 90.0] {
        let (b, _) = new_decision(iron, &sr, None, 1, 7000.0);
        assert_eq!(b, old_decision(iron, &sr), "iron={iron}");
    }
}

/// 建設中の Mine の増分が不足を満たす場合は、同じ不足で重複して発注しない
/// (同一更新中の先行注文も、建設キューとして同じ規則で計上される)。
#[test]
fn queued_and_same_update_mine_orders_are_not_double_counted() {
    let sr = two_iron_states();
    // 不足 46、1 レベルの見込み 50(州 1)・40(州 3)。1 件目: 見込み最大の州 1 に Mine。
    // 2 件目(同じ更新): 建設中の 50 が不足 46 を満たす → B1 は着工しない(優先度 3 の Factory)。
    let (b, _) = new_decision(90.0, &sr, Some(SUSTAINED_46), 2, 7000.0);
    assert_eq!(b, vec![BuildingType::Mine, BuildingType::Factory]);
    // 不足 120 なら、建設中の 50 を差し引いた残り 70 に対して 2 件目の Mine(州 3 の 40 見込み)。
    let big = IronSupplyContext { average_unmet: 120.0 };
    let (b, _) = new_decision(90.0, &sr, Some(big), 2, 7000.0);
    assert_eq!(b, vec![BuildingType::Mine, BuildingType::Mine]);
    // それでも建設キューの上限(2 件)を超えない。
    let (b, _) = new_decision(90.0, &sr, Some(big), 4, 7000.0);
    assert_eq!(b.len(), 2);
    assert!(b.iter().filter(|x| **x == BuildingType::Mine).count() <= 2);
}

/// 不足が 1 レベルの見込みの半分未満(目標をほぼ満たした)なら着工しない。
#[test]
fn a_negligible_remaining_shortage_does_not_trigger_a_mine() {
    let sr = two_iron_states();
    let small = IronSupplyContext { average_unmet: 20.0 }; // 見込み最大 50 の半分(25)未満
    let (b, _) = new_decision(90.0, &sr, Some(small), 1, 7000.0);
    assert_eq!(b, vec![BuildingType::Factory]);
}

/// 予算の制約(建設費 + 月次経常支出 × 3 か月分)は優先度 3 と同じ。満たさなければ何も着工しない。
#[test]
fn the_existing_reserve_rule_applies() {
    let sr = two_iron_states();
    // 必要 500 + 100 × 3 = 800。国庫 790 では Mine を着工せず、Factory(1000 + 300)も不可。
    let (b, treasury) = new_decision(90.0, &sr, Some(SUSTAINED_46), 1, 790.0);
    assert!(b.is_empty());
    assert_eq!(treasury, 790.0);
    // ちょうど 800 なら着工できる(包含的)。
    let (b, _) = new_decision(90.0, &sr, Some(SUSTAINED_46), 1, 800.0);
    assert_eq!(b, vec![BuildingType::Mine]);
}

/// Iron 鉱床がない・Mine が増設できない場合は、不足でも Mine を着工しない(建設不可を許可しない)。
#[test]
fn no_usable_iron_deposit_or_headroom_means_no_mine() {
    let none = world(vec![no_deposit_state(2, 5)]);
    let (b, _) = new_decision(90.0, &none, Some(SUSTAINED_46), 1, 7000.0);
    assert_eq!(b, vec![BuildingType::Factory], "鉱床なし → 従来判断(Factory)");
    let maxed = world(vec![iron_state(1, 10, 5, 50.0), no_deposit_state(2, 5)]);
    let (b, _) = new_decision(90.0, &maxed, Some(SUSTAINED_46), 1, 7000.0);
    assert_eq!(b, vec![BuildingType::Factory], "Mine が最大レベル → 従来判断");
}

/// 巨大な Factory 需要(= 巨大な未充足の観測)があっても、1 回の判断で着工するのは 1 レベルで、
/// 建設キューの上限(2)を超えない。需要の源泉は MilitaryFactory の未充足(下の生産テスト)に限られる。
#[test]
fn a_huge_shortage_is_capped_by_one_level_per_decision_and_the_queue_limit() {
    let sr = two_iron_states();
    let huge = IronSupplyContext { average_unmet: 100_000.0 };
    let (b, _) = new_decision(90.0, &sr, Some(huge), 1, 100_000.0);
    assert_eq!(b, vec![BuildingType::Mine]);
    let (b, _) = new_decision(90.0, &sr, Some(huge), 10, 100_000.0);
    assert!(b.len() <= 2);
}

/// 履歴 → 文脈: 継続的な不足だけが文脈になる。未集計・不足なし・Mine レベルの変化直後は`None`。
#[test]
fn only_a_sustained_shortage_in_the_history_becomes_a_context() {
    let sr = two_iron_states(); // Mine 合計 = 3 + 1 = 4
    let mut h = IronShortageHistory::default();
    for m in 1..=3u8 {
        h.record(1815, m, Some(46.0), 4);
    }
    assert!(iron_supply_context(&h, "1815/03/20", ME, &sr).is_none(), "3 か月では未集計");
    h.record(1815, 4, Some(46.0), 4);
    let ctx = iron_supply_context(&h, "1815/04/20", ME, &sr).expect("4 か月の継続的な不足");
    assert!((ctx.average_unmet - 46.0).abs() < 1e-9);
    assert!(iron_supply_context(&h, "1815/06/20", ME, &sr).is_none(), "古い履歴");
    // Mine レベルが現在と異なる(完成直後で実績に未反映)なら未集計。
    let more_mines = world(vec![iron_state(1, 4, 5, 50.0), iron_state(3, 1, 0, 40.0)]);
    assert!(iron_supply_context(&h, "1815/04/20", ME, &more_mines).is_none());
    // 一時的な不足(0 の月を含む)は文脈にならない。
    let mut t = IronShortageHistory::default();
    for (m, v) in [(1u8, 46.0), (2, 0.0), (3, 46.0), (4, 46.0)] {
        t.record(1815, m, Some(v), 4);
    }
    assert_eq!(t.assess((1815, 4), 4), IronShortageAssessment::NoSustainedShortage);
    assert!(iron_supply_context(&t, "1815/04/20", ME, &sr).is_none());
    // 日付が解釈できなければ文脈なし(従来判断)。
    assert!(iron_supply_context(&h, "bad", ME, &sr).is_none());
}

/// 生産側の観測: 実消費が Iron の供給量と一致していても、未充足の要求を検出する。
/// Iron 産出 100(Mine)に対し MilitaryFactory(lv4)の要求は 120。消費 = 供給 = 100 で
/// 「均衡」に見えるが、未充足は 20(= 120 − 100)として観測される。
#[test]
fn unmet_iron_is_observed_even_when_consumption_equals_supply() {
    let reg = registry();
    let mut state = StateData {
        id: StateId(1),
        owner_country_id: ME,
        population: 1_000_000,
        workforce_ratio: 0.5,
        employed_workforce: 500_000,
        logistics_ratio: 1.0,
        buildings: HashMap::from([(BuildingType::Mine, 1), (BuildingType::MilitaryFactory, 4)]),
        resource_deposits: vec![StateResourceDeposit {
            resource_type: ResourceType::Iron,
            base_output: 100.0,
            discovered: true,
            development_level: 1,
        }],
        ..Default::default()
    };
    let mut stock = CountryStockpile::new();
    stock.set(ResourceType::Iron, 100.0); // 前月の産出の持ち越し(月産 = 備蓄)
    stock.set(ResourceType::IndustrialGoods, 1.0e6);
    let outcome = {
        let mut refs: Vec<&mut StateData> = vec![&mut state];
        process_country_production_with_outcome(&mut refs, &mut stock, &reg, false)
    };
    // 備蓄は月産(100)に戻り、閾値 30 の判定では不足に見えない。
    assert!(stock.get(ResourceType::Iron) >= 99.0, "iron end = {}", stock.get(ResourceType::Iron));
    assert!((outcome.iron_produced - 100.0).abs() < 1e-9);
    assert!(
        (outcome.iron_unmet_military - 20.0).abs() < 1e-3,
        "unmet = {}",
        outcome.iron_unmet_military
    );
    // 未充足は MilitaryFactory の最大要求(30 × lv4 = 120)を超えない(巨大な需要の上限)。
    assert!(outcome.iron_unmet_military <= 120.0 + 1e-9);
}

/// Iron 以外の入力(IG)が律速している場合は、Iron の未充足に数えない。
#[test]
fn a_shortage_caused_by_another_input_is_not_counted_as_iron() {
    let reg = registry();
    let mut state = StateData {
        id: StateId(1),
        owner_country_id: ME,
        population: 1_000_000,
        workforce_ratio: 0.5,
        employed_workforce: 500_000,
        logistics_ratio: 1.0,
        buildings: HashMap::from([(BuildingType::MilitaryFactory, 4)]),
        ..Default::default()
    };
    let mut stock = CountryStockpile::new();
    stock.set(ResourceType::Iron, 1000.0);
    stock.set(ResourceType::IndustrialGoods, 15.0); // IG が律速(需要 60 に対し 15)
    let outcome = {
        let mut refs: Vec<&mut StateData> = vec![&mut state];
        process_country_production_with_outcome(&mut refs, &mut stock, &reg, false)
    };
    assert_eq!(outcome.iron_unmet_military, 0.0);
}

/// Iron が十分にあれば未充足は 0。
#[test]
fn no_unmet_iron_when_supply_is_sufficient() {
    let reg = registry();
    let mut state = StateData {
        id: StateId(1),
        owner_country_id: ME,
        population: 1_000_000,
        workforce_ratio: 0.5,
        employed_workforce: 500_000,
        logistics_ratio: 1.0,
        buildings: HashMap::from([(BuildingType::MilitaryFactory, 4)]),
        ..Default::default()
    };
    let mut stock = CountryStockpile::new();
    stock.set(ResourceType::Iron, 1000.0);
    stock.set(ResourceType::IndustrialGoods, 1000.0);
    let outcome = {
        let mut refs: Vec<&mut StateData> = vec![&mut state];
        process_country_production_with_outcome(&mut refs, &mut stock, &reg, false)
    };
    assert_eq!(outcome.iron_unmet_military, 0.0);
    assert_eq!(outcome.iron_unmet_other, 0.0);
}
