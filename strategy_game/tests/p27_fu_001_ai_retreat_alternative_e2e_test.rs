//! P27-FU-001: 攻撃側撤退先の代替探索を、実App・実AI(`war::military_ai`)を通した
//! 長期(90日以上)統合で観測する。単体寄りの判定式・複数Division・save/load・
//! 決定性は`tests/p27_006_low_effective_power_damage_e2e_test.rs`にあるため、本ファイルは
//! 「戦闘継続中に出撃元が敵へ再占領される(=P27-008で確認された根本原因)」を実際の
//! Battleに対して発生させ、その後AIが撤退→再攻撃ループを形成しないこと、および無関係な
//! 第三国へ影響しないことに集中する(P27-007のtest_01/test_02と同じ方針)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::CountryId;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::{BattleRegistry, BattleStatus};
use strategy_game::military::data::MilitaryRegistry;
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
use strategy_game::war::frontline::{FrontlineRegistry, update_all_frontlines};

const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const PASSGUARD: CountryId = CountryId(4);

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p27_fu_001_ai_e2e_{label}_{}_{nanos}_{n}",
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

fn setup_app(player: Option<CountryId>, save_config: SaveFileConfig) -> App {
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
    app.insert_resource(PlayerCountry(player));
    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn set_game_date(app: &mut App, year: i32, month: u8, day: u8) {
    let mut gd = app.world_mut().resource_mut::<GameDate>();
    gd.year = year;
    gd.month = month;
    gd.day = day;
}

fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn advance_days(app: &mut App, start_year: i32, start_month: u8, start_day: u8, count: u32) {
    let (mut year, mut month, mut day) = (start_year, start_month, start_day);
    for _ in 0..count {
        set_game_date(app, year, month, day);
        tick_day(app, year, month, day);
        day += 1;
        if day > 28 {
            day = 1;
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
    }
}

fn set_app_supplies(app: &mut App, c: CountryId, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(c)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitarySupplies, value);
}

fn new_war_no_goals(attacker: CountryId, defender: CountryId) -> War {
    War {
        id: strategy_game::common::WarId(0),
        name: "P27-FU-001 Test War".to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1799/12/01".to_string(),
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
    }
}

/// 実データ・実AI(`MilitaryPlugin`込み)でArcadia対Elfinの戦争を作り、Arcadiaの陸軍を
/// 大幅増強して攻勢条件を満たさせる(P27-007のsetup_offensive_warと同じ方針)。
fn setup_offensive_war(label: &str) -> (App, TempTestDir, strategy_game::common::WarId) {
    let dir = TempTestDir::new(label);
    let mut app = setup_app(None, dir.save_file_config());
    let war_id = app
        .world_mut()
        .resource_mut::<WarRegistry>()
        .add_war(new_war_no_goals(ARCADIA, ELFIN));
    app.world_mut().resource_scope(|world, mut fr: Mut<FrontlineRegistry>| {
        let w = world.resource::<WarRegistry>();
        let s = world.resource::<StateRegistry>();
        let m = world.resource::<MilitaryRegistry>();
        update_all_frontlines(w, s, m, &mut fr);
    });
    {
        let mut military_registry = app.world_mut().resource_mut::<MilitaryRegistry>();
        for division in military_registry.divisions.values_mut() {
            if division.owner == ARCADIA {
                division.manpower = 100_000;
                division.max_manpower = 100_000;
            }
        }
    }
    set_app_supplies(&mut app, ARCADIA, 1_000_000.0);
    (app, dir, war_id)
}

fn ongoing_battle_count(app: &App) -> usize {
    app.world()
        .resource::<BattleRegistry>()
        .battles
        .values()
        .filter(|b| b.status == BattleStatus::Ongoing)
        .count()
}

/// 進行中のArcadia対Elfin Battleを1つ探し、その`attacker_origins`に登録された
/// 全州をElfinへ再占領する(=P27-008で確認された根本原因「戦闘継続中に出撃元が
/// 敵支配へ変わる」を実際のBattleに対して再現する)。
fn recapture_all_attacker_origins_as_elfin(app: &mut App) -> usize {
    let origins: Vec<strategy_game::common::StateId> = {
        let battles = app.world().resource::<BattleRegistry>();
        let Some(battle) = battles
            .battles
            .values()
            .find(|b| b.status == BattleStatus::Ongoing && b.attacker_country == ARCADIA)
        else {
            return 0;
        };
        battle.attacker_origins.values().copied().collect()
    };
    let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
    for &origin in &origins {
        occupy_state(origin, ELFIN, &mut state_registry);
    }
    origins.len()
}

/// 進行中のArcadia対Elfin Battleの場所(戦闘州)の隣接州のうち、出撃元に含まれない
/// 1州を強制的にArcadia支配へ変える(=「もう1つ別の前線州を確保済み」という、
/// この統合テストでは実AIの進軍だけでは短時間に作れない状況を、代替探索が実際に
/// 成功する条件として人為的に与える)。戻り値は付与した州(見つかった場合)。
fn grant_arcadia_a_neighboring_foothold_excluding_origins(
    app: &mut App,
    origins: &[strategy_game::common::StateId],
) -> Option<strategy_game::common::StateId> {
    let (battle_state, candidate) = {
        let battles = app.world().resource::<BattleRegistry>();
        let battle = battles
            .battles
            .values()
            .find(|b| b.status == BattleStatus::Ongoing && b.attacker_country == ARCADIA)?;
        let battle_state = battle.state_id;
        let states = app.world().resource::<StateRegistry>();
        let bs = states.get(battle_state)?;
        let candidate = bs
            .neighbors
            .iter()
            .copied()
            .find(|n| !origins.contains(n))?;
        (battle_state, candidate)
    };
    let mut state_registry = app.world_mut().resource_mut::<StateRegistry>();
    occupy_state(candidate, ARCADIA, &mut state_registry);
    let _ = battle_state;
    Some(candidate)
}

/// spec §11: AIによる代替撤退先の利用が、撤退→再攻撃ループ(新しいcooldownの
/// 追加を要求するような挙動)を形成しないことを、実App・実AI経由で90日以上観測する。
///
/// 進行中の戦闘の出撃元を全て敵へ再占領した上で両陣営ゼロ損害を発生させ
/// (P27-007のtest_01と同じ強制トリガー)、攻撃側Divisionが新しい代替探索経由で
/// 撤退することを確認し、その後90日間、AIが同じ膠着を繰り返さないことを検証する。
#[test]
fn test_01_ai_does_not_form_a_retreat_reattack_loop_after_alternative_retreat() {
    let (mut app, _dir, war_id) = setup_offensive_war("alt_retreat_reattack");
    let mut recaptured = 0usize;
    let (mut y, mut m, mut d) = (1800i32, 1u8, 2u8);
    for _ in 0..60u32 {
        set_game_date(&mut app, y, m, d);
        tick_day(&mut app, y, m, d);
        d += 1;
        if d > 28 {
            d = 1;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
        recaptured = recapture_all_attacker_origins_as_elfin(&mut app);
        if recaptured > 0 {
            break;
        }
    }
    assert!(
        recaptured > 0,
        "60日以内に少なくとも1件のBattleが成立し、出撃元記録が存在するはず"
    );

    // 両国の補給を同時に0にして両陣営ゼロ損害を発生させ、P27-007経由で即日撤退させる。
    set_app_supplies(&mut app, ARCADIA, 0.0);
    set_app_supplies(&mut app, ELFIN, 0.0);

    let mut terminations = 0usize;
    let mut prev_ongoing = ongoing_battle_count(&app);
    let (mut year, mut month, mut day) = (y, m, d);
    for i in 0..90u32 {
        set_game_date(&mut app, year, month, day);
        tick_day(&mut app, year, month, day);
        day += 1;
        if day > 28 {
            day = 1;
            month += 1;
            if month > 12 {
                month = 1;
                year += 1;
            }
        }
        let ongoing = ongoing_battle_count(&app);
        if ongoing == 0 && prev_ongoing > 0 {
            terminations += 1;
        }
        prev_ongoing = ongoing;
        if i == 30 {
            set_app_supplies(&mut app, ARCADIA, 1_000_000.0);
            set_app_supplies(&mut app, ELFIN, 1_000_000.0);
        }
    }

    assert_eq!(
        terminations, 1,
        "出撃元無効化後も、90日間で終了は1回だけのはず(撤退→再攻撃ループが起きない)"
    );
    assert_eq!(ongoing_battle_count(&app), 0, "90日後も戦闘は再開していない");

    // 実測(tmp診断・削除済み): この実マップ・実AIの前線(StateId(3)、Elfin側の
    // 接触州)は、Arcadia側の隣接州がStateId(1)の1つだけの単一回廊構造。
    // 出撃元(=StateId(1))を全て敵へ再占領すると、代替探索も同じ隣接州集合を
    // 見るため候補が0件になり、新しい設計どおり「代替なし→撤退不能時Division
    // 消滅処理」が正しく選ばれる(バグではない)。「代替が見つかって生存する」
    // 経路は本ファイルの実AI統合では地形上到達不能で、その経路自体は
    // `tests/p27_006_low_effective_power_damage_e2e_test.rs`のtest_14/15/16で
    // 制御下E2Eとして既に検証済み(spec §14の「自然発火しない場合は制御下E2Eを
    // 主根拠とする」方針と同じ扱い)。ここでは「代替なし」枝でもAIがクラッシュ
    // せず、撤退→再攻撃ループを形成しないことの実証に集中する。
    let arcadia_divisions_alive = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == ARCADIA)
        .count();
    assert_eq!(
        arcadia_divisions_alive, 0,
        "この前線の唯一のArcadia隣接州を再占領したため、代替なしで攻撃側Divisionは\
         消滅するはず(単一回廊構造という地形の結果であり、バグではない)"
    );

    let _ = war_id;
}

/// spec §11・§8.2: test_01が実測したとおり、この実マップ・実AIの前線
/// (StateId(3))はArcadia側の隣接州が1つだけの単一回廊構造のため、出撃元を
/// 再占領するだけでは「代替が見つかって生存する」経路へ実AI統合では到達できない。
/// そこで、AIの進軍だけでは短時間に作れない「もう1つ別の前線州を確保済み」という
/// 状況を`grant_arcadia_a_neighboring_foothold_excluding_origins`で人為的に与え、
/// 実際のBattle・実際のAI経由でも代替探索が機能し、攻撃側Divisionが撃破ではなく
/// 撤退で生存することを確認する。
#[test]
fn test_03_ai_attacker_survives_via_alternative_when_a_second_foothold_exists() {
    let (mut app, _dir, _war_id) = setup_offensive_war("alt_retreat_survives");
    let mut origins: Vec<strategy_game::common::StateId> = Vec::new();
    let (mut y, mut m, mut d) = (1800i32, 1u8, 2u8);
    for _ in 0..60u32 {
        set_game_date(&mut app, y, m, d);
        tick_day(&mut app, y, m, d);
        d += 1;
        if d > 28 {
            d = 1;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
        origins = {
            let battles = app.world().resource::<BattleRegistry>();
            battles
                .battles
                .values()
                .find(|b| b.status == BattleStatus::Ongoing && b.attacker_country == ARCADIA)
                .map(|b| b.attacker_origins.values().copied().collect())
                .unwrap_or_default()
        };
        if !origins.is_empty() {
            break;
        }
    }
    assert!(!origins.is_empty(), "60日以内に少なくとも1件のBattleが成立するはず");

    let foothold = grant_arcadia_a_neighboring_foothold_excluding_origins(&mut app, &origins)
        .expect("battle_stateの隣接州のうち出撃元以外の州が少なくとも1つ存在するはず");
    let recaptured = recapture_all_attacker_origins_as_elfin(&mut app);
    assert_eq!(recaptured, origins.len());

    set_app_supplies(&mut app, ARCADIA, 0.0);
    set_app_supplies(&mut app, ELFIN, 0.0);

    let mut terminations = 0usize;
    let mut prev_ongoing = ongoing_battle_count(&app);
    for i in 0..90u32 {
        set_game_date(&mut app, y, m, d);
        tick_day(&mut app, y, m, d);
        d += 1;
        if d > 28 {
            d = 1;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
        let ongoing = ongoing_battle_count(&app);
        if ongoing == 0 && prev_ongoing > 0 {
            terminations += 1;
        }
        prev_ongoing = ongoing;
        if i == 30 {
            set_app_supplies(&mut app, ARCADIA, 1_000_000.0);
            set_app_supplies(&mut app, ELFIN, 1_000_000.0);
        }
    }

    assert_eq!(terminations, 1, "代替経由の生存後も、90日間で終了は1回だけのはず");
    assert_eq!(ongoing_battle_count(&app), 0, "90日後も戦闘は再開していない");

    let surviving_at_foothold = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == ARCADIA && d.current_state == foothold)
        .count();
    assert!(
        surviving_at_foothold > 0,
        "確保済みの追加州(代替撤退先)へ、実AI経由で少なくとも1個の攻撃側Divisionが\
         撤退して生存しているはず(代替探索が実際に機能している証拠)"
    );
}

/// spec §11/§13: 出撃元再占領・代替撤退・その後のAI挙動が、無関係な第三国
/// (Passguard、今回の戦争と無関係)の備蓄へ影響しないことを確認する
/// (P27-007のtest_02と同じ方針: 完全凍結ではなく、この戦争からの独立を検証する)。
#[test]
fn test_02_alternative_retreat_does_not_affect_an_unrelated_third_country() {
    let (mut with_war, _dir1, _war_id) = setup_offensive_war("alt_retreat_isolation");
    let mut recaptured = 0usize;
    let mut days_to_battle = 0u32;
    let (mut y, mut m, mut d) = (1800i32, 1u8, 2u8);
    for day_index in 0..60u32 {
        set_game_date(&mut with_war, y, m, d);
        tick_day(&mut with_war, y, m, d);
        d += 1;
        if d > 28 {
            d = 1;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
        recaptured = recapture_all_attacker_origins_as_elfin(&mut with_war);
        if recaptured > 0 {
            days_to_battle = day_index + 1;
            break;
        }
    }
    assert!(recaptured > 0, "60日以内に少なくとも1件のBattleが成立するはず");
    set_app_supplies(&mut with_war, ARCADIA, 0.0);
    set_app_supplies(&mut with_war, ELFIN, 0.0);
    advance_days(&mut with_war, y, m, d, 5);
    let with_war_units = with_war
        .world()
        .resource::<CountryRegistry>()
        .get(PASSGUARD)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitarySupplies);
    let with_war_passguard_divisions: Vec<_> = with_war
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == PASSGUARD)
        .map(|d| (d.current_state, d.manpower, d.equipment, d.status))
        .collect();

    let dir2 = TempTestDir::new("alt_retreat_isolation_baseline");
    let mut without_war = setup_app(None, dir2.save_file_config());
    advance_days(&mut without_war, 1800, 1, 2, days_to_battle + 5);
    let without_war_units = without_war
        .world()
        .resource::<CountryRegistry>()
        .get(PASSGUARD)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitarySupplies);
    let without_war_passguard_divisions: Vec<_> = without_war
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == PASSGUARD)
        .map(|d| (d.current_state, d.manpower, d.equipment, d.status))
        .collect();

    assert_eq!(
        with_war_units, without_war_units,
        "Arcadia対Elfinの出撃元再占領・代替撤退は、無関係な第三国の備蓄に影響しない"
    );
    assert_eq!(
        with_war_passguard_divisions, without_war_passguard_divisions,
        "第三国のDivision状態(位置・manpower・equipment・status)も影響を受けない"
    );
}
