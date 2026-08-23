//! P21-019: Crisis由来War講和のClaim消費・領土移転の統合テスト。
//!
//! 実データ(`assets/data/*.ron`、6か国28州の本番マップ)を`AppPlugin`経由で読み込み、
//! P21-018のClaim生成→P21-017のCrisis開始→拒否→多国間War開始という実際の
//! ゲームプレイ経路(`tests/p21_018_ai_claim_generation_e2e_test.rs`と同じ手法)で
//! Crisis由来Warを作った上で、実際の`execute_peace_settlement`(講和UI・自動降伏
//! 講和と全く同じ中央API)による講和が、元Claimの消費・target_stateの移転・
//! Warの終結を正しく行うこと、Save/Load往復後も結果が維持されること、
//! 次月以降は消費済みClaimがcooldown対象のまま再利用されないことを検証する。

use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::game_state::GameState;
use strategy_game::common::{CountryId, StateId, WarId};
use strategy_game::country::{CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::claims::{ClaimRegistry, ClaimStatus};
use strategy_game::diplomacy::crisis::{CrisisRegistry, ThirdCountryReaction};
use strategy_game::diplomacy::data::{
    ActiveTreaty, DiplomacyRegistry, DiplomaticPairKey, DiplomaticRelation, TreatyType,
};
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::state::data::StateRegistry;
use strategy_game::war::data::{WarRegistry, WarStatus};
use strategy_game::war::frontline::FrontlineRegistry;
use strategy_game::war::peace::{PeaceTerm, execute_peace_settlement};

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
            .expect("real-map save with P21-019 post-peace state must validate")
    };
    assert_eq!(
        apply_validated_save(app.world_mut(), validated),
        ApplyLoadOutcome::Success
    );
}

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

/// `pick_adjacent_border_pair`と同じだが、対象州が対象国の首都(capital_state_id)
/// である組は除外する(§26で報告する既存の`capital_state_id`未追従ギャップを
/// このテストの検証対象から意図的に外すため)。
fn pick_adjacent_border_pair_excluding_capital(
    app: &App,
    player: CountryId,
) -> (CountryId, CountryId, StateId) {
    let state_registry = app.world().resource::<StateRegistry>();
    let country_registry = app
        .world()
        .resource::<strategy_game::country::CountryRegistry>();
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
            if neighbor.owner_country_id == player {
                continue;
            }
            let is_capital = country_registry
                .get(neighbor.owner_country_id)
                .is_some_and(|c| c.capital_state_id == neighbor.id);
            if is_capital {
                continue;
            }
            return (
                state.owner_country_id,
                neighbor.owner_country_id,
                neighbor.id,
            );
        }
    }
    panic!(
        "real map must have at least one non-capital land border satisfying the target constraint"
    );
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

