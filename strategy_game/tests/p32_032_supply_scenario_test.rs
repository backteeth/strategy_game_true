//! P32-032 (M5-c): 補給連結性の検証用シナリオ(`world::supply_scenario`)の読込・検証・展開・既存の到達判定/ステート要約への接続。
//!
//! 実データ(検証地域: 29プロヴィンス・3ステート)の期待値は、既存の P32-030/031 の処理を**個別指定**で呼んだ結果と、
//! テスト内の独立した総当たりで確認する。時間は合否条件にしない。保存・アセットは書かない(読むだけ)。

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::json;
use strategy_game::hexmap::data::{HexMap, load_from_path, parse_hex_map};
use strategy_game::world::fingerprint::FINGERPRINT_ALGORITHM;
use strategy_game::world::state_reach::kinds;
use strategy_game::world::*;

const SCENARIO_PATH: &str = "assets/data/world/preview/supply_connectivity_scenario.ron";

const STATE_20: [u32; 7] = [18109, 18308, 18309, 18310, 18311, 18499, 18500];
const STATE_22: [u32; 6] = [17266, 17267, 17268, 17484, 17485, 17693];
const UNASSIGNED: [u32; 3] = [17903, 18110, 18501];

fn real() -> (HexMap, WorldPreviewData, ProvinceGraph) {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let w = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).unwrap();
    let g = ProvinceGraph::build(&w.model, &map).unwrap();
    (map, w, g)
}

fn s(id: u32) -> MapStateId {
    MapStateId(id)
}

fn ids(v: &[ProvinceId]) -> Vec<u32> {
    v.iter().map(|x| x.0).collect()
}

/// テキストを解決して、問題の種類(code)を返す(成功なら空)。
fn codes(text: &str, g: &ProvinceGraph, w: &WorldModel) -> Vec<&'static str> {
    let file = parse_supply_scenario(text).expect("parses");
    match resolve_supply_scenario(&file, g, w) {
        Ok(_) => vec![],
        Err(v) => v.iter().map(|i| i.code()).collect(),
    }
}

fn one(agent: &str) -> String {
    format!("(format_version: 1, agents: [{agent}])")
}

// ---------------------------------------------------------------- 形式(読込の失敗の区別)

#[test]
fn format_failures_are_distinguished_from_content_issues() {
    // 非対応の版
    assert_eq!(
        parse_supply_scenario("(format_version: 2, agents: [])").unwrap_err(),
        SupplyScenarioError::UnsupportedVersion { found: 2, supported: 1 }
    );
    // RON として不正
    assert!(matches!(parse_supply_scenario("(format_version: 1, agents: [ (label: 1,"), Err(SupplyScenarioError::Parse(_))));
    // 版が無い
    assert!(matches!(parse_supply_scenario("(agents: [])"), Err(SupplyScenarioError::Parse(_))));
    // 未知のフィールド(主体・トップレベル)は拒否
    assert!(matches!(parse_supply_scenario(&one("(label: 1, sources: [1], passable_states: [20], owner: 3)")), Err(SupplyScenarioError::Parse(_))));
    assert!(matches!(parse_supply_scenario("(format_version: 1, agents: [], extra: 1)"), Err(SupplyScenarioError::Parse(_))));
    // 型違い(ProvinceId に負数・文字列)
    assert!(matches!(parse_supply_scenario(&one("(label: 1, sources: [-1], passable_states: [20])")), Err(SupplyScenarioError::Parse(_))));
    assert!(matches!(parse_supply_scenario(&one("(label: 1, sources: [\"a\"], passable_states: [20])")), Err(SupplyScenarioError::Parse(_))));
    // 読めないファイル
    let (_m, w, g) = real();
    assert!(matches!(load_supply_scenario("assets/data/world/preview/no_such_file.ron", &g, &w.model), Err(SupplyScenarioError::Io { .. })));
    // 内容の問題は Invalid(形式の失敗とは別)
    let tmp = std::env::temp_dir().join(format!("p32_032_bad_{}.ron", std::process::id()));
    std::fs::write(&tmp, one("(label: 1, sources: [1], passable_provinces: [1])")).unwrap();
    let e = load_supply_scenario(tmp.to_str().unwrap(), &g, &w.model).unwrap_err();
    std::fs::remove_file(&tmp).ok();
    assert!(matches!(e, SupplyScenarioError::Invalid(_)), "{e}");
}

