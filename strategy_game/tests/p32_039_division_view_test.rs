//! P32-039 (M6-f): 新マップの師団の表示と選択(`hexmap::division_view`)のテスト。
//!
//! 純粋な `DivisionSession` と、実際の Bevy `App`(`MinimalPlugins` + `DivisionViewCorePlugin`)で、コマンドの受付と適用・ボタン(`Interaction`)・キー(`ButtonInput<KeyCode>`)からコマンドへの接続を通す。
//! 実ウィンドウの描画・実マウス/キーボードの操作は含まない(GUI確認は `verification_logs/phase-32/p32-039/` の別記録)。保存・アセットは書かない。

use std::collections::BTreeSet;

use bevy::prelude::*;
use strategy_game::common::CountryId;
use strategy_game::hexmap::data::{HexMap, load_from_path};
use strategy_game::hexmap::division_view::*;
use strategy_game::hexmap::view::{HexSelection, LoadedHexMap};
use strategy_game::hexmap::world_view::{LoadedWorld, catalog_names};
use strategy_game::world::*;

const MOVE: &str = "assets/data/world/preview/division_movement_scenario.ron";
const BATTLE: &str = "assets/data/world/preview/division_battle_scenario.ron";

fn p(id: u32) -> ProvinceId {
    ProvinceId(id)
}

fn md(id: u32) -> MapDivisionId {
    MapDivisionId(id)
}

struct Real {
    map: HexMap,
    data: WorldPreviewData,
    graph: ProvinceGraph,
}

fn real() -> Real {
    let map = load_from_path("assets/data/hex_atlas_map.json").unwrap();
    let data = load_world_preview("assets/data/world/preview/provinces.ron", "assets/data/world/preview/states.ron", &map).unwrap();
    let graph = ProvinceGraph::build(&data.model, &map).unwrap();
    Real { map, data, graph }
}

fn session(r: &Real, path: &str) -> DivisionSession {
    DivisionSession::load(path, r.graph.clone(), &r.data.model).unwrap()
}

fn run_days(s: &mut DivisionSession, n: u32) -> Vec<MoveEvent> {
    let mut all = Vec::new();
    for _ in 0..n {
        all.extend(s.advance_one_day().events);
    }
    all
}

/// コアプラグインだけのヘッドレス App(地図・ワールド・セッションを入れる)。
fn app_for(path: &str) -> App {
    let r = real();
    let s = session(&r, path);
    let names = catalog_names(&r.data);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(DivisionViewCorePlugin);
    app.init_resource::<ButtonInput<KeyCode>>();
    app.add_systems(Update, (division_buttons, division_keys).in_set(DivisionSet::Input));
    app.insert_resource(DivisionSessionRes(s));
    app.insert_resource(LoadedWorld { data: r.data, names });
    app.insert_resource(LoadedHexMap(r.map));
    app.insert_resource(HexSelection::default());
    app.update();
    app
}

fn sess(app: &App) -> &DivisionSession {
    &app.world().resource::<DivisionSessionRes>().0
}

fn send(app: &mut App, c: DivisionCommand) {
    app.world_mut().resource_mut::<Messages<DivisionCommand>>().write(c);
}

fn select_province_cell(app: &mut App, prov: u32) {
    let c = app.world().resource::<LoadedWorld>().data.model.province(p(prov)).unwrap().cell;
    let idx = app.world().resource::<LoadedHexMap>().0.cell_index(c.q, c.r);
    app.world_mut().resource_mut::<HexSelection>().0 = idx;
}

// ---------------------------------------------------------------- 起動・表示位置

#[test]
fn the_view_starts_stopped_with_the_initial_placement_and_no_selection() {
    let r = real();
    let s = session(&r, MOVE);
    assert_eq!(s.day(), 0);
    assert_eq!(s.selected(), None);
    assert_eq!(s.military().divisions().count(), 3);
    let m = s.markers();
    let placed: Vec<(u32, Vec<u32>)> = m.iter().map(|x| (x.province.0, x.divisions.iter().map(|d| d.0).collect())).collect();
    assert_eq!(placed, vec![(17266, vec![2]), (17269, vec![3]), (18309, vec![1])]);
    assert!(s.header_text().contains("初期配置(未処理)"));
}

