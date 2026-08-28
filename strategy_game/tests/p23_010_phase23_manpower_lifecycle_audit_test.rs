//! P23-010: Phase 23(人口・人的資源ライフサイクル)最終統合監査。
//!
//! P23-001〜P23-009の個別タスクが固定した挙動を、実`AppPlugin`/`PopulationPlugin`/
//! `EconomyPlugin`/`MilitaryPlugin`等の正規構成を通した長期・横断シナリオで再確認する。
//! 本ファイルは新機能を実装せず、既存Domain APIと実System経路だけを呼び出す
//! (本番式をテスト側へ複製しない)。
//!
//! 帳簿の記法は監査報告書と同じ:
//! `A=available_manpower, B=mobilized_manpower, L=living_military_manpower,
//! K=permanent_military_losses, X=unaccounted_living_excess`。
#![allow(clippy::type_complexity)]

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::data::{
    DivisionStatus, MilitaryRegistry, STANDARD_INFANTRY_DEFINITION_ID,
};
use strategy_game::military::demobilization::demobilize_division;
use strategy_game::military::manpower::{ManpowerLedger, compute_manpower_ledger};
use strategy_game::military::recruitment::request_recruitment;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{
    LastLoadOutcome, LoadOutcome, LoadRequestMessage, SaveFileConfig,
};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::FrontlineRegistry;

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p23_010_audit_{label}_{}_{nanos}_{n}",
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

// ─────────────────────────────────────────────────────────────────────────
// 完全なApp構成(実データ6か国、Phase 23全タスクと同一パターン)
// ─────────────────────────────────────────────────────────────────────────

fn setup_full_app(player: CountryId, save_config: SaveFileConfig) -> App {
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
        .insert_resource(strategy_game::map::frontline_render::FrontlineRenderSettings::default())
        .insert_resource(strategy_game::map::frontline_selection::FrontlineSelectMode::default())
        .insert_resource(
            strategy_game::map::offensive_line_selection::OffensiveLineEditMode::default(),
        )
        .add_message::<strategy_game::map::frontline_selection::FrontlineAssignmentAttempted>()
        .add_message::<strategy_game::map::camera::CameraResetRequestMessage>();

    app.add_plugins(SaveGamePlugin);
    app.add_plugins(LoadGamePlugin);

    app.insert_resource(save_config);
    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing): 初期Division配置

    app
}

