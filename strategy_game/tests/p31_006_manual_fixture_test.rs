//! P31-006: プレイヤー自身の戦争・講和・勝敗を実画面で確認するための、手動確認用 Save の生成。
//!
//! 3 つの環境を作る。いずれも**人工的な確認条件**であり、ユーザーの画面操作で戦争・講和・勝敗を行う。
//! - `player_war`: Dwarf(プレイヤー)が、実際の募兵経路で師団 3 個を持つ 1800/02/01。戦争・正当化は未開始。
//! - `victory`: 他 4 国が消滅済みで、Arcadia が最後の 1 州(Arcadia Capital)を Dwarf に占領された状態
//!   (Dwarf が攻撃側の戦争)。ロード後、通常の日次処理(降伏判定→州の移転→国家消滅→勝利検知)で勝利が成立する。
//! - `defeat`: Dwarf が最後の 1 州(Dwarf Mining Region)を Arcadia に占領された状態(Arcadia が攻撃側の戦争)。
//!   ロード後、通常の日次処理で Dwarf が消滅し、敗北が成立する。
//!
//! 勝敗フラグ・表示状態は直接設定しない。国家消滅は実際の国家消滅 Domain API(`apply_country_elimination`)を使う。
//! 生成後、Save ファイルを新しい App に Load し、fixture の維持なしで期待どおり進むことを検査する。
//! ユーザーの Save(`saves/savegame_v1.ron`)は読み書きしない。通常のテスト実行では動かさない(`#[ignore]`)。
//! 実行: `cargo test --release --test p31_006_manual_fixture_test -- --ignored --nocapture`
//! 生成先: `target/p31_006_fixture/{player_war,victory,defeat}/savegame_v1.ron`

#![allow(dead_code, unused_imports)]

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionDefinitionId, DivisionId, FrontlineId, StateId, WarId};
use strategy_game::country::power::CountryPowerRegistry;
use strategy_game::country::{
    CountryPlugin, CountryRegistry, PlayerCountry, PlayerEliminationMode, PlayerVictoryMode,
};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::diplomacy::claims::ClaimRegistry;
use strategy_game::diplomacy::crisis::{CrisisRegistry, WarGoal, WarGoalType};
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::{FrontlineRegistry, FrontlineStance};
use strategy_game::war::justification::WarJustificationRegistry;
use strategy_game::war::peace::{
    PeaceOffer, PeaceTerm, can_accept_peace_offer, execute_peace_settlement,
};

const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const DWARF: CountryId = CountryId(2);
const OCEANIC: CountryId = CountryId(3);
const PASSGUARD: CountryId = CountryId(4);
const FERROWYN: CountryId = CountryId(5);
const ARCADIA_CAPITAL: StateId = StateId(0);
const DWARF_CAPITAL: StateId = StateId(5);
/// player_war の戦争目標。Dwarf の首都に隣接する Arcadia の州(Western Mage Province)。
const WAR_GOAL: StateId = StateId(2);

fn out_dir(name: &str) -> std::path::PathBuf {
    let d = std::path::PathBuf::from("target/p31_006_fixture").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn setup_app(dir: &std::path::Path, player: Option<CountryId>) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(AppPlugin)
        .add_plugins(CountryPlugin)
        .add_plugins(StatePlugin)
        .add_plugins(BuildingPlugin)
        .add_plugins(EconomyPlugin)
        .add_plugins(ResearchPlugin)
        .add_plugins(PoliticsPlugin)
        .add_plugins(DiplomacyPlugin)
        .add_plugins(WarPlugin)
        .add_plugins(MilitaryPlugin)
        .add_plugins(PopulationPlugin);
    app.insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
        .insert_resource(strategy_game::map::division_selection::DragSelectState::default())
        .insert_resource(SelectedState::default())
        .insert_resource(strategy_game::ui::ActivePanel::default())
        .insert_resource(strategy_game::ui::diplomacy_panel::DiplomacyPanelState::default())
        .insert_resource(strategy_game::ui::military_panel::MilitaryPanelState::default())
        .insert_resource(strategy_game::ui::peace_panel::PeacePanelState::default())
        .insert_resource(strategy_game::ui::politics_panel::PoliticsPanelState::default())
        .insert_resource(strategy_game::ui::research_panel::ResearchPanelState::default())
        .insert_resource(strategy_game::map::camera::CameraDragState::default())
        .add_message::<strategy_game::map::camera::CameraResetRequestMessage>();
    app.add_plugins(SaveGamePlugin);
    app.add_plugins(LoadGamePlugin);
    app.insert_resource(SaveFileConfig {
        path: SavePathConfig {
            final_path: dir.join("savegame_v1.ron"),
        },
    });
    app.update();
    app.insert_resource(PlayerCountry(player));
    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn advance_one_day(app: &mut App) {
    let (old_month, next) = {
        let d = app.world().resource::<GameDate>();
        (d.month, d.add_days(1))
    };
    {
        let mut d = app.world_mut().resource_mut::<GameDate>();
        d.year = next.year;
        d.month = next.month;
        d.day = next.day;
    }
    let mut s: SystemState<(
        MessageWriter<DayChangedMessage>,
        MessageWriter<MonthChangedMessage>,
    )> = SystemState::new(app.world_mut());
    {
        let (mut dw, mut mw) = s.get_mut(app.world_mut()).unwrap();
        dw.write(DayChangedMessage {
            year: next.year,
            month: next.month,
            day: next.day,
        });
        if next.month != old_month {
            mw.write(MonthChangedMessage {
                year: next.year,
                month: next.month,
            });
        }
    }
    s.apply(app.world_mut());
    app.update();
}

fn date(app: &App) -> String {
    app.world().resource::<GameDate>().display()
}

fn send_save(app: &mut App) {
    let mut s: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut())
        .unwrap()
        .write(SaveRequestMessage);
    s.apply(app.world_mut());
    app.update();
}

