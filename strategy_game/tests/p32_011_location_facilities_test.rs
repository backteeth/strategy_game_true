//! P32-011: 位置施設(港・要塞・鉄道駅)のプロヴィンス入力層のテスト(GUIなし)。
//! 小さな合成地図(期待値を手で確認できるもの)と、実地図+検証用ワールド設定の両方を使う。
//! 施設の効果は未接続であり、ここでは入力・所在地・所属導出・地理の事実・既存経済への非干渉だけを確認する。

use strategy_game::building::data::BuildingType;
use strategy_game::hexmap::data::{HexMap, load_from_path, parse_hex_map};
use strategy_game::world::*;

const FILE: &str = "assets/data/world/preview/province_facilities.ron";

type Cell<'a> = (i32, i32, &'a str);

fn map_from(cells: &[Cell]) -> HexMap {
    let c: Vec<String> = cells
        .iter()
        .map(|(q, r, t)| {
            let (cl, v) = if *t == "sea" || *t == "lake" {
                ("unclassified", "unclassified")
            } else {
                ("C", "grassland")
            };
            format!(r#"{{"q":{q},"r":{r},"terrain":"{t}","climate":"{cl}","vegetation":"{v}"}}"#)
        })
        .collect();
    let json = format!(
        r#"{{"schemaVersion":9,"kind":"hex-atlas-game-map",
        "grid":{{"orientation":"pointy","size":2.5,"origin":{{"x":2.0,"y":2.5}},"imageWidth":200,"imageHeight":100,
        "extent":{{"minQ":-3,"maxQ":3,"minR":0,"maxR":3}}}},"cells":[{}],"rivers":[]}}"#,
        c.join(",")
    );
    parse_hex_map(json.as_bytes()).expect("synthetic map must parse")
}

/// 合成地図: 陸 (-3,1),(-2,1),(-1,1),(0,1),(0,2)。湖 (1,1)。海 (-1,0),(-2,2)。
/// pointy-top 軸座標の6近傍は (q+1,r),(q+1,r-1),(q,r-1),(q-1,r),(q-1,r+1),(q,r+1)。
fn fixture_map() -> HexMap {
    map_from(&[
        (-3, 1, "plains"),
        (-2, 1, "plains"),
        (-1, 1, "plains"),
        (0, 1, "plains"),
        (1, 1, "lake"),
        (-1, 0, "sea"),
        (-2, 2, "sea"),
        (0, 2, "plains"),
    ])
}

fn fixture_world(map: &HexMap, states: &[(u32, Vec<(i32, i32)>)]) -> WorldModel {
    let table = generate_initial_province_table(map, Coverage::World);
    let id_at = |c: &(i32, i32)| table.provinces.iter().find(|p| (p.q, p.r) == *c).unwrap().id;
    let layout = StateLayoutFile {
        format_version: WORLD_FORMAT_VERSION,
        states: states
            .iter()
            .map(|(id, cells)| StateEntry {
                id: *id,
                name_key: format!("test.{id}"),
                provinces: cells.iter().map(id_at).collect(),
                representative: None,
                allow_disconnected: true,
            })
            .collect(),
    };
    WorldModel::build(table, layout).unwrap()
}

fn pid(w: &WorldModel, q: i32, r: i32) -> u32 {
    w.province_at(q, r).unwrap().id.0
}

fn file(
    facilities: Vec<(u32, LocationFacilityKind, u32)>,
    empty: Vec<u32>,
    scope: FacilityScope,
) -> LocationFacilitiesFile {
    LocationFacilitiesFile {
        format_version: LOCATION_FACILITIES_FORMAT_VERSION,
        scope,
        facilities: facilities
            .into_iter()
            .map(|(id, kind, province)| LocationFacilityDef { id, kind, province })
            .collect(),
        confirmed_empty: empty,
    }
}

fn preview() -> (HexMap, WorldPreviewData) {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let w = load_world_preview(
        "assets/data/world/preview/provinces.ron",
        "assets/data/world/preview/states.ron",
        &map,
    )
    .unwrap();
    (map, w)
}

use LocationFacilityKind::{Fortress, Port, RailwayStation};

// ---------------------------------------------------------------- 入力