// ---------------------------------------------------------------- 入力検証(種類ごと)

#[test]
fn every_kind_of_invalid_input_is_reported_with_its_own_code() {
    let (_m, w, g) = real();
    let m = &w.model;
    let c = |text: String| codes(&text, &g, m);
    // トップレベル
    assert_eq!(codes("(format_version: 1)", &g, m), vec!["missing-field"]);
    assert_eq!(codes("(format_version: 1, agents: [])", &g, m), vec!["no-agents"]);
    // 必須項目の欠落(label / sources / 通行範囲)
    assert_eq!(c(one("(sources: [18309], passable_states: [20])")), vec!["missing-field"]);
    assert_eq!(c(one("(label: 1, passable_states: [20])")), vec!["missing-field"]);
    assert_eq!(c(one("(label: 1, sources: [18309])")), vec!["missing-field"]);
    // 補給源
    assert_eq!(c(one("(label: 1, sources: [], passable_states: [20])")), vec!["no-sources"]);
    assert_eq!(c(one("(label: 1, sources: [18309, 18309], passable_states: [20])")), vec!["duplicate-source"]);
    assert_eq!(c(one("(label: 1, sources: [99999], passable_states: [20])")), vec!["unknown-province"]);
    // 通行範囲: 未知のプロヴィンス・重複・未知のステート・ステートの重複
    assert_eq!(c(one("(label: 1, sources: [18309], passable_provinces: [18309, 99999])")), vec!["unknown-province"]);
    assert_eq!(c(one("(label: 1, sources: [18309], passable_provinces: [18309, 18309])")), vec!["duplicate-passable-province"]);
    assert_eq!(c(one("(label: 1, sources: [18309], passable_states: [20, 99])")), vec!["unknown-state"]);
    assert_eq!(c(one("(label: 1, sources: [18309], passable_states: [20, 20])")), vec!["duplicate-passable-state"]);
    // 補給源が通行範囲外 / 通行範囲が空
    assert_eq!(c(one("(label: 1, sources: [18309], passable_states: [22])")), vec!["source-not-passable"]);
    assert_eq!(c(one("(label: 1, sources: [18309], passable_provinces: [], passable_states: [])")), vec!["empty-passable"]);
    // ラベルの重複
    assert_eq!(
        c("(format_version: 1, agents: [(label: 1, sources: [18309], passable_states: [20]), (label: 1, sources: [18309], passable_states: [20])])".into()),
        vec!["duplicate-label"]
    );
    // 複数の問題はすべて集める(最初の1件で止めない)
    let all = c(one("(label: 1, sources: [99999, 99999], passable_provinces: [88888], passable_states: [77])"));
    assert_eq!(all, vec!["unknown-province", "duplicate-source", "unknown-province", "unknown-state"]);
    // 水域・範囲外の陸は ProvinceId が無いので名指しできない: 検証地域の外の番号は「未知のプロヴィンス」
    let unknown_id = w.model.next_province_id() + 1000;
    assert_eq!(c(one(&format!("(label: 1, sources: [18309], passable_states: [20], passable_provinces: [{unknown_id}])"))), vec!["unknown-province"]);
}

