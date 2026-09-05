//! P26-004-PRE-002: 鉱床を考慮したMine建設AI(`country::country_ai::process_construction_ai`)
//! の実App・実データE2Eテスト。
//!
//! `verification_logs/phase-26/p26-004-pre-001/`が実測した36か月シミュレーションの
//! 結果(Arcadiaが鉱床を持ちながら財政破綻前にMineを建てられない、Elfin/Oceanicが
//! 鉱床皆無なのにMineへ国庫を浪費し続ける)を踏まえ、本タスクのAI修正が実際に
//! この2つの誤判断を是正することを、実データ・実App経由(6か国同時、36か月)で
//! 確認する(spec §7 Test J・K・L・M相当)。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::BuildingType;
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::country_ai::{CountryAiDecisionReason, CountryAiRegistry};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
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

// 実データ上のCountryId(`assets/data/countries.ron`、`verification_logs/phase-26/
// p26-004-pre-001/`§5の表と一致)。
const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const DWARF: CountryId = CountryId(2);
const OCEANIC: CountryId = CountryId(3);
const PASSGUARD: CountryId = CountryId(4);
const FERROWYN: CountryId = CountryId(5);

// Arcadiaの鉱床(Iron50/Coal40)が実在する州(`assets/data/resources.ron`)。
const ARCADIA_DEPOSIT_STATE: StateId = StateId(0);

// ─────────────────────────────────────────────────────────────────────────
// App-level fixtures (P26-002/P26-003と同じ構成方針)
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
        "strategy_game_p26_004_pre_002_e2e_{label}_{}_{nanos}_{n}",
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
    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing)

    app
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
    state.apply(app.world_mut());
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
    state.apply(app.world_mut());
}