#[test]
fn the_marker_cell_corresponds_to_the_division_province() {
    let r = real();
    let s = session(&r, MOVE);
    for m in s.markers() {
        // 1 プロヴィンス = 1 セルのプレビュー世界: 表示セルはプロヴィンスの所属セルそのもの
        let cell = r.data.model.province(m.province).unwrap().cell;
        assert_eq!(m.cell, cell);
        assert_eq!(display_cell(s.graph(), m.province), Some(cell));
    }
    // 現在地と一致する(師団 → ProvinceId → 表示セル)
    for d in s.military().divisions() {
        let marker = s.markers().into_iter().find(|m| m.divisions.contains(&d.id)).unwrap();
        assert_eq!(marker.province, d.province);
    }
}

#[test]
fn a_multi_cell_province_gets_a_stable_display_cell_independent_of_the_input_order() {
    let r = real();
    let cells: Vec<HexCoord> = [18308u32, 18309, 18310].iter().map(|id| r.data.model.province(p(*id)).unwrap().cell).collect();
    let make = |order: &[usize]| {
        let defs = vec![ProvinceCells { id: p(1), state: None, cells: order.iter().map(|i| cells[*i]).collect() }];
        ProvinceGraph::from_definitions(&defs, &r.map, Coverage::World)
    };
    let a = make(&[0, 1, 2]);
    let b = make(&[2, 0, 1]);
    let (Ok(a), Ok(b)) = (a, b) else {
        // 3 セルが連結でなければ、隣り合う 2 セルで確認する(同距離の同点 → (q, r) が小さいほう)
        let two = |order: &[usize]| {
            let defs = vec![ProvinceCells { id: p(1), state: None, cells: order.iter().map(|i| cells[*i]).collect() }];
            ProvinceGraph::from_definitions(&defs, &r.map, Coverage::World).unwrap()
        };
        let (g1, g2) = (two(&[0, 1]), two(&[1, 0]));
        let c1 = display_cell(&g1, p(1)).unwrap();
        assert_eq!(c1, display_cell(&g2, p(1)).unwrap());
        let min = cells[..2].iter().min_by_key(|c| (c.q, c.r)).copied().unwrap();
        assert_eq!(c1, min, "a tie is broken by the smaller (q, r)");
        return;
    };
    let ca = display_cell(&a, p(1)).unwrap();
    assert_eq!(ca, display_cell(&b, p(1)).unwrap(), "the same cell whatever the input order");
    assert!(cells.contains(&ca), "the display cell belongs to the province");
    assert_eq!(display_cell(&a, p(99)), None, "an unknown province has no display cell");
}

// ---------------------------------------------------------------- 複数師団の選択

#[test]
fn several_divisions_in_one_province_show_one_marker_with_a_count_and_can_be_picked_in_id_order() {
    let r = real();
    let mut s = session(&r, BATTLE);
    // 18311 に師団 1 と 4
    let m = s.markers().into_iter().find(|m| m.province == p(18311)).unwrap();
    assert_eq!(m.divisions, vec![md(1), md(4)]);
    // 選択欄(ID 順)から目的の師団を選べる
    s.select(md(4)).unwrap();
    assert_eq!(s.selected(), Some(md(4)));
    assert!(s.detail_text(&r.data.model).contains("MapDivision#4"));
    assert!(s.detail_text(&r.data.model).contains("同じプロヴィンスの師団: 1, 4"));
    s.select(md(1)).unwrap();
    assert_eq!(s.selected(), Some(md(1)));
    // 選択セルの説明は、両方の師団を ID 順に出す
    let t = s.cell_divisions_text(Some(p(18311)));
    assert!(t.starts_with("選択セルの師団 2 件: 1(") && t.contains(" / 4("), "{t}");
    assert_eq!(s.cell_divisions_text(Some(p(18309))), "", "no divisions there");
    // 前後の選択は MapDivisionId 順に巡回する
    let ids: Vec<u32> = s.military().divisions().map(|d| d.id.0).collect();
    let mut s2 = session(&r, BATTLE);
    s2.cycle(true);
    assert_eq!(s2.selected().unwrap().0, ids[0]);
    s2.cycle(false);
    assert_eq!(s2.selected().unwrap().0, *ids.last().unwrap(), "wraps around backwards");
    s2.cycle(true);
    assert_eq!(s2.selected().unwrap().0, ids[0]);
    // 存在しない師団は拒否して選択を変えない
    assert!(s2.select(md(9999)).is_err());
    assert_eq!(s2.selected().unwrap().0, ids[0]);
}

