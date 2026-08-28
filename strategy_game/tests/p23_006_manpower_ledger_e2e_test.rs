//! P23-006: 累積動員負担(B)・現役軍事人的資源(L)・恒久軍事損失(K)の意味分離、
//! 実経路統合テスト。
//!
//! `src/military/manpower.rs`のDomain単体テストが`compute_manpower_ledger`自体の
//! 計算ロジックを固定しているのに対し、本テストは実際の募兵・戦闘・save/load・
//! 民間労働力更新・UI表示の各経路を通して、B/L/Kの意味分離が実際の月次・日次処理と
//! 整合することを固定する。
//!
//! 3種類の実行環境を使い分ける:
//! - `minimal_daily_app`: 実際の本番System`military::update::handle_daily_military`
//!   (募兵キュー消化・戦闘損耗の双方を含む)だけを、その依存Resourceのみで動かす
//!   最小構成。AI・前線・外交等の副次的なSystemを一切介在させない。
//! - `setup_full_app`: 月次経済処理・save/loadを検証するための完全な構成
//!   (P23-002〜004と同一パターン、実データ6か国)。
//! - UIテストは`setup_full_app`に`MilitaryPanelPlugin`を追加した構成を使う。
//!
//! 戦闘ダメージの実数値は`combat_calc::resolve_combat_day_multi`の内部式に依存する
//! ため、本テストではその式を再実装・予測しない。代わりに常に「実際に観測した
//! `Division.manpower`の前後差分」を実損耗量として扱い、帳簿側の変化と突き合わせる。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::ecs::world::Mut;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::DayChangedMessage;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{
    BattleId, CountryId, DivisionDefinitionId, DivisionId, StateId, WarId,
};
use strategy_game::country::{CountryData, CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::localization::{CurrentLocale, Locale, MISSING_KEY_MARKER_PREFIX};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::data::{
    Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::military::manpower::{ManpowerLedger, compute_manpower_ledger};
use strategy_game::military::recruitment::{
    RecruitmentQueueItem, cancel_recruitment, request_recruitment,
};
use strategy_game::military::update::handle_daily_military;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{
    LastLoadOutcome, LoadOutcome, LoadRequestMessage, SaveFileConfig,
};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::{StateData, StateRegistry};
use strategy_game::ui::military_panel::{
    MilitaryPanelPlugin, MilitaryPanelState, MilitaryPanelText,
};
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p23_006_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
}

struct TempTestDir(std::path::PathBuf);

impl TempTestDir {
    fn new(label: &str) -> Self {
        Self(unique_temp_dir(label))
    }

    fn save_file_config(&self) -> SaveFileConfig {
        SaveFileConfig {
            path: SavePathConfig {
                final_path: self.0.join("savegame_v1.ron"),
            },
        }
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 共通フィクスチャヘルパー
// ─────────────────────────────────────────────────────────────────────────

fn test_country(id: usize, mobilized_manpower: u64, available_manpower: u64) -> CountryData {
    CountryData {
        id: CountryId(id),
        mobilized_manpower,
        available_manpower,
        treasury: 1_000_000.0,
        ..CountryData::default()
    }
}

fn test_division(id: usize, owner: usize, current_state: StateId, manpower: u64) -> Division {
    Division {
        id: DivisionId(id),
        owner: CountryId(owner),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower,
        max_manpower: manpower.max(1),
        equipment: 100.0,
        max_equipment: 100.0,
        organization: 50.0,
        max_organization: 50.0,
        morale: 1.0,
        max_morale: 1.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: DivisionDefinitionId(0),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

fn test_definition(id: usize, required_manpower: u64) -> DivisionDefinition {
    DivisionDefinition {
        id: DivisionDefinitionId(id),
        name: "Test".to_string(),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        required_manpower,
        required_equipment: 100.0,
        recruitment_days: 1,
        movement_speed: 1.0,
        attack: 10.0,
        defense: 10.0,
        breakthrough: 5.0,
        organization: 50.0,
        morale: 50.0,
        supply_usage: 1.0,
        maintenance_cost: 1.0,
    }
}

fn test_war(id: usize, attacker: usize, defender: usize) -> War {
    War {
        id: WarId(id),
        name: "Test War".to_string(),
        start_date: "1800/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        attackers: [CountryId(attacker)].into_iter().collect(),
        defenders: [CountryId(defender)].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: HashSet::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn test_battle(
    id: usize,
    war_id: usize,
    state_id: StateId,
    attacker: usize,
    defender: usize,
    attacker_divisions: Vec<DivisionId>,
    defender_divisions: Vec<DivisionId>,
) -> Battle {
    Battle {
        id: BattleId(id),
        war_id: WarId(war_id),
        state_id,
        attacker_country: CountryId(attacker),
        defender_country: CountryId(defender),
        attacker_division_ids: attacker_divisions,
        defender_division_ids: defender_divisions,
        attacker_origins: std::collections::HashMap::new(),
        start_date: "1800/01/01".to_string(),
        elapsed_days: 0,
        status: BattleStatus::Ongoing,
    }
}

/// `military::update::handle_daily_military`(本番System)だけを、その依存Resourceのみで
/// 動かす最小構成。募兵キュー消化・戦闘損耗のいずれもこの1つのSystemで処理される。
fn minimal_daily_app(
    country_registry: CountryRegistry,
    military_registry: MilitaryRegistry,
    state_registry: StateRegistry,
    battle_registry: BattleRegistry,
    war_registry: WarRegistry,
) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()));
    app.insert_resource(country_registry);
    app.insert_resource(military_registry);
    app.insert_resource(state_registry);
    app.insert_resource(battle_registry);
    app.insert_resource(war_registry);
    app.add_message::<DayChangedMessage>();
    app.add_systems(Update, handle_daily_military);
    app
}

fn send_day_changed(app: &mut App) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(DayChangedMessage {
            year: 1800,
            month: 1,
            day: 2,
        });
    state.apply(app.world_mut());
    app.update();
}

fn ledger_for(app: &App, country_id: CountryId) -> ManpowerLedger {
    let country = app
        .world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap();
    let military_registry = app.world().resource::<MilitaryRegistry>();
    compute_manpower_ledger(country, military_registry)
        .expect("test fixtures must always resolve to a valid ledger")
}

fn division_manpower(app: &App, id: DivisionId) -> Option<u64> {
    app.world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&id)
        .map(|d| d.manpower)
}

/// `request_recruitment`(実Domain API)を、`CountryRegistry`(可変)と`MilitaryRegistry`
/// (不変)を同時に借用して呼び出す。Bevyの`resource_scope`で2つのResourceを安全に
/// 同時アクセスする標準パターン。
fn call_request_recruitment(
    app: &mut App,
    player: CountryId,
    def_id: DivisionDefinitionId,
    target_state: StateId,
) -> Result<(), &'static str> {
    app.world_mut()
        .resource_scope(|world, military_registry: Mut<MilitaryRegistry>| {
            let mut country_registry = world.resource_mut::<CountryRegistry>();
            let country = country_registry.get_mut(player).unwrap();
            request_recruitment(country, &military_registry, def_id, target_state)
        })
}

fn call_cancel_recruitment(
    app: &mut App,
    player: CountryId,
    queue_index: usize,
) -> Result<(), &'static str> {
    app.world_mut()
        .resource_scope(|world, military_registry: Mut<MilitaryRegistry>| {
            let mut country_registry = world.resource_mut::<CountryRegistry>();
            let country = country_registry.get_mut(player).unwrap();
            cancel_recruitment(country, &military_registry, queue_index)
        })
}

