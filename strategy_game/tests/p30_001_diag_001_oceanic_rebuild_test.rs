//! P30-001-DIAG-001: 軍需供給ゼロの国家(Oceanic)の軍備の立ち上げ・再建経路の診断。
//! 標準全 AI シナリオの Oceanic の月次の状態と、募兵を止めている判定の実評価を記録する。
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
use strategy_game::war::data::WarRegistry;

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

/// 国家の所有州ごとの建物・鉱床の記録(初期状態と指定日の状態)。
fn describe_states(app: &App, label: &str) {
    use strategy_game::building::data::BuildingType as B;
    let sr = app.world().resource::<StateRegistry>();
    for s in sr.states.iter().filter(|s| s.owner_country_id == OCEANIC) {
        let dep: Vec<String> = s
            .resource_deposits
            .iter()
            .filter(|d| d.discovered)
            .map(|d| format!("{:?}:{:.0}", d.resource_type, d.base_output))
            .collect();
        let t = {
            let wf = (s.employed_workforce as f32 / s.total_workforce().max(1) as f32).clamp(0.0, 1.0);
            wf.min(s.logistics_ratio)
        };
        println!(
            "STATE[{label}] id={} {} pop={} dep=[{}] t={:.2} Mine={} F={} MF={} Farm={}",
            s.id.0,
            s.name,
            s.population,
            dep.join(","),
            t,
            s.building_level(B::Mine),
            s.building_level(B::Factory),
            s.building_level(B::MilitaryFactory),
            s.building_level(B::Farm),
        );
    }
}

/// P30-001-DIAG-001: Oceanic の月次の記録。募兵を止めている判定を、実際に評価した結果で記録する
/// (古いゲートフラグの表示値と、同じ状態で評価し直した結果を区別する)。
#[test]
fn p30_001_oceanic_timeline() {
    use strategy_game::country::country_ai::CountryAiRegistry;
    use strategy_game::military::data::STANDARD_INFANTRY_DEFINITION_ID;
    use strategy_game::military::recruitment_supply_gate::{
        compute_bridge_deficit_units, evaluate_recruitment_supply_gate,
        proposed_division_daily_demand_units, resume_days_for, stop_days_for,
    };
    use strategy_game::military::supply_initialization::country_daily_demand_units;
    use strategy_game::population::growth::{compute_manpower_cap, compute_population_by_owner};

    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p30_001_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    describe_states(&app, "1800/01/01");
    let mut last_states = usize::MAX;
    let mut flag_matches_evaluation = true;
    let mut ever_recruited_after_loss = false;
    let mut lost_at: Option<String> = None;
    while app.world().resource::<GameDate>().year < 1830 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        let owned_now = app
            .world()
            .resource::<StateRegistry>()
            .states
            .iter()
            .filter(|s| s.owner_country_id == OCEANIC)
            .count();
        if owned_now != last_states && last_states != usize::MAX {
            println!("OWNERSHIP_CHANGE {} states {last_states} -> {owned_now}", date.display());
            describe_states(&app, &date.display());
        }
        if owned_now != last_states && last_states != usize::MAX && lost_at.is_none() {
            lost_at = Some(date.display());
        }
        last_states = owned_now;
        let show = date.day == 1 && (date.year == 1800 || date.year == 1801 || date.month == 1);
        if !show {
            continue;
        }
        let cr = app.world().resource::<CountryRegistry>();
        let mil = app.world().resource::<MilitaryRegistry>();
        let wr = app.world().resource::<WarRegistry>();
        let ai = app.world().resource::<CountryAiRegistry>();
        let sr = app.world().resource::<StateRegistry>();
        let c = cr.get(OCEANIC).unwrap();
        let def = mil.definitions.get(&STANDARD_INFANTRY_DEFINITION_ID).unwrap();
        let living = mil
            .divisions
            .values()
            .filter(|d| d.owner == OCEANIC && d.manpower > 0)
            .count();
        let stock_value = c.stockpile.get(ResourceType::MilitarySupplies);
        let stock_units = strategy_game::economy::resources::military_supplies_value_to_units(stock_value)
            .unwrap() as u128;
        let demand = country_daily_demand_units(OCEANIC, &mil.divisions, &mil.definitions).unwrap_or(0);
        let proposed = proposed_division_daily_demand_units(def).unwrap();
        let bridge = compute_bridge_deficit_units(demand + proposed, date.month, date.day, stock_units)
            .unwrap_or(0);
        let at_war = wr.is_country_at_war(OCEANIC);
        let eval_from_open = evaluate_recruitment_supply_gate(c, mil, wr, def, &date, false).ok();
        let eval_from_blocked = evaluate_recruitment_supply_gate(c, mil, wr, def, &date, true).ok();
        let pop = compute_population_by_owner(sr).get(&OCEANIC).copied().unwrap_or(0);
        let cap = compute_manpower_cap(pop);
        let target = strategy_game::military::ai::calculate_ai_target_divisions(cap, def.required_manpower);
        let flag = ai
            .ai_states
            .get(&OCEANIC)
            .map(|s| s.recruitment_supply_blocked);
        if lost_at.is_some() {
            // 古いフラグの表示値と、同じ状態での実評価(開放状態からの評価)が一致する = 古い値ではない。
            flag_matches_evaluation &= flag == eval_from_open;
            ever_recruited_after_loss |= living > 0;
        }
        println!(
            "OCE {} living={living} queue={} target={target} cap={cap} avail_mp={} required_mp={} treasury={:.0} MS={:.1} demand_units={demand} proposed_units={proposed} bridge_deficit_units={bridge} war={at_war} stop_days={} resume_days={} flag={flag:?} eval_open={eval_from_open:?} eval_blocked={eval_from_blocked:?} Fe={:.0} Co={:.0} IG={:.0}",
            date.display(),
            c.recruitment_queue.len(),
            c.available_manpower,
            def.required_manpower,
            c.treasury,
            stock_value,
            stop_days_for(at_war),
            resume_days_for(at_war),
            c.stockpile.get(ResourceType::Iron),
            c.stockpile.get(ResourceType::Coal),
            c.stockpile.get(ResourceType::IndustrialGoods),
        );
    }
    // 構造の記録(現状の挙動の記録であり、望ましい仕様の固定ではない): 州を失った後、募兵を止める判定は
    // 古いフラグ値ではなく実評価の結果で、師団は 0 のまま。
    assert!(lost_at.is_some(), "Oceanic が州を失う");
    assert!(flag_matches_evaluation, "フラグは実評価と一致している(古い値ではない)");
    assert!(!ever_recruited_after_loss);
}

