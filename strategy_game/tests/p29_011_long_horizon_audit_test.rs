//! P29-011: Phase 29 統合監査の長期確認。標準全 AI シナリオを 1800/01/01 から 1950/01/01 まで
//! (1800 か月)実際の日次・月次システムで進め、パニック・非有限値・不正な状態・履歴の肥大・
//! 建設キューの異常・恒久的に解除されないゲートの有無を確認する。国家の存続・戦争の終結・
//! 国庫が常に正であることは合否にしない(ゲーム内で起こり得る敗北・赤字・膠着は記録のみ)。
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
use strategy_game::war::data::{WarRegistry, WarStatus};

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


#[test]
fn p29_011_long_horizon_audit_1800_to_1950() {
    use strategy_game::country::country_ai::CountryAiRegistry;
    use strategy_game::economy::factory_starvation::FACTORY_STARVATION_WINDOW;
    use strategy_game::economy::iron_shortage::IRON_SHORTAGE_WINDOW;
    use strategy_game::military::supply_production_history::SUPPLY_HISTORY_WINDOW;

    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p29_011_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);

    let mut ended_wars: std::collections::BTreeSet<usize> = Default::default();
    let mut blocked_streak = [0u32; 6];
    let mut blocked_max = [0u32; 6];
    let mut max_construction_queue = 0usize;
    let mut max_recruitment_queue = 0usize;
    let mut min_treasury = [f64::MAX; 6];
    let mut negative_treasury_months = [0u32; 6];
    let mut eliminated_at: [Option<String>; 6] = Default::default();
    let mut unified: Option<(String, usize)> = None;
    let mut months = 0u32;

    while app.world().resource::<GameDate>().year < 1950 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        if date.day != 1 {
            continue;
        }
        months += 1;
        let cr = app.world().resource::<CountryRegistry>();
        let sr = app.world().resource::<StateRegistry>();
        let mil = app.world().resource::<MilitaryRegistry>();
        let wr = app.world().resource::<WarRegistry>();
        let ai = app.world().resource::<CountryAiRegistry>();

        let mut active = 0usize;
        let mut last_active = 0usize;
        for c in &cr.countries {
            let i = c.id.0;
            // 非有限値・不正な値(パニックしない・NaN を作らない)。
            assert!(c.treasury.is_finite(), "{} c{i}: treasury {}", date.display(), c.treasury);
            assert!(
                c.tax_rate.is_finite() && (0.0..=1.0).contains(&c.tax_rate),
                "{} c{i}: tax {}",
                date.display(),
                c.tax_rate
            );
            for r in ResourceType::ALL {
                let v = c.stockpile.get(r);
                assert!(v.is_finite() && v >= 0.0, "{} c{i}: {r:?} = {v}", date.display());
            }
            max_construction_queue = max_construction_queue.max(c.construction_queue.len());
            max_recruitment_queue = max_recruitment_queue.max(c.recruitment_queue.len());
            min_treasury[i] = min_treasury[i].min(c.treasury);
            if c.treasury < 0.0 {
                negative_treasury_months[i] += 1;
            }
            if c.is_active() {
                active += 1;
                last_active = i;
            }
            let owned = sr.states.iter().filter(|s| s.owner_country_id == c.id).count();
            if owned == 0 && eliminated_at[i].is_none() {
                eliminated_at[i] = Some(date.display());
            }
            // 履歴の肥大(窓幅を超えない)・州を持たない国の履歴は消去される。
            if let Some(h) = ai.supply_production_history.get(&c.id) {
                assert!(h.samples().len() <= SUPPLY_HISTORY_WINDOW);
            }
            if let Some(h) = ai.iron_shortage_history.get(&c.id) {
                assert!(h.samples().len() <= IRON_SHORTAGE_WINDOW);
            }
            if let Some(h) = ai.factory_starvation_history.get(&c.id) {
                assert!(h.samples().len() <= FACTORY_STARVATION_WINDOW);
            }
            if owned == 0 {
                assert!(
                    !ai.supply_production_history.contains_key(&c.id)
                        && !ai.iron_shortage_history.contains_key(&c.id)
                        && !ai.factory_starvation_history.contains_key(&c.id),
                    "{} c{i}: 州を持たない国の履歴が残っている",
                    date.display()
                );
            }
            // 恒久的に解除されないゲート(募兵の補給ゲートの停止状態の連続月数)。
            let blocked = ai
                .ai_states
                .get(&c.id)
                .is_some_and(|s| s.recruitment_supply_blocked);
            if blocked && owned > 0 {
                blocked_streak[i] += 1;
                blocked_max[i] = blocked_max[i].max(blocked_streak[i]);
            } else {
                blocked_streak[i] = 0;
            }
        }
        // 戦争の状態遷移: 終結した戦争が Active に戻らない・停滞記録は Active な戦争だけ。
        for (id, w) in &wr.wars {
            if w.status != WarStatus::Active {
                ended_wars.insert(id.0);
                assert!(w.end_date.is_some(), "{}: war {} ended without end_date", date.display(), id.0);
            } else {
                assert!(!ended_wars.contains(&id.0), "{}: war {} re-activated", date.display(), id.0);
            }
        }
        for id in wr.stalemate_records.keys() {
            let w = wr.wars.get(id).expect("停滞記録の戦争が存在する");
            assert!(w.status == WarStatus::Active, "{}: 終結した戦争の停滞記録が残っている", date.display());
        }
        let _ = mil;
        if active <= 1 {
            unified = Some((date.display(), last_active));
            break;
        }

        // 節目の要約。
        if date.month == 1 && date.year % 50 == 0 {
            println!("SUMMARY {}", date.display());
            for c in &cr.countries {
                let owned: Vec<_> = sr.states.iter().filter(|s| s.owner_country_id == c.id).collect();
                let lv = |b: strategy_game::building::data::BuildingType| -> u32 {
                    owned.iter().map(|s| s.building_level(b)).sum()
                };
                let divs = mil
                    .divisions
                    .values()
                    .filter(|d| d.owner == c.id && d.manpower > 0)
                    .count();
                println!(
                    "  c{} active={} states={} divs={} treasury={:.0} tax={:.2} MS={:.0} Fe={:.0} IG={:.0} lv[Mine={} F={} MF={}] queue c={} r={}",
                    c.id.0,
                    c.is_active(),
                    owned.len(),
                    divs,
                    c.treasury,
                    c.tax_rate,
                    c.stockpile.get(ResourceType::MilitarySupplies),
                    c.stockpile.get(ResourceType::Iron),
                    c.stockpile.get(ResourceType::IndustrialGoods),
                    lv(strategy_game::building::data::BuildingType::Mine),
                    lv(strategy_game::building::data::BuildingType::Factory),
                    lv(strategy_game::building::data::BuildingType::MilitaryFactory),
                    c.construction_queue.len(),
                    c.recruitment_queue.len(),
                );
            }
        }
    }

    let date = app.world().resource::<GameDate>().display();
    println!("END date={date} months={months} unified={unified:?}");
    println!("MAX construction_queue={max_construction_queue} recruitment_queue={max_recruitment_queue}");
    for i in 0..6 {
        println!(
            "C c{i} min_treasury={:.0} negative_months={} eliminated_at={:?} longest_supply_gate_block_months={}",
            min_treasury[i], negative_treasury_months[i], eliminated_at[i], blocked_max[i]
        );
    }
    let wr = app.world().resource::<WarRegistry>();
    let active: Vec<_> = wr.wars.values().filter(|w| w.status == WarStatus::Active).collect();
    println!("WARS total={} active={} ended={}", wr.wars.len(), active.len(), wr.wars.len() - active.len());
    for w in active {
        let mut a: Vec<_> = w.attackers.iter().map(|c| c.0).collect();
        let mut d: Vec<_> = w.defenders.iter().map(|c| c.0).collect();
        a.sort();
        d.sort();
        println!("ACTIVE_WAR att={a:?} def={d:?} start={} duration_days={}", w.start_date, w.duration_days);
    }
    // 構造的な契約(合否): 1800 か月(または統一まで)を完走し、キューが有界。
    assert!(months >= 1 || unified.is_some());
    assert!(max_construction_queue <= 10, "construction queue {max_construction_queue}");
    assert!(max_recruitment_queue <= 10, "recruitment queue {max_recruitment_queue}");
}

