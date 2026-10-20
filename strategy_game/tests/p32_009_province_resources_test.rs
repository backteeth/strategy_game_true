//! P32-009: プロヴィンス別の天然資源の入力 → 所属ステートへの集計 → 既存の生産計算への接続のテスト。
//! 値は検証用で、正式な資源分布・ゲームバランスではない。既存の`StateResourceDeposit`の意味
//! (`base_output` = Mine 1レベルあたりの月間産出能力。埋蔵量・在庫ではない)を維持する。

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::economy::map_state::{
    EconomyDataIssue, EconomyScenario, MapStateEconomyInput, MapStateEconomyRegistry,
    MultiMonthScenario, parse_economy_scenario, parse_multi_month_scenario,
    stockpile_from_scenario,
};
use strategy_game::economy::province_resources::{
    ProvinceResourceEntry, ProvinceResourcesFile, ResourceIssue, ResourceScope, StateResourceSetup,
    derive_state_resources, parse_province_resources,
};
use strategy_game::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use strategy_game::world::{
    MapStateId, ProvinceId, WorldModel, load_province_table, load_state_layout,
};

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

fn res_file() -> ProvinceResourcesFile {
    parse_province_resources(&std::fs::read_to_string(RES_PATH).unwrap()).unwrap()
}

fn dep(r: ResourceType, out: f64, discovered: bool, level: u32) -> StateResourceDeposit {
    StateResourceDeposit {
        resource_type: r,
        base_output: out,
        discovered,
        development_level: level,
    }
}

fn entry(province: u32, deposits: Vec<StateResourceDeposit>) -> ProvinceResourceEntry {
    ProvinceResourceEntry { province, deposits }
}

fn s(id: u32) -> MapStateId {
    MapStateId(id)
}

fn single_scenario() -> EconomyScenario {
    parse_economy_scenario(
        &std::fs::read_to_string("assets/data/world/preview/economy_scenario.ron").unwrap(),
    )
    .unwrap()
}

fn multi_scenario() -> MultiMonthScenario {
    parse_multi_month_scenario(
        &std::fs::read_to_string("assets/data/world/preview/economy_scenario_m4b.ron").unwrap(),
    )
    .unwrap()
}

fn strip_deposits(states: &mut [MapStateEconomyInput]) {
    for st in states {
        st.resource_deposits.clear();
    }
}

fn stock_vec(st: &CountryStockpile) -> Vec<(ResourceType, f64)> {
    ResourceType::ALL.iter().map(|r| (*r, st.get(*r))).collect()
}

// ---------------------------------------------------------------- 入力

#[test]
fn preview_file_loads_and_derives() {
    let (w, f) = (world(), res_file());
    assert_eq!(f.scope, ResourceScope::Partial);
    let d = derive_state_resources(&w, &f).unwrap();
    assert_eq!(d.scope(), ResourceScope::Partial);
    assert_eq!(d.states().len(), 3);
}

#[test]
fn unsupported_version_unknown_and_ambiguous_inputs_are_rejected() {
    let w = world();
    let text = std::fs::read_to_string(RES_PATH).unwrap();
    // 非対応バージョン(パーサーで明確に失敗)
    let e = parse_province_resources(&text.replace("format_version: 1", "format_version: 3"))
        .unwrap_err();
    assert!(e.contains("unsupported format_version 3"), "{e}");
    // 不正な資源種別(既存のResourceTypeに無い)
    assert!(
        parse_province_resources(
            &text.replace("Coal, base_output: 30.0", "Mithril, base_output: 30.0")
        )
        .is_err()
    );
    let derive = |f: &ProvinceResourcesFile| derive_state_resources(&w, f).unwrap_err();
    // 存在しないProvinceId
    let mut f = res_file();
    f.provinces.push(entry(5, vec![]));
    assert!(derive(&f).contains(&ResourceIssue::UnknownProvince {
        province: ProvinceId(5)
    }));
    // 同じプロヴィンスの重複定義(曖昧)
    let mut f = res_file();
    f.provinces.push(entry(18309, vec![]));
    assert!(derive(&f).contains(&ResourceIssue::DuplicateProvince {
        province: ProvinceId(18309)
    }));
    // 同じプロヴィンス内の同種資源の重複(曖昧)
    let mut f = res_file();
    f.provinces[2]
        .deposits
        .push(dep(ResourceType::Iron, 1.0, true, 1));
    assert!(
        derive(&f).contains(&ResourceIssue::DuplicateResourceInProvince {
            province: ProvinceId(18309),
            resource: ResourceType::Iron
        })
    );
    // 不正値: 負・NaN・無限大
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        let mut f = res_file();
        f.provinces[2].deposits[0].base_output = bad;
        assert!(
            derive(&f)
                .iter()
                .any(|i| matches!(i, ResourceIssue::InvalidValue { .. })),
            "{bad}"
        );
    }
    // 非対応バージョンの構造体も導出で拒否
    let mut f = res_file();
    f.format_version = 9;
    assert!(derive(&f).contains(&ResourceIssue::UnsupportedVersion { found: 9 }));
}

