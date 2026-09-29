//! P28-002: 戦時のMilitaryFactory建設AIによる攻勢補給の持続化 — 実データE2E。
//!
//! 全AI・無操作の実データNew Gameを、実時間システム(`app::time`)と同じく`GameDate`を
//! 1日ずつ進めて`DayChangedMessage`(月替わりには`MonthChangedMessage`も同じフレーム)を
//! 発行する方式で進める(P28-001-PRE-001/P28-002-PRE-001の診断と同じ進め方)。補給の
//! 直接注入は一切行わない——自国の生産で備蓄が回復し、既存の攻勢補給ゲート
//! (停止40日・再開70日)を正規に通過することを観測する。
//!
//! 期待値は本タスクの実測(`verification_logs/phase-28/p28-002/`)に基づく。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::BTreeMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::BuildingType;
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::{ResourceType, military_supplies_value_to_units};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::data::MilitaryRegistry;
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
            "strategy_game_p28_002_e2e_{label}_{}_{nanos}_{n}",
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

fn queued_military_factories(app: &App, c: CountryId) -> Vec<(StateId, u32)> {
    app.world()
        .resource::<CountryRegistry>()
        .get(c)
        .unwrap()
        .construction_queue
        .iter()
        .filter(|q| q.building_type == BuildingType::MilitaryFactory)
        .map(|q| (q.state_id, q.target_level))
        .collect()
}

fn military_factory_levels(app: &App) -> BTreeMap<usize, (usize, u32)> {
    app.world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .map(|s| {
            (
                s.id.0,
                (s.owner_country_id.0, s.buildings.get(&BuildingType::MilitaryFactory).copied().unwrap_or(0)),
            )
        })
        .collect()
}

/// 所有州のMilitaryFactoryが前月に実際に生産したMilitarySupplies月産(100×level×稼働率)。
fn realised_monthly_supply(app: &App, c: CountryId) -> f64 {
    app.world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .filter(|s| s.owner_country_id == c)
        .map(|s| {
            100.0
                * f64::from(s.building_level(BuildingType::MilitaryFactory))
                * f64::from(s.building_operation(BuildingType::MilitaryFactory))
        })
        .sum()
}

fn stock_units(app: &App, c: CountryId) -> u128 {
    let country = app.world().resource::<CountryRegistry>();
    u128::from(
        military_supplies_value_to_units(country.get(c).unwrap().stockpile.get(ResourceType::MilitarySupplies))
            .unwrap(),
    )
}

fn demand_units(app: &App, c: CountryId) -> u128 {
    let mr = app.world().resource::<MilitaryRegistry>();
    country_daily_demand_units(c, &mr.divisions, &mr.definitions).unwrap()
}

fn is_at_war(app: &App, c: CountryId) -> bool {
    app.world().resource::<WarRegistry>().is_country_at_war(c)
}

/// DwarfがArcadiaへ宣戦した戦争(自然運用の最初の戦争)の、Dwarf側の前線計画。
fn dwarf_war0_plan(app: &App) -> Option<(bool, FrontlineStance)> {
    let wars = app.world().resource::<WarRegistry>();
    let war = wars
        .wars
        .values()
        .find(|w| w.attackers.contains(&DWARF) && w.defenders.contains(&ARCADIA) && w.start_date == "1800/06/04")?;
    let fr = app.world().resource::<FrontlineRegistry>();
    let fl = fr.get_frontline_for_war(war.id)?;
    fr.get_plan(fl.frontline_id, DWARF).map(|p| (p.offensive_supply_blocked, p.stance))
}