// ── 旧 Save(Phase 29 の履歴フィールドなし)の読み込みの統合確認 ──────────────────

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

/// 旧 Save(Phase 29 で追加した 3 つの履歴を持たない)を実ロード経路で読むと、履歴は空(未集計)に
/// なり、各ゲートは「未集計」の仕様どおりに従来の判断へ委ねられる(E3 の通常経路は保留、B1・B2
/// は不発)。過去の月は推測されず、次の月次生産から計測し直す。一時セーブだけを使う。
#[test]
fn p29_011_old_save_without_phase29_histories_loads_as_unobserved_and_refills() {
    use strategy_game::country::country_ai::{
        CountryAiRegistry, factory_expansion_blocked, iron_supply_context,
    };
    use strategy_game::save::dto::SaveGameV1;

    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p29_011_old_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    // Playing 中の Save にはプレイヤー国家が必要(プレイヤー国は AI の履歴を持たない)。
    app.insert_resource(PlayerCountry(Some(strategy_game::common::CountryId(4))));
    for _ in 0..(31 + 28 + 31 + 30 + 31 + 30 + 5) {
        advance_one_day(&mut app);
    }
    {
        let ai = app.world().resource::<CountryAiRegistry>();
        assert!(!ai.supply_production_history.is_empty());
        assert!(!ai.iron_shortage_history.is_empty());
        assert!(!ai.factory_starvation_history.is_empty());
    }
    send_save(&mut app);
    let path = dir.join("savegame_v1.ron");
    let text = std::fs::read_to_string(&path).expect("一時セーブが書かれている");
    let mut save: SaveGameV1 = ron::from_str(&text).expect("新しい Save が読める");
    assert!(!save.country_ai.supply_production_history.is_empty(), "新しい Save は履歴を保存する");
    // 旧 Save を再現する: Phase 29 の履歴を持たない状態(serde の default で空になる)。
    save.country_ai.supply_production_history.clear();
    save.country_ai.iron_shortage_history.clear();
    save.country_ai.factory_starvation_history.clear();
    std::fs::write(
        &path,
        ron::ser::to_string_pretty(&save, ron::ser::PrettyConfig::default()).unwrap(),
    )
    .unwrap();

    for _ in 0..40 {
        advance_one_day(&mut app);
    }
    let date_before_load = app.world().resource::<GameDate>().display();
    send_load(&mut app);
    assert_ne!(
        app.world().resource::<GameDate>().display(),
        date_before_load,
        "ロードが成功して日付が保存時点へ戻る"
    );
    {
        let ai = app.world().resource::<CountryAiRegistry>();
        assert!(ai.supply_production_history.is_empty(), "旧 Save: 過去の月を推測しない");
        assert!(ai.iron_shortage_history.is_empty());
        assert!(ai.factory_starvation_history.is_empty());
        let date = app.world().resource::<GameDate>().display();
        let sr = app.world().resource::<StateRegistry>();
        for c in 0..6usize {
            let id = strategy_game::common::CountryId(c);
            let blocked = ai
                .factory_starvation_history
                .get(&id)
                .is_some_and(|h| factory_expansion_blocked(h, &date, id, sr));
            assert!(!blocked, "未集計の間 B2 は不発(従来の判断)");
            let empty = Default::default();
            assert!(
                iron_supply_context(
                    ai.iron_shortage_history.get(&id).unwrap_or(&empty),
                    &date,
                    id,
                    sr
                )
                .is_none(),
                "未集計の間 B1 は不発(従来の判断)"
            );
        }
    }
    // 次の月次生産から計測し直す(4 か月で窓が埋まる)。
    for _ in 0..(31 * 5) {
        advance_one_day(&mut app);
    }
    let ai = app.world().resource::<CountryAiRegistry>();
    assert!(!ai.supply_production_history.is_empty(), "ロード後に再計測される");
    assert!(!ai.iron_shortage_history.is_empty());
    assert!(!ai.factory_starvation_history.is_empty());
}

