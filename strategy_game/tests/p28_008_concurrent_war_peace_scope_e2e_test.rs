//! P28-008: 複数戦争下の講和処理の限定(候補B)の実App統合テスト。
//!
//! 実データ(6か国28州の本番マップ)を`AppPlugin`経由で読み込み、実Systemの
//! `handle_daily_war_resolution`(降伏判定・E1・E2・講和)を通して検証する。
//!
//! - 同日に2件の降伏条件が成立しても、`WarRegistry`の`HashMap`反復順(インスタンスごとに
//!   ランダム)によらず両Warの勝利が保たれる。
//! - 標準シナリオ(P28-008-PRE-001で実測)のwar3(E2による白紙講和, 1801/03/22)が、
//!   Active中のwar2の占領州s9を消さない。
//! - 講和の直前・直後にsave→load(2回連続を含む)を挟んでも、連続実行と一致する。
//!
//! 自然運用の検証は固定された標準シナリオ1本(全AI・プレイヤー操作なし)である。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use std::fmt::Write as _;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, StateId, WarId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::diplomacy::crisis::{WarGoal, WarGoalType};
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
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
use strategy_game::war::capitulation::{CapitulationResult, evaluate_war_capitulation};
use strategy_game::war::data::{War, WarRegistry, WarStatus};

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p28_008_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
}

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_app(dir: &std::path::Path) -> App {
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
    app.insert_resource(PlayerCountry(None));
    app.insert_state(GameState::Playing);
    app.update();
    app
}

/// 実暦(`GameDate`)を1日進め、`DayChangedMessage`(月が変わる日は`MonthChangedMessage`も同一
/// フレームで)を送って実Systemを1日分動かす。
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

fn send_save_request(app: &mut App) {
    let mut s: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut())
        .unwrap()
        .write(SaveRequestMessage);
    s.apply(app.world_mut());
    app.update();
}

fn send_load_request(app: &mut App) {
    let mut s: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut())
        .unwrap()
        .write(LoadRequestMessage);
    s.apply(app.world_mut());
    app.update();
    app.update();
}

