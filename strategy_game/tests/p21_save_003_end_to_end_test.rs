//! P21-SAVE-003: 起動直後の「続きから」ロード導線のエンドツーエンド統合テスト。
//!
//! `tests/p21_save_002e_end_to_end_test.rs`と同じパターンで、実際の`AppPlugin`
//! (`DataLoaderPlugin`経由で実データ`assets/data/*.ron`、7か国・28州の本番マップ)・
//! `CountryPlugin`・`StatePlugin`・`BuildingPlugin`・`EconomyPlugin`・`ResearchPlugin`・
//! `PoliticsPlugin`・`DiplomacyPlugin`・`WarPlugin`・`MilitaryPlugin`を実際に組み合わせ、
//! その上に`SaveGamePlugin`/`LoadGamePlugin`、そして今回は実際の
//! `ui::country_selection::CountrySelectionPlugin`(+`TranslationCorePlugin`)を追加する。
//! `MapPlugin`/`UiPlugin`全体(Window/Camera/フォント資産に依存する)は登録せず、それらが
//! 提供する一時状態Resourceだけを最小限手動で用意する(002Eと同じ理由)。カメラが
//! 実際に変更されない/されることの検証は、Window非依存の`save::runtime`側の単体テスト
//! (`load_from_country_selection_success_does_not_request_camera_reset`等)で別途
//! 完了しているため、ここでは重複させない。
//!
//! 固定PNGを書き込む既存headless-renderテストはここでは実行しない
//! (`cargo test --no-run --test p21_save_002e_headless_render_test`等の既存方針を踏襲)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::{GameState, PlayingEntryMode};
use strategy_game::app::time::{GameDate, GamePaused};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{ArmyId, CountryId, DivisionId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{
    LastLoadOutcome, LoadExecutionCount, LoadOutcome, LoadRequestMessage, SaveFileConfig,
};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::ui::country_selection::{
    ContinueButton, ContinueStatusText, CountrySelectionPlugin, PreviewCountry, StartGameButton,
};
use strategy_game::war::WarPlugin;

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p21_save_003_e2e_{label}_{}_{nanos}_{n}",
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

/// 実際の本番プラグイン一式(`Map`/`Ui`全体を除く)へ`SaveGamePlugin`/`LoadGamePlugin`、
/// そして実際の`CountrySelectionPlugin`を追加した統合テスト用App。`Startup`を1フレーム
/// 実行した時点で、実データの読込・`GameState::CountrySelection`への遷移・
/// `CountrySelectionRoot`のUIツリーspawnまでが完了している(`DataLoaderPlugin`の
/// `transition_to_country_selection`がStartupシステムであり、そのNextStateが同じ
/// `app.update()`呼び出し内の`StateTransition`スケジュールで即座に適用されるため)。
fn setup_app_at_country_selection() -> App {
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
        .add_plugins(MilitaryPlugin);

    // MapPlugin/UiPluginが提供する一時状態(Window/Camera/フォント資産には依存しない
    // 純粋なResourceだけ)を手動で用意する(002Eと同じ理由)。
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

    // 実際のCountrySelection UI(`EconomyPlugin`が`TranslationCorePlugin`を既に追加済み
    // だが、`is_unique() == false`のため明示的にも追加してこのファイル単体で自己完結させる)。
    app.add_plugins(strategy_game::localization::TranslationCorePlugin);
    app.add_plugins(CountrySelectionPlugin);

    // Startup(DataLoaderPlugin::load_game_data、transition_to_country_selection)を実行。
    // 同じ呼び出し内でStateTransitionスケジュールも走るため、この1回でGameStateは
    // 既にCountrySelectionへ遷移済み、CountrySelectionRootのUIツリーもspawn済みになる。
    app.update();
    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::CountrySelection,
        "test setup must reach CountrySelection after the first Startup frame"
    );

    app
}

fn press_button<T: Component>(app: &mut App) {
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<T>>()
        .iter(app.world())
        .next()
        .unwrap_or_else(|| panic!("button entity for {} must exist", std::any::type_name::<T>()));
    app.world_mut()
        .entity_mut(entity)
        .insert(Interaction::Pressed);
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(SaveRequestMessage);
    state.apply(app.world_mut());
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(LoadRequestMessage);
    state.apply(app.world_mut());
}

// 1〜4. 起動しただけ(CountrySelectionへ入っただけ)ではロードせず、saves/も作らない。
#[test]
fn reaching_country_selection_does_not_read_or_create_a_save_file() {
    let temp_dir = TempTestDir::new("no_autoload");
    let mut app = setup_app_at_country_selection();
    app.insert_resource(temp_dir.save_file_config());

    app.update();
    app.update();
    app.update();

    assert_eq!(app.world().resource::<LoadExecutionCount>().0, 0);
    assert!(app.world().resource::<LastLoadOutcome>().0.is_none());
    assert!(
        !temp_dir.0.join("savegame_v1.ron").exists(),
        "reaching CountrySelection and idling for several frames must not create a save file"
    );
}

