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
    /// 期間中に州を 1 つも所有しなくなった(征服された)国。
    eliminated: [bool; 6],
    /// 州を 1 つも所有しなくなった最初の月(月次サンプルの添字)。
    eliminated_month: [Option<usize>; 6],
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
        eliminated: [false; 6],
        eliminated_month: [None; 6],
    };
    let mut month_i = 0u32;
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
        if app.world().resource::<GameDate>().day == 1 {
            month_i += 1;
            for c in ALL_SIX {
                let post = stocks(&app, c);
                // 月次生産(Economy)の後、同じフレームの講和・占領で州の所有が変わった国は、
                // 再構成(現在の所有・稼働率)が生産時点とずれるため、保存則の照合から外す
                // (P29-005: 世界の分岐で、月初の同一フレームに州が移る月が現れた)。
                let territory_changed = owned_before[c.0] != owned_after[c.0];
                if owned_after[c.0].is_empty() {
                    s.eliminated[c.0] = true;
                    s.eliminated_month[c.0].get_or_insert(month_i as usize);
                }
                let (out, used, fi, mi, ms) = flows(&app, c);
                for i in 0..4 {
                    let resid = post[i] - (pre[c.0][i] + out[i] - used[i]);
                    if !territory_changed {
                        s.max_resid = s.max_resid.max(resid.abs());
                    }

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
    // P29-004(E3: 生産能力に基づく AI 募兵制限)の導入後の値(120・240 か月)。AI が生産能力を
    // 超える募兵をしなくなり、Dwarf は Division 3(旧 16)のまま、Iron は枯渇せず(Iron 備蓄が
    // 残るため D4 の優先配分が成立する局面自体が現れない)、IG 備蓄が積み上がる。この
    // テストの目的(保存則の閉鎖・備蓄の非負・Iron の据え置きなし)は上の検査で維持され、D4 の
    // 契約(適用条件・D0 への復帰・予算内の消費)は fixture テスト(12 件・単体 7 件)が担保する。
    // P29-005(複数月の実産出に基づく能力上限)で、m120/m240 の値を更新した(世界の分岐: Elfin が
    // 12 個まで増え、Passguard は 1 州を保持する)。
    // 世界の経過: E1 で戦争が分岐し(P29-003)、さらに E3 で募兵が変わる。
    // P31-002: M1(両陣営に師団がある停滞戦争の合意白紙講和)で戦争の経過が変わり(世界の分岐、報告書
    // p31-002 §7)、m120/m240 の値を更新した。保存則・備蓄の非負・Iron の据え置きなしの検査は維持。
    assert!(m120.contains("c2 divs=4 shortD=1837 IG=10172 tr=20043"), "{m120}");
    assert!(m240.contains("c2 divs=10 shortD=1837 IG=33674 tr=24421"), "{m240}");
    assert!(m240.contains("c1 divs=6 shortD=2734 IG=35 tr=6313"), "{m240}");
    assert!(m240.contains("c5 divs=2 shortD=3373 IG=5334 tr=1776"), "{m240}");
    // Passguard: Division 1・補給不足 119 日は旧 D0 の値(不変)。IG・国庫・生活水準は、
    // 戦争の分岐による州の増減で変わる。
    assert!(m240.contains("c4 divs=3 shortD=119 IG=19 tr=2976 LS=64.8"), "{m240}");
    assert!(m240.contains("c0 divs=0 shortD=3755 IG=8 tr=2616"), "{m240}");
    // 旧世界の「Ferrowyn 対 Oceanic の WarId(8) が 1819/02/07 に決着」は発生しない。P29-005 後の
    // WarId(8) は別の戦争(Elfin 対 Dwarf+Passguard、1817/03/02 開戦)で、1817/03/13 に攻撃側が
    // 降伏勝利する(変更前の世界では同じ組合せの戦争が 1817/03/02 から継続していた)。
    // P31-002: WarId(8) は、M1 後の世界では Elfin+Dwarf 対 Oceanic(1807/05/02 開戦)で、1807/05/22 に
    // 攻撃側が降伏勝利する。
    assert_eq!(s.war8_end.as_deref(), Some("1807/05/22"));
}

/// Passguardで D1/D3 が起こした「FactoryがIron不足→IG枯渇→MilitaryFactory停止→Ironが余って
/// Factory再起動」の月次振動が、D4では起きない(1〜250か月)。
#[test]
fn passguard_does_not_enter_the_d1_d3_starvation_oscillation() {
    let s = simulate(250, &[]);
    let p = &s.monthly[4];
    assert_eq!(p.len(), 250);
    // 序盤(Mine稼働前)を除く。E1 後の世界では、Passguard は 1812 年(約 m144)に Factory を持つ
    // 州を失う。Factory がある間(Factory の Iron 引落しが 0 でない間)だけを対象にする。
    let tail_all = &p[20..];
    let owned = tail_all.iter().position(|x| x.0 == 0.0).unwrap_or(tail_all.len());
    assert!(owned >= 100, "Factory を保持する期間が十分長い(観測 {owned} か月)");
    let tail = &tail_all[..owned];
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
    // 国庫の破産月数: D0 は序盤の 17 か月。P29-007(線形補間)では 18 か月(月 6〜23、最小 -63)。
    let neg = p.iter().filter(|x| x.4 < 0.0).count();
    assert!(neg <= 18, "negative treasury months {neg}");
}

/// 600か月の長期: 新たな破産・暴走的な税率振動が無いこと。
/// 観測上限であり目標値ではない。上限を超えたら因果を調べる(D4閾値は調整しない)。
#[test]
fn long_horizon_600_months_has_no_new_insolvency_or_tax_runaway() {
    let s = simulate(600, &[]);
    assert!(s.max_resid < 1e-3);
    assert_eq!(s.frozen_iron_months, 0);
    for c in 0..6 {
        // P29-007: 征服される国(消滅した国)は、消滅直前の戦時の赤字を許容する(下記)。消滅後は凍結した
        // 値のため数えない。州を保持する間の赤字月は従来どおり数える。
        // P31-002: 許容する赤字を「消滅月で終わる連続した赤字の末尾」(終末の衰退)として定義し直した
        // (従来は消滅前の固定 6 か月)。M1 で戦争の経過が変わり、征服される国の最後の衰退が長くなった:
        // Passguard は月 572〜585 の 14 か月、Ferrowyn は月 450〜477 の 28 か月(州を失い続けて小国になり、
        // 最後の戦争で消滅する。税率は 1.0 以下)。終末の衰退の長さは 36 か月以内とし、それ以前の
        // (散発的・再発する)赤字月は従来どおり数える。再発する破産・暴走の検出という目的は維持する。
        let len = s.monthly[c].len();
        let end = s.eliminated_month[c].map_or(len, |m| m.min(len));
        let before: Vec<bool> = s.monthly[c][..end].iter().map(|x| x.4 < 0.0).collect();
        let terminal_run = if s.eliminated[c] {
            before.iter().rev().take_while(|&&n| n).count()
        } else {
            0
        };
        assert!(terminal_run <= 36, "c{c}: 終末の赤字が長すぎる {terminal_run}");
        let neg = before.iter().filter(|&&n| n).count() - terminal_run;
        let max_tax = s.monthly[c].iter().map(|x| x.3).fold(0.0f32, f32::max);
        // P29-005 の世界では、Passguard(c4)が Elfin の連続する侵攻で全州を失う。征服される国は、
        // 州と税収を失う過程で税率を上げ続け(国庫は正のまま)、消滅後は凍結する。これは暴走ではなく、
        // 戦争の結果であるため、税率の上限(0.25)は、州を保持し続けた国にだけ適用する。
        // Passguardだけが序盤(最初の数十か月)に国庫が負になる(D0と同じ17か月)。
        // P29-007: 18 か月(D0 は 17 か月、最小 -63)。IG 備蓄が 3 か月分を下回る序盤に、線形補間の
        // 優先配分が Factory の Iron を減らすため、赤字が 1 か月長い(月 6〜23)。
        let allowed = if c == 4 { 18 } else { 0 };
        assert!(neg <= allowed, "c{c}: negative treasury months {neg}");
        if s.eliminated[c] {
            assert!(max_tax <= 1.0, "c{c}: tax {max_tax}");
        } else {
            assert!(max_tax <= 0.25 + 1e-6, "c{c}: tax runaway {max_tax}");
        }
    }
    // 税率の反転回数(観測: Arcadia 3、Elfin 8、Dwarf 5、Passguard 39、Ferrowyn 5。D0: 3/10/5/5/41。E1 後は下記)。
    // Passguardの39回は、IG備蓄がゲート(3か月分)付近で優先配分と比例配分が月ごとに切り替わる
    // 閾値での振動(振幅 0.01、国庫・生活水準は不変)。D4の契約上の既知の挙動で、報告書に記載。
    let rev: Vec<u32> = (0..6).map(|c| reversals(&s.monthly[c])).collect();
    // P31-002: Dwarf の反転回数の上限を 10 → 16 へ更新した(観測 15。M1 で戦争の経過が変わり、Dwarf が
    // 多くの州を獲得する世界になった。600 か月で税率の変更は 20 回、すべて 0.01 刻みで、帯は 0.11〜0.16)。
    // 崩壊型(振幅の大きい振動・破産・暴走)ではないことを、帯幅 0.05 以内の直接検査で担保する。
    assert!(rev[2] <= 16, "Dwarf tax reversals {}", rev[2]);
    {
        let band: Vec<f32> = s.monthly[2].iter().map(|x| x.3).collect();
        let (lo, hi) = band.iter().fold((f32::MAX, f32::MIN), |(l, h), &t| (l.min(t), h.max(t)));
        assert!(hi - lo <= 0.05 + 1e-6, "Dwarf tax band {lo}..{hi}");
    }
    // E1 後の世界(観測: Arcadia 3、Elfin 62、Dwarf 5、Passguard 9、Ferrowyn 4): Elfin は約 m190〜m374 に、
    // 税率が 0.12 と 0.14 の間を往復する有界な振動を示す(国庫は常に正、上限 0.25 未満、振幅は
    // 0.02 以内。単発の 0.04 の変化は州獲得後の再調整)。暴走・破産ではない。
    // P29-005 後の世界(観測: Arcadia 3、Elfin 101、Dwarf 3、Passguard 6〜8、Ferrowyn 2): Elfin の 101 回は、
    // m338 以降の 0.12 と 0.13 の間の 1 段(0.01)の往復で、国庫は常に正(最小 2499)・上限 0.25 未満。
    // P28-010 が検出対象とした崩壊型(振幅の大きい振動・破産・暴走)ではない。回数の上限は 110 へ
    // 緩め、代わりに「Elfin の振幅が 0.01 以内(m338 以降)・国庫が非負」を直接検査する。
    assert!(rev[1] <= 110 && rev[5] <= 10 && rev[0] <= 6, "{rev:?}");
    // P29-009(Iron 不足を反映する建設 AI)後の世界: Elfin の税率は 0.13〜0.15 の間の単発の段階的な変更
    // (600 か月で 13 回、反転 6 回、国庫の最小 2499)に落ち着き、P29-005 後の 0.12/0.13 の往復
    // (反転 101 回)は消える。帯幅の上限は 0.02、国庫は常に正を維持する。
    let tail: Vec<f32> = s.monthly[1][338..].iter().map(|x| x.3).collect();
    let (lo, hi) = tail.iter().fold((f32::MAX, f32::MIN), |(l, h), &t| (l.min(t), h.max(t)));
    assert!(hi - lo <= 0.02 + 1e-6, "Elfin tax band {lo}..{hi}");
    assert!(s.monthly[1].iter().all(|x| x.4 > 0.0), "Elfin treasury must stay positive");
    for c in 0..6 {
        let max_step = s.monthly[c]
            .windows(2)
            .map(|w| (w[1].3 - w[0].3).abs())
            .fold(0.0f32, f32::max);
        assert!(max_step <= 0.05 + 1e-6, "c{c}: tax step {max_step}");
    }
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




