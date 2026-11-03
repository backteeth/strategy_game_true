//! P32-026: 保存・復元の工程別計時(`economy::phase_timing`、`map_economy_file::{save_timed, load_timed}`)の自動検証。
//! 小さい入力(検証用ワールドの m4e・小さい合成ケース)だけを使う。大規模測定は `economy_save_timing` を手動で実行する。
//! 時間の値(何ミリ秒以下か、必ず非ゼロか)は合否条件にしない。保存先はテストごとのOSの一時ディレクトリで、`saves/` へは書かない。

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use strategy_game::building::data::BuildingRegistry;
use strategy_game::economy::map_economy_file::{FileError, SaveLocation, load, load_timed, save, save_timed};
use strategy_game::economy::map_session::{EconomySession, GeographyData, SessionPaths, load_buildings, scenario_by_id};
use strategy_game::economy::map_state::static_identity::StaticInputs;
use strategy_game::economy::phase_timing::{Outcome, Timing};
use strategy_game::economy::save_timing_bench::{BenchSpec, run_bench_case};
use strategy_game::economy::scale_harness::{CaseStatus, ScaleSpec, fresh_temp_base};
use strategy_game::hexmap::data::{HexMap, load_from_path};
use strategy_game::world::LocationFacilityRegistry;

fn geo() -> &'static GeographyData {
    static G: OnceLock<GeographyData> = OnceLock::new();
    G.get_or_init(|| GeographyData::load(&SessionPaths::default()).unwrap())
}

fn session() -> EconomySession {
    EconomySession::load_files(scenario_by_id("m4e").unwrap(), geo().model(), &SessionPaths::default()).unwrap()
}

/// 45日進めた状態(建設注文が進行中)。
fn advanced() -> EconomySession {
    let mut s = session();
    for _ in 0..45 {
        s.advance_day();
    }
    s
}

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!("p32_026_{tag}_{}_{nanos}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn loc(&self) -> SaveLocation {
        SaveLocation::new(&self.0)
    }
    fn dir(&self) -> PathBuf {
        self.0.join("map_economy")
    }
    fn files(&self) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = fs::read_dir(self.dir())
            .map(|d| d.map(|e| e.unwrap()).map(|e| (e.file_name().to_string_lossy().to_string(), fs::read(e.path()).unwrap())).collect())
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

fn inputs<'a>(s: &'a EconomySession) -> StaticInputs<'a> {
    s.inputs(&geo().map, geo().model())
}

/// 状態(`sim`)を可変に借りたまま使える静的入力(セッションの建物・施設の欄だけを借りる)。
fn parts<'a>(buildings: &'a BuildingRegistry, facilities: &'a Option<LocationFacilityRegistry>) -> StaticInputs<'a> {
    StaticInputs { map: &geo().map, world: geo().model(), buildings, facilities: facilities.as_ref() }
}

const SAVE_SPANS: [&str; 12] = [
    "save.snapshot",
    "save.identity",
    "save.encode",
    "save.verify_decode_header",
    "save.verify_decode_body",
    "save.verify_compare",
    "save.target",
    "save.existing_check",
    "save.temp_create",
    "save.write",
    "save.sync",
    "save.replace",
];

const LOAD_SPANS: [&str; 11] = [
    "load.target",
    "load.read",
    "load.decode_header",
    "load.decode_body",
    "load.identity",
    "load.judge",
    "restore.validate",
    "restore.build_candidate",
    "restore.roundtrip_snapshot",
    "restore.roundtrip_compare",
    "restore.roundtrip_drop",
];

fn names(t: &Timing) -> Vec<&'static str> {
    t.spans().iter().map(|s| s.name).collect()
}

// ---------------------------------------------------------------- 計時の入れ物

#[test]
fn a_disabled_timing_records_nothing_and_just_runs_the_work() {
    let mut t = Timing::off();
    assert!(!t.is_on());
    let v = t.run("anything", || 41 + 1);
    assert_eq!(v, 42);
    assert!(t.spans().is_empty());
    assert_eq!(t.outcome(), Outcome::Unset);
    assert_eq!(t.total().as_nanos(), 0);
}

