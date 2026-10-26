//! P32-017: 静的入力の項目別ダイジェストと、保存時・復元先の不一致の判定表のテスト。
//! 生成するファイルはすべてOSの一時ディレクトリ(テストごとに一意)に置き、`saves/` へは書かない。

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde_json::Value;
use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::economy::map_economy_file::{
    FileError, SaveLocation, decode, encode, load, read_file, save,
};
use strategy_game::economy::map_state::snapshot::{MapEconomySimulation, MapEconomySnapshot, RestoreIssue};
use strategy_game::economy::map_state::static_identity::{
    Class, IDENTITY_SCHEME, ItemKey, RULES, StaticIdentity, StaticInputs, Structural, judge, rule_of,
};
use strategy_game::economy::map_state::{MapStateEconomyRegistry, SimDate, parse_multi_month_scenario};
use strategy_game::economy::province_resources::{derive_state_resources, parse_province_resources};
use strategy_game::hexmap::data::{HexMap, parse_hex_map};
use strategy_game::world::{
    LocationFacilityRegistry, WorldModel, parse_location_facilities, parse_province_table, parse_state_layout,
};

const START: SimDate = SimDate { year: 1800, month: 1, day: 1 };
const P: &str = "assets/data/world/preview/";
const MAP_PATH: &str = "assets/data/hex_atlas_map.json";

// ---------------------------------------------------------------- 入力一式(テキストから作る)

/// 入力のテキスト一式。各テストは必要な部分だけ書き換えて、別の静的入力を作る。
#[derive(Clone)]
struct Texts {
    provinces: String,
    states: String,
    scenario: String,
    resources: String,
    buildings: String,
    facilities: String,
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap()
}

fn texts() -> Texts {
    Texts {
        provinces: read(&format!("{P}provinces.ron")),
        states: read(&format!("{P}states.ron")),
        scenario: read(&format!("{P}economy_scenario_m4d.ron")),
        resources: read(&format!("{P}province_resources_m4d.ron")),
        buildings: read("assets/data/buildings.ron"),
        facilities: read(&format!("{P}province_facilities.ron")),
    }
}

fn replace_first(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "test setup: {from:?} not found");
    text.replacen(from, to, 1)
}

fn base_json() -> &'static Value {
    static V: OnceLock<Value> = OnceLock::new();
    V.get_or_init(|| serde_json::from_slice(&fs::read(MAP_PATH).unwrap()).unwrap())
}

fn map_of(v: &Value) -> HexMap {
    parse_hex_map(&serde_json::to_vec(v).unwrap()).unwrap()
}

fn base_map() -> &'static Arc<HexMap> {
    static M: OnceLock<Arc<HexMap>> = OnceLock::new();
    M.get_or_init(|| Arc::new(map_of(base_json())))
}

/// 地図JSONを書き換えた新しい地図(元のJSONは変えない)。
fn mutated_map(f: impl FnOnce(&mut Value)) -> HexMap {
    let mut v = base_json().clone();
    f(&mut v);
    map_of(&v)
}

fn cell_mut(v: &mut Value, q: i64, r: i64) -> &mut Value {
    v["cells"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["q"] == q && c["r"] == r)
        .expect("cell exists")
}

/// 検証地域のプロヴィンス(陸セル)の座標。
fn province_cell(w: &WorldModel) -> (i64, i64) {
    let c = w.provinces()[0].cell;
    (c.q as i64, c.r as i64)
}

fn world_of(t: &Texts) -> WorldModel {
    WorldModel::build(parse_province_table(&t.provinces).unwrap(), parse_state_layout(&t.states).unwrap()).unwrap()
}

fn buildings_of(t: &Texts) -> BuildingRegistry {
    let defs: Vec<BuildingDefinition> = ron::from_str(&t.buildings).unwrap();
    BuildingRegistry { definitions: defs.into_iter().map(|d| (d.building_type, d)).collect() }
}

fn sim_of(t: &Texts, w: &WorldModel, b: &BuildingRegistry) -> MapEconomySimulation {
    let sc = parse_multi_month_scenario(&t.scenario).unwrap();
    let (mut reg, cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, w, b).unwrap();
    let f = parse_province_resources(&t.resources).unwrap();
    let d = derive_state_resources(w, &f).unwrap();
    reg.attach_province_resources(&d, w, &f).unwrap();
    MapEconomySimulation::new(START, reg, cs)
}

fn facilities_of(t: &Texts, w: &WorldModel) -> LocationFacilityRegistry {
    LocationFacilityRegistry::build(&parse_location_facilities(&t.facilities).unwrap(), w).unwrap()
}

/// 1組の静的入力(所有者)。`inputs()` で借用する。
struct Env {
    map: Arc<HexMap>,
    world: WorldModel,
    buildings: BuildingRegistry,
    facilities: Option<LocationFacilityRegistry>,
    texts: Texts,
}

impl Env {
    fn base() -> Env {
        Env::with(base_map().clone(), texts(), true)
    }
    fn with(map: Arc<HexMap>, texts: Texts, facilities: bool) -> Env {
        let world = world_of(&texts);
        let buildings = buildings_of(&texts);
        let facilities = facilities.then(|| facilities_of(&texts, &world));
        Env { map, world, buildings, facilities, texts }
    }
    fn edited(&self, f: impl FnOnce(&mut Texts)) -> Env {
        let mut t = self.texts.clone();
        f(&mut t);
        Env::with(self.map.clone(), t, self.facilities.is_some())
    }
    fn with_map(&self, map: HexMap) -> Env {
        Env::with(Arc::new(map), self.texts.clone(), self.facilities.is_some())
    }
    fn inputs(&self) -> StaticInputs<'_> {
        StaticInputs { map: &self.map, world: &self.world, buildings: &self.buildings, facilities: self.facilities.as_ref() }
    }
    fn sim(&self) -> MapEconomySimulation {
        sim_of(&self.texts, &self.world, &self.buildings)
    }
    fn advanced(&self, days: usize) -> MapEconomySimulation {
        let mut s = self.sim();
        for _ in 0..days {
            s.advance_day(&self.buildings);
        }
        s
    }
    fn identity(&self) -> StaticIdentity {
        StaticIdentity::compute(&self.sim(), &self.inputs()).unwrap()
    }
}

