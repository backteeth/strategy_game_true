//! P21-018: AI国家による月次Claim生成の統合テスト。
//!
//! 実データ(`assets/data/*.ron`、6か国28州の本番マップ)を`AppPlugin`経由で読み込み、
//! AI国家が国境を接する外国州へClaimを生成し、同月内ではCrisis化されないこと、
//! 次回以降の月変更でP21-017が拾ってCrisis→AI応答/拒否→多国間War(source_crisis_id
//! 維持)まで進むこと、Save/Load往復・決定論的選択までを一気通貫で検証する。

use bevy::prelude::*;
use strategy_game::app::game_state::GameState;
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::PlayerCountry;
use strategy_game::diplomacy::claims::{ClaimRegistry, ClaimSource};
use strategy_game::diplomacy::crisis::{CrisisPhase, CrisisRegistry};
use strategy_game::diplomacy::data::{DiplomacyRegistry, DiplomaticPairKey, DiplomaticRelation};
use strategy_game::state::data::StateRegistry;
use strategy_game::war::data::WarRegistry;

use bevy::app::ScheduleRunnerPlugin;
use strategy_game::app::AppPlugin;
use strategy_game::building::BuildingPlugin;
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
            .expect("real-map save with P21-018 AI-generated claim state must validate")
    };
    assert_eq!(
        apply_validated_save(app.world_mut(), validated),
        ApplyLoadOutcome::Success
    );
}

/// 実マップ上で、陸上国境を接する(claimant, target, target_state)の組を1つ選ぶ。
/// `allow_player_target`が`false`の場合、targetがplayerである組は除外する
/// (claimantが常にplayer自身になることはない — §4項目1と同じ制約)。
fn pick_adjacent_border_pair(
    app: &App,
    player: CountryId,
    allow_player_target: bool,
) -> (CountryId, CountryId, StateId) {
    let state_registry = app.world().resource::<StateRegistry>();
    for state in &state_registry.states {
        if state.is_sea || state.owner_country_id == player {
            continue;
        }
        for &nid in &state.neighbors {
            let Some(neighbor) = state_registry.get(nid) else {
                continue;
            };
            if neighbor.is_sea || neighbor.owner_country_id == state.owner_country_id {
                continue;
            }
            if neighbor.owner_country_id == player && !allow_player_target {
                continue;
            }
            return (
                state.owner_country_id,
                neighbor.owner_country_id,
                neighbor.id,
            );
        }
    }
    panic!("real map must have at least one land border satisfying the target constraint");
}

/// 同じ組で、targetがplayer固定の隣接ペアを選ぶ。
fn pick_adjacent_border_pair_targeting_player(
    app: &App,
    player: CountryId,
) -> (CountryId, StateId) {
    let state_registry = app.world().resource::<StateRegistry>();
    for state in &state_registry.states {
        if state.is_sea || state.owner_country_id != player {
            continue;
        }
        for &nid in &state.neighbors {
            let Some(neighbor) = state_registry.get(nid) else {
                continue;
            };
            if neighbor.is_sea || neighbor.owner_country_id == player {
                continue;
            }
            return (neighbor.owner_country_id, state.id);
        }
    }
    panic!("real map must have at least one AI country bordering the player");
}

fn set_hostile_opinion(app: &mut App, a: CountryId, b: CountryId) {
    let key = DiplomaticPairKey::new(a, b).unwrap();
    app.world_mut()
        .resource_mut::<DiplomacyRegistry>()
        .relations
        .insert(
            key,
            DiplomaticRelation {
                opinion: -50.0,
                ..Default::default()
            },
        );
}

/// claimantの人口を圧倒的に増やし、targetの人口を圧倒的に減らしてから
/// `CountryPowerRegistry`を即座に再構築する。実マップの自然な国力バランスに
/// 依存せず、power ratio(claimant/target) >= 0.75を決定論的に満たすための
/// テスト専用の強制操作(`p21_017`のE2Eテストが`available_manpower`を直接
/// 上書きしたのと同じ考え方)。
fn force_power_advantage(app: &mut App, claimant: CountryId, target: CountryId) {
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut() {
            if state.owner_country_id == claimant {
                state.population = state.population.max(10_000_000);
            }
            if state.owner_country_id == target {
                state.population = state.population.min(100);
            }
        }
    }
    strategy_game::country::power::rebuild_country_power_registry_from_world(app.world_mut());
}

/// claimantによる`ClaimRegistry`へのClaim発生まで日を進める。
fn advance_until_claim_created_by(app: &mut App, claimant: CountryId, max_days: u32) {
    for _ in 0..max_days {
        advance_one_day(app);
        if app
            .world()
            .resource::<ClaimRegistry>()
            .claims
            .values()
            .any(|c| c.claimant_country == claimant)
        {
            return;
        }
    }
    panic!("AI({claimant:?}) did not generate a claim within {max_days} days");
}

fn advance_until_crisis_started_by(app: &mut App, claimant: CountryId, max_days: u32) {
    for _ in 0..max_days {
        advance_one_day(app);
        if app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .values()
            .any(|c| c.initiator == claimant)
        {
            return;
        }
    }
    panic!("AI({claimant:?}) did not start a crisis within {max_days} days");
}

