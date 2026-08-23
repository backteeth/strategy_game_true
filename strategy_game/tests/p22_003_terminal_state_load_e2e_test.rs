//! P22-003: 終局状態(P22-001プレイヤー勝利／P22-002観戦者向け世界統一)の
//! ロード再導出と、オーバーレイ表示順序の統合テスト。
//!
//! `PlayerEliminationMode`/`PlayerVictoryMode`/`WorldConclusionMode`はいずれも
//! `CountryRegistry`(セーブ済み)から毎フレーム再導出されるランタイムUI状態であり、
//! `SaveGameV1`自体には含まれない(P22-001/P22-002の既存設計)。この設計は正しい前提
//! だが、ロードで`CountryRegistry`だけが更新されても、この3つのResource自体は
//! 何も手を加えなければ前セッションの値(`Acknowledged`/`*Pending`、古い勝者国家ID・
//! 古い日付)を保持したままになってしまう。本テストはこの残留がないことを、
//! 実際の`save::write`/`save::read`/`save::validate`/`save::apply`経路(メッセージ駆動の
//! `SaveRequestMessage`→`handle_save_requests`、`LoadRequestMessage`→
//! `handle_load_requests`)を通して固定する。
//!
//! 実データ(`assets/data/countries.ron`、6か国)を使い、`CountryId(0)`をプレイヤーとして
//! 固定する。他国の消滅そのものを引き起こす講和・州移転の正しさ自体は
//! `tests/p21_019_crisis_war_peace_resolution_e2e_test.rs`等で既にE2E検証済みのため、
//! ここでは`P22-002`のE2Eテストと同じ設計判断により、消滅の結果
//! (`CountryStatus::Eliminated`)を`CountryRegistry`へ直接反映してからセーブする
//! (「消滅がどう起きたか」ではなく「消滅済みセーブのロード後の終局UI再導出順序」に
//! 焦点を絞るため)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::CountryId;
use strategy_game::country::{
    CountryPlugin, CountryRegistry, CountryStatus, PlayerCountry, PlayerEliminationMode,
    PlayerVictoryMode, WorldConclusionMode,
};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{
    LastLoadOutcome, LoadOutcome, LoadRequestMessage, SaveFileConfig,
};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::ui::player_elimination::{
    ContinueSpectatingButton, PlayerEliminationOverlayRoot, PlayerEliminationPlugin,
};
use strategy_game::ui::player_victory::{
    ContinuePlayingButton, PlayerVictoryOverlayRoot, PlayerVictoryPlugin,
};
use strategy_game::ui::world_conclusion::{
    ContinueSpectatingAfterConclusionButton, WorldConclusionOverlayRoot, WorldConclusionPlugin,
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
        "strategy_game_p22_003_e2e_{label}_{}_{nanos}_{n}",
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

/// 実7プラグイン(`p21_save_003_end_to_end_test.rs`と同一構成)+ Save/Load +
/// P22-001/P22-002の3終局UI Pluginを組み合わせた統合テスト用App。
/// `MapPlugin`/`UiPlugin`全体は登録せず、それらが提供する一時状態Resourceだけを
/// 最小限手動で用意する(既存p21_save_003/002eと同じ理由)。
fn setup_app_in_playing(player: CountryId, save_config: SaveFileConfig) -> App {
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
    app.add_plugins(PlayerEliminationPlugin);
    app.add_plugins(PlayerVictoryPlugin);
    app.add_plugins(WorldConclusionPlugin);

    app.insert_resource(save_config);
    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing)

    app
}