fn send_load(app: &mut App) {
    let mut s: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut())
        .unwrap()
        .write(LoadRequestMessage);
    s.apply(app.world_mut());
    app.update();
    app.update();
}

fn load_into_fresh_app(dir: &std::path::Path, player: CountryId) -> App {
    let mut app = setup_app(dir, Some(player));
    send_load(&mut app);
    app
}

// ---- プレイヤーの画面操作と同じ Domain 操作 ----

type JustifyParams<'w> = (
    Res<'w, CountryRegistry>,
    Res<'w, StateRegistry>,
    ResMut<'w, DiplomacyRegistry>,
    ResMut<'w, WarJustificationRegistry>,
    ResMut<'w, WarRegistry>,
);

/// 外交パネルの「戦争を正当化」ボタン。
fn player_justify(app: &mut App, who: CountryId, target: CountryId, st: StateId) {
    let d = date(app);
    let mut s: SystemState<JustifyParams> = SystemState::new(app.world_mut());
    let (cr, sr, dr, mut jr, _wr) = s.get_mut(app.world_mut()).unwrap();
    let r = jr.start_justification(who, target, st, d, &cr, &sr, &dr);
    assert!(r.is_ok(), "正当化を開始できる: {r:?}");
}

/// 外交パネルの「宣戦布告」ボタン。
fn player_declare(
    app: &mut App,
    who: CountryId,
    target: CountryId,
    st: StateId,
) -> Result<WarId, &'static str> {
    let d = date(app);
    let mut s: SystemState<JustifyParams> = SystemState::new(app.world_mut());
    let (cr, sr, mut dr, mut jr, mut wr) = s.get_mut(app.world_mut()).unwrap();
    wr.declare_war(who, target, st, d, &cr, &sr, &mut dr, &mut jr)
}

/// 軍事パネルの「募兵」ボタン(`request_recruitment`)。
fn player_recruit(app: &mut App, who: CountryId, def: usize, st: StateId) {
    use strategy_game::military::recruitment::request_recruitment;
    let mut s: SystemState<(ResMut<CountryRegistry>, Res<MilitaryRegistry>)> =
        SystemState::new(app.world_mut());
    let (mut cr, mr) = s.get_mut(app.world_mut()).unwrap();
    let r = request_recruitment(cr.get_mut(who).unwrap(), &mr, DivisionDefinitionId(def), st);
    assert!(r.is_ok(), "募兵できる: {r:?}");
}

/// 軍事パネルの「選択部隊を割当」「攻勢」ボタン。
fn player_order_offensive(app: &mut App, flid: FrontlineId, who: CountryId, divs: Vec<DivisionId>) {
    let mut s: SystemState<(
        Res<MilitaryRegistry>,
        Res<WarRegistry>,
        ResMut<FrontlineRegistry>,
    )> = SystemState::new(app.world_mut());
    let (mr, wr, mut fr) = s.get_mut(app.world_mut()).unwrap();
    for d in divs {
        let r = fr.assign_division(d, flid, who, &mr, &wr);
        assert!(r.is_ok(), "割当できる: {r:?}");
    }
    fr.get_plan_mut(flid, who).unwrap().stance = FrontlineStance::Offensive;
}