#[test]
fn a_retired_id_is_reported_as_retired() {
    let map = map_from(&["###"]);
    let mut t = table(&map, &[(10, 0, 0), (11, 1, 0)], Coverage::Partial);
    t.next_province_id = 13;
    t.retired = vec![12];
    let w = WorldModel::build(t, layout(&[(1, &[10, 11])])).unwrap();
    let g = ProvinceGraph::build(&w, &map).unwrap();
    let file = parse_supply_scenario(&one("(label: 1, sources: [10], passable_provinces: [10, 12, 99])")).unwrap();
    let issues = resolve_supply_scenario(&file, &g, &w).unwrap_err();
    let flags: Vec<(u32, bool)> = issues
        .iter()
        .filter_map(|i| match i {
            SupplyScenarioIssue::UnknownProvince { province, retired, .. } => Some((province.0, *retired)),
            _ => None,
        })
        .collect();
    assert_eq!(flags, vec![(12, true), (99, false)]);
}

// ---------------------------------------------------------------- 展開・併用・未所属

#[test]
fn passable_states_expand_through_the_existing_membership_and_explicit_passable() {
    let (_m, w, g) = real();
    let file = parse_supply_scenario(&one("(label: 1, sources: [18309], passable_states: [20, 22])")).unwrap();
    let sc = resolve_supply_scenario(&file, &g, &w.model).unwrap();
    let a = &sc.agents[0];
    // 所属の正本(WorldModel)と、既存の ExplicitPassable::from_states の結果に一致
    let expected = ExplicitPassable::from_states(&g, &[s(20), s(22)]).unwrap();
    assert_eq!(a.passable, expected);
    let mut members: Vec<u32> = ids(w.model.provinces_of(s(20)).unwrap());
    members.extend(ids(w.model.provinces_of(s(22)).unwrap()));
    members.sort();
    assert_eq!(a.passable.ids().map(|p| p.0).collect::<Vec<_>>(), members);
    assert_eq!(a.passable.len(), 13);
    assert_eq!((a.states.clone(), a.explicit_provinces.len()), (vec![s(20), s(22)], 0));
    // 経済シナリオの担当ステート(states: [20] など)・配列順から推定していない: 指定していないステート21は通れない
    use strategy_game::world::Passability;
    assert!(!a.passable.passable(ProvinceId(17269)));
}

#[test]
fn states_and_provinces_combine_as_a_union_and_overlap_after_expansion_is_an_error() {
    let (_m, w, g) = real();
    // 併用: ステート20・22 + 未所属3件(ステートに属さない既知の ID を明示すれば通行できる)
    let ok = parse_supply_scenario(&one("(label: 1, sources: [18309], passable_states: [20, 22], passable_provinces: [17903, 18110, 18501])")).unwrap();
    let sc = resolve_supply_scenario(&ok, &g, &w.model).unwrap();
    assert_eq!(sc.agents[0].passable.len(), 16);
    for p in UNASSIGNED {
        assert_eq!(w.model.state_of(ProvinceId(p)), None, "unassigned in the layout");
        use strategy_game::world::Passability;
        assert!(sc.agents[0].passable.passable(ProvinceId(p)));
    }
    // 展開後の重複: 明示した 18309 は、挙げたステート20の所属でもある
    assert_eq!(
        codes(&one("(label: 1, sources: [18309], passable_states: [20], passable_provinces: [18309])"), &g, &w.model),
        vec!["overlap-province-in-state"]
    );
    // 別のステート(21)の所属を明示するのは重複ではない(ステート指定の展開に含まれない)
    assert!(codes(&one("(label: 1, sources: [18309], passable_states: [20], passable_provinces: [17269])"), &g, &w.model).is_empty());
}

// ---------------------------------------------------------------- 実データ A / B(同梱のRONを、既存の個別指定と照合)

fn outcome_of(label: u32) -> (SupplyScenario, Vec<AgentOutcome>, WorldPreviewData, ProvinceGraph) {
    let (_m, w, g) = real();
    let sc = load_supply_scenario(SCENARIO_PATH, &g, &w.model).unwrap();
    let out = evaluate_supply_scenario(&g, &w.model, &sc).unwrap();
    assert!(sc.agents.iter().any(|a| a.label == label));
    (sc, out, w, g)
}

