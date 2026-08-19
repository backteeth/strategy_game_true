//! P21-015: AI大国によるCrisis支持判断の統合テスト。
//!
//! 前半は`GameTimePlugin`(実際の`DayChangedMessage`発行タイミング・Pause判定)だけを
//! 使った軽量Appで、`DayChangedMessage`駆動での支持判断・Pause中は判断しないこと・
//! 同一日に複数Updateしても重複しないこと・P21-012のAI対象国回答より支持が先に
//! 適用されること・deadline当日は支持せず既存期限処理が優先されることを検証する。
//! 後半は実データ(`assets/data/*.ron`、6か国28州の本番マップ)を`AppPlugin`経由で
//! 読み込み、実際のGreat Power数・save往復・プレイヤー手動支持との共存を確認する。

use bevy::prelude::*;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DailySimulationSet, GameDate, GamePaused, GameTimePlugin};
use strategy_game::common::{CountryId, DiplomaticCrisisId, StateId};
use strategy_game::country::power::{CountryPowerRegistry, PowerTier, evaluate_country_power};
use strategy_game::country::{CountryData, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::claims::ClaimRegistry;
use strategy_game::diplomacy::crisis::{
    CrisisPhase, CrisisRegistry, DiplomaticCrisis, ThirdCountryReaction, WarGoal, WarGoalType,
};
use strategy_game::diplomacy::crisis_response;
use strategy_game::diplomacy::data::{
    ActiveTreaty, DiplomacyRegistry, DiplomaticPairKey, DiplomaticRelation, TreatyType,
};
use strategy_game::diplomacy::update::handle_daily_diplomacy;
use strategy_game::localization::{CurrentLocale, TranslationCatalog};
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::ui::notification::GameNotification;
use strategy_game::war::justification::WarJustificationRegistry;

// ─────────────────────────────────────────────────────────────────────────
// 軽量App: GameTimePlugin + handle_daily_diplomacyのみ
// ─────────────────────────────────────────────────────────────────────────

/// country 1を圧倒的なGreat Power(id=1)にした5か国構成のCountryPowerRegistry。
/// `compute_tier_counts(5) == (1, 2, 2)`なので、1だけがGreat Powerになる。
fn power_registry_with_great_power(great_power_id: usize) -> CountryPowerRegistry {
    let mut other_ids: Vec<usize> = (100..104).collect();
    other_ids.retain(|&id| id != great_power_id);
    let mut countries = vec![CountryData {
        id: CountryId(great_power_id),
        ..CountryData::default()
    }];
    countries.extend(other_ids.iter().map(|&id| CountryData {
        id: CountryId(id),
        ..CountryData::default()
    }));

    let mut states = vec![StateData {
        id: StateId(9000),
        owner_country_id: CountryId(great_power_id),
        population: 10_000_000,
        ..Default::default()
    }];
    for (i, &id) in other_ids.iter().enumerate() {
        states.push(StateData {
            id: StateId(9001 + i),
            owner_country_id: CountryId(id),
            population: 100,
            ..Default::default()
        });
    }

    evaluate_country_power(
        &CountryRegistry { countries },
        &StateRegistry::build(states),
        &strategy_game::military::data::MilitaryRegistry::default(),
        &strategy_game::building::data::BuildingRegistry::default(),
        "1800/01/01".to_string(),
    )
}

fn build_lightweight_ai_support_app(great_power_id: usize) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .init_state::<GameState>()
        .add_plugins(GameTimePlugin);

    app.insert_resource(DiplomacyRegistry::default());
    app.insert_resource(CrisisRegistry::default());
    app.insert_resource(PlayerCountry(None));
    app.insert_resource(CountryRegistry {
        countries: vec![
            CountryData {
                id: CountryId(0),
                ..CountryData::default()
            },
            CountryData {
                id: CountryId(great_power_id),
                ..CountryData::default()
            },
            CountryData {
                id: CountryId(10),
                ..CountryData::default()
            },
        ],
    });
    app.insert_resource(StateRegistry::build(vec![]));
    app.insert_resource(WarJustificationRegistry::default());
    app.insert_resource(ClaimRegistry::default());
    app.insert_resource(power_registry_with_great_power(great_power_id));
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
    initiator: usize,
    target: usize,
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
            deadline_date: Some(deadline_date.to_string()),
            international_concern: 0.0,
            third_party_reactions: Default::default(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        })
}