fn press_button<T: Component>(app: &mut App) {
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<T>>()
        .iter(app.world())
        .next()
        .unwrap_or_else(|| {
            panic!(
                "button entity for {} must exist",
                std::any::type_name::<T>()
            )
        });
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

/// ロード成功後、`Update`スケジュール(`DailySimulationSet::UiUpdate`)の検知Systemが
/// リセット後の状態を反映するには、`handle_load_requests`(`PostUpdate`)実行フレームの
/// 次のフレームが必要(`PostUpdate`は同じフレームの`Update`より後に実行されるため)。
fn load_and_settle(app: &mut App) {
    send_load_request(app);
    app.update(); // Update(古い状態のまま) -> PostUpdate: handle_load_requests(apply + reset)
    app.update(); // Update: リセット後のCountryRegistryを検知Systemが再判定
}

/// `country::elimination::apply_country_elimination`が実際に生成する結果を模す:
/// 消滅国家の状態を`Eliminated`へ遷移させるだけでなく、実際の消滅処理と同じく
/// 所有していた全陸州を併合国へ移転し、残存Divisionを解散する。`save::validate`は
/// 「Eliminated国家が州やDivisionを所有したまま」を不変条件違反として拒否するため
/// (本テスト作成時に実際に発見、`InvalidRange`/`ParticipantMismatch`)、この後処理を
/// 省略すると正式なセーブ経路(read/validate/apply)が最初から失敗し、P22-003が
/// 検証したい「ロード成功後の終局UI再導出」まで到達できない。
fn eliminate(app: &mut App, id: CountryId, annexed_by: CountryId, date: &str) {
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut() {
            if state.owner_country_id == id {
                state.owner_country_id = annexed_by;
                state.controller_country = Some(annexed_by);
            }
        }
    }
    {
        let mut military_registry = app.world_mut().resource_mut::<MilitaryRegistry>();
        military_registry.divisions.retain(|_, d| d.owner != id);
    }
    let mut registry = app.world_mut().resource_mut::<CountryRegistry>();
    registry.get_mut(id).unwrap().status = CountryStatus::Eliminated {
        eliminated_date: date.to_string(),
        annexed_by,
    };
}

fn defeat_display(app: &mut App) -> Display {
    let mut q = app
        .world_mut()
        .query::<(&PlayerEliminationOverlayRoot, &Node)>();
    q.single(app.world()).unwrap().1.display
}

fn victory_display(app: &mut App) -> Display {
    let mut q = app
        .world_mut()
        .query::<(&PlayerVictoryOverlayRoot, &Node)>();
    q.single(app.world()).unwrap().1.display
}

fn conclusion_display(app: &mut App) -> Display {
    let mut q = app
        .world_mut()
        .query::<(&WorldConclusionOverlayRoot, &Node)>();
    q.single(app.world()).unwrap().1.display
}

fn assert_load_succeeded(app: &App) {
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }
}

/// シナリオA: プレイヤー勝利済みセーブ。
#[test]
fn scenario_a_player_victory_save_loads_into_a_fresh_app_and_shows_only_the_victory_overlay() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("scenario_a");
    let save_config = temp_dir.save_file_config();

    // 1〜3. 2か国以上の世界からプレイヤー以外を全消滅させ、プレイヤーだけがActiveの
    // 状態を正式な保存経路(SaveRequestMessage -> handle_save_requests)で保存する。
    {
        let mut producer = setup_app_in_playing(player, save_config.clone());
        for rival in [1, 2, 3, 4, 5] {
            eliminate(&mut producer, CountryId(rival), player, "1850/06/15");
        }
        producer.update();
        assert!(matches!(
            *producer.world().resource::<PlayerVictoryMode>(),
            PlayerVictoryMode::VictoryOverlayPending { .. }
        ));
        send_save_request(&mut producer);
        producer.update();
        assert!(save_config.path.final_path.exists());
    }

    // 4. 新しいAppへ読み込む。
    let mut app = setup_app_in_playing(player, save_config.clone());
    // 5. ロード直後(まだロードしていない時点)は古いランタイムResourceに依存しない
    //    ――新しいAppは実データの6か国全てActiveから始まるため、まだ勝利は出ない。
    assert_eq!(victory_display(&mut app), Display::None);
    assert_eq!(
        *app.world().resource::<PlayerVictoryMode>(),
        PlayerVictoryMode::NotConcluded
    );

    load_and_settle(&mut app);
    assert_load_succeeded(&app);

    // 6. 判定System実行後、PlayerVictoryModeだけがPendingになる。
    assert!(matches!(
        *app.world().resource::<PlayerVictoryMode>(),
        PlayerVictoryMode::VictoryOverlayPending { .. }
    ));
    // 7. PlayerEliminationModeとWorldConclusionModeは発火しない。
    assert_eq!(
        *app.world().resource::<PlayerEliminationMode>(),
        PlayerEliminationMode::NotEliminated
    );
    assert_eq!(
        *app.world().resource::<WorldConclusionMode>(),
        WorldConclusionMode::NotConcluded
    );
    // 8. 勝利オーバーレイが表示される(プレイヤー国家名・日付は`PlayerVictoryMode`の
    //    フィールドとして保持され、UI表示ロジック自体はP22-001のユニットテストで
    //    別途検証済み)。
    assert_eq!(victory_display(&mut app), Display::Flex);
    assert_eq!(defeat_display(&mut app), Display::None);
    assert_eq!(conclusion_display(&mut app), Display::None);

    // 9. 閉じた後は再表示されない。
    press_button::<ContinuePlayingButton>(&mut app);
    app.update();
    assert_eq!(
        *app.world().resource::<PlayerVictoryMode>(),
        PlayerVictoryMode::Acknowledged
    );
    assert_eq!(victory_display(&mut app), Display::None);
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(victory_display(&mut app), Display::None);
}

