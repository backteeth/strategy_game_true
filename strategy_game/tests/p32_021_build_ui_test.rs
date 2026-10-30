//! P32-021 (S2): 建設候補・注文・建設キュー・キャンセルのUI(`hexmap::build_view` と `economy_view` のコマンド)のテスト。
//! 実際の Bevy `App`(`MinimalPlugins` + 実際のシステム: `session_buttons` → `apply_session_commands` → `sync_build_panel`)を通し、
//! ボタン(`Interaction`)→コマンド→共通API→パネルの作り直し、までを確認する。描画(実ウィンドウ)・実マウスは含まない。
//! ファイルは一切作らない。

use std::fs;
use std::sync::OnceLock;

use bevy::prelude::*;
use bevy::ui::{OverflowAxis, RelativeCursorPosition};
use strategy_game::building::data::BuildingType;
use strategy_game::economy::map_session::{
    ActorError, BuildRequestError, CancelRequestError, EconomySession, GeographyData, SessionPaths, scenario_by_id,
};
use strategy_game::economy::map_state::{
    CancelError, ConstructionOrderId, OrderError, SimDate, is_state_level_economic_facility,
};
use strategy_game::hexmap::build_view::{
    BuildButton, BuildHeaderText, BuildListRoot, BuildPanelRoot, CancelButton, build_panel_model, spawn_build_panel,
    sync_build_panel, sync_build_panel_visibility,
};
use strategy_game::hexmap::economy_view::{
    CommandOutcome, SelectedView, SelectionTarget, SessionCommand, SessionRes, SessionUi, apply_command,
    apply_session_commands, outcome_text, session_buttons,
};
use strategy_game::world::{MapStateId, ProvinceId};

use BuildingType::{CrystalMine, Farm, LoggingCamp, Mine};

fn geo() -> &'static GeographyData {
    static G: OnceLock<GeographyData> = OnceLock::new();
    G.get_or_init(|| GeographyData::load(&SessionPaths::default()).unwrap())
}

fn session() -> EconomySession {
    EconomySession::load_files(scenario_by_id("m4e").unwrap(), geo().model(), &SessionPaths::default()).unwrap()
}

fn state_target(state: u32, countries: Vec<u32>) -> SelectionTarget {
    SelectionTarget::State { state: MapStateId(state), province: ProvinceId(1), in_economy: true, countries }
}

/// 実際のシステム(ボタン → コマンド → 適用 → パネルの作り直し)をこの順に動かす画面のApp。
fn ui_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_message::<SessionCommand>().init_resource::<SessionUi>().init_resource::<SelectedView>();
    // S3以降 `apply_session_commands` は保存画面の状態と保存先も必要とする(この試験では使わない。ファイルは作らない)。
    app.init_resource::<strategy_game::hexmap::save_view::SaveUi>().init_resource::<strategy_game::hexmap::save_view::SaveLocationRes>();
    app.insert_resource(SessionRes { session: session() });
    app.add_systems(
        Update,
        (session_buttons, apply_session_commands, sync_build_panel, sync_build_panel_visibility).chain(),
    );
    let mut commands = app.world_mut().commands();
    spawn_build_panel(&mut commands);
    app.world_mut().flush();
    app.update();
    app
}

fn select(app: &mut App, t: SelectionTarget) {
    app.world_mut().resource_mut::<SelectedView>().0 = t;
}

fn send(app: &mut App, cmd: SessionCommand) {
    app.world_mut().resource_mut::<Messages<SessionCommand>>().write(cmd);
}

fn sim(app: &App) -> strategy_game::economy::map_state::snapshot::MapEconomySimulation {
    app.world().resource::<SessionRes>().session.sim.clone()
}

fn treasury(app: &App, c: u32) -> f64 {
    app.world().resource::<SessionRes>().session.country(c).unwrap().treasury
}

fn queue_ids(app: &App, c: u32) -> Vec<ConstructionOrderId> {
    app.world().resource::<SessionRes>().session.country(c).unwrap().construction_queue.iter().map(|o| o.id).collect()
}

