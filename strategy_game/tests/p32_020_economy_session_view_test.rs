//! P32-020 (S1): 新マップ経済セッションの画面(`hexmap::economy_view`)のテスト。
//! 純粋関数だけでなく、実際の Bevy `App`(`MinimalPlugins` + `EconomySessionCorePlugin`)で、コマンドの受付と適用・
//! ボタン(`Interaction`)からコマンドへの接続・選択対象の導出を通す。描画(実ウィンドウ)は含まない(GUI確認は別記録)。
//! ファイルは一切作らない。

use std::fs;
use std::sync::OnceLock;

use bevy::prelude::*;
use strategy_game::app::time::{DayChangedMessage, GameDate, GamePaused, GameSpeed, MonthChangedMessage};
use strategy_game::economy::map_session::{
    ActorError, CLI_DEFAULT_SCENARIO, EconomySession, GeographyData, SCREEN_DEFAULT_SCENARIO, SessionPaths,
    build_simulation, scenario_by_id,
};
use strategy_game::economy::map_state::SimDate;
use strategy_game::hexmap::data::{HexMap, load_from_path};
use strategy_game::hexmap::economy_view::{
    ADVANCE_STEPS, ActorButton, AdvanceButton, AdvanceRejection, ClearActorButton, CommandOutcome,
    EconomySessionCorePlugin, MAX_DAYS_PER_COMMAND, SelectedView, SelectionTarget, SessionCommand, SessionConfig,
    SessionLabels, SessionRes, SessionSet, SessionUi, apply_command, block_close_key, classify_selection,
    date_text, economy_text, header_text, monthly_computed, outcome_text, session_buttons, start_session,
};
use strategy_game::hexmap::view::{HexSelection, LoadedHexMap};
use strategy_game::hexmap::world_view::{LoadedWorld, catalog_names};
use strategy_game::world::{CellStatus, MapStateId, cell_status, load_world_preview};

const START: SimDate = SimDate { year: 1800, month: 1, day: 1 };

fn geo() -> &'static GeographyData {
    static G: OnceLock<GeographyData> = OnceLock::new();
    G.get_or_init(|| GeographyData::load(&SessionPaths::default()).unwrap())
}

fn session(id: &str) -> EconomySession {
    EconomySession::load_files(scenario_by_id(id).unwrap(), geo().model(), &SessionPaths::default()).unwrap()
}

fn config(scenario: &str, actor: Option<u32>) -> SessionConfig {
    SessionConfig { scenario_id: scenario.to_string(), actor, paths: SessionPaths::default() }
}

/// コアプラグインだけのヘッドレスApp。`actor` は SessionUi へ直接入れる(未選択が既定)。
fn app_with(session: EconomySession) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(EconomySessionCorePlugin);
    app.insert_resource(SessionRes { session });
    app.add_systems(Update, session_buttons.in_set(SessionSet::Input));
    app.update(); // 初回のフレーム(コマンドなし)
    app
}

fn send(app: &mut App, cmd: SessionCommand) {
    app.world_mut().resource_mut::<Messages<SessionCommand>>().write(cmd);
}

fn sim(app: &App) -> strategy_game::economy::map_state::snapshot::MapEconomySimulation {
    app.world().resource::<SessionRes>().session.sim.clone()
}

fn actor(app: &App) -> Option<u32> {
    app.world().resource::<SessionUi>().actor
}

fn last_log(app: &App) -> (bool, String) {
    app.world().resource::<SessionUi>().log.back().cloned().expect("a log line")
}

// ---------------------------------------------------------------- 共有初期化・起動

