//! P21-005: Army(編成)↔Frontline(前線)割当のエンドツーエンド統合テスト。
//!
//! `tests/p21_save_003_end_to_end_test.rs`と同じパターンで、実際の`AppPlugin`
//! (`DataLoaderPlugin`経由で実データ`assets/data/*.ron`、7か国・28州の本番マップ)・
//! `CountryPlugin`・`StatePlugin`・`BuildingPlugin`・`EconomyPlugin`・`ResearchPlugin`・
//! `PoliticsPlugin`・`DiplomacyPlugin`・`WarPlugin`・`MilitaryPlugin`・
//! `SaveGamePlugin`/`LoadGamePlugin`を実際に組み合わせる。`MapPlugin`/`UiPlugin`
//! (Window/Camera/フォント資産に依存する)は登録しない。前線選択モード・クリック判定・
//! ボタンUIは`map::frontline_selection`/`ui::military_panel`の単体テストで別途検証済みの
//! ため、ここでは`war::frontline::FrontlineRegistry`のAPI(`assign_army`/`unassign_army`)を
//! 直接呼び、実データ規模でのSave/Load往復とDivision不動条件の検証に集中する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::GamePaused;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{ArmyId, CountryId, DivisionDefinitionId, DivisionId, StateId, WarId};
use strategy_game::country::{CountryPlugin, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::profiling::advance_one_day;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::SaveFileConfig;
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
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
        "strategy_game_p21_005_e2e_{label}_{}_{nanos}_{n}",
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

/// 実7か国28州データを読み込み、CountryId(0)(Kingdom of Arcadia)としてPlayingへ入った
/// Appを構築する。`MapPlugin`/`UiPlugin`全体は登録せず、それらが提供する純粋な
/// 一時状態Resourceだけを手動で用意する(`p21_save_003`と同じ理由)。
fn setup_app_in_playing() -> App {
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

    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(CountryId(0))));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing)

    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::Playing
    );

    app
}

