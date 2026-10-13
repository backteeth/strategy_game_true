//! P29-002-VER-002: 補給収支UI(国家経済パネルの「軍用補給物資の収支」)の残存契約を、
//! 実際の New Game 初期化・日次/月次システム・実際の募兵/復員 API・実際のパネル
//! System(`EconomyPanelPlugin`)を通して検証する。
//!
//! - 年越しの対象年月、月次集計までの実績維持
//! - 需要 0 と、募兵完成・復員に伴う需要・備蓄日数の更新(パネルを開き直さない)
//! - ロード/New Game(国家選択画面進入)で前のセッションの実績が残らない
//! - 国を変えて開始した場合、現在の国の値を表示する
//!
//! 値を直接代入して表示関数だけを確認するテストは `ui::economy_panel::tests` にある。
//! 本ファイルのテストは、それらを経路全体の検証として扱わず、実システムを通す。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::CountryId;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::economy::supply_balance::{
    MilitarySupplyProductionLedger, MonthlyProductionRecord,
};
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::data::{MilitaryRegistry, STANDARD_INFANTRY_DEFINITION_ID};
use strategy_game::military::demobilization::demobilize_division;
use strategy_game::military::recruitment::request_recruitment;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::ui::economy_panel::{EconomyPanelPlugin, EconomyPanelText};
use strategy_game::war::WarPlugin;
use strategy_game::war::data::WarRegistry;
use strategy_game::war::frontline::FrontlineRegistry;

const DWARF: CountryId = CountryId(2);
const ELFIN: CountryId = CountryId(1);

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "strategy_game_p29_002_ver002_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )))
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 実際の New Game 初期化 + 国家経済パネル(実System)。`player`で開始する。
fn setup(player: CountryId, dir: &TempDir) -> App {
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
        .add_plugins(PopulationPlugin)
        .add_plugins(EconomyPanelPlugin);
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
    app.insert_resource(SaveFileConfig {
        path: SavePathConfig {
            final_path: dir.0.join("savegame_v1.ron"),
        },
    });
    app.update();
    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update();
    app.update(); // パネルの初回描画
    app
}

fn panel_text(app: &mut App) -> String {
    // 日次システムと同じフレームでパネルが先に走った場合の1フレーム遅れを解消する(メッセージなし)。
    app.update();
    app.world_mut()
        .query_filtered::<&Text, With<EconomyPanelText>>()
        .single(app.world())
        .expect("economy panel text")
        .0
        .clone()
}

fn ledger_of(app: &App, c: CountryId) -> Option<(i32, u8, Option<f64>)> {
    app.world()
        .resource::<MilitarySupplyProductionLedger>()
        .get(c)
        .map(|e| {
            (
                e.year,
                e.month,
                match e.record {
                    MonthlyProductionRecord::Produced(v) => Some(v),
                    MonthlyProductionRecord::Unavailable => None,
                },
            )
        })
}

fn days_in_month(month: u8) -> u8 {
    [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][(month - 1) as usize]
}

/// `advance_game_date`と同じ順序で 1 日進める。月次メッセージが無い日に、プレイヤーの
/// 実績が書き換わっていないこと(日次消費で実績が変わらないこと)を毎日確認する。
fn advance_one_day(app: &mut App, year: i32, month: u8, day: u8, player: CountryId) {
    let before = ledger_of(app, player);
    {
        let mut date = app.world_mut().resource_mut::<GameDate>();
        date.year = year;
        date.month = month;
        date.day = day;
    }
    let mut state: SystemState<(
        MessageWriter<DayChangedMessage>,
        MessageWriter<MonthChangedMessage>,
    )> = SystemState::new(app.world_mut());
    {
        let (mut days, mut months) = state.get_mut(app.world_mut()).unwrap();
        days.write(DayChangedMessage { year, month, day });
        if day == 1 {
            months.write(MonthChangedMessage { year, month });
        }
    }
    state.apply(app.world_mut());
    app.update();
    if day != 1 {
        assert_eq!(
            before,
            ledger_of(app, player),
            "{year}/{month:02}/{day:02}: a daily tick must not rewrite the monthly production record"
        );
    }
}