#[test]
fn the_screen_default_is_m4e_the_cli_default_stays_m4d_and_both_use_the_shared_assembly() {
    assert_eq!(SCREEN_DEFAULT_SCENARIO, "m4e");
    assert_eq!(CLI_DEFAULT_SCENARIO, "m4d");
    // 画面の起動処理(start_session)は共有初期化(EconomySession::load_files → build_simulation)の結果と一致する。
    let cfg = config(SCREEN_DEFAULT_SCENARIO, None);
    let (s, ui) = start_session(&cfg, true, Some(geo().model())).unwrap();
    assert_eq!(s.scenario().id, "m4e");
    assert_eq!(ui.actor, None, "no actor is selected at start");
    let reference = build_simulation(scenario_by_id("m4e").unwrap(), geo().model(), &s.buildings).unwrap();
    assert_eq!(s.sim, reference);
    assert_eq!(s.sim.date, START);
    // バイナリは既定シナリオとして SCREEN_DEFAULT_SCENARIO を使い、既存CLI(world_economy_check)は CLI_DEFAULT_SCENARIO のまま。
    let bin = fs::read_to_string("src/bin/map_economy_session.rs").unwrap();
    assert!(bin.contains("SCREEN_DEFAULT_SCENARIO"));
    let cli = fs::read_to_string("src/bin/world_economy_check.rs").unwrap();
    assert!(cli.contains("CLI_DEFAULT_SCENARIO"));
}

#[test]
fn startup_does_not_fall_back_to_another_scenario_or_country() {
    let w = Some(geo().model());
    let err = |cfg: SessionConfig| start_session(&cfg, true, w).err().expect("must fail");
    assert!(err(config("nope", None)).contains("nope"));
    assert!(err(config("m4e", Some(9))).contains("国家9はこのシナリオにありません"));
    assert!(err(config("m4e", Some(3))).contains("AI国家"), "the AI country 3 is refused, not replaced");
    assert!(err(config("m4d", Some(1))).contains("AI国家"), "m4d has no manual country");
    assert!(start_session(&config("m4e", None), false, w).err().unwrap().contains("地図"));
    assert!(start_session(&config("m4e", None), true, None).err().unwrap().contains("ワールド"));
    // 有効な --actor は、画面の国家選択と同じ検証を通った選択として始まる。
    let (_, ui) = start_session(&config("m4e", Some(2)), true, w).unwrap();
    assert_eq!(ui.actor, Some(2));
}

#[test]
fn the_startup_actor_and_the_screen_selection_share_one_validation() {
    for sel in [None, Some(1), Some(2), Some(3), Some(9)] {
        let s = session("m4e");
        // 起動時の --actor
        let startup = start_session(&config("m4e", sel), true, Some(geo().model()));
        // 画面の国家選択(コマンド経由)
        let mut app = app_with(session("m4e"));
        send(&mut app, SessionCommand::SetActor(sel));
        app.update();
        match (startup, s.resolve_actor(sel), sel) {
            (Ok((_, ui)), _, _) => assert_eq!(actor(&app), ui.actor, "{sel:?}"),
            (Err(_), Err(_), Some(_)) => assert_eq!(actor(&app), None, "{sel:?}: rejected the same way"),
            (a, b, c) => panic!("{sel:?}: unexpected combination (startup ok: {}, resolve ok: {}, sel: {c:?})", a.is_ok(), b.is_ok()),
        }
    }
}

