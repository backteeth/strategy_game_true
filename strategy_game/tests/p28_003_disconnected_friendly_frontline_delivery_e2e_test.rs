//! P28-003: 飛び地に取り残されたDivisionを攻勢の前線配送判定から除外する — 実データE2E。
//!
//! 全AI・無操作の実データNew Gameを、実時間システムと同じく`GameDate`を1日ずつ進めて
//! 進める(P28-002のE2Eと同じ進め方)。Oceanicの降伏で州8(旧Oceanic首都)がDwarfへ割譲され、
//! Dwarfの2個Divisionが首都と陸路でつながらない自国支配州に残る。国家備蓄が十分でも、
//! 従来はその充足率0が前線配送判定を塞いでいた(P28-003-PRE-001のwar8)。
//! 期待値は本タスクの実測(`verification_logs/phase-28/p28-003/`)に基づく。
use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::BTreeMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::{ResourceType, military_supplies_value_to_units};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::{BattleRegistry, BattleStatus};
use strategy_game::military::capital_connectivity::connected_states_from_capital;
use strategy_game::military::data::{DivisionStatus, MilitaryRegistry};
use strategy_game::military::recruitment_supply_gate::{WARTIME_RESUME_DAYS, WARTIME_STOP_DAYS};
use strategy_game::military::supply_initialization::country_daily_demand_units;
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
use strategy_game::war::data::{WarRegistry, WarStatus};
use strategy_game::war::frontline::{FrontlineRegistry, FrontlineStance};
use strategy_game::war::offensive_supply_gate::MIN_OFFENSIVE_SUPPLY_FULFILLMENT;

const ARCADIA: CountryId = CountryId(0);
const DWARF: CountryId = CountryId(2);
const ALL_SIX: [CountryId; 6] = [CountryId(0), CountryId(1), CountryId(2), CountryId(3), CountryId(4), CountryId(5)];

// ─── App-level fixtures ─────────────────────────────────────────────────────

struct TempTestDir(std::path::PathBuf);
impl TempTestDir {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        Self(std::env::temp_dir().join(format!(
            "strategy_game_p28_003_e2e_{label}_{}_{nanos}_{n}",
            std::process::id()
        )))
    }
    fn save_file_config(&self) -> SaveFileConfig {
        SaveFileConfig { path: SavePathConfig { final_path: self.0.join("savegame_v1.ron") } }
    }
}
impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_app(player: Option<CountryId>, save_config: SaveFileConfig) -> App {
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
    // デフォルトstateのまま最初のupdateを呼ばないと、Playing挿入が遷移として認識されない。
    app.update();
    app.insert_resource(PlayerCountry(player));
    app.insert_state(GameState::Playing);
    app.update();
    app
}

/// `app::time`と同じく`GameDate`を1日進め、`DayChangedMessage`と(月替わりなら)
/// `MonthChangedMessage`を同じフレームで発行する。
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
    let mut s: SystemState<(MessageWriter<DayChangedMessage>, MessageWriter<MonthChangedMessage>)> =
        SystemState::new(app.world_mut());
    {
        let (mut days, mut months) = s.get_mut(app.world_mut()).unwrap();
        days.write(DayChangedMessage { year: next.year, month: next.month, day: next.day });
        if next.month != old_month {
            months.write(MonthChangedMessage { year: next.year, month: next.month });
        }
    }
    s.apply(app.world_mut());
    app.update();
}

fn date(app: &App) -> String {
    app.world().resource::<GameDate>().display()
}

/// `GameDate`が`until`("YYYY/MM/DD")に達するまで進める。
fn advance_until(app: &mut App, until: &str) {
    while date(app).as_str() < until {
        advance_one_day(app);
    }
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
    state.apply(app.world_mut());
    app.update();
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
    app.update();
}


// ─── Observation helpers ────────────────────────────────────────────────────

const OCEANIC: CountryId = CountryId(3);
const OCEANIC_OLD_CAPITAL: StateId = StateId(8);

fn owner_of(app: &App, s: StateId) -> CountryId {
    app.world().resource::<StateRegistry>().get(s).unwrap().owner_country_id
}

