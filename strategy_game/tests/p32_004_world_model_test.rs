//! P32-004 (M0): ワールドモデル(プロヴィンス/ステート)のデータ層テスト。
//! 小地域のフィクスチャ(`tests/fixtures/p32_004/`、正式な世界設定ではない)と、
//! 実地図(`assets/data/hex_atlas_map.json`)に対するメモリ上の検証の両方を使う。

use std::path::PathBuf;
use strategy_game::hexmap::data::{HexMap, parse_hex_map};
use strategy_game::world::*;

const SMALL_MAP: &str = include_str!("fixtures/p32_004/small_map.json");
const SMALL_LAYOUT: &str = include_str!("fixtures/p32_004/small_layout.ron");

fn small_map_from(json: &str) -> HexMap {
    parse_hex_map(json.as_bytes()).expect("fixture map must parse")
}
fn small_map() -> HexMap {
    small_map_from(SMALL_MAP)
}
fn table_for(map: &HexMap) -> ProvinceTableFile {
    generate_initial_province_table(map, Coverage::World)
}
fn small_model() -> (HexMap, WorldModel) {
    let map = small_map();
    let table = table_for(&map);
    let layout = parse_state_layout(SMALL_LAYOUT).unwrap();
    let model = WorldModel::build(table, layout).expect("fixture world must build");
    (map, model)
}
fn layout_with(states: Vec<StateEntry>) -> StateLayoutFile {
    StateLayoutFile {
        format_version: WORLD_FORMAT_VERSION,
        states,
    }
}
fn state(id: u32, provinces: &[u32]) -> StateEntry {
    StateEntry {
        id,
        name_key: format!("test.state.{id}"),
        provinces: provinces.to_vec(),
        representative: None,
        allow_disconnected: false,
    }
}
fn codes(issues: &[WorldIssue]) -> Vec<IssueCode> {
    issues.iter().map(|i| i.code).collect()
}
fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("p32_004_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

// ---------------------------------------------------------------- 型・ID・検索

#[test]
fn id_types_are_distinct() {
    use std::any::TypeId;
    assert_ne!(TypeId::of::<ProvinceId>(), TypeId::of::<MapStateId>());
    // 同じ数値でも別種のIDで、旧StateIdとも別型(相互変換を提供しない)。
    assert_ne!(
        TypeId::of::<MapStateId>(),
        TypeId::of::<strategy_game::common::StateId>()
    );
    assert_eq!(ProvinceId(3).0, MapStateId(3).0);
}

#[test]
fn stable_ids_load_and_lookups_agree() {
    let (_, m) = small_model();
    assert_eq!(m.provinces().len(), 7);
    // 座標 → ID(初回採番は (r,q) 昇順)
    assert_eq!(m.province_at(1, 0).unwrap().id, ProvinceId(0));
    assert_eq!(m.province_at(3, 2).unwrap().id, ProvinceId(6));
    // 水域(海・湖)・存在しない座標はプロヴィンスではない
    assert!(m.province_at(0, 0).is_none(), "sea");
    assert!(m.province_at(2, 1).is_none(), "lake");
    assert!(m.province_at(99, 99).is_none());
    // ID → プロヴィンス、プロヴィンス → ステート、ステート → プロヴィンス
    assert_eq!(m.province(ProvinceId(4)).unwrap().cell, HexCoord::new(3, 1));
    assert_eq!(m.state_of(ProvinceId(1)), Some(MapStateId(10)));
    assert_eq!(m.state_of(ProvinceId(5)), Some(MapStateId(11)));
    assert_eq!(
        m.provinces_of(MapStateId(10)).unwrap(),
        &[ProvinceId(0), ProvinceId(1), ProvinceId(2), ProvinceId(3)]
    );
    assert!(m.provinces_of(MapStateId(99)).is_none());
    assert!(m.check_state_ref(MapStateId(10)).is_ok());
    assert_eq!(
        m.check_state_ref(MapStateId(99)).unwrap_err().code,
        IssueCode::UnknownState
    );
    // 逆引きは正本(ステート→プロヴィンス)と常に一致する
    for s in m.states() {
        for p in &s.provinces {
            assert_eq!(m.state_of(*p), Some(s.id));
        }
    }
}

#[test]
fn shuffled_input_gives_identical_model_and_ids() {
    let map = small_map();
    let table = table_for(&map);
    let layout = parse_state_layout(SMALL_LAYOUT).unwrap();
    let base = WorldModel::build(table.clone(), layout.clone()).unwrap();

    let mut t2 = table.clone();
    t2.provinces.reverse();
    t2.retired.reverse();
    let mut l2 = layout.clone();
    l2.states.reverse();
    for s in &mut l2.states {
        s.provinces.reverse();
    }
    let shuffled = WorldModel::build(t2, l2).unwrap();
    assert_eq!(base, shuffled);
    assert_eq!(shuffled.province_at(3, 1).unwrap().id, ProvinceId(4));
}

#[test]
fn json_cell_order_and_formatting_do_not_change_fingerprint_or_ids() {
    let v: serde_json::Value = serde_json::from_str(SMALL_MAP).unwrap();
    let mut shuffled = v.clone();
    shuffled["cells"].as_array_mut().unwrap().reverse();
    shuffled["rivers"].as_array_mut().unwrap().reverse();
    let compact = serde_json::to_string(&shuffled).unwrap();
    let a = small_map();
    let b = small_map_from(&compact);
    assert_eq!(compute_fingerprint(&a), compute_fingerprint(&b));
    // 並べ替え後のJSONから再生成してもIDは同じ((r,q)順で採番するため)。
    assert_eq!(table_for(&a).provinces, table_for(&b).provinces);
    // 既存の表を使う限り、並び替え後の地図でもIDは変わらず検証も通る。
    let model =
        WorldModel::build(table_for(&a), parse_state_layout(SMALL_LAYOUT).unwrap()).unwrap();
    let report = validate_against_geography(&model, &b, ValidationMode::GameStart);
    assert!(report.is_ok(), "{:#?}", report.issues);
}

// ---------------------------------------------------------------- 地理データとの照合

fn replace_in_map(from: &str, to: &str) -> HexMap {
    assert!(SMALL_MAP.contains(from), "fixture must contain {from}");
    small_map_from(&SMALL_MAP.replacen(from, to, 1))
}

#[test]
fn attribute_change_keeps_ids_and_membership() {
    let (_, model) = small_model();
    let changed = replace_in_map(
        r#""q": 2, "r": 0, "terrain": "plains", "climate": "C", "vegetation": "forest""#,
        r#""q": 2, "r": 0, "terrain": "plains", "climate": "B", "vegetation": "none""#,
    );
    let report = validate_against_geography(&model, &changed, ValidationMode::GameStart);
    assert!(report.is_ok(), "{:#?}", report.issues);
    assert_eq!(
        report.severity_of(IssueCode::AttributesChanged),
        Some(Severity::Info)
    );
    assert!(!report.has(IssueCode::LandSetChanged));
    assert!(report.world_coverage_verified);
    // 同じIDと所属がそのまま使える
    assert_eq!(
        model.state_of(model.province_at(2, 0).unwrap().id),
        Some(MapStateId(10))
    );
    let (a, b) = (
        compute_fingerprint(&small_map()),
        compute_fingerprint(&changed),
    );
    let ch = a.compare(&b);
    assert!(ch.attributes && !ch.grid && !ch.coordinates && !ch.land_set);
}

#[test]
fn terrain_change_between_land_types_is_an_attribute_change() {
    let (_, model) = small_model();
    let changed = replace_in_map(
        r#""q": 3, "r": 0, "terrain": "hills""#,
        r#""q": 3, "r": 0, "terrain": "mountain""#,
    );
    let report = validate_against_geography(&model, &changed, ValidationMode::GameStart);
    assert!(report.is_ok());
    assert!(report.has(IssueCode::AttributesChanged));
}

#[test]
fn river_change_is_detected_but_harmless() {
    let (_, model) = small_model();
    let changed = small_map_from(&SMALL_MAP.replace(
        r#"{ "q": 3, "r": 0, "direction": 5 }"#,
        r#"{ "q": 3, "r": 0, "direction": 4 }"#,
    ));
    let report = validate_against_geography(&model, &changed, ValidationMode::GameStart);
    assert!(report.is_ok());
    assert_eq!(
        report.severity_of(IssueCode::RiversChanged),
        Some(Severity::Info)
    );
}

#[test]
fn land_to_water_with_same_coordinate_set_is_detected() {
    let (_, model) = small_model();
    let changed = replace_in_map(
        r#""q": 1, "r": 1, "terrain": "plains", "climate": "C", "vegetation": "none""#,
        r#""q": 1, "r": 1, "terrain": "lake", "climate": "unclassified", "vegetation": "unclassified""#,
    );
    let ch = model.geography().compare(&compute_fingerprint(&changed));
    assert!(!ch.coordinates, "coordinate set is unchanged");
    assert!(ch.land_set, "but the land set changed");
    let report = validate_against_geography(&model, &changed, ValidationMode::Authoring);
    assert!(!report.is_ok());
    assert!(report.has(IssueCode::LandSetChanged));
    assert!(report.has(IssueCode::ProvinceCellIsWater));
    assert!(!report.world_coverage_verified);
    let msg = &report
        .issues
        .iter()
        .find(|i| i.code == IssueCode::ProvinceCellIsWater)
        .unwrap()
        .message;
    assert!(
        msg.contains("Province#3") && msg.contains("q=1, r=1"),
        "{msg}"
    );
}

#[test]
fn water_to_land_leaves_a_land_cell_without_province() {
    let (_, model) = small_model();
    let changed = replace_in_map(
        r#""q": 2, "r": 1, "terrain": "lake", "climate": "unclassified", "vegetation": "unclassified""#,
        r#""q": 2, "r": 1, "terrain": "plains", "climate": "C", "vegetation": "grassland""#,
    );
    let report = validate_against_geography(&model, &changed, ValidationMode::Authoring);
    assert!(!report.is_ok());
    assert!(report.has(IssueCode::LandSetChanged));
    assert_eq!(
        report.severity_of(IssueCode::LandCellWithoutProvince),
        Some(Severity::Error)
    );
    assert!(report.issues.iter().any(|i| i.message.contains("q=2, r=1")));
}

#[test]
fn removed_land_cell_is_detected_as_missing_province_cell() {
    let (_, model) = small_model();
    let json = SMALL_MAP.replace(
        r#"    { "q": 3, "r": 2, "terrain": "plateau", "climate": "B", "vegetation": "none" },"#,
        "",
    );
    let changed = small_map_from(&json);
    let report = validate_against_geography(&model, &changed, ValidationMode::Authoring);
    assert!(!report.is_ok());
    assert!(report.has(IssueCode::LandSetChanged));
    assert!(report.has(IssueCode::ProvinceCellMissing));
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.message.contains("Province#6"))
    );
}