// ─────────────────────────────────────────────────────────────────────
// 要求テスト65: 実データでAIが隣接外国州へClaimを生成する。
// 要求テスト74: 同一入力から同じ対象州を選ぶ(決定論性)。
// ─────────────────────────────────────────────────────────────────────

#[test]
fn ai_generates_claim_on_adjacent_foreign_land_state() {
    let bystander = CountryId(99); // 実マップに存在しないIDをplayerに使い、全国家をAI扱いにする
    let mut app = setup_app_in_playing(bystander);
    let (claimant, target, target_state) = pick_adjacent_border_pair(&app, bystander, true);
    force_power_advantage(&mut app, claimant, target);
    set_hostile_opinion(&mut app, claimant, target);

    advance_until_claim_created_by(&mut app, claimant, 40);

    let claim = app
        .world()
        .resource::<ClaimRegistry>()
        .claims
        .values()
        .find(|c| c.claimant_country == claimant)
        .unwrap()
        .clone();
    assert_eq!(claim.target_state, target_state);
    assert_eq!(claim.source, ClaimSource::AiExpansion);
    assert_eq!(
        claim.status,
        strategy_game::diplomacy::claims::ClaimStatus::Active
    );

    // 決定論性(要求74): 同一初期条件から作り直した2つ目のAppでも同じ対象州が選ばれる。
    let mut app2 = setup_app_in_playing(bystander);
    let (claimant2, target2, target_state2) = pick_adjacent_border_pair(&app2, bystander, true);
    assert_eq!(
        (claimant2, target2, target_state2),
        (claimant, target, target_state)
    );
    force_power_advantage(&mut app2, claimant2, target2);
    set_hostile_opinion(&mut app2, claimant2, target2);
    advance_until_claim_created_by(&mut app2, claimant2, 40);
    let claim2 = app2
        .world()
        .resource::<ClaimRegistry>()
        .claims
        .values()
        .find(|c| c.claimant_country == claimant2)
        .unwrap()
        .clone();
    assert_eq!(claim2.target_state, claim.target_state);
}

// ─────────────────────────────────────────────────────────────────────
// 要求テスト61,62,66: 同月ではCrisis化されず、次回以降の月変更でP21-017が拾う。
// ─────────────────────────────────────────────────────────────────────

#[test]
fn claim_generated_this_month_is_not_immediately_a_crisis_but_becomes_one_next_month() {
    let bystander = CountryId(99);
    let mut app = setup_app_in_playing(bystander);
    let (claimant, target, _target_state) = pick_adjacent_border_pair(&app, bystander, true);
    force_power_advantage(&mut app, claimant, target);
    set_hostile_opinion(&mut app, claimant, target);

    advance_until_claim_created_by(&mut app, claimant, 40);

    assert!(
        !app.world()
            .resource::<CrisisRegistry>()
            .crises
            .values()
            .any(|c| c.initiator == claimant),
        "a claim generated this month must not become a crisis in the same month-changed frame"
    );

    // 次回以降の月変更でP21-017が通常のClaimとして拾う(cooldown対象にならないよう、
    // 発生月から十分先までではなく、単に「次の月変更」まで進めるだけでよい —
    // P21-017自身にcooldownは存在しない)。
    advance_until_crisis_started_by(&mut app, claimant, 40);

    let crisis = app
        .world()
        .resource::<CrisisRegistry>()
        .crises
        .values()
        .find(|c| c.initiator == claimant)
        .unwrap()
        .clone();
    assert_eq!(crisis.target, target);
    assert_eq!(crisis.current_phase, CrisisPhase::DemandSent);
}

// ─────────────────────────────────────────────────────────────────────
// 要求テスト67,69,70: AI→AI Crisisが応答へ進み、拒否後は多国間War
// (source_crisis_id維持)へ接続する。
// ─────────────────────────────────────────────────────────────────────

#[test]
fn rejected_ai_generated_claim_crisis_leads_to_war_with_source_crisis_id_preserved() {
    let bystander = CountryId(99);
    let mut app = setup_app_in_playing(bystander);
    let (claimant, target, _target_state) = pick_adjacent_border_pair(&app, bystander, true);
    force_power_advantage(&mut app, claimant, target);
    set_hostile_opinion(&mut app, claimant, target);

    advance_until_claim_created_by(&mut app, claimant, 40);
    advance_until_crisis_started_by(&mut app, claimant, 40);

    let crisis_id = app
        .world()
        .resource::<CrisisRegistry>()
        .crises
        .values()
        .find(|c| c.initiator == claimant)
        .unwrap()
        .id;

    // targetの国力を圧倒的劣位のまま保ち、opinionも敵対的なままなので
    // P21-012の受諾スコアは拒否側へ傾く。デッドライン(30日)+宣戦までの猶予を
    // 十分に超えて日を進め、Warが成立するまで待つ。
    let mut war_id = None;
    for _ in 0..90 {
        advance_one_day(&mut app);
        if let Some(w) = app
            .world()
            .resource::<WarRegistry>()
            .wars
            .values()
            .find(|w| w.attackers.contains(&claimant) && w.defenders.contains(&target))
        {
            war_id = Some(w.id);
            break;
        }
    }
    let war_id = war_id.expect("rejected AI-generated claim crisis must eventually lead to war");

    let war = app
        .world()
        .resource::<WarRegistry>()
        .wars
        .get(&war_id)
        .unwrap()
        .clone();
    assert_eq!(
        war.source_crisis_id,
        Some(crisis_id),
        "the war must preserve source_crisis_id back to the AI-generated-claim crisis (P21-016 invariant)"
    );
}