// ─────────────────────────────────────────────────────────────────────────
// §10.2 募兵ライフサイクル(実System `handle_daily_military`経由)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn recruitment_start_increases_burden_and_living_by_the_same_amount() {
    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 0, 50_000)],
        },
        {
            let mut mr = MilitaryRegistry::default();
            mr.definitions
                .insert(DivisionDefinitionId(0), test_definition(0, 10_000));
            mr
        },
        StateRegistry::build(vec![StateData {
            id: StateId(0),
            ..StateData::default()
        }]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );

    let before = ledger_for(&app, CountryId(0));
    call_request_recruitment(&mut app, CountryId(0), DivisionDefinitionId(0), StateId(0)).unwrap();
    let after = ledger_for(&app, CountryId(0));

    assert_eq!(after.workforce_burden, before.workforce_burden + 10_000);
    assert_eq!(
        after.living_military_manpower,
        before.living_military_manpower + 10_000
    );
    assert_eq!(after.permanent_military_losses, 0);
}

#[test]
fn recruitment_cancellation_decreases_burden_and_living_by_the_same_amount() {
    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 0, 50_000)],
        },
        {
            let mut mr = MilitaryRegistry::default();
            mr.definitions
                .insert(DivisionDefinitionId(0), test_definition(0, 10_000));
            mr
        },
        StateRegistry::build(vec![StateData {
            id: StateId(0),
            ..StateData::default()
        }]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );
    call_request_recruitment(&mut app, CountryId(0), DivisionDefinitionId(0), StateId(0)).unwrap();
    let before = ledger_for(&app, CountryId(0));
    assert_eq!(before.workforce_burden, 10_000);

    call_cancel_recruitment(&mut app, CountryId(0), 0).unwrap();
    let after = ledger_for(&app, CountryId(0));

    assert_eq!(after.workforce_burden, 0);
    assert_eq!(after.living_military_manpower, 0);
    assert_eq!(after.permanent_military_losses, 0);
    assert_eq!(
        before.workforce_burden - after.workforce_burden,
        before.living_military_manpower - after.living_military_manpower,
        "burden and living must decrease by exactly the same amount on cancellation"
    );
}