#[test]
fn an_enabled_timing_merges_calls_by_name_in_first_use_order() {
    let mut t = Timing::on();
    t.run("b", || ());
    t.run("a", || ());
    t.run("b", || ());
    assert_eq!(names(&t), ["b", "a"]);
    assert_eq!(t.span("b").unwrap().calls, 2);
    assert_eq!(t.span("a").unwrap().calls, 1);
    // 親子がないので、覆った時間は工程の合計そのもの。覆っていない時間 = 全体 - 覆った時間(負にならない)
    assert_eq!(t.covered(), t.spans().iter().map(|s| s.total).sum());
    assert_eq!(t.uncovered(), t.total().saturating_sub(t.covered()));
}

// ---------------------------------------------------------------- 通常経路と計時経路の一致(保存)

#[test]
fn a_timed_save_writes_the_same_bytes_as_the_normal_save_and_records_every_stage() {
    let s = advanced();
    let (a, b) = (Tmp::new("save_a"), Tmp::new("save_b"));
    let ra = save(&s.sim, &inputs(&s), &a.loc(), "x").unwrap();
    let mut t = Timing::on();
    let rb = save_timed(&s.sim, &inputs(&s), &b.loc(), "x", &mut t).unwrap();
    // 保存データ(バイト列)・受領書(パス以外)が同じ
    assert_eq!(a.files(), b.files());
    assert_eq!((ra.bytes, ra.date, ra.replaced_existing), (rb.bytes, rb.date, rb.replaced_existing));
    // 主要工程が記録され、成功として記録される
    let got: BTreeSet<_> = names(&t).into_iter().collect();
    assert_eq!(got, SAVE_SPANS.into_iter().collect::<BTreeSet<_>>());
    assert!(t.spans().iter().all(|s| s.calls == 1), "each save stage ran once");
    assert_eq!(t.outcome(), Outcome::Success);
    // 計時情報はファイルに入らない
    let text = String::from_utf8(b.files()[0].1.clone()).unwrap();
    for marker in ["save.snapshot", "restore.", "load.", "uncovered", "spans", "timing"] {
        assert!(!text.contains(marker), "the save file must not contain timing information ({marker})");
    }
    // 状態は変えない
    let before = advanced();
    assert!(s.sim == before.sim);
}

#[test]
fn saving_again_over_an_existing_file_is_timed_with_the_existing_file_check() {
    let s = advanced();
    let tmp = Tmp::new("replace");
    save(&s.sim, &inputs(&s), &tmp.loc(), "x").unwrap();
    let first = tmp.files();
    let mut t = Timing::on();
    let r = save_timed(&s.sim, &inputs(&s), &tmp.loc(), "x", &mut t).unwrap();
    assert!(r.replaced_existing);
    assert_eq!(tmp.files(), first, "same state, same bytes");
    assert_eq!(t.outcome(), Outcome::Success);
}

// ---------------------------------------------------------------- 通常経路と計時経路の一致(復元・継続)

#[test]
fn a_timed_restore_gives_the_same_state_and_verdict_as_the_normal_restore_and_continues_identically() {
    let s = advanced();
    let tmp = Tmp::new("restore");
    save(&s.sim, &inputs(&s), &tmp.loc(), "x").unwrap();
    let files_before = tmp.files();

    let mut normal = session();
    let rn = load(&mut normal.sim, &parts(&normal.buildings, &normal.facilities), &tmp.loc(), "x").unwrap();
    let mut timed = session();
    let mut t = Timing::on();
    let rt = load_timed(&mut timed.sim, &parts(&timed.buildings, &timed.facilities), &tmp.loc(), "x", &mut t).unwrap();

    assert!(normal.sim == timed.sim, "restored states are identical");
    assert!(normal.sim == s.sim, "and equal to the state at the time of saving");
    assert_eq!((rn.date, &rn.verdict), (rt.date, &rt.verdict), "same date, same verdict (warnings/rejections)");
    let got: BTreeSet<_> = names(&t).into_iter().collect();
    let mut want: BTreeSet<_> = LOAD_SPANS.into_iter().collect();
    want.insert("restore.apply");
    assert_eq!(got, want);
    assert_eq!(t.outcome(), Outcome::Success);
    assert_eq!(tmp.files(), files_before, "loading does not change the file");

    // 復元後の継続: 各日の状態が一致
    let mut reference = advanced();
    for day in 0..60 {
        reference.advance_day();
        normal.advance_day();
        timed.advance_day();
        assert!(reference.sim == normal.sim && normal.sim == timed.sim, "day {day} after the restore");
    }
}

// ---------------------------------------------------------------- 拒否・エラーを成功として記録しない