/// P21-018と全く同じ手法(隣接ペア選択・国力/関係値の強制)でAI Claim生成→
/// P21-017 Crisis開始→デッドライン経過による拒否→AI正当化→宣戦布告までを
/// 実際のゲームプレイ経路で進め、実際に成立したWarのIDを返す
/// (要求53-58: Claim生成・Crisis開始・応答/拒否・多国間War開始まで)。
fn build_crisis_derived_war_via_real_gameplay(
    app: &mut App,
    claimant: CountryId,
    target: CountryId,
) -> WarId {
    force_power_advantage(app, claimant, target);
    set_hostile_opinion(app, claimant, target);

    advance_until_claim_created_by(app, claimant, 40);
    advance_until_crisis_started_by(app, claimant, 40);

    let mut war_id = None;
    for _ in 0..90 {
        advance_one_day(app);
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
    war_id.expect("the AI-generated claim/crisis pipeline must eventually lead to a declared war")
}

/// `execute_peace_settlement`を実際のApp World上の全レジストリを使って呼ぶ
/// (`SystemState`経由、`peace_panel.rs`/`war/update.rs`と全く同じ中央APIを呼ぶ)。
#[allow(clippy::type_complexity)]
fn settle_peace(
    app: &mut App,
    war_id: WarId,
    term: PeaceTerm,
    current_date: &str,
) -> Result<(), &'static str> {
    let mut system_state: SystemState<(
        ResMut<StateRegistry>,
        ResMut<WarRegistry>,
        ResMut<MilitaryRegistry>,
        ResMut<BattleRegistry>,
        ResMut<DiplomacyRegistry>,
        ResMut<FrontlineRegistry>,
        ResMut<CrisisRegistry>,
        ResMut<ClaimRegistry>,
        ResMut<CountryRegistry>,
        ResMut<strategy_game::military::army::ArmyRegistry>,
        ResMut<strategy_game::war::justification::WarJustificationRegistry>,
        ResMut<strategy_game::country::power::CountryPowerRegistry>,
    )> = SystemState::new(app.world_mut());
    let (
        mut state_reg,
        mut war_reg,
        mut mil_reg,
        mut battle_reg,
        mut dip_reg,
        mut fl_reg,
        mut crisis_reg,
        mut claim_reg,
        mut country_reg,
        mut army_reg,
        mut justification_reg,
        mut power_reg,
    ) = system_state
        .get_mut(app.world_mut())
        .expect("all P21-019 peace settlement resources must be present in the real App world");

    execute_peace_settlement(
        war_id,
        term,
        "P21-019 E2E Test Settlement",
        current_date,
        &mut state_reg,
        &mut war_reg,
        &mut mil_reg,
        &mut battle_reg,
        &mut dip_reg,
        &mut fl_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
}

// ─────────────────────────────────────────────────────────────────────
// 要求テスト53-63,65: Claim生成→Crisis→拒否→War→講和→領土移転→Claim消費→
// War終結→次月以降は消費済みClaimが再利用されない、までの完全なE2E接続。
// ─────────────────────────────────────────────────────────────────────

#[test]
fn crisis_derived_war_peace_settlement_transfers_territory_consumes_claim_and_ends_the_war() {
    let bystander = CountryId(99);
    let mut app = setup_app_in_playing(bystander);
    let (claimant, target, target_state) = pick_adjacent_border_pair(&app, bystander, true);

    let war_id = build_crisis_derived_war_via_real_gameplay(&mut app, claimant, target);

    let claim_id = app
        .world()
        .resource::<ClaimRegistry>()
        .claims
        .values()
        .find(|c| c.claimant_country == claimant)
        .expect("the claim that led to this war must still exist")
        .id;
    assert_eq!(
        app.world()
            .resource::<ClaimRegistry>()
            .claims
            .get(&claim_id)
            .unwrap()
            .status,
        ClaimStatus::Active,
        "the claim must still be Active immediately before the peace settlement"
    );

    let current_date = app
        .world()
        .resource::<strategy_game::app::time::GameDate>()
        .display();
    let result = settle_peace(
        &mut app,
        war_id,
        PeaceTerm::CedeWarGoalRegion,
        &current_date,
    );
    assert!(result.is_ok(), "peace settlement must succeed: {result:?}");

    // 要求61: target_stateがclaimantへ移転する。
    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(target_state)
            .unwrap()
            .owner_country_id,
        claimant,
        "the crisis's target_state must transfer to the claimant after CedeWarGoalRegion peace"
    );

    // 要求62: 元Claimが消費される。
    assert_eq!(
        app.world()
            .resource::<ClaimRegistry>()
            .claims
            .get(&claim_id)
            .unwrap()
            .status,
        ClaimStatus::Consumed
    );

    // 要求63: Warが終結する。
    assert_eq!(
        app.world()
            .resource::<WarRegistry>()
            .wars
            .get(&war_id)
            .unwrap()
            .status,
        WarStatus::AttackerVictory
    );

    // 要求65(部分): 講和成立と同じ月のうちは、消費済みClaimの新規生成対象には
    // ならない(cooldown 12か月未満)。次の月変更でclaimantが同じ対象へ即座に
    // 別Claimを再生成しないことを、1回分の月変更を進めて確認する。
    let claims_for_claimant_before = app
        .world()
        .resource::<ClaimRegistry>()
        .claims
        .values()
        .filter(|c| c.claimant_country == claimant)
        .count();
    for _ in 0..35 {
        advance_one_day(&mut app);
    }
    let claims_for_claimant_after = app
        .world()
        .resource::<ClaimRegistry>()
        .claims
        .values()
        .filter(|c| c.claimant_country == claimant)
        .count();
    assert_eq!(
        claims_for_claimant_before, claims_for_claimant_after,
        "cooldown (12 months) must still block regeneration shortly after the settlement"
    );
}

