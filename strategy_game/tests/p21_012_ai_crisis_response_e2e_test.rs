//! P21-012: AI国家によるCrisis最後通牒への受諾・拒否判断の接続の統合テスト。
//!
//! 前半は`GameTimePlugin`(実際の`DayChangedMessage`発行タイミング・Pause判定)だけを使った
//! 軽量Appで、`diplomacy::update::handle_daily_diplomacy`に追加したAI応答経路
//! (`diplomacy::ai::calculate_demand_acceptance`による受諾/拒否判定)を検証する。後半は
//! 実データ(`assets/data/*.ron`、7か国28州の本番マップ)を`AppPlugin`経由で読み込み、
//! player initiator→AI target、AI initiator→AI target→既存宣戦AI→`WarStarted`まで、
//! およびsave往復を一気通貫で検証する。

use bevy::prelude::*;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DailySimulationSet, GameDate, GamePaused, GameTimePlugin};
use strategy_game::common::{CountryId, DiplomaticCrisisId, StateId};
use strategy_game::country::{CountryData, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::ai::calculate_demand_acceptance;
use strategy_game::diplomacy::claims::ClaimRegistry;
use strategy_game::diplomacy::crisis::{
    CrisisPhase, CrisisRegistry, DiplomaticCrisis, WarGoal, WarGoalType,
};
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

fn country(id: usize, available_manpower: u64, capital_state_id: usize) -> CountryData {
    CountryData {
        id: CountryId(id),
        available_manpower,
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

#[allow(clippy::too_many_arguments)]
fn insert_demand_sent_crisis(
    app: &mut App,
    initiator: usize,
    target: usize,
    target_state: usize,
    start_date: &str,
    deadline_date: &str,
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
        })
}

fn tick_respecting_pause(app: &mut App, accumulator_delta: f64) {
    app.world_mut()
        .resource_mut::<GameDate>()
        .add_accumulator(accumulator_delta);
    app.update();
}

fn crisis_phase(app: &App, id: DiplomaticCrisisId) -> CrisisPhase {
    app.world()
        .resource::<CrisisRegistry>()
        .crises
        .get(&id)
        .unwrap()
        .current_phase
}

/// AI target has overwhelming initiator power (0→1), non-capital, low-value, high-integration
/// state demanded: `calculate_demand_acceptance` returns a positive score (accept), mirroring
/// the exact scenario already proven positive by `diplomacy::tests::test_demand_acceptance`.
fn setup_accept_scenario(app: &mut App) -> DiplomaticCrisisId {
    set_countries(
        app,
        vec![
            country(0, 100_000, 99), // initiator, strong, capital elsewhere
            country(1, 10_000, 5),   // target, weak, capital = StateId(5)
        ],
    );
    set_states(app, vec![land_state(10, 1, 50_000, 100.0)]);
    insert_demand_sent_crisis(app, 0, 1, 10, "1800/01/01", "1800/01/31")
}

/// AI target's capital is demanded: `calculate_demand_acceptance` always returns <= -100.0
/// (reject), mirroring `test_demand_acceptance`'s `score_capital <= -100.0` assertion.
fn setup_reject_scenario(app: &mut App) -> DiplomaticCrisisId {
    set_countries(app, vec![country(0, 100_000, 99), country(1, 10_000, 5)]);
    set_states(app, vec![land_state(5, 1, 50_000, 100.0)]);
    insert_demand_sent_crisis(app, 0, 1, 5, "1800/01/01", "1800/01/31")
}

fn unpause_and_advance(app: &mut App, days: u32) {
    app.world_mut().resource_mut::<GamePaused>().0 = false;
    for _ in 0..days {
        tick_respecting_pause(app, 1.0);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// AI判断 (要求テスト項目1-6)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn ai_target_accepts_when_calculate_demand_acceptance_is_positive() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_accept_scenario(&mut app);
    unpause_and_advance(&mut app, 1);

    assert_eq!(crisis_phase(&app, id), CrisisPhase::ResolvedPeacefully);
}

#[test]
fn ai_target_rejects_when_calculate_demand_acceptance_is_negative() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_reject_scenario(&mut app);
    unpause_and_advance(&mut app, 1);

    assert_eq!(crisis_phase(&app, id), CrisisPhase::Escalating);
}

/// Both scenarios differ only in which state is demanded; since they produce opposite
/// outcomes, the AI response cannot be a fixed accept-always/reject-always/coin-flip —
/// it is actually driven by `calculate_demand_acceptance`'s own varying output.
#[test]
fn ai_response_is_not_a_fixed_bypass_of_the_decision_function() {
    let mut accept_app = build_lightweight_diplomacy_app();
    let accept_id = setup_accept_scenario(&mut accept_app);
    unpause_and_advance(&mut accept_app, 1);

    let mut reject_app = build_lightweight_diplomacy_app();
    let reject_id = setup_reject_scenario(&mut reject_app);
    unpause_and_advance(&mut reject_app, 1);

    assert_ne!(
        crisis_phase(&accept_app, accept_id),
        crisis_phase(&reject_app, reject_id),
        "identical initiator power but different demanded states must yield different outcomes"
    );
}

#[test]
fn player_target_never_auto_responds() {
    let mut app = build_lightweight_diplomacy_app();
    app.world_mut().resource_mut::<PlayerCountry>().0 = Some(CountryId(1));
    let id = setup_accept_scenario(&mut app); // would accept if evaluated
    unpause_and_advance(&mut app, 29); // up to but not past the deadline

    assert_eq!(
        crisis_phase(&app, id),
        CrisisPhase::DemandSent,
        "a player-target crisis must never be auto-resolved by the AI response path"
    );
}

#[test]
fn ai_target_responds_regardless_of_whether_initiator_is_player_or_ai() {
    // initiator = player (CountryId(0)), target = AI (CountryId(1))
    let mut app_player_initiator = build_lightweight_diplomacy_app();
    app_player_initiator
        .world_mut()
        .resource_mut::<PlayerCountry>()
        .0 = Some(CountryId(0));
    let id1 = setup_accept_scenario(&mut app_player_initiator);
    unpause_and_advance(&mut app_player_initiator, 1);
    assert_eq!(
        crisis_phase(&app_player_initiator, id1),
        CrisisPhase::ResolvedPeacefully
    );

    // initiator = AI (CountryId(0)), target = AI (CountryId(1)), player is a bystander
    let mut app_ai_initiator = build_lightweight_diplomacy_app();
    app_ai_initiator
        .world_mut()
        .resource_mut::<PlayerCountry>()
        .0 = None;
    let id2 = setup_accept_scenario(&mut app_ai_initiator);
    unpause_and_advance(&mut app_ai_initiator, 1);
    assert_eq!(
        crisis_phase(&app_ai_initiator, id2),
        CrisisPhase::ResolvedPeacefully
    );
}

/// A third country (neither initiator nor target) being the player must not gate the
/// AI target's response — only the crisis's own `target` field matters.
#[test]
fn third_country_player_does_not_block_ai_target_response() {
    let mut app = build_lightweight_diplomacy_app();
    // player is CountryId(5), unrelated to the CountryId(0)/CountryId(1) crisis.
    app.world_mut().resource_mut::<PlayerCountry>().0 = Some(CountryId(5));
    let id = setup_accept_scenario(&mut app);
    unpause_and_advance(&mut app, 1);

    assert_eq!(crisis_phase(&app, id), CrisisPhase::ResolvedPeacefully);
}

// ─────────────────────────────────────────────────────────────────────────
// 日次処理 (要求テスト項目7-13)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn no_response_without_day_changed_message() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_accept_scenario(&mut app);
    // GamePaused defaults to true, and no accumulator was added: no DayChangedMessage fires.
    app.update();
    app.update();

    assert_eq!(crisis_phase(&app, id), CrisisPhase::DemandSent);
}

