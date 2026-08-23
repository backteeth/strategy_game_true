//! P21-020: 州所有権移転の共通Domain API化と講和エラー可視化の統合テスト。
//!
//! 実データ(`assets/data/states.ron`/`countries.ron`、6か国28州の本番マップ)を使い、
//! 複数のActive Warが同じ州を目標にし得る(`can_declare_war_with_date`は対象州の
//! グローバルな一意性を検証しない)実際の状況で、片方の講和が州所有権を実際に
//! 移した瞬間、もう一方のWarが安全に白紙講和で失効すること(ユーザー指摘対応、
//! §「複数Warの同一州競合」)を、以下2つの角度で確認する。
//!
//! 1. `normal_war_conflicting_over_the_same_state_is_safely_invalidated_not_stuck`:
//!    `tests/land_war_combat_peace_test.rs`と同じ、Bevy Appを介さない直接Registry
//!    呼び出しの手法で、ドメインAPI自体の挙動を確認する。
//! 2. `two_real_wars_over_the_same_state_resolve_via_the_real_peace_button_and_pass_save_validation`:
//!    実際のBevy App(`AppPlugin`+`PeacePanelPlugin`)で、実際の講和ボタン
//!    (`PeaceActionButton`+`Interaction::Pressed`)を押し、実際の`GameNotification`が
//!    送られることを確認したうえで、通常プレイで到達し得るこの状態が
//!    Save validationを通過することを確認する(要求69-76の完全な統合確認)。

