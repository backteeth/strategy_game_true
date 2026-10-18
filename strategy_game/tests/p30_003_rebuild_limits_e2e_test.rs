//! P30-003: 軍需工場の再建の限界ケース(実 App fixture。状態を操作した補助実験で、通常シナリオの
//! 再現ではない): 着工までの待ちの分類と、低雇用の州での再建後の能力不足。
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
    }
}


/// 再建の条件のうち建設キュー以外を満たしているか。国家の複製でキューを空にして、実際の判断経路
/// (`military_rebuild_context` と `process_construction_ai_with_contexts`)を評価する。
/// 実際の国家・州は変更しない。`with_actual_queue` なら実際のキューのまま評価する。
fn rebuild_would_fire(app: &App, with_actual_queue: bool) -> bool {
    use strategy_game::building::data::BuildingType as B;
    use strategy_game::country::country_ai::{
        CountryAiRegistry, military_rebuild_context, process_construction_ai_with_contexts,
    };
    let cr = app.world().resource::<CountryRegistry>();
    let sr = app.world().resource::<StateRegistry>();
    let mil = app.world().resource::<MilitaryRegistry>();
    let br = app.world().resource::<strategy_game::building::data::BuildingRegistry>();
    let ai = app.world().resource::<CountryAiRegistry>();
    let date = app.world().resource::<GameDate>().display();
    let mut c = cr.get(OCEANIC).unwrap().clone();
    if !with_actual_queue {
        c.construction_queue.clear();
    }
    let empty = Default::default();
    let ctx = military_rebuild_context(
        &c,
        sr,
        mil,
        ai.supply_production_history.get(&OCEANIC).unwrap_or(&empty),
        &date,
    );
    let mut reg = CountryRegistry::default();
    reg.countries.push(c);
    let mut scratch = CountryAiRegistry::default();
    process_construction_ai_with_contexts(
        OCEANIC,
        &mut reg,
        sr,
        br,
        scratch.get_or_create_mut(OCEANIC),
        None,
        None,
        false,
        ctx,
    );
    reg.get(OCEANIC)
        .unwrap()
        .construction_queue
        .iter()
        .any(|q| q.building_type == B::MilitaryFactory)
}

/// 着工までの待ちの時系列。「キュー以外の再建条件を満たした日」「キューを占めていた注文と進捗」
/// 「空き枠ができた日」「着工日」「その間に新規発注された注文」を記録し、有限の既存注文を待った結果か、
/// 低優先の注文が繰り返し先行する飢餓かを区別する。
#[test]
fn rebuild_wait_timeline_is_classified() {
    let dir = temp_dir("wait");
    let mut app = setup_app(&dir);
    while app.world().resource::<GameDate>().display().as_str() < "1801/08/01" {
        advance_one_day(&mut app);
    }
    make_supplied_but_without_military_factory(&mut app);
    let manipulated = app.world().resource::<GameDate>().display();
    let mut first_conditions_met: Option<String> = None;
    let mut first_free_slot_after_met: Option<String> = None;
    let mut queued_on: Option<String> = None;
    let mut seen: std::collections::BTreeSet<(String, usize, u32)> = Default::default();
    let mut new_orders_after_met: Vec<String> = Vec::new();
    let mut prev_sig = String::new();
    while queued_on.is_none() && app.world().resource::<GameDate>().year < 1803 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().display();
        let met = rebuild_would_fire(&app, false);
        let cr = app.world().resource::<CountryRegistry>();
        let c = cr.get(OCEANIC).unwrap();
        let queue_len = c.construction_queue.len();
        if met && first_conditions_met.is_none() {
            first_conditions_met = Some(date.clone());
        }
        if met && queue_len < 2 && first_free_slot_after_met.is_none() {
            first_free_slot_after_met = Some(date.clone());
        }
        for q in &c.construction_queue {
            let key = (format!("{:?}", q.building_type), q.state_id.0, q.target_level);
            if seen.insert(key.clone()) && first_conditions_met.is_some() && key.0 != "MilitaryFactory" {
                new_orders_after_met.push(format!("{date} {}@{}", key.0, key.1));
            }
        }
        let sig = format!(
            "met={met} q=[{}]",
            c.construction_queue
                .iter()
                .map(|q| format!(
                    "{:?}@{} {:.0}%",
                    q.building_type,
                    q.state_id.0,
                    100.0 * q.progress / q.required_progress
                ))
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !snap(&app).mf_queued.is_empty() {
            queued_on = Some(date.clone());
        }
        let day = app.world().resource::<GameDate>().day;
        if sig != prev_sig && (day.is_multiple_of(10) || queued_on.is_some() || met) {
            println!("WAIT {date} {sig}");
        }
        prev_sig = sig;
    }
    println!(
        "WAIT_SUMMARY manipulated={manipulated} conditions_met_first={first_conditions_met:?} free_slot_first_after_met={first_free_slot_after_met:?} mf_queued={queued_on:?}"
    );
    println!("WAIT_NEW_ORDERS_AFTER_CONDITIONS_MET {new_orders_after_met:?}");
    let met = first_conditions_met.expect("キュー以外の条件を満たす日がある");
    let free = first_free_slot_after_met.expect("条件が揃った後に空き枠がある");
    let queued = queued_on.expect("着工される");
    // 待ちの主因はキューの飢餓ではなく供給条件(Mine の稼働が必要量に届く日)と、その後の月次判断日まで。
    assert!(met <= free && free <= queued);
    assert!(new_orders_after_met.is_empty(), "条件が揃った後に低優先の注文が先行していない");
}

