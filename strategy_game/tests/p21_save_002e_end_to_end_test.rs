//! P21-SAVE-002E: セーブ→状態変更→ロードのエンドツーエンド統合テスト。
//!
//! `tests/daily_system_integration_test.rs`と同じパターンで、実際の`AppPlugin`
//! (`DataLoaderPlugin`経由で実データ`assets/data/*.ron`、7か国・28州の本番マップ)・
//! `CountryPlugin`・`StatePlugin`・`BuildingPlugin`・`EconomyPlugin`・`ResearchPlugin`・
//! `PoliticsPlugin`・`DiplomacyPlugin`・`WarPlugin`・`MilitaryPlugin`を実際に組み合わせ、
//! その上に`SaveGamePlugin`/`LoadGamePlugin`を追加する。`MapPlugin`/`UiPlugin`
//! (Window/Camera/フォント資産に依存する)は登録せず、それらが提供する一時状態
//! Resourceだけを最小限手動で用意する(既存のheadless-render統合テスト2本を除く、
//! という既存方針と同じ理由)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{GameDate, GamePaused, GameSpeed};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{ArmyId, CountryId, DivisionId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::{ArmyRegistry, SelectedArmy};
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
use strategy_game::state::data::StateRegistry;
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
        "strategy_game_p21_save_002e_e2e_{label}_{}_{nanos}_{n}",
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

/// 実際の本番プラグイン一式(`Map`/`Ui`を除く)へ`SaveGamePlugin`/`LoadGamePlugin`を
/// 追加した統合テスト用App。`Map`/`Ui`が提供する一時状態Resourceだけを最小限手動で
/// 用意する(値は全て既定値、ロード時にリセット対象になることの確認が目的であって、
/// 特定の初期値を検証したいわけではないため)。
fn setup_full_game_app() -> App {
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
    // 純粋なResourceだけ)を手動で用意する。
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

    app.update(); // Startup(DataLoaderPlugin)を実行し、実データを読み込む

    app.insert_resource(PlayerCountry(Some(CountryId(0))));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing): spawn_debug_divisions等

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

/// 起動しただけ(セーブ／ロードを一度も要求しない)では`saves/`を作らないことを確認する。
/// 実リポジトリの`saves/`ではなく、一意な一時ディレクトリを`SaveFileConfig`として
/// 明示的に注入しているため、このテスト自体は安全にリポジトリへ触れない。
#[test]
fn starting_the_game_does_not_create_a_save_file() {
    let temp_dir = TempTestDir::new("no_autosave");
    let mut app = setup_full_game_app();
    app.insert_resource(temp_dir.save_file_config());

    app.update();
    app.update();
    app.update();

    assert!(
        !temp_dir.0.join("savegame_v1.ron").exists(),
        "starting the game and running several frames without a Save/Load request must not create a save file"
    );
}