// ---------------------------------------------------------------- 日送り・移動(Bevy App)

#[test]
fn rendering_updates_selection_changes_and_view_changes_never_advance_the_day() {
    let mut app = app_for(MOVE);
    let before = sess(&app).clone();
    for i in 0..10 {
        select_province_cell(&mut app, if i % 2 == 0 { 18308 } else { 17266 });
        app.update();
    }
    assert_eq!(sess(&app).day(), 0);
    assert_eq!(sess(&app).military(), before.military(), "no command, nothing advances");
}

#[test]
fn one_button_press_advances_exactly_one_day_even_if_held() {
    let mut app = app_for(MOVE);
    let e = app.world_mut().spawn((Interaction::None, DivisionButton(DivisionButtonKind::Advance))).id();
    app.update();
    assert_eq!(sess(&app).day(), 0);
    *app.world_mut().get_mut::<Interaction>(e).unwrap() = Interaction::Pressed;
    app.update();
    assert_eq!(sess(&app).day(), 1, "the press is one day");
    for _ in 0..5 {
        app.update(); // 押し続けても Changed は発火しない
    }
    assert_eq!(sess(&app).day(), 1);
    *app.world_mut().get_mut::<Interaction>(e).unwrap() = Interaction::Hovered;
    app.update();
    assert_eq!(sess(&app).day(), 1, "releasing does not advance");
    *app.world_mut().get_mut::<Interaction>(e).unwrap() = Interaction::Pressed;
    app.update();
    assert_eq!(sess(&app).day(), 2);
}

#[test]
fn the_advance_key_is_one_day_per_press() {
    let mut app = app_for(MOVE);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Space);
    app.update();
    assert_eq!(sess(&app).day(), 1);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
    app.update();
    assert_eq!(sess(&app).day(), 1, "held key does not repeat");
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::Space);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyN);
    app.update();
    assert_eq!(sess(&app).day(), 2);
}

#[test]
fn the_movement_scenario_division_stays_until_day_5_then_the_marker_moves() {
    let mut app = app_for(MOVE);
    send(&mut app, DivisionCommand::Select(md(1)));
    select_province_cell(&mut app, 18308);
    send(&mut app, DivisionCommand::OrderToSelectedCell);
    app.update();
    assert_eq!(sess(&app).selected(), Some(md(1)));
    assert_eq!(sess(&app).military().division(md(1)).unwrap().destination, Some(p(18308)));
    let province_of_marker = |s: &DivisionSession| s.markers().into_iter().find(|m| m.divisions.contains(&md(1))).unwrap().province;
    for day in 1..=4 {
        send(&mut app, DivisionCommand::AdvanceDay);
        app.update();
        assert_eq!(sess(&app).day(), day);
        assert_eq!(sess(&app).military().division(md(1)).unwrap().province, p(18309), "day {day}");
        assert_eq!(province_of_marker(sess(&app)), p(18309));
    }
    send(&mut app, DivisionCommand::AdvanceDay);
    app.update();
    assert_eq!(sess(&app).day(), 5);
    assert_eq!(sess(&app).military().division(md(1)).unwrap().province, p(18308));
    assert_eq!(province_of_marker(sess(&app)), p(18308));
    // 到着後も選択は維持される
    assert_eq!(sess(&app).selected(), Some(md(1)));
    assert!(sess(&app).detail_text(&app.world().resource::<LoadedWorld>().data.model).contains("待機"));
}

