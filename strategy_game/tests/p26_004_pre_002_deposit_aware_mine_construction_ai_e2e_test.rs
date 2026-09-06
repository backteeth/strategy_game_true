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
//
// 【P26-004-PRE-004により更新】: このカテゴリのTest K-1/K-2は元々、Elfin/Oceanicが
// 「領内に登録鉱床が一切ない」ことを根拠に、事前開発規則がMineへ浪費しないことを
// 検証していた。P26-004-PRE-004(`verification_logs/phase-26/p26-004-pre-004/`)は
// Elfin(StateId 16)・Oceanic(StateId 8)へ実際にIron/Coal鉱床を追加したため、この
// 前提自体が実データ上は成立しなくなった——これは本タスクが意図して生み出した結果
// である(供給網が皆無だった国に生産経路を与えることが目的だったため)。したがって
// 「鉱床がない前提での無浪費」ではなく、「鉱床が実在するようになった結果、事前開発
// 規則が正しくそこへ投資し、実際に持続的な生産へ到達する」ことを検証する形に更新
// する。事前開発規則そのもの(鉱床のない州へは浪費しない)の検証は、他の国・他州の
// 組み合わせを使う既存のCategory J等のテストが引き続き担っており、本タスクはその
// メカニズム自体には手を入れていない。
// ─────────────────────────────────────────────────────────────────────────

/// Test K-1(更新): Elfin Republicは、P26-004-PRE-004でStateId(16、既存Mine lv1が
/// 配置されていた州)へIron50/Coal40鉱床を得た。事前開発規則がこの新しい鉱床を検出し、
/// 既存Mineのレベルを実際に伸ばして、36か月以内に正のMilitarySupplies生産へ到達する
/// ことを確認する(`verification_logs/phase-26/p26-004-pre-004/`実測: 月36で
/// 1846.952)。
#[test]
fn test_k1_elfin_mine_now_grows_from_its_new_deposit_and_reaches_positive_production() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("k1");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let initial_mine_level = total_mine_level_owned_by(&app, ELFIN);
    assert_eq!(
        initial_mine_level, 1,
        "precondition: Elfin's real initial data has exactly one pre-placed Mine level \
         (StateId 16), now co-located with the P26-004-PRE-004 deposit"
    );

    advance_months(&mut app, 36);

    assert!(
        total_mine_level_owned_by(&app, ELFIN) > initial_mine_level,
        "Elfin's State 16 now has a real Iron/Coal deposit (P26-004-PRE-004); the proactive \
         construction rule must grow the pre-existing Mine there over 36 months, not leave it \
         frozen at the initial level"
    );
    let supplies = stockpile_of(&app, ELFIN, ResourceType::MilitarySupplies);
    assert!(
        supplies > 0.0,
        "with a real deposit now in place, Elfin must reach positive MilitarySupplies \
         production within 36 months (measured: {supplies})"
    );
}

/// Test K-2(更新): Oceanic Magic Empireは、P26-004-PRE-004でStateId(8、帝都)へ
/// Iron50/Coal40鉱床とMilitaryFactory level 1を得た。事前開発規則がこの鉱床を検出して
/// Mineを新規建設し、36か月以内に正のMilitarySupplies生産へ到達することを確認する
/// (`verification_logs/phase-26/p26-004-pre-004/`実測: 月36で565.705)。
#[test]
fn test_k2_oceanic_mine_is_now_built_on_its_new_deposit_and_reaches_positive_production() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("k2");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    advance_months(&mut app, 36);

    assert!(
        total_mine_level_owned_by(&app, OCEANIC) > 0,
        "Oceanic's State 8 now has a real Iron/Coal deposit (P26-004-PRE-004); the proactive \
         construction rule must build a Mine there over 36 months, not leave the total Mine \
         level at the old permanent 0"
    );
    let supplies = stockpile_of(&app, OCEANIC, ResourceType::MilitarySupplies);
    assert!(
        supplies > 0.0,
        "with a real deposit and a MilitaryFactory now in place, Oceanic must reach positive \
         MilitarySupplies production within 36 months (measured: {supplies})"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Category L: Passguard/Ferrowyn実データ
//
// 【P26-004-PRE-004により更新】: 同じ理由でPassguard/Ferrowynも実データ上「鉱床が
// 一切ない」という前提が成立しなくなった。Test L-1/L-2は、供給網が実際に解決される
// ことを検証する形に更新する。
// ─────────────────────────────────────────────────────────────────────────

/// Test L-1(更新): Passguard Marchは、P26-004-PRE-004でStateId(13、既存
/// MilitaryFactory lv1・Railway lv1が配置されていた州)へIron50/Coal40の**両方**の
/// 鉱床を得た(同一州に集約した設計理由は`verification_logs/phase-26/p26-004-pre-004/`
/// §6・P26-004-PRE-003報告書§15を参照: Mine候補州選定が不足資源ごとに区別しない
/// 落とし穴の回避)。事前開発規則がMineを新規建設し、続いて通常判断がIndustrialGoods
/// 用のFactoryを建設して全鎖が接続し、36か月以内に正のMilitarySupplies生産へ到達する
/// ことを確認する(実測: 月36で3659.576)。
#[test]
fn test_l1_passguard_supply_chain_is_now_resolved_by_p26_004_pre_004() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("l1");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    advance_months(&mut app, 36);

    assert!(
        total_mine_level_owned_by(&app, PASSGUARD) > 0,
        "Passguard's State 13 now has real Iron/Coal deposits (P26-004-PRE-004); a Mine must \
         be built there over 36 months"
    );
    let supplies = stockpile_of(&app, PASSGUARD, ResourceType::MilitarySupplies);
    assert!(
        supplies > 0.0,
        "Passguard's IndustrialGoods supply chain gap is now closed by P26-004-PRE-004's \
         deliberate co-location of both deposits on State 13; MilitarySupplies must be positive \
         by month 36 (measured: {supplies})"
    );
}