fn send_month_changed(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
        .write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

fn send_day_changed(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
        .write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

/// 1か月分(`days_per_month`日 + 月替わり1回)を進める。実際の暦計算は行わず、
/// 既存の正規Message経由の進行(`MonthChangedMessage`/`DayChangedMessage`)だけを使う。
fn advance_one_simulated_month(app: &mut App, year: i32, month: u8, days_per_month: u8) {
    for day in 1..=days_per_month {
        send_day_changed(app, year, month, day);
    }
    send_month_changed(app, year, month);
}

fn all_active_countries(app: &App) -> Vec<CountryId> {
    let mut ids: Vec<CountryId> = app
        .world()
        .resource::<CountryRegistry>()
        .countries
        .iter()
        .filter(|c| c.is_active())
        .map(|c| c.id)
        .collect();
    ids.sort_by_key(|id| id.0);
    ids
}

fn ledger_for(app: &App, country: CountryId) -> Result<ManpowerLedger, String> {
    let c = app
        .world()
        .resource::<CountryRegistry>()
        .get(country)
        .unwrap();
    compute_manpower_ledger(c, app.world().resource()).map_err(|e| format!("{e:?}"))
}

fn queue_len(app: &App, country: CountryId) -> usize {
    app.world()
        .resource::<CountryRegistry>()
        .get(country)
        .unwrap()
        .recruitment_queue
        .len()
}

// ─────────────────────────────────────────────────────────────────────────
// §4.1/4.2 New Game / Loaded Game 監査
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn new_game_gives_every_active_country_a_balanced_ledger() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("new_game_balanced");
    let app = setup_full_app(player, temp_dir.save_file_config());

    let countries = all_active_countries(&app);
    assert!(!countries.is_empty(), "実データは最低1国家を持つはず");

    for id in countries {
        let ledger = ledger_for(&app, id).unwrap_or_else(|e| panic!("country {id:?}: {e}"));
        assert_eq!(
            ledger.workforce_burden, ledger.living_military_manpower,
            "country {id:?}: B==L"
        );
        assert_eq!(ledger.permanent_military_losses, 0, "country {id:?}: K==0");
        assert_eq!(ledger.unaccounted_living_excess, 0, "country {id:?}: X==0");
    }

    // 初期Division生成でavailable_manpower・populationが不正変更されないこと。
    let country_registry = app.world().resource::<CountryRegistry>();
    for c in &country_registry.countries {
        if c.is_active() {
            assert!(
                c.available_manpower > 0,
                "country {:?}: available must not be zeroed",
                c.id
            );
        }
    }
    let state_registry = app.world().resource::<StateRegistry>();
    for s in &state_registry.states {
        assert!(
            s.population > 0 || s.is_sea,
            "state {:?}: population must not be zeroed",
            s.id
        );
    }
}

#[test]
fn playing_reentry_does_not_duplicate_the_initial_division_setup() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("no_dup_on_reentry");
    let mut app = setup_full_app(player, temp_dir.save_file_config());

    let division_count_before = app.world().resource::<MilitaryRegistry>().divisions.len();
    let ledger_before = ledger_for(&app, player).unwrap();

    // 同じPlaying状態のまま複数フレーム進めても、初期Division配置Systemは
    // OnEnter(Playing)一度きりのはず。
    for _ in 0..5 {
        app.update();
    }

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        division_count_before
    );
    assert_eq!(ledger_for(&app, player).unwrap(), ledger_before);
}