/// 自国支配州にいるが首都と陸路でつながらない(飛び地に取り残された)Division。
fn stranded_divisions(app: &App, c: CountryId) -> Vec<(DivisionId, StateId, f64)> {
    let w = app.world();
    let sr = w.resource::<StateRegistry>();
    let mr = w.resource::<MilitaryRegistry>();
    let capital = w.resource::<CountryRegistry>().get(c).unwrap().current_capital_state_id();
    let connected = capital.map(|cap| connected_states_from_capital(cap, c, sr)).unwrap_or_default();
    let mut v: Vec<_> = mr
        .divisions
        .values()
        .filter(|d| d.owner == c && d.manpower > 0 && d.status != DivisionStatus::Destroyed)
        .filter(|d| sr.get(d.current_state).is_some_and(|s| s.controller() == c && !connected.contains(&s.id)))
        .map(|d| (d.id, d.current_state, mr.supply_fulfillment_of(d.id)))
        .collect();
    v.sort_by_key(|x| x.0.0);
    v
}

fn reserve_days(app: &App, c: CountryId) -> f64 {
    let w = app.world();
    let stock = military_supplies_value_to_units(
        w.resource::<CountryRegistry>().get(c).unwrap().stockpile.get(ResourceType::MilitarySupplies),
    )
    .unwrap() as f64;
    let mr = w.resource::<MilitaryRegistry>();
    stock / country_daily_demand_units(c, &mr.divisions, &mr.definitions).unwrap().max(1) as f64
}

fn is_war8(w: &strategy_game::war::data::War) -> bool {
    w.attackers.contains(&DWARF) && w.defenders.contains(&ARCADIA) && w.start_date == "1806/05/02"
}

/// 1806/05/02にDwarfがArcadiaへ宣戦した戦争(P28-003-PRE-001のwar8)の、Dwarf側の前線計画。
fn dwarf_war8_plan(app: &App) -> Option<(bool, FrontlineStance, Vec<DivisionId>)> {
    let wars = app.world().resource::<WarRegistry>();
    let war = wars.wars.values().find(|w| is_war8(w))?;
    let fr = app.world().resource::<FrontlineRegistry>();
    let fl = fr.get_frontline_for_war(war.id)?;
    fr.get_plan(fl.frontline_id, DWARF).map(|p| (p.offensive_supply_blocked, p.stance, p.assigned_division_ids.clone()))
}

/// その日に進行中の戦闘のうち、攻撃側に補給充足率0のDivisionが含まれる戦闘の数。
fn unsupplied_attacking_battles(app: &App) -> usize {
    let w = app.world();
    let mr = w.resource::<MilitaryRegistry>();
    w.resource::<BattleRegistry>()
        .battles
        .values()
        .filter(|b| b.status == BattleStatus::Ongoing)
        .filter(|b| {
            b.attacker_division_ids
                .iter()
                .any(|id| mr.divisions.contains_key(id) && mr.supply_fulfillment_of(*id) <= 0.0)
        })
        .count()
}

fn fingerprint(app: &App) -> String {
    let w = app.world();
    let cr = w.resource::<CountryRegistry>();
    let mut out = format!("date={} ", date(app));
    for c in ALL_SIX {
        let country = cr.get(c).unwrap();
        out.push_str(&format!(
            "c{}[treasury={:.6} supplies={:.6} queue={}] ",
            c.0,
            country.treasury,
            country.stockpile.get(ResourceType::MilitarySupplies),
            country.construction_queue.len()
        ));
    }
    let owners: BTreeMap<usize, (usize, usize)> = w
        .resource::<StateRegistry>()
        .states
        .iter()
        .map(|s| (s.id.0, (s.owner_country_id.0, s.controller().0)))
        .collect();
    out.push_str(&format!("states={owners:?} "));
    let mut divs: Vec<_> = w
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .map(|d| (d.id.0, d.owner.0, d.current_state.0, d.manpower, format!("{:?}", d.status)))
        .collect();
    divs.sort();
    out.push_str(&format!("divs={divs:?} "));
    let wars = w.resource::<WarRegistry>();
    let mut ws: Vec<_> = wars.wars.values().map(|x| (x.id.0, format!("{:?}", x.status), x.end_date.clone())).collect();
    ws.sort();
    out.push_str(&format!("wars={ws:?} battles_next_id={}", w.resource::<BattleRegistry>().next_id()));
    out
}

// ─── Tests ──────────────────────────────────────────────────────────────────

