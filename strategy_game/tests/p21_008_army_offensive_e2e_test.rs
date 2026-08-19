//! P21-008: 攻勢線(FrontlinePlan.offensive_line_region_ids)を実際の攻撃処理へ接続した
//! ことのエンドツーエンド統合テスト。
//!
//! `tests/p21_005_army_frontline_e2e_test.rs`と同じパターンで、実際の`AppPlugin`
//! (`DataLoaderPlugin`経由で実データ`assets/data/*.ron`、7か国・28州の本番マップ)・
//! `CountryPlugin`・`StatePlugin`・`BuildingPlugin`・`EconomyPlugin`・`ResearchPlugin`・
//! `PoliticsPlugin`・`DiplomacyPlugin`・`WarPlugin`・`MilitaryPlugin`・
//! `SaveGamePlugin`/`LoadGamePlugin`を実際に組み合わせる。`MapPlugin`/`UiPlugin`は
//! 登録しない(Window/Camera/フォント資産に依存するため、UIそのものは
//! `ui::military_panel`の単体テストで別途検証済み)。
//!
//! `profiling::advance_one_day`は`DailySimulationSet`の全SystemSet
//! (`FrontlineOrders`→`MilitaryAction`を含む)を実際の本番順序のまま1日分実行するため、
//! 攻勢線に基づく移動命令の発行(`FrontlineOrders`)と、その移動の実際の前進・
//! 敵領到着時の交戦判定(`MilitaryAction`、`military::movement::process_movement`/
//! `military::invasion::process_division_arrival`)が、同一の日次サイクルの中で
//! 本当につながっていることを検証できる。
//!
//! `states.ron`は変更しない。State(1)(Arcadia領、実データ)とState(3)(Elfin領、実データ)は
//! 直接隣接している(`assets/data/states.ron`で確認済み、P21-005のテストと同じ国境)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionDefinitionId, DivisionId, StateId, WarId};
use strategy_game::country::{CountryPlugin, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::military::invasion::process_division_arrival;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::profiling::advance_one_day;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::{FrontlineRegistry, FrontlineStance};

/// `p21_005_army_frontline_e2e_test.rs`と同じ最小Appセットアップ
/// (`MapPlugin`/`UiPlugin`本体は登録せず、それらが提供する純粋な一時状態Resourceだけを
/// 手動で用意する)。
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

/// 要求テスト項目24: 実7か国28州データ上で、Army経由Divisionが攻勢線に沿って
/// 実際に前進し、敵支配地域(State(3)、Elfin領)へ向けて日次シミュレーションが
/// 移動・交戦まで一気通貫でつながることを確認する。
#[test]
fn army_offensive_line_advances_and_engages_with_real_map_data() {
    let mut app = setup_app_in_playing();

    // Arcadia(0) vs Elfin(1)。State(1)(Arcadia領)とState(3)(Elfin領)は実データ上、
    // 直接隣接する国境(P21-005のテストと同じ組み合わせ)。
    let war = War {
        id: WarId(0), // add_warが上書きする
        name: "P21-008 Test War".to_string(),
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

    // War宣戦当日ガードを避けるため、まず1日進めて前線を発生させる。
    advance_one_day(&mut app);
    let fl_id = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_frontline_for_war(war_id)
        .expect("a real Frontline must be generated for the active war")
        .frontline_id;

    // Armyを1つ用意する(Arcadia所属Division1体、国境州State(1)に配置)。
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

    // Armyを前線へ割当。
    app.world_mut()
        .resource_scope(|world, army_registry: Mut<ArmyRegistry>| {
            world.resource_scope(|world, war_registry: Mut<WarRegistry>| {
                world
                    .resource_mut::<FrontlineRegistry>()
                    .assign_army(army_id, fl_id, CountryId(0), &army_registry, &war_registry)
                    .expect("assigning an own army to a real frontline must succeed");
            });
        });

    // 攻勢線をState(3)(Elfin領、隣接済み)へ設定する。
    app.world_mut()
        .resource_scope(|world, state_registry: Mut<StateRegistry>| {
            world.resource_scope(|world, war_registry: Mut<WarRegistry>| {
                world
                    .resource_mut::<FrontlineRegistry>()
                    .set_offensive_line(
                        fl_id,
                        CountryId(0),
                        &[StateId(3)],
                        &state_registry,
                        &war_registry,
                    )
                    .expect("State(3) is enemy-controlled and reachable, must be accepted");
            });
        });
    if let Some(plan) = app
        .world_mut()
        .resource_mut::<FrontlineRegistry>()
        .get_plan_mut(fl_id, CountryId(0))
    {
        plan.stance = FrontlineStance::Offensive;
    }

    // ── 攻勢線設定・Offensive姿勢設定だけではまだ動かない(次回の通常処理から反映) ──
    let division_before = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&division_id)
        .unwrap()
        .clone();
    assert_eq!(division_before.status, DivisionStatus::Idle);
    assert_eq!(division_before.destination, None);

    // ── 1日進める: FrontlineOrdersで攻撃移動が発行され、同じ日次サイクルの
    // MilitaryActionで実際に前進が始まる ──
    advance_one_day(&mut app);
    let division_after_order = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&division_id)
        .unwrap()
        .clone();
    assert_eq!(
        division_after_order.status,
        DivisionStatus::Moving,
        "攻勢線・Offensive設定後の最初の日次処理で移動が発行されるはず"
    );
    assert_eq!(division_after_order.destination, Some(StateId(3)));
    assert!(
        app.world()
            .resource::<FrontlineRegistry>()
            .frontline_generated_movements
            .contains(&division_id)
    );

    // ── さらに日数を進め、実際の前進・敵領到着時の交戦判定まで一気通貫でつながる
    // ことを確認する(移動日数は速度・地形コストに依存するため、幅を持たせて確認する)。
    let mut reached_or_engaged = false;
    for _ in 0..30 {
        advance_one_day(&mut app);
        let division = app
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .get(&division_id)
            .unwrap();
        let state3_controller = app
            .world()
            .resource::<StateRegistry>()
            .get(StateId(3))
            .unwrap()
            .controller();
        if division.current_state == StateId(3)
            || division.status == DivisionStatus::Fighting
            || division.combat_id.is_some()
            || state3_controller == CountryId(0)
        {
            reached_or_engaged = true;
            break;
        }
        assert_ne!(
            division.status,
            DivisionStatus::Destroyed,
            "30日以内にDivisionが全滅することは想定しない(手動で不正な移動は発行していない)"
        );
    }
    assert!(
        reached_or_engaged,
        "攻勢線設定から30日以内に、Divisionが実際にState(3)へ到達するか交戦するはず \
         (FrontlineOrdersで発行した移動命令がMilitaryActionの実移動・侵攻処理へ \
         正しくつながっていることの証拠)"
    );

    // ── 実データE2Eでも、宣戦当日ガード・P21-006のArmy解決・既存の移動/侵攻処理を \
    // 一切新規実装していないことの間接的確認として、`process_division_arrival`が \
    // このcrateから引き続き参照可能であること(シグネチャ非破壊)だけを軽く確認する。
    let _ = process_division_arrival;
}

