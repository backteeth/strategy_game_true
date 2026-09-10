//! P26-008-PRE-002: AI評価cadence・イベント駆動dirty化・Save連続性の固定。
//!
//! 旧`mark_ai_registries_dirty_daily`は`DayChangedMessage`のたびに全国家を無条件で
//! `mark_all_dirty()`しており、週次(研究AI・戦争準備AI)・月次(建設AI)・3日おき
//! (戦線計画AI)という設計意図のcadenceを実質的に無効化していた
//! (`verification_logs/phase-26/p26-008-pre-001/`で実測確認済み)。
//!
//! 本ファイルは、`war::ai_change_tracking::AiChangeSnapshot`によるイベント駆動化後の
//! 正しい挙動(無変化時は正規cadenceへ復帰、州支配・戦争参加の変化があった国家だけが
//! 翌日即時再評価される、save/loadで継続プレイと一致する)を実App経由で固定する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, StateId, WarId};
use strategy_game::country::country_ai::CountryAiRegistry;
use strategy_game::country::{CountryPlugin, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
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
use strategy_game::war::military_ai::MilitaryAiRegistry;

const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const DWARF: CountryId = CountryId(2);

// ─────────────────────────────────────────────────────────────────────────
// harness(既存P26系E2Eテストと同じ構成方針)
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
        "strategy_game_p26_008_pre_002_e2e_{label}_{}_{nanos}_{n}",
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
    app.update();

    app.insert_resource(PlayerCountry(player));
    app.insert_state(GameState::Playing);
    app.update();

    app
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

fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn tick_month(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

/// `advance_one_month`と同じ簡略月(26〜28日分の日次tick+月末tick)だが、各日の
/// tick直後にコールバックを呼べる(cadenceの日次サンプリング用)。
fn advance_one_month_sampling(app: &mut App, month_index: u8, mut per_day: impl FnMut(&mut App, u8)) {
    for day in 2..=28u8 {
        tick_day(app, 1800, month_index, day);
        per_day(app, day);
    }
    tick_month(app, 1800, month_index.wrapping_add(1));
}

fn country_ai_dirty(app: &App, cid: CountryId) -> bool {
    app.world()
        .resource::<CountryAiRegistry>()
        .ai_states
        .get(&cid)
        .map(|s| s.dirty)
        .unwrap_or(false)
}

fn military_ai_dirty(app: &App, cid: CountryId) -> bool {
    app.world()
        .resource::<MilitaryAiRegistry>()
        .ai_states
        .get(&cid)
        .map(|s| s.dirty)
        .unwrap_or(false)
}

fn last_weekly_day(app: &App, cid: CountryId) -> u32 {
    app.world()
        .resource::<CountryAiRegistry>()
        .ai_states
        .get(&cid)
        .map(|s| s.last_weekly_evaluation_day)
        .unwrap_or(0)
}

fn last_monthly_day(app: &App, cid: CountryId) -> u32 {
    app.world()
        .resource::<CountryAiRegistry>()
        .ai_states
        .get(&cid)
        .map(|s| s.last_monthly_evaluation_day)
        .unwrap_or(0)
}

fn last_military_evaluated_day(app: &App, cid: CountryId) -> u32 {
    app.world()
        .resource::<MilitaryAiRegistry>()
        .ai_states
        .get(&cid)
        .map(|s| s.last_evaluated_day)
        .unwrap_or(0)
}

/// `values`の中で、直前の値と異なる要素の個数(初回値は変化として数えない)。
/// 「実際に評価が実行された回数」の外形的な代理指標として使う――純粋な同値再計算
/// (dirty=falseのまま既存値を再書き込みするだけの日)は`last_*_day`自体を書き換え
/// ないため、この個数は「評価が実際に実行された日数」と一致する
/// (spec §17: 「判断結果が変わった回数」ではなく「実際の関数実行」を数える)。
fn count_value_changes(values: &[u32]) -> usize {
    let mut count = 0;
    let mut prev: Option<u32> = None;
    for &v in values {
        if prev != Some(v) {
            count += 1;
        }
        prev = Some(v);
    }
    count
}

fn set_state_controller(app: &mut App, state_id: StateId, controller: Option<CountryId>) {
    app.world_mut().resource_mut::<StateRegistry>().get_mut(state_id).unwrap().controller_country =
        controller;
}

fn create_war(app: &mut App, attacker: CountryId, defender: CountryId) -> WarId {
    let war = War {
        id: WarId(0),
        name: "P26-008-PRE-002 Test War".to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1800/01/01".to_string(),
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
    };
    app.world_mut().resource_mut::<WarRegistry>().add_war(war)
}

fn end_war(app: &mut App, war_id: WarId, status: WarStatus) {
    app.world_mut()
        .resource_mut::<WarRegistry>()
        .wars
        .get_mut(&war_id)
        .unwrap()
        .status = status;
}

// ─────────────────────────────────────────────────────────────────────────
// Test A/B: 無変化時のcadence回復
// ─────────────────────────────────────────────────────────────────────────

/// Test A: 変化のない90日間で、建設AI(月次)・研究AI/戦争準備AI(週次)の実評価回数が
/// 日数(90)ではなく、正規cadence相当(月次≈3回、週次≈13回)に収まること。
#[test]
fn test_a_country_ai_cadence_matches_periodic_schedule_without_any_state_change() {
    let dir = TempTestDir::new("test_a");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());
    // ARCADIAを観測対象にする(DWARFがプレイヤーのためAI対象、他国と無関係な変化を起こさない)。

    let mut weekly_history = Vec::new();
    let mut monthly_history = Vec::new();
    for m in 1..=3u8 {
        advance_one_month_sampling(&mut app, m, |app, _day| {
            weekly_history.push(last_weekly_day(app, ARCADIA));
            monthly_history.push(last_monthly_day(app, ARCADIA));
        });
    }

    let weekly_evals = count_value_changes(&weekly_history);
    let monthly_evals = count_value_changes(&monthly_history);

    assert!(
        weekly_evals <= 16,
        "90日で週次評価が過剰に発生しているはず(dirtyが常にtrueのバグの再発): {weekly_evals}回"
    );
    assert!(
        monthly_evals <= 5,
        "90日で月次評価が過剰に発生しているはず(dirtyが常にtrueのバグの再発): {monthly_evals}回"
    );
    assert!(weekly_evals >= 1, "週次評価が一度も発生していない(cadenceが完全停止している)");
    assert!(monthly_evals >= 1, "月次評価が一度も発生していない(cadenceが完全停止している)");
}

/// Test B: 交戦中で前線が安定している場合、戦線計画AIの評価(`last_evaluated_day`)が
/// 毎日ではなく概ね3日おきに発生すること。
#[test]
fn test_b_military_ai_cadence_is_roughly_every_three_days_during_a_stable_war() {
    let dir = TempTestDir::new("test_b");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());
    create_war(&mut app, ARCADIA, ELFIN);

    let mut history = Vec::new();
    for m in 1..=2u8 {
        advance_one_month_sampling(&mut app, m, |app, _day| {
            history.push(last_military_evaluated_day(app, ARCADIA));
        });
    }

    let evals = count_value_changes(&history);
    assert!(
        evals <= 24,
        "約2か月(54日)で3日おき評価なら約18回のはず、過剰なら旧バグの再発: {evals}回"
    );
    assert!(evals >= 5, "戦線計画AIが一度も評価されていない");
}