/// 続きからロード成功シナリオ用に、実7か国28州データで一度Playingへ入り、
/// 意図的にDivision集合を空にした上でセーブしたファイルを作る(実データの規模を保ちつつ、
/// 「Division 0件のセーブでもデバッグDivisionを追加しない」[項目21]を同じセーブで検証できる)。
/// 戻り値は(セーブ後のGameDate.year, セーブ後のtreasury, セーブ元のPlayerCountry)。
fn produce_zero_division_save(save_config: &SaveFileConfig) -> (i32, f64, CountryId) {
    let mut producer = App::new();
    producer
        .add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
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
        .add_plugins(MilitaryPlugin);

    producer
        .insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
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

    producer.add_plugins(SaveGamePlugin);
    producer.add_plugins(LoadGamePlugin);
    producer.insert_resource(save_config.clone());

    producer.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    producer.insert_resource(PlayerCountry(Some(CountryId(0))));
    producer.insert_state(GameState::Playing);
    producer.update(); // OnEnter(Playing): spawn_debug_divisions(PlayingEntryMode既定=NewGame)

    assert!(
        !producer
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .is_empty(),
        "spawn_debug_divisions must have created divisions for the producer session"
    );
    // 意図的に全Divisionを削除し、「0件のセーブ」を作る。
    producer
        .world_mut()
        .resource_mut::<MilitaryRegistry>()
        .divisions
        .clear();

    producer.world_mut().resource_mut::<GameDate>().year += 3;
    let treasury = 12345.0;
    producer
        .world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(CountryId(0))
        .unwrap()
        .treasury = treasury;

    send_save_request(&mut producer);
    producer.update();
    assert!(save_config.path.final_path.exists());

    let year = producer.world().resource::<GameDate>().year;
    let player_country = producer.world().resource::<PlayerCountry>().0.unwrap();
    (year, treasury, player_country)
}

// 12,13,14,15,16,17,18,19,20,21,25,26. 実7か国28州セーブをCountrySelectionから
// 「続きから」でロードでき、Playingへ正確に1回だけ遷移し、保存済みの値(日付・国庫・
// PlayerCountry)を復元し、GamePaused(true)、デバッグDivisionを追加せず(0件のまま)、
// LoadExecutionCountが1だけ増え、翌フレームに再実行されず、ロード後に再セーブでき、
// 新規Division/Army IDが衝突しないことを検証する。
#[test]
fn continue_button_loads_real_save_and_enters_playing_without_debug_divisions() {
    let temp_dir = TempTestDir::new("continue_success");
    let save_config = temp_dir.save_file_config();
    let (saved_year, saved_treasury, saved_player_country) =
        produce_zero_division_save(&save_config);

    let mut app = setup_app_at_country_selection();
    app.insert_resource(save_config);

    press_button::<ContinueButton>(&mut app);
    app.update(); // Update: handle_continue_button送信 -> PostUpdate: handle_load_requests実行

    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }
    assert_eq!(app.world().resource::<LoadExecutionCount>().0, 1);
    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::CountrySelection,
        "the Playing transition is only queued this frame"
    );

    app.update(); // 遷移フレーム: OnEnter(Playing)実行(spawn_debug_divisionsはLoadedGameでスキップされるはず)

    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::Playing,
        "a successful continue-load must transition to Playing"
    );
    assert_eq!(
        app.world().resource::<LoadExecutionCount>().0,
        1,
        "the transition frame itself must not re-execute the load"
    );
    assert_eq!(app.world().resource::<GameDate>().year, saved_year);
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(saved_player_country)
            .unwrap()
            .treasury,
        saved_treasury
    );
    assert_eq!(
        app.world().resource::<PlayerCountry>().0,
        Some(saved_player_country)
    );
    assert!(app.world().resource::<GamePaused>().0);
    assert!(
        app.world().resource::<MilitaryRegistry>().divisions.is_empty(),
        "a zero-division save must not gain debug divisions on a continue-load"
    );

    // ── ロード後に再セーブできる ─────────────────────────────────────────────
    send_save_request(&mut app);
    app.update();
    let resaved_contents =
        std::fs::read_to_string(app.world().resource::<SaveFileConfig>().path.final_path.clone())
            .unwrap();
    let resaved: strategy_game::save::SaveGameV1 = ron::from_str(&resaved_contents).unwrap();
    assert_eq!(resaved.date.year, saved_year);

    // ── ロード後の新規ID(Division/Army)が既存と衝突しない ─────────────────────
    let existing_division_ids: std::collections::HashSet<DivisionId> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .keys()
        .copied()
        .collect();
    let template = strategy_game::military::data::Division {
        id: DivisionId(0),
        owner: saved_player_country,
        division_type: strategy_game::military::data::DivisionType::Infantry,
        size: strategy_game::military::data::DivisionSize::Standard,
        current_state: strategy_game::common::StateId(0),
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 1000,
        max_manpower: 1000,
        equipment: 10.0,
        max_equipment: 10.0,
        organization: 50.0,
        max_organization: 50.0,
        morale: 1.0,
        max_morale: 1.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: strategy_game::military::data::DivisionStatus::Idle,
        def_id: strategy_game::common::DivisionDefinitionId(0),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    };
    let new_division_id = app
        .world_mut()
        .resource_mut::<MilitaryRegistry>()
        .add_division(template.clone());
    assert!(!existing_division_ids.contains(&new_division_id));

    let existing_army_ids: std::collections::HashSet<ArmyId> = app
        .world()
        .resource::<ArmyRegistry>()
        .armies
        .keys()
        .copied()
        .collect();
    let new_army_id =
        app.world_mut()
            .resource_scope(|world, mut army_registry: Mut<ArmyRegistry>| {
                let military_registry = world.resource::<MilitaryRegistry>();
                army_registry
                    .create_army(saved_player_country, &[new_division_id], military_registry)
                    .expect("freshly added division must be usable")
            });
    assert!(!existing_army_ids.contains(&new_army_id));

    // ── ロード後もゲーム中Loadが従来どおり動作する(回帰、項目27) ───────────────
    send_load_request(&mut app);
    app.update();
    assert_eq!(app.world().resource::<LoadExecutionCount>().0, 2);
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("in-Playing load after a continue-load must still succeed, got {other:?}"),
    }
}