#[test]
fn world_scope_requires_every_province_to_be_listed() {
    let w = world();
    let mut f = res_file();
    f.scope = ResourceScope::World;
    let e = derive_state_resources(&w, &f).unwrap_err();
    assert!(e.iter().any(
        |i| matches!(i, ResourceIssue::WorldScopeIncomplete { missing } if !missing.is_empty())
    ));
    // 全プロヴィンス(未割当を含む29件)を列挙すれば受理される
    let mut all = res_file();
    all.scope = ResourceScope::World;
    for p in w.provinces() {
        if !all.provinces.iter().any(|e| e.province == p.id.0) {
            all.provinces.push(entry(p.id.0, vec![]));
        }
    }
    assert!(derive_state_resources(&w, &all).is_ok());
}

// ---------------------------------------------------------------- 集計(手計算)

#[test]
fn preview_aggregation_matches_hand_computation() {
    let (w, f) = (world(), res_file());
    let d = derive_state_resources(&w, &f).unwrap();
    // 20: 同一ステート内の複数プロヴィンス: Iron 25 + 15 = 40(発見済み)、Coal 30。
    let a = d.state(s(20)).unwrap();
    assert_eq!(
        a.deposits,
        vec![
            dep(ResourceType::Iron, 40.0, true, 2),
            dep(ResourceType::Coal, 30.0, true, 1)
        ]
    );
    assert_eq!(a.sources.len(), 3);
    assert_eq!(a.setup, StateResourceSetup::Complete);
    assert_eq!(
        (
            a.configured_provinces,
            a.explicit_none_provinces,
            a.unset_provinces.len()
        ),
        (7, 4, 0)
    );
    // 21: 別ステートの同種資源。発見済み 20 と未発見 12 は別の鉱床として保持(加算しない)。
    let b = d.state(s(21)).unwrap();
    assert_eq!(
        b.deposits,
        vec![
            dep(ResourceType::Iron, 20.0, true, 1),
            dep(ResourceType::Iron, 12.0, false, 1)
        ]
    );
    assert_eq!(b.setup, StateResourceSetup::Incomplete);
    assert_eq!(
        (
            b.configured_provinces,
            b.explicit_none_provinces,
            b.unset_provinces.len()
        ),
        (3, 1, 10)
    );
    // 22: 未設定(資源ゼロとは別)
    let c = d.state(s(22)).unwrap();
    assert!(c.deposits.is_empty() && c.sources.is_empty());
    assert_eq!(c.setup, StateResourceSetup::Unconfigured);
    assert_eq!(c.unset_provinces.len(), 6);
    // 未割当プロヴィンスの資源(Iron 99)は、どのステートにも集計されない(架空のステートも作らない)
    assert_eq!(d.unassigned_configured(), &[ProvinceId(17903)]);
    let iron_all: f64 = d
        .states()
        .iter()
        .flat_map(|st| st.deposits.iter())
        .filter(|x| x.resource_type == ResourceType::Iron && x.discovered)
        .map(|x| x.base_output)
        .sum();
    assert_eq!(
        iron_all, 60.0,
        "40 + 20; the unassigned 99 and the undiscovered 12 are not mixed in"
    );
    // 出典はProvinceId昇順で、各プロヴィンスは1回だけ
    let prov: Vec<u32> = a.sources.iter().map(|x| x.province.0).collect();
    assert_eq!(prov, vec![18309, 18310, 18311]);
    assert!(
        !d.all_states_complete(),
        "partial setup must not be reported as complete"
    );
}

#[test]
fn aggregation_does_not_depend_on_input_order() {
    let w = world();
    let f = res_file();
    let base = derive_state_resources(&w, &f).unwrap();
    let mut g = f.clone();
    g.provinces.reverse();
    for e in &mut g.provinces {
        e.deposits.reverse();
    }
    let shuffled = derive_state_resources(&w, &g).unwrap();
    assert_eq!(base, shuffled);
}

