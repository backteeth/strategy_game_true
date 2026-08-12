//! P20-008: 1000州以上規模での本番日次シミュレーション性能プロファイリング。
//!
//! 実行方法:
//!   cargo run --release --bin profile_1000_states -- <output_subdir>
//!
//! `<output_subdir>` 省略時は `baseline`。
//! `strategy_game/verification_logs/p20-008/<output_subdir>/` へ以下を出力する:
//!   - summary.txt   (人間可読サマリー)
//!   - results.csv   (規模・シナリオ別の機械可読結果)
//!   - set_timings.csv (SystemSet別内訳)
//!   - environment.txt (OS/CPU/メモリ/Rustバージョン)
//!
//! Windowは一切生成せず、対話操作を必要としない。乱数は固定Seedを使用し、
//! 同一入力から常に同一のワールドを生成する。処理失敗（正しさ検証NG等）は
//! 即座にpanicして非ゼロ終了する。

use bevy::app::App;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use strategy_game::profiling::{
    self, DAILY_SET_NAMES, RuntimeCounts, Scenario, WorkloadConfig, WorkloadCounts,
};

const FIXED_SEED: u64 = 0x00C0_FFEE_1234_5678;
const WARMUP_DAYS: usize = 10;
/// 60日 (約2ヶ月) 計測することで、月次イベント(Economy/Research)による
/// スパイクを含む代表的な分布(平均・中央値・p95)を得る。
const MEASURED_DAYS: usize = 60;
const SCALES: [usize; 4] = [100, 500, 1000, 2000];

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
    fn p95(&self) -> f64 {
        self.percentile(0.95)
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

struct RunResult {
    scale: usize,
    scenario: Scenario,
    workload: WorkloadCounts,
    runtime_after: RuntimeCounts,
    init_ms: f64,
    overall: DayStats,
    per_set: [DayStats; 11],
    treasury_before: f64,
    treasury_after: f64,
    population_before: u64,
    population_after: u64,
    memory_bytes: Option<u64>,
    peak_memory_bytes: Option<u64>,
}

fn query_powershell(cmd: &str) -> Option<String> {
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", cmd])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn current_memory_bytes() -> Option<u64> {
    let pid = std::process::id();
    query_powershell(&format!("(Get-Process -Id {pid}).WorkingSet64"))?
        .parse::<u64>()
        .ok()
}

/// プロセス開始からその時点までのピークワーキングセット(Windowsの`PeakWorkingSet64`)。
/// `WorkingSet64`(現在値)とは異なり、一時的なスパイクも含めた最大値を報告する。
fn peak_memory_bytes() -> Option<u64> {
    let pid = std::process::id();
    query_powershell(&format!("(Get-Process -Id {pid}).PeakWorkingSet64"))?
        .parse::<u64>()
        .ok()
}

fn write_environment_info(path: &Path) {
    let mut out = String::new();
    out.push_str(&format!("os = {}\n", std::env::consts::OS));
    out.push_str(&format!("arch = {}\n", std::env::consts::ARCH));

    if let Some(rustc) = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    {
        out.push_str(&format!("rustc = {rustc}\n"));
    }

    if let Some(cpu) = query_powershell("(Get-CimInstance Win32_Processor).Name") {
        out.push_str(&format!("cpu = {cpu}\n"));
    }
    if let Some(cores) =
        query_powershell("(Get-CimInstance Win32_Processor).NumberOfLogicalProcessors")
    {
        out.push_str(&format!("logical_processors = {cores}\n"));
    }
    if let Some(mem) = query_powershell(
        "[math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory/1GB,1)",
    ) {
        out.push_str(&format!("total_physical_memory_gb = {mem}\n"));
    }
    out.push_str("build_profile = release (cargo run --release)\n");

    fs::write(path, out).expect("environment.txt 書き込みに失敗");
}

fn run_one(scale: usize, scenario: Scenario) -> RunResult {
    let config = WorkloadConfig {
        state_count: scale,
        scenario,
        seed: FIXED_SEED,
    };

    let init_start = Instant::now();
    let (mut app, workload): (App, WorkloadCounts) = profiling::build_app(&config);
    profiling::install_set_timings(&mut app);
    let init_ms = init_start.elapsed().as_secs_f64() * 1000.0;

    if let Err(e) = profiling::validate_world_sanity(&app) {
        panic!(
            "[P20-008][FAIL:correctness] scale={scale} scenario={:?}: world sanity check failed immediately after construction: {e}",
            scenario
        );
    }

    // ── ウォームアップ (未計測) ──────────────────────────────────────────────
    for _ in 0..WARMUP_DAYS {
        profiling::advance_one_day(&mut app);
    }

    if let Err(e) = profiling::validate_world_sanity(&app) {
        panic!(
            "[P20-008][FAIL:correctness] scale={scale} scenario={:?}: world sanity check failed after warmup: {e}",
            scenario
        );
    }

    let treasury_before = profiling::total_treasury(&app);
    let population_before = profiling::total_population(&app);

    // ── 計測 ────────────────────────────────────────────────────────────────
    let mut overall = DayStats::new();
    let mut per_set: [DayStats; 11] = std::array::from_fn(|_| DayStats::new());

    for _ in 0..MEASURED_DAYS {
        let start = Instant::now();
        profiling::advance_one_day(&mut app);
        let elapsed = start.elapsed();
        overall.push(elapsed);

        if let Some(durations) = profiling::read_set_durations(&app) {
            for (i, d) in durations.iter().enumerate() {
                per_set[i].push(*d);
            }
        }
    }

    if let Err(e) = profiling::validate_world_sanity(&app) {
        panic!(
            "[P20-008][FAIL:correctness] scale={scale} scenario={:?}: world sanity check failed after measured ticks: {e}",
            scenario
        );
    }

    let runtime_after = profiling::snapshot_runtime_counts(&app);
    let treasury_after = profiling::total_treasury(&app);
    let population_after = profiling::total_population(&app);
    let memory_bytes = current_memory_bytes();
    let peak_memory_bytes_val = peak_memory_bytes();

    println!(
        "[P20-008] scale={scale} scenario={} done: mean={:.4}ms median={:.4}ms p95={:.4}ms max={:.4}ms ticks/s={:.1}",
        scenario.label(),
        overall.mean(),
        overall.median(),
        overall.p95(),
        overall.max(),
        1000.0 / overall.mean().max(0.0001),
    );

    RunResult {
        scale,
        scenario,
        workload,
        runtime_after,
        init_ms,
        overall,
        per_set,
        treasury_before,
        treasury_after,
        population_before,
        population_after,
        memory_bytes,
        peak_memory_bytes: peak_memory_bytes_val,
    }
}

fn write_results_csv(path: &Path, results: &[RunResult]) {
    let mut csv = String::new();
    csv.push_str(
        "scale,scenario,state_count,country_count,division_initial,war_initial,division_final,war_active_final,frontline_final,battle_final,ecs_entity_count,init_ms,warmup_days,measured_days,mean_ms,median_ms,p95_ms,min_ms,max_ms,ticks_per_sec,treasury_before,treasury_after,treasury_changed,population_before,population_after,population_changed,memory_bytes,peak_memory_bytes\n",
    );
    for r in results {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{:.4},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.2},{:.2},{:.2},{},{},{},{},{},{}\n",
            r.scale,
            r.scenario.label(),
            r.workload.state_count,
            r.workload.country_count,
            r.workload.initial_division_count,
            r.workload.initial_war_count,
            r.runtime_after.division_count,
            r.runtime_after.active_war_count,
            r.runtime_after.frontline_count,
            r.runtime_after.battle_count,
            r.runtime_after.ecs_entity_count,
            r.init_ms,
            WARMUP_DAYS,
            MEASURED_DAYS,
            r.overall.mean(),
            r.overall.median(),
            r.overall.p95(),
            r.overall.min(),
            r.overall.max(),
            1000.0 / r.overall.mean().max(0.0001),
            r.treasury_before,
            r.treasury_after,
            r.treasury_before != r.treasury_after,
            r.population_before,
            r.population_after,
            r.population_before != r.population_after,
            r.memory_bytes.map(|b| b.to_string()).unwrap_or_default(),
            r.peak_memory_bytes.map(|b| b.to_string()).unwrap_or_default(),
        ));
    }
    fs::write(path, csv).expect("results.csv 書き込みに失敗");
}