fn last_log(app: &App) -> (bool, String) {
    app.world().resource::<SessionUi>().log.back().cloned().expect("a log line")
}

fn build_buttons(app: &mut App) -> Vec<(Entity, u32, MapStateId, BuildingType)> {
    let mut q = app.world_mut().query::<(Entity, &BuildButton)>();
    q.iter(app.world()).map(|(e, b)| (e, b.actor, b.state, b.building)).collect()
}

fn cancel_buttons(app: &mut App) -> Vec<(Entity, u32, ConstructionOrderId)> {
    let mut q = app.world_mut().query::<(Entity, &CancelButton)>();
    q.iter(app.world()).map(|(e, b)| (e, b.actor, b.order)).collect()
}

fn build_button(app: &mut App, b: BuildingType) -> Entity {
    build_buttons(app).into_iter().find(|(_, _, _, x)| *x == b).unwrap_or_else(|| panic!("{b:?} has no order button")).0
}

/// 押した瞬間(`Pressed` へ変化)を1フレーム与える。
fn press(app: &mut App, e: Entity) {
    *app.world_mut().get_mut::<Interaction>(e).expect("a button has an Interaction") = Interaction::Pressed;
    app.update();
}

/// 操作国家を選び(コマンド経由)、ステートを選択して、パネルを作り直す。
fn start(app: &mut App, actor: u32, state: u32, owner: u32) {
    send(app, SessionCommand::SetActor(Some(actor)));
    select(app, state_target(state, vec![owner]));
    app.update();
}

// ---------------------------------------------------------------- 候補

#[test]
fn candidates_come_from_the_existing_definitions_and_agree_with_the_shared_check() {
    let s = session();
    let names = std::collections::HashMap::new();
    let m = build_panel_model(&s, Some(1), &state_target(20, vec![1]), &names);
    let listed: Vec<BuildingType> = m.candidates.iter().map(|c| c.building).collect();
    let expected: Vec<BuildingType> = BuildingType::ALL.into_iter().filter(|b| is_state_level_economic_facility(*b)).collect();
    assert_eq!(listed, expected, "the list is the existing facility definition, not a UI-owned list");
    for c in &m.candidates {
        let direct = s.check_build(Some(1), MapStateId(20), c.building);
        match direct {
            Ok(check) => {
                assert_eq!(c.order, Some((1, check)), "{:?}", c.building);
                assert!(c.reason.is_none() && c.text.contains("費用") && c.text.contains("期間"));
            }
            Err(e) => {
                assert!(c.order.is_none(), "{:?}", c.building);
                let reason = c.reason.as_ref().expect("a rejected candidate states its reason");
                assert!(c.text.contains(reason.as_str()), "the reason is shown");
                if let BuildRequestError::Order(OrderError::NoCrystalDeposit(_)) = e {
                    assert!(reason.contains("クリスタル鉱床"), "{reason}");
                }
            }
        }
    }
    // 既存規則の例: クリスタル鉱山は鉱床が無いので拒否され、理由が表示される。国庫が足りない施設も理由つき。
    let crystal = m.candidates.iter().find(|c| c.building == CrystalMine).unwrap();
    assert!(crystal.order.is_none() && crystal.reason.as_deref().unwrap().contains("クリスタル"));
    let mut poor = session();
    poor.sim.countries[0].treasury = 100.0;
    let m = build_panel_model(&poor, Some(1), &state_target(20, vec![1]), &names);
    let farm = m.candidates.iter().find(|c| c.building == Farm).unwrap();
    assert!(farm.order.is_none() && farm.reason.as_deref().unwrap().contains("国庫が足りません"), "{farm:?}");
    // 候補の表示だけでは何も変わらない。
    let before = poor.sim.clone();
    for _ in 0..3 {
        let _ = build_panel_model(&poor, Some(1), &state_target(20, vec![1]), &names);
    }
    assert_eq!(poor.sim, before);
}

