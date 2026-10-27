//! P32-019 (S0): 経済セッションの組み立ての共通化、操作主体の検証、建設の受付可否確認(`check_order`/`check_build`)、
//! 保存ファイルの事前確認(`inspect`)、保存の一覧(`list_saves`)のテスト。
//! 生成するファイルはすべてOSの一時ディレクトリ(テストごとに一意)に置き、`saves/` には書かない。
//! 旧V1の負例は一時ディレクトリ内の**コピー**で行う(実ファイルは読み取りのみ)。

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use strategy_game::building::data::{BuildingRegistry, BuildingType};
use strategy_game::economy::map_economy_file::{
    DetectedKind, DirectoryStatus, EntryStatus, FileError, SaveLocation, decode, encode, read_file,
};
use strategy_game::economy::map_session::{
    ActorError, BuildRequestError, CLI_DEFAULT_SCENARIO, CancelRequestError, EconomySession, GeographyData,
    SCENARIOS, SCREEN_DEFAULT_SCENARIO, SessionError, SessionPaths, build_simulation, build_simulation_from_text,
    scenario_by_id,
};
use strategy_game::economy::map_state::snapshot::MapEconomySimulation;
use strategy_game::economy::map_state::static_identity::{ItemKey, StaticIdentity, StaticInputs};
use strategy_game::economy::map_state::{
    CancelError, MapStateEconomyRegistry, OrderError, SimDate, parse_multi_month_scenario,
};
use strategy_game::economy::province_resources::{derive_state_resources, load_province_resources};
use strategy_game::world::MapStateId;

const P: &str = "assets/data/world/preview/";

fn geo() -> &'static GeographyData {
    static G: OnceLock<GeographyData> = OnceLock::new();
    G.get_or_init(|| GeographyData::load(&SessionPaths::default()).unwrap())
}

fn session(id: &str) -> EconomySession {
    EconomySession::load_files(scenario_by_id(id).unwrap(), geo().model(), &SessionPaths::default()).unwrap()
}