#[test]
fn overflowing_totals_are_rejected_not_silently_infinite() {
    let w = world();
    let mut f = res_file();
    f.provinces[2].deposits = vec![dep(ResourceType::Iron, f64::MAX, true, 1)];
    f.provinces[3].deposits = vec![dep(ResourceType::Iron, f64::MAX, true, 1)];
    let e = derive_state_resources(&w, &f).unwrap_err();
    assert!(e.contains(&ResourceIssue::NonFiniteTotal {
        state: s(20),
        resource: ResourceType::Iron
    }));
}

#[test]
fn input_files_are_never_modified_by_loading_or_deriving() {
    let before = std::fs::read(RES_PATH).unwrap();
    let w = world();
    let f = parse_province_resources(&String::from_utf8(before.clone()).unwrap()).unwrap();
    let _ = derive_state_resources(&w, &f).unwrap();
    assert_eq!(before, std::fs::read(RES_PATH).unwrap());
}

// ---------------------------------------------------------------- 変更への追従(データ層の再導出)

#[test]
fn membership_change_moves_the_resource_and_invalidates_the_old_aggregate() {
    let (w, f) = (world(), res_file());
    let old = derive_state_resources(&w, &f).unwrap();
    assert!(old.is_current_for(&w, &f));
    // 18310(Iron 15)を MapState#20 から #21 へ移した所属データ(占領や和平ではなく、データ層の再導出)
    let table = load_province_table("assets/data/world/preview/provinces.ron").unwrap();
    let mut layout = load_state_layout("assets/data/world/preview/states.ron").unwrap();
    layout
        .states
        .iter_mut()
        .find(|st| st.id == 20)
        .unwrap()
        .provinces
        .retain(|p| *p != 18310);
    layout
        .states
        .iter_mut()
        .find(|st| st.id == 21)
        .unwrap()
        .provinces
        .push(18310);
    let w2 = WorldModel::build(table, layout).unwrap();
    assert!(
        !old.is_current_for(&w2, &f),
        "old aggregate must be detected as stale"
    );
    let new = derive_state_resources(&w2, &f).unwrap();
    // 資源は元のステートへ残らない
    assert_eq!(
        new.state(s(20)).unwrap().deposits[0],
        dep(ResourceType::Iron, 25.0, true, 1)
    );
    let b = new.state(s(21)).unwrap();
    assert!(
        b.deposits.contains(&dep(ResourceType::Iron, 35.0, true, 2)),
        "20 + 15 merged: {:?}",
        b.deposits
    );
}

#[test]
fn resource_definition_change_and_deleted_references_are_detected() {
    let (w, f) = (world(), res_file());
    let d = derive_state_resources(&w, &f).unwrap();
    let mut changed = f.clone();
    changed.provinces[2].deposits[0].base_output = 26.0;
    assert!(
        !d.is_current_for(&w, &changed),
        "definition change invalidates the old aggregate"
    );
    let mut reordered = f.clone();
    reordered.provinces.reverse();
    assert!(
        d.is_current_for(&w, &reordered),
        "reordering alone is not a change"
    );
    // 削除された参照: プロヴィンス表からプロヴィンスを取り除くと、資源定義の参照が宙に浮く → 検出
    let mut table = load_province_table("assets/data/world/preview/provinces.ron").unwrap();
    table.provinces.retain(|p| p.id != 18309);
    table.retired.push(18309);
    let mut layout = load_state_layout("assets/data/world/preview/states.ron").unwrap();
    layout
        .states
        .iter_mut()
        .find(|st| st.id == 20)
        .unwrap()
        .provinces
        .retain(|p| *p != 18309);
    let w2 = WorldModel::build(table, layout).unwrap();
    let e = derive_state_resources(&w2, &f).unwrap_err();
    assert!(e.contains(&ResourceIssue::UnknownProvince {
        province: ProvinceId(18309)
    }));
}

// ---------------------------------------------------------------- 経済への接続