#[test]
fn the_shipped_scenario_reproduces_conditions_a_and_b_and_matches_the_individual_options() {
    let (sc, out, w, g) = outcome_of(1);
    assert_eq!(sc.agents.iter().map(|a| a.label).collect::<Vec<_>>(), vec![1, 2, 3], "label order");
    let by = |l: u32| out.iter().find(|o| o.label == l).unwrap();
    // A
    let a = by(1);
    assert_eq!(ids(&a.reach.reached_ids()), STATE_20.to_vec());
    assert_eq!(ids(&a.reach.unknown_ids()), STATE_22.to_vec());
    let k = kinds(&a.summary);
    assert_eq!((k[&s(20)], k[&s(21)], k[&s(22)]), (StateReachKind::Full, StateReachKind::Unreachable, StateReachKind::Unknown));
    // B
    let b = by(2);
    let mut expected_b: Vec<u32> = STATE_20.iter().chain(STATE_22.iter()).chain(UNASSIGNED.iter()).copied().collect();
    expected_b.sort();
    assert_eq!(ids(&b.reach.reached_ids()), expected_b);
    assert_eq!(b.reach.unknown_count(), 0);
    let k = kinds(&b.summary);
    assert_eq!((k[&s(20)], k[&s(21)], k[&s(22)]), (StateReachKind::Full, StateReachKind::Unreachable, StateReachKind::Full));
    assert_eq!(b.summary.unassigned().reachable, 3);
    // 既存の個別指定(P32-030 の ExplicitPassable + reach、P32-031 の要約)と一致
    let pa = ExplicitPassable::from_states(&g, &[s(20), s(22)]).unwrap();
    let ra = reach(&g, &[ProvinceId(18309)], &pa).unwrap();
    assert_eq!(a.reach, ra);
    assert_eq!(a.summary, summarize_by_state(&g, &ra).unwrap());
    let pb = pa.union(&ExplicitPassable::new(&g, UNASSIGNED.iter().map(|x| ProvinceId(*x))).unwrap());
    let rb = reach(&g, &[ProvinceId(18309)], &pb).unwrap();
    assert_eq!(b.reach, rb);
    assert_eq!(b.summary, summarize_by_state(&g, &rb).unwrap());
    assert_eq!(b.summary, summarize_model(&w.model, &rb).unwrap());
}

#[test]
fn multiple_sources_reach_each_islands_and_all_sources_must_be_passable() {
    let (_sc, out, w, g) = outcome_of(3);
    let c = out.iter().find(|o| o.label == 3).unwrap();
    // 18309(ステート20側)と 17266(ステート22側): 22 の6件は2つ目の補給源から既知の辺で着くので、不明にならない
    let mut expected: Vec<u32> = STATE_20.iter().chain(STATE_22.iter()).copied().collect();
    expected.sort();
    assert_eq!(ids(&c.reach.reached_ids()), expected);
    assert_eq!(c.reach.unknown_count(), 0);
    assert_eq!(c.reach.sources().iter().map(|p| p.0).collect::<Vec<_>>(), vec![17266, 18309]);
    let k = kinds(&c.summary);
    assert_eq!((k[&s(20)], k[&s(22)], k[&s(21)]), (StateReachKind::Full, StateReachKind::Full, StateReachKind::Unreachable));
    // 独立した総当たり: 13件の通行範囲を、両方の補給源から既知の辺だけでたどる
    let allowed: HashSet<u32> = expected.iter().copied().collect();
    assert_eq!(brute_known_reach(&w, &allowed, &[18309, 17266]).into_iter().collect::<Vec<_>>(), expected);
    // 補給源の1つでも通行範囲に無ければ、解決の段階で拒否(探索に進まない)
    assert_eq!(codes(&one("(label: 1, sources: [18309, 17266], passable_states: [20])"), &g, &w.model), vec!["source-not-passable"]);
}