#[test]
fn no_response_while_paused() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_accept_scenario(&mut app);
    assert!(app.world().resource::<GamePaused>().0);
    tick_respecting_pause(&mut app, 1.0);
    tick_respecting_pause(&mut app, 1.0);

    assert_eq!(crisis_phase(&app, id), CrisisPhase::DemandSent);
}

#[test]
fn crisis_resolves_exactly_once_after_one_day_advances() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_accept_scenario(&mut app);
    unpause_and_advance(&mut app, 1);
    assert_eq!(crisis_phase(&app, id), CrisisPhase::ResolvedPeacefully);
}

#[test]
fn repeated_update_calls_within_the_same_day_do_not_double_process() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_accept_scenario(&mut app);
    app.world_mut().resource_mut::<GamePaused>().0 = false;
    tick_respecting_pause(&mut app, 1.0);
    assert_eq!(crisis_phase(&app, id), CrisisPhase::ResolvedPeacefully);

    // No new accumulator added: no further DayChangedMessage, must stay stable.
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(crisis_phase(&app, id), CrisisPhase::ResolvedPeacefully);
}

#[test]
fn ai_decision_is_used_before_the_deadline() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_accept_scenario(&mut app); // deadline is day 30
    unpause_and_advance(&mut app, 1); // day 1, far before the deadline

    assert_eq!(
        crisis_phase(&app, id),
        CrisisPhase::ResolvedPeacefully,
        "AI must be able to accept well before the deadline is reached"
    );
}