/// 旧入力: ステート単位の鉱床をシナリオに直書き(M4-A/Bの形)。新入力: ステート単位の鉱床を
/// 空にし、プロヴィンス別の資源定義を接続。同じ総資源なら結果は完全に一致する。
#[test]
fn legacy_state_level_input_and_province_input_give_identical_production() {
    let (w, b, f) = (world(), buildings(), res_file());
    let derived = derive_state_resources(&w, &f).unwrap();
    let legacy = single_scenario();
    assert!(
        !legacy.states[0].resource_deposits.is_empty(),
        "legacy scenario carries state-level deposits"
    );
    let mut province_based = legacy.clone();
    strip_deposits(&mut province_based.states);

    let mut r_old = MapStateEconomyRegistry::from_scenario(&legacy, &w, &b).unwrap();
    let mut r_new = MapStateEconomyRegistry::from_scenario(&province_based, &w, &b).unwrap();
    r_new.attach_province_resources(&derived, &w, &f).unwrap();
    // 接続された鉱床は旧入力と同じ内容(順序・値)で、開発レベルのみ代表値(最大)
    let old20: Vec<_> = r_old
        .get(s(20))
        .unwrap()
        .resource_deposits
        .iter()
        .map(|d| (d.resource_type, d.base_output, d.discovered))
        .collect();
    let new20: Vec<_> = r_new
        .get(s(20))
        .unwrap()
        .resource_deposits
        .iter()
        .map(|d| (d.resource_type, d.base_output, d.discovered))
        .collect();
    assert_eq!(old20, new20);

    let (mut so, mut sn) = (
        stockpile_from_scenario(&legacy),
        stockpile_from_scenario(&legacy),
    );
    let before = stock_vec(&sn);
    let oo = r_old.run_month(
        legacy.country_mobilized_manpower,
        &mut so,
        &b,
        legacy.is_bankrupt,
    );
    let on = r_new.run_month(
        legacy.country_mobilized_manpower,
        &mut sn,
        &b,
        legacy.is_bankrupt,
    );
    assert_eq!(stock_vec(&so), stock_vec(&sn));
    assert_ne!(
        before,
        stock_vec(&sn),
        "production did change the stockpile (via the production step only)"
    );
    assert_eq!(
        (oo.science, oo.magic, oo.iron_produced),
        (on.science, on.magic, on.iron_produced)
    );
    for (a, c) in r_old.iter().zip(r_new.iter()) {
        assert_eq!(
            (
                a.employed_workforce,
                a.unemployed_workforce,
                a.logistics_ratio
            ),
            (
                c.employed_workforce,
                c.unemployed_workforce,
                c.logistics_ratio
            )
        );
        assert_eq!(a.building_operation_ratios, c.building_operation_ratios);
    }
}

#[test]
fn six_month_multi_country_run_is_identical_for_legacy_and_province_input() {
    let (w, b, f) = (world(), buildings(), res_file());
    let derived = derive_state_resources(&w, &f).unwrap();
    let legacy = multi_scenario();
    let mut pb = legacy.clone();
    strip_deposits(&mut pb.states);
    let (mut r_old, mut c_old) =
        MapStateEconomyRegistry::from_multi_scenario(&legacy, &w, &b).unwrap();
    let (mut r_new, mut c_new) = MapStateEconomyRegistry::from_multi_scenario(&pb, &w, &b).unwrap();
    r_new.attach_province_resources(&derived, &w, &f).unwrap();
    for month in 1..=legacy.months {
        for (co, cn) in c_old.iter_mut().zip(c_new.iter_mut()) {
            r_old.run_country_month(co, &b);
            r_new.run_country_month(cn, &b);
            assert_eq!(co, cn, "country differs at month {month}");
        }
        for (a, c) in r_old.iter().zip(r_new.iter()) {
            assert_eq!(
                (
                    a.population,
                    a.employed_workforce,
                    a.living_standard,
                    a.unrest
                ),
                (
                    c.population,
                    c.employed_workforce,
                    c.living_standard,
                    c.unrest
                ),
                "state differs at month {month}"
            );
        }
    }
}

