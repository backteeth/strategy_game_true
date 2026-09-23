//! P26-005: 補給充足率の戦闘力反映、実App(実Bevy schedule)経由の検証。
//!
//! `calculate_combat_readiness_with_supply`/`resolve_combat_day_with_supply`/
//! `resolve_combat_day_multi_with_supply`の数式的正確性(3要素min合成・積合成でない
//! ことの悉皆確認・境界値・非有限値の拒否)は`src/military/combat_calc.rs`自身の
//! `#[cfg(test)]`内部テストモジュールで検証済み。
//!
//! `process_daily_battles`のpreflight拡張(補給充足率の妥当性検証)・攻撃側/防御側
//! 対称適用・国家間分離・下流(撃破可否・州支配権)への伝播は、private関数を直接
//! 呼び出せる`src/military/update.rs`自身の`#[cfg(test)]`内部テストモジュールで
//! 検証済み。
//!
//! 本ファイルは、実際にBevy Appへ登録されたSystem経由でのみ検証できる項目
//! (P26-004の日次補給処理が確定した「当日分」の値を同日の戦闘が実際に読むこと、
//! 月境界日の生産→消費→戦闘の順序、Save V1互換性・save/load分岐比較・決定性、
//! P26-004実測のOceanic供給不足がゲームプレイ上の戦闘力低下として実際に現れる
//! ことの実証)を担当する。
//!
//! `verification_logs/phase-26/p26-004/`・`tests/p25_007_equipment_combat_effectiveness_e2e_test.rs`が
//! 確立した実App harness(`setup_app_in_playing`等)を踏襲する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{BattleId, CountryId, DivisionId, StateId, WarId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::{ResourceType, military_supplies_units_to_value};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::data::{Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::dto::SaveGameV1;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};

// 実データ上のCountryId(`assets/data/countries.ron`)。P26-004報告書と同じ割当。
const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const DWARF: CountryId = CountryId(2);
const OCEANIC: CountryId = CountryId(3);
const FERROWYN: CountryId = CountryId(5);

// ─────────────────────────────────────────────────────────────────────────
// harness(P26-004/P25-007と同じ構成方針)
// ─────────────────────────────────────────────────────────────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p26_005_e2e_{label}_{}_{nanos}_{n}",
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

    fn final_path(&self) -> std::path::PathBuf {
        self.0.join("savegame_v1.ron")
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_app_in_playing(player: CountryId, save_config: SaveFileConfig) -> App {
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

    app.insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
        .insert_resource(strategy_game::map::division_selection::DragSelectState::default())
        .insert_resource(SelectedState::default())
        .insert_resource(strategy_game::ui::ActivePanel::default())
        .insert_resource(strategy_game::ui::diplomacy_panel::DiplomacyPanelState::default())
        .insert_resource(strategy_game::ui::military_panel::MilitaryPanelState::default())
        .insert_resource(strategy_game::ui::peace_panel::PeacePanelState::default())
        .insert_resource(strategy_game::ui::politics_panel::PoliticsPanelState::default())
        .insert_resource(strategy_game::ui::research_panel::ResearchPanelState::default())
        .insert_resource(strategy_game::map::camera::CameraDragState::default())
        .add_message::<strategy_game::map::camera::CameraResetRequestMessage>();

    app.add_plugins(SaveGamePlugin);
    app.add_plugins(LoadGamePlugin);

    app.insert_resource(save_config);
    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing): spawn_debug_divisions -> ブリッジ付与 -> 国力再評価

    app
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
    state.apply(app.world_mut());
    app.update();
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
    app.update();
}

fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

/// P26-004-PRE-002〜005と同じ簡略暦(1か月=27日[day2〜28]+月境界tick)で1か月進める。
fn advance_one_month(app: &mut App, current_month_index: u8) {
    for day in 2..=28u8 {
        tick_day(app, 1800, current_month_index, day);
    }
    tick_month_only(app, 1800, current_month_index.wrapping_add(1));
}

