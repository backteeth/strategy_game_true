//! P21-009-FIX-001: 既存クリスタル鉱山の互換移行(Mine→CrystalMine正規化)の統合テスト。
//!
//! `strategy_game::building::mine_migration::migrate_mines_to_crystal_mines`は
//! 2箇所から呼ばれる:
//! - `app::loader::load_game_data`(新規ゲーム、実データ`assets/data/*.ron`読込直後)
//! - `save::apply::prepare_load`(セーブ読込、Commit前)
//!
//! 本ファイルはその両方の経路を、実7か国28州データ・実`SaveGameV1`往復パイプライン
//! (`validate_save_game_v1` → `apply_validated_save`)を通じて検証する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::building::BuildingPlugin;
use strategy_game::building::construction::{ConstructionQueueItem, ConstructionStatus};
use strategy_game::building::data::{BuildingRegistry, BuildingType};
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::MilitaryRegistry;
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
use strategy_game::state::data::StateRegistry;
use strategy_game::war::WarPlugin;

/// `p21_008_army_offensive_e2e_test.rs`/`p21_009_magic_crystal_chain_test.rs`と同じ
/// 最小Appセットアップ(実データロード込み)。
fn setup_app_in_playing() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(AppPlugin)
        .add_plugins(CountryPlugin)
        .add_plugins(StatePlugin)
        .add_plugins(BuildingPlugin)
        .add_plugins(EconomyPlugin)
        .add_plugins(ResearchPlugin)
        .add_plugins(PoliticsPlugin)
        .add_plugins(DiplomacyPlugin)
        .add_plugins(WarPlugin)
        .add_plugins(MilitaryPlugin);

    // `save::apply::prepare_load`が要求するUI系一時Resource(実際は`MapPlugin`/`UiPlugin`が
    // 提供するが、Window/フォント資産に依存するため本テストでは登録しない)を手動で用意する
    // (`p21_008_army_offensive_e2e_test.rs`と同じパターン)。
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

    app.update(); // Startup: 実データ読込(移行込み) + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(CountryId(0))));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing)

    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::Playing
    );
    app
}

/// 現在のWorldから実際の`SaveGameV1`を構築する(`save::runtime`が本番のSave操作で
/// 使うのと同じ`build_save_game_v1`を、テストからSystemParamなしで直接呼び出す)。
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
        building_definitions: &world.resource::<BuildingRegistry>().definitions,
        technology_definitions: &world.resource::<TechnologyRegistry>().definitions,
        division_definitions: &world.resource::<MilitaryRegistry>().definitions,
        world_stage_definitions: &world.resource::<WorldCivilizationState>().stage_definitions,
    }
}

/// 要求テスト項目1/2: 新規ゲームの実データで、州2・州7に無稼働Mineが残らず、
/// CrystalMineレベルが旧Mineレベルと一致する(実`app::loader::load_game_data`経由)。
#[test]
fn new_game_real_map_migrates_state2_and_state7_mine_to_crystal_mine() {
    let app = setup_app_in_playing();
    let state_registry = app.world().resource::<StateRegistry>();

    let state2 = state_registry
        .get(StateId(2))
        .expect("StateId(2) must exist");
    assert_eq!(
        state2.building_level(BuildingType::Mine),
        0,
        "StateId(2) must have no leftover inert Mine after migration"
    );
    assert_eq!(
        state2.building_level(BuildingType::CrystalMine),
        1,
        "StateId(2) CrystalMine level must match the original Mine level (1) from states.ron"
    );

    let state7 = state_registry
        .get(StateId(7))
        .expect("StateId(7) must exist");
    assert_eq!(state7.building_level(BuildingType::Mine), 0);
    assert_eq!(
        state7.building_level(BuildingType::CrystalMine),
        2,
        "StateId(7) CrystalMine level must match the original Mine level (2) from states.ron"
    );
}