#[test]
fn splitting_the_same_total_across_provinces_does_not_change_the_result() {
    let (w, b) = (world(), buildings());
    let mk = |provinces: Vec<ProvinceResourceEntry>| -> ProvinceResourcesFile {
        // MapState#20 の7プロヴィンス全てを設定(足りない分は資源なしを明示)
        let mut all = provinces;
        for id in [18109u32, 18308, 18309, 18310, 18311, 18499, 18500] {
            if !all.iter().any(|e| e.province == id) {
                all.push(entry(id, vec![]));
            }
        }
        ProvinceResourcesFile {
            format_version: 1,
            scope: ResourceScope::Partial,
            provinces: all,
        }
    };
    let one = mk(vec![
        entry(18309, vec![dep(ResourceType::Iron, 40.0, true, 1)]),
        entry(18311, vec![dep(ResourceType::Coal, 30.0, true, 1)]),
    ]);
    let four = mk(vec![
        entry(18309, vec![dep(ResourceType::Iron, 10.0, true, 1)]),
        entry(18310, vec![dep(ResourceType::Iron, 10.0, true, 1)]),
        entry(18499, vec![dep(ResourceType::Iron, 10.0, true, 1)]),
        entry(
            18500,
            vec![
                dep(ResourceType::Iron, 10.0, true, 1),
                dep(ResourceType::Coal, 30.0, true, 1),
            ],
        ),
    ]);
    let run = |f: &ProvinceResourcesFile| {
        let derived = derive_state_resources(&w, f).unwrap();
        let mut sc = single_scenario();
        strip_deposits(&mut sc.states);
        let mut reg = MapStateEconomyRegistry::from_scenario(&sc, &w, &b).unwrap();
        // 資源を接続するのは MapState#20 だけ(他は未設定でもMineを持たない)
        reg.attach_province_resources(&derived, &w, f).unwrap();
        let mut stock = stockpile_from_scenario(&sc);
        reg.run_month(sc.country_mobilized_manpower, &mut stock, &b, false);
        stock_vec(&stock)
    };
    assert_eq!(
        run(&one),
        run(&four),
        "same total in 1 or 4 provinces must give the same production"
    );
}

#[test]
fn production_equals_the_simple_hand_formula_and_is_not_amplified_by_province_count() {
    // 人口 200000・比率 0.5 → 潜在労働力 100000。Mine 1 は必要労働力 10000 → 雇用 10000
    // → 経済用労働力比率 0.1 → 稼働率 0.1(物流 1.0、入力なし)。Iron 産出 = base_output × level × 稼働率。
    let (w, b, f) = (world(), buildings(), res_file());
    let derived = derive_state_resources(&w, &f).unwrap();
    let mut sc = single_scenario();
    sc.states.truncate(1); // MapState#20 のみ
    strip_deposits(&mut sc.states);
    sc.states[0].population = 200_000;
    sc.states[0].workforce_ratio = 0.5;
    sc.states[0].buildings = [(BuildingType::Mine, 1)].into_iter().collect();
    sc.country_mobilized_manpower = 0;
    sc.stockpile.clear();
    let mut reg = MapStateEconomyRegistry::from_scenario(&sc, &w, &b).unwrap();
    reg.attach_province_resources(&derived, &w, &f).unwrap();
    let mut stock = stockpile_from_scenario(&sc);
    reg.run_month(0, &mut stock, &b, false);
    let op = reg.get(s(20)).unwrap().building_operation_ratios[&BuildingType::Mine];
    assert!((op - 0.1).abs() < 1e-6);
    // 7プロヴィンスに分かれていても Iron = 40 × 1 × 0.1 = 4.0(7倍にも3倍にもならない)、Coal = 30 × 0.1 = 3.0
    assert!(
        (stock.get(ResourceType::Iron) - 4.0).abs() < 1e-6,
        "{}",
        stock.get(ResourceType::Iron)
    );
    assert!((stock.get(ResourceType::Coal) - 3.0).abs() < 1e-6);
    // 未割当プロヴィンス(Iron 99)・他ステートの資源は混ざらない(他の資源は0のまま)
    assert_eq!(stock.get(ResourceType::Food), 0.0);
}

#[test]
fn deposits_are_never_credited_to_the_stockpile_directly() {
    // 鉱床はあるがMineが無いステート: 備蓄は(生産処理を通っても)全く変わらない。
    let (w, b, f) = (world(), buildings(), res_file());
    let derived = derive_state_resources(&w, &f).unwrap();
    let mut sc = single_scenario();
    sc.states.truncate(1);
    strip_deposits(&mut sc.states);
    sc.states[0].buildings = Default::default();
    sc.stockpile = [(ResourceType::Iron, 5.0)].into_iter().collect();
    let mut reg = MapStateEconomyRegistry::from_scenario(&sc, &w, &b).unwrap();
    reg.attach_province_resources(&derived, &w, &f).unwrap();
    assert!(!reg.get(s(20)).unwrap().resource_deposits.is_empty());
    let mut stock = stockpile_from_scenario(&sc);
    let before = stock_vec(&stock);
    assert_eq!(
        before,
        stock_vec(&stockpile_from_scenario(&sc)),
        "attach does not touch any stockpile"
    );
    reg.run_month(0, &mut stock, &b, false);
    assert_eq!(stock_vec(&stock), before);
}