/// Requirement #3: once `current_date >= deadline_date`, the timeout path must win even if
/// the AI would have accepted — achieved by giving the crisis a deadline that is already due
/// on day 1 while using country data that would otherwise make the AI accept.
#[test]
fn timeout_rejection_takes_priority_over_ai_acceptance_on_the_deadline_day() {
    let mut app = build_lightweight_diplomacy_app();
    set_countries(
        &mut app,
        vec![country(0, 100_000, 99), country(1, 10_000, 5)],
    );
    set_states(&mut app, vec![land_state(10, 1, 50_000, 100.0)]);
    // deadline == start_date: already due on the very first tick.
    let id = insert_demand_sent_crisis(&mut app, 0, 1, 10, "1800/01/01", "1800/01/01");

    unpause_and_advance(&mut app, 1);

    assert_eq!(
        crisis_phase(&app, id),
        CrisisPhase::Escalating,
        "a crisis whose deadline is already due must be timed out, not AI-accepted, \
         even though the same country data would make the AI accept if evaluated"
    );
}

#[test]
fn terminal_phase_crises_are_not_touched_by_ai_response() {
    let mut app = build_lightweight_diplomacy_app();
    set_countries(
        &mut app,
        vec![country(0, 100_000, 99), country(1, 10_000, 5)],
    );
    set_states(&mut app, vec![land_state(10, 1, 50_000, 100.0)]);
    let id = insert_demand_sent_crisis(&mut app, 0, 1, 10, "1800/01/01", "1800/01/31");
    {
        let mut registry = app.world_mut().resource_mut::<CrisisRegistry>();
        registry.crises.get_mut(&id).unwrap().current_phase = CrisisPhase::Cancelled;
    }

    unpause_and_advance(&mut app, 1);

    assert_eq!(crisis_phase(&app, id), CrisisPhase::Cancelled);
}

// ─────────────────────────────────────────────────────────────────────────
// 受諾 (要求テスト項目14-18)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn ai_accept_transfers_state_ownership_to_initiator() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_accept_scenario(&mut app);
    unpause_and_advance(&mut app, 1);

    assert_eq!(crisis_phase(&app, id), CrisisPhase::ResolvedPeacefully);
    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(StateId(10))
            .unwrap()
            .owner_country_id,
        CountryId(0)
    );
}

#[test]
fn ai_accept_without_a_related_claim_does_not_crash_and_still_resolves() {
    // setup_accept_scenario never creates a ClaimRegistry entry (related_claim_id stays
    // None), matching how the crisis was constructed directly rather than via start_crisis.
    // accept_demand must treat "no related claim" as a no-op for claim consumption.
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_accept_scenario(&mut app);
    unpause_and_advance(&mut app, 1);

    assert_eq!(crisis_phase(&app, id), CrisisPhase::ResolvedPeacefully);
    assert!(app.world().resource::<ClaimRegistry>().claims.is_empty());
}

#[test]
fn ai_accept_produces_no_war_or_justification() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_accept_scenario(&mut app);
    unpause_and_advance(&mut app, 1);

    assert_eq!(crisis_phase(&app, id), CrisisPhase::ResolvedPeacefully);
    assert!(
        app.world()
            .resource::<WarJustificationRegistry>()
            .justifications
            .is_empty()
    );
}

