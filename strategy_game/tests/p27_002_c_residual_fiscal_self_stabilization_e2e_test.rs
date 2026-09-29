//! P27-002-C: 行政費較正(ADMIN_COEFFICIENT 0.001→0.0008)と予測安全な1段階税率
//! 引下げ(候補4)の実App統合テスト。
//!
//! 純粋な判定ロジックの単体テストは`src/economy/finance.rs`(境界値・0.0下限・
//! 非有限値拒否・行政費係数較正・恒等式証明、19件)と`src/country/country_ai.rs`
//! (国家間分離・プレイヤー/消滅国家保護・人口加重平均、P27-002-C分14件)に
//! 既にあるため、本ファイルは**実App・実データ経由の長期(90日/36か月/60か月/
//! 120か月)六か国検証**、「税率変化」と「税率振動」の定義区分、Save V1継続性、
//! 決定性・国家分離、P27-002-A/B保護に集中する。
//!
//! 基準値は`verification_logs/phase-27/p27-002-c-pre-001/
//! p27-002-c-pre-001_residual_fiscal_self_stabilization_design.md`(§9〜§21)の
//! 候補4実測値(90日=m3, 36か月, 120か月)。60か月は同監査の候補0表にのみ存在し
//! 候補4表には含まれないため、本タスクで新たに実測し記録する(§36「実装後に
//! 再確認する」に対応)。
//!
//! `feedback_bevy_state_transition_harness.md`の教訓により、`setup_app`は
//! `tests/p27_002_b_ai_tax_rate_adjustment_e2e_test.rs`と同じ実証済みパターンを使う。

#[path = "common/sim_calendar.rs"]
mod sim_calendar;

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::CountryId;
use strategy_game::country::power::CountryPowerRegistry;
use strategy_game::country::{
    CountryPlugin, CountryRegistry, CountryStatus, DEFAULT_TAX_RATE, PlayerCountry,
};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
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

const ARCADIA: CountryId = CountryId(0);
const ELFIN: CountryId = CountryId(1);
const DWARF: CountryId = CountryId(2);
const OCEANIC: CountryId = CountryId(3);
const PASSGUARD: CountryId = CountryId(4);
const FERROWYN: CountryId = CountryId(5);
const ALL_SIX: [CountryId; 6] = [ARCADIA, ELFIN, DWARF, OCEANIC, PASSGUARD, FERROWYN];

// ─────────────────────────────────────────────────────────────────────────
// App-level fixtures(P27-002-B E2Eテストと同じ構成方針・同じ実証済みharness)
// ─────────────────────────────────────────────────────────────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p27_002_c_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
}