#[test]
fn no_legacy_game_plugin_is_registered_by_the_binary_or_the_core_plugin() {
    let bin_full = fs::read_to_string("src/bin/map_economy_session.rs").unwrap();
    // コメント(説明文)は除いた、実際のコードだけを検査する。
    let bin: String = bin_full.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("
");
    for banned in [
        "GameTimePlugin", "AppPlugin", "EconomyPlugin", "SaveGamePlugin", "LoadGamePlugin", "CountryPlugin", "StatePlugin",
        "BuildingPlugin", "PopulationPlugin", "LogisticsPlugin", "ResearchPlugin", "PoliticsPlugin", "DiplomacyPlugin",
        "MilitaryPlugin", "WarPlugin", "MapPlugin", "UiPlugin", "DebugPlugin", "DataLoaderPlugin",
    ] {
        assert!(!bin.contains(banned), "{banned} must not be registered by map_economy_session");
    }
    // 登録するのは3つだけ(add_plugins の呼び出しは3回)。
    assert_eq!(bin.matches("add_plugins(").count(), 3);
    for wanted in ["DefaultPlugins", "HexPreviewPlugin", "EconomySessionPlugin"] {
        assert!(bin.contains(wanted), "{wanted}");
    }
    // 実際にコアプラグインを入れたAppには、旧経路の時間・日次の資源もメッセージも存在しない。
    let mut app = app_with(session("m4e"));
    for _ in 0..5 {
        app.update();
    }
    let w = app.world();
    assert!(!w.contains_resource::<GameDate>() && !w.contains_resource::<GamePaused>() && !w.contains_resource::<GameSpeed>());
    assert!(!w.contains_resource::<Messages<DayChangedMessage>>() && !w.contains_resource::<Messages<MonthChangedMessage>>());
    assert!(!w.contains_resource::<State<strategy_game::app::game_state::GameState>>());
}

#[test]
fn the_session_view_never_writes_files() {
    let src = fs::read_to_string("src/hexmap/economy_view.rs").unwrap();
    for banned in ["fs::write", "File::create", "create_dir", ".save(", "map_economy_file::save", "OpenOptions", "remove_file"] {
        assert!(!src.contains(banned), "S1 must not write files ({banned})");
    }
    let before: Vec<_> = fs::read_dir("saves").unwrap().map(|e| e.unwrap().file_name()).collect();
    let mut app = app_with(session("m4e"));
    send(&mut app, SessionCommand::SetActor(Some(1)));
    send(&mut app, SessionCommand::AdvanceDays(30));
    app.update();
    let after: Vec<_> = fs::read_dir("saves").unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(before, after);
    assert!(!std::path::Path::new("saves/map_economy").exists());
}

// ---------------------------------------------------------------- 操作主体と進行(実際のコマンド経路)

#[test]
fn advancing_is_refused_without_an_actor_and_changes_nothing() {
    let mut app = app_with(session("m4e"));
    let before = sim(&app);
    send(&mut app, SessionCommand::AdvanceDays(1));
    app.update();
    assert_eq!(sim(&app), before, "the state (date, treasury, queues) is untouched");
    assert_eq!(actor(&app), None);
    let (ok, text) = last_log(&app);
    assert!(!ok && text.contains("操作国家が未選択"), "{text}");
}

#[test]
fn unknown_and_ai_countries_are_rejected_and_the_current_actor_is_kept() {
    let mut app = app_with(session("m4e"));
    for bad in [9u32, 3] {
        send(&mut app, SessionCommand::SetActor(Some(bad)));
        app.update();
        assert_eq!(actor(&app), None, "country {bad} is not silently replaced by another country");
        assert!(!last_log(&app).0);
    }
    send(&mut app, SessionCommand::SetActor(Some(2)));
    app.update();
    assert_eq!(actor(&app), Some(2));
    send(&mut app, SessionCommand::SetActor(Some(3))); // AI国家: 拒否され、国家2のまま(先頭国家へのフォールバックなし)
    app.update();
    assert_eq!(actor(&app), Some(2));
    send(&mut app, SessionCommand::SetActor(None));
    app.update();
    assert_eq!(actor(&app), None);
    // 全国家がAIのシナリオでは、どの国家も選べず、進行もできない。
    let mut d = app_with(session("m4d"));
    send(&mut d, SessionCommand::SetActor(Some(1)));
    send(&mut d, SessionCommand::AdvanceDays(1));
    d.update();
    assert_eq!(actor(&d), None);
    assert_eq!(sim(&d), session("m4d").sim);
}

#[test]
fn one_step_and_multi_day_advances_match_each_other_and_the_direct_session() {
    // 画面のコマンド経路(1日×38 / 1回で7+30+1)と、共有セッションの直接の advance_day×38 が一致する。
    let mut direct = session("m4e");
    for _ in 0..38 {
        direct.advance_day();
    }
    let mut single = app_with(session("m4e"));
    send(&mut single, SessionCommand::SetActor(Some(1)));
    single.update();
    for _ in 0..38 {
        send(&mut single, SessionCommand::AdvanceDays(1));
        single.update();
    }
    let mut multi = app_with(session("m4e"));
    send(&mut multi, SessionCommand::SetActor(Some(1)));
    send(&mut multi, SessionCommand::AdvanceDays(30));
    send(&mut multi, SessionCommand::AdvanceDays(7));
    send(&mut multi, SessionCommand::AdvanceDays(1));
    multi.update(); // 同じフレームに3コマンド: 到着順に適用
    assert_eq!(sim(&single), direct.sim);
    assert_eq!(sim(&multi), direct.sim);
    assert_eq!(direct.sim.date, SimDate { year: 1800, month: 2, day: 8 });
    // 参照用の独立した組み立て(共有APIを使わない)との一致も確認する。
    let mut reference = build_simulation(scenario_by_id("m4e").unwrap(), geo().model(), &direct.buildings).unwrap();
    for _ in 0..38 {
        reference.advance_day(&direct.buildings);
    }
    assert_eq!(reference, direct.sim);
}

#[test]
fn monthly_processing_runs_once_per_month_boundary_regardless_of_how_the_days_are_split() {
    let mut runs_by_split = Vec::new();
    for split in [vec![30u32, 30], vec![7, 23, 30], vec![30, 15, 15], vec![1; 60]] {
        let mut s = session("m4e");
        let mut actor = Some(1);
        let mut monthly = 0;
        for d in split {
            let CommandOutcome::Advanced(sum) = apply_command(&mut s, &mut actor, SessionCommand::AdvanceDays(d)) else {
                panic!("advance refused")
            };
            monthly += sum.monthly_runs;
        }
        runs_by_split.push((monthly, s.sim.clone()));
    }
    // 1800/01/01 から60日 = 1800/03/02: 月境界は 2/1 と 3/1 の2回だけ。分け方で回数も状態も変わらない。
    for (m, sim) in &runs_by_split {
        assert_eq!(*m, 2, "exactly two monthly runs (Feb 1 and Mar 1)");
        assert_eq!(sim.date, SimDate { year: 1800, month: 3, day: 2 });
        assert_eq!(sim, &runs_by_split[0].1);
    }
}

#[test]
fn invalid_day_counts_are_refused_without_changing_anything() {
    let mut s = session("m4e");
    let mut actor = Some(1);
    let before = s.sim.clone();
    for n in [0u32, MAX_DAYS_PER_COMMAND + 1, 1000, u32::MAX] {
        assert_eq!(
            apply_command(&mut s, &mut actor, SessionCommand::AdvanceDays(n)),
            CommandOutcome::AdvanceRejected(AdvanceRejection::InvalidDays(n))
        );
    }
    assert_eq!(s.sim, before);
    assert!(ADVANCE_STEPS.iter().all(|d| (1..=MAX_DAYS_PER_COMMAND).contains(d)));
    assert_eq!(
        apply_command(&mut s, &mut None, SessionCommand::AdvanceDays(1)),
        CommandOutcome::AdvanceRejected(AdvanceRejection::Actor(ActorError::NotSelected))
    );
    assert_eq!(s.sim, before);
}

#[test]
fn commands_in_one_frame_are_applied_in_arrival_order_and_the_actor_is_checked_when_applied() {
    let mut app = app_with(session("m4e"));
    // 先に進行 → 後で国家選択: 進行は(その時点で未選択なので)拒否される。
    send(&mut app, SessionCommand::AdvanceDays(1));
    send(&mut app, SessionCommand::SetActor(Some(1)));
    send(&mut app, SessionCommand::AdvanceDays(1)); // これは通る
    send(&mut app, SessionCommand::SetActor(None));
    send(&mut app, SessionCommand::AdvanceDays(1)); // 解除後なので拒否
    app.update();
    assert_eq!(sim(&app).date, SimDate { year: 1800, month: 1, day: 2 }, "exactly one day was processed");
    assert_eq!(actor(&app), None);
    let log: Vec<bool> = app.world().resource::<SessionUi>().log.iter().map(|(ok, _)| *ok).collect();
    assert_eq!(log, vec![false, true, true, true, false]);
}

#[test]
fn rendering_updates_selection_changes_and_actor_changes_do_not_change_the_economy() {
    let mut app = app_with(session("m4e"));
    send(&mut app, SessionCommand::SetActor(Some(1)));
    send(&mut app, SessionCommand::AdvanceDays(10));
    app.update();
    let before = sim(&app);
    // 何もしないフレームを多数: 日付も経済も進まない。
    for _ in 0..120 {
        app.update();
    }
    assert_eq!(sim(&app), before);
    // 選択の変更・国家の変更・解除でも、経済状態(日付・国庫・施設・キュー)はリセットも変更もされない。
    app.insert_resource(HexSelection(Some(0)));
    app.update();
    for sel in [Some(2), Some(1), None, Some(2)] {
        send(&mut app, SessionCommand::SetActor(sel));
        app.update();
        assert_eq!(sim(&app), before, "{sel:?}");
    }
    assert_eq!(sim(&app).date, SimDate { year: 1800, month: 1, day: 11 });
}

// ---------------------------------------------------------------- ボタン → コマンド → 適用

#[test]
fn a_button_press_issues_exactly_one_command_even_if_the_press_is_held() {
    let mut app = app_with(session("m4e"));
    let actor_btn = app.world_mut().spawn((ActorButton(1), Interaction::None)).id();
    let adv = app.world_mut().spawn((AdvanceButton(1), Interaction::None)).id();
    let clear = app.world_mut().spawn((ClearActorButton, Interaction::None)).id();
    app.update();
    // 国家ボタン
    *app.world_mut().get_mut::<Interaction>(actor_btn).unwrap() = Interaction::Pressed;
    app.update();
    assert_eq!(actor(&app), Some(1));
    // 進行ボタンを押し続ける: 押した瞬間の1回だけ(10フレーム押下状態でも1日)。
    *app.world_mut().get_mut::<Interaction>(adv).unwrap() = Interaction::Pressed;
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(sim(&app).date, SimDate { year: 1800, month: 1, day: 2 });
    // 離して、もう一度押すと、もう1日。
    *app.world_mut().get_mut::<Interaction>(adv).unwrap() = Interaction::None;
    app.update();
    *app.world_mut().get_mut::<Interaction>(adv).unwrap() = Interaction::Pressed;
    app.update();
    assert_eq!(sim(&app).date, SimDate { year: 1800, month: 1, day: 3 });
    // ホバーだけでは進まない。
    *app.world_mut().get_mut::<Interaction>(adv).unwrap() = Interaction::Hovered;
    app.update();
    assert_eq!(sim(&app).date, SimDate { year: 1800, month: 1, day: 3 });
    // 選択解除ボタン → 未選択 → 進行ボタンは拒否される。
    *app.world_mut().get_mut::<Interaction>(clear).unwrap() = Interaction::Pressed;
    app.update();
    assert_eq!(actor(&app), None);
    *app.world_mut().get_mut::<Interaction>(adv).unwrap() = Interaction::None;
    app.update();
    *app.world_mut().get_mut::<Interaction>(adv).unwrap() = Interaction::Pressed;
    app.update();
    assert_eq!(sim(&app).date, SimDate { year: 1800, month: 1, day: 3 }, "refused while no actor is selected");
}

#[test]
fn the_tab_key_that_would_discard_the_session_is_swallowed() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<ButtonInput<KeyCode>>();
    app.add_systems(Update, block_close_key);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Tab);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyG);
    app.update();
    let keys = app.world().resource::<ButtonInput<KeyCode>>();
    assert!(!keys.just_pressed(KeyCode::Tab), "Tab never reaches the preview's close handler");
    assert!(keys.just_pressed(KeyCode::KeyG), "other shortcuts are untouched");
}