// 22. New Gameでは従来どおりデバッグDivisionを正確に1回生成する(実UIボタン経由の回帰確認)。
#[test]
fn start_game_button_from_country_selection_still_spawns_debug_divisions() {
    let mut app = setup_app_at_country_selection();
    // CountryRegistryは実データ(7か国)で既に埋まっている。setup_uiがPreviewCountryを
    // 最初の国へ設定済み(この時点で既にOnEnter(CountrySelection)が実行済みのため)。
    assert!(app.world().resource::<PreviewCountry>().0.is_some());

    press_button::<StartGameButton>(&mut app);
    app.update();

    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::CountrySelection,
        "the transition is only queued this frame"
    );
    app.update(); // OnEnter(Playing)実行

    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::Playing
    );
    assert!(
        !app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .is_empty(),
        "starting a New Game via the real StartGameButton must still spawn debug divisions exactly as before P21-SAVE-003"
    );
    assert_eq!(
        *app.world().resource::<PlayingEntryMode>(),
        PlayingEntryMode::NewGame,
        "PlayingEntryMode must be reset back to NewGame after OnEnter(Playing) processing"
    );
}

// 28,34,35,36,37,38,39,40. FileNotFoundでCountrySelectionに留まり、World不変、
// 失敗通知(インライン状態表示)が出て、CameraReset無し、GamePaused不変、翌フレーム
// 再実行なし、失敗後に正しいファイルを置いて再試行すれば成功できる。
#[test]
fn continue_button_failure_stays_in_country_selection_and_recovers_on_retry() {
    let temp_dir = TempTestDir::new("continue_failure_then_retry");
    let save_config = temp_dir.save_file_config();
    let mut app = setup_app_at_country_selection();
    app.insert_resource(save_config.clone()); // ファイル未作成 -> FileNotFound

    let before_player_country = app.world().resource::<PlayerCountry>().0;

    press_button::<ContinueButton>(&mut app);
    app.update();

    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Failure { .. }) => {}
        other => panic!("expected a load failure (no save file yet), got {other:?}"),
    }
    assert_eq!(app.world().resource::<LoadExecutionCount>().0, 1);
    assert_eq!(app.world().resource::<PlayerCountry>().0, before_player_country);

    app.update();
    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::CountrySelection,
        "a failed continue-load must never transition to Playing"
    );
    assert_eq!(
        app.world().resource::<LoadExecutionCount>().0,
        1,
        "must not re-execute on the following frame without a new request"
    );

    // インライン失敗表示が空でないこと(GameNotification/NotificationHistoryはPlaying専用UIの
    // ため、CountrySelectionではContinueStatusTextが唯一の可視化経路)。
    let status_text = app
        .world_mut()
        .query_filtered::<&Text, With<ContinueStatusText>>()
        .single(app.world())
        .expect("status text entity must exist")
        .0
        .clone();
    assert!(
        !status_text.is_empty(),
        "a load failure must produce a non-empty inline status message on the CountrySelection screen"
    );

    // ── 失敗後、ファイルを正しく用意して再試行すれば成功する ────────────────────
    produce_zero_division_save(&save_config);

    press_button::<ContinueButton>(&mut app);
    app.update();

    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected the retry to succeed, got {other:?}"),
    }
    assert_eq!(app.world().resource::<LoadExecutionCount>().0, 2);
}