#[test]
fn building_the_panel_repeatedly_never_changes_the_economy() {
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    let before = sim(&app);
    for _ in 0..60 {
        app.update();
    }
    // 選択を行き来してパネルを何度も作り直しても、経済状態(国庫・備蓄・キュー・連番・order_log・AI履歴・施設)は不変。
    for t in [state_target(21, vec![2]), SelectionTarget::Water, state_target(20, vec![1]), SelectionTarget::None] {
        select(&mut app, t);
        app.update();
    }
    assert_eq!(sim(&app), before);
}

// ---------------------------------------------------------------- 発注

#[test]
fn pressing_an_order_button_orders_and_charges_exactly_once_even_when_held() {
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    assert_eq!(treasury(&app, 1), 5000.0);
    let farm = build_button(&mut app, Farm);
    press(&mut app, farm);
    for _ in 0..6 {
        app.update(); // 押し続けた状態・再描画
    }
    assert_eq!(treasury(&app, 1), 4800.0, "charged once (Farm cost 200)");
    assert_eq!(queue_ids(&app, 1), vec![ConstructionOrderId { country: 1, seq: 1 }]);
    assert_eq!(app.world().resource::<SessionRes>().session.country(1).unwrap().next_order_seq, 2);
    // 作り直された行は新しいEntityで、押下状態を引き継がない。Farm は既に注文済みなので候補のボタンから消える。
    assert!(!build_buttons(&mut app).iter().any(|(_, _, _, b)| *b == Farm));
    let mut q = app.world_mut().query::<(&BuildButton, &Interaction)>();
    assert!(q.iter(app.world()).all(|(_, i)| *i == Interaction::None));
    let (ok, text) = last_log(&app);
    assert!(ok && text.contains("発注しました") && text.contains("Farm") && text.contains("費用 200"), "{text}");
    // UI独自の重複禁止は無い: 別の施設は注文できる。同じ施設の重複は共通APIが拒否する(強制したコマンドでも)。
    let logging = build_button(&mut app, LoggingCamp);
    press(&mut app, logging);
    assert_eq!(treasury(&app, 1), 4550.0);
    let before = sim(&app);
    send(&mut app, SessionCommand::Build { actor: 1, state: MapStateId(20), building: Farm });
    app.update();
    assert_eq!(last_log(&app).1, "発注できません: 同じ施設の注文が建設キューにあります");
    assert_eq!(sim(&app), before);
}

#[test]
fn an_order_reaches_completion_exactly_like_the_direct_session_including_months_and_ai() {
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    let farm = build_button(&mut app, Farm);
    press(&mut app, farm);
    send(&mut app, SessionCommand::AdvanceDays(30)); // Farm は30日で完成する
    send(&mut app, SessionCommand::AdvanceDays(30));
    send(&mut app, SessionCommand::AdvanceDays(10));
    app.update();
    let mut direct = session();
    direct.request_build(Some(1), MapStateId(20), Farm).unwrap();
    for _ in 0..70 {
        direct.advance_day();
    }
    assert_eq!(sim(&app), direct.sim, "same orders and days give the same state (months, completion and AI orders included)");
    let farm_level = |s: &EconomySession| s.state_economy(MapStateId(20)).unwrap().buildings[&Farm];
    assert_eq!(farm_level(&app.world().resource::<SessionRes>().session), 4, "the daily processing raised the level, not the UI");
    assert!(queue_ids(&app, 1).is_empty());
}

