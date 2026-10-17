//! P30-002: 軍需工場の再建の実 App fixture。Oceanic に国内供給(Mine・Factory・鉱床)を与えた
//! 仮想の状態(状態を操作した補助実験。通常シナリオの再現ではない)で、AI の判断・建設・完成・月次生産・
//! 募兵までを既存の経路だけで進める。物資の無償生成・補給ゲートの免除は使わない。
use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::MilitaryRegistry;
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

fn setup_app(dir: &std::path::Path) -> App {
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
    app.insert_resource(SaveFileConfig {
        path: SavePathConfig {
            final_path: dir.join("savegame_v1.ron"),
        },
    });
    app.update();
    app.insert_resource(PlayerCountry(None));
    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn advance_one_day(app: &mut App) {
    let (old_month, next) = {
        let d = app.world().resource::<GameDate>();
        (d.month, d.add_days(1))
    };
    {
        let mut d = app.world_mut().resource_mut::<GameDate>();
        d.year = next.year;
        d.month = next.month;
        d.day = next.day;
    }
    let mut s: SystemState<(
        MessageWriter<DayChangedMessage>,
        MessageWriter<MonthChangedMessage>,
    )> = SystemState::new(app.world_mut());
    {
        let (mut dw, mut mw) = s.get_mut(app.world_mut()).unwrap();
        dw.write(DayChangedMessage {
            year: next.year,
            month: next.month,
            day: next.day,
        });
        if next.month != old_month {
            mw.write(MonthChangedMessage {
                year: next.year,
                month: next.month,
            });
        }
    }
    s.apply(app.world_mut());
    app.update();
}




const OCEANIC: strategy_game::common::CountryId = strategy_game::common::CountryId(3);

fn send_save(app: &mut App) {
    use strategy_game::save::SaveRequestMessage;
    let mut s: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut()).unwrap().write(SaveRequestMessage);
    s.apply(app.world_mut());
    app.update();
}

fn send_load(app: &mut App) {
    use strategy_game::save::runtime::LoadRequestMessage;
    let mut s: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut()).unwrap().write(LoadRequestMessage);
    s.apply(app.world_mut());
    app.update();
    app.update();
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "strategy_game_p30_002_{label}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// 仮想の状態(状態を操作): Oceanic の MilitaryFactory を全て 0 にし、州 9 に Iron・Coal の鉱床と
/// 稼働できる Mine lv2・Factory lv2 を与える(= 国内で供給できる「喪失後」の国家)。
fn make_supplied_but_without_military_factory(app: &mut App) {
    use strategy_game::building::data::BuildingType as B;
    use strategy_game::economy::resources::StateResourceDeposit;
    let mut sr = app.world_mut().resource_mut::<StateRegistry>();
    for s in sr.states.iter_mut().filter(|s| s.owner_country_id == OCEANIC) {
        s.buildings.remove(&B::MilitaryFactory);
        if s.id.0 == 9 {
            s.resource_deposits = vec![
                StateResourceDeposit {
                    resource_type: ResourceType::Iron,
                    base_output: 50.0,
                    discovered: true,
                    development_level: 1,
                },
                StateResourceDeposit {
                    resource_type: ResourceType::Coal,
                    base_output: 40.0,
                    discovered: true,
                    development_level: 1,
                },
            ];
            s.buildings.insert(B::Mine, 2);
            s.buildings.insert(B::Factory, 2);
        }
    }
}

/// Oceanic の MilitaryFactory(レベル合計)・建設キュー・師団・備蓄の要約。
struct Snap {
    mf_levels: u32,
    mf_queued: Vec<(usize, u32)>,
    living: usize,
    ms: f64,
    fe: f64,
    ig: f64,
    treasury: f64,
}

fn snap(app: &App) -> Snap {
    use strategy_game::building::data::BuildingType as B;
    let sr = app.world().resource::<StateRegistry>();
    let cr = app.world().resource::<CountryRegistry>();
    let mil = app.world().resource::<MilitaryRegistry>();
    let c = cr.get(OCEANIC).unwrap();
    Snap {
        mf_levels: sr
            .states
            .iter()
            .filter(|s| s.owner_country_id == OCEANIC)
            .map(|s| s.building_level(B::MilitaryFactory))
            .sum(),
        mf_queued: c
            .construction_queue
            .iter()
            .filter(|q| q.building_type == B::MilitaryFactory)
            .map(|q| (q.state_id.0, q.target_level))
            .collect(),
        living: mil
            .divisions
            .values()
            .filter(|d| d.owner == OCEANIC && d.manpower > 0)
            .count(),
        ms: c.stockpile.get(ResourceType::MilitarySupplies),
        fe: c.stockpile.get(ResourceType::Iron),
        ig: c.stockpile.get(ResourceType::IndustrialGoods),
        treasury: c.treasury,
    }
}