fn base_env() -> &'static Env {
    static E: OnceLock<Env> = OnceLock::new();
    E.get_or_init(Env::base)
}

fn differing(a: &StaticIdentity, b: &StaticIdentity) -> Vec<ItemKey> {
    ItemKey::ALL.into_iter().filter(|k| a.digest(*k) != b.digest(*k)).collect()
}

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!(
            "p32_017_{tag}_{}_{nanos}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn loc(&self) -> SaveLocation {
        SaveLocation::new(&self.0)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join("map_economy").join(format!("{name}.mapeco.ron"))
    }
    fn write_raw(&self, name: &str, content: &[u8]) {
        fs::create_dir_all(self.0.join("map_economy")).unwrap();
        fs::write(self.file(name), content).unwrap();
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------- 判定表

#[test]
fn the_judgement_table_covers_every_item_once_and_the_warn_set_is_the_enumerated_list() {
    assert_eq!(RULES.len(), ItemKey::ALL.len());
    for (rule, key) in RULES.iter().zip(ItemKey::ALL) {
        assert_eq!(rule.key, key, "table order equals ItemKey::ALL");
        assert_eq!(ItemKey::parse(key.name()), Some(key));
        assert!(!rule.used_by.is_empty() && !rule.normalization.is_empty());
        assert!(!rule.impact.is_empty() && !rule.rationale.is_empty());
    }
    let mut names: Vec<_> = ItemKey::ALL.iter().map(|k| k.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), ItemKey::ALL.len(), "item names are unique");
    // 警告で許す項目の限定列挙(増減は意図した変更でなければならない)。
    let warn: Vec<ItemKey> = RULES.iter().filter(|r| r.class == Class::Warn).map(|r| r.key).collect();
    assert_eq!(
        warn,
        vec![
            ItemKey::LandTerrain,
            ItemKey::Climate,
            ItemKey::Vegetation,
            ItemKey::Rivers,
            ItemKey::StatePresentation,
            ItemKey::BuildingPresentation,
            ItemKey::LocationFacilities,
        ]
    );
    // 判定表にない項目名は存在しない。
    assert_eq!(ItemKey::parse("future_item"), None);
    // 拒否が既定: 陸水・ID・所属・資源・国家・軍事・建物計算・規則は必ず拒否。
    for k in [
        ItemKey::Grid, ItemKey::CellCoordinates, ItemKey::LandSet, ItemKey::WaterKinds, ItemKey::ProvinceTable,
        ItemKey::StateMembership, ItemKey::StateConstraints, ItemKey::ScenarioStructure, ItemKey::StateEconomyStatic,
        ItemKey::ResourceDeposits, ItemKey::ResourceSetup, ItemKey::MilitaryInputs, ItemKey::BuildingComputation,
        ItemKey::EconomyRules,
    ] {
        assert_eq!(rule_of(k).class, Class::Reject, "{k}");
    }
}

// ---------------------------------------------------------------- 正規化

#[test]
fn identity_has_all_items_as_64_hex_digits_and_is_deterministic() {
    let e = base_env();
    let a = e.identity();
    let b = e.identity();
    assert_eq!(a, b);
    for k in ItemKey::ALL {
        let d = a.digest(k).unwrap();
        assert_eq!(d.len(), 64, "{k}");
        assert!(d.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)), "{k}");
    }
    // 項目ごとに領域が分かれている(同じ内容でも項目が違えば別の値)。
    let mut all: Vec<&str> = ItemKey::ALL.iter().map(|k| a.digest(*k).unwrap()).collect();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), ItemKey::ALL.len());
    let saved = a.to_saved();
    assert_eq!(saved.scheme, IDENTITY_SCHEME);
    assert_eq!(saved.items.len(), ItemKey::ALL.len());
}

#[test]
fn dynamic_state_does_not_change_the_identity() {
    let e = base_env();
    let fresh = StaticIdentity::compute(&e.sim(), &e.inputs()).unwrap();
    let later = StaticIdentity::compute(&e.advanced(130), &e.inputs()).unwrap();
    assert_eq!(fresh, later, "population, treasury, queues, histories are not static inputs");
}

#[test]
fn formatting_and_meaningless_ordering_do_not_change_any_item() {
    let e = base_env();
    let before = e.identity();
    // 地図: セル・河川の並び順を逆にし、整形(インデント)も変える。
    let mut v = base_json().clone();
    v["cells"].as_array_mut().unwrap().reverse();
    v["rivers"].as_array_mut().unwrap().reverse();
    let pretty = serde_json::to_vec_pretty(&v).unwrap();
    let map = parse_hex_map(&pretty).unwrap();
    // プロヴィンス表・ステート所属・建物定義・施設の並びを逆にする。
    let mut table = parse_province_table(&e.texts.provinces).unwrap();
    table.provinces.reverse();
    table.retired.reverse();
    let mut layout = parse_state_layout(&e.texts.states).unwrap();
    layout.states.reverse();
    for s in &mut layout.states {
        s.provinces.reverse();
    }
    let world = WorldModel::build(table, layout).unwrap();
    let mut defs: Vec<BuildingDefinition> = ron::from_str(&e.texts.buildings).unwrap();
    defs.reverse();
    let buildings = BuildingRegistry { definitions: defs.into_iter().map(|d| (d.building_type, d)).collect() };
    let mut ff = parse_location_facilities(&e.texts.facilities).unwrap();
    ff.facilities.reverse();
    ff.confirmed_empty.reverse();
    let facilities = LocationFacilityRegistry::build(&ff, &world).unwrap();
    let sim = sim_of(&e.texts, &world, &buildings);
    let inputs = StaticInputs { map: &map, world: &world, buildings: &buildings, facilities: Some(&facilities) };
    let after = StaticIdentity::compute(&sim, &inputs).unwrap();
    assert_eq!(differing(&before, &after), vec![], "reordering / re-formatting changes nothing");
}