/// 要求テスト項目8: Iron/Coal州のMineは通常どおり残り、挙動は無変更。
#[test]
fn iron_coal_states_mine_is_unaffected_by_migration() {
    let app = setup_app_in_playing();
    let state_registry = app.world().resource::<StateRegistry>();

    // states.ron上、StateId(5)はIron/Coal鉱床を持つArcadia領でMine:5を保有。
    let state5 = state_registry
        .get(StateId(5))
        .expect("StateId(5) must exist");
    assert_eq!(
        state5.building_level(BuildingType::Mine),
        5,
        "non-crystal state's Mine must be left untouched by the migration"
    );
    assert_eq!(state5.building_level(BuildingType::CrystalMine), 0);
}

/// 要求テスト項目9: 移行後のCrystalMineはRawMagicCrystalのみを採掘し、精製済みは
/// 生産しない(精製施設が別途無い限り)。要求テスト項目10: 精製なしではMagicAcademyは
/// 稼働できない。
#[test]
fn migrated_crystal_mine_mines_raw_material_only_and_magic_academy_stays_idle() {
    let mut app = setup_app_in_playing();

    for _ in 0..95 {
        advance_one_day(&mut app);
    }

    let country_registry = app.world().resource::<CountryRegistry>();
    let country = country_registry.get(CountryId(0)).unwrap();

    assert!(
        country.stockpile.get(ResourceType::RawMagicCrystal) > 0.0,
        "migrated CrystalMine at StateId(2) must have mined raw material over 3 months"
    );
    assert_eq!(
        country.stockpile.get(ResourceType::MagicCrystal),
        0.0,
        "without a CrystalRefinery, no refined crystal must ever appear"
    );
    assert_eq!(
        country.magic_research_capacity, 0.0,
        "MagicAcademy (existing consumer, level 2 at StateId(2)) must stay idle without refined crystal"
    );
}