/// 国内供給がある Oceanic(仮想)が、AI の判断だけで MilitaryFactory を建設し、完成・月次生産・募兵まで
/// 進み、Iron が継続供給されて生産が止まらない(補助実験の Iron 枯渇で停止した経路との対比)。
#[test]
fn a_supplied_country_rebuilds_produces_and_recruits_through_the_real_paths() {
    use strategy_game::building::data::BuildingType as B;
    let dir = temp_dir("rebuild");
    let mut app = setup_app(&dir);
    while app.world().resource::<GameDate>().display().as_str() < "1801/08/01" {
        advance_one_day(&mut app);
    }
    make_supplied_but_without_military_factory(&mut app);
    let treasury_before = snap(&app).treasury;
    let mut queued_on: Option<String> = None;
    let mut completed_on: Option<String> = None;
    let mut first_division_on: Option<String> = None;
    let mut max_ms = 0.0f64;
    let mut last_mf_op = 0.0f32;
    let mut mf_op_zero_months_after_completion = 0u32;
    while app.world().resource::<GameDate>().year < 1806 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        let s = snap(&app);
        if queued_on.is_none() && !s.mf_queued.is_empty() {
            queued_on = Some(date.display());
            println!("REBUILD_QUEUED {} queue={:?} treasury={:.0}", date.display(), s.mf_queued, s.treasury);
        }
        if completed_on.is_none() && s.mf_levels > 0 {
            completed_on = Some(date.display());
            println!("REBUILD_COMPLETED {}", date.display());
        }
        if first_division_on.is_none() && s.living > 0 {
            first_division_on = Some(date.display());
            println!("FIRST_DIVISION {} MS={:.1}", date.display(), s.ms);
        }
        max_ms = max_ms.max(s.ms);
        if date.day == 1 && completed_on.is_some() {
            let sr = app.world().resource::<StateRegistry>();
            let op = sr
                .states
                .iter()
                .filter(|st| st.owner_country_id == OCEANIC)
                .filter_map(|st| st.building_operation_ratios.get(&B::MilitaryFactory).copied())
                .fold(0.0f32, f32::max);
            last_mf_op = op;
            if op <= 0.0 {
                mf_op_zero_months_after_completion += 1;
            }
            if date.month == 1 || date.month == 7 {
                println!(
                    "REBUILD_TRACK {} MF_op={op:.2} MS={:.1} living={} Fe={:.0} IG={:.0}",
                    date.display(),
                    s.ms,
                    s.living,
                    s.fe,
                    s.ig
                );
            }
        }
    }
    let end = snap(&app);
    println!(
        "REBUILD_SUMMARY queued={queued_on:?} completed={completed_on:?} first_division={first_division_on:?} max_ms={max_ms:.1} end_living={} end_ms={:.1} end_mf_op={last_mf_op:.2} mf_op_zero_months={mf_op_zero_months_after_completion} treasury {treasury_before:.0}->{:.0}",
        end.living, end.ms, end.treasury
    );
    assert!(queued_on.is_some(), "AI が MilitaryFactory を着工する(手動の注文ではない)");
    assert!(completed_on.is_some(), "建設が完成する");
    assert!(max_ms > 31.0, "備蓄が橋渡し量を超える(max {max_ms})");
    assert!(first_division_on.is_some(), "既存の募兵経路で募兵される(ゲート免除・無償生成なし)");
    // 原料が継続供給される: Mine と Factory が稼働し、MilitaryFactory が完成後も止まらない。
    assert!(last_mf_op > 0.0, "完成から数年後も MilitaryFactory が稼働している");
    assert_eq!(mf_op_zero_months_after_completion, 0, "Iron 枯渇による停止がない");
}

fn rebuild_context_is_some(app: &App) -> bool {
    use strategy_game::country::country_ai::{CountryAiRegistry, military_rebuild_context};
    let cr = app.world().resource::<CountryRegistry>();
    let sr = app.world().resource::<StateRegistry>();
    let mil = app.world().resource::<MilitaryRegistry>();
    let ai = app.world().resource::<CountryAiRegistry>();
    let date = app.world().resource::<GameDate>().display();
    let empty = Default::default();
    military_rebuild_context(
        cr.get(OCEANIC).unwrap(),
        sr,
        mil,
        ai.supply_production_history.get(&OCEANIC).unwrap_or(&empty),
        &date,
    )
    .is_some()
}

