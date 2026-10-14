//! P28-010-IMPL-001: 実データ・全6か国の実Appで、入力の原子的会計(資源保存則・無償産出の消滅・
//! 決定性・入力失敗がなかった国の非回帰)を確認する。

#![allow(unused)]
use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::building::data::BuildingType;
use strategy_game::common::{CountryId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::finance::LOGISTICS_WELFARE_BONUS_RATIO;
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

const DWARF: CountryId = CountryId(2);
const ALL_SIX: [CountryId; 6] = [CountryId(0), CountryId(1), CountryId(2), CountryId(3), CountryId(4), CountryId(5)];
/// 自然運用で最初にRailwayが着工される州(実測、`verification_logs/phase-28/p28-005/`)。
const FIRST_RAILWAY_STATE: StateId = StateId(6);

// ─── App-level fixtures ─────────────────────────────────────────────────────

struct TempTestDir(std::path::PathBuf);
impl TempTestDir {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        Self(std::env::temp_dir().join(format!(
            "strategy_game_p28_010_iron_{label}_{}_{nanos}_{n}",
            std::process::id()
        )))
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
    // デフォルトstateのまま最初のupdateを呼ばないと、Playing挿入が遷移として認識されない。
    app.update();
    app.insert_resource(PlayerCountry(player));
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
    let mut s: SystemState<(MessageWriter<DayChangedMessage>, MessageWriter<MonthChangedMessage>)> =
        SystemState::new(app.world_mut());
    {
        let (mut days, mut months) = s.get_mut(app.world_mut()).unwrap();
        days.write(DayChangedMessage { year: next.year, month: next.month, day: next.day });
        if next.month != old_month {
            months.write(MonthChangedMessage { year: next.year, month: next.month });
        }
    }
    s.apply(app.world_mut());
    app.update();
}

fn date(app: &App) -> String {
    app.world().resource::<GameDate>().display()
}

fn advance_until(app: &mut App, until: &str) {
    while date(app).as_str() < until {
        advance_one_day(app);
    }
}

use strategy_game::building::data::BuildingRegistry;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::war::data::{WarRegistry, WarStatus};

const TRACKED: [ResourceType; 4] = [
    ResourceType::Iron,
    ResourceType::Coal,
    ResourceType::IndustrialGoods,
    ResourceType::MagicCrystal,
];

struct Fingerprint {
    rows: Vec<String>,
    max_resid: f64,
    min_stock: f64,
    frozen_iron_months: u32,
    arcadia_ms_free_suspects: u32,
}

fn stocks(app: &App, c: CountryId) -> [f64; 4] {
    let cr = app.world().resource::<CountryRegistry>();
    let st = &cr.get(c).unwrap().stockpile;
    [
        st.get(TRACKED[0]),
        st.get(TRACKED[1]),
        st.get(TRACKED[2]),
        st.get(TRACKED[3]),
    ]
}

/// 月次の産出(Mine鉱床+各建物の出力)と引落し(入力)を、記録された稼働率から再構成する。
fn flows(app: &App, c: CountryId) -> ([f64; 4], [f64; 4]) {
    let br = app.world().resource::<BuildingRegistry>();
    let sr = app.world().resource::<StateRegistry>();
    let mut out = [0.0; 4];
    let mut used = [0.0; 4];
    for st in sr.states.iter().filter(|s| s.owner_country_id == c) {
        for b in BuildingType::ALL {
            let l = st.building_level(b) as f64;
            if l == 0.0 {
                continue;
            }
            let op = st.building_operation_ratios.get(&b).copied().unwrap_or(0.0) as f64;
            let Some(def) = br.get(b) else { continue };
            for (i, r) in TRACKED.iter().enumerate() {
                if let Some(&base) = def.input_resources.get(r) {
                    used[i] += base * l * op;
                }
                if b == BuildingType::Mine {
                    for d in &st.resource_deposits {
                        if d.discovered && d.resource_type == *r && *r != ResourceType::MagicCrystal {
                            out[i] += d.base_output * l * op;
                        }
                    }
                } else if let Some(&base) = def.output_resources.get(r) {
                    out[i] += base * l * op;
                }
            }
        }
    }
    (out, used)
}

/// 国家が所有する州の集合(月次処理の同一フレームでの講和・占領による所有変更の検出用)。
fn owned_state_ids(app: &App, c: CountryId) -> Vec<usize> {
    app.world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .filter(|s| s.owner_country_id == c)
        .map(|s| s.id.0)
        .collect()
}

fn simulate(months: u32, report_at: &[u32], print: bool) -> Fingerprint {
    let dir = TempTestDir::new("impl001");
    let mut app = setup_app(None, dir.save_file_config());
    let end = format!("{:04}/{:02}/01", 1800 + (months / 12) as i32, 1 + months % 12);
    let mut pre = ALL_SIX.map(|c| stocks(&app, c));
    let mut shortage = [0u32; 6];
    let mut fp = Fingerprint {
        rows: Vec::new(),
        max_resid: 0.0,
        min_stock: f64::MAX,
        frozen_iron_months: 0,
        arcadia_ms_free_suspects: 0,
    };
    let mut month_i = 0u32;
    let mut did90 = false;
    while date(&app).as_str() < end.as_str() {
        if app.world().resource::<GameDate>().add_days(1).day == 1 {
            pre = ALL_SIX.map(|c| stocks(&app, c));
        }
        let owned_before = ALL_SIX.map(|c| owned_state_ids(&app, c));
        advance_one_day(&mut app);
        let owned_after = ALL_SIX.map(|c| owned_state_ids(&app, c));
        {
            let mr = app.world().resource::<MilitaryRegistry>();
            for d in mr.divisions.values() {
                if mr.supply_fulfillment_of(d.id) < 0.34 {
                    shortage[d.owner.0] += 1;
                }
            }
        }
        let snap = !did90 && date(&app).as_str() >= "1800/04/01";
        if snap {
            did90 = true;
        }
        let is_month = app.world().resource::<GameDate>().day == 1;
        if is_month {
            month_i += 1;
            for c in ALL_SIX {
                let post = stocks(&app, c);
                // 月次生産(Economy)の後、同じフレームの講和・占領で州の所有が変わった国は、
                // 再構成(現在の所有・稼働率)が生産時点とずれるため、保存則の照合から外す
                // (P29-005: 世界の分岐で、月初の同一フレームに州が移る月が現れた)。
                let territory_changed = owned_before[c.0] != owned_after[c.0];
                let (out, used) = flows(&app, c);
                for i in 0..4 {
                    let resid = post[i] - (pre[c.0][i] + out[i] - used[i]);
                    if !territory_changed {
                        fp.max_resid = fp.max_resid.max(resid.abs());
                    }

                    fp.min_stock = fp.min_stock.min(post[i]);
                }
                // 入力を使う建物が稼働しているのに、Ironが据え置かれ(産出もなく)続ける=無償産出。
                let ops = used[0];
                if ops > 1e-9 && (post[0] - pre[c.0][0]).abs() < 1e-12 && out[0] == 0.0 {
                    fp.frozen_iron_months += 1;
                }
            }
        }
        if snap || (is_month && report_at.contains(&month_i)) {
            let label = if snap { "90d".to_string() } else { format!("m{month_i}") };
            let cr = app.world().resource::<CountryRegistry>();
            let sr = app.world().resource::<StateRegistry>();
            let mr = app.world().resource::<MilitaryRegistry>();
            let wr = app.world().resource::<WarRegistry>();
            let active = wr.wars.values().filter(|w| w.status == WarStatus::Active).count();
            let battles = app.world().resource::<BattleRegistry>().next_id();
            let mut line = format!("{label} wars={} active={} battles={}", wr.wars.len(), active, battles);
            for c in ALL_SIX {
                let divs = mr.divisions.values().filter(|d| d.owner == c).count();
                let own: Vec<_> = sr.states.iter().filter(|s| s.owner_country_id == c).collect();
                let ls = own.iter().map(|s| s.living_standard as f64).sum::<f64>() / own.len().max(1) as f64;
                let ct = cr.get(c).unwrap();
                let lv = |b| own.iter().map(|s| s.building_level(b)).sum::<u32>();
                let s = stocks(&app, c);
                line.push_str(&format!(
                    " | c{} divs={} shortD={} Iron={:.0} Coal={:.0} IG={:.0} MC={:.0} tr={:.0} tax={:.2} LS={:.1} M{}F{}MF{}",
                    c.0, divs, shortage[c.0], s[0], s[1], s[2], s[3], ct.treasury, ct.tax_rate, ls,
                    lv(BuildingType::Mine), lv(BuildingType::Factory), lv(BuildingType::MilitaryFactory)
                ));
            }
            if print {
                eprintln!("ROW {line}");
            }
            fp.rows.push(line);
        }
    }
    fp
}

#[test]
fn real_app_240_months_keeps_the_resource_ledger_closed_for_all_six_countries() {
    let fp = simulate(240, &[36, 120, 240], true);
    assert!(
        fp.max_resid < 1e-3,
        "Iron/Coal/IndustrialGoods/MagicCrystal conservation residual must be ~0, got {}",
        fp.max_resid
    );
    assert!(fp.min_stock >= 0.0, "no negative stock");
    assert_eq!(fp.frozen_iron_months, 0, "Iron must not stay frozen while consumers run");
}

#[test]
fn real_app_is_deterministic_across_two_runs() {
    let a = simulate(36, &[36], false);
    let b = simulate(36, &[36], false);
    assert_eq!(a.rows, b.rows);
}

/// 入力引落しの失敗が無かったDwarf・Passguardは、IMPL-001(FIX1)の時点では修正前の正規値と
/// 一致した。P28-010-IMPL-002(D4: IG備蓄で保護したMilitaryFactory優先配分)以降は、
/// - Passguard: 120か月まで同じ値(Iron不足でもIG備蓄がゲートを満たさない)。
/// - Dwarf: Iron枯渇(m100)でD4が成立し、Division 11→16、補給不足 5857→4649、
///   IG備蓄 5074→2838、国庫 15176→15340 へ変わる(最初の分岐は m099〜m100)。
#[test]
fn passguard_keeps_pre_change_values_and_dwarf_follows_d4_at_month_120() {
    let fp = simulate(120, &[120], false);
    let row = fp.rows.iter().find(|r| r.starts_with("m120")).unwrap();
    // P29-005(複数月の実産出に基づく能力上限)で、m120 の Dwarf を shortD 3184→3161・Iron 2323→2319・
    // Coal 4176→4156・IG 8952→9093・国庫 15090→15243 へ更新した(世界の分岐。Passguard は不変)。
    // P29-004(E3: 生産能力に基づく AI 募兵制限)後: Dwarf は生産能力を超える募兵をしなくなり、
    // 120 か月で Division 3(旧 16)。Iron は枯渇せず(2323)、D4 の優先配分が成立する局面自体が
    // 現れない。Passguard は変わらない(下の検査。FIX1 が入力引落しの失敗の無い国を変えない
    // という本テストの目的はこちらで維持される)。
    assert!(row.contains("c2 divs=3 shortD=3161 Iron=2319 Coal=4156 IG=9093"), "Dwarf: {row}");
    assert!(row.contains("tr=15243"), "Dwarf treasury: {row}");
    assert!(row.contains("c4 divs=1 shortD=119 Iron=37 Coal=2900 IG=18"), "Passguard: {row}");
    assert!(row.contains("tr=904"), "Passguard treasury: {row}");
}
