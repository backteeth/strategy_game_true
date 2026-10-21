//! P32-015: 新ステート経路の実行時状態の、メモリ上のスナップショット取得と検証付き復元のテスト。
//! 比較は**新経路自身の保存前後**(旧経路との比較ではない)。復元は、同じ静的入力から作った
//! **別の(初期状態の)集約**へ行い、スナップショットをcloneして比較するだけにはしない。
//! 入力は検証用シナリオで、正式な配置・バランスではない。ファイルI/Oは行わない。

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use strategy_game::economy::map_state::snapshot::{
    MapEconomySimulation, MapEconomySnapshot, RestoreIssue,
};
use strategy_game::economy::map_state::{
    CancelError, CancelTarget, ConstructionOrderId, DayReport, MapStateEconomyRegistry,
    MultiMonthScenario, SimDate, parse_multi_month_scenario,
};
use strategy_game::economy::factory_starvation::FactoryStarvationSample;
use strategy_game::economy::iron_shortage::IronShortageSample;
use strategy_game::economy::province_resources::{
    ProvinceResourcesFile, derive_state_resources, load_province_resources,
};
use strategy_game::military::supply_production_history::MonthlyProductionSample;
use strategy_game::world::{MapStateId, WorldModel, load_province_table, load_state_layout};

use BuildingType::{Farm, Mine};

const START: SimDate = SimDate { year: 1800, month: 1, day: 1 };
const P: &str = "assets/data/world/preview/";

fn buildings() -> BuildingRegistry {
    let defs: Vec<BuildingDefinition> =
        ron::from_str(&std::fs::read_to_string("assets/data/buildings.ron").unwrap()).unwrap();
    BuildingRegistry { definitions: defs.into_iter().map(|d| (d.building_type, d)).collect() }
}

fn world() -> WorldModel {
    WorldModel::build(
        load_province_table(&format!("{P}provinces.ron")).unwrap(),
        load_state_layout(&format!("{P}states.ron")).unwrap(),
    )
    .unwrap()
}

fn scenario(name: &str) -> MultiMonthScenario {
    parse_multi_month_scenario(&std::fs::read_to_string(format!("{P}{name}")).unwrap()).unwrap()
}

fn resources(omit: &[u32]) -> ProvinceResourcesFile {
    let mut f = load_province_resources(&format!("{P}province_resources_m4d.ron")).unwrap();
    f.provinces.retain(|e| !omit.contains(&e.province));
    f
}

/// シナリオと資源定義から、開始日(まだ1日も処理していない)の集約を作る。
fn make(sc: &MultiMonthScenario, w: &WorldModel, b: &BuildingRegistry, omit: &[u32]) -> MapEconomySimulation {
    let (mut reg, cs) = MapStateEconomyRegistry::from_multi_scenario(sc, w, b).unwrap();
    // 直接指定の鉱床を持つ旧来の検証用シナリオ(m4b等)は、プロヴィンス別の資源定義を接続しない。
    if sc.states.iter().all(|s| s.resource_deposits.is_empty()) {
        let f = resources(omit);
        let d = derive_state_resources(w, &f).unwrap();
        reg.attach_province_resources(&d, w, &f).unwrap();
    }
    MapEconomySimulation::new(START, reg, cs)
}

fn s(id: u32) -> MapStateId {
    MapStateId(id)
}

struct Trace {
    /// `snaps[k]` = k日処理を完了した直後(`snaps[0]`は開始時)。
    snaps: Vec<MapEconomySnapshot>,
    /// `reports[k]` = (k+1)日目の処理の結果。
    reports: Vec<(SimDate, DayReport)>,
    last: MapEconomySimulation,
}

fn run_trace(mut sim: MapEconomySimulation, w: &WorldModel, b: &BuildingRegistry, days: usize) -> Trace {
    let mut snaps = vec![sim.snapshot(w, b)];
    let mut reports = Vec::new();
    for _ in 0..days {
        let r = sim.advance_day(b);
        reports.push(r);
        snaps.push(sim.snapshot(w, b));
    }
    Trace { snaps, reports, last: sim }
}

