//! P21-010-PERF-VERIFY: CrisisRegistry件数に対する日次処理コストのスケーリング計測。
//!
//! `src/bin/profile_1000_states.rs` と同一の `strategy_game::profiling` ビルド・計測経路を
//! 再利用し、World構築直後に合成`DiplomaticCrisis`をN件`CrisisRegistry`へ直接挿入してから
//! 計測する。本ファイルは検証専用の一時worktreeにのみ存在し、本流には反映しない。
//!
//! 実行方法:
//!   cargo run --release --bin profile_crisis_scaling -- <output_subdir>

use bevy::app::App;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use strategy_game::common::{CountryId, DiplomaticCrisisId};
use strategy_game::diplomacy::crisis::{CrisisPhase, CrisisRegistry, DiplomaticCrisis};
use strategy_game::profiling::{self, DAILY_SET_NAMES, Scenario, WorkloadConfig};

const FIXED_SEED: u64 = 0x00C0_FFEE_1234_5678;
const WARMUP_DAYS: usize = 10;
const MEASURED_DAYS: usize = 60;
const STATE_COUNT: usize = 2000;

#[derive(Debug, Clone, Copy)]
enum CrisisMix {
    /// 全件非terminal(days_in_phaseが毎日加算される最悪ケース)
    AllActive,
    /// 全件terminal(ResolvedPeacefully; 毎日スキップされるはずのケース)
    AllTerminal,
}

struct DayStats {
    samples_ms: Vec<f64>,
}
impl DayStats {
    fn new() -> Self {
        Self {
            samples_ms: Vec::new(),
        }
    }
    fn push(&mut self, d: Duration) {
        self.samples_ms.push(d.as_secs_f64() * 1000.0);
    }
    fn mean(&self) -> f64 {
        if self.samples_ms.is_empty() {
            return 0.0;
        }
        self.samples_ms.iter().sum::<f64>() / self.samples_ms.len() as f64
    }
    fn percentile(&self, p: f64) -> f64 {
        if self.samples_ms.is_empty() {
            return 0.0;
        }
        let mut sorted = self.samples_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }
    fn median(&self) -> f64 {
        self.percentile(0.5)
    }
    fn min(&self) -> f64 {
        self.samples_ms
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min)
    }
    fn max(&self) -> f64 {
        self.samples_ms.iter().cloned().fold(0.0, f64::max)
    }
}

fn make_crisis(idx: usize, country_count: usize, phase: CrisisPhase) -> DiplomaticCrisis {
    let a = idx % country_count;
    let b = (idx + 1) % country_count;
    DiplomaticCrisis {
        id: DiplomaticCrisisId(0),
        initiator: CountryId(a),
        target: CountryId(b),
        war_goals: Vec::new(),
        start_date: "1800/01/01".to_string(),
        current_phase: phase,
        escalation: 0.0,
        initiator_support: 0.0,
        target_resistance: 0.0,
        days_in_phase: 0,
        deadline_date: None,
        international_concern: 0.0,
        third_party_reactions: std::collections::HashMap::new(),
    }
}

fn run_one(crisis_count: usize, mix: CrisisMix) -> (WorkloadConfig, DayStats, DayStats, u32, u32) {
    let config = WorkloadConfig {
        state_count: STATE_COUNT,
        scenario: Scenario::Normal,
        seed: FIXED_SEED,
    };

    let (mut app, workload): (App, _) = profiling::build_app(&config);
    profiling::install_set_timings(&mut app);

    let phase = match mix {
        CrisisMix::AllActive => CrisisPhase::Preparing,
        CrisisMix::AllTerminal => CrisisPhase::ResolvedPeacefully,
    };
    {
        let mut registry = app.world_mut().resource_mut::<CrisisRegistry>();
        for i in 0..crisis_count {
            let c = make_crisis(i, workload.country_count, phase);
            registry.add_crisis(c);
        }
    }

    if let Err(e) = profiling::validate_world_sanity(&app) {
        panic!(
            "[P21-010-PERF-VERIFY][FAIL:correctness] crisis_count={crisis_count} mix={mix:?}: world sanity check failed immediately after crisis injection: {e}"
        );
    }

    for _ in 0..WARMUP_DAYS {
        profiling::advance_one_day(&mut app);
    }

    let mut overall = DayStats::new();
    let mut diplomacy_set = DayStats::new();
    let diplomacy_idx = DAILY_SET_NAMES
        .iter()
        .position(|n| *n == "Diplomacy")
        .expect("Diplomacy set must exist in DAILY_SET_NAMES");

    for _ in 0..MEASURED_DAYS {
        let start = Instant::now();
        profiling::advance_one_day(&mut app);
        let elapsed = start.elapsed();
        overall.push(elapsed);

        if let Some(durations) = profiling::read_set_durations(&app) {
            diplomacy_set.push(durations[diplomacy_idx]);
        }
    }

    if let Err(e) = profiling::validate_world_sanity(&app) {
        panic!(
            "[P21-010-PERF-VERIFY][FAIL:correctness] crisis_count={crisis_count} mix={mix:?}: world sanity check failed after measured ticks: {e}"
        );
    }

    // HashMap反復順に依存せず、全crisisのdays_in_phase合計が決定論的であることを確認する。
    let registry = app.world().resource::<CrisisRegistry>();
    let total_days_in_phase: u32 = registry.crises.values().map(|c| c.days_in_phase).sum();
    let expected_days_in_phase: u32 = match mix {
        CrisisMix::AllActive => (crisis_count as u32) * ((WARMUP_DAYS + MEASURED_DAYS) as u32),
        CrisisMix::AllTerminal => 0,
    };
    assert_eq!(
        total_days_in_phase, expected_days_in_phase,
        "[P21-010-PERF-VERIFY][FAIL:correctness] crisis_count={crisis_count} mix={mix:?}: \
         days_in_phase合計がHashMap反復順に非依存で決定論的な期待値と一致すること"
    );

    (config, overall, diplomacy_set, total_days_in_phase, expected_days_in_phase)
}