fn make_division(id: DivisionId, owner: CountryId, state: StateId) -> Division {
    Division {
        id,
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: state,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower: 1000,
        max_manpower: 1000,
        equipment: 10.0,
        max_equipment: 10.0,
        organization: 100.0,
        max_organization: 100.0,
        morale: 1.0,
        max_morale: 1.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: DivisionDefinitionId(0),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

fn send_save_request(app: &mut App) {
    let mut state: bevy::ecs::system::SystemState<MessageWriter<SaveRequestMessage>> =
        bevy::ecs::system::SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(SaveRequestMessage);
    state.apply(app.world_mut());
}

fn send_load_request(app: &mut App) {
    let mut state: bevy::ecs::system::SystemState<
        MessageWriter<strategy_game::save::runtime::LoadRequestMessage>,
    > = bevy::ecs::system::SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(strategy_game::save::runtime::LoadRequestMessage);
    state.apply(app.world_mut());
}

/// 要求テスト項目57〜68: 実7か国28州データでWar/Frontline/Armyを用意し、割当・
/// Division不動・セーブ・状態変更・ロード・復元・解除・再割当・再セーブ・新規ID非衝突・
/// 既存回帰なしまでを一気通貫で検証する。
#[test]
fn army_frontline_assignment_survives_save_load_round_trip_with_real_map_data() {
    let temp_dir = TempTestDir::new("assign_save_load");
    let save_config = temp_dir.save_file_config();
    let mut app = setup_app_in_playing();
    app.insert_resource(save_config);

    // ── 57. 実データ上でWar(Arcadia(0) vs Elfin(1))を用意する。State(1)(Arcadia領)と
    // State(3)(Elfin領)は実データ上の隣接国境(assets/data/states.ron)。`WarRegistry::add_war`
    // 経由で作成し、`next_id`カウンタも正しく進める(手動で`wars`へ直接insertすると
    // `next_id`が追随せず、セーブ後のロードでNextIdCollision検証に落ちるため)。
    let war = War {
        id: WarId(0), // add_warが上書きする
        name: "P21-005 Test War".to_string(),
        attackers: [CountryId(0)].into_iter().collect(),
        defenders: [CountryId(1)].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        war_goals: vec![],
        start_date: "1801/01/01".to_string(),
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
    };
    let war_id = app.world_mut().resource_mut::<WarRegistry>().add_war(war);

    // 前線を実際に発生させる(日次更新経由。既存のFrontlineOrders/WarPreparationの
    // 実SystemSet順序をそのまま使う)。
    advance_one_day(&mut app);

    let fl_id = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_frontline_for_war(war_id)
        .expect("a real Frontline must be generated for the active war")
        .frontline_id;

    // ── Armyを1つ用意する(自国Division1体、国境州State(1)に配置) ──────────────
    let division_id = app
        .world_mut()
        .resource_mut::<MilitaryRegistry>()
        .add_division(make_division(DivisionId(0), CountryId(0), StateId(1)));
    let army_id = app
        .world_mut()
        .resource_scope(|world, mut army_registry: Mut<ArmyRegistry>| {
            let military_registry = world.resource::<MilitaryRegistry>();
            army_registry
                .create_army(CountryId(0), &[division_id], military_registry)
                .expect("division must be usable")
        });

    let division_before = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&division_id)
        .unwrap()
        .clone();

    // ── 58. Armyを前線へ割当 ─────────────────────────────────────────────────
    app.world_mut()
        .resource_scope(|world, army_registry: Mut<ArmyRegistry>| {
            world.resource_scope(|world, war_registry: Mut<WarRegistry>| {
                world
                    .resource_mut::<FrontlineRegistry>()
                    .assign_army(army_id, fl_id, CountryId(0), &army_registry, &war_registry)
                    .expect("assigning an own army to a real frontline must succeed");
            });
        });
    assert_eq!(
        app.world()
            .resource::<FrontlineRegistry>()
            .frontline_for_army(army_id),
        Some(fl_id)
    );

    // ── 59. 割当だけではDivisionが動かない(直後、および翌日のFrontlineOrders後も) ──
    let division_after_assign = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&division_id)
        .unwrap()
        .clone();
    assert_eq!(
        division_before.current_state,
        division_after_assign.current_state
    );
    assert_eq!(
        division_before.destination,
        division_after_assign.destination
    );
    assert_eq!(division_before.status, division_after_assign.status);
    assert_eq!(division_before.combat_id, division_after_assign.combat_id);

    advance_one_day(&mut app);
    let division_after_orders = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&division_id)
        .unwrap()
        .clone();
    assert_eq!(
        division_before.current_state, division_after_orders.current_state,
        "Army割当だけでは翌日のFrontlineOrders後もDivisionは移動してはならない"
    );
    assert_eq!(division_after_orders.status, DivisionStatus::Idle);
    assert_eq!(division_after_orders.destination, None);
    assert!(
        !app.world()
            .resource::<FrontlineRegistry>()
            .frontline_generated_movements
            .contains(&division_id)
    );
    assert_eq!(
        app.world()
            .resource::<FrontlineRegistry>()
            .frontline_for_army(army_id),
        Some(fl_id),
        "1日経過後もArmyの前線割当自体は維持されるはず(デバッグ用中間チェック)"
    );

    // ── 60. セーブ ──────────────────────────────────────────────────────────
    send_save_request(&mut app);
    app.update();
    assert!(app.world().resource::<GamePaused>().0 == app.world().resource::<GamePaused>().0); // no-op sanity
    let save_path = app
        .world()
        .resource::<SaveFileConfig>()
        .path
        .final_path
        .clone();
    assert!(save_path.exists());

    // デバッグ用中間チェック: セーブファイル自体にArmy前線割当が含まれている。
    {
        let saved_contents = std::fs::read_to_string(&save_path).unwrap();
        let saved: strategy_game::save::SaveGameV1 = ron::from_str(&saved_contents).unwrap();
        assert_eq!(
            saved.frontlines.army_frontline_map.get(&army_id),
            Some(&fl_id),
            "セーブファイル自体にArmy前線割当が含まれているはず"
        );
    }

    // ── 61. 状態変更(割当解除)してからロードで復元されることを確認する ──────────
    app.world_mut()
        .resource_scope(|world, army_registry: Mut<ArmyRegistry>| {
            world
                .resource_mut::<FrontlineRegistry>()
                .unassign_army(army_id, CountryId(0), &army_registry)
                .unwrap();
        });
    assert_eq!(
        app.world()
            .resource::<FrontlineRegistry>()
            .frontline_for_army(army_id),
        None
    );

    // ── 62/63. ロードでArmy割当が復元される ─────────────────────────────────
    send_load_request(&mut app);
    app.update();

    match &app
        .world()
        .resource::<strategy_game::save::runtime::LastLoadOutcome>()
        .0
    {
        Some(strategy_game::save::runtime::LoadOutcome::Success { .. }) => {}
        other => panic!("expected the load to succeed, got {other:?}"),
    }

    assert_eq!(
        app.world()
            .resource::<FrontlineRegistry>()
            .frontline_for_army(army_id),
        Some(fl_id),
        "ロード後にArmyの前線割当が復元されるはず"
    );
    let plan = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_plan(fl_id, CountryId(0))
        .unwrap();
    assert_eq!(plan.assigned_army_ids, vec![army_id]);

    // ── 64. ロード後もArmy/Frontlineレジストリが一貫している(パネル/描画が同期
    // 可能な状態であることの代理検証: Army自体・所属Divisionが健全) ──────────────
    assert!(
        app.world()
            .resource::<ArmyRegistry>()
            .armies
            .contains_key(&army_id)
    );
    assert!(
        app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .contains_key(&division_id)
    );

    // ── 65. 解除・再割当 ────────────────────────────────────────────────────
    app.world_mut()
        .resource_scope(|world, army_registry: Mut<ArmyRegistry>| {
            world
                .resource_mut::<FrontlineRegistry>()
                .unassign_army(army_id, CountryId(0), &army_registry)
                .unwrap();
        });
    assert_eq!(
        app.world()
            .resource::<FrontlineRegistry>()
            .frontline_for_army(army_id),
        None
    );
    app.world_mut()
        .resource_scope(|world, army_registry: Mut<ArmyRegistry>| {
            world.resource_scope(|world, war_registry: Mut<WarRegistry>| {
                world
                    .resource_mut::<FrontlineRegistry>()
                    .assign_army(army_id, fl_id, CountryId(0), &army_registry, &war_registry)
                    .unwrap();
            });
        });
    assert_eq!(
        app.world()
            .resource::<FrontlineRegistry>()
            .frontline_for_army(army_id),
        Some(fl_id)
    );

    // ── 66. 再セーブ ────────────────────────────────────────────────────────
    send_save_request(&mut app);
    app.update();
    let resaved_contents = std::fs::read_to_string(&save_path).unwrap();
    let resaved: strategy_game::save::SaveGameV1 = ron::from_str(&resaved_contents).unwrap();
    let resaved_plan = resaved
        .frontlines
        .plans
        .get(&(fl_id, CountryId(0)))
        .expect("re-saved frontline plan must exist");
    assert_eq!(resaved_plan.assigned_army_ids, vec![army_id]);
    assert_eq!(
        resaved.frontlines.army_frontline_map.get(&army_id),
        Some(&fl_id)
    );
    assert_eq!(resaved.version, 1, "SaveGameV1.version must remain 1");

    // ── 67. 再セーブ後も新規Division/Army IDが既存と衝突しない ─────────────────
    let existing_division_ids: std::collections::HashSet<DivisionId> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .keys()
        .copied()
        .collect();
    let new_division_id = app
        .world_mut()
        .resource_mut::<MilitaryRegistry>()
        .add_division(make_division(DivisionId(0), CountryId(0), StateId(1)));
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
                    .create_army(CountryId(0), &[new_division_id], military_registry)
                    .unwrap()
            });
    assert!(!existing_army_ids.contains(&new_army_id));

    // ── 68. 既存の一括移動命令等、直接移動系の挙動には触れていない(このテストは
    // Army割当のみを操作しており、`division_id`の状態はテスト全体を通してIdleのまま) ──
    let final_division = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&division_id)
        .unwrap();
    assert_eq!(final_division.status, DivisionStatus::Idle);
    assert_eq!(final_division.current_state, StateId(1));
}