#[test]
fn added_land_cell_is_detected() {
    let (_, model) = small_model();
    // extent内の新しい陸セル(0,2)を追加(元は海)ではなく、未使用座標を足す
    let json = SMALL_MAP.replace(
        r#""extent": { "minQ": 0, "maxQ": 4, "minR": 0, "maxR": 2 }"#,
        r#""extent": { "minQ": 0, "maxQ": 5, "minR": 0, "maxR": 2 }"#,
    );
    let json = json.replace(
        r#"    { "q": 4, "r": 2, "terrain": "sea", "climate": "unclassified", "vegetation": "unclassified" }"#,
        r#"    { "q": 4, "r": 2, "terrain": "sea", "climate": "unclassified", "vegetation": "unclassified" },
    { "q": 5, "r": 2, "terrain": "plains", "climate": "C", "vegetation": "grassland" }"#,
    );
    let changed = small_map_from(&json);
    let report = validate_against_geography(&model, &changed, ValidationMode::Authoring);
    assert!(report.has(IssueCode::GridChanged), "extent changed");
    assert!(report.has(IssueCode::LandSetChanged));
    assert_eq!(
        report.severity_of(IssueCode::LandCellWithoutProvince),
        Some(Severity::Error)
    );
    assert!(!report.is_ok());
}