// ─────────────────────────────────────────────────────────────────────
// 要求テスト68,83: AI→player Claimからplayer対象Crisisが生まれ、既存の
// プレイヤーClaim操作(受諾/拒否UI経路)を壊さない。
// ─────────────────────────────────────────────────────────────────────

#[test]
fn ai_generates_claim_against_player_state_and_it_becomes_a_crisis_the_player_can_see() {
    // 実マップの国家の1つをplayerとして使う(隣接するAI国家が存在する必要がある)。
    let player = CountryId(0);
    let mut app = setup_app_in_playing(player);
    let (claimant, target_state) = pick_adjacent_border_pair_targeting_player(&app, player);
    force_power_advantage(&mut app, claimant, player);
    set_hostile_opinion(&mut app, claimant, player);

    advance_until_claim_created_by(&mut app, claimant, 40);
    let claim = app
        .world()
        .resource::<ClaimRegistry>()
        .claims
        .values()
        .find(|c| c.claimant_country == claimant)
        .unwrap()
        .clone();
    assert_eq!(claim.target_state, target_state);

    advance_until_crisis_started_by(&mut app, claimant, 40);
    let crisis = app
        .world()
        .resource::<CrisisRegistry>()
        .crises
        .values()
        .find(|c| c.initiator == claimant)
        .unwrap()
        .clone();
    assert_eq!(crisis.target, player);
    assert_eq!(
        crisis.current_phase,
        CrisisPhase::DemandSent,
        "the existing generic crisis list/accept/reject UI (diplomacy_panel) already handles \
         any initiator/target pair since P21-013, so a claim targeting the player must produce \
         a normal DemandSent crisis the player can answer through it"
    );
}

// ─────────────────────────────────────────────────────────────────────
// 要求テスト71,72,73: AI生成ClaimのSave/Load往復、重複なし、Load後の次月Crisis開始。
// ─────────────────────────────────────────────────────────────────────

#[test]
fn ai_generated_claim_round_trips_through_save_load_without_duplication_and_next_month_still_works()
{
    // Save検証はplayer_countryが実在するCountryIdであることを要求するため、
    // (他のテストと違い)実マップ上の国家IDを傍観者として使う。
    let bystander = CountryId(2);
    let mut app = setup_app_in_playing(bystander);
    let (claimant, target, target_state) = pick_adjacent_border_pair(&app, bystander, true);
    force_power_advantage(&mut app, claimant, target);
    set_hostile_opinion(&mut app, claimant, target);

    advance_until_claim_created_by(&mut app, claimant, 40);
    let before = app
        .world()
        .resource::<ClaimRegistry>()
        .claims
        .values()
        .find(|c| c.claimant_country == claimant)
        .unwrap()
        .clone();

    round_trip_through_save(&mut app);

    let claims_after_load: Vec<_> = app
        .world()
        .resource::<ClaimRegistry>()
        .claims
        .values()
        .filter(|c| c.claimant_country == claimant)
        .cloned()
        .collect();
    assert_eq!(
        claims_after_load.len(),
        1,
        "load must not duplicate the AI-generated claim"
    );
    let restored = &claims_after_load[0];
    assert_eq!(restored.id, before.id);
    assert_eq!(restored.claimant_country, before.claimant_country);
    assert_eq!(restored.target_state, before.target_state);
    assert_eq!(restored.strength, before.strength);
    assert_eq!(restored.created_date, before.created_date);
    assert_eq!(restored.is_permanent, before.is_permanent);
    assert_eq!(restored.source, before.source);
    assert_eq!(restored.status, before.status);

    // Load直後(MonthChangedMessage無し)だけでは何も評価されない: 1フレーム進めても
    // 新しいClaimは増えない。
    app.update();
    assert_eq!(
        app.world()
            .resource::<ClaimRegistry>()
            .claims
            .values()
            .filter(|c| c.claimant_country == claimant)
            .count(),
        1
    );

    // 次の通常の月変更からP21-017が既存Claimとして評価を再開できる。
    advance_until_crisis_started_by(&mut app, claimant, 40);
    let crisis = app
        .world()
        .resource::<CrisisRegistry>()
        .crises
        .values()
        .find(|c| c.initiator == claimant)
        .unwrap();
    assert_eq!(crisis.target, target);
    assert_eq!(
        crisis
            .war_goals
            .first()
            .and_then(|g| g.target_states.first()),
        Some(&target_state)
    );
}
