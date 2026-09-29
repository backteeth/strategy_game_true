//! P28-003: 飛び地に取り残されたDivisionを攻勢の前線配送判定から除外する — 実データE2E。
//!
//! 全AI・無操作の実データNew Gameを、実時間システムと同じく`GameDate`を1日ずつ進めて
//! 進める(P28-002のE2Eと同じ進め方)。
//!
//! 【P28-005による更新】: 当初(P28-003完了時点)はOceanicの降伏で州8(旧Oceanic首都)が
//! Dwarfへ割譲され、Dwarfの2個Divisionが首都と陸路でつながらない自国支配州に残る
//! シナリオ(P28-003-PRE-001のwar8)が自然運用で再現していた。P28-005のRailway優先
//! 建設(優先度2.7)の追加により戦争タイムライン全体が変わり、この自然再現が消滅した
//! (`verification_logs/phase-28/p28-005/`で240か月まで探索し、同種の自然発生シナリオが
//! 一つも見つからないことを確認済み)。契約自体(`offensive_delivery_division_ids`)は
//! `src/war/military_ai.rs`の単体テスト7件(無変更・全件緑)で純粋関数として引き続き
//! 検証されているため、本ファイルは「実App・実データの日次補給/前線Systemを通した
//! 統合検証」という役割に絞り、制御されたfixture(P26-008-Bの
//! `test_90_day_controlled_war_offensive_supply_gate_real_app_verification`と同じ
//! パターン: 実New Gameデータ + `WarRegistry::add_war`で戦争を直接構成し、
//! `update_all_frontlines`で前線を既存の正規経路のまま作る)で同じ因果を再現する。
//! 飛び地の構成は、Dwarf自身の所有権を書き換えず(Oceanicの首都所有はそのまま)、
//! 州8の`controller_country`だけをDwarfに上書きして「他国所有・自国占領」の飛び地を
//! 作る(P28-003契約が除外する2分類の片方、`src/war/military_ai.rs`の
//! `p28_stranded_friendly_exclave_divisions_are_removed_when_connected_ones_remain`と
//! 同じ形)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionId, FrontlineId, StateId, WarId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::{BattleRegistry, BattleStatus};
use strategy_game::military::data::{DivisionStatus, MilitaryRegistry};
use strategy_game::military::recruitment_supply_gate::{WARTIME_RESUME_DAYS, WARTIME_STOP_DAYS};
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
use strategy_game::war::offensive_supply_gate::MIN_OFFENSIVE_SUPPLY_FULFILLMENT;

const ARCADIA: CountryId = CountryId(0);
const DWARF: CountryId = CountryId(2);
/// Oceanicの実際の首都州。Dwarfの本土(州5・6・7・20、`connected_states_from_capital`で
/// 確認済み)とは陸路でつながっていない、実データ上の距離飛び地。
const EXCLAVE: StateId = StateId(8);
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

// ─── War fixture (P26-008-Bの`new_war_no_goals`/`test_90_day_...`と同じパターン) ──