/// (year, month, day) の次の日。
fn next_day((y, m, d): (i32, u8, u8)) -> (i32, u8, u8) {
    if d < days_in_month(m) {
        (y, m, d + 1)
    } else if m < 12 {
        (y, m + 1, 1)
    } else {
        (y + 1, 1, 1)
    }
}

/// `from`の翌日から`days`日進め、到達した日付を返す。
fn advance_days(app: &mut App, from: (i32, u8, u8), days: u32, player: CountryId) -> (i32, u8, u8) {
    let mut cur = from;
    for _ in 0..days {
        cur = next_day(cur);
        advance_one_day(app, cur.0, cur.1, cur.2, player);
    }
    cur
}

fn stock_of(app: &App, c: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(c)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitarySupplies)
}

fn living_divisions_of(app: &App, c: CountryId) -> Vec<strategy_game::common::DivisionId> {
    let mut v: Vec<_> = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == c && d.manpower > 0)
        .map(|d| d.id)
        .collect();
    v.sort_by_key(|d| d.0);
    v
}

#[allow(clippy::type_complexity)]
fn demobilize(app: &mut App, c: CountryId, id: strategy_game::common::DivisionId) {
    let mut state: SystemState<(
        ResMut<CountryRegistry>,
        ResMut<MilitaryRegistry>,
        ResMut<ArmyRegistry>,
        ResMut<FrontlineRegistry>,
        Res<WarRegistry>,
        Res<BattleRegistry>,
    )> = SystemState::new(app.world_mut());
    let (mut countries, mut military, mut armies, mut frontlines, wars, battles) =
        state.get_mut(app.world_mut()).unwrap();
    demobilize_division(
        &mut countries,
        &mut military,
        &mut armies,
        &mut frontlines,
        &wars,
        &battles,
        c,
        id,
    )
    .expect("idle own division can be demobilized (the same API as the military panel button)");
}

fn recruit_infantry(app: &mut App, c: CountryId) {
    let mut state: SystemState<(ResMut<CountryRegistry>, Res<MilitaryRegistry>)> =
        SystemState::new(app.world_mut());
    let (mut countries, military) = state.get_mut(app.world_mut()).unwrap();
    let capital = countries.get(c).unwrap().capital_state_id;
    let country = countries.countries.iter_mut().find(|x| x.id == c).unwrap();
    request_recruitment(country, &military, STANDARD_INFANTRY_DEFINITION_ID, capital)
        .expect("recruitment request (the same API as the military panel button)");
}

// ─────────────────────────────────────────────────────────────────────────
// A: 実績の対象年月(年越し・維持・更新)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn production_period_survives_the_year_rollover_and_is_held_until_the_next_monthly_tick() {
    let dir = TempDir::new("year");
    let mut app = setup(DWARF, &dir);
    // 実際の暦(1800/01/02 → 1801/02/02)を 1 日ずつ進める(日次ごとに実績が書き換わらないことも確認)。
    let mut cur = (1800, 1, 1);
    let mut seen_periods: Vec<(i32, u8)> = Vec::new();
    for _ in 0..(365 + 32) {
        cur = next_day(cur);
        advance_one_day(&mut app, cur.0, cur.1, cur.2, DWARF);
        if let Some((y, m, _)) = ledger_of(&app, DWARF)
            && seen_periods.last() != Some(&(y, m))
        {
            seen_periods.push((y, m));
        }
        // 実績の対象年月は常に「現在日付の年月」と一致する(月次処理が当日に走るため)。
        if let Some((y, m, _)) = ledger_of(&app, DWARF) {
            assert_eq!((y, m), (cur.0, cur.1), "period tracks the current month at {cur:?}");
        }
    }
    // 12 月 → 翌年 1 月をまたいで、各月が 1 回ずつ順に現れる。
    assert_eq!(seen_periods.first(), Some(&(1800, 2)));
    assert!(seen_periods.contains(&(1800, 12)));
    let dec = seen_periods.iter().position(|p| *p == (1800, 12)).unwrap();
    assert_eq!(seen_periods[dec + 1], (1801, 1), "year rollover");
    assert_eq!(seen_periods[dec + 2], (1801, 2));
    // 画面には「年/月」の形式で出る(JA)。
    let text = panel_text(&mut app);
    assert!(text.contains("実績・1801/02分)"), "{text}");
}