fn buildings() -> BuildingRegistry {
    BuildingRegistry::from_ron(&fs::read_to_string("assets/data/buildings.ron").unwrap()).unwrap()
}

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!(
            "p32_019_{tag}_{}_{nanos}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn loc(&self) -> SaveLocation {
        SaveLocation::new(&self.0)
    }
    fn dir(&self) -> PathBuf {
        self.0.join("map_economy")
    }
    fn file(&self, name: &str) -> PathBuf {
        self.dir().join(format!("{name}.mapeco.ron"))
    }
    fn write_raw(&self, file_name: &str, content: &[u8]) {
        fs::create_dir_all(self.dir()).unwrap();
        fs::write(self.dir().join(file_name), content).unwrap();
    }
    /// 保存先の全ファイルの(名前, 内容)。
    fn snapshot(&self) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<(String, Vec<u8>)> = fs::read_dir(self.dir())
            .map(|d| {
                d.map(|e| {
                    let e = e.unwrap();
                    (e.file_name().to_string_lossy().to_string(), fs::read(e.path()).unwrap_or_default())
                })
                .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fnv_digest(sim: &MapEconomySimulation, session: &EconomySession) -> u64 {
    let text = ron::to_string(&sim.snapshot(geo().model(), &session.buildings)).unwrap();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// ---------------------------------------------------------------- 組み立て

/// 共通化前のCLI(`m4d_simulation`)と同じ手順を、テスト内に独立に書いた基準の組み立て。
fn reference_assembly(scenario: &str, resources: &str, buildings: &BuildingRegistry) -> MapEconomySimulation {
    let sc = parse_multi_month_scenario(&fs::read_to_string(format!("{P}{scenario}")).unwrap()).unwrap();
    let res = load_province_resources(&format!("{P}{resources}")).unwrap();
    let w = geo().model();
    let (mut reg, cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, w, buildings).unwrap();
    let d = derive_state_resources(w, &res).unwrap();
    reg.attach_province_resources(&d, w, &res).unwrap();
    MapEconomySimulation::new(SimDate { year: 1800, month: 1, day: 1 }, reg, cs)
}

#[test]
fn the_library_assembly_equals_the_previous_cli_assembly_for_every_known_scenario() {
    let b = buildings();
    for (id, scenario_file) in [("m4d", "economy_scenario_m4d.ron"), ("m4e", "economy_scenario_m4e.ron")] {
        let s = session(id);
        let reference = reference_assembly(scenario_file, "province_resources_m4d.ron", &b);
        assert_eq!(s.sim, reference, "{id}: initial state is identical");
        let inputs = s.inputs(&geo().map, geo().model());
        let a = StaticIdentity::compute(&s.sim, &inputs).unwrap();
        let ref_inputs = StaticInputs { map: &geo().map, world: geo().model(), buildings: &b, facilities: s.facilities.as_ref() };
        let r = StaticIdentity::compute(&reference, &ref_inputs).unwrap();
        for k in ItemKey::ALL {
            assert_eq!(a.digest(k), r.digest(k), "{id}: identity item {k}");
        }
        assert_eq!(s.identity(&geo().map, geo().model()).unwrap(), a);
        // 複数日の実行結果が維持される。
        let mut lib = s;
        let mut reference = reference;
        for _ in 0..200 {
            let (d1, r1) = lib.advance_day();
            let (d2, r2) = reference.advance_day(&b);
            assert_eq!((d1, r1), (d2, r2), "{id}");
        }
        assert_eq!(lib.sim, reference, "{id}: 200 days");
    }
}

#[test]
fn the_cli_baseline_recorded_in_p32_017_is_reproduced_by_the_library_assembly() {
    // `verification_logs/phase-32/p32-017/cli_identity_output.txt`(共通化前のCLIの出力)の記録値。
    // m4d を100日進めた直後の翌日(1800/04/12)と、160日目(1800/06/10)の状態ダイジェスト。
    let mut s = session("m4d");
    for _ in 0..101 {
        s.advance_day();
    }
    assert_eq!(s.sim.date, SimDate { year: 1800, month: 4, day: 12 });
    assert_eq!(fnv_digest(&s.sim, &s), 0x0efe_fcfc_fc39_8595);
    for _ in 101..160 {
        s.advance_day();
    }
    assert_eq!(s.sim.date, SimDate { year: 1800, month: 6, day: 10 });
    assert_eq!(fnv_digest(&s.sim, &s), 0x7f6f_2d0d_dbd2_cc19);
}

#[test]
fn the_screen_default_is_m4e_with_m4d_resources_and_the_cli_default_stays_m4d() {
    assert_eq!(SCREEN_DEFAULT_SCENARIO, "m4e");
    assert_eq!(CLI_DEFAULT_SCENARIO, "m4d");
    let e = scenario_by_id("m4e").unwrap();
    assert!(e.resources_path.ends_with("province_resources_m4d.ron"), "m4e uses the m4d resource setup");
    assert!(scenario_by_id("m4d").unwrap().resources_path.ends_with("province_resources_m4d.ron"));
    // 対応関係が検証を通る(組み立てに成功し、国家・ステートが期待どおり)。
    let s = session("m4e");
    assert_eq!(s.sim.registry.len(), 3);
    assert_eq!(s.sim.countries.iter().map(|c| c.label).collect::<Vec<_>>(), vec![1, 2, 3]);
    // m4e のシナリオ自身が、この資源設定を前提にすると明記している(黙って組み合わせていない)。
    let text = fs::read_to_string(e.scenario_path).unwrap();
    assert!(text.contains("province_resources_m4d.ron"), "the scenario file names its resource file");
    for sc in &SCENARIOS {
        assert_eq!(scenario_by_id(sc.id), Some(sc));
    }
    assert!(scenario_by_id("m9").is_none());
}

#[test]
fn broken_or_mismatched_inputs_are_reported_not_silently_combined() {
    let w = geo().model();
    let b = buildings();
    let sc = fs::read_to_string(format!("{P}economy_scenario_m4e.ron")).unwrap();
    let res = fs::read_to_string(format!("{P}province_resources_m4d.ron")).unwrap();
    let start = SimDate { year: 1800, month: 1, day: 1 };
    assert!(build_simulation_from_text(&sc, &res, start, w, &b).is_ok());
    // シナリオが壊れている
    assert!(matches!(build_simulation_from_text("(", &res, start, w, &b), Err(SessionError::Scenario(_))));
    // 資源設定が壊れている / 存在しないプロヴィンスを含む
    assert!(matches!(build_simulation_from_text(&sc, "(", start, w, &b), Err(SessionError::Data { .. })));
    let bad = res.replacen("(province: 17906,", "(province: 999999,", 1);
    assert!(matches!(build_simulation_from_text(&sc, &bad, start, w, &b), Err(SessionError::Data { .. })));
    // 国家のステートがワールドに無い
    let bad_sc = sc.replacen("states: [22],", "states: [99],", 1);
    assert!(matches!(build_simulation_from_text(&bad_sc, &res, start, w, &b), Err(SessionError::Scenario(_))));
    // 読み込みの失敗(存在しないシナリオファイル)
    let paths = SessionPaths { buildings: "assets/data/does_not_exist.ron".to_string(), ..SessionPaths::default() };
    let err = EconomySession::load_files(scenario_by_id("m4e").unwrap(), w, &paths).err().expect("must fail");
    assert!(matches!(err, SessionError::Io { .. }), "{err}");
    let _ = build_simulation(scenario_by_id("m4d").unwrap(), w, &b).unwrap();
    let msg = SessionError::UnknownScenario("zzz".into()).to_string();
    assert!(msg.contains("m4d") && msg.contains("m4e"), "{msg}");
}

#[test]
fn building_definitions_are_validated_by_the_shared_parser() {
    let text = fs::read_to_string("assets/data/buildings.ron").unwrap();
    let reg = BuildingRegistry::from_ron(&text).unwrap();
    assert_eq!(reg.definitions.len(), BuildingType::ALL.len());
    let neg = text.replacen("construction_cost: 200.0", "construction_cost: -1.0", 1);
    assert!(BuildingRegistry::from_ron(&neg).err().unwrap().contains("negative construction cost"));
    let zero = text.replacen("required_progress: 30.0", "required_progress: 0.0", 1);
    assert!(BuildingRegistry::from_ron(&zero).err().unwrap().contains("required_progress"));
    let lvl = text.replacen("max_level: 10", "max_level: 0", 1);
    assert!(BuildingRegistry::from_ron(&lvl).err().unwrap().contains("max_level of 0"));
    assert!(BuildingRegistry::from_ron("[(").err().unwrap().contains("Failed to parse"));
}

// ---------------------------------------------------------------- 操作主体

#[test]
fn actors_are_explicit_manual_countries_and_there_is_no_fallback_to_the_first_country() {
    let mut s = session("m4e");
    assert_eq!(s.manual_actors(), vec![1, 2], "candidates only; nothing is selected by default");
    assert_eq!(s.resolve_actor(Some(1)), Ok(1));
    assert_eq!(s.resolve_actor(Some(2)), Ok(2));
    assert_eq!(s.resolve_actor(None), Err(ActorError::NotSelected));
    assert_eq!(s.resolve_actor(Some(9)), Err(ActorError::UnknownCountry(9)));
    assert_eq!(s.resolve_actor(Some(3)), Err(ActorError::NotManual(3)), "AI country");
    // 経済計算上の対応(国家1はステート20だけ)。
    assert_eq!(s.resolve_actor_for_state(Some(1), MapStateId(20)), Ok(1));
    assert_eq!(
        s.resolve_actor_for_state(Some(1), MapStateId(21)),
        Err(ActorError::StateNotMapped { actor: 1, state: MapStateId(21) })
    );
    assert_eq!(s.resolve_actor_for_state(None, MapStateId(20)), Err(ActorError::NotSelected));
    // 未選択・存在しない・AI国家では、状態変更の操作を受け付けず、状態は変わらない(別国家に置き換えない)。
    let before = s.sim.clone();
    for sel in [None, Some(9), Some(3)] {
        let expected = s.resolve_actor(sel).unwrap_err();
        assert_eq!(
            s.request_build(sel, MapStateId(20), BuildingType::Farm),
            Err(BuildRequestError::Actor(expected)),
            "{sel:?}"
        );
        let id = strategy_game::economy::map_state::ConstructionOrderId { country: 1, seq: 1 };
        assert_eq!(s.cancel_order(sel, id).unwrap_err(), CancelRequestError::Actor(expected), "{sel:?}");
        assert_eq!(s.check_build(sel, MapStateId(20), BuildingType::Farm), Err(BuildRequestError::Actor(expected)));
    }
    assert_eq!(s.sim, before, "rejected requests change nothing (no queue entry, no sequence number, no payment)");
    // m4d は全国家がAI: 操作できる国家がなく、先頭の国家へのフォールバックもない。
    let d = session("m4d");
    assert!(d.manual_actors().is_empty());
    assert_eq!(d.resolve_actor(Some(1)), Err(ActorError::NotManual(1)));
    assert_eq!(d.resolve_actor(None), Err(ActorError::NotSelected));
}

#[test]
fn reference_queries_work_without_an_actor_but_never_report_an_order_as_acceptable() {
    let s = session("m4e");
    assert!(s.state_economy(MapStateId(20)).is_some());
    assert!(s.state_economy(MapStateId(99)).is_none());
    assert_eq!(s.country(1).unwrap().label, 1);
    assert!(s.country(9).is_none());
    for b in BuildingType::ALL {
        assert!(s.check_build(None, MapStateId(20), b).is_err(), "{b:?}: an unselected actor is never acceptable");
    }
}

// ---------------------------------------------------------------- 受付可否の確認

/// `check_build` と `request_build`(複製した状態への実注文)が、全組み合わせで同じ結果になる。
fn assert_check_matches_request(s: &EconomySession, selections: &[Option<u32>], states: &[u32]) {
    for &sel in selections {
        for &st in states {
            for b in BuildingType::ALL {
                let state = MapStateId(st);
                let check = s.check_build(sel, state, b);
                let mut clone = s.clone();
                let order = clone.request_build(sel, state, b);
                match (check, order) {
                    (Ok(c), Ok(id)) => {
                        let country = clone.country(id.country).unwrap();
                        let placed = country.construction_queue.last().unwrap();
                        assert_eq!(placed.id, id);
                        assert_eq!(
                            (placed.state, placed.building_type, placed.target_level, placed.required_progress, placed.paid_cost),
                            (c.state, c.building, c.target_level, c.required_progress, c.cost),
                            "{sel:?} {st} {b:?}: the check describes the order that is placed"
                        );
                        let before = s.country(id.country).unwrap().treasury;
                        assert_eq!(country.treasury, before - c.cost, "{sel:?} {st} {b:?}");
                    }
                    (Err(e1), Err(e2)) => {
                        assert_eq!(e1, e2, "{sel:?} {st} {b:?}: same rejection reason");
                        assert_eq!(clone.sim, s.sim, "a rejected request changes nothing");
                    }
                    (c, o) => panic!("{sel:?} {st} {b:?}: check {c:?} disagrees with request {o:?}"),
                }
            }
        }
    }
}

#[test]
fn check_build_agrees_with_request_build_for_every_actor_state_and_building() {
    let s = session("m4e");
    assert_check_matches_request(&s, &[None, Some(1), Some(2), Some(3), Some(9)], &[19, 20, 21, 22]);
    // 途中の状態(施設あり・建設中・予算不足・クリスタル/Mine条件)でも一致する。
    let mut s = session("m4e");
    s.request_build(Some(1), MapStateId(20), BuildingType::Farm).unwrap();
    s.request_build(Some(2), MapStateId(21), BuildingType::Mine).unwrap();
    for _ in 0..35 {
        s.advance_day();
    }
    assert_check_matches_request(&s, &[Some(1), Some(2)], &[20, 21, 22]);
    s.sim.countries[0].treasury = 10.0; // 予算不足
    assert_check_matches_request(&s, &[Some(1)], &[20]);
}

#[test]
fn check_build_matches_request_build_for_unconfigured_resources() {
    // 国家3を手動にし、MapState#22 の 17268 の資源設定を外した検証用の変種(P32-013 の変種と同じ)。
    let w = geo().model();
    let b = buildings();
    let sc = fs::read_to_string(format!("{P}economy_scenario_m4e.ron")).unwrap().replacen("ai_controlled: true", "ai_controlled: false", 1);
    let res: String = fs::read_to_string(format!("{P}province_resources_m4d.ron"))
        .unwrap()
        .lines()
        .filter(|l| !l.contains("province: 17268"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut s = session("m4e");
    s.sim = build_simulation_from_text(&sc, &res, SimDate { year: 1800, month: 1, day: 1 }, w, &b).unwrap();
    assert_eq!(s.manual_actors(), vec![1, 2, 3]);
    let e = s.check_build(Some(3), MapStateId(22), BuildingType::Mine).unwrap_err();
    assert!(matches!(e, BuildRequestError::Order(OrderError::Precondition(_))), "{e:?}");
    assert_check_matches_request(&s, &[Some(3)], &[22]);
}

#[test]
fn checking_changes_nothing_and_does_not_consume_order_numbers() {
    let mut s = session("m4e");
    for _ in 0..12 {
        s.advance_day();
    }
    let before = s.sim.clone();
    let seq_before = s.country(1).unwrap().next_order_seq;
    for _ in 0..3 {
        for st in [19u32, 20, 21, 22] {
            for b in BuildingType::ALL {
                let _ = s.check_build(Some(1), MapStateId(st), b);
                let _ = s.check_build(None, MapStateId(st), b);
            }
        }
    }
    assert_eq!(s.sim, before, "the whole runtime state (treasury, stockpile, queue, seq, order_log, AI history, levels) is identical");
    assert_eq!(s.country(1).unwrap().next_order_seq, seq_before);
    // 確認だけでは連番を消費しない: 最初の実注文が最初の番号を受け取る。
    let id = s.request_build(Some(1), MapStateId(20), BuildingType::Farm).unwrap();
    assert_eq!(id.seq, seq_before);
}

#[test]
fn an_acceptable_check_is_not_trusted_the_real_order_is_revalidated_against_the_current_state() {
    let mut s = session("m4e");
    let ok = s.check_build(Some(1), MapStateId(20), BuildingType::Factory).unwrap();
    assert!(ok.cost > 0.0);
    // 確認後に国庫が減ると、実注文は拒否される(古い確認結果では通らない)。
    s.sim.countries[0].treasury = ok.cost - 1.0;
    let before = s.sim.clone();
    let e = s.request_build(Some(1), MapStateId(20), BuildingType::Factory).unwrap_err();
    assert!(matches!(e, BuildRequestError::Order(OrderError::InsufficientFunds { .. })), "{e:?}");
    assert_eq!(s.sim, before);
    // 確認後に同じ注文が入ると、重複として拒否される。
    s.sim.countries[0].treasury = 100_000.0;
    let ok = s.check_build(Some(1), MapStateId(20), BuildingType::Farm).unwrap();
    s.request_build(Some(1), MapStateId(20), BuildingType::Farm).unwrap();
    let again = s.request_build(Some(1), MapStateId(20), BuildingType::Farm).unwrap_err();
    assert_eq!(again, BuildRequestError::Order(OrderError::AlreadyQueued));
    assert_eq!(ok.target_level, 4, "the earlier check was about level 3 -> 4 (Farm: 3 at the start)");
    // 確認後に建物定義(費用)が変わると、実注文は現在の費用で支払う(確認時の費用を使わない)。
    let mut s = session("m4e");
    let ok = s.check_build(Some(1), MapStateId(20), BuildingType::Farm).unwrap();
    s.buildings.definitions.get_mut(&BuildingType::Farm).unwrap().construction_cost = ok.cost + 50.0;
    let t0 = s.country(1).unwrap().treasury;
    let id = s.request_build(Some(1), MapStateId(20), BuildingType::Farm).unwrap();
    assert_eq!(s.country(1).unwrap().treasury, t0 - (ok.cost + 50.0));
    assert_eq!(s.country(1).unwrap().construction_queue.last().unwrap().id, id);
}

#[test]
fn cancelling_is_by_order_id_and_refuses_other_countries_orders() {
    let mut s = session("m4e");
    let a = s.request_build(Some(1), MapStateId(20), BuildingType::Farm).unwrap();
    let b = s.request_build(Some(1), MapStateId(20), BuildingType::Mine).unwrap();
    let c = s.request_build(Some(2), MapStateId(21), BuildingType::Farm).unwrap();
    // 国家2は国家1の注文を取り消せない。
    let before = s.sim.clone();
    assert_eq!(
        s.cancel_order(Some(2), a).unwrap_err(),
        CancelRequestError::Cancel(CancelError::NotIssuersOrder(a))
    );
    assert_eq!(s.sim, before);
    // 先に作った注文(a)を取り消しても、キュー位置がずれた b・c は取り消されない。
    let receipt = s.cancel_order(Some(1), a).unwrap();
    assert_eq!(receipt.id, a);
    assert_eq!(s.country(1).unwrap().construction_queue.iter().map(|o| o.id).collect::<Vec<_>>(), vec![b]);
    assert_eq!(s.country(2).unwrap().construction_queue.iter().map(|o| o.id).collect::<Vec<_>>(), vec![c]);
    assert_eq!(
        s.cancel_order(Some(1), a).unwrap_err(),
        CancelRequestError::Cancel(CancelError::AlreadyCancelled(a))
    );
}

// ---------------------------------------------------------------- inspect

fn saved(s: &EconomySession, t: &Tmp, name: &str) {
    s.save(&geo().map, geo().model(), &t.loc(), name).unwrap();
}

#[test]
fn inspect_reports_a_valid_file_without_changing_the_state_or_the_files() {
    let t = Tmp::new("inspect");
    let mut src = session("m4e");
    src.request_build(Some(1), MapStateId(20), BuildingType::Farm).unwrap();
    for _ in 0..40 {
        src.advance_day();
    }
    saved(&src, &t, "a");
    let files_before = t.snapshot();
    let target = session("m4e");
    let sim_before = target.sim.clone();
    let r = target.inspect(&geo().map, geo().model(), &t.loc(), "a").unwrap();
    assert_eq!(r.kind, "map_economy_state");
    assert_eq!(r.format_version, 2);
    assert_eq!(r.path, t.file("a"));
    assert_eq!((r.date.year, r.date.month, r.date.day), (1800, 2, 10));
    assert_eq!((r.summary.states, r.summary.countries), (3, 3));
    assert!(r.verdict.is_exact() && r.verdict.matched.len() == ItemKey::ALL.len());
    assert_eq!(r.restorable, Some(Ok(())));
    assert_eq!(r.bytes, fs::read(t.file("a")).unwrap().len());
    assert_eq!(target.sim, sim_before, "inspect does not apply the file");
    assert_eq!(t.snapshot(), files_before, "inspect does not change the file or the directory");
}

#[test]
fn inspect_returns_warnings_and_rejections_with_reasons_and_never_applies_anything() {
    let t = Tmp::new("inspect_verdict");
    saved(&session("m4e"), &t, "a");
    let files = t.snapshot();
    // 警告つき許可(建物の表示名だけ違う)。
    let mut warn = session("m4e");
    warn.buildings.definitions.get_mut(&BuildingType::Farm).unwrap().name = "Farm X".to_string();
    let before = warn.sim.clone();
    let r = warn.inspect(&geo().map, geo().model(), &t.loc(), "a").unwrap();
    assert_eq!(r.verdict.warnings.iter().map(|w| w.key).collect::<Vec<_>>(), vec![ItemKey::BuildingPresentation]);
    assert!(r.verdict.is_compatible() && !r.verdict.is_exact());
    assert_eq!(r.restorable, Some(Ok(())));
    assert_eq!(warn.sim, before);
    // 互換性拒否(建設費が違う)。確認そのものは成功し、理由を返す。復元の見込みは評価しない。
    let mut reject = session("m4e");
    reject.buildings.definitions.get_mut(&BuildingType::Farm).unwrap().construction_cost += 1.0;
    let before = reject.sim.clone();
    let r = reject.inspect(&geo().map, geo().model(), &t.loc(), "a").unwrap();
    assert_eq!(r.verdict.rejections.iter().map(|w| w.key).collect::<Vec<_>>(), vec![ItemKey::BuildingComputation]);
    assert!(!r.verdict.is_compatible());
    assert_eq!(r.restorable, None);
    assert_eq!(reject.sim, before);
    // 別シナリオの保存(国家構造が違う)。
    let r = session("m4d").inspect(&geo().map, geo().model(), &t.loc(), "a").unwrap();
    assert!(!r.verdict.is_compatible());
    assert!(r.verdict.rejections.iter().any(|f| f.key == ItemKey::ScenarioStructure));
    assert_eq!(t.snapshot(), files, "files untouched");
}

#[test]
fn inspect_errors_for_unsupported_broken_or_foreign_files_without_touching_them() {
    let t = Tmp::new("inspect_errors");
    let s = session("m4e");
    saved(&s, &t, "good");
    let good = fs::read_to_string(t.file("good")).unwrap();
    let v1 = fs::read("saves/savegame_v1.ron").unwrap();
    t.write_raw("v1.mapeco.ron", &v1);
    t.write_raw("broken.mapeco.ron", b"(kind: ");
    t.write_raw("empty.mapeco.ron", b"");
    t.write_raw("other.mapeco.ron", good.replacen("kind: \"map_economy_state\"", "kind: \"other_kind\"", 1).as_bytes());
    t.write_raw("future.mapeco.ron", good.replacen("format_version: 2", "format_version: 9", 1).as_bytes());
    // 形式1(P32-016)は項目別の識別情報が無く、非対応。
    let snap = decode(&good).unwrap().snapshot;
    #[derive(serde::Serialize)]
    struct Format1<'a> {
        kind: &'a str,
        format_version: u32,
        snapshot: &'a strategy_game::economy::map_state::snapshot::MapEconomySnapshot,
    }
    let f1 = ron::ser::to_string_pretty(
        &Format1 { kind: "map_economy_state", format_version: 1, snapshot: &snap },
        ron::ser::PrettyConfig::default(),
    )
    .unwrap();
    t.write_raw("format1.mapeco.ron", f1.as_bytes());
    let files = t.snapshot();
    let before = s.sim.clone();
    let insp = |n: &str| s.inspect(&geo().map, geo().model(), &t.loc(), n).unwrap_err();
    assert!(matches!(insp("v1"), FileError::NotThisKind(DetectedKind::LegacyV1 { .. })));
    assert!(matches!(insp("broken"), FileError::Malformed(_)));
    assert!(matches!(insp("empty"), FileError::Malformed(_)));
    assert!(matches!(insp("other"), FileError::NotThisKind(DetectedKind::Other(k)) if k == "other_kind"));
    assert!(matches!(insp("future"), FileError::UnsupportedVersion { found: 9 }));
    assert!(matches!(insp("format1"), FileError::UnsupportedVersion { found: 1 }));
    assert!(matches!(insp("missing"), FileError::NotFound(_)));
    for bad in ["../x", "a/b", "CON", "", "a.b"] {
        assert!(matches!(insp(bad), FileError::InvalidName(_)), "{bad:?}");
    }
    assert_eq!(s.sim, before);
    assert_eq!(t.snapshot(), files);
    assert_eq!(fs::read("saves/savegame_v1.ron").unwrap(), v1, "the real V1 slot is untouched");
}

#[test]
fn inspect_does_not_create_the_save_directory() {
    let t = Tmp::new("inspect_nodir");
    let s = session("m4e");
    let e = s.inspect(&geo().map, geo().model(), &t.loc(), "a").unwrap_err();
    assert!(matches!(e, FileError::NotFound(_)), "{e}");
    assert!(!t.dir().exists(), "inspect never creates the directory");
}

#[test]
fn inspect_marks_a_compatible_but_invalid_snapshot_as_not_restorable_and_load_refuses_it() {
    let t = Tmp::new("inspect_invalid");
    let s = session("m4e");
    saved(&s, &t, "a");
    let f = read_file(&t.loc(), "a").unwrap();
    let mut snap = f.snapshot.clone();
    snap.countries[0].next_order_seq = 99; // 識別情報は一致するが、注文の連番の整合が取れない
    t.write_raw("bad.mapeco.ron", encode(&snap, &f.identity).unwrap().as_bytes());
    let r = s.inspect(&geo().map, geo().model(), &t.loc(), "bad").unwrap();
    assert!(r.verdict.is_exact());
    assert!(matches!(r.restorable, Some(Err(_))), "{:?}", r.restorable);
    let mut target = session("m4e");
    let before = target.sim.clone();
    let e = target.load(&geo().map, geo().model(), &t.loc(), "bad").unwrap_err();
    assert!(matches!(e, FileError::Restore(_)), "{e}");
    assert_eq!(target.sim, before);
}

#[test]
fn a_successful_inspection_does_not_exempt_the_later_load_from_full_validation() {
    let t = Tmp::new("inspect_then_load");
    let mut src = session("m4e");
    for _ in 0..20 {
        src.advance_day();
    }
    saved(&src, &t, "a");
    let mut target = session("m4e");
    let r = target.inspect(&geo().map, geo().model(), &t.loc(), "a").unwrap();
    assert!(r.verdict.is_exact() && r.restorable == Some(Ok(())));

    // 確認の後にファイルが別の内容(別の建物定義で保存したもの)に変わる → load は読み直して拒否する。
    let mut other = session("m4e");
    other.buildings.definitions.get_mut(&BuildingType::Farm).unwrap().construction_cost += 5.0;
    fs::write(t.file("a"), {
        let snapshot = other.sim.snapshot(geo().model(), &other.buildings);
        let id = other.identity(&geo().map, geo().model()).unwrap().to_saved();
        encode(&snapshot, &id).unwrap()
    })
    .unwrap();
    let before = target.sim.clone();
    let e = target.load(&geo().map, geo().model(), &t.loc(), "a").unwrap_err();
    assert!(matches!(&e, FileError::Incompatible(v) if v.rejections.iter().any(|f| f.key == ItemKey::BuildingComputation)), "{e}");
    assert_eq!(target.sim, before);

    // 確認の後にファイルが壊れる → load は拒否する。
    fs::write(t.file("a"), b"(kind: \"map_economy_state\"").unwrap();
    assert!(matches!(target.load(&geo().map, geo().model(), &t.loc(), "a").unwrap_err(), FileError::Malformed(_)));
    assert_eq!(target.sim, before);

    // 確認の後に復元先の静的入力が変わる → 元のファイルでも load は拒否する(古い確認結果は使わない)。
    saved(&src, &t, "b");
    let r = target.inspect(&geo().map, geo().model(), &t.loc(), "b").unwrap();
    assert!(r.verdict.is_compatible());
    target.buildings.definitions.get_mut(&BuildingType::Farm).unwrap().construction_cost += 1.0;
    let before = target.sim.clone();
    assert!(matches!(
        target.load(&geo().map, geo().model(), &t.loc(), "b").unwrap_err(),
        FileError::Incompatible(_)
    ));
    assert_eq!(target.sim, before);
    // 静的入力を戻せば、同じファイルが復元できる(確認結果の有無に依存しない)。
    target.buildings.definitions.get_mut(&BuildingType::Farm).unwrap().construction_cost -= 1.0;
    target.load(&geo().map, geo().model(), &t.loc(), "b").unwrap();
    assert_eq!(target.sim, src.sim);
}

// ---------------------------------------------------------------- 保存の一覧

#[test]
fn listing_without_a_save_directory_reports_no_saves_and_creates_nothing() {
    let t = Tmp::new("list_none");
    let l = EconomySession::list_saves(&t.loc());
    assert_eq!(l.directory, DirectoryStatus::Missing);
    assert!(l.entries.is_empty());
    assert!(!t.dir().exists(), "listing never creates the directory");
    // 保存先の親(base)自体が無くても「保存なし」。
    let l = EconomySession::list_saves(&SaveLocation::new(t.0.join("no_such_base")));
    assert_eq!(l.directory, DirectoryStatus::Missing);
    assert!(!t.0.join("no_such_base").exists());
}

#[test]
fn listing_returns_valid_saves_in_a_stable_order_and_distinguishes_unusable_candidates() {
    let t = Tmp::new("list_mixed");
    let mut s = session("m4e");
    // 作成順をわざと名前順と変える。
    saved(&s, &t, "charlie");
    for _ in 0..10 {
        s.advance_day();
    }
    saved(&s, &t, "alpha");
    saved(&s, &t, "bravo");
    let good = fs::read_to_string(t.file("alpha")).unwrap();
    let v1 = fs::read("saves/savegame_v1.ron").unwrap();
    t.write_raw("broken.mapeco.ron", b"(kind: ");
    t.write_raw("legacy_copy.mapeco.ron", &v1);
    t.write_raw("other_kind.mapeco.ron", good.replacen("kind: \"map_economy_state\"", "kind: \"x\"", 1).as_bytes());
    t.write_raw("future.mapeco.ron", good.replacen("format_version: 2", "format_version: 7", 1).as_bytes());
    t.write_raw("bad name.mapeco.ron", good.as_bytes());
    fs::create_dir_all(t.dir().join("a_directory.mapeco.ron")).unwrap();
    // 無関係なファイル・一時ファイル・旧V1のスロット名のファイル(いずれも一覧に出さず、触れない)。
    t.write_raw("notes.txt", b"hello");
    t.write_raw("alpha.mapeco.ron.123-1-1.tmp", b"partial");
    t.write_raw("savegame_v1.ron", &v1);
    let before = t.snapshot();

    let l = EconomySession::list_saves(&t.loc());
    assert_eq!(l.directory, DirectoryStatus::Readable);
    let names: Vec<&str> = l.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["a_directory", "alpha", "bad name", "bravo", "broken", "charlie", "future", "legacy_copy", "other_kind"],
        "sorted by name; unrelated files and temp files are not listed"
    );
    let by = |n: &str| l.entries.iter().find(|e| e.name == n).unwrap();
    for n in ["alpha", "bravo", "charlie"] {
        assert!(matches!(by(n).status, EntryStatus::Valid { .. }), "{n}");
        assert_eq!(by(n).path, t.file(n));
    }
    let EntryStatus::Valid { date, bytes } = &by("alpha").status else { unreachable!() };
    assert_eq!((date.year, date.month, date.day), (1800, 1, 11));
    assert_eq!(*bytes, good.len());
    let EntryStatus::Valid { date, .. } = &by("charlie").status else { unreachable!() };
    assert_eq!((date.month, date.day), (1, 1), "charlie was saved before any day was processed");
    assert!(matches!(&by("broken").status, EntryStatus::Unusable(FileError::Malformed(_))));
    assert!(matches!(&by("legacy_copy").status, EntryStatus::Unusable(FileError::NotThisKind(DetectedKind::LegacyV1 { .. }))));
    assert!(matches!(&by("other_kind").status, EntryStatus::Unusable(FileError::NotThisKind(DetectedKind::Other(_)))));
    assert!(matches!(&by("future").status, EntryStatus::Unusable(FileError::UnsupportedVersion { found: 7 })));
    assert!(matches!(&by("bad name").status, EntryStatus::Unusable(FileError::InvalidName(_))));
    assert!(matches!(&by("a_directory").status, EntryStatus::Unusable(FileError::LocationRefused(_))));
    // 一覧は何も変えない(修復・変換・削除・作成なし)。
    assert_eq!(t.snapshot().len(), before.len());
    assert_eq!(t.snapshot(), before);
    assert!(t.dir().join("alpha.mapeco.ron.123-1-1.tmp").exists(), "temp files are not removed by listing");
    // 同じ入力なら同じ順序(列挙順に依存しない)。
    assert_eq!(EconomySession::list_saves(&t.loc()), l);
}

#[test]
fn a_directory_that_is_not_a_usable_save_location_is_reported_separately_from_candidates() {
    let t = Tmp::new("list_refused");
    // `map_economy` が通常ファイル。
    fs::write(t.dir(), b"not a directory").unwrap();
    let l = EconomySession::list_saves(&t.loc());
    assert!(matches!(l.directory, DirectoryStatus::Refused(FileError::LocationRefused(_))), "{:?}", l.directory);
    assert!(l.entries.is_empty());
    assert_eq!(fs::read(t.dir()).unwrap(), b"not a directory");
}

#[test]
fn a_junction_save_directory_pointing_elsewhere_is_refused_and_the_outside_is_not_read() {
    if !cfg!(windows) {
        return;
    }
    let t = Tmp::new("list_junction");
    let outside = t.0.join("outside");
    fs::create_dir_all(&outside).unwrap();
    // 保存先の外にある、一覧に出てはいけない正常な保存。
    let s = session("m4e");
    s.save(&geo().map, geo().model(), &SaveLocation::new(&outside), "secret").unwrap();
    let base = t.0.join("base");
    fs::create_dir_all(&base).unwrap();
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(base.join("map_economy"))
        .arg(outside.join("map_economy"))
        .output();
    if !status.map(|o| o.status.success()).unwrap_or(false) {
        eprintln!("SKIPPED: could not create a junction in this environment");
        return;
    }
    let l = EconomySession::list_saves(&SaveLocation::new(&base));
    assert!(matches!(l.directory, DirectoryStatus::Refused(_)), "{:?}", l.directory);
    assert!(l.entries.is_empty(), "files outside the save location are never listed");
    assert!(outside.join("map_economy").join("secret.mapeco.ron").exists());
}

// ---------------------------------------------------------------- CLI(別プロセス)

fn cli(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_world_economy_check")).args(args).output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn day_lines(stdout: &str) -> Vec<String> {
    stdout.lines().filter(|l| l.starts_with("day ")).map(str::to_string).collect()
}

#[test]
fn the_cli_keeps_its_behaviour_after_the_shared_assembly_and_exposes_list_and_inspect() {
    let t = Tmp::new("cli");
    let base = t.0.to_string_lossy().to_string();
    // 保存先がまだ無い: 一覧は「保存なし」で、何も作らない。
    let (ok, out, err) = cli(&["--economy-list", "--save-base", &base]);
    assert!(ok && out.contains("保存なし"), "{out}{err}");
    assert!(!t.dir().exists());
    // 従来どおり(既定 m4d): 100日で保存 → 別プロセスで60日再開 = 連続実行の101〜160日。
    let (ok, out, err) = cli(&["--economy-save", "100", "--save-base", &base, "--name", "cli"]);
    assert!(ok && out.contains("保存成功") && out.contains("1800/04/11"), "{out}{err}");
    let (ok, resumed, err) = cli(&["--economy-resume", "60", "--save-base", &base, "--name", "cli"]);
    assert!(ok && resumed.contains("復元成功") && resumed.contains("一致 21 項目"), "{resumed}{err}");
    let (ok, cont, err) = cli(&["--economy-run", "160", "--digest-from", "100"]);
    assert!(ok, "{cont}{err}");
    let (a, b) = (day_lines(&resumed), day_lines(&cont));
    assert_eq!(a.len(), 60);
    assert_eq!(a, b);
    // 共通化前(P32-017 の記録)と同じ値。
    assert!(a[0].starts_with("day 1800/04/12 digest=0efefcfcfc398595 "), "{}", a[0]);
    assert!(a[59].starts_with("day 1800/06/10 digest=7f6f2d0ddbd2cc19 "), "{}", a[59]);
    // 明示した `--scenario m4d` は既定と同じ。
    let (ok, explicit, err) = cli(&["--economy-run", "160", "--digest-from", "100", "--scenario", "m4d"]);
    assert!(ok, "{err}");
    assert_eq!(day_lines(&explicit), a);
    // m4e は明示的に選べて、m4d とは別の結果になる。m4d の保存は m4e の入力へは復元できない。
    let (ok, m4e, err) = cli(&["--economy-run", "160", "--digest-from", "100", "--scenario", "m4e"]);
    assert!(ok && day_lines(&m4e).len() == 60, "{err}");
    assert_ne!(day_lines(&m4e), a);
    let (ok, out, err) = cli(&["--economy-resume", "5", "--save-base", &base, "--name", "cli", "--scenario", "m4e"]);
    assert!(!ok && out.contains("拒否") && !out.contains("復元成功") && day_lines(&out).is_empty(), "{out}{err}");
    let (ok, _, err) = cli(&["--economy-run", "1", "--scenario", "nope"]);
    assert!(!ok && err.contains("m4d") && err.contains("m4e"), "{err}");

    // 一覧と事前確認(どちらもファイルを変えない)。
    let before = fs::read(t.file("cli")).unwrap();
    let (ok, out, err) = cli(&["--economy-list", "--save-base", &base]);
    assert!(ok && out.contains("保存 1 件") && out.contains("cli : 解析できる保存"), "{out}{err}");
    let (ok, out, err) = cli(&["--economy-inspect", "--save-base", &base, "--name", "cli"]);
    assert!(ok && out.contains("kind map_economy_state / 形式 2") && out.contains("一致 21 項目") && out.contains("復元できる見込み"), "{out}{err}");
    assert!(out.contains("1800/04/11"), "{out}");
    assert_eq!(fs::read(t.file("cli")).unwrap(), before);
    let (ok, out, _) = cli(&["--economy-inspect", "--save-base", &base, "--name", "cli", "--scenario", "m4e"]);
    assert!(ok && out.contains("拒否") && out.contains("復元できません"), "{out}");
    let (ok, _, err) = cli(&["--economy-inspect", "--save-base", &base, "--name", "missing"]);
    assert!(!ok && err.contains("確認失敗"), "{err}");
    // 一覧に壊れたファイルが混ざっていても、正常な保存として表示されない。
    t.write_raw("broken.mapeco.ron", b"(");
    let (ok, out, _) = cli(&["--economy-list", "--save-base", &base]);
    assert!(ok && out.contains("broken : 使えません"), "{out}");
}