fn write_set_timings_csv(path: &Path, results: &[RunResult]) {
    let mut csv = String::new();
    csv.push_str("scale,scenario,set_name,mean_ms,median_ms,p95_ms,max_ms,pct_of_overall_mean\n");
    for r in results {
        let overall_mean = r.overall.mean().max(1e-9);
        for (i, set_name) in DAILY_SET_NAMES.iter().enumerate() {
            let s = &r.per_set[i];
            csv.push_str(&format!(
                "{},{},{},{:.5},{:.5},{:.5},{:.5},{:.2}\n",
                r.scale,
                r.scenario.label(),
                set_name,
                s.mean(),
                s.median(),
                s.p95(),
                s.max(),
                (s.mean() / overall_mean) * 100.0,
            ));
        }
    }
    fs::write(path, csv).expect("set_timings.csv 書き込みに失敗");
}

fn write_results_json(path: &Path, results: &[RunResult]) {
    let mut json = String::from("{\n  \"runs\": [\n");
    for (idx, r) in results.iter().enumerate() {
        json.push_str("    {\n");
        json.push_str(&format!("      \"scale\": {},\n", r.scale));
        json.push_str(&format!(
            "      \"scenario\": \"{}\",\n",
            r.scenario.label()
        ));
        json.push_str(&format!(
            "      \"state_count\": {},\n",
            r.workload.state_count
        ));
        json.push_str(&format!(
            "      \"country_count\": {},\n",
            r.workload.country_count
        ));
        json.push_str(&format!(
            "      \"division_initial\": {},\n",
            r.workload.initial_division_count
        ));
        json.push_str(&format!(
            "      \"war_initial\": {},\n",
            r.workload.initial_war_count
        ));
        json.push_str(&format!(
            "      \"division_final\": {},\n",
            r.runtime_after.division_count
        ));
        json.push_str(&format!(
            "      \"war_active_final\": {},\n",
            r.runtime_after.active_war_count
        ));
        json.push_str(&format!(
            "      \"frontline_final\": {},\n",
            r.runtime_after.frontline_count
        ));
        json.push_str(&format!(
            "      \"battle_final\": {},\n",
            r.runtime_after.battle_count
        ));
        json.push_str(&format!(
            "      \"ecs_entity_count\": {},\n",
            r.runtime_after.ecs_entity_count
        ));
        json.push_str(&format!("      \"init_ms\": {:.4},\n", r.init_ms));
        json.push_str(&format!("      \"warmup_days\": {WARMUP_DAYS},\n"));
        json.push_str(&format!("      \"measured_days\": {MEASURED_DAYS},\n"));
        json.push_str(&format!("      \"mean_ms\": {:.4},\n", r.overall.mean()));
        json.push_str(&format!(
            "      \"median_ms\": {:.4},\n",
            r.overall.median()
        ));
        json.push_str(&format!("      \"p95_ms\": {:.4},\n", r.overall.p95()));
        json.push_str(&format!("      \"min_ms\": {:.4},\n", r.overall.min()));
        json.push_str(&format!("      \"max_ms\": {:.4},\n", r.overall.max()));
        json.push_str(&format!(
            "      \"ticks_per_sec\": {:.2},\n",
            1000.0 / r.overall.mean().max(0.0001)
        ));
        json.push_str(&format!(
            "      \"memory_bytes\": {},\n",
            r.memory_bytes
                .map(|b| b.to_string())
                .unwrap_or_else(|| "null".to_string())
        ));
        json.push_str(&format!(
            "      \"peak_memory_bytes\": {},\n",
            r.peak_memory_bytes
                .map(|b| b.to_string())
                .unwrap_or_else(|| "null".to_string())
        ));
        json.push_str("      \"per_set\": [\n");
        for (i, set_name) in DAILY_SET_NAMES.iter().enumerate() {
            let s = &r.per_set[i];
            json.push_str(&format!(
                "        {{\"name\": \"{}\", \"mean_ms\": {:.5}, \"median_ms\": {:.5}, \"p95_ms\": {:.5}}}{}\n",
                set_name,
                s.mean(),
                s.median(),
                s.p95(),
                if i + 1 < DAILY_SET_NAMES.len() { "," } else { "" }
            ));
        }
        json.push_str("      ]\n");
        json.push_str(if idx + 1 < results.len() {
            "    },\n"
        } else {
            "    }\n"
        });
    }
    json.push_str("  ]\n}\n");
    fs::write(path, json).expect("results.json 書き込みに失敗");
}