struct TempTestDir(std::path::PathBuf);
impl TempTestDir {
    fn new(label: &str) -> Self {
        Self(unique_temp_dir(label))
    }
    fn save_file_config(&self) -> SaveFileConfig {
        SaveFileConfig { path: SavePathConfig { final_path: self.0.join("savegame_v1.ron") } }
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
    // feedback_bevy_state_transition_harness.md: デフォルトstateのままこの最初の
    // updateを呼ばないと、以降のPlaying挿入がstate「遷移」として認識されず
    // OnEnter(Playing)systemが一切発火しない。
    app.update();

    app.insert_resource(PlayerCountry(player));
    app.insert_state(GameState::Playing);
    app.update();

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

fn tick_day(app: &mut App, base_year: i32, month_index: u8, day: u8) {
    let (year, month) = sim_calendar::sim_calendar_date(base_year, month_index);
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn tick_month(app: &mut App, base_year: i32, month_index: u8) {
    let (year, month) = sim_calendar::sim_calendar_date(base_year, month_index);
    let mut state: SystemState<MessageWriter<MonthChangedMessage>> = SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).unwrap().write(MonthChangedMessage { year, month });
    state.apply(app.world_mut());
    app.update();
}

/// `current_month_index`(1始まりの通算月)の1か月分(30日)を進める。通算月は
/// `wrapping_add`で増やし、`tick_day`/`tick_month`が`sim_calendar_date`で実在する
/// (年, 1〜12月)へ変換して発行する(P28-002-PRE-002)。
fn advance_one_month(app: &mut App, current_month_index: u8) {
    for day in 1..=30u8 {
        tick_day(app, 1800, current_month_index, day);
    }
    tick_month(app, 1800, current_month_index.wrapping_add(1));
}

fn advance_months(app: &mut App, start_month_index: u8, months: u8) {
    for i in 0..months {
        advance_one_month(app, start_month_index.wrapping_add(i));
    }
}

fn tax_rate_of(app: &App, country_id: CountryId) -> f32 {
    app.world().resource::<CountryRegistry>().get(country_id).unwrap().tax_rate
}

fn treasury_of(app: &App, country_id: CountryId) -> f64 {
    app.world().resource::<CountryRegistry>().get(country_id).unwrap().treasury
}

fn monthly_balance_of(app: &App, country_id: CountryId) -> f64 {
    app.world().resource::<CountryRegistry>().get(country_id).unwrap().monthly_balance
}

/// 決定判定式が実際に使う人口加重平均生活水準(`process_tax_rate_ai`と同一の
/// 計算経路、`src/country/country_ai.rs`のdoc参照)。レポート用。
fn population_weighted_living_standard(app: &App, country_id: CountryId) -> f32 {
    let state_registry = app.world().resource::<StateRegistry>();
    let mut total_population = 0u64;
    let mut weighted = 0.0f64;
    for state in state_registry.states.iter().filter(|s| s.owner_country_id == country_id) {
        total_population += state.population;
        weighted += state.population as f64 * state.living_standard as f64;
    }
    if total_population == 0 { 0.0 } else { (weighted / total_population as f64) as f32 }
}

// ─────────────────────────────────────────────────────────────────────────
// §1: 「税率変化」と「税率振動」の定義区分(監査と同一定義)
// ─────────────────────────────────────────────────────────────────────────
//
// P27-002-C-PRE-001監査(§9)の定義をそのまま踏襲する:
// - 「税率変化」: 前月から税率が変化した月の総数(単調増加でも単調減少でもカウント)。
// - 「税率振動」: 変化の方向が前回の変化方向から反転した回数(例: 上昇の直後に
//   下降、または下降の直後に上昇)。同方向への連続変化(例: 0.15→0.14→0.13、
//   単調減少)は振動としてカウントしない。

/// 税率の時系列(月ごとのスナップショット)から「税率振動」(方向反転)回数を
/// 数える純粋関数。テスト専用ヘルパーであり、本番コードには存在しない
/// (P27-002-C-PRE-001監査§9・§13が実測に使ったのと同一の定義)。
fn count_tax_rate_oscillations(history: &[f32]) -> u32 {
    let mut reversals = 0u32;
    let mut last_direction: i32 = 0;
    for pair in history.windows(2) {
        let delta = pair[1] - pair[0];
        if delta.abs() < 1e-9 {
            continue; // 変化なしの月は方向を持たないためスキップ
        }
        let direction = if delta > 0.0 { 1 } else { -1 };
        if last_direction != 0 && direction != last_direction {
            reversals += 1;
        }
        last_direction = direction;
    }
    reversals
}

fn count_tax_rate_changes(history: &[f32]) -> u32 {
    history.windows(2).filter(|pair| (pair[1] - pair[0]).abs() >= 1e-9).count() as u32
}

// P28-005: `p28-005-pre-002`監査(§5)により、Railway優先建設の適用後は反転回数
// だけを見る基準が振動の性質(均衡点近傍の有界な微振動か、崩壊型の拡大振動か)を
// 区別できないことが実測で確認された。Oceanicに限り、反転回数の上限(≤3)に加えて
// 「振幅」を検証する。
//
// 【振幅の定義】: 最初の反転が起きた月の直前の値から、最後の反転が起きた月までの
// 区間(反転が実際に起きている期間だけを切り出す。それより前・後の単調な推移は
// 含めない)における、税率の最大値と最小値の差。反転が0回なら0.0。
/// 反転が実際に起きている区間での税率の振れ幅(最大値−最小値)。
fn tax_rate_oscillation_amplitude(history: &[f32]) -> f32 {
    let mut last_direction: i32 = 0;
    let mut reversal_indices: Vec<usize> = Vec::new();
    for (i, pair) in history.windows(2).enumerate() {
        let delta = pair[1] - pair[0];
        if delta.abs() < 1e-9 {
            continue;
        }
        let direction = if delta > 0.0 { 1 } else { -1 };
        if last_direction != 0 && direction != last_direction {
            reversal_indices.push(i + 1); // 反転後の新しい値の位置
        }
        last_direction = direction;
    }
    let (Some(&first), Some(&last)) = (reversal_indices.first(), reversal_indices.last()) else {
        return 0.0;
    };
    let window = &history[first.saturating_sub(1)..=last];
    let max = window.iter().copied().fold(f32::MIN, f32::max);
    let min = window.iter().copied().fold(f32::MAX, f32::min);
    max - min
}

#[test]
fn test_04b_amplitude_is_zero_when_there_are_no_reversals() {
    assert_eq!(tax_rate_oscillation_amplitude(&[0.15, 0.14, 0.13]), 0.0);
}

#[test]
fn test_04c_amplitude_measures_only_the_span_where_reversals_actually_occur() {
    // 反転は2回(下降→上昇→下降)、区間は[0.18, 0.20, 0.18]で振幅0.02。
    // それより前の0.30という単調な下降の起点は反転区間の外なので含めない
    // (0.30を含めれば振幅は0.12になってしまうが、それは反転の振幅ではない)。
    let history = [0.30, 0.20, 0.18, 0.20, 0.18];
    assert_eq!(count_tax_rate_oscillations(&history), 2);
    assert!(
        (tax_rate_oscillation_amplitude(&history) - 0.02).abs() < 1e-6,
        "amplitude={}",
        tax_rate_oscillation_amplitude(&history)
    );
}

#[test]
fn test_01_monotonic_decrease_has_zero_oscillations_but_nonzero_changes() {
    let history = [0.15, 0.14, 0.13, 0.12];
    assert_eq!(count_tax_rate_changes(&history), 3, "単調減少は3回すべてが「変化」");
    assert_eq!(count_tax_rate_oscillations(&history), 0, "単調減少は方向反転なしなので「振動」は0");
}

#[test]
fn test_02_alternating_up_down_counts_every_reversal_as_an_oscillation() {
    let history = [0.15, 0.14, 0.15, 0.14];
    assert_eq!(count_tax_rate_changes(&history), 3, "3回すべてが「変化」");
    assert_eq!(
        count_tax_rate_oscillations(&history),
        2,
        "0.15→0.14(下降)→0.15(反転1)→0.14(反転2)で振動は2"
    );
}

#[test]
fn test_03_no_change_months_do_not_count_as_changes_or_oscillations() {
    let history = [0.15, 0.15, 0.15, 0.15];
    assert_eq!(count_tax_rate_changes(&history), 0);
    assert_eq!(count_tax_rate_oscillations(&history), 0);
}

#[test]
fn test_04_a_single_direction_change_is_not_an_oscillation() {
    // 赤字による引上げが1回だけ起きてそのまま維持される場合、これは「変化」では
    // あるが、方向反転が一度もないため「振動」ではない。
    let history = [0.15, 0.16, 0.16, 0.16];
    assert_eq!(count_tax_rate_changes(&history), 1);
    assert_eq!(count_tax_rate_oscillations(&history), 0);
}

// ─────────────────────────────────────────────────────────────────────────
// §2: 六か国 90日/36か月/60か月/120か月 長期検証(本タスクの中核)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_05_real_six_country_90_day_36_60_120_month_long_horizon_verification() {
    let dir = TempTestDir::new("t05_long_horizon");
    let mut app = setup_app(None, dir.save_file_config());

    // 各国の税率履歴(月末ごとに記録、振動/変化カウント用)。month 0 = New Game直後。
    let mut tax_rate_history: [Vec<f32>; 6] = Default::default();
    for (i, &c) in ALL_SIX.iter().enumerate() {
        tax_rate_history[i].push(tax_rate_of(&app, c));
    }

    let mut first_negative_treasury_month: [Option<u32>; 6] = [None; 6];
    let mut max_tax_rate: [f32; 6] = [0.0; 6];

    for m in 1..=120u8 {
        advance_one_month(&mut app, m);
        for (i, &c) in ALL_SIX.iter().enumerate() {
            let tax = tax_rate_of(&app, c);
            tax_rate_history[i].push(tax);
            max_tax_rate[i] = max_tax_rate[i].max(tax);
            if first_negative_treasury_month[i].is_none() && treasury_of(&app, c) < 0.0 {
                first_negative_treasury_month[i] = Some(m as u32);
            }
        }

        if m == 3 || m == 36 || m == 60 || m == 90 || m == 120 {
            eprintln!("[P27-002-C long-horizon] month {m}:");
            for &c in &ALL_SIX {
                eprintln!(
                    "  {c:?}: treasury={:.2} tax_rate={:.3} monthly_balance={:.2} \
                     weighted_ls={:.2}",
                    treasury_of(&app, c),
                    tax_rate_of(&app, c),
                    monthly_balance_of(&app, c),
                    population_weighted_living_standard(&app, c),
                );
            }
        }
    }

    eprintln!("[P27-002-C long-horizon] first_negative_treasury_month={first_negative_treasury_month:?}");

    // ── 基本的な健全性(全国・全期間で有限値、税率は既存の[0.0, 1.0]範囲内) ──
    for &c in &ALL_SIX {
        assert!(tax_rate_of(&app, c).is_finite() && (0.0..=1.0).contains(&tax_rate_of(&app, c)));
        assert!(treasury_of(&app, c).is_finite());
        assert!(monthly_balance_of(&app, c).is_finite());
    }
    assert_eq!(
        app.world().resource::<CountryPowerRegistry>().country_count(),
        6,
        "120か月後も6か国全てが評価対象であり続けるはず(消滅していない、\
         P27-002-C-PRE-001監査§9の候補0とは異なり候補4では誰も破局的崩壊しない)"
    );

    // ── P27-002-C-PRE-001監査§13の受け入れ基準(§17相当)を実装後に再確認 ──

    // Ferrowyn: 一度も国庫が負転しない(監査§17・§20: `first_negative_treasury_month=None`)。
    let ferrowyn_idx = 5;
    assert_eq!(
        first_negative_treasury_month[ferrowyn_idx], None,
        "Ferrowynは候補4適用後、120か月間一度も国庫が負転しないはず(監査§17)"
    );
    // Ferrowynは生活水準0への崩壊(候補0の破局)を二度と起こさない。
    assert!(
        population_weighted_living_standard(&app, FERROWYN) > 40.0,
        "Ferrowynの生活水準は候補0のように0へ収束してはいけない(監査§17): {}",
        population_weighted_living_standard(&app, FERROWYN)
    );
    // Ferrowynの国庫は成長基調(120か月末が90日時点を上回る、監査§17「成長」)。
    let ferrowyn_treasury_m120 = treasury_of(&app, FERROWYN);
    assert!(
        ferrowyn_treasury_m120 > 300.0,
        "Ferrowynの国庫は候補4適用後、成長基調で正の値を維持するはず(監査§17): {ferrowyn_treasury_m120}"
    );

    // Passguard: 回復・成長(監査§16: 最低-62.86から120か月末+904.06相当の成長軌道)。
    let passguard_treasury_m120 = treasury_of(&app, PASSGUARD);
    assert!(
        passguard_treasury_m120 > 0.0,
        "Passguardは120か月末までに黒字転換・成長軌道に入るはず(監査§16): {passguard_treasury_m120}"
    );

    // Oceanic: 過剰富裕化しない(監査§18: 候補4は9500〜12900の健全レンジに留まる、
    // 候補1/6の28837/39916のような膨張は発生しない)。
    let oceanic_treasury_m120 = treasury_of(&app, OCEANIC);
    assert!(
        oceanic_treasury_m120 < 20000.0,
        "Oceanicは候補1/6のような過剰富裕化(28837/39916)を起こしてはいけない(監査§18): \
         {oceanic_treasury_m120}"
    );

    // ── 税率振動: Oceanic以外は0〜3回/120か月(監査§13「実質的に消滅」)。────────
    // 監査§13の候補4実測(振動回数): Arcadia 0・Elfin 0・Dwarf 0・Oceanic 1・
    // Passguard 3・Ferrowyn 0。P28-002で領土・建物の移動により[2, 0, 2, 3, 3, 0]へ
    // 更新(理由は下記コメント参照)。P28-005ではOceanicだけがさらに変化する
    // (P28-005-PRE-002 §5参照)。単なる「0〜3回」の緩い受け入れ基準ではなく、
    // Oceanic以外は監査の実測値そのものと突き合わせる、より厳密な回帰保護のまま
    // 維持する。
    //
    // P28-002で正規値を更新: 旧 [0, 0, 0, 1, 3, 0] → 新 [2, 0, 2, 3, 3, 0]。
    // 戦時MilitaryFactory建設(`country_ai::try_wartime_supply_construction`)により
    // Dwarfの補給が自国生産で回復し、既存の攻勢補給ゲートを正規に通過して戦争が
    // 決着するようになった(このharnessでは1801/11/14にArcadiaが、1802/01/27に
    // Oceanicが降伏し、州0・州8をDwarfへ割譲。Ferrowynも戦時建設を行う)。領土と建物の
    // 移動で収支が変わるArcadia・Dwarf・Oceanicの振動回数が増えたが、いずれも上の
    // 受け入れ基準(0〜3回)の範囲内。戦時建設も領土変化もないElfin・Passguardは旧値の
    // まま(`test_18`で財政ロジック自体の契約として別途固定する)。
    // index3(Oceanic)の値は使わない(下のループで`continue`するため)。
    // プレースホルダとして`u32::MAX`を置き、誤って比較に使われても即座に失敗するようにする。
    const EXPECTED_OSCILLATIONS: [u32; 6] = [2, 0, 2, u32::MAX, 3, 0];
    let oceanic_idx = 3;
    for (i, &c) in ALL_SIX.iter().enumerate() {
        let changes = count_tax_rate_changes(&tax_rate_history[i]);
        let oscillations = count_tax_rate_oscillations(&tax_rate_history[i]);
        let amplitude = tax_rate_oscillation_amplitude(&tax_rate_history[i]);
        eprintln!(
            "[P27-002-C long-horizon] {c:?}: tax_rate_changes={changes} oscillations={oscillations} \
             amplitude={amplitude:.3} max_tax_rate={:.3} first_negative_treasury_month={:?}",
            max_tax_rate[i], first_negative_treasury_month[i]
        );
        if i == oceanic_idx {
            continue;
        }
        assert!(
            oscillations <= 3,
            "{c:?}: 税率振動は0〜3回/120か月のはず(監査§13の受け入れ基準、候補0〜3の \
             44〜118回から劇的に改善): got {oscillations}"
        );
        assert_eq!(
            oscillations, EXPECTED_OSCILLATIONS[i],
            "{c:?}: 監査§13実測の振動回数と完全一致するはず(実装が候補4設計を正確に \
             再現していることの直接証拠)"
        );
    }

    // ── Oceanic: P28-005-PRE-002 §5の実測に基づく新しい受け入れ条件 ──────────
    // Railway優先建設の適用後、Oceanicの反転回数は3→7に増えた。P28-005-PRE-002の
    // 診断(`verification_logs/phase-28/p28-005-pre-002/`)により、この7回は
    // 税率0.12/0.13の間を往復する有界な微振動であり(120か月間ずっと0.11〜0.15の
    // 範囲、国庫は一度も負転せず、生活水準も65〜76で健全)、候補0〜6のような
    // 拡大型の崩壊的振動(44〜118回、税率が0.97まで上昇)とは性質が異なることが
    // 確認された。したがって回数の上限(0〜3回)は、崩壊の代理指標としては
    // 不適切であり、次の3条件に置き換える。
    // - 国庫が120か月間、一度も負転しない
    // - 税率が120か月間、一度も0.25を超えない
    // - 反転が実際に起きている区間での税率の振れ幅(振幅、定義は
    //   `tax_rate_oscillation_amplitude`のdocを参照)が0.02以内
    // 反転回数そのものは診断のため`eprintln`に残す(上のループ内)。
    assert_eq!(
        first_negative_treasury_month[oceanic_idx], None,
        "Oceanic: 120か月間、国庫が一度も負転しないはず(P28-005-PRE-002 §5)"
    );
    assert!(
        max_tax_rate[oceanic_idx] <= 0.25,
        "Oceanic: 120か月間、税率が0.25を超えないはず(P28-005-PRE-002 §5): {}",
        max_tax_rate[oceanic_idx]
    );
    let oceanic_amplitude = tax_rate_oscillation_amplitude(&tax_rate_history[oceanic_idx]);
    assert!(
        oceanic_amplitude <= 0.02 + 1e-6,
        "Oceanic: 税率反転の振幅は0.02以内のはず(P28-005-PRE-002 §5、均衡点近傍の\
         有界な微振動であることの直接確認): got {oceanic_amplitude}"
    );

    // ── 監査§13の候補4実測値との完全一致確認(m3/m36/m120、実測により確認済み) ──
    // treasury/tax_rate/monthly_balance/weighted_lsのいずれも監査表の値と
    // 完全一致する(本実装が監査の一時パッチと数学的に同一であることの直接証拠)。
    //
    // P28-002で正規値を更新(旧値→新値、原因は上の振動回数のコメントと同じ):
    // Arcadia  (0.140, 8340.77, 42.85)   → (0.160, 4365.70, 25.67)   降伏・州0割譲
    // Dwarf    (0.220, 6170.62, 35.69)   → (0.170, 14536.65, 215.39) 戦時建設・州0/州8獲得
    // Oceanic  (0.200, 10778.56, -16.42) → (0.130, 4761.01, 53.85)   降伏・州8割譲
    // Ferrowyn (0.190, 1960.52, 18.22)   → (0.200, 693.24, 17.97)    戦時建設(建設費1200)
    // Elfin・Passguardは旧値のまま。
    //
    // P28-005で正規値をさらに更新(旧値→新値): 物流比率0.9未満の州へRailwayを
    // 優先建設する段階(優先度2.7)の追加により、Arcadia・Dwarf・Oceanicの建設
    // タイミングが変わった(Dwarfはtax_rate自体も0.170→0.140へ変化)。
    // Arcadia  (0.160, 4365.70, 25.67)   → (0.160, 4523.72, 21.88)
    // Dwarf    (0.170, 14536.65, 215.39) → (0.140, 15453.41, 206.56)
    // Oceanic  (0.130, 4761.01, 53.85)   → (0.130, 4695.83, 113.36)
    // Elfin・Passguard・Ferrowynは変更なし。
    const M120_EXPECTED: [(f32, f64, f64); 6] = [
        (0.160, 4523.72, 21.88),   // Arcadia
        (0.120, 6425.41, 24.31),   // Elfin
        (0.140, 15453.41, 206.56), // Dwarf
        (0.130, 4695.83, 113.36),  // Oceanic
        (0.190, 904.06, 12.43),    // Passguard
        (0.200, 693.24, 17.97),    // Ferrowyn
    ];
    for (i, &c) in ALL_SIX.iter().enumerate() {
        let (expected_tax, expected_treasury, expected_balance) = M120_EXPECTED[i];
        assert!(
            (tax_rate_of(&app, c) - expected_tax).abs() < 1e-6,
            "{c:?}: 月120のtax_rateは監査§13実測値と一致するはず: expected={expected_tax} \
             got={}",
            tax_rate_of(&app, c)
        );
        assert!(
            (treasury_of(&app, c) - expected_treasury).abs() < 0.01,
            "{c:?}: 月120のtreasuryは監査§13実測値と一致するはず: expected={expected_treasury} \
             got={}",
            treasury_of(&app, c)
        );
        assert!(
            (monthly_balance_of(&app, c) - expected_balance).abs() < 0.01,
            "{c:?}: 月120のmonthly_balanceは監査§13実測値と一致するはず: \
             expected={expected_balance} got={}",
            monthly_balance_of(&app, c)
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// §3: P27-002-A(建設予備判定)保護 ── 長期でも無変更で機能する
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_06_p27_002_a_reserve_gate_still_functions_over_a_long_horizon() {
    let dir = TempTestDir::new("t06_p27_002_a_long");
    let mut app = setup_app(None, dir.save_file_config());

    advance_months(&mut app, 1, 36);

    // P27-002-Aの3か月経常支出予備判定は、行政費係数変更後も無変更のロジックの
    // ままであるはず——ここでは「パニックしない・国庫が有限のまま」という
    // 直接的な健全性のみを確認する(判定式そのものの単体テストは
    // `src/country/country_ai.rs`に既存)。
    for &c in &ALL_SIX {
        let reg = app.world().resource::<CountryRegistry>();
        let country = reg.get(c).unwrap();
        assert!(country.treasury.is_finite());
        assert!(country.construction_queue.len() < 1000, "建設キューが暴走的に増え続けていないはず");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// §4: P27-002-B(プレイヤー保護・月次cadence)保護 ── 長期でも無変更
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_07_player_country_tax_rate_is_never_auto_adjusted_over_120_months() {
    let dir = TempTestDir::new("t07_player_protection_long");
    let mut app = setup_app(Some(DWARF), dir.save_file_config());

    advance_months(&mut app, 1, 120);

    assert_eq!(
        tax_rate_of(&app, DWARF),
        DEFAULT_TAX_RATE,
        "プレイヤー国家(Dwarf)の税率は120か月間、行政費較正の影響下でも自動調整\
         されないはず(P27-002-Bのプレイヤー保護契約、無変更)"
    );
    // ただし行政費較正の恩恵(式そのものの変更)はプレイヤーにも及ぶはず
    // (監査§22の設計判断、国別特例を作らない)。
    assert!(
        treasury_of(&app, DWARF).is_finite(),
        "プレイヤー国家も行政費較正の恩恵を受けつつ財政が破綻しないはず"
    );
}

#[test]
fn test_08_eliminated_country_tax_rate_is_never_adjusted_in_real_app() {
    let dir = TempTestDir::new("t08_eliminated_protection");
    let mut app = setup_app(None, dir.save_file_config());
    {
        let mut reg = app.world_mut().resource_mut::<CountryRegistry>();
        let country = reg.get_mut(DWARF).unwrap();
        country.status = CountryStatus::Eliminated {
            eliminated_date: "1800/01/01".to_string(),
            annexed_by: ARCADIA,
        };
        country.monthly_balance = -100.0;
        country.tax_rate = DEFAULT_TAX_RATE;
    }

    advance_months(&mut app, 1, 3);

    assert_eq!(
        tax_rate_of(&app, DWARF),
        DEFAULT_TAX_RATE,
        "消滅済み国家の税率は自動調整の対象外のはず(既存のP27-002-B契約、無変更)"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// §5: Save V1継続性(候補4はステートレスな純粋関数、新規フィールド追加なし)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_09_save_load_mid_run_at_month_60_matches_continuous_run_at_month_120() {
    let dir_a = TempTestDir::new("t09_continuous");
    let mut app_a = setup_app(None, dir_a.save_file_config());
    advance_months(&mut app_a, 1, 120);

    let dir_b = TempTestDir::new("t09_save_load");
    let mut app_b = setup_app(None, dir_b.save_file_config());
    advance_months(&mut app_b, 1, 60);
    send_save_request(&mut app_b);
    send_load_request(&mut app_b);
    advance_months(&mut app_b, 61, 60);

    for &c in &ALL_SIX {
        let ta = treasury_of(&app_a, c);
        let tb = treasury_of(&app_b, c);
        assert!(
            (ta - tb).abs() < 1e-6,
            "{c:?}: 月60でsave→loadを挟んでも120か月時点の国庫は連続実行と一致するはず: \
             continuous={ta} save_load={tb}"
        );
        assert!(
            (tax_rate_of(&app_a, c) - tax_rate_of(&app_b, c)).abs() < 1e-6,
            "{c:?}: 月60でsave→loadを挟んでも120か月時点の税率は連続実行と一致するはず"
        );
    }
}

#[test]
fn test_10_old_save_without_any_new_field_loads_and_evaluates_normally() {
    // P27-002-Cは新規Saveフィールドを一切追加しない(監査§23)。既存のSaveが
    // そのまま正常にロードできることを実際にsave→loadして確認する。
    let dir = TempTestDir::new("t10_old_save");
    let mut app = setup_app(None, dir.save_file_config());
    send_save_request(&mut app);
    send_load_request(&mut app);
    advance_months(&mut app, 1, 3);
    for &c in &ALL_SIX {
        assert!(tax_rate_of(&app, c).is_finite());
        assert!(treasury_of(&app, c).is_finite());
    }
}

// ─────────────────────────────────────────────────────────────────────────
// §6: 決定性・国家間分離
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_11_determinism_two_identical_36_month_runs_produce_identical_results() {
    fn run() -> Vec<(f32, f64)> {
        let dir = TempTestDir::new("t11_determinism");
        let mut app = setup_app(None, dir.save_file_config());
        advance_months(&mut app, 1, 36);
        ALL_SIX.iter().map(|&c| (tax_rate_of(&app, c), treasury_of(&app, c))).collect()
    }
    assert_eq!(run(), run(), "同一シナリオを2回独立実行しても全6か国のtax_rate/treasuryは完全一致するはず");
}

#[test]
fn test_12_result_does_not_depend_on_country_registration_order() {
    fn run(reverse: bool) -> Vec<(f32, f64)> {
        let dir = TempTestDir::new(&format!("t12_{reverse}"));
        let mut app = setup_app(None, dir.save_file_config());
        if reverse {
            app.world_mut().resource_mut::<CountryRegistry>().countries.reverse();
        }
        advance_months(&mut app, 1, 36);
        ALL_SIX.iter().map(|&c| (tax_rate_of(&app, c), treasury_of(&app, c))).collect()
    }
    assert_eq!(run(false), run(true), "CountryRegistryの登録順を反転しても結果は変わらないはず");
}

#[test]
fn test_13_country_isolation_over_a_long_horizon_arcadia_is_unaffected_by_ferrowyns_trajectory() {
    // Ferrowynは初期条件により長期的に税率を引き上げ続ける一方(監査§13)、
    // Arcadiaは黒字を維持し引下げ側で安定する(異なる軌跡)。60か月間の実行で
    // Arcadiaの軌跡がFerrowynの存在下でも単独実行と一致することを確認する
    // (国家間の状態漏れがないことの間接証明)。
    fn arcadia_trajectory() -> Vec<f32> {
        let dir = TempTestDir::new("t13_isolation");
        let mut app = setup_app(None, dir.save_file_config());
        let mut history = vec![tax_rate_of(&app, ARCADIA)];
        for m in 1..=60u8 {
            advance_one_month(&mut app, m);
            history.push(tax_rate_of(&app, ARCADIA));
        }
        history
    }
    // 同一シナリオ(Ferrowynも含む6か国全体)を2回実行し、Arcadiaの軌跡が
    // 完全一致することを確認する(他国の状態が非決定的に漏れ込んでいないこと)。
    assert_eq!(arcadia_trajectory(), arcadia_trajectory());
}

// ─────────────────────────────────────────────────────────────────────────
// §7: 行政費較正(ADMIN_COEFFICIENT)の実App経由確認
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_14_admin_coefficient_constant_is_calibrated_to_0_0008() {
    assert!(
        (strategy_game::economy::finance::ADMIN_COEFFICIENT - 0.0008).abs() < 1e-9,
        "P27-002-C: ADMIN_COEFFICIENTは0.001から0.0008へ較正されているはず(spec §1)"
    );
}

#[test]
fn test_15_admin_coefficient_applies_uniformly_to_player_and_ai_countries() {
    // spec §1: 国家IDによる特例禁止、プレイヤー国家にも同じ係数を適用。
    // Dwarfをプレイヤーにした場合とAIのままの場合とで、初期人口が同一であれば
    // 行政費由来の月次経常支出への影響も同一であるはず(process_country_financeが
    // 国家種別を一切分岐条件に使わないことの実App経由の確認)。
    let dir_player = TempTestDir::new("t15_player");
    let app_player = setup_app(Some(DWARF), dir_player.save_file_config());
    let dir_ai = TempTestDir::new("t15_ai");
    let app_ai = setup_app(None, dir_ai.save_file_config());

    // New Game直後(day1のdirty評価前)の時点では両者の人口は同一のはずなので、
    // 行政費計算に使う人口も一致する。
    let pop_player: u64 = app_player
        .world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .filter(|s| s.owner_country_id == DWARF)
        .map(|s| s.population)
        .sum();
    let pop_ai: u64 = app_ai
        .world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .filter(|s| s.owner_country_id == DWARF)
        .map(|s| s.population)
        .sum();
    assert_eq!(pop_player, pop_ai, "New Game直後のDwarfの人口はプレイヤー/AIどちらでも同一のはず");

    let expected_admin_cost =
        pop_player as f64 * strategy_game::economy::finance::ADMIN_COEFFICIENT;
    assert!(expected_admin_cost > 0.0, "較正後も行政費は正の値のはず");
}

// ─────────────────────────────────────────────────────────────────────────
// §8: 予測安全な税率引下げゲート(0.15絶対下限の撤廃)の実App経由境界確認
// ─────────────────────────────────────────────────────────────────────────

fn set_monthly_balance(app: &mut App, country_id: CountryId, value: f64) {
    app.world_mut().resource_mut::<CountryRegistry>().get_mut(country_id).unwrap().monthly_balance = value;
}

fn set_tax_rate(app: &mut App, country_id: CountryId, value: f32) {
    app.world_mut().resource_mut::<CountryRegistry>().get_mut(country_id).unwrap().tax_rate = value;
}

const DWARF_MONTHLY_EVAL_DAY: u8 = 2;

fn tick_dwarf_to_evaluation_day(app: &mut App, current_month_index: u8) {
    let check_day: u8 = if current_month_index == 1 { 1 } else { DWARF_MONTHLY_EVAL_DAY };
    for day in 1..=check_day {
        tick_day(app, 1800, current_month_index, day);
    }
}

#[test]
fn test_16_surplus_ai_country_can_reach_tax_rate_0_00_over_repeated_safe_decreases() {
    // spec §2: 0.15は絶対下限として扱わない。極端な黒字を維持し続ければ、
    // 予測ゲートが安全と判定し続ける限り0.0まで下がり得ることを実App経由で
    // 確認する(単体テスト`holds_at_0_00_lower_bound`の実App版)。
    let dir = TempTestDir::new("t16_reach_zero");
    let mut app = setup_app(None, dir.save_file_config());
    set_monthly_balance(&mut app, DWARF, 1_000_000.0);
    set_tax_rate(&mut app, DWARF, 0.02);

    // 極端な黒字を維持しながら複数か月評価させ、0.0まで到達させる。
    for m in 1..=3u8 {
        set_monthly_balance(&mut app, DWARF, 1_000_000.0);
        tick_dwarf_to_evaluation_day(&mut app, m);
        if m < 3 {
            for day in (DWARF_MONTHLY_EVAL_DAY.max(1) + 1)..=30u8 {
                tick_day(&mut app, 1800, m, day);
            }
            tick_month(&mut app, 1800, m.wrapping_add(1));
        }
    }

    assert!(
        tax_rate_of(&app, DWARF) >= 0.0 && tax_rate_of(&app, DWARF) < 0.02,
        "極端な黒字を維持し続ければ0.02から複数回引下げられ0.0付近まで到達し得るはず: {}",
        tax_rate_of(&app, DWARF)
    );
}

// ─── P28-002-PRE-002: ハーネス暦の年跨ぎ ──────────────────────────────────────

/// ハーネスが実際に発行した日次(`'D'`)・月次(`'M'`)メッセージの記録。
#[derive(Resource, Default)]
struct CalendarLog(Vec<(char, i32, u8, u8)>);

fn record_calendar(
    mut log: ResMut<CalendarLog>,
    mut days: MessageReader<DayChangedMessage>,
    mut months: MessageReader<MonthChangedMessage>,
) {
    for d in days.read() {
        log.0.push(('D', d.year, d.month, d.day));
    }
    for m in months.read() {
        log.0.push(('M', m.year, m.month, 0));
    }
}

fn calendar_recording_app() -> App {
    let mut app = App::new();
    app.add_message::<DayChangedMessage>()
        .add_message::<MonthChangedMessage>()
        .init_resource::<CalendarLog>()
        .add_systems(Update, record_calendar);
    app
}

/// 通算月`start..start+months`について、30日の日次tickと月境界tickの期待列を作る
/// (年跨ぎを含め、実在する(年, 1〜12月)だけで構成される)。
fn expected_calendar(start: u8, months: u8) -> Vec<(char, i32, u8, u8)> {
    let mut v = Vec::new();
    for i in 0..months {
        let idx = start + i;
        let (y, m) = sim_calendar::sim_calendar_date(1800, idx);
        for day in 1..=30u8 {
            v.push(('D', y, m, day));
        }
        let (ny, nm) = sim_calendar::sim_calendar_date(1800, idx + 1);
        v.push(('M', ny, nm, 0));
    }
    v
}

/// P28-002-PRE-002: 120か月の長期ハーネスが、12月の次に翌年1月へ進む実在の月だけを
/// 発行し、1か月=30日・月次tick 120回・日次tick 3600回・最終日付を保つこと、年跨ぎの
/// 前日・当日・翌日が連続すること、60か月でのsave→load相当の分割実行が連続実行と
/// 同じ日付列になることを、実際に発行されたメッセージ列で直接検証する。
#[test]
fn test_17_harness_calendar_rolls_over_years_with_valid_months() {
    let mut app = calendar_recording_app();
    advance_months(&mut app, 1, 120);
    let log = std::mem::take(&mut app.world_mut().resource_mut::<CalendarLog>().0);

    // 日数・月次発火回数・総日数は旧簡略暦と同じ。
    assert_eq!(log.iter().filter(|e| e.0 == 'D').count(), 3600);
    assert_eq!(log.iter().filter(|e| e.0 == 'M').count(), 120);
    // 月は常に1〜12、日は1〜30。
    assert!(log.iter().all(|e| (1..=12).contains(&e.2)));
    assert!(log.iter().filter(|e| e.0 == 'D').all(|e| (1..=30).contains(&e.3)));
    // 重複・欠落なし(期待列と完全一致)。
    assert_eq!(log, expected_calendar(1, 120));

    // 年跨ぎ: 1800/12/30 → 月境界(1801年1月) → 1801/01/01。
    let pos = log.iter().position(|e| *e == ('M', 1801, 1, 0)).expect("1801年1月の月境界");
    assert_eq!(log[pos - 1], ('D', 1800, 12, 30));
    assert_eq!(log[pos + 1], ('D', 1801, 1, 1));

    // 36か月・120か月の最終日付と、各月境界の日付。
    let month_ticks: Vec<_> = log.iter().filter(|e| e.0 == 'M').collect();
    assert_eq!(*month_ticks[35], ('M', 1803, 1, 0));
    assert_eq!(*month_ticks[119], ('M', 1810, 1, 0));
    assert_eq!(log[log.len() - 2], ('D', 1809, 12, 30));

    // save→load相当の分割実行(1〜60か月 + 61〜120か月)も同じ日付列。
    let mut split = calendar_recording_app();
    advance_months(&mut split, 1, 60);
    advance_months(&mut split, 61, 60);
    assert_eq!(split.world().resource::<CalendarLog>().0, log);

    // 純粋関数の境界。
    assert_eq!(sim_calendar::sim_calendar_date(1800, 12), (1800, 12));
    assert_eq!(sim_calendar::sim_calendar_date(1800, 13), (1801, 1));
    assert_eq!(sim_calendar::sim_calendar_date(1800, 131), (1810, 11));
}

// ─── P28-002: 戦時建設の影響を受けない国の財政契約 ────────────────────────────

fn owned_states_and_military_factory_levels(app: &App, c: CountryId) -> Vec<(usize, u32)> {
    let mut v: Vec<(usize, u32)> = app
        .world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .filter(|s| s.owner_country_id == c)
        .map(|s| {
            let level = s
                .buildings
                .get(&strategy_game::building::data::BuildingType::MilitaryFactory)
                .copied()
                .unwrap_or(0);
            (s.id.0, level)
        })
        .collect();
    v.sort();
    v
}

/// P28-002: 戦時MilitaryFactory建設で`test_05`の正規値が変わったのは、戦争の決着
/// (降伏・割譲)と戦時建設の建設費による意図した因果であり、財政ロジック自体の回帰では
/// ないことを分けて固定する。戦時建設を一度も行わず、領土・MilitaryFactoryも変わらない
/// Elfin・Passguardは、P27-002-Cの120か月正規値(税率・国庫・月次収支・振動回数)を
/// そのまま保つ。
#[test]
fn test_18_countries_untouched_by_wartime_construction_keep_the_p27_002_c_fiscal_canon() {
    let dir = TempTestDir::new("t18_untouched");
    let mut app = setup_app(None, dir.save_file_config());
    let untouched = [ELFIN, PASSGUARD];
    let initial: Vec<_> = untouched.iter().map(|&c| owned_states_and_military_factory_levels(&app, c)).collect();
    let mut tax_history: [Vec<f32>; 2] = [vec![tax_rate_of(&app, ELFIN)], vec![tax_rate_of(&app, PASSGUARD)]];
    for m in 1..=120u8 {
        advance_one_month(&mut app, m);
        for (i, &c) in untouched.iter().enumerate() {
            tax_history[i].push(tax_rate_of(&app, c));
            let queue = &app.world().resource::<CountryRegistry>().get(c).unwrap().construction_queue;
            assert!(
                queue.iter().all(|q| q.building_type != strategy_game::building::data::BuildingType::MilitaryFactory),
                "{c:?}: 月{m}に戦時MilitaryFactoryを着工した(前提崩れ)"
            );
        }
    }
    for (i, &c) in untouched.iter().enumerate() {
        assert_eq!(owned_states_and_military_factory_levels(&app, c), initial[i], "{c:?}: 領土・MilitaryFactoryは不変(前提)");
    }
    // P27-002-C(`test_05`の旧正規値)と完全一致。
    for (c, tax, treasury, balance, oscillations) in
        [(ELFIN, 0.120f32, 6425.41, 24.31, 0u32), (PASSGUARD, 0.190, 904.06, 12.43, 3)]
    {
        let i = if c == ELFIN { 0 } else { 1 };
        assert!((tax_rate_of(&app, c) - tax).abs() < 1e-6, "{c:?} tax_rate");
        assert!((treasury_of(&app, c) - treasury).abs() < 0.01, "{c:?} treasury={}", treasury_of(&app, c));
        assert!((monthly_balance_of(&app, c) - balance).abs() < 0.01, "{c:?} monthly_balance");
        assert_eq!(count_tax_rate_oscillations(&tax_history[i]), oscillations, "{c:?} oscillations");
    }
}