#[test]
fn accepted_crisis_is_not_reprocessed_the_next_day() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_accept_scenario(&mut app);
    unpause_and_advance(&mut app, 1);
    assert_eq!(crisis_phase(&app, id), CrisisPhase::ResolvedPeacefully);

    let owner_after_day1 = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(10))
        .unwrap()
        .owner_country_id;

    unpause_and_advance(&mut app, 1);
    assert_eq!(crisis_phase(&app, id), CrisisPhase::ResolvedPeacefully);
    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(StateId(10))
            .unwrap()
            .owner_country_id,
        owner_after_day1,
        "a resolved crisis must not be re-accepted / re-transfer ownership on a later day"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// 拒否・宣戦 (要求テスト項目19-25、AI宣戦部分は real_map_e2e で検証)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn ai_reject_escalates_and_grants_exactly_one_completed_justification() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_reject_scenario(&mut app);
    unpause_and_advance(&mut app, 1);

    assert_eq!(crisis_phase(&app, id), CrisisPhase::Escalating);
    let justifications = &app
        .world()
        .resource::<WarJustificationRegistry>()
        .justifications;
    assert_eq!(justifications.len(), 1);
    let j = justifications.values().next().unwrap();
    assert!(j.is_ready);
    assert_eq!(j.initiator, CountryId(0));
    assert_eq!(j.target, CountryId(1));
}

#[test]
fn ai_reject_alone_does_not_start_a_war() {
    let mut app = build_lightweight_diplomacy_app();
    let id = setup_reject_scenario(&mut app);
    unpause_and_advance(&mut app, 1);

    assert_eq!(crisis_phase(&app, id), CrisisPhase::Escalating);
    assert_ne!(crisis_phase(&app, id), CrisisPhase::WarStarted);
    // The lightweight app has no WarRegistry/declare_war path at all — reaching this point
    // without panicking already proves the AI response system never calls declare_war itself.
}

// ─────────────────────────────────────────────────────────────────────────
// 決定論・複数件 (要求テスト項目26-30)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn multiple_crises_registered_in_different_orders_produce_the_same_outcomes() {
    fn run(insert_reject_first: bool) -> (CrisisPhase, CrisisPhase) {
        let mut app = build_lightweight_diplomacy_app();
        set_countries(
            &mut app,
            vec![
                country(0, 100_000, 99),
                country(1, 10_000, 5),
                country(2, 10_000, 6),
            ],
        );
        set_states(
            &mut app,
            vec![
                land_state(10, 1, 50_000, 100.0), // non-capital of country 1 -> accept
                land_state(6, 2, 50_000, 100.0),  // capital of country 2 -> reject
            ],
        );

        let (accept_id, reject_id) = if insert_reject_first {
            let reject_id =
                insert_demand_sent_crisis(&mut app, 0, 2, 6, "1800/01/01", "1800/01/31");
            let accept_id =
                insert_demand_sent_crisis(&mut app, 0, 1, 10, "1800/01/01", "1800/01/31");
            (accept_id, reject_id)
        } else {
            let accept_id =
                insert_demand_sent_crisis(&mut app, 0, 1, 10, "1800/01/01", "1800/01/31");
            let reject_id =
                insert_demand_sent_crisis(&mut app, 0, 2, 6, "1800/01/01", "1800/01/31");
            (accept_id, reject_id)
        };

        unpause_and_advance(&mut app, 1);
        (crisis_phase(&app, accept_id), crisis_phase(&app, reject_id))
    }

    let (accept_a, reject_a) = run(false);
    let (accept_b, reject_b) = run(true);

    assert_eq!(accept_a, CrisisPhase::ResolvedPeacefully);
    assert_eq!(reject_a, CrisisPhase::Escalating);
    assert_eq!(
        (accept_a, reject_a),
        (accept_b, reject_b),
        "insertion order must not affect either crisis's own outcome"
    );
}

#[test]
fn mixed_accept_and_reject_crises_are_each_processed_exactly_once() {
    let mut app = build_lightweight_diplomacy_app();
    set_countries(
        &mut app,
        vec![
            country(0, 100_000, 99),
            country(1, 10_000, 5),
            country(2, 10_000, 6),
        ],
    );
    set_states(
        &mut app,
        vec![
            land_state(10, 1, 50_000, 100.0),
            land_state(6, 2, 50_000, 100.0),
        ],
    );
    let accept_id = insert_demand_sent_crisis(&mut app, 0, 1, 10, "1800/01/01", "1800/01/31");
    let reject_id = insert_demand_sent_crisis(&mut app, 0, 2, 6, "1800/01/01", "1800/01/31");

    unpause_and_advance(&mut app, 3);

    assert_eq!(
        crisis_phase(&app, accept_id),
        CrisisPhase::ResolvedPeacefully
    );
    assert_eq!(crisis_phase(&app, reject_id), CrisisPhase::Escalating);
    assert_eq!(
        app.world()
            .resource::<WarJustificationRegistry>()
            .justifications
            .len(),
        1,
        "exactly one justification must exist (from the rejected crisis only)"
    );
}

