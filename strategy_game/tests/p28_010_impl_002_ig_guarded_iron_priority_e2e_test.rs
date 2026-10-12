//! P28-010-IMPL-002: 実データ・全6か国の実Appで、IG備蓄で保護したIron軍需優先配分(D4)の
//! 保存則・監査値との照合・Passguardの振動の不在・長期財政・プレイヤー適用・save→load・決定性を確認する。

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
use strategy_game::military::data::MilitaryRegistry;
use strategy_game::war::data::{WarRegistry, WarStatus};

const TRACKED: [ResourceType; 4] = [
    ResourceType::Iron,
    ResourceType::Coal,
    ResourceType::IndustrialGoods,
    ResourceType::MagicCrystal,
];

fn stocks(app: &App, c: CountryId) -> [f64; 4] {
    let cr = app.world().resource::<CountryRegistry>();
    let st = &cr.get(c).unwrap().stockpile;
    [st.get(TRACKED[0]), st.get(TRACKED[1]), st.get(TRACKED[2]), st.get(TRACKED[3])]
}

/// 月次の産出と引落しを、記録された稼働率から再構成する。戻り値: (産出, 引落し, Factory Iron, MilitaryFactory Iron, MS産出)
fn flows(app: &App, c: CountryId) -> ([f64; 4], [f64; 4], f64, f64, f64) {
    let br = app.world().resource::<BuildingRegistry>();
    let sr = app.world().resource::<StateRegistry>();
    let mut out = [0.0; 4];
    let mut used = [0.0; 4];
    let (mut fi, mut mi, mut ms) = (0.0, 0.0, 0.0);
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
                    if *r == ResourceType::Iron {
                        if b == BuildingType::Factory {
                            fi += base * l * op;
                        }
                        if b == BuildingType::MilitaryFactory {
                            mi += base * l * op;
                        }
                    }
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
            if b == BuildingType::MilitaryFactory {
                ms += def.output_resources.get(&ResourceType::MilitarySupplies).copied().unwrap_or(0.0) * l * op;
            }
        }
    }
    (out, used, fi, mi, ms)
}

/// 月ごとの(Factory Iron, MilitaryFactory Iron, MS産出, 税率, 国庫)。
type MonthlyRow = (f64, f64, f64, f32, f64);

struct Series {
    /// 月ごとの(Factory Iron, MilitaryFactory Iron, MS産出, 税率, 国庫)(国別)
    monthly: Vec<Vec<MonthlyRow>>,
    rows: Vec<String>,
    max_resid: f64,
    min_stock: f64,
    frozen_iron_months: u32,
    war8_end: Option<String>,
}

fn simulate(months: u32, report_at: &[u32]) -> Series {
    let dir = TempTestDir::new("impl002");
    let mut app = setup_app(None, dir.save_file_config());
    let end = format!("{:04}/{:02}/01", 1800 + (months / 12) as i32, 1 + months % 12);
    let mut pre = ALL_SIX.map(|c| stocks(&app, c));
    let mut shortage = [0u32; 6];
    let mut s = Series {
        monthly: vec![Vec::new(); 6],
        rows: Vec::new(),
        max_resid: 0.0,
        min_stock: f64::MAX,
        frozen_iron_months: 0,
        war8_end: None,
    };
    let mut month_i = 0u32;
    while date(&app).as_str() < end.as_str() {
        if app.world().resource::<GameDate>().add_days(1).day == 1 {
            pre = ALL_SIX.map(|c| stocks(&app, c));
        }
        advance_one_day(&mut app);
        {
            let mr = app.world().resource::<MilitaryRegistry>();
            for d in mr.divisions.values() {
                if mr.supply_fulfillment_of(d.id) < 0.34 {
                    shortage[d.owner.0] += 1;
                }
            }
        }
        if app.world().resource::<GameDate>().day == 1 {
            month_i += 1;
            for c in ALL_SIX {
                let post = stocks(&app, c);
                let (out, used, fi, mi, ms) = flows(&app, c);
                for i in 0..4 {
                    let resid = post[i] - (pre[c.0][i] + out[i] - used[i]);
                    s.max_resid = s.max_resid.max(resid.abs());
                    s.min_stock = s.min_stock.min(post[i]);
                }
                if used[0] > 1e-9 && (post[0] - pre[c.0][0]).abs() < 1e-12 && out[0] == 0.0 {
                    s.frozen_iron_months += 1;
                }
                let cr = app.world().resource::<CountryRegistry>();
                let ct = cr.get(c).unwrap();
                s.monthly[c.0].push((fi, mi, ms, ct.tax_rate, ct.treasury));
            }
            if report_at.contains(&month_i) {
                let cr = app.world().resource::<CountryRegistry>();
                let mr = app.world().resource::<MilitaryRegistry>();
                let sr = app.world().resource::<StateRegistry>();
                let mut line = format!("m{month_i}");
                for c in ALL_SIX {
                    let divs = mr.divisions.values().filter(|d| d.owner == c).count();
                    let own: Vec<_> = sr.states.iter().filter(|x| x.owner_country_id == c).collect();
                    let ls = own.iter().map(|x| x.living_standard as f64).sum::<f64>() / own.len().max(1) as f64;
                    let st = stocks(&app, c);
                    line.push_str(&format!(
                        " | c{} divs={} shortD={} IG={:.0} tr={:.0} LS={:.1}",
                        c.0, divs, shortage[c.0], st[2], cr.get(c).unwrap().treasury, ls
                    ));
                }
                s.rows.push(line);
            }
        }
    }
    let wr = app.world().resource::<WarRegistry>();
    if let Some(w) = wr
        .wars
        .get(&strategy_game::common::WarId(8))
        .filter(|w| w.status == WarStatus::AttackerVictory)
    {
        s.war8_end = w.end_date.clone();
    }
    s
}

