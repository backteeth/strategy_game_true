//! P27-FU-001 spec §12: 攻撃側撤退先の代替探索が、国家消滅・講和処理と衝突しないことを
//! 確認する統合テスト。
//!
//! 判定式・複数Division・save/load・決定性は`tests/p27_006_low_effective_power_damage_e2e_test.rs`
//! に、AI長期観測は`tests/p27_fu_001_ai_retreat_alternative_e2e_test.rs`にあるため、本ファイルは
//! 「代替探索経由で撤退したDivisionが存在する状態」で(1)攻撃側国家が消滅する、
//! (2)防御側国家が戦闘継続中に消滅する、(3)講和が成立する、の3ケースに集中する。
//! `country::elimination`・`war::peace`のいずれも、この変更(呼び出し元が渡す撤退先が
//! 常に検証済みの実州になった)以外は一切変更していないため、既存の消滅・講和APIを
//! そのまま(重複実装せず)呼び出す。
#![allow(clippy::type_complexity)] // SystemStateのタプル型はテスト内での使い捨て構成であり、型エイリアス化の利益がない

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::DayChangedMessage;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{BattleId, CountryId, DivisionId, StateId, WarId};
use strategy_game::country::power::CountryPowerRegistry;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::diplomacy::claims::ClaimRegistry;
use strategy_game::diplomacy::crisis::CrisisRegistry;
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::economy::EconomyPlugin;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::data::{Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry};
use strategy_game::military::invasion::occupy_state;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::SaveFileConfig;
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::FrontlineRegistry;
use strategy_game::war::justification::WarJustificationRegistry;

