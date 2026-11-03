//! P32-026: 保存・復元の工程別計時の測定(P32-024 の合成入力と共有シミュレーションを使う)。
//!
//! **保存・復元の別実装は持たない。** 計時なしは通常のAPI(`map_economy_file::save` / `load`)、計時ありは同じ共通本体を
//! 計時つきで呼ぶ API(`save_timed` / `load_timed`)を呼ぶだけ。入力生成・組み立て・比較用のダイジェスト・一時ディレクトリの
//! 後始末は、保存・復元の計時の外で行う。
//!
//! 手順(1ケース=1プロセス=1モード): 合成入力を作り、180日進めた状態を基準にする → 試行ごとに新しい一時ディレクトリへ保存し、
//! 新しく組み立てたセッションへ復元する(各試行の保存ファイル・復元状態・判定を確認) → 最後の復元状態から180日継続し、
//! 基準側の同じ180日と各日のダイジェストを比較する。

use std::path::Path;
use std::time::{Duration, Instant};

use crate::building::data::BuildingRegistry;
use crate::economy::map_economy_file::{self, SaveLocation};
use crate::economy::map_state::snapshot::MapEconomySimulation;
use crate::economy::map_state::static_identity::StaticInputs;
use crate::economy::phase_timing::{Outcome, Timing};
use crate::economy::scale_harness::{
    Abort, CaseStatus, Deadline, RunStats, ScaleSpec, Script, assemble, build_world, generate, ms, state_digest,
};
use crate::hexmap::data::HexMap;

const SAVE_NAME: &str = "p32_026_timing";

#[derive(Debug, Clone, PartialEq)]
pub struct BenchSpec {
    pub scale: ScaleSpec,
    /// 計時あり(`save_timed` / `load_timed`)か、通常のAPIか。
    pub timed: bool,
    pub trials: u32,
}

/// 1回の保存または復元の結果(外側から計った時間と、計時ありなら工程別の記録)。
#[derive(Debug, Clone)]
pub struct OpRecord {
    /// 呼び出しの前後を外側から計った時間(計時ありのときは、計時の費用を含む)。
    pub wall_ms: f64,
    pub timing: Option<Timing>,
}

#[derive(Debug, Clone)]
pub struct Trial {
    pub index: u32,
    pub save: OpRecord,
    pub restore: OpRecord,
    pub file_bytes: u64,
    pub file_blake3: String,
    pub restored_equals_state_at_save: bool,
    pub verdict_exact: bool,
    pub verdict_warnings: usize,
    pub restored_date: String,
}