/// k日目の完了直後のスナップショットを、**別の初期状態の集約**へ復元して再開し、連続実行と
/// 各日の結果(出来事・AIの判断・状態)が一致することを確認する。
fn assert_resume_matches(
    fresh: impl Fn() -> MapEconomySimulation,
    t: &Trace,
    k: usize,
    w: &WorldModel,
    b: &BuildingRegistry,
) {
    let mut r = fresh();
    r.restore(&t.snaps[k], w, b).unwrap_or_else(|e| panic!("restore at day {k}: {e}"));
    assert_eq!(r.snapshot(w, b), t.snaps[k], "snapshot after restore at day {k}");
    let days = t.reports.len();
    for d in k + 1..=days {
        let (date, report) = r.advance_day(b);
        assert_eq!((date, &report), (t.reports[d - 1].0, &t.reports[d - 1].1), "report on day {d} (resumed at {k})");
        assert_eq!(r.snapshot(w, b), t.snaps[d], "state after day {d} (resumed at {k})");
    }
    assert_eq!(r, t.last, "final state (resumed at {k})");
}

// ---------------------------------------------------------------- 往復

#[test]
fn round_trip_restores_every_saved_value_into_an_independent_candidate() {
    let (w, b) = (world(), buildings());
    for name in ["economy_scenario_m4b.ron", "economy_scenario_m4c.ron", "economy_scenario_m4d.ron", "economy_scenario_m4e.ron"] {
        let sc = scenario(name);
        for days in [0usize, 1, 45, 150] {
            let t = run_trace(make(&sc, &w, &b, &[]), &w, &b, days);
            // 復元先は、同じ静的入力から作った**初期状態の別の集約**(動的な値は進んだ側と異なる)。
            let target = make(&sc, &w, &b, &[]);
            let cand = target.restore_candidate(&t.snaps[days], &w, &b).unwrap();
            assert_eq!(cand, t.last, "{name} day {days}: restored state equals the original");
            assert_eq!(cand.snapshot(&w, &b), t.snaps[days], "{name} day {days}: re-capture equals");
            if days > 0 {
                assert_ne!(target, t.last, "the target must differ so the restore is meaningful");
            }
        }
    }
}

#[test]
fn snapshot_is_deterministic_and_serializes_without_hash_maps() {
    let (w, b) = (world(), buildings());
    let t = run_trace(make(&scenario("economy_scenario_m4d.ron"), &w, &b, &[]), &w, &b, 120);
    let a = ron::to_string(&t.snaps[120]).unwrap();
    assert_eq!(a, ron::to_string(&t.snaps[120].clone()).unwrap());
    let back: MapEconomySnapshot = ron::from_str(&a).unwrap();
    assert_eq!(back, t.snaps[120], "in-memory text round trip (no file) reproduces the snapshot exactly");
}

// ---------------------------------------------------------------- 途中再開

#[test]
fn resuming_from_every_day_boundary_matches_the_continuous_run_for_m4d() {
    let (w, b) = (world(), buildings());
    let sc = scenario("economy_scenario_m4d.ron");
    let t = run_trace(make(&sc, &w, &b, &[]), &w, &b, 300);
    // 再開点の性質(月次の前後・完成の前後・AI初回評価前・履歴)が実際に含まれていることを確認する。
    let monthly: Vec<usize> = (0..300).filter(|d| t.reports[*d].1.monthly_ran).collect();
    let completed: Vec<usize> = (0..300).filter(|d| !t.reports[*d].1.completed.is_empty()).collect();
    assert!(monthly.len() >= 9 && !completed.is_empty(), "monthly {monthly:?} completed {completed:?}");
    assert!(t.snaps[0].countries.iter().all(|c| c.ai.as_ref().is_some_and(|a| a.dirty)), "day 0: AI first evaluation pending");
    assert!(t.snaps[300].countries.iter().any(|c| c.ai.as_ref().is_some_and(|a| !a.iron.is_empty())));
    for k in 0..=300 {
        assert_resume_matches(|| make(&sc, &w, &b, &[]), &t, k, &w, &b);
    }
}

