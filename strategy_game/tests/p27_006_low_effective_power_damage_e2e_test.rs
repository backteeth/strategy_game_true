//! P27-006: 低実効戦力の損害連続性(M: 損害分母の下限1、T: 正の入力の実効戦力下限1、
//! C: 全対象weight 0時のmanpower配分)の実App統合テスト。
//!
//! 両陣営の真の実効戦力0(readiness 0)の終了・撤退契約はP27-007で実装した
//! (`test_08`が即日終了・防御側勝利・攻撃側撤退の契約を固定する)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, StateId, WarId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::{Division, DivisionStatus, MilitaryRegistry};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::{FrontlineRegistry, update_all_frontlines};

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
        "strategy_game_p27_006_e2e_{label}_{}_{nanos}_{n}",
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

fn setup_app(save_config: SaveFileConfig) -> App {
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
    app.insert_resource(PlayerCountry(None));
    app.insert_state(GameState::Playing);
    app.update();
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
    app.update();
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(LoadRequestMessage);
    state.apply(app.world_mut());
    app.update();
    app.update();
}

fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    {
        let mut gd = app.world_mut().resource_mut::<GameDate>();
        gd.year = year;
        gd.month = month;
        gd.day = day;
    }
    let mut state: SystemState<MessageWriter<DayChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .unwrap()
        .write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn no_goal_war() -> War {
    War {
        id: WarId(0),
        name: "P27-006".to_string(),
        attackers: [ARCADIA].into_iter().collect(),
        defenders: [ELFIN].into_iter().collect(),
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

fn set_supplies(app: &mut App, c: CountryId, v: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(c)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitarySupplies, v);
}

// ─────────────────────────────────────────────────────────────────────────
// 制御シナリオ: 実データの州3(Elfin首都、Elfin支配)で、実AppのSystemが日次で
// 戦闘・補給を処理する。攻撃側はArcadia(州1=Arcadia支配・首都連結済みが隣接するため
// P27-005の前線補給ブリッジが効く)、防御側はElfin。
// ─────────────────────────────────────────────────────────────────────────

const BATTLE_STATE: StateId = StateId(3);

#[derive(Clone, Copy, Debug)]
struct Unit {
    manpower: u64,
    /// 装備充足率(max_equipment=100.0に対する比)。
    equipment_ratio: f64,
    organization: f32,
    attack: i32,
    defense: i32,
}

impl Unit {
    /// 満編制・満装備・満組織(攻撃力10・防御力15)。
    fn full() -> Self {
        Unit {
            manpower: 10_000,
            equipment_ratio: 1.0,
            organization: 50.0,
            attack: 10,
            defense: 15,
        }
    }
}

#[derive(Clone, Debug)]
struct Spec {
    attackers: Vec<Unit>,
    defenders: Vec<Unit>,
    arcadia_supplies: f64,
    elfin_supplies: f64,
    days: u32,
    save_at: Option<u32>,
    save_twice: bool,
    reverse_insert_order: bool,
    /// P27-007: `attackers`と同じ長さ・同じ順序の出撃元州上書き(`None`=既定の州1、
    /// `Some(x)`=州x、空なら全員既定)。`attacker_origins`から意図的に欠落させたい
    /// (退路なし)場合は別途`omit_origin_for`を使う。
    attacker_origin_overrides: Vec<Option<StateId>>,
    /// P27-007: このインデックス(`attackers`内の0始まり)の攻撃師団を
    /// `attacker_origins`から意図的に除外する(退路なしのDivision消滅を検証する)。
    omit_origin_for: Vec<usize>,
    /// P27-FU-001: Battle登録より前に適用する`(州, controller)`の上書き一覧。
    /// origin無効化時の代替退却先の有無を、実データのデフォルトcontrollerに
    /// 依存せず明示的に構成するために使う(例: 州1を敵支配にして退路なしを作る、
    /// 州4を自国支配にしてそこを正規の出撃元として使う)。
    pre_battle_controller_overrides: Vec<(StateId, CountryId)>,
    /// P27-FU-001: Battle登録の**後**(=`attacker_origins`記録済みの後)に適用する
    /// `(州, controller)`の上書き一覧。「戦闘開始時点では有効だった出撃元が、
    /// 戦闘継続中に敵へ再占領される」(P27-008で確認した実際の状況)を再現する。
    post_battle_controller_overrides: Vec<(StateId, CountryId)>,
}

impl Spec {
    fn new(attackers: Vec<Unit>, defenders: Vec<Unit>) -> Self {
        Spec {
            attackers,
            defenders,
            arcadia_supplies: 1_000_000.0,
            elfin_supplies: 1_000_000.0,
            days: 30,
            save_at: None,
            save_twice: false,
            reverse_insert_order: false,
            attacker_origin_overrides: Vec::new(),
            omit_origin_for: Vec::new(),
            pre_battle_controller_overrides: Vec::new(),
            post_battle_controller_overrides: Vec::new(),
        }
    }
}

/// 1師団の日末スナップショット(ID順)。
#[derive(Clone, Debug, PartialEq)]
struct DivSnap {
    id: usize,
    owner: usize,
    status: DivisionStatus,
    state: usize,
    manpower: u64,
    /// organization×10(f32の丸め差を避けるため整数化)。
    organization_x10: i64,
    equipment_x10: i64,
    fulfillment: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct DaySnap {
    controller3: usize,
    battle_ongoing: bool,
    divisions: Vec<DivSnap>,
}

fn make_division(template: &Division, u: &Unit, owner: CountryId) -> Division {
    let mut d = template.clone();
    d.owner = owner;
    d.current_state = BATTLE_STATE;
    d.status = DivisionStatus::Fighting;
    d.destination = None;
    d.current_path.clear();
    d.target_state = None;
    d.movement_progress = 0.0;
    d.max_manpower = 10_000;
    d.manpower = u.manpower;
    d.max_equipment = 100.0;
    d.equipment = 100.0 * u.equipment_ratio;
    d.max_organization = 50.0;
    d.organization = u.organization;
    d.attack_power = u.attack;
    d.defense_power = u.defense;
    d
}

struct Run {
    snaps: Vec<DaySnap>,
    /// 追加順(=ID昇順)の攻撃側・防御側の師団ID。
    attackers: Vec<usize>,
    defenders: Vec<usize>,
}

fn run_scenario(spec: &Spec) -> Run {
    let dir = TempTestDir::new("scenario");
    let mut app = setup_app(dir.save_file_config());
    let war_id = app
        .world_mut()
        .resource_mut::<WarRegistry>()
        .add_war(no_goal_war());
    app.world_mut()
        .resource_scope(|world, mut fr: Mut<FrontlineRegistry>| {
            let w = world.resource::<WarRegistry>();
            let s = world.resource::<StateRegistry>();
            let m = world.resource::<MilitaryRegistry>();
            update_all_frontlines(w, s, m, &mut fr);
        });

    // 既存のArcadia・Elfin師団を除き、制御された師団だけを州3へ置く。
    let (attacker_ids, defender_ids) = {
        let mut m = app.world_mut().resource_mut::<MilitaryRegistry>();
        let a_template = m
            .divisions
            .values()
            .find(|d| d.owner == ARCADIA)
            .unwrap()
            .clone();
        let d_template = m
            .divisions
            .values()
            .find(|d| d.owner == ELFIN)
            .unwrap()
            .clone();
        let remove: Vec<_> = m
            .divisions
            .values()
            .filter(|d| d.owner == ARCADIA || d.owner == ELFIN)
            .map(|d| d.id)
            .collect();
        for id in remove {
            m.remove_division(id);
        }
        let order = |units: &Vec<Unit>| -> Vec<Unit> {
            let mut v = units.clone();
            if spec.reverse_insert_order {
                v.reverse();
            }
            v
        };
        let atk_ids: Vec<_> = order(&spec.attackers)
            .iter()
            .map(|u| m.add_division(make_division(&a_template, u, ARCADIA)))
            .collect();
        let def_ids: Vec<_> = order(&spec.defenders)
            .iter()
            .map(|u| m.add_division(make_division(&d_template, u, ELFIN)))
            .collect();
        (atk_ids, def_ids)
    };

    // P27-FU-001: Battle登録より前に、指定された州のcontrollerを明示的に設定する。
    for &(state, controller) in &spec.pre_battle_controller_overrides {
        let mut sr = app.world_mut().resource_mut::<StateRegistry>();
        strategy_game::military::invasion::occupy_state(state, controller, &mut sr);
    }

    // 実際の戦闘登録(進行中のBattle)。両陣営の師団はFighting・州3。攻撃側の出撃元は州1。
    {
        let mut br = app
            .world_mut()
            .resource_mut::<strategy_game::military::battle::BattleRegistry>();
        let battle = strategy_game::military::battle::Battle {
            id: strategy_game::common::BattleId(0),
            war_id,
            state_id: BATTLE_STATE,
            attacker_country: ARCADIA,
            defender_country: ELFIN,
            attacker_division_ids: attacker_ids.clone(),
            defender_division_ids: defender_ids.clone(),
            // P27-007: overrides/omitはspec.attackersと同じ(=reverse_insert_order=falseの)
            // 順序を前提とする(このテストファイルの新規テストは全てreverse_insert_order=false)。
            attacker_origins: attacker_ids
                .iter()
                .enumerate()
                .filter(|(i, _)| !spec.omit_origin_for.contains(i))
                .map(|(i, id)| {
                    let origin = spec
                        .attacker_origin_overrides
                        .get(i)
                        .copied()
                        .flatten()
                        .unwrap_or(StateId(1));
                    (*id, origin)
                })
                .collect(),
            start_date: "1800/01/01".to_string(),
            elapsed_days: 0,
            status: strategy_game::military::battle::BattleStatus::Ongoing,
        };
        let battle_id = br.start_battle(battle).expect("battle registers");
        let mut m = app.world_mut().resource_mut::<MilitaryRegistry>();
        for id in attacker_ids.iter().chain(defender_ids.iter()) {
            m.divisions.get_mut(id).unwrap().combat_id = Some(battle_id);
        }
    }

    // P27-FU-001: `attacker_origins`記録済みの後で、出撃元の再占領を模擬する。
    for &(state, controller) in &spec.post_battle_controller_overrides {
        let mut sr = app.world_mut().resource_mut::<StateRegistry>();
        strategy_game::military::invasion::occupy_state(state, controller, &mut sr);
    }

    let (mut year, mut month, mut day) = (1800, 1u8, 2u8);
    let mut snaps = Vec::new();
    for n in 1..=spec.days {
        // 国家備蓄を毎日固定して、月次生産による補給の混入を防ぐ(補給0のシナリオを保つ)。
        set_supplies(&mut app, ARCADIA, spec.arcadia_supplies);
        set_supplies(&mut app, ELFIN, spec.elfin_supplies);
        if spec.save_at == Some(n) {
            send_save_request(&mut app);
            send_load_request(&mut app);
            if spec.save_twice {
                send_save_request(&mut app);
                send_load_request(&mut app);
            }
        }
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
        let mil = app.world().resource::<MilitaryRegistry>();
        let sr = app.world().resource::<StateRegistry>();
        let br = app
            .world()
            .resource::<strategy_game::military::battle::BattleRegistry>();
        let mut divisions: Vec<DivSnap> = mil
            .divisions
            .values()
            .filter(|d| d.owner == ARCADIA || d.owner == ELFIN)
            .map(|d| DivSnap {
                id: d.id.0,
                owner: d.owner.0,
                status: d.status,
                state: d.current_state.0,
                manpower: d.manpower,
                organization_x10: (d.organization * 10.0).round() as i64,
                equipment_x10: (d.equipment * 10.0).round() as i64,
                fulfillment: mil.supply_fulfillment_of(d.id),
            })
            .collect();
        divisions.sort_by_key(|d| d.id);
        snaps.push(DaySnap {
            controller3: sr.get(BATTLE_STATE).unwrap().controller().0,
            battle_ongoing: br.get_ongoing_battle_in_state(BATTLE_STATE).is_some(),
            divisions,
        });
    }
    Run {
        snaps,
        attackers: attacker_ids.iter().map(|i| i.0).collect(),
        defenders: defender_ids.iter().map(|i| i.0).collect(),
    }
}

fn find(day: &DaySnap, id: usize) -> Option<&DivSnap> {
    day.divisions.iter().find(|d| d.id == id)
}

fn snap_of(day: &DaySnap, id: usize) -> &DivSnap {
    find(day, id).unwrap_or_else(|| panic!("division {id} should exist"))
}

fn low_positive_spec() -> Spec {
    Spec::new(
        vec![Unit {
            manpower: 3000,
            equipment_ratio: 0.3,
            organization: 10.0,
            attack: 10,
            defense: 10,
        }],
        vec![Unit {
            manpower: 3000,
            equipment_ratio: 0.3,
            organization: 10.0,
            attack: 10,
            defense: 15,
        }],
    )
}

/// 攻撃側の実効戦力3(readiness 0.784 × 組織率0.4 × 攻撃力10)を、補給0の防御側へ当てる。
/// P26-008-Bの膠着状態と同一の入力(P27-006-PRE-001の再現値)。
fn stuck_state_spec() -> Spec {
    let mut s = Spec::new(
        vec![Unit {
            manpower: 7840,
            equipment_ratio: 0.784,
            organization: 20.0,
            attack: 10,
            defense: 10,
        }],
        vec![Unit::full()],
    );
    s.elfin_supplies = 0.0;
    s
}

fn one_v_two_all_zero_spec() -> Spec {
    let mut s = Spec::new(vec![Unit::full()], vec![Unit::full(), Unit::full()]);
    s.elfin_supplies = 0.0;
    s
}

fn partial_zero_spec() -> Spec {
    Spec::new(
        vec![Unit::full()],
        vec![
            Unit::full(),
            Unit {
                equipment_ratio: 0.0,
                ..Unit::full()
            },
        ],
    )
}

fn both_zero_spec() -> Spec {
    let mut s = Spec::new(vec![Unit::full()], vec![Unit::full()]);
    s.arcadia_supplies = 0.0;
    s.elfin_supplies = 0.0;
    s.days = 40;
    s
}

// ── 14.1 1対1・低正値 ───────────────────────────────────────────────────

/// 双方の入力(manpower・装備・組織率・補給)がすべて正だが、旧実装では両者の実効戦力が
/// 整数の切り捨てで0になり、損害0のまま戦闘が膠着した(P27-006-PRE-001のシミュレーションI)。
/// T(実効戦力1)とM(分母下限1)により、損害が有限かつ正になり、戦闘が有限日数で進む。
#[test]
fn test_01_low_positive_inputs_no_longer_freeze_the_battle() {
    let r = run_scenario(&low_positive_spec());
    let (a, d) = (r.attackers[0], r.defenders[0]);
    let day1 = &r.snaps[0];
    // 補給は双方とも正(補給起因ではない)。
    assert_eq!(snap_of(day1, a).fulfillment, 1.0);
    assert_eq!(snap_of(day1, d).fulfillment, 1.0);
    // 実効戦力1同士: 損害 1*100/1 = 100/日、組織率-10で双方が同日に組織率0 → 撤退(追加で-1000)。
    assert_eq!(snap_of(day1, a).manpower, 3000 - 100 - 1000);
    assert_eq!(snap_of(day1, d).manpower, 3000 - 100 - 1000);
    assert!(!day1.battle_ongoing, "戦闘は有限日数(初日)で終了する");
    assert_eq!(
        day1.controller3, ELFIN.0,
        "双方撤退なので州のcontrollerは変わらない"
    );
    assert_eq!(snap_of(day1, a).state, 1, "攻撃側は出撃元へ撤退");
    // 全期間で状態が有限に進み、NaN等がない。
    assert!(
        r.snaps
            .iter()
            .all(|s| s.divisions.iter().all(|x| x.fulfillment.is_finite()))
    );
}

// ── 14.2 P26-008-B再現(防御側補給0・攻撃側実効戦力3) ────────────────────

#[test]
fn test_02_supplied_attacker_with_low_power_now_defeats_an_unsupplied_defender() {
    let r = run_scenario(&stuck_state_spec());
    let (a, d) = (r.attackers[0], r.defenders[0]);
    let (day1, day2) = (&r.snaps[0], &r.snaps[1]);
    // 補給: 攻撃側は前線補給で正(P27-005)、防御側は0(契約は無変更)。
    assert_eq!(snap_of(day1, a).fulfillment, 1.0);
    assert_eq!(snap_of(day1, d).fulfillment, 0.0);
    // 損害: 防御側は 3*100/1 = 300 の損害(旧: 3*100/1000 = 0 で恒久膠着)。攻撃側は無傷。
    assert_eq!(snap_of(day1, d).manpower, 10_000 - 300);
    assert_eq!(snap_of(day1, d).organization_x10, 200, "組織率-30");
    assert_eq!(snap_of(day1, a).manpower, 7840);
    assert!(day1.battle_ongoing);
    // 戦闘終了・controller変更: 翌日組織率0で防御側が撤退し、州3をArcadiaが占領する。
    assert!(!day2.battle_ongoing);
    assert_eq!(day2.controller3, ARCADIA.0);
    assert_eq!(snap_of(day2, d).state, 4, "防御側は隣接する自国州へ撤退");
    // 2日目もう300の損害を受け(9400)、撤退で最大manpowerの10%(1000)を失う。
    assert_eq!(snap_of(day2, d).manpower, 10_000 - 300 - 300 - 1000);
}

// ── 14.3 複数Division・全weight 0 ───────────────────────────────────────

#[test]
fn test_03_all_zero_weight_defenders_share_the_damage_by_manpower() {
    let r = run_scenario(&one_v_two_all_zero_spec());
    let (d1, d2) = (r.defenders[0], r.defenders[1]);
    let day1 = &r.snaps[0];
    // 攻撃力10・防御側総実効戦力0: 総損失 10*100/1 = 1000 → 等manpowerで500ずつ。
    // 組織率0で撤退し追加で-1000(最大manpowerの10%)。
    assert_eq!(snap_of(day1, d1).manpower, 10_000 - 500 - 1000);
    assert_eq!(snap_of(day1, d2).manpower, 10_000 - 500 - 1000);
    // 損害保存: 防御側の戦闘損害合計 = 総損失1000。
    let combat_casualty: u64 = [d1, d2]
        .iter()
        .map(|d| 10_000 - snap_of(day1, *d).manpower - 1000)
        .sum();
    assert_eq!(combat_casualty, 1000);
    assert_eq!(
        snap_of(day1, r.attackers[0]).manpower,
        10_000,
        "攻撃側は無傷"
    );
    assert_eq!(day1.controller3, ARCADIA.0);
    assert!(!day1.battle_ongoing);
}

// ── 14.4 複数Division・一部weight 0 ─────────────────────────────────────

#[test]
fn test_04_partial_zero_weight_keeps_the_existing_allocation() {
    let r = run_scenario(&partial_zero_spec());
    let (a, da, db) = (r.attackers[0], r.defenders[0], r.defenders[1]);
    let day1 = &r.snaps[0];
    // 防御側の総実効戦力は正(A=15)。Aが総損失 10*100/15 = 66 を全て受け、B(装備0)は無傷。
    // 常時manpower比配分(不採用の候補2)なら(33, 33)になってしまう。
    assert_eq!(snap_of(day1, da).manpower, 10_000 - 66);
    assert_eq!(snap_of(day1, db).manpower, 10_000);
    assert_eq!(snap_of(day1, db).organization_x10, 500);
    // 攻撃側は 15*100/10 = 150 の損害。
    assert_eq!(snap_of(day1, a).manpower, 10_000 - 150);
    assert!(day1.battle_ongoing);
}

// ── 14.5 Equipment 0 ────────────────────────────────────────────────────

#[test]
fn test_05_equipment_zero_is_true_zero_readiness_and_no_invulnerability() {
    let r = run_scenario(&Spec::new(
        vec![Unit::full()],
        vec![Unit {
            equipment_ratio: 0.0,
            ..Unit::full()
        }],
    ));
    let (a, d) = (r.attackers[0], r.defenders[0]);
    let day1 = &r.snaps[0];
    assert_eq!(
        snap_of(day1, d).fulfillment,
        1.0,
        "補給は正(Equipment 0が原因)"
    );
    // 防御側のoutgoingは0(Tでも1にしない): 攻撃側は無傷。
    assert_eq!(snap_of(day1, a).manpower, 10_000);
    assert_eq!(snap_of(day1, a).organization_x10, 500);
    // 防御側は 10*100/1 = 1000 の損害を受け(無敵にならない)、組織率0で撤退(-1000)。
    assert_eq!(snap_of(day1, d).manpower, 10_000 - 1000 - 1000);
    assert!(!day1.battle_ongoing);
    assert_eq!(day1.controller3, ARCADIA.0);
}

// ── 14.6 supply 0 ───────────────────────────────────────────────────────

#[test]
fn test_06_defender_supply_zero_takes_damage_and_attacker_supply_zero_is_not_invulnerable() {
    // 防御側の補給0: 防御側だけが損害を受ける。
    let mut s = Spec::new(vec![Unit::full()], vec![Unit::full()]);
    s.elfin_supplies = 0.0;
    let r = run_scenario(&s);
    let day1 = &r.snaps[0];
    assert_eq!(snap_of(day1, r.defenders[0]).fulfillment, 0.0);
    assert_eq!(
        snap_of(day1, r.attackers[0]).manpower,
        10_000,
        "outgoing 0の防御側は攻撃側を害さない"
    );
    assert_eq!(snap_of(day1, r.defenders[0]).manpower, 10_000 - 1000 - 1000);

    // 攻撃側の補給0(国家備蓄0=前線補給も届かない): 攻撃側は無敵ではなく、防御側の損害はない。
    let mut s = Spec::new(vec![Unit::full()], vec![Unit::full()]);
    s.arcadia_supplies = 0.0;
    let r = run_scenario(&s);
    let day1 = &r.snaps[0];
    assert_eq!(snap_of(day1, r.attackers[0]).fulfillment, 0.0);
    assert_eq!(
        snap_of(day1, r.defenders[0]).manpower,
        10_000,
        "outgoing 0の攻撃側は防御側を害さない"
    );
    // 防御力15の防御側から 15*100/1 = 1500 を受け、組織率0で出撃元へ撤退(-1000)。
    assert_eq!(snap_of(day1, r.attackers[0]).manpower, 10_000 - 1500 - 1000);
    assert_eq!(snap_of(day1, r.attackers[0]).state, 1);
    assert!(!day1.battle_ongoing);
    assert_eq!(day1.controller3, ELFIN.0);
}

// ── 14.7 manpower 0 ─────────────────────────────────────────────────────

#[test]
fn test_07_manpower_zero_division_gets_no_invented_weight_and_is_removed() {
    let r = run_scenario(&Spec::new(
        vec![Unit::full()],
        vec![
            Unit::full(),
            Unit {
                manpower: 0,
                ..Unit::full()
            },
        ],
    ));
    let (da, db) = (r.defenders[0], r.defenders[1]);
    let day1 = &r.snaps[0];
    assert!(
        find(day1, db).is_none(),
        "manpower 0の師団は既存の消滅処理で除去される"
    );
    // 生存する防御Divisionが通常の損害(10*100/15 = 66)を全て受ける。
    assert_eq!(snap_of(day1, da).manpower, 10_000 - 66);
    assert!(day1.battle_ongoing);
}

// ── 14.8 両陣営の真の0(P27-007: 即日終了・攻撃側撤退) ─────────────────────

/// P27-007により、双方のreadinessが真に0(=その日どちらの陣営も損害を与えられない)の
/// 戦闘は、初日のうちに防御側勝利として終了し、攻撃側は出撃元(`attacker_origins`)へ
/// 撤退する。架空のdamageは生成せず、controllerも変えない(防御側勝利と同じ契約)。
#[test]
fn test_08_both_sides_truly_zero_ends_same_day_with_defender_victory_and_attacker_retreat() {
    let r = run_scenario(&both_zero_spec());
    assert_eq!(r.snaps.len(), 40);
    let day1 = &r.snaps[0];
    assert!(!day1.battle_ongoing, "day1: 両陣営ゼロ損害のため即日終了する");
    assert_eq!(day1.controller3, ELFIN.0, "防御側勝利と同じ契約でcontrollerは変わらない");
    let attacker = snap_of(day1, r.attackers[0]);
    let defender = snap_of(day1, r.defenders[0]);
    assert_eq!(attacker.status, DivisionStatus::Idle, "撤退後はIdle");
    assert_eq!(attacker.state, 1, "既存のattacker_origins(state1)へ撤退する");
    assert_eq!(
        attacker.manpower,
        10_000 - 1_000,
        "撤退ペナルティ(最大manpowerの10%)以外の架空damageは生成しない"
    );
    assert_eq!(defender.status, DivisionStatus::Idle, "防御側勝利後の既存状態");
    assert_eq!(defender.state, 3, "防御側は自陣に留まる");
    assert_eq!(defender.manpower, 10_000, "防御側は無傷(架空のdamageを生成しない)");
    // 以後の日は既に戦闘が存在しないため、双方とも状態は変化しない。
    for (i, day) in r.snaps.iter().enumerate().skip(1) {
        assert!(!day.battle_ongoing, "day {}: 戦闘は既に終了している", i + 1);
        assert_eq!(day.controller3, ELFIN.0);
    }
}

// ── 14.9 決定性 ─────────────────────────────────────────────────────────

#[test]
fn test_09_determinism_independent_runs_and_insertion_order() {
    let a = run_scenario(&one_v_two_all_zero_spec());
    let b = run_scenario(&one_v_two_all_zero_spec());
    assert_eq!(a.snaps, b.snaps, "独立2回は完全一致");

    // manpowerの異なる防御側2個(比 6:4)を逆順で追加しても、陣営ごとの多重集合は同一。
    let mut spec = Spec::new(
        vec![Unit::full()],
        vec![
            Unit {
                manpower: 6000,
                ..Unit::full()
            },
            Unit {
                manpower: 4000,
                ..Unit::full()
            },
        ],
    );
    spec.elfin_supplies = 0.0;
    let forward = run_scenario(&spec);
    spec.reverse_insert_order = true;
    let reversed = run_scenario(&spec);
    type Row = (usize, DivisionStatus, usize, u64, i64);
    let normalize = |run: &Run| -> Vec<Vec<Row>> {
        run.snaps
            .iter()
            .map(|d| {
                let mut v: Vec<_> = d
                    .divisions
                    .iter()
                    .map(|x| (x.owner, x.status, x.state, x.manpower, x.organization_x10))
                    .collect();
                v.sort_by_key(|t| (t.0, t.3, t.4));
                v
            })
            .collect()
    };
    assert_eq!(
        normalize(&forward),
        normalize(&reversed),
        "登録順反転で結果が同一"
    );
    // 6:4 で総損失1000を配分: 600 / 400。組織率-60の側だけが組織率0で撤退(-1000)し4400、
    // 組織率-40の側は戦闘継続で3600。
    let day1 = &forward.snaps[0];
    let mut def_manpower: Vec<u64> = forward
        .defenders
        .iter()
        .map(|d| snap_of(day1, *d).manpower)
        .collect();
    def_manpower.sort();
    assert_eq!(def_manpower, vec![3600, 4400]);
}

// ── 14.10 save→load ─────────────────────────────────────────────────────

fn assert_save_load_matches(mut spec: Spec, save_days: &[u32]) {
    let continuous = run_scenario(&spec);
    for &n in save_days {
        spec.save_at = Some(n);
        spec.save_twice = false;
        let once = run_scenario(&spec);
        assert_eq!(
            once.snaps, continuous.snaps,
            "day {n}のsave→load後も連続実行と一致"
        );
        spec.save_twice = true;
        let twice = run_scenario(&spec);
        assert_eq!(
            twice.snaps, continuous.snaps,
            "day {n}の2回連続save→load後も一致"
        );
    }
}

#[test]
fn test_10_save_load_matches_the_continuous_run_at_every_critical_point() {
    // 低実効戦力(実効戦力1へ切り上げられる日)の直前・当日。
    assert_save_load_matches(low_positive_spec(), &[1, 2]);
    // 実効戦力3 vs 補給0: 損害適用前(1)・適用後(2)・controller変更後(3)。
    assert_save_load_matches(stuck_state_spec(), &[1, 2, 3]);
    // 全weight 0のフォールバック(manpower配分)中・撤退直前後。
    assert_save_load_matches(one_v_two_all_zero_spec(), &[1, 2]);
    // 一部weight 0の継続戦闘(Division消滅・撤退の直前を含む)。
    assert_save_load_matches(partial_zero_spec(), &[1, 2, 3]);
    // 両陣営の真の0(P27-007により即日終了・攻撃側撤退): 終了当日(1)・終了後(20)。
    assert_save_load_matches(both_zero_spec(), &[1, 20]);
}

// ─────────────────────────────────────────────────────────────────────────
// P27-007: 両陣営ゼロ損害終了の退却先・退路なし・決定性
// ─────────────────────────────────────────────────────────────────────────

/// 【P27-FU-001により修正(分類D: fixtureの過去の誤り)】実データ上、州3の隣接州で
/// デフォルトから自国(Arcadia)支配なのは州1だけで、州4はElfin(敵)の領土である
/// (P27-FU-001-PRE-001 §10で判明)。P27-007時点の実装(originを無条件に信頼する)では
/// この誤りが表面化しなかったが、P27-FU-001(originのcontrollerを検証する)では
/// 州4を出撃元に指定しても無効と判定され、代替探索(州3の隣接州のうち他に有効な
/// 自国州がない)の結果、2機目の攻撃師団は退路なしとして消滅してしまう。
/// 本来の検証目的(複数の攻撃師団がそれぞれ個別の出撃元へ撤退する)を保つため、
/// `pre_battle_controller_overrides`で州4を戦闘開始前に明示的にArcadia支配へ
/// 変更し、州4が「正規の出撃元」として成立する状態を作る。
#[test]
fn test_11_multiple_attackers_with_distinct_origins_retreat_to_their_own_origin() {
    let mut spec = Spec::new(
        vec![Unit::full(), Unit::full()],
        vec![Unit::full()],
    );
    spec.arcadia_supplies = 0.0;
    spec.elfin_supplies = 0.0;
    spec.pre_battle_controller_overrides = vec![(StateId(4), ARCADIA)];
    spec.attacker_origin_overrides = vec![Some(StateId(1)), Some(StateId(4))];
    let r = run_scenario(&spec);
    let day1 = &r.snaps[0];
    assert!(!day1.battle_ongoing, "両陣営ゼロ損害のため即日終了する");
    let a0 = snap_of(day1, r.attackers[0]);
    let a1 = snap_of(day1, r.attackers[1]);
    assert_eq!(a0.state, 1, "師団ごとの出撃元(州1)へ撤退する");
    assert_eq!(a1.state, 4, "師団ごとの出撃元(州4、事前にArcadia支配へ変更済み)へ撤退する");
    assert_eq!(a0.status, DivisionStatus::Idle);
    assert_eq!(a1.status, DivisionStatus::Idle);
    assert_eq!(a0.manpower, 10_000 - 1_000);
    assert_eq!(a1.manpower, 10_000 - 1_000);
}

/// 【P27-FU-001により再設計(分類B: 退路なし・消滅を検証するテスト)】P27-FU-001以降、
/// originが無効・欠落でも、戦闘州(州3)に隣接する他の自国州(既定では州1)への
/// 代替撤退が試みられる。そのため単に出撃元を欠落させるだけでは、州1という
/// 有効な代替先が既に存在し、退路なしにならない(実測で師団が州1へ生存撤退する)。
/// 真に退路なしを検証するには、州1も無効化(敵支配へ変更)し、州3の他の隣接州も
/// 元々自国支配ではない(実データのデフォルト)ことを利用して、代替先も存在しない
/// 状態を明示的に作る。
#[test]
fn test_12_missing_origin_and_no_alternative_destroys_the_attacker_without_panicking() {
    let mut spec = Spec::new(vec![Unit::full()], vec![Unit::full()]);
    spec.arcadia_supplies = 0.0;
    spec.elfin_supplies = 0.0;
    spec.omit_origin_for = vec![0];
    // 州3の隣接州のうちデフォルトで自国(Arcadia)支配なのは州1だけ(§実データ確認済み)。
    // それも敵支配へ変更し、代替撤退先が本当に存在しない状態を作る。
    spec.pre_battle_controller_overrides = vec![(StateId(1), ELFIN)];
    let r = run_scenario(&spec);
    let day1 = &r.snaps[0];
    assert!(!day1.battle_ongoing);
    assert!(
        find(day1, r.attackers[0]).is_none(),
        "originが無効・欠落かつ代替先も存在しない攻撃師団は既存の消滅処理で除去される"
    );
    assert_eq!(day1.controller3, ELFIN.0, "controllerは変わらない");
}

#[test]
fn test_13_zero_damage_termination_is_independent_of_registration_order() {
    let mut spec = Spec::new(
        vec![Unit::full(), Unit::full()],
        vec![Unit::full(), Unit::full()],
    );
    spec.arcadia_supplies = 0.0;
    spec.elfin_supplies = 0.0;
    let forward = run_scenario(&spec);
    spec.reverse_insert_order = true;
    let reversed = run_scenario(&spec);
    let normalize = |run: &Run| -> Vec<(usize, String, u64, i64)> {
        let day1 = &run.snaps[0];
        let mut v: Vec<_> = day1
            .divisions
            .iter()
            .map(|d| (d.owner, format!("{:?}", d.status), d.manpower, d.organization_x10))
            .collect();
        v.sort();
        v
    };
    assert_eq!(normalize(&forward), normalize(&reversed), "登録順反転で結果が同一");
    assert!(!forward.snaps[0].battle_ongoing);
    assert!(!reversed.snaps[0].battle_ongoing);
}

// ─────────────────────────────────────────────────────────────────────────
// P27-FU-001: 撤退先の検証・代替探索(実App・実System経由)
// ─────────────────────────────────────────────────────────────────────────

/// P27-008で確認した実際の状況(戦闘開始時点では有効だった出撃元が、戦闘継続中に
/// 敵へ再占領される)を再現する。攻撃側の補給0により通常の組織率撤退経路
/// (P27-007のゼロ損害経路ではない)で撤退が発生し、出撃元(州1)が既に敵支配へ
/// 変わっているため、州4(事前にArcadia支配へ変更した代替候補)へ撤退する。
/// P27-008で確認された「敵支配州で`Idle`になる」不整合が解消されることを確認する。
#[test]
fn test_14_attacker_retreats_to_an_alternative_when_the_origin_is_recaptured_mid_battle() {
    let mut spec = Spec::new(vec![Unit::full()], vec![Unit::full()]);
    spec.arcadia_supplies = 0.0; // 攻撃側(Arcadia)の国家備蓄0 → 攻撃側の実効戦力0。
    spec.pre_battle_controller_overrides = vec![(StateId(4), ARCADIA)];
    spec.attacker_origin_overrides = vec![Some(StateId(1))];
    // 戦闘開始(=attacker_origins記録)の後に、出撃元(州1)を敵が再占領する。
    spec.post_battle_controller_overrides = vec![(StateId(1), ELFIN)];
    let r = run_scenario(&spec);
    let day1 = &r.snaps[0];
    assert!(!day1.battle_ongoing, "攻撃側の組織率0により通常経路で戦闘が終わる");
    let attacker = snap_of(day1, r.attackers[0]);
    assert_eq!(attacker.status, DivisionStatus::Idle);
    assert_eq!(attacker.state, 4, "出撃元(州1)は無効化されたため代替先(州4)へ撤退する");
    assert_ne!(attacker.state, 1, "敵支配化した出撃元へは戻らない");
    assert_eq!(day1.controller3, ELFIN.0, "controllerは変わらない");
}

/// 同一Battle内で、正常originの攻撃師団と、origin欠落だが代替先(州4)がある攻撃師団が
/// それぞれ独立して正しい結果になることを確認する。
///
/// 退路なしの師団を同じBattle内へさらに混在させることはできない: 代替探索は
/// `owner`と`battle_state`だけで決まり、同じBattleの攻撃側は全員同じownerを
/// 共有するため、「あるorigin無効師団は代替先へ撤退できるが、別のorigin無効師団は
/// 退路なしになる」状況は構造的に作れない(退路なし単体の検証はtest_12で行う)。
#[test]
fn test_15_mixed_attackers_each_resolve_independently_in_the_same_battle() {
    let mut spec = Spec::new(vec![Unit::full(), Unit::full()], vec![Unit::full()]);
    spec.arcadia_supplies = 0.0;
    spec.elfin_supplies = 0.0;
    spec.pre_battle_controller_overrides = vec![(StateId(4), ARCADIA)];
    // 0番目: 正常originを州4に設定(origin優先はStateId順によらずそのまま使う)。
    // 1番目: origin欠落 → 代替探索は州1・州4のうちStateId昇順で州1を選ぶ。
    // 両者が異なる州になることで、origin優先と代替探索が別々の経路であることを示す。
    spec.attacker_origin_overrides = vec![Some(StateId(4))];
    spec.omit_origin_for = vec![1];
    let r = run_scenario(&spec);
    let day1 = &r.snaps[0];
    assert!(!day1.battle_ongoing);
    let a0 = snap_of(day1, r.attackers[0]);
    assert_eq!(a0.state, 4, "0番目は正常origin(州4)へ、StateId順に関わらずそのまま撤退する");
    let a1 = snap_of(day1, r.attackers[1]);
    assert_eq!(a1.state, 1, "1番目はorigin欠落のため代替探索(StateId昇順で州1)を使う");
}

/// 複数の攻撃師団が同じ代替先へ撤退しても、欠落・重複が生じないことを確認する。
#[test]
fn test_16_multiple_divisions_retreating_to_the_same_alternative_are_not_lost_or_duplicated() {
    let mut spec = Spec::new(vec![Unit::full(), Unit::full()], vec![Unit::full()]);
    spec.arcadia_supplies = 0.0;
    spec.elfin_supplies = 0.0;
    // 両方ともorigin欠落 → 両方とも同じ代替先(州1)へ撤退する。
    spec.omit_origin_for = vec![0, 1];
    let r = run_scenario(&spec);
    let day1 = &r.snaps[0];
    assert!(!day1.battle_ongoing);
    let a0 = snap_of(day1, r.attackers[0]);
    let a1 = snap_of(day1, r.attackers[1]);
    assert_eq!(a0.state, 1);
    assert_eq!(a1.state, 1);
    assert_ne!(a0.id, a1.id, "2個の異なるDivisionが両方とも生存している(重複・欠落なし)");
    assert_eq!(a0.manpower, 10_000 - 1_000);
    assert_eq!(a1.manpower, 10_000 - 1_000);
}

/// origin無効化(代替撤退)・退路なし消滅の各分岐点で、save→loadが連続実行と一致する
/// ことを確認する(§8.7)。
#[test]
fn test_17_save_load_matches_across_the_retreat_destination_branches() {
    // origin有効な戦闘中(day1にはまだ両陣営正の実効戦力があり戦闘継続、後段のtest_10と
    // 同じ`stuck_state_spec`的な非ゼロ損害シナリオを使う代わりに、ここでは撤退先
    // 分岐そのものを対象にするため、両陣営ゼロ損害の分岐点(day1=即日終了)を使う)。
    let mut recapture_spec = Spec::new(vec![Unit::full()], vec![Unit::full()]);
    recapture_spec.arcadia_supplies = 0.0;
    recapture_spec.pre_battle_controller_overrides = vec![(StateId(4), ARCADIA)];
    recapture_spec.attacker_origin_overrides = vec![Some(StateId(1))];
    recapture_spec.post_battle_controller_overrides = vec![(StateId(1), ELFIN)];
    assert_save_load_matches(recapture_spec, &[1]);

    // 退路なし消滅後。
    let mut no_retreat_spec = Spec::new(vec![Unit::full()], vec![Unit::full()]);
    no_retreat_spec.arcadia_supplies = 0.0;
    no_retreat_spec.elfin_supplies = 0.0;
    no_retreat_spec.omit_origin_for = vec![0];
    no_retreat_spec.pre_battle_controller_overrides = vec![(StateId(1), ELFIN)];
    no_retreat_spec.days = 5;
    assert_save_load_matches(no_retreat_spec, &[1, 3]);
}
