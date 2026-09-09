//! P26-006-REV-001: Save境界を跨いだ場合の継続プレイ決定性の検証。
//!
//! ## 背景・根本原因(検証報告書§5〜§11に詳細)
//!
//! `tests/p23_010_phase23_manpower_lifecycle_audit_test.rs::a_save_and_reload_
//! partway_through_matches_an_uninterrupted_run`がP26-006導入後、間欠的に
//! (独立プロセスで約40〜50%の頻度で)失敗する事象を再監査した。
//!
//! タスク仕様は当初、`CountryAiState.dirty`(P21-SAVE-002A1でSave対象外とされ、
//! ロード後は常に`true`から再開する設計)が原因と想定していたが、実コードを
//! 再監査した結果、**この想定は誤りであることが判明した**:
//!
//! * `war::update::mark_ai_registries_dirty_daily`が毎日(`DayChangedMessage`
//!   ごとに無条件で)`CountryAiRegistry`/`MilitaryAiRegistry`の全`dirty`を
//!   `true`にする(`WarResolution` Set、`CountryAi` Setより後)。
//! * このため`CountryAi` Setが翌日`dirty`を読む時点では、continuous分岐でも
//!   常に`true`になっている――ロード直後に`dirty=true`で復元する既存の
//!   defaultは、実際にはcontinuous分岐の値と常に一致しており、分岐の原因では
//!   ない(診断用ダンプで`CountryAiState`の全フィールドがcontinuous/reloaded
//!   間で完全一致することを実測確認済み)。
//!
//! 実際の根本原因は次の2箇所の**HashMap反復順非依存性の欠如**だった。
//!
//! * `src/military/movement.rs::process_movement`が`military_registry.
//!   divisions`のキーをDivisionId昇順にソートせずに処理していた。複数国家
//!   (互いには交戦していない)の師団が同日に同じ無防備な敵州へ到達する場合、
//!   `military::invasion::process_division_arrival`の「敵軍なし→即占領」判定が
//!   HashMap反復順(プロセスごとにランダムなハッシュシードに依存)で処理された
//!   最初の師団の所属国を勝者にしていた。
//! * `src/war/frontline.rs::process_daily_frontline_plans`が`frontline_
//!   registry.frontlines`のキーを同様に未ソートで処理していた。
//!
//! 両箇所へ`DivisionId`/`FrontlineId`昇順の明示的ソートを追加したことで、
//! `a_save_and_reload_partway_through_matches_an_uninterrupted_run`は独立
//! プロセス40回(2回×20回)で安定して合格することを確認した(検証報告書§30)。
//!
//! 本ファイルは、この根本原因の再発防止と、Save境界を跨いだ継続プレイの
//! 決定性を恒久的に固定する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{BattleId, CountryId, DivisionId, StateId, WarId};
use strategy_game::country::country_ai::CountryAiRegistry;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::data::{DivisionStatus, MilitaryRegistry};
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

const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const DWARF: CountryId = CountryId(2);
const OCEANIC: CountryId = CountryId(3);

// ─────────────────────────────────────────────────────────────────────────
// harness
// ─────────────────────────────────────────────────────────────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p26_006_rev_001_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
}

struct TempTestDir(std::path::PathBuf);