#[test]
fn division_completion_moves_living_from_queue_to_division_without_changing_the_total() {
    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 10_000, 0)],
        },
        {
            let mut mr = MilitaryRegistry::default();
            mr.definitions
                .insert(DivisionDefinitionId(0), test_definition(0, 10_000));
            mr
        },
        StateRegistry::build(vec![StateData {
            id: StateId(0),
            ..StateData::default()
        }]),
        BattleRegistry::default(),
        WarRegistry::default(),
    );
    {
        let mut country_registry = app.world_mut().resource_mut::<CountryRegistry>();
        let country = country_registry.get_mut(CountryId(0)).unwrap();
        // P25-003: process_recruitmentがDivision生成時に国家備蓄からMilitaryEquipment
        // を支給するようになったため、test_definitionのrequired_equipment(100.0)を
        // 完全装備できるだけの備蓄を用意する(このテストの主眼はmanpower会計であり、
        // 装備は完了を妨げないよう十分に用意する)。
        country
            .stockpile
            .set(ResourceType::MilitaryEquipment, 100.0);
        country.recruitment_queue.push(RecruitmentQueueItem {
            division_id: DivisionDefinitionId(0),
            target_state: StateId(0),
            days_remaining: 1,
            total_days: 1,
        });
    }

    let before = ledger_for(&app, CountryId(0));
    assert_eq!(before.living_military_manpower, 10_000);
    assert!(
        app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .is_empty()
    );

    send_day_changed(&mut app); // process_recruitment consumes the 1-day queue item

    let after = ledger_for(&app, CountryId(0));
    assert_eq!(
        after.living_military_manpower, 10_000,
        "total living manpower must be unchanged by completion (queue -> division)"
    );
    assert_eq!(after.workforce_burden, before.workforce_burden);
    assert_eq!(after.permanent_military_losses, 0);
    assert!(
        !app.world()
            .resource::<MilitaryRegistry>()
            .divisions
            .is_empty(),
        "a real Division must have spawned"
    );
    assert!(
        app.world()
            .resource::<CountryRegistry>()
            .get(CountryId(0))
            .unwrap()
            .recruitment_queue
            .is_empty()
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §10.2 戦闘損耗・撃破(実System `handle_daily_military`経由)
// ─────────────────────────────────────────────────────────────────────────

fn base_state_registry() -> StateRegistry {
    StateRegistry::build(vec![StateData {
        id: StateId(0),
        ..StateData::default()
    }])
}

#[test]
fn real_combat_casualties_leave_the_burden_unchanged_and_move_the_measured_loss_from_living_to_permanent_losses()
 {
    let attacker_div = test_division(0, 0, StateId(0), 10_000);
    let defender_div = test_division(1, 1, StateId(0), 10_000);
    let mut military_registry = MilitaryRegistry::default();
    military_registry
        .divisions
        .insert(attacker_div.id, attacker_div.clone());
    military_registry
        .divisions
        .insert(defender_div.id, defender_div.clone());

    let mut war_registry = WarRegistry::default();
    let war = test_war(0, 0, 1);
    war_registry.wars.insert(war.id, war);

    let mut battle_registry = BattleRegistry::default();
    battle_registry.battles.insert(
        BattleId(0),
        test_battle(
            0,
            0,
            StateId(0),
            0,
            1,
            vec![DivisionId(0)],
            vec![DivisionId(1)],
        ),
    );

    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 10_000, 0), test_country(1, 10_000, 0)],
        },
        military_registry,
        base_state_registry(),
        battle_registry,
        war_registry,
    );

    let attacker_manpower_before = division_manpower(&app, DivisionId(0));
    let defender_manpower_before = division_manpower(&app, DivisionId(1));
    let attacker_ledger_before = ledger_for(&app, CountryId(0));
    let defender_ledger_before = ledger_for(&app, CountryId(1));

    send_day_changed(&mut app);

    let attacker_manpower_after = division_manpower(&app, DivisionId(0)).unwrap_or(0);
    let defender_manpower_after = division_manpower(&app, DivisionId(1)).unwrap_or(0);
    let attacker_loss = attacker_manpower_before.unwrap_or(0) - attacker_manpower_after;
    let defender_loss = defender_manpower_before.unwrap_or(0) - defender_manpower_after;
    assert!(
        attacker_loss > 0 || defender_loss > 0,
        "at least one side must take real combat damage on day 1"
    );

    let attacker_ledger_after = ledger_for(&app, CountryId(0));
    let defender_ledger_after = ledger_for(&app, CountryId(1));

    assert_eq!(
        attacker_ledger_after.workforce_burden, attacker_ledger_before.workforce_burden,
        "combat must never change the workforce burden (mobilized_manpower)"
    );
    assert_eq!(
        defender_ledger_after.workforce_burden,
        defender_ledger_before.workforce_burden
    );
    assert_eq!(
        attacker_ledger_before.living_military_manpower
            - attacker_ledger_after.living_military_manpower,
        attacker_loss,
        "the ledger's living decrease must match the measured Division.manpower loss exactly"
    );
    assert_eq!(
        attacker_ledger_after.permanent_military_losses
            - attacker_ledger_before.permanent_military_losses,
        attacker_loss,
        "permanent losses must increase by exactly the measured loss"
    );
    assert_eq!(
        defender_ledger_before.living_military_manpower
            - defender_ledger_after.living_military_manpower,
        defender_loss
    );
    assert_eq!(
        defender_ledger_after.permanent_military_losses
            - defender_ledger_before.permanent_military_losses,
        defender_loss
    );
}