/// 監査のwar8と同じ因果: 州8の割譲 → Dwarfの2個Divisionが首都非連結の自国支配州に残り
/// 補給0 → 国家備蓄は十分 → 従来の母集合なら前線配送判定で停止 → 飛び地を除いた母集合で
/// 攻勢・占領・降伏。全期間を通じて、補給0のDivisionを含む攻撃は発生しない。
#[test]
fn test_01_exclave_divisions_no_longer_block_war8_and_no_unsupplied_attack_occurs() {
    assert_eq!((WARTIME_STOP_DAYS, WARTIME_RESUME_DAYS), (40, 70), "攻勢補給ゲートの定数は変更しない");
    assert!((MIN_OFFENSIVE_SUPPLY_FULFILLMENT - 0.34).abs() < 1e-12, "前線配送のしきい値は変更しない");
    let dir = TempTestDir::new("t01");
    let mut app = setup_app(None, dir.save_file_config());

    let mut unsupplied_attack_days = 0usize;
    let mut ceded_on: Option<String> = None;
    let mut old_scope_would_block: Option<bool> = None;
    let mut first_offensive: Option<String> = None;
    let mut war8_ended: Option<(Option<String>, WarStatus)> = None;
    while date(&app).as_str() < "1806/06/01" {
        let owner_before = owner_of(&app, OCEANIC_OLD_CAPITAL);
        advance_one_day(&mut app);
        if unsupplied_attacking_battles(&app) > 0 {
            unsupplied_attack_days += 1;
        }
        if owner_before == OCEANIC && owner_of(&app, OCEANIC_OLD_CAPITAL) == DWARF {
            ceded_on = Some(date(&app));
        }
        if date(&app) == "1806/05/02" {
            // 宣戦当日: 飛び地のDivisionは補給0、国家備蓄は再開値70日を大きく超える。
            let stranded = stranded_divisions(&app, DWARF);
            assert_eq!(stranded.len(), 2, "州8に取り残されたDwarfの2個Division: {stranded:?}");
            assert!(stranded.iter().all(|s| s.1 == OCEANIC_OLD_CAPITAL && s.2 == 0.0), "{stranded:?}");
            assert!(reserve_days(&app, DWARF) > f64::from(WARTIME_RESUME_DAYS));
        }
        if first_offensive.is_none()
            && let Some((blocked, stance, assigned)) = dwarf_war8_plan(&app)
            && stance == FrontlineStance::Offensive
        {
            first_offensive = Some(date(&app));
            assert!(!blocked);
            // 従来の母集合(割当全体)では、割り当てられた飛び地のDivisionの充足率0が最小値になり、
            // しきい値0.34を下回っていたはず。
            let stranded = stranded_divisions(&app, DWARF);
            let assigned_stranded: Vec<_> = stranded.iter().filter(|s| assigned.contains(&s.0)).collect();
            old_scope_would_block = Some(
                !assigned_stranded.is_empty()
                    && assigned_stranded.iter().all(|s| s.2 < MIN_OFFENSIVE_SUPPLY_FULFILLMENT),
            );
        }
        if war8_ended.is_none() {
            let wars = app.world().resource::<WarRegistry>();
            if let Some(w) = wars.wars.values().find(|w| is_war8(w))
                && w.status != WarStatus::Active
            {
                war8_ended = Some((w.end_date.clone(), w.status));
            }
        }
    }
    assert_eq!(ceded_on.as_deref(), Some("1802/02/09"), "Oceanicの降伏で州8がDwarfへ割譲される");
    assert_eq!(first_offensive.as_deref(), Some("1806/05/03"), "宣戦翌日(軍事AIが動ける最初の日)に攻勢へ移る");
    assert_eq!(old_scope_would_block, Some(true), "従来の母集合なら飛び地の補給0で停止していた");
    assert_eq!(war8_ended, Some((Some("1806/05/18".to_string()), WarStatus::AttackerVictory)));
    assert_eq!(unsupplied_attack_days, 0, "補給0のDivisionを含む攻撃は発生しない");
}

/// 州8割譲前・飛び地成立後・war8の攻勢開始前後でsave→loadを挟んでも、連続実行と一致する。
#[test]
fn test_02_save_load_around_cession_exclave_and_offensive_matches_continuous_run() {
    const END: &str = "1806/06/01";
    let dir = TempTestDir::new("t02_continuous");
    let mut continuous = setup_app(None, dir.save_file_config());
    advance_until(&mut continuous, END);
    let expected = fingerprint(&continuous);

    for (label, branch) in [
        ("before_cession", "1802/02/01"),
        ("exclave", "1803/01/01"),
        ("before_offensive", "1806/05/03"),
        ("after_offensive", "1806/05/10"),
    ] {
        let dir = TempTestDir::new(label);
        let mut app = setup_app(None, dir.save_file_config());
        advance_until(&mut app, branch);
        send_save_request(&mut app);
        send_load_request(&mut app);
        assert_eq!(date(&app), branch, "{label}: loadでGameDateが保存時点に戻る");
        advance_until(&mut app, END);
        assert_eq!(fingerprint(&app), expected, "{label}: save→load後も連続実行と一致するはず");
    }
}