type PeaceParams<'w> = (
    ResMut<'w, StateRegistry>,
    ResMut<'w, WarRegistry>,
    ResMut<'w, MilitaryRegistry>,
    ResMut<'w, BattleRegistry>,
    ResMut<'w, DiplomacyRegistry>,
    ResMut<'w, FrontlineRegistry>,
    ResMut<'w, CrisisRegistry>,
    ResMut<'w, ClaimRegistry>,
    ResMut<'w, CountryRegistry>,
    ResMut<'w, ArmyRegistry>,
    ResMut<'w, WarJustificationRegistry>,
    ResMut<'w, CountryPowerRegistry>,
);

/// 講和パネルの講和ボタン(`can_accept_peace_offer` → `execute_peace_settlement`)。
fn player_propose_peace(
    app: &mut App,
    war_id: WarId,
    who: CountryId,
    term: PeaceTerm,
) -> Result<(), &'static str> {
    let d = date(app);
    let mut s: SystemState<PeaceParams> = SystemState::new(app.world_mut());
    let (mut sr, mut wr, mut mr, mut br, mut dr, mut fr, mut crs, mut cl, mut cor, mut ar, mut jr, mut pr) =
        s.get_mut(app.world_mut()).unwrap();
    let war = wr.wars.get(&war_id).unwrap();
    let recipient = if war.attackers.contains(&who) {
        war.primary_defender_id()
    } else {
        war.primary_attacker_id()
    };
    let offer = PeaceOffer {
        war_id,
        proposer_country_id: who,
        recipient_country_id: recipient,
        term,
        created_date: d.clone(),
    };
    can_accept_peace_offer(&offer, &wr, Some(&sr), Some(&mr), Some(&cor), &d)?;
    execute_peace_settlement(
        war_id,
        term,
        &format!("Peace Treaty Accepted: {}", term.display_name()),
        &d,
        &mut sr,
        &mut wr,
        &mut mr,
        &mut br,
        &mut dr,
        &mut fr,
        &mut crs,
        &mut cl,
        &mut cor,
        &mut ar,
        &mut jr,
        &mut pr,
    )
}

type ElimParams<'w> = (
    ResMut<'w, StateRegistry>,
    ResMut<'w, CountryRegistry>,
    ResMut<'w, MilitaryRegistry>,
    ResMut<'w, ArmyRegistry>,
    ResMut<'w, FrontlineRegistry>,
    ResMut<'w, WarRegistry>,
    ResMut<'w, BattleRegistry>,
    ResMut<'w, DiplomacyRegistry>,
    ResMut<'w, CrisisRegistry>,
    ResMut<'w, ClaimRegistry>,
    ResMut<'w, WarJustificationRegistry>,
    ResMut<'w, CountryPowerRegistry>,
);

fn land_states_of(app: &App, c: CountryId) -> Vec<StateId> {
    let sr = app.world().resource::<StateRegistry>();
    let mut v: Vec<StateId> = sr
        .states
        .iter()
        .filter(|s| !s.is_sea && s.owner_country_id == c)
        .map(|s| s.id)
        .collect();
    v.sort_by_key(|s| s.0);
    v
}

/// 州の所有を移す(講和の領土移転と同じ `transfer_state_ownership`)。
fn transfer_state(app: &mut App, st: StateId, from: CountryId, to: CountryId) {
    let mut s: SystemState<(ResMut<StateRegistry>, ResMut<CountryRegistry>)> =
        SystemState::new(app.world_mut());
    let (mut sr, mut cr) = s.get_mut(app.world_mut()).unwrap();
    strategy_game::state::ownership::transfer_state_ownership(st, from, to, &mut sr, &mut cr)
        .unwrap_or_else(|e| panic!("州の移転 {st:?} {from:?}->{to:?}: {e:?}"));
}