#[test]
fn preview_file_loads_and_is_not_modified() {
    let before = std::fs::read(FILE).unwrap();
    let (_, w) = preview();
    let f = load_location_facilities(FILE).unwrap();
    let reg = LocationFacilityRegistry::build(&f, &w.model).unwrap();
    assert_eq!(reg.len(), 6);
    assert_eq!(reg.scope(), FacilityScope::Partial);
    assert_eq!(std::fs::read(FILE).unwrap(), before, "input file must not change");
}

#[test]
fn unsupported_version_is_rejected_clearly() {
    let text = std::fs::read_to_string(FILE)
        .unwrap()
        .replace("format_version: 1", "format_version: 2");
    let e = parse_location_facilities(&text).unwrap_err();
    assert!(e.contains("unsupported format_version 2"), "{e}");
    let (_, w) = preview();
    let mut f = load_location_facilities(FILE).unwrap();
    f.format_version = 9;
    let issues = LocationFacilityRegistry::build(&f, &w.model).unwrap_err();
    assert!(issues.contains(&FacilityIssue::UnsupportedVersion { found: 9 }));
}

#[test]
fn invalid_kind_and_malformed_values_are_rejected() {
    let src = std::fs::read_to_string(FILE).unwrap();
    let bad_kind = src.replace("kind: Port, province: 18309", "kind: Harbor, province: 18309");
    assert!(parse_location_facilities(&bad_kind).is_err());
    let neg = src.replace("province: 18309", "province: -5");
    assert!(parse_location_facilities(&neg).is_err());
    let missing = src.replace(
        "(id: 101, kind: Port, province: 18309)",
        "(id: 101, kind: Port)",
    );
    assert!(parse_location_facilities(&missing).is_err());
}

#[test]
fn duplicate_ids_unknown_provinces_and_contradictions_are_rejected() {
    let (_, w) = preview();
    let base = load_location_facilities(FILE).unwrap();
    let mut f = base.clone();
    f.facilities.push(LocationFacilityDef { id: 101, kind: Fortress, province: 18310 });
    let e = LocationFacilityRegistry::build(&f, &w.model).unwrap_err();
    assert!(e.contains(&FacilityIssue::DuplicateFacilityId { id: LocationFacilityId(101) }));

    let mut f = base.clone();
    f.facilities.push(LocationFacilityDef { id: 999, kind: Port, province: 424242 });
    f.confirmed_empty.push(555555);
    let e = LocationFacilityRegistry::build(&f, &w.model).unwrap_err();
    assert!(e.contains(&FacilityIssue::UnknownProvince {
        facility: Some(LocationFacilityId(999)),
        province: ProvinceId(424242)
    }));
    assert!(e.contains(&FacilityIssue::UnknownProvince {
        facility: None,
        province: ProvinceId(555555)
    }));

    let mut f = base.clone();
    f.confirmed_empty.push(18499);
    f.confirmed_empty.push(18309);
    let e = LocationFacilityRegistry::build(&f, &w.model).unwrap_err();
    assert!(e.contains(&FacilityIssue::DuplicateConfirmedEmpty { province: ProvinceId(18499) }));
    assert!(e.contains(&FacilityIssue::EmptyButHasFacility {
        province: ProvinceId(18309),
        facility: LocationFacilityId(101)
    }));
}

// ---------------------------------------------------------------- 参照

#[test]
fn lookups_by_id_province_and_kind_match_the_known_layout() {
    let (_, w) = preview();
    let reg =
        LocationFacilityRegistry::build(&load_location_facilities(FILE).unwrap(), &w.model).unwrap();
    assert_eq!(reg.facility(LocationFacilityId(201)).unwrap().province, 18310);
    assert!(reg.facility(LocationFacilityId(1)).is_none());
    let ids = |v: Vec<&LocationFacilityDef>| v.iter().map(|d| d.id).collect::<Vec<_>>();
    assert_eq!(ids(reg.facilities_in_province(ProvinceId(18309))), vec![101]);
    assert!(reg.facilities_in_province(ProvinceId(18499)).is_empty());
    assert_eq!(ids(reg.facilities_of_kind(Port)), vec![101, 102, 103]);
    assert_eq!(ids(reg.facilities_of_kind(Fortress)), vec![201, 202]);
    assert_eq!(ids(reg.facilities_of_kind(RailwayStation)), vec![301]);
    // 手で確認した所属: 20={101,201}, 21={202,301}, 22={102}, 未割当={103}
    assert_eq!(ids(reg.facilities_in_state(&w.model, MapStateId(20)).unwrap()), vec![101, 201]);
    assert_eq!(ids(reg.facilities_in_state(&w.model, MapStateId(21)).unwrap()), vec![202, 301]);
    assert_eq!(ids(reg.facilities_in_state(&w.model, MapStateId(22)).unwrap()), vec![102]);
    assert_eq!(ids(reg.unassigned_facilities(&w.model)), vec![103]);
    assert!(reg.facilities_in_state(&w.model, MapStateId(99)).is_none());
    // 重複計上なし: 全ステートの施設 + 未割当 = 全施設(各1回)
    let total: usize = w
        .model
        .states()
        .iter()
        .map(|s| reg.facilities_in_state(&w.model, s.id).unwrap().len())
        .sum::<usize>()
        + reg.unassigned_facilities(&w.model).len();
    assert_eq!(total, reg.len());
    assert_eq!(reg.iter().count(), reg.len());
}