#[test]
fn identity_does_not_depend_on_where_the_input_files_are() {
    // 同じ内容を別の場所(一時ディレクトリ)に置いて読んでも、識別は変わらない。
    let t = Tmp::new("location");
    let moved = t.0.join("moved_map.json");
    fs::copy(MAP_PATH, &moved).unwrap();
    let map = strategy_game::hexmap::data::load_from_path(moved.to_str().unwrap()).unwrap();
    let e = base_env();
    let env2 = e.with_map(map);
    assert_eq!(differing(&e.identity(), &env2.identity()), vec![]);
}

#[test]
fn content_changes_are_detected_as_the_corresponding_items() {
    let e = base_env();
    let base = e.identity();
    let (q, r) = province_cell(&e.world);
    let cur = |v: &Value, f: &str| v["cells"].as_array().unwrap().iter().find(|c| c["q"] == q && c["r"] == r).unwrap()[f]
        .as_str()
        .unwrap()
        .to_string();
    let other_terrain = if cur(base_json(), "terrain") == "hills" { "plains" } else { "hills" };
    let other_veg = if cur(base_json(), "vegetation") == "forest" { "none" } else { "forest" };
    let sea_cell = base_json()["cells"].as_array().unwrap().iter().find(|c| c["terrain"] == "sea").unwrap().clone();
    let (sq, sr) = (sea_cell["q"].as_i64().unwrap(), sea_cell["r"].as_i64().unwrap());

    // (名前, 変えた入力, 期待する差分の項目の集合)
    let map_cases: Vec<(&str, HexMap, Vec<ItemKey>)> = vec![
        ("grid origin", mutated_map(|v| v["grid"]["origin"]["x"] = Value::from(2.5)), vec![ItemKey::Grid]),
        (
            "remove a sea cell",
            mutated_map(|v| {
                v["cells"].as_array_mut().unwrap().retain(|c| !(c["q"] == sq && c["r"] == sr));
            }),
            vec![ItemKey::CellCoordinates, ItemKey::WaterKinds, ItemKey::Climate, ItemKey::Vegetation],
        ),
        (
            "land becomes sea",
            mutated_map(|v| cell_mut(v, q, r)["terrain"] = Value::from("sea")),
            vec![ItemKey::LandSet, ItemKey::WaterKinds, ItemKey::LandTerrain],
        ),
        (
            "sea becomes lake",
            mutated_map(|v| cell_mut(v, sq, sr)["terrain"] = Value::from("lake")),
            vec![ItemKey::WaterKinds],
        ),
        (
            "land terrain",
            mutated_map(|v| cell_mut(v, q, r)["terrain"] = Value::from(other_terrain)),
            vec![ItemKey::LandTerrain],
        ),
        (
            "climate",
            mutated_map(|v| cell_mut(v, q, r)["climate"] = Value::from("A")),
            vec![ItemKey::Climate],
        ),
        (
            "vegetation",
            mutated_map(|v| cell_mut(v, q, r)["vegetation"] = Value::from(other_veg)),
            vec![ItemKey::Vegetation],
        ),
        ("rivers", mutated_map(|v| v["rivers"] = Value::Array(vec![])), vec![ItemKey::Rivers]),
    ];
    for (name, map, expected) in map_cases {
        let after = e.with_map(map).identity();
        assert_eq!(differing(&base, &after), expected, "{name}");
    }

    // ワールド・経済・建物・施設のテキスト変更(必ず期待する項目を含む。副次的な項目も許す場合は含有で確認)。
    let contains_cases: Vec<(&str, Env, ItemKey)> = vec![
        ("province table", e.edited(|t| t.provinces = replace_first(&t.provinces, "next_province_id: 34503", "next_province_id: 34504")), ItemKey::ProvinceTable),
        ("membership", e.edited(|t| t.states = replace_first(&t.states, "18499,", "")), ItemKey::StateMembership),
        ("deposit output", e.edited(|t| t.resources = replace_first(&t.resources, "base_output: 25.0", "base_output: 26.0")), ItemKey::ResourceDeposits),
        ("workforce", e.edited(|t| t.scenario = replace_first(&t.scenario, "workforce_ratio: 0.5", "workforce_ratio: 0.6")), ItemKey::StateEconomyStatic),
        ("logistics", e.edited(|t| t.scenario = replace_first(&t.scenario, "base_logistics_capacity: 200.0", "base_logistics_capacity: 210.0")), ItemKey::StateEconomyStatic),
        ("ai flag", e.edited(|t| t.scenario = replace_first(&t.scenario, "ai_controlled: true", "ai_controlled: false")), ItemKey::ScenarioStructure),
        ("military", e.edited(|t| t.scenario = replace_first(&t.scenario, "at_war: true", "at_war: false")), ItemKey::MilitaryInputs),
        ("building cost", e.edited(|t| t.buildings = replace_first(&t.buildings, "construction_cost: 200.0", "construction_cost: 201.0")), ItemKey::BuildingComputation),
        ("building output", e.edited(|t| t.buildings = replace_first(&t.buildings, "output_resources: { Food: 100.0 }", "output_resources: { Food: 101.0 }")), ItemKey::BuildingComputation),
        ("state name", e.edited(|t| t.states = replace_first(&t.states, "world_preview.state.alpha", "world_preview.state.alpha2")), ItemKey::StatePresentation),
        ("representative", e.edited(|t| t.states = replace_first(&t.states, "representative: None", "representative: Some(18109)")), ItemKey::StatePresentation),
        ("allow_disconnected", e.edited(|t| t.states = replace_first(&t.states, "allow_disconnected: false", "allow_disconnected: true")), ItemKey::StateConstraints),
        ("building name", e.edited(|t| t.buildings = replace_first(&t.buildings, "name: \"Farm\"", "name: \"Farm X\"")), ItemKey::BuildingPresentation),
        ("facility moved", e.edited(|t| t.facilities = replace_first(&t.facilities, "(id: 101, kind: Port, province: 18309)", "(id: 101, kind: Port, province: 18308)")), ItemKey::LocationFacilities),
    ];
    for (name, env, key) in contains_cases {
        let after = env.identity();
        let d = differing(&base, &after);
        assert!(d.contains(&key), "{name}: expected {key} in {d:?}");
    }
    // 純粋に表示だけの変更は、その項目だけ。
    let name_only = e.edited(|t| t.buildings = replace_first(&t.buildings, "name: \"Farm\"", "name: \"Farm X\"")).identity();
    assert_eq!(differing(&base, &name_only), vec![ItemKey::BuildingPresentation]);
    let state_name = e.edited(|t| t.states = replace_first(&t.states, "world_preview.state.alpha", "world_preview.state.alpha2")).identity();
    assert_eq!(differing(&base, &state_name), vec![ItemKey::StatePresentation]);
    let disc = e.edited(|t| t.states = replace_first(&t.states, "allow_disconnected: false", "allow_disconnected: true")).identity();
    assert_eq!(differing(&base, &disc), vec![ItemKey::StateConstraints]);
    let cost = e.edited(|t| t.buildings = replace_first(&t.buildings, "construction_cost: 200.0", "construction_cost: 201.0")).identity();
    assert_eq!(differing(&base, &cost), vec![ItemKey::BuildingComputation]);
    // 位置施設: 提供の有無・内容の変更。
    let none = Env::with(e.map.clone(), e.texts.clone(), false).identity();
    assert_eq!(differing(&base, &none), vec![ItemKey::LocationFacilities]);
    let moved = e.edited(|t| t.facilities = replace_first(&t.facilities, "(id: 101, kind: Port, province: 18309)", "(id: 101, kind: Port, province: 18308)"));
    assert_eq!(differing(&base, &moved.identity()), vec![ItemKey::LocationFacilities]);
}