const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p27_fu_001_elim_peace_e2e_{label}_{}_{nanos}_{n}",
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
    app.update();
    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn send_day_changed(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn get_division(app: &App, id: DivisionId) -> Option<Division> {
    app.world().resource::<MilitaryRegistry>().divisions.get(&id).cloned()
}

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
fn spawn_division(
    app: &mut App,
    owner: CountryId,
    state: StateId,
    manpower: u64,
    max_manpower: u64,
    equipment: f64,
    max_equipment: f64,
    organization: f32,
    attack: i32,
    defense: i32,
) -> DivisionId {
    let division = Division {
        id: DivisionId(0),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: state,
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower,
        max_manpower,
        equipment,
        max_equipment,
        organization,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Fighting,
        def_id: strategy_game::common::DivisionDefinitionId(0),
        attack_power: attack,
        defense_power: defense,
        combat_id: None,
    };
    app.world_mut().resource_mut::<MilitaryRegistry>().add_division(division)
}

fn create_war(app: &mut App, attacker: CountryId, defender: CountryId) -> WarId {
    let war = War {
        id: WarId(0),
        name: "P27-FU-001 Elimination/Peace Test War".to_string(),
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
        occupied_states: std::collections::HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: std::collections::HashSet::new(),
    };
    app.world_mut().resource_mut::<WarRegistry>().add_war(war)
}

#[allow(clippy::too_many_arguments)]
fn create_battle(
    app: &mut App,
    war_id: WarId,
    state_id: StateId,
    attacker_country: CountryId,
    defender_country: CountryId,
    attacker_ids: Vec<DivisionId>,
    defender_ids: Vec<DivisionId>,
    attacker_origin: StateId,
) -> BattleId {
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

fn reassign_all_states_except(app: &mut App, from: CountryId, to: CountryId, keep_state: StateId) {
    let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
    for state in state_registry.states.iter_mut() {
        if state.owner_country_id == from && state.id != keep_state {
            state.owner_country_id = to;
            state.controller_country = Some(to);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn eliminate(
    app: &mut App,
    eliminated: CountryId,
    annexed_by: CountryId,
    final_state_id: StateId,
) -> Result<
    strategy_game::country::elimination::CountryEliminationOutcome,
    strategy_game::country::elimination::CountryEliminationError,
> {
    let mut state: SystemState<(
        ResMut<StateRegistry>,
        ResMut<CountryRegistry>,
        ResMut<MilitaryRegistry>,
        ResMut<ArmyRegistry>,
        ResMut<FrontlineRegistry>,
        ResMut<WarRegistry>,
        ResMut<BattleRegistry>,
        ResMut<DiplomacyRegistry>,
        ResMut<CrisisRegistry>,
        ResMut<ClaimRegistry>,
        ResMut<WarJustificationRegistry>,
        ResMut<CountryPowerRegistry>,
    )> = SystemState::new(app.world_mut());
    let (
        mut state_registry,
        mut country_registry,
        mut military_registry,
        mut army_registry,
        mut frontline_registry,
        mut war_registry,
        mut battle_registry,
        mut diplomacy_registry,
        mut crisis_registry,
        mut claim_registry,
        mut justification_registry,
        mut power_registry,
    ) = state.get_mut(app.world_mut()).unwrap();

    let plan = strategy_game::country::elimination::preflight_country_elimination(
        final_state_id,
        eliminated,
        annexed_by,
        "1900/01/01",
        &state_registry,
        &country_registry,
    )?;

    let result = strategy_game::country::elimination::apply_country_elimination(
        plan,
        None,
        None,
        "1900/01/01",
        &mut state_registry,
        &mut country_registry,
        &mut military_registry,
        &mut army_registry,
        &mut frontline_registry,
        &mut war_registry,
        &mut battle_registry,
        &mut diplomacy_registry,
        &mut crisis_registry,
        &mut claim_registry,
        &mut justification_registry,
        &mut power_registry,
    );
    state.apply(app.world_mut());
    result
}

fn settle_peace(app: &mut App, war_id: WarId, term: strategy_game::war::peace::PeaceTerm) -> Result<(), &'static str> {
    let mut state: SystemState<(
        ResMut<StateRegistry>,
        ResMut<WarRegistry>,
        ResMut<MilitaryRegistry>,
        ResMut<BattleRegistry>,
        ResMut<DiplomacyRegistry>,
        ResMut<FrontlineRegistry>,
        ResMut<CrisisRegistry>,
        ResMut<ClaimRegistry>,
        ResMut<CountryRegistry>,
        ResMut<ArmyRegistry>,
        ResMut<WarJustificationRegistry>,
        ResMut<CountryPowerRegistry>,
    )> = SystemState::new(app.world_mut());
    let (
        mut state_registry,
        mut war_registry,
        mut military_registry,
        mut battle_registry,
        mut diplomacy_registry,
        mut frontline_registry,
        mut crisis_registry,
        mut claim_registry,
        mut country_registry,
        mut army_registry,
        mut justification_registry,
        mut power_registry,
    ) = state.get_mut(app.world_mut()).unwrap();
    let result = strategy_game::war::peace::execute_peace_settlement(
        war_id,
        term,
        "Test Peace Settlement",
        "1900/01/01",
        &mut state_registry,
        &mut war_registry,
        &mut military_registry,
        &mut battle_registry,
        &mut diplomacy_registry,
        &mut frontline_registry,
        &mut crisis_registry,
        &mut claim_registry,
        &mut country_registry,
        &mut army_registry,
        &mut justification_registry,
        &mut power_registry,
    );
    state.apply(app.world_mut());
    result
}

/// StateId(3)(Elfinの前線接触州)を戦闘州とし、出撃元StateId(1)(Arcadia既定支配)を
/// 戦闘登録直後に敵へ再占領して無効化、代わりにStateId(4)(既定はElfin支配)を
/// Arcadiaへ強制付与して唯一の代替撤退先とする(`tests/p27_006_low_effective_power_
/// damage_e2e_test.rs`のtest_14と同じ実マップ地形・同じ手法)。攻撃10/防御15の
/// 非対称statsと装備0で、攻撃側が確実に組織率0へ達して代替探索経由で撤退する
/// シナリオを実App上に組み立てる。
fn setup_battle_with_alternative_retreat(app: &mut App) -> (WarId, DivisionId, DivisionId, StateId) {
    let battle_state = StateId(3);
    let origin = StateId(1);
    let alternative = StateId(4);

    let attacker_id = spawn_division(app, ARCADIA, origin, 10_000, 10_000, 0.0, 0.0, 100.0, 10, 10);
    let defender_id = spawn_division(app, ELFIN, battle_state, 10_000, 10_000, 0.0, 0.0, 100.0, 10, 15);
    let war_id = create_war(app, ARCADIA, ELFIN);
    create_battle(
        app,
        war_id,
        battle_state,
        ARCADIA,
        ELFIN,
        vec![attacker_id],
        vec![defender_id],
        origin,
    );

    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        occupy_state(origin, ELFIN, &mut state_registry);
        occupy_state(alternative, ARCADIA, &mut state_registry);
    }

    (war_id, attacker_id, defender_id, alternative)
}

fn run_until_battle_resolves(app: &mut App, attacker_id: DivisionId) {
    for day in 1..=30u8 {
        send_day_changed(app, 1800, 1, day);
        if get_division(app, attacker_id).is_none_or(|d| d.status != DivisionStatus::Fighting) {
            break;
        }
    }
}

/// spec §12: 代替探索経由で撤退したDivisionが存在する状態で攻撃側国家が消滅しても、
/// Divisionが復活(resurrect)したり、Battle清掃と二重処理になったりしないことを確認する。
#[test]
fn test_01_attacker_elimination_after_alternative_retreat_does_not_resurrect_or_double_process() {
    let temp_dir = TempTestDir::new("attacker_elim");
    let mut app = setup_app_in_playing(ARCADIA, temp_dir.save_file_config());
    let (_war_id, attacker_id, _defender_id, alternative) = setup_battle_with_alternative_retreat(&mut app);

    run_until_battle_resolves(&mut app, attacker_id);

    let retreated = get_division(&app, attacker_id).expect("代替探索により撤退して生存しているはず");
    assert_eq!(retreated.status, DivisionStatus::Idle);
    assert_eq!(retreated.current_state, alternative, "撤退先は付与した代替州(StateId(4))のはず");
    assert!(retreated.manpower > 0 && retreated.manpower < 10_000);

    let arcadia_division_count_before = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == ARCADIA)
        .count();

    // `occupy_state`はcontroller_countryのみを変え、owner_country_idは変えない
    // (実占領と法的所有の分離は既存仕様)。`preflight_country_elimination`は
    // 「最後の陸州」の法的所有者一致を要求するため、代替撤退先(alternative)を
    // Arcadiaの正式な所有地として確定させる(過去に併合済みという設定)。
    {
        let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
        if let Some(s) = state_registry.states.iter_mut().find(|s| s.id == alternative) {
            s.owner_country_id = ARCADIA;
        }
    }
    // Arcadiaが代替州(alternative)以外の全陸州を失い、消滅条件(最後の陸州)を満たす。
    reassign_all_states_except(&mut app, ARCADIA, ELFIN, alternative);
    let outcome = eliminate(&mut app, ARCADIA, ELFIN, alternative).expect("国家消滅は成功するはず");

    assert_eq!(
        outcome.divisions_disbanded, arcadia_division_count_before,
        "解散件数は、代替探索経由で生存していたDivisionを含むArcadia所有Division総数と一致するはず"
    );
    assert!(
        !app.world().resource::<MilitaryRegistry>().divisions.contains_key(&attacker_id),
        "代替探索経由で撤退したDivisionも、他と同様に消滅時に解散されるはず(復活しない)"
    );

    // 消滅処理をもう一度実行しても、既に消滅済みのため新たな解散は発生しない
    // (二重処理防止の確認)。
    let second = eliminate(&mut app, ARCADIA, ELFIN, alternative);
    assert!(second.is_err(), "既に消滅済みの国家への再適用はエラーになるはず(二重処理防止)");
    assert!(
        !app.world().resource::<MilitaryRegistry>().divisions.contains_key(&attacker_id),
        "二重処理を試みても、解散済みDivisionが再度現れたりしない"
    );
}

/// spec §12: 攻撃側Divisionが代替探索経由で撤退可能な状態のまま、防御側国家が
/// 戦闘継続中に消滅しても、controller更新・War終結処理と衝突しないことを確認する。
#[test]
fn test_02_defender_elimination_mid_battle_does_not_conflict_with_controller_update() {
    let temp_dir = TempTestDir::new("defender_elim");
    let mut app = setup_app_in_playing(ARCADIA, temp_dir.save_file_config());
    let (war_id, attacker_id, defender_id, alternative) = setup_battle_with_alternative_retreat(&mut app);

    // まだ組織率0に達する前(戦闘継続中)にElfinを消滅させる。Elfinの最後の陸州として、
    // 今回の戦闘に無関係な実州(state_owned_by)を残す。
    let elfin_final_state = state_owned_by(&app, ELFIN);
    reassign_all_states_except(&mut app, ELFIN, ARCADIA, elfin_final_state);
    let outcome = eliminate(&mut app, ELFIN, ARCADIA, elfin_final_state).expect("防御側国家消滅は成功するはず");
    assert!(
        outcome.other_wars_ended.contains(&war_id) || outcome.wars_supporter_removed.contains(&war_id),
        "消滅国家が防御側として参加していたWarは、消滅処理の一部として終結/除去されるはず"
    );

    // 消滅処理後もクラッシュせず、攻撃側Divisionは正常な状態を保つ(既にBattleが
    // 講和により終結しているため、以後の日次処理でパニックしないことを確認する)。
    send_day_changed(&mut app, 1800, 1, 2);
    if let Some(attacker_after) = get_division(&app, attacker_id) {
        assert_ne!(attacker_after.current_state, StateId(3), "戦闘州へ取り残されたままではないはず");
    }

    let ongoing_after = app
        .world()
        .resource::<BattleRegistry>()
        .battles
        .values()
        .any(|b| b.status == BattleStatus::Ongoing && (b.attacker_country == ELFIN || b.defender_country == ELFIN));
    assert!(!ongoing_after, "消滅した防御側が関与する進行中Battleは残らないはず");

    let _ = defender_id;
    let _ = alternative;
}

/// spec §12: 代替探索経由で撤退したDivisionが存在する状態で講和(白紙講和)が成立しても、
/// 撤退時に適用済みの10%manpower損失が二重適用されたり、Divisionが敵領内へIdleのまま
/// 取り残されたり、第三国領へ移動したりしないことを確認する。
#[test]
fn test_03_peace_settlement_after_alternative_retreat_does_not_double_apply_or_misplace_division() {
    let temp_dir = TempTestDir::new("peace_after_retreat");
    let mut app = setup_app_in_playing(ARCADIA, temp_dir.save_file_config());
    let (war_id, attacker_id, _defender_id, alternative) = setup_battle_with_alternative_retreat(&mut app);

    run_until_battle_resolves(&mut app, attacker_id);
    let after_retreat = get_division(&app, attacker_id).expect("代替探索により撤退して生存しているはず");
    assert_eq!(after_retreat.current_state, alternative);
    let manpower_after_retreat = after_retreat.manpower;
    let equipment_after_retreat = after_retreat.equipment;

    settle_peace(&mut app, war_id, strategy_game::war::peace::PeaceTerm::WhitePeace)
        .expect("白紙講和は成功するはず");

    let war_after = app.world().resource::<WarRegistry>().wars.get(&war_id).cloned().unwrap();
    assert_eq!(war_after.status, WarStatus::WhitePeace, "白紙講和後はWarStatus::WhitePeaceで終結しているはず");

    let after_peace = get_division(&app, attacker_id).expect("講和処理はDivisionを消滅させないはず");
    assert_eq!(
        after_peace.manpower, manpower_after_retreat,
        "講和(白紙講和)は撤退時に適用済みのmanpower損失を再適用しないはず(二重適用なし)"
    );
    assert_eq!(
        after_peace.equipment, equipment_after_retreat,
        "講和(白紙講和)は撤退時の装備量をさらに変更しないはず"
    );

    // `alternative`(StateId(4))はArcadiaが`occupy_state`で一時的に支配していた
    // だけで法的所有者はElfinのままだったため、既存の講和処理§2「占領状態の正常化」
    // (占領解除されなかった陸上地域の支配権を法的所有国へ戻す)により、講和成立と
    // 同時に再びElfin支配へ戻る。これはP27-FU-001の変更対象ではない既存仕様であり、
    // その結果、Division は§5「敵国領残存陸軍の帰還」(`relocate_hostile_territory_
    // divisions`、既存関数・未変更)によって、Idleのまま敵領へ取り残されることなく
    // Arcadia自身の支配地へ安全に帰還する――ここで検証したいのはまさにこの「取り残され
    // ない」ことと「第三国領へは移動しない」こと。
    let alternative_controller_after_peace = app
        .world()
        .resource::<StateRegistry>()
        .get(alternative)
        .unwrap()
        .controller();
    assert_eq!(
        alternative_controller_after_peace, ELFIN,
        "占領のみだった代替撤退先は、講和成立時に法的所有国(Elfin)へ支配が戻るはず"
    );
    assert_ne!(
        after_peace.current_state, alternative,
        "支配が敵へ戻った代替撤退先に、Divisionが取り残されたままではないはず"
    );
    let final_state = app
        .world()
        .resource::<StateRegistry>()
        .get(after_peace.current_state)
        .expect("帰還先は実在する州のはず");
    assert!(
        !final_state.is_sea
            && (final_state.controller() == ARCADIA || final_state.owner_country_id == ARCADIA),
        "帰還先はArcadia自身が支配/所有する陸州のはず(第三国領へは移動しない)"
    );
    assert_eq!(after_peace.status, DivisionStatus::Idle);
}