fn reversals(history: &[MonthlyRow]) -> u32 {
    let mut dir = 0i32;
    let mut rev = 0u32;
    for w in history.windows(2) {
        let d = w[1].3 - w[0].3;
        if d.abs() < 1e-9 {
            continue;
        }
        let nd = if d > 0.0 { 1 } else { -1 };
        if dir != 0 && nd != dir {
            rev += 1;
        }
        dir = nd;
    }
    rev
}

#[test]
fn d4_240_months_keeps_the_ledger_closed_and_matches_the_audited_outcomes() {
    let s = simulate(240, &[120, 240]);
    assert!(s.max_resid < 1e-3, "residual {}", s.max_resid);
    assert!(s.min_stock >= 0.0);
    assert_eq!(s.frozen_iron_months, 0);
    let m120 = s.rows.iter().find(|r| r.starts_with("m120")).unwrap();
    let m240 = s.rows.iter().find(|r| r.starts_with("m240")).unwrap();
    // Iron枯渇後のDwarf(m100〜): MilitaryFactory優先でDivision増・補給不足率の改善。
    assert!(m120.contains("c2 divs=16 shortD=4649 IG=2838 tr=15340"), "{m120}");
    assert!(m240.contains("c2 divs=16 shortD=17065 IG=9425 tr=16462"), "{m240}");
    // Elfin・Ferrowyn: 補給不足日数の大幅減(旧D0: 23879 / 5069)。
    assert!(m240.contains("c1 divs=9 shortD=17066 IG=638 tr=8187"), "{m240}");
    assert!(m240.contains("c5 divs=8 shortD=2080 IG=6873 tr=3899"), "{m240}");
    // Passguard: D0と完全一致(旧D0 240か月: Division1・補給不足119・IG18・国庫1842・LS70.0)。
    assert!(m240.contains("c4 divs=1 shortD=119 IG=18 tr=1842 LS=70.0"), "{m240}");
    // Arcadia・Oceanic(MilitaryFactoryなし/Iron枯渇): D0と同一(旧D0の240か月値)。
    assert!(m240.contains("c0 divs=0 shortD=7835 IG=6 tr=3731"), "{m240}");
    // Ferrowynが Oceanic に勝つ戦争(WarId(8))が 1819/02/07 に決着(旧D0: 継続)。
    assert_eq!(s.war8_end.as_deref(), Some("1819/02/07"));
}