#[test]
fn queue_rows_show_progress_from_the_real_queue_without_guessing() {
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    let mine = build_button(&mut app, Mine);
    press(&mut app, mine);
    let text_of = |app: &App| {
        let r = app.world().resource::<SessionRes>();
        let m = build_panel_model(&r.session, Some(1), &state_target(20, vec![1]), &Default::default());
        m.queues[0].rows[0].text.clone()
    };
    let t0 = text_of(&app);
    assert!(t0.contains("Mine Lv3") && t0.contains("待機中") && t0.contains("進捗 0/60日(残り 60日)") && t0.contains("支払済み 500"), "{t0}");
    send(&mut app, SessionCommand::AdvanceDays(7));
    app.update();
    let t1 = text_of(&app);
    assert!(t1.contains("建設中") && t1.contains("進捗 7/60日(残り 53日)"), "{t1}");
    // 取消し時の返金の表示は共通の cancel_refund(支払済み×0.5)。
    let (_, _, order) = cancel_buttons(&mut app)[0];
    let r = app.world().resource::<SessionRes>();
    let m = build_panel_model(&r.session, Some(1), &state_target(20, vec![1]), &Default::default());
    assert_eq!(m.queues[0].rows[0].refund, 250.0);
    assert_eq!(m.queues[0].rows[0].order, order);
}

// ---------------------------------------------------------------- キャンセル

#[test]
fn cancelling_by_the_button_refunds_through_the_shared_api_exactly_once() {
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    let farm = build_button(&mut app, Farm);
    press(&mut app, farm);
    send(&mut app, SessionCommand::AdvanceDays(5));
    app.update();
    let (btn, actor, order) = cancel_buttons(&mut app).remove(0);
    assert_eq!((actor, order), (1, ConstructionOrderId { country: 1, seq: 1 }));
    press(&mut app, btn);
    for _ in 0..4 {
        app.update();
    }
    assert!(queue_ids(&app, 1).is_empty());
    assert_eq!(treasury(&app, 1), 5000.0 - 200.0 + 100.0, "refund = paid x 0.5, once");
    assert!(cancel_buttons(&mut app).is_empty());
    // 直接実行と一致。
    let mut direct = session();
    direct.request_build(Some(1), MapStateId(20), Farm).unwrap();
    for _ in 0..5 {
        direct.advance_day();
    }
    direct.cancel_order(Some(1), order).unwrap();
    assert_eq!(sim(&app), direct.sim);
    // 古い取消し(同じ識別子)を繰り返しても二重に返金しない。
    let after = sim(&app);
    for _ in 0..3 {
        send(&mut app, SessionCommand::Cancel { actor: 1, order });
        app.update();
        assert_eq!(last_log(&app).1, "取り消せません: すでに取り消された注文です(二重に返金しません)");
    }
    assert_eq!(sim(&app), after);
}

#[test]
fn a_stale_cancel_after_completion_neither_refunds_nor_cancels_another_order() {
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    let farm = build_button(&mut app, Farm);
    press(&mut app, farm);
    let farm_id = cancel_buttons(&mut app)[0].2;
    send(&mut app, SessionCommand::AdvanceDays(30)); // Farm が完成してキューを離れる
    app.update();
    assert!(queue_ids(&app, 1).is_empty());
    // 新しい注文(Mine)を入れた後でも、古い Farm の取消しは Mine を取り消さず、返金もしない。
    let mine = build_button(&mut app, Mine);
    press(&mut app, mine);
    let mine_id = queue_ids(&app, 1)[0];
    let before = sim(&app);
    send(&mut app, SessionCommand::Cancel { actor: 1, order: farm_id });
    app.update();
    assert_eq!(last_log(&app).1, "取り消せません: 完成済みの注文のため取り消せません(返金しません)");
    assert_eq!(sim(&app), before);
    assert_eq!(queue_ids(&app, 1), vec![mine_id]);
}