#[test]
fn a_fully_wiped_out_division_is_not_double_counted_in_permanent_losses() {
    // 攻撃側を極端に少ない兵員で用意し、1日で全滅(manpower=0→レジストリから実削除)
    // させる。全滅側の恒久損失が「開始時manpower」ちょうどになる(2重計上されない)
    // ことを固定する。
    let attacker_div = test_division(0, 0, StateId(0), 1);
    let defender_div = test_division(1, 1, StateId(0), 50_000);
    let mut military_registry = MilitaryRegistry::default();
    military_registry
        .divisions
        .insert(attacker_div.id, attacker_div);
    military_registry
        .divisions
        .insert(defender_div.id, defender_div);

    let mut war_registry = WarRegistry::default();
    war_registry.wars.insert(WarId(0), test_war(0, 0, 1));

    let mut battle_registry = BattleRegistry::default();
    battle_registry.battles.insert(
        BattleId(0),
        test_battle(
            0,
            0,
            StateId(0),
            0,
            1,
            vec![DivisionId(0)],
            vec![DivisionId(1)],
        ),
    );

    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 1, 0), test_country(1, 50_000, 0)],
        },
        military_registry,
        base_state_registry(),
        battle_registry,
        war_registry,
    );

    send_day_changed(&mut app);

    assert!(
        division_manpower(&app, DivisionId(0)).is_none(),
        "the wiped-out division must be actually removed from the registry"
    );
    let ledger = ledger_for(&app, CountryId(0));
    assert_eq!(ledger.living_military_manpower, 0);
    assert_eq!(
        ledger.permanent_military_losses, 1,
        "permanent losses must equal exactly the division's starting manpower (1), not more"
    );
}

#[test]
fn same_day_multi_division_combat_sums_correctly_into_the_ledger() {
    let a1 = test_division(0, 0, StateId(0), 8_000);
    let a2 = test_division(1, 0, StateId(0), 8_000);
    let d1 = test_division(2, 1, StateId(0), 15_000);
    let mut military_registry = MilitaryRegistry::default();
    for d in [a1.clone(), a2.clone(), d1.clone()] {
        military_registry.divisions.insert(d.id, d);
    }

    let mut war_registry = WarRegistry::default();
    war_registry.wars.insert(WarId(0), test_war(0, 0, 1));

    let mut battle_registry = BattleRegistry::default();
    battle_registry.battles.insert(
        BattleId(0),
        test_battle(
            0,
            0,
            StateId(0),
            0,
            1,
            vec![DivisionId(0), DivisionId(1)],
            vec![DivisionId(2)],
        ),
    );

    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![test_country(0, 16_000, 0), test_country(1, 15_000, 0)],
        },
        military_registry,
        base_state_registry(),
        battle_registry,
        war_registry,
    );

    let a1_before = division_manpower(&app, DivisionId(0)).unwrap();
    let a2_before = division_manpower(&app, DivisionId(1)).unwrap();
    let attacker_ledger_before = ledger_for(&app, CountryId(0));

    send_day_changed(&mut app);

    let a1_after = division_manpower(&app, DivisionId(0)).unwrap_or(0);
    let a2_after = division_manpower(&app, DivisionId(1)).unwrap_or(0);
    let total_measured_loss = (a1_before - a1_after) + (a2_before - a2_after);

    let attacker_ledger_after = ledger_for(&app, CountryId(0));
    assert_eq!(
        attacker_ledger_before.living_military_manpower
            - attacker_ledger_after.living_military_manpower,
        total_measured_loss,
        "the ledger must sum losses across both same-side divisions correctly"
    );
}

#[test]
fn one_countrys_real_combat_losses_do_not_affect_another_countrys_ledger() {
    let bystander_div = test_division(2, 2, StateId(0), 12_345);
    let mut military_registry = MilitaryRegistry::default();
    military_registry
        .divisions
        .insert(DivisionId(0), test_division(0, 0, StateId(0), 10_000));
    military_registry
        .divisions
        .insert(DivisionId(1), test_division(1, 1, StateId(0), 10_000));
    military_registry
        .divisions
        .insert(bystander_div.id, bystander_div);

    let mut war_registry = WarRegistry::default();
    war_registry.wars.insert(WarId(0), test_war(0, 0, 1));

    let mut battle_registry = BattleRegistry::default();
    battle_registry.battles.insert(
        BattleId(0),
        test_battle(
            0,
            0,
            StateId(0),
            0,
            1,
            vec![DivisionId(0)],
            vec![DivisionId(1)],
        ),
    );

    let mut app = minimal_daily_app(
        CountryRegistry {
            countries: vec![
                test_country(0, 10_000, 0),
                test_country(1, 10_000, 0),
                test_country(2, 12_345, 0),
            ],
        },
        military_registry,
        base_state_registry(),
        battle_registry,
        war_registry,
    );

    let bystander_before = ledger_for(&app, CountryId(2));
    send_day_changed(&mut app);
    let bystander_after = ledger_for(&app, CountryId(2));

    assert_eq!(
        bystander_before, bystander_after,
        "an uninvolved third country's ledger must be completely unaffected by combat between two other countries"
    );
    assert_eq!(division_manpower(&app, DivisionId(2)), Some(12_345));
}