#[derive(Debug, Clone)]
pub struct BenchResult {
    pub spec: BenchSpec,
    pub status: CaseStatus,
    pub states: usize,
    pub countries: usize,
    pub queued_orders_at_save: usize,
    pub save_date: String,
    /// 入力生成・世界の検証・組み立て・基準の180日進行(保存・復元の計時の外)。
    pub setup_ms: f64,
    pub trials: Vec<Trial>,
    pub compared_days: u32,
    pub digest_mismatch_days: Vec<u32>,
    pub final_states_equal: bool,
    pub distinct_digests: usize,
    /// 後半180日の各日のダイジェストを順につなげたもののBLAKE3(別プロセス・別モードとの比較用)。
    pub digest_chain: String,
    pub file_hashes_equal_across_trials: bool,
    pub total_ms: f64,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 1ケースを測定する。`base` は測定用保存の置き場所(試行ごとの一時ディレクトリを下に作り、自分のものだけ消す)。
pub fn run_bench_case(map: &HexMap, buildings: &BuildingRegistry, spec: &BenchSpec, base: &Path) -> BenchResult {
    let t0 = Instant::now();
    let mut r = BenchResult {
        spec: spec.clone(),
        status: CaseStatus::Success,
        states: 0,
        countries: 0,
        queued_orders_at_save: 0,
        save_date: String::new(),
        setup_ms: 0.0,
        trials: Vec::new(),
        compared_days: 0,
        digest_mismatch_days: Vec::new(),
        final_states_equal: false,
        distinct_digests: 0,
        digest_chain: String::new(),
        file_hashes_equal_across_trials: false,
        total_ms: 0.0,
    };
    let deadline = Deadline { start: t0, limit: spec.scale.time_limit };
    r.status = match inner(map, buildings, spec, base, &deadline, &mut r) {
        Ok(failures) if failures.is_empty() => CaseStatus::Success,
        Ok(failures) => CaseStatus::CorrectnessFailure(failures),
        Err(Abort::Timeout(p)) => CaseStatus::TimedOut { phase: p },
        Err(Abort::Unrunnable(m)) => CaseStatus::Unrunnable(m),
    };
    r.total_ms = ms(t0.elapsed());
    r
}

fn inner(
    map: &HexMap,
    buildings: &BuildingRegistry,
    spec: &BenchSpec,
    base: &Path,
    deadline: &Deadline,
    r: &mut BenchResult,
) -> Result<Vec<String>, Abort> {
    let scale = &spec.scale;
    scale.validate()?;
    if spec.trials == 0 || spec.trials > 20 {
        return Err(Abort::Unrunnable(format!("trials must be within 1..=20 (got {})", spec.trials)));
    }
    if base.components().any(|c| c.as_os_str() == "saves") {
        return Err(Abort::Unrunnable(format!("measurement save base must not be inside a 'saves' directory: {}", base.display())));
    }
    let mut failures: Vec<String> = Vec::new();

    // --- 準備(計時の外): 入力・世界・セッション・基準の前半
    let t = Instant::now();
    let input = generate(scale, map)?;
    let world = build_world(map, &input)?;
    let sim0 = assemble(&input, &world, buildings)?;
    r.states = sim0.registry.len();
    r.countries = sim0.countries.len();
    let script = Script::new(&input, scale.scripted_orders);
    let mut reference = sim0;
    let mut ref_stats = RunStats::default();
    crate::economy::scale_harness::run_days(&mut reference, buildings, &script, 0, scale.split_day, deadline, "reference run (first half)", &mut ref_stats, |_, _, _| Ok(()))?;
    r.save_date = format!("{:?}", reference.date);
    r.queued_orders_at_save = reference.countries.iter().map(|c| c.construction_queue.len()).sum();
    let state_at_save = reference.clone();
    r.setup_ms = ms(t.elapsed());
    deadline.check("setup")?;

    let inputs = StaticInputs { map, world: &world, buildings, facilities: None };
    let mut last_resumed: Option<MapEconomySimulation> = None;
    let mut first_hash: Option<String> = None;
    let mut hashes_equal = true;

    // --- 試行(保存→新しいセッションへ復元)
    for index in 1..=spec.trials {
        let trial_base = base.join(format!("trial{index}"));
        std::fs::create_dir_all(&trial_base).map_err(|e| Abort::Unrunnable(format!("cannot create {}: {e}", trial_base.display())))?;
        let location = SaveLocation::new(&trial_base);
        // 計時の外: 復元先のセッションを新しく組み立てる
        let mut resumed = assemble(&input, &world, buildings)?;

        let mut save_timing = spec.timed.then(Timing::on);
        let t = Instant::now();
        let receipt = match &mut save_timing {
            Some(tm) => map_economy_file::save_timed(&reference, &inputs, &location, SAVE_NAME, tm),
            None => map_economy_file::save(&reference, &inputs, &location, SAVE_NAME),
        };
        let save_wall = t.elapsed();
        let receipt = receipt.map_err(|e| Abort::Unrunnable(format!("save failed: {e:?}")))?;
        if let Some(tm) = &save_timing
            && tm.outcome() != Outcome::Success
        {
            failures.push(format!("trial {index}: the save was timed with outcome {:?}", tm.outcome()));
        }
        deadline.check("save")?;

        let mut load_timing = spec.timed.then(Timing::on);
        let t = Instant::now();
        let loaded = match &mut load_timing {
            Some(tm) => map_economy_file::load_timed(&mut resumed, &inputs, &location, SAVE_NAME, tm),
            None => map_economy_file::load(&mut resumed, &inputs, &location, SAVE_NAME),
        };
        let load_wall = t.elapsed();
        let loaded = loaded.map_err(|e| Abort::Unrunnable(format!("restore failed: {e:?}")))?;
        if let Some(tm) = &load_timing
            && tm.outcome() != Outcome::Success
        {
            failures.push(format!("trial {index}: the restore was timed with outcome {:?}", tm.outcome()));
        }
        deadline.check("restore")?;

        // 計時の外の確認
        let bytes = std::fs::read(&receipt.path).map_err(|e| Abort::Unrunnable(format!("cannot read back the save: {e}")))?;
        let file_blake3 = hex(blake3::hash(&bytes).as_bytes());
        match &first_hash {
            None => first_hash = Some(file_blake3.clone()),
            Some(h) => hashes_equal &= *h == file_blake3,
        }
        let equal = resumed == state_at_save;
        if !equal {
            failures.push(format!("trial {index}: the restored state differs from the state at the time of saving"));
        }
        r.trials.push(Trial {
            index,
            save: OpRecord { wall_ms: ms(save_wall), timing: save_timing },
            restore: OpRecord { wall_ms: ms(load_wall), timing: load_timing },
            file_bytes: bytes.len() as u64,
            file_blake3,
            restored_equals_state_at_save: equal,
            verdict_exact: loaded.verdict.is_exact(),
            verdict_warnings: loaded.verdict.warnings.len(),
            restored_date: format!("{:?}", loaded.date),
        });
        // 後始末(自分が作った保存ファイルと試行用ディレクトリだけ)
        let _ = std::fs::remove_file(&receipt.path);
        let _ = std::fs::remove_dir(trial_base.join("map_economy"));
        let _ = std::fs::remove_dir(&trial_base);
        last_resumed = Some(resumed);
    }
    r.file_hashes_equal_across_trials = hashes_equal;
    if !hashes_equal {
        failures.push("the saved file bytes differ between trials of the same state".into());
    }
    let _ = std::fs::remove_dir(base);

    // --- 継続の比較(計時の外): 基準側と、最後に復元した状態の、後半の各日のダイジェスト
    let mut ref_digests: Vec<[u8; 32]> = Vec::new();
    crate::economy::scale_harness::run_days(&mut reference, buildings, &script, scale.split_day, scale.days, deadline, "reference run (second half)", &mut ref_stats, |_, s, _| {
        ref_digests.push(state_digest(s, &world, buildings).map_err(Abort::Unrunnable)?);
        Ok(())
    })?;
    let mut resumed = last_resumed.ok_or_else(|| Abort::Unrunnable("no trial ran".into()))?;
    let mut res_digests: Vec<[u8; 32]> = Vec::new();
    let mut res_stats = RunStats::default();
    crate::economy::scale_harness::run_days(&mut resumed, buildings, &script, scale.split_day, scale.days, deadline, "resumed run", &mut res_stats, |_, s, _| {
        res_digests.push(state_digest(s, &world, buildings).map_err(Abort::Unrunnable)?);
        Ok(())
    })?;
    let expected = scale.days - scale.split_day;
    r.compared_days = ref_digests.len().min(res_digests.len()) as u32;
    if ref_digests.len() as u32 != expected || res_digests.len() as u32 != expected {
        failures.push(format!("compared {} / {} days but {expected} were expected", ref_digests.len(), res_digests.len()));
    }
    for i in 0..r.compared_days as usize {
        if ref_digests[i] != res_digests[i] {
            r.digest_mismatch_days.push(scale.split_day + 1 + i as u32);
        }
    }
    if !r.digest_mismatch_days.is_empty() {
        failures.push(format!("state digests differ on {} days (first: day {})", r.digest_mismatch_days.len(), r.digest_mismatch_days[0]));
    }
    r.final_states_equal = resumed == reference;
    if !r.final_states_equal {
        failures.push("the final state after resuming differs from the continuous run".into());
    }
    r.distinct_digests = ref_digests.iter().collect::<std::collections::HashSet<_>>().len();
    let mut chain = blake3::Hasher::new();
    for d in &ref_digests {
        chain.update(d);
    }
    r.digest_chain = hex(chain.finalize().as_bytes());
    if r.compared_days == 0 || r.distinct_digests < 2 {
        failures.push("the comparison is empty or constant (nothing proves the states moved)".into());
    }
    if r.queued_orders_at_save == 0 {
        failures.push("no order was in progress at save time (the continuity check would not cover orders)".into());
    }
    Ok(failures)
}

fn dur_ms(d: Duration) -> f64 {
    ms(d)
}

impl BenchResult {
    /// 生の記録(1行1事実の `key=value`)。集計は `scripts` 側ではなく記録の読み取りで行う。
    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let case = format!("{}x{}", self.spec.scale.states, self.spec.scale.countries);
        let mode = if self.spec.timed { "on" } else { "off" };
        let mut o = String::new();
        let _ = writeln!(o, "CASE case={case} mode={mode} status={} states={} countries={} save_date={} queued_orders_at_save={} setup_ms={:.3} total_ms={:.3}",
            self.status.label(), self.states, self.countries, self.save_date, self.queued_orders_at_save, self.setup_ms, self.total_ms);
        for t in &self.trials {
            let _ = writeln!(o, "TRIAL case={case} mode={mode} trial={} save_wall_ms={:.3} restore_wall_ms={:.3} bytes={} file_blake3={} restored_equal={} verdict_exact={} warnings={} restored_date={}",
                t.index, t.save.wall_ms, t.restore.wall_ms, t.file_bytes, t.file_blake3, t.restored_equals_state_at_save, t.verdict_exact, t.verdict_warnings, t.restored_date);
            for (op, rec) in [("save", &t.save), ("restore", &t.restore)] {
                if let Some(tm) = &rec.timing {
                    let _ = writeln!(o, "TOTAL case={case} mode={mode} trial={} op={op} total_ms={:.3} covered_ms={:.3} uncovered_ms={:.3} outcome={:?}",
                        t.index, dur_ms(tm.total()), dur_ms(tm.covered()), dur_ms(tm.uncovered()), tm.outcome());
                    for s in tm.spans() {
                        let _ = writeln!(o, "SPAN case={case} mode={mode} trial={} op={op} name={} calls={} ms={:.3}", t.index, s.name, s.calls, dur_ms(s.total));
                    }
                    // 同じ呼出の整合性(P32-027): 各工程は全体の区間の中にあり、互いに重ならないので、工程の合計 <= 全体 <= 外側から計った時間。
                    let max_span = tm.spans().iter().map(|s| s.total).max().unwrap_or_default();
                    let _ = writeln!(o, "CHECK case={case} mode={mode} trial={} op={op} wall_ms={:.3} total_ms={:.3} covered_ms={:.3} max_span_ms={:.3} covered_le_total={} total_le_wall={} max_span_le_total={}",
                        t.index, rec.wall_ms, dur_ms(tm.total()), dur_ms(tm.covered()), dur_ms(max_span),
                        tm.covered() <= tm.total(), dur_ms(tm.total()) <= rec.wall_ms + 1e-6, max_span <= tm.total());
                }
            }
        }
        let _ = writeln!(o, "CONTINUITY case={case} mode={mode} compared_days={} mismatch_days={} final_equal={} distinct_digests={} digest_chain={} file_hashes_equal={}",
            self.compared_days, self.digest_mismatch_days.len(), self.final_states_equal, self.distinct_digests, self.digest_chain, self.file_hashes_equal_across_trials);
        match &self.status {
            CaseStatus::Success => {}
            CaseStatus::CorrectnessFailure(v) => {
                for f in v {
                    let _ = writeln!(o, "FAILURE case={case} mode={mode} {f}");
                }
            }
            CaseStatus::TimedOut { phase } => {
                let _ = writeln!(o, "INCOMPLETE case={case} mode={mode} timeout_in={phase}");
            }
            CaseStatus::Unrunnable(m) => {
                let _ = writeln!(o, "INCOMPLETE case={case} mode={mode} unrunnable={m}");
            }
        }
        o
    }
}