#[test]
fn buttons_keep_the_real_order_id_so_a_changed_queue_order_never_moves_the_target() {
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    for b in [Farm, Mine, LoggingCamp] {
        let e = build_button(&mut app, b);
        press(&mut app, e);
    }
    let ids = queue_ids(&app, 1);
    assert_eq!(ids.len(), 3);
    let shown: Vec<ConstructionOrderId> = cancel_buttons(&mut app).iter().map(|(_, _, o)| *o).collect();
    let mut a = shown.clone();
    a.sort();
    let mut b = ids.clone();
    b.sort();
    assert_eq!(a, b, "every row holds its own real order id");
    // 先頭(Farm)を取り消すとキュー上の位置が全てずれるが、残りの行は自分の識別子を持ち続け、別の注文は取り消されない。
    send(&mut app, SessionCommand::Cancel { actor: 1, order: ids[0] });
    app.update();
    assert_eq!(queue_ids(&app, 1), vec![ids[1], ids[2]]);
    let shown: Vec<ConstructionOrderId> = cancel_buttons(&mut app).iter().map(|(_, _, o)| *o).collect();
    assert!(shown.contains(&ids[1]) && shown.contains(&ids[2]) && !shown.contains(&ids[0]));
    // 中間(Mine)を取り消しても、末尾(LoggingCamp)の注文は残る。
    send(&mut app, SessionCommand::Cancel { actor: 1, order: ids[1] });
    app.update();
    assert_eq!(queue_ids(&app, 1), vec![ids[2]]);
}

// ---------------------------------------------------------------- 拒否・古い操作

#[test]
fn unselected_other_country_and_non_state_targets_cannot_order() {
    let mut app = ui_app();
    // 操作国家が未選択: 候補は確認だけ(ボタンなし)。強制したコマンドも実行されない。
    select(&mut app, state_target(20, vec![1]));
    app.update();
    assert!(build_buttons(&mut app).is_empty());
    let header = app.world_mut().query::<(&Text, &BuildHeaderText)>().single(app.world()).unwrap().0.0.clone();
    assert!(header.contains("操作国家が未選択のため発注できません"), "{header}");
    let before = sim(&app);
    send(&mut app, SessionCommand::Build { actor: 1, state: MapStateId(20), building: Farm });
    send(&mut app, SessionCommand::Cancel { actor: 1, order: ConstructionOrderId { country: 1, seq: 1 } });
    app.update();
    assert_eq!(sim(&app), before);
    assert!(!last_log(&app).0 && last_log(&app).1.contains("操作国家が変わっています"));
    // 他国家のステート: 候補は共通APIの拒否理由つきで、ボタンなし。強制しても共通APIが拒否する。
    send(&mut app, SessionCommand::SetActor(Some(1)));
    select(&mut app, state_target(21, vec![2]));
    app.update();
    assert!(build_buttons(&mut app).is_empty());
    let header = app.world_mut().query::<(&Text, &BuildHeaderText)>().single(app.world()).unwrap().0.0.clone();
    assert!(header.contains("他国家のステートのため発注できません"), "{header}");
    let before = sim(&app);
    send(&mut app, SessionCommand::Build { actor: 1, state: MapStateId(21), building: Farm });
    app.update();
    assert!(last_log(&app).1.contains("他国家のステート"), "{}", last_log(&app).1);
    assert_eq!(sim(&app), before);
    // 水域・未割当・検証範囲外・地図外・選択なし・経済に含まれないステート: 候補なし・理由つき。
    for (t, needle) in [
        (SelectionTarget::Water, "水域"),
        (SelectionTarget::Unassigned { province: ProvinceId(7) }, "未割当"),
        (SelectionTarget::OutOfScope, "検証範囲外"),
        (SelectionTarget::NoCell, "地図に存在しない"),
        (SelectionTarget::None, "ステートを選択"),
        (SelectionTarget::State { state: MapStateId(99), province: ProvinceId(1), in_economy: false, countries: vec![] }, "経済に含まれない"),
    ] {
        select(&mut app, t);
        app.update();
        assert!(build_buttons(&mut app).is_empty(), "{needle}: no stale order buttons remain");
        let header = app.world_mut().query::<(&Text, &BuildHeaderText)>().single(app.world()).unwrap().0.0.clone();
        assert!(header.contains(needle), "{needle}: {header}");
    }
}

