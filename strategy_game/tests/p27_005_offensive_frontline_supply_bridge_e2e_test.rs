//! P27-005: 攻撃側前線補給ブリッジ(`military::daily_supply::is_frontline_supply_eligible`)
//! の実App統合テスト。
//!
//! 資格判定・需要1回計上・Hamilton按分の単体テストは`src/military/daily_supply.rs`にあるため、
//! 本ファイルはP27-005-PRE-001が再現した制御シナリオ(Arcadia対Elfin、実データ)を
//! 実App・実Systemで再現し、監査時に一時パッチで実測した正規値(d10戦闘開始・d11勝利、
//! 回廊遮断の補給0がちょうど25日、等)と一致することを確認する。
//!
//! 複数Division戦闘の`distribute_damage`(実効戦力0の側が被害を受けない別件、
//! P27-006-PRE-001候補)による戦闘の非終結は、本ファイルの合否条件から分離している。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, StateId, WarId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::{ResourceType, military_supplies_value_to_units};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::{Division, DivisionStatus, MilitaryRegistry};
use strategy_game::military::invasion::occupy_state;
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
use strategy_game::war::frontline::{FrontlineRegistry, update_all_frontlines};

const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p27_005_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
}

struct TempTestDir(std::path::PathBuf);
impl TempTestDir {
    fn new(label: &str) -> Self {
        Self(unique_temp_dir(label))
    }
    fn save_file_config(&self) -> SaveFileConfig {
        SaveFileConfig {
            path: SavePathConfig {
                final_path: self.0.join("savegame_v1.ron"),
            },
        }
    }
}
impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_app(save_config: SaveFileConfig) -> App {
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
    app.insert_resource(save_config);
    app.update();
    app.insert_resource(PlayerCountry(None));
    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(SaveRequestMessage);
    state.apply(app.world_mut());
    app.update();
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
    app.update();
}

fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    {
        let mut gd = app.world_mut().resource_mut::<GameDate>();
        gd.year = year;
        gd.month = month;
        gd.day = day;
    }
    let mut state: SystemState<MessageWriter<DayChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
        .write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn no_goal_war() -> War {
    War {
        id: WarId(0),
        name: "P27-005".to_string(),
        attackers: [ARCADIA].into_iter().collect(),
        defenders: [ELFIN].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1799/12/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: std::collections::HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: std::collections::HashSet::new(),
    }
}

fn set_supplies(app: &mut App, c: CountryId, v: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(c)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitarySupplies, v);
}

fn supplies(app: &App, c: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(c)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitarySupplies)
}

struct Cfg {
    with_war: bool,
    attackers: usize,
    extra_defenders: usize,
    remove_defenders: bool,
    cut: Option<(u32, u32)>,
    capital_falls: Option<u32>,
    save_at: Option<u32>,
    days: u32,
}