/// 全体の状態のうち、save→loadの一致確認に使う要約。
fn fingerprint(app: &App) -> String {
    let cr = app.world().resource::<CountryRegistry>();
    let mut out = format!("date={} ", date(app));
    for c in ALL_SIX {
        let country = cr.get(c).unwrap();
        out.push_str(&format!(
            "c{}[treasury={:.6} supplies={:.6} queue={:?}] ",
            c.0,
            country.treasury,
            country.stockpile.get(ResourceType::MilitarySupplies),
            country
                .construction_queue
                .iter()
                .map(|q| (q.state_id.0, format!("{:?}", q.building_type), q.target_level, q.progress))
                .collect::<Vec<_>>()
        ));
    }
    out.push_str(&format!("mf={:?} ", military_factory_levels(app)));
    let wars = app.world().resource::<WarRegistry>();
    let mut ws: Vec<_> = wars.wars.values().map(|w| (w.id.0, format!("{:?}", w.status), w.start_date.clone())).collect();
    ws.sort();
    out.push_str(&format!("wars={ws:?} battles_next_id={}", app.world().resource::<BattleRegistry>().next_id()));
    out
}

/// 全AI運用を1日ずつ進めながら、MilitaryFactoryの新規着工がいつも「その日の開始時点で
/// 交戦中だったAI国家」によるものであることを毎日確認し、着工・完成を記録する。
/// (建設判断は`CountryAi`、戦争の決着は後段の`WarResolution`で起こるため、着工した日に
/// 戦争が終わる場合がある。日の開始時点の交戦状態で判定する。)
struct Observer {
    player: Option<CountryId>,
    seen: Vec<(CountryId, StateId, u32)>,
    levels: BTreeMap<usize, (usize, u32)>,
    starts: Vec<(String, CountryId, StateId, u32)>,
    completions: Vec<(String, usize, usize, u32)>,
}

impl Observer {
    fn new(app: &App, player: Option<CountryId>) -> Self {
        Self { player, seen: Vec::new(), levels: military_factory_levels(app), starts: Vec::new(), completions: Vec::new() }
    }

    fn advance(&mut self, app: &mut App) {
        let at_war_at_day_start: Vec<bool> = ALL_SIX.iter().map(|&c| is_at_war(app, c)).collect();
        advance_one_day(app);
        let today = date(app);
        for (i, c) in ALL_SIX.into_iter().enumerate() {
            for (state, level) in queued_military_factories(app, c) {
                if !self.seen.contains(&(c, state, level)) {
                    self.seen.push((c, state, level));
                    assert!(at_war_at_day_start[i], "{today}: {c:?}が平時にMilitaryFactoryを着工した");
                    assert_ne!(Some(c), self.player, "{today}: プレイヤー国家にAIが着工した");
                    self.starts.push((today.clone(), c, state, level));
                }
            }
        }
        let now = military_factory_levels(app);
        for (state, (owner, level)) in &now {
            if self.levels.get(state).is_some_and(|(_, before)| level > before) {
                self.completions.push((today.clone(), *owner, *state, *level));
            }
        }
        self.levels = now;
    }