#[test]
fn an_operation_made_for_a_previous_country_or_state_is_never_applied_to_the_new_one() {
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    let farm = build_button(&mut app, Farm); // 国家1・ステート20 用のボタン
    let before_t2 = treasury(&app, 2);
    // 国家の切替(コマンド)の後に、古いボタンが押された: 国家2の操作として実行されない。
    send(&mut app, SessionCommand::SetActor(Some(2)));
    press(&mut app, farm);
    assert_eq!(queue_ids(&app, 1), vec![]);
    assert_eq!(queue_ids(&app, 2), vec![]);
    assert_eq!(treasury(&app, 1), 5000.0);
    assert_eq!(treasury(&app, 2), before_t2);
    assert!(last_log(&app).1.contains("操作国家が変わっています(操作時: 国家1 / 現在: 国家2)"), "{}", last_log(&app).1);
    // ボタンが保持する対象は、押した時点のステート・施設のまま(選択が変わっても別のステートへ移らない)。
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    let mine = build_button(&mut app, Mine);
    let held = build_buttons(&mut app).into_iter().find(|(e, ..)| *e == mine).unwrap();
    assert_eq!((held.1, held.2, held.3), (1, MapStateId(20), Mine));
    select(&mut app, state_target(21, vec![2]));
    *app.world_mut().get_mut::<Interaction>(mine).unwrap() = Interaction::Pressed;
    app.update();
    // 古いボタンは(選択が変わった同じフレームで)押されても、ステート20の注文として共通APIが処理する(国家1のステート20)。
    assert!(queue_ids(&app, 2).is_empty(), "never ordered for the newly selected state / country");
    assert!(queue_ids(&app, 1).iter().all(|id| id.country == 1));
}

#[test]
fn an_old_acceptable_check_does_not_let_an_order_through_after_the_state_changed() {
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    let factory = build_button(&mut app, BuildingType::Factory); // 費用1000: 表示時は受理可能
    // 画面を作り直す前に国庫が減る(表示は古いまま)。押した時点の状態で共通APIが再検証して拒否する。
    app.world_mut().resource_mut::<SessionRes>().session.sim.countries[0].treasury = 10.0;
    let before = sim(&app);
    *app.world_mut().get_mut::<Interaction>(factory).unwrap() = Interaction::Pressed;
    app.update();
    assert!(last_log(&app).1.contains("国庫が足りません"), "{}", last_log(&app).1);
    assert_eq!(sim(&app), before, "no payment and no queue entry");
    // 重複: 表示後に同じ施設が先に注文されていれば、古いボタンは重複として拒否される。
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    let farm = build_button(&mut app, Farm);
    app.world_mut().resource_mut::<SessionRes>().session.request_build(Some(1), MapStateId(20), Farm).unwrap();
    let before = sim(&app);
    *app.world_mut().get_mut::<Interaction>(farm).unwrap() = Interaction::Pressed;
    app.update();
    assert_eq!(last_log(&app).1, "発注できません: 同じ施設の注文が建設キューにあります");
    assert_eq!(sim(&app), before);
}

#[test]
fn other_countries_orders_are_view_only_and_cannot_be_cancelled() {
    let mut app = ui_app();
    start(&mut app, 1, 21, 2);
    let id = app.world_mut().resource_mut::<SessionRes>().session.request_build(Some(2), MapStateId(21), Mine).unwrap();
    app.world_mut().resource_mut::<SessionUi>().log.clear(); // 変更検知で作り直させる
    app.update();
    {
        let r = app.world().resource::<SessionRes>();
        let m = build_panel_model(&r.session, Some(1), &state_target(21, vec![2]), &Default::default());
        assert_eq!(m.queues.len(), 2);
        assert!(m.queues[1].title.contains("閲覧のみ") && m.queues[1].rows[0].order == id && m.queues[1].rows[0].cancel_by.is_none());
    }
    assert!(cancel_buttons(&mut app).is_empty(), "no cancel button for another country's order");
    let before = sim(&app);
    send(&mut app, SessionCommand::Cancel { actor: 1, order: id });
    app.update();
    assert_eq!(last_log(&app).1, "取り消せません: 他国家の注文は取り消せません");
    assert_eq!(sim(&app), before);
}