// ─────────────────────────────────────────────────────────────────────
// 要求テスト64: Save→Load後も講和結果(Claim消費・領土移転・War終結)が維持される。
// ─────────────────────────────────────────────────────────────────────

#[test]
fn peace_settlement_result_survives_save_load_round_trip() {
    // Save検証はplayer_countryが実在するCountryIdであることを要求するため、
    // (P21-018のE2Eテストと同様)実マップ上の国家IDを傍観者として使う。
    let bystander = CountryId(2);
    let mut app = setup_app_in_playing(bystander);
    // 既存の仕様差(§26参照): `transfer_region_ownership`はtarget_stateが
    // 対象国の首都でも無条件に移転するが、`capital_state_id`は追従して
    // 再割当されないため、対象州がたまたま相手国の首都だとSave検証が
    // `OwnershipMismatch`を出す(P21-019が生んだ問題ではなく、通常Warの
    // 領土割譲講和でも同じ手順で再現し得る既存ギャップ)。このテストは
    // P21-019自身のSave/Load往復を検証する目的のため、対象州が相手国の
    // 首都ではない隣接ペアを選ぶことでこの既存ギャップを回避する。
    let (claimant, target, target_state) =
        pick_adjacent_border_pair_excluding_capital(&app, bystander);

    let war_id = build_crisis_derived_war_via_real_gameplay(&mut app, claimant, target);
    let claim_id = app
        .world()
        .resource::<ClaimRegistry>()
        .claims
        .values()
        .find(|c| c.claimant_country == claimant)
        .unwrap()
        .id;

    let current_date = app
        .world()
        .resource::<strategy_game::app::time::GameDate>()
        .display();
    settle_peace(
        &mut app,
        war_id,
        PeaceTerm::CedeWarGoalRegion,
        &current_date,
    )
    .unwrap();

    round_trip_through_save(&mut app);

    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(target_state)
            .unwrap()
            .owner_country_id,
        claimant,
        "territory transfer must survive save/load"
    );
    assert_eq!(
        app.world()
            .resource::<ClaimRegistry>()
            .claims
            .get(&claim_id)
            .unwrap()
            .status,
        ClaimStatus::Consumed,
        "claim consumption must survive save/load"
    );
    assert_eq!(
        app.world()
            .resource::<WarRegistry>()
            .wars
            .get(&war_id)
            .unwrap()
            .status,
        WarStatus::AttackerVictory,
        "war end status must survive save/load"
    );
    assert!(
        app.world()
            .resource::<WarRegistry>()
            .wars
            .get(&war_id)
            .unwrap()
            .source_crisis_id
            .is_some(),
        "source_crisis_id lineage must survive save/load"
    );
}

// ─────────────────────────────────────────────────────────────────────
// ユーザー指摘対応(2026-08-21): P21-018はP21-018専用の首都禁止ルールを
// 追加しないと明示しており、首都州も実際にClaim候補として許可されている。
// その正規経路(Claim→Crisis→War→CedeWarGoalRegion講和)で対象州が
// たまたま相手国の首都だった場合でも、Save検証まで含めて壊れないことを
// 実データで確認する(単にE2Eで首都を除外して回避するだけでは不十分、
// というユーザー指摘への対応)。
// ─────────────────────────────────────────────────────────────────────

