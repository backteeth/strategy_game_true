//! P21-021: 国家消滅・完全併合の統合テスト。
//!
//! 実データ(`assets/data/states.ron`/`countries.ron`、6か国28州の本番マップ)を使い、
//! Passguard March(CountryId(4))を最後の所有陸州(首都StateId(15))まで削り、実際の
//! 講和ボタン(`PeaceActionButton`+`Interaction::Pressed`)でその最後の州を割譲させる。
//! これにより国家消滅(完全併合)が実際にトリガーされ、以下を実データE2Eとして確認する:
//!
//! - `CountryStatus::Eliminated`への遷移(`CountryRegistry`から物理削除されない)
//! - 軍事清掃(残存Divisionの解散)
//! - 戦争清掃(消滅国家が主要参加国の、無関係な別Warの終結)
//! - Claim清掃(消滅国家自身のActive Claimの無効化)
//! - Crisis清掃(消滅国家が当事国の、未解決Crisisの強制終端)
//! - 実際の`GameNotification`
//! - Save/Load往復後も消滅状態が正しく維持され、国家が復活しないこと

use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::common::{ClaimId, CountryId, DiplomaticCrisisId, StateId, WarId};
use strategy_game::country::{CountryRegistry, CountryStatus, PlayerCountry};
use strategy_game::diplomacy::claims::{ClaimRegistry, ClaimSource, ClaimStatus, TerritorialClaim};
use strategy_game::diplomacy::crisis::{CrisisPhase, CrisisRegistry, DiplomaticCrisis};
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::state::data::StateRegistry;
use strategy_game::ui::notification::GameNotification;
use strategy_game::ui::peace_panel::{PeaceActionButton, PeacePanelPlugin};
use strategy_game::war::data::{WarRegistry, WarStatus};
use strategy_game::war::justification::WarJustificationRegistry;
use strategy_game::war::peace::PeaceTerm;

/// 実データの正当化→開戦フローで、指定した攻撃国/防御国/対象州のWarを実際に
/// 作成する(`tests/p21_020_ownership_transfer_e2e_test.rs`と同じ手法)。
#[allow(clippy::too_many_arguments)]
fn declare_real_war(
    war_reg: &mut WarRegistry,
    justification_reg: &mut WarJustificationRegistry,
    diplomacy_reg: &mut DiplomacyRegistry,
    country_reg: &CountryRegistry,
    state_reg: &StateRegistry,
    attacker: CountryId,
    defender: CountryId,
    target_state: StateId,
    start_date: &str,
) -> WarId {
    justification_reg
        .start_justification(
            attacker,
            defender,
            target_state,
            start_date.to_string(),
            country_reg,
            state_reg,
            diplomacy_reg,
        )
        .expect("justification start must succeed for a real target");

    for _ in 0..30 {
        justification_reg.process_daily_justifications(state_reg);
    }

    war_reg
        .declare_war(
            attacker,
            defender,
            target_state,
            start_date.to_string(),
            country_reg,
            state_reg,
            diplomacy_reg,
            justification_reg,
        )
        .expect("war declaration must succeed after justification completes")
}

fn setup_app_in_playing(player: CountryId) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(AppPlugin)
        .add_plugins(strategy_game::country::CountryPlugin)
        .add_plugins(strategy_game::state::StatePlugin)
        .add_plugins(strategy_game::building::BuildingPlugin)
        .add_plugins(strategy_game::economy::EconomyPlugin)
        .add_plugins(strategy_game::research::ResearchPlugin)
        .add_plugins(strategy_game::politics::PoliticsPlugin)
        .add_plugins(strategy_game::diplomacy::DiplomacyPlugin)
        .add_plugins(strategy_game::war::WarPlugin)
        .add_plugins(strategy_game::military::MilitaryPlugin)
        .add_plugins(PeacePanelPlugin);

    app.insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
        .insert_resource(strategy_game::map::division_selection::DragSelectState::default())
        .insert_resource(strategy_game::military::army::SelectedArmy::default())
        .insert_resource(strategy_game::state::SelectedState::default())
        .insert_resource(strategy_game::ui::ActivePanel::default())
        .insert_resource(strategy_game::ui::diplomacy_panel::DiplomacyPanelState::default())
        .insert_resource(strategy_game::ui::military_panel::MilitaryPanelState::default())
        .insert_resource(strategy_game::ui::politics_panel::PoliticsPanelState::default())
        .insert_resource(strategy_game::ui::research_panel::ResearchPanelState::default())
        .insert_resource(strategy_game::map::camera::CameraDragState::default());

    app.add_plugins(strategy_game::save::SaveGamePlugin);
    app.add_plugins(strategy_game::save::LoadGamePlugin);

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