// ---------------------------------------------------------------- 受付順

#[test]
fn orders_cancels_and_days_are_applied_in_arrival_order_and_never_interrupt_a_multi_day_advance() {
    let mut app = ui_app();
    send(&mut app, SessionCommand::SetActor(Some(1)));
    let id = ConstructionOrderId { country: 1, seq: 1 };
    for cmd in [
        SessionCommand::Build { actor: 1, state: MapStateId(20), building: Farm },
        SessionCommand::AdvanceDays(5),
        SessionCommand::Cancel { actor: 1, order: id },
        SessionCommand::AdvanceDays(30),
        SessionCommand::Build { actor: 1, state: MapStateId(20), building: Mine },
        SessionCommand::AdvanceDays(1),
    ] {
        send(&mut app, cmd);
    }
    app.update();
    let mut direct = session();
    direct.request_build(Some(1), MapStateId(20), Farm).unwrap();
    for _ in 0..5 {
        direct.advance_day();
    }
    direct.cancel_order(Some(1), id).unwrap();
    for _ in 0..30 {
        direct.advance_day();
    }
    direct.request_build(Some(1), MapStateId(20), Mine).unwrap(); // 30日の進行の後(途中ではない)
    direct.advance_day();
    assert_eq!(sim(&app), direct.sim);
    assert_eq!(sim(&app).date, SimDate { year: 1800, month: 2, day: 6 });
    // 同じコマンドを別のフレームに分けても同じ結果(フレーム数に依存しない)。
    let mut app2 = ui_app();
    send(&mut app2, SessionCommand::SetActor(Some(1)));
    let cmds = [
        SessionCommand::Build { actor: 1, state: MapStateId(20), building: Farm },
        SessionCommand::AdvanceDays(5),
        SessionCommand::Cancel { actor: 1, order: id },
        SessionCommand::AdvanceDays(30),
        SessionCommand::Build { actor: 1, state: MapStateId(20), building: Mine },
        SessionCommand::AdvanceDays(1),
    ];
    for c in cmds {
        send(&mut app2, c);
        app2.update();
        app2.update();
    }
    assert_eq!(sim(&app2), direct.sim);
}

#[test]
fn applying_commands_directly_matches_the_screen_path_for_every_outcome() {
    // 画面のコマンド適用(apply_command)の結果が、共通APIの直接呼び出しと一致し、文言に変換できる。
    let mut s = session();
    let mut actor = Some(1);
    let out = apply_command(&mut s, &mut actor, SessionCommand::Build { actor: 1, state: MapStateId(20), building: Farm });
    let CommandOutcome::Built { id, cost, target_level, required_progress, .. } = out.clone() else { panic!("{out:?}") };
    assert_eq!((cost, target_level, required_progress), (200.0, 4, 30.0));
    let (ok, text) = outcome_text(&out, &s, &Default::default());
    assert!(ok && text.contains("発注しました"));
    let out = apply_command(&mut s, &mut actor, SessionCommand::Cancel { actor: 1, order: id });
    let CommandOutcome::Cancelled(r) = &out else { panic!("{out:?}") };
    assert_eq!((r.paid_cost, r.refund), (200.0, 100.0));
    let again = apply_command(&mut s, &mut actor, SessionCommand::Cancel { actor: 1, order: id });
    assert_eq!(
        again,
        CommandOutcome::CancelRejected(CancelRequestError::Cancel(CancelError::AlreadyCancelled(id)))
    );
    let none = apply_command(&mut s, &mut None, SessionCommand::Build { actor: 1, state: MapStateId(20), building: Farm });
    assert_eq!(none, CommandOutcome::ActorMismatch { commanded: 1, current: None });
    let mut a2 = Some(1);
    let ai = apply_command(&mut s, &mut a2, SessionCommand::Build { actor: 3, state: MapStateId(22), building: Farm });
    assert_eq!(ai, CommandOutcome::ActorMismatch { commanded: 3, current: Some(1) });
    // 共通APIが返す操作主体の拒否(AI国家)もそのまま伝わる。
    let mut a3 = Some(3);
    let ai = apply_command(&mut s, &mut a3, SessionCommand::Build { actor: 3, state: MapStateId(22), building: Farm });
    assert_eq!(ai, CommandOutcome::BuildRejected(BuildRequestError::Actor(ActorError::NotManual(3))));
}