// ---------------------------------------------------------------- 選択対象

struct World2 {
    map: HexMap,
    loaded: LoadedWorld,
}

fn world2() -> World2 {
    let p = SessionPaths::default();
    let map = load_from_path(&p.map).unwrap();
    let data = load_world_preview(&p.provinces, &p.states, &map).unwrap();
    let names = catalog_names(&data);
    World2 { map, loaded: LoadedWorld { data, names } }
}

fn find_cell(w: &World2, want: &dyn Fn(CellStatus) -> bool) -> u32 {
    let m = &w.loaded.data.model;
    w.map
        .cells()
        .iter()
        .filter(|c| (c.q - 83).abs() <= 8 && (c.r - 81).abs() <= 8)
        .find(|c| want(cell_status(m, &w.map, c.q, c.r)))
        .and_then(|c| w.map.cell_index(c.q, c.r))
        .expect("a cell of that kind exists")
}

fn state_cell(w: &World2, id: u32) -> u32 {
    let m = &w.loaded.data.model;
    let p = m.provinces_of(MapStateId(id)).unwrap()[0];
    let c = m.province(p).unwrap().cell;
    w.map.cell_index(c.q, c.r).unwrap()
}

#[test]
fn the_selected_target_follows_the_cell_and_never_keeps_the_previous_state() {
    let w = world2();
    let s = session("m4e");
    let m = &w.loaded.data.model;
    let t = |sel: Option<u32>| classify_selection(m, &w.map, sel, &s);
    assert_eq!(t(None), SelectionTarget::None);
    assert_eq!(t(Some(u32::MAX)), SelectionTarget::NoCell);
    assert!(matches!(t(Some(state_cell(&w, 20))), SelectionTarget::State { state, in_economy: true, ref countries, .. } if state == MapStateId(20) && *countries == vec![1]));
    assert!(matches!(t(Some(state_cell(&w, 22))), SelectionTarget::State { ref countries, .. } if *countries == vec![3]));
    assert_eq!(t(Some(find_cell(&w, &|c| matches!(c, CellStatus::Water)))), SelectionTarget::Water);
    assert!(matches!(t(Some(find_cell(&w, &|c| matches!(c, CellStatus::Unassigned { .. })))), SelectionTarget::Unassigned { .. }));
    assert_eq!(t(Some(find_cell(&w, &|c| matches!(c, CellStatus::OutOfScope)))), SelectionTarget::OutOfScope);

    // Bevy経由: 選択を State → Water → 無効 と変えると、SelectedView は毎回その対象に置き換わる。
    let mut app = app_with(session("m4e"));
    let names = w.loaded.names.clone();
    let water = find_cell(&w, &|c| matches!(c, CellStatus::Water));
    let st20 = state_cell(&w, 20);
    app.insert_resource(LoadedHexMap(w.map));
    app.insert_resource(w.loaded);
    app.insert_resource(HexSelection(Some(st20)));
    app.update();
    assert!(matches!(app.world().resource::<SelectedView>().0, SelectionTarget::State { state, .. } if state == MapStateId(20)));
    app.insert_resource(HexSelection(Some(water)));
    app.update();
    assert_eq!(app.world().resource::<SelectedView>().0, SelectionTarget::Water, "no state reference remains");
    let text = {
        let r = app.world().resource::<SessionRes>();
        economy_text(&r.session, &SessionLabels::build(), None, &app.world().resource::<SelectedView>().0, &names)
    };
    assert!(text.contains("水域") && !text.contains("ステート#20") && !text.contains("人口"), "{text}");
    app.insert_resource(HexSelection(Some(u32::MAX)));
    app.update();
    assert_eq!(app.world().resource::<SelectedView>().0, SelectionTarget::NoCell);
    app.world_mut().remove_resource::<HexSelection>();
    app.update();
    assert_eq!(app.world().resource::<SelectedView>().0, SelectionTarget::None);
}