/// シナリオB: プレイヤー敗北＋他国世界統一済みセーブ。
#[test]
fn scenario_b_defeat_and_world_conclusion_save_shows_defeat_before_world_conclusion() {
    let player = CountryId(0);
    let winner = CountryId(1);
    let temp_dir = TempTestDir::new("scenario_b");
    let save_config = temp_dir.save_file_config();

    // 1〜3. プレイヤー国家が消滅済み、プレイヤー以外のActive国家が正確に1か国、
    // という状態を正式な保存経路で保存する。
    {
        let mut producer = setup_app_in_playing(player, save_config.clone());
        *producer
            .world_mut()
            .resource_mut::<strategy_game::app::time::GameDate>() =
            strategy_game::app::time::GameDate::new(1850, 6, 15);
        eliminate(&mut producer, player, winner, "1850/06/15");
        for rival in [2, 3, 4, 5] {
            eliminate(&mut producer, CountryId(rival), winner, "1850/06/10");
        }
        producer.update();
        assert_eq!(
            *producer.world().resource::<PlayerEliminationMode>(),
            PlayerEliminationMode::DefeatOverlayPending
        );
        send_save_request(&mut producer);
        producer.update();
        assert!(save_config.path.final_path.exists());
    }

    // 4. ロードする(新規App)。
    let mut app = setup_app_in_playing(player, save_config.clone());
    load_and_settle(&mut app);
    assert_load_succeeded(&app);

    // 5. 判定後、敗北状態と世界統一状態が正しく再導出される。
    assert_eq!(
        *app.world().resource::<PlayerEliminationMode>(),
        PlayerEliminationMode::DefeatOverlayPending
    );
    assert!(matches!(
        *app.world().resource::<WorldConclusionMode>(),
        WorldConclusionMode::OverlayPending { .. }
    ));
    // 11. PlayerVictoryModeは発火しない(プレイヤーは消滅済みのため)。
    assert_eq!(
        *app.world().resource::<PlayerVictoryMode>(),
        PlayerVictoryMode::NotConcluded
    );

    // 6. 最初に敗北オーバーレイだけが表示される。
    assert_eq!(defeat_display(&mut app), Display::Flex);
    assert_eq!(
        conclusion_display(&mut app),
        Display::None,
        "world-conclusion must stay hidden until the defeat overlay is dismissed, even though \
         its domain state is already pending"
    );
    assert_eq!(victory_display(&mut app), Display::None);

    // 7. 敗北通知を閉じる。
    press_button::<ContinueSpectatingButton>(&mut app);
    app.update();
    assert_eq!(
        *app.world().resource::<PlayerEliminationMode>(),
        PlayerEliminationMode::Spectating
    );

    // 8. 次に世界統一オーバーレイが表示される。
    assert_eq!(defeat_display(&mut app), Display::None);
    assert_eq!(conclusion_display(&mut app), Display::Flex);

    // 9. 勝者国家名・成立日が正しい(Resourceレベルで確認)。
    assert_eq!(
        *app.world().resource::<WorldConclusionMode>(),
        WorldConclusionMode::OverlayPending {
            winner_country_id: winner,
            conclusion_date: "1850/06/15".to_string(),
        }
    );

    // 10. 世界統一通知を閉じた後は再表示されない。
    press_button::<ContinueSpectatingAfterConclusionButton>(&mut app);
    app.update();
    assert_eq!(
        *app.world().resource::<WorldConclusionMode>(),
        WorldConclusionMode::Acknowledged
    );
    assert_eq!(conclusion_display(&mut app), Display::None);
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(conclusion_display(&mut app), Display::None);
    assert_eq!(defeat_display(&mut app), Display::None);
    assert_eq!(victory_display(&mut app), Display::None);
}

