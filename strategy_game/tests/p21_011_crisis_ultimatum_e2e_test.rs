//! P21-011: Crisis最後通牒(ultimatum)・平和受諾・戦争正当化への接続の統合テスト。
//!
//! 前半は`GameTimePlugin`(実際の`DayChangedMessage`発行タイミング・Pause判定)だけを
//! 使った軽量Appで、期限切れによる自動拒否(`diplomacy::update::handle_daily_diplomacy`の
//! deadline判定)を検証する。後半は実データ(`assets/data/*.ron`、7か国28州の本番マップ)を
//! `AppPlugin`経由で読み込み、Claim作成→Crisis開始→受諾/拒否/撤回→(拒否時は宣戦布告接続)→
//! save往復までを一気通貫させる。

use bevy::prelude::*;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DailySimulationSet, GameDate, GamePaused, GameTimePlugin};
use strategy_game::common::CountryId;
use strategy_game::country::{CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::claims::ClaimRegistry;
use strategy_game::diplomacy::crisis::{CrisisPhase, CrisisRegistry};
use strategy_game::diplomacy::crisis_response;
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::diplomacy::update::handle_daily_diplomacy;
use strategy_game::localization::{CurrentLocale, TranslationCatalog};
use strategy_game::state::data::StateRegistry;
use strategy_game::ui::notification::GameNotification;
use strategy_game::war::justification::WarJustificationRegistry;

// ─────────────────────────────────────────────────────────────────────────
// 軽量App: GameTimePlugin(実際の日付進行・Pause判定)+ handle_daily_diplomacyのみ
// ─────────────────────────────────────────────────────────────────────────

fn build_lightweight_diplomacy_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .init_state::<GameState>()
        .add_plugins(GameTimePlugin);

    app.insert_resource(DiplomacyRegistry::default());
    app.insert_resource(CrisisRegistry::default());
    app.insert_resource(PlayerCountry(None));
    app.insert_resource(CountryRegistry::default());
    app.insert_resource(StateRegistry::build(vec![]));
    app.insert_resource(WarJustificationRegistry::default());
    app.insert_resource(ClaimRegistry::default());
    app.insert_resource(strategy_game::country::power::CountryPowerRegistry::default());
    app.insert_resource(CurrentLocale::default());
    app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
    app.add_message::<GameNotification>();

    app.add_systems(
        Update,
        handle_daily_diplomacy
            .in_set(DailySimulationSet::Diplomacy)
            .run_if(in_state(GameState::Playing)),
    );

    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn insert_demand_sent_crisis(
    app: &mut App,
    start_date: &str,
    deadline_date: &str,
) -> strategy_game::common::DiplomaticCrisisId {
    app.world_mut().resource_mut::<CrisisRegistry>().add_crisis(
        strategy_game::diplomacy::crisis::DiplomaticCrisis {
            id: strategy_game::common::DiplomaticCrisisId(0),
            initiator: CountryId(0),
            target: CountryId(1),
            war_goals: vec![strategy_game::diplomacy::crisis::WarGoal {
                attacker: CountryId(0),
                defender: CountryId(1),
                goal_type: strategy_game::diplomacy::crisis::WarGoalType::ConquerState,
                target_states: vec![strategy_game::common::StateId(1)],
                base_peace_cost: 0.0,
                international_concern: 0.0,
                completion: 0.0,
                is_primary: true,
            }],
            start_date: start_date.to_string(),
            current_phase: CrisisPhase::DemandSent,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: Some(deadline_date.to_string()),
            international_concern: 0.0,
            third_party_reactions: Default::default(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        },
    )
}

fn tick_respecting_pause(app: &mut App, accumulator_delta: f64) {
    app.world_mut()
        .resource_mut::<GameDate>()
        .add_accumulator(accumulator_delta);
    app.update();
}

/// 期限前日(day 29)ではまだDemandSentのまま。
#[test]
fn crisis_stays_demand_sent_one_day_before_deadline() {
    let mut app = build_lightweight_diplomacy_app();
    let id = insert_demand_sent_crisis(&mut app, "1800/01/01", "1800/01/31");

    app.world_mut().resource_mut::<GamePaused>().0 = false;
    for _ in 0..29 {
        tick_respecting_pause(&mut app, 1.0);
    }

    let crisis = app
        .world()
        .resource::<CrisisRegistry>()
        .crises
        .get(&id)
        .unwrap()
        .clone();
    assert_eq!(crisis.current_phase, CrisisPhase::DemandSent);
    assert_eq!(crisis.related_justification_id, None);
}

/// 期限到達日(day 30、GameDate::new(1800,1,1).add_days(30) == "1800/01/31")に、
/// 自動拒否(Escalating遷移 + initiatorへの完成済みJustification付与)が起きる。
#[test]
fn crisis_auto_rejects_and_grants_justification_when_deadline_is_reached() {
    let mut app = build_lightweight_diplomacy_app();
    let id = insert_demand_sent_crisis(&mut app, "1800/01/01", "1800/01/31");

    app.world_mut().resource_mut::<GamePaused>().0 = false;
    for _ in 0..30 {
        tick_respecting_pause(&mut app, 1.0);
    }

    let crisis = app
        .world()
        .resource::<CrisisRegistry>()
        .crises
        .get(&id)
        .unwrap()
        .clone();
    assert_eq!(
        crisis.current_phase,
        CrisisPhase::Escalating,
        "deadline reached must auto-reject into Escalating"
    );
    let j_id = crisis
        .related_justification_id
        .expect("auto-rejection must grant a justification id");
    let justification = app
        .world()
        .resource::<WarJustificationRegistry>()
        .justifications
        .get(&j_id)
        .unwrap()
        .clone();
    assert!(justification.is_ready);
    assert_eq!(justification.initiator, CountryId(0));
    assert_eq!(justification.target, CountryId(1));

    // 一度Escalatingへ遷移した後は、繰り返しtickしても再度拒否処理が走らない
    // (related_justification_idが変化しない = 同じjustification idのまま)。
    tick_respecting_pause(&mut app, 1.0);
    let crisis_after = app
        .world()
        .resource::<CrisisRegistry>()
        .crises
        .get(&id)
        .unwrap()
        .clone();
    assert_eq!(crisis_after.related_justification_id, Some(j_id));
}

// ─────────────────────────────────────────────────────────────────────────
// 実データE2E(実7か国28州マップ)
// ─────────────────────────────────────────────────────────────────────────

mod real_map_e2e {
    use super::*;
    use bevy::app::ScheduleRunnerPlugin;
    use strategy_game::app::AppPlugin;
    use strategy_game::building::BuildingPlugin;
    use strategy_game::common::StateId;
    use strategy_game::diplomacy::claims::ClaimSource;
    use strategy_game::economy::EconomyPlugin;
    use strategy_game::military::MilitaryPlugin;
    use strategy_game::politics::PoliticsPlugin;
    use strategy_game::research::ResearchPlugin;
    use strategy_game::research::data::TechnologyRegistry;
    use strategy_game::research::world_stage::WorldCivilizationState;
    use strategy_game::save::{
        ApplyLoadOutcome, LoadGamePlugin, SaveGamePlugin, SaveGameSources, SaveValidationContext,
        apply_validated_save, build_save_game_v1, validate_save_game_v1,
    };
    use strategy_game::state::StatePlugin;
    use strategy_game::war::WarPlugin;
    use strategy_game::war::data::WarRegistry;

    fn setup_app_in_playing() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_resource::<ButtonInput<KeyCode>>()
            .add_plugins(AppPlugin)
            .add_plugins(strategy_game::country::CountryPlugin)
            .add_plugins(StatePlugin)
            .add_plugins(BuildingPlugin)
            .add_plugins(EconomyPlugin)
            .add_plugins(ResearchPlugin)
            .add_plugins(PoliticsPlugin)
            .add_plugins(strategy_game::diplomacy::DiplomacyPlugin)
            .add_plugins(WarPlugin)
            .add_plugins(MilitaryPlugin);

        app.insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
            .insert_resource(strategy_game::map::division_selection::DragSelectState::default())
            .insert_resource(strategy_game::military::army::SelectedArmy::default())
            .insert_resource(strategy_game::state::SelectedState::default())
            .insert_resource(strategy_game::ui::ActivePanel::default())
            .insert_resource(strategy_game::ui::diplomacy_panel::DiplomacyPanelState::default())
            .insert_resource(strategy_game::ui::military_panel::MilitaryPanelState::default())
            .insert_resource(strategy_game::ui::peace_panel::PeacePanelState::default())
            .insert_resource(strategy_game::ui::politics_panel::PoliticsPanelState::default())
            .insert_resource(strategy_game::ui::research_panel::ResearchPanelState::default())
            .insert_resource(strategy_game::map::camera::CameraDragState::default());

        app.add_plugins(SaveGamePlugin);
        app.add_plugins(LoadGamePlugin);

        app.update();

        app.insert_resource(PlayerCountry(Some(CountryId(0))));
        app.insert_state(GameState::Playing);
        app.update();

        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::Playing
        );
        app
    }

    fn snapshot_save(app: &App) -> strategy_game::save::SaveGameV1 {
        let world = app.world();
        let sources = SaveGameSources {
            game_date: world.resource(),
            game_speed: world.resource(),
            player_country: world.resource(),
            world_civilization: world.resource(),
            country_registry: world.resource(),
            state_registry: world.resource(),
            diplomacy_registry: world.resource(),
            war_justification_registry: world.resource(),
            war_registry: world.resource(),
            claim_registry: world.resource(),
            crisis_registry: world.resource(),
            country_ai_registry: world.resource(),
            military_ai_registry: world.resource(),
            military_registry: world.resource(),
            battle_registry: world.resource(),
            army_registry: world.resource(),
            frontline_registry: world.resource(),
        };
        build_save_game_v1(&sources)
    }

    fn validation_context(app: &App) -> SaveValidationContext<'_> {
        let world = app.world();
        SaveValidationContext {
            building_definitions: &world
                .resource::<strategy_game::building::data::BuildingRegistry>()
                .definitions,
            technology_definitions: &world.resource::<TechnologyRegistry>().definitions,
            division_definitions: &world
                .resource::<strategy_game::military::data::MilitaryRegistry>()
                .definitions,
            world_stage_definitions: &world.resource::<WorldCivilizationState>().stage_definitions,
        }
    }

    fn round_trip_through_save(app: &mut App) {
        let save = snapshot_save(app);
        let validated = {
            let context = validation_context(app);
            validate_save_game_v1(save, &context)
                .expect("real-map save with P21-011 crisis state must validate")
        };
        assert_eq!(
            apply_validated_save(app.world_mut(), validated),
            ApplyLoadOutcome::Success
        );
    }

    /// CountryId(0)からCountryId(1)所有の`target_state`へのClaimを作成し、Crisisを開始する。
    fn create_claim_and_start_crisis(
        app: &mut App,
        target_state: StateId,
    ) -> (
        strategy_game::common::ClaimId,
        strategy_game::common::DiplomaticCrisisId,
    ) {
        let claim_id = {
            let world = app.world_mut();
            let country_registry = world.resource::<CountryRegistry>().clone_for_test();
            let state_registry = world.resource::<StateRegistry>().clone_for_test();
            assert_eq!(
                state_registry.get(target_state).map(|s| s.owner_country_id),
                Some(CountryId(1))
            );
            let date_str = world.resource::<GameDate>().display();
            let mut claim_registry = world.resource_mut::<ClaimRegistry>();
            claim_registry
                .create_claim(
                    CountryId(0),
                    CountryId(1),
                    target_state,
                    date_str,
                    ClaimSource::Strategic,
                    &country_registry,
                    &state_registry,
                )
                .expect("valid real-map claim must succeed")
        };

        let crisis_id = {
            let world = app.world_mut();
            let claim = world
                .resource::<ClaimRegistry>()
                .claims
                .get(&claim_id)
                .unwrap()
                .clone();
            let date_str = world.resource::<GameDate>().display();
            let state_registry = world.resource::<StateRegistry>().clone_for_test();
            let mut crisis_registry = world.resource_mut::<CrisisRegistry>();
            crisis_registry
                .start_crisis(
                    &claim,
                    CountryId(0),
                    CountryId(1),
                    date_str,
                    &state_registry,
                )
                .expect("valid claim must allow crisis start")
        };

        (claim_id, crisis_id)
    }

    /// 実データE2E: 受諾により州が移転しClaimが消費され、save往復後も維持される。
    #[test]
    fn accept_demand_transfers_state_and_survives_save_round_trip() {
        let mut app = setup_app_in_playing();
        // StateId(4)"Forest Research Zone"はCountryId(1)所有だが首都ではない
        // (首都StateId(3)を移転すると`capital_state_id`不整合でsave検証に失敗するため)。
        let (claim_id, crisis_id) = create_claim_and_start_crisis(&mut app, StateId(4));

        app.world_mut()
            .resource_scope(|world, mut crisis_registry: Mut<CrisisRegistry>| {
                world.resource_scope(|world, mut claim_registry: Mut<ClaimRegistry>| {
                    let mut state_registry = world.resource_mut::<StateRegistry>();
                    crisis_response::accept_demand(
                        &mut crisis_registry,
                        &mut claim_registry,
                        &mut state_registry,
                        crisis_id,
                        CountryId(1),
                    )
                    .expect("target must be able to accept the demand");
                });
            });

        assert_eq!(
            app.world()
                .resource::<StateRegistry>()
                .get(StateId(4))
                .unwrap()
                .owner_country_id,
            CountryId(0)
        );
        assert_eq!(
            app.world()
                .resource::<ClaimRegistry>()
                .claims
                .get(&claim_id)
                .unwrap()
                .status,
            strategy_game::diplomacy::claims::ClaimStatus::Consumed
        );

        round_trip_through_save(&mut app);

        assert_eq!(
            app.world()
                .resource::<StateRegistry>()
                .get(StateId(4))
                .unwrap()
                .owner_country_id,
            CountryId(0),
            "accepted state ownership must survive a save round trip"
        );
        assert_eq!(
            app.world()
                .resource::<ClaimRegistry>()
                .claims
                .get(&claim_id)
                .unwrap()
                .status,
            strategy_game::diplomacy::claims::ClaimStatus::Consumed,
            "consumed claim status must survive a save round trip"
        );
        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::ResolvedPeacefully
        );
    }

    /// 実データE2E: 拒否→Justification付与→既存の宣戦布告APIで宣戦→
    /// CrisisがWarStartedへ同期し、save往復後もrelated_war_idが維持される。
    #[test]
    fn reject_then_declare_war_syncs_crisis_to_war_started_and_survives_save_round_trip() {
        let mut app = setup_app_in_playing();
        let (_claim_id, crisis_id) = create_claim_and_start_crisis(&mut app, StateId(3));

        let date_str = app.world().resource::<GameDate>().display();
        app.world_mut()
            .resource_scope(|world, mut crisis_registry: Mut<CrisisRegistry>| {
                world.resource_scope(
                    |_world, mut justification_registry: Mut<WarJustificationRegistry>| {
                        crisis_response::reject_demand(
                            &mut crisis_registry,
                            &mut justification_registry,
                            crisis_id,
                            CountryId(1),
                            date_str.clone(),
                        )
                        .expect("target must be able to reject the demand");
                    },
                );
            });

        let j_id = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .related_justification_id
            .expect("rejection must grant a justification id");

        // 既存の宣戦布告処理(UI/AIが呼ぶのと同じ`WarRegistry::declare_war`)経由で宣戦する。
        let country_registry = app.world().resource::<CountryRegistry>().clone_for_test();
        let state_registry = app.world().resource::<StateRegistry>().clone_for_test();
        let war_id = app
            .world_mut()
            .resource_scope(|world, mut war_registry: Mut<WarRegistry>| {
                world.resource_scope(|world, mut diplomacy_registry: Mut<DiplomacyRegistry>| {
                    let mut justification_registry =
                        world.resource_mut::<WarJustificationRegistry>();
                    war_registry
                        .declare_war(
                            CountryId(0),
                            CountryId(1),
                            StateId(3),
                            date_str,
                            &country_registry,
                            &state_registry,
                            &mut diplomacy_registry,
                            &mut justification_registry,
                        )
                        .expect(
                            "initiator must be able to declare war using the granted justification",
                        )
                })
            });

        crisis_response::sync_crisis_on_war_declared(
            &mut app.world_mut().resource_mut::<CrisisRegistry>(),
            CountryId(0),
            CountryId(1),
            j_id,
            war_id,
        );

        let crisis = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .clone();
        assert_eq!(crisis.current_phase, CrisisPhase::WarStarted);
        assert_eq!(crisis.related_war_id, Some(war_id));

        round_trip_through_save(&mut app);

        let crisis_after = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .clone();
        assert_eq!(
            crisis_after.current_phase,
            CrisisPhase::WarStarted,
            "WarStarted phase must survive a save round trip"
        );
        assert_eq!(
            crisis_after.related_war_id,
            Some(war_id),
            "related_war_id must survive a save round trip"
        );
        assert!(
            app.world()
                .resource::<WarRegistry>()
                .wars
                .contains_key(&war_id)
        );
    }

    /// 実データE2E: initiatorによる撤回はCancelledへ遷移し、save往復後も維持される。
    #[test]
    fn withdraw_cancels_crisis_and_survives_save_round_trip() {
        let mut app = setup_app_in_playing();
        let (_claim_id, crisis_id) = create_claim_and_start_crisis(&mut app, StateId(3));

        app.world_mut()
            .resource_scope(|world, mut crisis_registry: Mut<CrisisRegistry>| {
                world.resource_scope(
                    |_world, mut justification_registry: Mut<WarJustificationRegistry>| {
                        crisis_response::withdraw_crisis(
                            &mut crisis_registry,
                            &mut justification_registry,
                            crisis_id,
                            CountryId(0),
                        )
                        .expect("initiator must be able to withdraw");
                    },
                );
            });

        round_trip_through_save(&mut app);

        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::Cancelled,
            "Cancelled phase must survive a save round trip"
        );
    }

    trait CloneForTest {
        fn clone_for_test(&self) -> Self;
    }
    impl CloneForTest for CountryRegistry {
        fn clone_for_test(&self) -> Self {
            CountryRegistry {
                countries: self.countries.clone(),
            }
        }
    }
    impl CloneForTest for StateRegistry {
        fn clone_for_test(&self) -> Self {
            StateRegistry::build(self.states.clone())
        }
    }
}