fn ally_with(app: &mut App, a: usize, b: usize) {
    let key = DiplomaticPairKey::new(CountryId(a), CountryId(b)).unwrap();
    app.world_mut()
        .resource_mut::<DiplomacyRegistry>()
        .relations
        .insert(
            key,
            DiplomaticRelation {
                treaties: vec![ActiveTreaty {
                    treaty_type: TreatyType::Alliance,
                    countries: (CountryId(a), CountryId(b)),
                    signed_date: "1800/01/01".to_string(),
                    is_active: true,
                }],
                ..Default::default()
            },
        );
}

fn supporter_of(
    app: &App,
    crisis_id: DiplomaticCrisisId,
    id: usize,
) -> Option<ThirdCountryReaction> {
    app.world()
        .resource::<CrisisRegistry>()
        .crises
        .get(&crisis_id)
        .unwrap()
        .third_party_reactions
        .get(&CountryId(id))
        .copied()
}

fn crisis_phase(app: &App, id: DiplomaticCrisisId) -> CrisisPhase {
    app.world()
        .resource::<CrisisRegistry>()
        .crises
        .get(&id)
        .unwrap()
        .current_phase
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

/// 要求テスト28: `DayChangedMessage`なしでは状態を変更しない。
#[test]
fn no_ai_support_without_day_changed_message() {
    let mut app = build_lightweight_ai_support_app(1);
    ally_with(&mut app, 1, 0);
    let crisis_id = insert_demand_sent_crisis(&mut app, 0, 10, "1800/01/31");

    app.update();
    app.update();

    assert_eq!(supporter_of(&app, crisis_id, 1), None);
}

/// 要求テスト29: DayChangedで支持判断が1回実行される。
#[test]
fn day_changed_triggers_ai_support_once() {
    let mut app = build_lightweight_ai_support_app(1);
    ally_with(&mut app, 1, 0);
    let crisis_id = insert_demand_sent_crisis(&mut app, 0, 10, "1800/01/31");

    unpause_and_advance(&mut app, 1);

    assert_eq!(
        supporter_of(&app, crisis_id, 1),
        Some(ThirdCountryReaction::SupportsInitiator)
    );
}

/// 要求テスト30: 同一フレームに複数のDayChangedがあっても重複処理しない。
#[test]
fn multiple_day_changes_in_one_frame_do_not_duplicate_support() {
    let mut app = build_lightweight_ai_support_app(1);
    ally_with(&mut app, 1, 0);
    let crisis_id = insert_demand_sent_crisis(&mut app, 0, 10, "1800/01/31");
    // 診断用Resourceを挿入し、`evaluate_ai_crisis_support`の実際の呼び出し回数を
    // 直接数える(支持結果の重複がないことだけでは、「1回だけ実行して支持した」のか
    // 「2回実行され、2回目はcan_pledge_supportの冪等性で単に上書きされただけ」なのかを
    // 区別できないため — 結果状態だけに頼らず、呼び出し回数そのものを検証する)。
    app.insert_resource(strategy_game::diplomacy::update::AiSupportEvaluationCount::default());

    // GamePausedを一度だけ解除し、accumulatorへ2日分を積んで、同一Updateフレーム内で
    // advance_game_dateがDayChangedMessageを2件書き込む状況を作る。
    app.world_mut().resource_mut::<GamePaused>().0 = false;
    app.world_mut()
        .resource_mut::<GameDate>()
        .add_accumulator(2.0);
    app.update();

    assert_eq!(
        app.world()
            .resource::<strategy_game::diplomacy::update::AiSupportEvaluationCount>()
            .0,
        1,
        "evaluate_ai_crisis_support must run exactly once per frame even when 2 \
         DayChangedMessages were queued in that same frame"
    );
    assert_eq!(
        supporter_of(&app, crisis_id, 1),
        Some(ThirdCountryReaction::SupportsInitiator)
    );
    assert_eq!(
        app.world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .third_party_reactions
            .len(),
        1
    );
}

/// 同上の重複防止テストの「全員中立(Abstain)」版。要求テスト30の重要な補足:
/// 支持が1件も追加されない(=`third_party_reactions`が常に空のまま)ケースこそ、
/// 結果状態だけでは「1回評価して誰も支持しなかった」のか「2回評価して2回とも
/// 誰も支持しなかった」のかを区別できない、最も見逃しやすいケースである。診断用
/// カウンタで実際の呼び出し回数を直接検証する。
#[test]
fn multiple_day_changes_in_one_frame_do_not_duplicate_evaluation_even_when_everyone_abstains() {
    let mut app = build_lightweight_ai_support_app(1);
    // 同盟を結ばず、関係値も未登録(=中立値0.0)のまま — Great Power(1)は
    // 必ずAbstainする状況。
    let crisis_id = insert_demand_sent_crisis(&mut app, 0, 10, "1800/01/31");
    app.insert_resource(strategy_game::diplomacy::update::AiSupportEvaluationCount::default());

    app.world_mut().resource_mut::<GamePaused>().0 = false;
    app.world_mut()
        .resource_mut::<GameDate>()
        .add_accumulator(2.0);
    app.update();

    assert_eq!(
        app.world()
            .resource::<strategy_game::diplomacy::update::AiSupportEvaluationCount>()
            .0,
        1,
        "evaluate_ai_crisis_support must run exactly once per frame even when every \
         candidate abstains on both queued day-change events"
    );
    assert_eq!(
        supporter_of(&app, crisis_id, 1),
        None,
        "precondition: with no alliance and a neutral relation, the Great Power must abstain"
    );
}

/// 要求テスト31: Pause中は支持しない。
#[test]
fn no_ai_support_while_paused() {
    let mut app = build_lightweight_ai_support_app(1);
    ally_with(&mut app, 1, 0);
    let crisis_id = insert_demand_sent_crisis(&mut app, 0, 10, "1800/01/31");
    assert!(app.world().resource::<GamePaused>().0, "starts paused");

    for _ in 0..5 {
        tick_respecting_pause(&mut app, 1.0);
    }

    assert_eq!(supporter_of(&app, crisis_id, 1), None);
}

/// 要求テスト32: 同日に支持とAI対象国回答が起きる場合、支持処理が先に適用される
/// (=同日中に追加された支持が、直後に実行されるP21-012の受諾スコア計算へ反映される)。
/// `player_third_country_supporting_initiator_flips_ai_target_to_accept`
/// (P21-013 E2Eテスト)と同じ境界ケース設計を使うが、今回は第三国プレイヤーではなく
/// AI大国自身が支持を追加する点が異なる。
#[test]
fn ai_support_is_applied_before_p21_012_ai_response_on_the_same_day() {
    let mut app = build_lightweight_ai_support_app(1);
    // country 0(initiator)/10(target)は互角の戦力・国家データ(P21-012の同種テストと
    // 同じ境界設計: 支持なしなら拒否、要求国側への十分な支持があれば受諾に転じる)。
    app.insert_resource(CountryRegistry {
        countries: vec![
            CountryData {
                id: CountryId(0),
                available_manpower: 60_000,
                capital_state_id: StateId(99),
                ..CountryData::default()
            },
            CountryData {
                id: CountryId(1),
                available_manpower: 500_000,
                capital_state_id: StateId(199),
                ..CountryData::default()
            },
            CountryData {
                id: CountryId(10),
                available_manpower: 60_000,
                capital_state_id: StateId(5),
                ..CountryData::default()
            },
        ],
    });
    app.insert_resource(StateRegistry::build(vec![StateData {
        id: StateId(1),
        owner_country_id: CountryId(10),
        population: 200_000,
        integration: 100.0,
        ..Default::default()
    }]));
    ally_with(&mut app, 1, 0); // Great Power(1)が要求国(0)と同盟 → 必ず要求国側を支持する

    // 支持なしでは拒否になることの前提確認。
    let mut baseline_app = build_lightweight_ai_support_app(1);
    baseline_app.insert_resource(CountryRegistry {
        countries: vec![
            CountryData {
                id: CountryId(0),
                available_manpower: 60_000,
                capital_state_id: StateId(99),
                ..CountryData::default()
            },
            CountryData {
                id: CountryId(1),
                available_manpower: 500_000,
                capital_state_id: StateId(199),
                ..CountryData::default()
            },
            CountryData {
                id: CountryId(10),
                available_manpower: 60_000,
                capital_state_id: StateId(5),
                ..CountryData::default()
            },
        ],
    });
    baseline_app.insert_resource(StateRegistry::build(vec![StateData {
        id: StateId(1),
        owner_country_id: CountryId(10),
        population: 200_000,
        integration: 100.0,
        ..Default::default()
    }]));
    let baseline_crisis_id = insert_demand_sent_crisis(&mut baseline_app, 0, 10, "1800/01/31");
    unpause_and_advance(&mut baseline_app, 1);
    assert_eq!(
        crisis_phase(&baseline_app, baseline_crisis_id),
        CrisisPhase::Escalating,
        "precondition: without support, the AI target must reject"
    );

    let crisis_id = insert_demand_sent_crisis(&mut app, 0, 10, "1800/01/31");
    unpause_and_advance(&mut app, 1);

    assert_eq!(
        supporter_of(&app, crisis_id, 1),
        Some(ThirdCountryReaction::SupportsInitiator),
        "the Great Power must have pledged support the same day"
    );
    assert_eq!(
        crisis_phase(&app, crisis_id),
        CrisisPhase::ResolvedPeacefully,
        "the same-day AI support must have been visible to the same day's P21-012 acceptance calculation"
    );
}

/// 要求テスト33: deadline当日は支持せず既存期限処理が優先される。
#[test]
fn deadline_day_takes_priority_over_ai_support() {
    let mut app = build_lightweight_ai_support_app(1);
    ally_with(&mut app, 1, 0);
    // 期限は開始日と同日(初日で既に到達済み)。
    let crisis_id = insert_demand_sent_crisis(&mut app, 0, 10, "1800/01/01");

    unpause_and_advance(&mut app, 1);

    assert_eq!(
        supporter_of(&app, crisis_id, 1),
        None,
        "no new support must be added once the deadline has been reached"
    );
    assert_eq!(
        crisis_phase(&app, crisis_id),
        CrisisPhase::Escalating,
        "the existing timeout processing must still resolve the crisis"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// 実データE2E(実6か国28州マップ) — 要求テスト37-48
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
                .expect("real-map save with P21-015 AI support state must validate")
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

    /// 要求テスト47: 実6か国データで自動判断対象がGreat Power 2か国以下になる
    /// (P21-014完了報告どおり、実6か国の構成は大国2・地域大国2・小国2)。
    #[test]
    fn real_map_has_at_most_two_great_powers() {
        let app = setup_app_in_playing(CountryId(0));
        let power_registry = app.world().resource::<CountryPowerRegistry>();

        let great_power_count = power_registry
            .ordered_country_ids()
            .iter()
            .filter(|&&id| {
                power_registry.get(id).map(|a| a.power_tier) == Some(PowerTier::GreatPower)
            })
            .count();

        assert!(
            great_power_count <= 2,
            "expected at most 2 great powers on the real 6-country map, got {great_power_count}"
        );
    }

    /// 要求テスト37: AI支持済みCrisisのSave→Loadで陣営と支持国が維持される。
    /// 要求テスト40: 期限切れ`DemandSent` CrisisのSave→Load後に支持しない。
    /// `third_party_reactions`をCountryId昇順のVecへ変換する
    /// (`CountryId`は`Ord`を派生していないため`BTreeMap`は使えない — `.0: usize`で
    /// 比較して安定した順序で比較できるようにする)。
    fn sorted_supporters(
        app: &App,
        crisis_id: DiplomaticCrisisId,
    ) -> Vec<(CountryId, ThirdCountryReaction)> {
        let mut v: Vec<(CountryId, ThirdCountryReaction)> = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .map(|c| c.third_party_reactions.clone().into_iter().collect())
            .unwrap_or_default();
        v.sort_by_key(|(id, _)| id.0);
        v
    }

    /// 要求テスト48: 実データで同一Saveから複数回実行して同じ支持結果になる(決定論性)。
    #[test]
    fn ai_support_survives_save_round_trip_and_is_deterministic() {
        let player = CountryId(0); // プレイヤーはGreat Power候補から除外される第三国
        let mut app = setup_app_in_playing(player);
        let initiator = CountryId(1);
        let target = CountryId(3);
        let crisis_id = create_claim_and_start_crisis(&mut app, initiator, target, StateId(9));

        for _ in 0..3 {
            advance_one_day(&mut app);
        }

        let supporters_before = sorted_supporters(&app, crisis_id);

        round_trip_through_save(&mut app);

        let supporters_after = sorted_supporters(&app, crisis_id);

        assert_eq!(
            supporters_before, supporters_after,
            "AI-pledged support must survive a save round trip unchanged"
        );

        // 決定論性: 同じ操作を最初からやり直しても同じ支持結果になる。
        let mut app2 = setup_app_in_playing(player);
        let crisis_id2 = create_claim_and_start_crisis(&mut app2, initiator, target, StateId(9));
        for _ in 0..3 {
            advance_one_day(&mut app2);
        }
        let supporters2 = sorted_supporters(&app2, crisis_id2);
        assert_eq!(
            supporters_before, supporters2,
            "the same scenario replayed from scratch must produce the same support outcome"
        );
    }

    /// 要求テスト38: Load後の最初の日次tickで重複追加されない。
    /// 要求テスト39: 未支持の有効なCrisisはLoad後の次の日次tickで評価される。
    #[test]
    fn load_does_not_duplicate_existing_support_and_still_evaluates_unsupported_crises() {
        let player = CountryId(0);
        let mut app = setup_app_in_playing(player);
        let initiator = CountryId(1);
        let target = CountryId(3);
        let crisis_id = create_claim_and_start_crisis(&mut app, initiator, target, StateId(9));

        for _ in 0..3 {
            advance_one_day(&mut app);
        }
        let count_before_load = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .map(|c| c.third_party_reactions.len())
            .unwrap_or(0);

        round_trip_through_save(&mut app);
        let count_immediately_after_load = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .map(|c| c.third_party_reactions.len())
            .unwrap_or(0);
        assert_eq!(
            count_before_load, count_immediately_after_load,
            "loading itself must not add or remove any support"
        );

        advance_one_day(&mut app);
        let count_after_one_more_day = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .map(|c| c.third_party_reactions.len())
            .unwrap_or(0);
        assert_eq!(
            count_before_load, count_after_one_more_day,
            "already-supporting Great Powers must not be re-added on the next tick after load"
        );
    }

    /// 要求テスト41: P21-013のプレイヤー手動支持が、AI支持ロジックと同居していても
    /// 引き続き動作する。
    /// 要求テスト42: P21-013のプレイヤー撤回が引き続き動作する。
    #[test]
    fn player_manual_support_and_withdrawal_still_work_alongside_ai_support() {
        let player = CountryId(5); // 第三国プレイヤー
        let mut app = setup_app_in_playing(player);
        let initiator = CountryId(1);
        let target = CountryId(3);
        let crisis_id = create_claim_and_start_crisis(&mut app, initiator, target, StateId(9));

        let current_date = app.world().resource::<GameDate>().clone();
        let countries = app.world().resource::<CountryRegistry>().clone_for_test();
        crisis_response::pledge_support(
            &mut app.world_mut().resource_mut::<CrisisRegistry>(),
            &countries,
            crisis_id,
            player,
            crisis_response::CrisisSupportSide::Target,
            &current_date,
        )
        .expect("player manual support must still succeed");

        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .third_party_reactions
                .get(&player),
            Some(&ThirdCountryReaction::SupportsTarget)
        );

        crisis_response::withdraw_support(
            &mut app.world_mut().resource_mut::<CrisisRegistry>(),
            crisis_id,
            player,
            &current_date,
        )
        .expect("player manual withdrawal must still succeed");

        assert!(
            !app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .third_party_reactions
                .contains_key(&player)
        );
    }
}