/// テスト内の独立した総当たり(既知の辺だけ。範囲外の陸は通らない)。
fn brute_known_reach(w: &WorldPreviewData, allowed: &HashSet<u32>, sources: &[u32]) -> BTreeSet<u32> {
    const OFFSETS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
    let cell_of: HashMap<(i32, i32), u32> = w.model.provinces().iter().map(|d| ((d.cell.q, d.cell.r), d.id.0)).collect();
    let mut seen = HashSet::new();
    let mut stack: Vec<(i32, i32)> = sources.iter().map(|s| *cell_of.iter().find(|(_, id)| **id == *s).unwrap().0).collect();
    let mut out = BTreeSet::new();
    while let Some(c) = stack.pop() {
        if !seen.insert(c) {
            continue;
        }
        out.insert(cell_of[&c]);
        for (dq, dr) in OFFSETS {
            let n = (c.0 + dq, c.1 + dr);
            if let Some(id) = cell_of.get(&n)
                && allowed.contains(id)
                && !seen.contains(&n)
            {
                stack.push(n);
            }
        }
    }
    out
}

// ---------------------------------------------------------------- 入力順・通行条件の変更

#[test]
fn results_do_not_depend_on_the_order_of_agents_sources_or_ids() {
    let (_m, w, g) = real();
    let a = "(format_version: 1, agents: [
        (label: 2, sources: [18309, 17266], passable_provinces: [17903, 18110, 18501], passable_states: [20, 22]),
        (label: 1, sources: [18309], passable_states: [20, 22]),
    ])";
    let b = "(format_version: 1, agents: [
        (label: 1, passable_states: [22, 20], sources: [18309]),
        (passable_states: [22, 20], passable_provinces: [18501, 17903, 18110], sources: [17266, 18309], label: 2),
    ])";
    let eval = |t: &str| {
        let sc = resolve_supply_scenario(&parse_supply_scenario(t).unwrap(), &g, &w.model).unwrap();
        let out = evaluate_supply_scenario(&g, &w.model, &sc).unwrap();
        (sc, out)
    };
    let (sa, oa) = eval(a);
    let (sb, ob) = eval(b);
    assert_eq!(sa, sb);
    assert_eq!(oa, ob);
    assert_eq!(oa.iter().map(|o| o.label).collect::<Vec<_>>(), vec![1, 2], "label order");
}

#[test]
fn changing_the_passable_range_changes_the_summary() {
    let (_m, w, g) = real();
    let run = |text: &str| {
        let sc = resolve_supply_scenario(&parse_supply_scenario(text).unwrap(), &g, &w.model).unwrap();
        kinds(&evaluate_supply_scenario(&g, &w.model, &sc).unwrap()[0].summary)
    };
    let without = run(&one("(label: 1, sources: [18309], passable_states: [20, 22])"));
    let with = run(&one("(label: 1, sources: [18309], passable_states: [20, 22], passable_provinces: [17903, 18110, 18501])"));
    assert_eq!(without[&s(22)], StateReachKind::Unknown);
    assert_eq!(with[&s(22)], StateReachKind::Full);
    // ステート20だけ: 22 は通行不可(到達不能・確定)
    let only20 = run(&one("(label: 1, sources: [18309], passable_states: [20])"));
    assert_eq!((only20[&s(20)], only20[&s(22)]), (StateReachKind::Full, StateReachKind::Unreachable));
}

#[test]
fn a_province_range_without_states_works_and_an_unassigned_source_is_allowed() {
    let (_m, w, g) = real();
    // ステート未所属の 17903 を補給源にし、通行範囲もプロヴィンスだけで明示
    let sc = resolve_supply_scenario(
        &parse_supply_scenario(&one("(label: 1, sources: [17903], passable_provinces: [17903, 18309, 18110])")).unwrap(),
        &g,
        &w.model,
    )
    .unwrap();
    let out = evaluate_supply_scenario(&g, &w.model, &sc).unwrap();
    let reached = ids(&out[0].reach.reached_ids());
    assert!(reached.contains(&17903));
    assert!(out[0].summary.assigned_totals().total + out[0].summary.unassigned().total == 29);
    assert_eq!(out[0].summary.unassigned().total, 3);
}

// ---------------------------------------------------------------- 範囲外の陸・隔離