fn tick_month_only(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

/// spec §6/P26-004 §18: 月境界日には、実際の`advance_game_date`と同じく
/// `DayChangedMessage`と`MonthChangedMessage`が同一フレームで発行される。
fn tick_month_boundary_day(app: &mut App, year: i32, new_month: u8, new_day: u8) {
    let mut day_state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    day_state
        .get_mut(app.world_mut())
        .unwrap()
        .write(DayChangedMessage { year, month: new_month, day: new_day });
    day_state.apply(app.world_mut());

    let mut month_state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    month_state
        .get_mut(app.world_mut())
        .unwrap()
        .write(MonthChangedMessage { year, month: new_month });
    month_state.apply(app.world_mut());

    app.update();
}

fn stockpile_of(app: &App, country_id: CountryId, resource: ResourceType) -> f64 {
    app.world().resource::<CountryRegistry>().get(country_id).unwrap().stockpile.get(resource)
}

fn set_stockpile(app: &mut App, country_id: CountryId, resource: ResourceType, value: f64) {
    let mut registry = app.world_mut().resource_mut::<CountryRegistry>();
    registry.get_mut(country_id).unwrap().stockpile.set(resource, value);
}

fn fulfillment_of(app: &App, division_id: DivisionId) -> f64 {
    app.world().resource::<MilitaryRegistry>().supply_fulfillment_of(division_id)
}

fn division_ids_of(app: &App, country_id: CountryId) -> Vec<DivisionId> {
    let mut ids: Vec<DivisionId> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == country_id)
        .map(|d| d.id)
        .collect();
    ids.sort_by_key(|id| id.0);
    ids
}

fn get_division(app: &App, id: DivisionId) -> Option<Division> {
    app.world().resource::<MilitaryRegistry>().divisions.get(&id).cloned()
}

fn total_manpower(app: &App, ids: &[DivisionId]) -> u64 {
    ids.iter().map(|id| get_division(app, *id).unwrap().manpower).sum()
}

/// 実マップ上の州(terrain補正は常にPLAINなので、combatの数値には影響しない)。
const TEST_STATE_ID: StateId = StateId(999_999);

/// New Gameの満編制Infantryと同じ規格の追加Divisionを、TEST_STATE_IDへ直接World経由で
/// 挿入する(募兵パイプラインを経由しない、実国家の既存Divisionへ影響を与えない
/// テスト専用の高速なシナリオ構築、P25-007と同じ方式)。
#[allow(clippy::too_many_arguments)]
fn spawn_division(
    app: &mut App,
    owner: CountryId,
    manpower: u64,
    max_manpower: u64,
    equipment: f64,
    max_equipment: f64,
    atk: i32,
    def: i32,
) -> DivisionId {
    let division = Division {
        id: DivisionId(0),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: TEST_STATE_ID,
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower,
        max_manpower,
        equipment,
        max_equipment,
        organization: 100.0,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Fighting,
        def_id: strategy_game::common::DivisionDefinitionId(0),
        attack_power: atk,
        defense_power: def,
        combat_id: None,
    };
    app.world_mut().resource_mut::<MilitaryRegistry>().add_division(division)
}

fn create_war(app: &mut App, attacker: CountryId, defender: CountryId) -> WarId {
    let war = War {
        id: WarId(0),
        name: "P26-005 Test War".to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1800/01/01".to_string(),
        end_date: None,
        duration_days: 0,
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
    };
    app.world_mut().resource_mut::<WarRegistry>().add_war(war)
}

/// P26-005の攻撃側Divisionは全て`TEST_STATE_ID`(実在しない合成州)上に直接
/// 生成されるため、そのまま出撃元(origin)としては使えない――新しい出撃元
/// 検証は実在しない州を無効と判定し、代替探索も`TEST_STATE_ID`に隣接州が
/// 存在しないため即失敗し、Divisionが誤って消滅する。本ファイルの戦闘結果
/// (与ダメージ・組織率・Save一致性)検証は出撃元の地理と無関係なので、実マップ
/// 上で当該国が実際に支配する州を出撃元として渡す。
fn state_owned_by(app: &App, owner: CountryId) -> StateId {
    app.world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .find(|s| s.owner_country_id == owner)
        .map(|s| s.id)
        .expect("real map must have at least one state owned by this country")
}

#[allow(clippy::too_many_arguments)]
fn create_battle(
    app: &mut App,
    war_id: WarId,
    attacker_country: CountryId,
    defender_country: CountryId,
    state_id: StateId,
    attacker_ids: Vec<DivisionId>,
    defender_ids: Vec<DivisionId>,
) -> BattleId {
    let attacker_origin = state_owned_by(app, attacker_country);
    let attacker_origins = attacker_ids.iter().map(|id| (*id, attacker_origin)).collect();
    let battle = Battle {
        id: BattleId(0),
        war_id,
        state_id,
        attacker_country,
        defender_country,
        attacker_division_ids: attacker_ids.clone(),
        defender_division_ids: defender_ids.clone(),
        attacker_origins,
        start_date: "1800/01/01".to_string(),
        elapsed_days: 0,
        status: BattleStatus::Ongoing,
    };
    let battle_id = app.world_mut().resource_mut::<BattleRegistry>().start_battle(battle).unwrap();
    let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
    for id in attacker_ids.iter().chain(defender_ids.iter()) {
        if let Some(d) = mr.divisions.get_mut(id) {
            d.combat_id = Some(battle_id);
        }
    }
    battle_id
}