/// 手動注文・取消し・AIが混在するシナリオ(m4e)。注文を出した後の各日から再開できる。
fn m4e_with_manual_orders(w: &WorldModel, b: &BuildingRegistry) -> (MultiMonthScenario, MapEconomySimulation) {
    let sc = scenario("economy_scenario_m4e.ron");
    let sim = make(&sc, w, b, &[]);
    (sc, sim)
}

fn order_all(sim: &mut MapEconomySimulation, b: &BuildingRegistry) {
    let r = &sim.registry;
    r.request_build(&mut sim.countries, 1, s(20), Farm, b).unwrap();
    r.request_build(&mut sim.countries, 1, s(20), Mine, b).unwrap();
    r.request_build(&mut sim.countries, 3, s(22), BuildingType::Factory, b).unwrap();
    r.request_build(&mut sim.countries, 2, s(21), Mine, b).unwrap();
}

#[test]
fn resume_points_with_manual_orders_cancel_and_completion_match_the_continuous_run() {
    let (w, b) = (world(), buildings());
    let (sc, mut sim) = m4e_with_manual_orders(&w, &b);
    order_all(&mut sim, &b);
    let mut snaps = vec![sim.snapshot(&w, &b)];
    let mut reports = Vec::new();
    for day in 1..=90usize {
        if day == 11 {
            // 建設途中(10日目)の取消し。
            let rc = sim.registry.cancel_order(&mut sim.countries, 1, CancelTarget::Order(ConstructionOrderId { country: 1, seq: 2 })).unwrap();
            assert_eq!(rc.refund, 250.0);
            sim.registry.request_build(&mut sim.countries, 1, s(20), Mine, &b).unwrap();
        }
        reports.push(sim.advance_day(&b));
        snaps.push(sim.snapshot(&w, &b));
    }
    let t = Trace { snaps, reports, last: sim };
    assert!(t.last.countries[0].order_log.len() >= 2, "completed and cancelled orders are both logged");
    // 注文が済んだ時点から全日で再開(取消し・再注文の前後、完成の前後を含む)。注文のタイミング自体は
    // 取得後の状態に含まれる(取得時点以降の手動操作は再開側でも同じ手順を繰り返す必要があるため、
    // 手動操作が無い区間(12日目以降)から比較する)。
    for k in 12..=90 {
        assert_resume_matches(|| fresh_after_orders(&sc, &w, &b), &t, k, &w, &b);
    }
}

fn fresh_after_orders(sc: &MultiMonthScenario, w: &WorldModel, b: &BuildingRegistry) -> MapEconomySimulation {
    make(sc, w, b, &[])
}

#[test]
fn a_cancelled_order_is_never_refunded_again_after_a_resume_and_new_ids_do_not_collide() {
    let (w, b) = (world(), buildings());
    let (sc, mut sim) = m4e_with_manual_orders(&w, &b);
    let id = sim.registry.request_build(&mut sim.countries, 1, s(20), Mine, &b).unwrap();
    for _ in 0..10 {
        sim.advance_day(&b);
    }
    let receipt = sim.registry.cancel_order(&mut sim.countries, 1, CancelTarget::Order(id)).unwrap();
    assert_eq!(receipt.refund, 250.0);
    sim.advance_day(&b);
    let snap = sim.snapshot(&w, &b);

    let mut a = sim.clone(); // 連続実行
    let mut r = make(&sc, &w, &b, &[]); // 復元側
    r.restore(&snap, &w, &b).unwrap();
    for x in [&mut a, &mut r] {
        let before = x.countries[0].treasury;
        let e = x.registry.cancel_order(&mut x.countries, 1, CancelTarget::Order(id)).unwrap_err();
        assert_eq!(e, CancelError::AlreadyCancelled(id), "no second refund");
        assert_eq!(x.countries[0].treasury, before);
        let new_id = x.registry.request_build(&mut x.countries, 1, s(20), Farm, &b).unwrap();
        assert!(new_id.seq > id.seq && new_id.country == 1, "the next id continues after the restored high-water mark");
        assert!(x.countries[0].order_log.iter().all(|(l, _)| *l != new_id));
    }
    assert_eq!(a, r);
    for _ in 0..40 {
        assert_eq!(a.advance_day(&b), r.advance_day(&b));
    }
    assert_eq!(a, r);
}