#[test]
fn non_finite_static_values_have_no_identity_and_block_saving() {
    let e = base_env();
    let mut b = e.buildings.clone_registry();
    b.definitions.get_mut(&BuildingType::Farm).unwrap().construction_cost = f64::NAN;
    let inputs = StaticInputs { map: &e.map, world: &e.world, buildings: &b, facilities: None };
    let sim = e.sim();
    assert!(StaticIdentity::compute(&sim, &inputs).is_err());
    let t = Tmp::new("nan");
    let err = save(&sim, &inputs, &t.loc(), "nan").unwrap_err();
    assert!(matches!(err, FileError::Identity(_)), "{err}");
    assert!(!t.file("nan").exists(), "nothing was written");
}

trait CloneRegistry {
    fn clone_registry(&self) -> BuildingRegistry;
}

impl CloneRegistry for BuildingRegistry {
    fn clone_registry(&self) -> BuildingRegistry {
        BuildingRegistry { definitions: self.definitions.clone() }
    }
}

// ---------------------------------------------------------------- 保存・復元(判定の接続)

fn saved(env: &Env, days: usize, t: &Tmp, name: &str) -> MapEconomySimulation {
    let sim = env.advanced(days);
    save(&sim, &env.inputs(), &t.loc(), name).unwrap();
    sim
}

#[test]
fn an_identical_environment_restores_with_every_item_matched_and_no_warning() {
    let e = base_env();
    let t = Tmp::new("match");
    let sim = saved(e, 100, &t, "s");
    let mut target = e.sim();
    let r = load(&mut target, &e.inputs(), &t.loc(), "s").unwrap();
    assert_eq!(r.verdict.matched, ItemKey::ALL.to_vec());
    assert!(r.verdict.warnings.is_empty() && r.verdict.is_exact());
    assert_eq!(target, sim);
    // ファイルには全項目の識別情報がある。
    let f = read_file(&t.loc(), "s").unwrap();
    assert_eq!(f.identity.scheme, IDENTITY_SCHEME);
    assert_eq!(f.identity.items.len(), ItemKey::ALL.len());
}

/// 警告で許す変更の1つ。変えた環境と、期待する警告の項目。
fn warn_cases(e: &Env) -> Vec<(&'static str, Env, Vec<ItemKey>)> {
    let (q, r) = province_cell(&e.world);
    let cur_t = base_json()["cells"].as_array().unwrap().iter().find(|c| c["q"] == q && c["r"] == r).unwrap()["terrain"].as_str().unwrap().to_string();
    let other_terrain = if cur_t == "hills" { "plains" } else { "hills" };
    vec![
        ("land terrain", e.with_map(mutated_map(|v| cell_mut(v, q, r)["terrain"] = Value::from(other_terrain))), vec![ItemKey::LandTerrain]),
        ("climate", e.with_map(mutated_map(|v| cell_mut(v, q, r)["climate"] = Value::from(other_climate(&cell_climate(q, r))))), vec![ItemKey::Climate]),
        ("vegetation", e.with_map(mutated_map(|v| cell_mut(v, q, r)["vegetation"] = Value::from("none"))), vec![ItemKey::Vegetation]),
        ("rivers", e.with_map(mutated_map(|v| v["rivers"] = Value::Array(vec![]))), vec![ItemKey::Rivers]),
        ("state name", e.edited(|t| t.states = replace_first(&t.states, "world_preview.state.alpha", "world_preview.state.alpha2")), vec![ItemKey::StatePresentation]),
        ("building name", e.edited(|t| t.buildings = replace_first(&t.buildings, "name: \"Farm\"", "name: \"Farm X\"")), vec![ItemKey::BuildingPresentation]),
        ("facilities not provided", Env::with(e.map.clone(), e.texts.clone(), false), vec![ItemKey::LocationFacilities]),
        (
            "terrain + climate + vegetation together",
            e.with_map(mutated_map(|v| {
                cell_mut(v, q, r)["terrain"] = Value::from(other_terrain);
                cell_mut(v, q, r)["climate"] = Value::from(other_climate(&cell_climate(q, r)));
                cell_mut(v, q, r)["vegetation"] = Value::from("none");
            })),
            vec![ItemKey::LandTerrain, ItemKey::Climate, ItemKey::Vegetation],
        ),
    ]
}

