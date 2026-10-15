//! P29-008-DIAG-001: 建設 AI が Iron 不足を「どの観測」で認識するかの制御診断。
//!
//! 同じ領土・設備・国庫・他の備蓄で、国家の Iron 備蓄だけを変えて `process_construction_ai`
//! の判断を比べる(状態を操作した補助実験であり、通常シナリオの再現ではない)。
//! 結果は**現状の仕様の記録**であり、望ましい恒久仕様の固定ではない。
//!
//! 観測(標準シナリオ、`p29_008_diag_001_construction_trace_test`): Elfin は Mine lv3→4 を
//! Iron 備蓄 16・28(< 30)の日に着工し、その後は Iron 備蓄が 46〜92(≥ 30)のため、Factory の
//! Iron 需要が Mine の産出を大きく超えていても(Factory lv39 の需要 約 700 / Mine 産出 約 100)
//! Mine を着工せず、Factory を建て続けた(国庫は 7,000〜10,000 で常に余裕)。

use std::collections::HashMap;

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::country_ai::{CountryAiRegistry, process_construction_ai};
use strategy_game::country::{CountryData, CountryRegistry};
use strategy_game::economy::resources::{ResourceType, StateResourceDeposit};
use strategy_game::state::data::{StateData, StateRegistry};

fn def(b: BuildingType, cost: f64, max_level: u32) -> BuildingDefinition {
    BuildingDefinition {
        building_type: b,
        name: format!("{b:?}"),
        construction_cost: cost,
        required_progress: 60.0,
        required_workforce: 1000.0,
        logistics_cost: 0.0,
        input_resources: HashMap::new(),
        output_resources: HashMap::new(),
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
        def(BuildingType::Mine, 500.0, 10),
        def(BuildingType::Factory, 1000.0, 10),
        def(BuildingType::MilitaryFactory, 1200.0, 10),
        def(BuildingType::Railway, 800.0, 5),
    ] {
        r.definitions.insert(d.building_type, d);
    }
    r
}

/// Elfin 1815 年頃の相当: Mine lv3(鉱床に増設余地あり)、Factory lv10、MilitaryFactory lv3。
fn states(owner: CountryId) -> StateRegistry {
    let s = |id: usize, buildings: &[(BuildingType, u32)], deposits: bool| StateData {
        id: StateId(id),
        name: format!("S{id}"),
        owner_country_id: owner,
        logistics_ratio: 1.0,
        buildings: buildings.iter().copied().collect(),
        resource_deposits: if deposits {
            vec![
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
            ]
        } else {
            vec![]
        },
        world_position: [id as f32 * 100.0, 0.0],
        size: [100.0, 100.0],
        ..Default::default()
    };
    StateRegistry::build(vec![
        s(1, &[(BuildingType::Mine, 3), (BuildingType::Factory, 5)], true),
        s(2, &[(BuildingType::Factory, 5), (BuildingType::MilitaryFactory, 3)], false),
    ])
}

fn decide(iron: f64) -> (BuildingType, f64) {
    let me = CountryId(1);
    let mut c = CountryData {
        id: me,
        name: "Elfin-like".to_string(),
        capital_state_id: StateId(2),
        treasury: 7000.0,
        ..Default::default()
    };
    c.stockpile.set(ResourceType::Food, 5000.0);
    c.stockpile.set(ResourceType::Wood, 5000.0);
    c.stockpile.set(ResourceType::Coal, 2000.0);
    c.stockpile.set(ResourceType::Iron, iron);
    c.stockpile.set(ResourceType::IndustrialGoods, 100.0);
    let mut cr = CountryRegistry { countries: vec![c] };
    let sr = states(me);
    let mut ai = CountryAiRegistry::default();
    let st = ai.get_or_create_mut(me);
    process_construction_ai(me, &mut cr, &sr, &registry(), st);
    let c = cr.get(me).unwrap();
    assert_eq!(c.construction_queue.len(), 1, "iron={iron}: 1 件着工する");
    (c.construction_queue[0].building_type, c.treasury)
}

/// Iron 備蓄が 30 未満の時だけ Mine を着工する。30 以上では、備蓄がどれだけ少なくても
/// (他の条件が同じなら)Factory を着工する。判定は備蓄の瞬間値の閾値で、Iron の需要(Factory・
/// MilitaryFactory の最大需要・実消費)・産出量・稼働率・不足量は参照しない。
#[test]
fn mine_is_chosen_only_below_the_iron_stock_threshold_of_30() {
    for iron in [0.0, 16.0, 28.0, 29.999] {
        assert_eq!(decide(iron).0, BuildingType::Mine, "iron={iron}");
    }
    for iron in [30.0, 31.0, 46.0, 92.0, 124.0, 5000.0] {
        assert_eq!(decide(iron).0, BuildingType::Factory, "iron={iron}");
    }
}

/// Factory が増えて Iron 需要が増えても(建物レベルだけを変えても)、備蓄が 30 以上なら
/// 判断は変わらない: Iron 不足(需要 ≫ 産出)は認識されない。
#[test]
fn a_larger_factory_demand_does_not_change_the_decision_while_the_stock_is_at_least_30() {
    let me = CountryId(1);
    let build = |factory_levels: u32| {
        let mut c = CountryData {
            id: me,
            name: "Elfin-like".to_string(),
            capital_state_id: StateId(2),
            treasury: 7000.0,
            ..Default::default()
        };
        for (r, v) in [
            (ResourceType::Food, 5000.0),
            (ResourceType::Wood, 5000.0),
            (ResourceType::Coal, 2000.0),
            (ResourceType::Iron, 90.0),
            (ResourceType::IndustrialGoods, 100.0),
        ] {
            c.stockpile.set(r, v);
        }
        let mut cr = CountryRegistry { countries: vec![c] };
        let mut sr = states(me);
        for s in sr.states.iter_mut() {
            if s.building_level(BuildingType::Factory) > 0 {
                s.buildings.insert(BuildingType::Factory, factory_levels);
            }
        }
        let mut ai = CountryAiRegistry::default();
        let st = ai.get_or_create_mut(me);
        process_construction_ai(me, &mut cr, &sr, &registry(), st);
        cr.get(me).unwrap().construction_queue[0].building_type
    };
    assert_eq!(build(1), BuildingType::Factory);
    assert_eq!(build(9), BuildingType::Factory);
}