#[test]
fn out_of_scope_land_is_never_named_or_bridged_by_the_scenario_layer() {
    let (map, w, g) = real();
    let known: HashSet<(i32, i32)> = w.model.provinces().iter().map(|d| (d.cell.q, d.cell.r)).collect();
    let oos = map.cells().iter().find(|c| !c.terrain.is_water() && !known.contains(&(c.q, c.r))).unwrap();
    // 範囲外の陸セルには ProvinceId が無い(グラフのノードは29のまま。評価しても増えない)
    let sc = load_supply_scenario(SCENARIO_PATH, &g, &w.model).unwrap();
    let out = evaluate_supply_scenario(&g, &w.model, &sc).unwrap();
    assert_eq!(g.node_count(), 29);
    for o in &out {
        assert_eq!(o.summary.grand_totals().total, 29);
        assert_eq!(
            o.reach.reachability_of_cell(&g, &map, HexCoord::new(oos.q, oos.r)),
            Reachability::NotApplicable(NotApplicableReason::OutOfScopeLand)
        );
    }
}

#[test]
fn the_scenario_layer_stays_separate_from_economy_saving_and_the_military() {
    let src = std::fs::read_to_string("src/world/supply_scenario.rs").unwrap();
    let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    for banned in [
        "MapEconomy", "economy::", "EconomyScenario", "ScenarioCountry", "snapshot", "SaveLocation", "military::", "Division", "StateRegistry", "std::time", "Instant",
        "rand::", "fs::write", "File::create",
    ] {
        assert!(!code.contains(banned), "supply_scenario.rs must stay separate: found {banned}");
    }
    // 探索・要約の処理を持たない(既存の reach / summarize_model を呼ぶだけ)
    for banned in ["VecDeque", "BinaryHeap", "components_among"] {
        assert!(!code.contains(banned), "no search code here: found {banned}");
    }
    assert!(code.contains("reach(graph") && code.contains("summarize_model("));
}

// ---------------------------------------------------------------- 人工地図の部品

fn map_from(rows: &[&str]) -> HexMap {
    let (h, w) = (rows.len(), rows.iter().map(|r| r.len()).max().unwrap());
    let mut cells = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (q, ch) in row.chars().enumerate() {
            let (terrain, climate, veg) = match ch {
                '.' => ("sea", "unclassified", "unclassified"),
                _ => ("plains", "C", "grassland"),
            };
            cells.push(json!({ "q": q, "r": r, "terrain": terrain, "climate": climate, "vegetation": veg }));
        }
    }
    let v = json!({
        "schemaVersion": 9,
        "kind": "hex-atlas-game-map",
        "grid": {
            "orientation": "pointy", "size": 2.5, "origin": { "x": 2.1650635094610964, "y": 2.5 },
            "imageWidth": 100, "imageHeight": 60,
            "extent": { "minQ": 0, "maxQ": w - 1, "minR": 0, "maxR": h - 1 }
        },
        "cells": cells,
        "rivers": [],
    });
    parse_hex_map(v.to_string().as_bytes()).expect("synthetic map must parse")
}

fn table(map: &HexMap, provinces: &[(u32, i32, i32)], coverage: Coverage) -> ProvinceTableFile {
    let mut geography = compute_fingerprint(map);
    geography.algorithm = FINGERPRINT_ALGORITHM;
    ProvinceTableFile {
        format_version: WORLD_FORMAT_VERSION,
        coverage,
        geography,
        next_province_id: provinces.iter().map(|p| p.0).max().unwrap_or(0) + 1,
        retired: Vec::new(),
        provinces: provinces.iter().map(|&(id, q, r)| ProvinceEntry { id, q, r }).collect(),
    }
}

fn layout(states: &[(u32, &[u32])]) -> StateLayoutFile {
    StateLayoutFile {
        format_version: WORLD_FORMAT_VERSION,
        states: states
            .iter()
            .map(|(id, ps)| StateEntry { id: *id, name_key: format!("test.{id}"), provinces: ps.to_vec(), representative: None, allow_disconnected: true })
            .collect(),
    }
}