#[test]
fn warn_class_changes_restore_with_warnings_keep_dynamic_state_and_resume_identically() {
    let e = base_env();
    // 連続実行(基準)。
    let mut cont = e.sim();
    let mut reports = Vec::new();
    let mut snaps = vec![cont.snapshot(&e.world, &e.buildings)];
    for _ in 0..260 {
        reports.push(cont.advance_day(&e.buildings));
        snaps.push(cont.snapshot(&e.world, &e.buildings));
    }
    assert!(reports.iter().any(|(_, r)| r.monthly_ran) && reports.iter().any(|(_, r)| !r.completed.is_empty()));
    for (name, env2, expected) in warn_cases(e) {
        let t = Tmp::new("warn");
        let sim = saved(e, 100, &t, "s");
        let mut target = env2.sim();
        let r = load(&mut target, &env2.inputs(), &t.loc(), "s").unwrap_or_else(|err| panic!("{name}: {err}"));
        let keys: Vec<ItemKey> = r.verdict.warnings.iter().map(|w| w.key).collect();
        assert_eq!(keys, expected, "{name}: warnings reach the caller");
        assert!(r.verdict.rejections.is_empty() && r.verdict.structural.is_empty());
        assert_eq!(r.verdict.matched.len() + r.verdict.warnings.len(), ItemKey::ALL.len(), "{name}");
        for w in &r.verdict.warnings {
            assert!(!w.rationale.is_empty() && w.saved != w.current);
        }
        // 動的状態は保存された値のまま(初期化・再計算されない)。
        assert_eq!(target.snapshot(&e.world, &e.buildings), snaps[100], "{name}: dynamic state kept");
        assert_eq!(target, sim, "{name}");
        // 再開後の結果が連続実行と一致(月次・建設完成・履歴条件の成立をまたぐ)。
        for d in 101..=260 {
            let rep = target.advance_day(&env2.buildings);
            assert_eq!(rep, reports[d - 1], "{name}: day {d}");
            assert_eq!(target.snapshot(&e.world, &e.buildings), snaps[d], "{name}: state after day {d}");
        }
    }
}

#[test]
fn reject_class_changes_are_refused_and_leave_the_runtime_state_untouched() {
    let e = base_env();
    let (q, r) = province_cell(&e.world);
    let cases: Vec<(&str, Env, ItemKey)> = vec![
        ("grid", e.with_map(mutated_map(|v| v["grid"]["origin"]["x"] = Value::from(2.5))), ItemKey::Grid),
        ("coordinates", e.with_map(mutated_map(|v| { v["cells"].as_array_mut().unwrap().remove(0); })), ItemKey::CellCoordinates),
        ("land becomes water", e.with_map(mutated_map(|v| cell_mut(v, q, r)["terrain"] = Value::from("sea"))), ItemKey::LandSet),
        ("sea becomes lake", e.with_map(mutated_map(|v| {
            let c = v["cells"].as_array_mut().unwrap().iter_mut().find(|c| c["terrain"] == "sea").unwrap();
            c["terrain"] = Value::from("lake");
        })), ItemKey::WaterKinds),
        ("province id/location", e.edited(|t| t.provinces = replace_first(&t.provinces, "next_province_id: 34503", "next_province_id: 34504")), ItemKey::ProvinceTable),
        ("membership", e.edited(|t| t.states = replace_first(&t.states, "18499,", "")), ItemKey::StateMembership),
        ("resource definition", e.edited(|t| t.resources = replace_first(&t.resources, "base_output: 25.0", "base_output: 26.0")), ItemKey::ResourceDeposits),
        ("state static value", e.edited(|t| t.scenario = replace_first(&t.scenario, "workforce_ratio: 0.5", "workforce_ratio: 0.6")), ItemKey::StateEconomyStatic),
        ("country structure", e.edited(|t| t.scenario = replace_first(&t.scenario, "ai_controlled: true", "ai_controlled: false")), ItemKey::ScenarioStructure),
        ("military input", e.edited(|t| t.scenario = replace_first(&t.scenario, "at_war: true", "at_war: false")), ItemKey::MilitaryInputs),
        ("building definition", e.edited(|t| t.buildings = replace_first(&t.buildings, "construction_cost: 200.0", "construction_cost: 201.0")), ItemKey::BuildingComputation),
        ("connectivity flag", e.edited(|t| t.states = replace_first(&t.states, "allow_disconnected: false", "allow_disconnected: true")), ItemKey::StateConstraints),
    ];
    for (name, env2, key) in cases {
        let t = Tmp::new("reject");
        saved(e, 100, &t, "s");
        let bytes_before = fs::read(t.file("s")).unwrap();
        let mut target = env2.advanced(40);
        let before = target.clone();
        let err = load(&mut target, &env2.inputs(), &t.loc(), "s").expect_err(name);
        let FileError::Incompatible(v) = &err else { panic!("{name}: unexpected error {err}") };
        assert!(v.rejections.iter().any(|f| f.key == key), "{name}: {key} not rejected: {v:?}");
        assert!(!v.is_compatible());
        assert_eq!(target, before, "{name}: runtime state is byte-for-byte unchanged");
        assert_eq!(fs::read(t.file("s")).unwrap(), bytes_before, "{name}: file untouched");
    }
}