/// 要求テスト項目16/18相当: 実データ上でもArmyの前線解除後は新規攻撃が発生せず、
/// 和平相当のFrontline削除後は前線参照が残らない。
#[test]
fn unassigning_army_and_removing_frontline_stops_offensive_line_attacks_with_real_map_data() {
    let mut app = setup_app_in_playing();

    let war = War {
        id: WarId(0),
        name: "P21-008 Cleanup Test War".to_string(),
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
    advance_one_day(&mut app);
    let fl_id = app
        .world()
        .resource::<FrontlineRegistry>()
        .get_frontline_for_war(war_id)
        .unwrap()
        .frontline_id;

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
                .unwrap()
        });
    app.world_mut()
        .resource_scope(|world, army_registry: Mut<ArmyRegistry>| {
            world.resource_scope(|world, war_registry: Mut<WarRegistry>| {
                world
                    .resource_mut::<FrontlineRegistry>()
                    .assign_army(army_id, fl_id, CountryId(0), &army_registry, &war_registry)
                    .unwrap();
            });
        });
    app.world_mut()
        .resource_scope(|world, state_registry: Mut<StateRegistry>| {
            world.resource_scope(|world, war_registry: Mut<WarRegistry>| {
                world
                    .resource_mut::<FrontlineRegistry>()
                    .set_offensive_line(
                        fl_id,
                        CountryId(0),
                        &[StateId(3)],
                        &state_registry,
                        &war_registry,
                    )
                    .unwrap();
            });
        });
    if let Some(plan) = app
        .world_mut()
        .resource_mut::<FrontlineRegistry>()
        .get_plan_mut(fl_id, CountryId(0))
    {
        plan.stance = FrontlineStance::Offensive;
    }

    // Army前線解除後は、以降の日次処理で新規攻撃(destination設定)が発生しない。
    app.world_mut()
        .resource_scope(|world, army_registry: Mut<ArmyRegistry>| {
            world
                .resource_mut::<FrontlineRegistry>()
                .unassign_army(army_id, CountryId(0), &army_registry)
                .unwrap();
        });
    advance_one_day(&mut app);
    let division = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&division_id)
        .unwrap();
    assert_eq!(
        division.status,
        DivisionStatus::Idle,
        "Army前線解除後は新規攻撃が発生しないはず"
    );
    assert_eq!(division.destination, None);

    // Frontline削除(和平相当)後は、前線・作戦命令・部隊割り当ての参照が残らない。
    app.world_mut()
        .resource_scope(|world, military_registry: Mut<MilitaryRegistry>| {
            world
                .resource_mut::<FrontlineRegistry>()
                .remove_frontline(fl_id, &military_registry);
        });
    assert!(
        app.world()
            .resource::<FrontlineRegistry>()
            .get_frontline_for_war(war_id)
            .is_none()
    );
    assert!(
        !app.world()
            .resource::<FrontlineRegistry>()
            .army_frontline_map
            .contains_key(&army_id)
    );
}