/// Oceanic の首都州(州 8、MilitaryFactory のある唯一の州)の労働力・物流。稼働率の上限 t =
/// min(雇用率, 物流比) の内訳を記録する。
#[test]
fn p30_001_oceanic_capital_workforce_and_logistics() {
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p30_001_wf_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    while app.world().resource::<GameDate>().year < 1801 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        if date.day != 1 || !matches!(date.month, 2 | 4 | 6 | 9 | 12) {
            continue;
        }
        let sr = app.world().resource::<StateRegistry>();
        let s = sr.get(strategy_game::common::StateId(8)).unwrap();
        println!(
            "WF {} pop={} total_workforce={} employed={} employment_ratio={:.3} logistics={:.3} buildings={:?}",
            date.display(),
            s.population,
            s.total_workforce(),
            s.employed_workforce,
            s.employed_workforce as f64 / s.total_workforce().max(1) as f64,
            s.logistics_ratio,
            s.buildings
        );
    }
}

/// 補助実験(状態を操作。通常シナリオの再現ではない): 州 8 を失った後(1801/08/01)に、Oceanic の
/// 州 9 へ MilitaryFactory lv1 の建設注文を 1 件だけ入れ、既存の機能(生産・補給ゲート・募兵 AI・
/// 日次の建設)だけで、備蓄の形成と募兵がどこまで到達するかを記録する。物資の無償生成・ゲートの
/// 免除・資源の追加はしない。
#[test]
fn p30_001_counterfactual_one_military_factory_after_the_loss() {
    use strategy_game::building::data::BuildingType as B;
    use strategy_game::country::country_ai::CountryAiRegistry;
    use strategy_game::building::construction::{ConstructionQueueItem, ConstructionStatus};
    use strategy_game::military::data::STANDARD_INFANTRY_DEFINITION_ID;
    use strategy_game::military::recruitment_supply_gate::evaluate_recruitment_supply_gate;

    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p30_001_cf_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    while app.world().resource::<GameDate>().display().as_str() < "1801/08/01" {
        advance_one_day(&mut app);
    }
    {
        let def = app
            .world()
            .resource::<strategy_game::building::data::BuildingRegistry>()
            .get(B::MilitaryFactory)
            .unwrap()
            .clone();
        let mut cr = app.world_mut().resource_mut::<CountryRegistry>();
        let c = cr.get_mut(OCEANIC).unwrap();
        c.treasury -= def.construction_cost;
        c.construction_queue.push(ConstructionQueueItem {
            state_id: strategy_game::common::StateId(9),
            building_type: B::MilitaryFactory,
            target_level: 1,
            progress: 0.0,
            required_progress: def.required_progress,
            paid_cost: def.construction_cost,
            status: ConstructionStatus::InQueue,
        });
    }
    let mut recruited = false;
    let mut max_ms = 0.0f64;
    let mut prev_living = 0usize;
    while app.world().resource::<GameDate>().year < 1806 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        let cr = app.world().resource::<CountryRegistry>();
        let mil = app.world().resource::<MilitaryRegistry>();
        let living = mil
            .divisions
            .values()
            .filter(|d| d.owner == OCEANIC && d.manpower > 0)
            .count();
        max_ms = max_ms.max(c_ms(&app));
        recruited |= living > prev_living;
        if living != prev_living {
            println!("CF_DIVISIONS {} living {prev_living} -> {living}", date.display());
            prev_living = living;
        }
        if date.day != 1 || !(date.month % 3 == 1 || date.year == 1801) {
            continue;
        }
        let sr = app.world().resource::<StateRegistry>();
        let s9 = sr.get(strategy_game::common::StateId(9)).unwrap();
        let c = cr.get(OCEANIC).unwrap();
        let wr = app.world().resource::<WarRegistry>();
        let def = mil.definitions.get(&STANDARD_INFANTRY_DEFINITION_ID).unwrap();
        let eval = evaluate_recruitment_supply_gate(c, mil, wr, def, &date, false).ok();
        let _ = app.world().resource::<CountryAiRegistry>();
        println!(
            "CF {} MF9={} op9={:.2} employment9={:.3} MS={:.1} living={living} queue={} Fe={:.0} Co={:.0} IG={:.0} treasury={:.0} gate_blocked_if_open={eval:?}",
            date.display(),
            s9.building_level(B::MilitaryFactory),
            s9.building_operation_ratios.get(&B::MilitaryFactory).copied().unwrap_or(0.0),
            s9.employed_workforce as f64 / s9.total_workforce().max(1) as f64,
            c.stockpile.get(ResourceType::MilitarySupplies),
            c.recruitment_queue.len(),
            c.stockpile.get(ResourceType::Iron),
            c.stockpile.get(ResourceType::Coal),
            c.stockpile.get(ResourceType::IndustrialGoods),
            c.treasury,
        );
    }
    // 既存の機構の到達可能性の記録(物資の無償生成・ゲートの免除なし): 建設注文 1 件で MilitaryFactory が
    // 稼働し、備蓄が 1 個分の橋渡し量(約 31)を超えて補給ゲートが開き、募兵 AI が 1 個を募兵する。
    // 一方、Iron の供給源がなく備蓄が尽き、生産は止まって備蓄は 0 へ戻る(持続できない)。
    assert!(recruited, "既存の機構で 1 個の募兵まで到達する");
    assert!(max_ms > 31.0, "備蓄が橋渡し量を超える(max {max_ms})");
    assert_eq!(c_ms(&app), 0.0, "Iron が尽きると備蓄も 0 へ戻る");
}