#[test]
fn a_warn_item_together_with_a_reject_item_is_refused() {
    let e = base_env();
    let (q, r) = province_cell(&e.world);
    // 地形(警告)と建設費(拒否)を同時に変える。
    let env2 = e
        .with_map(mutated_map(|v| cell_mut(v, q, r)["climate"] = Value::from(other_climate(&cell_climate(q, r)))))
        .edited(|t| t.buildings = replace_first(&t.buildings, "construction_cost: 200.0", "construction_cost: 201.0"));
    let t = Tmp::new("mixed");
    saved(e, 60, &t, "s");
    let mut target = env2.advanced(10);
    let before = target.clone();
    let err = load(&mut target, &env2.inputs(), &t.loc(), "s").unwrap_err();
    let FileError::Incompatible(v) = err else { panic!("{err}") };
    assert_eq!(v.warnings.iter().map(|f| f.key).collect::<Vec<_>>(), vec![ItemKey::Climate]);
    assert_eq!(v.rejections.iter().map(|f| f.key).collect::<Vec<_>>(), vec![ItemKey::BuildingComputation]);
    assert_eq!(target, before);
    // 複数の拒否項目が同時にあれば、全部が報告される(一つだけ見逃さない)。
    let env3 = e
        .with_map(mutated_map(|v| v["grid"]["origin"]["x"] = Value::from(2.5)))
        .edited(|t| t.scenario = replace_first(&t.scenario, "at_war: true", "at_war: false"));
    let mut target = env3.sim();
    let err = load(&mut target, &env3.inputs(), &t.loc(), "s").unwrap_err();
    let FileError::Incompatible(v) = err else { panic!("{err}") };
    let keys: Vec<ItemKey> = v.rejections.iter().map(|f| f.key).collect();
    assert!(keys.contains(&ItemKey::Grid) && keys.contains(&ItemKey::MilitaryInputs), "{keys:?}");
}

#[test]
fn missing_unknown_or_unsupported_identity_is_refused() {
    let e = base_env();
    let t = Tmp::new("structural");
    let sim = saved(e, 50, &t, "s");
    let good = read_file(&t.loc(), "s").unwrap();
    let snapshot: MapEconomySnapshot = good.snapshot.clone();
    let mutate = |f: &dyn Fn(&mut strategy_game::economy::map_state::static_identity::SavedIdentity)| {
        let mut id = good.identity.clone();
        f(&mut id);
        encode(&snapshot, &id).unwrap()
    };
    type Check = Box<dyn Fn(&Structural) -> bool>;
    let cases: Vec<(&str, String, Check)> = vec![
        ("missing item", mutate(&|id| id.items.retain(|i| i.key != "land_set")), Box::new(|s| matches!(s, Structural::MissingItem(ItemKey::LandSet)))),
        ("all items missing", mutate(&|id| id.items.clear()), Box::new(|s| matches!(s, Structural::MissingItem(_)))),
        ("unknown item", mutate(&|id| id.items.push(strategy_game::economy::map_state::static_identity::SavedItem { key: "future_item".into(), digest: "0".repeat(64) })), Box::new(|s| matches!(s, Structural::UnknownItem(k) if k == "future_item"))),
        ("duplicate item", mutate(&|id| { let d = id.items[0].clone(); id.items.push(d); }), Box::new(|s| matches!(s, Structural::DuplicateItem(_)))),
        ("other scheme", mutate(&|id| id.scheme = "sha-256".into()), Box::new(|s| matches!(s, Structural::UnsupportedScheme { found } if found == "sha-256"))),
        ("scheme version", mutate(&|id| id.scheme_version = 2), Box::new(|s| matches!(s, Structural::UnsupportedSchemeVersion { found: 2 }))),
        ("short digest", mutate(&|id| id.items[3].digest = "abcd".into()), Box::new(|s| matches!(s, Structural::MalformedDigest { .. }))),
        ("upper-case digest", mutate(&|id| id.items[3].digest = id.items[3].digest.to_uppercase()), Box::new(|s| matches!(s, Structural::MalformedDigest { .. }))),
    ];
    for (name, text, check) in cases {
        t.write_raw("bad", text.as_bytes());
        let mut target = e.advanced(5);
        let before = target.clone();
        let err = load(&mut target, &e.inputs(), &t.loc(), "bad").unwrap_err();
        let FileError::Incompatible(v) = err else { panic!("{name}: {err}") };
        assert!(v.structural.iter().any(&check), "{name}: {v:?}");
        assert!(!v.is_compatible());
        assert_eq!(target, before, "{name}");
    }
    // 不明なフィールド・identity自体の欠落は、封筒の段階で拒否される。
    let text = fs::read_to_string(t.file("s")).unwrap();
    t.write_raw("bad", replace_first(&text, "scheme:", "extra: 1,\n        scheme:").as_bytes());
    let mut target = e.sim();
    assert!(matches!(load(&mut target, &e.inputs(), &t.loc(), "bad").unwrap_err(), FileError::Malformed(_)));
    #[derive(Serialize)]
    struct NoIdentity<'a> {
        kind: &'a str,
        format_version: u32,
        snapshot: &'a MapEconomySnapshot,
    }
    let no_id = ron::ser::to_string_pretty(
        &NoIdentity { kind: "map_economy_state", format_version: 2, snapshot: &snapshot },
        ron::ser::PrettyConfig::default(),
    )
    .unwrap();
    t.write_raw("bad", no_id.as_bytes());
    assert!(matches!(load(&mut target, &e.inputs(), &t.loc(), "bad").unwrap_err(), FileError::Malformed(_)));
    assert_eq!(target, e.sim());
    let _ = sim;
}