/// 比較測定(P32-027)の入力: 製品の保存で作った保存文章(ファイル内容そのもの)と、その規模。
#[derive(Debug, Clone)]
pub struct SavedText {
    pub text: String,
    pub states: usize,
    pub countries: usize,
    pub queued_orders: usize,
}

/// 合成入力を作り、`scale.split_day` 日進めた状態を**製品の `save`**で一時ディレクトリへ保存し、その文章を読み戻して返す。
/// 保存・読込は解析時間の測定に含めない(呼び出し側が文章を使い回す)。後始末は自分のファイルだけ。
pub fn build_saved_text(map: &HexMap, buildings: &BuildingRegistry, scale: &ScaleSpec, base: &Path) -> Result<SavedText, String> {
    scale.validate().map_err(|e| e.to_string())?;
    if base.components().any(|c| c.as_os_str() == "saves") {
        return Err(format!("measurement save base must not be inside a 'saves' directory: {}", base.display()));
    }
    let t0 = Instant::now();
    let deadline = Deadline { start: t0, limit: scale.time_limit };
    let abort = |a: Abort| match a {
        Abort::Timeout(p) => format!("timeout in {p}"),
        Abort::Unrunnable(m) => m,
    };
    let input = generate(scale, map).map_err(|e| e.to_string())?;
    let world = build_world(map, &input).map_err(|e| e.to_string())?;
    let mut sim = assemble(&input, &world, buildings).map_err(|e| e.to_string())?;
    let script = Script::new(&input, scale.scripted_orders);
    let mut stats = RunStats::default();
    crate::economy::scale_harness::run_days(&mut sim, buildings, &script, 0, scale.split_day, &deadline, "advance", &mut stats, |_, _, _| Ok(())).map_err(abort)?;
    let inputs = StaticInputs { map, world: &world, buildings, facilities: None };
    std::fs::create_dir_all(base).map_err(|e| format!("cannot create {}: {e}", base.display()))?;
    let location = SaveLocation::new(base);
    let receipt = map_economy_file::save(&sim, &inputs, &location, SAVE_NAME).map_err(|e| format!("save failed: {e:?}"))?;
    let text = std::fs::read_to_string(&receipt.path).map_err(|e| format!("cannot read back the save: {e}"));
    let _ = std::fs::remove_file(&receipt.path);
    let _ = std::fs::remove_dir(base.join("map_economy"));
    let _ = std::fs::remove_dir(base);
    Ok(SavedText {
        text: text?,
        states: sim.registry.len(),
        countries: sim.countries.len(),
        queued_orders: sim.countries.iter().map(|c| c.construction_queue.len()).sum(),
    })
}