#[test]
fn a_completed_facility_is_not_added_twice_and_the_fee_is_not_charged_again() {
    let (w, b) = (world(), buildings());
    let (sc, mut sim) = m4e_with_manual_orders(&w, &b);
    let level0 = sim.registry.get(s(20)).unwrap().buildings[&Farm];
    let id = sim.registry.request_build(&mut sim.countries, 1, s(20), Farm, &b).unwrap();
    for _ in 0..31 {
        sim.advance_day(&b);
    }
    assert_eq!(sim.registry.get(s(20)).unwrap().buildings[&Farm], level0 + 1, "completed by day 30");
    let snap = sim.snapshot(&w, &b);
    let mut r = make(&sc, &w, &b, &[]);
    r.restore(&snap, &w, &b).unwrap();
    assert_eq!(r.registry.get(s(20)).unwrap().buildings[&Farm], level0 + 1, "restored level is the saved one, not +2");
    assert_eq!(r.countries[0].treasury, sim.countries[0].treasury, "no fee re-charged by the restore");
    assert!(r.countries[0].construction_queue.is_empty());
    for _ in 0..20 {
        r.advance_day(&b);
        sim.advance_day(&b);
    }
    assert_eq!(r, sim);
    assert_eq!(r.registry.get(s(20)).unwrap().buildings[&Farm], level0 + 1);
    let e = r.registry.cancel_order(&mut r.countries, 1, CancelTarget::Order(id)).unwrap_err();
    assert_eq!(e, CancelError::AlreadyCompleted(id));
}

#[test]
fn a_blocked_order_with_excess_progress_is_restored_and_keeps_being_re_evaluated() {
    let (w, b) = (world(), buildings());
    let sc = scenario("economy_scenario_m4e.ron");
    // 注文(完全な資源定義)→ 20日後に資源定義が変わる(17696の設定を外す)→ 60日で完成条件を満たすが保留。
    let mut a = make(&sc, &w, &b, &[]);
    a.registry.request_build(&mut a.countries, 2, s(21), Mine, &b).unwrap();
    for _ in 0..20 {
        a.advance_day(&b);
    }
    let variant = make(&sc, &w, &b, &[17696]);
    let start = MapEconomySimulation::new(a.date, variant.registry.clone(), a.countries.clone());
    let fresh = || MapEconomySimulation::new(START, variant.registry.clone(), make(&sc, &w, &b, &[]).countries);
    let t = run_trace(start, &w, &b, 60);
    let blocked_days = t.reports.iter().filter(|(_, r)| !r.blocked.is_empty()).count();
    assert!(blocked_days >= 5, "the order is blocked on several days: {blocked_days}");
    let over = t.snaps[60].countries[1].construction_queue[0].clone();
    assert!(over.progress > over.required_progress, "excess progress {over:?}");
    // 日付だけは開始日の集約から復元される(復元側の日付は保存値で置き換わる)。
    for k in [0usize, 39, 40, 41, 50, 60] {
        assert_resume_matches(fresh, &t, k, &w, &b);
    }
}

// ---------------------------------------------------------------- AI履歴