/// 国家消滅(実際の `apply_country_elimination`)。最後の 1 州以外を先に併合国へ移し、最後の州で消滅させる。
fn annex_country(app: &mut App, victim: CountryId, annexer: CountryId) {
    let states = land_states_of(app, victim);
    let (last, rest) = states.split_last().unwrap();
    for st in rest {
        transfer_state(app, *st, victim, annexer);
    }
    let d = date(app);
    let mut s: SystemState<ElimParams> = SystemState::new(app.world_mut());
    let (mut sr, mut cr, mut mr, mut ar, mut fr, mut wr, mut br, mut dr, mut crs, mut cl, mut jr, mut pr) =
        s.get_mut(app.world_mut()).unwrap();
    let plan = strategy_game::country::elimination::preflight_country_elimination(
        *last, victim, annexer, &d, &sr, &cr,
    )
    .unwrap_or_else(|e| panic!("消滅の事前検証 {victim:?}: {e:?}"));
    strategy_game::country::elimination::apply_country_elimination(
        plan, None, None, &d, &mut sr, &mut cr, &mut mr, &mut ar, &mut fr, &mut wr, &mut br,
        &mut dr, &mut crs, &mut cl, &mut jr, &mut pr,
    )
    .unwrap_or_else(|e| panic!("国家消滅 {victim:?}: {e:?}"));
}

/// 初期設定として、国の師団・募兵・動員可能人員を取り除く(以降は書き換えない)。
fn disarm(app: &mut App, c: CountryId) {
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        let ids: Vec<_> = mr
            .divisions
            .values()
            .filter(|d| d.owner == c)
            .map(|d| d.id)
            .collect();
        for id in ids {
            mr.remove_division(id);
        }
    }
    let mut cr = app.world_mut().resource_mut::<CountryRegistry>();
    let country = cr.get_mut(c).unwrap();
    country.available_manpower = 0;
    country.recruitment_queue.clear();
}

fn conquest_war(
    name: &str,
    attacker: CountryId,
    defender: CountryId,
    target: StateId,
    start: &str,
) -> War {
    War {
        id: WarId(0), // add_war が上書きする
        name: name.to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![WarGoal {
            attacker,
            defender,
            goal_type: WarGoalType::ConquerState,
            target_states: vec![target],
            base_peace_cost: 20.0,
            international_concern: 10.0,
            completion: 0.0,
            is_primary: true,
        }],
        start_date: start.to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: HashSet::new(),
    }
}

fn set_controller(app: &mut App, st: StateId, by: CountryId) {
    let mut sr = app.world_mut().resource_mut::<StateRegistry>();
    let s = sr.states.iter_mut().find(|s| s.id == st).unwrap();
    s.controller_country = Some(by);
}

fn controller_of(app: &App, st: StateId) -> CountryId {
    app.world()
        .resource::<StateRegistry>()
        .get(st)
        .unwrap()
        .controller()
}

fn owner_of(app: &App, st: StateId) -> CountryId {
    app.world()
        .resource::<StateRegistry>()
        .get(st)
        .unwrap()
        .owner_country_id
}

fn is_eliminated(app: &App, c: CountryId) -> bool {
    app.world()
        .resource::<CountryRegistry>()
        .get(c)
        .unwrap()
        .is_eliminated()
}

fn dwarf_divisions(app: &App) -> Vec<DivisionId> {
    let mut v: Vec<_> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == DWARF)
        .map(|d| d.id)
        .collect();
    v.sort_by_key(|d| d.0);
    v
}

fn active_war_of(app: &App, c: CountryId) -> Option<WarId> {
    app.world()
        .resource::<WarRegistry>()
        .get_active_war_for_country(c)
        .map(|w| w.id)
}

// ---- player_war ----