use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::common::{CountryId, StateId, WarId};
use strategy_game::country::{CountryData, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::claims::ClaimRegistry;
use strategy_game::diplomacy::crisis::CrisisRegistry;
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::state::data::StateData;
use strategy_game::state::data::StateRegistry;
use strategy_game::ui::notification::GameNotification;
use strategy_game::ui::peace_panel::{PeaceActionButton, PeacePanelPlugin};
use strategy_game::war::data::{WarRegistry, WarStatus};
use strategy_game::war::frontline::FrontlineRegistry;
use strategy_game::war::justification::WarJustificationRegistry;
use strategy_game::war::peace::{PeaceTerm, execute_peace_settlement};

fn load_real_map() -> (StateRegistry, CountryRegistry) {
    let states_ron = std::fs::read_to_string("assets/data/states.ron").unwrap();
    let states: Vec<StateData> = ron::from_str(&states_ron).unwrap();
    let state_reg = StateRegistry::build(states);

    let countries_ron = std::fs::read_to_string("assets/data/countries.ron").unwrap();
    let countries: Vec<CountryData> = ron::from_str(&countries_ron).unwrap();
    let country_reg = CountryRegistry { countries };

    (state_reg, country_reg)
}

/// 実データの正当化→開戦フローで、指定した攻撃国/防御国/対象州のWarを実際に
/// 作成する(`tests/land_war_combat_peace_test.rs`と同じ手法)。
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
        .expect("justification start must succeed for a real adjacent pair");

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

// ─────────────────────────────────────────────────────────────────────
// 1. ドメインAPI自体の挙動(直接Registry呼び出し)
// ─────────────────────────────────────────────────────────────────────

/// 通常War2件(Dwarf(2)→Oceanic(3)、Ferrowyn(5)→Oceanic(3))が実データ上の
/// 同じ非首都州(State 9, Magic Harbor)を目標にできてしまう(`can_declare_war_
/// with_date`は対象州のグローバルな一意性を検証しないため)実際の状況を作り、
/// 片方(War B)の講和で州所有権が実際にFerrowynへ移った瞬間、もう一方(War A)が
/// 自動的に白紙講和で安全に失効し、「Active Warのまま毎日自動降伏が失敗し
/// 続けて永久停止する」状態には陥らないことを確認する。
#[test]
fn normal_war_conflicting_over_the_same_state_is_safely_invalidated_not_stuck() {
    let (mut state_reg, country_reg) = load_real_map();

    let dwarf = CountryId(2);
    let ferrowyn = CountryId(5);
    let oceanic = CountryId(3);
    let target_state = StateId(9);
    assert_eq!(
        state_reg.get(target_state).unwrap().owner_country_id,
        oceanic
    );
    assert!(
        state_reg
            .get(StateId(7))
            .unwrap()
            .neighbors
            .contains(&target_state)
    );
    assert!(
        state_reg
            .get(StateId(26))
            .unwrap()
            .neighbors
            .contains(&target_state)
    );

    let mut war_reg = WarRegistry::default();
    let mut justification_reg = WarJustificationRegistry::default();
    let mut diplomacy_reg = DiplomacyRegistry::default();
    let mut military_reg = MilitaryRegistry::default();
    let mut battle_reg = BattleRegistry::default();
    let mut frontline_reg = FrontlineRegistry::default();
    let mut crisis_reg = CrisisRegistry::default();
    let mut claim_reg = ClaimRegistry::default();
    let mut country_reg_mut = country_reg;
    let mut army_reg = strategy_game::military::army::ArmyRegistry::default();
    let mut power_reg = strategy_game::country::power::CountryPowerRegistry::default();

    // War A: Dwarf(2) → Oceanic(3)、対象 State 9。
    let war_a = declare_real_war(
        &mut war_reg,
        &mut justification_reg,
        &mut diplomacy_reg,
        &country_reg_mut,
        &state_reg,
        dwarf,
        oceanic,
        target_state,
        "1936/01/01",
    );

    // War B: Ferrowyn(5) → Oceanic(3)、**同じ対象州** State 9。
    let war_b = declare_real_war(
        &mut war_reg,
        &mut justification_reg,
        &mut diplomacy_reg,
        &country_reg_mut,
        &state_reg,
        ferrowyn,
        oceanic,
        target_state,
        "1936/02/01",
    );

    assert_eq!(war_reg.wars.get(&war_a).unwrap().status, WarStatus::Active);
    assert_eq!(war_reg.wars.get(&war_b).unwrap().status, WarStatus::Active);

    // War Bを実際に`execute_peace_settlement`で割譲講和し、State 9を
    // Ferrowynへ正当に移転させる。
    execute_peace_settlement(
        war_b,
        PeaceTerm::CedeWarGoalRegion,
        "Test Cession",
        "1936/03/01",
        &mut state_reg,
        &mut war_reg,
        &mut military_reg,
        &mut battle_reg,
        &mut diplomacy_reg,
        &mut frontline_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg_mut,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
    .expect("War B's cession must succeed via the normal domain API");

    assert_eq!(
        state_reg.get(target_state).unwrap().owner_country_id,
        ferrowyn,
        "State 9 must now legitimately belong to Ferrowyn"
    );
    assert_eq!(
        war_reg.wars.get(&war_b).unwrap().status,
        WarStatus::AttackerVictory
    );

    // War Aは、対象州が既に第三国(Ferrowyn)へ渡った時点で自動的に白紙講和され、
    // Activeのまま取り残されない(ユーザー指摘対応: 「安全に処理済み」の実装)。
    assert_eq!(
        war_reg.wars.get(&war_a).unwrap().status,
        WarStatus::WhitePeace,
        "War A must be automatically invalidated (white peace) once its target state \
         legitimately belongs to a third party, not left Active forever"
    );
    assert_eq!(
        state_reg.get(target_state).unwrap().owner_country_id,
        ferrowyn,
        "the third party (Ferrowyn) must not lose the state to War A's attacker"
    );

    // 自動失効後、War Aへ改めて自動降伏講和を試みても(War Bとは無関係な日次
    // ループを模して)二重適用防止によりOk(())で安全に素通りするだけで、
    // 何も壊れない。
    let result_a_retry = execute_peace_settlement(
        war_a,
        PeaceTerm::CedeWarGoalRegion,
        "Retry",
        "1936/04/01",
        &mut state_reg,
        &mut war_reg,
        &mut military_reg,
        &mut battle_reg,
        &mut diplomacy_reg,
        &mut frontline_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg_mut,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    );
    assert_eq!(result_a_retry, Ok(()));

    // 一方、War A/Bとは無関係な別の正常なWarは問題なく講和が成立できる
    // ——システム全体が壊れているわけではない。
    let war_c = declare_real_war(
        &mut war_reg,
        &mut justification_reg,
        &mut diplomacy_reg,
        &country_reg_mut,
        &state_reg,
        CountryId(0), // Arcadia
        CountryId(1), // Elfin Republic
        StateId(3),   // Elfin Central、Arcadiaの State 1 と隣接
        "1936/05/01",
    );
    execute_peace_settlement(
        war_c,
        PeaceTerm::CedeWarGoalRegion,
        "Test Cession",
        "1936/06/01",
        &mut state_reg,
        &mut war_reg,
        &mut military_reg,
        &mut battle_reg,
        &mut diplomacy_reg,
        &mut frontline_reg,
        &mut crisis_reg,
        &mut claim_reg,
        &mut country_reg_mut,
        &mut army_reg,
        &mut justification_reg,
        &mut power_reg,
    )
    .expect("an unrelated, valid war's cession must still succeed");
    assert_eq!(
        state_reg.get(StateId(3)).unwrap().owner_country_id,
        CountryId(0)
    );
}

// ─────────────────────────────────────────────────────────────────────
// 2. 実際の講和ボタン・GameNotification・Save validationまでの完全な統合確認
// ─────────────────────────────────────────────────────────────────────

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

/// 要求69-76(完全版): 実際のBevy App・実データ・実際の講和ボタン・実際の
/// `GameNotification`を通し、複数Warの同一州競合が自動的に安全処理された後の
/// 状態がSave validationを通過することまで確認する。
#[test]
fn two_real_wars_over_the_same_state_resolve_via_the_real_peace_button_and_pass_save_validation() {
    // Save検証はplayer_countryが実在するCountryIdであることを要求するため、
    // War Bの当事国(Ferrowyn)をそのままプレイヤーとして使う
    // (講和ボタンは常にplayer視点から申し入れる仕様のため)。
    let ferrowyn = CountryId(5);
    let mut app = setup_app_in_playing(ferrowyn);

    let dwarf = CountryId(2);
    let oceanic = CountryId(3);
    let target_state = StateId(9);

    let (war_a, war_b) =
        app.world_mut()
            .resource_scope(|world, mut war_registry: Mut<WarRegistry>| {
                world.resource_scope(
                    |world, mut justification_registry: Mut<WarJustificationRegistry>| {
                        world.resource_scope(
                            |world, mut diplomacy_registry: Mut<DiplomacyRegistry>| {
                                world.resource_scope(
                                    |world, country_registry: Mut<CountryRegistry>| {
                                        world.resource_scope(
                                            |_world, state_registry: Mut<StateRegistry>| {
                                                assert_eq!(
                                                    state_registry
                                                        .get(target_state)
                                                        .unwrap()
                                                        .owner_country_id,
                                                    oceanic
                                                );

                                                let war_a = declare_real_war(
                                                    &mut war_registry,
                                                    &mut justification_registry,
                                                    &mut diplomacy_registry,
                                                    &country_registry,
                                                    &state_registry,
                                                    dwarf,
                                                    oceanic,
                                                    target_state,
                                                    "1936/01/01",
                                                );
                                                let war_b = declare_real_war(
                                                    &mut war_registry,
                                                    &mut justification_registry,
                                                    &mut diplomacy_registry,
                                                    &country_registry,
                                                    &state_registry,
                                                    ferrowyn,
                                                    oceanic,
                                                    target_state,
                                                    "1936/02/01",
                                                );
                                                (war_a, war_b)
                                            },
                                        )
                                    },
                                )
                            },
                        )
                    },
                )
            });

    // 講和成立の前提(war_score等)を満たすため、War Bのwar_scoreを引き上げる
    // (`can_accept_peace_offer`のCedeWarGoalRegion分岐: war_score>=60 または
    // 防御側降伏)。
    app.world_mut()
        .resource_mut::<WarRegistry>()
        .wars
        .get_mut(&war_b)
        .unwrap()
        .war_score = 100.0;

    // 既存の通知を捨ててから、実際の講和ボタンを押す(PeaceActionButton +
    // Interaction::Pressed、`handle_peace_action_buttons`が実際に処理する)。
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
        "the real peace button settlement must succeed, got: {:?}",
        notifications[0]
    );

    assert_eq!(
        app.world()
            .resource::<StateRegistry>()
            .get(target_state)
            .unwrap()
            .owner_country_id,
        ferrowyn,
        "State 9 must now legitimately belong to Ferrowyn via the real peace button"
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
    assert_eq!(
        app.world()
            .resource::<WarRegistry>()
            .wars
            .get(&war_a)
            .unwrap()
            .status,
        WarStatus::WhitePeace,
        "War A must have been automatically invalidated by the real peace button's settlement"
    );

    // 要求: 通常プレイでこの状態(2つのWarが同じ州を争い、片方の講和で
    // もう片方が自動的に白紙講和された状態)へ到達した場合、そのSaveが
    // validationを通過する。
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
            "a Save reached through real war declarations + a real peace-button settlement \
             (including the auto-invalidated conflicting War) must pass validation",
        )
    };

    assert_eq!(
        strategy_game::save::apply_validated_save(app.world_mut(), validated),
        strategy_game::save::ApplyLoadOutcome::Success
    );

    // Load後も両方のWarの終結状態が維持されていることを確認する。
    assert_eq!(
        app.world()
            .resource::<WarRegistry>()
            .wars
            .get(&war_b)
            .unwrap()
            .status,
        WarStatus::AttackerVictory
    );
    assert_eq!(
        app.world()
            .resource::<WarRegistry>()
            .wars
            .get(&war_a)
            .unwrap()
            .status,
        WarStatus::WhitePeace
    );
}