#[test]
fn several_commands_in_one_frame_are_applied_in_arrival_order_one_day_each() {
    let mut app = app_for(MOVE);
    send(&mut app, DivisionCommand::AdvanceDay);
    send(&mut app, DivisionCommand::AdvanceDay);
    send(&mut app, DivisionCommand::AdvanceDay);
    app.update();
    assert_eq!(sess(&app).day(), 3);
}

// ---------------------------------------------------------------- 命令の拒否

#[test]
fn a_rejected_order_gives_the_reason_and_keeps_the_existing_path_and_progress() {
    let r = real();
    let mut s = session(&r, BATTLE);
    // 師団1(国家1)を 18310 へ向かわせ、途中まで進める
    s.select(md(1)).unwrap();
    s.order_to(p(18310)).unwrap();
    run_days(&mut s, 2);
    let before = s.military().division(md(1)).unwrap().clone();
    let rev = s.revision();
    assert_eq!(before.status, MapDivisionStatus::Moving);
    // 18312 は国家3の支配(国家1は交戦していない)→ 拒否
    let err = s.order_to(p(18312)).unwrap_err();
    assert!(err.contains("拒否") && err.contains("18312"), "{err}");
    assert_eq!(s.military().division(md(1)).unwrap(), &before, "the division is not touched");
    assert_eq!(s.revision(), rev, "nothing to redraw");
    let (ok, text) = s.log().last().unwrap();
    assert!(!*ok && text.contains("既存の経路・進捗は変わりません"), "{text}");
    // 地図にない ProvinceId・師団未選択も拒否
    assert!(s.order_to(p(1)).is_err());
    s.clear_selection();
    let e = s.order_to(p(18310)).unwrap_err();
    assert!(e.contains("選択されていない"), "{e}");
}

#[test]
fn ordering_without_a_selected_cell_or_to_water_is_refused_in_the_app() {
    let mut app = app_for(MOVE);
    send(&mut app, DivisionCommand::Select(md(1)));
    send(&mut app, DivisionCommand::OrderToSelectedCell); // セル未選択
    app.update();
    let (ok, text) = sess(&app).log().last().unwrap().clone();
    assert!(!ok && text.contains("移動先のセルが選ばれていません"), "{text}");
    assert_eq!(sess(&app).military().division(md(1)).unwrap().status, MapDivisionStatus::Idle);
    // 水域のセル
    let water = {
        let w = app.world().resource::<LoadedWorld>();
        let m = app.world().resource::<LoadedHexMap>();
        m.0.cells().iter().find(|c| c.terrain.is_water()).and_then(|c| m.0.cell_index(c.q, c.r)).map(|i| (i, w.data.model.provinces().len()))
    };
    app.world_mut().resource_mut::<HexSelection>().0 = water.map(|w| w.0);
    send(&mut app, DivisionCommand::OrderToSelectedCell);
    app.update();
    assert!(!sess(&app).log().last().unwrap().0);
    assert_eq!(sess(&app).military().division(md(1)).unwrap().status, MapDivisionStatus::Idle);
}

// ---------------------------------------------------------------- 戦闘・占領・退却・撃破

#[test]
fn battle_and_occupation_update_the_state_and_the_control_display() {
    let r = real();
    let mut s = session(&r, BATTLE);
    s.select(md(2)).unwrap();
    s.order_to(p(17694)).unwrap();
    let marks_before = s.control_marks(&r.data.model);
    assert!(marks_before.iter().all(|m| m.province != p(17694)));
    let ev = run_days(&mut s, 12);
    assert!(ev.iter().any(|e| matches!(e, MoveEvent::BattleStarted { province, .. } if *province == p(17694))));
    assert!(ev.iter().any(|e| matches!(e, MoveEvent::ControlChanged { province, .. } if *province == p(17694))));
    // 師団 2 は 17694 にいて、実効支配は国家1。法的所有(ステート21)は国家2のまま。別項目で出す。
    let t = s.detail_text(&r.data.model);
    assert!(t.contains("現在地: Province#17694 (MapState#21)"), "{t}");
    assert!(t.contains("法的所有国(ステート): 国家2") && t.contains("実効支配国(現在地): 国家1"), "{t}");
    // 支配の印: 17694 は法的所有(国家2)と違う国家1の支配
    let m = s.control_marks(&r.data.model).into_iter().find(|m| m.province == p(17694)).expect("a control mark");
    assert_eq!(m.controller, CountryId(1));
    assert_eq!(m.legal_owner, Some(CountryId(2)));
    assert!(s.control_text(&r.data.model).contains("MapState#21: 法的所有 国家2 / 実効支配"), "{}", s.control_text(&r.data.model));
    // 戦闘中の表示(場面 G: 5〜10 日目に戦闘が続く)
    let mut s = session(&r, BATTLE);
    s.select(md(14)).unwrap();
    s.order_to(p(17486)).unwrap();
    run_days(&mut s, 6);
    assert_eq!(s.military().division(md(14)).unwrap().status, MapDivisionStatus::Fighting);
    assert!(s.detail_text(&r.data.model).contains("戦闘中"));
}