/// シナリオC: 通常プレイ中セーブ。以前のセッションが残した終局Resourceの汚染が
/// ロードによって初期化されることを確認する。
#[test]
fn scenario_c_normal_save_resets_stale_terminal_state_from_a_previous_session() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("scenario_c");
    let save_config = temp_dir.save_file_config();

    // 1. プレイヤーを含む複数国家がActiveな通常プレイセーブを作る。
    {
        let mut producer = setup_app_in_playing(player, save_config.clone());
        send_save_request(&mut producer);
        producer.update();
        assert!(save_config.path.final_path.exists());
    }

    let mut app = setup_app_in_playing(player, save_config.clone());

    // 2. App上の3つの終局Resourceを、以前のセッションが残った状態を模して
    // Pending/Acknowledgedへ変更する。
    *app.world_mut().resource_mut::<PlayerEliminationMode>() =
        PlayerEliminationMode::DefeatOverlayPending;
    *app.world_mut().resource_mut::<PlayerVictoryMode>() = PlayerVictoryMode::Acknowledged;
    *app.world_mut().resource_mut::<WorldConclusionMode>() = WorldConclusionMode::OverlayPending {
        winner_country_id: CountryId(3),
        conclusion_date: "1799/01/01".to_string(),
    };

    // 3. 通常セーブをロードする。
    load_and_settle(&mut app);
    assert_load_succeeded(&app);

    // 4. 成功したロードによって終局Resourceが初期化される。
    assert_eq!(
        *app.world().resource::<PlayerEliminationMode>(),
        PlayerEliminationMode::NotEliminated
    );
    assert_eq!(
        *app.world().resource::<PlayerVictoryMode>(),
        PlayerVictoryMode::NotConcluded
    );
    assert_eq!(
        *app.world().resource::<WorldConclusionMode>(),
        WorldConclusionMode::NotConcluded
    );

    // 5. 判定Systemを実行しても、いずれのオーバーレイも表示されない。
    assert_eq!(defeat_display(&mut app), Display::None);
    assert_eq!(victory_display(&mut app), Display::None);
    assert_eq!(conclusion_display(&mut app), Display::None);
}

/// シナリオD: 同一App内での状態残留の組み合わせ確認。
mod scenario_d_same_app_state_does_not_leak_across_loads {
    use super::*;

    /// 勝利Acknowledged -> 敗北済みセーブをロード。
    #[test]
    fn victory_acknowledged_then_loading_a_defeat_and_conclusion_save() {
        let player = CountryId(0);
        let winner = CountryId(1);
        let temp_dir = TempTestDir::new("scenario_d1");
        let save_config = temp_dir.save_file_config();

        {
            let mut producer = setup_app_in_playing(player, save_config.clone());
            eliminate(&mut producer, player, winner, "1850/06/15");
            for rival in [2, 3, 4, 5] {
                eliminate(&mut producer, CountryId(rival), winner, "1850/06/10");
            }
            send_save_request(&mut producer);
            producer.update();
        }

        let mut app = setup_app_in_playing(player, save_config.clone());
        // 前セッションで既にプレイヤー勝利がAcknowledgedだった、という状態を模す。
        *app.world_mut().resource_mut::<PlayerVictoryMode>() = PlayerVictoryMode::Acknowledged;

        load_and_settle(&mut app);
        assert_load_succeeded(&app);

        assert_eq!(
            *app.world().resource::<PlayerVictoryMode>(),
            PlayerVictoryMode::NotConcluded,
            "a stale Acknowledged victory must not survive a load into a defeat/conclusion save"
        );
        assert_eq!(
            *app.world().resource::<PlayerEliminationMode>(),
            PlayerEliminationMode::DefeatOverlayPending
        );
        assert!(matches!(
            *app.world().resource::<WorldConclusionMode>(),
            WorldConclusionMode::OverlayPending { .. }
        ));
        assert_eq!(defeat_display(&mut app), Display::Flex);
        assert_eq!(victory_display(&mut app), Display::None);
    }

    /// 世界統一Acknowledged -> プレイヤー勝利済みセーブをロード。
    #[test]
    fn conclusion_acknowledged_then_loading_a_player_victory_save() {
        let player = CountryId(0);
        let temp_dir = TempTestDir::new("scenario_d2");
        let save_config = temp_dir.save_file_config();

        {
            let mut producer = setup_app_in_playing(player, save_config.clone());
            for rival in [1, 2, 3, 4, 5] {
                eliminate(&mut producer, CountryId(rival), player, "1850/06/15");
            }
            send_save_request(&mut producer);
            producer.update();
        }

        let mut app = setup_app_in_playing(player, save_config.clone());
        // 前セッションで観戦中の世界統一が既にAcknowledgedだった、という状態を模す
        // (古い勝者国家ID・古い日付を保持していたことを想定)。
        *app.world_mut().resource_mut::<WorldConclusionMode>() = WorldConclusionMode::Acknowledged;
        *app.world_mut().resource_mut::<PlayerEliminationMode>() =
            PlayerEliminationMode::Spectating;

        load_and_settle(&mut app);
        assert_load_succeeded(&app);

        assert_eq!(
            *app.world().resource::<WorldConclusionMode>(),
            WorldConclusionMode::NotConcluded,
            "a stale Acknowledged world-conclusion must not survive a load into a player-victory \
             save"
        );
        assert_eq!(
            *app.world().resource::<PlayerEliminationMode>(),
            PlayerEliminationMode::NotEliminated,
            "a stale Spectating elimination mode must not survive a load where the player is \
             Active again"
        );
        assert!(matches!(
            *app.world().resource::<PlayerVictoryMode>(),
            PlayerVictoryMode::VictoryOverlayPending { .. }
        ));
        assert_eq!(victory_display(&mut app), Display::Flex);
        assert_eq!(conclusion_display(&mut app), Display::None);
    }