// ─────────────────────────────────────────────────────────────────────────
// B / D: 需要 0 と、募兵完成・復員に伴う更新(実 API・実 System)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn demand_and_stock_days_follow_real_recruitment_and_demobilization_without_reopening_the_panel() {
    let dir = TempDir::new("divisions");
    let mut app = setup(DWARF, &dir);
    let mut cur = (1800, 1, 1);
    cur = advance_days(&mut app, cur, 3, DWARF);
    let text = panel_text(&mut app);
    assert!(text.contains("現在の需要: 1.0 / 日"), "{text}");

    // 復員(実際の復員 API): 生存 Division が 0 になり、需要 0 → 「消費なし」。
    let original = living_divisions_of(&app, DWARF);
    assert_eq!(original.len(), 1);
    demobilize(&mut app, DWARF, original[0]);
    cur = advance_days(&mut app, cur, 1, DWARF);
    let text = panel_text(&mut app);
    assert!(text.contains("現在の需要: 0.0 / 日"), "{text}");
    assert!(text.contains("備蓄: 消費なし"), "{text}");
    for bad in ["NaN", "inf", "日分(追加"] {
        assert!(!text.contains(bad), "zero demand must not show a finite day count: {text}");
    }
    // 需要 0 の間は備蓄が減らない(日次消費が 0)。
    let stock_a = stock_of(&app, DWARF);
    cur = advance_days(&mut app, cur, 2, DWARF);
    assert_eq!(stock_of(&app, DWARF), stock_a);

    // 募兵(実際の募兵 API → 日次の募兵処理で完成)。完成の翌日から需要が戻る。
    recruit_infantry(&mut app, DWARF);
    cur = advance_days(&mut app, cur, 29, DWARF);
    assert_eq!(living_divisions_of(&app, DWARF).len(), 0, "not complete yet");
    assert!(panel_text(&mut app).contains("備蓄: 消費なし"));
    cur = advance_days(&mut app, cur, 2, DWARF);
    let after = living_divisions_of(&app, DWARF);
    assert_eq!(after.len(), 1, "recruited division completed");
    let text = panel_text(&mut app);
    assert!(text.contains("現在の需要: 1.0 / 日"), "{text}");
    let stock = stock_of(&app, DWARF);
    let expected_days = format!("備蓄: {:.1}日分", stock);
    assert!(text.contains(&expected_days), "expected {expected_days} in {text}");

    // 2 個目の募兵 → 需要 2.0、備蓄日数は 備蓄 ÷ 2.0。
    recruit_infantry(&mut app, DWARF);
    cur = advance_days(&mut app, cur, 32, DWARF);
    assert_eq!(living_divisions_of(&app, DWARF).len(), 2);
    let text = panel_text(&mut app);
    assert!(text.contains("現在の需要: 2.0 / 日"), "{text}");
    let expected_days = format!("備蓄: {:.1}日分", stock_of(&app, DWARF) / 2.0);
    assert!(text.contains(&expected_days), "expected {expected_days} in {text}");

    // 1 個を復員 → 需要 1.0 に戻る。
    let two = living_divisions_of(&app, DWARF);
    demobilize(&mut app, DWARF, two[1]);
    advance_days(&mut app, cur, 1, DWARF);
    let text = panel_text(&mut app);
    assert!(text.contains("現在の需要: 1.0 / 日"), "{text}");
}