#[test]
fn unrelated_and_terminal_crises_are_unaffected_by_other_crises_processing() {
    let mut app = build_lightweight_diplomacy_app();
    set_countries(
        &mut app,
        vec![country(0, 100_000, 99), country(1, 10_000, 5)],
    );
    set_states(&mut app, vec![land_state(10, 1, 50_000, 100.0)]);
    let active_id = insert_demand_sent_crisis(&mut app, 0, 1, 10, "1800/01/01", "1800/01/31");
    let already_resolved_id =
        insert_demand_sent_crisis(&mut app, 0, 1, 10, "1800/01/01", "1800/01/31");
    {
        let mut registry = app.world_mut().resource_mut::<CrisisRegistry>();
        registry
            .crises
            .get_mut(&already_resolved_id)
            .unwrap()
            .current_phase = CrisisPhase::WarStarted;
    }

    unpause_and_advance(&mut app, 1);

    assert_eq!(
        crisis_phase(&app, active_id),
        CrisisPhase::ResolvedPeacefully
    );
    assert_eq!(
        crisis_phase(&app, already_resolved_id),
        CrisisPhase::WarStarted
    );
}

/// A crisis whose target country no longer exists in `CountryRegistry` must be safely
/// skipped, not panic.
#[test]
fn dangling_target_country_reference_does_not_panic() {
    let mut app = build_lightweight_diplomacy_app();
    set_countries(&mut app, vec![country(0, 100_000, 99)]); // no CountryId(1) at all
    set_states(&mut app, vec![land_state(10, 1, 50_000, 100.0)]);
    let id = insert_demand_sent_crisis(&mut app, 0, 1, 10, "1800/01/01", "1800/01/31");

    unpause_and_advance(&mut app, 1); // must not panic

    assert_eq!(
        crisis_phase(&app, id),
        CrisisPhase::DemandSent,
        "a crisis with a dangling target country must be safely skipped, not resolved"
    );
}

/// A crisis whose demanded state no longer exists must also be safely skipped.
#[test]
fn dangling_target_state_reference_does_not_panic() {
    let mut app = build_lightweight_diplomacy_app();
    set_countries(
        &mut app,
        vec![country(0, 100_000, 99), country(1, 10_000, 5)],
    );
    set_states(&mut app, vec![]); // StateId(10) does not exist
    let id = insert_demand_sent_crisis(&mut app, 0, 1, 10, "1800/01/01", "1800/01/31");

    unpause_and_advance(&mut app, 1); // must not panic

    assert_eq!(crisis_phase(&app, id), CrisisPhase::DemandSent);
}