#[test]
fn save_change_load_restores_state_a_new_ids_do_not_collide_and_resave_works() {
    let temp_dir = TempTestDir::new("full_cycle");
    let mut app = setup_full_game_app();
    app.insert_resource(temp_dir.save_file_config());

    // ── 状態A: 現在の実データをセーブする ──────────────────────────────────
    let country0_capital = app
        .world()
        .resource::<CountryRegistry>()
        .get(CountryId(0))
        .unwrap()
        .capital_state_id;
    let state_a_treasury = app
        .world()
        .resource::<CountryRegistry>()
        .get(CountryId(0))
        .unwrap()
        .treasury;
    let state_a_year = app.world().resource::<GameDate>().year;
    let state_a_division_ids: Vec<DivisionId> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .keys()
        .copied()
        .collect();
    assert!(
        !state_a_division_ids.is_empty(),
        "spawn_debug_divisions must have created at least one division"
    );
    let observed_division_id = state_a_division_ids[0];
    let state_a_division_state = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&observed_division_id)
        .unwrap()
        .current_state;

    send_save_request(&mut app);
    app.update();
    assert!(temp_dir.0.join("savegame_v1.ron").exists());

    // ── 状態B: セーブ後に大きく変更する ────────────────────────────────────
    app.world_mut().resource_mut::<GameDate>().year += 5;
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(CountryId(0))
        .unwrap()
        .treasury = 999_999.0;
    app.world_mut().resource_mut::<GameSpeed>().0 = 4;
    app.world_mut().resource_mut::<GamePaused>().0 = false;
    // Divisionを他の州へ強制移動させる(状態Bの目印)。
    let displaced_state = StateId(country0_capital.0 + 1);
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .divisions
        .get_mut(&observed_division_id)
        .unwrap()
        .current_state = displaced_state;

    assert_ne!(
        app.world()
            .resource::<CountryRegistry>()
            .get(CountryId(0))
            .unwrap()
            .treasury,
        state_a_treasury
    );

    // ── ロードで状態Aへ戻す ────────────────────────────────────────────────
    send_load_request(&mut app);
    app.update();

    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }
    assert_eq!(app.world().resource::<LoadExecutionCount>().0, 1);

    assert_eq!(
        app.world().resource::<GameDate>().year,
        state_a_year,
        "date must revert to state A"
    );
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(CountryId(0))
            .unwrap()
            .treasury,
        state_a_treasury,
        "treasury must revert to state A"
    );
    assert!(
        app.world().resource::<GamePaused>().0,
        "load success must pause the game"
    );
    let restored_division = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&observed_division_id)
        .unwrap();
    assert_eq!(
        restored_division.current_state, state_a_division_state,
        "the division's location must revert to state A, not stay displaced at state B's location"
    );

    // ── 一時状態がロードによってリセットされていること(002Dの適用範囲) ──────
    assert_eq!(app.world().resource::<SelectedArmy>().0, None);
    assert_eq!(app.world().resource::<SelectedState>().0, None);

    // ── ロード後に新規IDが衝突しないこと ─────────────────────────────────
    let existing_division_ids: std::collections::HashSet<DivisionId> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .keys()
        .copied()
        .collect();
    let template = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&observed_division_id)
        .unwrap()
        .clone();
    let template_owner = template.owner;
    let new_division_id = app
        .world_mut()
        .resource_mut::<MilitaryRegistry>()
        .add_division(strategy_game::military::data::Division {
            id: DivisionId(0),
            ..template
        });
    assert!(
        !existing_division_ids.contains(&new_division_id),
        "a freshly issued DivisionId after reload must not collide with a restored one"
    );

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
                    .create_army(template_owner, &[new_division_id], military_registry)
                    .expect("freshly added division must be usable")
            });
    assert!(
        !existing_army_ids.contains(&new_army_id),
        "a freshly issued ArmyId after reload must not collide with a restored one"
    );

    // ── ロード後に再度セーブできること ──────────────────────────────────────
    send_save_request(&mut app);
    app.update();
    let resaved_contents = std::fs::read_to_string(temp_dir.0.join("savegame_v1.ron")).unwrap();
    let resaved: strategy_game::save::SaveGameV1 = ron::from_str(&resaved_contents).unwrap();
    assert_eq!(resaved.date.year, state_a_year);
}

/// ロードに失敗しても(セーブファイルなし)、現在プレイ中の実ゲーム状態を維持することを
/// 実際の本番プラグイン構成で確認する(runtime.rsの最小Appテストとは別に、実データ規模
/// でも同じ保証が成立することの追加確認)。
#[test]
fn failed_load_preserves_the_real_running_game_state() {
    let temp_dir = TempTestDir::new("failed_load_real_state");
    let mut app = setup_full_game_app();
    app.insert_resource(temp_dir.save_file_config()); // ファイルは作らない → FileNotFound

    let before_treasury = app
        .world()
        .resource::<CountryRegistry>()
        .get(CountryId(0))
        .unwrap()
        .treasury;
    let before_state_count = app.world().resource::<StateRegistry>().states.len();

    send_load_request(&mut app);
    app.update();

    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Failure { .. }) => {}
        other => panic!("expected a load failure, got {other:?}"),
    }
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(CountryId(0))
            .unwrap()
            .treasury,
        before_treasury
    );
    assert_eq!(
        app.world().resource::<StateRegistry>().states.len(),
        before_state_count
    );
}