/// `pick_adjacent_border_pair`と同じだが、対象州が対象国の首都
/// (capital_state_id)である組を明示的に選ぶ(通常のE2Eとは逆に、
/// 首都ケースを意図的に狙うため)。
fn pick_adjacent_border_pair_targeting_a_capital(
    app: &App,
    player: CountryId,
) -> (CountryId, CountryId, StateId) {
    let state_registry = app.world().resource::<StateRegistry>();
    let country_registry = app.world().resource::<CountryRegistry>();
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
            if neighbor.owner_country_id == player {
                continue;
            }
            let is_capital = country_registry
                .get(neighbor.owner_country_id)
                .is_some_and(|c| c.capital_state_id == neighbor.id);
            if !is_capital {
                continue;
            }
            return (
                state.owner_country_id,
                neighbor.owner_country_id,
                neighbor.id,
            );
        }
    }
    panic!(
        "real map must have at least one land border where the neighbor is the neighbor-owner's capital"
    );
}

/// 首都Claim → Crisis → War → 割譲 → Save validation → Save/Loadの完全な
/// パイプライン。対象州が相手国の首都であっても講和が成功し、その国の
/// `capital_state_id`が残存する陸州(決定論的に最小StateId)へ再割当され、
/// 割譲後のSaveが検証を通過し、Save/Load後も再割当された首都・領土移転・
/// Claim消費のいずれも維持されることを確認する。
#[test]
fn cede_capital_claim_generates_crisis_war_cession_and_survives_save_validation_and_load() {
    let bystander = CountryId(2);
    let mut app = setup_app_in_playing(bystander);
    let (claimant, target, target_state) =
        pick_adjacent_border_pair_targeting_a_capital(&app, bystander);

    let war_id = build_crisis_derived_war_via_real_gameplay(&mut app, claimant, target);

    let claim_id = app
        .world()
        .resource::<ClaimRegistry>()
        .claims
        .values()
        .find(|c| c.claimant_country == claimant)
        .expect("the claim that led to this war must still exist")
        .id;

    let current_date = app
        .world()
        .resource::<strategy_game::app::time::GameDate>()
        .display();
    let result = settle_peace(
        &mut app,
        war_id,
        PeaceTerm::CedeWarGoalRegion,
        &current_date,
    );
    assert!(
        result.is_ok(),
        "ceding a capital state must succeed (with capital reassignment), not fail: {result:?}"
    );

    // 領土は割譲どおりclaimantへ移り、元Claimは消費される。
    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(target_state)
            .unwrap()
            .owner_country_id,
        claimant
    );
    assert_eq!(
        app.world()
            .resource::<ClaimRegistry>()
            .claims
            .get(&claim_id)
            .unwrap()
            .status,
        ClaimStatus::Consumed
    );

    // targetの首都が、割譲された旧首都(target_state)から別の州へ再割当されている
    // (targetがまだ陸州を所有していることが前提——実マップの6か国はいずれも
    // 複数州を持つため、単一州の割譲で無所有国になることはない)。
    let new_capital = app
        .world()
        .resource::<CountryRegistry>()
        .get(target)
        .unwrap()
        .capital_state_id;
    assert_ne!(
        new_capital, target_state,
        "the target's capital must have been reassigned away from the ceded state"
    );
    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(new_capital)
            .unwrap()
            .owner_country_id,
        target,
        "the reassigned capital must still be a state the (former) capital owner actually owns"
    );

    // Save validationを通過し、Save/Load後も割譲・Claim消費・再割当首都が維持される。
    round_trip_through_save(&mut app);

    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(target_state)
            .unwrap()
            .owner_country_id,
        claimant,
        "territory transfer must survive save/load"
    );
    assert_eq!(
        app.world()
            .resource::<ClaimRegistry>()
            .claims
            .get(&claim_id)
            .unwrap()
            .status,
        ClaimStatus::Consumed,
        "claim consumption must survive save/load"
    );
    assert_eq!(
        app.world()
            .resource::<CountryRegistry>()
            .get(target)
            .unwrap()
            .capital_state_id,
        new_capital,
        "the reassigned capital must survive save/load"
    );
}