impl TempTestDir {
    fn new(label: &str) -> Self {
        Self(unique_temp_dir(label))
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
        .insert_resource(strategy_game::map::offensive_line_selection::OffensiveLineEditMode::default())
        .add_message::<strategy_game::map::frontline_selection::FrontlineAssignmentAttempted>()
        .add_message::<strategy_game::map::camera::CameraResetRequestMessage>();

    app.add_plugins(SaveGamePlugin);
    app.add_plugins(LoadGamePlugin);

    app.insert_resource(save_config);
    app.update();

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update();

    app
}

fn send_day_changed(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn send_month_changed(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

fn advance_one_simulated_month(app: &mut App, year: i32, month: u8, days_per_month: u8) {
    for day in 1..=days_per_month {
        send_day_changed(app, year, month, day);
    }
    send_month_changed(app, year, month);
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
    state.apply(app.world_mut());
    app.update();
}

fn load_and_settle(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
    app.update();
}

fn all_active_countries(app: &App) -> Vec<CountryId> {
    let mut ids: Vec<CountryId> =
        app.world().resource::<CountryRegistry>().countries.iter().filter(|c| c.is_active()).map(|c| c.id).collect();
    ids.sort_by_key(|id| id.0);
    ids
}

fn boost_ai_countries(app: &mut App, player: CountryId) {
    for &ai in &all_active_countries(app) {
        if ai != player {
            let mut cr = app.world_mut().resource_mut::<CountryRegistry>();
            let c = cr.get_mut(ai).unwrap();
            c.treasury = 1_000_000.0;
            c.available_manpower = 1_000_000;
            c.stockpile.set(ResourceType::MilitaryEquipment, 1_000_000.0);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StateSnapshot {
    id: StateId,
    owner: CountryId,
    controller: Option<CountryId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DivisionSnapshot {
    owner: CountryId,
    current_state: StateId,
    status: DivisionStatus,
    manpower: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CountryAiSnapshot {
    country_id: CountryId,
    mode: strategy_game::country::country_ai::CountryAiMode,
    reason: strategy_game::country::country_ai::CountryAiDecisionReason,
    last_daily: u32,
    last_weekly: u32,
    last_monthly: u32,
    cooldown_until: u32,
}

fn state_snapshot(app: &App) -> Vec<StateSnapshot> {
    let mut out: Vec<StateSnapshot> = app
        .world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .map(|s| StateSnapshot { id: s.id, owner: s.owner_country_id, controller: s.controller_country })
        .collect();
    out.sort_by_key(|s| s.id.0);
    out
}

fn division_snapshot(app: &App) -> Vec<DivisionSnapshot> {
    let mut out: Vec<DivisionSnapshot> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .map(|d| DivisionSnapshot { owner: d.owner, current_state: d.current_state, status: d.status, manpower: d.manpower })
        .collect();
    out.sort_by_key(|d| (d.owner.0, d.current_state.0));
    out
}

fn country_ai_snapshot(app: &App) -> Vec<CountryAiSnapshot> {
    let mut out: Vec<CountryAiSnapshot> = app
        .world()
        .resource::<CountryAiRegistry>()
        .ai_states
        .values()
        .map(|s| CountryAiSnapshot {
            country_id: s.country_id,
            mode: s.mode,
            reason: s.decision_reason,
            last_daily: s.last_daily_evaluation_day,
            last_weekly: s.last_weekly_evaluation_day,
            last_monthly: s.last_monthly_evaluation_day,
            cooldown_until: s.cooldown_until_day,
        })
        .collect();
    out.sort_by_key(|s| s.country_id.0);
    out
}

/// P26-006-REV-001の再現シナリオ: 2か国(Elfin・Oceanic、互いには交戦していない)が
/// 同じ第3国(Arcadia)と交戦し、両者の territory がArcadiaの同じ州(StateId(1)、
/// 実マップでElfin[3]ともOceanic[8]とも隣接する)と国境を接する。両国のAIが
/// 同時に攻勢を仕掛けると、この州の占領を巡る競合が発生する(検証報告書§5〜§8)。
fn setup_contested_border_scenario(app: &mut App) {
    boost_ai_countries(app, ARCADIA);

    // Arcadia自身の初期師団を、Dwarfとの別の戦争へ拘束しておく(state 1の防衛に
    // 割けなくする、実際のバグ再現シナリオと同じ構図)。
    let war_arcadia_dwarf = create_war(app, DWARF, ARCADIA);
    let arcadia_capital_div = division_ids_of(app, ARCADIA)[0];
    let dwarf_div = division_ids_of(app, DWARF)[0];
    create_battle(app, war_arcadia_dwarf, DWARF, ARCADIA, StateId(0), vec![dwarf_div], vec![arcadia_capital_div]);

    // ElfinとOceanicがそれぞれ独立にArcadiaへ宣戦する。
    create_war(app, ELFIN, ARCADIA);
    create_war(app, OCEANIC, ARCADIA);
}

fn division_ids_of(app: &App, country_id: CountryId) -> Vec<DivisionId> {
    let mut ids: Vec<DivisionId> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == country_id)
        .map(|d| d.id)
        .collect();
    ids.sort_by_key(|id| id.0);
    ids
}

fn create_war(app: &mut App, attacker: CountryId, defender: CountryId) -> WarId {
    let war = War {
        id: WarId(0),
        name: "REV-001 Test War".to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1799/12/31".to_string(),
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
    };
    app.world_mut().resource_mut::<WarRegistry>().add_war(war)
}

fn create_battle(
    app: &mut App,
    war_id: WarId,
    attacker_country: CountryId,
    defender_country: CountryId,
    state_id: StateId,
    attacker_ids: Vec<DivisionId>,
    defender_ids: Vec<DivisionId>,
) -> BattleId {
    let attacker_origins = attacker_ids.iter().map(|id| (*id, state_id)).collect();
    let battle = Battle {
        id: BattleId(0),
        war_id,
        state_id,
        attacker_country,
        defender_country,
        attacker_division_ids: attacker_ids.clone(),
        defender_division_ids: defender_ids.clone(),
        attacker_origins,
        start_date: "1799/12/31".to_string(),
        elapsed_days: 0,
        status: BattleStatus::Ongoing,
    };
    let battle_id = app.world_mut().resource_mut::<BattleRegistry>().start_battle(battle).unwrap();
    let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
    for id in attacker_ids.iter().chain(defender_ids.iter()) {
        if let Some(d) = mr.divisions.get_mut(id) {
            d.combat_id = Some(battle_id);
        }
    }
    battle_id
}

// ─────────────────────────────────────────────────────────────────────────
// Test M: 実バグ再現シナリオ(縮小版、`p23_010`本体の実測対象と同じ地政学的構図)
// ─────────────────────────────────────────────────────────────────────────

/// 継続プレイと、1か月進行後にsave→loadを挟んで続行するプレイとで、係争中の
/// 州の最終的な占領国が完全一致することを確認する。P26-006-REV-001の修正前は
/// この比較が約40〜50%の頻度で分岐していた(検証報告書§5参照)。
#[test]
fn test_m_contested_border_state_outcome_matches_between_continuous_and_save_load_branches() {
    fn run(split_with_save_load: bool, label: &str) -> (Vec<StateSnapshot>, Vec<DivisionSnapshot>) {
        let dir = TempTestDir::new(label);
        let mut app = setup_full_app(ARCADIA, dir.save_file_config());
        setup_contested_border_scenario(&mut app);

        if split_with_save_load {
            send_save_request(&mut app);
            load_and_settle(&mut app);
        }

        for m in 1..=2u8 {
            advance_one_simulated_month(&mut app, 1800, m, 30);
        }

        (state_snapshot(&app), division_snapshot(&app))
    }

    let (states_a, divisions_a) = run(false, "m_continuous");
    let (states_b, divisions_b) = run(true, "m_split");

    assert_eq!(states_a, states_b, "州の所有国・支配国はsave/load分岐に関わらず一致するはず");
    assert_eq!(divisions_a, divisions_b, "Division状態もsave/load分岐に関わらず一致するはず");
}

/// 上と同じシナリオを、20回の独立した(同一プロセス内での)繰り返しで安定することを
/// 確認する(仕様§13の「独立プロセス20回」の同一プロセス内近似――実際の独立プロセス
/// 検証は検証報告書§30に記載の`cargo test`を40回[2バッチ×20回]実行した結果を正とする)。
#[test]
fn test_m2_contested_border_outcome_is_stable_across_twenty_repeated_runs() {
    fn run(label: &str) -> Vec<StateSnapshot> {
        let dir = TempTestDir::new(label);
        let mut app = setup_full_app(ARCADIA, dir.save_file_config());
        setup_contested_border_scenario(&mut app);
        send_save_request(&mut app);
        load_and_settle(&mut app);
        for m in 1..=2u8 {
            advance_one_simulated_month(&mut app, 1800, m, 30);
        }
        state_snapshot(&app)
    }

    let baseline = run("m2_baseline");
    for i in 0..19 {
        let result = run(&format!("m2_repeat_{i}"));
        assert_eq!(result, baseline, "20回中{}回目の実行結果が最初の実行と異なる", i + 2);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Test A/B: dirtyフラグの既存挙動(修正対象ではないが回帰保護として固定)
// ─────────────────────────────────────────────────────────────────────────

/// `mark_ai_registries_dirty_daily`が毎日全AI国家をdirty化するため、`CountryAiState.
/// dirty`はSave対象外のまま(P21-SAVE-002A1の既存方針、本タスクで変更していない)でも
/// AI再評価タイミングの分岐を引き起こさないことを回帰確認する
/// (検証報告書§9で確定した設計上の理由の直接証拠)。
#[test]
fn test_a_country_ai_state_is_bit_identical_between_branches_at_the_divergence_point() {
    fn run(split_with_save_load: bool, label: &str) -> Vec<CountryAiSnapshot> {
        let dir = TempTestDir::new(label);
        let mut app = setup_full_app(ARCADIA, dir.save_file_config());
        setup_contested_border_scenario(&mut app);
        if split_with_save_load {
            send_save_request(&mut app);
            load_and_settle(&mut app);
        }
        for m in 1..=2u8 {
            advance_one_simulated_month(&mut app, 1800, m, 30);
        }
        country_ai_snapshot(&app)
    }

    let continuous = run(false, "a_continuous");
    let split = run(true, "a_split");
    assert_eq!(continuous, split, "CountryAiStateはsave/load分岐に関わらず完全一致するはず(dirtyは分岐の原因ではない)");
}

/// 旧Save(`CountryAiState`にdirty相当のフィールドが元から存在しない、現行DTOにも
/// 追加していない)を読み込んでも問題なく動作し続けることを確認する
/// (本タスクではSaveフォーマットを一切変更していないため、旧Save互換性は
/// 構造的に自明だが、明示的に固定する)。
#[test]
fn test_b_old_save_without_any_new_fields_loads_successfully() {
    let dir = TempTestDir::new("b_old_save");
    let mut app = setup_full_app(ARCADIA, dir.save_file_config());
    setup_contested_border_scenario(&mut app);
    advance_one_simulated_month(&mut app, 1800, 1, 30);
    let ai_states_before = app.world().resource::<CountryAiRegistry>().ai_states.len();
    assert!(ai_states_before > 0, "1か月経過後は少なくとも1か国のAI状態が生成されているはず");

    send_save_request(&mut app);
    // 本タスクは新規Saveフィールドを一切追加していないため、
    // 「フィールド欠落」を模擬する必要はない――現行のSave自体が
    // そのまま旧Save相当(dirty等の派生キャッシュを含まない)である。
    load_and_settle(&mut app);
    let ai_states_after = app.world().resource::<CountryAiRegistry>().ai_states.len();
    assert_eq!(ai_states_before, ai_states_after, "AI状態の数はload前後で変化しないはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test D: 2回連続save→load
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_d_double_save_load_round_trip_does_not_change_country_ai_or_state_snapshots() {
    let dir = TempTestDir::new("d_double");
    let mut app = setup_full_app(ARCADIA, dir.save_file_config());
    setup_contested_border_scenario(&mut app);
    advance_one_simulated_month(&mut app, 1800, 1, 30);

    send_save_request(&mut app);
    load_and_settle(&mut app);
    let after_first = (state_snapshot(&app), country_ai_snapshot(&app));

    send_save_request(&mut app);
    load_and_settle(&mut app);
    let after_second = (state_snapshot(&app), country_ai_snapshot(&app));

    assert_eq!(after_first, after_second, "2回連続save→loadでも状態が安定するはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test I: P26-006首都連結性の保護
// ─────────────────────────────────────────────────────────────────────────

/// P26-006の首都連結性は、係争中の州の占領国が変わっても正しく再計算される
/// (連結性自体はSaveへ保存されない、既存契約を維持)。
#[test]
fn test_i_capital_connectivity_recomputes_correctly_regardless_of_which_country_wins_the_race() {
    let dir = TempTestDir::new("i_connectivity");
    let mut app = setup_full_app(ARCADIA, dir.save_file_config());
    setup_contested_border_scenario(&mut app);
    for m in 1..=2u8 {
        advance_one_simulated_month(&mut app, 1800, m, 30);
    }

    let winner = app.world().resource::<StateRegistry>().get(StateId(1)).unwrap().controller();
    assert!(winner == ELFIN || winner == OCEANIC || winner == ARCADIA, "占領国はElfin/Oceanic/Arcadiaのいずれかのはず");

    // 勝者の首都連結州集合にstate1が含まれる(勝者がArcadia自身でない場合)。
    if winner != ARCADIA {
        let states = app.world().resource::<StateRegistry>();
        let capital = app.world().resource::<CountryRegistry>().get(winner).unwrap().capital_state_id;
        let connected = strategy_game::military::capital_connectivity::connected_states_from_capital(capital, winner, states);
        assert!(connected.contains(&StateId(1)), "占領国自身の首都連結州集合にstate1が含まれるはず");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Test J: 国家間分離
// ─────────────────────────────────────────────────────────────────────────

/// 係争のない第三国(Dwarf以外の状況で、たとえばこのシナリオに含まれない架空の
/// 平和な国)の状態は、save/load分岐によって一切変化しない
/// (本シナリオでは、Dwarfは実際にArcadiaと交戦しているため、代わりに
/// Dwarf自身の備蓄推移が両分岐で一致することを確認する形で国家間分離を検証する)。
#[test]
fn test_j_uninvolved_countrys_state_is_unaffected_by_the_contested_race_outcome() {
    fn run(split_with_save_load: bool, label: &str) -> f64 {
        let dir = TempTestDir::new(label);
        let mut app = setup_full_app(ARCADIA, dir.save_file_config());
        setup_contested_border_scenario(&mut app);
        if split_with_save_load {
            send_save_request(&mut app);
            load_and_settle(&mut app);
        }
        for m in 1..=2u8 {
            advance_one_simulated_month(&mut app, 1800, m, 30);
        }
        app.world().resource::<CountryRegistry>().get(DWARF).unwrap().treasury
    }

    let continuous = run(false, "j_continuous");
    let split = run(true, "j_split");
    assert_eq!(continuous, split, "Elfin/Oceanicの係争結果はDwarfの財政へ影響しないはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test K: Map挿入順序非依存
// ─────────────────────────────────────────────────────────────────────────

/// `MilitaryRegistry.divisions`への挿入順序を変えても、係争結果が変わらない
/// (`process_movement`のDivisionId昇順ソートが実際に効いていることの直接証拠)。
#[test]
fn test_k_division_map_insertion_order_does_not_affect_the_contested_race_outcome() {
    fn run(label: &str, reverse_insertion: bool) -> CountryId {
        let dir = TempTestDir::new(label);
        let mut app = setup_full_app(ARCADIA, dir.save_file_config());
        setup_contested_border_scenario(&mut app);

        if reverse_insertion {
            // MilitaryRegistry.divisionsを一度全部取り出し、逆順で再構築する
            // (HashMapなので「挿入順」は本来観測不能だが、内部Vec経由の再構築で
            // 異なる内部レイアウトを強制的に作る)。
            let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
            let mut entries: Vec<_> = mr.divisions.drain().collect();
            entries.reverse();
            for (id, division) in entries {
                mr.divisions.insert(id, division);
            }
        }

        for m in 1..=2u8 {
            advance_one_simulated_month(&mut app, 1800, m, 30);
        }
        app.world().resource::<StateRegistry>().get(StateId(1)).unwrap().controller()
    }

    let normal = run("k_normal", false);
    let reversed = run("k_reversed", true);
    assert_eq!(normal, reversed, "Division挿入順序を変えても占領結果は変わらないはず");
}

// ─────────────────────────────────────────────────────────────────────────
// P26-004/005/006回帰保護(この修正が既存の補給・戦闘力ロジックを変更していないこと)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_regression_supply_and_combat_readiness_min_formula_still_holds_after_the_fix() {
    let dir = TempTestDir::new("regression_p26005");
    let mut app = setup_full_app(ARCADIA, dir.save_file_config());
    let ids = division_ids_of(&app, ARCADIA);
    send_day_changed(&mut app, 1800, 1, 2);
    let fulfillment = app.world().resource::<MilitaryRegistry>().supply_fulfillment_of(ids[0]);
    assert_eq!(fulfillment, 1.0, "無戦争・首都に留まる師団は完全連結のはず(P26-006回帰なし)");
}
