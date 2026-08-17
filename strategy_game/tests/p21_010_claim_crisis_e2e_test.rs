//! P21-010: 外交UIからClaim作成→Crisis開始→日次進行の統合テスト。
//!
//! 前半は`GameTimePlugin`(実際の`DayChangedMessage`発行タイミング・Pause判定)だけを
//! 使った軽量Appで`diplomacy::update::handle_daily_diplomacy`のCrisis日次進行
//! (`days_in_phase`インクリメントのみ、P21-010の接続範囲)を検証する。後半は実データ
//! (`assets/data/*.ron`、7か国28州の本番マップ)を`AppPlugin`経由で読み込み、
//! Claim作成→Crisis開始→日次進行→save往復までを一気通貫させる。

use bevy::prelude::*;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DailySimulationSet, GameDate, GamePaused, GameTimePlugin};
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::claims::{ClaimRegistry, ClaimSource};
use strategy_game::diplomacy::crisis::{CrisisPhase, CrisisRegistry};
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::diplomacy::update::handle_daily_diplomacy;
use strategy_game::localization::{CurrentLocale, Locale, TranslationCatalog, t};
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

fn insert_crisis(app: &mut App, phase: CrisisPhase) -> strategy_game::common::DiplomaticCrisisId {
    app.world_mut().resource_mut::<CrisisRegistry>().add_crisis(
        strategy_game::diplomacy::crisis::DiplomaticCrisis {
            id: strategy_game::common::DiplomaticCrisisId(0),
            initiator: CountryId(0),
            target: CountryId(1),
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            current_phase: phase,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: None,
            international_concern: 0.0,
            third_party_reactions: Default::default(),
        },
    )
}

fn days_in_phase(app: &App, id: strategy_game::common::DiplomaticCrisisId) -> u32 {
    app.world()
        .resource::<CrisisRegistry>()
        .crises
        .get(&id)
        .unwrap()
        .days_in_phase
}

/// テスト専用ヘルパー: `GamePaused`を変更せず(=一時停止状態を尊重したまま)、
/// accumulatorだけ加算してupdateする。`profiling::advance_one_day`は内部で強制的に
/// unpauseするため、Pause挙動自体を検証する本ファイルの目的には使えない。
fn tick_respecting_pause(app: &mut App, accumulator_delta: f64) {
    app.world_mut()
        .resource_mut::<GameDate>()
        .add_accumulator(accumulator_delta);
    app.update();
}

/// 要求テスト項目15: Crisisが日次で1回だけ進行する。
#[test]
fn crisis_advances_days_in_phase_once_per_real_day() {
    let mut app = build_lightweight_diplomacy_app();
    let id = insert_crisis(&mut app, CrisisPhase::Preparing);
    assert_eq!(days_in_phase(&app, id), 0);

    app.world_mut().resource_mut::<GamePaused>().0 = false;
    tick_respecting_pause(&mut app, 1.0);
    assert_eq!(days_in_phase(&app, id), 1);

    tick_respecting_pause(&mut app, 1.0);
    assert_eq!(days_in_phase(&app, id), 2);
}

/// 要求テスト項目16: Pause中は進行しない。
#[test]
fn crisis_does_not_advance_while_paused() {
    let mut app = build_lightweight_diplomacy_app();
    let id = insert_crisis(&mut app, CrisisPhase::Preparing);
    // GamePausedは既定でtrue(GameTimePlugin::build)。ここでは明示的にunpauseしない。
    assert!(app.world().resource::<GamePaused>().0);

    tick_respecting_pause(&mut app, 1.0);
    tick_respecting_pause(&mut app, 1.0);
    tick_respecting_pause(&mut app, 1.0);

    assert_eq!(
        days_in_phase(&app, id),
        0,
        "a paused game must never advance DayChangedMessage, so days_in_phase must stay 0"
    );
}

/// 要求テスト項目17: 同日複数updateで重複進行しない。
#[test]
fn crisis_does_not_double_advance_within_the_same_day() {
    let mut app = build_lightweight_diplomacy_app();
    let id = insert_crisis(&mut app, CrisisPhase::Preparing);

    app.world_mut().resource_mut::<GamePaused>().0 = false;
    // 1日分だけaccumulatorを進める(1回のDayChangedMessage発行)。
    tick_respecting_pause(&mut app, 1.0);
    assert_eq!(days_in_phase(&app, id), 1);

    // accumulatorを追加せずに複数回updateする(新しい日付は発生しない)。
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(
        days_in_phase(&app, id),
        1,
        "repeated Update ticks within the same in-game day must not advance days_in_phase again"
    );
}