fn rewrite_save_file(path: &std::path::Path, edit: impl FnOnce(&mut SaveGameV1)) {
    let contents = std::fs::read_to_string(path).expect("read save file");
    let mut save: SaveGameV1 = ron::from_str(&contents).expect("parse SaveGameV1");
    edit(&mut save);
    let ron_str = ron::to_string(&save).expect("serialize SaveGameV1");
    std::fs::write(path, ron_str).expect("write save file");
}

// ─────────────────────────────────────────────────────────────────────────
// Category C: 実System統合(同日の日次補給結果を戦闘が読むこと)
// ─────────────────────────────────────────────────────────────────────────

/// spec §14: 実App経由で、DWARFの備蓄不足がP26-004の日次消費・配分を経て
/// `supply_fulfillment`を1未満へ落とし、その値が**同日**の戦闘(ELFINへの攻撃)へ
/// 実際に反映される(=防御側[ELFIN]への被害が、備蓄十分な対照状態より小さくなる)。
/// DivisionDefinition・manpower・equipment・status・敵Division・日付は両シナリオで
/// 完全に同一とし、差分をMilitarySupplies備蓄量だけに限定する(spec §14)。
#[test]
fn test_c1_real_daily_supply_shortage_reduces_same_day_combat_damage_dealt() {
    fn run_scenario(label: &str, dwarf_supplies: f64) -> u64 {
        let dir = TempTestDir::new(label);
        let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
        set_stockpile(&mut app, DWARF, ResourceType::MilitarySupplies, dwarf_supplies);

        let attacker_ids = division_ids_of(&app, DWARF);
        let defender_ids = division_ids_of(&app, ELFIN);
        assert!(!attacker_ids.is_empty() && !defender_ids.is_empty());

        let war_id = create_war(&mut app, DWARF, ELFIN);
        create_battle(&mut app, war_id, DWARF, ELFIN, TEST_STATE_ID, attacker_ids, defender_ids.clone());

        let defender_before = total_manpower(&app, &defender_ids);
        // DayChangedMessage一発で、同一フレーム内に日次補給消費(Economy Set)→
        // 戦闘(MilitaryAction Set)の両方が実行される(P26-004 §4/spec §9)。
        tick_day(&mut app, 1800, 1, 2);
        let defender_after = total_manpower(&app, &defender_ids);
        defender_before - defender_after
    }

    // 十分な備蓄(需要を大きく上回る)の対照シナリオ
    let loss_with_full_supply = run_scenario("c1_full", 1_000_000.0);
    // 備蓄0のシナリオ(同日中に消費すべき備蓄が全くない -> fulfillment 0.0)
    let loss_with_zero_supply = run_scenario("c1_zero", 0.0);

    assert!(
        loss_with_zero_supply < loss_with_full_supply,
        "DWARFの補給不足(実際の日次消費・配分の結果)は、同日のELFINへの攻撃で\
         与える被害を減らすはず: full={loss_with_full_supply} zero={loss_with_zero_supply}"
    );
}