#[test]
fn several_facilities_in_one_province_are_kept_and_input_order_does_not_matter() {
    let map = fixture_map();
    let w = fixture_world(&map, &[(1, vec![(-2, 1), (-1, 1)])]);
    let p = pid(&w, -2, 1);
    let q = pid(&w, -1, 1);
    let a = file(
        vec![(5, Port, p), (3, Fortress, p), (4, Fortress, p), (9, RailwayStation, q)],
        vec![],
        FacilityScope::Partial,
    );
    let mut b = a.clone();
    b.facilities.reverse();
    let ra = LocationFacilityRegistry::build(&a, &w).unwrap();
    let rb = LocationFacilityRegistry::build(&b, &w).unwrap();
    assert_eq!(ra, rb, "order of the input must not change the result");
    let ids: Vec<u32> = ra.facilities_in_province(ProvinceId(p)).iter().map(|d| d.id).collect();
    assert_eq!(ids, vec![3, 4, 5], "same province: all kept (no slot limit decided), ID ascending");
    let s: Vec<u32> = ra
        .facilities_in_state(&w, MapStateId(1))
        .unwrap()
        .iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(s, vec![3, 4, 5, 9]);
    assert!(ra.facility(LocationFacilityId(9)).is_some() && ra.facility(LocationFacilityId(0)).is_none());
}

// ---------------------------------------------------------------- 所属

#[test]
fn membership_change_is_followed_with_no_stale_index() {
    let map = fixture_map();
    let w1 = fixture_world(&map, &[(1, vec![(-2, 1), (-1, 1)]), (2, vec![(0, 1)])]);
    let p = pid(&w1, -1, 1);
    let reg = LocationFacilityRegistry::build(
        &file(vec![(1, Fortress, p)], vec![], FacilityScope::Partial),
        &w1,
    )
    .unwrap();
    assert_eq!(reg.state_of_facility(&w1, LocationFacilityId(1)), Some(MapStateId(1)));
    // 同じ施設・同じ所在地のまま、プロヴィンスが別ステートへ移った世界
    let w2 = fixture_world(&map, &[(1, vec![(-2, 1)]), (2, vec![(-1, 1), (0, 1)])]);
    assert!(reg.facilities_in_state(&w2, MapStateId(1)).unwrap().is_empty());
    assert_eq!(reg.facilities_in_state(&w2, MapStateId(2)).unwrap().len(), 1);
    assert_eq!(reg.state_of_facility(&w2, LocationFacilityId(1)), Some(MapStateId(2)));
    // 未割当になった世界: 架空のステートへ入れず、未割当として保持
    let w3 = fixture_world(&map, &[(1, vec![(-2, 1)])]);
    assert!(reg.facilities_in_state(&w3, MapStateId(1)).unwrap().is_empty());
    assert_eq!(reg.unassigned_facilities(&w3).len(), 1);
    assert_eq!(reg.state_of_facility(&w3, LocationFacilityId(1)), None);
    assert_eq!(reg.facility(LocationFacilityId(1)).unwrap().province, p);
}

// ---------------------------------------------------------------- 完全性

