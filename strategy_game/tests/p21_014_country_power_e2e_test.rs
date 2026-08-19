//! P21-014: 国家総合力評価・国家ランク基盤の統合テスト。
//!
//! 前半は`GameTimePlugin`(実際の`MonthChangedMessage`発行タイミング・Pause判定)だけを
//! 使った軽量Appで、New Game直後の初回評価・月次再評価・Pause中は再評価しないこと・
//! 同一月内の重複再評価防止を検証する。後半は実データ(`assets/data/*.ron`、6か国28州の
//! 本番マップ)を`AppPlugin`経由で読み込み、全国家評価・ランク人数・順位の決定論性・
//! save往復での再構築を一気通貫で検証する。

use bevy::prelude::*;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DailySimulationSet, GameDate, GamePaused, GameTimePlugin};
use strategy_game::building::data::BuildingRegistry;
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::power::rebuild_country_power_monthly;
use strategy_game::country::power::{CountryPowerRegistry, PowerTier, evaluate_country_power};
use strategy_game::country::{CountryData, CountryRegistry};
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::state::data::{StateData, StateRegistry};

// ─────────────────────────────────────────────────────────────────────────
// 軽量App: GameTimePlugin + rebuild_country_power_monthlyのみ
// ─────────────────────────────────────────────────────────────────────────

fn build_lightweight_power_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .init_state::<GameState>()
        .add_plugins(GameTimePlugin);

    app.insert_resource(CountryRegistry {
        countries: vec![
            CountryData {
                id: CountryId(0),
                ..CountryData::default()
            },
            CountryData {
                id: CountryId(1),
                ..CountryData::default()
            },
        ],
    });
    app.insert_resource(StateRegistry::build(vec![
        StateData {
            id: StateId(1),
            owner_country_id: CountryId(0),
            population: 100_000,
            ..Default::default()
        },
        StateData {
            id: StateId(2),
            owner_country_id: CountryId(1),
            population: 50_000,
            ..Default::default()
        },
    ]));
    app.insert_resource(MilitaryRegistry::default());
    app.insert_resource(BuildingRegistry::default());
    app.insert_resource(CountryPowerRegistry::default());

    app.add_systems(
        Update,
        rebuild_country_power_monthly
            .in_set(DailySimulationSet::UiUpdate)
            .run_if(in_state(GameState::Playing)),
    );

    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn tick_respecting_pause(app: &mut App, accumulator_delta: f64) {
    app.world_mut()
        .resource_mut::<GameDate>()
        .add_accumulator(accumulator_delta);
    app.update();
}

fn unpause_and_advance_days(app: &mut App, days: u32) {
    app.world_mut().resource_mut::<GamePaused>().0 = false;
    for _ in 0..days {
        tick_respecting_pause(app, 1.0);
    }
}

fn last_evaluated_date(app: &App) -> Option<String> {
    app.world()
        .resource::<CountryPowerRegistry>()
        .last_evaluated_date()
        .map(|s| s.to_string())
}

/// 要求テスト39: 通常Updateだけでは(月が変わらなければ)再評価しない。
/// 要求テスト40: `DayChangedMessage`だけ(=日は進むが月は変わらない)では再評価しない。
#[test]
fn no_reevaluation_without_a_month_change() {
    let mut app = build_lightweight_power_app();
    assert_eq!(
        last_evaluated_date(&app),
        None,
        "precondition: no evaluation has happened yet in this lightweight app (no OnEnter hook wired here)"
    );

    app.update();
    app.update();
    assert_eq!(
        last_evaluated_date(&app),
        None,
        "a bare Update with no day/month change must not trigger evaluation"
    );

    // 1800/01/01 -> 1800/01/02: 日は進むが月は変わらない。
    unpause_and_advance_days(&mut app, 1);
    assert_eq!(
        last_evaluated_date(&app),
        None,
        "a day change alone (no month change) must not trigger evaluation"
    );
}