#[test]
fn a_retreating_selected_division_stays_selected_and_follows_to_its_new_province() {
    let r = real();
    let mut s = session(&r, BATTLE);
    s.select(md(2)).unwrap();
    s.order_to(p(17694)).unwrap();
    s.select(md(5)).unwrap(); // 防御側(国家2)。戦闘に敗れて退却する
    let before = s.military().division(md(5)).unwrap().province;
    let ev = run_days(&mut s, 12);
    let retreat = ev.iter().find_map(|e| if let MoveEvent::Retreated { division, to, .. } = e { (*division == md(5)).then_some(*to) } else { None });
    let to = retreat.expect("division 5 retreated");
    assert_eq!(s.selected(), Some(md(5)), "the selection is kept");
    let now = s.military().division(md(5)).unwrap().province;
    assert_eq!((now, now != before), (to, true));
    // マーカーは新しい現在地に出る
    assert!(s.markers().iter().any(|m| m.province == to && m.divisions.contains(&md(5))));
    assert!(s.detail_text(&r.data.model).contains(&format!("現在地: {to}")));
}

#[test]
fn a_destroyed_selected_division_clears_the_selection_and_the_detail() {
    let r = real();
    let mut s = session(&r, BATTLE);
    // 場面 E: 師団11(国家1)が 17906 の孤立した守備(師団12)を攻撃し、退路がなく撃破される
    s.select(md(11)).unwrap();
    s.order_to(p(17906)).unwrap();
    s.select(md(12)).unwrap();
    let mut cleared_on = None;
    for _ in 0..20 {
        let res = s.advance_one_day();
        if let Some(d) = res.deselected {
            cleared_on = Some((res.day, d));
            break;
        }
    }
    let (day, d) = cleared_on.expect("division 12 was destroyed while selected");
    assert_eq!(d, md(12));
    assert!(s.military().division(md(12)).is_none());
    assert_eq!(s.selected(), None);
    assert!(s.detail_text(&r.data.model).contains("未選択"));
    assert!(s.markers().iter().all(|m| !m.divisions.contains(&md(12))), "no stale marker");
    let (ok, text) = s.log().last().unwrap();
    assert!(!*ok && text.contains("撃破") && text.contains("解除"), "day {day}: {text}");
    // 撃破されていない別の選択は維持される
    let mut s = session(&r, BATTLE);
    s.select(md(11)).unwrap();
    s.order_to(p(17906)).unwrap();
    run_days(&mut s, 20);
    assert_eq!(s.selected(), Some(md(11)), "a surviving selected division stays selected");
}

// ---------------------------------------------------------------- 終了・再入場、不正入力、通常の画面への影響