#[test]
fn unset_and_confirmed_empty_are_distinguished_and_partial_is_not_world_complete() {
    let (_, w) = preview();
    let reg =
        LocationFacilityRegistry::build(&load_location_facilities(FILE).unwrap(), &w.model).unwrap();
    assert_eq!(reg.province_setup(ProvinceId(18309)), ProvinceFacilitySetup::HasFacilities);
    assert_eq!(reg.province_setup(ProvinceId(18499)), ProvinceFacilitySetup::ConfirmedEmpty);
    assert_eq!(reg.province_setup(ProvinceId(18501)), ProvinceFacilitySetup::Unset);
    assert!(reg.facilities_in_province(ProvinceId(18499)).is_empty());
    assert!(reg.facilities_in_province(ProvinceId(18501)).is_empty());
    // MapState#20: 7件中 18109 / 18308 は未設定 → 一部未設定
    assert_eq!(reg.state_setup(&w.model, MapStateId(20)), Some(StateFacilitySetup::Incomplete));
    assert_eq!(reg.scope(), FacilityScope::Partial);
    // Worldスコープを宣言して未設定が残れば入力エラー(小地域の設定を全世界の完成と扱わない)
    let mut f = load_location_facilities(FILE).unwrap();
    f.scope = FacilityScope::World;
    let e = LocationFacilityRegistry::build(&f, &w.model).unwrap_err();
    assert!(matches!(&e[0], FacilityIssue::WorldScopeIncomplete { missing } if missing.len() == 20));
}

#[test]
fn world_scope_is_complete_only_when_every_province_is_set() {
    let map = fixture_map();
    let w = fixture_world(&map, &[(1, vec![(-3, 1), (-2, 1)])]);
    let all: Vec<u32> = w.provinces().iter().map(|p| p.id.0).collect();
    let f = file(vec![(1, Port, all[0])], all[1..].to_vec(), FacilityScope::World);
    let reg = LocationFacilityRegistry::build(&f, &w).unwrap();
    assert_eq!(reg.scope(), FacilityScope::World);
    assert_eq!(reg.state_setup(&w, MapStateId(1)), Some(StateFacilitySetup::Complete));
    // 全プロヴィンスが「施設なし」でも有効(未設定とは別)
    let none = file(vec![], all.clone(), FacilityScope::World);
    let r = LocationFacilityRegistry::build(&none, &w).unwrap();
    assert!(r.is_empty());
    assert_eq!(r.state_setup(&w, MapStateId(1)), Some(StateFacilitySetup::Complete));
    // 何も設定しない = 未設定(施設ゼロとは別)
    let unset = file(vec![], vec![], FacilityScope::Partial);
    assert_eq!(
        LocationFacilityRegistry::build(&unset, &w).unwrap().state_setup(&w, MapStateId(1)),
        Some(StateFacilitySetup::Unconfigured)
    );
}

// ---------------------------------------------------------------- 地理

#[test]
fn water_adjacency_distinguishes_sea_lake_none_and_unmapped_with_negative_q() {
    let map = fixture_map();
    // (0,1): 近傍 (1,1)湖 のみ水域 → 湖1・海0
    let a = water_adjacency(&map, HexCoord::new(0, 1));
    assert_eq!((a.sea, a.lake), (0, 1));
    assert!(a.touches_lake() && !a.touches_sea() && a.touches_water());
    // (-1,1): 近傍 (-1,0)海,(-2,2)海 → 海2・湖0
    let b = water_adjacency(&map, HexCoord::new(-1, 1));
    assert_eq!((b.sea, b.lake), (2, 0));
    assert!(b.touches_sea() && !b.touches_lake());
    // (-3,1): 負のq。水域なし。(-4,1),(-4,2) は地図の範囲外 = 水域とは判断しない
    let c = water_adjacency(&map, HexCoord::new(-3, 1));
    assert_eq!((c.sea, c.lake), (0, 0));
    assert!(!c.touches_water());
    assert!(c.unmapped >= 2, "{c:?}");
    // 実地図の内陸セル(Province#18311)にも水域なしと診断できる
    let (real, w) = preview();
    let inland = w.model.province(ProvinceId(18311)).unwrap();
    assert!(!water_adjacency(&real, inland.cell).touches_water());
}