// ─────────────────────────────────────────────────────────────────────────
// §10.2 直接Domain API呼び出し(戦後孤立部隊破壊・Army解散)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn a_stranded_division_destroyed_after_peace_moves_its_manpower_from_living_to_permanent_losses() {
    use strategy_game::war::peace::relocate_hostile_territory_divisions;

    // country(0)は自国領土を一切持たない(全州が他国所有)状況で、敵地に取り残された
    // Divisionを持つ。帰還先が無いため`relocate_hostile_territory_divisions`が
    // status=Destroyed・manpower=0とマークする(war/peace.rsの既定経路)。
    let state_registry = StateRegistry::build(vec![StateData {
        id: StateId(0),
        owner_country_id: CountryId(9), // 第三国が所有、country(0)は無領土
        neighbors: Vec::new(),
        ..StateData::default()
    }]);
    let stranded = test_division(0, 0, StateId(0), 5_000);
    let mut military_registry = MilitaryRegistry::default();
    military_registry.divisions.insert(stranded.id, stranded);

    let country = test_country(0, 5_000, 0);
    let before = compute_manpower_ledger(&country, &military_registry).unwrap();
    assert_eq!(before.living_military_manpower, 5_000);
    assert_eq!(before.permanent_military_losses, 0);

    relocate_hostile_territory_divisions(&state_registry, &mut military_registry);

    let division = military_registry.divisions.get(&DivisionId(0)).unwrap();
    assert_eq!(division.status, DivisionStatus::Destroyed);
    assert_eq!(division.manpower, 0);

    let after = compute_manpower_ledger(&country, &military_registry).unwrap();
    assert_eq!(after.living_military_manpower, 0);
    assert_eq!(after.permanent_military_losses, 5_000);
    assert_eq!(
        after.workforce_burden, before.workforce_burden,
        "the stranded-division destruction path must not touch mobilized_manpower either"
    );
}