#[test]
fn identity_values_in_the_file_are_never_adopted_for_the_target() {
    // 保存側が別の環境のとき、ファイルの値が復元先に「コピー」されて通ることはない。
    let e = base_env();
    let env2 = e.edited(|t| t.buildings = replace_first(&t.buildings, "construction_cost: 200.0", "construction_cost: 201.0"));
    let t = Tmp::new("adopt");
    saved(&env2, 60, &t, "s");
    let mut target = e.sim();
    let before = target.clone();
    let err = load(&mut target, &e.inputs(), &t.loc(), "s").unwrap_err();
    assert!(matches!(err, FileError::Incompatible(_)), "{err}");
    assert_eq!(target, before);
    // 判定そのものも、保存側と復元先の値を比べるだけ。
    let verdict = judge(&read_file(&t.loc(), "s").unwrap().identity, &e.identity());
    assert_eq!(verdict.rejections.iter().map(|f| f.key).collect::<Vec<_>>(), vec![ItemKey::BuildingComputation]);
}

#[test]
fn the_economy_rules_revision_item_rejects_a_different_revision() {
    let e = base_env();
    let t = Tmp::new("rules");
    saved(e, 20, &t, "s");
    let mut id = read_file(&t.loc(), "s").unwrap().identity;
    // 別の改訂番号のダイジェスト(別の規則で保存されたファイルを模す)に差し替える。
    id.items.iter_mut().find(|i| i.key == "economy_rules").unwrap().digest = "1".repeat(64);
    let verdict = judge(&id, &e.identity());
    assert_eq!(verdict.rejections.iter().map(|f| f.key).collect::<Vec<_>>(), vec![ItemKey::EconomyRules]);
    assert!(!verdict.is_compatible());
}

// ---------------------------------------------------------------- 内部コンテキスト照合との関係

#[test]
fn the_internal_context_covers_reject_items_only_so_warned_differences_are_not_re_refused() {
    let e = base_env();
    let sim = e.advanced(80);
    let snap = sim.snapshot(&e.world, &e.buildings);
    // 世界ファイルに記録された属性・河川の識別が変わっても(再同期した世界ファイルを模す)、内部照合は通る。
    let mut table = parse_province_table(&e.texts.provinces).unwrap();
    table.geography.attributes = "ffffffffffffffff".into();
    table.geography.rivers = "eeeeeeeeeeeeeeee".into();
    let w2 = WorldModel::build(table, parse_state_layout(&e.texts.states).unwrap()).unwrap();
    let mut target = sim_of(&e.texts, &w2, &e.buildings);
    target.restore(&snap, &w2, &e.buildings).expect("attributes/rivers are outside the internal context");
    assert_eq!(target, sim);
    // 陸集合・グリッドの識別が変われば、内部照合も拒否する(無効化されていない)。
    for tamper in [0u8, 1] {
        let mut table = parse_province_table(&e.texts.provinces).unwrap();
        if tamper == 0 {
            table.geography.land_set = "0000000000000000".into();
        } else {
            table.geography.grid = "0000000000000000".into();
        }
        let w3 = WorldModel::build(table, parse_state_layout(&e.texts.states).unwrap()).unwrap();
        let mut target = sim_of(&e.texts, &w3, &e.buildings);
        let before = target.clone();
        let err = target.restore(&snap, &w3, &e.buildings).unwrap_err();
        assert!(err.issues.contains(&RestoreIssue::ContextMismatch), "{err}");
        assert_eq!(target, before);
    }
}

// ---------------------------------------------------------------- 形式バージョン

#[test]
fn format_1_files_are_refused_with_a_clear_reason_and_are_neither_converted_nor_overwritten() {
    let e = base_env();
    let t = Tmp::new("format1");
    let sim = e.advanced(30);
    #[derive(Serialize)]
    struct Format1<'a> {
        kind: &'a str,
        format_version: u32,
        snapshot: &'a MapEconomySnapshot,
    }
    let snap = sim.snapshot(&e.world, &e.buildings);
    let old = ron::ser::to_string_pretty(
        &Format1 { kind: "map_economy_state", format_version: 1, snapshot: &snap },
        ron::ser::PrettyConfig::default(),
    )
    .unwrap();
    t.write_raw("old", old.as_bytes());
    let mut target = e.sim();
    let before = target.clone();
    let err = load(&mut target, &e.inputs(), &t.loc(), "old").unwrap_err();
    assert!(matches!(err, FileError::UnsupportedVersion { found: 1 }), "{err}");
    let text = err.to_string();
    assert!(text.contains("format_version 1") && text.contains("no per-item static identity"), "{text}");
    assert!(text.contains("unrelated to the legacy game SaveGameV1"), "mapeco format 1 is not SaveGameV1: {text}");
    assert_eq!(target, before);
    assert_eq!(fs::read(t.file("old")).unwrap(), old.as_bytes(), "the old file is not rewritten");
    // 上書き保存の対象にもならない(自形式の現行版として識別できないため)。
    let err = save(&sim, &e.inputs(), &t.loc(), "old").unwrap_err();
    assert!(matches!(err, FileError::OverwriteRefused { .. }), "{err}");
    assert_eq!(fs::read(t.file("old")).unwrap(), old.as_bytes());
    // 現行版の本文は decode でき、識別情報を持つ。
    let new_text = fs::read_to_string({
        save(&sim, &e.inputs(), &t.loc(), "new").unwrap();
        t.file("new")
    })
    .unwrap();
    assert_eq!(decode(&new_text).unwrap().identity.items.len(), ItemKey::ALL.len());
}

#[test]
fn the_legacy_v1_save_and_other_kinds_are_still_protected() {
    let e = base_env();
    let t = Tmp::new("v1");
    let real_before = fs::read("saves/savegame_v1.ron").unwrap();
    t.write_raw("v1", &real_before);
    let sim = e.advanced(10);
    let mut target = e.sim();
    let err = load(&mut target, &e.inputs(), &t.loc(), "v1").unwrap_err();
    assert!(matches!(err, FileError::NotThisKind(_)), "{err}");
    let err = save(&sim, &e.inputs(), &t.loc(), "v1").unwrap_err();
    assert!(matches!(err, FileError::OverwriteRefused { .. }), "{err}");
    assert_eq!(fs::read(t.file("v1")).unwrap(), real_before);
    assert_eq!(fs::read("saves/savegame_v1.ron").unwrap(), real_before);
    assert!(save(&sim, &e.inputs(), &t.loc(), "savegame_v1").is_ok(), "a name never maps to the V1 slot");
    assert_eq!(fs::read("saves/savegame_v1.ron").unwrap(), real_before);
}