/// spec §9/P26-004 §18: 月境界日には月次生産が同日の日次消費より先に確定する。
/// 単独では当日の需要を賄えない水準まで意図的に減らした備蓄でも、月境界日の
/// 生産により当日中にfulfillment=1.0まで回復し、**その回復後の値**を同日の戦闘が
/// 読むことを実証する(=月次生産前の古い[低い]fulfillmentが誤って使われない)。
#[test]
fn test_c2_month_boundary_production_before_same_day_combat_reads_post_production_fulfillment() {
    let dir = TempTestDir::new("c2");
    let mut app = setup_app_in_playing(ARCADIA, dir.save_file_config());
    // 生産が確実に軌道に乗るまで数か月分進める(P26-004 test_f1と同じ技法:
    // Arcadiaは月6時点で十分正の生産があることが確認済み)。
    for m in 1..=6u8 {
        advance_one_month(&mut app, m);
    }
    // 当日の需要(1.0)をわずかに下回る水準まで意図的に減らす(P26-004 test_f1と同一)。
    let short_of_demand = military_supplies_units_to_value(999_999);
    set_stockpile(&mut app, ARCADIA, ResourceType::MilitarySupplies, short_of_demand);

    let attacker_ids = division_ids_of(&app, ARCADIA);
    let defender_ids = division_ids_of(&app, ELFIN);
    let war_id = create_war(&mut app, ARCADIA, ELFIN);
    create_battle(&mut app, war_id, ARCADIA, ELFIN, TEST_STATE_ID, attacker_ids.clone(), defender_ids.clone());

    let defender_before = total_manpower(&app, &defender_ids);
    // 月境界日: 月次生産(Economy Set前段)→ 日次消費(Economy Set後段)→ 戦闘
    // (MilitaryAction Set)が同一フレームで発行される。
    tick_month_boundary_day(&mut app, 1800, 8, 1);

    // 生産後、Arcadiaの師団は完全充足(1.0)になっているはず(P26-004 test_f1と同じ結果)。
    assert_eq!(fulfillment_of(&app, attacker_ids[0]), 1.0, "月次生産後は完全充足のはず");

    let defender_after = total_manpower(&app, &defender_ids);
    let actual_loss = defender_before - defender_after;

    // 対照: 同じ月6進行後の状態から、月境界(生産)を挟まず備蓄を明示的に0へ落として
    // 通常の日次ティックだけを行った場合の被害(=もし戦闘が生産前の低い値を
    // 読んでいたらこの水準になっていたはず)。
    let dir_ref = TempTestDir::new("c2_ref_zero");
    let mut app_ref = setup_app_in_playing(ARCADIA, dir_ref.save_file_config());
    for m in 1..=6u8 {
        advance_one_month(&mut app_ref, m);
    }
    set_stockpile(&mut app_ref, ARCADIA, ResourceType::MilitarySupplies, 0.0);
    let attacker_ids_ref = division_ids_of(&app_ref, ARCADIA);
    let defender_ids_ref = division_ids_of(&app_ref, ELFIN);
    let war_ref = create_war(&mut app_ref, ARCADIA, ELFIN);
    create_battle(&mut app_ref, war_ref, ARCADIA, ELFIN, TEST_STATE_ID, attacker_ids_ref, defender_ids_ref.clone());
    let ref_before = total_manpower(&app_ref, &defender_ids_ref);
    // 月次生産を伴わない通常の日次ティック(月境界ではない、月中の1日)。
    tick_day(&mut app_ref, 1800, 7, 2);
    let ref_after = total_manpower(&app_ref, &defender_ids_ref);
    let zero_supply_loss = ref_before - ref_after;

    assert!(
        actual_loss > zero_supply_loss,
        "月次生産後の完全充足を同日の戦闘が読んでいるはず(生産前の低い値を\
         読んでいたら被害はzero_supply_loss相当になっていたはず): actual={actual_loss} \
         zero_supply_reference={zero_supply_loss}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Category D: Save/load・決定性
// ─────────────────────────────────────────────────────────────────────────

/// spec §17: 部分補給状態(補給率0.0..1.0)の戦闘中save→loadで、load直後の値が
/// 保存前と一致し、その後の戦闘結果も継続run(save/loadを挟まない)と一致する。
#[test]
fn test_d1_partial_supply_combat_state_round_trips_and_continues_identically() {
    fn scenario(app: &mut App, split_with_save_load: bool) -> (u64, f64) {
        let attacker_id = spawn_division(app, DWARF, 10_000, 10_000, 100.0, 100.0, 10, 10);
        let defender_id = spawn_division(app, ELFIN, 10_000, 10_000, 100.0, 100.0, 10, 15);
        app.world_mut()
            .resource_mut::<MilitaryRegistry>()
            .division_supply_fulfillment
            .insert(attacker_id, 0.42);
        let war_id = create_war(app, DWARF, ELFIN);
        create_battle(app, war_id, DWARF, ELFIN, TEST_STATE_ID, vec![attacker_id], vec![defender_id]);

        if split_with_save_load {
            send_save_request(app);
            send_load_request(app);
            // load直後の値がdefault(1.0)へ誤って戻っていないことを確認(spec §17)。
            assert_eq!(fulfillment_of(app, attacker_id), 0.42, "load直後は保存前の値を維持するはず");
        }

        tick_day(app, 1800, 1, 2);
        let defender = get_division(app, defender_id).unwrap();
        (defender.manpower, fulfillment_of(app, attacker_id))
    }

    let dir_a = TempTestDir::new("d1_a");
    let mut app_a = setup_app_in_playing(DWARF, dir_a.save_file_config());
    let (manpower_a, fulfillment_a) = scenario(&mut app_a, false);

    let dir_b = TempTestDir::new("d1_b");
    let mut app_b = setup_app_in_playing(DWARF, dir_b.save_file_config());
    let (manpower_b, fulfillment_b) = scenario(&mut app_b, true);

    assert_eq!(manpower_a, manpower_b, "save/load分岐は継続runと同じ戦闘結果になるはず");
    assert_eq!(fulfillment_a, fulfillment_b);
}

/// spec §17: 補給率0.0の戦闘中save→loadでも、値が正確に往復する。
#[test]
fn test_d2_zero_supply_combat_state_round_trips_through_save_load() {
    let dir = TempTestDir::new("d2");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let attacker_id = spawn_division(&mut app, DWARF, 10_000, 10_000, 100.0, 100.0, 10, 10);
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .division_supply_fulfillment
        .insert(attacker_id, 0.0);

    send_save_request(&mut app);
    send_load_request(&mut app);
    assert_eq!(fulfillment_of(&app, attacker_id), 0.0);
}

/// spec §17: 2回連続save→loadでも補給充足率が安定する。
#[test]
fn test_d3_double_save_load_round_trip_is_stable() {
    let dir = TempTestDir::new("d3");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let attacker_id = spawn_division(&mut app, DWARF, 10_000, 10_000, 100.0, 100.0, 10, 10);
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .division_supply_fulfillment
        .insert(attacker_id, 0.73);

    send_save_request(&mut app);
    send_load_request(&mut app);
    let after_first = fulfillment_of(&app, attacker_id);
    send_save_request(&mut app);
    send_load_request(&mut app);
    let after_second = fulfillment_of(&app, attacker_id);

    assert_eq!(after_first, 0.73);
    assert_eq!(after_second, 0.73);
}

/// spec §17: 補給充足率フィールドが欠落した旧SaveをloadしたDivisionは、その日の
/// 戦闘でdefault 1.0として扱われる(=完全充足の対照シナリオと同じ被害になる)。
#[test]
fn test_d4_old_save_without_fulfillment_field_defaults_combat_to_full_supply() {
    let dir = TempTestDir::new("d4");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    let attacker_id = spawn_division(&mut app, DWARF, 10_000, 10_000, 100.0, 100.0, 10, 10);
    let defender_id = spawn_division(&mut app, ELFIN, 10_000, 10_000, 100.0, 100.0, 10, 10);

    send_save_request(&mut app);
    rewrite_save_file(&dir.final_path(), |save| {
        save.military.division_supply_fulfillment.clear();
    });
    send_load_request(&mut app);
    assert_eq!(fulfillment_of(&app, attacker_id), 1.0, "旧Save(フィールド欠落)はdefault 1.0のはず");

    let war_id = create_war(&mut app, DWARF, ELFIN);
    create_battle(&mut app, war_id, DWARF, ELFIN, TEST_STATE_ID, vec![attacker_id], vec![defender_id]);
    let before = get_division(&app, defender_id).unwrap().manpower;
    tick_day(&mut app, 1800, 1, 2);
    let after = get_division(&app, defender_id).unwrap().manpower;
    let loss_from_old_save = before - after;

    // 対照: save/loadを挟まない、明示的に補給率1.0の同一シナリオ。
    let dir_ref = TempTestDir::new("d4_ref");
    let mut app_ref = setup_app_in_playing(DWARF, dir_ref.save_file_config());
    let attacker_ref = spawn_division(&mut app_ref, DWARF, 10_000, 10_000, 100.0, 100.0, 10, 10);
    let defender_ref = spawn_division(&mut app_ref, ELFIN, 10_000, 10_000, 100.0, 100.0, 10, 10);
    let war_ref = create_war(&mut app_ref, DWARF, ELFIN);
    create_battle(&mut app_ref, war_ref, DWARF, ELFIN, TEST_STATE_ID, vec![attacker_ref], vec![defender_ref]);
    let ref_before = get_division(&app_ref, defender_ref).unwrap().manpower;
    tick_day(&mut app_ref, 1800, 1, 2);
    let ref_after = get_division(&app_ref, defender_ref).unwrap().manpower;
    let reference_full_supply_loss = ref_before - ref_after;

    assert_eq!(loss_from_old_save, reference_full_supply_loss);
}

/// spec §19: 独立した2つのAppインスタンス(補給率0.5を含む)で、戦闘結果が
/// bit-for-bit一致する。
#[test]
fn test_d5_two_independent_app_instances_produce_identical_combat_results_with_partial_supply() {
    fn run(label: &str) -> (u64, f32, u64, f32) {
        let dir = TempTestDir::new(label);
        let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
        let attacker_id = spawn_division(&mut app, DWARF, 8_000, 10_000, 60.0, 100.0, 12, 8);
        let defender_id = spawn_division(&mut app, ELFIN, 6_000, 10_000, 90.0, 100.0, 9, 14);
        app.world_mut()
            .resource_mut::<MilitaryRegistry>()
            .division_supply_fulfillment
            .insert(attacker_id, 0.5);
        app.world_mut()
            .resource_mut::<MilitaryRegistry>()
            .division_supply_fulfillment
            .insert(defender_id, 0.8);
        let war_id = create_war(&mut app, DWARF, ELFIN);
        create_battle(&mut app, war_id, DWARF, ELFIN, TEST_STATE_ID, vec![attacker_id], vec![defender_id]);
        tick_day(&mut app, 1800, 1, 2);
        let a = get_division(&app, attacker_id).unwrap();
        let d = get_division(&app, defender_id).unwrap();
        (a.manpower, a.organization, d.manpower, d.organization)
    }

    assert_eq!(run("d5_run1"), run("d5_run2"));
}

/// spec §16/§21: 一国(DWARF)の補給不足Divisionが、無関係な別の同時進行戦争
/// (OCEANIC vs FERROWYN、両者とも十分な補給)の結果へ波及しない。P26-004の国家間
/// 分離・保存則も同時に維持する(各国のstockpileは自国の消費分しか動かない)。
#[test]
fn test_isolation_one_countrys_supply_shortage_does_not_leak_into_unrelated_war() {
    let dir = TempTestDir::new("isolation");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());
    set_stockpile(&mut app, DWARF, ResourceType::MilitarySupplies, 0.0);

    let dwarf_ids = division_ids_of(&app, DWARF);
    let elfin_ids = division_ids_of(&app, ELFIN);
    let oceanic_ids = division_ids_of(&app, OCEANIC);
    let ferrowyn_ids = division_ids_of(&app, FERROWYN);
    let oceanic_supplies_before = stockpile_of(&app, OCEANIC, ResourceType::MilitarySupplies);
    let ferrowyn_supplies_before = stockpile_of(&app, FERROWYN, ResourceType::MilitarySupplies);

    let war_1 = create_war(&mut app, DWARF, ELFIN);
    create_battle(&mut app, war_1, DWARF, ELFIN, TEST_STATE_ID, dwarf_ids, elfin_ids);
    let war_2 = create_war(&mut app, OCEANIC, FERROWYN);
    create_battle(&mut app, war_2, OCEANIC, FERROWYN, StateId(999_998), oceanic_ids.clone(), ferrowyn_ids.clone());

    let oceanic_before = total_manpower(&app, &oceanic_ids);
    let ferrowyn_before = total_manpower(&app, &ferrowyn_ids);

    tick_day(&mut app, 1800, 1, 2);

    // 参照(DWARF/ELFINの戦争を作らない、OCEANIC/FERROWYNの戦争だけの独立シナリオ)。
    let dir_ref = TempTestDir::new("isolation_ref");
    let mut app_ref = setup_app_in_playing(DWARF, dir_ref.save_file_config());
    let oceanic_ids_ref = division_ids_of(&app_ref, OCEANIC);
    let ferrowyn_ids_ref = division_ids_of(&app_ref, FERROWYN);
    let war_ref = create_war(&mut app_ref, OCEANIC, FERROWYN);
    create_battle(&mut app_ref, war_ref, OCEANIC, FERROWYN, StateId(999_998), oceanic_ids_ref.clone(), ferrowyn_ids_ref.clone());
    tick_day(&mut app_ref, 1800, 1, 2);

    assert_eq!(
        total_manpower(&app, &oceanic_ids),
        total_manpower(&app_ref, &oceanic_ids_ref),
        "DWARFの補給不足はOCEANIC/FERROWYN間の独立した戦争へ波及してはいけない"
    );
    assert_eq!(total_manpower(&app, &ferrowyn_ids), total_manpower(&app_ref, &ferrowyn_ids_ref));
    let _ = (oceanic_before, ferrowyn_before);

    // P26-004保存則: OCEANIC/FERROWYNの備蓄は、DWARFの状態と無関係に自国の
    // 需要・生産だけで決まる(この日は生産日ではないため、消費分だけ減るはず)。
    assert!(stockpile_of(&app, OCEANIC, ResourceType::MilitarySupplies) <= oceanic_supplies_before);
    assert!(stockpile_of(&app, FERROWYN, ResourceType::MilitarySupplies) <= ferrowyn_supplies_before);
}

// ─────────────────────────────────────────────────────────────────────────
// Oceanic実測(spec §15)
// ─────────────────────────────────────────────────────────────────────────

/// P26-004報告書§32.2が記録した「Oceanicは36か月・通常AI play中972日のうち
/// 418日不足」という既知の供給網の弱さを、本タスクで再測定し、その不足発生日に
/// おいて実際にOceanicの戦闘力(戦闘準備率)が低下していることを実証する
/// (spec §15: Oceanicだけを特別扱いする本番コードは追加しない、備蓄・生産量も
/// 一切調整しない)。
#[test]
fn test_oceanic_36_month_shortage_is_measured_and_demonstrably_weakens_combat_readiness() {
    let dir = TempTestDir::new("oceanic_36mo");
    let mut app = setup_app_in_playing(DWARF, dir.save_file_config());

    let mut min_fulfillment: f64 = 1.0;
    let mut shortage_days: u32 = 0;
    let mut first_shortage_day: Option<u32> = None;
    let mut first_shortage_snapshot: Option<(u32, f64, u64, f64)> = None; // (day, demand-proxy stockpile before, division id, fulfillment)

    let mut day_counter: u32 = 0;
    for m in 1..=36u8 {
        for day in 2..=28u8 {
            let stockpile_before = stockpile_of(&app, OCEANIC, ResourceType::MilitarySupplies);
            tick_day(&mut app, 1800, m, day);
            day_counter += 1;

            let ids = division_ids_of(&app, OCEANIC);
            for &id in &ids {
                let f = fulfillment_of(&app, id);
                if f < min_fulfillment {
                    min_fulfillment = f;
                }
                if f < 1.0 {
                    shortage_days += 1;
                    if first_shortage_day.is_none() {
                        first_shortage_day = Some(day_counter);
                        first_shortage_snapshot = Some((day_counter, stockpile_before, id.0 as u64, f));
                    }
                    break;
                }
            }
        }
        tick_month_only(&mut app, 1800, m.wrapping_add(1));
    }

    eprintln!(
        "[P26-005 Oceanic 36-month measurement] shortage_days={shortage_days} \
         first_shortage_day={first_shortage_day:?} min_fulfillment={min_fulfillment:.6}"
    );
    if let Some((day, stockpile_before, division_id, fulfillment)) = first_shortage_snapshot {
        eprintln!(
            "  first shortage: day={day} stockpile_before_that_days_consumption={stockpile_before:.6} \
             division_id={division_id} fulfillment_that_day={fulfillment:.6}"
        );
    }

    assert!(shortage_days > 0, "P26-004報告書のとおり、Oceanicは36か月中に不足するはず");
    assert!(min_fulfillment < 1.0);

    // 実際に不足したOceanicの生きているDivisionを1体取得し、その戦闘準備率
    // (manpower/equipment/補給の3要素min)を、同じDivisionが補給満タンだった場合と
    // 比較する診断用の純粋関数呼び出し(供給網自体は一切変更していない)。
    let oceanic_ids = division_ids_of(&app, OCEANIC);
    assert!(!oceanic_ids.is_empty(), "OCEANICは少なくとも1個師団を維持しているはず");
    let division = get_division(&app, oceanic_ids[0]).unwrap();
    let actual_fulfillment = fulfillment_of(&app, oceanic_ids[0]);

    let readiness_actual = strategy_game::military::combat_calc::calculate_combat_readiness_with_supply(
        division.manpower,
        division.max_manpower,
        division.equipment,
        division.max_equipment,
        actual_fulfillment,
    )
    .unwrap();
    let readiness_if_full_supply = strategy_game::military::combat_calc::calculate_combat_readiness_with_supply(
        division.manpower,
        division.max_manpower,
        division.equipment,
        division.max_equipment,
        1.0,
    )
    .unwrap();

    eprintln!(
        "  final-day readiness: actual_fulfillment={actual_fulfillment:.6} \
         combat_readiness_units(actual)={} combat_readiness_units(if_full_supply)={}",
        readiness_actual.combat_readiness_units, readiness_if_full_supply.combat_readiness_units
    );

    if actual_fulfillment < 1.0 {
        assert!(
            readiness_actual.combat_readiness_units < readiness_if_full_supply.combat_readiness_units,
            "36か月終了時点でOceanicの師団が補給不足であれば、実際の戦闘準備率は\
             満タン仮定より低いはず"
        );
    }

    // 不足発生日そのものにおける診断用の実戦闘比較: 同一manpower/equipment/base
    // strengthで、補給率だけが異なる2師団(実測値 vs 1.0)を対戦させ、補給不足側の
    // 実効戦力が劣ることを確認する(spec §15の最終要求)。
    if let Some((_, _, _, fulfillment_that_day)) = first_shortage_snapshot {
        let shortage_division = strategy_game::military::data::Division {
            id: DivisionId(9001),
            owner: OCEANIC,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: TEST_STATE_ID,
            destination: None,
            current_path: vec![],
            target_state: None,
            manpower: 10_000,
            max_manpower: 10_000,
            equipment: 100.0,
            max_equipment: 100.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 100.0,
            max_morale: 100.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Fighting,
            def_id: strategy_game::common::DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let mut full_supply_division = shortage_division.clone();
        full_supply_division.id = DivisionId(9002);
        let defender = {
            let mut d = shortage_division.clone();
            d.id = DivisionId(9003);
            d.owner = FERROWYN;
            d
        };

        let (_, _, def_loss_from_shortage_attacker, _) =
            strategy_game::military::combat_calc::resolve_combat_day_with_supply(
                &shortage_division,
                &defender,
                strategy_game::military::combat_calc::TERRAIN_BONUS_PLAIN,
                fulfillment_that_day,
                1.0,
            );
        let (_, _, def_loss_from_full_supply_attacker, _) =
            strategy_game::military::combat_calc::resolve_combat_day_with_supply(
                &full_supply_division,
                &defender,
                strategy_game::military::combat_calc::TERRAIN_BONUS_PLAIN,
                1.0,
                1.0,
            );

        eprintln!(
            "  diagnostic battle at first shortage day: def_loss(shortage_attacker)={def_loss_from_shortage_attacker} \
             def_loss(full_supply_attacker)={def_loss_from_full_supply_attacker}"
        );
        if fulfillment_that_day < 1.0 {
            assert!(
                def_loss_from_shortage_attacker < def_loss_from_full_supply_attacker,
                "不足発生日の実測補給率を持つ攻撃側は、満タン補給の攻撃側より与える\
                 被害が小さいはず(既知の供給網の弱さが実際の戦闘力低下として現れる)"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 死んだ経路の再確認(spec §5、P26-005導入後も引き続き未接続であること)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_dead_paths_remain_unconnected_after_p26_005() {
    // `military::supply::process_supply`(state.logistics_ratioベースの旧経路)・
    // `war::combat::process_combat`(P25-001監査で死んだ経路と確認済み)は、
    // どちらもP26-005で再活性化していない。
    use strategy_game::common::CountryId as Cid;
    use strategy_game::war::data::{War as W, WarRegistry as WR, WarStatus as WS};

    let mut war_reg = WR::default();
    war_reg.wars.insert(
        WarId(0),
        W {
            id: WarId(0),
            name: "dead path check".to_string(),
            attackers: [Cid(1)].into_iter().collect(),
            defenders: [Cid(2)].into_iter().collect(),
            primary_attacker: None,
            primary_defender: None,
            source_crisis_id: None,
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            end_date: None,
            duration_days: 0,
            war_score: 0.0,
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: HashSet::new(),
            status: WS::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: HashSet::new(),
        },
    );

    let mut mil = MilitaryRegistry::default();
    mil.definitions.insert(
        strategy_game::common::DivisionDefinitionId(0),
        strategy_game::military::data::DivisionDefinition {
            id: strategy_game::common::DivisionDefinitionId(0),
            name: "Test".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 10_000,
            required_equipment: 0.0,
            recruitment_days: 30,
            movement_speed: 4.0,
            attack: 10.0,
            defense: 10.0,
            breakthrough: 5.0,
            organization: 100.0,
            morale: 100.0,
            supply_usage: 1.0,
            maintenance_cost: 1.0,
        },
    );
    let a = Division {
        id: DivisionId(0),
        owner: Cid(1),
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(1),
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower: 10_000,
        max_manpower: 10_000,
        equipment: 100.0,
        max_equipment: 100.0,
        organization: 100.0,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Fighting,
        def_id: strategy_game::common::DivisionDefinitionId(0),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    };
    let mut b = a.clone();
    b.id = DivisionId(1);
    b.owner = Cid(2);
    mil.divisions.insert(a.id, a.clone());
    mil.divisions.insert(b.id, b.clone());
    // 実production経路(P26-005)なら決して読まれないはずの、著しく不正な
    // division_supply_fulfillmentを注入しても、dead pathである
    // `war::combat::process_combat`はそもそもこの補助mapを一切参照しない。
    mil.division_supply_fulfillment.insert(a.id, 0.0);

    strategy_game::war::combat::process_combat(&mut mil, &war_reg);

    // 旧来の素朴なf64減算がそのまま動く(補給率が一切考慮されない)ことを確認する
    // ことで、この経路が引き続き死んでいる(P26-005が接続した正規経路ではない)
    // ことを実証する。
    assert_ne!(
        mil.divisions[&a.id].manpower, a.manpower,
        "dead pathは補給率0.0を無視して通常どおりダメージを与えるはず(=接続されていない証拠)"
    );
}
