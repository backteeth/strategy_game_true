//! P21-016: Crisis支持コミットメントの多国間War参加への接続の統合テスト。
//!
//! 実データ(`assets/data/*.ron`、6か国28州の本番マップ)を`AppPlugin`経由で読み込み、
//! 複数の第三国が攻撃側・防御側それぞれを支持した状態でCrisisが拒否され、
//! 実際のAI宣戦布告により`WarStarted`となった際に、支持国が正しい陣営の
//! War参加国として追加されること、既存の`WarRegistry`共有API(`are_countries_at_war`/
//! `is_country_at_war`)がそれらの支持国も正しく認識すること、save往復で
//! 参加者・代表国が保持されることまでを一気通貫で検証する。

use bevy::prelude::*;
use strategy_game::app::game_state::GameState;
use strategy_game::common::{CountryId, DiplomaticCrisisId, StateId};
use strategy_game::country::{CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::claims::ClaimRegistry;
use strategy_game::diplomacy::crisis::{CrisisPhase, CrisisRegistry};
use strategy_game::diplomacy::crisis_response::{self, CrisisSupportSide};
use strategy_game::state::data::StateRegistry;
use strategy_game::war::data::WarRegistry;

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
            .expect("real-map save with P21-016 multilateral war state must validate")
    };
    assert_eq!(
        apply_validated_save(app.world_mut(), validated),
        ApplyLoadOutcome::Success
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
        let date_str = world
            .resource::<strategy_game::app::time::GameDate>()
            .display();
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
    let date_str = world
        .resource::<strategy_game::app::time::GameDate>()
        .display();
    let state_registry = world.resource::<StateRegistry>().clone_for_test();
    let mut crisis_registry = world.resource_mut::<CrisisRegistry>();
    crisis_registry
        .start_crisis(&claim, claimant, target, date_str, &state_registry)
        .expect("valid claim must allow crisis start")
}

impl CloneForTest for StateRegistry {
    fn clone_for_test(&self) -> Self {
        StateRegistry::build(self.states.clone())
    }
}

/// 要求テスト: 攻撃側・防御側それぞれに支持を表明していた第三国が、実データでの
/// 拒否→AI宣戦布告後、正しい陣営のWar参加国として追加される。プレイヤーは
/// どちらの陣営にも関与しない純粋な傍観者とし、支持表明そのものが
/// プレイヤー操作に依存しないことを確認する。
#[test]
fn multiple_supporters_on_both_sides_become_war_participants_on_correct_sides() {
    let bystander = CountryId(2); // 傍観者(プレイヤー、どちらの陣営にも属さない)
    let mut app = setup_app_in_playing(bystander);
    let initiator = CountryId(1);
    let target = CountryId(3);
    let target_state = StateId(8); // CountryId(3)の首都 -> 確実に拒否させる
    let attacker_supporter = CountryId(4);
    let defender_supporter = CountryId(5);

    let crisis_id = create_claim_and_start_crisis(&mut app, initiator, target, target_state);

    {
        let countries = app.world().resource::<CountryRegistry>().clone_for_test();
        let current_date = app
            .world()
            .resource::<strategy_game::app::time::GameDate>()
            .clone();
        crisis_response::pledge_support(
            &mut app.world_mut().resource_mut::<CrisisRegistry>(),
            &countries,
            crisis_id,
            attacker_supporter,
            CrisisSupportSide::Initiator,
            &current_date,
        )
        .expect("a genuine third country must be able to support the initiator");
        crisis_response::pledge_support(
            &mut app.world_mut().resource_mut::<CrisisRegistry>(),
            &countries,
            crisis_id,
            defender_supporter,
            CrisisSupportSide::Target,
            &current_date,
        )
        .expect("a genuine third country must be able to support the target");
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

    assert_eq!(
        war.attackers,
        [initiator, attacker_supporter].into_iter().collect(),
        "attacker supporter must join the attacking side alongside the initiator"
    );
    assert_eq!(
        war.defenders,
        [target, defender_supporter].into_iter().collect(),
        "defender supporter must join the defending side alongside the target"
    );
    assert_eq!(war.primary_attacker, Some(initiator));
    assert_eq!(war.primary_defender, Some(target));
    assert!(
        !war.attackers.contains(&bystander) && !war.defenders.contains(&bystander),
        "an uninvolved bystander must never be added to either side"
    );

    // 共有の敵味方判定APIが支持国も正しく認識することを確認する(単一の主要国ペアだけを
    // 見る比較ではなく、多国間参加者全体を対象とする)。
    let war_registry = app.world().resource::<WarRegistry>();
    assert!(war_registry.are_countries_at_war(attacker_supporter, target));
    assert!(war_registry.are_countries_at_war(attacker_supporter, defender_supporter));
    assert!(!war_registry.are_countries_at_war(attacker_supporter, initiator));
    assert!(war_registry.is_country_at_war(attacker_supporter));
    assert!(war_registry.is_country_at_war(defender_supporter));
    assert!(!war_registry.is_country_at_war(bystander));
    assert_eq!(
        war_registry.wars_for_country(attacker_supporter),
        vec![war_id]
    );

    round_trip_through_save(&mut app);

    // save往復後も参加者・代表国が保持されていることを確認する。
    let war_after = app
        .world()
        .resource::<WarRegistry>()
        .wars
        .get(&war_id)
        .unwrap()
        .clone();
    assert_eq!(war_after.attackers, war.attackers);
    assert_eq!(war_after.defenders, war.defenders);
    assert_eq!(war_after.primary_attacker, Some(initiator));
    assert_eq!(war_after.primary_defender, Some(target));
}