#[test]
fn leaving_and_reentering_starts_from_the_initial_placement() {
    let r = real();
    let mut a = session(&r, MOVE);
    a.select(md(1)).unwrap();
    a.order_to(p(18308)).unwrap();
    run_days(&mut a, 3);
    assert_ne!(a.military(), session(&r, MOVE).military());
    // 開き直し = シナリオを読み直した新しいセッション(古い選択・日数・軍の状態を持ち越さない)
    let b = session(&r, MOVE);
    assert_ne!(a.instance(), b.instance());
    assert_eq!((b.day(), b.selected()), (0, None));
    assert_eq!(b.military(), session(&r, MOVE).military());
    // 画面を閉じる処理は、セッションとエラーのリソースを破棄する
    let mut app = App::new();
    app.insert_resource(DivisionSessionRes(a));
    app.insert_resource(DivisionLoadError("x".into()));
    app.add_systems(Update, exit_division_view);
    app.update();
    assert!(app.world().get_resource::<DivisionSessionRes>().is_none());
    assert!(app.world().get_resource::<DivisionLoadError>().is_none());
}

#[test]
fn the_screen_panels_are_buttons_so_clicks_never_reach_the_map() {
    let r = real();
    let s = session(&r, MOVE);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(DivisionSessionRes(s));
    app.add_systems(Startup, |mut commands: Commands, s: Res<DivisionSessionRes>| spawn_division_ui(&mut commands, Some(&s.0), None));
    app.update();
    let mut strips = app.world_mut().query_filtered::<&Button, With<DivisionStrip>>();
    assert_eq!(strips.iter(app.world()).count(), 2, "top and bottom strips are `Button`s (they publish `Interaction`, which suppresses map input)");
    // 操作ボタンはすべて Button(操作 6 つ + 師団 3 件)
    let mut buttons = app.world_mut().query::<(&Button, &DivisionButton)>();
    let kinds: Vec<DivisionButtonKind> = buttons.iter(app.world()).map(|(_, b)| b.0).collect();
    assert_eq!(kinds.len(), 6 + 3);
    for id in [1, 2, 3] {
        assert!(kinds.contains(&DivisionButtonKind::Pick(md(id))));
    }
}

#[test]
fn invalid_files_and_mismatches_are_refused_with_a_reason() {
    let r = real();
    let e = DivisionSession::load("assets/data/world/preview/does_not_exist.ron", r.graph.clone(), &r.data.model).unwrap_err();
    assert!(!e.is_empty());
    let dir = std::env::temp_dir().join(format!("p32_039_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.ron");
    std::fs::write(&bad, "not ron at all {").unwrap();
    let e = DivisionSession::load(bad.to_str().unwrap(), r.graph.clone(), &r.data.model).unwrap_err();
    assert!(!e.is_empty());
    // 地図にないプロヴィンス
    let mismatch = dir.join("mismatch.ron");
    std::fs::write(
        &mismatch,
        "(format_version: 1, description: \"x\", base_days: 5.0, countries: [(label: 1, country_id: 1)], state_owners: [], divisions: [(id: 1, country: 1, province: 99999, speed: 1.0)])",
    )
    .unwrap();
    let e = DivisionSession::load(mismatch.to_str().unwrap(), r.graph.clone(), &r.data.model).unwrap_err();
    assert!(e.contains("99999"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_the_division_plugin_the_normal_preview_has_no_division_state() {
    // 通常の画面(`HexPreviewPlugin` だけ)は師団のリソース・メッセージを持たない。コアプラグインを入れない App では何も増えない。
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.update();
    assert!(app.world().get_resource::<Messages<DivisionCommand>>().is_none());
    assert!(app.world().get_resource::<DivisionSessionRes>().is_none());
    // 国家ラベルの色は決定的で、未知のラベルは灰色
    assert_ne!(country_color(Some(1)), country_color(Some(2)));
    assert_eq!(country_color(Some(1)), country_color(Some(7)), "the palette wraps by label");
    assert_eq!(country_color(None), country_color(Some(0)));
}

#[test]
fn every_division_marker_has_a_unique_province_and_all_divisions_are_covered() {
    let r = real();
    let s = session(&r, BATTLE);
    let m = s.markers();
    let provs: BTreeSet<ProvinceId> = m.iter().map(|x| x.province).collect();
    assert_eq!(provs.len(), m.len(), "one marker per province");
    let covered: usize = m.iter().map(|x| x.divisions.len()).sum();
    assert_eq!(covered, s.military().divisions().count());
}