    /// Pending状態 -> 通常セーブをロード。
    #[test]
    fn pending_state_then_loading_a_normal_save() {
        let player = CountryId(0);
        let temp_dir = TempTestDir::new("scenario_d3");
        let save_config = temp_dir.save_file_config();

        {
            let mut producer = setup_app_in_playing(player, save_config.clone());
            send_save_request(&mut producer);
            producer.update();
        }

        let mut app = setup_app_in_playing(player, save_config.clone());
        *app.world_mut().resource_mut::<PlayerVictoryMode>() =
            PlayerVictoryMode::VictoryOverlayPending {
                victory_date: "1801/01/01".to_string(),
            };

        load_and_settle(&mut app);
        assert_load_succeeded(&app);

        assert_eq!(
            *app.world().resource::<PlayerVictoryMode>(),
            PlayerVictoryMode::NotConcluded,
            "a stale Pending victory must not survive a load into a normal multi-country save"
        );
        assert_eq!(victory_display(&mut app), Display::None);
        assert_eq!(defeat_display(&mut app), Display::None);
        assert_eq!(conclusion_display(&mut app), Display::None);
    }
}

/// シナリオE: ロード失敗。
#[test]
fn scenario_e_a_failed_load_leaves_the_current_game_and_terminal_state_untouched() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("scenario_e");
    // ファイルを一切作らない -> FileNotFoundでロード失敗する。
    let save_config = temp_dir.save_file_config();

    let mut app = setup_app_in_playing(player, save_config);

    // 1. 現在のゲームと終局Resourceを任意の有効状態にする。
    eliminate(&mut app, CountryId(1), player, "1850/06/15");
    app.update();
    let before_country_len = app.world().resource::<CountryRegistry>().countries.len();
    let before_country1_status = app
        .world()
        .resource::<CountryRegistry>()
        .get(CountryId(1))
        .unwrap()
        .status
        .clone();
    *app.world_mut().resource_mut::<PlayerVictoryMode>() =
        PlayerVictoryMode::VictoryOverlayPending {
            victory_date: "1801/01/01".to_string(),
        };
    let before_victory_mode = app.world().resource::<PlayerVictoryMode>().clone();
    let before_elimination_mode = *app.world().resource::<PlayerEliminationMode>();
    let before_conclusion_mode = app.world().resource::<WorldConclusionMode>().clone();

    // 2〜3. 存在しないセーブファイルをロードしようとして失敗する。
    send_load_request(&mut app);
    app.update();
    app.update();

    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Failure { .. }) => {}
        other => panic!("expected a load failure, got {other:?}"),
    }

    // 4. CountryRegistryを含む現在のゲーム状態が変わらない。
    assert_eq!(
        app.world().resource::<CountryRegistry>().countries.len(),
        before_country_len
    );
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(CountryId(1))
            .unwrap()
            .status,
        before_country1_status
    );

    // 5. 3つの終局Resourceも変わらない。
    assert_eq!(
        *app.world().resource::<PlayerVictoryMode>(),
        before_victory_mode
    );
    assert_eq!(
        *app.world().resource::<PlayerEliminationMode>(),
        before_elimination_mode
    );
    assert_eq!(
        *app.world().resource::<WorldConclusionMode>(),
        before_conclusion_mode
    );

    // 6. 既存のロード失敗通知が正常に表示される(LastLoadOutcomeがFailureであることは
    // 上で確認済み。実際の通知メッセージ生成・表示ロジック自体は既存の
    // save::runtime::tests::load_failures_never_panic_across_multiple_kinds等で
    // 別途検証済みのため、ここでは通知が実際に1件生成されたことだけを確認する)。
    let mut reader_state: SystemState<
        MessageReader<strategy_game::ui::notification::GameNotification>,
    > = SystemState::new(app.world_mut());
    let notif_count = reader_state
        .get_mut(app.world_mut())
        .expect("MessageReader<GameNotification> system param must be valid")
        .read()
        .count();
    assert!(
        notif_count >= 1,
        "a load failure must still emit a notification"
    );
}