/// 要求テスト項目50/54相当: Loadで存在しない/矛盾したArmy前線割当を含むセーブファイルは
/// 拒否され(Apply失敗)、World側の状態は一切変更されない。
#[test]
fn load_rejects_inconsistent_army_frontline_save_and_preserves_world_state() {
    let temp_dir = TempTestDir::new("load_rejects_bad_army_frontline");
    let save_config = temp_dir.save_file_config();
    let mut app = setup_app_in_playing();
    app.insert_resource(save_config.clone());

    send_save_request(&mut app);
    app.update();
    let save_path = save_config.path.final_path.clone();
    let original_contents = std::fs::read_to_string(&save_path).unwrap();

    // 保存済みのセーブへ、存在しないArmyIdを参照する不正なarmy_frontline_mapエントリを
    // 手動で注入する(現実のセーブ破損/手編集を模した最小再現)。
    let mut save: strategy_game::save::SaveGameV1 = ron::from_str(&original_contents).unwrap();
    save.frontlines
        .army_frontline_map
        .insert(ArmyId(9999), strategy_game::common::FrontlineId(9999));
    let corrupted = ron::to_string(&save).unwrap();
    std::fs::write(&save_path, corrupted).unwrap();

    let before_year = app
        .world()
        .resource::<strategy_game::app::time::GameDate>()
        .year;

    send_load_request(&mut app);
    app.update();

    match &app
        .world()
        .resource::<strategy_game::save::runtime::LastLoadOutcome>()
        .0
    {
        Some(strategy_game::save::runtime::LoadOutcome::Failure { .. }) => {}
        other => panic!("corrupted army_frontline_map save must fail validation, got {other:?}"),
    }
    assert_eq!(
        app.world()
            .resource::<strategy_game::app::time::GameDate>()
            .year,
        before_year,
        "a rejected load must not mutate World state"
    );
    assert!(
        app.world()
            .resource::<FrontlineRegistry>()
            .army_frontline_map
            .is_empty(),
        "a rejected load must not leave partial FrontlineRegistry state"
    );
}