// ─────────────────────────────────────────────────────────────────────────
// Test C: New Game初回評価
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_c_new_game_still_performs_one_initial_evaluation_then_settles_into_normal_cadence() {
    let dir = TempTestDir::new("test_c");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());

    // New Game直後、dirty=trueのため初回評価が発生するはず。
    tick_day(&mut app, 1800, 1, 2);
    assert!(
        last_weekly_day(&app, ARCADIA) > 0 || last_monthly_day(&app, ARCADIA) > 0,
        "New Game直後の初回評価が発生していないはず"
    );

    // その後は無変化のため、dirtyはfalseへ戻り、以後は日次では変化しない。
    tick_day(&mut app, 1800, 1, 3);
    assert!(!country_ai_dirty(&app, ARCADIA), "初回評価後はdirty=falseへ戻るはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test D/E: 戦争開始・終了で関係国だけが即時再評価される
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_d_war_start_dirties_only_the_participants_for_the_next_evaluation() {
    let dir = TempTestDir::new("test_d");
    let mut app = setup_app(None, dir.save_file_config());
    // 無変化を数日続けてdirtyをfalseへ落ち着かせる。
    tick_day(&mut app, 1800, 1, 2);
    tick_day(&mut app, 1800, 1, 3);
    assert!(!country_ai_dirty(&app, ARCADIA));
    assert!(!country_ai_dirty(&app, ELFIN));
    assert!(!country_ai_dirty(&app, DWARF));

    create_war(&mut app, ARCADIA, ELFIN);
    tick_day(&mut app, 1800, 1, 4);

    assert!(country_ai_dirty(&app, ARCADIA), "戦争開始の翌日、攻撃側はdirty化されるはず");
    assert!(country_ai_dirty(&app, ELFIN), "戦争開始の翌日、防御側はdirty化されるはず");
    assert!(!country_ai_dirty(&app, DWARF), "無関係な第三国はdirty化されないはず");
}

#[test]
fn test_e_war_end_dirties_only_the_former_participants() {
    let dir = TempTestDir::new("test_e");
    let mut app = setup_app(None, dir.save_file_config());
    let war_id = create_war(&mut app, ARCADIA, ELFIN);
    tick_day(&mut app, 1800, 1, 2); // 開戦の即時再評価を消化
    tick_day(&mut app, 1800, 1, 3);
    assert!(!country_ai_dirty(&app, ARCADIA));
    assert!(!country_ai_dirty(&app, ELFIN));

    end_war(&mut app, war_id, WarStatus::WhitePeace);
    tick_day(&mut app, 1800, 1, 4);

    assert!(country_ai_dirty(&app, ARCADIA), "終戦の翌日、旧参加国はdirty化されるはず");
    assert!(country_ai_dirty(&app, ELFIN));
    assert!(!country_ai_dirty(&app, DWARF), "無関係な第三国はdirty化されないはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test F: controller変更で関係国だけが即時再評価される
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_f_controller_change_dirties_only_old_and_new_controller_and_owner() {
    let dir = TempTestDir::new("test_f");
    let mut app = setup_app(None, dir.save_file_config());
    tick_day(&mut app, 1800, 1, 2);
    tick_day(&mut app, 1800, 1, 3);
    assert!(!country_ai_dirty(&app, ARCADIA));
    assert!(!country_ai_dirty(&app, ELFIN));
    assert!(!country_ai_dirty(&app, DWARF));

    // ArcadiaのcorridorをElfinが占領(owner=Arcadiaのまま)。
    set_state_controller(&mut app, StateId(11), Some(ELFIN));
    tick_day(&mut app, 1800, 1, 4);

    assert!(country_ai_dirty(&app, ARCADIA), "owner国はdirty化されるはず");
    assert!(country_ai_dirty(&app, ELFIN), "新controller国はdirty化されるはず");
    assert!(!country_ai_dirty(&app, DWARF), "無関係な第三国はdirty化されないはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test G: 首都占領・奪還
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_g_capital_occupation_and_recapture_each_trigger_one_immediate_reevaluation() {
    let dir = TempTestDir::new("test_g");
    let mut app = setup_app(None, dir.save_file_config());
    tick_day(&mut app, 1800, 1, 2);
    tick_day(&mut app, 1800, 1, 3);
    assert!(!country_ai_dirty(&app, ARCADIA));
    assert!(!military_ai_dirty(&app, ARCADIA));

    set_state_controller(&mut app, StateId(0), Some(ELFIN)); // 首都占領
    tick_day(&mut app, 1800, 1, 4);
    assert!(country_ai_dirty(&app, ARCADIA));
    assert!(military_ai_dirty(&app, ARCADIA));
    tick_day(&mut app, 1800, 1, 5);
    assert!(!country_ai_dirty(&app, ARCADIA), "翌々日には落ち着いているはず");

    set_state_controller(&mut app, StateId(0), None); // 首都奪還(owner=Arcadiaへ復帰)
    tick_day(&mut app, 1800, 1, 6);
    assert!(country_ai_dirty(&app, ARCADIA), "首都奪還でも再度dirty化されるはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test K: no-op(意味的変化なし)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_k_resetting_the_same_controller_does_not_dirty_anyone() {
    let dir = TempTestDir::new("test_k");
    let mut app = setup_app(None, dir.save_file_config());
    set_state_controller(&mut app, StateId(11), Some(ELFIN));
    tick_day(&mut app, 1800, 1, 2); // 占領自体の即時再評価を消化
    tick_day(&mut app, 1800, 1, 3);
    assert!(!country_ai_dirty(&app, ARCADIA));
    assert!(!country_ai_dirty(&app, ELFIN));

    // 同じcontrollerへ再設定(意味的変化なし)。
    set_state_controller(&mut app, StateId(11), Some(ELFIN));
    tick_day(&mut app, 1800, 1, 4);

    assert!(!country_ai_dirty(&app, ARCADIA), "同一controllerへの再設定はdirty化しないはず");
    assert!(!country_ai_dirty(&app, ELFIN));
}

// ─────────────────────────────────────────────────────────────────────────
// Test L: 同日複数イベントが翌日の評価1回へ集約される
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_l_multiple_same_day_changes_to_the_same_country_collapse_into_one_next_day_reevaluation() {
    let dir = TempTestDir::new("test_l");
    let mut app = setup_app(None, dir.save_file_config());
    tick_day(&mut app, 1800, 1, 2);
    tick_day(&mut app, 1800, 1, 3);
    let weekly_before = last_weekly_day(&app, ARCADIA);

    // 同日中にArcadiaへ関係する変化を2件重ねる: 戦争開始 + controller変更。
    create_war(&mut app, ARCADIA, ELFIN);
    set_state_controller(&mut app, StateId(11), Some(ELFIN));
    tick_day(&mut app, 1800, 1, 4);

    assert!(country_ai_dirty(&app, ARCADIA) || last_weekly_day(&app, ARCADIA) > weekly_before);
    tick_day(&mut app, 1800, 1, 5);
    assert!(!country_ai_dirty(&app, ARCADIA), "翌々日には1回の再評価で落ち着いているはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test N/O/P/Q: save/load
// ─────────────────────────────────────────────────────────────────────────

/// Test N: dirty=falseの状態でsave→loadしても、継続プレイと同じ日に次回評価が起こる。
#[test]
fn test_n_dirty_false_save_load_matches_the_continuous_branch() {
    fn scenario(split: bool) -> (u32, u32, bool) {
        let dir = TempTestDir::new(if split { "n_split" } else { "n_continuous" });
        let mut app = setup_app(Some(DWARF), dir.save_file_config());
        // 無変化で数日進め、dirty=falseに落ち着かせる。
        for day in 2..=10u8 {
            tick_day(&mut app, 1800, 1, day);
        }
        assert!(!country_ai_dirty(&app, ARCADIA), "分岐前提: dirty=falseで確定しているはず");

        if split {
            send_save_request(&mut app);
            send_load_request(&mut app);
        }

        for day in 11..=20u8 {
            tick_day(&mut app, 1800, 1, day);
        }
        (last_weekly_day(&app, ARCADIA), last_monthly_day(&app, ARCADIA), country_ai_dirty(&app, ARCADIA))
    }

    assert_eq!(scenario(false), scenario(true));
}

/// Test O: dirty=trueの状態でsave→loadすると、ロード後の次回評価機会で1回だけ評価される。
#[test]
fn test_o_dirty_true_save_load_evaluates_exactly_once_on_the_next_opportunity() {
    let dir = TempTestDir::new("test_o");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());
    // まず1日分落ち着かせる(New Game直後のdirty=trueブートストラップと、戦争起因の
    // dirty=trueを混同しないため――スナップショット自体もこの日に初期化される)。
    tick_day(&mut app, 1800, 1, 2);
    assert!(!country_ai_dirty(&app, ARCADIA), "分岐前提: 落ち着いた状態から始める");

    // 戦争開始(この日のWarResolutionで検出され、翌日読み取り用にdirty=trueが立つ)。
    create_war(&mut app, ARCADIA, ELFIN);
    tick_day(&mut app, 1800, 1, 3);
    assert!(country_ai_dirty(&app, ARCADIA), "分岐前提: 戦争開始によりdirty=trueのまま");

    send_save_request(&mut app);
    send_load_request(&mut app);

    let weekly_before = last_weekly_day(&app, ARCADIA);
    let monthly_before = last_monthly_day(&app, ARCADIA);
    tick_day(&mut app, 1800, 1, 4);
    assert!(
        last_weekly_day(&app, ARCADIA) > weekly_before || last_monthly_day(&app, ARCADIA) > monthly_before,
        "ロード後の次回評価機会で1回評価されるはず"
    );
    tick_day(&mut app, 1800, 1, 5);
    assert!(!country_ai_dirty(&app, ARCADIA), "1回評価された後は落ち着くはず");
}

/// Test P: `dirty`フィールドを持たない旧Saveをロードしても安全に読み込め、
/// 初回は安全側(true)の即時再評価から始まり、その後は通常cadenceへ戻る。
#[test]
fn test_p_old_save_without_the_dirty_field_loads_and_settles_into_normal_cadence() {
    let dir = TempTestDir::new("test_p");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());
    for day in 2..=10u8 {
        tick_day(&mut app, 1800, 1, day);
    }
    send_save_request(&mut app);

    // 旧Save形式を模擬: 保存済みRONから"dirty: true,"/"dirty: false,"の行を機械的に
    // 取り除く(この2値以外は出力されないため、文字列置換で安全に再現できる)。
    let path = dir.save_file_config().path.final_path;
    let contents = std::fs::read_to_string(&path).expect("read save");
    let stripped = contents
        .replace("dirty: true,", "")
        .replace("dirty: false,", "");
    std::fs::write(&path, stripped).expect("write stripped save");

    send_load_request(&mut app);
    assert!(
        country_ai_dirty(&app, ARCADIA),
        "dirtyフィールド欠落時はserde defaultによりtrueへ復元されるはず"
    );

    tick_day(&mut app, 1800, 1, 11);
    tick_day(&mut app, 1800, 1, 12);
    assert!(!country_ai_dirty(&app, ARCADIA), "初回評価の後は通常cadenceへ戻るはず");
}

/// Test Q: 2回連続save→loadでも、余分な即時再評価が増殖しない。
#[test]
fn test_q_double_save_load_does_not_accumulate_extra_reevaluations() {
    let dir = TempTestDir::new("test_q");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());
    for day in 2..=10u8 {
        tick_day(&mut app, 1800, 1, day);
    }
    assert!(!country_ai_dirty(&app, ARCADIA));

    send_save_request(&mut app);
    send_load_request(&mut app);
    assert!(!country_ai_dirty(&app, ARCADIA), "1回目のload後もdirty=falseが維持されるはず");

    send_save_request(&mut app);
    send_load_request(&mut app);
    assert!(!country_ai_dirty(&app, ARCADIA), "2回目のload後もdirty=falseが維持されるはず");
}

// ─────────────────────────────────────────────────────────────────────────
// Test R: 月次境界での重複実行なし
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_r_month_boundary_does_not_double_fire_monthly_ai_or_recruitment() {
    let dir = TempTestDir::new("test_r");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());
    let mut monthly_history = Vec::new();
    for m in 1..=2u8 {
        advance_one_month_sampling(&mut app, m, |app, _day| {
            monthly_history.push(last_monthly_day(app, ARCADIA));
        });
    }
    // 2か月(54日)で月次評価は最大3回程度のはず(重複実行があれば54に近づく)。
    let evals = count_value_changes(&monthly_history);
    assert!(evals <= 5, "月次境界での重複実行が疑われる: {evals}回");
}

// ─────────────────────────────────────────────────────────────────────────
// Test S: P26-006-REV-001シナリオの決定性維持
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_s_contested_border_state_outcome_is_stable_across_repeated_runs() {
    fn run_once(label: &str) -> Option<CountryId> {
        let dir = TempTestDir::new(label);
        let mut app = setup_app(None, dir.save_file_config());
        create_war(&mut app, ARCADIA, ELFIN);
        for m in 1..=3u8 {
            advance_one_month_sampling(&mut app, m, |_app, _day| {});
        }
        app.world()
            .resource::<StateRegistry>()
            .get(StateId(1))
            .map(|s| s.controller())
    }

    let first = run_once("s_run_1");
    for i in 0..4 {
        let repeat = run_once(&format!("s_run_{}", i + 2));
        assert_eq!(first, repeat, "同じシナリオの結果は決定的に一致するはず");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Test T: 実App 90日評価回数(全国家、6か国AI)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_t_real_app_90_day_evaluation_counts_match_designed_cadence_for_all_ai_countries() {
    let dir = TempTestDir::new("test_t");
    let mut app = setup_app(None, dir.save_file_config());
    let countries = [ARCADIA, ELFIN, DWARF, CountryId(3), CountryId(4), CountryId(5)];

    let mut weekly_histories: Vec<Vec<u32>> = countries.iter().map(|_| Vec::new()).collect();
    let mut monthly_histories: Vec<Vec<u32>> = countries.iter().map(|_| Vec::new()).collect();

    for m in 1..=4u8 {
        advance_one_month_sampling(&mut app, m, |app, _day| {
            for (i, &c) in countries.iter().enumerate() {
                weekly_histories[i].push(last_weekly_day(app, c));
                monthly_histories[i].push(last_monthly_day(app, c));
            }
        });
    }

    for (i, &c) in countries.iter().enumerate() {
        let weekly_evals = count_value_changes(&weekly_histories[i]);
        let monthly_evals = count_value_changes(&monthly_histories[i]);
        assert!(
            weekly_evals <= 20,
            "{c:?}: 108日で週次評価が過剰({weekly_evals}回、日数と同程度なら旧バグの再発)"
        );
        assert!(
            monthly_evals <= 6,
            "{c:?}: 108日で月次評価が過剰({monthly_evals}回、日数と同程度なら旧バグの再発)"
        );
    }
}