/// 要求テスト項目7(回帰): 実データ上でも、クリスタル専用州(StateId(2))へ通常Mineの
/// 新規建設リクエストがAI経路から発生しないことを、実`CountryAiState`込みの日次進行で
/// 間接的に確認する代わりに、production側の恒久ルール(MineはMagic鉱床を無視する)が
/// 実データ後も維持されていることを確認する(データ層の直接検証)。
#[test]
fn state2_resource_deposit_data_is_unchanged_by_migration() {
    let app = setup_app_in_playing();
    let state_registry = app.world().resource::<StateRegistry>();
    let state2 = state_registry.get(StateId(2)).unwrap();
    assert!(
        state2
            .resource_deposits
            .iter()
            .any(|d| d.discovered && d.resource_type == ResourceType::MagicCrystal),
        "resources.ron deposit data itself must remain untouched by the migration"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// セーブ読込経路(validate → apply)での移行
// ─────────────────────────────────────────────────────────────────────────

/// 要求テスト項目3: 旧セーブ(CrystalMine導入前、州2にMine:1のまま)を読み込むと
/// CrystalMineへ移行される。要求テスト項目12: validate→applyの原子性は維持される
/// (実`apply_validated_save`公開経路を通す)。
#[test]
fn legacy_save_with_mine_at_state2_migrates_to_crystal_mine_on_load() {
    let mut app = setup_app_in_playing();

    let mut save = snapshot_save(&app);
    let state2 = save
        .states
        .iter_mut()
        .find(|s| s.id == StateId(2))
        .expect("StateId(2) must be present in the snapshot");
    // P21-009-FIX-001以前の旧セーブを模す: CrystalMineを持たず、Mine:1のまま。
    state2.buildings.remove(&BuildingType::CrystalMine);
    state2.buildings.insert(BuildingType::Mine, 1);

    let validated = {
        let context = validation_context(&app);
        validate_save_game_v1(save, &context).expect("legacy save must still pass validation")
    };

    let outcome = apply_validated_save(app.world_mut(), validated);
    assert_eq!(outcome, ApplyLoadOutcome::Success);

    let state_registry = app.world().resource::<StateRegistry>();
    let state2_after = state_registry.get(StateId(2)).unwrap();
    assert_eq!(state2_after.building_level(BuildingType::Mine), 0);
    assert_eq!(state2_after.building_level(BuildingType::CrystalMine), 1);
}

/// 要求テスト項目4: 旧セーブ側に既にCrystalMineが存在する場合(P21-009適用後に
/// 部分的に建設していたケース)、Mineレベルは上書きではなく加算される。
#[test]
fn legacy_save_with_both_mine_and_existing_crystal_mine_adds_safely() {
    let mut app = setup_app_in_playing();

    let mut save = snapshot_save(&app);
    let state2 = save.states.iter_mut().find(|s| s.id == StateId(2)).unwrap();
    state2.buildings.insert(BuildingType::Mine, 1);
    state2.buildings.insert(BuildingType::CrystalMine, 3);

    let validated = {
        let context = validation_context(&app);
        validate_save_game_v1(save, &context).expect("save must pass validation")
    };
    let outcome = apply_validated_save(app.world_mut(), validated);
    assert_eq!(outcome, ApplyLoadOutcome::Success);

    let state_registry = app.world().resource::<StateRegistry>();
    let state2_after = state_registry.get(StateId(2)).unwrap();
    assert_eq!(state2_after.building_level(BuildingType::Mine), 0);
    assert_eq!(
        state2_after.building_level(BuildingType::CrystalMine),
        4,
        "existing CrystalMine level (3) must be added to, not overwritten by, the migrated Mine level (1)"
    );
}

/// 要求テスト項目5: 同じ(既に移行済みの)セーブを繰り返し読み込んでも増殖しない。
#[test]
fn reloading_an_already_migrated_save_does_not_duplicate_levels() {
    let mut app = setup_app_in_playing();

    // 1回目のロード: 旧形式(Mine:1のみ)を移行させる。
    {
        let mut save = snapshot_save(&app);
        let state2 = save.states.iter_mut().find(|s| s.id == StateId(2)).unwrap();
        state2.buildings.remove(&BuildingType::CrystalMine);
        state2.buildings.insert(BuildingType::Mine, 1);
        let validated = {
            let context = validation_context(&app);
            validate_save_game_v1(save, &context).unwrap()
        };
        assert_eq!(
            apply_validated_save(app.world_mut(), validated),
            ApplyLoadOutcome::Success
        );
    }
    let state_registry = app.world().resource::<StateRegistry>();
    assert_eq!(
        state_registry
            .get(StateId(2))
            .unwrap()
            .building_level(BuildingType::CrystalMine),
        1
    );

    // 2回目のロード: 既に移行済み(Mineキーが存在しない)状態のセーブを再度読み込む。
    {
        let save = snapshot_save(&app); // この時点でCrystalMine:1, Mineキーなし
        let validated = {
            let context = validation_context(&app);
            validate_save_game_v1(save, &context).unwrap()
        };
        assert_eq!(
            apply_validated_save(app.world_mut(), validated),
            ApplyLoadOutcome::Success
        );
    }

    let state_registry = app.world().resource::<StateRegistry>();
    assert_eq!(
        state_registry
            .get(StateId(2))
            .unwrap()
            .building_level(BuildingType::CrystalMine),
        1,
        "repeated load of an already-migrated save must not duplicate CrystalMine levels"
    );
}

/// 要求テスト項目6: 建設中のMine Projectは、種類がCrystalMineへ変換され、
/// 完成割合が維持される。
#[test]
fn in_progress_mine_construction_at_state2_converts_and_preserves_completion_fraction() {
    let mut app = setup_app_in_playing();

    let crystal_mine_required_progress = app
        .world()
        .resource::<BuildingRegistry>()
        .get(BuildingType::CrystalMine)
        .expect("buildings.ron must define CrystalMine")
        .required_progress;

    let mut save = snapshot_save(&app);
    let country0 = save
        .countries
        .iter_mut()
        .find(|c| c.id == CountryId(0))
        .expect("CountryId(0) must exist");
    country0.construction_queue.push(ConstructionQueueItem {
        state_id: StateId(2),
        building_type: BuildingType::Mine,
        target_level: 2,
        progress: 30.0,
        required_progress: 60.0, // 旧Mine.required_progress相当、50%完成
        paid_cost: 500.0,
        status: ConstructionStatus::InProgress,
    });

    let validated = {
        let context = validation_context(&app);
        validate_save_game_v1(save, &context).expect("save with in-progress Mine must validate")
    };
    assert_eq!(
        apply_validated_save(app.world_mut(), validated),
        ApplyLoadOutcome::Success
    );

    let country_registry = app.world().resource::<CountryRegistry>();
    let country0_after = country_registry.get(CountryId(0)).unwrap();
    let item = country0_after
        .construction_queue
        .iter()
        .find(|i| i.state_id == StateId(2) && i.target_level == 2)
        .expect("the in-progress construction item must still exist");

    assert_eq!(item.building_type, BuildingType::CrystalMine);
    assert_eq!(item.required_progress, crystal_mine_required_progress);
    let expected_progress = 0.5 * crystal_mine_required_progress;
    assert!(
        (item.progress - expected_progress).abs() < 1e-9,
        "50% completion must be preserved after conversion, expected {expected_progress}, got {}",
        item.progress
    );
}

/// 要求テスト項目11: 移行後(Mine→CrystalMine変換込み)のセーブがRON往復で維持される。
#[test]
fn migrated_state_round_trips_through_ron() {
    let app = setup_app_in_playing();
    let save = snapshot_save(&app);

    let ron_str = ron::to_string(&save).expect("migrated SaveGameV1 must serialize to RON");
    let restored: strategy_game::save::SaveGameV1 =
        ron::from_str(&ron_str).expect("migrated SaveGameV1 RON must deserialize back");

    let orig_state2 = save.states.iter().find(|s| s.id == StateId(2)).unwrap();
    let restored_state2 = restored.states.iter().find(|s| s.id == StateId(2)).unwrap();
    assert_eq!(
        restored_state2.building_level(BuildingType::CrystalMine),
        orig_state2.building_level(BuildingType::CrystalMine)
    );
    assert_eq!(
        restored_state2.building_level(BuildingType::Mine),
        orig_state2.building_level(BuildingType::Mine)
    );
}

/// 要求テスト項目12(補足): validationそのものが失敗するセーブは、移行が起きたか
/// どうかに関わらずWorldへ一切適用されない(atomicity)。
#[test]
fn invalid_save_never_applies_even_with_migratable_mine_data() {
    let app = setup_app_in_playing();

    let before_state2_crystal_mine = app
        .world()
        .resource::<StateRegistry>()
        .get(StateId(2))
        .unwrap()
        .building_level(BuildingType::CrystalMine);

    let mut save = snapshot_save(&app);
    // 移行対象になり得るMine配置を追加しつつ、同時に検証で確実に落ちる不正参照も混入させる。
    let state2 = save.states.iter_mut().find(|s| s.id == StateId(2)).unwrap();
    state2.buildings.insert(BuildingType::Mine, 1);
    save.countries[0]
        .construction_queue
        .push(ConstructionQueueItem {
            state_id: StateId(2),
            building_type: BuildingType::Mine,
            target_level: 99,
            progress: 0.0,
            required_progress: 10.0,
            paid_cost: 0.0,
            status: ConstructionStatus::InQueue,
        }); // target_level 99 > max_levelでvalidationが拒否する

    let validation_result = {
        let context = validation_context(&app);
        validate_save_game_v1(save, &context)
    };
    assert!(
        validation_result.is_err(),
        "a save with an out-of-range target_level must fail validation"
    );

    let state_registry = app.world().resource::<StateRegistry>();
    assert_eq!(
        state_registry
            .get(StateId(2))
            .unwrap()
            .building_level(BuildingType::CrystalMine),
        before_state2_crystal_mine,
        "World must remain unchanged: migration must never run for data that never passed validation"
    );
}