fn press_peace_action_button(app: &mut App, war_id: WarId, term: PeaceTerm) {
    app.world_mut().spawn((
        Button,
        Interaction::Pressed,
        PeaceActionButton { war_id, term },
    ));
    app.update();
}

fn drain_notifications(app: &mut App) -> Vec<String> {
    app.world_mut()
        .resource_mut::<Messages<GameNotification>>()
        .drain()
        .map(|n| n.message)
        .collect()
}

#[test]
fn war_cession_of_a_countrys_last_state_eliminates_it_cleans_up_and_survives_save_round_trip() {
    let arcadia = CountryId(0); // attacker / player / annexer
    let passguard = CountryId(4); // Passguard March, elimination target
    let elfin = CountryId(1); // bystander receiving Passguard's non-capital states

    let mut app = setup_app_in_playing(arcadia);

    // Passguard(4)の首都はStateId(15)(実データ)。
    let (war_a, war_b, non_capital_state) =
        app.world_mut()
            .resource_scope(|world, mut state_registry: Mut<StateRegistry>| {
                let passguard_capital = world
                    .resource::<CountryRegistry>()
                    .get(passguard)
                    .unwrap()
                    .capital_state_id;
                assert_eq!(passguard_capital, StateId(15));

                let non_capital_states: Vec<StateId> = state_registry
                    .states
                    .iter()
                    .filter(|s| {
                        !s.is_sea && s.owner_country_id == passguard && s.id != passguard_capital
                    })
                    .map(|s| s.id)
                    .collect();
                assert!(
                    !non_capital_states.is_empty(),
                    "the real map must give Passguard more than just its capital initially"
                );
                let non_capital_state = non_capital_states[0];

                let (war_a, war_b) =
                    world.resource_scope(|world, mut war_registry: Mut<WarRegistry>| {
                        world.resource_scope(
                            |world, mut justification_registry: Mut<WarJustificationRegistry>| {
                                world.resource_scope(
                                    |world, mut diplomacy_registry: Mut<DiplomacyRegistry>| {
                                        let country_registry = world.resource::<CountryRegistry>();

                                        // Passguardが主要参加国である、causing war以外の
                                        // 無関係な別のActive War(§9: 白紙講和で終結するはず)。
                                        // Passguardがまだ非首都州を実際に所有している間に
                                        // 宣戦する(所有者不一致で拒否されないため)。
                                        let war_a = declare_real_war(
                                            &mut war_registry,
                                            &mut justification_registry,
                                            &mut diplomacy_registry,
                                            country_registry,
                                            &state_registry,
                                            CountryId(5), // Ferrowyn League (no existing treaty with Passguard)
                                            passguard,
                                            non_capital_state,
                                            "1936/01/01",
                                        );

                                        // causing war: Arcadia(0)がPassguard(4)の最後の州
                                        // (首都StateId(15))を目標に宣戦する。
                                        let war_b = declare_real_war(
                                            &mut war_registry,
                                            &mut justification_registry,
                                            &mut diplomacy_registry,
                                            country_registry,
                                            &state_registry,
                                            arcadia,
                                            passguard,
                                            passguard_capital,
                                            "1936/02/01",
                                        );

                                        (war_a, war_b)
                                    },
                                )
                            },
                        )
                    });

                // 首都以外の全所有陸州を、この戦争群とは無関係な第三国(Elfin)へ
                // 直接付け替え、Passguardが最後の陸州(首都)だけを残した
                // 「実際のプレイで到達し得る」状態を作る(両Warの宣戦布告後)。
                for sid in non_capital_states {
                    state_registry.get_mut(sid).unwrap().owner_country_id = elfin;
                }
                assert_eq!(
                    state_registry
                        .states
                        .iter()
                        .filter(|s| !s.is_sea && s.owner_country_id == passguard)
                        .count(),
                    1,
                    "Passguard must now own exactly its capital as its only land state"
                );

                (war_a, war_b, non_capital_state)
            });
    let _ = non_capital_state;

    // Passguard(4)所有のDivisionを1件配置する(§8: 消滅時に解散されるはず)。
    let division_id =
        app.world_mut()
            .resource_scope(|_world, mut military_registry: Mut<MilitaryRegistry>| {
                military_registry.add_division(Division {
                    id: strategy_game::common::DivisionId(0),
                    owner: passguard,
                    division_type: DivisionType::Infantry,
                    size: DivisionSize::Standard,
                    current_state: StateId(15),
                    destination: None,
                    current_path: vec![],
                    target_state: None,
                    manpower: 5_000,
                    max_manpower: 5_000,
                    equipment: 100.0,
                    max_equipment: 100.0,
                    organization: 100.0,
                    max_organization: 100.0,
                    morale: 100.0,
                    max_morale: 100.0,
                    experience: 0.0,
                    supply_ratio: 1.0,
                    movement_progress: 0.0,
                    status: DivisionStatus::Idle,
                    def_id: strategy_game::common::DivisionDefinitionId(0),
                    attack_power: 10,
                    defense_power: 10,
                    combat_id: None,
                })
            });

    // Passguard(4)自身が請求者のActive Claimを1件登録する(§11: 消滅時にInvalidatedへ)。
    let claim_id = app
        .world_mut()
        .resource_mut::<ClaimRegistry>()
        .add_claim(TerritorialClaim {
            id: ClaimId(0),
            claimant_country: passguard,
            target_state: StateId(16),
            strength: 50.0,
            created_date: "1936/01/01".to_string(),
            is_permanent: false,
            source: ClaimSource::Strategic,
            status: ClaimStatus::Active,
        });

    // Passguard(4)が当事国の未解決Crisisを1件登録する(§11: 消滅時にCancelledByCountryElimination)。
    let crisis_id = app
        .world_mut()
        .resource_mut::<CrisisRegistry>()
        .add_crisis(DiplomaticCrisis {
            id: DiplomaticCrisisId(0),
            initiator: CountryId(2),
            target: passguard,
            war_goals: vec![],
            start_date: "1936/01/01".to_string(),
            current_phase: CrisisPhase::DemandSent,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: Some("1936/12/31".to_string()),
            international_concern: 0.0,
            third_party_reactions: Default::default(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        });

    // 講和成立の前提(war_score>=60)を満たす。
    app.world_mut()
        .resource_mut::<WarRegistry>()
        .wars
        .get_mut(&war_b)
        .unwrap()
        .war_score = 100.0;

    let _ = drain_notifications(&mut app);
    press_peace_action_button(&mut app, war_b, PeaceTerm::CedeWarGoalRegion);

    let notifications = drain_notifications(&mut app);
    assert_eq!(
        notifications.len(),
        1,
        "the real peace button must send exactly one notification"
    );
    assert!(
        !notifications[0].to_lowercase().contains("fail") && !notifications[0].contains("失敗"),
        "the real peace button settlement (triggering elimination) must succeed, got: {:?}",
        notifications[0]
    );

    // ── 州所有権・戦争終結 ──────────────────────────────────────────────
    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(StateId(15))
            .unwrap()
            .owner_country_id,
        arcadia,
        "Passguard's last state must now belong to Arcadia"
    );
    assert_eq!(
        app.world()
            .resource::<WarRegistry>()
            .wars
            .get(&war_b)
            .unwrap()
            .status,
        WarStatus::AttackerVictory
    );
    assert_ne!(
        app.world()
            .resource::<WarRegistry>()
            .wars
            .get(&war_a)
            .unwrap()
            .status,
        WarStatus::Active,
        "an unrelated other war Passguard was a primary participant in must end"
    );

    // ── 国家状態 ─────────────────────────────────────────────────────
    let country_registry = app.world().resource::<CountryRegistry>();
    let eliminated = country_registry.get(passguard).unwrap();
    assert!(
        eliminated.is_eliminated(),
        "Passguard must transition to Eliminated"
    );
    assert_eq!(
        eliminated.status,
        CountryStatus::Eliminated {
            eliminated_date: app
                .world()
                .resource::<strategy_game::app::time::GameDate>()
                .display(),
            annexed_by: arcadia,
        }
    );
    assert_eq!(
        country_registry.countries.len(),
        6,
        "the eliminated country must not be physically removed from the registry"
    );

    // ── 軍事清掃 ─────────────────────────────────────────────────────
    assert!(
        !app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .contains_key(&division_id),
        "Passguard's division must be disbanded"
    );

    // ── Claim清掃 ────────────────────────────────────────────────────
    assert_eq!(
        app.world()
            .resource::<ClaimRegistry>()
            .claims
            .get(&claim_id)
            .unwrap()
            .status,
        ClaimStatus::Invalidated
    );

    // ── Crisis清掃 ───────────────────────────────────────────────────
    assert_eq!(
        app.world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .current_phase,
        CrisisPhase::CancelledByCountryElimination
    );

    // ── Save/Load往復 ────────────────────────────────────────────────
    let save = {
        let world = app.world();
        let sources = strategy_game::save::SaveGameSources {
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
        strategy_game::save::build_save_game_v1(&sources)
    };

    let validated = {
        let world = app.world();
        let context = strategy_game::save::SaveValidationContext {
            building_definitions: &world
                .resource::<strategy_game::building::data::BuildingRegistry>()
                .definitions,
            technology_definitions: &world
                .resource::<strategy_game::research::data::TechnologyRegistry>()
                .definitions,
            division_definitions: &world
                .resource::<strategy_game::military::data::MilitaryRegistry>()
                .definitions,
            world_stage_definitions: &world
                .resource::<strategy_game::research::world_stage::WorldCivilizationState>()
                .stage_definitions,
        };
        strategy_game::save::validate_save_game_v1(save, &context).expect(
            "a Save reached through a real war declaration + a real peace-button settlement \
             that triggers country elimination must pass validation",
        )
    };

    assert_eq!(
        strategy_game::save::apply_validated_save(app.world_mut(), validated),
        strategy_game::save::ApplyLoadOutcome::Success
    );

    // Load後も消滅状態・所有権が維持され、国家が復活していない。
    let country_registry = app.world().resource::<CountryRegistry>();
    let reloaded = country_registry.get(passguard).unwrap();
    assert!(
        reloaded.is_eliminated(),
        "Passguard must remain Eliminated after a save/load round trip"
    );
    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(StateId(15))
            .unwrap()
            .owner_country_id,
        arcadia
    );

    // 追加の日次進行(AIループを含む)でpanicせず、消滅国家が復活しないことを確認する。
    app.world_mut()
        .write_message(strategy_game::app::time::DayChangedMessage {
            year: 1936,
            month: 3,
            day: 2,
        });
    app.update();
    assert!(
        app.world()
            .resource::<CountryRegistry>()
            .get(passguard)
            .unwrap()
            .is_eliminated(),
        "a further day of AI simulation must not resurrect an eliminated country"
    );
}