/// Passguardで D1/D3 が起こした「FactoryがIron不足→IG枯渇→MilitaryFactory停止→Ironが余って
/// Factory再起動」の月次振動が、D4では起きない(1〜250か月)。
#[test]
fn passguard_does_not_enter_the_d1_d3_starvation_oscillation() {
    let s = simulate(250, &[]);
    let p = &s.monthly[4];
    assert_eq!(p.len(), 250);
    // 序盤(Mine稼働前)を除く。
    let tail = &p[20..];
    let max_mf = tail.iter().map(|x| x.1).fold(f64::MIN, f64::max);
    let min_mf = tail.iter().map(|x| x.1).fold(f64::MAX, f64::min);
    let min_f = tail.iter().map(|x| x.0).fold(f64::MAX, f64::min);
    let max_ms = tail.iter().map(|x| x.2).fold(f64::MIN, f64::max);
    let min_ms = tail.iter().map(|x| x.2).fold(f64::MAX, f64::min);
    // D1の振動は MS 27〜153(月産の最小/最大 = 0.18)、Factoryの引落しが 2.5 まで落ちた。
    assert!(min_f > 5.0, "Factoryが飢えてはいけない: min Factory iron {min_f}");
    assert!(min_mf > 0.7 * max_mf, "MilitaryFactoryのIron引落しは安定: {min_mf}..{max_mf}");
    assert!(min_ms > 0.7 * max_ms, "MS月産は安定: {min_ms}..{max_ms}");
    // 税率の往復も、D0(同期間 5回)と同程度。
    assert!(reversals(p) <= 8, "tax reversals {}", reversals(p));
    // 国庫の破産月数は D0 と同じ(序盤の17か月)。
    let neg = p.iter().filter(|x| x.4 < 0.0).count();
    assert!(neg <= 17, "negative treasury months {neg}");
}

/// 600か月の長期: 新たな破産・暴走的な税率振動が無いこと。
/// 観測上限であり目標値ではない。上限を超えたら因果を調べる(D4閾値は調整しない)。
#[test]
fn long_horizon_600_months_has_no_new_insolvency_or_tax_runaway() {
    let s = simulate(600, &[]);
    assert!(s.max_resid < 1e-3);
    assert_eq!(s.frozen_iron_months, 0);
    for c in 0..6 {
        let neg = s.monthly[c].iter().filter(|x| x.4 < 0.0).count();
        let max_tax = s.monthly[c].iter().map(|x| x.3).fold(0.0f32, f32::max);
        // Passguardだけが序盤(最初の数十か月)に国庫が負になる(D0と同じ17か月)。
        let allowed = if c == 4 { 17 } else { 0 };
        assert!(neg <= allowed, "c{c}: negative treasury months {neg}");
        assert!(max_tax <= 0.25 + 1e-6, "c{c}: tax runaway {max_tax}");
    }
    // 税率の反転回数(観測: Arcadia 3、Elfin 8、Dwarf 5、Passguard 39、Ferrowyn 5。D0: 3/10/5/5/41)。
    // Passguardの39回は、IG備蓄がゲート(3か月分)付近で優先配分と比例配分が月ごとに切り替わる
    // 閾値での振動(振幅 0.01、国庫・生活水準は不変)。D4の契約上の既知の挙動で、報告書に記載。
    let rev: Vec<u32> = (0..6).map(|c| reversals(&s.monthly[c])).collect();
    assert!(rev[2] <= 10, "Dwarf tax reversals {}", rev[2]);
    assert!(rev[1] <= 12 && rev[5] <= 10 && rev[0] <= 6, "{rev:?}");
    assert!(rev[4] <= 45, "Passguard tax reversals {}", rev[4]);
}

#[test]
fn d4_is_deterministic_across_two_runs() {
    let a = simulate(130, &[36, 120]);
    let b = simulate(130, &[36, 120]);
    assert_eq!(a.rows, b.rows);
    assert_eq!(a.monthly, b.monthly);
}

fn month_flows(player: Option<CountryId>, iron: f64, ig: f64) -> (f64, f64) {
    let dir = TempTestDir::new("impl002_player");
    let mut app = setup_app(player, dir.save_file_config());
    {
        let mut cr = app.world_mut().resource_mut::<CountryRegistry>();
        let st = &mut cr.get_mut(DWARF).unwrap().stockpile;
        st.set(ResourceType::Iron, iron);
        st.set(ResourceType::IndustrialGoods, ig);
    }
    // 初回の月次経済(1800/02/01)まで進める。
    loop {
        advance_one_day(&mut app);
        if app.world().resource::<GameDate>().day == 1 {
            break;
        }
    }
    let (_, _, fi, mi, _) = flows(&app, DWARF);
    (fi, mi)
}