// ─────────────────────────────────────────────────────────────────────
// ユーザー指摘対応(2026-08-21): P21-015(AI Crisis支持)/P21-016(多国間War)を
// 手構築の単体テストではなく、実際のAI判断経路(`decide_ai_crisis_support`/
// `evaluate_ai_crisis_support`が日次ループから自動的に呼ばれる経路)を通した
// 完全E2Eで検証する。`pledge_support`を直接呼ばない・早期returnで
// スキップしない・実マップデータを使う。
// ─────────────────────────────────────────────────────────────────────

/// `claimant`/`target`以外の実在国家を`CountryId`昇順で1つ選ぶ
/// (第三国支持国として使う。プレイヤー国は除外する)。
fn pick_third_country(app: &App, exclude: &[CountryId]) -> CountryId {
    let country_registry = app.world().resource::<CountryRegistry>();
    let mut ids: Vec<CountryId> = country_registry.countries.iter().map(|c| c.id).collect();
    ids.sort_by_key(|id| id.0);
    ids.into_iter()
        .find(|id| !exclude.contains(id))
        .expect("real map must have at least one country beyond claimant/target/player")
}

/// `claimant`と`supporter`を(6か国中2枠しかない)`PowerTier::GreatPower`へ
/// 決定論的に押し上げ、`target`を確実にそこから除外する
/// (`force_power_advantage`と同じ人口操作手法を第三国にも拡張したもの)。
fn force_power_advantage_with_ally(
    app: &mut App,
    claimant: CountryId,
    target: CountryId,
    supporter: CountryId,
) {
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        for state in state_registry.states.iter_mut() {
            if state.owner_country_id == claimant {
                state.population = state.population.max(10_000_000);
            }
            if state.owner_country_id == supporter {
                state.population = state.population.max(8_000_000);
            }
            if state.owner_country_id == target {
                state.population = state.population.min(100);
            }
        }
    }
    strategy_game::country::power::rebuild_country_power_registry_from_world(app.world_mut());
}

/// `a`・`b`間にAlliance条約を締結する(`decide_ai_crisis_support`の
/// `ally_of_initiator`分岐を無条件に発火させ、関係値の大小に依存させないため)。
fn set_alliance(app: &mut App, a: CountryId, b: CountryId) {
    let key = DiplomaticPairKey::new(a, b).unwrap();
    app.world_mut()
        .resource_mut::<DiplomacyRegistry>()
        .relations
        .insert(
            key,
            DiplomaticRelation {
                treaties: vec![ActiveTreaty {
                    treaty_type: TreatyType::Alliance,
                    countries: (a, b),
                    signed_date: "1800/01/01".to_string(),
                    is_active: true,
                }],
                ..Default::default()
            },
        );
}

/// 実際のAI支持判断経路(`evaluate_ai_crisis_support`が日次ループから呼ばれる)で
/// `supporter`がinitiator支持を表明するまで日を進める。`pledge_support`を
/// 直接呼ばない(要求: 早期returnなしで実経路を1本通す)。
fn advance_until_supporter_pledges_initiator(
    app: &mut App,
    claimant: CountryId,
    supporter: CountryId,
    max_days: u32,
) {
    for _ in 0..max_days {
        advance_one_day(app);
        let pledged = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .values()
            .any(|c| {
                c.initiator == claimant
                    && c.third_party_reactions.get(&supporter)
                        == Some(&ThirdCountryReaction::SupportsInitiator)
            });
        if pledged {
            return;
        }
    }
    panic!(
        "the real AI crisis-support path (evaluate_ai_crisis_support) never had \
         supporter({supporter:?}) pledge SupportsInitiator within {max_days} days"
    );
}