#[test]
fn ai_history_is_continuous_across_a_resume_and_is_needed_for_the_same_decisions() {
    let (w, b) = (world(), buildings());
    let sc = scenario("economy_scenario_m4d.ron");
    let t = run_trace(make(&sc, &w, &b, &[]), &w, &b, 300);
    // 履歴を持つ時点の保存値が、そのまま復元される。
    let hist_len = |s: &MapEconomySnapshot| s.countries.iter().map(|c| c.ai.as_ref().map_or(0, |a| a.supply.len() + a.iron.len() + a.factory.len())).sum::<usize>();
    assert!(hist_len(&t.snaps[100]) > 0);
    let mut r = make(&sc, &w, &b, &[]);
    r.restore(&t.snaps[100], &w, &b).unwrap();
    assert_eq!(hist_len(&r.snapshot(&w, &b)), hist_len(&t.snaps[100]));
    // 対照: 履歴を消した保存値(検証は通る)から再開すると、どこかの日で連続実行と判断・結果が変わる。
    let mut diverged = false;
    for k in [60usize, 90, 120, 150, 180] {
        let mut cleared = t.snaps[k].clone();
        for c in &mut cleared.countries {
            if let Some(a) = &mut c.ai {
                a.supply.clear();
                a.iron.clear();
                a.factory.clear();
            }
        }
        let mut x = make(&sc, &w, &b, &[]);
        x.restore(&cleared, &w, &b).unwrap();
        for d in k + 1..=300 {
            let (_, rep) = x.advance_day(&b);
            if rep != t.reports[d - 1].1 || x.snapshot(&w, &b) != t.snaps[d] {
                diverged = true;
                break;
            }
        }
    }
    assert!(diverged, "without the histories the decisions differ, so saving them is required");
}

// ---------------------------------------------------------------- 復元は操作の再実行ではない

#[test]
fn restore_replaces_the_state_without_running_any_order_cost_ai_or_monthly_step() {
    let (w, b) = (world(), buildings());
    let sc = scenario("economy_scenario_m4d.ron");
    let initial = make(&sc, &w, &b, &[]);
    let snap0 = initial.snapshot(&w, &b);
    let mut progressed = initial.clone();
    for _ in 0..100 {
        progressed.advance_day(&b);
    }
    assert_ne!(progressed, initial);
    // 100日進んだ状態へ開始時のスナップショットを復元すると、開始時と完全に同じになる(AIの初回評価が
    // 走って注文が増えたり、月次経済・費用・返金が実行されたりしていない)。
    progressed.restore(&snap0, &w, &b).unwrap();
    assert_eq!(progressed, initial);
    assert!(progressed.countries.iter().all(|c| c.construction_queue.is_empty() && c.ai.as_ref().unwrap().dirty));
    // 日付は「最後に処理を完了した日」。復元後の最初の処理は翌日で、二重実行も飛ばしもしない。
    assert_eq!(progressed.date, START);
    let (d, _) = progressed.advance_day(&b);
    assert_eq!(d, SimDate { year: 1800, month: 1, day: 2 });
}

#[test]
fn paid_cost_is_a_saved_value_and_is_not_recomputed_from_the_current_building_cost() {
    let (w, b) = (world(), buildings());
    let sc = scenario("economy_scenario_m4e.ron");
    let mut sim = make(&sc, &w, &b, &[]);
    sim.registry.request_build(&mut sim.countries, 1, s(20), Mine, &b).unwrap();
    sim.countries[0].construction_queue[0].paid_cost = 123.0;
    let snap = sim.snapshot(&w, &b);
    let mut r = make(&sc, &w, &b, &[]);
    r.restore(&snap, &w, &b).unwrap();
    assert_eq!(r.countries[0].construction_queue[0].paid_cost, 123.0);
    let before = r.countries[0].treasury;
    let rc = r.registry.cancel_order(&mut r.countries, 1, CancelTarget::Latest).unwrap();
    assert_eq!(rc.refund, 61.5);
    assert_eq!(r.countries[0].treasury, before + 61.5);
}

// ---------------------------------------------------------------- 実行時状態専用の検証