    fn run_until(&mut self, app: &mut App, until: &str) {
        while date(app).as_str() < until {
            self.advance(app);
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

/// 自然運用: 宣戦したDwarfが戦時MilitaryFactoryを建設し、完成後の自国生産で
/// MilitarySupplies備蓄が既存ゲートの再開値(70日)に達してから攻勢・戦闘・勝利へ進む。
#[test]
fn test_01_dwarf_builds_completes_and_passes_the_unchanged_offensive_gate_by_its_own_production() {
    assert_eq!((WARTIME_STOP_DAYS, WARTIME_RESUME_DAYS), (40, 70), "攻勢補給ゲートの定数は変更しない");
    let dir = TempTestDir::new("t01");
    let mut app = setup_app(None, dir.save_file_config());

    // 開戦前: どの国も新経路でMilitaryFactoryを着工しない(毎日確認)。
    let mut observer = Observer::new(&app, None);
    observer.run_until(&mut app, "1800/06/04");
    assert!(observer.starts.is_empty(), "開戦前の着工: {:?}", observer.starts);
    let dwarf_supply_before_war = realised_monthly_supply(&app, DWARF);
    let dwarf_demand_before_war = demand_units(&app, DWARF) as f64 / 1e6 * 30.0;
    assert!(
        dwarf_supply_before_war < dwarf_demand_before_war,
        "開戦時のDwarfは月産{dwarf_supply_before_war} < 月需要{dwarf_demand_before_war}の構造的赤字"
    );

    // 開戦〜完成後の生産、ゲート再開、攻勢、戦闘を1日ずつ観測する。
    let mut resumed_on: Option<String> = None;
    let mut first_offensive: Option<String> = None;
    let mut first_battle: Option<String> = None;
    let mut prev_blocked: Option<bool> = None;
    while date(&app).as_str() < "1801/04/01" {
        observer.advance(&mut app);
        if let Some((blocked, stance)) = dwarf_war0_plan(&app) {
            if prev_blocked == Some(true) && !blocked && resumed_on.is_none() {
                // 再開は既存の70日しきい値を満たした日だけ起こる(ゲート迂回なし)。
                let stock = stock_units(&app, DWARF);
                let demand = demand_units(&app, DWARF);
                assert!(
                    stock >= demand * u128::from(WARTIME_RESUME_DAYS),
                    "{}: 再開時の備蓄{stock}は70日分(需要{demand}×70)以上のはず",
                    date(&app)
                );
                resumed_on = Some(date(&app));
            }
            if stance == FrontlineStance::Offensive && first_offensive.is_none() {
                assert!(resumed_on.is_some(), "ゲート再開より前に攻勢が始まってはいけない");
                first_offensive = Some(date(&app));
            }
            prev_blocked = Some(blocked);
        }
        let battles = app.world().resource::<BattleRegistry>();
        if first_battle.is_none() && battles.battles.values().any(|b| b.attacker_country == DWARF) {
            first_battle = Some(date(&app));
        }
    }

    // ── 機能上の契約(州を問わない。P28-005でRailway優先建設が加わっても、この
    // 契約自体は不変であるはず): 戦時に着工が起こり、完成後の実生産が開戦時を
    // 上回り、正規のゲート再開(既存の70日しきい値)→攻勢→戦闘→勝利へ進む。
    assert!(!observer.starts.iter().filter(|s| s.1 == DWARF).collect::<Vec<_>>().is_empty(), "Dwarfは戦時に少なくとも1件着工するはず");
    assert!(!observer.completions.iter().filter(|c| c.1 == DWARF.0).collect::<Vec<_>>().is_empty(), "戦時建設が少なくとも1件完成するはず");
    let dwarf_supply_after = realised_monthly_supply(&app, DWARF);
    assert!(
        dwarf_supply_after > dwarf_supply_before_war,
        "完成後の月産{dwarf_supply_after}は開戦時{dwarf_supply_before_war}より大きいはず"
    );
    assert!(resumed_on.is_some(), "自国生産により既存ゲートが正規に再開するはず");
    assert!(first_offensive.is_some(), "ゲート再開後に攻勢が始まるはず");
    assert!(first_battle.is_some(), "攻勢後に戦闘が起こるはず");
    let wars = app.world().resource::<WarRegistry>();
    let war0 = wars
        .wars
        .values()
        .find(|w| w.attackers.contains(&DWARF) && w.defenders.contains(&ARCADIA) && w.start_date == "1800/06/04")
        .unwrap();
    assert_eq!(war0.status, WarStatus::AttackerVictory, "自国生産による補給で最終的に攻撃側が勝利するはず");

    // ── 自然再現(参考。P28-005のRailway優先建設により、着工州・日付が変わったことを
    // 記録する回帰保護。Railwayが州6の物流を先に改善したことで、優先度2.5の増産見込み
    // 判定(`projected_military_factory_supply_gain`)が州20ではなく州6を選ぶようになり、
    // ゲート再開・攻勢・戦闘・終戦の日付も早まった——いずれも
    // `verification_logs/phase-28/p28-005/`で確認した経路依存の結果であり、
    // 上の機能上の契約はいずれの州が選ばれても成立する)。
    assert_eq!(
        observer.starts.iter().filter(|s| s.1 == DWARF).map(|s| (s.0.as_str(), s.2, s.3)).collect::<Vec<_>>(),
        vec![("1800/06/05", StateId(6), 3), ("1801/03/03", StateId(6), 4)],
        "P28-005適用後の自然再現(旧: 州20のlv1/lv2。理由は上記コメント参照)"
    );
    assert!(
        observer.completions.iter().any(|c| c.0 == "1800/09/03" && c.1 == DWARF.0 && c.2 == 6 && c.3 == 3),
        "州6のMilitaryFactory lv3は1800/09/03に完成する: {:?}", observer.completions
    );
    assert_eq!(resumed_on.as_deref(), Some("1800/11/03"), "P28-005適用後の自然再現(旧: 1801/03/01)");
    assert_eq!(first_offensive.as_deref(), Some("1800/12/03"), "P28-005適用後の自然再現(旧: 1801/03/04)");
    assert_eq!(first_battle.as_deref(), Some("1800/12/07"), "P28-005適用後の自然再現(旧: 1801/03/08)");
    assert_eq!(war0.end_date.as_deref(), Some("1801/01/30"), "P28-005適用後の自然再現(旧: 1801/03/29)");
}

/// プレイヤーが交戦中でも、そのプレイヤー国家にはAIの戦時建設を適用しない。
/// 同じ戦争の相手(AIのDwarf)は従来どおり建設する。
#[test]
fn test_02_player_country_at_war_never_receives_the_ai_wartime_factory() {
    let dir = TempTestDir::new("t02");
    let mut app = setup_app(Some(ARCADIA), dir.save_file_config());
    let mut run = Observer::new(&app, Some(ARCADIA));
    run.run_until(&mut app, "1801/01/01");
    assert!(is_at_war(&app, ARCADIA), "前提: プレイヤーのArcadiaはAIに宣戦されて交戦中");
    assert!(run.starts.iter().all(|s| s.1 != ARCADIA));
    assert!(queued_military_factories(&app, ARCADIA).is_empty());
    assert!(run.starts.iter().any(|s| s.1 == DWARF), "AIのDwarfは戦時建設を行う: {:?}", run.starts);
}

/// 戦時建設の着工前(宣戦当日)、建設キューに入った後、完成後の3分岐でsave→loadを挟んでも、
/// 建設キューが重複せず、連続実行と同じ結果になる。
#[test]
fn test_03_save_load_before_during_and_after_wartime_construction_matches_continuous_run() {
    const END: &str = "1801/04/01";
    let dir = TempTestDir::new("t03_continuous");
    let mut continuous = setup_app(None, dir.save_file_config());
    advance_until(&mut continuous, END);
    let expected = fingerprint(&continuous);

    for (label, branch) in [("before", "1800/06/04"), ("queued", "1800/07/01"), ("completed", "1800/10/01")] {
        let dir = TempTestDir::new(label);
        let mut app = setup_app(None, dir.save_file_config());
        advance_until(&mut app, branch);
        let queued_before_save = queued_military_factories(&app, DWARF);
        match label {
            "before" => assert!(queued_before_save.is_empty()),
            // P28-005適用後の自然再現(旧: 州20のlv1。test_01のコメント参照)。
            "queued" => assert_eq!(queued_before_save, vec![(StateId(6), 3)]),
            _ => assert!(queued_before_save.is_empty(), "1800/10/01にはlv3が完成済み"),
        }
        send_save_request(&mut app);
        send_load_request(&mut app);
        assert_eq!(date(&app), branch, "{label}: loadでGameDateが保存時点に戻る");
        assert_eq!(queued_military_factories(&app, DWARF), queued_before_save, "{label}: loadでキューが重複しない");
        while date(&app).as_str() < END {
            advance_one_day(&mut app);
            for c in ALL_SIX {
                assert!(queued_military_factories(&app, c).len() <= 1, "{label}: {c:?}のMilitaryFactory重複発注");
            }
        }
        assert_eq!(fingerprint(&app), expected, "{label}: save→load後も連続実行と一致するはず");
    }
}