fn write_summary_txt(path: &Path, results: &[RunResult]) {
    let mut out = String::new();
    out.push_str("P20-008 Profiling Summary\n");
    out.push_str("=========================\n\n");
    out.push_str(&format!(
        "warmup_days={WARMUP_DAYS} measured_days={MEASURED_DAYS} seed=0x{FIXED_SEED:X}\n\n"
    ));

    for r in results {
        out.push_str(&format!(
            "--- scale={} scenario={} ---\n",
            r.scale,
            r.scenario.label()
        ));
        out.push_str(&format!(
            "  states={} countries={} divisions(init={} final={}) wars(init={} active_final={}) frontlines_final={} battles_final={} ecs_entity_count={}\n",
            r.workload.state_count,
            r.workload.country_count,
            r.workload.initial_division_count,
            r.runtime_after.division_count,
            r.workload.initial_war_count,
            r.runtime_after.active_war_count,
            r.runtime_after.frontline_count,
            r.runtime_after.battle_count,
            r.runtime_after.ecs_entity_count,
        ));
        out.push_str(&format!("  init_ms={:.3}\n", r.init_ms));
        out.push_str(&format!(
            "  day_tick: mean={:.4}ms median={:.4}ms p95={:.4}ms min={:.4}ms max={:.4}ms ticks/s={:.1}\n",
            r.overall.mean(),
            r.overall.median(),
            r.overall.p95(),
            r.overall.min(),
            r.overall.max(),
            1000.0 / r.overall.mean().max(0.0001),
        ));
        out.push_str(&format!(
            "  memory_bytes={} peak_memory_bytes={}\n",
            r.memory_bytes
                .map(|b| b.to_string())
                .unwrap_or_else(|| "unavailable".to_string()),
            r.peak_memory_bytes
                .map(|b| b.to_string())
                .unwrap_or_else(|| "unavailable".to_string())
        ));
        out.push_str(&format!(
            "  treasury: before={:.2} after={:.2} changed={}\n",
            r.treasury_before,
            r.treasury_after,
            r.treasury_before != r.treasury_after
        ));
        out.push_str(&format!(
            "  population: before={} after={} changed={}\n",
            r.population_before,
            r.population_after,
            r.population_before != r.population_after
        ));
        out.push_str("  per-SystemSet mean(ms) [share of overall mean]:\n");
        let overall_mean = r.overall.mean().max(1e-9);
        for (i, set_name) in DAILY_SET_NAMES.iter().enumerate() {
            let s = &r.per_set[i];
            out.push_str(&format!(
                "    {:<16} mean={:>10.5}ms  p95={:>10.5}ms  ({:>5.1}%)\n",
                set_name,
                s.mean(),
                s.p95(),
                (s.mean() / overall_mean) * 100.0
            ));
        }
        out.push('\n');
    }

    fs::write(path, out).expect("summary.txt 書き込みに失敗");
}

fn main() {
    let output_subdir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "baseline".to_string());
    let output_dir: PathBuf = Path::new("verification_logs/p20-008").join(&output_subdir);
    fs::create_dir_all(&output_dir).expect("出力ディレクトリの作成に失敗");

    println!("[P20-008] output_dir = {}", output_dir.display());
    write_environment_info(&output_dir.join("environment.txt"));

    let mut results = Vec::new();
    for &scale in &SCALES {
        for scenario in [Scenario::Normal, Scenario::HighLoad] {
            println!(
                "[P20-008] running scale={scale} scenario={} ...",
                scenario.label()
            );
            let result = run_one(scale, scenario);
            results.push(result);
            std::io::stdout().flush().ok();
        }
    }

    write_results_csv(&output_dir.join("results.csv"), &results);
    write_set_timings_csv(&output_dir.join("set_timings.csv"), &results);
    write_results_json(&output_dir.join("results.json"), &results);
    write_summary_txt(&output_dir.join("summary.txt"), &results);

    println!("[P20-008] All scales/scenarios completed successfully.");
    println!("[P20-008] Evidence written to {}", output_dir.display());
}