/// 要求テスト項目18: terminal state(ResolvedPeacefully/WarStarted/Cancelled)は
/// 既存規則どおり日数を進めない。
#[test]
fn terminal_state_crises_never_advance_days_in_phase() {
    let mut app = build_lightweight_diplomacy_app();
    let resolved = insert_crisis(&mut app, CrisisPhase::ResolvedPeacefully);
    let war_started = insert_crisis(&mut app, CrisisPhase::WarStarted);
    let cancelled = insert_crisis(&mut app, CrisisPhase::Cancelled);
    let active = insert_crisis(&mut app, CrisisPhase::Negotiating);

    app.world_mut().resource_mut::<GamePaused>().0 = false;
    tick_respecting_pause(&mut app, 3.0);

    assert_eq!(days_in_phase(&app, resolved), 0);
    assert_eq!(days_in_phase(&app, war_started), 0);
    assert_eq!(days_in_phase(&app, cancelled), 0);
    assert_eq!(
        days_in_phase(&app, active),
        3,
        "a non-terminal crisis must still advance for comparison"
    );
}

/// 要求テスト項目20: JA/EN即時更新(エラーメッセージ・フェーズ表示キーの翻訳)。
#[test]
fn claim_and_crisis_translation_keys_resolve_differently_per_locale() {
    let catalog = TranslationCatalog::load().expect("embedded catalogs must parse");

    let ja_reason = t(&catalog, Locale::JaJp, "diplomacy_error.claim.own_state");
    let en_reason = t(&catalog, Locale::EnUs, "diplomacy_error.claim.own_state");
    assert_ne!(ja_reason, en_reason);
    assert!(!ja_reason.is_empty());
    assert!(!en_reason.is_empty());

    let ja_phase = t(
        &catalog,
        Locale::JaJp,
        "diplomacy_panel.crisis_phase_negotiating",
    );
    let en_phase = t(
        &catalog,
        Locale::EnUs,
        "diplomacy_panel.crisis_phase_negotiating",
    );
    assert_ne!(ja_phase, en_phase);
}

// ─────────────────────────────────────────────────────────────────────────
// 実データE2E(実7か国28州マップ)
// ─────────────────────────────────────────────────────────────────────────

mod real_map_e2e {
    use super::*;
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

    /// 要求テスト項目29: 実データ上で、Claim作成 → Crisis開始 → 日次進行 →
    /// save往復までを一気通貫で確認する。StateId(3)"Elfin Central"は
    /// `assets/data/states.ron`上でCountryId(1)所有の陸上州(実データ)。
    #[test]
    fn claim_to_crisis_to_daily_progression_e2e_with_real_map_data() {
        let mut app = setup_app_in_playing();

        // Claim作成(登録API経由、UIコマンドと同じ検証込みAPI)。
        let claim_id = {
            let world = app.world_mut();
            let country_registry = world.resource::<CountryRegistry>().clone_for_test();
            let state_registry = world.resource::<StateRegistry>().clone_for_test();
            assert_eq!(
                state_registry.get(StateId(3)).map(|s| s.owner_country_id),
                Some(CountryId(1)),
                "StateId(3) must be owned by CountryId(1) in the real map data"
            );
            let date_str = world.resource::<GameDate>().display();
            let mut claim_registry = world.resource_mut::<ClaimRegistry>();
            claim_registry
                .create_claim(
                    CountryId(0),
                    CountryId(1),
                    StateId(3),
                    date_str,
                    ClaimSource::Strategic,
                    &country_registry,
                    &state_registry,
                )
                .expect("valid real-map claim must succeed")
        };
        assert_eq!(app.world().resource::<ClaimRegistry>().claims.len(), 1);

        // Crisis開始(登録API経由)。
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
        assert_eq!(app.world().resource::<CrisisRegistry>().crises.len(), 1);
        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .days_in_phase,
            0
        );

        // 日次進行(本番のDailySimulationSet経由)。
        for _ in 0..5 {
            advance_one_day(&mut app);
        }

        let days_after = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .days_in_phase;
        assert_eq!(
            days_after, 5,
            "crisis must have advanced exactly 5 days via the real daily diplomacy system"
        );

        // save往復(要求テスト項目27相当: 実validate/apply経路を通す)。
        let save = snapshot_save(&app);
        let validated = {
            let context = validation_context(&app);
            validate_save_game_v1(save, &context)
                .expect("real-map save with claim/crisis must validate")
        };
        assert_eq!(
            apply_validated_save(app.world_mut(), validated),
            ApplyLoadOutcome::Success
        );

        let crisis_registry = app.world().resource::<CrisisRegistry>();
        assert_eq!(crisis_registry.crises.len(), 1);
        assert_eq!(
            crisis_registry
                .crises
                .get(&crisis_id)
                .unwrap()
                .days_in_phase,
            5,
            "days_in_phase must round-trip through save/load unchanged"
        );
        let claim_registry = app.world().resource::<ClaimRegistry>();
        assert_eq!(claim_registry.claims.len(), 1);

        // 要求テスト項目24: ロード直後は追加の日次進行が起きていないこと
        // (advance_one_dayを呼んでいないため、日数はロード直前と完全一致するはず)。
        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .days_in_phase,
            5,
            "loading must not silently advance an extra day"
        );
    }

    /// StateRegistry/CountryRegistryはCloneを実装しているため、借用衝突を避けるための
    /// テスト専用ヘルパー(本番コードには存在しない、テストの都合だけの複製)。
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