// ---------------------------------------------------------------- 表示テキスト

#[test]
fn the_date_is_the_last_processed_day_and_unprocessed_at_the_start() {
    let s = session("m4e");
    let t = date_text(START, s.sim.date);
    assert!(t.contains("未処理") && t.contains("開始日 1800/01/01") && t.contains("次に処理する日: 1800/01/02"), "{t}");
    let mut s = s;
    s.advance_day();
    let t = date_text(START, s.sim.date);
    assert!(t.contains("処理済み日付: 1800/01/02") && t.contains("次に処理する日: 1800/01/03") && !t.contains("未処理"), "{t}");
    assert!(!monthly_computed(START, SimDate { year: 1800, month: 1, day: 31 }));
    assert!(monthly_computed(START, SimDate { year: 1800, month: 2, day: 1 }));
}

#[test]
fn uncomputed_monthly_values_are_not_shown_as_zero_and_become_visible_after_the_month_boundary() {
    let w = world2();
    let mut s = session("m4e");
    let labels = SessionLabels::build();
    let target = classify_selection(&w.loaded.data.model, &w.map, Some(state_cell(&w, 20)), &s);
    let names = &w.loaded.names;
    let t0 = economy_text(&s, &labels, Some(1), &target, names);
    assert!(t0.contains("未計算"), "{t0}");
    assert!(!t0.contains("雇用: 0"), "an uncomputed value is not displayed as a confirmed 0: {t0}");
    assert!(t0.contains("人口: 600001") && t0.contains("国庫: 5000.0"), "inputs are shown: {t0}");
    assert!(t0.contains("操作国家のステートです") && t0.contains("経済計算上の対応国家: 国家1(手動)"), "{t0}");
    let mut a = Some(1);
    apply_command(&mut s, &mut a, SessionCommand::AdvanceDays(30));
    assert!(economy_text(&s, &labels, Some(1), &target, names).contains("未計算"), "still Jan 31");
    apply_command(&mut s, &mut a, SessionCommand::AdvanceDays(1)); // 2/1: 最初の月次処理
    let t1 = economy_text(&s, &labels, Some(1), &target, names);
    assert!(!t1.contains("未計算") && t1.contains("雇用:") && t1.contains("物流:") && t1.contains("月次収支:"), "{t1}");
}