/// 要求53-65(完全版): AI Claim生成→Crisis開始→**実際の**AI支持判断
/// (`evaluate_ai_crisis_support`/`decide_ai_crisis_support`が日次ループから
/// 自動的に呼ぶ経路、`pledge_support`直接呼び出しではない)→拒否→
/// 多国間`declare_war`(supporterがWar参加国として実際に組み込まれる)→
/// `execute_peace_settlement`→支持国(supporter)には割譲州が渡らず、
/// 主導国(claimant)にのみ渡ること、元Claimが消費されることを、
/// 早期returnなしで最後まで確認する。
#[test]
fn ai_crisis_support_via_real_decision_path_joins_war_but_does_not_receive_ceded_territory() {
    let bystander = CountryId(99);
    let mut app = setup_app_in_playing(bystander);
    let (claimant, target, target_state) = pick_adjacent_border_pair(&app, bystander, true);
    let supporter = pick_third_country(&app, &[claimant, target, bystander]);

    force_power_advantage_with_ally(&mut app, claimant, target, supporter);
    set_alliance(&mut app, supporter, claimant);
    set_hostile_opinion(&mut app, claimant, target);

    advance_until_claim_created_by(&mut app, claimant, 40);
    advance_until_crisis_started_by(&mut app, claimant, 40);

    // 要求: `pledge_support`を直接呼ばず、実際の日次AI判断経路だけで支持を発生させる。
    advance_until_supporter_pledges_initiator(&mut app, claimant, supporter, 40);

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
    let war_id =
        war_id.expect("the AI-generated claim/crisis/support pipeline must lead to a declared war");

    // 要求: supporterが実際にWar参加国(attackers)として組み込まれている
    // (third_party_reactionsのスナップショットだけでなく、Warそのものに反映済み)。
    let war = app
        .world()
        .resource::<WarRegistry>()
        .wars
        .get(&war_id)
        .unwrap()
        .clone();
    assert!(
        war.attackers.contains(&supporter),
        "supporter must have actually joined the war as an attacker via the real AI support path, \
         got attackers={:?}",
        war.attackers
    );
    assert_eq!(
        war.primary_attacker_id(),
        claimant,
        "claimant (the crisis initiator) must remain the primary attacker, not the supporter"
    );

    let claim_id = app
        .world()
        .resource::<ClaimRegistry>()
        .claims
        .values()
        .find(|c| c.claimant_country == claimant)
        .expect("the claim that led to this war must still exist")
        .id;

    let current_date = app
        .world()
        .resource::<strategy_game::app::time::GameDate>()
        .display();
    let result = settle_peace(
        &mut app,
        war_id,
        PeaceTerm::CedeWarGoalRegion,
        &current_date,
    );
    assert!(
        result.is_ok(),
        "peace settlement for the multilateral crisis-derived war must succeed: {result:?}"
    );

    // 要求: 割譲州は主導国(claimant)へ渡り、支持国(supporter)には渡らない。
    let new_owner = app
        .world()
        .resource::<StateRegistry>()
        .get(target_state)
        .unwrap()
        .owner_country_id;
    assert_eq!(
        new_owner, claimant,
        "the ceded state must go to the primary attacker (claimant)"
    );
    assert_ne!(
        new_owner, supporter,
        "the ceded state must NOT go to the supporting third country"
    );

    // 要求: 元Claimが消費される。
    assert_eq!(
        app.world()
            .resource::<ClaimRegistry>()
            .claims
            .get(&claim_id)
            .unwrap()
            .status,
        ClaimStatus::Consumed
    );
}