#[test]
fn a_rejected_restore_is_recorded_as_rejected_and_changes_nothing() {
    // 更新前の地図で作った保存(P32-025 の固定fixture)は、現在の地図では既存の方針どおり拒否される。
    let tmp = Tmp::new("rejected");
    fs::create_dir_all(tmp.dir()).unwrap();
    let bytes = fs::read("tests/fixtures/p32_025/old_map_m4e.mapeco.ron").unwrap();
    fs::write(tmp.dir().join("old.mapeco.ron"), &bytes).unwrap();

    let mut a = advanced();
    let mut b = advanced();
    let before = a.sim.clone();
    let ea = load(&mut a.sim, &parts(&a.buildings, &a.facilities), &tmp.loc(), "old").unwrap_err();
    let mut t = Timing::on();
    let eb = load_timed(&mut b.sim, &parts(&b.buildings, &b.facilities), &tmp.loc(), "old", &mut t).unwrap_err();
    assert!(matches!(ea, FileError::Incompatible(_)));
    assert_eq!(ea, eb, "the same rejection with the same reasons");
    assert_eq!(t.outcome(), Outcome::Rejected);
    assert!(a.sim == before && b.sim == before, "the current state is unchanged");
    assert_eq!(fs::read(tmp.dir().join("old.mapeco.ron")).unwrap(), bytes, "the file is unchanged");
    // 判定までは計るが、復元の工程には入らない
    assert!(t.span("load.judge").is_some());
    assert!(t.span("restore.validate").is_none() && t.span("restore.apply").is_none());
}

#[test]
fn errors_are_recorded_as_errors_not_as_success() {
    let s = advanced();
    let tmp = Tmp::new("errors");
    // 読み込めない(存在しない)
    let mut t = Timing::on();
    let mut sim = session();
    let before = sim.sim.clone();
    let e = load_timed(&mut sim.sim, &parts(&sim.buildings, &sim.facilities), &tmp.loc(), "missing", &mut t).unwrap_err();
    assert!(matches!(e, FileError::NotFound(_) | FileError::Io { .. }), "{e:?}");
    assert_eq!(t.outcome(), Outcome::Error);
    assert!(sim.sim == before);
    // 不正な名前(保存・復元とも)
    let mut t = Timing::on();
    assert!(matches!(save_timed(&s.sim, &inputs(&s), &tmp.loc(), "../bad", &mut t), Err(FileError::InvalidName(_))));
    assert_eq!(t.outcome(), Outcome::Error);
    assert!(!tmp.dir().exists(), "nothing was created");
    let mut t = Timing::on();
    assert!(matches!(load_timed(&mut sim.sim, &parts(&sim.buildings, &sim.facilities), &tmp.loc(), "a/b", &mut t), Err(FileError::InvalidName(_))));
    assert_eq!(t.outcome(), Outcome::Error);
    // 壊れたファイル
    fs::create_dir_all(tmp.dir()).unwrap();
    fs::write(tmp.dir().join("broken.mapeco.ron"), b"(not a save").unwrap();
    let mut t = Timing::on();
    assert!(matches!(load_timed(&mut sim.sim, &parts(&sim.buildings, &sim.facilities), &tmp.loc(), "broken", &mut t), Err(FileError::Malformed(_))));
    assert_eq!(t.outcome(), Outcome::Error);
    assert!(sim.sim == before);
}

// ---------------------------------------------------------------- 通常経路は計測物を作らない

#[test]
fn the_normal_save_and_load_create_only_the_save_file() {
    let s = advanced();
    let tmp = Tmp::new("normal");
    save(&s.sim, &inputs(&s), &tmp.loc(), "x").unwrap();
    let mut r = session();
    load(&mut r.sim, &parts(&r.buildings, &r.facilities), &tmp.loc(), "x").unwrap();
    let names: Vec<_> = tmp.files().into_iter().map(|f| f.0).collect();
    assert_eq!(names, ["x.mapeco.ron"], "no temporary, log or measurement file remains");
    assert_eq!(fs::read_dir(&tmp.0).unwrap().count(), 1, "only the map_economy directory under the base");
    // 通常のセッションAPIも同じ経路(計時なし)
    let tmp2 = Tmp::new("session_api");
    s.save(&geo().map, geo().model(), &tmp2.loc(), "x").unwrap();
    assert_eq!(tmp2.files(), tmp.files());
}