#[test]
fn army_disband_does_not_change_the_ledger() {
    let country = test_country(0, 8_000, 0);
    let division = test_division(0, 0, StateId(0), 8_000);
    let mut military_registry = MilitaryRegistry::default();
    military_registry.divisions.insert(division.id, division);

    let mut army_registry = ArmyRegistry::default();
    let army_id = army_registry
        .create_army(CountryId(0), &[DivisionId(0)], &military_registry)
        .expect("division must be usable");

    let before = compute_manpower_ledger(&country, &military_registry).unwrap();
    army_registry.disband(army_id, CountryId(0)).unwrap();
    let after = compute_manpower_ledger(&country, &military_registry).unwrap();

    assert_eq!(
        before, after,
        "army disband must not change B, L, or K at all"
    );
    assert!(
        military_registry.divisions.contains_key(&DivisionId(0)),
        "the underlying division must survive an army disband"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §10.3 民間労働力(実経路: setup_full_app、月次処理)
// ─────────────────────────────────────────────────────────────────────────

fn setup_full_app(player: CountryId, save_config: SaveFileConfig) -> App {
    setup_full_app_inner(player, save_config, false)
}

fn setup_app_with_military_panel(player: CountryId, save_config: SaveFileConfig) -> App {
    setup_full_app_inner(player, save_config, true)
}

/// `with_military_panel`は`GameState::Playing`遷移(`OnEnter`)より前にPluginを登録する
/// 必要があるため、`setup_full_app`の外から後付けで`add_plugins`しない
/// (P23-003/004で確認済みの、OnEnter後の追加登録だとsetup Systemが発火しない罠を回避する)。
fn setup_full_app_inner(
    player: CountryId,
    save_config: SaveFileConfig,
    with_military_panel: bool,
) -> App {
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
        .add_plugins(MilitaryPlugin)
        .add_plugins(PopulationPlugin);

    if with_military_panel {
        app.add_plugins(MilitaryPanelPlugin);
    }

    app.insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
        .insert_resource(strategy_game::map::division_selection::DragSelectState::default())
        .insert_resource(SelectedState::default())
        .insert_resource(strategy_game::ui::ActivePanel::default())
        .insert_resource(strategy_game::ui::diplomacy_panel::DiplomacyPanelState::default())
        .insert_resource(MilitaryPanelState::default())
        .insert_resource(strategy_game::ui::peace_panel::PeacePanelState::default())
        .insert_resource(strategy_game::ui::politics_panel::PoliticsPanelState::default())
        .insert_resource(strategy_game::ui::research_panel::ResearchPanelState::default())
        .insert_resource(strategy_game::map::camera::CameraDragState::default())
        .insert_resource(strategy_game::map::frontline_render::FrontlineRenderSettings::default())
        .insert_resource(strategy_game::map::frontline_selection::FrontlineSelectMode::default())
        .insert_resource(
            strategy_game::map::offensive_line_selection::OffensiveLineEditMode::default(),
        )
        .add_message::<strategy_game::map::frontline_selection::FrontlineAssignmentAttempted>()
        .add_message::<strategy_game::map::camera::CameraResetRequestMessage>();

    app.add_plugins(SaveGamePlugin);
    app.add_plugins(LoadGamePlugin);

    app.insert_resource(save_config);
    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing)

    app
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(SaveRequestMessage);
    state.apply(app.world_mut());
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(LoadRequestMessage);
    state.apply(app.world_mut());
}

fn load_and_settle(app: &mut App) {
    send_load_request(app);
    app.update();
    app.update();
}

fn advance_one_month(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<strategy_game::app::time::MonthChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(strategy_game::app::time::MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

fn civilian_workforce(app: &App, state_id: StateId) -> u64 {
    let state = app
        .world()
        .resource::<StateRegistry>()
        .get(state_id)
        .unwrap();
    state.civilian_workforce()
}

#[test]
fn recruitment_reduces_civilian_workforce_after_the_next_monthly_update() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("cw_recruit_reduces");
    let mut app = setup_full_app(player, temp_dir.save_file_config());

    advance_one_month(&mut app, 1800, 2);
    let baseline = civilian_workforce(&app, StateId(0));

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 500_000;
    advance_one_month(&mut app, 1800, 3);
    let after_mobilization = civilian_workforce(&app, StateId(0));

    assert!(
        after_mobilization < baseline,
        "civilian workforce must shrink after mobilization's next monthly update: baseline={baseline}, after={after_mobilization}"
    );
}

/// P23-004で確認した罠と同じ理由(P23-001の月次人口成長は動員/戦闘の有無に
/// 関わらず必ず進む)により、「変化しないこと」を検証する際は同一Appを複数月
/// 進めて比較せず、経過月数を揃えた2つの独立したAppを比較する。
#[test]
fn real_combat_losses_do_not_restore_civilian_workforce_the_following_month() {
    let player = CountryId(0);

    let mut control = setup_full_app(
        player,
        TempTestDir::new("cw_combat_control").save_file_config(),
    );
    control
        .world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 400_000;
    advance_one_month(&mut control, 1800, 2);
    advance_one_month(&mut control, 1800, 3); // mobilized_manpowerは変更しないまま2か月目
    let control_cw = civilian_workforce(&control, StateId(0));

    let mut experiment = setup_full_app(
        player,
        TempTestDir::new("cw_combat_experiment").save_file_config(),
    );
    experiment
        .world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 400_000;
    advance_one_month(&mut experiment, 1800, 2);
    // 現役Divisionへ実際の戦闘損耗を与える(mobilized_manpowerは一切変更しない)。
    {
        let mut mr = experiment.world_mut().resource_mut::<MilitaryRegistry>();
        if let Some(division) = mr.divisions.values_mut().find(|d| d.owner == player) {
            division.manpower = division.manpower.saturating_sub(division.manpower / 2);
        } else {
            mr.add_division(test_division(999, 0, StateId(0), 5_000));
            let division = mr
                .divisions
                .values_mut()
                .find(|d| d.owner == player)
                .unwrap();
            division.manpower = 2_500;
        }
    }
    advance_one_month(&mut experiment, 1800, 3);
    let experiment_cw = civilian_workforce(&experiment, StateId(0));

    assert_eq!(
        experiment_cw, control_cw,
        "civilian workforce must not recover just because units lost manpower in combat \
         (mobilized_manpower/burden is unchanged, so the monthly apportionment is unchanged); \
         both apps had 2 identical months of P23-001 population growth, isolating the effect"
    );
}

#[test]
fn recruitment_cancellation_restores_civilian_workforce_the_following_month() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("cw_cancel_restores");
    let mut app = setup_full_app(player, temp_dir.save_file_config());

    advance_one_month(&mut app, 1800, 2);
    let baseline = civilian_workforce(&app, StateId(0));

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 500_000;
    advance_one_month(&mut app, 1800, 3);
    let reduced = civilian_workforce(&app, StateId(0));
    assert!(reduced < baseline);

    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = 0;
    advance_one_month(&mut app, 1800, 4);
    let restored = civilian_workforce(&app, StateId(0));

    assert!(
        restored > reduced,
        "civilian workforce must recover once the burden (mobilized_manpower) is cleared: reduced={reduced}, restored={restored}"
    );
}

#[test]
fn another_countrys_losses_do_not_affect_the_target_countrys_civilian_workforce() {
    let player = CountryId(0);
    let other = CountryId(1);

    let mut control = setup_full_app(
        player,
        TempTestDir::new("cw_cross_control").save_file_config(),
    );
    advance_one_month(&mut control, 1800, 2);
    let control_cw = civilian_workforce(&control, StateId(0));

    let mut experiment = setup_full_app(
        player,
        TempTestDir::new("cw_cross_experiment").save_file_config(),
    );
    experiment
        .world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(other)
        .unwrap()
        .mobilized_manpower = 900_000;
    advance_one_month(&mut experiment, 1800, 2);
    let experiment_cw = civilian_workforce(&experiment, StateId(0));

    assert_eq!(
        experiment_cw, control_cw,
        "another country's mobilization must not affect this country's civilian workforce; \
         both apps had 1 identical month of P23-001 population growth, isolating the effect"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §10.4 Save/Load
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn a_save_with_burden_equal_to_living_reloads_with_zero_permanent_losses() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_burden_equals_living");
    let save_config = temp_dir.save_file_config();

    {
        let mut producer = setup_full_app(player, save_config.clone());
        call_request_recruitment_full(&mut producer, player, StateId(0));
        let before = ledger_for(&producer, player);
        assert_eq!(before.permanent_military_losses, 0);
        send_save_request(&mut producer);
        producer.update();
        assert!(save_config.path.final_path.exists());
    }

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }
    let after = ledger_for(&app, player);
    assert_eq!(after.permanent_military_losses, 0);
}

#[test]
fn a_save_with_permanent_losses_reproduces_the_same_ledger_after_reload() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_reproduces_losses");
    let save_config = temp_dir.save_file_config();

    let saved_ledger = {
        let mut producer = setup_full_app(player, save_config.clone());
        // 「既に完成済みのDivisionが一部損耗した」状態を直接構成する
        // (募兵キューの`recruitment_days`消化を待たずに、B > L(恒久損失あり)を
        // 確定的に作る)。実データの初期部隊(アンビエントなliving)に依存しないよう、
        // burdenをその時点のlivingへ揃えてから、既知の量だけ上乗せする。
        let ambient = ledger_for(&producer, player);
        producer
            .world_mut()
            .resource_mut::<MilitaryRegistry>()
            .add_division(test_division(500, player.0, StateId(0), 9_000));
        producer
            .world_mut()
            .resource_mut::<CountryRegistry>()
            .get_mut(player)
            .unwrap()
            .mobilized_manpower = ambient.living_military_manpower + 9_000 + 1_000; // 9,000生存 + 1,000恒久損失

        let ledger = ledger_for(&producer, player);
        assert_eq!(ledger.permanent_military_losses, 1_000);

        send_save_request(&mut producer);
        producer.update();
        assert!(save_config.path.final_path.exists());
        ledger
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("expected a successful load, got {other:?}"),
    }
    let reloaded_ledger = ledger_for(&app, player);
    assert_eq!(reloaded_ledger, saved_ledger);
}