#[test]
fn other_countries_states_are_labelled_as_view_only_and_an_unselected_actor_is_stated() {
    let w = world2();
    let s = session("m4e");
    let labels = SessionLabels::build();
    let st21 = classify_selection(&w.loaded.data.model, &w.map, Some(state_cell(&w, 21)), &s);
    let other = economy_text(&s, &labels, Some(1), &st21, &w.loaded.names);
    assert!(other.contains("他国家のステートです") && other.contains("国家2(手動)") && !other.contains("操作国家のステートです"), "{other}");
    let none = economy_text(&s, &labels, None, &st21, &w.loaded.names);
    assert!(none.contains("操作国家: 未選択") && none.contains("操作国家が未選択です"), "{none}");
    let ai = classify_selection(&w.loaded.data.model, &w.map, Some(state_cell(&w, 22)), &s);
    assert!(economy_text(&s, &labels, Some(1), &ai, &w.loaded.names).contains("国家3(AI)"));
    // 経済に含まれないケース・範囲外・未割当。
    for (cell, needle) in [
        (find_cell(&w, &|c| matches!(c, CellStatus::Unassigned { .. })), "ステート未割当"),
        (find_cell(&w, &|c| matches!(c, CellStatus::OutOfScope)), "検証範囲外"),
        (find_cell(&w, &|c| matches!(c, CellStatus::Water)), "水域"),
    ] {
        let t = classify_selection(&w.loaded.data.model, &w.map, Some(cell), &s);
        assert!(economy_text(&s, &labels, Some(1), &t, &w.loaded.names).contains(needle), "{needle}");
    }
}