fn main() {
    let output_subdir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "crisis_scaling".to_string());
    let output_dir: PathBuf = Path::new("verification_logs/p20-008").join(&output_subdir);
    fs::create_dir_all(&output_dir).expect("出力ディレクトリの作成に失敗");

    let cases: [(usize, CrisisMix); 5] = [
        (0, CrisisMix::AllActive),
        (1, CrisisMix::AllActive),
        (100, CrisisMix::AllActive),
        (1000, CrisisMix::AllActive),
        (1000, CrisisMix::AllTerminal),
    ];

    let mut out = String::new();
    out.push_str("P21-010-PERF-VERIFY Crisis Scaling Summary\n");
    out.push_str("===========================================\n\n");
    out.push_str(&format!(
        "state_count={STATE_COUNT} warmup_days={WARMUP_DAYS} measured_days={MEASURED_DAYS} seed=0x{FIXED_SEED:X}\n\n"
    ));

    let mut csv = String::from(
        "crisis_count,mix,overall_mean_ms,overall_median_ms,overall_min_ms,overall_max_ms,diplomacy_mean_ms,diplomacy_median_ms,total_days_in_phase,expected_days_in_phase\n",
    );

    for (crisis_count, mix) in cases {
        println!("[P21-010-PERF-VERIFY] running crisis_count={crisis_count} mix={mix:?} ...");
        let (_config, overall, diplomacy_set, total, expected) = run_one(crisis_count, mix);

        println!(
            "[P21-010-PERF-VERIFY] crisis_count={crisis_count} mix={mix:?} done: overall_mean={:.5}ms diplomacy_mean={:.5}ms total_days_in_phase={} expected={}",
            overall.mean(),
            diplomacy_set.mean(),
            total,
            expected,
        );

        out.push_str(&format!("--- crisis_count={crisis_count} mix={mix:?} ---\n"));
        out.push_str(&format!(
            "  overall day_tick: mean={:.5}ms median={:.5}ms min={:.5}ms max={:.5}ms\n",
            overall.mean(),
            overall.median(),
            overall.min(),
            overall.max(),
        ));
        out.push_str(&format!(
            "  Diplomacy set:    mean={:.5}ms median={:.5}ms\n",
            diplomacy_set.mean(),
            diplomacy_set.median(),
        ));
        out.push_str(&format!(
            "  days_in_phase total={total} expected={expected} (HashMap反復順に非依存で決定論的)\n\n"
        ));

        csv.push_str(&format!(
            "{},{:?},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{},{}\n",
            crisis_count,
            mix,
            overall.mean(),
            overall.median(),
            overall.min(),
            overall.max(),
            diplomacy_set.mean(),
            diplomacy_set.median(),
            total,
            expected,
        ));
    }

    fs::write(output_dir.join("summary.txt"), out).expect("summary.txt 書き込みに失敗");
    fs::write(output_dir.join("results.csv"), csv).expect("results.csv 書き込みに失敗");

    println!("[P21-010-PERF-VERIFY] All crisis scaling cases completed successfully.");
    println!("[P21-010-PERF-VERIFY] Evidence written to {}", output_dir.display());
}