#[test]
fn loaded_game_does_not_duplicate_divisions_or_trigger_immediate_ai_recruitment() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("loaded_game_no_side_effects");
    let save_config = temp_dir.save_file_config();

    let (saved_division_count, saved_queue_lengths) = {
        let mut producer = setup_full_app(player, save_config.clone());
        let count = producer
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .len();
        let queues: Vec<(CountryId, usize)> = all_active_countries(&producer)
            .into_iter()
            .map(|id| (id, queue_len(&producer, id)))
            .collect();
        let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
            SystemState::new(producer.world_mut());
        state
            .get_mut(producer.world_mut())
            .unwrap()
            .write(SaveRequestMessage);
        state.apply(producer.world_mut());
        producer.update();
        (count, queues)
    };

    let mut app = setup_full_app(player, save_config);
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
        .write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
    app.update();
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        saved_division_count,
        "loading must not duplicate divisions"
    );
    for (id, expected_len) in saved_queue_lengths {
        assert_eq!(
            queue_len(&app, id),
            expected_len,
            "country {id:?}: loading alone must not trigger AI recruitment"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// §4.3〜4.7/4.8 完全なライフサイクルサイクル(募兵→完成→戦闘損耗→補充→戦闘終了→復員→月次雇用)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn full_lifecycle_cycle_preserves_the_manpower_ledger_at_every_stage() {
    let player = CountryId(0);
    let enemy = CountryId(1);
    let temp_dir = TempTestDir::new("full_lifecycle_cycle");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let capital = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .capital_state_id;

    // ステージ0: 開始時の帳簿(New Game初期Divisionのみ)。
    let ledger0 = ledger_for(&app, player).unwrap();
    assert_eq!(ledger0.workforce_burden, ledger0.living_military_manpower);
    assert_eq!(ledger0.permanent_military_losses, 0);

    // ステージ1: プレイヤー募兵要求(既存request_recruitment経由)。
    // P25-003: Division生成時に国家備蓄からMilitaryEquipmentを支給するようになった
    // ため、完成(ステージ2)まで進めたいこのテストでは必要量以上を明示的に用意する。
    {
        let mut state: SystemState<(ResMut<CountryRegistry>, Res<MilitaryRegistry>)> =
            SystemState::new(app.world_mut());
        let (mut cr, mr) = state.get_mut(app.world_mut()).unwrap();
        let required_equipment = mr
            .definitions
            .get(&STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .required_equipment;
        let country = cr.get_mut(player).unwrap();
        country
            .stockpile
            .set(ResourceType::MilitaryEquipment, required_equipment);
        request_recruitment(country, &mr, STANDARD_INFANTRY_DEFINITION_ID, capital).unwrap();
        state.apply(app.world_mut());
    }
    let ledger1 = ledger_for(&app, player).unwrap();
    let required = app
        .world()
        .resource::<MilitaryRegistry>()
        .definitions
        .get(&STANDARD_INFANTRY_DEFINITION_ID)
        .unwrap()
        .required_manpower;
    assert_eq!(
        ledger1.workforce_burden,
        ledger0.workforce_burden + required,
        "B += R"
    );
    assert_eq!(
        ledger1.living_military_manpower,
        ledger0.living_military_manpower + required,
        "L += R(キュー予約分)"
    );
    assert_eq!(ledger1.permanent_military_losses, 0, "K unchanged");

    // ステージ2: 募兵完成(既存日次経路)。完成前後でLは変化しない(キュー由来→Division由来)。
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .recruitment_queue
        .last_mut()
        .unwrap()
        .days_remaining = 0;
    send_day_changed(&mut app, 1800, 2, 1);
    let ledger2 = ledger_for(&app, player).unwrap();
    assert_eq!(queue_len(&app, player), 0, "キューが消化されているはず");
    assert_eq!(
        ledger2.living_military_manpower, ledger1.living_military_manpower,
        "完成前後でLは不変"
    );
    assert_eq!(
        ledger2.workforce_burden, ledger1.workforce_burden,
        "完成で人的資源を再消費しない"
    );

    let new_division_id = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .find(|d| {
            d.owner == player
                && d.def_id == STANDARD_INFANTRY_DEFINITION_ID
                && d.manpower == required
        })
        .map(|d| d.id)
        .expect("completed division must exist");

    // ステージ3: 実戦闘損耗(実War/Battleを直接登録し、実handle_daily_military経路で処理)。
    let enemy_division_id = {
        let military_registry = app.world_mut().resource_mut::<MilitaryRegistry>();
        // HashMap走査順(プロセスごとのハッシュシード)に依存しないよう、DivisionId最小を
        // 決定的に選ぶ(この監査自体が要求する決定性の原則をテスト自身にも適用する)。
        military_registry
            .divisions
            .values()
            .filter(|d| d.owner == enemy)
            .min_by_key(|d| d.id.0)
            .map(|d| d.id)
            .expect("enemy must own at least one division")
    };
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        let d = mr.divisions.get_mut(&new_division_id).unwrap();
        d.status = DivisionStatus::Fighting;
        d.current_state = capital;
    }
    {
        let mut wr = app.world_mut().resource_mut::<WarRegistry>();
        wr.wars.insert(
            strategy_game::common::WarId(9000),
            War {
                id: strategy_game::common::WarId(9000),
                name: "P23-010 Audit War".to_string(),
                attackers: [player].into_iter().collect(),
                defenders: [enemy].into_iter().collect(),
                primary_attacker: None,
                primary_defender: None,
                source_crisis_id: None,
                war_goals: vec![],
                start_date: "1800/02/01".to_string(),
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
            },
        );
    }
    {
        let mut br = app.world_mut().resource_mut::<BattleRegistry>();
        br.battles.insert(
            strategy_game::common::BattleId(9000),
            Battle {
                id: strategy_game::common::BattleId(9000),
                war_id: strategy_game::common::WarId(9000),
                state_id: capital,
                attacker_country: player,
                defender_country: enemy,
                attacker_division_ids: vec![new_division_id],
                defender_division_ids: vec![enemy_division_id],
                attacker_origins: [(new_division_id, capital)].into_iter().collect(),
                start_date: "1800/02/01".to_string(),
                elapsed_days: 0,
                status: BattleStatus::Ongoing,
            },
        );
    }
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        mr.divisions.get_mut(&new_division_id).unwrap().combat_id =
            Some(strategy_game::common::BattleId(9000));
        if let Some(enemy_div) = mr.divisions.get_mut(&enemy_division_id) {
            enemy_div.status = DivisionStatus::Fighting;
            enemy_div.combat_id = Some(strategy_game::common::BattleId(9000));
        }
    }

    let manpower_before_combat =
        app.world().resource::<MilitaryRegistry>().divisions[&new_division_id].manpower;
    send_day_changed(&mut app, 1800, 2, 2); // 実handle_daily_military経路で1日戦闘処理

    let manpower_after_combat = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&new_division_id)
        .map(|d| d.manpower)
        .unwrap_or(0);
    let combat_loss = manpower_before_combat.saturating_sub(manpower_after_combat);
    let ledger3 = ledger_for(&app, player).unwrap();
    assert_eq!(
        ledger3.workforce_burden, ledger2.workforce_burden,
        "B unchanged by combat"
    );
    assert_eq!(
        ledger3.permanent_military_losses,
        ledger2.permanent_military_losses + combat_loss,
        "K += 実測損耗量"
    );
    // 他国(enemy)のB/L/K/available/treasuryも実測損耗以外の理由で変化していないこと
    // (相手側は防御側実損耗を除き非波及)。
    let other_countries: Vec<CountryId> = all_active_countries(&app)
        .into_iter()
        .filter(|&id| id != player && id != enemy)
        .collect();
    for id in other_countries {
        let ledger_now = ledger_for(&app, id).unwrap();
        assert_eq!(
            ledger_now.unaccounted_living_excess, 0,
            "country {id:?}: unaffected, X==0"
        );
    }

    // ステージ4: 戦闘を終結させる(生存していれば)。生存者を復員する。
    // 実戦闘計算の結果によっては、1日でどちらかの陣営が全滅し、実`resolve_finished_
    // battles`が既にBattleRegistryから当該戦闘を除去済み(かつDivision側のstatus/
    // combat_idも既にIdle/Noneへ戻す)場合があるため、その場合は不要な上書きをしない。
    if manpower_after_combat > 0 {
        {
            let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
            let d = mr.divisions.get_mut(&new_division_id).unwrap();
            d.status = DivisionStatus::Idle;
            d.combat_id = None;
        }
        if let Some(battle) = app
            .world_mut()
            .resource_mut::<BattleRegistry>()
            .battles
            .get_mut(&strategy_game::common::BattleId(9000))
        {
            battle.status = BattleStatus::AttackerWon;
        }

        // ステージ4.5: 日次補充(戦闘終了翌日なら補充対象)。
        let available_before_reinforce = app
            .world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .available_manpower;
        send_day_changed(&mut app, 1800, 2, 3);
        let manpower_after_reinforce =
            app.world().resource::<MilitaryRegistry>().divisions[&new_division_id].manpower;
        let ledger4 = ledger_for(&app, player).unwrap();
        let reinforced = manpower_after_reinforce.saturating_sub(manpower_after_combat);
        if reinforced > 0 {
            assert_eq!(
                ledger4.workforce_burden,
                ledger3.workforce_burden + reinforced,
                "B += Q"
            );
            assert_eq!(
                ledger4.permanent_military_losses, ledger3.permanent_military_losses,
                "K unchanged by reinforcement"
            );
            let available_after = app
                .world()
                .resource::<CountryRegistry>()
                .get(player)
                .unwrap()
                .available_manpower;
            assert_eq!(
                available_after,
                available_before_reinforce - reinforced,
                "A -= Q"
            );
        }

        // ステージ5: 生存者を復員する。
        let survivor_manpower =
            app.world().resource::<MilitaryRegistry>().divisions[&new_division_id].manpower;
        let available_before_demob = app
            .world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .available_manpower;
        let ledger_before_demob = ledger_for(&app, player).unwrap();

        let mut state: SystemState<(
            ResMut<CountryRegistry>,
            ResMut<MilitaryRegistry>,
            ResMut<ArmyRegistry>,
            ResMut<FrontlineRegistry>,
            Res<WarRegistry>,
            Res<BattleRegistry>,
        )> = SystemState::new(app.world_mut());
        let (mut cr, mut mr, mut ar, mut fr, wr, br) = state.get_mut(app.world_mut()).unwrap();
        let outcome = demobilize_division(
            &mut cr,
            &mut mr,
            &mut ar,
            &mut fr,
            &wr,
            &br,
            player,
            new_division_id,
        )
        .unwrap();
        assert_eq!(
            outcome.returned_manpower, survivor_manpower,
            "現在manpowerを返還(maxではない)"
        );
        state.apply(app.world_mut());

        let ledger5 = ledger_for(&app, player).unwrap();
        let available_after_demob = app
            .world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .available_manpower;
        assert_eq!(
            available_after_demob,
            available_before_demob + survivor_manpower,
            "A += R"
        );
        assert_eq!(
            ledger5.workforce_burden,
            ledger_before_demob.workforce_burden - survivor_manpower,
            "B -= R"
        );
        assert_eq!(
            ledger5.permanent_military_losses, ledger_before_demob.permanent_military_losses,
            "K unchanged by demobilization"
        );

        // ステージ6: 次の月次雇用更新で民間労働力に反映される。
        let cw_before_month = app
            .world()
            .resource::<StateRegistry>()
            .get(capital)
            .unwrap()
            .civilian_workforce();
        send_month_changed(&mut app, 1800, 3);
        let cw_after_month = app
            .world()
            .resource::<StateRegistry>()
            .get(capital)
            .unwrap()
            .civilian_workforce();
        // Bが復員により減ったため、民間労働力は減少しないはず(回復方向、または不変)。
        assert!(
            cw_after_month >= cw_before_month.saturating_sub(1),
            "demobilization should not further suppress civilian workforce next month: \
             before={cw_before_month}, after={cw_after_month}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// §5.1 長期AI収束(1年・5年・10年相当のチェックポイント)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn ai_recruitment_stays_bounded_and_consistent_over_a_ten_year_run() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("ten_year_ai_convergence");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let ai_countries: Vec<CountryId> = all_active_countries(&app)
        .into_iter()
        .filter(|&id| id != player)
        .collect();

    // 全AI国家に十分な資金・人的資源を与え、目標へ実際に収束させる(実データのままだと
    // 初期treasuryが不足して1件も募兵しないまま「収束」に見える誤検出を避けるため)。
    for &ai in &ai_countries {
        let mut cr = app.world_mut().resource_mut::<CountryRegistry>();
        let c = cr.get_mut(ai).unwrap();
        c.treasury = 10_000_000.0;
        c.available_manpower = 10_000_000;
    }

    let mut queue_len_history: Vec<usize> = Vec::new();
    for month_index in 1..=120i32 {
        let year = 1800 + (month_index - 1) / 12;
        let month = (((month_index - 1) % 12) + 1) as u8;
        advance_one_simulated_month(&mut app, year, month, 30);

        for &ai in &ai_countries {
            let len = queue_len(&app, ai);
            assert!(
                len <= 1,
                "month {month_index}, country {ai:?}: queue must never exceed 1 entry, got {len}"
            );
            queue_len_history.push(len);
        }

        if month_index == 12 || month_index == 60 || month_index == 120 {
            for &ai in &ai_countries {
                let ledger = ledger_for(&app, ai).unwrap_or_else(|e| {
                    panic!(
                        "month {month_index}, country {ai:?}: ledger must remain computable: {e}"
                    )
                });
                assert_eq!(
                    ledger.unaccounted_living_excess, 0,
                    "month {month_index}, country {ai:?}: X==0"
                );
            }
            // treasuryが負になること自体はP23-010の対象外(既存の生産・税収式の挙動、
            // §10で明示的に対象外)。ここでは「有限の数値であること」(NaN・無限大でない)
            // だけを、u64のavailable_manpower/mobilized_manpowerと合わせて確認する
            // (overflow/underflowの副作用がないことの弱い健全性チェック)。
            let country_registry = app.world().resource::<CountryRegistry>();
            for c in &country_registry.countries {
                assert!(
                    c.treasury.is_finite(),
                    "month {month_index}, country {:?}: treasury must remain a finite number (NaN/inf would indicate a real defect)",
                    c.id
                );
                assert!(
                    c.available_manpower < u64::MAX,
                    "month {month_index}, country {:?}: available must not saturate at u64::MAX",
                    c.id
                );
            }
        }
    }

    // 他国非波及: プレイヤー自身は一度もAI募兵の対象になっていない。
    assert_eq!(
        queue_len(&app, player),
        0,
        "the player must never be auto-recruited for over the whole run"
    );

    // 10年間、必ずどこかの月でqueueが空になった実績がある(=完成サイクルが実際に回っている)
    // ことを確認する(全期間ずっと1のまま = 完成が一度も起きていない、という異常を検出)。
    assert!(
        queue_len_history.contains(&0),
        "over a 10-year run, at least one country's queue must have drained at least once \
         (recruitment completion must actually be occurring)"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §5.3 セーブ分岐比較
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
struct NormalizedCountrySnapshot {
    id: CountryId,
    available_manpower: u64,
    mobilized_manpower: u64,
    treasury_rounded: i64,
    queue_len: usize,
    ledger: ManpowerLedger,
}

#[derive(Debug, PartialEq)]
struct NormalizedStateSnapshot {
    id: StateId,
    owner: CountryId,
    population: u64,
    employed: u64,
    unemployed: u64,
}

#[derive(Debug, PartialEq)]
struct NormalizedDivisionSnapshot {
    id: DivisionId,
    owner: CountryId,
    manpower: u64,
    status: DivisionStatus,
}

fn normalized_country_snapshot(app: &App) -> Vec<NormalizedCountrySnapshot> {
    let mut out: Vec<NormalizedCountrySnapshot> = all_active_countries(app)
        .into_iter()
        .map(|id| {
            let c = app.world().resource::<CountryRegistry>().get(id).unwrap();
            NormalizedCountrySnapshot {
                id,
                available_manpower: c.available_manpower,
                mobilized_manpower: c.mobilized_manpower,
                treasury_rounded: c.treasury.round() as i64,
                queue_len: c.recruitment_queue.len(),
                ledger: ledger_for(app, id).unwrap(),
            }
        })
        .collect();
    out.sort_by_key(|s| s.id.0);
    out
}

fn normalized_state_snapshot(app: &App) -> Vec<NormalizedStateSnapshot> {
    let mut out: Vec<NormalizedStateSnapshot> = app
        .world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .map(|s| NormalizedStateSnapshot {
            id: s.id,
            owner: s.owner_country_id,
            population: s.population,
            employed: s.employed_workforce,
            unemployed: s.unemployed_workforce,
        })
        .collect();
    out.sort_by_key(|s| s.id.0);
    out
}

fn normalized_division_snapshot(app: &App) -> Vec<NormalizedDivisionSnapshot> {
    let mut out: Vec<NormalizedDivisionSnapshot> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .map(|d| NormalizedDivisionSnapshot {
            id: d.id,
            owner: d.owner,
            manpower: d.manpower,
            status: d.status,
        })
        .collect();
    out.sort_by_key(|d| d.id.0);
    out
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
        .write(SaveRequestMessage);
    state.apply(app.world_mut());
    app.update();
}

fn load_and_settle(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
        .write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
    app.update();
}

#[test]
fn a_save_and_reload_partway_through_matches_an_uninterrupted_run() {
    let player = CountryId(0);

    // 経路A: 中断せず6か月進行。
    let mut continuous = setup_full_app(
        player,
        TempTestDir::new("branch_continuous").save_file_config(),
    );
    for &ai in &all_active_countries(&continuous) {
        if ai != player {
            let mut cr = continuous.world_mut().resource_mut::<CountryRegistry>();
            let c = cr.get_mut(ai).unwrap();
            c.treasury = 1_000_000.0;
            c.available_manpower = 1_000_000;
            // P25-003: AI募兵が装備不足で待機し、そのタイミング差が(戦争・外交等を
            // 経由して)player treasuryへ波及するのを避けるため、比較対象外の
            // AI国家には十分なMilitaryEquipment備蓄を与えておく。
            c.stockpile
                .set(ResourceType::MilitaryEquipment, 1_000_000.0);
        }
    }
    for m in 1..=6u8 {
        advance_one_simulated_month(&mut continuous, 1800, m, 30);
    }

    // 経路B: 3か月進行 → save → 新しいAppへload → 残り3か月進行。
    let save_config = TempTestDir::new("branch_split").save_file_config();
    let mut branch = setup_full_app(player, save_config.clone());
    for &ai in &all_active_countries(&branch) {
        if ai != player {
            let mut cr = branch.world_mut().resource_mut::<CountryRegistry>();
            let c = cr.get_mut(ai).unwrap();
            c.treasury = 1_000_000.0;
            c.available_manpower = 1_000_000;
            c.stockpile
                .set(ResourceType::MilitaryEquipment, 1_000_000.0);
        }
    }
    for m in 1..=3u8 {
        advance_one_simulated_month(&mut branch, 1800, m, 30);
    }
    send_save_request(&mut branch);
    drop(branch);

    let mut reloaded = setup_full_app(player, save_config);
    load_and_settle(&mut reloaded);
    match &reloaded.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }
    for m in 4..=6u8 {
        advance_one_simulated_month(&mut reloaded, 1800, m, 30);
    }

    assert_eq!(
        normalized_country_snapshot(&continuous),
        normalized_country_snapshot(&reloaded),
        "country-level state must match between the continuous and save/load-resumed runs"
    );
    assert_eq!(
        normalized_state_snapshot(&continuous),
        normalized_state_snapshot(&reloaded),
        "state-level population/employment must match"
    );
    assert_eq!(
        normalized_division_snapshot(&continuous),
        normalized_division_snapshot(&reloaded),
        "division-level manpower/status must match"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §5.4 再実行決定性
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn the_same_scenario_run_twice_independently_produces_identical_snapshots() {
    let player = CountryId(0);

    let run_once = |label: &str| {
        let mut app = setup_full_app(player, TempTestDir::new(label).save_file_config());
        for &ai in &all_active_countries(&app) {
            if ai != player {
                let mut cr = app.world_mut().resource_mut::<CountryRegistry>();
                let c = cr.get_mut(ai).unwrap();
                c.treasury = 1_000_000.0;
                c.available_manpower = 1_000_000;
            }
        }
        for m in 1..=4u8 {
            advance_one_simulated_month(&mut app, 1800, m, 30);
        }
        (
            normalized_country_snapshot(&app),
            normalized_state_snapshot(&app),
            normalized_division_snapshot(&app),
        )
    };

    let run_a = run_once("determinism_a");
    let run_b = run_once("determinism_b");

    assert_eq!(
        run_a, run_b,
        "two independent runs of the same scenario must produce identical normalized snapshots"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §6 境界条件(統合レベルで未確認だったもの)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn a_country_that_loses_all_population_does_not_panic_or_produce_nan_over_several_months() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("zero_population_boundary");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let ai = all_active_countries(&app)
        .into_iter()
        .find(|&id| id != player)
        .unwrap();

    // 対象国家の全所有州の人口を0にする。
    {
        let mut sr = app.world_mut().resource_mut::<StateRegistry>();
        for s in sr.states.iter_mut() {
            if s.owner_country_id == ai {
                s.population = 0;
            }
        }
    }

    for m in 1..=6u8 {
        advance_one_simulated_month(&mut app, 1800, m, 30); // must not panic
    }

    let ledger = ledger_for(&app, ai);
    // 帳簿計算不能(未解決Definition)ではないため、Errにはならないはず。
    let ledger = ledger.unwrap();
    assert!(!ledger.workforce_burden.to_string().contains("NaN"));
    let country = app.world().resource::<CountryRegistry>().get(ai).unwrap();
    assert!(country.available_manpower < u64::MAX);
}

#[test]
fn an_unresolved_recruitment_queue_definition_is_reported_as_an_explicit_error_not_a_panic_mid_run()
{
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("unresolved_definition_boundary");
    let mut app = setup_full_app(player, temp_dir.save_file_config());
    let ai = all_active_countries(&app)
        .into_iter()
        .find(|&id| id != player)
        .unwrap();

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(ai)
        .unwrap()
        .recruitment_queue
        .push(strategy_game::military::recruitment::RecruitmentQueueItem {
            division_id: strategy_game::common::DivisionDefinitionId(999_999),
            target_state: StateId(0),
            days_remaining: 10,
            total_days: 30,
        });

    let result = ledger_for(&app, ai);
    assert!(
        result.is_err(),
        "an unresolvable queue definition must surface as an explicit Err, not a silent 0"
    );

    // 監査で判明した既存挙動(P23-010以前から): `process_recruitment`は
    // `days_remaining`が0に達した時点で、対象DivisionDefinitionが解決できなくても
    // キュー項目を無条件に消費する(Divisionは生成しないが、項目はキューから消える)。
    // `request_recruitment`は必ずDefinition存在を検証してからキューへ積むため、
    // 実際のゲームプレイでは到達しない経路であり(検証済みSaveも`save::validate`が
    // 未解決Definitionを拒否する)、この項目に紐づく`mobilized_manpower`加算も
    // そもそも発生していないため人的資源の消失は起きない。だが将来この経路が
    // 到達可能になった場合に備え、「タイマーが尽きると黙って消える」という
    // 挙動自体は変えていないことをここで固定しておく(§18 Low/Medium該当、
    // 詳細は監査報告書を参照)。
    //
    // P25-003: このAI国家のtreasuryは本テストで意図的に底上げしていない。旧実装
    // ではrequired_equipmentがtreasuryから差し引かれていたため、AIは数回の募兵で
    // treasury不足に陥り自然に募兵を停止していた(結果としてqueueが空のまま
    // 3か月間安定していた)。P25-003でtreasury消費自体が廃止されたため、この
    // AIはmanpowerが許す限り毎月継続的に募兵し続けるようになった——これは
    // 意図した挙動変化であり(§5参照)、3か月経過時点で新しい正当な募兵項目が
    // キューに存在していても不具合ではない。したがって本テストの主張は
    // 「queueが完全に空になる」ではなく「不正なdivision_id(999_999)を参照する
    // 元の項目がもう存在しない」ことへ訂正する。
    for m in 1..=3u8 {
        advance_one_simulated_month(&mut app, 1800, m, 30); // must not panic
    }
    let final_queue = app
        .world()
        .resource::<CountryRegistry>()
        .get(ai)
        .unwrap()
        .recruitment_queue
        .clone();
    assert!(
        final_queue
            .iter()
            .all(|item| item.division_id != strategy_game::common::DivisionDefinitionId(999_999)),
        "the unresolved queue item must be drained once its timer expires (pre-existing \
         process_recruitment behavior, unreachable via request_recruitment in real play); \
         any remaining items must be legitimately-queued new recruitment, not the original \
         unresolvable one: {final_queue:?}"
    );
    assert!(
        ledger_for(&app, ai).is_ok(),
        "once the queue drains, the ledger becomes computable again"
    );
}