fn war_with_goal(att: CountryId, def: CountryId, target: StateId) -> War {
    War {
        id: WarId(0), // add_warが上書きする
        name: "P28-008 same-day capitulation".to_string(),
        attackers: [att].into_iter().collect(),
        defenders: [def].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![WarGoal {
            attacker: att,
            defender: def,
            goal_type: WarGoalType::ConquerState,
            target_states: vec![target],
            base_peace_cost: 20.0,
            international_concern: 10.0,
            completion: 0.0,
            is_primary: true,
        }],
        start_date: "1799/12/01".to_string(),
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

// ─────────────────────────────────────────────────────────────────────────
// 同日2件の降伏(実`handle_daily_war_resolution`)
// ─────────────────────────────────────────────────────────────────────────

/// 実マップで、Elfin(c1)に対しArcadia(c0)とDwarf(c2)がそれぞれElfinの別の州を目標に宣戦し、
/// 両方とも目標州を占領済み・Elfinは戦闘可能な陸軍なし(新規募兵も不可)という状態を作り、
/// 1日だけ実Systemで進める。戻り値は (WarAの状態, WarBの状態, 目標A所有国, 目標B所有国)。
fn run_two_same_day_capitulations() -> (WarStatus, WarStatus, CountryId, CountryId) {
    let dir = TempDir(unique_temp_dir("two_cap"));
    let mut app = setup_app(&dir.0);
    let (x, y, z) = (CountryId(0), CountryId(1), CountryId(2));

    let (target_a, target_b) = {
        let sr = app.world().resource::<StateRegistry>();
        let mut owned: Vec<StateId> = sr
            .states
            .iter()
            .filter(|s| !s.is_sea && s.owner_country_id == y)
            .map(|s| s.id)
            .collect();
        owned.sort_by_key(|s| s.0);
        assert!(owned.len() >= 2, "実マップのElfinは陸州を2つ以上持つ前提");
        (owned[0], owned[1])
    };
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        let ids: Vec<_> = mr
            .divisions
            .values()
            .filter(|d| d.owner == y)
            .map(|d| d.id)
            .collect();
        for id in ids {
            mr.remove_division(id);
        }
    }
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(y)
        .unwrap()
        .available_manpower = 0;
    {
        let mut sr = app.world_mut().resource_mut::<StateRegistry>();
        sr.get_mut(target_a).unwrap().controller_country = Some(x);
        sr.get_mut(target_b).unwrap().controller_country = Some(z);
    }
    let (a, b) = {
        let mut wr = app.world_mut().resource_mut::<WarRegistry>();
        (
            wr.add_war(war_with_goal(x, y, target_a)),
            wr.add_war(war_with_goal(z, y, target_b)),
        )
    };

    // 前提: 実行前の時点で両Warとも降伏条件を満たす(順序によって片方だけが失われる状況)。
    {
        let wr = app.world().resource::<WarRegistry>();
        let sr = app.world().resource::<StateRegistry>();
        let mr = app.world().resource::<MilitaryRegistry>();
        for id in [a, b] {
            assert_eq!(
                evaluate_war_capitulation(&wr.wars[&id], sr, mr),
                CapitulationResult::DefenderCapitulated
            );
        }
    }

    advance_one_day(&mut app);

    let wr = app.world().resource::<WarRegistry>();
    let sr = app.world().resource::<StateRegistry>();
    (
        wr.wars[&a].status,
        wr.wars[&b].status,
        sr.get(target_a).unwrap().owner_country_id,
        sr.get(target_b).unwrap().owner_country_id,
    )
}

#[test]
fn real_handler_same_day_capitulations_both_win_for_every_registry_hash_order() {
    // `WarRegistry.wars`(std HashMap)の反復順はインスタンスごとにランダムなため、新しいAppを
    // 繰り返し作ることで両方の処理順を(確率的に)網羅する。旧実装では片方が必ず負けた。
    for i in 0..12 {
        let (sa, sb, oa, ob) = run_two_same_day_capitulations();
        assert_eq!(sa, WarStatus::AttackerVictory, "iteration {i}: War A");
        assert_eq!(sb, WarStatus::AttackerVictory, "iteration {i}: War B");
        assert_eq!(oa, CountryId(0), "iteration {i}: 目標AはArcadiaへ割譲");
        assert_eq!(ob, CountryId(2), "iteration {i}: 目標BはDwarfへ割譲");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 標準シナリオのE2(war3)と他Warの占領 / save→load
// ─────────────────────────────────────────────────────────────────────────

fn war_line(app: &App, id: usize) -> String {
    let wr = app.world().resource::<WarRegistry>();
    match wr.wars.get(&WarId(id)) {
        Some(w) => format!(
            "{:?}|{}|{}",
            w.status,
            w.end_date.clone().unwrap_or_default(),
            w.end_reason.clone().unwrap_or_default()
        ),
        None => "absent".to_string(),
    }
}

fn controller_of(app: &App, sid: usize) -> usize {
    app.world()
        .resource::<StateRegistry>()
        .get(StateId(sid))
        .unwrap()
        .controller()
        .0
}

/// 状態の決定論的な文字列化(War・州・Division)。
fn world_snapshot(app: &App) -> String {
    let mut s = String::new();
    let wr = app.world().resource::<WarRegistry>();
    let mut ids: Vec<_> = wr.wars.keys().copied().collect();
    ids.sort_by_key(|i| i.0);
    for id in ids {
        let w = &wr.wars[&id];
        let _ = writeln!(
            s,
            "war{} {:?} {:?} {:?} {:?}",
            id.0, w.status, w.winner, w.end_date, w.end_reason
        );
    }
    for st in &app.world().resource::<StateRegistry>().states {
        let _ = writeln!(
            s,
            "s{} o{} c{}",
            st.id.0,
            st.owner_country_id.0,
            st.controller().0
        );
    }
    let mr = app.world().resource::<MilitaryRegistry>();
    let mut dv: Vec<_> = mr.divisions.values().collect();
    dv.sort_by_key(|d| d.id.0);
    for d in dv {
        let _ = writeln!(
            s,
            "div{} c{} s{} {:?} mp{}",
            d.id.0, d.owner.0, d.current_state.0, d.status, d.manpower
        );
    }
    s
}

/// 標準シナリオを`stop_date`(その日のtick後)まで進める。`hook`は各日のtick後に呼ばれる。
fn run_standard_scenario(stop_date: &str, mut hook: impl FnMut(&mut App, &str)) -> (App, TempDir) {
    let dir = TempDir(unique_temp_dir("natural"));
    let mut app = setup_app(&dir.0);
    loop {
        advance_one_day(&mut app);
        let d = date(&app);
        hook(&mut app, &d);
        if d == stop_date {
            break;
        }
    }
    (app, dir)
}

#[test]
fn standard_scenario_e2_peace_of_war3_does_not_erase_war2s_conquest() {
    // 【P28-010-IMPL-001でWarIdを更新】: 生産入力の原子的会計で、戦争の発生順が変わり、
    // 旧WarId(3)(att c5/def c3、1801/03/02開戦)は新WarId(1)(同じ組み合わせ、1800/08/03
    // 開戦、終結は同じく1801/03/22のE2)になった。旧WarId(2)(att c2/def c3)は同じWarId(2)
    // のまま、開戦1801/03/02・終結1801/04/06・敗者降伏で一致する。検証対象の因果
    // (E2の講和がActiveなwar2のs9占領を消さない)は同じ実データ上で引き続き再現する。
    // P28-008-PRE-001の実測(標準シナリオ): war3(att c5/def c3)は1801/03/22にE2で白紙講和。
    // 旧実装では、同日にその所有国c3のs9をActiveのwar2(att c2/def c3)が占領中であるのに、
    // 正常化がs9のc2占領を消してしまい、war2の終結が04/06→04/12へ遅れた。
    // 【P29-004(E3: 生産能力に基づく AI 募兵制限)で日付を更新】: 募兵が生産能力に制限され、
    // 戦争の開戦・終結日が変わった。同じ組み合わせ(att c5/def c3 = WarId(1)、att c2/def c3 =
    // WarId(2))は、E2 の白紙講和が 1801/03/22 → 1801/05/18、war2 の降伏が 1801/04/06 →
    // 1801/06/01 になる。検証対象の因果(E2 の講和が Active な war2 の s9 占領を消さない)は、
    // 同じ実データ上で引き続き再現する。
    // 【P29-005(複数月の実産出に基づく能力上限)で日付と WarId を更新】: 同じ組み合わせ(att c5/def c3 =
    // WarId(1)、att c2/def c3 = WarId(3))は、E2 の白紙講和が 1801/05/18、降伏が 1801/06/01 になる
    // (c2 対 c3 の戦争は、Elfin 対 Arcadia の戦争が先に開戦したため WarId(3) になる)。因果は同じ。
    let mut s9_before = None;
    let mut s9_on_e2_day = None;
    let mut war2_end_seen = None;
    let (app, _dir) = run_standard_scenario("1801/06/04", |app, d| {
        match d {
            "1801/05/17" => s9_before = Some(controller_of(app, 9)),
            "1801/05/18" => s9_on_e2_day = Some(controller_of(app, 9)),
            _ => {}
        }
        if war2_end_seen.is_none() && war_line(app, 3).starts_with("AttackerVictory") {
            war2_end_seen = Some(d.to_string());
        }
    });

    assert_eq!(
        war_line(&app, 1),
        "WhitePeace|1801/05/18|Frontline Unreachable",
        "同じ対Oceanic戦(att c5/def c3 = WarId(1))はP28-007のE2で05/18に終結する"
    );
    assert_eq!(s9_before, Some(2), "前提: c2(Dwarf)が05/17時点でs9を占領中");
    assert_eq!(
        s9_on_e2_day,
        Some(2),
        "E2の講和でwar2のs9占領が消えてはならない"
    );
    assert_eq!(
        war_line(&app, 3),
        "AttackerVictory|1801/06/01|Defender Capitulation",
        "war2はE2講和の影響を受けず06/01に降伏勝利する"
    );
    assert_eq!(war2_end_seen.as_deref(), Some("1801/06/01"));
}

#[test]
fn save_load_around_the_peace_matches_continuous_run() {
    let (continuous, _d0) = run_standard_scenario("1801/06/04", |_, _| {});
    let expected = world_snapshot(&continuous);

    // 講和(03/22)の直前でsave→load→(2回連続)load。
    let mut done_before = false;
    let (before, _d1) = run_standard_scenario("1801/06/04", |app, d| {
        if d == "1801/05/17" && !done_before {
            done_before = true;
            send_save_request(app);
            send_load_request(app);
            send_load_request(app);
        }
    });
    assert!(done_before);
    assert_eq!(world_snapshot(&before), expected, "講和直前のsave→load×2");

    // 講和(03/22)の直後でsave→load。
    let mut done_after = false;
    let (after, _d2) = run_standard_scenario("1801/06/04", |app, d| {
        if d == "1801/05/18" && !done_after {
            done_after = true;
            send_save_request(app);
            send_load_request(app);
        }
    });
    assert!(done_after);
    assert_eq!(world_snapshot(&after), expected, "講和直後のsave→load");
}