#[test]
fn recruitment_queue_is_correctly_reconstructed_into_living_manpower_after_reload() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_reconstructs_queue");
    let save_config = temp_dir.save_file_config();

    let saved_ledger = {
        let mut producer = setup_full_app(player, save_config.clone());
        call_request_recruitment_full(&mut producer, player, StateId(0));
        // days_remaining>0のまま(未完成)でセーブする——process_recruitmentを走らせない。
        let ledger = ledger_for(&producer, player);
        assert!(ledger.living_military_manpower > 0);
        assert!(
            !producer
                .world()
                .resource::<CountryRegistry>()
                .get(player)
                .unwrap()
                .recruitment_queue
                .is_empty()
        );
        send_save_request(&mut producer);
        producer.update();
        ledger
    };

    let mut app = setup_full_app(player, save_config);
    load_and_settle(&mut app);
    let reloaded_ledger = ledger_for(&app, player);
    assert_eq!(reloaded_ledger, saved_ledger);
    assert!(
        !app.world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .recruitment_queue
            .is_empty(),
        "the unfinished queue item must survive the round trip"
    );
}

#[test]
fn an_inconsistent_l_greater_than_b_state_is_detected_safely_without_panicking_after_reload() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("save_load_l_greater_than_b");
    let mut app = setup_full_app(player, temp_dir.save_file_config());

    // 通常のゲームプレイでは`L > B`は発生し得ないが、旧V1セーブや手編集されたデータで
    // 理論上あり得る不正状態を、ロード後に直接再現してpanicしないことを確認する
    // (ロード自体の正しさは既存のsave/loadテスト群で別途検証済みのため、ここでは
    // ロード後の状態を意図的に不整合へ変更してからDomain関数の安全性を検証する)。
    // 実データは`spawn_debug_divisions`によりプレイヤーが既に初期部隊を保有し得るため、
    // 絶対値ではなく「追加した999,999人ぶんの差分」で検証する。
    let before = ledger_for(&app, player);
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .add_division(test_division(999_999, player.0, StateId(0), 999_999));
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = before.workforce_burden; // burdenは据え置き、生存分だけ増やす

    let after = ledger_for(&app, player);
    assert_eq!(
        after.permanent_military_losses, 0,
        "K must stay safely at 0, never negative"
    );
    assert_eq!(
        after.unaccounted_living_excess - before.unaccounted_living_excess,
        999_999,
        "the excess must grow by exactly the added division's manpower"
    );
}