#[test]
#[ignore = "手動確認用の人工的な Save を生成する(通常のテスト実行では動かさない)"]
fn generate_the_player_war_manual_fixture_save() {
    let dir = out_dir("player_war");
    let mut app = setup_app(&dir, Some(DWARF));
    // 実際の募兵経路(軍事パネルの「募兵」と同じ request_recruitment)で、首都に Standard Infantry を 2 個追加する。
    player_recruit(&mut app, DWARF, 0, DWARF_CAPITAL);
    player_recruit(&mut app, DWARF, 0, DWARF_CAPITAL);
    while date(&app).as_str() < "1800/02/01" {
        advance_one_day(&mut app);
    }
    assert_eq!(dwarf_divisions(&app).len(), 3, "募兵が完成して師団 3 個");
    assert!(
        app.world().resource::<WarRegistry>().wars.is_empty(),
        "戦争はまだない"
    );
    send_save(&mut app);
    let path = dir.join("savegame_v1.ron");
    assert!(path.exists());
    println!(
        "PLAYER_WAR_FIXTURE_SAVE path={} date={} divisions={:?}",
        path.display(),
        date(&app),
        dwarf_divisions(&app)
    );

    // 検証: 新しい App へ Load し、ユーザーと同じ操作(正当化→30 日→宣戦→割当→攻勢→戦況→講和)を Domain 経由で行う。
    let mut app2 = load_into_fresh_app(&dir, DWARF);
    assert_eq!(date(&app2), "1800/02/01");
    assert_eq!(dwarf_divisions(&app2).len(), 3);
    assert!(app2.world().resource::<WarRegistry>().wars.is_empty());
    player_justify(&mut app2, DWARF, ARCADIA, WAR_GOAL);
    for _ in 0..30 {
        advance_one_day(&mut app2);
    }
    let declare_date = date(&app2);
    let wid = player_declare(&mut app2, DWARF, ARCADIA, WAR_GOAL)
        .expect("正当化完了後に宣戦できる");
    advance_one_day(&mut app2);
    let flid = app2
        .world()
        .resource::<FrontlineRegistry>()
        .get_frontline_for_war(wid)
        .expect("前線が作られる")
        .frontline_id;
    let divs = dwarf_divisions(&app2);
    player_order_offensive(&mut app2, flid, DWARF, divs);
    let mut occupied_on = None;
    let mut first_score_on = None;
    for _ in 0..90 {
        advance_one_day(&mut app2);
        if occupied_on.is_none() && controller_of(&app2, WAR_GOAL) == DWARF {
            occupied_on = Some(date(&app2));
        }
        let w = &app2.world().resource::<WarRegistry>().wars[&wid];
        if first_score_on.is_none() && w.war_score > 0.0 {
            first_score_on = Some((date(&app2), w.war_score));
        }
        assert_eq!(
            w.status,
            WarStatus::Active,
            "AI の終戦で戦争が消えない(日付 {})",
            date(&app2)
        );
    }
    let w = &app2.world().resource::<WarRegistry>().wars[&wid];
    println!(
        "PLAYER_WAR_FLOW declared={declare_date} goal_occupied_on={occupied_on:?} first_score={first_score_on:?} score_after_90d={} date={}",
        w.war_score,
        date(&app2)
    );
    assert!(occupied_on.is_some(), "攻勢で戦争目標の州を占領できる");
    // 割譲要求は戦勝点が足りなければ拒否される(通常はまだ 60 未満)。
    let cede = player_propose_peace(&mut app2, wid, DWARF, PeaceTerm::CedeWarGoalRegion);
    println!("PLAYER_WAR_CEDE_ATTEMPT {cede:?}");
    // 白紙講和(開戦 30 日後から)。
    let r = player_propose_peace(&mut app2, wid, DWARF, PeaceTerm::WhitePeace);
    assert!(r.is_ok(), "白紙講和を提案できる: {r:?}");
    let w = &app2.world().resource::<WarRegistry>().wars[&wid];
    assert_eq!(w.status, WarStatus::WhitePeace);
    assert_eq!(owner_of(&app2, WAR_GOAL), ARCADIA, "白紙講和では領土が移らない");
    assert_eq!(controller_of(&app2, WAR_GOAL), ARCADIA, "占領が解除される");
    let d = date(&app2);
    assert!(
        app2.world()
            .resource::<DiplomacyRegistry>()
            .is_in_truce(DWARF, ARCADIA, &d),
        "休戦が成立する"
    );
    assert!(active_war_of(&app2, DWARF).is_none());
    println!(
        "PLAYER_WAR_PEACE white_peace_ok date={d} end_reason={:?}",
        w.end_reason
    );
}

// ---- victory ----

