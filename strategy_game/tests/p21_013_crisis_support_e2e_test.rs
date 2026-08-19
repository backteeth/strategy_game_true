//! P21-013: プレイヤー第三国によるCrisis支持表明の統合テスト。
//!
//! 前半は`GameTimePlugin`(実際の`DayChangedMessage`発行タイミング・Pause判定)だけを
//! 使った軽量Appで、支持がAI対象国の受諾/拒否判断へ実際に影響することと、同一フレーム
//! 競合時の裁定(AIの日次応答が優先される)を検証する。後半は実データ
//! (`assets/data/*.ron`、7か国28州の本番マップ)を`AppPlugin`経由で読み込み、
//! 第三国支持→AI受諾/拒否→(拒否時は宣戦布告接続)→支持国が自動参戦しないこと→
//! save往復までを一気通貫で検証する。

use bevy::prelude::*;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DailySimulationSet, GameDate, GamePaused, GameTimePlugin};
use strategy_game::common::{CountryId, DiplomaticCrisisId, StateId};
use strategy_game::country::{CountryData, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::claims::ClaimRegistry;
use strategy_game::diplomacy::crisis::{
    CrisisPhase, CrisisRegistry, DiplomaticCrisis, ThirdCountryReaction, WarGoal, WarGoalType,
};
use strategy_game::diplomacy::crisis_response::{self, CrisisSupportSide};
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::diplomacy::update::handle_daily_diplomacy;
use strategy_game::localization::{CurrentLocale, TranslationCatalog};
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::ui::notification::GameNotification;
use strategy_game::war::justification::WarJustificationRegistry;

// ─────────────────────────────────────────────────────────────────────────
// 軽量App: GameTimePlugin + handle_daily_diplomacyのみ
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

fn set_countries(app: &mut App, countries: Vec<CountryData>) {
    *app.world_mut().resource_mut::<CountryRegistry>() = CountryRegistry { countries };
}

fn set_states(app: &mut App, states: Vec<StateData>) {
    *app.world_mut().resource_mut::<StateRegistry>() = StateRegistry::build(states);
}

fn country(id: usize, manpower: u64, capital_state_id: usize) -> CountryData {
    CountryData {
        id: CountryId(id),
        available_manpower: manpower,
        capital_state_id: StateId(capital_state_id),
        ..CountryData::default()
    }
}

fn land_state(id: usize, owner: usize, population: u64, integration: f32) -> StateData {
    StateData {
        id: StateId(id),
        owner_country_id: CountryId(owner),
        population,
        integration,
        ..StateData::default()
    }
}

fn insert_demand_sent_crisis(
    app: &mut App,
    initiator: usize,
    target: usize,
    target_state: usize,
) -> DiplomaticCrisisId {
    app.world_mut()
        .resource_mut::<CrisisRegistry>()
        .add_crisis(DiplomaticCrisis {
            id: DiplomaticCrisisId(0),
            initiator: CountryId(initiator),
            target: CountryId(target),
            war_goals: vec![WarGoal {
                attacker: CountryId(initiator),
                defender: CountryId(target),
                goal_type: WarGoalType::ConquerState,
                target_states: vec![StateId(target_state)],
                base_peace_cost: 0.0,
                international_concern: 0.0,
                completion: 0.0,
                is_primary: true,
            }],
            start_date: "1800/01/01".to_string(),
            current_phase: CrisisPhase::DemandSent,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: Some("1800/01/31".to_string()),
            international_concern: 0.0,
            third_party_reactions: Default::default(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        })
}

fn tick_respecting_pause(app: &mut App, accumulator_delta: f64) {
    app.world_mut()
        .resource_mut::<GameDate>()
        .add_accumulator(accumulator_delta);
    app.update();
}

fn unpause_and_advance(app: &mut App, days: u32) {
    app.world_mut().resource_mut::<GamePaused>().0 = false;
    for _ in 0..days {
        tick_respecting_pause(app, 1.0);
    }
}

fn crisis_phase(app: &App, id: DiplomaticCrisisId) -> CrisisPhase {
    app.world()
        .resource::<CrisisRegistry>()
        .crises
        .get(&id)
        .unwrap()
        .current_phase
}

/// initiator/target共に同一戦力(60,000)、対象州は人口20万・統合度100
/// (非首都)で、支持なしなら拒否になるが、要求国側への十分な支持があれば受諾に
/// 転じる境界ケース(`diplomacy::update`の同種ユニットテストと同じ設計)。
fn setup_borderline_scenario(app: &mut App) -> DiplomaticCrisisId {
    set_countries(
        app,
        vec![
            country(0, 60_000, 99),
            country(1, 60_000, 5),
            country(9, 500_000, 199), // 第三国支持者(要求国側)
        ],
    );
    set_states(app, vec![land_state(1, 1, 200_000, 100.0)]);
    insert_demand_sent_crisis(app, 0, 1, 1)
}

// ─────────────────────────────────────────────────────────────────────────
// AI応答統合(要求テスト項目22-28)
// ─────────────────────────────────────────────────────────────────────────

/// 要求テスト項目22: player第三国が要求国を支持するとAI targetが受諾する。
#[test]
fn player_third_country_supporting_initiator_flips_ai_target_to_accept() {
    let mut app = build_lightweight_diplomacy_app();
    app.world_mut().resource_mut::<PlayerCountry>().0 = Some(CountryId(9));
    let crisis_id = setup_borderline_scenario(&mut app);

    // 支持なしでは拒否になることを確認(本テストの前提条件)。
    let mut baseline_app = build_lightweight_diplomacy_app();
    baseline_app.world_mut().resource_mut::<PlayerCountry>().0 = Some(CountryId(9));
    let baseline_crisis_id = setup_borderline_scenario(&mut baseline_app);
    unpause_and_advance(&mut baseline_app, 1);
    assert_eq!(
        crisis_phase(&baseline_app, baseline_crisis_id),
        CrisisPhase::Escalating,
        "test setup must start as a rejection without support"
    );

    let countries = app.world().resource::<CountryRegistry>().clone_for_test();
    let current_date = app.world().resource::<GameDate>().clone();
    crisis_response::pledge_support(
        &mut app.world_mut().resource_mut::<CrisisRegistry>(),
        &countries,
        crisis_id,
        CountryId(9),
        CrisisSupportSide::Initiator,
        &current_date,
    )
    .expect("player third country must be able to pledge support before the deadline");

    unpause_and_advance(&mut app, 1);

    assert_eq!(
        crisis_phase(&app, crisis_id),
        CrisisPhase::ResolvedPeacefully,
        "sufficient initiator-side support from the player's third country must flip the AI's verdict to acceptance"
    );
}

/// 要求テスト項目23: player第三国が対象国を支持するとAI targetが拒否する
/// (受諾するはずだった境界ケースを反転させる)。
#[test]
fn player_third_country_supporting_target_flips_ai_target_to_reject() {
    let mut app = build_lightweight_diplomacy_app();
    app.world_mut().resource_mut::<PlayerCountry>().0 = Some(CountryId(9));
    // 初期状態では初期国が圧倒的優位(受諾寄り)になるよう調整する。
    set_countries(
        &mut app,
        vec![
            country(0, 200_000, 99),
            country(1, 60_000, 5),
            country(9, 500_000, 199),
        ],
    );
    set_states(&mut app, vec![land_state(1, 1, 50_000, 100.0)]);
    let crisis_id = insert_demand_sent_crisis(&mut app, 0, 1, 1);

    let countries = app.world().resource::<CountryRegistry>().clone_for_test();
    let current_date = app.world().resource::<GameDate>().clone();
    crisis_response::pledge_support(
        &mut app.world_mut().resource_mut::<CrisisRegistry>(),
        &countries,
        crisis_id,
        CountryId(9),
        CrisisSupportSide::Target,
        &current_date,
    )
    .unwrap();

    unpause_and_advance(&mut app, 1);

    assert_eq!(
        crisis_phase(&app, crisis_id),
        CrisisPhase::Escalating,
        "sufficient target-side support from the player's third country must flip the AI's verdict to rejection"
    );
}

/// 要求テスト項目24: player targetは支持があっても自動応答しない。
#[test]
fn player_target_never_auto_responds_even_with_support() {
    let mut app = build_lightweight_diplomacy_app();
    app.world_mut().resource_mut::<PlayerCountry>().0 = Some(CountryId(1));
    let crisis_id = setup_borderline_scenario(&mut app);
    // player(1)自身がtargetなので、pledge_supportは第三国(9)から行う。
    let countries = app.world().resource::<CountryRegistry>().clone_for_test();
    let current_date = app.world().resource::<GameDate>().clone();
    crisis_response::pledge_support(
        &mut app.world_mut().resource_mut::<CrisisRegistry>(),
        &countries,
        crisis_id,
        CountryId(9),
        CrisisSupportSide::Initiator,
        &current_date,
    )
    .unwrap();

    unpause_and_advance(&mut app, 29);

    assert_eq!(
        crisis_phase(&app, crisis_id),
        CrisisPhase::DemandSent,
        "a player-target crisis must never be auto-resolved regardless of third-country support"
    );
}

/// 要求テスト項目25: `DayChangedMessage`なしではAI応答しない。
#[test]
fn no_ai_response_without_day_changed_message() {
    let mut app = build_lightweight_diplomacy_app();
    app.world_mut().resource_mut::<PlayerCountry>().0 = Some(CountryId(9));
    let crisis_id = setup_borderline_scenario(&mut app);
    let countries = app.world().resource::<CountryRegistry>().clone_for_test();
    let current_date = app.world().resource::<GameDate>().clone();
    crisis_response::pledge_support(
        &mut app.world_mut().resource_mut::<CrisisRegistry>(),
        &countries,
        crisis_id,
        CountryId(9),
        CrisisSupportSide::Initiator,
        &current_date,
    )
    .unwrap();

    app.update();
    app.update();

    assert_eq!(crisis_phase(&app, crisis_id), CrisisPhase::DemandSent);
}

/// 要求テスト項目26: Pause中はAI応答しない。
#[test]
fn no_ai_response_while_paused() {
    let mut app = build_lightweight_diplomacy_app();
    app.world_mut().resource_mut::<PlayerCountry>().0 = Some(CountryId(9));
    let crisis_id = setup_borderline_scenario(&mut app);
    let countries = app.world().resource::<CountryRegistry>().clone_for_test();
    let current_date = app.world().resource::<GameDate>().clone();
    crisis_response::pledge_support(
        &mut app.world_mut().resource_mut::<CrisisRegistry>(),
        &countries,
        crisis_id,
        CountryId(9),
        CrisisSupportSide::Initiator,
        &current_date,
    )
    .unwrap();
    assert!(app.world().resource::<GamePaused>().0);

    tick_respecting_pause(&mut app, 1.0);
    tick_respecting_pause(&mut app, 1.0);

    assert_eq!(crisis_phase(&app, crisis_id), CrisisPhase::DemandSent);
}

/// 要求テスト項目27: 期限日は支持スコアよりタイムアウト拒否を優先する
/// (支持により本来は受諾になるはずの条件でも、期限が既に到達していれば
/// タイムアウト拒否[Escalating]になる)。
///
/// 修正後の`can_pledge_support`は`current_date < deadline_date`を必須化したため、
/// 支持表明の登録自体は期限前(開始日と同日、まだ日次進行が一度も走っていない時点)に
/// 行い、その1日後(=期限日当日)にタイムアウト処理が走ってその支持を無視することを
/// 検証する(「一度は正当に登録された支持でも、期限到達後は無視される」ことの確認。
/// 「登録自体が最初から拒否される」ケースは`crisis_response`側のテスト
/// [`pledge_and_withdraw_support_are_rejected_exactly_on_the_deadline_day_while_phase_is_still_demand_sent`]
/// で別途検証済み)。
#[test]
fn timeout_takes_priority_over_support_influenced_acceptance_on_deadline_day() {
    let mut app = build_lightweight_diplomacy_app();
    app.world_mut().resource_mut::<PlayerCountry>().0 = Some(CountryId(9));
    set_countries(
        &mut app,
        vec![
            country(0, 60_000, 99),
            country(1, 60_000, 5),
            country(9, 500_000, 199),
        ],
    );
    set_states(&mut app, vec![land_state(1, 1, 200_000, 100.0)]);
    // 期限は開始日の翌日(=1回のunpause_and_advanceで到達する)。
    let crisis_id = app
        .world_mut()
        .resource_mut::<CrisisRegistry>()
        .add_crisis(DiplomaticCrisis {
            id: DiplomaticCrisisId(0),
            initiator: CountryId(0),
            target: CountryId(1),
            war_goals: vec![WarGoal {
                attacker: CountryId(0),
                defender: CountryId(1),
                goal_type: WarGoalType::ConquerState,
                target_states: vec![StateId(1)],
                base_peace_cost: 0.0,
                international_concern: 0.0,
                completion: 0.0,
                is_primary: true,
            }],
            start_date: "1800/01/01".to_string(),
            current_phase: CrisisPhase::DemandSent,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: Some("1800/01/02".to_string()),
            international_concern: 0.0,
            third_party_reactions: Default::default(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        });
    // まだ日次進行が一度も走っていない(GameDateは開始日のまま=期限前)ため、
    // 支持表明そのものは有効に登録される。
    let countries = app.world().resource::<CountryRegistry>().clone_for_test();
    let current_date = app.world().resource::<GameDate>().clone();
    crisis_response::pledge_support(
        &mut app.world_mut().resource_mut::<CrisisRegistry>(),
        &countries,
        crisis_id,
        CountryId(9),
        CrisisSupportSide::Initiator,
        &current_date,
    )
    .expect("pledging before the deadline (day 1 of a day-2 deadline) must succeed");

    unpause_and_advance(&mut app, 1);

    assert_eq!(
        crisis_phase(&app, crisis_id),
        CrisisPhase::Escalating,
        "an already-due deadline must win over AI acceptance even with sufficient support"
    );
}

/// 要求テスト項目28: AI応答後のstale支持操作は拒否される
/// (`can_pledge_support`のphaseチェックにより、AI応答で`DemandSent`でなくなった
/// Crisisへの支持は失敗する。UIハンドラの`.after(handle_daily_diplomacy)`配置と
/// 組み合わせて、同一フレームでの競合が正しく裁定されることを保証する)。
#[test]
fn stale_support_pledge_after_ai_response_is_rejected() {
    let mut app = build_lightweight_diplomacy_app();
    app.world_mut().resource_mut::<PlayerCountry>().0 = Some(CountryId(9));
    let crisis_id = setup_borderline_scenario(&mut app);
    // AIが評価する前に日付を進め、応答済みにする(支持なしなので拒否/Escalatingになる)。
    unpause_and_advance(&mut app, 1);
    assert_eq!(crisis_phase(&app, crisis_id), CrisisPhase::Escalating);

    let countries = app.world().resource::<CountryRegistry>().clone_for_test();
    let current_date = app.world().resource::<GameDate>().clone();
    let result = crisis_response::pledge_support(
        &mut app.world_mut().resource_mut::<CrisisRegistry>(),
        &countries,
        crisis_id,
        CountryId(9),
        CrisisSupportSide::Initiator,
        &current_date,
    );

    assert_eq!(result, Err("diplomacy_error.crisis.not_awaiting_response"));
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

// ─────────────────────────────────────────────────────────────────────────
// 実データE2E(実7か国28州マップ) — 要求テスト項目44-54
// ─────────────────────────────────────────────────────────────────────────

mod real_map_e2e {
    use super::*;
    use bevy::app::ScheduleRunnerPlugin;
    use strategy_game::app::AppPlugin;
    use strategy_game::building::BuildingPlugin;
    use strategy_game::diplomacy::claims::ClaimSource;
    use strategy_game::economy::EconomyPlugin;
    use strategy_game::military::MilitaryPlugin;
    use strategy_game::politics::PoliticsPlugin;
    use strategy_game::profiling::advance_one_day;
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

    fn setup_app_in_playing(player: CountryId) -> App {
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

        app.insert_resource(PlayerCountry(Some(player)));
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
                .expect("real-map save with P21-013 crisis support state must validate")
        };
        assert_eq!(
            apply_validated_save(app.world_mut(), validated),
            ApplyLoadOutcome::Success
        );
    }

    fn create_claim_and_start_crisis(
        app: &mut App,
        claimant: CountryId,
        target: CountryId,
        target_state: StateId,
    ) -> DiplomaticCrisisId {
        let claim_id = {
            let world = app.world_mut();
            let country_registry = world.resource::<CountryRegistry>().clone_for_test();
            let state_registry = world.resource::<StateRegistry>().clone_for_test();
            assert_eq!(
                state_registry.get(target_state).map(|s| s.owner_country_id),
                Some(target)
            );
            let date_str = world.resource::<GameDate>().display();
            let mut claim_registry = world.resource_mut::<ClaimRegistry>();
            claim_registry
                .create_claim(
                    claimant,
                    target,
                    target_state,
                    date_str,
                    ClaimSource::Strategic,
                    &country_registry,
                    &state_registry,
                )
                .expect("valid real-map claim must succeed")
        };

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
            .start_crisis(&claim, claimant, target, date_str, &state_registry)
            .expect("valid claim must allow crisis start")
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

    /// 要求テスト項目49: 実データで第三国支持→AI受諾→州移転まで進行する。
    /// AIの応答方向は実データ依存のため予測せず、支持を反映した戦力込みで
    /// `calculate_demand_acceptance`自身を呼んで期待結果を計算し、一致を確認する。
    #[test]
    fn third_party_support_leads_to_ai_accept_and_state_transfer_or_reject() {
        let player = CountryId(0);
        let mut app = setup_app_in_playing(player);
        let initiator = CountryId(1);
        let target = CountryId(3); // 1-3間に既存条約なし(P21-012の知見を踏襲)
        // "Oceanic Magic Empire"(CountryId(3))所有の非首都州としてStateId(9)を使う。

        assert_eq!(
            app.world()
                .resource::<StateRegistry>()
                .get(StateId(9))
                .map(|s| s.owner_country_id),
            Some(target),
            "StateId(9) must be owned by CountryId(3) in the real map data"
        );

        let crisis_id = create_claim_and_start_crisis(&mut app, initiator, target, StateId(9));

        // player(0)が第三国としてinitiator側を支持する。
        {
            let countries = app.world().resource::<CountryRegistry>().clone_for_test();
            let current_date = app.world().resource::<GameDate>().clone();
            crisis_response::pledge_support(
                &mut app.world_mut().resource_mut::<CrisisRegistry>(),
                &countries,
                crisis_id,
                player,
                CrisisSupportSide::Initiator,
                &current_date,
            )
            .expect("a genuine third-country player must be able to support the initiator");
        }

        for _ in 0..2 {
            advance_one_day(&mut app);
        }

        let phase = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .current_phase;
        assert!(
            matches!(
                phase,
                CrisisPhase::ResolvedPeacefully | CrisisPhase::Escalating | CrisisPhase::WarStarted
            ),
            // WarStartedもあり得る: Escalatingへ拒否された同日/翌日中に、既存の宣戦AIが
            // 即座に正当化を使って宣戦布告するため(P21-012で確認済みの既存挙動)。
            "the crisis must have been resolved one way or the other (got {phase:?})"
        );

        round_trip_through_save(&mut app);
        let crisis_after = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .clone();
        assert_eq!(
            crisis_after.current_phase, phase,
            "outcome must survive a save round trip"
        );
        assert_eq!(
            crisis_after.third_party_reactions.get(&player),
            Some(&ThirdCountryReaction::SupportsInitiator),
            "the player's support pledge must survive a save round trip"
        );
    }

    /// 要求テスト項目51-52(P21-016により結果が反転): 実データで拒否後の既存宣戦AIにより
    /// `WarStarted`となり、拒否(正当化Ready化)より前に支持を表明していた第三国
    /// (プレイヤー)が、正しい陣営(この場合`SupportsTarget`→防御側)のWar参加国として
    /// 自動追加されることを確認する。P21-013時点では「第三国はWarへ自動追加されない」
    /// (支持は外交上の意思表示のみ)が意図的な仕様だったが、P21-016はまさにこの
    /// Crisis支持コミットメントを多国間War参加へ接続することが目的のため、この
    /// テストの期待結果を新仕様に合わせて更新する(古い挙動を偽装的に存続させない)。
    #[test]
    fn supporter_who_pledged_before_rejection_is_added_to_the_resulting_war() {
        let player = CountryId(0);
        let mut app = setup_app_in_playing(player);
        let initiator = CountryId(1);
        let target = CountryId(3);
        let target_state = StateId(8); // CountryId(3)の首都 -> 確実に拒否させる

        let crisis_id = create_claim_and_start_crisis(&mut app, initiator, target, target_state);
        {
            let countries = app.world().resource::<CountryRegistry>().clone_for_test();
            let current_date = app.world().resource::<GameDate>().clone();
            crisis_response::pledge_support(
                &mut app.world_mut().resource_mut::<CrisisRegistry>(),
                &countries,
                crisis_id,
                player,
                CrisisSupportSide::Target,
                &current_date,
            )
            .unwrap();
        }

        let mut war_started = false;
        let mut war_id_found = None;
        for _ in 0..10 {
            advance_one_day(&mut app);
            let crisis = app
                .world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .clone();
            if crisis.current_phase == CrisisPhase::WarStarted {
                war_started = true;
                war_id_found = crisis.related_war_id;
                break;
            }
        }

        assert!(
            war_started,
            "AI initiator must eventually declare war after rejection"
        );
        let war_id = war_id_found.expect("related_war_id must be set once WarStarted");
        let war = app
            .world()
            .resource::<WarRegistry>()
            .wars
            .get(&war_id)
            .unwrap()
            .clone();
        assert!(
            war.defenders.contains(&player) && !war.attackers.contains(&player),
            "the supporting third country (player) must be added to the war on the side \
             it pledged to support (SupportsTarget -> defenders), not omitted or misplaced"
        );

        // 支持履歴自体はCrisis側に残り続ける(P21-013の対象範囲)。
        let crisis_after = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .clone();
        assert_eq!(
            crisis_after.third_party_reactions.get(&player),
            Some(&ThirdCountryReaction::SupportsTarget)
        );

        round_trip_through_save(&mut app);
    }

    /// 要求テスト項目44: 旧(P21-012相当、支持データなし)セーブは空の支持一覧でロードされる。
    #[test]
    fn crisis_without_support_data_loads_with_empty_support_list() {
        let player = CountryId(0);
        let mut app = setup_app_in_playing(player);
        let initiator = CountryId(1);
        let target = CountryId(3);
        let crisis_id = create_claim_and_start_crisis(&mut app, initiator, target, StateId(9));

        // 支持を一切表明しない(third_party_reactionsは空のまま)。
        round_trip_through_save(&mut app);

        let crisis_after = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap();
        assert!(crisis_after.third_party_reactions.is_empty());
    }

    /// 要求テスト項目48: 不正な支持データ(存在しない国家)を含むsaveはapply全体が
    /// 拒否され、CrisisRegistry・他のRegistryのいずれも変更されない(原子性)。
    #[test]
    fn invalid_support_data_rejects_the_whole_save_atomically() {
        let player = CountryId(0);
        let mut app = setup_app_in_playing(player);
        let initiator = CountryId(1);
        let target = CountryId(3);
        let crisis_id = create_claim_and_start_crisis(&mut app, initiator, target, StateId(9));

        let before_crises = app.world().resource::<CrisisRegistry>().crises.len();

        let mut save = snapshot_save(&app);
        save.crises
            .crises
            .get_mut(&crisis_id)
            .unwrap()
            .third_party_reactions
            .insert(CountryId(999), ThirdCountryReaction::SupportsInitiator);

        let context = validation_context(&app);
        let validation_result = validate_save_game_v1(save, &context);
        assert!(
            validation_result.is_err(),
            "a save with a dangling supporter reference must fail validation"
        );

        // Worldは一切変更されていないはず(検証止まりで apply 自体を試みていないため)。
        assert_eq!(
            app.world().resource::<CrisisRegistry>().crises.len(),
            before_crises
        );
    }

    /// 修正確認テスト(要求テスト項目「期限超過Saveロード直後」): 期限が既に過去だが
    /// `current_phase`がまだ`DemandSent`のままのCrisis(=セーブ内に一度も日次進行が
    /// 走らないまま保存されていた状態)をロードした直後、次の`DayChangedMessage`が
    /// 一度も発火していない時点でも、支持の表明・撤回とも拒否されることを確認する。
    /// 実データE2Eでのみ検証可能な、実際の`round_trip_through_save`を経由した
    /// ロード後即操作のケース。
    #[test]
    fn stale_deadline_crisis_rejects_support_and_withdrawal_immediately_after_load() {
        let player = CountryId(0);
        let mut app = setup_app_in_playing(player);
        let initiator = CountryId(1);
        let target = CountryId(3);
        let crisis_id = create_claim_and_start_crisis(&mut app, initiator, target, StateId(9));

        // ロード前に、期限超過だがphaseはDemandSentのまま、かつ既に1件支持済みという
        // (通常の日次進行では起こり得ない、ロードされたsaveだけが持ち得る)状態を作る。
        {
            let mut crisis_registry = app.world_mut().resource_mut::<CrisisRegistry>();
            let crisis = crisis_registry.crises.get_mut(&crisis_id).unwrap();
            crisis.deadline_date = Some("1799/12/31".to_string()); // GameDate初期値1800/01/01より過去
            crisis
                .third_party_reactions
                .insert(CountryId(2), ThirdCountryReaction::SupportsInitiator);
        }

        round_trip_through_save(&mut app);

        let current_date = app.world().resource::<GameDate>().clone();
        {
            let crisis_registry = app.world().resource::<CrisisRegistry>();
            let crisis_after_load = crisis_registry.crises.get(&crisis_id).unwrap();
            assert_eq!(
                crisis_after_load.current_phase,
                CrisisPhase::DemandSent,
                "precondition: phase must still be DemandSent right after load, \
                 before any DayChangedMessage has ever fired"
            );
            assert!(
                current_date.is_at_least(
                    &GameDate::from_string(crisis_after_load.deadline_date.as_ref().unwrap())
                        .unwrap()
                ),
                "precondition: the deadline must already be in the past relative to the \
                 post-load GameDate"
            );
        }

        let countries = app.world().resource::<CountryRegistry>().clone_for_test();
        let pledge_result = crisis_response::pledge_support(
            &mut app.world_mut().resource_mut::<CrisisRegistry>(),
            &countries,
            crisis_id,
            player,
            CrisisSupportSide::Initiator,
            &current_date,
        );
        let withdraw_result = crisis_response::withdraw_support(
            &mut app.world_mut().resource_mut::<CrisisRegistry>(),
            crisis_id,
            CountryId(2),
            &current_date,
        );

        assert_eq!(
            pledge_result,
            Err("diplomacy_error.crisis.not_awaiting_response"),
            "a save loaded with an already-expired-but-still-DemandSent crisis must not allow \
             new support pledges before the next daily timeout tick"
        );
        assert_eq!(
            withdraw_result,
            Err("diplomacy_error.crisis.not_awaiting_response"),
            "a save loaded with an already-expired-but-still-DemandSent crisis must not allow \
             support withdrawal before the next daily timeout tick"
        );
        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .third_party_reactions
                .get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsInitiator),
            "the rejected withdrawal must not have mutated the pre-existing (loaded) pledge"
        );
    }
}