// ─────────────────────────────────────────────────────────────────────────
// C: ロード後(未集計)・国を変えた開始
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn load_discards_the_previous_sessions_production_record_and_shows_pending_until_the_next_tick() {
    let dir = TempDir::new("load");
    let mut app = setup(DWARF, &dir);
    let mut cur = (1800, 1, 1);
    // 2 月・3 月の月次処理を通す。
    cur = advance_days(&mut app, cur, 31 + 28 + 1, DWARF); // 1800/03/02
    assert_eq!(cur, (1800, 3, 2));
    assert_eq!(ledger_of(&app, DWARF).map(|e| (e.0, e.1)), Some((1800, 3)));
    assert!(panel_text(&mut app).contains("実績・1800/03分)"));

    // 保存(一時ディレクトリの Save)。ユーザーの saves/savegame_v1.ron は使わない。
    {
        let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
            SystemState::new(app.world_mut());
        state.get_mut(app.world_mut()).unwrap().write(SaveRequestMessage);
        state.apply(app.world_mut());
        app.update();
    }
    assert!(dir.0.join("savegame_v1.ron").exists());

    // 保存後にさらに進めて別の実績を作る。
    advance_days(&mut app, cur, 30, DWARF); // 1800/04/01 以降
    assert_eq!(ledger_of(&app, DWARF).map(|e| (e.0, e.1)), Some((1800, 4)));

    // プレイヤー国家を別の国へ変更してからロードする(保存時の国へ戻る)。
    app.insert_resource(PlayerCountry(Some(ELFIN)));
    {
        let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
            SystemState::new(app.world_mut());
        state.get_mut(app.world_mut()).unwrap().write(LoadRequestMessage);
        state.apply(app.world_mut());
        app.update();
        app.update();
    }
    assert_eq!(
        app.world().resource::<PlayerCountry>().0,
        Some(DWARF),
        "the loaded save restores the saved player country"
    );
    assert!(
        app.world().resource::<MilitarySupplyProductionLedger>().is_empty(),
        "no record from before the load survives (the record is not part of Save V1)"
    );
    let text = panel_text(&mut app);
    assert!(text.contains("前回の月次生産(実績): 未集計"), "{text}");
    assert!(!text.contains("実績・"), "{text}");
    // 需要・備蓄日数は読み込まれた世界から通常どおり表示される。
    assert!(text.contains("現在の需要:"), "{text}");

    // ロード後の最初の月次処理で再び記録される(日付はセーブ時点 1800/03/02 から進める)。
    let mut cur = (1800, 3, 2);
    cur = advance_days(&mut app, cur, 30, DWARF); // 1800/04/01
    assert_eq!(cur, (1800, 4, 1));
    assert_eq!(ledger_of(&app, DWARF).map(|e| (e.0, e.1)), Some((1800, 4)));
    assert!(panel_text(&mut app).contains("実績・1800/04分)"));
}

#[test]
fn starting_with_a_different_country_shows_that_countrys_values_only() {
    let dir_a = TempDir::new("dwarf");
    let dir_b = TempDir::new("elfin");
    let mut dwarf = setup(DWARF, &dir_a);
    let mut elfin = setup(ELFIN, &dir_b);
    // 開始直後はどちらも未集計(別セッションの値は存在しない)。
    for app in [&mut dwarf, &mut elfin] {
        assert!(app.world().resource::<MilitarySupplyProductionLedger>().is_empty());
        assert!(panel_text(app).contains("未集計"));
    }
    let to_march = |app: &mut App, c: CountryId| {
        advance_days(app, (1800, 1, 1), 31 + 28 + 1, c);
    };
    to_march(&mut dwarf, DWARF);
    to_march(&mut elfin, ELFIN);

    let dwarf_entry = ledger_of(&dwarf, DWARF).unwrap();
    let elfin_entry = ledger_of(&elfin, ELFIN).unwrap();
    let dwarf_text = panel_text(&mut dwarf);
    let elfin_text = panel_text(&mut elfin);
    let fmt = |v: f64| format!("{:.1}", v);
    assert!(
        dwarf_text.contains(&format!("実績・1800/03分): {}", fmt(dwarf_entry.2.unwrap()))),
        "{dwarf_text}"
    );
    assert!(
        elfin_text.contains(&format!("実績・1800/03分): {}", fmt(elfin_entry.2.unwrap()))),
        "{elfin_text}"
    );
    // 各セッションの備蓄日数は、自国の備蓄 ÷ 自国の需要。
    let stock = stock_of(&elfin, ELFIN);
    assert!(elfin_text.contains(&format!("備蓄: {:.1}日分", stock)), "{elfin_text}");
    // 国ごとに値が異なる(Dwarf の値が Elfin の画面に出ない)。
    assert_ne!(
        fmt(dwarf_entry.2.unwrap()),
        fmt(elfin_entry.2.unwrap()),
        "the two countries produce different amounts in March, so a leak would be visible"
    );
    assert!(!elfin_text.contains(&fmt(dwarf_entry.2.unwrap())));
}