/// 戦争目標を持たせない(`select_ai_offensive_objective`は目標未設定でも候補2
/// [前線から最短の敵支配地域]へフォールバックする)。`WarRegistry::add_war`(既存の
/// 正規ID発行API)で登録し、手でWarIdを組み立てない(Save側の`next_id`整合性を保つ)。
fn new_war_no_goals(attacker: CountryId, defender: CountryId) -> War {
    War {
        id: WarId(0), // add_warが上書きする
        name: "Test War".to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1799/12/01".to_string(), // process_daily_military_aiの当日判定を避ける
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: Default::default(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: Default::default(),
    }
}

/// 実New Gameデータ上に、Dwarf→Arcadiaの制御された戦争と、Dwarfの飛び地
/// (他国[Oceanic]所有・Dwarf占領の州8)に取り残された2個Divisionを構成する。
/// 州8の所有権(Oceanic)自体は変更しない——占領(`controller_country`)だけを
/// 上書きする、契約が除外するもう一方の分類(P28-003報告§4.1)。
fn build_exclave_war_fixture(app: &mut App) -> (WarId, FrontlineId, Vec<DivisionId>) {
    {
        let mut sr = app.world_mut().resource_mut::<StateRegistry>();
        let s = sr.get_mut(EXCLAVE).unwrap();
        assert_ne!(s.owner_country_id, DWARF, "前提: 州8はDwarf自身の所有ではない");
        s.controller_country = Some(DWARF);
    }

    let stranded_ids: Vec<DivisionId> = {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        let template =
            mr.divisions.values().find(|d| d.owner == DWARF).cloned().expect("Dwarfは開始時に1個以上Divisionを持つ");
        (0..2)
            .map(|_| {
                let mut d = template.clone();
                d.current_state = EXCLAVE;
                d.destination = None;
                d.current_path = Vec::new();
                d.status = DivisionStatus::Idle;
                d.combat_id = None;
                d.manpower = 10_000;
                d.max_manpower = 10_000;
                mr.add_division(d)
            })
            .collect()
    };

    let war_id = app.world_mut().resource_mut::<WarRegistry>().add_war(new_war_no_goals(DWARF, ARCADIA));
    app.world_mut().resource_scope(|world, mut frontline_registry: Mut<FrontlineRegistry>| {
        let war_registry = world.resource::<WarRegistry>();
        let state_registry = world.resource::<StateRegistry>();
        let military_registry = world.resource::<MilitaryRegistry>();
        update_all_frontlines(war_registry, state_registry, military_registry, &mut frontline_registry);
    });

    // Dwarfの戦力比110%を確実に満たす(既存の攻勢判定式110%/90%自体は変更しない、
    // 増強のみ。P26-008-Bの90日実App検証と同じ方針)。
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        for d in mr.divisions.values_mut() {
            if d.owner == DWARF && !stranded_ids.contains(&d.id) {
                d.manpower = 100_000;
                d.max_manpower = 100_000;
            }
        }
    }
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(DWARF)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitarySupplies, 1_000_000.0);

    let fl_id = app.world().resource::<FrontlineRegistry>().get_frontline_for_war(war_id).unwrap().frontline_id;
    {
        let mut fr = app.world_mut().resource_mut::<FrontlineRegistry>();
        if let Some(plan) = fr.get_plan_mut(fl_id, DWARF) {
            for &id in &stranded_ids {
                if !plan.assigned_division_ids.contains(&id) {
                    plan.assigned_division_ids.push(id);
                }
            }
            plan.assigned_division_ids.sort_by_key(|id| id.0);
        }
    }

    (war_id, fl_id, stranded_ids)
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
    let mr = w.resource::<MilitaryRegistry>();
    let mut divs: Vec<_> = mr
        .divisions
        .values()
        .map(|d| (d.id.0, d.owner.0, d.current_state.0, d.manpower, format!("{:?}", d.status)))
        .collect();
    divs.sort();
    out.push_str(&format!("divs={divs:?} "));
    let fr = w.resource::<FrontlineRegistry>();
    let wars = w.resource::<WarRegistry>();
    let mut ws: Vec<_> = wars
        .wars
        .values()
        .map(|x| {
            let assigned = fr
                .get_frontline_for_war(x.id)
                .and_then(|fl| fr.get_plan(fl.frontline_id, DWARF))
                .map(|p| p.assigned_division_ids.clone())
                .unwrap_or_default();
            (x.id.0, format!("{:?}", x.status), x.end_date.clone(), assigned.iter().map(|d| d.0).collect::<Vec<_>>())
        })
        .collect();
    ws.sort();
    out.push_str(&format!("wars={ws:?} battles_next_id={}", w.resource::<BattleRegistry>().next_id()));
    out
}

// ─── Tests ──────────────────────────────────────────────────────────────────

