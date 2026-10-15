//! P29-006-DIAG-001: Elfin の軍需生産(MilitaryFactory 稼働率)の月次の入力財収支の診断。
//! 標準全 AI シナリオを 600 か月進め、月初の備蓄・需要・充足率・最終稼働率・産出を記録する。
use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::BTreeMap;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::BuildingRegistry;
use strategy_game::common::CountryId;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
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


/// 診断対象の国(既定 Elfin = 1。環境変数 `P29_DIAG_COUNTRY` で他国へ変更できる)。
fn elfin_id() -> CountryId {
    CountryId(
        std::env::var("P29_DIAG_COUNTRY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
    )
}

#[derive(Clone, Debug, Default)]
struct Row {
    date: String,
    // 月初(同月の月次生産の直前)の備蓄
    iron: f64,
    coal: f64,
    ig: f64,
    // 需要(Step 1: base × level × min(wf, log))
    iron_need_f: f64,
    iron_need_mf: f64,
    coal_need_f: f64,
    ig_need_mf: f64,
    t_min: f32,
    sat_iron: f64,
    sat_coal: f64,
    sat_ig: f64,
    d4: bool,
    // 最終稼働率(同月の記録)
    op_mine: f32,
    op_f: f32,
    op_mf: f32,
    lv_mine: u32,
    lv_f: u32,
    lv_mf: u32,
    // 産出・消費(記録された稼働率からの再構成)
    iron_out: f64,
    ig_out: f64,
    iron_used: f64,
    coal_used: f64,
    ig_used: f64,
    ms_out: f64,
    // 月末(同フレーム後)
    iron_end: f64,
    ig_end: f64,
    owned_changed: bool,
    bind: &'static str,
}

#[test]
fn p29_006_elfin_production_cycle_trace() {
    use strategy_game::building::data::BuildingType as B;
    let dir = std::env::temp_dir().join(format!(
        "strategy_game_p29_006_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = setup_app(&dir);
    let mut rows: Vec<Row> = Vec::new();
    let mut owned_prev: Vec<(usize, u32)> = Vec::new();
    while app.world().resource::<GameDate>().year < 1850 {
        // 前日末の備蓄(Iron/Coal/IG は日次で消費されないため、月次生産の直前の値)。
        let pre = {
            let cr = app.world().resource::<CountryRegistry>();
            let st = &cr.get(elfin_id()).unwrap().stockpile;
            (
                st.get(ResourceType::Iron),
                st.get(ResourceType::Coal),
                st.get(ResourceType::IndustrialGoods),
            )
        };
        advance_one_day(&mut app);
        let date = app.world().resource::<GameDate>().clone();
        if date.day != 1 {
            continue;
        }
        let cr = app.world().resource::<CountryRegistry>();
        let sr = app.world().resource::<StateRegistry>();
        let br = app.world().resource::<BuildingRegistry>();
        let st = &cr.get(elfin_id()).unwrap().stockpile;
        let owned: Vec<(usize, u32)> = sr
            .states
            .iter()
            .filter(|s| s.owner_country_id == elfin_id())
            .map(|s| (s.id.0, s.building_level(B::MilitaryFactory)))
            .collect();
        let mut r = Row {
            date: date.display(),
            iron: pre.0,
            coal: pre.1,
            ig: pre.2,
            t_min: 1.0,
            iron_end: st.get(ResourceType::Iron),
            ig_end: st.get(ResourceType::IndustrialGoods),
            owned_changed: owned != owned_prev,
            ..Default::default()
        };
        owned_prev = owned;
        let def = |b: B| br.get(b).unwrap();
        for s in sr.states.iter().filter(|s| s.owner_country_id == elfin_id()) {
            let wf = (s.employed_workforce as f32 / s.total_workforce().max(1) as f32).clamp(0.0, 1.0);
            let t = wf.min(s.logistics_ratio);
            for b in [B::Mine, B::Factory, B::MilitaryFactory] {
                let lv = s.building_level(b);
                if lv == 0 {
                    continue;
                }
                let op = s.building_operation_ratios.get(&b).copied().unwrap_or(0.0);
                let (lvf, opf) = (lv as f64, op as f64);
                match b {
                    B::Mine => {
                        r.lv_mine += lv;
                        r.op_mine = op;
                        for d in &s.resource_deposits {
                            if d.discovered && d.resource_type == ResourceType::Iron {
                                r.iron_out += d.base_output * lvf * opf;
                            }
                        }
                    }
                    B::Factory => {
                        r.lv_f += lv;
                        r.op_f = op;
                        r.t_min = r.t_min.min(t);
                        let d = def(b);
                        r.iron_need_f += d.input_resources[&ResourceType::Iron] * lvf * t as f64;
                        r.coal_need_f += d.input_resources[&ResourceType::Coal] * lvf * t as f64;
                        r.iron_used += d.input_resources[&ResourceType::Iron] * lvf * opf;
                        r.coal_used += d.input_resources[&ResourceType::Coal] * lvf * opf;
                        r.ig_out += d.output_resources[&ResourceType::IndustrialGoods] * lvf * opf;
                    }
                    _ => {
                        r.lv_mf += lv;
                        r.op_mf = op;
                        r.t_min = r.t_min.min(t);
                        let d = def(b);
                        r.iron_need_mf += d.input_resources[&ResourceType::Iron] * lvf * t as f64;
                        r.ig_need_mf += d.input_resources[&ResourceType::IndustrialGoods] * lvf * t as f64;
                        r.iron_used += d.input_resources[&ResourceType::Iron] * lvf * opf;
                        r.ig_used += d.input_resources[&ResourceType::IndustrialGoods] * lvf * opf;
                        r.ms_out += d.output_resources[&ResourceType::MilitarySupplies] * lvf * opf;
                    }
                }
            }
        }
        let need_iron = r.iron_need_f + r.iron_need_mf;
        r.sat_iron = if need_iron > 0.0 { (r.iron / need_iron).min(1.0) } else { 1.0 };
        r.sat_coal = if r.coal_need_f > 0.0 { (r.coal / r.coal_need_f).min(1.0) } else { 1.0 };
        r.sat_ig = if r.ig_need_mf > 0.0 { (r.ig / r.ig_need_mf).min(1.0) } else { 1.0 };
        r.d4 = r.iron_need_mf > 0.0 && need_iron > r.iron && r.ig >= 3.0 * r.ig_need_mf;
        // MilitaryFactory の稼働率を律速した要因(D4 成立時は Iron の優先配分)。
        let t = r.t_min as f64;
        r.bind = if r.op_mf <= 0.0 {
            "zero"
        } else if r.d4 {
            "d4-priority"
        } else if (r.op_mf as f64) >= t - 1e-4 && r.sat_iron.min(r.sat_ig) >= t - 1e-4 {
            "workforce/logistics"
        } else if r.sat_iron <= r.sat_ig {
            "iron-share"
        } else {
            "ig-share"
        };
        if r.date.as_str() >= "1800/02/01" {
            rows.push(r);
        }
    }

    // ── 出力: 月次表(全期間)と統計 ─────────────────────────────────────
    for r in &rows {
        println!(
            "TRACE {} pre[Fe={:.0} Co={:.0} IG={:.0}] need[FeF={:.0} FeMF={:.0} CoF={:.0} IGMF={:.0}] sat[Fe={:.2} Co={:.2} IG={:.2}] t={:.2} d4={} lv[M{} F{} MF{}] op[M={:.2} F={:.2} MF={:.2}] out[Fe={:.0} IG={:.0} MS={:.0}] used[Fe={:.0} Co={:.0} IG={:.0}] end[Fe={:.0} IG={:.0}] terr_changed={} bind={}",
            r.date, r.iron, r.coal, r.ig, r.iron_need_f, r.iron_need_mf, r.coal_need_f, r.ig_need_mf,
            r.sat_iron, r.sat_coal, r.sat_ig, r.t_min, r.d4, r.lv_mine, r.lv_f, r.lv_mf,
            r.op_mine, r.op_f, r.op_mf, r.iron_out, r.ig_out, r.ms_out, r.iron_used, r.coal_used, r.ig_used,
            r.iron_end, r.ig_end, r.owned_changed, r.bind
        );
    }
    let windows: [(&str, &str, &str); 6] = [
        ("1800-1810", "1800/02/01", "1810/01/01"),
        ("1810-1820", "1810/01/01", "1820/01/01"),
        ("1820-1830", "1820/01/01", "1830/01/01"),
        ("1830-1840", "1830/01/01", "1840/01/01"),
        ("1840-1850", "1840/01/01", "1850/01/01"),
        ("all", "1800/02/01", "1850/01/01"),
    ];
    for (name, from, to) in windows {
        let xs: Vec<f64> = rows
            .iter()
            .filter(|r| r.date.as_str() >= from && r.date.as_str() < to)
            .map(|r| r.ms_out)
            .collect();
        if xs.len() < 24 {
            continue;
        }
        let n = xs.len() as f64;
        let mean = xs.iter().sum::<f64>() / n;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        let min = xs.iter().cloned().fold(f64::MAX, f64::min);
        let max = xs.iter().cloned().fold(f64::MIN, f64::max);
        let mut ac = String::new();
        for lag in 1..=12usize {
            let c = xs
                .iter()
                .zip(xs.iter().skip(lag))
                .map(|(a, b)| (a - mean) * (b - mean))
                .sum::<f64>()
                / (n * var.max(1e-9));
            ac.push_str(&format!(" {lag}:{c:.2}"));
        }
        let low = xs.iter().filter(|&&x| x < 0.75 * max).count();
        println!(
            "STAT {name} months={} mean={mean:.1} min={min:.1} max={max:.1} cv={:.2} low(<75% max)={low} autocorr{ac}",
            xs.len(),
            var.sqrt() / mean.max(1e-9)
        );
    }
    let mut binds: BTreeMap<&str, u32> = BTreeMap::new();
    let mut d4_months = 0;
    for r in rows.iter().filter(|r| !r.owned_changed) {
        *binds.entry(r.bind).or_insert(0) += 1;
        if r.d4 {
            d4_months += 1;
        }
    }
    println!("BIND {binds:?} d4_months={d4_months} total={}", rows.len());
    let mut max_resid = 0.0f64;
    for r in rows.iter().filter(|r| !r.owned_changed) {
        let iron = r.iron_end - (r.iron + r.iron_out - r.iron_used);
        let ig = r.ig_end - (r.ig + r.ig_out - r.ig_used);
        max_resid = max_resid.max(iron.abs()).max(ig.abs());
    }
    println!("CONSERVATION max_resid(Iron,IG; terr unchanged months)={max_resid:.6}");
    // 保存則: 領土変更のない月は、Iron/IG の月末備蓄が 月初 + 産出 - 消費 と一致する
    // (二重配分・二重消費・不正な丸めがない)。
    assert!(max_resid < 1e-6, "conservation residual {max_resid}");
    assert!(rows.iter().all(|r| r.iron_end >= 0.0 && r.ig_end >= 0.0), "no negative stock");
    // (P29-007 で D4 を IG 備蓄に応じた線形補間へ変更したため、「D4 が成立する月」の数は
    // 配分方式に依存する診断値であり、検査しない。基準 HEAD では 386 か月だった。)
    let _ = d4_months;
}