/// プレイヤー国家には自動で建設しない。
#[test]
fn the_player_country_is_never_rebuilt_automatically() {
    let dir = temp_dir("player");
    let mut app = setup_app(&dir);
    app.insert_resource(PlayerCountry(Some(OCEANIC)));
    while app.world().resource::<GameDate>().display().as_str() < "1801/08/01" {
        advance_one_day(&mut app);
    }
    make_supplied_but_without_military_factory(&mut app);
    while app.world().resource::<GameDate>().year < 1803 {
        advance_one_day(&mut app);
        let s = snap(&app);
        assert!(s.mf_queued.is_empty(), "プレイヤー国には自動建設しない");
    }
}

/// Save/Load: 同じ状態から同じ判断(着工日が一致)、旧 Save(履歴なし)では履歴が揃うまで保留。
/// 判断は登録済みの状態と P29-005 の履歴から再導出される(新しい永続状態なし)。
#[test]
fn save_load_reproduces_the_decision_and_an_old_save_waits_for_the_history() {
    use strategy_game::save::dto::SaveGameV1;
    let dir = temp_dir("save");
    let mut app = setup_app(&dir);
    // Playing 中の Save にはプレイヤー国家が必要(Passguard をプレイヤーにする。Oceanic は AI)。
    app.insert_resource(PlayerCountry(Some(strategy_game::common::CountryId(4))));
    while app.world().resource::<GameDate>().display().as_str() < "1801/08/01" {
        advance_one_day(&mut app);
    }
    make_supplied_but_without_military_factory(&mut app);
    let first_queue_date = |app: &mut App, limit: &str| -> Option<String> {
        while app.world().resource::<GameDate>().display().as_str() < limit {
            advance_one_day(app);
            if !snap(app).mf_queued.is_empty() {
                return Some(app.world().resource::<GameDate>().display());
            }
        }
        None
    };
    send_save(&mut app);
    let saved_date = app.world().resource::<GameDate>().display();
    let saved_ym = {
        let d = app.world().resource::<GameDate>();
        (d.year, d.month)
    };
    let d1 = first_queue_date(&mut app, "1803/01/01").expect("着工される");
    // ロードして、同じ判断が同じ日に起きる。
    send_load(&mut app);
    assert_eq!(app.world().resource::<GameDate>().display(), saved_date, "保存時点へ戻る");
    assert!(snap(&app).mf_queued.is_empty());
    let d2 = first_queue_date(&mut app, "1803/01/01").expect("ロード後も着工される");
    assert_eq!(d1, d2, "同じ状態から同じ判断");

    // 旧 Save(P29-005 の生産履歴を持たない)を再現して読む。履歴が 4 か月揃うまで保留される。
    let path = dir.join("savegame_v1.ron");
    let text = std::fs::read_to_string(&path).unwrap();
    let mut save: SaveGameV1 = ron::from_str(&text).unwrap();
    save.country_ai.supply_production_history.clear();
    std::fs::write(
        &path,
        ron::ser::to_string_pretty(&save, ron::ser::PrettyConfig::default()).unwrap(),
    )
    .unwrap();
    send_load(&mut app);
    assert_eq!(app.world().resource::<GameDate>().display(), saved_date);
    // 旧 Save: 履歴が 4 か月揃うまで再建需要を認識しない(保留)。揃った月から認識する。
    let months_since_load = |app: &App| {
        let d = app.world().resource::<GameDate>();
        let (y, m) = saved_ym;
        (d.year - y) * 12 + d.month as i32 - m as i32
    };
    let mut first_context_month: Option<i32> = None;
    let mut context_seen_before_history_full = false;
    while months_since_load(&app) <= 8 {
        advance_one_day(&mut app);
        let some = rebuild_context_is_some(&app);
        let m = months_since_load(&app);
        if some && first_context_month.is_none() {
            first_context_month = Some(m);
        }
        if some && m < 4 {
            context_seen_before_history_full = true;
        }
    }
    assert!(!context_seen_before_history_full, "旧 Save では 4 か月の履歴が揃うまで保留する");
    let first = first_context_month.expect("履歴が揃った後は再建需要を認識する");
    assert!((4..=5).contains(&first), "ロードの 4〜5 か月後から認識する(実際: {first})");
}