impl Cfg {
    fn base() -> Self {
        Cfg {
            with_war: true,
            attackers: 4,
            extra_defenders: 0,
            remove_defenders: false,
            cut: None,
            capital_falls: None,
            save_at: None,
            days: 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Summary {
    first_fight_day: Option<u32>,
    fight_days: u32,
    ful0_fight_days: u32,
    ctrl3_to_arcadia_day: Option<u32>,
    final_ctrl3: usize,
    /// (日, 攻撃Divisionの最小fulfillment)のうち戦闘中の日だけ
    fight_min_ful: Vec<(u32, f64)>,
    day10_min_ful: f64,
    day11_min_ful: f64,
    stock_never_negative: bool,
    final_arcadia_supplies_units: u64,
    final_state: Vec<(usize, DivisionStatus, usize, u64)>,
    passguard_units: u64,
}

fn run(c: &Cfg) -> Summary {
    let dir = TempTestDir::new("run");
    let mut app = setup_app(dir.save_file_config());
    if c.with_war {
        app.world_mut()
            .resource_mut::<WarRegistry>()
            .add_war(no_goal_war());
        app.world_mut()
            .resource_scope(|world, mut fr: Mut<FrontlineRegistry>| {
                let w = world.resource::<WarRegistry>();
                let s = world.resource::<StateRegistry>();
                let m = world.resource::<MilitaryRegistry>();
                update_all_frontlines(w, s, m, &mut fr);
            });
    }
    {
        let mut m = app.world_mut().resource_mut::<MilitaryRegistry>();
        let base: Division = m
            .divisions
            .values()
            .find(|d| d.owner == ARCADIA)
            .unwrap()
            .clone();
        for _ in 1..c.attackers {
            m.add_division(base.clone());
        }
        let def: Division = m
            .divisions
            .values()
            .find(|d| d.owner == ELFIN)
            .unwrap()
            .clone();
        for _ in 0..c.extra_defenders {
            m.add_division(def.clone());
        }
        if c.remove_defenders {
            let ids: Vec<_> = m
                .divisions
                .values()
                .filter(|d| d.owner == ELFIN)
                .map(|d| d.id)
                .collect();
            for id in ids {
                m.divisions.remove(&id);
            }
        }
    }
    set_supplies(&mut app, ARCADIA, 1_000_000.0);
    set_supplies(&mut app, ELFIN, 1_000_000.0);

    let (mut year, mut month, mut day) = (1800, 1u8, 2u8);
    let mut s = Summary {
        first_fight_day: None,
        fight_days: 0,
        ful0_fight_days: 0,
        ctrl3_to_arcadia_day: None,
        final_ctrl3: 99,
        fight_min_ful: vec![],
        day10_min_ful: -1.0,
        day11_min_ful: -1.0,
        stock_never_negative: true,
        final_arcadia_supplies_units: 0,
        final_state: vec![],
        passguard_units: 0,
    };
    for n in 1..=c.days {
        if let Some((cut, recon)) = c.cut {
            if n == cut {
                let mut sr = app.world_mut().resource_mut::<StateRegistry>();
                occupy_state(StateId(1), ELFIN, &mut sr);
                occupy_state(StateId(4), ELFIN, &mut sr);
            }
            if n == recon {
                let mut sr = app.world_mut().resource_mut::<StateRegistry>();
                occupy_state(StateId(1), ARCADIA, &mut sr);
                occupy_state(StateId(4), ARCADIA, &mut sr);
            }
        }
        if c.capital_falls == Some(n) {
            let mut sr = app.world_mut().resource_mut::<StateRegistry>();
            occupy_state(StateId(0), ELFIN, &mut sr);
        }
        if c.save_at == Some(n) {
            send_save_request(&mut app);
            send_load_request(&mut app);
        }
        tick_day(&mut app, year, month, day);
        day += 1;
        if day > 28 {
            day = 1;
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
        if supplies(&app, ARCADIA) < 0.0 {
            s.stock_never_negative = false;
        }
        let mil = app.world().resource::<MilitaryRegistry>();
        let sr = app.world().resource::<StateRegistry>();
        let fighting: Vec<&Division> = mil
            .divisions
            .values()
            .filter(|d| d.owner == ARCADIA && d.status == DivisionStatus::Fighting)
            .collect();
        let min_ful = fighting
            .iter()
            .map(|d| mil.supply_fulfillment_of(d.id))
            .fold(f64::INFINITY, f64::min);
        if !fighting.is_empty() {
            s.fight_days += 1;
            s.first_fight_day.get_or_insert(n);
            if min_ful < 0.001 {
                s.ful0_fight_days += 1;
            }
            s.fight_min_ful.push((n, min_ful));
            if n == 10 {
                s.day10_min_ful = min_ful;
            }
        }
        if n == 11 {
            // d11中に勝利して戦闘が終わるため、Fighting限定ではなく全Arcadia Divisionで見る。
            s.day11_min_ful = mil
                .divisions
                .values()
                .filter(|d| d.owner == ARCADIA)
                .map(|d| mil.supply_fulfillment_of(d.id))
                .fold(f64::INFINITY, f64::min);
        }
        if sr.get(StateId(3)).unwrap().controller() == ARCADIA {
            s.ctrl3_to_arcadia_day.get_or_insert(n);
        }
    }
    let mil = app.world().resource::<MilitaryRegistry>();
    let sr = app.world().resource::<StateRegistry>();
    s.final_ctrl3 = sr.get(StateId(3)).unwrap().controller().0;
    let mut fs: Vec<_> = mil
        .divisions
        .values()
        .filter(|d| d.owner == ARCADIA)
        .map(|d| (d.id.0, d.status, d.current_state.0, d.manpower))
        .collect();
    fs.sort_by_key(|t| t.0);
    s.final_state = fs;
    s.final_arcadia_supplies_units =
        military_supplies_value_to_units(supplies(&app, ARCADIA)).unwrap();
    s.passguard_units = military_supplies_value_to_units(supplies(&app, CountryId(4))).unwrap();
    s
}

// ─────────────────────────────────────────────────────────────────────────

/// P27-005-PRE-001の一時候補パッチで実測した正規値: d10に敵州(StateId(3))へ到着して戦闘開始、
/// d10は前日補給率1.0、d11にArcadiaが勝利しcontrollerが変わる。旧来の補給0膠着は0日。
#[test]
fn test_01_arcadia_baseline_bridge_matches_the_audited_canonical_timeline() {
    let s = run(&Cfg::base());
    assert_eq!(s.first_fight_day, Some(10), "d10に敵州へ到着し戦闘開始");
    assert_eq!(s.day10_min_ful, 1.0, "到着当日は前日値(1日ラグ、変更なし)");
    assert!(s.day11_min_ful > 0.0, "d11は前線補給により正の補給率");
    assert_eq!(s.ful0_fight_days, 0, "補給0の膠着日は0(旧: 110/111日)");
    assert_eq!(
        s.ctrl3_to_arcadia_day,
        Some(11),
        "d11に勝利しcontroller変更"
    );
    assert_eq!(s.final_ctrl3, ARCADIA.0);
    assert!(s.stock_never_negative);
}

#[test]
fn test_02_without_the_war_nothing_changes_for_bystanders_and_national_reserve_is_finite() {
    let with_war = run(&Cfg {
        days: 40,
        ..Cfg::base()
    });
    let without = run(&Cfg {
        with_war: false,
        attackers: 1,
        days: 40,
        ..Cfg::base()
    });
    assert_eq!(
        with_war.passguard_units, without.passguard_units,
        "第三国の備蓄は影響を受けない"
    );
    assert!(with_war.final_arcadia_supplies_units > 0);
}

#[test]
fn test_03_corridor_cut_stops_bridge_supply_for_exactly_the_cut_days_then_recovers() {
    let s = run(&Cfg {
        cut: Some((20, 45)),
        days: 80,
        ..Cfg::base()
    });
    assert_eq!(
        s.ful0_fight_days, 25,
        "d20〜d44の遮断期間ちょうど25日だけ補給0"
    );
    let after: Vec<f64> = s
        .fight_min_ful
        .iter()
        .filter(|(d, _)| *d >= 45)
        .map(|(_, f)| *f)
        .collect();
    assert!(
        !after.is_empty() && after.iter().all(|f| *f > 0.99),
        "再接続後は自然に回復"
    );
}

#[test]
fn test_04_capital_fall_keeps_the_siege_supply_at_zero() {
    let s = run(&Cfg {
        capital_falls: Some(20),
        days: 60,
        ..Cfg::base()
    });
    assert!(
        s.ful0_fight_days >= 40,
        "首都陥落後は投射されず補給0が維持される(実測41日)"
    );
    let before_fall: Vec<f64> = s
        .fight_min_ful
        .iter()
        .filter(|(d, _)| *d < 20)
        .map(|(_, f)| *f)
        .collect();
    assert!(before_fall.iter().all(|f| *f > 0.0));
}

#[test]
fn test_05_multiple_defenders_do_not_block_the_supply_bridge_itself() {
    let s = run(&Cfg {
        extra_defenders: 2,
        days: 60,
        ..Cfg::base()
    });
    assert_eq!(s.first_fight_day, Some(10));
    assert_eq!(
        s.ful0_fight_days, 0,
        "4対3でも攻撃側の補給は0にならない(戦闘の非終結は別件)"
    );
    assert!(s.stock_never_negative);
}

#[test]
fn test_06_undefended_enemy_state_is_occupied_on_arrival_without_any_battle() {
    let s = run(&Cfg {
        remove_defenders: true,
        attackers: 1,
        days: 20,
        ..Cfg::base()
    });
    assert_eq!(s.fight_days, 0, "戦闘は発生せず、架空の戦闘状態も作らない");
    assert_eq!(s.ctrl3_to_arcadia_day, Some(10), "既存の即時占領契約を維持");
}

#[test]
fn test_07_save_load_mid_battle_matches_the_continuous_run() {
    let cont = run(&Cfg {
        days: 60,
        ..Cfg::base()
    });
    for at in [5, 10, 11, 15] {
        let branched = run(&Cfg {
            save_at: Some(at),
            days: 60,
            ..Cfg::base()
        });
        assert_eq!(cont, branched, "d{at}でのsave→load分岐が連続実行と一致");
    }
}

#[test]
fn test_08_corridor_cut_with_save_load_matches_the_continuous_run() {
    let cont = run(&Cfg {
        cut: Some((20, 45)),
        days: 60,
        ..Cfg::base()
    });
    let branched = run(&Cfg {
        cut: Some((20, 45)),
        save_at: Some(30),
        days: 60,
        ..Cfg::base()
    });
    assert_eq!(cont, branched);
}

#[test]
fn test_09_determinism_two_independent_runs_are_identical() {
    assert_eq!(run(&Cfg::base()), run(&Cfg::base()));
    let c = Cfg {
        extra_defenders: 2,
        cut: Some((20, 45)),
        ..Cfg::base()
    };
    assert_eq!(run(&c), run(&c));
}

/// 国家備蓄不足: 戦闘中(d10)に備蓄を絞り、通常Divisionと前線Divisionを既存Hamilton規則で
/// 按分する。支給総量=消費量、備蓄は負にならず、補給率は0と1の間になる。
#[test]
fn test_10_shortage_is_shared_by_the_existing_hamilton_rule_and_conserves_supplies() {
    let dir = TempTestDir::new("shortage");
    let mut app = setup_app(dir.save_file_config());
    app.world_mut()
        .resource_mut::<WarRegistry>()
        .add_war(no_goal_war());
    app.world_mut()
        .resource_scope(|world, mut fr: Mut<FrontlineRegistry>| {
            let w = world.resource::<WarRegistry>();
            let s = world.resource::<StateRegistry>();
            let m = world.resource::<MilitaryRegistry>();
            update_all_frontlines(w, s, m, &mut fr);
        });
    {
        let mut m = app.world_mut().resource_mut::<MilitaryRegistry>();
        let base: Division = m
            .divisions
            .values()
            .find(|d| d.owner == ARCADIA)
            .unwrap()
            .clone();
        for _ in 1..4 {
            m.add_division(base.clone());
        }
    }
    set_supplies(&mut app, ARCADIA, 1_000_000.0);
    set_supplies(&mut app, ELFIN, 1_000_000.0);
    let (mut year, mut month, mut day) = (1800, 1u8, 2u8);
    for n in 1..=12u32 {
        if n == 11 {
            set_supplies(&mut app, ARCADIA, 2.0);
        }
        tick_day(&mut app, year, month, day);
        day += 1;
        if day > 28 {
            day = 1;
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
        if n == 11 {
            let mil = app.world().resource::<MilitaryRegistry>();
            let fs: Vec<f64> = mil
                .divisions
                .values()
                .filter(|d| d.owner == ARCADIA)
                .map(|d| mil.supply_fulfillment_of(d.id))
                .collect();
            assert!(
                fs.iter().all(|f| *f > 0.0 && *f < 1.0),
                "不足時は全Divisionが按分され0と1の間: {fs:?}"
            );
            assert_eq!(
                supplies(&app, ARCADIA),
                0.0,
                "備蓄2.0は全額消費され、負にならない"
            );
        }
    }
}