#[test]
fn values_a_run_can_legitimately_reach_are_accepted_even_if_the_input_rules_would_reject_them() {
    let (w, b) = (world(), buildings());
    let (sc, mut sim) = m4e_with_manual_orders(&w, &b);
    for _ in 0..40 {
        sim.advance_day(&b);
    }
    let mut snap = sim.snapshot(&w, &b);
    // 入力では「(0,1]は旧スケールとして曖昧」と拒否される値。実行中の値としては正当に取り得る。
    snap.states[0].living_standard = 0.5;
    snap.states[0].unrest = 0.3;
    // 赤字の国庫(月次財政で到達しうる)。
    snap.countries[1].treasury = -250.0;
    let mut r = make(&sc, &w, &b, &[]);
    r.restore(&snap, &w, &b).unwrap();
    assert_eq!(r.registry.get(s(20)).unwrap().living_standard, 0.5);
    assert_eq!(r.countries[1].treasury, -250.0);
}

#[test]
fn every_state_reached_in_300_days_of_every_scenario_passes_the_runtime_validation() {
    let (w, b) = (world(), buildings());
    for name in ["economy_scenario.ron", "economy_scenario_m4b.ron", "economy_scenario_m4c.ron", "economy_scenario_m4d.ron", "economy_scenario_m4e.ron"] {
        let text = std::fs::read_to_string(format!("{P}{name}")).unwrap();
        // 単一国家の旧形式(`economy_scenario.ron`)は複数国家シナリオではないので対象外。
        let Ok(sc) = parse_multi_month_scenario(&text) else { continue };
        let mut sim = make(&sc, &w, &b, &[]);
        let fresh = make(&sc, &w, &b, &[]);
        for day in 0..=300 {
            let snap = sim.snapshot(&w, &b);
            fresh.restore_candidate(&snap, &w, &b).unwrap_or_else(|e| panic!("{name} day {day}: {e}"));
            sim.advance_day(&b);
        }
    }
}

struct Rich {
    sim: MapEconomySimulation,
    snap: MapEconomySnapshot,
    w: WorldModel,
    b: BuildingRegistry,
}

/// 注文(完成・取消し済み・進行中)・履歴・複数国家を持つ状態(m4e、40日)。
fn rich() -> Rich {
    let (w, b) = (world(), buildings());
    let (_, mut sim) = m4e_with_manual_orders(&w, &b);
    order_all(&mut sim, &b);
    for day in 1..=40 {
        if day == 11 {
            sim.registry.cancel_order(&mut sim.countries, 1, CancelTarget::Latest).unwrap();
            sim.registry.request_build(&mut sim.countries, 1, s(20), Mine, &b).unwrap();
        }
        sim.advance_day(&b);
    }
    let snap = sim.snapshot(&w, &b);
    let c1 = &snap.countries[0];
    assert_eq!(c1.order_log.len(), 2);
    assert_eq!(c1.construction_queue.len(), 1);
    assert_eq!(c1.next_order_seq, 4);
    assert!(!snap.countries[2].ai.as_ref().unwrap().supply.is_empty());
    Rich { sim, snap, w, b }
}

fn reject(mutate: impl FnOnce(&mut MapEconomySnapshot), expected: impl Fn(&RestoreIssue) -> bool) {
    let r = rich();
    let mut bad = r.snap.clone();
    mutate(&mut bad);
    let mut target = r.sim.clone();
    let before = target.clone();
    let err = target.restore(&bad, &r.w, &r.b).expect_err("the invalid snapshot must be rejected");
    assert!(err.issues.iter().any(expected), "unexpected issues: {err}");
    assert_eq!(target, before, "a failed restore must not change any part of the current state");
}