/// 低雇用の州での再建後の能力不足。工場完成・初回募兵・持続的な軍備復帰を別々に記録する
/// (Oceanic の全州の人口を 200 万にして雇用率を下げる状態操作)。
#[test]
fn low_employment_rebuild_completes_recruits_but_may_not_sustain() {
    use strategy_game::building::data::BuildingType as B;
    use strategy_game::country::country_ai::military_rebuild_gain_units;
    let dir = temp_dir("lowemp");
    let mut app = setup_app(&dir);
    while app.world().resource::<GameDate>().display().as_str() < "1801/08/01" {
        advance_one_day(&mut app);
    }
    make_supplied_but_without_military_factory(&mut app);
    {
        let mut sr = app.world_mut().resource_mut::<StateRegistry>();
        for s in sr.states.iter_mut().filter(|s| s.owner_country_id == OCEANIC) {
            s.population = 2_000_000;
        }
    }
    let mut predicted: Option<(usize, u64)> = None;
    let mut completed_on: Option<String> = None;
    let mut first_recruit_on: Option<String> = None;
    let mut recruits: Vec<String> = Vec::new();
    let mut prev_living = 0usize;
    let mut ms_zero_months = 0u32;
    let mut input_zero_months = 0u32;
    let mut mf_lost_to: Option<strategy_game::common::CountryId> = None;
    while app.world().resource::<GameDate>().year < 1808 {
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        let s = snap(&app);
        if predicted.is_none() && !s.mf_queued.is_empty() {
            let (sid, _) = s.mf_queued[0];
            let sr = app.world().resource::<StateRegistry>();
            let br = app.world().resource::<strategy_game::building::data::BuildingRegistry>();
            let g = military_rebuild_gain_units(
                sr.get(strategy_game::common::StateId(sid)).unwrap(),
                br,
            )
            .unwrap_or(0);
            predicted = Some((sid, g));
            println!("LOWEMP_QUEUED {} state={sid} predicted_units={g} ({:.1} MS/month)", date.display(), g as f64 / 1e6);
        }
        if completed_on.is_none() && s.mf_levels > 0 {
            completed_on = Some(date.display());
        }
        if s.living > prev_living {
            recruits.push(format!("{} -> {} (MS {:.1})", date.display(), s.living, s.ms));
            first_recruit_on.get_or_insert(date.display());
        }
        prev_living = s.living;
        if date.day == 1 && first_recruit_on.is_some() {
            // P31-002: M1 で戦争の経過が変わり、Oceanic は 1807/05/22 の降伏で再建した MilitaryFactory の州を
            // 失う。補給が尽きたかどうかは、工場を保持している間だけ数える(工場の喪失は再建の限界ではなく
            // 戦争の結果)。
            if s.ms <= 0.0 && s.mf_levels > 0 {
                ms_zero_months += 1;
            }
            // 工場を保持している間に、原料(Iron)や IG の備蓄が尽きていないこと(原料枯渇・供給不足を
            // 工場の喪失で隠さない)。
            if s.mf_levels > 0 && (s.fe <= 0.0 || s.ig <= 0.0) {
                input_zero_months += 1;
            }
            // 工場を失った場合、その州の所有者が Oceanic でなくなっている(戦争で州を失った)こと。
            if completed_on.is_some() && s.mf_levels == 0 && mf_lost_to.is_none() {
                let sr = app.world().resource::<StateRegistry>();
                let sid = strategy_game::common::StateId(predicted.unwrap().0);
                mf_lost_to = Some(sr.get(sid).unwrap().owner_country_id);
            }
            if date.month == 1 || date.month == 7 {
                let sr = app.world().resource::<StateRegistry>();
                let op = sr
                    .states
                    .iter()
                    .filter(|st| st.owner_country_id == OCEANIC)
                    .filter_map(|st| st.building_operation_ratios.get(&B::MilitaryFactory).copied())
                    .fold(0.0f32, f32::max);
                println!(
                    "LOWEMP_TRACK {} MF_op={op:.2} MS={:.1} living={} Fe={:.0} IG={:.0}",
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
        "LOWEMP_SUMMARY mf_lost_to={mf_lost_to:?} input_zero_months={input_zero_months} predicted={predicted:?} completed={completed_on:?} first_recruit={first_recruit_on:?} recruits={recruits:?} end_living={} end_ms={:.1} ms_zero_months_after_first_recruit={ms_zero_months}",
        end.living, end.ms
    );
    assert!(predicted.is_some(), "再建が着工される");
    assert!(completed_on.is_some(), "工場は完成する");
    assert!(first_recruit_on.is_some(), "低雇用でも初回の募兵は起きる");
    assert_eq!(ms_zero_months, 0, "補給が尽きて止まってはいない");
    assert_eq!(input_zero_months, 0, "工場を保持している間に原料・IG が尽きない");
    if let Some(owner) = mf_lost_to {
        assert_ne!(owner, OCEANIC, "工場の喪失は、戦争で州を失った結果である");
    }
}