#[test]
fn p29_011_arcadia_iron_availability() {
    use strategy_game::building::data::BuildingType as B;
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p29_011_arc_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let app = setup_app(&dir);
    let sr = app.world().resource::<StateRegistry>();
    let cr = app.world().resource::<CountryRegistry>();
    let me = strategy_game::common::CountryId(0);
    println!("ARC capital={:?}", cr.get(me).unwrap().capital_state_id);
    for s in sr.states.iter().filter(|s| s.owner_country_id == me) {
        let dep: Vec<String> = s
            .resource_deposits
            .iter()
            .map(|d| format!("{:?}:{:.0}:disc={}", d.resource_type, d.base_output, d.discovered))
            .collect();
        println!(
            "ARC state={} {} sea={} deposits=[{}] buildings={:?}",
            s.id.0, s.name, s.is_sea, dep.join(", "), s.buildings
        );
    }
    // 全世界の Iron 鉱床(所有者別)。
    let mut by_owner: std::collections::BTreeMap<usize, (u32, f64)> = Default::default();
    for s in sr.states.iter() {
        for d in &s.resource_deposits {
            if d.resource_type == ResourceType::Iron && d.discovered {
                let e = by_owner.entry(s.owner_country_id.0).or_default();
                e.0 += 1;
                e.1 += d.base_output;
            }
        }
    }
    println!("IRON_DEPOSITS_BY_OWNER (count, base_output sum) {by_owner:?}");
    let _ = B::Mine;
}