#[test]
fn invalid_headers_dates_and_contexts_are_rejected_without_changing_the_current_state() {
    reject(|s| s.section_version = 2, |i| matches!(i, RestoreIssue::UnsupportedVersion { .. }));
    reject(|s| s.rules_revision = 9, |i| matches!(i, RestoreIssue::RulesRevisionMismatch { .. }));
    reject(|s| s.context ^= 1, |i| matches!(i, RestoreIssue::ContextMismatch));
    reject(|s| s.date.month = 13, |i| matches!(i, RestoreIssue::InvalidDate(_)));
    reject(|s| s.date.day = 32, |i| matches!(i, RestoreIssue::InvalidDate(_)));
    reject(|s| s.date.day = 0, |i| matches!(i, RestoreIssue::InvalidDate(_)));
    reject(|s| s.date.year = 0, |i| matches!(i, RestoreIssue::InvalidDate(_)));
}

#[test]
fn another_context_is_rejected_by_id_alone_not_accepted() {
    let r = rich();
    let sc = scenario("economy_scenario_m4e.ron");
    // 別のシナリオ(同じ MapStateId を含む)・別の資源定義・別の建物定義へは復元できない。
    let other_scenario = make(&scenario("economy_scenario_m4d.ron"), &r.w, &r.b, &[]);
    let other_resources = make(&sc, &r.w, &r.b, &[17696]);
    let mut cheaper = buildings();
    cheaper.definitions.get_mut(&Mine).unwrap().construction_cost += 1.0;
    for (name, target, b) in [
        ("scenario", other_scenario, &r.b),
        ("resources", other_resources, &r.b),
        ("building definitions", make(&sc, &r.w, &cheaper, &[]), &cheaper),
    ] {
        let mut t = target.clone();
        let err = t.restore(&r.snap, &r.w, b).expect_err(name);
        assert!(err.issues.contains(&RestoreIssue::ContextMismatch), "{name}: {err}");
        assert_eq!(t, target);
    }
}

#[test]
fn invalid_states_countries_and_numbers_are_rejected_without_changing_the_current_state() {
    reject(|s| { s.states.pop(); }, |i| matches!(i, RestoreIssue::StateSetMismatch(_)));
    reject(|s| { let d = s.states[0].clone(); s.states.push(d); }, |i| matches!(i, RestoreIssue::StateSetMismatch(_)));
    reject(|s| s.states[2].state = 99_999, |i| matches!(i, RestoreIssue::StateSetMismatch(_)));
    reject(|s| s.states.swap(0, 1), |i| matches!(i, RestoreIssue::StateSetMismatch(_)));
    reject(|s| s.states[0].living_standard = f32::NAN, |i| matches!(i, RestoreIssue::InvalidValue(_)));
    reject(|s| s.states[0].unrest = 101.0, |i| matches!(i, RestoreIssue::InvalidValue(_)));
    reject(|s| s.states[0].logistics_ratio = -1.0, |i| matches!(i, RestoreIssue::InvalidValue(_)));
    reject(|s| { let d = s.states[0].buildings[0]; s.states[0].buildings.push(d); }, |i| matches!(i, RestoreIssue::InvalidValue(_)));
    reject(|s| { s.countries.pop(); }, |i| matches!(i, RestoreIssue::CountrySetMismatch(_)));
    reject(|s| s.countries.swap(0, 1), |i| matches!(i, RestoreIssue::CountrySetMismatch(_)));
    reject(|s| s.countries[0].label = 77, |i| matches!(i, RestoreIssue::CountrySetMismatch(_)));
    reject(|s| s.countries[2].ai = None, |i| matches!(i, RestoreIssue::CountrySetMismatch(_)));
    reject(|s| s.countries[0].treasury = f64::NAN, |i| matches!(i, RestoreIssue::InvalidValue(_)));
    reject(|s| s.countries[0].treasury = f64::INFINITY, |i| matches!(i, RestoreIssue::InvalidValue(_)));
    reject(|s| s.countries[0].tax_rate = 1.5, |i| matches!(i, RestoreIssue::InvalidValue(_)));
    reject(|s| s.countries[0].monthly_balance = f64::NAN, |i| matches!(i, RestoreIssue::InvalidValue(_)));
    reject(|s| s.countries[0].stockpile[0].1 = -1.0, |i| matches!(i, RestoreIssue::InvalidValue(_)));
    reject(|s| { s.countries[0].stockpile.pop(); }, |i| matches!(i, RestoreIssue::InvalidValue(_)));
}