/// 要求テスト38: `MonthChangedMessage`で再評価する。
/// 要求テスト43: 月次再評価後に順位変化が反映される。
#[test]
fn month_change_triggers_reevaluation_and_reflects_rank_changes() {
    let mut app = build_lightweight_power_app();

    // 1800/01/01 -> 1800/02/01 (31日進めて月を変える)。
    unpause_and_advance_days(&mut app, 31);
    assert!(
        last_evaluated_date(&app).is_some(),
        "a month change must trigger the first evaluation"
    );
    let rank0_before = app
        .world()
        .resource::<CountryPowerRegistry>()
        .get(CountryId(0))
        .unwrap()
        .world_rank;
    assert_eq!(
        rank0_before, 1,
        "country 0 starts with more population, so rank 1"
    );

    // country 1 の人口を大幅に増やし、次の月次評価で順位が逆転するはず。
    app.world_mut()
        .resource_mut::<StateRegistry>()
        .get_mut(StateId(2))
        .unwrap()
        .population = 10_000_000;

    // 1800/02/01 -> 1800/03/01
    unpause_and_advance_days(&mut app, 28);

    let rank1_after = app
        .world()
        .resource::<CountryPowerRegistry>()
        .get(CountryId(1))
        .unwrap()
        .world_rank;
    assert_eq!(
        rank1_after, 1,
        "after the monthly reevaluation, country 1's much larger population must flip it to rank 1"
    );
}

/// 要求テスト41: Pause中は再評価しない(`GamePaused`のままだと`advance_game_date`が
/// `MonthChangedMessage`自体を発行しないため)。
#[test]
fn no_reevaluation_while_paused() {
    let mut app = build_lightweight_power_app();
    assert!(
        app.world().resource::<GamePaused>().0,
        "starts paused by default"
    );

    for _ in 0..40 {
        tick_respecting_pause(&mut app, 1.0);
    }

    assert_eq!(
        last_evaluated_date(&app),
        None,
        "no evaluation must happen while the game stays paused, even across many Updates"
    );
}