fn tick_day(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn tick_month(app: &mut App, year: i32, month: u8) {
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

/// P26-003と同じ簡略暦(1か月=27日[day2〜28]+月境界tick)で、`month_index`か月分
/// (1始まり)を進める。呼び出し側が月ごとにチェックポイントを取れるよう、
/// 実App全体を進めるのではなく1か月分だけを進める関数として分離する
/// (`verification_logs/phase-26/p26-004-pre-001/`§12と同じ簡略暦を踏襲)。
fn advance_one_month(app: &mut App, current_month_index: u8) {
    for day in 2..=28u8 {
        tick_day(app, 1800, current_month_index, day);
    }
    tick_month(app, 1800, current_month_index.wrapping_add(1));
}

fn advance_months(app: &mut App, months: u8) {
    for m in 1..=months {
        advance_one_month(app, m);
    }
}

fn stockpile_of(app: &App, country_id: CountryId, resource: ResourceType) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(resource)
}

fn treasury_of(app: &App, country_id: CountryId) -> f64 {
    app.world().resource::<CountryRegistry>().get(country_id).unwrap().treasury
}

fn construction_queue_len_of(app: &App, country_id: CountryId) -> usize {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .construction_queue
        .len()
}

fn queue_contains_mine_at(app: &App, country_id: CountryId, state_id: StateId) -> bool {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .construction_queue
        .iter()
        .any(|item| item.building_type == BuildingType::Mine && item.state_id == state_id)
}

fn mine_level_at(app: &App, state_id: StateId) -> u32 {
    app.world()
        .resource::<StateRegistry>()
        .get(state_id)
        .unwrap()
        .building_level(BuildingType::Mine)
}

fn total_mine_level_owned_by(app: &App, country_id: CountryId) -> u32 {
    app.world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .filter(|s| s.owner_country_id == country_id)
        .map(|s| s.building_level(BuildingType::Mine))
        .sum()
}

fn decision_reason_of(app: &App, country_id: CountryId) -> Option<CountryAiDecisionReason> {
    app.world()
        .resource::<CountryAiRegistry>()
        .ai_states
        .get(&country_id)
        .map(|s| s.decision_reason)
}

// ─────────────────────────────────────────────────────────────────────────
// Category J: Arcadia実データ — 事前開発が財政破綻前にMineを建てる
// ─────────────────────────────────────────────────────────────────────────

/// Test J-1: New Game開始直後(初回の日次AI評価、`ai_state.dirty=true`により月1・
/// 日2で即時発火)、Arcadia(通常の財政条件、`countries.ron`のtreasury=5000.0を
/// 一切変更しない)が、実際に鉱床の存在するStateId(0)へMine建設を着手する。
/// `RawMaterialMinePriority`の理由づけで、既存のIron<30トリガーより先に発火する
/// ことを、月1のうちに(=財政破綻するとされていた月6よりずっと前に)確認する。
#[test]
fn test_j1_arcadia_queues_mine_on_state0_within_month_1_under_normal_treasury() {
    let player = DWARF; // Arcadia自身はAI操作(非プレイヤー)のまま検証する
    let temp_dir = TempTestDir::new("j1");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let initial_treasury = treasury_of(&app, ARCADIA);
    assert_eq!(
        initial_treasury, 5000.0,
        "precondition: real countries.ron Arcadia treasury must be unmodified"
    );
    assert_eq!(
        mine_level_at(&app, ARCADIA_DEPOSIT_STATE),
        0,
        "precondition: State 0's Mine level starts at 0 (real states.ron)"
    );

    // 月1、日2〜28まで日次tickを進める(月次tickの前に建設着手が起きるはず)。
    for day in 2..=28u8 {
        tick_day(&mut app, 1800, 1, day);
        if queue_contains_mine_at(&app, ARCADIA, ARCADIA_DEPOSIT_STATE) {
            break;
        }
    }

    assert!(
        queue_contains_mine_at(&app, ARCADIA, ARCADIA_DEPOSIT_STATE),
        "Arcadia must queue a Mine at State 0 within month 1 under real, unmodified finances"
    );
    assert_eq!(
        decision_reason_of(&app, ARCADIA),
        Some(CountryAiDecisionReason::RawMaterialMinePriority)
    );
    assert_eq!(
        treasury_of(&app, ARCADIA),
        initial_treasury - 500.0,
        "treasury must decrease by exactly the Mine's construction cost (500.0), atomically"
    );
}

/// Test J-2: 36か月経過後、Arcadiaの州0Mineが実際に完成(レベル>=1)し、Iron備蓄が
/// P26-004-PRE-001実測の恒久停止値(月18以降373.96で固定)から改善しているか、
/// 少なくとも財政破綻前に建設着手されたことの帰結を実測する。数値は実App経由の
/// 測定値であり、`verification_logs/phase-26/p26-004-pre-001/`の記録済み実測値
/// (改修前トレース)とだけ比較する(旧AIを再実行して比較したものではない)。
#[test]
fn test_j2_arcadia_mine_completes_and_iron_production_improves_over_36_months() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("j2");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let mut monthly_supplies = Vec::new();
    let mut monthly_iron = Vec::new();
    for m in 1..=36u8 {
        advance_one_month(&mut app, m);
        monthly_supplies.push(stockpile_of(&app, ARCADIA, ResourceType::MilitarySupplies));
        monthly_iron.push(stockpile_of(&app, ARCADIA, ResourceType::Iron));
    }

    assert!(
        mine_level_at(&app, ARCADIA_DEPOSIT_STATE) >= 1,
        "State 0's Mine must have completed construction (required_progress=60.0, i.e. ~2 \
         simplified months) well within 36 months"
    );

    let supplies_month_18 = monthly_supplies[17];
    let supplies_month_36 = monthly_supplies[35];
    assert!(
        supplies_month_36 > supplies_month_18,
        "P26-004-PRE-001 recorded a permanent freeze at 373.96 from month 18 onward under the \
         old AI; with the deposit-aware fix, month 36 ({supplies_month_36}) must exceed month 18 \
         ({supplies_month_18}), proving production did not permanently stall"
    );
    assert!(
        monthly_iron[35] > 0.0 || monthly_supplies[35] > 373.96,
        "Iron supply must not have permanently bottomed out at the old broken plateau"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Category K: Elfin/Oceanic実データ — 鉱床皆無国家がMineへ浪費しない
// ─────────────────────────────────────────────────────────────────────────

/// Test K-1: Elfin Republic(領内に登録鉱床が一切ない、`assets/data/resources.ron`
/// にはStateId 0,2,5,7しか鉱床が登録されておらず、いずれもElfinの所有州ではない)は、
/// 36か月間AI操作させてもMine総レベルが一切増加しない(P26-004-PRE-001実測の
/// 「月30時点でMine総lv10=上限」という浪費からの回帰)。
///
/// 注: Elfinは実データ上、既にStateId(16)へ`Mine: 1`が初期配置されている
/// (`assets/data/states.ron`、鉱床の有無とは無関係な既存マップ上の初期建物であり、
/// P26-004-PRE-002が作った状態でも、本タスクが除去すべき対象でもない)。したがって
/// 「一切浪費しない」の検証は「初期値(1)からの増加が一切ない」という形で行う。
#[test]
fn test_k1_elfin_never_wastes_treasury_on_a_mine_over_36_months() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("k1");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let initial_mine_level = total_mine_level_owned_by(&app, ELFIN);
    assert_eq!(
        initial_mine_level, 1,
        "precondition: Elfin's real initial data has exactly one pre-placed Mine level \
         (StateId 16) unrelated to any deposit"
    );

    advance_months(&mut app, 36);

    assert_eq!(
        total_mine_level_owned_by(&app, ELFIN),
        initial_mine_level,
        "Elfin has no usable deposit anywhere in its territory; its total Mine level must never \
         grow beyond the pre-existing initial value across 36 months (P26-004-PRE-001 measured \
         it wastefully reaching total level 10 under the old AI)"
    );
    assert_eq!(
        stockpile_of(&app, ELFIN, ResourceType::MilitarySupplies),
        0.0,
        "with no deposit, no active mining route, and no MilitaryFactory input chain, \
         MilitarySupplies must remain 0 (this fix does not and cannot resolve Elfin's \
         underlying supply problem)"
    );
}

/// Test K-2: Oceanic Magic Empire(同じく領内に登録鉱床が一切ない)も、36か月間で
/// Mine総レベルが一切増加しない(P26-004-PRE-001実測の「月36時点でMine総lv32」から
/// の回帰)。
#[test]
fn test_k2_oceanic_never_wastes_treasury_on_a_mine_over_36_months() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("k2");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    advance_months(&mut app, 36);

    assert_eq!(
        total_mine_level_owned_by(&app, OCEANIC),
        0,
        "Oceanic has no usable deposit anywhere in its territory; its total Mine level must \
         remain exactly 0 across 36 months (P26-004-PRE-001 measured it wastefully reaching \
         total level 32 under the old AI)"
    );
    assert_eq!(stockpile_of(&app, OCEANIC, ResourceType::MilitarySupplies), 0.0);
}