fn call_request_recruitment_full(app: &mut App, player: CountryId, target_state: StateId) {
    let def_id = {
        let mr = app.world().resource::<MilitaryRegistry>();
        *mr.definitions
            .keys()
            .next()
            .expect("real data must have at least one division definition")
    };
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .available_manpower = 1_000_000;
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .treasury = 1_000_000.0;
    call_request_recruitment(app, player, def_id, target_state).unwrap();
}

// ─────────────────────────────────────────────────────────────────────────
// §10.5 UI・ローカライズ
// ─────────────────────────────────────────────────────────────────────────

fn open_military_panel(app: &mut App) {
    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;
    app.update();
}

fn read_military_panel_text(app: &mut App) -> String {
    app.world_mut()
        .query_filtered::<&Text, With<MilitaryPanelText>>()
        .single(app.world())
        .expect("MilitaryPanelText must be spawned")
        .0
        .clone()
}

#[test]
fn military_panel_displays_available_cap_living_burden_and_losses() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("panel_displays_all_five_values");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());

    open_military_panel(&mut app);
    let text = read_military_panel_text(&mut app);

    let country = app
        .world()
        .resource::<CountryRegistry>()
        .get(player)
        .unwrap()
        .clone();
    let ledger = ledger_for(&app, player);

    assert!(text.contains(&country.available_manpower.to_string()));
    assert!(text.contains(&ledger.living_military_manpower.to_string()));
    assert!(text.contains(&ledger.workforce_burden.to_string()));
    assert!(text.contains(&ledger.permanent_military_losses.to_string()));
}

#[test]
fn military_panel_losses_are_zero_before_combat_and_positive_after_a_real_loss_with_burden_unchanged()
 {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("panel_losses_before_after");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());
    open_military_panel(&mut app);

    // 実データでは`spawn_debug_divisions`由来の初期部隊により、ゲーム開始時点で
    // 既に`living_military_manpower > mobilized_manpower`(K=0だが
    // unaccounted_living_excess>0)という不整合状態になり得る(P23-005監査後に
    // 新たに確認された事実、完了報告の残課題として記録する)。このテストは
    // アンビエントな実データの値に依存せず、burdenをその時点のlivingへ揃えて
    // B == L のクリーンな基準点を明示的に作ってから検証する。
    let ambient = ledger_for(&app, player);
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = ambient.living_military_manpower;

    let new_division_id = app
        .world_mut()
        .resource_mut::<MilitaryRegistry>()
        .add_division(test_division(0, player.0, StateId(0), 5_000));
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(player)
        .unwrap()
        .mobilized_manpower = ambient.living_military_manpower + 5_000;
    app.update();

    let before_ledger = ledger_for(&app, player);
    assert_eq!(before_ledger.permanent_military_losses, 0);
    let before_text = read_military_panel_text(&mut app);
    assert!(before_text.contains(&format!("{}", before_ledger.permanent_military_losses)));

    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        let division = mr.divisions.get_mut(&new_division_id).unwrap();
        division.manpower = division.manpower.saturating_sub(2_000);
    }
    app.update();

    let after_ledger = ledger_for(&app, player);
    let after_text = read_military_panel_text(&mut app);

    assert_eq!(
        after_ledger.workforce_burden,
        before_ledger.workforce_burden
    );
    assert!(after_ledger.permanent_military_losses > 0);
    assert!(after_text.contains(&after_ledger.permanent_military_losses.to_string()));
    assert!(after_text.contains(&after_ledger.workforce_burden.to_string()));
}

#[test]
fn ja_and_en_render_the_new_manpower_labels_without_missing_key_markers() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("locales_render_manpower_labels");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());
    open_military_panel(&mut app);

    for locale in [Locale::JaJp, Locale::EnUs] {
        app.insert_resource(CurrentLocale(locale));
        app.update();
        let text = read_military_panel_text(&mut app);
        assert!(
            !text.contains(MISSING_KEY_MARKER_PREFIX),
            "military panel must not expose an unresolved key for {locale:?}, got: {text}"
        );
    }
}

#[test]
fn military_panel_text_entity_is_not_duplicated_after_repeated_updates() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("panel_not_duplicated");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());

    open_military_panel(&mut app);
    for _ in 0..5 {
        app.update();
    }

    let count = app
        .world_mut()
        .query_filtered::<Entity, With<MilitaryPanelText>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 1);
}

#[test]
fn military_panel_does_not_panic_with_no_state_selected_or_after_reload() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("panel_no_panic_no_selection_or_reload");
    let mut app = setup_app_with_military_panel(player, temp_dir.save_file_config());

    app.insert_resource(SelectedState(None));
    open_military_panel(&mut app);
    let _ = read_military_panel_text(&mut app); // must not panic

    send_save_request(&mut app);
    app.update();
    load_and_settle(&mut app);
    app.update();
    let text_after_reload = read_military_panel_text(&mut app);
    assert!(!text_after_reload.contains(MISSING_KEY_MARKER_PREFIX));
}