#[test]
#[ignore = "手動確認用の人工的な Save を生成する(通常のテスト実行では動かさない)"]
fn generate_the_victory_manual_fixture_save() {
    let dir = out_dir("victory");
    let mut app = setup_app(&dir, Some(DWARF));
    // 他の 4 国を実際の国家消滅で併合する(Dwarf が併合国)。
    for victim in [ELFIN, OCEANIC, PASSGUARD, FERROWYN] {
        annex_country(&mut app, victim, DWARF);
    }
    // Arcadia の最後の 1 州(首都)以外を Dwarf へ移し、Arcadia を無力にする(初期設定)。
    for st in land_states_of(&app, ARCADIA) {
        if st != ARCADIA_CAPITAL {
            transfer_state(&mut app, st, ARCADIA, DWARF);
        }
    }
    disarm(&mut app, ARCADIA);
    assert_eq!(land_states_of(&app, ARCADIA), vec![ARCADIA_CAPITAL]);
    // Dwarf が攻撃側の戦争。首都を占領済み(降伏判定は次の日次処理で行われる)。
    app.world_mut()
        .resource_mut::<WarRegistry>()
        .add_war(conquest_war(
            "P31-006 victory fixture",
            DWARF,
            ARCADIA,
            ARCADIA_CAPITAL,
            "1800/01/01",
        ));
    set_controller(&mut app, ARCADIA_CAPITAL, DWARF);
    send_save(&mut app);
    let path = dir.join("savegame_v1.ron");
    assert!(path.exists());
    println!(
        "VICTORY_FIXTURE_SAVE path={} date={}",
        path.display(),
        date(&app)
    );

    // 検証: 新しい App に Load。ロード直後はまだ勝利ではなく、通常の日次処理だけで勝利が成立する。
    let mut app2 = load_into_fresh_app(&dir, DWARF);
    let load_date = date(&app2);
    assert_eq!(
        *app2.world().resource::<PlayerVictoryMode>(),
        PlayerVictoryMode::NotConcluded
    );
    assert!(!is_eliminated(&app2, ARCADIA), "ロード直後は Arcadia が存続している");
    for c in [ELFIN, OCEANIC, PASSGUARD, FERROWYN] {
        assert!(is_eliminated(&app2, c));
    }
    assert!(active_war_of(&app2, DWARF).is_some());
    let mut victory_on = None;
    for _ in 0..5 {
        advance_one_day(&mut app2);
        if let PlayerVictoryMode::VictoryOverlayPending { victory_date } =
            app2.world().resource::<PlayerVictoryMode>()
        {
            victory_on = Some(victory_date.clone());
            break;
        }
    }
    println!(
        "VICTORY_FLOW load_date={load_date} victory_on={victory_on:?} arcadia_eliminated={}",
        is_eliminated(&app2, ARCADIA)
    );
    assert!(victory_on.is_some(), "通常の日次処理で勝利が成立する");
    assert!(is_eliminated(&app2, ARCADIA));
    assert_eq!(
        app2.world()
            .resource::<CountryRegistry>()
            .active_country_ids(),
        vec![DWARF]
    );
}

// ---- defeat ----

#[test]
#[ignore = "手動確認用の人工的な Save を生成する(通常のテスト実行では動かさない)"]
fn generate_the_defeat_manual_fixture_save() {
    let dir = out_dir("defeat");
    let mut app = setup_app(&dir, Some(DWARF));
    // Dwarf の最後の 1 州(首都)以外を Arcadia へ移し、Dwarf を無力にする(初期設定)。
    for st in land_states_of(&app, DWARF) {
        if st != DWARF_CAPITAL {
            transfer_state(&mut app, st, DWARF, ARCADIA);
        }
    }
    disarm(&mut app, DWARF);
    assert_eq!(land_states_of(&app, DWARF), vec![DWARF_CAPITAL]);
    // Arcadia が攻撃側の戦争。Dwarf の首都を占領済み(降伏判定は次の日次処理で行われる)。
    app.world_mut()
        .resource_mut::<WarRegistry>()
        .add_war(conquest_war(
            "P31-006 defeat fixture",
            ARCADIA,
            DWARF,
            DWARF_CAPITAL,
            "1800/01/01",
        ));
    set_controller(&mut app, DWARF_CAPITAL, ARCADIA);
    send_save(&mut app);
    let path = dir.join("savegame_v1.ron");
    assert!(path.exists());
    println!(
        "DEFEAT_FIXTURE_SAVE path={} date={}",
        path.display(),
        date(&app)
    );

    let mut app2 = load_into_fresh_app(&dir, DWARF);
    let load_date = date(&app2);
    assert_eq!(
        *app2.world().resource::<PlayerEliminationMode>(),
        PlayerEliminationMode::NotEliminated
    );
    assert!(!is_eliminated(&app2, DWARF));
    assert!(active_war_of(&app2, DWARF).is_some());
    let mut defeat_on = None;
    for _ in 0..5 {
        advance_one_day(&mut app2);
        if *app2.world().resource::<PlayerEliminationMode>()
            == PlayerEliminationMode::DefeatOverlayPending
        {
            defeat_on = Some(date(&app2));
            break;
        }
    }
    println!("DEFEAT_FLOW load_date={load_date} defeat_on={defeat_on:?}");
    assert!(
        defeat_on.is_some(),
        "通常の日次処理で Dwarf が消滅し、敗北が成立する"
    );
    assert!(is_eliminated(&app2, DWARF));
}