#[test]
fn the_screen_says_it_is_a_validation_session_and_exposes_no_internal_names() {
    let w = world2();
    let s = session("m4e");
    let labels = SessionLabels::build();
    let h = header_text(&s, None);
    assert!(h.contains("検証用") && h.contains("M4-E") && h.contains("正式な世界設定・ゲーム開始ではありません"), "{h}");
    assert!(h.contains("操作国家: 未選択") && h.contains("操作国家を選択してください"), "{h}");
    let target = classify_selection(&w.loaded.data.model, &w.map, Some(state_cell(&w, 20)), &s);
    let all = format!("{h}\n{}", economy_text(&s, &labels, Some(1), &target, &w.loaded.names));
    for internal in ["MapStateId", "ScenarioCountry", "MapStateEconomy", "ConstructionOrderId", "StaticIdentity", "blake3"] {
        assert!(!all.contains(internal), "{internal} leaked: {all}");
    }
    let hex16 = all.split(|c: char| !c.is_ascii_hexdigit()).any(|t| t.len() >= 16);
    assert!(!hex16, "no digest-like token is shown: {all}");
    assert!(all.contains("保存・復元の操作は後続の段階(S3)で追加します"));
    // 結果欄の文言: 進行の要約・拒否の理由。
    let mut s = session("m4e");
    let mut a = Some(1);
    apply_command(&mut s, &mut a, SessionCommand::AdvanceDays(30));
    let out = apply_command(&mut s, &mut a, SessionCommand::AdvanceDays(2));
    let (ok, text) = outcome_text(&out, &s, &w.loaded.names);
    assert!(out.is_ok() && ok && text.contains("2日進めました") && text.contains("月次経済 1回"), "{text}");
    let (ok, text) = outcome_text(&apply_command(&mut s, &mut None, SessionCommand::AdvanceDays(1)), &s, &w.loaded.names);
    assert!(!ok && text.contains("未選択"), "{text}");
    let (ok, text) = outcome_text(&apply_command(&mut s, &mut a, SessionCommand::AdvanceDays(0)), &s, &w.loaded.names);
    assert!(!ok && text.contains("範囲外"), "{text}");
}