#[test]
fn preview_ports_touch_the_sea_and_adjacency_is_not_a_build_permission() {
    let (map, w) = preview();
    let reg =
        LocationFacilityRegistry::build(&load_location_facilities(FILE).unwrap(), &w.model).unwrap();
    for d in reg.facilities_of_kind(Port) {
        let cell = w.model.province(ProvinceId(d.province)).unwrap().cell;
        let a = water_adjacency(&map, cell);
        assert!(a.touches_sea(), "{d:?} {a:?}");
        assert_eq!(a.lake, 0);
    }
    // 水域に接さない陸セルの港も、データ層は拒否しない(建設可否は未決のルール)。診断だけが事実を返す。
    let inland = ProvinceId(18310);
    assert!(!water_adjacency(&map, w.model.province(inland).unwrap().cell).touches_water());
    let f = file(vec![(1, Port, inland.0)], vec![], FacilityScope::Partial);
    let reg = LocationFacilityRegistry::build(&f, &w.model).unwrap();
    assert_eq!(reg.facilities_of_kind(Port).len(), 1);
}

#[test]
fn geography_file_is_not_modified() {
    let before = std::fs::read("assets/data/hex_atlas_map.json").unwrap();
    let _ = preview();
    assert_eq!(std::fs::read("assets/data/hex_atlas_map.json").unwrap(), before);
}

// ---------------------------------------------------------------- 既存経済への非干渉

#[test]
fn existing_building_types_do_not_include_location_facilities() {
    assert_eq!(BuildingType::ALL.len(), 10);
    for b in BuildingType::ALL {
        let n = format!("{b:?}");
        assert!(!["Port", "Fortress", "RailwayStation", "Station"].contains(&n.as_str()), "{n}");
    }
    let defs = std::fs::read_to_string("assets/data/buildings.ron").unwrap();
    for w in ["Port", "Fortress", "Station"] {
        assert!(!defs.contains(w), "buildings.ron must not define {w}");
    }
}

#[test]
fn loading_facilities_does_not_change_the_economy_scenario_results() {
    use strategy_game::building::data::{BuildingDefinition, BuildingRegistry};
    use strategy_game::economy::map_state::{
        MapStateEconomyRegistry, SimDate, parse_multi_month_scenario,
    };
    use strategy_game::economy::province_resources::{
        ProvinceResourceEntry, derive_state_resources, load_province_resources,
    };
    let (_, w) = preview();
    let defs: Vec<BuildingDefinition> =
        ron::from_str(&std::fs::read_to_string("assets/data/buildings.ron").unwrap()).unwrap();
    let b = BuildingRegistry {
        definitions: defs.into_iter().map(|d| (d.building_type, d)).collect(),
    };
    let sc = parse_multi_month_scenario(
        &std::fs::read_to_string("assets/data/world/preview/economy_scenario_m4c.ron").unwrap(),
    )
    .unwrap();
    let mut res = load_province_resources("assets/data/world/preview/province_resources.ron").unwrap();
    for p in w.model.provinces() {
        if !res.provinces.iter().any(|e| e.province == p.id.0) {
            res.provinces.push(ProvinceResourceEntry { province: p.id.0, deposits: vec![] });
        }
    }
    let run = |with_facilities: bool| {
        if with_facilities {
            let reg = LocationFacilityRegistry::build(
                &load_location_facilities(FILE).unwrap(),
                &w.model,
            )
            .unwrap();
            assert_eq!(reg.len(), 6);
        }
        let (mut reg, mut cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, &w.model, &b).unwrap();
        let d = derive_state_resources(&w.model, &res).unwrap();
        reg.attach_province_resources(&d, &w.model, &res).unwrap();
        let days = reg.run_days(&mut cs, SimDate { year: 1800, month: 1, day: 1 }, 120, &b);
        let orders: Vec<_> = days
            .iter()
            .map(|(d, r)| (*d, r.ordered.clone(), r.completed.clone()))
            .collect();
        let st: Vec<_> = reg
            .iter()
            .map(|s| (s.state, s.employed_workforce, s.buildings.len(), s.logistics_ratio))
            .collect();
        let tr: Vec<_> = cs.iter().map(|c| (c.treasury, c.tax_rate)).collect();
        (orders, st, tr)
    };
    let (o1, s1, t1) = run(false);
    let (o2, s2, t2) = run(true);
    assert_eq!(o1, o2);
    assert_eq!(s1, s2);
    assert_eq!(t1, t2);
    // 手計算の既知値(P32-010): 国家1は1/02にMapState#20へMine(費用500)を注文する。施設の追加で変わらない
    assert!(o2[0].1.iter().any(|x| x.0 == 1
        && x.1 == MapStateId(20)
        && x.2 == BuildingType::Mine
        && x.3 == 500.0));
}