// ---------------------------------------------------------------- 別プロセス(CLI)

fn cli(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_world_economy_check")).args(args).output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn day_lines(stdout: &str) -> Vec<String> {
    stdout.lines().filter(|l| l.starts_with("day ")).map(str::to_string).collect()
}

#[test]
fn a_separate_process_reports_matches_warnings_and_rejections_and_resumes_identically() {
    let t = Tmp::new("cli");
    let base = t.0.to_string_lossy().to_string();
    let (ok, out, err) = cli(&["--economy-save", "100", "--save-base", &base, "--name", "cli"]);
    assert!(ok && out.contains("保存成功"), "{out}{err}");
    let (ok, cont, err) = cli(&["--economy-run", "160", "--digest-from", "100"]);
    assert!(ok, "{cont}{err}");
    let expected = day_lines(&cont);
    assert_eq!(expected.len(), 60);
    assert!(expected[0].starts_with("day 1800/04/12"), "{}", expected[0]);
    assert!(expected[59].starts_with("day 1800/06/10"), "{}", expected[59]);

    // 1. 同じ入力: 全項目一致・警告なし。
    let (ok, out, err) = cli(&["--economy-resume", "60", "--save-base", &base, "--name", "cli"]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("一致 21 項目") && out.contains("警告つき許可 0 項目"), "{out}");
    assert_eq!(day_lines(&out), expected);

    // 2. 警告で許す変更(陸の地形・建物名): 警告が表示され、再開後の結果は連続実行と一致する。
    let e = base_env();
    let (q, r) = province_cell(&e.world);
    let cur_t = base_json()["cells"].as_array().unwrap().iter().find(|c| c["q"] == q && c["r"] == r).unwrap()["terrain"].as_str().unwrap().to_string();
    let other = if cur_t == "hills" { "plains" } else { "hills" };
    let map_warn = t.0.join("map_warn.json");
    let mut v = base_json().clone();
    cell_mut(&mut v, q, r)["terrain"] = Value::from(other);
    fs::write(&map_warn, serde_json::to_vec(&v).unwrap()).unwrap();
    let (ok, out, err) = cli(&["--economy-resume", "60", "--save-base", &base, "--name", "cli", "--map-file", map_warn.to_str().unwrap()]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("警告(許可): land_terrain") && out.contains("警告つき許可 1 項目"), "{out}");
    assert_eq!(day_lines(&out), expected, "resume with a warned difference equals the continuous run");

    let names = t.0.join("buildings_names.ron");
    fs::write(&names, replace_first(&e.texts.buildings, "name: \"Farm\"", "name: \"Farm X\"")).unwrap();
    let (ok, out, err) = cli(&["--economy-resume", "60", "--save-base", &base, "--name", "cli", "--buildings-file", names.to_str().unwrap()]);
    assert!(ok && out.contains("警告(許可): building_presentation"), "{out}{err}");
    assert_eq!(day_lines(&out), expected);

    // 3. 拒否(建物の計算に関わる定義): 非ゼロ終了。拒否項目と理由が表示され、日次の出力はない。
    let cost = t.0.join("buildings_cost.ron");
    fs::write(&cost, replace_first(&e.texts.buildings, "construction_cost: 200.0", "construction_cost: 201.0")).unwrap();
    let (ok, out, err) = cli(&["--economy-resume", "60", "--save-base", &base, "--name", "cli", "--buildings-file", cost.to_str().unwrap()]);
    assert!(!ok && out.contains("拒否: building_computation") && !out.contains("復元成功"), "{out}{err}");
    assert!(day_lines(&out).is_empty() && err.contains("復元失敗"), "{out}{err}");

    // 4. 警告と拒否の同時: 拒否(警告は許可されない)。
    let (ok, out, err) = cli(&[
        "--economy-resume", "60", "--save-base", &base, "--name", "cli",
        "--map-file", map_warn.to_str().unwrap(), "--buildings-file", cost.to_str().unwrap(),
    ]);
    assert!(!ok && out.contains("拒否: building_computation") && out.contains("警告(拒否項目が併存するため許可されない): land_terrain"), "{out}{err}");
    assert!(day_lines(&out).is_empty());

    // 5. 形式1のファイル: 非ゼロ終了で理由を表示する。
    let snap = e.advanced(10).snapshot(&e.world, &e.buildings);
    #[derive(Serialize)]
    struct Format1<'a> {
        kind: &'a str,
        format_version: u32,
        snapshot: &'a MapEconomySnapshot,
    }
    let old = ron::ser::to_string_pretty(&Format1 { kind: "map_economy_state", format_version: 1, snapshot: &snap }, ron::ser::PrettyConfig::default()).unwrap();
    t.write_raw("old", old.as_bytes());
    let (ok, out, err) = cli(&["--economy-resume", "5", "--save-base", &base, "--name", "old"]);
    assert!(!ok && err.contains("format_version 1") && !out.contains("復元成功"), "{out}{err}");
}

fn cell_climate(q: i64, r: i64) -> String {
    base_json()["cells"].as_array().unwrap().iter().find(|c| c["q"] == q && c["r"] == r).unwrap()["climate"]
        .as_str()
        .unwrap()
        .to_string()
}

/// 現在と違う気候ID。
fn other_climate(cur: &str) -> &'static str {
    ["A", "B", "C", "D", "E"].into_iter().find(|c| *c != cur).unwrap()
}