#[test]
fn timing_code_has_no_file_output_and_is_not_an_input_to_the_economy() {
    let timing = fs::read_to_string("src/economy/phase_timing.rs").unwrap();
    for banned in ["fs::", "File::", "println!", "eprintln!", "write!("] {
        assert!(!timing.contains(banned), "phase_timing must not output anything ({banned})");
    }
    // 経済の計算(日次処理・状態の型)から計時を参照しない
    for f in ["src/economy/map_state.rs", "src/economy/map_state/snapshot.rs"] {
        let src = fs::read_to_string(f).unwrap();
        // 復元の共通本体が受け取る以外(`restore_*_timed`)に、計時の使用がない
        for (i, line) in src.lines().enumerate() {
            if line.contains("Timing") || line.contains("timing") {
                let ok = line.contains("timing: &mut Timing")
                    || line.contains("Timing::off()")
                    || line.contains("buildings, timing)")
                    || line.contains("timing.run(")
                    || line.contains("use crate::economy::phase_timing::Timing")
                    || line.trim_start().starts_with("//");
                assert!(ok, "{f}:{}: unexpected use of timing: {line}", i + 1);
            }
        }
    }
}

// ---------------------------------------------------------------- 小さい合成ケースの測定手順(計時なし/あり)

fn map() -> &'static HexMap {
    static M: OnceLock<HexMap> = OnceLock::new();
    M.get_or_init(|| load_from_path("assets/data/hex_atlas_map.json").unwrap())
}

fn buildings() -> &'static BuildingRegistry {
    static B: OnceLock<BuildingRegistry> = OnceLock::new();
    B.get_or_init(|| load_buildings("assets/data/buildings.ron").unwrap())
}

fn bench(timed: bool) -> strategy_game::economy::save_timing_bench::BenchResult {
    let mut scale = ScaleSpec::new(12, 3);
    scale.days = 90;
    scale.split_day = 45;
    let spec = BenchSpec { scale, timed, trials: 2 };
    let base = fresh_temp_base("p26_test");
    let r = run_bench_case(map(), buildings(), &spec, &base);
    let _ = fs::remove_dir(base.join("map_economy"));
    let _ = fs::remove_dir(&base);
    r
}

#[test]
fn the_bench_gives_identical_results_with_and_without_timing() {
    let off = bench(false);
    let on = bench(true);
    assert_eq!(off.status, CaseStatus::Success, "{:?}", off.status);
    assert_eq!(on.status, CaseStatus::Success, "{:?}", on.status);
    assert_eq!((off.states, off.countries), (12, 3));
    // 同じ入力・同じ状態: 保存ファイル・復元・継続(各日のダイジェスト)が計時の有無で一致
    assert_eq!(off.trials.len(), 2);
    assert_eq!(on.trials.len(), 2);
    for (a, b) in off.trials.iter().zip(&on.trials) {
        assert_eq!(a.file_blake3, b.file_blake3, "same saved bytes");
        assert_eq!((a.file_bytes, a.verdict_exact, a.verdict_warnings), (b.file_bytes, b.verdict_exact, b.verdict_warnings));
        assert!(a.restored_equals_state_at_save && b.restored_equals_state_at_save);
        assert_eq!(a.restored_date, b.restored_date);
    }
    assert!(off.file_hashes_equal_across_trials && on.file_hashes_equal_across_trials);
    assert_eq!(off.digest_chain, on.digest_chain, "the continued 45 days are identical day by day");
    assert_eq!((off.compared_days, off.digest_mismatch_days.len(), off.final_states_equal), (45, 0, true));
    assert_eq!((on.compared_days, on.digest_mismatch_days.len(), on.final_states_equal), (45, 0, true));
    assert!(off.queued_orders_at_save > 0, "orders were in progress at save time");
    // 計時なしは工程を記録せず、計時ありは保存・復元の主要工程を記録する
    assert!(off.trials.iter().all(|t| t.save.timing.is_none() && t.restore.timing.is_none()));
    for t in &on.trials {
        let (s, r) = (t.save.timing.as_ref().unwrap(), t.restore.timing.as_ref().unwrap());
        assert_eq!((s.outcome(), r.outcome()), (Outcome::Success, Outcome::Success));
        assert!(s.span("save.encode").is_some() && s.span("save.verify_decode_body").is_some());
        assert!(r.span("load.decode_body").is_some() && r.span("restore.build_candidate").is_some());
    }
    // 記録の出力に、すべての工程と結果が出る
    let text = on.render();
    for k in ["SPAN", "TOTAL", "outcome=Success", "name=save.encode", "name=restore.validate", "CONTINUITY"] {
        assert!(text.contains(k), "render lacks {k}");
    }
    assert!(!off.render().contains("SPAN"));
}