// ---------------------------------------------------------------- 長い一覧・入力

#[test]
fn a_long_queue_keeps_every_row_and_cancel_button_inside_a_scrollable_body() {
    let mut app = ui_app();
    app.world_mut().resource_mut::<SessionRes>().session.sim.countries[0].treasury = 100_000.0;
    start(&mut app, 1, 20, 1);
    for b in BuildingType::ALL {
        send(&mut app, SessionCommand::Build { actor: 1, state: MapStateId(20), building: b });
    }
    app.update();
    let ids = queue_ids(&app, 1);
    assert!(ids.len() >= 8, "{}", ids.len());
    let buttons = cancel_buttons(&mut app);
    assert_eq!(buttons.len(), ids.len(), "no row or button is dropped");
    let mut seen: Vec<_> = buttons.iter().map(|(_, _, o)| *o).collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), ids.len(), "each button has a distinct real order id");
    // 一覧はスクロール本体(縦スクロール+カーソル位置)の中にあり、パネル全体は Button(入力を地図へ渡さない)。
    let list = app.world_mut().query_filtered::<Entity, With<BuildListRoot>>().single(app.world()).unwrap();
    let parent = app.world().get::<ChildOf>(list).unwrap().parent();
    let node = app.world().get::<Node>(parent).unwrap();
    assert_eq!(node.overflow.y, OverflowAxis::Scroll);
    assert!(app.world().get::<ScrollPosition>(parent).is_some() && app.world().get::<RelativeCursorPosition>(parent).is_some());
    let root = app.world_mut().query_filtered::<Entity, With<BuildPanelRoot>>().single(app.world()).unwrap();
    assert!(app.world().get::<Button>(root).is_some() && app.world().get::<Interaction>(root).is_some());
    // すべての行のボタンは Button(Interaction を発行する)。
    for (e, ..) in &buttons {
        assert!(app.world().get::<Interaction>(*e).is_some());
    }
}

#[test]
fn the_build_panel_can_be_hidden_without_touching_the_economy() {
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    let root = app.world_mut().query_filtered::<Entity, With<BuildPanelRoot>>().single(app.world()).unwrap();
    assert_ne!(app.world().get::<Node>(root).unwrap().display, Display::None);
    let before = sim(&app);
    send(&mut app, SessionCommand::ToggleBuildPanel);
    app.update();
    assert_eq!(app.world().get::<Node>(root).unwrap().display, Display::None);
    assert_eq!(sim(&app), before);
    send(&mut app, SessionCommand::ToggleBuildPanel);
    app.update();
    assert_ne!(app.world().get::<Node>(root).unwrap().display, Display::None);
}

#[test]
fn the_build_ui_never_writes_files() {
    for file in ["src/hexmap/build_view.rs", "src/hexmap/economy_view.rs"] {
        let src = fs::read_to_string(file).unwrap();
        for banned in ["fs::write", "File::create", "create_dir", ".save(", "map_economy_file::save", "OpenOptions", "remove_file"] {
            assert!(!src.contains(banned), "{file}: S2 must not write files ({banned})");
        }
    }
    let before: Vec<_> = fs::read_dir("saves").unwrap().map(|e| e.unwrap().file_name()).collect();
    let mut app = ui_app();
    start(&mut app, 1, 20, 1);
    let farm = build_button(&mut app, Farm);
    press(&mut app, farm);
    send(&mut app, SessionCommand::AdvanceDays(30));
    app.update();
    let after: Vec<_> = fs::read_dir("saves").unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(before, after);
    assert!(!std::path::Path::new("saves/map_economy").exists());
}