#[test]
fn one_thousand_demand_sent_crises_complete_without_panicking() {
    let mut app = build_lightweight_diplomacy_app();
    let mut countries = Vec::new();
    for i in 0..10 {
        countries.push(country(i, 50_000, 900 + i));
    }
    set_countries(&mut app, countries);
    let mut states = Vec::new();
    for i in 0..1000 {
        let owner = 1 + (i % 9); // countries 1..=9 own states; country 0 is the initiator
        states.push(land_state(i, owner, 10_000, 80.0));
    }
    set_states(&mut app, states);

    let mut ids = Vec::new();
    for i in 0..1000 {
        let owner = 1 + (i % 9);
        ids.push(insert_demand_sent_crisis(
            &mut app,
            0,
            owner,
            i,
            "1800/01/01",
            "1800/01/31",
        ));
    }

    unpause_and_advance(&mut app, 1); // must complete without panicking or hanging

    let resolved_or_escalated = ids
        .iter()
        .filter(|id| !matches!(crisis_phase(&app, **id), CrisisPhase::DemandSent))
        .count();
    assert_eq!(
        resolved_or_escalated, 1000,
        "every eligible DemandSent crisis must have been evaluated exactly once"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// 実データE2E(実7か国28州マップ) — 要求テスト項目22-25, 31-35
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
                .expect("real-map save with P21-012 AI-response crisis state must validate")
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

    /// 要求テスト項目34: 実データでplayer initiator→AI target→受諾または拒否まで進行する。
    /// AIの応答方向は実データ依存のため予測せず、`calculate_demand_acceptance`自身を
    /// テスト内でも呼び出して期待結果を計算し、実際の結果と一致することを確認する。
    #[test]
    fn player_initiator_ai_target_resolves_according_to_the_decision_function() {
        let player = CountryId(0);
        let mut app = setup_app_in_playing(player);
        let target = CountryId(1);
        let target_state = StateId(4); // "Forest Research Zone", CountryId(1)-owned, non-capital

        let crisis_id = create_claim_and_start_crisis(&mut app, player, target, target_state);

        let expected_accept = {
            let world = app.world();
            let crisis_registry = world.resource::<CrisisRegistry>();
            let crisis = crisis_registry.crises.get(&crisis_id).unwrap();
            let country_registry = world.resource::<CountryRegistry>();
            let state_registry = world.resource::<StateRegistry>();
            let diplomacy_registry = world.resource::<DiplomacyRegistry>();
            let target_country = country_registry.get(target).unwrap();
            let initiator_country = country_registry.get(player).unwrap();
            let state = state_registry.get(target_state).unwrap();
            let relation = diplomacy_registry.get_or_default(player, target);
            calculate_demand_acceptance(
                crisis,
                target_country,
                initiator_country,
                &[state],
                &relation,
                0.0,
                0.0,
            ) > 0.0
        };

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
        assert_eq!(
            phase,
            if expected_accept {
                CrisisPhase::ResolvedPeacefully
            } else {
                CrisisPhase::Escalating
            },
            "the real-map outcome must match calculate_demand_acceptance's own verdict"
        );

        round_trip_through_save(&mut app);
        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            phase,
            "AI response outcome must survive a save round trip"
        );
    }

    /// 要求テスト項目22-25, 35: 実データでAI initiator→AI target→拒否→既存宣戦AI→
    /// `WarStarted`まで接続され、dangling justification参照が残らないことを確認する。
    #[test]
    fn ai_initiator_ai_target_rejection_flows_through_existing_war_ai_to_war_started() {
        // player is a bystander country not involved in this crisis. CountryId(1)/(3) have
        // no pre-existing treaty in assets/data/diplomacy.ron (unlike 1-2's NonAggressionPact
        // or 2-4's Alliance), so nothing blocks declare_war's own treaty check.
        let player = CountryId(0);
        let mut app = setup_app_in_playing(player);
        let initiator = CountryId(1);
        let target = CountryId(3);
        let target_state = StateId(8); // capital of CountryId(3) -> guaranteed reject

        let crisis_id = create_claim_and_start_crisis(&mut app, initiator, target, target_state);
        assert_eq!(
            app.world()
                .resource::<StateRegistry>()
                .get(target_state)
                .map(|s| s.owner_country_id),
            Some(target)
        );
        assert_eq!(
            app.world()
                .resource::<CountryRegistry>()
                .get(target)
                .unwrap()
                .capital_state_id,
            target_state,
            "test setup must actually target the real capital to force a reject verdict"
        );

        // Give the AI a few days: day 1 rejects (Diplomacy set), and the same or a following
        // day the existing war-declaration AI (CountryAi set, runs after Diplomacy the same
        // tick) picks up the now-ready justification and declares war.
        let mut war_started = false;
        for _ in 0..10 {
            advance_one_day(&mut app);
            let phase = app
                .world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase;
            if phase == CrisisPhase::WarStarted {
                war_started = true;
                break;
            }
        }

        assert!(
            war_started,
            "AI initiator must eventually declare war via the existing war-declaration AI \
             after the AI target rejects the demand"
        );

        let crisis = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .clone();
        assert_eq!(crisis.current_phase, CrisisPhase::WarStarted);
        assert!(
            crisis.related_war_id.is_some(),
            "related_war_id must be set once WarStarted"
        );
        assert_eq!(
            crisis.related_justification_id, None,
            "related_justification_id must be cleared once the justification is consumed \
             by the real declare_war call (otherwise it would dangling-reference)"
        );

        round_trip_through_save(&mut app);
        let crisis_after = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .clone();
        assert_eq!(crisis_after.current_phase, CrisisPhase::WarStarted);
        assert_eq!(crisis_after.related_war_id, crisis.related_war_id);
        assert!(
            app.world()
                .resource::<WarRegistry>()
                .wars
                .contains_key(&crisis.related_war_id.unwrap())
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