#[test]
fn water_coordinate_addition_is_only_a_warning() {
    let (_, model) = small_model();
    let json = SMALL_MAP.replace(
        r#"    { "q": 4, "r": 2, "terrain": "sea", "climate": "unclassified", "vegetation": "unclassified" }"#,
        "",
    ).replace(r#"{ "q": 3, "r": 2, "terrain": "plateau", "climate": "B", "vegetation": "none" },"#, r#"{ "q": 3, "r": 2, "terrain": "plateau", "climate": "B", "vegetation": "none" }"#);
    let changed = small_map_from(&json);
    let ch = model.geography().compare(&compute_fingerprint(&changed));
    assert!(ch.coordinates && !ch.land_set);
    let report = validate_against_geography(&model, &changed, ValidationMode::GameStart);
    assert!(report.is_ok(), "{:#?}", report.issues);
    assert_eq!(
        report.severity_of(IssueCode::CoordinatesChanged),
        Some(Severity::Warning)
    );
}

#[test]
fn grid_definition_change_is_an_error() {
    let (_, model) = small_model();
    let changed = replace_in_map(r#""size": 2.5"#, r#""size": 3.0"#);
    let report = validate_against_geography(&model, &changed, ValidationMode::Authoring);
    assert!(report.has(IssueCode::GridChanged));
    assert!(!report.is_ok());
}

#[test]
fn province_on_missing_or_water_cell_in_table_is_rejected() {
    let map = small_map();
    // 存在しないセル / 海セルを指すプロヴィンス
    let mut table = table_for(&map);
    table.provinces.push(ProvinceEntry {
        id: 7,
        q: 40,
        r: 40,
    });
    table.provinces.push(ProvinceEntry { id: 8, q: 0, r: 0 });
    table.next_province_id = 9;
    let model = WorldModel::build(table, parse_state_layout(SMALL_LAYOUT).unwrap()).unwrap();
    let report = validate_against_geography(&model, &map, ValidationMode::Authoring);
    assert!(report.has(IssueCode::ProvinceCellMissing));
    assert!(report.has(IssueCode::ProvinceCellIsWater));
    let water = report
        .issues
        .iter()
        .find(|i| i.code == IssueCode::ProvinceCellIsWater)
        .unwrap();
    assert!(
        water.message.contains("Province#8") && water.message.contains("sea"),
        "{}",
        water.message
    );
}

// ---------------------------------------------------------------- 構造の検証

#[test]
fn rejects_duplicate_province_ids_and_cells() {
    let map = small_map();
    let layout = parse_state_layout(SMALL_LAYOUT).unwrap();

    let mut t = table_for(&map);
    t.provinces.push(ProvinceEntry { id: 0, q: 2, r: 2 });
    let err = WorldModel::build(t, layout.clone()).unwrap_err();
    assert!(codes(&err).contains(&IssueCode::DuplicateProvinceId));
    assert!(err.iter().any(|i| i.message.contains("Province#0")));

    let mut t = table_for(&map);
    t.provinces.push(ProvinceEntry { id: 7, q: 1, r: 0 });
    t.next_province_id = 8;
    let err = WorldModel::build(t, layout).unwrap_err();
    assert!(codes(&err).contains(&IssueCode::DuplicateProvinceCell));
    assert!(err.iter().any(|i| i.message.contains("q=1, r=0")));
}

#[test]
fn rejects_id_reuse_violations() {
    let map = small_map();
    let layout = parse_state_layout(SMALL_LAYOUT).unwrap();
    let mut t = table_for(&map);
    t.next_province_id = 3; // 既存IDより小さい高水位標
    let err = WorldModel::build(t, layout.clone()).unwrap_err();
    assert!(codes(&err).contains(&IssueCode::ProvinceIdNotBelowNext));

    let mut t = table_for(&map);
    t.retired.push(2); // 使用中のIDを廃止扱い
    let err = WorldModel::build(t, layout).unwrap_err();
    assert!(codes(&err).contains(&IssueCode::RetiredIdInUse));
}

#[test]
fn retired_ids_are_kept_and_cannot_be_referenced() {
    let map = small_map();
    let mut t = table_for(&map);
    t.provinces.retain(|p| p.id != 6);
    t.retired.push(6);
    // 廃止IDを所属に含めたままだとエラー(IDを別の土地へ再利用させない)
    let err = WorldModel::build(t.clone(), parse_state_layout(SMALL_LAYOUT).unwrap()).unwrap_err();
    let msg = err
        .iter()
        .find(|i| i.code == IssueCode::UnknownProvinceInState)
        .unwrap();
    assert!(
        msg.message.contains("Province#6") && msg.message.contains("retired"),
        "{}",
        msg.message
    );
    // 所属から外せば構築でき、廃止IDは保持される。新規発行は next_province_id 以上。
    let layout = layout_with(vec![state(10, &[0, 1, 2, 3]), state(11, &[4, 5])]);
    let m = WorldModel::build(t, layout).unwrap();
    assert_eq!(m.retired_ids(), &[ProvinceId(6)]);
    assert_eq!(m.next_province_id(), 7);
}

#[test]
fn rejects_bad_state_definitions() {
    let map = small_map();
    let t = || table_for(&map);

    let err =
        WorldModel::build(t(), layout_with(vec![state(1, &[0]), state(1, &[1])])).unwrap_err();
    assert!(codes(&err).contains(&IssueCode::DuplicateStateId));

    let err = WorldModel::build(t(), layout_with(vec![state(1, &[0, 77])])).unwrap_err();
    assert!(codes(&err).contains(&IssueCode::UnknownProvinceInState));
    assert!(
        err.iter()
            .any(|i| i.message.contains("Province#77") && i.message.contains("MapState#1"))
    );

    let err = WorldModel::build(t(), layout_with(vec![state(1, &[0, 1]), state(2, &[1, 2])]))
        .unwrap_err();
    assert!(codes(&err).contains(&IssueCode::ProvinceInMultipleStates));
    assert!(err.iter().any(|i| i.message.contains("Province#1")
        && i.message.contains("MapState#1")
        && i.message.contains("MapState#2")));

    let err = WorldModel::build(t(), layout_with(vec![state(1, &[])])).unwrap_err();
    assert!(codes(&err).contains(&IssueCode::EmptyState));

    let mut s = state(1, &[0, 1]);
    s.representative = Some(5);
    let err = WorldModel::build(t(), layout_with(vec![s])).unwrap_err();
    assert!(codes(&err).contains(&IssueCode::RepresentativeNotMember));
}

#[test]
fn all_structural_problems_are_reported_together() {
    let map = small_map();
    let err = WorldModel::build(
        table_for(&map),
        layout_with(vec![state(1, &[0, 77]), state(1, &[2]), state(2, &[])]),
    )
    .unwrap_err();
    let c = codes(&err);
    assert!(c.contains(&IssueCode::UnknownProvinceInState));
    assert!(c.contains(&IssueCode::DuplicateStateId));
    assert!(c.contains(&IssueCode::EmptyState));
}

// ---------------------------------------------------------------- 制作途中 / ゲーム開始用

#[test]
fn authoring_tolerates_unassigned_but_game_start_rejects() {
    let map = small_map();
    // 4プロヴィンスだけをステートへ入れ、残り3つは未割当
    let model =
        WorldModel::build(table_for(&map), layout_with(vec![state(10, &[0, 1, 2, 3])])).unwrap();
    assert_eq!(
        model.unassigned_provinces(),
        vec![ProvinceId(4), ProvinceId(5), ProvinceId(6)]
    );
    // 未割当は None(ダミーのステートやID=0へは置き換えない)
    assert_eq!(model.state_of(ProvinceId(4)), None);

    let authoring = validate_against_geography(&model, &map, ValidationMode::Authoring);
    assert!(authoring.is_ok(), "{:#?}", authoring.issues);
    assert_eq!(
        authoring.severity_of(IssueCode::UnassignedProvince),
        Some(Severity::Warning)
    );
    assert_eq!(authoring.unassigned_province_count, 3);

    let start = validate_against_geography(&model, &map, ValidationMode::GameStart);
    assert!(!start.is_ok());
    assert_eq!(
        start.severity_of(IssueCode::UnassignedProvince),
        Some(Severity::Error)
    );
    let msg = &start
        .issues
        .iter()
        .find(|i| i.code == IssueCode::UnassignedProvince)
        .unwrap()
        .message;
    assert!(
        msg.contains("Province#4") && msg.contains("Province#6"),
        "{msg}"
    );
}

#[test]
fn fully_assigned_world_passes_game_start() {
    let (map, model) = small_model();
    let r = validate_against_geography(&model, &map, ValidationMode::GameStart);
    assert!(r.is_ok(), "{:#?}", r.issues);
    assert!(r.world_coverage_verified);
    assert_eq!(r.unassigned_province_count, 0);
}

#[test]
fn partial_table_is_not_a_verified_world() {
    let map = small_map();
    // 小地域だけを網羅する表(Partial)。残りの陸セルにはプロヴィンスが無い。
    let mut table = generate_initial_province_table(&map, Coverage::Partial);
    table.provinces.retain(|p| p.id <= 3);
    let model = WorldModel::build(table, layout_with(vec![state(10, &[0, 1, 2, 3])])).unwrap();

    let authoring = validate_against_geography(&model, &map, ValidationMode::Authoring);
    assert!(authoring.is_ok(), "{:#?}", authoring.issues);
    assert!(
        !authoring.world_coverage_verified,
        "a partial table must not count as whole-world verification"
    );
    assert_eq!(
        authoring.severity_of(IssueCode::LandCellWithoutProvince),
        Some(Severity::Warning)
    );
    assert_eq!(
        authoring.severity_of(IssueCode::PartialCoverage),
        Some(Severity::Info)
    );

    let start = validate_against_geography(&model, &map, ValidationMode::GameStart);
    assert!(!start.is_ok());
    assert_eq!(
        start.severity_of(IssueCode::PartialCoverage),
        Some(Severity::Error)
    );
}

#[test]
fn world_table_missing_land_cells_is_an_error_in_every_mode() {
    let map = small_map();
    let mut table = table_for(&map); // Coverage::World
    table.provinces.retain(|p| p.id != 3);
    let model = WorldModel::build(
        table,
        layout_with(vec![state(10, &[0, 1, 2]), state(11, &[4, 5, 6])]),
    )
    .unwrap();
    for mode in [ValidationMode::Authoring, ValidationMode::GameStart] {
        let r = validate_against_geography(&model, &map, mode);
        assert!(!r.is_ok());
        assert_eq!(
            r.severity_of(IssueCode::LandCellWithoutProvince),
            Some(Severity::Error)
        );
        assert!(r.issues.iter().any(|i| i.message.contains("q=1, r=1")));
    }
}

#[test]
fn disconnected_states_are_warnings_not_errors() {
    let map = small_map();
    // (1,0) と (3,1) は隣接しない
    let mut s = state(10, &[0, 4]);
    let model = WorldModel::build(
        table_for(&map),
        layout_with(vec![s.clone(), state(11, &[1, 2, 3, 5, 6])]),
    )
    .unwrap();
    let r = validate_against_geography(&model, &map, ValidationMode::GameStart);
    assert!(
        r.is_ok(),
        "islands/exclaves must not be rejected: {:#?}",
        r.issues
    );
    assert_eq!(
        r.severity_of(IssueCode::StateDisconnected),
        Some(Severity::Warning)
    );
    assert!(r.issues.iter().any(|i| i.message.contains("MapState#10")));
    // allow_disconnected で明示すると警告は出ない
    s.allow_disconnected = true;
    let mut other = state(11, &[1, 2, 3, 5, 6]);
    other.allow_disconnected = true;
    let model = WorldModel::build(table_for(&map), layout_with(vec![s, other])).unwrap();
    let r = validate_against_geography(&model, &map, ValidationMode::GameStart);
    assert!(!r.has(IssueCode::StateDisconnected));
}

// ---------------------------------------------------------------- ファイル形式・ローダー

#[test]
fn table_and_layout_round_trip_through_ron() {
    let map = small_map();
    let table = table_for(&map);
    let text = ron::ser::to_string_pretty(&table, ron::ser::PrettyConfig::default()).unwrap();
    assert_eq!(parse_province_table(&text).unwrap(), table);
    let layout = parse_state_layout(SMALL_LAYOUT).unwrap();
    let text = ron::ser::to_string_pretty(&layout, ron::ser::PrettyConfig::default()).unwrap();
    assert_eq!(parse_state_layout(&text).unwrap(), layout);
}

#[test]
fn rejects_unsupported_versions_and_malformed_files() {
    let fx = |n: &str| format!("{}/tests/fixtures/p32_004/{n}", env!("CARGO_MANIFEST_DIR"));
    let e = load_state_layout(&fx("layout_unsupported_version.ron")).unwrap_err();
    assert!(
        matches!(
            e,
            WorldDataError::UnsupportedVersion {
                found: 99,
                supported: 1,
                ..
            }
        ),
        "{e}"
    );
    // 版以外のフィールドが現行形式と合わなくても、版の不一致が先に明確に報告される
    let e = load_province_table(&fx("table_unsupported_version.ron")).unwrap_err();
    assert!(
        matches!(e, WorldDataError::UnsupportedVersion { found: 2, .. }),
        "{e}"
    );
    let e = load_state_layout(&fx("layout_malformed.ron")).unwrap_err();
    assert!(matches!(e, WorldDataError::Parse { .. }), "{e}");
    let e = load_state_layout(&fx("does_not_exist.ron")).unwrap_err();
    assert!(matches!(e, WorldDataError::Io { .. }));
    assert!(e.to_string().contains("does_not_exist.ron"));
}

#[test]
fn loading_and_failed_validation_never_modify_input_files() {
    let dir = temp_dir("readonly");
    let map = small_map();
    let table_path = dir.join("table.ron");
    let layout_path = dir.join("layout.ron");
    let bad_layout_path = dir.join("bad_layout.ron");
    std::fs::write(
        &table_path,
        ron::ser::to_string_pretty(&table_for(&map), ron::ser::PrettyConfig::default()).unwrap(),
    )
    .unwrap();
    std::fs::write(&layout_path, SMALL_LAYOUT).unwrap();
    std::fs::write(
        &bad_layout_path,
        "(format_version: 1, states: [(id: 1, name_key: \"x\", provinces: [0, 999])])",
    )
    .unwrap();
    let before: Vec<Vec<u8>> = [&table_path, &layout_path, &bad_layout_path]
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();

    // 正常な読み込み
    let t = load_province_table(table_path.to_str().unwrap()).unwrap();
    let l = load_state_layout(layout_path.to_str().unwrap()).unwrap();
    assert!(WorldModel::build(t.clone(), l).is_ok());
    // 検証に失敗する読み込み
    let bad = load_state_layout(bad_layout_path.to_str().unwrap()).unwrap();
    assert!(WorldModel::build(t, bad).is_err());

    let after: Vec<Vec<u8>> = [&table_path, &layout_path, &bad_layout_path]
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    assert_eq!(before, after, "input files must not change");
    // 地理JSONの原本(実行時アセット)も変更しない: 読み取りの前後でバイト列が同じ
    let asset = std::fs::read("assets/data/hex_atlas_map.json").unwrap();
    let _ = strategy_game::hexmap::data::load_from_path("assets/data/hex_atlas_map.json").unwrap();
    assert_eq!(
        asset,
        std::fs::read("assets/data/hex_atlas_map.json").unwrap()
    );
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------- 実地図(メモリ上の検証。正式な世界設定ではない)

fn real_map() -> HexMap {
    strategy_game::hexmap::data::load_from_path("assets/data/hex_atlas_map.json").unwrap()
}

#[test]
fn real_map_full_table_validates_authoring_but_not_game_start_until_assigned() {
    let map = real_map();
    let table = generate_initial_province_table(&map, Coverage::World);
    let land = map.cells().iter().filter(|c| !c.terrain.is_water()).count();
    assert_eq!(table.provinces.len(), land, "1 land hex = 1 province");
    assert_eq!(table.next_province_id as usize, land);

    // ステート無し: 制作途中なら通る(全て未割当の診断)。ゲーム開始用は失敗する。
    let empty = WorldModel::build(table.clone(), layout_with(vec![])).unwrap();
    let authoring = validate_against_geography(&empty, &map, ValidationMode::Authoring);
    assert!(
        authoring.is_ok(),
        "{:?}",
        authoring.errors().collect::<Vec<_>>()
    );
    assert!(
        authoring.world_coverage_verified,
        "table covers every land cell"
    );
    assert_eq!(authoring.unassigned_province_count, land);
    let start = validate_against_geography(&empty, &map, ValidationMode::GameStart);
    assert!(!start.is_ok());
    assert!(start.has(IssueCode::UnassignedProvince));

    // テスト専用: 全陸プロヴィンスを1つの検証用ステートへ入れれば、ゲーム開始用検証を通る
    // (国境・ステート境界の確定ではない。非連結は明示)。
    let mut all = state(1, &table.provinces.iter().map(|p| p.id).collect::<Vec<_>>());
    all.allow_disconnected = true;
    let model = WorldModel::build(table, layout_with(vec![all])).unwrap();
    let r = validate_against_geography(&model, &map, ValidationMode::GameStart);
    assert!(r.is_ok(), "{:?}", r.errors().take(3).collect::<Vec<_>>());
    assert!(r.world_coverage_verified);
    // 連結成分が複数(島)でもエラーにしない
    assert!(
        r.issues
            .iter()
            .all(|i| i.code != IssueCode::StateDisconnected)
    );
}

#[test]
fn real_map_islands_are_warnings_when_not_acknowledged() {
    let map = real_map();
    let table = generate_initial_province_table(&map, Coverage::World);
    let all = state(1, &table.provinces.iter().map(|p| p.id).collect::<Vec<_>>());
    let model = WorldModel::build(table, layout_with(vec![all])).unwrap();
    let r = validate_against_geography(&model, &map, ValidationMode::GameStart);
    assert!(r.is_ok(), "islands must not make validation fail");
    let w = r
        .issues
        .iter()
        .find(|i| i.code == IssueCode::StateDisconnected)
        .expect("land has many islands");
    assert_eq!(w.severity, Severity::Warning);
}

#[test]
fn real_map_ids_survive_reordering_and_attribute_edits() {
    let map = real_map();
    let table = generate_initial_province_table(&map, Coverage::World);
    let model = WorldModel::build(table.clone(), layout_with(vec![])).unwrap();

    // JSONのセル/河川の並びを逆順にして再出力した想定
    let mut v: serde_json::Value =
        serde_json::from_slice(&std::fs::read("assets/data/hex_atlas_map.json").unwrap()).unwrap();
    v["cells"].as_array_mut().unwrap().reverse();
    v["rivers"].as_array_mut().unwrap().reverse();
    let reordered = parse_hex_map(serde_json::to_string(&v).unwrap().as_bytes()).unwrap();
    assert_eq!(compute_fingerprint(&map), compute_fingerprint(&reordered));
    assert!(validate_against_geography(&model, &reordered, ValidationMode::Authoring).is_ok());
    // 並び替え後のJSONから再生成しても同じID表
    assert_eq!(
        generate_initial_province_table(&reordered, Coverage::World).provinces,
        table.provinces
    );

    // 陸セル1つの気候・植生を変えた想定: IDは不変、検出は属性のみ
    let first_land = v["cells"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["terrain"] == "plains")
        .unwrap();
    let old = v["cells"][first_land]["climate"]
        .as_str()
        .unwrap()
        .to_string();
    let new_climate = if old == "A" { "B" } else { "A" };
    v["cells"][first_land]["climate"] = serde_json::json!(new_climate);
    let edited = parse_hex_map(serde_json::to_string(&v).unwrap().as_bytes()).unwrap();
    let ch = model.geography().compare(&compute_fingerprint(&edited));
    assert!(ch.attributes && !ch.land_set && !ch.coordinates && !ch.grid);
    let r = validate_against_geography(&model, &edited, ValidationMode::Authoring);
    assert!(r.is_ok());
    assert!(r.has(IssueCode::AttributesChanged));
}

#[test]
fn real_map_land_water_flip_is_detected() {
    let map = real_map();
    let table = generate_initial_province_table(&map, Coverage::World);
    let model = WorldModel::build(table, layout_with(vec![])).unwrap();
    let mut v: serde_json::Value =
        serde_json::from_slice(&std::fs::read("assets/data/hex_atlas_map.json").unwrap()).unwrap();
    let i = v["cells"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["terrain"] == "plains")
        .unwrap();
    v["cells"][i]["terrain"] = serde_json::json!("lake");
    let flipped = parse_hex_map(serde_json::to_string(&v).unwrap().as_bytes()).unwrap();
    let ch = model.geography().compare(&compute_fingerprint(&flipped));
    assert!(
        !ch.coordinates && ch.land_set,
        "same coordinate set, different land set"
    );
    let r = validate_against_geography(&model, &flipped, ValidationMode::Authoring);
    assert!(!r.is_ok());
    assert!(r.has(IssueCode::ProvinceCellIsWater));
}