// ─────────────────────────────────────────────────────────────────────────
// Category L: Passguard/Ferrowyn実データ — 未解決のまま残ることを確認(対象外)
// ─────────────────────────────────────────────────────────────────────────

/// Test L-1: Passguard Marchは領内に鉱床がなく、かつIndustrialGoods生産経路が
/// 皆無であるため、本タスクはこの国のMilitarySupplies供給問題を解決しない。
/// 36か月経過後もMilitarySupplies=0のままであることを確認し、誤って
/// 「供給可能」と判定していないことを明示する。
#[test]
fn test_l1_passguard_remains_unresolved_by_this_task() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("l1");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    advance_months(&mut app, 36);

    assert_eq!(
        total_mine_level_owned_by(&app, PASSGUARD),
        0,
        "Passguard has no usable deposit either; no waste, but also no fix for its \
         IndustrialGoods chain gap"
    );
    assert_eq!(
        stockpile_of(&app, PASSGUARD, ResourceType::MilitarySupplies),
        0.0,
        "Passguard's IndustrialGoods supply chain gap is explicitly out of scope for this task \
         and must remain unresolved"
    );
}

/// Test L-2: Ferrowyn Leagueも領内に鉱床がなく、この修正だけでは自給に到達しない。
///
/// 注: Ferrowynは実データ上、既にStateId(19)へ`Mine: 1`・StateId(21)へ`Mine: 2`
/// (合計3)が初期配置されている(`assets/data/states.ron`、`verification_logs/
/// phase-26/p26-004-pre-001/`§21も「州21、既存Mine lv2が既にある州」と言及済み)。
/// これらは鉱床の有無と無関係な既存マップ上の初期建物であり、本タスクが除去・追加
/// すべき対象ではない。検証は「初期値(3)からの増加が一切ない」という形で行う。
#[test]
fn test_l2_ferrowyn_remains_unresolved_by_this_task() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("l2");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let initial_mine_level = total_mine_level_owned_by(&app, FERROWYN);
    assert_eq!(
        initial_mine_level, 3,
        "precondition: Ferrowyn's real initial data has Mine levels 1 (StateId 19) + 2 \
         (StateId 21) = 3, unrelated to any deposit"
    );

    advance_months(&mut app, 36);

    assert_eq!(
        total_mine_level_owned_by(&app, FERROWYN),
        initial_mine_level,
        "Ferrowyn has no usable deposit anywhere in its territory; its total Mine level must \
         never grow beyond the pre-existing initial value"
    );
    // P26-004-PRE-001実測: FerrowynはIron/Coalの初期備蓄だけで一時的にMilitarySupplies
    // を産出し(月9〜15頃で320.84付近に固定化)、鉱床がないため以後は増加しない。
    // この一時的な非ゼロ値自体は初期備蓄の消費によるものであり、本タスクが解決した
    // 継続的な供給網ではない(out of scope、鉱床追加なしにこれ以上は改善しない)。
    let supplies = stockpile_of(&app, FERROWYN, ResourceType::MilitarySupplies);
    assert!(
        (0.0..=400.0).contains(&supplies),
        "Ferrowyn's supply remains bounded by its one-time initial Iron/Coal stockpile, not a \
         continuous supply chain (measured: {supplies})"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Category D/E regression: Dwarfの既存供給網を壊さない
// ─────────────────────────────────────────────────────────────────────────

/// Dwarf Federationは既に鉱床(州5、Iron100/Coal80)と稼働中Mine(lv5)が同居する
/// 唯一の持続可能国。事前開発規則はこの既に稼働中の供給網に対して不必要な追加Mineを
/// 強制せず、既存の(36か月間単調増加する)生産トレンドを壊さないことを確認する。
#[test]
fn test_dwarf_supply_chain_is_not_disturbed_by_the_proactive_rule() {
    let player = ARCADIA;
    let temp_dir = TempTestDir::new("dwarf_regression");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let mut monthly_supplies = Vec::new();
    for m in 1..=36u8 {
        advance_one_month(&mut app, m);
        monthly_supplies.push(stockpile_of(&app, DWARF, ResourceType::MilitarySupplies));
    }

    // 単調非減少(消費経路が未接続のため常に非減少)。
    for window in monthly_supplies.windows(2) {
        assert!(
            window[1] >= window[0],
            "Dwarf's MilitarySupplies must remain monotonically non-decreasing: {:?}",
            window
        );
    }
    assert!(
        monthly_supplies[35] > 0.0,
        "Dwarf must still be producing MilitarySupplies after 36 months"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Category G/H/I: Save V1・New Game/Loaded Game・save/load分岐の一致
// ─────────────────────────────────────────────────────────────────────────

/// Test H-1: New GameのままAI操作を進めた場合と、同じだけ進めてsave/loadを挟んだ
/// 場合とで、Arcadiaの建設判断(選ばれた州・レベル・国庫)が完全に一致する
/// (決定論的、`process_construction_ai`は既存の国家・州・建物・資源以外の
/// 新規保存フィールドを一切参照しない)。
#[test]
fn test_h1_new_game_and_loaded_game_agree_on_arcadia_mine_construction() {
    let player = DWARF;

    let temp_dir_continuous = TempTestDir::new("h1_continuous");
    let mut continuous = setup_app_in_playing(player, temp_dir_continuous.save_file_config());
    advance_months(&mut continuous, 3);

    let temp_dir_branch = TempTestDir::new("h1_branch");
    let mut branch = setup_app_in_playing(player, temp_dir_branch.save_file_config());
    advance_months(&mut branch, 3);
    send_save_request(&mut branch);
    branch.update();
    send_load_request(&mut branch);
    branch.update();
    branch.update();

    assert_eq!(
        mine_level_at(&continuous, ARCADIA_DEPOSIT_STATE),
        mine_level_at(&branch, ARCADIA_DEPOSIT_STATE)
    );
    assert_eq!(treasury_of(&continuous, ARCADIA), treasury_of(&branch, ARCADIA));
    assert_eq!(
        stockpile_of(&continuous, ARCADIA, ResourceType::MilitarySupplies),
        stockpile_of(&branch, ARCADIA, ResourceType::MilitarySupplies)
    );
}

/// Test G-1: 建設直前(Mineがまだキューに積まれる前)にsave→loadしても、二重建設・
/// 二重支払いが起きない。
///
/// 注: 実App(`mark_ai_registries_dirty_daily`、`src/war/update.rs`)は既存仕様上、
/// 全AI国家の`CountryAiState.dirty`を日次で強制的に再セットするため、建設AIは
/// 実質「月次」ではなく「(建設キューが上限2件に達するまでの)毎日」評価される
/// (P26-004-PRE-002が変更した挙動ではない、既存のP21-020分離仕様)。したがって
/// Arcadiaは数日のうちにMine(州0)とFactory(州0、事前開発規則が解決しなかった
/// 次の通常判断)の2件でキューが上限に達する。ここで検証すべき原子性は
/// 「Mineが複数回二重に積まれない・国庫がキュー内の正規費用合計と正確に一致する」
/// ことであり、`assert_eq!(queue_len, 1)`という誤った前提ではない。
#[test]
fn test_g1_save_load_immediately_before_construction_does_not_double_charge() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("g1");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let initial_treasury = treasury_of(&app, ARCADIA);

    // Mineがまだキューに積まれていないことを確認してから、即座にsave/loadする。
    assert_eq!(construction_queue_len_of(&app, ARCADIA), 0);
    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();
    assert_eq!(construction_queue_len_of(&app, ARCADIA), 0);
    assert_eq!(treasury_of(&app, ARCADIA), initial_treasury);

    // ロード後に日次tickを進める。建設キューは上限(2件)まで埋まり、以後は
    // (`construction_queue.len() >= 2`のガードにより)それ以上増えない。
    for day in 2..=28u8 {
        tick_day(&mut app, 1800, 1, day);
    }

    let queue: Vec<_> = app
        .world()
        .resource::<CountryRegistry>()
        .get(ARCADIA)
        .unwrap()
        .construction_queue
        .clone();

    let mine_items: Vec<_> = queue
        .iter()
        .filter(|item| item.building_type == BuildingType::Mine)
        .collect();
    assert_eq!(
        mine_items.len(),
        1,
        "exactly one Mine item must be queued, never a duplicate: {:?}",
        queue
    );
    assert_eq!(mine_items[0].state_id, ARCADIA_DEPOSIT_STATE);

    // 国庫の減少額は、実際にキューへ積まれた全項目の正規費用(`paid_cost`)の合計と
    // 正確に一致する(部分適用・二重支払いがないことの直接確認)。
    let total_paid: f64 = queue.iter().map(|item| item.paid_cost).sum();
    assert_eq!(
        treasury_of(&app, ARCADIA),
        initial_treasury - total_paid,
        "treasury must decrease by exactly the sum of the queue's own paid_cost fields, with no \
         double-charging and no partial update"
    );
}

/// Test G-2: Mine建設着手後にsave→loadしても、建物レベル・国庫・建設キューが完全に
/// 一致する(Save V1のまま、新規フィールド不要で再現できることの確認)。
#[test]
fn test_g2_save_load_after_construction_starts_preserves_state_exactly() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("g2");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    advance_months(&mut app, 3);
    let treasury_before = treasury_of(&app, ARCADIA);
    let mine_level_before = mine_level_at(&app, ARCADIA_DEPOSIT_STATE);
    let queue_len_before = construction_queue_len_of(&app, ARCADIA);

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    assert_eq!(treasury_of(&app, ARCADIA), treasury_before);
    assert_eq!(mine_level_at(&app, ARCADIA_DEPOSIT_STATE), mine_level_before);
    assert_eq!(construction_queue_len_of(&app, ARCADIA), queue_len_before);
}

// ─────────────────────────────────────────────────────────────────────────
// Category M: 6か国36か月比較のスナップショット
// ─────────────────────────────────────────────────────────────────────────

/// 6か国全てを同時にAI操作(プレイヤーはいずれの供給問題にも関与しないDwarfへ設定)
/// して36か月進め、P26-004-PRE-001が記録した表(§13)と比較可能な形で
/// MilitarySupplies・Mine総レベルのスナップショットを取得する。数値は本タスクでの
/// 実測値であり、比較対象は同報告書に記録済みの改修前実測値である
/// (改修前を同一テスト内で再実行して比較したものではない)。
#[test]
fn test_m1_six_country_36_month_snapshot_matches_expected_deposit_aware_outcomes() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("m1");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let countries = [
        ("Arcadia", ARCADIA),
        ("Elfin", ELFIN),
        ("Dwarf", DWARF),
        ("Oceanic", OCEANIC),
        ("Passguard", PASSGUARD),
        ("Ferrowyn", FERROWYN),
    ];

    // 実データ上の初期Mine総レベル(鉱床の有無とは無関係な既存マップ上の初期建物を
    // 含む、`assets/data/states.ron`)。事前開発規則の検証は「この初期値から鉱床の
    // ない国が増加しないこと」で行い、「常に0であること」ではない(Elfin=1,
    // Ferrowyn=3が実データ上の初期値であり、本タスクが除去すべき対象ではない)。
    let initial_mine_totals: std::collections::HashMap<&str, u32> = countries
        .iter()
        .map(|(name, id)| (*name, total_mine_level_owned_by(&app, *id)))
        .collect();

    advance_months(&mut app, 36);

    for (name, id) in countries {
        let supplies = stockpile_of(&app, id, ResourceType::MilitarySupplies);
        let mine_total = total_mine_level_owned_by(&app, id);
        let initial_mine_total = initial_mine_totals[name];
        match name {
            "Arcadia" => {
                assert!(
                    mine_total > initial_mine_total,
                    "Arcadia must have built at least one new Mine level by month 36 \
                     (initial={initial_mine_total}, final={mine_total})"
                );
                assert!(
                    supplies > 373.96,
                    "Arcadia's month-36 supplies ({supplies}) must exceed the old permanent \
                     plateau of 373.96"
                );
            }
            "Elfin" | "Oceanic" => {
                assert_eq!(
                    mine_total, initial_mine_total,
                    "{name} has no usable deposit; Mine total level must never grow beyond its \
                     pre-existing initial value ({initial_mine_total})"
                );
                assert_eq!(supplies, 0.0, "{name}'s supply problem remains unresolved (out of scope)");
            }
            "Ferrowyn" => {
                assert_eq!(
                    mine_total, initial_mine_total,
                    "Ferrowyn has no usable deposit; Mine total level must never grow beyond its \
                     pre-existing initial value ({initial_mine_total})"
                );
                // P26-004-PRE-001実測: 初期Iron/Coal備蓄だけの一時的な非ゼロ値
                // (320.84付近で固定化)であり、継続的な供給網ではない(out of scope)。
                assert!(
                    (0.0..=400.0).contains(&supplies),
                    "Ferrowyn's supply remains bounded by its one-time initial stockpile only \
                     (measured: {supplies})"
                );
            }
            "Passguard" => {
                assert_eq!(
                    mine_total, initial_mine_total,
                    "Passguard has no usable deposit; Mine total level must never grow"
                );
                assert_eq!(
                    supplies, 0.0,
                    "Passguard's IndustrialGoods chain gap remains unresolved (out of scope)"
                );
            }
            "Dwarf" => {
                assert!(supplies > 0.0, "Dwarf's existing supply chain must remain productive");
            }
            _ => unreachable!(),
        }
    }
}