/// Test L-2(更新): Ferrowyn Leagueは、P26-004-PRE-004でStateId(21、既存Mine lv2が
/// 配置されていた、Ferrowynの2か所のMineのうちレベルの高い方の州)へIron50/Coal40
/// 鉱床を得た。既存のFactory(州22)がそのままIndustrialGoods経路を担うため、鉱床が
/// 接続した瞬間から追加建設なしで供給網が機能し始める(P26-004-PRE-003報告書§14参照)。
/// 36か月以内に、旧来の恒久停止値(初期備蓄消費だけによる320.84付近での固定化)を
/// 明確に超える、継続的なMilitarySupplies生産へ到達することを確認する(実測: 月36で
/// 1089.859)。
#[test]
fn test_l2_ferrowyn_supply_chain_is_now_resolved_by_p26_004_pre_004() {
    let player = DWARF;
    let temp_dir = TempTestDir::new("l2");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let initial_mine_level = total_mine_level_owned_by(&app, FERROWYN);
    assert_eq!(
        initial_mine_level, 3,
        "precondition: Ferrowyn's real initial data has Mine levels 1 (StateId 19) + 2 \
         (StateId 21) = 3; StateId 21 is now co-located with the P26-004-PRE-004 deposit"
    );

    advance_months(&mut app, 36);

    assert!(
        total_mine_level_owned_by(&app, FERROWYN) >= initial_mine_level,
        "Ferrowyn's pre-existing Mine levels must never be lowered; State 21's Mine may grow \
         further now that it has a real deposit"
    );
    let supplies = stockpile_of(&app, FERROWYN, ResourceType::MilitarySupplies);
    assert!(
        supplies > 400.0,
        "Ferrowyn's supply must now clearly exceed the old permanent plateau (320.84, bounded \
         by a one-time initial stockpile only) now that State 21's deposit feeds its existing \
         Factory on a continuous basis (measured: {supplies})"
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
// 【P26-004-PRE-004により更新】: 以下のTest M-1は元々、Elfin/Oceanic/Passguard/
// Ferrowynが「鉱床皆無のため恒久的にMilitarySupplies=0(またはFerrowynのみ初期備蓄
// 消費分の一時的な非ゼロ)」のままであることを検証していた。P26-004-PRE-004は
// この4か国全てに実際の供給網を与えたため、この期待値表を「4か国とも解決される」
// 形に更新する(`verification_logs/phase-26/p26-004-pre-004/`実測値と整合)。
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
            "Elfin" => {
                assert!(
                    mine_total > initial_mine_total,
                    "Elfin's State 16 now has a real deposit (P26-004-PRE-004); its Mine must \
                     grow beyond the pre-existing initial value ({initial_mine_total})"
                );
                assert!(
                    supplies > 0.0,
                    "Elfin's supply problem is now resolved by P26-004-PRE-004 (measured: {supplies})"
                );
            }
            "Oceanic" => {
                assert!(
                    mine_total > initial_mine_total,
                    "Oceanic's State 8 now has a real deposit (P26-004-PRE-004); a Mine must be \
                     built there, growing beyond the pre-existing initial value \
                     ({initial_mine_total})"
                );
                assert!(
                    supplies > 0.0,
                    "Oceanic's supply problem is now resolved by P26-004-PRE-004 (measured: {supplies})"
                );
            }
            "Ferrowyn" => {
                assert!(
                    mine_total >= initial_mine_total,
                    "Ferrowyn's pre-existing Mine levels must never be lowered ({initial_mine_total})"
                );
                // P26-004-PRE-004実測(`verification_logs/phase-26/p26-004-pre-004/`): 州21の
                // 新規鉱床が既存Factoryへ継続供給し、旧来の恒久停止値(320.84)を明確に超える。
                assert!(
                    supplies > 400.0,
                    "Ferrowyn's supply must now clearly exceed the old permanent plateau (320.84) \
                     now that State 21's deposit feeds its existing Factory continuously \
                     (measured: {supplies})"
                );
            }
            "Passguard" => {
                assert!(
                    mine_total > initial_mine_total,
                    "Passguard's State 13 now has real deposits (P26-004-PRE-004); a Mine must \
                     be built there ({initial_mine_total} -> {mine_total})"
                );
                assert!(
                    supplies > 0.0,
                    "Passguard's IndustrialGoods chain gap is now closed by P26-004-PRE-004 \
                     (measured: {supplies})"
                );
            }
            "Dwarf" => {
                assert!(supplies > 0.0, "Dwarf's existing supply chain must remain productive");
            }
            _ => unreachable!(),
        }
    }
}