#[test]
fn same_total_in_different_states_gives_the_same_production() {
    // MapState#20(7プロヴィンス)と #22(6プロヴィンス)に同じ経済入力・同じ総Iron 40(分割は異なる)
    let (w, b) = (world(), buildings());
    let input = |state: u32| MapStateEconomyInput {
        state,
        population: 200_000,
        workforce_ratio: 0.5,
        living_standard: 50.0,
        unrest: 10.0,
        base_logistics_capacity: 100.0,
        buildings: [(BuildingType::Mine, 1)].into_iter().collect(),
        resource_deposits: vec![],
    };
    let mut f = res_file();
    // 22 の6プロヴィンスを全て設定: 1か所に 40
    for (i, id) in [17266u32, 17267, 17268, 17484, 17485, 17693]
        .into_iter()
        .enumerate()
    {
        let d = if i == 0 {
            vec![dep(ResourceType::Iron, 40.0, true, 1)]
        } else {
            vec![]
        };
        f.provinces.push(entry(id, d));
    }
    f.provinces.retain(|e| e.province != 18311); // 20のCoalを外す(Ironだけ比較)
    f.provinces.push(entry(18311, vec![]));
    let derived = derive_state_resources(&w, &f).unwrap();
    let produce = |state: u32| {
        let sc = EconomyScenario {
            format_version: 1,
            description: "t".into(),
            country_mobilized_manpower: 0,
            is_bankrupt: false,
            stockpile: Default::default(),
            states: vec![input(state)],
        };
        let mut reg = MapStateEconomyRegistry::from_scenario(&sc, &w, &b).unwrap();
        reg.attach_province_resources(&derived, &w, &f).unwrap();
        let mut stock = stockpile_from_scenario(&sc);
        reg.run_month(0, &mut stock, &b, false);
        stock.get(ResourceType::Iron)
    };
    assert_eq!(produce(20), produce(22));
    assert!((produce(20) - 4.0).abs() < 1e-6);
}

#[test]
fn double_source_unset_mine_state_and_stale_input_are_rejected_without_mutation() {
    let (w, b, f) = (world(), buildings(), res_file());
    let derived = derive_state_resources(&w, &f).unwrap();
    // 二重入力: シナリオのステート単位の鉱床が残っている
    let legacy = single_scenario();
    let mut reg = MapStateEconomyRegistry::from_scenario(&legacy, &w, &b).unwrap();
    let before = reg.clone();
    let e = reg.attach_province_resources(&derived, &w, &f).unwrap_err();
    assert!(e.contains(&EconomyDataIssue::ResourceDoubleSource { state: s(20) }));
    assert_eq!(reg, before, "no partial mutation");
    // 未設定: Mineを持つ MapState#21 は未設定のプロヴィンスが残る
    let mut sc = single_scenario();
    strip_deposits(&mut sc.states);
    sc.states[1].buildings.insert(BuildingType::Mine, 1);
    let mut reg = MapStateEconomyRegistry::from_scenario(&sc, &w, &b).unwrap();
    let before = reg.clone();
    let e = reg.attach_province_resources(&derived, &w, &f).unwrap_err();
    assert!(e.iter().any(|i| matches!(i, EconomyDataIssue::ResourcesNotConfigured { state, unset_provinces } if *state == s(21) && unset_provinces.len() == 10)), "{e:?}");
    assert_eq!(reg, before);
    // 全く未設定のステート(22)にMine: 全プロヴィンスが未設定として報告される
    let mut sc = single_scenario();
    strip_deposits(&mut sc.states);
    sc.states[2].buildings.insert(BuildingType::Mine, 1);
    let mut reg = MapStateEconomyRegistry::from_scenario(&sc, &w, &b).unwrap();
    let e = reg.attach_province_resources(&derived, &w, &f).unwrap_err();
    assert!(e.iter().any(|i| matches!(i, EconomyDataIssue::ResourcesNotConfigured { state, unset_provinces } if *state == s(22) && unset_provinces.len() == 6)));
    // 古い集計
    let mut changed = f.clone();
    changed.provinces[2].deposits[0].base_output = 1.0;
    let mut sc = single_scenario();
    strip_deposits(&mut sc.states);
    let mut reg = MapStateEconomyRegistry::from_scenario(&sc, &w, &b).unwrap();
    assert_eq!(
        reg.attach_province_resources(&derived, &w, &changed)
            .unwrap_err(),
        vec![EconomyDataIssue::StaleResources]
    );
}