fn c_ms(app: &App) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(OCEANIC)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitarySupplies)
}

/// 初期データで、MilitaryFactory・Iron/Coal 鉱床を持たない国があるか(立ち上げの区分の該当確認)。
#[test]
fn p30_001_initial_military_factories_and_iron_by_country() {
    use strategy_game::building::data::BuildingType as B;
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p30_001_init_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let app = setup_app(&dir);
    let sr = app.world().resource::<StateRegistry>();
    for c in 0..6usize {
        let id = strategy_game::common::CountryId(c);
        let owned: Vec<_> = sr.states.iter().filter(|s| s.owner_country_id == id).collect();
        let mf: u32 = owned.iter().map(|s| s.building_level(B::MilitaryFactory)).sum();
        let iron: usize = owned
            .iter()
            .filter(|s| {
                s.resource_deposits
                    .iter()
                    .any(|d| d.discovered && d.resource_type == ResourceType::Iron)
            })
            .count();
        let coal: usize = owned
            .iter()
            .filter(|s| {
                s.resource_deposits
                    .iter()
                    .any(|d| d.discovered && d.resource_type == ResourceType::Coal)
            })
            .count();
        let mf_states: Vec<usize> = owned
            .iter()
            .filter(|s| s.building_level(B::MilitaryFactory) > 0)
            .map(|s| s.id.0)
            .collect();
        let iron_states: Vec<usize> = owned
            .iter()
            .filter(|s| s.resource_deposits.iter().any(|d| d.discovered && d.resource_type == ResourceType::Iron))
            .map(|s| s.id.0)
            .collect();
        println!(
            "INIT c{c} states={} MF_levels={mf} MF_states={mf_states:?} iron_states={iron_states:?} (iron={iron}, coal={coal})",
            owned.len()
        );
    }
}