/// 制御されたfixture: Dwarfが占領した飛び地(州8)の2個Divisionが前線割当に
/// 含まれても、実際の日次補給・前線Systemを通して、飛び地を除いた母集合で
/// 攻勢継続可否が判定され、攻勢が塞がれないことを確認する(P28-003の中核契約)。
///
/// 「補給0のDivisionを含む攻撃が発生しない」こと自体はP28-003の契約ではなく、
/// 取り残されたDivisionが配送判定からは除外されても前線割当・移動命令の対象には
/// 残る(既存のLow L-6、`verification_logs/phase-28/p28-003/`§10で記録済み、
/// 本タスクの対象外)ため、飛び地のDivisionが実際に敵領内の戦闘に参加してしまう
/// 日が生じ得る。自然運用(P28-003完了時点)ではこの飛び地が地理的に遠く、
/// 観測窓内で戦闘に参加しなかったための偶然の結果であり、恒久的な保証ではない。
/// 本fixtureでは診断としてのみ記録し、L-6の対象としてassertしない。
#[test]
fn test_01_exclave_divisions_no_longer_block_a_constructed_offensive_and_no_unsupplied_attack_occurs() {
    assert_eq!((WARTIME_STOP_DAYS, WARTIME_RESUME_DAYS), (40, 70), "攻勢補給ゲートの定数は変更しない");
    assert!((MIN_OFFENSIVE_SUPPLY_FULFILLMENT - 0.34).abs() < 1e-12, "前線配送のしきい値は変更しない");
    let dir = TempTestDir::new("t01");
    let mut app = setup_app(None, dir.save_file_config());

    let (_war_id, fl_id, stranded_ids) = build_exclave_war_fixture(&mut app);

    let mut unsupplied_attack_days = 0usize;
    for _ in 0..30 {
        advance_one_day(&mut app);
        if unsupplied_attacking_battles(&app) > 0 {
            unsupplied_attack_days += 1;
        }
    }
    eprintln!(
        "[P28-003 constructed fixture] unsupplied_attack_days={unsupplied_attack_days} \
         (既存Low L-6の対象、本テストのassert対象ではない)"
    );

    // 飛び地のDivisionは実際の日次補給System(P26-006の首都連結判定、無変更)により
    // 充足率0になる。
    let mr = app.world().resource::<MilitaryRegistry>();
    for &id in &stranded_ids {
        assert_eq!(mr.supply_fulfillment_of(id), 0.0, "飛び地のDivision{id:?}は補給0のはず");
    }

    // 前線配送判定: 割当には飛び地以外の健全なDivisionも残っているはず(前提)。
    let fr = app.world().resource::<FrontlineRegistry>();
    let plan = fr.get_plan(fl_id, DWARF).unwrap();
    assert!(
        plan.assigned_division_ids.len() > stranded_ids.len(),
        "飛び地以外にも割当があるはず(前提): {:?}",
        plan.assigned_division_ids
    );
    // 従来の母集合(飛び地を含む全割当)なら、飛び地の充足率0が最小値になり、
    // 0.34未満で攻勢が停止していたはず(この比較のためだけの計算、本番コードは
    // 呼ばない)。
    let old_scope_min_fulfillment =
        plan.assigned_division_ids.iter().map(|id| mr.supply_fulfillment_of(*id)).fold(1.0f64, f64::min);
    assert!(
        old_scope_min_fulfillment < MIN_OFFENSIVE_SUPPLY_FULFILLMENT,
        "従来の母集合(全割当)なら飛び地の充足率0で0.34未満のはず: {old_scope_min_fulfillment}"
    );
    assert!(
        !plan.offensive_supply_blocked,
        "飛び地を除いた母集合では、健全なDivisionの充足率で判定されるため、攻勢は塞がれないはず"
    );
}

/// 制御されたfixtureの構成直後・数日後の2分岐でsave→loadを挟んでも、飛び地の
/// Division・前線割当・補給充足率が連続実行と一致する。
#[test]
fn test_02_save_load_around_the_exclave_scenario_matches_continuous_run() {
    const DAYS: u32 = 20;
    let dir = TempTestDir::new("t02_continuous");
    let mut continuous = setup_app(None, dir.save_file_config());
    build_exclave_war_fixture(&mut continuous);
    for _ in 0..DAYS {
        advance_one_day(&mut continuous);
    }
    let expected = fingerprint(&continuous);

    for (label, days_before_save) in [("just_built", 0u32), ("mid_run", 10)] {
        let dir = TempTestDir::new(label);
        let mut app = setup_app(None, dir.save_file_config());
        build_exclave_war_fixture(&mut app);
        for _ in 0..days_before_save {
            advance_one_day(&mut app);
        }
        send_save_request(&mut app);
        send_load_request(&mut app);
        for _ in days_before_save..DAYS {
            advance_one_day(&mut app);
        }
        assert_eq!(fingerprint(&app), expected, "{label}: save→load後も連続実行と一致するはず");
    }
}