#[test]
fn invalid_orders_and_identifiers_are_rejected_without_changing_the_current_state() {
    let is_order = |i: &RestoreIssue| matches!(i, RestoreIssue::InvalidOrder(_));
    reject(|s| { let o = s.countries[0].construction_queue[0].clone(); s.countries[0].construction_queue.push(o); }, is_order);
    reject(|s| s.countries[0].construction_queue[0].id.country = 2, is_order);
    reject(|s| { let n = s.countries[0].next_order_seq; s.countries[0].construction_queue[0].id.seq = n; }, is_order);
    reject(|s| s.countries[0].next_order_seq = 3, is_order);
    reject(|s| s.countries[0].next_order_seq = 10, is_order);
    reject(|s| s.countries[0].next_order_seq = 0, is_order);
    reject(|s| { let id = s.countries[0].construction_queue[0].id; s.countries[0].order_log[0].0 = id; }, is_order);
    reject(|s| { s.countries[0].order_log.pop(); }, is_order);
    reject(|s| s.countries[0].construction_queue[0].target_level += 1, is_order);
    reject(|s| s.countries[0].construction_queue[0].state = 21, is_order);
    reject(|s| s.countries[0].construction_queue[0].progress = f64::NAN, is_order);
    reject(|s| s.countries[0].construction_queue[0].paid_cost = -1.0, is_order);
    reject(|s| s.countries[0].construction_queue[0].required_progress = 0.0, is_order);
    // 完成済みの注文がキューに残っている(再追加の兆候): 完成済みの注文のログを消して同じ内容をキューへ入れる。
    reject(
        |s| {
            let mut o = s.countries[0].construction_queue[0].clone();
            o.id = s.countries[0].order_log[0].0;
            o.state = 20;
            o.building_type = Farm;
            o.target_level = 1;
            s.countries[0].order_log.remove(0);
            s.countries[0].construction_queue.push(o);
        },
        is_order,
    );
}

#[test]
fn invalid_histories_are_rejected_without_changing_the_current_state() {
    let is_hist = |i: &RestoreIssue| matches!(i, RestoreIssue::InvalidHistory(_));
    let p = |y: i32, m: u8| MonthlyProductionSample { year: y, month: m, units: Some(1) };
    let iron = |y: i32, m: u8| IronShortageSample { year: y, month: m, unmet_milli: Some(1), mine_levels: 1 };
    let fac = |y: i32, m: u8| FactoryStarvationSample { year: y, month: m, signature: 1, starved: false };
    reject(|s| s.countries[2].ai.as_mut().unwrap().supply = (1..=5).map(|m| p(1799, m)).collect(), is_hist); // 窓幅超過
    reject(|s| s.countries[2].ai.as_mut().unwrap().supply = vec![p(1799, 3), p(1799, 2)], is_hist); // 逆順
    reject(|s| s.countries[2].ai.as_mut().unwrap().iron = vec![iron(1799, 2), iron(1799, 2)], is_hist); // 重複
    reject(|s| s.countries[2].ai.as_mut().unwrap().factory = vec![fac(1799, 0)], is_hist); // 不正な月
    reject(|s| s.countries[2].ai.as_mut().unwrap().factory = vec![fac(1799, 13)], is_hist);
    reject(|s| s.countries[2].ai.as_mut().unwrap().iron = vec![iron(1801, 1)], is_hist); // 保存日付より後
    reject(|s| s.countries[2].ai.as_mut().unwrap().last_monthly_evaluation_day = u32::MAX, is_hist);
    // 正当な履歴(窓幅いっぱい・過去の月)は受け入れられる。
    let r = rich();
    let mut ok = r.snap.clone();
    ok.countries[2].ai.as_mut().unwrap().supply = (1..=4).map(|m| p(1799, m)).collect();
    let mut t = r.sim.clone();
    t.restore(&ok, &r.w, &r.b).unwrap();
}