/// プレイヤー国家にも同じD4が適用される(国家IDによる特例なし)。
/// Dwarfをプレイヤーにし、Iron不足・IG十分(優先配分が成立)とIG不足(D0)でFactory/MilitaryFactoryの
/// Iron配分が切り替わること、およびそれがAI国家のときと同じであることを確認する。
#[test]
fn player_country_uses_the_same_rule_as_ai_countries() {
    let (f_rich_player, m_rich_player) = month_flows(Some(DWARF), 15.0, 50_000.0);
    let (f_poor_player, m_poor_player) = month_flows(Some(DWARF), 15.0, 5.0);
    let (f_rich_ai, m_rich_ai) = month_flows(None, 15.0, 50_000.0);
    let (f_poor_ai, m_poor_ai) = month_flows(None, 15.0, 5.0);
    // プレイヤーとAIで配分は同一(建設AIの差は初月には影響しない)。
    assert!((f_rich_player - f_rich_ai).abs() < 1e-9 && (m_rich_player - m_rich_ai).abs() < 1e-9);
    assert!((f_poor_player - f_poor_ai).abs() < 1e-9 && (m_poor_player - m_poor_ai).abs() < 1e-9);
    // IG十分: MilitaryFactoryがIronを優先確保し、Factoryの取り分(予算)は残りの按分に減る。
    // MilitaryFactoryは両方の月で計画稼働率(この初月は Mine の当月産出で賄える)。
    assert!(m_rich_player >= m_poor_player - 1e-9, "MF iron rich={m_rich_player} poor={m_poor_player}");
    assert!(f_rich_player < f_poor_player, "F iron rich={f_rich_player} poor={f_poor_player}");
}

fn send_save_request(app: &mut App) {
    let mut s: SystemState<MessageWriter<SaveRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut()).unwrap().write(SaveRequestMessage);
    s.apply(app.world_mut());
    app.update();
}

fn send_load_request(app: &mut App) {
    let mut s: SystemState<MessageWriter<LoadRequestMessage>> = SystemState::new(app.world_mut());
    s.get_mut(app.world_mut()).unwrap().write(LoadRequestMessage);
    s.apply(app.world_mut());
    app.update();
    app.update();
}

fn fingerprint(app: &App) -> String {
    let mut s = String::new();
    let cr = app.world().resource::<CountryRegistry>();
    for c in ALL_SIX {
        let ct = cr.get(c).unwrap();
        s.push_str(&format!("c{} tr={:.4} tax={} ", c.0, ct.treasury, ct.tax_rate));
        for r in [
            ResourceType::Iron,
            ResourceType::Coal,
            ResourceType::IndustrialGoods,
            ResourceType::MilitarySupplies,
            ResourceType::MilitaryEquipment,
        ] {
            s.push_str(&format!("{:?}={:.4} ", r, ct.stockpile.get(r)));
        }
        s.push('\n');
    }
    s
}

/// IG閾値の前後(Dwarf: Iron枯渇でD4が成立する月 m99付近、Passguard: ゲートが月ごとに切り替わる
/// m266付近)でsave→loadを挟んでも、連続実行と一致する。
#[test]
fn save_load_around_the_ig_gate_matches_continuous_run() {
    for (save_month, end_month) in [(98u32, 112u32), (264, 280)] {
        let end = format!("{:04}/{:02}/01", 1800 + (end_month / 12) as i32, 1 + end_month % 12);
        let save_at = format!("{:04}/{:02}/01", 1800 + (save_month / 12) as i32, 1 + save_month % 12);

        let dir_a = TempTestDir::new("impl002_cont");
        let mut cont = setup_app(None, dir_a.save_file_config());
        while date(&cont).as_str() < end.as_str() {
            advance_one_day(&mut cont);
        }

        let dir_b = TempTestDir::new("impl002_branch");
        let mut branch = setup_app(None, dir_b.save_file_config());
        while date(&branch).as_str() < save_at.as_str() {
            advance_one_day(&mut branch);
        }
        send_save_request(&mut branch);
        send_load_request(&mut branch);
        while date(&branch).as_str() < end.as_str() {
            advance_one_day(&mut branch);
        }
        assert_eq!(fingerprint(&branch), fingerprint(&cont), "save at m{save_month}");
    }
}