/// 要求テスト42: 同一月に複数Updateしても重複再評価しない
/// (=`MonthChangedMessage`が発行されたその1フレームだけ再評価し、以降は同じ月の間
/// 再評価しない。ここでは評価結果の日付文字列が、月をまたがない限り変化しないことで
/// 間接的に確認する)。
#[test]
fn multiple_updates_within_the_same_month_do_not_reevaluate_repeatedly() {
    let mut app = build_lightweight_power_app();
    unpause_and_advance_days(&mut app, 31); // 1800/02/01へ到達、初回評価
    let date_after_month_change = last_evaluated_date(&app);
    assert!(date_after_month_change.is_some());

    // 同じ月の間に人口を変えても、月が変わるまでは再評価されないはず。
    app.world_mut()
        .resource_mut::<StateRegistry>()
        .get_mut(StateId(1))
        .unwrap()
        .population = 999_999_999;
    for _ in 0..10 {
        app.update(); // GamePausedはtickごとに個別管理していないため、ここは日次進行なし
    }

    let unchanged_raw = app
        .world()
        .resource::<CountryPowerRegistry>()
        .get(CountryId(0))
        .unwrap()
        .population_raw;
    assert_ne!(
        unchanged_raw, 999_999_999.0,
        "without a further day/month tick, no reevaluation should have picked up the new population"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// 実データE2E(実6か国28州マップ) — 要求テスト57-61
// ─────────────────────────────────────────────────────────────────────────

mod real_map_e2e {
    use super::*;
    use bevy::app::ScheduleRunnerPlugin;
    use strategy_game::app::AppPlugin;
    use strategy_game::building::BuildingPlugin;
    use strategy_game::country::PlayerCountry;
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
                .expect("real-map save with P21-014 world state must validate")
        };
        assert_eq!(
            apply_validated_save(app.world_mut(), validated),
            ApplyLoadOutcome::Success
        );
    }

    /// 要求テスト57: 実6か国28州で全国家が評価される
    /// (仕様書は「実7か国」を前提としていたが、`assets/data/countries.ron`の実測は
    /// 6か国だった。差異は完了報告に記載する — 推測で「7」に合わせない)。
    #[test]
    fn all_real_map_countries_are_evaluated_on_playing_entry() {
        let app = setup_app_in_playing(CountryId(0));

        let country_registry = app.world().resource::<CountryRegistry>();
        let power_registry = app.world().resource::<CountryPowerRegistry>();

        assert_eq!(
            country_registry.countries.len(),
            6,
            "the real countries.ron data must have exactly 6 countries (spec assumed 7)"
        );
        assert_eq!(power_registry.country_count(), 6);
        for country in &country_registry.countries {
            assert!(
                power_registry.get(country.id).is_some(),
                "country {:?} must have an assessment immediately after entering Playing",
                country.id
            );
        }
    }

    /// 要求テスト58(実測ベース): 実6か国では大国2・地域大国2・小国2になる
    /// (仕様書の例示表は7か国前提の「2/3/2」だが、`compute_tier_counts(6) ==
    /// (2, 2)`であり、実測の6か国データに対する正しい期待値はこちら)。
    #[test]
    fn real_map_tier_split_matches_the_formula_for_6_countries() {
        let app = setup_app_in_playing(CountryId(0));
        let power_registry = app.world().resource::<CountryPowerRegistry>();

        let mut great = 0;
        let mut regional = 0;
        let mut minor = 0;
        for &id in power_registry.ordered_country_ids() {
            match power_registry.get(id).unwrap().power_tier {
                PowerTier::GreatPower => great += 1,
                PowerTier::RegionalPower => regional += 1,
                PowerTier::MinorPower => minor += 1,
            }
        }

        assert_eq!((great, regional, minor), (2, 2, 2));
    }

    /// 要求テスト59: 実データで順位が決定論的(2回評価しても同じ順位)。
    /// 要求テスト60: save→load後に同じ評価・順位を再構築する。
    #[test]
    fn real_map_ranking_is_deterministic_and_survives_save_round_trip() {
        let mut app = setup_app_in_playing(CountryId(0));

        let before: Vec<(CountryId, usize, PowerTier)> = {
            let power_registry = app.world().resource::<CountryPowerRegistry>();
            power_registry
                .ordered_country_ids()
                .iter()
                .map(|&id| {
                    let a = power_registry.get(id).unwrap();
                    (id, a.world_rank, a.power_tier)
                })
                .collect()
        };

        // 決定論性: 同一状態からもう一度評価しても同じ結果。
        let recomputed = {
            let world = app.world();
            evaluate_country_power(
                world.resource::<CountryRegistry>(),
                world.resource::<StateRegistry>(),
                world.resource::<MilitaryRegistry>(),
                world.resource::<BuildingRegistry>(),
                "1800/01/01".to_string(),
            )
        };
        for &(id, rank, tier) in &before {
            let a = recomputed.get(id).unwrap();
            assert_eq!(a.world_rank, rank);
            assert_eq!(a.power_tier, tier);
        }

        // save→load後も同じ評価・順位が再構築される。
        round_trip_through_save(&mut app);
        let power_registry_after = app.world().resource::<CountryPowerRegistry>();
        for &(id, rank, tier) in &before {
            let a = power_registry_after.get(id).unwrap();
            assert_eq!(a.world_rank, rank, "rank must survive a save round trip");
            assert_eq!(a.power_tier, tier, "tier must survive a save round trip");
        }
    }

    /// 要求テスト61: 月次変化(実データ、AppPlugin全体経由)で再評価される。
    #[test]
    fn real_map_reevaluates_on_month_change() {
        let mut app = setup_app_in_playing(CountryId(0));
        let first_date = app
            .world()
            .resource::<CountryPowerRegistry>()
            .last_evaluated_date()
            .map(|s| s.to_string());
        assert!(first_date.is_some());

        for _ in 0..40 {
            advance_one_day(&mut app);
        }

        let second_date = app
            .world()
            .resource::<CountryPowerRegistry>()
            .last_evaluated_date()
            .map(|s| s.to_string());
        assert_ne!(
            first_date, second_date,
            "40 days (more than 1 month) must trigger at least one monthly reevaluation with an updated date"
        );
    }

    /// 要求テスト46(実データ側の裏付け): 国家ランクのフィールドをSaveGameV1へ
    /// 追加していないため、旧形式のsave(このテストが生成するsaveそのもの)にも
    /// 新規フィールドは存在しない。ロード後、CountryPowerRegistryは
    /// commit_load対象ではなく`apply_validated_save`内の明示的な再構築だけで
    /// 復元されることを確認する(=Save DTOに依存しない)。
    #[test]
    fn loading_a_save_without_any_power_related_fields_still_rebuilds_the_registry() {
        let mut app = setup_app_in_playing(CountryId(0));
        let country_count_before = app.world().resource::<CountryRegistry>().countries.len();

        // ロード前に意図的にCountryPowerRegistryを空へ差し替える
        // (「ロードのapply自体が再構築する」ことを確実に検証するため)。
        *app.world_mut().resource_mut::<CountryPowerRegistry>() = CountryPowerRegistry::default();
        assert_eq!(
            app.world()
                .resource::<CountryPowerRegistry>()
                .country_count(),
            0
        );

        round_trip_through_save(&mut app);

        assert_eq!(
            app.world()
                .resource::<CountryPowerRegistry>()
                .country_count(),
            country_count_before,
            "apply_validated_save must rebuild CountryPowerRegistry even though it isn't part of the save DTO"
        );
    }
}
